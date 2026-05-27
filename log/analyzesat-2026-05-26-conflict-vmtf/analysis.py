#!/usr/bin/env python3
"""
Work × speed decomposition for analyzesat-2026-05-26-conflict-vmtf.

Reads each config's results.csv and stats.jsonl, computes per-(config,instance)
work_ratio, speed_ratio, net (vs A_baseline), classifies dominant cause.
"""
import csv
import json
import sys
from pathlib import Path

ROOT = Path("/home/bojji/code/SAT-playground/log/analyzesat-2026-05-26-conflict-vmtf")
CONFIGS = ["A_baseline", "B_ccmin_off", "C_ccmin_basic", "D_ccmin_inblock",
           "E_otfs_on", "F_resolved", "G_deep_min"]
TIMEOUT = 300


def load_results(cfg):
    csv_path = ROOT / cfg / "results.csv"
    if not csv_path.exists():
        return {}
    rows = {}
    with open(csv_path) as fh:
        for r in csv.DictReader(fh):
            rows[r["instance"]] = r
    return rows


def load_stats(cfg):
    jl_path = ROOT / cfg / "stats.jsonl"
    if not jl_path.exists():
        return {}
    stats = {}
    with open(jl_path) as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            ipath = obj.get("input_path", "")
            inst = Path(ipath).name.replace(".cnf", "").replace(".xz", "")
            stats[inst] = obj
    return stats


def par2(rows):
    total = 0.0
    solved = 0
    to = 0
    unk = 0
    err = 0
    for r in rows.values():
        res = r.get("result", "")
        try:
            t = float(r.get("time_s", 0) or 0)
        except Exception:
            t = 0
        if res in ("SAT", "UNSAT"):
            total += t
            solved += 1
        elif res == "TIMEOUT":
            total += 2 * TIMEOUT
            to += 1
        elif res == "UNKNOWN":
            total += 2 * TIMEOUT
            unk += 1
        else:
            total += 2 * TIMEOUT
            err += 1
    return total, solved, to, unk, err


def main():
    print("=" * 80)
    print("Multi-config ablation: conflict analysis / minimization / OTFS / VMTF")
    print("=" * 80)

    all_results = {cfg: load_results(cfg) for cfg in CONFIGS}
    all_stats = {cfg: load_stats(cfg) for cfg in CONFIGS}

    print("\n## PAR-2 per config\n")
    print(f"{'config':<22}  {'solved':>6}  {'to':>3}  {'unk':>3}  {'err':>3}  {'PAR-2':>8}")
    for cfg in CONFIGS:
        total, solved, to, unk, err = par2(all_results[cfg])
        print(f"{cfg:<22}  {solved:>6}  {to:>3}  {unk:>3}  {err:>3}  {total:>8.1f}")

    # Per-instance comparison
    instances = sorted(all_results["A_baseline"].keys()) if "A_baseline" in all_results else []
    print(f"\n## Per-instance wall time (s)\n")
    header = ["instance".ljust(40)] + [c.replace("_", "")[:10].rjust(10) for c in CONFIGS]
    print("  ".join(header))
    for inst in instances:
        short = inst[:40].ljust(40)
        cells = [short]
        for cfg in CONFIGS:
            r = all_results.get(cfg, {}).get(inst)
            if r and r.get("result") in ("SAT", "UNSAT"):
                cells.append(f"{float(r['time_s']):>10.2f}")
            elif r:
                cells.append(f"{r['result']:>10}")
            else:
                cells.append(f"{'-':>10}")
        print("  ".join(cells))

    # Work × speed decomposition
    print(f"\n## Work × speed decomposition vs A_baseline\n")
    if "A_baseline" not in all_stats or not all_stats["A_baseline"]:
        print("(no A_baseline stats yet)")
        return

    decomp_rows = []
    for cfg in CONFIGS[1:]:
        if not all_stats.get(cfg):
            continue
        for inst in instances:
            a_stat = all_stats["A_baseline"].get(inst)
            c_stat = all_stats[cfg].get(inst)
            a_res = all_results["A_baseline"].get(inst, {})
            c_res = all_results[cfg].get(inst, {})
            if not a_stat or not c_stat or not a_res or not c_res:
                continue
            try:
                a_wall = float(a_res.get("time_s", 0) or 0)
                c_wall = float(c_res.get("time_s", 0) or 0)
                a_conf = a_stat.get("conflicts", 0) or 0
                c_conf = c_stat.get("conflicts", 0) or 0
                a_prop = a_stat.get("propagations", 0) or 0
                c_prop = c_stat.get("propagations", 0) or 0
                a_sec = a_stat.get("search_sec", a_wall) or a_wall
                c_sec = c_stat.get("search_sec", c_wall) or c_wall
                if a_conf == 0 or a_prop == 0 or a_sec == 0 or c_sec == 0:
                    continue
                work_ratio = c_conf / a_conf if a_conf else 0
                a_pps = a_prop / a_sec if a_sec else 0
                c_pps = c_prop / c_sec if c_sec else 0
                speed_ratio = a_pps / c_pps if c_pps else 0
                net = work_ratio * speed_ratio
                actual = c_wall / a_wall if a_wall else 0
                decomp_rows.append((cfg, inst[:36], work_ratio, speed_ratio, net, actual))
            except (TypeError, ValueError):
                continue

    print(f"{'cfg':<22} {'instance':<37} {'work':>6} {'speed':>6} {'net':>6} {'actual':>7}")
    for cfg, inst, w, s, n, a in decomp_rows:
        print(f"{cfg:<22} {inst:<37} {w:>6.2f} {s:>6.2f} {n:>6.2f} {a:>7.2f}")

    # Diff key stats per (cfg, instance)
    print("\n## Conflict/propagation/restart counters (vs A_baseline)\n")
    print(f"{'cfg':<22} {'instance':<37} {'A_conf':>8} {'A_prop':>10} {'C_conf':>8} {'C_prop':>10}")
    for cfg in CONFIGS[1:]:
        if not all_stats.get(cfg):
            continue
        for inst in instances:
            a = all_stats["A_baseline"].get(inst, {})
            c = all_stats[cfg].get(inst, {})
            if not a or not c:
                continue
            print(f"{cfg:<22} {inst[:36]:<37} "
                  f"{a.get('conflicts',0):>8} {a.get('propagations',0):>10} "
                  f"{c.get('conflicts',0):>8} {c.get('propagations',0):>10}")


if __name__ == "__main__":
    main()
