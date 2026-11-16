#!/usr/bin/env python3
"""Write a formula with the cases of a split as an incremental CNF file
(epic SAT-playground-bp3, step 5: the control split on assumptions).

The cases are every value of a few variables (the control bits). Each case
becomes one `a` line: "solve with these bits set this way". The solver's test
mode (`--inccnf=1`) then answers all of them with one solver, which is what
stage 1 of the split does. This is the tool for measuring that stage from
outside, before the split itself is rebuilt.

A case's value counts the bits low first: in case `w`, bit `i` is true when
`(w >> i) & 1` is 1. That is the order of src/control.rs.

usage:
  as_cases_icnf.py FORMULA --bits 10,11,12,13,14,15 --out cases.icnf
  as_cases_icnf.py FORMULA --bits 16,17,18,19,20 --fixed "10 11 12 13 14 15" --out level2.icnf

FORMULA is a DIMACS file (plain, .xz or .gz) or a headerless file of
clauses. --fixed puts the same literals in front of every case (a deeper
level of the split).
"""

import argparse
import gzip
import lzma
import sys


def open_any(path):
    if path.endswith(".xz"):
        return lzma.open(path, "rb")
    if path.endswith(".gz"):
        return gzip.open(path, "rb")
    return open(path, "rb")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("formula")
    ap.add_argument("--bits", required=True, help="the case variables, comma-separated, low bit first")
    ap.add_argument("--fixed", default="", help="literals in front of every case, space-separated")
    ap.add_argument("--cases", default="", help="only these case values, comma-separated (default: all)")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    bits = [int(t) for t in args.bits.split(",") if t.strip()]
    if not bits or any(b <= 0 for b in bits) or len(set(bits)) != len(bits):
        ap.error("--bits takes distinct positive variables")
    if len(bits) > 16:
        ap.error("more than 16 bits is more than 65,536 cases")
    fixed = [int(t) for t in args.fixed.split()]
    values = [int(t) for t in args.cases.split(",") if t.strip()] or list(range(1 << len(bits)))
    clauses = 0
    with open_any(args.formula) as src, open(args.out, "wb") as dst:
        dst.write(b"p inccnf\n")
        for line in src:
            if line[:1] in (b"p", b"c"):
                continue
            if line.strip():
                dst.write(line if line.endswith(b"\n") else line + b"\n")
                clauses += line.count(b" 0") + (1 if line.strip() == b"0" else 0)
        for w in values:
            lits = fixed + [b if (w >> i) & 1 else -b for i, b in enumerate(bits)]
            dst.write(("a " + " ".join(map(str, lits)) + " 0\n").encode())
    print(f"{args.out}: about {clauses} clause ends, {len(values)} cases over {len(bits)} bits"
          f"{', after ' + str(len(fixed)) + ' fixed literals' if fixed else ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
