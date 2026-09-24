"""tools/rl/model.py — the shared-trunk policy net in PyTorch, its export and import.

Plan: plan/rl-scheduler-solver13-plan.md §6.4 (network and inference),
§7 item 8 (the forward-pass test). Bead SAT-playground-p9m.9.2 (step D.2).

The net the solver runs (solver/13-kissat-rs/src/policy_net.rs): the
observation standardized by a fixed mean and std, a two-layer ReLU trunk,
and one linear head per knob whose entries are the knob's menu, in the
order probe, eliminate, reduce, rephase, reorder (5-way, stock at index 2),
mode (3-way, stock 1), restart margin (3-way, stock 1), sweep effort
(4-way, stock 2). A head may instead be linear on the standardized input
(kind 1), which is how a per-knob logistic ranker ships (plan §6.1 step 4).

`export` writes the weights file through the solver's own writer
(solver/13-kissat-rs/tools/rl/policy_net.py `write_net`), rounding every
number to float32; `from_file` loads one back. `scores_f64` evaluates a
file's weights in float64 numpy exactly as the solver does (only the
summation order differs; roundtrip.py is the bit-level check against the
solver's logged scores). `choose` mirrors policy_net::Net::choose: an entry
other than stock is taken only when its score beats stock's by the margin.

First net = stock (plan §6.4): `PolicyNet(..., stock_bias=b)` adds `b` to
every head's stock entry bias at construction, so an untrained net acts
like stock at a working margin.
"""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import torch
from torch import nn

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "solver" / "13-kissat-rs" / "tools" / "rl"))
sys.path.insert(0, str(ROOT / "solver" / "13-kissat-rs" / "tools"))
from policy_net import HEAD_MENUS, HEAD_NAMES, layout_hash, read_net, write_net  # noqa: E402

N_HEADS = len(HEAD_NAMES)
STOCK_INDEX = [stock for _, stock in HEAD_MENUS]
MENU_SIZES = [n for n, _ in HEAD_MENUS]


class PolicyNet(nn.Module):
    """Trunk n_in -> h1 -> h2 (ReLU) and one head per knob."""

    def __init__(self, names: list[str], mean, std, hidden=(128, 64), stock_bias: float = 0.0,
                 head_kinds: list[int] | None = None):
        super().__init__()
        self.names = list(names)
        n_in = len(names)
        self.register_buffer("mean", torch.as_tensor(np.asarray(mean, dtype=np.float32)))
        self.register_buffer("std", torch.as_tensor(np.asarray(std, dtype=np.float32)))
        h1, h2 = hidden
        self.fc1 = nn.Linear(n_in, h1)
        self.fc2 = nn.Linear(h1, h2)
        self.head_kinds = list(head_kinds) if head_kinds else [0] * N_HEADS
        if len(self.head_kinds) != N_HEADS:
            raise ValueError(f"{len(self.head_kinds)} head kinds, expected {N_HEADS}")
        self.heads = nn.ModuleList()
        for k in range(N_HEADS):
            width = h2 if self.head_kinds[k] == 0 else n_in
            self.heads.append(nn.Linear(width, MENU_SIZES[k]))
        with torch.no_grad():
            for k, head in enumerate(self.heads):
                head.bias[STOCK_INDEX[k]] += stock_bias

    @property
    def hidden(self) -> tuple[int, int]:
        return self.fc1.out_features, self.fc2.out_features

    def standardize(self, x: torch.Tensor) -> torch.Tensor:
        return (x - self.mean) / self.std

    def forward(self, x: torch.Tensor) -> list[torch.Tensor]:
        """Per-head score tensors (batch, menu) from raw observations."""
        z = self.standardize(x)
        a = torch.relu(self.fc2(torch.relu(self.fc1(z))))
        return [head(a if kind == 0 else z) for head, kind in zip(self.heads, self.head_kinds)]

    # -- the weights file ----------------------------------------------------

    def export(self, path: str | Path) -> None:
        f = lambda t: [float(v) for v in t.detach().cpu().numpy().reshape(-1)]
        rows = lambda t: [[float(v) for v in r] for r in t.detach().cpu().numpy()]
        heads = [{"kind": kind, "w": rows(h.weight), "b": f(h.bias)} for h, kind in zip(self.heads, self.head_kinds)]
        write_net(str(path), self.names, f(self.mean), f(self.std), rows(self.fc1.weight), f(self.fc1.bias),
                  rows(self.fc2.weight), f(self.fc2.bias), heads)

    @classmethod
    def from_file(cls, path: str | Path, names: list[str]) -> "PolicyNet":
        net = read_net(str(path))
        if net["n_in"] != len(names):
            raise ValueError(f"{path}: {net['n_in']} inputs, layout has {len(names)}")
        if net["layout_hash"] != layout_hash(names):
            # the same count in another order would silently pair weights
            # with the wrong entries (the solver refuses such a file too)
            raise ValueError(f"{path}: layout hash {net['layout_hash']:016x} is not the hash of the given names "
                             f"{layout_hash(names):016x}")
        kinds = [h["kind"] for h in net["heads"]]
        m = cls(names, net["mean"], net["std"], hidden=(net["h1"], net["h2"]), head_kinds=kinds)
        with torch.no_grad():
            m.fc1.weight.copy_(torch.tensor(net["w1"]))
            m.fc1.bias.copy_(torch.tensor(net["b1"]))
            m.fc2.weight.copy_(torch.tensor(net["w2"]))
            m.fc2.bias.copy_(torch.tensor(net["b2"]))
            for head, hd in zip(m.heads, net["heads"]):
                head.weight.copy_(torch.tensor(hd["w"]))
                head.bias.copy_(torch.tensor(hd["b"]))
        return m


def scores_f64(net: dict, X: np.ndarray) -> np.ndarray:
    """Scores (n, 35) of a read_net dict on raw observations, float64 on the
    file's float32 weights: what the solver computes, up to summation order."""
    x = (X.astype(np.float64) - np.asarray(net["mean"])) / np.asarray(net["std"])
    a1 = np.maximum(x @ np.asarray(net["w1"]).T + np.asarray(net["b1"]), 0.0)
    a2 = np.maximum(a1 @ np.asarray(net["w2"]).T + np.asarray(net["b2"]), 0.0)
    parts = []
    for hd in net["heads"]:
        inp = a2 if hd["kind"] == 0 else x
        parts.append(inp @ np.asarray(hd["w"]).T + np.asarray(hd["b"]))
    return np.concatenate(parts, axis=1)


def split_scores(scores: np.ndarray) -> list[np.ndarray]:
    out, pos = [], 0
    for n in MENU_SIZES:
        out.append(scores[:, pos:pos + n])
        pos += n
    return out


def choose(scores: np.ndarray, margin: float) -> np.ndarray:
    """(n, 8) chosen entry per head, as policy_net::Net::choose: the best
    entry when it beats stock by more than `margin`, else stock."""
    out = np.empty((scores.shape[0], N_HEADS), dtype=np.int64)
    for k, s in enumerate(split_scores(scores)):
        stock = STOCK_INDEX[k]
        best = np.argmax(s, axis=1)
        gap = s[np.arange(len(s)), best] - s[:, stock]
        take = (best != stock) & (gap > margin)
        out[:, k] = np.where(take, best, stock)
    return out


def mask_choice(choice: np.ndarray, stable: np.ndarray) -> np.ndarray:
    """policy::mask at default options: rephase is stock in focused mode and
    the restart margin in stable mode; the other knobs act in both modes."""
    c = choice.copy()
    k_rephase = HEAD_NAMES.index("rephase")
    k_margin = HEAD_NAMES.index("margin")
    c[~stable, k_rephase] = STOCK_INDEX[k_rephase]
    c[stable, k_margin] = STOCK_INDEX[k_margin]
    return c


def deviations(choice: np.ndarray) -> np.ndarray:
    """Per row, the number of heads whose chosen entry is not stock."""
    return (choice != np.asarray(STOCK_INDEX)).sum(axis=1)


def pairwise_stock_loss(scores: list[torch.Tensor]) -> torch.Tensor:
    """The ranking loss with stock as the winner of every pair: per head,
    mean over rows and alternatives of softplus(s_alt - s_stock). A
    pairwise-logistic score gap is a log-odds, which is what the solver's
    margin assumes (plan §6.4)."""
    total = 0.0
    for k, s in enumerate(scores):
        stock = STOCK_INDEX[k]
        gap = s - s[:, stock:stock + 1]
        alt = torch.cat([gap[:, :stock], gap[:, stock + 1:]], dim=1)
        total = total + torch.nn.functional.softplus(alt).mean()
    return total / len(scores)
