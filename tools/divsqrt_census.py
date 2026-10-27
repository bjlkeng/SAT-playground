#!/usr/bin/env python3
"""Divider and square-root miter census (bead SAT-playground-skz.17).

The 2025 suite's div and sqrt miters (div-mitern172, div_miter_lec__2,
sqrt-mitern169..171) read back as circuits.  Per cell:

  gates      every clause belongs to the gate its first literal names: the
             files list each gate as the on-set and off-set covers of a
             function of at most four inputs, the gate's own literal first
             (the shape of ABC's write_cnf).  Each group is checked to be a
             definition by its truth table.  The unit on a gate is the
             miter's root; a unit on any other variable is a constant.
  direction  gates whose clause set also defines one of their inputs
             (XOR-like tables): read from the clauses alone their direction
             is open, and the numbering does not settle it (outputs are
             numbered below their inputs for some gates, above for others).
  miter      the root is the XOR of two signals; the two sides are their
             cones (gates in both are counted as shared).
  function   the inputs' roles from probes: a divider output (bit k of
             floor(a / b) for n-bit a and b, the inputs being a[k..n-1] and
             b[0..n-1], n = 64 in the real cells; b = 0 gives all ones, as a
             restoring array does) or a
             square-root output (bit k of isqrt(a) for a 128-bit a, the
             inputs being a[2k..127]).  The inputs are numbered in the text
             order of their names (b[0], b[10], ..., b[19], b[1], b[20], ...).
             Checked on random samples; a cell that fits neither is
             'unknown'.
  rows       the restoring recurrence simulated alongside the circuit.  For
             a divider, row j (n-1 down to k) brings down a_j, T = 2R + a_j,
             q_j = T >= b, R = q_j ? T - b : T.  For a square root, step i
             gives Q = isqrt(a >> 2i) and R = (a >> 2i) - Q^2.  Per side,
             how many of the decided bits (q_j or the root bits) and of the
             remainder bits some signal computes: the same or the
             complemented signature over random samples, with divisors of
             random bit length so that every row's quotient bit varies.  A
             match is a candidate, not a proof.

Subcommands:
  census [--out TABLE.tsv] [FILES...]   one row per cell (default: the 2025
                                        suite's div and sqrt miters)
  chain FILE --solver BIN --out DIR [--jobs N] [--limit S] [--window W]
        the cut-point chain: rows top down, one signal per side for each
        matched value (in a row the decided bit first, then the remainder
        bits from the lowest); each pair proven equal by the solver on the
        circuit without the miter's assertion, assuming every pair before
        it.  The pairs are checked W at a time (default 32), each assuming
        the proven pairs before the window and the window's pairs before
        it; a pair that is not proven is dropped and the window's later
        pairs are checked again without it.  A failed check's model is an
        input the circuit is simulated on, and every pending pair that
        differs there is dropped as well.  Then the miter itself with
        every proven pair added.  The solver runs the stock path (every
        structure pass off) under the time limit and --mem-mb (default
        16,000); a crash or an answer with the wrong exit code stops the
        run, a timeout or memory-out drops the pair.

  blif FILE --out FILE.blif              the netlist as BLIF, one output (the
                                        miter asserted), for measurements with
                                        ABC (tools/checkers/abc: read_blif;
                                        strash; iprove), never shipped

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import random
import re
import subprocess
import sys
import tempfile
import time
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from math import isqrt
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from proof_gate import iter_clauses  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
HOLDOUT = ROOT / "benchmarks" / "sat-comp-2026"
CELL_RE = re.compile(r"(div-mitern|div_miter|sqrt-mitern)")
MAX_SUPPORT = 8          # largest gate support read by truth table
SAMPLES = 2048           # random samples for the function check and the rows
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0",
         "--structsort=0", "--gauss=0"]
MEM_MB = 16000           # memory cap per solver process (the repo's 16 GB rule)


# ---------------------------------------------------------------------------
# reading
# ---------------------------------------------------------------------------


def read_cnf(path):
    """(header variable count, clauses), read with the solver's DIMACS rules
    (tools/proof_gate.py)."""
    gen = iter_clauses(path)
    clauses = []
    while True:
        try:
            clauses.append(tuple(next(gen)))
        except StopIteration as stop:
            return stop.value or 0, clauses


def definition_tt(v, cls, sup):
    """Truth table of v over sup if the clauses define v there, else None.
    Every clause must contain v; the others' literals must lie in sup."""
    idx = {u: i for i, u in enumerate(sup)}
    pos, neg = [], []
    for c in cls:
        others = []
        sign = 0
        for lit in c:
            if abs(lit) == v:
                if sign and sign != (1 if lit > 0 else -1):
                    return None  # both polarities of v: a tautology, no definition
                sign = 1 if lit > 0 else -1
            else:
                others.append((idx[abs(lit)], lit > 0))
        if sign == 0:
            return None
        (pos if sign > 0 else neg).append(others)
    tt = 0
    for m in range(1 << len(sup)):
        pv = any(all(((m >> i) & 1) != val for i, val in oth) for oth in pos)
        nv = any(all(((m >> i) & 1) != val for i, val in oth) for oth in neg)
        if pv == nv:
            return None  # forbidden or free here
        if pv:
            tt |= 1 << m
    return tt


class Netlist:
    """Gates by first literal.  Raises ValueError when the file does not
    read that way."""

    def __init__(self, path):
        self.path = Path(path)
        self.header_vars, self.clauses = read_cnf(path)
        groups = defaultdict(list)
        units = {}
        for i, c in enumerate(self.clauses):
            lits = set(c)
            if not lits:
                raise ValueError("empty clause")
            if len(lits) == 1:
                lit = c[0]
                if abs(lit) in units and units[abs(lit)] != (lit > 0):
                    raise ValueError("contradicting units")
                units[abs(lit)] = lit > 0
                continue
            groups[abs(c[0])].append(i)
        self.groups = groups
        self.gates = {}
        for v, ids in groups.items():
            cls = [self.clauses[i] for i in ids]
            sup = tuple(sorted({abs(lit) for c in cls for lit in c} - {v}))
            tt = definition_tt(v, cls, sup) if len(sup) <= MAX_SUPPORT else None
            if tt is None:
                raise ValueError(f"the clauses of {v} are no definition")
            self.gates[v] = (sup, tt)
        self.roots = {v: b for v, b in units.items() if v in self.gates}
        self.const = {v: b for v, b in units.items() if v not in self.gates}
        present = {abs(lit) for c in self.clauses for lit in c}
        self.inputs = sorted(present - set(self.gates) - set(self.const))
        self.order = self._topo()

    def _topo(self):
        order, state = [], {}
        for root in sorted(self.gates):
            if root in state:
                continue
            stack = [(root, False)]
            while stack:
                v, done = stack.pop()
                if done:
                    state[v] = 2
                    order.append(v)
                    continue
                if v in state:
                    if state[v] == 1:
                        raise ValueError(f"cycle through {v}")
                    continue
                state[v] = 1
                stack.append((v, True))
                for u in self.gates[v][0]:
                    if u in self.gates and state.get(u) != 2:
                        if state.get(u) == 1:
                            raise ValueError(f"cycle through {u}")
                        stack.append((u, False))
        return order

    def cone(self, v):
        seen, stack = set(), [v]
        while stack:
            x = stack.pop()
            if x in seen:
                continue
            seen.add(x)
            if x in self.gates:
                stack.extend(self.gates[x][0])
        return seen

    def simulate(self, inputs, mask):
        """inputs: var -> bit-parallel int; returns every signal's value."""
        val = dict(inputs)
        for v, b in self.const.items():
            val[v] = mask if b else 0
        for v in self.order:
            sup, tt = self.gates[v]
            ins = [val[u] for u in sup]
            out = 0
            for m in range(1 << len(sup)):
                if (tt >> m) & 1:
                    term = mask
                    for i, w in enumerate(ins):
                        term &= w if (m >> i) & 1 else ~w & mask
                    out |= term
            val[v] = out
        return val


def direction_open(net):
    """Gates whose clause set also defines one of their inputs."""
    n = 0
    for v, (sup, tt) in net.gates.items():
        cls = [net.clauses[i] for i in net.groups[v]]
        for u in sup:
            if all(any(abs(lit) == u for lit in c) for c in cls):
                rest = tuple(sorted((set(sup) - {u}) | {v}))
                if definition_tt(u, cls, rest) is not None:
                    n += 1
                    break
    return n


# ---------------------------------------------------------------------------
# the miter and the function
# ---------------------------------------------------------------------------


def miter_sides(net):
    """(root, A, B, invert).  The root is an xor or xnor of the two side
    signals A and B, asserted by a unit; invert is True when what it
    asserts is A == B.  A refutation then shows A != B always, i.e. A is
    the complement of B (the sides compute the same bit, one negated)."""
    if len(net.roots) != 1:
        raise ValueError(f"{len(net.roots)} asserted gates")
    (root, val), = net.roots.items()
    sup, tt = net.gates[root]
    if len(sup) != 2 or tt not in (0b0110, 0b1001):
        raise ValueError("the root is no xor of two signals")
    invert = (tt == 0b1001) == val   # xnor asserted true, or xor asserted false
    return root, sup[0], sup[1], invert


def text_order(prefix, lo, hi):
    """Bit indices lo..hi-1 in the text order of their names."""
    return [int(s[len(prefix) + 1:-1]) for s in sorted(f"{prefix}[{i}]" for i in range(lo, hi))]


def to_inputs(net, roles, values):
    """roles: input var -> (word, bit); values: per sample {word: int}."""
    inp = {v: 0 for v in net.inputs}
    for s, vals in enumerate(values):
        for v, (word, bit) in roles.items():
            if (vals[word] >> bit) & 1:
                inp[v] |= 1 << s
    return inp


def quotient(a, b, width):
    return (1 << width) - 1 if b == 0 else a // b


def read_divider(net, side, rnd):
    """(roles, k, negated, width) if the side is bit k of floor(a / b) for
    width-bit a and b, else None."""
    n = len(net.inputs)
    mask = (1 << (n + 1)) - 1
    inp = {v: 1 << (1 + i) for i, v in enumerate(net.inputs)}
    val = net.simulate(inp, mask)[side]
    zero = val & 1
    a_like = [v for i, v in enumerate(net.inputs) if ((val >> (1 + i)) & 1) == zero]
    b_like = [v for i, v in enumerate(net.inputs) if ((val >> (1 + i)) & 1) != zero]
    width = len(b_like)
    if width < 2 or not a_like or len(a_like) > width:
        return None
    k = width - len(a_like)
    roles = {v: ("b", i) for v, i in zip(b_like, text_order("b", 0, width))}
    roles.update({v: ("a", i) for v, i in zip(a_like, text_order("a", k, width))})
    samples = div_samples(rnd, k, width)
    mask = (1 << len(samples)) - 1
    got = net.simulate(to_inputs(net, roles, samples), mask)[side]
    want = 0
    for s, x in enumerate(samples):
        if (quotient(x["a"], x["b"], width) >> k) & 1:
            want |= 1 << s
    if got == want:
        return roles, k, False, width
    if got == ~want & mask:
        return roles, k, True, width
    return None


def div_samples(rnd, k, width, n=SAMPLES):
    """Mostly divisors of a random bit length up to width-k (so that every
    row's quotient bit is 1 on some samples and 0 on others); some zero and
    some full-width ones."""
    out = []
    small = width - k
    for _ in range(n):
        r = rnd.random()
        if r < 0.05 or small < 1:
            b = 0 if r < 0.05 else rnd.getrandbits(width)
        elif r < 0.15:
            b = rnd.getrandbits(width)
        else:
            length = rnd.randint(1, small)
            b = (1 << (length - 1)) | rnd.getrandbits(length - 1)
        a = rnd.getrandbits(width) & ~((1 << k) - 1)
        out.append({"a": a, "b": b})
    return out


def read_sqrt(net, side, rnd):
    """roles and k if the side is bit k of isqrt(a) for a 128-bit a."""
    n = len(net.inputs)
    lo = 128 - n
    if lo < 0 or lo % 2:
        return None
    k = lo // 2
    roles = {v: ("a", i) for v, i in zip(net.inputs, text_order("a", lo, 128))}
    samples = sqrt_samples(rnd, lo)
    mask = (1 << len(samples)) - 1
    got = net.simulate(to_inputs(net, roles, samples), mask)[side]
    want = 0
    for s, x in enumerate(samples):
        if (isqrt(x["a"]) >> k) & 1:
            want |= 1 << s
    if got == want:
        return roles, k, False, 128
    if got == ~want & mask:
        return roles, k, True, 128
    return None


def sqrt_samples(rnd, lo, n=SAMPLES):
    return [{"a": rnd.getrandbits(128 - lo) << lo} for _ in range(n)]


def row_values(kind, k, width, samples):
    """name -> per-sample bit list, in chain order (rows top down, the
    decided bit first, then the remainder bits from the lowest)."""
    spec = {}
    if kind == "div":
        for j in range(width - 1, k - 1, -1):
            spec[f"q{j}"] = []
            for i in range(width - j + 1):
                spec[f"R{j}.{i}"] = []
        for x in samples:
            a, b = x["a"], x["b"]
            r = 0
            for j in range(width - 1, k - 1, -1):
                t = 2 * r + ((a >> j) & 1)
                q = 1 if t >= b else 0
                r = t - b if q else t
                spec[f"q{j}"].append(q)
                for i in range(width - j + 1):
                    spec[f"R{j}.{i}"].append((r >> i) & 1)
    else:
        steps = 64 - k
        for i in range(steps - 1, -1, -1):
            spec[f"r{k + i}"] = []
            for bit in range(steps - i + 2):
                spec[f"R{k + i}.{bit}"] = []
        for x in samples:
            a = x["a"] >> (2 * k)
            for i in range(steps - 1, -1, -1):
                part = a >> (2 * i)
                q = isqrt(part)
                r = part - q * q
                spec[f"r{k + i}"].append(q & 1)
                for bit in range(steps - i + 2):
                    spec[f"R{k + i}.{bit}"].append((r >> bit) & 1)
    return spec


def pack(bits):
    w = 0
    for s, x in enumerate(bits):
        if x:
            w |= 1 << s
    return w


def read_cell(path, seed=1):
    """Everything the census and the chain need about one cell."""
    net = Netlist(path)
    root, a, b, invert = miter_sides(net)
    rnd = random.Random(seed)
    kind, fit = None, None
    for kd, reader in (("div", read_divider), ("sqrt", read_sqrt)):
        fit = reader(net, a, rnd)
        if fit is not None:
            kind = kd
            break
    cell = dict(net=net, root=root, a=a, b=b, invert=invert, kind=kind, fit=fit)
    if kind is None:
        return cell
    roles, k, neg, width = fit
    samples = div_samples(rnd, k, width) if kind == "div" else sqrt_samples(rnd, 128 - len(net.inputs))
    mask = (1 << len(samples)) - 1
    val = net.simulate(to_inputs(net, roles, samples), mask)
    cell["agree"] = val[a] == (val[b] ^ (mask if invert else 0))
    spec = {nm: pack(bits) for nm, bits in row_values(kind, k, width, samples).items()}
    cone_a, cone_b = net.cone(a), net.cone(b)
    shared = cone_a & cone_b
    level = {}
    for v in net.order:   # gate levels from the inputs: a pair's signals as shallow as they come
        level[v] = 1 + max((level.get(u, 0) for u in net.gates[v][0]), default=0)
    index = {}
    for side, cn in (("A", cone_a), ("B", cone_b)):
        idx = defaultdict(list)
        for v in sorted(cn, key=lambda u: (u in shared, level.get(u, 0), u)):  # own gates first, shallow first
            if v in net.gates:
                idx[val[v]].append(v)
                idx[~val[v] & mask].append(-v)
        index[side] = idx
    pairs, found = [], {"A": [0, 0], "B": [0, 0]}
    for nm, w in spec.items():
        if w in (0, mask):
            continue  # constant on the samples: no candidate
        is_bit = "." not in nm
        for side in ("A", "B"):
            if w in index[side]:
                found[side][0 if is_bit else 1] += 1
        if w in index["A"] and w in index["B"]:
            if any(abs(lit) in shared for lit in index["A"][w]):
                continue  # a gate both sides share computes it: nothing to prove
            pairs.append((nm, index["A"][w][0], index["B"][w][0]))
    total_bits = sum(1 for nm, w in spec.items() if "." not in nm and w not in (0, mask))
    total_rem = sum(1 for nm, w in spec.items() if "." in nm and w not in (0, mask))
    cell.update(k=k, negated=neg, width=width, cone_a=cone_a, cone_b=cone_b, shared=shared, pairs=pairs,
                found=found, total_bits=total_bits, total_rem=total_rem)
    return cell


# ---------------------------------------------------------------------------
# census
# ---------------------------------------------------------------------------


FIELDS = ["stem", "vars", "clauses", "gates", "inputs", "direction_open", "numbered_down",
          "numbered_up", "side_a_gates", "side_b_gates", "shared_gates", "function",
          "sides_agree", "bits_a", "bits_b", "rem_a", "rem_b", "pairs", "error"]


def census_row(path):
    stem = Path(path).name.split(".cnf")[0]
    row = {f: "" for f in FIELDS}
    row["stem"] = stem
    try:
        cell = read_cell(path)
    except ValueError as e:
        row["error"] = str(e)
        return row
    net = cell["net"]
    gates = net.gates
    row.update(vars=net.header_vars, clauses=len(net.clauses), gates=len(gates),
               inputs=len(net.inputs), direction_open=direction_open(net),
               numbered_down=sum(1 for v, (s, t) in gates.items() if all(u > v for u in s)),
               numbered_up=sum(1 for v, (s, t) in gates.items() if all(u < v for u in s)))
    if cell["kind"] is None:
        row["function"] = "unknown"
        return row
    ca, cb, sh = cell["cone_a"], cell["cone_b"], cell["shared"]
    row.update(side_a_gates=sum(1 for v in ca if v in gates), side_b_gates=sum(1 for v in cb if v in gates),
               shared_gates=sum(1 for v in sh if v in gates))
    neg = " (negated)" if cell["negated"] else ""
    row["function"] = (f"quotient bit {cell['k']} of {cell['width']}/{cell['width']}{neg}" if cell["kind"] == "div"
                       else f"root bit {cell['k']} of {cell['width']}{neg}")
    row["sides_agree"] = cell["agree"]
    fa, fb = cell["found"]["A"], cell["found"]["B"]
    row.update(bits_a=f"{fa[0]}/{cell['total_bits']}", bits_b=f"{fb[0]}/{cell['total_bits']}",
               rem_a=f"{fa[1]}/{cell['total_rem']}", rem_b=f"{fb[1]}/{cell['total_rem']}",
               pairs=len(cell["pairs"]))
    return row


def default_cells():
    return sorted(p for p in SUITE.glob("*.cnf*") if CELL_RE.search(p.name))


def refuse_holdout(paths):
    for p in paths:
        if HOLDOUT.resolve() in Path(p).resolve().parents:
            sys.exit(f"{p}: the 2026 suite is the holdout (plan section 8)")


def cmd_census(args):
    paths = [Path(p) for p in args.files] or default_cells()
    refuse_holdout(paths)
    rows = [census_row(p) for p in paths]
    out = open(args.out, "w", newline="") if args.out else sys.stdout
    w = csv.DictWriter(out, FIELDS, delimiter="\t", lineterminator="\n")
    w.writeheader()
    for r in rows:
        w.writerow(r)
    if args.out:
        out.close()
    return 0


# ---------------------------------------------------------------------------
# the cut-point chain
# ---------------------------------------------------------------------------


def write_cnf(path, n_vars, clauses):
    with open(path, "w") as f:
        f.write(f"p cnf {n_vars} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


def model_of(stdout):
    """The variables a model sets true (the solver's v lines)."""
    true = set()
    for line in stdout.splitlines():
        if line.startswith("v "):
            true.update(int(t) for t in line.split()[1:] if int(t) > 0)
    return true


def workclock(stdout):
    """(conflicts, work) from the solver's workclock line, -1 when missing."""
    m = re.search(r"^c workclock .*", stdout, re.M)
    line = m.group(0) if m else ""
    c = re.search(r" conflicts=(\d+)", line)
    w = re.search(r" work=(\d+)", line)
    return int(c.group(1)) if c else -1, int(w.group(1)) if w else -1


def run_solver(solver, args, limit, mem_mb=MEM_MB):
    """The solver under a time limit and a memory cap.  (status, seconds,
    stdout): status is the answer of its s line, or TIMEOUT or MEMOUT for a
    budget stop.  Any other exit, or an answer whose exit code disagrees,
    raises: a broken run must not pass for a budget stop."""
    cmd = ["prlimit", f"--as={mem_mb * 1000000}", "--", "timeout", str(limit), solver] + list(args)
    t = time.time()
    p = subprocess.run(cmd, capture_output=True, text=True)
    el = time.time() - t
    rc = p.returncode
    m = re.search(r"^s (\S+)", p.stdout, re.M)
    if m:
        st = m.group(1)
        codes = {"SATISFIABLE": (10,), "UNSATISFIABLE": (20,), "UNKNOWN": (124,)}.get(st, ())
        if rc not in codes:
            raise RuntimeError(f"solver answered {st} with exit code {rc}: {p.stderr[-300:]}")
        return ("TIMEOUT" if st == "UNKNOWN" else st), el, p.stdout
    if rc == 124:
        return "TIMEOUT", el, p.stdout
    if "memory allocation of" in p.stderr:
        return "MEMOUT", el, p.stdout
    raise RuntimeError(f"solver exit code {rc} without an answer: {p.stderr[-300:]}")


def total(values):
    """The sum, or None when any value is missing (-1): a missing counter is
    reported as missing, never as zero."""
    values = list(values)
    return None if any(v < 0 for v in values) else sum(values)


def solve(solver, path, limit, extra=(), mem_mb=MEM_MB):
    """(status, seconds, conflicts, true variables of the model or None, work)"""
    st, el, out = run_solver(solver, STOCK + list(extra) + [str(path)], limit, mem_mb)
    confl, work = workclock(out)
    return st, el, confl, model_of(out) if st == "SATISFIABLE" else None, work


def run_chain(net, pairs, check, jobs, window, progress=False):
    """Prove the pairs in order.  check(i, assumed) returns (status, seconds,
    conflicts, model, payload, work); a pair is proven when the solver refutes it
    assuming every proven pair before it.  The pending pairs are checked
    `window` at a time, each assuming the window's pairs before it; the first
    failure is dropped and the window's later pairs are checked again.  Every
    model is an input on which some pair differs: the circuit is simulated
    on it and every pending pair that differs there is dropped too.
    Returns (proven in order, dropped by a check, dropped by a model, results,
    spent): spent counts every check run, the ones run again included; a
    check without counters (killed before it printed them) is counted in
    missing_counters and left out of the totals."""
    proven, dropped, by_model, results = [], [], [], {}
    proven_set = set()
    spent = dict(calls=0, seconds=0.0, conflicts=0, work=0, missing_counters=0)
    alive = [True] * len(pairs)
    t0 = time.time()

    def refute(model):
        val = net.simulate({v: (1 if v in model else 0) for v in net.inputs}, 1)
        for i, (nm, x, y) in enumerate(pairs):
            if not alive[i]:
                continue
            if (val[abs(x)] ^ (x < 0)) != (val[abs(y)] ^ (y < 0)):
                if i in proven_set:
                    raise RuntimeError(f"proven pair {nm} differs on a model: the solver or the reading is wrong")
                alive[i] = False
                by_model.append(i)

    with ThreadPoolExecutor(jobs) as ex:
        while True:
            todo = [i for i in range(len(pairs)) if alive[i] and i not in results][:window]
            if not todo:
                break
            base = list(proven)
            while todo:
                got = list(ex.map(lambda t: check(todo[t], base + todo[:t]), range(len(todo))))
                for r in got:
                    spent["calls"] += 1
                    spent["seconds"] += r[1]
                    if r[2] < 0 or r[5] < 0:
                        spent["missing_counters"] += 1   # the totals then leave this check out
                    else:
                        spent["conflicts"] += r[2]
                        spent["work"] += r[5]
                bad = [t for t, r in enumerate(got) if r[0] != "UNSATISFIABLE"]
                first = bad[0] if bad else len(todo)
                for t in range(first):
                    results[todo[t]] = got[t]
                    proven.append(todo[t])
                    proven_set.add(todo[t])
                    base.append(todo[t])
                if not bad:
                    break
                results[todo[first]] = got[first]
                alive[todo[first]] = False
                dropped.append(todo[first])
                for t in bad:
                    if got[t][3] is not None:
                        refute(got[t][3])
                todo = [i for i in todo[first + 1:] if alive[i]]
            if progress:
                print(f"proven {len(proven)}, dropped {len(dropped)} by a check and {len(by_model)} by a model, "
                      f"{len(pairs) - len(proven) - len(dropped) - len(by_model)} left, {time.time() - t0:.0f} s",
                      file=sys.stderr, flush=True)
    spent["seconds"] = round(spent["seconds"], 2)
    return proven, dropped, by_model, results, spent


def cmd_chain(args):
    path = Path(args.file)
    refuse_holdout([path])
    cell = read_cell(path)
    if cell["kind"] is None:
        sys.exit(f"{path.name}: no divider or square-root function found")
    net, pairs = cell["net"], cell["pairs"]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    n_vars = max([net.header_vars] + [abs(lit) for c in net.clauses for lit in c])
    root_units = [i for i, c in enumerate(net.clauses) if len(set(c)) == 1 and abs(c[0]) == cell["root"]]
    circuit = [c for i, c in enumerate(net.clauses) if i not in set(root_units)]
    tmp = Path(tempfile.mkdtemp(prefix="chain-", dir=out))

    def eq(p):
        _, x, y = p
        return [(-x, y), (x, -y)]

    def check(i, assumed):
        nm, x, y = pairs[i]
        f = tmp / f"lemma{i:05d}.cnf"
        write_cnf(f, n_vars, circuit + [c for j in assumed for c in eq(pairs[j])] + [(x, y), (-x, -y)])
        r = solve(args.solver, f, args.limit, mem_mb=args.mem_mb)
        f.unlink()
        return r[0], r[1], r[2], r[3], None, r[4]

    t0 = time.time()
    proven, dropped, by_model, results, spent = run_chain(net, pairs, check, args.jobs, args.window, args.progress)
    os.rmdir(tmp)
    final = out / "miter_with_pairs.cnf"
    write_cnf(final, n_vars, list(net.clauses) + [c for i in proven for c in eq(pairs[i])])
    fin = solve(args.solver, final, args.limit, mem_mb=args.mem_mb)
    final.unlink()
    summary = dict(cell=path.name, pairs=len(pairs), proven=len(proven), dropped=len(dropped),
                   dropped_by_model=len(by_model), dropped_kinds=sorted({results[i][0] for i in dropped}),
                   proven_seconds=round(sum(results[i][1] for i in proven), 2),
                   proven_conflicts=total(results[i][2] for i in proven),
                   dropped_seconds=round(sum(results[i][1] for i in dropped), 2), all_checks=spent,
                   costliest=[(pairs[i][0], round(results[i][1], 2), results[i][2])
                              for i in sorted(proven, key=lambda i: -results[i][2])[:5]],
                   miter=dict(result=fin[0], seconds=round(fin[1], 3), conflicts=fin[2], work=fin[4]),
                   wall_seconds=round(time.time() - t0, 1))
    with open(out / "chain.json", "w") as f:
        json.dump(dict(summary=summary, proven=[pairs[i] for i in proven],
                       dropped=[pairs[i] for i in dropped], dropped_by_model=[pairs[i] for i in by_model],
                       results={pairs[i][0]: results[i][:3] for i in results}), f, indent=1)
    print(json.dumps(summary))
    return 0


def cmd_blif(args):
    path = Path(args.file)
    refuse_holdout([path])
    net = Netlist(path)
    root, _, _, _ = miter_sides(net)
    with open(args.out, "w") as f:
        f.write(".model miter\n.inputs " + " ".join(f"n{v}" for v in net.inputs) + "\n.outputs out\n")
        for v, b in sorted(net.const.items()):
            f.write(f".names n{v}\n" + ("1\n" if b else ""))
        for v in net.order:
            sup, tt = net.gates[v]
            f.write(".names " + " ".join(f"n{u}" for u in sup) + f" n{v}\n")
            for m in range(1 << len(sup)):
                if (tt >> m) & 1:
                    f.write("".join(str((m >> i) & 1) for i in range(len(sup))) + " 1\n")
        # the miter is satisfiable iff the root can take its asserted value
        f.write(f".names n{root} out\n" + ("1 1\n" if net.roots[root] else "0 1\n") + ".end\n")
    print(f"{args.out}: {len(net.inputs)} inputs, {len(net.gates)} gates")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--out")
    c.add_argument("files", nargs="*")
    ch = sub.add_parser("chain")
    ch.add_argument("file")
    ch.add_argument("--solver", required=True)
    ch.add_argument("--out", required=True)
    ch.add_argument("--jobs", type=int, default=8)
    ch.add_argument("--limit", type=float, default=300)
    ch.add_argument("--window", type=int, default=32)
    ch.add_argument("--progress", action="store_true")
    ch.add_argument("--mem-mb", type=int, default=MEM_MB)
    bl = sub.add_parser("blif")
    bl.add_argument("file")
    bl.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    return {"census": cmd_census, "chain": cmd_chain, "blif": cmd_blif}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
