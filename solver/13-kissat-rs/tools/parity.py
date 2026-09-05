#!/usr/bin/env python3
"""Parity oracle for solver 13 vs reference kissat 4.0.4.

Runs both binaries on a corpus with identical options and diffs the
deterministic first-column counters of the `-s` statistics block, plus the
`s` status line. Wall-time-derived ratio columns are ignored.

Usage:
  parity.py [--conflicts N] [--options '--probe=0 ...'] cnf [cnf ...]
  parity.py --corpus default [--conflicts N]
  parity.py --solver /path/to/frozen/sat-solver ...   (test a copied binary)
  parity.py --phases ...   (also run both with -v and diff the bracketed
                            phase lines: catches watch-stack layout
                            divergences the counters cannot see — the
                            2026-09-04 SET_END_OF_WATCHES bug showed only
                            as a different "[vectors] enlarged"/"[defrag]"
                            sequence and 4x RSS; numeric tokens compare
                            with 1e-5 relative tolerance, report rows and
                            option/banner lines are skipped)

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


PHASE_RE = re.compile(r"^c \[([a-z]+)(?:-\d+)?\] (.*)$")
NUM_RE = re.compile(r"^[-+]?(\d+\.?\d*(?:e[-+]?\d+)?|\.\d+(?:e[-+]?\d+)?)%?$", re.I)


def phase_lines(stdout):
    """Bracketed verbose lines as (name, tokens); layout-sensitive messages
    (vectors/defrag/arena) included, options/banner excluded."""
    out = []
    for line in stdout.splitlines():
        m = PHASE_RE.match(line)
        if not m:
            continue
        name, rest = m.group(1), m.group(2)
        if name in ("options", "banner", "kissat"):
            continue
        rest = rest.replace(" (moved)", "").replace(" (in place)", "")
        rest = rest.replace("18446744073709551615", "N")  # IGNOREd METRIC count
        out.append((name, rest.split()))
    return out


def tokens_equal(a, b):
    if a == b:
        return True
    ma, mb = NUM_RE.match(a), NUM_RE.match(b)
    if not (ma and mb):
        return False
    try:
        x, y = float(ma.group(1)), float(mb.group(1))
    except ValueError:
        return False
    return abs(x - y) <= 1e-5 * max(abs(x), abs(y), 1e-300)


def diff_phases(ours, ref):
    """Return the first mismatching (index, ours, ref) or None."""
    n = min(len(ours), len(ref))
    for i in range(n):
        (na, ta), (nb, tb) = ours[i], ref[i]
        if na != nb or len(ta) != len(tb) or not all(tokens_equal(x, y) for x, y in zip(ta, tb)):
            return i, ours[i], ref[i]
    if len(ours) != len(ref):
        return n, ours[n] if n < len(ours) else None, ref[n] if n < len(ref) else None
    return None


def run(binary, cnf, extra, timeout):
    cmd = [binary, "-n", "-s"] + extra + [cnf]
    try:
        p = subprocess.run(
            cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return None, {}, f"TIMEOUT-{timeout}s", ""
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
    return status, stats, None, p.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cnfs", nargs="*")
    ap.add_argument("--conflicts", type=int)
    ap.add_argument("--decisions", type=int)
    ap.add_argument("--options", default="", help="extra options for both")
    ap.add_argument("--corpus", choices=["default"], help="use built-in corpus")
    ap.add_argument("--timeout", type=int, default=600, help="per-run seconds")
    ap.add_argument("--solver", default=SOLVER13, help="solver 13 binary to test")
    ap.add_argument("--kissat", default=KISSAT, help="reference kissat binary")
    ap.add_argument("--phases", action="store_true",
                    help="also run with -v and diff the bracketed phase lines "
                         "(layout oracle: [vectors]/[defrag]/[arena] sequences)")
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
    if args.phases:
        extra.append("-v")

    failures = 0
    for cnf in cnfs:
        ks, kstats, kerr, kout = run(args.kissat, cnf, extra, args.timeout)
        ss, sstats, serr, sout = run(args.solver, cnf, extra, args.timeout)
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
            continue
        if args.phases:
            kp, sp = phase_lines(kout), phase_lines(sout)
            mism = diff_phases(sp, kp)
            if mism is not None:
                i, ours, ref = mism
                print(f"FAIL {name}: status={ks}, counters match, phase line {i} differs "
                      f"({len(sp)} v {len(kp)} lines)")
                print(f"    solver13: {ours}")
                print(f"    kissat:   {ref}")
                failures += 1
                continue
            print(f"ok   {name}: status={ks}, {len(kstats)} counters match, {len(kp)} phase lines match")
        else:
            print(f"ok   {name}: status={ks}, {len(kstats)} counters match")
    print(f"\n{len(cnfs) - failures}/{len(cnfs)} instances at parity")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
