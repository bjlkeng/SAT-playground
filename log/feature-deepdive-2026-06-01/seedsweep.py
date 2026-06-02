#!/usr/bin/env python3
"""Seed-distribution sweep: default vs binary_fast / target_phase / chrono on the easy-13 set.

For each (config, instance, seed) run the solver with SAT_SEED, a memory cap (ulimit -v), and a
timeout. Record result, conflicts (contention-immune search-quality metric), and wall time.
Runs JOBS in parallel, each pinned to a physical core via taskset. Conflicts are the primary signal;
time/PAR-2 secondary. Resumable: skips (config,inst,seed) whose result file already exists.
"""
import csv, os, subprocess, sys, time, itertools
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

ROOT = Path("/home/bojji/code/SAT-playground")
SOLVER = ROOT / "solver/11-kissat-port/target/release/sat-solver"
SUITE = Path("/tmp/p20deep")
OUT = ROOT / "log/seedsweep-2026-06-01-v2"
OUT.mkdir(parents=True, exist_ok=True)
EASY13 = [l.strip() for l in open("/tmp/p20deep/easy13.txt") if l.strip()]
SEEDS = [0, 1, 2, 3, 4]
TIMEOUT = 300
MEM_MB = 14000
CORES = [0, 1, 2, 3]
CONFIGS = {
    "default": {},
    "binary_fast": {"SAT_BINARY_FAST": "on"},
    "target_phase": {"SAT_PHASE": "target-then-saved"},
    "chrono": {"SAT_CHRONO": "on"},
}

jobs = list(itertools.product(CONFIGS, EASY13, SEEDS))
print(f"[seedsweep] {len(jobs)} runs ({len(CONFIGS)} cfg x {len(EASY13)} inst x {len(SEEDS)} seeds), "
      f"t={TIMEOUT}s m={MEM_MB}MB jobs={len(CORES)}", flush=True)

def run(idx_job):
    idx, (cfg, inst, seed) = idx_job
    core = CORES[idx % len(CORES)]
    rdir = OUT / cfg / inst
    rdir.mkdir(parents=True, exist_ok=True)
    rfile = rdir / f"seed{seed}.txt"
    if rfile.exists():
        return f"skip {cfg}/{inst[:20]}/s{seed}"
    cnf = SUITE / (inst + ".cnf")
    odir = OUT / "_work" / f"{cfg}_{idx}"
    odir.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, **CONFIGS[cfg], "SAT_SEED": str(seed), "SAT_STATS_JSON": "on"}
    # ulimit -v (KB) in a shell wrapper, taskset for pinning, timeout for the cap
    cmd = ["taskset", "-c", str(core), "bash", "-c",
           f'ulimit -v {MEM_MB*1024}; exec timeout {TIMEOUT} "{SOLVER}" "{cnf}" "{odir}"']
    t0 = time.time()
    try:
        p = subprocess.run(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           text=True, timeout=TIMEOUT + 30)
        dt = time.time() - t0
        res = next((ln.split()[1] for ln in p.stdout.splitlines() if ln.startswith("s ")), None)
        rc = p.returncode
        if res is None:
            res = "TIMEOUT" if rc == 124 else f"UNKNOWN_rc{rc}"
        js = ""
        for ln in p.stderr.splitlines():
            if '"conflicts"' in ln:
                br = ln.find("{")
                if br >= 0:
                    js = ln[br:]   # strip the 'c JSON_STATS ' prefix
        def grab(k):
            import re
            m = re.search(rf'"{k}":([0-9.eE+-]+)', js)
            return m.group(1) if m else "NA"
        conflicts = grab("conflicts"); props = grab("propagations"); decs = grab("decisions")
    except subprocess.TimeoutExpired:
        dt = time.time() - t0; res = "TIMEOUT"; conflicts = props = decs = "NA"
    rfile.write_text(f"{cfg}\t{inst}\t{seed}\t{res}\t{dt:.1f}\t{conflicts}\t{props}\t{decs}\n")
    return f"{cfg}/{inst[:18]}/s{seed} {res} {dt:.0f}s conf={conflicts}"

done = 0
with ThreadPoolExecutor(max_workers=len(CORES)) as ex:
    for msg in ex.map(run, enumerate(jobs)):
        done += 1
        if not msg.startswith("skip"):
            print(f"[{done}/{len(jobs)}] {msg}", flush=True)

# aggregate
agg = OUT / "results.tsv"
with open(agg, "w") as out:
    out.write("config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\n")
    for cfg in CONFIGS:
        for inst in EASY13:
            for seed in SEEDS:
                rf = OUT / cfg / inst / f"seed{seed}.txt"
                if rf.exists():
                    out.write(rf.read_text())
(OUT / "DONE").write_text("seedsweep complete\n")
print(f"[seedsweep] DONE -> {agg}", flush=True)
