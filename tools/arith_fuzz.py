#!/usr/bin/env python3
"""Fuzz the linear arithmetic pass (beads SAT-playground-skz.3, skz.12).

Random miters of two small multipliers over shared AND partial products,
with random shapes (array or column-wise, reduction orders, a constant one
folded into a random pair, complemented half adders, 2-input or XOR3 gates)
and, on a share of them, a random fault that makes the two sides differ
(a constant on one side only, a partial product dropped, a carry read at the
wrong weight, an inverted wire). Since skz.12 a side may also reduce its
columns to two rows and add them with a final adder (ripple, Kogge-Stone,
Brent-Kung, Sklansky or carry-skip, over generate/propagate half adders
written in random gate polarities), write a column's half-adder carry over
two other products of the same four inputs (as a mapper does), and keep
only the gates under the compared bit (dead logic removed: no lower sum
bits, only the carry into the compared column). For each: the solver with
the pass off and with it on must agree; a SAT answer's model is checked
against every clause; an UNSAT answer from a run where the pass fired goes
through the two-stage gate (tools/proof_gate.py).

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
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0", "--structdiv=0", "--gauss=0"]


class Circuit:
    def __init__(self, rng=None):
        self.n = 0
        self.clauses = []
        self.gates = {}      # output var -> (input literals, first clause index, end)
        self.rng = rng       # when set, gates come in random polarities (a mapped look)

    def var(self):
        self.n += 1
        return self.n

    def add(self, *lits):
        self.clauses.append(list(lits))

    def _gate(self, inputs, clauses_fn):
        o = self.var()
        start = len(self.clauses)
        clauses_fn(o)
        self.gates[o] = (list(inputs), start, len(self.clauses))
        return o

    def and2_plain(self, a, b):
        return self._gate([a, b], lambda o: (self.add(-o, a), self.add(-o, b), self.add(o, -a, -b)))

    def or2_plain(self, a, b):
        return self._gate([a, b], lambda o: (self.add(o, -a), self.add(o, -b), self.add(-o, a, b)))

    def and2(self, a, b):
        # the same function in a random gate shape: and, nor of the
        # negations (a nor gate), or the negation of a nand
        if self.rng is not None and self.rng.random() < 0.4:
            return -self.or2_plain(-a, -b)
        return self.and2_plain(a, b)

    def or2(self, a, b):
        if self.rng is not None and self.rng.random() < 0.4:
            return -self.and2_plain(-a, -b)
        return self.or2_plain(a, b)

    def xor2(self, a, b):
        if self.rng is not None and self.rng.random() < 0.3:
            # as the or of the two and-nots (a mapped xor; written as a
            # nor it is read negated)
            if self.rng.random() < 0.5:
                return self.or2_plain(self.and2_plain(a, -b), self.and2_plain(-a, b))
            return -self.and2_plain(-self.and2_plain(a, -b), -self.and2_plain(-a, b))
        if self.rng is not None and self.rng.random() < 0.3:
            return -self._gate([a, b], lambda o: (self.add(-o, a, -b), self.add(-o, -a, b), self.add(o, a, b), self.add(o, -a, -b)))  # xnor
        return self._gate([a, b], lambda o: (self.add(-o, a, b), self.add(-o, -a, -b), self.add(o, -a, b), self.add(o, a, -b)))

    def cone(self, roots):
        """The gate outputs under the roots (the roots included)."""
        seen = set()
        stack = [abs(r) for r in roots]
        while stack:
            v = stack.pop()
            if v in seen or v not in self.gates:
                continue
            seen.add(v)
            stack.extend(abs(l) for l in self.gates[v][0])
        return seen

    def prune(self, keep):
        """Keep the clauses of the gates in `keep` and every clause that is
        no gate's (the miter's): dead logic removed, as a synthesis tool
        does for a single compared bit."""
        owned = set()
        for v, (_, start, end) in self.gates.items():
            if v not in keep:
                owned.update(range(start, end))
        self.clauses = [c for i, c in enumerate(self.clauses) if i not in owned]

    def xor3(self, a, b, c):
        def clauses(o):
            for m in range(8):
                ma, mb, mc = m & 1, (m >> 1) & 1, (m >> 2) & 1
                par = ma ^ mb ^ mc
                self.add(a if ma == 0 else -a, b if mb == 0 else -b, c if mc == 0 else -c, o if par else -o)
        return self._gate([a, b, c], clauses)

    def maj3(self, a, b, c):
        def clauses(o):
            self.add(-o, a, b); self.add(-o, a, c); self.add(-o, b, c)
            self.add(o, -a, -b); self.add(o, -a, -c); self.add(o, -b, -c)
        if self.rng is not None and self.rng.random() < 0.3:
            return -self._gate([a, b, c], lambda o: clauses(-o))  # the complement, read negated
        return self._gate([a, b, c], clauses)

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


def column(c, cols, two, rng, plus_one, complement, reverse, rows=1, regroup=None):
    """Reduce every column to `rows` bits (1: the sum bits; 2: two rows for
    a final adder). `regroup`: (grid, index) to write a half adder over two
    partial products pp(i,j), pp(k,l) with its carry as the and of pp(i,l)
    and pp(k,j), the way a mapper shares product nodes, and its sum as the
    nor of that carry and the nor of the two."""
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
        while len(cols[k]) > rows:
            take = (lambda: cols[k].pop(0)) if reverse else (lambda: cols[k].pop())
            if len(cols[k]) >= 3:
                s, cy = c.fa(take(), take(), take(), two)
            else:
                x, y = take(), take()
                done = False
                if regroup is not None and rng.random() < 0.7:
                    grid, where = regroup
                    if x in where and y in where:
                        (i, j), (kk, l) = where[x], where[y]
                        if i != kk and j != l:
                            cy = c.and2(grid[i][l], grid[kk][j])
                            s = -c.or2(cy, -c.or2(x, y))
                            done = True
                if not done:
                    s, cy = c.ha(x, y, complement and rng.random() < 0.5)
            cols[k].append(s)
            if k + 1 < len(cols):
                cols[k + 1].append(cy)
        out.append(list(cols[k]))
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
    return [[x] for x in out + acc]


def final_adder(c, rows, kind, rng):
    """Add the two rows (a list per column of at most two bits) with a
    carry-propagate adder of the given kind; returns the sum bits."""
    width = len(rows)
    a = [col[0] if len(col) > 0 else None for col in rows]
    b = [col[1] if len(col) > 1 else None for col in rows]
    if kind == "ripple":
        out, carry = [], None
        for i in range(width):
            ins = [x for x in (a[i], b[i], carry) if x is not None]
            if len(ins) == 0:
                out.append(None); carry = None
            elif len(ins) == 1:
                out.append(ins[0]); carry = None
            elif len(ins) == 2:
                s, carry = c.ha(ins[0], ins[1], rng.random() < 0.3)
                out.append(s)
            else:
                s, carry = c.fa(ins[0], ins[1], ins[2], rng.choice([False, True, "exotic"]))
                out.append(s)
        return out
    # generate and propagate per column (a half adder; a lone bit is its own propagate)
    g, p = [], []
    for i in range(width):
        if a[i] is None and b[i] is None:
            g.append(None); p.append(None)
        elif b[i] is None:
            g.append(None); p.append(a[i])
        else:
            p.append(c.xor2(a[i], b[i])); g.append(c.and2(a[i], b[i]))
    # a missing generate is a constant zero, a missing propagate a constant
    # one (an empty column stops no carry)
    def gor(x, y):          # or of generates/carries
        return y if x is None else (x if y is None else c.or2(x, y))

    def pand(x, y):         # and of propagates
        return y if x is None else (x if y is None else c.and2(x, y))

    def pg(pbit, carry):    # propagate and carry
        return None if carry is None else (carry if pbit is None else c.and2(pbit, carry))

    def op2(hi, lo):
        gh, ph = hi
        gl, pl = lo
        return (gor(gh, pg(ph, gl)), pand(ph, pl))
    carries = [None] * (width + 1)   # carries[i] = carry into column i
    if kind == "kogge-stone":
        G = [(g[i], p[i]) for i in range(width)]   # G[i] = group over [i : i-span+1]
        span = 1
        while span < width:
            G = [op2(G[i], G[i - span]) if i - span >= 0 else G[i] for i in range(width)]
            span *= 2
        for i in range(width):
            carries[i + 1] = G[i][0]
    elif kind == "brent-kung":
        G = [(g[i], p[i]) for i in range(width)]
        # up-sweep
        d = 1
        while d < width:
            for i in range(2 * d - 1, width, 2 * d):
                G[i] = op2(G[i], G[i - d])
            d *= 2
        # down-sweep
        d //= 2
        while d >= 1:
            for i in range(3 * d - 1, width, 2 * d):
                G[i] = op2(G[i], G[i - d])
            d //= 2
        for i in range(width):
            carries[i + 1] = G[i][0]
    elif kind == "sklansky":
        G = [(g[i], p[i]) for i in range(width)]
        d = 1
        while d < width:
            for i in range(width):
                if (i // d) % 2 == 1:
                    j = (i // d) * d - 1
                    G[i] = op2(G[i], G[j])
            d *= 2
        for i in range(width):
            carries[i + 1] = G[i][0]
    elif kind == "carry-skip":
        # blocks: inside a block the carries ripple from zero, and the
        # carry in skips over the block through its propagate
        block = rng.choice([2, 3, 4])
        cin = None
        for start in range(0, width, block):
            end = min(start + block, width)
            cz = None
            P = None
            for i in range(start, end):
                cz = gor(g[i], pg(p[i], cz))
                P = pand(P, p[i])
                carries[i + 1] = gor(cz, pg(P, cin))
            cin = carries[end]
    else:
        raise ValueError(kind)
    out = []
    for i in range(width):
        ci = carries[i]
        if p[i] is None:
            out.append(None if ci is None else ci)
        elif ci is None:
            out.append(p[i])
        else:
            out.append(c.xor2(p[i], ci))
    return out


def build(seed):
    rng = random.Random(seed)
    pruned = rng.random() < 0.4
    n = rng.randint(5, 7) if pruned else rng.randint(3, 6)   # a pruned cone of a tiny multiplier has too few cells to be a miter
    two = rng.choice([False, True, "exotic"])
    mapped = rng.random() < 0.5
    c = Circuit(rng if mapped else None)
    A = [c.var() for _ in range(n)]
    B = [c.var() for _ in range(n)]
    pp = [[c.and2_plain(A[i], B[j]) for j in range(n)] for i in range(n)]
    where = {pp[i][j]: (i, j) for i in range(n) for j in range(n)}
    cols = [[] for _ in range(2 * n)]
    for i in range(n):
        for j in range(n):
            cols[i + j].append(pp[i][j])
    fault = None
    plus = rng.choice([None, None, rng.randrange(1, 2 * n - 2)])
    sides = []
    shapes = []
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
        shape = rng.choice(["column", "column", "array", "rows"])
        if shape == "array" and fault is None and side == 0 and plus is None:
            out = [col[0] for col in array(c, pp, two, rng, rng.random() < 0.3)]
        elif shape == "rows":
            regroup = (pp, where) if rng.random() < 0.5 else None
            rows = column(c, cs, two, rng, p_one, rng.random() < 0.3, rng.random() < 0.5, rows=2, regroup=regroup)
            kind = rng.choice(["ripple", "kogge-stone", "brent-kung", "sklansky", "carry-skip"])
            shape = f"rows+{kind}"
            out = final_adder(c, rows, kind, rng)
        else:
            regroup = (pp, where) if rng.random() < 0.3 else None
            out = [col[0] if col else None for col in column(c, cs, two, rng, p_one, rng.random() < 0.3, rng.random() < 0.5, rows=1, regroup=regroup)]
        sides.append(out)
        shapes.append(shape)
    width = min(len(sides[0]), len(sides[1]))
    cand = [k for k in range(width) if sides[0][k] is not None and sides[1][k] is not None and sides[0][k] != sides[1][k]]
    if not cand:
        return None
    # a pruned miter compares a high bit (a low one has a tiny cone)
    high = [k for k in cand if k >= n] if pruned else cand
    k = rng.choice(high or cand)
    if pruned:
        # dead logic removed: only the gates under the two compared bits
        c.prune(c.cone([sides[0][k], sides[1][k]]))
    # the compare itself is a plain xor (as the real miters write it)
    c.rng = None
    x = c.xor2(sides[0][k], sides[1][k])
    c.add(x)
    return c, fault, n, k, f"{shapes[0]}/{shapes[1]}{'/pruned' if pruned else ''}{'/mapped' if mapped else ''}"


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
    c, fault, n, k, shape = built
    d = Path(out) / f"f{seed}"
    d.mkdir(parents=True, exist_ok=True)
    cnf = d / "m.cnf"
    cnf.write_text(f"p cnf {c.n} {len(c.clauses)}\n" + "".join(" ".join(map(str, cl)) + " 0\n" for cl in c.clauses))
    off, m_off, _ = solve(cnf, [])
    on, m_on, fired = solve(cnf, ["--structarith=1"])
    proof = d / "p.drat"
    onp, m_onp, firedp = solve(cnf, ["--structarith=1"], proof)
    tag = f"n={n} bit={k} shape={shape} fault={fault} fired={fired}"
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
