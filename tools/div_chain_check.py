#!/usr/bin/env python3
"""div_chain_check.py — the divider and square-root pass's two chains, side
by side (bead SAT-playground-lhf, 2026-11-18).

The pass (solver/13-kissat-rs/src/structdiv.rs) proves a miter's row pairs
equal one after another. It can do that in two ways:

  fresh   a fresh solver for every pair, the circuit loaded again each time
          (--structdivassume=0, the chain as it was);
  one     the pairs as questions to one solver that is loaded once and keeps
          what it learned (--structdivassume=1, the default).

This tool runs each cell both ways and prints per cell and chain: the pairs
proven, the pass's work, the search's conflicts afterwards, the whole run's
work and time, and with --proof the size of the proof and the verdict and
time of tools/proof_gate.py on it.

    python3 tools/div_chain_check.py run --solver BIN --out DIR [--proof]
        [--jobs N] [--cores LIST] [--timeout S] [--gate-timeout S]
        [--mem-mb MB] CNF...
    python3 tools/div_chain_check.py selftest --solver BIN

How a cell is run. Its two runs get two cores of their own (neighbours in
--cores) and are started at the same moment, so their times are taken under
the same load; with --proof the two gates are then started together on the
same two cores. --jobs is the number of cells at a time (two cores each,
default 1). Every solver runs under a memory limit (--mem-mb, 16,000 by
default, the repo's cap) and every process in a group of its own: at a time
limit the whole group is stopped before its core is given to the next run.

What counts as a problem (exit code 1): a run that ends without an answer
and without reaching the time limit, or whose exit code does not fit its
answer (a crash, a refused option, a memory-out); an UNKNOWN before the time
limit (the tool gives the solver no budget, so nothing explains one); a SAT
answer whose model does not satisfy the input (tools/verify_sat.py, always);
two chains that disagree on a cell's answer; a proof that fails the gate;
`one` proving fewer pairs than `fresh` (no chain at all counts as none,
unless the pass refuted the formula), or reaching the time limit where
`fresh` answers. A run stopped by the time limit is reported as TIMEOUT and is no
problem by itself. A cell on which the pass does not fire is reported as
such (no chain).

Files. DIR must be new or empty: every check reads files of this run only,
never a model, a proof or an unpacked input an older run left behind. Each
input's files in DIR carry the input's own file name (without .cnf and the
compression's ending); a second input of the same name gets a number, so
two inputs never share a file. The table (DIR/table.tsv) has the
input's path and that name beside the short name shown on the screen.

When the tool itself is stopped (SIGTERM, SIGINT), it stops every solver and
checker it started before it exits.
"""
from __future__ import annotations

import argparse
import csv
import os
import re
import shlex
import signal
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GATE = ROOT / "tools" / "proof_gate.py"
VERIFY_SAT = ROOT / "tools" / "verify_sat.py"
CHAINS = (("fresh", "--structdivassume=0"), ("one", "--structdivassume=1"))
NOTE = re.compile(r"div/sqrt: (.*?), (\d+) of (\d+) row pairs proven equal \((\d+) dropped by a check, (\d+) by a model\).*?work (\d+)K")
REFUTED = re.compile(r"div/sqrt: (.*?), the circuit without its root is unsatisfiable.*?work (\d+)K")
CLOCK = re.compile(r"^c workclock .*? conflicts=(\d+) .*work=(\d+)", re.M)
FIELDS = ("cell", "chain", "status", "exit", "what", "proven", "pairs", "dropped", "by_model", "pass_work", "ratio",
          "search_conflicts", "work", "seconds", "model", "pbp_bytes", "drat_bytes", "gate", "gate_seconds", "files", "input")
EXIT_OF = {"SATISFIABLE": 10, "UNSATISFIABLE": 20, "UNKNOWN": 0}
KILL_GRACE_S = 10


def parse_cores(text: str) -> list:
    out = []
    for part in text.split(","):
        if "-" in part:
            a, b = part.split("-")
            out.extend(range(int(a), int(b) + 1))
        else:
            out.append(int(part))
    return out


def file_stem(path: str) -> str:
    name = os.path.basename(path)
    for end in (".xz", ".gz", ".cnf"):
        if name.endswith(end):
            name = name[: -len(end)]
    return name


def cell_name(path: str) -> str:
    """The short name for the screen (the benchmark's hash prefix left out)."""
    return re.sub(r"^[0-9a-f]{32}-", "", file_stem(path))


def file_names(cnfs: list) -> list:
    """One name per input for its files in the output folder, no two alike:
    the input's own file name, with a number when it was taken already."""
    used, names = set(), []
    for path in cnfs:
        stem = file_stem(path) or "input"
        name, k = stem, 1
        while name in used:
            k += 1
            name = f"{stem}~{k}"
        used.add(name)
        names.append(name)
    return names


# every process started and not yet waited for, so that a signal to this
# tool can stop them all (each is in a session of its own)
ACTIVE: set = set()
ACTIVE_LOCK = threading.Lock()
STOPPING = False


def stop_all(signum, _frame) -> None:
    """The tool is being stopped: stop every group it started, then exit."""
    global STOPPING
    with ACTIVE_LOCK:
        STOPPING = True
        procs = list(ACTIVE)
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for proc in procs:
            try:
                os.killpg(proc.p.pid, sig)
            except ProcessLookupError:
                pass
        end = time.time() + KILL_GRACE_S
        while time.time() < end:
            alive = False
            for proc in procs:
                proc.p.poll()
                try:
                    os.killpg(proc.p.pid, 0)
                    alive = True
                except ProcessLookupError:
                    pass
            if not alive:
                break
            time.sleep(0.05)
    try:
        sys.stdout.flush()
    finally:
        os._exit(128 + signum)


class Proc:
    """One command in a process group of its own, under a memory limit (none
    when `mem_mb` is None), with its output in files. `wait` returns (exit
    code, seconds, timed out); at the time limit the whole group is stopped
    before `wait` returns."""

    def __init__(self, cmd: list, stdout: str, stderr: str, mem_mb, env=None):
        inner = "exec " + " ".join(shlex.quote(str(a)) for a in cmd)
        if mem_mb is not None:
            inner = f"ulimit -v {mem_mb * 1024}; " + inner
        self.out, self.err = open(stdout, "w"), open(stderr, "w")
        with ACTIVE_LOCK:
            if STOPPING:
                raise RuntimeError("the tool is being stopped")
            self.start = time.time()
            self.p = subprocess.Popen(["bash", "-c", inner], stdout=self.out, stderr=self.err, env=env, start_new_session=True)
            ACTIVE.add(self)

    def stop_group(self) -> None:
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(self.p.pid, sig)
            except ProcessLookupError:
                return
            end = time.time() + KILL_GRACE_S
            while time.time() < end:
                try:
                    os.killpg(self.p.pid, 0)
                except ProcessLookupError:
                    return
                self.p.poll()
                time.sleep(0.05)

    def wait(self, timeout: float) -> tuple:
        timed_out = False
        try:
            self.p.wait(timeout=max(0.0, self.start + timeout - time.time()))
        except subprocess.TimeoutExpired:
            timed_out = True
        seconds = time.time() - self.start
        if timed_out:
            # the leader is still there, so the group's number is its own:
            # stop every process in it, the checkers a gate started included
            self.stop_group()
        self.p.wait()
        with ACTIVE_LOCK:
            ACTIVE.discard(self)
        self.out.close()
        self.err.close()
        return self.p.returncode, seconds, timed_out


def together(procs: list, timeout: float) -> list:
    """Wait for processes that were started together; each one's own time."""
    results = [None] * len(procs)

    def wait(i):
        results[i] = procs[i].wait(timeout)

    threads = [threading.Thread(target=wait, args=(i,)) for i in range(len(procs))]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return results


def read_run(base: str, cell: str, chain: str, code, seconds: float, timed_out: bool) -> dict:
    text = open(base + ".out", errors="replace").read()
    row = dict.fromkeys(FIELDS, "")
    row.update(cell=cell, chain=chain, seconds=f"{seconds:.1f}", exit=str(code))
    status = re.search(r"^s (\w+)", text, re.M)
    if timed_out:
        row["status"] = "TIMEOUT"
    elif status and status.group(1) in ("SATISFIABLE", "UNSATISFIABLE") and EXIT_OF[status.group(1)] == code:
        row["status"] = status.group(1)
    else:
        # no answer and no time limit, an exit code that does not fit the
        # answer, or an UNKNOWN with no budget to explain it: a crash, a
        # refused option, a memory-out, a search that gave up early
        row["status"] = f"ERROR(exit {code}" + (f", s {status.group(1)}" if status else ", no answer") + ")"
    note = NOTE.search(text)
    refuted = REFUTED.search(text)
    if note:
        row.update(what=note.group(1), proven=note.group(2), pairs=note.group(3), dropped=note.group(4),
                   by_model=note.group(5), pass_work=str(int(note.group(6)) * 1000))
    elif refuted:
        row.update(what=refuted.group(1) + " (refuted by the pass)", pass_work=str(int(refuted.group(2)) * 1000))
    clock = CLOCK.search(text)
    if clock:
        row.update(search_conflicts=clock.group(1), work=clock.group(2))
    return row


def run_cell(cores: tuple, solver: str, cnf: str, name: str, out: str, proof: bool, timeout: float, gate_timeout: float, mem_mb: int) -> list:
    cell = cell_name(cnf)
    # (no budget of the solver's own: the time limit is the only one, so an
    # UNKNOWN before it is an error)
    env = {k: v for k, v in os.environ.items() if k != "SAT_LIMIT_TICKS"}
    env["SAT_STRUCT_DEBUG"] = "1"
    bases, procs = [], []
    for (chain, option), core in zip(CHAINS, cores):
        base = os.path.join(out, f"{name}.{chain}")
        cmd = ["taskset", "-c", str(core), solver, option, cnf]
        if proof:
            cmd.append(base + ".drat")
        bases.append(base)
        procs.append(Proc(cmd, base + ".out", base + ".err", mem_mb, env))
    rows = [read_run(base, cell, chain, *result)
            for base, (chain, _), result in zip(bases, CHAINS, together(procs, timeout))]
    for row in rows:
        row.update(files=name, input=cnf)
    # a SAT answer counts only with a model that satisfies the input
    checks = {}
    for row, base, core in zip(rows, bases, cores):
        if row["status"] == "SATISFIABLE":
            plain = cnf
            if cnf.endswith(".xz"):
                # (the model check reads plain and gzip files; the folder
                # was empty at the start, so a file here is this cell's own)
                plain = os.path.join(out, f"{name}.cnf")
                if not checks:
                    with open(plain, "wb") as f:
                        subprocess.run(["xz", "-dc", cnf], stdout=f, check=True)
            cmd = ["taskset", "-c", str(core), sys.executable, str(VERIFY_SAT), plain, base + ".out"]
            checks[row["chain"]] = Proc(cmd, base + ".model", base + ".model.err", None)
    results = together(list(checks.values()), gate_timeout)
    for row in rows:
        if row["chain"] in checks:
            code, _seconds, timed_out = results[list(checks).index(row["chain"])]
            row["model"] = "checker-timeout" if timed_out else ("ok" if code == 0 else "FAIL")
    if proof:
        gates = {}
        for row, base, core in zip(rows, bases, cores):
            for key, path in (("pbp_bytes", base + ".drat.pbp"), ("drat_bytes", base + ".drat")):
                if os.path.exists(path):
                    row[key] = str(os.path.getsize(path))
            if row["status"] == "UNSATISFIABLE":
                cmd = ["taskset", "-c", str(core), sys.executable, str(GATE), cnf, base + ".drat", "--timeout", str(gate_timeout)]
                # (no memory limit here: the checkers are not the measured runs)
                gates[row["chain"]] = Proc(cmd, base + ".gate", base + ".gate.err", None)
        # the gate has a limit per stage; the outer one covers all of them
        results = together(list(gates.values()), 4 * gate_timeout + 600)
        for row, base in zip(rows, bases):
            if row["chain"] not in gates:
                continue
            code, seconds, timed_out = results[list(gates).index(row["chain"])]
            lines = open(base + ".gate", errors="replace").read().strip().splitlines()
            row["gate"] = "checker-timeout" if timed_out else (lines[-1] if lines else f"no output (exit {code})")
            row["gate_seconds"] = f"{seconds:.1f}"
    return rows


def run_cells(solver: str, cnfs: list, out: str, proof: bool, jobs: int, cores: list, timeout: float,
              gate_timeout: float, mem_mb: int) -> tuple:
    os.makedirs(out, exist_ok=True)
    if os.listdir(out):
        raise SystemExit(f"{out} is not empty: every run needs a folder of its own, so that no check reads an older run's files")
    pairs = [(cores[i], cores[i + 1]) for i in range(0, len(cores) - 1, 2)]
    if not pairs:
        raise SystemExit("two cores are needed: a cell's two runs are started together")
    jobs = max(1, min(jobs, len(pairs), len(cnfs)))
    free = list(pairs[:jobs])
    lock = threading.Lock()

    def task(item):
        cnf, name = item
        with lock:
            pair = free.pop()
        try:
            return run_cell(pair, solver, cnf, name, out, proof, timeout, gate_timeout, mem_mb)
        finally:
            with lock:
                free.append(pair)

    with ThreadPoolExecutor(jobs) as pool:
        rows = [row for cell_rows in pool.map(task, list(zip(cnfs, file_names(cnfs)))) for row in cell_rows]
    problems = []
    for at in range(0, len(rows), 2):
        fresh, one = rows[at], rows[at + 1]
        cell = fresh["files"]
        for r in (fresh, one):
            if r["status"].startswith("ERROR"):
                problems.append(f"{cell} {r['chain']}: the run ended without a fitting answer, {r['status']}")
            if r["status"] == "SATISFIABLE" and r["model"] != "ok":
                problems.append(f"{cell} {r['chain']}: the model check says {r['model'] or 'nothing'}")
            if r["gate"] and r["gate"] != "ok":
                problems.append(f"{cell} {r['chain']}: the gate says {r['gate']}")
        if fresh["pass_work"] and one["pass_work"]:
            one["ratio"] = f"{int(one['pass_work']) / max(1, int(fresh['pass_work'])):.2f}"
            fresh["ratio"] = "1.00"
        answers = {r["status"] for r in (fresh, one)} & {"SATISFIABLE", "UNSATISFIABLE"}
        if len(answers) > 1:
            problems.append(f"{cell}: the chains disagree ({fresh['status']} v {one['status']})")
        # one solver's pairs against the fresh chain's; a run of `one` that
        # ended with no chain at all proved none, unless the pass refuted
        # the formula there (a run at the time limit is judged below)
        one_proven = one["proven"] or ("" if "refuted" in one["what"] or one["status"] == "TIMEOUT" else "0")
        if fresh["proven"] and one_proven and int(one_proven) < int(fresh["proven"]):
            problems.append(f"{cell}: one solver proves fewer pairs ({one_proven} v {fresh['proven']})")
        if one["status"] == "TIMEOUT" and fresh["status"] in ("SATISFIABLE", "UNSATISFIABLE"):
            problems.append(f"{cell}: one solver reaches the time limit where the fresh chain answers {fresh['status']}")
    with open(os.path.join(out, "table.tsv"), "w", newline="") as f:
        w = csv.DictWriter(f, FIELDS, delimiter="\t", lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    return rows, problems


def show(rows: list, proof: bool) -> None:
    def num(text):
        return f"{int(text):,}" if text else "-"

    def mb(text):
        return f"{int(text) / 1e6:.1f}" if text else "-"

    head = f"{'cell':22} {'chain':5} {'answer':13} {'pairs proven':>13} {'pass work':>14} {'ratio':>5} {'search confl':>12} {'run work':>15} {'time s':>7}"
    if proof:
        head += f" {'pbp MB':>7} {'drat MB':>7} {'gate':>15} {'gate s':>7}"
    print(head)
    for r in rows:
        pairs = f"{r['proven']} of {r['pairs']}" if r["pairs"] else ("refuted" if "refuted" in r["what"] else "no chain")
        answer = r["status"] if r["model"] in ("", "ok") else f"SAT, model {r['model']}"
        line = (f"{r['cell'][:22]:22} {r['chain']:5} {answer[:13]:13} {pairs:>13} {num(r['pass_work']):>14} {r['ratio']:>5} "
                f"{num(r['search_conflicts']):>12} {num(r['work']):>15} {r['seconds']:>7}")
        if proof:
            line += f" {mb(r['pbp_bytes']):>7} {mb(r['drat_bytes']):>7} {r['gate'] or '-':>15} {r['gate_seconds'] or '-':>7}"
        print(line)


def cmd_run(args) -> int:
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, stop_all)
    rows, problems = run_cells(args.solver, args.cnf, args.out, args.proof, args.jobs, parse_cores(args.cores),
                               args.timeout, args.gate_timeout, args.mem_mb)
    show(rows, args.proof)
    for p in problems:
        print("PROBLEM:", p)
    print(f"[div-chain] {len(rows) // 2} cells, {len(problems)} problems; table in {os.path.join(args.out, 'table.tsv')}")
    return 1 if problems else 0


def cmd_selftest(args) -> int:
    """Toy dividers through both chains with proofs: the same pairs, both
    proofs through the gate. Then the checks themselves, each on a stand-in
    for the solver (a small shell script): a run that fails, an answer with
    the wrong exit code, an UNKNOWN with no budget, two chains that disagree,
    a run that never ends, one solver past the limit where the fresh chain
    answers, the memory limit, the two runs started together, two inputs of
    one name, and the tool itself stopped by a signal."""
    sys.path.insert(0, str(ROOT / "tools"))
    import divsqrt_proof  # noqa: E402  (the toy generator)

    cores = parse_cores(args.cores)
    failures = []
    with tempfile.TemporaryDirectory() as d:
        cnfs = []
        for n, k, shortcut in ((8, 3, False), (12, 4, True)):
            path = os.path.join(d, f"toy_n{n}_k{k}_{int(shortcut)}.cnf")
            ns = argparse.Namespace(n=n, k=k, shortcut=shortcut, out=path)
            with open(os.devnull, "w") as quiet:
                stdout, sys.stdout = sys.stdout, quiet
                try:
                    divsqrt_proof.cmd_toy(ns)
                finally:
                    sys.stdout = stdout
            cnfs.append(path)
        rows, problems = run_cells(args.solver, cnfs, os.path.join(d, "out"), True, 2, cores, 300, 300, 16000)
        show(rows, True)
        if problems:
            failures.append(f"problems on the toys: {problems}")
        for r in rows:
            if r["status"] != "UNSATISFIABLE" or r["gate"] != "ok" or not r["pairs"]:
                failures.append(f"{r['cell']} {r['chain']}: {r['status']}, gate {r['gate']!r}, pairs {r['pairs']!r}")
        table = list(csv.DictReader(open(os.path.join(d, "out", "table.tsv")), delimiter="\t"))
        if len(table) != len(rows) or table[0]["cell"] != rows[0]["cell"] or table[0]["input"] != cnfs[0]:
            failures.append("table.tsv does not hold the rows")

        def stand_in(name: str, body: str) -> str:
            path = os.path.join(d, name)
            with open(path, "w") as f:
                f.write("#!/bin/bash\n" + body + "\n")
            os.chmod(path, 0o755)
            return path

        def check(name: str, body: str, want: str, timeout: float = 30, inputs=None) -> list:
            rows, problems = run_cells(stand_in(name, body), inputs or [tiny], os.path.join(d, name + ".out"), False, 1, cores, timeout, 30, 16000)
            got = [r["status"] for r in rows]
            if want == "problem" and not problems:
                failures.append(f"{name}: no problem reported (statuses {got})")
            if want == "none" and problems:
                failures.append(f"{name}: a problem reported where there is none: {problems}")
            return rows

        def running(pidfile: str) -> list:
            left = []
            for pid in open(pidfile).read().split():
                try:
                    os.kill(int(pid), 0)
                    left.append(pid)
                except ProcessLookupError:
                    pass
            return left

        # the stand-ins answer a formula of one clause, (x1), with its model
        tiny = os.path.join(d, "tiny.cnf")
        open(tiny, "w").write("p cnf 1 1\n1 0\n")
        sat = "echo 's SATISFIABLE'; echo 'v 1 0'; exit 10"
        # a solver that fails at once is an error, not a timeout
        rows = check("fails", "exit 1", "problem")
        if not all(r["status"].startswith("ERROR") for r in rows):
            failures.append(f"a failing solver is read as {[r['status'] for r in rows]}")
        # an answer with the wrong exit code is an error
        check("wrongcode", "echo 's UNSATISFIABLE'; exit 0", "problem")
        # an UNKNOWN before the time limit is an error: the solver has no budget
        check("unknown", "echo 's UNKNOWN'; exit 0", "problem")
        # an answer with its own exit code and a model of the input is none
        rows = check("answers", sat, "none")
        if not all(r["status"] == "SATISFIABLE" and r["model"] == "ok" for r in rows):
            failures.append(f"an answer is read as {[(r['status'], r['model']) for r in rows]}")
        # a folder an older run wrote to is refused: its files could pass for this run's
        try:
            run_cells(stand_in("answers", sat), [tiny], os.path.join(d, "answers.out"), False, 1, cores, 30, 30, 16000)
            failures.append("a second run into the same folder is not refused")
        except SystemExit:
            pass
        # a SAT answer with a model that does not satisfy the input, or with none
        rows = check("badmodel", "echo 's SATISFIABLE'; echo 'v -1 0'; exit 10", "problem")
        if not all(r["model"] == "FAIL" for r in rows):
            failures.append(f"a wrong model is read as {[r['model'] for r in rows]}")
        check("nomodel", "echo 's SATISFIABLE'; exit 10", "problem")
        # the fresh chain proves pairs and one solver reports no chain at all
        line = "c structure pass: div/sqrt: quotient bit 1 of a 2/2 divider, 8 of 8 row pairs proven equal (0 dropped by a check, 0 by a model), work 5K"
        check("nochain", f"case \"$*\" in *structdivassume=0*) echo '{line}';; esac; {sat}", "problem")
        # ... but none when the pass refuted the formula there, or both report the same
        refuted = "c structure pass: div/sqrt: quotient bit 1 of a 2/2 divider, the circuit without its root is unsatisfiable (asked of one solver), work 5K"
        check("refutes", f"case \"$*\" in *structdivassume=0*) echo '{line}';; *) echo '{refuted}';; esac; {sat}", "none")
        check("samechain", f"echo '{line}'; {sat}", "none")
        check("fewer", f"case \"$*\" in *structdivassume=0*) echo '{line}';; *) echo '{line}' | sed 's/8 of 8/7 of 8/';; esac; {sat}", "problem")
        # the memory limit is set (16,000 MB in KB) and no work limit is passed on
        os.environ["SAT_LIMIT_TICKS"] = "1"
        try:
            rows = check("limit", f"test \"$(ulimit -v)\" = 16384000 || exit 3; test -z \"$SAT_LIMIT_TICKS\" || exit 5; {sat}", "none")
        finally:
            del os.environ["SAT_LIMIT_TICKS"]
        if not all(r["status"] == "SATISFIABLE" for r in rows):
            failures.append(f"the memory limit is not set, or a work limit is passed on: {[r['status'] for r in rows]}")
        # two chains that disagree
        check("disagree", "case \"$*\" in *structdivassume=0*) echo 's SATISFIABLE'; exit 10;; *) echo 's UNSATISFIABLE'; exit 20;; esac", "problem")
        # a run that never ends is a TIMEOUT, no problem, and nothing of its
        # group is left running: its child wrote its own pid, and is gone
        pidfile = os.path.join(d, "child.pid")
        hang = f"(echo $BASHPID >> {shlex.quote(pidfile)}; exec sleep 600) & wait"
        start = time.time()
        rows = check("hangs", hang, "none", timeout=1)
        if not all(r["status"] == "TIMEOUT" for r in rows):
            failures.append(f"a run past the limit is read as {[r['status'] for r in rows]}")
        if time.time() - start > 60:
            failures.append("stopping a run at the limit took over a minute")
        if running(pidfile) or len(open(pidfile).read().split()) != 2:
            failures.append(f"children left running after the limit: {running(pidfile)} (pids written: {open(pidfile).read().split()})")
        # one solver past the limit where the fresh chain answers is a problem
        check("onehangs", f"case \"$*\" in *structdivassume=0*) {sat};; *) exec sleep 600;; esac", "problem", timeout=1)
        # the two runs of a cell are started together: each stand-in waits
        # for the other's start mark
        mark = os.path.join(d, "mark")
        check("together", f"touch {shlex.quote(mark)}.$$; for i in $(seq 100); do [ $(ls {shlex.quote(mark)}.* | wc -l) -ge 2 ] && {{ {sat}; }}; sleep 0.05; done; exit 4", "none")
        # two inputs of one name (in two folders) keep their own files
        twins = []
        for folder in ("a", "b"):
            os.makedirs(os.path.join(d, folder))
            twins.append(os.path.join(d, folder, "0123456789abcdef0123456789abcdef-twin.cnf"))
            open(twins[-1], "w").write("p cnf 0 0\n")
        rows = check("twins", f"echo \"c input $2\"; {sat}", "none", inputs=twins)
        files = sorted({r["files"] for r in rows})
        if len(files) != 2 or [r["input"] for r in rows] != [twins[0], twins[0], twins[1], twins[1]]:
            failures.append(f"two inputs of one name: files {files}")
        for r in rows:
            text = open(os.path.join(d, "twins.out", f"{r['files']}.{r['chain']}.out")).read()
            if f"c input {r['input']}\n" not in text:
                failures.append(f"two inputs of one name: {r['files']}.{r['chain']}.out is not the run of {r['input']}")
        # the tool stopped by a signal stops what it started
        pidfile2 = os.path.join(d, "child2.pid")
        hang2 = stand_in("hangs2", f"(echo $BASHPID >> {shlex.quote(pidfile2)}; exec sleep 600) & wait")
        runner = subprocess.Popen([sys.executable, os.path.abspath(__file__), "run", "--solver", hang2, "--out", os.path.join(d, "signal.out"),
                                   "--cores", args.cores, "--timeout", "600", tiny], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        end = time.time() + 30
        while time.time() < end and not (os.path.exists(pidfile2) and len(open(pidfile2).read().split()) == 2):
            time.sleep(0.05)
        runner.send_signal(signal.SIGTERM)
        try:
            code = runner.wait(timeout=60)
        except subprocess.TimeoutExpired:
            runner.kill()
            code = None
        if code != 128 + signal.SIGTERM:
            failures.append(f"the tool stopped by SIGTERM exits with {code}")
        if not os.path.exists(pidfile2) or len(open(pidfile2).read().split()) != 2 or running(pidfile2):
            failures.append(f"the tool stopped by SIGTERM leaves its runs: {running(pidfile2) if os.path.exists(pidfile2) else 'none started'}")
            for pid in running(pidfile2) if os.path.exists(pidfile2) else []:
                os.kill(int(pid), signal.SIGKILL)
    for f in failures:
        print("FAIL:", f)
    print("[div-chain] selftest", "FAILED" if failures else "ok")
    return 1 if failures else 0


def main(argv) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("cnf", nargs="+")
    r.add_argument("--solver", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--proof", action="store_true", help="write proofs and check them with tools/proof_gate.py")
    r.add_argument("--jobs", type=int, default=1, help="cells at a time (a cell's two runs take two cores)")
    r.add_argument("--cores", default="0-15,18-33", help="the cores to pin runs to, two neighbours per cell")
    r.add_argument("--timeout", type=float, default=3600)
    r.add_argument("--gate-timeout", type=float, default=7200)
    r.add_argument("--mem-mb", type=int, default=16000, help="memory limit per solver process")
    r.set_defaults(func=cmd_run)
    s = sub.add_parser("selftest")
    s.add_argument("--solver", required=True)
    s.add_argument("--cores", default="0-15,18-33")
    s.set_defaults(func=cmd_selftest)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
