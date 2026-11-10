#!/usr/bin/env python3
"""Score the free-choice walk's 400-cell run (bead SAT-playground-d96.2.5).

  free_walk_score.py RUN_DIR... [--timeout 1800] [--families] [--tests] [--cells]

A RUN_DIR is a feature_ablation.py --arm run (log/abtest-...) whose arms are named
off<seed> and <placement><seed> (turn0, first1, turn10-0, ...; the digits at the
end are the seed, the rest names the placement). Every arm has results.tsv; the
walk arms also kept.tsv, from --keep-lines 'c freewalk ' --keep-lines 'c freeread '
--keep-lines 'c workclock '. With several runs the arms of all are scored together
(their names must differ), so a run without off arms (stage 2) is scored against
another run's; its searches are the same as they would have been side by side,
but its times come from another window of the host.

Per arm, and per placement over its seeds: solved (SAT and UNSAT, how many models
and proofs the checkers passed, how many answers timed out in the checker), how
many answers the walk itself found, the cells gained and lost against off at the
same seed, wall PAR-2 (a solved cell costs its time, an unsolved one twice the
time limit) and tick PAR-2 (a solved cell costs its work W, an unsolved one twice
the W it reached before the kill), each against off at the same seed. Under a wall
limit the unsolved part of tick PAR-2 moves with the host, so the work is also
compared like for like: W summed over the cells both the arm and off solve. And
the reading's share of the arm's tick PAR-2.

--families: solved per family for every arm, where the arms differ.

--tests: the file tests, scored as the bead says. A test admits a cell by the
reading's shape (the `c freewalk` line of the walk arm; the record of d96.2.4.1,
benchmarks/rl/freeread_2025.tsv, for a run that printed none). For a test and a
walk arm, take the arm's result on the cells the test admits and off's result at
the same seed on the others, adding the reading's time and work measured on that
cell in the walk arm: with the walk on, the reading runs on every cell, and on a
cell the test turns down the search is the stock one (tests/freewalk.rs). A cell
off solved within the time limit but not once the reading's time is added counts
as unsolved. The tests: strict (groups, none left out for overlapping, no free
variable, a definition or gate), loose (groups and a definition or gate), broad
(groups or a definition or gate: the arms as run), and the broad test with a limit
on the census's choice share ((groups + free) / variables present), free share
(free / present) or share of constraints false at a random start (the mean of 64
starts over the constraints), a few limits each.

--cells: one line per cell where any arm differs from off in solved.

The 2026 suite is not read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import rl_sweep_report as rsr  # noqa: E402  (the harness's exit classes)
FAMILIES = ROOT / "benchmarks" / "rl" / "families.tsv"
READ_RECORD = ROOT / "benchmarks" / "rl" / "freeread_2025.tsv"
# the record counted one unit a tick; the solver counts 16 since d96.2.5
RECORD_UNITS_PER_TICK = 16
SOLVED = ("SATISFIABLE", "UNSATISFIABLE")


def _num(x, default=None):
    try:
        return float(x)
    except (TypeError, ValueError):
        return default


def kv(line: str) -> dict:
    """The key=value tokens of a solver line."""
    return dict(t.split("=", 1) for t in line.split() if "=" in t)


def seed_of(arm: str) -> str:
    m = re.search(r"(\d+)$", arm)
    return m.group(1) if m else ""


def placement_of(arm: str) -> str:
    return arm[: len(arm) - len(seed_of(arm))]


class Run:
    """feature_ablation runs: per arm, per cell, the result row and kept lines."""

    def __init__(self, paths: list[Path]):
        where = {}
        for path in paths:
            arms = [p.name for p in path.iterdir() if (p / "results.tsv").is_file()]
            if not arms:
                raise SystemExit(f"{path}: no arm with results.tsv")
            for a in arms:
                if a in where:
                    raise SystemExit(f"arm {a} is in both {where[a]} and {path}")
                where[a] = path
        self.arms = sorted(where)
        self.rows = {}    # arm -> stem -> row
        self.walk = {}    # arm -> stem -> c freewalk fields
        self.read = {}    # arm -> stem -> c freeread fields
        for a in self.arms:
            path = where[a]
            with open(path / a / "results.tsv") as fh:
                self.rows[a] = {r["instance"]: r for r in csv.DictReader(fh, delimiter="\t")}
            self.walk[a], self.read[a] = {}, {}
            kept = path / a / "kept.tsv"
            if kept.is_file():
                with open(kept) as fh:
                    for ln in fh:
                        parts = ln.rstrip("\n").split("\t", 5)
                        if len(parts) < 6:
                            continue
                        stem, line = parts[0], parts[5]
                        if line.startswith("c freewalk "):
                            self.walk[a][stem] = kv(line)
                        elif line.startswith("c freeread "):
                            self.read[a][stem] = kv(line)
        cells = [set(self.rows[a]) for a in self.arms]
        self.cells = sorted(set.intersection(*cells))
        if any(c != set(self.cells) for c in cells):
            print(f"warning: the arms ran different cells; scoring the {len(self.cells)} all ran", file=sys.stderr)
        self.walk_arms = [a for a in self.arms if self.off_for(a) not in (None, a)]
        self.placements = sorted({placement_of(a) for a in self.walk_arms})

    def off_for(self, arm: str) -> str | None:
        off = "off" + seed_of(arm)
        return off if off in self.rows else None

    def arms_of(self, placement: str) -> list[str]:
        return [a for a in self.walk_arms if placement_of(a) == placement]


# A SAT or UNSAT answer counts only when its check passed or ran out of time in the
# checker (both counted apart). Every stop is classed as rl_sweep_report.py classes
# it: an honest stop (a timeout, a kill, a memory abort, an answer cut short by the
# time limit) is unsolved; a failed or missing check, a crash, a premature UNKNOWN,
# an answer not checked, or SAT against UNSAT on one cell is a correctness failure
# (CLAUDE.md "Correctness is absolute") and fails the comparison.
COUNTED = ("ok", "checker-timeout")


def problem(r: dict) -> str:
    why = rsr.failed(r)
    if why:
        return why
    if r["result"] in SOLVED and not rsr.incomplete(r) and r["verified"] not in COUNTED:
        return f"answer not checked (verified={r['verified']})"
    return ""


def solved(r: dict) -> bool:
    return r["result"] in SOLVED and not rsr.incomplete(r) and not problem(r)


def failures(run: "Run") -> list[str]:
    """The correctness failures of every arm, and the cells answered SAT in one arm
    and UNSAT in another (any seed)."""
    bad, answers = [], defaultdict(dict)
    for a in run.arms:
        for s, r in sorted(run.rows[a].items()):
            why = problem(r)
            if why:
                bad.append(f"{a} {s}: {why} (reported {r['result']})")
            elif solved(r):
                answers[s][a] = r["result"]
    for s, by in sorted(answers.items()):
        if len(set(by.values())) > 1:
            bad.append(f"{s}: SAT and UNSAT across arms: " + ", ".join(f"{a}={v}" for a, v in sorted(by.items())))
    return bad


def reading_cost(run: Run, arm: str, stem: str, record: dict) -> tuple[float, float]:
    """The reading's (seconds, ticks) on a cell in a walk arm: from its `c freeread`
    line in this run, else the record of d96.2.4.1 (freeread_2025.tsv)."""
    fr = run.read[arm].get(stem)
    if fr is not None:
        return _num(fr.get("ms"), 0.0) / 1000.0, _num(fr.get("work"), 0.0)
    rec = record.get(stem)
    if rec is not None:
        return _num(rec.get("ms"), 0.0) / 1000.0, _num(rec.get("work"), 0.0) / RECORD_UNITS_PER_TICK
    return 0.0, 0.0


class Cell:
    """A scored result: solved, seconds, work W (None when the solver printed none),
    the reading's work inside W, and whether the walk found the answer."""
    __slots__ = ("solved", "secs", "work", "read_work", "walk_found", "result", "verified")

    def __init__(self, solved, secs, work, read_work, walk_found, result, verified):
        self.solved, self.secs, self.work, self.read_work = solved, secs, work, read_work
        self.walk_found, self.result, self.verified = walk_found, result, verified


def as_cell(run: Run, arm: str, stem: str) -> Cell:
    r = run.rows[arm][stem]
    fw = run.walk[arm].get(stem, {})
    fr = run.read[arm].get(stem, {})
    return Cell(solved(r), float(r["time_s"]), _num(r["work"]), _num(fr.get("work"), 0.0),
                fw.get("status") == "found" and r["result"] == "SATISFIABLE", r["result"], r["verified"])


SHAPE = ("read_groups", "overlap_groups", "read_free", "defs", "gates")


def shape(run: Run, arm: str, stem: str, record: dict) -> dict | None:
    """The reading's shape of a cell in a walk arm, whatever became of the walk after
    it (built, declined, out of budget while building): from the arm's `c freewalk`
    line, else from the record of d96.2.4.1 (a run that printed no line, or one whose
    budget ran out before the shape was looked at, which prints it as zeros). `read`
    is whether the reading gave a shape, `broad` whether the broad test admits it.
    The census's shares come from the record. None when neither has the cell."""
    rec = record.get(stem, {})
    present, cons, sim_false = _num(rec.get("present")), _num(rec.get("constraints")), _num(rec.get("sim_false"))
    shares = {"choice_share": _num(rec.get("choice_share")),
              "false_share": sim_false / cons if cons and sim_false is not None else (0.0 if cons == 0 else None)}
    fw = run.walk[arm].get(stem)
    unread = fw is not None and fw.get("status") == "unread"
    looked = fw is not None and not unread and not (
        fw.get("status") == "budget" and not any(_num(fw.get(k), 0) for k in SHAPE))
    if looked:
        g = lambda k: int(_num(fw.get(k), 0))
        sh = {"read": True, "groups": g("read_groups"), "overlap": g("overlap_groups"), "free": g("read_free"),
              "defs": g("defs"), "gates": g("gates")}
    elif rec and not unread:
        g = lambda k: int(_num(rec.get(k), 0))
        sh = {"read": rec.get("status") == "read", "groups": g("groups"), "overlap": g("overlap_groups"),
              "free": g("free"), "defs": g("defs"), "gates": g("gates")}
    elif unread:
        sh = {"read": False, "groups": 0, "overlap": 0, "free": 0, "defs": 0, "gates": 0}
    else:
        return None
    sh["broad"] = sh["read"] and (sh["groups"] > 0 or sh["defs"] > 0 or sh["gates"] > 0)
    sh.update(shares)
    sh["free_share"] = sh["free"] / present if present else None
    return sh


def mixed_cell(run: Run, arm: str, stem: str, admits, record: dict, timeout: int) -> Cell:
    """A test's result on a cell: the arm's where the test admits it, else off's with
    the reading's time and work added."""
    sh = shape(run, arm, stem, record)
    if sh is not None and admits(sh):
        return as_cell(run, arm, stem)
    off = as_cell(run, run.off_for(arm), stem)
    secs, work = reading_cost(run, arm, stem, record)
    t = off.secs + secs
    ok = off.solved and t <= timeout
    w = None if off.work is None else off.work + work
    return Cell(ok, t if ok else max(t, timeout), w, work, False, off.result if ok else "TIMEOUT", off.verified)


class Score:
    """Sums over cells (one arm, or a placement over its seeds) against off."""

    def __init__(self):
        self.n = defaultdict(float)

    def add(self, c: Cell, o: Cell, timeout: int) -> None:
        n = self.n
        n["na"] += c.work is None
        if c.solved:
            n["solved"] += 1
            n["sat" if c.result == "SATISFIABLE" else "unsat"] += 1
            if c.verified == "ok":
                n["models_ok" if c.result == "SATISFIABLE" else "proofs_ok"] += 1
            elif c.verified == "checker-timeout":
                n["checker_timeout"] += 1
            n["walk_found"] += c.walk_found
        n["plus"] += c.solved and not o.solved
        n["minus"] += o.solved and not c.solved
        n["off_solved"] += o.solved
        n["wall"] += c.secs if c.solved else 2 * timeout
        n["off_wall"] += o.secs if o.solved else 2 * timeout
        if c.work is not None and o.work is not None:
            n["read"] += (1 if c.solved else 2) * c.read_work
            n["tick"] += c.work if c.solved else 2 * c.work
            n["off_tick"] += o.work if o.solved else 2 * o.work
            if c.solved and o.solved:
                n["both_w"] += c.work
                n["off_both_w"] += o.work
                n["both"] += 1

    def __getitem__(self, k):
        return self.n[k]


def ratio(a: float, b: float) -> str:
    return f"{a / b:.3f}" if b else "-"


def score_arm(run: Run, arm: str, timeout: int, cell_fn=None) -> Score:
    off = run.off_for(arm)
    sc = Score()
    for s in run.cells:
        c = cell_fn(s) if cell_fn else as_cell(run, arm, s)
        sc.add(c, as_cell(run, off, s), timeout)
    return sc


def merge(scores: list[Score]) -> Score:
    m = Score()
    for s in scores:
        for k, v in s.n.items():
            m.n[k] += v
    return m


ARM_HEAD = ("| Arm | Solved | Off solved | + | - | SAT | UNSAT | Models ok | Proofs ok | Checker timeouts "
            "| Walk found | Wall PAR-2 x off | Tick PAR-2 x off | Both solved | W on both x off | Reading share |")


def arm_row(name: str, sc: Score) -> str:
    row = [name, int(sc["solved"]), int(sc["off_solved"]), int(sc["plus"]), int(sc["minus"]), int(sc["sat"]),
           int(sc["unsat"]), int(sc["models_ok"]), int(sc["proofs_ok"]), int(sc["checker_timeout"]),
           int(sc["walk_found"]), ratio(sc["wall"], sc["off_wall"]), ratio(sc["tick"], sc["off_tick"]),
           int(sc["both"]), ratio(sc["both_w"], sc["off_both_w"]),
           f"{sc['read'] / sc['tick']:.3f}" if sc["tick"] else "-"]
    return "| " + " | ".join(str(x) for x in row) + " |"


def arm_table(run: Run, timeout: int) -> None:
    print(f"## Arms against off at the same seed ({len(run.cells)} cells, {timeout} s)\n")
    print(ARM_HEAD)
    print("|" + "---|" * 16)
    for a in run.arms:
        if a in run.walk_arms:
            print(arm_row(a, score_arm(run, a, timeout)))
        elif run.off_for(a) == a:
            print(arm_row(a, score_arm(run, a, timeout)))
    for p in run.placements:
        arms = run.arms_of(p)
        if len(arms) > 1:
            print(arm_row(f"{p} (seeds {', '.join(seed_of(a) for a in arms)})",
                          merge([score_arm(run, a, timeout) for a in arms])))
    print()
    for a in run.arms:
        sc = score_arm(run, a, timeout) if run.off_for(a) else None
        if sc and sc["na"]:
            print(f"({a}: {int(sc['na'])} cells printed no work clock line; left out of tick PAR-2)\n")


def family_table(run: Run) -> None:
    fam = {}
    with open(FAMILIES) as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            if r["suite"] == "sat-comp-2025":
                fam[r["stem"]] = r["family"]
    by = defaultdict(list)
    for s in run.cells:
        by[fam.get(s, "?")].append(s)
    print("## Solved per family (where the arms differ)\n")
    print("| Family | Cells | " + " | ".join(run.arms) + " |")
    print("|" + "---|" * (len(run.arms) + 2))
    for f in sorted(by):
        counts = [sum(solved(run.rows[a][s]) for s in by[f]) for a in run.arms]
        if len(set(counts)) > 1:
            print(f"| {f} | {len(by[f])} | " + " | ".join(str(c) for c in counts) + " |")
    print()


def cell_table(run: Run) -> None:
    print("## Cells where an arm differs from off in solved\n")
    print("| Cell | " + " | ".join(run.arms) + " |")
    print("|" + "---|" * (len(run.arms) + 1))
    for s in run.cells:
        res = {a: run.rows[a][s] for a in run.arms}
        if any(run.off_for(a) and solved(res[a]) != solved(run.rows[run.off_for(a)][s]) for a in run.arms):
            def show(a):
                r = res[a]
                if not solved(r):
                    return "-"
                found = run.walk[a].get(s, {}).get("status") == "found" and r["result"] == "SATISFIABLE"
                return f"{float(r['time_s']):.0f}{' w' if found else ''}"
            print(f"| {s.split('-', 1)[-1][:48]} | " + " | ".join(show(a) for a in run.arms) + " |")
    print("\n(seconds; w = the walk found the model; - = unsolved)\n")


def file_tests():
    def strict(sh):
        return sh["broad"] and sh["groups"] > 0 and sh["overlap"] == 0 and sh["free"] == 0 \
            and (sh["defs"] > 0 or sh["gates"] > 0)

    def loose(sh):
        return sh["broad"] and sh["groups"] > 0 and (sh["defs"] > 0 or sh["gates"] > 0)

    def broad(sh):
        return sh["broad"]

    def limit(key, x):
        return lambda sh: sh["broad"] and sh[key] is not None and sh[key] <= x

    named = [("strict", strict), ("loose", loose), ("broad", broad)]
    named += [(f"broad, choice share <= {x:g}", limit("choice_share", x)) for x in (0.02, 0.05, 0.1, 0.2)]
    named += [(f"broad, free share <= {x:g}", limit("free_share", x)) for x in (0.0, 0.01, 0.05)]
    named += [(f"broad, false at start <= {x:g}", limit("false_share", x)) for x in (0.1, 0.2, 0.3)]
    return named


def tests(run: Run, record: dict, timeout: int) -> None:
    print("## File tests (each placement over its seeds: the arm on the cells a test admits, off plus "
          "the reading elsewhere)\n")
    print("| Test | Placement | Cells admitted | Solved | Off solved | + | - | Walk found | Wall PAR-2 x off "
          "| Tick PAR-2 x off | W on both x off |")
    print("|" + "---|" * 11)
    for name, t in file_tests():
        for p in run.placements:
            arms = run.arms_of(p)
            scores = [score_arm(run, a, timeout, lambda s, a=a: mixed_cell(run, a, s, t, record, timeout))
                      for a in arms]
            sc = merge(scores)
            adm = {s for a in arms for s in run.cells
                   if (sh := shape(run, a, s, record)) is not None and t(sh)}
            print(f"| {name} | {p} | {len(adm)} | {int(sc['solved'])} | {int(sc['off_solved'])} | {int(sc['plus'])} "
                  f"| {int(sc['minus'])} | {int(sc['walk_found'])} | {ratio(sc['wall'], sc['off_wall'])} "
                  f"| {ratio(sc['tick'], sc['off_tick'])} | {ratio(sc['both_w'], sc['off_both_w'])} |")
    print()


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run", nargs="+")
    ap.add_argument("--timeout", type=int, default=1800)
    ap.add_argument("--families", action="store_true")
    ap.add_argument("--tests", action="store_true")
    ap.add_argument("--cells", action="store_true")
    args = ap.parse_args(argv)
    run = Run([Path(r) for r in args.run])
    suites = defaultdict(set)
    with open(FAMILIES) as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            suites[r["stem"]].add(r["suite"])
    if any("sat-comp-2026" in suites[s] and "sat-comp-2025" not in suites[s] for s in run.cells):
        raise SystemExit("the 2026 suite is the holdout and is not read")
    with open(READ_RECORD) as fh:
        record = {r["stem"]: r for r in csv.DictReader(fh, delimiter="\t")}
    bad = failures(run)
    if bad:
        print(f"**CORRECTNESS: {len(bad)} failures (a failed, missing or skipped check, a crash, a premature "
              f"stop, or SAT against UNSAT); the run fails the gate until they are debugged; they count as "
              f"unsolved below:**\n")
        for b in bad:
            print(f"- {b}")
        print()
    arm_table(run, args.timeout)
    if args.families:
        family_table(run)
    if args.cells:
        cell_table(run)
    if args.tests:
        tests(run, record, args.timeout)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
