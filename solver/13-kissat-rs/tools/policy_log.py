#!/usr/bin/env python3
"""Reader for solver 13's RL policy log (SAT_POLICY_LOG; plan step A.5).

File layout (written by solver/13-kissat-rs/src/policy_log.rs):
  line 1  "SAT13POLICYLOG 1"
  line 2  JSON header: format, solver, k_res, cnf, pid, policy, options,
          columns, kinds ('u' = u64, 'f' = f64 bit pattern), row_bytes
  rows    row_bytes each, little-endian u64 per column
  sentinel a row whose first u64 is 2^64 - 1
  footer  JSON: result, exit, reason (solve | signal | output-error; only
          "solve" is a normal completion), rows, wall_ns, cpu_ns,
          peak_rss_bytes, work, conflicts, static (the A.7 static features,
          {} until then). A file without a footer was cut off (a write
          failure, or a kill the handler could not finalize).

    from policy_log import read
    header, rows, footer = read("run.log")        # rows: list of lists (ints/floats)
    col = header["columns"].index("conflicts")

Usage: policy_log.py <log> [--tail N] [--columns a,b,c]
"""
import json
import struct
import sys

MAGIC = b"SAT13POLICYLOG 1"
SENTINEL = (1 << 64) - 1


def read(path):
    with open(path, "rb") as fh:
        data = fh.read()
    nl1 = data.index(b"\n")
    if data[:nl1] != MAGIC:
        raise ValueError(f"{path}: not a policy log (magic {data[:nl1]!r})")
    nl2 = data.index(b"\n", nl1 + 1)
    header = json.loads(data[nl1 + 1:nl2])
    ncol = len(header["columns"])
    row_bytes = 8 * ncol
    assert header["row_bytes"] == row_bytes
    kinds = header["kinds"]
    fmt = "<" + "Q" * ncol
    pos = nl2 + 1
    rows = []
    footer = None
    while pos + row_bytes <= len(data):
        vals = struct.unpack_from(fmt, data, pos)
        pos += row_bytes
        if vals[0] == SENTINEL:
            rest = data[pos:].strip()
            footer = json.loads(rest) if rest else None
            break
        rows.append([struct.unpack("<d", struct.pack("<Q", v))[0] if k == "f" else v
                     for v, k in zip(vals, kinds)])
    else:
        # No sentinel: the run was killed before its footer; rows are still good.
        footer = None
    return header, rows, footer


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    path = argv[1]
    tail = 5
    cols = None
    i = 2
    while i < len(argv):
        if argv[i] == "--tail":
            tail = int(argv[i + 1]); i += 2
        elif argv[i] == "--columns":
            cols = argv[i + 1].split(","); i += 2
        else:
            raise SystemExit(f"unknown argument {argv[i]}")
    header, rows, footer = read(path)
    names = header["columns"]
    print(f"cnf: {header['cnf']}")
    print(f"policy: {header['policy']}")
    print(f"columns: {len(names)}  rows: {len(rows)}  footer: {footer}")
    show = cols or ["obs_epoch", "policy_decisions", "search_ticks", "work", "conflicts",
                    "wall_ns", "stable", "level"]
    idx = [names.index(c) for c in show]
    print("\t".join(show))
    for r in rows[-tail:]:
        print("\t".join(str(r[j]) for j in idx))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
