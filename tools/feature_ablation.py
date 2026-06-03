#!/usr/bin/env python3
"""Solver-11 feature ablation on the profile20 suite.

Process (per the 2026-05-30 design):
  * Target suite = benchmarks/profile20 (10 easy control + 10 hard headroom).
  * Decision metric = aggregate PAR-2 over all 20, with easy-10 / hard-10 reported separately.
  * Parallelism = 4 workers pinned to physical cores 0,1,2,3 (taskset), one bench -j1 each over a
    4-way shard; siblings left idle for clean timing. Memory capped at 14 GiB/job (4x14<57 free).
  * 3% repeat rule: a config within +/-3% of the baseline PAR-2 is in the noise band -> rerun
    (up to n=3, take the mean); a clear (>3%) win/loss is accepted from n=1.
  * Two stages: Stage 1 screens every config at 300 s on all 20; Stage 2 (separate invocation)
    re-runs a shortlist at a long timeout on the hard-10 only to measure real headroom.

Each config is a same-binary SAT_* env toggle on solver/11-kissat-port (built once); `solver10`
is the solver/10-bve-preprocess reference floor. Flag `requires` deps (CONFIG_SCHEMA.csv) are
encoded so no toggle is a silent no-op; parent-only controls (use_lbd, lbd_tiered, fstab) are
included for attribution.

Usage:
  python3 tools/feature_ablation.py --stage1 [--timeout 300] [--mem-mb 14000] [--jobs 4]
  python3 tools/feature_ablation.py --stage2 --configs tag1,tag2 [--timeout 900]
  python3 tools/feature_ablation.py --smoke           # 2 configs x 2 instances, fast self-check
"""
from __future__ import annotations
import argparse, csv, os, shutil, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import compare_bench  # noqa: E402

SUITE = ROOT / "benchmarks" / "profile20"
SELECTION = SUITE / "selection.csv"
S11 = "solver/11-kissat-port"
S10 = "solver/10-bve-preprocess"
CORES = [0, 1, 2, 3]            # distinct physical cores (siblings 6..9 left idle)
TOL = 0.03                      # 3% noise band for the repeat rule

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

    def run(job):
        idx, stem, seed = job
        core = CORES[idx % len(CORES)]
        odir = camp / "_work" / f"{idx}"
        odir.mkdir(parents=True, exist_ok=True)
        env = {**os.environ, **env_extra, "SAT_SEED": str(seed), "SAT_STATS_JSON": "on"}
        cmd = ["taskset", "-c", str(core), "bash", "-c",
               f'ulimit -v {args.mem_mb*1024}; exec timeout {args.timeout} '
               f'"{ROOT/solver_dir}/target/release/sat-solver" "{cnf_for[stem]}" "{odir}"']
        t0 = time.time()
        try:
            try:
                p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, env=env, timeout=args.timeout + 30)
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
                import re
                g = lambda k: (re.search(rf'"{k}":([0-9.eE+-]+)', js) or [None, "NA"])[1]
                return (stem, seed, res, dt, g("conflicts"), g("propagations"), g("decisions"))
            except subprocess.TimeoutExpired:
                return (stem, seed, "TIMEOUT", time.time() - t0, "NA", "NA", "NA")
        finally:
            # Drop the per-run DRAT proof scratch immediately. These are multi-GB on hard-UNSAT
            # instances and are never read back; left to accumulate across a seedgate sweep they
            # fill the disk and kill the run (see log/hard-search-2026-06-02 _work blowout).
            shutil.rmtree(odir, ignore_errors=True)

    from concurrent.futures import ThreadPoolExecutor
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
                    help="multi-seed per-(config,instance,seed) sweep -> gate TSV (use with --configs <tag>)")
    ap.add_argument("--seeds", type=int, default=10, help="seeds for --seedgate (default 10)")
    ap.add_argument("--half", default="", help="restrict --seedgate to 'easy' or 'hard' half")
    ap.add_argument("--configs", default="")
    ap.add_argument("--timeout", type=int, default=300)
    ap.add_argument("--mem-mb", type=int, default=14000)
    ap.add_argument("--jobs", type=int, default=4)
    args = ap.parse_args()
    if args.seedgate:
        if not args.configs:
            ap.error("--seedgate requires --configs <single-tag>")
        if args.timeout == 300:
            args.timeout = 600   # seedgate default: longer to minimize censoring
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
        if args.timeout == 300:
            args.timeout = 900
        return stage2(args)
    ap.error("one of --smoke / --stage1 / --stage2 required")


if __name__ == "__main__":
    sys.exit(main())
