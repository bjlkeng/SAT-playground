#!/usr/bin/env python3
"""What the solver holds after its own simplification, and whether it has
repaired an encoding by itself.

    python3 tools/live_set.py read CELL.cnf PROOF [--derived FILE --derived-first K]
                                   [--pyramid N] [--added FILE] [--sample 3000]
                                   [--seed 1] [--out HELD.cnf] [--json ROW.json]
    python3 tools/live_set.py snap OUT.tsv JOBS.tsv --scratch DIR
                                   [--points=-1,0,30000,300000] [--jobs 8]
                                   [--cores LIST] [--solver BIN] [--seed 2]
                                   [--args="..."] [--mem-mb 16000]
                                   [--read-mem-mb 48000] [--timeout 3600]
                                   [--sample 3000] [--keep]
    python3 tools/live_set.py report TABLE.tsv...
    python3 tools/live_set.py selftest

The question it answers.  An encoder can write a constraint in a way that
loses propagation (Z3 writes "x = 2" as a count over the sorted bits of x,
and propagation then runs from the bits to the test and not back).  One can
add the missing clauses, or write the constraint afresh.  Before doing
either: does the solver, left alone, get there by itself?  Its simplifier
merges equal variables, eliminates helper variables and rewrites clauses
all through a run, and the search learns clauses.

How.  The solver is run with a conflict limit K and a text proof
(--conflicts=K --no-binary).  A proof lists every clause the solver adds and
every clause it deletes, so the clauses it holds at the moment it stops are
the cell's clauses, plus the added ones, less the deleted ones.  That set
(the held set) is what the search propagates over.  The solver's structure
stage can add clauses of its own before the search; those are not in the
proof but in PROOF.derived.cnf, and the solver says how many they are ("c
structure pass: K derived clauses").  The parity and count engines write
their clauses into the same file later, during the search; those the solver
holds only if it stored them, and then the proof says so.  So the first K
clauses of that file are taken as held from the start (--derived FILE
--derived-first K) and the rest of the file is left out.  `read` rebuilds
the held set and prints:
- how many variables and clauses are held, next to the solver's own last
  report line.  That is the check on the rebuilt set: the copies of
  two-literal clauses in it (`raw_two`) against the solver's count, and
  the copies of longer clauses (`raw_longer`) against its kept clauses plus
  its learned ones.  On the pyramid cells they are the same number.  They
  need not be: a proof may add a clause more often than the solver keeps
  it.  This solver's merging of equal gates does that (when several pairs
  of gates give the same equality "a = b" it writes the two clauses of it
  once for each pair and keeps one copy), so the rebuilt set can still
  hold such an equality after the solver has merged the two variables and
  dropped it.  Those clauses are true, and propagation through them is
  propagation through the variable the solver kept; they make the counts
  too high, by what `report` prints as the difference (seen on 12 of 62
  snapshots of other families: 12 to 25,380 two-literal clauses, 3 to
  6,345 variables).  The numbers under "held" are after one more step,
  which the tests need: one copy of a clause that is there twice, and the
  fixed variables taken out (a clause that a fixed variable satisfies is
  dropped, a false literal is cut);
- with --pyramid N (a Bel's Pyramid cell of size N): what the held variables
  stand for (placements, label bits, other variables of the file, variables
  the solver made itself), and the README's test of the channel: set a
  placement, propagate, and see whether its label bit is set;
- with --added FILE (the cell with implied clauses appended, as `one-way
  equal` or `number_read.py` writes it): for the appended two-literal
  clauses "a or b", whether the held set sees them from both sides (a off
  sets b, and b off sets a).

A variable with no clause left was either merged into another (the solver
keeps one of each set of equal variables) or eliminated.  When the solver
merges a variable into another it first writes the two clauses of that
equality into the proof ("a or not b", "not a or b").  So for the tests a
variable with no clause left is followed along the equalities that the file
or the proof gave for it, the one given last first, to a variable that is
held; where none is reached, the case is counted apart as eliminated.

Not every line of a proof is a clause the solver stores.  The control
split (on by default; it runs on the cells where the solver reads a
control set, the four oisc cells of 2025) asks its cases of a solver of
their own and writes what that solver learns into the proof, so that the
proof holds; the main solver does not keep those clauses.  A snapshot of
such a run would be of a stronger formula than the search works on, so
`snap` gives it an ERROR row.  Take it with the split off
(--args=--ctlsplit=0) if a snapshot of that cell is wanted, and do not use
`read` on the proof of a run in which the split ran.

`snap` does this for many cells and several moments K (-1 is the file as
written, with no run).  JOBS.tsv has one line a cell: NAME, CELL.cnf, the
pyramid's size or 0, ADDED.cnf or "-", separated by tabs.  One row a cell
and moment goes to OUT.tsv; a cell and moment that has a result there is
not run again, and one that has an ERROR or TIMEOUT row is tried again when
the call asks for it (a failed row that the call does not ask for stays,
and the call then ends with exit code 1).  Each solver runs in a process group of its own under a memory limit
and a time limit; a run that does not end as its answer says (UNKNOWN at
the conflict limit with exit code 0, SATISFIABLE with 10, UNSATISFIABLE with
20) gets an ERROR row and no snapshot.  --jobs is the number of solver
processes at a time: keep it at 32 or below on this host.

The 2026 holdout is refused.
"""
from __future__ import annotations

import argparse
import csv
import heapq
import json
import lzma
import os
import random
import re
import shlex
import signal
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cnf_prop  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SOLVER = ROOT / "solver/13-kissat-rs/target/release/sat-solver"
KILL_GRACE_S = 5.0
EXIT_OF = {"UNKNOWN": 0, "SATISFIABLE": 10, "UNSATISFIABLE": 20}

FIELDS = ["cell", "point", "pyramid", "with_added", "status", "conflicts", "work", "solver_seconds", "read_seconds",
          "file_vars", "file_clauses", "proof_adds", "proof_deletes", "unmatched_deletes", "refuted",
          "held_vars", "held_clauses", "held_two", "held_three", "held_longer", "fixed", "made_by_solver",
          "raw_two", "raw_longer", "solver_vars", "solver_two", "solver_longer", "solver_learned",
          "placements", "label_bits", "stand_placement", "stand_bit", "stand_other", "stand_solver",
          "links", "sampled", "link_set", "link_by_clause", "link_unset", "placement_off", "placement_gone",
          "bit_gone",
          "added_two", "added_sampled", "added_both", "added_one", "added_none", "added_true", "added_gone"]


# ---------------------------------------------------------------- reading

def refuse_holdout(path) -> None:
    if "sat-comp-2026" in Path(path).resolve().parts:
        sys.exit(f"{path}: the 2026 holdout is not read by this tool")


def _open(path):
    with open(path, "rb") as fh:
        xz = fh.read(6) == b"\xfd7zXZ\x00"
    return (lzma.open if xz else open)(path, "rt", encoding="ascii", errors="strict")


def read_cnf(path):
    """(variables, clauses as written).  Every clause is kept as it stands,
    an empty one and one with a repeated literal too: the proof names
    clauses as the solver read them.  Reading stops at a line that starts
    with '%' (the end mark of old files)."""
    refuse_holdout(path)
    n, clauses, cur = 0, [], []
    with _open(path) as f:
        for line in f:
            if not line.strip() or line[0] == "c":
                continue
            if line[0] == "%":
                break
            if line[0] == "p":
                parts = line.split()
                if len(parts) < 4 or parts[1] != "cnf":
                    sys.exit(f"{path}: not a 'p cnf' header: {line.strip()!r}")
                n = int(parts[2])
                continue
            for tok in line.split():
                if tok == "0":
                    clauses.append(cur)
                    cur = []
                else:
                    cur.append(int(tok))
    if cur:
        sys.exit(f"{path}: the last clause has no closing 0")
    for c in clauses:
        for l in c:
            if abs(l) > n:
                n = abs(l)
    return n, clauses


def key(lits) -> tuple:
    """A clause as a set: the same clause whatever the order of its literals."""
    return tuple(sorted(set(lits)))


class ProofError(Exception):
    pass


def apply_proof(live: Counter, path, on_add=None) -> dict:
    """Apply a text DRAT proof to the clauses in `live` (clause -> how many
    copies are held): a line "d l1 l2 ... 0" takes one copy away, any other
    line "l1 l2 ... 0" adds one.  A deletion of a clause that is not held is
    counted and otherwise ignored.  A line that is not of that form (a binary
    proof, a line cut short) raises ProofError."""
    adds = dels = unmatched = 0
    top = 0
    try:
        with open(path, "rt", encoding="ascii", errors="strict") as f:
            for no, line in enumerate(f, 1):
                t = line.split()
                if not t:
                    continue
                delete = t[0] == "d"
                if delete:
                    t = t[1:]
                if not line.endswith("\n") or not t or t[-1] != "0":
                    raise ProofError(f"{path}: line {no} is not a clause closed by 0 (a proof cut short, or not a text proof)")
                try:
                    lits = [int(x) for x in t[:-1]]
                except ValueError:
                    raise ProofError(f"{path}: line {no} is not a list of literals (write the proof with --no-binary)") from None
                if 0 in lits:
                    raise ProofError(f"{path}: line {no} has a 0 inside a clause")
                k = key(lits)
                if delete:
                    if live.get(k, 0) > 0:
                        live[k] -= 1
                        dels += 1
                    else:
                        unmatched += 1
                else:
                    live[k] += 1
                    adds += 1
                    if on_add is not None:
                        on_add(k)
                    for l in k:
                        if abs(l) > top:
                            top = abs(l)
    except UnicodeDecodeError:
        raise ProofError(f"{path}: not a text proof (write it with --no-binary)") from None
    return {"proof_adds": adds, "proof_deletes": dels, "unmatched_deletes": unmatched, "top": top}


class Equalities:
    """Every equality between two variables that the file or the proof gave,
    with the moment it was last given.

    Two clauses "x or y" and "not x or not y" say that x is "not y".  Every
    time a two-literal clause arrives whose mirror was seen before, the two
    variables are noted as equal (up to sign), at that moment.  The solver
    writes such a pair for a variable just before it merges it into another
    and never names the variable again, so for a merged variable the
    equality given last is the one with the variable it was merged into.
    Earlier equalities are kept as well: a variable can also go because the
    solver eliminated it through an equality that was there all along."""

    def __init__(self):
        self.seen = set()            # every two-literal clause seen so far
        self.links = {}              # variable -> {literal its positive literal equals: the moment last given}
        self.clock = 0

    def note(self, k: tuple) -> None:
        if len(k) != 2 or k[0] == -k[1]:
            return
        x, y = k
        if (-y, -x) in self.seen or (-x, -y) in self.seen:      # with "x or y": x is "not y"
            self.clock += 1
            self.links.setdefault(abs(x), {})[-y if x > 0 else y] = self.clock
            self.links.setdefault(abs(y), {})[-x if y > 0 else x] = self.clock
        self.seen.add(k)

    def done(self) -> None:
        self.seen = set()            # only the equalities are needed from here on


class Held:
    """The clauses the solver holds, with what is fixed taken out, and the
    means to test them: propagation, and a held stand-in for a variable that
    has no clause left."""

    def __init__(self, input_vars: set, top: int, live: Counter, equal: dict):
        """input_vars: the variables the file's clauses use (a variable that
        is held and not among them was made by the solver; the header may
        count more variables than the clauses use, and the solver numbers
        its own from the largest one used); top: the largest variable the
        file or the proof names."""
        clauses = [list(k) for k, c in live.items() if c > 0]
        # copies as the solver counts them, before anything is taken out
        self.raw_two = sum(c for k, c in live.items() if c > 0 and len(k) == 2)
        self.raw_longer = sum(c for k, c in live.items() if c > 0 and len(k) > 2)
        self.input_vars = input_vars
        self.top = top
        self.p = cnf_prop.Prop(top, clauses)
        self.refuted = bool(self.p.root_conflict)
        p = self.p
        self.fixed = 0 if self.refuted else sum(1 for v in range(1, top + 1) if p.value(v) != 0)
        self.left = []
        self.occurs = set()
        self.held_bin = set()
        if not self.refuted:
            for c in clauses:
                if any(p.value(l) == 1 for l in c):
                    continue
                c = [l for l in c if p.value(l) == 0]
                self.left.append(c)
                for l in c:
                    self.occurs.add(abs(l))
                if len(c) == 2:
                    self.held_bin.add(key(c))
        self.equal = equal           # variable -> {the literal it equals: the moment last given}
        self.stands = {}             # what stand_in found, by literal

    def known(self, l: int) -> bool:
        return abs(l) in self.occurs or self.p.value(l) != 0

    def stand_in(self, l: int):
        """l when its variable is held or fixed; else a held or fixed
        literal that is equal to it; else None (the variable is eliminated
        with nothing held that equals it).  The equalities are followed
        from l, the one given last first, so that a merged variable leads
        to the variable the solver merged it into before any other."""
        if self.known(l):
            return l
        if l in self.stands:
            return self.stands[l]
        found = None
        seen = {abs(l)}
        heap = []                    # (-moment, a literal equal to l)
        cur = l
        while True:
            for m, t in self.equal.get(abs(cur), {}).items():
                if abs(m) not in seen:
                    heapq.heappush(heap, (-t, m if cur > 0 else -m))
            while heap and abs(heap[0][1]) in seen:
                heapq.heappop(heap)
            if not heap:
                break
            cur = heapq.heappop(heap)[1]
            seen.add(abs(cur))
            if self.known(cur):
                found = cur
                break
        self.stands[l] = found
        self.stands[-l] = -found if found is not None else None
        return found

    def sets(self, a: int, b: int) -> bool:
        """With a set, does propagation set b, or run into a conflict?"""
        p = self.p
        if p.value(b) == 1 or p.value(a) == -1:
            return True
        got = p.implied([a])
        return got is None or b in got

    def counts(self) -> dict:
        sizes = Counter(min(len(c), 4) for c in self.left)
        return {"refuted": int(self.refuted), "held_vars": len(self.occurs), "held_clauses": len(self.left),
                "held_two": sizes[2], "held_three": sizes[3], "held_longer": sizes[4], "fixed": self.fixed,
                "made_by_solver": sum(1 for v in self.occurs if v not in self.input_vars),
                "raw_two": self.raw_two, "raw_longer": self.raw_longer}


def pyramid_test(h: Held, orig: list, n: int, sample: int, rng) -> dict:
    import pyramid_read
    reading = pyramid_read.Reading([list(dict.fromkeys(c)) for c in orig if c], n)
    P = reading.P
    bit_vars = {abs(l) for seq in reading.order.values() for l in seq}
    links, _counts = reading.implied({"fwd"})
    # the held variables that a placement, or a label bit, was merged into
    for_placement, for_bit = set(), set()
    for group, into in ((range(1, P + 1), for_placement), (bit_vars, for_bit)):
        for v in group:
            m = h.stand_in(v)
            if m is not None and abs(m) in h.occurs:
                into.add(abs(m))
    stands = Counter()
    for v in h.occurs:
        if v in for_placement:
            stands["placement"] += 1
        elif v in for_bit:
            stands["bit"] += 1
        elif v not in h.input_vars:
            stands["solver"] += 1
        else:
            stands["other"] += 1
    smp = links if sample <= 0 or sample >= len(links) else rng.sample(links, sample)
    off = gone_p = gone_b = set_ = unset = by_clause = 0
    for neg_p, lit in smp:
        pl, bl = h.stand_in(-neg_p), h.stand_in(lit)
        # first what is settled for good, whatever became of the other
        # variable: the placement ruled out, or the bit fixed as the link wants
        if pl is not None and h.p.value(pl) == -1:
            off += 1
        elif bl is not None and h.p.value(bl) == 1:
            set_ += 1
            by_clause += 1
        elif pl is None:
            gone_p += 1
        elif bl is None:
            gone_b += 1
        elif h.sets(pl, bl):
            set_ += 1
            by_clause += key((-pl, bl)) in h.held_bin
        else:
            unset += 1
    return {"placements": P, "label_bits": len(bit_vars), "stand_placement": stands["placement"],
            "stand_bit": stands["bit"], "stand_other": stands["other"], "stand_solver": stands["solver"],
            "links": len(links), "sampled": len(smp), "link_set": set_, "link_by_clause": by_clause,
            "link_unset": unset, "placement_off": off, "placement_gone": gone_p, "bit_gone": gone_b}


def appended(orig: list, with_added: list) -> list:
    """The two-literal clauses of `with_added` that are not clauses of the
    cell (as sets, counted with their copies)."""
    base = Counter(key(c) for c in orig)
    extra = []
    for c in with_added:
        k = key(c)
        if base.get(k, 0) > 0:
            base[k] -= 1
        elif len(k) == 2 and k[0] != -k[1]:
            extra.append(k)
    return extra


def added_test(h: Held, extra: list, sample: int, rng) -> dict:
    smp = extra if sample <= 0 or sample >= len(extra) else rng.sample(extra, sample)
    both = one = none = gone = true_ = 0
    for a, b in smp:
        sa, sb = h.stand_in(a), h.stand_in(b)
        # a literal that is fixed true settles the clause, whatever became of the other variable
        if (sa is not None and h.p.value(sa) == 1) or (sb is not None and h.p.value(sb) == 1):
            true_ += 1
        elif sa is None or sb is None:
            gone += 1
        else:
            n = h.sets(-sa, sb) + h.sets(-sb, sa)
            both += n == 2
            one += n == 1
            none += n == 0
    return {"added_two": len(extra), "added_sampled": len(smp), "added_both": both, "added_one": one,
            "added_none": none, "added_true": true_, "added_gone": gone}


def analyse(cell, proof, derived=None, derived_first=0, pyramid=0, added=None, sample=3000, seed=1, out=None) -> dict:
    """The held set of `cell` at the end of `proof`, counted and tested.
    `derived` with `derived_first`: the first so many clauses of that file
    are the structure stage's and are held from the start; what follows in
    the file are clauses the engines proved during the search, which are in
    the proof where the solver stored them."""
    nv, orig = read_cnf(cell)
    res = {"file_vars": nv, "file_clauses": len(orig)}
    live = Counter()
    eq = Equalities()
    input_vars = set()
    for c in orig:
        k = key(c)
        live[k] += 1
        eq.note(k)
        for l in k:
            input_vars.add(abs(l))
    top = nv
    if derived and derived_first > 0:
        _n, extra = read_cnf(derived)
        if len(extra) < derived_first:
            raise ProofError(f"{derived}: {len(extra)} clauses, fewer than the {derived_first} the structure stage added")
        for c in extra[:derived_first]:
            k = key(c)
            live[k] += 1
            eq.note(k)
            for l in k:
                top = max(top, abs(l))
    st = apply_proof(live, proof, eq.note)
    top = max(top, st.pop("top"))
    res.update(st)
    eq.done()
    h = Held(input_vars, top, live, eq.links)
    del live
    res.update(h.counts())
    rng = random.Random(seed)
    if not h.refuted:
        if pyramid:
            res.update(pyramid_test(h, orig, pyramid, sample, rng))
        if added:
            _n, with_added = read_cnf(added)
            res.update(added_test(h, appended(orig, with_added), sample, rng))
    if out:
        with open(out, "w") as f:
            if h.refuted:               # the held clauses refute the formula: say so with the empty clause
                f.write(f"p cnf {h.top} 1\n0\n")
            else:
                f.write(f"p cnf {h.top} {len(h.left)}\n")
                for c in h.left:
                    f.write(" ".join(map(str, c)) + " 0\n")
    return res


def show(r: dict) -> None:
    def g(k):
        return r.get(k, "")
    print(f"{g('cell') or 'cell'}: {g('file_vars')} variables, {g('file_clauses')} clauses in the file; the proof adds "
          f"{g('proof_adds')}, deletes {g('proof_deletes')} ({g('unmatched_deletes')} deletions of a clause not held)")
    if str(g("refuted")) == "1":
        print("  the held clauses refute the formula by propagation: nothing to test")
        return
    print(f"  held: {g('held_clauses')} clauses ({g('held_two')} of two literals, {g('held_three')} of three, "
          f"{g('held_longer')} longer), {g('held_vars')} variables in them ({g('made_by_solver')} made by the solver); "
          f"{g('fixed')} variables fixed")
    print(f"  copies before the fixed variables are taken out: {g('raw_two')} clauses of two literals, {g('raw_longer')} longer")
    if g("solver_vars") != "":
        print(f"  the solver's own last report line: {g('solver_vars')} variables, {g('solver_two')} two-literal clauses, "
              f"{g('solver_longer')} longer kept and {g('solver_learned')} learned")
    if g("placements") != "":
        print(f"  the held variables stand for (one that is equal to a placement counts as a placement): placements "
              f"{g('stand_placement')}, label bits {g('stand_bit')}, other variables of the file {g('stand_other')}, "
              f"made by the solver {g('stand_solver')}; the file has {g('placements')} placements and {g('label_bits')} label bits")
        print(f"  a placement sets its label bit ({g('sampled')} sampled of {g('links')} links): set {g('link_set')} "
              f"({g('link_by_clause')} by a held two-literal clause or a fixed bit), not set {g('link_unset')}; "
              f"placement already ruled out {g('placement_off')}; placement eliminated {g('placement_gone')}; "
              f"bit eliminated {g('bit_gone')}")
    if g("added_two") != "":
        print(f"  the appended two-literal clauses ({g('added_sampled')} sampled of {g('added_two')}): seen from both sides "
              f"{g('added_both')}, from one {g('added_one')}, from none {g('added_none')}; one literal true already "
              f"{g('added_true')}; a variable eliminated {g('added_gone')}")


def cmd_read(a) -> int:
    try:
        r = analyse(a.cell, a.proof, a.derived, a.derived_first, a.pyramid, a.added, a.sample, a.seed, a.out)
    except ProofError as e:
        print(e, file=sys.stderr)
        return 1
    r["cell"] = Path(a.cell).name
    show(r)
    if a.json:
        Path(a.json).write_text(json.dumps(r) + "\n")
    return 0


# ---------------------------------------------------------------- running

# every process started and not yet waited for, so that a signal to this
# tool can stop them all (each is in a session of its own)
ACTIVE: set = set()
ACTIVE_LOCK = threading.Lock()
STOPPING = False


def stop_all(signum, _frame) -> None:
    """The tool is being stopped: stop every group it started, then exit."""
    global STOPPING
    with ACTIVE_LOCK:
        STOPPING = True
        procs = list(ACTIVE)
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for proc in procs:
            try:
                os.killpg(proc.p.pid, sig)
            except ProcessLookupError:
                pass
        end = time.time() + KILL_GRACE_S
        while time.time() < end:
            alive = False
            for proc in procs:
                proc.p.poll()
                try:
                    os.killpg(proc.p.pid, 0)
                    alive = True
                except ProcessLookupError:
                    pass
            if not alive:
                break
            time.sleep(0.05)
    try:
        sys.stdout.flush()
    finally:
        os._exit(128 + signum)


class Proc:
    """One command in a process group of its own, under a memory limit, with
    its output in a file.  `wait` returns (exit code, seconds, timed out); at
    the time limit the whole group is stopped before `wait` returns."""

    def __init__(self, cmd: list, out: str, mem_mb, env=None):
        inner = "exec " + " ".join(shlex.quote(str(x)) for x in cmd)
        if mem_mb:
            inner = f"ulimit -v {int(mem_mb) * 1024}; " + inner
        self.out = open(out, "w")
        with ACTIVE_LOCK:
            if STOPPING:
                self.out.close()
                raise RuntimeError("the tool is being stopped")
            self.start = time.time()
            self.p = subprocess.Popen(["bash", "-c", inner], stdout=self.out, stderr=subprocess.STDOUT, env=env,
                                      stdin=subprocess.DEVNULL, start_new_session=True)
            ACTIVE.add(self)

    def stop_group(self) -> None:
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(self.p.pid, sig)
            except ProcessLookupError:
                return
            end = time.time() + KILL_GRACE_S
            while time.time() < end:
                self.p.poll()               # reap the leader: a zombie keeps its group alive
                try:
                    os.killpg(self.p.pid, 0)
                except ProcessLookupError:
                    return
                time.sleep(0.02)

    def wait(self, timeout: float) -> tuple:
        end = self.start + timeout
        timed_out = False
        while True:
            # look without reaping: while the leader is running or a zombie,
            # the group's number is its own and cannot go to another process
            try:
                over = os.waitid(os.P_PID, self.p.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None
            except ChildProcessError:
                over = True
            if over:
                break
            if time.time() >= end:
                timed_out = True
                break
            time.sleep(0.02)
        seconds = time.time() - self.start
        # stop whatever is left in the group: everything at the time limit,
        # and after a normal end a child that outlived the leader
        self.stop_group()
        self.p.wait()
        with ACTIVE_LOCK:
            ACTIVE.discard(self)
        self.out.close()
        return self.p.returncode, seconds, timed_out


def parse_cores(text: str) -> list:
    cores = []
    for part in text.split(","):
        part = part.strip()
        if not part:
            continue
        if "-" in part:
            lo, hi = part.split("-", 1)
            cores.extend(range(int(lo), int(hi) + 1))
        else:
            cores.append(int(part))
    if len(set(cores)) != len(cores):
        sys.exit(f"--cores {text}: a core is named twice")
    return cores


# a report line: c, a letter, seconds, MB, level, switched, reductions, restarts, rate, conflicts, learned
# clauses held, size/glue, size, glue, tier1, tier2, trail%, two-literal clauses, longer kept, variables, left%
REPORT = re.compile(r"^c \S +\d+\.\d+ +\d+ +\d+ +\d+ +\d+ +\d+ +\d+ +\d+ +(\d+) .* (\d+) +(\d+) +(\d+) +\d+%\s*$")
CONFLICTS = re.compile(r"^c conflicts:\s+(\d+)\b", re.M)
STAGE = re.compile(r"^c structure pass: (\d+) derived clauses? \(", re.M)
SPLIT = re.compile(r"^c control ctlsplit=\d+ status=(\S+) cases=(\d+)\b", re.M)
# the control split's ways of saying that it did not run its cases
SPLIT_DID_NOT_RUN = ("too-big", "no-formula", "walk-model", "no-vars")
WORK = re.compile(r"^c workclock .*\bwork=(\d+)\b", re.M)
STATUS = re.compile(r"^s (\w+)\s*$", re.M)


def read_solver_output(text: str) -> dict:
    """What the solver said: its answer, the conflicts and work it reached,
    how many clauses its structure stage added, whether the control split
    ran its cases, and its last report line (learned clauses held,
    two-literal clauses, longer clauses kept, variables)."""
    d = {}
    answers = STATUS.findall(text)
    if len(answers) == 1:
        d["answer"] = answers[0]
    m = CONFLICTS.search(text)
    if m:
        d["conflicts"] = int(m.group(1))
    m = WORK.search(text)
    if m:
        d["work"] = int(m.group(1))
    split = SPLIT.findall(text)
    if split:
        status, cases = split[-1]
        if status not in SPLIT_DID_NOT_RUN or int(cases) > 0:
            d["split"] = f"status {status}, {cases} cases"
    stage = STAGE.findall(text)
    if len(stage) > 1:
        d["stage_clauses"] = None       # more than one such line: not understood
    else:
        d["stage_clauses"] = int(stage[0]) if stage else 0
    for line in text.splitlines():
        m = REPORT.match(line)
        if m:
            d["solver_learned"], d["solver_two"], d["solver_longer"], d["solver_vars"] = (int(m.group(i)) for i in (1, 2, 3, 4))
    return d


def child_env() -> dict:
    """The environment of a solver run: without the work and time limits a
    harness may have set, so that a run that stops with UNKNOWN stopped at
    the conflict limit."""
    return {k: v for k, v in os.environ.items() if not k.startswith("SAT_LIMIT")}


def snapshot(idx: int, job: tuple, point: int, a, core) -> dict:
    name, cell, pyramid, added = job
    row = dict.fromkeys(FIELDS, "")
    row.update(cell=name, point=str(point), pyramid=str(pyramid), with_added=str(int(added != "-")))
    base = os.path.join(a.scratch, f"{idx:04d}-{name}.c{point}")
    proof, log, js = base + ".proof", base + ".out", base + ".json"
    pin = ["taskset", "-c", str(core)] if core is not None else []
    made = [proof, proof + ".derived.cnf", proof + ".pbp", js, base + ".read.out"]
    try:
        derived, derived_first = None, 0
        if point < 0:
            Path(proof).write_text("")
            Path(log).write_text("")
            row["status"] = "FILE"
        else:
            cmd = pin + [a.solver, f"--seed={a.seed}", f"--conflicts={point}", "--no-binary"] + a.solver_args + [cell, proof]
            code, seconds, timed_out = Proc(cmd, log, a.mem_mb, child_env()).wait(a.timeout)
            said = read_solver_output(Path(log).read_text(errors="replace"))
            row["solver_seconds"] = f"{seconds:.1f}"
            for k in ("conflicts", "work", "solver_two", "solver_longer", "solver_learned", "solver_vars"):
                if k in said:
                    row[k] = str(said[k])
            answer = said.get("answer")
            if timed_out:
                row["status"] = "TIMEOUT"
                return row
            if answer not in EXIT_OF or EXIT_OF[answer] != code:
                row["status"] = f"ERROR(exit {code}, " + (f"s {answer}" if answer else "no single answer") + ")"
                return row
            if answer == "UNKNOWN" and said.get("conflicts", -1) < point:
                row["status"] = f"ERROR(UNKNOWN after {said.get('conflicts', 'no count of')} conflicts, before the limit)"
                return row
            if "split" in said:
                row["status"] = (f"ERROR(the control split ran ({said['split']}): its case solvers write clauses into the proof "
                                 f"that the main solver does not hold; take the snapshot with --args=--ctlsplit=0)")
                return row
            if said["stage_clauses"] is None:
                row["status"] = "ERROR(more than one line 'structure pass: K derived clauses')"
                return row
            if said["stage_clauses"] > 0:
                derived, derived_first = proof + ".derived.cnf", said["stage_clauses"]
                if not os.path.exists(derived):
                    row["status"] = f"ERROR(the structure stage added {derived_first} clauses and wrote no {os.path.basename(derived)})"
                    return row
            row["status"] = answer
        cmd = pin + [sys.executable, os.path.abspath(__file__), "read", cell, proof, "--json", js,
                     "--sample", str(a.sample), "--seed", "1"]
        if derived:
            cmd += ["--derived", derived, "--derived-first", str(derived_first)]
        if pyramid:
            cmd += ["--pyramid", str(pyramid)]
        if added != "-":
            cmd += ["--added", added]
        code, seconds, timed_out = Proc(cmd, base + ".read.out", a.read_mem_mb).wait(a.timeout)
        row["read_seconds"] = f"{seconds:.1f}"
        if timed_out or code != 0 or not os.path.exists(js):
            row["status"] = f"ERROR(the proof was not read: {'time limit' if timed_out else f'exit {code}'})"
            return row
        got = json.loads(Path(js).read_text())
        for k, v in got.items():
            if k in row and k != "cell":
                row[k] = str(v)
        return row
    finally:
        if not a.keep:
            for f in made + [log]:
                try:
                    os.unlink(f)
                except FileNotFoundError:
                    pass


def read_jobs(path: str) -> list:
    jobs, seen = [], set()
    for no, line in enumerate(Path(path).read_text().splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        t = line.split("\t")
        if len(t) != 4:
            sys.exit(f"{path}: line {no} has {len(t)} fields, not NAME, CELL, PYRAMID, ADDED")
        name, cell, pyramid, added = t
        if not re.fullmatch(r"[A-Za-z0-9._+-]+", name):
            sys.exit(f"{path}: line {no}: the name {name!r} must be made of letters, digits and . _ + -")
        if name in seen:
            sys.exit(f"{path}: the name {name} is there twice")
        seen.add(name)
        for f in (cell,) if added == "-" else (cell, added):
            refuse_holdout(f)
            if not os.path.isfile(f):
                sys.exit(f"{path}: line {no}: no file {f}")
        jobs.append((name, cell, int(pyramid), added))
    return jobs


def failed(status: str) -> bool:
    return status.startswith(("ERROR", "TIMEOUT"))


def done_rows(path: str, wanted: set) -> tuple:
    """(the cells and moments that have a result, how many rows without one
    are tried again, how many rows without one stay).  `wanted` are the
    cells and moments this call asks for.  A row without a result that this
    call asks for is taken out of the table and tried again; one that it
    does not ask for stays where it is, so that a failure is never dropped
    without a result in its place."""
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        return set(), 0, 0
    with open(path, newline="") as f:
        reader = csv.DictReader(f, delimiter="\t")
        if reader.fieldnames != FIELDS:
            sys.exit(f"{path}: not a table of this tool (other columns); give a new file")
        rows = list(reader)
    again = [r for r in rows if failed(r["status"]) and (r["cell"], r["point"]) in wanted]
    keep = [r for r in rows if not (failed(r["status"]) and (r["cell"], r["point"]) in wanted)]
    if again:
        tmp = path + ".tmp"
        with open(tmp, "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=FIELDS, delimiter="\t", lineterminator="\n")
            w.writeheader()
            w.writerows(keep)
        os.replace(tmp, path)
    return ({(r["cell"], r["point"]) for r in keep if not failed(r["status"])}, len(again),
            sum(1 for r in keep if failed(r["status"])))


def cmd_snap(a) -> int:
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, stop_all)
    a.solver = str(Path(a.solver).resolve())
    if not os.access(a.solver, os.X_OK):
        sys.exit(f"{a.solver}: no solver to run")
    a.solver_args = shlex.split(a.args)
    points = [int(x) for x in a.points.split(",") if x.strip()]
    if any(p < -1 for p in points) or len(set(points)) != len(points):
        sys.exit(f"--points {a.points}: each moment once, -1 (the file as written) or a conflict count")
    cores = parse_cores(a.cores) if a.cores else []
    if cores and len(cores) < a.jobs:
        sys.exit(f"--cores names {len(cores)} cores for --jobs {a.jobs}")
    if a.jobs < 1:
        sys.exit("--jobs must be at least 1")
    jobs = read_jobs(a.jobs_file)
    os.makedirs(a.scratch, exist_ok=True)
    wanted = {(job[0], str(pt)) for job in jobs for pt in points}
    done, retried, stay = done_rows(a.out, wanted)
    if retried:
        print(f"{retried} rows without a result are tried again", flush=True)
    tasks = [(i, job, pt) for i, job in enumerate(jobs) for pt in points if (job[0], str(pt)) not in done]
    tasks.sort(key=lambda t: (-os.path.getsize(t[1][1]), -t[2]))
    new = not os.path.exists(a.out) or os.path.getsize(a.out) == 0
    free = list(cores)
    free_lock = threading.Lock()

    def work(task):
        i, job, pt = task
        core = None
        if cores:
            with free_lock:
                core = free.pop()
        try:
            return snapshot(i, job, pt, a, core)
        finally:
            if cores:
                with free_lock:
                    free.append(core)

    bad = 0
    with open(a.out, "a", newline="") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS, delimiter="\t", lineterminator="\n")
        if new:
            w.writeheader()
        with ThreadPoolExecutor(a.jobs) as pool:
            for fut in as_completed([pool.submit(work, t) for t in tasks]):
                row = fut.result()
                w.writerow(row)
                f.flush()
                bad += failed(row["status"])
                print(f"{time.strftime('%H:%M:%S')} {row['cell']} at {row['point']}: {row['status']}"
                      f", {row['held_vars'] or '?'} variables held", flush=True)
    print(f"{len(tasks)} snapshots taken, {len(done)} were there already" + (f"; {bad} WITHOUT A RESULT" if bad else "")
          + (f"; {stay} rows without a result from an earlier call are still in the table (not asked for this time)" if stay else ""))
    return 1 if bad or stay else 0


# ---------------------------------------------------------------- report

def moment(point: int) -> str:
    return "file" if point < 0 else str(point) if point < 1000 else f"{point // 1000}K"


def num(r: dict, k: str) -> int:
    return int(r[k]) if r.get(k, "") != "" else 0


def cmd_report(a) -> int:
    rows = []
    for path in a.tables:
        with open(path, newline="") as f:
            rows.extend(csv.DictReader(f, delimiter="\t"))
    cells = defaultdict(dict)
    for r in rows:
        if int(r["point"]) in cells[r["cell"]]:
            sys.exit(f"two rows for {r['cell']} at {moment(int(r['point']))}: tables of different runs "
                     f"(another seed, other options) are reported one at a time")
        cells[r["cell"]][int(r["point"])] = r
    points = sorted({int(r["point"]) for r in rows})
    bad = [r for r in rows if failed(r["status"])]
    for r in bad:
        print(f"no result: {r['cell']} at {moment(int(r['point']))}: {r['status']}")
    ok = {c: {p: r for p, r in pts.items() if not failed(r["status"])} for c, pts in cells.items()}
    # the rebuilt set against the solver's own last report line
    runs = [r for pts in ok.values() for r in pts.values()
            if r.get("solver_vars", "") != "" and r.get("raw_two", "") != "" and r["refuted"] != "1"]
    if runs:
        d_two = [num(r, "raw_two") - num(r, "solver_two") for r in runs]
        d_long = [num(r, "raw_longer") - num(r, "solver_longer") - num(r, "solver_learned") for r in runs]
        d_var = [num(r, "solver_vars") - num(r, "held_vars") for r in runs]
        print(f"check on {len(runs)} snapshots of a run that is not refuted, the rebuilt set against the solver's last report "
              f"line: two-literal clauses the same on {sum(1 for x in d_two if x == 0)} (the rebuilt set has {min(d_two)} to "
              f"{max(d_two)} more), longer clauses the same on {sum(1 for x in d_long if x == 0)} ({min(d_long)} to {max(d_long)} "
              f"more); the solver counts {min(d_var)} to {max(d_var)} more variables as live than have a clause in the rebuilt "
              f"set; deletions of a clause not held: {sum(num(r, 'unmatched_deletes') for r in runs)}")
        for r in runs:
            two, lng = num(r, "raw_two") - num(r, "solver_two"), num(r, "raw_longer") - num(r, "solver_longer") - num(r, "solver_learned")
            if two or lng:
                print(f"    {r['cell']} at {moment(int(r['point']))}: {two} more two-literal clauses, {lng} more longer ones, "
                      f"{-(num(r, 'solver_vars') - num(r, 'held_vars'))} more variables than the solver counts")
        print()

    def at(r):
        """Where the run stood: the moment asked for, or the answer if it came first."""
        return {"SATISFIABLE": " (model found)", "UNSATISFIABLE": " (refuted)"}.get(r["status"], "")

    pyr = {c: pts for c, pts in ok.items() if any(r.get("pyramid", "0") not in ("", "0") for r in pts.values())}
    if pyr:
        print("== pyramid cells: the variables held, and the channel test (not set / tested)")
        head = " | ".join(f"{moment(p):>14}" for p in points)
        print(f"{'cell':34} {'file vars':>9} | {head}")
        tot = {p: [0, 0, 0] for p in points}
        for c in sorted(pyr):
            cols = []
            for p in points:
                r = pyr[c].get(p)
                if r is None:
                    cols.append(f"{'-':>14}")
                elif r["refuted"] == "1":
                    cols.append(f"{'refuted':>14}")
                elif r["placements"] == "":
                    cols.append(f"{'-':>14}")
                else:
                    tested = num(r, "link_set") + num(r, "link_unset")
                    cols.append(f"{num(r, 'held_vars'):>6} {num(r, 'link_unset'):>4}/{tested:<4}")
                    tot[p][0] += num(r, "link_unset")
                    tot[p][1] += tested
                    tot[p][2] += 1
            first = next(iter(pyr[c].values()))
            print(f"{c:34} {first['file_vars']:>9} | " + " | ".join(cols))
        print(f"{'all cells: not set / tested':44} | " + " | ".join(
            f"{tot[p][0]:>6}/{tot[p][1]:<7}" for p in points))
        print("\n== pyramid cells: what the held variables stand for at the last moment")
        print(f"{'cell':34} {'moment':>8} {'held':>7} {'placements':>10} {'label bits':>10} {'other':>7} {'solver-made':>11}")
        for c in sorted(pyr):
            live = {p: r for p, r in pyr[c].items() if r["refuted"] != "1" and r["placements"] != ""}
            if not live:
                continue
            p = max(live)
            r = live[p]
            print(f"{c:34} {moment(p) + at(r):>8} {num(r, 'held_vars'):>7} {num(r, 'stand_placement'):>10} "
                  f"{num(r, 'stand_bit'):>10} {num(r, 'stand_other'):>7} {num(r, 'stand_solver'):>11}")
    add = {c: pts for c, pts in ok.items() if any(r.get("with_added", "0") == "1" for r in pts.values())}
    if add:
        print("\n== appended two-literal clauses: seen from both sides / still testable  (+ true already, eliminated)")
        head = " | ".join(f"{moment(p):>24}" for p in points)
        print(f"{'cell':40} {'appended':>8} | {head}")
        tot = {p: [0, 0, 0, 0, 0] for p in points}
        for c in sorted(add):
            cols = []
            n_added = ""
            for p in points:
                r = add[c].get(p)
                if r is None:
                    cols.append(f"{'-':>24}")
                elif r["refuted"] == "1":
                    cols.append(f"{'refuted':>24}")
                elif r["added_two"] == "":
                    cols.append(f"{'-':>24}")
                else:
                    n_added = r["added_two"]
                    t = num(r, "added_both") + num(r, "added_one") + num(r, "added_none")
                    cols.append(f"{num(r, 'added_both'):>5}/{t:<5} (+{num(r, 'added_true')}, {num(r, 'added_gone')})".rjust(24))
                    for i, k in enumerate(("added_both", "added_true", "added_gone", "added_sampled")):
                        tot[p][i] += num(r, k)
                    tot[p][4] += t
            print(f"{c[:40]:40} {n_added:>8} | " + " | ".join(cols))
        print(f"{'all cells: both sides / testable':49} | " + " | ".join(
            f"{tot[p][0]:>5}/{tot[p][4]:<5} (+{tot[p][1]}, {tot[p][2]})".rjust(24) for p in points))
    return 1 if bad else 0


# ---------------------------------------------------------------- selftest

def selftest() -> int:
    # 1. the proof applied to a toy: copies, set order, a deletion of a clause not held
    live = Counter(key(c) for c in [[-1, 2], [-2, 3], [1, 4], [1, 4]])
    with tempfile.TemporaryDirectory() as d:
        proof = os.path.join(d, "p")
        Path(proof).write_text("3 -1 0\nd 2 -1 0\nd 4 1 0\nd 5 6 0\n\n7 0\n")
        st = apply_proof(live, proof)
        assert (st["proof_adds"], st["proof_deletes"], st["unmatched_deletes"], st["top"]) == (2, 2, 1, 7), st
        assert +live == Counter({(-2, 3): 1, (1, 4): 1, (-1, 3): 1, (7,): 1}), live
        for text, what in (("1 2\n", "closed by 0"), ("1 2 0", "closed by 0"), ("a 1 2 0\n", "list of literals"),
                           ("1 0 2 0\n", "0 inside")):
            Path(proof).write_text(text)
            try:
                apply_proof(Counter(), proof)
                raise AssertionError(text)
            except ProofError as e:
                assert what in str(e), (text, e)
        Path(proof).write_bytes(b"a\x82\x04\x00d\xff\x00")
        try:
            apply_proof(Counter(), proof)
            raise AssertionError("binary proof read")
        except ProofError:
            pass

        # 2. a hidden link, as Z3 writes it: x in 0..2 as bits 1 (x > 0) and
        # 2 (x > 1) with the chain 2 -> 1, and T = 3 for "x = 1" written as
        # "exactly one of the two bits".  T sets no bit; the links are
        # "T -> 1" and "T -> not 2".  The cell with the links appended:
        cell, plus = os.path.join(d, "cell.cnf"), os.path.join(d, "plus.cnf")
        cl = [[1, -2], [-3, 1, 2], [-3, -1, -2], [3, -1, 2], [3, 1, -2]]
        links = [[-3, 1], [-3, -2]]
        Path(cell).write_text("p cnf 3 5\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in cl))
        Path(plus).write_text("p cnf 3 7\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in cl + links))
        n, orig = read_cnf(cell)
        assert n == 3 and appended(orig, read_cnf(plus)[1]) == [(-3, 1), (-3, -2)]
        Path(proof).write_text("")
        r = analyse(cell, proof, added=plus)
        assert (r["held_vars"], r["held_clauses"], r["added_both"], r["added_one"], r["added_none"]) == (3, 5, 0, 2, 0), r
        # the solver's simplifier at work, by hand: it learns "T -> 1", cuts
        # the clause it subsumes, and strengthens another with the chain
        Path(proof).write_text("-3 1 0\nd -3 1 2 0\n-3 -2 0\nd -3 -1 -2 0\n")
        r = analyse(cell, proof, added=plus)
        assert (r["held_clauses"], r["added_both"], r["added_one"], r["unmatched_deletes"]) == (5, 2, 0, 0), r
        # the variable T merged into a new variable 4 (T = 4), T's clauses
        # rewritten and the two clauses of the equality deleted: T has no
        # clause left and 4 stands in for it
        Path(proof).write_text("-3 4 0\n3 -4 0\n-4 1 0\n-4 -2 0\n4 -1 2 0\n4 1 -2 0\n"
                               "d -3 1 2 0\nd -3 -1 -2 0\nd 3 -1 2 0\nd 3 1 -2 0\nd -3 4 0\nd 3 -4 0\n")
        r = analyse(cell, proof, added=plus)
        assert (r["held_vars"], r["made_by_solver"], r["added_both"], r["added_gone"]) == (3, 1, 2, 0), r
        # the same with a header that counts 9 variables where the clauses
        # use 3: variable 4 is still the solver's own
        wide = os.path.join(d, "wide.cnf")
        Path(wide).write_text("p cnf 9 5\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in cl))
        r = analyse(wide, proof, added=plus)
        assert (r["file_vars"], r["held_vars"], r["made_by_solver"], r["added_both"]) == (9, 3, 1, 2), r
        # T eliminated with nothing equal to it: counted apart
        Path(proof).write_text("d -3 1 2 0\nd -3 -1 -2 0\nd 3 -1 2 0\nd 3 1 -2 0\n")
        r = analyse(cell, proof, added=plus)
        assert (r["held_vars"], r["added_both"], r["added_gone"]) == (2, 0, 2), r
        # a literal fixed true settles an appended clause even when its
        # other variable is eliminated: T's clauses deleted, bit 1 fixed
        Path(proof).write_text("1 0\nd -3 1 2 0\nd -3 -1 -2 0\nd 3 -1 2 0\nd 3 1 -2 0\n")
        r = analyse(cell, proof, added=plus)
        assert (r["added_true"], r["added_gone"], r["added_both"]) == (1, 1, 0), r
        # a unit that makes an appended clause true; and a refutation
        Path(proof).write_text("1 0\n")
        r = analyse(cell, proof, added=plus)
        assert (r["fixed"], r["added_true"], r["added_both"], r["added_one"]) == (1, 1, 1, 0), r
        Path(proof).write_text("1 0\n-1 0\n")
        r = analyse(cell, proof, added=plus)
        assert r["refuted"] == 1 and "added_two" not in r, r
        # clauses the structure stage added before the proof starts
        der = os.path.join(d, "der.cnf")
        Path(der).write_text("p cnf 3 2\n-3 1 0\n-3 -2 0\n")
        Path(proof).write_text("")
        r = analyse(cell, proof, derived=der, derived_first=2, added=plus)
        assert (r["held_clauses"], r["added_both"]) == (7, 2), r
        # the same file with a clause an engine proved during the search
        # behind them: held only where the proof says the solver stored it,
        # and gone again when the proof deletes it
        Path(der).write_text("p cnf 3 3\n-3 1 0\n-3 -2 0\n3 -1 -2 0\n")
        r = analyse(cell, proof, derived=der, derived_first=2, added=plus)
        assert (r["held_clauses"], r["raw_longer"]) == (7, 4), r
        Path(proof).write_text("3 -1 -2 0\n")
        r = analyse(cell, proof, derived=der, derived_first=2, added=plus)
        assert (r["held_clauses"], r["raw_longer"]) == (8, 5), r
        Path(proof).write_text("3 -1 -2 0\nd 3 -1 -2 0\n")
        r = analyse(cell, proof, derived=der, derived_first=2, added=plus)
        assert (r["held_clauses"], r["raw_longer"]) == (7, 4), r
        # only the first is the stage's; with "T -> 1" held, "T -> not 2"
        # is seen from both sides too (T sets 1, and "not T or not 1 or not 2" gives not 2)
        r = analyse(cell, proof, derived=der, derived_first=1, added=plus)
        assert (r["held_clauses"], r["raw_two"], r["added_both"], r["added_one"]) == (6, 2, 2, 0), r
        try:
            analyse(cell, proof, derived=der, derived_first=4)
            raise AssertionError("a derived file that is too short was read")
        except ProofError:
            pass
        # a refuted snapshot is written as a refuted formula
        Path(proof).write_text("1 0\n-1 0\n")
        held = os.path.join(d, "held.cnf")
        analyse(cell, proof, out=held)
        assert Path(held).read_text().split("\n")[1] == "0", Path(held).read_text()
        Path(proof).write_text("")
        analyse(cell, proof, out=held)
        assert sorted(key(c) for c in read_cnf(held)[1]) == sorted(key(c) for c in cl), read_cnf(held)
        # a file with a repeated literal and a tautology: named in the proof as read
        odd = os.path.join(d, "odd.cnf")
        Path(odd).write_text("p cnf 2 2\n1 1 2 0\n1 -1 0\n")
        Path(proof).write_text("1 2 0\nd 1 1 2 0\nd 1 -1 0\n")
        r = analyse(odd, proof)
        assert (r["held_clauses"], r["unmatched_deletes"]) == (1, 0), r
        Path(odd).write_text("p cnf 2 1\n1 2\n")
        try:
            read_cnf(odd)
            raise AssertionError("an open clause was read")
        except SystemExit:
            pass

        # 3. the equalities: 1 = 2 in the file, then the proof says 2 = -5
        # and later 2 = 7; one-way clauses and a clause with its own mirror
        # ("x or not x") say nothing
        eq = Equalities()
        for k in ((-1, 2), (-2, 1), (-3, 4), (-9, 9)):
            eq.note(key(k))
        assert eq.links == {1: {2: 1}, 2: {1: 1}}, eq.links
        for k in ((2, 5), (-5, -2), (-2, 7), (-7, 2)):
            eq.note(key(k))
        assert eq.links == {1: {2: 1}, 2: {1: 1, -5: 2, 7: 3}, 5: {-2: 2}, 7: {2: 3}}, eq.links
        # followed from a variable with no clause left: the equality given
        # last first (2 leads to 7 when 7 is held), an earlier one when that
        # leads nowhere (7 gone too: on to -5, then to 1), signs kept
        def held_with(*vs):
            h = Held({1, 2, 5, 7}, 9, Counter({(v, 9): 1 for v in vs}), eq.links)
            return h.stand_in(2), h.stand_in(-2), h.stand_in(5)
        assert held_with(1, 5, 7) == (7, -7, 5), held_with(1, 5, 7)
        assert held_with(1, 5) == (-5, 5, 5), held_with(1, 5)
        assert held_with(1) == (1, -1, -1), held_with(1)
        assert held_with() == (None, None, None), held_with()
        # the file's own equalities 1 = 2 and 2 = 3, with 2 and 3 eliminated
        # through them and 1 kept: both still lead to 1
        eq2 = Equalities()
        for k in ((-1, 2), (1, -2), (-2, 3), (2, -3)):
            eq2.note(key(k))
        h = Held({1, 2, 3}, 9, Counter({(1, 9): 1}), eq2.links)
        assert (h.stand_in(2), h.stand_in(3), h.stand_in(-3)) == (1, 1, -1), (h.stand_in(2), h.stand_in(3))
        # a chain of merges: 3 into 4 (file's T into a new variable), then 4
        # into -6; and a variable whose partner was merged into it and that
        # was eliminated afterwards leads nowhere
        Path(proof).write_text("-3 4 0\n3 -4 0\n-4 1 0\n-4 -2 0\n4 -1 2 0\n4 1 -2 0\n"
                               "d -3 1 2 0\nd -3 -1 -2 0\nd 3 -1 2 0\nd 3 1 -2 0\nd -3 4 0\nd 3 -4 0\n"
                               "4 6 0\n-4 -6 0\n6 1 0\n6 -2 0\n-6 -1 2 0\n-6 1 -2 0\n"
                               "d -4 1 0\nd -4 -2 0\nd 4 -1 2 0\nd 4 1 -2 0\nd 4 6 0\nd -4 -6 0\n")
        r = analyse(cell, proof, added=plus)
        assert (r["held_vars"], r["added_both"], r["added_gone"], r["unmatched_deletes"]) == (3, 2, 0, 0), r
        Path(proof).write_text("-3 4 0\n3 -4 0\n-3 1 0\n-3 -2 0\nd -3 1 2 0\nd -3 -1 -2 0\nd -3 4 0\nd 3 -4 0\n"
                               "d -3 1 0\nd -3 -2 0\nd 3 -1 2 0\nd 3 1 -2 0\n")
        r = analyse(cell, proof, added=plus)
        assert (r["held_vars"], r["added_both"], r["added_gone"]) == (2, 0, 2), r

        # 4. the solver's output
        said = read_solver_output("c seconds\nc } 49.54 242 26 22 59 6534 4 300000 23007 1.6 12 7 3 7 40% 137610 24408 22507 24%\n"
                                  "c structure pass: 4647 derived clauses (49 units), equisatisfiable extension, work 5K\n"
                                  "s UNKNOWN\nc conflicts:   300000   6055.03 per second\nc workclock ticks=5 work=77 x=1\n")
        assert said == {"answer": "UNKNOWN", "conflicts": 300000, "work": 77, "solver_learned": 23007, "stage_clauses": 4647,
                        "solver_two": 137610, "solver_longer": 24408, "solver_vars": 22507}, said
        assert "answer" not in read_solver_output("s SATISFIABLE\ns UNSATISFIABLE\n")
        assert read_solver_output("c structure pass: 1 derived clause (1 unit), refuted, work 0K\n")["stage_clauses"] == 1
        assert read_solver_output("s UNKNOWN\n")["stage_clauses"] == 0
        quiet = "c control ctlsplit=1 status=no-vars cases=0 refuted=0 unknown=0 sat=0 work=0 lemmas=0 pre=4 read=8\n"
        ran = "c control ctlsplit=1 status=unsat cases=16 refuted=16 unknown=0 sat=0 work=9 lemmas=77 pre=4 read=8\n"
        assert "split" not in read_solver_output(quiet) and read_solver_output(ran)["split"] == "status unsat, 16 cases"
        assert read_solver_output("c control ctlsplit=1 status=unfinished cases=0 refuted=0\n")["split"] == "status unfinished, 0 cases"
        assert parse_cores("2-4,7") == [2, 3, 4, 7]

        # 5. snap with stand-in solvers: a good run, a wrong exit code, an
        # UNKNOWN before the limit, a hang (its child must go too), and the
        # tool itself stopped by a signal
        def script(name, body):
            path = os.path.join(d, name)
            Path(path).write_text("#!/bin/bash\nproof=\"${@: -1}\"\nfor x in \"$@\"; do case $x in --conflicts=*) k=${x#*=};; esac; done\n" + body)
            os.chmod(path, 0o755)
            return path

        good = script("good", "printf -- '-3 1 0\\nd -3 1 2 0\\n-3 -2 0\\nd -3 -1 -2 0\\n' > \"$proof\"\n"
                              "echo \"c conflicts: $k\"\necho 's UNKNOWN'\nexit 0\n")
        wrong = script("wrong", ": > \"$proof\"\necho \"c conflicts: $k\"\necho 's UNKNOWN'\nexit 3\n")
        early = script("early", ": > \"$proof\"\necho 'c conflicts: 1'\necho 's UNKNOWN'\nexit 0\n")
        hang = script("hang", f"sleep 300 &\necho $! > {shlex.quote(os.path.join(d, 'hang.pid'))}\nwait\n")
        cut = script("cut", "printf -- '-3 1' > \"$proof\"\necho \"c conflicts: $k\"\necho 's UNKNOWN'\nexit 0\n")
        split = script("split", ": > \"$proof\"\necho \"c conflicts: $k\"\necho 's UNKNOWN'\n"
                                "echo 'c control ctlsplit=1 status=unknown cases=4 refuted=3 unknown=1 sat=0 work=9'\nexit 0\n")
        jobs = os.path.join(d, "jobs.tsv")
        Path(jobs).write_text(f"toy\t{cell}\t0\t{plus}\n")

        def run(solver, out, *more, timeout="60"):
            return subprocess.run([sys.executable, os.path.abspath(__file__), "snap", out, jobs, "--scratch", os.path.join(d, "s"),
                                   "--solver", solver, "--points=-1,5", "--jobs", "2", "--timeout", timeout, *more],
                                  capture_output=True, text=True)

        def table(out):
            with open(out, newline="") as f:
                return {r["point"]: r for r in csv.DictReader(f, delimiter="\t")}

        out = os.path.join(d, "good.tsv")
        res = run(good, out)
        assert res.returncode == 0, res.stdout + res.stderr
        t = table(out)
        assert (t["-1"]["status"], t["-1"]["added_one"], t["-1"]["added_both"]) == ("FILE", "2", "0"), t["-1"]
        assert (t["5"]["status"], t["5"]["conflicts"], t["5"]["added_both"], t["5"]["held_clauses"]) == ("UNKNOWN", "5", "2", "5"), t["5"]
        assert "\r" not in Path(out).read_text()
        assert os.listdir(os.path.join(d, "s")) == [], os.listdir(os.path.join(d, "s"))
        res = run(good, out)                                   # nothing to do the second time
        assert res.returncode == 0 and "0 snapshots taken, 2 were there already" in res.stdout, res.stdout
        rep = subprocess.run([sys.executable, os.path.abspath(__file__), "report", out], capture_output=True, text=True)
        assert rep.returncode == 0 and "0/2" in rep.stdout and "2/2" in rep.stdout, rep.stdout + rep.stderr
        for solver, what in ((wrong, "ERROR(exit 3"), (early, "before the limit"), (split, "the control split ran"),
                             (cut, "the proof was not read")):
            out = os.path.join(d, os.path.basename(solver) + ".tsv")
            res = run(solver, out)
            assert res.returncode == 1 and what in table(out)["5"]["status"], (solver, res.stdout, res.stderr)
            assert table(out)["5"]["held_vars"] == "", table(out)["5"]
            # taken up again: the failed row is tried again and fails again, not passed over
            res = run(solver, out)
            assert res.returncode == 1 and "1 rows without a result are tried again" in res.stdout, res.stdout
            assert len(Path(out).read_text().splitlines()) == 3 and what in table(out)["5"]["status"], Path(out).read_text()
        # a call that does not ask for the failed moment leaves its row
        # where it is, and does not end as a success
        res = subprocess.run([sys.executable, os.path.abspath(__file__), "snap", out, jobs, "--scratch", os.path.join(d, "s"),
                              "--solver", good, "--points=-1", "--jobs", "1"], capture_output=True, text=True)
        assert res.returncode == 1 and "still in the table" in res.stdout, res.stdout + res.stderr
        assert "the proof was not read" in table(out)["5"]["status"], table(out)
        rep = subprocess.run([sys.executable, os.path.abspath(__file__), "report", out], capture_output=True, text=True)
        assert rep.returncode == 1 and "no result" in rep.stdout, rep.stdout
        # and with a good solver the failed row is replaced by a result
        res = run(good, out)
        assert res.returncode == 0 and table(out)["5"]["status"] == "UNKNOWN", res.stdout + res.stderr
        assert len(Path(out).read_text().splitlines()) == 3, Path(out).read_text()
        Path(os.path.join(d, "other.tsv")).write_text("a\tb\n1\t2\n")
        assert run(good, os.path.join(d, "other.tsv")).returncode != 0          # not a table of this tool
        out = os.path.join(d, "hang.tsv")
        res = run(hang, out, timeout="4")
        assert res.returncode == 1 and table(out)["5"]["status"] == "TIMEOUT", res.stdout + res.stderr
        pid = int(Path(os.path.join(d, "hang.pid")).read_text())
        time.sleep(0.2)
        assert not os.path.exists(f"/proc/{pid}"), "the hanging solver's child is still there"
        os.unlink(os.path.join(d, "hang.pid"))
        out = os.path.join(d, "signal.tsv")
        runner = subprocess.Popen([sys.executable, os.path.abspath(__file__), "snap", out, jobs, "--scratch", os.path.join(d, "s2"),
                                   "--solver", hang, "--points=5", "--jobs", "1", "--timeout", "300"],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        end = time.time() + 20
        while not os.path.exists(os.path.join(d, "hang.pid")) and time.time() < end:
            time.sleep(0.05)
        time.sleep(0.2)
        pid = int(Path(os.path.join(d, "hang.pid")).read_text())
        runner.send_signal(signal.SIGTERM)
        code = runner.wait(timeout=30)
        assert code == 128 + signal.SIGTERM, code
        time.sleep(0.2)
        assert not os.path.exists(f"/proc/{pid}"), "a solver outlived the tool"
        # two cells of one name, and a cell of the holdout, are refused
        Path(jobs).write_text(f"toy\t{cell}\t0\t-\ntoy\t{cell}\t0\t-\n")
        assert run(good, os.path.join(d, "twice.tsv")).returncode != 0
        hold = os.path.join(d, "sat-comp-2026")
        os.mkdir(hold)
        Path(os.path.join(hold, "x.cnf")).write_text("p cnf 1 1\n1 0\n")
        Path(jobs).write_text(f"toy\t{os.path.join(hold, 'x.cnf')}\t0\t-\n")
        assert run(good, os.path.join(d, "hold.tsv")).returncode != 0
    print("selftest ok")
    return 0


def main(argv) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("read")
    r.add_argument("cell")
    r.add_argument("proof")
    r.add_argument("--derived", help="PROOF.derived.cnf: the structure stage's clauses, then the engines' clauses")
    r.add_argument("--derived-first", type=int, default=None,
                   help="how many clauses at the start of --derived the structure stage added (the K of the solver's line "
                        "'c structure pass: K derived clauses'; 0 when it prints none)")
    r.add_argument("--pyramid", type=int, default=0, help="the pyramid's size N: read placements and labels")
    r.add_argument("--added", help="the cell with implied clauses appended")
    r.add_argument("--sample", type=int, default=3000)
    r.add_argument("--seed", type=int, default=1)
    r.add_argument("--out", help="write the held clauses to this file")
    r.add_argument("--json")
    s = sub.add_parser("snap")
    s.add_argument("out")
    s.add_argument("jobs_file")
    s.add_argument("--scratch", required=True)
    s.add_argument("--points", default="-1,0,30000,300000")
    s.add_argument("--jobs", type=int, default=8)
    s.add_argument("--cores", default="")
    s.add_argument("--solver", default=str(DEFAULT_SOLVER))
    s.add_argument("--seed", type=int, default=2)
    s.add_argument("--args", default="", help="more options for the solver")
    s.add_argument("--mem-mb", type=int, default=16000)
    s.add_argument("--read-mem-mb", type=int, default=48000)
    s.add_argument("--timeout", type=float, default=3600)
    s.add_argument("--sample", type=int, default=3000)
    s.add_argument("--keep", action="store_true", help="keep the proofs and outputs in the scratch folder")
    p = sub.add_parser("report")
    p.add_argument("tables", nargs="+")
    sub.add_parser("selftest")
    a = ap.parse_args(argv)
    if a.cmd == "read":
        if (a.derived is None) != (a.derived_first is None):
            ap.error("--derived and --derived-first go together")
        a.derived_first = a.derived_first or 0
        return cmd_read(a)
    if a.cmd == "snap":
        return cmd_snap(a)
    if a.cmd == "report":
        return cmd_report(a)
    return selftest()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
