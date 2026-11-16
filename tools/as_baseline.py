#!/usr/bin/env python3
"""Baselines for the assumptions branch (epic SAT-playground-bp3).

The rule of the branch is "off means off": with no assumptions the solver must
do exactly what main's solver does, step for step. This tool makes that
checkable. It runs one solver binary over a fixed set of cells under a work
limit, so every run ends at the same point however busy the host is, and
writes one row per cell. `compare` then checks two such tables field by field.

Sets:
  stock     the stock path (structure passes and engines off, the options
            tools/parity.py uses against the C kissat) on tests/cnf
  stockmed  the stock path on the 100 medium cells
  default   the shipped defaults on the 100 medium cells
  split     the shipped defaults with the control split on (--ctlsplit=1)
            on the 100 medium cells and the four oisc cells of 2025, where
            the split fires: the one-stage split must stay what it is

Each run is the binary as run.sh calls it, with a proof path, plus -s and
--seed=0. Per cell the table has: the exit code, the answer, the work-clock
line, a hash of the -s counters, a hash of the whole output with times and
paths taken out, a hash of the model, and the size and sha256 of each proof
file (the DRAT file, the .pbp file and the .derived.cnf file). Peak memory and
wall time are recorded too, but they are not exact and `compare` only reports
them.

usage:
  as_baseline.py run --solver BIN --set default --out DIR [--limit-ticks N]
  as_baseline.py compare DIR_A DIR_B
  as_baseline.py renormalize DIR
  as_baseline.py selftest
"""

import argparse
import difflib
import hashlib
import json
import lzma
import os
import queue
import re
import resource
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
PARITY = os.path.join(ROOT, "solver", "13-kissat-rs", "tools", "parity.py")

# 32 jobs on physical cores, the layout of the 32-job harness (CLAUDE.md,
# "Benchmarking operations"): cores 16, 17, 34 and 35 stay free.
DEFAULT_CORES = list(range(0, 16)) + list(range(18, 34))
MEM_BYTES = 16 * 1024 ** 3

# Work limits per set (the work clock W, CLAUDE.md "Evaluation").
DEFAULT_LIMIT = {"stock": 0, "stockmed": 2_000_000_000, "default": 10_000_000_000,
                 "split": 10_000_000_000}
OISC = ["113e692e379518bc3277855277dcc1b8-oisc-subrv-sll-nested-8",
        "d5b11b0b73eb4ecebdbf6ee8143da689-oisc-subrv-and-nested-11",
        "5aed29ce52192a55ffbd2a6f340017e7-oisc-subrv-and-nested-12",
        "053e06424279f5cfec8bb14dfa9af23c-oisc-subrv-and-nested-15"]

EXACT_FIELDS = ["exit", "status", "workclock", "counters_sha", "text_sha", "model_sha",
                "proof_bytes", "proof_sha", "pbp_bytes", "pbp_sha", "derived_bytes", "derived_sha"]
FIELDS = ["cell"] + EXACT_FIELDS + ["ncounters", "maxrss_kb", "wall_s"]

SECTION_RE = re.compile(r"^c ---- \[ (.+?) \] -+$")
STAT_RE = re.compile(r"^c ([a-z_0-9]+):\s+(\d+)")
ROW_TIME_RE = re.compile(r"^\d+\.\d\d$")
# a wall time on an exit line (the free-choice walk prints walk_ms and build_ms)
MS_RE = re.compile(r"^([a-z_]+_ms)=\d+$")


def stock_options():
    """The stock path's options, read from parity.py so there is one list."""
    text = open(PARITY).read()
    m = re.search(r"sextra = extra if args\.structure else \[(.*?)\] \+ extra", text, re.S)
    if not m:
        raise SystemExit(f"cannot find the stock option list in {PARITY}")
    opts = re.findall(r'"(--[a-z0-9]+=\d+)"', m.group(1))
    if len(opts) < 5:
        raise SystemExit(f"stock option list in {PARITY} looks wrong: {opts}")
    return opts


def cells_of(set_name):
    """(cell name, path) pairs of a set, sorted by name."""
    out = []
    if set_name == "stock":
        for sub in ("tests/cnf/sat", "tests/cnf/unsat"):
            d = os.path.join(ROOT, sub)
            for f in sorted(os.listdir(d)):
                if f.endswith(".cnf"):
                    out.append((sub.split("/")[-1] + "/" + f[:-4], os.path.join(d, f)))
    else:
        d = os.path.join(ROOT, "benchmarks", "sat-comp-2025-medium")
        for f in sorted(os.listdir(d)):
            if f.endswith(".cnf.xz"):
                out.append((f[:-len(".cnf.xz")], os.path.join(d, f)))
        if len(out) != 100:
            raise SystemExit(
                f"{d} has {len(out)} cells, expected 100 (in a worktree, run tools/worktree_links.sh first)")
        if set_name == "split":
            full = os.path.join(ROOT, "benchmarks", "sat-comp-2025")
            have = {cell for cell, _ in out}
            for stem in OISC:
                path = os.path.join(full, stem + ".cnf.xz")
                if not os.path.exists(path):
                    raise SystemExit(f"missing {path}")
                if stem not in have:    # one of them is a medium cell already
                    out.append((stem, path))
            out.sort()
    return out


def counters_of(lines):
    """First-column counters of the -s statistics block, as parity.py reads them."""
    stats = {}
    in_stats = False
    for line in lines:
        m = SECTION_RE.match(line)
        if m:
            in_stats = m.group(1) == "statistics"
            continue
        if in_stats:
            s = STAT_RE.match(line)
            if s:
                stats[s.group(1)] = int(s.group(2))
    return stats


def normalize(lines, paths):
    """The output with times, memory and paths taken out.

    What stays is everything the search decides: the report rows (without
    their seconds and MB columns), the answer, the counters, the glue table
    and the exit lines.
    """
    out = []
    section = ""
    for line in lines:
        m = SECTION_RE.match(line)
        if m:
            section = m.group(1)
            out.append(f"[{section}]")
            continue
        if section in ("banner", "proving", "profiling"):
            continue
        if line.startswith("v "):
            continue  # the model has its own hash
        for i, p in enumerate(paths):
            if p and p in line:
                line = line.replace(p, f"<path{i}>")
        if " seconds" in line:
            continue
        if line.startswith("c maximum-resident-set-size:") or line.startswith("c process-time:") \
                or line.startswith("c real-time:"):
            continue
        if section == "statistics":
            s = STAT_RE.match(line)
            if s:
                out.append(f"{s.group(1)}={s.group(2)}")
            continue
        tok = line.split()
        if section == "solving":
            if len(tok) >= 6 and tok[0] == "c" and ROW_TIME_RE.match(tok[2]):
                out.append(" ".join(tok[:2] + tok[4:]))  # drop seconds and MB
                continue
        # One space between words: the report's header lines are laid out
        # by the width of the seconds and MB columns.  And no milliseconds.
        # Both found by running the same build twice: 19 of 200 cells
        # differed in nothing but the spacing, 3 more in a walk_ms field.
        out.append(" ".join(MS_RE.sub(r"\1=<t>", t) for t in tok))
    return out


def sha_text(lines):
    h = hashlib.sha256()
    for line in lines:
        h.update(line.encode())
        h.update(b"\n")
    return h.hexdigest()


def sha_file(path):
    """(size, sha256) of a file, or (-1, 'none') when it does not exist."""
    if not os.path.isfile(path):
        return -1, "none"
    h = hashlib.sha256()
    n = 0
    with open(path, "rb") as f:
        while True:
            block = f.read(1 << 22)
            if not block:
                break
            h.update(block)
            n += len(block)
    return n, h.hexdigest()


def peak_kb(pid, solver):
    """The process's own peak memory in kB, or 0 when it cannot be read.

    Not `ru_maxrss`: a child keeps its parent's size at the fork as its
    maximum, so every small run showed this tool's own few hundred MB. VmHWM
    belongs to the memory image, which is new after the exec; it is read only
    once the process runs the solver."""
    try:
        if os.readlink(f"/proc/{pid}/exe") != solver:
            return 0
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def run_cell(solver, opts, limit, cell, path, out_dir, core, wall_limit):
    safe = cell.replace("/", "__")
    work = os.path.join(out_dir, "_work", safe)
    os.makedirs(work, exist_ok=True)
    if path.endswith(".xz"):
        cnf = os.path.join(work, "in.cnf")
        with lzma.open(path, "rb") as src, open(cnf, "wb") as dst:
            shutil.copyfileobj(src, dst, 1 << 22)
    else:
        cnf = path
    proof = os.path.join(work, "proof.out")
    raw = os.path.join(out_dir, "raw", safe + ".out")
    err = os.path.join(out_dir, "raw", safe + ".err")
    cmd = [solver, "-s", "--seed=0"] + opts + [cnf, proof]
    env = dict(os.environ)
    env.pop("SAT_LIMIT_TICKS", None)
    if limit:
        env["SAT_LIMIT_TICKS"] = str(limit)

    def pre():
        os.sched_setaffinity(0, {core})
        resource.setrlimit(resource.RLIMIT_AS, (MEM_BYTES, MEM_BYTES))

    start = time.time()
    with open(raw, "wb") as fo, open(err, "wb") as fe:
        p = subprocess.Popen(cmd, stdout=fo, stderr=fe, env=env, preexec_fn=pre)
        killed = False
        peak = 0
        real = os.path.realpath(solver)
        while True:
            peak = max(peak, peak_kb(p.pid, real))
            pid, status, usage = os.wait4(p.pid, os.WNOHANG)
            if pid:
                break
            if time.time() - start > wall_limit:
                p.kill()
                killed = True
                pid, status, usage = os.wait4(p.pid, 0)
                break
            time.sleep(0.02)
        p.returncode = os.waitstatus_to_exitcode(status)
    wall = time.time() - start

    lines = open(raw, errors="replace").read().splitlines()
    status_line = "none"
    workclock = "none"
    model = []
    for line in lines:
        if line.startswith("s "):
            status_line = line[2:].strip()
        elif line.startswith("c workclock "):
            workclock = line[len("c workclock "):].strip()
        elif line.startswith("v "):
            model.append(line)
    counters = counters_of(lines)
    norm = normalize(lines, [proof, cnf])
    with open(os.path.join(out_dir, "out", safe + ".txt"), "w") as f:
        f.write("\n".join(norm) + "\n")
    pb, ps = sha_file(proof)
    qb, qs = sha_file(proof + ".pbp")
    db, ds = sha_file(proof + ".derived.cnf")
    shutil.rmtree(work, ignore_errors=True)
    return {
        "cell": cell,
        "exit": "WALL-LIMIT" if killed else str(p.returncode),
        "status": status_line,
        "workclock": workclock,
        "counters_sha": sha_text(f"{k}={v}" for k, v in sorted(counters.items())),
        "text_sha": sha_text(norm),
        "model_sha": sha_text(model) if model else "none",
        "proof_bytes": str(pb), "proof_sha": ps,
        "pbp_bytes": str(qb), "pbp_sha": qs,
        "derived_bytes": str(db), "derived_sha": ds,
        "ncounters": str(len(counters)),
        "maxrss_kb": str(peak),
        "wall_s": f"{wall:.1f}",
    }


def cmd_run(args):
    solver = os.path.abspath(args.solver)
    if not os.access(solver, os.X_OK):
        raise SystemExit(f"not an executable: {solver}")
    opts = stock_options() if args.set in ("stock", "stockmed") else []
    if args.set == "split":
        opts = ["--ctlsplit=1"]
    opts += args.options.split() if args.options else []
    limit = DEFAULT_LIMIT[args.set] if args.limit_ticks is None else args.limit_ticks
    cells = cells_of(args.set)
    out_dir = os.path.abspath(args.out)
    if os.path.exists(os.path.join(out_dir, "results.tsv")):
        raise SystemExit(f"{out_dir} already holds a results.tsv; give a fresh folder")
    for sub in ("raw", "out", "_work"):
        os.makedirs(os.path.join(out_dir, sub), exist_ok=True)
    cores = [int(c) for c in args.cores.split(",")] if args.cores else DEFAULT_CORES
    cores = cores[:args.jobs]
    meta = {
        "set": args.set, "solver": solver, "solver_sha256": sha_file(solver)[1],
        "options": opts, "limit_ticks": limit, "cells": len(cells), "cores": cores,
        "mem_bytes": MEM_BYTES, "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "host": os.uname().nodename, "loadavg_start": os.getloadavg()[0],
    }
    print(f"[as_baseline] set={args.set} cells={len(cells)} limit={limit} jobs={len(cores)} "
          f"solver={solver} sha256={meta['solver_sha256'][:16]}", flush=True)
    pool = queue.Queue()
    for c in cores:
        pool.put(c)

    def job(item):
        cell, path = item
        core = pool.get()
        try:
            return run_cell(solver, opts, limit, cell, path, out_dir, core, args.wall_limit)
        finally:
            pool.put(core)

    rows = []
    t0 = time.time()
    with ThreadPoolExecutor(max_workers=len(cores)) as ex:
        for row in ex.map(job, cells):
            rows.append(row)
            print(f"  {row['cell'][:44]:44s} {row['status']:14s} exit={row['exit']:3s} "
                  f"{row['wall_s']:>7s}s proof={row['proof_bytes']}", flush=True)
    meta["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    meta["elapsed_s"] = round(time.time() - t0, 1)
    rows.sort(key=lambda r: r["cell"])
    with open(os.path.join(out_dir, "results.tsv"), "w") as f:
        f.write("\t".join(FIELDS) + "\n")
        for row in rows:
            f.write("\t".join(row[k] for k in FIELDS) + "\n")
    with open(os.path.join(out_dir, "meta.json"), "w") as f:
        json.dump(meta, f, indent=1)
        f.write("\n")
    shutil.rmtree(os.path.join(out_dir, "_work"), ignore_errors=True)
    by = {}
    for row in rows:
        by[row["status"]] = by.get(row["status"], 0) + 1
    print(f"[as_baseline] done in {meta['elapsed_s']} s: {by}; table {out_dir}/results.tsv", flush=True)
    bad = [r["cell"] for r in rows if r["exit"] == "WALL-LIMIT" or r["workclock"] == "none"]
    if bad:
        print(f"[as_baseline] NOT COMPARABLE, {len(bad)} cells hit the wall limit or printed no "
              f"work-clock line: {bad}", flush=True)
        return 1
    return 0


def read_table(d):
    path = os.path.join(d, "results.tsv") if os.path.isdir(d) else d
    rows = {}
    with open(path) as f:
        head = f.readline().rstrip("\n").split("\t")
        for line in f:
            vals = line.rstrip("\n").split("\t")
            row = dict(zip(head, vals))
            rows[row["cell"]] = row
    return rows


def cmd_compare(args):
    a, b = read_table(args.a), read_table(args.b)
    failures = 0
    for cell in sorted(set(a) | set(b)):
        if cell not in a or cell not in b:
            print(f"DIFF {cell}: only in {'A' if cell in a else 'B'}")
            failures += 1
            continue
        ra, rb = a[cell], b[cell]
        diffs = [k for k in EXACT_FIELDS if ra[k] != rb[k]]
        if not diffs:
            continue
        failures += 1
        print(f"DIFF {cell}: {', '.join(diffs)}")
        for k in ("exit", "status", "workclock"):
            if k in diffs:
                print(f"    A {k}: {ra[k]}")
                print(f"    B {k}: {rb[k]}")
        if "text_sha" in diffs and os.path.isdir(args.a) and os.path.isdir(args.b):
            safe = cell.replace("/", "__") + ".txt"
            pa, pb = os.path.join(args.a, "out", safe), os.path.join(args.b, "out", safe)
            if os.path.isfile(pa) and os.path.isfile(pb):
                la, lb = open(pa).read().splitlines(), open(pb).read().splitlines()
                shown = 0
                for d in difflib.unified_diff(la, lb, "A", "B", lineterm="", n=0):
                    print("    " + d)
                    shown += 1
                    if shown >= args.max_diff:
                        print("    ...")
                        break
    both = [c for c in a if c in b]
    mem = []
    for c in both:
        x, y = int(a[c]["maxrss_kb"]), int(b[c]["maxrss_kb"])
        if x and y and abs(x - y) > 16 * 1024 and (y > 1.05 * x or x > 1.05 * y):
            mem.append((c, x, y))
    for c, x, y in mem:
        print(f"note {c}: peak memory A {x // 1024} MB, B {y // 1024} MB ({y / x:.3f}x)")
    wa = sum(float(a[c]["wall_s"]) for c in both)
    wb = sum(float(b[c]["wall_s"]) for c in both)
    print(f"{len(both) - failures}/{len(both)} cells identical in every exact field "
          f"({', '.join(EXACT_FIELDS)})")
    print(f"peak memory notes: {len(mem)}; summed wall A {wa:.0f} s, B {wb:.0f} s "
          f"(not a timing measurement: the runs share the host)")
    return 1 if failures else 0


def cmd_renormalize(args):
    """Rebuild `out/` and the text hashes of a finished run from its raw
    outputs, after a change to `normalize`. No solver is run."""
    out_dir = os.path.abspath(args.dir)
    meta = json.load(open(os.path.join(out_dir, "meta.json")))
    paths = dict(cells_of(meta["set"]))
    rows = read_table(out_dir)
    changed = 0
    for cell, row in rows.items():
        safe = cell.replace("/", "__")
        raw = os.path.join(out_dir, "raw", safe + ".out")
        lines = open(raw, errors="replace").read().splitlines()
        work = os.path.join(out_dir, "_work", safe)
        cnf = os.path.join(work, "in.cnf") if paths[cell].endswith(".xz") else paths[cell]
        norm = normalize(lines, [os.path.join(work, "proof.out"), cnf])
        with open(os.path.join(out_dir, "out", safe + ".txt"), "w") as f:
            f.write("\n".join(norm) + "\n")
        sha = sha_text(norm)
        if sha != row["text_sha"]:
            changed += 1
            row["text_sha"] = sha
    with open(os.path.join(out_dir, "results.tsv"), "w") as f:
        f.write("\t".join(FIELDS) + "\n")
        for cell in sorted(rows):
            f.write("\t".join(rows[cell][k] for k in FIELDS) + "\n")
    print(f"renormalized {len(rows)} cells of {out_dir}: {changed} text hashes changed")
    return 0


SAMPLE = """c ---- [ banner ] ------------------------------------------------------------
c Version X
c ---- [ parsing ] -----------------------------------------------------------
c   {cnf}
c finished parsing after {t} seconds
c structure proof {proof}.pbp derived {proof}.derived.cnf
c structure pass: 3 derived clauses (2 units), refuted, work 0K
c ---- [ solving ] -----------------------------------------------------------
c seconds switched rate     size/glue tier1 binary  remaining
c *  {t}  {mb} 0 0 0  0 0   0   0 0.0 0 0 0 0 0%   0  0  0 0%
c ---- [ result ] ------------------------------------------------------------
s UNSATISFIABLE
c ---- [ statistics ] --------------------------------------------------------
c conflicts:                               {conf}             {rate} per second
c ---- [ resources ] ---------------------------------------------------------
c maximum-resident-set-size:          {rss} bytes          4 MB
c process-time:                                              {t} seconds
c workclock ticks=131 work=131
"""


def cmd_selftest(_args):
    def text(t, mb, rss, rate, conf, cnf, proof):
        return SAMPLE.format(t=t, mb=mb, rss=rss, rate=rate, conf=conf, cnf=cnf, proof=proof).splitlines()
    a = text("0.00", "5", "4407296", "0.0", "7", "/x/a.cnf", "/x/p.out")
    b = text("12.34", "61", "9999999", "3.5", "7", "/y/long/b.cnf", "/y/q.out")
    c = text("0.00", "5", "4407296", "0.0", "8", "/x/a.cnf", "/x/p.out")
    na = normalize(a, ["/x/p.out", "/x/a.cnf"])
    nb = normalize(b, ["/y/q.out", "/y/long/b.cnf"])
    nc = normalize(c, ["/x/p.out", "/x/a.cnf"])
    assert na == nb, "times, memory and paths must not show in the normalized text"
    wide = [line.replace("seconds switched rate", "seconds   switched  rate") for line in a]
    assert normalize(wide, ["/x/p.out", "/x/a.cnf"]) == na, "header spacing must not show"
    assert normalize(["c freewalk steps=5 walk_ms=12 forms=3"], []) == ["c freewalk steps=5 walk_ms=<t> forms=3"]
    assert na != nc, "a changed counter must show"
    assert counters_of(a) == {"conflicts": 7} and counters_of(c) == {"conflicts": 8}
    assert any("structure pass" in line for line in na), "pass lines stay"
    assert any(line.startswith("c * 0 0 0") for line in na), "report rows stay, without seconds and MB"
    assert peak_kb(os.getpid(), "/no/such/solver") == 0, "memory is read only from the solver itself"
    opts = stock_options()
    assert "--gauss=0" in opts and "--card=0" in opts and "--freewalk=0" in opts, opts
    print(f"selftest ok ({len(opts)} stock options: {' '.join(opts)})")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--solver", required=True)
    r.add_argument("--set", required=True, choices=sorted(DEFAULT_LIMIT))
    r.add_argument("--out", required=True)
    r.add_argument("--limit-ticks", type=int, help="work limit (0 for none); default per set")
    r.add_argument("--options", default="", help="extra solver options, after the set's own")
    r.add_argument("--jobs", type=int, default=32)
    r.add_argument("--cores", default="", help="comma-separated cores to pin to")
    r.add_argument("--wall-limit", type=int, default=7200,
                   help="safety net in seconds; a cell that hits it is not comparable")
    r.set_defaults(func=cmd_run)
    c = sub.add_parser("compare")
    c.add_argument("a")
    c.add_argument("b")
    c.add_argument("--max-diff", type=int, default=12)
    c.set_defaults(func=cmd_compare)
    n = sub.add_parser("renormalize")
    n.add_argument("dir")
    n.set_defaults(func=cmd_renormalize)
    s = sub.add_parser("selftest")
    s.set_defaults(func=cmd_selftest)
    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
