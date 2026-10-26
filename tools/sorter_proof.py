#!/usr/bin/env python3
"""Toy check of the sorter miter certificate (bead SAT-playground-skz.18).

A sorter miter compares two sorting programs over the same N words of W
bits, every output bit.  The census (tools/sorter_census.py) reads each
side back as a network of exchange units: W pairs of multiplexers under one
select s over two words X and Y, C = s ? Y : X and D = s ? X : Y.  Whatever
s is, {C, D} = {X, Y}, so each side's output is a rearrangement of its input
by construction.  What is left is that each side's output is sorted, and
that is easy for CDCL (0.1-1.5 s per adjacent pair on the 2025 cells,
against 250 s to a timeout for the miter).  The certificate, a VeriPB 3
proof:

  A. per side and adjacent output pair (k, k+1): the carry chain of
     out_k + ~out_k+1 as fresh variables (its carry out says out_k >
     out_k+1); solver 13 refutes the side plus that carry, its DRAT lemmas
     become the rup steps of one pbc, which gives "no carry out"; the carry
     cells' inequalities summed with weights 2^i turn that into the linear
     constraint  value(out_k+1) - value(out_k) >= 0;
  B. for every word w and threshold t in 1..2^W-1 a fresh g(w,t) with
     g -> value(w) >= t and ~g -> value(w) <= t-1 (two red steps);
  C. per exchange unit and t:  g(C,t) + g(D,t) = g(X,t) + g(Y,t), from the
     multiplexer clauses (eight implications, one pol line each, then two
     pbc steps with a case split on s);
  D. per side and t the unit equalities summed: every inner word cancels,
     so the number of outputs >= t equals the number of inputs >= t; the
     two sides' sums subtracted;
  E. per side, t and k:  g(out_k,t) -> g(out_k+1,t)  from A; then per t and
     k the two sides' g(out_k,t) are equal by one rup step each (two sorted
     0-1 vectors with the same count are equal);
  F. per output word, the bits from the thresholds, top bit first, by rup
     steps over the values of the higher bits;
  G. the miter's XORs are false, so its OR is: contradiction.

Then checks the proof with VeriPB and with tools/proof_gate.py, and checks
that tampered proofs are rejected (a unit's count off by one, a unit left
out of a side's sum, a carry weight changed).

Usage:
  python3 tools/sorter_proof.py toy --algs bubble,pancake --n 4 --w 3 [--bug] [--out DIR]
  python3 tools/sorter_proof.py prove CNF [--out DIR] [--no-tamper]
"""
from __future__ import annotations

import argparse
import os
import random
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sorter_census import HOLDOUT, census_cell, cone_clauses  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_VERIPB = ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb"
DEFAULT_SOLVER = ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"
# the sub-calls run the stock path without bounded variable addition, so
# every DRAT addition is a RUP clause over the lemma's own variables
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--gauss=0", "--factor=false"]

# ---------------------------------------------------------------------------
# the toy generator: the real cells' gate shapes
# ---------------------------------------------------------------------------

TRUE, FALSE = "T", "F"


class Gen:
    """Gates numbered after their inputs, as in the real cells.  Constants
    are folded; every gate output is a fresh positive variable."""

    def __init__(self):
        self.n = 0
        self.clauses = []
        self.drop_exchange = None   # the negative control: skip this exchange call (0 = the first)
        self.exchanges = 0

    def var(self):
        self.n += 1
        return self.n

    def add(self, *lits):
        self.clauses.append(list(lits))

    @staticmethod
    def neg(l):
        return FALSE if l is TRUE else TRUE if l is FALSE else -l

    def and_(self, lits):
        out = []
        for l in lits:
            if l is FALSE:
                return FALSE
            if l is not TRUE and l not in out:
                out.append(l)
        if any(-l in out for l in out):
            return FALSE
        if not out:
            return TRUE
        if len(out) == 1:
            return out[0]
        o = self.var()
        for l in out:
            self.add(-o, l)
        self.add(o, *[-l for l in out])
        return o

    def or_(self, lits):
        out = []
        for l in lits:
            if l is TRUE:
                return TRUE
            if l is not FALSE and l not in out:
                out.append(l)
        if any(-l in out for l in out):
            return TRUE
        if not out:
            return FALSE
        if len(out) == 1:
            return out[0]
        o = self.var()
        for l in out:
            self.add(o, -l)
        self.add(-o, *out)
        return o

    def not_(self, l):
        """An inverter gate (the real cells write one where a select is negated)."""
        if l in (TRUE, FALSE):
            return self.neg(l)
        o = self.var()
        self.add(o, l)
        self.add(-o, -l)
        return o

    def xor(self, a, b):
        o = self.var()
        self.add(-o, a, b)
        self.add(-o, -a, -b)
        self.add(o, -a, b)
        self.add(o, a, -b)
        return o

    def ite(self, s, t, e):
        assert s not in (TRUE, FALSE) and s > 0 and t > 0 and e > 0
        o = self.var()
        self.add(-s, -t, o)
        self.add(-s, t, -o)
        self.add(s, -e, o)
        self.add(s, e, -o)
        return o

    def gt(self, a, b):
        """a > b for LSB-first words, as the real cells write it: the top bit
        a & ~b, each lower bit a_i & ~b_i & (a_top == b_top) chained through
        u & (a_j == b_j), all of it ORed in a chain."""
        W = len(a)
        top = W - 1
        t_top = self.var()
        self.add(-t_top, a[top])
        self.add(-t_top, -b[top])
        self.add(t_top, -a[top], b[top])
        acc = t_top
        for i in range(W - 2, -1, -1):
            u = self.var()   # a_i & ~b_i & (a_top == b_top)
            self.add(-u, a[i])
            self.add(-u, -b[i])
            self.add(-u, -a[top], b[top])
            self.add(-u, a[top], -b[top])
            self.add(u, -a[i], b[i], a[top], b[top])
            self.add(u, -a[i], b[i], -a[top], -b[top])
            for j in range(W - 2, i, -1):
                v = self.var()   # u & (a_j == b_j)
                self.add(-v, u)
                self.add(-v, -a[j], b[j])
                self.add(-v, a[j], -b[j])
                self.add(v, -u, a[j], b[j])
                self.add(v, -u, -a[j], -b[j])
                u = v
            acc = self.or_([acc, u])
        return acc

    def exchange(self, s, X, Y):
        self.exchanges += 1
        if self.drop_exchange == self.exchanges - 1:
            return X, Y
        C = [self.ite(s, Y[i], X[i]) for i in range(len(X))]
        D = [self.ite(s, X[i], Y[i]) for i in range(len(X))]
        return C, D


def index_bits(N):
    return max(1, (N - 1).bit_length())


def eq_const(g, m, c):
    return g.and_([m[b] if (c >> b) & 1 else g.neg(m[b]) for b in range(len(m))])


def set_if(g, m, cond, j):
    """m := cond ? j : m, bit by bit."""
    return [g.or_([cond, m[b]]) if (j >> b) & 1 else g.and_([g.neg(cond), m[b]]) for b in range(len(m))]


def bubble(g, lanes):
    lanes = list(lanes)
    N = len(lanes)
    for p in range(N - 1):
        for j in range(N - 1 - p):
            s = g.gt(lanes[j], lanes[j + 1])
            lanes[j], lanes[j + 1] = g.exchange(s, lanes[j], lanes[j + 1])
    return lanes


def pancake(g, lanes):
    """For each size: the index of the first maximum (a binary register,
    x_j > x[m] read as OR_i (m == i and x_j > x_i)), then flip(0..m) and
    flip(0..size-1) merged as in the real cells: under m == c the swaps of
    flip(0..c) for c below size-1, then under m != size-1 the swaps of
    flip(0..size-1)."""
    lanes = list(lanes)
    N = len(lanes)
    B = index_bits(N)
    for size in range(N, 1, -1):
        m = [FALSE] * B
        for j in range(1, size):
            cond = g.or_([g.and_([eq_const(g, m, i), g.gt(lanes[j], lanes[i])]) for i in range(j)])
            m = set_if(g, m, cond, j)
        for c in range(1, size - 1):
            sel = eq_const(g, m, c)
            for p in range((c + 1) // 2):
                lanes[p], lanes[c - p] = g.exchange(sel, lanes[p], lanes[c - p])
        sel = g.not_(eq_const(g, m, size - 1))
        for p in range(size // 2):
            q = size - 1 - p
            lanes[p], lanes[q] = g.exchange(sel, lanes[p], lanes[q])
    return lanes


def selection(g, lanes):
    """For each position i: the index of the first minimum of the rest
    (x_j < x[m] read as OR_l (m == l and x_l > x_j)), then swap(i, m) as
    one conditional swap per candidate j under m == j."""
    lanes = list(lanes)
    N = len(lanes)
    B = index_bits(N)
    for i in range(N - 1):
        m = [TRUE if (i >> b) & 1 else FALSE for b in range(B)]
        for j in range(i + 1, N):
            cond = g.or_([g.and_([eq_const(g, m, l), g.gt(lanes[l], lanes[j])]) for l in range(i, j)])
            m = set_if(g, m, cond, j)
        for j in range(i + 1, N):
            sel = eq_const(g, m, j)
            lanes[i], lanes[j] = g.exchange(sel, lanes[i], lanes[j])
    return lanes


ALGS = {"bubble": bubble, "pancake": pancake, "selection": selection}


def make_toy(alg_a, alg_b, N, W, path, bug=False):
    """bug: side A skips its first exchange, so it no longer sorts and the
    miter is satisfiable."""
    g = Gen()
    words = [[g.var() for _ in range(W)] for _ in range(N)]
    g.drop_exchange = 0 if bug else None
    A = ALGS[alg_a](g, words)
    g.drop_exchange = None
    B = ALGS[alg_b](g, words)
    diffs = []
    for k in range(N):
        diffs.append(g.or_([g.xor(A[k][i], B[k][i]) for i in range(W)]))
    root = g.or_(diffs)
    g.add(root)
    with open(path, "w") as f:
        f.write(f"p cnf {g.n} {len(g.clauses)}\n")
        for c in g.clauses:
            f.write(" ".join(map(str, c)) + " 0\n")
    return g.n, len(g.clauses)


# ---------------------------------------------------------------------------
# the proof
# ---------------------------------------------------------------------------


def lit(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


def terms_str(terms):
    return " ".join(f"{c} {lit(l)}" for c, l in terms)


class Proof:
    def __init__(self, path, n_constraints):
        self.f = open(path, "w")
        self.next_id = n_constraints + 1
        self.lines = 0
        self.emit("pseudo-Boolean proof version 3.0")

    def emit(self, line):
        self.f.write(line + "\n")
        self.lines += 1

    def _id(self):
        i = self.next_id
        self.next_id += 1
        return i

    def red(self, terms, rhs, var, val):
        self.emit(f"red {terms_str(terms)} >= {rhs} : x{var} -> {val} ;")
        return self._id()

    def pol(self, expr):
        self.emit(f"pol {expr} ;")
        return self._id()

    def rup(self, lits):
        self.emit("rup " + " ".join(f"1 {lit(l)}" for l in lits) + " >= 1 ;" if lits else "rup >= 1 ;")
        return self._id()

    def pbc(self, terms, rhs, sub):
        """sub: subproof lines; VeriPB numbers the negated constraint and every
        subproof step (deleted at qed), then the derived constraint."""
        self.emit(f"pbc {terms_str(terms)} >= {rhs} : subproof")
        for s in sub:
            self.emit("    " + s)
        self.emit("qed;")
        self.next_id += 1 + len(sub)
        return self._id()

    def close(self):
        self.emit("output NONE ;")
        self.emit("conclusion UNSAT ;")
        self.emit("end pseudo-Boolean proof ;")
        self.f.close()


def rup_line(lits):
    return "rup " + " ".join(f"1 {lit(l)}" for l in lits) + " >= 1 ;" if lits else "rup >= 1 ;"


def sum_expr(ids_weights):
    """RPN for sum of weight * constraint."""
    expr = []
    for n, (cid, w) in enumerate(ids_weights):
        expr.append(f"{cid}" if w == 1 else f"{cid} {w} *")
        if n:
            expr.append("+")
    return " ".join(expr)


def positions_by_simulation(net, wd, outs, N, W, rnd):
    """Output words of a side in sorted order (position 0 the smallest),
    from inputs with distinct values; None if no consistent order."""
    if N > (1 << W):
        return None
    order = None
    for _ in range(64):
        vals = rnd.sample(range(1 << W), N)
        inputs = {}
        for k in range(N):
            for i, v in enumerate(wd.words[k]):
                inputs[v] = (vals[k] >> i) & 1
        val = net.simulate(inputs, 1)
        got = {w: sum(val[v] << i for i, v in enumerate(wd.words[w])) for w in outs}
        o = sorted(outs, key=lambda w: got[w])
        if sorted(got.values()) != sorted(vals):
            return None
        if order is None:
            order = o
        elif order != o:
            return None
    return order


def run_solver(solver, cnf, drat, timeout):
    t0 = time.time()
    p = subprocess.run([str(solver), "-q", "--binary=false", *STOCK, str(cnf), str(drat)], capture_output=True, text=True,
                       timeout=timeout)
    st = [l for l in p.stdout.splitlines() if l.startswith("s ")]
    return (st[0] if st else "s ?"), time.time() - t0


def drat_additions(path):
    adds = []
    with open(path) as f:
        for line in f:
            if not line.strip() or line[0] in "dc":
                continue
            lits = [int(x) for x in line.split()]
            assert lits[-1] == 0
            adds.append(lits[:-1])
    return adds


def write_proof(cnf_path, out, pbp, solver, timeout, tamper=None, cache=None, order_from="simulation"):
    """Write the proof for the sorter miter in cnf_path to pbp; the solver's
    lemma files go to out.  Returns (pbp, netlist, stats).  The output
    positions come from simulation; order_from="lanes" takes them from the
    lanes instead (the negative control, so that the solver is what refuses
    a side that does not sort)."""
    row, net, wd, sides, full_pairs, side_of_word = census_cell(cnf_path)
    N, W = wd.N, wd.W
    M = (1 << W) - 1
    if len(sides) != 2 or len(full_pairs) != N or row["mux_bits"] or row["max_data_readers"] != 1:
        raise SystemExit(f"not a two-sided exchange network: {row}")
    rnd = random.Random(7)
    # sides: units, inputs, outputs by position
    side_info = []
    for si, (outs, us) in enumerate(sides):
        if order_from == "lanes":
            by_lane = {wd.lane[w]: w for w in outs}
            order = [by_lane[k] for k in range(N)] if sorted(by_lane) == list(range(N)) else None
        else:
            order = positions_by_simulation(net, wd, sorted(outs), N, W, rnd)
        if order is None:
            raise SystemExit(f"side {si}: outputs not sorted in one order")
        units = sorted(us)
        # telescoping check: +1 per unit output, -1 per unit input
        bal = defaultdict(int)
        for u in units:
            U = wd.units[u]
            bal[U["C"]] += 1
            bal[U["D"]] += 1
            bal[U["X"]] -= 1
            bal[U["Y"]] -= 1
        want = defaultdict(int)
        for w in order:
            want[w] += 1
        for k in range(N):
            want[k] -= 1
        if {w: c for w, c in bal.items() if c} != {w: c for w, c in want.items() if c}:
            raise SystemExit(f"side {si}: the units do not telescope from the inputs to the outputs")
        side_info.append(dict(units=units, order=order))
    pos_of = [{w: k for k, w in enumerate(s["order"])} for s in side_info]
    pairs = []
    for a, b in full_pairs:
        if side_of_word[a] == 1:
            a, b = b, a
        if pos_of[0][a] != pos_of[1][b]:
            raise SystemExit("the miter compares words at different positions")
        pairs.append((pos_of[0][a], a, b))
    pairs.sort()

    clause_id = {}
    for i, c in enumerate(net.clauses):
        clause_id.setdefault(frozenset(c), i + 1)
    P = Proof(pbp, len(net.clauses))
    next_var = [net.n_vars]

    def fresh():
        next_var[0] += 1
        return next_var[0]

    stats = dict(N=N, W=W, units=[len(s["units"]) for s in side_info], lemma_s=0.0, lemma_lemmas=0)

    # A. sortedness of each side, by a solver call per adjacent pair
    sort_id = {}
    for si, s in enumerate(side_info):
        outs = s["order"]
        base_ids = cone_clauses(net, wd, [v for w in outs for v in wd.words[w]])
        base = [net.clauses[i] for i in base_ids]
        for k in range(N - 1):
            a = wd.words[outs[k]]
            b = wd.words[outs[k + 1]]
            cs = [fresh() for _ in range(W)]    # cs[i] = carry into bit i+1
            extra = []
            # c1 = a0 & ~b0 ; c(i+1) = maj(a_i, ~b_i, c_i)
            defs = []   # (clause, var, value) in red order
            c1 = cs[0]
            defs += [([-c1, a[0]], c1, 0), ([-c1, -b[0]], c1, 0), ([c1, -a[0], b[0]], c1, 1)]
            for i in range(1, W):
                c, ci = cs[i], cs[i - 1]
                defs += [([-c, a[i], -b[i]], c, 0), ([-c, a[i], ci], c, 0), ([-c, -b[i], ci], c, 0),
                         ([c, -a[i], b[i]], c, 1), ([c, -a[i], -ci], c, 1), ([c, b[i], -ci], c, 1)]
            ids = []
            for cl, v, val in defs:
                ids.append(P.red([(1, l) for l in cl], 1, v, val))
                extra.append(cl)
            key = (si, k)
            lemma_cnf = out / f"lemma_s{si}_k{k}.cnf"
            drat = out / f"lemma_s{si}_k{k}.drat"
            n_lemma_vars = next_var[0]
            with open(lemma_cnf, "w") as f:
                cls = base + extra + [[cs[W - 1]]]
                f.write(f"p cnf {n_lemma_vars} {len(cls)}\n")
                for c in cls:
                    f.write(" ".join(map(str, c)) + " 0\n")
            if cache is not None and key in cache:
                st, dt = cache[key]
            else:
                st, dt = run_solver(solver, lemma_cnf, drat, timeout)
                if cache is not None:
                    cache[key] = (st, dt)
            if st != "s UNSATISFIABLE":
                raise SystemExit(f"side {si} pair {k}: solver says {st}")
            adds = drat_additions(drat)
            if any(abs(l) > n_lemma_vars for c in adds for l in c):
                raise SystemExit(f"side {si} pair {k}: the DRAT proof adds variables (an extension step, not RUP)")
            stats["lemma_s"] += dt
            stats["lemma_lemmas"] += len(adds)
            sub = [rup_line(c) for c in adds]
            if not adds or adds[-1]:
                sub.append("rup >= 1 ;")
            no_carry = P.pbc([(1, -cs[W - 1])], 1, sub)
            # carry cells: cell_0 = (c1 | ~a0 | b0) + c1 ; cell_i = (three "-> c" clauses) / 2
            cell = [P.pol(f"{ids[2]} x{c1} +")]
            for i in range(1, W):
                j = 3 + 6 * (i - 1)
                cell.append(P.pol(f"{ids[j + 3]} {ids[j + 4]} + {ids[j + 5]} + 2 d"))
            weights = [(cell[i], 1 << i) for i in range(W)] + [(no_carry, 1 << W)]
            if tamper == "carry" and si == 0 and k == (N - 1) // 2:
                if W > 1:
                    weights[1] = (cell[1], 1)    # a carry cell at the wrong weight
                else:
                    weights.pop()                # the no-carry fact left out
            sort_id[key] = P.pol(sum_expr(weights))   # sum 2^i ~a_i + sum 2^i b_i >= M
    stats["sort_lines"] = P.lines

    # B. thresholds for every word of either side
    words_used = set(range(N))
    for s in side_info:
        for u in s["units"]:
            U = wd.units[u]
            words_used |= {U["X"], U["Y"], U["C"], U["D"]}
    gv, G1, G2 = {}, {}, {}
    for w in sorted(words_used):
        bits = wd.words[w]
        for t in range(1, M + 1):
            v = fresh()
            gv[w, t] = v
            G1[w, t] = P.red([(1 << i, b) for i, b in enumerate(bits)] + [(t, -v)], t, v, 0)
            G2[w, t] = P.red([(1 << i, -b) for i, b in enumerate(bits)] + [(M - t + 1, v)], M - t + 1, v, 1)

    # C. per unit and t: the count is kept
    def clause(lits):
        cid = clause_id.get(frozenset(lits))
        return cid if cid is not None else P.rup(lits)

    ge, le = {}, {}
    tamper_unit = side_info[1]["units"][len(side_info[1]["units"]) // 2]
    for s in side_info:
        for u in s["units"]:
            U = wd.units[u]
            sel = U["sel"]
            X, Y, C, D = (wd.words[U[n]] for n in ("X", "Y", "C", "D"))
            per_j = [[] for _ in range(8)]
            for i in range(W):
                x, y, c, d = wd.res(X[i]), wd.res(Y[i]), C[i], D[i]
                for j, cl in enumerate(([-sel, -y, c], [-sel, y, -c], [sel, -x, c], [sel, x, -c],
                                        [-sel, -x, d], [-sel, x, -d], [sel, -y, d], [sel, y, -d])):
                    per_j[j].append((clause(cl), 1 << i))
            L = [P.pol(sum_expr(per_j[j])) for j in range(8)]
            nX, nY, nC, nD = U["X"], U["Y"], U["C"], U["D"]
            for t in range(1, M + 1):
                def imp(src, dst, j):
                    P.pol(f"{G1[src, t]} {L[j]} + {G2[dst, t]} + {M} d")
                imp(nY, nC, 0); imp(nC, nY, 1); imp(nX, nC, 2); imp(nC, nX, 3)
                imp(nX, nD, 4); imp(nD, nX, 5); imp(nY, nD, 6); imp(nD, nY, 7)
                gX, gY, gC, gD = gv[nX, t], gv[nY, t], gv[nC, t], gv[nD, t]
                rhs = 2 + (1 if tamper == "count" and u == tamper_unit and t == (M + 1) // 2 else 0)
                ge[u, t] = P.pbc([(1, gC), (1, gD), (1, -gX), (1, -gY)], rhs,
                                 [rup_line([sel, -gX]), rup_line([sel]), rup_line([-gX]), "rup >= 1 ;"])
                le[u, t] = P.pbc([(1, -gC), (1, -gD), (1, gX), (1, gY)], 2,
                                 [rup_line([sel, gX]), rup_line([sel]), rup_line([gX]), "rup >= 1 ;"])

    # D. per side and t the sums; the two sides against each other
    diff = {}
    for t in range(1, M + 1):
        cnt = []
        for s in side_info:
            us = [u for u in s["units"] if not (tamper == "drop" and u == tamper_unit and t == (M + 1) // 2)]
            cnt.append((P.pol(sum_expr([(ge[u, t], 1) for u in us])), P.pol(sum_expr([(le[u, t], 1) for u in us]))))
        (ge0, le0), (ge1, le1) = cnt
        diff[t] = (P.pol(f"{ge0} {le1} +"), P.pol(f"{ge1} {le0} +"))

    # E. sorted thresholds, then equal thresholds position by position
    for t in range(1, M + 1):
        for s in side_info:
            si = side_info.index(s)
            o = s["order"]
            for k in range(N - 1):
                P.pol(f"{sort_id[si, k]} {G1[o[k], t]} + {G2[o[k + 1], t]} + {M} d")
        for k, a, b in pairs:
            P.rup([-gv[a, t], gv[b, t]])
            P.rup([-gv[b, t], gv[a, t]])

    # F. bits from thresholds, top bit first
    for k, a, b in pairs:
        A = wd.words[a]
        B = wd.words[b]

        def tree(i, fixed):
            depth = len(fixed)
            if depth < W - 1 - i:
                j = W - 1 - depth
                tree(i, fixed + [(j, 0)])
                tree(i, fixed + [(j, 1)])
            base = [A[j] if v == 0 else -A[j] for j, v in fixed]
            P.rup(base + [-A[i], B[i]])
            P.rup(base + [A[i], -B[i]])

        for i in range(W - 1, -1, -1):
            tree(i, [])

    # G. the miter's compares
    xors = {}
    for v, (sup, tt) in net.gates.items():
        if len(sup) == 2 and tt in (0b0110, 0b1001):
            xors[frozenset(wd.res(u) for u in sup)] = (v, tt)
    for k, a, b in pairs:
        for i in range(W):
            x, tt = xors[frozenset((wd.words[a][i], wd.words[b][i]))]
            if tt == 0b0110:
                P.rup([-x, wd.words[a][i]])
                P.rup([-x])
            else:
                P.rup([x, wd.words[a][i]])
                P.rup([x])
    P.rup([])
    P.close()
    stats["lines"] = P.lines
    stats["vars"] = next_var[0]
    stats["bytes"] = pbp.stat().st_size
    return pbp, net, stats


def write_opb(net, path):
    with open(path, "w") as f:
        f.write(f"* #variable= {net.n_vars} #constraint= {len(net.clauses)}\n")
        for c in net.clauses:
            f.write(" ".join(f"+1 {lit(l)}" for l in c) + " >= 1 ;\n")


def veripb_check(veripb, opb, pbp, timeout=3600):
    t0 = time.time()
    p = subprocess.run([str(veripb), str(opb), str(pbp)], capture_output=True, text=True, timeout=timeout)
    ok = p.returncode == 0 and "s VERIFIED UNSATISFIABLE" in p.stdout
    tail = [l for l in (p.stdout + p.stderr).splitlines() if l.strip()]
    return ok, time.time() - t0, (tail[-1] if tail else "")


def gate_check(cnf, pbp, out):
    """tools/proof_gate.py on the refutation: an empty DRAT file with the
    VeriPB proof and an empty derived-clause file beside it."""
    from proof_gate import check_unsat  # noqa: E402
    drat = out / "gate.drat"
    drat.write_text("")
    (out / "gate.drat.pbp").write_text(pbp.read_text())
    (out / "gate.drat.derived.cnf").write_text("p cnf 0 0\n")
    return check_unsat(cnf, drat)


def prove_and_check(cnf, out, args, log=print):
    out.mkdir(parents=True, exist_ok=True)
    cache = {}
    t0 = time.time()
    pbp, net, stats = write_proof(cnf, out, out / "proof.pbp", args.solver, args.timeout, cache=cache)
    t_write = time.time() - t0
    opb = out / "input.opb"
    write_opb(net, opb)
    ok, dt, last = veripb_check(args.veripb, opb, pbp)
    log(f"{Path(cnf).name}: N={stats['N']} W={stats['W']} units {stats['units']}; sortedness calls {stats['lemma_s']:.1f} s, "
        f"{stats['lemma_lemmas']} DRAT lemmas; proof {stats['lines']} lines ({stats['bytes'] / 1e6:.1f} MB, "
        f"{stats['sort_lines']} of them the sortedness part), written in {t_write:.1f} s")
    log(f"  VeriPB: {'accepted' if ok else 'REJECTED'} in {dt:.1f} s; last line: {last[:150]}")
    rc = 0 if ok else 1
    if ok and not args.no_gate:
        g = gate_check(cnf, pbp, out)
        log(f"  proof_gate.py: {g}")
        if g != "ok":
            rc = 1
    if not args.no_tamper:
        for kind in ("count", "drop", "carry"):
            tdir = out / f"tamper-{kind}"
            tdir.mkdir(exist_ok=True)
            bad, _, _ = write_proof(cnf, out, tdir / "proof.pbp", args.solver, args.timeout, tamper=kind, cache=cache)
            if bad.read_bytes() == pbp.read_bytes():
                log(f"  tampered ({kind}): the tamper did not apply (the proof is unchanged) MISMATCH")
                rc = 1
                continue
            tok, tdt, tlast = veripb_check(args.veripb, opb, bad)
            log(f"  tampered ({kind}): {'ACCEPTED' if tok else 'rejected'} in {tdt:.1f} s "
                f"{'MISMATCH' if tok else 'OK'}; last line: {tlast[:110]}")
            if tok:
                rc = 1
    return rc


def cmd_toy(args):
    alg_a, alg_b = args.algs.split(",")
    if (args.n < 3 or args.w < 2) and not args.no_tamper and not args.bug:
        print("the tampered checks need N >= 3 and W >= 2: on smaller toys unit propagation over the rest "
              "implies the tampered step, so a rejection means nothing (pass --no-tamper to check the proof alone)")
        return 2
    out = Path(args.out or f"log/sorter-toy-proof/{alg_a}-{alg_b}-{args.n}-{args.w}")
    out.mkdir(parents=True, exist_ok=True)
    name = f"toy-{alg_a.capitalize()}Vs{alg_b.capitalize()}Sort_{args.n}_{args.w}{'-bug' if args.bug else ''}.cnf"
    cnf = out / name
    nv, nc = make_toy(alg_a, alg_b, args.n, args.w, cnf, bug=args.bug)
    print(f"toy {name}: {nv} vars, {nc} clauses")
    # the toy must be the shape of the real cells, and unsatisfiable (satisfiable with the bug)
    st, dt = run_solver(args.solver, cnf, out / "direct.drat", args.timeout)
    print(f"  direct solve: {st} in {dt:.2f} s")
    if args.bug:
        if st != "s SATISFIABLE":
            return 1
        try:
            write_proof(cnf, out, out / "proof.pbp", args.solver, args.timeout, order_from="lanes")
        except SystemExit as e:
            print(f"  no proof written, as it should be: {e}")
            return 0
        print("  MISMATCH: a proof was written for a satisfiable miter")
        return 1
    if st != "s UNSATISFIABLE":
        return 1
    return prove_and_check(cnf, out, args)


def cmd_prove(args):
    cnf = Path(args.cnf)
    if HOLDOUT in cnf.resolve().parents:
        print("the 2026 suite is the holdout", file=sys.stderr)
        return 2
    out = Path(args.out or f"log/sorter-proof/{cnf.name.split('.')[0]}")
    return prove_and_check(cnf, out, args)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
                                 allow_abbrev=False)
    ap.add_argument("--veripb", default=str(DEFAULT_VERIPB))
    ap.add_argument("--solver", default=str(DEFAULT_SOLVER))
    ap.add_argument("--timeout", type=float, default=600.0, help="per solver call")
    ap.add_argument("--no-tamper", action="store_true")
    ap.add_argument("--no-gate", action="store_true")
    sub = ap.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("toy")
    t.add_argument("--algs", default="bubble,pancake")
    t.add_argument("--n", type=int, default=4)
    t.add_argument("--w", type=int, default=3)
    t.add_argument("--bug", action="store_true", help="side A skips its first exchange (a satisfiable miter)")
    t.add_argument("--out")
    t.set_defaults(func=cmd_toy)
    p = sub.add_parser("prove")
    p.add_argument("cnf")
    p.add_argument("--out")
    p.set_defaults(func=cmd_prove)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
