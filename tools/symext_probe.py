#!/usr/bin/env python3
"""symext_probe.py — general symmetry breaking in front of solver 13, measured
(bead SAT-playground-1v2.3.1, step two of SAT-playground-1v2.3).

The structure pass's symmetry step (structsym) fixes interchangeable rows. The
structure census (benchmarks/rl/structure_2025.tsv) finds symmetry generators
on 159 of the 400 cells of 2025, and the default solver leaves about 38 of them
unsolved on at least one seed. Satsuma, an outside symmetry tool, finds the
whole symmetry group with a graph automorphism search (dejavu) and uses it in
one of two modes: `fix` fixes variables, round after round, and `lex` adds
lex-leader clauses (they keep a solution only when no symmetric copy of it is
smaller). This probe puts Satsuma in front of the default solver and counts
the cells it adds.

Satsuma is a measuring stick, built outside the repo and never shipped (the
repo rule is no external solver dependency). Build both pieces at the commits
read for malicious code on 2026-11-04 (none found: no network, process,
environment or library-loading calls; it writes only the files named on its
command line). Do not use Satsuma's CMakeLists: it downloads dejavu's moving
main branch at configure time (GIT_TAG origin/main), so the code built would
not be the code read. One compiler call on the single source file, with the
flags the CMake release build uses, runs no downloaded build script:

    git clone https://github.com/markusa4/satsuma ~/.cache/sat13-tools/satsuma
    git -C ~/.cache/sat13-tools/satsuma checkout 4f73975c92cadaa83a64bb2c66976bde5ee8b618
    git clone https://github.com/markusa4/dejavu ~/.cache/sat13-tools/satsuma/src/dejavu
    git -C ~/.cache/sat13-tools/satsuma/src/dejavu checkout f9844859d9f3be5558a8f3957029236a7d26ff65
    cd ~/.cache/sat13-tools/satsuma && g++ -std=c++20 -O3 -Wall -march=native \\
        -DNDEBUG -DCLIQUES=0 -o satsuma src/satsuma.cpp

Subcommands (the RL venv, ~/.cache/sat13-rl/venv/bin/python, or python3):

  break     Satsuma MODE --add-reduced-as-unit on every target cell, under a
            memory limit and a time cap. The targets are the cells where the
            census found generators, plus the cells it could not finish (too
            big or out of time); --cells picks others. Writes the suite
            directory: <stem>.<mode>.cnf for every output, selection.csv (the
            instances feature_ablation runs), break.tsv (status, seconds and
            Satsuma's statistics per cell and mode), META.txt and logs/. With
            --add-reduced-as-unit every variable Satsuma removes comes back as
            a unit clause, so a model of the output is a model of the input.
            A second call resumes: rows already in break.tsv are kept.
  run       feature_ablation.py on the suite with a frozen solver: arms s0 and
            s1 (seeds 0 and 1), 1,800 s, 32 jobs, 16 GB. --dry-run prints it.
  report    Each cell and mode against the run of record: the default solver
            on the original formula, log/abtest-struct-card-400-2026-11-01-
            22-15-59, arms cur0 and cur1 (the current binary repeats its work
            counts). Satsuma's seconds count against the limit. Where Satsuma
            did not finish, the cell falls back to the original formula: the
            record's answer, after the seconds Satsuma spent. A cell answered
            SAT somewhere and UNSAT somewhere else (the record, either mode,
            either seed) is a FAIL, not a solve: the run's gate checks the
            answer against Satsuma's output, so only this comparison sees a
            satisfiable formula turned unsatisfiable. So is an answer the
            solver printed before an abnormal exit. Prints the summary and
            writes the per-cell table (--out).
  check     End-to-end checks of the answers the report lists as gained, and
            of every probe answer on a contradicted cell (or --only
            stem:mode:seed). The rerun must give the run's answer. Exit 0 when
            every pick is verified, 1 on any FAIL, 2 when one is unverified (a
            checker timeout, a rerun that did not finish). Each check runs
            Satsuma again with a VeriPB proof, the
            solver again on its output at the same seed, then a SAT model
            against the original formula, or for UNSAT two stages: VeriPB on
            the original formula with Satsuma's proof followed by one `rup`
            line per clause of Satsuma's output (every output clause follows
            from what the proof derived, so the output is satisfiable when the
            input is), and the gate (tools/proof_gate.py) on the solver's
            proof of the output. Where Satsuma refuted the formula itself (its
            output holds the empty clause), VeriPB alone settles it: the same
            stream ends in `conclusion UNSAT`, which VeriPB checks. A negative
            control, the same stream with a `pol` line on a constraint that
            does not exist, must be rejected, which shows VeriPB read the
            lines after Satsuma's proof. (A `rup` unit on a fresh variable is
            no control: after a contradiction every line follows, so VeriPB
            accepts it.)
  selftest  The report's arithmetic on a made-up table and, with --satsuma,
            the whole path on two pigeonhole toys with negative controls.

    PY=~/.cache/sat13-rl/venv/bin/python; SAT=~/.cache/sat13-tools/satsuma/satsuma
    $PY tools/symext_probe.py selftest --satsuma $SAT
    $PY tools/symext_probe.py break --satsuma $SAT --suite <scratch>/symext-suite --jobs 16
    nohup $PY tools/symext_probe.py run --suite <scratch>/symext-suite \\
        --solver solver/13-j9-symext-snapshot > <log> 2>&1 &
    $PY tools/symext_probe.py report --suite <scratch>/symext-suite --run log/abtest-symext-2025-<ts> \\
        --out benchmarks/rl/symext_2025.tsv
    $PY tools/symext_probe.py check --satsuma $SAT --suite <scratch>/symext-suite \\
        --run log/abtest-symext-2025-<ts> --solver-bin solver/13-j9-symext-snapshot/target/release/sat-solver
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import lzma
import math
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import proof_gate  # noqa: E402  (the solver's UNSAT answers are checked as the harness checks them)

SOURCE = ROOT / "benchmarks" / "sat-comp-2025"
CENSUS = ROOT / "benchmarks" / "rl" / "structure_2025.tsv"
RECORD = ROOT / "log" / "abtest-struct-card-400-2026-11-01-22-15-59"
RECORD_ARMS = ("cur0", "cur1")      # the default solver at seeds 0 and 1
PROBE_ARMS = ("s0", "s1")           # the arms `run` starts, at the same seeds
MODES = ("fix", "lex")
LIMIT = 1800.0
SOLVED = ("SATISFIABLE", "UNSATISFIABLE")
EXIT_CODE = {"SATISFIABLE": 10, "UNSATISFIABLE": 20}    # the solver's exit code with each answer (kissat's)
# the harness's check verdicts that let an answer count as solved: ok, and checker-timeout
# (solved but unverified, as the run of record counts it; the report shows the split)
CHECKED = ("ok", "checker-timeout")
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
STATS = {
    "gens": re.compile(r"dejavu_gens\s*=\s*(\d+)"),
    "row": re.compile(r"\brow\s*=\s*(\d+)"),
    "row_column": re.compile(r"\brow_column\s*=\s*(\d+)"),
    "johnson": re.compile(r"\bjohnson\s*=\s*(\d+)"),
    "units": re.compile(r"\bsymmetry_units\s*=\s*(\d+)"),
    "orbitopal_units": re.compile(r"\borbitopal_units\s*=\s*(\d+)"),
    "binary": re.compile(r"\bsymmetry_binary\s*=\s*(\d+)"),
    "amo_binary": re.compile(r"\bamo_binary\s*=\s*(\d+)"),
    "lex": re.compile(r"\bsymmetry_lex\s*=\s*(\d+)"),
    "iterations": re.compile(r"\biterations\s*=\s*(\d+)"),
}
BREAK_COLS = ["stem", "family", "mode", "status", "satsuma_s", "decided", *STATS,
              "vars_in", "clauses_in", "vars_out", "clauses_out", "out_mb", "sha256"]


# ---------------------------------------------------------------------------
# small helpers

def read_tsv(path) -> list[dict]:
    with open(path, newline="") as fh:
        return list(csv.DictReader(fh, delimiter="\t"))


def fnum(x) -> float | None:
    try:
        return float(x)
    except (TypeError, ValueError):
        return None


def decompress(src: Path, dst: Path) -> None:
    with lzma.open(src, "rb") as fi, open(dst, "wb") as fo:
        shutil.copyfileobj(fi, fo, 1 << 24)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 24), b""):
            h.update(chunk)
    return h.hexdigest()


def cnf_header(path: Path) -> tuple[int, int]:
    """The `p cnf V C` line, which comes before any clause."""
    with open(path, "rb") as fh:
        for line in fh:
            s = line.strip()
            if not s or s.startswith(b"c"):
                continue
            parts = s.split()
            if len(parts) >= 4 and parts[0] == b"p" and parts[1] == b"cnf":
                return int(parts[2]), int(parts[3])
            break
    raise ValueError(f"{path}: no p cnf header before the clauses")


def clauses_of(path: Path):
    """The clauses of a machine-written DIMACS file (Satsuma's output): the
    literals of the lines after the header, cut at each 0."""
    cur: list[int] = []
    with open(path, "rb") as fh:
        for line in fh:
            s = line.strip()
            if not s or s[:1] in (b"c", b"p"):
                continue
            for tok in s.split():
                lit = int(tok)
                if lit == 0:
                    yield cur
                    cur = []
                else:
                    cur.append(lit)
    if cur:
        raise ValueError(f"{path}: the last clause has no closing 0")


def run_limited(argv: list[str], log: Path, cap: float, mem_mb: int) -> tuple[int | None, float]:
    """argv under a memory limit (ulimit -v, as the harness sets it) and a wall
    cap; the process group is killed at the cap. (None, s) on the cap."""
    cmd = ["bash", "-c", f"ulimit -v {mem_mb * 1024}; exec " + shlex.join(argv)]
    t0 = time.monotonic()
    with open(log, "wb") as fh:
        p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=fh, stderr=subprocess.STDOUT,
                             start_new_session=True)
        try:
            rc = p.wait(timeout=cap)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            p.wait()
            rc = None
    return rc, time.monotonic() - t0


def satsuma_argv(satsuma: str, mode: str, plain: Path, out: Path, proof: Path | None = None) -> list[str]:
    argv = [satsuma, mode, "--file", str(plain), "--out-file", str(out), "--add-reduced-as-unit"]
    if proof is not None:
        argv += ["--proof-file", str(proof), "--veripb"]
    return argv


def classify(rc: int | None, text: str, out: Path) -> str:
    if rc is None:
        return "timeout"
    if rc == 0 and out.is_file():
        try:
            cnf_header(out)
            return "ok"
        except ValueError:
            return "bad-output"
    if "bad_alloc" in text or "cannot allocate" in text.lower() or "out of memory" in text.lower():
        return "memout"
    return f"exit{rc}"


def satsuma_stats(text: str) -> dict:
    row = {}
    for key, rx in STATS.items():
        found = rx.findall(text)
        if found:
            row[key] = int(found[-1])
    if "c UNSATISFIABLE" in text:
        row["decided"] = "UNSAT"
    return row


# ---------------------------------------------------------------------------
# break

def targets(census: Path, cells: list[str] | None) -> list[tuple[str, str]]:
    rows = read_tsv(census)
    if cells:
        want = set(cells)
        out = [(r["stem"], r["family"]) for r in rows if r["stem"] in want]
        missing = want - {s for s, _ in out}
        if missing:
            raise SystemExit(f"not in the census: {sorted(missing)[:5]}")
        return out
    out = []
    for r in rows:
        gens = int(r["sym_gens"]) if r["sym_gens"] else 0
        if gens > 0 or r["sym_status"] in ("too-big", "timeout"):
            out.append((r["stem"], r["family"]))
    return out


def break_one(job: tuple[str, str, str], cfg: dict) -> dict:
    stem, family, mode = job
    suite: Path = cfg["suite"]
    row = {c: "" for c in BREAK_COLS}
    row.update(stem=stem, family=family, mode=mode)
    work = suite / "_work" / f"{stem}.{mode}"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    try:
        plain, out = work / "in.cnf", work / "out.cnf"
        decompress(cfg["source"] / f"{stem}.cnf.xz", plain)
        row["vars_in"], row["clauses_in"] = cnf_header(plain)
        log = suite / "logs" / f"{stem}.{mode}.log"
        rc, dt = run_limited(satsuma_argv(cfg["satsuma"], mode, plain, out), log, cfg["cap"], cfg["mem_mb"])
        text = ANSI.sub("", log.read_text(errors="replace"))
        row["satsuma_s"] = round(dt, 2)
        row["status"] = classify(rc, text, out)
        row.update(satsuma_stats(text))
        if row["status"] == "ok":
            row["vars_out"], row["clauses_out"] = cnf_header(out)
            row["out_mb"] = round(out.stat().st_size / 1e6, 1)
            row["sha256"] = sha256_file(out)
            os.replace(out, suite / f"{stem}.{mode}.cnf")
    except Exception as e:                      # one cell's failure is a row, not a crash
        row["status"] = f"error:{e.__class__.__name__}"
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return row


def write_selection(suite: Path, rows: list[dict]) -> int:
    insts = sorted(f"{r['stem']}.{r['mode']}" for r in rows
                   if r["status"] == "ok" and (suite / f"{r['stem']}.{r['mode']}.cnf").is_file())
    with open(suite / "selection.csv", "w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["instance", "half"])
        for inst in insts:
            w.writerow([inst, inst.rsplit(".", 1)[1]])
    return len(insts)


def satsuma_identity(satsuma: str) -> str:
    try:
        ver = subprocess.run([satsuma, "lex", "--version"], capture_output=True, text=True, timeout=30)
        version = ANSI.sub("", (ver.stdout or "") + (ver.stderr or "")).strip().splitlines()[-1:]
    except (OSError, subprocess.TimeoutExpired):
        version = []
    commit = subprocess.run(["git", "-C", str(Path(satsuma).resolve().parent), "rev-parse", "--short", "HEAD"],
                            capture_output=True, text=True)
    return f"version {' '.join(version) or '?'}, commit {(commit.stdout or '?').strip()}"


def cmd_break(args) -> int:
    suite = Path(args.suite).resolve()
    (suite / "logs").mkdir(parents=True, exist_ok=True)
    tsv = suite / "break.tsv"
    new = not tsv.is_file() or tsv.stat().st_size == 0      # an empty table (killed before its header) is new
    if not new:
        with open(tsv, newline="") as fh:
            header = next(csv.reader(fh, delimiter="\t"), [])
        if header != BREAK_COLS:
            raise SystemExit(f"{tsv} has other columns; use a new --suite")
    rows = [] if new else read_tsv(tsv)
    done = {(r["stem"], r["mode"]) for r in rows}
    modes = [m for m in args.modes.split(",") if m]
    if any(m not in MODES for m in modes):
        raise SystemExit(f"--modes takes {MODES}")
    source = Path(args.source)
    cells = targets(Path(args.census), [c for c in args.cells.split(",") if c] if args.cells else None)
    jobs = [(s, f, m) for s, f in cells for m in modes if (s, m) not in done]
    jobs.sort(key=lambda j: -(source / f"{j[0]}.cnf.xz").stat().st_size)    # the big ones first
    ident = satsuma_identity(args.satsuma)
    with open(suite / "META.txt", "a") as fh:
        fh.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S%z')} break: satsuma {args.satsuma} ({ident}); "
                 f"modes {modes}; cap {args.cap} s; mem {args.mem_mb} MB; jobs {args.jobs}; "
                 f"{len(cells)} targets from {args.census}; {len(jobs)} new runs\n")
    print(f"{len(cells)} targets x {len(modes)} modes: {len(jobs)} runs to do ({len(done)} kept); satsuma {ident}",
          flush=True)
    cfg = {"suite": suite, "source": source, "satsuma": args.satsuma, "cap": args.cap, "mem_mb": args.mem_mb}
    with open(tsv, "a", newline="") as fh, ThreadPoolExecutor(max_workers=args.jobs) as ex:
        w = csv.DictWriter(fh, fieldnames=BREAK_COLS, delimiter="\t")
        if new:
            w.writeheader()
            fh.flush()
        futs = {ex.submit(break_one, j, cfg): j for j in jobs}
        for n, fut in enumerate(as_completed(futs), 1):
            row = fut.result()
            w.writerow(row)
            fh.flush()
            rows.append(row)
            print(f"  {n}/{len(jobs)} {row['mode']} {row['family']} {row['stem'][33:73]}: {row['status']} "
                  f"{row['satsuma_s']} s gens={row['gens']} units={row['units']} lex={row['lex']} "
                  f"clauses {row['clauses_in']}->{row['clauses_out']}", flush=True)
    shutil.rmtree(suite / "_work", ignore_errors=True)
    n = write_selection(suite, rows)
    print(Counter((r["mode"], r["status"]) for r in rows))
    print(f"selection.csv: {n} instances")
    return 0


# ---------------------------------------------------------------------------
# run

def cmd_run(args) -> int:
    suite = Path(args.suite).resolve()
    if not (suite / "selection.csv").is_file():
        raise SystemExit("no selection.csv: run `break` first")
    solver = Path(args.solver)
    if not (ROOT / solver / "build.sh").is_file():
        raise SystemExit(f"{solver} is not a solver directory")
    cmd = ["python3", "tools/feature_ablation.py", "--suite", str(suite), "--solver", str(solver),
           "--arm", f"{PROBE_ARMS[0]}:", "--arm", f"{PROBE_ARMS[1]}:SAT_EXTRA_ARGS=--seed=1",
           "--seeds", "1", "--jobs", str(args.jobs), "--mem-mb", str(args.mem_mb),
           "--timeout", str(int(args.limit)), "--name", args.name]
    print(shlex.join(cmd), flush=True)
    if args.dry_run:
        return 0
    return subprocess.call(cmd, cwd=ROOT)


# ---------------------------------------------------------------------------
# report

def load_arm(run: Path, arm: str) -> dict[str, dict]:
    p = run / arm / "results.tsv"
    if not p.is_file():
        raise SystemExit(f"missing {p}")
    return {r["instance"]: r for r in read_tsv(p)}


def check_budget(arms: list[dict[str, dict]], limit: float, what: str) -> None:
    """Every row of a run must have had at least `limit` seconds (its `timeout`
    column): a longer limit than a side ran with would count that side's
    timeouts against a budget it never had."""
    budgets = {fnum(r.get("timeout")) for arm in arms for r in arm.values()} - {None}
    if budgets and min(budgets) < limit:
        raise SystemExit(f"{what} ran with {min(budgets):.0f} s, less than --limit {limit:.0f}")


MEMOUT_NOTE = re.compile(r"memory allocation of .* failed|out of memory", re.I)


def stop_reason(r: dict) -> str:
    """Why a run ended, from feature_ablation's result and note: 'answer' (SAT
    or UNSAT and a normal exit), 'budget' (the time or memory limit, an honest
    stop priced into the metric) or 'failure:<why>' (a crash, or an UNKNOWN the
    solver gave on its own before the limit: correctness failures). The
    harness writes TIMEOUT when the limit struck before any status line,
    `exit rc=124 after the status line` when it struck while the answer was
    being printed (a model cut short), `exit rc=N after the status line` for
    another abnormal exit after one, and UNKNOWN_rc<N> for an exit without one."""
    res, note = r.get("result", ""), r.get("note") or ""
    if res == "TIMEOUT" or note.startswith("exit rc=124 "):
        return "budget"
    if res.startswith("UNKNOWN_rc"):
        if MEMOUT_NOTE.search(note):
            return "budget"
        return f"failure:{res}" + (f" ({note})" if note else "")
    if note.startswith("exit rc="):
        return f"failure:{note}"
    if res in SOLVED:
        return "answer"
    return f"failure:{res or 'no status line'} before the limit"


def outcome(r: dict, extra_s: float, limit: float) -> dict:
    """One run as the pipeline sees it: Satsuma's seconds first, then the
    solver's. Only an answer with a normal exit is a solve; a FAIL verdict
    or a failure stop is never one (and is listed), a budget stop is an
    honest unsolved run (its verdict is not a FAIL, even when the kill cut a
    model short)."""
    t = fnum(r.get("time_s")) or 0.0
    total = extra_s + t
    ver = r.get("verified", "")
    res = r.get("result", "")
    note = r.get("note") or ""
    stop = stop_reason(r)
    if stop.startswith("failure:"):
        ver = "FAIL:" + stop[len("failure:"):]
    elif stop == "answer" and ver not in CHECKED and not ver.startswith("FAIL"):
        ver = f"FAIL:{ver or 'no check verdict'}"     # no-proof, checker-error, no-checker, skip, off
    elif stop == "budget":
        ver = "skip"
        if res in SOLVED:
            res = "TIMEOUT"     # the answer was cut short by the limit: no answer
    solved = stop == "answer" and total <= limit and not ver.startswith("FAIL")
    return {"result": res, "solved": solved, "s": total, "solver_s": t, "work": fnum(r.get("work")),
            "verified": ver, "note": note, "cost": total if solved else 2 * limit}


def mark_contradictions(entries: list[dict], limit: float, known: dict[str, set]) -> None:
    """A cell answered SAT somewhere and UNSAT somewhere else, over the
    record's seeds and every mode and seed of the probe. A faulty
    preprocessor turns a satisfiable formula into an unsatisfiable output
    this way, and the run's own gate cannot see it (it checks the answer
    against Satsuma's output, not the original). Every probe answer on such
    a cell is a FAIL, not a solve, and `check` reruns it. known: every answer
    of the runs, gathered before any row was skipped (a partial report keeps
    the record's answer of a seed whose probe row is missing)."""
    answers = defaultdict(set, {s: set(a) for s, a in known.items()})
    for e in entries:
        for side in ("base", "probe"):
            if e[side]["result"] in SOLVED:
                answers[e["stem"]].add(e[side]["result"])
    for e in entries:
        e["contradiction"] = len(answers[e["stem"]]) > 1
        if e["contradiction"] and not e["fallback"]:
            p = e["probe"]
            if not p["verified"].startswith("FAIL"):
                p["verified"] = "FAIL:contradiction (the cell is answered both SAT and UNSAT)"
            p["solved"], p["cost"] = False, 2 * limit


def join(break_rows: list[dict], record: list[dict], probe: list[dict] | None, limit: float):
    """One entry per cell, mode and seed: the record's outcome and the
    probe's, with contradicting answers marked. probe None: Satsuma ran
    nowhere yet (selftest of the fallback)."""
    out, missing = [], []
    known: dict[str, set] = defaultdict(set)      # every answer, whatever is missing
    for b in break_rows:
        for seed in (0, 1):
            rows = [record[seed].get(b["stem"])]
            if b["status"] == "ok" and probe:
                rows.append(probe[seed].get(f"{b['stem']}.{b['mode']}"))
            for r in rows:
                if r is not None and outcome(r, 0.0, limit)["result"] in SOLVED:
                    known[b["stem"]].add(outcome(r, 0.0, limit)["result"])
    for b in break_rows:
        stem, mode = b["stem"], b["mode"]
        s = fnum(b["satsuma_s"]) or 0.0
        for seed in (0, 1):
            base = record[seed].get(stem)
            if base is None:
                missing.append(f"record:{RECORD_ARMS[seed]}:{stem}")
                continue
            e = {"stem": stem, "family": b["family"], "mode": mode, "seed": seed, "status": b["status"],
                 "satsuma_s": s, "base": outcome(base, 0.0, limit)}
            if b["status"] == "ok":
                pr = probe[seed].get(f"{stem}.{mode}") if probe else None
                if pr is None:
                    missing.append(f"probe:{PROBE_ARMS[seed]}:{stem}.{mode}")
                    continue
                e["probe"], e["fallback"] = outcome(pr, s, limit), False
            else:
                # Satsuma did not finish: the original formula after the seconds it spent
                e["probe"], e["fallback"] = outcome(base, s, limit), True
            out.append(e)
    mark_contradictions(out, limit, known)
    return out, missing


def summarize(entries: list[dict], mode: str, seed: int, limit: float) -> dict:
    es = [e for e in entries if e["mode"] == mode and e["seed"] == seed]
    gains = [e for e in es if e["probe"]["solved"] and not e["base"]["solved"]]
    losses = [e for e in es if e["base"]["solved"] and not e["probe"]["solved"]]
    ratios = [e["probe"]["work"] / e["base"]["work"] for e in es
              if e["base"]["solved"] and e["probe"]["solved"] and not e["fallback"]
              and e["base"]["work"] and e["probe"]["work"]]
    return {
        "n": len(es),
        "base": sum(e["base"]["solved"] for e in es),
        "probe": sum(e["probe"]["solved"] for e in es),
        "gains": gains, "losses": losses,
        "probe_checks": Counter(e["probe"]["verified"] for e in es if e["probe"]["solved"]),
        "fails": [e for e in es if e["probe"]["verified"].startswith("FAIL")],
        "base_fails": [e for e in es if e["base"]["verified"].startswith("FAIL")],
        "contradictions": [e for e in es if e["contradiction"]],
        "fallback": sum(e["fallback"] for e in es),
        "par2_base": sum(e["base"]["cost"] for e in es),
        "par2_probe": sum(e["probe"]["cost"] for e in es),
        "work_geo": math.exp(sum(math.log(r) for r in ratios) / len(ratios)) if ratios else float("nan"),
        "work_n": len(ratios),
    }


def short(stem: str) -> str:
    return stem[33:75] if len(stem) > 33 and stem[32] == "-" else stem[:42]


def cmd_report(args) -> int:
    suite = Path(args.suite).resolve()
    rows = read_tsv(suite / "break.tsv")
    record = [load_arm(Path(args.record), a) for a in RECORD_ARMS]
    probe = [load_arm(Path(args.run), a) for a in PROBE_ARMS]
    check_budget(record, args.limit, f"the record {args.record}")
    check_budget(probe, args.limit, f"the probe {args.run}")
    entries, missing = join(rows, record, probe, args.limit)
    if missing:
        print(f"MISSING {len(missing)} results, e.g. {missing[:4]}")
        if not args.partial:
            return 1
    modes = [m for m in MODES if any(r["mode"] == m for r in rows)]
    stems = {r["stem"] for r in rows}
    print(f"{len(stems)} target cells; limit {args.limit:.0f} s including Satsuma's time; "
          f"record {args.record}; probe {args.run}")
    for m in modes:
        mr = [r for r in rows if r["mode"] == m]
        ts = sorted(float(r["satsuma_s"]) for r in mr if r["satsuma_s"])
        print(f"\nSatsuma {m}: {Counter(r['status'] for r in mr)}; seconds median "
              f"{ts[len(ts) // 2] if ts else 0:.1f}, max {max(ts) if ts else 0:.1f}, total {sum(ts):.0f}; "
              f"decided {sum(r['decided'] == 'UNSAT' for r in mr)} UNSAT; with units "
              f"{sum(int(r['units'] or 0) > 0 for r in mr)}, with lex clauses {sum(int(r['lex'] or 0) > 0 for r in mr)}")
    print("\nsolved on the targets (record = default solver on the original formula)")
    print(f"{'mode':5} {'seed':>4} {'n':>4} {'record':>6} {'probe':>6} {'+':>3} {'-':>3} {'fallback':>8} "
          f"{'PAR-2 ratio':>11} {'work geomean':>12} {'(n)':>5}")
    for m in modes:
        for seed in (0, 1):
            s = summarize(entries, m, seed, args.limit)
            par2 = s["par2_probe"] / s["par2_base"] if s["par2_base"] > 0 else float("nan")
            print(f"{m:5} {seed:>4} {s['n']:>4} {s['base']:>6} {s['probe']:>6} {len(s['gains']):>3} "
                  f"{len(s['losses']):>3} {s['fallback']:>8} {par2:>11.3f} "
                  f"{s['work_geo']:>12.3f} {s['work_n']:>5}   probe solves by check: {dict(s['probe_checks'])}")
            for e in s["fails"]:
                print(f"   FAIL {m} seed {seed} {e['family']} {short(e['stem'])}: {e['probe']['verified']}")
            for e in s["base_fails"]:
                print(f"   RECORD FAIL seed {seed} {e['family']} {short(e['stem'])}: {e['base']['verified']}")
    noted = [e for e in entries if not e["fallback"] and e["probe"]["note"] and e["probe"]["solved"]]
    for e in noted:     # a solve with a stderr note that is not an abnormal exit: shown, for a person to read
        print(f"   NOTE {e['mode']} seed {e['seed']} {short(e['stem'])}: {e['probe']['note'][:100]}")
    # per family, both seeds together; each mode against the record on that mode's own entries
    fam = defaultdict(lambda: defaultdict(int))
    for e in entries:
        f = fam[e["family"]]
        f[f"n:{e['mode']}"] += 1
        f[f"base:{e['mode']}"] += e["base"]["solved"]
        f[e["mode"]] += e["probe"]["solved"]
    print("\nper family, two seeds together, record -> probe for each mode (cells x seeds)")
    for name in sorted(fam, key=lambda k: (-max(fam[k][f"n:{m}"] for m in modes), k)):
        f = fam[name]
        moved = any(f[m] != f[f"base:{m}"] for m in modes)
        if moved or args.all_families:
            print(f"  {name:24} " + "   ".join(f"{m} {f[f'base:{m}']:>3} -> {f[m]:>3} of {f[f'n:{m}']:>3}"
                                            for m in modes))
    print("\ncells whose answer changes (seconds include Satsuma's)")
    for e in sorted(entries, key=lambda e: (e["family"], e["stem"], e["mode"], e["seed"])):
        b, p = e["base"], e["probe"]
        if b["solved"] != p["solved"] or e["contradiction"]:
            tag = "CONTRADICTION" if e["contradiction"] else "GAIN" if p["solved"] else "LOSS"
            print(f"  {tag} {e['mode']} seed {e['seed']} {e['family']:18} {short(e['stem']):42} "
                  f"record {b['result'][:5]} {b['s']:7.1f} -> {p['result'][:5]} {p['s']:7.1f} "
                  f"(Satsuma {e['satsuma_s']:.1f}{', fallback ' + e['status'] if e['fallback'] else ''})")
    if args.out:
        write_table(Path(args.out), rows, entries)
        print(f"\nwrote {args.out}")
    return 0


def write_table(path: Path, rows: list[dict], entries: list[dict]) -> None:
    by = {(e["stem"], e["mode"], e["seed"]): e for e in entries}
    cols = ["stem", "family", "mode", "status", "satsuma_s", "decided", "gens", "row", "row_column", "johnson",
            "units", "binary", "lex", "clauses_in", "clauses_out"]
    for seed in (0, 1):
        cols += [f"record{seed}", f"record{seed}_s", f"record{seed}_work",
                 f"probe{seed}", f"probe{seed}_s", f"probe{seed}_work", f"probe{seed}_verified"]
    with open(path, "w", newline="") as fh:
        w = csv.writer(fh, delimiter="\t")
        w.writerow(cols)
        for r in sorted(rows, key=lambda r: (r["family"], r["stem"], r["mode"])):
            line = [r[c] for c in cols[:15]]
            for seed in (0, 1):
                e = by.get((r["stem"], r["mode"], seed))
                if e is None:
                    line += [""] * 7
                    continue
                b, p = e["base"], e["probe"]
                line += [b["result"] if b["solved"] or b["result"] not in SOLVED else "LATE",
                         f"{b['s']:.1f}", "" if b["work"] is None else f"{b['work']:.0f}",
                         ("FALLBACK:" if e["fallback"] else "") +
                         (p["result"] if p["solved"] or p["result"] not in SOLVED else "LATE"),
                         f"{p['s']:.1f}", "" if p["work"] is None or e["fallback"] else f"{p['work']:.0f}",
                         "" if e["fallback"] else p["verified"]]
            w.writerow(line)


# ---------------------------------------------------------------------------
# check

BOGUS_ID = 999_999_999      # more constraints than any proof here has: VeriPB rejects a reference to it


def stage_proof(src: Path, fprime: Path, dst: Path, control: bool = False, refuted: bool = False) -> None:
    """Satsuma's proof as one VeriPB 3 stream: the first header only (Satsuma
    writes one per round), every deletion once, its own ending dropped; then
    one `rup` line per clause of its output and an ending: `conclusion UNSAT`
    when Satsuma refuted the formula (refuted), no conclusion otherwise. The
    negative control (control) adds `pol` on a constraint that does not exist
    before the ending. VeriPB rejects that line whatever the database holds,
    even after a contradiction, where any implied line passes (a unit on a
    fresh variable included)."""
    deleted: set[str] = set()
    first = True
    with open(src, errors="replace") as fh, open(dst, "w") as out:
        for line in fh:
            s = line.strip()
            if s.startswith("pseudo-Boolean proof version"):
                if first:
                    out.write(s + "\n")
                    first = False
                continue
            if first and s:
                raise ValueError("Satsuma's proof does not start with a header")
            if s.startswith(("output ", "conclusion ", "end pseudo-Boolean proof")):
                continue
            if s.startswith("delc ") or s.startswith("del id "):
                ids = s.split()[2 if s.startswith("del id ") else 1:]
                ids = [i.rstrip(";") for i in ids if i.rstrip(";")]
                keep = [i for i in ids if i not in deleted]
                deleted.update(keep)
                if keep:
                    out.write("del id " + " ".join(keep) + " ;\n")
                continue
            out.write(line if line.endswith("\n") else line + "\n")
        if first:
            raise ValueError("Satsuma's proof is empty")
        for clause in clauses_of(fprime):
            out.write("rup " + "".join(f"1 {'~' if l < 0 else ''}x{abs(l)} " for l in clause) + ">= 1 ;\n")
        if control:
            out.write(f"pol {BOGUS_ID} ;\n")
        out.write("output NONE ;\n" + ("conclusion UNSAT ;\n" if refuted else "conclusion NONE ;\n")
                  + "end pseudo-Boolean proof ;\n")


def normalized_cnf(src: Path, dst: Path) -> None:
    """The formula as the solver reads it (proof_gate.iter_clauses, kissat's
    rules), one clause per line in the same order under an exact header, for
    VeriPB: its CNF reader rejects layouts the solver and Satsuma accept (a
    blank line before the header, tested). Every clause stays in place, the
    empty clause included, so constraint ids keep Satsuma's numbering."""
    body = dst.with_name(dst.name + ".body")
    n = maxvar = 0
    gen = proof_gate.iter_clauses(src)
    with open(body, "w") as fh:
        while True:
            try:
                cl = next(gen)
            except StopIteration as stop:
                header_vars = stop.value or 0
                break
            n += 1
            if cl:
                maxvar = max(maxvar, max(abs(l) for l in cl))
            fh.write(" ".join(map(str, cl)) + " 0\n")
    with open(dst, "w") as out, open(body) as fh:
        out.write(f"p cnf {max(header_vars, maxvar)} {n}\n")
        shutil.copyfileobj(fh, out, 1 << 24)
    body.unlink()


def veripb_run(veripb: str, plain: Path, proof: Path, timeout: float) -> tuple[int, list[str], str]:
    p = subprocess.run([veripb, str(plain), str(proof)], capture_output=True, text=True, timeout=timeout)
    text = (p.stdout or "") + "\n" + (p.stderr or "")
    return p.returncode, [l.strip() for l in text.splitlines() if l.strip()], text


def satsuma_stage(plain: Path, pbp: Path, fprime: Path, veripb: str, timeout: float, d: Path,
                  refuted: bool = False) -> str:
    """ok when every clause of Satsuma's output follows from the input and
    what Satsuma's proof derived (refuted: and VeriPB itself concludes UNSAT),
    and the negative control is rejected. ok-weaker: VeriPB passed under its
    weaker guarantee (a deletion it could not check; satisfiability is still
    preserved, which is what UNSAT needs)."""
    good, bad = d / "stage.pbp", d / "stage-bad.pbp"
    want = "s VERIFIED UNSATISFIABLE" if refuted else "s VERIFIED NO CONCLUSION"
    try:
        stage_proof(pbp, fprime, good, refuted=refuted)
        stage_proof(pbp, fprime, bad, control=True, refuted=refuted)
    except (OSError, ValueError) as e:
        return f"FAIL:{e}"
    try:
        rc, lines, text = veripb_run(veripb, plain, good, timeout)
        if rc != 0 or want not in lines:
            why = next((l for l in reversed(lines) if not l.startswith("c ")), "")
            return f"FAIL:VeriPB rejected ({why[:120]})"
        why = "ok-weaker" if "weaker guarantee" in text else "ok"
        rc_bad, lines_bad, _ = veripb_run(veripb, plain, bad, timeout)
        if rc_bad == 0 or any(l.startswith("s VERIFIED") for l in lines_bad):
            return "FAIL:VeriPB accepted the negative control (the lines after Satsuma's proof were not read)"
        return why
    except subprocess.TimeoutExpired:
        return "checker-timeout"


def check_one(stem: str, mode: str, seed: int, cfg: dict, measured: str = "") -> dict:
    """The answer again, end to end. measured: the run's answer for this cell,
    mode and seed (empty when not known); a rerun that disagrees is a FAIL."""
    out = check_rerun(stem, mode, seed, cfg)
    out["measured"] = measured or "n/a"
    if measured in SOLVED and out["result"] in SOLVED and out["result"] != measured:
        out["verdict"] = f"FAIL:the rerun answered {out['result']}, the measured run {measured}"
    return out


def check_rerun(stem: str, mode: str, seed: int, cfg: dict) -> dict:
    out = {"stem": stem, "mode": mode, "seed": seed, "same_output": "", "result": "", "satsuma_proof": "",
           "solver_proof": "", "model": "", "verdict": ""}
    d = Path(tempfile.mkdtemp(prefix="symext-check-", dir=cfg["tmp"]))
    try:
        plain, fprime, pbp = d / "in.cnf", d / "out.cnf", d / "satsuma.pbp"
        decompress(cfg["source"] / f"{stem}.cnf.xz", plain)
        vplain = d / "in.veripb.cnf"            # the same clauses, laid out for VeriPB's reader
        normalized_cnf(plain, vplain)
        rc, _ = run_limited(satsuma_argv(cfg["satsuma"], mode, plain, fprime, pbp), d / "satsuma.log",
                            cfg["cap"], cfg["mem_mb"])
        text = ANSI.sub("", (d / "satsuma.log").read_text(errors="replace"))
        if classify(rc, text, fprime) != "ok":
            out["verdict"] = "satsuma-failed"
            return out
        ref = cfg["suite"] / f"{stem}.{mode}.cnf" if cfg["suite"] else None
        out["same_output"] = ("yes" if sha256_file(ref) == sha256_file(fprime) else "no") \
            if ref is not None and ref.is_file() else "n/a"
        if any(not c for c in clauses_of(fprime)):
            # Satsuma refuted the formula itself: its proof ends in the empty clause,
            # so the first stage settles it and there is nothing for the solver to do
            out["result"] = "UNSATISFIABLE"
            out["satsuma_proof"] = satsuma_stage(vplain, pbp, fprime, cfg["veripb"], cfg["check_timeout"], d,
                                                 refuted=True)
            out["solver_proof"] = "not-needed"
            out["verdict"] = "ok" if out["satsuma_proof"] in ("ok", "ok-weaker") else \
                "FAIL" if out["satsuma_proof"].startswith("FAIL") else "unverified"
            return out
        # the solver as the harness runs it: --seed=N, then the formula and the proof file
        solver_out, proof = d / "solver.out", d / "proof.out"
        rc, _ = run_limited([cfg["solver_bin"], f"--seed={seed}", str(fprime), str(proof)], solver_out,
                            cfg["solver_cap"], cfg["mem_mb"])
        text = solver_out.read_text(errors="replace")
        status = next((l.split()[1] for l in text.splitlines() if l.startswith("s ") and len(l.split()) > 1), "")
        if rc is None:          # killed at the cap: a status line printed before the kill is no answer
            out["result"], out["verdict"] = "TIMEOUT", "unsolved"
            return out
        if status in SOLVED and rc != EXIT_CODE[status]:    # an answer, then an abnormal exit
            out["result"], out["verdict"] = status, f"FAIL:exit rc={rc} after the status line"
            return out
        if status not in SOLVED:
            out["result"] = status or f"rc{rc}"
            if MEMOUT_NOTE.search(text):    # the memory limit: an honest stop
                out["verdict"] = "unsolved"
            else:                           # an UNKNOWN or a crash before the cap: a failure
                out["verdict"] = f"FAIL:{status or 'no status line'} before the cap (rc {rc})"
            return out
        res = status
        out["result"] = res
        if res == "SATISFIABLE":
            try:
                v = subprocess.run([sys.executable, str(ROOT / "tools" / "verify_sat.py"), str(plain),
                                    str(solver_out)], capture_output=True, text=True, timeout=cfg["check_timeout"])
                out["model"] = "ok" if v.returncode == 0 else "FAIL"
                out["verdict"] = "ok" if v.returncode == 0 else "FAIL:the model does not satisfy the original formula"
            except subprocess.TimeoutExpired:
                out["model"], out["verdict"] = "checker-timeout", "unverified"
        else:
            out["satsuma_proof"] = satsuma_stage(vplain, pbp, fprime, cfg["veripb"], cfg["check_timeout"], d)
            out["solver_proof"] = proof_gate.check_unsat(fprime, proof, cfg["check_timeout"], workdir=str(d))
            stages = (out["satsuma_proof"], out["solver_proof"])
            if any(s.startswith("FAIL") for s in stages) or out["solver_proof"] == "no-proof":
                out["verdict"] = "FAIL"     # an UNSAT answer without its proof fails, as in the harness
            elif out["satsuma_proof"] in ("ok", "ok-weaker") and out["solver_proof"] == "ok":
                out["verdict"] = "ok"
            else:
                out["verdict"] = "unverified"
    finally:
        shutil.rmtree(d, ignore_errors=True)
    return out


CHECK_COLS = ["stem", "mode", "seed", "measured", "same_output", "result", "satsuma_proof", "solver_proof",
              "model", "verdict"]


def cmd_check(args) -> int:
    suite = Path(args.suite).resolve() if args.suite else None
    measured: dict[tuple[str, str, int], str] = {}
    if args.run:
        probe = [load_arm(Path(args.run), a) for a in PROBE_ARMS]
        for seed, arm in enumerate(probe):
            for inst, r in arm.items():
                stem, mode = inst.rsplit(".", 1)
                measured[(stem, mode, seed)] = r.get("result", "")
    if args.only:
        picks = []
        for item in args.only.split(","):
            stem, mode, seed = item.rsplit(":", 2)
            picks.append((stem, mode, int(seed)))
    else:
        rows = read_tsv(suite / "break.tsv")
        record = [load_arm(Path(args.record), a) for a in RECORD_ARMS]
        check_budget(record, args.limit, f"the record {args.record}")
        check_budget(probe, args.limit, f"the probe {args.run}")
        entries, missing = join(rows, record, probe, args.limit)
        if missing:
            raise SystemExit(f"MISSING {len(missing)} results, e.g. {missing[:4]}: the gains cannot be listed")
        # the gains, and every probe answer on a cell answered both SAT and UNSAT
        picks = [(e["stem"], e["mode"], e["seed"]) for e in entries if not e["fallback"] and (
                 (e["probe"]["solved"] and not e["base"]["solved"]) or
                 (e["contradiction"] and e["probe"]["result"] in SOLVED))]
    veripb = proof_gate.find_veripb()
    if not veripb or not proof_gate.find_drat_trim():
        raise SystemExit("VeriPB or drat-trim missing (tools/checkers)")
    cfg = {"source": Path(args.source), "suite": suite, "satsuma": args.satsuma, "solver_bin": args.solver_bin,
           "veripb": veripb, "cap": args.cap, "mem_mb": args.mem_mb, "solver_cap": args.solver_cap,
           "check_timeout": args.check_timeout, "tmp": args.tmp}
    print(f"{len(picks)} answers to check end to end", flush=True)
    results = []
    with ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(check_one, *p, cfg, measured.get(p, "")): p for p in picks}
        for fut in as_completed(futs):
            r = fut.result()
            results.append(r)
            print(f"  {r['mode']} seed {r['seed']} {short(r['stem'])}: {r['result']} (run {r['measured']}) -> "
                  f"{r['verdict']} (satsuma proof {r['satsuma_proof'] or '-'}, solver proof "
                  f"{r['solver_proof'] or '-'}, model {r['model'] or '-'}, same output {r['same_output']})",
                  flush=True)
    if args.out:
        with open(args.out, "w", newline="") as fh:
            w = csv.DictWriter(fh, fieldnames=CHECK_COLS, delimiter="\t")
            w.writeheader()
            for r in sorted(results, key=lambda r: (r["stem"], r["mode"], r["seed"])):
                w.writerow(r)
    print(Counter(r["verdict"] for r in results))
    # 1: a check failed; 2: no failure, but not every pick was verified (a checker
    # timeout, a missing proof, a rerun that did not finish); 0: every pick verified
    if any(r["verdict"].startswith("FAIL") for r in results):
        return 1
    return 2 if any(r["verdict"] != "ok" for r in results) else 0


# ---------------------------------------------------------------------------
# selftest

def php(pigeons: int, holes: int) -> str:
    v = lambda p, h: p * holes + h + 1
    cls = [[v(p, h) for h in range(holes)] for p in range(pigeons)]
    for h in range(holes):
        for p in range(pigeons):
            for q in range(p + 1, pigeons):
                cls.append([-v(p, h), -v(q, h)])
    return f"p cnf {pigeons * holes} {len(cls)}\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in cls)


def php_sat_twin(pigeons: int, holes: int) -> str:
    """php with pigeon 0's at-least-one clause swapped for one about pigeon 0
    alone: the same variables and clause count, so a proof's constraint ids
    line up, but satisfiable when pigeons - 1 <= holes (pigeon 0 stays out)."""
    lines = php(pigeons, holes).split("\n")
    assert lines[1] == " ".join(str(h + 1) for h in range(holes)) + " 0"
    lines[1] = "-1 -2 0"
    return "\n".join(lines)


def selftest_report() -> None:
    """The join's rules on made-up rows: Satsuma's seconds count, a fallback
    keeps the record's answer after Satsuma's seconds, FAIL is never a solve."""
    L = 100.0
    rec = lambda res, t, w=10: {"result": res, "time_s": str(t), "work": str(w), "verified": "ok"}
    rows = [
        {"stem": "a", "family": "f", "mode": "lex", "status": "ok", "satsuma_s": "5"},
        {"stem": "b", "family": "f", "mode": "lex", "status": "ok", "satsuma_s": "30"},
        {"stem": "c", "family": "g", "mode": "lex", "status": "timeout", "satsuma_s": "40"},
        {"stem": "d", "family": "g", "mode": "lex", "status": "ok", "satsuma_s": "1"},
        {"stem": "e", "family": "g", "mode": "lex", "status": "ok", "satsuma_s": "1"},
    ]
    record = [{"a": rec("TIMEOUT", 100), "b": rec("SATISFIABLE", 20), "c": rec("UNSATISFIABLE", 70),
               "d": rec("SATISFIABLE", 10, 40), "e": rec("TIMEOUT", 100)},
              {"a": rec("TIMEOUT", 100), "b": rec("SATISFIABLE", 20), "c": rec("UNSATISFIABLE", 50),
               "d": rec("SATISFIABLE", 10, 40), "e": rec("TIMEOUT", 100)}]
    fail = dict(rec("UNSATISFIABLE", 3), verified="FAIL:drat-trim rejected the proof")
    probe = [{"a.lex": rec("UNSATISFIABLE", 50), "b.lex": rec("SATISFIABLE", 80), "d.lex": rec("SATISFIABLE", 5, 10),
              "e.lex": fail},
             {"a.lex": rec("UNSATISFIABLE", 96), "b.lex": rec("SATISFIABLE", 60), "d.lex": rec("SATISFIABLE", 5, 10),
              "e.lex": rec("TIMEOUT", 100)}]
    entries, missing = join(rows, record, probe, L)
    assert not missing, missing
    s0, s1 = summarize(entries, "lex", 0, L), summarize(entries, "lex", 1, L)
    # seed 0: a gained (5+50); b lost (30+80 > 100); c falls back: 40+70 > 100, lost; d same; e FAIL is no solve
    assert [e["stem"] for e in s0["gains"]] == ["a"], s0["gains"]
    assert sorted(e["stem"] for e in s0["losses"]) == ["b", "c"], s0["losses"]
    assert [e["stem"] for e in s0["fails"]] == ["e"] and s0["fallback"] == 1
    assert s0["base"] == 3 and s0["probe"] == 2, (s0["base"], s0["probe"])
    # seed 1: a is 5+96 > 100, not a gain; b is 30+60, kept; c falls back at 40+50, kept
    assert not s1["gains"] and not s1["losses"] and s1["base"] == s1["probe"] == 3, s1
    assert abs(s0["work_geo"] - 0.25) < 1e-9 and s0["work_n"] == 1, s0["work_geo"]
    assert s0["par2_probe"] == 55 + 2 * L + 2 * L + 6 + 2 * L, s0["par2_probe"]
    # a missing probe row is reported, not taken as a fallback
    _, miss = join(rows, record, [dict(probe[0], **{}), {}], L)
    assert any(m.startswith("probe:s1:") for m in miss), miss
    # a contradiction (record SAT, probe UNSAT) and an answer followed by a crash
    # (feature_ablation's note): never a solve, always listed
    rows2 = [{"stem": "f", "family": "h", "mode": "fix", "status": "ok", "satsuma_s": "1"},
             {"stem": "g", "family": "h", "mode": "fix", "status": "ok", "satsuma_s": "1"}]
    record2 = [{"f": rec("SATISFIABLE", 10), "g": rec("TIMEOUT", 100)},
               {"f": rec("SATISFIABLE", 10), "g": rec("TIMEOUT", 100)}]
    crash = dict(rec("UNSATISFIABLE", 5), note="exit rc=-6 after the status line; panicked at x")
    probe2 = [{"f.fix": rec("UNSATISFIABLE", 5), "g.fix": crash},
              {"f.fix": rec("UNSATISFIABLE", 5), "g.fix": rec("UNSATISFIABLE", 5)}]
    entries2, missing2 = join(rows2, record2, probe2, L)
    assert not missing2, missing2
    t0, t1 = summarize(entries2, "fix", 0, L), summarize(entries2, "fix", 1, L)
    assert t0["base"] == 1 and t0["probe"] == 0 and not t0["gains"], t0
    assert [e["stem"] for e in t0["losses"]] == ["f"], t0["losses"]
    assert sorted(e["stem"] for e in t0["fails"]) == ["f", "g"], t0["fails"]
    assert [e["stem"] for e in t0["contradictions"]] == ["f"], t0["contradictions"]
    assert [e["stem"] for e in t1["gains"]] == ["g"] and [e["stem"] for e in t1["losses"]] == ["f"], t1
    # stops: a kill while printing is a budget stop, never a FAIL (even when the
    # cut model fails its check); a memory abort is a budget stop; a panic, an
    # abnormal exit after the answer and an UNKNOWN before the limit are FAILs
    o = lambda **kw: outcome(dict(rec("SATISFIABLE", 50), **kw), 0.0, L)
    cut = o(note="exit rc=124 after the status line", verified="FAIL:clause 3 not satisfied")
    assert not cut["solved"] and cut["verified"] == "skip" and cut["result"] == "TIMEOUT", cut
    mem = o(result="UNKNOWN_rc-6", verified="skip", note="memory allocation of 8 bytes failed")
    assert not mem["solved"] and not mem["verified"].startswith("FAIL"), mem
    for bad in (dict(result="UNKNOWN_rc-6", verified="skip", note="thread 'main' panicked at x"),
                dict(note="exit rc=-6 after the status line"), dict(result="UNKNOWN", verified="skip"),
                dict(result="UNKNOWN_rc1", verified="skip", note="error: bad header"),
                dict(result="UNSATISFIABLE", verified="no-proof"), dict(verified="checker-error"),
                dict(verified="skip")):
        x = o(**bad)
        assert not x["solved"] and x["verified"].startswith("FAIL"), (bad, x)
    assert o()["solved"] and o(result="TIMEOUT", verified="skip")["verified"] == "skip"
    assert o(result="UNSATISFIABLE", verified="checker-timeout")["solved"]
    # a partial report keeps the record's answer of a seed whose probe row is missing:
    # record SAT on seed 0, probe missing on seed 0 and UNSAT on seed 1 is a contradiction
    rows3 = [{"stem": "k", "family": "h", "mode": "lex", "status": "ok", "satsuma_s": "1"}]
    entries3, missing3 = join(rows3, [{"k": rec("SATISFIABLE", 10)}, {"k": rec("TIMEOUT", 100)}],
                              [{}, {"k.lex": rec("UNSATISFIABLE", 5)}], L)
    assert missing3 and len(entries3) == 1 and entries3[0]["contradiction"], entries3
    assert not entries3[0]["probe"]["solved"] and entries3[0]["probe"]["verified"].startswith("FAIL")
    # a limit longer than a side's budget is refused
    try:
        check_budget([{"a": {"timeout": "50"}}], L, "a toy run")
    except SystemExit:
        pass
    else:
        raise AssertionError("a run with a 50 s budget passed a 100 s limit")
    print("selftest report: ok")


def selftest_toys(satsuma: str, solver_bin: str) -> None:
    veripb = proof_gate.find_veripb()
    assert veripb and proof_gate.find_drat_trim(), "VeriPB or drat-trim missing (tools/checkers)"
    with tempfile.TemporaryDirectory(prefix="symext-selftest-") as tmp:
        tmp = Path(tmp)
        src = tmp / "src"
        src.mkdir()
        toys = {"php-6-5": (php(6, 5), "UNSATISFIABLE"), "php-5-5": (php(5, 5), "SATISFIABLE"),
                # a blank line before the header and a comment: VeriPB's CNF reader rejects
                # the blank line, so this toy verifies only through the normalized copy
                "php-6-5-layout": ("\nc layout toy\n" + php(6, 5), "UNSATISFIABLE")}
        for name, (text, _) in toys.items():
            with lzma.open(src / f"{name}.cnf.xz", "wt") as fh:
                fh.write(text)
        cfg = {"source": src, "suite": None, "satsuma": satsuma, "solver_bin": solver_bin, "veripb": veripb,
               "cap": 60.0, "mem_mb": 4000, "solver_cap": 60.0, "check_timeout": 120.0, "tmp": str(tmp)}
        for mode in MODES:
            for name, (_, want) in toys.items():
                r = check_one(name, mode, 0, cfg)
                print(f"  {mode} {name}: {r}")
                assert r["result"] == want and r["verdict"] == "ok", r
        # a rerun that disagrees with the measured answer fails, however well it verifies
        r = check_one("php-5-5", "lex", 0, cfg, measured="UNSATISFIABLE")
        assert r["verdict"].startswith("FAIL"), r
        print(f"  measured UNSAT, rerun SAT: {r['verdict']}")
        # a rerun that prints an answer and then exits abnormally fails
        fakes = {   # stand-in solvers: each one's rerun must FAIL
            "answer, then exit 3": ("php-5-5", f'"{solver_bin}" "$@"\nexit 3'),
            "s UNKNOWN at once": ("php-5-5", 'echo "s UNKNOWN"\nexit 0'),
            "a crash before any answer": ("php-5-5", "exit 134"),
            "UNSAT with its proof thrown away": ("php-6-5", f'args=("$@")\nunset "args[${{#args[@]}}-1]"\n'
                                                            f'"{solver_bin}" "${{args[@]}}" /dev/null'),
        }
        for i, (what, (toy, body)) in enumerate(fakes.items()):
            fake = tmp / f"fake-{i}.sh"
            fake.write_text(f"#!/bin/bash\n{body}\n")
            fake.chmod(0o755)
            r = check_one(toy, "lex", 0, dict(cfg, solver_bin=str(fake)))
            assert r["verdict"].startswith("FAIL"), (what, r)
            print(f"  {what}: {r['verdict'][:80]}")
        # no model satisfies a formula with an empty clause
        empty, m1 = tmp / "empty.cnf", tmp / "m1.out"
        empty.write_text("p cnf 1 2\n1 0\n0\n")
        m1.write_text("s SATISFIABLE\nv 1 0\n")
        v = subprocess.run([sys.executable, str(ROOT / "tools" / "verify_sat.py"), str(empty), str(m1)],
                           capture_output=True, text=True)
        assert v.returncode != 0, "verify_sat accepted a model of a formula with an empty clause"
        # negative controls: a wrong model, an output clause the proof never derived, and
        # (where Satsuma refutes php(6,5), so that every clause follows) its refutation
        # checked against a satisfiable formula of the same shape
        plain = tmp / "php55.cnf"
        plain.write_text(php(5, 5))
        model = tmp / "model.out"
        model.write_text("s SATISFIABLE\nv " + " ".join(str(-x) for x in range(1, 26)) + " 0\n")
        v = subprocess.run([sys.executable, str(ROOT / "tools" / "verify_sat.py"), str(plain), str(model)],
                           capture_output=True, text=True)
        assert v.returncode != 0, "verify_sat accepted an all-false model of php(5,5)"
        for mode in MODES:
            p66, f66, pbp = tmp / "php66.cnf", tmp / f"php66.{mode}.cnf", tmp / f"php66.{mode}.pbp"
            p66.write_text(php(6, 5))
            rc, _ = run_limited(satsuma_argv(satsuma, mode, p66, f66, pbp), tmp / "s.log", 60.0, 4000)
            assert rc == 0, rc
            d = tmp / f"neg-{mode}"
            d.mkdir()
            if any(not c for c in clauses_of(f66)):
                twin = tmp / "php65-sat-twin.cnf"
                twin.write_text(php_sat_twin(6, 5))
                verdict = satsuma_stage(twin, pbp, f66, veripb, 120.0, d, refuted=True)
                what = "the refutation passed against a satisfiable formula"
            else:
                with open(f66, "a") as fh:            # not implied: x1 alone
                    fh.write("1 0\n")
                text = f66.read_text().split("\n", 1)
                n, m = map(int, text[0].split()[2:4])
                f66.write_text(f"p cnf {n} {m + 1}\n" + text[1])
                verdict = satsuma_stage(p66, pbp, f66, veripb, 120.0, d)
                what = "an underived unit passed the Satsuma stage"
            assert verdict.startswith("FAIL"), f"{mode}: {what} ({verdict})"
            print(f"  negative control {mode} ({'refuted' if 'refutation' in what else 'unit'}): {verdict[:90]}")
    print("selftest toys: ok")


def cmd_selftest(args) -> int:
    selftest_report()
    if args.satsuma:
        selftest_toys(args.satsuma, args.solver_bin)
    else:
        print("selftest toys: skipped (no --satsuma)")
    return 0


# ---------------------------------------------------------------------------

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    default_bin = str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver")

    b = sub.add_parser("break")
    b.add_argument("--satsuma", required=True)
    b.add_argument("--suite", required=True, help="the scratch suite directory to write")
    b.add_argument("--modes", default="fix,lex")
    b.add_argument("--cells", default="", help="comma-separated stems instead of the census targets")
    b.add_argument("--census", default=str(CENSUS))
    b.add_argument("--source", default=str(SOURCE))
    b.add_argument("--cap", type=float, default=300.0, help="Satsuma's wall cap per cell and mode")
    b.add_argument("--mem-mb", type=int, default=16000)
    b.add_argument("--jobs", type=int, default=16)

    r = sub.add_parser("run")
    r.add_argument("--suite", required=True)
    r.add_argument("--solver", required=True, help="a frozen solver directory, e.g. solver/13-j9-symext-snapshot")
    r.add_argument("--jobs", type=int, default=32)
    r.add_argument("--mem-mb", type=int, default=16000)
    r.add_argument("--limit", type=float, default=LIMIT)
    r.add_argument("--name", default="symext-2025")
    r.add_argument("--dry-run", action="store_true")

    p = sub.add_parser("report")
    p.add_argument("--suite", required=True)
    p.add_argument("--run", required=True, help="the feature_ablation log directory with arms s0 and s1")
    p.add_argument("--record", default=str(RECORD))
    p.add_argument("--limit", type=float, default=LIMIT)
    p.add_argument("--out", default="")
    p.add_argument("--partial", action="store_true", help="report even with results missing")
    p.add_argument("--all-families", action="store_true")

    c = sub.add_parser("check")
    c.add_argument("--satsuma", required=True)
    c.add_argument("--suite", default="")
    c.add_argument("--run", default="")
    c.add_argument("--record", default=str(RECORD))
    c.add_argument("--only", default="", help="stem:mode:seed,... instead of the report's gains")
    c.add_argument("--source", default=str(SOURCE))
    c.add_argument("--solver-bin", default=default_bin)
    c.add_argument("--cap", type=float, default=300.0)
    c.add_argument("--mem-mb", type=int, default=16000)
    c.add_argument("--limit", type=float, default=LIMIT, help="the run's limit, to list its gains")
    c.add_argument("--solver-cap", type=float, default=3600.0,
                   help="the rerun's wall cap: it checks the answer, not the time")
    c.add_argument("--check-timeout", type=float, default=7200.0)
    c.add_argument("--jobs", type=int, default=8)
    c.add_argument("--tmp", default=None)
    c.add_argument("--out", default="")

    s = sub.add_parser("selftest")
    s.add_argument("--satsuma", default="")
    s.add_argument("--solver-bin", default=default_bin)

    args = ap.parse_args()
    if args.cmd == "check" and not args.only and not (args.suite and args.run):
        ap.error("check needs --only, or --suite and --run to list the gains")
    return {"break": cmd_break, "run": cmd_run, "report": cmd_report, "check": cmd_check,
            "selftest": cmd_selftest}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
