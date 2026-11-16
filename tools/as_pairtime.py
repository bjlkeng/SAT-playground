#!/usr/bin/env python3
"""Paired wall-time check of two solver builds (epic SAT-playground-bp3).

Two builds that do the same search can still differ in speed: a field moved,
a function laid out differently. This tool measures that. It runs build A and
build B on the same cell at the same moment, each on its own physical core,
and swaps the cores every repetition, so what the host does at that moment
hits both alike. Per cell it takes the mean of the B/A time ratios (the
geometric mean), and over the cells the mean and the spread.

With the same build as A and B this is the noise floor: how far the ratio is
from 1.000 when nothing changed. A real difference has to be clearly outside
that.

The cells are the 20 of benchmarks/discriminating, each stopped at a conflict
limit so the two runs do exactly the same work; the tool checks that their
work-clock lines are equal and says so if not.

--load N starts N background solver runs on other cores first. The port's
speed against the C has differed under load with the same search before, so
the check is made both ways.

usage:
  as_pairtime.py --a BIN --b BIN --out DIR [--reps 4] [--load 28]
"""

import argparse
import json
import lzma
import math
import os
import resource
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SCREEN = os.path.join(ROOT, "benchmarks", "discriminating")
MEM_BYTES = 16 * 1024 ** 3

# Pairs of physical cores. The host has 36 physical cores (0-35) whose second
# threads are 36-71 and stay idle. Core 0 takes the host's own chores.
QUIET_PAIRS = [(2, 4), (6, 8), (10, 12), (14, 16)]
LOAD_PAIRS = [(2, 4), (5, 7)]
LOAD_CORES = list(range(8, 36))
LOAD_CELL = "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7"


def unpack(stem, scratch):
    dst = os.path.join(scratch, stem + ".cnf")
    if not os.path.isfile(dst):
        with lzma.open(os.path.join(SCREEN, stem + ".cnf.xz"), "rb") as src, open(dst, "wb") as out:
            shutil.copyfileobj(src, out, 1 << 22)
    return dst


def run_one(binary, cnf, core, opts, out_path):
    def pre():
        os.sched_setaffinity(0, {core})
        resource.setrlimit(resource.RLIMIT_AS, (MEM_BYTES, MEM_BYTES))

    start = time.monotonic()
    with open(out_path, "wb") as fo:
        p = subprocess.Popen([binary, "-n"] + opts + [cnf], stdout=fo, stderr=subprocess.STDOUT, preexec_fn=pre)
        _, status, usage = os.wait4(p.pid, 0)
        p.returncode = os.waitstatus_to_exitcode(status)
    wall = time.monotonic() - start
    clock = "none"
    answer = "none"
    for line in open(out_path, errors="replace"):
        if line.startswith("c workclock "):
            clock = line.strip()
        elif line.startswith("s "):
            answer = line[2:].strip()
    return {"wall": wall, "cpu": usage.ru_utime + usage.ru_stime, "clock": clock,
            "answer": answer, "exit": p.returncode}


def geomean(xs):
    return math.exp(sum(math.log(x) for x in xs) / len(xs))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--a", required=True)
    ap.add_argument("--b", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--reps", type=int, default=4, help="repetitions per cell (even: the cores swap)")
    ap.add_argument("--conflicts", type=int, default=100000)
    ap.add_argument("--options", default="", help="extra solver options for both")
    ap.add_argument("--load", type=int, default=0, help="background solver runs on cores 8-35")
    ap.add_argument("--load-binary", default="", help="the binary of the background runs (default: A)")
    args = ap.parse_args()
    a, b = os.path.abspath(args.a), os.path.abspath(args.b)
    for x in (a, b):
        if not os.access(x, os.X_OK):
            raise SystemExit(f"not an executable: {x}")
    if args.reps % 2:
        raise SystemExit("--reps must be even, so each build runs on each core as often")
    out_dir = os.path.abspath(args.out)
    scratch = os.path.join(out_dir, "_cnf")
    os.makedirs(scratch, exist_ok=True)
    os.makedirs(os.path.join(out_dir, "out"), exist_ok=True)
    stems = sorted(f[:-len(".cnf.xz")] for f in os.listdir(SCREEN) if f.endswith(".cnf.xz"))
    if len(stems) != 20:
        raise SystemExit(f"{SCREEN} has {len(stems)} cells, expected 20")
    opts = [f"--conflicts={args.conflicts}", "--seed=0"] + (args.options.split() if args.options else [])
    print(f"[as_pairtime] unpacking {len(stems)} cells", flush=True)
    with ThreadPoolExecutor(max_workers=8) as ex:
        cnfs = dict(zip(stems, ex.map(lambda s: unpack(s, scratch), stems)))

    load = []
    pairs = LOAD_PAIRS if args.load else QUIET_PAIRS
    rows = []
    # The background runs are endless loops in sessions of their own, so they
    # are started inside the block whose `finally` stops them: an interrupt
    # during the warm-up, or a failure to start one, must not leave them
    # running (review round 3).
    try:
        if args.load:
            lb = os.path.abspath(args.load_binary) if args.load_binary else a
            lcnf = cnfs.get(LOAD_CELL) or unpack(LOAD_CELL, scratch)
            for core in LOAD_CORES[:args.load]:
                # a loop in a shell, so the load lasts as long as the measurement
                p = subprocess.Popen(
                    ["bash", "-c", f'while :; do "{lb}" -n -q "{lcnf}" >/dev/null 2>&1; done'],
                    preexec_fn=lambda c=core: (os.setsid(), os.sched_setaffinity(0, {c})))
                load.append(p)
            print(f"[as_pairtime] {len(load)} background runs started on cores "
                  f"{LOAD_CORES[0]}-{LOAD_CORES[len(load) - 1]}", flush=True)
            time.sleep(20)  # let them get past parsing

        def cell_job(item):
            slot, stem = item
            ca, cb = pairs[slot]
            reps = []
            for r in range(args.reps):
                x, y = (ca, cb) if r % 2 == 0 else (cb, ca)
                with ThreadPoolExecutor(max_workers=2) as ex2:
                    fa = ex2.submit(run_one, a, cnfs[stem], x, opts, os.path.join(out_dir, "out", f"{stem}.a{r}.out"))
                    fb = ex2.submit(run_one, b, cnfs[stem], y, opts, os.path.join(out_dir, "out", f"{stem}.b{r}.out"))
                    reps.append((fa.result(), fb.result()))
            return stem, reps

        # one worker per core pair; each worker takes the next cell
        import queue
        todo = queue.Queue()
        for s in stems:
            todo.put(s)

        def worker(slot):
            out = []
            while True:
                try:
                    stem = todo.get_nowait()
                except queue.Empty:
                    return out
                out.append(cell_job((slot, stem)))
                stem, reps = out[-1]
                ratio = geomean([rb["wall"] / ra["wall"] for ra, rb in reps])
                print(f"  {stem[33:73]:40s} A {reps[0][0]['wall']:7.1f}s  B/A {ratio:.4f}", flush=True)

        with ThreadPoolExecutor(max_workers=len(pairs)) as ex:
            for part in ex.map(worker, range(len(pairs))):
                rows.extend(part)
    finally:
        for p in load:
            try:
                os.killpg(p.pid, 15)
            except ProcessLookupError:
                pass
        for p in load:
            try:
                p.wait(timeout=20)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, 9)
        shutil.rmtree(scratch, ignore_errors=True)

    rows.sort()
    unequal = []
    table = []
    for stem, reps in rows:
        wall = [rb["wall"] / ra["wall"] for ra, rb in reps]
        cpu = [rb["cpu"] / ra["cpu"] for ra, rb in reps]
        clocks = {r["clock"] for pair in reps for r in pair}
        if len(clocks) != 1 or "none" in clocks:
            unequal.append(stem)
        table.append({
            "cell": stem, "answer": reps[0][0]["answer"],
            "a_wall": geomean([ra["wall"] for ra, _ in reps]),
            "wall_ratio": geomean(wall), "cpu_ratio": geomean(cpu),
            "wall_min": min(wall), "wall_max": max(wall),
        })
    with open(os.path.join(out_dir, "pairs.tsv"), "w") as f:
        f.write("cell\tanswer\ta_wall_s\twall_ratio\tcpu_ratio\trep_min\trep_max\n")
        for t in table:
            f.write(f"{t['cell']}\t{t['answer']}\t{t['a_wall']:.2f}\t{t['wall_ratio']:.4f}\t"
                    f"{t['cpu_ratio']:.4f}\t{t['wall_min']:.4f}\t{t['wall_max']:.4f}\n")
    wr = [t["wall_ratio"] for t in table]
    cr = [t["cpu_ratio"] for t in table]
    summary = {
        "a": a, "b": b, "options": opts, "reps": args.reps, "load": len(load), "cells": len(table),
        "wall_geomean": round(geomean(wr), 4), "wall_cell_min": round(min(wr), 4),
        "wall_cell_max": round(max(wr), 4), "cpu_geomean": round(geomean(cr), 4),
        "rep_min": round(min(t["wall_min"] for t in table), 4),
        "rep_max": round(max(t["wall_max"] for t in table), 4),
        "unequal_work": unequal, "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    with open(os.path.join(out_dir, "summary.json"), "w") as f:
        json.dump(summary, f, indent=1)
        f.write("\n")
    print(f"[as_pairtime] B/A wall time, {len(table)} cells x {args.reps} reps, load {len(load)}: "
          f"geomean {summary['wall_geomean']:.4f}, cells {summary['wall_cell_min']:.4f} to "
          f"{summary['wall_cell_max']:.4f}, single reps {summary['rep_min']:.4f} to {summary['rep_max']:.4f}; "
          f"CPU time geomean {summary['cpu_geomean']:.4f}")
    if unequal:
        print(f"[as_pairtime] NOT THE SAME WORK on {len(unequal)} cells (work-clock lines differ): {unequal}")
        return 1
    print("[as_pairtime] every pair did the same work (equal work-clock lines)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
