#!/usr/bin/env python3
"""tools/rl/clone.py — the stock clone: first net = stock (plan §6.4).

Plan: plan/rl-scheduler-solver13-plan.md §6.2 item 1 (behaviour cloning of
stock), §6.4 "First net = stock, enforced four ways", §8 (splits). Bead
SAT-playground-p9m.9.2 (step D.2); the exported net is the candidate of
the three-arm check D.3 (SAT-playground-p9m.9.3).

Under the relative encoding the stock action is the same entry of every
head at every state ("all ones"), so cloning stock is not a learning
problem: it is the plumbing check that a net trained here, exported to the
solver's weights file and run at a working margin deviates from stock on
about 0 % of decisions. The loss is the pairwise-logistic ranking loss
with stock as the winner of every pair (the same loss the fork-sibling
ranker uses later, so the scores are log-odds and the margin means what
plan §6.4 says), on the decision rows of the training-split stock traces;
the validation split is only reported on. Every head's stock bias starts
at `--stock-bias` (default 3, about 95 % odds), the trunk sees real states.

    ~/.cache/sat13-rl/venv/bin/python tools/rl/clone.py log/rl-stock2025-<ts> --out /path/clone.net.bin \\
        [--hidden 128,64] [--epochs 5] [--stock-bias 3] [--seed 1] [--margins 0,0.5,1,2]

Prints, per split and margin, the fraction of decision rows on which the
exported net (evaluated in float64 on its float32 weights, as the solver
does) would pick a non-stock entry on any head after the mode mask, and
per head; writes the net; then `roundtrip.py <net> <log>` is the check
against the solver's own forward pass.
"""
from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from data import load_decisions, load_norm  # noqa: E402
from model import (HEAD_NAMES, PolicyNet, choose, deviations, mask_choice, pairwise_stock_loss,  # noqa: E402
                   scores_f64)
from policy_net import read_net  # noqa: E402


def deviation_report(net: dict, X: np.ndarray, stable: np.ndarray, margins: list[float], label: str) -> dict:
    s = scores_f64(net, X)
    out = {}
    print(f"{label}: {len(X)} decision rows")
    print(f"  {'margin':>6s} {'rows deviating':>14s}  per head: " + " ".join(f"{h[:5]:>5s}" for h in HEAD_NAMES))
    for m in margins:
        ch = mask_choice(choose(s, m), stable)
        dev = deviations(ch) > 0
        per_head = (ch != np.array([net["heads"][k]["stock"] for k in range(len(HEAD_NAMES))])).mean(axis=0)
        out[m] = {"rows": float(dev.mean()), "per_head": per_head.tolist()}
        print(f"  {m:6.2f} {100 * dev.mean():13.3f}%  " + " ".join(f"{100 * p:5.2f}" for p in per_head))
    # the score gap stock enjoys: how far the net is from a deviation
    gaps = []
    pos = 0
    for k, h in enumerate(net["heads"]):
        sk = s[:, pos:pos + h["n_out"]]
        pos += h["n_out"]
        alt = np.delete(sk, h["stock"], axis=1)
        gaps.append(sk[:, h["stock"]] - alt.max(axis=1))
    g = np.stack(gaps, axis=1)
    print(f"  stock's lead over the best alternative (log-odds): min {g.min():.2f}, "
          f"1st percentile {np.percentile(g, 1):.2f}, median {np.median(g):.2f}")
    out["lead_min"] = float(g.min())
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir", help="the converted stock pass (plan §5.2)")
    ap.add_argument("--out", required=True, help="the weights file to write")
    ap.add_argument("--hidden", default="128,64")
    ap.add_argument("--epochs", type=int, default=5)
    ap.add_argument("--batch", type=int, default=256)
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--stock-bias", type=float, default=3.0)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--margins", default="0,0.5,1,2")
    ap.add_argument("--limit-cells", type=int, default=0, help="first N cells per split (tests)")
    args = ap.parse_args(argv)
    torch.manual_seed(args.seed)
    np.random.seed(args.seed)
    torch.set_num_threads(4)
    hidden = tuple(int(v) for v in args.hidden.split(","))
    margins = [float(v) for v in args.margins.split(",")]
    norm = load_norm()
    t0 = time.time()
    X_tr, m_tr = load_decisions(args.run_dir, "train", norm, limit_cells=args.limit_cells or None)
    X_va, m_va = load_decisions(args.run_dir, "val", norm, limit_cells=args.limit_cells or None)
    print(f"loaded {len(X_tr)} training and {len(X_va)} validation decision rows "
          f"({len(set(m_tr['stem']))} and {len(set(m_va['stem']))} cells) in {time.time() - t0:.0f}s")

    net = PolicyNet(norm.names, norm.mean, norm.std, hidden=hidden, stock_bias=args.stock_bias)
    opt = torch.optim.Adam(net.parameters(), lr=args.lr)
    Xt = torch.as_tensor(X_tr)
    Xv = torch.as_tensor(X_va)
    with torch.no_grad():
        print(f"initial loss: train {pairwise_stock_loss(net(Xt)).item():.4f} "
              f"val {pairwise_stock_loss(net(Xv)).item():.4f}")
    n = len(Xt)
    for ep in range(args.epochs):
        perm = torch.randperm(n)
        tot = 0.0
        for i in range(0, n, args.batch):
            idx = perm[i:i + args.batch]
            loss = pairwise_stock_loss(net(Xt[idx]))
            opt.zero_grad()
            loss.backward()
            opt.step()
            tot += loss.item() * len(idx)
        with torch.no_grad():
            lv = pairwise_stock_loss(net(Xv)).item()
        print(f"epoch {ep + 1}: train loss {tot / n:.4f}, val loss {lv:.4f}")

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    net.export(out)
    exported = read_net(str(out))
    print(f"wrote {out}: {exported['n_in']} inputs, hidden {exported['h1']},{exported['h2']}, "
          f"layout {exported['layout_hash']:016x}")
    deviation_report(exported, X_tr, m_tr["stable"], margins, "training split")
    deviation_report(exported, X_va, m_va["stable"], margins, "validation split")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
