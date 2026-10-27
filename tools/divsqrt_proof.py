#!/usr/bin/env python3
"""Divider and square-root miter certificate prototype (bead
SAT-playground-skz.17): the cut-point chain of tools/divsqrt_census.py with
one DRAT proof of the miter.

Per pair (x, y) (one signal per side computing the same row value) the
solver refutes the circuit without the miter's unit, the pairs proven
before it and the units x, -y.  Every clause its DRAT proof adds, widened
by -x and y, follows by unit propagation from the input and the lines
before it (under x and -y the widened lines are the run's own), and the
widened empty clause is (-x | y).  The same with -x, y gives (x | -y).  The
proof never trusts the census: a pair the solver does not refute is
dropped, and every line is checked by drat-trim through
tools/proof_gate.py.  Last, the miter with every proven pair added: its
DRAT lines as they are.  The solver runs the stock path with variable
addition off (--factor=false: its clauses on fresh variables are RAT, not
unit propagation steps).  A sub-call that crashes, answers UNKNOWN with
no budget set, or answers with the wrong exit code stops the run; a
timeout, a memory-out or the solver's own budget drops the pair.

Subcommands:
  toy --n N --k K --out FILE [--shortcut]
        a toy miter: quotient bit k of an n by n divider (b = 0 gives all
        ones), a full-width restoring array with a borrow chain against a
        narrow array of adders (T + ~b + 1) over the low n-k divisor bits.
        With --shortcut the narrow side's rows ignore the high divisor bits
        and only its output checks them (b >= 2^(n-k) forces the bit to 0),
        so its inner rows are right only for small divisors, as in the real
        cells.  The inputs a[k..n-1] and b[0..n-1] are numbered in the text
        order of their names; every clause starts with its gate's literal.
  prove CNF --solver BIN --out DIR [--jobs N] [--limit S] [--window W]
        [--subcall-args=OPTS] [--mem-mb MB] [--progress] [--tamper]
        the chain with its proof (pairs checked in windows as in the
        census's chain; --subcall-args=--plain runs every sub-call without
        pre- and inprocessing, which keeps the proof to about the learned
        clauses; every sub-call runs under --mem-mb, default 16,000), then
        tools/proof_gate.py on it.  --tamper also checks three broken
        proofs: the last pair's derivation left out (its conclusions kept),
        the whole chain left out (the pairs kept), and a middle pair
        concluded as x = -y without its derivation.  The run fails when one
        of them is accepted, as on cells too small for these controls.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import divsqrt_census as census  # noqa: E402
from proof_gate import check_unsat  # noqa: E402

SUBCALL = census.STOCK + ["--factor=false", "--no-binary"]


# ---------------------------------------------------------------------------
# toy miters
# ---------------------------------------------------------------------------


class Builder:
    """Gates with constant folding; each gate's clauses start with its own
    literal.  Signals are ints (literals) or the Python constants True/False."""

    def __init__(self):
        self.n = 0
        self.clauses = []

    def var(self):
        self.n += 1
        return self.n

    @staticmethod
    def neg(x):
        return (not x) if isinstance(x, bool) else -x

    def AND(self, a, b):
        if a is False or b is False:
            return False
        if a is True:
            return b
        if b is True:
            return a
        if a == b:
            return a
        if a == -b:
            return False
        g = self.var()
        self.clauses += [(-g, a), (-g, b), (g, -a, -b)]
        return g

    def OR(self, a, b):
        return self.neg(self.AND(self.neg(a), self.neg(b)))

    def XOR(self, a, b):
        if isinstance(a, bool):
            return self.neg(b) if a else b
        if isinstance(b, bool):
            return self.neg(a) if b else a
        if a == b:
            return False
        if a == -b:
            return True
        g = self.var()
        self.clauses += [(-g, a, b), (-g, -a, -b), (g, -a, b), (g, a, -b)]
        return g

    def MAJ(self, a, b, c):
        return self.OR(self.AND(a, b), self.AND(c, self.OR(a, b)))

    def MUX(self, s, t, e):
        """s ? t : e"""
        if isinstance(s, bool):
            return t if s else e
        if t == e:
            return t
        if isinstance(t, bool) or isinstance(e, bool):
            return self.OR(self.AND(s, t), self.AND(self.neg(s), e))
        g = self.var()
        self.clauses += [(-g, -s, t), (-g, s, e), (g, -s, -t), (g, s, -e)]
        return g


def toy_divider(n, k, shortcut):
    """(header vars, clauses) of the toy miter on quotient bit k."""
    bld = Builder()
    names = sorted([f"a[{i}]" for i in range(k, n)] + [f"b[{i}]" for i in range(n)])
    pi = {nm: bld.var() for nm in names}
    a = {i: pi[f"a[{i}]"] for i in range(k, n)}
    b = [pi[f"b[{i}]"] for i in range(n)]
    # side 1: full-width rows, T - b with a borrow chain
    r = [False] * n
    q1 = None
    for j in range(n - 1, k - 1, -1):
        t = [a[j]] + r                    # n + 1 bits
        borrow = False
        d = []
        for i in range(n + 1):
            bi = b[i] if i < n else False
            d.append(bld.XOR(bld.XOR(t[i], bi), borrow))
            borrow = bld.MAJ(bld.neg(t[i]), bi, borrow)
        q1 = bld.neg(borrow)
        r = [bld.MUX(q1, d[i], t[i]) for i in range(n)]
    # side 2: rows of width n-k+1 over the low divisor bits, T + ~b + 1
    w = n - k
    big = False
    for i in range(w, n):
        big = bld.OR(big, b[i])
    r = [False] * w
    q2 = None
    for j in range(n - 1, k - 1, -1):
        t = [a[j]] + r                    # w + 1 bits
        carry = True
        s = []
        for i in range(w + 1):
            nb = bld.neg(b[i]) if i < w else True
            s.append(bld.XOR(bld.XOR(t[i], nb), carry))
            carry = bld.MAJ(t[i], nb, carry)
        q2 = carry if shortcut else bld.AND(carry, bld.neg(big))
        r = [bld.MUX(q2, s[i], t[i]) for i in range(w)]
    if shortcut:
        q2 = bld.AND(q2, bld.neg(big))
    root = bld.XOR(q1, q2)
    if isinstance(root, bool):
        raise ValueError("the two sides folded to constants")
    return bld.n, bld.clauses + [(root,)]


def cmd_toy(args):
    n_vars, clauses = toy_divider(args.n, args.k, args.shortcut)
    census.write_cnf(args.out, n_vars, clauses)
    print(f"{args.out}: {n_vars} variables, {len(clauses)} clauses")
    return 0


# ---------------------------------------------------------------------------
# the chain with its proof
# ---------------------------------------------------------------------------


def read_drat(path):
    """The added clauses of an ASCII DRAT file (deletions dropped)."""
    out = []
    with open(path) as f:
        for line in f:
            tok = line.split()
            if not tok or tok[0] == "d":
                continue
            lits = [int(t) for t in tok]
            if lits[-1] != 0:
                raise ValueError(f"{path}: line without a final zero")
            out.append(lits[:-1])
    return out


def widen(lines, extra):
    """Each line with the extra literals; tautologies left out."""
    out = []
    for c in lines:
        lits = list(dict.fromkeys(c + extra))
        if any(-lit in lits for lit in lits):
            continue
        out.append(lits)
    return out


def subcall(solver, cnf_path, proof_path, limit, extra=(), mem_mb=census.MEM_MB):
    """(status, seconds, conflicts, true variables of the model or None, work)"""
    st, el, out = census.run_solver(solver, SUBCALL + list(extra) + [str(cnf_path), str(proof_path)], limit, mem_mb)
    confl, work = census.workclock(out)
    return st, el, confl, census.model_of(out) if st == "SATISFIABLE" else None, work


def cmd_prove(args):
    path = Path(args.file)
    census.refuse_holdout([path])
    cell = census.read_cell(path)
    if cell["kind"] is None:
        sys.exit(f"{path.name}: no divider or square-root function found")
    net, pairs = cell["net"], cell["pairs"]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    n_vars = max([net.header_vars] + [abs(lit) for c in net.clauses for lit in c])
    root_units = {i for i, c in enumerate(net.clauses) if len(set(c)) == 1 and abs(c[0]) == cell["root"]}
    circuit = [c for i, c in enumerate(net.clauses) if i not in root_units]
    tmp = Path(tempfile.mkdtemp(prefix="prove-", dir=out))

    def eq(i):
        _, x, y = pairs[i]
        return [(-x, y), (x, -y)]

    def run_pair(i, assumed):
        """Both directions of pair i: (status, seconds, conflicts, model, lines, work)."""
        _, x, y = pairs[i]
        base = circuit + [c for j in assumed for c in eq(j)]
        lines, secs, confls, works = [], 0.0, [], []
        for units, extra in (([(x,), (-y,)], [-x, y]), ([(-x,), (y,)], [x, -y])):
            f, pf = tmp / f"p{i:05d}.cnf", tmp / f"p{i:05d}.drat"
            census.write_cnf(f, n_vars, base + units)
            st, el, cf, model, wk = subcall(args.solver, f, pf, args.limit, args.subcall_args.split(), args.mem_mb)
            secs += el
            confls.append(cf)
            works.append(wk)
            drat = read_drat(pf) if st == "UNSATISFIABLE" else []
            f.unlink()
            if pf.exists():
                pf.unlink()
            confl, work = census.total(confls), census.total(works)
            confl, work = (-1 if confl is None else confl), (-1 if work is None else work)
            if st != "UNSATISFIABLE":
                return st, secs, confl, model, None, work
            lines += widen(drat, extra) + [extra]
        lf = tmp / f"pair{i:05d}.lines"     # a pair checked again overwrites its earlier lines
        with open(lf, "w") as fh:
            for c in lines:
                fh.write(" ".join(map(str, c)) + " 0\n")
        return "UNSATISFIABLE", secs, confl, None, (lf, len(lines)), work

    t0 = time.time()
    proven, dropped, by_model, res, spent = census.run_chain(net, pairs, run_pair, args.jobs, args.window,
                                                             args.progress)
    # the miter with every proven pair
    f, pf = tmp / "miter.cnf", tmp / "miter.drat"
    census.write_cnf(f, n_vars, list(net.clauses) + [c for i in proven for c in eq(i)])
    fin = subcall(args.solver, f, pf, args.limit, args.subcall_args.split(), args.mem_mb)
    final = read_drat(pf) if fin[0] == "UNSATISFIABLE" else None
    f.unlink()
    if pf.exists():
        pf.unlink()
    if final is None:
        for f in tmp.glob("pair*.lines"):
            f.unlink()
        os.rmdir(tmp)
        sys.exit(f"{path.name}: the miter with the proven pairs is {fin[0]}")
    if not final or final[-1]:
        final.append([])

    def write_proof(p, parts):
        """parts: (path, count) of a pair's lines, or a list of clauses."""
        n = 0
        with open(p, "w") as fh:
            for part in parts:
                if isinstance(part, tuple):
                    with open(part[0]) as src:
                        for line in src:
                            fh.write(line)
                    n += part[1]
                    continue
                for c in part:
                    fh.write(" ".join(map(str, c)) + " 0\n")
                    n += 1
        return n

    chain = [res[i][4] for i in proven]
    proof = out / "proof.drat"
    n_lines = write_proof(proof, chain + [final])
    tc = time.time()
    verdict = check_unsat(path, proof, timeout=args.check_timeout)
    check_s = time.time() - tc
    summary = dict(cell=path.name, pairs=len(pairs), proven=len(proven), dropped=len(dropped),
                   dropped_by_model=len(by_model),
                   proven_seconds=round(sum(res[i][1] for i in proven), 2),
                   proven_conflicts=census.total(res[i][2] for i in proven), all_checks=spent,
                   miter_conflicts=fin[2], miter_work=fin[4], subcall_args=args.subcall_args, proof_lines=n_lines,
                   proof_mb=round(proof.stat().st_size / 1e6, 1), gate=verdict,
                   gate_seconds=round(check_s, 1), wall_seconds=round(time.time() - t0, 1))
    if args.tamper and proven:
        tampers = {}
        # 1. the last pair's derivation left out, its two conclusions kept
        last = len(chain) - 1
        tampers["the last pair's derivation left out"] = chain[:last] + [eq(proven[last])]
        # 2. the chain left out, the pairs kept
        tampers["the chain left out, the pairs kept"] = [[c for i in proven for c in eq(i)]]
        # 3. a middle pair's conclusion flipped to x = -y, its derivation left out
        mid = len(chain) // 2
        _, x, y = pairs[proven[mid]]
        tampers["a middle pair concluded as x = -y without its derivation"] = (
            chain[:mid] + [[[-x, -y], [x, y]]] + chain[mid + 1:])
        summary["tamper"] = {}
        for name, parts in tampers.items():
            bad = out / "tampered.drat"
            write_proof(bad, parts + [final])
            summary["tamper"][name] = check_unsat(path, bad, timeout=args.check_timeout)
            bad.unlink()
    for f in tmp.glob("pair*.lines"):
        f.unlink()
    os.rmdir(tmp)
    with open(out / "prove.json", "w") as fh:
        json.dump(dict(summary=summary, proven=[pairs[i] for i in proven], dropped=[pairs[i] for i in dropped],
                       dropped_by_model=[pairs[i] for i in by_model],
                       results={pairs[i][0]: res[i][:3] for i in res}), fh, indent=1)
    print(json.dumps(summary))
    if verdict != "ok":
        return 1
    if args.tamper:
        accepted = [k for k, v in summary.get("tamper", {}).items() if not v.startswith("FAIL:")]
        if accepted or not proven:
            print(f"{path.name}: broken proofs not rejected ({accepted or 'no pair proven'}): "
                  f"this cell cannot exercise the controls", file=sys.stderr)
            return 1
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("toy")
    t.add_argument("--n", type=int, required=True)
    t.add_argument("--k", type=int, required=True)
    t.add_argument("--shortcut", action="store_true")
    t.add_argument("--out", required=True)
    p = sub.add_parser("prove")
    p.add_argument("file")
    p.add_argument("--solver", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--jobs", type=int, default=8)
    p.add_argument("--limit", type=float, default=300)
    p.add_argument("--window", type=int, default=32)
    p.add_argument("--progress", action="store_true")
    p.add_argument("--subcall-args", default="", help="extra solver options for every sub-call, e.g. --plain")
    p.add_argument("--mem-mb", type=int, default=census.MEM_MB)
    p.add_argument("--check-timeout", type=float, default=3600)
    p.add_argument("--tamper", action="store_true")
    args = ap.parse_args(argv)
    return cmd_toy(args) if args.cmd == "toy" else cmd_prove(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
