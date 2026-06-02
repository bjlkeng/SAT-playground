#!/usr/bin/env python3
"""Analyze hardsweep: per-instance conflict distributions, solve-rate, dominance, PAR-2 (n=10)."""
import csv, statistics as st
from collections import defaultdict

TSV = "/home/bojji/code/SAT-playground/log/hardsweep-2026-06-02/results.tsv"
TIMEOUT = 600
SOLVED = ("SAT", "UNSAT", "SATISFIABLE", "UNSATISFIABLE")
rows = list(csv.DictReader(open(TSV), delimiter="\t"))
data = defaultdict(list); insts, cfgs = [], []
for r in rows:
    data[(r["config"], r["instance"])].append(r)
    if r["instance"] not in insts: insts.append(r["instance"])
    if r["config"] not in cfgs: cfgs.append(r["config"])

def conflicts(rl): return [int(r["conflicts"]) for r in rl if r["result"] in SOLVED and r["conflicts"] not in ("NA","")]
def solverate(rl): return sum(1 for r in rl if r["result"] in SOLVED), len(rl)
def dominance(b, d):
    if not b or not d: return None
    gt = sum(1 for x in b for y in d if x > y); eq = sum(1 for x in b for y in d if x == y)
    return (gt + 0.5*eq)/(len(b)*len(d))
short = lambda i: i.split("-",1)[1][:26]

for feat in [c for c in cfgs if c != "default"]:
    print(f"\n{'='*96}\n=== {feat} vs default (n=10 seeds, 3 hard instances, 600s) ===")
    print(f"{'instance':28s}{'def_solve':>10}{'feat_solve':>11}{'def_med':>11}{'feat_med':>11}{'ratio':>7}{'P(f>d)':>8}")
    for inst in insts:
        ds, dn = solverate(data[("default",inst)]); fs, fn = solverate(data[(feat,inst)])
        dc, fc = conflicts(data[("default",inst)]), conflicts(data[(feat,inst)])
        dm = st.median(dc) if dc else 0; fm = st.median(fc) if fc else 0
        P = dominance(fc, dc)
        print(f"{short(inst):28s}{f'{ds}/{dn}':>10}{f'{fs}/{fn}':>11}{dm:>11.0f}{fm:>11.0f}"
              f"{(fm/dm if dm else 0):>7.2f}{(P if P is not None else float('nan')):>8.2f}")
    # aggregate PAR-2 per seed across the 3 instances
    def seed_par2(cfg):
        bys = defaultdict(float)
        for inst in insts:
            for r in data[(cfg,inst)]:
                bys[r["seed"]] += float(r["time_s"]) if r["result"] in SOLVED else 2*TIMEOUT
        return [bys[s] for s in sorted(bys)]
    def tot_solve(cfg):
        s = sum(1 for inst in insts for r in data[(cfg,inst)] if r["result"] in SOLVED)
        n = sum(len(data[(cfg,inst)]) for inst in insts); return s, n
    dp, fp = seed_par2("default"), seed_par2(feat)
    ds_, dn_ = tot_solve("default"); fs_, fn_ = tot_solve(feat)
    print(f"  -- aggregate (3 hard instances, per-seed PAR-2) --")
    print(f"  default      solved {ds_}/{dn_}  PAR-2/seed mean={st.mean(dp):.0f} ± {st.pstdev(dp):.0f}  [{min(dp):.0f}-{max(dp):.0f}]")
    print(f"  {feat:12s} solved {fs_}/{fn_}  PAR-2/seed mean={st.mean(fp):.0f} ± {st.pstdev(fp):.0f}  [{min(fp):.0f}-{max(fp):.0f}]")
    print(f"  Δ solved = {fs_-ds_:+d}   Δ mean PAR-2 = {st.mean(fp)-st.mean(dp):+.0f}  (default seed-spread ±{st.pstdev(dp):.0f})")
