#!/usr/bin/env python3
"""Checks of the Rust count reader, solver/13-kissat-rs/src/cardread.rs (epic
SAT-playground-iin, bead iin.7; plan/count-and-hall-lemmas-plan.md).

A count the reader returns can make the solver answer UNSAT wrongly when no
proof file is written, so everything here goes at one question: is every count
true, and is its proof good? Three things are checked for each count:

  - VeriPB accepts the proof lines, ending in an `e` line that says the returned
    id is exactly "sum of lits >= need";
  - the formula plus the count's negation is unsatisfiable, decided by the stock
    solver path, with the negation written by a plain counter (the census's
    `_count`), so this check shares nothing with the reader;
  - the run did not panic and left nothing unproved.

Usage (--solver P and --veripb P go before the command):
  python3 tools/card_read_check.py selftest
  python3 tools/card_read_check.py fuzz --cases N [--seed S] [--jobs 4]
  python3 tools/card_read_check.py real FILE.cnf ... [--census DIR] [--veripb-on SUBSTR,..]
  python3 tools/card_read_check.py budget [FILE.cnf ...]

`selftest`: toys for every form and variation, written by the encoders of
tools/card_census.py and tools/card_toy_proof.py, in both signs and with shuffled
inputs; broken copies, which must not read as the same bound unless it still holds.
`selftest --only diagram|adder|network` runs one form's toys and broken copies.
`fuzz`: random mutated encodings (noise, dropped clauses, renamed variables,
flipped signs, repeated literals, tautologies, duplicates, shuffled clauses).
`real`: per cell, the counts by form and n/need against the Python census, the
read ticks and time, and VeriPB on the real proofs when asked.
`budget`: stop at fractions of the total read ticks; the overrun stays within
10 %, and two identical runs give identical ticks.

The solver is target/release/sat-solver unless --solver or CARD_READ_SOLVER says
otherwise. Keep to at most 4 solver processes at once on a busy host (--jobs).
"""
from __future__ import annotations

import argparse
import itertools
import json
import os
import random
import re
import subprocess
import sys
import tempfile
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import card_census as cc  # noqa: E402
import card_toy_proof as ctp  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
VERIPB = ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb"
SOLVER = ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0", "--structdiv=0", "--structdd=0", "--gauss=0", "--card=0"]
LINE = re.compile(r"^c cardread (\w+) n=(\d+) need=(-?\d+) lits=\[(.*)\]$")


class Cfg:
    solver = str(SOLVER)
    veripb = str(VERIPB)
    tmp = None


# ---------------------------------------------------------------------------
# running the reader and the checkers
# ---------------------------------------------------------------------------


# per form, the proof with the most terms written for each term planned
PLANS = {}


class Read:
    def __init__(self):
        self.found = []       # (form, lits, need)
        self.proved = {}      # i -> (id, lines)
        self.unproved = None
        self.ticks = None
        self.stages = {}
        self.ms = None
        self.finished = None
        self.code = None
        self.out = ""


def write_cnf(path, clauses, nv):
    with open(path, "w") as f:
        f.write(f"p cnf {nv} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


def run_reader(cnf, pbp, env_extra=None, args_extra=None, timeout=600):
    env = dict(os.environ)
    for key in list(env):
        if key.startswith("SAT_"):
            del env[key]
    env["SAT_CARD_READ_CHECK"] = str(pbp)
    if env_extra:
        env.update(env_extra)
    cmd = [Cfg.solver, "-q", *STOCK, "--card=1", "--conflicts=0", *(args_extra or []), str(cnf)]
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
    r = Read()
    r.code = p.returncode
    r.out = p.stderr[-2000:]
    for line in p.stdout.splitlines():
        if not line.startswith("c cardread"):
            continue
        m = LINE.match(line)
        if m:
            lits = [int(t) for t in m.group(4).split(",")] if m.group(4).strip() else []
            r.found.append((m.group(1), lits, int(m.group(3))))
            continue
        t = line.split()
        if t[2] == "proved":
            r.proved[int(t[3])] = (int(t[4]), int(t[5]))
            if len(t) > 7:
                # what the proof wrote against what the proof budget plans for it
                i, written, planned = int(t[3]), int(t[6].split("=")[1]), int(t[7].split("=")[1])
                form = r.found[i][0] if i < len(r.found) else "?"
                worst = PLANS.get(form)
                if worst is None or written * max(worst[1], 1) > worst[0] * max(planned, 1):
                    PLANS[form] = (written, planned, len(r.found[i][1]) if i < len(r.found) else 0)
        elif t[2].startswith("unproved="):
            r.unproved = int(t[2].split("=")[1])
        elif t[2].startswith("ticks="):
            for kv in t[2:]:
                k, v = kv.split("=")
                r.stages[k] = v
            r.ticks = int(r.stages["ticks"])
            r.ms = int(r.stages.get("ms", 0))
            r.finished = r.stages.get("finished") == "true"
    return r


def cut_proof(pbp, out, m_clauses, last_id, checks):
    """The proof up to the reader's last line, then the checks. The hook writes
    the grid reader's lines after the count reader's: those are not checked here."""
    cid = m_clauses
    in_block = False
    with open(pbp) as f, open(out, "w") as o:
        for line in f:
            s = line.strip()
            if s.startswith(("output NONE", "conclusion", "end pseudo")):
                break
            if not in_block and cid >= last_id:
                break
            o.write(line)
            if s.startswith("pbc"):
                in_block = True
                cid += 1
            elif s == "qed;":
                in_block = False
                cid += 1
            elif s.startswith(("pol", "rup", "red")):
                cid += 1
        for c in checks:
            o.write(c + "\n")
        o.write("output NONE ;\nconclusion NONE ;\nend pseudo-Boolean proof ;\n")


def lit_str(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


def check_proof(cnf, pbp, m_clauses, read, timeout=1800):
    """VeriPB on the reader's lines, with an `e` line per count. Returns (ok, why)."""
    if read.unproved not in (0, None):
        return False, f"unproved={read.unproved}"
    if not read.found:
        return True, ""
    if len(read.proved) != len(read.found):
        return False, f"proved {len(read.proved)} of {len(read.found)}"
    checks = []
    last = 0
    for i, (_form, lits, need) in enumerate(read.found):
        cid, _lines = read.proved[i]
        last = max(last, cid)
        checks.append("e " + " ".join(f"1 {lit_str(l)}" for l in lits) + f" >= {need} : {cid} ;")
    cut = str(pbp) + ".cut"
    cut_proof(pbp, cut, m_clauses, last, checks)
    p = subprocess.run([Cfg.veripb, str(cnf), cut], capture_output=True, text=True, timeout=timeout)
    ok = "s VERIFIED NO CONCLUSION" in p.stdout
    return ok, "" if ok else (p.stdout + p.stderr)[-1500:]


def stock_sat(cnf, timeout=300):
    """10 sat, 20 unsat, other unknown."""
    p = subprocess.run([Cfg.solver, "-q", "-n", "--card=0", *STOCK, str(cnf)], capture_output=True, timeout=timeout)
    return p.returncode


def holds(clauses, nv, lits, need, workdir, tag):
    """Is `at least need of lits` implied by the formula? True when the formula
    plus the opposite count is unsatisfiable."""
    extra, top, _ = cc._count(list(lits), "atmost", need - 1, max(nv, max(abs(l) for l in lits)))
    path = Path(workdir) / f"sem{tag}.cnf"
    write_cnf(path, list(clauses) + [list(c) for c in extra], top)
    code = stock_sat(path)
    try:
        path.unlink()
    except OSError:
        pass
    return code == 20, code


def check_all(clauses, nv, workdir, tag, want=None):
    """Read a toy and check everything. Returns (read, list of problems)."""
    cnf = Path(workdir) / f"t{tag}.cnf"
    pbp = Path(workdir) / f"t{tag}.pbp"
    write_cnf(cnf, clauses, nv)
    read = run_reader(cnf, pbp)
    problems = []
    if read.code not in (0, 10, 20):
        problems.append(f"exit code {read.code}: {read.out}")
        return read, problems
    ok, why = check_proof(cnf, pbp, len(clauses), read)
    if not ok:
        problems.append(f"VeriPB: {why}")
    # the plan's line count is what a proof from nothing writes
    if len(read.found) == 1 and read.proved:
        cid, plan = read.proved[0]
        real = lines_written(pbp, len(clauses), cid)
        if real != plan:
            problems.append(f"the plan says {plan} lines, the proof took {real}")
    for i, (form, lits, need) in enumerate(read.found):
        vs = [abs(l) for l in lits]
        if len(set(vs)) != len(vs):
            problems.append(f"count {i} holds a variable twice: {lits}")
        if not 1 <= need < len(lits):
            problems.append(f"count {i} has a useless bound: need {need} of {len(lits)}")
        true, code = holds(clauses, nv, lits, need, workdir, f"{tag}_{i}")
        if not true:
            problems.append(f"count {i} {form} need {need} of {lits} is not implied (stock solver code {code})")
    return read, problems


def lines_written(pbp, m_clauses, last_id):
    """How many lines the proof takes to reach constraint `last_id`."""
    cid = m_clauses
    count = 0
    in_block = False
    with open(pbp) as f:
        next(f)
        for line in f:
            s = line.strip()
            if not in_block and cid >= last_id:
                # the helpers' deletion closes the proof
                count += s.startswith("del")
                break
            count += 1
            if s.startswith("pbc"):
                in_block = True
                cid += 1
            elif s == "qed;":
                in_block = False
                cid += 1
            elif not s.startswith("del"):
                cid += 1
    return count


def key(lits, need):
    return (tuple(sorted(lits)), need)


def has(read, lits, need):
    return key(lits, need) in {key(l, n) for _f, l, n in read.found}


# ---------------------------------------------------------------------------
# encoders beyond the two toy files
# ---------------------------------------------------------------------------


def as_lists(clauses):
    return [list(c) for c in clauses]


def compact_sinz(n, k):
    """No register s_ij with j > i, so the early columns need no overflow clause."""
    t = cc.Toy()
    x = t.new(n)
    x = [x] if n == 1 else x
    s = {}
    for i in range(1, n):
        for j in range(1, min(i, k) + 1):
            s[i, j] = t.new()
    t.add(-x[0], s[1, 1])
    for i in range(2, n):
        t.add(-x[i - 1], s[i, 1])
        t.add(-s[i - 1, 1], s[i, 1])
        for j in range(2, min(i, k) + 1):
            t.add(-x[i - 1], -s[i - 1, j - 1], s[i, j])
            if j <= i - 1:
                t.add(-s[i - 1, j], s[i, j])
        if i - 1 >= k:
            t.add(-x[i - 1], -s[i - 1, k])
    t.add(-x[n - 1], -s[n - 1, k])
    return t, x


def one_way_fixed(n, k, binary_hi=True, fixed_width=True):
    """A one-way counter as the timetable cells write it: every column has all k
    registers, the first column's upper ones false by units, (-s v hi) binary.
    Returns (clauses, nv, inputs, top)."""
    enc = ctp.Enc(n)
    x = list(range(1, n + 1))
    s = {}
    for i in range(1, n + 1):
        for j in range(1, k + 1):
            if fixed_width or j <= i:
                s[i, j] = enc.new()
    for j in range(1, k + 1):
        if (1, j) in s:
            if j == 1:
                enc.add(-s[1, 1], x[0])
            else:
                enc.add(-s[1, j])
    for i in range(2, n + 1):
        for j in range(1, k + 1):
            if (i, j) not in s:
                continue
            v = s[i, j]
            lo = s.get((i - 1, j))
            if lo is None:
                enc.add(-v, x[i - 1])
            else:
                enc.add(-v, lo, x[i - 1])
            if j > 1:
                hi = s[i - 1, j - 1]
                if binary_hi or lo is None:
                    enc.add(-v, hi)
                else:
                    enc.add(-v, lo, hi)
    enc.add(s[n, k])
    return enc.c, enc.nv, x, s[n, k]


def two_way_sinz(n, k):
    """Registers meaning exactly "at least j of the first i": both directions on
    one set of registers, as census test (2) knows it has no dual."""
    t = cc.Toy()
    x = t.new(n)
    s = {}
    for i in range(1, n + 1):
        for j in range(1, k + 1):
            s[i, j] = t.new()
    t.add(-x[0], s[1, 1])
    t.add(-s[1, 1], x[0])
    for j in range(2, k + 1):
        t.add(-s[1, j])
    for i in range(2, n + 1):
        for j in range(1, k + 1):
            # forward
            t.add(-s[i - 1, j], s[i, j])
            if j == 1:
                t.add(-x[i - 1], s[i, 1])
            else:
                t.add(-x[i - 1], -s[i - 1, j - 1], s[i, j])
            # back
            t.add(-s[i, j], s[i - 1, j], x[i - 1])
            if j >= 2:
                t.add(-s[i, j], s[i - 1, j - 1])
    return t, x, s


def merge_fwd(t, A, B, c=None):
    c = len(A) + len(B) if c is None else c
    R = t.new(c) if c > 1 else [t.new()]
    for i in range(len(A) + 1):
        for j in range(len(B) + 1):
            if 1 <= i + j <= c:
                t.add(*([R[i + j - 1]] + ([-A[i - 1]] if i else []) + ([-B[j - 1]] if j else [])))
    return R


def capped_direct(n, k, arity=3):
    """A capped totalizer whose leaves are written out: every m-subset of a group
    implies output m, up to k, and every (k+1)-subset is a clause."""
    t = cc.Toy()
    x = t.new(n)

    def merge(A, B):
        c = min(len(A) + len(B), k)
        R = t.new(c) if c > 1 else [t.new()]
        for i in range(len(A) + 1):
            for j in range(len(B) + 1):
                if i + j == 0:
                    continue
                cl = ([-A[i - 1]] if i else []) + ([-B[j - 1]] if j else [])
                if i + j <= c:
                    t.add(*(cl + [R[i + j - 1]]))
                elif i + j == c + 1:
                    t.add(*cl)
        return R

    def leaf(g):
        c = min(len(g), k)
        R = t.new(c) if c > 1 else [t.new()]
        for m in range(1, c + 1):
            for S in itertools.combinations(g, m):
                t.add(*([-s for s in S] + [R[m - 1]]))
        if len(g) > k:
            for S in itertools.combinations(g, k + 1):
                t.add(*[-s for s in S])
        return R

    vecs = [leaf(x[i:i + arity]) if len(x[i:i + arity]) > 1 else [x[i]] for i in range(0, n, arity)]
    while len(vecs) > 1:
        vecs = [merge(vecs[i], vecs[i + 1]) if i + 1 < len(vecs) else vecs[i] for i in range(0, len(vecs), 2)]
    return t, x


def dd_exact(n, k, third=True, subsumed=False, seed=None):
    """A diagram for exactly k of n, as the rook cells write it: (i, c) with c still
    wanted; a node that wants nothing more forces its input false, one that wants all
    that is left forces it true. `subsumed` writes (-v v child) alone there, as they do.
    Returns (clauses, nv, inputs)."""
    enc = ctp.Enc(n)
    ys = list(range(1, n + 1))
    if seed is not None:
        random.Random(seed).shuffle(ys)
    table = {}

    def node(i, c):
        if c < 0 or c > n - i:
            return False
        if i == n:
            return True
        if (i, c) in table:
            return table[i, c]
        hi, lo = node(i + 1, c - 1), node(i + 1, c)
        v = enc.new()
        table[i, c] = v
        y = ys[i]
        if hi is False or lo is False:
            kid = lo if hi is False else hi
            enc.add(-v, -y if hi is False else y)
            if kid is True:
                pass
            elif subsumed:
                enc.add(-v, kid)
            else:
                enc.add(-v, y if hi is False else -y, kid)
                if third:
                    enc.add(-v, kid)
        else:
            if hi is not True:
                enc.add(-v, -y, hi)
            if lo is not True:
                enc.add(-v, y, lo)
            if third and hi is not True and lo is not True:
                enc.add(-v, hi, lo)
        return v

    root = node(0, k)
    enc.add(root)
    return enc.c, enc.nv, list(range(1, n + 1))


def dd_grid(n, k):
    """An exact grid as the ET cells write it: e(i, c) says exactly c of the first i, both
    ways, an at-most-one and an at-least-one per level, c up to k + 1 with the top one
    false by a unit, and at the edges a node's other side says only "not that child"
    (the rest of the formula rules it out). Unit on e(n, k). Returns (clauses, nv, x)."""
    enc = ctp.Enc(n)
    x = list(range(1, n + 1))
    e = {}
    for i in range(1, n + 1):
        for c in range(0, min(i, k + 1) + 1):
            e[i, c] = enc.new()
    enc.add(x[0], e[1, 0])
    enc.add(-x[0], e[1, 1])
    for i in range(1, n + 1):
        cs = [c for c in range(0, min(i, k + 1) + 1)]
        for p, q in itertools.combinations(cs, 2):
            enc.add(-e[i, p], -e[i, q])
        enc.add(*[e[i, c] for c in cs])
        if (i, k + 1) in e:
            enc.add(-e[i, k + 1])
    for i in range(2, n + 1):
        y = x[i - 1]
        for c in range(0, min(i, k + 1) + 1):
            v = e[i, c]
            hi = e.get((i - 1, c - 1))
            lo = e.get((i - 1, c))
            if hi is not None:
                enc.add(-v, -y, hi)
                enc.add(-v, -hi, y)
                enc.add(v, -y, -hi)
            if lo is not None:
                enc.add(-v, y, lo)
                enc.add(-v, -lo, -y)
                enc.add(v, y, -lo)
    for c in range(0, min(n, k + 1) + 1):
        if c != k:
            enc.add(-e[n, c]) if c != k + 1 else None
    enc.add(e[n, k])
    return enc.c, enc.nv, x


def dd_weighted(ys, ws, need, nv, third=True, literals=True):
    """A reduced diagram for sum of ws[i] ys[i] >= need: equal nodes merged, a node
    with one child left out so that a parent skips levels, and with `literals` a node
    that only says "y_j" written as that literal. Returns (clauses, nv)."""
    enc = ctp.Enc(nv)
    memo = {}
    cons = {}

    def node(i, c):
        if c <= 0:
            return True
        if c > sum(ws[i:]):
            return False
        if (i, c) in memo:
            return memo[i, c]
        hi, lo = node(i + 1, c - ws[i]), node(i + 1, c)
        y = ys[i]
        if hi == lo and type(hi) is type(lo):
            r = hi
        elif literals and hi is True and lo is False:
            r = y
        elif literals and hi is False and lo is True:
            r = -y
        else:
            key = (y, hi, lo)
            if key in cons:
                r = cons[key]
            else:
                v = enc.new()
                cons[key] = v
                if hi is not True:
                    enc.add(*([-v, -y] + ([] if hi is False else [hi])))
                if lo is not True:
                    enc.add(*([-v, y] + ([] if lo is False else [lo])))
                if third and hi is not True and lo is not True and not (hi is False and lo is False):
                    enc.add(*([-v] + [q for q in (hi, lo) if q is not False]))
                r = v
        memo[i, c] = r
        return r

    root = node(0, need)
    if root is True:
        return enc.c, enc.nv
    if root is False:
        enc.c.append([])
        return enc.c, enc.nv
    enc.add(root)
    return enc.c, enc.nv


# ---------------------------------------------------------------------------
# renaming: shuffled variables, flipped inputs, flipped registers
# ---------------------------------------------------------------------------


def variant(clauses, nv, inputs, flip_inputs, flip_regs, rng, shuffle):
    """A literal map over 1..nv. Returns (new clauses, map function)."""
    perm = list(range(1, nv + 1))
    if shuffle:
        rng.shuffle(perm)
    inset = {abs(l) for l in inputs}

    def m(l):
        v = abs(l)
        nv2 = perm[v - 1]
        flip = (v in inset and flip_inputs) or (v not in inset and flip_regs)
        out = nv2 if (l > 0) != flip else -nv2
        return out

    new = [[m(l) for l in c] for c in clauses]
    if shuffle:
        new = [rng.sample(c, len(c)) for c in new]
        rng.shuffle(new)
    return new, m


def run_toy(name, clauses, nv, inputs, expected, rng, workdir, results, forms=None, forbidden=()):
    """All sign and shuffle variants of one toy. `expected`: (lits, need) that must
    be read; `forbidden`: (lits, need) that must not be."""
    n_case = 0
    for flip_inputs, flip_regs, shuffle in itertools.product((False, True), (False, True), (False, True)):
        n_case += 1
        new, m = variant(clauses, nv, inputs, flip_inputs, flip_regs, rng, shuffle)
        read, problems = check_all(new, nv, workdir, f"s{len(results)}")
        for lits, need in expected:
            mapped = [m(l) for l in lits]
            if name.startswith("(any sign) "):
                # an input that changes no node's value has no sign of its own, and an
                # exact grid's lowest level can be read on either of its two nodes
                if not any(nd == need and len(got) == len(mapped) and len({abs(l) for l in got} - {abs(l) for l in mapped}) <= 1 for _f, got, nd in read.found):
                    problems.append(f"not read, in any sign: need {need} of {sorted(mapped)}; read {[(f, len(l), nd) for f, l, nd in read.found]}")
                continue
            if not has(read, mapped, need):
                problems.append(f"not read: need {need} of {sorted(mapped)}; read {[(f, len(l), nd) for f, l, nd in read.found]}")
            if forms:
                got = [f for f, l, nd in read.found if key(l, nd) == key(mapped, need)]
                if got and got[0] not in forms:
                    problems.append(f"read as {got[0]}, wanted {forms}")
        for lits, need in forbidden:
            mapped = [m(l) for l in lits]
            if has(read, mapped, need):
                problems.append(f"read and must not be: need {need} of {sorted(mapped)}")
        tag = f"{name} [inputs {'-' if flip_inputs else '+'} regs {'-' if flip_regs else '+'} {'shuffled' if shuffle else 'in order'}]"
        if problems:
            print(f"FAIL {tag}")
            for p in problems[:6]:
                print("     ", p[:600])
        results.append(not problems)
    print(f"ok   {name}: {n_case} variants" if all(results[-n_case:]) else f"FAIL {name}")


def atmost(x, k):
    return ([-l for l in x], len(x) - k)


def atleast(x, k):
    return (list(x), k)


def toys_sinz():
    out = []
    for n, k, units, last in ((7, 3, True, False), (7, 3, False, False), (6, 2, True, True), (12, 5, True, False), (30, 10, True, False), (9, 1, True, False), (3, 2, True, False), (20, 19, True, False)):
        t, x, s = cc.toy_seq(n, k, units=units, last_column=last)
        out.append((f"Sinz toy_seq n={n} k={k} units={units} last_column={last}", as_lists(t.clauses), t.n, x, [atmost(x, k)]))
    t, x = compact_sinz(7, 3)
    out.append(("Sinz compact, no register above its column, at most 3 of 7", as_lists(t.clauses), t.n, x, [atmost(x, 3)]))
    t, x = compact_sinz(12, 6)
    out.append(("Sinz compact at most 6 of 12", as_lists(t.clauses), t.n, x, [atmost(x, 6)]))
    for n in (3, 5, 8, 13):
        k = max(1, n // 2)
        enc = ctp.Enc(n)
        x = list(range(1, n + 1))
        ctp.sinz(enc, x, k)
        out.append((f"Sinz of card_toy_proof at most {k} of {n}", enc.c, enc.nv, x, [atmost(x, k)]))
    # a unit on a root register: the prefix count, and the cap over all inputs
    t, x, s = cc.toy_seq(7, 3)
    out.append(("Sinz n=7 k=3 with unit -s_{6,3}: at most 2 of the six prefix inputs",
                as_lists(t.clauses) + [[-s[6][3]]], t.n, x, [atmost(x[:6], 2), atmost(x, 3)]))
    return out


def toys_totalizer():
    out = []
    for n, k, arity, both, bounded in ((6, 2, 2, False, False), (7, 3, 2, False, False), (9, 3, 3, False, False), (9, 2, 3, False, True), (8, 3, 2, True, False), (16, 4, 2, False, True), (32, 5, 2, False, True), (27, 2, 3, False, True), (12, 1, 2, False, True), (15, 7, 3, False, False)):
        t, x, R = cc.toy_totalizer(n, k, leaf_arity=arity, both=both, bounded=bounded)
        out.append((f"totalizer toy n={n} k={k} arity={arity} both={both} bounded={bounded}", as_lists(t.clauses), t.n, x, [atmost(x, k)]))
    # two-way with a positive unit: a lower bound; both units: exactly
    t, x, R = cc.toy_totalizer(6, both=True)
    out.append(("totalizer two-way, unit r_3: at least 3", as_lists(t.clauses) + [[R[2]]], t.n, x, [atleast(x, 3)]))
    t, x, R = cc.toy_totalizer(8, both=True)
    out.append(("totalizer two-way, unit r_3 and -r_4: exactly 3", as_lists(t.clauses) + [[R[2]], [-R[3]]], t.n, x, [atleast(x, 3), atmost(x, 3)]))
    for n in (3, 5, 8, 13):
        k = max(1, n // 2)
        x = list(range(1, n + 1))
        enc = ctp.Enc(n)
        root = ctp.totalizer(enc, x, k + 1)
        out.append((f"totalizer of card_toy_proof bounded at {k + 1} with a unit, n={n}", enc.c + [[-root.out(k + 1)]], enc.nv, x, [atmost(x, k)]))
        enc = ctp.Enc(n)
        ctp.totalizer(enc, x, k, True)
        out.append((f"totalizer of card_toy_proof capped at {k}, n={n}", enc.c, enc.nv, x, [atmost(x, k)]))
    # backward: the same clauses over negated literals, at least k
    n, k = 9, 4
    x = list(range(1, n + 1))
    enc = ctp.Enc(n)
    root = ctp.totalizer(enc, [-l for l in x], n - k + 1)
    out.append(("totalizer backward, at least 4 of 9", enc.c + [[-root.out(n - k + 1)]], enc.nv, x, [atleast(x, k)]))
    # one node written out: every m-subset implies output m
    for t_size, c, k in ((6, 3, 2), (8, 4, 3), (14, 3, 2), (5, 5, 3)):
        t = cc.Toy()
        x = t.new(t_size)
        R = t.new(c)
        for m in range(1, c + 1):
            for S in itertools.combinations(x, m):
                t.add(*([-q for q in S] + [R[m - 1]]))
        t.add(-R[k])
        out.append((f"one direct node over {t_size} inputs with {c} outputs, at most {k}", as_lists(t.clauses), t.n, x, [atmost(x, k)]))
    # capped, with leaves written out: the leaves' own overflow is the plain subsets
    for n, k in ((9, 2), (12, 2), (14, 3), (18, 2), (10, 4)):
        for arity in (3, 4):
            t, x = capped_direct(n, k, arity)
            out.append((f"totalizer capped at {k} of {n}, leaves of {arity} written out", as_lists(t.clauses), t.n, x, [atmost(x, k)]))
    # a repeated ternary clause does not hide the node (census 17)
    t, x, R = cc.toy_totalizer(9, 2, leaf_arity=3, bounded=True)
    first = next(c for c in t.clauses if len(c) == 3)
    out.append(("totalizer with a repeated ternary clause", as_lists(t.clauses) + [list(first)], t.n, x, [atmost(x, 2)]))
    # a redundant looser unit does not hide the tight one (census 25)
    t, x, R = cc.toy_totalizer(8, 2)
    out.append(("totalizer with a redundant looser unit", as_lists(t.clauses) + [[-R[5]]], t.n, x, [atmost(x, 2)]))
    # children of four inputs counting all their outputs (census 9, the good half)
    t = cc.Toy()
    xs = t.new(8)
    a = merge_fwd(t, merge_fwd(t, [xs[0]], [xs[1]], 2), merge_fwd(t, [xs[2]], [xs[3]], 2), 4)
    b = merge_fwd(t, merge_fwd(t, [xs[4]], [xs[5]], 2), merge_fwd(t, [xs[6]], [xs[7]], 2), 4)
    r = merge_fwd(t, a, b, 4)
    t.add(-r[3])
    out.append(("totalizer, children exposing all four registers, root -r_4", as_lists(t.clauses), t.n, xs, [atmost(xs, 3)]))
    # a node read by two parents: two counts sharing the proof of the node
    t = cc.Toy()
    xs = t.new(8)
    shared = merge_fwd(t, merge_fwd(t, [xs[0]], [xs[1]]), merge_fwd(t, [xs[2]], [xs[3]]))
    p1 = merge_fwd(t, shared, merge_fwd(t, [xs[4]], [xs[5]]))
    p2 = merge_fwd(t, shared, merge_fwd(t, [xs[6]], [xs[7]]))
    t.add(-p1[3])
    t.add(-p2[4])
    out.append(("two totalizers sharing a node", as_lists(t.clauses), t.n, xs, [atmost(xs[:6], 3), atmost(xs[:4] + xs[6:], 4)]))
    return out


def toys_one_way():
    out = []
    for n, k in ((3, 2), (6, 3), (10, 4), (12, 11), (9, 1), (50, 2), (20, 5)):
        x = list(range(1, n + 1))
        enc = ctp.Enc(n)
        ctp.one_way(enc, x, k)
        out.append((f"one-way of card_toy_proof at least {k} of {n}", enc.c, enc.nv, x, [atleast(x, k)]))
    for n, k, b, fw in ((50, 2, True, True), (50, 3, True, True), (7, 3, False, True), (7, 3, True, False), (9, 4, False, False), (9, 1, True, True)):
        c, nv, x, top = one_way_fixed(n, k, b, fw)
        out.append((f"one-way n={n} k={k} binary_hi={b} fixed_width={fw}", c, nv, x, [atleast(x, k)]))
    t, x, s = two_way_sinz(8, 3)
    out.append(("a counter both ways on one register set, unit s_{8,3}: at least 3 of 8",
                as_lists(t.clauses) + [[s[8, 3]]], t.n, x, [atleast(x, 3)]))
    return out


def toys_subset():
    out = []
    for t_size, m, sign in ((5, 3, -1), (7, 3, 1), (6, 4, -1), (4, 3, 1), (14, 3, -1), (10, 9, 1), (8, 5, -1)):
        t, x, g = cc.toy_binomial(t_size, m, sign)
        lits = [s if sign > 0 else -s for s in x]
        out.append((f"subset every {m}-subset of {t_size}, sign {sign:+d}", as_lists(t.clauses), t.n, x, [(lits, t_size - m + 1)]))
    return out


def toys_diagram():
    out = []
    for n, k in ((4, 2), (5, 3), (6, 3), (10, 4), (12, 11), (8, 7), (20, 5), (30, 15), (16, 2)):
        x = list(range(1, n + 1))
        for third in (True, False):
            for twoway in (False, True):
                enc = ctp.Enc(n)
                ctp.dd_nodes(enc, x, k, twoway, third)
                out.append((f"diagram at least {k} of {n}, third={third}, two-way={twoway}", enc.c, enc.nv, x, [atleast(x, k)]))
    # inputs in another order than their numbers
    rng = random.Random(5)
    for n, k in ((12, 4), (25, 7)):
        x = list(range(1, n + 1))
        ys = rng.sample(x, n)
        for third in (True, False):
            enc = ctp.Enc(n)
            ctp.dd_nodes(enc, ys, k, False, third)
            out.append((f"diagram at least {k} of {n}, levels shuffled, third={third}", enc.c, enc.nv, x, [atleast(x, k)]))
    # at most k, with the census's true terminal: a variable under a unit
    for n, k in ((6, 2), (10, 3), (15, 7), (6, 4)):
        t, x, node, tnode = cc.toy_dd(n, k)
        out.append((f"diagram at most {k} of {n}, a true terminal under a unit", as_lists(t.clauses), t.n, x, [atmost(x, k)]))
    # exactly k: both bounds, in the rook cells' shape
    for n, k in ((6, 3), (9, 2), (14, 5)):
        for subsumed in (False, True):
            for third in (True, False):
                c, nv, x = dd_exact(n, k, third, subsumed)
                out.append((f"diagram exactly {k} of {n}, third={third}, subsumed={subsumed}", c, nv, x, [atleast(x, k), atmost(x, k)]))
        c, nv, x = dd_exact(n, k, seed=n)
        out.append((f"diagram exactly {k} of {n}, levels shuffled", c, nv, x, [atleast(x, k), atmost(x, k)]))
    # two roots over one diagram: both counts, the lower nodes' sums written once
    for third in (True, False):
        n, k = 9, 4
        x = list(range(1, n + 1))
        enc = ctp.Enc(n + 1)
        y = n + 1
        root, table = ctp.dd_nodes(enc, x, k, False, third)
        other = enc.new()
        hi, lo = table[1, k - 1][0], table[1, k][0]
        enc.add(-other, -y, hi)
        enc.add(-other, y, lo)
        enc.add(other)
        out.append((f"two roots over one diagram, third={third}", enc.c, enc.nv, x + [y], [atleast(x, k), atleast(x[1:] + [y], k)]))
    # the ET cells' grid, with its edges ruled out by the rest of the formula
    for n, k in ((6, 2), (9, 4), (12, 3), (16, 6)):
        c, nv, x = dd_grid(n, k)
        # the lowest level is two nodes with no diagram of their own: the second literal
        # stands for x_1, as in the cells
        out.append((f"(any sign) exact grid {k} of {n}, the ET shape", c, nv, x, [(x[1:] + [-(n + 1)], k), (([-l for l in x[1:]] + [n + 1]), n - k)]))
    # a weighted sum: the count is the fewest trues that reach it; merged nodes,
    # levels skipped, literals for children
    for ws, need in (((5, 1, 1, 1, 5, 2), 7), ((3, 3, 3, 2, 2, 1, 1), 8), ((4, 1, 4, 1, 4, 1, 4, 1), 9), ((2, 2, 2, 2, 2, 2, 1), 5)):
        n = len(ws)
        x = list(range(1, n + 1))
        srt = sorted(ws, reverse=True)
        m = next(i + 1 for i in range(n) if sum(srt[:i + 1]) >= need)
        for third in (True, False):
            for literals in (True, False):
                c, nv = dd_weighted(x, ws, need, n, third, literals)
                out.append((f"(any sign) diagram of the sum {ws} >= {need}, third={third}, literals={literals}: at least {m}", c, nv, x, [atleast(x, m)]))
    return out


def compare_gates(enc, bits, k):
    """At most k with the compare written as gates over the bits and one unit: the
    reader does not follow these."""
    y = enc.new()
    prev = None
    for i in range(len(bits) - 1, -1, -1):
        over = enc.new()
        if (k >> i) & 1:
            enc.add(-over, bits[i])
            if prev is not None:
                enc.add(-over, prev)
        else:
            if prev is not None:
                enc.add(-prev, -bits[i], over)
        prev = over
    enc.add(-prev if prev is not None else -y)


def net_clauses(enc, pairs, wires, two_way=False, keep=None):
    """Comparators over `wires`, each (i, j) putting the max on i and the min on j."""
    wires = list(wires)
    gates = []
    for i, j in pairs:
        h, l = enc.new(), enc.new()
        gates.append((wires[i], wires[j], h, l))
        wires[i], wires[j] = h, l
    return gates, wires


def insertion_pairs(n):
    return [(j, j + 1) for i in range(1, n) for j in range(i - 1, -1, -1)]


def bubble_pairs(n):
    return [(j, j + 1) for i in range(n - 1) for j in range(n - 2, i - 1, -1)]


def transposition_pairs(n):
    return [(j, j + 1) for r in range(n) for j in range(r % 2, n - 1, 2)]


def pairwise_pairs(n):
    """The pairwise sorting network, the max on the lower wire."""
    out = []
    a = 1
    while a < n:
        b, c = a, 0
        while b < n:
            out.append((b - a, b))
            b += 1
            c = (c + 1) % a
            if c == 0:
                b += a
        a *= 2
    a //= 4
    e = 1
    while a > 0:
        d = e
        while d > 0:
            b, c = (d + 1) * a, 0
            while b < n:
                out.append((b - d * a, b))
                b += 1
                c = (c + 1) % a
                if c == 0:
                    b += a
            d //= 2
        a //= 2
        e = 2 * e + 1
    return out


def sorts(n, pairs):
    """The 0-1 principle, by brute force: only for small n."""
    for mask in range(1 << n):
        v = [(mask >> i) & 1 for i in range(n)]
        for i, j in pairs:
            v[i], v[j] = max(v[i], v[j]), min(v[i], v[j])
        if any(v[i] < v[i + 1] for i in range(n - 1)):
            return False
    return True


def net_toy(n, pairs, k, unit_all=False, two_way=False, prune=False):
    """A network over n inputs with a unit on output k + 1 (or on all from it on)."""
    enc = ctp.Enc(n)
    x = list(range(1, n + 1))
    gates, wires = net_clauses(enc, pairs, x)
    ctp.write_gates(enc, gates, ctp.cone(gates, wires[k:] if unit_all else [wires[k]]) if prune else None, two_way)
    for o in (wires[k:] if unit_all else [wires[k]]):
        enc.add(-o)
    return enc.c, enc.nv, x


def toys_network():
    out = []
    # Batcher's sort, of any size, with one unit on an output
    for n in (3, 4, 5, 7, 8, 12, 16, 24, 32):
        x = list(range(1, n + 1))
        for k in sorted({2, n // 2, n - 2}):
            if not 0 < k < n or n - k < 2:
                continue
            for prune in (False, True):
                for two_way in (False, True):
                    enc = ctp.Enc(n)
                    gates = []
                    root = ctp.sort_net(enc, x, gates)
                    o = root.wires[k]
                    ctp.write_gates(enc, gates, ctp.cone(gates, [o]) if prune else None, two_way)
                    enc.add(-o)
                    out.append((f"sorting network at most {k} of {n}, one unit, pruned={prune}, two-way={two_way}", enc.c, enc.nv, x, [atmost(x, k)]))
            # every output above the bound false: the sum says it
            enc = ctp.Enc(n)
            gates = []
            root = ctp.sort_net(enc, x, gates)
            ctp.write_gates(enc, gates)
            for o in root.wires[k:]:
                enc.add(-o)
            out.append((f"sorting network at most {k} of {n}, every higher output false", enc.c, enc.nv, x, [atmost(x, k)]))
        # a lower bound: two-way, a positive unit on output m
        m = max(2, n // 2)
        enc = ctp.Enc(n)
        gates = []
        root = ctp.sort_net(enc, x, gates)
        ctp.write_gates(enc, gates, None, True)
        enc.add(root.wires[m - 1])
        if n - m >= 1 and (m >= 2):
            out.append((f"sorting network at least {m} of {n}, two-way, a positive unit", enc.c, enc.nv, x, [atleast(x, m)]))
    # the census's toy
    for k in (2, 3, 5):
        t, x, wire = cc.toy_network(k)
        out.append((f"Batcher of the census, at most {k} of 8", as_lists(t.clauses), t.n, x, [atmost(x, k)]))
        t, x, wire = cc.toy_network(k, two_way=True)
        out.append((f"Batcher of the census two-way, at most {k} of 8", as_lists(t.clauses), t.n, x, [atmost(x, k)]))
    # insertion and bubble sort read as merges with one input
    for n, k in ((6, 2), (9, 4), (20, 3), (40, 2)):
        for name, pairs in (("insertion", insertion_pairs(n)), ("bubble", bubble_pairs(n))):
            assert n > 14 or sorts(n, pairs)
            c, nv, x = net_toy(n, pairs, k)
            out.append((f"{name} network at most {k} of {n}", c, nv, x, [atmost(x, k)]))
    # networks that are no merge of two sorted vectors: by subsets up to 14 inputs
    for n, k in ((5, 2), (8, 3), (11, 4), (14, 2)):
        c, nv, x = net_toy(n, transposition_pairs(n), k)
        out.append((f"odd-even transposition at most {k} of {n}, by subsets", c, nv, x, [atmost(x, k)]))
    for n, k in ((8, 3), (8, 2)):
        pairs = pairwise_pairs(n)
        assert sorts(n, pairs)
        c, nv, x = net_toy(n, pairs, k)
        out.append((f"pairwise network at most {k} of {n}, by subsets", c, nv, x, [atmost(x, k)]))
    # the sum where nothing sorts: comparators in no useful order with the outputs false
    for n in (6, 9, 10):
        pairs = [(i, i + 1) for i in range(0, n - 1, 2)] + [(i, i + 1) for i in range(1, n - 1, 2)]
        enc = ctp.Enc(n)
        x = list(range(1, n + 1))
        gates, wires = net_clauses(enc, pairs, x)
        ctp.write_gates(enc, gates)
        false = wires[3:]
        for o in false:
            enc.add(-o)
        # what the units reach: only those gates are read
        keep = ctp.cone(gates, false)
        made = {w for g in gates for w in g[2:]}
        read = {w for g in gates if g[2] in keep or g[3] in keep for w in g[:2]}
        ins = [v for v in read if v not in made]
        outs = {w for g in gates if g[2] in keep or g[3] in keep for w in g[2:]} - read
        free = len(outs - set(false))
        if 1 < free + 1 < len(ins):
            out.append((f"comparators that do not sort, {n} wires, all but three outputs false: at most {free} of {len(ins)}", enc.c, enc.nv, x, [atmost(ins, free)]))
    return out


def broken_networks(check, workdir, results):
    n, k = 6, 2
    x = list(range(1, n + 1))
    enc = ctp.Enc(n)
    gates = []
    root = ctp.sort_net(enc, x, gates)
    ctp.write_gates(enc, gates)
    good = as_lists(enc.c)
    o = root.wires[k]
    cl = good + [[-o]]
    for drop in cl:
        check(f"sorting network at most {k} of {n}, {drop} dropped", [c for c in cl if c != drop], enc.nv, atmost(x, k))
    # the unit one output up and one down
    for j in (k - 1, k + 1):
        check(f"sorting network, the unit on output {j + 1}: no at most {k} unless j is k", good + [[-root.wires[j]]], enc.nv, atmost(x, k))
    # the same for the sum: one output's unit gone
    enc2 = ctp.Enc(n)
    gates2 = []
    root2 = ctp.sort_net(enc2, x, gates2)
    ctp.write_gates(enc2, gates2)
    us = [[-q] for q in root2.wires[k:]]
    for drop in us:
        check(f"sorting network, every higher output false but {drop}", as_lists(enc2.c) + [u for u in us if u != drop], enc2.nv, atmost(x, k))
    # a comparator left out
    a, b, h, l = gates[len(gates) // 2]
    without = [c for c in good if c not in ([-a, h], [-b, h], [-a, -b, l])]
    check(f"sorting network with one comparator's clauses gone", without + [[-o]], enc.nv, atmost(x, k))
    # a wire swapped
    u = gates[-1]
    v = gates[-2]
    swap = [[(v[3] if q == u[0] else -v[3] if q == -u[0] else q) for q in c] for c in good]
    check(f"sorting network with a wire of the last comparator read from elsewhere", swap + [[-o]], enc.nv, atmost(x, k))
    # a network that does not sort: nothing says at most k
    pairs = [(0, 1), (2, 3), (1, 2)]
    c4, nv4, x4 = net_toy(4, pairs, 1)
    check("three comparators on four wires, one unit on output 2: no at most 1", c4, nv4, atmost(x4, 1))
    # two comparators on one wire, and a wire read twice
    enc5 = ctp.Enc(4)
    h1, l1 = enc5.new(), enc5.new()
    enc5.add(-1, h1)
    enc5.add(-2, h1)
    enc5.add(-1, -2, l1)
    h2, l2 = enc5.new(), enc5.new()
    enc5.add(-l1, h2)
    enc5.add(-h1, h2)
    enc5.add(-l1, -h1, l2)
    h3, l3 = enc5.new(), enc5.new()
    enc5.add(-l1, h3)
    enc5.add(-3, h3)
    enc5.add(-l1, -3, l3)
    enc5.add(-l2)
    enc5.add(-l3)
    check("a wire read by two comparators", as_lists(enc5.c), enc5.nv, ([-1, -2, -3], 2))
    # two comparators that share an input: the parts of the last one overlap, so no merge
    # of two vectors holds, though every subset of the inputs runs to the wire
    enc8 = ctp.Enc(3)
    h1, l1 = enc8.new(), enc8.new()
    h2, l2 = enc8.new(), enc8.new()
    h3, l3 = enc8.new(), enc8.new()
    ctp.write_gates(enc8, [(1, 2, h1, l1), (1, 3, h2, l2), (h1, h2, h3, l3)])
    enc8.add(-l3)
    read, problems = check_all(as_lists(enc8.c), enc8.nv, workdir, f"p{len(results)}")
    ok = not problems and has(read, *atmost([1, 2, 3], 1))
    print(f"{'ok  ' if ok else 'FAIL'} two comparators sharing an input: at most 1 of 3, read by subsets, not by a merge")
    for p_ in problems[:3]:
        print("     ", p_[:300])
    results.append(ok)
    # the same with the shared input first in the reading's order: it alone sets the wire,
    # so the rank reads as 1, at most none, and that is no bound here
    enc8 = ctp.Enc(3)
    h1, l1 = enc8.new(), enc8.new()
    h2, l2 = enc8.new(), enc8.new()
    h3, l3 = enc8.new(), enc8.new()
    ctp.write_gates(enc8, [(1, 2, h1, l1), (2, 3, h2, l2), (h1, h2, h3, l3)])
    enc8.add(-l3)
    read, problems = check_all(as_lists(enc8.c), enc8.nv, workdir, f"p{len(results)}")
    ok = not problems and not read.found
    print(f"{'ok  ' if ok else 'FAIL'} two comparators sharing their first input: at most 1 of 3 holds, not read, the rank is a guess (a known limit)")
    results.append(ok)
    # the same over more inputs than the subsets take: not read
    enc9 = ctp.Enc(16)
    wires = []
    for i in range(1, 16):
        h, l = enc9.new(), enc9.new()
        ctp.write_gates(enc9, [(i, i + 1, h, l)])
        wires.append(h)
    cur = wires[0]
    for w in wires[1:]:
        h, l = enc9.new(), enc9.new()
        ctp.write_gates(enc9, [(cur, w, h, l)])
        cur = l
    enc9.add(-cur)
    check("a chain of comparators over overlapping pairs, 16 inputs", as_lists(enc9.c), enc9.nv, atmost(list(range(1, 17)), 1))
    # a positive unit on a network written one way is no lower bound
    enc6 = ctp.Enc(n)
    gates6 = []
    root6 = ctp.sort_net(enc6, x, gates6)
    ctp.write_gates(enc6, gates6)
    enc6.add(root6.wires[2])
    check("a one-way network, a positive unit on output 3: no at least 3", as_lists(enc6.c), enc6.nv, ([l for l in x], 3))
    # a two-way network with one comparator's way back gone: no lower bound through it
    enc7b = ctp.Enc(n)
    gates7b = []
    root7b = ctp.sort_net(enc7b, x, gates7b)
    ctp.write_gates(enc7b, gates7b, None, True)
    a7, b7, h7, l7 = gates7b[0]
    two = [c for c in as_lists(enc7b.c) if c != [-h7, a7, b7]] + [[root7b.wires[2]]]
    check("a two-way network missing one (-h a b), a positive unit on output 3", two, enc7b.nv, ([l for l in x], 3))
    # known limits, each with a toy
    for name, pairs, m in (("odd-even transposition", transposition_pairs(16), 16), ("pairwise", pairwise_pairs(16), 16)):
        cl6, nv6, x6 = net_toy(m, pairs, 3)
        read, problems = check_all(cl6, nv6, workdir, f"p{len(results)}")
        got = has(read, *atmost(x6, 3))
        ok = not problems and not got
        print(f"{'ok  ' if ok else 'FAIL'} {name} on {m} wires with one unit: not read (no merge of sorted vectors, over the {14} inputs of the subsets; a known limit)")
        results.append(ok)
    # a unit on a wire in the middle of a merge: true, but no rank to read
    enc7 = ctp.Enc(8)
    gates7 = []
    root7 = ctp.sort_net(enc7, list(range(1, 9)), gates7)
    ctp.write_gates(enc7, gates7)
    mid = gates7[-3][3]
    enc7.add(-mid)
    read, problems = check_all(as_lists(enc7.c), enc7.nv, workdir, f"p{len(results)}")
    print(f"{'ok  ' if not problems else 'FAIL'} a unit on a wire inside a merge: {sorted(Counter((f, len(l), nd) for f, l, nd in read.found).items())}, all true")
    results.append(not problems)


def toys_adder():
    out = []
    for n in (3, 4, 5, 7, 8, 11, 15, 16, 30):
        x = list(range(1, n + 1))
        for k in sorted({2, n // 2, n - 1}):
            if not 1 <= k < n:
                continue
            # a bound of one is a clause in disguise
            most = n - k >= 2
            least = k >= 2
            for both in (True, False):
                # at most k by compare clauses
                enc = ctp.Enc(n)
                _cells, bits = ctp.popcount(enc, x, both)
                ctp.compare_at_most(enc, bits, k)
                if most:
                    out.append((f"adder at most {k} of {n}, compare clauses, both ways={both}", enc.c, enc.nv, x, [atmost(x, k)]))
            # at least k: the two-way cells over negated literals
            enc = ctp.Enc(n)
            _cells, bits = ctp.popcount(enc, x, True)
            ctp.compare_at_least(enc, bits, k)
            if least:
                out.append((f"adder at least {k} of {n}, compare clauses", enc.c, enc.nv, x, [atleast(x, k)]))
            # exactly k by units on every bit
            enc = ctp.Enc(n)
            _cells, bits = ctp.popcount(enc, x, True)
            for i, b in enumerate(bits):
                enc.add(b if (k >> i) & 1 else -b)
            want = ([atleast(x, k)] if least else []) + ([atmost(x, k)] if most else [])
            out.append((f"adder exactly {k} of {n}, a unit per bit", enc.c, enc.nv, x, want))
        # the top bits false: at most the rest
        enc = ctp.Enc(n)
        _cells, bits = ctp.popcount(enc, x, True)
        if len(bits) >= 3:
            enc.add(-bits[-1])
            top = (1 << (len(bits) - 1)) - 1
            if 2 <= top < n and n - top >= 2:
                out.append((f"adder of {n}, the top bit false: at most {top}", enc.c, enc.nv, x, [atmost(x, top)]))
    # the census's chain of full adders
    for const in (2, 3, 5):
        t, x, bits = cc.toy_popcount(7, const)
        out.append((f"adder chain of the census, sum = {const} of 7", as_lists(t.clauses), t.n, x, [atleast(x, const), atmost(x, const)]))
    return out


def broken_adders(check, workdir, results):
    n, k = 7, 3
    x = list(range(1, n + 1))
    for both in (True, False):
        enc = ctp.Enc(n)
        cells, bits = ctp.popcount(enc, x, both)
        ctp.compare_at_most(enc, bits, k)
        cl = as_lists(enc.c)
        for drop in cl:
            check(f"adder at most {k} of {n}, both ways={both}, {drop} dropped", [c for c in cl if c != drop], enc.nv, atmost(x, k))
        # a wire swapped: two inputs of different cells, in every clause of one cell
        inputs, s, m, w = cells[0]
        other = cells[1][0][0]
        cl2 = [[(other if l == inputs[0] else -other if l == -inputs[0] else l) for l in c] for c in cl]
        check(f"adder at most {k} of {n}, both ways={both}, input {inputs[0]} replaced by {other}", cl2, enc.nv, atmost(x, k))
        # a sum read twice
        s0 = cells[0][1]
        cl3 = [[(s0 if l == x[-1] else -s0 if l == -x[-1] else l) for l in c] for c in cl]
        check(f"adder at most {k} of {n}, both ways={both}, a sum read in place of input {x[-1]}", cl3, enc.nv, atmost(x, k))
        # a carry read at the weight of its inputs
        m0 = cells[0][2]
        cl4 = [[(m0 if l == x[-1] else -m0 if l == -x[-1] else l) for l in c] for c in cl]
        check(f"adder at most {k} of {n}, both ways={both}, a carry read in place of input {x[-1]}", cl4, enc.nv, atmost(x, k))
        # the compare off by one
        enc2 = ctp.Enc(n)
        cells2, bits2 = ctp.popcount(enc2, x, both)
        ctp.compare_at_most(enc2, bits2, k + 1)
        check(f"adder compared with {k + 1}: no at most {k}", as_lists(enc2.c), enc2.nv, atmost(x, k))
        # a unit moved between bits
        enc3 = ctp.Enc(n)
        cells3, bits3 = ctp.popcount(enc3, x, both)
        enc3.add(-bits3[0])
        check(f"adder with a unit on the low bit instead: no at most {k}", as_lists(enc3.c), enc3.nv, atmost(x, k))
    # three low bits of one weight: the top-down number is too high here, and the
    # propagation does not close on it. 3 full adders and one over their carries
    enc = ctp.Enc(9)
    s1, m1 = ctp.full_adder(enc, 1, 2, 3)
    s2, m2 = ctp.full_adder(enc, 4, 5, 6)
    s3, m3 = ctp.full_adder(enc, 7, 8, 9)
    s4, m4 = ctp.full_adder(enc, m1, m2, m3)
    for q in (s1, s2, s3):
        enc.add(-s4, -q)
    check("three bits of one weight under compare clauses: at most 7 of 9, not at most 6", as_lists(enc.c), enc.nv, atmost(list(range(1, 10)), 6))
    # a compare clause on the wrong side
    enc = ctp.Enc(n)
    cells, bits = ctp.popcount(enc, x, True)
    ctp.compare_at_least(enc, bits, k)
    check(f"adder at least {k}: no at most {k}", as_lists(enc.c), enc.nv, atmost(x, k))
    # a bit fixed the other way
    enc = ctp.Enc(n)
    cells, bits = ctp.popcount(enc, x, True)
    enc.add(bits[-1])
    check("adder, the top bit true: no at most 3", as_lists(enc.c), enc.nv, atmost(x, 3))
    # a two-bit word adder with its sum fixed: two weights among the inputs, no count
    enc = ctp.Enc(4)
    s0_, c0 = ctp.half_adder(enc, 1, 3)
    s1, c1 = ctp.full_adder(enc, 2, 4, c0)
    enc.add(-c1)
    enc.add(s0_)
    enc.add(s1)
    read, problems = check_all(as_lists(enc.c), enc.nv, workdir, f"p{len(results)}")
    ok = not problems and not read.found
    print(f"{'ok  ' if ok else 'FAIL'} a word adder (inputs of two weights) is no count: {len(read.found)} counts")
    results.append(ok)
    # what the reader leaves out, each with a toy
    enc = ctp.Enc(8)
    _cells, bits = ctp.popcount(enc, list(range(1, 9)), True)
    compare_gates(enc, bits, 3)
    read, problems = check_all(as_lists(enc.c), enc.nv, workdir, f"p{len(results)}")
    got = has(read, *atmost(list(range(1, 9)), 3))
    ok = not problems and not got
    print(f"{'ok  ' if ok else 'FAIL'} an adder compared through gates with their own variables: not read (a known limit)")
    results.append(ok)
    # half adders alone with a compare clause and no unit: nothing to start from
    enc = ctp.Enc(3)
    s1_, c1_ = ctp.half_adder(enc, 1, 2)
    s2_, c2_ = ctp.half_adder(enc, s1_, 3)
    s3_, c3_ = ctp.half_adder(enc, c1_, c2_)
    enc.add(-s2_, -s3_)
    read, problems = check_all(as_lists(enc.c), enc.nv, workdir, f"p{len(results)}")
    ok = not problems and not has(read, *atmost([1, 2, 3], 2))
    print(f"{'ok  ' if ok else 'FAIL'} half adders alone, a compare clause and no unit: not read, though at most 2 of 3 is only a clause (a known limit)")
    results.append(ok)
    # the same with a unit on a bit: a network of half adders alone reads from there
    for n3, compare in ((3, False),):
        enc = ctp.Enc(4)
        s1_, c1_ = ctp.half_adder(enc, 1, 2)
        s2_, c2_ = ctp.half_adder(enc, s1_, 3)
        s3_, c3_ = ctp.half_adder(enc, s2_, 4)
        s4_, c4_ = ctp.half_adder(enc, c1_, c2_)
        s5_, c5_ = ctp.half_adder(enc, s4_, c3_)
        s6_, c6_ = ctp.half_adder(enc, c4_, c5_)
        enc.add(-s6_)
        enc.add(-c6_)
        read, problems = check_all(as_lists(enc.c), enc.nv, workdir, f"p{len(results)}")
        ok = not problems and has(read, *atmost([1, 2, 3, 4], 3)) is False and len(read.found) >= 0
        print(f"{'ok  ' if not problems else 'FAIL'} half adders alone over four inputs with units on the top bits: {sorted(Counter((f, len(l), nd) for f, l, nd in read.found).items())}")
        results.append(not problems)
    # a full adder built from two half adders and an OR
    enc = ctp.Enc(9)
    n9 = 9
    wires = list(range(1, n9 + 1))
    # at most k through two-gate full adders: not read
    def fa2(a, b, c):
        t, m1 = ctp.half_adder(enc, a, b)
        s, m2 = ctp.half_adder(enc, t, c)
        o = enc.new()
        enc.add(-m1, o)
        enc.add(-m2, o)
        enc.add(m1, m2, -o)
        return s, o
    s1, c1 = fa2(wires[0], wires[1], wires[2])
    s2, c2 = fa2(wires[3], wires[4], wires[5])
    s3, c3 = fa2(wires[6], wires[7], wires[8])
    s4, c4 = fa2(s1, s2, s3)
    s5, c5 = fa2(c1, c2, c3)
    s6, c6 = ctp.half_adder(enc, s5, c4)
    s7, c7 = ctp.half_adder(enc, c5, c6)
    enc.add(-c7)
    read, problems = check_all(as_lists(enc.c), enc.nv, workdir, f"p{len(results)}")
    print(f"{'ok  ' if not problems else 'FAIL'} a popcount of two-gate full adders: {sorted(Counter((f, len(l), nd) for f, l, nd in read.found).items())}")
    results.append(not problems)


def broken_diagrams(check, workdir, results):
    # the plan's lines with the edge lemmas: the two bounds share nothing, so the first
    # count's proof takes exactly what its plan says
    for n, k in ((6, 2), (9, 4)):
        c, nv, x = dd_grid(n, k)
        cnf = Path(workdir) / f"plan{n}.cnf"
        pbp = Path(workdir) / f"plan{n}.pbp"
        write_cnf(cnf, c, nv)
        read = run_reader(cnf, pbp)
        cid, plan = read.proved[0]
        real = lines_written(pbp, len(c), cid)
        ok = read.found[0][0] == "Diagram" and real == plan
        print(f"{'ok  ' if ok else 'FAIL'} exact grid {k} of {n}: the first count's plan says {plan} lines, the proof took {real}")
        results.append(ok)
    # a root asserted only by a clause with a repeated literal: its coefficient is two, and
    # the sum with the unit would not come out as the count
    x = list(range(1, 7))
    enc = ctp.Enc(6)
    root, _table = ctp.dd_nodes(enc, x, 3, False, False)
    cl = [c for c in as_lists(enc.c) if c != [root]] + [[root, root]]
    read, problems = check_all(cl, enc.nv, workdir, f"p{len(results)}")
    ok = not problems and not has(read, *atleast(x, 3))
    print(f"{'ok  ' if ok else 'FAIL'} a diagram's root asserted by (root root) only: not read, its unit has another coefficient")
    results.append(ok)
    # at most k - 1 and at least n of n are left out: a bound of one is a clause
    t, x, node, tnode = cc.toy_dd(5, 4)
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and not has(read, *atmost(x, 4))
    print(f"{'ok  ' if ok else 'FAIL'} a diagram for at most 4 of 5 says one clause: not read as a count")
    results.append(ok)
    for n, k, third in ((6, 3, True), (6, 3, False), (7, 2, False)):
        x = list(range(1, n + 1))
        enc = ctp.Enc(n)
        root, table = ctp.dd_nodes(enc, x, k, False, third)
        cl = as_lists(enc.c)
        for drop in cl:
            check(f"diagram at least {k} of {n}, third={third}, {drop} dropped", [c for c in cl if c != drop], enc.nv, atleast(x, k))
        # a wire swapped: one child replaced by another node of the same level
        nodes = list(table.values())
        for v, hi, lo in nodes[:4]:
            for q, other in ((hi, lo), (lo, hi)):
                if isinstance(q, bool) or isinstance(other, bool):
                    continue
                cl2 = [[other if l == q and (-v) in c else l for l in c] for c in cl]
                check(f"diagram at least {k} of {n}, third={third}, node {v}'s child {q} rewired to {other}", cl2, enc.nv, atleast(x, k))
        # an input swapped in one clause of a node
        v, hi, lo = nodes[0]
        cl2 = [list(c) for c in cl]
        for c in cl2:
            if -v in c and -x[0] in c:
                c[c.index(-x[0])] = -x[1]
        check(f"diagram at least {k} of {n}, third={third}, the root reads another input on one side", cl2, enc.nv, atleast(x, k))
        # the unit moved one node down: at most one less
        below = table[1, k][0] if (1, k) in table else table[1, k - 1][0]
        check(f"diagram at least {k} of {n}, third={third}, the unit moved down", cl[:-1] + [[below]], enc.nv, atleast(x, k))
        # the root unit gone
        check(f"diagram at least {k} of {n}, third={third}, no unit", cl[:-1], enc.nv, atleast(x, k))
        # the root unit the other way round
        check(f"diagram at least {k} of {n}, third={third}, the unit negated", cl[:-1] + [[-root]], enc.nv, atleast(x, k))
        # one input's sign turned round in every clause of one level
        cl3 = [[(-l if abs(l) == x[1] else l) for l in c] for c in cl]
        check(f"diagram at least {k} of {n}, third={third}, an input negated", cl3, enc.nv, atleast(x, k))
    # a node both ways round in the order: no level order
    enc = ctp.Enc(4)
    a, b, c_ = enc.new(), enc.new(), enc.new()
    enc.add(-a, 1, b)
    enc.add(-a, -1, c_)
    enc.add(-b, 2, a)
    enc.add(-b, -2, c_)
    enc.add(-c_, 3, 4)
    enc.add(-c_, -3, 4)
    enc.add(a)
    check("a diagram in a cycle", as_lists(enc.c), enc.nv, ([1, 2, 3, 4], 2))


def broken_check(workdir, results):
    """Broken copies: `gone` is the bound of the good copy. It must not be read
    unless it still holds, and whatever is read must be true."""

    def check(name, clauses, nv, gone):
        # a dropped clause the proof does not need leaves the bound true, and it
        # may then be read; a bound that no longer holds must not be
        read, problems = check_all(clauses, nv, workdir, f"b{len(results)}")
        lits, need = gone
        got = has(read, lits, need)
        true, _code = holds(clauses, nv, lits, need, workdir, f"bb{len(results)}")
        if got and not true:
            problems.append("read a false bound")
        if problems:
            print(f"FAIL broken: {name}")
            for p in problems[:5]:
                print("     ", p[:600])
        else:
            print(f"ok   broken: {name}: bound {'read and true' if got else 'not read'}; read {sorted(Counter((f, len(l), nd) for f, l, nd in read.found).items())[:6]}")
        results.append(not problems)

    return check


def broken(workdir, results, rng):
    check = broken_check(workdir, results)
    broken_diagrams(check, workdir, results)
    broken_adders(check, workdir, results)
    broken_networks(check, workdir, results)
    # Sinz
    t, x, s = cc.toy_seq(7, 3)
    cl = as_lists(t.clauses)
    for pos in range(0, len([c for c in cl if len(c) == 3]), 2):
        drop = [c for c in cl if len(c) == 3][pos]
        check(f"Sinz n=7 k=3, ternary {drop} dropped", [c for c in cl if c != drop], t.n, atmost(x, 3))
    for pos, drop in enumerate([c for c in cl if len(c) == 2 and c[0] < 0 and c[1] < 0]):
        check(f"Sinz n=7 k=3, overflow {drop} dropped", [c for c in cl if c != drop], t.n, atmost(x, 3))
    for pos, drop in enumerate([c for c in cl if len(c) == 2 and (c[0] > 0) != (c[1] > 0)][:6]):
        check(f"Sinz n=7 k=3, binary {drop} dropped", [c for c in cl if c != drop], t.n, atmost(x, 3))
    # overflow row off by one (at most 3 reads no more)
    cl3 = [([c[0], -s[3][2]] if len(c) == 2 and c[0] < 0 and c[1] < 0 and -c[1] == s[3][3] else c) for c in cl]
    check("Sinz, one overflow row points at register k-1", cl3, t.n, atmost(x, 2))
    # a register of another column swapped in
    cl4 = [[s[4][2] if l == s[5][2] else (-s[4][2] if l == -s[5][2] else l) for l in c] for c in cl]
    check("Sinz, a register swapped between columns", cl4, t.n, atmost(x, 2))
    # one input read twice
    cl5 = [[x[0] if l == x[5] else (-x[0] if l == -x[5] else l) for l in c] for c in cl]
    check("Sinz, an input wired to another input", cl5, t.n, atmost(x[:5] + x[6:], 2))
    # totalizer: the census's review toys
    t, x, R = cc.toy_totalizer(9, 3, leaf_arity=3)
    cl = as_lists(t.clauses)
    for pos in (3, 7, 11, 19):
        drop = [c for c in cl if len(c) == 3][pos]
        check(f"totalizer 9 k=3 arity 3, ternary {drop} dropped", [c for c in cl if c != drop], t.n, atmost(x, 3))
    t, x, R = cc.toy_totalizer(6, 3)
    cl = as_lists(t.clauses)
    check("totalizer at most 3 of 6, the unit moved up to r_5: no at most 2", [c for c in cl if len(c) != 1] + [[-R[4]]], t.n, atmost(x, 2))
    # a positive unit on a forward-only output is no lower bound (census 2)
    t, x, R = cc.toy_totalizer(6)
    check("forward totalizer, unit r_3: no at least 3", as_lists(t.clauses) + [[R[2]]], t.n, atleast(x, 3))
    # reverse clauses of the asserted register removed (census 4)
    t, x, R = cc.toy_totalizer(6, both=True)
    check("two-way totalizer, r_3's reverse clauses removed, unit r_3", [list(c) for c in t.clauses if -R[2] not in c] + [[R[2]]], t.n, atleast(x, 3))
    # children that hide counts (census 9, the bad half)
    t = cc.Toy()
    xs = t.new(8)
    a = merge_fwd(t, merge_fwd(t, [xs[0]], [xs[1]], 2), merge_fwd(t, [xs[2]], [xs[3]], 2), 2)
    b = merge_fwd(t, merge_fwd(t, [xs[4]], [xs[5]], 2), merge_fwd(t, [xs[6]], [xs[7]], 2), 2)
    r = merge_fwd(t, a, b, 4)
    t.add(-r[3])
    check("children counting to 2 of 4, no overflow, root -r_4", as_lists(t.clauses), t.n, atmost(xs, 3))
    # a child's top register linked to the wrong output (census 10)
    t = cc.Toy()
    xa, xb = t.new(3), t.new(3)
    A = merge_fwd(t, merge_fwd(t, [xa[0]], [xa[1]]), [xa[2]])
    B = merge_fwd(t, merge_fwd(t, [xb[0]], [xb[1]]), [xb[2]])
    R10 = merge_fwd(t, A[:2], B)
    t.add(-A[2], R10[4])
    t.add(-R10[3])
    check("a_3 linked to r_5, unit -r_4", as_lists(t.clauses), t.n, atmost(xa + xb, 3))
    # the restored registers lack overflow clauses (census 12)
    t = cc.Toy()
    xa, xb = t.new(3), t.new(3)
    A = merge_fwd(t, merge_fwd(t, [xa[0]], [xa[1]]), [xa[2]])
    B = merge_fwd(t, merge_fwd(t, [xb[0]], [xb[1]]), [xb[2]])
    R12 = merge_fwd(t, A[:2], B[:2], 3)
    t.add(-A[1], -B[1])
    t.add(-A[2], R12[2])
    t.add(-B[2], R12[2])
    check("a capped root over children whose top registers have no overflow", as_lists(t.clauses), t.n, atmost(xa + xb, 3))
    # nodes in a cycle (census 1): must finish
    p_, q_, r1, r2, s1, s2, z, t1, t2, t3 = range(1, 11)
    cyc = [[-s1, r1], [-s2, r2], [-p_, r1], [-s1, -p_, r2], [-s2, -p_], [-r1, s1], [-r2, s2], [-q_, s1], [-r1, -q_, s2], [-r2, -q_],
           [-r1, t1], [-r2, t2], [-z, t1], [-r1, -z, t2], [-r2, -z, t3]]
    t0 = time.time()
    check("merge nodes in a cycle", cyc, 10, ([-p_, -q_, -z], 1))
    results.append(time.time() - t0 < 5)
    # complementary inputs, repeated inputs (census 24, 26)
    tF, xF, RF = cc.toy_totalizer(6, 3)
    ren = {xF[3]: -xF[2], xF[4]: -xF[1], xF[5]: -xF[0]}
    clF = [[ren.get(l, -ren[-l] if -l in ren else l) for l in c] for c in tF.clauses]
    check("at most 3 of six literals that are three complementary pairs", clF, tF.n, atmost(xF[:3] + [-xF[2], -xF[1], -xF[0]], 3))
    tI, xI, RI = cc.toy_totalizer(6, 3)
    renI = {xI[3]: xI[0], xI[4]: xI[1], xI[5]: xI[2]}
    clI = [[(renI.get(abs(l), abs(l)) if l > 0 else -renI.get(abs(l), abs(l))) for l in c] for c in tI.clauses]
    check("a totalizer reading three inputs twice", clI, tI.n, atmost(xI[:3], 1))
    # one-way
    for n, k in ((6, 3), (10, 4)):
        x = list(range(1, n + 1))
        enc = ctp.Enc(n)
        ctp.one_way(enc, x, k)
        cl = as_lists(enc.c)
        for pos in range(0, len(cl) - 1, 3):
            drop = cl[pos]
            check(f"one-way at least {k} of {n}, {drop} dropped", [c for c in cl if c != drop], enc.nv, atleast(x, k))
        # the unit moved one register down: a lower bound off by one
        s = enc.nv
        check(f"one-way at least {k} of {n}, unit on another register", cl[:-1] + [[s - 1]], enc.nv, atleast(x, k))
        # a register swapped with the next one in one clause
        ter = [c for c in cl if len(c) == 3]
        victim = ter[len(ter) // 2]
        cl2 = [list(c) for c in cl]
        cl2[cl.index(victim)] = [victim[0], victim[1] - 1 if victim[1] > n + 1 else victim[1] + 1, victim[2]]
        check(f"one-way at least {k} of {n}, a register swapped in {victim}", cl2, enc.nv, atleast(x, k))
    c, nv, x, top = one_way_fixed(20, 3, True, True)
    check("one-way, positive unit removed", [q for q in c if q != [top]], nv, atleast(x, 3))
    # subset
    t, x, g = cc.toy_binomial(5, 3, -1)
    cl = as_lists(t.clauses)
    for drop in cl[::3]:
        check(f"at most 2 of 5, subset {drop} missing", [c for c in cl if c != drop], t.n, atmost(x, 2))
    t, x, g = cc.toy_binomial(4, 3, 1, guard=True)
    check("guarded at least 2 of 4: no plain count", as_lists(t.clauses), t.n, atleast(x, 2))
    t, x, g = cc.toy_binomial(4, 2, -1)
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and not read.found
    print(f"{'ok  ' if ok else 'FAIL'} pairwise at most one is left to the grid reader: {len(read.found)} counts")
    results.append(ok)
    t, x, g = cc.toy_binomial(15, 3, -1)
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and not read.found
    print(f"{'ok  ' if ok else 'FAIL'} 15 literals are over the subset limit of 14: {len(read.found)} counts")
    results.append(ok)
    # a pair of inputs plus one excluded literal is a clause in disguise, not a count
    t = cc.Toy()
    a_, b_, u_ = t.new(3)
    R = merge_fwd(t, [a_], [b_], 2)
    t.add(-R[1], -u_)
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and not read.found
    print(f"{'ok  ' if ok else 'FAIL'} one pair node with its top register excluded by a literal: {len(read.found)} counts")
    results.append(ok)
    # what the reader leaves out, each with a toy (a claim needs a run)
    t, x, R = cc.toy_totalizer(8, 3)
    cl = as_lists(t.clauses)
    nv = t.n
    for extra in range(16):
        nv += 1
        cl.append([-nv, R[0]])
    read, problems = check_all(cl, nv, workdir, f"p{len(results)}")
    ok = not problems and not has(read, *atmost(x, 3))
    print(f"{'ok  ' if ok else 'FAIL'} a root first output implied by 18 literals is no register (over the limit of 16): not read")
    results.append(ok)
    cl = as_lists(t.clauses)
    nv = t.n
    for extra in range(14):
        nv += 1
        cl.append([-nv, R[0]])
    read, problems = check_all(cl, nv, workdir, f"p{len(results)}")
    ok = not problems and has(read, *atmost(x, 3))
    print(f"{'ok  ' if ok else 'FAIL'} the same with 16 implying literals: read")
    results.append(ok)
    # a direct node over more inputs than the proof's table takes
    t = cc.Toy()
    x = t.new(15)
    R = t.new(3)
    for m in range(1, 4):
        for S in itertools.combinations(x, m):
            t.add(*([-q for q in S] + [R[m - 1]]))
    t.add(-R[2])
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and not has(read, *atmost(x, 2))
    print(f"{'ok  ' if ok else 'FAIL'} a direct node over 15 inputs is over the limit of 14: not read")
    results.append(ok)
    # a bound that asks for every literal is units, which propagation already has
    t, x, R = cc.toy_totalizer(6)
    read, problems = check_all(as_lists(t.clauses) + [[-R[0]]], t.n, workdir, f"p{len(results)}")
    ok = not problems and not read.found
    print(f"{'ok  ' if ok else 'FAIL'} a unit on the first output (at most none) is left to propagation: {len(read.found)} counts")
    results.append(ok)
    # a chain that counts to one is a long clause: only from five inputs on
    for n, want in ((4, False), (5, True)):
        x = list(range(1, n + 1))
        enc = ctp.Enc(n)
        ctp.one_way(enc, x, 1)
        read, problems = check_all(enc.c, enc.nv, workdir, f"p{len(results)}")
        ok = not problems and has(read, *atleast(x, 1)) == want
        print(f"{'ok  ' if ok else 'FAIL'} a one-way chain over {n} inputs: {'read' if want else 'not read'}")
        results.append(ok)
    # a register in more clauses than a counter gives it is no register
    n, k = 8, 3
    x = list(range(1, n + 1))
    enc = ctp.Enc(n)
    ctp.one_way(enc, x, k)
    cl = as_lists(enc.c)
    top = cl[-1][0]
    nv = enc.nv
    for _ in range(7):
        nv += 1
        cl.append([-top, nv])
    read, problems = check_all(cl, nv, workdir, f"p{len(results)}")
    ok = not problems and not has(read, *atleast(x, k))
    print(f"{'ok  ' if ok else 'FAIL'} a one-way register with 9 clauses is over the limit of 8: not read")
    results.append(ok)
    # two trees over the same children: told apart by the numbers of their outputs
    t = cc.Toy()
    xs = t.new(4)
    a = merge_fwd(t, [xs[0]], [xs[1]])
    b = merge_fwd(t, [xs[2]], [xs[3]])
    r1 = merge_fwd(t, a, b, 3)
    r2 = merge_fwd(t, a, b, 3)
    t.add(-r1[1])
    t.add(-r2[2])
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and has(read, *atmost(xs, 1)) and has(read, *atmost(xs, 2))
    print(f"{'ok  ' if ok else 'FAIL'} two trees over one pair of children, outputs numbered in order: both read")
    results.append(ok)
    # the same with the outputs numbered in turn: the guess takes the other tree's
    # output and the first tree is lost, which costs a count and nothing else
    t = cc.Toy()
    xs = t.new(4)
    a = merge_fwd(t, [xs[0]], [xs[1]])
    b = merge_fwd(t, [xs[2]], [xs[3]])
    base = t.n
    t.n += 6
    r1 = [base + 1, base + 3, base + 5]
    r2 = [base + 2, base + 4, base + 6]
    for R in (r1, r2):
        for i in range(3):
            for j in range(3):
                if 1 <= i + j <= 3:
                    t.add(*([R[i + j - 1]] + ([-a[i - 1]] if i else []) + ([-b[j - 1]] if j else [])))
    t.add(-r2[2])
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and not has(read, *atmost(xs, 2))
    print(f"{'ok  ' if ok else 'FAIL'} two trees over one pair of children, outputs numbered in turn, the unit on the second tree: not read (a known limit)")
    results.append(ok)
    # commanders: pairwise groups under a pairwise group, left to the grid reader
    t = cc.Toy()
    groups = [t.new(3) for _ in range(3)]
    com = []
    for g in groups:
        c_ = t.new()
        com.append(c_)
        for a, b in itertools.combinations(g, 2):
            t.add(-a, -b)
        for a in g:
            t.add(-a, c_)
    for a, b in itertools.combinations(com, 2):
        t.add(-a, -b)
    read, problems = check_all(as_lists(t.clauses), t.n, workdir, f"p{len(results)}")
    ok = not problems and not read.found
    print(f"{'ok  ' if ok else 'FAIL'} commanders over groups of 3 are the grid reader's: {len(read.found)} counts")
    results.append(ok)
    # a dirty clause in a sum: only a copy with a repeated literal
    t, x, g = cc.toy_binomial(5, 3, -1)
    cl = as_lists(t.clauses)
    cl2 = [list(c) for c in cl]
    cl2[3] = cl2[3] + [cl2[3][0]]
    read, problems = check_all(cl2, t.n, workdir, f"p{len(results)}")
    got = has(read, *atmost(x, 2))
    ok = not problems and not got
    print(f"{'ok  ' if ok else 'FAIL'} a subset clause with a repeated literal is not summed: bound {'read' if got else 'not read'}")
    results.append(ok)
    cl2 = cl + [cl[3] + [cl[3][0]]]
    read, problems = check_all(cl2, t.n, workdir, f"p{len(results)}")
    got = has(read, *atmost(x, 2))
    ok = not problems and got
    print(f"{'ok  ' if ok else 'FAIL'} a clean copy next to the repeated-literal copy is used: bound {'read' if got else 'not read'}")
    results.append(ok)


def cmd_selftest(args):
    rng = random.Random(args.seed)
    results = []
    with tempfile.TemporaryDirectory(dir=Cfg.tmp) as d:
        if args.only:
            toys = {"diagram": toys_diagram, "adder": toys_adder, "network": toys_network}.get(args.only, lambda: [])()
            forms = {"diagram": ("Diagram",), "adder": ("Adder",), "network": ("Network", "Totalizer", "Sinz", "OneWay")}.get(args.only)
            for toy_name, clauses, nv, x, expected in toys:
                run_toy(toy_name, clauses, nv, x, expected, rng, d, results, forms=forms)
            check = broken_check(d, results)
            if args.only == "diagram":
                broken_diagrams(check, d, results)
            elif args.only == "adder":
                broken_adders(check, d, results)
            elif args.only == "network":
                broken_networks(check, d, results)
            print(f"{sum(results)} of {len(results)} checks passed")
            return 0 if all(results) else 1
        for name, toys in (("Sinz", toys_sinz()), ("totalizer", toys_totalizer()), ("one-way", toys_one_way()), ("subset", toys_subset()), ("diagram", toys_diagram()), ("adder", toys_adder()), ("network", toys_network())):
            print(f"--- {name}")
            # a tree with a leaf at every node is a chain, so a small totalizer reads as Sinz
            forms = {"Sinz": ("Sinz",), "totalizer": ("Totalizer", "Sinz"), "one-way": ("OneWay",), "subset": ("Subset",), "diagram": ("Diagram",), "adder": ("Adder",), "network": ("Network", "Totalizer", "Sinz", "OneWay")}[name]
            for toy_name, clauses, nv, x, expected in toys:
                run_toy(toy_name, clauses, nv, x, expected, rng, d, results, forms=forms)
        print("--- broken copies")
        broken(d, results, rng)
    # the proof budget plans at least what each form's proofs write
    for form, (written, planned, n) in sorted(PLANS.items()):
        ok = written <= planned
        results.append(ok)
        print(f"{'ok  ' if ok else 'FAIL'} plan for {form}: {written} terms written, {planned} planned, on {n} literals")
    good = sum(results)
    print(f"{good} of {len(results)} checks passed")
    print("ALL OK" if all(results) else "FAILURES")
    return 0 if all(results) else 1


# ---------------------------------------------------------------------------
# fuzz
# ---------------------------------------------------------------------------


def random_base(rng):
    kind = rng.choice(["seq", "seq", "compact", "tot", "tot3", "bounded", "capped", "twoway", "oneway", "oneway2", "both", "subset", "sinzproof", "many",
                       "dd", "dd", "ddexact", "ddmost", "ddpb", "adder", "adder", "adderunits", "net", "net", "netsum", "netbad"])
    n = rng.randint(3, 16)
    k = rng.randint(1, max(1, n - 1))
    if kind in ("net", "netsum", "netbad"):
        x = list(range(1, n + 1))
        rng.shuffle(x)
        enc = ctp.Enc(n)
        gates = []
        if kind == "netbad":
            pairs = [(rng.randrange(n), rng.randrange(n)) for _ in range(rng.randint(1, 3 * n))]
            pairs = [(i, j) for i, j in pairs if i != j]
            gates, wires = net_clauses(enc, pairs, x)
        else:
            root = ctp.sort_net(enc, x, gates)
            wires = root.wires
        wanted = wires[k:] if kind == "netsum" else [wires[min(k, len(wires) - 1)]]
        ctp.write_gates(enc, gates, ctp.cone(gates, wanted) if rng.random() < 0.4 else None, rng.random() < 0.3)
        for o in wanted:
            enc.add(-o if rng.random() < 0.9 else o)
        return enc.c, enc.nv
    if kind == "adder":
        x = list(range(1, n + 1))
        rng.shuffle(x)
        enc = ctp.Enc(n)
        _cells, bits = ctp.popcount(enc, x, rng.random() < 0.6)
        if rng.random() < 0.5:
            ctp.compare_at_most(enc, bits, k)
        else:
            ctp.compare_at_least(enc, bits, k)
        return enc.c, enc.nv
    if kind == "adderunits":
        x = list(range(1, n + 1))
        rng.shuffle(x)
        enc = ctp.Enc(n)
        _cells, bits = ctp.popcount(enc, x, True)
        for i, b in enumerate(bits):
            if rng.random() < 0.7:
                enc.add(b if (k >> i) & 1 else -b)
        return enc.c, enc.nv
    if kind == "dd":
        x = list(range(1, n + 1))
        rng.shuffle(x)
        enc = ctp.Enc(n)
        ctp.dd_nodes(enc, x, k, rng.random() < 0.3, rng.random() < 0.5)
        return enc.c, enc.nv
    if kind == "ddexact":
        c, nv, x = dd_exact(n, k, rng.random() < 0.5, rng.random() < 0.5, seed=rng.randint(0, 99))
        return c, nv
    if kind == "ddmost":
        t, x, node, tnode = cc.toy_dd(n, k)
        return as_lists(t.clauses), t.n
    if kind == "ddpb":
        ws = [rng.randint(1, 4) for _ in range(n)]
        x = list(range(1, n + 1))
        rng.shuffle(x)
        c, nv = dd_weighted(x, ws, rng.randint(1, sum(ws)), n, rng.random() < 0.5, rng.random() < 0.5)
        return c, max(nv, 1)
    if kind == "seq":
        t, x, s = cc.toy_seq(n, k, units=rng.random() < 0.7, last_column=rng.random() < 0.3)
        return as_lists(t.clauses), t.n
    if kind == "compact":
        t, x = compact_sinz(n, k)
        return as_lists(t.clauses), t.n
    if kind == "tot":
        t, x, R = cc.toy_totalizer(n, k if k < n else None)
        return as_lists(t.clauses), t.n
    if kind == "tot3":
        n = rng.randint(3, 15)
        k = rng.randint(1, n - 1)
        t, x, R = cc.toy_totalizer(n, k, leaf_arity=3, bounded=rng.random() < 0.7)
        return as_lists(t.clauses), t.n
    if kind == "bounded":
        t, x, R = cc.toy_totalizer(n, k, bounded=True)
        return as_lists(t.clauses), t.n
    if kind == "capped":
        enc = ctp.Enc(n)
        x = list(range(1, n + 1))
        ctp.totalizer(enc, x, k, True)
        return enc.c, enc.nv
    if kind == "twoway":
        t, x, R = cc.toy_totalizer(n, both=True)
        c = as_lists(t.clauses)
        if rng.random() < 0.5:
            c.append([R[k - 1]])
        else:
            c.append([-R[k - 1]])
        return c, t.n
    if kind == "oneway":
        x = list(range(1, n + 1))
        enc = ctp.Enc(n)
        ctp.one_way(enc, x, k)
        return enc.c, enc.nv
    if kind == "oneway2":
        c, nv, x, top = one_way_fixed(n, k, rng.random() < 0.5, rng.random() < 0.5)
        return c, nv
    if kind == "both":
        t, x, s = two_way_sinz(n, k)
        return as_lists(t.clauses) + [[s[n, k]]], t.n
    if kind == "subset":
        t_size = rng.randint(4, 8)
        m = rng.randint(3, t_size - 1)
        t, x, g = cc.toy_binomial(t_size, m, rng.choice((-1, 1)), guard=rng.random() < 0.1)
        return as_lists(t.clauses), t.n
    if kind == "sinzproof":
        enc = ctp.Enc(n)
        x = list(range(1, n + 1))
        ctp.sinz(enc, x, k)
        return enc.c, enc.nv
    if rng.random() < 0.3:
        t, x = capped_direct(rng.randint(4, 14), rng.randint(1, 4), rng.randint(2, 4))
        return as_lists(t.clauses), t.n
    # two encodings over shared inputs
    enc = ctp.Enc(n)
    x = list(range(1, n + 1))
    rng.shuffle(x)
    ctp.sinz(enc, x, min(k, 3))
    y = rng.sample(x, rng.randint(2, n))
    root = ctp.totalizer(enc, y, min(rng.randint(1, 3) + 1, len(y)))
    enc.add(-root.out(root.width()))
    return enc.c, enc.nv


def mutate(clauses, nv, rng):
    c = [list(q) for q in clauses]
    done = []

    def lit():
        return rng.randint(1, nv) * rng.choice((1, -1))

    for _ in range(rng.choice((0, 0, 1, 1, 2, 4))):
        if c:
            c.pop(rng.randrange(len(c)))
            done.append("drop")
    for _ in range(rng.choice((0, 0, 1, 2, 5))):
        c.append([lit() for _ in range(rng.randint(1, 4))])
        done.append("noise")
    if rng.random() < 0.01:
        c.insert(rng.randrange(len(c) + 1), [])
        done.append("empty")
    for _ in range(rng.choice((0, 0, 1, 3))):
        if c:
            q = list(rng.choice(c))
            c.insert(rng.randrange(len(c) + 1), q)
            done.append("duplicate")
    for _ in range(rng.choice((0, 0, 1, 2))):
        if c:
            q = rng.choice(c)
            if q:
                q.append(rng.choice(q))
                done.append("repeat")
    for _ in range(rng.choice((0, 0, 0, 1))):
        if c:
            i = rng.randrange(len(c))
            l = lit()
            if rng.random() < 0.5:
                c[i] = c[i] + [l, -l]
            else:
                c.insert(i, [l, -l, lit()])
            done.append("tautology")
    for _ in range(rng.choice((0, 0, 0, 1, 2))):
        if c:
            q = rng.choice(c)
            if q:
                q[rng.randrange(len(q))] = lit()
                done.append("replace")
    if rng.random() < 0.25:
        a, b = rng.randint(1, nv), rng.randint(1, nv)
        swap = {a: b, b: a}
        for q in c:
            if rng.random() < 0.5:
                for i, l in enumerate(q):
                    if abs(l) in swap:
                        q[i] = swap[abs(l)] if l > 0 else -swap[abs(l)]
        done.append("swap")
    # renamed variables, flipped signs
    perm = list(range(1, nv + 1))
    rng.shuffle(perm)
    flip = [rng.random() < 0.5 for _ in range(nv + 1)]
    mode = rng.random()
    spread = 1
    if mode < 0.05:
        spread = 1000  # sparse numbering

    def m(l):
        v = abs(l)
        w = perm[v - 1] * spread if mode < 0.5 else v
        s = l > 0
        if mode < 0.5 and flip[v]:
            s = not s
        return w if s else -w

    c = [[m(l) for l in q] for q in c]
    new_nv = nv * spread
    for q in c:
        rng.shuffle(q)
    if rng.random() < 0.7:
        rng.shuffle(c)
    return c, new_nv, done


def fuzz_case(args):
    seed, base_dir = args
    rng = random.Random(seed)
    clauses, nv = random_base(rng)
    if rng.random() < 0.3:
        c2, nv2 = random_base(rng)
        # a second encoding on its own variables, or sharing some
        off = nv if rng.random() < 0.6 else max(0, nv - rng.randint(1, 3))
        clauses = clauses + [[l + off if l > 0 else l - off for l in q] for q in c2]
        nv = off + nv2
    clauses, nv, done = mutate(clauses, nv, rng)
    nv = max(nv, 1)
    d = Path(base_dir) / f"c{seed}"
    d.mkdir(parents=True, exist_ok=True)
    try:
        read, problems = check_all(clauses, nv, d, "f")
        return seed, len(read.found), Counter(f for f, _l, _n in read.found), problems, done
    except Exception as e:  # noqa: BLE001
        return seed, 0, Counter(), [f"{type(e).__name__}: {e}"], done
    finally:
        for p in d.iterdir():
            try:
                p.unlink()
            except OSError:
                pass
        try:
            d.rmdir()
        except OSError:
            pass


def cmd_fuzz(args):
    failures = 0
    total_found = Counter()
    with_found = 0
    t0 = time.time()
    with tempfile.TemporaryDirectory(dir=Cfg.tmp) as d:
        with ThreadPoolExecutor(args.jobs) as pool:
            work = [(args.seed + i, d) for i in range(args.cases)]
            for n, (seed, nfound, forms, problems, done) in enumerate(pool.map(fuzz_case, work)):
                total_found.update(forms)
                with_found += nfound > 0
                if problems:
                    failures += 1
                    print(f"FAIL seed {seed} {done}")
                    for p in problems[:4]:
                        print("     ", p[:500])
                if (n + 1) % 200 == 0:
                    print(f"... {n + 1} cases, {failures} failures, {sum(total_found.values())} counts, {round(time.time() - t0)} s", flush=True)
    print(f"{args.cases} cases, {with_found} with counts, counts by form {dict(total_found)}, {failures} failures, {round(time.time() - t0)} s")
    return 0 if failures == 0 else 1


# ---------------------------------------------------------------------------
# real cells, against the census
# ---------------------------------------------------------------------------


def parse_plain(path):
    from rl_structure import normalize
    clauses = []
    nv = 0
    header = 0
    cur = []
    with open(path) as f:
        for line in f:
            if not line or line[0] in "c%":
                continue
            if line[0] == "p":
                t = line.split()
                nv, header = int(t[2]), int(t[3])
                continue
            for tok in line.split():
                v = int(tok)
                if v == 0:
                    c = normalize(cur)
                    cur = []
                    if c is not None:
                        clauses.append(c)
                else:
                    cur.append(v)
    return nv, clauses, header


def census_counts(path, census_dir):
    stem = Path(path).name
    cached = Path(census_dir) / f"{stem}.json" if census_dir else None
    if cached and cached.exists():
        return json.load(open(cached))
    nv, clauses, _ = parse_plain(path)
    t0 = time.time()
    res = cc.analyse(clauses, nv, want_pop=False)
    cn = cc.counted(res)

    def tree(t):
        return {"kind": t["kind"], "n": t["n"], "bound": list(t["bound"]), "leaves": t["leaves"], "mirror": t["mirror"], "dual": t["dual"],
                "unresolved": t["unresolved"], "cyclic": t["cyclic"], "saturates": t["saturates"], "dropped": t.get("bound_dropped", False)}

    data = {"vars": nv, "clauses": len(clauses), "secs": round(time.time() - t0, 1),
            "trees": [tree(t) for t in res["trees"]],
            "counted": {k: [tree(t) for t in cn[k]] for k in ("seq", "seq_free", "tot", "tot_free", "mirror", "pairs", "partial")},
            "binom": [{"S": f["S"], "t": f["t"], "m": f["m"], "form": list(f["form"]), "guard": f["guard"]} for f in res["binom"]],
            "dd": [{"nodes": d["nodes"], "levels": d["levels"], "max_count": d["max_count"], "asserted": d["asserted"], "bound": list(d["bound"]) if d.get("bound") else None} for d in cn["dd"]],
            "nets": [{"gates": d["gates"], "inputs": d["n"], "groups": [{"n": g["n"], "bound": list(g["bound"])} for g in d["groups"]]} for d in cn["nets"]],
            "net_free": len(cn["net_free"]), "pop": [{"cells": p["cells"], "leaves": p["leaves"], "use": p["use"]} for p in cn["pop"]]}
    if cached:
        json.dump(data, open(cached, "w"))
    return data


def cmd_real(args):
    bad = 0
    want_veripb = [s for s in (args.veripb_on or "").split(",") if s]
    with tempfile.TemporaryDirectory(dir=Cfg.tmp) as d:
        for path in args.files:
            name = Path(path).name
            pbp = Path(d) / "real.pbp"
            read = run_reader(path, pbp)
            print(f"=== {name}: exit {read.code}, {len(read.found)} counts, ticks {read.ticks} {read.stages}, unproved {read.unproved}")
            tally = Counter((f, len(l), n) for f, l, n in read.found)
            print("  read:", ", ".join(f"{f} n={n} need={nd} x{v}" for (f, n, nd), v in sorted(tally.items(), key=lambda kv: -kv[1])[:30]))
            total_lines = sum(v[1] for v in read.proved.values())
            print(f"  proof lines by plan: {total_lines}; largest count {max((v[1] for v in read.proved.values()), default=0)}")
            census = census_counts(path, args.census)
            print(f"  census: {census['secs']} s")
            mine = {key(l, n): f for f, l, n in read.found}
            # what the census counts or finds with a bound
            matched = Counter()
            missing = []
            for group in ("seq", "tot"):
                for t in census["counted"][group]:
                    k_ = t["bound"][1]
                    want = key([-l for l in t["leaves"]], t["n"] - k_)
                    if want in mine:
                        matched[(group, t["n"], k_)] += 1
                    else:
                        missing.append((group, t["n"], k_, t["leaves"][:8]))
            print("  census counted and read here:", sorted(matched.items(), key=lambda kv: -kv[1])[:20])
            print(f"  census counted and NOT read here: {len(missing)}", missing[:6])
            known = set()
            for group in ("seq", "tot", "seq_free", "tot_free", "partial", "mirror", "pairs"):
                for t in census["counted"][group]:
                    if t["bound"][0] == "atmost":
                        known.add(key([-l for l in t["leaves"]], t["n"] - t["bound"][1]))
            partial = [t for t in census["counted"]["partial"] if t["bound"][0] == "atmost"]
            print(f"  census partial trees with a bound (not counted there): {len(partial)}, by n: {Counter(t['n'] for t in partial).most_common(12)}")
            beyond = Counter((f, len(l), n) for f, l, n in read.found if key(l, n) not in known)
            print("  read here, not in the census's trees:", ", ".join(f"{f} n={n} need={nd} x{v}" for (f, n, nd), v in sorted(beyond.items(), key=lambda kv: -kv[1])[:30]))
            print(f"  census diagrams: {[(d['nodes'], d['levels'], d['max_count'], d['asserted'], d['bound']) for d in census.get('dd', [])][:6]}")
            print(f"  census networks: {[(d['gates'], d['inputs'], d['groups'][:3]) for d in census.get('nets', [])][:4]}, free {census.get('net_free')}")
            print(f"  census adder networks (candidates): {[(p['cells'], p['leaves'], p['use']) for p in census.get('pop', [])][:4]}")
            binom = [f for f in census["binom"] if not f["guard"]]
            subset_mine = {key(l, n) for f, l, n in read.found if f == "Subset"}
            bm = sum(1 for f in binom if key(f["S"], f["t"] - f["m"] + 1) in subset_mine)
            print(f"  census unguarded subset families {len(binom)}, read here {bm}; guarded (no plain count) {len(census['binom']) - len(binom)}")
            if any(s in name for s in want_veripb):
                t0 = time.time()
                _nv, clauses, header = parse_plain(path)
                ok, why = check_proof(path, pbp, header, read)
                print(f"  VeriPB on the real proofs: {'s VERIFIED NO CONCLUSION' if ok else 'FAILED ' + why} in {round(time.time() - t0, 1)} s, every count's `e` line included")
                bad += not ok
            if read.code not in (0, 10, 20) or read.unproved:
                bad += 1
    return 0 if bad == 0 else 1


# ---------------------------------------------------------------------------
# budget
# ---------------------------------------------------------------------------


def adversaries():
    """Toys that once made passes overrun: per-variable tables, duplicate clauses."""
    out = []
    t, x, s = cc.toy_seq(200, 20)
    out.append(("Sinz n=200 k=20", as_lists(t.clauses), t.n))
    t, x, R = cc.toy_totalizer(128, 10, bounded=True)
    out.append(("totalizer n=128 bounded at 11", as_lists(t.clauses), t.n))
    c, nv, x, top = one_way_fixed(150, 12)
    out.append(("one-way n=150 k=12", c, nv))
    t, x, g = cc.toy_binomial(14, 5, -1)
    out.append(("subset 5 of 14", as_lists(t.clauses), t.n))
    # a stray variable makes every per-variable table huge
    t, x, s = cc.toy_seq(40, 5)
    out.append(("Sinz n=40 and a stray variable 2,000,000", as_lists(t.clauses) + [[2_000_000, 1]], 2_000_000))
    # 60,000 copies of one clause
    t, x, s = cc.toy_seq(40, 5)
    out.append(("Sinz n=40 and 60,000 copies of one clause", as_lists(t.clauses) + [[1, 2, 3]] * 60000, t.n))
    # a hub literal
    t, x, s = cc.toy_seq(40, 5)
    hub = t.new()
    nvh = t.n
    cl = as_lists(t.clauses)
    for i in range(30000):
        nvh += 1
        cl.append([-hub, nvh])
    out.append(("Sinz n=40 and a hub literal implying 30,000 others", cl, nvh))
    # many ternaries through one pair of literals
    nvh = 2
    cl = []
    for i in range(30000):
        nvh += 1
        cl.append([-1, -2, nvh])
    out.append(("30,000 ternaries through one pair", cl, nvh))
    # a large pairwise group
    cl = [[-a, -b] for a in range(1, 300) for b in range(a + 1, 300)]
    out.append(("a pairwise group of 299", cl, 299))
    # the forms with proofs over many terms
    x = list(range(1, 201))
    enc = ctp.Enc(200)
    ctp.dd_nodes(enc, x, 50, False, True)
    out.append(("diagram at least 50 of 200", enc.c, enc.nv))
    enc = ctp.Enc(120)
    ctp.dd_nodes(enc, list(range(1, 121)), 30, True, False)
    out.append(("diagram both ways, at least 30 of 120", enc.c, enc.nv))
    c, nv, _x = dd_grid(60, 12)
    out.append(("an exact grid, 12 of 60", c, nv))
    # many units, each a small diagram, and many units on one large one
    enc = ctp.Enc(300)
    for i in range(0, 280, 6):
        ctp.dd_nodes(enc, list(range(i + 1, i + 7)), 3, False, True)
    out.append(("47 small diagrams, each with its unit", enc.c, enc.nv))
    enc = ctp.Enc(80)
    root, table = ctp.dd_nodes(enc, list(range(1, 81)), 20, False, True)
    for v, _h, _l in list(table.values())[:60]:
        enc.add(v)
    out.append(("one diagram with 60 of its nodes asserted", enc.c, enc.nv))
    # a chain of nodes that look like clauses: all wait for a decision variable
    enc = ctp.Enc(1)
    prev = enc.new()
    enc.add(prev)
    for i in range(5000):
        nxt, y = enc.new(), enc.new()
        enc.add(-prev, y, nxt)
        prev = nxt
    out.append(("a chain of 5,000 one-clause nodes", enc.c, enc.nv))
    enc = ctp.Enc(500)
    _cells, bits = ctp.popcount(enc, list(range(1, 501)), True)
    ctp.compare_at_most(enc, bits, 100)
    out.append(("adders at most 100 of 500", enc.c, enc.nv))
    # adders everywhere and no bound: every cell is looked at
    enc = ctp.Enc(3000)
    for i in range(0, 2997, 3):
        ctp.full_adder(enc, i + 1, i + 2, i + 3)
    out.append(("1,000 full adders, no bound", enc.c, enc.nv))
    enc = ctp.Enc(128)
    gates = []
    root = ctp.sort_net(enc, list(range(1, 129)), gates)
    ctp.write_gates(enc, gates)
    enc.add(-root.wires[10])
    out.append(("sorting network, at most 10 of 128, one unit", enc.c, enc.nv))
    enc = ctp.Enc(64)
    gates = []
    root = ctp.sort_net(enc, list(range(1, 65)), gates)
    ctp.write_gates(enc, gates, None, True)
    for o in root.wires[20:]:
        enc.add(-o)
    out.append(("sorting network two-way, at most 20 of 64, the sum", enc.c, enc.nv))
    enc = ctp.Enc(14)
    pairs = transposition_pairs(14)
    gates, wires = net_clauses(enc, pairs, list(range(1, 15)))
    ctp.write_gates(enc, gates)
    enc.add(-wires[6])
    out.append(("transposition network on 14 wires, by subsets", enc.c, enc.nv))
    return out


def budget_case(name, cnf, m_clauses, fractions, results):
    d = Path(cnf).parent
    full = run_reader(cnf, d / "full.pbp")
    again = run_reader(cnf, d / "again.pbp")
    same = full.ticks == again.ticks and [key(l, n) for _f, l, n in full.found] == [key(l, n) for _f, l, n in again.found]
    ns = (full.ms * 1e6 / full.ticks) if full.ticks else 0
    print(f"{name}: total {full.ticks} ticks, {len(full.found)} counts, {full.ms} ms, {ns:.0f} ns a tick; two runs identical: {same}")
    # a formula with a stray large variable pays for tables per variable: what comes last
    # may then not start, which is the limit doing its work
    results.append(same and (full.finished or "stray" in name))
    worst = 0.0
    for frac in fractions:
        limit = max(1, int(full.ticks * frac))
        r1 = run_reader(cnf, d / "cut.pbp", env_extra={"SAT_LIMIT_TICKS": str(limit)})
        r2 = run_reader(cnf, d / "cut2.pbp", env_extra={"SAT_LIMIT_TICKS": str(limit)})
        used = r1.ticks if r1.ticks is not None else 0
        over = used / limit if limit else 0
        worst = max(worst, over)
        good = over <= 1.10 and r1.ticks == r2.ticks and r1.code in (0, 10, 20) and r1.unproved in (0, None)
        ok, why = check_proof(cnf, d / "cut.pbp", m_clauses, r1) if r1.found else (True, "")
        good = good and ok
        results.append(good)
        print(f"   limit {frac:>5.0%} = {limit:>10}: used {used:>10} ({over:6.1%} of the limit), {len(r1.found):>5} counts, {r1.ms} ms, same twice {r1.ticks == r2.ticks}, proofs {'ok' if ok else 'FAILED ' + why[:200]}" + ("" if good else "  FAIL"))
    print(f"   worst use of a limit: {worst:.1%}")


def cmd_budget(args):
    results = []
    fractions = (0.02, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99)
    with tempfile.TemporaryDirectory(dir=Cfg.tmp) as d:
        for name, clauses, nv in adversaries():
            cnf = Path(d) / "b.cnf"
            write_cnf(cnf, clauses, nv)
            budget_case(name, cnf, len(clauses), fractions, results)
        for path in args.files:
            _nv, _c, header = parse_plain(path)
            local = Path(d) / Path(path).name
            os.symlink(os.path.abspath(path), local)
            budget_case(Path(path).name, local, header, fractions, results)
    print(f"{sum(results)} of {len(results)} checks passed")
    print("ALL OK" if all(results) else "FAILURES")
    return 0 if all(results) else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--solver", default=os.environ.get("CARD_READ_SOLVER", str(SOLVER)))
    ap.add_argument("--veripb", default=str(VERIPB))
    ap.add_argument("--tmp", default=os.environ.get("CARD_READ_TMP"))
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("selftest")
    s.add_argument("--seed", type=int, default=7)
    s.add_argument("--only", help="the toys of one form")
    s.set_defaults(func=cmd_selftest)
    f = sub.add_parser("fuzz")
    f.add_argument("--cases", type=int, default=1000)
    f.add_argument("--seed", type=int, default=1)
    f.add_argument("--jobs", type=int, default=4)
    f.set_defaults(func=cmd_fuzz)
    r = sub.add_parser("real")
    r.add_argument("files", nargs="+")
    r.add_argument("--census", help="a directory of cached census results")
    r.add_argument("--veripb-on", help="comma-separated parts of the file names to run VeriPB on")
    r.set_defaults(func=cmd_real)
    b = sub.add_parser("budget")
    b.add_argument("files", nargs="*")
    b.set_defaults(func=cmd_budget)
    args = ap.parse_args()
    Cfg.solver = args.solver
    Cfg.veripb = args.veripb
    Cfg.tmp = args.tmp
    os.environ["CARD_CENSUS_SOLVER"] = args.solver
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
