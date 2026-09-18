#!/usr/bin/env python3
"""rl_features.py — critic-tier static features per benchmark cell, offline.

Plan: plan/rl-scheduler-solver13-plan.md §3.5 (critic-only tier), §3.6
"Critic-only, not for the actor". Bead SAT-playground-p9m.7.6 (step B.8).

The actor's static features are computed by the solver itself (step A.7,
solver/13-kissat-rs/src/policy_static.rs) and land in every policy log
footer. This script computes the features the actor must never see: they
are for the critic, per-family reporting, the validation split and
analyses. Two commands, both idempotent.

    python3 tools/rl_features.py families [--out benchmarks/rl/families.tsv]
        One row per cell of sat-comp-2025 and sat-comp-2026 with its family
        label. The label comes from the ordered RULES table below (a regex
        over the instance name, in order; the first match wins); a name no
        rule matches gets the repo's old heuristic (the first alphabetic
        token after the hash prefix, rl_sweep_report.py) and the rule
        column says so. The rules are the metadata: the competition
        manifests carry no family column.

    python3 tools/rl_features.py static [--suite ...] [--jobs N] [--cores a,b]
        [--out benchmarks/rl/static_features.tsv] [--cache log/rl-features-cache]
        Per cell: family; header from the DIMACS `p` line (vars, clauses);
        compressed and uncompressed size and their ratio (xz --list, no
        decompression); a fingerprint of the comment lines before the `p`
        line (generator names leak the family); and two graph estimates on
        a clause sample of the variable-interaction graph (VIG): modularity
        of a label-propagation partition and a treewidth upper bound from
        min-degree elimination. Both are sample-based estimates meant to
        rank cells, not exact values; the sample sizes are recorded. Per
        cell results are cached as JSON so a rerun only computes new cells,
        and the TSV is rewritten from the cache every time.

The SAT/UNSAT label and the stock solve time are joined from the stock
traces by tools/rl_dataset.py (B.6 table), not here: this script needs
only the CNF files.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import lzma
import os
import random
import re
import subprocess
import sys
import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SUITES = ("sat-comp-2025", "sat-comp-2026")
DEFAULT_FAMILIES = ROOT / "benchmarks" / "rl" / "families.tsv"
DEFAULT_STATIC = ROOT / "benchmarks" / "rl" / "static_features.tsv"
DEFAULT_CACHE = ROOT / "log" / "rl-features-cache"

# ---------------------------------------------------------------------------
# Family rules: (regex on the instance NAME (stem without the hash prefix),
# family). Ordered; first match wins. Groups follow the generator or source,
# which is what "family" means for per-family reporting and the split.
# ---------------------------------------------------------------------------
RULES: list[tuple[str, str]] = [
    # arithmetic circuits and miters
    (r"^16_16_", "multiplier-16x16"),
    (r"^(bd|bw|do)[a-z]*_\d+_\d+\.sanitized$", "mult-miter"),
    (r"^multiplier_\d+bits__miter", "mult-miter-bits"),
    (r"^Circuit_multiplier", "circuit-multiplier"),
    (r"^lec_mult_", "lec-mult"),
    (r"^(div_miter|div-mitern|sqrt-mitern|DivS_)", "div-sqrt-miter"),
    (r"^(Carry_Bits_Fast|Wallace_Bits_Fast)", "bits-fast"),
    (r"^gm\d+sp", "gm-sparrc"),
    (r"^circuit_\d+in\d+out", "circuit-gates"),
    (r"^(c7552|s38417|s38584)$", "iscas"),
    (r"^b(14|15|17|18|19_1|20_1|21|22|22_1)$", "itc99"),
    (r"^(BubbleVsPancakeSort|PancakeVsSelectionSort)", "sort-equivalence"),
    (r"^(minandmaxor|smulo|mod4block|uniqinv)", "arith-xor-misc"),
    # hardware model checking
    (r"^(6s\d+|intel\d+).*_Iter\d+$", "hwmcc-iter"),
    (r"^oski15", "oski"),
    (r"^(hwmcc17miters|rpoc_xits)", "xits-miters"),
    (r"^(g2-|T9\d\.2\.)", "g2-t"),
    (r"^bob12", "hwmcc-bob"),
    (r"^(2018D_VexRiscv|veer_axi|x-epic|bv_ILA|nla-digbench)", "hwmc-step-transition"),
    (r"^(4pipe|velev-pipe)", "velev-pipe"),
    (r"^ak128mod", "ak128"),
    # crypto
    (r"^(aes_equiv|post-cbmc-aes|bivium|grain-|hitag2)", "crypto-cipher"),
    (r"^(gus-md5|sha1__|preimage_|asconhash)", "crypto-hash"),
    (r"^(factorize-|toughsat_|fermat-)", "factoring"),
    (r"^sat-bench-trig", "trig"),
    (r"^satcoin", "satcoin"),
    # software / smt
    (r"\.smt2", "smt2"),
    (r"^coreMQTT", "software-verif"),
    (r"^linked_list_swap", "linked-list"),
    (r"^SAT_dat\.", "sat-dat"),
    # argumentation frameworks (.af / .apx, and the st_/stb_/ER_/WS_ generators)
    (r"^crusti_g2io", "crusti-g2io"),
    (r"(\.af(_\d+)?\.sanitized$|\.apx_|^af-synthesis|^stb_\d+|^st_\d+_)", "argumentation"),
    # combinatorics
    (r"^(VanDerWaerden|vdw_|vdwb_)", "vdw"),
    (r"^ramsey_", "ramsey"),
    (r"^GreenTao", "greentao"),
    (r"^EDP3-", "edp"),
    (r"^(sum_of_3_cubes|sum_of_three_cubes)", "sum-of-cubes"),
    (r"^brocard_problem", "brocard"),
    (r"^hantzsche_wendt", "hantzsche-wendt"),
    (r"^(rook-|knight_|korf-|mchess_)", "chess-puzzles"),
    (r"^(rphp5_|rphp_p|harder-fphp|php_sudoku)", "php"),
    (r"^(sudoku-N30|puzzle32)", "sudoku"),
    (r"^Kakuro-", "kakuro"),
    (r"^lightsout_", "lightsout"),
    (r"^battleship-", "battleship"),
    (r"^(1x1x1[34]_1x[23]x[69]_pair_)", "pair-packing"),
    (r"^(mrpp_|maximum_constrained_partition|size_5_5_5|MM-23|summle_|StConn_|two-trees|TT7F|w19-|E00X|ex095|p160_|os_fwalk|lru_|FmlaImplyChain|hhyp_cec|fastlec_cep|marijn-philips|soelberg_unit|anbul-dated|jgiraldezlevy|gto_p60|Ptn-|hid-uns-enc|crn_40|dislog_|goldb-heqc|UR-15|289-unsat|16_2$|001$|ACG-20|atco_enc|SocialGolfers|QuasiGroup|latin_square|fixedbandwidth)", "singletons"),
    (r"^sbdp4_", "sbdp"),
    (r"^count_p", "count"),
    (r"^crafted_n", "crafted-n"),
    (r"^constraints_", "constraints"),
    (r"^em_\d", "em"),
    (r"^chnl-", "chnl"),
    (r"^adv_gc_", "adv-gc"),
    (r"^clqcl_", "clqcl"),
    (r"^(clique_n|cliquecolo)", "clique-coloring"),
    (r"^(6g_6color|color-19|le450_|fpsol2|inithx|mulsol|zeroin|miles1500|myciel)", "dimacs-coloring"),
    (r"^(HCP-|hcp_)", "hcp"),
    (r"^xorshift_", "xorshift"),
    (r"^tseitin_", "tseitin"),
    (r"^xor_op_", "xor-op"),
    (r"^(or_randxor|par32-|mod2c?-rand3bip)", "parity"),
    (r"^(spg_|grs-|sted\d|snw_|newpol|quad_res|SDP_|oddball_|ncc_none|frb\d+|rbsat-|stable-400|REGRandom|contest04-lksat|connm-ue-csp|gensys-ukn|QG7-gensys|linvrinv5|SGI_30|sgp_7-4-6|shuffling-|qwh\.60|x2_64|marg6x6|ais8|dubois50|genurq7|valves-gates|homer\d+|2013113162201nw|544707209399n[cw]|1-(ET|TC)-|j30\d\d_\d_rggt|x9-\d+\.sat|arles_thres|as-p\d+-l\d+|jkkk-one-one|simon-r\d+|DSC125|GP_\d+_\d+_\d+|fsf-300|abw-K-|cabp-|cfi-rigid|DLTM_twitter|mdp-\d+|reconf\d+_|ktf_TF|manthey_single|sembuster|scc_\d+|n3\d\dp5q2|Large-result|Medium-result|ER_\d+_|WS_500)", "__subfamily__"),
    # scheduling / planning / timetabling
    (r"^(SC2[135]_Timetable)", "timetable"),
    (r"^schooltt-", "schooltt"),
    (r"^(exam_|mexam_)", "exam-timetabling"),
    (r"^ITC2021_", "itc2021"),
    (r"^(MVRoundRobin|RoundRobin)", "roundrobin"),
    (r"^Break_", "break-scheduling"),
    (r"^(blocks-blocks|openstacks-|aaai10-planning)", "planning"),
    (r"^SCPC-", "scpc"),
    (r"^lockchart-", "lockchart"),
    (r"^baseballcover", "baseballcover"),
    (r"^ntil-90d", "ntil"),
    (r"^pj20\d\d_k", "pj"),
    (r"^AProVE07", "aprove"),
    (r"^at-least-two-", "at-least-two"),
    (r"^(mp1-|Nb54T6)", "mp1"),
    (r"^oisc-subrv", "oisc"),
    (r"^bp[45]_", "bp"),
    (r"^case\d+", "case"),
    (r"^goldcrest-and", "goldcrest"),
    (r"^lattice-", "lattice"),
    (r"^01-integer-programming", "integer-programming"),
    # anonymised numbered instances, one family per naming style
    (r"^\d+\.normalised$", "anon-normalised"),
    (r"^\d+\.xz\.sanitized$", "anon-xz-sanitized"),
    (r"^\d+\.sanitized$", "anon-sanitized"),
    (r"^\d{9}$", "anon-id"),
    (r"^\d\d-\d{6}$", "anon-pair"),
]

# The `__subfamily__` rule marks generators that are clearly their own
# family; the label is the generator's token from SUBFAMILY below so the
# table stays reviewable in one place.
SUBFAMILY: list[tuple[str, str]] = [
    (r"^spg_", "spg"), (r"^grs-", "grs"), (r"^sted\d", "sted"), (r"^snw_", "snw"),
    (r"^newpol", "newpol"), (r"^quad_res", "quad-res"), (r"^SDP_", "sdp"), (r"^oddball_", "oddball"),
    (r"^ncc_none", "ncc"), (r"^frb\d+", "frb"), (r"^rbsat-", "rbsat"), (r"^stable-400", "stable"),
    (r"^REGRandom", "regrandom"), (r"^contest04-lksat", "lksat"), (r"^connm-ue-csp", "connm-csp"),
    (r"^(gensys-ukn|QG7-gensys)", "gensys"), (r"^linvrinv5", "linvrinv"), (r"^SGI_30", "sgi"),
    (r"^sgp_7-4-6", "sgp"), (r"^shuffling-", "shuffling-sat04"), (r"^qwh\.60", "qwh"),
    (r"^(x2_64|marg6x6|ais8|dubois50|genurq7)", "mis-debugged"), (r"^valves-gates", "valves"),
    (r"^homer\d+", "homer"), (r"^(2013113162201nw|544707209399n[cw])", "sat03-nw"),
    (r"^1-(ET|TC)-", "et-tc"), (r"^j30\d\d_\d_rggt", "rggt"), (r"^x9-\d+\.sat", "x9"),
    (r"^arles_thres", "arles"), (r"^as-p\d+-l\d+", "as-p"), (r"^jkkk-one-one", "jkkk"),
    (r"^simon-r\d+", "simon"), (r"^DSC125", "dsc"), (r"^GP_\d+_\d+_\d+", "gp"), (r"^fsf-300", "fsf"),
    (r"^(abw-K-|cabp-)", "bandwidth-mtx"), (r"^cfi-rigid", "cfi-rigid"), (r"^DLTM_twitter", "dltm"),
    (r"^mdp-\d+", "mdp"), (r"^reconf\d+_", "reconf"), (r"^ktf_TF", "ktf"), (r"^manthey_single", "manthey"),
    (r"^(sembuster|scc_\d+|n3\d\dp5q2|Large-result|Medium-result|ER_\d+_|WS_500)", "argumentation"),
]


def name_of(stem: str) -> str:
    """The instance name without the 32-hex-digit hash prefix."""
    return stem.split("-", 1)[1] if re.match(r"^[0-9a-f]{32}-", stem) else stem


def token_family(name: str) -> str:
    toks = [t for t in re.split(r"[-_0-9.]", name) if t]
    return toks[0].lower() if toks else name.lower()


def family_of(stem: str) -> tuple[str, str]:
    """(family, rule) for a stem: the first RULES match, else the token heuristic."""
    name = name_of(stem)
    for i, (rx, fam) in enumerate(RULES):
        if re.search(rx, name):
            if fam == "__subfamily__":
                for rx2, fam2 in SUBFAMILY:
                    if re.search(rx2, name):
                        return fam2, f"sub:{rx2}"
                return token_family(name), "token(sub)"
            return fam, f"rule{i}:{rx}"
    return token_family(name), "token"


def suite_stems(suite: str) -> list[str]:
    d = ROOT / "benchmarks" / suite
    stems = sorted(p.name[:-len(".cnf.xz")] for p in d.glob("*.cnf.xz"))
    if not stems:
        raise SystemExit(f"no cells in {d}")
    return stems


def cmd_families(args) -> int:
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    stems_of = {suite: suite_stems(suite) for suite in SUITES}
    for suite in SUITES:
        for stem in stems_of[suite]:
            fam, rule = family_of(stem)
            # the same file (same hash prefix) in another suite: the 2026
            # holdout shares 8 cells with 2025, which the split must know
            also = ",".join(s2 for s2 in SUITES if s2 != suite and stem in stems_of[s2])
            rows.append((suite, stem, name_of(stem), fam, rule, also))
    sizes: dict[str, int] = {}
    for r in rows:
        sizes[r[3]] = sizes.get(r[3], 0) + 1
    with open(out, "w", newline="") as f:
        w = csv.writer(f, dialect="excel-tab", lineterminator="\n")
        w.writerow(["suite", "stem", "name", "family", "family_size", "also_in", "rule"])
        for r in rows:
            w.writerow([r[0], r[1], r[2], r[3], sizes[r[3]], r[5], r[4]])
    by_token = sum(1 for r in rows if r[4].startswith("token"))
    shared = sum(1 for r in rows if r[5])
    print(f"{out}: {len(rows)} cells, {len(sizes)} families, {by_token} labelled by the token heuristic, "
          f"{shared} rows whose file is in the other suite too")
    if args.show:
        for fam, n in sorted(sizes.items(), key=lambda kv: (-kv[1], kv[0])):
            names = [r[2] for r in rows if r[3] == fam]
            print(f"{n:4d}  {fam:<24} {', '.join(names[:4])}{' ...' if n > 4 else ''}")
    return 0


# ---------------------------------------------------------------------------
# static features per cell
# ---------------------------------------------------------------------------

def xz_sizes(path: Path) -> tuple[int, int]:
    """(compressed, uncompressed) bytes from xz --robot --list (no decompression)."""
    out = subprocess.run(["xz", "--robot", "--list", str(path)], text=True, stdout=subprocess.PIPE,
                         check=True).stdout
    for ln in out.splitlines():
        f = ln.split("\t")
        if f and f[0] == "totals":
            return int(f[3]), int(f[4])
        if f and f[0] == "file":
            comp, uncomp = int(f[3]), int(f[4])
    return comp, uncomp


def header_and_sample(path: Path, max_clauses: int, max_len: int, seed: int) -> dict:
    """Stream the CNF once: the `p` line, the comment fingerprint, exact
    clause-count and literal-count, and a uniform reservoir sample of clauses
    (each clause clipped to `max_len` literals for the graph)."""
    rng = random.Random(seed)
    comments: list[str] = []
    p_vars = p_clauses = None
    n_clauses = 0
    n_literals = 0
    sample: list[list[int]] = []
    cur: list[int] = []
    with lzma.open(path, "rt", errors="replace") as f:
        for line in f:
            if line.startswith("c"):
                if p_vars is None and len(comments) < 12:
                    comments.append(line.rstrip("\n")[:200])
                continue
            if line.startswith("p"):
                parts = line.split()
                if len(parts) >= 4:
                    p_vars, p_clauses = int(parts[2]), int(parts[3])
                continue
            for tok in line.split():
                if tok == "0":
                    n_clauses += 1
                    n_literals += len(cur)
                    if len(sample) < max_clauses:
                        sample.append(cur[:max_len] if len(cur) > max_len else cur)
                    else:
                        j = rng.randrange(n_clauses)
                        if j < max_clauses:
                            sample[j] = cur[:max_len] if len(cur) > max_len else cur
                    cur = []
                else:
                    cur.append(int(tok))
    fp_text = "\n".join(comments)
    return {
        "p_vars": p_vars, "p_clauses": p_clauses, "clauses": n_clauses, "literals": n_literals,
        "header_lines": len(comments), "header_fingerprint": hashlib.sha1(fp_text.encode()).hexdigest()[:12],
        "header_excerpt": " | ".join(c[:60] for c in comments[:3]),
        "sample": sample, "sample_clauses": len(sample),
    }


def graph_estimates(sample: list[list[int]], tw_nodes: int, lp_rounds: int, seed: int, tw_cap: int = 400) -> dict:
    """Modularity (label propagation) and a treewidth upper bound (min-degree
    elimination) on the VIG of the clause sample. numpy for the partition,
    plain sets for the elimination on a bounded node set."""
    import numpy as np

    us, vs = [], []
    for cl in sample:
        vars_ = sorted({abs(x) for x in cl})
        L = len(vars_)
        if L < 2:
            continue
        for i in range(L):
            for j in range(i + 1, L):
                us.append(vars_[i])
                vs.append(vars_[j])
    if not us:
        return {"vig_nodes": 0, "vig_edges": 0, "modularity": None, "communities": None, "treewidth_ub": None,
                "tw_capped": False}
    u = np.array(us, dtype=np.int64)
    v = np.array(vs, dtype=np.int64)
    nodes, inv = np.unique(np.concatenate([u, v]), return_inverse=True)
    n = len(nodes)
    eu, ev = inv[:len(u)], inv[len(u):]
    key = np.unique(np.minimum(eu, ev) * n + np.maximum(eu, ev))
    eu, ev = key // n, key % n
    m = len(eu)
    deg = np.bincount(np.concatenate([eu, ev]), minlength=n)
    # label propagation, synchronous, with the node id as the tie-break
    both_src = np.concatenate([eu, ev])
    both_dst = np.concatenate([ev, eu])
    order = np.argsort(both_src, kind="stable")
    src_sorted = both_src[order]
    dst_sorted = both_dst[order]
    starts = np.searchsorted(src_sorted, np.arange(n + 1))
    labels = np.arange(n)
    rng = np.random.default_rng(seed)
    for _ in range(lp_rounds):
        nl = labels[dst_sorted]
        # most frequent neighbour label per node: sort (node, label), count runs
        comp = src_sorted * n + nl
        comp_sorted = np.sort(comp)
        vals, counts = np.unique(comp_sorted, return_counts=True)
        node_of = vals // n
        lab_of = vals % n
        # pick, per node, the label with the highest count (random tie-break)
        noise = rng.random(len(vals))
        score = counts + noise * 0.5
        best = np.zeros(n, dtype=np.float64) - 1
        np.maximum.at(best, node_of, score)
        pick = score == best[node_of]
        new = labels.copy()
        new[node_of[pick]] = lab_of[pick]
        if np.array_equal(new, labels):
            break
        labels = new
    # modularity of the partition
    _, comm = np.unique(labels, return_inverse=True)
    k = comm.max() + 1
    inside = np.bincount(comm[eu][comm[eu] == comm[ev]], minlength=k)
    dsum = np.bincount(comm, weights=deg, minlength=k)
    q = float(np.sum(inside / m - (dsum / (2.0 * m)) ** 2))
    # treewidth upper bound: min-degree elimination on the highest-degree subgraph
    keep = np.argsort(-deg, kind="stable")[:tw_nodes]
    keepset = set(int(x) for x in keep)
    adj: dict[int, set[int]] = {x: set() for x in keepset}
    for a, b in zip(eu.tolist(), ev.tolist()):
        if a in keepset and b in keepset:
            adj[a].add(b)
            adj[b].add(a)
    tw = 0
    capped = False
    import heapq
    heap = [(len(adj[x]), x) for x in adj]
    heapq.heapify(heap)
    alive = set(adj)
    while heap:
        d, x = heapq.heappop(heap)
        if x not in alive or d != len(adj[x]):
            continue
        nb = list(adj[x])
        tw = max(tw, len(nb))
        if tw >= tw_cap:
            # the fill-in past this point costs minutes for a number that
            # says only "dense"; the bound is reported as the cap
            tw, capped = tw_cap, True
            break
        for i in range(len(nb)):
            for j in range(i + 1, len(nb)):
                a, b = nb[i], nb[j]
                if b not in adj[a]:
                    adj[a].add(b)
                    adj[b].add(a)
        for y in nb:
            adj[y].discard(x)
            heapq.heappush(heap, (len(adj[y]), y))
        alive.discard(x)
        del adj[x]
    return {"vig_nodes": int(n), "vig_edges": int(m), "modularity": round(q, 4),
            "communities": int(k), "treewidth_ub": int(tw), "tw_capped": capped, "tw_nodes": int(min(tw_nodes, n))}


def params_of(max_clauses: int, max_len: int, tw_nodes: int, lp_rounds: int, seed: int, tw_cap: int) -> dict:
    return {"max_clauses": max_clauses, "max_len": max_len, "tw_nodes": tw_nodes, "lp_rounds": lp_rounds,
            "seed": seed, "tw_cap": tw_cap}


def cached(rec_path: Path, params: dict) -> dict | None:
    """The cached record when it was computed with exactly these parameters, else None."""
    if not rec_path.is_file():
        return None
    try:
        rec = json.loads(rec_path.read_text())
    except json.JSONDecodeError:
        return None
    return rec if rec.get("params") == params and not rec.get("error") else None


def compute_cell(suite: str, stem: str, cache: Path, max_clauses: int, max_len: int, tw_nodes: int,
                 lp_rounds: int, seed: int, tw_cap: int = 400) -> dict:
    rec_path = cache / suite / f"{stem}.json"
    params = params_of(max_clauses, max_len, tw_nodes, lp_rounds, seed, tw_cap)
    hit = cached(rec_path, params)
    if hit is not None:
        return hit
    t0 = time.time()
    path = ROOT / "benchmarks" / suite / f"{stem}.cnf.xz"
    fam, rule = family_of(stem)
    rec = {"suite": suite, "stem": stem, "name": name_of(stem), "family": fam}
    try:
        comp, uncomp = xz_sizes(path)
        rec.update({"compressed_bytes": comp, "uncompressed_bytes": uncomp,
                    "compression_ratio": round(comp / uncomp, 5) if uncomp else None})
        hs = header_and_sample(path, max_clauses, max_len, seed)
        sample = hs.pop("sample")
        rec.update(hs)
        rec.update(graph_estimates(sample, tw_nodes, lp_rounds, seed, tw_cap))
        rec["error"] = ""
    except Exception as e:  # keep the row; the error column says why
        rec["error"] = repr(e)[:200]
    rec["seconds"] = round(time.time() - t0, 2)
    rec["params"] = params
    rec_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = rec_path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(rec, sort_keys=True) + "\n")
    os.replace(tmp, rec_path)
    return rec


STATIC_COLUMNS = ("suite", "stem", "name", "family", "p_vars", "p_clauses", "clauses", "literals",
                  "compressed_bytes", "uncompressed_bytes", "compression_ratio", "header_lines",
                  "header_fingerprint", "header_excerpt", "sample_clauses", "vig_nodes", "vig_edges",
                  "modularity", "communities", "treewidth_ub", "tw_capped", "tw_nodes", "seconds", "error")


def cmd_static(args) -> int:
    cache = Path(args.cache)
    suites = [args.suite] if args.suite else list(SUITES)
    cells = [(s, stem) for s in suites for stem in suite_stems(s)]
    if args.limit:
        cells = cells[:args.limit]
    params = params_of(args.max_clauses, args.max_len, args.tw_nodes, args.lp_rounds, args.seed, args.tw_cap)
    todo = [(s, stem) for s, stem in cells if cached(cache / s / f"{stem}.json", params) is None]
    print(f"{len(cells)} cells, {len(cells) - len(todo)} cached, {len(todo)} to compute with {args.jobs} worker(s)",
          flush=True)
    if args.cores:
        os.sched_setaffinity(0, {int(c) for c in args.cores.split(",")})
    kw = dict(cache=cache, max_clauses=args.max_clauses, max_len=args.max_len, tw_nodes=args.tw_nodes,
              lp_rounds=args.lp_rounds, seed=args.seed, tw_cap=args.tw_cap)
    done = 0
    if todo:
        with ProcessPoolExecutor(max_workers=args.jobs) as ex:
            futs = {ex.submit(compute_cell, s, stem, **kw): (s, stem) for s, stem in todo}
            from concurrent.futures import as_completed
            for fut in as_completed(futs):
                rec = fut.result()
                done += 1
                print(f"  [{done}/{len(todo)}] {rec['stem'][:50]:<50} {rec.get('seconds', 0):7.1f}s "
                      f"Q={rec.get('modularity')} tw={rec.get('treewidth_ub')} {rec.get('error', '')}", flush=True)
    rows = [compute_cell(s, stem, **kw) for s, stem in cells]   # all cached now
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, "w", newline="") as f:
        w = csv.writer(f, dialect="excel-tab", lineterminator="\n")
        w.writerow(STATIC_COLUMNS)
        for r in sorted(rows, key=lambda r: (r["suite"], r["stem"])):
            w.writerow(["" if r.get(c) is None else r.get(c, "") for c in STATIC_COLUMNS])
    errs = [r for r in rows if r.get("error")]
    print(f"{out}: {len(rows)} rows, {len(errs)} with errors")
    for r in errs[:10]:
        print("  error", r["stem"], r["error"])
    return 1 if errs else 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("families")
    f.add_argument("--out", default=str(DEFAULT_FAMILIES))
    f.add_argument("--show", action="store_true", help="print every family with its size and a few names")
    f.set_defaults(fn=cmd_families)
    s = sub.add_parser("static")
    s.add_argument("--suite", default="", help="one suite (default both)")
    s.add_argument("--out", default=str(DEFAULT_STATIC))
    s.add_argument("--cache", default=str(DEFAULT_CACHE))
    s.add_argument("--jobs", type=int, default=2)
    s.add_argument("--cores", default="", help="pin the workers to these cores (comma list)")
    s.add_argument("--limit", type=int, default=0)
    s.add_argument("--max-clauses", type=int, default=200000, help="reservoir sample size for the VIG")
    s.add_argument("--max-len", type=int, default=12, help="literals kept per sampled clause for the VIG")
    s.add_argument("--tw-nodes", type=int, default=4000, help="highest-degree nodes kept for the treewidth bound")
    s.add_argument("--lp-rounds", type=int, default=20)
    s.add_argument("--tw-cap", type=int, default=400, help="stop the elimination once the bound reaches this")
    s.add_argument("--seed", type=int, default=0)
    s.set_defaults(fn=cmd_static)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
