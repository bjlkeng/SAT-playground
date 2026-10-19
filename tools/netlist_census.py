#!/usr/bin/env python3
"""Netlist recovery census (bead SAT-playground-skz.1).

What comes back from a CNF when it is read as a circuit, per cell:

  gates      Tseitin definitions: a variable that its clauses determine
             as a function of a few others (the resolvent test, done as a
             truth table over the support).  AND, XOR, ITE, majority and
             any other small cell count alike, so technology-mapped
             netlists come back too.
  merged     structural hashing (congruence closure): gates with the same
             function over the same inputs are one signal.
  adders     half adders (XOR2 with AND2 on the same two leaves) and full
             adders (XOR3 with majority on the same three leaves), found
             by truth tables over 2- and 3-input cuts, so an adder built
             from ANDs is found as well as one written with XOR clauses.
  weights    the adder cells chain into a weight for every signal (a sum
             keeps the weight, a carry doubles it); reported consistent or
             not per adder network.
  operands   the leaves of an adder network that are AND gates of two
             primary inputs form a grid a_i * b_j; its two sides are the
             operands of a multiplier.  Leaves that are not such ANDs are
             counted as nonstandard partial products (Booth).
  miter      the clauses no gate definition uses are the top constraints;
             two adder networks over the same primary inputs are the two
             cones of a miter; the leaves they share are counted.
  eqcands    random simulation of the recovered netlist: signals with the
             same (or complementary) signature on 64 random inputs that
             structural hashing did not merge.

Usage:
  python3 tools/netlist_census.py census --out TABLE.tsv [--cells FILE] [--stems a,b,c]
  python3 tools/netlist_census.py report TABLE.tsv

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import os
import random
import sys
import time
from collections import defaultdict
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rl_structure import parse_prefix  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
CELLS = ROOT / "benchmarks" / "rl" / "structure_2025.tsv"

MAX_OCC = 2048      # a variable with more occurrences is not tried as a gate output (control
                    # signals fan out to hundreds of clauses; 64 lost the Booth encoders)
MAX_STARTS = 32     # distinct start clauses tried per variable
COMPARE_OVERLAP = 0.75  # two cones sharing less than this (of the smaller) are two sides of a miter
MIN_COMPARE_CONE = 16   # a compare's two inputs each have a cone at least this large
MAX_SUPPORT = 6     # largest cell support (truth table of 64 entries)
MAX_GATE_VARS = 400_000  # cells with more variables are reported as too-big
MAX_FULL_GATES = 250_000 # the adder, weight and simulation analysis runs up to this many gates

# ---------------------------------------------------------------------------
# gate recovery
# ---------------------------------------------------------------------------


def _definition_tt(v, clause_ids, clauses, support):
    """Truth table of v over `support` (a tuple of variables) if the given
    clauses determine v for every assignment of the support, else None.
    A clause's other literals must all lie in the support."""
    idx = {u: i for i, u in enumerate(support)}
    pos, neg = [], []
    for ci in clause_ids:
        c = clauses[ci]
        others = []
        sign = 0
        for l in c:
            if abs(l) == v:
                sign = 1 if l > 0 else -1
            else:
                others.append((idx[abs(l)], l > 0))
        (pos if sign > 0 else neg).append(others)
    n = len(support)
    tt = 0
    for m in range(1 << n):
        # a clause is violated when every other literal is false under m
        pv = any(all(((m >> i) & 1) != val for i, val in oth) for oth in pos)
        nv = any(all(((m >> i) & 1) != val for i, val in oth) for oth in neg)
        if pv and nv:
            return None  # the clauses forbid this assignment: a constraint, not a gate
        if not pv and not nv:
            return None  # v is free here
        if pv:
            tt |= 1 << m
    return tt


def recover_gates(clauses, n_vars):
    """{output var: (support tuple, truth table)} plus the set of clause
    ids used by the accepted definitions.  Parity clause sets define every
    variable in them; the one with the largest index is taken as the
    output (Tseitin encoders number outputs after inputs), and cycles that
    survive are broken afterwards."""
    occ = defaultdict(list)
    units = set()
    for i, c in enumerate(clauses):
        for l in c:
            occ[abs(l)].append(i)
        if len(c) == 1:
            units.add(abs(c[0]))
    # a variable fixed by a unit clause can be a gate's output but never part
    # of a support: the clauses around it are constraints (a miter's compare
    # with its unit would otherwise define the compared outputs)
    cands = defaultdict(list)  # v -> [(support, tt, frozenset(clause ids))], every definition found
    for v in range(1, n_vars + 1):
        ids = occ.get(v)
        if not ids or len(ids) > MAX_OCC:
            continue
        short = [ci for ci in ids if 2 <= len(clauses[ci]) <= MAX_SUPPORT + 1]  # units are constraints
        if not short:
            continue
        tried = set()
        starts = 0
        for start in sorted(short, key=lambda ci: len(clauses[ci])):
            support = frozenset(abs(l) for l in clauses[start] if abs(l) != v)
            if not support or support in tried or support & units:
                continue
            starts += 1
            if starts > MAX_STARTS:
                break
            for _grow in range(4):
                tried.add(support)
                sel = [ci for ci in short if all(abs(l) == v or abs(l) in support for l in clauses[ci])]
                tt = _definition_tt(v, sel, clauses, tuple(sorted(support)))
                if tt is not None:
                    key = frozenset(sel)
                    if all(d[2] != key for d in cands[v]):
                        cands[v].append((tuple(sorted(support)), tt, key))
                    break
                # grow by the clause that adds the fewest new variables
                best = None
                for ci in short:
                    oth = frozenset(abs(l) for l in clauses[ci] if abs(l) != v)
                    new = oth - support
                    if not new or oth & units or not (oth & support) and len(support) > 1:
                        continue
                    if len(support | oth) > MAX_SUPPORT:
                        continue
                    if best is None or len(new) < len(best - support):
                        best = oth
                if best is None:
                    break
                support = support | best
    # Settle the netlist forward.  A candidate is accepted once every
    # variable of its support is settled (a primary input or an accepted
    # gate) and none of its clauses belongs to an accepted gate already.
    # That fixes the direction of parity clause sets (XOR clauses define
    # each of their variables) and kills definitions assembled from halves
    # of two real gates (which would form cycles).  A deadlock means a
    # primary input: the unsettled variable that the most waiting
    # candidates read is declared one.  Acyclic by construction.
    settled = set()
    waiting_on = defaultdict(list)   # var -> candidate ids waiting for it
    cand_list = []                   # (v, support, tt, clause set)
    remaining = []                   # unsettled support count per candidate
    for v, lst in cands.items():
        for sup, tt, cl in lst:
            cand_list.append((v, sup, tt, cl))
            remaining.append(0)
    present = set(occ)
    with_cands = set(cands)
    sets_of = defaultdict(list)
    for v, lst in cands.items():
        for sup, tt, cl in lst:
            sets_of[cl].append(v)
    for v in present:
        if v not in with_cands:
            settled.add(v)           # no definition found: a primary input
    for ci, (v, sup, tt, cl) in enumerate(cand_list):
        for u in sup:
            if u not in settled:
                waiting_on[u].append(ci)
                remaining[ci] += 1
    gates = {}
    used = set()
    ready = [ci for ci in range(len(cand_list)) if remaining[ci] == 0]
    deadlocks = 0
    unsettled_defined = set(with_cands)
    # Is the file numbered in circuit order (outputs after inputs)?  Judged
    # on the clause sets that define a single variable (AND, majority, ITE),
    # which have no direction ambiguity.  If so, a deadlock is broken by
    # index: the smaller variable is the input.  Otherwise by fan-out.
    single = [(v, sup) for v, lst in cands.items() for sup, tt, cl in lst if len(sets_of[cl]) == 1]
    ordered = sum(1 for v, sup in single if all(u < v for u in sup))
    topo_numbered = len(single) >= 20 and ordered >= 0.95 * len(single)

    def accept(ci):
        v, sup, tt, cl = cand_list[ci]
        if v in settled or (cl & used):
            return False
        gates[v] = (sup, tt, cl)
        used.update(cl)
        return True

    def settle(v):
        settled.add(v)
        unsettled_defined.discard(v)
        for ci in waiting_on.pop(v, ()):
            remaining[ci] -= 1
            if remaining[ci] == 0:
                ready.append(ci)

    while unsettled_defined:
        progressed = False
        while ready:
            ci = ready.pop()
            if accept(ci):
                settle(cand_list[ci][0])
                progressed = True
        if not unsettled_defined:
            break
        if not progressed:
            # deadlock: the unsettled variable most waited for becomes a primary input
            if topo_numbered:
                best = min(unsettled_defined)
            else:
                best = max(unsettled_defined, key=lambda u: (len(waiting_on.get(u, ())), len(occ[u]), -u))
            deadlocks += 1
            settle(best)
    dbg = os.environ.get("NETLIST_DEBUG_VAR")
    if dbg:
        dv = int(dbg)
        print(f"cands[{dv}] supports: {[d[0] for d in cands.get(dv, [])]}; gate: {gates.get(dv, (None,))[0]}", file=sys.stderr)
    return gates, used, deadlocks, topo_numbered


def topo_order(gates):
    indeg = {v: sum(1 for u in sup if u in gates) for v, (sup, tt, cl) in gates.items()}
    fan = defaultdict(list)
    for v, (sup, tt, cl) in gates.items():
        for u in sup:
            if u in gates:
                fan[u].append(v)
    order = [v for v, d in indeg.items() if d == 0]
    i = 0
    while i < len(order):
        u = order[i]
        i += 1
        for w in fan[u]:
            indeg[w] -= 1
            if indeg[w] == 0:
                order.append(w)
    return order


# ---------------------------------------------------------------------------
# structural hashing (congruence closure)
# ---------------------------------------------------------------------------


def _fold_negations(tt, n, negmask):
    """Truth table over inputs whose polarities in negmask are flipped."""
    if not negmask:
        return tt
    out = 0
    for m in range(1 << n):
        if (tt >> (m ^ negmask)) & 1:
            out |= 1 << m
    return out


def structural_hash(gates, order):
    """rep[v] = (representative var, negated) for every gate output, with
    equal functions over equal inputs merged; primary inputs represent
    themselves."""
    rep = {}
    key_to_rep = {}
    merged = 0
    for v in order:
        sup, tt, cl = gates[v]
        n = len(sup)
        reps = [rep.get(u, (u, False)) for u in sup]
        negmask = 0
        for i, (r, ng) in enumerate(reps):
            if ng:
                negmask |= 1 << i
        tt2 = _fold_negations(tt, n, negmask)
        # sort the support by representative; permute the truth table
        perm = sorted(range(n), key=lambda i: reps[i][0])
        if perm != list(range(n)):
            tt3 = 0
            for m in range(1 << n):
                m2 = 0
                for new_i, old_i in enumerate(perm):
                    if (m >> old_i) & 1:
                        m2 |= 1 << new_i
                if (tt2 >> m) & 1:
                    tt3 |= 1 << m2
            tt2 = tt3
        rsup = tuple(reps[i][0] for i in perm)
        neg = bool(tt2 & 1)
        if neg:
            tt2 = (~tt2) & ((1 << (1 << n)) - 1)
        if n == 1 and tt2 == 0b10:  # buffer / inverter: alias the input
            rep[v] = (rsup[0], neg)
            merged += 1
            continue
        key = (rsup, tt2)
        if key in key_to_rep:
            r, rneg = key_to_rep[key]
            rep[v] = (r, rneg != neg)
            merged += 1
        else:
            key_to_rep[key] = (v, neg)
            rep[v] = (v, False)
    return rep, merged


# ---------------------------------------------------------------------------
# bit-parallel simulation of the merged netlist
# ---------------------------------------------------------------------------


def eval_gate(tt, inputs, width_mask):
    """Bit-parallel value of a gate from its truth table and the words of its inputs."""
    n = len(inputs)
    out = 0
    for m in range(1 << n):
        if (tt >> m) & 1:
            term = width_mask
            for i in range(n):
                term &= inputs[i] if (m >> i) & 1 else (~inputs[i] & width_mask)
            out |= term
    return out


def simulate(gates, order, rep, words, width_mask):
    """words: dict var -> word for the primary inputs; fills every gate output."""
    val = dict(words)
    for v in order:
        r, neg = rep[v]
        if r != v:
            continue  # merged: value comes from the representative
        sup, tt, cl = gates[v]
        ins = []
        for u in sup:
            ru, nu = rep.get(u, (u, False))
            w = val[ru]
            ins.append((~w & width_mask) if nu else w)
        val[v] = eval_gate(tt, ins, width_mask)
    for v in order:
        r, neg = rep[v]
        if r != v:
            val[v] = (~val[r] & width_mask) if neg else val[r]
    return val


# ---------------------------------------------------------------------------
# adder cells by cuts
# ---------------------------------------------------------------------------

XOR2, XNOR2 = 0b0110, 0b1001
AND_FAMILY2 = {0b1000: (0, 0), 0b0100: (1, 0), 0b0010: (0, 1), 0b0001: (1, 1)}  # tt -> (neg a, neg b)
OR_FAMILY2 = {0b1110: (0, 0), 0b1101: (1, 0), 0b1011: (0, 1), 0b0111: (1, 1)}   # a half adder with carry-in 1
XOR3, XNOR3 = 0x96, 0x69


def _maj_tts():
    out = {}
    for negmask in range(8):
        tt = 0
        for m in range(8):
            x = m ^ negmask
            if bin(x).count("1") >= 2:
                tt |= 1 << m
        out[tt] = negmask
    return out


MAJ_FAMILY3 = _maj_tts()  # tt -> polarity mask of the three leaves


def enumerate_cuts(gates, order, rep, k, max_cuts=16):
    """k-feasible cuts per representative node, as frozensets of leaf vars."""
    cuts = {}

    def node_cuts(u):
        return cuts.get(u, [frozenset([u])])

    for v in order:
        r, neg = rep[v]
        if r != v:
            continue
        sup, tt, cl = gates[v]
        lists = [node_cuts(rep.get(u, (u, False))[0]) for u in sup]
        acc = {frozenset([v])}
        combos = [frozenset()]
        for lst in lists:
            nxt = []
            for c in combos:
                for d in lst:
                    u = c | d
                    if len(u) <= k:
                        nxt.append(u)
            # keep the smallest
            nxt.sort(key=len)
            combos = nxt[: max_cuts * 4]
        for c in combos:
            if 1 <= len(c) <= k:
                acc.add(c)
        lst = sorted(acc, key=len)[:max_cuts]
        cuts[v] = lst
    return cuts


def cut_truth_table(v, cut, gates, rep, cone_cache):
    """Truth table of representative node v over the leaves of `cut` (sorted)."""
    leaves = tuple(sorted(cut))
    n = len(leaves)
    width = 1 << n
    mask = (1 << width) - 1
    words = {}
    for i, leaf in enumerate(leaves):
        w = 0
        for m in range(width):
            if (m >> i) & 1:
                w |= 1 << m
        words[leaf] = w
    # evaluate the cone of v down to the leaves, iteratively (a 3-leaf cut
    # can still hold a chain of a thousand gates)
    memo = dict(words)
    stack = [v]
    while stack:
        u = stack[-1]
        if u in memo:
            stack.pop()
            continue
        r, neg = rep.get(u, (u, False))
        if r != u:
            if r in memo:
                memo[u] = (~memo[r] & mask) if neg else memo[r]
                stack.pop()
            else:
                stack.append(r)
            continue
        if u not in gates:
            return None  # a primary input that is not a leaf: the cut is not a cut
        sup, tt, cl = gates[u]
        pending = [rep.get(x, (x, False))[0] for x in sup if rep.get(x, (x, False))[0] not in memo]
        if pending:
            stack.extend(pending)
            continue
        ins = []
        for x in sup:
            rx, nx = rep.get(x, (x, False))
            w = memo[rx]
            ins.append((~w & mask) if nx else w)
        memo[u] = eval_gate(tt, ins, mask)
        stack.pop()
    return memo[v] & mask


def find_adders(gates, order, rep):
    """Full and half adders over the merged netlist.
    Returns a list of cells: (kind, leaves (3 or 2 vars), leaf_negmask, sum var,
    sum_neg, carry var, carry_neg) meaning
        sum of the leaf literals = sum_literal + 2 * carry_literal."""
    cuts3 = enumerate_cuts(gates, order, rep, 3)
    by_cut = defaultdict(list)  # cut -> [(node, tt)]
    for v, lst in cuts3.items():
        for c in lst:
            if len(c) in (2, 3) and v not in c:
                tt = cut_truth_table(v, c, gates, rep, None)
                if tt is not None:
                    by_cut[c].append((v, tt))
    cells = []
    used = set()
    # full adders first (3 leaves), then half adders
    for c, nodes in by_cut.items():
        if len(c) != 3:
            continue
        sums = [(v, tt) for v, tt in nodes if tt in (XOR3, XNOR3) and v not in used]
        carries = [(v, tt) for v, tt in nodes if tt in MAJ_FAMILY3 and v not in used]
        if not sums or not carries:
            continue
        leaves = tuple(sorted(c))
        for (s, stt), (cv, ctt) in zip(sums, carries):
            negmask = MAJ_FAMILY3[ctt]
            # XOR of the leaf literals: flipping k leaves flips the parity k times
            flips = bin(negmask).count("1") & 1
            sum_neg = (stt == XNOR3) != bool(flips)
            cells.append(("FA", leaves, negmask, s, sum_neg, cv, False))
            used.add(s)
            used.add(cv)
    # a full adder built from 2-input gates holds two half-adder shapes
    # (xor and and of the same two signals) inside it: the gates inside an
    # accepted full adder's cone are taken, so cells do not overlap
    for kind, leaves, negmask, s, sneg, cv, cneg in cells:
        stop = set(leaves)
        stack = [s, cv]
        while stack:
            u = stack.pop()
            r, ng = rep.get(u, (u, False))
            if r in stop or r in used and r not in (s, cv) or r not in gates:
                continue
            used.add(r)
            stack.extend(gates[r][0])
    for c, nodes in by_cut.items():
        if len(c) != 2:
            continue
        sums = [(v, tt) for v, tt in nodes if tt in (XOR2, XNOR2) and v not in used]
        carries = [(v, tt) for v, tt in nodes if tt in AND_FAMILY2 and v not in used]
        carries1 = [(v, tt) for v, tt in nodes if tt in OR_FAMILY2 and v not in used]
        leaves = tuple(sorted(c))
        # a half adder: la + lb = s + 2 c with c = la and lb, s = la xor lb
        for (s, stt), (cv, ctt) in zip(sums, carries):
            na, nb = AND_FAMILY2[ctt]
            negmask = (1 if na else 0) | (2 if nb else 0)
            flips = (na + nb) & 1
            sum_neg = (stt == XNOR2) != bool(flips)
            cells.append(("HA", leaves, negmask, s, sum_neg, cv, False))
            used.add(s)
            used.add(cv)
        # a half adder with a constant one folded in (Booth sign extension):
        # la + lb + 1 = s + 2 c with c = la or lb, s = not (la xor lb)
        sums = [(v, tt) for v, tt in sums if v not in used]
        for (s, stt), (cv, ctt) in zip(sums, carries1):
            na, nb = OR_FAMILY2[ctt]
            negmask = (1 if na else 0) | (2 if nb else 0)
            flips = (na + nb) & 1
            k = (stt == XNOR2) != bool(flips)   # s = (la xor lb) xor k
            sum_neg = not k                      # the sum literal is not (la xor lb)
            cells.append(("HA1", leaves, negmask, s, sum_neg, cv, False))
            used.add(s)
            used.add(cv)
    return cells


# ---------------------------------------------------------------------------
# weights, networks, operands
# ---------------------------------------------------------------------------


class WeightedUF:
    """Union-find with an integer offset: exp(x) - exp(root(x))."""

    def __init__(self):
        self.parent = {}
        self.off = {}
        self.conflicts = 0

    def find(self, x):
        if x not in self.parent:
            self.parent[x] = x
            self.off[x] = 0
            return x, 0
        path = []
        while self.parent[x] != x:
            path.append(x)
            x = self.parent[x]
        root = x
        # compress
        acc = 0
        for y in reversed(path):
            acc += self.off[y]
            self.off[y] = acc
            self.parent[y] = root
        return root, (self.off[path[0]] if path else 0)

    def union(self, a, b, diff):
        """Require exp(a) - exp(b) == diff."""
        ra, oa = self.find(a)
        rb, ob = self.find(b)
        if ra == rb:
            if oa - ob != diff:
                self.conflicts += 1
            return
        # attach ra under rb: off[ra] = exp(ra) - exp(rb) = (exp(a) - oa) - (exp(b) - ob) = diff - oa + ob
        self.parent[ra] = rb
        self.off[ra] = diff - oa + ob


def product_of(gate_sup, tt, rep, operand_ok):
    """(x, y) if the gate is an AND of two operand signals (or its complement,
    a NAND, whose negation the adder consumes), looked up through buffers and
    inverters (structural hashing resolved them), else None."""
    if len(gate_sup) != 2 or (tt not in AND_FAMILY2 and tt not in OR_FAMILY2):
        return None
    r0, n0 = rep.get(gate_sup[0], (gate_sup[0], False))
    r1, n1 = rep.get(gate_sup[1], (gate_sup[1], False))
    if r0 in operand_ok and r1 in operand_ok and r0 != r1:
        return (r0, r1)
    return None


def analyse_networks(cells, gates, rep, primary):
    """Adder networks (connected components of cells), their leaves, roots,
    weight consistency and operand grids."""
    # weights: accept cells greedily, full adders first, and reject a cell
    # that contradicts the weights fixed so far (an XOR and an AND over the
    # same two signals is also what control logic looks like)
    uf = WeightedUF()
    accepted = []
    rejected = 0
    order = sorted(range(len(cells)), key=lambda ci: (cells[ci][0] != "FA", ci))
    for ci in order:
        kind, leaves, negmask, s, sneg, cv, cneg = cells[ci]
        base = leaves[0]
        trial = [(x, base, 0) for x in leaves[1:]] + [(s, base, 0), (cv, base, 1)]
        # the cell's equations checked together: rewrite each as a required
        # difference between two current roots and look for a contradiction
        # among those (a tiny union-find over the roots), then commit
        local = WeightedUF()
        for a, b, d in trial:
            ra, oa = uf.find(a)
            rb, ob = uf.find(b)
            local.union(ra, rb, d - oa + ob)   # exp(ra) - exp(rb) must be d - oa + ob
        if local.conflicts:
            rejected += 1
            continue
        for a, b, d in trial:
            uf.union(a, b, d)
        accepted.append(ci)
    cells = [cells[ci] for ci in accepted]
    sig_cells = defaultdict(list)
    for ci, (kind, leaves, negmask, s, sneg, cv, cneg) in enumerate(cells):
        for x in leaves + (s, cv):
            sig_cells[x].append(ci)
    uf.conflicts = rejected
    # components over cells
    comp = {}
    comps = []
    for ci in range(len(cells)):
        if ci in comp:
            continue
        stack = [ci]
        comp[ci] = len(comps)
        members = []
        while stack:
            c = stack.pop()
            members.append(c)
            kind, leaves, negmask, s, sneg, cv, cneg = cells[c]
            for x in leaves + (s, cv):
                for d in sig_cells[x]:
                    if d not in comp:
                        comp[d] = len(comps)
                        stack.append(d)
        comps.append(members)
    out = []
    for members in comps:
        outs = set()
        ins = set()
        for c in members:
            kind, leaves, negmask, s, sneg, cv, cneg = cells[c]
            outs.add(s)
            outs.add(cv)
            ins.update(leaves)
        leaves = ins - outs
        roots = outs - ins
        # weight consistency inside the component
        roots_uf = {uf.find(x)[0] for x in ins | outs}
        # operands: leaves that are AND2 of two primary inputs
        pairs = []
        other = 0
        for leaf in leaves:
            g = gates.get(leaf)
            prod = product_of(g[0], g[1], rep, primary) if g else None
            if prod:
                pairs.append((prod[0], prod[1], uf.find(leaf)[1]))
            else:
                other += 1
        grid = None
        if len(pairs) >= 4:
            # two-colour the pair graph into operands A and B
            adj = defaultdict(set)
            for a, b, e in pairs:
                adj[a].add(b)
                adj[b].add(a)
            colour = {}
            ok = True
            for start in adj:
                if start in colour:
                    continue
                colour[start] = 0
                stack = [start]
                while stack and ok:
                    x = stack.pop()
                    for y in adj[x]:
                        if y not in colour:
                            colour[y] = 1 - colour[x]
                            stack.append(y)
                        elif colour[y] == colour[x]:
                            ok = False
                            break
            if ok:
                A = sorted(x for x, c in colour.items() if c == 0)
                B = sorted(x for x, c in colour.items() if c == 1)
                # exponents e(a_i b_j) = i + j: solve with another weighted
                # union-find over the operand signals (B's exponents enter
                # negated so that the sum becomes a difference); a word
                # multiplier gives every operand signal its own bit position,
                # a popcount-like product does not
                wf = WeightedUF()
                for a, b, e in pairs:
                    x, y = (a, b) if colour[a] == 0 else (b, a)
                    wf.union(("A", x), ("B", y), e)  # exp(A x) - (-exp(B y)) = e
                pos_a = {}
                pos_b = {}
                for a, b, e in pairs:
                    x, y = (a, b) if colour[a] == 0 else (b, a)
                    pos_a[x] = wf.find(("A", x))
                    pos_b[y] = wf.find(("B", y))
                wf_roots = {r for r, o in pos_a.values()} | {r for r, o in pos_b.values()}
                distinct = (wf.conflicts == 0
                            and len(wf_roots) == 1
                            and len({o for r, o in pos_a.values()}) == len(pos_a)
                            and len({o for r, o in pos_b.values()}) == len(pos_b))
                grid = (len(A), len(B), len(pairs), wf.conflicts, "word" if distinct else "nonword")
        out.append({
            "cells": len(members),
            "fa": sum(1 for c in members if cells[c][0] == "FA"),
            "ha": sum(1 for c in members if cells[c][0] != "FA"),
            "leaves": len(leaves),
            "roots": len(roots),
            "leaf_vars": leaves,
            "root_vars": roots,
            "pp_and": len(pairs),
            "pp_other": other,
            "grid": grid,
        })
    out.sort(key=lambda d: -d["cells"])
    return out, uf.conflicts, cells


def primary_inputs_of(vars_, gates, rep):
    """Primary inputs in the transitive fan-in of the given variables."""
    seen = set()
    prim = set()
    stack = list(vars_)
    while stack:
        u = stack.pop()
        if u in seen:
            continue
        seen.add(u)
        r, neg = rep.get(u, (u, False))
        if r != u:
            stack.append(r)
            continue
        g = gates.get(u)
        if g is None:
            prim.add(u)
        else:
            stack.extend(g[0])
    return prim


# ---------------------------------------------------------------------------
# one cell
# ---------------------------------------------------------------------------


def census_one(args):
    """One cell; never raises (an error is a row with the `error` column set)."""
    stem, path, max_clauses = args
    t0 = time.time()
    row = {"stem": stem}
    try:
        return _census_one(stem, path, max_clauses, t0, row)
    except Exception as e:  # noqa: BLE001
        row["error"] = f"{type(e).__name__}: {str(e)[:80]}"
        row["detect_s"] = round(time.time() - t0, 1)
        return row


def _census_one(stem, path, max_clauses, t0, row):
    try:
        n_vars, n_cls, clauses, truncated = parse_prefix(Path(path), max_clauses)
    except Exception as e:  # noqa: BLE001
        row["error"] = f"parse: {e}"
        return row
    row.update(vars=n_vars, clauses=len(clauses), truncated=int(truncated))
    if n_vars > MAX_GATE_VARS:
        row["error"] = "too-big"
        row["detect_s"] = round(time.time() - t0, 1)
        return row
    gates, used, deadlocks, topo_numbered = recover_gates(clauses, n_vars)
    order = topo_order(gates)
    present = {abs(l) for c in clauses for l in c}
    primary = present - set(gates)
    row.update(
        gates=len(gates),
        gate_cov=round(len(gates) / max(1, len(present)), 3),
        primary=len(primary),
        deadlocks=deadlocks,
        topo_numbered=int(topo_numbered),
        constraint_clauses=len(clauses) - len(used),
    )
    kinds = defaultdict(int)
    for v, (sup, tt, cl) in gates.items():
        n = len(sup)
        if n == 2 and tt in (XOR2, XNOR2):
            kinds["xor2"] += 1
        elif n == 2 and tt in AND_FAMILY2:
            kinds["and2"] += 1
        elif n == 3 and tt in (XOR3, XNOR3):
            kinds["xor3"] += 1
        elif n == 3 and tt in MAJ_FAMILY3:
            kinds["maj3"] += 1
        elif n == 3:
            kinds["other3"] += 1
        elif n == 1:
            kinds["buf"] += 1
        else:
            kinds[f"other{n}"] += 1
    row["kinds"] = ";".join(f"{k}={v}" for k, v in sorted(kinds.items()))
    rep, merged = structural_hash(gates, order)
    row["merged"] = merged
    if not gates or len(gates) > MAX_FULL_GATES:
        row["error"] = "no-gates" if not gates else "gates-only"
        row["detect_s"] = round(time.time() - t0, 1)
        return row
    units = {abs(c[0]) for c in clauses if len(c) == 1}
    cone_cache = {}

    def cone(rt):
        if rt not in cone_cache:
            cone_set = set()
            stack = [rt]
            while stack:
                u = stack.pop()
                r, ng = rep.get(u, (u, False))
                if r in cone_set or r not in gates:
                    continue
                cone_set.add(r)
                stack.extend(gates[r][0])
            cone_cache[rt] = cone_set
        return cone_cache[rt]

    def different_cones(a, b):
        ca, cb = cone(a), cone(b)
        return len(ca & cb) < COMPARE_OVERLAP * max(1, min(len(ca), len(cb)))

    cells = find_adders(gates, order, rep)
    # the top: constraint clauses, and the walk from each unit through and/or
    # gates to the compared signals.  A cell met on that walk whose two
    # inputs have large, nearly disjoint cones is the miter's compare (an
    # xor and an and over an output bit of each side), not an adder: it is
    # dropped and the walk repeated without it.  Adders deep in a tree are
    # never reached this way (the walk stops at the first cell outputs).
    top_vars = set()
    for i, c in enumerate(clauses):
        if i not in used:
            top_vars.update(abs(l) for l in c)

    def cell_outputs(cells_):
        out = {}
        for ci, (kind, leaves, negmask, sv, sneg, cv, cneg) in enumerate(cells_):
            out[rep.get(sv, (sv, False))[0]] = ci
            out[rep.get(cv, (cv, False))[0]] = ci
        return out

    def top_walk(cell_out_):
        compared_ = set(top_vars - units)
        pairs_ = []
        reached_ = set()
        stack = [(u, 0) for u in units]
        while stack:
            u, depth = stack.pop()
            u = rep.get(u, (u, False))[0]   # through buffers and inverters
            g = gates.get(u)
            if g is None or u in reached_ or depth > 12:
                continue
            reached_.add(u)
            sup, tt, cl = g
            if len(sup) == 1:               # a buffer or inverter that hashing left as a gate
                stack.append((sup[0], depth))
                continue
            if u in cell_out_:
                compared_.add(u)        # an adder output: the circuit's own signal
                continue
            if len(sup) == 2 and tt in (XOR2, XNOR2):
                if different_cones(*sup):
                    compared_.update(sup)   # a compare of two sides
                    pairs_.append(tuple(sup))
                else:
                    compared_.add(u)        # an output bit
            elif len(sup) == 2 and (tt in AND_FAMILY2 or tt in OR_FAMILY2):
                stack.extend((x, depth + 1) for x in sup)
            else:
                compared_.add(u)
        return compared_, pairs_, reached_

    for _round in range(3):
        cell_out = cell_outputs(cells)
        compared, compare_pairs, reached = top_walk(cell_out)
        drop = set()
        for ci, c in enumerate(cells):
            if len(c[1]) != 2:
                continue
            sv, cv = rep.get(c[3], (c[3], False))[0], rep.get(c[5], (c[5], False))[0]
            if sv not in reached and cv not in reached:
                continue
            a, b = c[1]
            if min(len(cone(a)), len(cone(b))) >= MIN_COMPARE_CONE and different_cones(a, b):
                drop.add(ci)
        if not drop:
            break
        cells = [c for ci, c in enumerate(cells) if ci not in drop]
    row["fa"] = sum(1 for c in cells if c[0] == "FA")
    row["ha"] = sum(1 for c in cells if c[0] == "HA")
    reps = {v for v in gates if rep[v][0] == v}
    operand_ok = set(primary)
    for v, (sup, tt, cl) in gates.items():
        if len(sup) == 2 and tt in (XOR2, XNOR2) and any(u in primary for u in sup):
            operand_ok.add(v)
    nets, wconf, cells = analyse_networks(cells, gates, rep, operand_ok)
    cell_out = cell_outputs(cells)
    row["networks"] = len(nets)
    row["weight_conflicts"] = wconf
    row["fa"] = sum(1 for c in cells if c[0] == "FA")
    row["ha"] = sum(1 for c in cells if c[0] == "HA")
    row["ha1"] = sum(1 for c in cells if c[0] == "HA1")
    # explained gates: every representative gate inside an accepted cell's
    # cone (outputs down to the cut leaves), plus the partial-product ANDs
    explained = set()
    for kind, leaves, negmask, sv, sneg, cv, cneg in cells:
        stop = set(leaves)
        stack = [sv, cv]
        while stack:
            u = stack.pop()
            r, ng = rep.get(u, (u, False))
            if r in stop or r in explained or r not in gates:
                continue
            explained.add(r)
            stack.extend(gates[r][0])
    for nd in nets:
        for leaf in nd["leaf_vars"]:
            g = gates.get(leaf)
            if g and product_of(g[0], g[1], rep, operand_ok):
                explained.add(leaf)
    unexplained = [v for v in reps if v not in explained]
    row["adder_cov"] = round(1 - len(unexplained) / max(1, len(reps)), 3)
    ukinds = defaultdict(int)
    for v in unexplained:
        sup, tt, cl = gates[v]
        n = len(sup)
        if n == 2 and tt in (XOR2, XNOR2):
            ukinds["xor2"] += 1
        elif n == 2 and tt in AND_FAMILY2:
            ukinds["and2"] += 1
        elif n == 2:
            ukinds["or2"] += 1
        elif n == 3 and tt in (XOR3, XNOR3):
            ukinds["xor3"] += 1
        elif n == 3 and tt in MAJ_FAMILY3:
            ukinds["maj3"] += 1
        else:
            ukinds[f"other{n}"] += 1
    row["unexplained"] = ";".join(f"{k}={v}" for k, v in sorted(ukinds.items()))
    row["top_vars"] = len(top_vars)
    # per side: the cone of each compared variable (a miter compares two),
    # how much of it the cells explain, and what is left (a carry-lookahead
    # final adder shows as AND trees over half-adder outputs)
    compared = sorted(compared)
    sides = []
    cones = {}
    # candidates: every compare pair's two variables (a whole-word miter has
    # one pair per bit; the widest pair is the most telling), else the
    # compared variables in index order
    if compare_pairs:
        widest = max(compare_pairs, key=lambda ab: min(len(cone(ab[0])), len(cone(ab[1]))))
        candidates = list(widest) + [v for v in compared if v not in widest][:6]
    else:
        candidates = sorted(compared[:64], key=lambda v: -len(cone(v)))[:8]
    for rt in candidates:
        c = cone(rt)
        if c:
            cones[rt] = c
    # the two sides of a miter: the widest pair of compared variables whose
    # cones are apart, else the pair that overlaps least
    pick = list(cones)[:1]
    if len(cones) >= 2:
        def overlap(a, b):
            return len(cones[a] & cones[b]) / max(1, min(len(cones[a]), len(cones[b])))
        pairs = [(a, b) for a in cones for b in cones if a < b]
        apart = [(a, b) for a, b in pairs if overlap(a, b) < COMPARE_OVERLAP]
        if apart:
            a, b = max(apart, key=lambda ab: min(len(cones[ab[0]]), len(cones[ab[1]])))
        else:
            a, b = min(pairs, key=lambda ab: overlap(*ab))
        pick = [a, b]
        row["sides_overlap"] = round(overlap(a, b), 3)
    for rt in pick:
        cone_set = cones[rt]
        n_pp = sum(1 for v in cone_set if v in explained and v not in cell_out and len(gates[v][0]) == 2)
        n_cells = len({cell_out[v] for v in cone_set if v in cell_out})
        unex = [v for v in cone_set if v not in explained]
        cpa = 0
        for v in unex:
            sup, tt, cl = gates[v]
            if len(sup) == 2 and tt in (XOR2, XNOR2):
                kinds2 = [rep.get(u, (u, False))[0] in cell_out for u in sup]
                if kinds2.count(True) == 1:
                    cpa += 1  # a sum bit of a final adder: half-adder propagate xor a carry from the prefix network
        sides.append(f"side{rt}:gates={len(cone_set)},cells={n_cells},explained={1 - len(unex) / len(cone_set):.2f},cpa_bits={cpa}")
    row["sides"] = " | ".join(sides)
    row["n_compared"] = len(compared)
    if len(pick) == 2:
        # partial products (ANDs of two operand signals) in both cones
        pp_sets = []
        for rt in pick:
            pp_sets.append({v for v in cones[rt] if product_of(gates[v][0], gates[v][1], rep, operand_ok)})
        row["sides_shared_pp"] = round(len(pp_sets[0] & pp_sets[1]) / max(1, min(len(pp_sets[0]), len(pp_sets[1]))), 3)
    big = nets[:2]
    desc = []
    for i, nd in enumerate(big):
        prim = primary_inputs_of(nd["leaf_vars"] | nd["root_vars"], gates, rep)
        g = nd["grid"]
        gtxt = f"grid={g[0]}x{g[1]}/{g[2]}pp/{g[3]}conf/{g[4]}" if g else "grid=-"
        desc.append(f"net{i}:cells={nd['cells']}(fa={nd['fa']},ha={nd['ha']}),leaves={nd['leaves']},roots={nd['roots']},"
                    f"pp_and={nd['pp_and']},pp_other={nd['pp_other']},{gtxt},inputs={len(prim)}")
    row["nets_desc"] = " | ".join(desc)

    # simulation: equivalence candidates not merged
    width = 64
    mask = (1 << width) - 1
    rnd = random.Random(1)
    words = {u: rnd.getrandbits(width) for u in primary}
    val = simulate(gates, order, rep, words, mask)
    sigs = defaultdict(list)
    for v in reps:
        w = val[v]
        key = min(w, (~w) & mask)
        sigs[key].append(v)
    eqc = sum(len(vs) for vs in sigs.values() if len(vs) > 1)
    const = sum(1 for v in reps if val[v] in (0, mask))
    row["eq_cands"] = eqc
    row["const_cands"] = const
    row["detect_s"] = round(time.time() - t0, 1)
    return row


COLUMNS = ["stem", "vars", "clauses", "truncated", "gates", "gate_cov", "primary", "deadlocks", "topo_numbered", "constraint_clauses",
           "kinds", "merged", "fa", "ha", "ha1", "adder_cov", "unexplained", "networks", "weight_conflicts", "top_vars",
           "n_compared", "sides_overlap", "sides_shared_pp", "sides", "eq_cands", "const_cands", "nets_desc", "detect_s", "error"]


def cmd_census(args):
    stems = None
    if args.stems:
        stems = set(args.stems.split(","))
    files = [Path(p) for p in args.files] if args.files else sorted(SUITE.glob("*.cnf.xz"))
    jobs = []
    for f in files:
        stem = f.name[: -len(".cnf.xz")] if f.name.endswith(".cnf.xz") else f.stem
        if stems and not any(s in stem for s in stems):
            continue
        jobs.append((stem, str(f), args.max_clauses))
    # rows are written as they complete, in completion order (a run takes an
    # hour or more and one slow cell must not hold the others back)
    with open(args.out, "w", newline="") as f, ProcessPoolExecutor(max_workers=args.workers) as ex:
        w = csv.DictWriter(f, fieldnames=COLUMNS, delimiter="\t", extrasaction="ignore")
        w.writeheader()
        f.flush()
        futures = [ex.submit(census_one, job) for job in jobs]
        for fut in as_completed(futures):
            row = fut.result()
            w.writerow(row)
            f.flush()
            print(f"{row['stem'][:70]:70} gates={row.get('gates','-'):>6} cov={row.get('gate_cov','-'):>5} "
                  f"fa={row.get('fa','-'):>4} ha={row.get('ha','-'):>4} nets={row.get('networks','-'):>3} "
                  f"wconf={row.get('weight_conflicts','-'):>3} shpp={row.get('sides_shared_pp','-'):>5} "
                  f"eq={row.get('eq_cands','-'):>5} {row.get('error','')} {row.get('detect_s','')}s", flush=True)
    return 0


def _family(stem, families):
    return families.get(stem, stem[33:].split("_")[0][:18] if len(stem) > 33 else stem[:18])


def cmd_report(args):
    with open(args.table) as f:
        rows = list(csv.DictReader(f, delimiter="\t"))
    families = {}
    if CELLS.is_file():
        with open(CELLS) as f:
            for r in csv.DictReader(f, delimiter="\t"):
                families[r["stem"]] = r["family"]

    def num(r, k, default=0.0):
        try:
            return float(r.get(k) or default)
        except ValueError:
            return default

    def side_expl(r):
        out = []
        for part in (r.get("sides") or "").split(" | "):
            if ":" in part:
                d = dict(kv.split("=") for kv in part.split(":", 1)[1].split(","))
                out.append(float(d["explained"]))
        return out

    if args.cells:
        for r in rows:
            print(f"== {r['stem']}")
            for k in COLUMNS[1:]:
                if r.get(k):
                    print(f"   {k}: {r[k]}")
        return 0
    by_fam = defaultdict(list)
    for r in rows:
        by_fam[_family(r["stem"], families)].append(r)
    print(f"{len(rows)} cells; {sum(1 for r in rows if r.get('error'))} not fully analysed "
          f"({', '.join(sorted({r['error'] for r in rows if r.get('error')}))})")
    print(f"{'family':22} {'cells':>5} {'gcov':>5} {'adders>=50':>10} {'grid>=8x8':>9} {'2-cone':>6} {'side>=.9':>8} {'eq>5%':>5}")
    for fam in sorted(by_fam, key=lambda f: -len(by_fam[f])):
        rs = by_fam[fam]
        covs = sorted(num(r, "gate_cov") for r in rs if r.get("gate_cov"))
        med = covs[len(covs) // 2] if covs else 0
        adders = sum(1 for r in rs if num(r, "fa") + num(r, "ha") + num(r, "ha1") >= 50)
        grid = 0
        for r in rs:
            for part in (r.get("nets_desc") or "").split(" | "):
                g = [p for p in part.split(",") if p.startswith("grid=") and "x" in p]
                if g:
                    a, b = g[0][5:].split("/")[0].split("x")
                    if int(a) >= 8 and int(b) >= 8 and g[0].endswith("/word"):
                        grid += 1
                        break
        two = sum(1 for r in rs if len(side_expl(r)) == 2)
        good = sum(1 for r in rs if side_expl(r) and min(side_expl(r)) >= 0.9)
        eq = sum(1 for r in rs if num(r, "gates") and num(r, "eq_cands") / num(r, "gates") > 0.05)
        print(f"{fam[:22]:22} {len(rs):>5} {med:>5.2f} {adders:>10} {grid:>9} {two:>6} {good:>8} {eq:>5}")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--out", required=True)
    c.add_argument("--stems", help="comma-separated substrings of stems to analyse (default: all)")
    c.add_argument("--files", nargs="*", help="explicit .cnf.xz files instead of the suite (toys, checks)")
    c.add_argument("--workers", type=int, default=4)
    c.add_argument("--max-clauses", type=int, default=3_000_000)
    c.set_defaults(func=cmd_census)
    r = sub.add_parser("report")
    r.add_argument("table")
    r.add_argument("--cells", action="store_true", help="per-cell detail instead of the family summary")
    r.set_defaults(func=cmd_report)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
