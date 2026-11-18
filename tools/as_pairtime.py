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

Other cells, run to their answer. The 20 cells are short and most fit in the
processor's caches. A build can also differ on long runs that wait on memory,
and there two runs of one build differ by several percent with the core they
happen to get. For those: --suite DIR takes the cells of a folder of .cnf.xz
files (--cells FILE gives their names in the order to run, the longest first
keeps the pairs busy to the end), --conflicts 0 sets no conflict limit, and
--pairs gives the core pairs, each pair two neighbouring physical cores of
one socket. With many pairs the measured runs are each other's load. A cell's
two builds then share the moment, the socket and the neighbours, and swap
cores every repetition. The summary gives the summed time too, which is what
counts when the cells differ in length.

--proof file makes every run write its proof, to a file of its own under the
output folder, removed when the run ends. A solver in the harness or in a
competition always writes one, and making its lines is part of the time;
without this option no proof is written and only the search is timed. With
it the disk is in the wall time, so read the processor time. (Not /dev/null:
the structure stage writes side files next to the proof, cannot do that
there, and the run becomes another search: sqrt-mitern171 took 157 s instead
of 7 s.)

usage:
  as_pairtime.py --a BIN --b BIN --out DIR [--reps 4] [--load 28]
  as_pairtime.py --a BIN --b BIN --out DIR --suite DIR [--cells FILE] --conflicts 0
                 --pairs 0-1,2-3,... --reps 2 [--proof file] [--options=--seed=2]
(write --options=... with the equals sign: its value starts with two dashes)
"""

import argparse
import json
import lzma
import math
import os
import resource
import shutil
import signal
import subprocess
import sys
import threading
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


def unpack(stem, scratch, suite=SCREEN):
    dst = os.path.join(scratch, stem + ".cnf")
    if not os.path.isfile(dst):
        with lzma.open(os.path.join(suite, stem + ".cnf.xz"), "rb") as src, open(dst, "wb") as out:
            shutil.copyfileobj(src, out, 1 << 22)
    return dst


# Stopping.  An interrupt (Ctrl-C, SIGTERM) must end every solver this tool
# started, at once: the measured ones and the background loops.  `CANCEL`
# tells the workers to take no more work, and `LIVE` holds the measured
# solvers that are running, so they can be killed without waiting for them.
CANCEL = threading.Event()
LIVE = set()
LIVE_LOCK = threading.Lock()


def stop_measured():
    CANCEL.set()
    with LIVE_LOCK:
        running = list(LIVE)
    for p in running:
        try:
            p.kill()
        except ProcessLookupError:
            pass


def run_one(binary, cnf, core, opts, out_path, proof=None):
    """One measured run, or None when the tool is being stopped. `proof` is the file the
    solver writes its proof to (the argument after the formula), or None for no proof."""
    def pre():
        os.sched_setaffinity(0, {core})
        resource.setrlimit(resource.RLIMIT_AS, (MEM_BYTES, MEM_BYTES))

    start = time.monotonic()
    with open(out_path, "wb") as fo:
        with LIVE_LOCK:
            if CANCEL.is_set():
                return None
            p = subprocess.Popen([binary, "-n"] + opts + [cnf] + ([proof] if proof else []),
                                 stdout=fo, stderr=subprocess.STDOUT, preexec_fn=pre)
            LIVE.add(p)
        try:
            _, status, usage = os.wait4(p.pid, 0)
            p.returncode = os.waitstatus_to_exitcode(status)
        finally:
            with LIVE_LOCK:
                LIVE.discard(p)
    if CANCEL.is_set():
        return None
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
    ap.add_argument("--conflicts", type=int, default=100000, help="0: no limit, every run goes to its answer")
    ap.add_argument("--suite", default="", help="a folder of .cnf.xz cells (default: benchmarks/discriminating)")
    ap.add_argument("--cells", default="", help="a file with the names of the cells to run, one a line, in the "
                                                "order to run them (default: every cell of the suite, by name)")
    ap.add_argument("--pairs", default="", help="core pairs such as 0-1,2-3 (default: four pairs, two with --load)")
    ap.add_argument("--proof", choices=("none", "file"), default="none",
                    help="file: every run writes its proof to a file of its own (removed after the run), so "
                         "making the proof's lines is timed too")
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
    suite = os.path.abspath(args.suite) if args.suite else SCREEN
    stems = sorted(f[:-len(".cnf.xz")] for f in os.listdir(suite) if f.endswith(".cnf.xz"))
    if not args.suite and len(stems) != 20:
        raise SystemExit(f"{SCREEN} has {len(stems)} cells, expected 20")
    if args.cells:
        with open(args.cells) as fh:
            listed = [line.strip() for line in fh if line.strip()]
        unknown = [c for c in listed if c not in stems]
        if unknown or len(set(listed)) != len(listed) or not listed:
            raise SystemExit(f"--cells: {len(unknown)} names are not cells of {suite}, or a name is there twice, "
                             f"or the file is empty")
        stems = listed
    if not stems:
        raise SystemExit(f"no .cnf.xz cells in {suite}")
    opts = ([f"--conflicts={args.conflicts}"] if args.conflicts else []) + ["--seed=0"] \
        + (args.options.split() if args.options else [])
    if args.pairs:
        try:
            pairs = [tuple(int(c) for c in pr.split("-")) for pr in args.pairs.split(",")]
        except ValueError:
            raise SystemExit("--pairs must look like 0-1,2-3")
        used = [c for pr in pairs for c in pr]
        if any(len(pr) != 2 for pr in pairs) or len(set(used)) != len(used):
            raise SystemExit("--pairs: every pair is two cores, and no core may be there twice")
        if args.load and set(used) & set(LOAD_CORES[:args.load]):
            raise SystemExit("--pairs: a core is also one of the background runs' cores")
    else:
        pairs = LOAD_PAIRS if args.load else QUIET_PAIRS
    # the host's rule: at most 32 solver processes at once, measured and background together
    if 2 * len(pairs) + args.load > 32:
        raise SystemExit(f"{len(pairs)} pairs and {args.load} background runs are {2 * len(pairs) + args.load} "
                         "solver processes at once; the limit is 32")
    os.makedirs(scratch, exist_ok=True)
    os.makedirs(os.path.join(out_dir, "out"), exist_ok=True)
    proofs = os.path.join(out_dir, "_proof") if args.proof == "file" else ""
    if proofs:
        os.makedirs(proofs, exist_ok=True)
    print(f"[as_pairtime] unpacking {len(stems)} cells", flush=True)
    with ThreadPoolExecutor(max_workers=8) as ex:
        cnfs = dict(zip(stems, ex.map(lambda s: unpack(s, scratch, suite), stems)))

    load = []
    rows = []
    ex = ThreadPoolExecutor(max_workers=len(pairs))

    def on_term(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, on_term)

    def load_pre(core):
        # a session of its own (so the loop and its solver are one group to
        # kill), its core, and the memory cap, which the solvers inherit
        os.setsid()
        os.sched_setaffinity(0, {core})
        resource.setrlimit(resource.RLIMIT_AS, (MEM_BYTES, MEM_BYTES))

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
                    preexec_fn=lambda c=core: load_pre(c))
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
                    # with --proof file: one folder a run, so the proof and the side files the
                    # structure stage writes next to it go away together
                    pa, pb = (os.path.join(proofs, f"{stem}.{w}{r}") for w in "ab")
                    for d in (pa, pb):
                        if proofs:
                            os.makedirs(d, exist_ok=True)
                    try:
                        fa = ex2.submit(run_one, a, cnfs[stem], x, opts, os.path.join(out_dir, "out", f"{stem}.a{r}.out"),
                                        os.path.join(pa, "proof.out") if proofs else None)
                        fb = ex2.submit(run_one, b, cnfs[stem], y, opts, os.path.join(out_dir, "out", f"{stem}.b{r}.out"),
                                        os.path.join(pb, "proof.out") if proofs else None)
                        ra, rb = fa.result(), fb.result()
                    finally:
                        for d in (pa, pb):
                            if proofs:
                                shutil.rmtree(d, ignore_errors=True)
                if ra is None or rb is None:
                    return None     # the tool is being stopped
                ra["core"], rb["core"] = x, y
                reps.append((ra, rb))
            return stem, reps

        # one worker per core pair; each worker takes the next cell
        import queue
        todo = queue.Queue()
        for s in stems:
            todo.put(s)

        def worker(slot):
            out = []
            while not CANCEL.is_set():
                try:
                    stem = todo.get_nowait()
                except queue.Empty:
                    return out
                done = cell_job((slot, stem))
                if done is None:
                    return out
                out.append(done)
                stem, reps = done
                ratio = geomean([rb["wall"] / ra["wall"] for ra, rb in reps])
                print(f"  {stem[33:73]:40s} A {reps[0][0]['wall']:7.1f}s  B/A {ratio:.4f}", flush=True)
            return out

        futures = [ex.submit(worker, slot) for slot in range(len(pairs))]
        for f in futures:
            rows.extend(f.result())
    except KeyboardInterrupt:
        # stop the measured solvers first, so the workers come back at once
        # instead of running the rest of the sweep (review round 4)
        stop_measured()
        print("[as_pairtime] interrupted: stopping every solver", flush=True)
        raise
    finally:
        stop_measured()
        ex.shutdown(wait=True)
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
        if proofs:
            shutil.rmtree(proofs, ignore_errors=True)

    if len(rows) != len(stems):
        print(f"[as_pairtime] only {len(rows)} of {len(stems)} cells were measured: no summary")
        return 1
    rows.sort()
    unequal = []
    table = []
    # every single run, for a later look: which core each build had, and its times
    with open(os.path.join(out_dir, "reps.tsv"), "w") as f:
        f.write("cell\trep\ta_core\tb_core\ta_wall_s\tb_wall_s\ta_cpu_s\tb_cpu_s\ta_exit\tb_exit\n")
        for stem, reps in rows:
            for r, (ra, rb) in enumerate(reps):
                f.write(f"{stem}\t{r}\t{ra['core']}\t{rb['core']}\t{ra['wall']:.3f}\t{rb['wall']:.3f}\t"
                        f"{ra['cpu']:.3f}\t{rb['cpu']:.3f}\t{ra['exit']}\t{rb['exit']}\n")
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
            # the cell's seconds, the mean over its repetitions, for the summed ratios
            "a_wall_sum": sum(ra["wall"] for ra, _ in reps) / len(reps),
            "b_wall_sum": sum(rb["wall"] for _, rb in reps) / len(reps),
            "a_cpu_sum": sum(ra["cpu"] for ra, _ in reps) / len(reps),
            "b_cpu_sum": sum(rb["cpu"] for _, rb in reps) / len(reps),
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
        "wall_summed": round(sum(t["b_wall_sum"] for t in table) / sum(t["a_wall_sum"] for t in table), 4),
        "cpu_summed": round(sum(t["b_cpu_sum"] for t in table) / sum(t["a_cpu_sum"] for t in table), 4),
        "a_cpu_seconds": round(sum(t["a_cpu_sum"] for t in table), 1),
        "suite": suite, "pairs": [list(pr) for pr in pairs], "proof": args.proof,
        "unequal_work": unequal, "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    with open(os.path.join(out_dir, "summary.json"), "w") as f:
        json.dump(summary, f, indent=1)
        f.write("\n")
    print(f"[as_pairtime] B/A wall time, {len(table)} cells x {args.reps} reps, load {len(load)}: "
          f"geomean {summary['wall_geomean']:.4f}, cells {summary['wall_cell_min']:.4f} to "
          f"{summary['wall_cell_max']:.4f}, single reps {summary['rep_min']:.4f} to {summary['rep_max']:.4f}; "
          f"CPU time geomean {summary['cpu_geomean']:.4f}; summed over the cells: wall {summary['wall_summed']:.4f}, "
          f"CPU {summary['cpu_summed']:.4f} (A's CPU time {summary['a_cpu_seconds']:.0f} s)")
    if unequal:
        print(f"[as_pairtime] NOT THE SAME WORK on {len(unequal)} cells (work-clock lines differ): {unequal}")
        return 1
    print("[as_pairtime] every pair did the same work (equal work-clock lines)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
