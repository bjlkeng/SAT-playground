#!/usr/bin/env python3
"""number_read.py - read numbers stored as sorted bits by what they mean, and
list the clauses the encoding lacks (bead SAT-playground-d96.9.2).

Why.  Z3 writes an integer as sorted bits and a test on it as a count over
the bits.  Unit propagation (the solver's cheap rule) then runs from the bits
to the test and not back (solver README, "Numbers stored as sorted bits").
The clause shapes change with Z3's version and options, so this reader does
not match shapes.  It goes by meaning, and it uses propagation itself as the
judge, because the catalog showed that from the bits to the tests nothing is
ever missing (tools/z3_catalog.py, 60,134 of 60,134).

How, on a small example.  x in 0..3 as three sorted bits g1, g2, g3 (x >= 1,
x >= 2, x >= 3; clauses g2 -> g1 and g3 -> g2) and a test T <-> (x = 2)
written as a count.
  1. Find the chain.  Among the variables that no gate defines, g3 -> g2 ->
     g1 is a path of implications: a candidate number with the four patterns
     000, 100, 110, 111.  (Three tests by propagation then settle what is a
     bit and what is a helper or a guard that sits on the path: see
     Reading._chains, _prune and _trim.)
  2. Set it to each value and propagate.  T comes out off, off, on, off: so T
     is "x = 2", whatever clauses say so.
  3. Say what should follow the other way.  T on means x = 2: g2 on, g3 off.
     T off says nothing by itself (x may be below or above).
  4. Ask propagation.  From "T on" it sets neither bit.  So the clauses
     (-T or g2) and (-T or -g3) are missing, and each is checked: assume T
     and not g2, and propagation runs into a conflict.

What is read:
  numbers   chains of implications between variables no gate defines (the
            two-literal clauses; in a small formula propagation is asked
            too, since a link can hide behind a helper); a value whose
            pattern propagation refutes is dead
  readers   variables that some pattern sets.  A full reader is set by every
            pattern: a threshold (it equals a bit), a value test (= c), its
            opposite (!= c), or another function of the number.  A partial
            reader is set by some patterns only (a placement that needs three
            numbers; a guard T -> x = c)
  pairs     two numbers that a constraint ties (x <= y, x + y = c): found
            where they share a reader or one's bits read the other; then
            every joint value is tried
  codes     a choice among k values on a few bits (every value test is the
            AND of all the bits, each with its sign)

What is listed (the links), each one true in every model of the formula:
  equal     a threshold reader and its bit, both ways (two clauses)
  bound     a reader, with its sign, to the lowest or highest value it allows
  hole      a reader that rules out one value in the middle (three literals)
  dead      a value no model takes
  unit      a reader that no value allows
  pair      a bit of one number to a bit of another
and for each whether propagation already has it, and whether a proof checker
would take it as it stands (RUP) or after a few such steps (the steps are
built and checked here, lowest value first or highest first).

Commands:
  one FILE [--budget STEPS] [--json OUT] [--all-partial]
                                 read one formula, print the summary
  variant FILE --add equal,bound,hole,dead,unit,pair[,partial] --out OUT.cnf
                                 the file plus the chosen links
  census --out TABLE.tsv FILES_OR_FOLDERS... [--workers N] [--seconds S] [--mem-mb M]
                                 one row a file; a table that is there is
                                 added to, not redone.  With --solver BIN the
                                 solver's own number pass reads each file
                                 (seconds, not minutes) and the row is its
                                 report line
  report TABLE...                the census tables family by family (GBD's
                                 family and author from benchmarks/rl)
  pyramid FILE N                 this reader against tools/pyramid_read.py on
                                 a Bel's Pyramid cell of size N
  selftest [--jobs N] [--write]  hand-made toys, then every record of the Z3
                                 catalog with a sorted-bit number, plain and
                                 renumbered twice with signs flipped.  Two
                                 rules hold for every reading (each listed
                                 clause is true in every model, and checkable
                                 as the reading says); in the base setting a
                                 record with no constraint of its own must
                                 read in full; and the state of every reading
                                 is held to benchmarks/rl/number_read_catalog.tsv

The 2026 suite is never read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import json
import lzma
import os
import random
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from cnf_prop import Prop, satisfiable  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BUDGET = 400_000_000      # propagation steps for one formula
FULL_SCAN_VARS = 2_000            # ask propagation about every free variable up to this many
PARTIAL_SAMPLE = 2_000            # partial readers tried when not all of them are
MAX_CLAUSES = 30_000_000
MAX_VALUES = 4096                 # a longer chain is not read value by value
PAIR_FAN = 8                      # a reader shared by more numbers than this ties no pairs
SHORT_LIMIT = 300                 # the census: more two-member chains than this are not read


class Budget(Exception):
    pass


class Number:
    def __init__(self, lits):
        self.lits = lits              # lits[k] true <-> the value is above k; lits[k+1] -> lits[k]
        self.dead = set()             # values whose pattern propagation refutes
        self.readers = {}             # variable -> (mask of values where it is set true, mask where false)
        self.unread = False           # too long to set value by value: nothing is known of it

    @property
    def n(self):
        return len(self.lits)

    def pattern(self, v):
        return [l if k < v else -l for k, l in enumerate(self.lits)]

    def live(self):
        """The values no pattern refuted, in order; none for a chain that was
        not read."""
        if self.unread:
            return []
        return [v for v in range(self.n + 1) if v not in self.dead]


def kind_of(tmask, fmask, values):
    """What a reader is, from where the patterns set it.  `values` are the
    live values, in order."""
    known = [v for v in values if (tmask >> v) & 1 or (fmask >> v) & 1]
    if len(known) < len(values):
        return "partial"
    ts = [v for v in values if (tmask >> v) & 1]
    if not ts or len(ts) == len(values):
        return "constant"
    if ts == values[-len(ts):]:
        return "ge"                    # on from a value upwards: a threshold
    if ts == values[:len(ts)]:
        return "le"
    if len(ts) == 1:
        return "eq"
    if len(ts) == len(values) - 1:
        return "ne"
    return "set"


def small_sat(clauses, limit=400):
    """Can these clauses all be true together?  A plain case split for a
    handful of clauses; past `limit` splits the answer is "yes" (the caller
    then keeps a candidate it might have dropped: more work, nothing wrong)."""
    stack = [(clauses, {})]
    nodes = 0
    while stack:
        cls, asg = stack.pop()
        dead = False
        changed = True
        while changed and not dead:
            changed = False
            new = []
            for c in cls:
                keep = None
                sat = False
                for l in c:
                    a = asg.get(l if l > 0 else -l)
                    if a is None:
                        if keep is None:
                            keep = [l]
                        else:
                            keep.append(l)
                    elif a == (l > 0):
                        sat = True
                        break
                if sat:
                    continue
                if keep is None:
                    dead = True
                    break
                if len(keep) == 1:
                    l = keep[0]
                    asg[l if l > 0 else -l] = l > 0
                    changed = True
                else:
                    new.append(keep)
            cls = new
        if dead:
            continue
        if not cls:
            return True
        # a variable that shows one sign only can be set that way: no split needed
        signs = {}
        for c in cls:
            for l in c:
                v = l if l > 0 else -l
                signs[v] = signs.get(v, 0) | (1 if l > 0 else 2)
        pure = {v: sg == 1 for v, sg in signs.items() if sg != 3}
        if pure:
            asg.update(pure)
            stack.append((cls, asg))
            continue
        nodes += 1
        if nodes > limit:
            return True
        l = min(cls, key=len)[0]
        other = dict(asg)
        other[abs(l)] = not (l > 0)
        asg[abs(l)] = l > 0
        stack.append((cls, other))
        stack.append((cls, asg))
    return False


class Reading:
    def __init__(self, n_vars, clauses, budget=DEFAULT_BUDGET, full_scan=FULL_SCAN_VARS, pairs=True, partial="sample",
                 short_limit=None, kinds=None, proved_only=False, max_bits=MAX_VALUES - 1,
                 inputs_only=False, counted_only=False):
        """`partial`: True links every partial reader (a placement that needs
        three numbers; there can be a million); "sample" tries PARTIAL_SAMPLE
        of them after the full readers are linked and only counts what is
        still missing; False leaves them alone.
        `pairs`: True tries every two numbers that share a reader, "counted"
        only numbers under a count (is_counted), False none.
        `short_limit`: with more candidate chains of two members than this,
        those are not read (a formula with no gates has thousands: two
        variables and one clause between them).
        `kinds`: the kinds of link to add (all when None).  A link of another
        kind is only counted: it is not given to propagation, so whether a
        later link is missing is judged against what is really added, and
        `added` is, in order, a list a proof checker can follow.
        `proved_only`: a link with no short proof is counted and not added
        (the solver's pass, src/structnum.rs, works this way: it adds only
        what its proof can show).  `max_bits`: a longer chain is not read.
        `inputs_only`: a bit must be a variable that no shape could define
        at all; one in a ring of shapes (a member of an exactly-one group, a
        gate next to a chain that reads two ways) is left out.
        `counted_only`: only a number under a count (is_counted) gets
        clauses.  The solver's pass sets both: it is meant for integers that
        a model counts over, and must stay cheap and silent on a circuit."""
        self.n_vars = n_vars
        self.budget = budget
        self.full_scan = full_scan
        self.want_pairs = pairs
        self.want_partial = partial
        self.short_limit = short_limit
        self.kinds = None if kinds is None else set(kinds)
        self.proved_only = proved_only
        self.max_bits = max_bits
        self.inputs_only = inputs_only
        self.counted_only = counted_only
        self.shaped_any = set()
        self.added = []                # every clause given to propagation, in order: (clause, kind or "step")
        self.defs = {}
        self.p = Prop(n_vars, clauses)
        self.numbers = []
        self.links = []                # (kind, clause, how): how is rup | steps:N | hard
        self.steps_added = []          # helper clauses of multi-step proofs, in order, with the link they serve
        self.codes = []                # (bits, [(test literal, pattern)])
        self.pairs = []                # (index a, index b, allowed joint values or None)
        self.notes = Counter()
        self.truncated = ""
        self.owner = {}                # variable -> (number index, position)
        self.link_of = {}              # a link's clause -> the chain it was read from
        if self.p.root_conflict:
            self.notes["root_conflict"] = 1
            return
        try:
            self._scan(clauses)
            self._chains()
            self._link_numbers()
            if pairs:
                self._pairs()
        except Budget:
            self.truncated = "budget"

    # ------------------------------------------------------------------
    def _spend(self):
        if self.budget is not None and self.p.steps > self.budget:
            raise Budget()

    def _scan(self, clauses):
        """After the root units: the two-literal clauses as implications, the
        variables that are equal (a -> b and b -> a), and which variables a
        gate defines (v <-> OR or AND of others, of any width; the XOR of two
        others).

        A shape can be read more than one way: an XOR has no direction, an
        exactly-one group makes each member the NOR of the rest, and next to
        a chain b1 -> b0 the clauses of s = b0 and not b1 also say b1 = b0
        and not s.  So the gates are settled forward from the variables that
        no shape could define; what stays unsettled is treated as free, and
        the propagation tests of _chains decide what is a bit."""
        val = self.p.val
        imp = defaultdict(set)          # literal -> literals it implies by one two-literal clause
        for c in clauses:
            if len(c) > 2 and not any(val[abs(l)] != 0 for l in c):
                continue
            keep = self._reduce(c)
            if keep is not None and len(keep) == 2:
                a, b = keep
                imp[-a].add(b)
                imp[-b].add(a)
        self.imp = imp
        # equal variables, with sign: same[v] = (the lowest-numbered equal variable, +1 or -1)
        parent = {}                     # v -> (w, sign): v equals sign * w

        def find(v):
            sign, path = 1, []
            while v in parent:
                path.append((v, sign))
                v, sg = parent[v][0], parent[v][1]
                sign *= sg
            for u, before in path:
                parent[u] = (v, sign * before)
            return v, sign

        for a, outs in imp.items():
            for b in outs:
                if abs(a) < abs(b) and a in imp.get(b, ()):
                    ra, sa = find(abs(a))
                    rb, sb = find(abs(b))
                    if ra == rb:
                        continue
                    rel = (1 if a > 0 else -1) * (1 if b > 0 else -1) * sa * sb
                    lo, hi = (ra, rb) if ra < rb else (rb, ra)
                    parent[hi] = (lo, rel)
        self.same = {}
        for v in list(parent):
            self.same[v] = find(v)
        self.notes["equal_vars"] = len(self.same)
        defs = {}                       # output literal -> tuple of input literals: output <-> OR(inputs)
        three = defaultdict(set)        # three variables -> the sign patterns of their three-literal clauses
        for c in clauses:
            if len(c) < 3:
                continue
            keep = c if not any(val[abs(l)] != 0 for l in c) else self._reduce(c)
            if keep is None or len(keep) < 3:
                continue
            if len(keep) == 3:
                srt = sorted(keep, key=abs)
                three[tuple(abs(l) for l in srt)].add(tuple(l > 0 for l in srt))
            need = len(keep) - 1
            for t in keep:
                back = imp.get(t)       # literals that "not out" implies: the opposites of its inputs
                if back is None or len(back) < need:
                    continue
                if all((-l in back) for l in keep if l != t):
                    defs.setdefault(-t, tuple(l for l in keep if l != t))
        self.defs = defs
        # every way a variable could be a gate's output: (output class, classes it reads)
        could = defaultdict(list)
        for out, ins in defs.items():
            could[self.cls(abs(out))].append(frozenset(self.cls(abs(l)) for l in ins))
        odd = {(True, True, True), (True, False, False), (False, True, False), (False, False, True)}
        even = {(False, False, False), (False, True, True), (True, False, True), (True, True, False)}
        for vs, pats in three.items():
            if pats >= odd or pats >= even:     # the four clauses of a parity: each of the three is the XOR of the others
                self.notes["xor3"] += 1
                cs = [self.cls(v) for v in vs]
                for k in range(3):
                    could[cs[k]].append(frozenset(cs[:k] + cs[k + 1:]))
        waiting = defaultdict(list)     # class -> [output, the unsettled inputs of one of its definitions]
        ready = []
        for v, options in could.items():
            for ins in options:
                left = {w for w in ins if w != v and w in could}
                if not left:
                    ready.append(v)
                    break
                entry = [v, left]
                for w in left:
                    waiting[w].append(entry)
        defined = set()
        while ready:
            v = ready.pop()
            if v in defined:
                continue
            defined.add(v)
            for entry in waiting.pop(v, ()):
                entry[1].discard(v)
                if not entry[1] and entry[0] not in defined:
                    ready.append(entry[0])
        self.defined = defined
        self.shaped_any = set(could)
        self.notes["defined"] = len(defined)
        self.notes["ring"] = len(could) - len(defined)

    def cls(self, v):
        return self.same[v][0] if v in self.same else v

    def lit_cls(self, l):
        """The literal's class representative, with the sign carried over."""
        v = abs(l)
        if v in self.same:
            r, sg = self.same[v]
            return r * sg * (1 if l > 0 else -1)
        return l

    def _reduce(self, c):
        val = self.p.val
        keep = []
        for l in c:
            v = val[abs(l)]
            if v == 0:
                if -l in keep:
                    return None
                if l not in keep:
                    keep.append(l)
            elif (v > 0) == (l > 0):
                return None
        return keep

    def _free(self, v):
        """Not set at the root, not defined by a gate, and the one name of its
        class of equal variables."""
        if self.inputs_only:
            return self.p.val[v] == 0 and v not in self.same and v not in self.shaped_any
        return self.p.val[v] == 0 and v not in self.same and v not in self.defined

    def _chains(self):
        """Numbers: chains of implications between free variables, picked so
        that no bit is a function of the other bits or of another number.

        The links come from the two-literal clauses; when the formula is small,
        propagation is asked too (a link may hide behind a helper).  A path of
        links is only a candidate: a helper that the gate reading did not see
        can sit on it.  Two tests sort that out, both by propagation:
          the bit test   set a chain's lower members on and its higher ones
                         off, around one member: a real bit stays open (the
                         value may be either side of it), a helper is set;
          who reads whom a chain whose members are all set by every pattern of
                         another chain is a function of that number, not a
                         number of its own."""
        p = self.p
        free = [v for v in range(1, self.n_vars + 1) if self._free(v)]
        self.notes["free"] = len(free)
        edges = defaultdict(set)
        for a, outs in self.imp.items():
            ra = self.lit_cls(a)
            if not self._free(abs(ra)):
                continue
            for b in outs:
                rb = self.lit_cls(b)
                if abs(ra) != abs(rb) and self._free(abs(rb)):
                    edges[ra].add(rb)
        if len(free) <= self.full_scan:
            self.notes["scan"] = 1
            succ = {}
            for v in free:
                for l in (v, -v):
                    got = p.implied([l])
                    self._spend()
                    if got is None:
                        self.notes["failed_literals"] += 1
                    else:
                        succ[l] = {m for m in got if m != l and self._free(abs(m))}
            for x, outs in succ.items():
                for y in outs:
                    if -x in succ.get(-y, ()):      # propagation runs it both ways: a hidden two-literal clause
                        edges[x].add(y)
        # direct links only: drop u -> w when u -> x -> w
        direct = {}
        for u, outs in edges.items():
            far = set()
            for x in outs:
                sx = edges.get(x)
                if sx:
                    far |= sx
            d = outs - far
            if d:
                direct[u] = d
        pred = defaultdict(set)
        for u, outs in direct.items():
            for w in outs:
                pred[w].add(u)
        # the bit test, then one reading per candidate
        seen = {}
        cands = self._candidates(direct, pred)
        if self.short_limit is not None:
            short = sum(1 for c in cands if len(c) == 2)
            if short > self.short_limit:
                cands = [c for c in cands if len(c) > 2]
                self.notes["short_skipped"] = short
        for lits in cands:
            lits = self._prune(lits)
            if lits is None or tuple(lits) in seen:
                continue
            num = self._trim(self._read(lits))
            seen.setdefault(tuple(num.lits), num)
        # who reads whom
        full = {}
        for t, num in seen.items():
            live = num.live()
            full[t] = {w for w, (tm, fm) in num.readers.items() if kind_of(tm, fm, live) not in ("partial", "constant")}
        members = {t: {abs(l) for l in t} for t in seen}
        rank = lambda t: (-len(t), min(abs(l) for l in t), t)
        by_var = defaultdict(list)
        for t in seen:
            for v in members[t]:
                by_var[v].append(t)
        chosen = []
        for t in sorted(seen, key=rank):
            if seen[t].unread or len(seen[t].live()) < 2:
                continue
            # the chains that could hold t as a function: they read or hold its first member
            first = abs(t[0])
            rivals = set(by_var[first])
            for a in seen:
                if first in full[a]:
                    rivals.add(a)
            beaten = False
            for a in rivals:
                if a == t or not members[t] <= (full[a] | members[a]):
                    continue
                back = members[a] <= (full[t] | members[t])
                if not back or rank(a) < rank(t):
                    beaten = True
                    break
            if not beaten:
                chosen.append(t)
        # two chains that still share a bit are two readings of one number: keep
        # the one whose patterns set more (a helper in a chain leaves a value vague)
        taken = set()
        final = []
        for t in sorted(chosen, key=lambda t: (-len(full[t]), -len(t), min(abs(l) for l in t), t)):
            if members[t] & taken:
                self.notes["overlap_dropped"] += 1
                continue
            taken |= members[t]
            final.append(t)
        for t in sorted(final, key=lambda t: (min(abs(l) for l in t), t)):
            num = seen[t]
            idx = len(self.numbers)
            self.numbers.append(num)
            for k, l in enumerate(num.lits):
                self.owner.setdefault(abs(l), (idx, k))
        self.notes["candidates"] = len(seen)

    def _candidates(self, direct, pred, per_source=64, total=20000):
        """Maximal paths of direct links, top first; past the limits, one
        longest path per start.  Each is returned lowest bit first, in one
        orientation (a chain and its mirror are one number)."""
        height = {}
        state = {}
        for root in list(direct):
            if root in height:
                continue
            stack = [(root, iter(sorted(direct.get(root, ()))))]
            state[root] = 1
            while stack:
                x, it = stack[-1]
                for w in it:
                    if w in height or state.get(w) == 1:
                        continue
                    state[w] = 1
                    stack.append((w, iter(sorted(direct.get(w, ())))))
                    break
                else:
                    height[x] = 1 + max((height[w] for w in direct.get(x, ()) if w in height), default=0)
                    state[x] = 2
                    stack.pop()
        out, have = [], set()

        def emit(path):
            # an end that two chains share is a guard on both and a bit of
            # neither: the lowest member when two links lead into it from
            # literals that are themselves implied by something, the highest
            # member the same way round
            path = list(path)
            while len(path) >= 2 and sum(1 for w in direct.get(path[0], ()) if direct.get(w)) >= 2:
                path.pop(0)
            while len(path) >= 2 and sum(1 for u in pred.get(path[-1], ()) if pred.get(u)) >= 2:
                path.pop()
            if len(path) < 2:
                return
            if len(path) > 4 * self.max_bits:
                # far past what can be read value by value, even with a few
                # helpers taken out: the bit test alone would cost its length squared
                self.notes["too_long"] += 1
                return
            lits = path[::-1]
            neg = sum(1 for l in lits if l < 0)
            if 2 * neg > len(lits) or (2 * neg == len(lits) and lits[0] < 0):
                lits = [-l for l in reversed(lits)]
            if tuple(lits) not in have:
                have.add(tuple(lits))
                out.append(lits)

        sources = sorted((u for u in direct if not pred.get(u)), key=lambda u: (-height.get(u, 1), abs(u), u))
        for src in sources:
            count = 0
            if len(out) < total:
                stack = [[src]]
                while stack and count <= per_source:
                    path = stack.pop()
                    nxt = [w for w in sorted(direct.get(path[-1], ()), key=lambda w: (-height.get(w, 1), abs(w), w))
                           if w not in path]
                    if not nxt:
                        emit(path)
                        count += 1
                        continue
                    for w in reversed(nxt):
                        stack.append(path + [w])
                if count <= per_source:
                    continue
                self.notes["many_paths"] += 1
            path = [src]                    # past a limit: the longest path from here
            while True:
                nxt = [w for w in direct.get(path[-1], ()) if w not in path]
                if not nxt:
                    break
                path.append(max(nxt, key=lambda w: (height.get(w, 1), -abs(w), w)))
            emit(path)
        return out

    def _prune(self, lits):
        """The bit test.  Drop every member that propagation sets once the
        members below it are on and the members above it are off.  What it
        was set to is a fact worth keeping: the member equals a neighbour (a
        value between them is dead) or is a constant, and that goes into the
        links as a clause."""
        p = self.p
        lits = list(lits)
        while len(lits) >= 2:
            fail = []
            for k in range(len(lits)):
                got = p.implied(lits[:k] + [-l for l in lits[k + 1:]])
                self._spend()
                # set by this propagation, or set from the start (a member
                # that an earlier clause of this reading fixed for good)
                if got is None:
                    fail.append((k, 0))
                elif lits[k] in got or p.value(lits[k]) == 1:
                    fail.append((k, 1))
                elif -lits[k] in got or p.value(lits[k]) == -1:
                    fail.append((k, -1))
            if not fail:
                break
            # two members can each be set by the other: one of them goes, the
            # same one whichever way the chain is written
            k, how = max(fail, key=lambda kh: abs(lits[kh[0]]))
            below = [-lits[k - 1]] if k > 0 else []
            above = [lits[k + 1]] if k + 1 < len(lits) else []
            if how == 0:
                clause = below + above                  # the members around it cannot stand like that
            elif how == 1:
                clause = [lits[k]] + (below or above)   # on whenever the one below is (or: always on)
            else:
                clause = [-lits[k]] + (above or below)  # off whenever the one above is (or: always off)
            if clause:
                self._add("dead", clause)
            del lits[k]
            self.notes["not_a_bit"] += 1
        return lits if len(lits) >= 2 else None

    def _trim(self, num):
        """An end member that nothing reads is no bit of the number: it is a
        guard on the end value, or a helper that is free there.  The two
        patterns that differ in it alone show it: when propagation sets
        exactly the same variables to the same values in both, no one can
        tell the two values apart, and the member goes.  A real end bit is
        read by something: a count gives it back as a threshold, a test is
        on at one of the two values, a guard is off at one of them."""
        while num.n > 2 and not num.unread:
            for end, (a, b) in ((0, (0, 1)), (num.n - 1, (num.n - 1, num.n))):
                if a in num.dead or b in num.dead:
                    continue
                same = True
                for t, f in num.readers.values():
                    if ((t >> a) & 1, (f >> a) & 1) != ((t >> b) & 1, (f >> b) & 1):
                        same = False
                        break
                if same:
                    num = self._read(num.lits[1:] if end == 0 else num.lits[:-1])
                    self.notes["end_dropped"] += 1
                    break
            else:
                break
        return num

    # ------------------------------------------------------------------
    def _read(self, lits):
        """Set the chain to each value in turn and note what propagation sets."""
        num = Number(list(lits))
        if num.n > self.max_bits:
            # not read: and "nothing reads its ends" must not be concluded from
            # that (the end test would trim it down until it fits)
            self.notes["too_long"] += 1
            num.unread = True
            return num
        p = self.p
        own = {abs(l) for l in num.lits}
        tm = defaultdict(int)
        fm = defaultdict(int)
        for v in range(num.n + 1):
            got = p.implied(num.pattern(v))
            self._spend()
            if got is None:
                num.dead.add(v)
                continue
            bit = 1 << v
            for l in got:
                if abs(l) in own:
                    continue
                if l > 0:
                    tm[l] |= bit
                else:
                    fm[-l] |= bit
        for w in set(tm) | set(fm):
            num.readers[w] = (tm.get(w, 0), fm.get(w, 0))
        return num

    def _how(self, clause):
        """have: propagation runs the clause both ways already (for two
        literals) or all ways (for more); rup: it does not, and a checker takes
        the clause as it stands; no: neither."""
        p = self.p
        every = True
        for skip in range(len(clause)):
            got = p.implied([-l for k, l in enumerate(clause) if k != skip])
            self._spend()
            if got is None:
                continue                # the others' opposites are refuted already
            if clause[skip] not in got and p.value(clause[skip]) != 1:
                every = False
                break
        if every:
            return "have"
        ok = p.refutes([-l for l in clause])
        self._spend()
        return "rup" if ok else "no"

    def _add(self, kind, clause, num=None):
        """Note one implied clause and give it to propagation.  When a checker
        would not take it as it stands, build the steps: the same clause for
        the values below (or above), one at a time."""
        how = self._how(clause)
        if how == "have":
            self.notes["have_" + kind] += 1
            return how
        if self.kinds is not None and kind not in self.kinds:
            self.notes["left_" + kind] += 1     # missing, and not asked for: counted only
            return "left"
        if how == "no":
            steps = self._steps(clause, num) if num is not None else None
            if steps is None:
                how = "hard"            # true (the patterns say so), and no short proof was found here
                self.notes["unproved_" + kind] += 1
                if self.proved_only:
                    return how
            else:
                self.steps_added += [(tuple(c), tuple(clause)) for c in steps]
                how = f"steps:{len(steps)}"
        self.links.append((kind, tuple(clause), how))
        if num is not None:
            self.link_of[tuple(clause)] = tuple(num.lits)
        self.added.append((tuple(clause), kind))
        self.p.add(clause)
        return how

    def _steps(self, clause, num):
        """For (z or B), B a bound literal of `num`: the clauses (z or B') for
        the weaker bounds B', weakest first, each refuted by propagation once
        the one before it is there, and the clause itself after the last.
        Example: (z or value > 2) from (z or value > 0), then (z or value > 1):
        with "not z" and "value <= 1", the step before gives "value > 0", so
        the value is exactly 1, a full pattern, and propagation finishes.
        A clause with no bit at all (z holds at every value) walks all the
        way up the same way.  None when that does not work.  The steps are
        given to propagation as they are made; each is true whether or not
        the whole chain succeeds."""
        own = {abs(x) for x in num.lits}
        bits = [l for l in clause if abs(l) in own]
        rest = [l for l in clause if abs(l) not in own]
        pos = {l: k for k, l in enumerate(num.lits)}
        if not bits:
            seq = list(num.lits)        # z or (value > 0), ..., z or (value is the highest): then z
        elif len(bits) != 1:
            return None
        elif bits[0] in pos:            # z or (value > j): walk up from the lowest bit
            seq = [num.lits[k] for k in range(0, pos[bits[0]])]
        elif -bits[0] in pos:           # z or (value <= j): walk down from the highest bit
            seq = [-num.lits[k] for k in range(num.n - 1, pos[-bits[0]], -1)]
        else:
            return None
        p = self.p
        out = []
        proved = False
        for lit in seq:
            c = rest + [lit]
            ok = p.refutes([-l for l in c])
            self._spend()
            if not ok:
                break
            p.add(c)
            self.added.append((tuple(c), "step"))
            out.append(c)
        else:
            proved = p.refutes([-l for l in clause])
            self._spend()
        if proved:
            return out
        # no proof this way: what was added stays (it is true) and is noted as no one's step
        self.steps_added += [(tuple(c), None) for c in out]
        return None

    def _link_numbers(self):
        """The links of every number: thresholds first (a threshold equals a
        bit, and with those in most of the rest follows by propagation), then
        the other full readers, then the partial ones."""
        todo = []
        for num in self.numbers:
            live = num.live()
            if len(live) < 2 or (self.counted_only and not is_counted(num)):
                continue
            # a dead value: the values below and above it have nothing between them
            for v in sorted(num.dead):
                if 0 < v < num.n:
                    self._add("dead", [-num.lits[v - 1], num.lits[v]], num)
                elif v == 0:
                    self._add("dead", [num.lits[0]], num)
                else:
                    self._add("dead", [-num.lits[num.n - 1]], num)
            for w, (t, f) in sorted(num.readers.items()):
                k = kind_of(t, f, live)
                self.notes["reader_" + k] += 1
                # a constant (every value sets it the same way) is a unit clause to add
                todo.append((0 if k in ("ge", "le") else 2 if k == "partial" else 1, num, w, t, f, live, k))
        for rank in (0, 1):
            for r, num, w, t, f, live, k in todo:
                if r == rank:
                    self._link_reader(num, w, t, f, live, k)
        partial = [x for x in todo if x[0] == 2]
        if self.want_partial is True:
            for r, num, w, t, f, live, k in partial:
                self._link_reader(num, w, t, f, live, k)
        elif self.want_partial == "sample" and partial:
            rng = random.Random(1)
            pick = partial if len(partial) <= PARTIAL_SAMPLE else rng.sample(partial, PARTIAL_SAMPLE)
            for r, num, w, t, f, live, k in pick:
                self.notes["partial_sampled"] += 1
                for clause in self._reader_clauses(num, w, t, f, live):
                    if self._how(clause) != "have":
                        self.notes["partial_sample_missing"] += 1

    def _reader_clauses(self, num, w, t, f, live):
        """The bound, hole and unit clauses of a reader that is no threshold."""
        out = []
        for sign, forbidden in ((w, f), (-w, t)):
            allowed = [v for v in live if not (forbidden >> v) & 1]
            if len(allowed) == len(live):
                continue
            if not allowed:
                out.append([-sign])
                continue
            bounds, holes = self._bound_lits(num, allowed, live)
            out += [[-sign, b] for b in bounds]
            out += [[-sign, -num.lits[v - 1], num.lits[v]] for v in holes]
        return out

    def _bound_lits(self, num, allowed, live):
        """The bound literals that 'the value is one of `allowed`' gives, and
        the middle values it rules out."""
        lo, hi = allowed[0], allowed[-1]
        out = []
        if lo > live[0]:
            out.append(num.lits[lo - 1])
        if hi < live[-1]:
            out.append(-num.lits[hi])
        holes = [v for v in live if lo < v < hi and v not in allowed]
        return out, holes

    def _link_reader(self, num, w, t, f, live, kind):
        if kind in ("ge", "le"):
            # w equals a bit: on from the first value where a pattern sets it true
            ts = [v for v in live if (t >> v) & 1]
            bit = num.lits[ts[0] - 1] if kind == "ge" else -num.lits[ts[-1]]
            self._add("equal", [-w, bit], num)
            self._add("equal", [w, -bit], num)
            return
        for clause in self._reader_clauses(num, w, t, f, live):
            self._add("unit" if len(clause) == 1 else "bound" if len(clause) == 2 else "hole", clause, num)

    # ------------------------------------------------------------------
    def _pairs(self):
        """Two numbers tied by a constraint: one's bits read the other.  Every
        joint value is tried; the ones propagation refutes are ruled out, and
        what that says about the bounds becomes two-literal clauses."""
        seen = set()
        readers_of = defaultdict(list)
        for i, num in enumerate(self.numbers):
            for w in num.readers:
                o = self.owner.get(w)
                if o is not None and o[0] != i:
                    seen.add((min(i, o[0]), max(i, o[0])))
                else:
                    readers_of[w].append(i)
        for w, idxs in readers_of.items():
            if 2 <= len(idxs) <= PAIR_FAN:
                for x in range(len(idxs)):
                    for y in range(x + 1, len(idxs)):
                        seen.add((idxs[x], idxs[y]))
        if self.want_pairs == "counted":
            ok_num = {i for i, num in enumerate(self.numbers) if is_counted(num)}
            seen = {(i, j) for i, j in seen if i in ok_num and j in ok_num}
        p = self.p
        for i, j in sorted(seen):
            a, b = self.numbers[i], self.numbers[j]
            la, lb = a.live(), b.live()
            if len(la) * len(lb) > 4096:
                self.notes["pair_too_big"] += 1
                continue
            ok = set()
            for x in la:
                for y in lb:
                    if not p.refutes(a.pattern(x) + b.pattern(y)):
                        ok.add((x, y))
                    self._spend()
            if len(ok) == len(la) * len(lb):
                continue
            self.pairs.append((i, j, ok))
            self.notes["pairs"] += 1
            if not ok:
                continue
            for (first, second, table, lf, ls) in ((a, b, ok, la, lb), (b, a, {(y, x) for x, y in ok}, lb, la)):
                for x in lf[1:]:        # first >= x
                    ys = sorted({y for (xx, y) in table if xx >= x})
                    self._pair_links(first, second, first.lits[x - 1], ys, ls)
                for x in lf[:-1]:       # first <= x
                    ys = sorted({y for (xx, y) in table if xx <= x})
                    self._pair_links(first, second, -first.lits[x], ys, ls)

    def _pair_links(self, first, second, lit, ys, live):
        if not ys:
            self._add("pair", [-lit], None)
            return
        if ys[0] > live[0]:
            self._add("pair", [-lit, second.lits[ys[0] - 1]], None)
        if ys[-1] < live[-1]:
            self._add("pair", [-lit, -second.lits[ys[-1]]], None)

    # ------------------------------------------------------------------
    def find_codes(self):
        """Choices written as a binary code: at least three value tests, each
        the AND of the same free variables with their signs."""
        groups = defaultdict(list)
        if self.p.root_conflict:
            return self.codes
        for out, ins in self.defs.items():
            # out <-> OR(ins): the test is -out, on exactly when every input is false
            vs = tuple(sorted(abs(l) for l in ins))
            if len(vs) != len(ins) or any(self.p.val[v] != 0 or self.cls(v) in self.defined for v in vs):
                continue
            groups[vs].append((-out, tuple(sorted((-l for l in ins), key=abs))))
        self.codes = [(vs, sorted(tests)) for vs, tests in sorted(groups.items())
                      if len(tests) >= 3 and len({pat for _t, pat in tests}) == len(tests)]
        return self.codes

    # ------------------------------------------------------------------
    def summary(self):
        s = Counter()
        s["numbers"] = len(self.numbers)
        s["values_max"] = max((n.n + 1 for n in self.numbers), default=0)
        s["bits"] = sum(n.n for n in self.numbers)
        s["dead_values"] = sum(len(n.dead) for n in self.numbers)
        for kind, _c, how in self.links:
            s["link_" + kind] += 1
            s["how_" + how.split(":")[0]] += 1
        s["step_clauses"] = sum(1 for _c, to in self.steps_added if to is not None)
        for k, v in self.notes.items():
            s[k] += v
        s["pairs"] = len(self.pairs)
        s["codes"] = len(self.codes)
        lens = Counter(n.n + 1 for n in self.numbers)
        return dict(s), " ".join(f"{k}x{c}" for k, c in sorted(lens.items()))

    def link_clauses(self):
        """Every clause the reading added, in the order it added them: a
        link's steps come right before it, and a link may lean on the ones
        before it.  Read with `kinds` set to get a chosen part."""
        return [c for c, _k in self.added]


# ---------------------------------------------------------------------------
# files
# ---------------------------------------------------------------------------

def _open(path):
    with open(path, "rb") as fh:
        xz = fh.read(6) == b"\xfd7zXZ\x00"
    return (lzma.open if xz else open)(path, "rt", encoding="ascii", errors="replace")


def read_cnf(path, max_clauses=MAX_CLAUSES):
    """(variables in the header, clauses in the header, clauses, truncated).
    Clauses are kept as written, empty ones too.  A file of the 2026 suite is
    refused here, so no command reads one."""
    if is_holdout(path):
        sys.exit(f"{path}: the 2026 suite is the holdout, not read (plan section 8)")
    n_vars = n_cls = 0
    clauses, cur = [], []
    with _open(path) as f:
        for line in f:
            if not line or line[0] in "c%\n":
                continue
            if line[0] == "p":
                parts = line.split()
                n_vars, n_cls = int(parts[2]), int(parts[3])
                continue
            for tok in line.split():
                v = int(tok)
                if v == 0:
                    clauses.append(cur)
                    cur = []
                    if len(clauses) >= max_clauses:
                        return n_vars, n_cls, clauses, True
                else:
                    cur.append(v)
    if cur:
        clauses.append(cur)
    return n_vars, n_cls, clauses, False


COLUMNS = ["stem", "vars", "clauses", "cut_short", "seconds", "steps", "stopped",
           "numbers", "values", "tested", "tested_values", "counted", "counted_values", "counted_links", "valued",
           "profile", "short_skipped", "bits", "dead_values",
           "readers_threshold", "readers_eq", "readers_ne", "readers_set", "readers_partial",
           "link_equal", "link_bound", "link_hole", "link_dead", "link_unit", "link_pair", "links",
           "there_already", "as_it_stands", "in_steps", "not_proved", "step_clauses",
           "partial_sampled", "partial_missing", "pairs", "codes", "code_tests",
           "free", "gates", "ring", "candidates", "error"]


def profile_of(num):
    """(values, cut points that some variable equals, values with a test of
    their own).  A cut point is the step from one value to the next: one per
    bit.  A count over sorted bits gives every bit back as a threshold; a
    model that asks "is it 3?" leaves a value test."""
    live = num.live()
    cuts, tests = set(), set()
    for t, f in num.readers.values():
        k = kind_of(t, f, live)
        # a variable and its opposite say the same thing: a threshold that is
        # off from a value upwards marks the same cut point, and a variable
        # that is off at exactly one value tests that value
        if k in ("ge", "le"):
            cuts.add(t if k == "ge" else f)
        elif k in ("eq", "ne"):
            tests.add(t if k == "eq" else f)
    return len(live), len(cuts), len(tests)


def is_counted(num):
    """The mark of an integer under a count: three bits or more, and every
    cut point between its values, but one at most, has a variable equal to
    it."""
    values, cuts, _tests = profile_of(num)
    return num.n >= 3 and values >= 2 and cuts >= values - 2


def is_valued(num, at_least=2):
    """Four values or more, and value tests at `at_least` different values."""
    values, _cuts, tests = profile_of(num)
    return values >= 4 and tests >= at_least


def row_of(stem, n_vars, n_clauses, truncated, r, seconds):
    """One census row.  `tested` counts the numbers that some variable reads
    in full (a threshold, a value test, its opposite, a set): the others are
    bare chains of implications."""
    s, lens = r.summary()
    tested, counted = [], []
    counted_chains = set()
    valued = 0
    prof = Counter()
    for num in r.numbers:
        live = num.live()
        if any(kind_of(t, f, live) in ("ge", "le", "eq", "ne", "set") for t, f in num.readers.values()):
            tested.append(num.n + 1)
        if len(live) >= 4:
            prof[profile_of(num)] += 1
        valued += is_valued(num)
        if is_counted(num):
            counted.append(num.n + 1)
            counted_chains.add(tuple(num.lits))
    links = sum(v for k, v in s.items() if k.startswith("link_"))
    row = {"stem": stem, "vars": n_vars, "clauses": n_clauses, "cut_short": int(truncated), "seconds": f"{seconds:.1f}",
           "steps": r.p.steps, "stopped": r.truncated,
           "numbers": len(r.numbers), "values": lens, "tested": len(tested),
           "tested_values": " ".join(f"{k}x{c}" for k, c in sorted(Counter(tested).items())),
           "counted": len(counted), "counted_values": " ".join(f"{k}x{c}" for k, c in sorted(Counter(counted).items())),
           "counted_links": sum(1 for _k, c, _h in r.links if r.link_of.get(c) in counted_chains),
           "valued": valued,
           "profile": " ".join(f"{v}v{c}c{t}t:{n}" for (v, c, t), n in sorted(prof.items())[:40]),
           "short_skipped": s.get("short_skipped", 0),
           "bits": s.get("bits", 0), "dead_values": s.get("dead_values", 0),
           "readers_threshold": s.get("reader_ge", 0) + s.get("reader_le", 0),
           "readers_eq": s.get("reader_eq", 0), "readers_ne": s.get("reader_ne", 0), "readers_set": s.get("reader_set", 0),
           "readers_partial": s.get("reader_partial", 0),
           "links": links, "there_already": sum(v for k, v in s.items() if k.startswith("have_")),
           "as_it_stands": s.get("how_rup", 0), "in_steps": s.get("how_steps", 0), "not_proved": s.get("how_hard", 0),
           "step_clauses": s.get("step_clauses", 0),
           "partial_sampled": s.get("partial_sampled", 0), "partial_missing": s.get("partial_sample_missing", 0),
           "pairs": len(r.pairs), "codes": len(r.codes), "code_tests": sum(len(t) for _vs, t in r.codes),
           "free": s.get("free", 0), "gates": s.get("defined", 0), "ring": s.get("ring", 0),
           "candidates": s.get("candidates", 0), "error": "root conflict" if s.get("root_conflict") else ""}
    for k in ("equal", "bound", "hole", "dead", "unit", "pair"):
        row["link_" + k] = s.get("link_" + k, 0)
    return row


def read_file(path, budget=DEFAULT_BUDGET, max_clauses=MAX_CLAUSES, partial="sample", pairs=True, **kw):
    t0 = time.time()
    n_vars, n_cls, clauses, truncated = read_cnf(path, max_clauses)
    top = max((abs(l) for c in clauses for l in c), default=0)
    r = Reading(max(n_vars, top), clauses, budget=budget, partial=partial, pairs=pairs, **kw)
    del clauses
    r.find_codes()
    return r, row_of(_stem(path), n_vars, n_cls, truncated, r, time.time() - t0)


def _stem(path):
    name = Path(path).name
    for suf in (".xz", ".gz", ".bz2", ".cnf"):
        if name.endswith(suf):
            name = name[: -len(suf)]
    return name


def cmd_one(a):
    r, row = read_file(a.file, budget=a.budget, partial=True if a.all_partial else "sample")
    print(f"{row['stem']}: {row['vars']} variables, {row['clauses']} clauses{' (cut short)' if row['cut_short'] else ''}, "
          f"{row['seconds']} s, {row['steps']} propagation steps{', stopped at the budget' if r.truncated else ''}")
    print(f"  numbers {row['numbers']} (values x count: {row['values'] or '-'}); read in full by some variable: {row['tested']} "
          f"({row['tested_values'] or '-'}); under a count (every bit given back): {row['counted']} "
          f"({row['counted_values'] or '-'}, {row['counted_links']} links)")
    print(f"  dead values {row['dead_values']}; pairs {row['pairs']}; binary-coded choices {row['codes']}")
    print(f"  readers: threshold {row['readers_threshold']}, = {row['readers_eq']}, != {row['readers_ne']}, set {row['readers_set']}, "
          f"partial {row['readers_partial']}")
    print(f"  links to add {row['links']}: equal {row['link_equal']}, bound {row['link_bound']}, hole {row['link_hole']}, dead "
          f"{row['link_dead']}, unit {row['link_unit']}, pair {row['link_pair']}; there already {row['there_already']}")
    print(f"  checkable as they stand {row['as_it_stands']}, in steps {row['in_steps']} ({row['step_clauses']} step clauses), "
          f"not proved here {row['not_proved']}")
    if row["partial_sampled"]:
        print(f"  partial readers: {row['partial_sampled']} tried after the links, {row['partial_missing']} clauses still missing")
    if a.json:
        Path(a.json).write_text(json.dumps({
            "row": row,
            "numbers": [{"lits": n.lits, "dead": sorted(n.dead)} for n in r.numbers],
            "links": [[k, list(c), h] for k, c, h in r.links],
            "steps": [[list(c), list(to) if to else None] for c, to in r.steps_added],
            "codes": [[list(vs), [[t, list(pat)] for t, pat in tests]] for vs, tests in r.codes]}))


def _census_one(job):
    path, budget, seconds, mem_mb, max_clauses = job
    import gc
    import resource
    import signal
    stem = _stem(path)
    if mem_mb:
        resource.setrlimit(resource.RLIMIT_AS, (mem_mb << 20, mem_mb << 20))

    def alarm(_s, _f):
        raise TimeoutError()

    signal.signal(signal.SIGALRM, alarm)
    signal.alarm(seconds)
    t0 = time.time()
    row, err = None, ""
    try:
        # the census reads the two-literal clauses only (propagation is not asked
        # for hidden links), leaves a flood of two-member chains alone, and
        # tries pairs only between numbers under a count
        r, row = read_file(path, budget=budget, max_clauses=max_clauses, full_scan=0, pairs="counted", short_limit=SHORT_LIMIT)
        del r
    except TimeoutError:
        err = "time"
    except MemoryError:
        err = "memory"
    except Exception as e:      # one bad file must not end the census
        err = type(e).__name__
    finally:
        signal.alarm(0)
    if row is None:
        # the failed reading's memory is free only now, after the handler: make the row here
        gc.collect()
        row = {"stem": stem, "seconds": f"{time.time() - t0:.1f}", "error": err}
    return row


HOLDOUT = ROOT / "benchmarks" / "sat-comp-2026"
SOLVER = ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"
# the solver with every other pass and engine off: the number pass alone reads
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0", "--structdiv=0",
         "--structdd=0", "--gauss=0", "--card=0", "--structpoints=0", "--freewalk=0", "--ctlsplit=0"]
# the census by the solver's own pass (src/structnum.rs): its report line, field for field
SOLVER_COLUMNS = ["stem", "seconds", "status", "numbers", "values", "counted", "countedvalues", "added", "equal", "bound", "dead", "unit",
                  "steps", "there", "noproof", "holes", "partial", "candidates", "free", "gates", "ring", "notbit", "ends",
                  "long", "manypaths", "codes", "codetests", "codebits", "work", "error"]


def solver_row(job):
    """One file through the solver's number pass (--structnum=1, the search
    stopped at once): the pass's report line as a row."""
    path, solver, seconds, mem_mb, extra = job
    import resource
    import subprocess
    stem = _stem(path)
    if is_holdout(path):
        return {"stem": stem, "error": "holdout"}

    def limit():
        if mem_mb:
            resource.setrlimit(resource.RLIMIT_AS, (mem_mb << 20, mem_mb << 20))

    t0 = time.time()
    try:
        p = subprocess.run([str(solver), "-n", *STOCK, "--structnum=1", "--conflicts=0", *extra, str(path)],
                           capture_output=True, text=True, timeout=seconds, preexec_fn=limit)
    except subprocess.TimeoutExpired:
        return {"stem": stem, "seconds": f"{time.time() - t0:.1f}", "error": "time"}
    row = {"stem": stem, "seconds": f"{time.time() - t0:.1f}", "error": ""}
    lines = [l for l in p.stdout.splitlines() if l.startswith("c numbers ")]
    if len(lines) != 1:
        off = [l for l in p.stdout.splitlines() if l.startswith("c structure pass off")]
        row["error"] = "too many clauses for the structure pass" if off else f"no report line (exit {p.returncode})"
        return row
    for kv in lines[0].split()[2:]:
        k, _eq, v = kv.partition("=")
        if k in SOLVER_COLUMNS:
            row[k] = v
    return row


def is_holdout(path):
    """The file is in the 2026 suite, by where it really is (a link into the
    suite counts; a path that merely has 2026 in it, such as a log folder
    named by its date, does not)."""
    try:
        Path(path).resolve().relative_to(HOLDOUT.resolve())
        return True
    except ValueError:
        return False


def cmd_census(a):
    files = []
    for f in a.files:
        pth = Path(f)
        files += sorted(pth.glob("*.cnf*")) if pth.is_dir() else [pth]
    if any(is_holdout(f) for f in files):
        sys.exit("the 2026 suite is the holdout: not read (plan section 8)")
    columns = SOLVER_COLUMNS if a.solver else COLUMNS
    out = Path(a.out)
    done = {}
    new_file = not out.exists() or out.stat().st_size == 0     # a run stopped before its first row leaves an empty file
    if not new_file:
        with open(out, newline="") as fh:
            reader = csv.DictReader(fh, delimiter="\t")
            if reader.fieldnames != columns:
                sys.exit(f"{out}: not a table of this census (other columns); give another --out")
            for r in reader:
                done[r["stem"]] = r
    todo = [f for f in files if _stem(f) not in done]
    print(f"{len(files)} files, {len(done)} already in {out}, {len(todo)} to read", flush=True)
    import multiprocessing
    with open(out, "a", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=columns, delimiter="\t", lineterminator="\n", restval="")
        if new_file:
            w.writeheader()
            fh.flush()
        if a.solver:
            jobs = [(str(f), a.solver, a.seconds, a.mem_mb, a.solver_args.split()) for f in todo]
            worker = solver_row
        else:
            jobs = [(str(f), a.budget, a.seconds, a.mem_mb, a.max_clauses) for f in todo]
            worker = _census_one
        with multiprocessing.get_context("fork").Pool(a.workers, maxtasksperchild=1) as pool:
            for n, row in enumerate(pool.imap_unordered(worker, jobs), 1):
                w.writerow(row)
                fh.flush()
                if n % 20 == 0 or n == len(jobs):
                    print(f"  {n} of {len(jobs)}", flush=True)
    print(f"wrote {out}")


def cmd_report(a):
    """The census tables, family by family: which cells carry numbers under a
    count, and how many clauses the encoding lacks there.  Reads the tables
    of this tool's own census and of the solver's (census --solver)."""
    fam, author = {}, {}
    f = ROOT / "benchmarks" / "rl" / "families.tsv"
    if f.exists():
        with open(f, newline="") as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                fam[r["stem"].split("-")[0]] = r["family"]
    f = ROOT / "benchmarks" / "rl" / "prior_tracks.tsv"
    if f.exists():
        with open(f, newline="") as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                fam.setdefault(r["hash"], r["family"])
                author[r["hash"]] = r["author"]
    rows = []
    for t in a.tables:
        with open(t, newline="") as fh:
            rows += list(csv.DictReader(fh, delimiter="\t"))
    num = lambda r, k: int(r[k]) if r.get(k, "") not in ("", None) else 0
    # the two table kinds name things a little differently
    links = lambda r: num(r, "counted_links") if "counted_links" in r else num(r, "added")
    all_links = lambda r: num(r, "links") if "links" in r else num(r, "added")
    # the values of the numbers under a count (not of every chain): each table kind has that column
    values = lambda r: (r.get("counted_values") if "counted_values" in r else r.get("countedvalues", "")) or ""
    stopped = sum(1 for r in rows if r.get("stopped") or r.get("status") in ("stopped", "no-budget"))
    print(f"{len(rows)} files; not read: {dict(Counter(r['error'] for r in rows if r['error']))}; stopped at the limit: {stopped}")
    read = [r for r in rows if not r["error"]]
    print(f"read: {len(read)}; with a chain: {sum(1 for r in read if num(r, 'numbers'))}; with a number under a count: "
          f"{sum(1 for r in read if num(r, 'counted'))}; with clauses to add: {sum(1 for r in read if all_links(r))}")
    if any("work" in r for r in read):
        works = sorted(num(r, "work") for r in read if r.get("work"))
        if works:
            print(f"work of the pass: sum {sum(works) / 1e9:.2f}G, median {works[len(works) // 2] / 1e6:.2f}M, largest {works[-1] / 1e6:.0f}M")
    by = defaultdict(list)
    for r in read:
        h = r["stem"].split("-")[0]
        by[(fam.get(h, "?"), author.get(h, ""))].append(r)
    print("\nfamily\tauthor\tcells\twith_counted\tcounted_numbers\tvalues\tclauses_to_add\texample")
    out = []
    for (fm, au), rs in by.items():
        hit = [r for r in rs if num(r, "counted") >= a.min_numbers]
        if not hit:
            continue
        vals = Counter()
        for r in hit:
            for part in values(r).replace(",", " ").split():
                v, c = part.split("x")
                vals[int(v)] += int(c)
        out.append((-sum(links(r) for r in hit), fm, au, len(rs), len(hit), sum(num(r, "counted") for r in hit),
                    " ".join(f"{v}x{c}" for v, c in sorted(vals.items())[:8]), hit[0]["stem"].split("-", 1)[-1][:40]))
    for neg, fm, au, n, h, cn, vals, ex in sorted(out):
        print(f"{fm}\t{au}\t{n}\t{h}\t{cn}\t{vals}\t{-neg}\t{ex}")


def cmd_pyramid(a):
    """This reader against the puzzle's own reader (tools/pyramid_read.py), on
    one Bel's Pyramid cell: are the labels found, and does a placement set
    its label bits once the links are in."""
    import pyramid_read as pr
    t0 = time.time()
    n_vars, n_cls, clauses, truncated = read_cnf(a.file)
    r = Reading(n_vars, clauses, budget=a.budget, partial=False, pairs=False)
    if r.truncated:
        sys.exit("the reading stopped at its budget")
    if r.p.root_conflict:
        sys.exit("the formula is refuted from the start")
    pyr = pr.Reading([tuple(c) for c in clauses], a.N)
    p0 = Prop(n_vars, clauses)
    p1 = Prop(n_vars, clauses)
    for c in r.link_clauses():
        p1.add(list(c))
    exact = inside = small = 0
    lost = []
    for key, full in sorted(pyr.order.items()):
        # an option can fix some of a label's bits; the links say which
        seq = [l for l in full if p1.value(l) == 0]
        if len(seq) < 2:
            small += 1
            continue
        hit = 0
        for num in r.numbers:
            for ch in (num.lits, [-l for l in reversed(num.lits)]):
                if any(ch[k:k + len(seq)] == seq for k in range(len(ch) - len(seq) + 1)):
                    hit = 1 if len(ch) == len(seq) else 2
        exact += hit == 1
        inside += hit == 2
        if not hit:
            lost.append((key, seq))
    extra, _counts = pyr.implied({"fwd"})
    rng = random.Random(a.seed)
    sample = extra if a.sample <= 0 or a.sample >= len(extra) else rng.sample(extra, a.sample)
    before = after = 0
    missed = []
    for neg_p, lit in sample:
        got = p0.implied([-neg_p])
        before += got is None or lit in got or p0.value(lit) == 1
        got = p1.implied([-neg_p])
        if got is None or lit in got or p1.value(lit) == 1:
            after += 1
        else:
            missed.append((-neg_p, lit))
    s, lens = r.summary()
    wide = len(pyr.order) - small
    print(f"{_stem(a.file)}: {time.time() - t0:.0f} s")
    print(f"  the puzzle's reader: {len(pyr.order)} labels, {wide} with two or more open bits; this reader: {len(r.numbers)} numbers ({lens})")
    print(f"  labels found as a chain: {exact + inside} of {wide} ({exact} exactly, {inside} with a member more at an end)")
    for key, seq in lost[:5]:
        print(f"    not found: {key} {seq}")
    print(f"  links: {len(r.links)} ({dict(Counter(k for k, _c, _h in r.links))}), against {len(extra)} placement links of the puzzle's reader")
    print(f"  a placement sets its label bit ({len(sample)} sampled): {before} before, {after} with the links")
    for pl, lit in missed[:5]:
        print(f"    still unset: placement {pl} -> {lit}")
    if exact + inside != wide or after != len(sample):
        sys.exit(1)


def cmd_variant(a):
    kinds = set(a.add.split(","))
    known = {"equal", "bound", "hole", "dead", "unit", "pair", "partial"}
    if not kinds or kinds - known:
        sys.exit(f"--add takes a list from {sorted(known)}")
    n_vars, n_cls, clauses, truncated = read_cnf(a.file)
    if truncated:
        sys.exit(f"{a.file}: more than {MAX_CLAUSES} clauses, not read")
    if any(not c for c in clauses):
        sys.exit(f"{a.file}: an empty clause; the file is unsatisfiable as written, not read")
    top = max((abs(l) for c in clauses for l in c), default=0)
    pairs = False if "pair" not in kinds else True if a.all_pairs else "counted"
    r = Reading(max(n_vars, top), clauses, budget=a.budget, partial="partial" in kinds, pairs=pairs,
                kinds=kinds - {"partial"})
    if r.truncated:
        sys.exit(f"{a.file}: the reading stopped at its budget; no variant written")
    extra = r.link_clauses()
    tmp = a.out + ".tmp"
    with open(tmp, "w") as f:
        f.write(f"p cnf {max(n_vars, top)} {len(clauses) + len(extra)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")
        for c in extra:
            f.write(" ".join(map(str, c)) + " 0\n")
    os.replace(tmp, a.out)
    s, lens = r.summary()
    print(f"{_stem(a.file)}: numbers {s.get('numbers', 0)} ({lens}), added {len(extra)} clauses "
          f"({dict(Counter(k for _c, k in r.added))}); left out, not asked for: "
          f"{ {k[5:]: v for k, v in s.items() if k.startswith('left_')} or 'nothing'}; wrote {a.out}")


# ---------------------------------------------------------------------------
# selftest
# ---------------------------------------------------------------------------

def _implied(p0, clause):
    return satisfiable(p0, [-l for l in clause]) is None


def renumber(n_vars, clauses, seed):
    """A random renaming of the variables, with random signs."""
    rng = random.Random(seed)
    perm = list(range(1, n_vars + 1))
    rng.shuffle(perm)
    sign = [rng.choice((1, -1)) for _ in range(n_vars)]
    m = lambda l: (perm[abs(l) - 1] * sign[abs(l) - 1]) * (1 if l > 0 else -1)
    out = [[m(l) for l in c] for c in clauses]
    rng.shuffle(out)
    for c in out:
        rng.shuffle(c)
    return out, m


def sequence_problems(p0, n_vars, clauses, r):
    """The clauses a reading added, as a list a proof checker would follow:
    each is true in every model (brute force on `p0`, for a toy), and each is
    refuted by propagation, in the order given, from the formula and the
    clauses before it.  A link the reading calls hard is excused from the
    second test, and only such a link."""
    bad = []
    hard = {c for _k, c, how in r.links if how == "hard"}
    if [c for c, k in r.added if k != "step"] != [c for _k, c, _h in r.links]:
        bad.append("the links and the list of added clauses do not agree")
    pc = Prop(n_vars, clauses)
    for c, kind in r.added:
        if not _implied(p0, c):
            bad.append(f"{kind} {list(c)} is not implied")
        if not pc.refutes([-l for l in c]) and not (kind != "step" and c in hard):
            bad.append(f"{kind} {list(c)} is not refuted by propagation where it stands in the list")
        pc.add(list(c))
    return bad


def check_record(rec, shape_fn, seed=None, kinds=None):
    """Problems of the reading of one catalog record (none is the goal):
    every listed clause is true in every model; every sorted-bit input is a
    chain of the reading (or part of one); and with the clauses added, a test
    alone sets every bit that follows from it, and so does a test with one
    more bit of the same number."""
    clauses = rec["clauses"]
    m = lambda l: l
    if seed is not None:
        clauses, m = renumber(rec["n_vars"], rec["clauses"], seed)
    r = Reading(rec["n_vars"], clauses, partial=True, kinds=kinds)
    bad = []
    if r.truncated:
        return ["the reading stopped at its budget"]
    p0 = Prop(rec["n_vars"], clauses)
    bad += sequence_problems(p0, rec["n_vars"], clauses, r)
    # every sorted-bit input is a number of the reading: each of its bits is a
    # member of one chain, in order, or equal to a member (propagation, on the
    # formula with the links, sets each from the other, both ways)
    chains = []
    for num in r.numbers:
        chains.append(num.lits)
        chains.append([-l for l in reversed(num.lits)])
    after = {}

    pc = Prop(rec["n_vars"], clauses)
    for c in r.link_clauses():
        pc.add(list(c))

    def sets(a, b):
        if a not in after:
            got = pc.implied([a])
            after[a] = set(got) if got is not None else None
        return after[a] is None or b in after[a]

    def same(a, b):
        return a == b or (sets(a, b) and sets(b, a) and sets(-a, -b) and sets(-b, -a))

    for inp in rec["inputs"]:
        if inp["kind"] != "order" or len(inp["vars"]) < 2:
            continue
        # a bit that the formula with the links fixes for good is no part of
        # the number any more (a toy's own constraint can rule values out)
        want = [m(v) for v in inp["vars"] if pc.value(m(v)) == 0]
        if len(want) < 2:
            continue
        found = False
        for ch in chains:
            pos = 0
            for b in want:
                while pos < len(ch) and not same(b, ch[pos]):
                    pos += 1
                if pos == len(ch):
                    break
                pos += 1
            else:
                found = True
                break
        if not found:
            bad.append(f"input {inp['name']}: its bits {want} are no chain of the reading ({[n.lits for n in r.numbers]})")
    # the gap, measured as the catalog measures it, on the formula with the links
    extended = dict(rec)
    back = {}
    if seed is not None:
        back = {abs(m(v)): (v if m(v) > 0 else -v) for v in range(1, rec["n_vars"] + 1)}
        unmap = lambda l: back[abs(l)] * (1 if l > 0 else -1)
    else:
        unmap = lambda l: l
    added = [[unmap(l) for l in c] for c in r.link_clauses()]
    extended["clauses"] = rec["clauses"] + added
    order_lits = {s * v for inp in rec["inputs"] if inp["kind"] == "order" for v in inp["vars"] for s in (1, -1)}
    sh = shape_fn(extended, only=order_lits)
    if sh["0_need"] != sh["0_up"]:
        bad.append(f"a test alone: {sh['0_need']} bit literals follow, propagation sets {sh['0_up']} with the links")
    single = len([i for i in rec["inputs"] if i["kind"] == "order"]) == 1 and len(rec["inputs"]) == 1
    if single and sh["1_need"] != sh["1_up"]:
        bad.append(f"a test and one bit: {sh['1_need']} follow, propagation sets {sh['1_up']} with the links")
    if "hard" in rec and all(i["kind"] == "order" for i in rec["inputs"]) and len(rec["inputs"]) == 2 and sh["h1_need"] != sh["h1_up"]:
        bad.append(f"the constraint and one bit: {sh['h1_need']} follow, propagation sets {sh['h1_up']} with the links")
    return bad


def problem_kinds(bad):
    """sound: a listed clause is not true in every model, or not checkable as
    said (never acceptable).  chain: a sorted-bit input is no chain of the
    reading.  gap0, gap1, pair: with the links added, a test alone, a test
    with one bit, or a constraint with one bit still leaves a bit unset."""
    kinds = set()
    for b in bad:
        if "not implied" in b or "refute" in b or "budget" in b or "do not agree" in b:
            kinds.add("sound")
        elif "no chain" in b:
            kinds.add("chain")
        elif "a test alone" in b:
            kinds.add("gap0")
        elif "a test and one bit" in b:
            kinds.add("gap1")
        elif "the constraint and one bit" in b:
            kinds.add("pair")
        else:
            kinds.add("other")
    return ",".join(sorted(kinds)) or "ok"


_CAT = None


def _catalog_job(job):
    index, seed = job
    import z3_catalog as zc
    rec = _CAT[index]
    status = problem_kinds(check_record(rec, zc.shape, seed))
    # the same record with only a part of the kinds added: whatever is added
    # must still be true, and a list a checker can follow
    part = Reading(rec["n_vars"], rec["clauses"], partial=True, kinds={"bound", "dead", "unit"})
    if sequence_problems(Prop(rec["n_vars"], rec["clauses"]), rec["n_vars"], rec["clauses"], part):
        status = (status + ",sound") if status != "ok" else "sound"
    return rec["id"], seed, status, "hard" in rec, rec["options"]


CATALOG_TABLE = ROOT / "benchmarks" / "rl" / "number_read_catalog.tsv"


def selftest(a):
    # the docstring's example, by hand: g1..g3 are 1..3, T = 4 is "exactly two
    # of the three", written with the helpers 7 = "at least one", 5 = "at
    # least two" and 6 = "all three"
    cl = [[1, -2], [2, -3],
          [-7, 1, 2, 3], [7, -1], [7, -2], [7, -3],
          [-5, 1, 2], [-5, 1, 3], [-5, 2, 3], [5, -1, -2], [5, -1, -3], [5, -2, -3],
          [-6, 1], [-6, 2], [-6, 3], [6, -1, -2, -3],
          [-4, 5], [-4, -6], [4, -5, 6]]
    r = Reading(7, cl, partial=True)
    assert [n.lits for n in r.numbers] == [[1, 2, 3]], [n.lits for n in r.numbers]
    num = r.numbers[0]
    live = num.live()
    assert [kind_of(*num.readers[w], live) for w in (4, 5, 6, 7)] == ["eq", "ge", "ge", "ge"]
    got = {(k, c) for k, c, _h in r.links}
    # the helpers equal bits (7 is g1, 5 is g2, 6 is g3).  "g1 -> 7" and
    # "6 -> g3" are clauses already; 5 lacks both directions.  With the four
    # missing ones in, T's own links follow by propagation
    assert got == {("equal", (-7, 1)), ("equal", (-5, 2)), ("equal", (5, -2)), ("equal", (6, -3))}, got
    assert r.notes["have_equal"] == 2, r.notes
    p = Prop(7, cl + [list(c) for _k, c, _h in r.links])
    assert set(p.implied([4])) >= {1, 2, -3}
    assert all(h == "rup" for _k, _c, h in r.links), r.links
    assert r.link_clauses() == [c for _k, c, _h in r.links] and sequence_problems(Prop(7, cl), 7, cl, r) == []
    # asked for no equalities, the reader leaves the four out, counts them,
    # and so has to add clauses of T's own, which the equalities carried
    # before: "T -> g2" (with it "T -> not g3" follows through h3's clause),
    # and "x = 2 -> T" read from the other side
    only = Reading(7, cl, partial=True, kinds={"bound", "hole", "dead", "unit"})
    assert {(k, c) for k, c, _h in only.links} == {("bound", (-4, 2)), ("hole", (4, -2, 3))}, only.links
    assert only.notes["left_equal"] == 4 and sequence_problems(Prop(7, cl), 7, cl, only) == []
    assert set(Prop(7, cl + only.link_clauses()).implied([4])) >= {1, 2, -3}
    # a reader that every value sets the same way is a unit clause: here 3 is
    # on whether x is 0, 1 or 2, and propagation does not see it from the
    # start.  The proof walks the values: (3 or x > 0), (3 or x > 1), then (3)
    const = [[1, -2], [-3, 1, -2], [3, -1], [3, 2]]
    rc = Reading(3, const, partial=True)
    assert ("unit", (3,)) in {(k, c) for k, c, _h in rc.links}, rc.links
    assert sequence_problems(Prop(3, const), 3, const, rc) == [], sequence_problems(Prop(3, const), 3, const, rc)
    assert Prop(3, const + rc.link_clauses()).value(3) == 1
    # an end bit that nothing reads is no part of the number: without the
    # helper 7 nothing tells x = 0 from x = 1, and the chain is g2, g3
    bare = [c for c in cl if 7 not in c and -7 not in c]
    assert [n.lits for n in Reading(7, bare, partial=True).numbers] == [[2, 3]]
    # the same reading when the partial readers are only sampled, or left alone
    for mode in ("sample", False):
        assert {(k, c) for k, c, _h in Reading(7, cl, partial=mode).links} == got
    # kinds
    assert kind_of(0b0100, 0b1011, [0, 1, 2, 3]) == "eq" and kind_of(0b1011, 0b0100, [0, 1, 2, 3]) == "ne"
    assert kind_of(0b1100, 0b0011, [0, 1, 2, 3]) == "ge" and kind_of(0b0011, 0b1100, [0, 1, 2, 3]) == "le"
    assert kind_of(0b0101, 0b1010, [0, 1, 2, 3]) == "set" and kind_of(0b0100, 0b0011, [0, 1, 2, 3]) == "partial"
    assert kind_of(0b1111, 0, [0, 1, 2, 3]) == "constant" and kind_of(0b0100, 0b1001, [0, 2, 3]) == "eq"
    # a guard (G = 4, G -> x = 1) is a partial reader: its two links are there
    # already, as its own clauses.  Nothing tells x = 2 from x = 3, so the top
    # bit is no part of the number
    guard = [[1, -2], [2, -3], [-4, 1], [-4, -2]]
    r = Reading(4, guard, partial=True)
    assert [n.lits for n in r.numbers] == [[1, 2]] and r.links == [], (r.numbers, r.links, r.notes)
    # a member that is fixed from the start is no bit: here g3 is off by a unit
    # that propagation does not see at once (two clauses on a helper), the bit
    # test learns it and the chain is g1, g2
    fixed = [[1, -2], [2, -3], [-3, 8], [-3, -8],
             [-7, 1, 2, 3], [7, -1], [7, -2], [7, -3], [-5, 1, 2], [5, -1, -2], [-5, 2], [-5, 1]]
    rf = Reading(8, fixed, partial=True)
    assert all(3 not in n.lits and -3 not in n.lits for n in rf.numbers), [n.lits for n in rf.numbers]
    assert sequence_problems(Prop(8, fixed), 8, fixed, rf) == []
    # a value no model takes (x = 2 is ruled out by "not g2 or g3"): the chain
    # loses the bit that the other bits now set, and nothing false is listed
    dead = [[1, -2], [2, -3], [-2, 3], [4, 1], [-4, -1]]
    r = Reading(4, dead, partial=True)
    p0 = Prop(4, dead)
    assert all(_implied(p0, c) for _k, c, _h in r.links), r.links
    # a formula with no chain, one refuted from the start, one past its budget
    r = Reading(3, [[1, 2, 3], [-1, -2], [-1, -3], [-2, -3]])
    assert r.links == []
    dead_start = Reading(1, [[1], [-1]])
    assert dead_start.notes["root_conflict"] == 1 and dead_start.find_codes() == [] and dead_start.link_clauses() == []
    assert row_of("x", 1, 2, False, dead_start, 0.0)["error"] == "root conflict"
    assert Reading(7, cl, budget=3).truncated == "budget"
    # a chain too long to read is left alone, helpers and all: no clause, and
    # the end test does not trim it down to a length that fits (it knows
    # nothing of a chain it did not read)
    long_chain = [[k, -(k + 1)] for k in range(1, 12)] + [[-13, 3, 7], [13, -3], [13, -7], [14, -5, -9], [-14, 5], [-14, 9]]
    assert {c for _k, c, _h in Reading(14, long_chain, partial=True).links} == {(-13, 3), (14, -9)}
    for bits in (8, 3):
        short = Reading(14, long_chain, partial=True, max_bits=bits)
        assert short.links == [] and short.numbers == [] and short.notes["too_long"] >= 1, (bits, short.links, short.notes)
    # the small search
    assert small_sat([]) and not small_sat([[]]) and not small_sat([[1], [-1]])
    assert small_sat([[1, 2], [-1, 2], [1, -2]]) and not small_sat([[1, 2], [-1, 2], [1, -2], [-1, -2]])
    # reading a file as it is written, empty clause and all
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        f = Path(d) / "t.cnf"
        f.write_text("c x\np cnf 3 4\n1 -2 0\n0\n2 3\n 0\n-1 0\n")
        assert read_cnf(f) == (3, 4, [[1, -2], [], [2, 3], [-1]], False)
        assert read_cnf(f, max_clauses=2)[3] is True
        # the census: a run that was stopped before its first row leaves an
        # empty table; the next run must still write the header, or `report`
        # would take the first row for the column names
        ex = Path(d) / "ex.cnf"
        ex.write_text("p cnf 7 %d\n" % len(cl) + "".join(" ".join(map(str, c)) + " 0\n" for c in cl))
        table = Path(d) / "census.tsv"
        table.write_text("")
        args = argparse.Namespace(files=[str(ex)], out=str(table), workers=1, budget=DEFAULT_BUDGET, seconds=120, mem_mb=0,
                                  max_clauses=MAX_CLAUSES, solver="")
        import contextlib
        import io
        with contextlib.redirect_stdout(io.StringIO()):
            cmd_census(args)
            cmd_census(args)            # a second run adds nothing: the row is there
        with open(table, newline="") as fh:
            got_rows = list(csv.DictReader(fh, delimiter="\t"))
        assert len(got_rows) == 1 and got_rows[0]["stem"] == "ex" and got_rows[0]["numbers"] == "1" and got_rows[0]["links"] == "4", got_rows
        # the census by the solver's pass: a stand-in that prints the report
        # line, one that prints none, and a table of the other kind refused
        fake = Path(d) / "fake.sh"
        fake.write_text("#!/bin/sh\necho 'c numbers status=read numbers=2 values=3x1,7x1 counted=1 countedvalues=7x1 added=5 equal=4 work=77'\n")
        fake.chmod(0o755)
        st = Path(d) / "solver.tsv"
        sargs = argparse.Namespace(files=[str(ex)], out=str(st), workers=1, budget=0, seconds=60, mem_mb=0, max_clauses=0,
                                   solver=str(fake), solver_args="--structclauses=5")
        with contextlib.redirect_stdout(io.StringIO()):
            cmd_census(sargs)
        with open(st, newline="") as fh:
            srows = list(csv.DictReader(fh, delimiter="\t"))
        assert len(srows) == 1 and srows[0]["counted"] == "1" and srows[0]["values"] == "3x1,7x1" and srows[0]["work"] == "77", srows
        assert srows[0]["countedvalues"] == "7x1"
        fake.write_text("#!/bin/sh\necho 's UNKNOWN'\n")
        assert "no report line" in solver_row((str(ex), str(fake), 60, 0, []))["error"]
        assert solver_row((str(ROOT / "benchmarks" / "sat-comp-2026" / "x.cnf"), str(fake), 60, 0, []))["error"] == "holdout"
        sargs.out = str(table)
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                cmd_census(sargs)
            raise AssertionError("a table of the other census was added to")
        except SystemExit as e:
            assert "other columns" in str(e)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            cmd_report(argparse.Namespace(tables=[str(st), str(table)], min_numbers=1))
        assert "2 files" in buf.getvalue() and "with a number under a count: 2" in buf.getvalue(), buf.getvalue()
        # the family line gives the values of the numbers under a count, not of every chain:
        # the reader's four-valued number and the stand-in's seven-valued one, and not the
        # stand-in's three-valued chain that no count reads
        assert "\t4x1 7x1\t" in buf.getvalue() and "3x1" not in buf.getvalue(), buf.getvalue()
        other = Path(d) / "other.tsv"
        other.write_text("a\tb\n1\t2\n")
        args.out = str(other)
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                cmd_census(args)
            raise AssertionError("a table with other columns was added to")
        except SystemExit as e:
            assert "other columns" in str(e)
    # renumbering keeps the formula: the reading of the example is the same up
    # to names, and so is what the census says of the number (three cut
    # points with a variable equal to them, one value test), whichever sign a
    # renaming gives the test variable
    assert profile_of(Reading(7, cl, partial=True).numbers[0]) == (4, 3, 1)
    for seed in range(1, 9):
        ren, m = renumber(7, cl, seed)
        r2 = Reading(7, ren, partial=True)
        assert len(r2.numbers) == 1 and len(r2.links) == 4 and profile_of(r2.numbers[0]) == (4, 3, 1), (seed, profile_of(r2.numbers[0]))
    # the holdout is told by where a file is, not by a 2026 in its path
    assert is_holdout(ROOT / "benchmarks" / "sat-comp-2026" / "x.cnf.xz")
    assert not is_holdout(ROOT / "log" / "sorted-bits-2026-11-18" / "variants" / "x.cnf")
    assert not is_holdout(ROOT / "benchmarks" / "sat-comp-2025" / "x-2026.cnf")
    # and every command that reads a file refuses one of the holdout, before opening it
    for call in (lambda q: read_cnf(q), lambda q: read_file(q),
                 lambda q: cmd_one(argparse.Namespace(file=q, budget=1, json="", all_partial=False)),
                 lambda q: cmd_variant(argparse.Namespace(file=q, add="equal", out="/dev/null", budget=1, all_pairs=False)),
                 lambda q: cmd_pyramid(argparse.Namespace(file=q, N=4, sample=1, seed=1, budget=1))):
        try:
            call(str(ROOT / "benchmarks" / "sat-comp-2026" / "no-such-file.cnf.xz"))
            raise AssertionError("a holdout path was read")
        except SystemExit as e:
            assert "holdout" in str(e), e
    # the catalog, plain and renumbered with signs flipped
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import z3_catalog as zc
    global _CAT
    _CAT = [rec for rec in zc.load(a.catalog) if any(i["kind"] == "order" and len(i["vars"]) >= 2 for i in rec["inputs"])]
    # binary-coded choices: every one in the catalog is read, with its value tests
    coded = 0
    for rec in zc.load(a.catalog):
        wide = [i for i in rec["inputs"] if i["kind"] == "binary" and len(i["values"]) >= 3]
        if not wide:
            continue
        codes = Reading(rec["n_vars"], rec["clauses"], partial=True).find_codes()
        for i in wide:
            hit = [tests for vs, tests in codes if set(vs) == set(i["vars"])]
            assert hit and len(hit[0]) >= 3, (rec["id"], i["name"], codes)
            # each test's pattern is one of the input's values
            assert all(sorted(pat, key=abs) in [sorted(q, key=abs) for q in i["patterns"]] for _t, pat in hit[0]), (rec["id"], hit[0])
            coded += 1
    assert coded >= 15, coded
    jobs = [(i, seed) for i in range(len(_CAT)) for seed in (None, 1, 2)]
    import multiprocessing
    with multiprocessing.get_context("fork").Pool(a.jobs) as pool:
        rows = pool.map(_catalog_job, jobs, chunksize=4)
    failed = 0
    lines = ["id\trenumbered\tstatus"]
    by_opt = defaultdict(lambda: [0, 0])
    for rid, seed, status, hard, options in rows:
        lines.append(f"{rid}\t{0 if seed is None else seed}\t{status}")
        opt = options[0]
        by_opt[opt][0] += status == "ok"
        by_opt[opt][1] += 1
        if "sound" in status or "other" in status:
            failed += 1
            print(f"FAIL {rid} renumbered {seed}: {status} (a listed clause must be true and checkable, always)")
        elif status != "ok" and "base" in options and not hard:
            failed += 1
            print(f"FAIL {rid} renumbered {seed}: {status} (the base setting, no constraint of its own: must read fully)")
    text = "\n".join(lines) + "\n"
    total_ok = sum(1 for r in rows if r[2] == "ok")
    print(f"catalog: {len(rows)} readings ({len(_CAT)} records, plain and renumbered twice), {total_ok} read fully")
    print("  by first option set (read fully of readings): " + ", ".join(f"{k} {v[0]}/{v[1]}" for k, v in sorted(by_opt.items())))
    if a.write:
        CATALOG_TABLE.write_text(text)
        print(f"wrote {CATALOG_TABLE}")
    elif Path(a.catalog).resolve() == (ROOT / "benchmarks" / "rl" / "z3_catalog.jsonl.xz").resolve():
        if not CATALOG_TABLE.exists() or CATALOG_TABLE.read_text() != text:
            failed += 1
            old = dict((tuple(l.split("\t")[:2]), l.split("\t")[2]) for l in CATALOG_TABLE.read_text().splitlines()[1:]) if CATALOG_TABLE.exists() else {}
            diff = [(l.split("\t")[0], l.split("\t")[1], old.get(tuple(l.split("\t")[:2])), l.split("\t")[2])
                    for l in lines[1:] if old.get(tuple(l.split("\t")[:2])) != l.split("\t")[2]]
            print(f"FAIL: {len(diff)} readings differ from {CATALOG_TABLE.name} (--write records the new state): {diff[:6]}")
    if failed:
        sys.exit(1)
    print("number_read selftest ok")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    o = sub.add_parser("one")
    o.add_argument("file")
    o.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    o.add_argument("--json", default="")
    o.add_argument("--all-partial", action="store_true", help="link every partial reader (slow), not a sample")
    c = sub.add_parser("census")
    c.add_argument("files", nargs="+", help="files, or folders of *.cnf*")
    c.add_argument("--out", required=True)
    c.add_argument("--workers", type=int, default=16)
    c.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    c.add_argument("--seconds", type=int, default=3600)
    c.add_argument("--mem-mb", type=int, default=16000)
    c.add_argument("--max-clauses", type=int, default=MAX_CLAUSES)
    c.add_argument("--solver", default="", help="read with the solver's own number pass (this binary) instead of this tool: "
                   "seconds a file, not minutes; the table then has the pass's report fields")
    c.add_argument("--solver-args", default="", help="more options for that binary, such as --structclauses=60000000 "
                   "to read files past the structure stage's usual limit of 10M clauses")
    rp = sub.add_parser("report")
    rp.add_argument("tables", nargs="+")
    rp.add_argument("--min-numbers", type=int, default=1, help="list a family when a cell has this many numbers under a count")
    y = sub.add_parser("pyramid")
    y.add_argument("file")
    y.add_argument("N", type=int)
    y.add_argument("--sample", type=int, default=3000)
    y.add_argument("--seed", type=int, default=1)
    y.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    v = sub.add_parser("variant")
    v.add_argument("file")
    v.add_argument("--add", required=True)
    v.add_argument("--out", required=True)
    v.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    v.add_argument("--all-pairs", action="store_true",
                   help="with pair: try every two numbers that share a reader, not only numbers under a count")
    s = sub.add_parser("selftest")
    s.add_argument("--catalog", default=str(ROOT / "benchmarks" / "rl" / "z3_catalog.jsonl.xz"))
    s.add_argument("--jobs", type=int, default=8)
    s.add_argument("--write", action="store_true", help="record the per-record state as the table to hold to")
    a = ap.parse_args(argv)
    {"one": cmd_one, "variant": cmd_variant, "selftest": selftest, "census": cmd_census, "pyramid": cmd_pyramid,
     "report": cmd_report}[a.cmd](a)


if __name__ == "__main__":
    main()
