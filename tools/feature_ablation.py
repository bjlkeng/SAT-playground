#!/usr/bin/env python3
"""Current-solver feature ablation on the profile20 suite.

Process (per the 2026-05-30 design):
  * Target suite = benchmarks/profile20 (10 easy control + 10 hard headroom).
  * Decision metric = aggregate PAR-2 over all 20, with easy-10 / hard-10 reported separately.
  * Parallelism = --jobs workers pinned to physical cores 0..jobs-1 (taskset), one bench -j1 each
    over a jobs-way shard; siblings left idle for clean timing. Default 4 jobs; for faster feature
    iteration on a quiet host use --jobs 5 (cores 0-4). Cap --mem-mb so jobs x mem fits RAM.
  * 3% repeat rule: a config within +/-3% of the baseline PAR-2 is in the noise band -> rerun
    (up to n=3, take the mean); a clear (>3%) win/loss is accepted from n=1.
  * Two stages: Stage 1 screens every config at 300 s on all 20; Stage 2 (separate invocation)
    re-runs a shortlist at a long timeout on the hard-10 only to measure real headroom.

Each config is a same-binary SAT_* env toggle on the current solver (built once); `solver10`
defaults to the solver/10-bve-subsume reference floor. Flag `requires` deps (CONFIG_SCHEMA.csv) are
encoded so no toggle is a silent no-op; parent-only controls (use_lbd, lbd_tiered, fstab) are
included for attribution.

Usage:
  python3 tools/feature_ablation.py --stage1 [--timeout 300] [--mem-mb 14000] [--jobs 4]
  python3 tools/feature_ablation.py --stage2 --configs tag1,tag2 [--timeout 900]
  python3 tools/feature_ablation.py --seedgate --configs <tag> --seeds 5 --jobs 5   # 5 threads x 5 seeds
  # measure a NEW feature flag ad-hoc (no CONFIG_MAP edit) — A/B candidate vs baseline:
  python3 tools/feature_ablation.py --seedgate --env "SAT_NEWFEAT=on" --tag newfeat --seeds 5 --jobs 5
  python3 tools/feature_ablation.py --seedgate --env ""              --tag baseline --seeds 5 --jobs 5
  # A/B (or N-way) with a SIMULTANEOUS, interleaved, fair shared-core run (defaults: 32 cores, 16GB, 30min):
  python3 tools/feature_ablation.py --arm "cand:SAT_NEWFEAT=on" --arm "base:"            # A vs B, one shot
  python3 tools/feature_ablation.py --arm "cand:SAT_X=on" --arm "base:" --arm solver10   # N-way + s10 floor
  python3 tools/feature_ablation.py --smoke           # 2 configs x 2 instances, fast self-check

Before launching a parallel sweep, check for competing solver/bench processes (the script prints a
contention warning at startup; the agent must `ps`/`pgrep` and ASK the user first per CLAUDE.md).
"""
from __future__ import annotations
import argparse, csv, os, shutil, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import compare_bench  # noqa: E402

SUITE = ROOT / "benchmarks" / "profile20"
SELECTION = SUITE / "selection.csv"


def _solver_sort_key(path: Path) -> tuple[int, str]:
    prefix = path.name.split("-", 1)[0]
    try:
        return int(prefix), path.name
    except ValueError:
        return -1, path.name


def current_solver(default: str | None = None) -> str:
    candidate = (
        default
        or os.environ.get("SAT_TARGET_SOLVER")
        or os.environ.get("SAT_CURRENT_SOLVER")
        or os.environ.get("SAT_SOLVER")
    )
    if candidate:
        path = Path(candidate)
        if path.is_absolute():
            try:
                return str(path.resolve().relative_to(ROOT))
            except ValueError:
                return str(path)
        return candidate
    solvers = [
        path for path in (ROOT / "solver").glob("[0-9][0-9]-*")
        if (path / "build.sh").is_file() and (path / "run.sh").is_file()
    ]
    if not solvers:
        raise SystemExit("no solver/NN-* directory with build.sh and run.sh found")
    return str(sorted(solvers, key=_solver_sort_key)[-1].relative_to(ROOT))


S11 = current_solver()
S10 = os.environ.get("SAT_REFERENCE_SOLVER", "solver/10-bve-subsume")
CORES = [0, 1, 2, 3]            # default worker cores; overridden to range(--jobs) by preflight()
TOL = 0.03                      # 3% noise band for the repeat rule


def preflight(args) -> None:
    """Pin workers to cores 0..jobs-1 and warn about contention / memory before a sweep.

    --jobs sets BOTH the worker count and the physical cores used (taskset 0..jobs-1), so
    `--jobs 5 --seeds 5` runs 5 threads (cores 0-4) with 5 seeds/instance. We do NOT prompt here
    (sweeps usually run backgrounded); we print loud warnings. The agent-facing rule (CLAUDE.md)
    is to check `ps`/`pgrep` and ASK the user before launching when other solver jobs are running.
    """
    global CORES
    CORES = list(range(max(1, args.jobs)))
    # memory sanity: jobs x per-job cap must fit physical RAM or the OOM-killer reaps workers
    try:
        total_mb = next(int(l.split()[1]) for l in open("/proc/meminfo")
                        if l.startswith("MemTotal")) // 1024
        want_mb = args.jobs * args.mem_mb
        if want_mb > total_mb * 9 // 10:
            safe = total_mb * 9 // 10 // max(1, args.jobs)
            print(f"[preflight] WARNING: {args.jobs} jobs x {args.mem_mb}MB = {want_mb}MB > 90% of "
                  f"{total_mb}MB RAM; lower --mem-mb to ~{safe} to avoid OOM-kill.", flush=True)
    except Exception:
        pass
    # contention: any other solver/bench process on the same cores taints PAR-2 timing
    try:
        me = str(os.getpid())
        out = subprocess.run(
            ["pgrep", "-af", r"sat-solver|kissat|minisat|tools/bench\.sh|feature_ablation|hard_search"],
            stdout=subprocess.PIPE, text=True).stdout
        others = [l for l in out.splitlines() if l and l.split(" ", 1)[0] != me]
        if others:
            print(f"[preflight] WARNING: {len(others)} existing solver/bench process(es) detected — "
                  "concurrent runs on the same cores contaminate PAR-2 timing. Stop them or confirm "
                  "before trusting results:", flush=True)
            for l in others[:8]:
                print(f"[preflight]   {l}", flush=True)
    except Exception:
        pass
    print(f"[preflight] cores={CORES} jobs={args.jobs} seeds={args.seeds} "
          f"mem={args.mem_mb}MB/job timeout={args.timeout}s", flush=True)

# tag -> (solver_dir, {env}).  Empty env = that solver's default profile.
CONFIGS: list[tuple[str, str, dict]] = [
    # --- references ---
    ("baseline",            S11, {}),
    ("solver10",            S10, {}),
    # --- independent singles (no requires) ---
    ("lucky",               S11, {"SAT_LUCKY": "on"}),
    ("chrono",              S11, {"SAT_CHRONO": "on"}),
    ("binary_fast",         S11, {"SAT_BINARY_FAST": "on"}),
    ("restart_reuse_trail", S11, {"SAT_RESTART_REUSE_TRAIL": "on"}),
    ("reorder",             S11, {"SAT_REORDER": "on"}),
    ("otfs",                S11, {"SAT_OTFS": "on"}),                       # clause_min on by default
    ("otss",                S11, {"SAT_OTSS": "on"}),
    # --- LBD family (parent SAT_USE_LBD=on) ---
    ("use_lbd",             S11, {"SAT_USE_LBD": "on"}),                    # parent control
    ("lbd_update_reasons",  S11, {"SAT_USE_LBD": "on", "SAT_LBD_UPDATE_REASONS": "on"}),
    ("lbd_update_pair",     S11, {"SAT_USE_LBD": "on", "SAT_LBD_UPDATE_REASONS": "on",
                                  "SAT_LBD_UPDATE_PROP_REASONS": "on"}),
    ("lbd_tiered",          S11, {"SAT_USE_LBD": "on", "SAT_REDUCE": "lbd-tiered"}),   # parent control
    ("reduce_tier2",        S11, {"SAT_USE_LBD": "on", "SAT_REDUCE": "lbd-tiered",
                                  "SAT_REDUCE_TIER2_AT_BUDGET": "on"}),
    ("watch_compact",       S11, {"SAT_USE_LBD": "on", "SAT_REDUCE": "lbd-tiered",
                                  "SAT_WATCH_COMPACT": "on"}),
    # --- focused-stable family (parent stack) ---
    ("fstab",               S11, {"SAT_USE_LBD": "on", "SAT_SEARCH_MODE": "focused-stable",
                                  "SAT_MODE_USE_TICKS": "on"}),
    ("fstab_novmtf",        S11, {"SAT_USE_LBD": "on", "SAT_SEARCH_MODE": "focused-stable",
                                  "SAT_MODE_USE_TICKS": "on", "SAT_VMTF": "off"}),
    ("fstab_lbdtier",       S11, {"SAT_USE_LBD": "on", "SAT_SEARCH_MODE": "focused-stable",
                                  "SAT_MODE_USE_TICKS": "on", "SAT_REDUCE": "lbd-tiered"}),
    ("fstab_rephase",       S11, {"SAT_USE_LBD": "on", "SAT_SEARCH_MODE": "focused-stable",
                                  "SAT_MODE_USE_TICKS": "on", "SAT_REPHASE": "on"}),
    ("fstab_full",          S11, {"SAT_USE_LBD": "on", "SAT_SEARCH_MODE": "focused-stable",
                                  "SAT_MODE_USE_TICKS": "on", "SAT_REDUCE": "lbd-tiered",
                                  "SAT_REPHASE": "on"}),
    # --- clause minimization + shrink (inblock = kissat-style in-block minimize+shrink;
    #     SH-A/SH-B fix landed 5a01032; bug prz still open so treat as diagnostic) ---
    ("inblock",             S11, {"SAT_CLAUSE_MIN": "inblock"}),
    ("inblock_otfs_otss",   S11, {"SAT_CLAUSE_MIN": "inblock", "SAT_OTFS": "on", "SAT_OTSS": "on"}),
    # --- cross combos ---
    ("lucky_chrono",        S11, {"SAT_LUCKY": "on", "SAT_CHRONO": "on"}),
    ("otfs_otss",           S11, {"SAT_OTFS": "on", "SAT_OTSS": "on"}),
    ("restart_reuse_chrono",S11, {"SAT_RESTART_REUSE_TRAIL": "on", "SAT_CHRONO": "on"}),
]
# --- Sweep 2 (2026-05-31): combine the NEW fstab_lbdtier default with other features. ---
# Empty env == the new default (focused-stable + LBD + ticks + lbd-tiered + vmtf-focused + lucky),
# since that was promoted to the default profile. Each config layers overrides ON TOP of it.
# Two families per the sweep design: (A) focused-stable features newly UNLOCKED by the default
# (the kissat restart/phase stack was invalid/no-op on the old single-mode base), and (B) prior
# singles RE-TESTED on the new base where interactions may flip. Plus hand-picked combos.
# NOTE: validity is non-obvious (requires/conflicts_with); run `--validate --sweep2` before launching.
CONFIGS_V2: list[tuple[str, str, dict]] = [
    # --- references ---
    ("default",             S11, {}),                                  # the new fstab_lbdtier default
    ("solver10",            S10, {}),
    # --- (A) newly-unlocked focused-stable / kissat-native features ---
    ("ema",                 S11, {"SAT_RESTART": "kissat-ema"}),
    ("reluctant",           S11, {"SAT_RESTART": "reluctant"}),
    ("target_phase",        S11, {"SAT_PHASE": "target-then-saved"}),
    ("best_phase",          S11, {"SAT_PHASE": "best-then-target-then-saved"}),
    ("rephase",             S11, {"SAT_REPHASE": "on"}),
    ("reuse_focused",       S11, {"SAT_RESTART_REUSE_TRAIL_FOCUSED": "on"}),
    ("reuse_stable",        S11, {"SAT_RESTART_REUSE_TRAIL_STABLE": "on"}),
    # --- (B) prior singles re-tested on the new base ---
    ("chrono",              S11, {"SAT_CHRONO": "on"}),
    ("binary_fast",         S11, {"SAT_BINARY_FAST": "on"}),
    ("otfs",                S11, {"SAT_OTFS": "on"}),
    ("otss",                S11, {"SAT_OTSS": "on"}),
    ("reorder",             S11, {"SAT_REORDER": "on"}),
    ("reuse_trail",         S11, {"SAT_RESTART_REUSE_TRAIL": "on"}),
    ("lbd_update_reasons",  S11, {"SAT_LBD_UPDATE_REASONS": "on"}),
    ("lbd_update_pair",     S11, {"SAT_LBD_UPDATE_REASONS": "on", "SAT_LBD_UPDATE_PROP_REASONS": "on"}),
    ("reduce_tier2",        S11, {"SAT_REDUCE_TIER2_AT_BUDGET": "on"}),
    ("watch_compact",       S11, {"SAT_WATCH_COMPACT": "on"}),
    # --- hand-picked combos (mechanism-motivated) ---
    ("ema_target",          S11, {"SAT_RESTART": "kissat-ema", "SAT_PHASE": "target-then-saved"}),
    ("ema_target_rephase",  S11, {"SAT_RESTART": "kissat-ema", "SAT_PHASE": "target-then-saved",
                                  "SAT_REPHASE": "on"}),
    ("ema_reuse",           S11, {"SAT_RESTART": "kissat-ema",
                                  "SAT_RESTART_REUSE_TRAIL_FOCUSED": "on",
                                  "SAT_RESTART_REUSE_TRAIL_STABLE": "on"}),
    ("target_rephase",      S11, {"SAT_PHASE": "target-then-saved", "SAT_REPHASE": "on"}),
    ("chrono_ema",          S11, {"SAT_CHRONO": "on", "SAT_RESTART": "kissat-ema"}),
    ("binfast_tier2",       S11, {"SAT_BINARY_FAST": "on", "SAT_REDUCE_TIER2_AT_BUDGET": "on"}),
    ("otfs_otss",           S11, {"SAT_OTFS": "on", "SAT_OTSS": "on"}),
]
# Merged tag->(solver,env) lookup used by run_one_config (tags unique across matrices; the only
# overlap is `solver10`, identical in both, and `default`/`baseline` which are empty-env refs).
CONFIG_MAP = {t: (s, e) for (t, s, e) in CONFIGS + CONFIGS_V2}


def load_halves() -> dict[str, str]:
    halves = {}
    with open(SELECTION) as f:
        for row in csv.DictReader(f):
            halves[row["instance"].strip()] = row["half"]
    return halves


def instances(half: str | None = None) -> list[str]:
    """profile20 instance stems (== bench results.csv instance keys)."""
    halves = load_halves()
    out = [name for name, h in halves.items() if half is None or h == half]
    return sorted(out)


def file_for(stem: str) -> str:
    return stem + ".cnf.xz"


def shard(items: list[str], k: int) -> list[list[str]]:
    groups: list[list[str]] = [[] for _ in range(k)]
    for i, it in enumerate(items):        # round-robin so each shard mixes easy+hard
        groups[i % k].append(it)
    return [g for g in groups if g]


def run_one_config(tag: str, out_dir: Path, insts: list[str], timeout: int,
                   mem_mb: int, jobs: int) -> Path:
    """Run one config over `insts` using `jobs` taskset-pinned bench -j1 shards; merge results.csv."""
    solver_dir, env_extra = CONFIG_MAP[tag]
    out_dir.mkdir(parents=True, exist_ok=True)
    merged = out_dir / "results.csv"
    if merged.exists():
        return merged   # resume: already done

    groups = shard(insts, min(jobs, len(insts)))
    procs = []
    shard_csvs = []
    for idx, group in enumerate(groups):
        core = CORES[idx % len(CORES)]
        sd = out_dir / f"shard{idx}"
        bdir = sd / "bench"
        bdir.mkdir(parents=True, exist_ok=True)
        # symlink the shard's instances into a private benchdir
        for stem in group:
            link = bdir / file_for(stem)
            tgt = (SUITE / file_for(stem)).resolve()
            if not link.exists():
                link.symlink_to(tgt)
        ldir = sd / "log"
        env = {**os.environ, **env_extra}
        cmd = ["taskset", "-c", str(core), "bash", "tools/bench.sh",
               "-t", str(timeout), "-m", str(mem_mb), "-j", "1",
               "-d", str(bdir), "--log-dir", str(ldir), solver_dir]
        log_fh = open(sd / "console.log", "w")
        procs.append((idx, subprocess.Popen(cmd, cwd=ROOT, env=env,
                                             stdout=log_fh, stderr=subprocess.STDOUT), log_fh))
        shard_csvs.append(ldir / "results.csv")

    for idx, p, fh in procs:
        p.wait()
        fh.close()

    # merge shard results.csv -> one
    header = None
    rows = []
    for csvp in shard_csvs:
        if not csvp.exists():
            continue
        with open(csvp) as f:
            r = csv.reader(f)
            h = next(r, None)
            if h and header is None:
                header = h
            for row in r:
                rows.append(row)
    if header is None:
        header = ["instance", "result", "verified", "time_s", "timeout", "exit_code"]
    with open(merged, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(header)
        w.writerows(sorted(rows))
    return merged


def par2_split(results_csv: Path, timeout: int) -> dict[str, float]:
    rows = compare_bench.read_rows(results_csv)
    halves = load_halves()
    easy = {k: v for k, v in rows.items() if halves.get(k) == "easy"}
    hard = {k: v for k, v in rows.items() if halves.get(k) == "hard"}
    return {
        "all": compare_bench.par2(rows, timeout),
        "easy": compare_bench.par2(easy, timeout),
        "hard": compare_bench.par2(hard, timeout),
        "solved": compare_bench.solved_count(rows),
        "n": len(rows),
    }


def build(solver_dir: str) -> None:
    print(f"[build] {solver_dir}", flush=True)
    subprocess.run(["bash", "build.sh"], cwd=ROOT / solver_dir, check=True)


def append_summary(summary: Path, tag: str, run_idx: int, m: dict, env: dict) -> None:
    new = not summary.exists()
    with open(summary, "a", newline="") as f:
        w = csv.writer(f, delimiter="\t")
        if new:
            w.writerow(["tag", "run", "par2_all", "par2_easy", "par2_hard", "solved", "n", "env"])
        w.writerow([tag, run_idx, f"{m['all']:.3f}", f"{m['easy']:.3f}", f"{m['hard']:.3f}",
                    m["solved"], m["n"], " ".join(f"{k}={v}" for k, v in sorted(env.items()))])


def active_matrix(args):
    """Return (matrix_list, reference_tag) for the requested sweep."""
    if getattr(args, "sweep2", False):
        return CONFIGS_V2, "default"
    return CONFIGS, "baseline"


def stage1(args) -> int:
    matrix, ref_tag = active_matrix(args)
    matrix_map = {t: (s, e) for (t, s, e) in matrix}
    ts = time.strftime("%Y-%m-%d-%H-%M-%S")
    camp = ROOT / "log" / f"feature-ablation-{ts}"
    camp.mkdir(parents=True, exist_ok=True)
    summary = camp / "summary.tsv"
    insts = instances()
    # optional subset: --configs tag1,tag2 runs only those (the reference tag is always kept)
    selected = {t.strip() for t in args.configs.split(",") if t.strip()} if args.configs else None
    run_list = [c for c in matrix if selected is None or c[0] in selected or c[0] == ref_tag]
    print(f"[stage1] {len(run_list)} configs x {len(insts)} instances, t={args.timeout}s "
          f"m={args.mem_mb}MB jobs={args.jobs} ref={ref_tag} -> {camp}", flush=True)

    build(S11)
    build(S10)

    # reference config twice for a stable 3% band
    means: dict[str, dict] = {}
    runs: dict[str, list[dict]] = {}
    for tag, _, env in run_list:
        reps = 2 if tag == ref_tag else 1
        for r in range(1, reps + 1):
            odir = camp / tag / f"r{r}"
            t0 = time.time()
            res = run_one_config(tag, odir, insts, args.timeout, args.mem_mb, args.jobs)
            m = par2_split(res, args.timeout)
            runs.setdefault(tag, []).append(m)
            append_summary(summary, tag, r, m, env)
            print(f"[{tag} r{r}] PAR2 all={m['all']:.1f} easy={m['easy']:.1f} "
                  f"hard={m['hard']:.1f} solved={m['solved']} ({time.time()-t0:.0f}s)", flush=True)
        means[tag] = {k: sum(d[k] for d in runs[tag]) / len(runs[tag])
                      for k in ("all", "easy", "hard")}

    base = means[ref_tag]["all"]
    band = base * TOL

    # 3% repeat rule: configs within +/-band of the reference get one confirming rerun (n=2)
    for tag, _, env in run_list:
        if tag in (ref_tag, "solver10"):
            continue
        while len(runs[tag]) < 2 and abs(means[tag]["all"] - base) <= band:
            r = len(runs[tag]) + 1
            odir = camp / tag / f"r{r}"
            res = run_one_config(tag, odir, insts, args.timeout, args.mem_mb, args.jobs)
            m = par2_split(res, args.timeout)
            runs[tag].append(m)
            append_summary(summary, tag, r, m, env)
            means[tag] = {k: sum(d[k] for d in runs[tag]) / len(runs[tag])
                          for k in ("all", "easy", "hard")}
            print(f"[{tag} repeat r{r}] within 3% -> mean all={means[tag]['all']:.1f}", flush=True)

    # report
    report = camp / "STAGE1.md"
    rows = sorted(((t, means[t]) for t, _, _ in run_list), key=lambda x: x[1]["all"])
    floor = f"solver-10 floor all-20 = {means['solver10']['all']:.1f}." if "solver10" in means \
            else "(solver-10 floor not in this subset run)"
    lines = [f"# Stage-1 feature ablation on profile20 ({ts})", "",
             f"Reference config `{ref_tag}` all-20 PAR-2 = **{base:.1f}**; 3% band = +/-{band:.1f}. "
             f"{floor}", "",
             "| config | all-20 Δ | all-20 | easy-10 | hard-10 | n |", "|---|---:|---:|---:|---:|---:|"]
    for t, m in rows:
        d = m["all"] - base
        verdict = "WIN" if d < -band else ("regress" if d > band else "neutral")
        if t == ref_tag: verdict = "—"
        lines.append(f"| {t} | {d:+.1f} {verdict} | {m['all']:.1f} | {m['easy']:.1f} | "
                     f"{m['hard']:.1f} | {len(runs[t])} |")
    # Stage-2 shortlist: net all-20 win, or any hard-10 improvement vs the reference beyond band
    ref_hard = means[ref_tag]["hard"]
    shortlist = [t for t, _, _ in run_list if t not in (ref_tag, "solver10")
                 and (means[t]["all"] < base - band
                      or means[t]["hard"] < ref_hard - ref_hard * TOL)]
    lines += ["", "## Proposed Stage-2 shortlist (long-timeout, hard-10)",
              ", ".join(shortlist) if shortlist else "(none beat the reference beyond 3% — review manually)"]
    report.write_text("\n".join(lines) + "\n")
    (camp / "DONE").write_text("stage1 complete\n")
    print("\n".join(lines), flush=True)
    print(f"\n[stage1] DONE -> {report}", flush=True)
    return 0


def stage2(args) -> int:
    tags = [t.strip() for t in args.configs.split(",") if t.strip()]
    ts = time.strftime("%Y-%m-%d-%H-%M-%S")
    camp = ROOT / "log" / f"feature-ablation-stage2-{ts}"
    camp.mkdir(parents=True, exist_ok=True)
    summary = camp / "summary.tsv"
    insts = instances("hard")
    print(f"[stage2] {tags} on {len(insts)} hard instances t={args.timeout}s", flush=True)
    build(S11); build(S10)
    for tag in ["baseline", "solver10"] + [t for t in tags if t not in ("baseline", "solver10")]:
        _, env = CONFIG_MAP[tag]
        odir = camp / tag / "r1"
        res = run_one_config(tag, odir, insts, args.timeout, args.mem_mb, args.jobs)
        m = par2_split(res, args.timeout)
        append_summary(summary, tag, 1, m, env)
        print(f"[{tag}] hard PAR2={m['hard']:.1f} solved={m['solved']}/{m['n']}", flush=True)
    (camp / "DONE").write_text("stage2 complete\n")
    print(f"[stage2] DONE -> {camp}", flush=True)
    return 0


def smoke(args) -> int:
    """Fast self-check: baseline + chrono on 2 instances, short timeout."""
    camp = ROOT / "log" / "feature-ablation-smoke"
    if camp.exists():
        shutil.rmtree(camp)
    camp.mkdir(parents=True)
    insts = instances("easy")[:2]
    print(f"[smoke] instances={insts}", flush=True)
    build(S11)
    for tag in ("baseline", "chrono"):
        _, env = CONFIG_MAP[tag]
        res = run_one_config(tag, camp / tag, insts, 60, args.mem_mb, 2)
        m = par2_split(res, 60)
        print(f"[smoke {tag}] env={env} -> {compare_bench.read_rows(res)}", flush=True)
        print(f"[smoke {tag}] PAR2 all={m['all']:.1f} solved={m['solved']}/{m['n']}", flush=True)
    print("[smoke] ok", flush=True)
    return 0


def parse_env_arg(spec: str) -> dict:
    """Parse --env 'SAT_X=on,SAT_Y=2' -> {'SAT_X':'on','SAT_Y':'2'}.  '' -> {} (the solver default)."""
    env = {}
    for kv in spec.split(","):
        kv = kv.strip()
        if not kv:
            continue
        if "=" not in kv:
            raise SystemExit(f"--env entry {kv!r} must be KEY=VALUE")
        k, v = kv.split("=", 1)
        env[k.strip()] = v.strip()
    return env


def env_slug(env: dict) -> str:
    """Short filesystem-safe label for an ad-hoc env dict (names the seedgate-<tag> dir + gate TSV)."""
    if not env:
        return "adhoc-default"
    parts = [f"{k.replace('SAT_', '').lower()}{str(v).lower().replace('.', '').replace('-', '')}"
             for k, v in sorted(env.items())]
    return ("adhoc-" + "-".join(parts))[:60]


def seedgate(args) -> int:
    """Multi-seed per-(config,instance,seed) sweep -> gate-compatible TSV (2026-06-02 procedure).

    The standard pre-keep/pre-promote measurement: run a config across N seeds (default 10) on each
    instance, capturing conflicts (deterministic per (config,seed), contention-immune) for the
    lexicographic solved->conflicts->PAR-2 decision. Decompresses .cnf.xz to a scratch dir (the
    solver does not read .xz directly), runs N workers pinned to physical cores, writes one TSV per
    config that check_solver11_promotion.py --multiseed consumes. Resumable.

    Usage: --seedgate --tag <config-tag> [--seeds 10] [--timeout 600] on the suite's instances.
    """
    import shutil
    if getattr(args, "env", None) is not None:
        # ad-hoc mode: measure ANY config/feature flag without registering it in CONFIG_MAP
        env_extra = parse_env_arg(args.env)
        solver_dir = (getattr(args, "solver", None) or S11)
        tag = (getattr(args, "tag", "") or "").strip() or args.configs.strip() or env_slug(env_extra)
    else:
        tag = args.configs.strip() or "default"
        if tag not in CONFIG_MAP:
            print(f"unknown config tag {tag!r}; known: {sorted(CONFIG_MAP)}", flush=True)
            return 2
        solver_dir, env_extra = CONFIG_MAP[tag]
    seeds = list(range(args.seeds))
    insts = instances(args.half if getattr(args, "half", None) else None)
    ts = time.strftime("%Y-%m-%d-%H-%M-%S")
    camp = ROOT / "log" / f"seedgate-{tag}-{ts}"
    (camp / "_work").mkdir(parents=True, exist_ok=True)
    scratch = camp / "_cnf"
    scratch.mkdir(parents=True, exist_ok=True)
    build(solver_dir)

    # decompress instances once (solver can't read .cnf.xz)
    cnf_for = {}
    for stem in insts:
        dst = scratch / (stem + ".cnf")
        if not dst.exists():
            with open(SUITE / (stem + ".cnf.xz"), "rb") as fh:
                import lzma
                dst.write_bytes(lzma.decompress(fh.read()))
        cnf_for[stem] = dst

    jobs = [(i, stem, seed) for i, (stem, seed) in enumerate(
        (s, sd) for s in insts for sd in seeds)]
    print(f"[seedgate] tag={tag} {len(jobs)} runs ({len(insts)} inst x {len(seeds)} seeds) "
          f"t={args.timeout}s m={args.mem_mb}MB jobs={len(CORES)} -> {camp}", flush=True)

    # Free-core pool: one physical core per running job (get() on start, put() on finish), identical
    # to abtest(). This GUARANTEES each concurrent run owns a distinct physical core even under uneven
    # instance runtimes. A static `CORES[idx % len(CORES)]` pin (the pre-2026-07-03 seedgate default)
    # double-books a core the moment a later job (idx >= len(CORES)) starts while its index-congruent
    # earlier job is still running — inflating that cell's PAR-2 with hyperthread/scheduler contention
    # while another core sits idle. The pool checks a core back in only when a run actually finishes,
    # so there is never more than one running job per core. Solve/scrape/proof-cleanup are shared with
    # abtest via _solve_one so the two modes cannot drift.
    import queue
    from concurrent.futures import ThreadPoolExecutor
    core_pool = queue.Queue()
    for c in CORES:
        core_pool.put(c)

    def run(job):
        idx, stem, seed = job
        core = core_pool.get()
        try:
            res, dt, cf, pr, dc = _solve_one(
                core, solver_dir, env_extra, cnf_for[stem], camp / "_work" / f"{idx}",
                args.timeout, args.mem_mb, seed)
        finally:
            core_pool.put(core)
        return (stem, seed, res, dt, cf, pr, dc)

    results = []
    with ThreadPoolExecutor(max_workers=len(CORES)) as ex:
        for r in ex.map(run, jobs):
            results.append(r)
            print(f"  {r[0][:26]}/s{r[1]} {r[2]} {r[3]:.0f}s conf={r[4]}", flush=True)

    tsv = camp / "results.tsv"
    with open(tsv, "w") as f:
        f.write("config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\ttimeout\n")
        for stem, seed, res, dt, cf, pr, dc in sorted(results):
            f.write(f"{tag}\t{stem}\t{seed}\t{res}\t{dt:.3f}\t{cf}\t{pr}\t{dc}\t{args.timeout}\n")
    (camp / "DONE").write_text("seedgate complete\n")
    solved = sum(1 for r in results if r[2].upper() in ("SAT", "UNSAT", "SATISFIABLE", "UNSATISFIABLE"))
    print(f"[seedgate] DONE solved={solved}/{len(results)} -> {tsv}", flush=True)
    return 0


def parse_arm(spec: str, default_solver: str) -> tuple[str, str, dict]:
    """'tag:ENVSPEC' -> (tag, solver_dir, env); no ':' -> a registered CONFIG_MAP tag.

    'cand:SAT_X=on,SAT_Y=2' -> ('cand', default_solver, {SAT_X:on, SAT_Y:2})
    'base:'                  -> ('base', default_solver, {})            # ':' + empty env = solver default
    'solver10'               -> CONFIG_MAP lookup (its own solver_dir + env; lets you A/B vs the floor)
    """
    if ":" in spec:
        tag, envspec = spec.split(":", 1)
        tag = tag.strip()
        if not tag:
            raise SystemExit(f"--arm {spec!r}: empty tag before ':'")
        return tag, default_solver, parse_env_arg(envspec)
    tag = spec.strip()
    if tag not in CONFIG_MAP:
        raise SystemExit(f"--arm {spec!r}: not a CONFIG_MAP tag and has no ':ENV'. Use 'tag:SAT_X=on' "
                         f"(':' with empty env = baseline), or a registered tag from CONFIG_MAP.")
    solver_dir, env = CONFIG_MAP[tag]
    return tag, solver_dir, env


def pick_baseline(tags: list[str], explicit: str) -> str:
    """Choose the A/B reference arm: --baseline if given, else a conventionally-named arm, else last."""
    if explicit:
        if explicit not in tags:
            raise SystemExit(f"--baseline {explicit!r} is not one of the arms: {tags}")
        return explicit
    for pref in ("baseline", "base", "default", "prev", "previous"):
        for t in tags:
            if t.lower() == pref:
                return t
    return tags[-1]


def _solve_one(core: int, solver_dir: str, env_extra: dict, cnf_path: Path, odir: Path,
               timeout: int, mem_mb: int, seed: int) -> tuple:
    """Run the solver on one (instance,seed) pinned to `core`; return (result, secs, conf, props, decs).

    Mirrors seedgate()'s inner run: ulimit -v address-space cap, SAT_STATS_JSON conflicts scraped from
    stderr, and the per-run DRAT-proof scratch deleted in `finally` (multi-GB on hard-UNSAT, never read
    back — left to accumulate it fills the disk and kills the sweep).
    """
    import re
    odir.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, **env_extra, "SAT_SEED": str(seed), "SAT_STATS_JSON": "on"}
    cmd = ["taskset", "-c", str(core), "bash", "-c",
           f'ulimit -v {mem_mb*1024}; exec timeout {timeout} '
           f'"{ROOT/solver_dir}/target/release/sat-solver" "{cnf_path}" "{odir}"']
    t0 = time.time()
    try:
        try:
            p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, env=env, timeout=timeout + 30)
            dt = time.time() - t0
            res = next((ln.split()[1] for ln in p.stdout.splitlines() if ln.startswith("s ")), None)
            if res is None:
                res = "TIMEOUT" if p.returncode == 124 else f"UNKNOWN_rc{p.returncode}"
            js = ""
            for ln in p.stderr.splitlines():
                if '"conflicts"' in ln:
                    br = ln.find("{")
                    if br >= 0:
                        js = ln[br:]
            g = lambda k: (re.search(rf'"{k}":([0-9.eE+-]+)', js) or [None, "NA"])[1]
            return (res, dt, g("conflicts"), g("propagations"), g("decisions"))
        except subprocess.TimeoutExpired:
            return ("TIMEOUT", time.time() - t0, "NA", "NA", "NA")
    finally:
        shutil.rmtree(odir, ignore_errors=True)


def abtest(args) -> int:
    """Simultaneous-start, interleaved, fair shared-core A/B (or N-way) seedgate (2026-07-03).

    Every (arm, instance, seed) run goes into ONE queue pulled by `jobs` workers, each pinned to its own
    physical core drawn from a free-core pool. Jobs are submitted interleaved as (instance, seed, arm) so
    consecutive jobs cycle the arms — an instance's arm-A and arm-B runs start within one scheduling slot
    of each other, both arms run in the SAME wall-clock window, and each arm lands on whatever core is
    free (uniform spread across all cores) so neither arm inherits a socket/thermal/turbo bias. Writes one
    gate-compatible results.tsv per arm (feed check_solver11_promotion.py --multiseed) + an inline verdict.

    Mode defaults (resolved in main): --jobs 32 (physical cores 0..31 on this 36-core box; siblings idle),
    --mem-mb 16000, --timeout 1800. Conflicts are deterministic per (config,seed) and contention-immune,
    so the solved->conflicts tie-breaks stay clean even under the shared-core load; only PAR-2 is timing-
    sensitive, and the interleave keeps that fair too.
    """
    import lzma
    import queue
    from collections import defaultdict
    from concurrent.futures import ThreadPoolExecutor

    default_solver = (getattr(args, "solver", None) or S11)
    arms = [parse_arm(a, default_solver) for a in args.arm]
    tags = [t for t, _, _ in arms]
    if len(set(tags)) != len(tags):
        raise SystemExit(f"--arm tags must be unique (A/A noise checks are fine — just distinct tags): {tags}")
    baseline = pick_baseline(tags, args.baseline.strip())

    seeds = list(range(args.seeds))
    insts = instances(args.half if getattr(args, "half", None) else None)
    ts = time.strftime("%Y-%m-%d-%H-%M-%S")
    camp = ROOT / "log" / f"abtest-{('-vs-'.join(tags))[:48]}-{ts}"
    (camp / "_work").mkdir(parents=True, exist_ok=True)
    scratch = camp / "_cnf"
    scratch.mkdir(parents=True, exist_ok=True)

    for sd in sorted({s for _, s, _ in arms}):
        build(sd)

    # decompress instances once (shared across arms; the solver can't read .cnf.xz directly)
    cnf_for = {}
    for stem in insts:
        dst = scratch / (stem + ".cnf")
        if not dst.exists():
            with open(SUITE / (stem + ".cnf.xz"), "rb") as fh:
                dst.write_bytes(lzma.decompress(fh.read()))
        cnf_for[stem] = dst

    # interleaved job list: (instance, seed, arm) so consecutive jobs cycle arms -> arms start together
    jobs = []
    idx = 0
    for stem in insts:
        for seed in seeds:
            for (tag, sdir, env) in arms:
                jobs.append((idx, tag, sdir, env, stem, seed))
                idx += 1

    print(f"[abtest] arms={tags} baseline={baseline} {len(insts)} inst x {len(seeds)} seeds "
          f"= {len(jobs)} runs; jobs={len(CORES)} cores={CORES[0]}..{CORES[-1]} "
          f"t={args.timeout}s m={args.mem_mb}MB -> {camp}", flush=True)

    # free-core pool: one physical core per running job. max_workers == len(CORES), so get() never
    # checks out more than len(CORES) cores at once and never blocks longer than a scheduling gap.
    core_pool = queue.Queue()
    for c in CORES:
        core_pool.put(c)

    def run(job):
        jidx, tag, sdir, env_extra, stem, seed = job
        core = core_pool.get()
        try:
            res, dt, cf, pr, dc = _solve_one(
                core, sdir, env_extra, cnf_for[stem], camp / "_work" / f"{jidx}",
                args.timeout, args.mem_mb, seed)
        finally:
            core_pool.put(core)
        return (tag, stem, seed, res, dt, cf, pr, dc)

    results = []
    with ThreadPoolExecutor(max_workers=len(CORES)) as ex:
        for r in ex.map(run, jobs):
            results.append(r)
            print(f"  [{r[0]}] {r[1][:24]}/s{r[2]} {r[3]} {r[4]:.0f}s conf={r[5]}", flush=True)

    # one gate-compatible results.tsv per arm (identical schema to seedgate's, so --multiseed consumes it)
    by_arm = defaultdict(list)
    for r in results:
        by_arm[r[0]].append(r)
    arm_tsv = {}
    for tag in tags:
        adir = camp / tag
        adir.mkdir(parents=True, exist_ok=True)
        tsv = adir / "results.tsv"
        with open(tsv, "w") as f:
            f.write("config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\ttimeout\n")
            for tag_, stem, seed, res, dt, cf, pr, dc in sorted(by_arm[tag]):
                f.write(f"{tag}\t{stem}\t{seed}\t{res}\t{dt:.3f}\t{cf}\t{pr}\t{dc}\t{args.timeout}\n")
        arm_tsv[tag] = tsv

    (camp / "DONE").write_text("abtest complete\n")
    _ab_verdict(arm_tsv, tags, baseline, args.timeout, args.mem_mb)
    print(f"\n[abtest] DONE -> {camp}", flush=True)
    return 0


def _ab_verdict(arm_tsv: dict, tags: list[str], baseline: str, timeout: int, mem_mb: int) -> None:
    """Inline solved -> conflicts(both-solved) -> PAR-2 verdict.

    Uses the SAME compare_bench scoring the promotion gate uses (lexicographic_score /
    both_solved_conflict_totals / lexicographic_decision), so this verdict and
    check_solver11_promotion.py --multiseed can never disagree. Also prints the exact gate command.
    """
    cells, score = {}, {}
    for tag in tags:
        by_cfg = compare_bench.seed_cells_by_config(compare_bench.read_seed_tsv(arm_tsv[tag]))
        cells[tag] = by_cfg.get(tag) or next(iter(by_cfg.values()), {})
        score[tag] = compare_bench.lexicographic_score(cells[tag], timeout)

    print("\n=== A/B verdict — metric: solved -> conflicts(both-solved) -> PAR-2 "
          f"(baseline: {baseline}) ===", flush=True)
    print(f"{'arm':<22} {'solved':>8} {'conf(own solved)':>18} {'PAR-2':>12}   vs baseline", flush=True)
    for tag in tags:
        s = score[tag]
        solved = f"{s['solved']}/{s['cells']}"
        conf = f"{s['conflicts_solved']:,}"
        line = f"{tag:<22} {solved:>8} {conf:>18} {s['par2']:>12.1f}"
        if tag == baseline:
            print(line + "   —  (reference)", flush=True)
            continue
        both = compare_bench.both_solved_conflict_totals(cells[tag], cells[baseline])
        decision, reason = compare_bench.lexicographic_decision(score[tag], score[baseline], both)
        verdict = {"win": "WIN", "regress": "LOSE", "tie": "tie"}.get(decision, decision)
        # Same decision the gate makes, but flag win/regress that rest ONLY on the PAR-2 tie-break
        # (solved & conflicts both tie): sub-noise timing, not a real signal — don't over-read it.
        note = "  [PAR-2 tie-break only — solved & conflicts tie; timing noise]" \
            if reason.startswith("equal solved+conflicts; PAR-2") else ""
        print(line + f"   {verdict}  ({reason}){note}", flush=True)
        contra = compare_bench.seed_contradictions(cells[tag], cells[baseline])
        if contra:
            print(f"    *** CORRECTNESS ALERT: {tag} vs {baseline} disagree SAT<->UNSAT on "
                  f"{len(contra)} cell(s): {contra[:3]} — a real bug, investigate before trusting metrics.",
                  flush=True)
    print("* conf(own solved) sums that arm's own solved cells; the WIN/LOSE decision compares conflicts "
          "only over cells BOTH arms solve (as the gate does).", flush=True)

    floor = arm_tsv.get("solver10", "<solver10.tsv>")
    for tag in tags:
        if tag == baseline:
            continue
        print(f"\ngate[{tag} vs {baseline}]: python3 tools/check_solver11_promotion.py --multiseed \\\n"
              f"  --candidate {arm_tsv[tag]} \\\n  --previous {arm_tsv[baseline]} \\\n"
              f"  --floor {floor} --timeout {timeout} --memory-mb {mem_mb}", flush=True)


def validate(args) -> int:
    """Pre-flight: run each config on a trivial CNF and report invalid-config rejections.

    The promoted default carries a web of requires/conflicts_with constraints, so some layered
    combos may be rejected by validate_runtime_support() (exit 2). Catch those here before a
    multi-hour campaign wastes a config's slot on all-ERROR rows.
    """
    matrix, ref_tag = active_matrix(args)
    build(S11)
    triv = ROOT / "log" / "_fa_validate.cnf"
    triv.write_text("p cnf 2 2\n1 -2 0\n2 0\n")
    bad = []
    for tag, solver_dir, env_extra in matrix:
        if solver_dir != S11:
            continue
        out = ROOT / "log" / "_fa_validate_out"
        proc = subprocess.run(
            [str(ROOT / solver_dir / "target/release/sat-solver"), str(triv), str(out)],
            cwd=ROOT, env={**os.environ, **env_extra},
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        ok = proc.returncode in (0, 10, 20)   # 0/SAT/UNSAT exit conventions
        flag = "OK " if ok else "BAD"
        msg = ""
        if not ok:
            inv = [l for l in proc.stderr.splitlines() if "Invalid config" in l or "must" in l]
            msg = (inv[0] if inv else f"exit={proc.returncode}")
            bad.append((tag, msg))
        print(f"  [{flag}] {tag:22s} {' '.join(f'{k}={v}' for k,v in sorted(env_extra.items()))}"
              + (f"   -> {msg}" if msg else ""), flush=True)
    print(f"\n{len(matrix)} configs checked; {len(bad)} invalid.", flush=True)
    if bad:
        print("INVALID:", ", ".join(t for t, _ in bad), flush=True)
        return 1
    print("all configs valid", flush=True)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--stage1", action="store_true")
    ap.add_argument("--stage2", action="store_true")
    ap.add_argument("--smoke", action="store_true")
    ap.add_argument("--validate", action="store_true")
    ap.add_argument("--sweep2", action="store_true", help="use the CONFIGS_V2 matrix (new-default combos)")
    ap.add_argument("--seedgate", action="store_true",
                    help="multi-seed per-(config,instance,seed) sweep -> gate TSV "
                         "(use with --configs <tag> from CONFIG_MAP, or --env 'K=V,...' ad-hoc)")
    ap.add_argument("--seeds", type=int, default=10, help="seeds for --seedgate (default 10)")
    ap.add_argument("--half", default="", help="restrict --seedgate to 'easy' or 'hard' half")
    ap.add_argument("--env", default=None,
                    help="ad-hoc --seedgate config without editing CONFIG_MAP: 'SAT_X=on,SAT_Y=2' "
                         "(''=solver default). A/B it against --env '' (baseline). Pair with --tag.")
    ap.add_argument("--solver", default=None,
                    help="solver dir for --env --seedgate (default: current solver)")
    ap.add_argument("--tag", default="", help="label for an --env --seedgate run (output dir + gate TSV)")
    ap.add_argument("--configs", default="")
    ap.add_argument("--arm", action="append", default=None, metavar="TAG:ENV",
                    help="A/B arm 'tag:SAT_X=on,SAT_Y=2' (':' with empty env = solver default), or a "
                         "bare CONFIG_MAP tag. Repeat for N-way. Triggers a simultaneous-start, "
                         "interleaved, fair shared-core seedgate across all arms (defaults: 32 cores, "
                         "16GB/job, 1800s; one gate-compatible results.tsv per arm + inline verdict).")
    ap.add_argument("--baseline", default="",
                    help="arm tag used as the A/B reference (default: an arm named "
                         "base/baseline/default/prev/previous, else the last --arm).")
    # None => resolve per mode below: A/B (--arm) wants 32 cores / 16GB / 30min; other modes keep 4/14000/300.
    ap.add_argument("--timeout", type=int, default=None)
    ap.add_argument("--mem-mb", type=int, default=None)
    ap.add_argument("--jobs", type=int, default=None)
    args = ap.parse_args()
    arm_mode = bool(args.arm)
    timeout_explicit = args.timeout is not None
    if args.jobs is None:
        args.jobs = 32 if arm_mode else 4
    if args.mem_mb is None:
        args.mem_mb = 16000 if arm_mode else 14000
    if args.timeout is None:
        args.timeout = 1800 if arm_mode else 300
    preflight(args)
    if arm_mode:
        return abtest(args)
    if args.seedgate:
        if not args.configs and args.env is None:
            ap.error("--seedgate requires --configs <tag> OR --env 'KEY=VAL,...' (ad-hoc); "
                     "for A/B across configs use --arm TAG:ENV (repeatable)")
        if not timeout_explicit:
            args.timeout = 900   # seedgate default: 15min — long enough for the hard-10 headroom
        return seedgate(args)
    if args.validate:
        return validate(args)
    if args.smoke:
        return smoke(args)
    if args.stage1:
        return stage1(args)
    if args.stage2:
        if not args.configs:
            ap.error("--stage2 requires --configs tag1,tag2,...")
        if not timeout_explicit:
            args.timeout = 900
        return stage2(args)
    ap.error("one of --smoke / --stage1 / --stage2 required")


if __name__ == "__main__":
    sys.exit(main())
