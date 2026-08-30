#!/usr/bin/env python3
"""Parity oracle for solver 13 vs reference kissat 4.0.4.

Runs both binaries on a corpus with identical options and diffs the
deterministic first-column counters of the `-s` statistics block, plus the
`s` status line. Wall-time-derived ratio columns are ignored.

Usage:
  parity.py [--conflicts N] [--options '--probe=0 ...'] cnf [cnf ...]
  parity.py --corpus default [--conflicts N]

Exit 0 iff every instance matches on status + all counters.
"""

import argparse
import os
import re
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
KISSAT = os.path.join(
    ROOT, "benchmarks", "reference-solvers", "kissat-latest", "build", "kissat"
)
SOLVER13 = os.path.join(
    ROOT, "solver", "13-kissat-rs", "target", "release", "sat-solver"
)

STAT_RE = re.compile(r"^c ([a-z_0-9]+):\s+(\d+)")

# Counters whose *first column* is still time-dependent (none known; ratios in
# later columns are ignored by construction). Resources section is excluded.
IGNORED = set()


def run(binary, cnf, extra):
    cmd = [binary, "-n", "-s"] + extra + [cnf]
    try:
        p = subprocess.run(
            cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, timeout=600,
        )
    except subprocess.TimeoutExpired:
        return None, {}, "TIMEOUT-600s"
    status = None
    stats = {}
    in_stats = False
    for line in p.stdout.splitlines():
        if line.startswith("s "):
            status = line[2:].strip()
        elif "[ statistics ]" in line:
            in_stats = True
        elif in_stats and ("[ " in line and " ]" in line):
            in_stats = False
        elif in_stats:
            m = STAT_RE.match(line)
            if m and m.group(1) not in IGNORED:
                stats[m.group(1)] = int(m.group(2))
    return status, stats, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cnfs", nargs="*")
    ap.add_argument("--conflicts", type=int)
    ap.add_argument("--decisions", type=int)
    ap.add_argument("--options", default="", help="extra options for both")
    ap.add_argument("--corpus", choices=["default"], help="use built-in corpus")
    args = ap.parse_args()

    cnfs = list(args.cnfs)
    if args.corpus == "default":
        for sub in ("tests/cnf/sat", "tests/cnf/unsat"):
            d = os.path.join(ROOT, sub)
            cnfs += sorted(
                os.path.join(d, f) for f in os.listdir(d) if f.endswith(".cnf")
            )
    if not cnfs:
        ap.error("no instances given")

    extra = args.options.split() if args.options else []
    if args.conflicts is not None:
        extra.append(f"--conflicts={args.conflicts}")
    if args.decisions is not None:
        extra.append(f"--decisions={args.decisions}")

    failures = 0
    for cnf in cnfs:
        ks, kstats, kerr = run(KISSAT, cnf, extra)
        ss, sstats, serr = run(SOLVER13, cnf, extra)
        name = os.path.basename(cnf)
        if kerr or serr:
            print(f"FAIL {name}: kissat={kerr or 'ok'} solver13={serr or 'ok'}")
            failures += 1
            continue
        if ks != ss:
            print(f"FAIL {name}: status kissat={ks} solver13={ss}")
            failures += 1
            continue
        diffs = []
        for key in sorted(set(kstats) | set(sstats)):
            kv, sv = kstats.get(key), sstats.get(key)
            if kv != sv:
                diffs.append(f"    {key}: kissat={kv} solver13={sv}")
        if diffs:
            print(f"FAIL {name}: status={ks}, {len(diffs)} counter diffs")
            print("\n".join(diffs[:40]))
            failures += 1
        else:
            print(f"ok   {name}: status={ks}, {len(kstats)} counters match")
    print(f"\n{len(cnfs) - failures}/{len(cnfs)} instances at parity")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
