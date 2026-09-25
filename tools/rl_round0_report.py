#!/usr/bin/env python3
"""rl_round0_report.py — the round-0 sibling sets: labels, knob effect sizes, flavours.

Plan: plan/rl-scheduler-solver13-plan.md §4 (reward: paired, censored),
§5.3-5.4 (flavours, run mix), §6.1b (knob triage), §6.2 item 2 (fork-
sibling ranking labels; all-censored sets carry no label). Bead
SAT-playground-p9m.8.4 (step C.4); feeds the knob-triage decision
SAT-playground-p9m.14 and the ranking labels of E.1.

From a converted round-0 pass (tools/rl_dataset.py) this writes one row per
fork child, `<out>_children.tsv`, with its sibling set, the parent's own
outcome at the same budget (in round 0 the parent IS the stock
continuation from the branch point: same seed, same trajectory; in later
rounds it runs a learned policy, so the table also records the entry the
parent itself took at the branch decision, `parent_entry`, read from its
`act_taken_*` value at that decision row: the fork mode forks every other
entry of the menu) and the child's label against it, and prints:

  1. Labels. Per cell class, the sibling sets and how many carry a
     terminal ordering (some member solved). A set where the parent and
     every child stop at the budget is all-censored: it stays in the
     table (flag `labeled` 0) for the offline-RL side line but is no
     ranking label. A child or parent that hit the pass's safety wall
     cap (the collector's `anomaly` wall_cap) has no known outcome at the
     budget: such a child is labeled `capped`, a capped parent unlabels
     its whole set, and neither enters any count. A run the collector
     recorded as `cut` without a failure is an honest resource stop (an
     out-of-memory abort under the ulimit): it counts as unsolved, like a
     budget stop (CLAUDE.md "Evaluation"). Every SAT/UNSAT answer of a cell,
     from any flavour, must agree (the collector checks fork siblings and
     the oracle, not the wild runs against the parent): a contradiction
     is a solver bug and stops the report before any table is written.
  2. Knob effect sizes. Per knob and menu entry: how often the child beats
     the parent (solves when the parent does not, or both solve and the
     child's total work is lower by more than --margin), loses, or ties,
     and the median work ratio among both-solved pairs; plus the rescue
     rate on sets whose parent did not solve. Reported separately for
     the timer-due and the uniform branch points (the schedule marks the
     due ones with `*`; a round-1 schedule marks its actively chosen
     points the same way, so there the split reads active v random),
     which is the check that the delay entries act where the schedule
     put them.
  3. Flavours. The segmented and jitter runs of a cell against its fork
     parent (the stock run at the same budget): outcome changes and the
     spread of the work ratio, next to the same numbers for the children
     of one branch point. This is the "advantage variance per run" that
     decides the mix of later rounds (§5.4).

    ~/.cache/sat13-rl/venv/bin/python tools/rl_round0_report.py log/rl-round0-<ts> \\
        --schedule benchmarks/rl/round0_jobs.tsv.schedule.tsv --out benchmarks/rl/round0 [--margin 0.01]
"""
from __future__ import annotations

import argparse
import csv
import math
import sys
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import load_split  # noqa: E402

KNOBS = ("probe", "eliminate", "reduce", "rephase", "reorder", "mode", "margin", "sweep")
# round 0's classes in report order; a later round's classes (round 1 adds
# rescuable and probe_solved) follow in name order
CLASSES = ("fast", "slow", "band_solved", "band_timeout")
CHILD_COLUMNS = ("stem", "family", "cls", "set_id", "decision", "knob", "due", "entry", "parent_entry", "child_run",
                 "child_solved", "child_work", "child_cpu_s", "parent_solved", "parent_work", "budget",
                 "labeled", "label", "work_ratio")
ACT_COLUMN = {"probe": "act_taken_interval_probe", "eliminate": "act_taken_interval_eliminate",
              "reduce": "act_taken_interval_reduce", "rephase": "act_taken_interval_rephase",
              "reorder": "act_taken_interval_reorder", "mode": "act_taken_interval_mode",
              "margin": "act_taken_restart_margin", "sweep": "act_taken_effort_sweep"}
KNOB_COLUMNS = ("knob", "entry", "n", "better", "worse", "tie", "censored", "beat_frac", "lose_frac", "rescued",
                "parent_unsolved", "ratio_median", "ratio_p10", "ratio_p90")


def load_schedule(path: Path) -> tuple[dict[str, str], dict[tuple[str, int], bool]]:
    """cell class per stem, and whether a (stem, decision) branch point was timer-due."""
    cls: dict[str, str] = {}
    due: dict[tuple[str, int], bool] = {}
    with open(path, newline="") as f:
        for r in csv.DictReader(f, dialect="excel-tab"):
            cls[r["stem"]] = r["class"]
            for item in r["knobs"].split():
                d, k = item.split(":")
                due[(r["stem"], int(d))] = k.endswith("*")
    return cls, due


def finite_int(x) -> int:
    """A work or clock reading as an int; a missing reading (None or NaN, a
    log cut before its first row) is 0, which only a capped or resource-
    stopped run carries and no label reads."""
    return int(x) if x is not None and x == x else 0


def label_of(child_solved: bool, child_work: float, parent_solved: bool, parent_work: float, margin: float) -> str:
    """The child against the parent's continuation (plan §4): a solve beats
    a budget stop; two solves compare on total work with a margin; two
    budget stops carry no order (censored)."""
    if child_solved and not parent_solved:
        return "better"
    if parent_solved and not child_solved:
        return "worse"
    if not child_solved:
        return "censored"
    if child_work < (1.0 - margin) * parent_work:
        return "better"
    if child_work > (1.0 + margin) * parent_work:
        return "worse"
    return "tie"


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir")
    ap.add_argument("--schedule", default=str(ROOT / "benchmarks" / "rl" / "round0_jobs.tsv.schedule.tsv"))
    ap.add_argument("--out", default=str(ROOT / "benchmarks" / "rl" / "round0"), help="prefix of the tables written")
    ap.add_argument("--margin", type=float, default=0.01, help="relative work margin for a tie between two solves")
    ap.add_argument("--partial", action="store_true", help="accept a pass that is not DONE (jobs still missing)")
    args = ap.parse_args(argv)
    run = Path(args.run_dir).resolve()
    cls_of, due_of = load_schedule(Path(args.schedule))
    split = load_split()
    # The collector's own records first: a job that failed before writing a
    # readable log has no row in the converted dataset, so the gate is
    # checked on results.tsv against the job table, not on the dataset.
    with open(run / "jobs.tsv", newline="") as f:
        job_keys = {f"{j['stem']}.{j['tag']}" for j in csv.DictReader(f, dialect="excel-tab")}
    with open(run / "results.tsv", newline="") as f:
        results = {f"{x['stem']}.{x['tag']}": x for x in csv.DictReader(f, dialect="excel-tab")}
    failed_jobs = sorted(k for k, x in results.items() if x.get("failed", "0") == "1")
    if failed_jobs:
        print(f"*** {len(failed_jobs)} job(s) flagged by the collector (verify, oracle, sibling agreement, premature "
              f"UNKNOWN or crash), e.g. {failed_jobs[:3]}: a correctness failure, no labels from this pass")
        return 1
    missing_jobs = sorted(job_keys - set(results))
    if missing_jobs or not (run / "DONE").is_file():
        print(f"*** pass not complete: {len(missing_jobs)} job(s) without a record, DONE {'present' if (run / 'DONE').is_file() else 'absent'}")
        if not args.partial:
            return 1
    r = pq.read_table(run / "dataset" / "runs.parquet").to_pydict()
    n = len(r["run_id"])
    by_id = {r["run_id"][i]: i for i in range(n)}
    bad = [i for i in range(n) if r["failed"][i]]
    if bad:
        print(f"*** {len(bad)} run(s) flagged in the dataset: a correctness failure, no labels from this pass")
        return 1
    # Coverage of the conversion: every collected job must have its run in
    # the dataset and every forked child its row, else the tables would be
    # written from a subset (a `convert --jobs ...` pass) and pass for the
    # whole; a child the converter could not read (an empty or truncated
    # log) is a missing sample and counts the same way.
    ds_keys = {r["run_id"][i].split("/")[-1] for i in range(n) if not r["is_child"][i]}
    n_children_ds = Counter(r["parent_run_id"][i].split("/")[-1] for i in range(n) if r["is_child"][i])
    unconverted = sorted(k for k in results if k not in ds_keys)
    unreadable = sum(max(int(x.get("children") or 0) - n_children_ds.get(k, 0), 0)
                     for k, x in results.items() if x.get("flavour") == "fork")
    if unconverted or unreadable:
        print(f"*** conversion incomplete: {len(unconverted)} collected job(s) without a run in the dataset, "
              f"{unreadable} forked child(ren) without a row (e.g. {unconverted[:3]})")
        if not args.partial:
            return 1
    # the collector's anomalies: wall_cap = the safety wall ended the run,
    # no outcome at the budget (never a label); cut = the log ended without
    # a footer and without a failure flag, an out-of-memory stop under the
    # ulimit, an honest unsolved outcome; wall_cap_after_answer = the parent
    # had answered and sealed its log before the cap, a complete run
    anomalies = {r["run_id"][i] for i in range(n) if r["anomaly"][i] == "wall_cap"}
    resource_stops = {r["run_id"][i] for i in range(n) if r["anomaly"][i] == "cut"}
    unknown_kinds = {r["anomaly"][i] for i in range(n) if r["anomaly"][i] not in (None, "", "wall_cap", "cut",
                                                                                   "wall_cap_after_answer")}
    if unknown_kinds:
        print(f"*** unknown anomaly kind(s) {sorted(unknown_kinds)}: refusing to guess their meaning")
        return 1
    shared = [r["stem"][i] for i in range(n) if split.get(r["stem"][i]) != "train"]
    if shared:
        print(f"*** {len(shared)} run(s) of cells outside the training split: refusing")
        return 1
    # every answer of a cell must agree, whatever the flavour (CLAUDE.md
    # "Correctness is absolute"): the collector compares fork siblings and
    # the oracle, so a wild run against its cell's parent is checked here
    answers: dict[str, set] = defaultdict(set)
    for i in range(n):
        if r["solved"][i] and r["run_id"][i] not in anomalies:
            answers[r["stem"][i]].add(str(r["result"][i]).upper()[:3])
    contradictions = {stem: a for stem, a in answers.items() if len(a) > 1}
    if contradictions:
        for stem, a in list(contradictions.items())[:10]:
            print(f"*** CORRECTNESS FAILURE: {stem}: answers {sorted(a)} across its runs")
        print(f"*** {len(contradictions)} cell(s) with contradicting answers: a solver bug, no tables written")
        return 1

    # --- 1. the children table -------------------------------------------
    # the entry the parent itself took at each branch decision (stock in
    # round 0; a learned parent's choice later), from its decision rows
    parent_taken: dict[tuple[str, int, str], float] = {}
    by_parent: dict[str, set] = defaultdict(set)
    for i in range(n):
        if r["is_child"][i]:
            by_parent[r["parent_run_id"][i].split("/")[-1]].add((r["stem"][i], int(r["branch_decision"][i]),
                                                                  r["branch_knob"][i]))
    for key, want in sorted(by_parent.items()):
        cols = ["is_decision", "is_child"] + sorted({ACT_COLUMN[k] for _, _, k in want})
        t = pq.read_table(run / "dataset" / "rows" / f"{key}.parquet", columns=cols).to_pydict()
        idx = np.nonzero(np.array(t["is_decision"], dtype=bool) & ~np.array(t["is_child"], dtype=bool))[0]
        for stem, d, knob in want:
            if d < len(idx):
                parent_taken[(stem, d, knob)] = float(t[ACT_COLUMN[knob]][idx[d]])
    children = []
    sets: dict[tuple[str, int], list[dict]] = defaultdict(list)
    for i in range(n):
        if not r["is_child"][i]:
            continue
        p = by_id[r["parent_run_id"][i]]
        stem = r["stem"][i]
        d = int(r["branch_decision"][i])
        row = {
            "stem": stem, "family": r["family"][i], "cls": cls_of.get(stem, "?"), "set_id": f"{stem}:{d}",
            "decision": d, "knob": r["branch_knob"][i], "due": int(bool(due_of.get((stem, d), False))),
            "entry": r["branch_entry"][i],
            "parent_entry": parent_taken.get((stem, d, r["branch_knob"][i]), ""),
            "child_run": r["run_id"][i].split("/")[-1],
            "child_solved": int(bool(r["solved"][i]) and r["run_id"][i] not in resource_stops),
            "child_work": finite_int(r["work_end"][i]),
            "child_cpu_s": round(finite_int(r["cpu_ns_end"][i]) / 1e9, 1),
            "parent_solved": int(bool(r["solved"][p]) and r["parent_run_id"][i] not in resource_stops),
            "parent_work": finite_int(r["work_end"][p]),
            "budget": int(r["limit_ticks"][i]),
        }
        if r["run_id"][i] in anomalies or r["parent_run_id"][i] in anomalies:
            # a run cut by the safety wall cap has no outcome at the budget
            row["label"] = "capped"
        else:
            row["label"] = label_of(bool(row["child_solved"]), row["child_work"], bool(row["parent_solved"]),
                                    row["parent_work"], args.margin)
        row["work_ratio"] = (round(row["child_work"] / row["parent_work"], 4)
                             if row["child_solved"] and row["parent_solved"] else "")
        children.append(row)
        sets[(stem, d)].append(row)
    for key, rows in sets.items():
        parent_capped = any(x["label"] == "capped" and r["parent_run_id"][by_id[f"{run.name}/{x['child_run']}"]]
                            in anomalies for x in rows)
        labeled = (not parent_capped) and any(x["parent_solved"] or (x["child_solved"] and x["label"] != "capped")
                                              for x in rows)
        for x in rows:
            x["labeled"] = int(labeled and x["label"] != "capped")
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(f"{out}_children.tsv", "w", newline="") as f:
        f.write(f"# round-0 fork children of {run.name}: one row per child, its sibling set and its label against "
                f"the parent's continuation (tie margin {args.margin}); built by tools/rl_round0_report.py\n")
        w = csv.DictWriter(f, fieldnames=list(CHILD_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for x in children:
            w.writerow(x)
    n_capped = sum(1 for x in children if x["label"] == "capped")
    print(f"{run.name}: {len(results)} jobs, {n} runs in the dataset, {len(children)} children in {len(sets)} sibling "
          f"sets; {len(anomalies)} run(s) capped by the safety wall, {len(resource_stops)} resource stop(s) counted as "
          f"unsolved, {unreadable} forked child(ren) absent from the dataset (unreadable log); "
          f"{n_capped} child row(s) labeled capped (never a label)")
    # the effect tables and the flavour comparison see only rows with a known outcome
    children_known = [x for x in children if x["label"] != "capped"]

    # labels per class
    print(f"\n1. sibling sets per cell class (labeled = some member solved; parent-unsolved = only a child solve labels it)")
    print(f"   {'class':13s} {'cells':>5s} {'sets':>5s} {'labeled':>8s} {'all-censored':>12s} {'parent-unsolved':>15s} "
          f"{'children':>8s} {'child solved':>12s}")
    per_cls: dict[str, dict] = defaultdict(lambda: Counter())
    cells_of: dict[str, set] = defaultdict(set)
    for key, rows in sets.items():
        c = rows[0]["cls"]
        cells_of[c].add(key[0])
        per_cls[c]["sets"] += 1
        per_cls[c]["labeled"] += int(any(x["labeled"] for x in rows))
        per_cls[c]["parent_unsolved"] += 0 if rows[0]["parent_solved"] else 1
        per_cls[c]["children"] += len(rows)
        per_cls[c]["child_solved"] += sum(x["child_solved"] for x in rows if x["label"] != "capped")
    for c in list(CLASSES) + sorted(set(per_cls) - set(CLASSES)):
        s = per_cls.get(c)
        if not s:
            continue
        print(f"   {c:13s} {len(cells_of[c]):5d} {s['sets']:5d} {s['labeled']:8d} {s['sets'] - s['labeled']:12d} "
              f"{s['parent_unsolved']:15d} {s['children']:8d} {s['child_solved']:12d}")
    tot = Counter()
    for s in per_cls.values():
        tot.update(s)
    print(f"   {'total':13s} {sum(len(v) for v in cells_of.values()):5d} {tot['sets']:5d} {tot['labeled']:8d} "
          f"{tot['sets'] - tot['labeled']:12d} {tot['parent_unsolved']:15d} {tot['children']:8d} {tot['child_solved']:12d}")

    # --- 2. knob effect sizes ---------------------------------------------
    def effect_table(rows_sel: list[dict], title: str):
        print(f"\n2. {title}")
        print(f"   {'knob':9s} {'entry':>5s} {'n':>5s} {'better':>7s} {'worse':>6s} {'tie':>5s} {'cens':>5s} "
              f"{'beat%':>6s} {'lose%':>6s} {'rescue':>8s} {'W ratio median':>15s} {'p10':>6s} {'p90':>6s}")
        knob_rows: list[dict] = []
        for knob in KNOBS:
            entries = sorted({x["entry"] for x in rows_sel if x["knob"] == knob})
            for entry in entries + ["all"]:
                sel = [x for x in rows_sel if x["knob"] == knob and (entry == "all" or x["entry"] == entry)]
                if not sel:
                    continue
                lab = Counter(x["label"] for x in sel)
                unsolved_parent = [x for x in sel if not x["parent_solved"]]
                rescued = sum(1 for x in unsolved_parent if x["child_solved"])
                ratios = [float(x["work_ratio"]) for x in sel if x["work_ratio"] != ""]
                med = float(np.median(ratios)) if ratios else float("nan")
                p10 = float(np.percentile(ratios, 10)) if ratios else float("nan")
                p90 = float(np.percentile(ratios, 90)) if ratios else float("nan")
                ordered = len(sel) - lab["censored"]
                beat = lab["better"] / ordered if ordered else float("nan")
                lose = lab["worse"] / ordered if ordered else float("nan")
                print(f"   {knob:9s} {str(entry):>5s} {len(sel):5d} {lab['better']:7d} {lab['worse']:6d} {lab['tie']:5d} "
                      f"{lab['censored']:5d} {100 * beat:6.1f} {100 * lose:6.1f} "
                      f"{rescued:3d}/{len(unsolved_parent):<4d} {med:15.3f} {p10:6.3f} {p90:6.3f}")
                knob_rows.append({"knob": knob, "entry": entry, "n": len(sel), "better": lab["better"],
                                  "worse": lab["worse"], "tie": lab["tie"], "censored": lab["censored"],
                                  "beat_frac": round(beat, 4) if ordered else "", "lose_frac": round(lose, 4) if ordered else "",
                                  "rescued": rescued, "parent_unsolved": len(unsolved_parent),
                                  "ratio_median": round(med, 4) if ratios else "", "ratio_p10": round(p10, 4) if ratios else "",
                                  "ratio_p90": round(p90, 4) if ratios else ""})
        return knob_rows
    knob_rows = effect_table(children_known, "knob effect sizes, every branch point (child v the parent's continuation; "
                                       "beat% and lose% over the ordered pairs; rescue = child solves where the parent did not)")
    if not knob_rows:
        print("   (no child with a known outcome: nothing to summarize)")
    with open(f"{out}_knobs.tsv", "w", newline="") as f:
        f.write(f"# round-0 per-knob effect sizes from {run.name} (tools/rl_round0_report.py, tie margin {args.margin})\n")
        w = csv.DictWriter(f, fieldnames=list(KNOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for x in knob_rows:
            w.writerow(x)
    effect_table([x for x in children_known if x["due"]], "timer-due branch points only")
    effect_table([x for x in children_known if not x["due"]], "uniform branch points only")
    # per knob: the share of ordered pairs where the child differs from the parent at all (any label but tie)
    print(f"\n   per knob, over the ordered pairs: better + worse (the entry moved the outcome or the work by more "
          f"than the margin) v tie")
    for knob in KNOBS:
        sel = [x for x in children_known if x["knob"] == knob and x["label"] != "censored"]
        moved = sum(1 for x in sel if x["label"] != "tie")
        print(f"   {knob:9s} moved {moved:5d} of {len(sel):5d} ({100 * moved / max(len(sel), 1):5.1f} %)")

    # --- 3. flavours ------------------------------------------------------
    print(f"\n3. flavours against the cell's fork parent (the stock run at the same budget); the fork children "
          f"are counted per child")
    print(f"   {'flavour':9s} {'runs':>5s} {'outcome differs':>15s} {'solve lost':>10s} {'solve gained':>12s} "
          f"{'both solved':>11s} {'|log W ratio| median':>20s} {'p90':>6s} {'cpu-h':>7s}")
    parent_of_stem = {r["stem"][i]: i for i in range(n) if r["flavour"][i] == "fork" and not r["is_child"][i]}
    for flav in ("random", "jitter", "children"):
        if flav == "children":
            pairs = [(bool(x["child_solved"]), x["child_work"], bool(x["parent_solved"]), x["parent_work"],
                      x["child_cpu_s"]) for x in children_known]
        else:
            pairs = []
            for i in range(n):
                if r["flavour"][i] != flav or r["run_id"][i] in anomalies:
                    continue
                p = parent_of_stem.get(r["stem"][i])
                if p is None or r["run_id"][p] in anomalies:
                    continue
                pairs.append((bool(r["solved"][i]), r["work_end"][i], bool(r["solved"][p]), r["work_end"][p],
                              (r["cpu_ns_end"][i] or 0) / 1e9))
        differs = sum(1 for cs, cw, ps, pw, _ in pairs if cs != ps or (cs and ps and abs(math.log(cw / pw)) > args.margin))
        lost = sum(1 for cs, cw, ps, pw, _ in pairs if ps and not cs)
        gained = sum(1 for cs, cw, ps, pw, _ in pairs if cs and not ps)
        both = [abs(math.log(cw / pw)) for cs, cw, ps, pw, _ in pairs if cs and ps]
        cpu = sum(c for *_, c in pairs) / 3600
        print(f"   {flav:9s} {len(pairs):5d} {differs:15d} {lost:10d} {gained:12d} {len(both):11d} "
              f"{float(np.median(both)) if both else float('nan'):20.3f} "
              f"{float(np.percentile(both, 90)) if both else float('nan'):6.3f} {cpu:7.0f}")
    print(f"\nwrote {out}_children.tsv and {out}_knobs.tsv")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
