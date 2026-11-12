#!/usr/bin/env python3
"""relearn_census.py — how often the solver learns a lesson again in another frame
(bead SAT-playground-yv7.5, epic yv7).

Copying a learned clause into every frame of an unrolled formula (yv7.3) can only
save the work of lessons the solver would otherwise learn again in another frame.
This tool counts those lessons in the solver's own DRAT proof.

Every proof line is a clause. Each one is moved to frame 0 by the census shift d
(benchmarks/rl/periodic_2025.tsv): with frame(v) = (v - b) // d for the census
boundary b, the clause's lowest frame m is taken off every variable (v - m*d,
signs kept). Two clauses with the same moved form are the same lesson at
different frames, and m says which frame. A clause with a variable outside the
repeated part (a variable in no clause that has a copy at +d or -d, or a
variable the solver added) has no copy to speak of and is counted as not
movable.

Each learned clause, in proof order, is one of:

  new      the first time this lesson is learned
  shifted  learned before, but only at other frames: what copying could save
  again    the same clause at the same frame learned before (deleted, then
           learned again; copying does not change this)

The proof has to come from the scratch build that marks learned clauses (see
`build`): their binary proof lines start with 'l' (learned in search) or 'p'
(learned while probing) instead of 'a'. All other added lines ('a') come from
simplification, units fixed at level 0 and the like; they are counted the same
way, separately (for yv7.7).

What this does not say is whether a lesson is safe to copy. A copy is safe only
when the lesson was derived from repeating clauses alone. A lesson that leans
on facts from the start state (fixed at level 0 and carried into later frames)
cannot be copied, so `shifted` is an upper bound on what copying could save.
Telling the two apart needs the dependency tracking that is yv7.3's first part.

Columns: learned (search and probing), probe (of them, while probing), movable
(share of learned that is movable), new, shifted, again, shifted_share (of the
learned), families (distinct movable lessons), multi (lessons met at two or
more frames), dist_med and dist_q90 (frames from the first time, over the
shifted ones), gap_med (learned clauses between the first time and a shifted
one), short_shifted (shifted ones of at most 8 literals), q1..q5 (shifted
share in each fifth of the run), orig (learned lessons that are moved copies
of an input clause), other, other_movable, other_shifted, other_again (the
same for the non-learned added lines with at least one literal), deleted, conflicts and status (from the
solver's output).

Usage (numpy needed; the RL venv has it):
  python relearn_census.py build --out DIR            # the scratch tagged build
  python relearn_census.py run --bin DIR/target/release/sat-solver \\
      --out log/relearn-DATE --proofs SCRATCH --jobs 24 --time 1800
  python relearn_census.py analyze --runs log/relearn-DATE \\
      --tsv benchmarks/rl/relearn_2025.tsv --jobs 8
  python relearn_census.py one CNF PROOF [--shift D --boundary B]
  python relearn_census.py same PROOF_A PROOF_B       # equal up to the tags
  python relearn_census.py report --tsv benchmarks/rl/relearn_2025.tsv
  python relearn_census.py selftest
"""
import argparse
import csv
import os
import re
import resource
import shutil
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor, ThreadPoolExecutor, as_completed
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "tools"))
import periodic_census as pc  # noqa: E402  (read_text, parse, canonical, hash_clauses, member, mix)

K3 = np.uint64(0xD6E8FEB86659FD93)
READ = 1 << 25              # proof bytes decoded at a time
TAGS = {ord("a"): "a", ord("d"): "d", ord("l"): "l", ord("p"): "p"}
SHORT = 8                   # a short lesson: what a copy filter would keep

# The unrolled cells of 2025 that reach search (2026-11-03 grouping, bead yv7.5):
# 15 slow searches (valves-gates-1-k617 waits for the frame finder, yv7.6),
# the 6 fast ones as controls, the 3 oisc and-nested cells that time out.
TARGETS = [
    "c0bd86bd7ca2c65e44311de374168150-goldcrest-and-14",
    "fe851239cedd4197c5390cee55fee723-grs-256-64",
    "a49ec44c8d9972cda278d48b6dbef09e-grs-32-128",
    "b27017cae233481c679e3c3be6f00af8-SAT_dat.k100-24_1_rule_2",
    "dff4ea0af62900ab4d404f3bf7649a4d-nla-digbench-scaling_dijkstra-u_valuebound1_step",
    "04f43bb4a4f2c193a18e7195a4fd8e41-goldcrest-and-11",
    "fcc4c4522c54db2f1cee86fe0fff5f4a-x-epic_a19-p16_step",
    "113e692e379518bc3277855277dcc1b8-oisc-subrv-sll-nested-8",
    "d6afa5689d75e37111656db8980dd54b-grs-160-48",
    "ebbda8d90dfd9b6500bf932f952907a6-2018D_VexRiscv-regch0-20-p1_step",
    "0d8779c22e2caa8d676723701e5a0347-pj2016_k100",
    "a28f59752c160af22b6d04cb89ddf802-pj2008_k200",
    "ac3887576c2ce781202b9fe0fdc31420-pj2008_k80",
    "f32bb347996c351bc3e9a91c58e8601d-DLTM_twitter774_83_17",
    "5ceb95f5960efe1f23c3014847dea118-pj2009_k80",
    "56913ff910c49dd5010653720970d944-grs-32-64",
    "3ef7fa06c6380f12cfe2970873da6fb1-grs-64-48",
    "2e058841373f34cc47d8dec927fd1796-blocks-blocks-36-0.150-NOTKNOWN",
    "931621d9964e9d0cd1c4001822015c92-DLTM_twitter845_79_19",
    "07cea7a64248a60d509368d09d3057c2-pj2013_k9",
    "546f8e069aa450002a30c9c47538fe95-intel047_Iter78",
    "d5b11b0b73eb4ecebdbf6ee8143da689-oisc-subrv-and-nested-11",
    "5aed29ce52192a55ffbd2a6f340017e7-oisc-subrv-and-nested-12",
    "053e06424279f5cfec8bb14dfa9af23c-oisc-subrv-and-nested-15",
]

COLUMNS = ["stem", "shift", "boundary", "status", "time_s", "conflicts", "learned", "probe", "movable",
           "new", "shifted", "again", "shifted_share", "families", "multi", "dist_med", "dist_q90",
           "gap_med", "short_shifted", "q1", "q2", "q3", "q4", "q5", "orig",
           "other", "other_movable", "other_shifted", "other_again", "deleted", "cut", "analyze_s", "error"]


# ---------------------------------------------------------------- the tagged build

PATCHES = [
    ("src/proof.rs", "use crate::file::File;\n",
     "use crate::file::File;\n\n/// SCRATCH (yv7.5 measurement build, never committed): set by learn.rs around a\n"
     "/// learned clause; its binary proof line starts with 'l' (search) or 'p'\n"
     "/// (probing) instead of 'a'.\npub static TAG_LEARNED: std::sync::atomic::AtomicU8 = "
     "std::sync::atomic::AtomicU8::new(0);\n"),
    ("src/proof.rs", "    if proof.binary {\n        write_char(proof, b'a');\n    }\n    print_proof_line(solver, proof);",
     "    if proof.binary {\n        let t = TAG_LEARNED.load(std::sync::atomic::Ordering::Relaxed);\n"
     "        write_char(proof, if t == 0 { b'a' } else { t });\n    }\n    print_proof_line(solver, proof);"),
]
LEARN_SITES = [("src/learn.rs", "    crate::assign::learned_unit(solver, not_uip);\n"),
               ("src/learn.rs", "    let ref_ = crate::clause::new_redundant_clause(solver, 1);\n"),
               ("src/learn.rs", "    let ref_ = crate::clause::new_redundant_clause(solver, glue);\n"),
               # a conflict at level one: analyze_failed_literal learns its units directly
               ("src/analyze.rs", "    for &lit in &units {\n        crate::assign::learned_unit(solver, lit);\n    }\n")]
TAG_ON = ("    crate::proof::TAG_LEARNED.store(if solver.probing { b'p' } else { b'l' }, "
          "std::sync::atomic::Ordering::Relaxed);\n")
TAG_OFF = "    crate::proof::TAG_LEARNED.store(0, std::sync::atomic::Ordering::Relaxed);\n"


def cmd_build(args):
    """Copies solver/13-kissat-rs (sources only) to --out, marks the places where
    conflict analysis adds a learned clause (learn.rs: unit, binary, long;
    analyze.rs: the units of a conflict at level one), and builds it the way
    build.sh does (generic x86-64). Every patch site must match exactly once, so
    a changed solver fails here instead of mis-tagging. Every other caller of
    learned_unit is a simplification pass or an engine, which 'a' describes."""
    src = REPO / "solver/13-kissat-rs"
    out = Path(args.out).resolve()
    if (out / "src").exists():
        print(f"{out}/src exists; give a new --out", file=sys.stderr)
        return 1
    out.mkdir(parents=True, exist_ok=True)
    for name in ("build.sh", "Cargo.toml", "Cargo.lock"):
        shutil.copy2(src / name, out / name)
    shutil.copytree(src / "src", out / "src")
    for rel, old, new in PATCHES:
        p = out / rel
        s = p.read_text()
        if s.count(old) != 1:
            print(f"patch site not found exactly once in {rel}: {old[:50]!r}", file=sys.stderr)
            return 1
        p.write_text(s.replace(old, new))
    for rel, old in LEARN_SITES:
        p = out / rel
        s = p.read_text()
        if s.count(old) != 1:
            print(f"{rel} site not found exactly once: {old.strip()!r}", file=sys.stderr)
            return 1
        p.write_text(s.replace(old, TAG_ON + old + TAG_OFF))
    env = dict(os.environ, RUSTFLAGS="", CARGO_TARGET_DIR=str(out / "target"))
    r = subprocess.run(["cargo", "build", "--release"], cwd=out, env=env)
    if r.returncode == 0:
        print(out / "target/release/sat-solver")
    return r.returncode


# ---------------------------------------------------------------- reading a binary DRAT proof

def decode(b):
    """One chunk of binary DRAT that ends with a line's 0 byte: (tags, lens, lits).
    A literal is written as 2*|lit| + (lit < 0) in 7-bit groups, low first, the
    high bit set on all but the last byte; a 0 byte ends a line. A 0 byte never
    occurs inside a literal (its last byte is nonzero and the others have the
    high bit set), so the 0 bytes alone give the lines."""
    zero = np.flatnonzero(b == 0)
    n = len(zero)
    start = np.empty(n, np.int64)
    start[0] = 0
    start[1:] = zero[:-1] + 1
    tags = b[start]
    if not np.isin(tags, np.array(list(TAGS), np.uint8)).all():
        raise ValueError("not a binary DRAT proof (a line starts with an unknown byte)")
    nonempty = zero > start + 1
    if (b[zero[nonempty] - 1] & 0x80).any():
        raise ValueError("a line ends inside a literal")
    mask = np.ones(len(b), bool)
    mask[start] = False
    mask[zero] = False
    pos = np.flatnonzero(mask)
    lb = b[pos]
    end = (lb & 0x80) == 0
    vend = np.flatnonzero(end)
    vid = np.zeros(len(lb), np.int64)
    vid[vend[:-1] + 1] = 1
    vid = np.cumsum(vid)
    vstart = np.concatenate(([0], vend[:-1] + 1))
    k = np.arange(len(lb), dtype=np.int64) - vstart[vid]
    if len(k) and k.max() > 4:
        raise ValueError("a literal longer than five bytes")
    terms = (lb & 0x7F).astype(np.float64) * np.exp2(7.0 * k)   # exact: below 2^35
    val = np.bincount(vid, weights=terms, minlength=len(vend)).astype(np.int64)
    lits = np.where(val & 1, -(val >> 1), val >> 1)
    lens = np.bincount(np.searchsorted(zero, pos[vend]), minlength=n).astype(np.int64)
    return tags, lens, lits


def proof_chunks(path):
    """Yields decode() of the proof a chunk at a time, then the number of bytes at
    the end that do not form a whole line (a run stopped while writing)."""
    rest = b""
    with open(path, "rb") as fh:
        while True:
            data = fh.read(READ)
            buf = rest + data
            cut = buf.rfind(b"\x00") + 1
            rest = buf[cut:]
            if cut:
                yield decode(np.frombuffer(buf[:cut], np.uint8))
            if not data:
                break
    yield len(rest)


def encode(lines):
    """(tag, [lits]) lines as binary DRAT bytes (the self-test's writer)."""
    out = bytearray()
    for tag, lits in lines:
        out.append(ord(tag))
        for lit in lits:
            x = 2 * abs(lit) + (lit < 0)
            while x & ~0x7F:
                out.append((x & 0x7F) | 0x80)
                x >>= 7
            out.append(x)
        out.append(0)
    return bytes(out)


# ---------------------------------------------------------------- the formula: repeated part

def read_all(path):
    """The whole decompressed text, read in pieces (read(n) would allocate n bytes
    first, which an address-space limit refuses)."""
    if str(path).endswith(".xz"):
        p = subprocess.Popen(["xz", "-dc", str(path)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        fh = p.stdout
    else:
        p, fh = None, open(path, "rb")
    parts = []
    while True:
        b = fh.read(1 << 26)
        if not b:
            break
        parts.append(b)
    fh.close()
    if p is not None:
        err = p.stderr.read()
        p.stderr.close()
        if p.wait() != 0:
            raise RuntimeError(f"xz failed ({p.returncode}): {err.decode(errors='replace').strip()[:120]}")
    return b"".join(parts)


def repeated_part(cnf, d):
    """The input's clauses (variables sorted within each clause: var, neg, starts,
    lens), pvar, maxvar and rep for shift d: rep[c] is True when clause c has a
    copy at +d or -d, pvar[v] when v is in such a clause."""
    data = read_all(cnf)
    hv, hc, lits, starts, lens = pc.parse(data)
    del data
    nvar = int(max(hv, int(np.abs(lits).max()))) + 1
    maxvar = nvar - 1
    s, starts, lens = pc.canonical(lits, starts, lens)
    del lits
    var = np.abs(s)
    neg = s < 0
    del s
    h, _ = pc.hash_clauses(var, neg, starts, lens)
    _, first = np.unique(h, return_index=True)
    keep = np.zeros(len(lens), bool)
    keep[first] = True
    hs = np.sort(h[keep])
    del h, first
    hf, vf = pc.hash_clauses(var, neg, starts, lens, shift=d, maxvar=maxvar)
    rep = vf & pc.member(hs, hf)
    del hf, vf
    hb, vb = pc.hash_clauses(var, neg, starts, lens, shift=-d, maxvar=maxvar)
    rep |= vb & pc.member(hs, hb)
    del hb, vb, hs
    pvar = np.zeros(nvar, bool)
    for a, b in pc.chunks(starts, lens):
        lo, hi = int(starts[a]), int(starts[b - 1] + lens[b - 1])
        cid = np.repeat(np.arange(b - a, dtype=np.int64), lens[a:b])
        pvar[var[lo:hi][rep[a:b][cid]]] = True
    return var, neg, starts, lens, pvar, maxvar, rep


def lessons(var, neg, lens, pvar, maxvar, d, bnd):
    """Per clause (literals in clause order, lens > 0): (hash of the moved form,
    lowest frame m, frames spanned, movable). The hash adds one mixed value per
    literal, so the order of the literals does not matter."""
    var = var.astype(np.int64)
    n = len(lens)
    if n == 0:   # a chunk of deletions only
        return np.zeros(0, np.uint64), np.zeros(0, np.int64), np.zeros(0, np.int64), np.zeros(0, bool)
    st = np.concatenate(([0], np.cumsum(lens)[:-1])).astype(np.int64)
    cid = np.repeat(np.arange(n, dtype=np.int64), lens)
    inside = (var >= 1) & (var <= maxvar)
    ok = inside & pvar[np.where(inside, var, 0)]
    frame = (var - bnd) // d
    m = np.minimum.reduceat(frame, st)
    span = np.maximum.reduceat(frame, st) - m
    movable = np.logical_and.reduceat(ok, st)
    moved = var - m[cid] * d
    code = (np.maximum(moved, 0).astype(np.uint64) << np.uint64(1)) | neg.astype(np.uint64)
    x = pc.mix(code)
    h = pc.mix(np.add.reduceat(x, st) ^ (lens.astype(np.uint64) * K3))
    return h, m.astype(np.int64), span.astype(np.int64), movable


def classify(h, m):
    """new / shifted / again for clauses in order, and for each the index of the
    first clause of its lesson."""
    n = len(h)
    if n == 0:
        z = np.zeros(0, bool)
        return z, z, z, np.zeros(0, np.int64)
    _, first_h, inv = np.unique(h, return_index=True, return_inverse=True)
    pair = pc.mix(h ^ (m.astype(np.uint64) * K3))
    _, first_p = np.unique(pair, return_index=True)
    is_h = np.zeros(n, bool)
    is_h[first_h] = True
    is_p = np.zeros(n, bool)
    is_p[first_p] = True
    return is_h, is_p & ~is_h, ~is_p, first_h[inv]


def proof_table(proof, pvar, maxvar, d, bnd):
    """Per added line with at least one literal, in proof order: tag, len, h, m,
    span, mov (see lessons); and the deleted and cut counts."""
    parts = {k: [] for k in ("tag", "len", "h", "m", "span", "mov")}
    deleted = 0
    cut = 0
    for item in proof_chunks(proof):
        if isinstance(item, int):
            cut = item
            break
        tags, plen, lits = item
        isdel = tags == ord("d")
        deleted += int(isdel.sum())
        # drop deletions and empty lines, keep the literals of the rest
        keep = ~isdel & (plen > 0)
        lits = lits[np.repeat(keep, plen)]
        plen = plen[keep]
        tg = tags[keep]
        h, m, span, mov = lessons(np.abs(lits), lits < 0, plen, pvar, maxvar, d, bnd)
        for k, a in (("tag", tg), ("len", plen.astype(np.int32)), ("h", h), ("m", m),
                     ("span", span.astype(np.int32)), ("mov", mov)):
            parts[k].append(a)
    empty = {"tag": np.uint8, "len": np.int32, "h": np.uint64, "m": np.int64, "span": np.int32, "mov": bool}
    cat = {k: (np.concatenate(v) if v else np.zeros(0, empty[k])) for k, v in parts.items()}
    return cat, deleted, cut


def lines_at(proof, wanted):
    """{position: literals} for the added lines with at least one literal at the
    given positions (positions as in proof_table)."""
    wanted = np.unique(np.asarray(wanted, np.int64))
    out = {}
    base = 0
    for item in proof_chunks(proof):
        if isinstance(item, int):
            break
        tags, plen, lits = item
        keep = (tags != ord("d")) & (plen > 0)
        st = np.concatenate(([0], np.cumsum(plen)[:-1]))
        kept = np.flatnonzero(keep)
        lo, hi = np.searchsorted(wanted, [base, base + len(kept)])
        for w in wanted[lo:hi]:
            i = kept[w - base]
            out[int(w)] = [int(x) for x in lits[st[i]:st[i] + plen[i]]]
        base += len(kept)
    return out


def analyze(cnf, proof, d, bnd):
    """The row of counts for one proof (see the module docstring)."""
    t0 = time.time()
    var, neg, starts, lens, pvar, maxvar, _ = repeated_part(cnf, d)
    # input clauses as lessons, for `orig`
    oparts = []
    for a, b in pc.chunks(starts, lens):
        lo, hi = int(starts[a]), int(starts[b - 1] + lens[b - 1])
        oh, _, _, omov = lessons(var[lo:hi], neg[lo:hi], lens[a:b], pvar, maxvar, d, bnd)
        oparts.append(oh[omov])
    orig = np.unique(np.concatenate(oparts)) if oparts else np.zeros(0, np.uint64)
    del var, neg, starts, lens, oparts
    cat, deleted, cut = proof_table(proof, pvar, maxvar, d, bnd)
    tag = cat["tag"].astype(np.uint8)
    row = {c: "" for c in COLUMNS}
    learned = (tag == ord("l")) | (tag == ord("p"))
    nl = int(learned.sum())
    row.update(learned=nl, probe=int((tag == ord("p")).sum()), deleted=deleted, cut=cut)
    L = np.flatnonzero(learned & cat["mov"])
    row["movable"] = round(len(L) / nl, 4) if nl else 0.0
    lidx = np.cumsum(learned) - 1
    new, sh, ag, first = classify(cat["h"][L], cat["m"][L])
    row.update(new=int(new.sum()), shifted=int(sh.sum()), again=int(ag.sum()),
               shifted_share=round(sh.sum() / nl, 4) if nl else 0.0)
    if len(L):
        hl = cat["h"][L]
        row["families"] = int(len(np.unique(hl)))
        pairs = np.unique(np.stack([hl.view(np.int64), cat["m"][L]], 1), axis=0)
        _, per = np.unique(pairs[:, 0], return_counts=True)
        row["multi"] = int((per >= 2).sum())
        dist = np.abs(cat["m"][L] - cat["m"][L][first])[sh]
        gap = (lidx[L] - lidx[L][first])[sh]
        if len(dist):
            row.update(dist_med=int(np.median(dist)), dist_q90=int(np.quantile(dist, 0.9)),
                       gap_med=int(np.median(gap)))
        row["short_shifted"] = int((sh & (cat["len"][L] <= SHORT)).sum())
        fifth = np.minimum(lidx[L] * 5 // max(1, nl), 4)
        for q in range(5):
            inq = fifth == q
            tot = int((learned & (np.minimum(lidx * 5 // max(1, nl), 4) == q)).sum())
            row[f"q{q + 1}"] = round(int((sh & inq).sum()) / tot, 4) if tot else ""
        row["orig"] = int(pc.member(orig, hl).sum()) if len(orig) else 0
    other = tag == ord("a")
    O = np.flatnonzero(other & cat["mov"])
    on, osh, oag, _ = classify(cat["h"][O], cat["m"][O])
    row.update(other=int(other.sum()), other_movable=len(O), other_shifted=int(osh.sum()),
               other_again=int(oag.sum()))
    row["analyze_s"] = round(time.time() - t0, 1)
    return row


# ---------------------------------------------------------------- is a lesson a fact of the step relation?

CLEAN_COLUMNS = ["stem", "kind", "sampled", "w1", "w4", "all", "start", "goal", "both", "unknown", "check_s",
                 "error"]
DETAIL_COLUMNS = ["stem", "kind", "pos", "frame", "span", "size", "family_shifted", "result", "w1", "w4", "all",
                  "past", "future"]
KINDS = ("multi", "single", "shifted")
WINDOWS = (1, 4)


def clause_text(var, neg, starts, lens, idx):
    """DIMACS lines of the clauses idx, in that order, and the byte length of each."""
    ln = lens[idx]
    total = int(ln.sum())
    pos = np.repeat(starts[idx] - np.concatenate(([0], np.cumsum(ln)[:-1])), ln) + np.arange(total)
    v = var[pos].astype(np.int64)
    g = neg[pos]
    lits = np.where(g, -v, v)
    width = np.floor(np.log10(v)).astype(np.int64) + 1 + g + 1          # digits, sign, space
    st = np.concatenate(([0], np.cumsum(ln)[:-1])).astype(np.int64)
    nbytes = np.add.reduceat(width, st) + 2 if total else np.zeros(0, np.int64)   # "0\n"
    out = np.zeros(total + len(idx), np.int64)
    mask = np.ones(len(out), bool)
    mask[np.cumsum(ln + 1) - 1] = False
    out[mask] = lits
    text = (" ".join(out.astype(str).tolist()) + "\n").replace(" 0 ", " 0\n").encode() if len(idx) else b""
    return text, nbytes


class Layout:
    """The input's clauses written once, in three files, so that every clause set a
    check needs is a few byte ranges: the repeated clauses sorted by (lowest frame,
    highest frame), the others sorted by highest frame (a prefix is 'everything
    at or before frame f') and again by lowest frame, descending ('everything at
    or after f')."""

    def __init__(self, var, neg, starts, lens, rep, fmin, fmax, base):
        self.files = {}
        orders = {
            "rep": np.flatnonzero(rep)[np.lexsort((fmax[rep], fmin[rep]))],
            "past": np.flatnonzero(~rep)[np.argsort(fmax[~rep], kind="stable")],
            "future": np.flatnonzero(~rep)[np.argsort(-fmin[~rep], kind="stable")],
        }
        for name, idx in orders.items():
            path = Path(f"{base}.{name}.cnf")
            nb = np.zeros(0, np.int64)
            with open(path, "wb") as fh:
                for a in range(0, len(idx), 1_000_000):
                    text, n = clause_text(var, neg, starts, lens, idx[a:a + 1_000_000])
                    fh.write(text)
                    nb = np.concatenate((nb, n))
            off = np.concatenate(([0], np.cumsum(nb)))
            if off[-1] != path.stat().st_size:
                raise RuntimeError(f"layout {name}: {off[-1]} bytes counted, {path.stat().st_size} written")
            self.files[name] = (path, off, fmin[idx], fmax[idx])

    def window(self, lo, hi):
        """Repeated clauses with every variable in frames lo..hi."""
        path, off, a, b = self.files["rep"]
        out = []
        for f in range(lo, hi + 1):
            i = np.searchsorted(a, f, "left")
            j = np.searchsorted(a, f, "right")
            k = i + np.searchsorted(b[i:j], hi, "right")
            if k > i:
                out.append((path, int(off[i]), int(off[k] - off[i]), int(k - i)))
        return out

    def all(self):
        path, off, _, _ = self.files["rep"]
        return [(path, 0, int(off[-1]), len(off) - 1)]

    def past(self, hi):
        path, off, _, b = self.files["past"]
        k = int(np.searchsorted(b, hi, "right"))
        return self.all() + ([(path, 0, int(off[k]), k)] if k else [])

    def future(self, lo):
        path, off, a, _ = self.files["future"]
        k = int(np.searchsorted(-a, -lo, "right"))
        return self.all() + ([(path, 0, int(off[k]), k)] if k else [])


def solve(binary, maxvar, lesson, ranges, limit):
    """Feeds the header, the lesson's negation (one unit a literal) and the byte
    ranges to the solver on its standard input: 'implied' (UNSAT), 'not' (SAT)
    or 'unknown'."""
    n = len(lesson) + sum(r[3] for r in ranges)
    head = (f"p cnf {maxvar} {n}\n" + "".join(f"{-x} 0\n" for x in lesson)).encode()
    cmd = f"ulimit -v {16000 * 1024}; exec timeout -s TERM {limit} {binary} -q"
    p = subprocess.Popen(["bash", "-c", cmd], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL)
    try:
        p.stdin.write(head)
        for path, a, nbytes, _ in ranges:
            with open(path, "rb") as fh:
                fh.seek(a)
                left = nbytes
                while left > 0:
                    chunk = fh.read(min(left, 1 << 24))
                    if not chunk:
                        raise RuntimeError(f"{path} shorter than its index")
                    p.stdin.write(chunk)
                    left -= len(chunk)
        p.stdin.close()
    except BrokenPipeError:
        pass          # the solver stopped early (the time limit)
    rc = p.wait()
    return {20: "implied", 10: "not"}.get(rc, "unknown")


def clean_one(stem, run_dir, binary, n_sample, limit, limit_all, jobs, tmp, seed=1, kinds=KINDS):
    r = census_rows()[stem]
    o = Path(run_dir) / stem
    proof = re.search(r"(?m)^proof (.+)$", (o / "run.txt").read_text()).group(1)
    return clean_core(stem, cnf_path(stem), proof, int(r["shift"]), int(r["boundary"] or 0), binary, n_sample,
                      limit, limit_all, jobs, tmp, seed, kinds)


def clean_core(stem, cnf, proof, d, bnd, binary, n_sample, limit, limit_all, jobs, tmp, seed=1, kinds=KINDS):
    """Three samples: lessons learned at two or more frames ('multi', the first
    time each was learned), lessons learned at one frame only ('single'), and the
    shifted re-learns themselves ('shifted': drawn uniformly over occurrences,
    each tested at its own frame). For each, asks
    the solver whether each is implied by the repeated clauses in a window of
    frames around it (one, then four frames either side), then by the whole
    repeated part. Implied: a fact of the step relation, safe to copy where the
    window's clauses have copies. A lesson the whole repeated part does not imply
    leans on clauses that are not copies, and cannot be copied without an
    induction proof; it is checked again with the other clauses on one side
    added: those at or before its frames ('start' when that implies it), those
    at or after ('goal'), and 'both' when neither side alone does.

    The 'shifted' sample bounds what copying could save: a copy made from a
    derivation over repeating clauses is implied by the repeating clauses at the
    frame it lands on, so a re-learn the repeated part does not imply there could
    not have been a copy. The 'multi' sample tests where each lesson was first
    learned, which says nothing about its later frames. Every picked lesson
    carries its lesson's number of shifted re-learns (family_shifted), for
    weighting by occurrence. Only the samples in kinds are checked; the others
    are still drawn (with the same random draws, so a sample does not depend on
    which kinds run) and listed with no result."""
    t0 = time.time()
    var, neg, starts, lens, pvar, maxvar, rep_ = repeated_part(cnf, d)
    value = (np.arange(maxvar + 1, dtype=np.int64) - bnd) // d
    every = np.ones(maxvar + 1, bool)
    fmin = pc.per_clause(var, starts, lens, value, every, "min")
    fmax = pc.per_clause(var, starts, lens, value, every, "max")
    cat, _, _ = proof_table(proof, pvar, maxvar, d, bnd)
    tag = cat["tag"].astype(np.uint8)
    learned = (tag == ord("l")) | (tag == ord("p"))
    L = np.flatnonzero(learned & cat["mov"])
    rows = []
    done = []
    if len(L):
        hl, ml = cat["h"][L], cat["m"][L]
        _, first_h, inv = np.unique(hl, return_index=True, return_inverse=True)
        pairs = np.unique(np.stack([hl.view(np.int64), ml], 1), axis=0)
        uh, per = np.unique(pairs[:, 0].view(np.uint64), return_counts=True)
        multi_h = uh[per >= 2]
        is_multi = pc.member(multi_h, hl[first_h]) if len(multi_h) else np.zeros(len(first_h), bool)
        _, sh, _, _ = classify(hl, ml)
        fam_sh = np.bincount(inv, weights=sh.astype(np.float64), minlength=len(first_h)).astype(np.int64)
        rng = np.random.default_rng(seed)
        picks = []
        for kind, mask in (("multi", is_multi), ("single", ~is_multi)):
            cand = first_h[mask]
            if len(cand) > n_sample:
                cand = rng.choice(cand, n_sample, replace=False)
            picks += [(kind, int(L[i]), int(ml[i]), int(cat["span"][L[i]]), int(fam_sh[inv[i]])) for i in sorted(cand)]
        cand = np.flatnonzero(sh)
        if len(cand) > n_sample:
            cand = np.random.default_rng(seed + 1).choice(cand, n_sample, replace=False)
        picks += [("shifted", int(L[i]), int(ml[i]), int(cat["span"][L[i]]), int(fam_sh[inv[i]])) for i in sorted(cand)]
        del cat
        lits = lines_at(proof, [p[1] for p in picks if p[0] in kinds])
        # the clause files only when something is checked
        lay = Layout(var, neg, starts, lens, rep_, fmin, fmax, Path(tmp) / stem) if lits else None
        del var, neg, starts, lens

        def check(j):
            res = {}
            lo, hi = j["frame"], j["frame"] + j["span"]
            for w in WINDOWS:
                res[f"w{w}"] = solve(binary, maxvar, j["lesson"], lay.window(lo - w, hi + w), limit)
                if res[f"w{w}"] == "implied":
                    break
            if "implied" not in res.values():
                res["all"] = solve(binary, maxvar, j["lesson"], lay.all(), limit_all)
            if res.get("all") == "not":
                res["past"] = solve(binary, maxvar, j["lesson"], lay.past(hi), limit_all)
                res["future"] = solve(binary, maxvar, j["lesson"], lay.future(lo), limit_all)
            if res.get("w1") == "implied":
                out = "w1"
            elif res.get("w4") == "implied":
                out = "w4"
            elif res.get("all") == "implied":
                out = "all"
            elif res.get("past") == "implied":
                out = "start"
            elif res.get("future") == "implied":
                out = "goal"
            elif res.get("past") == "not" and res.get("future") == "not":
                out = "both"
            else:
                out = "unknown"
            j.update(res, result=out)
            return j

        jobs_list = [dict(stem=stem, kind=k, pos=p, frame=m, span=sp, size=len(lits[p]), family_shifted=f,
                          lesson=lits[p]) for k, p, m, sp, f in picks if k in kinds]
        with ThreadPoolExecutor(jobs) as ex:
            done = list(ex.map(check, jobs_list))
        done += [dict(stem=stem, kind=k, pos=p, frame=m, span=sp, size="", family_shifted=f, result="")
                 for k, p, m, sp, f in picks if k not in kinds]
        for f in (lay.files.values() if lay else []):
            f[0].unlink()
    for kind in kinds:
        js = [j for j in done if j["kind"] == kind]
        row = {c: "" for c in CLEAN_COLUMNS}
        row.update(stem=stem, kind=kind, sampled=len(js), check_s=round(time.time() - t0, 1))
        for k in ("w1", "w4", "all", "start", "goal", "both", "unknown"):
            row[k] = sum(1 for j in js if j["result"] == k)
        rows.append(row)
    return rows, sorted(done, key=lambda j: (j["kind"], j["pos"]))


def _clean_job(stem, args):
    resource.setrlimit(resource.RLIMIT_AS, (args.mem_mb << 20, args.mem_mb << 20))
    try:
        return clean_one(stem, args.runs, Path(args.bin).resolve(), args.sample, args.limit, args.limit_all,
                         args.jobs, args.tmp, kinds=tuple(k for k in args.kinds.split(",") if k))
    except Exception as e:  # noqa: BLE001  (one bad cell must not stop the table)
        return [{**{c: "" for c in CLEAN_COLUMNS}, "stem": stem, "error": f"{type(e).__name__}: {str(e)[:120]}"}], []


def cmd_clean(args):
    """Cells in parallel (--cells), each with --jobs solver checks at a time: keep
    cells x jobs within the 32-process cap."""
    stems = args.only.split(",") if args.only else [s for s in TARGETS if (Path(args.runs) / s / "run.txt").exists()]
    Path(args.tmp).mkdir(parents=True, exist_ok=True)
    rows, details = [], []
    with ProcessPoolExecutor(args.cells) as ex:
        futs = {ex.submit(_clean_job, s, args): s for s in stems}
        for f in as_completed(futs):
            r, dt = f.result()
            rows += r
            details += dt
            for x in r:
                print(f"{x['stem'][-40:]:40s} {x['kind']:6s} n {x['sampled']} w1 {x['w1']} w4 {x['w4']} "
                      f"all {x['all']} start {x['start']} goal {x['goal']} both {x['both']} "
                      f"unknown {x['unknown']} {x['check_s']}s {x['error']}", flush=True)
    order = {s: i for i, s in enumerate(TARGETS)}
    rows.sort(key=lambda r: (order.get(r["stem"], 999), r["kind"]))
    with open(args.tsv, "w", newline="") as fh:
        w = csv.DictWriter(fh, CLEAN_COLUMNS, delimiter="\t")
        w.writeheader()
        w.writerows(rows)
    if args.detail:
        details.sort(key=lambda j: (order.get(j["stem"], 999), j["kind"], j["pos"]))
        with open(args.detail, "w", newline="") as fh:
            w = csv.DictWriter(fh, DETAIL_COLUMNS, delimiter="\t", extrasaction="ignore")
            w.writeheader()
            w.writerows(details)
    print(f"wrote {args.tsv}")
    return 0


# ---------------------------------------------------------------- runs

def census_rows():
    return {r["stem"]: r for r in csv.DictReader(open(REPO / "benchmarks/rl/periodic_2025.tsv"), delimiter="\t")}


def cnf_path(stem):
    p = REPO / "benchmarks/sat-comp-2025" / f"{stem}.cnf.xz"
    if not p.exists():
        p = REPO / "benchmarks/sat-comp-2025" / f"{stem}.cnf"
    return p


def run_one(binary, stem, out, proofs, core, limit, mem_mb, seed):
    o = Path(out) / stem
    o.mkdir(parents=True, exist_ok=True)
    proof = Path(proofs) / f"{stem}.drat"
    cmd = (f"ulimit -v {mem_mb * 1024}; exec timeout -s TERM {limit} taskset -c {core} "
           f"{binary} --seed={seed} {cnf_path(stem)} {proof}")
    t0 = time.time()
    with open(o / "stdout.txt", "w") as so, open(o / "stderr.txt", "w") as se:
        r = subprocess.run(["bash", "-c", cmd], stdout=so, stderr=se)
    (o / "run.txt").write_text(f"rc {r.returncode}\nwall {time.time() - t0:.1f}\nproof {proof}\n"
                               f"core {core}\nseed {seed}\nbinary {binary}\n")
    return stem, r.returncode, time.time() - t0


def cmd_run(args):
    binary = Path(args.bin).resolve()
    if not os.access(binary, os.X_OK):
        print(f"not executable: {binary}", file=sys.stderr)
        return 1
    stems = args.only.split(",") if args.only else TARGETS
    Path(args.out).mkdir(parents=True, exist_ok=True)
    Path(args.proofs).mkdir(parents=True, exist_ok=True)
    cores = list(range(args.first_core, args.first_core + args.jobs))
    with ThreadPoolExecutor(args.jobs) as ex:
        free = list(cores)
        futs = {}
        pending = list(stems)
        while pending or futs:
            while pending and free:
                s = pending.pop(0)
                c = free.pop(0)
                futs[ex.submit(run_one, binary, s, args.out, args.proofs, c, args.time, args.mem_mb, args.seed)] = c
            done = next(as_completed(futs))
            c = futs.pop(done)
            free.append(c)
            stem, rc, wall = done.result()
            print(f"{time.strftime('%H:%M:%S')} rc {rc:4d} {wall:7.1f}s {stem}", flush=True)
    return 0


def solver_output(o):
    """(status, conflicts, time) from a run directory."""
    txt = (o / "stdout.txt").read_text(errors="replace") if (o / "stdout.txt").exists() else ""
    run = (o / "run.txt").read_text() if (o / "run.txt").exists() else ""
    st = re.findall(r"(?m)^s (\S+)", txt)
    cf = re.findall(r"(?m)^c conflicts:\s+(\d+)", txt)
    wall = re.search(r"(?m)^wall (\S+)", run)
    rc = re.search(r"(?m)^rc (-?\d+)", run)
    status = st[-1] if st else ("TIMEOUT" if rc and rc.group(1) == "124" else "NONE")
    return status, (int(cf[-1]) if cf else ""), (float(wall.group(1)) if wall else "")


def _job(stem, run_dir, mem_mb):
    resource.setrlimit(resource.RLIMIT_AS, (mem_mb << 20, mem_mb << 20))
    r = census_rows()[stem]
    d, bnd = int(r["shift"]), int(r["boundary"] or 0)
    o = Path(run_dir) / stem
    proof = re.search(r"(?m)^proof (.+)$", (o / "run.txt").read_text()).group(1)
    try:
        row = analyze(cnf_path(stem), proof, d, bnd)
    except Exception as e:  # noqa: BLE001  (one bad cell must not stop the table)
        row = {c: "" for c in COLUMNS}
        row["error"] = f"{type(e).__name__}: {str(e)[:120]}"
    row.update(stem=stem, shift=d, boundary=bnd)
    row["status"], row["conflicts"], row["time_s"] = solver_output(o)
    return row


def cmd_analyze(args):
    stems = args.only.split(",") if args.only else [s for s in TARGETS if (Path(args.runs) / s / "run.txt").exists()]
    rows = []
    with ProcessPoolExecutor(args.jobs) as ex:
        futs = {ex.submit(_job, s, args.runs, args.mem_mb): s for s in stems}
        for f in as_completed(futs):
            r = f.result()
            rows.append(r)
            print(f"{r['stem'][-45:]:45s} learned {r['learned']} shifted {r['shifted']} "
                  f"({r['shifted_share']}) again {r['again']} {r['error']}", flush=True)
    rows.sort(key=lambda r: TARGETS.index(r["stem"]) if r["stem"] in TARGETS else 999)
    with open(args.tsv, "w", newline="") as fh:
        w = csv.DictWriter(fh, COLUMNS, delimiter="\t")
        w.writeheader()
        w.writerows(rows)
    print(f"wrote {args.tsv}")
    return 0


def cmd_one(args):
    r = census_rows().get(pc.stem_of(args.cnf), {})
    d = args.shift or int(r.get("shift") or 0)
    bnd = args.boundary if args.boundary is not None else int(r.get("boundary") or 0)
    if d <= 0:
        print("no shift: give --shift", file=sys.stderr)
        return 1
    row = analyze(args.cnf, args.proof, d, bnd)
    for c in COLUMNS:
        if row.get(c, "") != "":
            print(f"{c:15s} {row[c]}")
    return 0


def normalized_digest(path):
    """sha256 of the proof bytes with every line's 'l' or 'p' tag read as 'a', and
    the number of tagged lines."""
    import hashlib
    hsh = hashlib.sha256()
    tagged = 0
    rest = b""
    with open(path, "rb") as fh:
        while True:
            data = fh.read(READ)
            buf = rest + data
            cut = buf.rfind(b"\x00") + 1
            rest = buf[cut:]
            if cut:
                b = np.frombuffer(buf[:cut], np.uint8).copy()
                zero = np.flatnonzero(b == 0)
                start = np.concatenate(([0], zero[:-1] + 1))
                t = b[start]
                lp = (t == ord("l")) | (t == ord("p"))
                tagged += int(lp.sum())
                b[start[lp]] = ord("a")
                hsh.update(b.tobytes())
            if not data:
                hsh.update(rest)
                break
    return hsh.hexdigest(), tagged


def cmd_same(args):
    """Two proofs equal byte for byte once the 'l' and 'p' tags read as 'a'."""
    (da, ta), (db, tb) = normalized_digest(args.a), normalized_digest(args.b)
    n = os.path.getsize(args.a), os.path.getsize(args.b)
    print(f"{'same' if da == db else 'DIFFERENT'} ({n[0]} and {n[1]} bytes; tagged lines {ta} and {tb})")
    return 0 if da == db else 1


def cmd_report(args):
    rows = list(csv.DictReader(open(args.tsv), delimiter="\t"))
    print(f"{'cell':34s} {'status':6s} {'learned':>9s} {'movable':>7s} {'shifted':>8s} {'share':>6s} "
          f"{'again':>8s} {'short':>7s} {'dist':>5s} {'gap':>8s} {'other':>9s} {'o.shift':>8s}")
    for r in rows:
        name = re.sub(r"^[0-9a-f]+-", "", r["stem"])[:34]
        print(f"{name:34s} {r['status'][:6]:6s} {r['learned']:>9s} {r['movable']:>7s} {r['shifted']:>8s} "
              f"{r['shifted_share']:>6s} {r['again']:>8s} {r['short_shifted']:>7s} {r['dist_med']:>5s} "
              f"{r['gap_med']:>8s} {r['other']:>9s} {r['other_shifted']:>8s} {r['error']}")
    return 0


# ---------------------------------------------------------------- self-test

def cmd_selftest(args):
    import tempfile
    fails = []

    def check(name, ok, detail=""):
        print(f"  {'ok  ' if ok else 'FAIL'} {name} {detail}")
        if not ok:
            fails.append(name)

    rng = np.random.default_rng(7)
    # 1. the decoder reads back what the writer wrote, multi-byte literals included
    lines = []
    for _ in range(3000):
        t = "adlp"[int(rng.integers(4))]
        k = int(rng.integers(0, 12))
        lines.append((t, [int(x) * (1 if rng.random() < 0.5 else -1)
                          for x in rng.integers(1, 1 << int(rng.integers(1, 31)), k)]))
    blob = encode(lines)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        (tmp / "p.drat").write_bytes(blob + b"a\x84")   # a cut last line
        got = []
        cut = None
        global READ
        saved, READ = READ, 4096                         # many chunks
        for item in proof_chunks(tmp / "p.drat"):
            if isinstance(item, int):
                cut = item
                break
            tags, lens, lits = item
            o = 0
            for t, n in zip(tags, lens):
                got.append((chr(t), [int(x) for x in lits[o:o + n]]))
                o += n
        READ = saved
        check("decode", got == lines and cut == 2, f"{len(got)} lines, cut {cut}")

        # 2. a toy unrolling: frame f has variables 1 + 4f .. 4 + 4f (d = 4, six frames);
        # the transition a_f -> a_{f+1}, (b_f or c_f), a start unit and a goal clause
        d = 4
        cl = []
        for f in range(6):
            a, b_, c, e = 1 + d * f, 2 + d * f, 3 + d * f, 4 + d * f
            cl.append([b_, c])
            cl.append([-b_, e])
            if f < 5:
                cl.append([-a, a + d])
        cl.append([1])                 # start
        cl.append([-(1 + d * 5), 30])  # goal, with a variable outside the frames
        cl.append([-30])
        (tmp / "t.cnf").write_text(f"p cnf 30 {len(cl)}\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in cl))
        C = [-2, 7, 11]                       # a lesson at frames 0-2
        proof = [("l", C),                    # new
                 ("a", [3, 4]),               # other, new
                 ("l", [x + d if x > 0 else x - d for x in C]),     # shifted (frame 1)
                 ("l", [x + d if x > 0 else x - d for x in C]),     # again
                 ("d", C),
                 ("p", [6, -8]),              # new, while probing
                 ("l", [5, 30]),              # not movable: 30 is outside
                 ("l", [x + 2 * d if x > 0 else x - 2 * d for x in C]),  # shifted (frame 2)
                 ("a", [7, 8]),               # other, shifted copy of [3, 4]
                 ("l", [10, -12]),            # shifted copy of [6, -8] (probing lesson)
                 ("l", [6, 7]),               # a moved copy of the input clause (b, c) at frame 1
                 ("a", [])]                   # the empty clause
        (tmp / "t.drat").write_bytes(encode(proof))
        saved, READ = READ, 8     # tiny chunks: some hold deletions only
        r = analyze(tmp / "t.cnf", tmp / "t.drat", d, 1)
        READ = saved
        want = dict(learned=8, probe=1, new=3, shifted=3, again=1, other=2, other_shifted=1, deleted=1,
                    multi=2, orig=1, families=3)
        got = {k: r[k] for k in want}
        check("toy counts", got == want, f"{got}")
        check("toy movable", r["movable"] == round(7 / 8, 4), f"{r['movable']}")
        check("toy distances", r["dist_med"] == 1 and r["short_shifted"] == 3, f"dist {r['dist_med']}")
        # 3. same() ignores only the tags
        a = [("a" if t in "lp" else t, x) for t, x in proof]
        (tmp / "u.drat").write_bytes(encode(a))
        ns = argparse.Namespace(a=str(tmp / "t.drat"), b=str(tmp / "u.drat"))
        check("same up to tags", cmd_same(ns) == 0)
        a[2] = ("a", [5, 6])
        (tmp / "v.drat").write_bytes(encode(a))
        ns = argparse.Namespace(a=str(tmp / "t.drat"), b=str(tmp / "v.drat"))
        check("different clause seen", cmd_same(ns) == 1)
        # 4. the window check, with the solver as the oracle, on the toy (UNSAT: a_0, a_f -> a_f+1, not a_5)
        binary = REPO / "solver/13-kissat-rs/target/release/sat-solver"
        if not os.access(binary, os.X_OK):
            check("window check (no solver binary built)", False)
        else:
            lessons_ = [("l", [-1, 9]),     # a_0 -> a_2: the step relation alone, frames 0-2
                        ("l", [-5, 13]),    # the same lesson one frame on
                        ("l", [13]),        # a_3: needs the start
                        ("l", [-17]),       # not a_4: needs the goal
                        ("l", [11])]        # c_2: needs the start and the goal together
            (tmp / "c.drat").write_bytes(encode(lessons_))
            rows, det = clean_core("toy", tmp / "t.cnf", tmp / "c.drat", d, 1, binary, 10, 10, 10, 2, tmp)
            got = {(j["kind"], tuple(j["lesson"])): j["result"] for j in det}
            want = {("multi", (-1, 9)): "w1", ("single", (13,)): "start", ("single", (-17,)): "goal",
                    ("single", (11,)): "both", ("shifted", (-5, 13)): "w1"}
            fam = {(j["kind"], tuple(j["lesson"])): j["family_shifted"] for j in det}
            check("window check", got == want and rows[0]["sampled"] == 1 and fam[("multi", (-1, 9))] == 1,
                  f"{got}")
            # --kinds: two one-frame lessons (a_1 needs the start; a_1 -> a_2 is the step relation);
            # only the 'single' sample is checked, the other samples are listed without a result
            (tmp / "s.drat").write_bytes(encode([("l", [5]), ("l", [-5, 9])]))
            rows2, det2 = clean_core("toy", tmp / "t.cnf", tmp / "s.drat", d, 1, binary, 10, 10, 10, 1, tmp,
                                     kinds=("single",))
            check("only the chosen kinds are checked", all(j["result"] == "" for j in det2 if j["kind"] != "single")
                  and [r["kind"] for r in rows2] == ["single"], f"{[(j['kind'], j['result']) for j in det2]}")
        # 5. the tagged build marks every learned clause, the units of a conflict at
        # level one included (the review's toy: two conflicts, two units)
        if args.tagged:
            (tmp / "f.cnf").write_text("p cnf 4 4\n1 2 0\n-1 2 0\n3 4 0\n-3 4 0\n")
            stock_path = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0",
                          "--structsort=0", "--structdiv=0", "--structdd=0", "--gauss=0", "--card=0", "--structpoints=0", "--freewalk=0"]
            subprocess.run([args.tagged, "--lucky=0", "--preprocess=0", "--phase=0", "-q", *stock_path,
                            str(tmp / "f.cnf"), str(tmp / "f.drat")], stdout=subprocess.DEVNULL)
            got = []
            for item in proof_chunks(tmp / "f.drat"):
                if isinstance(item, int):
                    break
                tags, lens, lits = item
                o = 0
                for t, n in zip(tags, lens):
                    got.append((chr(t), [int(x) for x in lits[o:o + n]]))
                    o += n
            units = [x for x in got if len(x[1]) == 1]
            check("tagged build: units of level-one conflicts", len(units) == 2 and all(t == "l" for t, _ in units),
                  f"{got}")
    print(f"selftest: {'all passed' if not fails else 'FAILED: ' + ', '.join(fails)}")
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("build")
    a.add_argument("--out", required=True)
    a = sub.add_parser("run")
    a.add_argument("--bin", required=True)
    a.add_argument("--out", required=True)
    a.add_argument("--proofs", required=True)
    a.add_argument("--jobs", type=int, default=24)
    a.add_argument("--first-core", type=int, default=0)
    a.add_argument("--time", type=int, default=1800)
    a.add_argument("--mem-mb", type=int, default=16000)
    a.add_argument("--seed", type=int, default=0)
    a.add_argument("--only", default="")
    a = sub.add_parser("analyze")
    a.add_argument("--runs", required=True)
    a.add_argument("--tsv", default=str(REPO / "benchmarks/rl/relearn_2025.tsv"))
    a.add_argument("--jobs", type=int, default=8)
    a.add_argument("--mem-mb", type=int, default=16000)
    a.add_argument("--only", default="")
    a = sub.add_parser("clean")
    a.add_argument("--runs", required=True)
    a.add_argument("--bin", required=True, help="a stock build (the oracle)")
    a.add_argument("--tsv", default=str(REPO / "benchmarks/rl/relearn_clean_2025.tsv"))
    a.add_argument("--detail", default="")
    a.add_argument("--tmp", required=True)
    a.add_argument("--sample", type=int, default=40)
    a.add_argument("--kinds", default=",".join(KINDS),
                   help="samples to check: multi, single, shifted; empty lists the samples only")
    a.add_argument("--limit", type=int, default=10)
    a.add_argument("--limit-all", type=int, default=60)
    a.add_argument("--jobs", type=int, default=10, help="solver checks at a time, per cell")
    a.add_argument("--cells", type=int, default=3, help="cells at a time")
    a.add_argument("--mem-mb", type=int, default=16000)
    a.add_argument("--only", default="")
    a = sub.add_parser("one")
    a.add_argument("cnf")
    a.add_argument("proof")
    a.add_argument("--shift", type=int, default=0)
    a.add_argument("--boundary", type=int, default=None)
    a = sub.add_parser("same")
    a.add_argument("a")
    a.add_argument("b")
    a = sub.add_parser("report")
    a.add_argument("--tsv", default=str(REPO / "benchmarks/rl/relearn_2025.tsv"))
    a = sub.add_parser("selftest")
    a.add_argument("--tagged", default="", help="a tagged build to check as well")
    args = ap.parse_args()
    return {"build": cmd_build, "run": cmd_run, "analyze": cmd_analyze, "clean": cmd_clean, "one": cmd_one,
            "same": cmd_same, "report": cmd_report, "selftest": cmd_selftest}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
