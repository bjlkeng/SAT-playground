#!/usr/bin/env python3
"""sum_read.py - sums written with adders, read by what they compute (bead
SAT-playground-d96.9.11).

An encoder can write "w1*b1 + ... + wn*bn compared with c" as a circuit of
half and full adders that makes the sum as a binary number, and then say
which numbers are allowed.  Unit propagation gets nothing out of that until
nearly every input is set: an adder does not know that its output is
bounded.  The count readers of the solver take unit weights only, and no
tool here said what such a circuit adds up.  This one does:

  1. gates and adder cells come from tools/netlist_census.py (a cell says
     "the sum of two or three signals = sum bit + 2 * carry bit");
  2. cells that feed each other form a network; a weight is given to every
     signal (a carry weighs twice its cell), and the cells' equations are
     added up, so that the inner signals cancel:
         sum of weight * input  +  constant  =  sum of weight * output
  3. every cell is asked of the formula itself: with its inputs set, each
     way, propagation must set its two output bits as an adder would (the
     gate reading is a guess; this is the check).  A network with a cell
     that fails is not read.  The identity is also checked on 64 random
     settings of the free variables;
  4. every pattern of the outputs is set, and propagation says which the
     formula allows: "at most c", "at least c", "exactly c" or a set.

A small example (the selftest).  Inputs a, b of weight 1 and c of weight 2:
a half adder makes a + b = s0 + 2*k0, a second one k0 + c = s1 + 2*k1, so
a + b + 2c = s0 + 2*s1 + 4*k1.  With the clauses "not k1" and "not s1 or
not s0" the outputs may be 0, 1 or 2: the file says a + b + 2c <= 2.

  read FILE                      the sums of one formula, a line each
  census OUT.tsv DIR|FILE...     a row a file: networks, sums read, their
                                 sizes and bounds [--jobs N]
  loss FILE [--sample N]         for each sum with a bound: inputs are set
                                 until one more would break the bound; does
                                 propagation rule that one out?
  variant FILE OUT [--encoding E]
                                 FILE plus each bounded sum written a second
                                 time in an encoding that propagates (PySAT,
                                 venv ~/.cache/sat13-enc/venv): for timing
                                 tests.  The added clauses use new variables
                                 and say only what the adders say already.
  selftest

The 2026 suite is never read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import multiprocessing
import random
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import netlist_census as nc  # noqa: E402
from cnf_prop import Prop  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
HOLDOUT = ROOT / "benchmarks" / "sat-comp-2026"
MAX_OUT = 14            # output signals of a network whose patterns are all tried
MAX_VARS = 3_000_000


def is_holdout(path):
    try:
        Path(path).resolve().relative_to(HOLDOUT.resolve())
        return True
    except ValueError:
        return False


def read_cnf(path):
    import lzma
    if is_holdout(path):
        sys.exit(f"{path}: the 2026 suite is the holdout, not read (plan section 8)")
    op = lzma.open if str(path).endswith(".xz") else open
    n_vars, clauses, cur = 0, [], []
    with op(path, "rt") as fh:
        for line in fh:
            if line[:1] in "c%":
                continue
            if line[:1] == "p":
                n_vars = int(line.split()[2])
                continue
            for t in line.split():
                x = int(t)
                if x == 0:
                    clauses.append(cur)
                    cur = []
                else:
                    cur.append(x)
                    n_vars = max(n_vars, abs(x))
    return n_vars, clauses


class Sum:
    """One network: `terms` {variable: weight} with `const`, equal to the
    number that `outs` {variable: weight} make; `allowed` the values the
    formula lets that number take (None when not looked at)."""

    def __init__(self, cells, terms, const, outs):
        self.cells, self.terms, self.const, self.outs = cells, terms, const, outs
        self.checked = False
        self.allowed = None
        self.why = ""

    def low_high(self):
        lo = self.const + sum(w for w in self.terms.values() if w < 0)
        hi = self.const + sum(w for w in self.terms.values() if w > 0)
        return lo, hi

    def bound(self):
        """('le', c), ('ge', c), ('eq', c), ('set', values), ('free', None) or (None, why)."""
        if self.allowed is None:
            return None, self.why
        lo, hi = self.low_high()
        can = [v for v in range(lo, hi + 1)]
        a = sorted(v for v in self.allowed if lo <= v <= hi)
        if not a:
            return "none", None
        if len(a) == len(can):
            return "free", None
        if len(a) == 1:
            return "eq", a[0]
        if a == list(range(lo, a[-1] + 1)):
            return "le", a[-1]
        if a == list(range(a[0], hi + 1)):
            return "ge", a[0]
        return "set", a

    def text(self):
        ws = Counter(abs(w) for w in self.terms.values())
        kind, c = self.bound()
        lo, hi = self.low_high()
        what = {"le": f"at most {c}", "ge": f"at least {c}", "eq": f"exactly {c}", "free": "any value",
                "none": "no value allowed", "set": f"one of {c}", None: f"not looked at ({c})"}[kind]
        return (f"{len(self.cells)} cells, {len(self.terms)} inputs (weight x count: "
                f"{' '.join(f'{w}x{k}' for w, k in sorted(ws.items()))}), constant {self.const}, range {lo}..{hi}, "
                f"{len(self.outs)} outputs: {what}")


def networks(cells):
    """Cells joined where one's sum or carry bit is another's input, with a
    weight a cell (a power of two): [(cell indices, {cell: exponent})], and
    how many were left out because their weights do not fit.  A signal that
    no cell makes joins nothing and fixes no weight: the same input may sit
    in two columns of one sum (3a is a in the column of ones and of twos),
    or in two sums."""
    maker = {}                          # signal -> (cell, 0 for its sum bit or 1 for its carry)
    for ci, (_kind, _leaves, _nm, s, _sn, cv, _cn) in enumerate(cells):
        maker[s] = (ci, 0)
        maker[cv] = (ci, 1)
    links = defaultdict(list)           # cell -> [(cell, its exponent minus this one's)]
    for cj, (_kind, leaves, _nm, _s, _sn, _cv, _cn) in enumerate(cells):
        for x in leaves:
            if x in maker:
                ci, role = maker[x]
                links[ci].append((cj, role))
                links[cj].append((ci, -role))
    seen = set()
    out = []
    bad = 0
    for start in range(len(cells)):
        if start in seen:
            continue
        exp = {start: 0}
        stack = [start]
        ok = True
        while stack:
            ci = stack.pop()
            for cj, d in links[ci]:
                e = exp[ci] + d
                if cj not in exp:
                    exp[cj] = e
                    stack.append(cj)
                elif exp[cj] != e:
                    ok = False
        seen.update(exp)
        if ok:
            low = min(exp.values())
            out.append((sorted(exp), {ci: e - low for ci, e in exp.items()}))
        else:
            bad += 1
    return out, bad


def linear(cells, members, exp):
    """The cells' equations added up: ({variable: coefficient}, constant),
    with  sum coefficient * variable + constant = 0  in every model."""
    coef = defaultdict(int)
    const = 0
    for ci in members:
        kind, leaves, negmask, s, sneg, cv, cneg = cells[ci]
        w = 1 << exp[ci]
        for i, x in enumerate(leaves):
            if (negmask >> i) & 1:
                const += w
                coef[x] -= w
            else:
                coef[x] += w
        if kind == "HA1":
            const += w
        for x, neg, m in ((s, sneg, w), (cv, cneg, 2 * w)):
            if neg:
                const -= m
                coef[x] += m
            else:
                coef[x] -= m
    return {x: c for x, c in coef.items() if c}, const


def cell_holds(p, cell):
    """The cell's equation, asked of the formula itself: with its two or
    three inputs set, each way, propagation must set the sum bit and the
    carry bit to what an adder gives.  (A setting that propagation refutes
    has no model and asks nothing.)  The gate reading is a guess at what the
    clauses define; this is the check that they do."""
    kind, leaves, negmask, s, sneg, cv, cneg = cell
    for pat in range(1 << len(leaves)):
        lits = [x if (pat >> i) & 1 else -x for i, x in enumerate(leaves)]
        got = p.implied(lits)
        if got is None:
            continue
        got = set(got)
        total = sum(((pat >> i) & 1) ^ ((negmask >> i) & 1) for i in range(len(leaves))) + (kind == "HA1")
        for x, neg, bit in ((s, sneg, total & 1), (cv, cneg, total >> 1)):
            want = x if bool(bit) != bool(neg) else -x
            if want not in got and p.value(want) != 1:
                return False
        if total > 3:
            return False
    return True


def read_sums(n_vars, clauses, look=True, rng=None):
    """The sums of a formula: (list of Sum, notes).  A sum is kept only when
    every one of its cells holds in the formula by propagation (`cell_holds`),
    so that its identity follows from the clauses and not from a reading of
    them."""
    notes = Counter()
    # the gate reading wants clauses without a literal twice or with its opposite
    plain = []
    for c in clauses:
        c = list(dict.fromkeys(c))
        if not any(-l in c for l in c):
            plain.append(c)
    gates, _used, _deadlocks, _topo = nc.recover_gates(plain, n_vars)
    notes["gates"] = len(gates)
    if not gates:
        return [], notes
    order = nc.topo_order(gates)
    rep, _merged = nc.structural_hash(gates, order)
    cells = nc.find_adders(gates, order, rep)
    notes["cells"] = len(cells)
    if not cells:
        return [], notes
    nets, bad = networks(cells)
    notes["networks"] = len(nets) + bad
    notes["weights_do_not_fit"] = bad
    # 64 random settings of the free variables, every gate computed
    rng = rng or random.Random(1)
    mask = (1 << 64) - 1
    present = {abs(l) for c in clauses for l in c}
    words = {v: rng.getrandbits(64) for v in present if v not in gates}
    val = nc.simulate(gates, order, rep, words, mask)

    def word(x):
        if x in val:
            return val[x]
        r, neg = rep.get(x, (x, False))
        w = val.get(r, words.get(r, 0))
        return (w ^ mask) if neg else w

    sums = []
    prop = Prop(n_vars, clauses)
    if prop.root_conflict:
        notes["refuted_by_units"] = 1
        return [], notes
    for members, exp in nets:
        if not all(cell_holds(prop, cells[ci]) for ci in members):
            notes["cells_not_in_the_clauses"] += 1
            continue
        coef, const = linear(cells, members, exp)
        made = {x for ci in members for x in (cells[ci][3], cells[ci][5])}
        terms = {x: c for x, c in coef.items() if x not in made}
        outs = {x: -c for x, c in coef.items() if x in made}
        s = Sum(members, terms, const, outs)
        if not terms or not outs:
            notes["no_inputs_or_outputs"] += 1
            continue
        # the identity on the random settings
        good = True
        for bit in range(64):
            lhs = const + sum(c * ((word(x) >> bit) & 1) for x, c in terms.items())
            rhs = sum(c * ((word(x) >> bit) & 1) for x, c in outs.items())
            if lhs != rhs:
                good = False
                break
        if not good:
            notes["identity_fails"] += 1
            continue
        s.checked = True
        if look:
            if len(outs) > MAX_OUT:
                s.why = f"{len(outs)} outputs"
                notes["too_many_outputs"] += 1
            else:
                xs = sorted(outs)
                allowed = set()
                for pat in range(1 << len(xs)):
                    lits = [x if (pat >> i) & 1 else -x for i, x in enumerate(xs)]
                    if not prop.refutes(lits):
                        allowed.add(sum(outs[x] for i, x in enumerate(xs) if (pat >> i) & 1))
                s.allowed = allowed
        sums.append(s)
    notes["sums"] = len(sums)
    return sums, notes


def literal_form(s):
    """The sum over literals with positive weights: (literals, weights, shift)
    with  sum weight * literal + shift = the number."""
    lits, weights, shift = [], [], s.const
    for x, c in sorted(s.terms.items()):
        if c > 0:
            lits.append(x)
            weights.append(c)
        else:                           # c * x = c - c * (not x) = c + |c| * (not x)
            lits.append(-x)
            weights.append(-c)
            shift += c
    return lits, weights, shift


def cmd_read(a):
    n_vars, clauses = read_cnf(a.file)
    t0 = time.time()
    sums, notes = read_sums(n_vars, clauses)
    print(f"{Path(a.file).name}: {dict(notes)} ({time.time() - t0:.0f} s)")
    kinds = Counter()
    for s in sorted(sums, key=lambda s: -len(s.cells)):
        kinds[s.bound()[0]] += 1
    print(f"   bounds: {dict(kinds)}")
    for s in sorted(sums, key=lambda s: -len(s.cells))[:a.show]:
        print("   " + s.text())


def loss_of(n_vars, clauses, sums, sample, rng):
    """For each sum with an upper or a lower bound: set inputs one at a time,
    in a random order, as long as the bound holds; whenever one more input
    would break it, that input must be off (or on, for a lower bound).  Of
    those, how many does propagation set?  (need, found, walks, broken).
    An input can set other inputs of the same sum.  When what is set already
    breaks the bound and propagation has not noticed, the setting has no
    model: the walk ends there, counted in `broken`, and nothing is asked of
    it."""
    p = Prop(n_vars, clauses)
    need = found = sit = broken = 0
    todo = [s for s in sums if s.bound()[0] in ("le", "ge", "eq")]
    for _ in range(sample):
        if not todo:
            break
        s = rng.choice(todo)
        kind, c = s.bound()
        lits, weights, shift = literal_form(s)
        order = list(range(len(lits)))
        rng.shuffle(order)
        # an upper bound is walked with the literals set true, a lower one with them set false
        for upper in ([True] if kind == "le" else [False] if kind == "ge" else [True, False]):
            side = [l if upper else -l for l in lits]
            mark = p.mark()
            for i in order:
                # the number as far as it is known: the weights of the literals that are on
                # (walking down: the weights of the literals that are not off yet)
                on = sum(w for l, w in zip(side, weights) if p.value(l) == 1)
                total = shift + on if upper else shift + sum(weights) - on
                if total > c if upper else total < c:
                    broken += 1
                    break
                l, v = side[i], p.value(side[i])
                if v == 1:
                    continue
                breaks = total + weights[i] > c if upper else total - weights[i] < c
                if breaks:
                    need += 1
                    found += v == -1
                    continue
                if v == -1:
                    continue
                if not p.assume([l]):
                    break               # no model this way by propagation: the walk ends here
            p.undo(mark)
            sit += 1
    return need, found, sit, broken


def cmd_loss(a):
    n_vars, clauses = read_cnf(a.file)
    sums, notes = read_sums(n_vars, clauses)
    need, found, sit, broken = loss_of(n_vars, clauses, sums, a.sample, random.Random(a.seed))
    print(f"{Path(a.file).name}: {notes['sums']} sums; {sit} walks; inputs that a bound rules out: {need}, "
          f"set by propagation: {found}; walks ended because what was set broke the bound unnoticed: {broken}")


def variant_clauses(n_vars, sums, encoding):
    """Each sum with a bound written a second time by PySAT: (clauses, top
    variable, how many of each kind).  A bound that no setting meets comes
    back as the empty clause, and is kept as that."""
    from pysat.pb import EncType, PBEnc
    top = n_vars
    extra = []
    kinds = Counter()
    for s in sums:
        kind, c = s.bound()
        if kind not in ("le", "ge", "eq"):
            continue
        lits, weights, shift = literal_form(s)
        fn = {"le": PBEnc.leq, "ge": PBEnc.geq, "eq": PBEnc.equals}[kind]
        cnf = fn(lits=lits, weights=weights, bound=c - shift, top_id=top, encoding=getattr(EncType, encoding))
        for cl in cnf.clauses:
            extra.append(list(cl))
            top = max([top] + [abs(l) for l in cl])
        kinds[kind] += 1
    return extra, top, kinds


def cmd_variant(a):
    n_vars, clauses = read_cnf(a.file)
    sums, notes = read_sums(n_vars, clauses)
    extra, top, kinds = variant_clauses(n_vars, sums, a.encoding)
    with open(a.out, "w") as fh:
        fh.write(f"p cnf {top} {len(clauses) + len(extra)}\n")
        for c in clauses + extra:
            fh.write(" ".join(map(str, c + [0])) + "\n")
    print(f"{Path(a.file).name}: {notes['sums']} sums, written again: {dict(kinds)}; {len(extra)} clauses and "
          f"{top - n_vars} variables added; wrote {a.out}")


COLS = ["stem", "vars", "clauses", "gates", "cells", "networks", "weights_do_not_fit", "cells_not_in_the_clauses",
        "identity_fails", "sums", "bounded",
        "le", "ge", "eq", "set", "free", "inputs_max", "weights", "biggest", "seconds", "error"]


def census_job(path):
    row = {"stem": Path(path).name.replace(".cnf.xz", "").replace(".cnf", ""), "error": ""}
    t0 = time.time()
    try:
        n_vars, clauses = read_cnf(path)
        row.update(vars=n_vars, clauses=len(clauses))
        if n_vars > MAX_VARS:
            row["error"] = "too-big"
        else:
            sums, notes = read_sums(n_vars, clauses)
            kinds = Counter(s.bound()[0] for s in sums)
            ws = Counter(abs(w) for s in sums for w in s.terms.values())
            big = max(sums, key=lambda s: len(s.terms), default=None)
            row.update(gates=notes["gates"], cells=notes["cells"], networks=notes["networks"],
                       weights_do_not_fit=notes["weights_do_not_fit"], identity_fails=notes["identity_fails"],
                       cells_not_in_the_clauses=notes["cells_not_in_the_clauses"],
                       sums=len(sums), bounded=kinds["le"] + kinds["ge"] + kinds["eq"], le=kinds["le"], ge=kinds["ge"],
                       eq=kinds["eq"], set=kinds["set"], free=kinds["free"],
                       inputs_max=len(big.terms) if big else 0,
                       weights=" ".join(f"{w}x{k}" for w, k in sorted(ws.items())[:8]),
                       biggest=big.text() if big else "")
    except MemoryError:
        row["error"] = "memory"
    except Exception as e:  # noqa: BLE001
        row["error"] = f"{type(e).__name__}: {str(e)[:80]}"
    row["seconds"] = f"{time.time() - t0:.0f}"
    return row


def cmd_census(a):
    files = []
    for src in a.paths:
        pth = Path(src)
        files += sorted(pth.glob("*.cnf*")) if pth.is_dir() else [pth]
    if any(is_holdout(f) for f in files):
        sys.exit("the 2026 suite is the holdout: not read (plan section 8)")
    out = Path(a.out)
    done = {}
    if out.exists():
        with open(out, newline="") as fh:
            done = {r["stem"]: r for r in csv.DictReader(fh, delimiter="\t")}
    todo = [f for f in files if Path(f).name.replace(".cnf.xz", "").replace(".cnf", "") not in done]
    print(f"{len(files)} files, {len(todo)} to do", flush=True)
    rows = dict(done)

    def write():
        with open(out, "w", newline="") as fh:
            w = csv.DictWriter(fh, fieldnames=COLS, delimiter="\t", lineterminator="\n", extrasaction="ignore")
            w.writeheader()
            for r in sorted(rows.values(), key=lambda r: r["stem"]):
                w.writerow({k: r.get(k, "") for k in COLS})

    with multiprocessing.get_context("fork").Pool(a.jobs, maxtasksperchild=1) as pool:
        for k, row in enumerate(pool.imap_unordered(census_job, [str(f) for f in todo]), 1):
            rows[row["stem"]] = row
            if k % 10 == 0 or k == len(todo):
                write()
                print(f"{k} of {len(todo)} done", flush=True)
    write()


def selftest(a):
    # the inputs a, b, c are gates themselves (ANDs of free variables), as in a real file:
    # a + b + 2c by two half adders, at most 2
    A, B, C, S0, K0, S1, K1 = range(7, 14)

    def gate_and(o, x, y):
        return [[-o, x], [-o, y], [o, -x, -y]]

    def half(x, y, s, k):
        return [[-x, -y, -s], [x, y, -s], [x, -y, s], [-x, y, s]] + gate_and(k, x, y)

    base = gate_and(A, 1, 2) + gate_and(B, 3, 4) + gate_and(C, 5, 6)
    cl = base + half(A, B, S0, K0) + half(K0, C, S1, K1) + [[-K1], [-S1, -S0]]
    sums, notes = read_sums(13, cl)
    assert len(sums) == 1, (len(sums), dict(notes))
    s = sums[0]
    assert s.terms == {A: 1, B: 1, C: 2} and s.const == 0 and s.checked, (s.terms, s.const, dict(notes))
    assert s.outs == {S0: 1, S1: 2, K1: 4}, s.outs
    assert s.bound() == ("le", 2), s.bound()
    # with c on, a and b are ruled out; propagation does not see it (the adders do not run backwards)
    p = Prop(13, cl)
    got = p.implied([C])
    assert -A not in got and -B not in got
    need, found, _sit, _broken = loss_of(13, cl, sums, 300, random.Random(1))
    assert need > found >= 0, (need, found)
    # inputs that set each other.  The sum a + 2b + 3c + 2d + e + 3f <= 6 as a bound that
    # propagation knows nothing of (no clause holds it), with b -> c and b -> d: once b is set,
    # b, c and d weigh 7 and nothing has noticed.  Such a walk ends there, counted apart, and
    # nothing is asked of a setting that has no model.
    free = Sum([], {1: 1, 2: 2, 3: 3, 4: 2, 5: 1, 6: 3}, 0, {})
    free.allowed = set(range(7))
    assert free.bound() == ("le", 6)
    need3, found3, sit3, broken3 = loss_of(6, [[-2, 3], [-2, 4]], [free], 300, random.Random(1))
    assert sit3 == 300 and broken3 > 50 and need3 > 0 and found3 == 0, (need3, found3, sit3, broken3)
    # without those two clauses no walk is broken
    assert loss_of(6, [[1, 2, 3, 4, 5, 6]], [free], 300, random.Random(1))[3] == 0
    # exactly 1 of a, b by one half adder: the outputs must be s = 1, k = 0
    cl = base + half(A, B, S0, K0) + [[S0], [-K0]]
    sums, notes = read_sums(11, cl)
    assert len(sums) == 1 and sums[0].bound() == ("eq", 1), (dict(notes), [s.bound() for s in sums])
    lits, weights, shift = literal_form(sums[0])
    assert (sorted(lits), weights, shift) == ([A, B], [1, 1], 0)
    # an input in two columns: a + b = s0 + 2k and k + a = s1 + 2c make 3a + b = s0 + 2*s1 + 4c
    cl = base + half(A, B, S0, K0) + half(K0, A, S1, K1) + [[-K1]]
    sums, notes = read_sums(13, cl)
    assert len(sums) == 1 and sums[0].terms == {A: 3, B: 1} and sums[0].outs == {S0: 1, S1: 2, K1: 4}, (
        dict(notes), [(s.terms, s.outs) for s in sums])
    assert sums[0].bound() == ("le", 3), sums[0].bound()
    # a bound that nothing meets: 3a + b = 2.  The second writing is then the empty clause,
    # and it is kept (tried only where PySAT is there: the venv of tools/enc/requirements.txt)
    cl = base + half(A, B, S0, K0) + half(K0, A, S1, K1) + [[-S0], [S1], [-K1]]
    sums, notes = read_sums(13, cl)
    assert len(sums) == 1 and sums[0].bound() == ("eq", 2), (dict(notes), [s.bound() for s in sums])
    try:
        import pysat.pb  # noqa: F401
        extra, top, kinds = variant_clauses(13, sums, "bdd")
        assert [] in extra and top >= 13 and kinds == {"eq": 1}, (extra, top, kinds)
        # and one that can be met: the second writing holds in a model of the first
        cl2 = base + half(A, B, S0, K0) + half(K0, C, S1, K1) + [[-K1], [-S1, -S0]]
        sums2, _notes = read_sums(13, cl2)
        extra2, top2, _k = variant_clauses(13, sums2, "bdd")
        q = Prop(top2, cl2 + extra2)
        assert not q.root_conflict and q.implied([A, B, -C]) is not None and q.refutes([A, B, C])
        with_pysat = "the second writing of a sum tried with PySAT"
    except ImportError:
        with_pysat = "PySAT is not here: the second writing of a sum was not tried"
    # two sums that share an input and nothing else stay two sums
    cl = base + half(A, B, S0, K0) + half(A, C, S1, K1) + [[-K0], [-K1]]
    sums, notes = read_sums(13, cl)
    assert sorted(tuple(sorted(s.terms)) for s in sums) == [(A, B), (A, C)], [s.terms for s in sums]
    # the adder toys of the encoding catalog (PySAT writes them; the cells found there are a
    # part of each sum).  Whatever is read must hold in every model: for each allowed joint
    # value of a toy's inputs a model is found, and each sum's identity and bound are checked in it.
    import z3_catalog as zc
    from cnf_prop import satisfiable
    from enc_catalog import CATALOG
    checks = 0
    if CATALOG.exists():
        for rec in zc.load(CATALOG):
            if rec["primitive"] != "weighted" or rec["options"] != ["adder"]:
                continue
            cl = [list(c) for c in rec["clauses"]]
            got, notes = read_sums(rec["n_vars"], cl)
            assert got, (rec["id"], dict(notes))
            q = Prop(rec["n_vars"], cl)
            for i, lits in zc.joint(rec):
                if rec["hard"][i] != "1":
                    continue
                model = satisfiable(q, lits)
                assert model is not None, (rec["id"], lits)
                on = {v for v, value in model.items() if value}
                for sm in got:
                    lhs = sm.const + sum(c for x, c in sm.terms.items() if x in on)
                    rhs = sum(c for x, c in sm.outs.items() if x in on)
                    assert lhs == rhs, (rec["id"], sm.terms, sm.outs, lhs, rhs)
                    assert sm.allowed is None or rhs in sm.allowed, (rec["id"], rhs, sorted(sm.allowed))
                    checks += 1
        assert checks > 100, checks
    # a clause with a literal and its opposite says nothing.  Here the half adder's sum bit is
    # "defined" by such clauses only: the gate reading takes them for a definition, the formula
    # does not hold the cell, and no sum may be read (a + b = 1 would rule out the model a = b = 1)
    fake = [[-A, -B, -S0, S0], [A, B, -S0, S0], [A, -B, S0, -S0], [-A, B, S0, -S0]] + gate_and(K0, A, B)
    cl = base + fake + [[S0], [-K0]]
    sums, notes = read_sums(11, cl)
    assert not sums, (dict(notes), [s.text() for s in sums])
    # one half of the sum bit's definition only (s -> a xor b): the other way is not in the
    # clauses, so setting a and b does not set s, and the cell is not taken
    cl = base + [[-A, -B, -S0], [A, B, -S0]] + gate_and(K0, A, B) + [[S0], [-K0]]
    sums, notes = read_sums(11, cl)
    assert not sums, (dict(notes), [s.text() for s in sums])
    print(f"selftest: ok (a + b + 2c <= 2 read from two half adders: of {need} inputs the bound rules out, "
          f"propagation sets {found}; a + b = 1 read from one; 3a + b with a in two columns; the sums read from "
          f"PySAT's adder toys hold in {checks} model checks; {with_pysat})")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("read")
    s.add_argument("file")
    s.add_argument("--show", type=int, default=12)
    s = sub.add_parser("loss")
    s.add_argument("file")
    s.add_argument("--sample", type=int, default=3000)
    s.add_argument("--seed", type=int, default=1)
    s = sub.add_parser("variant")
    s.add_argument("file")
    s.add_argument("out")
    s.add_argument("--encoding", default="bdd")
    s = sub.add_parser("census")
    s.add_argument("out")
    s.add_argument("paths", nargs="+")
    s.add_argument("--jobs", type=int, default=4)
    sub.add_parser("selftest")
    a = ap.parse_args(argv)
    {"read": cmd_read, "loss": cmd_loss, "variant": cmd_variant, "census": cmd_census, "selftest": selftest}[a.cmd](a)


if __name__ == "__main__":
    main()
