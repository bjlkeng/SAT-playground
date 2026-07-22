#!/usr/bin/env python3
"""Solver-vs-kissat gap read on the medium suite.

Reads a feature_ablation seedgate results.tsv (solver12) and a kissat
results.csv, normalizes instance names, and prints the lexicographic gap:
solved counts, exclusive cells each way, both-solved conflict/time context,
and PAR-2. Purely a reporting tool — no solving.
"""
from __future__ import annotations
import argparse, csv, sys
from pathlib import Path

SOLVED = {"SAT", "UNSAT"}


def strip(name: str) -> str:
    for s in (".cnf.xz", ".cnf.gz", ".cnf"):
        if name.endswith(s):
            name = name[: -len(s)]
    return name


def norm_status(s: str) -> str:
    s = (s or "").upper()
    if s.startswith("SATISF") or s == "SAT":
        return "SAT"
    if s.startswith("UNSAT"):
        return "UNSAT"
    return "TIMEOUT"  # UNKNOWN/TIMEOUT/other -> unsolved


def read_solver_tsv(p: Path) -> dict:
    out = {}
    with p.open() as f:
        for row in csv.DictReader(f, delimiter="\t"):
            inst = strip(row["instance"])
            out[inst] = {
                "status": norm_status(row["result"]),
                "time": float(row["time_s"]),
                "conflicts": int(row["conflicts"]) if row.get("conflicts", "").strip() not in ("", "NA") else None,
            }
    return out


def read_kissat_csv(p: Path) -> dict:
    out = {}
    with p.open() as f:
        for row in csv.DictReader(f):
            inst = strip(row["instance"])
            out[inst] = {"status": norm_status(row["result"]), "time": float(row["time_s"])}
    return out


def par2(rows: dict, insts: set, timeout: float) -> float:
    tot = 0.0
    for i in insts:
        r = rows.get(i)
        if r and r["status"] in SOLVED:
            tot += r["time"]
        else:
            tot += 2 * timeout
    return tot


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--solver", required=True, help="feature_ablation results.tsv")
    ap.add_argument("--kissat", required=True, help="kissat results.csv")
    ap.add_argument("--timeout", type=float, default=1800.0)
    ap.add_argument("--label-a", default="solver12")
    ap.add_argument("--label-b", default="kissat")
    a = ap.parse_args()

    A = read_solver_tsv(Path(a.solver))
    B = read_kissat_csv(Path(a.kissat))
    insts = sorted(set(A) | set(B))
    la, lb = a.label_a, a.label_b

    a_solved = {i for i in insts if A.get(i, {}).get("status") in SOLVED}
    b_solved = {i for i in insts if B.get(i, {}).get("status") in SOLVED}

    print(f"# Gap read: {la} vs {lb}  (suite={len(insts)} instances, timeout={a.timeout:.0f}s)\n")
    # correctness cross-check: any SAT/UNSAT contradiction on both-solved cells
    contradictions = [i for i in (a_solved & b_solved)
                      if A[i]["status"] != B[i]["status"]]
    print(f"{la:>10}: {len(a_solved)}/{len(insts)} solved   "
          f"(SAT={sum(A[i]['status']=='SAT' for i in a_solved)} "
          f"UNSAT={sum(A[i]['status']=='UNSAT' for i in a_solved)})")
    print(f"{lb:>10}: {len(b_solved)}/{len(insts)} solved   "
          f"(SAT={sum(B[i]['status']=='SAT' for i in b_solved)} "
          f"UNSAT={sum(B[i]['status']=='UNSAT' for i in b_solved)})")
    print(f"{'gap':>10}: {len(b_solved) - len(a_solved):+d}  ({lb} minus {la})\n")

    print(f"PAR-2 {la}: {par2(A, set(insts), a.timeout):.1f}")
    print(f"PAR-2 {lb}: {par2(B, set(insts), a.timeout):.1f}\n")

    if contradictions:
        print("!!! STATUS CONTRADICTIONS (investigate — possible correctness bug):")
        for i in contradictions:
            print(f"    {i}: {la}={A[i]['status']} {lb}={B[i]['status']}")
        print()

    only_b = sorted(b_solved - a_solved)   # kissat solves, solver12 does not
    only_a = sorted(a_solved - b_solved)   # solver12 solves, kissat does not
    print(f"## {lb}-only cells ({len(only_b)}) — the gap to close:")
    for i in only_b:
        print(f"    {B[i]['status']:5} {B[i]['time']:8.1f}s  {i}")
    print(f"\n## {la}-only cells ({len(only_a)}) — where we win:")
    for i in only_a:
        print(f"    {A[i]['status']:5} {A[i]['time']:8.1f}s  {i}")

    both_to = sorted(set(insts) - a_solved - b_solved)
    print(f"\n## both-timeout cells ({len(both_to)}):")
    for i in both_to:
        print(f"    {i}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
