#!/usr/bin/env python3
"""Deep differential analysis: solver12 vs kissat, full sat-comp-2025 (400 inst, 3600s)."""
import csv, re, sys
from pathlib import Path
from collections import defaultdict

ROOT = Path("/home/bojji/code/SAT-playground")
S12 = ROOT / "log/seedgate-solver12-full3600-2026-07-29-21-07-58/results.tsv"
KIS = ROOT / "log/kissat-full-20260729-210758/results.csv"
MEDIUM = {p.name[:-len(".cnf.xz")] for p in (ROOT / "benchmarks/sat-comp-2025-medium").glob("*.cnf.xz")}
TO = 3600.0
SOLVED = {"SAT", "UNSAT"}

def norm(s):
    s = (s or "").upper()
    if s.startswith("UNSAT"): return "UNSAT"
    if s.startswith("SAT") or s == "SATISFIABLE": return "SAT"
    return "TIMEOUT"

s12 = {}
for r in csv.DictReader(open(S12), delimiter="\t"):
    s12[r["instance"]] = dict(st=norm(r["result"]), t=float(r["time_s"]),
        conf=r["conflicts"], ver=r["verified"], raw=r["result"])
kis = {}
for r in csv.DictReader(open(KIS)):
    kis[r["instance"]] = dict(st=norm(r["result"]), t=float(r["time_s"]),
        ec=r["exit_code"], raw=r["result"])

insts = sorted(s12)
assert set(insts) == set(kis), (len(s12), len(kis))

def family(name):
    n = name.split("-", 1)[1] if re.match(r"^[0-9a-f]{32}-", name) else name
    n = n.replace(".normalised", "").replace(".sanitized", "")
    pats = [
        (r"16_16_.*(booth|wallace|dadda|default|and_).*", "multiplier-miter-16x16"),
        (r"multiplier_1[56]bits__miter.*", "multiplier-miter-16x16"),
        (r"(booth|wallace|dadda)", "multiplier-miter-16x16"),
        (r"Circuit_multiplier.*", "circuit-multiplier"),
        (r"lec_mult.*", "circuit-multiplier"),
        (r"mchess.*", "mchess"),
        (r"rook-.*", "rook"),
        (r"ramsey.*", "ramsey"),
        (r"rphp.*|harder-fphp.*", "php"),
        (r"clqcl.*|cliquecolou?ring.*", "cliquecoloring"),
        (r"tseitin.*", "tseitin"),
        (r"xor_op.*", "xor_op"),
        (r"oddball.*", "oddball"),
        (r"(MV)?RoundRobin.*", "roundrobin"),
        (r"SC25_Timetable.*|TT7F.*", "timetable"),
        (r"ITC2021.*", "itc-timetable"),
        (r"Kakuro.*", "kakuro"),
        (r"lockchart.*", "lockchart"),
        (r"st_\d+.*", "st-kernel"),
        (r"stb_.*|ER_\d.*", "argumentation"),
        (r"pj20\d\d.*", "pj-bmc-giant"),
        (r"(oski|g2-|2018D_VexRiscv|goldcrest|x-epic|nla-digbench|dspam|blaster|itox).*", "hwmcc-bmc"),
        (r"bp4_.*|bp5_.*", "bitvector-bp"),
        (r"BubbleVsPancake.*|.*[Pp]ancake.*", "sorting-networks"),
        (r"VanDerWaerden.*", "vdw"),
        (r"grs-.*", "grs-crypto"),
        (r"bivium.*|fermat.*|mod2c.*|mod4block.*|dislog.*|at-least-two-vmpc.*", "crypto-arith"),
        (r"sqrt-miter.*", "sqrt-miter"),
        (r"HCP-.*", "hamiltonian"),
        (r"fsf-.*", "fsf"),
        (r"reconf.*", "reconf"),
        (r"sudoku.*", "sudoku"),
        (r"battleship.*", "battleship"),
        (r"baseballcover.*|Nb\d+T\d+|mp1-Nb.*", "sports-sched"),
        (r"mp1-.*", "mp1"),
        (r"SAT_dat.*", "channel-routing"),
        (r"(b1[89]|17|18|2|16_2)$|^\d+$", "giant-bmc"),
        (r"(case|s38417|gm24|gto_|GP_|goldb|uniqinv|myciel|SGI_|contest04|connm|cfi|dubois|par32|6g_6color|hhyp|1-ET|div_miter|oisc|stp212|test_v7|valves|ncc_none|fixedbandwidth|frb80|shuffling|rbsat|sted2|vex|18).*", "misc"),
    ]
    for p, f in pats:
        if re.match(p, n, re.I): return f
    return "misc"

fam = {i: family(i) for i in insts}

print("=" * 70)
print("1. HEADLINE")
print("=" * 70)
for tag, d in (("solver12", s12), ("kissat", kis)):
    sv = [i for i in insts if d[i]["st"] in SOLVED]
    sat = sum(1 for i in sv if d[i]["st"] == "SAT")
    par2 = sum(d[i]["t"] if d[i]["st"] in SOLVED else 2 * TO for i in insts)
    print(f"{tag:10s} solved={len(sv):3d}/400  SAT={sat}  UNSAT={len(sv)-sat}  PAR-2={par2:,.0f}")

print()
print("=" * 70)
print("2. TRUNCATION CURVE (same-deal virtual cutoffs)")
print("=" * 70)
print(f"{'cutoff':>7} {'s12':>5} {'kissat':>7} {'delta':>6}")
for cut in (300, 600, 900, 1200, 1800, 2400, 3000, 3600):
    a = sum(1 for i in insts if s12[i]["st"] in SOLVED and s12[i]["t"] <= cut)
    b = sum(1 for i in insts if kis[i]["st"] in SOLVED and kis[i]["t"] <= cut)
    print(f"{cut:>6}s {a:>5} {b:>7} {b-a:>+6}")

print()
print("=" * 70)
print("3. MEDIUM-100 vs NON-MEDIUM-300 SPLIT")
print("=" * 70)
for label, sel in (("medium-100", [i for i in insts if i + ".cnf.xz" in {m + ".cnf.xz" for m in MEDIUM} or i in MEDIUM],),
                   ("non-medium-300", [i for i in insts if i not in MEDIUM])):
    a = sum(1 for i in sel if s12[i]["st"] in SOLVED)
    b = sum(1 for i in sel if kis[i]["st"] in SOLVED)
    print(f"{label:16s} n={len(sel):3d}  s12={a:3d}  kissat={b:3d}  delta={b-a:+d}")

print()
print("=" * 70)
print("4. EXCLUSIVE CELLS BY FAMILY")
print("=" * 70)
konly = [i for i in insts if kis[i]["st"] in SOLVED and s12[i]["st"] not in SOLVED]
sonly = [i for i in insts if s12[i]["st"] in SOLVED and kis[i]["st"] not in SOLVED]
both_to = [i for i in insts if s12[i]["st"] not in SOLVED and kis[i]["st"] not in SOLVED]
def famcount(lst):
    c = defaultdict(list)
    for i in lst: c[fam[i]].append(i)
    return sorted(c.items(), key=lambda kv: -len(kv[1]))
print(f"-- kissat-only ({len(konly)}) by family:")
for f, cells in famcount(konly):
    ts = sorted(kis[i]["t"] for i in cells)
    print(f"  {f:26s} {len(cells):3d}   kissat times: {', '.join(f'{t:.0f}' for t in ts[:8])}")
print(f"-- solver12-only ({len(sonly)}) by family:")
for f, cells in famcount(sonly):
    ts = sorted(s12[i]["t"] for i in cells)
    print(f"  {f:26s} {len(cells):3d}   s12 times: {', '.join(f'{t:.0f}' for t in ts[:8])}")
print(f"-- both-timeout ({len(both_to)}) by family:")
for f, cells in famcount(both_to):
    print(f"  {f:26s} {len(cells):3d}")

print()
print("=" * 70)
print("5. KISSAT-ONLY CELLS BY KISSAT TIME BAND (capability vs tail)")
print("=" * 70)
bands = [(0, 600), (600, 1200), (1200, 1800), (1800, 2400), (2400, 3000), (3000, 3600)]
for lo, hi in bands:
    cells = [i for i in konly if lo < kis[i]["t"] <= hi]
    print(f"  kissat {lo:>4}-{hi:<4}s: {len(cells):3d} cells")
sub600 = sorted((i for i in konly if kis[i]["t"] <= 600), key=lambda i: kis[i]["t"])
print("\n  Sharpest capability gaps (kissat <=600s, s12 dead at 3600s):")
for i in sub600:
    print(f"    {kis[i]['st']:5s} {kis[i]['t']:7.1f}s  [{fam[i]:24s}] {i[33:]}")

print()
print("=" * 70)
print("6. BOTH-SOLVED THROUGHPUT")
print("=" * 70)
bs = [i for i in insts if s12[i]["st"] in SOLVED and kis[i]["st"] in SOLVED]
contra = [i for i in bs if s12[i]["st"] != kis[i]["st"]]
print(f"both-solved={len(bs)}  status-contradictions={len(contra)} {contra}")
faster = sum(1 for i in bs if s12[i]["t"] < kis[i]["t"])
print(f"s12 faster on {faster}/{len(bs)}; total wall s12={sum(s12[i]['t'] for i in bs):,.0f}s kissat={sum(kis[i]['t'] for i in bs):,.0f}s")
ratios = sorted(bs, key=lambda i: (s12[i]["t"] / max(kis[i]["t"], 0.5)), reverse=True)
print("\n  Worst s12/kissat wall ratios (both-solved, s12 >=300s):")
n = 0
for i in ratios:
    if s12[i]["t"] < 300: continue
    r = s12[i]["t"] / max(kis[i]["t"], 0.5)
    print(f"    {r:5.1f}x  s12={s12[i]['t']:7.1f} kis={kis[i]['t']:7.1f}  [{fam[i]:22s}] {i[33:]}")
    n += 1
    if n >= 15: break
print("\n  Best s12/kissat wall ratios (kissat >=300s):")
n = 0
for i in reversed(ratios):
    if kis[i]["t"] < 300: continue
    r = s12[i]["t"] / max(kis[i]["t"], 0.5)
    print(f"    {r:5.2f}x  s12={s12[i]['t']:7.1f} kis={kis[i]['t']:7.1f}  [{fam[i]:22s}] {i[33:]}")
    n += 1
    if n >= 15: break

print()
print("=" * 70)
print("7. MARGIN BANDS (what an extra hour would buy each solver)")
print("=" * 70)
s12_late = [i for i in insts if s12[i]["st"] in SOLVED and s12[i]["t"] > 3000]
kis_late = [i for i in insts if kis[i]["st"] in SOLVED and kis[i]["t"] > 3000]
print(f"s12 solves in 3000-3600s band: {len(s12_late)}; kissat: {len(kis_late)}")
print("s12 band cells:", [i[33:] for i in s12_late])
print("kissat band cells:", [i[33:] for i in kis_late])

print()
print("=" * 70)
print("8. STATUS ODDITIES")
print("=" * 70)
for i in insts:
    if s12[i]["raw"].startswith("UNKNOWN") or kis[i]["raw"] == "UNKNOWN" or (s12[i]["ver"] not in ("ok", "skip")):
        print(f"  {i[33:]:55s} s12={s12[i]['raw']}/{s12[i]['ver']}@{s12[i]['t']:.0f}s  kissat={kis[i]['raw']}(ec={kis[i]['ec']})@{kis[i]['t']:.0f}s")
