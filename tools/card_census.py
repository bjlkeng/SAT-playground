#!/usr/bin/env python3
"""Cardinality encoding census (bead SAT-playground-skz.5).

Which encoded at-most-k / at-least-k / exactly-k constraints with k >= 2
a CNF carries, found from the clause shapes alone.  The structure census
(tools/rl_structure.py) sees pairwise at-most-one groups only (k = 1).
Detectors:

  seq    the Sinz sequential counter (LT_SEQ): registers s_{i,j} for
         inputs x_1..x_n and bound k, clauses (-x_i v s_{i,1}),
         (-s_{i-1,j} v s_{i,j}), (-x_i v -s_{i-1,j-1} v s_{i,j}) and the
         overflow (-x_i v -s_{i-1,k}); reported as n, k and the inputs.
  tot    the totalizer (Bailleux-Boufkhad): a tree of unary merges, a
         node with children a_1..a_p and b_1..b_q and outputs r_1..r_{p+q}
         has (-a_i v -b_j v r_{i+j}) with a_0 = b_0 = true (the binaries
         (-a_i v r_i), (-b_j v r_j)); outputs built only up to k+1 (PySAT,
         RC2) and direct leaves (every m-subset of t inputs implies o_m, as
         the ITC2021 cells write them with t = 3) are read too.
  net    comparator networks (sorting and cardinality networks): 2-input
         merge nodes (max <- a v b, min <- a & b) wired into each other,
         plus half comparators; an output's rank (it is true exactly when
         at least t inputs are) comes from simulation, 64 random input
         sets for every count up to 128 inputs (above that a sweep, and a
         rank whose step falls between two sampled counts is unresolved).
  dd     decision diagrams: node variables v with a decision variable x
         and two children, (-v v -x v hi), (-v v x v lo), split into
         diagrams by their node connections (two diagrams that share input
         variables are two); a counter diagram when there is one decision
         variable per level, every edge goes one level down, hi edges add
         one to a consistent count, every node of a level has its own
         count, levels hold two or more nodes on average and some node
         counts two (the rook and the et-tc cells); any other ITE pattern
         (mux networks, ITE chains through gates) is reported as `ite`.
         A counter's bound is read by a walk from its asserted root that
         branches on each level's variable and follows the literals the
         clauses force (boundary nodes written as plain binaries
         included): a branch is rejected only by a clause it makes empty
         and accepts every completion when nothing constrains it, so the
         lowest and highest accepted totals are sound bounds (possibly
         loose).  `dd_max_count` is the largest count a node stores, a
         state statistic, not a bound.
  pop    popcounts: adder networks (tools/netlist_census.py's recovery)
         whose leaves are mostly primary inputs, with what their top bits
         feed (a unit: compared with a constant; an XOR with another
         network's bit: compared with another count).
  binom  direct (binomial) encodings: every m-subset of a set of t
         literals as a clause of one sign, maybe with one more literal in
         each (a guard); at most m-1 of the negated clause literals, only
         m >= 3 and t > m counted.

seq and tot come from one procedure: a merge node is seeded from a triple
(a_1, b_1, r_2) whose two inputs share an implied r_1, the children are
ordered by walking the binaries (a_{i+1} -> r_{i+1}) and the grid, and
the node is accepted only when the whole staircase (every (i, j) with
i + j <= c) and every binary is present.  Nodes are linked into trees by
their output vectors; a chain in which every node adds one input is a
sequential counter, any other tree a totalizer, a tree over two inputs a
pair (not counted), and a component with a cycle or a node read by two
parents is partial (not counted), as is a bounded tree with a node below
the root that stops counting short of its inputs and of the bound without
overflow clauses of its own (it would hide counts from the root).

Bounds: a tree's bound comes from its own forward clauses (inputs imply
registers) only: a unit -r_m (at most m-1) or the overflow clauses at the
root.  A positive unit r_m says nothing alone (the register can be true
with every input false); a lower bound is reported only as the upper bound
of a complete dual tree over the negated inputs whose output is -r_m,
which is r_m's whole reverse implication, and the tree it mirrors then
counts for coverage only.  A Sinz counter written both ways on the same
registers has no such dual, so its positive unit is not read (reported
free).  A network's positive unit is read only when every gate of the
output's cone also has its reverse clauses; a network and its dual reading
one unit count once.  `(neg)` marks a bound on negated literals: at most k
of n negations is at least n-k of the variables.

Usage:
  python3 tools/card_census.py selftest
  python3 tools/card_census.py census --out TABLE.tsv [--cells FILE] [--stems a,b] [--workers 4]
  python3 tools/card_census.py report TABLE.tsv
  python3 tools/card_census.py one FILE.cnf.xz [--uncovered]   # one cell, every detail

Files are read with the 3M-clause cap of the other census tools
(`truncated` says so).  The 2026 suite is never analysed (holdout).
Columns: per detector the count, the input widths (n x how many) and the
bounds; `*_free` trees have no bound (other clauses read their outputs);
`*_cov` is the share of the clauses each detector explains, `cov_total`
their union.

Self test (toys written by hand, no PySAT in the venv): sequential
counters (n=7 k=3 with and without the first-column units, n=6 k=2 with a
register column for the last input, n=12 k=5); totalizers over 6, 9 and 16
inputs (full and bounded at k+1, 2-ary and with 3-input direct leaves,
both directions for an exactly-k); a popcount of 7 inputs through full
adders compared with a constant; a counter decision diagram for at most 2
of 6; Batcher networks on 8 wires at most 2, 3, 5; binomial families (at
most 2 of 5, guarded at least 2 of 4, guarded at most 3 of 6, at least 5
of 7).  Each is matched with the right n and k and every clause covered;
a broken copy (a clause dropped, a register of another column swapped in,
k off by one in the overflow row, a unit moved, an adder input rewired, a
DD edge moved, a comparator clause dropped or two comparators' outputs
crossed, one subset missing) is not, and a pairwise at-most-one is not
counted.  The review findings (2026-10-19) are toys too: merge nodes in a
cycle (the walk used to loop forever), a positive unit on a forward-only
totalizer or network (no lower bound; with the reverse clauses it is one,
read once), a 64-input Batcher network capped at 41 (ranks above 40 used
to read as n), a two-way totalizer with the asserted register's three
reverse clauses removed (no lower bound: a partial reverse encoding), and
at most 3 of 6 written as all 4-subsets (one family, not seven: each of
its clauses also reads as a guarded clause of a smaller family), a
counter diagram whose rejecting edge at count 2 goes to the true terminal
(no at-most-2 read), two counter diagrams over inputs sharing three
variables (two counters, each at most 2), and a capped Sinz counter n=7
k=3 with a unit on a root register (at most 2 of the six prefix inputs:
the last input is free under that unit), and two four-input children
exposing two registers each without overflow clauses under a root with
-r_4 (no at-most-3: the children hide counts past two).
"""
from __future__ import annotations

import argparse
import csv
import itertools
import os
import subprocess
import sys
import tempfile
import time
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rl_structure import normalize, parse_prefix  # noqa: E402
import netlist_census as nc  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
CELLS = ROOT / "benchmarks" / "rl" / "structure_2025.tsv"
MAX_CLAUSES = 3_000_000
MAX_DEG = 20000          # a literal implied by more literals than this is a hub, not a register


# ---------------------------------------------------------------------------
# indexes over the clause list
# ---------------------------------------------------------------------------


class Index:
    """Binary clauses as implications between literals, ternary clauses as
    oriented triples (-u v -v v r) for every choice of the output r, and
    longer clauses with one positive literal by that literal."""

    def __init__(self, clauses):
        self.clauses = clauses
        self.imp = defaultdict(set)      # l -> {m}: clause (-l v m)
        self.pre = defaultdict(set)      # m -> {l}: clause (-l v m)
        self.bin_id = {}                 # (l, m) -> clause id for l -> m
        self.tri = defaultdict(set)      # (u, v) sorted -> {r}: clause (-u v -v v r)
        self.tri_id = {}                 # (u, v, r) -> clause id
        self.partners = defaultdict(set) # u -> {v}
        self.tri_out = defaultdict(list) # r -> [(u, v)]
        self.units = set()
        self.unit_id = {}
        self.big = defaultdict(list)     # (m, frozenset of m negated literals) -> [(r, clause id)] for clauses of length m+1 in 4..8
        self.by_len = Counter()
        for i, c in enumerate(clauses):
            n = len(c)
            self.by_len[n] += 1
            if n == 1:
                self.units.add(c[0])
                self.unit_id[c[0]] = i
            elif n == 2:
                l1, l2 = c
                self.imp[-l1].add(l2)
                self.imp[-l2].add(l1)
                self.pre[l2].add(-l1)
                self.pre[l1].add(-l2)
                self.bin_id[(-l1, l2)] = i
                self.bin_id[(-l2, l1)] = i
            elif n == 3:
                for t in range(3):
                    r = c[t]
                    u, v = (-c[(t + 1) % 3], -c[(t + 2) % 3])
                    if u > v:
                        u, v = v, u
                    self.tri[(u, v)].add(r)
                    self.tri_id[(u, v, r)] = i
                    self.partners[u].add(v)
                    self.partners[v].add(u)
                    self.tri_out[r].append((u, v))
            elif n <= 8:
                for t in range(n):
                    r = c[t]
                    others = frozenset(-c[s] for s in range(n) if s != t)
                    self.big[(n - 1, others)].append((r, i))

    def out(self, u, v):
        if u > v:
            u, v = v, u
        return self.tri.get((u, v), ())

    def tri_cid(self, u, v, r):
        if u > v:
            u, v = v, u
        return self.tri_id.get((u, v, r))


# ---------------------------------------------------------------------------
# unary merge nodes (sequential counter columns and totalizer nodes)
# ---------------------------------------------------------------------------


def _chain(ix, a1, b1, r1, r2, bset):
    """Order one child: a_1 = a1, then a_{i+1} is a literal implying
    r_{i+1} that has a triple with b_1 (whose output is r_{i+2}).  Returns
    the child list and the outputs r_1.. found along it."""
    A = [a1]
    R = [r1, r2]
    seen = {a1, b1, r1, r2}
    while True:
        r_next = R[-1]
        pre = ix.pre.get(r_next, ())
        if len(pre) > MAX_DEG:
            break
        cands = []
        capped = []
        for u in pre:
            if u in seen or -u in seen or u == b1:
                continue
            if ix.out(u, b1):
                cands.append(u)
            elif (u, -b1) in ix.bin_id:
                capped.append(u)   # the overflow (-a_i v -b_1): a_i is the last child, i + 1 = c + 1
        if not cands:
            if len(capped) == 1:
                A.append(capped[0])
            break
        if len(cands) > 1:
            # a shared leaf can drag in partners of another constraint: prefer
            # the candidate that pairs with the most of this node's other child
            cands.sort(key=lambda u: (-len(ix.partners[u] & bset), u))
        u = cands[0]
        outs = sorted(ix.out(u, b1))
        nxt = [r for r in outs if r not in seen and -r not in seen]
        if not nxt:
            break
        A.append(u)
        R.append(nxt[0])
        seen.add(u)
        seen.add(nxt[0])
    return A, R


def _node_from_seed(ix, a1, b1, r1, r2):
    """A unary merge node from (a_1, b_1, r_1, r_2), or None if the grid is
    not complete.  A node is a dict with children A, B, outputs R (index
    1..c), `capped` (the overflow (-a_i v -b_j) for i + j = c + 1 present
    at every such pair), `tail` (the cap-level inputs that have only the
    binary to r_c: a_c and b_c of a bounded node, not told apart), and the
    clause ids it covers."""
    A, RA = _chain(ix, a1, b1, r1, r2, ix.partners[a1])
    B, RB = _chain(ix, b1, a1, r1, r2, ix.partners[b1])
    # the two walks must agree on the outputs they both reach
    m = min(len(RA), len(RB))
    if RA[:m] != RB[:m]:
        return None
    R = RA if len(RA) >= len(RB) else RB
    R = list(R)
    p, q = len(A), len(B)
    lits = set(A) | set(B)
    if len(lits) != p + q or any(-l in lits for l in lits):
        return None
    # the grid: every (i, j) with i + j <= c, c being the largest sum present;
    # the outputs beyond the walks are defined by the first pair that reaches them
    pos = {r: m + 1 for m, r in enumerate(R)}
    cids = set()
    cmax = len(R)
    missing = set()
    for i in range(1, p + 1):
        for j in range(1, q + 1):
            s = i + j
            outs = ix.out(A[i - 1], B[j - 1])
            if s <= len(R):
                want = R[s - 1]
                if want in outs:
                    cids.add(ix.tri_cid(A[i - 1], B[j - 1], want))
                else:
                    missing.add(s)
            else:
                cand = [r for r in outs if r not in pos and -r not in pos and r not in lits and -r not in lits]
                if len(R) == s - 1 and len(cand) == 1 and len(set(R) | set(cand)) == len(R) + 1:
                    R.append(cand[0])
                    pos[cand[0]] = s
                    cids.add(ix.tri_cid(A[i - 1], B[j - 1], cand[0]))
                elif len(R) >= s:
                    pass
                else:
                    missing.add(s)
    # the grid must be a staircase: a missing sum s means c < s, so every
    # sum >= s must be missing too and nothing above s may exist
    c = len(R)
    if missing:
        c = min(missing) - 1
        if len(R) > c:
            return None
    if c < 2:
        return None
    # re-verify completeness up to c (a late-defined output may have been skipped by an earlier pair)
    cids = set()
    for i in range(1, p + 1):
        for j in range(1, q + 1):
            s = i + j
            if s > c:
                continue
            cid = ix.tri_cid(A[i - 1], B[j - 1], R[s - 1])
            if cid is None:
                return None
            cids.add(cid)
    # binaries a_i -> r_i, b_j -> r_j for i, j <= c
    for X in (A, B):
        for i, x in enumerate(X, start=1):
            if i > c:
                return None  # a child wider than the node's outputs: not this node
            cid = ix.bin_id.get((x, R[i - 1]))
            if cid is None:
                return None
            cids.add(cid)
    # the cap: (-a_i v -b_j) for i + j = c + 1, all present or none
    cap_pairs = [(i, j) for i in range(1, p + 1) for j in range(1, q + 1) if i + j == c + 1]
    capped = False
    if cap_pairs:
        have = [ix.bin_id.get((A[i - 1], -B[j - 1])) for i, j in cap_pairs]
        if all(h is not None for h in have):
            capped = True
            cids.update(have)
        elif any(h is not None for h in have):
            return None
    # cap-level inputs: literals implying r_c (c <= p + q) that are neither
    # children nor outputs and have no triple with the first inputs
    tail = set()
    if c >= 2 and c > min(p, q) and len(ix.pre.get(R[c - 1], ())) <= MAX_DEG:
        for u in ix.pre.get(R[c - 1], ()):
            if u in lits or -u in lits or u in pos or -u in pos:
                continue
            if ix.out(u, a1) or ix.out(u, b1):
                continue
            tail.add(u)
        if len(tail) > 2 - (p >= c) - (q >= c):
            tail = set()   # more cap-level literals than children that could reach index c
        # the tail binaries are claimed when a child node confirms the literal (build_trees)
    return {"kind": "merge", "A": A, "B": B, "R": R, "c": c, "capped": capped, "tail": tail, "cids": cids}


def _direct_nodes(ix, claimed):
    """Direct unary nodes over t >= 3 inputs: o_1 implied by every input,
    o_2 by every pair (the ternaries with output o_2 form a clique over the
    inputs), o_m by every m-subset for m <= c, and for m = c + 1 either
    nothing (truncated) or the plain clauses (capped)."""
    nodes = []
    for o2, pairs in ix.tri_out.items():
        pairs = list({tuple(sorted(p)) for p in pairs})   # a repeated clause counts once
        if len(pairs) < 3:
            continue
        X = set()
        for u, v in pairs:
            X.add(u)
            X.add(v)
        t = len(X)
        if t < 3 or len(pairs) != t * (t - 1) // 2 or any(-x in X for x in X):
            continue
        if len({tuple(sorted(p)) for p in pairs}) != len(pairs):
            continue
        Xs = sorted(X)
        cids = set()
        ok = True
        for u, v in itertools.combinations(Xs, 2):
            cid = ix.tri_cid(u, v, o2)
            if cid is None:
                ok = False
                break
            cids.add(cid)
        if not ok:
            continue
        common = None
        for x in Xs:
            s = ix.imp.get(x, set())
            common = set(s) if common is None else common & s
            if not common:
                break
        if not common:
            continue
        o1s = [o for o in common if o != o2 and o not in X and -o not in X]
        if not o1s:
            continue
        o1 = min(o1s)
        for x in Xs:
            cids.add(ix.bin_id[(x, o1)])
        outs = [o1, o2]
        c = 2
        for m in range(3, min(t, 7) + 1):
            # o_m: a literal r with a clause (-S v r) for every m-subset S;
            # the candidates come from one subset
            found = None
            for r in _big_outputs(ix, m, frozenset(Xs[:m])):
                if r in outs or r in X or -r in X:
                    continue
                allok = True
                ids = []
                for S in itertools.combinations(Xs, m):
                    cid = _big_cid(ix, m, r, frozenset(S))
                    if cid is None:
                        allok = False
                        break
                    ids.append(cid)
                if allok:
                    found = (r, ids)
                    break
            if found is None:
                break
            outs.append(found[0])
            cids.update(found[1])
            c = m
        capped = False
        if c < t:
            m = c + 1
            ids = []
            allok = True
            for S in itertools.combinations(Xs, m):
                cid = _plain_cid(ix, S)
                if cid is None:
                    allok = False
                    break
                ids.append(cid)
            if allok:
                capped = True
                cids.update(ids)
        if cids & claimed:
            continue
        nodes.append({"kind": "direct", "X": Xs, "R": outs, "c": c, "capped": capped, "tail": set(), "cids": cids})
    return nodes


def _big_outputs(ix, m, S):
    """Literals r with a clause (-S v r), S a frozenset of m literals."""
    return [r for r, cid in ix.big.get((m, S), ())]


def _big_cid(ix, m, r, S):
    for rr, cid in ix.big.get((m, S), ()):
        if rr == r:
            return cid
    return None


def _plain_cid(ix, S):
    """Clause id of the plain clause (-s_1 v ... v -s_m), S a tuple of literals."""
    m = len(S)
    if m == 2:
        return ix.bin_id.get((S[0], -S[1]))
    if m == 3:
        # (-u v -v v -w): oriented as (u, v) -> -w
        return ix.tri_cid(S[0], S[1], -S[2])
    if m > 8:
        return None
    return _big_cid(ix, m - 1, -S[-1], frozenset(S[:-1]))


def find_unary_nodes(ix):
    """All unary merge and direct nodes, non-overlapping, biggest first."""
    claimed = set()
    nodes = []
    direct = _direct_nodes(ix, claimed)
    direct.sort(key=lambda d: -len(d["cids"]))
    for d in direct:
        if d["cids"] & claimed:
            continue
        claimed.update(d["cids"])
        nodes.append(d)
    cands = []
    seen_seed = set()
    for (u, v), outs in ix.tri.items():
        iu = ix.imp.get(u)
        iv = ix.imp.get(v)
        if not iu or not iv or len(iu) > MAX_DEG or len(iv) > MAX_DEG:
            continue
        common = iu & iv if len(iu) < len(iv) else iv & iu
        if not common:
            continue
        for r2 in outs:
            for r1 in common:
                if r1 == r2 or r1 in (u, v, -u, -v) or -r1 == r2:
                    continue
                key = (u, v, r1, r2)
                if key in seen_seed:
                    continue
                seen_seed.add(key)
                nd = _node_from_seed(ix, u, v, r1, r2)
                if nd is not None:
                    cands.append(nd)
    # biggest first; between the two readings of one clause set (registers
    # implied by inputs, or the contrapositive over negated registers)
    # prefer positive outputs, which is how every encoder writes registers
    cands.sort(key=lambda d: (-len(d["cids"]), sum(1 for r in d["R"] if r < 0), d["R"][0]))
    for d in cands:
        if d["cids"] & claimed:
            continue
        claimed.update(d["cids"])
        nodes.append(d)
    return nodes


def find_networks(ix, nodes, parent, claimed):
    """Comparator networks (sorting and cardinality networks): 2-leaf merge
    nodes (max = a v b, min = a & b) whose wires feed other comparators one
    at a time, plus half comparators (an AND or an OR over two wires of the
    network whose other output was pruned).  A network's output wires are
    ranked by simulation: wire o has threshold t when it is true exactly
    when at least t inputs are (64 random subsets per size tried)."""
    import random
    comps = [ni for ni, nd in enumerate(nodes)
             if nd["kind"] == "merge" and len(nd["A"]) == 1 and len(nd["B"]) == 1 and nd["c"] == 2 and ni not in parent]
    if not comps:
        return []
    claimed = set(claimed)
    gates = []          # (kind, a, b, outs tuple, cids, node id)
    produced = {}       # wire -> gate index (one producer per wire)
    W = set()
    for ni in comps:
        nd = nodes[ni]
        a, b = nd["A"][0], nd["B"][0]
        o1, o2 = nd["R"][0], nd["R"][1]
        if o1 in produced or o2 in produced:
            continue   # two comparators on one output wire: keep the first
        gates.append(("cmp", a, b, (o1, o2), nd["cids"], ni))
        produced[o1] = produced[o2] = len(gates) - 1
        W.update((a, b, o1, o2))
    # half comparators are grown over produced wires only (a raw input is an
    # arbitrary variable with gates of its own all over the formula)
    frontier = set(produced)
    for _round in range(12):
        new = set()
        if len(gates) > 300_000:
            break
        for w in frontier:
            # AND: (-w v -v v c), v a produced wire, c not yet produced
            for v in ix.partners.get(w, ()):
                if v not in produced or v == w or v == -w:
                    continue
                for c in ix.out(w, v):
                    # a wire another gate already reads may still get its
                    # producer here (a pruned network); acyclicity is checked
                    # later and any bound is proven before it counts
                    if c in produced or -c in produced or -c in W:
                        continue
                    cid = ix.tri_cid(w, v, c)
                    if cid in claimed:
                        continue
                    gates.append(("and", w, v, (c,), {cid}, None))
                    produced[c] = len(gates) - 1
                    new.add(c)
                    claimed.add(cid)
            # OR: c with pre[c] = {w, v}, both produced wires
            for c in ix.imp.get(w, ()):
                if c in produced or -c in produced or -c in W or c in new:
                    continue
                pre = ix.pre.get(c, ())
                if len(pre) != 2:
                    continue
                v = [u for u in pre if u != w][0]
                if v not in produced or v == -w:
                    continue
                c1, c2 = ix.bin_id[(w, c)], ix.bin_id[(v, c)]
                if c1 in claimed or c2 in claimed:
                    continue
                gates.append(("or", w, v, (c,), {c1, c2}, None))
                produced[c] = len(gates) - 1
                new.add(c)
                claimed.update((c1, c2))
        if not new:
            break
        W |= new
        frontier = new
    # components over gates through wires
    uf = {}

    def find(x):
        while uf.get(x, x) != x:
            uf[x] = uf.get(uf[x], uf[x])
            x = uf[x]
        return x

    def union(a, b):
        ra, rb = find(a), find(b)
        if ra != rb:
            uf[ra] = rb

    # a network is connected through produced wires only: two constraints
    # that share an input variable are two networks
    for gi, g in enumerate(gates):
        for w in (g[1], g[2]):
            if w in produced:
                union(("g", produced[w]), ("g", gi))
    comp_gates = defaultdict(list)
    for gi in range(len(gates)):
        comp_gates[find(("g", gi))].append(gi)
    nets = []
    rnd = random.Random(7)
    full64 = (1 << 64) - 1
    for key, gis in comp_gates.items():
        if len(gis) < 2:
            continue
        consumed = set()
        outs = set()
        for gi in gis:
            g = gates[gi]
            consumed.update((g[1], g[2]))
            outs.update(g[3])
        inputs = sorted(consumed - outs, key=abs)
        outputs = sorted(outs - consumed, key=abs)
        # topological order of the gates
        indeg = {gi: 0 for gi in gis}
        users = defaultdict(list)
        gis_set = set(gis)
        for gi in gis:
            g = gates[gi]
            for w in (g[1], g[2]):
                if w in produced and produced[w] in gis_set:
                    indeg[gi] += 1
                    users[produced[w]].append(gi)
        order = [gi for gi in gis if indeg[gi] == 0]
        i = 0
        while i < len(order):
            gi = order[i]
            i += 1
            for u in users[gi]:
                indeg[u] -= 1
                if indeg[u] == 0:
                    order.append(u)
        cyclic = len(order) < len(gis)
        groups = []
        cids = set()
        for gi in gis:
            cids |= gates[gi][4]
        if not cyclic:
            # the input cone of every wire, as a bitset over the inputs; the
            # watched wires (outputs, and wires under a unit) grouped by cone:
            # a shared sorted sub-sequence is one group
            bit = {w: 1 << i for i, w in enumerate(inputs)}
            cone = dict(bit)
            gate_of = {}
            for gi in order:
                kind, a, b, os_, _c, ni = gates[gi]
                cb = cone[a] | cone[b]
                for o in os_:
                    cone[o] = cb
                    gate_of[o] = gi
            watched = set(outputs) | {w for w in outs if w in ix.units or -w in ix.units}
            by_cone = defaultdict(list)
            for w in watched:
                by_cone[cone[w]].append(w)
            for cb, ws in sorted(by_cone.items(), key=lambda kv: -len(kv[1]))[:400]:
                g_inputs = [w for w in inputs if bit[w] & cb]
                n = len(g_inputs)
                if n < 2:
                    continue
                # the gates of this cone, in order
                need = set()
                stack = list(ws)
                while stack:
                    w = stack.pop()
                    gi = gate_of.get(w)
                    if gi is None or gi in need:
                        continue
                    need.add(gi)
                    stack.extend((gates[gi][1], gates[gi][2]))
                g_order = [gi for gi in order if gi in need]
                if n <= 128:
                    ms = list(range(0, n + 1))
                else:
                    ms = sorted(set(range(0, 65)) | set(range(64, n + 1, max(1, (n - 64) // 32))) | {n})
                ms_set = set(ms)
                states = defaultdict(dict)
                for m in ms:
                    val = {w: 0 for w in g_inputs}
                    for t in range(64):
                        for w in rnd.sample(g_inputs, m):
                            val[w] |= 1 << t
                    for gi in g_order:
                        kind, a, b, os_, _c, ni = gates[gi]
                        va, vb = val[a], val[b]
                        if kind == "cmp":
                            val[os_[0]] = va | vb
                            val[os_[1]] = va & vb
                        elif kind == "and":
                            val[os_[0]] = va & vb
                        else:
                            val[os_[0]] = va | vb
                    for w in ws:
                        v = val[w]
                        states[w][m] = "T" if v == full64 else ("F" if v == 0 else "X")
                thresholds = {}
                for w in ws:
                    st = states[w]
                    if any(s == "X" for s in st.values()):
                        continue
                    ts = [m for m in ms if st[m] == "T"]
                    fs = [m for m in ms if st[m] == "F"]
                    if ts and (not fs or max(fs) < min(ts)):
                        t0 = min(ts)
                        # exact only when the count just below was sampled too
                        # (else the step lies somewhere in a gap: unresolved)
                        if t0 >= 1 and (t0 - 1) in ms_set:
                            thresholds[w] = t0
                bound = ("none", 0)
                bound_cid = None
                g_cids = set()
                rev = _reverse_cids(ix, gates, need)   # None unless every gate also has its reverse clauses
                # every asserted output gives a bound: keep the tightest upper
                # one (the smallest threshold whose output is false), else the
                # tightest lower one (the largest threshold whose output is
                # true, two-way gates only); redundant looser units must not
                # hide it
                ups = [w for w in ws if w in thresholds and -w in ix.units]
                los = [w for w in ws if w in thresholds and w in ix.units] if rev is not None else []
                if ups:
                    w = min(ups, key=lambda w: (thresholds[w], abs(w)))
                    bound, bound_cid = ("atmost", thresholds[w] - 1), ix.unit_id[-w]
                elif los:
                    w = max(los, key=lambda w: (thresholds[w], -abs(w)))
                    bound, bound_cid = ("atleast", thresholds[w]), ix.unit_id[w]
                if bound_cid is not None:
                    g_cids.add(bound_cid)
                if rev:
                    g_cids |= rev
                cids |= g_cids
                for gi in need:
                    g_cids |= gates[gi][4]
                ranked = sorted([w for w in ws if w in thresholds], key=lambda w: (thresholds[w], abs(w)))
                groups.append({
                    "inputs": g_inputs, "n": n, "wires": sorted(ws, key=abs), "ranked": ranked, "thresholds": thresholds,
                    "clean": all(w in thresholds for w in ws), "bound": bound, "bound_cid": bound_cid,
                    "gates": len(need), "cids": g_cids,
                })
        groups.sort(key=lambda d: (-d["n"], -d["gates"]))
        nets.append({
            "gates": len(gis), "full": sum(1 for gi in gis if gates[gi][0] == "cmp"),
            "half": sum(1 for gi in gis if gates[gi][0] != "cmp"), "node_ids": [gates[gi][5] for gi in gis if gates[gi][5] is not None],
            "inputs": inputs, "n": len(inputs), "outputs": outputs, "groups": groups, "cyclic": cyclic, "cids": cids,
        })
    nets = [d for d in nets if not d["cyclic"]]   # a cyclic component is not a comparator network
    nets.sort(key=lambda d: -d["gates"])
    return nets


def _reverse_cids(ix, gates, gis):
    """The clauses that make every gate two-way (max -> a v b, min -> a,
    min -> b for a comparator; c -> a, c -> b for an AND; c -> a v b for an
    OR), or None when one is missing."""
    out = set()
    for gi in gis:
        kind, a, b, os_, _c, ni = gates[gi]
        if kind == "cmp":
            hi, lo = os_
            need = [ix.tri_cid(hi, -a, b), ix.bin_id.get((lo, a)), ix.bin_id.get((lo, b))]
        elif kind == "and":
            c = os_[0]
            need = [ix.bin_id.get((c, a)), ix.bin_id.get((c, b))]
        else:
            need = [ix.tri_cid(os_[0], -a, b)]
        if any(x is None for x in need):
            return None
        out.update(need)
    return out


def build_trees(ix, nodes):
    """Link nodes by their output vectors into trees; classify each tree.
    Returns (trees, networks not consumed by a tree)."""
    out_of = {}
    for ni, nd in enumerate(nodes):
        for r in nd["R"]:
            out_of[r] = ni
    children = {}
    parent = {}
    extra_cids = defaultdict(set)   # node -> clause ids of first-column children (the binary x_1 -> s_{1,1} and the units)
    occ_count = Counter()
    for c in ix.clauses:
        for l in c:
            occ_count[abs(l)] += 1

    def first_column(vec):
        """A Sinz first column [s_{1,1}, s_{1,2}, ..]: s_{1,1} implied by one
        leaf x_1 only, the others false by a unit (or unused otherwise).
        Returns (x_1, clause ids) or None."""
        pre = ix.pre.get(vec[0], set())
        if len(pre) != 1:
            return None
        x1 = next(iter(pre))   # any signal can be an input, even one another node defines
        ids = {ix.bin_id[(x1, vec[0])]}
        for v in vec[1:]:
            if -v in ix.units:
                ids.add(ix.unit_id[-v])
            elif occ_count[abs(v)] > 3:
                return None
        return x1, ids

    def match_child(ni_parent, vec, tail):
        """The node whose outputs are vec (a child of width len(vec)), a
        leaf signal (any width-1 child), or unresolved when vec is a unary
        vector no node defines."""
        ni = out_of.get(vec[0])
        if len(vec) == 1:
            # a width-1 child is a leaf, unless it is a node's first output
            # whose second output is one of this node's tails (a child cut
            # short by a bounded node: its count goes on through the tail)
            if ni is not None and tail:
                R = nodes[ni]["R"]
                P = nodes[ni_parent]
                # the omitted register R[1] counts 2 and may stand in for the
                # parent's top register only when that one counts 2 as well
                if len(R) == 2 and R[0] == vec[0] and R[1] in tail and P["c"] == 2:
                    cid = ix.bin_id.get((R[1], P["R"][P["c"] - 1]))
                    if cid is not None:
                        extra_cids[ni_parent].add(cid)
                        return ("node", ni)
            return ("leaf", vec[0])
        if ni is None:
            fc = first_column(vec)
            if fc is not None:
                extra_cids[ni_parent] |= fc[1]
                return ("leaf", fc[0])
            return ("unresolved", tuple(vec))
        R = nodes[ni]["R"]
        n = min(len(R), len(vec))
        if R[:n] != vec[:n]:
            return ("unresolved", tuple(vec))
        if len(R) == len(vec):
            return ("node", ni)
        P = nodes[ni_parent]
        if len(R) == len(vec) + 1 and R[-1] in tail and len(R) == P["c"]:
            # the child's top output reaches the parent's top output by a binary
            # only (a bounded node); sound only at the same count: R[-1] means
            # "this child holds len(R)", the parent's top "the node holds c"
            cid = ix.bin_id.get((R[-1], P["R"][P["c"] - 1]))
            if cid is not None:
                extra_cids[ni_parent].add(cid)
                return ("node", ni)
        return ("unresolved", tuple(vec))

    for ni, nd in enumerate(nodes):
        if nd["kind"] == "merge":
            kids = [match_child(ni, nd["A"], nd["tail"]), match_child(ni, nd["B"], nd["tail"])]
            if nd.get("capped"):
                # `capped` was checked on the children as the staircase showed
                # them; a child whose last register came back through a tail
                # counts one further, so every pair at c + 1 is checked again
                # on the restored vectors (a missing overflow clause there
                # lets the node hold more than c)
                full = []
                for (kind, val), vec in zip(kids, (nd["A"], nd["B"])):
                    if kind == "node" and len(nodes[val]["R"]) > len(vec):
                        full.append(list(nodes[val]["R"]))
                    else:
                        full.append(list(vec))
                if full[0] != list(nd["A"]) or full[1] != list(nd["B"]):
                    c = nd["c"]
                    A2, B2 = full
                    still = len(A2) <= c and len(B2) <= c
                    for i in range(1, len(A2) + 1):
                        j = c + 1 - i
                        if still and 1 <= j <= len(B2):
                            a, b = A2[i - 1], B2[j - 1]
                            if (a, -b) not in ix.bin_id and (b, -a) not in ix.bin_id:
                                still = False
                    if not still:
                        nd["capped"] = False
        else:
            kids = [("leaf", x) for x in nd["X"]]
        children[ni] = kids
        for kind, val in kids:
            if kind == "node":
                parent[val] = ni
    # comparator networks from the 2-leaf nodes no tree consumes; their
    # ranked outputs are unary vectors a merge node may consume (ITC2021)
    claimed = set()
    for nd in nodes:
        claimed |= nd["cids"]
    nets = find_networks(ix, nodes, parent, claimed)
    in_net = {}
    net_first = {}
    for ki, net in enumerate(nets):
        for ni in net["node_ids"]:
            in_net[ni] = ki
        for gi, grp in enumerate(net["groups"]):
            if grp["ranked"]:
                net_first.setdefault(grp["ranked"][0], (ki, gi))
    net_used = set()
    for ni, kids in children.items():
        for idx, (kind, val) in enumerate(kids):
            if kind != "unresolved":
                continue
            hit = net_first.get(val[0])
            if hit is None:
                continue
            ki, gi = hit
            grp = nets[ki]["groups"][gi]
            if list(val) == grp["ranked"][: len(val)] and all(grp["thresholds"][w] == i + 1 for i, w in enumerate(val)):
                kids[idx] = ("net", (ki, gi, len(val)))
                net_used.add(ki)
    n_parents = Counter()
    for kids in children.values():
        for kind, val in kids:
            if kind == "node":
                n_parents[val] += 1
    trees = []
    for ni, nd in enumerate(nodes):
        if ni in parent or ni in in_net:
            continue
        # walk the tree; a node reached twice (a cycle through recovered
        # nodes, which a valid CNF can hold) or read by two parents makes
        # the component no tree: it is kept as partial and never counted
        leaves = []
        unresolved = 0
        members = []
        nets_in = []
        stack = [ni]
        seen = set()
        cyclic = False
        chain = True
        while stack:
            x = stack.pop()
            if x in seen:
                cyclic = True
                continue
            seen.add(x)
            members.append(x)
            if n_parents[x] > 1:
                cyclic = True
            nonleaf = [v for k, v in children[x] if k in ("node", "net")]
            if nodes[x]["kind"] == "direct" and len(nodes[x]["X"]) >= 3:
                chain = False
            if len(nonleaf) >= 2:
                chain = False
            for k, v in children[x]:
                if k == "leaf":
                    leaves.append(v)
                elif k == "node":
                    if v in seen:
                        cyclic = True
                    else:
                        stack.append(v)
                elif k == "net":
                    nets_in.append(v[:2])
                    leaves.extend(nets[v[0]]["groups"][v[1]]["inputs"])
                    chain = False
                else:
                    unresolved += len(v)
        cids = set()
        for x in members:
            cids |= nodes[x]["cids"]
            cids |= extra_cids.get(x, set())
        for ki, gi in nets_in:
            cids |= nets[ki]["groups"][gi]["cids"]
        root = nodes[ni]
        R = root["R"]
        # a tree's bound comes from its own forward implications (inputs
        # imply registers) only: a unit -r_m (at most m-1) or the overflow
        # clauses of the root.  A positive unit r_m says nothing on its own
        # (the register can be true with every input false); a lower bound
        # shows up as the upper bound of the dual tree over the negated
        # inputs, whose output -r_m carries r_m's whole reverse implication
        ub = ub_cid = None
        for m, r in enumerate(R, start=1):
            if -r in ix.units:
                ub, ub_cid = m - 1, ix.unit_id[-r]
                cids.add(ub_cid)
                break
        if ub is None and root["capped"]:
            ub = root["c"]
        bound = ("atmost", ub) if ub is not None else ("none", root["c"])
        # a node below the root that stops counting at c outputs short of its
        # inputs, with no overflow clauses of its own, shows the root at most
        # c of its count; the root's bound holds only if every such node (and
        # every network child read short of its inputs) counts past it
        saturates = False
        if ub is not None and not cyclic:
            n_under = {}
            for x in reversed(members):
                total = 0
                for k_, v_ in children[x]:
                    if k_ == "leaf":
                        total += 1
                    elif k_ == "node":
                        total += n_under.get(v_, 0)
                    elif k_ == "net":
                        g_ = nets[v_[0]]["groups"][v_[1]]
                        total += g_["n"]
                        if v_[2] < g_["n"] and v_[2] <= ub:
                            saturates = True
                    else:
                        total += len(v_)
                n_under[x] = total
                if x != ni and nodes[x]["c"] < total and not nodes[x]["capped"] and nodes[x]["c"] <= ub:
                    saturates = True
        all_capped = all(nodes[x]["capped"] for x in members)
        # a Sinz counter over three inputs has one merge node (x_1, x_2) and
        # its last input only in the overflow clause: a capped single node is
        # a chain too, so the last input is looked for below
        kind = "seq" if chain and ((len(members) >= 2 and len(leaves) >= 3) or (len(members) == 1 and len(leaves) == 2 and root["capped"])) else "tot"
        if kind == "seq" and not all(nodes[x]["kind"] == "merge" and min(len(nodes[x]["A"]), len(nodes[x]["B"])) == 1 for x in members):
            kind = "tot"
        if kind == "seq" and root["capped"] and ub_cid is None:
            # the last input of a Sinz counter has only the overflow clause
            # (-x_n v -s_{n-1,k}): one literal implying -s_{n-1,k} from outside
            # the tree.  It joins the count only when the bound is the cap's
            # (at most k of the prefix, and x_n only below k): a unit -r_m
            # bounds the prefix alone, and x_n is then free (its overflow
            # clause names s_{n-1,k}, which the unit leaves unconstrained)
            top = R[root["c"] - 1]
            inside = set(leaves) | {l for x in members for l in nodes[x]["R"]}
            cands = [u for u in ix.pre.get(-top, ()) if u not in inside and -u not in inside and u not in out_of and -u not in out_of]
            if len(cands) == 1:
                leaves.append(cands[0])
                cids.add(ix.bin_id[(cands[0], -top)])
        n_in = len(leaves) + unresolved
        two_way = 0.0
        if kind == "seq":
            # a Sinz counter caps every column at k (the same width throughout);
            # a counter bounded by a unit caps none; anything in between is broken
            # (only columns that can overflow: in a compact counter the early
            # columns, which cannot reach k + 1 yet, have no overflow clauses)
            can_overflow = [x for x in members if len(nodes[x]["A"]) + len(nodes[x]["B"]) > nodes[x]["c"]]
            caps = {nodes[x]["capped"] for x in can_overflow}
            widths = {nodes[x]["c"] for x in can_overflow}
            if len(caps) > 1 or len(widths) > 1:
                kind = "partial"
            # the reverse direction (s_{i,j} -> s_{i-1,j} v x_i, s_{i,j} -> s_{i-1,j} v s_{i-1,j-1}):
            # registers that mean exactly "at least j", as the timetable cells write them
            want = have = 0
            for x in members:
                nd = nodes[x]
                if len(nd["A"]) == 1 and len(nd["B"]) >= 1:
                    xi, B, R = nd["A"][0], nd["B"], nd["R"]
                elif len(nd["B"]) == 1:
                    xi, B, R = nd["B"][0], nd["A"], nd["R"]
                else:
                    continue
                for j, r in enumerate(R, start=1):
                    if j > len(B):
                        break
                    want += 1
                    cid = ix.tri_cid(r, -B[j - 1], xi)
                    if cid is not None:
                        have += 1
                        cids.add(cid)
                    if j >= 2:
                        want += 1
                        cid = ix.bin_id.get((r, B[j - 2]))
                        if cid is None:
                            cid = ix.tri_cid(r, -B[j - 1], B[j - 2])
                        if cid is not None:
                            have += 1
                            cids.add(cid)
            two_way = round(have / want, 2) if want else 0.0
        if unresolved or cyclic or saturates:
            kind = "partial"   # a child vector no node defines, no tree, or a child that hides counts: not a clean match
        trees.append({
            "kind": kind, "root": ni, "nodes": len(members), "leaves": sorted(leaves, key=abs),
            "n": n_in, "unresolved": unresolved, "cyclic": cyclic, "saturates": saturates, "bound": bound, "width": root["c"],
            "ub": ub, "ub_cid": ub_cid,
            "capped_all": all_capped, "nets": len(nets_in), "two_way": two_way, "cids": cids,
        })
    # the dual of an exactly-k encoding: a tree over the negated leaves whose
    # registers are this tree's registers negated (the reverse implications)
    by_leaves = defaultdict(list)
    for ti, t in enumerate(trees):
        by_leaves[frozenset(t["leaves"])].append(ti)
    for t in trees:
        t["dual"], t["dual_of"] = False, None
        if not t["leaves"]:
            continue
        neg_regs = {-r for x in [t["root"]] for r in nodes[x]["R"]}
        for tj in by_leaves.get(frozenset(-l for l in t["leaves"]), ()):
            if neg_regs & set(nodes[trees[tj]["root"]]["R"]):
                t["dual"], t["dual_of"] = True, tj
                break
    # one constraint seen from both sides: a tree with no bound of its own
    # whose dual is bounded is the other half of that constraint (an
    # exactly-k or at-least-k encoding); its clauses count for coverage,
    # not as another constraint
    for t in trees:
        t["mirror"] = False
        if t["bound"][0] == "none" and t["dual"] and t["kind"] in ("seq", "tot"):
            o = trees[t["dual_of"]]
            if o["bound"][0] == "atmost" and o["kind"] in ("seq", "tot"):
                t["mirror"] = True
    standalone = [net for ki, net in enumerate(nets) if ki not in net_used]
    return trees, standalone


# ---------------------------------------------------------------------------
# decision diagrams
# ---------------------------------------------------------------------------


def find_dd(ix, clauses):
    """Decision-diagram nodes: a literal v with clauses (-v v -x v hi) and
    (-v v x v lo), hi and lo positive literals (or one side the false
    terminal, written as the binary (-v v -x) or (-v v x)).  An encoding
    that also writes the clauses upward (et-tc) gives every node a second
    candidate decision variable, the one of its parents; the orientation
    is settled by a walk from the asserted node: at a node reached over
    decision variable x, the candidate with another variable is its own."""
    by_node = defaultdict(lambda: defaultdict(lambda: {"hi": [], "lo": []}))   # v -> x -> side -> [(child, cid)]
    for i, c in enumerate(clauses):
        if len(c) != 3:
            continue
        negs = [l for l in c if l < 0]
        poss = [l for l in c if l > 0]
        if len(negs) == 2 and len(poss) == 1:
            for v in negs:
                x = [l for l in negs if l != v][0]
                by_node[-v][-x]["hi"].append((poss[0], i))     # (-v v -x v h): x true -> h
        elif len(negs) == 1 and len(poss) == 2:
            v = -negs[0]
            for x in poss:
                child = [l for l in poss if l != x][0]
                by_node[v][x]["lo"].append((child, i))        # (-v v x v l): x false -> l
    cands = {}   # v -> [(x, (hi, cid), (lo, cid))]
    full_node = set()   # nodes with two ternaries (a one-sided node is a child only, never a start)
    for v, byvar in by_node.items():
        lst = []
        for xv, d in byvar.items():
            if xv == v:
                continue
            hi = list({ch: (ch, cid) for ch, cid in d["hi"] if ch > 0 and ch != v}.values())   # a repeated clause counts once
            lo = list({ch: (ch, cid) for ch, cid in d["lo"] if ch > 0 and ch != v}.values())
            if len(hi) == 1 and len(lo) == 1 and hi[0][0] != lo[0][0]:
                lst.append((xv, hi[0], lo[0]))
                full_node.add(v)
            elif len(lo) == 1 and not hi and (v, -xv) in ix.bin_id:
                lst.append((xv, (0, ix.bin_id[(v, -xv)]), lo[0]))   # x true is impossible at v: hi = false
            elif len(hi) == 1 and not lo and (v, xv) in ix.bin_id:
                lst.append((xv, hi[0], (0, ix.bin_id[(v, xv)])))    # x false is impossible at v: lo = false
        if lst:
            cands[v] = lst
    if not full_node:
        return None
    # orientation by a walk from the asserted nodes (else from the nodes
    # with one candidate that no candidate edge reaches)
    child_of = set()
    for v, lst in cands.items():
        for xv, (h, hc), (l, lc) in lst:
            if h:
                child_of.add(h)
            if l:
                child_of.add(l)
    asserted = [v for v in full_node if v in ix.units]
    free_roots = [v for v in full_node if v not in child_of and len(cands[v]) == 1]
    # the asserted diagrams first (their roots fix the orientation), then
    # the roots of diagrams no unit asserts, which the first walk did not
    # reach: an asserted diagram must not hide an independent free one
    phases = [asserted, free_roots] if asserted else [free_roots]
    nodes = {}
    came_by = {}
    ambiguous = 0
    queue = []
    qi = 0
    for phase in phases:
        seeds = [v for v in phase if v not in nodes and v not in came_by]
        for s0 in seeds:
          came_by[s0] = None
        queue.extend(seeds)
        while qi < len(queue):
            v = queue[qi]
            qi += 1
            if v in nodes:
                continue
            lst = [c for c in cands[v] if c[0] != came_by.get(v)]
            if len(lst) > 1:
                # prefer the candidate whose children are not settled yet (the forward direction)
                fresh = [c for c in lst if all(ch not in nodes for ch in (c[1][0], c[2][0]) if ch)]
                if len(fresh) == 1:
                    lst = fresh
            if len(lst) != 1:
                ambiguous += 1
                continue
            xv, (h, hc), (l, lc) = lst[0]
            cids = {hc, lc}
            extra = ix.tri_cid(v, -h, l) if h and l else None   # the redundant (-v v hi v lo)
            if extra is not None:
                cids.add(extra)
            nodes[v] = {"x": xv, "hi": h, "lo": l, "cids": cids}
            for ch in (h, l):
                if ch and ch in cands and ch not in nodes:
                    came_by.setdefault(ch, xv)
                    queue.append(ch)
    if not nodes:
        return None
    asserted_set = set(asserted)
    child_of = defaultdict(list)
    for v, nd in nodes.items():
        for ch in (nd["hi"], nd["lo"]):
            if ch and ch in nodes:
                child_of[ch].append(v)
    # one diagram per connected group of nodes: diagrams that share input
    # variables (but no node) are checked apart
    comp_of = {}
    comps = []
    for v in nodes:
        if v in comp_of:
            continue
        comp_of[v] = len(comps)
        stack = [v]
        members = []
        while stack:
            u = stack.pop()
            members.append(u)
            for w in [nodes[u]["hi"], nodes[u]["lo"]] + child_of.get(u, []):
                if w and w in nodes and w not in comp_of:
                    comp_of[w] = len(comps)
                    stack.append(w)
        comps.append(members)
    diagrams = [_dd_diagram(ix, nodes, members, child_of, asserted_set) for members in comps]
    diagrams.sort(key=lambda d: -d["nodes"])
    return {"diagrams": diagrams, "candidates": len(cands), "ambiguous": ambiguous}


def _outgoing(ix, v):
    """The binary and ternary clauses with -v, as (clause id, the other literals)."""
    out = []
    seen = set()
    for m in ix.imp.get(v, ()):
        cid = ix.bin_id.get((v, m))
        if cid is not None and cid not in seen:
            seen.add(cid)
            out.append((cid, (m,)))
    for w in ix.partners.get(v, ()):
        for r in ix.out(v, w):
            cid = ix.tri_cid(v, w, r)
            if cid is not None and cid not in seen:
                seen.add(cid)
                out.append((cid, (-w, r)))
    return out


def _dd_bound(ix, root, level_var, L, node_vars, max_states):
    """The totals a diagram accepts, by a walk from its asserted root.  At
    level l the walk branches on the level's decision variable; on each
    branch v's clauses are read: one made empty rejects, one left with a
    single literal forces that literal (the walk follows it to level l+1),
    none accepts every completion.  Clauses with two literals left and
    literals forced beside the one followed are ignored, and ignoring a
    clause only adds accepted totals, so the lowest and highest accepted
    totals stay sound bounds (possibly loose).  Returns (lowest, highest,
    complete, clause ids read, states)."""
    input_vars = set(level_var.values())
    states = {(root, 0, 0)}
    queue = [(root, 0, 0)]
    lo_tot = hi_tot = None
    used = set()
    complete = True

    def accept(a, b):
        nonlocal lo_tot, hi_tot
        lo_tot = a if lo_tot is None else min(lo_tot, a)
        hi_tot = b if hi_tot is None else max(hi_tot, b)

    while queue:
        v, l, c = queue.pop()
        if l >= L:
            accept(c, c)
            continue
        x = level_var[l]
        req = {True: set(), False: set()}
        for cid, D in _outgoing(ix, v):
            for br in (True, False):
                xl = x if br else -x
                if xl in D:
                    continue
                Db = [d for d in D if d != -xl]
                if not Db:
                    req[br].add(0)                       # a clause made empty: this branch rejects
                    used.add(cid)
                elif len(Db) == 1:
                    req[br].add(Db[0])
                    used.add(cid)
        for br in (True, False):
            c2 = c + (1 if br else 0)
            r = req[br]
            if 0 in r:
                continue                                 # rejected
            falsified = [d for d in r if -d in ix.units]
            if falsified:
                used.add(ix.unit_id[-falsified[0]])      # rejected by a forced literal a unit makes false: that unit is part of the bound
                continue
            for d in r:
                if d in ix.units:
                    used.add(ix.unit_id[d])              # a literal a unit makes true is no constraint
            lits = [d for d in r if d not in ix.units]
            if not lits:
                accept(c2, c2 + (L - l - 1))
                continue
            # follow one forced literal: a diagram node on the next level first,
            # then any literal that is not an input variable
            lits.sort(key=lambda d: (d not in node_vars or node_vars.get(d) != level_var.get(l + 1),
                                     abs(d) in input_vars, abs(d)))
            st = (lits[0], l + 1, c2)
            if st not in states:
                states.add(st)
                queue.append(st)
                if len(states) > max_states:
                    return None, None, False, used, len(states)
    return lo_tot, hi_tot, complete, used, len(states)


def _dd_diagram(ix, nodes, members, child_of, asserted_set):
    """One connected diagram: levels, counts, whether it reads as a counter,
    and its accepted totals when its root is asserted."""
    mset = set(members)
    roots = [v for v in members if not any(p in mset for p in child_of.get(v, ()))]
    d = {"nodes": len(members), "roots": len(roots), "asserted": sum(1 for v in roots if v in asserted_set),
         "levels": 0, "max_width": 0, "mean_width": 0.0, "chain_frac": 0.0, "grid": False, "pos_unique": False,
         "layered": False, "max_count": 0, "counter": False, "bound": None, "lo_total": None, "hi_total": None,
         "walk_states": 0, "cids": set()}
    for v in members:
        d["cids"] |= nodes[v]["cids"]
    d["inputs"] = sorted({nodes[v]["x"] for v in members}, key=abs)   # the counted decision literals
    levels = defaultdict(list)
    for v in members:
        levels[nodes[v]["x"]].append(v)
    widths = [len(vs) for vs in levels.values()]
    d["levels"] = len(levels)
    d["max_width"] = max(widths)
    d["mean_width"] = round(len(members) / len(levels), 2)
    next_level = defaultdict(Counter)
    for x, vs in levels.items():
        for v in vs:
            for ch in (nodes[v]["hi"], nodes[v]["lo"]):
                if ch and ch in mset:
                    next_level[x][nodes[ch]["x"]] += 1
    all_edges = sum(sum(cnt.values()) for cnt in next_level.values())
    d["chain_frac"] = round(sum(max(cnt.values()) for cnt in next_level.values() if cnt) / all_edges, 3) if all_edges else 0.0
    if len(roots) != 1:
        return d
    root = roots[0]
    # levels and counts from the root: every edge one level down, hi adds one
    lvl = {root: 0}
    pos = {root: 0}
    order = [root]
    layered = grid = True
    i = 0
    while i < len(order):
        v = order[i]
        i += 1
        for ch, inc in ((nodes[v]["hi"], 1), (nodes[v]["lo"], 0)):
            if not ch or ch not in mset:
                continue
            if ch in lvl:
                if lvl[ch] != lvl[v] + 1:
                    layered = False
                if pos[ch] != pos[v] + inc:
                    grid = False
            else:
                lvl[ch] = lvl[v] + 1
                pos[ch] = pos[v] + inc
                order.append(ch)
    level_var = {}
    for v, l in lvl.items():
        if level_var.setdefault(l, nodes[v]["x"]) != nodes[v]["x"]:
            layered = False
    if len(set(level_var.values())) != len(level_var) or len(lvl) != len(members):
        layered = False
    d["layered"], d["grid"] = layered, grid
    by_level = defaultdict(list)
    for v, l in lvl.items():
        by_level[l].append(v)
    d["pos_unique"] = all(len({pos[v] for v in vs}) == len(vs) for vs in by_level.values())
    d["max_count"] = max(pos.values())
    d["counter"] = bool(layered and grid and d["pos_unique"] and d["chain_frac"] >= 0.9 and d["levels"] >= 3
                        and d["mean_width"] >= 1.5 and d["max_count"] >= 2)
    if d["counter"] and root in asserted_set:
        L = len(level_var)
        node_vars = {v: nodes[v]["x"] for v in members}
        lo_t, hi_t, complete, used, nst = _dd_bound(ix, root, level_var, L, node_vars, max_states=4 * len(members) + 100_000)
        d["walk_states"] = nst
        d["cids"] |= used
        d["cids"].add(ix.unit_id[root])
        if complete and lo_t is not None:
            d["lo_total"], d["hi_total"] = lo_t, hi_t
            if hi_t < L and lo_t > 0:
                d["bound"] = ("between", lo_t, hi_t)
            elif hi_t < L:
                d["bound"] = ("atmost", hi_t)
            elif lo_t > 0:
                d["bound"] = ("atleast", lo_t)
        elif complete and lo_t is None:
            d["bound"] = ("none-accepted",)
    return d


# ---------------------------------------------------------------------------
# popcount adder trees (netlist recovery from tools/netlist_census.py)
# ---------------------------------------------------------------------------


def find_popcounts(clauses, n_vars):
    """Adder networks (connected groups of half and full adders) with what
    their leaves are and what their top bits feed.  Returns (list of network
    dicts, note); each dict carries the clause ids of its cells' gates."""
    if n_vars > nc.MAX_GATE_VARS:
        return [], "too-big"
    gates, used, deadlocks, topo_numbered = nc.recover_gates(clauses, n_vars)
    if not gates:
        return [], "no-gates"
    if len(gates) > nc.MAX_FULL_GATES:
        return [], "gates-only"
    order = nc.topo_order(gates)
    rep, merged = nc.structural_hash(gates, order)
    present = {abs(l) for c in clauses for l in c}
    primary = present - set(gates)
    cells = nc.find_adders(gates, order, rep)
    if not cells:
        return [], f"gates={len(gates)} no-adders"
    _nets, wconf, cells = nc.analyse_networks(cells, gates, rep, primary)   # the weight-consistent cells
    units = {abs(c[0]) for c in clauses if len(c) == 1}
    occ = defaultdict(list)
    for i, c in enumerate(clauses):
        for l in c:
            occ[abs(l)].append(i)
    readers = defaultdict(list)   # gates reading a variable
    for v, (sup, tt, cl) in gates.items():
        for u in sup:
            readers[u].append(v)
    # components over cells through produced wires only (a cell's output
    # read by another cell): two counts that merely share an input are two
    # counts, not one
    producer = {}
    for ci, (kind, leaves, negmask, s, sneg, cv, cneg) in enumerate(cells):
        producer[s] = ci
        producer[cv] = ci
    linked = defaultdict(set)
    for ci, (kind, leaves, negmask, s, sneg, cv, cneg) in enumerate(cells):
        for x in leaves:
            d = producer.get(x)
            if d is not None and d != ci:
                linked[ci].add(d)
                linked[d].add(ci)
    comp = {}
    comps = []
    for ci in range(len(cells)):
        if ci in comp:
            continue
        comp[ci] = len(comps)
        stack = [ci]
        members = []
        while stack:
            c = stack.pop()
            members.append(c)
            for d in linked[c]:
                if d not in comp:
                    comp[d] = len(comps)
                    stack.append(d)
        comps.append(members)
    info = []
    root_owner = {}
    outs_of_comp = []
    for ki, members in enumerate(comps):
        outs, ins = set(), set()
        for c in members:
            kind, leaves, negmask, s, sneg, cv, cneg = cells[c]
            outs.update((s, cv))
            ins.update(leaves)
        info.append((ins - outs, outs - ins))
        outs_of_comp.append(outs)
        for r in outs - ins:
            root_owner[r] = ki
    out = []
    for ki, members in enumerate(comps):
        leaves, roots = info[ki]
        use = Counter()
        for r in roots:
            if r in units:
                use["unit"] += 1
                continue
            for g in readers.get(r, ()):
                sup, tt, cl = gates[g]
                n = len(sup)
                other = [u for u in sup if u != r]
                if n == 2 and tt in (nc.XOR2, nc.XNOR2):
                    o = other[0]
                    if o in root_owner and root_owner[o] != ki:
                        use["xor_other_net"] += 1
                    elif o in primary:
                        use["xor_primary"] += 1
                    else:
                        use["xor"] += 1
                elif n == 2:
                    use["and_or"] += 1
                else:
                    use[f"gate{n}"] += 1
            cons = [i for i in occ[r] if i not in used]   # constraint clauses (not gate definitions) on the root
            if cons:
                use["constraint"] += len(cons)
        cids = set()
        seen = set()
        for c in members:
            kind, cl_leaves, negmask, s, sneg, cv, cneg = cells[c]
            stop = set(cl_leaves)
            stack = [s, cv]
            while stack:
                u = stack.pop()
                r, ng = rep.get(u, (u, False))
                if r in stop or r in seen or r not in gates:
                    continue
                seen.add(r)
                cids |= gates[r][2]
                stack.extend(gates[r][0])
        # a popcount sums bits of one weight; a word adder's inputs sit at
        # several (a sum keeps its inputs' weight, a carry doubles it)
        wuf = nc.WeightedUF()
        for c in members:
            kind, cl_leaves, negmask, s, sneg, cv, cneg = cells[c]
            base = cl_leaves[0]
            for x in cl_leaves[1:]:
                wuf.union(x, base, 0)
            wuf.union(s, base, 0)
            wuf.union(cv, base, 1)
        leaf_w = {wuf.find(v) for v in leaves}
        # every internal wire is read exactly once (a sum read by two adders
        # is counted twice) and in the polarity it was produced in
        prod_neg = {}                     # internal wire -> its produced literal is the negation
        for d in members:
            _, _, _, sv_, sn_, cv_, cn_ = cells[d]
            prod_neg[sv_] = bool(sn_)
            prod_neg[cv_] = bool(cn_)
        reads = Counter()
        for c in members:
            kind, cl_leaves, negmask, s, sneg, cv, cneg = cells[c]
            for i, x in enumerate(cl_leaves):
                if x in prod_neg:
                    reads[x] += 1
                    if bool((negmask >> i) & 1) != prod_neg[x]:
                        reads[x] += 100       # read in the other polarity: not a count
        # (primary inputs read twice are not checked: xor clauses define every
        # variable in them, gate recovery can orient a full adder backwards,
        # and a real popcount then shows its true sum as a leaf read twice; so
        # popcounts are candidates, never counted structures, see counted())
        single_use = all(n_ == 1 for n_ in reads.values())
        out.append({
            "cells": len(members), "fa": sum(1 for c in members if cells[c][0] == "FA"),
            "ha": sum(1 for c in members if cells[c][0] != "FA"), "leaves": len(leaves),
            "primary_leaves": sum(1 for v in leaves if v in primary), "roots": len(roots), "use": dict(use), "cids": cids,
            "equal_weight": len(leaf_w) == 1 and wuf.conflicts == 0 and single_use,
        })
    out.sort(key=lambda d: -d["cells"])
    return out, f"gates={len(gates)} cells={len(cells)}"


# ---------------------------------------------------------------------------
# direct (binomial) encodings
# ---------------------------------------------------------------------------


def find_binomial(clauses, max_len=9, max_t=14):
    """Direct (binomial) encodings: a set S of t literals and a size m with
    every m-subset of S written as a clause, the subset's literals all of
    one sign, possibly with one more literal g in every clause (a guard).
    Reading: at most m-1 of the negated clause literals are true, that is
    at most m-1 of S for negative clauses and at least t-m+1 of S for
    positive ones; when g is there, only while g is false.  Only m >= 3
    (k >= 2) and t > m are kept: pairwise at-most-one is the structure
    census's (tools/rl_structure.py) and a single clause is no count."""
    present = {}                # (guard or 0, frozenset body) -> clause id
    sub = defaultdict(set)      # (guard, body minus one literal) -> {the missing literal}
    for i, c in enumerate(clauses):
        L = len(c)
        if L < 3 or L > max_len:
            continue
        npos = sum(1 for l in c if l > 0)
        decomps = []
        if npos == 0 or npos == L:
            decomps.append((0, c))
            if L >= 4:
                decomps.extend((g, tuple(l for l in c if l != g)) for g in c)
        elif L >= 4 and (npos == 1 or npos == L - 1):
            g = next(l for l in c if (l > 0) == (npos == 1))
            decomps.append((g, tuple(l for l in c if l != g)))
        for g, body in decomps:
            fb = frozenset(body)
            if (g, fb) in present:
                continue
            present[(g, fb)] = i
            for l in fb:
                sub[(g, fb - {l})].add(l)
    used = set()
    fams = []
    unguarded_cids = set()   # clauses of the accepted unguarded families
    # unguarded families first: every clause of an all-one-sign family also
    # reads as a guarded clause (any literal as the guard), and such a
    # guarded view is the same constraint again, not another one
    for (g, R), cid in sorted(present.items(), key=lambda kv: kv[0][0] != 0):
        if (g, R) in used:
            continue
        m = len(R)
        if m < 3:
            continue
        cand = None
        for r in R:
            ext = sub.get((g, R - {r}), set())
            cand = set(ext) if cand is None else cand & ext
            if not cand:
                break
        cand = (cand or set()) - R
        sign = next(iter(R)) > 0
        cand = {l for l in cand if (l > 0) == sign and l != g and -l not in R}
        if not cand:
            continue
        S = sorted(R | cand, key=abs)
        t = len(S)
        if t > max_t:
            continue
        ids = []
        for T in itertools.combinations(S, m):
            j = present.get((g, frozenset(T)))
            if j is None:
                ids = None
                break
            ids.append(j)
        if ids is None:
            continue
        for T in itertools.combinations(S, m):
            used.add((g, frozenset(T)))
        if g and set(ids) <= unguarded_cids:
            continue   # a guarded view of an accepted unguarded family
        if not g:
            unguarded_cids.update(ids)
        k = m - 1
        form = ("atleast", t - k) if sign else ("atmost", k)
        fams.append({"S": S, "t": t, "m": m, "form": form, "guard": g, "cids": set(ids)})
    return fams


# ---------------------------------------------------------------------------
# one cell
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# every reported bound proven before it counts
# ---------------------------------------------------------------------------

SOLVER = Path(os.environ.get("CARD_CENSUS_SOLVER", str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver")))


def _at_most(lits, k, top):
    """Sinz clauses for at most k of `lits`, auxiliaries from top + 1.
    Returns (clauses, new top)."""
    n = len(lits)
    if k >= n:
        return [], top
    if k < 0:
        return [[]], top
    if k == 0:
        return [[-l] for l in lits], top
    s = {}
    for i in range(1, n):
        for j in range(1, k + 1):
            top += 1
            s[i, j] = top
    out = [[-lits[0], s[1, 1]]] + [[-s[1, j]] for j in range(2, k + 1)]
    for i in range(2, n):
        x = lits[i - 1]
        out.append([-x, s[i, 1]])
        out.append([-s[i - 1, 1], s[i, 1]])
        for j in range(2, k + 1):
            out.append([-x, -s[i - 1, j - 1], s[i, j]])
            out.append([-s[i - 1, j], s[i, j]])
        out.append([-x, -s[i - 1, k]])
    out.append([-lits[n - 1], -s[n - 1, k]])
    return out, top


def _at_least(lits, m, top):
    """Clauses for at least m of `lits` (n * m auxiliaries, from top + 1):
    s(i, j) says at least j of the first i are true, and only that
    direction is written (enough for a satisfiability check); s(n, m) is
    asserted.  Returns (clauses, new top)."""
    n = len(lits)
    if m <= 0:
        return [], top
    if m > n:
        return [[]], top
    s = {}
    for i in range(1, n + 1):
        for j in range(1, min(i, m) + 1):
            top += 1
            s[i, j] = top
    out = [[-s[1, 1], lits[0]]]
    for i in range(2, n + 1):
        x = lits[i - 1]
        for j in range(1, min(i, m) + 1):
            prev_j = s.get((i - 1, j))       # absent when j > i - 1: that count cannot be reached yet
            if j == 1:
                out.append([-s[i, 1]] + ([prev_j] if prev_j else []) + [x])
            else:
                out.append([-s[i, j]] + ([prev_j] if prev_j else []) + [x])
                out.append([-s[i, j]] + ([prev_j] if prev_j else []) + [s[i - 1, j - 1]])
    out.append([s[n, m]])
    return out, top


def _count(lits, kind, bound, top):
    """Clauses for `kind` ('atleast' or 'atmost') `bound` of `lits`, in the
    smaller of the two forms (a count of the literals or of their
    negations).  Returns (clauses, new top, size estimate)."""
    n = len(lits)
    if kind == "atleast":
        m = bound
        if m <= n - m:
            return (*_at_least(list(lits), m, top), n * max(m, 0))
        cl, t2 = _at_most([-l for l in lits], n - m, top)
        return cl, t2, n * max(n - m, 0)
    K = bound
    if K <= n - K:
        cl, t2 = _at_most(list(lits), K, top)
        return cl, t2, n * max(K, 0)
    return (*_at_least([-l for l in lits], n - K, top), n * max(n - K, 0))


def _check_instance(clauses, cids, lits, want, b):
    """The proof instance: the structure's clauses plus the opposite count,
    with the variables renumbered 1..N (a cell's variable numbers run to
    millions and the solver sizes its arrays from the header).  Returns
    (top, clauses)."""
    base = [list(clauses[i]) for i in sorted(cids)]
    ren = {}
    for v in sorted({abs(l) for c in base for l in c} | {abs(l) for l in lits}):
        ren[v] = len(ren) + 1

    def r(l):
        return ren[l] if l > 0 else -ren[-l]

    base = [[r(l) for l in c] for c in base]
    extra, top, _ = _count([r(l) for l in lits], want, b, len(ren))
    return top, base + extra


VERIFY_MAX_AUX = 2_000_000   # a bound whose check would need more auxiliaries stays unverified


def _implies(clauses, cids, lits, kind, k):
    """Whether the structure's own clauses imply the claim: `atmost` k (no
    k + 1 of `lits` can be true) or `atleast` k (no n - k + 1 false).
    True when proven, False when refuted, None when unknown."""
    n = len(lits)
    if kind == "atmost" and k >= n:
        return True
    if kind == "atleast" and k <= 0:
        return True
    # the opposite count, in its smaller form; refused above a size cap
    want, b = ("atleast", k + 1) if kind == "atmost" else ("atmost", k - 1)
    size = n * min(max(b, 0), max(n - b, 0))
    if size > VERIFY_MAX_AUX:
        return None
    top, cnf = _check_instance(clauses, cids, lits, want, b)
    with tempfile.NamedTemporaryFile("w", suffix=".cnf", delete=False) as f:
        f.write(f"p cnf {top} {len(cnf)}\n")
        for c in cnf:
            f.write(" ".join(map(str, c)) + " 0\n")
        path = f.name
    try:
        # the repository's 16 GB cap through prlimit, and a KILL after 65 s
        # from timeout, which outlives a killed census (no preexec_fn: it
        # forces a full fork of a worker holding gigabytes, 29x slower on
        # bp4's 33,908 checks)
        p = subprocess.run(["timeout", "-s", "KILL", "65", "prlimit", f"--as={16000 << 20}", "--",
                            str(SOLVER), "-q", "-n", "--structsym=0", "--structparity=0", "--structcount=0",
                            "--structarith=0", "--structsort=0", "--gauss=0", path], capture_output=True, timeout=70)
        return {20: True, 10: False}.get(p.returncode)
    except (OSError, subprocess.TimeoutExpired):
        return None
    finally:
        os.unlink(path)


def _brute_sat(cnf, nvars):
    """Satisfiability by enumeration with unit propagation first (the
    encodings' auxiliaries are mostly forced); for the self-test only."""
    assign = {}
    changed = True
    while changed:
        changed = False
        for c in cnf:
            unassigned = [l for l in c if abs(l) not in assign]
            if any(assign.get(abs(l)) == (l > 0) for l in c if abs(l) in assign):
                continue
            if not unassigned:
                return False
            if len(unassigned) == 1:
                l = unassigned[0]
                assign[abs(l)] = l > 0
                changed = True
    free = [v for v in range(1, nvars + 1) if v not in assign]
    if len(free) > 20:
        raise ValueError("too many free variables for the brute-force check")
    for mask in range(1 << len(free)):
        a = dict(assign)
        for i, v in enumerate(free):
            a[v] = bool((mask >> i) & 1)
        if all(any(a.get(abs(l)) == (l > 0) for l in c) for c in cnf):
            return True
    return False


def verify_bounds(res, clauses):
    """Prove every bound a detector reports from that structure's own
    clauses; a bound not proven is dropped (the structure is reported as
    free) and counted, so a structural misreading can lose a bound but
    never report a false one.  Returns (proven, refuted, unknown)."""
    tally = Counter()

    def check(cids, lits, bound):
        if bound is None or bound[0] in ("none", "none-accepted"):
            return True
        if bound[0] == "between":
            parts = [("atleast", bound[1]), ("atmost", bound[2])]
        else:
            parts = [(bound[0], bound[1])]
        for kind, k in parts:
            r = _implies(clauses, cids, lits, kind, k)
            if r is not True:
                tally["refuted" if r is False else "unknown"] += 1
                return False
        tally["proven"] += 1
        return True

    for t in res["trees"]:
        if t["bound"][0] != "none" and not check(t["cids"], t["leaves"], t["bound"]):
            t["bound"] = ("none", t.get("width", 0))
            t["bound_dropped"] = True
    for d in res["nets"]:
        for g in d["groups"]:
            if g["bound"] and g["bound"][0] != "none" and not check(g["cids"], g["inputs"], g["bound"]):
                g["bound"] = ("none", g["n"])
                g["bound_dropped"] = True
    if res.get("dd"):
        for d in res["dd"]["diagrams"]:
            if d.get("bound") and not check(d["cids"], d["inputs"], d["bound"]):
                d["bound"] = None
                d["bound_dropped"] = True
    return tally["proven"], tally["refuted"], tally["unknown"]


def analyse(clauses, n_vars, want_pop=True):
    """Every detector on one clause list.  Returns a dict of results."""
    ix = Index(clauses)
    res = {"clauses": len(clauses), "vars": n_vars}
    t0 = time.time()
    nodes = find_unary_nodes(ix)
    trees, nets = build_trees(ix, nodes)
    res["nodes"] = nodes
    res["trees"] = trees
    res["nets"] = nets
    res["unary_s"] = round(time.time() - t0, 1)
    t0 = time.time()
    res["dd"] = find_dd(ix, clauses)
    res["dd_s"] = round(time.time() - t0, 1)
    t0 = time.time()
    res["binom"] = find_binomial(clauses)
    res["binom_s"] = round(time.time() - t0, 1)
    t0 = time.time()
    if want_pop:
        pops, note = find_popcounts(clauses, n_vars)
    else:
        pops, note = [], "skipped"
    res["pop"] = pops
    res["pop_note"] = note
    res["pop_s"] = round(time.time() - t0, 1)
    t0 = time.time()
    res["bounds_proven"], res["bounds_refuted"], res["bounds_unknown"] = verify_bounds(res, clauses)
    res["verify_s"] = round(time.time() - t0, 1)
    return res


def _effective(lits, n):
    """(pairs, n_eff): an input and its negation in one count add exactly
    1 together, so p such pairs leave n - 2p free inputs and shift every
    bound by p."""
    if not lits:
        return 0, n
    s = set(lits)
    p = sum(1 for l in s if l > 0 and -l in s)
    return p, n - 2 * p


def restrictive(bound, n, lits=None):
    """Whether a bound restricts its n inputs at all (with complementary
    inputs cancelled: x and -x always add 1)."""
    if not bound or bound[0] in ("none", "none-accepted"):
        return False
    p, m = _effective(lits, n)
    if bound[0] == "atmost":
        return bound[1] - p < m
    if bound[0] == "atleast":
        return bound[1] - p > 0
    if bound[0] == "between":
        return bound[1] - p > 0 or bound[2] - p < m
    return False


def counts_k2(bound, n, lits=None):
    """Whether a bound is a counting constraint with k >= 2, the census's
    subject: at most K of the inputs (or of their negations, for an at-least
    bound) with 2 <= K <= n - 2; K = 1 is at-most-one, K = n - 1 a single
    clause, K = 0 units.  Complementary inputs are cancelled first."""
    if lits is not None and len(set(lits)) != len(lits):
        return False             # an input read twice: a weighted sum, not a count
    if not restrictive(bound, n, lits):
        return False
    p, m = _effective(lits, n)

    def ok(K):
        return 2 <= K <= m - 2

    if bound[0] == "atmost":
        return ok(bound[1] - p)
    if bound[0] == "atleast":
        return ok(m - (bound[1] - p))
    if bound[0] == "between":
        return ok(m - (bound[1] - p)) or ok(bound[2] - p)
    return False


def counted(res):
    """The structures the census counts, split by detector:
    seq / tot trees with a bound (a unit or the overflow clauses), free
    trees (registers whose outputs other clauses read), comparator networks
    with a clean group, a counter DD, popcount networks and binomial
    families with k >= 2.  Pairs (a tree over two inputs) and partial
    trees are not counted."""
    trees = res["trees"]

    def bounded(t):
        # a bound that restricts nothing (at most n or more of n inputs, at
        # least 0, complementary inputs cancelled) or that is no count with
        # k >= 2 (at most one, a single clause) is not counted: free
        return counts_k2(t["bound"], t["n"], t["leaves"])

    out = {
        "seq": [t for t in trees if t["kind"] == "seq" and bounded(t) and not t["mirror"]],
        "seq_free": [t for t in trees if t["kind"] == "seq" and not bounded(t) and not t["mirror"]],
        "tot": [t for t in trees if t["kind"] == "tot" and t["n"] >= 3 and bounded(t) and not t["mirror"]],
        "mirror": [t for t in trees if t["kind"] in ("seq", "tot") and t["mirror"]],
        "tot_free": [t for t in trees if t["kind"] == "tot" and t["n"] >= 3 and not bounded(t) and not t["mirror"]],
        "pairs": [t for t in trees if t["kind"] == "tot" and t["n"] < 3],
        "partial": [t for t in trees if t["kind"] == "partial" and t["n"] >= 3],
    }
    # networks with a clean group; a network and its dual (two-way
    # comparators) read one unit twice: the upper-bound reading is counted,
    # the other group is a mirror (coverage only)
    clean = []
    for d in res["nets"]:
        # (a group with unranked outputs still counts when its own bound
        # was proven: a partly sorted network can carry one valid threshold)
        groups = [g for g in d["groups"] if (g["clean"] or (g["bound"] and g["bound"][0] != "none")) and g["n"] >= 3]
        if d["full"] >= 2 and groups:
            clean.append((d, groups))
    by_unit = defaultdict(list)
    for d, groups in clean:
        for g in groups:
            if g["bound_cid"] is not None:
                by_unit[g["bound_cid"]].append(g)
    mirror_ids = set()

    def duals(g, h):
        # the same count read twice: one group's inputs are the other's
        # negations and the bounds agree (at most k of X = at least n-k of -X)
        if g["n"] != h["n"] or sorted(g["inputs"]) != sorted(-x for x in h["inputs"]):
            return False
        bg, bh = g["bound"], h["bound"]
        if bg[0] == bh[0]:
            return False
        a, b = (bg, bh) if bg[0] == "atmost" else (bh, bg)
        return a[0] == "atmost" and b[0] == "atleast" and a[1] == g["n"] - b[1]

    for gs in by_unit.values():
        if len(gs) > 1:
            keep = next((g for g in gs if g["bound"][0] == "atmost"), gs[0])
            mirror_ids.update(id(g) for g in gs if g is not keep and duals(g, keep))
    kept = [{**d, "groups": [g for g in groups if id(g) not in mirror_ids]}
            for d, groups in clean if any(id(g) not in mirror_ids for g in groups)]
    out["nets"] = [d for d in kept if any(counts_k2(g["bound"], g["n"], g["inputs"]) for g in d["groups"])]
    out["net_free"] = [d for d in kept if not any(counts_k2(g["bound"], g["n"], g["inputs"]) for g in d["groups"])]
    out["net_mirror"] = [g for d, groups in clean for g in groups if id(g) in mirror_ids]
    out["dd"] = [d for d in (res["dd"]["diagrams"] if res["dd"] else []) if d["counter"]]
    for d in out["dd"]:
        if d.get("bound") and not counts_k2(d["bound"], len(d.get("inputs", ())) or d.get("levels", 0), d.get("inputs")):
            d["bound"] = None   # a bound that restricts nothing, or no count with k >= 2, reads as free
    # popcount adder networks are candidates only (reported, never counted
    # or covered): their leaf set rests on gate recovery, which can orient a
    # full adder backwards, and no bound of theirs is proven
    out["pop"] = [p for p in res["pop"] if p["cells"] >= 2 and p["leaves"] >= 3 and p["primary_leaves"] >= 0.5 * p["leaves"]
                  and p.get("equal_weight")]
    out["binom"] = res["binom"]
    return out


def bound_text(t):
    """A tree's bound on its leaf literals; `(neg)` when every leaf is a
    negated variable (at most k of the negations = at least n-k of the
    variables)."""
    b = t["bound"]
    if b[0] == "none":
        return f"free(w{t['width']})"
    neg = bool(t["leaves"]) and all(l < 0 for l in t["leaves"])
    return f"{b[0]}{b[1]}" + ("(neg)" if neg else "")


def dd_bound_text(d):
    """A diagram's verified bound: from the walk over its asserted root
    (the lowest and highest accepted totals); `free` when the root is not
    asserted or the walk gave no bound."""
    b = d["bound"]
    if not b:
        return "free"
    if b[0] == "between":
        return f"between{b[1]}-{b[2]}"
    if b[0] == "none-accepted":
        return "rejects-all"
    return f"{b[0]}{b[1]}"


def _tally(items, key):
    c = Counter(key(x) for x in items)
    return ";".join(f"{k}x{v}" for k, v in sorted(c.items(), key=lambda kv: (-kv[1], str(kv[0])))[:8])


def summarize(res):
    """The TSV row fields from an analysis."""
    cn = counted(res)
    n = max(1, res["clauses"])

    def cov(items):
        s = set()
        for x in items:
            s |= x["cids"]
        return s

    seq_c = cov(cn["seq"] + cn["seq_free"] + [t for t in cn["mirror"] if t["kind"] == "seq"])
    tot_c = cov(cn["tot"] + cn["tot_free"] + [t for t in cn["mirror"] if t["kind"] == "tot"])
    net_c = set()
    for d in cn["nets"]:
        for g in d["groups"]:
            net_c |= g["cids"]
    for g in cn["net_mirror"]:
        net_c |= g["cids"]
    for d in cn["net_free"]:
        for g in d["groups"]:
            net_c |= g["cids"]
    dd_c = cov(cn["dd"])
    pop_c = cov(cn["pop"])
    binom_c = cov(cn["binom"])
    alld = res["dd"]["diagrams"] if res["dd"] else []
    shown = cn["dd"] or alld
    row = {
        "bounds_proven": res.get("bounds_proven", 0), "bounds_refuted": res.get("bounds_refuted", 0),
        "bounds_unknown": res.get("bounds_unknown", 0),
        "seq_count": len(cn["seq"]), "seq_n": _tally(cn["seq"], lambda t: t["n"]), "seq_k": _tally(cn["seq"], bound_text),
        "seq_nk": _tally(cn["seq"], lambda t: f"{t['n']}:{bound_text(t)}"),
        "seq_free": len(cn["seq_free"]), "seq_two_way": round(sum(t["two_way"] for t in cn["seq"]) / len(cn["seq"]), 2) if cn["seq"] else 0.0,
        "tot_count": len(cn["tot"]), "tot_n": _tally(cn["tot"], lambda t: t["n"]), "tot_k": _tally(cn["tot"], bound_text),
        "tot_nk": _tally(cn["tot"], lambda t: f"{t['n']}:{bound_text(t)}"),
        "tot_nodes": sum(t["nodes"] for t in cn["tot"]), "tot_free": len(cn["tot_free"]), "tot_free_n": _tally(cn["tot_free"], lambda t: t["n"]),
        "dual_trees": sum(1 for t in cn["tot"] + cn["seq"] if t["dual"]), "mirror_trees": len(cn["mirror"]),
        "pair_nodes": len(cn["pairs"]), "partial_count": len(cn["partial"]), "partial_n": _tally(cn["partial"], lambda t: t["n"]),
        "net_count": len(cn["nets"]), "net_n": _tally([g for d in cn["nets"] for g in d["groups"]], lambda g: g["n"]),
        "net_k": _tally([g for d in cn["nets"] for g in d["groups"]],
                        lambda g: (f"{g['bound'][0]}{g['bound'][1]}" + ("(neg)" if all(l < 0 for l in g["inputs"]) else ""))
                        if g["bound"][0] != "none" else "free"),
        "net_gates": sum(d["gates"] for d in cn["nets"]), "net_free": len(cn["net_free"]),
        "dd_kind": ("counter" if cn["dd"] else "ite") if alld else "", "dd_count": len(cn["dd"]),
        "dd_nodes": sum(d["nodes"] for d in shown), "dd_levels": _tally(cn["dd"], lambda d: d["levels"]),
        "dd_width": max((d["max_width"] for d in shown), default=0),
        "dd_max_count": max((d["max_count"] for d in cn["dd"]), default=0),
        "dd_asserted": sum(1 for d in cn["dd"] if d["asserted"]),
        "dd_bounds": _tally(cn["dd"], dd_bound_text),
        "pop_nets": len(cn["pop"]), "pop_compared": sum(1 for p in cn["pop"] if p["use"].get("unit") or p["use"].get("xor_other_net")),
        "pop_desc": " | ".join(
            f"cells={p['cells']}(fa={p['fa']},ha={p['ha']}),leaves={p['leaves']}/{p['primary_leaves']}prim,roots={p['roots']},use={p['use']}"
            for p in cn["pop"][:3]),
        "pop_note": res["pop_note"],
        "binom_count": len(cn["binom"]), "binom_desc": _tally(cn["binom"], lambda f: f"{f['form'][0]}{f['form'][1]}of{f['t']}" + ("|g" if f["guard"] else "")),
        "seq_cov": round(len(seq_c) / n, 4), "tot_cov": round(len(tot_c) / n, 4), "net_cov": round(len(net_c) / n, 4),
        "dd_cov": round(len(dd_c) / n, 4), "pop_cov": round(len(pop_c) / n, 4), "binom_cov": round(len(binom_c) / n, 4),
        # popcounts are candidates: their clauses are not in the total
        "cov_total": round(len(seq_c | tot_c | net_c | dd_c | binom_c) / n, 4),
    }
    return row


COLUMNS = ["stem", "family", "vars", "clauses", "truncated",
           "seq_count", "seq_n", "seq_k", "seq_nk", "seq_free", "seq_two_way",
           "tot_count", "tot_n", "tot_k", "tot_nk", "tot_nodes", "tot_free", "tot_free_n", "dual_trees", "mirror_trees", "pair_nodes", "partial_count", "partial_n",
           "net_count", "net_n", "net_k", "net_gates", "net_free",
           "dd_kind", "dd_count", "dd_nodes", "dd_levels", "dd_width", "dd_max_count", "dd_asserted", "dd_bounds",
           "pop_nets", "pop_compared", "pop_desc", "pop_note", "binom_count", "binom_desc",
           "seq_cov", "tot_cov", "net_cov", "dd_cov", "pop_cov", "binom_cov", "cov_total",
           "bounds_proven", "bounds_refuted", "bounds_unknown", "len_hist", "detect_s", "error"]


def census_one(args):
    stem, path, family, max_clauses, want_pop = args
    t0 = time.time()
    row = {"stem": stem, "family": family}
    try:
        n_vars, n_cls, clauses, truncated = parse_prefix(Path(path), max_clauses)
        row.update(vars=n_vars, clauses=len(clauses), truncated=int(truncated))
        lens = Counter(len(c) for c in clauses)
        top = sorted(lens.items(), key=lambda kv: -kv[1])[:6]
        row["len_hist"] = ";".join(f"{k}:{v}" for k, v in top) + (f";max{max(lens)}" if lens else "")
        res = analyse(clauses, n_vars, want_pop=want_pop)
        row.update(summarize(res))
    except Exception as e:  # noqa: BLE001
        row["error"] = f"{type(e).__name__}: {str(e)[:100]}"
    row["detect_s"] = round(time.time() - t0, 1)
    return row


def _families():
    fam = {}
    if CELLS.is_file():
        with open(CELLS) as f:
            for r in csv.DictReader(f, delimiter="\t"):
                fam[r["stem"]] = r["family"]
    return fam


def cmd_census(args):
    fam = _families()
    files = []
    if args.files:
        files = [Path(p) for p in args.files]
    elif args.cells:
        stems = [l.strip() for l in open(args.cells) if l.strip()]
        for s in stems:
            p = SUITE / f"{s}.cnf.xz"
            if not p.is_file():
                print(f"missing: {p}", file=sys.stderr)
                continue
            files.append(p)
    else:
        files = sorted(SUITE.glob("*.cnf.xz"))
    stems = set(args.stems.split(",")) if args.stems else None
    jobs = []
    for f in files:
        stem = f.name[: -len(".cnf.xz")] if f.name.endswith(".cnf.xz") else f.stem
        if stems and not any(s in stem for s in stems):
            continue
        jobs.append((stem, str(f), fam.get(stem, ""), args.max_clauses, not args.no_pop))
    if args.order_from:
        # slowest first (the largest detect_s any given table records), so one
        # slow cell does not start last and hold the run
        cost = defaultdict(float)
        for table in args.order_from.split(","):
            with open(table) as tf:
                for r in csv.DictReader(tf, delimiter="\t"):
                    try:
                        cost[r["stem"]] = max(cost[r["stem"]], float(r.get("detect_s") or 0))
                    except ValueError:
                        pass
        jobs.sort(key=lambda j: -cost.get(j[0], 0.0))
    with open(args.out, "w", newline="") as f, ProcessPoolExecutor(max_workers=args.workers) as ex:
        w = csv.DictWriter(f, fieldnames=COLUMNS, delimiter="\t", extrasaction="ignore")
        w.writeheader()
        f.flush()
        futures = [ex.submit(census_one, job) for job in jobs]
        for fut in as_completed(futures):
            row = fut.result()
            w.writerow(row)
            f.flush()
            print(f"{row['stem'][:60]:60} seq={row.get('seq_count','-'):>4} tot={row.get('tot_count','-'):>4} "
                  f"net={row.get('net_count','-'):>3} dd={row.get('dd_kind','')}:{row.get('dd_nodes','-'):>6} "
                  f"pop={row.get('pop_nets','-'):>2} binom={row.get('binom_count','-'):>5} cov={row.get('cov_total','-'):>6} "
                  f"{row.get('error','')} {row.get('detect_s','')}s", flush=True)
    return 0


def cmd_one(args):
    n_vars, n_cls, clauses, truncated = parse_prefix(Path(args.file), args.max_clauses)
    res = analyse(clauses, n_vars, want_pop=not args.no_pop)
    print(f"vars={n_vars} clauses={len(clauses)} truncated={truncated}")
    for k, v in summarize(res).items():
        print(f"  {k}: {v}")
    cn = counted(res)
    for t in sorted(cn["seq"] + cn["tot"] + cn["seq_free"] + cn["tot_free"], key=lambda t: -t["n"])[: args.show]:
        print(f"  tree {t['kind']} n={t['n']} bound={bound_text(t)} nodes={t['nodes']} nets={t['nets']} width={t['width']} "
              f"capped_all={t['capped_all']} two_way={t['two_way']} dual={t['dual']} leaves={t['leaves'][:12]}")
    for d in cn["nets"][: args.show]:
        print(f"  net n={d['n']} gates={d['gates']} (full={d['full']}, half={d['half']}) outputs={len(d['outputs'])} groups={len(d['groups'])}")
        for g in d["groups"][:4]:
            print(f"     group n={g['n']} gates={g['gates']} wires={len(g['wires'])} bound={g['bound']} "
                  f"thresholds={[g['thresholds'].get(w) for w in g['ranked'][:8]]} inputs={g['inputs'][:6]}")
    if res["dd"]:
        print(f"  dd: {len(res['dd']['diagrams'])} diagrams, candidates {res['dd']['candidates']}, ambiguous {res['dd']['ambiguous']}")
        for d in res["dd"]["diagrams"][: args.show]:
            print("    ", {k: v for k, v in d.items() if k != "cids"}, "bound:", dd_bound_text(d) if d["counter"] else "-")
    for p in cn["pop"][: args.show]:
        print("  pop net:", {k: v for k, v in p.items() if k != "cids"})
    for f in cn["binom"][: args.show]:
        print(f"  binomial {f['form']} t={f['t']} m={f['m']} guard={f['guard']} S={f['S'][:10]}")
    if args.uncovered:
        cov = set()
        for key in ("seq", "seq_free", "tot", "tot_free", "mirror", "binom"):   # popcount candidates cover nothing
            for x in cn[key]:
                cov |= x["cids"]
        for d in cn["nets"]:
            for g in d["groups"]:
                cov |= g["cids"]
        for g in cn["net_mirror"] + [g for d in cn["net_free"] for g in d["groups"]]:
            cov |= g["cids"]
        for d in cn["dd"]:
            cov |= d["cids"]
        left = [i for i in range(len(clauses)) if i not in cov]
        lens = Counter(len(clauses[i]) for i in left)
        print(f"  uncovered {len(left)} clauses by length: {sorted(lens.items())[:20]}")
        shown = Counter()
        for i in left:
            c = clauses[i]
            if shown[len(c)] < 4:
                shown[len(c)] += 1
                print("   ", c[:14])
        for t in cn["partial"][: args.show]:
            nd = res["nodes"][t["root"]]
            print(f"  partial root: A={nd.get('A')} B={nd.get('B')} R={nd['R']} c={nd['c']} tail={nd['tail']} bound={t['bound']}")
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------


def cmd_report(args):
    with open(args.table) as f:
        rows = list(csv.DictReader(f, delimiter="\t"))
    rows.sort(key=lambda r: (r.get("family", ""), r["stem"]))
    print(f"{len(rows)} cells")

    def num(r, k):
        try:
            return int(float(r.get(k) or 0))
        except ValueError:
            return 0

    for r in rows:
        bits = []
        if r.get("error"):
            bits.append(f"error={r['error']}")
        if num(r, "seq_count"):
            bits.append(f"seq {r['seq_count']} (n {r['seq_n']}; {r['seq_k']})")
        if num(r, "tot_count"):
            bits.append(f"tot {r['tot_count']} (n {r['tot_n']}; {r['tot_k']})")
        if num(r, "net_count"):
            bits.append(f"net {r['net_count']} (n {r['net_n']}; {r['net_k']})")
        if r.get("dd_kind") == "counter":
            bits.append(f"dd counter {r['dd_count']} ({r['dd_nodes']} nodes, levels {r['dd_levels']}, width {r['dd_width']}, bounds {r['dd_bounds']})")
        if num(r, "pop_nets"):
            bits.append(f"pop {r['pop_nets']} ({r['pop_compared']} compared)")
        if num(r, "binom_count"):
            bits.append(f"binomial {r['binom_count']} ({r['binom_desc']})")
        if num(r, "seq_free") or num(r, "tot_free") or num(r, "net_free"):
            bits.append(f"free seq {r.get('seq_free', 0)} tot {r.get('tot_free', 0)} net {r.get('net_free', 0)}")
        if not bits:
            bits.append("nothing" + (f" (ite dd {r['dd_nodes']} nodes)" if r.get("dd_kind") == "ite" else ""))
        tr = " prefix" if r.get("truncated") == "1" else ""
        print(f"{r['stem'][:58]:58} {r.get('family', '')[:14]:14} cov={r.get('cov_total', '')}{tr}: " + "; ".join(bits))
    return 0


# ---------------------------------------------------------------------------
# self test on toy encodings
# ---------------------------------------------------------------------------


class Toy:
    def __init__(self):
        self.n = 0
        self.clauses = []

    def new(self, k=1):
        out = list(range(self.n + 1, self.n + 1 + k))
        self.n += k
        return out if k > 1 else out[0]

    def add(self, *lits):
        c = normalize(list(lits))   # as parse_prefix does: no repeated literals, no tautologies
        if c is not None:
            self.clauses.append(c)


def toy_seq(n, k, units=True, last_column=False):
    """Sinz LT_SEQ: at most k of x_1..x_n.  Returns (toy, inputs, registers).
    units=False leaves the first column's s_{1,j} (j > 1) unconstrained
    instead of false; last_column=True gives x_n a register column too (the
    overflow row of every column then carries the bound)."""
    t = Toy()
    x = t.new(n)
    cols = n if last_column else n - 1
    s = [None] + [[None] + (t.new(k) if k > 1 else [t.new()]) for _ in range(cols)]  # s[i][j], i = 1..cols
    t.add(-x[0], s[1][1])
    if units:
        for j in range(2, k + 1):
            t.add(-s[1][j])
    for i in range(2, cols + 1):
        t.add(-x[i - 1], s[i][1])
        t.add(-s[i - 1][1], s[i][1])
        for j in range(2, k + 1):
            t.add(-x[i - 1], -s[i - 1][j - 1], s[i][j])
            t.add(-s[i - 1][j], s[i][j])
        t.add(-x[i - 1], -s[i - 1][k])
    if not last_column:
        t.add(-x[n - 1], -s[n - 1][k])
    return t, x, s


def toy_totalizer(n, k=None, leaf_arity=2, both=False, bounded=False):
    """Bailleux-Boufkhad totalizer over n inputs; a unit on r_{k+1} when
    k is given (bounded=True builds outputs only up to k+1 as PySAT does);
    both=True adds the at-least direction clauses (exactly-k capable);
    leaf_arity=3 builds direct 3-input leaves (the ITC2021 shape)."""
    t = Toy()
    x = t.new(n)
    cap = (k + 1) if (bounded and k is not None) else None

    def merge(A, B):
        p, q = len(A), len(B)
        c = p + q if cap is None else min(p + q, cap)
        R = t.new(c) if c > 1 else [t.new()]
        for i in range(0, p + 1):
            for j in range(0, q + 1):
                if 1 <= i + j <= c:
                    # (-a_i v -b_j v r_{i+j}) with a_0 = b_0 = true
                    lits = [R[i + j - 1]]
                    if i:
                        lits.append(-A[i - 1])
                    if j:
                        lits.append(-B[j - 1])
                    t.add(*lits)
                if both and i + j + 1 <= c:
                    # (a_{i+1} v b_{j+1} v -r_{i+j+1}) with a_{p+1} = b_{q+1} = false
                    lits2 = [-R[i + j]]
                    if i + 1 <= p:
                        lits2.append(A[i])
                    if j + 1 <= q:
                        lits2.append(B[j])
                    t.add(*lits2)
        return R

    def direct(X):
        tt = len(X)
        c = tt if cap is None else min(tt, cap)
        R = t.new(c) if c > 1 else [t.new()]
        for m in range(1, c + 1):
            for S in itertools.combinations(X, m):
                t.add(*([-s for s in S] + [R[m - 1]]))
        return R

    def build(xs):
        if len(xs) == 1:
            return [xs[0]]
        if leaf_arity == 3 and len(xs) == 3:
            return direct(xs)
        h = len(xs) // 2
        return merge(build(xs[:h]), build(xs[h:]))

    # group into leaves of arity 3 first when asked
    if leaf_arity == 3:
        groups = [x[i:i + 3] for i in range(0, n, 3)]
        vecs = [direct(g) if len(g) == 3 else (merge([g[0]], [g[1]]) if len(g) == 2 else [g[0]]) for g in groups]
        while len(vecs) > 1:
            nxt = []
            for i in range(0, len(vecs), 2):
                nxt.append(merge(vecs[i], vecs[i + 1]) if i + 1 < len(vecs) else vecs[i])
            vecs = nxt
        R = vecs[0]
    else:
        R = build(x)
    if k is not None and k < len(R):
        t.add(-R[k])
    return t, x, R


def toy_popcount(n, const):
    """n inputs summed by a chain of full adders into a 3-bit result, the
    result compared with a constant by units (sum == const)."""
    t = Toy()
    x = t.new(n)

    def fa(a, b, c):
        s, cy = t.new(), t.new()
        # s = a xor b xor c
        for sa in (1, -1):
            for sb in (1, -1):
                for sc in (1, -1):
                    par = (sa < 0) + (sb < 0) + (sc < 0)
                    ssign = -1 if par % 2 else 1
                    t.add(-sa * a, -sb * b, -sc * c, ssign * s)
        # cy = maj(a, b, c)
        t.add(-a, -b, cy); t.add(-a, -c, cy); t.add(-b, -c, cy)
        t.add(a, b, -cy); t.add(a, c, -cy); t.add(b, c, -cy)
        return s, cy

    def ha(a, b):
        s, cy = t.new(), t.new()
        t.add(-a, -b, -s); t.add(a, b, -s); t.add(-a, b, s); t.add(a, -b, s)
        t.add(-a, -b, cy); t.add(a, -cy); t.add(b, -cy)
        return s, cy

    # ripple: bits of weight 1, 2, 4 via full adders (n = 7: 7 = 3 + 3 + 1)
    s1, c1 = fa(x[0], x[1], x[2])
    s2, c2 = fa(x[3], x[4], x[5])
    s3, c3 = fa(s1, s2, x[6])          # weight 1 sum = s3, carry c3 (weight 2)
    s4, c4 = fa(c1, c2, c3)            # weight 2 sum = s4, carry c4 (weight 4)
    bits = [s3, s4, c4]
    for i, b in enumerate(bits):
        t.add(b if (const >> i) & 1 else -b)
    return t, x, bits


def toy_dd(n, k, t=None, x=None):
    """A reduced counter decision diagram for at most k of n: node (i, c)
    decides x_i after c of x_1..x_i-1 were true; hi -> (i+1, c+1), or the
    false terminal when c = k (a binary (-v v -x_i)); lo -> (i+1, c); a
    node whose two branches both end true is left out (its parent points
    at the true terminal); the root is asserted; the redundant
    (-v v hi v lo) is written as the rook cells do.  Built into toy t over
    inputs x when given.  Returns (toy, inputs, nodes, true terminal)."""
    t = t or Toy()
    x = x or t.new(n)
    tnode = t.new()  # the true terminal, asserted
    t.add(tnode)
    node = {}

    def get(i, c):
        if i >= n:
            return tnode
        if c < k and n - i <= k - c:
            return tnode          # even if every remaining input is true the bound holds
        if (i, c) not in node:
            node[(i, c)] = t.new()
            v = node[(i, c)]
            lo = get(i + 1, c)
            if c + 1 > k:
                t.add(-v, -x[i])
                t.add(-v, x[i], lo)
            else:
                hi = get(i + 1, c + 1)
                t.add(-v, -x[i], hi)
                t.add(-v, x[i], lo)
                t.add(-v, hi, lo)
        return node[(i, c)]

    root = get(0, 0)
    t.add(root)
    return t, x, node, tnode


def batcher(n):
    """Comparators (i, j), i < j, of Batcher's odd-even merge sort on n = 2^p wires."""
    def merge(lo, hi, r):
        step = r * 2
        if step < hi - lo:
            yield from merge(lo, hi, step)
            yield from merge(lo + r, hi, step)
            yield from ((i, i + r) for i in range(lo + r, hi - r, step))
        else:
            yield (lo, lo + r)

    def sort(lo, hi):
        if hi - lo >= 1:
            mid = lo + (hi - lo) // 2
            yield from sort(lo, mid)
            yield from sort(mid + 1, hi)
            yield from merge(lo, hi, 1)

    return list(sort(0, n - 1))


def toy_network(k, n=8, two_way=False, unit=None):
    """Batcher's odd-even merge sort on n wires, each comparator written in
    the one direction cardinality networks use (max <- a, max <- b,
    min <- a & b), or both ways (also max -> a v b, min -> a, min -> b);
    the outputs sorted in decreasing order, out_m true when at least m
    inputs are; a unit -out_{k+1} for at most k, or the unit given."""
    t = Toy()
    x = t.new(n)
    wire = list(x)
    for i, j in batcher(n):
        a, b = wire[i], wire[j]
        hi, lo = t.new(), t.new()
        t.add(-a, hi)
        t.add(-b, hi)
        t.add(-a, -b, lo)
        if two_way:
            t.add(-hi, a, b)
            t.add(-lo, a)
            t.add(-lo, b)
        wire[i], wire[j] = hi, lo
    t.add(unit(wire) if unit else -wire[k])
    return t, x, wire


def toy_binomial(t_size, m, sign, guard=False):
    """Every m-subset of t_size variables as a clause of one sign, with a
    guard literal in each when asked: negative clauses say at most m-1,
    positive ones at least t_size-m+1 (while the guard is false)."""
    t = Toy()
    x = t.new(t_size)
    g = t.new() if guard else None
    for S in itertools.combinations(x, m):
        lits = [s if sign > 0 else -s for s in S]
        if g:
            lits.append(g)
        t.add(*lits)
    return t, x, g


def _check(name, cond, detail=""):
    print(f"  {'ok  ' if cond else 'FAIL'} {name} {detail}")
    return bool(cond)


def cmd_selftest(args):
    ok = True
    print("sequential counter")
    for n, k, units, last in ((7, 3, True, False), (7, 3, False, False), (6, 2, True, True), (12, 5, True, False)):
        t, x, s = toy_seq(n, k, units=units, last_column=last)
        res = analyse(t.clauses, t.n, want_pop=False)
        seq = [tr for tr in res["trees"] if tr["kind"] == "seq"]
        good = len(seq) == 1 and seq[0]["n"] == n and seq[0]["bound"] == ("atmost", k) and sorted(seq[0]["leaves"]) == x
        row = summarize(res)
        ok &= _check(f"n={n} k={k} units={units} last_column={last}", good and row["seq_count"] == 1 and row["seq_k"] == f"atmost{k}x1",
                     f"-> {[(tr['kind'], tr['n'], tr['bound']) for tr in res['trees']]} seq_k={row['seq_k']}")
        ok &= _check("  every clause covered", row["seq_cov"] == 1.0 and row["cov_total"] == 1.0, f"cov={row['seq_cov']}")
    # broken copies
    t, x, s = toy_seq(7, 3)
    cl = list(t.clauses)
    drop = [c for c in cl if len(c) == 3][4]
    cl2 = [c for c in cl if c != drop]
    res = analyse(cl2, t.n, want_pop=False)
    seq = [tr for tr in res["trees"] if tr["kind"] == "seq" and tr["n"] == 7]
    ok &= _check("dropped ternary: no n=7 counter", not seq, f"-> {[(tr['kind'], tr['n'], tr['bound']) for tr in res['trees']]}")
    # a wrong k: the overflow row points at register k-1 (at most k-1 in disguise? no: an inconsistent column)
    cl3 = []
    for c in cl:
        if len(c) == 2 and c[0] < 0 and c[1] < 0 and -c[1] == s[3][3]:
            cl3.append((c[0], -s[3][2]))
        else:
            cl3.append(c)
    res = analyse(cl3, t.n, want_pop=False)
    seq = [tr for tr in res["trees"] if tr["kind"] == "seq" and tr["n"] == 7 and tr["bound"] == ("atmost", 3)]
    ok &= _check("overflow row off by one: no n=7 k=3 counter", not seq, f"-> {[(tr['kind'], tr['n'], tr['bound']) for tr in res['trees']]}")
    # a register swapped between columns
    cl4 = [tuple(s[4][2] if l == s[5][2] else (-s[4][2] if l == -s[5][2] else l) for l in c) for c in cl]
    res = analyse(cl4, t.n, want_pop=False)
    seq = [tr for tr in res["trees"] if tr["kind"] == "seq" and tr["n"] == 7]
    ok &= _check("register swapped: no n=7 counter", not seq, f"-> {[(tr['kind'], tr['n'], tr['bound']) for tr in res['trees']]}")

    # (8) review round 7: a capped counter with a unit on a root register:
    # the unit bounds the six prefix inputs only; x_7 stays free (inputs 1, 2
    # and 7 true satisfy the formula), so no at-most-2 over seven
    t, x, s = toy_seq(7, 3)
    res = analyse(t.clauses + [(-s[6][3],)], t.n, want_pop=False)
    seq = [tr for tr in res["trees"] if tr["kind"] == "seq"]
    ok &= _check("(8) Sinz n=7 k=3 with unit -s_{6,3}: at most 2 of the six prefix inputs, x_7 not counted",
                 len(seq) == 1 and seq[0]["n"] == 6 and seq[0]["bound"] == ("atmost", 2) and sorted(seq[0]["leaves"]) == x[:6],
                 f"-> {[(tr['kind'], tr['n'], tr['bound'], tr['leaves']) for tr in res['trees']]}")

    # (9) review round 8: two four-input children that expose only their
    # first two registers and have no overflow clauses, merged into a root of
    # four registers with -r_4: one child's four inputs true and the other's
    # false satisfy every clause, so at most 3 of 8 must not be read; with
    # children exposing all four registers it is read
    for width, want in ((2, False), (4, True)):
        t = Toy()
        xs = t.new(8)

        def merge(A, B, c):
            R = t.new(c) if c > 1 else [t.new()]
            for i in range(0, len(A) + 1):
                for j in range(0, len(B) + 1):
                    if 1 <= i + j <= c:
                        t.add(*([R[i + j - 1]] + ([-A[i - 1]] if i else []) + ([-B[j - 1]] if j else [])))
            return R

        a = merge(merge([xs[0]], [xs[1]], 2), merge([xs[2]], [xs[3]], 2), width)
        b = merge(merge([xs[4]], [xs[5]], 2), merge([xs[6]], [xs[7]], 2), width)
        r = merge(a, b, 4)
        t.add(-r[3])
        res = analyse(t.clauses, t.n, want_pop=False)
        row = summarize(res)
        got = row["tot_nk"] == "8:atmost3x1"
        ok &= _check(f"(9) children counting to {width} of their 4 inputs, no overflow clauses, root -r_4: "
                     + ("at most 3 of 8" if want else "no bound (the children hide counts)"), got == want,
                     f"-> tot_nk={row['tot_nk']!r} partial={row['partial_count']}")

    print("totalizer")
    for n, k, arity, both, bounded in ((6, 2, 2, False, False), (9, 3, 3, False, False), (9, 2, 3, False, True), (8, 3, 2, True, False), (16, 4, 2, False, True)):
        t, x, R = toy_totalizer(n, k, leaf_arity=arity, both=both, bounded=bounded)
        res = analyse(t.clauses, t.n, want_pop=False)
        main = [tr for tr in res["trees"] if sorted(tr["leaves"]) == x]
        good = len(main) == 1 and main[0]["kind"] == "tot" and main[0]["n"] == n and main[0]["bound"] == ("atmost", k)
        if both:
            dual = [tr for tr in res["trees"] if sorted(tr["leaves"], key=abs) == [-v for v in x]]
            good = good and main[0]["dual"] and len(dual) == 1 and dual[0]["kind"] == "tot" and dual[0]["bound"][0] == "none" \
                and dual[0]["mirror"] and not main[0]["mirror"]
        row = summarize(res)
        ok &= _check(f"n={n} k={k} arity={arity} both={both} bounded={bounded}", good and row["tot_count"] == 1,
                     f"-> {[(tr['kind'], tr['n'], tr['bound'], tr['unresolved']) for tr in res['trees']]} tot_k={row['tot_k']}")
        ok &= _check("  every clause covered", row["tot_cov"] == 1.0 and row["cov_total"] == 1.0, f"cov={row['tot_cov']}")
    t, x, R = toy_totalizer(9, 3, leaf_arity=3)
    cl = list(t.clauses)
    drop = [c for c in cl if len(c) == 3][7]
    res = analyse([c for c in cl if c != drop], t.n, want_pop=False)
    main = [tr for tr in res["trees"] if sorted(tr["leaves"]) == x]
    ok &= _check("dropped ternary: no full tree", not main, f"-> {[(tr['kind'], tr['n'], tr['bound']) for tr in res['trees']]}")
    cl5 = [c for c in cl if not (len(c) == 1)] + [(-R[2],)]   # the unit moved to r_3: at most 2
    res = analyse(cl5, t.n, want_pop=False)
    main = [tr for tr in res["trees"] if sorted(tr["leaves"]) == x]
    ok &= _check("unit moved: k reads 2 not 3", len(main) == 1 and main[0]["bound"] == ("atmost", 2), f"-> {[(tr['kind'], tr['n'], tr['bound']) for tr in res['trees']]}")

    print("popcount")
    t, x, bits = toy_popcount(7, 3)
    res = analyse(t.clauses, t.n, want_pop=True)
    pops = res["pop"]
    good = len(pops) == 1 and pops[0]["fa"] == 4 and pops[0]["primary_leaves"] == 7 and pops[0]["use"].get("unit") == 3
    ok &= _check("7 inputs, 4 full adders, 3 sum bits under units", good,
                 f"-> {[{k: v for k, v in p.items() if k != 'cids'} for p in pops]} {res['pop_note']}")
    cl = list(t.clauses)
    # rewire: swap the carry of the first adder with an input in the last adder
    c1 = x[0]
    cl6 = [tuple((bits[2] if l == c1 else (-bits[2] if l == -c1 else l)) for l in c) for c in cl]
    res = analyse(cl6, t.n, want_pop=True)
    pops = res["pop"]
    ok &= _check("rewired: not the same network", not (len(pops) == 1 and pops[0]["fa"] == 4 and pops[0]["primary_leaves"] == 7),
                 f"-> {[{k: v for k, v in p.items() if k != 'cids'} for p in pops]}")

    print("decision diagram")
    t, x, node, tnode = toy_dd(6, 2)
    res = analyse(t.clauses, t.n, want_pop=False)
    cn = counted(res)
    row = summarize(res)
    good = len(cn["dd"]) == 1 and cn["dd"][0]["nodes"] == len(node) == 12 and cn["dd"][0]["levels"] == 6 and cn["dd"][0]["max_width"] == 3
    ok &= _check("at most 2 of 6: one counter diagram, 12 nodes, 6 levels, width 3", good,
                 f"-> {[{k: v for k, v in d.items() if k != 'cids'} for d in res['dd']['diagrams']]}")
    ok &= _check("  bound read by the walk from the asserted root: at most 2; every clause covered",
                 row["dd_bounds"] == "atmost2x1" and row["dd_cov"] == 1.0, f"-> bounds={row['dd_bounds']} cov={row['dd_cov']}")
    cl = list(t.clauses)
    # move one hi edge: node (1,1)'s hi goes to (2,0) instead of (2,2): counts inconsistent
    v = node[(1, 1)]
    cl7 = [((-v, -x[1], node[(2, 0)]) if c == (-v, -x[1], node[(2, 2)]) else c) for c in cl]
    res = analyse(cl7, t.n, want_pop=False)
    ok &= _check("one hi edge moved: not a counter diagram", not counted(res)["dd"],
                 f"-> {[(d['grid'], d['layered'], d['counter']) for d in res['dd']['diagrams']]}")
    # (6) review round 5: an early accepting branch: node (2,2)'s rejecting hi
    # edge sent to the true terminal; all six inputs true now satisfy the formula
    v = node[(2, 2)]
    cl8 = [((-v, -x[2], tnode) if c == (-v, -x[2]) else c) for c in cl]
    res = analyse(cl8, t.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(6) rejecting edge redirected to the true terminal: no at-most-2 read",
                 "atmost2" not in row["dd_bounds"] and "between" not in row["dd_bounds"], f"-> bounds={row['dd_bounds']} kind={row['dd_kind']}")
    # (7) review round 5: two diagrams with their own nodes over inputs that
    # share three variables are two counters, each at most 2
    t2 = Toy()
    xa = t2.new(6)
    xb = xa[:3] + t2.new(3)
    toy_dd(6, 2, t2, xa)
    toy_dd(6, 2, t2, xb)
    res = analyse(t2.clauses, t2.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(7) two diagrams sharing three inputs: two counters, each at most 2, every clause covered",
                 row["dd_count"] == 2 and row["dd_bounds"] == "atmost2x2" and row["dd_cov"] == 1.0,
                 f"-> count={row['dd_count']} bounds={row['dd_bounds']} cov={row['dd_cov']}")
    # (11) review round 10: an asserted diagram must not hide an independent
    # free one: two diagrams over their own inputs, only the first root unit
    # kept; both are found, one bounded and one free
    t3 = Toy()
    xa3, xb3 = t3.new(6), t3.new(6)
    _, _, node_a, _ = toy_dd(6, 2, t3, xa3)
    _, _, node_b, _ = toy_dd(6, 2, t3, xb3)
    root_b = node_b[(0, 0)] if isinstance(node_b, dict) else None
    clauses3 = [c for c in t3.clauses if not (root_b is not None and len(c) == 1 and c[0] == root_b)]
    res = analyse(clauses3, t3.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(11) an asserted diagram beside a free one: both found",
                 root_b is not None and len(clauses3) == len(t3.clauses) - 1 and row["dd_count"] == 2,
                 f"-> count={row['dd_count']} bounds={row['dd_bounds']} root_b={root_b}")
    # (12) review round 11: a tail restores a child's last register, so the
    # overflow pairs must be checked on the restored vectors: two three-input
    # children read through their first two registers by a three-register
    # root, only (-a_2 v -b_2) as overflow, a_3 -> r_3 and b_3 -> r_3; three
    # left inputs and one right input satisfy every clause
    t4 = Toy()
    xa4, xb4 = t4.new(3), t4.new(3)

    def merge_fwd4(A, B, cap=None):
        c = len(A) + len(B) if cap is None else cap
        R = t4.new(c)
        for i in range(len(A) + 1):
            for j in range(len(B) + 1):
                if 1 <= i + j <= c:
                    t4.add(*([R[i + j - 1]] + ([-A[i - 1]] if i else []) + ([-B[j - 1]] if j else [])))
        return R

    A4 = merge_fwd4(merge_fwd4([xa4[0]], [xa4[1]]), [xa4[2]])
    B4 = merge_fwd4(merge_fwd4([xb4[0]], [xb4[1]]), [xb4[2]])
    R4 = merge_fwd4(A4[:2], B4[:2], cap=3)
    t4.add(-A4[1], -B4[1])        # the one overflow clause
    t4.add(-A4[2], R4[2])         # a_3 -> r_3
    t4.add(-B4[2], R4[2])         # b_3 -> r_3
    res = analyse(t4.clauses, t4.n, want_pop=False)
    cn = counted(res)
    ok &= _check("(12) a capped root whose children's restored registers lack overflow clauses: no 'at most 3 of 6'",
                 not any(tr["n"] == 6 and tr["bound"] == ("atmost", 3) for tr in cn["tot"]),
                 f"-> counted {[(tr['n'], bound_text(tr)) for tr in cn['tot']]}")
    # (13) the verifier: a true bound is proven and kept, a false one injected
    # by hand is refuted and dropped
    t5, x5, R5 = toy_totalizer(6)
    cl5 = t5.clauses + [(-R5[3],)]          # at most 3 of 6
    res = analyse(cl5, t5.n, want_pop=False)
    main = [tr for tr in res["trees"] if sorted(tr["leaves"]) == x5]
    kept = len(main) == 1 and main[0]["bound"] == ("atmost", 3) and res["bounds_proven"] >= 1
    if main:
        main[0]["bound"] = ("atmost", 2)      # a misreading: the clauses allow 3
        main[0].pop("bound_dropped", None)
        proven, refuted, unknown = verify_bounds(res, cl5)
        dropped = main[0]["bound"][0] == "none" and main[0].get("bound_dropped") and refuted == 1
    else:
        dropped = False
    ok &= _check("(13) the verifier keeps a true bound and drops an injected false one", kept and dropped,
                 f"-> kept={kept} dropped={dropped} proven={res['bounds_proven']}")
    # (14) review round 12: a diagram whose rejecting edges go to a false
    # terminal written as a variable f with the unit -f keeps its bound
    t6, x6, node6, tnode6 = toy_dd(6, 2)
    nodevars6 = set(node6.values())
    f6 = t6.new()
    cl6 = []
    swapped = 0
    for c in t6.clauses:
        if len(c) == 2 and all(l < 0 for l in c) and sum(1 for l in c if -l in nodevars6) == 1 and sum(1 for l in c if -l in x6) == 1:
            cl6.append(tuple(c) + (f6,))
            swapped += 1
        else:
            cl6.append(c)
    cl6.append((-f6,))
    res = analyse(cl6, t6.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(14) rejecting edges to a false-terminal variable with a unit: the bound is kept",
                 swapped > 0 and row["dd_count"] == 1 and "atmost2" in row["dd_bounds"] and row["bounds_refuted"] == 0,
                 f"-> swapped={swapped} count={row['dd_count']} bounds={row['dd_bounds']} refuted={row['bounds_refuted']}")
    # (15) review round 12: a Sinz counter over three inputs is counted
    t7, x7, s7 = toy_seq(3, 2)
    res = analyse(t7.clauses, t7.n, want_pop=False)
    cn = counted(res)
    # (a single clause in effect, so not counted by the k >= 2 rule; the
    # check is that the recovered tree spans all three inputs)
    ok &= _check("(15) at most 2 of 3 as a Sinz counter: recovered over all three inputs (not counted: K = n - 1)",
                 any(tr["kind"] == "seq" and tr["n"] == 3 and tr["bound"] == ("atmost", 2) for tr in res["trees"])
                 and not cn["seq"] and not cn["tot"],
                 f"-> trees {[(tr['kind'], tr['n'], bound_text(tr)) for tr in res['trees']]}")
    # (16) the verifier's count encodings, checked exhaustively on small n:
    # each form is satisfiable with the literals fixed exactly when the count
    # allows it (a tiny brute-force propagation-free check by enumeration)
    enc_ok = True
    for n_ in range(1, 6):
        lits_ = list(range(1, n_ + 1))
        for b_ in range(0, n_ + 2):
            for kind_ in ("atleast", "atmost"):
                cl_, top_, _ = _count(lits_, kind_, b_, n_)
                for mask in range(1 << n_):
                    ones = bin(mask).count("1")
                    want_sat = ones >= b_ if kind_ == "atleast" else ones <= b_
                    fixed = [[l if (mask >> (l - 1)) & 1 else -l] for l in lits_]
                    sat = _brute_sat(cl_ + fixed, top_)
                    if sat != want_sat:
                        enc_ok = False
    ok &= _check("(16) the verifier's at-least / at-most encodings agree with the count on every assignment (n <= 5)", enc_ok, "")
    # (17) review round 13: a repeated ternary clause does not hide a
    # complete direct node
    t8, x8, R8 = toy_totalizer(9, 2, leaf_arity=3, bounded=True)
    first_tri = next(c for c in t8.clauses if len(c) == 3)
    res = analyse(t8.clauses + [first_tri], t8.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(17) a repeated ternary clause in a bounded 3-leaf totalizer: still 9:atmost2",
                 "9:atmost2" in row["tot_nk"], f"-> {row['tot_nk']}")
    # (18) review round 13: a pruned comparator network (only the clauses
    # wire 3 needs) is still at most 2 of 8
    t9, x9, wire9 = toy_network(2, n=8)
    target9 = wire9[2]
    cone9, keep9 = {target9}, []
    grew = True
    while grew:
        grew = False
        for c in t9.clauses:
            if tuple(c) in keep9:
                continue
            pos9 = [l for l in c if l > 0]
            if len(pos9) == 1 and pos9[0] in cone9 and all(l < 0 for l in c if l != pos9[0]):
                keep9.append(tuple(c))
                for l in c:
                    if l < 0 and -l not in cone9:
                        cone9.add(-l)
                        grew = True
    cl9 = keep9 + [tuple(c) for c in t9.clauses if len(c) == 1 and abs(c[0]) == target9]
    res = analyse(cl9, t9.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(18) a pruned comparator network: at most 2 of 8 found and proven",
                 row["net_count"] == 1 and row["net_k"] == "atmost2x1" and row["bounds_refuted"] == 0,
                 f"-> clauses {len(cl9)} of {len(t9.clauses)}; net_count={row['net_count']} net_k={row['net_k']}")
    # (19) review round 14: a compact Sinz counter (no register s_ij with
    # j > i, so the early columns need no overflow clause) is counted
    tA = Toy()
    xA = tA.new(7)
    kA = 3
    sA = {}
    for i in range(1, 7):
        for j in range(1, min(i, kA) + 1):
            sA[i, j] = tA.new()
    tA.add(-xA[0], sA[1, 1])
    for i in range(2, 7):
        tA.add(-xA[i - 1], sA[i, 1])
        tA.add(-sA[i - 1, 1], sA[i, 1])
        for j in range(2, min(i, kA) + 1):
            tA.add(-xA[i - 1], -sA[i - 1, j - 1], sA[i, j])
            if j <= i - 1:
                tA.add(-sA[i - 1, j], sA[i, j])
        if i - 1 >= kA:
            tA.add(-xA[i - 1], -sA[i - 1, kA])
    tA.add(-xA[6], -sA[6, kA])
    res = analyse(tA.clauses, tA.n, want_pop=False)
    cn = counted(res)
    ok &= _check("(19) a compact Sinz counter, at most 3 of 7: counted",
                 any(tr["n"] == 7 and tr["bound"] == ("atmost", 3) for tr in cn["seq"] + cn["tot"]),
                 f"-> seq {[(tr['n'], bound_text(tr)) for tr in cn['seq']]} tot {[(tr['n'], bound_text(tr)) for tr in cn['tot']]} partial {[tr['n'] for tr in cn['partial']]}")
    # (20) review round 15: a word adder is not a popcount (its inputs sit
    # at two weights): a + b over two bits with the result fixed to 2
    tB = Toy()
    a0, a1, b0, b1 = tB.new(4)

    def xor2B(p_, q_):
        o = tB.new()
        tB.add(-o, p_, q_); tB.add(-o, -p_, -q_); tB.add(o, -p_, q_); tB.add(o, p_, -q_)
        return o

    def and2B(p_, q_):
        o = tB.new()
        tB.add(-o, p_); tB.add(-o, q_); tB.add(o, -p_, -q_)
        return o

    def maj3B(p_, q_, r_):
        o = tB.new()
        for u, v in ((p_, q_), (p_, r_), (q_, r_)):
            tB.add(-o, u, v); tB.add(o, -u, -v)
        return o

    s0B, c0B = xor2B(a0, b0), and2B(a0, b0)
    s1B = xor2B(xor2B(a1, b1), c0B)
    c1B = maj3B(a1, b1, c0B)
    tB.add(-s0B); tB.add(s1B); tB.add(-c1B)     # a + b = 2
    res = analyse(tB.clauses, tB.n, want_pop=True)
    row = summarize(res)
    ok &= _check("(20) a two-bit word adder with its sum fixed: not a popcount", row["pop_nets"] == 0,
                 f"-> pop_nets={row['pop_nets']} pop_compared={row['pop_compared']}")
    # a real popcount still counts
    tP = toy_popcount(7, 3)
    tPt = tP[0] if isinstance(tP, tuple) else tP
    res = analyse(tPt.clauses, tPt.n, want_pop=True)
    row = summarize(res)
    ok &= _check("(20) a popcount of seven bits compared with 3: still a popcount", row["pop_nets"] >= 1,
                 f"-> pop_nets={row['pop_nets']} pop_compared={row['pop_compared']}")
    # (21) review round 16: two popcounts that share one input are two counts
    tC1, xC1, _ = toy_popcount(7, 3)
    tC2, xC2, _ = toy_popcount(7, 3)
    off = tC1.n - 1                      # the second copy's variables renumbered, its first input shared
    def ren(l):
        v = abs(l)
        nv = xC1[0] if v == xC2[0] else v + off
        return nv if l > 0 else -nv
    clC = list(tC1.clauses) + [tuple(ren(l) for l in c) for c in tC2.clauses]
    res = analyse(clC, tC1.n + tC2.n, want_pop=True)
    pops = [p for p in res["pop"] if p.get("equal_weight") and p["leaves"] >= 3]
    ok &= _check("(21) two popcounts sharing one input: two networks of 7 leaves, not one of 13",
                 len(pops) == 2 and all(p["leaves"] == 7 for p in pops), f"-> {[(p['leaves'], p['cells']) for p in res['pop']]}")
    # (22) review round 16: a bound at least the input count restricts nothing
    ok &= _check("(22) at most 3 of 3 and at least 0 restrict nothing; at most 2 of 3 does",
                 not restrictive(("atmost", 3), 3) and not restrictive(("atleast", 0), 5) and restrictive(("atmost", 2), 3)
                 and not restrictive(("between", 0, 4), 4) and restrictive(("between", 1, 4), 4), "")
    # (23) review round 17: a sum read by two adders is not a popcount:
    # s = a xor b feeds both s + c and s + d
    tD = Toy()
    aD, bD, cD, dD = tD.new(4)

    def xorD(p_, q_):
        o = tD.new()
        tD.add(-o, p_, q_); tD.add(-o, -p_, -q_); tD.add(o, -p_, q_); tD.add(o, p_, -q_)
        return o

    def andD(p_, q_):
        o = tD.new()
        tD.add(-o, p_); tD.add(-o, q_); tD.add(o, -p_, -q_)
        return o

    sD = xorD(aD, bD); kD = andD(aD, bD)
    s2D = xorD(sD, cD); k2D = andD(sD, cD)
    s3D = xorD(sD, dD); k3D = andD(sD, dD)
    for u in (s2D, k2D, s3D, k3D, kD):
        tD.add(-u)
    res = analyse(tD.clauses, tD.n, want_pop=True)
    row = summarize(res)
    ok &= _check("(23) a sum wire read by two adders: not a popcount", row["pop_nets"] == 0,
                 f"-> pop_nets={row['pop_nets']} {[(p['leaves'], p['cells'], p.get('equal_weight')) for p in res['pop']]}")
    # (24) final review: complementary inputs cancel; k >= 2 is the subject
    tF, xF, RF = toy_totalizer(6, 3)
    renF = {xF[3]: -xF[2], xF[4]: -xF[1], xF[5]: -xF[0]}
    clF = [tuple(renF.get(l, -renF[-l] if -l in renF else l) for l in c) for c in tF.clauses]
    res = analyse(clF, tF.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(24) at most 3 of six literals that are three complementary pairs: not counted", row["tot_count"] == 0,
                 f"-> tot_nk={row['tot_nk']}")
    tG, xG, RG = toy_totalizer(6, 1)
    res = analyse(tG.clauses, tG.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(24) at most 1 of 6 (at-most-one) is not a k >= 2 count", row["tot_count"] == 0, f"-> tot_nk={row['tot_nk']}")
    ok &= _check("(24) the k >= 2 rule: at most 2 of 6 counts, at most 5 of 6 and at least 5 of 6 do not",
                 counts_k2(("atmost", 2), 6) and not counts_k2(("atmost", 5), 6) and not counts_k2(("atleast", 5), 6)
                 and counts_k2(("atleast", 4), 6), "")
    # (25) review round 22: a redundant looser output unit must not hide the
    # tight bound: at most 2 of 4 (unit -out_3) plus a unit -out_4
    tH, xH, wH = toy_network(2, n=4, two_way=True)
    res = analyse(tH.clauses + [(-wH[3],)], tH.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(25) at most 2 of 4 with a redundant unit on the fourth output: still at most 2",
                 row["net_count"] == 1 and "atmost2" in row["net_k"], f"-> net_count={row['net_count']} net_k={row['net_k']}")
    # (26) review round 23: a tree that reads inputs twice is a weighted sum:
    # inputs 4-6 of a six-input totalizer wired to inputs 1-3
    tI, xI, RI = toy_totalizer(6, 3)
    renI = {xI[3]: xI[0], xI[4]: xI[1], xI[5]: xI[2]}
    clI = [tuple((renI.get(abs(l), abs(l)) if l > 0 else -renI.get(abs(l), abs(l))) for l in c) for c in tI.clauses]
    res = analyse(clI, tI.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(26) a totalizer reading three inputs twice: not a count", row["tot_count"] == 0, f"-> tot_nk={row['tot_nk']}")
    # (27) a partly sorted network: the last comparator removed, the unit
    # kept; the at most 2 still holds and is proven
    tJ, xJ, wJ = toy_network(2, n=8)
    clJ = tJ.clauses[:-4] + tJ.clauses[-1:]
    res = analyse(clJ, tJ.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(27) a network missing its last comparator: the proven at most 2 still counts",
                 row["net_count"] == 1 and "atmost2" in row["net_k"] and row["bounds_refuted"] == 0,
                 f"-> net_count={row['net_count']} net_k={row['net_k']} proven={row['bounds_proven']}")
    # (28) the proof instance is renumbered densely: a six-input totalizer
    # with every variable shifted by 300,000,000
    SH = 300_000_000
    clK = [tuple(l + SH if l > 0 else l - SH for l in c) for c in tI.clauses]
    topK, cnfK = _check_instance(clK, range(len(clK)), [v + SH for v in xI], "atleast", 4)
    ok &= _check("(28) the proof instance of a shifted structure has a small header", topK < 200, f"-> p cnf {topK}")
    print("comparator network")
    for k in (2, 3, 5):
        t, x, wire = toy_network(k)
        res = analyse(t.clauses, t.n, want_pop=False)
        row = summarize(res)
        cn = counted(res)
        grp = [g for d in cn["nets"] for g in d["groups"] if sorted(g["inputs"]) == x]
        good = row["net_count"] == 1 and len(grp) == 1 and grp[0]["bound"] == ("atmost", k) and \
            [grp[0]["thresholds"][w] for w in grp[0]["ranked"]] == list(range(1, 9))
        ok &= _check(f"Batcher 8, at most {k}: one network, outputs ranked 1..8, bound read", good,
                     f"-> net_k={row['net_k']} net_n={row['net_n']} cov={row['net_cov']}")
        ok &= _check("  every clause covered", row["net_cov"] == 1.0 and row["cov_total"] == 1.0, f"cov={row['net_cov']}")
    t, x, wire = toy_network(3)
    cl = list(t.clauses)
    # one comparator's min clause dropped: that comparator is no longer one, the outputs are not ranked over all 8
    drop = [c for c in cl if len(c) == 3][9]
    res = analyse([c for c in cl if c != drop], t.n, want_pop=False)
    cn = counted(res)
    grp = [g for d in cn["nets"] for g in d["groups"] if sorted(g["inputs"]) == x and g["bound"] == ("atmost", 3)]
    ok &= _check("dropped comparator clause: no ranked 8-input network at most 3", not grp,
                 f"-> {summarize(res)['net_k']}")
    # a comparator rewired: its min goes to the max side, the sort is wrong
    i, j = 10, 11
    cl8 = list(cl)
    tri = [c for c in cl if len(c) == 3]
    a = tri[i]
    b = tri[j]
    cl8[cl8.index(a)] = (a[0], a[1], b[2])
    cl8[cl8.index(b)] = (b[0], b[1], a[2])
    res = analyse(cl8, t.n, want_pop=False)
    cn = counted(res)
    grp = [g for d in cn["nets"] for g in d["groups"] if sorted(g["inputs"]) == x and g["bound"] == ("atmost", 3)]
    ok &= _check("two comparators' outputs crossed: no ranked 8-input network at most 3", not grp,
                 f"-> {summarize(res)['net_k']}")

    print("review findings (2026-10-19)")
    # (1) two capped 2-wide merge nodes that read each other's registers, and
    # a root reading one of them: a valid CNF; the walk used to loop forever
    p_, q_, r1, r2, s1, s2, z, t1, t2, t3 = range(1, 11)
    cyc = [(-s1, r1), (-s2, r2), (-p_, r1), (-s1, -p_, r2), (-s2, -p_),
           (-r1, s1), (-r2, s2), (-q_, s1), (-r1, -q_, s2), (-r2, -q_),
           (-r1, t1), (-r2, t2), (-z, t1), (-r1, -z, t2), (-r2, -z, t3)]
    t0 = time.time()
    res = analyse(cyc, 10, want_pop=False)
    cn = counted(res)
    ok &= _check("(1) merge nodes in a cycle: finishes, the component is not counted",
                 time.time() - t0 < 5 and not cn["seq"] and not cn["tot"] and any(t["cyclic"] for t in res["trees"]),
                 f"-> {[(t['kind'], t['n'], t['bound'], t['cyclic']) for t in res['trees']]}")
    # (2) a positive unit on a forward-only output is no lower bound
    t, x, R = toy_totalizer(6)
    res = analyse(t.clauses + [(R[2],)], t.n, want_pop=False)
    main = [tr for tr in res["trees"] if sorted(tr["leaves"]) == x]
    ok &= _check("(2) forward-only totalizer with unit r_3: no lower bound read",
                 len(main) == 1 and main[0]["bound"][0] == "none" and summarize(res)["tot_count"] == 0,
                 f"-> {[(tr['kind'], tr['n'], tr['bound']) for tr in res['trees']]}")
    t, x, R = toy_totalizer(6, both=True)
    res = analyse(t.clauses + [(R[2],)], t.n, want_pop=False)
    cn = counted(res)
    ok &= _check("(2) two-way totalizer with unit r_3: at least 3, read once (at most 3 of the negations)",
                 len(cn["tot"]) == 1 and sorted(cn["tot"][0]["leaves"], key=abs) == [-v for v in x] and cn["tot"][0]["bound"] == ("atmost", 3)
                 and len(cn["mirror"]) == 1 and sorted(cn["mirror"][0]["leaves"]) == x and summarize(res)["tot_cov"] == 1.0,
                 f"-> counted {[(tr['n'], bound_text(tr)) for tr in cn['tot']]} mirror {[(tr['n'], bound_text(tr)) for tr in cn['mirror']]}")
    # (10) round 9: a child's omitted register may stand in for the parent's
    # top register only at the same index; here a_3 (index 3) is linked to
    # r_5 and r_4 is asserted false, and three left inputs plus one right
    # input still satisfy every clause, so "at most 3 of 6" must not be read
    t = Toy()
    xa, xb = t.new(3), t.new(3)

    def merge_fwd(A, B):
        c = len(A) + len(B)
        R = t.new(c)
        for i in range(len(A) + 1):
            for j in range(len(B) + 1):
                if 1 <= i + j <= c:
                    t.add(*([R[i + j - 1]] + ([-A[i - 1]] if i else []) + ([-B[j - 1]] if j else [])))
        return R

    A = merge_fwd(merge_fwd([xa[0]], [xa[1]]), [xa[2]])
    B = merge_fwd(merge_fwd([xb[0]], [xb[1]]), [xb[2]])
    R = merge_fwd(A[:2], B)      # the parent reads only a_1, a_2 of the left child
    t.add(-A[2], R[4])            # a_3 -> r_5
    t.add(-R[3])                  # not r_4
    res = analyse(t.clauses, t.n, want_pop=False)
    cn = counted(res)
    ok &= _check("(10) a_3 linked to the parent's r_5 (index 3 against 5): no 'at most 3 of 6' read",
                 not any(tr["n"] == 6 and tr["bound"] == ("atmost", 3) for tr in cn["tot"]),
                 f"-> counted {[(tr['n'], bound_text(tr)) for tr in cn['tot']]}")
    # (4) round 2: a partial reverse encoding is no lower bound: the three
    # clauses with -r_3 removed, r_3 asserted; every input false with only r_3
    # true satisfies every clause
    t, x, R = toy_totalizer(6, both=True)
    res = analyse([c for c in t.clauses if -R[2] not in c] + [(R[2],)], t.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(4) two-way totalizer, r_3's reverse clauses removed, unit r_3: no bound read",
                 row["tot_count"] == 0 and row["seq_count"] == 0 and row["net_count"] == 0,
                 f"-> tot_k={row['tot_k']} free={row['tot_free']} {[(tr['kind'], tr['n'], bound_text(tr)) for tr in res['trees']]}")
    t, x, wire = toy_network(0, unit=lambda w: w[2])
    res = analyse(t.clauses, t.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(2) forward-only network with unit out_3: no lower bound read", row["net_count"] == 0 and row["net_free"] == 1,
                 f"-> count={row['net_count']} free={row['net_free']}")
    t, x, wire = toy_network(0, two_way=True, unit=lambda w: w[2])
    res = analyse(t.clauses, t.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(2) two-way network with unit out_3: at least 3, read once (at most 5 of the negations)",
                 row["net_k"] == "atmost5(neg)x1" and row["net_count"] == 1 and row["net_cov"] == 1.0,
                 f"-> {row['net_k']} cov={row['net_cov']}")
    # (3) a 64-input network: the outputs above 40 used to be ranked n
    t, x, wire = toy_network(41, n=64)
    res = analyse(t.clauses, t.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(3) Batcher 64 capped at 41: at most 41", row["net_k"] == "atmost41x1" and row["net_n"] == "64x1",
                 f"-> {row['net_k']} {row['net_n']}")

    print("binomial")
    for t_size, m, sign, guard, want in ((5, 3, -1, False, ("atmost", 2)), (4, 3, 1, True, ("atleast", 2)),
                                         (6, 4, -1, True, ("atmost", 3)), (7, 3, 1, False, ("atleast", 5))):
        t, x, g = toy_binomial(t_size, m, sign, guard)
        res = analyse(t.clauses, t.n, want_pop=False)
        fams = res["binom"]
        S = sorted(x if sign > 0 else [-v for v in x], key=abs)
        good = len(fams) == 1 and fams[0]["form"] == want and fams[0]["t"] == t_size and fams[0]["S"] == S and fams[0]["guard"] == (g or 0)
        row = summarize(res)
        ok &= _check(f"{m}-subsets of {t_size}, sign {sign:+d}, guard {guard}: {want[0]} {want[1]}", good and row["binom_cov"] == 1.0,
                     f"-> {[(f['form'], f['t'], f['guard']) for f in fams]} cov={row['binom_cov']}")
    t, x, g = toy_binomial(5, 3, -1)
    cl = list(t.clauses)
    res = analyse(cl[:-1], t.n, want_pop=False)
    ok &= _check("one subset missing: no family over the 5", not [f for f in res["binom"] if f["t"] == 5],
                 f"-> {[(f['form'], f['t']) for f in res['binom']]}")
    t, x, g = toy_binomial(4, 2, -1)
    res = analyse(t.clauses, t.n, want_pop=False)
    ok &= _check("pairwise at-most-one (k = 1) is not counted", not res["binom"], f"-> {res['binom']}")
    # (5) review round 4: the guarded views of an unguarded family (any literal
    # of an all-negative clause read as a guard) are the same constraint
    t, x, g = toy_binomial(6, 4, -1)
    res = analyse(t.clauses, t.n, want_pop=False)
    row = summarize(res)
    ok &= _check("(5) at most 3 of 6 is one family, not seven (its guarded views dropped)",
                 row["binom_count"] == 1 and row["binom_desc"] == "atmost3of6x1" and row["binom_cov"] == 1.0,
                 f"-> {row['binom_count']} {row['binom_desc']}")
    print("ALL OK" if ok else "FAILURES")
    return 0 if ok else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--out", required=True)
    c.add_argument("--cells", help="file with one stem per line (default: the whole 2025 suite)")
    c.add_argument("--stems", help="comma-separated substrings of stems to analyse")
    c.add_argument("--files", nargs="*", help="explicit .cnf.xz files")
    c.add_argument("--workers", type=int, default=4)
    c.add_argument("--max-clauses", type=int, default=MAX_CLAUSES)
    c.add_argument("--no-pop", action="store_true", help="skip the adder-tree detector")
    c.add_argument("--order-from", help="comma-separated TSVs with stem and detect_s: run the slowest cells first")
    c.set_defaults(func=cmd_census)
    o = sub.add_parser("one")
    o.add_argument("file")
    o.add_argument("--max-clauses", type=int, default=MAX_CLAUSES)
    o.add_argument("--no-pop", action="store_true")
    o.add_argument("--show", type=int, default=10)
    o.add_argument("--uncovered", action="store_true", help="print what the detectors left and the partial trees")
    o.set_defaults(func=cmd_one)
    r = sub.add_parser("report")
    r.add_argument("table")
    r.set_defaults(func=cmd_report)
    s = sub.add_parser("selftest")
    s.set_defaults(func=cmd_selftest)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
