#!/usr/bin/env python3
"""Checks of the grid reader, solver/13-kissat-rs/src/gridread.rs (epic
SAT-playground-iin, bead iin.8).

The solver has a test hook: with --card=1 and SAT_CARD_READ_CHECK=<file> it prints
one `c gridread` line per grid and writes every proof to <file>. With
SAT_STRUCT_DEBUG=2 it also prints what the reader charged and, per grid, the clause
behind each row and the proof lines each row and column claims to take. This tool
runs that hook and checks what comes out in ways that do not trust the reader:

- the shape of each grid: n rows, d columns, p values only one row has;
- by hand from the CNF: a row's clause is the clause it says, a column holds at most
  one literal of a row, and the line counts are the pairs that are not input clauses;
- VeriPB on the whole proof, with an `e` line per row and column saying the id the
  reader returned is exactly that constraint;
- the stock solver path (the structure passes and --card off): the formula plus the
  two literals of a claimed pair as units, or plus a row's negated literals, must be
  unsatisfiable. A base formula that is itself unsatisfiable passes this for nothing,
  so the fuzz counts those cases.

Usage:
  python3 tools/grid_read_check.py selftest [--solver BIN] [--veripb BIN] [--tmp DIR]
  python3 tools/grid_read_check.py fuzz --count 1000 [--seed 1] [--workers 4]
  python3 tools/grid_read_check.py compare --cells DIR [--veripb-cells SUBSTR ...]
  python3 tools/grid_read_check.py budget --cells DIR
  python3 tools/grid_read_check.py sweep --out TSV [--workers 4]
"""
from __future__ import annotations

import argparse
import csv
import itertools
import json
import os
import random
import re
import subprocess
import sys
import time
from collections import Counter
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from alldiff_census import Enc, flipped, toy_circuit, toy_colouring, toy_latin, toy_php, toy_sudoku, toy_xor  # noqa: E402

STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0", "--structdiv=0", "--structdd=0", "--gauss=0", "--card=0", "--structpoints=0", "--freewalk=0"]
TABLE = ROOT / "benchmarks" / "rl" / "alldiff_2025.tsv"
BIG = 8                 # a core grid of this many rows or more is a big one, as in the census


class Cfg:
    solver = str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver")
    veripb = str(ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb")
    tmp = "/tmp/grid_read_check"


# --------------------------------------------------------------- running the hook

class Grid:
    def __init__(self, rows, cols):
        self.rows = rows
        self.cols = cols
        self.clauses = []
        self.rowlines = []
        self.collines = []

    @property
    def shape(self):
        return (len(self.rows), len(self.cols), self.private)

    @property
    def private(self):
        return sum(len(r) for r in self.rows) - sum(len(c) for c in self.cols)


class Read:
    def __init__(self):
        self.grids = []
        self.unproved = None
        self.code = None
        self.begin = self.end = self.budget = 0
        self.notes = {}
        self.wall = 0.0
        self.stderr = ""

    @property
    def ticks(self):
        return self.end - self.begin

    def shapes(self):
        return Counter(g.shape for g in self.grids)


GRID_LINE = re.compile(r"c gridread rows=(\d+) cols=(\d+) rowlits=(\[.*\]) collits=(\[.*\])$")
INFO_LINE = re.compile(r"gridinfo clauses=(\[.*\]) rowlines=(\[.*\]) collines=(\[.*\])$")


def run_read(cnf, pbp=None, args=(), env_extra=None, timeout=600):
    """One run of the hook, stopped before the search."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("SAT_")}
    env["SAT_STRUCT_DEBUG"] = "2"
    env["SAT_CARD_READ_CHECK"] = str(pbp) if pbp else "/dev/null"
    if env_extra:
        env.update(env_extra)
    t0 = time.time()
    p = subprocess.run([Cfg.solver, "-q", *STOCK, "--card=1", *args, "--conflicts=0", str(cnf)], capture_output=True, text=True, timeout=timeout, env=env)
    r = Read()
    r.wall = time.time() - t0
    r.code = p.returncode
    r.stderr = p.stderr
    for line in p.stdout.splitlines():
        m = GRID_LINE.match(line)
        if m:
            r.grids.append(Grid(json.loads(m.group(3)), json.loads(m.group(4))))
        elif line.startswith("c cardread unproved="):
            r.unproved = int(line.split("=")[1])
    k = 0
    for line in p.stderr.splitlines():
        m = INFO_LINE.match(line)
        if m:
            g = r.grids[k]
            g.clauses, g.rowlines, g.collines = json.loads(m.group(1)), json.loads(m.group(2)), json.loads(m.group(3))
            k += 1
        elif line.startswith("gridread: "):
            t = line.split()[1:]
            r.notes = {t[i]: t[i + 1] for i in range(0, len(t) - 1, 2)}
            r.begin, r.end, r.budget = int(r.notes["begin"]), int(r.notes["end"]), int(r.notes["budget"])
    return r


def write_cnf(path, clauses, nv=None):
    nv = nv if nv is not None else max((abs(l) for c in clauses for l in c), default=0)
    with open(path, "w") as f:
        f.write(f"p cnf {nv} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


# ------------------------------------------------------- checks that do not trust it

def pair_ids(clauses):
    """(a, b) -> id of the input clause (~a v ~b), when it is written just so."""
    ids = {}
    for i, c in enumerate(clauses):
        if len(c) == 2 and c[0] != c[1] and c[0] != -c[1]:
            ids.setdefault(frozenset((-c[0], -c[1])), i + 1)
    return ids


def structure_errors(r, clauses):
    """What is wrong with the grids, read by hand from the CNF."""
    errors = []
    if r.code not in (0, 10, 20):
        errors.append(f"exit code {r.code}")
    if "panicked" in r.stderr:
        errors.append("a panic")
    if r.unproved not in (0, None):
        errors.append(f"unproved {r.unproved}")
    if int(r.notes.get("rejected", 0)) != 0:
        errors.append(f"the check rejected {r.notes['rejected']} grids: detection and check disagree")
    pairs = pair_ids(clauses)
    last = None
    for gi, g in enumerate(r.grids):
        if len(g.rows) < 3:
            errors.append(f"grid {gi}: {len(g.rows)} rows")
        if last is not None and len(g.rows) > last:
            errors.append(f"grid {gi}: not biggest first")
        last = len(g.rows)
        owner = {}
        for ri, row in enumerate(g.rows):
            if len(set(row)) != len(row) or any(-l in row for l in row):
                errors.append(f"grid {gi} row {ri}: a repeated literal or both signs")
            for l in row:
                owner.setdefault(l, []).append(ri)
            i = g.clauses[ri] if g.clauses else -1
            if i >= 0:
                raw = clauses[i]
                if sorted(raw) != sorted(row):
                    errors.append(f"grid {gi} row {ri}: clause {i} is {raw}, not {row}")
        seen = set()
        for ci, col in enumerate(g.cols):
            if len(col) < 2:
                errors.append(f"grid {gi} column {ci}: one literal")
            rows_here = []
            for l in col:
                if l in seen:
                    errors.append(f"grid {gi}: literal {l} in two columns")
                seen.add(l)
                if len(owner.get(l, ())) != 1:
                    errors.append(f"grid {gi}: literal {l} of a column is in {len(owner.get(l, ()))} rows")
                else:
                    rows_here.append(owner[l][0])
            if len(rows_here) != len(set(rows_here)):
                errors.append(f"grid {gi} column {ci}: two literals of one row")
            if g.collines:
                rups = sum(1 for a, b in itertools.combinations(col, 2) if frozenset((a, b)) not in pairs)
                if g.collines[ci] != rups + (len(col) >= 3):
                    errors.append(f"grid {gi} column {ci}: {g.collines[ci]} lines claimed, {rups} pairs with no input clause")
    return errors


def literal(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


def veripb_check(cnf, pbp, clauses, r, timeout=3000):
    """VeriPB on the proof with an `e` line per row and column: the id the reader
    returned is exactly that constraint. The hook writes the count reader's lines
    first and then, in order, what each grid's rows with no clause and its columns
    take; the ids are counted here from the file. Returns an error or None."""
    with open(pbp) as f:
        lines = f.read().splitlines()
    if len(lines) < 4 or not lines[-1].startswith("end pseudo"):
        return f"the proof file ends with {lines[-1:]}"
    footer = lines[-3:]
    body = lines[:-3]
    total = sum(sum(g.rowlines) + sum(g.collines) for g in r.grids)
    start = len(body) - total
    next_id = len(clauses)
    in_block = False
    for line in body[1:start]:
        s = line.strip()
        if s.startswith("pbc"):
            in_block = True
            next_id += 1
        elif s == "qed;":
            in_block = False
            next_id += 1
        elif s.startswith(("pol", "rup", "red")):
            next_id += 1
    pairs = pair_ids(clauses)
    checks = []
    at = start

    def take(n):
        nonlocal at, next_id
        last = None
        for _ in range(n):
            if not body[at].startswith(("rup", "pol")):
                raise ValueError(f"line {at}: {body[at][:60]}")
            next_id += 1
            last = next_id
            at += 1
        return last

    try:
        for g in r.grids:
            for ri, row in enumerate(g.rows):
                terms = " ".join(f"1 {literal(l)}" for l in row)
                if g.clauses[ri] >= 0:
                    checks.append(f"e {terms} >= 1 : {g.clauses[ri] + 1} ;")
                else:
                    checks.append(f"e {terms} >= 1 : {take(g.rowlines[ri])} ;")
            for ci, col in enumerate(g.cols):
                terms = " ".join(f"1 {literal(-l)}" for l in col)
                got = take(g.collines[ci])
                if got is None:
                    got = pairs[frozenset(col)]
                checks.append(f"e {terms} >= {len(col) - 1} : {got} ;")
    except (ValueError, IndexError, KeyError) as e:
        return f"the proof's lines do not match what the reader said: {e!r}"
    if at != len(body):
        return f"{len(body) - at} proof lines more than the reader said"
    cut = str(pbp) + ".e"
    with open(cut, "w") as f:
        f.write("\n".join(body + checks + footer) + "\n")
    p = subprocess.run([Cfg.veripb, str(cnf), cut], capture_output=True, text=True, timeout=timeout)
    os.unlink(cut)
    if "s VERIFIED NO CONCLUSION" not in p.stdout:
        return f"VeriPB: {(p.stdout + p.stderr)[-400:]}"
    return None


def solver_says(clauses, extra_units, path, timeout=120):
    """The stock path on the clauses plus units: 'sat', 'unsat' or 'unknown'."""
    with open(path, "w") as f:
        nv = max((abs(l) for c in clauses for l in c), default=0)
        nv = max([nv] + [abs(u) for u in extra_units])
        f.write(f"p cnf {nv} {len(clauses) + len(extra_units)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")
        for u in extra_units:
            f.write(f"{u} 0\n")
    p = subprocess.run([Cfg.solver, "-q", "--card=0", *STOCK, path], capture_output=True, text=True, timeout=timeout)
    return {10: "sat", 20: "unsat"}.get(p.returncode, "unknown")


def semantic_errors(r, clauses, path, rng=None, limit=None):
    """Each claim decided by the stock solver path. Returns (errors, claims checked)."""
    claims = set()
    for g in r.grids:
        for row in g.rows:
            claims.add(("row", tuple(sorted(row))))
        for col in g.cols:
            for a, b in itertools.combinations(sorted(col), 2):
                claims.add(("pair", (a, b)))
    claims = sorted(claims)
    if limit is not None and len(claims) > limit:
        claims = (rng or random.Random(1)).sample(claims, limit)
    errors = []
    for kind, lits in claims:
        units = list(lits) if kind == "pair" else [-l for l in lits]
        said = solver_says(clauses, units, path)
        if said != "unsat":
            errors.append(f"{kind} {lits}: the stock solver says {said}")
    return errors, len(claims)


def stats(r):
    """The census's numbers from the grids (tools/alldiff_census.py, `analyse`)."""
    core = big = 0
    max_n = big_n = 0
    big_tight = 0.0
    for g in r.grids:
        n, shared, private = g.shape
        tight = n / max(1, shared + private)
        if n >= 3 and shared >= 3 and private <= shared:
            core += 1
            max_n = max(max_n, n)
            if n >= BIG:
                big += 1
                if (tight, n) > (big_tight, big_n):
                    big_tight, big_n = tight, n
    return {"grids3": len(r.grids), "core": core, "big": big, "big_n": big_n, "big_tight": round(big_tight, 3), "max_n": max_n}


# ------------------------------------------------------------------------ the toys

def amo(e, lits, how):
    {"pairwise": e.amo_pairwise, "ladder": e.amo_ladder, "commander": e.amo_commander}[how](lits)


def toy_php_enc(pigeons, holes, how):
    e = Enc()
    x = [[e.var() for _ in range(holes)] for _ in range(pigeons)]
    for row in x:
        e.alo(row)
    for h in range(holes):
        amo(e, [x[p][h] for p in range(pigeons)], how)
    return e, x


def toy_queens(n):
    e = Enc()
    q = [[e.var() for _ in range(n)] for _ in range(n)]
    for i in range(n):
        e.alo(q[i])
        e.amo_pairwise(q[i])
    for j in range(n):
        e.amo_pairwise([q[i][j] for i in range(n)])
    for k in range(-n + 1, n):
        e.amo_pairwise([q[i][i - k] for i in range(n) if 0 <= i - k < n])
    for k in range(0, 2 * n - 1):
        e.amo_pairwise([q[i][k - i] for i in range(n) if 0 <= k - i < n])
    return e


def toy_guarded(n=3, d=3):
    e = Enc()
    x = [[e.var() for _ in range(d)] for _ in range(n)]
    for i in range(n):
        g = e.var()
        e.cl.append([g] + x[i])
        e.amo_pairwise(x[i])
    for j in range(d):
        e.amo_pairwise([x[i][j] for i in range(n)])
    return e


def toy_split(n=4, parts=3, per=4, how="pairwise"):
    """Each row split through guards: (g1 v g2 v g3), (~gi v its leaves)."""
    e = Enc()
    d = parts * per
    x = [[e.var() for _ in range(d)] for _ in range(n)]
    for i in range(n):
        guards = [e.var() for _ in range(parts)]
        e.alo(guards)
        for k, g in enumerate(guards):
            e.cl.append([-g] + x[i][k * per:(k + 1) * per])
    for j in range(d):
        amo(e, [x[i][j] for i in range(n)], how)
    return e


def toy_chain(n=4, d=7, how="pairwise"):
    """Each row split into a chain of three-literal clauses, as a conversion to 3-SAT
    does: (x1 v x2 v g1), (~g1 v x3 v g2), ..., (~gk v x(d-1) v xd)."""
    e = Enc()
    x = [[e.var() for _ in range(d)] for _ in range(n)]
    for i in range(n):
        g = e.var()
        e.cl.append([x[i][0], x[i][1], g])
        for j in range(2, d - 2):
            h = e.var()
            e.cl.append([-g, x[i][j], h])
            g = h
        e.cl.append([-g, x[i][d - 2], x[i][d - 1]])
    for j in range(d):
        amo(e, [x[i][j] for i in range(n)], how)
    return e


def toy_oneway(pigeons=4, holes=3):
    """Pigeonhole with exclusions that show from one side only."""
    e = Enc()
    x = [[e.var() for _ in range(holes)] for _ in range(pigeons)]
    for row in x:
        e.alo(row)
        for v in row:
            e.cl.append([-v, e.var()])     # every literal propagates something
    for h in range(holes):
        for p, q in itertools.combinations(range(pigeons), 2):
            a, b = e.var(), e.var()
            e.cl += [[-x[p][h], a], [-x[p][h], b], [-a, -b, -x[q][h]]]
    return e


def toy_one_side():
    e = Enc()
    t = [e.var() for _ in range(9)]
    aux, aux2 = e.var(), e.var()
    e.alo(t[0:3])
    e.alo(t[3:6])
    e.alo(t[6:9])
    e.amo_pairwise([t[0], t[6]])
    e.amo_pairwise([t[1], t[7]])
    e.amo_pairwise([t[2], t[8]])
    e.cl += [[-t[3], aux], [-t[3], aux2], [-aux, -aux2, -t[0]], [-t[4], -t[1]], [-t[4], -t[7]], [-t[5], -t[2]], [-t[5], -t[8]]]
    e.cl += [[-t[3], -t[6]]]
    return e


def toy_or_gates():
    e = Enc()
    ins = [e.var() for _ in range(8)]
    for i in range(0, 6):
        o = e.var()
        e.cl += [[o, -ins[i]], [o, -ins[i + 1]], [-o, ins[i], ins[i + 1]]]
    return e


def toy_dense_table():
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
    return e


def toy_grid_with_gates():
    e = toy_php(5, 4)
    ins = list(range(1, 21))
    for i in range(0, 19):
        o = e.var()
        e.cl += [[o, -ins[i]], [o, -ins[i + 1]], [-o, ins[i], ins[i + 1]]]
    return e


def toy_partial(pigeons=6, holes=5, part=3):
    """Rows with an at-most-one over part of them: not exact, not plain, not guarded."""
    e, x = toy_php_enc(pigeons, holes, "pairwise")
    for row in x:
        e.amo_pairwise(row[:part])
    return e


def toy_blocks(pigeons=5, holes=4):
    """Rows of two at-most-one blocks: no literal excludes all the others."""
    e, x = toy_php_enc(pigeons, holes, "pairwise")
    for row in x:
        e.amo_pairwise(row[:holes // 2])
        e.amo_pairwise(row[holes // 2:])
    return e


def toy_with_units():
    """One row satisfied and one literal of each other row false at the root."""
    e = toy_php(4, 4, row_amo=True)
    e.cl.append([1])
    return e


def toy_shortened_row():
    """A row with an extra literal a unit makes false: read as the row without it."""
    e = toy_php(4, 3)
    z = e.var()
    e.cl[0] = e.cl[0] + [z]
    e.cl.append([-z])
    return e


def toy_repeated_literal():
    """A row written with a literal twice: its constraint has another coefficient."""
    e = toy_php(4, 3)
    e.cl[0] = [e.cl[0][0]] + e.cl[0]
    return e


def toy_repeated_pair():
    """A pair clause written with a literal twice is no input pair clause for `pol`."""
    e = toy_php(4, 3)
    for i, c in enumerate(e.cl):
        if len(c) == 2:
            e.cl[i] = [c[0], c[0], c[1]]
            break
    return e


def toy_failed_literal():
    """A row with a literal that fails alone: it is left out of the row."""
    e = toy_php(4, 3)
    f = e.var()
    g = e.var()
    e.cl[0] = e.cl[0] + [f]
    e.cl += [[-f, g], [-f, -g]]
    return e


def toy_two_failed():
    """Two failing literals in one clause of a row: their proofs must come first."""
    e = toy_php(4, 3)
    f1, f2, g = e.var(), e.var(), e.var()
    e.cl[0] = e.cl[0] + [f1, f2]
    e.cl += [[-f1, g], [-f1, -g], [-f2, g], [-f2, -g]]
    return e


def toy_negated_values():
    """A literal in one row and its negation in another: a column of the two, next
    to two columns of pair clauses."""
    e = Enc()
    a = e.var()
    p1, p2, q1, q2, r2, r3, r4 = [e.var() for _ in range(7)]
    e.cl += [[a, p1, p2], [-a, q1, q2], [r2, r3, r4]]
    e.amo_pairwise([p1, q1, r2])
    e.amo_pairwise([p2, q2, r3])
    return e


def toy_long_ladder(pigeons=1100, holes=3):
    """Columns too long for a literal's first, short read: the second pass reads them."""
    e, _ = toy_php_enc(pigeons, holes, "ladder")
    return e


def toy_hall_units():
    """Three rows left with three values by units, two rows with all five."""
    e = toy_php(5, 5, row_amo=True)
    for p in range(3):
        e.cl += [[-(p * 5 + 4)], [-(p * 5 + 5)]]
    return e


def toy_amo_only():
    e = Enc()
    v = [e.var() for _ in range(5)]
    e.amo_pairwise(v)
    return e


def toy_hub():
    e = Enc()
    hub = e.var()
    for _ in range(50_010):
        e.cl.append([-hub, e.var()])
    rows = [[e.var() for _ in range(3)] for _ in range(3)]
    for r0 in rows:
        e.alo(r0)
    for j in range(3):
        e.amo_pairwise([rows[i][j] for i in range(3)])
    e.alo([hub, rows[0][0], rows[1][1]])
    return e


def toy_behind_rows(n=6, d=10, groups=40, per=10):
    """A grid with many rows around it that are no domains: long clauses over groups
    of literals that exclude each other inside a group. Each of those literals also
    excludes a whole column of the grid, so such a row has more exclusions towards a
    grid row than two grid rows have. Read with a budget too small to read all rows
    before linking: those rows then come first among a grid row's neighbours."""
    e = Enc()
    rows = [[e.var() for _ in range(d)] for _ in range(n)]
    for r in rows:
        e.alo(r)
        e.amo_pairwise(r)
    for j in range(d):
        e.amo_pairwise([rows[i][j] for i in range(n)])
    pool = [[e.var() for _ in range(per)] for _ in range(groups)]
    for g in pool:
        e.amo_pairwise(g)
        for i, p in enumerate(g):
            for r in rows:
                e.cl.append([-p, -r[i % d]])
    for a in range(groups):
        for b in range(a):
            e.alo(pool[a] + pool[b])
    return e


def toy_capacity(n=5, d=3, cap=2):
    """Rows over values that hold up to two rows each: no pair excludes, so no grid."""
    e = Enc()
    x = [[e.var() for _ in range(d)] for _ in range(n)]
    for row in x:
        e.alo(row)
    for j in range(d):
        for s in itertools.combinations(range(n), cap + 1):
            e.cl.append([-x[i][j] for i in s])
    return e


def toy_private_value():
    e = toy_php(3, 3)
    e.cl[0] = e.cl[0] + [e.var()]
    return e


def has(*shapes):
    def test(r):
        got = r.shapes()
        miss = [s for s in shapes if s not in got]
        return (not miss), f"missing {miss}; read {got.most_common(6)}"
    return test


def has_at_least(count, shape):
    def test(r):
        got = r.shapes()[shape]
        return got >= count, f"{got} of {shape}, wanted {count}; read {r.shapes().most_common(6)}"
    return test


def both(*tests):
    def test(r):
        for t in tests:
            ok, text = t(r)
            if not ok:
                return ok, text
        return True, ""
    return test


def no_grid(r):
    return not r.grids, f"read {r.shapes().most_common(6)}"


def grids_of(n):
    def test(r):
        return len(r.grids) == n, f"{len(r.grids)} grids, wanted {n}: {r.shapes().most_common(6)}"
    return test


def at_least_grids(n):
    def test(r):
        return len(r.grids) >= n, f"{len(r.grids)} grids, wanted at least {n}"
    return test


def row_is_resolvent(leaves):
    def test(r):
        ok = any(len(row) == leaves and g.clauses[i] == -1 for g in r.grids for i, row in enumerate(g.rows))
        return ok, f"no resolvent row of {leaves} leaves"
    return test


def no_clause_row(n):
    def test(r):
        ok = any(len(row) == n and g.clauses[i] == -1 for g in r.grids for i, row in enumerate(g.rows))
        return ok, f"no row of {n} literals without a clause of its own"
    return test


def row_lines_above_one(r):
    ok = any(k > 1 for g in r.grids for k in g.rowlines)
    return ok, "no row whose proof shows a literal false first"


def complementary_column(r):
    ok = any(any(-l in c for l in c) for g in r.grids for c in g.cols)
    return ok, f"no column of a literal and its negation: {[g.cols for g in r.grids][:2]}"


def toys():
    """(name, clauses, what must hold, how many claims the stock solver decides)."""
    k5 = list(itertools.combinations(range(5), 2))
    c5 = [(i, (i + 1) % 5) for i in range(5)]
    two = list(itertools.combinations(range(4), 2)) + list(itertools.combinations([3, 4, 5, 6], 2))
    out = []
    for how in ("pairwise", "ladder", "commander"):
        out.append((f"latin 4, {how}", toy_latin(4, how).cl, both(has((4, 4, 0)), at_least_grids(8)), None))
    out += [
        ("latin 4, signs flipped", flipped(toy_latin(4), 3), both(has((4, 4, 0)), at_least_grids(8)), None),
        ("latin 4 commander, signs flipped", flipped(toy_latin(4, "commander"), 5), has((4, 4, 0)), None),
        ("latin 5, ladder", toy_latin(5, "ladder").cl, both(has((5, 5, 0)), at_least_grids(10)), 400),
        ("sudoku of order 4", toy_sudoku(2).cl, both(has_at_least(12, (4, 4, 0)), at_least_grids(12)), None),
        ("sudoku of order 9", toy_sudoku(3).cl, has_at_least(27, (9, 9, 0)), 300),
        ("n-queens 8: columns next to diagonals", toy_queens(8).cl, has((8, 8, 0)), 300),
        ("pigeonhole 5 into 4", toy_php(5, 4).cl, has((5, 4, 0)), None),
        ("pigeonhole 4 into 4", toy_php(4, 4).cl, has((4, 4, 0)), None),
        ("pigeonhole 30 into 29", toy_php(30, 29).cl, grids_of(1), 300),
        ("pigeonhole, one row fixed by a unit", toy_with_units().cl, has((3, 3, 0)), None),
        ("three rows left with three values by units", toy_hall_units().cl, has((5, 5, 0)), None),
        ("pigeonhole, ladder columns", toy_php_enc(6, 5, "ladder")[0].cl, has((6, 5, 0)), None),
        ("pigeonhole, commander columns", toy_php_enc(6, 5, "commander")[0].cl, has((6, 5, 0)), None),
        ("colouring a 5-clique with 4 colours", toy_colouring(k5, 5, 4).cl, has((5, 4, 0)), None),
        ("colouring a 5-cycle: edges differ, no grid of three", toy_colouring(c5, 5, 3).cl, no_grid, None),
        ("two 4-cliques sharing a vertex", toy_colouring(two, 7, 5).cl, grids_of(2), None),
        ("one-way exclusions", toy_oneway().cl, has((4, 3, 0)), None),
        ("a pair that shows from one side only", toy_one_side().cl, has((3, 3, 0)), None),
        ("a split at-least-one reads as a row over its leaves", toy_split().cl, both(has((4, 12, 0)), row_is_resolvent(12)), None),
        ("split rows, ladder columns", toy_split(how="ladder").cl, has((4, 12, 0)), None),
        ("a split in two parts", toy_split(4, 2, 3).cl, both(has((4, 6, 0)), row_is_resolvent(6)), None),
        ("a chain of three-literal clauses", toy_chain().cl, both(has((4, 7, 0)), row_is_resolvent(7)), None),
        ("a long chain, ladder columns", toy_chain(5, 30, "ladder").cl, both(has((5, 30, 0)), row_is_resolvent(30)), 300),
        ("guarded rows", toy_guarded().cl, has((3, 3, 3)), None),
        ("partial rows", toy_partial().cl, has((6, 5, 0)), None),
        ("partial rows of two blocks", toy_blocks().cl, has((5, 4, 0)), None),
        ("a value only one row has", toy_private_value().cl, has((3, 3, 1)), None),
        ("a row a unit shortened has no clause of its own", toy_shortened_row().cl, both(has((4, 3, 0)), no_clause_row(3)), None),
        ("a row with a repeated literal has no clause of its own", toy_repeated_literal().cl, both(has((4, 3, 0)), no_clause_row(3)), None),
        ("a pair clause with a repeated literal goes by propagation", toy_repeated_pair().cl, has((4, 3, 0)), None),
        ("a row with a failing literal leaves it out", toy_failed_literal().cl, both(has((4, 3, 0)), no_clause_row(3)), None),
        ("two failing literals in one clause: their proofs first", toy_two_failed().cl, both(has((4, 3, 0)), row_lines_above_one), None),
        ("a literal and its negation in two rows", toy_negated_values().cl, both(has((3, 3, 1)), complementary_column), None),
        ("columns too long for the first read", toy_long_ladder().cl, has((1100, 3, 0)), 100),
        ("a hub literal does not stop the read", toy_hub().cl, has((3, 3, 0)), 12),
        ("or-gates over a grid's literals leave it", toy_grid_with_gates().cl, has((5, 4, 0)), None),
        ("an at-most-one with no row over it: no grid", toy_amo_only().cl, no_grid, None),
        ("values that hold two rows each: not read, no grid", toy_capacity().cl, no_grid, None),
        ("an empty clause in the input: no grid", toy_php(4, 3).cl + [[]], no_grid, None),
        ("units that conflict at the root: no grid", toy_php(4, 3).cl + [[1], [-1]], no_grid, None),
        ("an and-chain: no grid", toy_circuit(6).cl, no_grid, None),
        ("or-gates over shared inputs: no grid", toy_or_gates().cl, no_grid, None),
        ("an xor chain: no grid", toy_xor(6).cl, no_grid, None),
        ("a dense table constraint: no grid", toy_dense_table().cl, no_grid, None),
    ]
    return out


# ---------------------------------------------------------------------- selftest

class Report:
    def __init__(self):
        self.ok = True

    def check(self, name, cond, detail=""):
        print(("ok   " if cond else "FAIL ") + name + (f"  ({detail})" if detail not in ("", None) else ""), flush=True)
        self.ok &= bool(cond)


def full_check(name, clauses, tmp, want=None, limit=None, args=()):
    """Reads a toy and runs every check on it. Returns (errors, the read)."""
    base = Path(tmp) / re.sub(r"[^a-z0-9]+", "_", name.lower())[:60]
    cnf = f"{base}.cnf"
    pbp = f"{base}.pbp"
    write_cnf(cnf, clauses)
    r = run_read(cnf, pbp, args=args)
    errors = structure_errors(r, clauses)
    if want is not None:
        ok, text = want(r)
        if not ok:
            errors.append(text)
    if not errors:
        e = veripb_check(cnf, pbp, clauses, r)
        if e:
            errors.append(e)
    claims = 0
    if not errors:
        sem, claims = semantic_errors(r, clauses, f"{base}.sem.cnf", limit=limit)
        errors += sem[:5]
    r.claims = claims
    return errors, r


def broken_copies(rep, tmp):
    """A clause dropped from an encoding: the exclusions it carried are not claimed,
    and everything still claimed is true."""
    # a pair clause dropped: the two literals may not share a column any more
    e = toy_php(5, 4)
    pair = next(c for c in e.cl if len(c) == 2)
    cl = [c for c in e.cl if c is not pair]
    errors, r = full_check("broken pair clause", cl, tmp)
    together = [c for g in r.grids for c in g.cols if -pair[0] in c and -pair[1] in c]
    rep.check("a pair clause dropped from pigeonhole: the pair shares no column, the rest is true", not errors and not together, errors or together)
    rep.check("  and the formula plus the pair is satisfiable", solver_says(cl, [-pair[0], -pair[1]], f"{tmp}/broken_sem.cnf") == "sat")
    e = toy_latin(4)
    pair = next(c for c in e.cl if len(c) == 2)
    cl = [c for c in e.cl if c is not pair]
    errors, r = full_check("broken latin pair", cl, tmp)
    together = [c for g in r.grids for c in g.cols if -pair[0] in c and -pair[1] in c]
    rep.check("a pair clause dropped from a latin square: the pair shares no column, the rest is true", not errors and not together, errors or together)
    # every ladder clause in turn
    for how, build in (("ladder", lambda: toy_php_enc(5, 4, "ladder")[0]), ("commander", lambda: toy_php_enc(5, 4, "commander")[0])):
        base = build()
        rows = {tuple(c) for c in base.cl[:5]}
        whole_err, whole = full_check(f"unbroken {how}", base.cl, tmp)
        full = {tuple(c) for g in whole.grids for c in g.cols}
        lost_all = True
        bad = []
        tested = 0
        for i, c in enumerate(base.cl):
            if tuple(c) in rows:
                continue
            cl = base.cl[:i] + base.cl[i + 1:]
            errors, r = full_check(f"broken {how} {i}", cl, tmp)
            tested += 1
            if errors:
                bad.append((c, errors[:2]))
            now = {tuple(x) for g in r.grids for x in g.cols}
            if full <= now:
                lost_all = False
                bad.append((c, "all columns of the whole formula are still read"))
        rep.check(f"{tested} {how} clauses dropped one at a time: a column goes each time, all that is read is true", not whole_err and not bad and lost_all, (whole_err or bad[:3]))


def cmd_selftest(args):
    rep = Report()
    tmp = args.tmp
    Path(tmp).mkdir(parents=True, exist_ok=True)
    for name, clauses, want, limit in toys():
        errors, r = full_check(name, clauses, tmp, want, limit)
        rep.check(name, not errors, errors[:3] if errors else f"{r.shapes().most_common(3)}, {r.claims} claims decided, {r.ticks} ticks")
    name = "a grid behind many rows that are no domains, small budget"
    def six_rows(r):
        return any(len(g.rows) >= 6 for g in r.grids), f"read {r.shapes().most_common(3)}"
    errors, r = full_check(name, toy_behind_rows().cl, tmp, six_rows, 100, args=["--cardreadticks=400"])
    rep.check(name, not errors, errors[:3] if errors else f"{r.shapes().most_common(3)}, {r.claims} claims decided, {r.ticks} ticks")
    broken_copies(rep, tmp)
    print("selftest:", "all passed" if rep.ok else "FAILED")
    return 0 if rep.ok else 1


# -------------------------------------------------------------------------- fuzz

def random_base(rng):
    """A small grid formula with a random shape and encoding per constraint."""
    kind = rng.choice(["latin", "php", "colour", "queens", "sudoku", "guarded", "split", "oneway", "partial", "gates", "blocks", "special"])
    how = lambda: rng.choice(["pairwise", "ladder", "commander"])  # noqa: E731
    if kind == "latin":
        return kind, toy_latin(rng.randint(3, 4), how()).cl
    if kind == "php":
        p = rng.randint(3, 7)
        h = rng.randint(max(2, p - 1), p + 1)
        e = Enc()
        x = [[e.var() for _ in range(h)] for _ in range(p)]
        for row in x:
            e.alo(row)
            if rng.random() < 0.3:
                amo(e, row, how())
        for j in range(h):
            amo(e, [x[i][j] for i in range(p)], how())
        return kind, e.cl
    if kind == "colour":
        n = rng.randint(4, 8)
        edges = [(a, b) for a, b in itertools.combinations(range(n), 2) if rng.random() < 0.8]
        return kind, toy_colouring(edges, n, rng.randint(3, 5)).cl
    if kind == "queens":
        return kind, toy_queens(rng.randint(4, 6)).cl
    if kind == "sudoku":
        return kind, toy_sudoku(2).cl
    if kind == "guarded":
        return kind, toy_guarded(rng.randint(3, 5), rng.randint(3, 5)).cl
    if kind == "split":
        if rng.random() < 0.3:
            return kind, toy_chain(rng.randint(3, 5), rng.randint(4, 8), how()).cl
        return kind, toy_split(rng.randint(3, 5), rng.randint(2, 3), rng.randint(2, 3), how()).cl
    if kind == "oneway":
        return kind, toy_oneway(rng.randint(3, 5), rng.randint(2, 4)).cl
    if kind == "partial":
        return kind, toy_partial(rng.randint(4, 6), 5, rng.randint(2, 4)).cl
    if kind == "blocks":
        return kind, toy_blocks(rng.randint(4, 6), rng.randint(4, 6)).cl
    if kind == "special":
        toy = rng.choice([toy_negated_values, toy_failed_literal, toy_two_failed, toy_with_units, toy_shortened_row, toy_repeated_literal, toy_repeated_pair, toy_one_side, toy_private_value, toy_grid_with_gates, toy_or_gates, toy_dense_table,
                          lambda: toy_long_ladder(rng.randint(1005, 1030), 3)])
        return kind, toy().cl
    e = toy_php(rng.randint(3, 5), 3)
    ins = list(range(1, e.n + 1))
    for i in range(len(ins) - 1):
        o = e.var()
        e.cl += [[o, -ins[i]], [o, -ins[i + 1]], [-o, ins[i], ins[i + 1]]]
    return kind, e.cl


def mutate(cl, rng):
    """Noise, dropped clauses, renamed variables, flipped signs, repeated literals,
    tautologies, duplicates, units, shuffled clauses and literals, each at random."""
    cl = [list(c) for c in cl]
    nv = max((abs(l) for c in cl for l in c), default=1)
    pdrop = rng.choice([0, 0, 0.01, 0.03, 0.08])
    cl = [c for c in cl if rng.random() >= pdrop]
    lit = lambda: rng.randint(1, nv) * rng.choice([1, -1])  # noqa: E731
    for _ in range(rng.choice([0, 0, 1, 3, 8])):
        cl.append([lit(), lit()])
    for _ in range(rng.choice([0, 0, 1, 3])):
        cl.append([lit() for _ in range(rng.randint(3, 5))])
    if rng.random() < 0.3 and cl:
        for _ in range(rng.randint(1, 5)):                 # repeated literals
            c = rng.choice(cl)
            c.insert(rng.randrange(len(c) + 1), rng.choice(c))
    if rng.random() < 0.3:
        for _ in range(rng.randint(1, 4)):                 # tautologies
            v = rng.randint(1, nv)
            cl.append(rng.choice([[v, -v], [v, -v, lit()], [lit(), v, lit(), -v]]))
    if rng.random() < 0.3 and cl:
        for _ in range(rng.randint(1, 8)):                 # duplicates
            cl.append(list(rng.choice(cl)))
    if rng.random() < 0.3:
        for _ in range(rng.randint(1, 3)):                 # units
            cl.append([lit()])
    # flipped signs and renamed variables, with gaps in the numbering
    sign = {v: rng.choice([1, -1]) for v in range(1, nv + 1)} if rng.random() < 0.5 else {v: 1 for v in range(1, nv + 1)}
    names = list(range(1, nv + 1))
    if rng.random() < 0.5:
        rng.shuffle(names)
    gap = rng.choice([1, 1, 2, 5])
    new = {v: names[v - 1] * gap for v in range(1, nv + 1)}
    cl = [[new[abs(l)] * sign[abs(l)] * (1 if l > 0 else -1) for l in c] for c in cl]
    if rng.random() < 0.7:
        rng.shuffle(cl)
    if rng.random() < 0.5:
        for c in cl:
            rng.shuffle(c)
    return cl


def fuzz_case(job):
    seed, tmp, solver, veripb = job
    Cfg.solver, Cfg.veripb = solver, veripb
    rng = random.Random(seed)
    kind, cl = random_base(rng)
    cl = mutate(cl, rng)
    base = Path(tmp) / f"f{seed}"
    cnf, pbp = f"{base}.cnf", f"{base}.pbp"
    write_cnf(cnf, cl)
    out = {"seed": seed, "kind": kind, "grids": 0, "claims": 0, "unsat_base": False, "errors": [], "rups": 0, "ticks": 0, "stopped": False}
    # one case in five with a budget that stops the read
    args = [f"--cardreadticks={rng.choice([0, 1, 2, 5, 10, 30])}"] if rng.random() < 0.2 else []
    try:
        r = run_read(cnf, pbp, args=args, timeout=120)
        out["stopped"] = r.notes.get("stopped") == "true"
        out["errors"] += structure_errors(r, cl)
        out["grids"] = len(r.grids)
        out["ticks"] = r.ticks
        out["rups"] = sum(sum(g.collines) for g in r.grids)
        if r.grids and not out["errors"]:
            e = veripb_check(cnf, pbp, cl, r, timeout=300)
            if e:
                out["errors"].append(e)
            sem_cnf = f"{base}.sem.cnf"
            sem, claims = semantic_errors(r, cl, sem_cnf, rng=rng, limit=300)
            out["errors"] += sem[:4]
            out["claims"] = claims
            out["unsat_base"] = solver_says(cl, [], sem_cnf) == "unsat"
    except subprocess.TimeoutExpired as e:
        out["errors"].append(f"timeout: {e.cmd[:2]}")
    if not out["errors"]:
        for p in (cnf, pbp, f"{base}.sem.cnf"):
            if os.path.exists(p):
                os.unlink(p)
    return out


def cmd_fuzz(args):
    Path(args.tmp).mkdir(parents=True, exist_ok=True)
    jobs = [(args.seed + i, args.tmp, Cfg.solver, Cfg.veripb) for i in range(args.count)]
    kinds = Counter()
    totals = Counter()
    failures = []
    t0 = time.time()
    with ProcessPoolExecutor(max_workers=args.workers) as ex:
        for out in ex.map(fuzz_case, jobs, chunksize=4):
            kinds[out["kind"]] += 1
            totals["with a grid"] += out["grids"] > 0
            totals["grids"] += out["grids"]
            totals["claims"] += out["claims"]
            totals["unsat base with a grid"] += out["unsat_base"]
            totals["proof lines"] += out["rups"]
            totals["stopped by a budget"] += out["stopped"]
            if out["errors"]:
                failures.append(out)
                print(f"FAIL seed {out['seed']} {out['kind']}: {out['errors'][:3]}", flush=True)
    print(f"{args.count} cases in {time.time() - t0:.0f} s: {dict(totals)}")
    print(f"kinds: {dict(kinds)}")
    print(f"failures: {len(failures)}; their files stay in {args.tmp}")
    return 1 if failures else 0


# ---------------------------------------------------------------- real formulas

def read_clauses(path):
    clauses = []
    with open(path) as f:
        for line in f:
            if line[:1] in "cp":
                continue
            lits = [int(t) for t in line.split()]
            if lits and lits[-1] == 0:
                clauses.append(lits[:-1])
    return clauses


def cmd_compare(args):
    table = {r["stem"]: r for r in csv.DictReader(open(TABLE), delimiter="\t")}
    rep = Report()
    print(f"{'cell':40} {'':6} {'grids3':>7} {'core':>6} {'big':>5} {'big_n':>5} {'tight':>6} {'max_n':>5} {'ticks/visits':>13} {'stopped':>8} {'wall s':>7}")
    for path in sorted(Path(args.cells).glob("*.cnf")):
        stem = path.name.split(".cnf")[0].replace(".prefix3M", "")
        t = table.get(stem)
        pbp = Path(args.tmp) / (stem + ".pbp")
        want_proof = any(s in stem for s in args.veripb_cells or [])
        r = run_read(path, pbp if want_proof else None)
        s = stats(r)
        print(f"{stem[-40:]:40} {'reader':6} {s['grids3']:>7} {s['core']:>6} {s['big']:>5} {s['big_n']:>5} {s['big_tight']:>6} {s['max_n']:>5} {r.ticks:>13} {r.notes.get('stopped', '?'):>8} {float(r.notes.get('wall', 0)):>7.2f}", flush=True)
        if t:
            print(f"{'':40} {'census':6} {t['grids3']:>7} {t['grids_core']:>6} {t['grids_big']:>5} {t['big_n']:>5} {t['big_tight']:>6} {t['max_n']:>5} {t['visits']:>13} {t['stopped'] or 'no':>8} {t['detect_s']:>7}", flush=True)
        clauses = None
        if want_proof or args.structure:
            clauses = read_clauses(path)
            errors = structure_errors(r, clauses)
            rep.check(f"  by hand from the CNF: {stem[-30:]}", not errors, errors[:3])
        if want_proof:
            t0 = time.time()
            e = veripb_check(path, pbp, clauses, r)
            rep.check(f"  VeriPB on {stem[-30:]}, {pbp.stat().st_size / 1e6:.1f} MB, {sum(sum(g.collines) + sum(g.rowlines) for g in r.grids)} lines, {time.time() - t0:.0f} s", e is None, e)
            pbp.unlink()
    return 0 if rep.ok else 1


def sweep_cell(job):
    stem, path, solver = job
    Cfg.solver = solver
    t0 = time.time()
    out = {"stem": stem}
    try:
        r = run_read(path, timeout=1200)
    except subprocess.TimeoutExpired:
        out["error"] = "timeout"
        return out
    out.update(stats(r))
    out.update(ticks=r.ticks, budget=r.budget - r.begin, wall=r.notes.get("wall", ""), setup=r.notes.get("setup", ""), detect=r.notes.get("detect", ""), check=r.notes.get("check", ""),
               rows=r.notes.get("rows", ""), classified=r.notes.get("classified", ""), partial=r.notes.get("partial", ""), found=r.notes.get("found", ""), rejected=r.notes.get("rejected", ""),
               stopped=r.notes.get("stopped", ""), partial_read=r.notes.get("partial_read", ""), code=r.code, proc_s=round(time.time() - t0, 1), panic=int("panicked" in r.stderr))
    return out


def cmd_sweep(args):
    """The reader on every cell the structure stage reads, against the census's table."""
    table = {r["stem"]: r for r in csv.DictReader(open(TABLE), delimiter="\t")}
    jobs = [(stem, str(ROOT / "benchmarks" / "sat-comp-2025" / f"{stem}.cnf.xz"), Cfg.solver) for stem, r in sorted(table.items()) if r["over_limit"] == "0"]
    fields = ["stem", "family", "vars", "clauses", "census_grids3", "census_core", "census_big", "census_big_n", "census_big_tight", "census_max_n", "census_visits", "census_stopped",
              "grids3", "core", "big", "big_n", "big_tight", "max_n", "ticks", "budget", "wall", "setup", "detect", "check", "rows", "classified", "partial", "found", "rejected", "stopped", "partial_read", "code", "proc_s", "panic", "error"]
    with open(args.out, "w", newline="") as f, ProcessPoolExecutor(max_workers=args.workers) as ex:
        wr = csv.DictWriter(f, fieldnames=fields, delimiter="\t", extrasaction="ignore")
        wr.writeheader()
        for out in ex.map(sweep_cell, jobs):
            t = table[out["stem"]]
            out.update(family=t["family"], vars=t["vars"], clauses=t["clauses"], census_grids3=t["grids3"], census_core=t["grids_core"], census_big=t["grids_big"], census_big_n=t["big_n"],
                       census_big_tight=t["big_tight"], census_max_n=t["max_n"], census_visits=t["visits"], census_stopped=t["stopped"])
            wr.writerow(out)
            f.flush()
    return 0


# ---------------------------------------------------------------- the budget

def budget_cases(cells):
    out = []
    tmp = Path(Cfg.tmp)
    a = tmp / "budget_sudoku16.cnf"
    write_cnf(a, toy_sudoku(4).cl)
    out.append(("sudoku of order 16", a))
    b = tmp / "budget_latin8_ladder.cnf"
    write_cnf(b, toy_latin(8, "ladder").cl)
    out.append(("latin 8, ladder", b))
    # padding: a stray variable far out, and thousands of copies of clauses
    c = tmp / "budget_padded.cnf"
    e = toy_php(12, 11)
    pad = [list(e.cl[0])] * 30000 + [list(e.cl[-1])] * 30000 + [[-(2_000_000)]] * 3000
    write_cnf(c, e.cl + pad)
    out.append(("pigeonhole padded: a stray variable, 63,000 copies", c))
    for s in cells:
        out.append((s.name[:44], s))
    return out


def cmd_budget(args):
    rep = Report()
    Path(args.tmp).mkdir(parents=True, exist_ok=True)
    cells = sorted(Path(args.cells).glob("*.cnf")) if args.cells else []
    cells = [c for c in cells if any(s in c.name for s in args.only)] if args.only else cells
    for name, path in budget_cases(cells):
        whole = run_read(path)
        again = run_read(path)
        same = whole.ticks == again.ticks and [g.shape for g in whole.grids] == [g.shape for g in again.grids] and [g.cols for g in whole.grids] == [g.cols for g in again.grids]
        rep.check(f"{name}: two runs alike", same, f"{whole.ticks} ticks, {len(whole.grids)} grids")
        top = 0.0
        lines = []
        for pct in (1, 2, 5, 10, 20, 30, 50, 70, 90, 99):
            k = max(1, whole.ticks * pct // 100 // 1000)
            r = run_read(path, args=[f"--cardreadticks={k}"])
            over = (r.ticks - k * 1000) / (k * 1000)
            top = max(top, over)
            lines.append(f"{pct}%: {r.ticks - k * 1000:+d} {len(r.grids)}g")
        rep.check(f"{name}: stopped at fractions, overrun at most {top * 100:.2f}% of the limit", top <= 0.10, "; ".join(lines))
    print("budget:", "all passed" if rep.ok else "FAILED")
    return 0 if rep.ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["selftest", "fuzz", "compare", "budget", "sweep"])
    ap.add_argument("--solver", default=Cfg.solver)
    ap.add_argument("--veripb", default=Cfg.veripb)
    ap.add_argument("--tmp", default=Cfg.tmp)
    ap.add_argument("--count", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--cells", default=None)
    ap.add_argument("--only", nargs="*", default=None)
    ap.add_argument("--veripb-cells", nargs="*", default=None)
    ap.add_argument("--structure", action="store_true", help="also check each grid by hand from the CNF")
    ap.add_argument("--out", default=None, help="sweep: the table to write")
    args = ap.parse_args()
    Cfg.solver, Cfg.veripb, Cfg.tmp = args.solver, args.veripb, args.tmp
    Path(args.tmp).mkdir(parents=True, exist_ok=True)
    return {"selftest": cmd_selftest, "fuzz": cmd_fuzz, "compare": cmd_compare, "budget": cmd_budget, "sweep": cmd_sweep}[args.mode](args)


if __name__ == "__main__":
    sys.exit(main())
