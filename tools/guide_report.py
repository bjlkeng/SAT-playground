#!/usr/bin/env python3
"""Tables of the nudge experiments (beads SAT-playground-d96.14.9 and d96.14.6).

A nudge is a solver option that leans the search toward the variables a reading
calls real unknowns (solver/13-kissat-rs/src/guide.rs).  An experiment runs each
nudge (an "arm") and the plain solver (the arm `base`) on the same formulas
("cells"), at the same seeds and under the same work limit.  This tool turns the
runs into one table and compares the arms.

  guide_report.py collect STAGE [STAGE ...] --dir RUNS --out results.tsv [--brief]
      read RUNS/run-STAGE/*.out with RUNS/budgets_STAGE.tsv and
      RUNS/run-STAGE/results.tsv into one table: a row a run (cell, group, kind,
      arm, seed, answer, work, limit, ...).  Says how many runs have a fault and
      which: no answer line, an exit code that is no answer, a model that was not
      checked or does not hold, an UNKNOWN before the work limit, and a cell with
      the answers SATISFIABLE and UNSATISFIABLE both.  The exit code is 1 with a
      fault.  --brief leaves out the columns exit, fault and guide (the nudge's
      own exit line); it is refused when a run has a fault.
  guide_report.py table [results.tsv] [--base base] [--arms A,B] [--groups G,..]
                        [--cells easy|hard] [--seeds 2,3] [--by-group] [--by-cell ARM]
      each arm against the base arm on the runs both have (same cell, same seed):
      solved runs, runs gained and lost, work PAR-2 as a share of the base's (a
      solved run costs its work, an unsolved one twice its limit), and on the
      pairs both solved the typical work ratio (geometric mean) with its spread.
  guide_report.py noise [results.tsv] [--arm base]
      the same numbers for one arm at one seed against itself at another seed:
      what two seeds alone differ by.
  guide_report.py boot [results.tsv] [--arms A,B] [--groups ..] [--cells easy|hard] [--seeds ..]
      each arm against the base arm with a range for luck: the cells are drawn
      again with replacement 2,000 times (a cell comes with all its seeds, since a
      cell's runs go together), and the range holds the middle 95% of the draws.
      For the solved runs' difference, for work PAR-2 as a share of the base's,
      and for the typical work ratio where both solved.  The draws are seeded, so
      a call prints the same ranges every time; another list of arms draws other
      samples, and a range can move by one run.
  guide_report.py selftest

Without a results file the tables are those of the first experiment,
benchmarks/rl/guide_nudges_2025.tsv.  The 2026 suite is not read (plan section
8: it is the holdout).
"""
import argparse
import csv
import math
import random
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / "benchmarks" / "rl" / "guide_nudges_2025.tsv"
FIELDS = ["cell", "group", "kind", "arm", "seed", "status", "solved", "work", "limit", "conflicts", "decisions", "wall", "exit",
          "model", "fault", "guide"]
BRIEF_OUT = ("exit", "fault", "guide")
ANSWERS = ("SATISFIABLE", "UNSATISFIABLE")


def cell_table(runs):
    """cell -> (group, easy or hard).  A cell is easy when the reference run in the
    cells table solved it in under 26G of work at one of its seeds."""
    out = {}
    for ln in (runs / "cells.tsv").read_text().splitlines():
        p = ln.split("\t")
        if len(p) < 8:
            continue
        work = p[7]
        if work == "-":
            easy = p[0] == "oisc-sll-8"
        else:
            easy = any(not w.endswith("T") and float(w) < 26 for w in work.split())
        out[p[0]] = (p[1], "easy" if easy else "hard")
    return out


def parse_out(path):
    status, work, conflicts, decisions, model, guide = "", "", "", "", "", ""
    try:
        with open(path, errors="replace") as f:
            for ln in f:
                if ln.startswith("s "):
                    status = ln[2:].strip()
                elif ln.startswith("c workclock "):
                    kv = dict(t.split("=", 1) for t in ln.split()[2:] if "=" in t)
                    work, conflicts, decisions = kv.get("work", ""), kv.get("conflicts", ""), kv.get("decisions", "")
                elif ln.startswith("c model "):
                    model = ln.split()[2]
                elif ln.startswith("c guide "):
                    guide = " ".join(ln.split()[2:])
    except OSError:
        pass
    return status, work, conflicts, decisions, model, guide


def fault_of(status, code, model, work, limit):
    """What is wrong with a run, or the empty string."""
    if status not in ANSWERS + ("UNKNOWN",):
        return "no answer line"
    if code not in ("0", "10", "20"):
        return f"exit {code}"
    if status == "SATISFIABLE" and model != "ok":
        return "model not checked" if not model else "model wrong"
    if status == "UNKNOWN" and (not work or int(work) < int(limit)):
        return "unknown before the limit"
    if not work:
        return "no work line"
    return ""


def collect(a):
    runs = Path(a.dir)
    cells = cell_table(runs)
    rows, faults = [], []
    for stage in a.stages:
        run = runs / f"run-{stage}"
        limits = dict(ln.split("\t")[:2] for ln in (runs / f"budgets_{stage}.tsv").read_text().splitlines() if ln.strip())
        done = {}
        if (run / "results.tsv").exists():
            for ln in (run / "results.tsv").read_text().splitlines():
                p = ln.split("\t")
                if len(p) >= 3:
                    done[p[0]] = (p[1], p[2])
        for name, limit in sorted(limits.items()):
            if name not in done:
                continue  # not run yet
            cell, arm, seed = name.rsplit(".", 2)
            seed = seed[1:]
            group, kind = cells[cell]
            status, work, conflicts, decisions, model, guide = parse_out(run / f"{name}.out")
            code, wall = done[name]
            fault = fault_of(status, code, model, work, limit)
            if fault:
                faults.append((name, fault))
            solved = int(status in ANSWERS and not fault)
            rows.append(dict(cell=cell, group=group, kind=kind, arm=arm, seed=seed, status=status or "-", solved=solved, work=work or "0",
                             limit=limit, conflicts=conflicts, decisions=decisions, wall=wall, exit=code, model=model, fault=fault, guide=guide))
    answers = {}
    for r in rows:
        if r["status"] in ANSWERS:
            answers.setdefault(r["cell"], set()).add(r["status"])
    for cell, got in sorted(answers.items()):
        if len(got) > 1:
            faults.append((cell, "both answers"))
    if a.brief and faults:
        print(f"{len(faults)} faults: no brief table is written")
    else:
        fields = [f for f in FIELDS if not (a.brief and f in BRIEF_OUT)]
        with open(a.out, "w", newline="") as f:
            w = csv.DictWriter(f, fields, delimiter="\t", lineterminator="\n", extrasaction="ignore")
            w.writeheader()
            w.writerows(rows)
        print(f"{len(rows)} runs in {a.out}; {len(faults)} faults")
    for name, fault in faults[:40]:
        print(f"  FAULT {name}: {fault}")
    return 1 if faults else 0


def load(path):
    with open(path, newline="") as f:
        return list(csv.DictReader(f, delimiter="\t"))


def cost(r):
    return float(r["work"]) if r["solved"] == "1" else 2.0 * float(r["limit"])


def log_ratio(b, x):
    """log of the arm's work over the base's on a pair both solved, else None."""
    if b["solved"] == "1" and x["solved"] == "1" and float(b["work"]) > 0 and float(x["work"]) > 0:
        return math.log(float(x["work"]) / float(b["work"]))
    return None


def compare(base, arm):
    """base, arm: {(cell, seed): row}.  Numbers over the keys both have."""
    keys = sorted(set(base) & set(arm))
    out = dict(pairs=len(keys), base=0, arm=0, gained=0, lost=0, par_base=0.0, par_arm=0.0, logs=[])
    for k in keys:
        b, x = base[k], arm[k]
        bs, xs = b["solved"] == "1", x["solved"] == "1"
        out["base"] += bs
        out["arm"] += xs
        out["gained"] += xs and not bs
        out["lost"] += bs and not xs
        out["par_base"] += cost(b)
        out["par_arm"] += cost(x)
        lg = log_ratio(b, x)
        if lg is not None:
            out["logs"].append(lg)
    return out


def line(name, c):
    logs = c["logs"]
    if logs:
        mean = sum(logs) / len(logs)
        sd = math.sqrt(sum((v - mean) ** 2 for v in logs) / (len(logs) - 1)) if len(logs) > 1 else 0.0
        se = sd / math.sqrt(len(logs))
        ratio = f"{math.exp(mean):6.3f} ({math.exp(mean - 2 * se):.2f}-{math.exp(mean + 2 * se):.2f}, {len(logs)} pairs)"
    else:
        ratio = "     -"
    par = c["par_arm"] / c["par_base"] if c["par_base"] else float("nan")
    return f"{name:18s} {c['pairs']:5d} {c['base']:6d} {c['arm']:6d}  +{c['gained']:<3d} -{c['lost']:<3d} {par:7.3f}   {ratio}"


HEAD = f"{'arm':18s} {'pairs':>5s} {'base':>6s} {'arm':>6s}  {'gained lost':9s} {'PAR-2':>7s}   work ratio where both solved (2 se range)"


def pick(rows, a):
    if a.groups:
        g = set(a.groups.split(","))
        rows = [r for r in rows if r["group"] in g]
    if a.cells:
        rows = [r for r in rows if r["kind"] == a.cells]
    if a.seeds:
        s = set(a.seeds.split(","))
        rows = [r for r in rows if r["seed"] in s]
    return rows


def by_arm(rows):
    arms = {}
    for r in rows:
        arms.setdefault(r["arm"], {})[(r["cell"], r["seed"])] = r
    return arms


def table(a):
    rows = pick(load(a.results), a)
    arms = by_arm(rows)
    if a.base not in arms:
        sys.exit(f"no runs of the base arm {a.base}")
    names = a.arms.split(",") if a.arms else [n for n in arms if n != a.base]
    print(HEAD)
    for n in names:
        if n in arms:
            print(line(n, compare(arms[a.base], arms[n])))
    if a.by_group:
        for g in sorted({r["group"] for r in rows}):
            print(f"\n== {g}")
            sub = by_arm([r for r in rows if r["group"] == g])
            if a.base not in sub:
                continue
            print(HEAD)
            for n in names:
                if n in sub:
                    print(line(n, compare(sub[a.base], sub[n])))
    if a.by_cell:
        print(f"\n== {a.by_cell} against {a.base}, by cell (work in G; * unsolved at the limit)")

        def show(r):
            return f"{float(r['work']) / 1e9:6.2f}{' ' if r['solved'] == '1' else '*'}"

        for cell in sorted({r["cell"] for r in rows}):
            parts = []
            for (c, seed), r in sorted(arms[a.base].items()):
                if c != cell or (c, seed) not in arms.get(a.by_cell, {}):
                    continue
                parts.append(f"s{seed} {show(r)} -> {show(arms[a.by_cell][(c, seed)])}")
            if parts:
                print(f"{cell:44s} " + "   ".join(parts))
    return 0


def noise(a):
    rows = pick(load(a.results), a)
    arm = by_arm(rows).get(a.arm, {})
    seeds = sorted({s for _, s in arm}, key=int)
    print(f"arm {a.arm}: one seed against another (the first is the 'base')")
    print(HEAD)
    for i in range(len(seeds)):
        for j in range(i + 1, len(seeds)):
            x = {(c, "x"): r for (c, s), r in arm.items() if s == seeds[i]}
            y = {(c, "x"): r for (c, s), r in arm.items() if s == seeds[j]}
            print(line(f"s{seeds[i]} v s{seeds[j]}", compare(x, y)))
    return 0


def boot_arm(base, arm, rng, draws=2000):
    """One arm against the base: the numbers, and for each the middle 95% of
    `draws` samples of the cells drawn again with replacement."""
    keys = sorted(set(base) & set(arm))
    cells = sorted({c for c, _ in keys})
    per = {}
    for c, s in keys:
        b, x = base[(c, s)], arm[(c, s)]
        bs, xs = b["solved"] == "1", x["solved"] == "1"
        per.setdefault(c, []).append((int(xs) - int(bs), cost(b), cost(x), log_ratio(b, x), int(bs), int(xs)))

    def stat(sample):
        d = sum(v[0] for c in sample for v in per[c])
        pb = sum(v[1] for c in sample for v in per[c])
        px = sum(v[2] for c in sample for v in per[c])
        logs = [v[3] for c in sample for v in per[c] if v[3] is not None]
        return d, (px / pb if pb else float("nan")), (math.exp(sum(logs) / len(logs)) if logs else float("nan"))

    here = stat(cells)
    samples = [stat([rng.choice(cells) for _ in cells]) for _ in range(draws)] if cells else []

    def middle(i):
        vals = sorted(v[i] for v in samples if v[i] == v[i])
        return (vals[int(0.025 * len(vals))], vals[int(0.975 * len(vals)) - 1]) if vals else (float("nan"), float("nan"))

    return dict(pairs=len(keys), base=sum(v[4] for c in cells for v in per[c]), arm=sum(v[5] for c in cells for v in per[c]),
                solved=(here[0],) + middle(0), par=(here[1],) + middle(1), ratio=(here[2],) + middle(2))


def boot(a):
    rows = pick(load(a.results), a)
    arms = by_arm(rows)
    if a.base not in arms:
        sys.exit(f"no runs of the base arm {a.base}")
    names = a.arms.split(",") if a.arms else [n for n in arms if n != a.base]
    rng = random.Random(20261123)
    print(f"{'arm':20s} {'pairs':>5s} {'base':>5s} {'arm':>5s}  {'solved, arm - base':24s} {'PAR-2 / base':24s} work ratio where both solved")
    for n in names:
        if n not in arms:
            continue
        b = boot_arm(arms[a.base], arms[n], rng)
        (d0, dl, dh), (p0, pl, ph), (r0, rl, rh) = b["solved"], b["par"], b["ratio"]
        print(f"{n:20s} {b['pairs']:5d} {b['base']:5d} {b['arm']:5d}  {d0:+4d} ({dl:+.0f} to {dh:+.0f}){'':8s} "
              f"{p0:6.3f} ({pl:.2f} to {ph:.2f}){'':5s} {r0:6.3f} ({rl:.2f} to {rh:.2f})")
    return 0


def selftest(_a):
    """A hand-made table: the numbers of a comparison, the faults of a collection."""
    def row(cell, arm, seed, solved, work, limit=100):
        return dict(cell=cell, group="g", kind="easy", arm=arm, seed=str(seed), status="SATISFIABLE" if solved else "UNKNOWN",
                    solved=str(int(solved)), work=str(work), limit=str(limit))

    rows = [row("a", "base", 1, True, 10), row("a", "x", 1, True, 20), row("b", "base", 1, False, 100), row("b", "x", 1, True, 50),
            row("c", "base", 1, True, 40), row("c", "x", 1, False, 100), row("d", "base", 1, True, 30), row("d", "x", 1, True, 30),
            row("e", "x", 1, True, 5)]
    arms = by_arm(rows)
    c = compare(arms["base"], arms["x"])
    assert (c["pairs"], c["base"], c["arm"], c["gained"], c["lost"]) == (4, 3, 3, 1, 1), c
    assert (c["par_base"], c["par_arm"]) == (10 + 200 + 40 + 30, 20 + 50 + 200 + 30), c
    assert len(c["logs"]) == 2 and abs(sum(c["logs"]) - math.log(2.0)) < 1e-12, c
    b = boot_arm(arms["base"], arms["x"], random.Random(1), draws=200)
    assert b["pairs"] == 4 and b["solved"][0] == 0 and b["solved"][1] <= 0 <= b["solved"][2], b
    assert abs(b["par"][0] - 300 / 280) < 1e-12 and abs(b["ratio"][0] - math.sqrt(2.0)) < 1e-12, b
    same = boot_arm(arms["base"], arms["base"], random.Random(1), draws=50)
    assert same["solved"] == (0, 0, 0) and same["par"] == (1.0, 1.0, 1.0), same
    # the faults
    assert fault_of("SATISFIABLE", "10", "ok", "5", "9") == ""
    assert fault_of("SATISFIABLE", "10", "", "5", "9") == "model not checked"
    assert fault_of("SATISFIABLE", "10", "WRONG", "5", "9") == "model wrong"
    assert fault_of("UNKNOWN", "0", "", "5", "9") == "unknown before the limit"
    assert fault_of("UNKNOWN", "0", "", "9", "9") == ""
    assert fault_of("", "0", "", "9", "9") == "no answer line"
    assert fault_of("UNSATISFIABLE", "137", "", "9", "9") == "exit 137"
    assert fault_of("UNSATISFIABLE", "20", "", "", "9") == "no work line"
    # a collection: two runs, the second answers the other way
    with tempfile.TemporaryDirectory() as tmp:
        runs = Path(tmp)
        (runs / "cells.tsv").write_text("toy\tg\tstem\t\tchoice\tchoice\t\t1.00 2.00\n")
        (runs / "budgets_T.tsv").write_text("toy.base.s1\t9\ntoy.x.s1\t9\ntoy.y.s1\t9\n")
        (runs / "run-T").mkdir()
        (runs / "run-T" / "results.tsv").write_text("toy.base.s1\t10\t0.1\ntoy.x.s1\t20\t0.1\n")
        (runs / "run-T" / "toy.base.s1.out").write_text("s SATISFIABLE\nc model ok\nc workclock work=5 conflicts=1 decisions=2\n")
        (runs / "run-T" / "toy.x.s1.out").write_text("c guide list=3 bumped=1\ns UNSATISFIABLE\nc workclock work=6 conflicts=1 decisions=2\n")
        out = runs / "out.tsv"
        args = argparse.Namespace(dir=str(runs), stages=["T"], out=str(out), brief=False)
        assert collect(args) == 1                       # both answers on one cell
        got = load(out)
        assert [(r["arm"], r["status"], r["solved"], r["kind"]) for r in got] == [("base", "SATISFIABLE", "1", "easy"), ("x", "UNSATISFIABLE", "1", "easy")], got
        assert got[1]["guide"] == "list=3 bumped=1", got
        args.brief = True
        out.unlink()
        assert collect(args) == 1 and not out.exists()  # a fault: no brief table
        (runs / "run-T" / "toy.x.s1.out").write_text("s SATISFIABLE\nc model ok\nc workclock work=6 conflicts=1 decisions=2\n")
        (runs / "run-T" / "results.tsv").write_text("toy.base.s1\t10\t0.1\ntoy.x.s1\t10\t0.1\n")
        assert collect(args) == 0
        assert "guide" not in load(out)[0] and "fault" not in load(out)[0]
    print("selftest ok")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("collect")
    c.add_argument("stages", nargs="+")
    c.add_argument("--dir", required=True, help="the experiment's folder: cells.tsv, budgets_STAGE.tsv, run-STAGE/")
    c.add_argument("--out", required=True)
    c.add_argument("--brief", action="store_true")
    for name in ("table", "noise", "boot"):
        t = sub.add_parser(name)
        t.add_argument("results", nargs="?", default=str(DEFAULT))
        t.add_argument("--groups", default="")
        t.add_argument("--cells", default="", choices=("", "easy", "hard"))
        t.add_argument("--seeds", default="")
        if name == "table":
            t.add_argument("--base", default="base")
            t.add_argument("--arms", default="")
            t.add_argument("--by-group", action="store_true")
            t.add_argument("--by-cell", default="")
        elif name == "boot":
            t.add_argument("--base", default="base")
            t.add_argument("--arms", default="")
        else:
            t.add_argument("--arm", default="base")
    sub.add_parser("selftest")
    a = ap.parse_args()
    sys.exit({"collect": collect, "table": table, "noise": noise, "boot": boot, "selftest": selftest}[a.cmd](a))


if __name__ == "__main__":
    main()
