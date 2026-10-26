#!/usr/bin/env python3
"""Sorter miter census (bead SAT-playground-skz.18).

The 2025 suite's sorter miters (BubbleVsPancakeSort_N_W and
PancakeVsSelectionSort_N_W: two sorting programs over the same N words of
W bits, every output bit compared) read back as word-level circuits.
Per cell:

  gates     the clauses whose largest variable is v are v's gate (the files
            list every gate after its inputs); each group is checked to be a
            definition by its truth table.  A unit on a variable that no
            group defines is a constant; the one unit on a gate is the
            miter's root.
  words     the N input words (W bits, least significant first: word k is
            variables kW+1 .. kW+W) and the words the exchange units make.
  exchange  W pairs of 2-to-1 multiplexers under one select s over two words
            X and Y:  C = s ? Y : X,  D = s ? X : Y.  Whatever s is,
            {C, D} = {X, Y}: no value is lost or copied.  A unit whose
            select compares its own X and Y is a compare-exchange
            (C = min, D = max, or the reverse); any other select is a
            conditional swap driven by index logic (a flip or a swap).
  muxes     a multiplexer with no partner (it could copy a value): counted,
            since it would break the multiset argument.
  compares  gates computing X > Y (or >=, <, <=) for two words, found by
            simulation over the gates between the two words' bits.
  index     every other gate of a side: index registers, loop conditions.
  miter     an XOR per output bit pair, an OR per output word, the OR of
            those asserted; which word of each side is compared with which.
  check     simulation on random words, with ties: each side's outputs are
            its inputs sorted, ascending by lane (input word k starts in
            lane k; C keeps X's lane, D keeps Y's), so the sides agree.

Subcommands:
  census [--out TABLE.tsv] [FILES...]     one row per cell (default: the
                                          2025 suite's *Sort* cells)
  lemmas FILE --out DIR                   CNFs for the hardness probes:
        side<S>_sorted<k>.cnf  side S alone plus "out_k > out_k+1" asserted
                               (unsatisfiable: the side sorts)
        pos<k>.cnf             the miter with only output word k compared

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import lzma
import random
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
HOLDOUT = ROOT / "benchmarks" / "sat-comp-2026"

MAX_SUPPORT = 10        # largest gate support read by truth table
SAMPLES = 512           # simulated input vectors (bit-parallel)
MAX_COMPARE_CONE = 400  # a compare's cone between its two words has at most this many gates


# ---------------------------------------------------------------------------
# reading
# ---------------------------------------------------------------------------


def read_cnf(path):
    opener = lzma.open if str(path).endswith(".xz") else open
    clauses = []
    n_vars = 0
    with opener(path, "rt") as f:
        for line in f:
            if not line.strip() or line[0] in "c%":
                continue
            if line[0] == "p":
                n_vars = int(line.split()[2])
                continue
            lits = [int(x) for x in line.split()]
            if lits and lits[-1] == 0:
                lits.pop()
            if lits:
                clauses.append(tuple(lits))
    return n_vars, clauses


def definition_tt(v, cls, sup):
    """Truth table of v over sup if the clauses define v there, else None."""
    idx = {u: i for i, u in enumerate(sup)}
    pos, neg = [], []
    for c in cls:
        others = []
        sign = 0
        for l in c:
            if abs(l) == v:
                sign = 1 if l > 0 else -1
            else:
                others.append((idx[abs(l)], l > 0))
        if sign == 0:
            return None  # a clause without v: a constraint, not a definition
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
    def __init__(self, n_vars, clauses):
        self.n_vars = n_vars
        self.clauses = clauses
        groups = defaultdict(list)
        units = {}
        for i, c in enumerate(clauses):
            if len(c) == 1:
                units[abs(c[0])] = c[0] > 0
            else:
                groups[max(abs(l) for l in c)].append(i)
        self.group = groups
        self.gates = {}
        self.not_gates = []
        for v, ids in groups.items():
            cls = [clauses[i] for i in ids]
            sup = tuple(sorted({abs(l) for c in cls for l in c} - {v}))
            tt = definition_tt(v, cls, sup) if len(sup) <= MAX_SUPPORT else None
            if tt is None:
                self.not_gates.append(v)
            else:
                self.gates[v] = (sup, tt)
        self.roots = {v: b for v, b in units.items() if v in self.gates}
        self.const = {v: b for v, b in units.items() if v not in self.gates}
        present = {abs(l) for c in clauses for l in c}
        self.primary = sorted(present - set(self.gates) - set(self.const))
        self.topo = all(all(u < v for u in sup) for v, (sup, tt) in self.gates.items())

    def simulate(self, inputs, mask):
        """inputs: var -> bit-parallel int; returns every signal's value."""
        val = dict(inputs)
        for v, b in self.const.items():
            val[v] = mask if b else 0
        for v in sorted(self.gates):
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


def ite_of(sup, tt):
    """(select, then, else) with out = s ? t : e, or None."""
    if len(sup) != 3:
        return None
    for p in range(3):
        q, r = [i for i in range(3) if i != p]
        for t, e in ((q, r), (r, q)):
            ok = True
            for m in range(8):
                want = (m >> t) & 1 if (m >> p) & 1 else (m >> e) & 1
                if ((tt >> m) & 1) != want:
                    ok = False
                    break
            if ok:
                return sup[p], sup[t], sup[e]
    return None


# ---------------------------------------------------------------------------
# word-level reading
# ---------------------------------------------------------------------------


class Words:
    """Words, exchange units, compares and the miter of one sorter cell."""

    def __init__(self, net: Netlist, n_words: int, width: int):
        self.net = net
        self.N, self.W = n_words, width
        self.alias = {}            # buffer output -> source
        self.word_of = {}          # var -> (word id, bit)
        self.words = []            # word id -> [vars by bit]
        self.lane = []             # word id -> lane
        self.made_by = []          # word id -> unit id or None
        self.units = []            # dict(sel, X, Y, C, D, kind)
        self.mux_bits = []         # unpartnered multiplexers
        self.inverted_data = 0
        prim = net.primary
        if len(prim) != n_words * width:
            raise ValueError(f"{len(prim)} primary inputs, expected {n_words}x{width}")
        for k in range(n_words):
            self._new_word(prim[k * width:(k + 1) * width], k, None)
        self._read_units()

    def _new_word(self, bits, lane, unit):
        wid = len(self.words)
        self.words.append(list(bits))
        self.lane.append(lane)
        self.made_by.append(unit)
        for i, v in enumerate(bits):
            self.word_of[v] = (wid, i)
        return wid

    def res(self, v):
        while v in self.alias:
            v = self.alias[v]
        return v

    def _read_units(self):
        net = self.net
        for v in sorted(net.gates):
            sup, tt = net.gates[v]
            if len(sup) == 1 and tt == 0b10:
                self.alias[v] = sup[0]
        # multiplexers by select and data pair
        slices = defaultdict(dict)   # (sel, frozenset(a, b)) -> {then var: gate}
        for v in sorted(net.gates):
            sup, tt = net.gates[v]
            r = ite_of(sup, tt)
            if r is None:
                continue
            s, t, e = (self.res(u) for u in r)
            slices[(s, frozenset((t, e)))][t] = v
        pending = defaultdict(dict)  # (sel, X, Y) -> {bit: (C gate, D gate)}
        todo = sorted(slices.items(), key=lambda kv: min(kv[1].values()))
        changed = True
        while changed:
            changed = False
            rest = []
            for (s, pair), by_then in todo:
                if len(by_then) != 2:
                    rest.append(((s, pair), by_then))
                    continue
                a, b = sorted(pair)
                if a not in self.word_of or b not in self.word_of:
                    rest.append(((s, pair), by_then))
                    continue
                (wa, ia), (wb, ib) = self.word_of[a], self.word_of[b]
                if ia != ib or wa == wb:
                    rest.append(((s, pair), by_then))
                    continue
                if wa > wb:
                    a, b, wa, wb = b, a, wb, wa
                # C = s ? Y : X (then = b) ; D = s ? X : Y (then = a)
                pending[(s, wa, wb)][ia] = (by_then[b], by_then[a])
                if len(pending[(s, wa, wb)]) == self.W:
                    bits = pending.pop((s, wa, wb))
                    uid = len(self.units)
                    c = self._new_word([bits[i][0] for i in range(self.W)], self.lane[wa], uid)
                    d = self._new_word([bits[i][1] for i in range(self.W)], self.lane[wb], uid)
                    self.units.append(dict(sel=s, X=wa, Y=wb, C=c, D=d, kind=None))
                    changed = True
            todo = rest
        self.leftover_slices = sum(len(b) for _, b in todo) + sum(2 * len(p) for p in pending.values())
        for (s, pair), by_then in todo:
            if len(by_then) == 1:
                self.mux_bits.extend(by_then.values())

    # -- compares ----------------------------------------------------------

    def compare_cone(self, g):
        """Words read by g through gates that are not word bits; None if g
        reads anything else or the cone is too big."""
        net = self.net
        seen = set()
        words = set()
        stack = [g]
        while stack:
            u = self.res(stack.pop())
            if u in self.word_of:
                words.add(self.word_of[u][0])
                continue
            if u in seen or u in net.const:
                continue
            if u not in net.gates:
                return None
            seen.add(u)
            if len(seen) > MAX_COMPARE_CONE:
                return None
            stack.extend(net.gates[u][0])
        return words, seen


def word_values(words, val, n_samples):
    """word id -> list of per-sample integer values."""
    out = []
    for bits in words:
        vals = [0] * n_samples
        for i, v in enumerate(bits):
            w = val[v]
            for j in range(n_samples):
                if (w >> j) & 1:
                    vals[j] |= 1 << i
        out.append(vals)
    return out


def mask_of(pred, xs, ys):
    m = 0
    for j, (x, y) in enumerate(zip(xs, ys)):
        if pred(x, y):
            m |= 1 << j
    return m


OPS = {">": lambda x, y: x > y, ">=": lambda x, y: x >= y, "<": lambda x, y: x < y, "<=": lambda x, y: x <= y}


def name_shape(path):
    m = re.search(r"([A-Za-z]+)Vs([A-Za-z]+)_(\d+)_(\d+)", Path(path).name)
    if not m:
        return None
    return m.group(1), m.group(2), int(m.group(3)), int(m.group(4))


def census_cell(path, rnd_seed=1):
    shape = name_shape(path)
    if shape is None:
        raise ValueError("not a <A>Vs<B>_<N>_<W> sorter cell")
    alg_a, alg_b, N, W = shape
    n_vars, clauses = read_cnf(path)
    net = Netlist(n_vars, clauses)
    row = dict(stem=Path(path).name.split("-", 1)[-1].replace(".cnf.xz", "").replace(".cnf", ""),
               vars=n_vars, clauses=len(clauses), N=N, W=W, gates=len(net.gates),
               not_gates=len(net.not_gates), consts=len(net.const), topo=int(net.topo),
               primary=len(net.primary))
    wd = Words(net, N, W)
    row["mux_bits"] = len(wd.mux_bits)
    row["leftover_slices"] = wd.leftover_slices

    # simulation: uniform words, words from a small range (ties), and blocks of equal words
    rnd = random.Random(rnd_seed)
    S = SAMPLES
    mask = (1 << S) - 1
    samples = []
    for j in range(S):
        mode = j % 3
        if mode == 0:
            samples.append([rnd.randrange(1 << W) for _ in range(N)])
        elif mode == 1:
            lo = rnd.randrange(1 << W)
            samples.append([min((1 << W) - 1, lo + rnd.randrange(3)) for _ in range(N)])
        else:
            pool = [rnd.randrange(1 << W) for _ in range(2)]
            samples.append([rnd.choice(pool) for _ in range(N)])
    inputs = {}
    for k in range(N):
        for i, v in enumerate(wd.words[k]):
            w = 0
            for j in range(S):
                if (samples[j][k] >> i) & 1:
                    w |= 1 << j
            inputs[v] = w
    val = net.simulate(inputs, mask)
    for v in list(wd.alias):
        val[v] = val[wd.res(v)]
    wv = word_values(wd.words, val, S)

    # exchange units: compare-exchange or conditional swap
    for u in wd.units:
        s = val[u["sel"]]
        xs, ys = wv[u["X"]], wv[u["Y"]]
        kind = "CS"
        for op, f in OPS.items():
            m = mask_of(f, xs, ys)
            if s == m or s == (~m & mask):
                kind = "CE"
                break
        u["kind"] = kind

    # the miter: XORs over a bit pair of two words, ORs per word pair
    xor_pairs = defaultdict(set)   # (word a, word b) -> bits
    for v, (sup, tt) in net.gates.items():
        if len(sup) == 2 and tt in (0b0110, 0b1001):
            a, b = (wd.res(x) for x in sup)
            if a in wd.word_of and b in wd.word_of:
                (wa, ia), (wb, ib) = wd.word_of[a], wd.word_of[b]
                if ia == ib and wa != wb:
                    xor_pairs[tuple(sorted((wa, wb)))].add(ia)
    full_pairs = [p for p, bits in xor_pairs.items() if len(bits) == W]
    row["compared_words"] = len(full_pairs)
    row["roots"] = len(net.roots)

    # sides: the data-flow cone of each compared word
    def data_cone(wid):
        cone = set()
        stack = [wid]
        while stack:
            w = stack.pop()
            u = wd.made_by[w]
            if u is None or u in cone:
                continue
            cone.add(u)
            stack.extend((wd.units[u]["X"], wd.units[u]["Y"]))
        return cone

    side_of_word = {}
    sides = []  # list of (set of output words, set of units)
    for a, b in full_pairs:
        for w in (a, b):
            cw = data_cone(w)
            placed = False
            for si, (outs, us) in enumerate(sides):
                if cw & us or (not cw and not us):
                    outs.add(w)
                    us |= cw
                    side_of_word[w] = si
                    placed = True
                    break
            if not placed:
                sides.append(({w}, set(cw)))
                side_of_word[w] = len(sides) - 1
    row["sides"] = len(sides)
    pair_sides_ok = all(side_of_word[a] != side_of_word[b] for a, b in full_pairs)

    # compares: gates whose cone between word bits reads exactly two words
    compare_of = {}
    for v in sorted(net.gates):
        if wd.res(v) != v or v in wd.word_of:
            continue
        r = wd.compare_cone(v)
        if r is None:
            continue
        ws, cone = r
        if len(ws) != 2:
            continue
        x, y = sorted(ws)
        for op, f in OPS.items():
            m = mask_of(f, wv[x], wv[y])
            if val[v] == m or val[v] == (~m & mask):
                compare_of[v] = (x, y, op)
                break
    # a compare root: a compare gate read by something that is not a gate of the same compare
    readers = defaultdict(set)
    for v, (sup, tt) in net.gates.items():
        for u in sup:
            readers[wd.res(u)].add(v)
    roots = []
    for v, (x, y, op) in compare_of.items():
        outside = [r for r in readers[v] if compare_of.get(r, (None, None))[:2] != (x, y)]
        if outside or v in net.roots:
            roots.append(v)
    sel_set = {u["sel"] for u in wd.units}

    # per side: units, compares and other gates in its logic cone
    def logic_cone(out_words):
        seen = set()
        stack = [v for w in out_words for v in wd.words[w]]
        while stack:
            u = wd.res(stack.pop())
            if u in seen or u not in net.gates:
                continue
            seen.add(u)
            stack.extend(net.gates[u][0])
        return seen

    side_rows = []
    sorted_ok = []
    for si, (outs, us) in enumerate(sides):
        cone = logic_cone(outs)
        ce = sum(1 for u in us if wd.units[u]["kind"] == "CE")
        cs = len(us) - ce
        unit_gates = {v for u in us for w in (wd.units[u]["C"], wd.units[u]["D"]) for v in wd.words[w]}
        cmp_gates = set()
        cmp_roots = [v for v in roots if v in cone]
        for v in cmp_roots:
            cmp_gates |= wd.compare_cone(v)[1]
        cmp_on_inputs = sum(1 for v in cmp_roots if max(compare_of[v][:2]) < N)
        cmp_as_sel = sum(1 for v in cmp_roots if v in sel_set)
        index = cone - unit_gates - cmp_gates - set(wd.alias)
        # lanes and the sort check
        by_lane = {}
        for w in outs:
            by_lane.setdefault(wd.lane[w], []).append(w)
        lanes_ok = sorted(by_lane) == list(range(N)) and all(len(ws) == 1 for ws in by_lane.values())
        ok = lanes_ok
        if lanes_ok:
            order = [by_lane[k][0] for k in range(N)]
            for j in range(S):
                got = [wv[w][j] for w in order]
                if got != sorted(samples[j]):
                    ok = False
                    break
        sorted_ok.append(ok)
        side_rows.append(f"side{si}:units={len(us)}(ce={ce},cs={cs}),compares={len(cmp_roots)}"
                         f"(on_inputs={cmp_on_inputs},as_select={cmp_as_sel}),cmp_gates={len(cmp_gates)},"
                         f"index_gates={len(index)},cone={len(cone)},lanes_ok={int(lanes_ok)},sorts={int(ok)}")
    row["side_desc"] = " | ".join(side_rows)
    row["pairs_cross_sides"] = int(pair_sides_ok)
    row["both_sort"] = int(len(sorted_ok) == 2 and all(sorted_ok))
    # within a side, each word read as data by at most one unit (the network
    # shape the multiset argument needs; the input words are read by both sides)
    worst = 0
    for outs, us in sides:
        data_readers = defaultdict(int)
        for u in us:
            data_readers[wd.units[u]["X"]] += 1
            data_readers[wd.units[u]["Y"]] += 1
        worst = max([worst] + list(data_readers.values()))
    row["max_data_readers"] = worst
    row["units"] = len(wd.units)
    row["algs"] = f"{alg_a}/{alg_b}"
    return row, net, wd, sides, full_pairs, side_of_word


COLUMNS = ["stem", "algs", "N", "W", "vars", "clauses", "gates", "not_gates", "consts", "topo", "primary", "units",
           "mux_bits", "leftover_slices", "max_data_readers", "compared_words", "roots", "sides", "pairs_cross_sides",
           "both_sort", "side_desc"]


def cmd_census(args):
    files = [Path(f) for f in args.files] if args.files else sorted(SUITE.glob("*Sort*.cnf.xz"))
    rows = []
    for f in files:
        if HOLDOUT in f.resolve().parents:
            print(f"skip {f.name}: the 2026 suite is the holdout", file=sys.stderr)
            continue
        try:
            row = census_cell(f)[0]
        except Exception as e:  # noqa: BLE001
            row = dict(stem=f.name, side_desc=f"error: {type(e).__name__}: {e}")
        rows.append(row)
        print("\t".join(str(row.get(c, "")) for c in COLUMNS), flush=True)
    if args.out:
        with open(args.out, "w", newline="") as fo:
            w = csv.DictWriter(fo, COLUMNS, delimiter="\t", extrasaction="ignore")
            w.writeheader()
            w.writerows(rows)
    return 0


# ---------------------------------------------------------------------------
# lemma CNFs for the hardness probes
# ---------------------------------------------------------------------------


def cone_clauses(net, wd, out_vars):
    """Clause ids of every gate in the cone of out_vars, plus constant units."""
    seen = set()
    stack = list(out_vars)
    while stack:
        u = stack.pop()
        if u in seen or u not in net.gates:
            continue
        seen.add(u)
        stack.extend(net.gates[u][0])
    ids = [i for v in seen for i in net.group[v]]
    ids += [i for i, c in enumerate(net.clauses) if len(c) == 1 and abs(c[0]) in net.const]
    return sorted(ids)


def write_cnf(path, n_vars, clauses):
    with open(path, "w") as f:
        f.write(f"p cnf {n_vars} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


def gt_encoding(a_bits, b_bits, next_var):
    """Tseitin clauses for g <-> (a > b), LSB-first bit lists; returns (g, clauses, next_var).
    e_i <-> (a_i == b_i); t_i <-> a_i & ~b_i & e_{i+1..}; g <-> OR t_i."""
    W = len(a_bits)
    cls = []

    def new():
        nonlocal next_var
        next_var += 1
        return next_var

    eq = {}
    for i in range(W):
        e = new()
        a, b = a_bits[i], b_bits[i]
        cls += [[-e, -a, b], [-e, a, -b], [e, a, b], [e, -a, -b]]
        eq[i] = e
    ts = []
    for i in range(W - 1, -1, -1):
        t = new()
        conj = [a_bits[i], -b_bits[i]] + [eq[j] for j in range(i + 1, W)]
        for l in conj:
            cls.append([-t, l])
        cls.append([t] + [-l for l in conj])
        ts.append(t)
    g = new()
    for t in ts:
        cls.append([g, -t])
    cls.append([-g] + ts)
    return g, cls, next_var


def cmd_lemmas(args):
    path = Path(args.file)
    if HOLDOUT in path.resolve().parents:
        print("the 2026 suite is the holdout", file=sys.stderr)
        return 2
    row, net, wd, sides, full_pairs, side_of_word = census_cell(path)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    N = wd.N
    # side outputs by lane
    for si, (outs, us) in enumerate(sides):
        by_lane = {wd.lane[w]: w for w in outs}
        ids = cone_clauses(net, wd, [v for w in outs for v in wd.words[w]])
        base = [net.clauses[i] for i in ids]
        for k in range(N - 1):
            a, b = wd.words[by_lane[k]], wd.words[by_lane[k + 1]]
            g, cls, nv = gt_encoding([wd.res(v) for v in a], [wd.res(v) for v in b], net.n_vars)
            write_cnf(out / f"side{si}_sorted{k}.cnf", nv, base + cls + [[g]])
    # per-position miters: drop the root unit, assert the OR over word k's XORs
    root = next(iter(net.roots))
    keep = [c for c in net.clauses if not (len(c) == 1 and abs(c[0]) == root)]
    lane_pairs = []
    for a, b in full_pairs:
        lane_pairs.append((wd.lane[a], a, b))
    for k, a, b in sorted(lane_pairs):
        xs = [wd.res(v) for v in wd.words[a]]
        ys = [wd.res(v) for v in wd.words[b]]
        nv = net.n_vars
        extra = []
        ds = []
        for x, y in zip(xs, ys):
            nv += 1
            d = nv
            extra += [[-d, x, y], [-d, -x, -y], [d, -x, y], [d, x, -y]]
            ds.append(d)
        extra.append(ds)
        write_cnf(out / f"pos{k}.cnf", nv, keep + extra)
    print(f"{row['stem']}: wrote {len(sides) * (N - 1)} sortedness lemmas and {len(lane_pairs)} position miters to {out}")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("files", nargs="*")
    c.add_argument("--out")
    c.set_defaults(func=cmd_census)
    lm = sub.add_parser("lemmas")
    lm.add_argument("file")
    lm.add_argument("--out", required=True)
    lm.set_defaults(func=cmd_lemmas)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
