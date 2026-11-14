#!/usr/bin/env python3
"""Effect test for candidate control sets (bead SAT-playground-d96.8.3).

A shape is a guess: a split pays only when its cases fall cheaply.  For each
cell and candidate source in a candidate census (tools/control_read.py census
--cands), this runs the split from outside the solver (tools/control_census.py
split) and compares its cost with the plain run's:

  cases    the first j <= 6 variables of the set, every value (2^j cases), or
           a choice group: one case per member set true and one with all false
  limit    each case runs under a work limit L = P / 64, kept within
           [0.1G, 3G], P the plain run's work (the shipped solver's last
           400-cell run, the two seeds' mean); a case that reaches L counts L
  bits     the solver's own probe (--ctlsplit=2, src/control.rs): every value
           in one process, the whole probe within P / 4 (a first level that
           does not fit in a quarter of the plain run cannot pay much)
  groups   the outside split (--group): 16 cases first (all when 16 or
           fewer), all of them only when at least 12 of the 16 are answered
           (on gm16spctrc a sample of 8 missed every hard value, solver README
           "Control words")
  verdict  'decides': every case answered (or a model found) and the total
           work below P; 'deeper': at least 3/4 of the cases answered, the
           rest could be split again; 'no' otherwise

Each split runs in its own folder under --runs (resumable: a rerun reads the
cases it already has).  Results: one row per cell and source in --out.

Usage:
  python3 tools/control_effect.py select --cands TABLE --plain RUN_DIR [--arms pick2,pick3] [--min-work 2e9] > JOBS.tsv
  python3 tools/control_effect.py run --jobs JOBS.tsv --out TABLE --runs DIR --solver BIN --probe-solver BIN
                                      [--cores 0-29] [--probe-cores 0-23]
  python3 tools/control_effect.py selftest

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPLIT = [sys.executable, str(ROOT / "tools" / "control_census.py"), "split"]
MAX_BITS = 6
SAMPLE = 16
GO_ON = 12
FIELDS = ["stem", "source", "set", "cases", "run", "answered", "sat", "limit", "work_sum", "estimate",
          "plain_work", "plain_solved", "verdict", "dir"]


def plain_runs(run_dir, arms=None):
    """stem -> (mean work, seeds solved, seeds) from a feature_ablation run
    folder's arms (results.tsv per arm; only the named arms when given)."""
    out = {}
    for res in sorted(Path(run_dir).glob("*/results.tsv")):
        if arms and res.parent.name not in arms:
            continue
        for r in csv.DictReader(open(res), delimiter="\t"):
            stem = r["instance"]
            w = float(r["work"]) if r.get("work") not in (None, "", "NA") else None
            ok = r["result"] in ("SATISFIABLE", "UNSATISFIABLE")
            ws, solved, n = out.get(stem, ([], 0, 0))
            out[stem] = (ws + ([w] if w is not None else []), solved + ok, n + 1)
    return {s: (sum(ws) / len(ws) if ws else None, solved, n) for s, (ws, solved, n) in out.items()}


def strong(row, source):
    """Is the source's signal strong enough to be worth an effect test?"""
    f = lambda k: float(row[k]) if row.get(k) not in (None, "") else 0.0  # noqa: E731
    if not row.get(f"set_{source}"):
        return False
    if source == "switch":
        return f("muxes") >= 1000 and f("cover16") >= 0.5
    if source == "enable":
        return f("enables") >= 1000 and f("muxes") < 1000
    if source == "guard":
        return f("guard_top") >= 3 * max(1.0, f("guard_median"))
    if source == "group":
        return 2 <= f("group_top_size") <= 63
    if source == "gd":
        return f("gd_share") >= 0.99
    return False


def cmd_select(a):
    plain = plain_runs(a.plain, a.arms.split(",") if a.arms else None)
    w = csv.writer(sys.stdout, delimiter="\t")
    w.writerow(["stem", "source", "set", "plain_work", "plain_solved"])
    for r in csv.DictReader(open(a.cands), delimiter="\t"):
        if r.get("error"):
            continue
        pw, solved, n = plain.get(r["stem"], (None, 0, 0))
        if pw is None or pw < a.min_work:
            continue                     # no plain run, or one too cheap to need a split
        for src in ("switch", "enable", "guard", "group", "gd"):
            if strong(r, src):
                w.writerow([r["stem"], src, r[f"set_{src}"], int(pw), f"{solved}/{n}"])


def split_run(path, args, out, solver, cores, timeout):
    cmd = SPLIT + [str(path)] + args + ["--solver", solver, "--cores", cores, "--seed", "0",
                                        "--mem-mb", "16000", "--timeout", str(timeout), "--out", str(out)]
    p = subprocess.run(cmd, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(f"split failed ({p.returncode}): {p.stderr.strip()[-300:]}")
    rows = [line.split("\t") for line in (out / "results.tsv").read_text().splitlines()]
    return rows


def summarise(rows, limit):
    answered = sum(1 for r in rows if r[1] in ("UNSATISFIABLE", "SATISFIABLE"))
    sat = any(r[1] == "SATISFIABLE" for r in rows)
    # an answered case costs what it reports (it may run past the limit
    # before the limit is polled); one without an answer at most the limit;
    # one without a work count (a crash) at the limit, never as free
    def cost(r):
        if not r[4]:
            return limit
        if r[1] in ("UNSATISFIABLE", "SATISFIABLE"):
            return int(r[4])
        return min(limit, int(r[4]))
    work = sum(cost(r) for r in rows)
    return answered, sat, work


def find_file(stem, suites):
    for d in suites:
        p = Path(d) / f"{stem}.cnf.xz"
        if p.exists():
            return p
    return None


def probe_run(path, bits, limit, total, solver, core, timeout):
    """The solver's own probe (--ctlsplit=2, one level): every value of the
    bits a case within `limit`, the whole probe within `total`, the search
    after it cut by SAT_LIMIT_TICKS at the same total.  Returns the exit
    line's counts and the status."""
    env = dict(os.environ)
    env["SAT_CTL_VARS"] = ",".join(map(str, bits))
    env["SAT_LIMIT_TICKS"] = str(int(total) + 1)
    for k in ("SAT_POLICY",):
        env.pop(k, None)
    import resource

    def cap():
        lim = 16000 * 1024 * 1024        # the repo's 16 GB a solver process
        resource.setrlimit(resource.RLIMIT_AS, (lim, lim))
    cmd = ["timeout", str(timeout), "taskset", "-c", str(core), solver, "--seed=0", "--ctlsplit=2",
           f"--ctlsplitbits={len(bits)}", "--ctlsplitlevels=1", f"--ctlsplitcase={max(1, limit // 1000)}",
           f"--ctlsplitticks={max(1, int(total) // 1000)}", str(path)]
    p = subprocess.run(cmd, capture_output=True, text=True, env=env, preexec_fn=cap)
    counts = {}
    status = ""
    clock = None
    for line in p.stdout.splitlines():
        if line.startswith("s "):
            status = line[2:].strip()
        if line.startswith("c control ctlsplit="):
            for tok in line.split()[2:]:
                k, _, v = tok.partition("=")
                counts[k] = v
        if line.startswith("c workclock "):
            for tok in line.split()[2:]:
                k, _, v = tok.partition("=")
                if k == "work" and v.isdigit():
                    clock = int(v)
    # a probe stopped by the wall timeout in the middle of a case: its
    # counts miss that case's work, the exit clock (printed at the signal)
    # has it all; the probe had not ended, so its status is unset
    if counts and counts.get("status") in ("off", "") and clock is not None:
        counts["interrupted"] = "1"
        counts["clock"] = str(clock)
    return counts, status


def do_job(a, j, suites, core):
    """One cell and source: the row for the table."""
    path = find_file(j["stem"], suites)
    row = {f: "" for f in FIELDS}
    row.update({"stem": j["stem"], "source": j["source"], "plain_work": j["plain_work"],
                "plain_solved": j["plain_solved"]})
    if path is None:
        row["verdict"] = "no-file"
        return row
    members = [int(x) for x in j["set"].split(",") if x]
    pw = float(j["plain_work"])
    # a case may spend 1/64 of the plain run, the first level a quarter of it:
    # a split whose first level does not fit in that cannot pay much
    limit = int(min(3e9, max(1e8, pw / 64)))
    if j["source"] == "group":
        members = members[:63]
        total = len(members) + 1
        d = Path(a.runs) / j["stem"][:60] / j["source"]
        row.update({"set": ",".join(map(str, members)), "cases": total, "limit": limit, "dir": str(d)})
        try:
            base = ["--group=" + ",".join(map(str, members)), "--ticks", str(limit)]
            if total <= SAMPLE:
                rows = split_run(path, base, d / "all", a.solver, a.cores, a.timeout)
            else:
                rows = split_run(path, base + ["--sample", str(SAMPLE)], d / "sample", a.solver, a.cores, a.timeout)
                ans, sat, _w = summarise(rows, limit)
                if ans >= GO_ON and not sat:
                    rows = split_run(path, base, d / "all", a.solver, a.cores, a.timeout)
        except RuntimeError as e:
            row["verdict"] = f"error: {e}"[:200]
            return row
        ans, sat, work = summarise(rows, limit)
        # a model ends the split: its cost is what was spent, not an estimate
        est = work if sat else work * total / len(rows)
        row.update({"run": len(rows), "answered": ans, "sat": int(sat), "work_sum": work, "estimate": int(est)})
        complete = ans == len(rows) and len(rows) == total
    else:
        bits = [abs(x) for x in members][:MAX_BITS]
        total = 1 << len(bits)
        budget = min(pw / 4, total * limit)
        row.update({"set": ",".join(map(str, bits)), "cases": total, "limit": limit})
        counts, status = probe_run(path, bits, limit, budget, a.probe_solver, core, a.timeout)
        if not counts:
            row["verdict"] = "error: no control line"
            return row
        ran = int(counts.get("cases", 0))
        ans = int(counts.get("refuted", 0)) + int(counts.get("sat", 0))
        sat = counts.get("status") == "sat"
        # the whole work clock, as the plain run's: what ran before the split
        # (structure passes, the walk), the reading, and the cases
        before = int(counts.get("pre", 0)) + int(counts.get("read", 0))
        work = before + int(counts.get("work", 0))
        if counts.get("interrupted"):
            work = int(counts["clock"])          # the unfinished case included
        # a model ends the split: its cost is what was spent, not an estimate;
        # an interrupted probe counts its unfinished case as one of those run
        cases_work = work - before
        est = work if sat or ran >= total else before + cases_work * total / max(1, ran)
        row.update({"run": ran, "answered": ans, "sat": int(sat), "work_sum": work, "estimate": int(est)})
        complete = counts.get("status") == "unsat"
        row["dir"] = f"probe status={counts.get('status')}"
    n_run = int(row["run"]) if row["run"] != "" else 0
    if (complete or sat) and float(row["estimate"]) < pw:
        row["verdict"] = "decides"
    elif n_run and int(row["answered"]) >= 0.75 * total:
        row["verdict"] = "deeper"
    else:
        row["verdict"] = "no"
    return row


def cores_of(text):
    out = []
    for part in text.split(","):
        if "-" in part:
            lo, hi = part.split("-")
            out += list(range(int(lo), int(hi) + 1))
        elif part:
            out.append(int(part))
    return out


def check_probe_solver(path):
    """Refuse a binary without the probe option (an older build)."""
    import tempfile
    if not Path(path).exists():
        sys.exit(f"probe solver {path} does not exist")
    with tempfile.NamedTemporaryFile("w", suffix=".cnf", delete=False) as f:
        f.write("p cnf 1 1\n1 0\n")
        tiny = f.name
    try:
        p = subprocess.run([path, "--ctlsplit=2", tiny], capture_output=True, text=True)
    finally:
        os.unlink(tiny)
    if p.returncode not in (10, 20) or "c control ctlsplit=2" not in p.stdout:
        sys.exit(f"probe solver {path} does not take --ctlsplit=2: {(p.stderr or p.stdout).strip()[:200]}")


def cmd_run(a):
    import queue
    import threading
    check_probe_solver(a.probe_solver)
    suites = a.suites.split(",")
    jobs = list(csv.DictReader(open(a.jobs), delimiter="\t"))
    out = Path(a.out)
    done = set()
    if out.exists():
        done = {(r["stem"], r["source"]) for r in csv.DictReader(open(out), delimiter="\t")}
    jobs = [j for j in jobs if (j["stem"], j["source"]) not in done]
    new = not out.exists()
    lock = threading.Lock()
    with open(out, "a", newline="") as fh:
        w = csv.DictWriter(fh, FIELDS, delimiter="\t")
        if new:
            w.writeheader()

        def emit(j, row):
            with lock:
                w.writerow(row)
                fh.flush()
                print(f"{j['stem'][:50]:50s} {j['source']:7s} cases {row['cases']!s:>3} run {row['run']!s:>3} "
                      f"answered {row['answered']!s:>3} sat {row['sat']} work {float(row['work_sum'] or 0) / 1e9:7.2f}G "
                      f"plain {float(j['plain_work']) / 1e9:7.2f}G {row['verdict']}", flush=True)
        # the bit sets: one probe per core, in parallel
        probes = [j for j in jobs if j["source"] != "group"]
        q = queue.Queue()
        for j in probes:
            q.put(j)

        def worker(core):
            while True:
                try:
                    j = q.get_nowait()
                except queue.Empty:
                    return
                emit(j, do_job(a, j, suites, core))
        threads = [threading.Thread(target=worker, args=(c,)) for c in cores_of(a.probe_cores)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        # the groups: the outside split, one at a time on --cores
        for j in jobs:
            if j["source"] == "group":
                emit(j, do_job(a, j, suites, None))


def selftest():
    # the verdict arithmetic on hand-made case rows
    rows = [["0", "UNSATISFIABLE", "1", "3", "100"], ["1", "UNKNOWN", "9", "40", "1000"],
            ["2", "UNSATISFIABLE", "1", "3", ""]]
    ans, sat, work = summarise(rows, 1000)
    assert (ans, sat, work) == (2, False, 2100), (ans, sat, work)
    rows = [["0", "SATISFIABLE", "1", "3", "50"]]
    assert summarise(rows, 1000) == (1, True, 50)
    # a case answered past its limit costs all of its work
    rows = [["0", "UNSATISFIABLE", "1", "3", "1500"]]
    assert summarise(rows, 1000) == (1, False, 1500)
    # the signal rules
    assert strong({"set_switch": "1", "muxes": "5000", "cover16": "0.7"}, "switch")
    assert not strong({"set_switch": "1", "muxes": "5000", "cover16": "0.2"}, "switch")
    assert strong({"set_guard": "1", "guard_top": "40", "guard_median": "10"}, "guard")
    assert not strong({"set_guard": "1", "guard_top": "20", "guard_median": "10"}, "guard")
    assert strong({"set_gd": "1", "gd_share": "1.0"}, "gd") and not strong({"set_gd": "1", "gd_share": "0.8"}, "gd")
    assert not strong({"set_group": "", "group_top_size": "4"}, "group")
    print("selftest: 10 checks ok")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("select")
    s.add_argument("--cands", required=True)
    s.add_argument("--plain", required=True, help="a run folder with one results.tsv per arm")
    s.add_argument("--min-work", type=float, default=2e9)
    s.add_argument("--arms", default="pick2,pick3", help="the run's arms that are the shipped solver")
    r = sub.add_parser("run")
    r.add_argument("--jobs", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--runs", required=True)
    r.add_argument("--solver", required=True)
    r.add_argument("--cores", default="0-29")
    r.add_argument("--timeout", type=int, default=1800)
    r.add_argument("--suites", default=str(ROOT / "benchmarks" / "sat-comp-2025"))
    r.add_argument("--probe-solver", required=True,
                   help="a solver with --ctlsplit=2 (solver 13 since bead d96.8.5), for the bit sets")
    r.add_argument("--probe-cores", default="0-23", help="one probe per core, in parallel")
    sub.add_parser("selftest")
    a = ap.parse_args(argv)
    if a.cmd == "select":
        cmd_select(a)
    elif a.cmd == "run":
        cmd_run(a)
    else:
        selftest()


if __name__ == "__main__":
    main()
