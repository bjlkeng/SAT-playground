#!/usr/bin/env python3
"""rl_split.py — the family-stratified validation split of sat-comp-2025.

Plan: plan/rl-scheduler-solver13-plan.md §8 "Splits". Bead
SAT-playground-p9m.7.11 (step B.11).

Every cell of sat-comp-2025 gets one of three labels in
benchmarks/rl/split_2025.tsv:

    train    fitting (normalization, behaviour cloning, the ranking policy,
             the critic, the runtime predictor)
    val      checkpoint selection only; never fitted on
    shared   the file is also in sat-comp-2026, the true holdout (8 cells,
             benchmarks/rl/families.tsv `also_in`); never fitted on and
             never used for selection, so the holdout stays unseen

The draw is deterministic (--seed, default 20260917) and stratified by
family (benchmarks/rl/families.tsv) and, inside a family, by the status
the paired 400-cell acceptance run gave the cell (SAT / UNSAT / TIMEOUT,
from --oracle), so validation is representative of difficulty as well as
of family. Families with fewer than --pool-below cells are pooled into one
stratum, since a 2-cell family cannot be split 3:1. About one cell in
four goes to validation (--val-fraction).

    python3 tools/rl_split.py make      # writes the split and prints the counts
    python3 tools/rl_split.py check     # the committed file matches a fresh draw

Training code imports this module:

    from rl_split import load_split, training_cells, assert_training_only
    split = load_split()                  # {stem: 'train' | 'val' | 'shared'}
    assert_training_only(stems_used)      # raises if a val/shared cell is fitted on
"""
from __future__ import annotations

import argparse
import csv
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FAMILIES = ROOT / "benchmarks" / "rl" / "families.tsv"
SPLIT = ROOT / "benchmarks" / "rl" / "split_2025.tsv"
DEFAULT_ORACLE = ROOT / "log" / "solver13-full-accept3-20260905-164713" / "results.csv"
SEED = 20260917
SUITE = "sat-comp-2025"


def load_families(path: Path = FAMILIES) -> list[dict]:
    with open(path, newline="") as f:
        return [r for r in csv.DictReader(f, dialect="excel-tab") if r["suite"] == SUITE]


def load_oracle(path: Path) -> dict[str, str]:
    """stem -> SAT | UNSAT | TIMEOUT | UNKNOWN from a run_kissat_full.sh results.csv."""
    out: dict[str, str] = {}
    if not path.is_file():
        raise SystemExit(f"oracle {path} is missing: the draw depends on it, and a draw without it would move cells "
                         f"between train and val under the same seed. `check` reads the statuses from the committed "
                         f"split instead (--oracle committed).")
    with open(path, newline="") as f:
        for r in csv.DictReader(f):
            out[r["instance"]] = r["result"].strip().upper()
    return out


def draw(rows: list[dict], oracle: dict[str, str], seed: int, val_fraction: float, pool_below: int) -> dict[str, str]:
    rng = random.Random(seed)
    strata: dict[str, list[dict]] = defaultdict(list)
    sizes = Counter(r["family"] for r in rows)
    shared = {r["stem"] for r in rows if r.get("also_in")}
    for r in rows:
        if r["stem"] in shared:
            continue
        key = r["family"] if sizes[r["family"]] >= pool_below else "__pool__"
        strata[key].append(r)
    out: dict[str, str] = {s: "shared" for s in shared}
    step = round(1.0 / val_fraction)
    for key in sorted(strata):
        cells = strata[key]
        # order by status (so every status is spread evenly), random inside a status
        cells.sort(key=lambda r: (oracle.get(r["stem"], "?"), rng.random()))
        offset = rng.randrange(step)
        for i, r in enumerate(cells):
            out[r["stem"]] = "val" if (i + offset) % step == 0 else "train"
    return out


def write_split(path: Path, rows: list[dict], split: dict[str, str], oracle: dict[str, str], seed: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", newline="") as f:
        f.write(f"# family-stratified split of {SUITE}; seed {seed}; tools/rl_split.py make\n")
        w = csv.writer(f, dialect="excel-tab", lineterminator="\n")
        w.writerow(["stem", "family", "status", "split"])
        for r in rows:
            w.writerow([r["stem"], r["family"], oracle.get(r["stem"], ""), split[r["stem"]]])


def report(rows: list[dict], split: dict[str, str], oracle: dict[str, str]) -> str:
    lines = []
    n = Counter(split.values())
    lines.append(f"cells {len(rows)}: train {n['train']}, val {n['val']}, shared {n['shared']}")
    st = defaultdict(Counter)
    for r in rows:
        st[split[r["stem"]]][oracle.get(r["stem"], "?")] += 1
    for k in ("train", "val", "shared"):
        tot = sum(st[k].values()) or 1
        lines.append(f"  {k:<6} " + ", ".join(f"{s} {c} ({100 * c / tot:.0f}%)" for s, c in sorted(st[k].items())))
    fam = defaultdict(Counter)
    for r in rows:
        fam[r["family"]][split[r["stem"]]] += 1
    lines.append("  per family (train/val/shared):")
    for f_, c in sorted(fam.items(), key=lambda kv: (-sum(kv[1].values()), kv[0])):
        lines.append(f"    {f_:<24} {c['train']:3d} / {c['val']:3d} / {c['shared']:2d}")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# the contract for training code
# ---------------------------------------------------------------------------

def load_split(path: Path = SPLIT) -> dict[str, str]:
    out: dict[str, str] = {}
    with open(path, newline="") as f:
        lines = [ln for ln in f if not ln.startswith("#")]
    for r in csv.DictReader(lines, dialect="excel-tab"):
        out[r["stem"]] = r["split"]
    if not out:
        raise RuntimeError(f"{path}: empty split")
    return out


def training_cells(path: Path = SPLIT) -> set[str]:
    return {s for s, k in load_split(path).items() if k == "train"}


def validation_cells(path: Path = SPLIT) -> set[str]:
    return {s for s, k in load_split(path).items() if k == "val"}


def assert_training_only(stems, path: Path = SPLIT) -> None:
    """Raise if any of `stems` (cells about to be fitted on) is not a training cell.
    A stem the split does not know (another suite) is refused too: only
    sat-comp-2025 training cells may be fitted on (plan §8)."""
    split = load_split(path)
    bad = sorted({s for s in stems if split.get(s) != "train"})
    if bad:
        kinds = Counter(split.get(s, "unknown") for s in bad)
        raise RuntimeError(f"refusing to fit on {len(bad)} non-training cell(s) ({dict(kinds)}): {bad[:5]} ...")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=("make", "check"))
    ap.add_argument("--seed", type=int, default=SEED)
    ap.add_argument("--val-fraction", type=float, default=0.25)
    ap.add_argument("--pool-below", type=int, default=4, help="families smaller than this share one stratum")
    ap.add_argument("--oracle", default=str(DEFAULT_ORACLE),
                    help="results.csv with the statuses (default the 2026-09-05 acceptance run); `committed` = the statuses in the split file")
    ap.add_argument("--out", default=str(SPLIT))
    args = ap.parse_args(argv)
    rows = load_families()
    if args.oracle == "committed" or (args.cmd == "check" and not Path(args.oracle).is_file()):
        # the committed split records each cell's status: enough to verify the draw on a fresh checkout
        with open(args.out, newline="") as f:
            lines = [ln for ln in f if not ln.startswith("#")]
        oracle = {r["stem"]: r["status"] for r in csv.DictReader(lines, dialect="excel-tab") if r["status"]}
    else:
        oracle = load_oracle(Path(args.oracle))
    split = draw(rows, oracle, args.seed, args.val_fraction, args.pool_below)
    if args.cmd == "make":
        write_split(Path(args.out), rows, split, oracle, args.seed)
        print(f"wrote {args.out}")
        print(report(rows, split, oracle))
        return 0
    committed = load_split(Path(args.out))
    diff = [s for s in split if committed.get(s) != split[s]]
    if diff or set(committed) != set(split):
        print(f"split differs from a fresh draw on {len(diff)} cell(s): {diff[:5]}")
        return 1
    print(f"{args.out}: matches a fresh draw with seed {args.seed}")
    print(report(rows, split, oracle))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
