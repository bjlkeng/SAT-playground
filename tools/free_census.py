#!/usr/bin/env python3
"""Free-choice census (bead SAT-playground-d96.2.1, G.8.1).

Which cells are a few multi-valued choices plus variables that the choices
fix, and how few.  After unit propagation every variable that is left gets
one role:

  choice   a member of a group.  An exactly-one group is a clause whose
           literals exclude each other, by a binary clause for every pair
           (pairwise) or through chains of binary clauses (ladder,
           sequential, commander, product and binary encodings: x excludes
           y when the binary implications from x reach -y).  An at-most-one
           group comes from a clause (g | m1 | ... | mk), g the only literal
           of its sign, whose m exclude each other: the m take at most one
           value and "none" is a value too (g needing one of them stays a
           constraint)
  def      a two-way definition.  A clause (x | m1 | ... | mk) with every
           binary (-x | -mi) makes -x the OR of the mi, of any width (the
           AND form is the same clause read with the signs of x and the mi
           flipped); or a small gate (up to MAX_GATE_SUPPORT inputs: XOR,
           mux, majority, ...) whose clauses fix it for every input value
  helper   a one-way helper: every clause holding a literal y of it has all
           its other variables computed already, so y can be set true
           exactly when one of those clauses needs it.  That keeps every
           clause with y true and sets -y as often as it can be; any model
           stays a model when a helper is reset this way.  Plaisted-
           Greenbaum gates and the lockchart helpers (h -> key, h -> -lock)
           read this way
  free     anything else: a variable no role fits, or one freed to break a
           cycle of definitions

Roles are settled forward from the groups: a definition or helper is taken
once all of its inputs are settled (two-way definitions before helpers), so
the reading is acyclic by construction; when nothing is ready, the
unsettled variable that the most waiting definitions read is freed (a
cycle break).  What a search over the choices has to satisfy is the
constraint clauses: those that no role makes true.  A clause belongs to the
last-settled variable in it; it is covered when that variable's role makes
it true (its definition's own clauses, the clauses with the helper's
literal, or a clause over one group's members that exactly-one implies).

Every reading is checked by simulation: 64 random choice assignments (one
member per group, random free variables), everything else computed in
order.  A covered clause that comes out false is a bug in the reading
(column sim_bad, must be 0); the mean number of constraint clauses left
false is the cost a random choice starts from (sim_false).

With --order (bead SAT-playground-d96.10) a number stored as sorted bits is a
choice too.  An order group is a run of bits b0, b1, ... with a clause
(b(k+1) -> bk) for every step; its values are the prefixes (the first v bits
on), one more than it has bits.  Three more steps make such a reading of
use, each general:
  what reads the numbers   an encoder can tie a test to a number in one-way
           pieces (Z3 writes "x = 2" as a count over x's bits, and a
           placement of the pyramid as a test on three labels in a dozen
           clauses), so no clause shape gives the test's definition.  Each
           number is set to each value and unit propagation is asked what
           it fixes (order_links); what it finds is added as the clauses
           of a two-way definition over the bits: a variable equal to a
           bit, a test "x from lo to hi", a test on several numbers at once
  always-true clauses   a clause whose variables the numbers alone fix,
           true for every joint value of the few numbers it depends on (all
           of them tried), is covered (cover_by_order)
  counts   an exactly-one group over variables that the numbers compute is
           one constraint, "exactly one of these is true"; the clauses of
           its encoding are covered by it (cover_by_count)
The reading without --order is unchanged, and a formula with no order group
reads the same with it.  Columns added: order_groups, order_sizes,
order_inputs, order_derived_groups, early (variables the order groups alone
fix), order_links (what propagation found), cov_order, counts, cov_count.

Columns: vars, clauses (read), truncated, fixed (variables set by units),
present (variables left), groups, group_vars, group_sizes (size x count),
chain_groups (groups whose exclusions run through other variables),
overlap_groups (exactly-one clauses not taken because they share a variable
with a taken group), defs (OR/AND definitions), wide_defs (more than six
inputs), def_inputs_max, gates, helpers, free, cycle_breaks, bits (log2 of
the number of choice assignments: log2 of each group's size plus one per
free variable), choice_share ((groups + free) / present), the clause counts
by cover (cov_group, cov_def, cov_gate, cov_helper), constraints,
constraint_share (of the clauses left after units), con_choice (constraint
clauses over choices and free variables only), con_lens (length x count),
sim_bad, sim_false, sim_false_share, seconds, error.

Usage:
  python3 tools/free_census.py census --out TABLE.tsv [--stems a,b] [--files F ...] [--workers 16] [--order]
  python3 tools/free_census.py report TABLE.tsv [--run RUN_DIR]
  python3 tools/free_census.py one FILE.cnf[.xz] [--order]
  python3 tools/free_census.py chains --out TABLE.tsv [--stems a,b] [--files F ...] [--workers 16]
                                 the order groups of every cell and nothing else of
                                 the reading (which cells hold such numbers)
  python3 tools/free_census.py selftest

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import lzma
import math
import os
import random
import signal
import sys
import time
from array import array
from collections import Counter, defaultdict, deque
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
FAMILIES = ROOT / "benchmarks" / "rl" / "families.tsv"
RUN_OF_RECORD = ROOT / "log" / "abtest-struct-points-400-2026-11-05-16-07-19"

MAX_CLAUSES = 50_000_000   # cells with up to this many clauses (by the header) are read whole: the oddball tto cells hold 19-47M
PREFIX_CLAUSES = 5_000_000 # of a bigger cell only this prefix is read (eight 2025 cells, 63-316M clauses)
MAX_HELPER_IN = 64         # a helper's literal sits in clauses with at most this many other literals in all
MAX_GATE_SUPPORT = 4       # small gates: truth tables over up to four inputs
MAX_GATE_SHORT = 32        # a variable in more short clauses than this is not tried as a small gate
MAX_GATE_STARTS = 6        # start clauses tried per variable
CHAIN_BUDGET = 256         # literals visited per exclusion walk, beside 8 per group member (16 times as many binaries)
MAX_CHAIN_GROUP = 2_048    # longest clause tried as a group whose exclusions run through other variables
QUICK_BUDGET = 128         # literals per walk in the first-pair test that screens group candidates
GROUP_SECONDS = 900        # wall time for the exclusion tests per cell (then the rest are skipped)
SIM_PATTERNS = 64          # random choice assignments in the check
MIN_ORDER = 3              # an order group has at least this many members (a number of four values or more)
ORDER_LINKS = 256          # an input with more one-way links than this to other inputs is not looked at (bounds the work)
ORDER_READ_CLAUSES = 5_000_000  # what reads the order groups is asked of propagation in a formula of up to this many clauses
ORDER_COVER_PATTERNS = 20_000   # a clause over up to this many joint values of its order groups is tried on all of them (7^5 = 16,807)
CELL_SECONDS = 3_600       # wall budget per cell
MASK = (1 << SIM_PATTERNS) - 1

# roles of a variable
ABSENT, FIXED, CHOICE, FREE, DEF, GATE, HELPER = range(7)
ROLE_NAMES = ("absent", "fixed", "choice", "free", "def", "gate", "helper")
# covers of a clause
C_NONE, C_GROUP, C_DEF, C_GATE, C_HELPER, C_ORDER, C_COUNT = range(7)


def _li(lit):
    """Index of a literal: 2v for v, 2v + 1 for -v."""
    return 2 * lit if lit > 0 else 1 - 2 * lit


# ---------------------------------------------------------------------------
# reading a CNF into flat arrays
# ---------------------------------------------------------------------------


class CNF:
    """Clause i is lits[starts[i]:starts[i + 1]]."""

    def __init__(self, nv, lits, starts, empty=0):
        self.nv = nv
        self.lits = lits
        self.starts = starts
        self.m = len(starts) - 1
        self.empty = empty            # empty clauses (kept as clauses of no literal): the formula is refuted
        self.occ_start = self.occ = None
        self.bin_start = self.bin_part = self.bin_cid = None

    @classmethod
    def from_clauses(cls, nv, clauses):
        lits, starts = array("i"), array("q", [0])
        empty = 0
        for c in clauses:
            lits.extend(c)
            starts.append(len(lits))
            empty += not c
        return cls(max([nv] + [abs(x) for x in lits]), lits, starts, empty)

    def clause(self, i):
        return self.lits[self.starts[i]:self.starts[i + 1]]

    def size(self, i):
        return self.starts[i + 1] - self.starts[i]

    def build_occ(self):
        """Occurrence lists by literal index and the binary-clause partners:
        bin_part[bin_start[li]:bin_start[li + 1]] are the literals y with
        (lit | y) a binary clause, bin_cid the clause of each."""
        nv, lits, starts, m = self.nv, self.lits, self.starts, self.m
        n_idx = 2 * nv + 2
        os_ = array("q", bytes(8 * (n_idx + 1)))
        for lit in lits:                     # a plain count: a Counter of 25M literals would cost gigabytes
            os_[(2 * lit if lit > 0 else 1 - 2 * lit) + 1] += 1
        for i in range(1, n_idx + 1):
            os_[i] += os_[i - 1]
        occ = array("i", bytes(4 * len(lits)))
        fill = array("q", os_)
        bcount = array("q", bytes(8 * (n_idx + 1)))
        for c in range(m):
            s0, s1 = starts[c], starts[c + 1]
            for lit in lits[s0:s1]:
                li = 2 * lit if lit > 0 else 1 - 2 * lit
                occ[fill[li]] = c
                fill[li] += 1
            if s1 - s0 == 2:
                a, b = lits[s0], lits[s0 + 1]
                bcount[(2 * a if a > 0 else 1 - 2 * a) + 1] += 1
                bcount[(2 * b if b > 0 else 1 - 2 * b) + 1] += 1
        for i in range(1, n_idx + 1):
            bcount[i] += bcount[i - 1]
        nb = bcount[n_idx]
        bpart = array("i", bytes(4 * nb))
        bcid = array("i", bytes(4 * nb))
        bfill = array("q", bcount)
        for c in range(m):
            s0 = starts[c]
            if starts[c + 1] - s0 != 2:
                continue
            a, b = lits[s0], lits[s0 + 1]
            la = 2 * a if a > 0 else 1 - 2 * a
            lb = 2 * b if b > 0 else 1 - 2 * b
            p = bfill[la]
            bpart[p] = b
            bcid[p] = c
            bfill[la] = p + 1
            p = bfill[lb]
            bpart[p] = a
            bcid[p] = c
            bfill[lb] = p + 1
        self.occ_start, self.occ = os_, occ
        self.bin_start, self.bin_part, self.bin_cid = bcount, bpart, bcid

    def occs(self, lit):
        li = 2 * lit if lit > 0 else 1 - 2 * lit
        return self.occ[self.occ_start[li]:self.occ_start[li + 1]]


def header_clauses(path):
    """The clause count of the 'p cnf' line (0 when there is none before
    the first clause)."""
    with open(path, "rb") as fh:
        xz = fh.read(6) == b"\xfd7zXZ\x00"
    opener = lzma.open if xz else open
    with opener(path, "rb") as f:
        for ln in f:
            s = ln.strip()
            if not s or s[:1] in (b"c", b"%"):
                continue
            if s[:1] == b"p":
                parts = s.split()
                return int(parts[3]) if len(parts) >= 4 else 0
            return 0
    return 0


def read_cnf(path, max_clauses=MAX_CLAUSES):
    """(CNF, truncated): at most max_clauses clauses, repeated literals
    dropped, tautologies skipped, empty clauses kept (CNF.empty counts
    them), comment and header lines skipped, and a line starting with '%'
    taken as the end of the formula (SATLIB files end in '%' and '0').
    xz or plain text, told apart by the first bytes."""
    with open(path, "rb") as fh:
        xz = fh.read(6) == b"\xfd7zXZ\x00"
    opener = lzma.open if xz else open
    toks = array("i")
    nv = 0
    nz = 0
    truncated = False
    tail = b""
    ended = False
    with opener(path, "rb") as f:
        while not ended:
            chunk = f.read(1 << 24)
            if chunk:
                buf = tail + chunk
                cut = buf.rfind(b"\n")
                if cut < 0:
                    tail = buf
                    continue
                body, tail = buf[:cut + 1], buf[cut + 1:]
            else:
                body, tail = tail, b""
            if body:
                if b"c" in body or b"p" in body or b"%" in body:
                    keep = []
                    for ln in body.split(b"\n"):
                        s = ln.strip()
                        if s[:1] == b"%":
                            ended = True
                            break
                        if not s or s[:1] == b"c":
                            continue
                        if s[:1] == b"p":
                            parts = s.split()
                            if len(parts) >= 4:
                                nv = max(nv, int(parts[2]))
                            continue
                        keep.append(s)
                    body = b"\n".join(keep)
                part = array("i", map(int, body.split()))
                z = part.count(0)
                if nz + z >= max_clauses:
                    p = -1
                    for _ in range(max_clauses - nz):
                        p = part.index(0, p + 1)
                    # anything left counts, a trailing empty clause ("0") too
                    truncated = len(part) > p + 1 or (not ended and (bool(tail.strip()) or bool(f.read(1 << 16).strip())))
                    del part[p + 1:]
                    toks.extend(part)
                    break
                toks.extend(part)
                nz += z
            if not chunk:
                break
    if toks and toks[-1] != 0:
        toks.append(0)
    lits, starts = array("i"), array("q", [0])
    pos, n = 0, len(toks)
    find = toks.index
    empty = 0
    while pos < n:
        end = find(0, pos)
        k = end - pos
        if k == 0:
            empty += 1
            starts.append(len(lits))
        elif k == 2:
            a, b = toks[pos], toks[pos + 1]
            if a != -b:
                lits.append(a)
                if b != a:
                    lits.append(b)
                starts.append(len(lits))
        elif k == 1:
            lits.append(toks[pos])
            starts.append(len(lits))
        elif k > 2:
            cl = toks[pos:end]
            s = set(cl)
            if not any(-x in s for x in s):
                if len(s) < k:
                    seen = set()
                    cl = [x for x in cl if not (x in seen or seen.add(x))]
                lits.extend(cl)
                starts.append(len(lits))
        pos = end + 1
    if lits:
        nv = max(nv, max(lits), -min(lits))
    return CNF(nv, lits, starts, empty), truncated


# ---------------------------------------------------------------------------
# unit propagation at the root
# ---------------------------------------------------------------------------


def unit_propagate(cnf):
    """(fixed, reduced CNF or None on a conflict, satisfied clause count).
    fixed maps a variable to its value.  When units are found, the given
    CNF's arrays are released: the reduced formula replaces it."""
    lits, starts, m = cnf.lits, cnf.starts, cnf.m
    if cnf.empty:
        return {}, None, 0
    queue = [lits[starts[i]] for i in range(m) if starts[i + 1] - starts[i] == 1]
    if not queue:
        return {}, cnf, 0
    if cnf.occ is None:
        cnf.build_occ()
    os_, occ = cnf.occ_start, cnf.occ
    val = bytearray(cnf.nv + 1)     # 0 unset, 1 true, 2 false
    sat = bytearray(m)
    nfalse = array("i", bytes(4 * m))  # false literals per clause: a clause is scanned once, when it turns unit
    qi = 0
    while qi < len(queue):
        lit = queue[qi]
        qi += 1
        v = abs(lit)
        want = 1 if lit > 0 else 2
        if val[v]:
            if val[v] != want:
                return {}, None, 0
            continue
        val[v] = want
        li = 2 * lit if lit > 0 else 1 - 2 * lit
        for c in occ[os_[li]:os_[li + 1]]:
            sat[c] = 1
        nli = li ^ 1
        for c in occ[os_[nli]:os_[nli + 1]]:
            if sat[c]:
                continue
            nf = nfalse[c] + 1
            nfalse[c] = nf
            k = starts[c + 1] - starts[c]
            if nf == k:
                return {}, None, 0
            if nf == k - 1:
                for x in lits[starts[c]:starts[c + 1]]:
                    vx = val[abs(x)]
                    if not vx:
                        queue.append(x)
                        break
                    if (vx == 1) == (x > 0):
                        sat[c] = 1
                        break
    fixed = {v: val[v] == 1 for v in range(1, cnf.nv + 1) if val[v]}
    nl, ns = array("i"), array("q", [0])
    for c in range(m):
        if sat[c]:
            continue
        for x in lits[starts[c]:starts[c + 1]]:
            if not val[abs(x)]:
                nl.append(x)
        ns.append(len(nl))
    # the original's arrays are released (the reduced formula replaces it)
    cnf.lits, cnf.starts = array("i"), array("q", [0])
    cnf.occ_start = cnf.occ = cnf.bin_start = cnf.bin_part = cnf.bin_cid = None
    return fixed, CNF(cnf.nv, nl, ns), sum(sat)


# ---------------------------------------------------------------------------
# the role reading
# ---------------------------------------------------------------------------


def free_order(items):
    """items: (variable, has a two-way definition, waiting candidates,
    occurrences).  The variables in an array with the one a dead lock frees
    first at the end (for pop): no two-way definition first, then the most
    waiting candidates, the most occurrences, the lowest number.  One
    unbounded int per variable while sorting, so any variable number (up to
    2^31, the literal width) and any count fits."""
    cap = (1 << 40) - 1
    keys = sorted((((tw << 40 | (cap - min(cap, w))) << 40 | (cap - min(cap, o))) << 32 | v for v, tw, w, o in items),
                  reverse=True)
    return array("q", (k & 0xFFFFFFFF for k in keys))


def definition_tt(v, clause_lists, support):
    """Truth table of v over `support` if the clauses (each holding v and
    otherwise only support variables) fix v for every support value with no
    value forbidden, else None (the resolvent test of netlist_census.py)."""
    idx = {u: i for i, u in enumerate(support)}
    pos, neg = [], []
    for cl in clause_lists:
        others, sign = [], 0
        for lit in cl:
            if abs(lit) == v:
                sign = 1 if lit > 0 else -1
            else:
                others.append((idx[abs(lit)], lit > 0))
        (pos if sign > 0 else neg).append(others)
    tt = 0
    for mt in range(1 << len(support)):
        pv = any(all(((mt >> i) & 1) != b for i, b in oth) for oth in pos)
        nv = any(all(((mt >> i) & 1) != b for i, b in oth) for oth in neg)
        if pv == nv:
            return None      # forbidden (both) or not determined (neither)
        if pv:
            tt |= 1 << mt
    return tt


class Reading:
    """Roles of one reduced formula; see the module docstring."""

    def __init__(self, cnf, deadline=None, order=False, order_min=MIN_ORDER, order_groups=None, exo_groups=()):
        self.cnf = cnf
        self.deadline = deadline
        self.want_order = order                # read numbers stored as sorted bits as choices too (find_order_groups)
        self.order_min = order_min
        self.given_order = order_groups        # the order groups, found before (read_roles): taken as they are
        self.links = Counter()                 # what order_links added to the formula (read_roles)
        self.no_walk = None                    # variables the exclusion walks do not pass through (order_chains)
        self.derived_exo = [(tuple(mem), -1) for mem in exo_groups]   # exactly-one groups over computed variables:
        #                                        (members, clause); given ones (found before, read_roles) and settle_order's
        self.counts = []                       # of them, the count constraints (cover_by_count): member literals
        self.count_of = {}                     # clause -> the count constraint that covers it
        nv = cnf.nv
        self.groups = []                       # member literals: at most one true, exactly one unless none_ok
        self.chain = []                        # per group: exclusions through other variables
        self.none_ok = []                      # per group: may all members be false (an at-most-one)
        self.ordered = []                      # per group: an order group (its values are the prefixes of its members)
        self.gid = array("i", [-1]) * (nv + 1)
        self.role = bytearray(nv + 1)
        self.pos = array("i", [-1]) * (nv + 1)
        self.order = array("i")
        self.acc = array("i", [-1]) * (nv + 1)  # accepted candidate per variable
        # candidates: output, kind, a (clause / gate index / literal), b (pivot literal)
        self.c_out, self.c_kind = array("i"), bytearray()
        self.c_a, self.c_b = array("q"), array("i")
        self.c_in_start, self.c_in = array("q", [0]), array("i")
        self.gates = []                        # (support, truth table, clause ids)
        self.cover = bytearray(cnf.m)
        self.stats = Counter()

    def _tick(self):
        if self.deadline is not None and time.time() > self.deadline:
            raise TimeoutError("wall")

    def _add_cand(self, v, kind, a, b, inputs):
        self.c_out.append(v)
        self.c_kind.append(kind)
        self.c_a.append(a)
        self.c_b.append(b)
        self.c_in.extend(inputs)
        self.c_in_start.append(len(self.c_in))

    # -- step 1: the pivot test -------------------------------------------
    def pivot_pass(self):
        """For every clause and literal x of it: is (-x | -m) a binary
        clause for every other literal m?  Then -x is the OR of the others."""
        cnf = self.cnf
        lits, starts = cnf.lits, cnf.starts
        os_, occ = cnf.occ_start, cnf.occ
        bs, bp = cnf.bin_start, cnf.bin_part
        piv_c, piv_x = array("i"), array("i")
        npass = array("i", bytes(4 * cnf.m))
        for v in range(1, cnf.nv + 1):
            if v & 0xFFFF == 0:
                self._tick()
            for x in (v, -v):
                li = 2 * x if x > 0 else 1 - 2 * x
                a, b = os_[li], os_[li + 1]
                if a == b:
                    continue
                nli = li ^ 1
                nb = bs[nli + 1] - bs[nli]
                if nb == 0:
                    continue
                part = None
                for c in occ[a:b]:
                    s0, s1 = starts[c], starts[c + 1]
                    if s1 - s0 - 1 > nb:
                        continue
                    if part is None:
                        part = set(bp[bs[nli]:bs[nli + 1]])
                    for y in lits[s0:s1]:
                        if y != x and -y not in part:
                            break
                    else:
                        piv_c.append(c)
                        piv_x.append(x)
                        npass[c] += 1
        return piv_c, piv_x, npass

    # -- step 2: exactly-one groups ---------------------------------------
    def _excludes_all(self, cl):
        """Does every literal of cl exclude every other one through the
        binary implications (x -> ... -> -y)?"""
        cnf = self.cnf
        bs, bp = cnf.bin_start, cnf.bin_part
        budget = CHAIN_BUDGET + 8 * len(cl)          # literals reached per walk
        edges = 16 * budget                          # binary clauses looked at per walk
        no_walk = self.no_walk
        for x in cl:
            targets = {-y for y in cl if y != x}
            seen = {x}
            stack = deque([x])                           # breadth first: the short chains come first
            looked = 0
            while stack and targets:
                y = stack.popleft()
                ny = -2 * y if y < 0 else 1 + 2 * y          # index of -y: the binaries (-y | z) are y -> z
                a, b = bs[ny], bs[ny + 1]
                looked += b - a
                if looked > edges:
                    return False
                for z in bp[a:b]:
                    if z in seen or (no_walk is not None and no_walk[abs(z)]):
                        continue
                    if -z in seen:
                        return False                         # x is a failed literal
                    seen.add(z)
                    stack.append(z)
                    targets.discard(z)
                if len(seen) > budget:
                    return False
            if targets:
                return False
        return True

    def _excludes(self, x, y, budget=QUICK_BUDGET):
        """Does x exclude y (x -> ... -> -y) within a small walk?"""
        cnf = self.cnf
        bs, bp = cnf.bin_start, cnf.bin_part
        seen = {x}
        stack = deque([x])
        looked = 0
        ny_target = -y
        no_walk = self.no_walk
        while stack:
            u = stack.popleft()
            nu = -2 * u if u < 0 else 1 + 2 * u
            a, b = bs[nu], bs[nu + 1]
            looked += b - a
            if looked > 16 * budget:
                return False
            for z in bp[a:b]:
                if z == ny_target:
                    return True
                if z not in seen and not (no_walk is not None and no_walk[abs(z)]):
                    seen.add(z)
                    stack.append(z)
            if len(seen) > budget:
                return False
        return False

    def find_groups(self, npass, piv_c, piv_x, computed_early=False):
        """The choices.  Two kinds of group:
          exactly-one   a clause whose literals all exclude each other
                        (pairwise, or through chains of binary clauses);
          at-most-one   a clause (g | m1 | ... | mk), g the only literal of
                        its sign, whose m exclude each other: the m take at
                        most one value, and "none" is a value too.  The
                        clause itself stays a constraint (g needs one of
                        them) unless it is g's definition.
        Groups are taken biggest first while they share no variable with a
        taken group, so one group over all the leaves of a commander
        encoding wins over its pieces.  A group with a member defined by
        other clauses (a gate output: an OR gate whose inputs happen to
        exclude each other reads as exactly-one) is not a choice:
        the member has a two-way definition of two or more inputs from a
        clause other than the group's that is not itself a group (an
        equivalence, such as a lock only one key opens, does not count, nor
        does a commander's definition by the group above it).  A literal
        set that passes the cheap first-pair screen is tested once.
        With computed_early (settle_order, after its first stage) a group
        with a member that is settled already is not a choice either."""
        cnf = self.cnf
        starts, lits = cnf.starts, cnf.lits
        cands = self._group_candidates(npass)
        gid = self.gid
        is_group = {c for _m, c, _ch, _n in cands}
        members = {abs(x) for mem, _c, _ch, _n in cands for x in mem}
        return self._take_groups(cands, is_group, members, piv_c, piv_x, computed_early)

    def _group_candidates(self, npass):
        """The clauses that could be groups: (members, clause, chained, none allowed)."""
        cnf = self.cnf
        starts, lits = cnf.starts, cnf.lits
        bs = cnf.bin_start
        cands = []                 # (members, clause, chained, none allowed)
        tested = {}
        t_end = time.time() + GROUP_SECONDS

        def has_impl(cl):
            for x in cl:
                li = 2 * x if x > 0 else 1 - 2 * x
                if bs[(li ^ 1) + 1] == bs[li ^ 1]:
                    return False
            return True

        def excl(mem):
            if not (has_impl(mem) and self._excludes(mem[0], mem[1])):
                return False                     # the cheap screen: not cached (circuits have millions)
            key = frozenset(mem)
            r = tested.get(key)
            if r is None:
                if time.time() > t_end:
                    self.stats["group_tests_cut"] += 1
                    return False
                r = self._excludes_all(mem)
                tested[key] = r
            return r

        for c in range(cnf.m):
            k = starts[c + 1] - starts[c]
            if k < 3:
                continue
            if npass[c] == k:
                cands.append((tuple(lits[starts[c]:starts[c + 1]]), c, False, False))
                continue
            if c & 0xFFF == 0:
                self._tick()
            cl = lits[starts[c]:starts[c + 1]]
            npos = sum(1 for x in cl if x > 0)
            if k >= 4 and (npos == 1 or npos == k - 1) and k - 1 <= MAX_CHAIN_GROUP:
                gsign = npos == 1                  # the guard is the one positive literal
                mem = [x for x in cl if (x > 0) != gsign]
                if excl(mem):
                    cands.append((tuple(mem), c, True, True))
                continue
            if npass[c] or k > MAX_CHAIN_GROUP:
                continue            # a definition's clause (or too wide): not tried as chained exactly-one
            if excl(list(cl)):
                cands.append((tuple(cl), c, True, False))
        return cands

    def _take_groups(self, cands, is_group, members, piv_c, piv_x, computed_early):
        """find_groups' choice among the candidates."""
        cnf = self.cnf
        starts, lits = cnf.starts, cnf.lits
        gid = self.gid
        def_clauses = defaultdict(list)     # the members' pivot definitions with two or more inputs
        for c, x in zip(piv_c, piv_x):
            if abs(x) in members and starts[c + 1] - starts[c] >= 3:
                def_clauses[abs(x)].append(c)
        gated = bytearray(cnf.nv + 1)
        for i in range(len(self.c_out)):
            if self.c_kind[i] == GATE:
                gated[self.c_out[i]] = 1

        def defined_elsewhere(v, c):
            # a definition from a clause that is itself a group (a commander:
            # the guard of one group is a member of the next) does not count
            return gated[v] or any(d != c and d not in is_group for d in def_clauses.get(v, ()))

        cands.sort(key=lambda t: (-len(t[0]), t[1]))   # bigger groups first
        pos = self.pos
        for mem, c, chained, none_ok in cands:
            if any(defined_elsewhere(abs(x), c) for x in mem):
                self.stats["derived_groups"] += 1
                if computed_early and not none_ok:
                    self.derived_exo.append((mem, c))
                continue
            if computed_early and any(pos[abs(x)] >= 0 and gid[abs(x)] < 0 for x in mem):
                # a member that the order groups alone fix (settle_order's first
                # stage): the group is a function of the numbers, not a choice
                self.stats["derived_groups"] += 1
                self.stats["order_derived_groups"] += 1
                if not none_ok:
                    self.derived_exo.append((mem, c))
                continue
            if any(gid[abs(x)] >= 0 for x in mem):
                self.stats["overlap_groups"] += 1
                continue
            g = len(self.groups)
            self.groups.append(mem)
            self.chain.append(chained)
            self.none_ok.append(none_ok)
            self.ordered.append(False)
            for x in mem:
                gid[abs(x)] = g

    # -- step 2a: order groups (only with order=True) ----------------------
    def _could_help(self, v, claimed):
        """helper_candidates' test for one variable: could a literal of v be
        a one-way helper (its clauses are small and no other variable's
        definition uses one of them)?"""
        cnf = self.cnf
        starts = cnf.starts
        os_, occ = cnf.occ_start, cnf.occ
        for y in (v, -v):
            ly = 2 * y if y > 0 else 1 - 2 * y
            w = 0
            for c in occ[os_[ly]:os_[ly + 1]]:
                if claimed[c] not in (-1, v):
                    break
                w += starts[c + 1] - starts[c] - 1
                if w > MAX_HELPER_IN:
                    break
            else:
                return True
        return False

    def find_order_groups(self, claimed):
        """Numbers stored as sorted bits.  An order group is a run of
        literals l0, l1, ..., l(k-1) with a two-literal clause (l(j+1) -> lj)
        for every step.  Its values are the k + 1 prefixes (the first v
        literals true, the rest false); by the chain's clauses every model
        is one of them, and a move of the search sets another prefix.

        The run is looked for among the inputs, the variables that nothing
        could compute.  Variables that two clauses make equal (x <-> y) are
        one class, and a class is an input when
          - no member has a two-way definition of two or more inputs that
            reads no member of the class (t = OR(b0, b1, b2) with t equal to
            b0 needs b0 itself: it reads the number, it does not compute it);
          - not every member could be a one-way helper (_could_help), or it
            stands beside an input it is tied to in a longer clause (below).
        A link runs from one input class to another when some clause says
        so one way; only direct links count (u -> w with no input x between
        them), and a step of a run is a link that is the only direct one
        out of its upper end and the only direct one into its lower end.
        (A bit of the pyramid has hundreds of placements hanging on it,
        "this placement needs the label above 0": no one of them is the
        next bit, so the run ends there.  Numbers that are compared bit by
        bit are not read this way.)  A run of fewer than order_min members
        is not read."""
        cnf = self.cnf
        nv = cnf.nv
        os_ = cnf.occ_start
        bs, bp = cnf.bin_start, cnf.bin_part
        cis = self.c_in_start
        # classes of equal variables: v is sgn * root
        parent = array("i", range(nv + 1))
        sgn = array("b", [1]) * (nv + 1)

        def find(v):
            r, sg = v, 1
            while parent[r] != r:
                sg *= sgn[r]
                r = parent[r]
            cur, cs = v, sg
            while parent[cur] != cur:
                nxt, ns = parent[cur], cs * sgn[cur]
                parent[cur], sgn[cur] = r, cs
                cur, cs = nxt, ns
            return r, sg

        wide = []
        for i in range(len(self.c_out)):
            if self.c_kind[i] == DEF and cis[i + 1] - cis[i] == 1:
                x = self.c_b[i]                         # the clause (x | m) and (-x | -m): x equals -m
                m = next(y for y in cnf.clause(self.c_a[i]) if y != x)
                rel = -1 if (x > 0) == (m > 0) else 1
                (ra, sa), (rb, sb) = find(abs(x)), find(abs(m))
                if ra != rb:
                    if ra > rb:
                        parent[ra], sgn[ra] = rb, sa * rel * sb
                    else:
                        parent[rb], sgn[rb] = ra, sa * rel * sb
            else:
                wide.append(i)
        defined = bytearray(nv + 1)                     # by class root: computed from outside the class
        for i in wide:
            k = find(self.c_out[i])[0]
            if not defined[k] and all(find(u)[0] != k for u in self.c_in[cis[i]:cis[i + 1]]):
                defined[k] = 1
        inp = bytearray(nv + 1)                         # by class root: an input
        soft = []                                       # the present variables of the other classes that nothing defines
        for v in range(1, nv + 1):
            if v & 0x3FFF == 0:
                self._tick()
            if os_[2 * v] == os_[2 * v + 2]:
                continue
            k = find(v)[0]
            if defined[k]:
                continue
            if not self._could_help(v, claimed):
                inp[k] = 1
            else:
                soft.append(v)
        # A class whose members could all be helpers is an input all the same when it sits
        # with an input in a clause of three or more literals and is tied to that input by
        # a two-literal clause too: a bit of a number that few clauses read (its neighbour
        # bit stands beside it in the tests "the value is a"), not a helper hanging on the
        # number's end (tied to the end bit by its one link and nothing else).  Taken in
        # rounds, since the neighbour may be such a bit itself.
        lits, starts, occ = cnf.lits, cnf.starts, cnf.occ
        soft = [v for v in soft if not inp[find(v)[0]]]
        while soft:
            took = []
            for v in soft:
                k = find(v)[0]
                if inp[k]:
                    continue
                tied = set()
                for x in (v, -v):
                    li = 2 * x if x > 0 else 1 - 2 * x
                    for y in bp[bs[li]:bs[li + 1]]:
                        ky = find(abs(y))[0]
                        if ky != k and inp[ky]:
                            tied.add(ky)
                if not tied:
                    continue
                seen = 0
                found = False
                for c in occ[os_[2 * v]:os_[2 * v + 2]]:
                    s0, s1 = starts[c], starts[c + 1]
                    if s1 - s0 < 3:
                        continue
                    seen += s1 - s0
                    if seen > 512:
                        break
                    if any(find(abs(z))[0] in tied for z in lits[s0:s1]):
                        found = True
                        break
                if found:
                    took.append(k)
            if not took:
                break
            for k in took:
                inp[k] = 1
            soft = [v for v in soft if not inp[find(v)[0]]]
        self.stats["order_inputs"] = sum(inp)

        def clit(x):
            r, sg = find(abs(x))
            return r * sg if x > 0 else -r * sg

        # one-way links between input classes: U -> W for a clause (-u | w); one clause of each is kept
        succ = {}
        via = {}
        for v in range(1, nv + 1):
            if os_[2 * v] == os_[2 * v + 2]:
                continue
            k = find(v)[0]
            if not inp[k]:
                continue
            for u in (v, -v):
                nu = -2 * u if u < 0 else 1 + 2 * u     # index of -u: the clauses (-u | y) are u -> y
                a, b = bs[nu], bs[nu + 1]
                if a == b:
                    continue
                cu = clit(u)
                for y in bp[a:b]:
                    ky = find(abs(y))[0]
                    if ky == k or not inp[ky]:
                        continue
                    cy = clit(y)
                    outs = succ.get(cu)
                    if outs is None:
                        outs = succ[cu] = set()
                    if cy not in outs:
                        outs.add(cy)
                        via[(cu, cy)] = (u, y)
        # direct links only: drop U -> W when U -> X -> W
        direct = {}
        for u, outs in succ.items():
            if len(outs) > ORDER_LINKS:
                continue                                 # not looked at: the work is its links times theirs
            far = set()
            for x in outs:
                sx = succ.get(x)
                if sx:
                    far |= sx
            d = outs - far
            if d:
                direct[u] = d
        into = defaultdict(int)
        for u, d in direct.items():
            for w in d:
                into[w] += 1
        for u, outs in succ.items():                    # the links that were not looked at count as ways in
            if len(outs) > ORDER_LINKS:
                for w in outs:
                    into[w] += 1
        down = {u: next(iter(d)) for u, d in direct.items() if len(d) == 1 and into[next(iter(d))] == 1}
        has_up = set(down.values())
        used = bytearray(nv + 1)
        gid = self.gid
        for top in sorted((u for u in down if u not in has_up), key=lambda u: (abs(u), -u)):
            path = [top]
            while path[-1] in down:
                path.append(down[path[-1]])
            if len(path) < self.order_min or any(used[abs(u)] for u in path):
                continue                                 # too short, or the mirror of a run that is taken
            # the member of each class: the literal the link from above arrives at (the top: where its link leaves)
            seq = [via[(path[j - 1], path[j])][1] for j in range(len(path) - 1, 0, -1)]
            seq.append(via[(path[0], path[1])][0])       # lowest first: seq[k + 1] -> seq[k]
            neg = sum(1 for x in seq if x < 0)
            if 2 * neg > len(seq) or (2 * neg == len(seq) and seq[0] < 0):
                seq = [-x for x in reversed(seq)]        # the mirror: the same number counted from the other end
            g = len(self.groups)
            self.groups.append(tuple(seq))
            self.chain.append(False)
            self.none_ok.append(False)
            self.ordered.append(True)
            for u in path:
                used[abs(u)] = 1
            for x in seq:
                gid[abs(x)] = g
        self.stats["order_groups"] = len(self.groups)

    # -- step 3: candidates -----------------------------------------------
    def two_way_candidates(self, piv_c, piv_x):
        """Every two-way definition found, for every variable: the OR/AND
        ones from the pivot test, and small gates for the variables the
        pivot test gives none.  Returns claimed: clause -> the variable whose
        definition uses it (-2 when two variables' do)."""
        cnf = self.cnf
        lits, starts = cnf.lits, cnf.starts
        os_ = cnf.occ_start
        bs, bp, bc = cnf.bin_start, cnf.bin_part, cnf.bin_cid
        claimed = array("i", [-1]) * cnf.m

        def claim(c, v):
            o = claimed[c]
            claimed[c] = v if o in (-1, v) else -2

        has_def = bytearray(cnf.nv + 1)
        for c, x in zip(piv_c, piv_x):
            v = abs(x)
            cl = lits[starts[c]:starts[c + 1]]
            self._add_cand(v, DEF, c, x, [abs(y) for y in cl if y != x])
            has_def[v] = 1
            claim(c, v)
            mem = set(cl)
            nx = -x
            ln = 2 * nx if nx > 0 else 1 - 2 * nx
            for p in range(bs[ln], bs[ln + 1]):
                if -bp[p] in mem and bp[p] != -x:
                    claim(bc[p], v)
        for v in range(1, cnf.nv + 1):
            if has_def[v] or os_[2 * v] == os_[2 * v + 2]:
                continue
            if v & 0x3FFF == 0:
                self._tick()
            g = self._gate_candidate(v)
            if g is not None:
                for c in self.gates[g][2]:
                    claim(c, v)
        return claimed

    def helper_candidates(self, claimed):
        """One-way helpers for the variables outside the groups: a literal
        y whose clauses hold at most MAX_HELPER_IN other literals in all,
        none of them a clause another variable's definition uses (an input
        of a gate is not computed back from the gate)."""
        cnf = self.cnf
        lits, starts = cnf.lits, cnf.starts
        os_, occ = cnf.occ_start, cnf.occ
        gid = self.gid
        for v in range(1, cnf.nv + 1):
            if gid[v] >= 0 or os_[2 * v] == os_[2 * v + 2]:
                continue
            if v & 0x3FFF == 0:
                self._tick()
            for y in (v, -v):
                ly = 2 * y if y > 0 else 1 - 2 * y
                w = 0
                ins = set()
                for c in occ[os_[ly]:os_[ly + 1]]:
                    if claimed[c] not in (-1, v):
                        break
                    s0, s1 = starts[c], starts[c + 1]
                    w += s1 - s0 - 1
                    if w > MAX_HELPER_IN:
                        break
                    for z in lits[s0:s1]:
                        if z != y:
                            ins.add(abs(z))
                else:
                    ins.discard(v)
                    self._add_cand(v, HELPER, y, 0, sorted(ins))

    def _gate_candidate(self, v):
        """A small gate for v (index into self.gates) or None."""
        cnf = self.cnf
        short = [c for c in list(cnf.occs(v)) + list(cnf.occs(-v)) if 2 <= cnf.size(c) <= MAX_GATE_SUPPORT + 1]
        if not short or len(short) > MAX_GATE_SHORT:
            return None
        vs = {c: frozenset(abs(x) for x in cnf.clause(c)) - {v} for c in short}
        tried = set()
        n_starts = 0
        for st in sorted((c for c in short if cnf.size(c) >= 3), key=cnf.size):
            support = vs[st]
            if support in tried:
                continue
            n_starts += 1
            if n_starts > MAX_GATE_STARTS:
                return None
            for _grow in range(3):
                tried.add(support)
                sel = [c for c in short if vs[c] <= support]
                sup = tuple(sorted(support))
                tt = definition_tt(v, [cnf.clause(c) for c in sel], sup)
                if tt is not None:
                    self.gates.append((sup, tt, tuple(sorted(sel))))
                    self._add_cand(v, GATE, len(self.gates) - 1, 0, sup)
                    return len(self.gates) - 1
                best = None
                for c in short:
                    oth = vs[c]
                    new = oth - support
                    if not new or not (oth & support) or len(support | oth) > MAX_GATE_SUPPORT:
                        continue
                    if best is None or len(new) < len(best - support):
                        best = oth
                if best is None:
                    break
                support = support | best
        return None

    # -- step 4: settle forward -------------------------------------------
    def settle(self):
        cnf = self.cnf
        nv = cnf.nv
        os_ = cnf.occ_start
        role, pos, order, acc, gid = self.role, self.pos, self.order, self.acc, self.gid
        present = bytearray(nv + 1)
        for v in range(1, nv + 1):
            if os_[2 * v] != os_[2 * v + 2]:
                present[v] = 1
        n_cand = len(self.c_out)
        has_cand = bytearray(nv + 1)
        two_way = bytearray(nv + 1)
        for v, k in zip(self.c_out, self.c_kind):
            has_cand[v] = 1
            if k != HELPER:
                two_way[v] = 1

        def put(v, r, ci):
            role[v] = r
            acc[v] = ci
            pos[v] = len(order)
            order.append(v)

        for g, mem in enumerate(self.groups):
            for x in mem:
                put(abs(x), CHOICE, -1)
        for v in range(1, nv + 1):
            if present[v] and gid[v] < 0 and not has_cand[v]:
                put(v, FREE, -1)
        # waiting lists over the unsettled inputs
        cis, cin = self.c_in_start, self.c_in
        rem = array("i", bytes(4 * n_cand))
        wcount = array("q", bytes(8 * (nv + 2)))
        for i in range(n_cand):
            k = 0
            for u in cin[cis[i]:cis[i + 1]]:
                if pos[u] < 0:
                    wcount[u + 1] += 1
                    k += 1
            rem[i] = k
        for u in range(1, nv + 2):
            wcount[u] += wcount[u - 1]
        wl = array("i", bytes(4 * wcount[nv + 1]))
        wfill = array("q", wcount)
        for i in range(n_cand):
            for u in cin[cis[i]:cis[i + 1]]:
                if pos[u] < 0:
                    wl[wfill[u]] = i
                    wfill[u] += 1
        kind = self.c_kind
        ready2, ready1 = deque(), deque()
        for i in range(n_cand):
            if rem[i] == 0:
                (ready2 if kind[i] != HELPER else ready1).append(i)
        unsettled = sum(1 for v in range(1, nv + 1) if present[v] and pos[v] < 0)
        heap = None
        low = []
        breaks = 0

        def settle_var(v, r, ci):
            put(v, r, ci)
            if ci >= 0:
                self._cover_cand(ci)
            for p in range(wcount[v], wcount[v + 1]):
                i = wl[p]
                rem[i] -= 1
                if rem[i] == 0:
                    (ready2 if kind[i] != HELPER else ready1).append(i)

        while unsettled:
            if ready2 or ready1:
                i = ready2.popleft() if ready2 else ready1.popleft()
                v = self.c_out[i]
                if pos[v] >= 0:
                    continue
                settle_var(v, kind[i], i)
                unsettled -= 1
                continue
            if heap is None:
                # A dead lock frees one variable.  In a file numbered in
                # circuit order (definitions read lower-numbered variables)
                # the lowest-numbered unsettled one: encoders number the
                # primary variables first (inputs, keys, values).  Otherwise
                # the one the most waiting candidates read, among those with
                # no two-way definition first (netlist_census.py's rule).
                topo = self._topo_numbered()
                self.stats["topo_numbered"] = int(topo)
                if topo:
                    low = array("q", (v for v in range(nv, 0, -1) if present[v] and pos[v] < 0))
                else:
                    low = free_order((v, two_way[v], wcount[v + 1] - wcount[v], os_[2 * v + 2] - os_[2 * v])
                                     for v in range(1, nv + 1) if present[v] and pos[v] < 0)
                heap = True
            self._tick()
            v = 0
            while low:
                u = low.pop()
                if pos[u] < 0:
                    v = u
                    break
            settle_var(v, FREE, -1)
            unsettled -= 1
            breaks += 1
        self.stats["cycle_breaks"] = breaks

    def settle_order(self, npass, piv_c, piv_x):
        """settle() for a reading with order groups, in two stages.
        First, what the order groups alone fix is computed from them: the
        definitions and helpers whose inputs are all settled, two-way
        definitions before helpers, and no variable is freed.  Then the
        other groups are found; one with a member that is computed already
        is a function of the numbers and not a choice (the pyramid's cube
        groups: every placement is a test on three labels).  Then the
        settling goes on as in settle(): the new groups' members, the
        variables with no candidate (free), and forward from there, a dead
        lock freeing one variable."""
        cnf = self.cnf
        nv = cnf.nv
        os_ = cnf.occ_start
        role, pos, order, acc, gid = self.role, self.pos, self.order, self.acc, self.gid
        present = bytearray(nv + 1)
        for v in range(1, nv + 1):
            if os_[2 * v] != os_[2 * v + 2]:
                present[v] = 1
        n_cand = len(self.c_out)
        has_cand = bytearray(nv + 1)
        two_way = bytearray(nv + 1)
        for v, k in zip(self.c_out, self.c_kind):
            has_cand[v] = 1
            if k != HELPER:
                two_way[v] = 1

        def put(v, r, ci):
            role[v] = r
            acc[v] = ci
            pos[v] = len(order)
            order.append(v)

        n_order = len(self.groups)
        for mem in self.groups:
            for x in mem:
                put(abs(x), CHOICE, -1)
        cis, cin = self.c_in_start, self.c_in
        rem = array("i", bytes(4 * n_cand))
        wcount = array("q", bytes(8 * (nv + 2)))
        for i in range(n_cand):
            k = 0
            for u in cin[cis[i]:cis[i + 1]]:
                if pos[u] < 0:
                    wcount[u + 1] += 1
                    k += 1
            rem[i] = k
        for u in range(1, nv + 2):
            wcount[u] += wcount[u - 1]
        wl = array("i", bytes(4 * wcount[nv + 1]))
        wfill = array("q", wcount)
        for i in range(n_cand):
            for u in cin[cis[i]:cis[i + 1]]:
                if pos[u] < 0:
                    wl[wfill[u]] = i
                    wfill[u] += 1
        kind = self.c_kind
        ready2, ready1 = deque(), deque()
        for i in range(n_cand):
            if rem[i] == 0:
                (ready2 if kind[i] != HELPER else ready1).append(i)

        def settle_var(v, r, ci):
            put(v, r, ci)
            if ci >= 0:
                self._cover_cand(ci)
            for p in range(wcount[v], wcount[v + 1]):
                i = wl[p]
                rem[i] -= 1
                if rem[i] == 0:
                    (ready2 if kind[i] != HELPER else ready1).append(i)

        def forward():
            n = 0
            while ready2 or ready1:
                i = ready2.popleft() if ready2 else ready1.popleft()
                v = self.c_out[i]
                if pos[v] >= 0:
                    continue
                settle_var(v, kind[i], i)
                n += 1
            return n

        # stage one: what the order groups alone fix
        self.stats["early"] = forward()
        self.early_end = len(order)                # the order groups' members and what they alone fix
        self._tick()
        # the other groups; their members and the variables with no candidate are sources too
        self.find_groups(npass, piv_c, piv_x, computed_early=True)
        for mem in self.groups[n_order:]:
            for x in mem:
                settle_var(abs(x), CHOICE, -1)
        for v in range(1, nv + 1):
            if present[v] and pos[v] < 0 and not has_cand[v]:
                settle_var(v, FREE, -1)
        # stage two: forward, a dead lock freeing one variable (settle()'s rule)
        unsettled = sum(1 for v in range(1, nv + 1) if present[v] and pos[v] < 0)
        heap = None
        low = []
        breaks = 0
        while unsettled:
            n = forward()
            unsettled -= n
            if not unsettled:
                break
            if heap is None:
                topo = self._topo_numbered()
                self.stats["topo_numbered"] = int(topo)
                if topo:
                    low = array("q", (v for v in range(nv, 0, -1) if present[v] and pos[v] < 0))
                else:
                    low = free_order((v, two_way[v], wcount[v + 1] - wcount[v], os_[2 * v + 2] - os_[2 * v])
                                     for v in range(1, nv + 1) if present[v] and pos[v] < 0)
                heap = True
            self._tick()
            v = 0
            while low:
                u = low.pop()
                if pos[u] < 0:
                    v = u
                    break
            settle_var(v, FREE, -1)
            unsettled -= 1
            breaks += 1
        self.stats["cycle_breaks"] = breaks

    def _topo_numbered(self):
        """Is the file numbered in circuit order?  Judged on the two-way
        definitions whose clauses define one variable only (an XOR's clauses
        define each of its variables, an exactly-one clause each member): at
        least 20, and 95% of them read only lower-numbered variables.  With
        fewer, on the helpers: 80% read only lower- or only higher-numbered
        variables (a one-way circuit has both directions, and the gates its
        XORs read in both polarities are written both ways)."""
        cis, cin = self.c_in_start, self.c_in
        gate_defines = Counter(g[2] for g in self.gates)   # how many variables a gate's clause set defines
        single = ordered = 0
        hel = mono = 0
        for i in range(len(self.c_out)):
            v = self.c_out[i]
            k = self.c_kind[i]
            ins = cin[cis[i]:cis[i + 1]]
            if k == HELPER:
                hel += 1
                if not ins or max(ins) < v or min(ins) > v:
                    mono += 1
            elif (self.npass[self.c_a[i]] if k == DEF else gate_defines[self.gates[self.c_a[i]][2]]) == 1:
                single += 1
                if not ins or max(ins) < v:
                    ordered += 1
        if single >= 20:
            return ordered >= 0.95 * single
        return hel >= 20 and mono >= 0.8 * hel

    def _cover_cand(self, i):
        """Mark the clauses the accepted candidate i makes true."""
        cnf = self.cnf
        cover = self.cover
        kind = self.c_kind[i]
        if kind == DEF:
            c, x = self.c_a[i], self.c_b[i]
            cover[c] = C_DEF
            mem = set(cnf.clause(c))
            nx = -x
            ln = 2 * nx if nx > 0 else 1 - 2 * nx
            bs, bp, bc = cnf.bin_start, cnf.bin_part, cnf.bin_cid
            for p in range(bs[ln], bs[ln + 1]):
                if -bp[p] in mem and bp[p] != -x:
                    cover[bc[p]] = C_DEF
        elif kind == GATE:
            for c in self.gates[self.c_a[i]][2]:
                cover[c] = C_GATE
        else:
            y = self.c_a[i]
            for c in cnf.occs(y):
                cover[c] = C_HELPER
            if not len(cnf.occs(y)):
                for c in cnf.occs(-y):        # a pure literal: -y is always true
                    cover[c] = C_HELPER

    # -- step 5: what is left ---------------------------------------------
    def classify_rest(self):
        """Clauses no role covered: over one group's members and implied by
        the group (two or more negated members, or every member for an
        exactly-one), or constraints."""
        cnf = self.cnf
        lits, starts, gid, cover = cnf.lits, cnf.starts, self.gid, self.cover
        groups = self.groups
        for c in range(cnf.m):
            if cover[c]:
                continue
            s0, s1 = starts[c], starts[c + 1]
            g = gid[abs(lits[s0])]
            if g < 0:
                continue
            same = True
            for x in lits[s0 + 1:s1]:
                if gid[abs(x)] != g:
                    same = False
                    break
            if not same:
                continue
            mem = groups[g]
            if self.ordered[g]:
                # implied when every prefix of the members makes it true
                where = {abs(x): (j, x > 0) for j, x in enumerate(mem)}
                cl = [(where[abs(z)][0], (z > 0) == where[abs(z)][1]) for z in lits[s0:s1]]
                if all(any((j < v) == same for j, same in cl) for v in range(len(mem) + 1)):
                    cover[c] = C_GROUP
                continue
            ms = set(mem)
            negs = sum(1 for x in lits[s0:s1] if -x in ms)
            if negs >= 2 or (negs == 0 and not self.none_ok[g] and ms <= set(lits[s0:s1])):
                cover[c] = C_GROUP

    def _lap(self, name):
        t = time.time()
        self.times[name] = round(t - self._t, 1)
        self._t = t
        self._tick()

    def run(self):
        cnf = self.cnf
        self.times = {}
        self._t = time.time()
        if cnf.occ is None:
            cnf.build_occ()
        self._lap("occ")
        piv_c, piv_x, npass = self.pivot_pass()
        self._lap("pivot")
        claimed = self.two_way_candidates(piv_c, piv_x)
        self._lap("defs")
        self.npass = npass                  # how many variables each clause defines (the circuit-order test)
        if self.given_order is not None:
            for seq in self.given_order:
                g = len(self.groups)
                self.groups.append(tuple(seq))
                self.chain.append(False)
                self.none_ok.append(False)
                self.ordered.append(True)
                for x in seq:
                    self.gid[abs(x)] = g
            self.stats["order_groups"] = len(self.groups)
        elif self.want_order:
            self.find_order_groups(claimed)
            self._lap("order")
        if self.groups:
            # order groups were found: what they alone fix is computed first, and
            # the other groups are found after that (settle_order)
            self.helper_candidates(claimed)
            del claimed
            self._lap("helpers")
            self.settle_order(npass, piv_c, piv_x)
            del piv_c, piv_x
            self._lap("settle")
        else:
            self.find_groups(npass, piv_c, piv_x)
            del piv_c, piv_x
            self._lap("groups")
            self.helper_candidates(claimed)
            del claimed
            self._lap("helpers")
            self.settle()
            self._lap("settle")
        self.classify_rest()
        self.check_acyclic()
        self._lap("rest")
        if any(self.ordered):
            self.cover_by_order()
            self.cover_by_count()
            self._lap("cover")
        return self

    def cover_by_order(self):
        """Clauses that the order groups make true whatever their values.
        An encoder that ties a test to a number in one-way pieces leaves
        clauses that no role claims, yet no choice can break them: the
        pyramid's "the label is above 0, so this placement is not here".
        A clause whose variables are all fixed by the order groups alone
        (settle_order's first stage) depends on a few of the groups; with up
        to ORDER_COVER_PATTERNS joint values of those, every one is tried
        (64 at a time would not do: all of them, bit-parallel), and a clause
        true in all of them is covered.  The search then never looks at it."""
        cnf = self.cnf
        lits, starts, cover = cnf.lits, cnf.starts, self.cover
        role, pos, gid, acc = self.role, self.pos, self.gid, self.acc
        cis, cin = self.c_in_start, self.c_in
        end = getattr(self, "early_end", 0)
        groups = self.groups
        intern = {}
        npat = {}

        def patterns(sset):
            n = npat.get(sset)
            if n is None:
                n = 1
                for g in sset:
                    n *= len(groups[g]) + 1
                npat[sset] = n
            return n

        sup = {}                                    # early variable -> the order groups it depends on (None: too many)
        for v in self.order[:end]:
            if role[v] == CHOICE:
                sset = frozenset((gid[v],))
            else:
                i = acc[v]
                sset = frozenset()
                for u in cin[cis[i]:cis[i + 1]]:
                    su = sup.get(u)
                    if su is None:
                        sset = None
                        break
                    if not su <= sset:
                        sset = sset | su
                        if patterns(sset) > ORDER_COVER_PATTERNS:
                            sset = None
                            break
            sup[v] = None if sset is None else intern.setdefault(sset, sset)
        by_sup = defaultdict(list)
        for c in range(cnf.m):
            if cover[c]:
                continue
            sset = frozenset()
            for z in lits[starts[c]:starts[c + 1]]:
                su = sup.get(abs(z))
                if su is None:
                    sset = None
                    break
                if not su <= sset:
                    sset = sset | su
                    if patterns(sset) > ORDER_COVER_PATTERNS:
                        sset = None
                        break
            if sset:
                by_sup[intern.setdefault(sset, sset)].append(c)
        n_cov = 0
        for sset, cls in by_sup.items():
            self._tick()
            gs = sorted(sset)
            n = patterns(sset)
            mask = (1 << n) - 1
            val = {}
            stride = 1
            for g in gs:                                # pattern p gives group g the value (p // stride) % (k + 1)
                k = len(groups[g])
                above = [0] * (k + 1)                   # above[j]: the patterns with a value above j
                for p in range(n):
                    v = (p // stride) % (k + 1)
                    if v:
                        above[v - 1] |= 1 << p
                for j in range(k - 1, 0, -1):
                    above[j - 1] |= above[j]
                for j, x in enumerate(groups[g]):
                    val[abs(x)] = above[j] if x > 0 else mask ^ above[j]
                stride *= k + 1
            # the variables the clauses read, down to the groups' members, in settling order
            need = set()
            stack = [abs(z) for c in cls for z in lits[starts[c]:starts[c + 1]]]
            while stack:
                v = stack.pop()
                if v in need or v in val:
                    continue
                need.add(v)
                i = acc[v]
                stack.extend(cin[cis[i]:cis[i + 1]])
            self._eval_into(val, need, mask)
            for c in cls:
                t = 0
                for z in lits[starts[c]:starts[c + 1]]:
                    t |= val[z] if z > 0 else mask ^ val[-z]
                    if t == mask:
                        cover[c] = C_ORDER
                        n_cov += 1
                        break
        self.stats["cov_order"] = n_cov

    def _eval_into(self, val, need, mask):
        """Bit-parallel values of the computed variables `need`, in settling
        order, into val (which holds their inputs' values already)."""
        cnf = self.cnf
        lits, starts = cnf.lits, cnf.starts
        role, pos, acc = self.role, self.pos, self.acc
        for v in sorted(need, key=lambda v: pos[v]):
            r = role[v]
            i = acc[v]
            if r == DEF:
                c, x = self.c_a[i], self.c_b[i]
                t = 0
                for y in lits[starts[c]:starts[c + 1]]:
                    if y != x:
                        t |= val[y] if y > 0 else mask ^ val[-y]
                xv = mask ^ t
                val[v] = xv if x > 0 else mask ^ xv
            elif r == GATE:
                gsup, tt, _cl = self.gates[self.c_a[i]]
                out = 0
                for mt in range(1 << len(gsup)):
                    if not (tt >> mt) & 1:
                        continue
                    t = mask
                    for k2, u in enumerate(gsup):
                        t &= val[u] if (mt >> k2) & 1 else mask ^ val[u]
                    out |= t
                val[v] = out
            else:
                y = self.c_a[i]
                yv = 0
                for c in cnf.occs(y):
                    t = mask
                    for z in lits[starts[c]:starts[c + 1]]:
                        if z != y:
                            t &= mask ^ (val[z] if z > 0 else mask ^ val[-z])
                    yv |= t
                val[v] = yv if y > 0 else mask ^ yv

    def cover_by_count(self):
        """Count constraints.  An exactly-one group over variables that the
        order groups compute is no choice, but it is still one constraint:
        exactly one of its members is true (the pyramid: each cube sits at
        one position, over its 190 to 504 placements).  As clauses that is
        the long clause plus the encoding of "at most one": pairs, and a tree
        of helper variables.  The search can keep one count instead, if the
        clauses it stands for are found.  A clause is one of them when all
        its variables are members of the group or computed from its members
        alone, and it is true in every one of the patterns "exactly this
        member is true" (all of them tried, bit-parallel).  Such a clause
        holds whenever the count is one, so the search does not look at it;
        with the count off it would be false, and the count says so in its
        place."""
        if not self.derived_exo:
            return
        cnf = self.cnf
        lits, starts, cover = cnf.lits, cnf.starts, self.cover
        pos, acc, role = self.pos, self.acc, self.role
        cis, cin = self.c_in_start, self.c_in
        end = getattr(self, "early_end", 0)
        taken = set()
        chosen = []
        for mem, c in sorted(self.derived_exo, key=lambda t: (-len(t[0]), t[1])):
            vs = [abs(x) for x in mem]
            if len(vs) < 2 or any(v in taken or not (0 <= pos[v] < end) or role[v] == CHOICE for v in vs):
                continue                         # a member in a bigger group, or not computed by the order groups
            taken.update(vs)
            chosen.append(mem)
        if not chosen:
            return
        ANY = -2                                 # computed from nothing (a helper no clause needs): fits every group
        own = {}                                 # variable -> the group whose members alone it is computed from
        for gi, mem in enumerate(chosen):
            for x in mem:
                own[abs(x)] = gi

        def owner(vs):
            """The one group the variables belong to (ANY when each fits every group), or -1."""
            g = ANY
            for u in vs:
                o = own.get(u, -1)
                if o == -1 or (o != ANY and g != ANY and o != g):
                    return -1
                if o != ANY:
                    g = o
            return g

        for v in self.order:
            if v in own or acc[v] < 0:
                continue
            g = owner(cin[cis[acc[v]]:cis[acc[v] + 1]])
            if g != -1:
                own[v] = g
        by_group = defaultdict(list)
        for c in range(cnf.m):
            if cover[c]:
                continue
            g = owner(abs(z) for z in lits[starts[c]:starts[c + 1]])
            if g >= 0:
                by_group[g].append(c)
        n_cov = 0
        for gi, mem in enumerate(chosen):
            self._tick()
            cls = by_group.get(gi, ())
            n = len(mem)
            mask = (1 << n) - 1
            val = {}
            for i, x in enumerate(mem):               # pattern i: member i is the one that is true
                val[abs(x)] = (1 << i) if x > 0 else mask ^ (1 << i)
            need = set()
            stack = [abs(z) for c in cls for z in lits[starts[c]:starts[c + 1]]]
            while stack:
                v = stack.pop()
                if v in need or v in val:
                    continue
                need.add(v)
                stack.extend(cin[cis[acc[v]]:cis[acc[v] + 1]])
            self._eval_into(val, need, mask)
            for c in cls:
                t = 0
                for z in lits[starts[c]:starts[c + 1]]:
                    t |= val[z] if z > 0 else mask ^ val[-z]
                    if t == mask:
                        cover[c] = C_COUNT
                        self.count_of[c] = gi
                        n_cov += 1
                        break
        self.counts = chosen
        self.stats["cov_count"] = n_cov

    def check_acyclic(self):
        """Every accepted definition and helper reads only variables settled
        before it (true by construction; checked anyway)."""
        pos, acc = self.pos, self.acc
        cis, cin = self.c_in_start, self.c_in
        for v in self.order:
            i = acc[v]
            if i < 0:
                continue
            for u in cin[cis[i]:cis[i + 1]]:
                if pos[u] >= pos[v]:
                    raise AssertionError(f"variable {v} reads {u}, settled later")

    # -- computing every variable from the choices --------------------------
    def evaluate(self, choice_masks, free_masks, mask=MASK):
        """Values (bit-parallel ints over the patterns) of every variable:
        choice_masks[g][j] says in which patterns member j of group g is the
        true one (for an order group: in which patterns its value is j, the
        number of true members), free_masks[v] the patterns in which free v
        is true."""
        cnf = self.cnf
        lits, starts = cnf.lits, cnf.starts
        val = [0] * (cnf.nv + 1)

        def lv(z):
            return val[z] if z > 0 else mask ^ val[-z]

        done_group = set()
        for v in self.order:
            r = self.role[v]
            if r == CHOICE:
                g = self.gid[v]
                if g in done_group:
                    continue
                done_group.add(g)
                if self.ordered[g]:
                    # choice_masks[g][v]: the patterns in which the value is v;
                    # member j is true in those with a value above j
                    mem = self.groups[g]
                    above = 0
                    for j in range(len(mem) - 1, -1, -1):
                        above |= choice_masks[g][j + 1]
                        x = mem[j]
                        val[abs(x)] = above if x > 0 else mask ^ above
                    continue
                for j, x in enumerate(self.groups[g]):
                    mj = choice_masks[g][j]
                    val[abs(x)] = mj if x > 0 else mask ^ mj
            elif r == FREE:
                val[v] = free_masks.get(v, 0)
            elif r == DEF:
                i = self.acc[v]
                c, x = self.c_a[i], self.c_b[i]
                t = 0
                for y in lits[starts[c]:starts[c + 1]]:
                    if y != x:
                        t |= lv(y)
                xv = mask ^ t                 # x is true exactly when no other literal is
                val[v] = xv if x > 0 else mask ^ xv
            elif r == GATE:
                sup, tt, _cl = self.gates[self.c_a[self.acc[v]]]
                out = 0
                for mt in range(1 << len(sup)):
                    if not (tt >> mt) & 1:
                        continue
                    t = mask
                    for k, u in enumerate(sup):
                        t &= val[u] if (mt >> k) & 1 else mask ^ val[u]
                    out |= t
                val[v] = out
            elif r == HELPER:
                y = self.c_a[self.acc[v]]
                yv = 0
                for c in cnf.occs(y):
                    t = mask
                    for z in lits[starts[c]:starts[c + 1]]:
                        if z != y:
                            t &= mask ^ lv(z)
                    yv |= t                   # y is needed by clause c
                val[v] = yv if y > 0 else mask ^ yv
        return val

    def simulate(self, seed=1):
        """(covered clauses false in some pattern, mean false constraint
        clauses per pattern) over SIM_PATTERNS random choice assignments."""
        rng = random.Random(seed)
        cm = []
        for mem, none_ok, ordered in zip(self.groups, self.none_ok, self.ordered):
            ms = [0] * (len(mem) + 1)           # the last slot is "none" (an order group: the value len(mem))
            for p in range(SIM_PATTERNS):
                ms[rng.randrange(len(mem) + (none_ok or ordered))] |= 1 << p
            cm.append(ms)
        fm = {v: rng.getrandbits(SIM_PATTERNS) for v in self.order if self.role[v] == FREE}
        val = self.evaluate(cm, fm)
        cnf = self.cnf
        lits, starts, cover = cnf.lits, cnf.starts, self.cover
        bad = 0
        false_total = 0
        # a count constraint: the patterns in which exactly one member is true
        one_of = []
        for mem in self.counts:
            one = two = 0
            for x in mem:
                mv = val[x] if x > 0 else MASK ^ val[-x]
                two |= one & mv
                one |= mv
            one_of.append(one & ~two)
            false_total += SIM_PATTERNS - bin(one & ~two).count("1")
        for c in range(cnf.m):
            s = 0
            for z in lits[starts[c]:starts[c + 1]]:
                s |= val[z] if z > 0 else MASK ^ val[-z]
                if s == MASK:
                    break
            if s != MASK:
                if cover[c] == C_COUNT:
                    bad += bool(~s & one_of[self.count_of[c]])    # false although its count is one
                elif cover[c]:
                    bad += 1
                else:
                    false_total += SIM_PATTERNS - bin(s).count("1")
        return bad, false_total / SIM_PATTERNS

    def full_assignment(self, choice_index, free_value):
        """One assignment: choice_index[g] is the true member of group g
        (len(group) for none, in an at-most-one group; for an order group
        its value, the number of true members), free_value[v] the value of
        free v; returns {var: bool}."""
        cm = [[1 if choice_index[g] == j else 0 for j in range(len(mem) + self.ordered[g])]
              for g, mem in enumerate(self.groups)]
        fm = {v: 1 if free_value.get(v) else 0 for v in self.order if self.role[v] == FREE}
        val = self.evaluate(cm, fm, mask=1)
        return {v: bool(val[v]) for v in self.order}

    # -- summary ------------------------------------------------------------
    def summary(self):
        cnf = self.cnf
        role, gid = self.role, self.gid
        rc = Counter(role[v] for v in self.order)
        sizes = Counter(len(g) for g in self.groups)
        n_free = rc[FREE]
        bits = sum(math.log2(len(g) + (n or o)) for g, n, o in zip(self.groups, self.none_ok, self.ordered)) + n_free
        osizes = Counter(len(g) for g, o in zip(self.groups, self.ordered) if o)
        present = len(self.order)
        cov = Counter(self.cover)
        lits, starts = cnf.lits, cnf.starts
        con_lens = Counter()
        con_choice = 0
        for c in range(cnf.m):
            if self.cover[c]:
                continue
            s0, s1 = starts[c], starts[c + 1]
            con_lens[s1 - s0] += 1
            if all(role[abs(z)] in (CHOICE, FREE) for z in lits[s0:s1]):
                con_choice += 1
        wide = 0
        dmax = 0
        cis = self.c_in_start
        for v in self.order:
            if role[v] == DEF:
                i = self.acc[v]
                k = cis[i + 1] - cis[i]
                dmax = max(dmax, k)
                wide += k > 6
        return {
            "present": present,
            "groups": len(self.groups),
            "group_vars": sum(len(g) for g in self.groups),
            "group_sizes": " ".join(f"{k}x{v}" for k, v in sorted(sizes.items(), key=lambda kv: -kv[1])[:6]),
            "chain_groups": sum(self.chain),
            "amo_groups": sum(self.none_ok),
            "order_groups": sum(self.ordered),
            "order_sizes": " ".join(f"{k}x{v}" for k, v in sorted(osizes.items(), key=lambda kv: -kv[1])[:6]),
            "order_inputs": self.stats["order_inputs"],
            "order_derived_groups": self.stats["order_derived_groups"],
            "early": self.stats["early"],
            "order_links": " ".join(f"{k}:{v}" for k, v in sorted(self.links.items())),
            "overlap_groups": self.stats["overlap_groups"],
            "derived_groups": self.stats["derived_groups"],
            "group_tests_cut": self.stats["group_tests_cut"],
            "defs": rc[DEF],
            "wide_defs": wide,
            "def_inputs_max": dmax,
            "gates": rc[GATE],
            "helpers": rc[HELPER],
            "free": n_free,
            "cycle_breaks": self.stats["cycle_breaks"],
            "bits": round(bits, 1),
            "choice_share": round((len(self.groups) + n_free) / present, 4) if present else 0,
            "cov_group": cov[C_GROUP],
            "cov_def": cov[C_DEF],
            "cov_gate": cov[C_GATE],
            "cov_helper": cov[C_HELPER],
            "cov_order": cov[C_ORDER],
            "cov_count": cov[C_COUNT],
            "counts": len(self.counts),
            "constraints": cov[C_NONE],
            "constraint_share": round(cov[C_NONE] / cnf.m, 4) if cnf.m else 0,
            "con_choice": con_choice,
            "con_lens": " ".join(f"{k}x{v}" for k, v in sorted(con_lens.items(), key=lambda kv: -kv[1])[:6]),
        }


def order_chains(red, deadline=None, order_min=MIN_ORDER):
    """The order groups of a reduced formula (find_order_groups), and when
    there are any its exactly-one clauses (the members of each; they are
    looked for here, in the formula as it stands: with the clauses that
    order_links adds every placement implies its bits, and the walk from
    one member to the opposite of another no longer fits its budget).
    Nothing else is read."""
    rd = Reading(red, deadline, order=True, order_min=order_min)
    if red.occ is None:
        red.build_occ()
    piv_c, piv_x, npass = rd.pivot_pass()
    claimed = rd.two_way_candidates(piv_c, piv_x)
    rd.find_order_groups(claimed)
    exo = []
    if rd.groups:
        # the walks from one member to the opposite of another do not pass through the bits of
        # the numbers: a placement implies its labels' bits, and a bit rules out every placement
        # that needs another value, which is not the exclusion looked for and uses up the budget
        rd.no_walk = bytearray(red.nv + 1)
        for mem in rd.groups:
            for x in mem:
                rd.no_walk[abs(x)] = 1
        exo = [mem for mem, _c, _chained, none_ok in rd._group_candidates(npass) if not none_ok]
    return list(rd.groups), rd.stats, exo


def order_links(red, groups, deadline=None):
    """What reads the numbers, asked of unit propagation (tools/cnf_prop.py).

    An encoder can tie a test to a number one way only.  Z3 writes "x = 2"
    as a count over x's sorted bits, and a placement of the pyramid as a
    test on three labels in several one-way clauses: no clause shape says
    "this variable is x = 2", so the role reading cannot compute it from
    the number, though it is a function of it in every model.  Propagation
    can say so (from the bits to the tests it misses nothing: solver
    README, "Numbers stored as sorted bits").  Each order group is set to
    each of its values in turn, and what propagation then fixes is noted:
      dead     a value whose pattern runs into a conflict: no model takes it
      unit     a variable every live value sets the same way
      equal    a variable on exactly from some value up (or down): it equals
               a bit of the number, or its opposite
      test     a variable on exactly on a stretch of values lo..hi: it is
               "bit lo-1 and not bit hi" (or its opposite)
      product  a variable that single numbers only ever turn off (outside a
               stretch each), and that propagation turns on when every one
               of those numbers sits in its stretch: the AND of those tests
               (a placement: x = 2 and y = 3 and z = 5); the same for one
               that is only ever turned on
      never    such a variable whose stretches run into a conflict together:
               no model has it on (a unit)
    Each finding is written as the clauses of a two-way definition over the
    numbers' bits.  They are true in every model of the formula (what
    propagation derives from a full pattern holds in every model with that
    pattern, and every model has one), so the formula with them has the
    same models; a model found there is checked against the original
    anyway.  Returns (units, clauses, counts)."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from cnf_prop import Prop
    lits, starts = red.lits, red.starts
    p = Prop(red.nv, [lits[starts[c]:starts[c + 1]] for c in range(red.m)])
    counts = Counter()
    units, clauses = [], []
    if p.root_conflict:
        return [1, -1], [], counts
    readers = {}                       # variable -> [(group, values that set it true, values that set it false)]
    live = []
    for g, mem in enumerate(groups):
        if deadline is not None and time.time() > deadline:
            raise TimeoutError("wall")
        k = len(mem)
        own = {abs(x) for x in mem}
        tm, fm = defaultdict(int), defaultdict(int)
        lv = 0
        for v in range(k + 1):
            got = p.implied([x if j < v else -x for j, x in enumerate(mem)])
            if got is None:
                counts["dead"] += 1
                if v == 0:
                    units.append(mem[0])
                elif v == k:
                    units.append(-mem[k - 1])
                else:
                    clauses.append((-mem[v - 1], mem[v]))
                continue
            lv |= 1 << v
            for z in got:
                if abs(z) in own:
                    continue
                if z > 0:
                    tm[z] |= 1 << v
                else:
                    fm[-z] |= 1 << v
        live.append(lv)
        for w in set(tm) | set(fm):
            readers.setdefault(w, []).append((g, tm.get(w, 0), fm.get(w, 0)))

    def stretch(mask, lv):
        """(lo, hi) when mask is, among the live values lv, exactly those from lo to hi; else None."""
        if not mask:
            return None
        lo = (mask & -mask).bit_length() - 1
        hi = mask.bit_length() - 1
        return (lo, hi) if lv & ((1 << (hi + 1)) - (1 << lo)) == mask else None

    def bounds(g, lo, hi):
        """The bits that say "the value of group g is from lo to hi"."""
        mem = groups[g]
        out = []
        if lv_low[g] < lo:
            out.append(mem[lo - 1])
        if lv_high[g] > hi:
            out.append(-mem[hi])
        return out

    lv_low = [(lv & -lv).bit_length() - 1 if lv else 0 for lv in live]
    lv_high = [lv.bit_length() - 1 if lv else 0 for lv in live]
    group_of = {abs(x): g for g, mem in enumerate(groups) for x in mem}
    occ, os_ = red.occ, red.occ_start

    def near(w):
        """The order groups with a bit in a clause of w."""
        out = set()
        for c in occ[os_[2 * w]:os_[2 * w + 2]]:
            for z in lits[starts[c]:starts[c + 1]]:
                g = group_of.get(abs(z))
                if g is not None:
                    out.add(g)
        return out

    done = set()
    partial = []
    for w in sorted(readers):
        fulls = [(g, t, f) for g, t, f in readers[w] if (t | f) == live[g] and live[g]]
        if not fulls:
            partial.append(w)
            continue
        if len(fulls) > 1:
            # set by every value of more than one number (a constraint ties the numbers):
            # the number whose bits share a clause with it is the one it is a test on
            nw = near(w)
            fulls.sort(key=lambda e: e[0] not in nw)
            counts["several_numbers"] += 1
        g, t, f = fulls[0]
        if not f or not t:
            units.append(w if not f else -w)
            counts["unit"] += 1
            done.add(w)
            continue
        for mask, y in ((t, w), (f, -w)):              # y is on exactly on the stretch
            st = stretch(mask, live[g])
            if st is None:
                continue
            b = bounds(g, *st)
            if len(b) == 1:
                clauses.append((-y, b[0]))
                clauses.append((y, -b[0]))
                counts["equal"] += 1
            else:
                clauses.append((-y, b[0]))
                clauses.append((-y, b[1]))
                clauses.append((y, -b[0], -b[1]))
                counts["test"] += 1
            done.add(w)
            break
        else:
            counts["other_function"] += 1
    for u in units:
        if not p.add([u]):
            return [1, -1], [], counts
    for cl in clauses:
        if not p.add(cl):
            return [1, -1], [], counts
    # the tests on several numbers at once; a variable equal to one that is done is left to that equality
    bs, bp = red.bin_start, red.bin_part
    n_first = len(clauses)
    for w in partial:
        if deadline is not None and (len(done) & 0xFF) == 0 and time.time() > deadline:
            raise TimeoutError("wall")
        if p.value(w) != 0:
            continue
        pos = set(bp[bs[2 * w]:bs[2 * w + 1]])          # y with a clause (w | y)
        if any(-y in pos and abs(y) in done for y in bp[bs[2 * w + 1]:bs[2 * w + 2]]):
            counts["equal_to_done"] += 1
            continue
        ent = readers[w]
        for sign in (1, -1):                            # sign 1: only ever turned off; -1: only ever turned on
            if any((t if sign > 0 else f) for _g, t, f in ent):
                continue
            per = []                                    # (group, the bits that hold it in its stretch, values allowed)
            ok = True
            for g, t, f in ent:
                allowed = live[g] & ~(f if sign > 0 else t)
                st = stretch(allowed, live[g])
                if st is None:
                    ok = False
                    break
                bg = bounds(g, *st)
                if bg:
                    per.append((g, bg, bin(allowed).count("1")))
            if not ok or not per:
                continue
            y = w * sign
            # Only the numbers the test needs.  A number can turn a variable off through a
            # constraint and not through what the variable is: a placement is off when
            # another label puts its cube elsewhere, by "each cube once".  With that number
            # in the definition the variable would be wrong where the constraint is broken,
            # and the clauses that say what it is would break in its place.  The numbers
            # that leave the variable at most half of their values are tried together first
            # (the placement's own three labels leave it one value each); if they do not
            # turn it on, the others are added one at a time, most telling first, one that
            # contradicts the rest is left out, and then each is dropped again if the rest
            # still turn the variable on.
            per.sort(key=lambda e: (e[2], e[0]))
            strong = [e for e in per if 2 * e[2] <= bin(live[e[0]]).count("1")]
            chosen = None
            never = False
            if strong:
                got = p.implied([x for e in strong for x in e[1]])
                if got is None:
                    never = True
                elif y in got:
                    chosen = strong
            if chosen is None and not never:
                acc = list(strong)
                for e in per:
                    if e in acc:
                        continue
                    acc.append(e)
                    got = p.implied([x for q in acc for x in q[1]])
                    if got is None:
                        never = True
                        break
                    if y in got:
                        chosen = acc
                        break
            if never:
                # every one of these stretches follows from the variable being on (outside
                # a stretch its number turns it off), and together they run into a
                # conflict: no model has it on
                units.append(-y)
                counts["never"] += 1
                done.add(w)
                if not p.add([-y]):
                    return [1, -1], [], counts
                break
            if chosen is None:
                counts["not_a_product"] += 1
                break
            if len(chosen) > len(strong):
                for e in list(chosen):
                    if len(chosen) == 1:
                        break
                    got = p.implied([x for q in chosen if q is not e for x in q[1]])
                    if got is not None and y in got:
                        chosen = [q for q in chosen if q is not e]
            counts["number_dropped"] += len(per) - len(chosen)
            per = chosen
            b = [x for e in per for x in e[1]]
            for x in b:
                clauses.append((-y, x))
            clauses.append((y,) + tuple(-x for x in b))
            counts["product"] += 1
            done.add(w)
            break
        else:
            counts["mixed"] += 1
    counts["clauses"] = len(clauses)
    return units, clauses, counts


def read_roles(cnf, deadline=None, order=False, order_min=MIN_ORDER):
    """(fixed, Reading or None on a root conflict, satisfied clause count).
    With order, numbers stored as sorted bits are read as choices too: the
    order groups are found (find_order_groups), what reads them is asked of
    propagation (order_links) and added to the formula as clauses, and the
    roles are read of that formula with those groups."""
    fixed, red, nsat = unit_propagate(cnf)
    if red is None:
        return fixed, None, 0
    if not order:
        return fixed, Reading(red, deadline).run(), nsat
    groups, found, exo = order_chains(red, deadline, order_min)
    if not groups:
        return fixed, Reading(red, deadline).run(), nsat        # no order group: the reading without them
    links = Counter()
    if red.m > ORDER_READ_CLAUSES:
        links["not_asked"] = 1
    else:
        units, clauses, counts = order_links(red, groups, deadline)
        links.update(counts)
        if units or clauses:
            lits, starts = array("i", red.lits), array("q", red.starts)
            for cl in [(u,) for u in units] + clauses:
                lits.extend(cl)
                starts.append(len(lits))
            more, red, nsat2 = unit_propagate(CNF(red.nv, lits, starts))
            fixed.update(more)
            nsat += nsat2
            if red is None:
                return fixed, None, 0
            if more:
                # a member that is fixed now is no choice: the low members can only
                # be fixed true and the high ones false (the chain), the rest stay
                kept = []
                for mem in groups:
                    rest = [x for x in mem if abs(x) not in more]
                    if len(rest) >= 2:
                        kept.append(tuple(rest))
                    else:
                        links["groups_fixed"] += 1
                groups = kept
                # the exactly-one clauses: a member fixed false is gone; one fixed true settles the clause
                kept = []
                for mem in exo:
                    if any(more.get(abs(x)) == (x > 0) for x in mem):
                        continue
                    rest = tuple(x for x in mem if abs(x) not in more)
                    if len(rest) >= 2:
                        kept.append(rest)
                exo = kept
    rd = Reading(red, deadline, order=True, order_min=order_min, order_groups=groups, exo_groups=exo)
    rd.links = links
    rd.stats["order_inputs"] = found["order_inputs"]         # the inputs the groups were found among
    return fixed, rd.run(), nsat


# ---------------------------------------------------------------------------
# one cell
# ---------------------------------------------------------------------------

COLUMNS = ["stem", "vars", "clauses", "truncated", "fixed", "satisfied", "present", "groups", "group_vars", "group_sizes",
           "chain_groups", "amo_groups", "overlap_groups", "derived_groups", "group_tests_cut", "defs", "wide_defs", "def_inputs_max", "gates", "helpers", "free",
           "cycle_breaks", "bits", "choice_share", "cov_group", "cov_def", "cov_gate", "cov_helper", "constraints",
           "constraint_share", "con_choice", "con_lens", "sim_bad", "sim_false", "sim_false_share", "read_seconds",
           "stages", "seconds", "error"]
# with --order: the order groups, the inputs they were looked for among, the
# other groups that turned out to be functions of them, and the variables the
# order groups alone fix
ORDER_COLUMNS = ["order_groups", "order_sizes", "order_inputs", "order_derived_groups", "early", "order_links",
                 "cov_order", "counts", "cov_count"]


def _stem(path):
    name = Path(path).name
    for ext in (".cnf.xz", ".cnf"):
        if name.endswith(ext):
            return name[:-len(ext)]
    return name


def analyse_cnf(cnf, row, deadline=None, order=False, order_min=MIN_ORDER):
    fixed, rd, nsat = read_roles(cnf, deadline, order, order_min)
    row["fixed"] = len(fixed)
    row["satisfied"] = nsat
    if rd is None:
        row["error"] = "root-conflict"
        return row, None
    row.update(rd.summary())
    t = time.time()
    bad, false_mean = rd.simulate()
    rd.times["sim"] = round(time.time() - t, 1)
    row["stages"] = " ".join(f"{k}:{v}" for k, v in rd.times.items())
    row["sim_bad"] = bad
    row["sim_false"] = round(false_mean, 2)
    # the mean counts the count constraints that do not hold, so the share is of clauses and counts
    n_con = row["constraints"] + row["counts"]
    row["sim_false_share"] = round(false_mean / n_con, 4) if n_con else 0
    return row, rd


def analyse(path, max_clauses=MAX_CLAUSES, seconds=CELL_SECONDS, order=False, order_min=MIN_ORDER):
    t0 = time.time()
    row = {"stem": _stem(path)}
    if header_clauses(path) > max_clauses:
        max_clauses = min(max_clauses, PREFIX_CLAUSES)
    cnf, trunc = read_cnf(path, max_clauses)
    row.update(vars=cnf.nv, clauses=cnf.m, truncated=int(trunc), read_seconds=round(time.time() - t0, 1))
    analyse_cnf(cnf, row, t0 + seconds, order, order_min)
    row["seconds"] = round(time.time() - t0, 1)
    return row


def _alarm(_sig, _frm):
    raise TimeoutError("wall")


def census_one(job):
    stem, path, max_clauses, seconds, order, order_min = job
    signal.signal(signal.SIGALRM, _alarm)
    signal.alarm(int(seconds) + 300)
    t0 = time.time()
    try:
        return analyse(path, max_clauses, seconds, order, order_min)
    except TimeoutError:
        return {"stem": stem, "error": "timeout:wall", "seconds": round(time.time() - t0, 1)}
    except MemoryError:
        return {"stem": stem, "error": "memory", "seconds": round(time.time() - t0, 1)}
    finally:
        signal.alarm(0)


def _limit_memory(mb):
    import resource
    resource.setrlimit(resource.RLIMIT_AS, (mb << 20, mb << 20))


def cmd_census(args):
    files = [Path(p) for p in args.files] if args.files else sorted(SUITE.glob("*.cnf.xz"))
    if any("sat-comp-2026" in str(f) for f in files):
        print("the 2026 suite is the holdout and is not analysed", file=sys.stderr)
        return 2
    stems = args.stems.split(",") if args.stems else None
    jobs = []
    for f in files:
        stem = _stem(f)
        if stems and not any(s in stem for s in stems):
            continue
        jobs.append((stem, str(f), args.max_clauses, args.seconds, args.order, args.order_min))
    jobs.sort(key=lambda j: -Path(j[1]).stat().st_size)      # the long tail starts early
    with open(args.out, "w", newline="") as fh, ProcessPoolExecutor(max_workers=args.workers, initializer=_limit_memory,
                                                                   initargs=(args.mem_mb,)) as ex:
        w = csv.DictWriter(fh, fieldnames=COLUMNS + (ORDER_COLUMNS if args.order else []), delimiter="\t",
                           extrasaction="ignore")
        w.writeheader()
        fh.flush()
        futs = {ex.submit(census_one, j): j for j in jobs}
        for fut in as_completed(futs):
            try:
                row = fut.result()
            except Exception as e:  # noqa: BLE001 (a worker killed by the memory limit)
                row = {"stem": futs[fut][0], "error": f"worker: {type(e).__name__}"}
            w.writerow(row)
            fh.flush()
            print(f"{row['stem'][:64]:64} groups={row.get('groups', '-')} free={row.get('free', '-')} "
                  f"bits={row.get('bits', '-')} con={row.get('constraints', '-')} bad={row.get('sim_bad', '-')} "
                  f"{row.get('error', '')} {row.get('seconds', '')}s", flush=True)
    return 0


CHAIN_COLUMNS = ["stem", "vars", "clauses", "truncated", "fixed", "present", "order_groups", "order_sizes",
                 "order_bits", "order_inputs", "exactly_one_clauses", "seconds", "error"]


def chains_one(job):
    """One row of the order-group census: the order groups of a cell (find_order_groups)
    and nothing else of the reading."""
    stem, path, max_clauses, seconds, order_min = job
    signal.signal(signal.SIGALRM, _alarm)
    signal.alarm(int(seconds) + 300)
    t0 = time.time()
    row = {"stem": stem}
    try:
        if header_clauses(path) > max_clauses:
            max_clauses = min(max_clauses, PREFIX_CLAUSES)
        cnf, trunc = read_cnf(path, max_clauses)
        row.update(vars=cnf.nv, clauses=cnf.m, truncated=int(trunc))
        fixed, red, _nsat = unit_propagate(cnf)
        row["fixed"] = len(fixed)
        if red is None:
            row["error"] = "root-conflict"
        else:
            groups, stats, exo = order_chains(red, t0 + seconds, order_min)
            sizes = Counter(len(g) for g in groups)
            os_ = red.occ_start
            row.update(present=sum(1 for v in range(1, red.nv + 1) if os_[2 * v] != os_[2 * v + 2]),
                       order_groups=len(groups),
                       order_sizes=" ".join(f"{k}x{v}" for k, v in sorted(sizes.items(), key=lambda kv: -kv[1])[:8]),
                       order_bits=sum(len(g) for g in groups), order_inputs=stats["order_inputs"],
                       exactly_one_clauses=len(exo))
    except TimeoutError:
        row["error"] = "timeout:wall"
    except MemoryError:
        row["error"] = "memory"
    finally:
        signal.alarm(0)
    row["seconds"] = round(time.time() - t0, 1)
    return row


def cmd_chains(args):
    """The order-group census: which cells hold numbers stored as sorted bits (one row a cell)."""
    files = [Path(p) for p in args.files] if args.files else sorted(SUITE.glob("*.cnf.xz"))
    if any("sat-comp-2026" in str(f.resolve()) for f in files):
        print("the 2026 suite is the holdout and is not analysed", file=sys.stderr)
        return 2
    stems = args.stems.split(",") if args.stems else None
    jobs = [(_stem(f), str(f), args.max_clauses, args.seconds, args.order_min) for f in files
            if not stems or any(t in _stem(f) for t in stems)]
    jobs.sort(key=lambda j: -Path(j[1]).stat().st_size)
    with open(args.out, "w", newline="") as fh, ProcessPoolExecutor(max_workers=args.workers, initializer=_limit_memory,
                                                                   initargs=(args.mem_mb,)) as ex:
        w = csv.DictWriter(fh, fieldnames=CHAIN_COLUMNS, delimiter="\t", extrasaction="ignore", lineterminator="\n")
        w.writeheader()
        fh.flush()
        futs = {ex.submit(chains_one, j): j for j in jobs}
        for fut in as_completed(futs):
            try:
                row = fut.result()
            except Exception as e:  # noqa: BLE001 (a worker killed by the memory limit)
                row = {"stem": futs[fut][0], "error": f"worker: {type(e).__name__}"}
            w.writerow(row)
            fh.flush()
            print(f"{row['stem'][:64]:64} order groups {row.get('order_groups', '-')} ({row.get('order_sizes', '')}) "
                  f"{row.get('error', '')} {row.get('seconds', '')}s", flush=True)
    return 0


def cmd_one(args):
    row = analyse(args.file, args.max_clauses, args.seconds, args.order, args.order_min)
    for k in COLUMNS + (ORDER_COLUMNS if args.order else []):
        if row.get(k) not in (None, ""):
            print(f"{k:18} {row[k]}")
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------


def _families():
    fam = {}
    if FAMILIES.is_file():
        for line in FAMILIES.read_text().splitlines()[1:]:
            p = line.split("\t")
            if p[0] == "sat-comp-2025":
                fam[p[1]] = p[3]
    return fam


def _unsolved(run_dir):
    """stem -> seeds (of pts0, pts1) on which the cell is unsolved; a cell
    missing from either arm's results gets None (unknown).  Both arms'
    results must exist."""
    out, seen = {}, Counter()
    for arm in ("pts0", "pts1"):
        path = Path(run_dir) / arm / "results.tsv"
        if not path.is_file():
            raise SystemExit(f"no results for arm {arm}: {path}")
        with open(path) as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                inst = r["instance"]        # already a stem; only a full file name loses its suffix
                stem = inst[:-len(".cnf.xz")] if inst.endswith(".cnf.xz") else inst
                out[stem] = out.get(stem, 0) + (r["result"] not in ("SATISFIABLE", "UNSATISFIABLE"))
                seen[stem] += 1
    return defaultdict(lambda: None, {k: (v if seen[k] == 2 else None) for k, v in out.items()})


def _f(r, k):
    try:
        return float(r.get(k) or 0)
    except ValueError:
        return 0.0


def _median(xs):
    xs = sorted(xs)
    return xs[len(xs) // 2] if xs else 0


# G.8's targets (lockchart, oddball) and the families the bead names to be read
TARGET_FAMILIES = ("lockchart", "oddball", "timetable", "mp1", "argumentation", "rbsat", "frb", "case")


def _computed(r):
    p = _f(r, "present")
    return (_f(r, "defs") + _f(r, "gates") + _f(r, "helpers")) / p if p else 0.0


def _line(r, fam, uns):
    u = uns[r["stem"]]
    return (f"{r['stem'][33:][:44]:44} {fam.get(r['stem'], '?')[:13]:13} uns {'?' if u is None else u}/2 "
            f"groups {r.get('groups')} ({r.get('group_sizes')}; amo {r.get('amo_groups')}) free {r.get('free')} "
            f"bits {r.get('bits')} defs {r.get('defs')} gates {r.get('gates')} helpers {r.get('helpers')} "
            f"constraints {r.get('constraints')} of {int(_f(r, 'clauses')) - int(_f(r, 'satisfied'))} ({r.get('con_lens')}) "
            f"falls {r.get('sim_false')}" + (f" {r.get('error')}" if r.get("error") else ""))


def cmd_report(args):
    with open(args.table) as fh:
        rows = list(csv.DictReader(fh, delimiter="\t"))
    fam = _families()
    uns = _unsolved(args.run)
    print(f"{len(rows)} cells; unsolved seeds from {args.run} (pts0, pts1)")
    errs = Counter((r.get("error") or "").split(":")[0] for r in rows if r.get("error"))
    print("not read:", dict(errs) or "none")
    bad = [r["stem"] for r in rows if _f(r, "sim_bad") > 0]
    print("cells where the simulation found a covered clause false:", bad or "none")
    print("truncated:", sum(1 for r in rows if r.get("truncated") == "1"))
    by = defaultdict(list)
    for r in rows:
        by[fam.get(r["stem"], "?")].append(r)
    print("\nper family, split by the run of record: solved on both seeds / unsolved on at least one / unknown "
          "(missing from an arm's results); medians:")
    print(f"  {'family':22} {'part':8} {'cells':>5} {'seeds':>5} {'groups':>6} {'amo':>4} {'free':>6} {'bits':>7} "
          f"{'choice%':>7} {'comput%':>7} {'constr%':>7} {'falls%':>6}")

    def part_line(name, part, rs):
        ok = [r for r in rs if not r.get("error")]
        if not rs:
            return
        ns = sum(uns[r["stem"]] or 0 for r in rs)
        print(f"  {name[:22]:22} {part:8} {len(rs):5} {ns:5} {_median([_f(r, 'groups') for r in ok]):6.0f} "
              f"{_median([_f(r, 'amo_groups') for r in ok]):4.0f} {_median([_f(r, 'free') for r in ok]):6.0f} "
              f"{_median([_f(r, 'bits') for r in ok]):7.0f} {100 * _median([_f(r, 'choice_share') for r in ok]):7.2f} "
              f"{100 * _median([_computed(r) for r in ok]):7.1f} {100 * _median([_f(r, 'constraint_share') for r in ok]):7.2f} "
              f"{100 * _median([_f(r, 'sim_false_share') for r in ok]):6.2f}"
              + (f"  ({len(rs) - len(ok)} not read)" if len(ok) < len(rs) else ""))

    for f, rs in sorted(by.items(), key=lambda kv: (-sum(uns[r['stem']] or 0 for r in kv[1]), kv[0])):
        sol = [r for r in rs if uns[r["stem"]] == 0]
        un = [r for r in rs if (uns[r["stem"]] or 0) > 0]
        unk = [r for r in rs if uns[r["stem"]] is None]
        part_line(f, "solved", sol)
        part_line(f if not sol else "", "unsolved", un)
        part_line(f if not (sol or un) else "", "unknown", unk)     # not in both arms' results
    print("\na few choices and what they fix: choices under 5% of the variables, at least half of the variables "
          "computed, constraint clauses under 10% of the clauses (unsolved cells in full, solved ones counted):")
    few = [r for r in rows if not r.get("error") and _f(r, "present") and _f(r, "choice_share") < 0.05
           and _computed(r) >= 0.5 and _f(r, "constraint_share") < 0.10]
    print("  solved on both seeds, per family:", dict(Counter(fam.get(r["stem"], "?") for r in few if uns[r["stem"]] == 0)))
    for r in sorted(few, key=lambda r: (fam.get(r["stem"], "?"), r["stem"])):
        if uns[r["stem"]] != 0:
            print("  " + _line(r, fam, uns))
    print("\nthe target cells of G.8 and the cells the bead names:")
    for r in sorted(rows, key=lambda r: (fam.get(r["stem"], "?"), r["stem"])):
        if fam.get(r["stem"]) in TARGET_FAMILIES or (args.stems and any(t in r["stem"] for t in args.stems.split(","))):
            print("  " + _line(r, fam, uns))
    return 0


# ---------------------------------------------------------------------------
# self test: hand-written encodings
# ---------------------------------------------------------------------------


class _Vars:
    def __init__(self):
        self.n = 0

    def new(self):
        self.n += 1
        return self.n


def toy_tseitin(n_in=12, n_gates=60, seed=1, one_way=False, unit=True):
    """A random circuit of AND, OR and XOR gates; full Tseitin, or one-way
    (Plaisted-Greenbaum: each gate only in the direction(s) its use needs).
    Returns (nv, clauses, inputs, gate outputs, a planted model: random
    inputs and every gate's value)."""
    rng = random.Random(seed)
    V = _Vars()
    ins = [V.new() for _ in range(n_in)]
    sig = list(ins)
    gates = []
    for _ in range(n_gates):
        a, b = rng.sample(sig[-24:], 2)
        kind = rng.choice(("and", "or", "xor"))
        o = V.new()
        gates.append((o, kind, a * rng.choice((1, -1)), b * rng.choice((1, -1))))
        sig.append(o)
    top = gates[-1][0]
    # polarity of every signal: +1 used positively, -1 negatively, 2 both
    pol = defaultdict(set)
    pol[top].add(1)
    for o, kind, a, b in reversed(gates):
        for s in pol[o]:
            for x in (a, b):
                if kind == "xor":
                    pol[abs(x)].update((1, -1))
                else:
                    pol[abs(x)].add(s if x > 0 else -s)
    cl = []
    for o, kind, a, b in gates:
        ps = pol[o] if one_way else {1, -1}
        if kind == "and":
            if 1 in ps:
                cl += [(-o, a), (-o, b)]
            if -1 in ps:
                cl += [(o, -a, -b)]
        elif kind == "or":
            if 1 in ps:
                cl += [(-o, a, b)]
            if -1 in ps:
                cl += [(o, -a), (o, -b)]
        else:
            if 1 in ps:
                cl += [(-o, a, b), (-o, -a, -b)]
            if -1 in ps:
                cl += [(o, -a, b), (o, a, -b)]
    plant = {v: rng.random() < 0.5 for v in ins}
    val = dict(plant)

    def lv(x):
        return val[abs(x)] if x > 0 else not val[abs(x)]

    for o, kind, a, b in gates:
        val[o] = (lv(a) and lv(b)) if kind == "and" else (lv(a) or lv(b)) if kind == "or" else (lv(a) != lv(b))
    if unit:
        cl.append((top if val[top] else -top,))
    return V.n, cl, ins, [g[0] for g in gates], val


def toy_csp(n=14, d=5, density=0.5, tight=0.3, seed=2, amo="pairwise"):
    """A direct-encoded CSP with a planted solution: n variables over d
    values (at-least-one plus at-most-one per variable), random binary
    nogoods that spare the plant.  amo: pairwise, or ladder (a sequential
    at-most-one through d - 1 auxiliary variables per variable)."""
    rng = random.Random(seed)
    V = _Vars()
    x = [[V.new() for _ in range(d)] for _ in range(n)]
    cl = []
    for i in range(n):
        cl.append(tuple(x[i]))
        if amo == "pairwise":
            for a in range(d):
                for b in range(a + 1, d):
                    cl.append((-x[i][a], -x[i][b]))
        else:
            s = [V.new() for _ in range(d - 1)]
            for a in range(d - 1):
                cl.append((-x[i][a], s[a]))
                if a:
                    cl.append((-s[a - 1], s[a]))
                cl.append((-s[a], -x[i][a + 1]))
    plant = [rng.randrange(d) for _ in range(n)]
    nogoods = 0
    for i in range(n):
        for j in range(i + 1, n):
            if rng.random() >= density:
                continue
            for a in range(d):
                for b in range(d):
                    if (a, b) != (plant[i], plant[j]) and rng.random() < tight:
                        cl.append((-x[i][a], -x[j][b]))
                        nogoods += 1
    return V.n, cl, x, plant, nogoods


def toy_lockchart(n_keys=10, n_locks=8, pins=4, depths=3, seed=3):
    """A small lockchart in the shape of the 2025 cells: key depth groups
    (pairwise exactly-one), adjacent pins at most one depth apart, locks as
    two-way ORs over the keys that must open them, helpers for the keys that
    must not open a lock.  The must-open relation comes from planted keys,
    so the formula is satisfiable."""
    rng = random.Random(seed)
    V = _Vars()
    key = [[[V.new() for _ in range(depths)] for _ in range(pins)] for _ in range(n_keys)]
    lock = [[[V.new() for _ in range(depths)] for _ in range(pins)] for _ in range(n_locks)]
    plant = []
    for k in range(n_keys):
        d = [rng.randrange(depths)]
        for _ in range(pins - 1):
            d.append(max(0, min(depths - 1, d[-1] + rng.choice((-1, 0, 1)))))
        plant.append(d)
    opens = [[rng.random() < 0.3 for _ in range(n_locks)] for _ in range(n_keys)]
    for l in range(n_locks):
        if not any(opens[k][l] for k in range(n_keys)):
            opens[rng.randrange(n_keys)][l] = True
    accept = [[{plant[k][p] for k in range(n_keys) if opens[k][l]} for p in range(pins)] for l in range(n_locks)]
    cl = []
    for k in range(n_keys):
        for p in range(pins):
            cl.append(tuple(key[k][p]))
            for a in range(depths):
                for b in range(a + 1, depths):
                    cl.append((-key[k][p][a], -key[k][p][b]))
        for p in range(pins - 1):
            for a in range(depths):
                for b in range(depths):
                    if abs(a - b) > 1:
                        cl.append((-key[k][p][a], -key[k][p + 1][b]))
    for l in range(n_locks):
        for p in range(pins):
            for d in range(depths):
                ks = [key[k][p][d] for k in range(n_keys) if opens[k][l]]
                cl.append((-lock[l][p][d],) + tuple(ks))
                for kv in ks:
                    cl.append((lock[l][p][d], -kv))
    n_must_not = 0
    n_helpers = 0
    for k in range(n_keys):
        for l in range(n_locks):
            if opens[k][l]:
                continue
            # the planted key must not open it: some pin depth outside the lock's
            if all(plant[k][p] in accept[l][p] for p in range(pins)):
                continue          # the plant opens it: leave the pair unconstrained
            hs = []
            for p in range(pins):
                for d in range(depths):
                    h = V.new()
                    hs.append(h)
                    cl.append((-h, key[k][p][d]))
                    cl.append((-h, -lock[l][p][d]))
            cl.append(tuple(hs))
            n_must_not += 1
            n_helpers += len(hs)
    adj = sum(1 for a in range(depths) for b in range(depths) if abs(a - b) > 1) * (pins - 1) * n_keys
    return V.n, cl, key, plant, {"groups": n_keys * pins, "defs": n_locks * pins * depths,
                                 "helpers": n_helpers, "constraints": n_must_not + adj}


def toy_guarded(n=10, d=5, tight=0.3, seed=8):
    """Slots that take none or one of d values: a guard g_i that needs a
    value (-g_i | x_i1 | ... | x_id), the values pairwise exclusive, two
    neighbouring slots not both empty (g_i | g_i+1), and nogoods between the
    values of different slots that spare a planted solution."""
    rng = random.Random(seed)
    V = _Vars()
    g = [V.new() for _ in range(n)]
    x = [[V.new() for _ in range(d)] for _ in range(n)]
    cl = []
    for i in range(n):
        cl.append((-g[i],) + tuple(x[i]))
        for a in range(d):
            for b in range(a + 1, d):
                cl.append((-x[i][a], -x[i][b]))
    for i in range(n - 1):
        cl.append((g[i], g[i + 1]))
    plant = [rng.randrange(d) if i % 2 == 0 else None for i in range(n)]
    nogoods = 0
    for i in range(n):
        for j in range(i + 1, n):
            for a in range(d):
                for b in range(d):
                    if (a, b) != (plant[i], plant[j]) and rng.random() < tight / 4:
                        cl.append((-x[i][a], -x[j][b]))
                        nogoods += 1
    val = {g[i]: plant[i] is not None for i in range(n)}
    for i in range(n):
        for a in range(d):
            val[x[i][a]] = plant[i] == a
    return V.n, cl, val, {"groups": n, "constraints": (n - 1) + nogoods}


def toy_order(n=8, d=4, k_pos=24, seed=21, direct=False):
    """Numbers stored as sorted bits, with tests tied to them one way, in the
    shape of the pyramid cells (Z3's encoding), small.  n numbers of d
    values, each d - 1 bits with the chain bit k+1 -> bit k.  k_pos
    positions, each a triple of numbers; the planted values give every
    position another triple of values, and the puzzle is: each of those
    k_pos value triples sits at exactly one position.
    A tile T(position, triple) is true exactly when the position's three
    numbers have that triple.  It is written in pieces, as Z3 does: T -> the
    bits that say so (two-literal clauses), and back through a helper h:
    (T | h | not "first number has its value") and (-h | not "the other two
    have theirs").  No clause is the tile's definition, yet propagation
    fixes every tile from the bits.  With direct, the reverse is one clause
    (T | not all six bits) and the tile is a plain two-way AND.
    Each triple exactly once: one long clause and all the pairs.
    The draw is repeated until every number meets every value in some tile.
    Returns (nv, clauses, the numbers' bit variables, the planted values,
    a planted model, what the reading must find)."""
    rng = random.Random(seed)
    V = _Vars()
    bits = [[V.new() for _ in range(d - 1)] for _ in range(n)]
    while True:
        plant = [rng.randrange(d) for _ in range(n)]
        triples = [(i, j, k) for i in range(n) for j in range(i + 1, n) for k in range(j + 1, n)]
        rng.shuffle(triples)
        positions, seen = [], set()
        for (i, j, k) in triples:
            t = (plant[i], plant[j], plant[k])
            if t not in seen:
                seen.add(t)
                positions.append((i, j, k))
            if len(positions) == k_pos:
                break
        if len(positions) < k_pos:
            continue
        # every bit of every number is read by some tile (a bit that nothing reads is no
        # part of the puzzle: the reading would rightly leave it out of the number)
        vals = [set() for _ in range(n)]
        for pos3 in positions:
            for slot, i in enumerate(pos3):
                vals[i].update(t[slot] for t in ((plant[a], plant[b], plant[c]) for (a, b, c) in positions))
        if all(len(v) == d for v in vals):
            break
    targets = [(plant[i], plant[j], plant[k]) for (i, j, k) in positions]
    cl = []
    for b in bits:
        for k in range(d - 2):
            cl.append((-b[k + 1], b[k]))

    def says(i, a):
        """The literals that say "number i has the value a"."""
        out = []
        if a > 0:
            out.append(bits[i][a - 1])
        if a < d - 1:
            out.append(-bits[i][a])
        return out

    model = {}
    for i in range(n):
        for k in range(d - 1):
            model[bits[i][k]] = plant[i] > k
    tile = {}
    for p, (i, j, k) in enumerate(positions):
        for t, (a, b, c) in enumerate(targets):
            T = V.new()
            tile[(p, t)] = T
            la, lbc = says(i, a), says(j, b) + says(k, c)
            for x in la + lbc:
                cl.append((-T, x))
            on_bc = plant[j] == b and plant[k] == c
            model[T] = plant[i] == a and on_bc
            if direct:
                cl.append((T,) + tuple(-x for x in la + lbc))
            else:
                h = V.new()
                cl.append((T, h) + tuple(-x for x in la))
                cl.append((-h,) + tuple(-x for x in lbc))
                model[h] = not on_bc
    for t in range(k_pos):
        ts = [tile[(p, t)] for p in range(k_pos)]
        cl.append(tuple(ts))
        for a in range(k_pos):
            for b in range(a + 1, k_pos):
                cl.append((-ts[a], -ts[b]))
    return V.n, cl, bits, plant, model, {"order_groups": n, "order_sizes": f"{d - 1}x{n}", "counts": k_pos,
                                         "tiles": k_pos * k_pos}


def toy_random3(n=200, ratio=4.0, seed=4):
    rng = random.Random(seed)
    cl = []
    for _ in range(int(n * ratio)):
        vs = rng.sample(range(1, n + 1), 3)
        cl.append(tuple(v * rng.choice((1, -1)) for v in vs))
    return n, cl


def toy_cycle(n=12, seed=5):
    """x_i = x_{i+1} AND c_i around a cycle (x_{n+1} = x_1), c_i inputs: one
    variable has to be freed to break the cycle."""
    V = _Vars()
    x = [V.new() for _ in range(n)]
    c = [V.new() for _ in range(n)]
    cl = []
    for i in range(n):
        a, b = x[(i + 1) % n], c[i]
        cl += [(-x[i], a), (-x[i], b), (x[i], -a, -b)]
    return V.n, cl


def _check(name, ok, detail=""):
    print(f"  {'ok  ' if ok else 'FAIL'} {name}" + (f"  ({detail})" if detail and not ok else ""))
    return ok


def _is_model(cl, assign):
    return all(any((assign.get(abs(z), False) if z > 0 else not assign.get(abs(z), False)) for z in c) for c in cl)


def _complete(fixed, rd, choice_index, free_value):
    a = dict(fixed)
    a.update(rd.full_assignment(choice_index, free_value))
    return a


def cmd_selftest(args):
    ok = True
    print("Tseitin circuit (AND, OR, XOR), output fixed by a unit")
    nv, cl, ins, outs, plant = toy_tseitin()
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    used = {abs(z) for c in cl for z in c}
    ok &= _check("no group, the inputs in use are the free variables",
                 row["groups"] == 0 and row["free"] == len(used & set(ins)),
                 f"groups {row['groups']} free {row['free']} inputs in use {len(used & set(ins))}")
    ok &= _check("every gate but the fixed output is defined", row["defs"] + row["gates"] + row["fixed"] == len(outs),
                 f"defs {row['defs']} gates {row['gates']} fixed {row['fixed']} of {len(outs)}")
    freed = {v for v in rd.order if rd.role[v] == FREE}
    ok &= _check("the free variables are the inputs in use, no helper", freed == used & set(ins) and row["helpers"] == 0,
                 f"free {sorted(freed)} helpers {row['helpers']}")
    ok &= _check("the simulation finds no covered clause false", row["sim_bad"] == 0, str(row["sim_bad"]))
    ok &= _check("the planted model's free part, computed through the reading, gives a model", _plant_ok(nv, cl, plant))
    nv0, cl0, _i, _o, plant0 = toy_tseitin(unit=False)
    row, rd = analyse_cnf(CNF.from_clauses(nv0, cl0), {})
    ok &= _check("without the unit: no constraint clause at all", row["constraints"] == 0, str(row["constraints"]))

    print("one-way (Plaisted-Greenbaum) circuit")
    nv, cl, ins, outs, plant = toy_tseitin(one_way=True, seed=6)
    fixed, rd = _fixed_rd(nv, cl)
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    left = {abs(z) for c in rd.cnf.lits for z in (c,)} & set(ins)
    ok &= _check("no more free variables than inputs left after the units", row["free"] <= len(left),
                 f"free {row['free']} inputs left {len(left)}")
    ok &= _check("one-way gates read as helpers", row["helpers"] > 0 and row["sim_bad"] == 0,
                 f"helpers {row['helpers']} bad {row['sim_bad']}")
    ok &= _check("the planted model's free part, computed through the reading, gives a model", _plant_ok(nv, cl, plant))

    print("direct-encoded CSP, pairwise at-most-one")
    nv, cl, x, plant, nog = toy_csp()
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    ok &= _check("one group per CSP variable, of the domain size", row["groups"] == len(x) and row["group_sizes"] == f"{len(x[0])}x{len(x)}",
                 f"{row['groups']} {row['group_sizes']}")
    ok &= _check("nothing free, nothing defined; the nogoods are the constraints",
                 row["free"] == 0 and row["defs"] == 0 and row["constraints"] == nog and row["con_choice"] == nog,
                 f"free {row['free']} defs {row['defs']} constraints {row['constraints']} of {nog}")
    idx = _plant_index(rd, x, plant)
    ok &= _check("the planted values give a model", idx is not None and _is_model(cl, _complete(*_fixed_rd(nv, cl), idx, {})))

    print("direct-encoded CSP, sequential (ladder) at-most-one")
    nv, cl, x, plant, nog = toy_csp(amo="ladder", seed=7)
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    ok &= _check("one chained group per CSP variable", row["groups"] == len(x) and row["chain_groups"] == len(x),
                 f"groups {row['groups']} chain {row['chain_groups']}")
    ok &= _check("the ladder variables are computed, none free", row["free"] == 0 and row["sim_bad"] == 0,
                 f"free {row['free']} bad {row['sim_bad']}")
    idx = _plant_index(rd, x, plant)
    ok &= _check("the planted values give a model", idx is not None and _is_model(cl, _complete(*_fixed_rd(nv, cl), idx, {})))

    print("small lockchart")
    nv, cl, key, plant, exp = toy_lockchart()
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    ok &= _check("key groups, lock definitions, helpers as built",
                 (row["groups"], row["defs"], row["helpers"], row["free"]) == (exp["groups"], exp["defs"], exp["helpers"], 0),
                 f"groups {row['groups']} defs {row['defs']} helpers {row['helpers']} free {row['free']} expected {exp}")
    ok &= _check("constraints: the must-not-open clauses and the adjacent-pin pairs", row["constraints"] == exp["constraints"],
                 f"{row['constraints']} expected {exp['constraints']} ({row['con_lens']})")
    ok &= _check("the simulation finds no covered clause false", row["sim_bad"] == 0, str(row["sim_bad"]))
    flat = [d for k in range(len(key)) for d in plant[k]]
    idx = _plant_index(rd, [key[k][p] for k in range(len(key)) for p in range(len(key[0]))], flat)
    ok &= _check("every key pin is a group", idx is not None)
    if idx is not None:
        a = _complete(*_fixed_rd(nv, cl), idx, {})
        ok &= _check("the planted keys, computed through the reading, give a model", _is_model(cl, a))

    print("slots of at most one value (a guard needs one)")
    nv, cl, plant, exp = toy_guarded()
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    ok &= _check("one at-most-one group per slot, the guards computed",
                 row["groups"] == exp["groups"] and row["amo_groups"] == exp["groups"] and row["free"] == 0,
                 f"groups {row['groups']} amo {row['amo_groups']} free {row['free']} helpers {row['helpers']}")
    ok &= _check("constraints: the neighbour clauses and the nogoods", row["constraints"] == exp["constraints"],
                 f"{row['constraints']} expected {exp['constraints']} ({row['con_lens']})")
    ok &= _check("the simulation (none among the values) finds no covered clause false", row["sim_bad"] == 0, str(row["sim_bad"]))
    ok &= _check("the planted slots, computed through the reading, give a model", _plant_ok(nv, cl, plant))

    print("a root conflict")
    row, rd = analyse_cnf(CNF.from_clauses(3, [(1,), (-1, 2), (-2, 3), (-3,)]), {})
    ok &= _check("unit propagation refutes it: no reading, an error row", rd is None and row.get("error") == "root-conflict",
                 str(row))

    print("reading files")
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        def put(name, text):
            path = os.path.join(tmp, name)
            with open(path, "w") as fh:
                fh.write(text)
            return path
        row = analyse(put("empty.cnf", "p cnf 2 2\n1 2 0\n0\n"))
        ok &= _check("an empty clause is kept and refutes the formula", row.get("error") == "root-conflict", str(row))
        cnf, trunc = read_cnf(put("satlib.cnf", "c x\np cnf 3 2\n1 -2 0\n2 3 0\n%\n0\n\n"))
        ok &= _check("a SATLIB '%' line ends the formula", (cnf.m, cnf.empty, trunc) == (2, 0, False),
                     f"clauses {cnf.m} empty {cnf.empty} truncated {trunc}")
        row = analyse(put("cut.cnf", "p cnf 2 2\n1 2 0\n0\n"), max_clauses=1)
        ok &= _check("a clause limit that leaves out an empty clause says truncated", row["truncated"] == 1, str(row))
        row = analyse(put("three.cnf", "p cnf 3 3\n1 0\n2 0\n3 0\n"), max_clauses=1)
        ok &= _check("a clause limit below the prefix size is kept", (row["clauses"], row["truncated"]) == (1, 1),
                     f"clauses {row['clauses']} truncated {row['truncated']}")
    a, b, c = (1 << 26) + 1, (1 << 30) + 5, (1 << 31) - 1
    fo = free_order([(a, 0, 5, 10), (b, 0, 7, 3), (c, 1, 100, 100), (3, 0, 7, 3)])
    got = [fo.pop() for _ in range(len(fo))]
    ok &= _check("the dead-lock order keeps variable numbers above 2^26 whole", got == [3, b, a, c], str(got))

    print("numbers stored as sorted bits, tests tied one way (order groups; needs --order)")
    nv, cl, bits, plant, model, exp = toy_order()
    ok &= _check("the planted model is a model", _is_model(cl, model))
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    ok &= _check("without --order: no order group, and the simulation finds no covered clause false",
                 row.get("order_groups", 0) == 0 and row["sim_bad"] == 0, f"{row.get('order_groups')} bad {row['sim_bad']}")
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {}, order=True)
    ok &= _check("one order group per number, of its bits",
                 (row["order_groups"], row["order_sizes"]) == (exp["order_groups"], exp["order_sizes"]),
                 f"{row['order_groups']} {row['order_sizes']} expected {exp}")
    ok &= _check("the row says among how many inputs they were found, and the share of false constraints counts the counts",
                 row["order_inputs"] >= 3 * exp["order_groups"] and 0 < row["sim_false_share"] <= 1
                 and abs(row["sim_false_share"] - row["sim_false"] / (row["constraints"] + row["counts"])) < 1e-3,
                 f"inputs {row['order_inputs']} share {row['sim_false_share']} mean {row['sim_false']}")
    got_bits = sorted(sorted(abs(x) for x in mem) for mem, o in zip(rd.groups, rd.ordered) if o)
    ok &= _check("the order groups are the numbers' bits, lowest bit first",
                 got_bits == sorted(sorted(b) for b in bits)
                 and all(list(mem) == sorted(mem) and all(x > 0 for x in mem) for mem, o in zip(rd.groups, rd.ordered) if o),
                 str(got_bits[:3]))
    ok &= _check("propagation finds every tile as a test on its three numbers, or as one no model has on",
                 rd.links["product"] + rd.links["never"] == exp["tiles"] and rd.links["product"] >= exp["counts"],
                 str(dict(rd.links)))
    ok &= _check("the numbers are the only choices: no other group, nothing free",
                 row["groups"] == exp["order_groups"] and row["free"] == 0, f"groups {row['groups']} free {row['free']}")
    ok &= _check("each triple exactly once is a count constraint", row["counts"] == exp["counts"], f"counts {row['counts']}")
    red = rd.cnf
    left = [c for c in range(red.m) if not rd.cover[c]]
    ok &= _check("the clauses of the counts' encodings are covered by them: no constraint clause left holds a tile "
                 "(what is left says which triples a position cannot hold)",
                 row["cov_count"] >= row["counts"] and all(rd.role[abs(z)] != DEF for c in left for z in red.clause(c)),
                 f"covered {row['cov_count']}, constraints {len(left)}")
    ok &= _check("the simulation finds no covered clause false", row["sim_bad"] == 0, str(row["sim_bad"]))
    ok &= _check("the planted numbers, computed through the reading, give a model", _plant_ok(nv, cl, model, order=True))
    nv2, cl2, bits2, _plant2, model2, exp2 = toy_order(direct=True, seed=22)
    row, rd = analyse_cnf(CNF.from_clauses(nv2, cl2), {}, order=True)
    ok &= _check("tiles written as plain two-way ANDs: the numbers are read the same way",
                 (row["order_groups"], row["free"], row["sim_bad"]) == (exp2["order_groups"], 0, 0)
                 and exp2["counts"] - 2 <= row["counts"] <= exp2["counts"] and _plant_ok(nv2, cl2, model2, order=True),
                 f"{row['order_groups']} counts {row['counts']} free {row['free']} bad {row['sim_bad']}")
    # a chain of helpers is no number: three variables in a row, each in two small clauses
    row, rd = analyse_cnf(CNF.from_clauses(6, [(-3, 2), (-2, 1), (1, 4), (-3, 5, 6), (3, -5), (2, -6)]), {}, order=True)
    ok &= _check("a short chain of variables that could each be a helper is not read as a number",
                 row.get("order_groups", 0) == 0 and row["sim_bad"] == 0, str(row.get("order_groups")))

    print("negatives")
    nv, cl = toy_random3()
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    ok &= _check("random 3-SAT: no group, no definition, most clauses constraints",
                 row["groups"] == 0 and row["defs"] == 0 and row["gates"] == 0 and row["constraint_share"] > 0.7,
                 f"groups {row['groups']} defs {row['defs']} gates {row['gates']} share {row['constraint_share']}")
    ok &= _check("random 3-SAT: the simulation finds no covered clause false", row["sim_bad"] == 0, str(row["sim_bad"]))
    nv, cl = toy_cycle()
    row, rd = analyse_cnf(CNF.from_clauses(nv, cl), {})
    ok &= _check("a cycle of definitions: one variable freed to break it, the rest defined",
                 row["cycle_breaks"] == 1 and row["defs"] == 11, f"breaks {row['cycle_breaks']} defs {row['defs']} free {row['free']}")
    ok &= _check("the cycle: the simulation finds no covered clause false", row["sim_bad"] == 0, str(row["sim_bad"]))
    print("all ok" if ok else "SOME CHECKS FAILED")
    return 0 if ok else 1


def _fixed_rd(nv, cl):
    fixed, rd, _ = read_roles(CNF.from_clauses(nv, cl))
    return fixed, rd


def _plant_ok(nv, cl, plant, order=False):
    """The planted model's values on the free variables (and the true member
    of each group; the value of an order group), everything else computed:
    is that a model?"""
    fixed, rd, _ = read_roles(CNF.from_clauses(nv, cl), order=order)
    idx = []
    for mem, none_ok, ordered in zip(rd.groups, rd.none_ok, rd.ordered):
        t = [j for j, x in enumerate(mem) if plant.get(abs(x)) == (x > 0)]
        if ordered:
            if t != list(range(len(t))):
                return False                 # not a prefix of the members
            idx.append(len(t))
            continue
        if len(t) == 0 and none_ok:
            t = [len(mem)]
        if len(t) != 1:
            return False
        idx.append(t[0])
    free = {v: plant.get(v, False) for v in rd.order if rd.role[v] == FREE}
    return _is_model(cl, _complete(fixed, rd, idx, free))


def _plant_index(rd, groups_vars, plant):
    """The planted value of each CSP variable as the index of its member in
    the reading's groups; None when a planted member is in no group."""
    where = {}
    for g, mem in enumerate(rd.groups):
        for j, x in enumerate(mem):
            where[x] = (g, j)
    idx = [0] * len(rd.groups)
    for vs, a in zip(groups_vars, plant):
        if vs[a] not in where:
            return None
        g, j = where[vs[a]]
        idx[g] = j
    return idx


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--out", required=True)
    c.add_argument("--stems", help="comma-separated substrings of stems (default: every 2025 cell)")
    c.add_argument("--files", nargs="*", help="explicit .cnf.xz files instead of the suite")
    c.add_argument("--workers", type=int, default=16)
    c.add_argument("--max-clauses", type=int, default=MAX_CLAUSES)
    c.add_argument("--seconds", type=int, default=CELL_SECONDS)
    c.add_argument("--mem-mb", type=int, default=16000)
    c.set_defaults(func=cmd_census)
    o = sub.add_parser("one")
    o.add_argument("file")
    o.add_argument("--max-clauses", type=int, default=MAX_CLAUSES)
    o.add_argument("--seconds", type=int, default=CELL_SECONDS)
    o.set_defaults(func=cmd_one)
    for q in (c, o):
        q.add_argument("--order", action="store_true",
                       help="read numbers stored as sorted bits as choices too (order groups); more columns")
        q.add_argument("--order-min", type=int, default=MIN_ORDER, help="members an order group has at least")
    h = sub.add_parser("chains", help="the order-group census: the order groups of every cell, nothing else read")
    h.add_argument("--out", required=True)
    h.add_argument("--stems", help="comma-separated substrings of stems (default: every 2025 cell)")
    h.add_argument("--files", nargs="*", help="explicit .cnf.xz files instead of the suite")
    h.add_argument("--workers", type=int, default=16)
    h.add_argument("--max-clauses", type=int, default=MAX_CLAUSES)
    h.add_argument("--seconds", type=int, default=CELL_SECONDS)
    h.add_argument("--mem-mb", type=int, default=16000)
    h.add_argument("--order-min", type=int, default=MIN_ORDER)
    h.set_defaults(func=cmd_chains)
    r = sub.add_parser("report")
    r.add_argument("table")
    r.add_argument("--run", default=str(RUN_OF_RECORD), help="paired-run directory whose pts0/pts1 results mark unsolved cells")
    r.add_argument("--stems", help="comma-separated substrings: print these cells in full")
    r.set_defaults(func=cmd_report)
    s = sub.add_parser("selftest")
    s.set_defaults(func=cmd_selftest)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
