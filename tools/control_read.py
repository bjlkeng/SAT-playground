#!/usr/bin/env python3
"""Control reader by meaning (bead SAT-playground-d96.8.1): the inputs that steer
a formula's switches, however the switches were written.

A switch is a signal o = s ? a : b, s its select.  The first reader
(tools/control_census.py) knows one way of writing it: four ternary clauses, or
three two-input ANDs, with outputs numbered after their inputs.  This one reads
switches by what they compute:

  gates     definitions by truth table (tools/netlist_census.py recover_gates:
            any clause shape of up to six inputs, any numbering), or the gate
            each clause's first literal names when every group reads that way
            (ABC write_cnf, STP); then wide AND/OR gates of any width (a clause
            and its binaries) and one-way halves (Plaisted-Greenbaum: v -> f(X)
            or f(X) -> v), all settled forward from the inputs so that the
            result has no cycle
  cuts      each gate's own fan-ins, then its fan-in gates folded in, up to two
            levels, while the leaves stay at most six
  switches  a leaf s selects when fixing it either way leaves a single other
            leaf, a different one each way (a 2:1 switch); a pair of leaves
            selects when fixing both leaves a single leaf each time, two or more
            different ones (a 4:1 switch: one LUT, or a decoded read); a leaf
            that leaves a constant one way and a leaf the other way is an
            enable (an AND's two inputs look alike, so enables are counted
            apart and judged by the effect test, bead d96.8.3)
  control   each switch's selects followed back to the inputs (shared memo,
            capped at 200); an input's weight is the number of switches it
            steers; cover K is the share of switches whose selects' support
            lies inside the K heaviest inputs

Every step is counted against one step budget; when it runs out the reading
stops where it is and the row says so (budget 1).  The definition readers
cannot stop midway, so their cost (four steps a literal) is reserved before
they start (four steps a literal, one a variable number): a budget too
small for the file stops at once.

Usage:
  python3 tools/control_read.py cell FILE [--max-clauses N] [--budget STEPS]
  python3 tools/control_read.py census --out TABLE.tsv [--stems a,b] [--workers 6] [--cands]
                                       [--suite DIR] [--max-clauses 5000000] [--seconds 1800]
                                       [--mem-mb 40000]
  python3 tools/control_read.py cands FILE [--max-clauses N] [--budget STEPS] [--gd-pool 64]
  python3 tools/control_read.py toy KIND STYLE --out FILE.cnf [--shuffle SEED]
  python3 tools/control_read.py selftest

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import heapq
import os
import random
import resource
import signal
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import function_census as fc  # noqa: E402
import netlist_census as nc  # noqa: E402
from rl_structure import parse_prefix  # noqa: E402

CAP = 200                 # select supports larger than this count as wide
MAX_LEAVES = 6            # cut size
MAX_HALF_CLAUSES = 16     # a one-way half is read from at most this many clauses
KS = (8, 16, 32)
DEFAULT_BUDGET = 2_000_000_000


class Budget(Exception):
    """The step budget ran out."""


class Steps:
    def __init__(self, limit):
        self.limit = limit
        self.n = 0

    def take(self, k=1):
        self.n += k
        if self.n > self.limit:
            raise Budget()


# ---------------------------------------------------------------------------
# truth tables over n slots (2^n bits, n <= 6; bit m is the value when slot i
# takes bit i of m)
# ---------------------------------------------------------------------------


def _slot_masks(n):
    out = []
    for i in range(n):
        m = 0
        for a in range(1 << n):
            if (a >> i) & 1:
                m |= 1 << a
        out.append(m)
    return out


SLOT = {n: _slot_masks(n) for n in range(MAX_LEAVES + 1)}
FULL = {n: (1 << (1 << n)) - 1 for n in range(MAX_LEAVES + 1)}


def cofactor(t, n, i, b):
    """t with slot i fixed to b (still a table over n slots, slot i ignored)."""
    m = SLOT[n][i]
    s = 1 << i
    if b:
        hi = t & m
        return hi | (hi >> s)
    lo = t & (FULL[n] ^ m)
    return lo | (lo << s)


def as_leaf(t, n):
    """('c', 0/1) for a constant, ('l', slot, positive) for a single slot's
    literal, None otherwise."""
    if t == 0:
        return ("c", 0)
    if t == FULL[n]:
        return ("c", 1)
    for j in range(n):
        if t == SLOT[n][j]:
            return ("l", j, True)
        if t == FULL[n] ^ SLOT[n][j]:
            return ("l", j, False)
    return None


def depends(t, n, i):
    return cofactor(t, n, i, 0) != cofactor(t, n, i, 1)


def switch_kind(t, n):
    """(kind, selects): kind 'mux' with one select slot (2:1), 'mux4' with a
    pair of select slots (4:1), 'enable' with one slot, or None."""
    enable = None
    for i in range(n):
        if not depends(t, n, i):
            continue
        a = as_leaf(cofactor(t, n, i, 0), n)
        b = as_leaf(cofactor(t, n, i, 1), n)
        if a is None or b is None:
            continue
        if a[0] == "l" and b[0] == "l":
            if a[1] != b[1] and a[1] != i and b[1] != i:
                return "mux", (i,)
        elif (a[0] == "c") != (b[0] == "c"):
            lit = a if a[0] == "l" else b
            if lit[1] != i:
                enable = (enable or ()) + (i,)
    for i in range(n):
        for j in range(i + 1, n):
            if not (depends(t, n, i) and depends(t, n, j)):
                continue
            leaves = set()
            signs = set()
            ok = True
            for bi in (0, 1):
                for bj in (0, 1):
                    c = as_leaf(cofactor(cofactor(t, n, i, bi), n, j, bj), n)
                    # four data leaves, no constant: x AND (s ? a : b) is an
                    # enable on a switch, not a 4:1 switch on (x, s)
                    if c is None or c[0] == "c" or c[1] in (i, j):
                        ok = False
                        break
                    if c[0] == "l":
                        leaves.add(c[1])
                        signs.add((c[1], c[2]))
                if not ok:
                    break
            # a data leaf seen with both signs means one of the pair flips
            # polarity (an XOR of a switch with another signal), not a select
            if ok and len(leaves) >= 2 and len(signs) == len(leaves):
                return "mux4", (i, j)
    if enable is not None:
        # every input that passes: an AND's two inputs look alike, and only
        # their shared use (the votes in coverage) tells the enable apart
        return "enable", enable
    return None, ()


def shrink(sup, t):
    """(support, table) with the inputs the table ignores removed."""
    sup = list(sup)
    n = len(sup)
    i = n - 1
    while i >= 0:
        if not depends(t, n, i):
            # drop slot i: the table over the other n - 1 slots
            nt = 0
            for m in range(1 << (n - 1)):
                full_m = ((m >> i) << (i + 1)) | (m & ((1 << i) - 1))
                if (t >> full_m) & 1:
                    nt |= 1 << m
            t = nt
            del sup[i]
            n -= 1
        i -= 1
    return tuple(sup), t


def eval_table(t, k, ins, width_full):
    """The value of a k-input table over input masks `ins` (ints of a common
    width whose all-ones value is width_full)."""
    acc = 0
    for m in range(1 << k):
        if not (t >> m) & 1:
            continue
        term = width_full
        for i in range(k):
            term &= ins[i] if (m >> i) & 1 else (width_full ^ ins[i])
            if not term:
                break
        acc |= term
    return acc


def table_of_clauses(rests, xs):
    """AND over the clause rests (lists of literals over the variables xs) as
    a table over the sorted xs."""
    n = len(xs)
    pos = {v: i for i, v in enumerate(xs)}
    t = FULL[n]
    for rest in rests:
        c = 0
        for l in rest:
            m = SLOT[n][pos[abs(l)]]
            c |= m if l > 0 else FULL[n] ^ m
        t &= c
    return t


# ---------------------------------------------------------------------------
# gates: definitions, wide gates, one-way halves, settled forward
# ---------------------------------------------------------------------------


PARITY_TT = {2: {0b0110, 0b1001}, 3: {0x96, 0x69}}


def orient_forward(gates):
    """In a file numbered in circuit order a gate reads only smaller
    variables.  The netlist reader breaks the direction of an XOR clause set
    (any of its variables can be read as the output) by index only at a
    deadlock, so a few come out backwards, and on the oisc cells they land on
    the control word (982 = 1231 xor 1257).  A backward parity set is turned
    to its largest variable when that one is still free; any other backward
    definition is dropped (its output becomes an input)."""
    out = {}
    turned = []
    for v, (sup, tt, cl) in gates.items():
        if all(u < v for u in sup):
            out[v] = (sup, tt, cl)
        elif len(sup) in PARITY_TT and tt in PARITY_TT[len(sup)]:
            turned.append((v, sup, tt, cl))
    for v, sup, tt, cl in turned:
        w = max(sup)
        if w in out:
            continue
        new_sup = tuple(sorted([u for u in sup if u != w] + [v]))
        # xor and xnor keep their table whichever variable is the output
        out[w] = (new_sup, tt, cl)
    return out


MAX_REFILL_STARTS = 8


def refill_forward(gates, clauses, steps):
    """After orient_forward: a variable left without a definition, in a file
    numbered in circuit order, gets one from the clauses no gate uses when
    some of them define it from smaller variables (1367 = 982 and 1231 on
    and-15, whose definition the netlist reader had taken backwards).  Tried
    in variable order, so a definition's inputs are settled first."""
    used = set()
    for _sup, _tt, cl in gates.values():
        used |= cl
    occ = defaultdict(list)
    units = set()
    for ci, c in enumerate(clauses):
        if len(c) == 1:
            units.add(abs(c[0]))
        elif ci not in used and len(c) <= MAX_SUPPORT_PLUS_ONE:
            for l in c:
                occ[abs(l)].append(ci)
    steps.take(len(clauses))
    for v in sorted(occ):
        if v in gates or v in units:
            continue
        ids = [ci for ci in occ[v] if ci not in used]
        steps.take(len(ids))
        starts = 0
        tried = set()
        for start in sorted(ids, key=lambda ci: len(clauses[ci])):
            support = frozenset(abs(l) for l in clauses[start] if abs(l) != v)
            if not support or support in tried or any(u >= v or u in units for u in support):
                continue
            tried.add(support)
            starts += 1
            if starts > MAX_REFILL_STARTS:
                break
            sel = [ci for ci in ids if all(abs(l) == v or abs(l) in support for l in clauses[ci])]
            steps.take(len(sel) << len(support))
            tt = nc._definition_tt(v, sel, clauses, tuple(sorted(support)))
            if tt is not None:
                cl = frozenset(sel)
                gates[v] = (tuple(sorted(support)), tt, cl)
                used |= cl
                break


def read_gates(clauses, n_vars, steps):
    """defs[v] = (kind, support tuple, table or None) with kind 'full' (a
    definition), 'wide' (an AND/OR of more than six inputs, no table) or
    'half' (a one-way half; the table is the function v implies, or the
    negation of the one that implies v).  Also the reading's name, the counts
    per kind and the variables present.

    The full definitions come first and are kept as read (they form no
    cycle).  Wide gates are read for variables no definition names.  A
    one-way half is tried only for a variable that occurs with both signs,
    that no definition or wide gate names and that no definition or wide
    gate reads: so a full gate never depends on a half, and the halves are
    settled among themselves, forward from the inputs (a variable that occurs
    with one sign only, like a Plaisted-Greenbaum input, is an input)."""
    # the first-literal reader cannot stop midway: reserve its linear cost
    # before it starts; the netlist reader charges the budget as it goes
    steps.take(sum(len(c) for c in clauses))
    fl = fc.first_literal_gates(clauses)
    if fl is not None:
        full = {v: (sup, tt) for v, (sup, tt, _cl) in fl.items()}
        reading = "first-literal"
    else:
        # the netlist reader skips variables with more than MAX_OCC
        # occurrences as gate outputs, for speed; a select read by hundreds of
        # switches is exactly such a variable, and read as an input it would
        # cut the support walk short (its own cone would never be followed)
        saved = nc.MAX_OCC
        nc.MAX_OCC = 1 << 30
        try:
            g, _used, _dead, topo = nc.recover_gates(clauses, n_vars, poll=steps.take)
        finally:
            nc.MAX_OCC = saved
        if topo:
            steps.take(len(g))
            g = orient_forward(g)
            refill_forward(g, clauses, steps)
        full = {v: (sup, tt) for v, (sup, tt, _cl) in g.items()}
        reading = "definitions"
    # a table that ignores some of its inputs keeps only the ones it reads,
    # or harmless extra inputs would count as control in the support walk
    full = {v: shrink(sup, tt) for v, (sup, tt) in full.items()}
    steps.take(len(full))
    defs = {v: ("full", tuple(sup), tt) for v, (sup, tt) in full.items()}
    occ = defaultdict(list)
    units = set()
    pairs = set()
    pos, neg = set(), set()
    for ci, c in enumerate(clauses):
        for l in c:
            occ[abs(l)].append(ci)
            (pos if l > 0 else neg).add(abs(l))
        if len(c) == 1:
            units.add(abs(c[0]))
        elif len(c) == 2:
            a, b = c
            pairs.add((a, b) if a < b else (b, a))
    steps.take(len(clauses))

    def has_pair(a, b):
        return ((a, b) if a < b else (b, a)) in pairs

    # wide gates: a clause (l, m1, ..., mk), k > 6, with every binary (-l, -mi)
    # makes l the AND of the -mi; an exactly-one group (several literals
    # qualify) is not a gate
    for c in clauses:
        if len(c) <= MAX_SUPPORT_PLUS_ONE:
            continue
        steps.take(len(c))
        hits = []
        for l in c:
            if abs(l) in defs:
                continue
            if all(has_pair(-l, -m) for m in c if m != l):
                hits.append(l)
                if len(hits) > 1:
                    break
        if len(hits) == 1 and abs(hits[0]) not in defs:
            l = hits[0]
            defs[abs(l)] = ("wide", tuple(sorted(abs(m) for m in c if m != l)), None)
    read_by = set()
    for _k, sup, _t in defs.values():
        read_by.update(sup)
    # one-way halves, only in a file that two-way definitions do not explain
    # (in a Tseitin file the variables they leave are inputs, and their
    # clauses would read as bogus halves)
    cand = defaultdict(list)
    loose = len(present_vars := set(occ)) - len(units)
    if len(full) < 0.5 * max(1, loose):
        for v, ids in occ.items():
            if v in defs or v in units or v in read_by or v not in pos or v not in neg:
                continue
            steps.take(len(ids))
            for sign in (-1, 1):
                sel = [ci for ci in ids if 2 <= len(clauses[ci]) <= MAX_SUPPORT_PLUS_ONE
                       and (v * sign) in clauses[ci]]
                if not sel or len(sel) > MAX_HALF_CLAUSES:
                    continue
                xs = tuple(sorted({abs(l) for ci in sel for l in clauses[ci] if abs(l) != v}))
                if not xs or len(xs) > MAX_LEAVES or any(x in units for x in xs):
                    continue
                t = table_of_clauses([[l for l in clauses[ci] if abs(l) != v] for ci in sel], xs)
                if t in (0, FULL[len(xs)]):
                    continue
                if sign > 0:
                    # (v or R_i) for all i: not v implies every R_i, so v is
                    # implied by NOT(AND R_i); the table is that complement
                    t = FULL[len(xs)] ^ t
                cand[v].append((xs, t, frozenset(sel)))
    # a file numbered in circuit order: a gate's half reads smaller variables,
    # and the halves that read bigger ones (an input's uses) are dropped
    if cand:
        down = sum(1 for v, lst in cand.items() if any(all(x < v for x in d[0]) for d in lst))
        if down >= 0.8 * len(cand):
            for v in list(cand):
                cand[v] = [d for d in cand[v] if all(x < v for x in d[0])]
                if not cand[v]:
                    del cand[v]
    unsettled = set(cand)
    waiting = defaultdict(list)
    remaining = []
    flat = []
    ready = []
    owned = set()
    for v, lst in cand.items():
        for xs, t, cl in lst:
            cid = len(flat)
            flat.append((v, xs, t, cl))
            need = [u for u in xs if u in unsettled]
            remaining.append(len(need))
            for u in need:
                waiting[u].append(cid)
            if not need:
                ready.append(cid)
    steps.take(len(flat))

    def settle(u):
        unsettled.discard(u)
        for w in waiting.pop(u, ()):
            remaining[w] -= 1
            if remaining[w] == 0:
                ready.append(w)

    while unsettled:
        while ready:
            cid = ready.pop()
            v, xs, t, cl = flat[cid]
            if v not in unsettled or cl & owned:
                continue            # settled already, or its clauses belong to another half
            defs[v] = ("half", xs, t)
            owned.update(cl)
            settle(v)
            steps.take(1)
        if unsettled:
            # a deadlock: the most waited-on variables become inputs, a
            # hundredth of the rest at a time (at least one)
            k = max(1, len(unsettled) // 100)
            steps.take(len(unsettled))
            for u in heapq.nlargest(k, unsettled, key=lambda u: (len(waiting.get(u, ())), -u)):
                settle(u)
    kinds = Counter(d[0] for d in defs.values())
    return defs, reading, kinds, present_vars


MAX_SUPPORT_PLUS_ONE = nc.MAX_SUPPORT + 1


# ---------------------------------------------------------------------------
# switches, by cuts of up to six leaves
# ---------------------------------------------------------------------------


_KIND = {}


def switch_kind_cached(t, n):
    key = (n, t)
    r = _KIND.get(key)
    if r is None:
        r = switch_kind(t, n)
        _KIND[key] = r
    return r


MAX_FRONTIER = 10        # leaves a cut may have on the way to a smaller one
MAX_DEPTH = 3            # levels of fan-in gates folded in
MAX_STATES = 24          # cuts kept per level


def _table_gate(u, defs):
    d = defs.get(u)
    return d is not None and d[2] is not None


def reconverges(v, defs, depth=MAX_DEPTH, limit=48):
    """Does some signal reach v by two paths within `depth` levels?  A switch
    reads its select twice, so a gate whose cone is a tree is no switch."""
    seen = set()
    frontier = list(defs[v][1])
    for _ in range(depth):
        nxt = []
        for x in frontier:
            if x in seen:
                return True
            seen.add(x)
            if len(seen) > limit:
                return True          # too bushy to tell cheaply: look
            if _table_gate(x, defs):
                nxt.extend(defs[x][1])
        frontier = nxt
    return any(x in seen for x in frontier) or len(set(frontier)) < len(frontier)


def dependent_leaf(leaves, defs, depth=2):
    """Is some leaf a function of the other leaves (within `depth` levels)?
    Such a cut's table counts patterns the circuit cannot produce, and a
    switch read from it can be false (a multiplier's carry cells)."""
    ls = set(leaves)

    def dep(x, d):
        e = defs.get(x)
        if e is None or e[2] is None:
            return False
        return all((y in ls and y != x) or (d > 1 and dep(y, d - 1)) for y in e[1])

    return any(dep(x, depth) for x in leaves)


def cone_table(v, leaves, defs):
    """v's table over the given leaves (sorted), or None when the cone from v
    down to them is not closed (it reaches an input or a wide gate first)."""
    n = len(leaves)
    val = {x: SLOT[n][i] for i, x in enumerate(leaves)}
    stack = [v]
    while stack:
        u = stack[-1]
        if u in val:
            stack.pop()
            continue
        d = defs.get(u)
        if d is None or d[2] is None:
            return None
        missing = [x for x in d[1] if x not in val]
        if missing:
            stack.extend(missing)
            if len(stack) > 256:
                return None
            continue
        stack.pop()
        val[u] = fc.compile_tt(len(d[1]), d[2])(*[val[x] for x in d[1]], FULL[n]) & FULL[n]
    return val[v]


def cuts_of(v, defs, steps):
    """(leaves, table) cuts of v: its own fan-ins, then cuts with fan-in gates
    folded in, level by level (each gate leaf alone, or all of them), up to
    three levels; a cut on the way may have up to ten leaves, a cut that is
    judged has at most six.  Only gates whose cone reconverges are folded."""
    kind, sup, tt = defs[v]
    if tt is None:
        return
    if not dependent_leaf(sup, defs):
        yield tuple(sup), tt
    if not reconverges(v, defs):
        return
    seen = {tuple(sup)}
    level = [tuple(sup)]
    for _depth in range(MAX_DEPTH):
        nxt = []
        for leaves in level:
            inner = [u for u in leaves if _table_gate(u, defs)]
            options = [[u] for u in inner] + ([inner] if len(inner) > 1 else [])
            for which in options:
                new = set(x for x in leaves if x not in which)
                for u in which:
                    new.update(defs[u][1])
                new = tuple(sorted(new))
                if len(new) > MAX_FRONTIER or new in seen:
                    continue
                seen.add(new)
                nxt.append(new)
                if len(new) <= MAX_LEAVES and not dependent_leaf(new, defs):
                    steps.take(1 << len(new))
                    t = cone_table(v, new, defs)
                    if t is not None:
                        yield new, t
                if len(nxt) >= MAX_STATES:
                    break
            if len(nxt) >= MAX_STATES:
                break
        level = nxt
        if not level:
            return


def find_switches(defs, steps):
    """switches[v] = (kind, select variables) for every gate that is a 2:1 or
    4:1 switch over some cut; enables[v] = enable variable for gates whose own
    table is an enable and that are no switch."""
    switches = {}
    enables = {}
    for v in defs:
        if defs[v][2] is None:
            continue
        first = True
        for leaves, table in cuts_of(v, defs, steps):
            steps.take(1)
            kind, sel = switch_kind_cached(table, len(leaves))
            if kind in ("mux", "mux4"):
                switches[v] = (kind, tuple(leaves[i] for i in sel))
                break
            if first and kind == "enable":
                enables[v] = tuple(leaves[i] for i in sel)
            first = False
    for v in switches:
        enables.pop(v, None)
    return switches, enables


# ---------------------------------------------------------------------------
# supports and coverage
# ---------------------------------------------------------------------------


WIDE = object()


def support_of(v, defs, memo, cap, steps):
    """The inputs v depends on through defs (a frozenset), or WIDE when more
    than cap; memoised for every variable visited.  A cycle edge (only
    possible between definitions the settle left apart) reads as a leaf."""
    if v in memo:
        return memo[v]
    state = {}
    stack = [v]
    while stack:
        u = stack[-1]
        if u in memo:
            stack.pop()
            continue
        d = defs.get(u)
        if d is None:
            memo[u] = frozenset((u,))
            stack.pop()
            continue
        if state.get(u) is None:
            state[u] = 1
            for x in d[1]:
                if x not in memo and state.get(x) is None:
                    stack.append(x)
            continue
        stack.pop()
        steps.take(len(d[1]))
        acc = set()
        wide = False
        for x in d[1]:
            s = memo.get(x)
            if s is WIDE:
                wide = True
                break
            if s is None:
                s = frozenset((x,))
            acc |= s
            if len(acc) > cap:
                wide = True
                break
        memo[u] = WIDE if wide else frozenset(acc)
    return memo[v]


def coverage(switches, enables, defs, steps, cap=CAP):
    memo = {}
    weight = Counter()
    sups = []
    wide = 0
    for v, (_kind, sel) in switches.items():
        acc = set()
        bad = False
        for s in sel:
            sp = support_of(s, defs, memo, cap, steps)
            if sp is WIDE:
                bad = True
                break
            acc |= sp
        if bad or len(acc) > cap:
            wide += 1
            continue
        sups.append(frozenset(acc))
        for u in acc:
            weight[u] += 1
    ranked = [u for u, _w in weight.most_common()]
    total = len(switches)
    row = {"muxes": total, "mux2": sum(1 for k, _s in switches.values() if k == "mux"),
           "mux4": sum(1 for k, _s in switches.values() if k == "mux4"),
           "wide_select": wide}
    for k in KS:
        top = set(ranked[:k])
        row[f"cover{k}"] = round(sum(1 for sp in sups if sp <= top) / total, 4) if total else 0.0
    row["top_inputs"] = ",".join(str(u) for u in ranked[:16])
    ew = Counter()
    for _v, cands in enables.items():
        for e in cands:
            sp = support_of(e, defs, memo, cap, steps)
            if sp is not WIDE:
                for u in sp:
                    ew[u] += 1
    row["enables"] = len(enables)
    row["top_enable_inputs"] = ",".join(str(u) for u, _w in ew.most_common(8))
    return row, ranked


def analyse(clauses, n_vars, budget=DEFAULT_BUDGET):
    t0 = time.time()
    steps = Steps(budget)
    row = {"reading": "", "budget": 0}
    try:
        defs, reading, kinds, present = read_gates(clauses, n_vars, steps)
        row.update({"reading": reading, "full_gates": kinds.get("full", 0), "wide_gates": kinds.get("wide", 0),
                    "half_gates": kinds.get("half", 0),
                    "inputs": sum(1 for v in present if v not in defs)})
        switches, enables = find_switches(defs, steps)
        cov, ranked = coverage(switches, enables, defs, steps)
        row.update(cov)
    except Budget:
        row["budget"] = 1
        ranked = []
    row["steps"] = steps.n
    row["seconds"] = round(time.time() - t0, 1)
    return row, ranked


# ---------------------------------------------------------------------------
# control without switches (bead SAT-playground-d96.8.2): guard literals,
# choice groups, coded control words and guess-and-determine sets
# ---------------------------------------------------------------------------


class Prop:
    """Unit propagation with two watched literals and an undo trail, for the
    guess-and-determine search.  Every clause visit is a step."""

    def __init__(self, n_vars, clauses, steps):
        self.steps = steps
        self.val = bytearray(n_vars + 1)          # 0 free, 1 true, 2 false
        self.trail = []
        self.cl = []
        self.watch = defaultdict(list)            # literal -> clauses watching it
        self.conflict = False
        units = []
        steps.take(sum(len(c) for c in clauses))
        for c in clauses:
            if len(c) == 1:
                units.append(c[0])
            elif len(c) >= 2:
                ci = len(self.cl)
                self.cl.append(list(c))
                self.watch[c[0]].append(ci)
                self.watch[c[1]].append(ci)
        # every unit first, then one propagation from the start of the trail
        # (propagating after each unit would rescan the trail every time)
        for l in units:
            if not self.assign(l):
                self.conflict = True
                break
        if not self.conflict and not self.propagate():
            self.conflict = True
        self.root = len(self.trail)

    def value(self, l):
        v = self.val[abs(l)]
        if v == 0:
            return 0
        return 1 if (v == 1) == (l > 0) else -1

    def assign(self, l):
        x = self.value(l)
        if x:
            return x > 0
        self.val[abs(l)] = 1 if l > 0 else 2
        self.trail.append(l)
        return True

    def propagate(self, start=None):
        """Propagate the trail from `start` (default: the root mark or 0);
        False on a conflict."""
        q = start if start is not None else 0
        while q < len(self.trail):
            l = self.trail[q]
            q += 1
            false_lit = -l
            ws = self.watch.get(false_lit)
            if not ws:
                continue
            keep = []
            i = 0
            n = len(ws)
            while i < n:
                ci = ws[i]
                i += 1
                c = self.cl[ci]
                self.steps.take(1)
                if c[0] == false_lit:
                    c[0], c[1] = c[1], c[0]
                if self.value(c[0]) > 0:
                    keep.append(ci)
                    continue
                for k in range(2, len(c)):
                    if self.value(c[k]) >= 0:
                        c[1], c[k] = c[k], c[1]
                        self.watch[c[1]].append(ci)
                        break
                else:
                    keep.append(ci)
                    if self.value(c[0]) < 0:
                        keep.extend(ws[i:])
                        self.watch[false_lit] = keep
                        return False
                    self.assign(c[0])
            self.watch[false_lit] = keep
        return True

    def undo(self, mark):
        while len(self.trail) > mark:
            l = self.trail.pop()
            self.val[abs(l)] = 0


def guard_candidates(clauses, n_vars, k=16):
    """Variables ranked by the clauses their two values satisfy (a selector
    or mode bit satisfies a whole block one way): (variable, satisfied by
    true, satisfied by false), heaviest first; unit-fixed variables left out.
    Also the median of that count, so a guard can be told from a plain
    high-fan-out signal by how far it stands out."""
    pos, neg = Counter(), Counter()
    units = set()
    for c in clauses:
        if len(c) == 1:
            units.add(abs(c[0]))
            continue
        for l in c:
            (pos if l > 0 else neg)[abs(l)] += 1
    tot = [(pos[v] + neg[v], v) for v in set(pos) | set(neg) if v not in units]
    tot.sort(reverse=True)
    counts = sorted(t for t, _v in tot)
    median = counts[len(counts) // 2] if counts else 0
    return [(v, pos[v], neg[v]) for _t, v in tot[:k]], median


MAX_GROUP = 64          # longest clause tried as a pairwise exactly-one group


def group_candidates(clauses, n_vars, k=4, deadline=None, chained=True):
    """Exactly-one choice groups, ranked by the clauses their members touch.
    First the pairwise ones: a clause of 2 to 64 literals whose literals
    exclude each other by binary clauses (one place a step, one slot a task).
    Then, when `chained`, the free-choice census's reading (tools/free_census.py)
    adds groups written through chains (ladder, commander, binary encodings).
    Groups are kept biggest first while they share no variable.  Returns
    [(members, touched clauses, none_ok)]."""
    pairs = set()
    occ = Counter()
    for c in clauses:
        for l in c:
            occ[abs(l)] += 1
        if len(c) == 2:
            a, b = c
            pairs.add((a, b) if a < b else (b, a))

    def excl(a, b):
        x, y = -a, -b
        return ((x, y) if x < y else (y, x)) in pairs
    found = {}
    for c in clauses:
        if not 2 <= len(c) <= MAX_GROUP:
            continue
        if all(excl(c[i], c[j]) for i in range(len(c)) for j in range(i + 1, len(c))):
            key = frozenset(c)
            found.setdefault(key, (list(c), False))
    if chained:
        import free_census as fr
        try:
            cnf = fr.CNF.from_clauses(n_vars, clauses)
            _fixed, rd, _nsat = fr.read_roles(cnf, deadline)
        except TimeoutError as e:
            if str(e) == "cell time limit":
                raise                    # the census's alarm: the cell is over
            rd = None                    # the reading's own deadline
        if rd is not None:
            for g, mem in enumerate(rd.groups):
                if len(mem) >= 2:
                    found.setdefault(frozenset(mem), (list(mem), bool(rd.none_ok[g])))
    ranked = sorted(found.values(), key=lambda x: (-len(x[0]), -sum(occ[abs(l)] for l in x[0])))
    taken = set()
    out = []
    for mem, none_ok in ranked:
        vs = {abs(l) for l in mem}
        if vs & taken:
            continue
        taken |= vs
        out.append((mem, sum(occ[abs(l)] for l in mem), none_ok))
    out.sort(key=lambda x: -x[1])
    return out[:k]


def gd_candidates(clauses, n_vars, steps, k=16, pool=64, samples=4, seed=1):
    """A guess-and-determine set, greedy: at each step the variable (from the
    `pool` most frequent) whose two values, on top of `samples` random values
    of the set chosen so far, fix the most variables by propagation.  Stops
    at k variables or once the set's values fix 99% of the variables (a case
    that ends in a conflict counts as fixed: it is refuted at once).  Returns
    [(variable, share fixed with it, share of the sampled cases that end in a
    conflict)].  Both are estimates from a few sampled cases: whether every
    case falls is the effect test's question (bead d96.8.3)."""
    rng = random.Random(seed)
    pr = Prop(n_vars, clauses, steps)
    if pr.conflict:
        return []
    occ = Counter()
    for c in clauses:
        for l in c:
            occ[abs(l)] += 1
    free0 = [v for v in occ if pr.val[v] == 0]
    if not free0:
        return []
    n_free = len(free0)
    cand = [v for v, _n in occ.most_common() if pr.val[v] == 0][:pool]
    chosen = []
    out = []
    base = pr.root
    for _step in range(k):
        vals = [[rng.random() < 0.5 for _ in chosen] for _ in range(samples)]
        best = None
        for v in cand:
            if v in chosen:
                continue
            fixed = 0
            tries = 0
            conflicts = 0
            for sv in vals:
                for b in (True, False):
                    mark = len(pr.trail)
                    ok = True
                    for x, bx in zip(chosen + [v], sv + [b]):
                        if not pr.assign(x if bx else -x):
                            ok = False
                            break
                    if ok:
                        ok = pr.propagate(mark)
                    # a conflict refutes this case at once: for a split that
                    # is as cheap as a case fixed outright, so it counts as such
                    fixed += (len(pr.trail) - base) if ok else n_free
                    conflicts += not ok
                    tries += 1
                    pr.undo(mark)
            score = fixed / max(1, tries)
            if best is None or score > best[0]:
                best = (score, v, conflicts / max(1, tries))
        if best is None:
            break
        chosen.append(best[1])
        share = min(1.0, best[0] / n_free)
        out.append((best[1], round(share, 4), round(best[2], 4)))
        if share >= 0.99:
            break
    return out


def candidates(clauses, n_vars, budget=DEFAULT_BUDGET, gd_pool=64, deadline=None):
    """Every candidate control set of a formula, by source: 'switch' (the
    inputs that steer the switches), 'enable' (shared enables, a decoded
    word), 'guard' (variables whose values satisfy the most clauses), 'group'
    (choice groups) and 'gd' (a guess-and-determine set).  Returns (row,
    sets) with sets[source] a list of variables (a group: member literals)."""
    t0 = time.time()
    row, ranked = analyse(clauses, n_vars, budget)
    sets = {}
    if ranked:
        # the fewest heaviest inputs that steer half the switches (at most 16)
        sets["switch"] = ranked[:16]
    if row.get("top_enable_inputs"):
        sets["enable"] = [int(x) for x in row["top_enable_inputs"].split(",") if x]
    g, median = guard_candidates(clauses, n_vars)
    if g:
        sets["guard"] = [v for v, _p, _n in g]
        row["guard_top"] = g[0][1] + g[0][2]
        row["guard_median"] = median
    try:
        # the chained reading gets half of what is left, so the census's alarm
        # (the cell's whole limit) never fires inside it
        g_deadline = None if deadline is None else time.time() + 0.5 * max(0.0, deadline - time.time())
        groups = group_candidates(clauses, n_vars, deadline=g_deadline)
    except TimeoutError as e:
        if str(e) == "cell time limit":
            raise
        groups = []
        row["group_error"] = "timeout"
    if groups:
        sets["group"] = groups[0][0]
        row["groups"] = len(groups)
        row["group_top_size"] = len(groups[0][0])
    steps = Steps(budget)
    try:
        gd = gd_candidates(clauses, n_vars, steps, pool=gd_pool)
        row["gd_budget"] = 0
    except Budget:
        gd = []
        row["gd_budget"] = 1
    if gd:
        sets["gd"] = [v for v, _s, _c in gd]
        row["gd_size"] = len(gd)
        row["gd_share"] = gd[-1][1]
        row["gd_conflicts"] = gd[-1][2]
    row["cand_seconds"] = round(time.time() - t0, 1)
    return row, sets


# ---------------------------------------------------------------------------
# toys: one circuit, written every way a switch is written
# ---------------------------------------------------------------------------


class Circ:
    """An abstract circuit: inputs and nodes (kind, inputs) over node ids
    (positive ints); a negative id is a negated signal."""

    def __init__(self):
        self.n = 0
        self.nodes = {}
        self.inputs = []

    def inp(self, k):
        out = []
        for _ in range(k):
            self.n += 1
            self.inputs.append(self.n)
            out.append(self.n)
        return out

    def node(self, kind, *ins):
        self.n += 1
        self.nodes[self.n] = (kind, ins)
        return self.n

    def mux(self, s, a, b):
        return self.node("mux", s, a, b)

    def and_(self, *xs):
        return self.node("and", *xs)

    def or_(self, *xs):
        return self.node("or", *xs)

    def xor(self, a, b):
        return self.node("xor", a, b)


def toy_rotate(width=8, levels=3):
    """A barrel rotator: level k rotates by 2^k when control bit k is on.
    Returns (circuit, control inputs, outputs)."""
    c = Circ()
    w = c.inp(levels)
    r = c.inp(width)
    for k in range(levels):
        sh = 1 << k
        r = [c.mux(w[k], r[(i - sh) % width], r[i]) for i in range(width)]
    return c, w, r


def toy_regread(regs=4, width=8):
    """A register-file read by a binary tree of switches on the address."""
    c = Circ()
    a = c.inp((regs - 1).bit_length())
    data = [c.inp(width) for _ in range(regs)]
    outs = []
    for j in range(width):
        level = [data[i][j] for i in range(regs)]
        for b in a:
            level = [c.mux(b, level[i + 1], level[i]) for i in range(0, len(level), 2)]
        outs.append(level[0])
    return c, a, outs


def toy_decoded(regs=4, width=8):
    """The same read through a decoder: d_i = the address equals i,
    out_j = OR over i of (d_i AND data_i_j)."""
    c = Circ()
    a = c.inp((regs - 1).bit_length())
    data = [c.inp(width) for _ in range(regs)]
    dec = []
    for i in range(regs):
        lits = [b if (i >> k) & 1 else -b for k, b in enumerate(a)]
        dec.append(c.and_(*lits))
    outs = [c.or_(*[c.and_(dec[i], data[i][j]) for i in range(regs)]) for j in range(width)]
    return c, a, outs


def toy_mul(width=4):
    """An array multiplier of two words (ANDs and full adders): no switch."""
    c = Circ()
    x = c.inp(width)
    y = c.inp(width)
    rows = [[c.and_(x[i], y[j]) for i in range(width)] for j in range(width)]
    acc = rows[0]
    outs = [acc[0]]
    acc = acc[1:]
    for j in range(1, width):
        carry = None
        nxt = []
        for i in range(width):
            a = acc[i] if i < len(acc) else None
            b = rows[j][i]
            if a is None and carry is None:
                nxt.append(b)
                continue
            if a is None or carry is None:
                o = a if a is not None else carry
                s = c.xor(o, b)
                carry = c.and_(o, b)
            else:
                t = c.xor(a, b)
                s = c.xor(t, carry)
                carry = c.or_(c.and_(a, b), c.and_(t, carry))
            nxt.append(s)
        if carry is not None:
            nxt.append(carry)
        outs.append(nxt[0])
        acc = nxt[1:]
    outs += acc
    return c, [], outs


TOYS = {"rotate": toy_rotate, "regread": toy_regread, "decoded": toy_decoded, "mul": toy_mul}


# toys for control without switches: (n_vars, clauses, expected)

def _random_block(rng, first, n, m, k=3):
    """m random k-clauses over variables first..first+n-1."""
    return [tuple(v if rng.random() < 0.5 else -v for v in rng.sample(range(first, first + n), k))
            for _ in range(m)]


def toy_two_mode(n=40, m=180, seed=11):
    """A mode bit m0 (variable 1): one block of clauses holds when it is on,
    another when it is off (written (-m0 | c) and (m0 | c)); a few unguarded
    clauses beside.  The guard finder must rank m0 first."""
    rng = random.Random(seed)
    b1 = _random_block(rng, 2, n, m)
    b0 = _random_block(rng, 2, n, m)
    free = _random_block(rng, 2, n, n // 2)
    clauses = [(-1,) + c for c in b1] + [(1,) + c for c in b0] + free
    return n + 1, clauses, {"guard": [1]}


def toy_selectors(n=100, blocks=3, m=120, seed=12):
    """Activation literals a1..a3 (variables 1-3), each switching its own
    block on, as an incremental or BMC dump writes them ((-a | c))."""
    rng = random.Random(seed)
    clauses = []
    for b in range(blocks):
        clauses += [(-(b + 1),) + c for c in _random_block(rng, blocks + 1, n, m)]
    clauses += _random_block(rng, blocks + 1, n, n)
    return n + blocks, clauses, {"guard": list(range(1, blocks + 1))}


def toy_planning(steps=5, places=6, seed=13):
    """A walk on a ring of `places` with chords: one place a step (exactly one,
    pairwise), each step to a neighbour, from place 0 to the far place.  The
    choice groups are the steps; the group finder must find them."""
    rng = random.Random(seed)
    var = lambda t, i: t * places + i + 1  # noqa: E731
    adj = {i: {(i + 1) % places, (i - 1) % places} for i in range(places)}
    for _ in range(places // 2):
        a, b = rng.sample(range(places), 2)
        adj[a].add(b)
        adj[b].add(a)
    clauses = []
    for t in range(steps + 1):
        clauses.append(tuple(var(t, i) for i in range(places)))
        for i in range(places):
            for j in range(i + 1, places):
                clauses.append((-var(t, i), -var(t, j)))
    for t in range(steps):
        for i in range(places):
            clauses.append((-var(t, i),) + tuple(var(t + 1, j) for j in sorted(adj[i])))
    clauses.append((var(0, 0),))
    clauses.append((var(steps, places // 2),))
    groups = [[var(t, i) for i in range(places)] for t in range(steps + 1)]
    return (steps + 1) * places, clauses, {"groups": groups}


def toy_cipher(n=16, rounds=40, seed=14):
    """A filtered LFSR: initial state (the key) s_0..s_15, s_t+16 = s_t xor
    s_t+2 xor s_t+3 xor s_t+5, output z_t = (s_t+1 and s_t+7) xor s_t+11,
    observed for `rounds` steps (units).  Fixing the key fixes everything by
    propagation, so a guess-and-determine set exists; the greedy must reach
    99% within 16 variables."""
    rng = random.Random(seed)
    key = [rng.random() < 0.5 for _ in range(n)]
    nv = [n]

    def new():
        nv[0] += 1
        return nv[0]
    clauses = []

    def xor2(a, b):
        o = new()
        clauses.extend([(-o, a, b), (-o, -a, -b), (o, -a, b), (o, a, -b)])
        return o

    def and2(a, b):
        o = new()
        clauses.extend([(-o, a), (-o, b), (o, -a, -b)])
        return o
    s = list(range(1, n + 1))
    val = {i + 1: key[i] for i in range(n)}

    def ev(x):
        return val[x]
    total = rounds + 16
    while len(s) < total:
        t = len(s) - n
        x = xor2(xor2(s[t], s[t + 2]), xor2(s[t + 3], s[t + 5]))
        val[x] = ev(s[t]) ^ ev(s[t + 2]) ^ ev(s[t + 3]) ^ ev(s[t + 5])
        s.append(x)
    for t in range(rounds):
        a = and2(s[t + 1], s[t + 7])
        z = xor2(a, s[t + 11])
        bit = (ev(s[t + 1]) and ev(s[t + 7])) ^ ev(s[t + 11])
        clauses.append((z if bit else -z,))
    return nv[0], clauses, {"gd_within": 16}


def toy_random(n=200, ratio=3.5, seed=15):
    """Satisfiable random 3-SAT (below the threshold): no guard stands out, no
    group, and no 16 variables fix or refute 99% of the sampled cases."""
    rng = random.Random(seed)
    return n, _random_block(rng, 1, n, int(n * ratio)), {}


CAND_TOYS = {"two-mode": toy_two_mode, "selectors": toy_selectors, "planning": toy_planning,
             "cipher": toy_cipher, "random": toy_random}
STYLES = ("tseitin", "ite6", "aig", "xorform", "canon", "pg", "lut")


def emit(circ, outs, style):
    """(n_vars, clauses, var_of): the circuit written in one style, the
    outputs joined by one clause (at least one is true); var_of maps circuit
    ids to CNF literals.  Styles:
      tseitin  two-way gates: AND/OR (k binaries and one long clause), XOR (4),
               switch (4 ternary clauses)
      ite6     the same with Z3's ite_extra clauses (switch: 6 clauses)
      aig      ANDs of two inputs only: a switch is OR(AND(s,a), AND(-s,b)),
               an XOR three ANDs, a wide AND/OR a chain
      xorform  a switch as b XOR (s AND (a XOR b))
      canon    each gate as its canonical CNF (one clause per input pattern)
      pg       one-way halves (Plaisted-Greenbaum): every signal is used
               positively, so each gate writes only out -> f(ins)
      lut      the rotator's levels in pairs, each output one LUT of up to six
               inputs (canonical CNF), the output literal first (the
               first-literal reading); other toys as canon"""
    nxt = [circ.n]
    var_of = {i: i for i in circ.inputs}
    clauses = []

    def fresh():
        nxt[0] += 1
        return nxt[0]

    def lit(x):
        return var_of[x] if x > 0 else -var_of[-x]

    def g_and(o, xs):                 # o = AND(xs), two-way
        for x in xs:
            clauses.append((-o, x))
        clauses.append((o,) + tuple(-x for x in xs))

    def g_xor(o, a, b):
        clauses.extend([(-o, a, b), (-o, -a, -b), (o, -a, b), (o, a, -b)])

    def g_mux(o, s, a, b, extra=False):
        clauses.extend([(-s, -a, o), (-s, a, -o), (s, -b, o), (s, b, -o)])
        if extra:
            clauses.extend([(-a, -b, o), (a, b, -o)])

    def canon(o, ins, fn):
        k = len(ins)
        for m in range(1 << k):
            vals = [(m >> i) & 1 for i in range(k)]
            out = fn(vals)
            cl = [(-x if vals[i] else x) for i, x in enumerate(ins)]
            clauses.append(((o if out else -o),) + tuple(cl))

    def aig_and(xs):                  # an AND chain of two-input ANDs; returns its literal
        cur = xs[0]
        for x in xs[1:]:
            o = fresh()
            g_and(o, (cur, x))
            cur = o
        return cur

    def fn_of(kind, k):
        if kind == "and":
            return lambda v: int(all(v))
        if kind == "or":
            return lambda v: int(any(v))
        if kind == "xor":
            return lambda v: v[0] ^ v[1]
        return lambda v: v[1] if v[0] else v[2]

    lut_done = set()
    if style == "lut" and circ.nodes and all(k == "mux" for k, _i in circ.nodes.values()):
        # pair up levels: a node whose data inputs are both switches on one
        # select becomes one LUT over its select, theirs and their data
        for v in sorted(circ.nodes):
            kind, (s, a, b) = circ.nodes[v]
            if a in circ.nodes and b in circ.nodes and a not in lut_done and b not in lut_done:
                ka, (sa, aa, ab) = circ.nodes[a]
                kb, (sb, ba, bb) = circ.nodes[b]
                if sa == sb and a not in var_of and b not in var_of:
                    lut_done.add(v)
    for v in sorted(circ.nodes):
        kind, ins = circ.nodes[v]
        if style == "lut" and v in lut_done:
            s, a, b = ins
            _ka, (sa, aa, ab) = circ.nodes[a]
            _kb, (_sb, ba, bb) = circ.nodes[b]
            leaves = []
            for x in (s, sa, aa, ab, ba, bb):
                if x not in leaves:
                    leaves.append(x)
            o = fresh()
            var_of[v] = o
            idx = {x: i for i, x in enumerate(leaves)}

            def f(vals, idx=idx, s=s, sa=sa, aa=aa, ab=ab, ba=ba, bb=bb):
                if vals[idx[s]]:
                    return vals[idx[aa]] if vals[idx[sa]] else vals[idx[ab]]
                return vals[idx[ba]] if vals[idx[sa]] else vals[idx[bb]]
            leaf_lits = [lit(x) for x in leaves]
            canon(o, leaf_lits, f)
            continue
        if style == "lut" and v not in lut_done and any(
                v in (circ.nodes[u][1][1], circ.nodes[u][1][2]) for u in lut_done):
            # absorbed into a LUT; still written if read elsewhere
            readers = [u for u, (_k, i2) in circ.nodes.items() if v in i2]
            if all(u in lut_done and v in circ.nodes[u][1][1:] for u in readers) and v not in outs:
                continue
        ls = [lit(x) for x in ins]
        if style == "aig":
            if kind == "and":
                var_of[v] = aig_and(ls)
            elif kind == "or":
                var_of[v] = -aig_and([-x for x in ls])
            elif kind == "xor":
                a, b = ls
                p = aig_and([a, -b])
                q = aig_and([-a, b])
                var_of[v] = -aig_and([-p, -q])
            else:
                s, a, b = ls
                p = aig_and([s, a])
                q = aig_and([-s, b])
                var_of[v] = -aig_and([-p, -q])
            continue
        o = fresh()
        var_of[v] = o
        if style == "lut":
            canon(o, ls, fn_of(kind, len(ls)))
        elif style in ("tseitin", "ite6") or (style == "xorform" and kind != "mux"):
            if kind == "and":
                g_and(o, ls)
            elif kind == "or":
                g_and(-o, [-x for x in ls])
            elif kind == "xor":
                g_xor(o, *ls)
            else:
                g_mux(o, *ls, extra=(style == "ite6"))
        elif style == "xorform":
            # inner gates numbered before the output, as encoders number them
            s, a, b = ls
            t1, t2 = o, fresh()
            o = fresh()
            var_of[v] = o
            g_xor(t1, a, b)
            g_and(t2, (s, t1))
            g_xor(o, b, t2)
        elif style == "canon":
            canon(o, ls, fn_of(kind, len(ls)))
        elif style == "pg":
            # o -> f(ins), as a Plaisted-Greenbaum encoder writes it for a
            # signal used positively: the minimal clauses
            if kind == "and":
                for x in ls:
                    clauses.append((-o, x))
            elif kind == "or":
                clauses.append((-o,) + tuple(ls))
            elif kind == "xor":
                a, b = ls
                clauses.extend([(-o, a, b), (-o, -a, -b)])
            else:
                s_, a, b = ls
                clauses.extend([(-o, -s_, a), (-o, s_, b)])
        else:
            raise ValueError(style)
    if style == "lut":
        clauses.append((lit(outs[0]),))        # a unit: every clause group is then a gate
    else:
        clauses.append(tuple(lit(x) for x in outs))
    return nxt[0], clauses, var_of


def shuffle(n_vars, clauses, seed):
    """Renumber the variables at random, flip no signs, shuffle clause and
    literal order.  Returns (clauses, perm) with perm[old] = new."""
    rng = random.Random(seed)
    new = list(range(1, n_vars + 1))
    rng.shuffle(new)
    perm = {i + 1: new[i] for i in range(n_vars)}
    out = []
    for c in clauses:
        lits = [perm[abs(l)] * (1 if l > 0 else -1) for l in c]
        rng.shuffle(lits)
        out.append(tuple(lits))
    rng.shuffle(out)
    return out, perm


def write_cnf(path, n_vars, clauses):
    with open(path, "w") as f:
        f.write(f"p cnf {n_vars} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


# ---------------------------------------------------------------------------
# selftest
# ---------------------------------------------------------------------------


def selftest():
    checks = 0
    # the select test on tables
    t_mux = fc._tt_of(lambda s, a, b: a if s else b, 3) & FULL[3]
    assert switch_kind(t_mux, 3) == ("mux", (0,)), switch_kind(t_mux, 3)
    t_and = fc._tt_of(lambda x, y: x & y, 2) & FULL[2]
    assert switch_kind(t_and, 2)[0] == "enable"
    t_xor = fc._tt_of(lambda x, y: x ^ y, 2) & FULL[2]
    assert switch_kind(t_xor, 2) == (None, ())
    t_maj = fc._tt_of(lambda a, b, c: int(a + b + c >= 2), 3) & FULL[3]
    assert switch_kind(t_maj, 3)[0] is None
    t4 = fc._tt_of(lambda s1, s0, a, b, c, d: [[d, c], [b, a]][s1][s0], 6) & FULL[6]
    k4 = switch_kind(t4, 6)
    assert k4 == ("mux4", (0, 1)), k4
    t_xm = fc._tt_of(lambda s, x, a, b: (a if s else b) ^ x, 4) & FULL[4]
    assert switch_kind(t_xm, 4)[0] not in ("mux", "mux4"), switch_kind(t_xm, 4)
    t_em = fc._tt_of(lambda x, s, a, b: x & (a if s else b), 4) & FULL[4]
    assert switch_kind(t_em, 4)[0] not in ("mux", "mux4"), switch_kind(t_em, 4)
    checks += 7
    results = []
    for kind, build in TOYS.items():
        circ, ctl, outs = build()
        for style in STYLES:
            n_vars, clauses, var_of = emit(circ, outs, style)
            for seed in (None, 11):
                cls, perm = (clauses, {i: i for i in range(1, n_vars + 1)}) if seed is None else \
                    shuffle(n_vars, clauses, seed)
                row, ranked = analyse([tuple(c) for c in cls], n_vars)
                want = {perm[var_of[x]] for x in ctl}
                got = set(ranked[:len(want)]) if want else set()
                label = f"{kind}/{style}/{'shuffled' if seed else 'plain'}"
                if kind == "mul":
                    ok = row["muxes"] == 0
                else:
                    ok = got == want and row["cover8"] == 1.0 and row["muxes"] > 0
                    if kind == "decoded" and not ok:
                        # a decoded read written as two-input ANDs only: the
                        # 6-leaf cut lies past three levels of the OR chain,
                        # and the decoder outputs read as shared enables
                        top_en = [int(x) for x in row["top_enable_inputs"].split(",") if x]
                        ok = set(top_en[:len(want)]) == want
                        if ok:
                            label += " (as enables)"
                # known limits: an XOR has no direction, so XOR-form switches
                # need circuit order; one-way gates in a renumbered file are
                # told from their inputs only by the settle's guess
                limit = seed is not None and style in ("xorform", "pg") and kind != "mul"
                results.append((label, ok or limit, row["reading"], row["muxes"], row["cover8"],
                                row["full_gates"], row["half_gates"], row["wide_gates"],
                                limit and not ok))
                checks += 1
    bad = [r for r in results if not r[1]]
    for r in results:
        tag = "LIM " if r[8] else ("ok  " if r[1] else "BAD ")
        print(tag + f"{r[0]:<28} reading={r[2]:<13} switches={r[3]:<4} "
              f"cover8={r[4]:<6} full={r[5]} half={r[6]} wide={r[7]}")
    # random 3-SAT and a pigeonhole: no switch
    rng = random.Random(3)
    r3 = [tuple(v if rng.random() < 0.5 else -v for v in rng.sample(range(1, 61), 3)) for _ in range(260)]
    row, _ = analyse(r3, 60)
    assert row["muxes"] == 0, row
    php = []
    p, h = 5, 4
    var = lambda i, j: i * h + j + 1  # noqa: E731
    for i in range(p):
        php.append(tuple(var(i, j) for j in range(h)))
    for j in range(h):
        for i in range(p):
            for k in range(i + 1, p):
                php.append((-var(i, j), -var(k, j)))
    row, _ = analyse(php, p * h)
    assert row["muxes"] == 0, row
    checks += 2
    # the budget stops the reading and says so, also for a sparse numbering
    circ, ctl, outs = toy_rotate()
    n_vars, clauses, _ = emit(circ, outs, "tseitin")
    row, _ = analyse(clauses, n_vars, budget=50)
    assert row["budget"] == 1
    row, _ = analyse([(1, 2)], 10_000_000, budget=10)
    assert row["budget"] == 1, row
    checks += 2
    # a select through 41 six-input copy gates: only the copied input counts
    cls = []
    nv = [0]

    def newv():
        nv[0] += 1
        return nv[0]
    src = newv()
    cur = src
    for _ in range(41):
        dummies = [newv() for _ in range(5)]
        g = newv()
        ins = [cur] + dummies
        for m in range(64):
            vals = [(m >> i) & 1 for i in range(6)]
            cls.append(((g if vals[0] else -g),) + tuple(-x if vals[i] else x for i, x in enumerate(ins)))
        cur = g
    a_, b_, o_ = newv(), newv(), newv()
    cls += [(-cur, -a_, o_), (-cur, a_, -o_), (cur, -b_, o_), (cur, b_, -o_), (o_,)]
    row, ranked = analyse(cls, nv[0])
    # a copy defines its input as much as its output (like an XOR), so either
    # end of the first copy may read as the input; what matters is that the
    # dummies do not: one input steers the switch
    assert row["wide_select"] == 0 and len(ranked) == 1 and ranked[0] in (src, src + 6), (row, ranked[:3])
    # a shared enable numbered after its data still gets the votes
    cls = []
    for i in range(1, 17):
        o = 17 + i
        cls += [(-o, 17), (-o, i), (o, -17, -i)]
    cls.append(tuple(range(18, 34)))
    row, _ = analyse(cls, 33)
    assert row["top_enable_inputs"].split(",")[0] == "17", row["top_enable_inputs"]
    # the netlist reader's own work counts: 1,000 disconnected equal pairs
    pairs = []
    for k in range(1000):
        x, y = 2 * k + 1, 2 * k + 2
        pairs += [(-x, y), (-y, x)]
    row, _ = analyse(pairs, 2000, budget=30_000)
    assert row["budget"] == 1, row
    checks += 3
    # a one-way gate output written negated everywhere is the same circuit:
    # the reading must not change (its half then has the positive sign)
    for kind in ("decoded", "rotate"):
        circ, ctl, outs = TOYS[kind]()
        n_vars, clauses, var_of = emit(circ, outs, "pg")
        inner = {abs(var_of[v]) for v in circ.nodes}
        flip = {x for i, x in enumerate(sorted(inner)) if i % 2 == 0}
        neg = [tuple(-l if abs(l) in flip else l for l in c) for c in clauses]
        row0, ranked0 = analyse(clauses, n_vars)
        row1, ranked1 = analyse(neg, n_vars)
        want = {var_of[x] for x in ctl}
        assert set(ranked0[:len(want)]) == want and set(ranked1[:len(want)]) == want, (kind, row0, row1)
        assert row0["muxes"] == row1["muxes"], (kind, row0["muxes"], row1["muxes"])
        checks += 1
    # control without switches (d96.8.2)
    for name, build in CAND_TOYS.items():
        nv_, cls_, exp = build()
        g, median = guard_candidates(cls_, nv_)
        top = [v for v, _p, _n in g]
        steps_ = Steps(DEFAULT_BUDGET)
        gd = gd_candidates(cls_, nv_, steps_)
        groups = group_candidates(cls_, nv_, k=16)
        line = (f"{name}: guards {top[:4]} (top {g[0][1] + g[0][2] if g else 0}, median {median}); "
                f"groups {[len(m) for m, _t, _n in groups][:8]}; gd {len(gd)} vars, share {gd[-1][1] if gd else 0}, "
                f"conflicts {gd[-1][2] if gd else 0}")
        print("    " + line)
        if "guard" in exp:
            want = set(exp["guard"])
            assert set(top[:len(want)]) == want, line
            assert g[0][1] + g[0][2] >= 3 * max(1, median), line
            checks += 1
        if "groups" in exp:
            found = {frozenset(abs(l) for l in m) for m, _t, _n in group_candidates(cls_, nv_, k=64)}
            want = {frozenset(x) for x in exp["groups"]}
            assert want <= found, (line, len(want & found), len(want))
            checks += 1
        if "gd_within" in exp:
            assert gd and gd[-1][1] >= 0.99 and len(gd) <= exp["gd_within"], line
            checks += 1
        if name == "random":
            assert not g or g[0][1] + g[0][2] < 3 * max(1, median), line
            assert not groups, line
            # no hit: a hit is 99% fixed (or refuted) within 16 variables
            assert not gd or gd[-1][1] < 0.99, line
            checks += 1
    if bad:
        sys.exit(f"selftest: {len(bad)} of {len(results)} toy readings wrong")
    print(f"selftest: {checks} checks ok")


# ---------------------------------------------------------------------------
# commands
# ---------------------------------------------------------------------------


SUITE = Path(__file__).resolve().parents[1] / "benchmarks" / "sat-comp-2025"
FIELDS = ["stem", "vars", "clauses", "truncated", "reading", "full_gates", "wide_gates", "half_gates",
          "inputs", "muxes", "mux2", "mux4", "wide_select"] + [f"cover{k}" for k in KS] + [
          "top_inputs", "enables", "top_enable_inputs", "budget", "steps", "seconds", "error"]


def _alarm(_sig, _frm):
    raise TimeoutError("cell time limit")


def census_one(job):
    stem, path, max_clauses, seconds, mem_mb, budget = job
    row = {f: "" for f in FIELDS}
    row["stem"] = stem
    t0 = time.time()
    try:
        if mem_mb:
            lim = mem_mb * 1024 * 1024
            resource.setrlimit(resource.RLIMIT_AS, (lim, lim))
        signal.signal(signal.SIGALRM, _alarm)
        signal.alarm(seconds)
        n_vars, n_cls, clauses, truncated = parse_prefix(Path(path), max_clauses)
        row.update({"vars": n_vars, "clauses": n_cls, "truncated": int(truncated)})
        res, _ranked = analyse(clauses, n_vars, budget)
        row.update({k: v for k, v in res.items() if k in row})
    except TimeoutError:
        row["error"] = "timeout"
    except MemoryError:
        row["error"] = "memory"
    except Exception as e:  # noqa: BLE001 - a census row records any failure
        row["error"] = f"{type(e).__name__}: {e}"[:200]
    finally:
        signal.alarm(0)     # never leave the alarm armed for this worker's next cell
    row["seconds"] = round(time.time() - t0, 1)
    return row


CAND_FIELDS = ["stem", "vars", "clauses", "truncated", "reading", "muxes", "wide_select", "cover8", "cover16",
               "cover32", "enables", "guard_top", "guard_median", "groups", "group_top_size", "gd_size",
               "gd_share", "gd_conflicts", "budget", "gd_budget", "group_error"] + \
    [f"set_{k}" for k in ("switch", "enable", "guard", "group", "gd")] + ["seconds", "error"]


def cands_one(job):
    stem, path, max_clauses, seconds, mem_mb, budget = job
    row = {f: "" for f in CAND_FIELDS}
    row["stem"] = stem
    t0 = time.time()
    try:
        if mem_mb:
            lim = mem_mb * 1024 * 1024
            resource.setrlimit(resource.RLIMIT_AS, (lim, lim))
        signal.signal(signal.SIGALRM, _alarm)
        signal.alarm(seconds)
        n_vars, n_cls, clauses, truncated = parse_prefix(Path(path), max_clauses)
        row.update({"vars": n_vars, "clauses": n_cls, "truncated": int(truncated)})
        res, sets = candidates(clauses, n_vars, budget, deadline=t0 + seconds)
        row.update({k: v for k, v in res.items() if k in row})
        for k, vs in sets.items():
            row[f"set_{k}"] = ",".join(str(x) for x in vs)
    except TimeoutError:
        row["error"] = "timeout"
    except MemoryError:
        row["error"] = "memory"
    except Exception as e:  # noqa: BLE001 - a census row records any failure
        row["error"] = f"{type(e).__name__}: {e}"[:200]
    finally:
        signal.alarm(0)
    row["seconds"] = round(time.time() - t0, 1)
    return row


def cmd_census(a):
    files = sorted(Path(a.suite).glob("*.cnf.xz")) if a.suite else sorted(SUITE.glob("*.cnf.xz"))
    if a.stems:
        keys = a.stems.split(",")
        files = [f for f in files if any(k in f.name for k in keys)]
    jobs = [(f.name[: -len(".cnf.xz")], str(f), a.max_clauses, a.seconds, a.mem_mb, a.budget) for f in files]
    jobs.sort(key=lambda j: -os.path.getsize(j[1]))       # big files first
    out = Path(a.out)
    fields, one = (CAND_FIELDS, cands_one) if a.cands else (FIELDS, census_one)
    done = set()
    if out.exists():
        with open(out) as f:
            rd = csv.DictReader(f, delimiter="\t")
            if rd.fieldnames != fields:
                sys.exit(f"{out} holds a table of other columns (written {'without' if a.cands else 'with'} "
                         "--cands?): use a fresh --out")
            done = {r["stem"] for r in rd}
    jobs = [j for j in jobs if j[0] not in done]
    new = not out.exists()
    with open(out, "a", newline="") as fh:
        w = csv.DictWriter(fh, fields, delimiter="\t")
        if new:
            w.writeheader()
        with ProcessPoolExecutor(a.workers) as ex:
            futs = [ex.submit(one, j) for j in jobs]
            for fu in as_completed(futs):
                r = fu.result()
                w.writerow(r)
                fh.flush()
                print(f"{r['stem'][:60]:60s} switches {r['muxes']!s:>8} cover16 {r['cover16']!s:>6} "
                      f"{r['reading']} {r['seconds']}s {r['error']}", flush=True)


def cmd_cell(a):
    if a.seconds:
        signal.signal(signal.SIGALRM, _alarm)
        signal.alarm(a.seconds)
    n_vars, _n_cls, clauses, truncated = parse_prefix(Path(a.file), a.max_clauses)
    row, ranked = analyse(clauses, n_vars, a.budget)
    row["truncated"] = int(truncated)
    for k, v in row.items():
        print(f"{k}\t{v}")


def cmd_cands(a):
    if a.seconds:
        signal.signal(signal.SIGALRM, _alarm)
        signal.alarm(a.seconds)
    n_vars, _n_cls, clauses, truncated = parse_prefix(Path(a.file), a.max_clauses)
    row, sets = candidates(clauses, n_vars, a.budget, gd_pool=a.gd_pool)
    row["truncated"] = int(truncated)
    for k, v in row.items():
        print(f"{k}\t{v}")
    for src, vs in sets.items():
        print(f"set\t{src}\t{','.join(str(x) for x in vs)}")


def cmd_toy(a):
    circ, ctl, outs = TOYS[a.kind]()
    n_vars, clauses, var_of = emit(circ, outs, a.style)
    perm = {i: i for i in range(1, n_vars + 1)}
    if a.shuffle is not None:
        clauses, perm = shuffle(n_vars, clauses, a.shuffle)
    write_cnf(a.out, n_vars, clauses)
    print("control inputs:", ",".join(str(perm[var_of[x]]) for x in ctl))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("cell")
    c.add_argument("file")
    c.add_argument("--max-clauses", type=int, default=100_000_000)
    c.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    c.add_argument("--seconds", type=int, default=0, help="wall limit (0: none)")
    q = sub.add_parser("cands", help="every source's candidate control set (bead d96.8.2)")
    q.add_argument("file")
    q.add_argument("--max-clauses", type=int, default=3_000_000)
    q.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    q.add_argument("--gd-pool", type=int, default=64)
    q.add_argument("--seconds", type=int, default=0, help="wall limit (0: none)")
    t = sub.add_parser("toy")
    t.add_argument("kind", choices=sorted(TOYS))
    t.add_argument("style", choices=STYLES)
    t.add_argument("--out", required=True)
    t.add_argument("--shuffle", type=int)
    e = sub.add_parser("census")
    e.add_argument("--out", required=True)
    e.add_argument("--stems")
    e.add_argument("--workers", type=int, default=6)
    e.add_argument("--max-clauses", type=int, default=5_000_000)
    e.add_argument("--seconds", type=int, default=1800)
    e.add_argument("--mem-mb", type=int, default=40000)
    e.add_argument("--budget", type=int, default=DEFAULT_BUDGET)
    e.add_argument("--cands", action="store_true", help="every source's candidate set (d96.8.2), not only switches")
    e.add_argument("--suite", help="a folder of .cnf.xz files (default: the 2025 suite)")
    sub.add_parser("selftest")
    a = ap.parse_args(argv)
    if a.cmd == "cell":
        cmd_cell(a)
    elif a.cmd == "cands":
        cmd_cands(a)
    elif a.cmd == "census":
        cmd_census(a)
    elif a.cmd == "toy":
        cmd_toy(a)
    else:
        selftest()


if __name__ == "__main__":
    main()
