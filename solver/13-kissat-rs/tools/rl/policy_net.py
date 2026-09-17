#!/usr/bin/env python3
"""Solver 13 policy weights files: writer, reader, reference forward pass.

The file format is defined in solver/13-kissat-rs/src/policy_net.rs
(SAT13POLICYNET, format 1): a header, then float32 arrays for the input
normalization, a two-layer ReLU trunk and one head per knob. This module is
the reference the Rust forward pass is tested against (tests in
policy_net.rs run `--forward`), the fixture generator for those tests, and
the exporter a trainer calls once it has a model (`write_net`).

Arithmetic matches the Rust pass exactly: weights are float32, every dot
product accumulates in float64 in index order with plain multiply-then-add,
so both sides produce the same bits. When PyTorch is installed `--forward`
also prints the PyTorch result of the same net evaluated in float64 on the
same float32 weights (the function the solver runs; a float32 evaluation
would differ by float32 rounding, about 1e-6 relative on 250-term dot
products, which is exactly what the check must not confuse with a bug),
which the Rust test requires to agree within 1e-6 relative.

Usage:
  policy_net.py --forward <net.bin> <cases.txt>       # one input vector per line
  policy_net.py --fixture <log> <net.bin> [--seed S] [--hidden H1,H2] [--stock-bias B]
  policy_net.py --info <net.bin>

The observation layout (entry names, hence the layout hash) is read from a
policy log's header: every `obs_<name>` column except `obs_epoch`.
"""
import json
import math
import os
import struct
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
from policy_log import read as read_log  # noqa: E402

MAGIC = b"SAT13POLICYNET\0\0"
FORMAT = 1
HEAD_NAMES = ["probe", "eliminate", "reduce", "rephase", "reorder", "mode", "margin", "sweep"]
# (menu size, stock index) per head, as policy.rs defines the menus.
HEAD_MENUS = [(5, 2), (5, 2), (5, 2), (5, 2), (5, 2), (3, 1), (3, 1), (4, 2)]


def layout_hash(names):
    """FNV-1a 64 of the names joined by newlines (policy_net::layout_hash)."""
    h = 0xCBF29CE484222325
    data = "\n".join(names).encode()
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def obs_names_from_log(path):
    header, _, _ = read_log(path)
    return [c[len("obs_"):] for c in header["columns"] if c.startswith("obs_") and c != "obs_epoch"]


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def write_net(path, names, mean, std, w1, b1, w2, b2, heads):
    """Write a net. `w1` is h1 rows of n_in, `w2` h2 rows of h1; `heads` is a
    list of dicts {kind, w (n_out rows), b (n_out)} in HEAD_NAMES order.
    Every number is rounded to float32 on the way out."""
    n_in = len(names)
    h1, h2 = len(w1), len(w2)
    if len(heads) != len(HEAD_NAMES):
        raise ValueError(f"{len(heads)} heads, expected {len(HEAD_NAMES)}")
    out = bytearray(MAGIC)
    out += struct.pack("<IIQIII", FORMAT, n_in, layout_hash(names), h1, h2, len(heads))
    for k, hd in enumerate(heads):
        n_out, stock = HEAD_MENUS[k]
        if len(hd["w"]) != n_out or len(hd["b"]) != n_out:
            raise ValueError(f"head {HEAD_NAMES[k]}: {len(hd['w'])} outputs, menu has {n_out}")
        width = h2 if hd["kind"] == 0 else n_in
        for row in hd["w"]:
            if len(row) != width:
                raise ValueError(f"head {HEAD_NAMES[k]}: row width {len(row)}, expected {width}")
        out += struct.pack("<III", hd["kind"], n_out, stock)
    def floats(xs):
        return struct.pack(f"<{len(xs)}f", *xs)
    assert len(mean) == n_in and len(std) == n_in
    out += floats(mean) + floats(std)
    for row in w1:
        assert len(row) == n_in
        out += floats(row)
    out += floats(b1)
    for row in w2:
        assert len(row) == h1
        out += floats(row)
    out += floats(b2)
    for hd in heads:
        for row in hd["w"]:
            out += floats(row)
        out += floats(hd["b"])
    with open(path, "wb") as fh:
        fh.write(out)


def read_net(path):
    data = open(path, "rb").read()
    if data[:16] != MAGIC:
        raise ValueError(f"{path}: bad magic")
    pos = 16
    fmt, n_in, lh, h1, h2, n_heads = struct.unpack_from("<IIQIII", data, pos)
    pos += struct.calcsize("<IIQIII")
    if fmt != FORMAT:
        raise ValueError(f"{path}: format {fmt}")
    shapes = []
    for _ in range(n_heads):
        shapes.append(struct.unpack_from("<III", data, pos))
        pos += 12
    def take(n):
        nonlocal pos
        v = list(struct.unpack_from(f"<{n}f", data, pos))
        pos += 4 * n
        return v
    def rows(n, width):
        return [take(width) for _ in range(n)]
    net = {"n_in": n_in, "h1": h1, "h2": h2, "layout_hash": lh}
    net["mean"] = take(n_in)
    net["std"] = [s if s > 0.0 else 1.0 for s in take(n_in)]
    net["w1"] = rows(h1, n_in)
    net["b1"] = take(h1)
    net["w2"] = rows(h2, h1)
    net["b2"] = take(h2)
    heads = []
    for kind, n_out, stock in shapes:
        width = h2 if kind == 0 else n_in
        heads.append({"kind": kind, "n_out": n_out, "stock": stock, "w": rows(n_out, width),
                      "b": take(n_out)})
    net["heads"] = heads
    if pos != len(data):
        raise ValueError(f"{path}: {len(data) - pos} trailing bytes")
    return net


def forward(net, obs):
    """Scores for one observation (a list of float32 values), float64 math in
    the Rust order. Returns the concatenated head scores."""
    x = [(o - m) / s for o, m, s in zip(obs, net["mean"], net["std"])]
    def layer(w, b, inp, relu):
        out = []
        for row, bj in zip(w, b):
            acc = bj
            for wi, xi in zip(row, inp):
                acc += wi * xi
            out.append(acc if (not relu or acc > 0.0) else 0.0)
        return out
    a1 = layer(net["w1"], net["b1"], x, True)
    a2 = layer(net["w2"], net["b2"], a1, True)
    scores = []
    for hd in net["heads"]:
        inp = a2 if hd["kind"] == 0 else x
        scores += layer(hd["w"], hd["b"], inp, False)
    return scores


def choose(net, scores, margin):
    """The chosen entry per head: the best one when it beats stock by more
    than `margin`, else stock (policy_net::Net::choose)."""
    out = []
    pos = 0
    for hd in net["heads"]:
        s = scores[pos:pos + hd["n_out"]]
        pos += hd["n_out"]
        best = hd["stock"]
        for j in range(hd["n_out"]):
            if s[j] > s[best]:
                best = j
        out.append(best if best != hd["stock"] and s[best] - s[hd["stock"]] > margin else hd["stock"])
    return out


def torch_forward(net, obs):
    """The same net in float64 PyTorch (float32 weights widened, like the
    solver's pass), or None when torch is not installed."""
    try:
        import torch
    except ImportError:
        return None
    t = lambda v: torch.tensor(v, dtype=torch.float64)
    x = (t(obs) - t(net["mean"])) / t(net["std"])
    a1 = torch.relu(t(net["w1"]) @ x + t(net["b1"]))
    a2 = torch.relu(t(net["w2"]) @ a1 + t(net["b2"]))
    scores = []
    for hd in net["heads"]:
        inp = a2 if hd["kind"] == 0 else x
        scores += (t(hd["w"]) @ inp + t(hd["b"])).tolist()
    return scores


def fixture(names, seed=1, hidden=(32, 16), stock_bias=3.0, kind1_heads=False):
    """A random net for the layout: small weights, and `stock_bias` added to
    every head's stock entry so the untrained net acts like stock at a
    working margin (plan §6.4). Returns the write_net arguments."""
    state = seed & 0xFFFFFFFFFFFFFFFF
    def u():
        nonlocal state
        state = (state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        return (state >> 11) / float(1 << 53) * 2.0 - 1.0
    n_in = len(names)
    h1, h2 = hidden
    mean = [u() * 0.5 for _ in range(n_in)]
    std = [1.0 + abs(u()) for _ in range(n_in)]
    w1 = [[u() * 0.2 for _ in range(n_in)] for _ in range(h1)]
    b1 = [u() * 0.1 for _ in range(h1)]
    w2 = [[u() * 0.3 for _ in range(h1)] for _ in range(h2)]
    b2 = [u() * 0.1 for _ in range(h2)]
    heads = []
    for k, (n_out, stock) in enumerate(HEAD_MENUS):
        kind = 1 if kind1_heads and k % 2 == 1 else 0
        width = h2 if kind == 0 else n_in
        w = [[u() * 0.3 for _ in range(width)] for _ in range(n_out)]
        b = [u() * 0.5 + (stock_bias if j == stock else 0.0) for j in range(n_out)]
        heads.append({"kind": kind, "w": w, "b": b})
    return mean, std, w1, b1, w2, b2, heads


def main(argv):
    if len(argv) >= 4 and argv[1] == "--forward":
        net = read_net(argv[2])
        cases = []
        for line in open(argv[3]):
            line = line.strip()
            if line:
                cases.append([f32(float(v)) for v in line.split()])
        for c in cases:
            if len(c) != net["n_in"]:
                raise SystemExit(f"case has {len(c)} values, net expects {net['n_in']}")
            print("python " + " ".join(repr(v) for v in forward(net, c)))
        torch_ok = True
        for c in cases:
            s = torch_forward(net, c)
            if s is None:
                torch_ok = False
                break
            print("torch " + " ".join(repr(v) for v in s))
        if not torch_ok:
            print("torch unavailable")
        return 0
    if len(argv) >= 4 and argv[1] == "--fixture":
        log, out = argv[2], argv[3]
        seed, hidden, bias, kind1 = 1, (32, 16), 3.0, False
        i = 4
        while i < len(argv):
            if argv[i] == "--seed":
                seed = int(argv[i + 1]); i += 2
            elif argv[i] == "--hidden":
                hidden = tuple(int(v) for v in argv[i + 1].split(",")); i += 2
            elif argv[i] == "--stock-bias":
                bias = float(argv[i + 1]); i += 2
            elif argv[i] == "--mixed-heads":
                kind1 = True; i += 1
            else:
                raise SystemExit(f"unknown argument {argv[i]}")
        names = obs_names_from_log(log)
        write_net(out, names, *fixture(names, seed, hidden, bias, kind1))
        print(f"wrote {out}: {len(names)} inputs, hidden {hidden}, layout {layout_hash(names):016x}")
        return 0
    if len(argv) >= 3 and argv[1] == "--info":
        net = read_net(argv[2])
        print(json.dumps({k: net[k] for k in ("n_in", "h1", "h2")}
                         | {"layout_hash": f"{net['layout_hash']:016x}",
                            "heads": [{"name": n, "kind": h["kind"], "n_out": h["n_out"],
                                       "stock": h["stock"]}
                                      for n, h in zip(HEAD_NAMES, net["heads"])]}, indent=1))
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
