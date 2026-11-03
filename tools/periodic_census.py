#!/usr/bin/env python3
"""periodic_census.py — periodic structure on the 400 cells (bead SAT-playground-yv7.1).

Epic SAT-playground-yv7 (G.2, almost-symmetry). Bounded model checking,
planning steps, program unrollings and k-induction write one frame many
times, shifted, plus a few clauses for the start and the goal. A learned
clause derived only from the repeated clauses holds in every frame, shifted
(Strichman's replication, 2001). This census says which formulas have that
shape, how much of each formula is the repeated part, and what is left over.

Two detectors.

  shift   The variable offset d that maps the most clauses onto clauses of
          the formula (C + d in F, every variable moved by d, signs kept).
          Candidates are the common distances between clauses of the same
          relative shape; each candidate is then scored exactly, by hashing
          every shifted clause.
  order   For files written frame by frame whose numbering is not one
          offset: the clause period P in file order (clause i and clause
          i + P have the same length, signs and occurrence counts), the
          renaming sigma read off literal by literal from those pairs, and
          the share of clauses sigma moves onto other clauses (a variable
          whose commonest image is itself gets no image: the identity maps
          every clause onto itself). Sigma's commonest offset joins the shift
          candidates. A shuffled file defeats both detectors.

The best shift is the candidate with the most clauses that have a copy at +d
(a shift between two halves of a formula has a copy for every clause in one
direction or the other, but only two frames). `copies` is how many times a
repeated clause appears along its shifts (C, C + d, C + 2d, ...), counted
exactly for every repeated clause (median and q90): what replication would get
from one learned clause.

When d >= 16 the numbering is frame by frame: a frame is a block of d
variables, the block boundary is the offset that cuts the fewest repeated
clauses, a block counts as a frame when at least half of the clauses starting
in it repeat, and runs of frames are segments. Every clause is then one of:

  within, crossing, reach2   repeated (a copy at +d or -d), spanning one
                             frame, two adjacent frames, or more
  start, goal                not repeated, all at or before the second frame,
                             or all at or after the second-to-last frame
  long                       not repeated, spanning three frames or more
  middle                     not repeated, the rest (frames that differ)
  outside                    not repeated, no variable inside the frames
                             (before the first or after the last)

When d < 16 the shift is a bit slice, a row of a grid or one step of a signal
numbered step by step; frames are not blocks of variables, so `frames` is
`copies` and the clause classes are left blank.

Columns: fwd, repeat and both are the shares with a copy at +d, at +d or -d,
and at both; shift2 and repeat2 the runner-up; shift_b and repeat_b the best
shift among the clauses the first one leaves (another segment), and
repeat_any the two together; periodic_vars the share of variables in a
repeated clause; cperiod, cmatch, sigma_* the order detector.

Empty clauses are dropped (`parsed` counts the rest: the two xits miters hold
292 and 1,096). Shares are of the distinct clauses (duplicates counted once).
A damaged .xz is an error, not a shorter formula. Files over
--max-mb of decompressed text are read up to that size (`prefix` = 1): the
shift and the frames come from the prefix, and the goal clauses at the end
of such a file are not seen.

Usage (numpy needed; the RL venv has it):
  python periodic_census.py one CNF [--max-mb N]
  python periodic_census.py census --suite benchmarks/sat-comp-2025 \\
      --out benchmarks/rl/periodic_2025.tsv --jobs 32 --mem-mb 16000
  python periodic_census.py report --tsv benchmarks/rl/periodic_2025.tsv \\
      --runs log/abtest-struct-card-400-2026-11-01-22-15-59
  python periodic_census.py selftest
"""
import argparse
import csv
import statistics
import os
import re
import resource
import subprocess
import sys
import time
from collections import defaultdict
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parent.parent
K1 = np.uint64(0x9E3779B97F4A7C15)
K2 = np.uint64(0xC2B2AE3D27D4EB4F)
CHUNK = 8_000_000          # literals hashed at a time
CANDIDATES = 10            # shift candidates scored exactly
TIME_MAJOR = 16            # d at or above this: frames are blocks of d variables
PERIODIC = 0.5             # a cell is periodic when this share of its clauses repeat ...
MIN_FRAMES = 3             # ... over at least this many frames

COLUMNS = ["stem", "family", "vars", "clauses", "parsed", "distinct", "prefix",
           "shift", "fwd", "repeat", "both", "shift2", "repeat2", "shift_b", "repeat_b", "repeat_any",
           "copies", "copies_q90", "frames", "frames_max", "segments", "frame_vars", "boundary", "periodic_vars",
           "within", "crossing", "reach2", "start", "goal", "long", "middle", "outside", "rem_units",
           "cperiod", "cmatch", "sigma_cover", "sigma_consistent", "sigma_shift", "sigma_shift_share",
           "read_s", "detect_s", "error"]


def stem_of(path):
    """The file name without its .cnf.xz or .cnf suffix (names can hold '.cnf' too)."""
    name = Path(path).name
    for suf in (".cnf.xz", ".cnf"):
        if name.endswith(suf):
            return name[: -len(suf)]
    return name


def mix(x):
    """splitmix64's finaliser on a uint64 array (wraps mod 2^64)."""
    x = x + K1
    x = (x ^ (x >> np.uint64(30))) * np.uint64(0xBF58476D1CE4E5B9)
    x = (x ^ (x >> np.uint64(27))) * np.uint64(0x94D049BB133111EB)
    return x ^ (x >> np.uint64(31))


# ---------------------------------------------------------------- reading

def read_text(path, max_bytes):
    """The decompressed text, cut at the last newline if longer than max_bytes."""
    if str(path).endswith(".xz"):
        p = subprocess.Popen(["xz", "-dc", str(path)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        data = p.stdout.read(max_bytes + 1)
        more = len(data) > max_bytes
        p.stdout.close()
        if more:
            p.kill()  # stopped on purpose at the cap: its status says nothing
            p.wait()
        else:
            err = p.stderr.read()
            if p.wait() != 0:
                # a damaged file must not pass for a complete formula
                raise RuntimeError(f"xz failed ({p.returncode}): {err.decode(errors='replace').strip()[:120]}")
        p.stderr.close()
    else:
        with open(path, "rb") as fh:
            data = fh.read(max_bytes + 1)
        more = len(data) > max_bytes
    if more:
        data = data[:data.rfind(b"\n", 0, max_bytes) + 1]
    return data, more


def parse(data):
    """(header vars, header clauses, literals in file order, clause starts, lengths)."""
    m = re.search(rb"(?m)^p\s+cnf\s+(\d+)\s+(\d+)\s*$", data)
    hv, hc = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    body = data[m.end():] if m else data
    if re.search(rb"(?m)^\s*[cp%]", body):
        body = re.sub(rb"(?m)^\s*[cp%].*$", b"", body)
    ints = np.fromstring(body, dtype=np.int64, sep=" ") if body.strip() else np.zeros(0, np.int64)
    zero = np.flatnonzero(ints == 0)
    if len(zero) == 0:
        return hv, hc, np.zeros(0, np.int32), np.zeros(0, np.int64), np.zeros(0, np.int64)
    ints = ints[:zero[-1] + 1]
    lens = np.diff(np.concatenate(([-1], zero))) - 1
    lits = ints[ints != 0].astype(np.int32)
    keep = lens > 0
    lens = lens[keep]
    starts = np.concatenate(([0], np.cumsum(lens)[:-1])).astype(np.int64)
    return hv, hc, lits, starts, lens.astype(np.int64)


def canonical(lits, starts, lens):
    """Literals sorted by variable within each clause (the negative one first when
    both signs occur), repeats removed."""
    cid = np.repeat(np.arange(len(lens), dtype=np.int64), lens)
    order = np.lexsort((lits, np.abs(lits), cid))
    s = lits[order]
    dup = np.zeros(len(s), bool)
    dup[1:] = (s[1:] == s[:-1]) & (cid[1:] == cid[:-1])
    if dup.any():
        s = s[~dup]
        cid = cid[~dup]
        lens = np.bincount(cid, minlength=len(lens)).astype(np.int64)
        starts = np.concatenate(([0], np.cumsum(lens)[:-1])).astype(np.int64)
    return s, starts, lens


# ---------------------------------------------------------------- hashing

def chunks(starts, lens):
    """Clause ranges [a, b) whose literals add up to about CHUNK."""
    n = len(lens)
    if n == 0:
        return
    ends = starts + lens
    a = 0
    while a < n:
        b = int(np.searchsorted(ends, starts[a] + CHUNK, side="right"))
        b = max(b, a + 1)
        yield a, min(b, n)
        a = b


def hash_clauses(var, neg, starts, lens, shift=0, maxvar=None, recode=None, resort=False, rebase=False):
    """One 64-bit hash per clause of (position, value, sign) over its literals in
    the order given. The value is the variable plus `shift`; or recode[variable]
    (0 = no image), re-sorted within each clause when `resort`; or, with
    `rebase`, the offset from the clause's first variable. Returns (hashes,
    valid): a clause is not valid when a value leaves 1..maxvar or has no image."""
    n = len(lens)
    out = np.zeros(n, np.uint64)
    valid = np.ones(n, bool)
    for a, b in chunks(starts, lens):
        lo, hi = int(starts[a]), int(starts[b - 1] + lens[b - 1])
        v = var[lo:hi].astype(np.int64)
        g = neg[lo:hi]
        ln = lens[a:b]
        st = starts[a:b] - lo
        cid = np.repeat(np.arange(b - a, dtype=np.int64), ln)
        if recode is not None:
            v = recode[v]
            bad = v == 0
            if resort:
                order = np.lexsort((~g, v, cid))
                v, g, bad = v[order], g[order], bad[order]
        elif rebase:
            v = v - v[st][cid] + 1
            bad = np.zeros(len(v), bool)
        else:
            v = v + shift
            bad = (v < 1) | (v > maxvar) if maxvar is not None else np.zeros(len(v), bool)
        pos = (np.arange(hi - lo, dtype=np.int64) - st[cid]).astype(np.uint64)
        code = (np.maximum(v, 0).astype(np.uint64) << np.uint64(1)) | g.astype(np.uint64)
        x = mix(code ^ (pos * K2))
        out[a:b] = mix(np.add.reduceat(x, st) ^ (ln.astype(np.uint64) * K1))
        if bad.any():
            valid[a:b] = np.logical_and.reduceat(~bad, st)
    return out, valid


def per_clause(var, starts, lens, value, mask, how):
    """Per clause, the min or max of value[v] over its variables v with mask[v]
    (None when a clause has none: +/- a large sentinel), chunked."""
    n = len(lens)
    big = np.iinfo(np.int64).max // 4
    out = np.zeros(n, np.int64)
    for a, b in chunks(starts, lens):
        lo, hi = int(starts[a]), int(starts[b - 1] + lens[b - 1])
        v = var[lo:hi]
        st = starts[a:b] - lo
        m = mask[v]
        if how == "min":
            out[a:b] = np.minimum.reduceat(np.where(m, value[v], big), st)
        elif how == "max":
            out[a:b] = np.maximum.reduceat(np.where(m, value[v], -big), st)
        else:  # "any"
            out[a:b] = np.logical_or.reduceat(m, st)
    return out


def member(sorted_set, q):
    i = np.searchsorted(sorted_set, q)
    i = np.minimum(i, len(sorted_set) - 1)
    return sorted_set[i] == q


# ---------------------------------------------------------------- the census of one formula

def shift_candidates(var, neg, starts, lens, keep):
    """The commonest distances between the smallest variables of clauses with the
    same relative shape (positions, sign and offset from the smallest variable)."""
    base = var[starts].astype(np.int64)
    shape, _ = hash_clauses(var, neg, starts, lens, rebase=True)
    shape, base = shape[keep], base[keep]
    o = np.lexsort((base, shape))
    same = shape[o][1:] == shape[o][:-1]
    d = (base[o][1:] - base[o][:-1])[same]
    d = d[d > 0]
    if len(d) == 0:
        return []
    vals, cnt = np.unique(d, return_counts=True)
    top = np.argsort(-cnt, kind="stable")[:CANDIDATES]
    return [int(x) for x in vals[top]]


def frames_time_major(var, starts, lens, keep, rep, nvar, d):
    """Blocks of d variables, the boundary offset that cuts the fewest repeated
    clauses. A block is a frame when at least half of the clauses starting in it
    repeat and it holds at least a quarter of the median repeated count; runs of
    frames are segments. Returns (frame of each variable, first and last frame,
    frames, segments, boundary)."""
    vmin = var[starts].astype(np.int64)
    vmax = var[starts + lens - 1].astype(np.int64)
    span = (vmax - vmin)[rep]
    lo = vmin[rep]
    short = span < d
    first = (lo[short] + 1) % d
    end = first + span[short]
    diff = np.bincount(first, minlength=d + 1).astype(np.int64)
    inside = end <= d
    diff -= np.bincount(end[inside], minlength=d + 1)
    wrap = ~inside
    diff[d] -= int(wrap.sum())
    diff[0] += int(wrap.sum())
    diff -= np.bincount(end[wrap] - d, minlength=d + 1)
    cuts = np.cumsum(diff)[:d]
    b = int(np.argmin(cuts))
    frame = (np.arange(nvar, dtype=np.int64) - b) // d
    blk = (vmin - b) // d
    bmin = int(blk[keep].min())
    allc = np.bincount(blk[keep] - bmin)
    repc = np.bincount(blk[rep] - bmin, minlength=len(allc))
    nz = repc[repc > 0]
    if len(nz) == 0:
        return frame, 0, -1, 0, 0, b
    per = np.flatnonzero((2 * repc >= allc) & (repc >= 0.25 * np.median(nz)))
    if len(per) == 0:
        return frame, 0, -1, 0, 0, b
    segments = 1 + int(np.count_nonzero(np.diff(per) > 1))
    return frame, bmin + int(per[0]), bmin + int(per[-1]), int(len(per)), segments, b


def chain_steps(nxt):
    """Steps from each node to the end of its chain, following nxt (-1 = end),
    by pointer doubling: exact, with no cap on the length."""
    rank = (nxt >= 0).astype(np.int64)
    ptr = nxt.copy()
    while True:
        on = np.flatnonzero(ptr >= 0)
        if len(on) == 0:
            return rank
        p = ptr[on]
        rank[on] += rank[p]
        ptr[on] = ptr[p]


def clause_copies(var, neg, starts, lens, h, keep, rep, d, maxvar):
    """For every repeated distinct clause, its number of copies along the shift
    (C - k d, ..., C, ..., C + m d all in the formula), exactly. Returns (median, q90)."""
    idx = np.flatnonzero(keep)
    hk = h[idx]
    order = np.argsort(hk, kind="stable")
    hsorted = hk[order]

    def image(shift):
        q, v = hash_clauses(var, neg, starts, lens, shift=shift, maxvar=maxvar)
        q, v = q[idx], v[idx]
        i = np.minimum(np.searchsorted(hsorted, q), len(hsorted) - 1)
        return np.where(v & (hsorted[i] == q), order[i], -1)

    after = chain_steps(image(d))
    before = chain_steps(image(-d))
    copies = (after + before + 1)[rep[idx]]
    if len(copies) == 0:
        return 0, 0
    return int(np.median(copies)), int(np.quantile(copies, 0.9))


def census_one(path, max_bytes=1_500_000_000, family=""):
    t0 = time.time()
    row = {c: "" for c in COLUMNS}
    row["stem"] = stem_of(path)
    row["family"] = family
    data, more = read_text(path, max_bytes)
    hv, hc, lits, starts, lens = parse(data)
    del data
    row.update(vars=hv, clauses=hc, parsed=len(lens), prefix=int(more))
    t1 = time.time()
    row["read_s"] = round(t1 - t0, 1)
    if len(lens) == 0:
        row["error"] = "empty"
        return row
    nvar = int(max(hv, int(np.abs(lits).max()))) + 1
    maxvar = nvar - 1
    # the order detector reads the literals as written, before they are sorted
    sigma, order_stats = order_detector(lits, starts, lens, nvar)
    s, starts, lens = canonical(lits, starts, lens)
    del lits
    var = np.abs(s)
    neg = s < 0
    del s
    h, _ = hash_clauses(var, neg, starts, lens)
    _, first = np.unique(h, return_index=True)
    keep = np.zeros(len(lens), bool)
    keep[first] = True
    hs = np.sort(h[keep])
    del first
    n = int(keep.sum())
    row["distinct"] = n
    if sigma is not None:
        hm, vm = hash_clauses(var, neg, starts, lens, recode=sigma, resort=True)
        order_stats["sigma_cover"] = round(float((vm & member(hs, hm) & keep & (hm != h)).sum() / n), 4)
        del hm, vm, sigma
    row.update(order_stats)

    # shift candidates scored by forward copies, the best two then both ways; the
    # renaming's commonest offset joins them (a frame shift can be missing from the
    # shape distances when the same shape repeats inside a frame, as in DLTM)
    cands = shift_candidates(var, neg, starts, lens, keep)
    ss = abs(int(order_stats.get("sigma_shift") or 0))
    if ss > 0 and float(order_stats.get("sigma_shift_share") or 0) >= 0.5 and ss not in cands:
        cands.append(ss)
    scored = []
    for d in cands:
        hf, vf = hash_clauses(var, neg, starts, lens, shift=d, maxvar=maxvar)
        scored.append((int((vf & member(hs, hf) & keep).sum()), d))
    scored.sort(key=lambda x: (-x[0], x[1]))
    best = []
    for _, d in scored[:2]:
        hf, vf = hash_clauses(var, neg, starts, lens, shift=d, maxvar=maxvar)
        fwd = vf & member(hs, hf) & keep
        hb, vb = hash_clauses(var, neg, starts, lens, shift=-d, maxvar=maxvar)
        bwd = vb & member(hs, hb) & keep
        best.append((int(fwd.sum()), int((fwd | bwd).sum()), d, fwd, bwd))
    best.sort(key=lambda x: (-x[0], x[2]))
    if not best or best[0][1] == 0:
        row.update(shift=0, fwd=0.0, repeat=0.0, both=0.0)
        row["detect_s"] = round(time.time() - t1, 1)
        return row
    _, _, d, fwd, bwd = best[0]
    rep = fwd | bwd
    row.update(shift=d, fwd=round(fwd.sum() / n, 4), repeat=round(rep.sum() / n, 4),
               both=round((fwd & bwd).sum() / n, 4))
    if len(best) > 1:
        row.update(shift2=best[1][2], repeat2=round(best[1][1] / n, 4))
    del best
    # a second shift among the clauses the first one leaves (another segment)
    rest = keep & ~rep
    row.update(shift_b=0, repeat_b=0.0, repeat_any=row["repeat"])
    if rest.sum() >= 0.05 * n:
        sc = []
        for d2 in shift_candidates(var, neg, starts, lens, rest):
            if d2 != d:
                hf, vf = hash_clauses(var, neg, starts, lens, shift=d2, maxvar=maxvar)
                sc.append((int((vf & member(hs, hf) & rest).sum()), d2))
        sc.sort(key=lambda x: (-x[0], x[1]))
        if sc and sc[0][0] > 0:
            d2 = sc[0][1]
            hf, vf = hash_clauses(var, neg, starts, lens, shift=d2, maxvar=maxvar)
            hb, vb = hash_clauses(var, neg, starts, lens, shift=-d2, maxvar=maxvar)
            rb = rest & ((vf & member(hs, hf)) | (vb & member(hs, hb)))
            row.update(shift_b=d2, repeat_b=round(rb.sum() / n, 4), repeat_any=round((rep | rb).sum() / n, 4))
            del hf, vf, hb, vb, rb
    del rest

    pvar = np.zeros(nvar, bool)
    for a, b in chunks(starts, lens):
        lo, hi = int(starts[a]), int(starts[b - 1] + lens[b - 1])
        cid = np.repeat(np.arange(b - a, dtype=np.int64), lens[a:b])
        pvar[var[lo:hi][rep[a:b][cid]]] = True
    row["periodic_vars"] = round(pvar[1:].sum() / max(1, maxvar), 4)
    cmed, cq90 = clause_copies(var, neg, starts, lens, h, keep, rep, d, maxvar)
    del h
    row.update(copies=cmed, copies_q90=cq90)
    if d < TIME_MAJOR:
        row.update(frames=cmed)
        row["detect_s"] = round(time.time() - t1, 1)
        return row
    frame, f0, f1, nfr, seg, b = frames_time_major(var, starts, lens, keep, rep, nvar, d)
    t_of = frame - f0
    left_of = f1 - frame
    del frame
    row.update(frames=nfr, frames_max=max(0, f1 - f0 + 1), segments=seg, frame_vars=d, boundary=b)
    if nfr < MIN_FRAMES:
        # no frames to speak of: the frame-based classes would mean nothing
        row["detect_s"] = round(time.time() - t1, 1)
        return row
    # repeated clauses: their span over all their variables (every variable has a
    # block); the rest: by the variables placed inside the detected frames, so a
    # variable that sits only in clauses without a copy still counts where it is
    every = np.ones(nvar, bool)
    k = keep
    span = per_clause(var, starts, lens, t_of, every, "max") - per_clause(var, starts, lens, t_of, every, "min")
    row["within"] = int((k & rep & (span == 0)).sum())
    row["crossing"] = int((k & rep & (span == 1)).sum())
    row["reach2"] = int((k & rep & (span >= 2)).sum())
    infr = (t_of >= 0) & (left_of >= 0)
    tmin = per_clause(var, starts, lens, t_of, infr, "min")
    tmax = per_clause(var, starts, lens, t_of, infr, "max")
    lmax = per_clause(var, starts, lens, left_of, infr, "max")
    anyp = per_clause(var, starts, lens, t_of, infr, "any").astype(bool)
    span = tmax - tmin
    rem = k & ~rep
    out = rem & ~anyp
    st_ = rem & anyp & (tmax <= 1)
    go = rem & anyp & ~st_ & (lmax <= 1)
    lg = rem & anyp & ~st_ & ~go & (span >= 2)
    mid = rem & anyp & ~st_ & ~go & ~lg
    row.update(start=int(st_.sum()), goal=int(go.sum()), long=int(lg.sum()), middle=int(mid.sum()),
               outside=int(out.sum()), rem_units=int((rem & (lens == 1)).sum()))
    row["detect_s"] = round(time.time() - t1, 1)
    return row


PAIRS = 20_000_000          # literal pairs read for the renaming, at most


def order_detector(lits, starts, lens, nvar):
    """The clause period in file order and the renaming it implies. Returns
    (sigma or None, stats); the caller scores sigma on the distinct clauses."""
    res = dict(cperiod=0, cmatch=0.0, sigma_cover=0.0, sigma_consistent=0.0, sigma_shift=0, sigma_shift_share=0.0)
    n = len(lens)
    if n < 64:
        return None, res
    var = np.abs(lits)
    neg = lits < 0
    occ = np.minimum(np.bincount(var, minlength=nvar), 255).astype(np.int64) + 1
    # shape as written: positions, signs and occurrence counts (no variable names)
    shape, _ = hash_clauses(var, neg, starts, lens, recode=occ)
    del occ
    idx = np.arange(n, dtype=np.int64)
    o = np.lexsort((idx, shape))
    so = shape[o]
    same = so[1:] == so[:-1]
    d = (o[1:] - o[:-1])[same]
    del o, so, same
    d = d[d > 0]
    if len(d) == 0:
        return None, res
    vals, cnt = np.unique(d, return_counts=True)
    del d
    cands = [int(x) for x in vals[np.argsort(-cnt, kind="stable")[:CANDIDATES]] if x < n]
    if not cands:
        return None, res
    best = max(cands, key=lambda p: (int(np.count_nonzero(shape[:-p] == shape[p:])), -p))
    match = shape[:-best] == shape[best:]
    res.update(cperiod=best, cmatch=round(float(match.mean()), 4))
    # literal pairs of matched clauses, position by position (an even sample when many)
    i = np.flatnonzero(match)
    del match, shape
    if len(i) == 0:
        return None, res
    tot = int(lens[i].sum())
    if tot > PAIRS:
        i = i[:: int(np.ceil(tot / PAIRS))]
    ln = lens[i]
    rep_i = np.repeat(i, ln)
    off = np.arange(int(ln.sum()), dtype=np.int64) - np.repeat(np.cumsum(ln) - ln, ln)
    v = var[starts[rep_i] + off].astype(np.int64)
    w = var[starts[rep_i + best] + off].astype(np.int64)
    del rep_i, off
    key = v * nvar + w
    del v, w
    uk, uc = np.unique(key, return_counts=True)
    del key
    uv, uw = uk // nvar, uk % nvar
    o = np.lexsort((-uc, uv))
    firsts = np.ones(len(o), bool)
    firsts[1:] = uv[o][1:] != uv[o][:-1]
    group = np.cumsum(firsts) - 1
    gtot = np.bincount(group, weights=uc[o])
    sel = o[firsts]
    # a variable whose commonest image is itself has no image: the identity maps
    # every clause onto itself and says nothing about frames
    moving = uw[sel] != uv[sel]
    if not moving.any():
        return None, res
    res["sigma_consistent"] = round(float(uc[sel][moving].sum() / gtot[moving].sum()), 4)
    sel = sel[moving]
    sigma = np.zeros(nvar, np.int64)
    sigma[uv[sel]] = uw[sel]
    diff = uw[sel] - uv[sel]
    if len(diff):
        dv, dc = np.unique(diff, return_counts=True)
        j = int(np.argmax(dc))
        res.update(sigma_shift=int(dv[j]), sigma_shift_share=round(float(dc[j] / len(diff)), 4))
    return sigma, res


# ---------------------------------------------------------------- the suite

def families():
    fam = {}
    p = REPO / "benchmarks/rl/families.tsv"
    if p.is_file():
        with open(p) as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                if r["suite"] == "sat-comp-2025":
                    fam[r["stem"]] = r["family"]
    return fam


def _worker(path, max_bytes, family, mem_mb):
    if mem_mb:
        lim = mem_mb << 20
        resource.setrlimit(resource.RLIMIT_AS, (lim, lim))
    try:
        return census_one(path, max_bytes, family)
    except MemoryError:
        row = {c: "" for c in COLUMNS}
        row.update(stem=stem_of(path), family=family, error="memory")
        return row
    except Exception as e:  # noqa: BLE001 — one bad cell must not stop the census
        row = {c: "" for c in COLUMNS}
        row.update(stem=stem_of(path), family=family, error=f"{type(e).__name__}: {e}"[:200])
        return row


def cmd_census(args):
    files = sorted(Path(args.suite).glob("*.cnf.xz"))
    if args.only:
        files = [f for f in files if any(s in f.name for s in args.only.split(","))]
    fam = families()
    done = {}
    out = Path(args.out)
    if out.is_file() and args.resume:
        with open(out) as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                if not r["error"]:
                    done[r["stem"]] = r
    todo = [f for f in files if stem_of(f) not in done]
    # the largest first, so the long ones do not finish last
    todo.sort(key=lambda f: -f.stat().st_size)
    rows = dict(done)
    t0 = time.time()
    with ProcessPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(_worker, str(f), args.max_mb << 20, fam.get(stem_of(f), ""), args.mem_mb): f
                for f in todo}
        for i, fu in enumerate(as_completed(futs), 1):
            r = fu.result()
            rows[r["stem"]] = r
            print(f"[{i}/{len(todo)} {time.time() - t0:6.0f}s] {r['stem'][:60]:60} shift {r['shift']} repeat {r['repeat']} "
                  f"frames {r['frames']} sigma {r['sigma_cover']} {r['error']}", flush=True)
    tmp = out.with_suffix(".tmp")
    with open(tmp, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=COLUMNS, delimiter="\t", extrasaction="ignore")
        w.writeheader()
        for stem in sorted(rows):
            w.writerow(rows[stem])
    os.replace(tmp, out)
    print(f"wrote {out}: {len(rows)} rows")
    return 0


def cmd_one(args):
    r = census_one(args.cnf, args.max_mb << 20)
    for c in COLUMNS:
        print(f"{c:18} {r[c]}")
    return 0


# ---------------------------------------------------------------- the report

SOLVED = ("SATISFIABLE", "UNSATISFIABLE")


def run_status(runs):
    """Per stem: results and times of the two seeds' arms with every default on."""
    res = defaultdict(dict)
    for arm in ("cur0", "cur1"):
        p = Path(runs) / arm / "results.tsv"
        if not p.is_file():
            continue
        with open(p) as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                res[r["instance"]][arm] = (r["result"], float(r["time_s"]))
    return res


def num(r, k):
    v = r.get(k, "")
    try:
        return float(v)
    except (TypeError, ValueError):
        return 0.0


# What each family is, read from its cell names (an unrolled kind only where the
# names say so: a depth k or a step, a design from the model checking contests)
KIND = {
    "pj": "BMC, pj20xx at depth k",
    "oski": "BMC, HWMCC15 oski designs, optimized",
    "g2-t": "BMC, HWMCC15 deep (k20) and g2-T49",
    "sat-dat": "BMC at depth 100",
    "valves": "BMC at depth 617, shuffled",
    "hwmcc-iter": "HWMCC designs, IterN",
    "hwmcc-bob": "HWMCC bob design, optimized",
    "xits-miters": "HWMCC17 miters",
    "hwmc-step-transition": "k-induction step or one transition",
    "planning": "planning steps",
    "oisc": "program unrolling (OISC)",
    "linked-list": "program unrolling (unwind 50)",
}


def frame_periodic(r):
    return num(r, "shift") >= TIME_MAJOR and num(r, "repeat") >= PERIODIC and num(r, "frames") >= MIN_FRAMES \
        and num(r, "copies") >= MIN_FRAMES


def small_periodic(r):
    return 0 < num(r, "shift") < TIME_MAJOR and num(r, "repeat") >= PERIODIC and num(r, "copies") >= MIN_FRAMES


def partly(r):
    return num(r, "shift") >= TIME_MAJOR and 0.25 <= num(r, "repeat") < PERIODIC and num(r, "frames") >= MIN_FRAMES \
        and num(r, "copies") >= MIN_FRAMES


def cmd_report(args):
    with open(args.tsv) as fh:
        rows = [r for r in csv.DictReader(fh, delimiter="\t")]
    st = run_status(args.runs) if args.runs else {}

    def solved(r):
        s = st.get(r["stem"], {})
        ok = sorted(t for (res, t) in s.values() if res in SOLVED)
        return len(ok), len(s), ok

    fr = [r for r in rows if frame_periodic(r)]
    sm = [r for r in rows if small_periodic(r)]
    pt = [r for r in rows if partly(r)]
    print(f"{len(rows)} cells; errors {sum(1 for r in rows if r['error'])}; read as a prefix {sum(1 for r in rows if r['prefix'] == '1')}")
    print(f"frame shift (d >= {TIME_MAJOR}, repeat >= {PERIODIC}, {MIN_FRAMES}+ frames and copies): {len(fr)} cells")
    print(f"partly (frame shift, repeat 0.25-{PERIODIC}): {len(pt)} cells")
    print(f"small shift (d < {TIME_MAJOR}, repeat >= {PERIODIC}, {MIN_FRAMES}+ copies): {len(sm)} cells")

    def fam_table(cells, title):
        byfam = defaultdict(list)
        for r in cells:
            byfam[r["family"] or "?"].append(r)
        allfam = Counter_(r["family"] for r in rows)
        print(f"\n{title}")
        print("| family | kind | cells | periodic | repeat | frame vars | copies | segments | within / crossing / reach2 "
              "| start / goal / long / middle / outside | solved (seeds) | median s |")
        print("|---|---|---:|---:|---:|---|---:|---:|---|---|---:|---:|")
        for f, rs in sorted(byfam.items(), key=lambda x: (-len(x[1]), x[0])):
            med = lambda k: statistics.median(num(r, k) for r in rs)
            rep_n = sum(num(r, "within") + num(r, "crossing") + num(r, "reach2") for r in rs) or 1
            dist = sum(num(r, "distinct") for r in rs) or 1
            sv = [solved(r) for r in rs]
            ok = sum(a for a, _, _ in sv)
            tot = sum(b for _, b, _ in sv)
            times = sorted(x for _, _, ts in sv for x in ts)
            fv = sorted({int(num(r, "shift")) for r in rs})
            fvs = ", ".join(f"{x:,}" for x in fv[:3]) + (" ..." if len(fv) > 3 else "")
            share = lambda k: sum(num(r, k) for r in rs) / dist
            print(f"| {f} | {KIND.get(f, '')} | {allfam[f]} | {len(rs)} | {med('repeat'):.2f} | {fvs} | {med('copies'):g} "
                  f"| {med('segments'):g} "
                  f"| {sum(num(r, 'within') for r in rs) / rep_n:.2f} / {sum(num(r, 'crossing') for r in rs) / rep_n:.2f} / "
                  f"{sum(num(r, 'reach2') for r in rs) / rep_n:.2f} "
                  f"| {share('start'):.3f} / {share('goal'):.3f} / {share('long'):.3f} / {share('middle'):.3f} / {share('outside'):.3f} "
                  f"| {ok}/{tot} | {(f'{statistics.median(times):.0f}' if times else '-')} |")

    fam_table(fr, "Frame shifts, per family")
    fam_table(pt, "Partly periodic at a frame shift, per family")
    byfam = defaultdict(list)
    for r in sm:
        byfam[r["family"] or "?"].append(r)
    print("\nSmall shifts, per family: cells, shifts, median repeat, median copies, solved seeds")
    for f, rs in sorted(byfam.items(), key=lambda x: (-len(x[1]), x[0])):
        sv = [solved(r) for r in rs]
        print(f"  {f:22} {len(rs):3}  shifts {sorted({int(num(r, 'shift')) for r in rs})}  repeat "
              f"{statistics.median(num(r, 'repeat') for r in rs):.2f}  copies {statistics.median(num(r, 'copies') for r in rs):g}"
              f"  solved {sum(a for a, _, _ in sv)}/{sum(b for _, b, _ in sv)}")
    print("\nA renaming in file order maps half the clauses or more, and the shift does not (repeat below 0.5):")
    for r in sorted(rows, key=lambda r: -num(r, "sigma_cover")):
        if num(r, "sigma_cover") >= 0.5 and num(r, "repeat") < PERIODIC:
            a, b, _ = solved(r)
            print(f"  {r['stem'][33:75]:42} {r['family']:20} repeat {num(r, 'repeat'):.2f} sigma {num(r, 'sigma_cover'):.2f} "
                  f"(consistent {num(r, 'sigma_consistent'):.2f}; as a shift {r['sigma_shift']} for {num(r, 'sigma_shift_share'):.2f}) "
                  f"cperiod {r['cperiod']} solved {a}/{b}")
    if st:
        print("\nPeriodic or partly periodic cells not solved on both seeds of the run of record:")
        for r in sorted(fr + pt + sm, key=lambda r: (r["family"], r["stem"])):
            a, b, ts = solved(r)
            if a < b:
                kind = "frame" if r in fr else ("partly" if r in pt else "small")
                print(f"  {r['stem'][33:80]:47} {r['family']:20} {kind:6} solved {a}/{b} repeat {num(r, 'repeat'):.2f} "
                      f"shift {r['shift']} copies {r['copies']} frames {r['frames']}")
        print("\nSlow solves (median of the solved seeds over 300 s) among frame and partly periodic cells:")
        for r in sorted(fr + pt, key=lambda r: (r["family"], r["stem"])):
            a, b, ts = solved(r)
            if a and statistics.median(ts) > 300:
                print(f"  {r['stem'][33:80]:47} {r['family']:20} solved {a}/{b} median {statistics.median(ts):.0f} s "
                      f"(seeds {', '.join(f'{x:.0f}' for x in ts)}) "
                      f"repeat {num(r, 'repeat'):.2f} copies {r['copies']}")
    return 0


def Counter_(it):
    c = defaultdict(int)
    for x in it:
        c[x] += 1
    return c


# ---------------------------------------------------------------- self-test on hand-made unrollings

class Circuit:
    """A random sequential circuit: inputs, latches reset to 0, AND gates and a
    bad-state gate. frame(t) numbers its variables from base + t * size."""

    def __init__(self, seed, inputs=3, latches=5, gates=24):
        import random
        rng = random.Random(seed)
        self.i, self.l, self.g = inputs, latches, gates
        self.size = inputs + latches + gates
        self.ands = []
        for k in range(gates):
            pool = inputs + latches + k
            a, b = rng.sample(range(pool), 2)
            self.ands.append(((a, rng.random() < 0.5), (b, rng.random() < 0.5)))
        self.next = [(rng.randrange(inputs + latches, self.size), rng.random() < 0.5) for _ in range(latches)]
        self.bad = self.size - 1

    def unroll(self, k, number, long_range=False):
        """Clauses of k frames; number(t, local) gives the variable. Returns
        (clauses, expected counts)."""
        cl = []
        lit = lambda t, x, neg: -number(t, x) if neg else number(t, x)
        for t in range(k):
            for j, ((a, na), (b, nb)) in enumerate(self.ands):
                gv = self.i + self.l + j
                cl.append([-number(t, gv), lit(t, a, na)])
                cl.append([-number(t, gv), lit(t, b, nb)])
                cl.append([number(t, gv), -lit(t, a, na), -lit(t, b, nb)])
        for t in range(k - 1):
            for j, (x, nx) in enumerate(self.next):
                lv = self.i + j
                cl.append([-number(t + 1, lv), lit(t, x, nx)])
                cl.append([number(t + 1, lv), -lit(t, x, nx)])
        start = [[-number(0, self.i + j)] for j in range(self.l)]
        goal = [[number(k - 1, self.bad)]]
        longc = []
        if long_range:
            # the state of frame 0 differs from the state of the last frame in latch 0 or 1
            l0, l1 = self.i, self.i + 1
            longc = [[number(0, l0), number(k - 1, l0), number(0, l1), number(k - 1, l1)]]
        return cl + start + goal + longc, dict(within=3 * self.g * k, crossing=2 * self.l * (k - 1),
                                              start=len(start), goal=len(goal), long=len(longc))


def write_toy(path, clauses, shuffle_seed=None):
    import random
    nv = max(abs(x) for c in clauses for x in c)
    if shuffle_seed is not None:
        rng = random.Random(shuffle_seed)
        perm = list(range(1, nv + 1))
        rng.shuffle(perm)
        clauses = [[(perm[abs(x) - 1] if x > 0 else -perm[abs(x) - 1]) for x in c] for c in clauses]
        rng.shuffle(clauses)
    with open(path, "w") as fh:
        fh.write(f"p cnf {nv} {len(clauses)}\n")
        for c in clauses:
            fh.write(" ".join(map(str, c)) + " 0\n")


def cmd_selftest(args):
    import random
    import tempfile
    fails = []

    def check(name, cond, detail):
        print(f"  {'ok  ' if cond else 'FAIL'} {name}: {detail}")
        if not cond:
            fails.append(name)

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        c = Circuit(1)
        k = 20
        # 1. frame by frame from variable 1: the shift is the frame size, every count exact
        cl, ex = c.unroll(k, lambda t, x: 1 + t * c.size + x)
        write_toy(tmp / "a.cnf", cl)
        r = census_one(tmp / "a.cnf")
        check("time-major shift", r["shift"] == c.size, f"shift {r['shift']} (frame {c.size})")
        check("time-major frames", r["frames"] == k, f"frames {r['frames']} (unrolled {k})")
        check("time-major counts", all(r[x] == ex[x] for x in ("within", "crossing", "start", "goal", "long"))
              and r["middle"] == 0 and r["reach2"] == 0 and r["outside"] == 0,
              f"within {r['within']}/{ex['within']} crossing {r['crossing']}/{ex['crossing']} start {r['start']}/{ex['start']} "
              f"goal {r['goal']}/{ex['goal']} long {r['long']} middle {r['middle']} reach2 {r['reach2']}")
        # 1b. a latch no gate reads: its start unit is still a start clause (its only
        # other clauses have no copy), not outside the frames
        c3 = Circuit(3)
        cl, ex = c3.unroll(k, lambda t, x: 1 + t * c3.size + x)
        write_toy(tmp / "a3.cnf", cl)
        r = census_one(tmp / "a3.cnf")
        check("unread latch", r["start"] == ex["start"] and r["outside"] == 0 and r["goal"] == ex["goal"],
              f"start {r['start']}/{ex['start']} goal {r['goal']}/{ex['goal']} outside {r['outside']}")
        # 2. an offset before frame 0 and a long-range property clause
        cl, ex = c.unroll(k, lambda t, x: 101 + t * c.size + x, long_range=True)
        write_toy(tmp / "b.cnf", cl)
        r = census_one(tmp / "b.cnf")
        check("offset and long-range", r["shift"] == c.size and r["frames"] == k and r["long"] == 1
              and r["start"] == ex["start"] and r["goal"] == ex["goal"],
              f"shift {r['shift']} frames {r['frames']} long {r['long']} start {r['start']} goal {r['goal']}")
        # 3. one variable per signal per step (planning style): the shift is 1, a gate
        # clause has a copy at every step, and the frame classes are left blank
        cl, ex = c.unroll(k, lambda t, x: 1 + x * k + t)
        write_toy(tmp / "c.cnf", cl)
        r = census_one(tmp / "c.cnf")
        check("signal-major shift", r["shift"] == 1 and r["copies"] == k and r["frames"] == k and r["start"] == "",
              f"shift {r['shift']} copies {r['copies']} frames {r['frames']} repeat {r['repeat']}")
        # 4. frames at uneven offsets (gaps of 0-40 variables): no single shift; the order detector finds the renaming
        rng = random.Random(5)
        bases, b = [], 1
        for t in range(k):
            bases.append(b)
            b += c.size + rng.randrange(0, 41)
        cl, ex = c.unroll(k, lambda t, x: bases[t] + x)
        # written frame by frame: gate clauses of frame t, then the transition into frame t + 1
        frame_of = lambda cls: min((abs(x) for x in cls))
        cl.sort(key=frame_of)
        write_toy(tmp / "d.cnf", cl)
        r = census_one(tmp / "d.cnf")
        check("uneven offsets: shift fails", r["repeat"] < 0.5, f"repeat {r['repeat']} at shift {r['shift']}")
        check("uneven offsets: renaming found", r["sigma_cover"] >= 0.85, f"sigma_cover {r['sigma_cover']} cperiod {r['cperiod']} "
              f"consistent {r['sigma_consistent']}")
        # 5. random 3-SAT: nothing periodic
        rng = random.Random(9)
        cl = [[rng.choice([-1, 1]) * v for v in rng.sample(range(1, 301), 3)] for _ in range(1260)]
        write_toy(tmp / "e.cnf", cl)
        r = census_one(tmp / "e.cnf")
        check("random 3-SAT", r["repeat"] < 0.05 and r["sigma_cover"] < 0.05, f"repeat {r['repeat']} sigma {r['sigma_cover']}")
        # 6. the time-major toy shuffled (variables and clauses): both detectors fail, as documented
        cl, ex = c.unroll(k, lambda t, x: 1 + t * c.size + x)
        write_toy(tmp / "f.cnf", cl, shuffle_seed=3)
        r = census_one(tmp / "f.cnf")
        check("shuffled defeats both", r["repeat"] < 0.2 and r["sigma_cover"] < 0.2, f"repeat {r['repeat']} sigma {r['sigma_cover']}")
        # 7. a prefix read: the shift and frames from the part read, the flag set
        cl, ex = c.unroll(k, lambda t, x: 1 + t * c.size + x)
        write_toy(tmp / "g.cnf", cl)
        r = census_one(tmp / "g.cnf", max_bytes=os.path.getsize(tmp / "g.cnf") // 2)
        check("prefix", r["prefix"] == 1 and r["shift"] == c.size and 5 <= r["frames"] < k,
              f"prefix {r['prefix']} shift {r['shift']} frames {r['frames']}")
        # 9. two copies of a 20-frame unrolling side by side: the frame shift wins over
        # the shift between the copies, though both give every clause a copy one way
        cl, ex = c.unroll(k, lambda t, x: 1 + t * c.size + x)
        off = k * c.size + 7
        write_toy(tmp / "i.cnf", cl + [[x + off if x > 0 else x - off for x in cc] for cc in cl])
        r = census_one(tmp / "i.cnf")
        # (frames counts the periodic blocks of both copies, 2k; a clause has k copies)
        check("frames beat halves", r["shift"] == c.size and r["copies"] == k and r["frames"] == 2 * k,
              f"shift {r['shift']} (copies offset {off}) frames {r['frames']} copies {r['copies']}")
        # 10. random 3-SAT written variable by variable: neighbouring clauses share their
        # first literal, which an identity renaming would map onto itself
        rng = random.Random(11)
        cl = sorted(([rng.choice([-1, 1]) * v for v in sorted(rng.sample(range(1, 301), 3))] for _ in range(1260)),
                    key=lambda cc: abs(cc[0]))
        write_toy(tmp / "j.cnf", cl)
        r = census_one(tmp / "j.cnf")
        check("no identity renaming", r["sigma_cover"] < 0.05, f"sigma {r['sigma_cover']} cperiod {r['cperiod']}")
        # 11. a truncated .xz is an error, not a shorter formula
        subprocess.run(["xz", "-kf", str(tmp / "a.cnf")], check=True)
        raw = (tmp / "a.cnf.xz").read_bytes()
        (tmp / "cut.cnf.xz").write_bytes(raw[: len(raw) // 2])
        try:
            census_one(tmp / "cut.cnf.xz")
            check("truncated xz", False, "accepted")
        except RuntimeError as e:
            check("truncated xz", "xz failed" in str(e), str(e)[:60])
        # 12. both signs of one variable: the order they are written in changes nothing
        write_toy(tmp / "k1.cnf", [[1, -1], [2, -2], [3, -3], [5, 9]])
        write_toy(tmp / "k2.cnf", [[1, -1], [-2, 2], [3, -3], [5, 9], [1, -1, 1]])
        r1, r2 = census_one(tmp / "k1.cnf"), census_one(tmp / "k2.cnf")
        check("tautologies by sign", r1["copies"] == r2["copies"] == 3 and r2["distinct"] == 4,
              f"copies {r1['copies']} and {r2['copies']}, distinct {r2['distinct']}")
        # 13. a chain far longer than any step-by-step scan would follow, counted exactly
        write_toy(tmp / "l.cnf", [[i, i + 1] for i in range(1, 50001)] + [[-1, -3, -7]])
        r = census_one(tmp / "l.cnf")
        check("long chain exact", r["shift"] == 1 and r["copies"] == 50000, f"shift {r['shift']} copies {r['copies']}")
        # 8. duplicates and repeated literals do not change the counts
        cl, ex = c.unroll(k, lambda t, x: 1 + t * c.size + x)
        cl2 = cl + cl[:50] + [cl[60] + cl[60][:1]]
        write_toy(tmp / "h.cnf", cl2)
        r = census_one(tmp / "h.cnf")
        check("duplicates", r["distinct"] == len(cl) and r["within"] == ex["within"] and r["crossing"] == ex["crossing"],
              f"distinct {r['distinct']}/{len(cl)} within {r['within']} crossing {r['crossing']}")
    print(f"selftest: {'all passed' if not fails else 'FAILED: ' + ', '.join(fails)}")
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("one")
    a.add_argument("cnf")
    a.add_argument("--max-mb", type=int, default=1500)
    a = sub.add_parser("census")
    a.add_argument("--suite", default=str(REPO / "benchmarks/sat-comp-2025"))
    a.add_argument("--out", default=str(REPO / "benchmarks/rl/periodic_2025.tsv"))
    a.add_argument("--jobs", type=int, default=32)
    a.add_argument("--mem-mb", type=int, default=16000)
    a.add_argument("--max-mb", type=int, default=1500, help="decompressed text read per file (MB)")
    a.add_argument("--only", default="", help="comma-separated name parts")
    a.add_argument("--resume", action="store_true")
    a = sub.add_parser("report")
    a.add_argument("--tsv", default=str(REPO / "benchmarks/rl/periodic_2025.tsv"))
    a.add_argument("--runs", default="")
    sub.add_parser("selftest")
    args = ap.parse_args()
    return {"one": cmd_one, "census": cmd_census, "report": cmd_report, "selftest": cmd_selftest}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
