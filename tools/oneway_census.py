#!/usr/bin/env python3
"""oneway_census.py - from which sides does unit propagation see an implied
clause?  A measure that is blind to the encoding (beads SAT-playground-d96.9.9
and d96.9.10).

Unit propagation is the solver's cheap rule: when every literal of a clause
but one is false, the last one is true.  Take a clause the formula implies,
say "not P or b".  It has two sides.  From P's side: set P, and propagation
sets b.  From b's side: set b false, and propagation sets P false.  An
encoding can give one side and not the other.  Z3's sorted-bit numbers do:
setting a bit false turns a value test off, and setting the test on sets no
bit (solver README, "Numbers stored as sorted bits").  The number reader
finds that case by reading numbers.  This tool counts it for any formula,
with no reading at all:

  two-literal clauses   two-way (both sides see it), one-way (one side),
                        hidden (no side, but setting both literals false
                        makes propagation fail: one probe shows it)
  three-literal clauses seen from 3, 2 or 1 of their three sides (a side:
                        two literals false, propagation sets the third), or
                        from none though the three together fail; only
                        clauses with no two-literal part that fails.  On a
                        big formula these are met by sampling pairs of
                        literals, two ways: near pairs (t1, t2, t3: the
                        second literal cuts a clause the first has cut,
                        where joint consequences are dense; no sample of
                        all pairs) and far pairs (f1, f2, f3: any open
                        literal, drawn evenly)

A small example.  s -> a, s -> b and "not a or not b or m": setting s sets a
and b, and the long clause sets m.  Setting m false leaves "not a or not b"
and sets nothing.  So "not s or m" is one-way: 1 one-way clause, 2 two-way.

The counting is done by tools/one-way (Rust; `cargo build --release` there,
or `build` here).  `reference` below is the same count in plain Python, every
pair and every triple, for toys: the selftest holds the two equal.

  build                         compile tools/one-way
  selftest                      toys by hand, then Rust against Python on
                                small records of the Z3 catalog, then closing
  one FILE [--all] [options]    the tool's line for one formula
  census OUT.tsv DIR|FILE... [--jobs N] [--cores LIST] [--work W] [--seed S]
                                a row a file; rows already in OUT.tsv are kept
  report TABLE... [--top N]     by family
  close FILE OUT [--max-add N]  FILE plus the missing side of every one-way
                                two-literal clause (each follows by unit
                                propagation from what stands before it)
  pyramid FILE N [--solver BIN] a Bel's Pyramid cell, as it is and with the
                                number pass's clauses, by kind of variable

The 2026 suite is never read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import itertools
import multiprocessing
import os
import subprocess
import sys
import tempfile
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from cnf_prop import Prop  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
CRATE = ROOT / "tools" / "one-way"
BIN = Path(os.environ.get("ONEWAY_BIN") or CRATE / "target" / "release" / "one-way")   # ONEWAY_BIN: a frozen copy
HOLDOUT = ROOT / "benchmarks" / "sat-comp-2026"
INT_KEYS = ("vars", "clauses", "units", "active", "bin", "long", "sources", "failed", "seen_max", "checked",
            "probe_sources", "pairs", "pair_one_way", "pair_hidden", "pairs_with_joint", "joint_checked", "triples",
            "work_a", "work")
COLUMNS = ["stem", "status", "vars", "clauses", "units", "active", "bin", "long", "sources", "failed", "seen", "seen_max",
           "checked", "two_way", "one_way", "hidden_unit", "one_way_share", "seen_all", "two_way_all", "one_way_all",
           "roots", "roots_two_way", "roots_unit", "roots_all",
           "eq_vars", "eq_two_way", "eq_hidden", "eq_units", "eq_hidden_all",
           "probe_sources", "pairs", "pair_one_way", "pair_hidden", "pairs_with_joint", "joint", "joint_checked",
           "improper", "t1", "t2", "t3", "t0", "triples",
           "far_pairs", "far_pair_one_way", "far_pair_hidden", "far_with_joint", "far_joint", "far_checked", "far_improper",
           "f1", "f2", "f3", "work_a", "work", "seconds", "wall_s", "error"]


def is_holdout(path):
    try:
        Path(path).resolve().relative_to(HOLDOUT.resolve())
        return True
    except ValueError:
        return False


def build():
    if os.environ.get("ONEWAY_BIN"):
        return
    src = max(p.stat().st_mtime for p in (CRATE / "src").glob("*.rs"))
    if BIN.exists() and BIN.stat().st_mtime >= src:
        return
    r = subprocess.run(["cargo", "build", "--release"], cwd=CRATE, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"cargo build failed:\n{r.stderr[-2000:]}")


def parse_line(text, head="c oneway "):
    lines = [l for l in text.splitlines() if l.startswith(head)]
    if len(lines) != 1:
        raise RuntimeError(f"no single '{head.strip()}' line: {text[-300:]}")
    out = {}
    for kv in lines[0].split()[2:]:
        k, v = kv.split("=", 1)
        out[k] = v
    return out


def measure(path, extra=(), timeout=None):
    """The tool's line for one file, as a dict of strings."""
    if is_holdout(path):
        raise RuntimeError("the 2026 suite is the holdout: not read")
    r = subprocess.run([str(BIN), "measure", str(path), *map(str, extra)], capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError(f"one-way failed (exit {r.returncode}): {r.stderr[-300:]}")
    return parse_line(r.stdout)


def write_cnf(path, n_vars, clauses):
    with open(path, "w") as f:
        f.write(f"p cnf {n_vars} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


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


# ---------------------------------------------------------------- reference

def reference(n_vars, clauses, triples=True):
    """Every pair and every triple of literals, in plain Python.  The numbers
    the Rust tool prints with --all, by the definitions in the docstring."""
    p = Prop(n_vars, clauses)
    if p.root_conflict:
        return {"status": "root-conflict"}
    used = set()
    for c in clauses:
        c = set(c)
        if any(-l in c for l in c) or any(p.value(l) == 1 for l in c):
            continue
        used.update(abs(l) for l in c if p.value(l) == 0)
    active = [l for v in sorted(used) for l in (v, -v)]
    imp = {}
    for l in active:
        got = p.implied([l])
        imp[l] = None if got is None else set(got) - {l}
    n = Counter()
    n["failed"] = sum(1 for l in active if imp[l] is None)
    n["seen"] = sum(len(s) for s in imp.values() if s is not None)
    for x, y in itertools.combinations(active, 2):
        if abs(x) == abs(y):
            continue
        ix, iy = imp[-x], imp[-y]           # what setting x false, y false sets
        if ix is None or iy is None:
            # a side whose own literal fails: the other literal is a unit one probe away
            if iy is not None and x in iy:
                n["hidden_unit"] += 1
            if ix is not None and y in ix:
                n["hidden_unit"] += 1
            continue
        sx, sy = x in iy, y in ix
        if sx and sy:
            n["two_way"] += 1
        elif sx or sy:
            n["one_way"] += 1
        elif p.refutes([-x, -y]):
            n["pair_hidden"] += 1
    # equal variables: h follows v both ways; hidden when h's own side does not see it
    eq_inst = 0
    for v in sorted(used):
        on, off = imp[v], imp[-v]
        if on is None or off is None:
            continue
        for h in on:
            if h in off:
                n["eq_units"] += 1
            elif -h in off and imp[h] is not None and imp[-h] is not None:
                if v in imp[h] and -v in imp[-h]:
                    eq_inst += 1
                else:
                    n["eq_hidden"] += 1
    n["eq_two_way"] = eq_inst / 2
    if triples:
        ok = [l for l in active if imp[-l] is not None]
        fails2 = {}

        def fails(a, b):
            key = (a, b) if a < b else (b, a)
            if key not in fails2:
                fails2[key] = p.refutes([a, b])
            return fails2[key]

        for x, y, z in itertools.combinations(ok, 3):
            if len({abs(x), abs(y), abs(z)}) < 3:
                continue
            if fails(-x, -y) or fails(-x, -z) or fails(-y, -z):
                continue
            sides = 0
            for a, b, c in ((x, y, z), (x, z, y), (y, z, x)):
                got = p.implied([-a, -b])
                sides += c in got
            if sides:
                n[f"t{sides}"] += 1
            elif p.refutes([-x, -y, -z]):
                n["t0"] += 1
    return {"status": "all", "active": len(active) // 2, **{k: n[k] for k in
            ("failed", "seen", "two_way", "one_way", "hidden_unit", "pair_hidden", "eq_two_way", "eq_hidden", "eq_units",
             "t1", "t2", "t3", "t0")}}


def same(ref, got, triples=True):
    """'' when the tool's --all line says what the reference says."""
    if ref["status"] == "root-conflict":
        return "" if got.get("status") == "root-conflict" else f"status {got.get('status')} for a root conflict"
    keys = ["active", "failed", "seen", "two_way", "one_way", "hidden_unit", "pair_hidden", "eq_two_way", "eq_hidden", "eq_units"]
    keys += ["t1", "t2", "t3", "t0"] if triples else []
    bad = [f"{k}: reference {ref[k]}, tool {got.get(k)}" for k in keys
           if got.get(k) in (None, "NA") or abs(float(got[k]) - ref[k]) > 1e-6]
    return "; ".join(bad)


def check_added(n_vars, clauses, closed):
    """'' when `closed` is `clauses` plus clauses of one or two literals that
    each follow by unit propagation from what stands before them."""
    if closed[:len(clauses)] != [list(c) for c in clauses]:
        return "the closed file does not start with the file"
    p = Prop(n_vars, clauses)
    for c in closed[len(clauses):]:
        if not 1 <= len(c) <= 2:
            return f"an added clause is not one or two literals: {c}"
        if not p.refutes([-l for l in c]):
            return f"an added clause does not follow by propagation: {c}"
        p.add(list(c))
        if p.root_conflict:
            break
    return ""


# ----------------------------------------------------------------- selftest

def number_toy():
    """x in 0..3 as sorted bits g1, g2, g3, a count with helpers h1 ('at
    least one'), h2, h3 ('all three'), and T <-> (x = 2), as in the README."""
    g1, g2, g3, h1, h2, h3, t = range(1, 8)
    cl = [[-g2, g1], [-g3, g2]]
    cl += [[-g1, h1], [-g2, h1], [-g3, h1], [g1, g2, g3, -h1]]                       # h1 <-> at least one
    cl += [[-g1, -g2, h2], [-g1, -g3, h2], [-g2, -g3, h2], [g1, g2, -h2], [g1, g3, -h2], [g2, g3, -h2]]
    cl += [[-g1, -g2, -g3, h3], [g1, -h3], [g2, -h3], [g3, -h3]]                     # h3 <-> all three
    cl += [[-t, h2], [-t, -h3], [-h2, h3, t]]                                        # T <-> h2 and not h3
    return 7, cl


def selftest(a):
    build()
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)

        def both(name, n_vars, clauses, expect=None, triples=True):
            f = d / "f.cnf"
            write_cnf(f, n_vars, clauses)
            ref = reference(n_vars, clauses, triples)
            got = measure(f, ["--all"])
            msg = same(ref, got, triples)
            assert not msg, f"{name}: {msg}"
            for k, v in (expect or {}).items():
                assert ref[k] == v, f"{name}: {k} is {ref[k]}, expected {v}"
            return ref

        # by hand
        both("the docstring's example", 4, [[-1, 2], [-1, 3], [-2, -3, 4]], {"one_way": 1, "two_way": 2})
        both("a clause of three", 3, [[1, 2, 3]], {"t3": 1, "t1": 0, "t2": 0, "one_way": 0})
        both("a pair only a probe shows", 3, [[1, 2, 3], [1, 2, -3]], {"pair_hidden": 1})
        both("a root conflict", 1, [[1], [-1]])
        both("units and a tautology", 4, [[1], [-1, 2, 3], [2, -2, 4], [-3, 4]])
        n_vars, cl = number_toy()
        ref = both("the number toy", n_vars, cl, {"one_way": 12, "two_way": 12})
        # the four clauses the number reader adds, "h1 -> g1", "h2 -> g2", "g2 -> h2" and
        # "not h3 -> not g3", are one-way here: each is seen from the side named first
        # below and not from the other.  The other eight one-way clauses follow from
        # these four (such as "T -> g2": T sets h2, and h2 does not set g2).
        p = Prop(n_vars, cl)
        g1, g2, g3, h1, h2, h3, t = range(1, 8)
        for s, m in ((-g1, -h1), (-g2, -h2), (g2, h2), (g3, h3)):
            assert m in p.implied([s]) and -s not in p.implied([-m]), (s, m)
        print(f"toys: ok (the number toy: {ref['two_way']} two-way, {ref['one_way']} one-way, {ref['pair_hidden']} hidden pairs; "
              f"triples seen from 3, 2, 1, 0 sides: {ref['t3']}, {ref['t2']}, {ref['t1']}, {ref['t0']})")
        # closing the number toy: nothing one-way is left, the test sets its bits, every added clause checks
        f, g = d / "f.cnf", d / "g.cnf"
        write_cnf(f, n_vars, cl)
        r = subprocess.run([str(BIN), "close", str(f), str(g)], capture_output=True, text=True)
        assert r.returncode == 0, r.stderr
        info = parse_line(r.stdout, "c oneway-close ")
        _n, closed = read_cnf(g)
        msg = check_added(n_vars, cl, closed)
        assert not msg, msg
        assert len(closed) - len(cl) == int(info["added"]) > 0
        after = reference(n_vars, closed)
        assert after["one_way"] == 0, after
        q = Prop(n_vars, closed)
        assert {g1, g2, -g3} <= set(q.implied([t])), "with the closing clauses the test sets its bits"
        assert not {g1, g2, -g3} <= set(Prop(n_vars, cl).implied([t]))
        print(f"closing the number toy: {info['added']} clauses added, none one-way after, T sets g1, g2, not g3")
        # the equal variables alone: three hidden ones (each helper is its bit), and their missing
        # halves are exactly the four clauses the number reader adds
        assert ref["eq_hidden"] == 3, ref
        r = subprocess.run([str(BIN), "equal", str(f), str(g)], capture_output=True, text=True)
        assert r.returncode == 0, r.stderr
        _n, eq_closed = read_cnf(g)
        msg = check_added(n_vars, cl, eq_closed)
        assert not msg, msg
        four = {frozenset(c) for c in ([-h1, g1], [-h2, g2], [-g2, h2], [h3, -g3])}
        assert {frozenset(c) for c in eq_closed[len(cl):]} == four, eq_closed[len(cl):]
        assert {g1, g2, -g3} <= set(Prop(n_vars, eq_closed).implied([t]))
        print("the equal variables of the number toy: h1 = g1, h2 = g2, h3 = g3, hidden; their missing halves are the "
              "reader's four clauses")
        # sampling with everything sampled is the full count of the two-literal part
        got = measure(f, ["--sources", 10**6, "--targets", 10**6, "--probe-sources", 0])
        assert abs(float(got["one_way"]) - ref["one_way"]) < 1e-6 and abs(float(got["two_way"]) - ref["two_way"]) < 1e-6
        # the holdout is refused, by where the file is
        try:
            measure(HOLDOUT / "no-such-file.cnf")
            raise AssertionError("the holdout was read")
        except RuntimeError as e:
            assert "holdout" in str(e)
        r = subprocess.run([str(BIN), "measure", str(HOLDOUT / "x.cnf")], capture_output=True, text=True)
        assert r.returncode != 0
        # a formula that its own units refute is written out as it is, and said to be so
        write_cnf(f, 2, [[1], [-1], [1, 2]])
        for cmd in ("close", "equal"):
            if g.exists():
                g.unlink()
            r = subprocess.run([str(BIN), cmd, str(f), str(g)], capture_output=True, text=True)
            assert r.returncode == 0 and "status=root-conflict" in r.stdout, (cmd, r.stdout, r.stderr)
            assert read_cnf(g)[1] == [[1], [-1], [1, 2]], cmd
        # a packed file that does not unpack to its end is no formula
        import lzma
        whole = lzma.compress(("p cnf 3 2\n" + "1 2 3 0\n" * 4000).encode())
        cut = d / "cut.cnf.xz"
        cut.write_bytes(whole[:len(whole) // 2])
        r = subprocess.run([str(BIN), "measure", str(cut)], capture_output=True, text=True)
        assert r.returncode != 0 and "c oneway" not in r.stdout, (r.returncode, r.stdout[:200])
        good = d / "good.cnf.xz"
        good.write_bytes(whole)
        assert measure(good, ["--all"])["clauses"] == "4000"
        # a part of the count that the work limit ends before its first look has no estimate:
        # NA, and the report leaves the row out of that part
        chain = [[-v, v + 1] for v in range(1, 1000)] + [[v, -(v + 1)] for v in range(1, 1000)]
        write_cnf(f, 1000, chain)
        got = measure(f, ["--sources", 0, "--probe-sources", 0, "--work", 20000])
        assert got["eq_vars"] == "0" and got["eq_hidden_all"] == "NA" and got["one_way_all"] == "NA", got
        assert not looked(got, "equal") and not looked(got, "pairs")
        got = measure(f, ["--probe-sources", 0, "--sources", 10, "--eq-vars", 5])
        assert got["eq_hidden_all"] == "0" and got["one_way_all"] == "0" and looked(got, "equal") and looked(got, "pairs"), got
        # the report: a row where no near pair met a three-literal clause has no share of them
        n_vars, cl = number_toy()
        write_cnf(f, n_vars, cl)
        got = measure(f, ["--work", 300])
        assert num(got, "t1") + num(got, "t2") + num(got, "t3") == 0, got
        table = d / "t.tsv"
        write_table(table, [{"stem": "toy", **got}])
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            cmd_report(argparse.Namespace(tables=[str(table)], top=5, sort="equal"))
        assert " NA" in out.getvalue(), out.getvalue()
        # a family with no row that measured a part has NA for that part, never 0, and comes last
        got = measure(f, ["--sources", 0, "--probe-sources", 0, "--work", 5])
        assert got["eq_hidden_all"] == "NA", got
        write_table(table, [{"stem": "toy", **got}])
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            cmd_report(argparse.Namespace(tables=[str(table)], top=5, sort="equal"))
        row = [l for l in out.getvalue().splitlines() if l.startswith("?")][0]
        assert "0.0000" not in row and row.count("NA") >= 5, row
        assert "middle NA" in out.getvalue(), out.getvalue()
        # Rust against Python on the Z3 catalog's small records
        import z3_catalog as zc
        recs = [r for r in zc.load(zc.CATALOG) if r["n_vars"] <= a.max_vars]
        if a.records:
            recs = recs[:a.records]
        t0 = time.time()
        tot = Counter()
        for rec in recs:
            cl = [list(c) for c in rec["clauses"]]
            ref = both(rec["id"], rec["n_vars"], cl)
            tot.update({k: v for k, v in ref.items() if isinstance(v, int)})
            # root clauses: one is one-way exactly when some pair is; closing leaves no one-way pair,
            # and every clause it adds follows by propagation from what stands before it
            write_cnf(f, rec["n_vars"], cl)
            got = measure(f, ["--all"])
            assert (int(got["roots"]) > 0) == (ref["one_way"] > 0), (rec["id"], got["roots"], ref["one_way"])
            r = subprocess.run([str(BIN), "close", str(f), str(g)], capture_output=True, text=True)
            assert r.returncode == 0, r.stderr
            info = parse_line(r.stdout, "c oneway-close ")
            assert info["status"] in ("closed", "root-conflict"), (rec["id"], info)
            if info["status"] == "closed":
                _n, closed = read_cnf(g)
                msg = check_added(rec["n_vars"], cl, closed)
                assert not msg, f"{rec['id']}: {msg}"
                after = reference(rec["n_vars"], closed, triples=False)
                assert after["one_way"] == 0, (rec["id"], after)
                tot["roots"] += int(got["roots"])
                tot["added"] += int(info["added"])
                r = subprocess.run([str(BIN), "equal", str(f), str(g)], capture_output=True, text=True)
                assert r.returncode == 0, r.stderr
                einfo = parse_line(r.stdout, "c oneway-equal ")
                _n, eq_closed = read_cnf(g)
                msg = check_added(rec["n_vars"], cl, eq_closed)
                assert not msg, f"{rec['id']} (equal): {msg}"
                assert reference(rec["n_vars"], eq_closed, triples=False)["eq_hidden"] == 0, rec["id"]
                tot["eq_added"] += int(einfo["added"])
        print(f"catalog: {len(recs)} records of at most {a.max_vars} variables, the tool equals the reference on every count "
              f"({tot['two_way']} two-way, {tot['one_way']} one-way, {tot['pair_hidden']} hidden pairs; triples "
              f"{tot['t3']}, {tot['t2']}, {tot['t1']}, {tot['t0']}); {tot['roots']} one-way root clauses, closing adds "
              f"{tot['added']} clauses and leaves no one-way pair on any record; {tot['eq_hidden']} hidden equal variables, their "
              f"missing halves are {tot['eq_added']} clauses ({time.time() - t0:.0f} s)")
    print("selftest: ok")


# ------------------------------------------------------------------- census

def stem_of(path):
    name = Path(path).name
    for s in (".cnf.xz", ".cnf.gz", ".cnf.bz2", ".cnf"):
        if name.endswith(s):
            return name[:-len(s)]
    return name


def _census_job(job):
    path, extra, timeout, core = job
    row = {"stem": stem_of(path), "error": ""}
    t0 = time.time()
    try:
        if is_holdout(path):
            raise RuntimeError("holdout")
        cmd = [str(BIN), "measure", str(path), *map(str, extra)]
        if core is not None:
            cmd = ["taskset", "-c", str(core)] + cmd
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        if r.returncode != 0:
            raise RuntimeError(f"exit {r.returncode}: {r.stderr.strip()[-120:]}")
        row.update(parse_line(r.stdout))
    except subprocess.TimeoutExpired:
        row["error"] = "timeout"
    except RuntimeError as e:
        row["error"] = str(e).replace("\t", " ").replace("\n", " ")
    row["wall_s"] = f"{time.time() - t0:.1f}"
    return row


def cores_of(spec):
    out = []
    for part in spec.split(","):
        if "-" in part:
            x, y = part.split("-")
            out += list(range(int(x), int(y) + 1))
        else:
            out.append(int(part))
    return out


def _init_worker(queue):
    global _CORE
    _CORE = queue.get() if queue is not None else None


def _census_entry(job):
    path, extra, timeout = job
    return _census_job((path, extra, timeout, _CORE))


def read_table(path):
    with open(path, newline="") as fh:
        return list(csv.DictReader(fh, delimiter="\t"))


def write_table(path, rows):
    with open(path, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=COLUMNS, delimiter="\t", lineterminator="\n", extrasaction="ignore")
        w.writeheader()
        for r in sorted(rows, key=lambda r: r["stem"]):
            w.writerow({k: r.get(k, "") for k in COLUMNS})


def cmd_census(a):
    build()
    files = []
    for src in a.paths:
        pth = Path(src)
        files += sorted(pth.glob("*.cnf*")) if pth.is_dir() else [pth]
    if any(is_holdout(f) for f in files):
        sys.exit("the 2026 suite is the holdout: not read (plan section 8)")
    out = Path(a.out)
    done = {r["stem"]: r for r in read_table(out)} if out.exists() else {}
    todo = [f for f in files if stem_of(f) not in done or (a.redo_errors and done[stem_of(f)].get("error"))]
    print(f"{len(files)} files, {len(files) - len(todo)} already in {out.name}, {len(todo)} to do", flush=True)
    extra = ["--seed", a.seed, "--work", a.work, "--sources", a.sources, "--targets", a.targets,
             "--probe-sources", a.probe_sources, "--probe-pairs", a.probe_pairs, "--joint", a.joint, "--eq-vars", a.eq_vars,
             "--far-pairs", a.far_pairs]
    queue = None
    if a.cores:
        cores = cores_of(a.cores)
        if len(cores) < a.jobs:
            sys.exit("fewer cores than jobs")
        queue = multiprocessing.get_context("fork").Queue()
        for c in cores[:a.jobs]:
            queue.put(c)
    rows = dict(done)
    with multiprocessing.get_context("fork").Pool(a.jobs, initializer=_init_worker, initargs=(queue,)) as pool:
        for k, row in enumerate(pool.imap_unordered(_census_entry, [(str(f), extra, a.timeout) for f in todo]), 1):
            rows[row["stem"]] = row
            if k % 20 == 0 or k == len(todo):
                write_table(out, rows.values())
                print(f"{k} of {len(todo)} done", flush=True)
    write_table(out, rows.values())
    errs = Counter(r.get("error") or r.get("status") for r in rows.values())
    print(f"wrote {out}: {len(rows)} rows; {dict(errs)}")


def families():
    fam = {}
    f = ROOT / "benchmarks" / "rl" / "families.tsv"
    with open(f, newline="") as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            if r["suite"] == "sat-comp-2025":
                fam[r["stem"]] = (r["family"], "2025")
    f = ROOT / "benchmarks" / "rl" / "prior_tracks.tsv"
    with open(f, newline="") as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            stem = stem_of(r["file"])
            fam.setdefault(stem, (f"{r['family']} ({r['author'] or '-'})", r["tracks"]))
    return fam


def num(r, k):
    try:
        return float(r.get(k) or 0)
    except ValueError:
        return 0.0


def looked(r, part):
    """A part of the count looked at something in this row: "pairs" (the
    two-literal clauses and the root clauses), "equal" (the equal
    variables), "near" or "far" (pairs of literals).  A part that the work
    limit ended before its first look has no estimate, and a 0 there is no
    measured 0."""
    key = {"pairs": "sources", "equal": "eq_vars", "near": "probe_sources", "far": "far_pairs"}[part]
    return num(r, key) > 0 or num(r, "active") == 0


def cmd_report(a):
    fam = families()
    rows = []
    for t in a.tables:
        rows += read_table(t)
    ok = [r for r in rows if not r.get("error") and r.get("status") in ("sampled", "all-sources", "all")]
    print(f"{len(rows)} rows, {len(ok)} measured; other: "
          f"{dict(Counter((r.get('error') or r.get('status')) for r in rows if r not in ok))}")
    by = defaultdict(list)
    for r in ok:
        by[fam.get(r["stem"], ("?", ""))[0]].append(r)

    def mid(xs):                        # None when there is nothing to take the middle of
        xs = sorted(xs)
        return xs[len(xs) // 2] if xs else None

    def show(x, width, digits):         # a part that no row of the family measured is NA, never 0
        return "NA".rjust(width) if x is None else f"{x:{width}.{digits}f}"

    def share(r):
        # of the two-literal clauses propagation sees, those it sees from one side only
        # (from the counts, so that tables of every version of the tool read alike)
        t = num(r, "two_way") + num(r, "one_way")
        return num(r, "one_way") / t if t else 0.0

    def tside(r):                       # only for a row where near pairs met a three-literal clause
        t = num(r, "t1") + num(r, "t2") + num(r, "t3")
        return (num(r, "t1") + num(r, "t2")) / t

    def met(r):
        return looked(r, "near") and num(r, "t1") + num(r, "t2") + num(r, "t3") > 0

    def eq(r):                          # hidden equal variables a variable
        return num(r, "eq_hidden_all") / max(num(r, "active"), 1)

    def roots(r):
        return num(r, "roots_all") / max(num(r, "active"), 1)

    print(f"\n{'family':40s} files | with a hidden equal variable; such pairs a variable: middle, most | "
          f"of the two-literal clauses propagation sees, the share seen from one side only: middle | "
          f"one-way root clauses a variable: middle | "
          f"three-literal clauses met by near pairs, not seen from every side: middle share")
    table = []
    for name, rs in by.items():
        e = [r for r in rs if looked(r, "equal")]
        pr = [r for r in rs if looked(r, "pairs")]
        nr = [r for r in rs if met(r)]
        have = [r for r in e if num(r, "eq_hidden") > 0]
        table.append({"name": name, "files": len(rs), "have": len(have) if e else None, "eq": mid(eq(r) for r in e),
                      "top": max((eq(r) for r in e), default=None), "share": mid(share(r) for r in pr),
                      "roots": mid(roots(r) for r in pr), "tside": mid(tside(r) for r in nr)})
    by_key = {"equal": "eq", "share": "share", "roots": "roots"}[a.sort]
    # a family with no measured row of the part it is sorted by comes last
    table.sort(key=lambda t: (t[by_key] is None, -(t[by_key] or 0), -(t["top"] or 0), t["name"]))
    for t in table[:a.top]:
        have = "  NA" if t["have"] is None else f"{t['have']:4d}"
        print(f"{t['name'][:40]:40s} {t['files']:5d} | {have}  {show(t['eq'], 8, 4)} {show(t['top'], 8, 4)} | "
              f"{show(t['share'], 7, 4)} | {show(t['roots'], 9, 2)} | {show(t['tside'], 7, 4)}")
    e = [r for r in ok if looked(r, "equal")]
    pr = [r for r in ok if looked(r, "pairs")]
    whole = sorted(share(r) for r in pr)
    have = [r for r in e if num(r, "eq_hidden") > 0]
    fams = sum(1 for rs in by.values() if any(looked(r, "equal") and num(r, "eq_hidden") > 0 for r in rs))
    print(f"\n{len(ok)} files measured; the equal variables were looked at on {len(e)}, the pairs on {len(pr)} "
          f"(on the others the work limit ended that part before its first look).  A hidden equal variable on {len(have)} "
          f"(in {fams} of {len(by)} families), more than one such pair a 100 variables on {sum(eq(r) > 0.01 for r in e)}; "
          f"one-way share in the middle {show(mid(whole), 6, 4).strip()}, above 0.1 on {sum(x > 0.1 for x in whole)}, "
          f"above 0.5 on {sum(x > 0.5 for x in whole)}; a failed literal in the sample on {sum(num(r, 'failed') > 0 for r in pr)}")
    far = [r for r in ok if looked(r, "far") and num(r, "f1") + num(r, "f2") + num(r, "f3") > 0]
    if far:
        fs = sorted((num(r, "f1") + num(r, "f2")) / (num(r, "f1") + num(r, "f2") + num(r, "f3")) for r in far)
        print(f"far pairs with a joint consequence on {len(far)} files; of the three-literal clauses they meet, the share not "
              f"seen from every side: middle {mid(fs):.4f}")


def cmd_one(a):
    build()
    extra = ["--all"] if a.all else []
    extra += a.rest
    print(" ".join(f"{k}={v}" for k, v in measure(a.file, extra).items()))


def cmd_close(a):
    build()
    if is_holdout(a.file):
        sys.exit("the 2026 suite is the holdout: not read (plan section 8)")
    r = subprocess.run([str(BIN), "close", a.file, a.out, "--max-add", str(a.max_add), "--work", str(a.work)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(r.stderr)
    print(r.stdout.strip())


# ------------------------------------------------------------------ pyramid

def cmd_pyramid(a):
    """A Bel's Pyramid cell by kind of variable: placements (the puzzle's
    reader knows them by number), bits of labels (found by the puzzle's
    reader), and the rest.  As the file stands and with the number pass's
    clauses added."""
    build()
    import number_pass_check as npc
    import number_read as nr
    import pyramid_read as pr
    n_vars, _n, clauses, truncated = nr.read_cnf(a.file)
    if truncated:
        sys.exit("too long")
    pyr = pr.Reading([tuple(c) for c in clauses], a.N)
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        kinds = d / "kinds.txt"
        bits = set()
        for seq in pyr.order.values():
            bits.update(abs(v) for v in seq)
        with open(kinds, "w") as fh:
            for v in range(1, pyr.P + 1):
                fh.write(f"{v} placement\n")
            for v in sorted(bits):
                fh.write(f"{v} bit\n")
        plain = d / "f.cnf"
        write_cnf(plain, n_vars, clauses)
        got, report = npc.pass_clauses(a.solver, plain, d)
        linked = d / "g.cnf"
        write_cnf(linked, n_vars, [list(c) for c in clauses] + [list(c) for c in got])
        extra = ["--kinds", kinds, "--seed", a.seed, "--work", a.work, "--sources", a.sources, "--targets", a.targets,
                 "--probe-sources", a.probe_sources]
        for name, f in (("as it is", plain), (f"with the pass's {len(got)} clauses", linked)):
            m = measure(f, extra)
            ks = {k[2:]: float(v) for k, v in m.items() if k.startswith("k:")}
            scale = int(m["active"]) * 2 / max(int(m["sources"]), 1)
            print(f"{Path(a.file).name[:60]} {name}: status={m['status']} one_way_share={m['one_way_share']} "
                  f"one_way_all={m['one_way_all']} two_way_all={m['two_way_all']} pairs={m['pairs']} pair_hidden={m['pair_hidden']} "
                  f"joint_checked={m['joint_checked']} t1={m['t1']} t2={m['t2']} t3={m['t3']} improper={m['improper']} work={m['work']}")
            for k, v in sorted(ks.items(), key=lambda kv: -kv[1]):
                print(f"    one-way, seen from > not seen from {k}: {v * scale:.0f}")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("build")
    s = sub.add_parser("selftest")
    s.add_argument("--max-vars", type=int, default=24)
    s.add_argument("--records", type=int, default=0)
    s = sub.add_parser("one")
    s.add_argument("file")
    s.add_argument("--all", action="store_true")
    s.add_argument("rest", nargs=argparse.REMAINDER)
    s = sub.add_parser("census")
    s.add_argument("out")
    s.add_argument("paths", nargs="+")
    s.add_argument("--jobs", type=int, default=8)
    s.add_argument("--cores", default="")
    s.add_argument("--timeout", type=int, default=3600)
    s.add_argument("--redo-errors", action="store_true")
    for name, default in (("seed", 1), ("work", 4_000_000_000), ("sources", 4_000), ("targets", 8),
                          ("probe-sources", 300), ("probe-pairs", 8), ("joint", 4), ("eq-vars", 400_000), ("far-pairs", 8)):
        s.add_argument(f"--{name}", type=int, default=default)
    s = sub.add_parser("report")
    s.add_argument("tables", nargs="+")
    s.add_argument("--top", type=int, default=40)
    s.add_argument("--sort", choices=["equal", "share", "roots"], default="equal")
    s = sub.add_parser("close")
    s.add_argument("file")
    s.add_argument("out")
    s.add_argument("--max-add", type=int, default=5_000_000)
    s.add_argument("--work", type=int, default=200_000_000_000)
    s = sub.add_parser("pyramid")
    s.add_argument("file")
    s.add_argument("N", type=int)
    s.add_argument("--solver", default=str(ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"))
    for name, default in (("seed", 1), ("work", 20_000_000_000), ("sources", 200_000), ("targets", 8), ("probe-sources", 2_000)):
        s.add_argument(f"--{name}", type=int, default=default)
    a = ap.parse_args(argv)
    {"build": lambda a: build(), "selftest": selftest, "one": cmd_one, "census": cmd_census, "report": cmd_report,
     "close": cmd_close, "pyramid": cmd_pyramid}[a.cmd](a)


if __name__ == "__main__":
    main()
