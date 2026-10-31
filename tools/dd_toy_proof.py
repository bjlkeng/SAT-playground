#!/usr/bin/env python3
"""Toy check and prototype of the decision-diagram counting step (bead
SAT-playground-skz.6; the rook cells of sat-comp-2025).

The shape. Board variables under pairwise at-most-one groups (the rows of
a board), and a counter written as a decision diagram: one-directional
ITE nodes  n -> (x ? hi : lo), one board variable per level, children on
later levels, the root asserted by a unit. The diagram says "at least k
of the board variables are true"; the groups say "at most one per group".
With more required than there are groups, the formula is unsatisfiable.

The proof, in plain terms. For every node we prove in VeriPB the sum
constraint "node -> at least val(node) of the groups still open below this
level have a true variable", where val is the least number of trues the
node's subtree forces (val(hi)+1 or val(lo), whichever is smaller). The
sum runs over groups, not board variables: one fresh variable per level
says "this level's group has a true variable at this level or below"
(w = x or w_next), so every constraint has at most as many terms as there
are groups. A node's constraint follows from its children's by a proof by
contradiction (`pbc`) of three lines: the lo child is impossible (its sum
is the node's sum or less), the hi child is impossible once x is true (x
takes its group's slot, the at-most-one clauses close the group below),
and unit propagation over the node's clauses does the rest. At the root
the sum has as many terms as groups but asks for more: contradiction.

Usage:
  python3 tools/dd_toy_proof.py toy [--n 4] [--k 5] [--seed 0] [--veripb PATH] [--out DIR]
  python3 tools/dd_toy_proof.py file <cnf[.xz]> [--veripb PATH] [--out DIR]
  python3 tools/dd_toy_proof.py selftest
"""
from __future__ import annotations

import argparse
import itertools
import lzma
import os
import random
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_VERIPB = ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb"

INF = float("inf")


# ---------------------------------------------------------------- reading

def read_cnf(path):
    """The clauses as the solver reads them (clauses split at 0, comments,
    one or several to a line; `tools/proof_gate.py` has the reader), each
    with its literals deduplicated; clause i is constraint i+1."""
    sys.path.insert(0, str(ROOT / "tools"))
    import proof_gate
    clauses = []
    nv = 0
    gen = proof_gate.iter_clauses(path)
    while True:
        try:
            lits = next(gen)
        except StopIteration as stop:
            nv = max(nv, stop.value or 0)
            break
        seen = []
        for l in lits:
            if l not in seen:
                seen.append(l)
            nv = max(nv, abs(l))
        clauses.append(seen)
    return nv, clauses


# -------------------------------------------------------------- detection

class Node:
    __slots__ = ("var", "defs", "cids", "bvars", "children", "x", "hi", "lo", "val", "level", "c1", "c2", "tid", "parents")

    def __init__(self, var):
        self.var = var
        self.defs = []      # clauses without ~var
        self.cids = []      # their 1-based ids
        self.bvars = set()
        self.children = set()
        self.x = None
        self.hi = None      # 'T', 'F', ('lit', l), ('node', var)
        self.lo = None
        self.val = None
        self.level = None
        self.c1 = None      # id of the clause that gives hi under x = 1
        self.c2 = None      # id of the clause that gives lo under x = 0
        self.tid = None
        self.parents = 0


class Diagram:
    """What the proof needs: nodes, board order, groups."""

    def __init__(self):
        self.nodes = {}        # var -> Node
        self.root = None
        self.root_unit = None  # clause id of the unit asserting the root
        self.order = []        # board variables by level
        self.level = {}        # board var -> level
        self.group = {}        # board var -> group index
        self.groups = []       # group -> levels ascending
        self.reason = ""


def residual(defs, cids, x, value):
    """The clauses left when x takes value (1 or 0), as (kind, payload,
    clause id): 'T' nothing left, 'F' an empty clause, or one literal that
    is a unit among them and sits in every other one."""
    left = []
    for d, cid in zip(defs, cids):
        if (x if value else -x) in d:
            continue
        rest = [l for l in d if abs(l) != x]
        left.append((rest, cid))
    if not left:
        return ("T", None, None)
    for rest, cid in left:
        if not rest:
            return ("F", None, cid)
    for rest, cid in left:
        if len(rest) == 1:
            l = rest[0]
            if all(l in r for r, _ in left):
                return ("lit", l, cid)
    return None


def detect(nv, clauses, debug=False):
    pos_occ = defaultdict(list)
    neg_occ = defaultdict(list)
    units = {}
    pair = defaultdict(set)
    for ci, c in enumerate(clauses):
        if len(c) == 1:
            units.setdefault(c[0], ci)
        if len(c) == 2 and c[0] < 0 and c[1] < 0:
            pair[-c[0]].add(-c[1])
            pair[-c[1]].add(-c[0])
        for l in c:
            (pos_occ if l > 0 else neg_occ)[abs(l)].append(ci)
    dd = Diagram()
    kind = {}  # var -> 'node' | 'not'
    nodes = dd.nodes

    def read_all(start):
        # a node's children are the positive literals of its clauses; a
        # variable is read once its children are, so a variable on the
        # path is 'pending' and a clause that holds it negated (the way a
        # board variable holds its level's nodes) ends the reading
        stack = [(start, 0)]
        while stack:
            v, phase = stack.pop()
            if phase == 0 and v in kind:
                continue
            occ = neg_occ.get(v, [])
            if not 1 <= len(occ) <= 3:
                kind[v] = "not"
                continue
            if phase == 0:
                kind[v] = "pending"
                stack.append((v, 1))
                for ci in occ:
                    for l in clauses[ci]:
                        if l > 0 and l != v and l not in kind:
                            stack.append((l, 0))
                continue
            nd = Node(v)
            ok = True
            for ci in occ:
                d = [l for l in clauses[ci] if l != -v]
                if v in d:
                    ok = False  # a tautology
                    break
                nd.defs.append(d)
                nd.cids.append(ci + 1)
                for l in d:
                    k = kind.get(abs(l))
                    if k == "node":
                        if l < 0:
                            ok = False
                        else:
                            nd.children.add(l)
                    elif k == "pending":
                        ok = False  # a cycle, or a level variable of a node on the path
                    else:
                        nd.bvars.add(abs(l))
            if ok and 1 <= len(nd.bvars) <= 2:
                nodes[v] = nd
                kind[v] = "node"
            else:
                kind[v] = "not"

    roots = []
    for lit, ci in units.items():
        if lit > 0:
            read_all(lit)
            if kind.get(lit) == "node":
                roots.append((lit, ci))
    if not roots:
        dd.reason = "no asserted root"
        return dd
    root, root_ci = roots[0]
    dd.root, dd.root_unit = root, root_ci + 1
    # the nodes reachable from the root
    reach = {}
    stack = [root]
    while stack:
        v = stack.pop()
        if v in reach:
            continue
        reach[v] = nodes[v]
        for c in nodes[v].children:
            stack.append(c)
    nodes = dd.nodes = reach
    board = set()
    for nd in nodes.values():
        board |= nd.bvars
    if board & set(nodes):
        dd.reason = "a variable is both a node and a board variable"
        return dd
    # order constraints among board variables
    after = defaultdict(set)
    indeg = defaultdict(int)
    for v in board:
        indeg[v] += 0
    for nd in nodes.values():
        x = None
        if len(nd.bvars) == 1:
            x = next(iter(nd.bvars))
        else:
            both = [b for b in nd.bvars if any(b in d for d in nd.defs) and any(-b in d for d in nd.defs)]
            if len(both) == 1:
                x = both[0]
        if x is None:
            continue  # settled by the order
        targets = set(nd.bvars) - {x}
        for c in nd.children:
            targets |= nodes[c].bvars
        for t in targets:
            if t not in after[x]:
                after[x].add(t)
                indeg[t] += 1
    ready = sorted(v for v in board if indeg[v] == 0)
    order = []
    import heapq
    heapq.heapify(ready)
    while ready:
        v = heapq.heappop(ready)
        order.append(v)
        for t in sorted(after[v]):
            indeg[t] -= 1
            if indeg[t] == 0:
                heapq.heappush(ready, t)
    if len(order) != len(board):
        dd.reason = "the board variables have no level order"
        return dd
    dd.order = order
    dd.level = {v: i for i, v in enumerate(order)}
    # read every node as an ITE on its earliest board variable
    for nd in nodes.values():
        nd.x = min(nd.bvars, key=lambda b: dd.level[b])
        nd.level = dd.level[nd.x]
        for c in nd.children:
            if nodes[c].level is None:
                pass
        hi = residual(nd.defs, nd.cids, nd.x, 1)
        lo = residual(nd.defs, nd.cids, nd.x, 0)
        if hi is None or lo is None:
            dd.reason = f"node {nd.var} is not an ITE on x{nd.x}: {nd.defs}"
            return dd
        for side, (k, l, cid) in (("hi", hi), ("lo", lo)):
            if k == "lit":
                child = ("node", l) if l in nodes else ("lit", l)
                if child[0] == "lit" and dd.level[abs(l)] <= nd.level:
                    dd.reason = f"node {nd.var}: literal child {l} not below level of x{nd.x}"
                    return dd
            else:
                child = k
            setattr(nd, side, child)
            setattr(nd, "c1" if side == "hi" else "c2", cid)
    for nd in nodes.values():
        for c in nd.children:
            if nodes[c].level <= nd.level:
                dd.reason = f"node {nd.var} at level {nd.level} has child {c} at level {nodes[c].level}"
                return dd
            nodes[c].parents += 1
    # val, bottom-up by level
    def val_of(child):
        if child == "T":
            return 0
        if child == "F":
            return INF
        if child[0] == "lit":
            return 1 if child[1] > 0 else 0
        return nodes[child[1]].val
    for nd in sorted(nodes.values(), key=lambda n: -n.level):
        nd.val = min(1 + val_of(nd.hi), val_of(nd.lo))
    # groups: a clique partition of the board variables in the pair graph
    unassigned = set(board)
    groups = []
    for v in order:
        if v not in unassigned:
            continue
        unassigned.discard(v)
        clique = [v]
        cand = sorted(pair.get(v, set()) & unassigned)
        if cand:
            # the first pick by common neighbours (row or column), then intersections
            best = max(cand, key=lambda m: len(pair[m] & set(cand)))
            clique.append(best)
            unassigned.discard(best)
            cand = [m for m in cand if m != best and m in pair[best]]
            while cand:
                m = cand[0]
                clique.append(m)
                unassigned.discard(m)
                cand = [u for u in cand[1:] if u in pair[m]]
        gi = len(groups)
        for m in clique:
            dd.group[m] = gi
        groups.append(sorted(dd.level[m] for m in clique))
    dd.groups = groups
    if debug:
        print(f"nodes {len(nodes)} board {len(board)} groups {len(groups)} root val {nodes[root].val}", file=sys.stderr)
    return dd


# ------------------------------------------------------------------ proof

class Proof:
    def __init__(self, path, input_clauses, nv):
        self.out = open(path, "w")
        self.out.write("pseudo-Boolean proof version 3.0\n")
        self.n = input_clauses
        self.next_var = nv
        self.lines = 0

    @staticmethod
    def lit(l):
        return f"x{l}" if l > 0 else f"~x{-l}"

    def fresh(self):
        self.next_var += 1
        return self.next_var

    def terms(self, terms, rhs):
        return " ".join(f"{c} {self.lit(l)}" for c, l in terms) + f" >= {rhs}"

    def rup(self, terms, rhs=1):
        self.out.write(f"rup {self.terms(terms, rhs)} ;\n")
        self.n += 1
        self.lines += 1
        return self.n

    def red(self, clause, var, value):
        self.out.write(f"red {self.terms([(1, l) for l in clause], 1)} : x{var} -> {1 if value else 0} ;\n")
        self.n += 1
        self.lines += 1
        return self.n

    def pol(self, expr):
        self.out.write(f"pol {expr} ;\n")
        self.n += 1
        self.lines += 1
        return self.n

    def pbc(self, terms, rhs, lines):
        """`lines` is a function of the negation's id returning the subproof lines."""
        neg = self.n + 1
        body = lines(neg)
        self.out.write(f"pbc {self.terms(terms, rhs)} : subproof\n")
        for l in body:
            self.out.write(f"    {l}\n")
        self.out.write("qed;\n")
        self.n += 1 + len(body) + 1
        self.lines += 1
        return self.n

    def delete(self, ids):
        if ids:
            self.out.write("del id " + " ".join(str(i) for i in ids) + " ;\n")

    def finish(self, refuted):
        if refuted:
            self.out.write("output NONE ;\nconclusion UNSAT ;\n")
        else:
            self.out.write("output EQUISATISFIABLE FILE ;\nconclusion NONE ;\n")
        self.out.write("end pseudo-Boolean proof ;\n")
        self.out.close()


def write_proof(dd, nv, clauses, path, delete=True):
    """Returns (refuted, stats)."""
    p = Proof(path, len(clauses), nv)
    nodes = dd.nodes
    order = dd.order
    m = len(order)
    g = len(dd.groups)
    # per group: its levels ascending, the variable v(R, level_k) for each
    # (the last level's board variable, else a fresh w), and clause ids
    gl = dd.groups                       # group -> levels
    pos_in_group = {}                    # level -> index k in its group
    for gi, levels in enumerate(gl):
        for k, lv in enumerate(levels):
            pos_in_group[lv] = k
    vv = [[None] * len(levels) for levels in gl]        # group, k -> variable
    h1 = [[None] * len(levels) for levels in gl]        # (~x v w)
    cc = [[None] * len(levels) for levels in gl]        # (~q v w): mono(k, k+1)
    h3 = [[None] * len(levels) for levels in gl]        # (~x_k v ~v_{k+1})
    for gi, levels in enumerate(gl):
        t = len(levels)
        vv[gi][t - 1] = order[levels[t - 1]]
        for k in range(t - 2, -1, -1):
            x = order[levels[k]]
            q = vv[gi][k + 1]
            w = p.fresh()
            vv[gi][k] = w
            p.red([-w, x, q], w, False)
            h1[gi][k] = p.red([-x, w], w, True)
            cc[gi][k] = p.red([-q, w], w, True)
    for gi, levels in enumerate(gl):
        for k in range(len(levels) - 1):
            x = order[levels[k]]
            h3[gi][k] = p.rup([(1, -x), (1, -vv[gi][k + 1])])
    mono_cache = {}
    h3_cache = {}

    def mono(gi, k, k2):
        """(v_k v ~v_k2) for k < k2 in group gi."""
        if k2 == k + 1:
            return cc[gi][k]
        key = (gi, k, k2)
        if key not in mono_cache:
            mono_cache[key] = p.rup([(1, vv[gi][k]), (1, -vv[gi][k2])])
        return mono_cache[key]

    def h3_at(gi, k, k2):
        if k2 == k + 1:
            return h3[gi][k]
        key = (gi, k, k2)
        if key not in h3_cache:
            x = order[gl[gi][k]]
            h3_cache[key] = p.rup([(1, -x), (1, -vv[gi][k2])])
        return h3_cache[key]

    def first_k(gi, i):
        """Index k of the first level of group gi at or after level i, or None."""
        levels = gl[gi]
        import bisect
        k = bisect.bisect_left(levels, i)
        return k if k < len(levels) else None

    # R_i as a list of (group, k) for each level i, built from the end
    rows_at = [None] * (m + 1)
    cur = {}
    rows_at[m] = {}
    for i in range(m - 1, -1, -1):
        gi = dd.group[order[i]]
        cur = dict(cur)
        cur[gi] = pos_in_group[i]
        rows_at[i] = cur

    def sum_terms(i):
        return [(1, vv[gi][k]) for gi, k in sorted(rows_at[i].items())]

    lit_t = {}

    def lit_T(y):
        """T for a positive board literal y: ~y + sum(level y) >= 1."""
        if y not in lit_t:
            j = dd.level[y]
            lit_t[y] = p.rup([(1, -y)] + sum_terms(j), 1)
        return lit_t[y]

    def child_info(child):
        """(val, T id or None, level or None)."""
        if child == "T":
            return 0, None, None
        if child == "F":
            return INF, None, None
        if child[0] == "lit":
            l = child[1]
            if l < 0:
                return 0, None, None
            return 1, lit_T(l), dd.level[l]
        nd = nodes[child[1]]
        return nd.val, nd.tid, nd.level

    def cancel(i, j, qgi, hi_side):
        """Tokens that cancel the sums of level i (negated, in ~T) against the
        sum of level j (a child's T), for the groups open at level i."""
        toks = []
        rj = rows_at[j] if j is not None else {}
        for gi, k in sorted(rows_at[i].items()):
            if hi_side and gi == qgi:
                if h1[gi][k] is not None:
                    toks.append(f"{h1[gi][k]} +")
                if gi in rj:
                    toks.append(f"{h3_at(gi, k, rj[gi])} +")
                continue
            if gi not in rj:
                toks.append(f"x{vv[gi][k]} +")
            elif rj[gi] != k:
                toks.append(f"{mono(gi, k, rj[gi])} +")
        return toks

    stats = {"nodes": 0, "false_nodes": 0, "skipped": 0}
    pending_parents = {v: nd.parents for v, nd in nodes.items()}
    for nd in sorted(nodes.values(), key=lambda n: -n.level):
        i = nd.level
        qgi = dd.group[nd.x]
        if nd.val == INF:
            nd.tid = p.rup([(1, -nd.var)], 1)
            stats["false_nodes"] += 1
        elif nd.val == 0:
            stats["skipped"] += 1
        else:
            a = nd.val
            bval, blo, jlo = child_info(nd.lo)
            cval, bhi, jhi = child_info(nd.hi)

            # the helper clauses (mono, h3 for skipped levels) are written
            # before the negation's id is taken
            lo_toks = cancel(i, jlo, qgi, False) if (bval != INF and bval >= 1) else None
            hi_toks = cancel(i, jhi, qgi, True) if cval != INF else None

            def lines(neg, blo=blo, bhi=bhi, lo_toks=lo_toks, hi_toks=hi_toks):
                out = []
                if lo_toks is not None:
                    out.append("pol " + " ".join([str(neg), f"{blo} +"] + lo_toks) + " ;")
                if hi_toks is not None:
                    out.append("pol " + " ".join([str(neg)] + ([f"{bhi} +"] if bhi is not None else []) + hi_toks) + " ;")
                out.append("rup >= 1 ;")
                return out

            nd.tid = p.pbc([(a, -nd.var)] + sum_terms(i), a, lines)
            stats["nodes"] += 1
        if delete:
            done = []
            for c in nd.children:
                pending_parents[c] -= 1
                if pending_parents[c] == 0 and nodes[c].tid is not None and c != dd.root:
                    done.append(nodes[c].tid)
            p.delete(done)
    root = nodes[dd.root]
    refuted = False
    stats["root_val"] = root.val
    stats["groups"] = g
    if root.val == INF:
        p.pol(f"{root.tid} {dd.root_unit} +")
        p.rup([], 1)
        refuted = True
    elif root.val > g:
        toks = [str(root.tid)] + [f"~x{vv[gi][k]} +" for gi, k in sorted(rows_at[0].items())] + [f"{root.val} d"]
        p.pol(" ".join(toks))
        p.rup([], 1)
        refuted = True
    p.finish(refuted)
    stats["lines"] = p.lines
    stats["fresh"] = p.next_var - nv
    return refuted, stats


# ------------------------------------------------------------------ toys

def encode_counter(clauses, var_of_level, k, next_var):
    """A one-directional counter diagram for "exactly k of the variables
    are true", in level order, as the rook generator writes it: ITE nodes
    with constants simplified, tautologies dropped and a node's subsumed
    clauses removed. Returns (root literal, next_var)."""
    m = len(var_of_level)
    memo = {}

    def node(i, c):
        rem = m - i
        if c < 0 or c > rem:
            return "F"
        if rem == 0:
            return "T"
        if (i, c) in memo:
            return memo[(i, c)]
        x = var_of_level[i]
        if rem == 1:
            r = x if c == 1 else -x
            memo[(i, c)] = r
            return r
        hi = node(i + 1, c - 1)
        lo = node(i + 1, c)
        nonlocal next_var
        next_var += 1
        n = next_var
        cls = []
        for cl in ([-n, -x, hi], [-n, x, lo], [-n, hi, lo]):
            if "T" in cl:
                continue
            cl = [l for l in cl if l != "F"]
            if any(-l in cl for l in cl):
                continue
            cls.append(cl)
        kept = []
        for cl in cls:
            if any(set(o) < set(cl) for o in cls):
                continue
            if cl not in kept:
                kept.append(cl)
        clauses.extend(kept)
        memo[(i, c)] = n
        return n

    # the root must be a variable: allocate it even for small boards
    r = node(0, k)
    return r, next_var


def rook_toy(n, k, seed, path):
    """n x n board, pairwise at-most-one on rows and columns, a counter
    diagram asserting exactly k rooks; the diagram's level order is a
    seeded shuffle (seed 0: rows from the last, as the rook files)."""
    clauses = []
    v = lambda r, c: r * n + c + 1
    for r in range(n):
        for c1, c2 in itertools.combinations(range(n), 2):
            clauses.append([-v(r, c1), -v(r, c2)])
    for c in range(n):
        for r1, r2 in itertools.combinations(range(n), 2):
            clauses.append([-v(r1, c), -v(r2, c)])
    levels = [v(r, c) for r in reversed(range(n)) for c in range(n)]
    if seed:
        random.Random(seed).shuffle(levels)
    root, nv = encode_counter(clauses, levels, k, n * n)
    assert isinstance(root, int) and root > n * n
    clauses.insert(0, [root])
    with open(path, "w") as f:
        f.write(f"p cnf {nv} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")
    return nv, clauses


def skip_toy(path):
    """Four variables in one at-most-one group (so capacity 1), a diagram
    needing two trues whose hi branch skips a level: root on x1 ->
    (x1 ? A : B), A on x3 (skips x2) -> (x3 ? T : x4), B on x2 -> (x2 ? C : D),
    C on x3 -> (x3 ? T : x4), D on x3 -> (x3 ? x4 : F)."""
    clauses = []
    for a, b in itertools.combinations([1, 2, 3, 4], 2):
        clauses.append([-a, -b])
    root, A, B, C, D = 5, 6, 7, 8, 9
    clauses += [[-root, -1, A], [-root, 1, B], [-root, A, B]]
    clauses += [[-A, 3, 4]]                 # hi = T dropped: lo = x4
    clauses += [[-B, -2, C], [-B, 2, D], [-B, C, D]]
    clauses += [[-C, 3, 4]]
    clauses += [[-D, 3], [-D, 4]]           # (~D ~3 4) subsumed by (~D 4)
    clauses.insert(0, [root])
    with open(path, "w") as f:
        f.write(f"p cnf 9 {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")
    return 9, clauses


# ----------------------------------------------------------------- check

def veripb_check(veripb, cnf, pbp, timeout=36000):
    t0 = time.time()
    p = subprocess.run([str(veripb), str(cnf), str(pbp)], capture_output=True, text=True, timeout=timeout)
    dt = time.time() - t0
    text = p.stdout + p.stderr
    ok = p.returncode == 0 and "s VERIFIED UNSATISFIABLE" in text and "FAIL" not in text.upper()
    return ok, dt, (p.stdout + p.stderr).strip().splitlines()[-3:]


def run_on(cnf_path, out_dir, veripb, label, expect=True, tamper=False, delete=True):
    nv, clauses = read_cnf(cnf_path)
    if str(cnf_path).endswith(".xz"):
        # VeriPB reads plain CNF only: the checks below use a decompressed copy
        plain = Path(out_dir) / (label + ".cnf")
        with lzma.open(cnf_path, "rb") as src, open(plain, "wb") as dst:
            dst.write(src.read())
        cnf_path = plain
    t0 = time.time()
    dd = detect(nv, clauses)
    t_detect = time.time() - t0
    if dd.reason:
        print(f"{label}: no diagram read: {dd.reason}")
        return not expect
    pbp = Path(out_dir) / (label + ".pbp")
    t0 = time.time()
    refuted, stats = write_proof(dd, nv, clauses, pbp, delete=delete)
    t_write = time.time() - t0
    print(f"{label}: nodes {len(dd.nodes)}, board {len(dd.order)}, groups {stats['groups']}, root val {stats['root_val']}, "
          f"fresh {stats['fresh']}, lines {stats['lines']}, {'refuted' if refuted else 'no refutation'}; "
          f"detect {t_detect:.2f} s, write {t_write:.2f} s, proof {pbp.stat().st_size / 1e6:.1f} MB")
    if not refuted:
        return not expect
    if not expect:
        print(f"{label}: REFUTED a case expected satisfiable (wrong answer)")
        return False
    ok, dt, tail = veripb_check(veripb, cnf_path, pbp)
    print(f"{label}: veripb {'ok' if ok else 'FAILED'} in {dt:.1f} s" + ("" if ok else f": {tail}"))
    if tamper and ok:
        text = pbp.read_text().splitlines()
        # strengthen one node's constraint: a degree one above what its
        # subproof shows (the node coefficient stays, so the claim is "one
        # more true below" than the subtree forces)
        for idx, line in enumerate(text):
            if line.startswith("pbc "):
                parts = line.split()
                ge = parts.index(">=")
                parts[ge + 1] = str(int(parts[ge + 1]) + 1)
                text[idx] = " ".join(parts)
                break
        bad = Path(out_dir) / (label + ".tampered.pbp")
        bad.write_text("\n".join(text) + "\n")
        okb, dtb, tailb = veripb_check(veripb, cnf_path, bad)
        print(f"{label}: tampered proof {'rejected' if not okb else 'ACCEPTED (bad)'} in {dtb:.1f} s")
        ok = ok and not okb
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["toy", "file", "selftest"])
    ap.add_argument("cnf", nargs="?")
    ap.add_argument("--n", type=int, default=4)
    ap.add_argument("--k", type=int, default=None)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--veripb", default=str(DEFAULT_VERIPB))
    ap.add_argument("--out", default=None)
    ap.add_argument("--no-delete", action="store_true")
    args = ap.parse_args()
    out = Path(args.out) if args.out else Path(os.environ.get("TMPDIR", "/tmp")) / "dd_toy_proof"
    out.mkdir(parents=True, exist_ok=True)
    if args.mode == "toy":
        k = args.k if args.k is not None else args.n + 1
        cnf = out / f"rook{args.n}_{k}_s{args.seed}.cnf"
        rook_toy(args.n, k, args.seed, cnf)
        ok = run_on(cnf, out, args.veripb, cnf.stem, expect=(k > args.n), tamper=True, delete=not args.no_delete)
        sys.exit(0 if ok else 1)
    if args.mode == "file":
        ok = run_on(args.cnf, out, args.veripb, Path(args.cnf).name.split(".")[0], delete=not args.no_delete)
        sys.exit(0 if ok else 1)
    # selftest
    results = []
    for n, k, seed in [(2, 3, 0), (3, 4, 0), (4, 5, 0), (4, 5, 1), (5, 6, 2), (6, 7, 3), (4, 4, 0), (5, 3, 1)]:
        cnf = out / f"rook{n}_{k}_s{seed}.cnf"
        rook_toy(n, k, seed, cnf)
        results.append(run_on(cnf, out, args.veripb, cnf.stem, expect=(k > n), tamper=(k > n)))
    cnf = out / "skip.cnf"
    skip_toy(cnf)
    results.append(run_on(cnf, out, args.veripb, "skip", expect=True, tamper=True))
    print(f"selftest: {sum(results)} of {len(results)} as expected")
    sys.exit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
