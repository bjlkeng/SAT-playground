#!/usr/bin/env python3
"""Toy check of the certificate for a Booth partial-product row (bead
SAT-playground-skz.4), before any solver code.

A radix-4 Booth digit d = -2 b2 + b1 + b0 takes a value in {-2, -1, 0, 1, 2}.
The row it makes from an n-bit a is a shifted or complemented copy of a
(the "+1" of the complement sits in the next row's lowest bit, so the
row alone has a term linear in b2). As an integer the row is

    sum_i w_i pp_i  =  sum_{t,k} c_tk a_t b_k + sum_t alpha_t a_t
                       + sum_k beta_k b_k + C

with the products a_t b_k the AND multiplier's partial products. That is
a polynomial identity of degree two; the pass proves it in cutting
planes by a case split over the digit's three bits: under each of the
eight cases every row bit is a constant or a literal of a single a bit
(the clause saying so is RUP through the row's gates), every product is
0 or a_t (RUP through its gate), and a weighted pol sum of those clauses
gives the identity plus a big-M multiple of the case's falsified
literals; with the identity's negation that is "some case literal is
false" (saturation), and the eight case clauses resolve to the empty
clause. Both directions, inside one pbc each.

The script builds the row from gates (the inverted form found in the
real cells: pp_i = xnor(b2, mux(one; a_i, a_{i-1}) and not zero)), reads
the polynomial off the truth table (so the proof does not know which
Booth variant it is), writes the two pbc derivations and checks them
with VeriPB; then checks that a tampered coefficient is rejected.

Usage: python3 tools/booth_toy_proof.py [--n 4] [--veripb PATH] [--out DIR]
"""
from __future__ import annotations

import argparse
import itertools
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

    def var(self):
        self.n += 1
        return self.n

    def add(self, *lits):
        self.clauses.append(list(lits))

    def gate(self, inputs, func):
        """o = func(inputs) by its minterm clauses (any truth table)."""
        o = self.var()
        for bits in itertools.product([0, 1], repeat=len(inputs)):
            val = func(*bits)
            self.add(*([-l if b else l for l, b in zip(inputs, bits)] + [o if val else -o]))
        return o

    def and2(self, a, b):
        return self.gate([a, b], lambda x, y: x and y)


def build(n):
    """The row and the products. Returns (circuit, a, b, pps, row_lits, weights)."""
    c = Circuit()
    a = [c.var() for _ in range(n)]
    b = [c.var() for _ in range(3)]  # b0, b1, b2: the digit's bits, low to high
    b0, b1, b2 = b
    one = c.gate([b1, b0], lambda x, y: x ^ y)
    zero = c.gate([b2, b1, b0], lambda x, y, z: x == y == z)
    row = []
    for i in range(n + 1):
        lo = a[i - 1] if i >= 1 else None
        hi = a[i] if i < n else None
        if lo is None:
            mux = c.gate([one, hi], lambda s, h: h if s else 0)
        elif hi is None:
            mux = c.gate([one, lo], lambda s, l: 0 if s else l)
        else:
            mux = c.gate([one, lo, hi], lambda s, l, h: h if s else l)
        andg = c.gate([mux, zero], lambda m, z: m and not z)
        pp = c.gate([b2, andg], lambda ng, x: not (ng ^ x))
        row.append(pp)
    pps = {(t, k): c.and2(a[t], b[k]) for t in range(n) for k in range(3)}
    weights = [1 << i for i in range(n + 1)]
    return c, a, b, pps, row, weights


def simulate(c, assignment):
    """Values of every variable under an assignment of the inputs (the
    gates in order: each clause set defines its output)."""
    val = dict(assignment)
    # gates were added in order; evaluate by finding the clause set of each
    # output: cheap brute force, the toy is tiny
    for v in range(1, c.n + 1):
        if v in val:
            continue
        for pick in (False, True):
            val[v] = pick
            ok = all(any((l > 0) == val.get(abs(l), None) for l in cl if abs(l) in val) or any(abs(l) not in val for l in cl) for cl in c.clauses)
            if ok:
                break
        else:
            raise RuntimeError("no value")
    return val


def polynomial(c, inputs, row, weights):
    """The multilinear polynomial of sum_i w_i pp_i over the inputs:
    {frozenset(vars): coefficient} by a Moebius transform."""
    m = len(inputs)
    table = {}
    for bits in itertools.product([0, 1], repeat=m):
        asg = {v: bool(x) for v, x in zip(inputs, bits)}
        val = simulate(c, asg)
        table[bits] = sum(w for w, p in zip(weights, row) if val[p])
    poly = {}
    for bits in itertools.product([0, 1], repeat=m):
        ones = [i for i, x in enumerate(bits) if x]
        coef = 0
        for sub in itertools.product([0, 1], repeat=len(ones)):
            sb = [0] * m
            for i, s in zip(ones, sub):
                sb[i] = s
            coef += (-1) ** (len(ones) - sum(sub)) * table[tuple(sb)]
        if coef:
            poly[frozenset(inputs[i] for i in ones)] = coef
    return poly


class Lin:
    """A linear form over variables plus a constant, with '~x = 1 - x'."""

    def __init__(self):
        self.coef = {}
        self.const = 0

    def add_lit(self, l, c):
        if l > 0:
            self.coef[l] = self.coef.get(l, 0) + c
        else:
            self.coef[-l] = self.coef.get(-l, 0) - c
            self.const += c
        return self

    def add(self, other, k=1):
        for v, c in other.coef.items():
            self.coef[v] = self.coef.get(v, 0) + k * c
        self.const += k * other.const
        return self

    def pruned(self):
        return {v: c for v, c in self.coef.items() if c}


def ge_terms(lin, rhs):
    """'lin >= rhs' as VeriPB terms over literals with positive
    coefficients and the normalised right-hand side."""
    terms = []
    r = rhs - lin.const
    for v, c in sorted(lin.coef.items()):
        if c > 0:
            terms.append((c, v))
        elif c < 0:
            terms.append((-c, -v))
            r += -c
    return terms, r


def lit(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


def pb(terms, rhs):
    return " ".join(f"{c} {lit(l)}" for c, l in terms) + f" >= {rhs}"


class Proof:
    def __init__(self, n_input):
        self.lines = ["pseudo-Boolean proof version 3.0"]
        self.next_id = n_input + 1

    def emit(self, s):
        self.lines.append(s)

    def new_id(self):
        i = self.next_id
        self.next_id += 1
        return i


def row_identity(proof, c, a, b, pps, row, weights, poly, direction, tamper=False):
    """Derive  sum w_i pp_i - target >= 0  (direction +1) or the reverse
    (direction -1) by pbc with the eight-case split. Returns its id."""
    # the target as a linear form over products, inputs and a constant
    target = Lin()
    for mono, coef in poly.items():
        if len(mono) == 0:
            target.const += coef
        elif len(mono) == 1:
            (v,) = mono
            target.add_lit(v, coef)
        elif len(mono) == 2:
            ts = [t for t in range(len(a)) if a[t] in mono]
            ks = [k for k in range(3) if b[k] in mono]
            if len(ts) != 1 or len(ks) != 1:
                raise RuntimeError(f"not a product monomial: {mono}")
            target.add_lit(pps[(ts[0], ks[0])], coef)
        else:
            raise RuntimeError(f"a monomial of degree {len(mono)}: the row is not a Booth row")
    if tamper:
        first = sorted(target.coef)[0]
        target.coef[first] += 1
    # the form to prove >= 0
    form = Lin()
    for w, p in zip(weights, row):
        form.add_lit(p, direction * w)
    form.add(target, -direction)
    terms, rhs = ge_terms(form, 0)
    # the negation's id and the subproof
    neg_id = proof.next_id
    proof.next_id += 1
    sub = []
    case_ids = []
    inputs = a + b
    for bits in itertools.product([0, 1], repeat=3):
        case = [b[k] if bits[k] else -b[k] for k in range(3)]  # the literals true in the case
        not_case = [-l for l in case]
        asg = {b[k]: bool(bits[k]) for k in range(3)}
        # every row bit under the case: a constant or a literal of one a bit
        facts = []  # (multiplier, Lin of the clause as 'sum >= 1', id)
        def rup(lits):
            i = proof.next_id
            proof.next_id += 1
            sub.append("    rup " + " ".join(f"1 {lit(l)}" for l in lits) + " >= 1 ;")
            l = Lin()
            for x in lits:
                l.add_lit(x, 1)
            return l, i
        # sum over the facts: we need  direction*(w_i pp_i) bounded below
        total = Lin()
        pol = []
        for w, p in zip(weights, row):
            # pp as a function of a under the case
            vals = {}
            for abits in itertools.product([0, 1], repeat=len(a)):
                full = dict(asg)
                full.update({a[t]: bool(abits[t]) for t in range(len(a))})
                vals[abits] = simulate(c, full)[p]
            if all(not v for v in vals.values()):
                red = None  # pp = 0
            elif all(vals.values()):
                red = 0  # pp = 1: a constant, as 'literal' 0 meaning true
            else:
                red = None
                for t in range(len(a)):
                    if all(vals[ab] == bool(ab[t]) for ab in vals):
                        red = a[t]
                        break
                    if all(vals[ab] != bool(ab[t]) for ab in vals):
                        red = -a[t]
                        break
                if red is None:
                    raise RuntimeError("a row bit that is not one a literal under a digit case")
            coef = direction * w
            # coef * pp >= coef * red(value): for coef > 0 need pp >= red: clause (~case or pp or ~red)
            # for coef < 0 need pp <= red: clause (~case or ~pp or red)
            if coef > 0:
                if red is None:
                    continue  # pp >= 0 is free
                lits = not_case + [p] + ([] if red == 0 else [-red])
            else:
                if red == 0:
                    continue  # pp <= 1 is free
                lits = not_case + [-p] + ([] if red is None else [red])
            l, i = rup(lits)
            total.add(l, abs(coef))
            pol.append((abs(coef), i))
        # the products under the case: P_tk = a_t when b_k is set, else 0
        for (t, k), pv in sorted(pps.items()):
            cf = -direction * target.coef.get(pv, 0)
            if cf == 0:
                continue
            if cf > 0:
                # need P >= value: only when b_k set: clause (~case or P or ~a_t)
                if not asg[b[k]]:
                    continue
                lits = not_case + [pv, -a[t]]
            else:
                lits = not_case + [-pv] + ([a[t]] if asg[b[k]] else [])
            l, i = rup(lits)
            total.add(l, abs(cf))
            pol.append((abs(cf), i))
        # 'total >= sum of multipliers' is what the pol sum says; it should
        # read  form + sum mu_j ~c_j + (literal axioms) >= K. Make up the
        # difference with literal axioms so the sum is exactly
        #   form - rhs + sum mu_j ~c_j >= 0
        want = Lin().add(form)
        diff = Lin().add(total).add(want, -1)  # total - form
        # diff must be a nonnegative combination of ~c_j and literal axioms
        # (x >= 0 or ~x >= 0) plus a constant; fix the a_t, b_k, product
        # and pp coefficients with axioms
        axioms = []
        extra = Lin()
        for v, cv in sorted(diff.pruned().items()):
            if v in b and cv != 0:
                continue  # the case literals: they are the big-M terms
            if cv > 0:
                # too much +v: add cv * (~v >= 0): total + cv*~v = total + cv - cv*v
                axioms.append((cv, -v))
                extra.add_lit(-v, cv)
            elif cv < 0:
                axioms.append((-cv, v))
                extra.add_lit(v, -cv)
        summed = Lin().add(total).add(extra)
        rest = Lin().add(summed).add(form, -1)
        # rest should be  sum mu_j * ~c_j + const  with every mu_j >= 0
        mus = {}
        for cl in case:
            v = abs(cl)
            cv = rest.coef.get(v, 0)
            # ~c = 1 - c for c positive literal: coefficient of v is -mu (c = v) or +mu (c = ~v)
            mu = -cv if cl > 0 else cv
            if mu < 0:
                raise RuntimeError("a case literal with a negative multiplier")
            mus[v] = mu
        expr = " ".join(f"{i} {m} *" for m, i in pol)
        expr = expr.replace(" *", " *", 1)
        parts = []
        for m, i in pol:
            parts.append(f"{i} {m} *")
        for m, l in axioms:
            parts.append(f"{lit(l)} {m} *")
        # reverse Polish: push each, then add them all
        rp = " ".join(parts) + " +" * (len(parts) - 1)
        # add the negation (form <= -1, i.e. -form >= 1 - ...) : the sum is
        # form + sum mu ~c >= K; with the negation -form >= 1 - rhs' ... let
        # VeriPB do the arithmetic: (sum) (negation) + then saturate
        sub.append(f"    pol {rp} {neg_id} + s ;")
        proof.next_id += 1
        case_ids.append(proof.next_id - 1)
    # resolve the eight case clauses to the empty clause
    def cl(lits):
        proof.next_id += 1
        sub.append("    rup " + " ".join(f"1 {lit(l)}" for l in lits) + " >= 1 ;")
    for x in (0, 1):
        for y in (0, 1):
            cl([-b[2] if x else b[2], -b[1] if y else b[1]])
        cl([-b[2] if x else b[2]])
    proof.next_id += 1
    sub.append("    rup >= 1 ;")
    proof.emit(f"pbc {pb(terms, rhs)} : subproof")
    for s in sub:
        proof.emit(s)
    proof.emit("qed;")
    return proof.new_id()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=4)
    ap.add_argument("--veripb", default=str(DEFAULT_VERIPB))
    ap.add_argument("--out", default=None)
    args = ap.parse_args()
    out = Path(args.out) if args.out else Path(os.environ.get("TMPDIR", "/tmp")) / "booth-toy"
    out.mkdir(parents=True, exist_ok=True)
    c, a, b, pps, row, weights = build(args.n)
    poly = polynomial(c, a + b, row, weights)
    degs = {}
    for mono, coef in poly.items():
        degs[len(mono)] = degs.get(len(mono), 0) + 1
    print(f"row polynomial over {len(a) + 3} inputs: {len(poly)} monomials by degree {dict(sorted(degs.items()))}")
    cnf = out / "row.cnf"
    with open(cnf, "w") as f:
        f.write(f"p cnf {c.n} {len(c.clauses)}\n")
        for cl in c.clauses:
            f.write(" ".join(map(str, cl)) + " 0\n")
    results = {}
    for name, tamper in (("proof", False), ("tampered", True)):
        proof = Proof(len(c.clauses))
        try:
            ge = row_identity(proof, c, a, b, pps, row, weights, poly, +1, tamper)
            le = row_identity(proof, c, a, b, pps, row, weights, poly, -1, tamper)
        except RuntimeError as e:
            print("cannot write:", e)
            return 1
        proof.emit("output NONE ;")
        proof.emit("conclusion NONE ;")
        proof.emit("end pseudo-Boolean proof ;")
        pbp = out / f"{name}.pbp"
        pbp.write_text("\n".join(proof.lines) + "\n")
        p = subprocess.run([args.veripb, str(cnf), str(pbp)], capture_output=True, text=True)
        ok = p.returncode == 0
        results[name] = ok
        tail = (p.stdout + p.stderr).strip().split("\n")[-1][:160]
        print(f"{name}: {'accepted' if ok else 'rejected'} ({len(proof.lines)} lines): {tail}")
    good = results["proof"] and not results["tampered"]
    print("booth toy:", "ok" if good else "FAILED")
    return 0 if good else 1


if __name__ == "__main__":
    sys.exit(main())
