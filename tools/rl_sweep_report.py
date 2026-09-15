#!/usr/bin/env python3
"""Per-arm, per-family and headroom report for a constant-knob sweep (RL plan step 0).

Reads a feature_ablation.py --arm run directory (one <arm>/results.tsv per arm, written when
the sweep ends) and prints, in the project's metric order (CLAUDE.md 'Evaluation'):

  0. correctness: answers that failed verification (verified=FAIL), UNSAT answers with no
     proof when proofs were on (verified=no-proof), a plain UNKNOWN that stopped before the
     wall budget (a premature non-budget UNKNOWN; wall sweeps set no other limit, so an honest
     UNKNOWN can only come from a run that used at least --budget-fraction of its timeout),
     abnormal exits (UNKNOWN_rc<n> other than the honest stops: 124 timeout, and SIGKILL or
     SIGTERM as 137/143 or Python's -9/-15, i.e. an external kill), solver panics, and
     instances where the solved answers disagree SAT v UNSAT across any arms and seeds. Any
     of these makes the report unfit for a decision; failed rows are never counted as solved.
     An answer whose process then died before finishing (note `exit rc=<n> after the status
     line`, e.g. killed or out of memory while printing the model) is never counted as solved
     either; its exit is classed by the same rules.
     SIGABRT (134 or -6) is ambiguous: solver 13 aborts both on an allocation failure under
     the memory ulimit (an honest stop) and on a panic (a bug). The harness's `note` column
     (the stderr line, since 2026-09-15) settles it; an abort with no recorded reason is listed
     as UNCLASSIFIED and also blocks the decision until the cell is rerun by hand, unless its
     instance is named in --memory-aborts. Tick-budgeted runs (SAT_LIMIT_TICKS, plan step A)
     will need the budget recorded per cell before their UNKNOWNs can be judged here.
  1. per arm: solved and wall PAR-2 on every common cell; tick PAR-2 on the work clock W
     (solved part and unsolved part shown separately) on the cells where every arm reported
     W; each ratio against the base arm.
  2. per family (first name token after the hash prefix, the repo's usual heuristic): the
     same per arm, written to <run>/report/per_family.tsv and printed for families where
     some arm differs from base.
  3. per knob (arms sharing a leading name, e.g. probeint50/probeint200): the best global
     constant, how often a non-stock constant beats or loses to stock per cell, and the
     headroom if every cell got its best constant for that knob.
  4. the joint headroom: every cell at its best arm over ALL knobs, an upper bound the epoch
     policy must beat (plan section 6.1 steps 2-3, 6.1b).

A cell is one (instance, seed). Tick PAR-2: a solved cell costs its W, an unsolved cell twice
the W it reached before the kill. Wall PAR-2 uses the TSV's timeout. "Best arm" for a cell is
always chosen solved-first, then cheapest, and that arm's solved flag and cost are used
together (a cheap timeout never masquerades as a solve). A "beats stock" cell is one the arm
solves and stock does not, or both solve and the arm's W is lower by more than --margin
(default 5%); "loses" is the mirror image.

  python3 tools/rl_sweep_report.py log/abtest-rl-step0-stage1-<ts> [--baseline base]
  python3 tools/rl_sweep_report.py --self-test
"""
from __future__ import annotations
import argparse, csv, math, re, sys, tempfile
from collections import defaultdict
from pathlib import Path

SOLVED = {"SAT", "UNSAT", "SATISFIABLE", "UNSATISFIABLE"}
Key = tuple  # (instance, seed)


def family(name: str) -> str:
    """First alphabetic token of the instance name after the hash prefix (select_profile20 rule)."""
    base = name.split("-", 1)[1] if "-" in name else name
    toks = [t for t in re.split(r"[-_0-9.]", base) if t]
    return toks[0].lower() if toks else base.lower()


KNOB_ALIASES = {"sweepoff": "sweepeffort"}   # plan section 2.1: sweep effort menu {0 = skip (--sweep=0), 0.5, 1, 2}


def knob_of(tag: str) -> str:
    """'probeint50' -> 'probeint'; 'reducefrachalf' -> 'reducefrac'; 'sweepoff' -> 'sweepeffort'; 'base' -> 'base'."""
    if tag in KNOB_ALIASES:
        return KNOB_ALIASES[tag]
    m = re.match(r"^([a-z]+?)(\d+|half|double|off|on)$", tag)
    return m.group(1) if m else tag


def load_arms(run: Path) -> dict[str, dict[Key, dict]]:
    arms: dict[str, dict[Key, dict]] = {}
    for tsv in sorted(run.glob("*/results.tsv")):
        with tsv.open(newline="") as f:
            rows = list(csv.DictReader(f, delimiter="\t"))
        if rows:
            arms[tsv.parent.name] = {(r["instance"], r.get("seed", "0")): r for r in rows}
    if not arms:
        sys.exit(f"{run}: no <arm>/results.tsv found (a sweep writes them when it ends)")
    return arms


HONEST_SIGNALS = {9, 15}      # SIGKILL, SIGTERM: an external kill (the OOM killer, or the harness)
BUDGET_FRACTION = 0.95        # a plain UNKNOWN below this share of the wall limit is a premature stop
UNCLASSIFIED = "UNCLASSIFIED abort (SIGABRT with no recorded reason: out-of-memory or panic)"
MEMORY_NOTE_RE = re.compile(r"memory allocation of .* failed|out of memory", re.I)
PANIC_NOTE_RE = re.compile(r"panicked", re.I)
MEMORY_ABORTS: set[str] = set()   # instances whose aborts were checked by hand (--memory-aborts)


def signal_of(rc_text: str) -> int | None:
    """Signal number behind an exit code: 128+N through `timeout`, -N from Python; None for a plain code."""
    try:
        rc = int(rc_text)
    except ValueError:
        return None
    return -rc if rc < 0 else rc - 128 if rc > 128 else None


INCOMPLETE = "exit rc="   # note prefix the harness writes when a status line was followed by an abnormal exit


def exit_kind(rc: str, note: str, instance: str) -> str:
    """'honest' (timeout, kill, or an out-of-memory abort), 'unclassified' (SIGABRT with no reason), or 'bug'."""
    if PANIC_NOTE_RE.search(note):
        return "bug"
    if rc == "124":
        return "honest"
    sig = signal_of(rc)
    if sig in HONEST_SIGNALS:
        return "honest"
    if sig == 6:   # SIGABRT: out-of-memory abort (honest) or a panic (bug); only the stderr note tells
        if MEMORY_NOTE_RE.search(note) or instance in MEMORY_ABORTS:
            return "honest"
        return "unclassified"
    return "bug"


def interrupted(r: dict) -> bool:
    """A status line (SAT, UNSAT or UNKNOWN) was printed but the process then exited abnormally."""
    return (r.get("note", "") or "").startswith(INCOMPLETE)


def incomplete(r: dict) -> bool:
    """An answer whose process died before finishing (killed or aborted mid-model): never a solve."""
    return r["result"].upper() in SOLVED and interrupted(r)


def failed(r: dict, budget_fraction: float = BUDGET_FRACTION) -> str:
    """Why this row is a correctness failure (or unclassified), or '' when it is an honest stop."""
    ver = r.get("verified", "").strip()
    res = r["result"].upper()
    note = r.get("note", "") or ""
    if interrupted(r):
        # Classed by the exit first: a truncated model after a timeout kill fails verification
        # too, but the kill is the honest stop, so the row is simply unsolved.
        rc = note[len(INCOMPLETE):].split()[0]
        kind = exit_kind(rc, note, r["instance"])
        if kind == "bug":
            return f"{res} printed but the solver died: {note[:100]}"
        return UNCLASSIFIED if kind == "unclassified" else ""
    if ver.upper() == "FAIL":
        return "verified=FAIL"
    if ver == "no-proof":
        return "no proof written for an UNSAT answer"
    if PANIC_NOTE_RE.search(note):
        return f"solver panic: {note[:100]}"
    if res.startswith("UNKNOWN_RC"):
        rc = res[len("UNKNOWN_RC"):]
        kind = exit_kind(rc, note, r["instance"])
        if kind == "bug":
            return f"abnormal exit rc={rc}" + (f": {note[:80]}" if note else "")
        return UNCLASSIFIED if kind == "unclassified" else ""
    elif res == "UNKNOWN":
        t, limit = float(r.get("time_s", 0) or 0), float(r.get("timeout", 0) or 0)
        if limit <= 0 or t < budget_fraction * limit:
            return f"premature UNKNOWN after {t:.0f} s of a {limit:.0f} s budget"
    return ""


def is_solved(r: dict) -> bool:
    """A failed answer is a correctness failure, never a solve; an answer cut short is not one either."""
    return r["result"].upper() in SOLVED and not failed(r) and not incomplete(r)


def answer(r: dict) -> str:
    return r["result"].upper()[:3]  # SAT / UNS


def work(r: dict) -> float | None:
    v = r.get("work", "NA")
    return None if v in ("", "NA") else float(v)


def wall_cost(r: dict) -> float:
    return float(r["time_s"]) if is_solved(r) else 2.0 * float(r["timeout"])


def tick_cost(r: dict) -> float:
    w = work(r)
    return w if is_solved(r) else 2.0 * w


def score(cells: dict[Key, dict], keys_all: list[Key], keys_priced: list[Key]) -> dict:
    """solved and wall on every common cell; tick on the cells priced on W."""
    return {
        "n": len(keys_all),
        "solved": sum(is_solved(cells[k]) for k in keys_all),
        "wall": sum(wall_cost(cells[k]) for k in keys_all),
        "n_priced": len(keys_priced),
        "tick": sum(tick_cost(cells[k]) for k in keys_priced),
        "tick_unsolved": sum(tick_cost(cells[k]) for k in keys_priced if not is_solved(cells[k])),
    }


def ratio(a: float, b: float) -> str:
    return f"{a / b:.4f}" if b > 0 else "  n/a"


def differs(a: float, b: float, margin: float) -> bool:
    """True when two positive totals differ by more than margin, or exactly one of them is zero."""
    if a <= 0 or b <= 0:
        return a != b
    return abs(math.log(a / b)) > abs(math.log(1 - margin))


def better(a: dict, b: dict, margin: float) -> bool:
    """Cell-level: a solves what b does not, or both solve and a's W is lower by > margin (needs W in both)."""
    if is_solved(a) != is_solved(b):
        return is_solved(a)
    if not is_solved(a):
        return False
    wa, wb = work(a), work(b)
    return wa is not None and wb is not None and wa < (1 - margin) * wb


def best_arm(arms: dict[str, dict[Key, dict]], tags: list[str], k: Key, cost) -> str:
    """Solved first, then cheapest by `cost`; ties go to the earliest tag (base first)."""
    return min(tags, key=lambda t: (not is_solved(arms[t][k]), cost(arms[t][k]), tags.index(t)))


def report(run: Path, base: str, margin: float, min_family: int, memory_aborts: set[str] | None = None) -> str:
    MEMORY_ABORTS.clear()
    MEMORY_ABORTS.update(memory_aborts or ())
    arms = load_arms(run)
    if base not in arms:
        sys.exit(f"baseline arm {base!r} not in {sorted(arms)}")
    tags = [base] + sorted(t for t in arms if t != base)
    common = sorted(set.intersection(*(set(arms[t]) for t in tags)))
    priced = [k for k in common if all(work(arms[t][k]) is not None for t in tags)]
    dropped = len(common) - len(priced)
    seeds = sorted({k[1] for k in common})
    out = [f"sweep {run}: {len(tags)} arms, {len(common)} common cells ({len({k[0] for k in common})} instances x "
           f"{len(seeds)} seed{'s' if len(seeds) != 1 else ''}), {len(priced)} priced on W"
           + (f" ({dropped} not priced: no work line in some arm; still counted in solved and wall)" if dropped else "")]

    # 0. correctness: failed rows, and SAT v UNSAT disagreement per INSTANCE across all arms and seeds
    fails = [(t, k, failed(arms[t][k])) for t in tags for k in arms[t] if failed(arms[t][k])]
    by_inst: dict[str, dict[str, str]] = defaultdict(dict)
    for t in tags:
        for k, r in arms[t].items():
            if is_solved(r):
                by_inst[k[0]][f"{t}/s{k[1]}"] = answer(r)
    contra = [(inst, answers) for inst, answers in sorted(by_inst.items()) if len(set(answers.values())) > 1]
    unverified = sum(1 for t in tags for k in common if is_solved(arms[t][k])
                     and arms[t][k].get("verified", "").strip() not in ("ok", ""))
    unclassified = [(t, k) for t, k, why in fails if why == UNCLASSIFIED]
    if fails or contra:
        out.append(f"*** CORRECTNESS FAILURES: {len(fails) - len(unclassified)} failed rows, {len(unclassified)} "
                   f"unclassified aborts, {len(contra)} instances with SAT/UNSAT contradictions across arms and seeds. "
                   "Wrong answers fail every gate with no trade (CLAUDE.md); this report is NOT decision evidence "
                   "until they are debugged (an unclassified abort: rerun the cell by hand with stderr, then pass "
                   "--memory-aborts <instance> if it was the memory limit). Failed rows count as unsolved below.")
        for t, k, why in fails[:10]:
            out.append(f"    {why}: arm {t}  {k[0][:60]} seed {k[1]}  reported {arms[t][k]['result']}")
        for inst, answers in contra[:10]:
            out.append(f"    contradiction  {inst[:60]}: " + ", ".join(f"{a}={v}" for a, v in sorted(answers.items())))
    else:
        out.append("correctness: no failed rows, no SAT/UNSAT contradictions across arms and seeds")
    out.append(f"solved answers not independently verified (verify off, skipped, or checker budget): {unverified}")
    cut = [(t, k) for t in tags for k in common if incomplete(arms[t][k]) and not failed(arms[t][k])]
    if cut:
        out.append(f"answers stopped after the status line by an honest kill or memory abort (counted unsolved): {len(cut)}  "
                   + ", ".join(f"{t}/{k[0][:40]}/s{k[1]}" for t, k in cut[:6]))

    # 1. per arm
    sc = {t: score(arms[t], common, priced) for t in tags}
    b = sc[base]
    out += ["", f"{'arm':<20} {'solved':>8} {'wall PAR-2':>11} {'ratio':>7} {'tick PAR-2':>12} {'ratio':>7} {'unsolved part':>14}   +solved/-solved v base"]
    for t in tags:
        s = sc[t]
        won = sum(1 for k in common if is_solved(arms[t][k]) and not is_solved(arms[base][k]))
        lost = sum(1 for k in common if is_solved(arms[base][k]) and not is_solved(arms[t][k]))
        out.append(f"{t:<20} {s['solved']:>4}/{s['n']:<3} {s['wall']:>11.1f} {ratio(s['wall'], b['wall']):>7} "
                   f"{s['tick']:>12.4g} {ratio(s['tick'], b['tick']):>7} {s['tick_unsolved']:>14.4g}   +{won}/-{lost}"
                   + ("   (base)" if t == base else ""))

    # 2. per family
    fam_all, fam_priced = defaultdict(list), defaultdict(list)
    for k in common:
        fam_all[family(k[0])].append(k)
    for k in priced:
        fam_priced[family(k[0])].append(k)
    rep = run / "report"
    rep.mkdir(exist_ok=True)
    with (rep / "per_family.tsv").open("w") as f:
        f.write("family\tcells\tpriced\tarm\tsolved\twall_par2\twall_ratio_v_base\ttick_par2\ttick_ratio_v_base\n")
        for fam in sorted(fam_all):
            fb = score(arms[base], fam_all[fam], fam_priced[fam])
            for t in tags:
                s = score(arms[t], fam_all[fam], fam_priced[fam])
                f.write(f"{fam}\t{len(fam_all[fam])}\t{len(fam_priced[fam])}\t{t}\t{s['solved']}\t{s['wall']:.1f}\t"
                        f"{ratio(s['wall'], fb['wall']).strip()}\t{s['tick']:.6g}\t{ratio(s['tick'], fb['tick']).strip()}\n")
    out += ["", f"per family (>= {min_family} cells, where some arm differs from base in solved or in tick PAR-2 by "
            f"> {margin:.0%}; full table: {rep / 'per_family.tsv'})"]
    for fam in sorted(fam_all, key=lambda x: (-len(fam_all[x]), x)):
        keys = fam_all[fam]
        if len(keys) < min_family:
            continue
        fb = score(arms[base], keys, fam_priced[fam])
        diffs = []
        for t in tags[1:]:
            s = score(arms[t], keys, fam_priced[fam])
            if s["solved"] != fb["solved"] or differs(s["tick"], fb["tick"], margin):
                r = ratio(s["tick"], fb["tick"]).strip()
                diffs.append(f"{t} {s['solved'] - fb['solved']:+d} {r}{'x' if r != 'n/a' else ''}")
        if diffs:
            out.append(f"  {fam:<18} {len(keys):>3} cells  base {fb['solved']}/{len(keys)}:  " + ", ".join(diffs))

    # 3. per knob
    knobs = defaultdict(list)
    for t in tags[1:]:
        knobs[knob_of(t)].append(t)
    out += ["", f"per knob (a constant beats stock on a cell when it solves what stock does not, or both solve and its W is lower by > {margin:.0%})",
            f"{'knob':<18} {'best global':<20} {'best/base':>9} {'tick ratio':>10} {'wall ratio':>10} {'beats':>6} {'loses':>6} {'oracle solved':>13} {'oracle tick':>11}"]
    ranking = []
    for knob in sorted(knobs):
        members = knobs[knob]
        best = min(members, key=lambda t: (-sc[t]["solved"], sc[t]["tick"], sc[t]["wall"]))
        beats = sum(1 for k in common if any(better(arms[t][k], arms[base][k], margin) for t in members))
        loses = sum(1 for k in common if any(better(arms[base][k], arms[t][k], margin) for t in members))
        group = [base] + members
        picks = {k: best_arm(arms, group, k, tick_cost) for k in priced}
        oracle_tick = sum(tick_cost(arms[picks[k]][k]) for k in priced)
        oracle_solved = sum(is_solved(arms[picks[k]][k]) for k in priced)
        gain = 1 - oracle_tick / b["tick"] if b["tick"] > 0 else 0.0
        ranking.append((gain, knob, oracle_solved - sum(is_solved(arms[base][k]) for k in priced)))
        out.append(f"{knob:<18} {best:<20} {sc[best]['solved']:>4}/{b['solved']:<4} {ratio(sc[best]['tick'], b['tick']):>10} "
                   f"{ratio(sc[best]['wall'], b['wall']):>10} {beats:>6} {loses:>6} {oracle_solved:>8}/{len(priced):<4} {ratio(oracle_tick, b['tick']):>11}")

    # 4. joint headroom
    pick_t = {k: best_arm(arms, tags, k, tick_cost) for k in priced}
    pick_w = {k: best_arm(arms, tags, k, wall_cost) for k in common}
    joint_tick = sum(tick_cost(arms[pick_t[k]][k]) for k in priced)
    joint_solved_priced = sum(is_solved(arms[pick_t[k]][k]) for k in priced)
    base_solved_priced = sum(is_solved(arms[base][k]) for k in priced)
    joint_wall = sum(wall_cost(arms[pick_w[k]][k]) for k in common)
    joint_solved = sum(is_solved(arms[pick_w[k]][k]) for k in common)
    out += ["", f"joint headroom (every cell at its best arm over all {len(tags) - 1} constants, chosen solved-first then cheapest; an upper bound for the policy):",
            f"  on all {len(common)} cells: solved {joint_solved} v stock {b['solved']} ({joint_solved - b['solved']:+d}), wall PAR-2 ratio {ratio(joint_wall, b['wall'])}",
            f"  on the {len(priced)} priced cells: solved {joint_solved_priced} v stock {base_solved_priced}, tick PAR-2 ratio {ratio(joint_tick, b['tick'])}",
            "  knobs ranked by per-knob oracle tick PAR-2 gain (solved gain on priced cells in brackets):"]
    for gain, knob, ds in sorted(ranking, reverse=True):
        out.append(f"    {knob:<18} {gain:+.2%}  [{ds:+d}]")
    if fails or contra:
        out.append("*** NOT DECISION EVIDENCE: correctness failures above.")
    text = "\n".join(out) + "\n"
    (rep / "report.txt").write_text(text)
    return text


HEADER = ("config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\ttimeout\tverified"
          "\tticks\teliminate_resolutions\twork\n")


def _write(run: Path, arm: str, rows: list[tuple]) -> None:
    """rows: (instance, seed, result, time, work, verified[, note])."""
    d = run / arm
    d.mkdir(parents=True, exist_ok=True)
    with (d / "results.tsv").open("w") as f:
        f.write(HEADER.rstrip("\n") + "\tnote\n")
        for inst, seed, res, t, w, ver, *note in rows:
            f.write(f"{arm}\t{inst}\t{seed}\t{res}\t{t}\t1\t1\t1\t600\t{ver}\t{w}\t0\t{w}\t{note[0] if note else ''}\n")


def self_test() -> None:
    S, U, T = "SATISFIABLE", "UNSATISFIABLE", "TIMEOUT"
    with tempfile.TemporaryDirectory() as tmp:
        run = Path(tmp) / "sweep"
        # 4 instances x 2 seeds; families: alpha (h1, h2), beta (h3, h4). W = work; timeout 600.
        _write(run, "base", [
            ("h1-alpha_a", 0, S, 100, 1000, "ok"), ("h1-alpha_a", 1, S, 100, 1000, "ok"),
            ("h2-alpha_b", 0, U, 200, 2000, "ok"), ("h2-alpha_b", 1, U, 200, 2000, "ok"),
            ("h3-beta_a", 0, S, 300, 3000, "ok"), ("h3-beta_a", 1, T, 600, 6000, "skip"),
            ("h4-beta_b", 0, T, 600, 6000, "skip"), ("h4-beta_b", 1, S, 100, "NA", "ok")])   # seed 1 of h4: no work line
        _write(run, "probeint50", [
            ("h1-alpha_a", 0, S, 100, 1000, "ok"), ("h1-alpha_a", 1, S, 100, 1000, "ok"),
            ("h2-alpha_b", 0, U, 150, 1500, "ok"), ("h2-alpha_b", 1, U, 200, 2000, "ok"),       # seed 0 beats stock by 25%
            ("h3-beta_a", 0, T, 600, 10, "skip"), ("h3-beta_a", 1, S, 500, 5000, "ok"),       # seed 0: cheap timeout (W=10) must not become an oracle "solve"
            ("h4-beta_b", 0, T, 600, 6000, "skip"), ("h4-beta_b", 1, T, 600, 6000, "skip")])  # seed 1 loses (stock solved)
        _write(run, "probeint200", [
            ("h1-alpha_a", 0, S, 100, 1000, "FAIL"), ("h1-alpha_a", 1, S, 100, 1000, "ok"),    # verified=FAIL: never a solve
            ("h2-alpha_b", 0, U, 250, 2500, "ok"), ("h2-alpha_b", 1, U, 200, 2000, "ok"),
            ("h3-beta_a", 0, S, 300, 3000, "ok"), ("h3-beta_a", 1, U, 400, 4000, "ok"),       # seed 1: UNSAT while probeint50 says SAT and base timed out
            ("h4-beta_b", 0, T, 600, 6000, "skip"), ("h4-beta_b", 1, S, 100, 1000, "ok")])
        text = report(run, "base", 0.05, 1)
    lines = text.splitlines()
    def row(prefix):
        return next(l for l in lines if l.startswith(prefix))
    def row_of(txt, prefix):
        return next(l for l in txt.splitlines() if l.startswith(prefix))
    assert "8 common cells (4 instances x 2 seeds), 7 priced on W" in lines[0], lines[0]
    assert "1 failed rows, 0 unclassified aborts, 1 instances with SAT/UNSAT contradictions" in text, text
    assert "contradiction  h3-beta_a: base/s0=SAT, probeint200/s0=SAT, probeint200/s1=UNS, probeint50/s1=SAT" in text, text
    # base: solved 6/8 (h3 s1, h4 s0 unsolved); probeint50: 5/8 (loses h4 s1, wins h3 s1); probeint200: 6/8 (h1 s0 FAIL, wins h3 s1)
    assert row("base").split()[1] == "6/8", row("base")
    assert row("probeint50").split()[1] == "5/8" and row("probeint50").rstrip().endswith("+1/-2"), row("probeint50")
    assert row("probeint200").split()[1] == "6/8" and row("probeint200").rstrip().endswith("+1/-1"), row("probeint200")
    # tick on 7 priced cells (h4 s1 excluded): base = 1000+1000+2000+2000+3000+12000+12000 = 33000
    assert row("base").split()[4] == "3.3e+04", row("base")
    # probeint50 priced: 1000+1000+1500+2000+20+5000+12000 = 22520 -> ratio 0.6824
    assert row("probeint50").split()[5] == "0.6824", row("probeint50")
    # per knob probeint: oracle over {base, probeint50, probeint200} solved-first: h1s0 1000 (base; probeint200 FAIL), h1s1 1000,
    # h2s0 1500, h2s1 2000, h3s0 3000 (base solved; probeint50's W=10 timeout must not win), h3s1 4000 (probeint200 solved, cheaper than 5000),
    # h4s0 12000 (nobody solves; min 2*6000) -> 24500 -> 0.7424; oracle solved 6/7
    knob = row("probeint ")
    assert knob.split()[-1] == "0.7424" and "6/7" in knob, knob
    assert "solved 7 v stock 6 (+1)" in text, text   # joint on all cells: h4 s1 solved by base, h3 s1 by probeint50/200, h4 s0 by nobody
    assert "NOT DECISION EVIDENCE" in text
    # family line: beta differs (solved and tick), alpha differs for probeint50 (h2 s0 -25% W) and probeint200 (FAIL -> -1 solved)
    assert row("  alpha").count("probeint50") == 1 and "probeint200 -1" in row("  alpha"), row("  alpha")
    # no-proof, abnormal exit, honest memory abort, and a cross-seed contradiction within ONE arm
    with tempfile.TemporaryDirectory() as tmp:
        run = Path(tmp) / "f"
        _write(run, "base", [("h1-x_a", 0, U, 1, 10, "no-proof"), ("h2-y_a", 0, S, 1, 10, "ok"),
                             ("h3-z_a", 0, "UNKNOWN_rc134", 1, 5, "skip", "memory allocation of 8589934592 bytes failed"),
                             ("h4-w_a", 0, S, 1, 10, "ok"), ("h4-w_a", 1, U, 1, 10, "ok"),
                             ("h5-v_a", 0, "UNKNOWN", 590, 5, "skip"), ("h6-u_a", 0, "UNKNOWN_rc-9", 1, 5, "skip"),
                             ("h7-t_a", 0, "UNKNOWN_rc134", 1, 5, "skip", "thread 'main' panicked at src/x.rs:1:1:")])
        _write(run, "knob1", [("h1-x_a", 0, U, 1, 10, "ok"), ("h2-y_a", 0, "UNKNOWN_rc-11", 1, 5, "off"),
                              ("h3-z_a", 0, "UNKNOWN_rc-6", 1, 5, "skip"), ("h4-w_a", 0, S, 1, 10, "ok"),
                              ("h4-w_a", 1, S, 1, 10, "ok"), ("h5-v_a", 0, "UNKNOWN", 1, 5, "skip"),
                              ("h6-u_a", 0, "UNKNOWN_rc143", 1, 5, "skip"), ("h7-t_a", 0, S, 1, 10, "ok"),
                              ("h8-s_a", 0, S, 1, 10, "off", "exit rc=134 after the status line; thread 'main' panicked at src/witness.rs:9:9:"),
                              ("h9-r_a", 0, U, 1, 10, "off", "exit rc=134 after the status line")])
        _write(run, "knob2", [("h1-x_a", 0, U, 1, 10, "ok"), ("h2-y_a", 0, S, 1, 10, "ok"), ("h3-z_a", 0, S, 1, 10, "ok"),
                              ("h4-w_a", 0, S, 1, 10, "ok"), ("h4-w_a", 1, U, 1, 10, "ok"), ("h5-v_a", 0, S, 1, 10, "ok"),
                              ("h6-u_a", 0, S, 1, 10, "ok"),
                              ("h7-t_a", 0, S, 1, "NA", "off", "exit rc=134 after the status line; memory allocation of 99 bytes failed"),
                              ("h8-s_a", 0, S, 1, "NA", "off", "exit rc=124 after the status line"),
                              ("h9-r_a", 0, S, 1, 10, "off", "exit rc=-11 after the status line")])
        text = report(run, "base", 0.05, 1)
        assert "6 failed rows, 2 unclassified aborts, 1 instances with SAT/UNSAT contradictions" in text, text
        assert "SATISFIABLE printed but the solver died: exit rc=134 after the status line; thread 'main' panicked at src/witness.rs:9:9:: arm knob1  h8-s_a" in text, text
        assert "UNCLASSIFIED abort (SIGABRT with no recorded reason: out-of-memory or panic): arm knob1  h9-r_a seed 0" in text, text
        assert "SATISFIABLE printed but the solver died: exit rc=-11 after the status line: arm knob2  h9-r_a seed 0" in text, text
        # h8 is not a common cell (base lacks it), so only knob2's h7 (memory abort mid-model) is listed; it is no solve
        assert "answers stopped after the status line by an honest kill or memory abort (counted unsolved): 1  knob2/h7-t_a/s0" in text, text
        assert row_of(text, "knob2").split()[1] == "7/8", text
        assert "no proof written for an UNSAT answer: arm base  h1-x_a seed 0" in text, text
        assert "abnormal exit rc=-11: arm knob1  h2-y_a seed 0" in text, text
        assert "premature UNKNOWN after 1 s of a 600 s budget: arm knob1  h5-v_a seed 0" in text, text
        assert "solver panic: thread 'main' panicked at src/x.rs:1:1:: arm base  h7-t_a seed 0" in text, text
        assert "UNCLASSIFIED abort (SIGABRT with no recorded reason: out-of-memory or panic): arm knob1  h3-z_a seed 0" in text, text
        for honest in ("arm base  h3-z_a", "h6-u_a", "590 s"):   # honest stops (memory-noted abort, kills, budget UNKNOWN) are never listed
            assert honest not in text, (honest, text)
        # h8/h9 exist only in knob1/knob2, so they are not common cells: base 3/8 and knob1 4/8 stand
        assert row_of(text, "base").split()[1] == "3/8" and row_of(text, "knob1").split()[1] == "4/8", text
        text2 = report(run, "base", 0.05, 1, memory_aborts={"h3-z_a", "h9-r_a"})   # hand-checked memory stops
        assert "6 failed rows, 0 unclassified aborts" in text2 and "UNCLASSIFIED" not in text2, text2
    assert knob_of("sweepoff") == "sweepeffort" == knob_of("sweepeffort200") and knob_of("reducefrachalf") == "reducefrac"
    assert [signal_of(x) for x in ("124", "134", "137", "143", "-6", "-9", "-15", "1", "0", "x")] == \
        [None, 6, 9, 15, 6, 9, 15, None, None, None]
    # interrupted rows: a truncated model that fails verification after a timeout kill is an honest stop; an
    # UNKNOWN followed by a crash is a bug; an UNKNOWN killed at the wall limit is honest whatever its time_s
    with tempfile.TemporaryDirectory() as tmp:
        run = Path(tmp) / "i"
        _write(run, "base", [("h1-x_a", 0, S, 600, "NA", "FAIL", "exit rc=124 after the status line"),
                             ("h2-y_a", 0, "UNKNOWN", 599, 5, "skip", "exit rc=-11 after the status line"),
                             ("h3-z_a", 0, "UNKNOWN", 5, "NA", "skip", "exit rc=124 after the status line")])
        _write(run, "knob1", [("h1-x_a", 0, S, 1, 10, "ok"), ("h2-y_a", 0, S, 1, 10, "ok"), ("h3-z_a", 0, S, 1, 10, "ok")])
        text = report(run, "base", 0.05, 1)
        assert "1 failed rows, 0 unclassified aborts, 0 instances" in text, text
        assert "UNKNOWN printed but the solver died: exit rc=-11 after the status line: arm base  h2-y_a" in text, text
        assert "verified=FAIL" not in text and "premature" not in text, text
        assert "(counted unsolved): 1  base/h1-x_a/s0" in text and row_of(text, "base").split()[1] == "0/3", text
    # zero-work families must not crash: rerun with a zero-W cell
    with tempfile.TemporaryDirectory() as tmp:
        run = Path(tmp) / "z"
        _write(run, "base", [("h1-x_a", 0, S, 1, 2, "ok")])
        _write(run, "knob1", [("h1-x_a", 0, S, 1, 0, "ok")])
        text = report(run, "base", 0.05, 1)
        assert "knob1 +0 0.0000x" in text, text
    print("rl_sweep_report self-test ok")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run", type=Path, nargs="?")
    ap.add_argument("--baseline", default="base")
    ap.add_argument("--margin", type=float, default=0.05, help="W margin for a per-cell win (default 0.05)")
    ap.add_argument("--min-family", type=int, default=1, help="print families with at least this many cells")
    ap.add_argument("--memory-aborts", default="",
                    help="comma-separated instances whose SIGABRT exits were checked by hand and were the memory "
                         "limit (an honest stop), for TSVs without a `note` column")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        self_test()
        return 0
    if a.run is None:
        ap.error("run directory required (or --self-test)")
    aborts = {x.strip() for x in a.memory_aborts.split(",") if x.strip()}
    print(report(a.run, a.baseline, a.margin, a.min_family, aborts), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
