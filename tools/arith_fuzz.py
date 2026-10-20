#!/usr/bin/env python3
"""Fuzz the linear arithmetic pass (bead SAT-playground-skz.3).

Random miters of two small multipliers over shared AND partial products,
with random shapes (array or column-wise, reduction orders, a constant one
folded into a random pair, complemented half adders, 2-input or XOR3 gates)
and, on a share of them, a random fault that makes the two sides differ
(a constant on one side only, a partial product dropped, a carry read at the
wrong weight, an inverted wire). For each: the solver with the pass off and
with it on must agree; a SAT answer's model is checked against every clause;
an UNSAT answer from a run where the pass fired goes through the two-stage
gate (tools/proof_gate.py).

Usage: python3 tools/arith_fuzz.py --count 500 [--seed 1] [--jobs 4] [--out DIR]
"""
from __future__ import annotations

import argparse
import os
import random
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOLVER = Path(os.environ.get("ARITH_FUZZ_SOLVER", str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver")))
GATE = ROOT / "tools" / "proof_gate.py"
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--gauss=0"]


class Circuit:
    def __init__(self):
        self.n = 0
        self.clauses = []

    def var(self):
        self.n += 1
        return self.n

    def add(self, *lits):
        self.clauses.append(list(lits))

    def and2(self, a, b):
        o = self.var()
        self.add(-o, a); self.add(-o, b); self.add(o, -a, -b)
        return o

    def or2(self, a, b):
        o = self.var()
        self.add(o, -a); self.add(o, -b); self.add(-o, a, b)
        return o

    def xor2(self, a, b):
        o = self.var()
        self.add(-o, a, b); self.add(-o, -a, -b); self.add(o, -a, b); self.add(o, a, -b)
        return o

    def xor3(self, a, b, c):
        o = self.var()
        for m in range(8):
            ma, mb, mc = m & 1, (m >> 1) & 1, (m >> 2) & 1
            par = ma ^ mb ^ mc
            self.add(a if ma == 0 else -a, b if mb == 0 else -b, c if mc == 0 else -c, o if par else -o)
        return o

    def maj3(self, a, b, c):
        o = self.var()
        self.add(-o, a, b); self.add(-o, a, c); self.add(-o, b, c)
        self.add(o, -a, -b); self.add(o, -a, -c); self.add(o, -b, -c)
        return o

    def fa(self, a, b, c, two):
        if two == "exotic":
            s = self.xor3(a, b, c)
            t = self.and2(self.or2(a, b), c)
            return s, self.or2(self.and2(a, b), self.xor2(self.xor2(t, b), b))
        if two:
            t = self.xor2(a, b)
            s = self.xor2(t, c)
            return s, self.or2(self.and2(a, b), self.and2(t, c))
        return self.xor3(a, b, c), self.maj3(a, b, c)

    def ha(self, a, b, complement):
        if complement:   # sum as xnor, carry as nand, both read negated
            return -self.xor2(-a, b), -self.or2(-a, -b)
        return self.xor2(a, b), self.and2(a, b)


def column(c, cols, two, rng, plus_one, complement, reverse):
    cols = [list(col) for col in cols]
    out = []
    for k in range(len(cols)):
        if plus_one is not None and k == plus_one and len(cols[k]) >= 2:
            i, j = rng.sample(range(len(cols[k])), 2)
            x, y = cols[k][i], cols[k][j]
            cols[k] = [v for t, v in enumerate(cols[k]) if t not in (i, j)]
            s = -c.xor2(x, y)          # x + y + 1 = xnor(x, y) + 2 or(x, y)
            cols[k].append(s)
            if k + 1 < len(cols):
                cols[k + 1].append(c.or2(x, y))
        while len(cols[k]) > 1:
            take = (lambda: cols[k].pop(0)) if reverse else (lambda: cols[k].pop())
            if len(cols[k]) >= 3:
                s, cy = c.fa(take(), take(), take(), two)
            else:
                s, cy = c.ha(take(), take(), complement and rng.random() < 0.5)
            cols[k].append(s)
            if k + 1 < len(cols):
                cols[k + 1].append(cy)
        out.append(cols[k][0] if cols[k] else None)
    return out


def array(c, pp, two, rng, complement):
    n = len(pp)
    acc = list(pp[0])
    out = [acc.pop(0)]
    for row in pp[1:]:
        new, carry = [], None
        for j in range(n):
            ins = [row[j]] + ([acc[j]] if j < len(acc) else []) + ([carry] if carry is not None else [])
            if len(ins) == 1:
                s, carry = ins[0], None
            elif len(ins) == 2:
                s, carry = c.ha(ins[0], ins[1], complement and rng.random() < 0.5)
            else:
                s, carry = c.fa(*ins, two)
            new.append(s)
        if carry is not None:
            new.append(carry)
        out.append(new.pop(0))
        acc = new
    return out + acc


def build(seed):
    rng = random.Random(seed)
    n = rng.randint(3, 6)
    two = rng.choice([False, True, "exotic"])
    c = Circuit()
    A = [c.var() for _ in range(n)]
    B = [c.var() for _ in range(n)]
    pp = [[c.and2(A[i], B[j]) for j in range(n)] for i in range(n)]
    cols = [[] for _ in range(2 * n)]
    for i in range(n):
        for j in range(n):
            cols[i + j].append(pp[i][j])
    fault = None
    plus = rng.choice([None, None, rng.randrange(1, 2 * n - 2)])
    sides = []
    for side in range(2):
        cs = [list(col) for col in cols]
        p_one = plus
        if side == 0 and rng.random() < 0.25:
            kind = rng.choice(["const", "drop", "shift", "invert", "duplicate"])
            fault = kind
            if kind == "const":
                p_one = rng.randrange(1, 2 * n - 2) if plus is None else None
            elif kind == "drop":
                k = rng.choice([k for k in range(2 * n) if len(cs[k]) >= 2])
                cs[k].pop(rng.randrange(len(cs[k])))
            elif kind == "shift":
                k = rng.choice([k for k in range(2 * n - 1) if len(cs[k]) >= 2])
                cs[k + 1].append(cs[k].pop(rng.randrange(len(cs[k]))))
            elif kind == "duplicate":
                k = rng.choice([k for k in range(2 * n) if cs[k]])
                cs[k].append(rng.choice(cs[k]))
            elif kind == "invert":
                k = rng.choice([k for k in range(2 * n) if cs[k]])
                t = rng.randrange(len(cs[k]))
                cs[k][t] = -cs[k][t]
        if rng.random() < 0.4 and fault is None and side == 0 and plus is None:
            out = array(c, pp, two, rng, rng.random() < 0.3)
        else:
            out = column(c, cs, two, rng, p_one, rng.random() < 0.3, rng.random() < 0.5)
        sides.append(out)
    width = min(len(sides[0]), len(sides[1]))
    cand = [k for k in range(width) if sides[0][k] is not None and sides[1][k] is not None and sides[0][k] != sides[1][k]]
    if not cand:
        return None
    k = rng.choice(cand)
    x = c.xor2(sides[0][k], sides[1][k])
    c.add(x)
    return c, fault, n, k


def solve(cnf, opts, proof=None):
    args = [str(SOLVER)] + STOCK + opts + [str(cnf)] + ([str(proof)] if proof else [])
    p = subprocess.run(args, capture_output=True, text=True, timeout=120)
    status = next((l[2:].strip() for l in p.stdout.splitlines() if l.startswith("s ")), "NONE")
    expected_rc = {"SATISFIABLE": 10, "UNSATISFIABLE": 20}.get(status)
    if expected_rc is None or p.returncode != expected_rc:
        status = f"FAILED(rc={p.returncode},s={status})"
    model = set()
    for l in p.stdout.splitlines():
        if l.startswith("v "):
            model.update(int(t) for t in l[2:].split() if t != "0")
    fired = "structure pass: arithmetic: two adder networks" in p.stdout
    return status, model, fired


def check_model(clauses, model):
    return all(any(l in model for l in cl) for cl in clauses)


def one(args):
    seed, out = args
    built = build(seed)
    if built is None:
        return seed, "skip", ""
    c, fault, n, k = built
    d = Path(out) / f"f{seed}"
    d.mkdir(parents=True, exist_ok=True)
    cnf = d / "m.cnf"
    cnf.write_text(f"p cnf {c.n} {len(c.clauses)}\n" + "".join(" ".join(map(str, cl)) + " 0\n" for cl in c.clauses))
    off, m_off, _ = solve(cnf, [])
    on, m_on, fired = solve(cnf, ["--structarith=1"])
    proof = d / "p.drat"
    onp, m_onp, firedp = solve(cnf, ["--structarith=1"], proof)
    tag = f"n={n} bit={k} fault={fault} fired={fired}"
    if any(st.startswith("FAILED") for st in (off, on, onp)):
        return seed, "NOANSWER", f"{tag}: off {off}, on {on}, on+proof {onp}"
    if not (off == on == onp):
        return seed, "MISMATCH", f"{tag}: off {off}, on {on}, on+proof {onp}"
    if off == "SATISFIABLE":
        for name, m in (("off", m_off), ("on", m_on), ("on+proof", m_onp)):
            if not check_model(c.clauses, m):
                return seed, "BADMODEL", f"{tag}: {name}"
    elif off == "UNSATISFIABLE" and firedp:
        verdict = subprocess.run([sys.executable, str(GATE), str(cnf), str(proof)], capture_output=True, text=True, timeout=300).stdout.strip().splitlines()
        verdict = verdict[-1] if verdict else "no verdict"
        if verdict != "ok":
            return seed, "GATE", f"{tag}: {verdict}"
    return seed, "ok", f"{tag} {off}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--out", default=str(ROOT / "log" / "arith-fuzz"))
    args = ap.parse_args()
    seeds = [(args.seed * 100000 + i, args.out) for i in range(args.count)]
    counts = {}
    fired_unsat = fired_sat = 0
    with ProcessPoolExecutor(max_workers=args.jobs) as ex:
        for seed, verdict, info in ex.map(one, seeds):
            counts[verdict] = counts.get(verdict, 0) + 1
            if "fired=True" in info and info.endswith("UNSATISFIABLE"):
                fired_unsat += 1
            if verdict not in ("ok", "skip"):
                print(f"seed {seed}: {verdict}: {info}", flush=True)
    print(f"{args.count} miters: {counts}; the pass fired and the answer was UNSAT on {fired_unsat}")
    return 0 if set(counts) <= {"ok", "skip"} else 1


if __name__ == "__main__":
    sys.exit(main())
