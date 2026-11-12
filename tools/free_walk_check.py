#!/usr/bin/env python3
"""The free-choice walk in the solver (src/freewalk.rs, bead
SAT-playground-d96.2.4.2) against the prototype it ports (tools/free-walk).

  same CNF... [--steps N] [--seeds 0,1] [--solver PATH] [--keep DIR]
      For each cell and seed: the census's reading exported for the prototype
      (tools/free_walk.py export), the prototype walked N steps with weights on
      throughout (--phase 0), and the solver's walk the same N steps
      (--freewalk=1 --freewalktest=3 --freewalksteps=N --freewalkphase=0; the
      broad test, so the cells the default strict test turns down are walked
      too). The two must agree step for step: the same counts of steps,
      trials, flips, updates, pushes, the same best and the step it came at,
      and the same assignment at the end (the solver prints a hash of it; the
      prototype's is computed from its dump).
      Exit 1 on any difference.

The 2026 suite is not read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))
import free_walk as fw  # noqa: E402

ROOT = TOOLS.parent
SOLVER = ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"
# the other structure passes off: they never change the walk, but one that
# refutes the formula (the parity pass on Tseitin cells) leaves nothing to walk
OTHERS_OFF = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0",
              "--structdiv=0", "--structdd=0", "--gauss=0", "--card=0", "--structpoints=0"]
FIELDS = ["steps", "trials", "flips", "updates", "pushes", "best", "best_step"]


def fnv(bits):
    h = 0xcbf29ce484222325
    for b in bits:
        h ^= b
        h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def prototype(fwr, seed, steps):
    p = subprocess.run([str(fw.WALKER), str(fwr), "--seed", str(seed), "--steps", str(steps), "--phase", "0",
                        "--seconds", "1000000", "--progress", "0", "--dump"], stdout=subprocess.PIPE, text=True)
    _res, st = fw.parse_walk(p.stdout)
    val = {}
    for ln in p.stdout.splitlines():
        if ln.startswith("v"):
            for t in ln.split()[1:]:
                z = int(t)
                if z:
                    val[abs(z)] = 1 if z > 0 else 0
    return st, val


def solver(path, cnf, seed, steps):
    cmd = [str(path), "-n", "--conflicts=0", *OTHERS_OFF, f"--seed={seed}", "--freewalk=1", "--freewalktest=3", f"--freewalksteps={steps}", "--freewalkphase=0",
           "--freewalkticks=2147483647", str(cnf)]
    p = subprocess.run(cmd, stdout=subprocess.PIPE, text=True)
    for ln in p.stdout.splitlines():
        if ln.startswith("c freewalk "):
            return dict(kv.split("=", 1) for kv in ln.split()[2:] if "=" in kv), p.stdout
    return {}, p.stdout


def cmd_same(args):
    seeds = [int(x) for x in args.seeds.split(",")]
    ok = True
    keep = Path(args.keep) if args.keep else None
    with tempfile.TemporaryDirectory() as tmp:
        for cnf in args.cnf:
            if "sat-comp-2026" in str(Path(cnf).resolve()):
                raise SystemExit("the 2026 suite is the holdout and is not read")
            name = Path(cnf).name.split(".")[0][-48:]
            fwr = (keep or Path(tmp)) / (Path(cnf).name + ".fwr")
            if not fwr.exists():
                info = fw.export(cnf, fwr)
                if info.get("refuted"):
                    print(f"{name}: refuted by unit propagation, nothing to walk")
                    continue
            for seed in seeds:
                pst, pval = prototype(fwr, seed, args.steps)
                sst, out = solver(args.solver, cnf, seed, args.steps)
                if not sst or "vars" not in sst:
                    print(f"{name} seed {seed}: the solver walked no step ({sst.get('status', 'no line')})")
                    ok = False
                    continue
                nv = int(sst["vars"])
                phash = fnv(pval.get(v, 0) for v in range(1, nv + 1))
                diff = [f for f in FIELDS if str(pst.get(f)) != sst.get(f)]
                if f"{phash:016x}" != sst.get("hash"):
                    diff.append("assignment")
                same = not diff
                ok &= same
                print(f"{name} seed {seed}: {'same' if same else 'DIFFERENT ' + ','.join(diff)}  "
                      + " ".join(f"{f}={pst.get(f)}/{sst.get(f)}" for f in FIELDS)
                      + f" hash={phash:016x}/{sst.get('hash')} status={sst.get('status')}", flush=True)
    print("all the same" if ok else "SOME DIFFER")
    return 0 if ok else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("same")
    p.add_argument("cnf", nargs="+")
    p.add_argument("--steps", type=int, default=3000)
    p.add_argument("--seeds", default="0,1")
    p.add_argument("--solver", default=str(SOLVER))
    p.add_argument("--keep", default=None, help="a folder for the exported readings (reused when present)")
    args = ap.parse_args(argv)
    return {"same": cmd_same}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
