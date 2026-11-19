#!/usr/bin/env python3
"""number_pass_check.py - the solver's number pass against the Python reader,
clause for clause (bead SAT-playground-d96.9.4).

The pass (solver/13-kissat-rs/src/structnum.rs, --structnum=1) and
tools/number_read.py read the same thing the same way: numbers stored as
sorted bits, and the units and two-literal clauses their encoding lacks.
This tool runs both on one formula and compares what they add, in order.

A small example. The toy of the README (x in 0..3 as three sorted bits, a
count with three helpers): the reader lists "h2 -> g2", "g2 -> h2", "not h3
-> not g3" and "h1 -> g1"; the solver, run with the pass on and a proof
path, writes the same four clauses to PROOF.derived.cnf, in the same order.
A difference in one clause or in the order is a failure here.

The reader is asked for exactly what the pass does: bits that no shape could
define, linked by two-literal clauses (propagation is not asked for hidden
links), no chain of two members, clauses only for numbers under a count, no
pairs, no partial readers, the kinds equal, bound, dead and unit, and
nothing without a short proof.

  catalog [--solver BIN] [--jobs N] [--proof]
                                      every record of the Z3 catalog with a
                                      sorted-bit number or a binary-coded
                                      choice, as written and renumbered
                                      twice with signs flipped
  files [--solver BIN] [--proof] FILE...
                                      formulas on disk (plain or .xz)
With --proof the pass's proof also goes to VeriPB: every step, and the
derived clauses as the proof's output (tools/proof_gate.py, first stage).
  selftest [--solver BIN]             the toy above, and a pass that is
                                      made to differ (a stand-in solver)

The 2026 suite is never read (tools/number_read.py refuses it).
"""
from __future__ import annotations

import argparse
import multiprocessing
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import number_read as nr  # noqa: E402
import proof_gate  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SOLVER = ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"
# every other pass and engine off: the number pass alone decides what is derived
STOCK = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0", "--structdiv=0",
         "--structdd=0", "--gauss=0", "--card=0", "--structpoints=0", "--freewalk=0", "--ctlsplit=0"]
KINDS = {"equal", "bound", "dead", "unit"}
MAX_BITS = 62


def reader_clauses(n_vars, clauses):
    """What tools/number_read.py adds when it reads as the pass does."""
    r = nr.Reading(n_vars, clauses, full_scan=0, pairs=False, partial=False, short_limit=-1, kinds=KINDS,
                   proved_only=True, max_bits=MAX_BITS, inputs_only=True, counted_only=True, end_fan=nr.END_FAN)
    if r.truncated:
        raise RuntimeError("the reader stopped at its budget")
    return [list(c) for c in r.link_clauses()], r


def proof_problem(cnf_path, workdir):
    """'' when VeriPB takes the pass's proof (every step, and the derived
    clauses as the proof's output), else why not.  A pass that added nothing
    leaves a proof with nothing to check: also ''."""
    proof = Path(workdir) / "proof"
    pbp, der = Path(str(proof) + ".pbp"), Path(str(proof) + ".derived.cnf")
    if not pbp.exists() or not der.exists():
        return "no proof file beside the derived clauses" if der.exists() else ""
    veripb = proof_gate.find_veripb()
    if veripb is None:
        return "no VeriPB to check with"
    sub = Path(workdir) / "gate"
    sub.mkdir(exist_ok=True)
    verdict, _fp = proof_gate.veripb_stage(str(cnf_path), pbp, der, veripb, 600, str(sub))
    return "" if verdict in ("", "ok") else verdict


def pass_clauses(solver, cnf_path, workdir, timeout=3600):
    """What the solver's pass adds: the clauses of PROOF.derived.cnf in
    order, and its report line as a dict.  The search is stopped at once
    (--conflicts=0): the pass runs before it."""
    proof = Path(workdir) / "proof"
    for side in (proof, Path(str(proof) + ".pbp"), Path(str(proof) + ".derived.cnf")):
        if side.exists():
            side.unlink()
    p = subprocess.run([str(solver), "-n", *STOCK, "--structnum=1", "--conflicts=0", str(cnf_path), str(proof)],
                       capture_output=True, text=True, timeout=timeout)
    line = [l for l in p.stdout.splitlines() if l.startswith("c numbers ")]
    if len(line) != 1:
        raise RuntimeError(f"no single 'c numbers' line (exit {p.returncode}): {p.stdout[-300:]} {p.stderr[-300:]}")
    report = dict(kv.split("=", 1) for kv in line[0].split()[2:])
    der = Path(str(proof) + ".derived.cnf")
    out = []
    if der.exists():
        for l in der.read_text().splitlines():
            if l and l[0] not in "cp":
                lits = [int(t) for t in l.split()]
                if lits[-1] != 0:
                    raise RuntimeError(f"a derived clause without its 0: {l!r}")
                out.append(lits[:-1])
    return out, report


def compare(name, want, got, report):
    """'' when the two lists are the same, else what differs."""
    if report.get("status") == "root-conflict":
        # the formula's own units refute it: nothing to read, for either of the two
        return "" if not want and not got else f"{name}: the units refute the formula, and clauses were added"
    if report.get("status") != "read":
        return f"{name}: the pass says status={report.get('status')}"
    if want == got:
        if int(report["added"]) != len(got):
            return f"{name}: the pass reports {report['added']} added clauses and wrote {len(got)}"
        return ""
    sw, sg = {tuple(c) for c in want}, {tuple(c) for c in got}
    if sw == sg:
        return f"{name}: the same {len(want)} clauses in another order"
    only_r = sorted(sw - sg)[:3]
    only_p = sorted(sg - sw)[:3]
    return (f"{name}: reader {len(want)} clauses, pass {len(got)}; only the reader: {only_r}{'...' if len(sw - sg) > 3 else ''}; "
            f"only the pass: {only_p}{'...' if len(sg - sw) > 3 else ''}")


def write_cnf(path, n_vars, clauses):
    with open(path, "w") as f:
        f.write(f"p cnf {n_vars} {len(clauses)}\n")
        for c in clauses:
            f.write(" ".join(map(str, c)) + " 0\n")


_CAT = None
_SOLVER = None
_PROOF = False


def _catalog_job(job):
    index, seed = job
    rec = _CAT[index]
    clauses = rec["clauses"]
    if seed is not None:
        clauses, _m = nr.renumber(rec["n_vars"], rec["clauses"], seed)
    name = f"{rec['id']}{'' if seed is None else f' renumbered {seed}'}"
    with tempfile.TemporaryDirectory() as d:
        cnf = Path(d) / "f.cnf"
        write_cnf(cnf, rec["n_vars"], clauses)
        try:
            want, r = reader_clauses(rec["n_vars"], clauses)
            got, report = pass_clauses(_SOLVER, cnf, d)
            msg = compare(name, want, got, report)
            # the binary-coded choices: the same number, with the same number of value tests
            codes = r.find_codes()
            mine = (len(codes), sum(len(t) for _vs, t in codes))
            if not msg and "codes" in report and mine != (int(report["codes"]), int(report["codetests"])):
                msg = f"{name}: the reader finds {mine[0]} coded choices with {mine[1]} value tests, the pass {report['codes']} with {report['codetests']}"
            if not msg and _PROOF:
                why = proof_problem(cnf, d)
                msg = f"{name}: {why}" if why else ""
        except (RuntimeError, subprocess.TimeoutExpired) as e:
            return name, f"{name}: {e}", 0, 0
    return name, msg, len(got), int(report.get("steps", 0))


def cmd_catalog(a):
    import z3_catalog as zc
    global _CAT, _SOLVER, _PROOF
    _SOLVER = a.solver
    _PROOF = a.proof
    _CAT = [rec for rec in zc.load(a.catalog)
            if any((i["kind"] == "order" and len(i["vars"]) >= 2) or (i["kind"] == "binary" and len(i["values"]) >= 3) for i in rec["inputs"])]
    jobs = [(i, seed) for i in range(len(_CAT)) for seed in (None, 1, 2)]
    with multiprocessing.get_context("fork").Pool(a.jobs) as pool:
        rows = pool.map(_catalog_job, jobs, chunksize=4)
    bad = [msg for _n, msg, _k, _s in rows if msg]
    for msg in bad[:20]:
        print("DIFF", msg)
    with_clauses = sum(1 for _n, _m, k, _s in rows if k)
    print(f"catalog: {len(rows)} readings, {with_clauses} where the pass adds clauses, {sum(k for _n, _m, k, _s in rows)} clauses in all "
          f"({sum(s for _n, _m, _k, s in rows)} of them proof steps), {len(bad)} differ from the reader"
          + (" or fail VeriPB" if a.proof else ""))
    sys.exit(1 if bad else 0)


def cmd_files(a):
    bad = 0
    for f in a.files:
        n_vars, _n, clauses, truncated = nr.read_cnf(f)
        if truncated:
            print(f"{f}: too long, not compared")
            bad += 1
            continue
        top = max((abs(l) for c in clauses for l in c), default=0)
        with tempfile.TemporaryDirectory() as d:
            src = f
            if str(f).endswith(".xz"):
                src = Path(d) / "f.cnf"
                write_cnf(src, max(n_vars, top), clauses)
            want, r = reader_clauses(max(n_vars, top), clauses)
            got, report = pass_clauses(a.solver, src, d)
            msg = compare(Path(f).name, want, got, report)
            if not msg and a.proof:
                msg = proof_problem(src, d)
        kinds = dict(Counter(k for _c, k in r.added))
        print(f"{Path(f).name}: reader {len(want)} clauses {kinds}, pass {len(got)} (work {report.get('work')}): {'the same' if not msg else 'DIFF ' + msg}")
        bad += bool(msg)
    sys.exit(1 if bad else 0)


def cmd_selftest(a):
    cl = [[1, -2], [2, -3],
          [-7, 1, 2, 3], [7, -1], [7, -2], [7, -3],
          [-5, 1, 2], [-5, 1, 3], [-5, 2, 3], [5, -1, -2], [5, -1, -3], [5, -2, -3],
          [-6, 1], [-6, 2], [-6, 3], [6, -1, -2, -3],
          [-4, 5], [-4, -6], [4, -5, 6]]
    want, _r = reader_clauses(7, cl)
    assert want == [[-5, 2], [5, -2], [6, -3], [-7, 1]], want
    with tempfile.TemporaryDirectory() as d:
        cnf = Path(d) / "f.cnf"
        write_cnf(cnf, 7, cl)
        got, report = pass_clauses(a.solver, cnf, d)
        assert compare("toy", want, got, report) == "", (got, report)
        assert report["numbers"] == "1" and report["values"] == "4x1" and report["equal"] == "4", report
        assert proof_problem(cnf, d) == "", proof_problem(cnf, d)
        # a proof with one step changed to a clause that does not follow is refused
        pbp = Path(d) / "proof.pbp"
        text = pbp.read_text()
        assert "rup 1 ~x5 1 x2 >= 1 ;" in text, text
        pbp.write_text(text.replace("rup 1 ~x5 1 x2 >= 1 ;", "rup 1 x5 1 x2 >= 1 ;"))
        assert proof_problem(cnf, d).startswith("FAIL"), proof_problem(cnf, d)
        # stand-in solvers that differ: one clause missing, another order, a
        # wrong count in the report, no report at all
        def fake(lines, report_line):
            sh = Path(d) / "fake.sh"
            body = "#!/bin/sh\nfor last; do :; done\n"
            body += f"printf '%s\\n' '{report_line}'\n" if report_line else ""
            body += "printf 'p cnf 7 0\\n" + "".join(" ".join(map(str, c)) + " 0\\n" for c in lines) + "' > \"$last.derived.cnf\"\n"
            sh.write_text(body)
            sh.chmod(0o755)
            return sh
        ok_line = "c numbers status=read numbers=1 values=4x1 added=4 equal=4 work=1"
        got2, rep2 = pass_clauses(fake(want, ok_line), cnf, d)
        assert compare("toy", want, got2, rep2) == ""
        got2, rep2 = pass_clauses(fake(want[:3], ok_line.replace("added=4", "added=3")), cnf, d)
        assert "reader 4 clauses, pass 3" in compare("toy", want, got2, rep2)
        got2, rep2 = pass_clauses(fake(want[::-1], ok_line), cnf, d)
        assert "another order" in compare("toy", want, got2, rep2)
        got2, rep2 = pass_clauses(fake(want, ok_line.replace("added=4", "added=5")), cnf, d)
        assert "reports 5 added" in compare("toy", want, got2, rep2)
        got2, rep2 = pass_clauses(fake(want, ok_line.replace("status=read", "status=stopped")), cnf, d)
        assert "status=stopped" in compare("toy", want, got2, rep2)
        # many ends on one bit: six variables that each need x at its top value and six
        # that need it at 0, no two of a kind on at once.  The ends are not followed, the
        # chain is the three bits, and the reader and the pass give the toy's four clauses
        fan = [list(c) for c in cl]
        for i in range(8, 14):
            fan.append([-i, 3])
            fan += [[-i, -k] for k in range(i + 1, 14)]
        for j in range(14, 20):
            fan.append([-j, -1])
            fan += [[-j, -k] for k in range(j + 1, 20)]
        want1, r1 = reader_clauses(19, fan)
        assert want1 == want and r1.notes["fan_links"] >= 12, (want1, dict(r1.notes))
        f1 = Path(d) / "fan.cnf"
        write_cnf(f1, 19, fan)
        got1, rep1 = pass_clauses(a.solver, f1, d)
        assert compare("fan", want1, got1, rep1) == "" and int(rep1["fanlinks"]) == r1.notes["fan_links"], (got1, rep1)
        # a helper on each end (8 needs the top value and sets 10; 9 needs 0 and sets
        # 11): the chain of five is not under a count, the stretch the count reads is
        core = [list(c) for c in cl] + [[-8, 3], [-8, 10], [-9, -1], [-9, 11]]
        want2, r2 = reader_clauses(11, core)
        assert want2 == want and r2.notes["core_cut"] == 1, (want2, dict(r2.notes))
        f2 = Path(d) / "core.cnf"
        write_cnf(f2, 11, core)
        got2, rep2 = pass_clauses(a.solver, f2, d)
        assert compare("core", want2, got2, rep2) == "" and rep2["corecuts"] == "1", (got2, rep2)
        # a formula that its own units refute: both say nothing, and that is a match
        dead = Path(d) / "dead.cnf"
        write_cnf(dead, 1, [[1], [-1]])
        want0, _r0 = reader_clauses(1, [[1], [-1]])
        got0, rep0 = pass_clauses(a.solver, dead, d)
        assert want0 == [] and got0 == [] and rep0["status"] == "root-conflict" and compare("dead", want0, got0, rep0) == "", (got0, rep0)
        assert "clauses were added" in compare("dead", [], [[1, 2]], rep0)
        try:
            pass_clauses(fake(want, ""), cnf, d)
            raise AssertionError("a run without a report line passed")
        except RuntimeError as e:
            assert "c numbers" in str(e)
    print("number_pass_check selftest ok")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("catalog", "files", "selftest"):
        s = sub.add_parser(name)
        s.add_argument("--solver", default=str(SOLVER))
        if name != "selftest":
            s.add_argument("--proof", action="store_true", help="also give the pass's proof to VeriPB")
        if name == "catalog":
            s.add_argument("--catalog", default=str(ROOT / "benchmarks" / "rl" / "z3_catalog.jsonl.xz"))
            s.add_argument("--jobs", type=int, default=8)
        if name == "files":
            s.add_argument("files", nargs="+")
    a = ap.parse_args(argv)
    {"catalog": cmd_catalog, "files": cmd_files, "selftest": cmd_selftest}[a.cmd](a)


if __name__ == "__main__":
    main()
