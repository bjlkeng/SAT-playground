#!/usr/bin/env python3
"""Rows, columns and alldifferent grids in the 2025 cells (bead
SAT-playground-iin.1, G.1.1 of the general-structure epic).

What is looked for, in plain terms. A *row* is a clause of three or more
literals: at least one of them is true. Two rows *differ* when at least two
literals of each exclude literals of the other and the exclusions are sparse
(at most three per literal on average): each row is a finite-domain variable,
an excluded pair is the same value, and the two variables may not take it
together. A *grid* is a set of rows around one row, each differing from that
one and sharing an exclusion with at least four in five of the others (all of
them below five rows), and a set of columns, where a column is a set of
literals that exclude each other pairwise and holds at most one literal of
each row. n rows over d columns is an alldifferent,
whatever other exclusions the rows have besides (n-queens has diagonals next
to its columns). The rows are fixed first, then the columns are read as a
clique cover of their literals, the row with the fewest choices next and, on
a tie, the literal that keeps the most choices open. CDCL propagates these
encodings but cannot learn the Hall argument on them (k rows whose values fit
in k-1 columns) in a short proof; that is what an in-search propagator would
add, so this census says where such structure sits and how tight it is.

Nothing here depends on one encoder. Exclusion is read by unit propagation:
l excludes m when propagating l alone derives ~m, or the other way round, so
pairwise clauses, ladders, commanders and gate-based encodings all look the
same. A row does not have to be exact-one (plain pigeonhole and graph
colouring have no at-most-one inside a row). Signs do not matter: a row may
hold literals of both signs, so a formula with flipped polarities reads the
same. A literal g of a row whose negation sits in exactly one clause C of
three or more literals is also resolved away, which gives the row over C's
literals as well (a split at-least-one reads as one row over its leaves);
that row is a resolvent, so the formula implies it.

A clause is taken as a finite-domain variable only when its own literals
exclude each other all or next to not at all: exact-one, plain at-least-one
(no exclusion inside, or in five or more literals none excluding more than a
quarter of the others), or exact-one over all but one literal (a guard). A
gate's clause fails this: in (o v ~a v ~b) the literal o excludes ~a and ~b
while ~a and ~b do not exclude each other. That is a test on what the clause
does, not on how it looks.

What a cell reports. Rows are grouped by sign pattern, length and whether
they are resolvents, and read round robin within the budget (`--budget`,
propagation visits), so that a small class is read in full and a big one
sampled. A row needs two literals with a binary implication to take part at
all, since propagating a literal with none derives nothing. The columns say
how many rows were read, how many are exact-one, how many are domains (the
test above) and how many of those differ from another row, with `est_`
estimates scaled by class, and the grids found around them: `nWdC` is n rows
over d shared columns, `+pP` adds p values only one row has. A grid is *core*
when it has three or more rows, three or more shared columns and no more
private values than shared columns; tightness is n over all its values, read
on core grids only. A core grid of eight or more rows is *big*: three-row
grids turn up inside circuits by accident, and the report counts big ones
apart and lists them first. Matching-based filtering (Regin) runs on the grids of
three or more rows at the root, core grids first, and counts literals it
removes and Hall violations; a value one row has alone counts as a column of
its own, which can only undercount. At-most-one cliques read directly from
the binary clauses are counted too, with how many have no at-least-one over
them. A cell over the structure stage's own clause limit (MAX_READ) is read
as a prefix and marked: no pass runs on it, whatever it holds.

The `trail` command answers what root filtering cannot: given the
assignments a real search visits (one line of signed literals per snapshot),
how often does Hall reasoning find a contradiction or a removal that unit
propagation has not found? tools/hall_probe.patch is the scratch change
that writes those lines, and that can add a violation's clause to the search
at restarts from the grids the `grids` command writes; it is not part of the
solver and writes no proof.

Usage:
  python3 tools/alldiff_census.py census --out TSV [--workers 16] [--budget 150000000] [--files ...]
  python3 tools/alldiff_census.py one FILE [--budget N]
  python3 tools/alldiff_census.py report TSV [--runs DIR]
  python3 tools/alldiff_census.py trail FILE SNAPSHOTS [--budget N] [--snapshots-limit N] [--grids N] [--check N]
  python3 tools/alldiff_census.py grids FILE --out GRIDS [--budget N] [--min-rows N]
  python3 tools/alldiff_census.py selftest [--tmp DIR]
"""
from __future__ import annotations

import argparse
import csv
import itertools
import random
import subprocess
import sys
import time
from collections import Counter, defaultdict, deque
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import proof_gate  # noqa: E402

SUITE = ROOT / "benchmarks" / "sat-comp-2025"
CELLS = ROOT / "benchmarks" / "rl" / "structure_2025.tsv"
RUN_OF_RECORD = ROOT / "log" / "abtest-struct-dd-400-2026-10-31-02-15-28"

MAX_READ = 10_000_000    # the structure stage's clause limit (option structclauses): no pass runs past it
PREFIX = 3_000_000       # what is read of a cell past that limit
BUDGET = 150_000_000     # propagation visits per cell
BIG = 8                  # a core grid of this many rows or more is a big one: a real alldifferent, not a gate's accident
HUB = 50_000             # a literal that forces more than this is a hub: its exclusions are not read
MIN_ROW = 3              # a two-literal row is one not-equal, not a domain
MAX_ROW = 5_000          # a longer resolved row is not built
ROW_STEPS = 5_000_000    # literals visited building resolved rows, per cell
MAX_CLIQUES = 6          # cliques grown per row read
ROW_SHARE = 50           # at least this part of the budget is one row's to spend: 1/50
CLASSIFY_SHARE = 0.4     # the part of the budget classifying rows may use
MAX_FAILS = 64           # rows in a row that may not fit before a clique stops trying, strongest first
MAX_ENTRIES = 30_000_000 # exclusion pairs cached per cell; past this the cache starts over
HALL_MAX_CELLS = 40_000  # n x d of a grid on which root filtering runs
HALL_MAX_GRIDS = 200     # grids per cell on which root filtering runs
AMO_BUDGET = 20_000_000  # steps of the greedy clique cover over the binary clauses

sys.setrecursionlimit(1_000_000)

COLUMNS = ["stem", "family", "vars", "clauses", "total_clauses", "truncated", "over_limit", "root_units",
           "root_conflict", "rows", "rows_active", "rows_read", "rows_cut", "rows_exact", "est_exact", "exact_sizes",
           "rows_domain", "est_domain", "rows_differ", "est_differ", "grids3", "grids_core", "rows_in_core",
           "max_n", "max_tight", "grids_big", "big_rows", "big_tight", "big_n", "grids",
           "hall_grids", "hall_removed", "hall_violations", "amo_cliques", "amo_max", "amo_only",
           "failed_lits", "hubs", "visits", "stopped", "read_s", "detect_s", "error"]


# ------------------------------------------------------------------ reading

def header_counts(path):
    """(variables, clauses) from the `p cnf` line, (0, 0) when there is none
    before the first clause."""
    with proof_gate.open_cnf(path) as fh:
        for raw in fh:
            if raw.startswith(b"p"):
                parts = raw.split()
                if len(parts) >= 4 and parts[1] == b"cnf":
                    return int(parts[2]), int(parts[3])
                return 0, 0
            if not raw.startswith(b"c") and raw.strip():
                return 0, 0
    return 0, 0


def clean(lits):
    """The clause with repeated literals removed, None for a tautology."""
    seen = set(lits)
    if any(-l in seen for l in seen):
        return None
    if len(seen) == len(lits):
        return lits
    out = []
    for l in lits:
        if l in seen:
            out.append(l)
            seen.discard(l)
    return out


def read_clauses(path, limit):
    """Up to `limit` clauses as the solver reads them (split at zeros, plain
    or compressed). A token the fast path cannot read sends the whole file
    to the gate's slower reader. Returns (variables seen, clauses, truncated)."""
    clauses = []
    cur = []
    nv = 0
    truncated = False
    try:
        with proof_gate.open_cnf(path) as fh:
            for raw in fh:
                head = raw[:1]
                if head == b"c" or head == b"p":
                    continue
                for tok in raw.split():
                    v = int(tok)
                    if v:
                        cur.append(v)
                        if v > nv:
                            nv = v
                        elif -v > nv:
                            nv = -v
                    else:
                        if len(clauses) >= limit:
                            truncated = True
                            break
                        c = clean(cur)
                        if c is not None:
                            clauses.append(c)
                        cur = []
                if truncated:
                    break
        return nv, clauses, truncated
    except ValueError:
        pass
    clauses = []
    nv = 0
    truncated = False
    gen = proof_gate.iter_clauses(path)
    while True:
        try:
            lits = next(gen)
        except StopIteration as stop:
            nv = max(nv, stop.value or 0)
            break
        if len(clauses) >= limit:
            truncated = True
            break
        for l in lits:
            nv = max(nv, abs(l))
        c = clean(list(lits))
        if c is not None:
            clauses.append(c)
    return nv, clauses, truncated


# ---------------------------------------------------------------- the formula

class Formula:
    """The clauses after the root units are propagated: `fixed` holds the
    literals true at the root, `live` the remaining clauses of two or more
    literals with their false literals removed, `occ[l]` the indexes of the
    live clauses holding l, and `has_binary` the literals l with a clause
    (~l v m), the only literals that propagate anything alone."""

    def __init__(self, clauses):
        self.fixed = set()
        self.conflict = False
        self.live = []
        self.occ = defaultdict(list)
        self.has_binary = set()
        if any(not c for c in clauses):
            self.conflict = True    # an empty clause in the input
            return
        occ0 = defaultdict(list)
        for i, c in enumerate(clauses):
            for l in c:
                occ0[l].append(i)
        nfalse = [0] * len(clauses)
        sat = [False] * len(clauses)
        queue = deque(c[0] for c in clauses if len(c) == 1)
        while queue and not self.conflict:
            l = queue.popleft()
            if l in self.fixed:
                continue
            if -l in self.fixed:
                self.conflict = True
                break
            self.fixed.add(l)
            for i in occ0.get(l, ()):
                sat[i] = True
            for i in occ0.get(-l, ()):
                if sat[i]:
                    continue
                nfalse[i] += 1
                left = len(clauses[i]) - nfalse[i]
                if left == 1:
                    for m in clauses[i]:
                        if -m not in self.fixed:
                            queue.append(m)
                            break
                elif left == 0:
                    self.conflict = True
                    break
        if self.conflict:
            return
        del occ0
        for i, c in enumerate(clauses):
            if sat[i]:
                continue
            rest = [l for l in c if -l not in self.fixed]
            if len(rest) >= 2:
                self.live.append(rest)
        for i, c in enumerate(self.live):
            for l in c:
                self.occ[l].append(i)
            if len(c) == 2:
                self.has_binary.add(-c[0])
                self.has_binary.add(-c[1])


FAILED = "failed"
TOO_BIG = "hub"


class Propagator:
    """Unit propagation from one literal over the live clauses, charged to a
    budget in clause visits."""

    def __init__(self, f: Formula, budget: int):
        self.f = f
        self.budget = budget
        self.used = 0

    def spent(self):
        return self.used >= self.budget

    def reach(self, l):
        """The literals l forces (l included), FAILED when l alone reaches a
        conflict (so l is false at the root), TOO_BIG past the hub limit."""
        trail = [l]
        val = {l: True, -l: False}
        nfalse = {}
        occ = self.f.occ
        live = self.f.live
        i = 0
        while i < len(trail):
            y = trail[i]
            i += 1
            touched = occ.get(-y, ())
            self.used += 1 + len(touched)
            for ci in touched:
                k = nfalse.get(ci, 0) + 1
                nfalse[ci] = k
                c = live[ci]
                if k < len(c) - 1:
                    continue
                unit = None
                sat = False
                for m in c:
                    v = val.get(m)
                    if v is True:
                        sat = True
                        break
                    if v is None:
                        unit = m
                if sat:
                    continue
                if unit is None:
                    return FAILED
                val[unit] = True
                val[-unit] = False
                trail.append(unit)
            if len(trail) > HUB:
                return TOO_BIG
        return trail


class Exclusions:
    """The literals of the rows each literal's propagation excludes, read
    from that literal's side and cached, and `incoming`, the literals read so
    far that exclude a given one: an exclusion may show from one side only.
    Both start over past MAX_ENTRIES, and a read after that is charged again."""

    EMPTY = frozenset()

    def __init__(self, prop: Propagator, universe):
        self.prop = prop
        self.universe = universe
        self.cache = {}
        self.incoming = defaultdict(set)
        self.entries = 0
        self.failed = set()
        self.hubs = set()

    def of(self, l):
        """The literals l excludes, None when the budget stops the read."""
        got = self.cache.get(l)
        if got is not None:
            return got
        if self.prop.spent():
            return None
        r = self.prop.reach(l)
        if r is FAILED:
            self.failed.add(l)
            got = self.EMPTY
        elif r is TOO_BIG:
            self.hubs.add(l)
            got = self.EMPTY
        else:
            universe = self.universe
            got = frozenset(-y for y in r if -y in universe and y != -l)
            self.prop.used += len(r)
        if self.entries > MAX_ENTRIES:
            self.cache = {}
            self.incoming = defaultdict(set)
            self.entries = 0
        self.cache[l] = got
        self.entries += 2 * len(got) + 1
        for m in got:
            self.incoming[m].add(l)
        return got


# ------------------------------------------------------------------- the rows

def build_rows(f: Formula):
    """Every live clause of MIN_ROW or more literals, once per literal set,
    and the resolvents that take literals g away when ~g sits in exactly one
    such clause, for clauses where that holds of at least half the literals
    (depth three, within ROW_STEPS). Returns (sorted tuples, the set of
    resolvents among them)."""
    long_with = defaultdict(list)
    for c in f.live:
        if len(c) >= MIN_ROW:
            for l in c:
                long_with[l].append(c)
    single = {l: cs[0] for l, cs in long_with.items() if len(cs) == 1}
    steps = 0

    def resolved(c):
        nonlocal steps
        out = []
        changed = False
        stack = [(l, 0) for l in c]
        while stack:
            l, depth = stack.pop()
            steps += 1
            d = single.get(-l)
            if d is not None and d is not c and depth < 3:
                changed = True
                stack.extend((m, depth + 1) for m in d if m != -l)
            else:
                out.append(l)
            if len(out) + len(stack) > MAX_ROW:
                return None
        return out if changed else None

    seen = set()
    rows = []
    made = set()
    for c in f.live:
        if len(c) < MIN_ROW:
            continue
        key = tuple(sorted(c))
        if key not in seen:
            seen.add(key)
            rows.append(key)
        if steps < ROW_STEPS and 2 * sum(1 for l in c if -l in single and single[-l] is not c) >= len(c):
            r = resolved(c)
            if r is not None:
                s = set(r)
                if MIN_ROW <= len(s) <= MAX_ROW and not any(-l in s for l in s):
                    key = tuple(sorted(s))
                    if key not in seen:
                        seen.add(key)
                        rows.append(key)
                        made.add(key)
    return rows, made


# ---------------------------------------------------------------- the census

def row_shape(lits, ex):
    """What the row's own literals do to each other: 'exact' (all exclude),
    'plain' (none do, or in five or more literals none excludes more than a
    quarter of the others), 'guarded' (all but one literal exclude each
    other, the one excludes nothing) or 'partial' (a gate's clause and the
    like). A pair counts from either side."""
    n = len(lits)
    inside = set(lits)
    deg = {l: 0 for l in lits}
    for l in lits:
        for m in ex[l] & inside:
            deg[l] += 1
    # the pairs seen from one side only
    for l in lits:
        for m in ex[l] & inside:
            if l not in ex[m]:
                deg[m] += 1
    top = max(deg.values())
    if top == 0 or (n >= 5 and top <= (n - 1) // 4):
        return "plain"
    if all(d == n - 1 for d in deg.values()):
        return "exact"
    zero = [l for l, d in deg.items() if d == 0]
    if n >= 4 and len(zero) == 1 and all(d == n - 2 for l, d in deg.items() if d != 0):
        return "guarded"
    return "partial"


class Census:
    """Two stages within the budget. First every row is classified, round
    robin over its class, up to CLASSIFY_SHARE of the budget. Then the
    domains among them look for the rows they differ from and for the grids
    through those; a row known to be no domain is skipped there, which is
    what keeps a wide row's scan cheap on a formula full of gates."""

    def __init__(self, f: Formula, budget: int, rng):
        self.f = f
        self.rng = rng
        self.prop = Propagator(f, budget)
        rows, made = build_rows(f)
        self.n_rows = len(rows)
        hb = f.has_binary
        self.rows = [r for r in rows if sum(1 for l in r if l in hb) >= 2]
        self.made = made
        self.rows_of = defaultdict(list)
        for ri, r in enumerate(self.rows):
            for l in r:
                self.rows_of[l].append(ri)
        self.ex = Exclusions(self.prop, self.rows_of)
        self.read = Counter()
        self.exact = Counter()
        self.is_domain = Counter()
        self.differ = Counter()
        self.exact_sizes = Counter()
        self.class_size = Counter()
        self.shape = {}             # row id -> what row_shape said
        self.grids = {}             # frozenset of row ids -> column label of each literal
        self.whole_of = defaultdict(set)   # row id -> rows it shares a whole grid with
        self.rows_read = 0
        self.cut = []
        self.row_cap = max(1, budget // ROW_SHARE)
        self.stopped = ""
        self.linked = set()
        self.sampled = set()
        self.counted_differ = set()
        self.counted_linked = set()
        self.linked_sample = Counter()
        self.queues = []
        self.domain_rows_of = defaultdict(list)   # filled after classifying: rows that may be domains
        self.domain_ex = {}         # literal -> what it excludes among those rows' literals

    def row_class(self, row):
        """(one sign or not, length bucket, a resolvent or not); a row is a
        sorted tuple, so its two ends tell whether it holds one sign."""
        n = len(row)
        bucket = n if n <= 4 else (8 if n <= 8 else (16 if n <= 16 else (64 if n <= 64 else 65)))
        return (row[0] < 0) == (row[-1] < 0), bucket, row in self.made

    def domain(self, ri):
        return [l for l in self.rows[ri] if l not in self.ex.failed]

    def exclusions(self, lits):
        ex = {}
        for l in lits:
            e = self.ex.of(l)
            if e is None:
                return None
            ex[l] = e
        return ex

    def classify(self, ri, limit):
        """Reads what the row's literals do to each other. False when the
        budget or this stage's limit stopped it."""
        lits = self.rows[ri]
        start = self.prop.used
        for l in lits:
            if self.prop.used >= limit or self.ex.of(l) is None:
                return False
            if self.prop.used - start > self.row_cap:
                return True     # stays unclassified: a row this dear is read as it comes up
        ex = self.exclusions(lits)
        if ex is None:
            return False
        dom = self.domain(ri)
        cls = self.row_class(lits)
        self.rows_read += 1
        self.read[cls] += 1
        self.sampled.add(ri)
        self.prop.used += len(dom)
        shape = row_shape(dom, ex) if len(dom) >= 2 else "partial"
        self.shape[ri] = shape
        if shape == "exact":
            self.exact[cls] += 1
            self.exact_sizes[len(dom)] += 1
        if shape != "partial":
            self.is_domain[cls] += 1
        return True

    def index_domains(self):
        """After a turn of classifying: the rows that are or may be domains,
        which is all that linking looks at. What each literal excludes among
        them is read again from the fuller exclusions."""
        self.domain_rows_of = defaultdict(list)
        self.domain_ex = {}
        for ri, r in enumerate(self.rows):
            if self.shape.get(ri) != "partial":
                for l in r:
                    self.domain_rows_of[l].append(ri)

    def link_exclusions(self, lits):
        """What each literal excludes among the domains' literals, from
        either side as far as has been read, cached."""
        ex = {}
        cache = self.domain_ex
        universe = self.domain_rows_of
        incoming = self.ex.incoming
        for l in lits:
            got = cache.get(l)
            if got is None:
                e = self.ex.of(l)
                if e is None:
                    return None
                back = incoming.get(l, Exclusions.EMPTY)
                self.prop.used += len(e) + len(back)
                got = frozenset(m for m in e if m in universe) | frozenset(m for m in back if m in universe)
                cache[l] = got
            ex[l] = got
        return ex

    def link(self, ri):
        """The rows this domain differs from and the grids through it; a row
        that passes its share of the budget goes on `cut`. False when the
        budget stopped it."""
        dom = self.domain(ri)
        ex = self.link_exclusions(dom)
        if ex is None:
            return False
        room = self.prop.budget - self.prop.used
        self.row_cap = max(self.prop.budget // ROW_SHARE, room // 4)
        start = self.prop.used
        nbrs = self.neighbours(ri, dom, ex, start)
        if nbrs is None:
            return False
        if nbrs == "cut":
            self.cut.append(ri)
            return True
        # the counts come from the sample `classify` read, not from rows a
        # grid's growth classified on the way
        sampled = ri in self.sampled
        if sampled and ri not in self.counted_linked:
            self.counted_linked.add(ri)
            self.linked_sample[self.row_class(self.rows[ri])] += 1
        if nbrs:
            if sampled and ri not in self.counted_differ:
                self.counted_differ.add(ri)
                self.differ[self.row_class(self.rows[ri])] += 1
            if not self.grow(ri, nbrs, start):
                self.cut.append(ri)
        return True

    def neighbours(self, ri, dom, ex, start):
        """The rows this one differs from, by what its own literals exclude:
        row id -> number of pairs. None when the budget stopped it, "cut"
        past the row's share of the budget."""
        pairs = defaultdict(list)
        own = set(self.rows[ri])
        rows_of = self.domain_rows_of
        for l in dom:
            if self.prop.spent():
                return None
            if self.prop.used - start > self.row_cap:
                return "cut"
            for m in ex[l]:
                holders = rows_of.get(m, ())
                self.prop.used += 1 + len(holders)
                for v in holders:
                    if v != ri:
                        pairs[v].append((l, m))
        out = {}
        for v, ps in pairs.items():
            if len(ps) < 2:
                continue
            ls = len({p[0] for p in ps})
            ms = len({p[1] for p in ps})
            if ls < 2 or ms < 2 or len(ps) > 3 * min(ls, ms):
                continue
            if own.intersection(self.rows[v]):
                continue
            out[v] = len(ps)
        return out

    def fits(self, v, lit_row, rev, n_members):
        """Whether row v joins the rows of a grid: no literal in common, at
        least two literals on each side against the first row, an exclusion
        with at least four in five of the others (all of them below five),
        and sparse exclusions throughout. `lit_row` says which member holds a
        literal, `rev` which of the members' literals exclude a literal.
        Returns its exclusions, or None."""
        lits = self.domain(v)
        if len(lits) < 2:
            return None
        if self.shape.get(v) is None:
            whole_ex = self.exclusions(lits)
            if whole_ex is None:
                return None
            lits = [m for m in lits if m not in self.ex.failed]
            self.shape[v] = row_shape(lits, whole_ex) if len(lits) >= 2 else "partial"
        if self.shape[v] == "partial":
            return None
        ex = self.link_exclusions(lits)
        if ex is None:
            return None
        lits = [m for m in lits if m not in self.ex.failed]
        if any(m in lit_row for m in lits):
            return None
        edges = [0] * n_members
        ours = [set() for _ in range(n_members)]
        theirs = [set() for _ in range(n_members)]
        for m in lits:
            e = ex[m]
            from_grid = rev.get(m, ())
            self.prop.used += 1 + len(e) + len(from_grid)
            for x in e:
                r = lit_row.get(x)
                if r is not None:
                    edges[r] += 1
                    ours[r].add(m)
                    theirs[r].add(x)
            for x in from_grid:
                if x not in e:
                    r = lit_row[x]
                    edges[r] += 1
                    ours[r].add(m)
                    theirs[r].add(x)
        met = 0
        for r in range(n_members):
            if edges[r]:
                if edges[r] > 3 * min(len(ours[r]), len(theirs[r])):
                    return None
                met += 1
        need = n_members if n_members < 5 else (4 * n_members + 4) // 5
        if met < need or len(ours[0]) < 2 or len(theirs[0]) < 2:
            return None
        return ex

    def read_columns(self, members):
        """The columns of a grid whose rows are fixed: a clique cover of the
        literals, one literal per row in each clique. From a literal the next
        row is the one with the fewest literals left that exclude the clique,
        and on a tie the literal that leaves the others the most. Returns the
        label of each literal, or None when the budget stopped it."""
        rows = []
        lit_at = {}
        for i, ri in enumerate(members):
            lits = self.domain(ri)
            rows.append(lits)
            for l in lits:
                lit_at[l] = i
        excl = {l: set() for l in lit_at}
        for l in lit_at:
            e = self.link_exclusions([l])
            if e is None:
                return None
            self.prop.used += 1 + len(e[l])
            for m in e[l]:
                if m in lit_at and lit_at[m] != lit_at[l]:
                    excl[l].add(m)
                    excl[m].add(l)
        col = {}
        fresh = 0
        for lits in rows:
            for l in lits:
                if l in col:
                    continue
                col[l] = fresh
                options = defaultdict(set)
                for m in excl[l]:
                    if m not in col:
                        options[lit_at[m]].add(m)
                self.prop.used += 1 + len(excl[l])
                while options:
                    j = min(options, key=lambda k: (len(options[k]), k))
                    pick = options.pop(j)
                    if len(pick) == 1:
                        m = next(iter(pick))
                    else:
                        m = max(sorted(pick), key=lambda x: sum(len(o & excl[x]) for o in options.values()))
                        self.prop.used += len(pick) * (1 + len(options))
                    col[m] = fresh
                    for k in list(options):
                        options[k] &= excl[m]
                        if not options[k]:
                            del options[k]
                fresh += 1
        return col

    def grow(self, ri, nbrs, start):
        """Maximal sets of rows through this row, each seeded by a neighbour
        no set holds yet and no whole grid shares with it, every neighbour
        tried in each. False when the row's share of the budget cut it short."""
        order = sorted(nbrs, key=lambda v: (-nbrs[v], v))
        uncovered = set(order) - self.whole_of.get(ri, set())
        for _ in range(MAX_CLIQUES):
            if not uncovered:
                break
            if self.prop.spent():
                return False
            seed = next(v for v in order if v in uncovered)
            own = self.link_exclusions(self.domain(ri))
            if own is None:
                return False
            lit_row = {}
            rev = defaultdict(list)
            for l, e in own.items():
                lit_row[l] = 0
                for y in e:
                    rev[y].append(l)
            members = [ri]
            whole = True
            fails = 0
            for v in [seed] + [u for u in order if u != seed]:
                if self.prop.spent() or self.prop.used - start > self.row_cap:
                    whole = False
                    break
                ex = self.fits(v, lit_row, rev, len(members))
                if ex is None:
                    fails += 1
                    if fails >= MAX_FAILS:
                        break
                    continue
                fails = 0
                for m, e in ex.items():
                    lit_row[m] = len(members)
                    for y in e:
                        rev[y].append(m)
                members.append(v)
            uncovered.discard(seed)
            uncovered.difference_update(members)
            if len(members) >= 3:
                key = frozenset(members)
                if key not in self.grids:
                    col = self.read_columns(members)
                    if col is None:
                        return False
                    self.grids[key] = col
            if whole:
                for a in members:
                    self.whole_of[a].update(members)
            else:
                return False
        return True

    def classify_some(self, limit):
        """Classifies rows round robin up to `limit` visits; the queues keep
        their place for the next turn."""
        while self.queues:
            nxt = []
            for i, q in enumerate(self.queues):
                if not self.classify(q[0], limit):
                    # the row stays at the head of its queue for the next turn
                    self.queues = nxt + self.queues[i:]
                    return
                q.popleft()
                if q:
                    nxt.append(q)
            self.queues = nxt

    def link_new(self):
        """Round robin over the domains not yet linked, by shape and size,
        exact-one rows first; then the second pass over what the share cut
        short. False when the budget stopped it."""
        by_kind = defaultdict(list)
        for ri, shape in self.shape.items():
            if shape != "partial" and ri not in self.linked:
                by_kind[(shape != "exact", self.row_class(self.rows[ri])[1])].append(ri)
        for rs in by_kind.values():
            self.rng.shuffle(rs)
        queues = [deque(rs) for _, rs in sorted(by_kind.items())]
        while queues:
            nxt = []
            for q in queues:
                ri = q.popleft()
                self.linked.add(ri)
                if not self.link(ri):
                    return False
                if q:
                    nxt.append(q)
            queues = nxt
        cut = self.cut
        self.cut = []
        for i, ri in enumerate(cut):
            if self.prop.spent() or not self.link(ri):
                self.cut.extend(cut[i:])
                return False
        return True

    def run(self):
        by_class = defaultdict(list)
        for ri, r in enumerate(self.rows):
            by_class[self.row_class(r)].append(ri)
        for cls, rs in by_class.items():
            self.rng.shuffle(rs)
            self.class_size[cls] = len(rs)
        self.queues = [deque(rs) for _, rs in sorted(by_class.items(), key=lambda kv: str(kv[0]))]
        limit = int(self.prop.budget * CLASSIFY_SHARE)
        while True:
            self.classify_some(limit)
            self.index_domains()
            if not self.link_new():
                self.stopped = "budget"
                break
            if not self.queues:
                break
            if self.prop.spent():
                self.stopped = "budget"
                break
            # what is left goes half to classifying and half to linking
            limit = self.prop.used + max(1, (self.prop.budget - self.prop.used) // 2)
        self.drop_parts()

    def drop_parts(self):
        """A grid whose rows another, larger grid holds is a part of it, from
        a read the share cut short or a neighbour tried in another order."""
        kept = []
        for k in sorted(self.grids, key=len, reverse=True):
            if any(k < o for o in kept[:200]):
                del self.grids[k]
            else:
                kept.append(k)

    def estimate(self, counts):
        """Per class, the share read scaled to the class's size."""
        total = 0.0
        for cls, n in self.read.items():
            total += counts[cls] / n * self.class_size[cls]
        return int(round(total))

    def estimate_differ(self):
        """Per class, the estimated domains times the share of the linked
        sample that differs from another row."""
        total = 0.0
        for cls, n in self.linked_sample.items():
            total += self.is_domain[cls] / self.read[cls] * self.class_size[cls] * self.differ[cls] / n
        return int(round(total))


def grid_shape(col):
    """(columns two or more rows share, values one row has alone)."""
    sizes = Counter(col.values())
    shared = sum(1 for k in sizes.values() if k >= 2)
    return shared, len(sizes) - shared


def is_core(n, col):
    shared, private = grid_shape(col)
    return n >= 3 and shared >= 3 and private <= shared


def grid_text(n, shared, private):
    return f"{n}w{shared}c" + (f"+{private}p" if private else "")


def domains(rows_in_grid, col):
    """Each row's values as small numbers, and how many values there are."""
    labels = {}
    out = []
    for lits in rows_in_grid:
        dom = set()
        for l in lits:
            a = col.get(l)
            if a is not None:
                dom.add(labels.setdefault(a, len(labels)))
        out.append(sorted(dom))
    return out, len(labels)


# ------------------------------------------------------------------ the filter

def hopcroft_karp(adj, n_left, n_right):
    """Maximum matching; adj[u] lists the right vertices of left u."""
    inf = float("inf")
    pair_u = [-1] * n_left
    pair_v = [-1] * n_right
    dist = [0] * n_left

    def bfs():
        dq = deque()
        found = False
        for u in range(n_left):
            if pair_u[u] == -1:
                dist[u] = 0
                dq.append(u)
            else:
                dist[u] = inf
        while dq:
            u = dq.popleft()
            for v in adj[u]:
                pu = pair_v[v]
                if pu == -1:
                    found = True
                elif dist[pu] == inf:
                    dist[pu] = dist[u] + 1
                    dq.append(pu)
        return found

    def dfs(u):
        for v in adj[u]:
            pu = pair_v[v]
            if pu == -1 or (dist[pu] == dist[u] + 1 and dfs(pu)):
                pair_u[u] = v
                pair_v[v] = u
                return True
        dist[u] = inf
        return False

    size = 0
    while bfs():
        for u in range(n_left):
            if pair_u[u] == -1 and dfs(u):
                size += 1
    return size, pair_u, pair_v


def hall_filter(adj, d):
    """Regin's filtering. adj[u] lists the values row u may take, out of d.
    Returns (rows a maximum matching leaves out, edges no matching of every
    row can use); on a violation the second number is 0."""
    n = len(adj)
    size, pair_u, pair_v = hopcroft_karp(adj, n, d)
    if size < n:
        return n - size, 0
    # the alternating graph: a matched edge runs row -> value, a free edge
    # value -> row. A free edge (u, v) is in some matching of every row iff v
    # is reached from an unmatched value, or u and v share a strongly
    # connected component.
    succ = [[] for _ in range(n + d)]
    for u in range(n):
        for v in adj[u]:
            if pair_u[u] == v:
                succ[u].append(n + v)
            else:
                succ[n + v].append(u)
    reach = [False] * (n + d)
    dq = deque()
    for v in range(d):
        if pair_v[v] == -1:
            reach[n + v] = True
            dq.append(n + v)
    while dq:
        x = dq.popleft()
        for y in succ[x]:
            if not reach[y]:
                reach[y] = True
                dq.append(y)
    index = [-1] * (n + d)
    low = [0] * (n + d)
    comp = [-1] * (n + d)
    on = [False] * (n + d)
    stack = []
    counter = 0
    for s in range(n + d):
        if index[s] != -1:
            continue
        work = [(s, 0)]
        index[s] = low[s] = counter
        counter += 1
        stack.append(s)
        on[s] = True
        while work:
            x, i = work[-1]
            if i < len(succ[x]):
                work[-1] = (x, i + 1)
                y = succ[x][i]
                if index[y] == -1:
                    index[y] = low[y] = counter
                    counter += 1
                    stack.append(y)
                    on[y] = True
                    work.append((y, 0))
                elif on[y]:
                    low[x] = min(low[x], index[y])
            else:
                work.pop()
                if work:
                    px = work[-1][0]
                    low[px] = min(low[px], low[x])
                if low[x] == index[x]:
                    while True:
                        y = stack.pop()
                        on[y] = False
                        comp[y] = x
                        if y == x:
                            break
    removed = 0
    for u in range(n):
        for v in adj[u]:
            if pair_u[u] == v or reach[n + v] or comp[u] == comp[n + v]:
                continue
            removed += 1
    return 0, removed


def amo_cover(f: Formula, rows_of, rows):
    """A greedy clique cover of the literals the binary clauses exclude
    directly, in order of degree. Returns (cliques of three or more, the
    largest, how many have no row inside them: a clause all of whose literals
    are in the clique is what makes one of them true)."""
    adj = defaultdict(set)
    pair_with = defaultdict(list)     # literal -> the other literal of its two-literal clauses
    for c in f.live:
        if len(c) == 2:
            a, b = -c[0], -c[1]
            adj[a].add(b)
            adj[b].add(a)
            pair_with[c[0]].append(c[1])
            pair_with[c[1]].append(c[0])
    steps = 0
    free = set(adj)
    cliques = 0
    largest = 0
    only = 0
    for v in sorted(adj, key=lambda x: (-len(adj[x]), x)):
        if v not in free:
            continue
        if steps > AMO_BUDGET:
            break
        free.discard(v)
        clique = [v]
        cand = adj[v] & free
        steps += len(adj[v])
        while cand and steps <= AMO_BUDGET:
            u = min(cand)
            clique.append(u)
            free.discard(u)
            cand = (cand & adj[u]) - {u}
            steps += min(len(cand) + 1, len(adj[u]))
        if len(clique) >= 3:
            cliques += 1
            largest = max(largest, len(clique))
            s = set(clique)
            covered = False
            for l in clique:
                holders = rows_of.get(l, ())
                others = pair_with.get(l, ())
                steps += len(holders) + len(others)
                if any(o in s for o in others) or any(len(rows[ri]) <= len(s) and s.issuperset(rows[ri]) for ri in holders):
                    covered = True
                    break
            if not covered:
                only += 1
    return cliques, largest, only


# ------------------------------------------------------------------ one cell

def analyse(clauses, budget=BUDGET, seed=1):
    f = Formula(clauses)
    out = {"root_units": len(f.fixed), "root_conflict": int(f.conflict)}
    if f.conflict:
        return out, None
    cen = Census(f, budget, random.Random(seed))
    cen.run()
    grids3 = {k: v for k, v in cen.grids.items() if len(k) >= 3}
    core = {k: v for k, v in grids3.items() if is_core(len(k), v)}
    in_core = set()
    for k in core:
        in_core |= k
    shapes = []
    max_tight = 0.0
    max_n = 0
    grids_big = 0
    big_rows = set()
    big_tight = 0.0
    big_n = 0
    for k, col in grids3.items():
        n = len(k)
        shared, private = grid_shape(col)
        tight = n / max(1, shared + private)
        if k in core:
            shapes.append((-(n >= BIG), -tight, -n, grid_text(n, shared, private)))
            max_tight = max(max_tight, tight)
            max_n = max(max_n, n)
            if n >= BIG:
                grids_big += 1
                big_rows |= k
                if (tight, n) > (big_tight, big_n):
                    big_tight, big_n = tight, n
    shapes.sort()
    texts = [s[3] for s in shapes]
    hall_grids = hall_removed = hall_viol = 0
    for k, col in sorted(grids3.items(), key=lambda kv: (kv[0] not in core, -len(kv[0]))):
        n = len(k)
        shared, private = grid_shape(col)
        if n * (shared + private) > HALL_MAX_CELLS or hall_grids >= HALL_MAX_GRIDS:
            continue
        adj, d = domains([cen.rows[ri] for ri in k], col)
        short, removed = hall_filter(adj, d)
        hall_grids += 1
        hall_removed += removed
        hall_viol += int(short > 0)
    cliques, largest, only = amo_cover(f, cen.rows_of, cen.rows)
    out.update(rows=cen.n_rows, rows_active=len(cen.rows), rows_read=cen.rows_read, rows_cut=len(cen.cut),
               rows_exact=sum(cen.exact.values()), est_exact=cen.estimate(cen.exact),
               exact_sizes=";".join(f"{k}x{v}" for k, v in sorted(cen.exact_sizes.items(), key=lambda kv: -kv[1])[:6]),
               rows_domain=sum(cen.is_domain.values()), est_domain=cen.estimate(cen.is_domain),
               rows_differ=sum(cen.differ.values()), est_differ=cen.estimate_differ(),
               grids3=len(grids3), grids_core=len(core), rows_in_core=len(in_core), max_n=max_n, max_tight=round(max_tight, 3),
               grids_big=grids_big, big_rows=len(big_rows), big_tight=round(big_tight, 3), big_n=big_n,
               grids=";".join(texts[:8]) + (f";+{len(texts) - 8}" if len(texts) > 8 else ""),
               hall_grids=hall_grids, hall_removed=hall_removed, hall_violations=hall_viol,
               amo_cliques=cliques, amo_max=largest, amo_only=only,
               failed_lits=len(cen.ex.failed), hubs=len(cen.ex.hubs), visits=cen.prop.used, stopped=cen.stopped)
    return out, cen


def read_cell(path):
    _, total = header_counts(path)
    over = total > MAX_READ
    nv, clauses, truncated = read_clauses(path, PREFIX if over else MAX_READ)
    return nv, clauses, truncated, total, over


def census_one(args):
    stem, path, family, budget = args
    t0 = time.time()
    row = {"stem": stem, "family": family}
    try:
        nv, clauses, truncated, total, over = read_cell(path)
        row.update(vars=nv, clauses=len(clauses), total_clauses=total, truncated=int(truncated), over_limit=int(over))
        row["read_s"] = round(time.time() - t0, 1)
        res, _ = analyse(clauses, budget)
        row.update(res)
    except Exception as e:  # noqa: BLE001
        row["error"] = f"{type(e).__name__}: {str(e)[:100]}"
    row["detect_s"] = round(time.time() - t0, 1)
    return row


def _families():
    fam = {}
    if CELLS.is_file():
        with open(CELLS) as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                fam[r["stem"]] = r["family"]
    return fam


def cmd_census(args):
    fam = _families()
    files = [Path(p) for p in args.files] if args.files else sorted(SUITE.glob("*.cnf.xz"))
    jobs = []
    for p in files:
        stem = p.name[: -len(".cnf.xz")] if p.name.endswith(".cnf.xz") else p.stem
        jobs.append((stem, str(p), fam.get(stem, ""), args.budget))
    jobs.sort(key=lambda j: -Path(j[1]).stat().st_size)   # the biggest first, so a slow one does not start last
    with open(args.out, "w", newline="") as out, ProcessPoolExecutor(max_workers=args.workers) as ex:
        wr = csv.DictWriter(out, fieldnames=COLUMNS, delimiter="\t", extrasaction="ignore")
        wr.writeheader()
        out.flush()
        for fut in as_completed([ex.submit(census_one, j) for j in jobs]):
            row = fut.result()
            wr.writerow(row)
            out.flush()
            print(f"{row['stem'][:52]:52} read={row.get('rows_read', '-'):>7}/{row.get('rows_active', '-'):<8} "
                  f"exact={row.get('rows_exact', '-'):>6} differ={row.get('rows_differ', '-'):>6} core={row.get('grids_core', '-'):>5} "
                  f"{str(row.get('grids', ''))[:34]:34} viol={row.get('hall_violations', '-')} rem={row.get('hall_removed', '-')} "
                  f"{row.get('error', '')} {row.get('detect_s', '')}s", flush=True)
    return 0


def cmd_one(args):
    t0 = time.time()
    nv, clauses, truncated, total, over = read_cell(args.file)
    row = {"vars": nv, "clauses": len(clauses), "total_clauses": total, "truncated": int(truncated), "over_limit": int(over)}
    res, _ = analyse(clauses, args.budget)
    row.update(res)
    row["detect_s"] = round(time.time() - t0, 1)
    for k in COLUMNS:
        if k in row:
            print(f"{k:16} {row[k]}")
    return 0


SOLVED = ("SATISFIABLE", "UNSATISFIABLE")


def run_status(runs):
    """Per stem: 'unsolved' when both seeds' arms failed, 'one-seed', or
    'solved'. A stem without a result in both arms has no status."""
    results = defaultdict(dict)
    for arm in ("cur0", "cur1"):
        p = Path(runs) / arm / "results.tsv"
        if p.is_file():
            with open(p) as fh:
                for r in csv.DictReader(fh, delimiter="\t"):
                    results[r["instance"]][arm] = r["result"] in SOLVED
    out = {}
    for stem, s in results.items():
        if len(s) == 2:
            out[stem] = "solved" if all(s.values()) else ("unsolved" if not any(s.values()) else "one-seed")
    return out


def num(row, key):
    v = row.get(key)
    return float(v) if v not in ("", None) else 0.0


def cmd_report(args):
    with open(args.tsv) as fh:
        rows = list(csv.DictReader(fh, delimiter="\t"))
    status = run_status(args.runs)
    byfam = defaultdict(list)
    for r in rows:
        byfam[r["family"] or "?"].append(r)
    big = [r for r in rows if num(r, "grids_big") > 0]
    tight = [r for r in big if num(r, "big_tight") >= 0.5]
    tighter = [r for r in big if num(r, "big_tight") >= 0.9]
    unsolved = [r for r in rows if status.get(r["stem"]) == "unsolved"]
    print(f"cells {len(rows)}; errors {sum(1 for r in rows if r['error'])}; over the clause limit {sum(1 for r in rows if r['over_limit'] == '1')}; "
          f"stopped by the budget {sum(1 for r in rows if r['stopped'])}; unsolved on both seeds {len(unsolved)}")
    print(f"cells with exact-one rows {sum(1 for r in rows if num(r, 'rows_exact') > 0)}; with rows that differ {sum(1 for r in rows if num(r, 'rows_differ') > 0)}; "
          f"with a core grid {sum(1 for r in rows if num(r, 'grids_core') > 0)}; with a big grid {len(big)}; of tightness 0.5 or more {len(tight)}; of 0.9 or more {len(tighter)}")
    print(f"unsolved on both seeds with a big grid {sum(1 for r in unsolved if num(r, 'grids_big') > 0)}; of tightness 0.5 or more {sum(1 for r in unsolved if num(r, 'big_tight') >= 0.5)}; "
          f"of 0.9 or more {sum(1 for r in unsolved if num(r, 'big_tight') >= 0.9)}")
    print(f"root filtering: {sum(int(num(r, 'hall_grids')) for r in rows)} grids, cells with a removal {sum(1 for r in rows if num(r, 'hall_removed') > 0)}, "
          f"cells with a violation {sum(1 for r in rows if num(r, 'hall_violations') > 0)}")
    print(f"\n{'family':22} {'cells':>5} {'>limit':>6} {'unsolv':>6} {'exact':>5} {'differ':>6} {'big':>4} {'t>=.5':>5} {'t>=.9':>5} {'uns+big':>7} {'viol':>4} {'rem':>4}  tightest big grid (unsolved first)")
    for fam, rs in sorted(byfam.items(), key=lambda kv: (-sum(1 for r in kv[1] if num(r, 'grids_big') > 0), -len(kv[1]))):
        uns = [r for r in rs if status.get(r["stem"]) == "unsolved"]
        b = [r for r in rs if num(r, "grids_big") > 0]
        ub = [r for r in uns if num(r, "grids_big") > 0]
        ex = sorted(ub or b, key=lambda r: -num(r, "big_tight"))[:1]
        txt = "; ".join(f"{r['stem'][-24:]} {r['big_n']} rows at {r['big_tight']}" for r in ex)
        print(f"{fam[:22]:22} {len(rs):>5} {sum(1 for r in rs if r['over_limit'] == '1'):>6} {len(uns):>6} "
              f"{sum(1 for r in rs if num(r, 'rows_exact') > 0):>5} {sum(1 for r in rs if num(r, 'rows_differ') > 0):>6} "
              f"{len(b):>4} {sum(1 for r in b if num(r, 'big_tight') >= 0.5):>5} {sum(1 for r in b if num(r, 'big_tight') >= 0.9):>5} {len(ub):>7} "
              f"{sum(1 for r in rs if num(r, 'hall_violations') > 0):>4} {sum(1 for r in rs if num(r, 'hall_removed') > 0):>4}  {txt}")
    print("\nunsolved on both seeds, each with a big grid:")
    for r in sorted(unsolved, key=lambda r: -num(r, "big_tight")):
        if num(r, "grids_big") > 0:
            print(f"  {r['stem'][-44:]:44} {r['family'][:14]:14} over={r['over_limit']} big {int(num(r, 'grids_big')):>5} rows {int(num(r, 'big_rows')):>6} tightest {r['big_n']:>4} rows at {r['big_tight']}  {r['grids'][:40]}")
    return 0


# ------------------------------------------------------------------ the trails

def hall_on_snapshot(rows_in_grid, col, true_lits):
    """Hall reasoning on one assignment. Rows with a true literal are done and
    their columns taken; the others keep the literals that are not false.
    Returns (open rows, violation or not, literals removed, literals unit
    propagation left open in a taken column)."""
    taken = set()
    for lits in rows_in_grid:
        for l in lits:
            if l in true_lits and l in col:
                taken.add(col[l])
    labels = {}
    adj = []
    gap = 0
    for lits in rows_in_grid:
        if any(l in true_lits for l in lits):
            continue
        dom = set()
        for l in lits:
            if -l in true_lits:
                continue
            a = col.get(l)
            if a is None:
                continue
            if a in taken:
                gap += 1
                continue
            dom.add(labels.setdefault(a, len(labels)))
        adj.append(sorted(dom))
    if not adj:
        return 0, False, 0, gap
    short, removed = hall_filter(adj, len(labels))
    return len(adj), short > 0, removed, gap


def hall_violation_set(rows_in_grid, col, true_lits):
    """On a violating assignment: the rows of a Hall set (the alternating
    search from a row a maximum matching leaves out), the values they reach,
    and the explanation clause, every literal of it false now: the rows'
    literals outside those values, where a literal propagation left open in a
    column a true literal t holds counts as ~t (t and it exclude each other).
    None when there is no violation."""
    taken = {}
    for lits in rows_in_grid:
        for l in lits:
            if l in true_lits and l in col:
                taken[col[l]] = l
    labels = {}
    adj = []
    open_rows = []
    for lits in rows_in_grid:
        if any(l in true_lits for l in lits):
            continue
        dom = set()
        for l in lits:
            a = col.get(l)
            if -l in true_lits or a is None or a in taken:
                continue
            dom.add(labels.setdefault(a, len(labels)))
        adj.append(sorted(dom))
        open_rows.append(lits)
    if not adj:
        return None
    size, pair_u, pair_v = hopcroft_karp(adj, len(adj), len(labels))
    if size == len(adj):
        return None
    start = next(u for u in range(len(adj)) if pair_u[u] == -1)
    rows = {start}
    vals = set()
    stack = [start]
    while stack:
        u = stack.pop()
        for v in adj[u]:
            if v not in vals:
                vals.add(v)
                w = pair_v[v]
                if w != -1 and w not in rows:
                    rows.add(w)
                    stack.append(w)
    inv = {i: a for a, i in labels.items()}
    kept = {inv[v] for v in vals}
    reason = set()
    for u in rows:
        for l in open_rows[u]:
            a = col.get(l)
            if a not in kept:
                reason.add(l if -l in true_lits or a not in taken else -taken[a])
    return [open_rows[u] for u in sorted(rows)], kept, sorted(reason)


def read_snapshots(path, limit):
    """Up to `limit` assignments spread evenly over the file, one line of
    literals each. The file is counted first and only the chosen lines are
    parsed, so a long log costs no memory."""
    with open(path, "rb") as fh:
        total = sum(1 for line in fh if line.strip())
    if total == 0:
        return []
    step = max(1.0, total / limit)
    chosen = {int(i * step) for i in range(min(limit, total))}
    out = []
    i = 0
    with open(path, "rb") as fh:
        for line in fh:
            if not line.strip():
                continue
            if i in chosen:
                out.append([int(t) for t in line.split()])
            i += 1
    return out


def trail_report(cen, snapshots, grid_limit, rng, samples=None):
    """Over every snapshot and a sample of the core grids. `samples`, a list,
    collects each violation's (rows, values, reason) for a closer look."""
    grids = [k for k, col in cen.grids.items() if is_core(len(k), col)]
    rng.shuffle(grids)
    grids = grids[:grid_limit]
    rows = {k: [cen.rows[ri] for ri in k] for k in grids}
    out = Counter()
    trail_sum = 0
    for snap in snapshots:
        s = set(snap)
        out["snapshots"] += 1
        trail_sum += len(snap)
        viol = rem = False
        for k in grids:
            n_open, v, removed, gap = hall_on_snapshot(rows[k], cen.grids[k], s)
            out["grid_checks"] += 1
            out["open_rows"] += n_open
            out["gap"] += gap
            if n_open >= 2:
                out["live_checks"] += 1
            if v:
                out["violations"] += 1
                viol = True
                got = hall_violation_set(rows[k], cen.grids[k], s)
                if got is not None:
                    out["hall_rows"] += len(got[0])
                    out["hall_reason"] += len(got[2])
                    if samples is not None:
                        samples.append((out["snapshots"], got))
            if removed:
                out["removals"] += removed
                out["grids_with_removal"] += 1
                rem = True
        out["snapshots_with_violation"] += viol
        out["snapshots_with_removal"] += rem
    out["grids"] = len(grids)
    out["avg_trail"] = trail_sum // max(1, out["snapshots"])
    return out


def cmd_grids(args):
    """The core grids, for the scratch experiment: a `G` line for each, then a
    row per line as literal:column pairs."""
    nv, clauses, truncated, total, over = read_cell(args.file)
    res, cen = analyse(clauses, args.budget)
    if cen is None:
        print("root conflict")
        return 0
    n = 0
    with open(args.out, "w") as fh:
        for k, col in cen.grids.items():
            if not is_core(len(k), col) or len(k) < args.min_rows:
                continue
            n += 1
            fh.write(f"G {len(k)}\n")
            for ri in sorted(k):
                fh.write(" ".join(f"{l}:{col[l]}" for l in cen.rows[ri] if l in col) + "\n")
    print(f"{Path(args.file).name[:60]}: {n} grids written, rows read {res['rows_read']}/{res['rows_active']}, stopped {res['stopped'] or 'no'}")
    return 0


def cmd_trail(args):
    t0 = time.time()
    nv, clauses, truncated, total, over = read_cell(args.file)
    res, cen = analyse(clauses, args.budget)
    if cen is None:
        print("root conflict")
        return 0
    snaps = read_snapshots(args.snapshots, args.snapshots_limit)
    samples = []
    out = trail_report(cen, snaps, args.grids, random.Random(2), samples)
    print(f"cell {Path(args.file).name[:60]} rows_read {res['rows_read']}/{res['rows_active']} core {res['grids_core']} tightest {res['max_tight']} "
          f"stopped {res['stopped'] or 'no'} took {time.time() - t0:.0f} s")
    for k in ("snapshots", "avg_trail", "grids", "grid_checks", "live_checks", "open_rows", "snapshots_with_violation",
              "violations", "snapshots_with_removal", "grids_with_removal", "removals", "gap"):
        print(f"{k:28} {out[k]}")
    if out["violations"]:
        print(f"{'hall set rows, mean':28} {out['hall_rows'] / out['violations']:.1f}")
        print(f"{'reason literals, mean':28} {out['hall_reason'] / out['violations']:.1f}")
        first = samples[0][0] if samples else 0
        print(f"{'first violation at snapshot':28} {first} of {out['snapshots']}")
    if args.check:
        return check_samples(args, samples, clauses)
    return 0


def check_samples(args, samples, clauses):
    """The measurement's own check: each sampled violation says the formula
    with its reason's literals false is unsatisfiable, whatever the rest of
    the trail held. The solver decides that formula on its own."""
    rng = random.Random(3)
    picked = rng.sample(samples, min(args.check, len(samples)))
    verdicts = Counter()
    tmp = Path(args.tmp)
    tmp.mkdir(parents=True, exist_ok=True)
    for i, (_, (rows, kept, reason)) in enumerate(picked):
        cnf = tmp / f"check_{i}.cnf"
        with open(cnf, "w") as fh:
            nv = max(abs(l) for c in clauses for l in c)
            fh.write(f"p cnf {nv} {len(clauses) + len(reason)}\n")
            for c in clauses:
                fh.write(" ".join(map(str, c)) + " 0\n")
            for l in reason:
                fh.write(f"{-l} 0\n")
        try:
            p = subprocess.run([args.solver, "-q", "--time=" + str(args.check_time), str(cnf)], capture_output=True, text=True, timeout=args.check_time + 30)
            status = next((ln[2:].strip() for ln in p.stdout.splitlines() if ln.startswith("s ")), "NONE")
        except subprocess.TimeoutExpired:
            status = "TIMEOUT"
        verdicts[status] += 1
        print(f"check {i}: {len(rows)} rows, {len(kept)} values, {len(reason)} reason literals: {status}", flush=True)
        cnf.unlink()
    print("checks:", dict(verdicts))
    return 1 if verdicts.get("SATISFIABLE") else 0


# ------------------------------------------------------------------- selftest

class Enc:
    def __init__(self):
        self.n = 0
        self.cl = []

    def var(self):
        self.n += 1
        return self.n

    def amo_pairwise(self, lits):
        for a, b in itertools.combinations(lits, 2):
            self.cl.append([-a, -b])

    def amo_ladder(self, lits):
        s = [self.var() for _ in lits[:-1]]
        for i, x in enumerate(lits):
            if i < len(s):
                self.cl.append([-x, s[i]])
            if i > 0:
                self.cl.append([-x, -s[i - 1]])
            if 0 < i < len(s):
                self.cl.append([-s[i - 1], s[i]])

    def amo_commander(self, lits, g=2):
        groups = [lits[i:i + g] for i in range(0, len(lits), g)]
        cmds = []
        for grp in groups:
            c = self.var()
            cmds.append(c)
            self.amo_pairwise(grp)
            for x in grp:
                self.cl.append([-x, c])
        self.amo_pairwise(cmds)

    def alo(self, lits):
        self.cl.append(list(lits))


def toy_latin(n, enc="pairwise"):
    e = Enc()
    x = [[[e.var() for _ in range(n)] for _ in range(n)] for _ in range(n)]
    amo = {"pairwise": e.amo_pairwise, "ladder": e.amo_ladder, "commander": e.amo_commander}[enc]
    for r in range(n):
        for c in range(n):
            e.alo(x[r][c])
            amo(x[r][c])
    for r in range(n):
        for v in range(n):
            amo([x[r][c][v] for c in range(n)])
    for c in range(n):
        for v in range(n):
            amo([x[r][c][v] for r in range(n)])
    return e


def toy_sudoku(b):
    """A sudoku of order b*b with all four kinds of exact-one."""
    n = b * b
    e = Enc()
    x = [[[e.var() for _ in range(n)] for _ in range(n)] for _ in range(n)]

    def exact(lits):
        e.alo(lits)
        e.amo_pairwise(lits)

    for r in range(n):
        for c in range(n):
            exact(x[r][c])
    for v in range(n):
        for r in range(n):
            exact([x[r][c][v] for c in range(n)])
        for c in range(n):
            exact([x[r][c][v] for r in range(n)])
        for br in range(b):
            for bc in range(b):
                exact([x[br * b + i][bc * b + j][v] for i in range(b) for j in range(b)])
    return e


def toy_php(pigeons, holes, row_amo=False):
    e = Enc()
    x = [[e.var() for _ in range(holes)] for _ in range(pigeons)]
    for p in range(pigeons):
        e.alo(x[p])
        if row_amo:
            e.amo_pairwise(x[p])
    for h in range(holes):
        e.amo_pairwise([x[p][h] for p in range(pigeons)])
    return e


def toy_colouring(edges, n, colours):
    e = Enc()
    x = [[e.var() for _ in range(colours)] for _ in range(n)]
    for v in range(n):
        e.alo(x[v])
    for a, b in edges:
        for c in range(colours):
            e.cl.append([-x[a][c], -x[b][c]])
    return e


def toy_circuit(n):
    e = Enc()
    ins = [e.var() for _ in range(n)]
    cur = ins[0]
    for a in ins[1:]:
        o = e.var()
        e.cl += [[-o, cur], [-o, a], [o, -cur, -a]]
        cur = o
    return e


def toy_xor(n):
    e = Enc()
    v = [e.var() for _ in range(n)]
    for a, b, c in zip(v, v[1:], v[2:]):
        for s in itertools.product([1, -1], repeat=3):
            if s[0] * s[1] * s[2] == -1:
                e.cl.append([s[0] * a, s[1] * b, s[2] * c])
    return e


def flipped(e, seed):
    rng = random.Random(seed)
    sign = {v: rng.choice([1, -1]) for v in range(1, e.n + 1)}
    return [[l * sign[abs(l)] for l in c] for c in e.cl]


def grid_errors(cen, key, col, clauses):
    """What is wrong with a grid, by first principles: two literals of one
    column that some propagation does not exclude, two literals of one row in
    one column, a row that is not implied."""
    prop = Propagator(cen.f, 10 ** 9)
    errors = []
    by_col = defaultdict(list)
    for l, a in col.items():
        by_col[a].append(l)
    reach = {}
    for lits in by_col.values():
        for x, y in itertools.combinations(lits, 2):
            for u in (x, y):
                if u not in reach:
                    r = prop.reach(u)
                    reach[u] = None if r in (FAILED, TOO_BIG) else set(r)
            if not ((reach[x] is None or -y in reach[x]) or (reach[y] is None or -x in reach[y])):
                errors.append(f"column pair {x} {y}")
    for ri in key:
        labels = [col[l] for l in cen.rows[ri] if l in col]
        if len(labels) != len(set(labels)):
            errors.append(f"row {ri} twice in a column")
    return errors


def brute_filter(adj):
    """hall_filter by enumeration, for the property check."""
    n = len(adj)
    usable = set()
    matched = False
    for pick in itertools.product(*adj):
        if len(set(pick)) == n:
            matched = True
            usable.update(enumerate(pick))
    if not matched:
        return True, 0
    return False, sum(len(a) for a in adj) - len(usable)


def cmd_selftest(args):
    ok = True

    def check(name, cond, detail=""):
        nonlocal ok
        print(("ok   " if cond else "FAIL ") + name + (f"  ({detail})" if detail != "" else ""))
        ok &= bool(cond)

    def run(clauses, budget=20_000_000):
        return analyse(clauses, budget)[0]

    def has(r, text):
        return text in r["grids"].split(";")

    # the filter against enumeration
    rng = random.Random(7)
    bad = 0
    for _ in range(3000):
        n = rng.randint(1, 5)
        d = rng.randint(1, 6)
        adj = [sorted(rng.sample(range(d), rng.randint(1, d))) for _ in range(n)]
        short, removed = hall_filter(adj, d)
        bv, br = brute_filter(adj)
        if (short > 0) != bv or removed != br:
            bad += 1
    check("Hall filter agrees with enumeration on 3,000 random grids", bad == 0, bad)

    for enc in ("pairwise", "ladder", "commander"):
        r = run(toy_latin(4, enc).cl)
        check(f"latin 4 {enc}: the 16 cells are exact-one", r["rows_exact"] >= 16, f"{r['rows_exact']} {r['exact_sizes']}")
        check(f"latin 4 {enc}: rows and columns of the square read as 4 by 4 grids", has(r, "4w4c") and r["grids3"] >= 8, f"{r['grids3']} {r['grids']}")
        check(f"latin 4 {enc}: nothing to remove at the root", r["hall_removed"] == 0 and r["hall_violations"] == 0, f"{r['hall_removed']} {r['hall_violations']}")
    r = run(flipped(toy_latin(4), 3))
    check("latin 4 with every variable's sign flipped at random reads the same", has(r, "4w4c") and r["rows_exact"] >= 16, f"{r['rows_exact']} {r['grids']}")
    r = run(toy_sudoku(2).cl)
    check("sudoku of order 4: rows, columns, boxes and digit words give 4 by 4 grids", r["grids"].split(";")[0] == "4w4c" and r["grids3"] >= 12, f"{r['grids3']} {r['grids']}")
    r, cen = analyse(toy_sudoku(3).cl, 20_000_000)
    n99 = sum(1 for k, col in cen.grids.items() if len(k) == 9 and grid_shape(col) == (9, 0))
    check("sudoku of order 9: the tightest grids are 9 by 9, the 27 units and more", r["grids"].split(";")[0] == "9w9c" and n99 >= 27, f"{n99} of {r['grids3']}: {r['grids']}")
    # every grid's columns are cliques and hold one literal per row
    bad = []
    for name, cl in (("sudoku 9", toy_sudoku(3).cl), ("latin 5 ladder", toy_latin(5, "ladder").cl), ("flipped", flipped(toy_latin(4, "commander"), 5)), ("colouring", toy_colouring(list(itertools.combinations(range(6), 2)), 6, 5).cl)):
        _, cen = analyse(cl, 20_000_000)
        for k, col in cen.grids.items():
            bad += [f"{name}: {x}" for x in grid_errors(cen, k, col, cl)]
    check("every grid on four toys holds: columns exclude pairwise, one literal per row", not bad, bad[:3])
    r = run(toy_php(5, 4).cl)
    check("pigeonhole 5 into 4 with no at-most-one in a row: one grid, a violation", has(r, "5w4c") and r["hall_violations"] == 1, f"{r['grids']} {r['hall_violations']}")
    check("  the rows are not exact-one", r["rows_exact"] == 0 and r["rows_differ"] == 5, f"{r['rows_exact']} {r['rows_differ']}")
    r = run(toy_php(4, 4).cl)
    check("pigeonhole 4 into 4: no violation, no removal", has(r, "4w4c") and r["hall_violations"] == 0 and r["hall_removed"] == 0, f"{r['grids']} {r['hall_violations']} {r['hall_removed']}")
    e = toy_php(4, 4, row_amo=True)
    e.cl.append([1])
    r = run(e.cl)
    check("one row fixed by a unit: three rows of three values left", has(r, "3w3c"), r["grids"])
    e = toy_php(5, 5, row_amo=True)
    for p in range(3):
        e.cl += [[-(p * 5 + 4)], [-(p * 5 + 5)]]
    r = run(e.cl)
    check("a Hall set of three rows on three values removes those values from the other two", r["hall_removed"] == 6, f"{r['hall_removed']} {r['grids']}")
    k5 = list(itertools.combinations(range(5), 2))
    r = run(toy_colouring(k5, 5, 4).cl)
    check("colouring a 5-clique with 4 colours: a grid and a violation", has(r, "5w4c") and r["hall_violations"] == 1, f"{r['grids']} {r['hall_violations']}")
    c5 = [(i, (i + 1) % 5) for i in range(5)]
    r = run(toy_colouring(c5, 5, 3).cl)
    check("colouring a 5-cycle: the edges differ, no grid of three", r["rows_differ"] == 5 and r["grids3"] == 0, f"{r['rows_differ']} {r['grids3']}")
    check("  its rows are plain at-least-ones", r["rows_domain"] == 5 and r["rows_exact"] == 0, f"{r['rows_domain']} {r['rows_exact']}")
    edges = list(itertools.combinations(range(4), 2)) + list(itertools.combinations([3, 4, 5, 6], 2))
    r = run(toy_colouring(edges, 7, 5).cl)
    check("two 4-cliques sharing a vertex are two grids", r["grids3"] == 2 and r["max_n"] == 4, f"{r['grids3']} {r['grids']}")
    e = toy_php(3, 3)
    e.cl[0] = e.cl[0] + [e.var()]
    r = run(e.cl)
    check("a value one row has alone reads as private", has(r, "3w3c+1p"), r["grids"])
    # a dense table constraint is not a not-equal
    e = Enc()
    a = [e.var() for _ in range(8)]
    b = [e.var() for _ in range(8)]
    d = [e.var() for _ in range(8)]
    e.alo(a)
    e.alo(b)
    e.alo(d)
    for x, y in itertools.product(range(8), range(8)):
        if (x + y) % 2 == 0:
            e.cl.append([-a[x], -b[y]])
            e.cl.append([-b[x], -d[y]])
            e.cl.append([-a[x], -d[y]])
    r = run(e.cl)
    check("a dense table constraint, one literal against four, does not differ", r["rows_differ"] == 0 and r["grids3"] == 0, f"{r['rows_differ']} {r['grids']}")
    # side exclusions next to the columns: n-queens
    e = Enc()
    q = [[e.var() for _ in range(8)] for _ in range(8)]
    for i in range(8):
        e.alo(q[i])
        e.amo_pairwise(q[i])
    for j in range(8):
        e.amo_pairwise([q[i][j] for i in range(8)])
    for k in range(-7, 8):
        e.amo_pairwise([q[i][i - k] for i in range(8) if 0 <= i - k < 8])
    for k in range(0, 15):
        e.amo_pairwise([q[i][k - i] for i in range(8) if 0 <= k - i < 8])
    r = run(e.cl)
    check("n-queens: the rows over the columns read as a grid, diagonals beside them", has(r, "8w8c"), r["grids"])
    r = run(toy_circuit(6).cl)
    check("an and-chain's clauses are read and are not domains", r["rows_read"] > 0 and r["rows_domain"] == 0 and r["grids3"] == 0, f"{r['rows_read']} {r['rows_domain']} {r['grids']}")
    # many or-gates over the same inputs: not domains, no grid
    e = Enc()
    ins = [e.var() for _ in range(8)]
    for i in range(0, 6):
        o = e.var()
        e.cl += [[o, -ins[i]], [o, -ins[i + 1]], [-o, ins[i], ins[i + 1]]]
    r = run(e.cl)
    check("or-gates over shared inputs are not domains", r["rows_domain"] == 0 and r["grids3"] == 0, f"{r['rows_domain']} {r['grids']}")
    # a guarded exact-one is a domain, and grids through the guard's private value
    e = Enc()
    x = [[e.var() for _ in range(3)] for _ in range(3)]
    for i in range(3):
        g = e.var()
        e.cl.append([g] + x[i])
        e.amo_pairwise(x[i])
    for j in range(3):
        e.amo_pairwise([x[i][j] for i in range(3)])
    r = run(e.cl)
    check("guarded exact-ones are domains with the guard as a private value", has(r, "3w3c+3p") and r["rows_domain"] == 3 and r["rows_exact"] == 0, f"{r['rows_domain']} {r['grids']}")
    r = run(toy_xor(6).cl)
    check("an xor chain has no exact-one row and no grid", r["rows_exact"] == 0 and r["grids3"] == 0, f"{r['rows_exact']} {r['grids']}")
    # a split at-least-one reads as a row over its leaves
    e = Enc()
    leaves = [e.var() for _ in range(12)]
    guards = [e.var() for _ in range(3)]
    e.alo(guards)
    for g, i in zip(guards, range(0, 12, 4)):
        e.cl.append([-g] + leaves[i:i + 4])
    e.amo_ladder(leaves)
    r = run(e.cl)
    check("a split at-least-one reads as one exact-one row of 12 leaves", "12x1" in r["exact_sizes"], r["exact_sizes"])
    e = Enc()
    x = [e.var() for _ in range(5)]
    e.alo(x)
    e.amo_pairwise(x[:3])
    r = run(e.cl)
    check("an at-least-one with only part of it exclusive is not exact-one", r["rows_exact"] == 0, r["rows_exact"])
    e = Enc()
    v = [e.var() for _ in range(5)]
    e.amo_pairwise(v)
    r = run(e.cl)
    check("an at-most-one with no row over it counts as at-most-one only", (r["amo_cliques"], r["amo_max"], r["amo_only"]) == (1, 5, 1), f"{r['amo_cliques']} {r['amo_max']} {r['amo_only']}")
    r = run(toy_php(3, 5, row_amo=True).cl)
    check("an exact-one's clique has its row", (r["amo_cliques"], r["amo_only"]) == (3, 0), f"{r['amo_cliques']} {r['amo_only']}")
    r = run([[1, 2, 3, 4], [-1, -2], [-1, -3], [-2, -3]])
    check("a clause holding more than the clique makes none of it true", (r["amo_cliques"], r["amo_only"]) == (1, 1), f"{r['amo_cliques']} {r['amo_only']}")
    r = run([[-1, -2], [-1, -3], [-2, -3], [1, 2]])
    check("a two-literal clause inside the clique makes one of it true", (r["amo_cliques"], r["amo_only"]) == (1, 0), f"{r['amo_cliques']} {r['amo_only']}")
    # a hub stops one literal, not the cell
    e = Enc()
    hub = e.var()
    far = [e.var() for _ in range(HUB + 10)]
    for y in far:
        e.cl.append([-hub, y])
    rows = [[e.var() for _ in range(3)] for _ in range(3)]
    for r0 in rows:
        e.alo(r0)
    for j in range(3):
        e.amo_pairwise([rows[i][j] for i in range(3)])
    e.alo([hub, rows[0][0], rows[1][1]])
    r = run(e.cl)
    check("a hub literal does not stop the read", r["hubs"] >= 1 and has(r, "3w3c"), f"hubs {r['hubs']} {r['grids']}")
    # a grid whose exclusions all show from one side only, by a fresh pair of
    # variables per exclusion: the same as the plain encoding
    e = Enc()
    x4 = [[e.var() for _ in range(3)] for _ in range(4)]
    for row4 in x4:
        e.alo(row4)
        for v4 in row4:
            e.cl.append([-v4, e.var()])    # every literal propagates something
    for h4 in range(3):
        for p4, q4 in itertools.combinations(range(4), 2):
            a4, b4 = e.var(), e.var()
            e.cl += [[-x4[p4][h4], a4], [-x4[p4][h4], b4], [-a4, -b4, -x4[q4][h4]]]
    r = run(e.cl)
    check("pigeonhole 4 into 3 with one-way exclusions: the grid and its violation", has(r, "4w3c") and r["hall_violations"] == 1, f"{r['grids']} {r['hall_violations']}")
    # a pair that shows only from one side still counts
    e = Enc()
    t = [e.var() for _ in range(9)]
    aux = e.var()
    aux2 = e.var()
    e.alo(t[0:3])
    e.alo(t[3:6])
    e.alo(t[6:9])
    e.amo_pairwise([t[0], t[6]])
    e.amo_pairwise([t[1], t[7]])
    e.amo_pairwise([t[2], t[8]])
    # t3 forces aux and aux2, and aux, aux2, t0 cannot all hold: t3 excludes t0, t0 alone derives nothing about t3
    e.cl += [[-t[3], aux], [-t[3], aux2], [-aux, -aux2, -t[0]], [-t[4], -t[1]], [-t[4], -t[7]], [-t[5], -t[2]], [-t[5], -t[8]]]
    e.cl += [[-t[3], -t[6]]]
    r = run(e.cl)
    check("an exclusion that shows from one side only still joins the column", has(r, "3w3c"), r["grids"])
    r = run(toy_sudoku(3).cl, budget=2_000)
    check("a small budget stops and says so", r["stopped"] == "budget" and r["rows_read"] < r["rows_active"], f"{r['stopped']} {r['rows_read']}")
    # a row the classifying quota interrupts is not lost: with room left every row is read
    e3 = toy_sudoku(2)
    cen3 = Census(Formula(e3.cl), 10 ** 9, random.Random(1))
    by_class = defaultdict(list)
    for ri, row3 in enumerate(cen3.rows):
        by_class[cen3.row_class(row3)].append(ri)
    cen3.queues = [deque(rs) for rs in by_class.values()]
    for lim in (cen3.prop.used, cen3.prop.used + 3, cen3.prop.used + 40):
        cen3.classify_some(lim)
    cen3.classify_some(10 ** 9)
    check("a row the classifying quota interrupts is read in a later turn", cen3.rows_read == len(cen3.rows) and not cen3.queues, f"{cen3.rows_read} of {len(cen3.rows)}")
    r = run(toy_php(30, 29).cl, budget=600_000)
    check("a whole grid is found once, within a tight budget", has(r, "30w29c") and r["hall_violations"] == 1 and r["grids3"] == 1 and r["stopped"] == "", f"{r['grids']} {r['grids3']} {r['hall_violations']} {r['visits']} {r['rows_cut']}")
    # gates around a domain do not get in its way
    e = toy_php(5, 4)
    ins = list(range(1, 21))
    for i in range(0, 19):
        o = e.var()
        e.cl += [[o, -ins[i]], [o, -ins[i + 1]], [-o, ins[i], ins[i + 1]]]
    r = run(e.cl)
    check("or-gates over a grid's literals leave the grid as it is", has(r, "5w4c") and r["hall_violations"] == 1, f"{r['grids']} {r['rows_domain']} {r['rows_read']}")
    # the reader
    tmp = Path(args.tmp)
    tmp.mkdir(parents=True, exist_ok=True)
    p = tmp / "read.cnf"
    p.write_text("c a comment\np cnf 4 3\n1 2\n3 0 -1 -2 0 c at the end\n2 2 4 0\n")
    nv, cl, tr = read_clauses(p, 100)
    check("the reader splits at zeros, keeps comments out, removes repeats", cl == [[1, 2, 3], [-1, -2], [2, 4]] and nv == 4 and not tr, f"{cl} {nv}")
    nv, cl, tr = read_clauses(p, 1)
    check("the reader stops at its limit and says so", len(cl) == 1 and tr, f"{cl} {tr}")
    check("the header's counts", header_counts(p) == (4, 3), str(header_counts(p)))
    p2 = tmp / "empty.cnf"
    p2.write_text("p cnf 1 1\n0\n")
    nv, cl, tr = read_clauses(p2, 100)
    check("an empty clause is kept and is a root conflict", cl == [[]] and analyse(cl)[0]["root_conflict"] == 1, f"{cl}")
    p2.write_text("p cnf 2 2\n1 c x\n2 0 0\n")
    nv, cl, tr = read_clauses(p2, 100)
    check("  on the gate's reader too", cl == [[1, 2], []] and analyse(cl)[0]["root_conflict"] == 1, f"{cl}")
    # a row retried after its share ran out counts once
    r0, cen0 = analyse(toy_sudoku(3).cl, 50_000)
    check("a row retried after its share ran out counts once", r0["rows_differ"] == len(cen0.counted_differ), f"{r0['rows_differ']} {len(cen0.counted_differ)}")
    # a low budget: the estimates stay inside the population, and rows_differ inside rows_read
    r = run(toy_colouring(list(itertools.combinations(range(18), 2)), 18, 5).cl, budget=10_000)
    check("under a low budget the estimates stay within the rows", r["est_differ"] <= r["rows_active"] and r["rows_differ"] <= r["rows_read"], f"read {r['rows_read']} differ {r['rows_differ']} est {r['est_differ']} of {r['rows_active']}")
    # snapshots spread over the file
    p3 = tmp / "snaps.txt"
    p3.write_text("".join(f"{i + 1} -{i + 2}\n" for i in range(10)) + "\n")
    got3 = read_snapshots(p3, 3)
    check("snapshots are chosen evenly and parsed", len(got3) == 3 and got3[0] == [1, -2] and got3[1][0] == 4, got3)
    check("  fewer lines than asked for: all of them", len(read_snapshots(p3, 50)) == 10, len(read_snapshots(p3, 50)))
    # two seeds or no status
    runs = tmp / "runs"
    (runs / "cur0").mkdir(parents=True, exist_ok=True)
    (runs / "cur1").mkdir(parents=True, exist_ok=True)
    (runs / "cur0" / "results.tsv").write_text("instance\tresult\na\tTIMEOUT\nb\tTIMEOUT\nc\tSATISFIABLE\n")
    (runs / "cur1" / "results.tsv").write_text("instance\tresult\nb\tTIMEOUT\nc\tTIMEOUT\n")
    check("a stem needs a result on both seeds to have a status", run_status(runs) == {"b": "unsolved", "c": "one-seed"}, run_status(runs))
    # the snapshot reasoning
    e = toy_php(4, 4, row_amo=True)
    cen = Census(Formula(e.cl), 5_000_000, random.Random(1))
    cen.run()
    key = max(cen.grids, key=len)
    rows_g = [cen.rows[ri] for ri in key]
    col = cen.grids[key]
    got = hall_on_snapshot(rows_g, col, {-3, -4, -7, -8})
    check("on a trail, two rows on two values remove those values from the others", got == (4, False, 4, 0), got)
    got = hall_on_snapshot(rows_g, col, {-2, -3, -4, -6, -7, -8})
    check("on a trail, two rows on one value are a violation", got[1], got)
    hs = hall_violation_set(rows_g, col, {-2, -3, -4, -6, -7, -8})
    check("the violation's Hall set is those two rows and its reason their false literals", hs is not None and len(hs[0]) == 2 and sorted(hs[2]) == [2, 3, 4, 6, 7, 8], hs and (len(hs[0]), hs[2]))
    got = hall_on_snapshot(rows_g, col, {1, -2, -3, -4, -5, -9, -13})
    check("on a trail, a row with a true literal is done and its column taken", got == (3, False, 0, 0), got)
    got = hall_on_snapshot(rows_g, col, {1})
    check("a column left open by propagation is reported as a gap", got[0] == 3 and got[3] == 3, got)
    # a violation resting on a column a true literal holds: its clause is false now
    true_now = {1, -6, -7, -8, -10, -11, -12}
    hs = hall_violation_set(rows_g, col, true_now)
    check("a violation's clause holds the true literal's negation, not the open literal", hs is not None and all(-l in true_now for l in hs[2]) and -1 in hs[2] and 5 not in hs[2], hs and hs[2])
    print("selftest:", "all passed" if ok else "FAILED")
    return 0 if ok else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--out", required=True)
    c.add_argument("--files", nargs="*")
    c.add_argument("--workers", type=int, default=16)
    c.add_argument("--budget", type=int, default=BUDGET)
    c.set_defaults(func=cmd_census)
    o = sub.add_parser("one")
    o.add_argument("file")
    o.add_argument("--budget", type=int, default=BUDGET)
    o.set_defaults(func=cmd_one)
    r = sub.add_parser("report")
    r.add_argument("tsv")
    r.add_argument("--runs", default=str(RUN_OF_RECORD))
    r.set_defaults(func=cmd_report)
    t = sub.add_parser("trail")
    t.add_argument("file")
    t.add_argument("snapshots")
    t.add_argument("--budget", type=int, default=BUDGET)
    t.add_argument("--snapshots-limit", type=int, default=200)
    t.add_argument("--grids", type=int, default=300)
    t.add_argument("--check", type=int, default=0, help="decide this many sampled violations with the solver")
    t.add_argument("--check-time", type=int, default=120)
    t.add_argument("--solver", default=str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"))
    t.add_argument("--tmp", default="/tmp/alldiff_census_checks")
    t.set_defaults(func=cmd_trail)
    g = sub.add_parser("grids")
    g.add_argument("file")
    g.add_argument("--out", required=True)
    g.add_argument("--budget", type=int, default=BUDGET)
    g.add_argument("--min-rows", type=int, default=3)
    g.set_defaults(func=cmd_grids)
    s = sub.add_parser("selftest")
    s.add_argument("--tmp", default="/tmp/alldiff_census_selftest")
    s.set_defaults(func=cmd_selftest)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
