#!/usr/bin/env python3
"""Offline recomputation of solver 13's observe() vector from a policy log.

Mirrors solver/13-kissat-rs/src/policy_obs.rs entry for entry: the static
block from the footer's `static` object, the global block from the row's
counters, the delta block over the ring of the last 1, 4 and 16 boundary
rows, the timer block from the `tm_*` columns and the pass block from a
replay of the per-pass recency state over the boundary rows. Every value is
computed in float64 and rounded to float32 exactly as the solver does, so
the recomputed vector equals the logged `obs_*` columns to the float.

A fork child's log (plan step A.8) starts at its branch point: its ring and
recency state were inherited from the parent, so recomputing a child needs
the parent's boundary rows first (`--parent` is how the dataset converter
will do it; this module exposes `observe_log(header, rows, footer,
prefix_rows=...)` for that).

    from policy_obs import observe_log
    names, vectors = observe_log(header, rows, footer)   # one list per row

Usage: policy_obs.py --check <log>      # recompute and compare, exit 1 on a mismatch
       policy_obs.py --names            # print the entry names
"""
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from policy_log import read  # noqa: E402

WINDOWS = (1, 4, 16)
RING = 16
TIMERS = ("probe", "eliminate", "reduce", "rephase", "reorder", "mode")
PASSES = ("congruence", "substitute", "backbone", "vivify", "sweep", "transitive",
          "factor", "eliminate", "forward", "reduce", "rephase", "walk")
LUCKY = ("all_true", "all_false", "forward_false", "forward_true", "backward_false",
         "backward_true")
GLUE_BINS = 8
NEVER = (1 << 64) - 1


def lg(x):
    return math.log1p(x) if x > 0.0 else 0.0


def ratio(a, b):
    return a / b if b > 0.0 else 0.0


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def pass_counters(st, work):
    """Per pass (runs, yield, cost), as policy_obs::pass_counters."""
    generic = max(work - st["search_ticks"], 0)
    return [
        (st["closures"], st["congruent"], generic),
        (st["substitutions"], st["substituted"], st["substitute_ticks"]),
        (st["backbone_computations"], st["backbone_units"], st["backbone_ticks"]),
        (st["vivifications"], st["vivified"], st["vivify_ticks"]),
        (st["sweep"], st["sweep_equivalences"] + st["sweep_units"], st["kitten_ticks"]),
        (st["transitive_reductions"], st["transitive_reduced"] + st["transitive_units"],
         st["transitive_ticks"]),
        (st["factorizations"], st["factored"], st["factor_ticks"]),
        (st["eliminations"], st["eliminated"], st["eliminate_resolutions"]),
        (st["forward_subsumptions"], st["forward_subsumed"] + st["forward_strengthened"],
         st["forward_steps"]),
        (st["reductions"], st["clauses_reduced"], generic),
        (st["rephased"], 0, generic),
        (st["walks"], st["walk_improved"], st["walk_steps"]),
    ]


def snapshot(r):
    """The Snapshot of one row (r: dict column -> value)."""
    pc = pass_counters(r, r["work"])
    return {
        "conflicts": r["conflicts"], "decisions": r["decisions"],
        "propagations": r["propagations"], "search_ticks": r["search_ticks"],
        "work": r["work"], "units": r["units"], "restarts": r["restarts"],
        "reductions": r["reductions"], "learned": r["clauses_learned"],
        "active": r["active"], "irredundant": r["clauses_irredundant"],
        "redundant": r["clauses_redundant"], "switched": r["switched"],
        "probing_ticks": r["probing_ticks"],
        "eliminate_resolutions": r["eliminate_resolutions"],
        "pass_runs": [p[0] for p in pc], "pass_yield": [p[1] for p in pc],
        "pass_cost": [p[2] for p in pc],
    }


def static_block(static):
    """The static entries, as (name, value) pairs, from the footer object."""
    g = static.get("groups", {}) if static else {}
    def v(group, name):
        return float(g.get(group, {}).get(name, 0))
    out = [("s_valid", 1.0 if static and static.get("computed") else 0.0)]
    out += [("s_vars", lg(v("shape", "vars"))), ("s_clauses", lg(v("shape", "clauses"))),
            ("s_literals", lg(v("shape", "literals"))),
            ("s_clauses_per_var", lg(v("shape", "clauses_per_var"))),
            ("s_lits_per_clause", lg(v("shape", "lits_per_clause"))),
            ("s_len_max", lg(v("shape", "len_max")))]
    for i, n in enumerate(("len_1", "len_2", "len_3", "len_4_8", "len_9_32", "len_33_up")):
        out.append((f"s_len_hist{i}", v("shape", n)))
    out += [("s_giant_share", v("shape", "giant_share")),
            ("s_occ_mean", lg(v("occurrence", "occ_mean"))),
            ("s_occ_var", lg(v("occurrence", "occ_var"))),
            ("s_occ_max", lg(v("occurrence", "occ_max"))),
            ("s_occ_entropy", v("occurrence", "occ_entropy")),
            ("s_near_singleton_frac", v("occurrence", "near_singleton_frac")),
            ("s_pure_frac", v("occurrence", "pure_frac")),
            ("s_balance_mean", v("occurrence", "balance_mean")),
            ("s_clause_pos_frac_mean", v("occurrence", "clause_pos_frac_mean")),
            ("s_horn_frac", v("occurrence", "horn_frac")),
            ("s_reverse_horn_frac", v("occurrence", "reverse_horn_frac")),
            ("s_binary_frac", v("big", "binary_frac")),
            ("s_big_touched_frac", v("big", "touched_frac")),
            ("s_big_deg_mean", lg(v("big", "deg_mean"))),
            ("s_big_deg_max", lg(v("big", "deg_max"))),
            ("s_big_root_frac", v("big", "root_frac")),
            ("s_big_leaf_frac", v("big", "leaf_frac")),
            ("s_span_mean", v("locality", "span_mean")),
            ("s_span_median", v("locality", "span_median")),
            ("s_span_small_frac", v("locality", "span_small_frac")),
            ("s_consecutive_frac", v("locality", "consecutive_frac")),
            ("s_class_small", v("classify", "small")),
            ("s_class_bigbig", v("classify", "bigbig")),
            ("s_scc_count", lg(v("scc", "count"))),
            ("s_scc_max", lg(v("scc", "max"))),
            ("s_bfs_depth_mean", lg(v("bfs", "depth_mean"))),
            ("s_bfs_reach_mean", lg(v("bfs", "reach_mean"))),
            ("s_bfs_reach_frac", v("bfs", "reach_frac")),
            ("s_bfs_failed_frac", v("bfs", "failed_frac")),
            ("s_gate_output_frac", v("gates", "output_frac")),
            ("s_gates_matched", lg(v("gates", "matched"))),
            ("s_congruent_equivalences", lg(v("gates", "equivalences"))),
            ("s_backbone_units", lg(v("backbone", "units"))),
            ("s_backbone_budget_hit", 1.0 if v("backbone", "budget_hits") > 0.0 else 0.0),
            ("s_sweep_yield", lg(v("sweep", "equivalences") + v("sweep", "units"))),
            ("s_sweep_budget_hit", 1.0 if v("sweep", "budget_hits") > 0.0 else 0.0),
            # kitten.solved counts every kitten call, UNKNOWN ones included.
            ("s_kitten_unknown_frac", ratio(v("kitten", "unknown"), v("kitten", "solved"))),
            ("s_fast_eliminated_per_var", v("fastel", "eliminated_per_var"))]
    for p in LUCKY:
        out.append((f"s_lucky_outcome_{p}", v("lucky", f"outcome_{p}") / 3.0))
        out.append((f"s_lucky_level_{p}", v("lucky", f"level_frac_{p}")))
    out += [("s_pre_vars_over_original", v("preprocess", "vars_over_original")),
            ("s_pre_clauses_over_original", v("preprocess", "clauses_over_original")),
            ("s_pre_probing_ticks", lg(v("preprocess", "probing_ticks"))),
            ("s_pre_kitten_ticks", lg(v("preprocess", "kitten_ticks"))),
            ("s_pre_factor_ticks", lg(v("preprocess", "factor_ticks"))),
            ("s_pre_work", lg(v("preprocess", "work")))]
    return out


def horizon(header, r):
    mode = header["policy"].get("horizon", "none")
    budget = float(header["policy"].get("horizon_budget", 0))
    work = float(r["work"])
    if mode in ("ticks", "limit"):
        return (min(work / max(budget, 1.0), 1.0), True)
    if mode == "wall":
        return (min((r["wall_ns"] * 1e-9) / budget, 1.0), True)
    return (0.0, False)


def build(header, r, static, ring, pushed, recency, k_res):
    """One row's vector as (name, value) pairs in float64 (before the f32 cast)."""
    out = static_block(static)
    now = snapshot(r)
    static_vars = float(static["groups"]["shape"]["vars"]) if static and static.get("computed") else 0.0
    vars0 = static_vars if static_vars > 0.0 else float(max(r["active"], 1))
    conflicts = float(now["conflicts"])
    search_ticks = float(now["search_ticks"])
    work = float(now["work"])
    stable = int(r["stable"])
    tag = "s" if stable else "f"
    e = out.append
    e(("g_conflicts", lg(conflicts)))
    e(("g_decisions", lg(float(now["decisions"]))))
    e(("g_propagations", lg(float(now["propagations"]))))
    e(("g_search_ticks", lg(search_ticks)))
    e(("g_work", lg(work)))
    e(("g_ticks_per_conflict", lg(ratio(search_ticks, conflicts))))
    e(("g_search_frac", ratio(search_ticks, work)))
    e(("g_probing_frac", ratio(float(now["probing_ticks"]), work)))
    e(("g_eliminate_frac", ratio(float(k_res) * float(now["eliminate_resolutions"]), work)))
    e(("g_active_frac", ratio(float(now["active"]), vars0)))
    e(("g_irredundant", lg(float(now["irredundant"]))))
    e(("g_binary", lg(float(r["clauses_binary"]))))
    e(("g_redundant", lg(float(now["redundant"]))))
    e(("g_redundant_ratio", ratio(float(now["redundant"]),
                                  float(now["irredundant"]) + float(r["clauses_binary"]))))
    e(("g_arena_wards", lg(float(r["arena_wards"]))))
    e(("g_units", lg(float(now["units"]))))
    e(("g_trail_frac", ratio(float(r["trail"]), float(max(r["active"], 1)))))
    e(("g_level", lg(float(r["level"]))))
    e(("g_unassigned_frac", ratio(float(r["unassigned"]), float(max(r["active"], 1)))))
    fast = r[f"avg_{tag}_fast_glue"]
    slow = r[f"avg_{tag}_slow_glue"]
    e(("g_avg_fast_glue", fast))
    e(("g_avg_slow_glue", slow))
    e(("g_avg_glue_ratio", ratio(fast, slow)))
    e(("g_avg_level", lg(r[f"avg_{tag}_level"])))
    e(("g_avg_size", lg(r[f"avg_{tag}_size"])))
    e(("g_avg_trail", lg(r[f"avg_{tag}_trail"])))
    e(("g_avg_decision_rate", r[f"avg_{tag}_decision_rate"]))
    e(("g_stable", float(stable)))
    e(("g_ticks_since_switch", lg(float(max(r["search_ticks"] - r["mode_ticks"], 0)))))
    e(("g_switched", lg(float(now["switched"]))))
    e(("g_restarts", lg(float(now["restarts"]))))
    e(("g_reused_trail_frac", ratio(float(r["restarts_reused_trails"]), float(now["restarts"]))))
    e(("g_reductions", lg(float(now["reductions"]))))
    e(("g_learned", lg(float(now["learned"]))))
    learned = float(r["epoch_learned"])
    for i in range(GLUE_BINS):
        e((f"g_epoch_glue{i}", ratio(float(r[f"epoch_glue_bin{i}"]), learned)))
    e(("g_epoch_learned", lg(learned)))
    e(("g_epoch_size_mean", ratio(float(r["epoch_learned_size"]), learned)))
    e(("g_epoch_glue_mean", ratio(float(r["epoch_learned_glue"]), learned)))
    e(("g_elim_bound", lg(float(r["bound_eliminate_max_completed"]))))
    h, hv = horizon(header, r)
    e(("g_horizon", h))
    e(("g_horizon_valid", 1.0 if hv else 0.0))
    # Deltas over the ring.
    for w in WINDOWS:
        valid = pushed >= w
        prev = ring[(pushed - w) % RING] if valid else None
        def d(key):
            return float(max(now[key] - prev[key], 0)) if valid else 0.0
        def signed(key):
            return float(now[key]) - float(prev[key]) if valid else 0.0
        dconf, ddec, dprop = d("conflicts"), d("decisions"), d("propagations")
        dst, dwork = d("search_ticks"), d("work")
        e((f"d{w}_valid", 1.0 if valid else 0.0))
        e((f"d{w}_conflicts", lg(dconf)))
        e((f"d{w}_decisions", lg(ddec)))
        e((f"d{w}_propagations", lg(dprop)))
        e((f"d{w}_search_ticks", lg(dst)))
        e((f"d{w}_work", lg(dwork)))
        e((f"d{w}_conflicts_per_kilotick", ratio(dconf, dst) * 1e3))
        e((f"d{w}_propagations_per_decision", lg(ratio(dprop, ddec))))
        e((f"d{w}_search_frac", ratio(dst, dwork)))
        e((f"d{w}_units", lg(d("units"))))
        e((f"d{w}_restarts", lg(d("restarts"))))
        e((f"d{w}_learned", lg(d("learned"))))
        e((f"d{w}_reductions", lg(d("reductions"))))
        e((f"d{w}_switched", lg(d("switched"))))
        e((f"d{w}_active_frac", ratio(signed("active"), vars0)))
        e((f"d{w}_irredundant_ratio",
           ratio(signed("irredundant"), float(max(prev["irredundant"], 1)) if valid else 0.0)))
        e((f"d{w}_redundant_ratio",
           ratio(signed("redundant"), float(max(prev["redundant"], 1)) if valid else 0.0)))
    # Timers.
    for t in TIMERS:
        if t == "mode" and (r["lim_mode_count"] & 1):
            clock = r["search_ticks"]
        else:
            clock = r["conflicts"]
        last_fire = r[f"tm_{t}_last_fire"]
        delta = r[f"tm_{t}_stock_delta"]
        e((f"t_{t}_log_delta", lg(float(delta))))
        e((f"t_{t}_progress", min(ratio(float(max(clock - last_fire, 0)), float(max(delta, 1))), 4.0)))
        e((f"t_{t}_fires", lg(float(r[f"tm_{t}_fires"]))))
        e((f"t_{t}_would_fire", float(r[f"tm_{t}_stock_would_fire"])))
    # Passes.
    epoch = r["obs_epoch"]
    for i, name in enumerate(PASSES):
        rc = recency[i]
        never = rc["last_epoch"] == NEVER
        since = epoch + 1 if never else max(epoch - rc["last_epoch"], 0)
        e((f"p_{name}_never", 1.0 if never else 0.0))
        e((f"p_{name}_since", lg(float(since))))
        e((f"p_{name}_runs", lg(float(rc["runs"]))))
        e((f"p_{name}_yield", lg(float(rc["last_yield"]))))
        e((f"p_{name}_cost", lg(float(rc["last_cost"]))))
    return out


def update_recency(recency, now, ring, pushed, epoch):
    if pushed == 0:
        for i in range(len(PASSES)):
            if now["pass_runs"][i] > 0:
                recency[i] = {"last_epoch": epoch, "runs": now["pass_runs"][i],
                              "last_yield": now["pass_yield"][i], "last_cost": now["pass_cost"][i]}
        return
    last = ring[(pushed - 1) % RING]
    for i in range(len(PASSES)):
        if now["pass_runs"][i] > last["pass_runs"][i]:
            recency[i] = {"last_epoch": epoch, "runs": now["pass_runs"][i],
                          "last_yield": max(now["pass_yield"][i] - last["pass_yield"][i], 0),
                          "last_cost": max(now["pass_cost"][i] - last["pass_cost"][i], 0)}


def observe_log(header, rows, footer, prefix_rows=None):
    """Recompute the vector of every row. `prefix_rows` are a parent's rows
    (as dicts) replayed first to rebuild an inherited ring and recency
    state for a fork child's log. Returns (names, vectors) with the values
    rounded to float32."""
    cols = header["columns"]
    static = footer.get("static") if footer else None
    k_res = header.get("k_res", header["policy"].get("k_res", 0))
    ring = [None] * RING
    pushed = 0
    recency = [{"last_epoch": NEVER, "runs": 0, "last_yield": 0, "last_cost": 0}
               for _ in PASSES]
    names = None
    vectors = []
    def replay(r, emit):
        nonlocal pushed, names
        now = snapshot(r)
        boundary = bool(r["row_boundary"])
        if boundary:
            update_recency(recency, now, ring, pushed, r["obs_epoch"])
        if emit:
            pairs = build(header, r, static, ring, pushed, recency, k_res)
            if names is None:
                names = [n for n, _ in pairs]
            vectors.append([f32(v) for _, v in pairs])
        if boundary:
            ring[pushed % RING] = now
            pushed += 1
    for r in prefix_rows or []:
        replay(r, False)
    for row in rows:
        replay(dict(zip(cols, row)), True)
    return names, vectors


def check(path, verbose=True):
    header, rows, footer = read(path)
    cols = header["columns"]
    names, vectors = observe_log(header, rows, footer)
    logged_names = [c[len("obs_"):] for c in cols if c.startswith("obs_") and c != "obs_epoch"]
    if names != logged_names:
        print(f"name mismatch: {len(names)} recomputed v {len(logged_names)} logged")
        for a, b in zip(names, logged_names):
            if a != b:
                print(f"  first difference: {a} v {b}")
                break
        return 1
    idx = [cols.index("obs_" + n) for n in names]
    worst = 0.0
    bad = 0
    for k, (row, vec) in enumerate(zip(rows, vectors)):
        for j, n in enumerate(names):
            a = row[idx[j]]
            b = vec[j]
            diff = abs(a - b)
            if diff > worst:
                worst = diff
            if diff > 1e-6 * max(1.0, abs(b)):
                bad += 1
                if bad <= 10:
                    print(f"row {k} {n}: logged {a!r} recomputed {b!r}")
    if verbose:
        print(f"{path}: {len(rows)} rows x {len(names)} entries, max abs diff {worst:.3g}, "
              f"{bad} mismatches")
    return 1 if bad else 0


def main(argv):
    if len(argv) >= 2 and argv[1] == "--names":
        header, rows, footer = read(argv[2]) if len(argv) > 2 else (None, None, None)
        if header is None:
            print("--names needs a log to read the layout from")
            return 2
        names, _ = observe_log(header, rows[:1], footer)
        print("\n".join(names))
        return 0
    if len(argv) >= 3 and argv[1] == "--check":
        return check(argv[2])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
