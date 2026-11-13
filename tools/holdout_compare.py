#!/usr/bin/env python3
"""The second look at the 2026 holdout (bead SAT-playground-1v2.11) against the first.

  python3 tools/holdout_compare.py NEW_RUN [--old OLD_RUN] [--kissat LABEL=DIR ...]

NEW_RUN is the feature_ablation run of 2026-11-12 with arms `default` (the shipped set) and
`symparity` (the same binary with the passes promoted since 2026-10-10 off); OLD_RUN the first
look of 2026-10-10 with arms `both` (symmetry and parity on) and `stock`. Each --kissat is a
tools/run_kissat_full.sh folder (results.csv); without one, the kissat run of 2026-08-29.

It prints the solved count of every arm, contradictions across all of them, the pairs (each
gain and loss with its family, times and conflicts, and what the passes did on that cell in the
default arm's kept lines), a host check (the wall ratio on cells where two runs did the same
search: the same conflicts, propagations and decisions), a per-family table and what the passes
did over the 400 cells. A solve is what tools/rl_sweep_report.py counts as one: a failed check
or an answer cut short is never a solve. The checks (failed rows, contradictions) read every row
of every arm; the counts and PAR-2 read the cells every arm has. It reads result files only,
never a CNF. `--self-test` runs it on a small made-up set of runs and checks the checks.
"""
from __future__ import annotations

import argparse
import csv
import re
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ROOT = TOOLS.parent
sys.path.insert(0, str(TOOLS))
import rl_sweep_report as rsr  # noqa: E402

OLD = ROOT / "log/abtest-struct-holdout-2026-2026-10-09-15-51-57"
KISSAT_AUG = ROOT / "log/kissat-full-20260829-202806"
NOTHING = ("no ", "nothing", "skipped")


def load_tsv(p: Path) -> dict:
    with open(p) as f:
        return {r["instance"]: r for r in csv.DictReader(f, delimiter="\t")}


KISSAT_RC = {"SAT": "10", "UNSAT": "20"}   # kissat's own exit code for each answer


def load_kissat(d: Path) -> dict:
    """A run_kissat_full.sh results.csv as harness rows, with the exit code where
    rl_sweep_report reads it. An answer with any other exit code than kissat's own for it
    becomes an answer followed by an abnormal exit (never a solve, and a failure unless the
    exit was a kill); an UNKNOWN with a nonzero exit code becomes UNKNOWN_RC<code>."""
    out = {}
    with open(d / "results.csv") as f:
        for r in csv.DictReader(f):
            res, rc = r["result"].upper(), (r.get("exit_code") or "").strip()
            note = ""
            if res in KISSAT_RC and rc != KISSAT_RC[res]:
                note = f"{rsr.INCOMPLETE}{rc or 'missing'} (kissat exits {KISSAT_RC[res]} after {res})"
            elif res == "UNKNOWN" and rc not in ("", "0"):
                res = f"UNKNOWN_RC{rc}"
            out[r["instance"]] = {"instance": r["instance"], "result": res, "time_s": r["time_s"],
                                  "timeout": r["timeout"], "verified": "", "note": note,
                                  "conflicts": "NA", "propagations": "NA", "decisions": "NA", "work": "NA"}
    return out


def families() -> dict:
    with open(ROOT / "benchmarks/rl/families.tsv") as f:
        return {r["stem"]: r["family"] for r in csv.DictReader(f, delimiter="\t") if r["suite"] == "sat-comp-2026"}


def solved(r) -> bool:
    return r is not None and rsr.is_solved(r)


def ans(r) -> str:
    return r["result"].upper()[:3] if solved(r) else "-"


def wall(r) -> float:
    """Wall PAR-2 cost: the time of a solve, twice the cell's own limit otherwise."""
    return float(r["time_s"]) if solved(r) else 2 * float(r["timeout"])


def short(stem: str) -> str:
    return stem.split("-", 1)[1] if "-" in stem else stem


def counted(v) -> bool:
    return v not in (None, "", "NA")


def same_search(a, b) -> bool:
    return all(counted(a.get(k)) and a.get(k) == b.get(k) for k in ("conflicts", "propagations", "decisions"))


def kept_notes(run: Path, arm: str) -> dict:
    """stem -> what the passes printed: {'notes': {step: text}, 'summary', 'walk', 'card', 'gauss'}."""
    out = defaultdict(lambda: {"notes": {}, "summary": "", "walk": "", "card": {}, "gauss": {}})
    p = run / arm / "kept.tsv"
    if not p.exists():
        return out
    for ln in open(p):
        parts = ln.rstrip("\n").split("\t", 5)
        if len(parts) < 6:
            continue
        stem, line = parts[0], parts[5]
        d = out[stem]
        if line.startswith("c structure pass: "):
            body = line[len("c structure pass: "):]
            m = re.match(r"(\d+) derived clauses? \((\d+) units?\), (refuted|equisatisfiable extension)", body)
            if m:
                d["summary"] = f"{m.group(1)} clauses, {m.group(2)} units, {m.group(3)}"
            elif ": " in body:
                name, text = body.split(": ", 1)
                d["notes"][name] = text
        elif line.startswith("c freewalk "):
            kv = dict(t.split("=", 1) for t in line.split()[2:] if "=" in t)
            d["walk"] = f"{kv.get('test', '?')}/{kv.get('status', '?')}" + (
                f"/{kv['reading']}" if kv.get("status") == "unread" and "reading" in kv else "")
        elif line.startswith("c card "):
            d["card"] = dict(t.split("=", 1) for t in line.split()[2:] if "=" in t)
        elif line.startswith("c gauss "):
            d["gauss"] = dict(t.split("=", 1) for t in line.split()[2:] if "=" in t)
    return out


def num(d: dict, k: str) -> int:
    try:
        return int(d.get(k, "0") or 0)
    except ValueError:
        return 0


def acting(d) -> list[str]:
    """What the passes did on one cell, from its kept lines (empty: nothing found)."""
    acts = [f"{name}: {text[:100]}" for name, text in d["notes"].items() if not text.startswith(NOTHING)]
    if d["summary"]:
        acts.append(f"[{d['summary']}]")
    if d["walk"] and "/declined" not in d["walk"] and "/unread" not in d["walk"]:
        acts.append(f"walk {d['walk']}")
    c = d["card"]
    if any(num(c, k) for k in ("units", "forced", "violations")):
        acts.append(f"card counts={c.get('counts')} units={c.get('units')} forced={c.get('forced')} "
                    f"violations={c.get('violations')}")
    g = d["gauss"]
    if num(g, "rows"):
        acts.append(f"gauss rows={g.get('rows')} conflicts={g.get('conflicts')} units={g.get('units')}")
    return acts


def pair(name: str, A: dict, B: dict, fam: dict, notes=None, show=True):
    common = sorted(set(A) & set(B))
    gains = [s for s in common if solved(A[s]) and not solved(B[s])]
    losses = [s for s in common if solved(B[s]) and not solved(A[s])]
    sa, sb = sum(solved(A[s]) for s in common), sum(solved(B[s]) for s in common)
    pa, pb = sum(wall(A[s]) for s in common), sum(wall(B[s]) for s in common)
    print(f"\n## {name}: {sa} v {sb} on {len(common)} cells, +{len(gains)} -{len(losses)}, "
          f"wall PAR-2 {pa:,.0f} v {pb:,.0f} ({pa / pb:.4f}x)")
    if not show:
        return
    for tag, lst, X, Y in (("+", gains, A, B), ("-", losses, B, A)):
        for s in lst:
            x, y = X[s], Y[s]
            how = ""
            if counted(x.get("conflicts")) and counted(y.get("conflicts")):
                how = ("stopped short of its conflicts" if int(y["conflicts"]) < int(x["conflicts"])
                       else "stopped past them: another search")
            print(f"  {tag} {short(s)[:60]:60} {fam.get(s, '?'):18} {ans(x)} {float(x['time_s']):7.0f}s v "
                  f"{y['result'][:7]:7} {float(y['time_s']):5.0f}s conf {x.get('conflicts')}/{y.get('conflicts')} {how}")
            if notes is not None:
                for a in acting(notes[s]):
                    print(f"        {a}")


def quantiles(rat: list) -> str:
    if len(rat) < 10:
        return f"{len(rat)} cells, too few"
    q = statistics.quantiles(rat, n=10)
    return f"median {statistics.median(rat):.3f}, p10 {q[0]:.3f}, p90 {q[-1]:.3f}"


def host_check(name: str, A: dict, B: dict):
    common = sorted(set(A) & set(B))
    both = [s for s in common if solved(A[s]) and solved(B[s])]
    same = [s for s in both if same_search(A[s], B[s])]
    rat = [float(A[s]["time_s"]) / float(B[s]["time_s"]) for s in same
           if min(float(A[s]["time_s"]), float(B[s]["time_s"])) >= 10]
    print(f"\n## host check, {name}: {len(both)} cells both solve, {len(same)} with the same search "
          f"(conflicts, propagations, decisions); wall ratio on the {len(rat)} of them at 10 s or more: {quantiles(rat)}")
    tt = [s for s in common if not solved(A[s]) and not solved(B[s])
          and counted(A[s].get("conflicts")) and counted(B[s].get("conflicts")) and int(B[s]["conflicts"]) > 0]
    if len(tt) >= 10:
        cr = [int(A[s]["conflicts"]) / int(B[s]["conflicts"]) for s in tt]
        print(f"   cells both stop at the limit: {len(tt)}; conflicts reached, ratio median {statistics.median(cr):.3f}")


def time_check(name: str, A: dict, B: dict):
    """Two runs without counters on one side (kissat): the time ratio where both solve."""
    both = [s for s in sorted(set(A) & set(B)) if solved(A[s]) and solved(B[s])]
    rat = [float(A[s]["time_s"]) / float(B[s]["time_s"]) for s in both
           if min(float(A[s]["time_s"]), float(B[s]["time_s"])) >= 10]
    print(f"\n## time check, {name}: {len(both)} cells both solve; wall ratio on the {len(rat)} at 10 s or more: "
          f"{quantiles(rat)}" + (f", geomean {statistics.geometric_mean(rat):.3f}" if rat else ""))


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    if argv == ["--self-test"]:
        return self_test()
    ap.add_argument("new", help="the 2026-11-12 run folder (arms default and symparity)")
    ap.add_argument("--old", default=str(OLD), help="the 2026-10-10 run folder (arms both and stock)")
    ap.add_argument("--kissat", action="append", default=[], metavar="LABEL=DIR",
                    help="a run_kissat_full.sh folder; repeatable (default: the run of 2026-08-29)")
    args = ap.parse_args(argv)
    new, old = Path(args.new), Path(args.old)
    kissats = [k.split("=", 1) for k in args.kissat] or [["kissat (08-29)", str(KISSAT_AUG)]]
    fam = families()
    arms = {
        "default (11-12)": load_tsv(new / "default/results.tsv"),
        "symparity (11-12)": load_tsv(new / "symparity/results.tsv"),
        "both (10-10)": load_tsv(old / "both/results.tsv"),
        "stock (10-10)": load_tsv(old / "stock/results.tsv"),
    }
    for label, d in kissats:
        arms[label] = load_kissat(Path(d))
    kissat_labels = [label for label, _ in kissats]
    notes, sym_notes = kept_notes(new, "default"), kept_notes(new, "symparity")
    every = sorted(set.union(*(set(a) for a in arms.values())))
    cells = sorted(set.intersection(*(set(a) for a in arms.values())))
    print(f"# cells in every arm: {len(cells)}")
    short_arms = {k: len(every) - len(A) for k, A in arms.items() if len(A) < len(every)}
    if short_arms:
        print(f"# WARNING: of the {len(every)} cells in some arm, missing: {short_arms}. The counts and PAR-2 "
              f"use the {len(cells)} in every arm; the checks use every row.")
    print("\n| arm | solved | SAT | UNSAT | wall PAR-2 | solves not checked | failed rows |")
    print("|---|---|---|---|---|---|---|")
    for k, A in arms.items():
        sv = [s for s in cells if solved(A[s])]
        c = Counter(ans(A[s]) for s in sv)
        unchecked = "no proofs" if k in kissat_labels else sum(1 for s in sv if A[s].get("verified", "").strip() != "ok")
        bad = [s for s in sorted(A) if rsr.failed(A[s])]
        print(f"| {k} | {len(sv)} | {c['SAT']} | {c['UNS']} | {sum(wall(A[s]) for s in cells):,.0f} | {unchecked} | {len(bad)} |")
        for s in bad:
            print(f"   FAILED {k} {short(s)}: {rsr.failed(A[s])}")
    clash = [s for s in every if len({ans(A[s]) for A in arms.values() if s in A and solved(A[s])}) > 1]
    print(f"\ncontradictions (SAT in one arm, UNSAT in another) over all {len(arms)} arms: {len(clash)}")
    for s in clash:
        print("  CLASH", short(s), {k: ans(A[s]) for k, A in arms.items() if s in A})
    D, S, Bo, St = (arms[k] for k in ("default (11-12)", "symparity (11-12)", "both (10-10)", "stock (10-10)"))
    pair("default v symparity (paired, same binary, 11-12)", D, S, fam, notes)
    pair("symparity (11-12) v both (10-10): the last look's candidate re-run", S, Bo, fam, sym_notes)
    pair("both (10-10) v stock (10-10): the last look", Bo, St, fam, show=False)
    pair("default (11-12) v stock (10-10)", D, St, fam, notes)
    for label in kissat_labels:
        pair(f"default (11-12) v {label}", D, arms[label], fam, notes)
        pair(f"stock (10-10) v {label}", St, arms[label], fam, show=False)
    host_check("symparity (11-12) v both (10-10)", S, Bo)
    host_check("default v symparity (11-12)", D, S)
    for label in kissat_labels:
        time_check(f"stock (10-10) v {label}", St, arms[label])
        time_check(f"symparity (11-12) v {label}", S, arms[label])
    print("\n## per family (where some arm differs in solved)")
    print("| family | cells | " + " | ".join(arms) + " |")
    print("|---|---|" + "---|" * len(arms))
    byfam = defaultdict(list)
    for s in cells:
        byfam[fam.get(s, "?")].append(s)
    rows = []
    for f, ss in byfam.items():
        counts = [sum(solved(A[s]) for s in ss) for A in arms.values()]
        if len(set(counts)) > 1:
            rows.append((f, len(ss), counts))
    for f, n, counts in sorted(rows, key=lambda x: (-(x[2][0] - x[2][1]), -(x[2][0] - x[2][3]), x[0])):
        print(f"| {f} | {n} | " + " | ".join(str(c) for c in counts) + " |")
    print("\n## what the passes did in the default arm (cells)")
    act = Counter()
    for s in cells:
        d = notes[s]
        for name, text in d["notes"].items():
            if not text.startswith(NOTHING):
                act[f"step {name}"] += 1
        if d["summary"]:
            act["derived clauses: " + d["summary"].split(", ")[-1]] += 1
        if d["walk"]:
            act[f"walk {d['walk']}"] += 1
        if num(d["card"], "counts"):
            act["card: counts read"] += 1
        if any(num(d["card"], k) for k in ("units", "forced", "violations")):
            act["card: units, forced or violations at the root"] += 1
        if num(d["gauss"], "rows"):
            act["gauss: a matrix"] += 1
    for k, v in sorted(act.items()):
        print(f"  {k}: {v}")
    return 0


def self_test() -> int:
    """Made-up runs: c1 agrees everywhere; c2 is SAT in default and UNSAT in symparity and missing
    from the kissat run; c3 is a kissat SAT that exited 139 (a crash after the answer)."""
    import contextlib
    import io
    import tempfile
    head = "instance\tseed\tresult\ttime_s\ttimeout\tverified\tnote\tconflicts\tpropagations\tdecisions\twork\n"

    def row(stem, res, ver="ok"):
        return f"{stem}\t0\t{res}\t5.0\t3600\t{ver}\t\t10\t20\t30\t100\n"

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        arms = {"new/default": [("c1", "SATISFIABLE"), ("c2", "SATISFIABLE"), ("c3", "TIMEOUT")],
                "new/symparity": [("c1", "SATISFIABLE"), ("c2", "UNSATISFIABLE"), ("c3", "TIMEOUT")],
                "old/both": [("c1", "SATISFIABLE"), ("c2", "TIMEOUT"), ("c3", "TIMEOUT")],
                "old/stock": [("c1", "SATISFIABLE"), ("c2", "TIMEOUT"), ("c3", "TIMEOUT")]}
        for arm, rows in arms.items():
            (tmp / arm).mkdir(parents=True)
            (tmp / arm / "results.tsv").write_text(
                head + "".join(row(s, r, "ok" if r != "TIMEOUT" else "") for s, r in rows))
        (tmp / "k").mkdir()
        (tmp / "k" / "results.csv").write_text(
            "instance,result,time_s,timeout,exit_code\nc1,SAT,4.0,3600,10\nc3,SAT,6.0,3600,139\n")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            main([str(tmp / "new"), "--old", str(tmp / "old"), "--kissat", f"k={tmp / 'k'}"])
        text = out.getvalue()
    checks = {
        "c2 is missing from the kissat run, and the warning says so": "missing: {'k': 1}" in text,
        "the contradiction on c2 is found though kissat lacks c2": "contradictions (SAT in one arm, UNSAT in another) over all 5 arms: 1" in text and "CLASH c2" in text,
        "the kissat SAT that exited 139 is a failed row": "FAILED k c3" in text,
        "and it is not a solve": "| k | 1 | 1 | 0 |" in text,
    }
    for what, ok in checks.items():
        print(("ok    " if ok else "FAIL  ") + what)
    if not all(checks.values()):
        print(text)
        return 1
    print("self-test ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
