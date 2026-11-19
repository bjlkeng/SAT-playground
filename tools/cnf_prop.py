#!/usr/bin/env python3
"""cnf_prop.py - unit propagation and a small complete search over a clause
list, for the tools that read a formula by what it means (beads
SAT-playground-d96.9.1 and d96.9.2).

Unit propagation is the solver's cheap rule: when every literal of a clause
but one is false, the last one must be true.  `Prop` does exactly that and
nothing more, so "does propagation set this bit" has one answer here and in
the solver.  `satisfiable` adds a plain case split on top, for toys only: it
is how the catalog checks a toy's meaning by brute force.

    p = Prop(n_vars, clauses)        # root units are propagated at once
    mark = p.mark()
    ok = p.assume([3, -7])           # False when propagation hits a conflict
    p.value(5)                       # 1 true, -1 false, 0 open
    p.undo(mark)                     # back to the mark, conflict or not

A literal is a non-zero int, as in DIMACS.  Clauses are sequences of
literals; a repeated literal is harmless, a clause with a literal and its
opposite is skipped, an empty clause makes `root_conflict` true.

    python3 tools/cnf_prop.py selftest
"""
from __future__ import annotations

import sys


class Prop:
    """Two watched literals for clauses of three and more, direct lists for
    the two-literal clauses (most of a Tseitin file), one trail."""

    def __init__(self, n_vars, clauses):
        self.n = n_vars
        self.val = [0] * (n_vars + 1)
        self.trail = []
        size = 2 * n_vars + 2
        self.bins = [None] * size      # literal index -> literals it implies
        self.watch = [None] * size     # literal index -> clause indices watching it
        self.cl = []
        self.steps = 0                 # literals taken off the trail and visited
        self.root_conflict = False
        units = []
        for c in clauses:
            c = list(dict.fromkeys(c))
            if any(-l in c for l in c):
                continue
            if not c:
                self.root_conflict = True
            elif len(c) == 1:
                units.append(c[0])
            elif len(c) == 2:
                a, b = c
                self._push(self.bins, -a, b)
                self._push(self.bins, -b, a)
            else:
                ci = len(self.cl)
                self.cl.append(c)
                self._push(self.watch, c[0], ci)
                self._push(self.watch, c[1], ci)
        if not self.root_conflict:
            self.root_conflict = not self.assume(units)
        self.root = len(self.trail)

    @staticmethod
    def _idx(l):
        return 2 * l if l > 0 else -2 * l + 1

    def _push(self, table, l, x):
        i = 2 * l if l > 0 else -2 * l + 1
        lst = table[i]
        if lst is None:
            table[i] = [x]
        else:
            lst.append(x)

    def value(self, l):
        v = self.val[l if l > 0 else -l]
        return v if l > 0 else -v

    def mark(self):
        return len(self.trail)

    def undo(self, mark):
        val, trail = self.val, self.trail
        while len(trail) > mark:
            l = trail.pop()
            val[l if l > 0 else -l] = 0

    def assume(self, lits):
        """Set the literals and propagate.  False on a conflict; the trail
        then holds what was set before it, and `undo` clears it."""
        val, trail = self.val, self.trail
        head = len(trail)
        for l in lits:
            v = val[l if l > 0 else -l]
            if v == 0:
                val[l if l > 0 else -l] = 1 if l > 0 else -1
                trail.append(l)
            elif (v > 0) != (l > 0):
                return False
        return self._propagate(head)

    def _propagate(self, head):
        val, trail, bins, watch, cl = self.val, self.trail, self.bins, self.watch, self.cl
        while head < len(trail):
            l = trail[head]
            head += 1
            self.steps += 1
            lst = bins[2 * l if l > 0 else -2 * l + 1]
            if lst is not None:
                for m in lst:
                    v = val[m if m > 0 else -m]
                    if v == 0:
                        val[m if m > 0 else -m] = 1 if m > 0 else -1
                        trail.append(m)
                    elif (v > 0) != (m > 0):
                        return False
            fl = -l
            fi = 2 * fl if fl > 0 else -2 * fl + 1
            ws = watch[fi]
            if not ws:
                continue
            keep = []
            j, n = 0, len(ws)
            while j < n:
                ci = ws[j]
                j += 1
                c = cl[ci]
                if c[0] == fl:
                    c[0], c[1] = c[1], fl
                o = c[0]
                ov = val[o if o > 0 else -o]
                if ov != 0 and (ov > 0) == (o > 0):
                    keep.append(ci)
                    continue
                for k in range(2, len(c)):
                    m = c[k]
                    v = val[m if m > 0 else -m]
                    if v == 0 or (v > 0) == (m > 0):
                        c[1], c[k] = m, fl
                        self._push(watch, m, ci)
                        break
                else:
                    keep.append(ci)
                    if ov != 0:
                        keep.extend(ws[j:])
                        watch[fi] = keep
                        return False
                    val[o if o > 0 else -o] = 1 if o > 0 else -1
                    trail.append(o)
            watch[fi] = keep
        return True

    def implied(self, lits):
        """The literals propagation adds when `lits` are set (with `lits`
        themselves), or None on a conflict.  The state is left as it was."""
        mark = len(self.trail)
        ok = self.assume(lits)
        out = self.trail[mark:] if ok else None
        self.undo(mark)
        return out

    def refutes(self, lits):
        """True when setting `lits` runs into a conflict by propagation alone:
        the clause of their opposites is then checkable by a proof checker as
        it stands (RUP, reverse unit propagation)."""
        if self.root_conflict:
            return True
        mark = len(self.trail)
        ok = self.assume(lits)
        self.undo(mark)
        return not ok


def satisfiable(p, assume=(), order=None, limit=2_000_000):
    """A model (dict var -> bool) of the clauses of `p` with `assume` set, or
    None.  Plain case split with propagation, for toys: `limit` bounds the
    number of decisions and running past it raises.  `p` is left as it was."""
    if p.root_conflict:
        return None
    base = p.mark()
    if not p.assume(list(assume)):
        p.undo(base)
        return None
    order = list(order) if order is not None else list(range(1, p.n + 1))
    stack = []                      # (mark before the decision, literal, flipped)
    pos = 0
    decisions = 0
    while True:
        while pos < len(order) and p.val[order[pos]] != 0:
            pos += 1
        if pos == len(order):
            model = {v: p.val[v] > 0 for v in range(1, p.n + 1) if p.val[v] != 0}
            p.undo(base)
            return model
        v = order[pos]
        decisions += 1
        if decisions > limit:
            p.undo(base)
            raise RuntimeError("satisfiable: decision limit reached (not a toy?)")
        stack.append((p.mark(), v, False, pos))
        ok = p.assume([v])
        while not ok:
            while stack and stack[-1][2]:
                stack.pop()
            if not stack:
                p.undo(base)
                return None
            mark, lit, _f, at = stack.pop()
            p.undo(mark)
            stack.append((mark, lit, True, at))
            pos = at
            ok = p.assume([-lit])


def selftest():
    # the rule itself
    p = Prop(4, [(1, 2), (-2, 3), (-1, -3, 4)])
    assert not p.root_conflict and p.root == 0
    assert sorted(p.implied([-1])) == [-1, 2, 3]
    assert p.implied([1, 3]) == [1, 3, 4]
    assert p.implied([1, 3, -4]) is None and p.refutes([1, 3, -4])
    assert p.mark() == 0 and all(v == 0 for v in p.val)        # nothing left behind
    # root units, a repeated literal, a tautology, an empty clause
    p = Prop(3, [(1,), (-1, 2, 2), (3, -3)])
    assert p.root == 2 and p.value(2) == 1 and p.value(-1) == -1 and p.value(3) == 0
    assert Prop(1, [()]).root_conflict and Prop(1, [(1,), (-1,)]).root_conflict
    assert Prop(1, [(1,), (-1,)]).refutes([])                   # refuted already: all is RUP
    # a long clause moves its watches and still fires at the last literal
    p = Prop(6, [(1, 2, 3, 4, 5, 6)])
    assert p.implied([-1, -2, -3, -4, -5]) == [-1, -2, -3, -4, -5, 6]
    assert p.implied([-6, -5, -4, -3, -2]) == [-6, -5, -4, -3, -2, 1]
    assert p.implied([-1, -2, -3, -4, -5, -6]) is None
    # propagation is not the whole truth: (1 or 2), (1 or -2) force 1, by a case
    # split and not by the rule
    p = Prop(2, [(1, 2), (1, -2)])
    assert p.implied([]) == [] and p.refutes([-1])
    assert satisfiable(p, [-1]) is None and satisfiable(p)[1] is True
    # the search: pigeons (3 into 2) has no model, 2 into 2 has one, and every
    # model returned satisfies every clause
    def pigeons(n, m):
        v = lambda i, j: i * m + j + 1
        cl = [tuple(v(i, j) for j in range(m)) for i in range(n)]
        cl += [(-v(i, j), -v(k, j)) for j in range(m) for i in range(n) for k in range(i + 1, n)]
        return n * m, cl
    n, cl = pigeons(3, 2)
    assert satisfiable(Prop(n, cl)) is None
    n, cl = pigeons(4, 3)
    assert satisfiable(Prop(n, cl)) is None
    n, cl = pigeons(3, 3)
    p = Prop(n, cl)
    m = satisfiable(p)
    assert m is not None and all(any(m.get(abs(l), False) == (l > 0) for l in c) for c in cl)
    assert satisfiable(p, [1, 4]) is None and p.mark() == p.root
    # against exhaustive enumeration on random small formulas
    import itertools
    import random
    rng = random.Random(7)
    for _ in range(300):
        nv = rng.randint(3, 7)
        cl = [tuple(rng.choice((1, -1)) * rng.randint(1, nv) for _ in range(rng.randint(1, 3)))
              for _ in range(rng.randint(3, 22))]
        want = any(all(any((a[abs(l) - 1]) == (l > 0) for l in c) for c in cl)
                   for a in itertools.product((False, True), repeat=nv))
        p = Prop(nv, cl)
        got = satisfiable(p)
        assert (got is not None) == want, (cl, got)
        if got is not None:
            full = {v: got.get(v, False) for v in range(1, nv + 1)}
            assert all(any(full[abs(l)] == (l > 0) for l in c) for c in cl), (cl, got)
        assert p.mark() == p.root
    print("cnf_prop selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["selftest"]:
        selftest()
    else:
        sys.exit(__doc__)
