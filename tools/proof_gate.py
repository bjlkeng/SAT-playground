#!/usr/bin/env python3
"""proof_gate.py — check an UNSAT answer of solver 13, with or without the
structure pass (plan section 11, 2026-10-07; bead SAT-playground-1v2.1).

Stock answer: `<proof>` is a DRAT proof of the input, checked with
drat-trim. Structure-pass answer: the solver also left `<proof>.pbp` (a
VeriPB 3 proof over the input's clause numbering) and
`<proof>.derived.cnf` (the clauses the pass added). Then the gate checks
two stages: VeriPB on the input with the extended formula F' = input plus
derived as the output file (`output EQUISATISFIABLE FILE`), and drat-trim on
F' with `<proof>`. When the pass derived the empty clause its proof ends in
`conclusion UNSAT` and stage 1 alone settles it.

VeriPB 3 reads CNF input but not CNF output files, so both formulas are
handed to it as OPB with the same constraint ids (clause i is constraint
i, duplicates and tautologies included).

    python3 tools/proof_gate.py <cnf[.xz|.gz]> <proof> [--timeout S]
    exit 0 and "ok" on a verified refutation, else a reason

As a module: `check_unsat(cnf, proof, timeout) -> str` returns ok | FAIL:<why>
| no-proof | no-checker | checker-timeout | checker-error.
"""
from __future__ import annotations

import argparse
import gzip
import lzma
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHECKERS = ROOT / "tools" / "checkers"


def find_drat_trim() -> str | None:
    local = CHECKERS / "drat-trim" / "drat-trim"
    if local.is_file() and os.access(local, os.X_OK):
        return str(local)
    return shutil.which("drat-trim")


def find_veripb() -> str | None:
    local = CHECKERS / "VeriPB" / "target" / "release" / "veripb"
    if local.is_file() and os.access(local, os.X_OK):
        return str(local)
    return shutil.which("veripb")


def open_cnf(path):
    """The file as bytes: lines split on newline only, as the solver splits
    them (a carriage return inside a comment does not end it)."""
    p = str(path)
    if p.endswith(".xz"):
        return lzma.open(p, "rb")
    if p.endswith(".gz"):
        return gzip.open(p, "rb")
    return open(p, "rb")


class MalformedCnf(Exception):
    pass


def iter_clauses(cnf):
    """The clauses of a DIMACS file as the solver reads them (kissat's
    rules): a `c` at a token start comments the rest of the line, wherever
    it is on the line and even right after a number; the header is the
    first `p cnf` line; anything else that is not an integer is an error.
    Yields lists of literals as integers; returns the header's variable count."""
    header_vars = 0
    cur: list[int] = []
    with open_cnf(cnf) as f:
        for raw in f:
            line = raw.decode("latin-1")
            i, n = 0, len(line)
            while i < n:
                ch = line[i]
                if ch in " \t\r\n":
                    i += 1
                    continue
                if ch == "\0":
                    raise MalformedCnf("binary data")
                if ch == "c":
                    break                     # comment to the end of the line
                if ch == "p":
                    if header_vars or cur:
                        raise MalformedCnf("second header")
                    parts = line[i:].split()
                    if len(parts) < 4 or parts[1] != "cnf":
                        raise MalformedCnf("bad header")
                    header_vars = int(parts[2])
                    break
                j = i
                if ch == "-":
                    j += 1
                while j < n and line[j].isdigit():
                    j += 1
                tok = line[i:j]
                if tok in ("", "-"):
                    raise MalformedCnf(f"unexpected '{ch}'")
                i = j
                lit = int(tok)                # the solver reads integers: 01 is 1
                if lit == 0:
                    yield cur
                    cur = []
                else:
                    cur.append(lit)
    if cur:
        raise MalformedCnf("trailing zero missing")
    return header_vars


def cnf_to_opb_and_copy(cnf, opb_out, cnf_out, extra_clauses):
    """Write the input as OPB (and as plain CNF) with the extra clauses
    appended; returns (vars, clauses) of the result."""
    n = 0
    with open(opb_out, "w") as o, open(cnf_out, "w") as c:
        o.write("* #variable= 0 #constraint= 0\n")       # fixed below
        c.write("p cnf 0 0\n")
        gen = iter_clauses(cnf)
        header_vars = 0
        while True:
            try:
                cur = next(gen)
            except StopIteration as stop:
                header_vars = stop.value or 0
                break
            n += 1
            o.write(" ".join(f"+1 {'~' if l < 0 else ''}x{abs(l)}" for l in cur) + " >= 1 ;\n")
            c.write(" ".join(map(str, cur)) + " 0\n")
        for clause in extra_clauses:
            n += 1
            o.write(" ".join(f"+1 {'~' if l < 0 else ''}x{abs(l)}" for l in clause) + " >= 1 ;\n")
            c.write(" ".join(map(str, clause)) + " 0\n")
    return header_vars, n


def fix_headers(opb_out, cnf_out, nvars, n):
    for path, header in ((opb_out, f"* #variable= {nvars} #constraint= {n}\n"), (cnf_out, f"p cnf {nvars} {n}\n")):
        with open(path, "r+") as fh:
            body = fh.read().split("\n", 1)[1]
            fh.seek(0)
            fh.write(header + body)
            fh.truncate()


def read_derived(path):
    """The derived clauses, read with the solver's DIMACS rules."""
    gen = iter_clauses(path)
    clauses = []
    while True:
        try:
            clauses.append(next(gen))
        except StopIteration as stop:
            return stop.value or 0, clauses


def run_veripb(veripb, args, timeout):
    p = subprocess.run([veripb, *args], capture_output=True, text=True, timeout=timeout)   # no deletions, so the checked-deletion default holds
    lines = [l.strip() for l in (p.stdout or "").splitlines()]
    tail = [l.strip() for l in ((p.stdout or "") + (p.stderr or "")).splitlines() if l.strip()]
    return p.returncode, lines, (tail[-1] if tail else "")


def check_unsat(cnf, proof, timeout: float = 3600.0, workdir=None) -> str:
    cnf, proof = Path(cnf), Path(proof)
    if not proof.is_file():
        return "no-proof"
    drat = find_drat_trim()
    pbp = Path(str(proof) + ".pbp")
    derived = Path(str(proof) + ".derived.cnf")
    try:
        if not pbp.is_file():
            if not drat:
                return "no-checker"
            if derived.is_file():
                return "FAIL:derived clauses without a VeriPB proof"
            with tempfile.TemporaryDirectory(prefix="proof-gate-", dir=workdir) as d:
                plain = str(cnf)
                if plain.endswith((".xz", ".gz")):
                    plain = os.path.join(d, "f.cnf")
                    with open_cnf(cnf) as src, open(plain, "wb") as dst:
                        shutil.copyfileobj(src, dst)
                p = subprocess.run([drat, plain, str(proof)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, timeout=timeout)
            ok = any(l.strip() in ("s VERIFIED", "s ACCEPTED") for l in (p.stdout or "").splitlines())
            return "ok" if ok else "FAIL:drat-trim rejected the proof"
        veripb = find_veripb()
        if not veripb or not drat:
            return "no-checker"
        if not derived.is_file():
            return "FAIL:VeriPB proof without the derived clauses"
        # the proof must end in a refutation or in the extended formula as
        # its checked output (lines split on newline only, as VeriPB does;
        # a comment cannot pass as a section line this way, and a second
        # output section fails VeriPB's own parse)
        tail = [l.rstrip("\r") for l in pbp.read_text(errors="replace").rstrip("\n").split("\n")[-4:]]
        concluded_unsat = any(l.startswith("conclusion UNSAT") for l in tail)
        names_output = any(l.startswith("output EQUISATISFIABLE FILE") or l.startswith("output DERIVABLE FILE") for l in tail)
        if not concluded_unsat and not names_output:
            return "FAIL:the structure proof neither refutes nor names the extended formula as its output"
        with tempfile.TemporaryDirectory(prefix="proof-gate-", dir=workdir) as d:
            dvars, extra = read_derived(derived)
            f_opb, fp_opb, fp_cnf = os.path.join(d, "f.opb"), os.path.join(d, "fprime.opb"), os.path.join(d, "fprime.cnf")
            nvars, n0 = cnf_to_opb_and_copy(cnf, f_opb, os.path.join(d, "f.cnf"), [])
            fix_headers(f_opb, os.path.join(d, "f.cnf"), max(nvars, dvars), n0)
            nvars2, n1 = cnf_to_opb_and_copy(cnf, fp_opb, fp_cnf, extra)
            fix_headers(fp_opb, fp_cnf, max(nvars2, dvars), n1)
            if concluded_unsat:
                code, lines, last = run_veripb(veripb, [f_opb, str(pbp)], timeout)
                if code == 0 and "s VERIFIED UNSATISFIABLE" in lines:
                    return "ok"
                return f"FAIL:VeriPB rejected the structure proof ({last[:120]})"
            code, lines, last = run_veripb(veripb, [f_opb, str(pbp), fp_opb], timeout)
            if code != 0 or "s VERIFIED NO CONCLUSION" not in lines:
                return f"FAIL:VeriPB rejected the structure proof or the extended formula ({last[:120]})"
            # negative control: the same proof against F' plus one clause the
            # proof never derived must be rejected; this shows VeriPB really
            # compared the output file, whatever the proof's text says
            bad_opb = os.path.join(d, "fprime-bad.opb")
            with open(fp_opb) as src, open(bad_opb, "w") as dst:
                for line in src:
                    dst.write(line)
                dst.write(f"+1 x{max(nvars2, dvars) + 1} >= 1 ;\n")
            code, lines, last = run_veripb(veripb, [f_opb, str(pbp), bad_opb], timeout)
            if code == 0 and "s VERIFIED NO CONCLUSION" in lines:
                return "FAIL:VeriPB did not check the extended formula (the proof's output section is not FILE)"
            p = subprocess.run([drat, fp_cnf, str(proof)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, timeout=timeout)
            ok = any(l.strip() in ("s VERIFIED", "s ACCEPTED") for l in (p.stdout or "").splitlines())
            return "ok" if ok else "FAIL:drat-trim rejected the proof of the extended formula"
    except subprocess.TimeoutExpired:
        return "checker-timeout"
    except MalformedCnf as e:
        return f"FAIL:malformed CNF ({e})"
    except Exception as e:                       # a malformed artifact is a failure, not a crash
        return f"checker-error:{e.__class__.__name__}"


def main(argv) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cnf")
    ap.add_argument("proof")
    ap.add_argument("--timeout", type=float, default=3600.0)
    args = ap.parse_args(argv)
    r = check_unsat(args.cnf, args.proof, args.timeout)
    print(r)
    return 0 if r == "ok" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
