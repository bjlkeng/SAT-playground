#!/usr/bin/env python3
"""enc_catalog.py - toys of the usual encodings, from encoders other than Z3
(bead SAT-playground-d96.9.12).

The Z3 catalog (tools/z3_catalog.py) has one toy per thing Z3 writes.  Other
encoders write numbers and counts in other ways, and a reader that is to be
general has to be tried on those too.  This tool makes small formulas with
known meaning in the encodings a SAT file is likely to hold:

  counts over plain Booleans   at most, at least, exactly k of 6: pairwise,
                               sequential counter, sorting network,
                               cardinality network, totalizer and two
                               variants, and for at most one also the binary
                               and the ladder form (PySAT, pysat.card)
  weighted sums over Booleans  sum of w*b at most, at least, equal to c:
                               decision diagram, sequential counter, sorting
                               network, adders, binary merge, totalizer,
                               watchdog, mixed radix (PySAT, pysat.pb)
  numbers 0..3 with a constraint between two or three of them
                               (x <= y, x != y, x + y <= 4, x + y = 3,
                               |x - y| >= 2, all different, x + y + z <= 5,
                               x + y = z), written by hand in the standard
                               ways: one variable a value with a clause a
                               forbidden pair (`direct-conflict`) or a clause
                               a value listing its partners
                               (`direct-support`); sorted bits with clauses
                               over bounds (`order-bounds`, as Sugar does) or
                               a clause a forbidden pair (`order-conflict`);
                               binary digits with a clause a forbidden pair
                               (`log-conflict`) or an adder (`log-adder`)
  value tests T <-> (x = c) on one number in each of the three codes

A record has the format of the Z3 catalog (clauses, inputs with a pattern a
value, tests, the table of allowed joint values), so that tool's checks and
counts work on it as they are:

  build     make the records (needs PySAT: venv ~/.cache/sat13-enc/venv,
            tools/enc/requirements.txt; run with that venv's python)
  check     brute force, no PySAT: every allowed joint value has a model, no
            other has one, every test is what its table says
  table     benchmarks/rl/enc_catalog_shapes.tsv: for each record, of the
            input literals that should follow from one or two input
            literals, how many propagation sets (tools/z3_catalog.py `shape`)
  summary   the table by encoding
  selftest  three records by hand

A small example.  `sum-le-4@order-bounds`: x and y in 0..3 as sorted bits,
x + y <= 4 as the clauses "x >= 2 -> y <= 2", "x >= 3 -> y <= 1" and the
same from y.  Setting "x >= 3" sets "y <= 1": nothing is lost.  The same
constraint as `log-adder` (binary digits, an adder, the sums 5 and 6
forbidden) sets nothing when x's high digit is set.
"""
from __future__ import annotations

import argparse
import itertools
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import z3_catalog as zc  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "benchmarks" / "rl" / "enc_catalog.jsonl.xz"
SHAPES = ROOT / "benchmarks" / "rl" / "enc_catalog_shapes.tsv"
D = 4           # values of a number: 0..3
N_BOOL = 6      # inputs of a count


class Vars:
    def __init__(self):
        self.n = 0

    def new(self, k=1):
        out = list(range(self.n + 1, self.n + k + 1))
        self.n += k
        return out


class Number:
    """A number 0..d-1 in one of three codes: its variables, the clauses
    that keep it to its values, and how to say things about it."""

    def __init__(self, vs, code, name, d=D):
        self.code, self.name, self.d = code, name, d
        self.clauses = []
        if code == "order":             # bit k on <-> value > k
            self.vars = vs.new(d - 1)
            self.clauses = [[-self.vars[k + 1], self.vars[k]] for k in range(d - 2)]
            self.patterns = [[v if k < j else -v for k, v in enumerate(self.vars)] for j in range(d)]
        elif code == "direct":          # one variable a value, exactly one on
            self.vars = vs.new(d)
            self.clauses = [list(self.vars)] + [[-a, -b] for a, b in itertools.combinations(self.vars, 2)]
            self.patterns = [[v if k == j else -v for k, v in enumerate(self.vars)] for j in range(d)]
        elif code == "log":             # binary digits, lowest first
            nb = max(1, (d - 1).bit_length())
            self.vars = vs.new(nb)
            self.patterns = [[v if (j >> k) & 1 else -v for k, v in enumerate(self.vars)] for j in range(d)]
            for j in range(d, 1 << nb):
                self.clauses.append([-v if (j >> k) & 1 else v for k, v in enumerate(self.vars)])
        else:
            raise ValueError(code)

    def input(self):
        inp = {"kind": {"order": "order", "direct": "direct", "log": "binary"}[self.code], "name": self.name,
               "vars": list(self.vars), "values": list(range(self.d)), "patterns": self.patterns}
        if self.code == "order":
            inp["lo"] = 0
        return inp

    def is_not(self, j):
        """A clause part that says "the value is not j"."""
        if self.code == "direct":
            return [-self.vars[j]]
        if self.code == "log":
            return [-l for l in self.patterns[j]]
        out = []
        if j >= 1:
            out.append(-self.vars[j - 1])       # value <= j - 1
        if j <= self.d - 2:
            out.append(self.vars[j])            # value >= j + 1
        return out

    def ge(self, k):
        """The literal "value >= k" of a number in sorted bits: True, False or a literal."""
        assert self.code == "order"
        if k <= 0:
            return True
        if k >= self.d:
            return False
        return self.vars[k - 1]


def clause(parts):
    """A clause from literals, True and False: None when it is always true."""
    out = []
    for l in parts:
        if l is True:
            return None
        if l is False:
            continue
        out.append(l)
    return out


def neg(l):
    return (not l) if isinstance(l, bool) else -l


RELATIONS = {
    "le": (2, lambda x, y: x <= y, "x <= y"),
    "ne": (2, lambda x, y: x != y, "x != y"),
    "sum-le-4": (2, lambda x, y: x + y <= 4, "x + y <= 4"),
    "sum-eq-3": (2, lambda x, y: x + y == 3, "x + y = 3"),
    "apart-2": (2, lambda x, y: abs(x - y) >= 2, "|x - y| >= 2"),
    "alldiff-3": (3, lambda x, y, z: len({x, y, z}) == 3, "x, y, z all different"),
    "sum3-le-5": (3, lambda x, y, z: x + y + z <= 5, "x + y + z <= 5"),
    "sum-is-z": (3, lambda x, y, z: x + y == z, "x + y = z"),
}


def bounds_clauses(name, nums):
    """The relation over sorted bits as clauses over bounds, as Sugar writes
    a linear constraint: None when the relation has no such form here."""
    out = []
    if name == "le":
        x, y = nums
        out = [clause([neg(x.ge(k)), y.ge(k)]) for k in range(1, D)]
    elif name == "sum-le-4":
        x, y = nums
        out = [clause([neg(x.ge(a)), neg(y.ge(4 - a + 1))]) for a in range(D)]
    elif name == "sum-eq-3":
        x, y = nums
        out = [clause([neg(x.ge(a)), neg(y.ge(3 - a + 1))]) for a in range(D)]            # x >= a -> y <= 3 - a
        out += [clause([x.ge(a + 1), y.ge(3 - a)]) for a in range(D)]                     # x <= a -> y >= 3 - a
    elif name == "sum3-le-5":
        x, y, z = nums
        out = [clause([neg(x.ge(a)), neg(y.ge(b)), neg(z.ge(5 - a - b + 1))]) for a in range(D) for b in range(D)]
    elif name == "sum-is-z":
        x, y, z = nums
        out = [clause([neg(x.ge(a)), neg(y.ge(b)), z.ge(a + b)]) for a in range(D) for b in range(D)]       # z >= x + y
        out += [clause([x.ge(a + 1), y.ge(b + 1), neg(z.ge(a + b + 1))]) for a in range(D) for b in range(D)]  # z <= x + y
    else:
        return None
    return [c for c in out if c is not None]


def adder_clauses(vs, name, nums):
    """x + y by a ripple adder over binary digits; the sum's value is then
    kept to the allowed ones by a clause a forbidden value.  None when the
    relation is no sum."""
    if name not in ("sum-le-4", "sum-eq-3", "sum-is-z"):
        return None
    x, y = nums[0], nums[1]
    cls = []
    carry = None
    sums = []
    for k in range(len(x.vars)):
        a, b = x.vars[k], y.vars[k]
        s, = vs.new()
        if carry is None:               # half adder
            c, = vs.new()
            cls += [[-a, -b, -s], [a, b, -s], [a, -b, s], [-a, b, s]]                 # s = a xor b
            cls += [[-c, a], [-c, b], [c, -a, -b]]                                    # c = a and b
        else:                           # full adder
            c, = vs.new()
            for sa, sb, sc in itertools.product((1, -1), repeat=3):
                odd = (sa < 0) ^ (sb < 0) ^ (sc < 0)        # the three inputs set to (sa>0, sb>0, sc>0)
                ones = (sa > 0) + (sb > 0) + (sc > 0)
                cls.append([-sa * a, -sb * b, -sc * carry, s if ones % 2 else -s])
                cls.append([-sa * a, -sb * b, -sc * carry, c if ones >= 2 else -c])
                del odd
        sums.append(s)
        carry = c
    sums.append(carry)                  # the sum, lowest digit first
    top = 1 << len(sums)
    if name == "sum-is-z":
        z = nums[2]
        for v in range(top):
            cube = [s if (v >> k) & 1 else -s for k, s in enumerate(sums)]
            if v < D:                   # sum = v -> z = v
                cls += [[-l for l in cube] + [zl] for zl in z.patterns[v]]
            else:
                cls.append([-l for l in cube])
    else:
        ok = (lambda v: v <= 4) if name == "sum-le-4" else (lambda v: v == 3)
        for v in range(top):
            if not ok(v):
                cls.append([-s if (v >> k) & 1 else s for k, s in enumerate(sums)])
    return cls


def number_records():
    recs = []
    for name, (arity, rel, what) in RELATIONS.items():
        for enc in ("direct-conflict", "direct-support", "order-bounds", "order-conflict", "log-conflict", "log-adder"):
            code = enc.split("-")[0]
            vs = Vars()
            nums = [Number(vs, code, "xyz"[i]) for i in range(arity)]
            cls = [c for nb in nums for c in nb.clauses]
            joint = list(itertools.product(range(D), repeat=arity))
            how = enc.split("-", 1)[1]
            if how == "conflict":
                for vals in joint:
                    if not rel(*vals):
                        cls.append([l for nb, v in zip(nums, vals) for l in nb.is_not(v)])
            elif how == "support":
                if arity != 2:
                    continue
                x, y = nums
                for a in range(D):
                    cls.append([-x.vars[a]] + [y.vars[b] for b in range(D) if rel(a, b)])
                    cls.append([-y.vars[a]] + [x.vars[b] for b in range(D) if rel(b, a)])
            elif how == "bounds":
                more = bounds_clauses(name, nums)
                if more is None:
                    continue
                cls += more
            elif how == "adder":
                more = adder_clauses(vs, name, nums)
                if more is None:
                    continue
                cls += more
            recs.append({"id": f"{name}@{enc}", "name": name, "primitive": "numbers", "options": [enc], "z3": "",
                         "n_vars": vs.n, "inputs": [nb.input() for nb in nums], "tests": [],
                         "hard": "".join("1" if rel(*v) else "0" for v in joint), "hard_what": [what], "clauses": cls})
    # value tests T_c <-> (x = c), one variable a test, as a gate over the number's own literals
    for code in ("order", "direct", "log"):
        vs = Vars()
        x = Number(vs, code, "x")
        cls = list(x.clauses)
        tests = []
        for c in range(D):
            t, = vs.new()
            cube = [-l for l in x.is_not(c)] if code != "order" else [-l for l in x.is_not(c)]
            cls += [[-t, l] for l in cube] + [[t] + [-l for l in cube]]
            tests.append({"form": "iff", "name": f"T_eq_{c}", "var": t, "what": f"x = {c}",
                          "table": "".join("1" if v == c else "0" for v in range(D))})
        recs.append({"id": f"value-tests@{code}", "name": "value-tests", "primitive": "tests", "options": [code], "z3": "",
                     "n_vars": vs.n, "inputs": [x.input()], "tests": tests, "clauses": cls})
    return recs


def bool_inputs(n):
    return [{"kind": "bool", "name": f"b{i}", "vars": [i + 1], "values": [0, 1], "patterns": [[-(i + 1)], [i + 1]]} for i in range(n)]


def pysat_records():
    from pysat.card import CardEnc, EncType
    from pysat.pb import EncType as PBType
    from pysat.pb import PBEnc
    import pysat
    recs = []
    lits = list(range(1, N_BOOL + 1))
    joint = list(itertools.product((0, 1), repeat=N_BOOL))
    version = f"pysat {pysat.__version__}"
    encs = ["pairwise", "seqcounter", "sortnetwrk", "cardnetwrk", "totalizer", "mtotalizer", "kmtotalizer"]
    for kind, bounds, fn, ok in (("atmost", (1, 2, 3), CardEnc.atmost, lambda s, k: s <= k),
                                 ("atleast", (2, 3), CardEnc.atleast, lambda s, k: s >= k),
                                 ("exactly", (1, 2, 3), CardEnc.equals, lambda s, k: s == k)):
        for k in bounds:
            for enc in encs + (["bitwise", "ladder"] if (kind, k) == ("atmost", 1) else []):
                try:
                    cnf = fn(lits=lits, bound=k, top_id=N_BOOL, encoding=getattr(EncType, enc))
                except Exception as e:      # an encoder that does not take this form
                    print(f"skipped count-{kind}-{k}@{enc}: {type(e).__name__}: {str(e)[:80]}")
                    continue
                cls = [list(c) for c in cnf.clauses]
                recs.append({"id": f"count-{kind}-{k}@{enc}", "name": f"count-{kind}-{k}", "primitive": "count", "options": [enc],
                             "z3": version, "n_vars": max([N_BOOL] + [abs(l) for c in cls for l in c]), "inputs": bool_inputs(N_BOOL),
                             "tests": [], "hard": "".join("1" if ok(sum(v), k) else "0" for v in joint),
                             "hard_what": [f"{kind} {k} of {N_BOOL}"], "clauses": cls})
    for wname, weights in (("w123213", [1, 2, 3, 2, 1, 3]), ("w112244", [1, 1, 2, 2, 4, 4])):
        for kind, bound, fn, ok in (("le", 6, PBEnc.leq, lambda s, c: s <= c), ("ge", 6, PBEnc.geq, lambda s, c: s >= c),
                                    ("eq", 6, PBEnc.equals, lambda s, c: s == c)):
            for enc in ("bdd", "seqcounter", "sortnetwrk", "adder", "binmerge", "totalizer", "watchdog", "mixedradix"):
                try:
                    cnf = fn(lits=lits, weights=weights, bound=bound, top_id=N_BOOL, encoding=getattr(PBType, enc))
                except Exception as e:
                    print(f"skipped weighted-{kind}-{wname}@{enc}: {type(e).__name__}: {str(e)[:80]}")
                    continue
                cls = [list(c) for c in cnf.clauses]
                table = "".join("1" if ok(sum(w * b for w, b in zip(weights, v)), bound) else "0" for v in joint)
                recs.append({"id": f"weighted-{kind}-{wname}@{enc}", "name": f"weighted-{kind}-{wname}", "primitive": "weighted",
                             "options": [enc], "z3": version, "n_vars": max([N_BOOL] + [abs(l) for c in cls for l in c]),
                             "inputs": bool_inputs(N_BOOL), "tests": [], "hard": table,
                             "hard_what": [f"sum of {weights} {kind} {bound}"], "clauses": cls})
    return recs


def cmd_build(a):
    recs = number_records() + pysat_records()
    bad = 0
    keep = []
    for r in recs:
        probs = zc.meaning_problems(r)
        if probs:
            bad += 1
            print(f"NOT KEPT {r['id']}: {probs[0]}")
        else:
            keep.append(r)
    keep.sort(key=lambda r: r["id"])
    zc.save(keep, a.catalog)
    print(f"{len(keep)} records written to {a.catalog}; {bad} not kept (their clauses do not mean the constraint)")


def cmd_check(a):
    recs = zc.load(a.catalog)
    bad = 0
    for r in recs:
        probs = zc.meaning_problems(r)
        if probs:
            bad += 1
            print(f"{r['id']}: {probs[0]}")
    print(f"{len(recs)} records, {bad} with a problem")
    sys.exit(1 if bad else 0)


COLS = ["id", "primitive", "encoding", "vars", "clauses", "h1_need", "h1_up", "h1_rup", "h2_need", "h2_up", "h2_rup",
        "t0_need", "t0_up", "t0_rup", "t1_need", "t1_up", "t1_rup"]


def shape_row(rec):
    s = zc.shape(rec)
    return {"id": rec["id"], "primitive": rec["primitive"], "encoding": rec["options"][0], "vars": rec["n_vars"],
            "clauses": len(rec["clauses"]), "h1_need": s["h1_need"], "h1_up": s["h1_up"], "h1_rup": s["h1_rup"],
            "h2_need": s["h2_need"], "h2_up": s["h2_up"], "h2_rup": s["h2_rup"], "t0_need": s["0_need"], "t0_up": s["0_up"],
            "t0_rup": s["0_rup"], "t1_need": s["1_need"], "t1_up": s["1_up"], "t1_rup": s["1_rup"]}


def cmd_table(a):
    recs = zc.load(a.catalog)
    rows = [shape_row(r) for r in recs]
    with open(a.shapes, "w") as fh:
        fh.write("\t".join(COLS) + "\n")
        for r in rows:
            fh.write("\t".join(str(r[c]) for c in COLS) + "\n")
    print(f"{len(rows)} rows written to {a.shapes}")


def cmd_summary(a):
    rows = []
    with open(a.shapes) as fh:
        head = fh.readline().rstrip("\n").split("\t")
        for line in fh:
            rows.append(dict(zip(head, line.rstrip("\n").split("\t"))))
    by = defaultdict(Counter)
    for r in rows:
        key = (r["primitive"], r["encoding"])
        by[key]["records"] += 1
        for c in COLS[5:]:
            if r[c] != "NA":
                by[key][c] += int(r[c])
        by[key]["gap"] += any(r[f"{k}_need"] != "NA" and int(r[f"{k}_need"]) > int(r[f"{k}_up"]) for k in ("h1", "h2", "t0", "t1"))
    print(f"{'primitive':9s} {'encoding':16s} toys with-a-gap | from one input literal: should follow, set | from two: should follow, set")
    for (prim, enc), n in sorted(by.items()):
        print(f"{prim:9s} {enc:16s} {n['records']:4d} {n['gap']:4d} | {n['h1_need'] + n['t0_need']:6d} {n['h1_up'] + n['t0_up']:6d} | "
              f"{n['h2_need'] + n['t1_need']:6d} {n['h2_up'] + n['t1_up']:6d}")


def selftest(a):
    recs = {r["id"]: r for r in number_records()}
    for r in recs.values():
        probs = zc.meaning_problems(r)
        assert not probs, (r["id"], probs[:2])
    # the docstring's example: over bounds nothing is lost; by an adder a digit of x sets nothing
    s = zc.shape(recs["sum-le-4@order-bounds"])
    assert s["h1_need"] == s["h1_up"] > 0 and s["h2_need"] == s["h2_up"], s
    s = zc.shape(recs["sum-le-4@log-adder"])
    assert s["h1_need"] > s["h1_up"] or s["h2_need"] > s["h2_up"], s
    # one variable a value.  A clause a forbidden pair: "x is not 0" should rule out "y is 3"
    # (3 needs 0), and propagation does not do it; one probe shows it.  A clause a value
    # listing its partners ("y is 3 -> x is 0") loses nothing.
    s = zc.shape(recs["sum-eq-3@direct-conflict"])
    assert (s["h1_need"], s["h1_up"], s["h1_rup"]) == (40, 32, 8), s
    s = zc.shape(recs["sum-eq-3@direct-support"])
    assert s["h1_need"] == s["h1_up"] == 40, s
    # value tests written as gates lose nothing, in any of the three codes
    for code in ("order", "direct", "log"):
        s = zc.shape(recs[f"value-tests@{code}"])
        assert s["0_need"] == s["0_up"] and s["fwd_need"] == s["fwd_up"], (code, s)
    print(f"selftest: ok ({len(recs)} hand-written records mean what they say)")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["build", "check", "table", "summary", "selftest"])
    ap.add_argument("--catalog", default=str(CATALOG))
    ap.add_argument("--shapes", default=str(SHAPES))
    a = ap.parse_args(argv)
    {"build": cmd_build, "check": cmd_check, "table": cmd_table, "summary": cmd_summary, "selftest": selftest}[a.cmd](a)


if __name__ == "__main__":
    main()
