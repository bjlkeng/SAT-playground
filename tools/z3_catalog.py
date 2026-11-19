#!/usr/bin/env python3
"""z3_catalog.py - the Z3 encoding catalog (bead SAT-playground-d96.9.1).

Why.  The 20 Bel's Pyramid cells of 2025 were written by Z3: a model over
integers, exported to clauses with Z3's own steps.  An integer becomes sorted
bits, a test such as "x = 2" becomes a count over those bits, and unit
propagation (the solver's cheap rule) then runs from the bits to the test and
not back.  To read such formulas in general we need to know every way Z3
writes a number, a test, a sum, a count or a choice.  Z3 can tell us itself:
this tool builds one toy per primitive and per encoding option, with Z3, and
keeps for each toy the clauses and what they mean.

A small example, the toy `int-eq-iff-0_3` in the base setting.  One integer x
in 0..3 and four tests T_c <-> (x = c).  Z3 writes x as three sorted bits
(g1 = x >= 1, g2 = x >= 2, g3 = x >= 3, with g2 -> g1 and g3 -> g2), so the
legal patterns are 000, 100, 110, 111.  The record says: input x, order kind,
variables [g1, g2, g3], one pattern per value; test T_2 with the table 0010.
The check takes each of the four patterns, sets the bits, and asks a small
complete search whether T_2 can be anything but its table entry (it cannot),
and whether any bit pattern outside the four has a model (none has).  The
shape columns then say what propagation does: from the bits to T_2 it always
arrives (fwd 4 of 4); from "T_2 is true" it should set g1, g2 and not g3 (need
3) and sets none of them (up 0); each of the three missing two-literal clauses
is found by propagation when its opposite is assumed (rup 3).

What is in a record (one JSON object per line of the catalog):
  name, primitive, options   the toy, and every option set that gives exactly
                             these clauses
  n_vars, clauses, names     the formula as Z3 wrote it, with Z3's names
  inputs                     what is free: a sorted-bit number (kind order),
                             a binary-coded choice (kind binary), a Boolean;
                             each with its variables and one pattern (a list
                             of literals) per value
  tests                      named Booleans and what they say about the
                             inputs: a table with one character per joint
                             value of the inputs (first input slowest).  Form
                             iff is T <-> test, form imp is T -> test
  hard                       which joint values the toy's own constraints
                             allow (left out when all are allowed)

The shape table (one row per record) counts, from the tables alone, what the
inputs' literals must be in each situation, and how much of that propagation
finds:
  fwd      every input set to a value: is each test then set
  0        a test alone: the input literals it forces
  1        a test and one input literal: what follows only from both
  2        a test and two input literals: what follows only from all three
  h1, h2   the same for the toy's hard constraints, with no test
For each: need (literals that follow), up (propagation sets them), rup (not
set, but the missing clause is checkable by propagation as it stands).

Commands:
  build [--out FILE] [--only SUBSTR] [--jobs N]   make the catalog; needs Z3:
        uv venv ~/.cache/sat13-z3/venv --python 3.12
        uv pip install --python ~/.cache/sat13-z3/venv/bin/python -r tools/z3/requirements.txt
        ~/.cache/sat13-z3/venv/bin/python tools/z3_catalog.py build
  check [FILE] [--jobs N]        no Z3: every record's meaning by brute force,
                                 and its shape row against the stored one
  table [FILE] [--out TSV]       write the shape table
  list [FILE]                    one line per record
  summary [FILE]                 the shape numbers summed per primitive
  emit NAME [--out X.cnf]        one toy as a DIMACS file
  selftest                       no Z3: the checker on hand-made records,
                                 right ones and wrong ones

Z3 is a tool here, never a dependency of the solver.
"""
from __future__ import annotations

import argparse
import itertools
import json
import lzma
import multiprocessing
import os
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from cnf_prop import Prop, satisfiable  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "benchmarks" / "rl" / "z3_catalog.jsonl.xz"
SHAPES = ROOT / "benchmarks" / "rl" / "z3_catalog_shapes.tsv"
NOTES = ROOT / "benchmarks" / "rl" / "z3_catalog_notes.txt"

z3 = None       # imported by need_z3()


def need_z3():
    global z3
    if z3 is None:
        try:
            import z3 as _z3
        except ImportError:
            sys.exit("this command needs Z3 (see the docstring: a venv with tools/z3/requirements.txt)")
        z3 = _z3
    return z3


# ---------------------------------------------------------------------------
# expressions that work on Z3 terms and on plain Python values alike, so one
# definition gives both the Z3 model and the table it is checked against
# ---------------------------------------------------------------------------

def _is_z3(x):
    return hasattr(x, "sexpr")


def AND(*a):
    return z3.And(*a) if any(_is_z3(x) for x in a) else all(a)


def OR(*a):
    return z3.Or(*a) if any(_is_z3(x) for x in a) else any(a)


def NOT(a):
    return z3.Not(a) if _is_z3(a) else not a


def ITE(c, a, b):
    return z3.If(c, a, b) if any(_is_z3(x) for x in (c, a, b)) else (a if c else b)


def IS(e, j):
    """Enum input `e` has its value number j."""
    return e == e.sort().constructor(j)() if _is_z3(e) else e == j


def ATMOST(bs, k):
    return z3.AtMost(*bs, k) if any(_is_z3(x) for x in bs) else sum(map(bool, bs)) <= k


def ATLEAST(bs, k):
    return z3.AtLeast(*bs, k) if any(_is_z3(x) for x in bs) else sum(map(bool, bs)) >= k


def EXACTLY(bs, k):
    return z3.PbEq([(b, 1) for b in bs], k) if any(_is_z3(x) for x in bs) else sum(map(bool, bs)) == k


def PBLE(pairs, k):
    return z3.PbLe(pairs, k) if any(_is_z3(b) for b, _w in pairs) else sum(w for b, w in pairs if b) <= k


def PBGE(pairs, k):
    return z3.PbGe(pairs, k) if any(_is_z3(b) for b, _w in pairs) else sum(w for b, w in pairs if b) >= k


def PBEQ(pairs, k):
    return z3.PbEq(pairs, k) if any(_is_z3(b) for b, _w in pairs) else sum(w for b, w in pairs if b) == k


def COUNT(bs):
    """The number of true Booleans, as an integer term."""
    return z3.Sum([z3.If(b, 1, 0) for b in bs]) if any(_is_z3(x) for x in bs) else sum(map(bool, bs))


def DISTINCT(*a):
    """Pairwise different.  Written pair by pair: this chain of Z3 steps leaves
    a `distinct` over three or more terms as it is, which is no clause."""
    return AND(*[x != y for x, y in itertools.combinations(a, 2)])


# ---------------------------------------------------------------------------
# the toys
# ---------------------------------------------------------------------------

class Toy:
    def __init__(self, name, primitive, inputs, tests=(), hard=()):
        self.name = name
        self.primitive = primitive
        self.inputs = inputs            # ("int", name, lo, hi) | ("bool", name) | ("enum", name, k)
        self.tests = list(tests)        # (name, form, what, fn(env))
        self.hard = list(hard)          # (what, fn(env))

    def domains(self):
        out = []
        for inp in self.inputs:
            if inp[0] == "int":
                out.append(list(range(inp[2], inp[3] + 1)))
            elif inp[0] == "bool":
                out.append([False, True])
            else:
                out.append(list(range(inp[2])))
        return out


def _int_tests(kind, form, lo, hi):
    ops = {"eq": lambda x, c: x == c, "ne": lambda x, c: x != c,
           "le": lambda x, c: x <= c, "ge": lambda x, c: x >= c}
    sym = {"eq": "=", "ne": "!=", "le": "<=", "ge": ">="}
    op = ops[kind]
    return [(f"T_{kind}_{c}", form, f"x {sym[kind]} {c}", (lambda v, c=c: op(v["x"], c))) for c in range(lo, hi + 1)]


def toys():
    out = []
    # one bounded integer and one kind of test, for every constant
    for lo, hi in ((0, 3), (0, 6), (2, 7)):
        for kind in ("eq", "ne", "le", "ge"):
            for form in ("iff", "imp"):
                out.append(Toy(f"int-{kind}-{form}-{lo}_{hi}", "int-test", [("int", "x", lo, hi)],
                               _int_tests(kind, form, lo, hi)))
    # the four kinds side by side on one integer, and tests that are neither
    out.append(Toy("int-mix-0_6", "int-test", [("int", "x", 0, 6)],
                   [t for kind in ("eq", "ne", "le", "ge") for t in _int_tests(kind, "iff", 0, 6) if t[0][-1] in "24"]))
    out.append(Toy("int-set-0_6", "int-test", [("int", "x", 0, 6)], [
        ("T_in_1_4", "iff", "x = 1 or x = 4", lambda v: OR(v["x"] == 1, v["x"] == 4)),
        ("T_range_2_4", "iff", "2 <= x <= 4", lambda v: AND(v["x"] >= 2, v["x"] <= 4)),
        ("T_out_2_4", "iff", "x < 2 or x > 4", lambda v: OR(v["x"] < 2, v["x"] > 4)),
        ("T_g_in_0_5", "imp", "x = 0 or x = 5", lambda v: OR(v["x"] == 0, v["x"] == 5)),
    ]))
    # two integers
    for (ax, ay) in (((0, 3), (0, 3)), ((0, 4), (0, 2))):
        tag = f"{ax[0]}_{ax[1]}x{ay[0]}_{ay[1]}"
        two = [("int", "x", *ax), ("int", "y", *ay)]
        out.append(Toy(f"pair-rel-{tag}", "int-pair", two, [
            ("T_le", "iff", "x <= y", lambda v: v["x"] <= v["y"]),
            ("T_lt", "iff", "x < y", lambda v: v["x"] < v["y"]),
            ("T_eq", "iff", "x = y", lambda v: v["x"] == v["y"]),
            ("T_ne", "iff", "x != y", lambda v: v["x"] != v["y"]),
        ]))
        top = ax[1] + ay[1]
        out.append(Toy(f"pair-sum-eq-{tag}", "int-pair", two,
                       [(f"T_s_{c}", "iff", f"x + y = {c}", (lambda v, c=c: v["x"] + v["y"] == c)) for c in range(0, top + 1)]))
        out.append(Toy(f"pair-sum-le-{tag}", "int-pair", two,
                       [(f"T_s_{c}", "iff", f"x + y <= {c}", (lambda v, c=c: v["x"] + v["y"] <= c)) for c in range(0, top + 1)]))
        out.append(Toy(f"pair-diff-le-{tag}", "int-pair", two,
                       [(f"T_d_{c + ay[1]}", "iff", f"x - y <= {c}", (lambda v, c=c: v["x"] - v["y"] <= c))
                        for c in range(-ay[1], ax[1] + 1)]))
        out.append(Toy(f"pair-sum-le-imp-{tag}", "int-pair", two,
                       [(f"T_s_{c}", "imp", f"x + y <= {c}", (lambda v, c=c: v["x"] + v["y"] <= c)) for c in range(0, top + 1)]))
    # three integers, and a weighted sum of two
    for n in (2, 3):
        three = [("int", "x", 0, n), ("int", "y", 0, n), ("int", "z", 0, n)]
        out.append(Toy(f"triple-sum-le-0_{n}", "int-triple", three,
                       [(f"T_s_{c}", "iff", f"x + y + z <= {c}", (lambda v, c=c: v["x"] + v["y"] + v["z"] <= c)) for c in range(0, 3 * n + 1)]))
        out.append(Toy(f"triple-sum-eq-0_{n}", "int-triple", three,
                       [(f"T_s_{c}", "iff", f"x + y + z = {c}", (lambda v, c=c: v["x"] + v["y"] + v["z"] == c)) for c in range(0, 3 * n + 1)]))
    out.append(Toy("triple-xyz", "int-triple", [("int", "x", 0, 2), ("int", "y", 0, 2), ("int", "z", 0, 4)], [
        ("T_sum", "iff", "x + y = z", lambda v: v["x"] + v["y"] == v["z"]),
        ("T_between", "iff", "x <= z and z <= y", lambda v: AND(v["x"] <= v["z"], v["z"] <= v["y"])),
    ]))
    out.append(Toy("pair-weighted", "int-pair", [("int", "x", 0, 3), ("int", "y", 0, 3)],
                   [(f"T_w_{c}", "iff", f"2x + y <= {c}", (lambda v, c=c: 2 * v["x"] + v["y"] <= c)) for c in range(0, 10)]))
    # counts over plain Booleans
    for k in (5, 6):
        bs = [("bool", f"b{i}") for i in range(k)]
        names = [f"b{i}" for i in range(k)]
        out.append(Toy(f"count-atmost-{k}", "bool-count", bs,
                       [(f"T_am_{c}", "iff", f"at most {c} of {k}", (lambda v, c=c, ns=tuple(names): ATMOST([v[n] for n in ns], c))) for c in range(0, k)]))
        out.append(Toy(f"count-atleast-{k}", "bool-count", bs,
                       [(f"T_al_{c}", "iff", f"at least {c} of {k}", (lambda v, c=c, ns=tuple(names): ATLEAST([v[n] for n in ns], c))) for c in range(1, k + 1)]))
        out.append(Toy(f"count-exactly-{k}", "bool-count", bs,
                       [(f"T_ex_{c}", "iff", f"exactly {c} of {k}", (lambda v, c=c, ns=tuple(names): EXACTLY([v[n] for n in ns], c))) for c in range(0, k + 1)]))
    names5 = [f"b{i}" for i in range(5)]
    out.append(Toy("count-sum-5", "bool-count", [("bool", n) for n in names5],
                   [(f"T_sum_{c}", "iff", f"the number of true ones <= {c}", (lambda v, c=c: COUNT([v[n] for n in names5]) <= c)) for c in range(0, 5)]))
    out.append(Toy("count-atmost-imp-5", "bool-count", [("bool", n) for n in names5],
                   [(f"T_am_{c}", "imp", f"at most {c} of 5", (lambda v, c=c: ATMOST([v[n] for n in names5], c))) for c in range(0, 5)]))
    # weighted sums over Booleans
    w = (1, 2, 2, 3, 4)
    wb = [("bool", n) for n in names5]
    pairs = lambda v: [(v[n], wi) for n, wi in zip(names5, w)]
    out.append(Toy("weighted-le", "bool-weighted", wb,
                   [(f"T_w_{c}", "iff", f"1 2 2 3 4 weighted sum <= {c}", (lambda v, c=c: PBLE(pairs(v), c))) for c in range(1, 12)]))
    out.append(Toy("weighted-ge", "bool-weighted", wb,
                   [(f"T_w_{c}", "iff", f"1 2 2 3 4 weighted sum >= {c}", (lambda v, c=c: PBGE(pairs(v), c))) for c in range(1, 12)]))
    out.append(Toy("weighted-eq", "bool-weighted", wb,
                   [(f"T_w_{c}", "iff", f"1 2 2 3 4 weighted sum = {c}", (lambda v, c=c: PBEQ(pairs(v), c))) for c in range(1, 12)]))
    # a choice among k values (Z3's enumeration sort, written as a binary code)
    for k in (3, 5, 8):
        out.append(Toy(f"enum-value-{k}", "enum", [("enum", "e", k)],
                       [(f"T_is_{j}", "iff", f"e is value {j}", (lambda v, j=j: IS(v["e"], j))) for j in range(k)]))
    out.append(Toy("enum-pair-5", "enum", [("enum", "e", 5), ("enum", "f", 5)], [
        ("T_same", "iff", "e = f", lambda v: v["e"] == v["f"]),
        ("T_differ", "iff", "e != f", lambda v: DISTINCT(v["e"], v["f"])),
    ]))
    out.append(Toy("enum-distinct-3of4", "enum", [("enum", "e", 4), ("enum", "f", 4), ("enum", "g", 4)], [
        ("T_alldiff", "iff", "e, f, g all different", lambda v: DISTINCT(v["e"], v["f"], v["g"])),
    ]))
    out.append(Toy("enum-hard-distinct", "enum", [("enum", "e", 3), ("enum", "f", 3), ("enum", "g", 3)],
                   [(f"T_e_{j}", "iff", f"e is value {j}", (lambda v, j=j: IS(v["e"], j))) for j in range(3)],
                   hard=[("e, f, g all different", lambda v: DISTINCT(v["e"], v["f"], v["g"]))]))
    # if-then-else chains
    sel = [("bool", n) for n in ("s1", "s2", "a", "b", "c")]
    out.append(Toy("ite-chain-2", "ite", sel, [
        ("T_ite", "iff", "s1 ? a : (s2 ? b : c)", lambda v: ITE(v["s1"], v["a"], ITE(v["s2"], v["b"], v["c"]))),
    ]))
    sel3 = [("bool", n) for n in ("s1", "s2", "s3", "a", "b", "c", "d")]
    out.append(Toy("ite-chain-3", "ite", sel3, [
        ("T_ite", "iff", "s1 ? a : (s2 ? b : (s3 ? c : d))",
         lambda v: ITE(v["s1"], v["a"], ITE(v["s2"], v["b"], ITE(v["s3"], v["c"], v["d"])))),
        ("T_other", "iff", "s3 ? a : (s1 ? b : d)", lambda v: ITE(v["s3"], v["a"], ITE(v["s1"], v["b"], v["d"]))),
    ]))
    out.append(Toy("ite-int-value", "ite", [("bool", "s1"), ("bool", "s2"), ("int", "x", 0, 4)],
                   [(f"T_eq_{c}", "iff", f"x = {c}", (lambda v, c=c: v["x"] == c)) for c in range(0, 5)],
                   hard=[("x = (s1 ? 1 : (s2 ? 3 : 4))", lambda v: v["x"] == ITE(v["s1"], 1, ITE(v["s2"], 3, 4)))]))
    out.append(Toy("ite-int-select", "ite", [("bool", "s"), ("int", "x", 0, 3), ("int", "y", 0, 3)],
                   [(f"T_le_{c}", "iff", f"(s ? x : y) <= {c}", (lambda v, c=c: ITE(v["s"], v["x"], v["y"]) <= c)) for c in range(0, 3)]))
    # the Bel's Pyramid channel in small: a placement is three value tests at once
    tri = [("int", "X", 0, 6), ("int", "Y", 0, 6), ("int", "Z", 0, 6)]
    cubes = [(2, 3, 5), (3, 2, 5), (0, 0, 0), (6, 6, 6), (1, 1, 4), (4, 1, 1), (0, 6, 3), (2, 2, 2)]
    for form in ("iff", "imp"):
        out.append(Toy(f"channel-{form}", "channel", tri,
                       [(f"P_{a}{b}{c}", form, f"X = {a} and Y = {b} and Z = {c}",
                         (lambda v, a=a, b=b, c=c: AND(v["X"] == a, v["Y"] == b, v["Z"] == c))) for a, b, c in cubes]))
    # constraints with no test variable at all
    out.append(Toy("hard-pair-le", "hard", [("int", "x", 0, 3), ("int", "y", 0, 3)],
                   hard=[("x <= y", lambda v: v["x"] <= v["y"])]))
    out.append(Toy("hard-pair-ne", "hard", [("int", "x", 0, 3), ("int", "y", 0, 3)],
                   hard=[("x != y", lambda v: v["x"] != v["y"])]))
    out.append(Toy("hard-pair-sum", "hard", [("int", "x", 0, 3), ("int", "y", 0, 3)],
                   hard=[("x + y = 4", lambda v: v["x"] + v["y"] == 4)]))
    out.append(Toy("hard-alldiff-3", "hard", [("int", "x", 0, 3), ("int", "y", 0, 3), ("int", "z", 0, 3)],
                   hard=[("x, y, z all different", lambda v: DISTINCT(v["x"], v["y"], v["z"]))]))
    out.append(Toy("hard-chain-le", "hard", [("int", "x", 0, 3), ("int", "y", 0, 3), ("int", "z", 0, 3)],
                   hard=[("x <= y", lambda v: v["x"] <= v["y"]), ("y <= z", lambda v: v["y"] <= v["z"])]))
    toy_names = [t.name for t in out]
    assert len(toy_names) == len(set(toy_names)), [n for n, c in Counter(toy_names).items() if c > 1]
    return out


# name -> {tactic: {parameter: value}}.  `base` is the chain that wrote the
# Bel's Pyramid cells: lia2card, dt2bv, card2bv with pb.solver=totalizer,
# bit-blast, tseitin-cnf.  Every other set changes one thing.
def option_sets():
    card = lambda **kw: {"card2bv": {"pb.solver": "totalizer", **kw}}
    return [
        ("base", card()),
        ("pb.sorting", {"card2bv": {"pb.solver": "sorting"}}),
        ("pb.binary_merge", {"card2bv": {"pb.solver": "binary_merge"}}),
        ("pb.bv", {"card2bv": {"pb.solver": "bv"}}),
        ("card.grouped", card(**{"cardinality.encoding": "grouped"})),
        ("card.bimander", card(**{"cardinality.encoding": "bimander"})),
        ("card.ordered", card(**{"cardinality.encoding": "ordered"})),
        ("card.unate", card(**{"cardinality.encoding": "unate"})),
        ("card.circuit", card(**{"cardinality.encoding": "circuit"})),
        ("lia.compile_equality", {**card(), "lia2card": {"compile_equality": True}}),
        ("cnf.no_ite_extra", {**card(), "tseitin-cnf": {"ite_extra": False}}),
        ("cnf.no_common_patterns", {**card(), "tseitin-cnf": {"common_patterns": False}}),
        ("cnf.no_distributivity", {**card(), "tseitin-cnf": {"distributivity": False}}),
        ("cnf.no_ite_chaing", {**card(), "tseitin-cnf": {"ite_chaing": False}}),
    ]


STAGES = ("lia2card", "dt2bv", "card2bv", "bit-blast", "tseitin-cnf")


# ---------------------------------------------------------------------------
# building a record with Z3
# ---------------------------------------------------------------------------

def _consts(expr):
    """Names of the uninterpreted constants in a Z3 term."""
    out, seen, stack = {}, set(), [expr]
    while stack:
        t = stack.pop()
        if t.get_id() in seen:
            continue
        seen.add(t.get_id())
        if z3.is_const(t) and t.decl().kind() == z3.Z3_OP_UNINTERPRETED:
            out[str(t)] = t
        else:
            stack.extend(t.children())
    return out


def _parse_dimacs(text):
    n_vars, clauses, names = 0, [], {}
    for line in text.splitlines():
        if line.startswith("p cnf"):
            n_vars = int(line.split()[2])
        elif line.startswith("c "):
            parts = line.split()
            if len(parts) == 3 and parts[1].isdigit():
                if parts[2] in names:
                    raise ValueError(f"Z3 named two variables {parts[2]}")
                names[parts[2]] = int(parts[1])
        elif line.strip():
            lits = [int(t) for t in line.split()]
            if lits[-1] != 0:
                raise ValueError(f"a clause line without its 0: {line!r}")
            clauses.append(lits[:-1])
    return n_vars, clauses, names


def z3_model(toy, ctx):
    """(goal, env): the toy as a Z3 goal, and its inputs as Z3 terms."""
    env = {}
    sorts = {}                      # choices of the same size share one sort, so they can be compared
    g = z3.Goal(ctx=ctx)
    for inp in toy.inputs:
        if inp[0] == "int":
            x = z3.Int(inp[1], ctx)
            g.add(x >= inp[2], x <= inp[3])
            env[inp[1]] = x
        elif inp[0] == "bool":
            env[inp[1]] = z3.Bool(inp[1], ctx)
        else:
            k = inp[2]
            if k not in sorts:
                sorts[k], _vals = z3.EnumSort(f"E{k}", [f"v{j}of{k}" for j in range(k)], ctx)
            env[inp[1]] = z3.Const(inp[1], sorts[k])
    for _what, fn in toy.hard:
        g.add(fn(env))
    for name, form, _what, fn in toy.tests:
        b = z3.Bool(name, ctx)
        e = fn(env)
        g.add(b == e if form == "iff" else z3.Implies(b, e))
    # every choice gets its value tests, so that its code can be read back below
    for inp in toy.inputs:
        if inp[0] == "enum":
            for j in range(inp[2]):
                g.add(z3.Bool(f"A_{inp[1]}_{j}", ctx) == IS(env[inp[1]], j))
    return g, env


def table_of(toy, fn):
    return "".join("1" if fn(dict(zip([i[1] for i in toy.inputs], vals))) else "0"
                   for vals in itertools.product(*toy.domains()))


def cross_check(toy):
    """The Python tables against Z3's own reading of the model, value by value."""
    ctx = z3.Context()
    g, env = z3_model(toy, ctx)
    names = [i[1] for i in toy.inputs]
    for vals in itertools.product(*toy.domains()):
        py = dict(zip(names, vals))
        s = z3.Solver(ctx=ctx)
        s.add(*list(g))
        for inp, val in zip(toy.inputs, vals):
            x = env[inp[1]]
            s.add(IS(x, val) if inp[0] == "enum" else x == val)
        allowed = all(fn(py) for _w, fn in toy.hard)
        res = s.check()
        if (res == z3.sat) != allowed:
            raise AssertionError(f"{toy.name}: at {py} Z3 says {res}, the table says allowed={allowed}")
        if not allowed:
            continue
        m = s.model()
        for name, form, what, fn in toy.tests:
            want = bool(fn(py))
            if form == "iff":
                got = z3.is_true(m.eval(z3.Bool(name, ctx), model_completion=True))
                if got != want:
                    raise AssertionError(f"{toy.name}: test {name} ({what}) at {py}: Z3 {got}, table {want}")
            else:
                s.push()
                s.add(z3.Bool(name, ctx))
                can = s.check() == z3.sat
                s.pop()
                if can != want:
                    raise AssertionError(f"{toy.name}: guard {name} ({what}) at {py}: Z3 can set it {can}, table {want}")


def z3_record(toy, opt_name, opts):
    """One record, or a string saying why this option set gives no clauses."""
    ctx = z3.Context()
    g, _env = z3_model(toy, ctx)
    cur, after = g, {}
    for name in STAGES:
        t = z3.Tactic(name, ctx)
        if opts.get(name):
            t = z3.With(t, **opts[name])
        try:
            res = t(cur)
        except z3.Z3Exception as e:
            return f"{name} failed: {str(e).strip()[:80]}"
        if len(res) != 1:
            return f"{name} gave {len(res)} subgoals"
        cur = res[0]
        after[name] = cur
    for f in cur:                   # every literal of the result must be a plain Boolean name
        for lit in (f.children() if z3.is_or(f) else [f]):
            atom = lit.arg(0) if z3.is_not(lit) else lit
            if not (z3.is_const(atom) and atom.decl().kind() == z3.Z3_OP_UNINTERPRETED):
                return f"no clause form: the result still holds the term {str(atom)[:60]}"
    try:
        n_vars, clauses, names = _parse_dimacs(cur.dimacs())
    except (z3.Z3Exception, ValueError) as e:
        return f"no clause form: {str(e).strip()[:80]}"
    inputs = []
    for inp in toy.inputs:
        if inp[0] == "int":
            _k, name, lo, hi = inp
            bits = {}
            for f in after["lia2card"]:
                bits.update({n: c for n, c in _consts(f).items() if n.startswith(name + "!")})
            down = {}
            for f in after["lia2card"]:
                if z3.is_implies(f) and str(f.arg(0)) in bits and str(f.arg(1)) in bits:
                    down[str(f.arg(0))] = str(f.arg(1))        # the higher bit implies the lower
            lowest = [b for b in bits if b not in down]
            if len(bits) != hi - lo or len(lowest) != 1 or len(down) != len(bits) - 1:
                return f"{name}: {len(bits)} sorted bits and {len(down)} chain links for {hi - lo + 1} values"
            up = {v: k for k, v in down.items()}
            order = [lowest[0]]
            while order[-1] in up:
                order.append(up[order[-1]])
            if len(order) != len(bits) or any(b not in names for b in order):
                return f"{name}: the chain does not reach every bit, or a bit is not in the clauses"
            vs = [names[b] for b in order]
            inputs.append({"name": name, "kind": "order", "lo": lo, "values": list(range(lo, hi + 1)), "vars": vs,
                           "patterns": [[v if k < val - lo else -v for k, v in enumerate(vs)] for val in range(lo, hi + 1)]})
        elif inp[0] == "bool":
            if inp[1] not in names:
                return f"{inp[1]}: not in the clauses"
            v = names[inp[1]]
            inputs.append({"name": inp[1], "kind": "bool", "values": [0, 1], "vars": [v], "patterns": [[-v], [v]]})
        else:
            _k, name, k = inp
            pats, used = [], {}
            for j in range(k):
                anchor = f"A_{name}_{j}"
                side = None
                for f in after["bit-blast"]:
                    if z3.is_eq(f) and str(f.arg(0)) == anchor:
                        side = f.arg(1)
                    elif z3.is_eq(f) and str(f.arg(1)) == anchor:
                        side = f.arg(0)
                    elif str(f) == anchor and k == 1:
                        side = z3.BoolVal(True, ctx)
                if side is None:
                    return f"{name}: no definition of its value test {j} after bit-blast"
                cs = _consts(side)
                used.update(cs)
                hits = []
                for bitsval in itertools.product((False, True), repeat=len(cs)):
                    sub = [(c, z3.BoolVal(b, ctx)) for c, b in zip(cs.values(), bitsval)]
                    if z3.is_true(z3.simplify(z3.substitute(side, *sub))):
                        hits.append(dict(zip(cs, bitsval)))
                if len(hits) != 1:
                    return f"{name}: value test {j} holds on {len(hits)} bit patterns"
                pats.append(hits[0])
            order = sorted(used, key=lambda n: (len(n), n))
            if any(b not in names for b in order) or any(set(p) != set(order) for p in pats):
                return f"{name}: its value tests do not read the same bits, or a bit is not in the clauses"
            vs = [names[b] for b in order]
            inputs.append({"name": name, "kind": "binary", "values": list(range(k)), "vars": vs,
                           "patterns": [[names[b] if p[b] else -names[b] for b in order] for p in pats]})
    tests, dropped = [], []
    for name, form, what, fn in toy.tests:
        if name in names:
            tests.append({"name": name, "var": names[name], "form": form, "what": what, "table": table_of(toy, fn)})
        else:
            dropped.append(name)
    rec = {"name": toy.name, "primitive": toy.primitive, "options": [opt_name], "z3": z3.get_version_string(),
           "n_vars": n_vars, "clauses": clauses, "names": names, "inputs": inputs, "tests": tests}
    if toy.hard:
        rec["hard_what"] = [w for w, _f in toy.hard]
        rec["hard"] = table_of(toy, lambda v: all(fn(v) for _w, fn in toy.hard))
    if dropped:
        rec["dropped"] = dropped
    return rec


# ---------------------------------------------------------------------------
# what a record means, checked without Z3
# ---------------------------------------------------------------------------

def joint(rec):
    """[(index, literals)]: every joint value of the inputs, with the
    literals that set it."""
    out = []
    for i, combo in enumerate(itertools.product(*[range(len(inp["values"])) for inp in rec["inputs"]])):
        lits = []
        for inp, j in zip(rec["inputs"], combo):
            lits.extend(inp["patterns"][j])
        out.append((i, lits))
    return out


def structure_problems(rec):
    """Shape of the record itself, before any clause is looked at."""
    bad = []
    n = rec["n_vars"]
    seen = {}
    size = 1
    for inp in rec["inputs"]:
        vs = inp["vars"]
        if len(set(vs)) != len(vs) or any(not (1 <= v <= n) for v in vs):
            bad.append(f"input {inp['name']}: variables {vs}")
        for v in vs:
            if v in seen:
                bad.append(f"variable {v} belongs to {seen[v]} and to {inp['name']}")
            seen[v] = inp["name"]
        if len(inp["patterns"]) != len(inp["values"]) or len({tuple(p) for p in inp["patterns"]}) != len(inp["patterns"]):
            bad.append(f"input {inp['name']}: one distinct pattern per value is needed")
        for p in inp["patterns"]:
            if sorted(abs(l) for l in p) != sorted(vs):
                bad.append(f"input {inp['name']}: pattern {p} does not set exactly its variables")
        if inp["kind"] == "order":
            want = [[v if k < j else -v for k, v in enumerate(vs)] for j in range(len(vs) + 1)]
            if inp["patterns"] != want:
                bad.append(f"input {inp['name']}: not the sorted patterns of its variables")
        size *= len(inp["values"])
    for t in rec["tests"]:
        if not (1 <= t["var"] <= n) or t["var"] in seen:
            bad.append(f"test {t['name']}: variable {t['var']}")
        seen[t["var"]] = t["name"]
        if len(t["table"]) != size or set(t["table"]) - set("01"):
            bad.append(f"test {t['name']}: table of length {len(t['table'])} for {size} joint values")
        if t["form"] not in ("iff", "imp"):
            bad.append(f"test {t['name']}: form {t['form']}")
    if "hard" in rec and (len(rec["hard"]) != size or set(rec["hard"]) - set("01")):
        bad.append("hard: wrong length")
    for c in rec["clauses"]:
        if any(l == 0 or abs(l) > n for l in c):
            bad.append(f"clause {c}")
            break
    return bad


GUARDS_TOGETHER = 12    # guards (form imp) whose tests may hold at once; all their combinations are tried


def meaning_problems(rec, limit=25):
    """Brute force: the clauses say exactly what the record says.  Every legal
    joint value has a model, every other pattern of an input's variables has
    none, and under each joint value every test is what its table says: a
    two-way test has its table's value, a guard is off where its test fails,
    and the guards whose tests hold can take every combination of values."""
    bad = structure_problems(rec)
    if bad:
        return bad
    p = Prop(rec["n_vars"], rec["clauses"])
    hard = rec.get("hard")
    for inp in rec["inputs"]:
        legal = {tuple(sorted(pat, key=abs)) for pat in inp["patterns"]}
        vs = sorted(inp["vars"])
        for signs in itertools.product((-1, 1), repeat=len(vs)):
            lits = tuple(s * v for s, v in zip(signs, vs))
            if lits not in legal and satisfiable(p, lits) is not None:
                bad.append(f"input {inp['name']}: the pattern {list(lits)} is no value and still has a model")
                if len(bad) >= limit:
                    return bad
    for i, lits in joint(rec):
        allowed = hard is None or hard[i] == "1"
        mark = p.mark()
        ok = p.assume(lits)
        has = ok and satisfiable(p) is not None
        if has != allowed:
            bad.append(f"joint value {i}: a model exists: {has}, the record says {allowed}")
        elif allowed:
            for t in rec["tests"]:
                want = t["table"][i] == "1"
                lit = t["var"] if want else -t["var"]
                if t["form"] == "iff":
                    if p.value(lit) != 1 and satisfiable(p, [-lit]) is not None:
                        bad.append(f"test {t['name']} at joint value {i}: can be {not want}")
                elif not want:
                    if p.value(lit) != 1 and satisfiable(p, [-lit]) is not None:
                        bad.append(f"guard {t['name']} at joint value {i}: can be true where its test is false")
                elif p.value(t["var"]) != 0 or satisfiable(p, [t["var"]]) is None or satisfiable(p, [-t["var"]]) is None:
                    bad.append(f"guard {t['name']} at joint value {i}: not free where its test is true")
            # the guards whose tests hold here are free together, not only one by one
            loose = [t["var"] for t in rec["tests"] if t["form"] == "imp" and t["table"][i] == "1"]
            if 2 <= len(loose) <= GUARDS_TOGETHER:
                for signs in itertools.product((1, -1), repeat=len(loose)):
                    if satisfiable(p, [s * v for s, v in zip(signs, loose)]) is None:
                        bad.append(f"joint value {i}: the guards {[s * v for s, v in zip(signs, loose)]} cannot be set together")
                        break
            elif len(loose) > GUARDS_TOGETHER:
                bad.append(f"joint value {i}: {len(loose)} guards hold at once, more than this check tries together")
        p.undo(mark)
        if len(bad) >= limit:
            break
    return bad


# ---------------------------------------------------------------------------
# the shape: what must follow, and what propagation finds
# ---------------------------------------------------------------------------

SHAPE_KEYS = ["fwd", "0", "1", "2", "h1", "h2"]
SHAPE_COLUMNS = ["name", "primitive", "options", "z3", "vars", "clauses", "inputs", "tests", "helpers", "lengths"] + \
    [f"{k}_{c}" for k in SHAPE_KEYS for c in (("need", "up") if k == "fwd" else ("need", "up", "rup"))] + ["gap"]
PAIR_LIMIT = 44     # input literals: past this the two-literal level is left out


def shape(rec):
    p = Prop(rec["n_vars"], rec["clauses"])
    hard = rec.get("hard")
    J = [(i, frozenset(l)) for i, l in joint(rec)]
    allowed = [(i, s) for i, s in J if hard is None or hard[i] == "1"]
    in_vars = [v for inp in rec["inputs"] for v in inp["vars"]]
    in_lits = [s * v for v in in_vars for s in (1, -1)]
    n = Counter()

    def common(sets):
        it = iter(sets)
        acc = set(next(it))
        for s in it:
            acc &= s
        return acc

    def tally(key, assume, need):
        """`need` must follow from `assume`: count what propagation sets, and
        which of the rest is refuted by propagation when its opposite is added."""
        if not need:
            return
        got = p.implied(assume)
        n[key + "_need"] += len(need)
        if got is None:                 # propagation refutes the situation itself
            n[key + "_up"] += len(need)
            return
        got = set(got)
        for l in need:
            if l in got or p.value(l) == 1:         # set now, or true from the start
                n[key + "_up"] += 1
            elif p.refutes(assume + [-l]):
                n[key + "_rup"] += 1

    def with_lits(pool, lits):
        return [s for _i, s in pool if all(l in s for l in lits)]

    everything = [s for _i, s in J]
    base = common(s for _i, s in allowed) if allowed else set()
    free_of = {}                        # what a literal says by its own input alone
    for a in in_lits:
        ss = with_lits(J, [a])
        free_of[a] = common(ss) if ss else None
    hard_of = {}
    for a in in_lits:
        ss = with_lits(allowed, [a])
        hard_of[a] = common(ss) if ss else None
    two = len(in_lits) <= PAIR_LIMIT
    pairs = [(a, b) for a, b in itertools.combinations(in_lits, 2) if abs(a) != abs(b)] if two else []
    # the toy's own constraints, with no test
    if hard is not None:
        for a in in_lits:
            if hard_of[a] is None:
                continue                # a itself is ruled out: that is in `base`
            tally("h1", [a], hard_of[a] - free_of[a] - base)
        for a, b in pairs:
            ss = with_lits(allowed, [a, b])
            if not ss or hard_of[a] is None or hard_of[b] is None:
                continue
            fr = with_lits(J, [a, b])
            tally("h2", [a, b], common(ss) - hard_of[a] - hard_of[b] - (common(fr) if fr else set()) - base)
    # every input set: the tests
    for i, s in allowed:
        got = p.implied(sorted(s, key=abs))
        for t in rec["tests"]:
            want = t["table"][i] == "1"
            if t["form"] == "imp" and want:
                continue
            lit = t["var"] if want else -t["var"]
            n["fwd_need"] += 1
            n["fwd_up"] += got is None or lit in got or p.value(lit) == 1
    # a test, then a test with one and with two input literals
    for t in rec["tests"]:
        for value in ((True, False) if t["form"] == "iff" else (True,)):
            tl = t["var"] if value else -t["var"]
            pool = [(i, s) for i, s in allowed if (t["table"][i] == "1") == value]
            if not pool:
                continue                # the test is a constant; nothing to say about the inputs
            m0 = common(s for _i, s in pool)
            tally("0", [tl], m0 - base)
            m1 = {}
            for a in in_lits:
                ss = with_lits(pool, [a])
                if not ss or a in m0:
                    continue
                m1[a] = common(ss)
                tally("1", [tl, a], m1[a] - m0 - hard_of[a])
            for a, b in pairs:
                if a not in m1 or b not in m1:
                    continue
                ss = with_lits(pool, [a, b])
                if not ss:
                    continue
                both = with_lits(allowed, [a, b])
                tally("2", [tl, a, b], common(ss) - m1[a] - m1[b] - common(both))
    row = {"name": rec["name"], "primitive": rec["primitive"], "options": ",".join(rec["options"]), "z3": rec["z3"],
           "vars": rec["n_vars"], "clauses": len(rec["clauses"]),
           "inputs": " ".join(f"{i['name']}:{i['kind']}{len(i['values'])}" for i in rec["inputs"]),
           "tests": len(rec["tests"]),
           "helpers": rec["n_vars"] - len(in_vars) - len(rec["tests"]),
           "lengths": " ".join(f"{k}x{c}" for k, c in sorted(Counter(len(c) for c in rec["clauses"]).items()))}
    for k in SHAPE_KEYS:
        for c in (("need", "up") if k == "fwd" else ("need", "up", "rup")):
            row[f"{k}_{c}"] = n[f"{k}_{c}"] if (two or k not in ("2", "h2")) else "NA"
    gaps = [k for k in SHAPE_KEYS if n[k + "_need"] > n[k + "_up"]]
    row["gap"] = "none" if not gaps else " ".join(
        f"{k}:{n[k + '_need'] - n[k + '_up']}" + ("" if k == "fwd" or n[k + "_rup"] == n[k + "_need"] - n[k + "_up"] else "*")
        for k in gaps)
    return row


def row_text(row):
    return "\t".join(str(row[c]) for c in SHAPE_COLUMNS)


# ---------------------------------------------------------------------------
# the catalog file
# ---------------------------------------------------------------------------

def load(path=CATALOG):
    path = Path(path)
    opener = lzma.open if path.suffix == ".xz" else open
    with opener(path, "rt", encoding="ascii") as f:
        return [json.loads(line) for line in f if line.strip()]


def save(records, path):
    path = Path(path)
    opener = lzma.open if path.suffix == ".xz" else open
    tmp = path.with_name(path.name + ".tmp")
    with opener(tmp, "wt", encoding="ascii") as f:
        for r in records:
            f.write(json.dumps(r, separators=(",", ":"), sort_keys=True) + "\n")
    os.replace(tmp, path)


def dimacs(rec):
    lines = [f"c {rec['name']} [{','.join(rec['options'])}] Z3 {rec['z3']}"]
    for inp in rec["inputs"]:
        lines.append(f"c input {inp['name']} {inp['kind']} values {len(inp['values'])} variables {' '.join(map(str, inp['vars']))}")
    for t in rec["tests"]:
        lines.append(f"c test {t['var']} {t['form']} {t['what']} table {t['table']}")
    lines.append(f"p cnf {rec['n_vars']} {len(rec['clauses'])}")
    lines += [" ".join(map(str, c)) + " 0" for c in rec["clauses"]]
    return "\n".join(lines) + "\n"


def _build_toy(job):
    """Every option set for one toy: (records, notes)."""
    index, only = job
    need_z3()
    toy = toys()[index]
    cross_check(toy)
    recs, notes = [], []
    for opt_name, opts in option_sets():
        r = z3_record(toy, opt_name, opts)
        if isinstance(r, str):
            notes.append(f"{toy.name} [{opt_name}]: {r}")
            continue
        for old in recs:
            if all(old[k] == r[k] for k in ("n_vars", "clauses", "inputs", "tests", "names")):
                old["options"].append(opt_name)
                break
        else:
            recs.append(r)
    kept = []
    for r in recs:
        bad = meaning_problems(r)
        if bad:
            # Z3's clauses do not say what the model says (seen with one-sided
            # count outputs under a two-sided test): no ground truth, not kept
            notes.append(f"{toy.name} [{','.join(r['options'])}]: not kept, the clauses do not mean the model: {bad[0]}"
                         + (f" (and {len(bad) - 1} more)" if len(bad) > 1 else ""))
            continue
        r["shape"] = shape(r)
        kept.append(r)
    return toy.name, kept, notes


def _pool(jobs):
    return multiprocessing.get_context("fork").Pool(jobs)


def shapes_text(records):
    lines = ["\t".join(["id"] + SHAPE_COLUMNS)]
    lines += [r["id"] + "\t" + row_text(r["shape"]) for r in records]
    return "\n".join(lines) + "\n"


def cmd_build(a):
    need_z3()
    whole = Path(a.out).resolve() == CATALOG.resolve()
    if a.only and whole:
        sys.exit("--only builds a part: give --out, so that it does not replace the catalog")
    notes_path = a.notes if a.notes is not None else (str(NOTES) if whole else a.out + ".notes.txt")
    all_toys = toys()
    todo = [(i, a.only) for i, t in enumerate(all_toys) if not a.only or a.only in t.name]
    records, notes = [], []
    with _pool(a.jobs) as pool:
        for name, recs, ns in pool.imap(_build_toy, todo):
            records += recs
            notes += ns
            print(f"  {name}: {len(recs)} distinct records, {sum(len(r['options']) for r in recs)} of {len(option_sets())} option sets kept", flush=True)
    names = Counter(r["name"] for r in records)
    for r in records:               # a record's own name: the toy, and its first option set when the toy has several
        r["id"] = r["name"] if names[r["name"]] == 1 else f"{r['name']}@{r['options'][0]}"
    save(records, a.out)
    head = f"{len(records)} records from {len(todo)} toys and {len(option_sets())} option sets, Z3 {z3.get_version_string()}"
    print(f"{head}; wrote {a.out}")
    for line in notes:
        print("  note:", line)
    if notes_path:
        Path(notes_path).write_text("\n".join([head, "Toy and option sets that gave no record, and why:"] + notes) + "\n")
        print(f"wrote {notes_path}")
    if whole:                       # the shape table goes with the catalog
        SHAPES.write_text(shapes_text(records))
        print(f"wrote {SHAPES}")


def _check_one(rec):
    bad = meaning_problems(rec)
    if not bad and "shape" in rec and shape(rec) != rec["shape"]:
        bad.append("the stored shape row is not what the clauses give")
    return rec.get("id", rec["name"]), bad


def cmd_check(a):
    records = load(a.file)
    failed = 0
    with _pool(a.jobs) as pool:
        for rid, bad in pool.imap(_check_one, records):
            if bad:
                failed += 1
                print(f"FAIL {rid}: " + "; ".join(bad[:3]))
    ids = [r.get("id") for r in records]
    if len(set(ids)) != len(ids):
        failed += 1
        print("FAIL: two records share an id")
    if Path(a.file).resolve() == CATALOG.resolve():
        if not SHAPES.exists() or SHAPES.read_text() != shapes_text(records):
            failed += 1
            print(f"FAIL: {SHAPES} is not the table of this catalog (run `table --out` or `build`)")
    print(f"{len(records)} records, {failed} failed")
    sys.exit(1 if failed else 0)


def cmd_table(a):
    records = load(a.file)
    text = shapes_text(records)
    if a.out:
        Path(a.out).write_text(text)
        print(f"wrote {a.out} ({len(records)} rows)")
    else:
        sys.stdout.write(text)


def cmd_summary(a):
    """Per primitive: how many literals must follow, and how many propagation sets."""
    records = load(a.file)
    by = {}
    for r in records:
        by.setdefault(r["primitive"], []).append(r["shape"])
    keys = ["0", "1", "2", "h1", "h2"]
    print("primitive\trecords\twith_gap\tfwd need/up\t" + "\t".join(f"{k} need/up/rup" for k in keys))
    for prim, rows in by.items():
        tot = lambda c: sum(int(r[c]) for r in rows if r[c] != "NA")
        cells = [f"{tot(k + '_need')}/{tot(k + '_up')}/{tot(k + '_rup')}" for k in keys]
        print(f"{prim}\t{len(rows)}\t{sum(1 for r in rows if r['gap'] != 'none')}\t{tot('fwd_need')}/{tot('fwd_up')}\t" + "\t".join(cells))
    print(f"all\t{len(records)}\t{sum(1 for r in records if r['shape']['gap'] != 'none')}")


def cmd_list(a):
    for r in load(a.file):
        s = r["shape"]
        print(f"{r['id']:44s} vars {s['vars']:4d} clauses {s['clauses']:5d} tests {s['tests']:2d} gap {s['gap']}")


def cmd_emit(a):
    for r in load(a.file):
        if r["id"] == a.name:
            text = dimacs(r)
            if a.out:
                Path(a.out).write_text(text)
            else:
                sys.stdout.write(text)
            return
    sys.exit(f"no record with the id {a.name} (see `list`)")


# ---------------------------------------------------------------------------
# selftest: the checker on hand-made records
# ---------------------------------------------------------------------------

def _toy_record():
    """x in 0..2 as two sorted bits (1 = x >= 1, 2 = x >= 2, chain 2 -> 1);
    T = 3 is "x = 1" written as "exactly one of the two bits" (a parity
    gate), so only the chain lets propagation link the test to the bits."""
    return {"name": "hand", "primitive": "int-test", "options": ["hand"], "z3": "none", "n_vars": 3,
            "clauses": [[1, -2], [-3, 1, 2], [-3, -1, -2], [3, -1, 2], [3, 1, -2]], "names": {},
            "inputs": [{"name": "x", "kind": "order", "lo": 0, "values": [0, 1, 2], "vars": [1, 2],
                        "patterns": [[-1, -2], [1, -2], [1, 2]]}],
            "tests": [{"name": "T", "var": 3, "form": "iff", "what": "x = 1", "table": "010"}]}


def selftest():
    import copy
    rec = _toy_record()
    assert meaning_problems(rec) == [], meaning_problems(rec)
    s = shape(rec)
    # from the bits the test always follows; from "T" both bits should follow
    # and propagation finds neither, but each missing clause is refuted by
    # propagation; from "not T" nothing follows by itself, and with one bit
    # the other does (x != 1 and x >= 1 give x = 2), which propagation finds
    assert (s["fwd_need"], s["fwd_up"]) == (3, 3), s
    assert (s["0_need"], s["0_up"], s["0_rup"]) == (2, 0, 2), s
    assert (s["1_need"], s["1_up"], s["1_rup"]) == (2, 2, 0), s
    assert s["gap"] == "0:2" and s["helpers"] == 0, s
    # a test that is always true is a unit clause: propagation "finds" it from the start
    always = copy.deepcopy(rec)
    always["n_vars"] = 4
    always["clauses"].append([4])
    always["tests"].append({"name": "A", "var": 4, "form": "iff", "what": "x >= 0", "table": "111"})
    assert meaning_problems(always) == [], meaning_problems(always)
    s2 = shape(always)
    assert (s2["fwd_need"], s2["fwd_up"]) == (6, 6) and s2["0_need"] == 2, s2
    # a wrong table, a missing chain, a wrong pattern, a guard read as a test
    bad = copy.deepcopy(rec)
    bad["tests"][0]["table"] = "011"
    assert any("can be" in b for b in meaning_problems(bad)), meaning_problems(bad)
    bad = copy.deepcopy(rec)
    bad["clauses"].remove([1, -2])
    assert any("is no value and still has a model" in b for b in meaning_problems(bad)), meaning_problems(bad)
    bad = copy.deepcopy(rec)
    bad["inputs"][0]["patterns"][1] = [-1, 2]
    assert any("not the sorted patterns" in b for b in meaning_problems(bad)), meaning_problems(bad)
    bad = copy.deepcopy(rec)
    bad["tests"][0]["form"] = "imp"
    assert any("not free" in b for b in meaning_problems(bad)), meaning_problems(bad)
    # a guard: G -> x = 1, as two clauses; it is free where the test holds
    guard = copy.deepcopy(rec)
    guard["clauses"] = [[1, -2], [-3, 1], [-3, -2]]
    guard["tests"][0]["form"] = "imp"
    assert meaning_problems(guard) == [], meaning_problems(guard)
    s = shape(guard)
    assert (s["fwd_need"], s["fwd_up"], s["0_need"], s["0_up"], s["gap"]) == (2, 2, 2, 2, "none"), s
    guard["tests"][0]["form"] = "iff"
    assert any("can be" in b for b in meaning_problems(guard))
    # two guards on one value, G -> x = 1 and H -> x >= 1: each is free by
    # itself, and a clause that forbids both together must be noticed
    two = copy.deepcopy(rec)
    two["n_vars"] = 4
    two["clauses"] = [[1, -2], [-3, 1], [-3, -2], [-4, 1]]
    two["tests"] = [{"name": "G", "var": 3, "form": "imp", "what": "x = 1", "table": "010"},
                    {"name": "H", "var": 4, "form": "imp", "what": "x >= 1", "table": "011"}]
    assert meaning_problems(two) == [], meaning_problems(two)
    two["clauses"].append([-3, -4])
    assert any("cannot be set together" in b for b in meaning_problems(two)), meaning_problems(two)
    # a hard constraint: x <= y over two 2-valued numbers (bits 1 and 2), written
    # as "not (x and not y)"; and the same toy claiming every joint value
    hard = {"name": "hand2", "primitive": "hard", "options": ["hand"], "z3": "none", "n_vars": 2,
            "clauses": [[-1, 2]], "names": {},
            "inputs": [{"name": "x", "kind": "order", "lo": 0, "values": [0, 1], "vars": [1], "patterns": [[-1], [1]]},
                       {"name": "y", "kind": "order", "lo": 0, "values": [0, 1], "vars": [2], "patterns": [[-2], [2]]}],
            "tests": [], "hard": "1101", "hard_what": ["x <= y"]}
    assert meaning_problems(hard) == [], meaning_problems(hard)
    s = shape(hard)
    assert (s["h1_need"], s["h1_up"]) == (2, 2), s           # x forces y, not y forces not x
    wrong = copy.deepcopy(hard)
    del wrong["hard"]
    assert any("a model exists: False" in b for b in meaning_problems(wrong)), meaning_problems(wrong)
    # a binary-coded choice of three values on two bits: the fourth code must be excluded
    enum = {"name": "hand3", "primitive": "enum", "options": ["hand"], "z3": "none", "n_vars": 3,
            "clauses": [[-1, -2], [3, 1, 2], [-3, -1], [-3, -2]], "names": {},
            "inputs": [{"name": "e", "kind": "binary", "values": [0, 1, 2], "vars": [1, 2],
                        "patterns": [[-1, -2], [1, -2], [-1, 2]]}],
            "tests": [{"name": "T0", "var": 3, "form": "iff", "what": "e is value 0", "table": "100"}]}
    assert meaning_problems(enum) == [], meaning_problems(enum)
    open_code = copy.deepcopy(enum)
    open_code["clauses"].remove([-1, -2])
    assert any("is no value" in b for b in meaning_problems(open_code))
    # structure: a variable in two inputs, a table of the wrong length
    bad = copy.deepcopy(hard)
    bad["inputs"][1]["vars"] = [1]
    bad["inputs"][1]["patterns"] = [[-1], [1]]
    assert any("belongs to" in b for b in structure_problems(bad))
    bad = copy.deepcopy(rec)
    bad["tests"][0]["table"] = "01"
    assert any("table of length" in b for b in structure_problems(bad))
    # the dual expressions agree with Python on plain values
    assert AND(True, 1 == 1) and not AND(True, False) and OR(False, True) and NOT(False) and ITE(False, 1, 2) == 2
    assert ATMOST([True, False, True], 2) and not ATMOST([True, True, True], 2) and ATLEAST([True, False], 1)
    assert EXACTLY([True, False, True], 2) and PBLE([(True, 3), (False, 9)], 3) and not PBGE([(True, 3)], 4)
    assert PBEQ([(True, 3), (True, 1)], 4) and COUNT([True, True, False]) == 2 and DISTINCT(1, 2, 3) and not DISTINCT(1, 1)
    # the file round trip, and the DIMACS text
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        for name in ("c.jsonl", "c.jsonl.xz"):
            save([rec, hard], Path(d) / name)
            assert load(Path(d) / name) == [rec, hard]
    text = dimacs(rec)
    assert "p cnf 3 5" in text and "c test 3 iff x = 1 table 010" in text and text.count(" 0\n") == 5
    # the toy list is well formed without Z3: unique names, tables of the right size
    ts = toys()
    assert len(ts) >= 60 and len(option_sets()) == 14
    for t in ts:
        size = 1
        for d in t.domains():
            size *= len(d)
        assert size <= 400, (t.name, size)
        for _n, _f, _w, fn in t.tests:
            assert len(table_of(t, fn)) == size
    print(f"z3_catalog selftest ok ({len(ts)} toys, {len(option_sets())} option sets)")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--out", default=str(CATALOG))
    b.add_argument("--only", default="")
    b.add_argument("--jobs", type=int, default=8)
    b.add_argument("--notes", default=None, help="where to write what gave no record ('' for nowhere; "
                   "default: next to the output)")
    for name in ("check", "table", "list", "summary"):
        c = sub.add_parser(name)
        c.add_argument("file", nargs="?", default=str(CATALOG))
        if name == "check":
            c.add_argument("--jobs", type=int, default=8)
        if name == "table":
            c.add_argument("--out", default="")
    e = sub.add_parser("emit")
    e.add_argument("name")
    e.add_argument("--file", default=str(CATALOG))
    e.add_argument("--out", default="")
    sub.add_parser("selftest")
    a = ap.parse_args(argv)
    if a.cmd == "selftest":
        selftest()
    else:
        {"build": cmd_build, "check": cmd_check, "table": cmd_table, "list": cmd_list, "emit": cmd_emit,
         "summary": cmd_summary}[a.cmd](a)


if __name__ == "__main__":
    main()
