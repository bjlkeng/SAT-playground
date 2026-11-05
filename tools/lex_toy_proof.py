#!/usr/bin/env python3
"""lex_toy_proof.py — the lex-leader chains of a point symmetry, with their
VeriPB proof, on a toy (bead SAT-playground-1v2.3.4). The reference for the
structure pass's point step (src/structjohnson.rs): the same chains, the same
proof steps, checked by VeriPB before any solver code was written.

The formula: two colours on the lines between n points, no red K_s and no blue
K_t (Ramsey R(s,t) on n points; line {i,j} is one variable, true = red). Any
renumbering of the points maps the formula onto itself. For each swap of
neighbouring points p and p+1 a chain of clauses says: read the lines in the
global order (line {i,j}, i < j, by i then j), and the colouring must not come
after its copy with p and p+1 swapped. The smallest copy of every colouring
passes every chain, so the chains keep a solution whenever there is one.

The proof (VeriPB 3, after Satsuma 1.4's dense order, MIT, Markus Anders):
one lex order over all the lines, as a single linear constraint with powers of
two (def_order with its transitivity proof, then load_order); per chain a
`dom` step derives "colouring <= its swapped copy" as a linear constraint, with
the swap as witness; the chain's clauses follow by rup, the helper variables
(a_i: "equal so far") are defined by red steps on themselves, and a pol step
carries the linear constraint past each compared pair. The helper clauses the
chain does not keep are deleted, the order is unloaded, the kept clauses move
to the core, and the output section names F' = F plus the chains.

Usage:
  lex_toy_proof.py check S T N [--flip SEED] [--before]   write and check one toy
  lex_toy_proof.py selftest                               the toys the tests use
--flip negates a random half of the variables (the chains then work on
literals); --before derives one clause before the order is loaded (the
structure pass's earlier steps).
"""

import itertools
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERIPB = ROOT / "tools" / "checkers" / "VeriPB" / "target" / "release" / "veripb"


def ramsey(s, t, n, flip=None):
    """(clauses, line -> literal): line {i,j} (0-based points, i < j)."""
    lines = list(itertools.combinations(range(n), 2))
    lit = {e: k + 1 for k, e in enumerate(lines)}
    if flip is not None:
        rng = random.Random(flip)
        for e in lines:
            if rng.random() < 0.5:
                lit[e] = -lit[e]
    cls = [[-lit[e] for e in itertools.combinations(q, 2)] for q in itertools.combinations(range(n), s)]
    cls += [[lit[e] for e in itertools.combinations(q, 2)] for q in itertools.combinations(range(n), t)]
    return cls, lit


def chains(n, lit):
    """Per swap of points p, p+1: the steps (x, sigma(x)) as literals, in the
    global order of x, up to the last x that is not the image of an earlier
    one (an image right after its partner is a step of its own: without it the
    next rup step fails, since its term in the linear constraint is still
    open), plus the swap as a literal map and the support in global order."""
    lines = list(itertools.combinations(range(n), 2))
    pos = {e: k for k, e in enumerate(lines)}
    out = []
    for p in range(n - 1):
        q = p + 1
        sw = lambda e: tuple(sorted(q if x == p else p if x == q else x for x in e))
        support = [e for e in lines if (p in e or q in e) and e != (p, q)]
        support.sort(key=lambda e: pos[e])
        last_new = max(k for k, e in enumerate(support) if sw(e) not in set(support[:k]))
        steps = [(lit[e], lit[sw(e)]) for e in support[: last_new + 1]]
        sigma = {}
        for e in support:
            a, b = lit[e], lit[sw(e)]
            sigma[abs(a)] = b if a > 0 else -b      # variable |a| maps to literal ...
        out.append((steps, sigma, support))
    return out, pos


def L(l):
    return f"x{l}" if l > 0 else f"~x{-l}"


def clause(c):
    return " ".join(f"1 {L(l)}" for l in c) + " >= 1"


def write_toy(s, t, n, d, flip=None, before=False):
    cls, lit = ramsey(s, t, n, flip)
    nv = n * (n - 1) // 2
    m = len(cls)
    ch, pos = chains(n, lit)
    N = nv
    order_vars = [abs(lit[e]) for e in sorted(lit, key=lambda e: pos[e])]
    sign = {abs(lit[e]): (1 if lit[e] > 0 else -1) for e in lit}
    K = {abs(lit[e]): 1 << (N - 1 - pos[e]) for e in lit}   # by variable
    out = []          # proof lines
    nid = m           # last constraint id
    kept = []         # (clause, id) part of F'
    fresh = nv
    def emit(line, adds=1):
        nonlocal nid
        out.append(line)
        nid += adds
        return nid
    out.append("pseudo-Boolean proof version 3.0")
    if before:
        # a clause the formula implies, derived before the order: the
        # structure pass's earlier steps leave such constraints behind
        c0 = cls[0] + [cls[1][0]]
        kept.append((c0, emit(f"rup {clause(c0)} ;")))
    # the order: u <= v iff sum 2^(N-i) (v_i - u_i) >= 0
    us = " ".join(f"u{i}" for i in range(1, N + 1))
    vs = " ".join(f"v{i}" for i in range(1, N + 1))
    ws = " ".join(f"w{i}" for i in range(1, N + 1))
    terms = " ".join(f"-{1 << (N - i)} u{i} {1 << (N - i)} v{i}" for i in range(1, N + 1))
    out += [f"def_order lex{N}", "vars", f"left {us} ;", f"right {vs} ;", "aux ;", "end ;",
            "def", f"{terms} >= 0 ;", "end ;",
            "transitivity", "vars", f"fresh_right {ws} ;", "end ;", "proof",
            "proofgoal #1", "pol 1 2 + 3 + ;", "qed : -1 ;", "qed ;", "end ;", "end ;"]
    out.append(f"load_order lex{N} " + " ".join(f"x{v}" for v in order_vars) + " ;")
    for steps, sigma, support in ch:
        # the linear constraint: sum over the support of K(y) * (sigma(y) - y),
        # y the literal of the line (the order is on variables: a negated
        # line's term is K * (~x - ...) of its literal)
        terms = []
        for e in support:
            # the order is on variables: in the swapped copy variable v has
            # the value of the literal sigma(v), so its term is
            # K(v) * (sigma(v) - x_v)
            v = abs(lit[e])
            terms.append(f"{K[v]} {L(sigma[v])} -{K[v]} x{v}")
        witness = " ".join(f"x{v} -> {L(sigma[v])}" for v in sorted(sigma))
        # the subproof's three constraints (the negated constraint, the
        # negated-order goal's premise, the pol) take ids before the
        # derived constraint gets the next one
        p_id = emit(f"dom {' '.join(terms)} >= 0 : {witness} : subproof\nproofgoal #2\npol -1 -2 + ;\nqed #2 : -1 ;\nqed ;", adds=4)
        # the chain, step by step over the variables (lex on variables:
        # x_v <= value of sigma(x_v))
        prev = None    # a_{i-1}
        drop = []
        for i, (xl, yl) in enumerate(steps):
            v = abs(xl)
            x, y = v, sigma[v]          # variable x, literal y = sigma(x)
            body = ([-prev] if prev else []) + [-x, y]
            kept.append((body, emit(f"rup {clause(body)} ;")))
            if i + 1 == len(steps):
                break
            fresh += 1
            a = fresh
            c1 = [y] + ([-prev] if prev else []) + [a]
            c2 = [-x] + ([-prev] if prev else []) + [a]
            kept.append((c1, emit(f"red {clause(c1)} : x{a} -> 1 ;")))
            kept.append((c2, emit(f"red {clause(c2)} : x{a} -> 1 ;")))
            if prev:
                drop.append(emit(f"red {clause([-a, prev])} : x{a} -> 0 ;"))
            r = emit(f"red {clause([-a, -y, x])} : x{a} -> 0 ;")
            drop.append(r)
            p_new = emit(f"pol {p_id} {r} {K[v]} * + ;")
            out.append(f"del id {p_id} ;")
            p_id = p_new
            prev = a
        out.append(f"del id {p_id} " + " ".join(map(str, drop)) + " ;")
    out.append("load_order ;")
    out.append("core id " + " ".join(str(i) for _, i in kept) + " ;")
    out.append("output EQUISATISFIABLE FILE ;")
    out.append("conclusion NONE ;")
    out.append("end pseudo-Boolean proof ;")
    nvars = fresh
    f = d / "f.opb"
    fp = d / "fprime.opb"
    with open(f, "w") as fh:
        fh.write(f"* #variable= {nvars} #constraint= {m}\n")
        for c in cls:
            fh.write(" ".join(f"+1 {L(l)}" for l in c) + " >= 1 ;\n")
    with open(fp, "w") as fh:
        fh.write(f"* #variable= {nvars} #constraint= {m + len(kept)}\n")
        for c in cls + [c for c, _ in kept]:
            fh.write(" ".join(f"+1 {L(l)}" for l in c) + " >= 1 ;\n")
    (d / "proof.pbp").write_text("\n".join(out) + "\n")
    with open(d / "fprime.cnf", "w") as fh:
        fh.write(f"p cnf {nvars} {m + len(kept)}\n")
        for c in cls + [c for c, _ in kept]:
            fh.write(" ".join(map(str, c)) + " 0\n")
    return len(ch), len(kept), nvars - nv


def veripb(d, output="fprime.opb"):
    t = time.time()
    p = subprocess.run([str(VERIPB), str(d / "f.opb"), str(d / "proof.pbp"), str(d / output)], capture_output=True, text=True)
    ok = p.returncode == 0 and "s VERIFIED NO CONCLUSION" in p.stdout
    tail = [l for l in (p.stdout + p.stderr).splitlines() if l.strip()][-3:]
    return ok, time.time() - t, tail


def check(s, t, n, flip=None, before=False, keep=None):
    d = Path(tempfile.mkdtemp(prefix="lex-toy-"))
    try:
        nch, nkept, nhelp = write_toy(s, t, n, d, flip, before)
        ok, secs, tail = veripb(d)
        print(f"R({s},{t}) n={n} flip={flip} before={before}: {nch} chains, {nkept} clauses, {nhelp} helpers; VeriPB {'ok' if ok else 'FAIL'} in {secs:.1f} s")
        if not ok:
            print("   " + "\n   ".join(tail))
        if keep:
            shutil.copytree(d, keep, dirs_exist_ok=True)
        return ok, d
    finally:
        if not keep:
            shutil.rmtree(d, ignore_errors=True)


def main():
    if len(sys.argv) >= 5 and sys.argv[1] == "check":
        s, t, n = map(int, sys.argv[2:5])
        flip = int(sys.argv[sys.argv.index("--flip") + 1]) if "--flip" in sys.argv else None
        keep = sys.argv[sys.argv.index("--keep") + 1] if "--keep" in sys.argv else None
        ok, _ = check(s, t, n, flip, "--before" in sys.argv, keep)
        sys.exit(0 if ok else 1)
    if len(sys.argv) == 2 and sys.argv[1] == "selftest":
        results = [check(3, 4, 9)[0], check(3, 4, 9, flip=7)[0], check(3, 4, 9, before=True)[0], check(3, 5, 14)[0]]
        sys.exit(0 if all(results) else 1)
    print(__doc__)
    sys.exit(2)


if __name__ == "__main__":
    main()
