#!/usr/bin/env python3
"""rl_structure.py — a structure census of the benchmark cells.

Plan: plan/rl-scheduler-solver13-plan.md §11 (2026-10-06). What kind of
structure each instance carries, so that rewrites (symmetry breaking,
cardinality and parity reasoning, gate-level rewriting) and a per-instance
selector have something to go on.

`census`: one row per cell of the cells table, from three sources.
  1. Cheap detectors on the CNF itself (streamed from the .xz, the first
     --max-clauses clauses on the giants, flagged `prefix`):
       - shape: clause-length histogram, binary share, a uniform-random flag
         (every clause the same length and Poisson-like occurrences);
       - XOR constraints: a variable set of size k whose 2^(k-1) clauses are
         exactly the sign patterns of one parity (k up to --xor-max);
       - at-most-one groups: cliques in the graph whose edges are the binary
         clauses (greedy maximal cliques, seeded by degree, --amo-seeds
         seeds), and exactly-one groups when the matching at-least-one
         clause is present;
       - unrolling: clauses grouped by their relative shape (signs and
         variable offsets); the most common distance between copies of a
         shape is the period, its coverage the share of clauses with a copy
         at that distance (time-step encodings: planning, model checking).
  2. The solver's own census from the converted stock pass (policy_static.rs):
     gates matched by kind, the share of variables defined by a gate,
     binary-implication statistics, locality and the two-class signature.
  3. Satsuma (the SAT Competition 2026 winner's symmetry preprocessor, built
     from source; --satsuma <binary>) in `fix` mode with --sym-timeout: the
     number of symmetry generators, the structures it recognises (row
     interchangeability, row-column, Johnson), the units it fixes, and
     whether it decides the instance outright. Skipped on cells whose .xz
     is over --sym-max-mb.

Builds (outside the repo; 2026-10-06 versions Satsuma 1.4 / dejavu 2.1 and
VeriPB 3.0.2):
    git clone --recursive https://github.com/markusa4/satsuma && cd satsuma \\
        && cmake . -DCMAKE_BUILD_TYPE=Release && make satsuma
    git clone https://gitlab.com/MIAOresearch/software/VeriPB && cd VeriPB \\
        && cargo build --release        # target/release/veripb

`report`: per family and for every cell a structure signature, the
structures on the unsolved cells, the small families next to the larger
ones with the same signature, and the cells that share a signature across
families.

`verify`: for every cell the census marked `sym_decided`, run Satsuma again
with a VeriPB proof and check it with VeriPB 3 (--veripb <binary>, built
from https://gitlab.com/MIAOresearch/software/VeriPB with cargo). Satsuma
1.4 writes VeriPB 2 proofs: one header per iteration, `delc` for every
deletion, the same clause deleted again by a later dedup pass, and no
conclusion. The rewrite keeps the first header, writes every deletion as
`del id`, drops a deletion of an ID already deleted and appends
`conclusion UNSAT`. Deletions never weaken a refutation, so the rewrite
cannot make an unsound proof pass; VeriPB runs with unchecked deletion
(-u), which is sound for an UNSAT conclusion. Output: a TSV with Satsuma's
verdict, VeriPB's verdict and both times.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_structure.py census --stock log/rl-stock2025-<ts> \\
        --satsuma <path>/satsuma --out benchmarks/rl/structure_2025.tsv --workers 24
    ~/.cache/sat13-rl/venv/bin/python tools/rl_structure.py report benchmarks/rl/structure_2025.tsv
    ~/.cache/sat13-rl/venv/bin/python tools/rl_structure.py verify benchmarks/rl/structure_2025.tsv \\
        --satsuma <path>/satsuma --veripb <path>/veripb --out log/structure-verify-<date>.tsv
"""
from __future__ import annotations

import argparse
import csv
import lzma
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CELLS = ROOT / "benchmarks" / "rl" / "cells_2025.tsv"
SUITE = ROOT / "benchmarks" / "sat-comp-2025"

STATIC_COLUMNS = ("s_shape_vars", "s_shape_clauses", "s_big_binary_frac", "s_gates_gates", "s_gates_ands",
                  "s_gates_xors", "s_gates_ites", "s_gates_matched", "s_gates_output_frac", "s_gates_equivalences",
                  "s_classify_small", "s_classify_bigbig", "s_locality_span_small_frac", "s_scc_count",
                  "s_occurrence_balance_mean", "s_big_deg_mean")

COLUMNS = ["stem", "family", "split", "status", "xz_mb", "vars", "clauses", "parsed", "prefix",
           "len_mode", "len_max", "binary_frac", "mean_len", "occ_var_mean", "uniform_random",
           "xor_k2", "xor_constraints", "xor_max_k", "xor_clause_frac", "xor_var_frac",
           "amo_groups", "amo_max", "amo_median", "amo_var_frac", "eo_groups",
           "period", "period_cover", "copy_cover",
           "gates_matched", "gates_ands", "gates_xors", "gates_ites", "gate_output_frac", "gate_equivalences",
           "classify_small", "classify_bigbig", "span_small_frac", "scc_count",
           "sym_status", "sym_gens", "sym_row", "sym_rowcol", "sym_johnson", "sym_units", "sym_amo_binary",
           "sym_remain_pct", "sym_decided", "sym_s", "detect_s", "signature"]


# ---------------------------------------------------------------------------
# the CNF detectors
# ---------------------------------------------------------------------------

def normalize(cur: list[int]):
    """A clause with repeated literals dropped, or None for a tautology."""
    seen = set(cur)
    if any(-l in seen for l in seen):
        return None
    return tuple(sorted(seen, key=abs)) if len(seen) < len(cur) else tuple(cur)


def parse_prefix(path: Path, max_clauses: int):
    """(n_vars, n_clauses_header, clauses, truncated) with at most
    max_clauses clauses, each a tuple of ints with repeated literals dropped
    and tautologies skipped; comment and header lines skipped; `truncated`
    says the file had more clauses than were read."""
    clauses = []
    n_vars = n_cls = 0
    cur: list[int] = []
    with lzma.open(path, "rt", encoding="ascii", errors="replace") as f:
        for line in f:
            if not line or line[0] in "c%":
                continue
            if line[0] == "p":
                parts = line.split()
                if len(parts) >= 4:
                    n_vars, n_cls = int(parts[2]), int(parts[3])
                continue
            for tok in line.split():
                v = int(tok)
                if v == 0:
                    if cur:
                        c = normalize(cur)
                        cur = []
                        if c is not None:
                            clauses.append(c)
                            if len(clauses) >= max_clauses:
                                return n_vars, n_cls, clauses, True
                else:
                    cur.append(v)
    if cur:
        c = normalize(cur)
        if c is not None:
            clauses.append(c)
    return n_vars, n_cls, clauses, False


def shape_stats(clauses, n_vars):
    lens = Counter(len(c) for c in clauses)
    n = len(clauses)
    mode = lens.most_common(1)[0][0] if lens else 0
    occ = Counter()
    for c in clauses:
        for lit in c:
            occ[abs(lit)] += 1
    counts = list(occ.values())
    mean = statistics.mean(counts) if counts else 0.0
    var = statistics.pvariance(counts) if len(counts) > 1 else 0.0
    ratio = var / mean if mean else 0.0
    uniform = int(n > 0 and lens[mode] == n and mode >= 3 and 0.6 <= ratio <= 1.6)
    return {"len_mode": mode, "len_max": max(lens) if lens else 0, "binary_frac": lens[2] / n if n else 0.0,
            "mean_len": sum(len(c) for c in clauses) / n if n else 0.0, "occ_var_mean": ratio, "uniform_random": uniform}


def xor_stats(clauses, n_vars, max_k):
    """XOR constraints: a variable set of size k (3..max_k) whose clauses are
    exactly the 2^(k-1) sign patterns of one parity; size-2 XORs (a pair of
    binary clauses that make two variables equal or opposite) counted apart."""
    groups: dict[tuple, set] = defaultdict(set)
    for c in clauses:
        k = len(c)
        if 2 <= k <= max_k:
            key = tuple(sorted(abs(l) for l in c))
            if len(set(key)) == k:
                groups[key].add(tuple(1 if l > 0 else 0 for l in sorted(c, key=abs)))
    n_xor = n_k2 = 0
    max_k_found = 0
    xor_clauses = 0
    xor_vars: set[int] = set()
    for key, pats in groups.items():
        k = len(key)
        need = 1 << (k - 1)
        if len(pats) < need:
            continue
        for parity in (0, 1):
            sel = [p for p in pats if (k - sum(p)) % 2 == parity]       # number of negative literals
            if len(sel) == need:
                if k == 2:
                    n_k2 += 1
                else:
                    n_xor += 1
                    max_k_found = max(max_k_found, k)
                    xor_clauses += need
                    xor_vars.update(key)
                break
    n = len(clauses)
    return {"xor_k2": n_k2, "xor_constraints": n_xor, "xor_max_k": max_k_found,
            "xor_clause_frac": xor_clauses / n if n else 0.0, "xor_var_frac": len(xor_vars) / n_vars if n_vars else 0.0}


def amo_stats(clauses, n_vars, max_seeds):
    """At-most-one groups: a binary clause (a v b) forbids -a and -b together,
    so the literals -a, -b are adjacent; a clique is an at-most-one over its
    literals. Greedy maximal cliques from the highest-degree seeds; a group
    is exactly-one when the clause over its literals is present."""
    adj: dict[int, set] = defaultdict(set)
    for c in clauses:
        if len(c) == 2:
            a, b = -c[0], -c[1]
            if a != b and a != -b:
                adj[a].add(b)
                adj[b].add(a)
    present = set(tuple(sorted(c)) for c in clauses if 3 <= len(c) <= 4096)
    seeds = sorted((l for l in adj if len(adj[l]) >= 2), key=lambda l: -len(adj[l]))[:max_seeds]
    seen_as_member: set[int] = set()
    sizes = []
    eo = 0
    covered: set[int] = set()
    for s in seeds:
        if s in seen_as_member:
            continue
        clique = [s]
        cand = set(adj[s])
        while cand:
            # the candidate with the most neighbours among the remaining candidates
            best = max(cand, key=lambda x: len(adj[x] & cand))
            clique.append(best)
            cand &= adj[best]
        if len(clique) >= 3:
            sizes.append(len(clique))
            seen_as_member.update(clique)
            covered.update(abs(l) for l in clique)
            if tuple(sorted(clique)) in present:
                eo += 1
    return {"amo_groups": len(sizes), "amo_max": max(sizes) if sizes else 0,
            "amo_median": statistics.median(sizes) if sizes else 0, "amo_var_frac": len(covered) / n_vars if n_vars else 0.0,
            "eo_groups": eo}


def unroll_stats(clauses, max_len=16):
    """Clauses grouped by relative shape (signs and variable offsets); the
    most common distance between copies of a shape is the period, its
    coverage the share of clauses with a copy at that distance. `copy_cover`
    is the share of clauses whose shape occurs more than once at all."""
    shapes: dict[tuple, list] = defaultdict(list)
    for c in clauses:
        if len(c) > max_len:
            continue
        s = sorted(c, key=abs)
        base = abs(s[0])
        shapes[tuple((1 if l > 0 else -1) * (abs(l) - base + 1) for l in s)].append(base)
    diffs: Counter = Counter()
    copies = 0
    n = len(clauses)
    multi = []
    for s, bases in shapes.items():
        if len(bases) < 2:
            continue
        copies += len(bases)
        bases.sort()
        multi.append(bases)
        for a, b in zip(bases, bases[1:]):
            if b > a:
                diffs[b - a] += 1
    if not diffs:
        return {"period": 0, "period_cover": 0.0, "copy_cover": copies / n if n else 0.0}
    period = diffs.most_common(1)[0][0]
    # the clauses with a copy one period before or after them
    covered = 0
    for bases in multi:
        present = set(bases)
        covered += sum(1 for b in bases if (b - period) in present or (b + period) in present)
    return {"period": period, "period_cover": covered / n if n else 0.0, "copy_cover": copies / n if n else 0.0}


SYM_RE = {
    "sym_gens": re.compile(r"dejavu_gens\s*=\s*(\d+)"),
    "sym_row": re.compile(r"\brow\s*=\s*(\d+)"),
    "sym_rowcol": re.compile(r"row_column\s*=\s*(\d+)"),
    "sym_johnson": re.compile(r"johnson\s*=\s*(\d+)"),
    "sym_units": re.compile(r"symmetry_units\s*=\s*(\d+)"),
    "sym_amo_binary": re.compile(r"amo_binary\s*=\s*(\d+)"),
}


def run_satsuma(binary: str, cnf_xz: Path, timeout: float, tmpdir: str):
    """Satsuma fix on the decompressed instance: the symmetry statistics it
    prints, the remaining share of the formula, and a verdict when it
    decides the instance."""
    out = {"sym_status": "", "sym_gens": "", "sym_row": "", "sym_rowcol": "", "sym_johnson": "", "sym_units": "",
           "sym_amo_binary": "", "sym_remain_pct": "", "sym_decided": "", "sym_s": ""}
    t0 = time.time()
    plain = os.path.join(tmpdir, "in.cnf")
    with lzma.open(cnf_xz, "rb") as src, open(plain, "wb") as dst:
        for chunk in iter(lambda: src.read(1 << 24), b""):
            dst.write(chunk)
    reduced_empty = False
    try:
        r = subprocess.run([binary, "fix", "--file", plain, "--out-file", os.path.join(tmpdir, "out.cnf")],
                           capture_output=True, text=True, timeout=timeout)
        text = re.sub(r"\x1b\[[0-9;]*m", "", (r.stdout or "") + (r.stderr or ""))
        out["sym_status"] = "ok" if r.returncode == 0 else f"exit{r.returncode}"
        # a satisfiable formula fixed down to nothing is written as `p cnf N 0`
        # with no verdict line
        try:
            with open(os.path.join(tmpdir, "out.cnf")) as fh:
                for line in fh:
                    if line.startswith("p cnf"):
                        reduced_empty = int(line.split()[3]) == 0
                        break
        except (OSError, IndexError, ValueError):
            pass
    except subprocess.TimeoutExpired as e:
        text = re.sub(r"\x1b\[[0-9;]*m", "", ((e.stdout or b"").decode(errors="replace") + (e.stderr or b"").decode(errors="replace")))
        out["sym_status"] = "timeout"
    except OSError as e:
        text = ""
        out["sym_status"] = f"error:{e.__class__.__name__}"
    finally:
        for f in ("in.cnf", "out.cnf"):
            try:
                os.remove(os.path.join(tmpdir, f))
            except OSError:
                pass
    for key, rx in SYM_RE.items():
        m = rx.search(text)
        if m:
            out[key] = int(m.group(1))
    m = re.findall(r"output.*?([0-9.]+)%", text)
    if m:
        out["sym_remain_pct"] = float(m[-1])
    if "c UNSATISFIABLE" in text:
        out["sym_decided"] = "UNSAT"
    elif "c SATISFIABLE" in text or (out["sym_status"] == "ok" and reduced_empty):
        out["sym_decided"] = "SAT"
    out["sym_s"] = round(time.time() - t0, 1)
    return out


def census_one(args_tuple):
    stem, family, split, status, cfg = args_tuple
    path = SUITE / f"{stem}.cnf.xz"
    row = {c: "" for c in COLUMNS}
    row.update({"stem": stem, "family": family, "split": split, "status": status})
    if not path.is_file():
        row["sym_status"] = "no-cnf"
        return row
    row["xz_mb"] = round(os.path.getsize(path) / 1e6, 1)
    t0 = time.time()
    n_vars, n_cls, clauses, truncated = parse_prefix(path, cfg["max_clauses"])
    row.update({"vars": n_vars, "clauses": n_cls, "parsed": len(clauses), "prefix": int(truncated)})
    row.update(shape_stats(clauses, n_vars))
    row.update(xor_stats(clauses, n_vars, cfg["xor_max"]))
    row.update(amo_stats(clauses, n_vars, cfg["amo_seeds"]))
    row.update(unroll_stats(clauses))
    del clauses
    row["detect_s"] = round(time.time() - t0, 1)
    if cfg["satsuma"]:
        if row["xz_mb"] > cfg["sym_max_mb"]:
            row["sym_status"] = "too-big"
        else:
            with tempfile.TemporaryDirectory(prefix="structure-", dir=cfg["tmp"]) as d:
                row.update(run_satsuma(cfg["satsuma"], path, cfg["sym_timeout"], d))
    return row


def signature(row: dict) -> str:
    """A short structure signature from the flags, for grouping. Tags:
    random (uniform clause length, Poisson occurrences), xor-pure (98 % or
    more of the clauses are XOR constraints), xor-heavy (half or more),
    xor (ten or more XOR constraints, or 5 % of the variables in one), exactly-one / at-most-one
    (five or more groups over 30 % / 10 % of the variables), unrolled (30 %
    of the clauses have a copy at the period), circuit (the solver's
    congruence closure defines 30 % of the active variables by a gate),
    xorgates (XOR gates for 2 % of the variables), sym (Satsuma found a
    symmetry generator), sym-rows (it recognised interchangeable rows,
    row-column or Johnson structure), sym-decided (symmetry fixing alone
    decided the instance)."""
    tags = []
    f = lambda k: float(row.get(k) or 0)
    if f("uniform_random"):
        tags.append("random")
    if f("xor_clause_frac") >= 0.98:
        tags.append("xor-pure")
    elif f("xor_clause_frac") >= 0.5:
        tags.append("xor-heavy")
    elif f("xor_constraints") >= 10 or f("xor_var_frac") >= 0.05:
        tags.append("xor")
    if f("eo_groups") >= 5 and f("amo_var_frac") >= 0.3:
        tags.append("exactly-one")
    elif f("amo_groups") >= 5 and f("amo_var_frac") >= 0.1:
        tags.append("at-most-one")
    if f("period_cover") >= 0.3 and f("period") >= 2:
        tags.append("unrolled")
    if f("gate_output_frac") >= 0.3:
        tags.append("circuit")
        if f("gates_xors") >= 0.02 * max(f("vars"), 1):
            tags.append("xorgates")
    if str(row.get("sym_status")) == "ok":
        structs = f("sym_row") + f("sym_rowcol") + f("sym_johnson")
        if structs > 0:
            tags.append("sym-rows")
        elif f("sym_gens") > 0:
            tags.append("sym")
        if row.get("sym_decided"):
            tags.append("sym-decided")
    return "+".join(tags) if tags else "plain"


def cmd_census(args) -> int:
    with open(args.cells, newline="") as fh:
        lines = [ln for ln in fh if not ln.startswith("#")]
    cells = list(csv.DictReader(lines, dialect="excel-tab"))
    static = {}
    if args.stock:
        import pyarrow.parquet as pq
        r = pq.read_table(Path(args.stock) / "dataset" / "runs.parquet").to_pydict()
        for i in range(len(r["run_id"])):
            if not r["is_child"][i] and r.get("static_computed", [True] * len(r["run_id"]))[i]:
                static[r["stem"][i]] = {c: r[c][i] for c in STATIC_COLUMNS if c in r}
    sym_from = {}
    if args.sym_from:
        with open(args.sym_from, newline="") as fh:
            for r in csv.DictReader(fh, dialect="excel-tab"):
                sym_from[r["stem"]] = {k: r[k] for k in COLUMNS if k.startswith("sym_")}
        if args.satsuma:
            raise SystemExit("--sym-from and --satsuma are alternatives")
    cfg = {"max_clauses": args.max_clauses, "xor_max": args.xor_max, "amo_seeds": args.amo_seeds,
           "satsuma": args.satsuma, "sym_timeout": args.sym_timeout, "sym_max_mb": args.sym_max_mb, "tmp": args.tmp}
    jobs = [(c["stem"], c.get("family", ""), c.get("split", ""), c.get("status", ""), cfg) for c in cells]
    rows = []
    t0 = time.time()
    with ProcessPoolExecutor(max_workers=args.workers) as ex:
        futs = {ex.submit(census_one, j): j[0] for j in jobs}
        for n, fut in enumerate(as_completed(futs), 1):
            try:
                row = fut.result()
            except Exception as e:                # one bad cell must not lose the census
                row = {c: "" for c in COLUMNS}
                row.update({"stem": futs[fut], "sym_status": f"error:{e.__class__.__name__}"})
            if row["stem"] in sym_from:
                row.update(sym_from[row["stem"]])
            s = static.get(row["stem"], {})
            row.update({"gates_matched": s.get("s_gates_matched", ""), "gates_ands": s.get("s_gates_ands", ""),
                        "gates_xors": s.get("s_gates_xors", ""), "gates_ites": s.get("s_gates_ites", ""),
                        "gate_output_frac": s.get("s_gates_output_frac", ""), "gate_equivalences": s.get("s_gates_equivalences", ""),
                        "classify_small": s.get("s_classify_small", ""), "classify_bigbig": s.get("s_classify_bigbig", ""),
                        "span_small_frac": s.get("s_locality_span_small_frac", ""), "scc_count": s.get("s_scc_count", "")})
            row["signature"] = signature(row)
            rows.append(row)
            if n % 25 == 0 or n == len(jobs):
                print(f"  {n}/{len(jobs)} cells, {time.time() - t0:.0f} s", flush=True)
    rows.sort(key=lambda r: (r["family"], r["stem"]))
    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=COLUMNS, dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    print(f"wrote {args.out}: {len(rows)} cells")
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------

def cmd_report(args) -> int:
    with open(args.table, newline="") as fh:
        rows = list(csv.DictReader(fh, dialect="excel-tab"))
    for r in rows:
        r["signature"] = signature(r)
    f = lambda r, k: float(r[k]) if r.get(k) not in ("", None) else float("nan")
    fams: dict[str, list] = defaultdict(list)
    for r in rows:
        fams[r["family"]].append(r)
    print(f"{len(rows)} cells, {len(fams)} families; prefix-parsed {sum(1 for r in rows if r.get('prefix') == '1')}, "
          f"symmetry run on {sum(1 for r in rows if r.get('sym_status') == 'ok')} "
          f"(timeout {sum(1 for r in rows if r.get('sym_status') == 'timeout')}, too big "
          f"{sum(1 for r in rows if r.get('sym_status') == 'too-big')})")
    print("\nper family: cells, and how many carry each structure")
    print(f"  {'family':<22s} {'n':>3s} {'random':>6s} {'xor':>4s} {'xorhvy':>6s} {'amo':>4s} {'eo':>3s} {'unroll':>6s} "
          f"{'circuit':>7s} {'xorg':>5s} {'sym':>4s} {'rows':>4s} {'symdec':>6s} {'signatures'}")
    for fam in sorted(fams, key=lambda x: (-len(fams[x]), x)):
        rs = fams[fam]
        sig = Counter(r["signature"] for r in rs)
        has = lambda tag: sum(1 for r in rs if tag in r["signature"].split("+"))
        print(f"  {fam[:22]:<22s} {len(rs):3d} {has('random'):6d} {has('xor'):4d} {has('xor-heavy') + has('xor-pure'):6d} "
              f"{has('at-most-one') + has('exactly-one'):4d} {has('exactly-one'):3d} {has('unrolled'):6d} "
              f"{has('circuit'):7d} {has('xorgates'):5d} {has('sym') + has('sym-rows'):4d} {has('sym-rows'):4d} "
              f"{has('sym-decided'):6d} "
              + ", ".join(f"{s} x{c}" if c > 1 else s for s, c in sig.most_common()))
    print("\nstructures on the unsolved cells (stock pass status TIMEOUT), and on all cells:")
    tags = ["random", "xor-pure", "xor-heavy", "xor", "exactly-one", "at-most-one", "unrolled", "circuit", "xorgates", "sym", "sym-rows", "sym-decided", "plain"]
    un = [r for r in rows if r.get("status") == "TIMEOUT"]
    print(f"  {'tag':<12s} {'unsolved':>8s} {'of':>4s} {'all':>5s} {'of':>4s}  families among the unsolved")
    for t in tags:
        u = [r for r in un if t in r["signature"].split("+")]
        a = [r for r in rows if t in r["signature"].split("+")]
        fc = Counter(r["family"] for r in u)
        print(f"  {t:<12s} {len(u):8d} {len(un):4d} {len(a):5d} {len(rows):4d}  " + ", ".join(f"{k} {v}" for k, v in fc.most_common(10)) + (" ..." if len(fc) > 10 else ""))
    print("\nsmall families (one or two cells): signature, our status, and the larger families with the same signature")
    big_by_sig: dict[str, Counter] = defaultdict(Counter)
    for r in rows:
        if len(fams[r["family"]]) >= 3:
            big_by_sig[r["signature"]][r["family"]] += 1
    for fam in sorted(fams):
        rs = fams[fam]
        if len(rs) > 2:
            continue
        for r in rs:
            like = big_by_sig.get(r["signature"], Counter())
            print(f"  {fam[:18]:<18s} {r['stem'][33:58]:<25s} {r.get('status', ''):<8s} {r['signature']:<44s} "
                  + (", ".join(f"{k} {v}" for k, v in like.most_common(5)) if like else "(no larger family)"))
    print("\nsignatures shared across families (cells per signature, families):")
    by_sig: dict[str, list] = defaultdict(list)
    for r in rows:
        by_sig[r["signature"]].append(r)
    for sig, rs in sorted(by_sig.items(), key=lambda kv: -len(kv[1])):
        fam_counts = Counter(r["family"] for r in rs)
        print(f"  {sig:<40s} {len(rs):3d} cells in {len(fam_counts):2d} families: "
              + ", ".join(f"{k} {v}" for k, v in fam_counts.most_common(8)) + (" ..." if len(fam_counts) > 8 else ""))
    print("\nsymmetry (Satsuma fix): cells with generators or recognised structures, by family")
    for fam in sorted(fams, key=lambda x: (-len(fams[x]), x)):
        rs = [r for r in fams[fam] if r.get("sym_status") == "ok" and (f(r, "sym_gens") > 0 or f(r, "sym_rowcol") > 0 or f(r, "sym_row") > 0 or f(r, "sym_johnson") > 0)]
        if rs:
            print(f"  {fam[:22]:<22s} {len(rs):3d} of {len(fams[fam]):3d}: generators median {statistics.median(f(r, 'sym_gens') for r in rs):.0f}, "
                  f"row {sum(int(f(r, 'sym_row')) for r in rs)}, row-column {sum(int(f(r, 'sym_rowcol')) for r in rs)}, "
                  f"johnson {sum(int(f(r, 'sym_johnson')) for r in rs)}, units fixed median {statistics.median(f(r, 'sym_units') for r in rs):.0f}, "
                  f"decided {sum(1 for r in rs if r.get('sym_decided'))}")
    print("\ncells with no recognised structure ('plain'):")
    plain = [r for r in rows if r["signature"] == "plain"]
    print("  " + ", ".join(f"{r['family']} ({r['stem'][33:50]})" for r in plain[:60]) + (" ..." if len(plain) > 60 else ""))
    print(f"  {len(plain)} cells in {len({r['family'] for r in plain})} families")
    return 0


# ---------------------------------------------------------------------------
# verify: check Satsuma's decisions with VeriPB
# ---------------------------------------------------------------------------

def rewrite_proof(src: str, dst: str) -> None:
    """Satsuma's VeriPB 2 proof as a VeriPB 3 refutation (see the module
    docstring): one header, `del id` deletions, repeated deletions dropped,
    an UNSAT conclusion."""
    deleted: set[str] = set()
    with open(src) as f, open(dst, "w") as out:
        first = True
        for line in f:
            if line.startswith("pseudo-Boolean proof version"):
                if first:
                    out.write(line)
                    first = False
                continue
            if line.startswith("delc ") or line.startswith("del id "):
                ids = line.split()[2 if line.startswith("del id") else 1:]
                ids = [i.rstrip(";") for i in ids if i.rstrip(";")]
                keep = [i for i in ids if i not in deleted]
                deleted.update(keep)
                if keep:
                    out.write("del id " + " ".join(keep) + " ;\n")
                continue
            out.write(line)
        out.write("output NONE ;\nconclusion UNSAT ;\nend pseudo-Boolean proof ;\n")


def verify_one(stem: str, satsuma: str, veripb: str, timeout: float, tmpdir: str) -> dict:
    out = {"stem": stem, "satsuma": "", "veripb": "", "satsuma_s": "", "veripb_s": ""}
    plain = os.path.join(tmpdir, "in.cnf")
    with lzma.open(SUITE / f"{stem}.cnf.xz", "rb") as src, open(plain, "wb") as dst:
        for chunk in iter(lambda: src.read(1 << 24), b""):
            dst.write(chunk)
    proof = os.path.join(tmpdir, "proof.pbp")
    t0 = time.time()
    try:
        r = subprocess.run([satsuma, "fix", "--file", plain, "--out-file", os.path.join(tmpdir, "out.cnf"),
                            "--proof-file", proof, "--veripb"], capture_output=True, text=True, timeout=timeout)
        text = re.sub(r"\x1b\[[0-9;]*m", "", (r.stdout or "") + (r.stderr or ""))
        out["satsuma"] = "UNSAT" if "c UNSATISFIABLE" in text else "SAT" if "c SATISFIABLE" in text else "undecided"
    except subprocess.TimeoutExpired:
        out["satsuma"] = "timeout"
    out["satsuma_s"] = round(time.time() - t0, 1)
    if out["satsuma"] == "UNSAT":
        fixed = os.path.join(tmpdir, "proof.v3.pbp")
        rewrite_proof(proof, fixed)
        t1 = time.time()
        try:
            r = subprocess.run([veripb, "-u", plain, fixed], capture_output=True, text=True, timeout=timeout)
            text = (r.stdout or "") + (r.stderr or "")
            m = re.search(r"s VERIFIED [A-Z]+", text)
            tail = [l.strip() for l in text.splitlines() if l.strip()]
            out["veripb"] = m.group(0)[2:] if m else "REJECTED: " + (tail[-1] if tail else "")[:120]
        except subprocess.TimeoutExpired:
            out["veripb"] = "timeout"
        out["veripb_s"] = round(time.time() - t1, 1)
    for f_ in ("in.cnf", "out.cnf", "proof.pbp", "proof.v3.pbp"):
        try:
            os.remove(os.path.join(tmpdir, f_))
        except OSError:
            pass
    return out


def cmd_verify(args) -> int:
    with open(args.table, newline="") as fh:
        rows = [r for r in csv.DictReader(fh, dialect="excel-tab") if r.get("sym_decided")]
    print(f"{len(rows)} decided cells to check")
    results = []
    with tempfile.TemporaryDirectory(prefix="structure-verify-", dir=args.tmp) as d:
        for n, r in enumerate(rows, 1):
            res = verify_one(r["stem"], args.satsuma, args.veripb, args.timeout, d)
            res["family"] = r["family"]
            res["status"] = r.get("status", "")
            results.append(res)
            print(f"  {n}/{len(rows)} {r['family']} {r['stem'][33:60]}: satsuma {res['satsuma']} ({res['satsuma_s']} s), "
                  f"veripb {res['veripb']} ({res['veripb_s']} s)", flush=True)
    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=["stem", "family", "status", "satsuma", "veripb", "satsuma_s", "veripb_s"],
                           dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        w.writerows(results)
    ok = sum(1 for r in results if r["veripb"] == "VERIFIED UNSATISFIABLE")
    print(f"wrote {args.out}: {ok} of {len(results)} verified by VeriPB; "
          f"{sum(1 for r in results if r['status'] == 'TIMEOUT' and r['veripb'] == 'VERIFIED UNSATISFIABLE')} of them unsolved in the stock pass")
    return 0


def main(argv) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--cells", default=str(CELLS))
    c.add_argument("--stock", help="the converted stock pass (the solver's static census)")
    c.add_argument("--satsuma", help="path to the satsuma binary (symmetry); omit to skip")
    c.add_argument("--sym-from", help="reuse the sym_* columns of this earlier census instead of running Satsuma")
    c.add_argument("--out", required=True)
    c.add_argument("--workers", type=int, default=16)
    c.add_argument("--max-clauses", type=int, default=3_000_000, help="clauses parsed per instance (default 3M)")
    c.add_argument("--xor-max", type=int, default=8)
    c.add_argument("--amo-seeds", type=int, default=200_000)
    c.add_argument("--sym-timeout", type=float, default=120.0)
    c.add_argument("--sym-max-mb", type=float, default=60.0, help="skip symmetry on .xz files over this size")
    c.add_argument("--tmp", default=tempfile.gettempdir())
    c.set_defaults(func=cmd_census)
    r = sub.add_parser("report")
    r.add_argument("table")
    r.set_defaults(func=cmd_report)
    v = sub.add_parser("verify")
    v.add_argument("table")
    v.add_argument("--satsuma", required=True)
    v.add_argument("--veripb", required=True)
    v.add_argument("--out", required=True)
    v.add_argument("--timeout", type=float, default=1800.0, help="seconds per Satsuma run and per VeriPB check")
    v.add_argument("--tmp", default=tempfile.gettempdir())
    v.set_defaults(func=cmd_verify)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
