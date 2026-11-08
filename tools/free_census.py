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
  python3 tools/free_census.py census --out TABLE.tsv [--stems a,b] [--files F ...] [--workers 16]
  python3 tools/free_census.py report TABLE.tsv [--run RUN_DIR]
  python3 tools/free_census.py one FILE.cnf[.xz]
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
CELL_SECONDS = 3_600       # wall budget per cell
MASK = (1 << SIM_PATTERNS) - 1

# roles of a variable
ABSENT, FIXED, CHOICE, FREE, DEF, GATE, HELPER = range(7)
ROLE_NAMES = ("absent", "fixed", "choice", "free", "def", "gate", "helper")
# covers of a clause
C_NONE, C_GROUP, C_DEF, C_GATE, C_HELPER = range(5)


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

    def __init__(self, cnf, deadline=None):
        self.cnf = cnf
        self.deadline = deadline
        nv = cnf.nv
        self.groups = []                       # member literals: at most one true, exactly one unless none_ok
        self.chain = []                        # per group: exclusions through other variables
        self.none_ok = []                      # per group: may all members be false (an at-most-one)
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
                    if z in seen:
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
                if z not in seen:
                    seen.add(z)
                    stack.append(z)
            if len(seen) > budget:
                return False
        return False

    def find_groups(self, npass, piv_c, piv_x):
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
        set that passes the cheap first-pair screen is tested once."""
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
        gid = self.gid
        is_group = {c for _m, c, _ch, _n in cands}
        members = {abs(x) for mem, _c, _ch, _n in cands for x in mem}
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
        for mem, c, chained, none_ok in cands:
            if any(defined_elsewhere(abs(x), c) for x in mem):
                self.stats["derived_groups"] += 1
                continue
            if any(gid[abs(x)] >= 0 for x in mem):
                self.stats["overlap_groups"] += 1
                continue
            g = len(self.groups)
            self.groups.append(mem)
            self.chain.append(chained)
            self.none_ok.append(none_ok)
            for x in mem:
                gid[abs(x)] = g

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
        self.find_groups(npass, piv_c, piv_x)
        self.npass = npass                  # how many variables each clause defines (the circuit-order test)
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
        return self

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
        true one, free_masks[v] the patterns in which free v is true."""
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
        for mem, none_ok in zip(self.groups, self.none_ok):
            ms = [0] * (len(mem) + 1)           # the last slot is "none"
            for p in range(SIM_PATTERNS):
                ms[rng.randrange(len(mem) + none_ok)] |= 1 << p
            cm.append(ms)
        fm = {v: rng.getrandbits(SIM_PATTERNS) for v in self.order if self.role[v] == FREE}
        val = self.evaluate(cm, fm)
        cnf = self.cnf
        lits, starts, cover = cnf.lits, cnf.starts, self.cover
        bad = 0
        false_total = 0
        for c in range(cnf.m):
            s = 0
            for z in lits[starts[c]:starts[c + 1]]:
                s |= val[z] if z > 0 else MASK ^ val[-z]
                if s == MASK:
                    break
            if s != MASK:
                if cover[c]:
                    bad += 1
                else:
                    false_total += SIM_PATTERNS - bin(s).count("1")
        return bad, false_total / SIM_PATTERNS

    def full_assignment(self, choice_index, free_value):
        """One assignment: choice_index[g] is the true member of group g
        (len(group) for none, in an at-most-one group), free_value[v] the
        value of free v; returns {var: bool}."""
        cm = [[1 if choice_index[g] == j else 0 for j in range(len(mem))] for g, mem in enumerate(self.groups)]
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
        bits = sum(math.log2(len(g) + n) for g, n in zip(self.groups, self.none_ok)) + n_free
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
            "constraints": cov[C_NONE],
            "constraint_share": round(cov[C_NONE] / cnf.m, 4) if cnf.m else 0,
            "con_choice": con_choice,
            "con_lens": " ".join(f"{k}x{v}" for k, v in sorted(con_lens.items(), key=lambda kv: -kv[1])[:6]),
        }


def read_roles(cnf, deadline=None):
    """(fixed, Reading or None on a root conflict, satisfied clause count)."""
    fixed, red, nsat = unit_propagate(cnf)
    if red is None:
        return fixed, None, 0
    return fixed, Reading(red, deadline).run(), nsat


# ---------------------------------------------------------------------------
# one cell
# ---------------------------------------------------------------------------

COLUMNS = ["stem", "vars", "clauses", "truncated", "fixed", "satisfied", "present", "groups", "group_vars", "group_sizes",
           "chain_groups", "amo_groups", "overlap_groups", "derived_groups", "group_tests_cut", "defs", "wide_defs", "def_inputs_max", "gates", "helpers", "free",
           "cycle_breaks", "bits", "choice_share", "cov_group", "cov_def", "cov_gate", "cov_helper", "constraints",
           "constraint_share", "con_choice", "con_lens", "sim_bad", "sim_false", "sim_false_share", "read_seconds",
           "stages", "seconds", "error"]


def _stem(path):
    name = Path(path).name
    for ext in (".cnf.xz", ".cnf"):
        if name.endswith(ext):
            return name[:-len(ext)]
    return name


def analyse_cnf(cnf, row, deadline=None):
    fixed, rd, nsat = read_roles(cnf, deadline)
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
    row["sim_false_share"] = round(false_mean / row["constraints"], 4) if row["constraints"] else 0
    return row, rd


def analyse(path, max_clauses=MAX_CLAUSES, seconds=CELL_SECONDS):
    t0 = time.time()
    row = {"stem": _stem(path)}
    if header_clauses(path) > max_clauses:
        max_clauses = min(max_clauses, PREFIX_CLAUSES)
    cnf, trunc = read_cnf(path, max_clauses)
    row.update(vars=cnf.nv, clauses=cnf.m, truncated=int(trunc), read_seconds=round(time.time() - t0, 1))
    analyse_cnf(cnf, row, t0 + seconds)
    row["seconds"] = round(time.time() - t0, 1)
    return row


def _alarm(_sig, _frm):
    raise TimeoutError("wall")


def census_one(job):
    stem, path, max_clauses, seconds = job
    signal.signal(signal.SIGALRM, _alarm)
    signal.alarm(int(seconds) + 300)
    t0 = time.time()
    try:
        return analyse(path, max_clauses, seconds)
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
        jobs.append((stem, str(f), args.max_clauses, args.seconds))
    jobs.sort(key=lambda j: -Path(j[1]).stat().st_size)      # the long tail starts early
    with open(args.out, "w", newline="") as fh, ProcessPoolExecutor(max_workers=args.workers, initializer=_limit_memory,
                                                                   initargs=(args.mem_mb,)) as ex:
        w = csv.DictWriter(fh, fieldnames=COLUMNS, delimiter="\t", extrasaction="ignore")
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


def cmd_one(args):
    row = analyse(args.file, args.max_clauses, args.seconds)
    for k in COLUMNS:
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


def _plant_ok(nv, cl, plant):
    """The planted model's values on the free variables (and the true member
    of each group), everything else computed: is that a model?"""
    fixed, rd = _fixed_rd(nv, cl)
    idx = []
    for mem, none_ok in zip(rd.groups, rd.none_ok):
        t = [j for j, x in enumerate(mem) if plant.get(abs(x)) == (x > 0)]
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
