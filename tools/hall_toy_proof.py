#!/usr/bin/env python3
"""Toy check of the alldifferent propagator's certificate (bead
SAT-playground-iin.2, G.1.2 of the general-structure epic).

A Hall set: k rows (at-least-one clauses) whose literals that are not false
all sit in fewer than k, or exactly k, columns (at-most-one cliques). A
propagator acts on it during search, so each step needs a clause the proof
can justify from the input alone. This tool writes those clauses in VeriPB 3,
checks them, and checks that a wrong one is rejected.

The shape the engine would emit, in terms of constraint ids:

  1. A column C = {c1, ..., cm} once, when the grid is set up: its at-most-one
     as one linear constraint, ~c1 + ... + ~cm >= m - 1, from its pair clauses
     by the counting pass's sum and division:
         AMO_2 is the pair clause;
         AMO_(j+1) = (pair(1,j+1) + ... + pair(j,j+1) + (j-1) AMO_j) / j.
     A pair the encoding does not write directly (a ladder, a commander) is a
     `rup` clause first: propagating one literal excludes the other.

  2. A violation, rows S on columns N(S) with |N(S)| = |S| - 1. Let E be the
     literals of S outside N(S) (all false now), and M the literals of N(S)
     outside S. One `pol` line:
         sum of S's rows + sum of N(S)'s AMO constraints + the axiom m >= 0
         for every m in M
     Each literal of S inside N(S) cancels against its negation, the constants
     balance, and what is left is exactly the clause over E:
         E1 + ... + Er >= 1.

  3. A removal, rows S on columns N(S) with |N(S)| = |S|, and a literal v of
     another row in one of those columns. The same line without the axiom for
     v gives the clause (~v v E1 v ... v Er), the reason for ~v.

A row given as a resolvent, (g1 v g2 v g3) with (~g1 v x1 v x2) and so on, is
one `pol` sum of those clauses first. A column value only one row has counts
as a column of one, whose constraint is the axiom ~c >= 0. If a literal m of E
is still open because a true literal t holds its column and propagation did
not reach m (an exclusion that shows from one side only), the pair clause
(~t v ~m) joins the sum: m cancels and the reason holds ~t in its place, so
every literal of the reason is false now.

A count needs none of this: on a Sinz counter or a totalizer an at-most-k
propagation is implied by unit propagation, so its reason
(~t1 v ... v ~tk v ~x) is a plain `rup` line (or a DRAT line). The toy checks
that too; the bead's text had assumed one `pol` line.

Where the clauses go. Exactly as for the parity engine (src/gaussproof.rs): the
structure pass's writer stays open through the search, every engine clause is
a line in `<proof>.pbp` and in `<proof>.derived.cnf`, and at exit the first
stage proves F' = F + derived from F, while drat-trim checks the search's DRAT
against F'. So an engine clause has to be in the stream by the end of the run,
not before the search's DRAT uses it. The `gate` toy goes through
tools/proof_gate.py with exactly those three files.

Usage:
  python3 tools/hall_toy_proof.py selftest [--veripb PATH] [--out DIR]
  python3 tools/hall_toy_proof.py stress --count 200 [--seed 1] [--veripb PATH] [--out DIR]
"""
from __future__ import annotations

import argparse
import itertools
import random
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from alldiff_census import hall_filter, hopcroft_karp  # noqa: E402

DEFAULT_VERIPB = ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb"


def lit_text(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


def clause_text(c):
    return " ".join(f"1 {lit_text(l)}" for l in c) + " >= 1"


class Stream:
    """A VeriPB 3 proof over a CNF's clause numbering."""

    def __init__(self, n_clauses):
        self.lines = ["pseudo-Boolean proof version 3.0"]
        self.n = n_clauses
        self.kept = []

    def pol(self, expr):
        self.lines.append(f"pol {expr} ;")
        self.n += 1
        return self.n

    def rup(self, clause):
        self.lines.append(f"rup {clause_text(clause)} ;")
        self.n += 1
        return self.n

    def write(self, path, for_gate=False):
        """Standalone a proof states nothing about an output; for the gate it
        says F' is equisatisfiable with F, as the structure pass's does."""
        out = list(self.lines)
        if for_gate:
            if self.kept:
                out.append("core id " + " ".join(map(str, self.kept)) + " ;")
            out += ["output EQUISATISFIABLE FILE ;", "conclusion NONE ;"]
        else:
            out += ["output NONE ;", "conclusion NONE ;"]
        out.append("end pseudo-Boolean proof ;")
        Path(path).write_text("\n".join(out) + "\n")


class Grid:
    """n rows over d columns as a CNF: literal of row r in column c, a row's
    at-least-one clause, and each column's at-most-one, pairwise or as a
    ladder through fresh variables. `flip` writes some variables negated, so
    a grid literal may be a negative one."""

    def __init__(self, domains, d, encoding="pairwise", flip=None):
        self.n_vars = 0
        self.clauses = []
        self.lit = {}
        self.encoding = encoding
        flip = flip or (lambda v: False)
        for r, dom in enumerate(domains):
            for c in dom:
                self.n_vars += 1
                self.lit[(r, c)] = -self.n_vars if flip(self.n_vars) else self.n_vars
        self.domains = domains
        self.d = d
        self.row_id = {}
        for r, dom in enumerate(domains):
            self.clauses.append([self.lit[(r, c)] for c in dom])
            self.row_id[r] = len(self.clauses)
        self.pair_id = {}
        self.columns = {}
        for c in range(d):
            lits = [self.lit[(r, c)] for r in range(len(domains)) if (r, c) in self.lit]
            self.columns[c] = lits
            if encoding == "pairwise":
                for a, b in itertools.combinations(lits, 2):
                    self.clauses.append([-a, -b])
                    self.pair_id[frozenset((a, b))] = len(self.clauses)
            elif len(lits) >= 2:
                s = []
                for _ in lits[:-1]:
                    self.n_vars += 1
                    s.append(self.n_vars)
                for i, x in enumerate(lits):
                    if i < len(s):
                        self.clauses.append([-x, s[i]])
                    if i > 0:
                        self.clauses.append([-x, -s[i - 1]])
                    if 0 < i < len(s):
                        self.clauses.append([-s[i - 1], s[i]])

    def write(self, path, extra=()):
        cl = self.clauses + [list(c) for c in extra]
        with open(path, "w") as f:
            f.write(f"p cnf {self.n_vars} {len(cl)}\n")
            for c in cl:
                f.write(" ".join(map(str, c)) + " 0\n")
        return cl


def build_amo(g: Grid, st: Stream, c, cache):
    """The column's at-most-one as one linear constraint. None for a column of
    one literal (its constraint is the axiom)."""
    if c in cache:
        return cache[c]
    lits = g.columns[c]
    if len(lits) < 2:
        cache[c] = None
        return None

    def pair(a, b):
        key = frozenset((a, b))
        got = g.pair_id.get(key)
        if got is None:
            got = st.rup([-a, -b])     # propagation excludes one from the other
            g.pair_id[key] = got
        return got

    current = pair(lits[0], lits[1])
    for m in range(2, len(lits)):
        toks = [str(pair(lits[0], lits[m]))]
        for i in range(1, m):
            toks += [str(pair(lits[i], lits[m])), "+"]
        toks += [str(current), str(m - 1), "*", "+", str(m), "d"]
        current = st.pol(" ".join(toks))
    cache[c] = current
    return current


def explain(g: Grid, st: Stream, rows, cols, keep_negated=None, cache=None):
    """One `pol` line and the clause it gives: the sum of `rows`, the sum of
    `cols`' at-most-ones and the axioms of the columns' other literals, except
    `keep_negated`, the literal that stays as ~v. Returns (clause, pol id,
    clause id, added literals)."""
    cache = {} if cache is None else cache
    rowset = set(rows)
    colset = set(cols)
    amo = {c: build_amo(g, st, c, cache) for c in cols}
    toks = []

    def add(tok):
        toks.append(tok)
        if len(toks) > 1:
            toks.append("+")

    for r in rows:
        add(str(g.row_id[r]))
    for c in cols:
        if amo[c] is None:
            add(lit_text(-g.columns[c][0]))    # the axiom ~c >= 0 of a column of one
        else:
            add(str(amo[c]))
    others = []
    for c in cols:
        for r in range(len(g.domains)):
            if (r, c) in g.lit and r not in rowset:
                others.append(g.lit[(r, c)])
    for m in others:
        if m != keep_negated:
            add(lit_text(m))
    outside = [g.lit[(r, c)] for r in rows for c in g.domains[r] if c not in colset]
    clause = list(outside) + ([-keep_negated] if keep_negated is not None else [])
    pid = st.pol(" ".join(toks))
    cid = st.rup(clause)
    return clause, pid, cid, len(others)


def hall_structure(adj, d):
    """From a row's values not yet false: a violation as (rows, columns) with
    one row too many, else the removals as (literal's row, column, rows of
    its Hall set, columns of it)."""
    n = len(adj)
    size, pair_u, pair_v = hopcroft_karp(adj, n, d)
    if size < n:
        # the rows an alternating search reaches from an unmatched row
        start = next(u for u in range(n) if pair_u[u] == -1)
        rows = {start}
        cols = set()
        stack = [start]
        while stack:
            u = stack.pop()
            for v in adj[u]:
                if v not in cols:
                    cols.add(v)
                    w = pair_v[v]
                    if w != -1 and w not in rows:
                        rows.add(w)
                        stack.append(w)
        return ("violation", sorted(rows), sorted(cols))
    removals = []
    # for a free edge (u, v): the rows reached from v's matched row through
    # their other values and those values' matched rows; if that closes without
    # an unmatched value and without u, it is a Hall set holding v
    for u in range(n):
        for v in adj[u]:
            if pair_u[u] == v:
                continue
            w = pair_v[v]
            if w == -1:
                continue    # v is free, so the edge is in a matching
            rows = {w}
            cols = set()
            stack = [w]
            free = False
            while stack and not free:
                x = stack.pop()
                for y in adj[x]:
                    if y in cols:
                        continue
                    cols.add(y)
                    z = pair_v[y]
                    if z == -1:
                        free = True
                        break
                    if z not in rows:
                        rows.add(z)
                        stack.append(z)
            if not free and u not in rows:
                removals.append((u, v, sorted(rows), sorted(cols)))
    return ("removals", removals)


def veripb(binary, cnf, pbp, timeout=600):
    p = subprocess.run([str(binary), str(cnf), str(pbp)], capture_output=True, text=True, timeout=timeout)
    text = p.stdout + p.stderr
    return p.returncode == 0 and "s VERIFIED" in text, text.strip().splitlines()[-1:] if text.strip() else [""]


def tamper_drop_literal(pbp, bad):
    """Removes one literal from the first explanation clause's `rup` line that
    has two or more, which must break it."""
    lines = Path(pbp).read_text().splitlines()
    for i in range(len(lines) - 1, -1, -1):
        if lines[i].startswith("rup ") and lines[i].count(" x") + lines[i].count(" ~x") >= 2:
            parts = lines[i].split()
            del parts[1:3]
            lines[i] = " ".join(parts)
            Path(bad).write_text("\n".join(lines) + "\n")
            return True
    return False


# ------------------------------------------------------------------- the toys

def toy_violation(out, binary, encoding):
    """Six rows on eight columns, three columns gone from every row: the rows
    fit in five columns, one too few."""
    domains = [list(range(8)) for _ in range(6)]
    g = Grid(domains, 8, encoding)
    cnf = Path(out) / f"violation_{encoding}.cnf"
    g.write(cnf)
    st = Stream(len(g.clauses))
    clause, pid, cid, _ = explain(g, st, list(range(6)), list(range(5)))
    pbp = Path(out) / f"violation_{encoding}.pbp"
    st.lines.append(f"rup {clause_text(clause)} ;")
    st.n += 1
    st.write(pbp)
    ok, tail = veripb(binary, cnf, pbp)
    bad = Path(out) / f"violation_{encoding}.bad.pbp"
    tamper_drop_literal(pbp, bad)
    ok2, _ = veripb(binary, cnf, bad)
    return ok and not ok2, f"{len(clause)} literals, {len(st.lines)} proof lines, checked {ok}, tampered accepted {ok2}, {tail}"


def toy_removal(out, binary, encoding):
    """Three rows restricted to three columns (the other columns false), so a
    fourth row's literal in one of those columns must be false."""
    domains = [list(range(6)) for _ in range(5)]
    g = Grid(domains, 6, encoding)
    cnf = Path(out) / f"removal_{encoding}.cnf"
    g.write(cnf)
    st = Stream(len(g.clauses))
    v = g.lit[(4, 1)]
    clause, pid, cid, _ = explain(g, st, [0, 1, 2], [0, 1, 2], keep_negated=v)
    pbp = Path(out) / f"removal_{encoding}.pbp"
    st.write(pbp)
    ok, tail = veripb(binary, cnf, pbp)
    bad = Path(out) / f"removal_{encoding}.bad.pbp"
    tamper_drop_literal(pbp, bad)
    ok2, _ = veripb(binary, cnf, bad)
    return ok and not ok2 and -v in clause, f"{len(clause)} literals with ~v, checked {ok}, tampered accepted {ok2}, {tail}"


def toy_wrong_set(out, binary):
    """A claim with as many columns as rows and no removal target: the sum
    gives nothing, so the clause over the outside literals must be rejected."""
    domains = [list(range(6)) for _ in range(4)]
    g = Grid(domains, 6)
    cnf = Path(out) / "wrong.cnf"
    g.write(cnf)
    st = Stream(len(g.clauses))
    clause, pid, cid, _ = explain(g, st, [0, 1, 2], [0, 1, 2])
    pbp = Path(out) / "wrong.pbp"
    st.write(pbp)
    ok, _ = veripb(binary, cnf, pbp)
    return not ok, f"accepted {ok}"


def toy_resolved_row(out, binary):
    """A row split through guards is one `pol` sum first; the same line then
    works on it."""
    lines = []
    leaves = list(range(1, 13))
    guards = [13, 14, 15]
    clauses = [list(guards)]
    for g, i in zip(guards, range(0, 12, 4)):
        clauses.append([-g] + leaves[i:i + 4])
    cnf = Path(out) / "resolved.cnf"
    with open(cnf, "w") as f:
        f.write(f"p cnf 15 {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")
    st = Stream(len(clauses))
    pid = st.pol("1 2 + 3 + 4 +")
    st.rup(leaves)
    pbp = Path(out) / "resolved.pbp"
    st.write(pbp)
    ok, tail = veripb(binary, cnf, pbp)
    return ok, f"checked {ok} {tail}"


def toy_count(out, binary):
    """On a Sinz counter (at most 2 of 6) and on a totalizer, every reason of
    the propagated literal is a plain rup line."""
    n, k = 6, 2
    x = list(range(1, n + 1))
    nv = n
    s = {}
    for i in range(n):
        for j in range(k):
            nv += 1
            s[(i, j)] = nv
    cl = []
    for i in range(n):
        cl.append([-x[i], s[(i, 0)]])
        if i > 0:
            for j in range(k):
                cl.append([-s[(i - 1, j)], s[(i, j)]])
            for j in range(1, k):
                cl.append([-x[i], -s[(i - 1, j - 1)], s[(i, j)]])
            cl.append([-x[i], -s[(i - 1, k - 1)]])
    cnf = Path(out) / "sinz.cnf"
    with open(cnf, "w") as f:
        f.write(f"p cnf {nv} {len(cl)}\n")
        for c in cl:
            f.write(" ".join(map(str, c)) + " 0\n")
    st = Stream(len(cl))
    count = 0
    for chosen in itertools.combinations(x, k + 1):
        st.rup([-l for l in chosen])
        count += 1
    pbp = Path(out) / "sinz.pbp"
    st.write(pbp)
    ok, tail = veripb(binary, cnf, pbp)
    return ok, f"{count} reasons, checked {ok} {tail}"


def toy_gate(out, binary):
    """The whole path, through the gate with the chosen checker: an engine
    clause in the stream and in the derived file, the search's DRAT using it.
    Units make the rows' other columns false, so the formula needs the Hall
    clause to be refuted: unit propagation alone leaves it open."""
    import proof_gate
    proof_gate.find_veripb = lambda: str(binary)
    domains = [list(range(8)) for _ in range(6)]
    g = Grid(domains, 8)
    units = [[-g.lit[(r, c)]] for r in range(6) for c in range(5, 8)]
    cnf = Path(out) / "gate.cnf"
    clauses = g.write(cnf, units)
    st = Stream(len(clauses))
    clause, pid, cid, _ = explain(g, st, list(range(6)), list(range(5)))
    st.kept.append(cid)
    proof = Path(out) / "gate.drat"
    proof.write_text("0\n")
    st.write(str(proof) + ".pbp", for_gate=True)
    with open(str(proof) + ".derived.cnf", "w") as f:
        f.write(f"p cnf {g.n_vars} 1\n" + " ".join(map(str, clause)) + " 0\n")
    good = proof_gate.check_unsat(cnf, proof, timeout=600)
    # without the Hall clause in F', the same DRAT line must fail
    with open(str(proof) + ".derived.cnf", "w") as f:
        f.write(f"p cnf {g.n_vars} 0\n")
    st2 = Stream(len(clauses))
    st2.write(str(proof) + ".pbp", for_gate=True)
    bad = proof_gate.check_unsat(cnf, proof, timeout=600)
    return good == "ok" and bad.startswith("FAIL"), f"with the clause: {good}; without: {bad[:80]}"


def stress(count, seed, binary, out, verbose=False):
    """Random grids and random partial assignments. Every Hall violation and
    removal found by matching is explained, and the clause must check; the
    removals must agree in number with the census's filter."""
    rng = random.Random(seed)
    checked = viol = rem = 0
    failures = []
    for t in range(count):
        n = rng.randint(3, 7)
        d = rng.randint(max(3, n - 1), n + 3)
        domains = [sorted(rng.sample(range(d), rng.randint(max(2, d - 2), d))) for _ in range(n)]
        encoding = rng.choice(["pairwise", "ladder"])
        flips = {v: rng.random() < 0.3 for v in range(1, n * d + 1)}
        g = Grid(domains, d, encoding, flip=lambda v: flips.get(v, False))
        false = set()
        for r in range(n):
            for c in domains[r]:
                if rng.random() < 0.45 and sum(1 for c2 in domains[r] if (r, c2) not in false) > 1:
                    false.add((r, c))
        adj = [[c for c in domains[r] if (r, c) not in false] for r in range(n)]
        res = hall_structure(adj, d)
        cnf = Path(out) / f"s{t}.cnf"
        g.write(cnf)
        st = Stream(len(g.clauses))
        cache = {}
        if res[0] == "violation":
            _, rows, cols = res
            clause, _, _, _ = explain(g, st, rows, cols, cache=cache)
            viol += 1
            short, _ = hall_filter(adj, d)
            if short == 0:
                failures.append((t, "matching disagrees on the violation"))
            currently_false = {g.lit[(r, c)] for r, c in false}
            if not all(l in currently_false for l in clause):
                failures.append((t, "a violation's clause holds a literal that is not false"))
        else:
            _, removals = res
            short, removed = hall_filter(adj, d)
            if removed != len(removals):
                failures.append((t, f"filter {removed} against {len(removals)} explained"))
            currently_false = {g.lit[(r, c)] for r, c in false}
            for u, v, rows, cols in removals:
                lit = g.lit[(u, v)]
                clause, _, _, _ = explain(g, st, rows, cols, keep_negated=lit, cache=cache)
                rem += 1
                if not all(l in currently_false or l == -lit for l in clause):
                    failures.append((t, "a removal's reason holds a literal that is not false"))
        if len(st.lines) > 1:
            pbp = Path(out) / f"s{t}.pbp"
            st.write(pbp)
            ok, tail = veripb(binary, cnf, pbp)
            checked += 1
            if not ok:
                failures.append((t, f"VeriPB: {tail}"))
    return checked, viol, rem, failures


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["selftest", "stress"])
    ap.add_argument("--veripb", default=str(DEFAULT_VERIPB))
    ap.add_argument("--out", default="/tmp/hall_toy_proof")
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    if args.mode == "stress":
        checked, viol, rem, failures = stress(args.count, args.seed, args.veripb, out)
        print(f"{args.count} grids: {checked} proofs checked, {viol} violations and {rem} removals explained, {len(failures)} failures")
        for f in failures[:10]:
            print("  ", f)
        return 0 if not failures else 1
    ok = True

    def report(name, res):
        nonlocal ok
        print(("ok   " if res[0] else "FAIL ") + name + f"  ({res[1]})")
        ok &= bool(res[0])

    report("a violation, pairwise columns", toy_violation(out, args.veripb, "pairwise"))
    report("a violation, ladder columns", toy_violation(out, args.veripb, "ladder"))
    report("a removal, pairwise columns", toy_removal(out, args.veripb, "pairwise"))
    report("a removal, ladder columns", toy_removal(out, args.veripb, "ladder"))
    report("as many columns as rows and no target: rejected", toy_wrong_set(out, args.veripb))
    report("a row split through guards", toy_resolved_row(out, args.veripb))
    report("a Sinz counter's reasons are rup lines", toy_count(out, args.veripb))
    report("through the gate, with F' and the search's DRAT", toy_gate(out, args.veripb))
    checked, viol, rem, failures = stress(args.count if args.count != 200 else 60, args.seed, args.veripb, out)
    report("random grids and assignments", (not failures, f"{checked} proofs, {viol} violations, {rem} removals, failures {failures[:2]}"))
    print("selftest:", "all passed" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
