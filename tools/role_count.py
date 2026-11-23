#!/usr/bin/env python3
"""Read the solver's count of decisions by role (bead SAT-playground-d96.14.1).

The solver option --rolecount=1 prints `c roles ...` lines at its end
(solver/13-kissat-rs/src/roles.rs).  This tool turns the outputs of a set of
runs into two tables, checks that the count left the search alone, and prints
a summary.

  table RUNDIR --cells CELLS.tsv --out PREFIX
        RUNDIR holds NAME.sSEED.count.out, a run with the count on, for the
        cells of CELLS.tsv (short name, group, stem, ...).  Writes
        PREFIX.tsv       a row per cell, seed, mode and role: the counts at the
                         run's end
        PREFIX_hold.tsv  a row per cell, seed, checkpoint and role: what became
                         of the role's variables
        Every run is checked first: the roles' decisions add up to the solver's
        own count in each mode, and the modes' conflicts to the run's.
  same RUNDIR --cells CELLS.tsv [--seed N]
        The same-search check: NAME.sSEED.plain.out is the same run with the
        count off.  Every counter of the statistics and every field of the
        work clock must be equal.
  report PREFIX [--group G] [--cell NAME] [--at CHECKPOINT] [--per-cell ROLE,ROLE]
        A summary by group of cells: each role's share of the variables the
        solver holds, of the decisions, of the trail's top, and what became of
        its variables.  Every share is worked out a run, and the table gives
        the mean over the runs with the lowest and the highest run.
  selftest

Columns of PREFIX.tsv.  cell, group, seed, status, conflicts (of the run), mode
(focused, stable, or both: their sum), role; the mode's totals: mode_conflicts,
decisions, propagations, restarts, samples (trail samples), levels (decision
levels summed over the samples); the role's counts, each `_own` by the decided
variable's own role and `_stands` for every role that variable stands for
(its own and those of variables the solver replaced by it): decide (decisions),
true (of them, a variable of the role is set true), first (the first decision
after a restart), top (decisions in the trail's first ten levels, summed over
the samples), set (variables set above the root level, summed over the
samples), dec (of those, the decisions).

Columns of PREFIX_hold.tsv.  cell, group, seed, at (start: the first decision;
1000, 10000, ...: that many conflicts; end), conflicts, role, vars, and where
they are: held (the solver holds the variable under its own number), fixed,
eliminated, absent (in no clause), merged_held / merged_fixed /
merged_eliminated (replaced by an equal variable that is held / fixed /
eliminated), carriers (held variables that stand for the role), into (the
roles of the held replacements).
"""
from __future__ import annotations

import argparse
import csv
import re
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

COUNTS = ("decide", "true", "first", "top", "set", "dec")
MODE_TOTALS = ("mode_conflicts", "decisions", "propagations", "restarts", "samples", "levels")
HOLD = ("vars", "absent", "held", "fixed", "eliminated", "merged_held", "merged_fixed", "merged_eliminated", "carriers")
TABLE = (["cell", "group", "seed", "status", "conflicts", "mode", "role"] + list(MODE_TOTALS)
         + [f"{c}_{k}" for c in COUNTS for k in ("own", "stands")])
HOLD_TABLE = ["cell", "group", "seed", "at", "conflicts", "role"] + list(HOLD) + ["into"]


def parse_output(path):
    """A run's output: status, the work clock's fields, the statistics' first numbers and the role lines."""
    text = Path(path).read_text(errors="replace")
    out = {"status": "", "workclock": {}, "stats": {}, "file": None, "hold": [], "search": [], "count": []}
    in_stats = False
    for line in text.splitlines():
        if line.startswith("s "):
            out["status"] = line[2:].strip()
        elif line.startswith("c ---- ["):
            in_stats = "[ statistics ]" in line
        elif line.startswith("c workclock "):
            out["workclock"] = dict(t.split("=", 1) for t in line.split()[2:] if "=" in t)
        elif line.startswith("c roles "):
            toks = line.split()
            kind = toks[2]
            fields = dict(t.split("=", 1) for t in toks[3:] if "=" in t)
            if kind == "file":
                out["file"] = fields
            elif kind in ("hold", "search", "count"):
                out[kind].append(fields)
        elif in_stats:
            m = re.match(r"c ([a-z_]+):\s+(\d+)", line)
            if m:
                out["stats"][m.group(1)] = int(m.group(2))
    return out


def check_run(name, run):
    """The count's own consistency; returns the faults found."""
    bad = []
    if run["file"] is None:
        return [f"{name}: no 'c roles file' line (the count did not run or the run did not end)"]
    if run["file"].get("ended") != "exit":
        bad.append(f"{name}: the run was ended by a signal")
    end_search = [s for s in run["search"] if s["at"] == "end"]
    end_count = [c for c in run["count"] if c["at"] == "end"]
    if len(end_search) != 2:
        return bad + [f"{name}: no end checkpoint"]
    total_conf = total_dec = 0
    for s in end_search:
        own = sum(int(c["decide_own"]) for c in end_count if c["mode"] == s["mode"])
        if own != int(s["decisions"]):
            bad.append(f"{name}: {s['mode']} decisions by role {own} are not the solver's {s['decisions']}")
        total_conf += int(s["mode_conflicts"])
        total_dec += int(s["decisions"])
    wc = run["workclock"]
    # conflicts found while the formula was loaded, before the count began: in the run's total, in no mode's
    before = int(run["file"].get("conflicts_before", 0))
    if wc:
        if total_conf + before != int(wc.get("conflicts", -1)):
            bad.append(f"{name}: conflicts by mode {total_conf} and {before} before the count are not the run's {wc.get('conflicts')}")
        if total_dec != int(wc.get("decisions", -1)):
            bad.append(f"{name}: decisions by mode {total_dec} are not the run's {wc.get('decisions')}")
    else:
        bad.append(f"{name}: no work-clock line")
    for c in end_count:
        for k in COUNTS:
            if int(c[f"{k}_stands"]) < int(c[f"{k}_own"]) and k != "true":
                bad.append(f"{name}: {c['mode']} {c['role']} {k}_stands below {k}_own")
        if int(c["true_own"]) > int(c["decide_own"]) or int(c["true_stands"]) > int(c["decide_stands"]):
            bad.append(f"{name}: {c['mode']} {c['role']} more true decisions than decisions")
        if int(c["dec_own"]) > int(c["set_own"]):
            bad.append(f"{name}: {c['mode']} {c['role']} more decisions on the trail than set variables")
    return bad


def read_cells(path):
    cells = []
    for line in Path(path).read_text().splitlines():
        if line.strip():
            parts = line.split("\t")
            cells.append((parts[0], parts[1]))
    return cells


def runs_of(rundir, cells, kind="count"):
    """[(cell, group, seed, path)] for the outputs present."""
    out = []
    for cell, group in cells:
        for path in sorted(Path(rundir).glob(f"{cell}.s*.{kind}.out")):
            m = re.fullmatch(re.escape(cell) + r"\.s(\d+)\." + kind + r"\.out", path.name)
            if m:
                out.append((cell, group, int(m.group(1)), path))
    return out


def table_rows(cell, group, seed, run):
    """(rows of the count table, rows of the hold table) of one run."""
    rows, hold = [], []
    end_search = {s["mode"]: s for s in run["search"] if s["at"] == "end"}
    by_role = defaultdict(dict)
    for c in run["count"]:
        if c["at"] == "end":
            by_role[c["role"]][c["mode"]] = c
    conflicts = next(iter(end_search.values()))["conflicts"] if end_search else ""
    # a run that was decided before its first decision has no count line: its
    # roles are those of its variables, each with a count of zero
    roles = sorted(set(by_role) | {h["role"] for h in run["hold"] if h["at"] == "end"})
    both_tot = {k: sum(int(end_search[m][k]) for m in end_search) for k in MODE_TOTALS}
    for mode in ("focused", "stable", "both"):
        tot = both_tot if mode == "both" else {k: int(end_search[mode][k]) for k in MODE_TOTALS}
        for role in roles:
            row = {"cell": cell, "group": group, "seed": seed, "status": run["status"], "conflicts": conflicts,
                   "mode": mode, "role": role}
            row.update(tot)
            for c in COUNTS:
                for k in ("own", "stands"):
                    key = f"{c}_{k}"
                    modes = ("focused", "stable") if mode == "both" else (mode,)
                    row[key] = sum(int(by_role[role].get(m, {}).get(key, 0)) for m in modes)
            rows.append(row)
    for h in run["hold"]:
        row = {"cell": cell, "group": group, "seed": seed, "at": h["at"], "conflicts": h["conflicts"], "role": h["role"],
               "into": h.get("into", "-")}
        row.update({k: int(h[k]) for k in HOLD})
        hold.append(row)
    return rows, hold


def cmd_table(args):
    cells = read_cells(args.cells)
    runs = runs_of(args.rundir, cells)
    if not runs:
        raise SystemExit(f"{args.rundir}: no NAME.sSEED.count.out of the cells")
    rows, hold, bad = [], [], []
    for cell, group, seed, path in runs:
        run = parse_output(path)
        faults = check_run(path.name, run)
        if faults:
            bad += faults
            continue
        r, h = table_rows(cell, group, seed, run)
        rows += r
        hold += h
    for b in bad:
        print("FAULT " + b)
    if bad and not args.keep_going:
        raise SystemExit(f"{len(bad)} faults: no table written (--keep-going leaves those runs out)")
    for path, cols, data in ((f"{args.out}.tsv", TABLE, rows), (f"{args.out}_hold.tsv", HOLD_TABLE, hold)):
        with open(path, "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=cols, delimiter="\t", lineterminator="\n")
            w.writeheader()
            w.writerows(data)
    done = {(c, s) for c, _g, s, _p in runs}
    print(f"{len(runs)} runs of {len({c for c, _s in done})} cells, {len(bad)} faults; {len(rows)} rows in {args.out}.tsv, "
          f"{len(hold)} in {args.out}_hold.tsv")
    return 0


def cmd_same(args):
    """The count on against the count off: the same search?"""
    cells = read_cells(args.cells)
    same = differ = missing = 0
    for cell, _group in cells:
        on = Path(args.rundir) / f"{cell}.s{args.seed}.count.out"
        off = Path(args.rundir) / f"{cell}.s{args.seed}.plain.out"
        if not on.exists() or not off.exists():
            missing += 1
            print(f"MISSING {cell}")
            continue
        a, b = parse_output(on), parse_output(off)
        if not a["workclock"] or not b["workclock"] or not a["stats"] or not b["stats"]:
            missing += 1
            print(f"MISSING {cell}: a run without its end lines")
            continue
        # the arm with the count must have counted to its end, or nothing was compared
        faults = check_run(on.name, a)
        if faults:
            missing += 1
            print(f"MISSING {cell}: " + "; ".join(faults[:3]))
            continue
        diffs = [f"status {b['status']} -> {a['status']}"] if a["status"] != b["status"] else []
        for what in ("workclock", "stats"):
            for k in sorted(set(a[what]) | set(b[what])):
                if a[what].get(k) != b[what].get(k):
                    diffs.append(f"{k} {b[what].get(k)} -> {a[what].get(k)}")
        if b["file"] is not None:
            diffs.append("the plain run printed a count")
        if diffs:
            differ += 1
            print(f"DIFFER {cell}: " + "; ".join(diffs[:8]))
        else:
            same += 1
            if args.verbose:
                wc = a["workclock"]
                print(f"same   {cell}: {a['status']} conflicts={wc.get('conflicts')} decisions={wc.get('decisions')} "
                      f"work={wc.get('work')} ({len(a['workclock'])} work-clock fields, {len(a['stats'])} counters)")
    print(f"same search on {same} cells, different on {differ}, not compared {missing}")
    return 0 if differ == 0 and missing == 0 else 1


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------

def _pct(a, b):
    return 100 * a / b if b else None


def _stat(xs):
    """mean (lowest to highest) of the values that exist; whole numbers once they are large."""
    xs = [x for x in xs if x is not None]
    if not xs:
        return f"{'-':>5s} {'':13s}"
    m = sum(xs) / len(xs)
    if max(xs) >= 1000:
        return f"{m:,.0f} ({min(xs):,.0f}-{max(xs):,.0f})"
    return f"{m:5.1f} ({min(xs):5.1f}-{max(xs):5.1f})"


def load(prefix):
    rows = list(csv.DictReader(open(f"{prefix}.tsv", newline=""), delimiter="\t"))
    hold = list(csv.DictReader(open(f"{prefix}_hold.tsv", newline=""), delimiter="\t"))
    return rows, hold


def run_shares(rows, hold, at="1000"):
    """Per run, per role, its shares in percent (None where there is nothing to share):
      held       of the variables the solver holds at the checkpoint (its end, if the run ended before)
      dec_own    of the decisions, by the decided variable's own role
      dec        of the decisions, for every role the variable stands for
      dec_f, dec_s   the same in focused and in stable mode
      top        of the decisions in the trail's first ten levels (stands)
      first      of the first decisions after a restart (stands)
      true       of the decisions on the role (stands), those that set its variable true
      by_dec     of the role's variables set on the trail (stands), those set by a decision
      set_n      not a share: how many variables that stand for the role are set at a sampled conflict,
                 on average (the samples are of the whole run, so no one checkpoint's variable count
                 is the right total to divide by)
    Returns {(cell, seed): {"late": bool, "per_conflict": (decisions, propagations) a conflict by mode, "roles": {role: {...}}}}."""
    out = {}
    hold_at = defaultdict(dict)
    for h in hold:
        hold_at[(h["cell"], h["seed"])].setdefault(h["at"], []).append(h)
    by_run = defaultdict(list)
    for r in rows:
        by_run[(r["cell"], r["seed"])].append(r)
    for key, rs in by_run.items():
        mine = hold_at.get(key, {})
        label = at if at in mine else "end"
        hs = {h["role"]: h for h in mine.get(label, [])}
        total_held = sum(int(h["held"]) for h in hs.values())
        mode_rows = {m: {r["role"]: r for r in rs if r["mode"] == m} for m in ("focused", "stable", "both")}
        tot = {m: next(iter(v.values()), None) for m, v in mode_rows.items()}
        per_conflict = {}
        for m in ("focused", "stable"):
            x = tot[m]
            c = int(x["mode_conflicts"]) if x else 0
            per_conflict[m] = (int(x["decisions"]) / c, int(x["propagations"]) / c) if c else None
        both = mode_rows["both"]
        dec_all = int(tot["both"]["decisions"]) if tot["both"] else 0
        samples = int(tot["both"]["samples"]) if tot["both"] else 0
        top_all = sum(int(r["top_own"]) for r in both.values())
        first_all = sum(int(r["first_own"]) for r in both.values())
        roles = {}
        for role in set(both) | set(hs):
            r = both.get(role)
            h = hs.get(role)
            g = lambda k: int(r[k]) if r else 0     # noqa: E731
            d = {"held": _pct(int(h["held"]), total_held) if h else None,
                 "dec_own": _pct(g("decide_own"), dec_all), "dec": _pct(g("decide_stands"), dec_all),
                 "top_own": _pct(g("top_own"), top_all), "true_own": _pct(g("true_own"), g("decide_own")),
                 # the counts behind the shares by own role, for a sum over several roles
                 "n_decide_own": g("decide_own"), "n_true_own": g("true_own"),
                 "top": _pct(g("top_stands"), top_all), "first": _pct(g("first_stands"), first_all),
                 "true": _pct(g("true_stands"), g("decide_stands")), "by_dec": _pct(g("dec_stands"), g("set_stands")),
                 "set_n": g("set_stands") / samples if samples else None}
            for m, k in (("focused", "dec_f"), ("stable", "dec_s")):
                x = mode_rows[m].get(role)
                dm = int(tot[m]["decisions"]) if tot[m] else 0
                d[k] = _pct(int(x["decide_stands"]) if x else 0, dm)
            if h:
                v = int(h["vars"])
                for k in HOLD[1:]:
                    d["var_" + k] = _pct(int(h[k]), v)
                d["vars"] = v
                d["carriers"] = int(h["carriers"])
            roles[role] = d
        # which shares exist in this run: a role the run does not have has the share zero
        # of every total that is not itself zero
        dec_f = int(tot["focused"]["decisions"]) if tot["focused"] else 0
        dec_s = int(tot["stable"]["decisions"]) if tot["stable"] else 0
        den = {"held": total_held > 0, "dec_own": dec_all > 0, "dec": dec_all > 0, "dec_f": dec_f > 0, "dec_s": dec_s > 0,
               "top": top_all > 0, "top_own": top_all > 0, "first": first_all > 0, "set_n": samples > 0}
        out[key] = {"late": label != at, "per_conflict": per_conflict, "roles": roles, "status": rs[0]["status"],
                    "conflicts": int(rs[0]["conflicts"] or 0), "decisions": dec_all, "den": den}
    return out


def share_of(run, role, key):
    """A role's share in one run of run_shares: zero for a role the run does not have, where
    the share's total is not zero; None where there is nothing to share (a ratio within the role)."""
    d = run["roles"].get(role)
    if d is not None and d.get(key) is not None:
        return d[key]
    return 0.0 if run["den"].get(key) else None


def summary(rows, hold, group=None, cell=None, at="1000"):
    """The shares of each role, the mean over the chosen runs with the lowest and the highest run."""
    pick = lambda r: (group is None or r["group"] == group) and (cell is None or r["cell"] == cell)   # noqa: E731
    shares = run_shares([r for r in rows if pick(r)], [h for h in hold if pick(h)], at)
    if not shares:
        return [f"no runs for group {group} cell {cell}"]
    out = []
    late = sum(1 for s in shares.values() if s["late"])
    quiet = sum(1 for s in shares.values() if s["decisions"] == 0)
    out.append(f"{len(shares)} runs of {len({c for c, _ in shares})} cells; each number is the mean over the runs (lowest-highest run); "
               f"variables as at {at} conflicts" + (f" ({late} runs ended before that: their end)" if late else "")
               + (f"; {quiet} runs without a decision" if quiet else ""))
    for m in ("focused", "stable"):
        pc = [s["per_conflict"][m] for s in shares.values() if s["per_conflict"][m]]
        if pc:
            out.append(f"{m}: {sum(x[0] for x in pc) / len(pc):.2f} decisions and {sum(x[1] for x in pc) / len(pc):.0f} propagations a conflict "
                       f"({len(pc)} runs)")
        else:
            out.append(f"{m}: no conflicts")
    roles = sorted({r for s in shares.values() for r in s["roles"]},
                   key=lambda r: -sum((share_of(s, r, "dec") or 0) for s in shares.values()))
    col = lambda role, k: _stat([share_of(s, role, k) for s in shares.values()])   # noqa: E731
    out.append("")
    out.append("share of the held variables and of the decisions, in percent")
    out.append(f"  {'role':10s} | {'held':>19s} | {'decisions, own':>19s} | {'decisions, stands':>19s} | {'focused':>19s} | {'stable':>19s}")
    for role in roles:
        out.append(f"  {role:10s} | {col(role, 'held')} | {col(role, 'dec_own')} | {col(role, 'dec')} | {col(role, 'dec_f')} | {col(role, 'dec_s')}")
    out.append("")
    out.append("where the decisions sit and what they try (stands), in percent; the variables set at a sampled conflict are a count")
    out.append(f"  {'role':10s} | {'first ten levels':>19s} | {'first after restart':>19s} | {'tried true':>19s} | {'set by a decision':>19s} | {'variables set at a conflict':>27s}")
    for role in roles:
        out.append(f"  {role:10s} | {col(role, 'top')} | {col(role, 'first')} | {col(role, 'true')} | {col(role, 'by_dec')} | {col(role, 'set_n'):>27s}")
    out.append("")
    out.append("what became of each role's variables, in percent of them")
    out.append(f"  {'role':10s} | {'variables':>9s} | {'held':>19s} | {'fixed':>19s} | {'eliminated':>19s} | {'in no clause':>19s} | "
               f"{'replaced, held':>19s} | {'replaced, fixed':>19s} | {'replaced, gone':>19s}")
    for role in sorted(roles, key=lambda r: -sum(s["roles"].get(r, {}).get("vars", 0) for s in shares.values())):
        n = [s["roles"][role]["vars"] for s in shares.values() if "vars" in s["roles"].get(role, {})]
        if not n:
            continue
        out.append(f"  {role:10s} | {sum(n) // len(n):>9d} | {col(role, 'var_held')} | {col(role, 'var_fixed')} | {col(role, 'var_eliminated')} | "
                   f"{col(role, 'var_absent')} | {col(role, 'var_merged_held')} | {col(role, 'var_merged_fixed')} | {col(role, 'var_merged_eliminated')}")
    return out


def per_cell(rows, hold, roles, group=None, at="1000"):
    """One line a cell (the mean of its runs).  For each entry of `roles`: the share of the held
    variables, of the decisions, of the decisions in the first ten levels, and of its decisions
    those that try true.  An entry is one role, counted for every role a decided variable stands
    for; or several roles joined by `+`, counted by the decided variable's own role, so that
    the roles' shares add up (a decision has one own role)."""
    pick = lambda r: group is None or r["group"] == group   # noqa: E731
    shares = run_shares([r for r in rows if pick(r)], [h for h in hold if pick(h)], at)
    cells = defaultdict(list)
    for (cell, _seed), s in shares.items():
        cells[cell].append(s)
    out = [f"  {'cell':38s} {'runs':>4s} {'conflicts':>10s} | " + " | ".join(f"{r + ': held dec top true':>30s}" for r in roles)]
    mean = lambda xs: (sum(xs) / len(xs)) if xs else None   # noqa: E731
    fmt = lambda x: f"{x:6.1f}" if x is not None else f"{'-':>6s}"   # noqa: E731

    def value(s, entry, key):
        if "+" not in entry:
            return share_of(s, entry, key)
        parts = entry.split("+")
        if key == "true":
            n = sum(s["roles"].get(r, {}).get("n_decide_own", 0) for r in parts)
            return _pct(sum(s["roles"].get(r, {}).get("n_true_own", 0) for r in parts), n)
        k = {"held": "held", "dec": "dec_own", "top": "top_own"}[key]
        xs = [share_of(s, r, k) for r in parts]
        return None if all(x is None for x in xs) else sum(x for x in xs if x is not None)

    for cell in sorted(cells):
        ss = cells[cell]
        parts = []
        for r in roles:
            vals = [fmt(mean([x for x in (value(s, r, k) for s in ss) if x is not None])) for k in ("held", "dec", "top", "true")]
            parts.append(f"{' '.join(vals):>30s}")
        out.append(f"  {cell[:38]:38s} {len(ss):>4d} {sum(s['conflicts'] for s in ss) // len(ss):>10d} | " + " | ".join(parts))
    return out


def cmd_report(args):
    rows, hold = load(args.prefix)
    groups = [args.group] if args.group else sorted({r["group"] for r in rows})
    if args.cell:
        print("\n".join(summary(rows, hold, cell=args.cell, at=args.at)))
        return 0
    for g in groups:
        print(f"== {g} ==")
        if args.per_cell:
            print("\n".join(per_cell(rows, hold, args.per_cell.split(","), group=g, at=args.at)))
        else:
            print("\n".join(summary(rows, hold, group=g, at=args.at)))
        print()
    return 0


# ---------------------------------------------------------------------------
# selftest
# ---------------------------------------------------------------------------

def _fake(decisions=(30, 12), conflicts=(7, 5), split=((20, 10), (8, 4)), broken=None, count=True, searched=True):
    """A solver output with two roles, `a` and `b`; `split` gives the roles' decisions in each mode.
    Without `searched` the run was decided before its first decision: no count line."""
    if not searched:
        decisions, conflicts = (0, 0), (0, 0)
    lines = ["c ---- [ result ] ----", "s UNKNOWN", "c ---- [ statistics ] ----", "c",
             f"c conflicts:        {sum(conflicts)}    1.0 per second", f"c decisions:   {sum(decisions)}  2.0 per conflict",
             "c ---- [ resources ] ----", "c process-time:   12 seconds",
             f"c workclock ticks=100 conflicts={sum(conflicts)} decisions={sum(decisions)} work=100"]
    if count:
        lines.append("c roles file path=x roles=2 listed=10 original=10 merges=1 checkpoints=2 top_levels=10 sample_conflicts=128 ended=exit")
        for at, conf in (("start", 0), ("end", sum(conflicts))):
            lines.append(f"c roles hold at={at} conflicts={conf} role=a vars=6 absent=0 held=5 fixed=1 eliminated=0 merged_held=0 merged_fixed=0 merged_eliminated=0 carriers=5 into=-")
            lines.append(f"c roles hold at={at} conflicts={conf} role=b vars=4 absent=0 held=2 fixed=0 eliminated=1 merged_held=1 merged_fixed=0 merged_eliminated=0 carriers=2 into=a:1")
        for m, mode in enumerate(("focused", "stable")):
            lines.append(f"c roles search at=end conflicts={sum(conflicts)} mode={mode} mode_conflicts={conflicts[m]} decisions={decisions[m]} propagations=900 ticks=50 restarts=3 samples=2 levels=9")
            for r, role in enumerate(("a", "b") if searched else ()):
                d = split[m][r]
                if broken == "sum" and m == 0 and r == 0:
                    d += 1
                lines.append(f"c roles count at=end conflicts={sum(conflicts)} mode={mode} role={role} decide_own={d} decide_stands={d + r} true_own={d // 2} true_stands={d // 2} "
                             f"first_own=1 first_stands=1 top_own=4 top_stands=4 set_own=6 set_stands=6 dec_own=3 dec_stands=3")
    return "\n".join(lines) + "\n"


def _check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'} {name}{(' ' + str(detail)) if detail and not ok else ''}")
    return bool(ok)


def cmd_selftest(_args):
    ok = True
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        (tmp / "cells.tsv").write_text("x\tg1\tstem-x\t\ny\tg2\tstem-y\t\n")
        (tmp / "x.s2.count.out").write_text(_fake())
        (tmp / "x.s2.plain.out").write_text(_fake(count=False))
        (tmp / "y.s2.count.out").write_text(_fake(decisions=(31, 12), split=((21, 10), (8, 4))))
        (tmp / "y.s2.plain.out").write_text(_fake(count=False))        # one decision fewer: a different search
        run = parse_output(tmp / "x.s2.count.out")
        ok &= _check("a run is parsed", run["status"] == "UNKNOWN" and run["stats"] == {"conflicts": 12, "decisions": 42}
                     and run["workclock"]["work"] == "100" and len(run["hold"]) == 4 and len(run["count"]) == 4, run)
        ok &= _check("a clean run has no fault", check_run("x", run) == [])
        (tmp / "bad.out").write_text(_fake(broken="sum"))
        ok &= _check("decisions that do not add up are a fault", any("decisions by role" in b for b in check_run("b", parse_output(tmp / "bad.out"))))
        (tmp / "nocount.out").write_text(_fake(count=False))
        ok &= _check("a run without the count is a fault", check_run("n", parse_output(tmp / "nocount.out")) != [])
        (tmp / "signal.out").write_text(_fake().replace("ended=exit", "ended=signal"))
        ok &= _check("a run ended by a signal is a fault", any("signal" in b for b in check_run("s", parse_output(tmp / "signal.out"))))
        # a conflict found while the formula was loaded: in the run's total, in no mode's
        early = _fake().replace("conflicts=12 decisions=42 work", "conflicts=13 decisions=42 work")
        (tmp / "early.out").write_text(early)
        ok &= _check("a conflict the modes do not have is a fault", any("conflicts by mode" in b for b in check_run("e", parse_output(tmp / "early.out"))))
        (tmp / "early.out").write_text(early.replace("merges=1 ", "merges=1 conflicts_before=1 "))
        ok &= _check("unless it was counted before the count began", check_run("e", parse_output(tmp / "early.out")) == [])
        ns = argparse.Namespace(rundir=str(tmp), cells=str(tmp / "cells.tsv"), out=str(tmp / "t"), keep_going=False)
        ok &= _check("the table is written", cmd_table(ns) == 0)
        rows, hold = load(str(tmp / "t"))
        ok &= _check("a row per cell, mode and role", len(rows) == 2 * 3 * 2 and len(hold) == 2 * 2 * 2, (len(rows), len(hold)))
        both = [r for r in rows if r["cell"] == "x" and r["mode"] == "both" and r["role"] == "a"][0]
        ok &= _check("the row of both modes is their sum", both["decide_own"] == "28" and both["decisions"] == "42" and both["mode_conflicts"] == "12", both)
        text = "\n".join(summary(rows, hold, group="g1", at="1000"))
        ok &= _check("a run that ended before the checkpoint is summed at its end", "1 runs ended before that" in text, text)
        shares = run_shares(rows, hold)[("x", "2")]
        a = shares["roles"]["a"]
        # role a: 28 of the 42 decisions, 20 of the 30 focused ones, 5 of the 7 held variables, 14 of its 28 decisions true
        ok &= _check("a role's shares", abs(a["dec_own"] - 100 * 28 / 42) < 1e-9 and abs(a["dec_f"] - 100 * 20 / 30) < 1e-9
                     and abs(a["held"] - 100 * 5 / 7) < 1e-9 and abs(a["true"] - 50) < 1e-9 and a["var_fixed"] == 100 / 6, a)
        ok &= _check("the rates a conflict", shares["per_conflict"]["focused"] == (30 / 7, 900 / 7), shares["per_conflict"])
        # 12 variables of role a set over the run's 4 samples: 3 a sample, whatever the checkpoint's variable count
        ok &= _check("the variables set at a conflict are a count a sample", a["set_n"] == 3.0 and a["by_dec"] == 50.0, a)
        ok &= _check("large counts are printed whole", _stat([1234.4, 250000.0]) == "125,617 (1,234-250,000)", _stat([1234.4, 250000.0]))
        ok &= _check("the share of decisions is printed", " 66.7 ( 66.7- 66.7)" in text, text)
        # a role that only one of two runs has: its share is the mean with a zero, not the one run's
        two = run_shares(rows, hold)
        only_y = dict(two[("y", "2")])
        only_y["roles"] = {k: v for k, v in only_y["roles"].items() if k != "b"}
        ok &= _check("a role a run does not have has the share zero", share_of(only_y, "b", "dec_own") == 0.0
                     and share_of(only_y, "b", "held") == 0.0 and share_of(only_y, "b", "true") is None
                     and share_of(two[("x", "2")], "b", "dec_own") == 100 * 14 / 42, share_of(two[("x", "2")], "b", "dec_own"))
        ok &= _check("and is averaged as zero", _stat([share_of(only_y, "b", "dec_own"), share_of(two[("x", "2")], "b", "dec_own")])
                     == " 16.7 (  0.0- 33.3)", _stat([share_of(only_y, "b", "dec_own"), share_of(two[("x", "2")], "b", "dec_own")]))
        cells = "\n".join(per_cell(rows, hold, ["a"], group="g1"))
        ok &= _check("a line a cell", "x " in cells and "  71.4   66.7" in cells, cells)
        # the two roles together, by own role: all 7 held variables, all 42 decisions, half of them true
        cells = "\n".join(per_cell(rows, hold, ["a+b"], group="g1"))
        ok &= _check("several roles together add up", " 100.0  100.0  100.0   50.0" in cells, cells)
        ns = argparse.Namespace(rundir=str(tmp), cells=str(tmp / "cells.tsv"), seed=2, verbose=False)
        ok &= _check("the same-search check finds the cell that differs", cmd_same(ns) == 1)
        # a run decided before its first decision, beside one that searched
        (tmp / "x.s3.count.out").write_text(_fake(searched=False))
        quiet = parse_output(tmp / "x.s3.count.out")
        ok &= _check("a run without a decision has no fault", check_run("q", quiet) == [] and quiet["count"] == [])
        ns = argparse.Namespace(rundir=str(tmp), cells=str(tmp / "cells.tsv"), out=str(tmp / "u"), keep_going=False)
        ok &= _check("the table takes a run without a decision", cmd_table(ns) == 0)
        rows, hold = load(str(tmp / "u"))
        zero = [r for r in rows if r["cell"] == "x" and r["seed"] == "3"]
        ok &= _check("its roles have rows of zeros", len(zero) == 3 * 2 and all(r["decide_own"] == "0" and r["decisions"] == "0" for r in zero), zero)
        text = "\n".join(summary(rows, hold, group="g1", at="1000"))
        ok &= _check("the summary takes it beside a run that searched", "2 runs of 1 cells" in text and "1 runs without a decision" in text
                     and " 66.7 ( 66.7- 66.7)" in text, text)
        only = "\n".join(summary([r for r in rows if r["seed"] == "3"], [h for h in hold if h["seed"] == "3"], group="g1"))
        ok &= _check("the summary of runs without a conflict says so", "no conflicts" in only, only)
        (tmp / "cells1.tsv").write_text("x\tg1\tstem-x\t\n")
        ns = argparse.Namespace(rundir=str(tmp), cells=str(tmp / "cells1.tsv"), seed=2, verbose=True)
        ok &= _check("the same-search check passes an equal pair", cmd_same(ns) == 0)
        # two equal runs neither of which counted: nothing was checked
        (tmp / "z.s2.count.out").write_text(_fake(count=False))
        (tmp / "z.s2.plain.out").write_text(_fake(count=False))
        (tmp / "cellsz.tsv").write_text("z\tg1\tstem-z\t\n")
        ns = argparse.Namespace(rundir=str(tmp), cells=str(tmp / "cellsz.tsv"), seed=2, verbose=False)
        ok &= _check("the same-search check refuses a counted arm that did not count", cmd_same(ns) == 1)
    print("selftest: all ok" if ok else "selftest: FAILED")
    return 0 if ok else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("table")
    t.add_argument("rundir")
    t.add_argument("--cells", required=True)
    t.add_argument("--out", required=True)
    t.add_argument("--keep-going", action="store_true", help="write the table without the runs that have a fault")
    s = sub.add_parser("same")
    s.add_argument("rundir")
    s.add_argument("--cells", required=True)
    s.add_argument("--seed", type=int, default=2)
    s.add_argument("--verbose", action="store_true")
    r = sub.add_parser("report")
    r.add_argument("prefix")
    r.add_argument("--group")
    r.add_argument("--cell")
    r.add_argument("--at", default="1000", help="the checkpoint of the variables' table (start, 1000, 10000, ..., end)")
    r.add_argument("--per-cell", help="ROLE[,ROLE]: one line a cell with these roles' shares, instead of the summary; "
                   "ROLE+ROLE is several roles together, by the decided variable's own role")
    sub.add_parser("selftest")
    args = ap.parse_args(argv)
    return {"table": cmd_table, "same": cmd_same, "report": cmd_report, "selftest": cmd_selftest}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
