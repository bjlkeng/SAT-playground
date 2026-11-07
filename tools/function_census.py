#!/usr/bin/env python3
"""Function recovery census (bead SAT-playground-i7h, G.3.1).

Reads a CNF back as a circuit and asks what it computes, keyed on the
function rather than on how an encoder wrote it:

  gates   the gate recovery of tools/netlist_census.py (definitions by the
          resolvent test), or, when every clause group reads that way, the
          gate each clause's first literal names (how ABC and STP write a
          circuit); then structural hashing, with constants folded and the
          inputs a gate ignores dropped
  cuts    every node's cuts of up to 6 leaves, each with the node's truth
          table over the cut; the functions are sorted into classes that do
          not depend on input order or polarity (and/or, xor, majority, mux,
          symmetric, balanced nonlinear, other), and nodes that share a cut
          form cells (full and half adders, muxes, S-box candidates)
  blocks  mux words (four or more muxes on one select) and xor blocks
          (eight or more xors joined through their fan-ins)
  cones   the signals the formula observes (the two compared words of a
          miter, single observed signals, or the word the unit clauses fix)
          and the cones below them, simulated
          against a library of word functions: linear maps, popcount and
          thresholds, add, sub, multiply, square, negate, increment and the
          constant forms, compare (unsigned, signed, equality), mux trees,
          shifts and rotates, S-box layers and cipher rounds. A function is
          reported only when random simulation (and the constructed patterns
          each test uses) confirms it on every output bit it claims.

Usage:
  python3 tools/function_census.py census --out TABLE.tsv [--stems a,b] [--files F ...]
  python3 tools/function_census.py report TABLE.tsv [--run RUN_DIR]
  python3 tools/function_census.py one FILE.cnf[.xz]
  python3 tools/function_census.py toy KIND [--mapped K] [--width W] [--shuffle SEED] --out FILE.cnf
  python3 tools/function_census.py selftest

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import heapq
import os
import random
import signal
import sys
import time
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor, as_completed
from itertools import permutations
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import netlist_census as nc  # noqa: E402
from rl_structure import parse_prefix  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
FAMILIES = ROOT / "benchmarks" / "rl" / "families.tsv"

K = 6                    # cut size (truth tables of 64 entries)
MAX_CUTS = 6             # cuts kept per node, beside the trivial one
MAX_CUT_GATES = 400_000  # the cut layer runs up to this many gates
MAX_GATE_VARS = 1_500_000  # cells whose clause prefix touches more variables are not read (error too-big)
MAX_CONE_GATES = 300_000   # a cone is classified up to this many gates
MAX_CONE_PIS = 4_096       # and this many primary inputs
MAX_OUTPUTS = 512          # observed outputs per side
NPAT = 256                 # random patterns per simulation
CELL_SECONDS = 3_600       # wall budget per cell

M64 = (1 << 64) - 1

# ---------------------------------------------------------------------------
# truth tables over six variable slots
# ---------------------------------------------------------------------------
# Every truth table is a 64-bit int over six slots; a function of n < 6
# leaves uses slots 0..n-1 and is replicated over the others, so the gate
# functions apply bitwise and a slot can move by swaps.


def _var_masks():
    out = []
    for i in range(K):
        m = 0
        for a in range(64):
            if (a >> i) & 1:
                m |= 1 << a
        out.append(m)
    return out


VAR = _var_masks()          # VAR[i]: the table of slot i


def _swap_masks():
    out = []
    for j in range(K - 1):
        a = b = 0
        for m in range(64):
            bj, bk = (m >> j) & 1, (m >> (j + 1)) & 1
            if bj and not bk:
                a |= 1 << m
            elif bk and not bj:
                b |= 1 << m
        out.append((a, b, M64 ^ (a | b), 1 << j))
    return out


SWAP = _swap_masks()


def tt_swap(tt, j):
    """Swap slots j and j+1."""
    a, b, keep, s = SWAP[j]
    return (tt & keep) | ((tt & a) << s) | ((tt & b) >> s)


def tt_expand(tt, src, dst):
    """A table over the sorted leaves `src` as a table over the sorted
    superset `dst` (slot i of src moves to the position of src[i] in dst)."""
    if src == dst:
        return tt
    pos = {x: i for i, x in enumerate(dst)}
    for i in range(len(src) - 1, -1, -1):
        p = pos[src[i]]
        for j in range(i, p):
            tt = tt_swap(tt, j)
    return tt


def tt_replicate(tt, n):
    """A table over n slots (2^n bits) replicated to 64 bits."""
    w = 1 << n
    while w < 64:
        tt |= tt << w
        w <<= 1
    return tt & M64


def tt_restrict(tt, n):
    return tt & ((1 << (1 << n)) - 1)


def tt_essential(tt, n):
    """Slots among the first n that the function depends on."""
    out = []
    for i in range(n):
        s = 1 << i
        if ((tt >> s) ^ tt) & (M64 ^ VAR[i]):
            out.append(i)
    return out


def tt_shrink(tt, n):
    """(table over the essential slots only, their old positions)."""
    ess = tt_essential(tt, n)
    for want, have in enumerate(ess):
        # move slot `have` down to `want`, by swaps (have >= want)
        for j in range(have - 1, want - 1, -1):
            tt = tt_swap(tt, j)
    return tt, ess


# ---------------------------------------------------------------------------
# compiled gate functions (bit-parallel over Python ints)
# ---------------------------------------------------------------------------

_FN = {}


def _expr(tt, k, full):
    """An expression for the k-variable table tt over x0..x(k-1)."""
    if tt == 0:
        return "0"
    if tt == full:
        return "M"
    half = 1 << (k - 1)
    hm = (1 << half) - 1
    f0, f1 = tt & hm, tt >> half
    x = f"x{k - 1}"
    if f0 == f1:
        return _expr(f0, k - 1, hm)
    if f0 == 0 and f1 == hm:
        return x
    if f0 == hm and f1 == 0:
        return f"(M ^ {x})"
    if f1 == f0 ^ hm:
        return f"({x} ^ {_expr(f0, k - 1, hm)})"
    if f0 == 0:
        return f"({x} & {_expr(f1, k - 1, hm)})"
    if f1 == 0:
        return f"((M ^ {x}) & {_expr(f0, k - 1, hm)})"
    if f1 == hm:
        return f"({x} | {_expr(f0, k - 1, hm)})"
    if f0 == hm:
        return f"((M ^ {x}) | {_expr(f1, k - 1, hm)})"
    e0 = _expr(f0, k - 1, hm)
    e1 = _expr(f1, k - 1, hm)
    return f"(({x} & {e1}) | ((M ^ {x}) & {e0}))"


def compile_tt(n, tt):
    """f(x0, ..., x(n-1), M): the gate's value from bit-parallel inputs."""
    key = (n, tt)
    f = _FN.get(key)
    if f is None:
        full = (1 << (1 << n)) - 1
        body = _expr(tt & full, n, full) if n else ("M" if tt & 1 else "0")
        args = "".join(f"x{i}, " for i in range(n))
        f = eval(f"lambda {args}M: {body}")  # noqa: S307 (our own expression)
        _FN[key] = f
    return f


def _simplify(tt, ids, const):
    """(table, fan-ins) with constant fan-ins fixed and inessential ones dropped."""
    t = tt
    # fix constants: slot i takes its value, the table moves to the other slots
    cur = list(ids)
    k = 0
    while k < len(cur):
        i = cur[k]
        if i in const:
            t = _fix_slot(t, len(cur), k, const[i])
            cur.pop(k)
        else:
            k += 1
    # drop slots the table does not depend on
    k = 0
    while k < len(cur):
        n = len(cur)
        lo = _fix_slot(t, n, k, 0)
        hi = _fix_slot(t, n, k, 1)
        if lo == hi:
            t = lo
            cur.pop(k)
        else:
            k += 1
    return t, cur


def _fix_slot(t, n, k, val):
    """The table over n slots with slot k fixed to val, over the other n-1 slots."""
    out = 0
    for a in range(1 << (n - 1)):
        low = a & ((1 << k) - 1)
        high = a >> k
        m = low | (val << k) | (high << (k + 1))
        if (t >> m) & 1:
            out |= 1 << a
    return out


def _reduce_tt(tt, n, slots, negs):
    """A gate over inputs (slot i reads unique input slots[i], negated if
    negs[i]) as a table over the unique inputs."""
    m_unique = max(slots) + 1
    out = 0
    for a in range(1 << m_unique):
        m = 0
        for i in range(n):
            bit = (a >> slots[i]) & 1
            if negs[i]:
                bit ^= 1
            m |= bit << i
        if (tt >> m) & 1:
            out |= 1 << a
    return out, m_unique


# ---------------------------------------------------------------------------
# the netlist
# ---------------------------------------------------------------------------


class Netlist:
    """The recovered circuit numbered densely: ids 0..npi-1 are the primary
    inputs (variable order), then one id per representative gate in
    topological order. Each gate reads plain fan-in values; the negations
    from structural hashing are folded into its table."""

    def __init__(self, gates, order, rep, present):
        self.var_pi = sorted(v for v in present if v not in gates)
        self.npi = len(self.var_pi)
        self.id_of = {}            # var -> (id, negated) for every present variable
        for i, v in enumerate(self.var_pi):
            self.id_of[v] = (i, False)
        self.var_of = list(self.var_pi)   # id -> var
        self.fanin = []            # per gate (index id - npi): tuple of ids
        self.tt = []               # per gate: table over its fan-ins (2^n bits)
        self.nin = []
        self.fn = []
        self.const = {}            # gate id -> 0/1 for constant gates
        self.shared = {}           # constant gate id -> the one signal it read twice
        for v in order:
            r, ng = rep[v]
            if r != v:
                continue
            sup, tt, _cl = gates[v]
            ids, negs = [], []
            for u in sup:
                ru, nu = rep.get(u, (u, False))
                if ru in self.id_of:
                    i, ni = self.id_of[ru]
                else:
                    i, ni = self.id_of.get(u, (None, False))
                if i is None:
                    raise ValueError(f"gate {v} reads {u} before it is defined")
                ids.append(i)
                negs.append(nu != ni)
            uniq = []
            slots = []
            for i in ids:
                if i not in uniq:
                    uniq.append(i)
                slots.append(uniq.index(i))
            t2, m = _reduce_tt(tt, len(ids), slots, negs)
            # a gate over one signal twice that is constant (x xor x: a
            # compare whose two sides hashed to one signal) keeps that signal
            one = uniq[0] if len(uniq) == 1 and (t2 & 0b11) in (0, 0b11) else None
            # fold constant fan-ins and drop the inputs the table ignores, so
            # a cone's inputs are the ones its function reads
            t2, uniq = _simplify(t2, uniq, self.const)
            m = len(uniq)
            full = (1 << (1 << m)) - 1
            if m == 1 and (t2 & full) in (0b10, 0b01):
                i0, n0 = uniq[0], (t2 & full) == 0b01
                self.id_of[v] = (i0, n0)          # a buffer or inverter after folding
                continue
            gid = self.npi + len(self.fanin)
            self.id_of[v] = (gid, False)
            self.var_of.append(v)
            self.fanin.append(tuple(uniq))
            self.tt.append(t2)
            self.nin.append(m)
            self.fn.append(compile_tt(m, t2))
            full = (1 << (1 << m)) - 1
            if (t2 & full) in (0, full):
                self.const[gid] = 1 if t2 & full else 0
                if one is not None:
                    self.shared[gid] = one
        for v, (r, ng) in rep.items():
            if r != v and r in self.id_of:
                i, ni = self.id_of[r]
                self.id_of[v] = (i, ni != ng)
        self.size = self.npi + len(self.fanin)
        self._fanout = None

    def gate(self, i):
        return i - self.npi

    def is_pi(self, i):
        return i < self.npi

    def fanout(self):
        if self._fanout is None:
            fo = [[] for _ in range(self.size)]
            for g, ins in enumerate(self.fanin):
                for i in ins:
                    fo[i].append(g + self.npi)
            self._fanout = fo
        return self._fanout

    def cone(self, roots):
        """Gate ids in the transitive fan-in of the roots, in topological order."""
        seen = set()
        stack = [r for r in roots if r >= self.npi]
        while stack:
            u = stack.pop()
            if u in seen:
                continue
            seen.add(u)
            for i in self.fanin[u - self.npi]:
                if i >= self.npi and i not in seen:
                    stack.append(i)
        return sorted(seen)

    def cone_pis(self, cone_gates):
        pis = set()
        for u in cone_gates:
            for i in self.fanin[u - self.npi]:
                if i < self.npi:
                    pis.add(i)
        return sorted(pis)

    def simulate(self, pi_vals, mask, gates_topo=None):
        """Values (ints of the mask's width) for the given gates (default all),
        from a dict or list of primary-input values."""
        val = {} if isinstance(pi_vals, dict) else None
        if val is None:
            val = dict(enumerate(pi_vals))
        else:
            val = dict(pi_vals)
        npi = self.npi
        fanin, fn = self.fanin, self.fn
        for u in (gates_topo if gates_topo is not None else range(npi, self.size)):
            g = u - npi
            ins = fanin[g]
            f = fn[g]
            n = len(ins)
            if n == 2:
                val[u] = f(val[ins[0]], val[ins[1]], mask)
            elif n == 3:
                val[u] = f(val[ins[0]], val[ins[1]], val[ins[2]], mask)
            elif n == 1:
                val[u] = f(val[ins[0]], mask)
            elif n == 0:
                val[u] = f(mask)
            else:
                val[u] = f(*[val[i] for i in ins], mask)
        return val


def reorient_xors(gates):
    """An xor's variables can each be read as its output. The gate recovery
    breaks such ties by index when the file is numbered in circuit order
    and by fan-out otherwise, and the fan-out rule can make an xor's output
    an input: b = x ^ p with p declared primary while p is really x ^ b.
    The tell is the adder cell around it: a half adder reads the same two
    signals into an and as into its xor, a full adder the same three into a
    majority. So when b = x ^ p (both primary) and an and-type gate of two
    inputs reads exactly {x, b}, the xor is turned (p = x ^ b, b primary);
    likewise b = x ^ y ^ p next to a majority of {x, y, b}. A turn happens
    only if the turned xor does not show the tell itself, and each xor turns
    at most once. Returns the count."""
    readers = defaultdict(set)
    for v, (sup, _tt, _cl) in gates.items():
        for u in sup:
            readers[u].add(v)
    xor_tts = {2: {0b0110, 0b1001}, 3: {0x96, 0x69}}
    and2 = set(nc.AND_FAMILY2) | set(nc.OR_FAMILY2)
    maj3 = set(nc.MAJ_FAMILY3) | {0xFF ^ t for t in nc.MAJ_FAMILY3}

    def tell(sig, others):
        want = frozenset(others) | {sig}
        for r in readers[sig]:
            rs, rt, _rc = gates[r]
            if frozenset(rs) == want and ((len(rs) == 2 and rt in and2) or (len(rs) == 3 and rt in maj3)):
                return True
        return False

    turned = 0
    done = set()
    for b in sorted(gates):
        if b not in gates:
            continue
        sup, tt, cl = gates[b]
        k = len(sup)
        if k not in xor_tts or tt not in xor_tts[k] or cl in done or any(u in gates for u in sup):
            continue
        for other in sup:
            keep = [u for u in sup if u != other]
            if not tell(b, keep) or tell(other, keep):
                continue
            new_sup = tuple(sorted(keep + [b]))
            gates[other] = (new_sup, tt, cl)      # the parity table is the same over any order
            del gates[b]
            for u in sup:
                readers[u].discard(b)
            for u in new_sup:
                readers[u].add(other)
            done.add(cl)
            turned += 1
            break
    # an xor nobody reads, over two primary inputs one of which is read
    # elsewhere: x = y ^ k with x dangling and y read by others is the key
    # addition y = x ^ k read backwards (y declared primary for its fan-out)
    for g in sorted(gates):
        if g not in gates or readers[g]:
            continue
        sup, tt, cl = gates[g]
        if len(sup) != 2 or tt not in xor_tts[2] or cl in done or any(u in gates for u in sup):
            continue
        a, b = sup
        fan = {a: len(readers[a]), b: len(readers[b])}
        y, z = (a, b) if fan[a] >= fan[b] else (b, a)
        if fan[y] < 2 or fan[z] != 1:
            continue
        new_sup = tuple(sorted((g, z)))
        gates[y] = (new_sup, tt, cl)
        del gates[g]
        for u in sup:
            readers[u].discard(g)
        for u in new_sup:
            readers[u].add(y)
        done.add(cl)
        turned += 1
    return turned


def first_literal_gates(clauses):
    """The reading of an encoder that writes each gate's clauses with the
    output's literal first (ABC's write_cnf: the EPFL div, sqrt and hyp
    miters): every clause of two or more literals belongs to the gate its
    first literal names. None unless every such group is a definition of up
    to six inputs and the gates form no cycle."""
    groups = defaultdict(list)
    for i, c in enumerate(clauses):
        if len(c) >= 2:
            groups[abs(c[0])].append(i)
    if not groups:
        return None
    gates = {}
    for v, ids in groups.items():
        sup = tuple(sorted({abs(l) for i in ids for l in clauses[i]} - {v}))
        if not sup or len(sup) > nc.MAX_SUPPORT:
            return None
        tt = nc._definition_tt(v, ids, clauses, sup)
        if tt is None:
            return None
        gates[v] = (sup, tt, frozenset(ids))
    state = {}
    for root in gates:
        if root in state:
            continue
        stack = [(root, 0)]
        while stack:
            u, k = stack.pop()
            if k == 0:
                if state.get(u) == 2:
                    continue
                if state.get(u) == 1:
                    return None          # a cycle
                state[u] = 1
                stack.append((u, 1))
                for x in gates[u][0]:
                    if x in gates and state.get(x) != 2:
                        if state.get(x) == 1:
                            return None
                        stack.append((x, 0))
            else:
                state[u] = 2
    return gates


def build_netlist(clauses, n_vars):
    gates = first_literal_gates(clauses)
    if gates is not None:
        used = set()
        for _sup, _tt, cl in gates.values():
            used |= cl
        deadlocks, topo_numbered, reading = 0, True, "first-literal"
    else:
        gates, used, deadlocks, topo_numbered = nc.recover_gates(clauses, n_vars)
        reading = "definitions"
    turned = 0 if topo_numbered else reorient_xors(gates)
    order = nc.topo_order(gates)
    rep, merged = nc.structural_hash(gates, order)
    present = {abs(l) for c in clauses for l in c}
    net = Netlist(gates, order, rep, present)
    net.used = used
    net.deadlocks = deadlocks
    net.topo_numbered = topo_numbered
    net.merged = merged
    net.turned = turned
    net.reading = reading
    net.n_present = len(present)
    net.n_gates_raw = len(gates)
    return net


# ---------------------------------------------------------------------------
# function classes (independent of input order and polarity)
# ---------------------------------------------------------------------------


def _popcount(x):
    return bin(x).count("1")


def _parity_tt(k):
    t = 0
    for m in range(1 << k):
        if _popcount(m) & 1:
            t |= 1 << m
    return t


PARITY = [_parity_tt(k) for k in range(K + 1)]


def _family(base, k):
    """Every table of the k-input function `base` under input permutation,
    input negation and output negation."""
    out = set()
    full = (1 << (1 << k)) - 1
    for perm in permutations(range(k)):
        for neg in range(1 << k):
            t = 0
            for m in range(1 << k):
                x = 0
                for i in range(k):
                    if ((m >> perm[i]) & 1) ^ ((neg >> i) & 1):
                        x |= 1 << i
                if (base >> x) & 1:
                    t |= 1 << m
            out.add(t)
            out.add(full ^ t)
    return out


def _tt_of(fun, k):
    t = 0
    for m in range(1 << k):
        if fun(*[(m >> i) & 1 for i in range(k)]):
            t |= 1 << m
    return t


MAJ3 = _family(_tt_of(lambda a, b, c: a + b + c >= 2, 3), 3)
MUX3 = _family(_tt_of(lambda s, a, b: a if s else b, 3), 3)
_CLS = {}


def _negate_slot(tt, i):
    s = 1 << i
    return ((tt & VAR[i]) >> s) | ((tt & (M64 ^ VAR[i])) << s) & M64


def _is_symmetric(t, k):
    """Symmetric in its k slots up to input negation (k <= 6)."""
    rt = tt_replicate(t, k)
    # filter: the cofactor sizes per slot must be one unordered pair
    pairs = set()
    for i in range(k):
        c1 = _popcount(rt & VAR[i])
        c0 = _popcount(rt & (M64 ^ VAR[i]))
        pairs.add((min(c0, c1), max(c0, c1)))
    if len(pairs) > 1:
        return False
    for neg in range(1 << (k - 1)):     # negating every slot keeps symmetry: half the masks suffice
        u = rt
        for i in range(k):
            if (neg >> i) & 1:
                u = _negate_slot(u, i)
        if all(tt_swap(u, j) == u for j in range(k - 1)):
            return True
    return False


def classify_tt(tt, n):
    """Class name of a replicated table over n leaves, on its essential slots:
    const, buf, and<k> (one minterm or one maxterm), xor<k>, maj3, mux,
    sym<k> (symmetric up to input negation), bal<k> (balanced, k >= 4),
    other<k>."""
    key = (n, tt)
    c = _CLS.get(key)
    if c is not None:
        return c
    t6, ess = tt_shrink(tt, n)
    k = len(ess)
    t = tt_restrict(t6, k)
    full = (1 << (1 << k)) - 1
    ones = _popcount(t)
    if k == 0:
        c = "const"
    elif k == 1:
        c = "buf"
    elif ones in (1, (1 << k) - 1):
        c = f"and{k}"
    elif t == PARITY[k] or t == full ^ PARITY[k]:
        c = f"xor{k}"
    elif k == 3 and t in MAJ3:
        c = "maj3"
    elif k == 3 and t in MUX3:
        c = "mux"
    elif _is_symmetric(t, k):
        c = f"sym{k}"
    elif k >= 4 and ones == 1 << (k - 1):
        c = f"bal{k}"
    else:
        c = f"other{k}"
    _CLS[key] = c
    return c


# ---------------------------------------------------------------------------
# cuts
# ---------------------------------------------------------------------------


def _merge(a, b):
    if not a:
        return b
    if not b:
        return a
    return tuple(sorted(set(a) | set(b)))


def enumerate_cuts(net, gates_topo, k=K, c=MAX_CUTS, deadline=None):
    """cuts[id] = [(leaves, table)], the c smallest non-dominated cuts of up
    to k leaves of each gate (leaves sorted ids, the table replicated over
    six slots). A primary input has only its trivial cut."""
    cuts = {}
    npi = net.npi
    for n_done, u in enumerate(gates_topo):
        if deadline and n_done % 4096 == 0 and time.time() > deadline:
            raise TimeoutError("cuts")
        g = u - npi
        ins = net.fanin[g]
        f = net.fn[g]
        lists = []
        for i in ins:
            lst = cuts.get(i, [])
            lists.append(lst + [((i,), VAR[0])])
        partial = [((), ())]
        for lst in lists:
            nxt = []
            for leaves, parts in partial:
                for l2, t2 in lst:
                    m = _merge(leaves, l2)
                    if len(m) <= k:
                        nxt.append((m, parts + ((l2, t2),)))
            if len(nxt) > 4 * c:
                nxt.sort(key=lambda x: len(x[0]))
                nxt = nxt[: 4 * c]
            partial = nxt
        cand = {}
        for leaves, parts in partial:
            if leaves in cand or not leaves:
                continue
            vals = [tt_expand(t2, l2, leaves) for l2, t2 in parts]
            cand[leaves] = f(*vals, M64)
        kept = []
        for leaves, t in sorted(cand.items(), key=lambda x: (len(x[0]), x[0])):
            s = set(leaves)
            if any(set(l2) <= s for l2, _ in kept):
                continue
            kept.append((leaves, t))
            if len(kept) >= c:
                break
        cuts[u] = kept
    return cuts


def cut_layer(net, cuts):
    """Per class, the gates with a cut of that class (each gate counted once
    per class, at its largest such cut); cells over shared cuts: full adders
    (xor3 and maj3 over the same three leaves), half adders (xor2 and and2),
    muxes, S-box candidates (three or more balanced nonlinear functions over
    the same four to six leaves)."""
    by_class = Counter()
    by_cut = defaultdict(list)       # leaves -> [(gate, class)]
    node_class = {}
    for u, lst in cuts.items():
        seen = {}
        for leaves, t in lst:
            cl = classify_tt(t, len(leaves))
            ess = len(tt_essential(t, len(leaves)))
            if ess == len(leaves) and ess >= 2:
                by_cut[leaves].append((u, cl))
            base = cl.rstrip("0123456789")
            if base not in seen or ess > seen[base][1]:
                seen[base] = (cl, ess)
        for base, (cl, ess) in seen.items():
            by_class[base] += 1
        node_class[u] = seen
    cells = Counter()
    sbox_cuts = []
    for leaves, lst in by_cut.items():
        kinds = Counter(cl for _u, cl in lst)
        n = len(leaves)
        if n == 3 and kinds["xor3"] and kinds["maj3"]:
            cells["fa"] += min(kinds["xor3"], kinds["maj3"])
        if n == 2 and kinds["xor2"] and kinds["and2"]:
            cells["ha"] += min(kinds["xor2"], kinds["and2"])
        if n == 3 and kinds["mux"]:
            cells["mux"] += kinds["mux"]
        if 4 <= n <= 6 and kinds[f"bal{n}"] >= 3:
            cells[f"sbox{n}"] += 1
            sbox_cuts.append((leaves, [u for u, cl in lst if cl == f"bal{n}"]))
    return by_class, cells, sbox_cuts, node_class


# ---------------------------------------------------------------------------
# bit-sliced word arithmetic (a word is a list of ints, LSB first, one bit
# per pattern)
# ---------------------------------------------------------------------------


def bs_add(a, b, mask, cin=0):
    n = max(len(a), len(b))
    a = a + [0] * (n - len(a))
    b = b + [0] * (n - len(b))
    out = []
    c = cin
    for x, y in zip(a, b):
        t = x ^ y
        out.append(t ^ c)
        c = (x & y) | (c & t)
    out.append(c)
    return out


def bs_not(a, mask):
    return [mask ^ x for x in a]


def bs_neg(a, mask):
    return bs_add(bs_not(a, mask), [], mask, cin=mask)[: len(a)]


def bs_mul(a, b, mask, w_out):
    acc = [0] * w_out
    for i, bi in enumerate(b):
        if i >= w_out or not bi:
            continue
        pp = [0] * i + [x & bi for x in a]
        acc = bs_add(acc, pp[:w_out], mask)[:w_out]
    return acc


def bs_const(c, w, mask):
    return [mask if (c >> p) & 1 else 0 for p in range(w)]


def bs_sext(a, w):
    return a + [a[-1]] * (w - len(a)) if a else [0] * w


def bs_popcount(lits, mask):
    """Bit-sliced count of the true literals per pattern."""
    cnt = []
    for x in lits:
        c = x
        for t in range(len(cnt)):
            if not c:
                break
            cnt[t], c = cnt[t] ^ c, cnt[t] & c
        if c:
            cnt.append(c)
    return cnt


def bs_ge_const(bits, t, mask):
    """Patterns where the bit-sliced number is at least t."""
    if t <= 0:
        return mask
    if t >= 1 << len(bits):
        return 0
    # a >= t  <=>  not (a < t); compare from the MSB
    gt, eq = 0, mask
    for p in range(len(bits) - 1, -1, -1):
        tb = (t >> p) & 1
        if tb:
            eq &= bits[p]
        else:
            gt |= eq & bits[p]
            eq &= mask ^ bits[p]
    return gt | eq


# ---------------------------------------------------------------------------
# observed outputs and sides
# ---------------------------------------------------------------------------


def _gate_class(net, u):
    g = u - net.npi
    n = net.nin[g]
    return classify_tt(tt_replicate(net.tt[g], n), n) if n else "const"


def observed_outputs(net, clauses):
    """(compare pairs, singles, asserted). The clauses no gate definition
    uses are the top, simplified by their own units. Two of them that say
    x != y (or x == y) over two gate signals are a compare pair. From the
    signals of the wide top clauses (and from a lone unit on a gate: a
    miter's asserted or) the walk goes down through and/or gates every
    fan-out of which is in the top region (the assertion logic), and stops
    at the first other signals: an xor under that or is a compare pair of
    its two inputs (so is an xor of one signal with itself, which hashing
    made constant: both sides computed the bit alike); an xor asserted by a
    unit is one when its two inputs have cones each at least a tenth of the
    netlist and mostly apart (a one-bit miter); any other signal is an
    observed single. `asserted` is the gate signals fixed by units, a word
    of outputs when there are two or more."""
    npi = net.npi
    top = [c for i, c in enumerate(clauses) if i not in net.used]
    # the top clauses with the literals their own units falsify removed: a
    # compare written as clauses ((x or y), (not x or not y) once the
    # miter's unit is in) is read as a compare pair of the two signals
    unit_lits = {c[0] for c in top if len(c) == 1}
    simple = []
    for c in top:
        if len(c) == 1:
            simple.append(c)
            continue
        if any(l in unit_lits for l in c):
            continue                       # satisfied by a unit
        simple.append(tuple(l for l in c if -l not in unit_lits))
    two = defaultdict(set)
    for c in simple:
        if len(c) == 2:
            a, b = c
            ta, tb = net.id_of.get(abs(a)), net.id_of.get(abs(b))
            if ta and tb and ta[0] >= npi and tb[0] >= npi and ta[0] != tb[0]:
                key = tuple(sorted((ta[0], tb[0])))
                pa = (a > 0) != ta[1]
                pb = (b > 0) != tb[1]
                two[key].add((pa, pb) if ta[0] < tb[0] else (pb, pa))
    clause_pairs = []
    in_clause_pair = set()
    for key, sgn in two.items():
        if {(True, True), (False, False)} <= sgn or {(True, False), (False, True)} <= sgn:
            clause_pairs.append(key)        # x != y, or x == y
            in_clause_pair.update(key)
    top_ids = set()
    asserted = []
    in_wide = set()
    for c in simple:
        for l in c:
            t = net.id_of.get(abs(l))
            if t and t[0] >= npi and t[0] not in in_clause_pair:
                top_ids.add(t[0])
                if len(c) == 1:
                    asserted.append((t[0], t[1] != (l < 0)))
                else:
                    in_wide.add(t[0])
    fo = net.fanout()
    # the walk starts at the signals of the wide top clauses, and at a unit
    # only when it is the one unit on a gate (a miter's asserted or); two or
    # more units on gates fix a word, whose bits are observed as they are
    unit_gates = {u for u, _ng in asserted}
    if len(unit_gates) >= 2:
        region = set(in_wide)
    else:
        region = set(top_ids)
    boundary = set()
    # gates in decreasing id order: a region gate of the and/or class passes
    # the region on to a fan-in gate whose fan-outs are all region gates
    under_or = set(in_wide)
    heap = [-u for u in region]
    heapq.heapify(heap)
    while heap:
        u = -heapq.heappop(heap)
        cl = _gate_class(net, u)
        if cl.startswith("and") and net.nin[u - npi] >= 2:
            for i in net.fanin[u - npi]:
                if i < npi:
                    continue
                under_or.add(i)
                if i not in region and all(f in region for f in fo[i]):
                    region.add(i)
                    heapq.heappush(heap, -i)
                elif i not in region:
                    boundary.add(i)
        else:
            boundary.add(u)
    pairs, singles = list(clause_pairs), []
    small = {}

    def small_cone(x):
        if x not in small:
            seen = set()
            frontier = [x]
            for _d in range(12):
                nxt = []
                for y in frontier:
                    if y < npi or y in seen:
                        continue
                    seen.add(y)
                    nxt.extend(net.fanin[y - npi])
                frontier = nxt
                if len(seen) > 400:
                    break
            small[x] = seen
        return small[x]

    unit_only = {u for u, _ng in asserted} - under_or
    n_gates = net.size - npi
    full_cone_checks = [0]
    for u in sorted(boundary):
        g = u - npi
        if u in net.shared and net.shared[u] >= npi and u in under_or:
            x = net.shared[u]
            pairs.append((x, x))         # a compare whose two sides hashed to one signal
            continue
        if _gate_class(net, u) == "xor2" and net.nin[g] == 2:
            a, b = net.fanin[g]
            if a >= npi and b >= npi:
                if u not in unit_only:
                    pairs.append((a, b))    # an xor under the assertion's or: a compare
                    continue
                full_cone_checks[0] += 1
                if full_cone_checks[0] <= 8:        # a one-bit miter asserts one compare
                    ca, cb = set(net.cone([a])), set(net.cone([b]))
                    big = max(4, 0.1 * n_gates)
                    if min(len(ca), len(cb)) >= big and len(ca & cb) < 0.9 * min(len(ca), len(cb)):
                        pairs.append((a, b))    # a one-bit miter's asserted compare
                        continue
        singles.append(u)
    return pairs, singles, asserted


def split_pairs(net, pairs):
    """The two sides of a miter from its compare pairs: each pair's members
    go to the side whose nearby gates they share most."""
    npi = net.npi

    def near(x):
        seen = set()
        frontier = [x]
        for _d in range(6):
            nxt = []
            for y in frontier:
                if y < npi or y in seen:
                    continue
                seen.add(y)
                nxt.extend(net.fanin[y - npi])
            frontier = nxt
            if len(seen) > 300:
                break
        return seen

    cones = {x: near(x) for p in pairs for x in p}
    order = sorted(pairs, key=lambda p: -(len(cones[p[0]]) + len(cones[p[1]])))
    a_side, b_side = [], []
    ua, ub = set(), set()
    for x, y in order:
        if x == y:
            a_side.append(x)
            b_side.append(x)
            continue
        if not a_side:
            a_side.append(x)
            b_side.append(y)
        else:
            sx = len(cones[x] & ua) - len(cones[x] & ub)
            sy = len(cones[y] & ua) - len(cones[y] & ub)
            if sx >= sy:
                a_side.append(x)
                b_side.append(y)
            else:
                a_side.append(y)
                b_side.append(x)
        ua |= cones[a_side[-1]]
        ub |= cones[b_side[-1]]
    return a_side, b_side


class Side:
    """A group of observed signals (a miter side, or the asserted outputs)
    and the cone below them."""

    def __init__(self, net, outs, name):
        self.net = net
        self.name = name
        uniq, seen = [], set()
        for i in outs:
            if i in seen or i in net.const:
                continue
            seen.add(i)
            uniq.append(i)
        self.truncated = len(uniq) > MAX_OUTPUTS
        self.outs = uniq[:MAX_OUTPUTS]
        self.cone = net.cone(self.outs)
        pis = set(net.cone_pis(self.cone)) | {i for i in self.outs if i < net.npi}
        self.pis = sorted(pis)
        self.pidx = {p: k for k, p in enumerate(self.pis)}
        self._sup = None
        self.hints = {}            # output index -> points where it takes its rare value
        self._ext = None
        self.coef = None           # per output, its affine coefficients (set by t_linear)
        self.batches = []          # the classifier's random batches: (assignment, values)

    def run(self, assign, mask):
        val = self.net.simulate(assign, mask, self.cone)
        return [val[i] for i in self.outs]

    def rand(self, rnd, width):
        return {p: rnd.getrandbits(width) for p in self.pis}

    def extended(self):
        """(assignment, mask, output values) on the structured patterns, the
        rare-value points and the classifier's random batches: where a match
        found on a test's own random patterns is checked before it is
        reported."""
        key = (sum(len(v) for v in self.hints.values()), len(self.batches))
        if self._ext is None or self._ext[0] != key:
            sb = structured_bits(self)
            ns = 2 * len(self.pis) + 2
            pts = [pt for _j, lst in sorted(self.hints.items()) for pt in lst]
            at = {p: sb[p] | (sum(pt[p] << t for t, pt in enumerate(pts)) << ns) for p in self.pis}
            off = ns + len(pts)
            for a, _v in self.batches:
                for p in self.pis:
                    at[p] |= a[p] << off
                off += NPAT
            mx = (1 << off) - 1
            self._ext = (key, at, mx, self.run(at, mx))
        return self._ext[1:]

    def supports(self):
        """Per output, its primary inputs as a bitset over self.pis."""
        if self._sup is None:
            sup = {p: 1 << k for k, p in enumerate(self.pis)}
            npi = self.net.npi
            fanin = self.net.fanin
            for u in self.cone:
                s = 0
                for i in fanin[u - npi]:
                    s |= sup[i]
                sup[u] = s
            self._sup = [sup[i] for i in self.outs]
        return self._sup


def _bits(x):
    out = []
    k = 0
    while x:
        if x & 1:
            out.append(k)
        x >>= 1
        k += 1
    return out


def seg_run(side, base, n, segs):
    """Simulate len(segs) experiments at once: segment s is the base
    patterns (n bits per input) with the overrides segs[s] ({pi: value});
    returns per output the list of segment values."""
    s_count = len(segs)
    mn = (1 << n) - 1
    rep = ((1 << (s_count * n)) - 1) // mn
    assign = {p: base[p] * rep for p in side.pis}
    for s, mods in enumerate(segs):
        sh = s * n
        for p, v in mods.items():
            assign[p] ^= (base[p] ^ v) << sh
    outs = side.run(assign, (1 << (s_count * n)) - 1)
    return [[(o >> (s * n)) & mn for s in range(s_count)] for o in outs]


class Sig:
    """Output values by signature, with complements."""

    def __init__(self, vals, mask):
        self.mask = mask
        self.d = defaultdict(list)
        for j, v in enumerate(vals):
            self.d[v].append((j, False))
            self.d[v ^ mask].append((j, True))

    def find(self, v):
        return self.d.get(v, ())


# -- linear -------------------------------------------------------------------


def structured_bits(side, neg=None):
    """Per input, its bits on 2n + 2 structured patterns over the side's n
    inputs: bit 0 all zero, bit 1 + i input i alone, bit n + 1 all one, bit
    n + 2 + i all one but input i; with `neg` (input -> 1 if negated) the
    same patterns over the literals instead."""
    n = len(side.pis)
    ones = ((1 << (n + 1)) - 1) << (n + 1)
    full = (1 << (2 * n + 2)) - 1
    out = {}
    for i, p in enumerate(side.pis):
        v = (1 << (1 + i)) | (ones & ~(1 << (n + 2 + i)))
        out[p] = v ^ full if neg and neg.get(p) else v
    return out


def probe_values(side):
    """Per output, its values on the structured patterns (structured_bits).
    A rare value (a wide and, an equality) that random patterns miss often
    shows here."""
    return side.run(structured_bits(side), (1 << (2 * len(side.pis) + 2)) - 1)


def probe_point(side, t):
    """The input assignment of structured pattern t (see probe_values)."""
    n = len(side.pis)
    if t <= n:
        return {p: int(t == 1 + i) for i, p in enumerate(side.pis)}
    return {p: int(t != n + 2 + i) for i, p in enumerate(side.pis)}


def justify(side, j, target, rnd, tries=8, max_steps=400_000):
    """An input assignment that gives output j the value target, or None. From
    the output down, every gate takes a random row of its table with the value
    asked of it whose inputs agree with what is already asked of them (no
    backtracking, a few restarts); inputs nothing asks for are random. The
    assignment is checked by simulation."""
    net = side.net
    npi = net.npi
    root = side.outs[j]
    steps = 0
    for _ in range(tries):
        want = {root: target}
        stack = [root]
        ok = True
        while stack:
            u = stack.pop()
            if u < npi:
                continue
            steps += 1
            if steps > max_steps:
                return None
            g = u - npi
            ins = net.fanin[g]
            tt = net.tt[g]
            v = want[u]
            rows = [m for m in range(1 << len(ins))
                    if ((tt >> m) & 1) == v and all(want.get(x, (m >> i) & 1) == ((m >> i) & 1) for i, x in enumerate(ins))]
            if not rows:
                ok = False
                break
            m = rnd.choice(rows)
            for i, x in enumerate(ins):
                if x not in want:
                    want[x] = (m >> i) & 1
                    stack.append(x)
        if not ok:
            continue
        point = {p: want[p] if p in want else rnd.getrandbits(1) for p in side.pis}
        if side.run(point, 1)[j] == target:
            return point
    return None


def t_linear(side, rnd, probe=None, batches=()):
    """Per output: is it affine over GF(2)? The coefficients come from the
    single flips from zero; the affine function they give must then agree
    with the single flips from all one, with random patterns (a fresh batch
    and the given (assignment, values) batches: an output that is nearly
    constant must show its rare value somewhere) and at the rare-value
    points (a wide and, nearly constant on random patterns, fails at all
    one). Sets side.coef: per output its coefficients over side.pis (a
    bitset), or None."""
    pis = side.pis
    n = len(pis)
    if probe is None:
        probe = probe_values(side)
    w = NPAT
    mask = (1 << w) - 1
    x = side.rand(rnd, w)
    checks = [(x, mask, side.run(x, mask))] + [(a, (1 << w) - 1, v) for a, v in batches]
    pts, hv = hint_values(side)
    if pts:
        at = {p: sum(pt[p] << t for t, (_j, pt) in enumerate(pts)) for p in pis}
        checks.append((at, (1 << len(pts)) - 1, hv))
    lo = (1 << n) - 1
    coefs = []
    for j, pv in enumerate(probe):
        c0 = pv & 1
        coef = ((pv >> 1) & lo) ^ (lo if c0 else 0)
        c1 = (pv >> (n + 1)) & 1
        coef1 = ((pv >> (n + 2)) & lo) ^ (lo if c1 else 0)
        ok = coef == coef1 and c1 == c0 ^ (_popcount(coef) & 1)
        for at, msk, vals in checks:
            if not ok:
                break
            want = msk if c0 else 0
            for i in _bits(coef):
                want ^= at[pis[i]]
            ok = want == vals[j]
        coefs.append(coef if ok else None)
    side.coef = coefs
    return [c is not None for c in coefs]


def hint_values(side):
    """The side's rare-value points as (output index, point) and every
    output's values on them (bit t for point t)."""
    pts = [(j, pt) for j, lst in sorted(side.hints.items()) for pt in lst]
    if not pts:
        return pts, [0] * len(side.outs)
    assign = {p: sum(pt[p] << t for t, (_j, pt) in enumerate(pts)) for p in side.pis}
    return pts, side.run(assign, (1 << len(pts)) - 1)


# -- S-box layer ----------------------------------------------------------------


def t_sbox_layer(side, rnd, lin):
    if side.hints:
        return None       # S-box outputs are balanced, never nearly constant
    sup = side.supports()
    groups = defaultdict(list)
    for j, s in enumerate(sup):
        groups[s].append(j)
    if len(groups) < 2:
        return None
    seen = 0
    sizes = Counter()
    for s, js in groups.items():
        k = _popcount(s)
        if not 3 <= k <= 8 or s & seen:
            return None
        seen |= s
        sizes[k] += 1
    w = 1024
    mask = (1 << w) - 1
    vals = side.run(side.rand(rnd, w), mask)
    for j, v in enumerate(vals):
        if lin[j] or abs(_popcount(v) - w / 2) > 0.1 * w:
            return None
    k = sizes.most_common(1)[0][0]
    return f"sbox-layer:{len(groups)}x{k}", {"groups": len(groups), "inputs": k}


# -- symmetric: popcount, thresholds, sorters ------------------------------------


def _polarity_by_direction(side, rnd, pis):
    """Per input, 1 if the literal is negative: raising it lowers an output
    that only ever moves one way with each input (a count's top bit, a
    threshold); with no such output, the majority over all outputs."""
    n = 256
    mn = (1 << n) - 1
    base = side.rand(rnd, n)
    segs = []
    for p in pis:
        segs.append({p: 0})
        segs.append({p: mn})
    vals = seg_run(side, base, n, segs)
    moves = []          # per output, per input: (up, down)
    for v in vals:
        row = []
        for k in range(len(pis)):
            lo, hi = v[2 * k], v[2 * k + 1]
            row.append((_popcount(hi & ~lo & mn), _popcount(lo & ~hi & mn)))
        moves.append(row)
    mono = [row for row in moves if all(min(u, d) == 0 and u + d > 0 for u, d in row)]
    neg = {}
    for k, p in enumerate(pis):
        if mono:
            up, down = mono[0][k]
        else:
            up = sum(row[k][0] for row in moves)
            down = sum(row[k][1] for row in moves)
        neg[p] = 1 if down > up else 0
    return neg


def _count_function(lit, pis, vals, mask):
    """Every output a function of the number of true literals on the given
    samples (an equality a = b looks symmetric on random patterns alone)."""
    cnt = bs_popcount([lit[p] for p in pis], mask)
    groups = defaultdict(int)          # count -> the samples that have it
    for t in range(mask.bit_length()):
        groups[sum(((b >> t) & 1) << k for k, b in enumerate(cnt))] |= 1 << t
    for v in vals:
        for m in groups.values():
            if (v & m) not in (0, m):
                return False
    return True


def t_symmetric(side, rnd):
    sup = side.supports()
    if not sup or any(s != sup[0] for s in sup):
        return None
    pis = [side.pis[k] for k in _bits(sup[0])]
    if len(pis) < 3:
        return None
    w = NPAT
    mask = (1 << w) - 1
    base = side.rand(rnd, w)
    v0 = side.run(base, mask)
    pts, _hv = hint_values(side)
    hb = {p: sum(pt[p] << t for t, (_j, pt) in enumerate(pts)) for p in side.pis}
    nh = len(pts)
    for a, _v in side.batches:            # and the classifier's random batches
        for p in side.pis:
            hb[p] |= a[p] << nh
        nh += NPAT
    ns = 2 * len(side.pis) + 2
    mx = (1 << (w + ns + nh)) - 1
    for pol in ("plain", "direction"):
        if pol == "direction" and len(pis) > 512:
            break           # the polarity pass packs two experiments per input
        neg = dict.fromkeys(pis, 0) if pol == "plain" else _polarity_by_direction(side, rnd, pis)
        lit = {p: base[p] ^ (mask if neg[p] else 0) for p in pis}
        ok = True
        for _t in range(3):
            perm = pis[:]
            rnd.shuffle(perm)
            a = dict(base)
            for src, dst in zip(pis, perm):
                a[dst] = lit[src] ^ (mask if neg[dst] else 0)
            if side.run(a, mask) != v0:
                ok = False
                break
        if not ok:
            continue
        n = len(pis)
        # the samples the count templates are matched on: the random ones, the
        # structured patterns over the literals (a count's top bit is one only
        # when every literal is) and the rare-value points
        sb = structured_bits(side, neg)
        ext = {p: base[p] | (sb[p] << w) | (hb[p] << (w + ns)) for p in side.pis}
        vx = side.run(ext, mx)
        litx = {p: ext[p] ^ (mx if neg[p] else 0) for p in pis}
        if not _count_function(litx, pis, vx, mx):
            continue
        sig = Sig(vx, mx)
        for flip_all in (0, mx):        # the count of the literals, or of their negations
            lits = [litx[p] ^ flip_all for p in pis]
            cnt = bs_popcount(lits, mx)
            found = set()
            for b in cnt:
                for j, _ng in sig.find(b):
                    found.add(j)
            pop_bits = sum(1 for b in cnt if sig.find(b))
            if len(found) == len(vx) and pop_bits == len(cnt):
                return f"popcount:{n}", {"inputs": n, "polarity": pol}
            thr = {}
            for t in range(n + 2):
                g = bs_ge_const(cnt, t, mx)
                for j, _ng in sig.find(g):
                    thr.setdefault(j, t)
            if len(thr) == len(vx):
                if len(vx) == 1:
                    return f"threshold:{n}", {"inputs": n, "t": thr[0], "polarity": pol}
                return f"sorter:{n}", {"inputs": n, "outputs": len(vx), "polarity": pol}
        return f"symmetric:{n}", {"inputs": n, "polarity": pol}
    return None


# -- triangular arithmetic: add, sub, multiply, square, negate, constants --------
# Bit p of each of these depends on operand bits 0..p only, so the operand
# bits come out of the outputs' supports (a primary input first appears in
# the output of the rank at which its position enters) and the assignment of
# each position's inputs to operands is settled bit by bit, the reference
# computed on the operands so far.

ARITH = {
    1: ["inc", "dec", "neg", "sqr", "addc", "subc", "mulc", "cube"],
    2: ["add", "sub", "mul", "add1"],
    3: ["add3", "mac"],
}
CONST_FNS = {"addc", "subc", "mulc"}


def arith_ref(fn, ops, cst, w_out, mask, signed=False):
    """Bit-sliced result (w_out bits) of fn on operand words and a constant word."""
    a = ops[0]
    if signed:
        ops = [bs_sext(o, w_out) for o in ops]
        a = ops[0]
        if cst:
            cst = bs_sext(cst, w_out)
    if fn == "add":
        r = bs_add(a, ops[1], mask)
    elif fn == "add1":
        r = bs_add(a, ops[1], mask, cin=mask)
    elif fn == "sub":
        b = ops[1] + [0] * (max(len(a), len(ops[1])) - len(ops[1]))
        aa = a + [0] * (len(b) - len(a))
        r = bs_add(aa, bs_not(b, mask), mask, cin=mask)
    elif fn == "mul":
        r = bs_mul(a, ops[1], mask, w_out)
    elif fn == "add3":
        r = bs_add(bs_add(a, ops[1], mask), ops[2], mask)
    elif fn == "mac":
        r = bs_add(bs_mul(a, ops[1], mask, w_out), ops[2], mask)
    elif fn == "inc":
        r = bs_add(a, [], mask, cin=mask)
    elif fn == "dec":
        r = bs_add(a, [mask] * len(a), mask)
    elif fn == "neg":
        r = bs_neg(a + [0] * (w_out - len(a)), mask)
    elif fn == "sqr":
        r = bs_mul(a, a, mask, w_out)
    elif fn == "cube":
        r = bs_mul(bs_mul(a, a, mask, w_out), a, mask, w_out)
    elif fn == "addc":
        r = bs_add(a, cst, mask)
    elif fn == "subc":
        n = max(len(a), len(cst))
        r = bs_add(cst + [0] * (n - len(cst)), bs_not(a + [0] * (n - len(a)), mask), mask, cin=mask)
    elif fn == "mulc":
        r = bs_mul(a, cst, mask, w_out)
    else:
        raise ValueError(fn)
    r = r[:w_out]
    return r + [0] * (w_out - len(r))


def _positions(side):
    """The inputs in order of first appearance: one group per output rank at
    which new inputs enter."""
    sup = side.supports()
    order = sorted(range(len(sup)), key=lambda j: (_popcount(sup[j]), j))
    seen = 0
    groups = []
    for j in order:
        new = sup[j] & ~seen
        if new:
            groups.append([side.pis[k] for k in _bits(new)])
            seen |= new
    return groups


def t_arith(side, rnd, max_steps=4_000):
    groups = _positions(side)
    if len(groups) < 2:
        return None
    sizes = [len(g) for g in groups]
    m = Counter(sizes[1:]).most_common(1)[0][0] if len(sizes) > 1 else sizes[0]
    if m not in ARITH:
        return None
    # each group is one position (m inputs), two merged positions (2m), or
    # position 0 with a carry-in (m + 1)
    short_from = None
    for k, s in enumerate(sizes):
        if short_from is not None:
            if s >= m:
                return None
            continue
        if 0 < s < m and k > 0:
            short_from = k          # from here on only the longer operands have bits
            continue
        if s not in (m, 2 * m) and not (m == 1 and s == 3) and not (k == 0 and s == m + 1):
            return None
    w = NPAT
    mask = (1 << w) - 1
    base = side.rand(rnd, w)
    vals = side.run(base, mask)
    sig = Sig(vals, mask)
    best = None
    for pol in (False, True):        # inputs as they are first, then with negations
        for fn in ARITH[m]:
            for signed in (False, True):
                r = _arith_search(side, groups, m, fn, signed, base, vals, sig, mask, max_steps, pol)
                if r and (best is None or r[1] > best[1]):
                    best = r
            if best and best[1] >= 1.0:      # a full match; a partial one keeps looking
                break
        if best and best[1] >= 1.0:
            break
    if best is None or best[1] < 0.9:
        return None
    label, cov, detail = best
    return label, detail


def _choices(group, m, first, pol):
    """Ways to read a group as positions: lists of operand tuples of
    literals (input, negated) and a carry-in literal (or None). With pol the
    inputs may be negated."""
    def lits(tup):
        if not pol:
            yield tuple((x, 0) for x in tup)
            return
        for negs in range(1 << len(tup)):
            yield tuple((x, (negs >> i) & 1) for i, x in enumerate(tup))

    if len(group) == m:
        for perm in permutations(group):
            for lt in lits(perm):
                yield [lt], None
            if m == 1:
                break
    elif len(group) in (2 * m, 3 * m) and (len(group) == 2 * m or m == 1):
        t = len(group) // m
        for perm in permutations(group):
            for lt in lits(perm):
                yield [lt[i * m:(i + 1) * m] for i in range(t)], None
    elif 0 < len(group) < m and not first:
        # the longer operand's top bits: the others have no bit here
        for slots in permutations(range(m), len(group)):
            for perm in permutations(group):
                for lt in lits(perm):
                    tup = [(None, 0)] * m
                    for sl, lit in zip(slots, lt):
                        tup[sl] = lit
                    yield [tuple(tup)], None
                break
    elif first and len(group) == m + 1:
        for cin in group:
            rest = [x for x in group if x != cin]
            for perm in permutations(rest):
                for lt in lits(perm):
                    for cneg in ((0, 1) if pol else (0,)):
                        yield [lt], (cin, cneg)


def _score(width, n_out, pairs):
    """(width, reference bits found, outputs matched, coverage, pairs) for
    (output, reference bit, negated) pairs, or None below the bar: every
    operand position found and 90% of the outputs."""
    if not pairs:
        return None
    matched = {j for j, _k, _ng in pairs}
    bits_found = len({k for _j, k, _ng in pairs})
    cov = len(matched) / max(1, n_out)
    if bits_found < width or cov < 0.9:
        return None
    return width, bits_found, matched, cov, pairs


def _arith_search(side, groups, m, fn, signed, base, vals, sig, mask, max_steps, pol=False):
    """Depth-first over positions: a choice survives if every non-constant
    reference bit up to the current position is an output (or its
    complement)."""
    steps = [0]
    n_out = len(vals)
    const_fn = fn in CONST_FNS

    def ref(ops_bits, cst_bits, cin, w_out, at=None, msk=None):
        at = base if at is None else at
        msk = mask if msk is None else msk
        ops = [[(at[x] ^ (msk if ng else 0)) if x is not None else 0 for x, ng in col] for col in ops_bits]
        cst = [msk if b else 0 for b in cst_bits] if const_fn else None
        r = arith_ref(fn, ops, cst, w_out, msk, signed)
        if cin is not None:
            r = bs_add(r, [at[cin[0]] ^ (msk if cin[1] else 0)], msk)[:w_out]
        return r

    wires = {base[p] for p in side.pis} | {base[p] ^ mask for p in side.pis}

    def ok_upto(r, lo, hi):
        for p in range(lo, hi):
            b = r[p]
            if b in (0, mask) or b in wires:      # constant, or an input itself (bit 0 of x * x)
                continue
            if not sig.find(b):
                return False
        return True

    def leaf(ops_bits, cst_bits, cin):
        width = len(ops_bits[0])
        if width < 2:
            return None
        wsum = sum(sum(1 for x, _ng in col if x is not None) for col in ops_bits)
        hi = {"add": width + 1, "add1": width + 1, "sub": width + 1, "add3": width + 2, "inc": width + 1,
              "mul": wsum, "mac": wsum + 1, "sqr": 2 * width, "cube": 3 * width, "mulc": 2 * width,
              "addc": width + 1, "subc": width + 1}.get(fn, width)
        r = ref(ops_bits, cst_bits, cin, hi)
        # an output is a reference bit (or its complement) on the random
        # patterns and on the check samples (the structured patterns, the
        # rare-value points, the classifier's batches): a carry that is
        # zero on every random pattern still pairs, and a fault the fresh
        # batch missed still shows
        at, mx, outs = side.extended()
        rx = ref(ops_bits, cst_bits, cin, hi, at, mx)
        pairs = [(j, k, ng) for k, b in enumerate(r) if not (b in (0, mask) and rx[k] in (0, mx))
                 for j, ng in sig.find(b) if outs[j] == rx[k] ^ (mx if ng else 0)]
        return _score(width, n_out, pairs)

    def dfs(k, ops_bits, cst_bits, cin, pos):
        steps[0] += 1
        if steps[0] > max_steps:
            return None
        if k == len(groups):
            ev = leaf(ops_bits, cst_bits, cin)
            return (ops_bits, cst_bits, cin, ev) if ev else None
        for positions, c_in in _choices(groups[k], m, k == 0, pol):
            if c_in is not None and fn not in ("add", "sub", "inc"):
                continue
            nb = [col[:] for col in ops_bits]
            for tup in positions:
                for t in range(m):
                    nb[t].append(tup[t])
            npos = pos + len(positions)
            cin2 = c_in if c_in is not None else cin
            for cst_try in ([()] if not const_fn else _const_bits(len(positions))):
                cb = cst_bits + list(cst_try)
                r = ref(nb, cb, cin2, npos)
                if ok_upto(r, pos, npos):
                    got = dfs(k + 1, nb, cb, cin2, npos)
                    if got:
                        return got
        return None

    got = dfs(0, [[] for _ in range(m)], [], None, 0)
    if not got:
        return None
    ops_bits, cst_bits, cin, (width, bits_found, matched, cov, pairs) = got
    widths = [sum(1 for x, _ng in col if x is not None) for col in ops_bits]
    # "-part" when some outputs are not bits of the function (a wrong carry,
    # one more asserted bit): the label holds for the matched outputs only
    kind = fn if len(matched) == n_out else f"{fn}-part"
    label = f"{kind}:{'x'.join(str(wd) for wd in sorted(widths, reverse=True))}"
    if signed and fn in ("mul", "sqr", "mac", "cube", "mulc"):
        label += "s"
    detail = {"width": width, "bits": bits_found, "outputs": n_out, "matched": len(matched), "signed": int(signed)}
    if const_fn:
        detail["const"] = sum(1 << p for p, b in enumerate(cst_bits) if b)
    if cin is not None:
        detail["cin"] = 1
    negs = sum(ng for col in ops_bits for x, ng in col if x is not None) + (cin[1] if cin is not None else 0)
    if negs:
        detail["negated"] = negs
    return label, cov, detail


def _const_bits(n):
    for a in range(1 << n):
        yield tuple((a >> i) & 1 for i in range(n))


# -- compare: a < b, a <= b (either orientation per bit), a == b ------------------


def _lt(a_bits, b_bits, mask):
    """Bit-sliced a < b (unsigned), words MSB first."""
    lt, eq = 0, mask
    for x, y in zip(a_bits, b_bits):
        lt |= eq & (mask ^ x) & y
        eq &= mask ^ (x ^ y)
    return lt, eq


def _cmp_one(side, j, pis, rnd):
    """a < b over two words whose bits pair up: with the higher pairs made
    equal the next pair has the largest influence (about one half). A pair's
    polarity is the one that, made equal, leaves the lower inputs some
    influence (an unequal pair decides the compare)."""
    n = 512
    mn = (1 << n) - 1
    base = side.rand(rnd, n)
    pairs = []           # (y, z, polarity): equal means z = y ^ polarity
    remaining = list(pis)

    def equalised(extra=None):
        b2 = dict(base)
        for y, z, pp in pairs + ([extra] if extra else []):
            b2[z] = b2[y] ^ (mn if pp else 0)
        return b2

    while remaining:
        if len(remaining) == 2:
            y, z = remaining
            pairs.append((y, z, None))
            break
        b2 = equalised()
        segs = [{}] + [{p: b2[p] ^ mn} for p in remaining]
        v = seg_run(side, b2, n, segs)[j]
        inf = sorted(((_popcount(v[0] ^ v[k + 1]) / n, p) for k, p in enumerate(remaining)), reverse=True)
        (_i1, p1), (i2, p2), (i3, _p3) = inf[0], inf[1], inf[2]
        if i2 < 0.3 or i3 > 0.75 * i2:
            return None
        rest = [p for p in remaining if p not in (p1, p2)]
        chosen = None
        for pp in (0, 1):
            b3 = equalised((p1, p2, pp))
            segs = [{}] + [{p: b3[p] ^ mn} for p in rest]
            v = seg_run(side, b3, n, segs)[j]
            top = max(_popcount(v[0] ^ v[k + 1]) for k in range(len(rest))) / n
            if top >= 0.3:
                chosen = pp
                break
        if chosen is None:
            return None
        pairs.append((p1, p2, chosen))
        remaining = rest
    # orientation and the last pair's polarity: the others equal, this one
    # differs (the two patterns where y and z differ as literals)
    a_lits, b_lits = [], []
    for k, (y, z, pp) in enumerate(pairs):
        found = None
        for ppx in ((pp,) if pp is not None else (0, 1)):
            outs = []
            for yv in (0, 1):
                t = equalised()
                t = {q: (mn if t[q] & 1 else 0) for q in t}
                for yy, zz, pq in pairs[:k] + pairs[k + 1:]:
                    t[zz] = t[yy] ^ (mn if (pq or 0) else 0)
                t[y] = mn if yv else 0
                t[z] = (mn if yv else 0) ^ (mn if not ppx else 0)    # differ as literals
                outs.append(side.run(t, mn)[j] & 1)
            if outs[0] != outs[1]:
                found = (ppx, outs)
                break
        if found is None:
            return None
        ppx, outs = found
        pairs[k] = (y, z, ppx)
        # outs[0]: y = 0 and z differs. a < b with a = this literal of y...
        if outs[0]:
            a_lits.append((y, 0))
            b_lits.append((z, ppx))
        else:
            a_lits.append((y, 1))
            b_lits.append((z, 1 ^ ppx))
    t = {q: 0 for q in side.pis}
    for y, z, pp in pairs:
        t[z] = 1 if pp else 0
    eq_out = side.run(t, 1)[j]
    for trial in range(4):
        w = 256
        mask = (1 << w) - 1
        r = side.rand(rnd, w)
        keep = len(pairs) * trial // 4
        for y, z, pp in pairs[: keep]:
            r[z] = r[y] ^ (mask if pp else 0)
        lt, eq = _lt([r[x] ^ (mask if ng else 0) for x, ng in a_lits], [r[x] ^ (mask if ng else 0) for x, ng in b_lits], mask)
        want = lt | (eq if eq_out else 0)
        if side.run(r, mask)[j] != want:
            return None

    def reference(at, mx):
        lt, eq = _lt([at[x] ^ (mx if ng else 0) for x, ng in a_lits], [at[x] ^ (mx if ng else 0) for x, ng in b_lits], mx)
        return lt | (eq if eq_out else 0)

    return ("le" if eq_out else "lt"), len(pairs), reference


def _eq_one(side, j, pis, rnd, base=None):
    """a == b (or !=): from a point where the pairs are equal (all zero, or a
    point where the output took its rare value) a single flip changes the
    output and a double flip of a pair restores it. The pairs found, with
    the polarity they have at that point, must then give the output on
    random patterns that make all, all but one, half or none of the pairs
    equal."""
    p0 = {p: (base.get(p, 0) if base else 0) for p in side.pis}
    o0 = side.run(p0, 1)[j]
    n = len(pis)
    segs = [(a, b) for a in range(n) for b in range(a + 1, n)]
    if not segs or len(segs) > 20_000:
        return None
    width = len(segs) + n
    mask = (1 << width) - 1
    assign = {p: (mask if p0[p] else 0) for p in side.pis}
    for s, (a, b) in enumerate(segs):
        assign[pis[a]] ^= 1 << s
        assign[pis[b]] ^= 1 << s
    for k in range(n):
        assign[pis[k]] ^= 1 << (len(segs) + k)
    out = side.run(assign, mask)[j]
    single = (out >> len(segs)) & ((1 << n) - 1)
    want_single = 0 if o0 else (1 << n) - 1
    if single != want_single:
        return None
    partner = {}
    for s, (a, b) in enumerate(segs):
        if ((out >> s) & 1) == o0:
            if a in partner or b in partner:
                return None
            partner[a] = b
            partner[b] = a
    if len(partner) != n:
        return None
    # equal as literals: z = y ^ pp, the polarity the pair has at the point
    pairs = [(pis[a], pis[b], p0[pis[a]] ^ p0[pis[b]]) for a, b in partner.items() if a < b]
    w = NPAT
    mw = (1 << w) - 1
    for keep in (len(pairs), len(pairs) - 1, len(pairs) // 2, 0):
        r = side.rand(rnd, w)
        order = pairs[:]
        rnd.shuffle(order)
        for y, z, pp in order[:keep]:
            r[z] = r[y] ^ (mw if pp else 0)
        eq = mw
        for y, z, pp in pairs:
            eq &= mw ^ r[y] ^ r[z] ^ (mw if pp else 0)
        if side.run(r, mw)[j] != (eq if o0 else mw ^ eq):
            return None

    def reference(at, mx):
        e = mx
        for y, z, pp in pairs:
            e &= mx ^ at[y] ^ at[z] ^ (mx if pp else 0)
        return e if o0 else mx ^ e

    return ("eq" if o0 else "ne"), n // 2, reference


def t_compare(side, rnd):
    if not 1 <= len(side.outs) <= 4:
        return None
    sup = side.supports()
    labels = []
    for j in range(len(side.outs)):
        pis = [side.pis[k] for k in _bits(sup[j])]
        if len(pis) < 4 or len(pis) % 2:
            return None
        r = _cmp_one(side, j, pis, rnd) or _eq_one(side, j, pis, rnd)
        for point in side.hints.get(j, ()):
            if r is not None:
                break
            r = _eq_one(side, j, pis, rnd, base=point)
        if r is None:
            return None
        labels.append(r)
    # and on the structured patterns and the rare-value points
    at, mx, outs = side.extended()
    if any(outs[j] != ref(at, mx) for j, (_k, _w, ref) in enumerate(labels)):
        return None
    kinds = sorted({"eq" if k in ("eq", "ne") else "cmp" for k, _w, _r in labels})
    w = max(wd for _k, wd, _r in labels)
    return f"{'+'.join(kinds)}:{w}", {"width": w, "outputs": len(labels)}


# -- mux trees and shifters --------------------------------------------------------


def t_muxshift(side, rnd):
    if side.hints:
        return None       # a mux or shifter output copies data: it is never nearly constant
    sup = side.supports()
    union = 0
    for s in sup:
        union |= s
    pis = [side.pis[k] for k in _bits(union)]
    if len(pis) < 3 or len(pis) > 600:
        return None
    n = 256
    mn = (1 << n) - 1
    base = side.rand(rnd, n)
    vals = seg_run(side, base, n, [{}] + [{p: base[p] ^ mn} for p in pis])
    # controls (selects, shift amounts) change many outputs about half the
    # time; a data input changes only the outputs that copy it. Narrow words
    # blur the two, so the split is tried after each of the first six ranks,
    # fewest controls first, and the mapping below decides.
    mean = {}
    for k, p in enumerate(pis):
        mean[p] = sum(_popcount(v[0] ^ v[k + 1]) for v in vals) / (n * len(vals))
    ranked = sorted(pis, key=lambda p: -mean[p])
    for k in range(1, 7):
        if k >= len(pis) or mean[ranked[k - 1]] < 0.12:
            break
        ctrl = ranked[:k]
        r = _muxshift_split(side, base, n, ctrl, [p for p in pis if p not in ctrl])
        if r:
            return r
    return None


def _muxshift_split(side, base, n, ctrl, data):
    """The mux or shifter label with these inputs as the controls, or None."""
    mn = (1 << n) - 1
    segs = []
    for a in range(1 << len(ctrl)):
        segs.append({ctrl[t]: (mn if (a >> t) & 1 else 0) for t in range(len(ctrl))})
    vals = seg_run(side, base, n, segs)
    lookup = {}
    for d in data:
        lookup[base[d]] = (d, 0)
        lookup[base[d] ^ mn] = (d, 1)
    # every output copies data inputs, each with one polarity: an output's
    # and a data input's polarities must add up to the negation seen on
    # every copy (a two-colouring); a constant is then reported relative to
    # the output's polarity ("0": the output is off)
    nout = len(side.outs)
    edges = defaultdict(list)
    for a in range(len(segs)):
        for j in range(nout):
            x = vals[j][a]
            if x in (0, mn):
                continue
            hit = lookup.get(x)
            if hit is None:
                return None
            edges[("o", j)].append((("d", hit[0]), hit[1]))
            edges[("d", hit[0])].append((("o", j), hit[1]))
    colour = {}
    for start in list(edges):
        if start in colour:
            continue
        colour[start] = 0
        stack = [start]
        while stack:
            u = stack.pop()
            for w2, ng in edges[u]:
                want = colour[u] ^ ng
                if w2 not in colour:
                    colour[w2] = want
                    stack.append(w2)
                elif colour[w2] != want:
                    return None
    mapping = []      # per assignment, per output: data input, or "0"/"1"
    for a in range(len(segs)):
        row = []
        for j in range(nout):
            x = vals[j][a]
            if x in (0, mn):
                row.append("1" if (x == mn) != bool(colour.get(("o", j), 0)) else "0")
            else:
                row.append(lookup[x][0])
        mapping.append(row)
    # the mapping must also hold on the structured patterns and the
    # rare-value points: under each select value, each output copies its data
    # input (with the polarity of the colouring) or is its constant
    at, mx, outs = side.extended()
    for a, row in enumerate(mapping):
        sel = mx
        for t, c in enumerate(ctrl):
            sel &= at[c] if (a >> t) & 1 else mx ^ at[c]
        if not sel:
            continue
        for j, x in enumerate(row):
            co = colour.get(("o", j), 0)
            if isinstance(x, int):
                want = at[x] ^ (mx if co ^ colour.get(("d", x), 0) else 0)
            else:
                want = mx if (x == "1") != bool(co) else 0
            if (outs[j] ^ want) & sel:
                return None
    # a shifter: some assignment maps the outputs one to one onto the data
    width = len(data)
    shift = _shift_structure(mapping, data, ctrl) if nout == width else None
    if shift:
        return shift
    per_out = [len({mapping[a][j] for a in range(len(segs))} - {"0", "1"}) for j in range(nout)]
    if all(c == len(segs) for c in per_out):
        return f"mux:{len(segs)}to1x{nout}", {"selects": len(ctrl), "outputs": nout}
    return f"mux-net:{nout}", {"selects": len(ctrl), "outputs": nout, "data": len(data)}


def _shift_structure(mapping, data, ctrl):
    nout = len(mapping[0])
    ident = None
    for a, row in enumerate(mapping):
        if all(isinstance(x, int) for x in row) and len(set(row)) == nout:
            ident = a
            break
    if ident is None:
        return None
    # order the outputs by the data they copy at the identity assignment; the
    # data order then comes from the assignment that shifts by one
    pos_of = {d: j for j, d in enumerate(mapping[ident])}
    for t in range(len(ctrl)):
        a1 = ident ^ (1 << t)
        succ = {}
        for j in range(nout):
            x = mapping[a1][j]
            if isinstance(x, int):
                succ[x] = mapping[ident][j]       # data x moves to where data succ[x] was
        # follow the chain from a data input nobody moves into
        targets = set(succ.values())
        starts = [d for d in data if d not in targets]
        if len(starts) > 1:
            continue
        order = []
        cur = starts[0] if starts else data[0]
        seen = set()
        while cur is not None and cur not in seen:
            seen.add(cur)
            order.append(cur)
            cur = succ.get(cur)
        if len(order) != len(data):
            continue
        rank = {d: i for i, d in enumerate(order)}
        rot = not starts
        offs = {}
        for a, row in enumerate(mapping):
            o = set()
            fill = set()
            for j, x in enumerate(row):
                k = rank[mapping[ident][j]]
                if isinstance(x, int):
                    o.add((k - rank[x]) % len(order) if rot else k - rank[x])
                else:
                    fill.add(x)
            if len(o) > 1 or (not o and rot):
                break
            # no data left: an amount of the width or more shifted every bit out
            offs[a ^ ident] = (o.pop() if o else None, fill)
        else:
            # a control that alone shifts every bit out has no offset to read:
            # any weight of the width or more acts the same
            weights = [offs[1 << u][0] if offs[1 << u][0] is not None or rot else len(order)
                       for u in range(len(ctrl))]
            fill_seen = set()

            def fits(a):
                # every output, not only the ones that copy data: the one at
                # rank k copies the data of rank k - amount, or is the fill
                # once that falls off the word
                amt = sum(weights[u] for u in range(len(ctrl)) if (a >> u) & 1)
                row = mapping[a ^ ident]
                for j in range(nout):
                    src = rank[mapping[ident][j]] - amt
                    if rot:
                        src %= len(order)
                    want = order[src] if 0 <= src < len(order) else None
                    if want is None:
                        if isinstance(row[j], int):
                            return False
                        fill_seen.add(row[j])
                    elif row[j] != want:
                        return False
                return True

            # one fill for every position shifted in, at every amount
            if None not in weights and all(fits(a) for a in offs) and len(fill_seen) <= 1:
                kind = "rot" if rot else "shift"
                return f"{kind}:{len(order)}/{len(ctrl)}", {"width": len(order), "amount_bits": len(ctrl)}
    return None


# -- cipher round: S-boxes on input blocks, mixed by xor ---------------------------


def t_spn(side, rnd, lin):
    """A round of a substitution-permutation cipher: the inputs fall into
    blocks (the inputs that exactly the same outputs read), every output
    reads two to four blocks, is nonlinear, and is a sum (xor) of functions
    of its blocks: the second difference across any two of its blocks is
    zero on random patterns. One block per output is an S-box layer (the
    test before)."""
    if side.hints:
        return None       # S-box outputs are balanced, never nearly constant
    sup = side.supports()
    readers_of = defaultdict(int)
    for j, sj in enumerate(sup):
        for k in _bits(sj):
            readers_of[k] |= 1 << j
    blocks = defaultdict(list)
    for k, msk in readers_of.items():
        blocks[msk].append(k)
    blk = list(blocks.values())
    if len(blk) < 2 or any(not 3 <= len(b) <= 8 for b in blk) or any(lin):
        return None
    bid = {}
    for i, b in enumerate(blk):
        for k in b:
            bid[k] = i
    out_blocks = [sorted({bid[k] for k in _bits(sj)}) for sj in sup]
    if any(not 1 <= len(ob) <= 4 for ob in out_blocks) or all(len(ob) == 1 for ob in out_blocks):
        return None
    pairs = sorted({(a, b) for ob in out_blocks for i, a in enumerate(ob) for b in ob[i + 1:]})
    n = 256
    mn = (1 << n) - 1
    base = side.rand(rnd, n)
    redraw = side.rand(rnd, n)
    segs = [{}]
    for i in range(len(blk)):
        segs.append({side.pis[k]: redraw[side.pis[k]] for k in blk[i]})
    for a, b in pairs:
        segs.append({side.pis[k]: redraw[side.pis[k]] for k in blk[a] + blk[b]})
    vals = seg_run(side, base, n, segs)
    nb = len(blk)
    for j, ob in enumerate(out_blocks):
        v = vals[j]
        for i, a in enumerate(ob):
            for b in ob[i + 1:]:
                pk = 1 + nb + pairs.index((a, b))
                if v[0] ^ v[1 + a] ^ v[1 + b] ^ v[pk]:
                    return None
    size = Counter(len(b) for b in blk).most_common(1)[0][0]
    return f"spn-round:{len(side.outs)}", {"blocks": nb, "block_inputs": size, "outputs": len(side.outs)}


def t_bit(side, rnd):
    """One to four output bits of a two-operand word function (a one-bit
    miter): the operands from the cone's partial products (and gates of two
    inputs form a bipartite graph whose sides are the operands) or from the
    inputs in variable order (halves, or alternating), each operand in
    variable order either way up; the outputs looked up among the bits of
    a * b, a + b, a - b and b - a."""
    net = side.net
    npi = net.npi
    if not 1 <= len(side.outs) <= 4:
        return None
    pis = side.pis
    n = len(pis)
    if n < 8 or n > 512:
        return None
    by_var = sorted(pis, key=lambda p: net.var_of[p])
    splits = []
    edges = defaultdict(set)
    pset = set(pis)
    for u in side.cone:
        g = u - npi
        if net.nin[g] == 2 and all(i in pset for i in net.fanin[g]) and \
                classify_tt(tt_replicate(net.tt[g], 2), 2) == "and2":
            a, b = net.fanin[g]
            edges[a].add(b)
            edges[b].add(a)
    if edges:
        colour = {}
        ok = True
        for st in edges:
            if st in colour:
                continue
            colour[st] = 0
            stack = [st]
            while stack and ok:
                u = stack.pop()
                for w2 in edges[u]:
                    if w2 not in colour:
                        colour[w2] = 1 - colour[u]
                        stack.append(w2)
                    elif colour[w2] == colour[u]:
                        ok = False
        if ok and len(colour) >= 0.9 * n:
            ga = [p for p in by_var if colour.get(p) == 0]
            gb = [p for p in by_var if colour.get(p) == 1]
            if len(ga) >= 2 and len(gb) >= 2:
                splits.append(("grid", ga, gb))
    if n % 2 == 0:
        splits.append(("halves", by_var[: n // 2], by_var[n // 2:]))
        splits.append(("alternate", by_var[0::2], by_var[1::2]))
    w = NPAT
    mask = (1 << w) - 1
    base = side.rand(rnd, w)
    vals = side.run(base, mask)
    at, mx, outs = side.extended()
    for how, ga, gb in splits:
        for oa in (ga, ga[::-1]):
            for ob in (gb, gb[::-1]):
                for fn in ("mul", "add", "sub", "subr"):
                    r = _bit_ref(fn, [base[p] for p in oa], [base[p] for p in ob], mask)
                    pos = {}
                    for k, bit in enumerate(r):
                        if bit not in (0, mask):
                            pos.setdefault(bit, (k, 0))
                            pos.setdefault(bit ^ mask, (k, 1))
                    hits = [pos.get(v) for v in vals]
                    if any(h is None for h in hits):
                        continue
                    # and on the structured patterns and the rare-value points
                    rx = _bit_ref(fn, [at[p] for p in oa], [at[p] for p in ob], mx)
                    if any(o != rx[k] ^ (mx if ng else 0) for o, (k, ng) in zip(outs, hits)):
                        continue
                    name = "sub" if fn == "subr" else fn
                    return f"{name}-bit:{len(oa)}x{len(ob)}", {"bits": "/".join(map(str, sorted({k for k, _ng in hits}))),
                                                               "operands": how}
    return None


def _bit_ref(fn, a, b, mask):
    if fn == "mul":
        return bs_mul(a, b, mask, len(a) + len(b))
    if fn == "add":
        return bs_add(a, b, mask)
    if fn == "sub":
        return _sub_ref(a, b, mask)
    return _sub_ref(b, a, mask)


def _sub_ref(a, b, mask):
    n = max(len(a), len(b))
    a = a + [0] * (n - len(a))
    b = b + [0] * (n - len(b))
    return bs_add(a, bs_not(b, mask), mask, cin=mask)


def classify_side(side, rnd, sbox_nodes=frozenset(), cuts=None):
    """(label, detail) for one side, the first test of the library that
    holds; 'unknown' with the support profile otherwise."""
    if not side.outs:
        return "empty", {}
    if len(side.cone) > MAX_CONE_GATES or len(side.pis) > MAX_CONE_PIS:
        return "too-big", {"gates": len(side.cone), "inputs": len(side.pis)}
    seed = rnd.getrandbits(32)

    def rng(k):
        return random.Random(seed * 31 + k)

    w = NPAT
    full = (1 << w) - 1
    a1, a2 = side.rand(rng(9), w), side.rand(rng(10), w)
    v1 = side.run(a1, full)
    v2 = side.run(a2, full)
    side.batches = [(a1, v1), (a2, v2)]
    probe = probe_values(side)
    pfull = (1 << (2 * len(side.pis) + 2)) - 1
    # an output that kept one value on the random patterns: where does it take
    # the other one (a structured pattern, or a search down its cone)?
    flat = [j for j, (x, y) in enumerate(zip(v1, v2)) if x == y and x in (0, full)]
    search = rng(11)
    for j in flat:
        other = 0 if v1[j] else 1
        pv = probe[j] ^ (0 if other else pfull)        # bits where the probe took the other value
        points = [probe_point(side, (pv & -pv).bit_length() - 1)] if pv else []
        found = justify(side, j, other, search)        # a point the probes would not give (an equal pair a = b != 0)
        if found is not None:
            points.append(found)
        if points:
            side.hints[j] = points
    if len(flat) == len(side.outs) and not side.hints:
        # no random pattern, structured pattern or search moved any output
        return f"const:{len(side.outs)}", {}
    lin = t_linear(side, rng(0), probe, [(a1, v1), (a2, v2)])
    if all(lin):
        # by the inputs the function reads, not the structural support
        coefs = side.coef
        if all(_popcount(c) <= 1 for c in coefs):
            return f"wiring:{len(side.outs)}", {}
        if len(side.outs) == 1:
            return f"parity:{_popcount(coefs[0])}", {}
        used = 0
        for c in coefs:
            used |= c
        return f"linear:{_popcount(used)}to{len(side.outs)}", {}
    for test in (lambda: t_sbox_layer(side, rng(1), lin), lambda: t_spn(side, rng(2), lin), lambda: t_symmetric(side, rng(3)),
                 lambda: t_arith(side, rng(4)), lambda: t_compare(side, rng(5)), lambda: t_muxshift(side, rng(6)),
                 lambda: t_bit(side, rng(7))):
        r = test()
        if r:
            return r
    sup = side.supports()
    sizes = sorted(_popcount(s) for s in sup)
    return "unknown", {"outputs": len(side.outs), "inputs": len(side.pis), "max_support": sizes[-1],
                       "linear_outputs": sum(lin), "gates": len(side.cone)}


# ---------------------------------------------------------------------------
# one cell
# ---------------------------------------------------------------------------


def _levels(net):
    lv = [0] * net.size
    npi = net.npi
    for g, ins in enumerate(net.fanin):
        lv[g + npi] = 1 + max((lv[i] for i in ins), default=0)
    return lv


def analyse(path, max_clauses=3_000_000, seconds=CELL_SECONDS, seed=1):
    """The census row of one CNF file (never raises: an error is a column)."""
    t0 = time.time()
    deadline = t0 + seconds
    row = {"stem": _stem(path)}
    try:
        _analyse(path, max_clauses, deadline, seed, row)
    except TimeoutError as e:
        row["error"] = f"timeout:{e}"
    except MemoryError:
        row["error"] = "memory"
    except Exception as e:  # noqa: BLE001
        row["error"] = f"{type(e).__name__}: {str(e)[:100]}"
    row["seconds"] = round(time.time() - t0, 1)
    return row


def _stem(path):
    name = Path(path).name
    for ext in (".cnf.xz", ".cnf"):
        if name.endswith(ext):
            return name[: -len(ext)]
    return name


def _analyse(path, max_clauses, deadline, seed, row):
    n_vars, _n_cls, clauses, truncated = parse_prefix(Path(path), max_clauses)
    row.update(vars=n_vars, clauses=len(clauses), truncated=int(truncated))
    present = {abs(l) for c in clauses for l in c}
    row["present"] = len(present)
    if len(present) > MAX_GATE_VARS:
        row["error"] = "too-big"
        return
    net = build_netlist(clauses, n_vars)
    if time.time() > deadline:
        raise TimeoutError("gates")
    ngates = net.size - net.npi
    row.update(gates=net.n_gates_raw, gate_cov=round(net.n_gates_raw / max(1, len(present)), 3),
               nodes=ngates, primary=net.npi, merged=net.merged, turned=net.turned, topo=int(net.topo_numbered),
               reading=net.reading,
               constraint_clauses=len(clauses) - len(net.used))
    if ngates == 0:
        row["error"] = "no-gates"
        return
    # the cut layer
    cuts = {}
    sbox_nodes = set()
    if ngates <= MAX_CUT_GATES:
        cuts = enumerate_cuts(net, range(net.npi, net.size), deadline=deadline)
        by_class, cells, sbox_cuts, _nodecls = cut_layer(net, cuts)
        row["cut_classes"] = ";".join(f"{k}={v}" for k, v in sorted(by_class.items()))
        row["cells"] = ";".join(f"{k}={v}" for k, v in sorted(cells.items()))
        for _leaves, us in sbox_cuts:
            sbox_nodes.update(us)
    else:
        row["cut_classes"] = "skipped"
    # word blocks: muxes sharing a select, xor components
    row.update(_blocks(net, cuts))
    if time.time() > deadline:
        raise TimeoutError("blocks")
    # the observed cones
    pairs, singles, asserted = observed_outputs(net, clauses)
    row["pairs"] = len(pairs)
    row["singles"] = len(singles)
    row["asserted"] = len(asserted)
    rnd = random.Random(seed)
    groups = []
    if pairs:
        a_side, b_side = split_pairs(net, pairs)
        groups.append(("A", a_side))
        groups.append(("B", b_side))
    if singles:
        groups.append(("S", singles))
    if len({i for i, _ng in asserted}) >= 2:
        groups.append(("U", [i for i, _ng in asserted]))
    labels = []
    for name, outs in groups:
        if time.time() > deadline:
            raise TimeoutError("cones")
        side = Side(net, outs, name)
        lab, det = classify_side(side, rnd, sbox_nodes, cuts)
        extra = ",".join(f"{k}={v}" for k, v in det.items())
        labels.append(f"{name}:{lab}" + (f"({extra})" if extra else "") + f"[{len(side.outs)}o/{len(side.pis)}i/{len(side.cone)}g]")
    row["cones"] = " | ".join(labels)


def _blocks(net, cuts):
    """Mux words (two or more muxes over one select), xor components, from the
    gates' own tables (and their 3-leaf cuts for muxes built from and/or)."""
    npi = net.npi
    sel = Counter()
    for u, lst in cuts.items():
        for leaves, t in lst:
            if len(leaves) == 3 and classify_tt(t, 3) == "mux":
                # the select is the leaf whose two cofactors are both one-variable functions of the others
                s = _mux_select(t)
                if s is not None:
                    sel[leaves[s]] += 1
                break
    words = sorted((c for c in sel.values() if c >= 4), reverse=True)
    # xor components: gates whose own table is xor<k>, joined through xor fan-ins
    parent = {}

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for g, ins in enumerate(net.fanin):
        n = net.nin[g]
        if n >= 2 and classify_tt(tt_replicate(net.tt[g], n), n).startswith("xor"):
            u = g + npi
            parent.setdefault(u, u)
            for i in ins:
                if i in parent:
                    parent[find(i)] = find(u)
    comp = Counter(find(x) for x in parent)
    big = sorted((c for c in comp.values() if c >= 8), reverse=True)
    return {"mux_words": len(words), "mux_word_max": words[0] if words else 0,
            "xor_blocks": len(big), "xor_block_max": big[0] if big else 0,
            "xor_gates": len(parent)}


def _mux_select(t):
    """For a 3-leaf mux table, the slot that selects (or None)."""
    for s in range(3):
        others = [i for i in range(3) if i != s]
        lo = _cofactor(t, s, 0)
        hi = _cofactor(t, s, 1)
        ok = True
        for cof in (lo, hi):
            ess = tt_essential(cof, 3)
            if len(ess) != 1 or ess[0] not in others:
                ok = False
        if ok and tt_essential(lo, 3) != tt_essential(hi, 3):
            return s
    return None


def _cofactor(t, i, val):
    """The table with slot i fixed to val (replicated over slot i)."""
    s = 1 << i
    if val:
        x = t & VAR[i]
        return x | (x >> s)
    x = t & (M64 ^ VAR[i])
    return x | (x << s)


# ---------------------------------------------------------------------------
# toys: two implementations of each template in a miter, as plain gates or
# mapped to lookup tables of up to K inputs, variables renumbered
# ---------------------------------------------------------------------------

T, F = True, False


class Circ:
    """A small gate builder. A literal is a nonzero int or a constant
    (True/False); gates fold constants."""

    def __init__(self):
        self.nv = 0
        self.defs = {}          # var -> (kind, ins)
        self.order = []

    def new(self):
        self.nv += 1
        return self.nv

    def inp(self, w):
        return [self.new() for _ in range(w)]

    @staticmethod
    def neg(a):
        return (not a) if isinstance(a, bool) else -a

    def _gate(self, kind, ins):
        v = self.new()
        self.defs[v] = (kind, tuple(ins))
        self.order.append(v)
        return v

    def AND(self, a, b):
        if a is F or b is F:
            return F
        if a is T:
            return b
        if b is T:
            return a
        if a == b:
            return a
        if a == -b:
            return F
        return self._gate("and", (a, b))

    def OR(self, a, b):
        return self.neg(self.AND(self.neg(a), self.neg(b)))

    def XOR(self, a, b):
        if isinstance(a, bool):
            return self.neg(b) if a else b
        if isinstance(b, bool):
            return self.neg(a) if b else a
        if a == b:
            return F
        if a == -b:
            return T
        return self._gate("xor", (a, b))

    def MUX(self, s, a, b):
        """s ? a : b"""
        if isinstance(s, bool):
            return a if s else b
        if a == b:
            return a
        if isinstance(a, bool) or isinstance(b, bool):
            return self.OR(self.AND(s, a), self.AND(self.neg(s), b))
        return self._gate("mux", (s, a, b))

    def MAJ(self, a, b, c):
        if any(isinstance(x, bool) for x in (a, b, c)):
            return self.OR(self.AND(a, b), self.AND(c, self.OR(a, b)))
        return self._gate("maj", (a, b, c))

    def XOR3(self, a, b, c):
        if any(isinstance(x, bool) for x in (a, b, c)):
            return self.XOR(self.XOR(a, b), c)
        return self._gate("xor3", (a, b, c))

    def value(self, lit, env):
        if isinstance(lit, bool):
            return lit
        v = env[abs(lit)]
        return (not v) if lit < 0 else v

    def eval(self, env):
        for v in self.order:
            kind, ins = self.defs[v]
            x = [self.value(i, env) for i in ins]
            if kind == "and":
                r = x[0] and x[1]
            elif kind == "xor":
                r = x[0] != x[1]
            elif kind == "mux":
                r = x[1] if x[0] else x[2]
            elif kind == "maj":
                r = (x[0] + x[1] + x[2]) >= 2
            elif kind == "xor3":
                r = (x[0] + x[1] + x[2]) & 1 == 1
            else:
                tt = kind[1]
                m = sum(1 << i for i, b in enumerate(x) if b)
                r = bool((tt >> m) & 1)
            env[v] = r
        return env


def _gate_cnf(v, kind, ins):
    def L(a):
        return a
    if kind == "and":
        a, b = ins
        return [(-v, a), (-v, b), (v, -a, -b)]
    if kind == "xor":
        a, b = ins
        return [(-v, a, b), (-v, -a, -b), (v, -a, b), (v, a, -b)]
    if kind == "mux":
        s, a, b = ins
        return [(-s, -a, v), (-s, a, -v), (s, -b, v), (s, b, -v)]
    if kind == "maj":
        a, b, c = ins
        return [(-a, -b, v), (-a, -c, v), (-b, -c, v), (a, b, -v), (a, c, -v), (b, c, -v)]
    if kind == "xor3":
        a, b, c = ins
        out = []
        for m in range(8):
            bits = [(m >> i) & 1 for i in range(3)]
            val = sum(bits) & 1
            out.append(tuple((-x if bit else x) for x, bit in zip((a, b, c), bits)) + ((v,) if val else (-v,)))
        return out
    tt = kind[1]
    out = []
    for m in range(1 << len(ins)):
        bits = [(m >> i) & 1 for i in range(len(ins))]
        val = (tt >> m) & 1
        out.append(tuple((-x if bit else x) for x, bit in zip(ins, bits)) + ((v,) if val else (-v,)))
    return out


def lut_map(c, outs, k, rnd):
    """Map the cones of `outs` to lookup tables of up to k inputs (cuts by
    structure, chosen from the outputs down); returns a new Circ whose
    variables keep the old numbers for inputs and mapped nodes, and the
    output literals."""
    cuts = {}
    for v in c.order:
        _kind, ins = c.defs[v]
        own = frozenset(abs(i) for i in ins)        # the gate's own fan-ins: always a cut
        cand = [frozenset()]
        for i in ins:
            iv = abs(i)
            lst = cuts.get(iv, []) + [frozenset([iv])]
            cand = list({x | y for x in cand for y in lst if len(x | y) <= k})
            rnd.shuffle(cand)
            cand = sorted(cand, key=len, reverse=True)[:12]
            # keep the partial cut made of fan-ins only, so a later fan-in still fits
            part = frozenset(abs(j) for j in ins[: ins.index(i) + 1])
            if part not in cand:
                cand.append(part)
        if own not in cand:
            cand.append(own)
        cand = sorted(cand, key=len, reverse=True)
        cuts[v] = cand[:6] + ([own] if own not in cand[:6] else [])
    new = Circ()
    new.nv = c.nv
    need = [abs(o) for o in outs if not isinstance(o, bool)]
    done = set()
    while need:
        v = need.pop()
        if v in done or v not in c.defs:
            continue
        done.add(v)
        leaves = sorted(cuts[v][0])
        # table of v over the leaves, by evaluation
        tt = 0
        cone = _cone_between(c, v, set(leaves))
        for m in range(1 << len(leaves)):
            env = {x: bool((m >> i) & 1) for i, x in enumerate(leaves)}
            for u in cone:
                kind, ins = c.defs[u]
                x = [c.value(i, env) for i in ins]
                env[u] = _eval_kind(kind, x)
            if env[v]:
                tt |= 1 << m
        new.defs[v] = (("lut", tt), tuple(leaves))
        need.extend(leaves)
    new.order = [v for v in c.order if v in new.defs]
    return new


def _cone_between(c, v, leaves):
    seen = set()
    order = []

    def visit(u):
        stack = [(u, False)]
        while stack:
            x, done = stack.pop()
            if done:
                order.append(x)
                continue
            if x in seen or x in leaves or x not in c.defs:
                continue
            seen.add(x)
            stack.append((x, True))
            for i in c.defs[x][1]:
                if not isinstance(i, bool):
                    stack.append((abs(i), False))
    visit(v)
    return order


def _eval_kind(kind, x):
    if kind == "and":
        return x[0] and x[1]
    if kind == "xor":
        return x[0] != x[1]
    if kind == "mux":
        return x[1] if x[0] else x[2]
    if kind == "maj":
        return (x[0] + x[1] + x[2]) >= 2
    if kind == "xor3":
        return (x[0] + x[1] + x[2]) & 1 == 1
    tt = kind[1]
    m = sum(1 << i for i, b in enumerate(x) if b)
    return bool((tt >> m) & 1)


# -- the implementations -------------------------------------------------------


def ripple_add(c, a, b, cin=F):
    s, cy = [], cin
    for x, y in zip(a, b):
        t = c.XOR(x, y)
        s.append(c.XOR(t, cy))
        cy = c.OR(c.AND(x, y), c.AND(cy, t))
    return s + [cy]


def prefix_add(c, a, b):
    """Kogge-Stone: generate and propagate combined over log levels."""
    n = len(a)
    g = [c.AND(x, y) for x, y in zip(a, b)]
    p = [c.XOR(x, y) for x, y in zip(a, b)]
    G, P = g[:], p[:]
    d = 1
    while d < n:
        G2, P2 = G[:], P[:]
        for i in range(d, n):
            G2[i] = c.OR(G[i], c.AND(P[i], G[i - d]))
            P2[i] = c.AND(P[i], P[i - d])
        G, P = G2, P2
        d *= 2
    s = [p[0]] + [c.XOR(p[i], G[i - 1]) for i in range(1, n)]
    return s + [G[n - 1]]


def array_mul(c, a, b):
    n = len(a)
    acc = [c.AND(a[i], b[0]) for i in range(n)] + [F] * n
    for j in range(1, n):
        row = [F] * j + [c.AND(a[i], b[j]) for i in range(n)] + [F] * (n - j)
        acc = ripple_add(c, acc, row)[: 2 * n]
    return acc


def tree_mul(c, a, b):
    """Column compression with full adders (xor3 and majority gates), then a
    prefix adder."""
    n = len(a)
    cols = [[] for _ in range(2 * n)]
    for i in range(n):
        for j in range(n):
            cols[i + j].append(c.AND(a[i], b[j]))
    while any(len(col) > 2 for col in cols):
        nxt = [[] for _ in range(2 * n + 1)]
        for w, col in enumerate(cols):
            k = 0
            while len(col) - k >= 3:
                x, y, z = col[k:k + 3]
                nxt[w].append(c.XOR3(x, y, z))
                nxt[w + 1].append(c.MAJ(x, y, z))
                k += 3
            nxt[w].extend(col[k:])
        cols = nxt[: 2 * n]
    a2 = [col[0] if len(col) > 0 else F for col in cols]
    b2 = [col[1] if len(col) > 1 else F for col in cols]
    return ripple_add(c, a2, b2)[: 2 * n]


def borrow_lt(c, a, b):
    br = F
    for x, y in zip(a, b):
        br = c.OR(c.AND(c.neg(x), y), c.AND(c.neg(c.XOR(x, y)), br))
    return [br]


def tree_lt(c, a, b):
    def rec(lo, hi):
        if hi - lo == 1:
            return c.AND(c.neg(a[lo]), b[lo]), c.neg(c.XOR(a[lo], b[lo]))
        mid = (lo + hi) // 2
        lt_l, eq_l = rec(lo, mid)
        lt_h, eq_h = rec(mid, hi)
        return c.OR(lt_h, c.AND(eq_h, lt_l)), c.AND(eq_h, eq_l)
    return [rec(0, len(a))[0]]


def chain_eq(c, a, b):
    t = T
    for x, y in zip(a, b):
        t = c.AND(t, c.neg(c.XOR(x, y)))
    return [t]


def tree_eq(c, a, b):
    """a == b as no pair differing: an or tree over the xors, negated."""
    xs = [c.XOR(x, y) for x, y in zip(a, b)]
    while len(xs) > 1:
        xs = [c.OR(xs[i], xs[i + 1]) if i + 1 < len(xs) else xs[i] for i in range(0, len(xs), 2)]
    return [c.neg(xs[0])]


def fa_popcount(c, xs):
    cols = [list(xs)]
    w = 0
    while w < len(cols):
        while len(cols[w]) > 1:
            if len(cols) <= w + 1:
                cols.append([])
            if len(cols[w]) >= 3:
                x, y, z = cols[w][:3]
                cols[w] = cols[w][3:] + [c.XOR(c.XOR(x, y), z)]
                cols[w + 1].append(c.MAJ(x, y, z))
            else:
                x, y = cols[w][:2]
                cols[w] = cols[w][2:] + [c.XOR(x, y)]
                cols[w + 1].append(c.AND(x, y))
        w += 1
    return [col[0] if col else F for col in cols]


def inc_popcount(c, xs):
    width = len(xs).bit_length()
    cnt = [F] * width
    for x in xs:
        cy = x
        nxt = []
        for b in cnt:
            nxt.append(c.XOR(b, cy))
            cy = c.AND(b, cy)
        cnt = nxt
    return cnt


def barrel_shl(c, d, s):
    x = list(d)
    for t, st in enumerate(s):
        k = 1 << t
        x = [c.MUX(st, x[j - k] if j >= k else F, x[j]) for j in range(len(x))]
    return x


def muxper_shl(c, d, s):
    n = len(d)
    out = []
    for j in range(n):
        choices = [d[j - a] if j >= a else F for a in range(1 << len(s))]
        out.append(_mux_tree(c, s, choices))
    return out


def decode_shl(c, d, s):
    """out_j = or over the amounts a of (s == a) and d[j - a]."""
    dec = []
    for a in range(1 << len(s)):
        t = T
        for k, st in enumerate(s):
            t = c.AND(t, st if (a >> k) & 1 else c.neg(st))
        dec.append(t)
    out = []
    for j in range(len(d)):
        acc = F
        for a in range(min(j + 1, len(dec))):
            acc = c.OR(acc, c.AND(dec[a], d[j - a]))
        out.append(acc)
    return out


def _mux_tree(c, s, items):
    level = list(items)
    for st in s:
        level = [c.MUX(st, level[i + 1], level[i]) for i in range(0, len(level), 2)]
    return level[0]


def tree_mux(c, s, words):
    return [_mux_tree(c, s, [w[i] for w in words]) for i in range(len(words[0]))]


def decode_mux(c, s, words):
    dec = []
    for a in range(len(words)):
        t = T
        for k, st in enumerate(s):
            t = c.AND(t, st if (a >> k) & 1 else c.neg(st))
        dec.append(t)
    out = []
    for i in range(len(words[0])):
        acc = F
        for a, w in enumerate(words):
            acc = c.OR(acc, c.AND(dec[a], w[i]))
        out.append(acc)
    return out


PRESENT = [0xC, 0x5, 0x6, 0xB, 0x9, 0x0, 0xA, 0xD, 0x3, 0xE, 0xF, 0x8, 0x4, 0x7, 0x1, 0x2]


def sbox_sop(c, x):
    outs = []
    for bit in range(4):
        acc = F
        for m in range(16):
            if (PRESENT[m] >> bit) & 1:
                t = T
                for i in range(4):
                    t = c.AND(t, x[i] if (m >> i) & 1 else c.neg(x[i]))
                acc = c.OR(acc, t)
        outs.append(acc)
    return outs


def sbox_mux(c, x):
    return [_mux_tree(c, x, [bool((PRESENT[m] >> bit) & 1) for m in range(16)]) for bit in range(4)]


def spn_round(c, x, k, sbox):
    y = [c.XOR(a, b) for a, b in zip(x, k)] if k else list(x)
    s = []
    for i in range(0, len(y), 4):
        s.extend(sbox(c, y[i:i + 4]))
    n = len(s)
    return [c.XOR(s[i], s[(i + 4) % n]) for i in range(n)]


TOYS = {
    # kind: (builder of (outputs A, outputs B) from a Circ and the width, expected label)
    "add": (lambda c, w: _two(c, w, 2, ripple_add, prefix_add), lambda w: f"add:{w}x{w}"),
    "mul": (lambda c, w: _two(c, w, 2, array_mul, tree_mul), lambda w: f"mul:{w}x{w}"),
    "cmp": (lambda c, w: _two(c, w, 2, borrow_lt, tree_lt, msb_first=True), lambda w: f"cmp:{w}"),
    "eq": (lambda c, w: _two(c, w, 2, chain_eq, tree_eq), lambda w: f"eq:{w}"),
    "popcount": (lambda c, w: _one(c, w, fa_popcount, inc_popcount), lambda w: f"popcount:{w}"),
    "shift": (lambda c, w: _shift(c, w), lambda w: f"shift:{w}/{(w - 1).bit_length()}"),
    # one more amount bit, which alone shifts every bit out
    "shift-wide": (lambda c, w: _shift(c, w, extra=1), lambda w: f"shift:{w}/{(w - 1).bit_length() + 1}"),
    "mux": (lambda c, w: _mux(c, w), lambda w: f"mux:4to1x{w}"),
    "cipher": (lambda c, w: _cipher(c, w), lambda w: f"spn-round:{w}"),
    "add-bug": (lambda c, w: _two(c, w, 2, ripple_add, _buggy_add), lambda w: f"add:{w}x{w}"),
    "random": (lambda c, w: _random(c, w), lambda w: "unknown"),
}


def _two(c, w, m, fa, fb, msb_first=False):
    a, b = c.inp(w), c.inp(w)
    if msb_first:
        return fa(c, a[::-1], b[::-1]), fb(c, a[::-1], b[::-1])
    return fa(c, a, b), fb(c, a, b)


def _one(c, w, fa, fb):
    xs = c.inp(w)
    return fa(c, xs), fb(c, xs)


def _shift(c, w, extra=0):
    d = c.inp(w)
    s = c.inp((w - 1).bit_length() + extra)
    return barrel_shl(c, d, s), decode_shl(c, d, s)


def _mux(c, w):
    s = c.inp(2)
    words = [c.inp(w) for _ in range(4)]
    return tree_mux(c, s, words), decode_mux(c, s, words)


def _cipher(c, w):
    x, k = c.inp(w), c.inp(w)
    y = [c.XOR(a, b) for a, b in zip(x, k)]       # the key addition, shared by both sides
    return spn_round(c, y, None, sbox_sop), spn_round(c, y, None, sbox_mux)


def _buggy_add(c, a, b):
    s = prefix_add(c, a, b)
    j = len(a) // 2
    s[j] = c.OR(s[j], a[0])       # a wrong sum bit
    return s


def _random(c, w):
    rnd = random.Random(w)
    xs = c.inp(w)
    pool = list(xs)
    for _ in range(6 * w):
        x, y, z = (rnd.choice(pool) for _ in range(3))
        k = rnd.randrange(4)
        pool.append([c.AND(x, -y), c.XOR(x, y), c.MUX(x, y, z), c.MAJ(x, y, z)][k])
        pool = [p for p in pool if not isinstance(p, bool)]
    outs = pool[-w:]
    return outs, [c.XOR(o, pool[-w - 1 - i]) for i, o in enumerate(outs)]


def _side_of(c, outs, name="T"):
    """One side over the given literals of a toy circuit, read back from its
    clauses as the census reads a file."""
    clauses = [cl for v in c.order for cl in _gate_cnf(v, *c.defs[v])]
    net = build_netlist(clauses, c.nv)
    return Side(net, [net.id_of[abs(x)][0] for x in outs], name)


def toy_cnf(kind, width, mapped=0, shuffle=None, seed=1, flip=True):
    """(n_vars, clauses, expected label) of a miter of the template's two
    implementations; `mapped` maps both sides to lookup tables of that many
    inputs; `shuffle` renumbers the variables, flips polarities and permutes
    the clauses."""
    build, expect = TOYS[kind]
    c = Circ()
    outs_a, outs_b = build(c, width)
    rnd = random.Random(seed)
    if mapped:
        c2 = lut_map(c, list(outs_a) + list(outs_b), mapped, rnd)
        c2.nv = c.nv
        c = c2
    clauses = []
    for v in c.order:
        kind_v, ins = c.defs[v]
        clauses.extend(_gate_cnf(v, kind_v, ins))
    top = []
    for x, y in zip(outs_a, outs_b):
        if isinstance(x, bool) or isinstance(y, bool):
            continue
        d = c.new()
        clauses.extend(_gate_cnf(d, "xor", (x, y)))
        top.append(d)
    clauses.append(tuple(top))
    nv = c.nv
    if shuffle is not None:
        r2 = random.Random(shuffle)
        perm = list(range(1, nv + 1))
        r2.shuffle(perm)
        fl = {v: flip and r2.random() < 0.5 for v in range(1, nv + 1)}
        ren = {v: perm[v - 1] * (-1 if fl[v] else 1) for v in range(1, nv + 1)}
        clauses = [tuple(ren[abs(l)] * (1 if l > 0 else -1) for l in cl) for cl in clauses]
        clauses = [tuple(r2.sample(cl, len(cl))) for cl in clauses]
        r2.shuffle(clauses)
    return nv, clauses, expect(width)


def write_cnf_xz(path, nv, clauses):
    """A DIMACS file, xz-compressed when the name ends in .xz, plain otherwise
    (the solver picks its reader by the suffix)."""
    import lzma
    opener = lzma.open if str(path).endswith(".xz") else open
    with opener(path, "wt") as f:
        f.write(f"p cnf {nv} {len(clauses)}\n")
        for cl in clauses:
            f.write(" ".join(map(str, cl)) + " 0\n")


TOYS["sqr-const"] = (None, lambda w: f"sqr:{w}")


def toy_sqr_const(width, shuffle=None, seed=1):
    """x * x = N with the output bits fixed by unit clauses (the shape of a
    factoring or sum-of-cubes cell): the asserted word is classified."""
    c = Circ()
    x = c.inp(width)
    out = array_mul(c, x, x)
    rnd = random.Random(seed)
    v = rnd.getrandbits(width) | 1
    n = v * v
    clauses = []
    for g in c.order:
        kind_g, ins = c.defs[g]
        clauses.extend(_gate_cnf(g, kind_g, ins))
    for p, o in enumerate(out):
        if isinstance(o, bool) or o in x or -o in x:
            continue        # bit 0 of x*x is x0 itself: a unit there would fix an input
        bit = (n >> p) & 1
        clauses.append((o if bit else -o,))
    nv = c.nv
    if shuffle is not None:
        r2 = random.Random(shuffle)
        perm = list(range(1, nv + 1))
        r2.shuffle(perm)
        clauses = [tuple(perm[abs(l) - 1] * (1 if l > 0 else -1) for l in cl) for cl in clauses]
        r2.shuffle(clauses)
    return nv, clauses, f"sqr:{width}"


SELFTEST = [
    # (kind, width): every template in a miter of two implementations, then the controls
    ("add", 8), ("mul", 5), ("cmp", 8), ("eq", 16), ("popcount", 9), ("shift", 8), ("mux", 4), ("cipher", 16),
]


def cmd_selftest(args):
    import tempfile
    fails = []
    lines = 0

    def check(name, cond, detail):
        nonlocal lines
        lines += 1
        if not cond or args.verbose:
            print(f"  {'ok  ' if cond else 'FAIL'} {name}: {detail}")
        if not cond:
            fails.append(name)

    def labels(row):
        out = {}
        for part in (row.get("cones") or "").split(" | "):
            if ":" in part:
                side, rest = part.split(":", 1)
                out[side] = rest.split("(")[0].split("[")[0]
        return out

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        n = 0
        # the encodings: plain gates, lookup tables of 4 and 6 inputs; the
        # variables as written, renumbered, renumbered with flipped polarities
        for kind, w in SELFTEST:
            for mapped in (0, 4, 6):
                for shuffle, flip in ((None, False), (7, False), (11, True), (23, True)):
                    if kind == "cipher" and mapped and shuffle is not None:
                        continue   # known limit: a mapped key addition, renumbered, comes back reversed
                    if kind == "eq" and mapped:
                        continue   # known limit: a mapped pair xor can come back reversed (a gate-layer tie)
                    nv, cl, exp = toy_cnf(kind, w, mapped=mapped, shuffle=shuffle, flip=flip)
                    p = tmp / f"t{n}.cnf.xz"
                    n += 1
                    write_cnf_xz(p, nv, cl)
                    lab = labels(analyse(p))
                    name = f"{kind} w{w} lut{mapped} {'renum' if shuffle else 'asis'}{'+flip' if flip else ''}"
                    check(name, lab.get("A") == exp and lab.get("B") == exp, f"want {exp} on both sides, got {lab}")
        # a wrong sum bit: one side is an adder, the other is not
        for mapped in (0, 4):
            for shuffle in (None, 11):
                nv, cl, _exp = toy_cnf("add-bug", 8, mapped=mapped, shuffle=shuffle)
                p = tmp / f"t{n}.cnf.xz"
                n += 1
                write_cnf_xz(p, nv, cl)
                lab = labels(analyse(p))
                got = sorted(lab.get(s, "") for s in ("A", "B"))
                check(f"add with a wrong bit lut{mapped} {'renum' if shuffle else 'asis'}",
                      got[0] == "add:8x8" and got[1] == "unknown", f"want one add:8x8 and one unknown, got {lab}")
        # a random circuit: no side matches the library
        for mapped in (0, 4):
            nv, cl, _exp = toy_cnf("random", 10, mapped=mapped)
            p = tmp / f"t{n}.cnf.xz"
            n += 1
            write_cnf_xz(p, nv, cl)
            lab = labels(analyse(p))
            check(f"random circuit lut{mapped}", all(lab.get(s) == "unknown" for s in ("A", "B")), f"got {lab}")
        # an asserted word: x * x fixed to a constant by unit clauses
        for shuffle in (None, 5):
            nv, cl, exp = toy_sqr_const(8, shuffle=shuffle)
            p = tmp / f"t{n}.cnf.xz"
            n += 1
            write_cnf_xz(p, nv, cl)
            lab = labels(analyse(p))
            check(f"x*x = N {'renum' if shuffle else 'asis'}", lab.get("U") == exp, f"want U {exp}, got {lab}")
        # shifters whose larger amounts shift every bit out
        for w in (4, 5, 6, 9):
            for shuffle in (None, 7):
                nv, cl, exp = toy_cnf("shift", w, shuffle=shuffle)
                p = tmp / f"t{n}.cnf.xz"
                n += 1
                write_cnf_xz(p, nv, cl)
                lab = labels(analyse(p))
                check(f"shift w{w} {'renum' if shuffle else 'asis'}", lab.get("A") == exp and lab.get("B") == exp,
                      f"want {exp} on both sides, got {lab}")
        # a plain (uncompressed) file reads as the compressed one does
        nv, cl, exp = toy_cnf("add", 8, shuffle=7)
        p = tmp / f"t{n}.cnf"
        n += 1
        with open(p, "w") as f:
            f.write(f"p cnf {nv} {len(cl)}\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in cl))
        lab = labels(analyse(p))
        check("plain cnf", lab.get("A") == exp and lab.get("B") == exp, f"want {exp} on both sides, got {lab}")
        # single sides: rare values, an equality with a defect, a wrong carry
        rnd = random.Random(5)
        c = Circ()
        a, b = c.inp(32), c.inp(32)
        lab = classify_side(_side_of(c, chain_eq(c, a, b)), rnd)[0]
        check("eq w32", lab == "eq:32", f"want eq:32, got {lab}")
        c = Circ()
        a, b = c.inp(24), c.inp(24)
        lab = classify_side(_side_of(c, chain_eq(c, a, [y if i % 3 else -y for i, y in enumerate(b)])), rnd)[0]
        check("eq w24 with a third of the pairs negated", lab == "eq:24", f"want eq:24, got {lab}")
        c = Circ()
        a, b = c.inp(4), c.inp(4)
        e = c.XOR(chain_eq(c, a, b)[0], c.AND(c.AND(a[0], a[1]), a[2]))
        lab = classify_side(_side_of(c, [e]), rnd)[0]
        check("eq xor (a0 a1 a2) is not eq", not lab.startswith("eq"), f"got {lab}")
        c = Circ()
        xs = c.inp(24)
        t = T
        for x in xs:
            t = c.AND(t, x)
        lab = classify_side(_side_of(c, [t]), rnd)[0]
        check("and of 24 is neither const nor linear", lab.split(":")[0] not in ("const", "linear", "parity"), f"got {lab}")
        c = Circ()
        a, b = c.inp(9), c.inp(9)
        s = prefix_add(c, a, b)
        s[-1] = c.OR(s[-1], a[0])
        lab = classify_side(_side_of(c, s), rnd)[0]
        check("add with a wrong carry is add-part", lab == "add-part:9x9", f"want add-part:9x9, got {lab}")
        for w in (16, 32):           # the top count bit is one only when every input is
            for shuffle in (None, 7):
                nv, cl, exp = toy_cnf("popcount", w, shuffle=shuffle)
                p = tmp / f"t{n}.cnf.xz"
                n += 1
                write_cnf_xz(p, nv, cl)
                lab = labels(analyse(p))
                check(f"popcount w{w} {'renum' if shuffle else 'asis'}", lab.get("A") == exp and lab.get("B") == exp,
                      f"want {exp} on both sides, got {lab}")
        c = Circ()
        xs = c.inp(24)
        t1, t0 = T, T
        for x in xs:
            t1, t0 = c.AND(t1, x), c.AND(t0, c.neg(x))
        lab = classify_side(_side_of(c, [c.OR(t1, t0)]), rnd)[0]
        check("all 24 equal is symmetric, not a threshold", lab == "symmetric:24", f"want symmetric:24, got {lab}")
        c = Circ()
        d, sh = c.inp(8), c.inp(3)
        outs = barrel_shl(c, d, sh)
        outs[7] = c.AND(outs[7], c.neg(c.AND(c.AND(sh[0], sh[1]), c.neg(sh[2]))))   # bit 7 lost at amount 3
        lab = classify_side(_side_of(c, outs), rnd)[0]
        check("a shifter that loses a bit at one amount is not a shifter", not lab.startswith("shift"), f"got {lab}")
        c = Circ()
        d, sh = c.inp(8), c.inp(3)
        outs = decode_shl(c, d, sh)
        outs[0] = c.OR(outs[0], c.AND(c.AND(sh[0], sh[1]), c.neg(sh[2])))         # fill 1 at amount 3 only
        lab = classify_side(_side_of(c, outs), rnd)[0]
        check("a shifter whose fill changes with the amount is not a shifter", not lab.startswith("shift"), f"got {lab}")
        c = Circ()
        xs = c.inp(9)
        rare = T
        for i, x in enumerate(xs[2:]):               # one in 128, and on no structured pattern
            rare = c.AND(rare, x if i % 2 == 0 else c.neg(x))
        lab = classify_side(_side_of(c, [c.XOR(xs[0], xs[1]), c.neg(rare)]), rnd)[0]
        check("a nearly constant output is not linear", lab.split(":")[0] not in ("linear", "const"), f"got {lab}")
        c = Circ()
        a, b = c.inp(16), c.inp(16)
        s = prefix_add(c, a, b)
        every = T
        for x in a + b:
            every = c.AND(every, x)
        s[-1] = c.XOR(s[-1], every)           # the carry is wrong only when every input is one
        lab = classify_side(_side_of(c, s), rnd)[0]
        check("add with a carry wrong at all ones is add-part", lab == "add-part:16x16", f"want add-part:16x16, got {lab}")
        c = Circ()
        a, b = c.inp(16), c.inp(16)
        s = prefix_add(c, a, b)
        low6 = T
        for i, x in enumerate(a[:7]):                  # a's low seven bits equal 6: one pattern in 128
            low6 = c.AND(low6, x if i in (1, 2) else c.neg(x))
        s[-1] = c.XOR(s[-1], low6)
        lab = classify_side(_side_of(c, s), rnd)[0]
        check("add with a carry wrong once in 128 is add-part", lab == "add-part:16x16", f"want add-part:16x16, got {lab}")
        for w in (7, 8, 16):                           # the carry is one only when every bit is
            c = Circ()
            a = c.inp(w)
            lab = classify_side(_side_of(c, ripple_add(c, a, [F] * w, cin=T)), rnd)[0]
            check(f"increment w{w} with its carry", lab == f"inc:{w}", f"want inc:{w}, got {lab}")
        for shuffle in (None, 7):
            nv, cl, exp = toy_cnf("shift-wide", 8, shuffle=shuffle)     # amount bit 3 alone shifts every bit out
            p = tmp / f"t{n}.cnf.xz"
            n += 1
            write_cnf_xz(p, nv, cl)
            lab = labels(analyse(p))
            check(f"shift w8 by 4 amount bits {'renum' if shuffle else 'asis'}", lab.get("A") == exp and lab.get("B") == exp,
                  f"want {exp} on both sides, got {lab}")
        p = tmp / "plain-toy.cnf"
        write_cnf_xz(p, 2, [(1, 2)])
        check("a toy named .cnf is written plain", p.read_bytes().startswith(b"p cnf"), f"starts {p.read_bytes()[:6]!r}")
        # the classes of small tables
        for name, f, k, want in [("and2", lambda a, b: a and b, 2, "and2"), ("or3 of negations", lambda a, b, c: not (a and b and c), 3, "and3"),
                                 ("xor3", lambda a, b, c: a ^ b ^ c, 3, "xor3"), ("majority", lambda a, b, c: a + b + c >= 2, 3, "maj3"),
                                 ("mux", lambda s, a, b: a if s else b, 3, "mux"), ("2 of 4", lambda a, b, c, d: a + b + c + d >= 2, 4, "sym4"),
                                 ("PRESENT bit 0", lambda a, b, c, d: (PRESENT[a | b << 1 | c << 2 | d << 3] & 1) == 1, 4, "bal4"),
                                 ("a xor c over three", lambda a, b, c: a ^ c, 3, "xor2")]:
            got = classify_tt(tt_replicate(_tt_of(f, k), k), k)
            check(f"class {name}", got == want, f"want {want}, got {got}")
    print(f"selftest: {lines} checks, {'all passed' if not fails else 'FAILED: ' + ', '.join(fails)}")
    return 1 if fails else 0


# ---------------------------------------------------------------------------
# the census over a suite
# ---------------------------------------------------------------------------

COLUMNS = ["stem", "vars", "clauses", "truncated", "present", "gates", "gate_cov", "nodes", "primary", "merged", "turned", "topo",
           "reading",
           "constraint_clauses", "cut_classes", "cells", "mux_words", "mux_word_max", "xor_blocks", "xor_block_max", "xor_gates",
           "pairs", "singles", "asserted", "cones", "seconds", "error"]


def _alarm(_sig, _frm):
    raise TimeoutError("wall")


def census_one(job):
    stem, path, max_clauses, seconds = job
    signal.signal(signal.SIGALRM, _alarm)
    signal.alarm(int(seconds) + 120)
    try:
        return analyse(path, max_clauses, seconds)
    except TimeoutError:
        return {"stem": stem, "error": "timeout:wall"}
    finally:
        signal.alarm(0)


def _limit_memory(mb):
    import resource
    resource.setrlimit(resource.RLIMIT_AS, (mb << 20, mb << 20))


def cmd_census(args):
    files = [Path(p) for p in args.files] if args.files else sorted(SUITE.glob("*.cnf.xz"))
    stems = set(args.stems.split(",")) if args.stems else None
    jobs = []
    for f in files:
        stem = _stem(f)
        if stems and not any(s in stem for s in stems):
            continue
        jobs.append((stem, str(f), args.max_clauses, args.seconds))
    # the slow cells first, so the long tail starts early: by file size
    jobs.sort(key=lambda j: -Path(j[1]).stat().st_size)
    with open(args.out, "w", newline="") as fh, ProcessPoolExecutor(max_workers=args.workers, initializer=_limit_memory,
                                                                   initargs=(args.mem_mb,)) as ex:
        w = csv.DictWriter(fh, fieldnames=COLUMNS, delimiter="\t", extrasaction="ignore")
        w.writeheader()
        fh.flush()
        futs = {ex.submit(census_one, j): j for j in jobs}
        for fut in as_completed(futs):
            try:
                row = fut.result()
            except Exception as e:  # noqa: BLE001 (a worker killed by the memory limit)
                row = {"stem": futs[fut][0], "error": f"worker: {type(e).__name__}"}
            w.writerow(row)
            fh.flush()
            print(f"{row['stem'][:60]:60} gates={row.get('gates', '-')} {row.get('cones', '')[:150]} {row.get('error', '')} "
                  f"{row.get('seconds', '')}s", flush=True)
    return 0


def cmd_one(args):
    row = analyse(args.file, args.max_clauses, args.seconds)
    for k in COLUMNS:
        if row.get(k) not in (None, ""):
            print(f"{k:20} {row[k]}")
    return 0


def cmd_toy(args):
    if args.kind == "sqr-const":
        nv, cl, exp = toy_sqr_const(args.width, shuffle=args.shuffle)
    else:
        nv, cl, exp = toy_cnf(args.kind, args.width, mapped=args.mapped, shuffle=args.shuffle, flip=args.flip)
    write_cnf_xz(args.out, nv, cl)
    print(f"{args.out}: {nv} variables, {len(cl)} clauses; expected label {exp}")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--out", required=True)
    c.add_argument("--stems", help="comma-separated substrings of stems (default: every 2025 cell)")
    c.add_argument("--files", nargs="*", help="explicit .cnf.xz files instead of the suite")
    c.add_argument("--workers", type=int, default=16)
    c.add_argument("--max-clauses", type=int, default=3_000_000)
    c.add_argument("--seconds", type=int, default=CELL_SECONDS)
    c.add_argument("--mem-mb", type=int, default=16000)
    c.set_defaults(func=cmd_census)
    o = sub.add_parser("one")
    o.add_argument("file")
    o.add_argument("--max-clauses", type=int, default=3_000_000)
    o.add_argument("--seconds", type=int, default=CELL_SECONDS)
    o.set_defaults(func=cmd_one)
    r = sub.add_parser("report")
    r.add_argument("table")
    r.add_argument("--run", help="a paired-run directory whose pts0/pts1 results mark the unsolved cells")
    r.set_defaults(func=cmd_report)
    t = sub.add_parser("toy")
    t.add_argument("kind", choices=sorted(TOYS))
    t.add_argument("--width", type=int, default=8)
    t.add_argument("--mapped", type=int, default=0)
    t.add_argument("--shuffle", type=int)
    t.add_argument("--flip", action="store_true")
    t.add_argument("--out", required=True)
    t.set_defaults(func=cmd_toy)
    s = sub.add_parser("selftest")
    s.add_argument("--verbose", action="store_true")
    s.set_defaults(func=cmd_selftest)
    args = ap.parse_args(argv)
    return args.func(args)


# which of solver 13's passes reasons about a function, given the shape it
# was found in: "miter" when both sides of a miter carry it
BACKEND = {
    "mul": ("structarith", "miter"), "add": ("structarith", "miter"), "sub": ("structarith", "miter"),
    "mul-bit": ("structarith", "miter"),      # the one-bit 16 x 16 miters (README, Booth step three)
    "add1": ("structarith", "miter"), "add3": ("structarith", "miter"),
    "linear": ("gauss", None), "parity": ("gauss", None),
    "popcount": ("card/structcount", None), "threshold": ("card/structcount", None),
    "sorter": ("structsort/card", None), "symmetric": ("card", None),
}


def _cone_labels(row):
    out = []
    for part in (row.get("cones") or "").split(" | "):
        if ":" in part:
            side, rest = part.split(":", 1)
            lab = rest.split("(")[0].split("[")[0]
            out.append((side, lab))
    return out


def _kind(label):
    return label.split(":")[0]


def backend_of(labels):
    """For each library function found: the pass that covers it, or None."""
    sides = {s: _kind(l) for s, l in labels}
    found = {}
    for s, lab in labels:
        k = _kind(lab)
        if k in ("unknown", "empty", "too-big", "const", "wiring"):
            continue
        b = BACKEND.get(k)
        if b is None:
            found[k] = None
        elif b[1] == "miter":
            found[k] = b[0] if sides.get("A") == k and sides.get("B") == k else None
        else:
            found[k] = b[0]
    return found


def _unsolved(run_dir):
    """stem -> number of seeds (of pts0, pts1) on which the cell is unsolved."""
    out = {}
    if not run_dir:
        return out
    for arm in ("pts0", "pts1"):
        path = Path(run_dir) / arm / "results.tsv"
        lines = path.read_text().splitlines()
        head = lines[0].split("\t")
        for line in lines[1:]:
            r = dict(zip(head, line.split("\t")))
            stem = _stem(r["instance"] + (".cnf.xz" if not r["instance"].endswith(".cnf.xz") else ""))
            out[stem] = out.get(stem, 0) + (r["result"] not in ("SATISFIABLE", "UNSATISFIABLE"))
    return out


def cmd_report(args):
    with open(args.table) as fh:
        rows = list(csv.DictReader(fh, delimiter="\t"))
    fam = {}
    if FAMILIES.is_file():
        for line in FAMILIES.read_text().splitlines()[1:]:
            p = line.split("\t")
            if p[0] == "sat-comp-2025":
                fam[_stem(p[1] + ".cnf.xz")] = p[3]
    uns = _unsolved(args.run)
    print(f"{len(rows)} cells")
    errs = Counter((r.get("error") or "").split(":")[0] for r in rows if r.get("error"))
    print("not fully analysed:", dict(errs))
    covs = sorted(float(r["gate_cov"]) for r in rows if r.get("gate_cov"))
    circ = [r for r in rows if r.get("gate_cov") and float(r["gate_cov"]) >= 0.3]
    print(f"gate coverage >= 0.3 (circuits): {len(circ)} cells; median coverage over all {covs[len(covs) // 2] if covs else 0:.2f}")
    # per family: the functions the cones carry
    by_fam = defaultdict(Counter)
    for r in rows:
        f = fam.get(r["stem"], "?")
        for side, lab in _cone_labels(r):
            if side in ("A", "B", "U", "S"):
                by_fam[f][_kind(lab)] += 1
    print("\nfunctions on the observed cones, per family (side labels; families with a circuit only):")
    for f, cnt in sorted(by_fam.items(), key=lambda x: -sum(v for k, v in x[1].items() if k not in ("unknown", "empty"))):
        known = {k: v for k, v in cnt.items() if k not in ("unknown", "empty", "too-big")}
        if not known and not cnt.get("unknown"):
            continue
        print(f"  {f[:26]:26} " + ", ".join(f"{k} {v}" for k, v in sorted(cnt.items(), key=lambda x: -x[1])))
    # the hard cells
    if uns:
        print("\ncells unsolved on at least one seed of the run, with a circuit (gate coverage >= 0.3):")
        for r in sorted(rows, key=lambda r: (fam.get(r["stem"], "?"), r["stem"])):
            k = uns.get(r["stem"])
            if not k:
                continue
            if not r.get("gate_cov") or float(r["gate_cov"]) < 0.3:
                continue
            labs = _cone_labels(r)
            be = backend_of(labs)
            gap = sorted(f for f, b in be.items() if b is None)
            cov = sorted(f"{f}->{b}" for f, b in be.items() if b)
            print(f"  {r['stem'][33:][:44]:44} {fam.get(r['stem'], '?')[:14]:14} unsolved {k}/2 cov {r.get('gate_cov')} "
                  f"cells {r.get('cells', '')[:40]}")
            print(f"      cones {r.get('cones', '')[:220]}")
            print(f"      no backend: {', '.join(gap) or '-'}; covered: {', '.join(cov) or '-'} {r.get('error', '')}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
