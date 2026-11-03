#!/usr/bin/env python3
"""Fuzz the decision-diagram count, src/structdd.rs (bead SAT-playground-skz.6).

Random small formulas with the pass's shape and with the ways it can go
wrong: board variables in at-most-one groups, some groups missing a pair
clause (so they are not cliques), a one-way counter diagram over a random
level order asking for at least k or exactly k trues, levels that count a
false instead of a true, levels skipped, nodes missing their redundant
third clause, a clause dropped or a child replaced by a constant (a fault
the reader must notice), nodes of negative polarity, extra units, noise
clauses, repeated literals, tautologies, shuffled clauses and renamed
variables.

For each formula: the solver with the pass off and with it on, with and
without a proof file, must agree. A SAT answer's model is checked against
every clause. An UNSAT answer from a run where the pass fired goes through
the two-stage gate (tools/proof_gate.py). The formulas are small enough for
the stock search to decide them, so a wrong refutation shows up as a
disagreement as well as a failed proof.

Usage: python3 tools/dd_fuzz.py --count 2000 [--seed 1] [--jobs 8] [--out DIR]
"""
from __future__ import annotations

import argparse
import itertools
import os
import random
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOLVER = Path(os.environ.get("DD_FUZZ_SOLVER", str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver")))
GATE = ROOT / "tools" / "proof_gate.py"
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0", "--structdiv=0", "--structdd=0", "--gauss=0", "--card=0"]


def build(seed):
    """Returns (variable count, clauses, a description of the choices)."""
    rng = random.Random(seed)
    g = rng.randint(2, 5)
    sizes = [rng.randint(1, 4) for _ in range(g)]
    m = sum(sizes)
    board = list(range(1, m + 1))
    rng.shuffle(board)
    groups = []
    i = 0
    for s in sizes:
        groups.append(board[i:i + s])
        i += s
    clauses = []
    fault = []
    for grp in groups:
        pairs = list(itertools.combinations(grp, 2))
        if pairs and rng.random() < 0.06:
            pairs.remove(rng.choice(pairs))   # not a clique any more
            fault.append("pair-dropped")
        for a, b in pairs:
            clauses.append([-a, -b])
    if rng.random() < 0.25:
        a, b = rng.sample(board, 2)
        clauses.append([-a, -b])              # an extra conflict across groups
    # the level order: a shuffle of all or most of the board
    levels = list(board)
    rng.shuffle(levels)
    if rng.random() < 0.2 and len(levels) > 2:
        levels = levels[:-1]                  # a board variable outside the diagram
    # most formulas are clean, so the pass fires; a share carries one kind
    # of change the reader has to read correctly or decline
    style = rng.choice(["clean"] * 6 + ["false-level", "skip", "drop-clause", "strong-clause"])
    counts_false = {x: style == "false-level" and rng.random() < 0.2 for x in levels}
    skip_rate = 0.05 if style == "skip" else 0.0
    exact = rng.random() < 0.5
    k = max(0, g + rng.choice([-1, 0, 1, 1, 1, 2]))
    keep_third = rng.random()
    next_var = [m]
    memo = {}
    T, F = "true", "false"   # never a literal, unlike Python's True and False

    def node(i, c):
        """A literal, or True or False: at least c (exactly c when `exact`)
        of the counted levels from i on hold."""
        rem = len(levels) - i
        if c > rem or c < 0:
            return F
        if rem == 0:
            return T if c == 0 else F
        if not exact and c == 0:
            return T
        key = (i, c)
        if key in memo:
            return memo[key]
        x = levels[i]
        counted = -x if counts_false[x] else x
        if rem == 1:
            r = (counted if c == 1 else -counted)
            memo[key] = r
            return r
        hi = node(i + 1, c - 1)
        lo = node(i + 1, c)
        if rng.random() < skip_rate and i + 2 < len(levels):
            lo = node(i + 2, c)               # a skipped level: not the same count
            fault.append("level-skipped")
        next_var[0] += 1
        n = next_var[0]
        cls = []
        for parts in ([-n, -counted, hi], [-n, counted, lo], [-n, hi, lo]):
            if T in parts:
                continue
            c2 = [p for p in parts if p != F]
            if any(-p in c2 for p in c2):
                continue
            cls.append(c2)
        if len(cls) == 3 and keep_third < 0.2:
            cls.pop()                         # the redundant third clause left out
        kept = [c1 for c1 in cls if not any(set(o) < set(c1) for o in cls)]
        memo[key] = n
        clauses.extend(kept)
        return n

    root = node(0, k)
    if root == F:
        clauses.append([1])
        clauses.append([-1])                  # the count cannot be met at all
    elif root != T:
        clauses.append([root])
    nv = next_var[0]
    # faults on the diagram
    defs = [c for c in clauses if len(c) >= 2 and c[0] < 0 and -c[0] > m]
    if defs and style == "drop-clause":
        clauses.remove(rng.choice(defs))
        fault.append("clause-dropped")
    if defs and style == "strong-clause":
        c = rng.choice(defs)
        if c in clauses and len(c) >= 2:
            clauses.remove(c)
            clauses.append(c[:-1] if len(c) > 2 else c)   # a stronger node clause
            fault.append("child-dropped")
    # noise
    if rng.random() < 0.2:
        clauses.append([rng.choice(board) * rng.choice([1, -1])])
    if rng.random() < 0.2:
        nv += 1
        clauses.append([nv, -rng.choice(board)])
    if rng.random() < 0.15:
        a = rng.choice(board)
        clauses.append([a, a, rng.choice(board)])   # a repeated literal
    if rng.random() < 0.15:
        a = rng.choice(board)
        clauses.append([a, -a, rng.choice(board)])  # a tautology
    if rng.random() < 0.2 and defs:
        c = rng.choice(defs)
        clauses.append(list(c))                # a duplicate clause
    if rng.random() < 0.5:
        rng.shuffle(clauses)
    # renamed variables, and negated node variables on a share of them
    if rng.random() < 0.5:
        perm = list(range(1, nv + 1))
        rng.shuffle(perm)
        flip_nodes = rng.random() < 0.1
        ren = {}
        for v in range(1, nv + 1):
            ren[v] = perm[v - 1] * (-1 if (flip_nodes and v > m) else 1)
        clauses = [[(ren[abs(l)] if l > 0 else -ren[abs(l)]) for l in c] for c in clauses]
    info = f"g={g} sizes={sizes} levels={len(levels)} k={k} exact={exact} style={style} faults={sorted(set(fault))}"
    return nv, clauses, info


def write_cnf(path, nv, clauses):
    with open(path, "w") as f:
        f.write(f"p cnf {nv} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


def solve(cnf, opts, proof=None):
    args = [str(SOLVER)] + STOCK + opts + [str(cnf)] + ([str(proof)] if proof else [])
    p = subprocess.run(args, capture_output=True, text=True, timeout=120)
    status = next((l[2:].strip() for l in p.stdout.splitlines() if l.startswith("s ")), "NONE")
    expected = {"SATISFIABLE": 10, "UNSATISFIABLE": 20}.get(status)
    if expected is None or p.returncode != expected:
        status = f"FAILED(rc={p.returncode},s={status})"
    model = set()
    for l in p.stdout.splitlines():
        if l.startswith("v "):
            model.update(int(t) for t in l[2:].split() if t != "0")
    fired = ("structure pass: decision-diagram count:" in p.stdout) and (", refuted" in p.stdout)
    return status, model, fired


def check_model(clauses, model):
    """Every clause holds a true literal, and no variable is true both ways."""
    if any(-l in model for l in model):
        return False
    return all(any(l in model for l in c) for c in clauses)


def one(args):
    seed, out = args
    d = Path(out) / f"s{seed}"
    d.mkdir(parents=True, exist_ok=True)
    cnf = d / "f.cnf"
    nv, clauses, info = build(seed)
    write_cnf(cnf, nv, clauses)
    verdict = "ok"
    stock, m0, _ = solve(cnf, [])
    on, m1, f1 = solve(cnf, ["--structdd=1"], d / "p.drat")
    on2, m2, f2 = solve(cnf, ["--structdd=1"])
    if not (stock == on == on2) or stock.startswith("FAILED"):
        verdict = f"DISAGREE stock={stock} on={on} noproof={on2}"
    elif f1 != f2:
        verdict = f"FIRED-DIFFERS proof={f1} noproof={f2}"
    elif stock == "SATISFIABLE":
        for name, mm in (("stock", m0), ("on", m1), ("noproof", m2)):
            if not check_model(clauses, mm):
                verdict = f"BAD-MODEL {name}"
    elif f1:
        g = subprocess.run([sys.executable, str(GATE), str(cnf), str(d / "p.drat")], capture_output=True, text=True, timeout=300)
        if g.stdout.strip() != "ok":
            verdict = f"GATE {g.stdout.strip()[:200]}"
    if verdict == "ok":
        for p in d.iterdir():
            p.unlink()
        d.rmdir()
    return seed, verdict, f"{info} stock={stock} fired={f1}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--out", default=str(ROOT / "log" / "dd-fuzz"))
    args = ap.parse_args()
    seeds = [(args.seed * 100000 + i, args.out) for i in range(args.count)]
    counts = {}
    fired = sat = unsat = 0
    with ProcessPoolExecutor(max_workers=args.jobs) as ex:
        for seed, verdict, info in ex.map(one, seeds):
            key = verdict.split()[0]
            counts[key] = counts.get(key, 0) + 1
            fired += "fired=True" in info
            sat += "stock=SATISFIABLE" in info
            unsat += "stock=UNSATISFIABLE" in info
            if verdict != "ok":
                print(f"seed {seed}: {verdict}: {info}", flush=True)
    print(f"{args.count} formulas: {counts}; {sat} SAT, {unsat} UNSAT by stock; the pass fired on {fired}")
    return 0 if set(counts) <= {"ok"} else 1


if __name__ == "__main__":
    sys.exit(main())
