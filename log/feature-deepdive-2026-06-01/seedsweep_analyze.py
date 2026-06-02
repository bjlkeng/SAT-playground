#!/usr/bin/env python3
"""Analyze the seed-sweep: per-instance conflict distributions + stochastic dominance + PAR-2."""
import csv, statistics as st
from collections import defaultdict

TSV = "/home/bojji/code/SAT-playground/log/seedsweep-2026-06-01-v2/results.tsv"
TIMEOUT = 300
rows = list(csv.DictReader(open(TSV), delimiter="\t"))

# index: data[(cfg,inst)] = list of (result, time, conflicts)
data = defaultdict(list)
insts, cfgs = [], []
for r in rows:
    data[(r["config"], r["instance"])].append(r)
    if r["instance"] not in insts: insts.append(r["instance"])
    if r["config"] not in cfgs: cfgs.append(r["config"])

def conflicts(rlist):
    out = []
    for r in rlist:
        if r["result"] in ("SAT", "UNSAT", "SATISFIABLE", "UNSATISFIABLE") and r["conflicts"] not in ("NA", ""):
            out.append(int(r["conflicts"]))
    return out

def times(rlist):
    return [float(r["time_s"]) for r in rlist]

def par2(rlist):
    tot = 0.0
    for r in rlist:
        tot += float(r["time_s"]) if r["result"] in ("SAT","UNSAT","SATISFIABLE","UNSATISFIABLE") else 2*TIMEOUT
    return tot/len(rlist) if rlist else 0

def dominance(b, d):
    """P(b_conflicts > d_conflicts) over all seed pairs; 0.5 = identical distributions."""
    if not b or not d: return None
    gt = sum(1 for x in b for y in d if x > y)
    eq = sum(1 for x in b for y in d if x == y)
    return (gt + 0.5*eq) / (len(b)*len(d))

short = lambda i: i.split("-",1)[1][:30]
feat_cfgs = [c for c in cfgs if c != "default"]

for feat in feat_cfgs:
    print(f"\n{'='*92}\n=== {feat} vs default — conflict distribution per instance (n=5 seeds) ===")
    print(f"{'instance':32s}{'def_median':>11}{'feat_median':>12}{'ratio':>7}{'P(f>d)':>8}  default_range / feat_range")
    Ps = []
    for inst in insts:
        dc = conflicts(data[("default", inst)]); fc = conflicts(data[(feat, inst)])
        if not dc or not fc:
            print(f"{short(inst):32s}  (insufficient solved-seed data: def={len(dc)} feat={len(fc)})")
            continue
        dm, fm = st.median(dc), st.median(fc)
        P = dominance(fc, dc); Ps.append((inst,P,fm/dm if dm else 0))
        print(f"{short(inst):32s}{dm:>11.0f}{fm:>12.0f}{(fm/dm if dm else 0):>7.2f}{P:>8.2f}  "
              f"[{min(dc)}-{max(dc)}] / [{min(fc)}-{max(fc)}]")
    # aggregate PAR-2 across all instances, per seed, with error bars
    print(f"\n  -- aggregate (across 13 instances) --")
    # par2 per seed: sum over instances of that seed's row
    def seed_par2(cfg):
        bys = defaultdict(float); cnt=defaultdict(int)
        for inst in insts:
            for r in data[(cfg,inst)]:
                s=r["seed"]; bys[s]+= float(r["time_s"]) if r["result"] in ("SAT","UNSAT","SATISFIABLE","UNSATISFIABLE") else 2*TIMEOUT; cnt[s]+=1
        return [bys[s] for s in sorted(bys)]
    dp, fp = seed_par2("default"), seed_par2(feat)
    print(f"  default PAR-2/seed: mean={st.mean(dp):.0f} ± {st.pstdev(dp):.0f}  range [{min(dp):.0f}-{max(dp):.0f}]")
    print(f"  {feat} PAR-2/seed: mean={st.mean(fp):.0f} ± {st.pstdev(fp):.0f}  range [{min(fp):.0f}-{max(fp):.0f}]")
    print(f"  Δ mean PAR-2 = {st.mean(fp)-st.mean(dp):+.0f}  (default seed-spread ±{st.pstdev(dp):.0f})")
    if Ps:
        meanP = st.mean([p for _,p,_ in Ps])
        worse = sum(1 for _,p,_ in Ps if p>0.5); better=sum(1 for _,p,_ in Ps if p<0.5)
        print(f"  per-instance P(feat>default) mean={meanP:.2f}  | instances worse:{worse} better:{better} of {len(Ps)}")
