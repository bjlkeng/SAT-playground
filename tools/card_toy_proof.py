#!/usr/bin/env python3
"""Proof templates for counts (epic SAT-playground-iin, plan
plan/count-and-hall-lemmas-plan.md): the checked reference for how each form's
clauses imply a linear constraint. No search here: every derivation goes through
VeriPB, and a false claim must be rejected.

Forward form. A node has children a and b over leaf sets LA and LB and outputs
r_t. Its clauses say that i trues in a and j trues in b imply r_(i+j):
(-a_i v -b_j v r_(i+j)), with a_0 and b_0 true. What it means is

    E(t):  M r_t + sum over L of ~l >= M,   M = |L| - t + 1

"t of the leaves are true, so r_t is". The `pbc` of E(t) assumes the opposite,
which forces ~r_t and at least t leaves true. Adding that to E(a_(i+1)) and
E(b_(t-i)) leaves, under ~r_t, the clause (a_(i+1) v b_(t-i)). Those clauses and
the splits close by unit propagation alone:

    ~a_t, then b_1, then ~a_(t-1), then b_2, ... then a contradiction

so a node costs one `pol` per split and one `rup`. A capped node has overflow
clauses (-a_i v -b_j) at i + j = c + 1, the same derivation with no r, giving

    A:  sum over L of ~l >= |L| - c

A unit -r_m turns E(m) into at most m - 1 of the leaves.

A backward form written as the same clauses over negated literals is the same
derivation: at least k of x is at most n - k of the negations.

One-way counter. Registers mean "at least j of the first i", in one direction:
(-s v lo v x) and (-s v lo v hi), with lo = s_(i-1,j), hi = s_(i-1,j-1):

    T(s):  j ~s + sum over the first i of x >= j

Its `pbc` assumes s and at most j - 1 true. Adding T(lo) gives ~lo, adding T(hi)
gives (~x v ~hi), and the two clauses close it.

Decision diagram. A node n at some level says "if y then A else B", with y the
level's counted literal: (-n v -y v A) and (-n v y v B), and sometimes the third
clause (-n v A v B), which the proof does not need. Its value c = min(val(A) + 1,
val(B)) is the least number of trues n forces among its level and the later ones:

    T(n):  c ~n + sum of the literals of n's level and later >= c

The `pbc` assumes n and at most c - 1 of those literals true. No case split is
needed: c <= val(B), so adding T(B) makes ~B follow, and (-n v y v B) gives y. With
y true c - 2 is left over and c - 1 <= val(A), so adding T(A) gives ~A, and
(-n v -y v A) closes it. The same two sums and one `rup` as a one-way register.
Past a false child the clause is (-n v y) or (-n v -y); a true child has no clause;
a literal child from a later level needs no sum, because the negation already holds
it. A root unit turns T(root) into at least c of all the literals.

Adder network. A full adder holds a + b + c <= s + 2 m by its clauses in one
direction: (-a -b m), (-a -c m), (-b -c m), (-a b c s), (a -b c s), (a b -c s),
(-a -b -c s). As a constraint, ~a + ~b + ~c + s + 2 m >= 3, with a `pbc` and six
`rup` lines: ~m first, then with s false each input is false, so s; then a; then
a contradiction. A half adder is a + b <= s + 2 m from (-a -b m), (-a b s), (a -b s),
~a + ~b + s + 2 m >= 2, by ~a, ~b, then a contradiction. Nothing here looks at which
literal is plain or negated: a + b + c >= s + 2 m is the same cell over the negated
literals, and for a half adder that is a + b + 1 <= s + 2 m over s = xnor, m = or,
~a + ~b + s + 2 m >= 3, by ~m, then a contradiction.

Multiply each cell by 2^w, w the weight of its inputs, and add: a wire read once at
the weight it was made at cancels, and what is left is sum of ~x + sum of 2^w o >=
n + the constants, o the bits nothing reads. A bound on the bits comes from a `rup`:
sum of 2^w ~o >= K, by units and compare clauses (-o_i v -o_h ...), which propagation
closes from the top bit down. Adding the two gives sum of ~x >= n + constants + K - W.

Comparator network. A comparator's clauses (-a v h), (-b v h), (-a v -b v l) give
a + b <= h + l: add them and divide by 2, ~a + ~b + h + l >= 2. A half comparator has
one output: ~a + ~b + l >= 1 is its clause, and ~a + ~b + h >= 1 is (-a v h) with ~b
added. Each wire read once cancels in the sum, so sum of ~x + sum of o >= n - halves.
Units on outputs and a `w` on the rest give at most (halves + free outputs) of x.

That bound is weak when one unit is all there is: the free outputs can be anything
in a one-way network, so the sum alone does not give "at most k". What does is that
the network sorts. For a merge of two sorted vectors, a over leaves LA and b over LB,
with outputs r_t, the claim is the one a totalizer node has:

    E(t):  M r_t + sum over L of ~l >= M,   M = |L| - t + 1

The `pbc` assumes ~r_t and t leaves true. For each pair x + y <= t + 1, adding E(a_x)
and E(b_y) says a_x or b_y, and past the leaves nothing is needed. Then a case split
over how many of a are true, from the most down: "not all of a_1..a_p" as a `rup`.
With a_1..a_p true, the line before gives ~a_(p+1), the pairs give b_1..b_(t-p), and
those run forward through the merge's own comparators to r_t, which is false. The
lines are one `pol` per pair and one `rup` per count of a: no clause of a totalizer
node is needed. The comparators are only ever run forward, so half comparators and
wires left out by a cardinality network do not matter, as long as r_t still comes out.

Usage: python3 tools/card_toy_proof.py selftest [--veripb PATH]
"""
import argparse
import itertools
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VERIPB = ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb"


def lit(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


class Proof:
    def __init__(self, path, n_clauses):
        self.out = open(path, "w")
        self.out.write("pseudo-Boolean proof version 3.0\n")
        self.n = n_clauses

    def pol(self, expr):
        self.out.write(f"pol {expr} ;\n")
        self.n += 1
        return self.n

    def pbc(self, terms, rhs, lines):
        neg = self.n + 1
        body = lines(neg)
        text = " ".join(f"{c} {lit(l)}" for c, l in terms)
        self.out.write(f"pbc {text} >= {rhs} : subproof\n")
        for b in body:
            self.out.write(f"    {b}\n")
        self.out.write("qed;\n")
        self.n += 1 + len(body) + 1
        return self.n

    def rup(self, terms, rhs):
        text = " ".join(f"{c} {lit(l)}" for c, l in terms)
        self.out.write(f"rup {text} >= {rhs} ;\n")
        self.n += 1
        return self.n

    def e(self, terms, rhs, cid):
        text = " ".join(f"{c} {lit(l)}" for c, l in terms)
        self.out.write(f"e {text} >= {rhs} : {cid} ;\n")

    def finish(self):
        self.out.write("output NONE ;\nconclusion NONE ;\nend pseudo-Boolean proof ;\n")
        self.out.close()


class Node:
    """A merge node; a leaf is a Node with one literal and no children."""

    def __init__(self, a=None, b=None, r=None, capped=False, leaf=None):
        self.a, self.b, self.r, self.capped = a, b, r or [], capped
        self.leaf = leaf
        self.leaves = [leaf] if leaf is not None else a.leaves + b.leaves
        self.e = {}
        self.cap_id = None

    def out(self, i):
        return self.leaf if self.leaf is not None else self.r[i - 1]

    def width(self):
        return 1 if self.leaf is not None else len(self.r)


def child_term(p, child, i):
    """The constraint id for child's output i in a sum, or None when nothing is
    needed: a leaf's E is an axiom, and past the leaves nothing can be true."""
    if i > len(child.leaves) or child.leaf is not None:
        return None
    if i <= child.width():
        return prove_e(p, child, i)
    if i == child.width() + 1 and child.capped:
        return prove_cap(p, child)
    raise ValueError("a child hides counts: no output and no cap")


def splits(node, t):
    a, b = node.a, node.b
    imax = min(t, len(a.leaves), a.width())
    imin = max(0, t - min(len(b.leaves), b.width()))
    return imin, imax


def node_lines(p, node, t, neg):
    imin, imax = splits(node, t)
    a, b = node.a, node.b
    lines = []
    ids = []
    for i in range(imax, imin - 2, -1):
        x, y = i + 1, t - i
        if x < 1 or y < 1:
            continue
        ta = child_term(p, a, x)
        tb = child_term(p, b, y)
        ids.append((ta, tb))
    for ta, tb in ids:
        expr = str(neg)
        for tid in (ta, tb):
            if tid is not None:
                expr += f" {tid} +"
        lines.append(f"pol {expr} ;")
    lines.append("rup >= 1 ;")
    return lines


def prove_children_first(p, node, t):
    """Helper constraints are written before the pbc's negation takes its id."""
    imin, imax = splits(node, t)
    for i in range(imax, imin - 2, -1):
        x, y = i + 1, t - i
        if x < 1 or y < 1:
            continue
        child_term(p, node.a, x)
        child_term(p, node.b, y)


def prove_e(p, node, t):
    if t in node.e:
        return node.e[t]
    prove_children_first(p, node, t)
    m = len(node.leaves) - t + 1
    terms = [(m, node.out(t))] + [(1, -l) for l in node.leaves]
    node.e[t] = p.pbc(terms, m, lambda neg: node_lines(p, node, t, neg))
    return node.e[t]


def prove_cap(p, node):
    if node.cap_id is not None:
        return node.cap_id
    t = node.width() + 1
    prove_children_first(p, node, t)
    m = len(node.leaves) - t + 1
    terms = [(1, -l) for l in node.leaves]
    node.cap_id = p.pbc(terms, m, lambda neg: node_lines(p, node, t, neg))
    return node.cap_id


class Enc:
    def __init__(self, nv):
        self.nv = nv
        self.c = []

    def new(self):
        self.nv += 1
        return self.nv

    def add(self, *lits):
        self.c.append(list(lits))


def merge(enc, a, b, c=None, capped=False):
    """Forward clauses of a merge, outputs up to c."""
    n = len(a.leaves) + len(b.leaves)
    c = min(c if c is not None else n, n)
    r = [enc.new() for _ in range(c)]
    for i in range(0, a.width() + 1):
        for j in range(0, b.width() + 1):
            t = i + j
            if t == 0:
                continue
            cl = []
            if i:
                cl.append(-a.out(i))
            if j:
                cl.append(-b.out(j))
            if t <= c:
                cl.append(r[t - 1])
                enc.c.append(cl)
            elif t == c + 1 and capped:
                enc.c.append(cl)
    return Node(a, b, r, capped)


def totalizer(enc, lits, c=None, capped=False):
    if len(lits) == 1:
        return Node(leaf=lits[0])
    m = len(lits) // 2
    return merge(enc, totalizer(enc, lits[:m], c, capped), totalizer(enc, lits[m:], c, capped), c, capped)


def sinz(enc, lits, k):
    """The sequential counter: a chain of merges, each capped at k."""
    node = Node(leaf=lits[0])
    for x in lits[1:]:
        node = merge(enc, node, Node(leaf=x), k, True)
    return node


def check(veripb, cnf_clauses, nv, build, want):
    with tempfile.TemporaryDirectory() as d:
        cnf = Path(d) / "f.cnf"
        with open(cnf, "w") as f:
            f.write(f"p cnf {nv} {len(cnf_clauses)}\n")
            for c in cnf_clauses:
                f.write(" ".join(map(str, c)) + " 0\n")
        pbp = Path(d) / "f.pbp"
        p = Proof(pbp, len(cnf_clauses))
        try:
            build(p)
        except ValueError:
            p.finish()
            return not want
        p.finish()
        r = subprocess.run([str(veripb), str(cnf), str(pbp)], capture_output=True, text=True, timeout=300)
        ok = "s VERIFIED" in r.stdout
        return ok == want


def one_way(enc, lits, k):
    """Registers meaning at least j of the first i, in one direction. Returns
    the table and writes the unit on the top register."""
    s = {}
    n = len(lits)
    for i in range(1, n + 1):
        for j in range(1, min(i, k) + 1):
            s[i, j] = enc.new()
    for (i, j), v in s.items():
        x = lits[i - 1]
        lo = s.get((i - 1, j))
        hi = s.get((i - 1, j - 1)) if j > 1 else None
        if lo is None:
            enc.add(-v, x)
            if j > 1:
                enc.add(-v, hi)
        else:
            enc.add(-v, lo, x)
            if j > 1:
                enc.add(-v, lo, hi)
    enc.add(s[n, k])
    return s


def prove_one_way(p, s, lits, k, unit_id):
    t = {}
    n = len(lits)
    for i in range(1, n + 1):
        for j in range(1, min(i, k) + 1):
            v = s[i, j]
            terms = [(j, -v)] + [(1, l) for l in lits[:i]]
            lo, hi = t.get((i - 1, j)), t.get((i - 1, j - 1))

            def lines(neg, lo=lo, hi=hi):
                out = []
                if lo is not None:
                    out.append(f"pol {neg} {lo} + ;")
                if hi is not None:
                    out.append(f"pol {neg} {hi} + ;")
                out.append("rup >= 1 ;")
                return out

            t[i, j] = p.pbc(terms, j, lines)
    return p.pol(f"{t[n, k]} {unit_id} {k} * +")


def dd_nodes(enc, ys, k, twoway=False, third=True):
    """A decision diagram for at least k of the literals ys, node (i, c) deciding
    ys[i] with c still needed. Returns (root literal, table (i, c) -> (node, hi, lo)
    with hi and lo a node, or True or False for a terminal). The root is asserted."""
    table = {}
    n = len(ys)

    def node(i, c):
        if c <= 0:
            return True
        if c > n - i:
            return False
        if (i, c) in table:
            return table[i, c][0]
        v = enc.new()
        table[i, c] = (v, None, None)
        hi, lo = node(i + 1, c - 1), node(i + 1, c)
        table[i, c] = (v, hi, lo)
        y = ys[i]
        if hi is not True:
            enc.add(*([-v, -y] + ([] if hi is False else [hi])))
        if lo is not True:
            enc.add(*([-v, y] + ([] if lo is False else [lo])))
        if third and hi is not True and lo is not True and not (hi is False and lo is False):
            enc.add(*([-v] + [q for q in (hi, lo) if q is not False]))
        if twoway:
            if hi is not False:
                enc.add(*([v, -y] + ([] if hi is True else [-hi])))
            if lo is not False:
                enc.add(*([v, y] + ([] if lo is True else [-lo])))
        return v

    root = node(0, k)
    enc.add(root)
    return root, table


def prove_dd(p, ys, table, root, k, unit_id, fake=0, lo_first=True):
    """T(n) bottom up, children first. `fake` asks for one more than the node's value
    at the root, which VeriPB must reject."""
    t = {}
    n = len(ys)
    for i in range(n - 1, -1, -1):
        for c in range(1, n - i + 1):
            if (i, c) not in table:
                continue
            v, hi, lo = table[i, c]
            ask = c + (fake if v == root else 0)
            terms = [(ask, -v)] + [(1, y) for y in ys[i:]]

            def lines(neg, hi=hi, lo=lo):
                out = []
                # the false branch first; the order is free, the propagation is not
                sides = (lo, hi) if lo_first else (hi, lo)
                for q in sides:
                    if q is not True and q is not False:
                        out.append(f"pol {neg} {t[q]} + ;")
                out.append("rup >= 1 ;")
                return out

            t[v] = p.pbc(terms, ask, lines)
    return p.pol(f"{t[root]} {unit_id} {k + fake} * +")


def full_adder(enc, a, b, c, both=True):
    """Clauses for a + b + c <= s + 2 m, and with `both` also >=."""
    s, m = enc.new(), enc.new()
    enc.add(-a, -b, m)
    enc.add(-a, -c, m)
    enc.add(-b, -c, m)
    enc.add(-a, b, c, s)
    enc.add(a, -b, c, s)
    enc.add(a, b, -c, s)
    enc.add(-a, -b, -c, s)
    if both:
        enc.add(a, b, -m)
        enc.add(a, c, -m)
        enc.add(b, c, -m)
        enc.add(a, b, c, -s)
        enc.add(-a, -b, c, -s)
        enc.add(-a, b, -c, -s)
        enc.add(a, -b, -c, -s)
    return s, m


def half_adder(enc, a, b, both=True):
    s, m = enc.new(), enc.new()
    enc.add(-a, -b, m)
    enc.add(-a, b, s)
    enc.add(a, -b, s)
    if both:
        enc.add(a, -m)
        enc.add(b, -m)
        enc.add(a, b, -s)
        enc.add(-a, -b, -s)
    return s, m


def popcount(enc, xs, both=True):
    """Column compression. Returns (cells, bits by weight): a cell is (inputs, s, m, w)."""
    cols = {0: list(xs)}
    cells = []
    bits = []
    w = 0
    while w in cols:
        col = cols[w]
        while len(col) > 2:
            a, b, c = col.pop(), col.pop(), col.pop()
            s, m = full_adder(enc, a, b, c, both)
            cells.append(((a, b, c), s, m, w))
            col.insert(0, s)
            cols.setdefault(w + 1, []).append(m)
        if len(col) == 2:
            a, b = col.pop(), col.pop()
            s, m = half_adder(enc, a, b, both)
            cells.append(((a, b), s, m, w))
            col.append(s)
            cols.setdefault(w + 1, []).append(m)
        bits.append(col[0])
        w += 1
    return cells, bits


def rup_lines(lits):
    return "rup " + " ".join(f"1 {lit(l)}" for l in lits) + " >= 1 ;" if lits else "rup >= 1 ;"


def cell_le(p, inputs, s, m, const=0):
    """inputs + const <= s + 2 m, as sum of ~inputs + s + 2 m >= len(inputs) + const."""
    terms = [(1, -a) for a in inputs] + [(1, s), (2, m)]
    if len(inputs) == 3:
        a, b, c = inputs
        body = [rup_lines([-m]), rup_lines([s, -a]), rup_lines([s, -b]), rup_lines([s, -c]), rup_lines([s]), rup_lines([a]), rup_lines([])]
    elif const:
        body = [rup_lines([-m]), rup_lines([])]
    else:
        a, b = inputs
        body = [rup_lines([-a]), rup_lines([-b]), rup_lines([])]
    return p.pbc(terms, len(inputs) + const, lambda neg: body)


def prove_at_most(p, cells, xs, bits, k, flip=False, fake=0):
    """At most k of xs, from the cells read as <=, or with `flip`, at least k of xs,
    from the same cells over the negated literals. Every bit bound by a `rup`."""
    n = len(xs)
    sign = -1 if flip else 1
    ids = []
    consts = 0
    for inputs, s, m, w in cells:
        ins = [sign * a for a in inputs]
        const = 1 if flip and len(inputs) == 2 else 0
        consts += const << w
        ids.append((cell_le(p, ins, sign * s, sign * m, const), 1 << w))
    expr = " ".join((f"{i} {c} * " if c > 1 else f"{i} ") + ("+" if q else "") for q, (i, c) in enumerate(ids))
    total = p.pol(expr)
    # the number on the bits: at most U
    wt = 1 << len(bits)
    if not flip:
        u = k
        terms = [(1 << w, -b) for w, b in enumerate(bits)]
        rhs = (wt - 1) - u
        need = n - k
    else:
        # the negated reading: the half adders' constants make up what the bits leave
        # above n, so the number on the bits is what is left of the count
        terms = [(1 << w, b) for w, b in enumerate(bits)]
        rhs = k
        need = n + consts + k - (wt - 1)
    rup = p.rup(terms, rhs + fake)
    return p.pol(f"{total} {rup} +"), need + fake


def compare_at_least(enc, bits, k):
    """The number on the bits is at least k: for each one bit of k, that bit or a
    higher bit where k has a zero."""
    for i in range(len(bits)):
        if (k >> i) & 1:
            enc.add(*([bits[i]] + [bits[h] for h in range(i + 1, len(bits)) if not (k >> h) & 1]))


def compare_at_most(enc, bits, k):
    """At most k: for each zero bit of k, not that bit or not a higher bit where k has
    a one."""
    for i in range(len(bits)):
        if not (k >> i) & 1:
            enc.add(*([-bits[i]] + [-bits[h] for h in range(i + 1, len(bits)) if (k >> h) & 1]))


class Vec:
    """A sorted vector of wires over some leaves; `a` and `b` its parts."""

    def __init__(self, leaves, wires, a=None, b=None):
        self.leaves, self.wires, self.a, self.b = leaves, wires, a, b
        self.e = {}


def comparator(enc, a, b, gates):
    h, l = enc.new(), enc.new()
    gates.append((a, b, h, l))
    return h, l


def oe_merge(enc, a, b, gates):
    """Batcher's odd-even merge of two sorted lists, the largest first."""
    if not a:
        return list(b)
    if not b:
        return list(a)
    if len(a) == 1 and len(b) == 1:
        return list(comparator(enc, a[0], b[0], gates))
    odd = oe_merge(enc, a[0::2], b[0::2], gates)
    even = oe_merge(enc, a[1::2], b[1::2], gates)
    out = [odd[0]]
    i = 1
    j = 0
    while i < len(odd) and j < len(even):
        h, l = comparator(enc, even[j], odd[i], gates)
        out += [h, l]
        i += 1
        j += 1
    out += odd[i:] + even[j:]
    return out


def sort_net(enc, xs, gates):
    if len(xs) == 1:
        return Vec(list(xs), list(xs))
    m = len(xs) // 2
    a, b = sort_net(enc, xs[:m], gates), sort_net(enc, xs[m:], gates)
    return Vec(list(xs), oe_merge(enc, a.wires, b.wires, gates), a, b)


def write_gates(enc, gates, keep=None, two_way=False):
    """The comparators' clauses, in one direction or both. With `keep`, the wires
    wanted: a comparator whose output nothing wants loses that output's clauses."""
    for a, b, h, l in gates:
        if keep is None or h in keep:
            enc.add(-a, h)
            enc.add(-b, h)
            if two_way:
                enc.add(-h, a, b)
        if keep is None or l in keep:
            enc.add(-a, -b, l)
            if two_way:
                enc.add(-l, a)
                enc.add(-l, b)


def cone(gates, wanted):
    """The wires the wanted ones are made from."""
    keep = set(wanted)
    for a, b, h, l in reversed(gates):
        if h in keep or l in keep:
            keep.add(a)
            keep.add(b)
    return keep


def prove_vec(p, v, t, keep=None):
    """E(t) of a vector, what it is made from first. With `keep`, only the wires of a
    pruned network have clauses; the others are not named."""
    if t in v.e or v.a is None:
        return v.e.get(t)
    na, nb = len(v.a.leaves), len(v.b.leaves)
    has = lambda u, x: keep is None or u.wires[x - 1] in keep
    ra = [x for x in range(1, na + 1) if has(v.a, x)]
    rb = [y for y in range(1, nb + 1) if has(v.b, y)]
    pairs = [(x, y) for x in ra + [na + 1] for y in rb + [nb + 1] if x + y <= t + 1]
    ids = {}
    for x, y in pairs:
        ids[x, y] = (prove_vec(p, v.a, x, keep) if x <= na else None, prove_vec(p, v.b, y, keep) if y <= nb else None)
    m = len(v.leaves) - t + 1
    terms = [(m, v.wires[t - 1])] + [(1, -l) for l in v.leaves]

    def body(neg):
        out = []
        for x, y in pairs:
            ia, ib = ids[x, y]
            if ia is None and ib is None and (x > na or len(v.a.leaves) > 1) and (y > nb or len(v.b.leaves) > 1):
                continue
            expr = str(neg)
            for i in (ia, ib):
                if i is not None:
                    expr += f" {i} +"
            if expr != str(neg):
                out.append(f"pol {expr} ;")
        # the case split, the most trues of a first; below t - nb every a is forced
        for q in sorted(ra, reverse=True):
            if t - nb < q <= t:
                out.append(rup_lines([-v.a.wires[w - 1] for w in ra if w <= q]))
        out.append("rup >= 1 ;")
        return out

    v.e[t] = p.pbc(terms, m, body)
    return v.e[t]


def clause_triples(enc, gates):
    """The clause ids of each comparator: (-a v h), (-b v h), (-a v -b v l)."""
    at = {tuple(c): i + 1 for i, c in reversed(list(enumerate(enc.c)))}
    return [(at[-a, h], at[-b, h], at[-a, -b, l]) for a, b, h, l in gates]


def selftest(veripb):
    results = []

    def report(name, ok):
        results.append(ok)
        print(f"{'ok  ' if ok else 'FAIL'} {name}")

    rng = random.Random(3)
    for n in (2, 3, 5, 8, 13):
        lits = [i + 1 for i in range(n)]
        rng.shuffle(lits)
        for flip in (False, True):
            ls = [-l for l in lits] if flip else lits
            # a full totalizer: every output, every leaf
            enc = Enc(n)
            root = totalizer(enc, ls)
            ok = check(veripb, enc.c, enc.nv, lambda p: [prove_e(p, root, t) for t in range(1, n + 1)], True)
            report(f"totalizer n={n} flip={flip}: every output", ok)
            if n >= 3:
                # bounded at k+1: outputs stop, with no cap, and the top one reads the count
                k = max(1, n // 2)
                enc = Enc(n)
                root = totalizer(enc, ls, k + 1)
                unit = len(enc.c) + 1
                enc.add(-root.out(k + 1))

                def build(p, root=root, k=k, unit=unit):
                    e = prove_e(p, root, k + 1)
                    m = n - k
                    p.pol(f"{e} {unit} {m} * +")

                report(f"totalizer n={n} at most {k}, flip={flip}: bounded at k+1 and a unit", check(veripb, enc.c, enc.nv, build, True))
                # capped: overflow clauses, no output past k, in every node
                enc = Enc(n)
                root = totalizer(enc, ls, k, True)
                report(f"totalizer n={n} at most {k}, flip={flip}: capped", check(veripb, enc.c, enc.nv, lambda p, root=root: prove_cap(p, root), True))
                # a Sinz counter
                enc = Enc(n)
                root = sinz(enc, ls, k)
                report(f"Sinz n={n} at most {k}, flip={flip}", check(veripb, enc.c, enc.nv, lambda p, root=root: prove_cap(p, root), True))
                # false claims are rejected: a clause dropped from a capped node
                enc = Enc(n)
                root = sinz(enc, ls, k)
                drop = [c for c in enc.c if len(c) == 2 and c[1] < 0 and c[0] < 0]
                if drop:
                    enc.c.remove(drop[-1])
                    report(f"Sinz n={n} at most {k}, flip={flip}: an overflow clause dropped, rejected",
                           check(veripb, enc.c, enc.nv, lambda p, root=root: prove_cap(p, root), False))
    # one-way counters
    for n, k in ((3, 2), (6, 3), (10, 4), (12, 11), (9, 1)):
        lits = [i + 1 for i in range(n)]
        rng.shuffle(lits)
        enc = Enc(n)
        s = one_way(enc, lits, k)
        unit = len(enc.c)
        report(f"one-way counter at least {k} of {n}", check(veripb, enc.c, enc.nv, lambda p: prove_one_way(p, s, lits, k, unit), True))
    # decision diagrams, with and without the third clause, one way and both ways,
    # the inputs in any order and of any sign
    for n, k in ((3, 2), (5, 2), (6, 3), (10, 4), (12, 11), (9, 1), (8, 7), (16, 5)):
        lits = [i + 1 for i in range(n)]
        rng.shuffle(lits)
        lits = [l if rng.random() < 0.5 else -l for l in lits]
        for third in (True, False):
            for twoway in (False, True):
                enc = Enc(n)
                root, table = dd_nodes(enc, lits, k, twoway, third)
                unit = len(enc.c)
                report(f"diagram at least {k} of {n}, third={third}, two-way={twoway}",
                       check(veripb, enc.c, enc.nv, lambda p: prove_dd(p, lits, table, root, k, unit), True))
                # the sum in the other order propagates just as well
                report(f"diagram at least {k} of {n}, third={third}, two-way={twoway}, hi side first",
                       check(veripb, enc.c, enc.nv, lambda p: prove_dd(p, lits, table, root, k, unit, lo_first=False), True))
                if k < n:
                    # one more than the diagram asks for: rejected
                    report(f"diagram at least {k + 1} of {n} claimed, third={third}, two-way={twoway}: rejected",
                           check(veripb, enc.c, enc.nv, lambda p: prove_dd(p, lits, table, root, k, unit, fake=1), False))
                # a clause gone from the middle: the same proof no longer goes through
                mid = next((c for c in enc.c if len(c) == 3 and c[1] in lits), None)
                if mid is not None and k >= 2 and n > k:
                    bad = [c for c in enc.c if c != mid]
                    unit2 = len(bad)
                    report(f"diagram at least {k} of {n}, third={third}, two-way={twoway}, {mid} dropped: rejected",
                           check(veripb, bad, enc.nv, lambda p: prove_dd(p, lits, table, root, k, unit2), False))
    # adder networks with a compare: the `e` line says the proof reached exactly the
    # count, so a wrong sum of constants is rejected too
    for n in (3, 5, 7, 8, 15, 16):
        lits = [i + 1 for i in range(n)]
        rng.shuffle(lits)
        lits = [l if rng.random() < 0.5 else -l for l in lits]
        for k in sorted({1, n // 2, n - 1}):
            for both in (True, False):
                enc = Enc(n)
                cells, bits = popcount(enc, lits, both)
                compare_at_most(enc, bits, k)

                def most(p, cells=cells, bits=bits, k=k, fake=0):
                    cid, need = prove_at_most(p, cells, lits, bits, k, fake=fake)
                    p.e([(1, -x) for x in lits], need, cid)

                report(f"adder at most {k} of {n}, both ways={both}", check(veripb, enc.c, enc.nv, most, True))
                report(f"adder at most {k - 1} of {n} claimed, both ways={both}: rejected",
                       check(veripb, enc.c, enc.nv, lambda p: most(p, fake=1), False))
                # a cell clause the proof relies on gone: (-a -b m) of one cell
                inputs, _s, m, _w = cells[len(cells) // 2]
                gone = [-inputs[0], -inputs[1], m]
                dropped = [c for c in enc.c if c != gone]
                report(f"adder at most {k} of {n}, both ways={both}, {gone} dropped: rejected",
                       check(veripb, dropped, enc.nv, most, False))
            # at least k: both ways round, the same cells over negated literals
            enc = Enc(n)
            cells, bits = popcount(enc, lits, True)
            compare_at_least(enc, bits, k)

            def least(p, cells=cells, bits=bits, k=k):
                cid, need = prove_at_most(p, cells, lits, bits, k, flip=True)
                p.e([(1, x) for x in lits], need, cid)

            report(f"adder at least {k} of {n}", check(veripb, enc.c, enc.nv, least, True))
    # comparator networks: the sum, with every output above the bound false
    for n in (2, 3, 4, 7, 8, 13, 16):
        lits = [i + 1 for i in range(n)]
        rng.shuffle(lits)
        lits = [l if rng.random() < 0.5 else -l for l in lits]
        for k in sorted({1, n // 2, n - 1}):
            if not 0 < k < n:
                continue
            enc = Enc(n)
            gates = []
            root = sort_net(enc, lits, gates)
            write_gates(enc, gates)
            unit0 = len(enc.c)
            for o in root.wires[k:]:
                enc.add(-o)

            def tele(p, gates=gates, root=root, k=k, fake=0):
                ids = [p.pol(f"{g[0]} {g[1]} + {g[2]} + 2 d") for g in clause_triples(enc, gates)]
                expr = " ".join(f"{i} " + ("+" if q else "") for q, i in enumerate(ids))
                for j, o in enumerate(root.wires[k:]):
                    expr += f" {unit0 + j + 1} +"
                for o in root.wires[:k]:
                    expr += f" x{abs(o)} w"
                last = p.pol(expr)
                p.e([(1, -x) for x in lits], n - k + fake, last)

            report(f"comparators, at most {k} of {n}, every higher output false: the sum", check(veripb, enc.c, enc.nv, tele, True))
            report(f"comparators, at most {k - 1} of {n} claimed, every higher output false: rejected",
                   check(veripb, enc.c, enc.nv, lambda p: tele(p, fake=1), False))
            # one unit on output k + 1 only: the merges' lines
            for prune in (False, True):
                for two_way in (False, True):
                    enc = Enc(n)
                    gates = []
                    root = sort_net(enc, lits, gates)
                    o = root.wires[k]
                    kept = cone(gates, [o]) if prune else None
                    write_gates(enc, gates, kept, two_way)
                    unit = len(enc.c) + 1
                    enc.add(-o)

                    def merges(p, root=root, k=k, o=o, unit=unit, kept=kept, ask=None):
                        ask = k + 1 if ask is None else ask
                        e = prove_vec(p, root, ask, kept)
                        m = n - ask + 1
                        last = p.pol(f"{e} {unit} {m} * +")
                        p.e([(1, -x) for x in lits], m, last)

                    report(f"sorting network, at most {k} of {n}, one unit, pruned={prune}, two-way={two_way}: the merges",
                           check(veripb, enc.c, enc.nv, merges, True))
                    if k >= 2:
                        # one less: E(k) of the output with the unit on another wire
                        bad = list(enc.c)
                        report(f"sorting network, at most {k - 1} of {n} claimed, one unit, pruned={prune}, two-way={two_way}: rejected",
                               check(veripb, bad, enc.nv, lambda p: merges(p, ask=k), False))
                        # a comparator clause gone
                        a0, b0, h0, l0 = next(g for g in reversed(gates) if [-g[0], -g[1], g[3]] in enc.c)
                        gone = [c for c in enc.c if c != [-a0, -b0, l0]]
                        unit2 = len(gone)
                        report(f"sorting network, at most {k} of {n}, one unit, pruned={prune}, two-way={two_way}, a comparator's (-a -b l) dropped: rejected",
                               check(veripb, gone, enc.nv, lambda p: merges(p, unit=unit2), False))
    print("all ok" if all(results) else "FAILED")
    return all(results)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["selftest"])
    ap.add_argument("--veripb", default=str(VERIPB))
    a = ap.parse_args()
    sys.exit(0 if selftest(a.veripb) else 1)


if __name__ == "__main__":
    main()
