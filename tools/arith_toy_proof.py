#!/usr/bin/env python3
"""Toy check of the linear arithmetic pass's certificate (bead SAT-playground-skz.2).

Builds a miter of two n x n multipliers over the same AND partial products
with different adder trees (an array multiplier and a column-wise one),
asserts that output bit k differs, and writes a VeriPB 3 proof that the
miter is unsatisfiable:

  1. per adder cell the two linear inequalities of
         la + lb (+ lc) = s + 2 c
     each by proof-by-contradiction with a short case split in rup steps;
  2. one weighted pol sum per side (the telescoping sum: every internal
     signal cancels, the partial products and the output bits remain);
  3. the two sides subtracted, so the shared partial products cancel and
     what is left says the two outputs are equal as integers;
  4. bit by bit from the top: weaken the lower bits and divide by the
     bit's weight, which gives the two clauses "bit of A = bit of B";
  5. the miter's compare against those clauses: contradiction.

Then checks the proof with VeriPB, and checks that a tampered proof (one
adder's weight doubled) is rejected.

Usage: python3 tools/arith_toy_proof.py --n 4 --bit 5 [--style xor3|and2] [--veripb PATH] [--out DIR]
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_VERIPB = ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb"


class Circuit:
    def __init__(self):
        self.n = 0
        self.clauses = []
        self.cells = []  # (kind, [leaf literals], sum literal, carry literal)

    def var(self):
        self.n += 1
        return self.n

    def add(self, *lits):
        self.clauses.append(list(lits))

    def and2(self, a, b):
        o = self.var()
        self.add(-o, a)
        self.add(-o, b)
        self.add(o, -a, -b)
        return o

    def or2(self, a, b):
        o = self.var()
        self.add(o, -a)
        self.add(o, -b)
        self.add(-o, a, b)
        return o

    def xor2(self, a, b):
        o = self.var()
        self.add(-o, a, b)
        self.add(-o, -a, -b)
        self.add(o, -a, b)
        self.add(o, a, -b)
        return o

    def xor3(self, a, b, c):
        o = self.var()
        for ma in (0, 1):
            for mb in (0, 1):
                for mc in (0, 1):
                    par = ma ^ mb ^ mc
                    self.add(a if ma == 0 else -a, b if mb == 0 else -b, c if mc == 0 else -c, o if par == 1 else -o)
        return o

    def maj3(self, a, b, c):
        o = self.var()
        self.add(-o, a, b)
        self.add(-o, a, c)
        self.add(-o, b, c)
        self.add(o, -a, -b)
        self.add(o, -a, -c)
        self.add(o, -b, -c)
        return o

    def full_adder(self, a, b, c, style):
        if style == "xor3":
            s, carry = self.xor3(a, b, c), self.maj3(a, b, c)
        else:
            t = self.xor2(a, b)
            s = self.xor2(t, c)
            carry = self.or2(self.and2(a, b), self.and2(t, c))
        self.cells.append(("FA", [a, b, c], s, carry))
        return s, carry

    def half_adder(self, a, b):
        s, carry = self.xor2(a, b), self.and2(a, b)
        self.cells.append(("HA", [a, b], s, carry))
        return s, carry


def array_multiplier(cnf, n, pp, style):
    """Row by row with ripple carries. Returns (output literals by weight, cells)."""
    start = len(cnf.cells)
    acc = [pp[0][j] for j in range(n)]
    out = [acc[0]]
    acc = acc[1:]
    for i in range(1, n):
        row = [pp[i][j] for j in range(n)]
        new = []
        carry = None
        for j in range(n):
            ins = [v for v in (row[j], acc[j] if j < len(acc) else None, carry) if v is not None]
            if len(ins) == 1:
                s, carry = ins[0], None
            elif len(ins) == 2:
                s, carry = cnf.half_adder(*ins)
            else:
                s, carry = cnf.full_adder(*ins, style)
            new.append(s)
        if carry is not None:
            new.append(carry)
        out.append(new[0])
        acc = new[1:]
    out.extend(acc)
    return out, cnf.cells[start:]


def column_multiplier(cnf, n, pp, style):
    """Column by column (Dadda-like)."""
    start = len(cnf.cells)
    cols = [[] for _ in range(2 * n)]
    for i in range(n):
        for j in range(n):
            cols[i + j].append(pp[i][j])
    out = []
    for k in range(2 * n):
        col = cols[k]
        while len(col) > 1:
            if len(col) >= 3:
                s, carry = cnf.full_adder(col.pop(), col.pop(), col.pop(), style)
            else:
                s, carry = cnf.half_adder(col.pop(), col.pop())
            col.append(s)
            if k + 1 < 2 * n:
                cols[k + 1].append(carry)
            else:
                pass  # the carry out of the top column is dropped (mod 2^2n); it is a dangling signal
        out.append(col[0] if col else None)
    return out, cnf.cells[start:]


def lit(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


def neg(l):
    return -l


class Proof:
    def __init__(self, n_input_constraints):
        self.lines = ["pseudo-Boolean proof version 3.0"]
        self.next_id = n_input_constraints + 1

    def emit(self, line):
        self.lines.append(line)

    def new_id(self):
        i = self.next_id
        self.next_id += 1
        return i

    def pbc(self, terms, rhs, subproof):
        """terms: (coef, literal); returns the id of the derived constraint."""
        body = " ".join(f"{c} {lit(l)}" for c, l in terms)
        self.emit(f"pbc {body} >= {rhs} : subproof")
        for s in subproof:
            self.emit("    " + s)
        self.emit("qed;")
        # VeriPB numbers the negated constraint and every subproof step
        # (deleted at qed), then the derived constraint
        self.next_id += 1 + len(subproof)
        return self.new_id()

    def pol(self, expr):
        self.emit(f"pol {expr} ;")
        return self.new_id()

    def rup_clause(self, lits):
        self.emit("rup " + " ".join(f"1 {lit(l)}" for l in lits) + " >= 1 ;")
        return self.new_id()


def rup(lits):
    return "rup " + " ".join(f"1 {lit(l)}" for l in lits) + " >= 1 ;" if lits else "rup >= 1 ;"


def cell_equalities(proof, cell, tamper=False):
    """The two inequalities of la + lb (+ lc) = s + 2c, as constraint ids (ge, le)
    where ge is  sum(leaves) - s - 2c >= 0  and le the reverse."""
    kind, leaves, s, c = cell
    k = len(leaves)
    # ge: leaves + ~s + 2 ~c >= 3   (sum(leaves) - s - 2c >= 0: -s is ~s - 1, -2c is 2~c - 2)
    ge_terms = [(1, l) for l in leaves] + [(1, neg(s)), (2, neg(c))]
    # le: ~leaves + s + 2 c >= k    (s + 2c - sum(leaves) >= 0: each -l is ~l - 1)
    le_terms = [(1, neg(l)) for l in leaves] + [(1, s), (2, c)]
    if kind == "FA":
        a, b, cc = leaves
        ge_sub = [rup([c]), rup([neg(s), a]), rup([neg(s), b]), rup([neg(s), cc]), rup([neg(s)]),
                  rup([neg(a)]), rup([neg(b)]), rup([neg(cc)]), rup([])]
        le_sub = [rup([neg(c)]), rup([s, neg(a)]), rup([s, neg(b)]), rup([s, neg(cc)]), rup([s]),
                  rup([a]), rup([b]), rup([cc]), rup([])]
    else:
        a, b = leaves
        # ge negated: c = 0 makes every other literal forced and breaks the xor;
        # c = 1 forces both inputs, the sum to 0, and the negation is then short by one
        ge_sub = [rup([c]), rup([])]
        # le negated forces c = 0; each input alone contradicts, then both zero does
        le_sub = [rup([neg(a)]), rup([neg(b)]), rup([])]
    ge = proof.pbc(ge_terms, 3 + (1 if tamper else 0), ge_sub)
    le = proof.pbc(le_terms, k, le_sub)
    return ge, le


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=4)
    ap.add_argument("--bit", type=int, default=5)
    ap.add_argument("--style", choices=["xor3", "and2"], default="xor3")
    ap.add_argument("--veripb", default=str(DEFAULT_VERIPB))
    ap.add_argument("--out", default="log/arith-toy-proof")
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    n = args.n
    if n < 3:
        print("n must be at least 3: on a 2-bit miter the tampered inequality is still implied by the rest, "
              "so the negative check means nothing")
        return 2
    cnf = Circuit()
    A = [cnf.var() for _ in range(n)]
    B = [cnf.var() for _ in range(n)]
    pp = [[cnf.and2(A[i], B[j]) for j in range(n)] for i in range(n)]
    oa, cells_a = array_multiplier(cnf, n, pp, args.style)
    ob, cells_b = column_multiplier(cnf, n, pp, args.style)
    k = args.bit
    if oa[k] == ob[k]:
        print(f"bit {k} is the same signal on both sides (a partial product that bypasses the adders): "
              f"the miter is refuted by unit propagation alone and the tampered check means nothing; pick another bit")
        return 2
    x = cnf.xor2(oa[k], ob[k])
    cnf.add(x)

    # weights: every cell's inputs carry the cell weight; the array and column
    # builders fix weights by construction, recover them by propagation from
    # the partial products (pp[i][j] has weight 2^(i+j))
    weight = {}
    for i in range(n):
        for j in range(n):
            weight[pp[i][j]] = i + j
    for cell in cells_a + cells_b:
        kind, leaves, s, c = cell
        e = weight[leaves[0]]
        assert all(weight[l] == e for l in leaves), cell
        weight[s] = e
        weight[c] = e + 1

    # the formula as OPB: clause i is constraint i
    opb = out / "toy.opb"
    with open(opb, "w") as f:
        f.write(f"* #variable= {cnf.n} #constraint= {len(cnf.clauses)}\n")
        for cl in cnf.clauses:
            f.write(" ".join(f"+1 {lit(l)}" for l in cl) + " >= 1 ;\n")
    with open(out / "toy.cnf", "w") as f:
        f.write(f"p cnf {cnf.n} {len(cnf.clauses)}\n")
        for cl in cnf.clauses:
            f.write(" ".join(map(str, cl)) + " 0\n")

    def write_proof(path, tamper_cell=None):
        proof = Proof(len(cnf.clauses))
        sides = []
        for cells, outs in ((cells_a, oa), (cells_b, ob)):
            ge_ids, le_ids = [], []
            for ci, cell in enumerate(cells):
                ge, le = cell_equalities(proof, cell, tamper=(tamper_cell is not None and cell is tamper_cell))
                ge_ids.append((ge, weight[cell[1][0]]))
                le_ids.append((le, weight[cell[1][0]]))
            # telescoping sums: sum_cells 2^e * ge  and  sum_cells 2^e * le
            def weighted_sum(ids):
                expr = []
                first = True
                for cid, e in ids:
                    expr.append(f"{cid} {2 ** e} *")
                    if not first:
                        expr.append("+")
                    first = False
                return proof.pol(" ".join(expr))
            sides.append((weighted_sum(ge_ids), weighted_sum(le_ids), outs))
        (ge_a, le_a, outs_a), (ge_b, le_b, outs_b) = sides
        # the partial products cancel between A's ge and B's le (and the reverse):
        # d1: sum 2^k outB_k - sum 2^k outA_k >= 0 ; d2 the reverse
        d1 = proof.pol(f"{ge_a} {le_b} +")
        d2 = proof.pol(f"{le_a} {ge_b} +")
        # the bits each side exposes: its output bits plus any dangling cell
        # output (the column multiplier drops its top carry; that carry is
        # still a root of the telescoped sum and has to be shown zero)
        def exposed(cells, outs):
            bits = {}
            for w, l in enumerate(outs):
                if l is not None:
                    bits[w] = l
            consumed = {l for kind, leaves, s_, c_ in cells for l in leaves}
            for kind, leaves, s_, c_ in cells:
                for l in (s_, c_):
                    if l not in consumed and l not in bits.values():
                        assert weight[l] not in bits, "two dangling signals of one weight"
                        bits[weight[l]] = l
            return bits
        bits_a = exposed(cells_a, outs_a)
        bits_b = exposed(cells_b, outs_b)
        weights = sorted(set(bits_a) | set(bits_b), reverse=True)
        cur1, cur2 = d1, d2   # cur1: sum_B - sum_A >= 0 on the remaining bits; cur2 the reverse
        for w in weights:
            lower_vars = [bits_a[u] for u in weights if u < w and u in bits_a] + \
                         [bits_b[u] for u in weights if u < w and u in bits_b]
            wk = " ".join(f"x{abs(v)} w" for v in lower_vars)
            a_here, b_here = w in bits_a, w in bits_b
            if a_here and b_here:
                c1 = proof.pol(f"{cur1} {wk} {2 ** w} d".strip())   # B_w + ~A_w >= 1
                c2 = proof.pol(f"{cur2} {wk} {2 ** w} d".strip())   # A_w + ~B_w >= 1
                cur1 = proof.pol(f"{cur1} {c2} {2 ** w} * +")
                cur2 = proof.pol(f"{cur2} {c1} {2 ** w} * +")
            elif b_here:
                c2 = proof.pol(f"{cur2} {wk} {2 ** w} d".strip())   # ~B_w >= 1: the dangling bit is zero
                cur1 = proof.pol(f"{cur1} {c2} {2 ** w} * +")
                cur2 = proof.pol(f"{cur2} x{abs(bits_b[w])} w")
            else:
                c1 = proof.pol(f"{cur1} {wk} {2 ** w} d".strip())   # ~A_w >= 1
                cur2 = proof.pol(f"{cur2} {c1} {2 ** w} * +")
                cur1 = proof.pol(f"{cur1} x{abs(bits_a[w])} w")
        # the miter: x = outA_k xor outB_k and x is a unit; with the two
        # equality clauses of bit k one case split closes it
        proof.rup_clause([bits_a[k]])
        proof.emit("rup >= 1 ;")
        proof.emit("output NONE ;")
        proof.emit("conclusion UNSAT ;")
        proof.emit("end pseudo-Boolean proof ;")
        with open(path, "w") as f:
            f.write("\n".join(proof.lines) + "\n")

    good = out / "toy.pbp"
    bad = out / "toy-tampered.pbp"
    write_proof(good)
    write_proof(bad, tamper_cell=cells_a[len(cells_a) // 2])
    print(f"toy: n={n} bit={k} style={args.style}: {cnf.n} vars, {len(cnf.clauses)} clauses, "
          f"cells A={len(cells_a)} B={len(cells_b)}; proof {sum(1 for _ in open(good))} lines")
    rc_all = 0
    for name, path, expect in (("proof", good, True), ("tampered proof", bad, False)):
        p = subprocess.run([args.veripb, str(opb), str(path)], capture_output=True, text=True)
        ok = p.returncode == 0 and "s VERIFIED UNSATISFIABLE" in p.stdout
        last = [l for l in (p.stdout + p.stderr).splitlines() if l.strip()][-1:]
        verdict = "accepted" if ok else "rejected"
        want = "accepted" if expect else "rejected"
        print(f"  {name}: {verdict} (wanted {want}) {'OK' if ok == expect else 'MISMATCH'}; last line: {last[0][:160] if last else ''}")
        with open(out / (path.stem + ".veripb.log"), "w") as f:
            f.write(p.stdout + p.stderr)
        if ok != expect:
            rc_all = 1
    return rc_all


if __name__ == "__main__":
    sys.exit(main())
