#!/usr/bin/env python3
"""Arms of 400-cell runs side by side: the tables for a sign-off
(epic SAT-playground-bp3, the speed gate and the runs).

An arm is one results.tsv of tools/feature_ablation.py (one row a cell), or the
kept.tsv the harness appends to while a run is still going (--keep-lines
'c workclock '), optionally with a folder of `perf stat` count files, one a
cell (<cell>.csv, written with -x ,). The arms may come from different runs.

Four parts:

0. Failures, over every row of every file, whether or not the other arms have
   that cell yet: contradictions, answers that do not count, and stops that are
   neither the clock nor the memory cap. All three must be empty.

1. Each arm, over the cells that all the arms have (so each is scored on the
   same cells): cells solved, wall PAR-2 (a solved cell costs its seconds, an
   unsolved one twice the time limit), tick PAR-2 (the same sum on the work
   clock W: a solved cell costs its W, an unsolved one twice the W it had
   reached), and how the answers were checked: models checked, proofs checked,
   proofs the checker did not finish, anything else.

2. Each arm against the baseline arm: which cells only one of the two solves
   and how close to the limit, on how many cells the search is the same, and
   the wall-time ratio on the cells both solve.

3. With count files: instructions, clock ticks, time on the processor and the
   share of micro-operations that went through the decoder again, on the cells
   where both arms ran the same search to an answer. Only there do two runs
   end at the same point, so only there can their counts be compared; a run
   stopped by the clock ends wherever it was.

What the words mean here:

- A cell is one instance at one seed. A file with several seeds keeps them all.
- Solved: a SAT or UNSAT answer from a run that ended normally and is not a
  bad answer. A bad answer is one whose check failed (verified=FAIL), an UNSAT
  answer with no proof file (no-proof), or an answer whose run left a line on
  the error stream. A bad answer is never a solve: it is priced as unsolved
  and listed by name.
- A run without an answer is one of three things. A timeout: the clock, or a
  kill from outside. Ended at the memory cap: the run died and its note is an
  allocation that failed. An early stop: anything else, such as an UNKNOWN
  before the limit or a crash (a run whose note is a panic is an early stop
  whatever else is recorded for it). The first two are honest stops; an early
  stop is a failure.
- A run that printed its status line and was then stopped (the harness notes
  `exit rc=N after the status line`) has no answer: the clock can cut a model
  in the middle of its lines. It is classed by how it ended, as above, and
  its unfinished model failing the check does not make it a bad answer.
- A contradiction is an instance with a SAT answer and an UNSAT answer anywhere
  in the files given: in any two arms, or at two seeds of one arm.
- The same search: both arms solve the cell with equal conflict counts and
  equal work W. Equal conflicts alone are not enough (two runs that the walk
  decides have 0 conflicts each and may differ in work), so the count of those
  is printed beside it and not used.
- A kept.tsv has no notes. Read from it, an answer of a run that then died is
  not seen as bad, and a run that died has no reason, so it shows as an early
  stop. The report says so; the numbers are final only from results.tsv.

usage:
  as_report.py [--timeout 1800] [--baseline NAME] [--floor 1.0]
               NAME=RESULTS.TSV[,PERF_FOLDER] ...
  as_report.py --selftest

The first arm is the baseline unless --baseline names another. --floor is the
shortest baseline time, in seconds, of a cell that enters a wall-time ratio
(tools/compare_full_runs.py uses 1 s too).
"""

import argparse
import csv
import math
import os
import re
import sys
import tempfile

SOLVED = ("SATISFIABLE", "UNSATISFIABLE")
EVENTS = ("instructions", "cycles", "idq.dsb_uops", "idq.mite_uops", "branch-misses")
MEMORY_NOTE = re.compile(r"memory allocation of .* failed|out of memory", re.I)
PANIC_NOTE = re.compile(r"panicked", re.I)
EXIT_NOTE = re.compile(r"^exit rc=(-?\d+) after the status line")  # written by feature_ablation._solve_one
HEAD_FMT = "%-8s %6s %5s %5s | %8s %7s %5s %4s | %12s %11s %11s %11s | %9s %9s %8s %5s"
ROW_FMT = "%-8s %6d %5d %5d | %8d %7d %5d %4d | %12.0f %11.4g %11.4g %11.4g | %9d %9d %8d %5d"


def read_arm(path):
    """({(instance, seed): row}, live) from a results.tsv, or from a kept.tsv of a run still going."""
    rows = {}
    with open(path, newline="") as fh:
        first = fh.readline().rstrip("\r\n").split("\t")
        fh.seek(0)
        live = first[0] != "config"
        if not live:
            for r in csv.DictReader(fh, delimiter="\t"):
                rows[(r["instance"], r.get("seed", "0") or "0")] = {
                    "result": r["result"], "time": float(r["time_s"]), "conflicts": r["conflicts"],
                    "work": r["work"], "verified": r["verified"], "note": r.get("note", "") or "",
                    "limit": r.get("timeout", "") or ""}
        else:
            # kept.tsv: cell, seed, result, seconds, check, the kept line. No notes.
            for line in fh:
                f = line.rstrip("\r\n").split("\t")
                if len(f) < 6 or not f[5].startswith("c workclock "):
                    continue
                kv = dict(t.split("=", 1) for t in f[5].split() if "=" in t)
                rows[(f[0], f[1])] = {"result": f[2], "time": float(f[3]), "conflicts": kv.get("conflicts", "NA"),
                                      "work": kv.get("work", "NA"), "verified": f[4], "note": "", "limit": ""}
    return rows, live


def read_counts(folder, cell):
    """One run's counts: ({event: count}, seconds on the processor), or (None, why).

    The count files carry the instance's name only, so they belong to one-seed runs.
    """
    path = os.path.join(folder, cell[0] + ".csv")
    if not os.path.isfile(path):
        return None, "no file"
    out, on_cpu = {}, None
    with open(path) as fh:
        for line in fh:
            f = line.strip().split(",")
            # perf: count, unit, event, time counted (ns), share of the run counted (%)
            if len(f) > 4 and f[0].isdigit():
                name = f[2][:-2] if f[2].endswith(":u") else f[2]
                try:
                    if float(f[4]) < 99.995:
                        return None, f"{name} counted {f[4]}% of the run"
                    if name == "instructions":
                        on_cpu = float(f[3]) / 1e9
                except ValueError:
                    return None, f"{name}: unreadable line"
                out[name] = float(f[0])
    missing = [e for e in EVENTS if e not in out]
    if missing or on_cpu is None:
        return None, "no count for " + ",".join(missing or ["instructions"])
    return out, on_cpu


def geomean(xs):
    return math.exp(sum(math.log(x) for x in xs) / len(xs))


def pct(xs, p):
    """The value a share p of the sorted list lies at or below (nearest rank)."""
    s = sorted(xs)
    return s[min(len(s) - 1, max(0, math.ceil(p * len(s)) - 1))]


def has_w(row):
    return row["work"] not in ("", "NA")


def stop_kind(rc_text, note):
    """How a run that left no answer ended: timeout (the clock or a kill from outside), memory, or early.

    The exit code is `timeout`'s 124, 128 + a signal's number through `timeout`, or minus the
    number from Python. The classes are those of tools/rl_sweep_report.py.
    """
    if PANIC_NOTE.search(note):
        return "early"
    try:
        rc = int(rc_text)
    except ValueError:
        return "early"
    signal = -rc if rc < 0 else rc - 128 if rc > 128 else None
    if rc == 124 or signal in (9, 15):
        return "timeout"
    if MEMORY_NOTE.search(note):
        return "memory"
    return "early"


def kind(row):
    """What a run is: solved, bad (an answer that does not count), timeout, memory, or early.

    The row carries its time limit (report() fills it in for a kept.tsv).
    """
    res, note = row["result"], row["note"]
    cut = EXIT_NOTE.match(note)
    if cut:
        # a status line, then an exit that is no answer: classed by the exit, whatever the line said
        return stop_kind(cut.group(1), note)
    if res in SOLVED:
        return "bad" if row["verified"] in ("FAIL", "no-proof") or note else "solved"
    if PANIC_NOTE.search(note):
        return "early"      # a crash is a failure, also when the clock ended the run after it
    if res == "TIMEOUT":
        return "timeout"
    if res.startswith("UNKNOWN_rc"):
        return stop_kind(res[len("UNKNOWN_rc"):], note)
    # a plain UNKNOWN: a budget stop only if it came at the limit
    return "timeout" if row["time"] >= 0.95 * float(row["limit"]) else "early"


def solved(row):
    return kind(row) == "solved"


def limit_of(row, timeout):
    try:
        return float(row["limit"])
    except ValueError:
        return float(timeout)


def score(rows, cells, priced, timeout):
    """One arm's numbers over `cells`; tick PAR-2 over `priced` (cells where every arm has W)."""
    s = {"sat": 0, "unsat": 0, "timeout": 0, "memory": 0, "early": 0, "bad": 0, "par2": 0.0,
         "tick_solved": 0.0, "tick_unsolved": 0.0, "models_ok": 0, "proofs_ok": 0,
         "checker_timeout": 0, "check_other": 0}
    for c in cells:
        r = rows[c]
        k = kind(r)
        if k == "solved":
            s["sat" if r["result"] == "SATISFIABLE" else "unsat"] += 1
            s["par2"] += r["time"]
            if r["verified"] == "ok":
                s["models_ok" if r["result"] == "SATISFIABLE" else "proofs_ok"] += 1
            elif r["verified"] == "checker-timeout":
                s["checker_timeout"] += 1
            else:
                s["check_other"] += 1
        else:
            s[k] += 1
            s["par2"] += 2.0 * limit_of(r, timeout)
    for c in priced:
        r = rows[c]
        w = float(r["work"])
        if solved(r):
            s["tick_solved"] += w
        else:
            s["tick_unsolved"] += 2.0 * w
    s["tick"] = s["tick_solved"] + s["tick_unsolved"]
    s["solved"] = s["sat"] + s["unsat"]
    return s


def row_of(name, s):
    return ROW_FMT % (name, s["solved"], s["sat"], s["unsat"], s["timeout"], s["memory"], s["early"], s["bad"],
                      s["par2"], s["tick"], s["tick_solved"], s["tick_unsolved"], s["models_ok"],
                      s["proofs_ok"], s["checker_timeout"], s["check_other"])


MANY_SEEDS = False  # set by report(): more than one seed among the cells


def short(cell, width=44):
    """A cell's name without its hash prefix; with its seed when the files hold several."""
    inst, seed = cell
    name = inst.split("-", 1)[1] if "-" in inst and len(inst.split("-", 1)[0]) == 32 else inst
    return name[:width] + (f" (seed {seed})" if MANY_SEEDS else "")


def failures(data, live, out):
    """Part 0, over every row read. Returns how many failures there are."""
    names = list(data)
    answers = {}
    for n in names:
        for (inst, seed), r in data[n].items():
            if r["result"] in SOLVED:
                answers.setdefault(inst, {}).setdefault(r["result"], []).append(f"{n} seed {seed}")
    contra = {inst: a for inst, a in answers.items() if len(a) > 1}
    out(f"contradictions (an instance answered SAT in one run and UNSAT in another, over all arms and "
        f"seeds): {len(contra)}")
    for inst, a in sorted(contra.items()):
        out(f"  CONTRADICTION {short((inst, '0'))}: SAT in {', '.join(a['SATISFIABLE'])}; "
            f"UNSAT in {', '.join(a['UNSATISFIABLE'])}")
    found = {"bad": [], "early": [], "memory": []}
    for n in names:
        for c in sorted(data[n]):
            k = kind(data[n][c])
            if k in found:
                found[k].append((n, c))
    out(f"answers that do not count (a failed check, no proof, or a run that left a note): {len(found['bad'])}")
    for n, c in found["bad"]:
        r = data[n][c]
        out(f"  BAD ANSWER {n} {short(c)}: {r['result']}, check {r['verified']}"
            + (f", note: {r['note'][:80]}" if r["note"] else ""))
    out(f"stops that are neither the clock nor the memory cap: {len(found['early'])}")
    for n, c in found["early"]:
        r = data[n][c]
        why = "no note in kept.tsv" if live[n] else (r["note"][:80] or "no note")
        out(f"  EARLY STOP {n} {short(c)}: {r['result']} after {r['time']:.0f} s ({why})")
    if found["memory"]:
        by_arm = {}
        for n, c in found["memory"]:
            by_arm.setdefault(n, []).append(short(c, 24))
        out("ended at the memory cap (an honest stop, priced as unsolved): "
            + "; ".join(f"{n}: {', '.join(cs)}" for n, cs in by_arm.items()))
    if any(live.values()):
        out("read from kept.tsv, which has no notes: " + ", ".join(n for n in names if live[n])
            + ". There an answer of a run that then died is not seen as bad, and a run that died shows as an "
            "early stop. Final only from results.tsv.")
    return len(contra) + len(found["bad"]) + len(found["early"])


def against(name, rows, base_name, base, cells, timeout, floor, out):
    """Part 2 for one arm. Returns the cells where both ran the same search to an answer."""
    both = [c for c in cells if solved(rows[c]) and solved(base[c])]
    only_new = [c for c in cells if solved(rows[c]) and not solved(base[c])]
    only_base = [c for c in cells if solved(base[c]) and not solved(rows[c])]
    same_conf = [c for c in both if rows[c]["result"] == base[c]["result"]
                 and rows[c]["conflicts"] == base[c]["conflicts"] and rows[c]["conflicts"] not in ("", "NA")]
    same = [c for c in same_conf if has_w(rows[c]) and rows[c]["work"] == base[c]["work"]]
    out(f"\n--- {name} against {base_name} ---")
    out(f"solved by both {len(both)}; only by {name} {len(only_new)}; only by {base_name} {len(only_base)}")
    for who, cs, mine, other in ((name, only_new, rows, base), (base_name, only_base, base, rows)):
        for c in cs:
            lim = limit_of(mine[c], timeout)
            out(f"  only {who}: {short(c):44s} {mine[c]['result'][:5]:5s} {mine[c]['time']:7.0f} s "
                f"({100 * mine[c]['time'] / lim:.0f}% of the limit); the other: {other[c]['result']}"
                + (" (a bad answer)" if kind(other[c]) == "bad" else ""))
    out(f"the same search (equal conflict counts and equal work W) on {len(same)} of the {len(both)} cells "
        f"both solve; equal conflict counts alone on {len(same_conf)}")
    for label, group in (("cells both solve", both), ("the same-search cells", same)):
        pairs = [(rows[c]["time"], base[c]["time"]) for c in group if base[c]["time"] >= floor and rows[c]["time"] > 0]
        if not pairs or (label != "cells both solve" and len(group) == len(both)):
            continue
        ratios = [a / b for a, b in pairs]
        out(f"wall time, {name} over {base_name}, on {label} with {floor:g} s or more in {base_name} "
            f"({len(pairs)} cells):")
        out(f"  summed time {sum(a for a, _ in pairs) / sum(b for _, b in pairs):.4f}   "
            f"geometric mean {geomean(ratios):.4f}   median {pct(ratios, 0.5):.4f}   "
            f"tenth to ninetieth percentile {pct(ratios, 0.1):.4f} to {pct(ratios, 0.9):.4f}   "
            f"smallest {min(ratios):.4f}   largest {max(ratios):.4f}")
    if both:
        wn = sum(float(rows[c]["work"]) for c in both if has_w(rows[c]) and has_w(base[c]))
        wb = sum(float(base[c]["work"]) for c in both if has_w(rows[c]) and has_w(base[c]))
        if wb > 0:
            out(f"work W summed over the cells both solve: {wn:.6g} against {wb:.6g} ({wn / wb:.6f})")
    return same


def counts_part(name, rows, folder, base_name, base, base_folder, same, out):
    """Part 3 for one arm: hardware counts on the cells where both ran the same search to an answer."""
    kept, bad_new, bad_base = [], [], []
    for c in same:
        a, a_cpu = read_counts(folder, c)
        b, b_cpu = read_counts(base_folder, c)
        if a is None:
            bad_new.append((c, a_cpu))
        if b is None:
            bad_base.append((c, b_cpu))
        if a is not None and b is not None and b["instructions"] > 0 and b["cycles"] > 0:
            kept.append((c, a, a_cpu, b, b_cpu))
    out(f"\n--- counts, {name} against {base_name}: {len(kept)} of the {len(same)} same-search cells "
        f"have complete counts in both ---")
    for who, missing in ((name, bad_new), (base_name, bad_base)):
        if missing:
            out(f"  {who}: {len(missing)} cells without usable counts, e.g. {short(missing[0][0])}: {missing[0][1]}")
    if not kept:
        return
    tot = lambda rs, k: sum(r[k] for r in rs)
    A = [a for _, a, _, _, _ in kept]
    B = [b for _, _, _, b, _ in kept]
    ir = [a["instructions"] / b["instructions"] for _, a, _, b, _ in kept]
    cr = [a["cycles"] / b["cycles"] for _, a, _, b, _ in kept]
    out(f"instructions: summed {tot(A, 'instructions') / tot(B, 'instructions'):.6f}; a cell: "
        f"smallest {min(ir):.6f}, median {pct(ir, 0.5):.6f}, largest {max(ir):.6f}; "
        f"within 0.2% on {sum(1 for x in ir if abs(x - 1) <= 0.002)} of {len(ir)} cells, "
        f"within 0.02% on {sum(1 for x in ir if abs(x - 1) <= 0.0002)}")
    out(f"clock ticks (cycles): summed {tot(A, 'cycles') / tot(B, 'cycles'):.4f}; a cell: geometric mean "
        f"{geomean(cr):.4f}, tenth to ninetieth percentile {pct(cr, 0.1):.4f} to {pct(cr, 0.9):.4f}, "
        f"smallest {min(cr):.4f}, largest {max(cr):.4f}")
    out(f"time on the processor: summed {sum(x for _, _, x, _, _ in kept) / sum(x for _, _, _, _, x in kept):.4f}; "
        f"wall time on the same cells: summed "
        f"{sum(rows[c]['time'] for c, *_ in kept) / sum(base[c]['time'] for c, *_ in kept):.4f}")
    share = lambda r: 100.0 * r["idq.mite_uops"] / (r["idq.mite_uops"] + r["idq.dsb_uops"])
    sa = 100.0 * tot(A, "idq.mite_uops") / (tot(A, "idq.mite_uops") + tot(A, "idq.dsb_uops"))
    sb = 100.0 * tot(B, "idq.mite_uops") / (tot(B, "idq.mite_uops") + tot(B, "idq.dsb_uops"))
    diffs = [share(a) - share(b) for _, a, _, b, _ in kept]
    out(f"share of micro-operations through the decoder: {name} {sa:.1f}%, {base_name} {sb:.1f}% (all cells "
        f"together); a cell's difference in points: median {pct(diffs, 0.5):+.2f}, "
        f"tenth to ninetieth percentile {pct(diffs, 0.1):+.2f} to {pct(diffs, 0.9):+.2f}")
    out(f"branch misses: summed {tot(A, 'branch-misses') / tot(B, 'branch-misses'):.4f}")
    out(f"cells grouped by the difference in decoder share ({name} minus {base_name}):")
    for label, test in (("higher by 1 point or more", lambda d: d >= 1.0),
                        ("within 1 point", lambda d: -1.0 < d < 1.0),
                        ("lower by 1 point or more", lambda d: d <= -1.0)):
        group = [i for i, d in enumerate(diffs) if test(d)]
        if group:
            out(f"  {label:26s} {len(group):4d} cells   clock ticks, geometric mean {geomean([cr[i] for i in group]):.4f}")
        else:
            out(f"  {label:26s}    0 cells")


def report(arms, baseline, timeout, floor, out=print):
    """arms: [(name, results path, perf folder or '')]. Returns 0, or 1 when part 0 found a failure."""
    global MANY_SEEDS
    data, live = {}, {}
    for name, path, _ in arms:
        data[name], live[name] = read_arm(path)
        for r in data[name].values():
            if not r["limit"]:      # a kept.tsv has no limit column: the run's limit is --timeout
                r["limit"] = str(timeout)
    folders = {name: folder for name, _, folder in arms}
    names = [name for name, _, _ in arms]
    cells = sorted(set.intersection(*(set(d) for d in data.values())))
    MANY_SEEDS = len({seed for d in data.values() for _, seed in d}) > 1
    if MANY_SEEDS and any(folders.values()):
        out("the count files carry no seed, so they cannot go with files of several seeds")
        return 1
    out("=== 0. Failures, over every row of every arm (all three must be 0) ===")
    failed = failures(data, live, out)
    out("\ncells in every arm: %d (%s)" % (len(cells), ", ".join(f"{n} has {len(data[n])}" for n in names)))
    if not cells:
        return 1
    priced = [c for c in cells if all(has_w(data[n][c]) for n in names)]
    scores = {n: score(data[n], cells, priced, timeout) for n in names}
    out(f"\n=== 1. Each arm (time limit {timeout} s; tick PAR-2 over the {len(priced)} cells where every arm "
        f"printed its work) ===")
    out(HEAD_FMT % ("arm", "solved", "SAT", "UNSAT", "timeouts", "mem cap", "early", "bad", "wall PAR-2",
                    "tick PAR-2", "solved part", "unsolved", "models ok", "proofs ok", "not done", "other"))
    for n in names:
        out(row_of(n, scores[n]))
    out("'mem cap': runs that ended at the memory cap. 'early' and 'bad' are the failures of part 0 among "
        "these cells. Checks: 'models ok' and 'proofs ok' are answers the checker confirmed; 'not done' are "
        "UNSAT answers whose proof the checker did not finish in its time; 'other' is any other check result "
        "of a counted answer.")
    b = scores[baseline]
    out(f"\nagainst {baseline}: " + "; ".join(
        f"{n} solved {scores[n]['solved'] - b['solved']:+d}, wall PAR-2 {scores[n]['par2'] / b['par2']:.4f}"
        + (f", tick PAR-2 {scores[n]['tick'] / b['tick']:.4f}" if b["tick"] > 0 else "")
        for n in names if n != baseline))
    out("\n=== 2. Each arm against the baseline, cell by cell ===")
    same_of = {}
    for n in names:
        if n != baseline:
            same_of[n] = against(n, data[n], baseline, data[baseline], cells, timeout, floor, out)
    if folders[baseline] and any(folders[n] for n in names if n != baseline):
        out("\n=== 3. Hardware counts (user space), on the cells where both ran the same search to an answer ===")
        for n in names:
            if n != baseline and folders[n]:
                counts_part(n, data[n], folders[n], baseline, data[baseline], folders[baseline], same_of[n], out)
    return 1 if failed else 0


def selftest():
    """Small arms with numbers that can be checked by hand."""
    head = ("config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\ttimeout"
            "\tverified\tticks\teliminate_resolutions\twork\tnote\n")
    S, U, T = "SATISFIABLE", "UNSATISFIABLE", "TIMEOUT"

    def write(path, tag, rows):
        with open(path, "w") as fh:
            fh.write(head)
            for cell, seed, res, t, conf, ver, work, *note in rows:
                fh.write(f"{tag}\t{cell}\t{seed}\t{res}\t{t}\t{conf}\t0\t0\t1800\t{ver}\t0\t0\t{work}\t"
                         f"{note[0] if note else ''}\n")

    def score_row(name, solved_, sat, unsat, timeouts, memory, early, bad, par2, tick_solved, tick_unsolved,
                  models, proofs, not_done, other):
        return ROW_FMT % (name, solved_, sat, unsat, timeouts, memory, early, bad, par2,
                          tick_solved + tick_unsolved, tick_solved, tick_unsolved, models, proofs, not_done, other)

    # First case: two arms, four cells, count files.
    h = "0123456789abcdef0123456789abcdef-"
    base = [(h + "c1", 0, S, 10.0, 5, "ok", 100), (h + "c2", 0, U, 100.0, 7, "ok", 300),
            (h + "c3", 0, T, 1800.0, 50, "skip", 900), (h + "c4", 0, T, 1800.0, 60, "skip", 500)]
    new = [(h + "c1", 0, S, 11.0, 5, "ok", 100), (h + "c2", 0, U, 90.0, 7, "checker-timeout", 300),
           (h + "c3", 0, S, 1700.0, 44, "ok", 850), (h + "c4", 0, T, 1800.0, 61, "skip", 520)]
    with tempfile.TemporaryDirectory(prefix="as-report-") as d:
        for tag, rows in (("base", base), ("new", new)):
            os.makedirs(os.path.join(d, tag, "perf"))
            write(os.path.join(d, tag, "results.tsv"), tag, rows)
        counts = {("base", "c1"): (1000, 500, 300, 100, 10), ("new", "c1"): (1001, 510, 280, 120, 10),
                  ("base", "c2"): (4000, 2000, 900, 100, 40), ("new", "c2"): (4000, 1900, 950, 50, 30)}
        for (tag, cell), vals in counts.items():
            with open(os.path.join(d, tag, "perf", h + cell + ".csv"), "w") as fh:
                fh.write("# started on a day\n\n")
                for ev, v in zip(EVENTS, vals):
                    fh.write(f"{v},,{ev}:u,{2000000000 if tag == 'base' else 1000000000},100.00,,\n")
        # a count file of the new arm for c3 that must not be used: an event counted for part of the run
        with open(os.path.join(d, "new", "perf", h + "c3.csv"), "w") as fh:
            fh.write("5,,instructions:u,10,62.50,,\n")
        lines = []
        rc = report([("base", os.path.join(d, "base", "results.tsv"), os.path.join(d, "base", "perf")),
                     ("new", os.path.join(d, "new", "results.tsv"), os.path.join(d, "new", "perf"))],
                    "base", 1800, 1.0, lines.append)
        text = "\n".join(lines)
        refused, why = read_counts(os.path.join(d, "new", "perf"), (h + "c3", "0"))
    want = [
        "over all arms and seeds): 0",
        "answers that do not count (a failed check, no proof, or a run that left a note): 0",
        "stops that are neither the clock nor the memory cap: 0",
        "cells in every arm: 4",
        # base: 10 + 100 + 3600 + 3600; W: 100 + 300 solved, 2 x (900 + 500) unsolved
        score_row("base", 2, 1, 1, 2, 0, 0, 0, 7310, 400, 2800, 1, 1, 0, 0),
        # new: 11 + 90 + 1700 + 3600; W: 100 + 300 + 850 solved, 2 x 520 unsolved
        score_row("new", 3, 2, 1, 1, 0, 0, 0, 5401, 1250, 1040, 2, 0, 1, 0),
        "new solved +1, wall PAR-2 0.7389, tick PAR-2 0.7156",
        "solved by both 2; only by new 1; only by base 0",
        "only new: c3",
        "(94% of the limit); the other: TIMEOUT",
        "the same search (equal conflict counts and equal work W) on 2 of the 2 cells both solve; "
        "equal conflict counts alone on 2",
        # ratios 1.1 and 0.9: summed 101/110, geometric mean sqrt(0.99)
        "summed time 0.9182   geometric mean 0.9950   median 0.9000",
        "work W summed over the cells both solve: 400 against 400 (1.000000)",
        "2 of the 2 same-search cells have complete counts in both",
        # instructions 5001/5000; a cell 1.001 and 1.000
        "instructions: summed 1.000200; a cell: smallest 1.000000, median 1.000000, largest 1.001000; "
        "within 0.2% on 2 of 2 cells, within 0.02% on 1",
        # cycles 2410/2500; a cell 1.02 and 0.95
        "clock ticks (cycles): summed 0.9640; a cell: geometric mean 0.9844",
        "time on the processor: summed 0.5000; wall time on the same cells: summed 0.9182",
        # decoder share: new (120+50)/(120+50+280+950) = 12.1%, base 200/1400 = 14.3%
        "share of micro-operations through the decoder: new 12.1%, base 14.3%",
        "branch misses: summed 0.8000",
        "higher by 1 point or more     1 cells   clock ticks, geometric mean 1.0200",
        "lower by 1 point or more      1 cells   clock ticks, geometric mean 0.9500",
    ]
    missing = [w for w in want if w not in text]
    if rc != 0 or missing or refused is not None or "62.50" not in why:
        print(text)
        print("\n[as_report] SELFTEST FAILED; missing:", *missing, sep="\n  ")
        return 1

    # Second case: three arms; two seeds of one instance; a bad answer; a contradiction between two
    # arms that are not the baseline; a cell with equal conflicts but unequal work; a run at the memory
    # cap, an UNKNOWN before the limit and a crash; and a failed answer on a cell only one arm has.
    oom = "memory allocation of 100 bytes failed"
    arms = {
        "a": [("x", 0, T, 1800, 9, "skip", 50), ("y", 0, S, 10, 3, "ok", 30), ("y", 1, T, 1800, 8, "skip", 60),
              ("z", 0, S, 5, 2, "ok", 20), ("w", 0, S, 10, 0, "ok", 100),
              ("m", 0, "UNKNOWN_rc-6", 30, 0, "skip", 10, oom), ("t", 0, T, 1800, 4, "skip", 10),
              # a crash that the clock then ended: recorded as a timeout, with the crash in the note
              ("p", 0, T, 1800, 4, "skip", 10, "thread 'main' panicked at src/y.rs")],
        "b": [("x", 0, S, 30, 4, "ok", 40), ("y", 0, T, 1800, 9, "skip", 70), ("y", 1, S, 20, 6, "ok", 45),
              ("z", 0, S, 1, 2, "FAIL", 20), ("w", 0, S, 10, 0, "ok", 200),
              ("m", 0, "UNKNOWN", 1, 0, "skip", 10), ("q", 0, U, 2, 1, "no-proof", 5),
              # a status line, then the run died on memory in the middle of its model
              ("t", 0, S, 900, 4, "FAIL", 10, "exit rc=-6 after the status line; " + oom),
              ("p", 0, T, 1800, 4, "skip", 10)],
        "c": [("x", 0, U, 40, 5, "ok", 45), ("y", 0, S, 12, 3, "ok", 30), ("y", 1, S, 13, 6, "ok", 45),
              ("z", 0, T, 1800, 7, "skip", 90), ("w", 0, S, 10, 0, "ok", 100),
              ("m", 0, "UNKNOWN_rc-6", 3, 0, "skip", 10, "thread 'main' panicked at src/x.rs"),
              # a status line, then the clock cut the model: a timeout, though the cut model fails its check
              ("t", 0, S, 1800, 4, "FAIL", 10, "exit rc=124 after the status line"),
              ("p", 0, T, 1800, 4, "skip", 10)],
    }
    with tempfile.TemporaryDirectory(prefix="as-report-") as d:
        for tag, rows in arms.items():
            write(os.path.join(d, tag + ".tsv"), tag, rows)
        lines = []
        rc2 = report([(tag, os.path.join(d, tag + ".tsv"), "") for tag in arms], "a", 1800, 1.0, lines.append)
        text = "\n".join(lines)
        # Third case: the same first two arms read from kept.tsv files, as during a run.
        for tag in ("a", "b"):
            with open(os.path.join(d, tag + ".kept.tsv"), "w") as fh:
                for cell, seed, res, t, conf, ver, work, *note in arms[tag]:
                    fh.write(f"{cell}\t{seed}\t{res}\t{t}\t{ver}\tc workclock ticks=1 conflicts={conf} work={work}\n")
        live_lines = []
        rc3 = report([(tag, os.path.join(d, tag + ".kept.tsv"), "") for tag in ("a", "b")], "a", 1800, 1.0,
                     live_lines.append)
        live_text = "\n".join(live_lines)
    want2 = [
        "over all arms and seeds): 1",
        "CONTRADICTION x (seed 0): SAT in b seed 0; UNSAT in c seed 0",
        # z of b fails its check; q, which only b has, is an UNSAT answer with no proof
        "answers that do not count (a failed check, no proof, or a run that left a note): 2",
        "BAD ANSWER b z (seed 0): SATISFIABLE, check FAIL",
        "BAD ANSWER b q (seed 0): UNSATISFIABLE, check no-proof",
        "stops that are neither the clock nor the memory cap: 3",
        "EARLY STOP a p (seed 0): TIMEOUT after 1800 s (thread 'main' panicked at src/y.rs)",
        "EARLY STOP b m (seed 0): UNKNOWN after 1 s (no note)",
        "EARLY STOP c m (seed 0): UNKNOWN_rc-6 after 3 s (thread 'main' panicked at src/x.rs)",
        "ended at the memory cap (an honest stop, priced as unsolved): a: m (seed 0); b: t (seed 0)",
        "cells in every arm: 8",
        # a: y seed 0, z, w solved (10 + 5 + 10); x, y seed 1 and t timeouts, m at the memory cap,
        # p a crash: 5 x 3600
        score_row("a", 3, 3, 0, 3, 1, 1, 0, 18025, 150, 280, 3, 0, 0, 0),
        # b: x, y seed 1, w solved (30 + 20 + 10); y seed 0 and p timeouts, t at the memory cap, m early,
        # z bad: 5 x 3600
        score_row("b", 3, 3, 0, 2, 1, 1, 1, 18060, 285, 240, 3, 0, 0, 0),
        # c: x, y seed 0, y seed 1, w solved (40 + 12 + 13 + 10); z, t and p timeouts, m early: 4 x 3600
        score_row("c", 4, 3, 1, 3, 0, 1, 0, 14475, 220, 240, 3, 1, 0, 0),
        "solved by both 1; only by b 2; only by a 2",
        "only b: y (seed 1)",
        "only a: y (seed 0)",
        "the other: SATISFIABLE (a bad answer)",
        "the same search (equal conflict counts and equal work W) on 0 of the 1 cells both solve; "
        "equal conflict counts alone on 1",
        # c against a: both solve y seed 0 and w, with equal conflicts and equal work
        "the same search (equal conflict counts and equal work W) on 2 of the 2 cells both solve; "
        "equal conflict counts alone on 2",
    ]
    want3 = [
        # no notes in kept.tsv: a's run at the memory cap has no reason and shows as an early stop
        "EARLY STOP a m (seed 0): UNKNOWN_rc-6 after 30 s (no note in kept.tsv)",
        "read from kept.tsv, which has no notes: a, b.",
        "cells in every arm: 8",
        # without notes a's crash p is a plain timeout and its memory stop m has no reason
        score_row("a", 3, 3, 0, 4, 0, 1, 0, 18025, 150, 280, 3, 0, 0, 0),
        # without its note, b's cut answer t cannot be told from a wrong one
        "BAD ANSWER b t (seed 0): SATISFIABLE, check FAIL",
    ]
    missing = [w for w in want2 if w not in text] + [w for w in want3 if w not in live_text]
    if rc2 != 1 or rc3 != 1 or missing:
        print(text)
        print(live_text)
        print("\n[as_report] SELFTEST FAILED (second or third case); missing:", *missing, sep="\n  ")
        return 1
    print(f"[as_report] selftest passed: {len(want) + len(want2) + len(want3)} lines checked by hand. A "
          "part-counted count file is refused; a failed answer and an answer without a proof are not counted, "
          "also on a cell only one arm has; both seeds are kept; a contradiction between two arms that are not "
          "the baseline is found; equal conflicts with unequal work are not the same search; a stop at the "
          "memory cap, an early UNKNOWN and a crash are told apart; an answer cut by the clock or by the memory "
          "cap is a stop, not a bad answer; a crash recorded as a timeout is a failure; a kept.tsv is read and marked as such.")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("arms", nargs="*", metavar="NAME=RESULTS.TSV[,PERF_FOLDER]")
    ap.add_argument("--baseline", default="")
    ap.add_argument("--timeout", type=int, default=1800, help="the runs' time limit in seconds")
    ap.add_argument("--floor", type=float, default=1.0)
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        return selftest()
    arms = []
    for spec in args.arms:
        if "=" not in spec:
            ap.error(f"{spec!r} is not NAME=RESULTS.TSV[,PERF_FOLDER]")
        name, rest = spec.split("=", 1)
        path, _, folder = rest.partition(",")
        if not os.path.isfile(path):
            ap.error(f"{name}: no such file: {path}")
        if folder and not os.path.isdir(folder):
            ap.error(f"{name}: no such folder: {folder}")
        arms.append((name, path, folder))
    names = [n for n, _, _ in arms]
    if len(arms) < 2 or len(set(names)) != len(names):
        ap.error("give two or more arms with different names")
    baseline = args.baseline or names[0]
    if baseline not in names:
        ap.error(f"--baseline {baseline!r} is not one of {names}")
    return report(arms, baseline, args.timeout, args.floor)


if __name__ == "__main__":
    sys.exit(main())
