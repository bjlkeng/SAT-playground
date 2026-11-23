#!/usr/bin/env python3
"""Role files for the solver's count of decisions by role (bead SAT-playground-d96.14.1).

The solver option --rolecount=1 counts what the search decides on, by the role
of the decided variable (solver/13-kissat-rs/src/roles.rs).  The roles come
from a file, one line a variable or a range of variables:

    c a comment
    17 choice
    18-40 helper

This tool writes that file from the free-choice reading (tools/free_census.py),
which gives every variable one role after unit propagation:

  fixed    set by the formula's unit clauses and what follows from them
  choice   a member of an exactly-one or at-most-one group
  number   a bit of a number stored as sorted bits (only with --order)
  test     a variable computed from bits of numbers alone, such as "x = 3 and
           y = 1" (only with --order; a placement of Bel's Pyramid reads so)
  def      any other two-way definition (an AND or OR of any width)
  gate     a small gate: XOR, mux, majority, ...
  helper   a one-way helper (true only where a clause needs it)
  free     nothing above fits

A variable in no clause is left out; the solver calls it `unlisted`.

--name FIRST-LAST=ROLE gives the variables FIRST to LAST a role of the caller's
(it may be given several times, the ranges apart).  It overrides the reading:
the control word of an oisc cell is named this way, and the placement
variables of a pyramid cell.  For each such name the tool prints what the
reading had said of its variables, which is a check of the reading.

A reading takes minutes on a pyramid cell and over an hour on the size-5 one.
`from-reading` writes the same file from a reading saved before
(tools/free_walk.py export, a .fwr file); the selftest holds the two ways to
the same roles.

Usage:
  python3 tools/role_file.py make FILE.cnf[.xz] OUT.roles [--order] [--name A-B=ROLE ...] [--seconds N]
  python3 tools/role_file.py from-reading READING.fwr OUT.roles [--name A-B=ROLE ...]
  python3 tools/role_file.py selftest

The 2026 suite is not read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import re
import sys
import tempfile
import time
from collections import Counter
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))
import free_census as fc  # noqa: E402

NAME = re.compile(r"^[A-Za-z0-9_-]{1,24}$")       # what the solver takes as a role name
RESERVED = ("unlisted", "new")                    # the solver's own two roles
READING_ROLES = ("fixed", "choice", "number", "test", "def", "gate", "helper", "free")


def reading_roles(cnf, order=False, deadline=None):
    """The role of every variable of the formula (index 0 unused; None: in no clause)."""
    nv = cnf.nv
    fixed, rd, _nsat = fc.read_roles(cnf, deadline, order)
    role = [None] * (nv + 1)
    for v in fixed:
        role[v] = "fixed"
    if rd is None:
        return role, None          # refuted by unit propagation: only what was fixed has a role
    red = rd.cnf
    lits, starts = red.lits, red.starts
    # the members of the order groups: the bits of numbers
    bits = {abs(x) for g, mem in enumerate(rd.groups) if rd.ordered[g] for x in mem}
    for v in range(1, min(nv, len(rd.role) - 1) + 1):
        r = rd.role[v]
        if r == fc.CHOICE:
            role[v] = "number" if v in bits else "choice"
        elif r == fc.FREE:
            role[v] = "free"
        elif r == fc.DEF:
            i = rd.acc[v]
            c, x = rd.c_a[i], rd.c_b[i]
            ins = [y for y in lits[starts[c]:starts[c + 1]] if y != x]
            role[v] = "test" if bits and ins and all(abs(y) in bits for y in ins) else "def"
        elif r == fc.GATE:
            role[v] = "gate"
        elif r == fc.HELPER:
            role[v] = "helper"
        elif r == fc.FIXED:
            role[v] = "fixed"
    return role, rd


def saved_roles(path):
    """The roles of a saved reading (the walker's file, tools/free_walk.py reading_array):
    (roles by variable, the formula's variable count)."""
    from array import array
    raw = Path(path).read_bytes()
    if raw[:4] not in (b"FWR1", b"FWR2"):
        raise SystemExit(f"{path}: not a saved reading")
    a = array("i")
    a.frombytes(raw[4:])
    if sys.byteorder != "little":
        a.byteswap()
    pos = 0

    def take(n=1):
        nonlocal pos
        if pos + n > len(a):
            raise SystemExit(f"{path}: the saved reading ends early")
        out = a[pos:pos + n]
        pos += n
        return out

    nv = take()[0]
    role = [None] * (nv + 1)

    def put(v, name):
        v = abs(v)
        if not 1 <= v <= nv or role[v] is not None:
            raise SystemExit(f"{path}: variable {v} has two roles or is not a variable")
        role[v] = name

    for _ in range(take()[0]):
        v, _value = take(2)
        put(v, "fixed")
    for _ in range(take()[0]):
        kind, size = take(2)
        for x in take(size):
            put(x, "number" if kind == 2 else "choice")       # 2: an order group (the bits of a number)
    for v in take(take()[0]):
        put(v, "free")
    for _ in range(take()[0]):
        kind, x, n = take(3)
        if kind == 7:                 # a test on numbers: n triples (group, low, high)
            take(3 * n)
            put(x, "test")
        elif kind == 4:               # -x is the OR of n literals
            take(n)
            put(x, "def")
        elif kind == 5:               # a small gate: n inputs and its truth table
            take(n + 1)
            put(x, "gate")
        elif kind == 6:               # a helper: n clauses, each its length and literals
            for _c in range(n):
                take(take()[0])
            put(x, "helper")
        else:
            raise SystemExit(f"{path}: a node of an unknown kind {kind}")
    return role, nv


def parse_names(specs, nv):
    """[(first, last, role)] of --name FIRST-LAST=ROLE, checked and apart."""
    out = []
    for spec in specs or ():
        m = re.fullmatch(r"(\d+)(?:-(\d+))?=(.+)", spec)
        if not m:
            raise SystemExit(f"--name {spec}: expected FIRST-LAST=ROLE or VARIABLE=ROLE")
        first, last, name = int(m.group(1)), int(m.group(2) or m.group(1)), m.group(3)
        if not NAME.match(name) or name in RESERVED:
            raise SystemExit(f"--name {spec}: '{name}' is not a role name the solver takes")
        if first < 1 or last < first or last > nv:
            raise SystemExit(f"--name {spec}: not a range of the formula's variables (1 to {nv})")
        out.append((first, last, name))
    out.sort()
    for a, b in zip(out, out[1:]):
        if b[0] <= a[1]:
            raise SystemExit(f"--name: the ranges {a[0]}-{a[1]} and {b[0]}-{b[1]} overlap")
    return out


def apply_names(role, names):
    """Give the named ranges their roles; returns, per name, what the reading had said."""
    said = {}
    for first, last, name in names:
        c = said.setdefault(name, Counter())
        for v in range(first, last + 1):
            c[role[v] or "in no clause"] += 1
            role[v] = name
    return said


def write_roles(path, role, header=()):
    """One line a run of variables with the same role; returns the lines written."""
    lines = [f"c {h}" for h in header]
    nv = len(role) - 1
    v = 1
    while v <= nv:
        if role[v] is None:
            v += 1
            continue
        w = v
        while w + 1 <= nv and role[w + 1] == role[v]:
            w += 1
        lines.append(f"{v} {role[v]}" if w == v else f"{v}-{w} {role[v]}")
        v = w + 1
    Path(path).write_text("\n".join(lines) + "\n")
    return len(lines) - len(header)


def read_roles_file(path):
    """The file back as {variable: role}, by the solver's rules (src/roles.rs, parse_roles)."""
    out = {}
    for n, raw in enumerate(Path(path).read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#") or line == "c" or line.startswith(("c ", "c\t")):
            continue
        toks = line.split()
        if len(toks) != 2:
            raise ValueError(f"line {n}: expected '<variable> <role>'")
        rng, name = toks
        a, _, b = rng.partition("-")
        b = b or a
        if not (a.isascii() and a.isdigit() and b.isascii() and b.isdigit()):
            raise ValueError(f"line {n}: '{rng}' is not a variable or a range")
        first, last = int(a), int(b)
        if first < 1 or last < first:
            raise ValueError(f"line {n}: '{rng}' is not a variable or a range")
        if not NAME.match(name) or name in RESERVED:
            raise ValueError(f"line {n}: '{name}' is not a role name")
        for v in range(first, last + 1):
            if v in out:
                raise ValueError(f"line {n}: variable {v} is listed twice")
            out[v] = name
    return out


def cmd_from_reading(args):
    if "sat-comp-2026" in str(Path(args.reading).resolve()):
        raise SystemExit("the 2026 suite is the holdout and is not read")
    role, nv = saved_roles(args.reading)
    names = parse_names(args.name, nv)
    by_reading = Counter(r or "in no clause" for r in role[1:])
    said = apply_names(role, names)
    counts = Counter(r for r in role[1:] if r is not None)
    header = [f"roles of the saved reading {Path(args.reading).name} by tools/role_file.py from-reading"
              + "".join(f" --name {a}-{b}={n}" for a, b, n in names),
              "variables " + " ".join(f"{k}={v}" for k, v in sorted(counts.items()))]
    lines = write_roles(args.out, role, header)
    back = read_roles_file(args.out)
    if back != {v: r for v, r in enumerate(role) if v and r is not None}:
        raise SystemExit(f"{args.out}: the file does not read back as written")
    print(f"file {args.out}")
    print(f"variables {nv} listed {len(back)} lines {lines}")
    print("reading " + " ".join(f"{k}={by_reading[k]}" for k in READING_ROLES + ("in no clause",) if by_reading[k]))
    print("written " + " ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    for name, c in said.items():
        print(f"named {name}: the reading said " + " ".join(f"{k}={v}" for k, v in sorted(c.items(), key=lambda t: -t[1])))
    return 0


def cmd_make(args):
    if "sat-comp-2026" in str(Path(args.file).resolve()):
        raise SystemExit("the 2026 suite is the holdout and is not read")
    t0 = time.time()
    cnf, truncated = fc.read_cnf(args.file)
    if truncated:
        raise SystemExit(f"{args.file}: more clauses than the census reads whole; a role file needs the whole formula")
    names = parse_names(args.name, cnf.nv)
    role, rd = reading_roles(cnf, args.order, t0 + args.seconds)
    if rd is not None:
        t = time.time()
        bad, _false = rd.simulate()          # the reading's own check: a covered clause that comes out false
        if bad:
            raise SystemExit(f"{args.file}: the reading fails its simulation check ({bad} clauses); no role file written")
        check_s = time.time() - t
    else:
        check_s = 0.0
    by_reading = Counter(r or "in no clause" for r in role[1:])
    said = apply_names(role, names)
    counts = Counter(r for r in role[1:] if r is not None)
    header = [f"roles of {Path(args.file).name} by tools/role_file.py"
              + (" --order" if args.order else "")
              + "".join(f" --name {a}-{b}={n}" for a, b, n in names),
              "variables " + " ".join(f"{k}={v}" for k, v in sorted(counts.items()))]
    lines = write_roles(args.out, role, header)
    back = read_roles_file(args.out)
    if back != {v: r for v, r in enumerate(role) if v and r is not None}:
        raise SystemExit(f"{args.out}: the file does not read back as written")
    print(f"file {args.out}")
    print(f"variables {cnf.nv} listed {len(back)} lines {lines} seconds {time.time() - t0:.1f} (check {check_s:.1f})")
    print("reading " + " ".join(f"{k}={by_reading[k]}" for k in READING_ROLES + ("in no clause",) if by_reading[k]))
    print("written " + " ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    for name, c in said.items():
        print(f"named {name}: the reading said " + " ".join(f"{k}={v}" for k, v in sorted(c.items(), key=lambda t: -t[1])))
    if rd is None:
        print("note: unit propagation refutes the formula; only fixed variables have a role")
    return 0


# ---------------------------------------------------------------------------
# selftest
# ---------------------------------------------------------------------------

def _check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'} {name}{(' ' + str(detail)) if detail and not ok else ''}")
    return bool(ok)


def _cnf_of(nv, clauses):
    from array import array
    lits, starts = array("i"), array("q", [0])
    for cl in clauses:
        lits.extend(cl)
        starts.append(len(lits))
    return fc.CNF(nv, lits, starts)


def cmd_selftest(_args):
    ok = True
    # 1. a formula small enough to read by hand:
    #    1-3 exactly one (choice); 4 = AND(1, 5) by three clauses (def); 5 in no group or definition (free);
    #    6 fixed by a unit; 7 in no clause; 8 a one-way helper (8 -> 1)
    clauses = [(1, 2, 3), (-1, -2), (-1, -3), (-2, -3),
               (-4, 1), (-4, 5), (4, -1, -5),
               (6,), (-8, 1), (5, 2, 4)]
    role, rd = reading_roles(_cnf_of(8, clauses))
    ok &= _check("hand formula: choices", role[1:4] == ["choice"] * 3, role)
    ok &= _check("hand formula: fixed", role[6] == "fixed", role)
    ok &= _check("hand formula: in no clause", role[7] is None, role)
    ok &= _check("hand formula: every other variable has a role", all(r in READING_ROLES for r in (role[4], role[5], role[8])), role)
    ok &= _check("hand formula: the reading passes its own check", rd.simulate()[0] == 0)
    # 2. the census's toys: every variable the reading settles gets the matching role
    toys = [("lockchart", fc.toy_lockchart(), False), ("tseitin", fc.toy_tseitin(), False),
            ("csp", fc.toy_csp(), False), ("order", fc.toy_order(), True), ("order, read plain", fc.toy_order(), False)]
    want = {fc.CHOICE: ("choice", "number"), fc.FREE: ("free",), fc.DEF: ("def", "test"), fc.GATE: ("gate",),
            fc.HELPER: ("helper",)}
    for name, toy, order in toys:
        nv, cl = toy[0], toy[1]
        role, rd = reading_roles(_cnf_of(nv, cl), order)
        fixed, rd2, _ = fc.read_roles(_cnf_of(nv, cl), None, order)
        good = rd is not None and rd2 is not None
        if good:
            for v in range(1, nv + 1):
                r = rd2.role[v]
                if v in fixed:
                    good &= role[v] == "fixed"
                elif r in want:
                    good &= role[v] in want[r]
                elif r == fc.ABSENT:
                    good &= role[v] is None
        ok &= _check(f"toy {name}: roles match the reading", good)
        # the same roles from the reading saved as a file
        if good:
            import free_walk as fw
            with tempfile.TemporaryDirectory() as tmp:
                saved = Path(tmp) / "t.fwr"
                fw.write_reading(saved, fw.reading_array(fixed, rd2), bool(rd2.counts))
                again, nv2 = saved_roles(saved)
                ok &= _check(f"toy {name}: a saved reading gives the same roles", nv2 == nv and again == role,
                             [(v, a, b) for v, (a, b) in enumerate(zip(again, role)) if a != b][:5])
        if order:
            n_bits = sum(1 for r in role if r == "number")
            n_test = sum(1 for r in role if r == "test")
            ok &= _check(f"toy {name}: bits of numbers and tests on them are named", n_bits > 0 and n_test > 0, (n_bits, n_test))
        else:
            ok &= _check(f"toy {name}: no number or test without --order", not any(r in ("number", "test") for r in role))
        # 3. the file: written, read back, ranges, names
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "t.roles"
            names = parse_names([f"1-2=first", f"{nv}=last"], nv)
            said = apply_names(role, names)
            lines = write_roles(path, role, ["a header"])
            back = read_roles_file(path)
            ok &= _check(f"toy {name}: the file reads back as written",
                         back == {v: r for v, r in enumerate(role) if v and r is not None})
            ok &= _check(f"toy {name}: names override the reading", back[1] == back[2] == "first" and back[nv] == "last")
            ok &= _check(f"toy {name}: what the reading said of a name is counted",
                         sum(said["first"].values()) == 2 and sum(said["last"].values()) == 1)
            ok &= _check(f"toy {name}: runs of one role are one line", lines <= len(back) and lines < nv)
    # 4. bad names are refused
    for spec in ("0-3=a", "5-2=a", "1-99=a", "1-3=new", "1-3=unlisted", "1-3=a.b", "x=a", "1-3"):
        try:
            parse_names([spec], 10)
            ok &= _check(f"--name {spec} is refused", False)
        except SystemExit:
            ok &= _check(f"--name {spec} is refused", True)
    try:
        parse_names(["1-5=a", "5-7=b"], 10)
        ok &= _check("overlapping names are refused", False)
    except SystemExit:
        ok &= _check("overlapping names are refused", True)
    # 5. a file the solver would refuse is refused here too
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "bad.roles"
        for text in ("1 a\n1 b\n", "2-3 a\n3 b\n", "0 a\n", "4-2 a\n", "1 new\n", "1 a b\n", "+3 a\n", "1 a.b\n"):
            path.write_text(text)
            try:
                read_roles_file(path)
                ok &= _check(f"bad file {text!r} is refused", False)
            except ValueError:
                ok &= _check(f"bad file {text!r} is refused", True)
        path.write_text("c x\n# y\n\n1 a\n3-4 b\n")
        ok &= _check("comments and ranges are read", read_roles_file(path) == {1: "a", 3: "b", 4: "b"})
    print("selftest: all ok" if ok else "selftest: FAILED")
    return 0 if ok else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("make")
    m.add_argument("file")
    m.add_argument("out")
    m.add_argument("--order", action="store_true", help="numbers stored as sorted bits are read too (roles number and test)")
    m.add_argument("--name", action="append", help="FIRST-LAST=ROLE: these variables get this role (overrides the reading)")
    m.add_argument("--seconds", type=int, default=fc.CELL_SECONDS, help="wall budget of the reading")
    f = sub.add_parser("from-reading")
    f.add_argument("reading")
    f.add_argument("out")
    f.add_argument("--name", action="append", help="FIRST-LAST=ROLE: these variables get this role (overrides the reading)")
    sub.add_parser("selftest")
    args = ap.parse_args(argv)
    return {"make": cmd_make, "from-reading": cmd_from_reading, "selftest": cmd_selftest}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
