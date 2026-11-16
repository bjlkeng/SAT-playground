#!/usr/bin/env python3
"""Hardware counters of several solver builds on the same cells, side by side
(epic SAT-playground-bp3, the performance gate).

`as_pairtime.py` says whether one build is slower than another. This tool says
why. Two builds that do the same search run the same instructions; if one
still takes more cycles, the counters show where they go:

  instructions       the work done. Equal builds agree to about 0.1%.
  cycles             the time, in processor clock ticks.
  idq.dsb_uops       micro-operations served from the cache of decoded
                     instructions (fast).
  idq.mite_uops      micro-operations that went through the decoder again
                     (slow). This host's Xeon Gold 6154 keeps a 32-byte window
                     of code out of the cache when a jump crosses the window's
                     end or stops on it, so the share depends on the address
                     the compiler happened to give a loop.
  branch-misses

These five are the default because the processor can count five at once. With
more, perf lets the events take turns and prints estimates (on this host
eight events were each counted 49 to 62% of the time, and the instruction
estimate was off by more than the difference looked for). The tool reads how
long each event was counted and stops if it was not the whole run: give fewer
events (others worth a run of their own: L1-dcache-load-misses, branches,
dsb2mite_switches.penalty_cycles).

Each build runs each cell once a round on its own physical core. The cores
rotate between the builds from round to round, and the rounds must be a
multiple of the number of builds, so every build sits on every core equally
often (default: one full rotation). Counters are user space only
(perf_event_paranoid 1 is enough). The cells are those of
benchmarks/discriminating, stopped at a conflict limit so every build does the
same work. Every run has the 16 GB memory cap. A run that does not end with a
solver exit code (0, 10 or 20) stops the tool: its counters would look like a
measurement and be none. An interrupt (Ctrl-C, SIGTERM) stops every run that
is going.

usage:
  as_counters.py --builds main=BIN,new=BIN [--cells case9,sqrt-mitern171,...]
                 [--rounds N] [--cores 2,4,6,8] [--events ...]
                 [--options "--conflicts=100000 --seed=0"]

The first build is the reference: every number is also given as a ratio to it.
"""

import argparse
import lzma
import os
import resource
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SCREEN = os.path.join(ROOT, "benchmarks", "discriminating")
# Physical cores of one processor; their second threads stay idle.
CORES = [2, 4, 6, 8, 10, 12, 14, 16]
EVENTS = ["instructions", "cycles", "idq.dsb_uops", "idq.mite_uops", "branch-misses"]
# The four small cells that fit in the cache, where a difference shows most.
CELLS = "case9,sqrt-mitern171,SCPC-500-1,circuit_48in64out"
MEM_BYTES = 16 * 1024 ** 3
SOLVER_EXITS = (0, 10, 20)

# The runs that are going, so an interrupt or a failed run can stop them all.
# Each is `perf` with the solver under it, in a session of its own, so the
# two are one group to kill.
CANCEL = threading.Event()
LIVE = set()
LIVE_LOCK = threading.Lock()


class Cancelled(Exception):
    pass


class Failed(Exception):
    pass


def stop_all():
    CANCEL.set()
    with LIVE_LOCK:
        running = list(LIVE)
    for p in running:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def unpack(cell, scratch):
    hits = [f for f in sorted(os.listdir(SCREEN)) if cell in f and f.endswith(".cnf.xz")]
    if len(hits) != 1:
        raise SystemExit(f"cell {cell!r} matches {len(hits)} files of {SCREEN}")
    dst = os.path.join(scratch, hits[0][:-3])
    with lzma.open(os.path.join(SCREEN, hits[0]), "rb") as src, open(dst, "wb") as out:
        shutil.copyfileobj(src, out, 1 << 22)
    return dst


def count(binary, cnf, core, events, opts):
    """One run under perf stat: {event: count}."""
    def pre():
        os.setsid()
        os.sched_setaffinity(0, {core})
        resource.setrlimit(resource.RLIMIT_AS, (MEM_BYTES, MEM_BYTES))

    cmd = ["perf", "stat", "-x", ",", "-e", ",".join(e + ":u" for e in events), binary, "-n"] + opts + [cnf]
    with LIVE_LOCK:
        if CANCEL.is_set():
            raise Cancelled()
        p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, preexec_fn=pre)
        LIVE.add(p)
    try:
        _, err = p.communicate()
    finally:
        with LIVE_LOCK:
            LIVE.discard(p)
    if CANCEL.is_set():
        raise Cancelled()
    if p.returncode not in SOLVER_EXITS:
        raise Failed(f"the run of {binary} ended with exit code {p.returncode}, not a solver answer; "
                     f"its last output:\n{err[-600:]}")
    # perf's lines: count, unit, event, time counted (ns), share of the run counted (%), ...
    out = {}
    shared = []
    for line in err.splitlines():
        f = line.split(",")
        if len(f) > 4 and f[0].isdigit():
            name = f[2][:-2] if f[2].endswith(":u") else f[2]
            out[name] = float(f[0])
            try:
                if float(f[4]) < 99.995:
                    shared.append(f"{name} {f[4]}%")
            except ValueError:
                shared.append(f"{name} ?")
    missing = [e for e in events if e not in out]
    if missing:
        raise Failed(f"perf gave no count for {missing} (binary {binary}); its output:\n{err[-600:]}")
    if shared:
        raise Failed("the events took turns on the processor's counters, so the counts are estimates "
                     f"({', '.join(shared)} of the run counted); give fewer events")
    return out


def measure(builds, cnf, events, opts, rounds, cores):
    """The mean counts of every build on one cell: {name: {event: count}}."""
    mean = {name: dict.fromkeys(events, 0.0) for name, _ in builds}
    for r in range(rounds):
        ex = ThreadPoolExecutor(len(builds))
        try:
            futs = {name: ex.submit(count, binary, cnf, cores[(i + r) % len(builds)], events, opts)
                    for i, (name, binary) in enumerate(builds)}
            for name, fut in futs.items():
                for k, v in fut.result().items():
                    mean[name][k] += v / rounds
        except BaseException:
            # an interrupt, or one run failed: stop the others before leaving
            stop_all()
            ex.shutdown(wait=True, cancel_futures=True)
            raise
        ex.shutdown(wait=True)
    return mean


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--builds", required=True, help="NAME=BINARY,... (the first is the reference)")
    ap.add_argument("--cells", default=CELLS, help="comma-separated parts of cell names")
    ap.add_argument("--rounds", type=int, default=0,
                    help="a multiple of the number of builds (default: that number, one full rotation)")
    ap.add_argument("--cores", default=",".join(str(c) for c in CORES),
                    help="physical cores to use; the first as many as there are builds")
    ap.add_argument("--events", default=",".join(EVENTS))
    ap.add_argument("--options", default="--conflicts=100000 --seed=0")
    args = ap.parse_args()
    builds = [b.split("=", 1) for b in args.builds.split(",")]
    for name, binary in builds:
        if not os.access(binary, os.X_OK):
            raise SystemExit(f"{name}: not an executable: {binary}")
    cores = [int(c) for c in args.cores.split(",")][:len(builds)]
    if len(builds) > len(cores):
        raise SystemExit(f"{len(builds)} builds need {len(builds)} cores, --cores gives {len(cores)}")
    rounds = args.rounds or len(builds)
    if rounds % len(builds):
        raise SystemExit(f"--rounds {rounds} is not a multiple of the {len(builds)} builds: "
                         "each build must sit on each core equally often")
    if not shutil.which("perf"):
        raise SystemExit("perf is not installed")
    events = args.events.split(",")
    opts = args.options.split()

    def on_term(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, on_term)
    try:
        return report(builds, args.cells.split(","), events, opts, rounds, cores)
    except (KeyboardInterrupt, Cancelled):
        stop_all()
        print("[as_counters] interrupted: the runs were stopped, no further table", flush=True)
        return 130
    except Failed as e:
        print(f"[as_counters] FAILED: {e}", flush=True)
        return 1


def report(builds, cells, events, opts, rounds, cores):
    print(f"[as_counters] {len(builds)} builds, {rounds} rounds a cell, cores {cores}, "
          f"events {','.join(events)}", flush=True)
    with tempfile.TemporaryDirectory(prefix="as-counters-") as scratch:
        for cell in cells:
            cnf = unpack(cell, scratch)
            mean = measure(builds, cnf, events, opts, rounds, cores)
            os.remove(cnf)
            base = mean[builds[0][0]]
            print(f"== {cell}")
            print("%-34s" % "counter" + "".join("%24s" % name for name, _ in builds))
            for k in events:
                row = "%-34s" % k
                for name, _ in builds:
                    v = mean[name][k]
                    row += "%15.0f (%6.4f)" % (v, v / base[k] if base[k] else float("nan"))
                print(row)
            if "idq.dsb_uops" in events and "idq.mite_uops" in events:
                row = "%-34s" % "share through the decoder"
                for name, _ in builds:
                    m = mean[name]
                    row += "%23.1f%%" % (100 * m["idq.mite_uops"] / (m["idq.mite_uops"] + m["idq.dsb_uops"]))
                print(row)
            sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
