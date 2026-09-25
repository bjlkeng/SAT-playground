#!/usr/bin/env python3
"""tools/rl/rank.py — E.1: per-knob rankers on the round-0 fork-sibling labels.

Plan: plan/rl-scheduler-solver13-plan.md §6.1 step 4 (simple models first),
§6.1c (bootstrap ensembles for active branching), §6.2 item 2 (fork-sibling
ranking), §8 (splits). Bead SAT-playground-p9m.10.1 (step E.1).

The data. Each branch point of round 0 is a sibling set: the parent's
continuation (the entry the parent took there: stock in round 0, a
learned parent's choice later, the table's `parent_entry`) and one child
per other entry of one knob, all from one state, each with a terminal outcome at the cell's work
budget (tools/rl_round0_report.py, benchmarks/rl/round0_children.tsv). The
ordered pairs of a set (a solve beats a budget stop; two solves compare on
total work with a 1 % margin) are the labels; the state is the parent's
observation vector at the branch decision (the 251 entries the solver's
net sees, standardized with benchmarks/rl/obs_norm_2025.json), pulled from
the converted pass and cached next to it as dataset/branch_states.npz.

The models, per knob: (a) stock-always, the reference: stock above every
alternative, a coin flip between two alternatives; (b) bias-only, one score
per entry and no state; (c) a per-entry linear scorer on the standardized
state (the solver's kind-1 head) with a pairwise-logistic loss at several
L2 strengths; (d) xgboost rank:pairwise on state + entry. Round 0 holds
training-split cells only, so "validation" here is cell-grouped K-fold
cross-validation inside the training split: no cell is in both a fold's
fit and its test. Reported per knob: held-out pairwise accuracy, and the
realized result of acting greedily with a margin over stock at held-out
states (how many sets the chosen entry beats or loses to the parent).
Selection among configurations is by the same held-out accuracy, so the
best-per-model number is a little optimistic; the bias-only row is the
honest floor a state-based ranker has to clear.

Bootstrap ensembles (§6.1c): for each knob, B rankers of the chosen linear
configuration fitted on cell-level bootstrap resamples; their disagreement
on the score gap of an entry over stock is the exploration signal of later
rounds. Saved as one npz. Feature importance: for the linear ensemble the
mean |weight| per input over entries and members (inputs are standardized,
so weights compare), for xgboost the total gain; the top inputs per knob
and pooled over knobs are written as a table.

    ~/.cache/sat13-rl/venv/bin/python tools/rl/rank.py log/rl-round0-<ts> \\
        [--children benchmarks/rl/round0_children.tsv] [--out benchmarks/rl/round0] [--folds 5] [--boot 10]
        [--knobs probe,reduce,...] [--no-xgb]

Writes <out>_rankers.tsv (per knob and model, the cross-validated numbers),
<out>_importance.tsv (top inputs), and <run>/dataset/rankers_ensemble.npz.
With --export <net.bin> it also refits each knob's chosen linear
configuration on all of its sets and writes the solver's weights file
(kind-1 heads, linear on the standardized input; a knob without a fit
gets a zero head that always scores stock highest), the parent policy of
the next DAgger round (plan §6.2 item 4) at whatever SAT_POLICY_MARGIN the
collection uses.
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import sys
import time
from collections import defaultdict
from pathlib import Path

os.environ.setdefault("OMP_NUM_THREADS", "4")
import numpy as np  # noqa: E402
import pyarrow.parquet as pq  # noqa: E402
import torch  # noqa: E402

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "tools"))
from data import load_norm  # noqa: E402
from rl_split import assert_training_only  # noqa: E402

MENU = {"probe": [0.0, 0.5, 1.0, 2.0, 4.0], "eliminate": [0.0, 0.5, 1.0, 2.0, 4.0], "reduce": [0.0, 0.5, 1.0, 2.0, 4.0],
        "rephase": [0.0, 0.5, 1.0, 2.0, 4.0], "reorder": [0.0, 0.5, 1.0, 2.0, 4.0], "mode": [0.5, 1.0, 2.0],
        "margin": [0.5, 1.0, 2.0], "sweep": [0.0, 0.5, 1.0, 2.0]}
L2_GRID = (1.0, 0.3, 0.1, 0.03, 0.01)
RANKER_COLUMNS = ("knob", "family", "config", "sets", "cells", "pairs", "pair_accuracy", "acted", "better", "worse")
XGB_GRID = ((2, 100), (3, 200), (3, 400))
MIN_SETS = 40


# ---------------------------------------------------------------------------
# data
# ---------------------------------------------------------------------------

def branch_states(run: Path, children: list[dict], names: list[str]) -> dict:
    """The parent's observation at every branch decision of the children
    table, from the converted rows (cached as dataset/branch_states.npz)."""
    cache = run / "dataset" / "branch_states.npz"
    wanted: dict[str, set] = defaultdict(set)
    for r in children:
        wanted[r["stem"]].add(int(r["decision"]))
    stems, decs, X = [], [], []
    # the cache is only good for the conversion it was read from: its
    # signature is the converter's manifest (which records k_res, the
    # pairing and every log's identity) and the run table
    sig_parts = []
    for f in (run / "dataset" / "MANIFEST.json", run / "dataset" / "runs.parquet"):
        st_ = f.stat()
        sig_parts.append(f"{f.name}:{st_.st_size}:{int(st_.st_mtime)}")
    signature = "|".join(sig_parts)
    if cache.is_file():
        z = np.load(cache)
        same_source = "signature" in z and str(z["signature"]) == signature
        if not same_source:
            print("  states: the cache was built from another conversion, rebuilding", flush=True)
        if same_source and list(z["names"]) == names:
            have = {(s, int(d)) for s, d in zip(z["stems"], z["decisions"])}
            missing = {(s, d) for s, ds in wanted.items() for d in ds} - have
            if not missing:
                return {"X": z["X"], "key": {(s, int(d)): i for i, (s, d) in enumerate(zip(z["stems"], z["decisions"]))}}
            # keep what the cache has and pull the rest (a table that grew
            # since the cache was built)
            print(f"  states: cache lacks {len(missing)} of the requested branch points, extending it", flush=True)
            stems, decs, X = list(z["stems"]), [int(d) for d in z["decisions"]], list(z["X"])
            wanted = defaultdict(set)
            for s, d in missing:
                wanted[s].add(d)
    cols = [f"obs_{n}" for n in names]
    t0 = time.time()
    for k, (stem, ds) in enumerate(sorted(wanted.items())):
        t = pq.read_table(run / "dataset" / "rows" / f"{stem}.fork.parquet",
                          columns=cols + ["is_decision", "is_child"]).to_pydict()
        idx = np.nonzero(np.array(t["is_decision"], dtype=bool) & ~np.array(t["is_child"], dtype=bool))[0]
        M = np.stack([np.array(t[c], dtype=np.float32) for c in cols], axis=1)
        for d in sorted(ds):
            if d < len(idx):
                stems.append(stem)
                decs.append(d)
                X.append(M[idx[d]])
        if k % 50 == 0:
            print(f"  states: {k} cells, {time.time() - t0:.0f}s", flush=True)
    X = np.stack(X)
    np.savez_compressed(cache, stems=np.array(stems), decisions=np.array(decs), X=X, names=np.array(names),
                        signature=np.array(signature))
    return {"X": X, "key": {(s, d): i for i, (s, d) in enumerate(zip(stems, decs))}}


def outcome(solved: bool, work: int) -> tuple[int, int]:
    return (1, -work) if solved else (0, 0)


def better(a, b, margin: float) -> bool:
    """a strictly beats b: a solve beats a stop; two solves by total work with the margin."""
    (sa, wa), (sb, wb) = a, b
    if sa != sb:
        return sa > sb
    if not sa:
        return False
    return -wa < (1.0 - margin) * -wb


def build_groups(children: list[dict], knob: str, key: dict, margin: float):
    """Per labeled sibling set of the knob: (state index, items [(entry index, outcome)], ordered pairs, stem)."""
    menu = MENU[knob]
    stock_i = menu.index(1.0)
    sets: dict[tuple[str, int], list[dict]] = defaultdict(list)
    for r in children:
        if r["knob"] == knob and r["labeled"] == "1" and r["label"] != "capped":
            sets[(r["stem"], int(r["decision"]))].append(r)
    groups = []
    for (stem, d), rs in sets.items():
        si = key.get((stem, d))
        if si is None:
            continue
        # the parent's continuation carries the entry the parent took there
        # (the fork mode forks every other entry); a table without the
        # column is round 0, where that entry is stock
        pe = rs[0].get("parent_entry", "")
        parent_i = menu.index(float(pe)) if pe not in ("", None) else stock_i
        items = [(parent_i, outcome(rs[0]["parent_solved"] == "1", int(rs[0]["parent_work"])))]
        for r in rs:
            e = menu.index(float(r["entry"]))
            if e == parent_i:
                raise SystemExit(f"{stem}:{d}: a child took the parent's own entry {pe} (inconsistent table)")
            items.append((e, outcome(r["child_solved"] == "1", int(r["child_work"]))))
        pairs = [(i, j) for i in range(len(items)) for j in range(len(items))
                 if i != j and better(items[i][1], items[j][1], margin)]
        if pairs:
            groups.append((si, items, pairs, stem))
    return groups, stock_i, len(menu)


# ---------------------------------------------------------------------------
# models: each fit returns score(groups) -> (len(groups), E)
# ---------------------------------------------------------------------------

def fit_linear(Z: np.ndarray, tr: list, E: int, stock_i: int, lam: float, iters: int = 400, seed: int = 0):
    torch.manual_seed(seed)
    n_in = Z.shape[1]
    W = torch.zeros(E, n_in, requires_grad=True)
    b = torch.zeros(E, requires_grad=True)
    with torch.no_grad():
        b[stock_i] = 1.0
    opt = torch.optim.Adam([W, b], lr=1e-2)
    Xtr = torch.as_tensor(Z[[g[0] for g in tr]])
    gi, wi, li = [], [], []
    for k, g in enumerate(tr):
        for i, j in g[2]:
            gi.append(k); wi.append(g[1][i][0]); li.append(g[1][j][0])
    gi, wi, li = torch.tensor(gi), torch.tensor(wi), torch.tensor(li)
    for _ in range(iters):
        S = Xtr @ W.T + b
        loss = torch.nn.functional.softplus(-(S[gi, wi] - S[gi, li])).mean() + lam * (W ** 2).sum()
        opt.zero_grad(); loss.backward(); opt.step()
    Wd, bd = W.detach().numpy(), b.detach().numpy()
    return (lambda gs: Z[[g[0] for g in gs]] @ Wd.T + bd), Wd, bd


def fit_bias(Z, tr, E, stock_i):
    return fit_linear(Z, tr, E, stock_i, lam=1e9)


def fit_xgb(Z: np.ndarray, tr: list, E: int, menu: list, depth: int, rounds: int, margin: float, seed: int = 0):
    """xgboost rank:pairwise on exactly the ordered pairs the linear model
    sees: every pair is its own two-item query (winner 1, loser 0), so the
    tie margin is kept and no relevance count adds a pair of its own."""
    import xgboost as xgb

    X, y, qid = [], [], []
    q = 0
    for g in tr:
        for i, j in g[2]:
            X.append(np.concatenate([Z[g[0]], [menu[g[1][i][0]]]]))
            y.append(1)
            X.append(np.concatenate([Z[g[0]], [menu[g[1][j][0]]]]))
            y.append(0)
            qid += [q, q]
            q += 1
    model = xgb.XGBRanker(objective="rank:pairwise", n_estimators=rounds, max_depth=depth, learning_rate=0.05,
                          subsample=0.8, colsample_bytree=0.5, min_child_weight=5, reg_lambda=5.0, n_jobs=4,
                          random_state=seed)
    model.fit(np.array(X, dtype=np.float32), np.array(y), qid=np.array(qid))

    def score(gs):
        out = np.zeros((len(gs), E))
        for k, g in enumerate(gs):
            Xg = np.array([np.concatenate([Z[g[0]], [menu[e]]]) for e in range(E)], dtype=np.float32)
            out[k] = model.predict(Xg)
        return out
    return score, model


def evaluate(groups: list, S: np.ndarray, stock_i: int, margin_act: float, tie_margin: float):
    """Held-out pairwise accuracy and the realized result of acting greedily
    with `margin_act` over stock: (pairs, correct, acted, better, worse).
    Two entries scored equal get half credit, as in the stock-always
    reference, so every model is scored by the same rule."""
    pairs = correct = acted = n_better = n_worse = 0
    for k, g in enumerate(groups):
        sc = S[k]
        for i, j in g[2]:
            pairs += 1
            a, b = sc[g[1][i][0]], sc[g[1][j][0]]
            correct += 1.0 if a > b else (0.5 if a == b else 0.0)
        present = {e: o for e, o in g[1]}
        best = int(np.argmax(sc))
        # the stock-relative count needs stock's own outcome, which a set
        # whose stock member was capped (a learned parent's child) lacks
        if best != stock_i and sc[best] - sc[stock_i] > margin_act and best in present and stock_i in present:
            acted += 1
            n_better += better(present[best], present[stock_i], tie_margin)
            n_worse += better(present[stock_i], present[best], tie_margin)
    return np.array([pairs, correct, acted, n_better, n_worse], dtype=float)


def stock_always(groups: list, stock_i: int):
    pairs = correct = 0.0
    for g in groups:
        for i, j in g[2]:
            pairs += 1
            ei, ej = g[1][i][0], g[1][j][0]
            correct += 1.0 if ei == stock_i else (0.0 if ej == stock_i else 0.5)
    return pairs, correct


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir")
    ap.add_argument("--children", default=str(ROOT / "benchmarks" / "rl" / "round0_children.tsv"))
    ap.add_argument("--out", default=str(ROOT / "benchmarks" / "rl" / "round0"))
    ap.add_argument("--folds", type=int, default=5)
    ap.add_argument("--boot", type=int, default=10, help="bootstrap rankers per knob (0 = none)")
    ap.add_argument("--knobs", default=",".join(MENU))
    ap.add_argument("--margin", type=float, default=0.01, help="work margin for a tie between two solves")
    ap.add_argument("--act-margin", type=float, default=1.0, help="score margin over stock for acting")
    ap.add_argument("--no-xgb", action="store_true")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--export", help="write the chosen linear rankers as a solver weights file (the next round's parent)")
    args = ap.parse_args(argv)
    run = Path(args.run_dir).resolve()
    torch.set_num_threads(4)
    norm = load_norm()
    with open(args.children, newline="") as f:
        children = [r for r in csv.DictReader((ln for ln in f if not ln.startswith("#")), dialect="excel-tab")]
    assert_training_only(sorted({r["stem"] for r in children}))
    st = branch_states(run, children, norm.names)
    Z = ((st["X"].astype(np.float64) - norm.mean) / norm.std).astype(np.float32)
    n_in = Z.shape[1]
    # two random streams: the folds depend on the seed and the knob only,
    # the bootstrap resamples on a second stream, so turning the ensemble
    # or the trees on or off cannot move a fold and with it the selection
    fold_rng = np.random.default_rng([args.seed, 0])
    boot_rng = np.random.default_rng([args.seed, 1])
    rows_out = []
    importance: dict[str, np.ndarray] = {}
    ensemble: dict[str, np.ndarray] = {}
    exported: dict[str, tuple[np.ndarray, np.ndarray]] = {}
    print(f"{run.name}: {len(children)} children, {len(Z)} branch-point states, {n_in} inputs; "
          f"{args.folds}-fold cross-validation grouped by cell inside the training split")
    for knob in args.knobs.split(","):
        groups, stock_i, E = build_groups(children, knob, st["key"], args.margin)
        cells = sorted({g[3] for g in groups})
        if len(groups) < MIN_SETS:
            print(f"\n{knob}: {len(groups)} labeled sets with an ordered pair, below {MIN_SETS}: skipped")
            continue
        knob_rng = np.random.default_rng([args.seed, 0, sum(map(ord, knob))])
        fold_of = {c: int(f) for c, f in zip(cells, knob_rng.permutation(len(cells)) % args.folds)}
        folds = [([g for g in groups if fold_of[g[3]] != k], [g for g in groups if fold_of[g[3]] == k])
                 for k in range(args.folds)]
        n_pairs = sum(len(g[2]) for g in groups)
        print(f"\n{knob}: {len(groups)} sets on {len(cells)} cells, {n_pairs} ordered pairs")
        sa_pairs, sa_correct = stock_always(groups, stock_i)
        results = [("stock-always", "", sa_pairs, sa_correct, 0, 0, 0)]
        configs = [("bias-only", "", lambda tr: fit_bias(Z, tr, E, stock_i)[0])]
        for lam in L2_GRID:
            configs.append(("linear", f"L2={lam}", lambda tr, lam=lam: fit_linear(Z, tr, E, stock_i, lam)[0]))
        if not args.no_xgb:
            for depth, rounds in XGB_GRID:
                configs.append(("xgb", f"depth={depth},rounds={rounds}",
                                lambda tr, d=depth, n=rounds: fit_xgb(Z, tr, E, MENU[knob], d, n, args.margin)[0]))
        for family, cfg, fit in configs:
            tot = np.zeros(5)
            for tr, te in folds:
                tot += evaluate(te, fit(tr)(te), stock_i, args.act_margin, args.margin)
            results.append((family, cfg, tot[0], tot[1], int(tot[2]), int(tot[3]), int(tot[4])))
        best = {}
        for family, cfg, pairs, correct, acted, nb, nw in results:
            acc = correct / pairs
            print(f"   {family:13s} {cfg:22s} held-out pair accuracy {100 * acc:5.1f} %  "
                  f"acting at margin {args.act_margin:g}: {acted:4d} states, better {nb:3d}, worse {nw:3d}")
            rows_out.append({"knob": knob, "family": family, "config": cfg, "sets": len(groups), "cells": len(cells),
                             "pairs": int(pairs), "pair_accuracy": round(acc, 4), "acted": acted, "better": nb,
                             "worse": nw})
            if family in ("linear", "xgb") and (family not in best or acc > best[family][0]):
                best[family] = (acc, cfg)
        # the chosen linear configuration refitted on every set: the next round's parent
        if "linear" in best and args.export:
            lam = float(best["linear"][1].split("=")[1])
            _, W, b = fit_linear(Z, groups, E, stock_i, lam)
            exported[knob] = (W, b)
        # the bootstrap ensemble of the best linear configuration, and importances
        if "linear" in best and args.boot > 0:
            lam = float(best["linear"][1].split("=")[1])
            members_W, members_b = [], []
            for bi in range(args.boot):
                pick = boot_rng.choice(cells, size=len(cells), replace=True)
                tr = [g for c in pick for g in groups if g[3] == c]
                _, W, b = fit_linear(Z, tr, E, stock_i, lam, seed=bi)
                members_W.append(W); members_b.append(b)
            ensemble[f"{knob}_W"] = np.stack(members_W)
            ensemble[f"{knob}_b"] = np.stack(members_b)
            imp = np.abs(np.stack(members_W)).mean(axis=(0, 1))
            importance[f"{knob}:linear"] = imp
            # disagreement across members on the gap of the best alternative over stock, at every state
            gaps = []
            for W, b in zip(members_W, members_b):
                S = Z[[g[0] for g in groups]] @ W.T + b
                gaps.append(np.delete(S, stock_i, axis=1) - S[:, stock_i][:, None])
            gaps = np.stack(gaps)                                   # (B, states, E-1)
            best_gap = gaps.max(axis=2).mean(axis=0)                # mean over members of the best alternative's gap
            disagreement = gaps.std(axis=0).max(axis=1)             # per fixed entry across members, largest entry
            print(f"   bootstrap x{args.boot} (linear {best['linear'][1]}): best alternative's gap over stock, "
                  f"mean {best_gap.mean():+.2f}; member disagreement (largest per-entry std across members, "
                  f"median over states) {np.median(disagreement):.2f}")
        if "xgb" in best and not args.no_xgb:
            d, n = (int(v.split("=")[1]) for v in best["xgb"][1].split(","))
            _, model = fit_xgb(Z, groups, E, MENU[knob], d, n, args.margin)
            gain = model.get_booster().get_score(importance_type="total_gain")
            imp = np.zeros(n_in + 1)
            for k, v in gain.items():
                imp[int(k[1:])] = v
            importance[f"{knob}:xgb"] = imp[:n_in] / max(imp.sum(), 1e-12)
    out = Path(args.out)
    if not rows_out:
        print("nothing fitted (every requested knob below the minimum set count): no table written")
        return 2
    with open(f"{out}_rankers.tsv", "w", newline="") as f:
        f.write(f"# E.1 per-knob rankers on {run.name}: {args.folds}-fold CV grouped by cell (training split only); "
                f"tools/rl/rank.py\n")
        w = csv.DictWriter(f, fieldnames=list(RANKER_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for r in rows_out:
            w.writerow(r)
    with open(f"{out}_importance.tsv", "w", newline="") as f:
        f.write("# E.1 feature importance: linear = mean |weight| over entries and bootstrap members (standardized "
                "inputs); xgb = share of total gain; top 15 per knob and model, plus the pooled linear rank\n")
        f.write("knob\tmodel\trank\tinput\timportance\n")
        for key_, imp in importance.items():
            knob, model = key_.split(":")
            for rank, j in enumerate(np.argsort(-imp)[:15], 1):
                f.write(f"{knob}\t{model}\t{rank}\t{norm.names[j]}\t{imp[j]:.5f}\n")
        pooled = np.zeros(n_in)
        n_lin = 0
        for key_, imp in importance.items():
            if key_.endswith(":linear"):
                pooled += imp / max(imp.sum(), 1e-12)
                n_lin += 1
        if n_lin:
            for rank, j in enumerate(np.argsort(-pooled)[:30], 1):
                f.write(f"all\tlinear-pooled\t{rank}\t{norm.names[j]}\t{pooled[j] / n_lin:.5f}\n")
    if ensemble:
        np.savez_compressed(run / "dataset" / "rankers_ensemble.npz", names=np.array(norm.names),
                            layout_hash=np.array(f"{norm.layout_hash:016x}"), **ensemble)
    if args.export:
        from policy_net import HEAD_MENUS, HEAD_NAMES, write_net

        heads = []
        for k, name in enumerate(HEAD_NAMES):
            n_out, stock = HEAD_MENUS[k]
            if name in exported:
                W, b = exported[name]
                heads.append({"kind": 1, "w": [[float(v) for v in row] for row in W], "b": [float(v) for v in b]})
            else:
                # no fit for this knob: a zero head whose stock entry always wins
                heads.append({"kind": 1, "w": [[0.0] * n_in for _ in range(n_out)],
                              "b": [1000.0 if j == stock else 0.0 for j in range(n_out)]})
        # the trunk is unused by kind-1 heads; the loader wants at least 1 x 1
        write_net(args.export, norm.names, [float(v) for v in norm.mean], [float(v) for v in norm.std],
                  [[0.0] * n_in], [0.0], [[0.0]], [0.0], heads)
        print(f"wrote {args.export}: linear rankers for {sorted(exported)} as kind-1 heads, stock-only heads for the rest")
    print(f"\nwrote {out}_rankers.tsv, {out}_importance.tsv"
          + (f", {run / 'dataset' / 'rankers_ensemble.npz'}" if ensemble else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
