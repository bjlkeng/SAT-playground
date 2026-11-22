#!/usr/bin/env python3
"""Bel's Pyramid in the puzzle's own terms, for two constraint solvers (bead
SAT-playground-d96.12).  A yardstick, not a part of the solver: it asks whether any
known kind of search finds the models of the 2025 'bp' cells fast, and which kind.

The puzzle (https://github.com/acmihal/bel_pyramid, formulation.py): a step pyramid
of N layers, 2N-1 labels, one label per line of cubes (X per (x, h), Y per (y, h),
Z per (x, y)); the cube at (x, y, h) shows the three labels of its lines, and every
multiset of three labels must show at exactly one position.  The options are the
generator's strategies (strategies.py), under the tokens of the cells' file names:

  BC012  the bottom centre cube is (0,0,0), (0,0,1) or (0,1,2)
  TCO    the top cube's x label is at most its y label
  CSO    lex order between mirrored x lines and between mirrored y lines
  AM     BC012, TCO, CSO, and all x labels in lex order before all y labels
  IXA    the bottom layer's x labels never decrease
  LP     labels appear in first-use order (z labels row by row, then x, then y)
  LPI    the same over the centre lines only
  FPBEQ  face counts: each label shows on as many faces as it has in all cubes
  FPBLE  the same per axis, as an upper bound
  CB     higher layers use the smaller labels only
  ZR     the outer ring of z labels fixed
  YS     y label (y, h) fixed to y + h

Two solvers, both from Python packages (venv ~/.cache/sat13-cpsat/venv: ortools,
pychoco):

  cpsat   Google's CP-SAT: clause learning over integer variables, with a linear
          program beside it.  Kinds of model:
            table  one number per position (its cube), tied to the three labels by a
                   table, and "all different" over the positions
            dual   the same, plus one number per cube (its position), the two tied
                   as inverse of each other
            bool   the file's way: a placement variable for every (position, three
                   label values), each cube exactly one placement; nothing says a
                   position holds one cube
  choco   Choco: classic propagation and backtracking, no clause learning.  "All
          different" by matching (algo AC), tables, and searches that decide the
          variable with the smallest domain (weighted by its failures).

Commands:
  solve     one run; prints one result line (and the labels with --out)
  check     a solver's model of a cell (its v lines) against the rules written here
  units     the placements a labelling makes true, as unit clauses for the cell
  labels    the labelling a solver's model of a cell holds, as JSON
  cellwith  the cell plus a labelling's placements as unit clauses
  selftest  small puzzles: the model kinds and both solvers agree, the rule check
            catches a changed label, and at size 2 with the ring fixed every option's
            models are exactly those of the rule check tried on all labellings

The check runs both ways (bead d96.12): `check` on our solver's models of the cells,
and `units` + our solver on what these solvers find.
"""
from __future__ import annotations

import argparse
import contextlib
import io
import json
import lzma
import random
import sys
import time
from itertools import combinations_with_replacement, permutations, product
from pathlib import Path

TOKENS = ("BC012", "TCO", "CSO", "AM", "IXA", "LP", "LPI", "FPBEQ", "FPBLE", "CB", "ZR", "YS")
# CP-SAT: a lex order between two lists is written as one comparison of two numbers
# while those numbers stay below this; longer lists get a place-by-place form
LEX_AS_NUMBER = 1 << 60


class Puzzle:
    """The geometry, as the generator's Formulation has it."""

    def __init__(self, N):
        self.N = N
        self.L = L = 2 * N - 1                  # the base's length, and the number of labels
        self.th = th = [min(x + 1, L - x) for x in range(L)]
        self.pos = [(x, y, h) for x, y in product(range(L), range(L)) for h in range(min(th[x], th[y]))]
        self.xkeys = [("X", x, h) for x in range(L) for h in range(th[x])]
        self.ykeys = [("Y", y, h) for y in range(L) for h in range(th[y])]
        self.zkeys = [("Z", x, y) for y in range(L) for x in range(L)]      # row by row, as hvar_matrix
        self.keys = self.zkeys + self.xkeys + self.ykeys                    # the generator's flat order
        self.cubes = list(combinations_with_replacement(range(L), 3))
        self.cube_ix = {c: i for i, c in enumerate(self.cubes)}
        self.size = [2 * level + 1 for level in range(N)]                   # cubes along a side, top layer first
        self.faces_eq = sum(c.count(0) for c in self.cubes)                 # faces one label has over all cubes
        self.faces_le = sum(c.count(0) > 0 for c in self.cubes)             # cubes that carry one label
        assert len(self.pos) == len(self.cubes)

    def at(self, x, y, h):
        return ("X", x, h), ("Y", y, h), ("Z", x, y)

    def weight(self, key):
        """How many cubes a label's line holds."""
        if key[0] == "Z":
            return min(self.th[key[1]], self.th[key[2]])
        return self.size[-1 - key[2]]


def parse_opts(text):
    opts = [t for t in text.replace(",", "_").split("_") if t]
    for t in opts:
        if t not in TOKENS:
            sys.exit(f"unknown option {t!r}; known: {' '.join(TOKENS)}")
    return opts


def name_of(key):
    return "%s%d_%d" % key


# ---------------------------------------------------------------------------
# The rules, checked on a full labelling (independent of both solvers)
# ---------------------------------------------------------------------------

def _first_use_order(vals):
    top = -1
    for a in vals:
        if a > top + 1:
            return False
        top = max(top, a)
    return True


def violations(puz, opts, lab):
    """The rules a full labelling breaks; empty for a model of the puzzle with opts."""
    L, mid, N = puz.L, puz.L // 2, puz.N
    bad = []
    if any(not 0 <= lab[k] < L for k in puz.keys):
        return ["a label out of range"]
    seen = {}
    for (x, y, h) in puz.pos:
        c = tuple(sorted(lab[k] for k in puz.at(x, y, h)))
        if c in seen:
            bad.append(f"cube {c} at {seen[c]} and {(x, y, h)}")
        seen[c] = (x, y, h)
    X = [[lab[("X", x, h)] for h in range(puz.th[x])] for x in range(L)]
    Y = [[lab[("Y", y, h)] for h in range(puz.th[y])] for y in range(L)]

    def Z(x, y):
        return lab[("Z", x, y)]

    def bc012():
        return (X[mid][0], Y[mid][0], Z(mid, mid)) in ((0, 0, 0), (0, 0, 1), (0, 1, 2))

    def tco():
        return X[mid][-1] <= Y[mid][-1]

    def cso():      # list comparison is the generator's precedes(): lex, equal allowed
        return all(X[x] <= X[-x - 1] for x in range(mid)) and all(Y[y] <= Y[-y - 1] for y in range(mid))

    def am():
        return bc012() and tco() and cso() and [v for r in X for v in r] <= [v for r in Y for v in r]

    def ixa():
        return all(X[x][0] <= X[x + 1][0] for x in range(L - 1))

    def lp():
        return _first_use_order([lab[k] for k in puz.keys])

    def lpi():
        return _first_use_order([Z(mid, mid)] + [v for pair in zip(X[mid], Y[mid]) for v in pair])

    def fpbeq():
        return all(sum(puz.weight(k) for k in puz.keys if lab[k] == v) == puz.faces_eq for v in range(L))

    def fpble():
        return all(sum(puz.weight(k) for k in keys if lab[k] == v) <= puz.faces_le
                   for v in range(L) for keys in (puz.ykeys, puz.xkeys, puz.zkeys))

    def cb():
        for level in range(1, N):
            top = L - 2 * level
            if any(v >= top for r in X for v in r[level:]) or any(v >= top for r in Y for v in r[level:]):
                return False
            if any(Z(x, y) >= top for x in range(level, L - level) for y in range(level, L - level)):
                return False
        return True

    def zr():
        return (all(Z(0, y) == y and Z(L - 1, y) == y for y in range(L))
                and all(Z(x, 0) == 0 and Z(x, L - 1) == L - 1 for x in range(L)))

    def ys():
        return all(Y[y][h] == y + h for y in range(L) for h in range(puz.th[y]))

    rule = {"BC012": bc012, "TCO": tco, "CSO": cso, "AM": am, "IXA": ixa, "LP": lp, "LPI": lpi,
            "FPBEQ": fpbeq, "FPBLE": fpble, "CB": cb, "ZR": zr, "YS": ys}
    for t in opts:
        if not rule[t]():
            bad.append(t)
    return bad


# ---------------------------------------------------------------------------
# Between a labelling and the competition cell's placement variables
# ---------------------------------------------------------------------------

def placement_rows(puz):
    """rows[i] = (cube index, the three labels in x, y, z order, (x, y, h)) of the cell's
    variable i + 1: the generator numbers its placements c{cube}_r{rot}_x{x}_y{y}_h{h}
    in the string order of their names (tools/pyramid_read.py, checked on every cell)."""
    rows = []
    for ci, cube in enumerate(puz.cubes):
        for ri, rot in enumerate(sorted(set(permutations(cube)))):
            for (x, y, h) in puz.pos:
                rows.append((f"c{ci}_r{ri}_x{x}_y{y}_h{h}", ci, rot, (x, y, h)))
    rows.sort(key=lambda r: r[0])
    return rows


def labels_of_model(puz, true_vars):
    """The labelling a model of a cell holds, read from its true placements."""
    rows = placement_rows(puz)
    lab, at = {}, {}
    for v in true_vars:
        if v <= len(rows):
            _n, _ci, rot, (x, y, h) = rows[v - 1]
            if (x, y, h) in at:
                sys.exit(f"two placements are true at {(x, y, h)}")
            at[(x, y, h)] = rot
            for key, r in zip(puz.at(x, y, h), rot):
                if lab.setdefault(key, r) != r:
                    sys.exit(f"placements disagree on label {name_of(key)}")
    if len(at) != len(puz.pos):
        sys.exit(f"{len(at)} of {len(puz.pos)} positions hold a true placement")
    return lab


def units_of_labels(puz, lab):
    """The cell's placement variables a labelling makes true."""
    index = {(r[1], r[2], r[3]): i + 1 for i, r in enumerate(placement_rows(puz))}
    out = []
    for (x, y, h) in puz.pos:
        rot = tuple(lab[k] for k in puz.at(x, y, h))
        out.append(index[(puz.cube_ix[tuple(sorted(rot))], rot, (x, y, h))])
    return out


def read_model(path):
    opener = lzma.open if str(path).endswith(".xz") else open
    true_vars = []
    status = None
    with opener(path, "rt") as f:
        for line in f:
            if line.startswith("s "):
                status = line.split()[1]
            elif line.startswith("v "):
                true_vars += [int(t) for t in line.split()[1:] if int(t) > 0]
    return status, true_vars


# ---------------------------------------------------------------------------
# CP-SAT
# ---------------------------------------------------------------------------

def build_cpsat(puz, opts, kind):
    from ortools.sat.python import cp_model
    m = cp_model.CpModel()
    L, mid, N = puz.L, puz.L // 2, puz.N
    var = {k: m.NewIntVar(0, L - 1, name_of(k)) for k in puz.keys}
    lits = {}

    def is_(k, a):
        if (k, a) not in lits:
            b = m.NewBoolVar(f"{name_of(k)}={a}")
            m.Add(var[k] == a).OnlyEnforceIf(b)
            m.Add(var[k] != a).OnlyEnforceIf(b.Not())
            lits[(k, a)] = b
        return lits[(k, a)]

    cube = dual = None
    if kind in ("table", "dual"):
        rows = [(a, b, c, puz.cube_ix[tuple(sorted((a, b, c)))]) for a, b, c in product(range(L), repeat=3)]
        cube = []
        for (x, y, h) in puz.pos:
            c = m.NewIntVar(0, len(puz.cubes) - 1, f"cube_{x}_{y}_{h}")
            m.AddAllowedAssignments([var[k] for k in puz.at(x, y, h)] + [c], rows)
            cube.append(c)
        if kind == "dual":
            dual = [m.NewIntVar(0, len(puz.pos) - 1, f"where_{i}") for i in range(len(puz.cubes))]
            m.AddInverse(cube, dual)
        else:
            m.AddAllDifferent(cube)
    elif kind == "bool":
        places = [[] for _ in puz.cubes]
        for (x, y, h) in puz.pos:
            kx, ky, kz = puz.at(x, y, h)
            for a, b, c in product(range(L), repeat=3):
                p = m.NewBoolVar("")
                ls = [is_(kx, a), is_(ky, b), is_(kz, c)]
                m.AddBoolAnd(ls).OnlyEnforceIf(p)
                m.AddBoolOr([l.Not() for l in ls] + [p])
                places[puz.cube_ix[tuple(sorted((a, b, c)))]].append(p)
        for ps in places:
            m.AddExactlyOne(ps)
    else:
        sys.exit(f"unknown kind {kind!r}")

    X = [[var[("X", x, h)] for h in range(puz.th[x])] for x in range(L)]
    Y = [[var[("Y", y, h)] for h in range(puz.th[y])] for y in range(L)]

    def Z(x, y):
        return var[("Z", x, y)]

    def lex_le(a, b):
        """a before b in lex order, equal allowed; the lists have equal lengths."""
        n = len(a)
        if L ** n < LEX_AS_NUMBER:
            # the lists as numbers in base L
            m.Add(sum(a[i] * L ** (n - 1 - i) for i in range(n)) <= sum(b[i] * L ** (n - 1 - i) for i in range(n)))
            return
        # too long for one 64-bit number (all labels of a size-5 puzzle, option AM):
        # same[i] says the lists agree before place i; while they agree, a[i] <= b[i]
        same = None
        for i in range(n):
            c = m.Add(a[i] <= b[i])
            if same is not None:
                c.OnlyEnforceIf(same)
            if i == n - 1:
                break
            eq = m.NewBoolVar("")
            m.Add(a[i] == b[i]).OnlyEnforceIf(eq)
            m.Add(a[i] != b[i]).OnlyEnforceIf(eq.Not())
            if same is None:
                same = eq
            else:
                nxt = m.NewBoolVar("")
                m.AddBoolAnd([same, eq]).OnlyEnforceIf(nxt)
                m.AddBoolOr([same.Not(), eq.Not(), nxt])
                same = nxt

    def first_use_order(vs):
        top = None
        for a in vs:
            if top is None:
                m.Add(a == 0)
                top = a
            else:
                m.Add(a <= top + 1)
                nxt = m.NewIntVar(0, L - 1, "")
                m.AddMaxEquality(nxt, [top, a])
                top = nxt

    def bc012():
        m.AddAllowedAssignments([X[mid][0], Y[mid][0], Z(mid, mid)], [(0, 0, 0), (0, 0, 1), (0, 1, 2)])

    def tco():
        m.Add(X[mid][-1] <= Y[mid][-1])

    def cso():
        for x in range(mid):
            lex_le(X[x], X[-x - 1])
        for y in range(mid):
            lex_le(Y[y], Y[-y - 1])

    def am():
        bc012()
        tco()
        cso()
        lex_le([v for r in X for v in r], [v for r in Y for v in r])

    def ixa():
        for x in range(L - 1):
            m.Add(X[x][0] <= X[x + 1][0])

    def lp():
        first_use_order([var[k] for k in puz.keys])

    def lpi():
        first_use_order([Z(mid, mid)] + [v for pair in zip(X[mid], Y[mid]) for v in pair])

    def fpbeq():
        for v in range(L):
            m.Add(sum(puz.weight(k) * is_(k, v) for k in puz.keys) == puz.faces_eq)

    def fpble():
        for v in range(L):
            for keys in (puz.ykeys, puz.xkeys, puz.zkeys):
                m.Add(sum(puz.weight(k) * is_(k, v) for k in keys) <= puz.faces_le)

    def cb():
        for level in range(1, N):
            top = L - 2 * level
            for r in X + Y:
                for v in r[level:]:
                    m.Add(v < top)
            for x in range(level, L - level):
                for y in range(level, L - level):
                    m.Add(Z(x, y) < top)

    def zr():
        for y in range(L):
            m.Add(Z(0, y) == y)
            m.Add(Z(L - 1, y) == y)
        for x in range(L):
            m.Add(Z(x, 0) == 0)
            m.Add(Z(x, L - 1) == L - 1)

    def ys():
        for y in range(L):
            for h in range(puz.th[y]):
                m.Add(Y[y][h] == y + h)

    rule = {"BC012": bc012, "TCO": tco, "CSO": cso, "AM": am, "IXA": ixa, "LP": lp, "LPI": lpi,
            "FPBEQ": fpbeq, "FPBLE": fpble, "CB": cb, "ZR": zr, "YS": ys}
    for t in opts:
        rule[t]()
    return m, var, cube, dual


def solve_cpsat(puz, opts, a):
    from ortools.sat.python import cp_model
    kind = a.kind or ("dual" if a.search == "cubes" else "table")
    m, var, cube, dual = build_cpsat(puz, opts, kind)
    for k, v in a.fixed.items():
        m.Add(var[k] == v)
    s = cp_model.CpSolver()
    p = s.parameters
    p.num_workers = a.workers
    p.random_seed = a.seed
    p.max_time_in_seconds = a.seconds
    p.log_search_progress = bool(a.log)
    if a.lin is not None:
        p.linearization_level = a.lin
    if a.interleave:
        p.interleave_search = True
    for name in a.subsolver or []:
        p.subsolvers.append(name)
    for name in a.ignore or []:
        p.ignore_subsolvers.append(name)
    for text in a.param or []:              # any other parameter, name=value
        key, val = text.split("=", 1)
        cur = getattr(p, key)
        setattr(p, key, type(cur)(int(val)) if isinstance(cur, bool) else type(cur)(val))
    order = {"labels": list(var.values()), "cells": cube, "cubes": dual}
    if a.search in order:
        if order[a.search] is None:
            sys.exit(f"--search {a.search} needs another kind of model than {kind}")
        vs = list(order[a.search])
        random.Random(a.seed).shuffle(vs)           # the seed breaks ties between equal domains
        m.AddDecisionStrategy(vs, cp_model.CHOOSE_MIN_DOMAIN_SIZE, cp_model.SELECT_MIN_VALUE)
        p.search_branching = cp_model.FIXED_SEARCH
    elif a.search != "auto":
        p.search_branching = getattr(cp_model, a.search.upper())
    t0 = time.time()
    status = s.Solve(m)
    secs = time.time() - t0
    name = s.StatusName(status)
    if name not in ("OPTIMAL", "FEASIBLE", "INFEASIBLE", "UNKNOWN"):
        # MODEL_INVALID: CP-SAT refused the model or a parameter (an unknown subsolver
        # name, say).  That is a broken run, not a search that found nothing.
        sys.exit(f"CP-SAT did not run: {name} {s.SolutionInfo()}")
    if name == "UNKNOWN" and secs < 0.5 * a.seconds and not a.param:
        sys.exit(f"CP-SAT stopped after {secs:.1f} s of {a.seconds:g} without an answer and without a limit of ours")
    lab = {k: s.Value(v) for k, v in var.items()} if name in ("OPTIMAL", "FEASIBLE") else None
    info = {"branches": s.NumBranches(), "conflicts": s.NumConflicts(), "found_by": s.SolutionInfo() if lab else ""}
    return ("SAT" if lab else "UNSAT" if name == "INFEASIBLE" else "UNKNOWN"), secs, lab, info, f"cpsat/{kind}"


# ---------------------------------------------------------------------------
# Choco
# ---------------------------------------------------------------------------

def solve_choco(puz, opts, a):
    from pychoco import Model
    rng = random.Random(a.seed)
    m = Model("pyramid")
    L, mid, N = puz.L, puz.L // 2, puz.N
    var = {k: m.intvar(0, L - 1, name_of(k)) for k in puz.keys}
    kind = "dual" if a.kind == "dual" or a.search_on in ("cubes", "mixed") else "table"
    if a.kind == "bool":
        sys.exit("choco: kinds table and dual only")
    # The numbering of cubes, of positions and of the label triples means nothing: the
    # seed shuffles all three, which changes the order of values a search tries.
    cube_no = list(range(len(puz.cubes)))
    pos_no = list(range(len(puz.pos)))
    trip_no = list(range(L ** 3))
    if a.seed:
        rng.shuffle(cube_no)
        rng.shuffle(pos_no)
        rng.shuffle(trip_no)
    # one table a position: its three labels, which triple that is, and which cube
    rows = [[x, y, z, trip_no[(x * L + y) * L + z], cube_no[puz.cube_ix[tuple(sorted((x, y, z)))]]]
            for x, y, z in product(range(L), repeat=3)]
    cube = [None] * len(puz.pos)
    trip = []
    for i, (x, y, h) in enumerate(puz.pos):
        c = m.intvar(0, len(puz.cubes) - 1, f"cube_{x}_{y}_{h}")
        t = m.intvar(0, L ** 3 - 1, f"trip_{x}_{y}_{h}")
        m.table([var[k] for k in puz.at(x, y, h)] + [t, c], rows, True, a.table).post()
        cube[pos_no[i]] = c
        trip.append(t)
    m.all_different(cube, a.alldiff).post()
    dual = None
    if kind == "dual":
        dual = [m.intvar(0, len(puz.pos) - 1, f"where_{i}") for i in range(len(puz.cubes))]
        m.inverse_channeling(cube, dual, 0, 0, True).post()

    X = [[var[("X", x, h)] for h in range(puz.th[x])] for x in range(L)]
    Y = [[var[("Y", y, h)] for h in range(puz.th[y])] for y in range(L)]

    def Z(x, y):
        return var[("Z", x, y)]

    def first_use_order(vs):
        top = None
        for v in vs:
            if top is None:
                m.arithm(v, "=", 0).post()
                top = v
            else:
                m.arithm(v, "-", top, "<=", 1).post()
                nxt = m.intvar(0, L - 1)
                m.max(nxt, [top, v]).post()
                top = nxt

    def weighted(keys, v, op, rhs):
        bools = [m.int_eq_view(var[k], v) for k in keys]
        m.scalar(bools, [puz.weight(k) for k in keys], op, rhs).post()

    def bc012():
        m.table([X[mid][0], Y[mid][0], Z(mid, mid)], [[0, 0, 0], [0, 0, 1], [0, 1, 2]]).post()

    def tco():
        m.arithm(X[mid][-1], "<=", Y[mid][-1]).post()

    def cso():
        for x in range(mid):
            m.lex_less_eq(X[x], X[-x - 1]).post()
        for y in range(mid):
            m.lex_less_eq(Y[y], Y[-y - 1]).post()

    def am():
        bc012()
        tco()
        cso()
        m.lex_less_eq([v for r in X for v in r], [v for r in Y for v in r]).post()

    def ixa():
        for x in range(L - 1):
            m.arithm(X[x][0], "<=", X[x + 1][0]).post()

    def cb():
        for level in range(1, N):
            top = L - 2 * level
            for r in X + Y:
                for v in r[level:]:
                    m.arithm(v, "<", top).post()
            for x in range(level, L - level):
                for y in range(level, L - level):
                    m.arithm(Z(x, y), "<", top).post()

    def zr():
        for y in range(L):
            m.arithm(Z(0, y), "=", y).post()
            m.arithm(Z(L - 1, y), "=", y).post()
        for x in range(L):
            m.arithm(Z(x, 0), "=", 0).post()
            m.arithm(Z(x, L - 1), "=", L - 1).post()

    def ys():
        for y in range(L):
            for h in range(puz.th[y]):
                m.arithm(Y[y][h], "=", y + h).post()

    rule = {"BC012": bc012, "TCO": tco, "CSO": cso, "AM": am, "IXA": ixa,
            "LP": lambda: first_use_order([var[k] for k in puz.keys]),
            "LPI": lambda: first_use_order([Z(mid, mid)] + [v for pair in zip(X[mid], Y[mid]) for v in pair]),
            "FPBEQ": lambda: [weighted(puz.keys, v, "=", puz.faces_eq) for v in range(L)],
            "FPBLE": lambda: [weighted(keys, v, "<=", puz.faces_le) for v in range(L)
                              for keys in (puz.ykeys, puz.xkeys, puz.zkeys)],
            "CB": cb, "ZR": zr, "YS": ys}
    for t in opts:
        rule[t]()
    for k, v in a.fixed.items():
        m.arithm(var[k], "=", v).post()

    s = m.get_solver()
    labels = list(var.values())
    rng.shuffle(labels)                 # the seed breaks ties between equal domains
    # every search ends with all labels fixed: "cells" decides a position's triple of
    # labels, "mixed" takes cubes and labels in one list, "cubes" is a rule of our own
    on = {"labels": labels, "cells": list(trip), "mixed": (dual or []) + labels, "cubes": None}[a.search_on]
    if a.search == "auto":
        a.search = "domwdeg"
    setter = {"domwdeg": s.set_dom_over_w_deg_search, "domwdegref": s.set_dom_over_w_deg_ref_search,
              "mindom": s.set_min_dom_lb_search, "activity": s.set_activity_based_search,
              "chs": s.set_conflict_history_search, "pickondom": s.set_pick_on_dom_search,
              "frba": s.set_failure_rate_based_search, "input": s.set_input_order_lb_search}
    if a.search_on == "cubes":
        # the tightest thing first, to the letter: the cube with the fewest positions
        # left, at one of them; when every cube sits, the labels still open (a cube's
        # position does not say which way round it lies).  Python calls, so slow a
        # node; read the node count, not the seconds.
        def pick(_vars):
            best, size = None, 1 << 30
            for group in (dual, labels):
                for v in group:
                    if not v.is_instantiated():
                        n = len(v.get_domain_values())
                        if n < size:
                            best, size = v, n
                if best is not None:
                    return best
            return None

        def value(v):
            vals = v.get_domain_values()
            return rng.choice(vals) if a.seed else vals[0]

        s.set_custom_search(dual + labels, pick, value)
        a.search = "tightest"
    else:
        with contextlib.redirect_stdout(io.StringIO()):     # pychoco 0.4.0 prints debug lines here
            if a.search == "random":
                s.set_random_search(*on, seed=a.seed + 1)
            elif a.search in setter:
                setter[a.search](*on)
            else:
                sys.exit(f"choco: unknown search {a.search!r}")
    if a.restart == "luby":
        # pychoco 0.4.0's own set_luby_restart() passes too few arguments to its
        # library; the library's call takes a counter of failures, the scale of the
        # Luby sequence and the number of restarts allowed
        from pychoco import backend
        backend.set_luby_restart(s._handle, backend.fail_counter(m._handle, 0), a.restart_base, 1 << 30)
    if a.nogoods:
        # the wrapper's own call lacks the list of variables its library wants
        from pychoco import backend
        from pychoco._utils import make_intvar_array
        backend.set_nogood_recording_from_restarts(s._handle, make_intvar_array((dual or []) + labels if a.search_on == "cubes" else on))
    t0 = time.time()
    if a.seconds < 0.001:
        sys.exit("choco: --seconds must be at least 0.001")
    sol = s.find_solution(time_limit=f"{a.seconds:.3f}s")       # "0.500s" is taken as half a second
    secs = time.time() - t0
    lab = {k: sol.get_int_val(v) for k, v in var.items()} if sol is not None else None
    info = {"fails": s.get_fail_count(), "restarts": s.get_restart_count()}     # pychoco 0.4.0's node count returns the time
    ended = s.get_search_state() == "TERMINATED"        # STOPPED: cut by the time limit
    return ("SAT" if lab else "UNSAT" if ended else "UNKNOWN"), secs, lab, info, f"choco/{kind}/{a.search_on}"


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

def cmd_solve(a):
    puz = Puzzle(a.n)
    opts = parse_opts(a.opts)
    a.fixed = {}
    if a.keep:                          # some labels of a known model, kept as they are
        known = labels_from_json(puz, a.keep)
        keys = sorted(puz.keys)
        random.Random(a.seed).shuffle(keys)
        a.fixed = {k: known[k] for k in keys[:a.keep_count]}
    status, secs, lab, info, how = (solve_cpsat if a.solver == "cpsat" else solve_choco)(puz, opts, a)
    if lab is not None:
        bad = violations(puz, opts, lab)
        if bad:
            sys.exit(f"the solver's labelling breaks the rules: {bad}")
        if a.out:
            Path(a.out).write_text(json.dumps({name_of(k): v for k, v in lab.items()}, sort_keys=True) + "\n")
    extra = " ".join(f"{k}={v}" for k, v in info.items())
    print(f"result n={a.n} opts={'_'.join(opts) or '-'} how={how} search={a.search} seed={a.seed} "
          f"status={status} seconds={secs:.2f} {extra}", flush=True)


def labels_from_json(puz, path):
    raw = json.loads(Path(path).read_text())
    return {k: raw[name_of(k)] for k in puz.keys}


def cmd_check(a):
    puz = Puzzle(a.n)
    opts = parse_opts(a.opts)
    worst = 0
    for path in a.models:
        status, true_vars = read_model(path)
        if status != "SATISFIABLE":
            print(f"{path}: {status}, no model")
            continue
        bad = violations(puz, opts, labels_of_model(puz, true_vars))
        print(f"{path}: {'ok' if not bad else 'BROKEN ' + '; '.join(bad)}")
        worst |= bool(bad)
    sys.exit(1 if worst else 0)


def cmd_units(a):
    puz = Puzzle(a.n)
    for v in units_of_labels(puz, labels_from_json(puz, a.labels)):
        print(f"{v} 0")


def cmd_labels(a):
    puz = Puzzle(a.n)
    status, true_vars = read_model(a.model)
    if status != "SATISFIABLE":
        sys.exit(f"{a.model}: {status}, no model")
    lab = labels_of_model(puz, true_vars)
    Path(a.out).write_text(json.dumps({name_of(k): v for k, v in lab.items()}, sort_keys=True) + "\n")


def cmd_cellwith(a):
    """The cell plus the labelling's placements as unit clauses: our solver must
    call it satisfiable if the labelling is a model of the cell."""
    puz = Puzzle(a.n)
    units = units_of_labels(puz, labels_from_json(puz, a.labels))
    opener = lzma.open if a.cell.endswith(".xz") else open
    with opener(a.cell, "rt") as src, open(a.out, "w") as dst:
        for line in src:
            if line.startswith("p cnf"):
                _p, _c, nv, nc = line.split()
                dst.write(f"p cnf {nv} {int(nc) + len(units)}\n")
            else:
                dst.write(line)
        for v in units:
            dst.write(f"{v} 0\n")


def selftest():
    n_ok = 0

    def ok(cond, what):
        nonlocal n_ok
        if not cond:
            sys.exit(f"selftest FAILED: {what}")
        n_ok += 1

    # geometry against the generator's numbers
    p4 = Puzzle(4)
    ok((p4.L, len(p4.pos), len(p4.xkeys), len(p4.zkeys), p4.faces_eq, p4.faces_le) == (7, 84, 16, 49, 36, 28), "size 4")
    ok(sum(p4.weight(k) for k in p4.keys) == 3 * 84, "every cube is on three lines")
    p5 = Puzzle(5)
    ok((p5.L, len(p5.pos)) == (9, 165), "size 5")
    ok(len(placement_rows(p4)) == 343 * 84, "placements of size 4")

    def args(**kw):
        base = dict(solver="cpsat", kind=None, search="auto", workers=1, seed=0, seconds=60, lin=None,
                    interleave=False, subsolver=None, ignore=None, param=None, log=False, search_on="labels",
                    table="CT+", alldiff="AC", restart="luby", restart_base=100, nogoods=False, fixed={})
        base.update(kw)
        return argparse.Namespace(**base)

    # size 2 and 3, bare and with options: every solver and kind finds a model the rule
    # check accepts, and placements round-trip
    for n, opts in ((2, []), (3, []), (3, ["ZR"]), (3, ["CSO", "IXA", "TCO"]), (3, ["LP"]), (3, ["FPBEQ", "FPBLE"]),
                    (3, ["BC012", "AM"]), (3, ["LPI", "YS"])):
        puz = Puzzle(n)
        got = []
        for kw in (dict(kind="table"), dict(kind="dual"), dict(kind="bool"), dict(kind="dual", search="cubes"),
                   dict(search="labels", seconds=10), dict(solver="choco"), dict(solver="choco", search_on="cubes"),
                   dict(solver="choco", search="mindom", search_on="cells"),
                   dict(solver="choco", search="mindom", search_on="mixed", seed=3)):
            status, _secs, lab, _info, how = (solve_cpsat if kw.get("solver", "cpsat") == "cpsat" else solve_choco)(
                puz, opts, args(**kw))
            got.append(status)
            if lab is not None:
                ok(not violations(puz, opts, lab), f"n={n} {opts} {how}: the labelling breaks a rule")
                units = units_of_labels(puz, lab)
                ok(labels_of_model(puz, units) == lab, "placements round-trip")
                # a changed label is caught
                k = puz.keys[len(puz.keys) // 2]
                worse = dict(lab)
                worse[k] = (lab[k] + 1) % puz.L
                ok(violations(puz, opts, worse), "a changed label is caught")
        # a search may run out of time (CP-SAT's fixed order on the labels does, at
        # size 3); those that end must agree, and the plain ones must end
        ended = set(got) - {"UNKNOWN"}
        ok(len(ended) == 1 and got[0] != "UNKNOWN" and got[5] != "UNKNOWN", f"n={n} {opts}: the solvers disagree: {got}")
    # Every option at size 2 with the ring fixed, where all 19,683 labellings can be
    # tried: the models by the rule check are exactly the models CP-SAT enumerates,
    # for each kind of model, and Choco says "a model" or "none" as they do.
    from ortools.sat.python import cp_model

    def enumerate_models(puz, opts, kind, keys):
        m, var, _c, _d = build_cpsat(puz, opts, kind)

        class Collect(cp_model.CpSolverSolutionCallback):
            def __init__(self):
                super().__init__()
                self.seen = set()

            def on_solution_callback(self):
                self.seen.add(tuple(self.Value(var[k]) for k in keys))

        cb = Collect()
        sv = cp_model.CpSolver()
        sv.parameters.enumerate_all_solutions = True
        sv.parameters.num_workers = 1
        sv.Solve(m, cb)
        return cb.seen

    puz = Puzzle(2)
    L = puz.L
    ring = {}
    for i in range(L):
        ring[("Z", 0, i)] = ring[("Z", L - 1, i)] = i
    for i in range(L):
        ring[("Z", i, 0)], ring[("Z", i, L - 1)] = 0, L - 1
    free = [k for k in puz.keys if k not in ring]
    ok(len(free) == 9, "size 2: nine labels are free with the ring fixed")
    some, none = 0, 0
    for opts in (["ZR"], ["ZR", "YS"], ["ZR", "LP"], ["ZR", "LPI"], ["ZR", "TCO", "CSO"], ["ZR", "IXA"], ["ZR", "BC012"],
                 ["ZR", "AM"], ["ZR", "FPBEQ"], ["ZR", "FPBLE"], ["ZR", "CB"], ["ZR", "BC012", "IXA", "YS"]):
        want = set()
        for t in product(range(L), repeat=len(free)):
            lab = dict(ring)
            lab.update(zip(free, t))
            if not violations(puz, opts, lab):
                want.add(t)
        for kind in ("table", "dual", "bool"):
            ok(enumerate_models(puz, opts, kind, free) == want, f"size 2 {opts} {kind}: CP-SAT's models are not the rule check's")
        ok(solve_choco(puz, opts, args(solver="choco"))[0] == ("SAT" if want else "UNSAT"), f"size 2 {opts}: Choco's answer")
        some += bool(want)
        none += not want
    ok(some >= 4 and none >= 1, f"size 2 with the ring: {some} option sets with a model, {none} without")
    # the place-by-place form of the lex order (used when the lists are too long for
    # one number) gives the same models
    global LEX_AS_NUMBER
    keep, LEX_AS_NUMBER = LEX_AS_NUMBER, 1
    try:
        for opts in (["ZR", "TCO", "CSO"], ["ZR", "AM"], ["ZR", "CSO"]):
            want = set()
            for t in product(range(L), repeat=len(free)):
                lab = dict(ring)
                lab.update(zip(free, t))
                if not violations(puz, opts, lab):
                    want.add(t)
            ok(enumerate_models(puz, opts, "table", free) == want, f"size 2 {opts}: the place-by-place lex order")
    finally:
        LEX_AS_NUMBER = keep
    # size 5 with AM builds (25 labels in one lex order: too long for one number)
    build_cpsat(Puzzle(5), ["AM"], "table")
    n_ok += 1
    # a setting CP-SAT refuses ends the run with an error, not with "unknown"
    try:
        solve_cpsat(Puzzle(2), [], args(subsolver=["no_such_subsolver"], seconds=5))
        ok(False, "a refused CP-SAT setting must be an error")
    except SystemExit as e:
        ok("did not run" in str(e) or "stopped after" in str(e), f"a refused CP-SAT setting: {e}")
    # Choco takes a limit below one second
    ok(solve_choco(Puzzle(2), [], args(solver="choco", seconds=0.5))[0] == "SAT", "choco with half a second")
    # the number of models agrees between the kinds (size 2 with the label order fixed)
    counts = []
    for kind in ("table", "dual", "bool"):
        seen = enumerate_models(puz, ["LP", "TCO"], kind, puz.keys)
        ok(all(not violations(puz, ["LP", "TCO"], dict(zip(puz.keys, t))) for t in seen), "an enumerated model breaks a rule")
        counts.append(len(seen))
    ok(len(set(counts)) == 1 and counts[0] > 0, f"model counts differ between kinds: {counts}")
    print(f"selftest: {n_ok} checks, all ok (size 2 with LP and TCO has {counts[0]} models; "
          f"with the ring fixed, {some} option sets with a model and {none} without)")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("selftest")
    s = sub.add_parser("solve")
    s.add_argument("--n", type=int, default=4)
    s.add_argument("--opts", default="", help="tokens joined by _ or comma, e.g. CSO_IXA_ZR")
    s.add_argument("--solver", choices=("cpsat", "choco"), default="cpsat")
    s.add_argument("--kind", choices=("table", "dual", "bool"))
    s.add_argument("--search", default="auto",
                   help="cpsat: auto, labels, cells, cubes (smallest domain first), or a search_branching name; "
                        "choco: domwdeg, domwdegref, mindom, activity, chs, pickondom, frba, input, random")
    s.add_argument("--search-on", choices=("labels", "cells", "mixed", "cubes"), default="labels",
                   help="choco: what to decide: labels; cells (a position's triple of labels); mixed (cubes "
                        "and labels in one list); cubes (the cube with the fewest positions left, then labels)")
    s.add_argument("--workers", type=int, default=1)
    s.add_argument("--seed", type=int, default=0)
    s.add_argument("--seconds", type=float, default=600)
    s.add_argument("--lin", type=int, help="cpsat: linearization_level 0, 1 or 2")
    s.add_argument("--interleave", action="store_true", help="cpsat: all methods in turn on the workers given")
    s.add_argument("--subsolver", action="append")
    s.add_argument("--ignore", action="append")
    s.add_argument("--param", action="append", help="cpsat: any other parameter, name=value")
    s.add_argument("--log", action="store_true")
    s.add_argument("--table", default="CT+", help="choco: table algorithm")
    s.add_argument("--alldiff", default="AC", help="choco: all-different algorithm (AC, BC, FC, AC_ZHANG, ...)")
    s.add_argument("--restart", choices=("luby", "none"), default="luby", help="choco: restarts by the Luby sequence")
    s.add_argument("--restart-base", type=int, default=100)
    s.add_argument("--nogoods", action="store_true", help="choco: record nogoods from restarts")
    s.add_argument("--out", help="write the labels found, as JSON")
    s.add_argument("--keep", help="labels of a known model (JSON); --keep-count of them, picked by the seed, are fixed")
    s.add_argument("--keep-count", type=int, default=0)
    c = sub.add_parser("check")
    c.add_argument("--n", type=int, default=4)
    c.add_argument("--opts", default="")
    c.add_argument("models", nargs="+")
    u = sub.add_parser("units")
    u.add_argument("--n", type=int, default=4)
    u.add_argument("labels")
    lb = sub.add_parser("labels")
    lb.add_argument("--n", type=int, default=4)
    lb.add_argument("model", help="a solver's output with v lines")
    lb.add_argument("out")
    cw = sub.add_parser("cellwith")
    cw.add_argument("--n", type=int, default=4)
    cw.add_argument("cell")
    cw.add_argument("labels")
    cw.add_argument("out")
    a = ap.parse_args(argv)
    {"selftest": lambda _a: selftest(), "solve": cmd_solve, "check": cmd_check, "units": cmd_units,
     "labels": cmd_labels, "cellwith": cmd_cellwith}[a.cmd](a)


if __name__ == "__main__":
    main()
