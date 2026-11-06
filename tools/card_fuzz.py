#!/usr/bin/env python3
"""Fuzz the card engine, src/card.rs (epic SAT-playground-iin).

Random small formulas with counts and alldifferent grids in many encodings, with
the engine's guardrails set at random, and with what can go wrong around them:
clauses dropped, noise clauses, repeated literals, tautologies, shuffled clauses
and renamed or flipped variables.

For each formula the stock path and the engine, with and without a proof file,
must agree. Every model is checked against every clause. Every UNSAT answer from
a run where the engine added a lemma goes through the two-stage gate
(tools/proof_gate.py). The formulas are small enough for the stock search to
decide them, so a wrong lemma shows up as a disagreement as well as a failed proof.

The engine's lemmas follow from the input whatever the search did, so the run with
a proof and the run without one must also make the same decisions: same lemmas and
same conflicts.

Usage: python3 tools/card_fuzz.py --count 2000 [--seed 1] [--jobs 8] [--out DIR]
"""
from __future__ import annotations

import argparse
import itertools
import os
import random
import re
import subprocess
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOLVER = Path(os.environ.get("CARD_FUZZ_SOLVER", str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver")))
GATE = ROOT / "tools" / "proof_gate.py"
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0", "--structdiv=0", "--structdd=0", "--gauss=0", "--card=0", "--structpoints=0"]


class Enc:
    def __init__(self, nv, rng):
        self.nv = nv
        self.c = []
        self.rng = rng

    def new(self):
        self.nv += 1
        return self.nv

    def add(self, *lits):
        self.c.append(list(lits))


def amo(enc, lits, kind):
    if len(lits) < 2:
        return
    if kind == "pairs":
        for a, b in itertools.combinations(lits, 2):
            enc.add(-a, -b)
    elif kind == "ladder":
        s = [enc.new() for _ in lits[:-1]]
        for i, x in enumerate(lits[:-1]):
            enc.add(-x, s[i])
        for i in range(len(s) - 1):
            enc.add(-s[i], s[i + 1])
        for i in range(1, len(lits)):
            enc.add(-lits[i], -s[i - 1])
    elif kind == "commander":
        if len(lits) <= 3:
            amo(enc, lits, "pairs")
            return
        m = len(lits) // 2
        for part in (lits[:m], lits[m:]):
            amo(enc, part, "pairs")
        c1, c2 = enc.new(), enc.new()
        for x in lits[:m]:
            enc.add(-x, c1)
        for x in lits[m:]:
            enc.add(-x, c2)
        enc.add(-c1, -c2)


def sinz_atmost(enc, lits, k):
    n = len(lits)
    if k >= n or n < 2:
        return
    if k <= 0:
        for x in lits:
            enc.add(-x)
        return
    s = [[enc.new() for _ in range(k)] for _ in range(n - 1)]
    enc.add(-lits[0], s[0][0])
    for j in range(1, k):
        enc.add(-s[0][j])
    for i in range(1, n - 1):
        enc.add(-lits[i], s[i][0])
        enc.add(-s[i - 1][0], s[i][0])
        for j in range(1, k):
            enc.add(-lits[i], -s[i - 1][j - 1], s[i][j])
            enc.add(-s[i - 1][j], s[i][j])
        enc.add(-lits[i], -s[i - 1][k - 1])
    enc.add(-lits[n - 1], -s[n - 2][k - 1])


def totalizer_outputs(enc, lits, cap=None):
    if len(lits) == 1:
        return [lits[0]]
    m = len(lits) // 2
    a, b = totalizer_outputs(enc, lits[:m], cap), totalizer_outputs(enc, lits[m:], cap)
    width = len(a) + len(b)
    if cap is not None:
        width = min(width, cap)
    out = [enc.new() for _ in range(width)]
    for i in range(len(a) + 1):
        for j in range(len(b) + 1):
            t = i + j
            if 1 <= t <= width:
                cl = [out[t - 1]]
                if i:
                    cl.append(-a[i - 1])
                if j:
                    cl.append(-b[j - 1])
                enc.c.append(cl)
    return out


def totalizer_atmost(enc, lits, k, bounded):
    if k >= len(lits):
        return
    out = totalizer_outputs(enc, lits, k + 1 if bounded else None)
    enc.add(-out[k])


def one_way_atleast(enc, lits, k):
    n = len(lits)
    if k <= 0:
        return
    if k > n:
        enc.add(lits[0])
        enc.add(-lits[0])
        return
    s = {}
    for i in range(1, n + 1):
        for j in range(1, min(i, k) + 1):
            s[i, j] = enc.new()
    for (i, j), v in s.items():
        x = lits[i - 1]
        lo = s.get((i - 1, j))
        hi = s.get((i - 1, j - 1)) if j > 1 else None
        if lo is None:
            enc.add(-v, x)
            if j > 1:
                enc.add(-v, hi)
        else:
            enc.add(-v, lo, x)
            if j > 1:
                enc.add(-v, lo, hi)
    enc.add(s[n, k])


def subset_atmost(enc, lits, k):
    if k >= len(lits) or len(lits) > 8:
        return
    for sub in itertools.combinations(lits, k + 1):
        enc.c.append([-x for x in sub])


def count(enc, lits, atleast, kind):
    """At least `atleast` of lits, in one of the forms."""
    n = len(lits)
    neg = [-x for x in lits]
    if kind == "sinz":
        sinz_atmost(enc, neg, n - atleast)
    elif kind == "totalizer":
        totalizer_atmost(enc, neg, n - atleast, False)
    elif kind == "bounded":
        totalizer_atmost(enc, neg, n - atleast, True)
    elif kind == "oneway":
        one_way_atleast(enc, lits, atleast)
    elif kind == "subset":
        subset_atmost(enc, neg, n - atleast)


def atmost(enc, lits, k, kind):
    n = len(lits)
    if kind == "sinz":
        sinz_atmost(enc, lits, k)
    elif kind == "totalizer":
        totalizer_atmost(enc, lits, k, False)
    elif kind == "bounded":
        totalizer_atmost(enc, lits, k, True)
    elif kind == "oneway":
        one_way_atleast(enc, [-x for x in lits], n - k)
    elif kind == "subset":
        subset_atmost(enc, lits, k)


def build(seed):
    rng = random.Random(seed)
    family = rng.choice(["board", "grid", "timetable", "timetable", "php", "latin", "colour", "block", "block", "block"])
    forms = ["sinz", "totalizer", "bounded", "oneway", "subset"]
    amos = ["pairs", "pairs", "ladder", "commander"]
    info = family
    if family == "board":
        n = rng.randint(2, 5)
        enc = Enc(n * n, rng)
        rows = [[i * n + j + 1 for j in range(n)] for i in range(n)]
        a = rng.choice(amos)
        for r in rows:
            amo(enc, r, a)
        if rng.random() < 0.7:
            for j in range(n):
                amo(enc, [rows[i][j] for i in range(n)], a)
        xs = [x for r in rows for x in r]
        rng.shuffle(xs)
        k = n + rng.choice([-2, -1, 0, 0, 1])
        form = rng.choice(forms)
        count(enc, xs, max(k, 1), form)
        for x in rng.sample(xs, rng.randint(0, len(xs) // 2)):
            enc.add(-x)
        info += f" n={n} k={k} {form} {a}"
    elif family == "block":
        # a board with a count over it, small enough to decide and big enough that
        # the search has to work: the engine's lemmas arrive at deep states, where
        # backtracking and the trail are at their most awkward
        n = rng.randint(5, 8)
        enc = Enc(n * n, rng)
        rows = [[i * n + j + 1 for j in range(n)] for i in range(n)]
        a = rng.choice(amos)
        for r in rows:
            amo(enc, r, a)
        for j in range(n):
            amo(enc, [rows[i][j] for i in range(n)], a)
        xs = [x for r in rows for x in r]
        rng.shuffle(xs)
        slack = rng.choice([0, 0, 1, 2])
        form = rng.choice(["sinz", "totalizer", "bounded", "oneway"])
        kind = rng.choice(["blocked", "planted"])
        if kind == "blocked":
            many, cols = 3 + slack * 2, 2 + slack
            for i in range(min(many, n)):
                for j in range(cols, n):
                    enc.add(-rows[i][j])
        else:
            perm = list(range(n))
            rng.shuffle(perm)
            for i in range(n):
                for j in range(n):
                    if perm[i] != j and rng.random() < 0.5:
                        enc.add(-rows[i][j])
        count(enc, xs, n - slack, form)
        info += f" n={n} slack={slack} {form} {a} {kind}"
    elif family == "grid":
        n, d = rng.randint(2, 6), rng.randint(2, 6)
        enc = Enc(n * d, rng)
        rows = [[i * d + j + 1 for j in range(d)] for i in range(n)]
        for r in rows:
            keep = [x for x in r if rng.random() < 0.8] or r[:1]
            if rng.random() < 0.15:
                enc.c.append(keep + [enc.new()])  # a guard or a private value
            else:
                enc.c.append(keep)
        a = rng.choice(amos)
        for j in range(d):
            amo(enc, [rows[i][j] for i in range(n)], a)
        info += f" n={n} d={d} {a}"
    elif family == "timetable":
        # events need one slot each, each slot holds at most K events
        n, d = rng.randint(3, 7), rng.randint(2, 4)
        enc = Enc(n * d, rng)
        rows = [[i * d + j + 1 for j in range(d)] for i in range(n)]
        for r in rows:
            enc.c.append(list(r))
        K = rng.randint(1, 3)
        form = rng.choice(forms)
        for j in range(d):
            atmost(enc, [rows[i][j] for i in range(n)], K, form)
        info += f" n={n} d={d} K={K} {form}"
    elif family == "php":
        h = rng.randint(2, 5)
        p = h + rng.choice([0, 1, 1])
        enc = Enc(p * h, rng)
        rows = [[i * h + j + 1 for j in range(h)] for i in range(p)]
        for r in rows:
            enc.c.append(list(r))
        a = rng.choice(amos)
        for j in range(h):
            amo(enc, [rows[i][j] for i in range(p)], a)
        info += f" p={p} h={h} {a}"
    elif family == "latin":
        n = rng.randint(2, 4)
        enc = Enc(n * n * n, rng)
        v = lambda i, j, k: i * n * n + j * n + k + 1
        a = rng.choice(amos)
        for i, j in itertools.product(range(n), repeat=2):
            enc.c.append([v(i, j, k) for k in range(n)])
            amo(enc, [v(i, j, k) for k in range(n)], a)
        for i, k in itertools.product(range(n), repeat=2):
            amo(enc, [v(i, j, k) for j in range(n)], a)
        for j, k in itertools.product(range(n), repeat=2):
            amo(enc, [v(i, j, k) for i in range(n)], a)
        for _ in range(rng.randint(0, n)):
            enc.add(v(rng.randrange(n), rng.randrange(n), rng.randrange(n)))
        info += f" n={n} {a}"
    else:
        # graph colouring, a clique inside so it can be impossible
        nodes, colours = rng.randint(3, 6), rng.randint(2, 4)
        enc = Enc(nodes * colours, rng)
        v = lambda i, c: i * colours + c + 1
        for i in range(nodes):
            enc.c.append([v(i, c) for c in range(colours)])
        for i, j in itertools.combinations(range(nodes), 2):
            if rng.random() < 0.7:
                for c in range(colours):
                    enc.add(-v(i, c), -v(j, c))
        info += f" nodes={nodes} colours={colours}"
    # what can go wrong around them
    if rng.random() < 0.12 and enc.c:
        enc.c.pop(rng.randrange(len(enc.c)))
        info += " dropped"
    base = enc.nv
    if base >= 2:
        if rng.random() < 0.2:
            for _ in range(rng.randint(1, 4)):
                a, b = rng.sample(range(1, base + 1), 2)
                enc.add(rng.choice([a, -a]), rng.choice([b, -b]))
        if rng.random() < 0.1:
            a = rng.randint(1, base)
            b = rng.randint(1, base)
            enc.c.append([a, a, b])
        if rng.random() < 0.1:
            a = rng.randint(1, base)
            enc.c.append([a, -a, rng.randint(1, base)])
        if rng.random() < 0.1 and enc.c:
            enc.c.append(list(rng.choice(enc.c)))
    if rng.random() < 0.5:
        rng.shuffle(enc.c)
    if rng.random() < 0.5:
        perm = list(range(1, enc.nv + 1))
        rng.shuffle(perm)
        flips = rng.random() < 0.3
        ren = {v: perm[v - 1] * (rng.choice([1, -1]) if flips else 1) for v in range(1, enc.nv + 1)}
        enc.c = [[(ren[abs(l)] if l > 0 else -ren[abs(l)]) for l in c] for c in enc.c]
    opts = [
        "--card=1",
        f"--lucky={rng.choice([0, 0, 1])}",
        f"--cardint={rng.choice([1, 1, 2, 10, 100])}",
        f"--cardrestart={rng.choice([0, 0, 1])}",
        f"--cardmaxlen={rng.choice([2, 4, 16, 64, 100000, 100000])}",
        f"--cardmaxper={rng.choice([1, 8, 64])}",
        f"--cardeffort={rng.choice([0, 0, 10, 50])}",
        f"--cardforced={rng.choice([0, 1, 1])}",
        f"--cardyield={rng.choice([0, 4, 8])}",
        f"--cardminlen={rng.choice([0, 0, 3])}",
        f"--cardminrows={rng.choice([0, 0, 0, 4, 6])}",
        f"--cardkinds={rng.choice([7, 7, 7, 1, 2, 4])}",
        f"--cardmulti={rng.choice([0, 1])}",
        f"--cardgrow={rng.choice([0, 0, 6, 12])}",
        f"--cardroot={rng.choice([0, 1, 1])}",
        f"--cardlook={rng.choice([0, 0, 10, 1000])}",
    ]
    return enc.nv, enc.c, opts, info


def write_cnf(path, nv, clauses):
    with open(path, "w") as f:
        f.write(f"p cnf {nv} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


def solve(cnf, opts, proof=None):
    args = [str(SOLVER)] + STOCK + opts + [str(cnf)] + ([str(proof)] if proof else [])
    p = subprocess.run(args, capture_output=True, text=True, timeout=300)
    status = next((l[2:].strip() for l in p.stdout.splitlines() if l.startswith("s ")), "NONE")
    expected = {"SATISFIABLE": 10, "UNSATISFIABLE": 20}.get(status)
    if expected is None or p.returncode != expected:
        status = f"FAILED(rc={p.returncode},s={status})"
    model = set()
    for l in p.stdout.splitlines():
        if l.startswith("v "):
            model.update(int(t) for t in l[2:].split() if t != "0")
    m = re.search(r"^c conflicts:\s+(\d+)", p.stdout, re.M)
    conflicts = int(m.group(1)) if m else -1
    c = re.search(r"^c card .*root_clauses=(\d+) violations=(\d+) forced=(\d+)", p.stdout, re.M)
    lemmas = (int(c.group(1)) + int(c.group(2)) + int(c.group(3))) if c else 0
    return status, model, conflicts, lemmas


def check_model(clauses, model):
    if any(-l in model for l in model):
        return False
    return all(any(l in model for l in c) for c in clauses)


def one(args):
    seed, out = args
    d = Path(out) / f"s{seed}"
    d.mkdir(parents=True, exist_ok=True)
    cnf = d / "f.cnf"
    nv, clauses, opts, info = build(seed)
    write_cnf(cnf, nv, clauses)
    verdict = "ok"
    stock, m0, _, _ = solve(cnf, [])
    plain, m1, c1, l1 = solve(cnf, opts)
    proved, m2, c2, l2 = solve(cnf, opts, d / "p.drat")
    if not (stock == plain == proved) or stock.startswith("FAILED"):
        verdict = f"DISAGREE stock={stock} plain={plain} proved={proved}"
    elif c1 != c2 or l1 != l2:
        verdict = f"DECISIONS-DIFFER conflicts {c1} {c2} lemmas {l1} {l2}"
    elif stock == "SATISFIABLE":
        for name, mm in (("stock", m0), ("plain", m1), ("proved", m2)):
            if not check_model(clauses, mm):
                verdict = f"BAD-MODEL {name}"
    else:
        g = subprocess.run([sys.executable, str(GATE), str(cnf), str(d / "p.drat")], capture_output=True, text=True, timeout=600)
        if g.stdout.strip() != "ok":
            verdict = f"GATE {g.stdout.strip()[:200]}"
    if verdict == "ok" and stock != "UNSATISFIABLE" and l2 > 0:
        # no refutation to check, but the lemmas were still written: VeriPB reads them
        sys.path.insert(0, str(GATE.parent))
        import proof_gate
        v = proof_gate.check_stream(cnf, d / "p.drat", timeout=600)
        if v != "ok":
            verdict = f"STREAM {v[:200]}"
    if verdict == "ok":
        for p in d.iterdir():
            p.unlink()
        d.rmdir()
    return seed, verdict, f"{info} opts={' '.join(opts[1:])} stock={stock} lemmas={l1}", stock, l1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--out", default=str(ROOT / "log" / "card-fuzz"))
    args = ap.parse_args()
    seeds = [(args.seed * 100000 + i, args.out) for i in range(args.count)]
    counts = {}
    fired = sat = unsat = 0
    lemmas = 0
    with ProcessPoolExecutor(max_workers=args.jobs) as ex:
        for seed, verdict, info, stock, l in ex.map(one, seeds):
            key = verdict.split()[0]
            counts[key] = counts.get(key, 0) + 1
            fired += l > 0
            lemmas += l
            sat += stock == "SATISFIABLE"
            unsat += stock == "UNSATISFIABLE"
            if verdict != "ok":
                print(f"seed {seed}: {verdict}: {info}", flush=True)
    print(f"{args.count} formulas: {counts}; {sat} SAT, {unsat} UNSAT by stock; the engine added lemmas on {fired}, {lemmas} in all")
    return 0 if set(counts) <= {"ok"} else 1


if __name__ == "__main__":
    sys.exit(main())
