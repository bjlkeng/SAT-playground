#!/usr/bin/env python3
"""rl_early_signal.py — do early progress signals under a held change predict its outcome?

Plan: plan/rl-scheduler-solver13-plan.md §11 (2026-09-30, "check 1" of the
regime-selection idea). A policy that picks a regime at kissat's own mode
switches and keeps what is working on this instance needs within-run
feedback: a few epochs into a regime, the state must already say whether
the regime will pay. This measures that on the hold pilots' children.

For every child of a converted hold pilot (tools/rl_hold_pilot.py), the
state k epochs after the fork is compared with the parent's state at the
same decision index, which is the same work: the deltas of the raw
counters (conflicts, propagations, restarts, level, trail, active
variables, clause counts, units, ...) and of the dynamic part of the
observation vector. Two targets, each with a logistic model under cell-grouped
cross-validation (5 folds, or as many as there are cells) (standardization fitted on the training
fold only). A pass with a collector or dataset correctness failure is
refused, and so is a pass that is not DONE or only partly converted, or
with a cell outside the training split; answers must agree across passes;
runs the safety wall cap stopped are excluded with their children. The
accuracy is pooled over every tested sample, the AUC over the folds
where it is defined:
  A. the child ends worse than its parent (unsolved, or more work by over
     1 %): accuracy against the majority rate, and AUC;
  B. among the siblings of one point, which of two children wins (a solve
     beats a timeout, else the lower work by over 1 %); the feature is the
     difference of the two children's deltas, no intercept: accuracy
     against 50 %, and AUC; also per knob.
Then single signals at k = 8 for reading: the share of children that end
worse among those above and at or below the median delta.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_early_signal.py log/rl-holdpilot2-<ts> [more run dirs]
"""
import glob, math, sys
from collections import defaultdict
from pathlib import Path
import numpy as np, pyarrow.parquet as pq
sys.path.insert(0, str(Path(__file__).resolve().parent))
from rl_split import assert_training_only  # noqa: E402
from sklearn.linear_model import LogisticRegression
from sklearn.model_selection import GroupKFold
from sklearn.metrics import roc_auc_score

runs_dirs = sys.argv[1:]
KS = (2, 4, 8, 16)
LM = math.log(1.01)
RAW = ["conflicts", "propagations", "restarts", "level", "trail", "active", "clauses_redundant",
       "clauses_irredundant", "clauses_binary", "units", "reductions", "rephased", "switched"]

samples = []   # dict(stem, hold, point, entry, k, feat(np), worse, solved, work, knob)
import csv
answers = defaultdict(set)       # stem -> answers, kept across every pass supplied
for rd in runs_dirs:
    # the correctness gates of tools/rl_hold_pilot.py report: a job the collector
    # flagged, or a run flagged in the dataset, ends the analysis; a run the safety
    # wall cap stopped (anomaly wall_cap) has no outcome at the budget and is
    # excluded, with every child of a capped parent
    with open(rd + "/results.tsv", newline="") as f:
        flagged = [x for x in csv.DictReader(f, dialect="excel-tab") if x.get("failed", "0") == "1"]
    if flagged:
        sys.exit(f"*** {len(flagged)} job(s) flagged by the collector in {rd}: a correctness failure, no analysis")
    r = pq.read_table(rd + "/dataset/runs.parquet").to_pydict()
    if any(r["failed"]):
        sys.exit(f"*** {sum(1 for x in r['failed'] if x)} run(s) flagged in the dataset of {rd}: no analysis")
    # completeness, as in tools/rl_hold_pilot.py report: the pass is DONE, every
    # job of its table has a record, every collected job its run in the dataset
    # and every forked child its row, else a `convert --jobs` subset would pass
    # for the whole pass
    with open(rd + "/jobs.tsv", newline="") as f:
        job_keys = {f"{j['stem']}.{j['tag']}" for j in csv.DictReader(f, dialect="excel-tab")}
    with open(rd + "/results.tsv", newline="") as f:
        results = {f"{x['stem']}.{x['tag']}": x for x in csv.DictReader(f, dialect="excel-tab")}
    import os
    missing = sorted(job_keys - set(results))
    if missing or not os.path.isfile(rd + "/DONE"):
        sys.exit(f"*** {rd}: pass not complete ({len(missing)} job(s) without a record, DONE "
                 f"{'present' if os.path.isfile(rd + '/DONE') else 'absent'}): no analysis")
    n_runs = len(r["run_id"])
    ds_keys = {r["run_id"][i].split("/")[-1] for i in range(n_runs) if not r["is_child"][i]}
    from collections import Counter
    n_children_ds = Counter(r["parent_run_id"][i].split("/")[-1] for i in range(n_runs) if r["is_child"][i])
    unconverted = sorted(k for k in results if k not in ds_keys)
    unreadable = sum(max(int(x.get("children") or 0) - n_children_ds.get(k, 0), 0)
                     for k, x in results.items() if x.get("flavour") == "fork")
    if unconverted or unreadable:
        sys.exit(f"*** {rd}: conversion incomplete ({len(unconverted)} collected job(s) without a run, "
                 f"{unreadable} forked child(ren) without a row): convert the whole pass first")
    capped = {r["run_id"][i] for i in range(len(r["run_id"])) if r["anomaly"][i] == "wall_cap"}
    # answers must agree across every run of a cell, across passes too (the
    # collector checks only within a job without an oracle): a contradiction is
    # a correctness failure. And every cell must be a training cell.
    assert_training_only(sorted({r["stem"][i] for i in range(len(r["run_id"]))}))
    for i in range(len(r["run_id"])):
        if r["solved"][i] and r["anomaly"][i] != "wall_cap":
            answers[r["stem"][i]].add(str(r["result"][i]).upper()[:3])
    contradictions = [st for st, a in answers.items() if len(a) > 1]
    if contradictions:
        sys.exit(f"*** CORRECTNESS FAILURE in {rd}: contradicting answers on {contradictions[:5]}: no analysis")
    info = {r["run_id"][i]: (bool(r["solved"][i]) and r["anomaly"][i] != "cut", float(r["work_end"][i]), r["stem"][i], r["tag"][i])
            for i in range(len(r["run_id"]))}
    for f in sorted(glob.glob(rd + "/dataset/rows/*.parquet")):
        t = pq.read_table(f)
        names = t.column_names
        dyn_obs = [c for c in names if c.startswith("obs_") and not c.startswith("obs_s_")]
        feats = RAW + dyn_obs
        cols = t.select(["run_id", "is_child", "branch_decision", "branch_knob", "branch_entry", "policy_decisions", "is_decision", "work"] + feats).to_pydict()
        X = np.array([cols[c] for c in feats], dtype=float).T
        n = len(cols["run_id"])
        parent_rows = []            # (work, row) of the parent's decision rows, in order
        child_rows = defaultdict(dict)
        pids = {cols["run_id"][i] for i in range(n) if not cols["is_child"][i]}
        if len(pids) != 1:
            sys.exit(f"*** {f}: expected one parent run, found {sorted(pids)}")
        pid = pids.pop()
        for i in range(n):
            if not cols["is_decision"][i]:
                continue
            pd_ = int(cols["policy_decisions"][i])
            if cols["is_child"][i]:
                child_rows[cols["run_id"][i]][pd_] = i
            else:
                parent_rows.append((float(cols["work"][i]), i))
        if pid in capped or not parent_rows:
            continue                # a capped parent, or one that never reached a decision
        parent_work = np.array([w for w, _ in parent_rows])
        psolved, pwork, stem, tag = info[pid]
        hold = int(tag[1:])
        for cid, rows in child_rows.items():
            if cid in capped:
                continue
            csolved, cwork, _, _ = info[cid]
            i0 = next(iter(rows.values()))
            bd = int(cols["branch_decision"][i0]); knob = cols["branch_knob"][i0]; entry = float(cols["branch_entry"][i0])
            if csolved and psolved:
                ratio = math.log(cwork / pwork); worse = ratio > LM; better = ratio < -LM
            else:
                worse = not csolved; better = csolved and not psolved
            for k in KS:
                if k > hold:
                    continue
                pd_ = bd + k
                if pd_ in rows:
                    # the parent snapshot at the same WORK (decisions follow search
                    # ticks, so equal decision counts are not equal work): the
                    # parent's decision row nearest in work, within 5 %
                    cw = float(cols["work"][rows[pd_]])
                    j = int(np.argmin(np.abs(parent_work - cw)))
                    if abs(parent_work[j] - cw) > 0.05 * cw:
                        continue
                    delta = X[rows[pd_]] - X[parent_rows[j][1]]
                    samples.append(dict(stem=stem, hold=hold, point=(f, bd, knob), entry=entry, k=k, feat=delta,
                                        worse=worse, better=better, solved=csolved, work=cwork, knob=knob))
print(f"{len(samples)} child-epoch samples from {len({s['stem'] for s in samples})} cells; features {len(feats)} "
      f"({len(RAW)} raw counters + {len(dyn_obs)} dynamic observation entries)")
feat_names = feats

def cv_logistic(Xa, y, groups, fit_intercept=True):
    Xa = np.nan_to_num(Xa)
    acc, auc = [], []
    hits = total = 0
    n_groups = len(set(groups))
    if n_groups < 2:
        return float("nan"), float("nan")       # nothing to hold out
    gkf = GroupKFold(n_splits=min(5, n_groups))
    for tr, te in gkf.split(Xa, y, groups):
        if len(set(y[tr])) < 2:
            continue                     # nothing to fit
        # standardization from the training fold only, applied to both
        mu, sd = Xa[tr].mean(0), Xa[tr].std(0) + 1e-9
        Ztr, Zte = (Xa[tr] - mu) / sd, (Xa[te] - mu) / sd
        m = LogisticRegression(C=0.05, max_iter=2000, fit_intercept=fit_intercept).fit(Ztr, y[tr])
        p = m.predict_proba(Zte)[:, 1]
        hits += int(((p > 0.5) == y[te]).sum()); total += len(te)     # accuracy over every tested sample
        if len(set(y[te])) >= 2:
            auc.append(roc_auc_score(y[te], p))                      # AUC only where it is defined
    return (hits / total if total else float("nan"), np.mean(auc) if auc else float("nan"))

print("\nA. the child ends WORSE than its parent, predicted from the state k epochs after the fork (cell-grouped 5-fold CV)")
print(f"{'hold':>4} {'k':>3} {'n':>5} {'worse%':>7} {'majority%':>9} {'cv acc%':>8} {'auc':>6}")
for hold in sorted({s["hold"] for s in samples}):
    for k in KS:
        S = [s for s in samples if s["hold"] == hold and s["k"] == k]
        if len(S) < 50:
            continue
        Xa = np.array([s["feat"] for s in S]); y = np.array([s["worse"] for s in S], dtype=int); g = [s["stem"] for s in S]
        acc, auc = cv_logistic(Xa, y, g)
        maj = max(y.mean(), 1 - y.mean())
        print(f"{hold:4d} {k:3d} {len(S):5d} {100 * y.mean():7.1f} {100 * maj:9.1f} {100 * acc:8.1f} {auc:6.3f}")

print("\nB. among SIBLINGS at one point, which of two children wins, from the difference of their early deltas (cell-grouped 5-fold CV, no intercept)")
print(f"{'hold':>4} {'k':>3} {'pairs':>6} {'cv acc%':>8} {'auc':>6}")
for hold in sorted({s["hold"] for s in samples}):
    for k in KS:
        S = [s for s in samples if s["hold"] == hold and s["k"] == k]
        by_point = defaultdict(list)
        for s in S:
            by_point[s["point"]].append(s)
        Xp, yp, gp = [], [], []
        for pt, sibs in by_point.items():
            for a in sibs:
                for b in sibs:
                    if a is b:
                        continue
                    # a beats b: solve first, then work by more than the margin
                    if a["solved"] != b["solved"]:
                        win = a["solved"]
                    elif not a["solved"]:
                        continue
                    else:
                        d = math.log(a["work"] / b["work"])
                        if abs(d) <= LM:
                            continue
                        win = d < 0
                    Xp.append(a["feat"] - b["feat"]); yp.append(int(win)); gp.append(a["stem"])
        if len(Xp) < 50:
            continue
        acc, auc = cv_logistic(np.array(Xp), np.array(yp), gp, fit_intercept=False)
        print(f"{hold:4d} {k:3d} {len(Xp):6d} {100 * acc:8.1f} {auc:6.3f}")

print("\nB2. the same per knob, k = 8 and 16")
print(f"{'hold':>4} {'knob':<8} {'k':>3} {'pairs':>6} {'cv acc%':>8} {'auc':>6}")
for hold in sorted({s["hold"] for s in samples}):
    for knob in sorted({s["knob"] for s in samples}):
        for k in (8, 16):
            S = [s for s in samples if s["hold"] == hold and s["k"] == k and s["knob"] == knob]
            by_point = defaultdict(list)
            for s in S:
                by_point[s["point"]].append(s)
            Xp, yp, gp = [], [], []
            for pt, sibs in by_point.items():
                for a in sibs:
                    for b in sibs:
                        if a is b:
                            continue
                        if a["solved"] != b["solved"]:
                            win = a["solved"]
                        elif not a["solved"]:
                            continue
                        else:
                            d = math.log(a["work"] / b["work"])
                            if abs(d) <= LM:
                                continue
                            win = d < 0
                        Xp.append(a["feat"] - b["feat"]); yp.append(int(win)); gp.append(a["stem"])
            if len(Xp) < 40:
                continue
            acc, auc = cv_logistic(np.array(Xp), np.array(yp), gp, fit_intercept=False)
            print(f"{hold:4d} {knob:<8} {k:3d} {len(Xp):6d} {100 * acc:8.1f} {auc:6.3f}")

print("\nC. single signals at k = 8: does 'more X than the parent at equal work' go with ending worse? (share worse among children above v below the median delta)")
for name in ["conflicts", "propagations", "restarts", "level", "trail", "active", "clauses_redundant", "clauses_irredundant", "units"]:
    j = feat_names.index(name)
    S = [s for s in samples if s["k"] == 8]
    v = np.array([s["feat"][j] for s in S]); w = np.array([s["worse"] for s in S])
    med = np.median(v)
    hi, lo = w[v > med], w[v <= med]
    print(f"  {name:<20} above median: {100 * hi.mean():5.1f}% worse (n {len(hi)})   at/below: {100 * lo.mean():5.1f}% worse (n {len(lo)})")
