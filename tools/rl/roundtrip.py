#!/usr/bin/env python3
"""tools/rl/roundtrip.py — check an exported net against the solver's own scores.

Plan: plan/rl-scheduler-solver13-plan.md §6.4, §7 item 8. Bead
SAT-playground-p9m.9.2 (step D.2).

Three evaluations of one weights file on the same observations must agree:

  1. the solver's forward pass, read back from a policy log written with
     `SAT_POLICY=<net> SAT_POLICY_LOG=<log>` (the `net_*` columns of every
     decision row hold the 35 scores the solver computed there);
  2. the pure-Python reference (solver/13-kissat-rs/tools/rl/policy_net.py
     `forward`, float64 in the solver's summation order), which must match
     the solver bit for bit;
  3. the PyTorch model loaded from the file (float64 on the same float32
     weights), which must agree within 1e-6 relative (summation order).

The chosen entries at the log's margin are compared too, per head.

    ~/.cache/sat13-rl/venv/bin/python tools/rl/roundtrip.py <net.bin> <policy.log> [--rows N]

Exit 1 on any mismatch, 2 when the log holds no policy decision (a cell
solved before its first decision: nothing to compare). With no log
(`--self`), only 2 v 3 is checked on
random observations drawn around the normalization, with the entry names
of benchmarks/rl/obs_norm_2025.json (or --norm), whose layout hash must be
the file's.
"""
from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / "solver" / "13-kissat-rs" / "tools" / "rl"))
sys.path.insert(0, str(HERE.parents[1] / "solver" / "13-kissat-rs" / "tools"))
from policy_net import HEAD_NAMES, choose as choose_ref, forward as forward_ref, read_net  # noqa: E402
from policy_log import read as read_log  # noqa: E402


def f32(x: float) -> float:
    return struct.unpack("<f", struct.pack("<f", x))[0]


def torch_scores(path: str, names: list[str], X: np.ndarray) -> np.ndarray:
    import torch
    from model import PolicyNet

    m = PolicyNet.from_file(path, names).double()
    with torch.no_grad():
        parts = m(torch.as_tensor(X, dtype=torch.float64))
    return torch.cat(parts, dim=1).numpy()


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("net")
    ap.add_argument("log", nargs="?")
    ap.add_argument("--rows", type=int, default=64, help="decision rows to check (default 64, spread over the log)")
    ap.add_argument("--self", action="store_true", help="no log: reference v PyTorch on random inputs")
    ap.add_argument("--norm", help="normalization file naming the layout for --self (default benchmarks/rl/obs_norm_2025.json)")
    args = ap.parse_args(argv)
    net = read_net(args.net)
    names: list[str]
    if args.log:
        header, rows, footer = read_log(args.log)
        cols = header["columns"]
        obs_cols = [i for i, c in enumerate(cols) if c.startswith("obs_") and c != "obs_epoch"]
        names = [cols[i][len("obs_"):] for i in obs_cols]
        if header.get("net", {}).get("layout_hash") and int(header["net"]["layout_hash"], 16) != net["layout_hash"]:
            print(f"layout hash of the log's net {header['net']['layout_hash']} differs from {args.net}")
            return 1
        net_cols = [i for i, c in enumerate(cols) if c.startswith("net_") and c != "net_deviations"]
        if len(net_cols) != sum(h["n_out"] for h in net["heads"]):
            print(f"log has {len(net_cols)} score columns, net has {sum(h['n_out'] for h in net['heads'])}")
            return 1
        dec_col = cols.index("policy_decisions") if "policy_decisions" in cols else None
        def val(r, i):
            return float(r[i])      # policy_log.read already decodes the f64 columns
        # A row is written at a boundary and THEN the decision is made from
        # that row's observation, so the decision count rises in the next
        # row, and a row's `net_*` columns hold the scores of the LAST
        # decision: the scores of the decision at row j are in row j + 1.
        if dec_col is None:
            print("log has no policy_decisions column")
            return 1
        decs = [val(r, dec_col) for r in rows]
        pick = [j for j in range(len(rows) - 1) if decs[j + 1] > decs[j]]
        if not pick:
            print(f"{args.log}: {len(rows)} rows and no policy decision (solved before the first one): "
                  f"nothing to compare")
            return 2
        if len(pick) > args.rows:
            n = max(args.rows, 1)
            pick = [pick[int(i * (len(pick) - 1) / max(n - 1, 1))] for i in range(n)]
        margin = float(header.get("net", {}).get("margin", 1.0))
        X = np.array([[f32(val(rows[j], i)) for i in obs_cols] for j in pick], dtype=np.float64)
        logged = np.array([[val(rows[j + 1], i) for i in net_cols] for j in pick], dtype=np.float64)
        n_scored = int((np.abs(logged).sum(axis=1) > 0).sum())
        print(f"{args.log}: {len(rows)} rows, {len(pick)} decision rows checked ({n_scored} with logged scores), "
              f"margin {margin}")
    else:
        from data import NORM, load_norm

        norm = load_norm(Path(args.norm) if args.norm else NORM)
        names = norm.names
        if norm.layout_hash != net["layout_hash"]:
            print(f"{args.net}: layout hash {net['layout_hash']:016x} is not the normalization file's "
                  f"{norm.layout_hash:016x}; pass --norm with the layout the net was trained on")
            return 1
        rng = np.random.default_rng(1)
        X = (np.asarray(net["mean"]) + rng.standard_normal((args.rows, net["n_in"])) * np.asarray(net["std"]))
        X = np.vectorize(f32)(X)
        logged = None
        margin = 1.0
    ref = np.array([forward_ref(net, list(x)) for x in X], dtype=np.float64)
    tor = torch_scores(args.net, names, X)
    rel = np.abs(ref - tor) / np.maximum(np.abs(ref), 1e-30)
    print(f"reference v PyTorch: max relative difference {rel.max():.2e} over {ref.size} scores")
    ok = rel.max() <= 1e-6
    if logged is not None:
        exact = np.array_equal(ref, logged)
        diff = np.abs(ref - logged).max()
        print(f"reference v solver log: {'bit-exact' if exact else f'max absolute difference {diff:.3e}'}")
        ok = ok and exact
        # the chosen entries at the log's margin
        ch_ref = np.array([choose_ref(net, list(s), margin) for s in ref])
        ch_log = np.array([choose_ref(net, list(s), margin) for s in logged])
        dev = int(((ch_ref != np.array([h["stock"] for h in net["heads"]])).sum(axis=1) > 0).sum())
        print(f"chosen entries agree on all {len(pick)} rows: {np.array_equal(ch_ref, ch_log)}; "
              f"rows with a non-stock choice at margin {margin}: {dev}")
        ok = ok and np.array_equal(ch_ref, ch_log)
        for k, name in enumerate(HEAD_NAMES):
            n_out = net["heads"][k]["n_out"]
            print(f"  {name:10s} {n_out}-way stock {net['heads'][k]['stock']}")
    print("OK" if ok else "MISMATCH")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
