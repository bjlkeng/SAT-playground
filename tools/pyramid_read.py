#!/usr/bin/env python3
"""Read a Bel's Pyramid CNF (the 2025 'bp' family) back to its puzzle (bead
SAT-playground-d96.3).

The cells come from https://github.com/acmihal/bel_pyramid (SAT Competition 2025
benchmark descriptions, 'Bel's Pyramid Puzzle', Mihal and Haba): a Z3Py model
exported with Then('lia2card', 'dt2bv', With('card2bv', pb.solver=totalizer),
'bit-blast', 'tseitin-cnf').  What that leaves in the CNF:

  placements  c{cube}_r{rot}_x{x}_y{y}_h{h}: cube `cube` turned to `rot` at
              position (x, y, h).  Numbered 1..P by the generator in the string
              order of their names (checked: every cube's at-least-one clause is
              exactly its placements).
  labels      one integer 0..2N-2 per axis line (X per (x, h), Y per (y, h), Z per
              (x, y)); lia2card writes each as 2N-2 bits in order encoding (bit k
              <-> label > k, chain clauses bit[k+1] -> bit[k]).
  channel     placement <-> [X = r0] and [Y = r1] and [Z = r2]; each value test is
              a PbEq over all of the label's bits, compiled to a totalizer.

A label's bits are found from the value-0 tests: the placements of cube (0,0,0)
are defined as the negated OR of the bits of their three labels, and the sets of
positions sharing a label intersect in that label's bits.

Commands:
  read FILE N            mapping checks and the per-label chain count
  variant FILE N --add fwd,rev,pos --out OUT.cnf
                         the file plus implied clauses (for timing tests):
                         fwd: placement -> its label bits (binary, RUP);
                         rev: the label bits -> placement (RUP);
                         pos: one at-least-one clause per position over its
                         placements that are not unit-false (implied, not RUP)
  rupcheck ORIG VARIANT [--sample 3000]
                         unit-propagation check of the added clauses
  selftest               toys: placement order, the OR and RUP readers

Measured with it (solver README, 'Reading the bp, oisc and smt2 cells'): the links
(fwd) gave 15 v 14 solved of 18 runs on 9 cells; the position clauses made the
unsatisfiable runs time out.  The 2026 suite is not read (plan section 8).
"""
from __future__ import annotations

import argparse
import lzma
import os
import random
import sys
from collections import Counter, defaultdict
from itertools import combinations_with_replacement, permutations, product
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from rl_structure import parse_prefix  # noqa: E402

MAX_CLAUSES = 80_000_000


def placements(N):
    """(labels, cubes, positions, rows); rows[i] describes variable i + 1 as
    (name, cube index, rotation tuple, (x, y, h))."""
    L = 2 * N - 1
    base = 2 * N - 1
    th = [min(x + 1, base - x) for x in range(base)]
    mh = [[min(th[x], th[y]) for x in range(base)] for y in range(base)]
    cubes = list(combinations_with_replacement(range(L), 3))
    pos = [(x, y, h) for x, y in product(range(base), range(base)) for h in range(mh[y][x])]
    assert len(pos) == len(cubes)
    rows = []
    for ci, cube in enumerate(cubes):
        for ri, rot in enumerate(sorted(set(permutations(cube)))):
            for (x, y, h) in pos:
                rows.append((f"c{ci}_r{ri}_x{x}_y{y}_h{h}", ci, rot, (x, y, h)))
    rows.sort(key=lambda r: r[0])
    return L, cubes, pos, rows


def or_definitions(clauses):
    """{(v, positive): frozenset(literals)}: v <-> OR(literals) when positive,
    -v <-> OR(literals) otherwise (binaries (-l, t) for every l and one clause
    (literals, -t))."""
    bins = defaultdict(set)
    for c in clauses:
        if len(c) == 2:
            a, b = c
            bins[b].add(-a)
            bins[a].add(-b)
    defs = {}
    for c in clauses:
        if len(c) < 3:
            continue
        for t in c:
            t_out = -t
            rest = [l for l in c if l != t]
            have = bins.get(t_out)
            if have and all(l in have for l in rest):
                defs.setdefault((abs(t_out), t_out > 0), frozenset(rest))
    return defs


class Reading:
    def __init__(self, clauses, N):
        self.N = N
        self.L, self.cubes, self.pos, self.rows = placements(N)
        self.P = len(self.rows)
        P = self.P
        self.units = {c[0] for c in clauses if len(c) == 1}
        by_cube = defaultdict(set)
        for i, (_n, ci, _rot, _p) in enumerate(self.rows):
            by_cube[ci].add(i + 1)
        sets = {frozenset(c) for c in clauses if len(c) >= 2 and all(0 < l <= P for l in c)}
        self.cube_clauses_matched = sum(1 for s in by_cube.values() if frozenset(s) in sets)
        defs = or_definitions(clauses)
        or_of = {k: v for k, v in defs.items() if k[0] > P and all(abs(l) > P for l in v)}
        occ = defaultdict(list)
        for i, c in enumerate(clauses):
            for l in c:
                occ[abs(l)].append(i)
        zero = {}
        for i, (_n, ci, _rot, p) in enumerate(self.rows):
            if ci != 0 or -(i + 1) in self.units:
                continue
            best = None
            for k in occ.get(i + 1, ()):
                for l in clauses[k]:
                    u = abs(l)
                    if u <= P:
                        continue
                    for f in (True, False):
                        s = or_of.get((u, f))
                        if s is not None and (best is None or len(s) > len(best)):
                            best = s
            if best is not None:
                zero[p] = frozenset(abs(l) for l in best)
        self.zero_found = len(zero)
        groups = defaultdict(list)
        for (x, y, h), s in zero.items():
            groups[("X", x, h)].append(s)
            groups[("Y", y, h)].append(s)
            groups[("Z", x, y)].append(s)
        bits = {}
        for key, ss in groups.items():
            if len(ss) >= 2:
                inter = frozenset.intersection(*ss)
                if len(inter) == self.L - 1:
                    bits[key] = inter
        for _ in range(3):
            for (x, y, h), s in zero.items():
                keys = [("X", x, h), ("Y", y, h), ("Z", x, y)]
                known = [k for k in keys if k in bits]
                if len(known) == 2:
                    k = [k for k in keys if k not in bits][0]
                    rest = s - bits[known[0]] - bits[known[1]]
                    if len(rest) == self.L - 1:
                        bits[k] = rest
        imp = set()
        for c in clauses:
            if len(c) == 2:
                imp.add((c[0], c[1]))
                imp.add((c[1], c[0]))
        self.order = {}
        self.no_chain = 0
        for key, bs in bits.items():
            up = defaultdict(list)
            for a in bs:
                for b in bs:
                    if a != b and (a, -b) in imp:   # clause (a, -b): b -> a, b the next bit up
                        up[a].append(b)
            has_down = {b for lst in up.values() for b in lst}
            bottoms = [b for b in bs if b not in has_down]
            seq = []
            if len(bottoms) == 1 and all(len(v) == 1 for v in up.values()):
                cur = bottoms[0]
                seq = [cur]
                while cur in up:
                    cur = up[cur][0]
                    seq.append(cur)
            if len(seq) == len(bs):
                self.order[key] = seq       # seq[k] <-> label > k
            else:
                self.no_chain += 1

    def summary(self):
        return (f"N={self.N} placements={self.P} cube_clauses_matched={self.cube_clauses_matched}/"
                f"{len(self.cubes)} units={len(self.units)} value0_tests={self.zero_found} "
                f"labels_ordered={len(self.order)} labels_without_chain={self.no_chain}")

    def implied(self, adds):
        extra = []
        counts = Counter()
        by_pos = defaultdict(list)
        for i, (_n, ci, rot, (x, y, h)) in enumerate(self.rows):
            pv = i + 1
            if -pv in self.units:
                continue
            by_pos[(x, y, h)].append(pv)
            lits = []
            complete = True
            for key, r in zip((("X", x, h), ("Y", y, h), ("Z", x, y)), rot):
                seq = self.order.get(key)
                if seq is None:
                    complete = False
                    continue
                if r > 0:
                    lits.append(seq[r - 1])
                if r < self.L - 1:
                    lits.append(-seq[r])
            if "fwd" in adds:
                for l in lits:
                    extra.append((-pv, l))
                    counts["fwd"] += 1
            if "rev" in adds and complete:
                extra.append(tuple([pv] + [-l for l in lits]))
                counts["rev"] += 1
        if "pos" in adds:
            for pvs in by_pos.values():
                extra.append(tuple(pvs))
                counts["pos"] += 1
        return extra, counts


class UnitProp:
    """Two-watched-literal unit propagation for RUP checks."""

    def __init__(self, n, clauses):
        self.cl = [list(c) for c in clauses]
        self.val = [0] * (n + 1)
        self.trail = []
        self.watch = defaultdict(list)
        units = []
        for ci, c in enumerate(self.cl):
            if len(c) == 1:
                units.append(c[0])
            else:
                self.watch[c[0]].append(ci)
                self.watch[c[1]].append(ci)
        self.root_conflict = not all(self._set(l) for l in units) or not self._prop(0)
        self.root = len(self.trail)

    def _v(self, l):
        v = self.val[abs(l)]
        return v if l > 0 else -v

    def _set(self, l):
        v = self._v(l)
        if v:
            return v == 1
        self.val[abs(l)] = 1 if l > 0 else -1
        self.trail.append(l)
        return True

    def _prop(self, i):
        while i < len(self.trail):
            fl = -self.trail[i]
            i += 1
            ws = self.watch.get(fl)
            if not ws:
                continue
            keep, j = [], 0
            while j < len(ws):
                ci = ws[j]
                j += 1
                c = self.cl[ci]
                if c[0] == fl:
                    c[0], c[1] = c[1], c[0]
                if self._v(c[0]) == 1:
                    keep.append(ci)
                    continue
                for k in range(2, len(c)):
                    if self._v(c[k]) != -1:
                        c[1], c[k] = c[k], c[1]
                        self.watch[c[1]].append(ci)
                        break
                else:
                    keep.append(ci)
                    if self._v(c[0]) == -1:
                        keep.extend(ws[j:])
                        self.watch[fl] = keep
                        return False
                    self._set(c[0])
            self.watch[fl] = keep
        return True

    def rup(self, clause):
        if self.root_conflict:
            return True             # the formula refutes itself: every clause is RUP
        head = len(self.trail)
        ok = all(self._set(-l) for l in clause) and self._prop(head)
        while len(self.trail) > self.root:
            self.val[abs(self.trail.pop())] = 0
        return not ok


def _open(path):
    with open(path, "rb") as fh:
        xz = fh.read(6) == b"\xfd7zXZ\x00"
    return (lzma.open if xz else open)(path, "rt", encoding="ascii", errors="replace")


def scan(path):
    """(header variables, clauses counted by their terminating 0, empty clauses):
    the file as written, before parse_prefix drops empty clauses and tautologies."""
    n_vars, count, empty, cur = 0, 0, 0, 0
    with _open(path) as f:
        for line in f:
            if not line.strip() or line[0] in "c%":
                continue
            if line[0] == "p":
                n_vars = int(line.split()[2])
                continue
            for tok in line.split():
                if tok == "0":
                    count += 1
                    empty += cur == 0
                    cur = 0
                else:
                    cur += 1
    return n_vars, count, empty


def load(path):
    """The parsed clauses, refusing what this reading cannot vouch for: a file
    longer than MAX_CLAUSES, or one with an empty clause (parse_prefix drops it)."""
    n_vars, count, empty = scan(path)
    if empty:
        sys.exit(f"{path}: {empty} empty clause(s); the file is unsatisfiable as written, not read")
    n, _m, clauses, truncated = parse_prefix(Path(path), MAX_CLAUSES)
    if truncated:
        sys.exit(f"{path}: more than {MAX_CLAUSES} clauses, not read")
    return n_vars, count, clauses


def cmd_read(a):
    _n, _count, clauses = load(a.file)
    r = Reading(clauses, a.N)
    print(f"{Path(a.file).name}: {r.summary()}")


def cmd_variant(a):
    n_vars, count, clauses = load(a.file)
    r = Reading(clauses, a.N)
    extra, counts = r.implied(set(a.add.split(",")))
    # copy the original clause lines as written (nothing dropped), then the additions
    with open(a.out, "w") as f, _open(a.file) as fin:
        f.write(f"p cnf {n_vars} {count + len(extra)}\n")
        for line in fin:
            if not line.strip() or line[0] in "cp%":
                continue
            f.write(line if line.endswith("\n") else line + "\n")
        for c in extra:
            f.write(" ".join(map(str, c)) + " 0\n")
    print(f"{r.summary()}\nadded {dict(counts)}; wrote {a.out}")


def cmd_rupcheck(a):
    n, _c, orig = load(a.orig)
    n2, _c2, var = load(a.variant)
    if var[: len(orig)] != orig:
        sys.exit("the variant does not start with the original's clauses (not written by `variant`?)")
    added = var[len(orig):]
    up = UnitProp(max(n, n2), orig)
    rng = random.Random(a.seed)
    sample = added if a.sample <= 0 or a.sample >= len(added) else rng.sample(added, a.sample)
    tally = defaultdict(lambda: [0, 0])
    for c in sample:
        kind = "binary" if len(c) == 2 else ("position" if all(l > 0 for l in c) else "reverse")
        tally[kind][0] += 1
        tally[kind][1] += up.rup(c)
    print(f"original {len(orig)} clauses, added {len(added)}, root conflict {up.root_conflict}")
    for k, (tot, ok) in sorted(tally.items()):
        print(f"  {k}: {ok} of {tot} RUP")


def selftest():
    """Toys: the placement count and order, the OR reader, the chain reader and the
    RUP check on a hand-built order-encoded label with a one-way value test."""
    L, cubes, pos, rows = placements(2)
    assert (L, len(cubes), len(pos), len(rows)) == (3, 10, 10, 270), (L, len(cubes), len(pos), len(rows))
    assert rows[0][0] == "c0_r0_x0_y0_h0" and [r[0] for r in rows] == sorted(r[0] for r in rows)
    assert sum(1 for r in rows if r[1] == 0) == 10        # cube (0,0,0): one rotation, 10 positions
    # one label X in 0..2 with bits 1 (X > 0) and 2 (X > 1) and the chain 2 -> 1;
    # 3 <-> OR(1, 2) is the negation of the value-0 test; P = 4 is "X = 1" written as
    # "exactly one of the two bits" (a parity gate), so only the chain lets
    # propagation link P to the bits
    chain = [(1, -2)]
    gates = [(-1, 3), (-2, 3), (1, 2, -3),
             (-4, 1, 2), (-4, -1, -2), (4, -1, 2), (4, 1, -2)]
    defs = or_definitions(chain + gates)
    assert defs.get((3, True)) == frozenset({1, 2}), defs
    up = UnitProp(4, chain + gates)
    assert up.rup((-4, 1)) and up.rup((-4, -2)) and up.rup((4, -1, 2))   # the links
    assert not up.rup((-4, -1)) and not up.rup((-4,))                     # not implied
    loose = UnitProp(4, gates)                                            # no chain
    assert not loose.rup((-4, 1)) and not loose.rup((-4, -2))
    assert UnitProp(2, [(1,), (-1,)]).rup((2,))                           # root conflict
    import tempfile
    with tempfile.NamedTemporaryFile("w", suffix=".cnf", delete=False) as tf:
        tf.write("c x\np cnf 3 3\n1 -2 0\n0\n2 3\n 0\n")
    try:
        assert scan(tf.name) == (3, 3, 1), scan(tf.name)
    finally:
        os.unlink(tf.name)
    print("selftest ok")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("selftest")
    r = sub.add_parser("read")
    r.add_argument("file")
    r.add_argument("N", type=int)
    v = sub.add_parser("variant")
    v.add_argument("file")
    v.add_argument("N", type=int)
    v.add_argument("--add", required=True)
    v.add_argument("--out", required=True)
    c = sub.add_parser("rupcheck")
    c.add_argument("orig")
    c.add_argument("variant")
    c.add_argument("--sample", type=int, default=3000)
    c.add_argument("--seed", type=int, default=1)
    a = ap.parse_args(argv)
    if a.cmd == "selftest":
        selftest()
        return
    {"read": cmd_read, "variant": cmd_variant, "rupcheck": cmd_rupcheck}[a.cmd](a)


if __name__ == "__main__":
    main()
