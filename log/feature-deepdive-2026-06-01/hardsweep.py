#!/usr/bin/env python3
"""Hard-instance seed sweep: default (fstab_lbdtier) vs +binary_fast/+target_phase/+chrono.

3 hard instances the default CAN solve (circuit, div-mitern172, sqrt-mitern171) x 10 seeds x 4
configs = 120 runs. 600s cap to minimize censoring. Each feature layers ON TOP of the promoted
default (empty env = default), so this measures the feature's effect on the shipped config where it
has headroom to help. Conflicts (contention-immune) primary; time/PAR-2 secondary. 4 pinned cores.
Resumable. Result rows: config<TAB>instance<TAB>seed<TAB>result<TAB>time<TAB>conflicts<TAB>props<TAB>decisions
"""
import os, re, subprocess, time, itertools
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

ROOT = Path("/home/bojji/code/SAT-playground")
SOLVER = ROOT / "solver/11-kissat-port/target/release/sat-solver"
SRC = Path("/tmp/p20deep")
OUT = ROOT / "log/hardsweep-2026-06-02"
OUT.mkdir(parents=True, exist_ok=True)
INSTANCES = [
    "849950561ddce887c78fef773dccfa80-circuit_48in64out_with_800gates_4in4out_dist128_seed3.sanitized",
    "f0bafebdcce23ccfbaf6c27a7522069b-div-mitern172",
    "31e843c53a76ff3961935ad55b953298-sqrt-mitern171",
]
SEEDS = list(range(10))
TIMEOUT = 600
MEM_MB = 14000
CORES = [0, 1, 2, 3]
CONFIGS = {                                   # all layer on top of the promoted default (empty env)
    "default": {},
    "binary_fast": {"SAT_BINARY_FAST": "on"},
    "target_phase": {"SAT_PHASE": "target-then-saved"},
    "chrono": {"SAT_CHRONO": "on"},
}

jobs = list(itertools.product(CONFIGS, INSTANCES, SEEDS))
print(f"[hardsweep] {len(jobs)} runs ({len(CONFIGS)} cfg x {len(INSTANCES)} inst x {len(SEEDS)} seeds), "
      f"t={TIMEOUT}s m={MEM_MB}MB jobs={len(CORES)}", flush=True)

def run(idx_job):
    idx, (cfg, inst, seed) = idx_job
    core = CORES[idx % len(CORES)]
    rdir = OUT / cfg / inst
    rdir.mkdir(parents=True, exist_ok=True)
    rfile = rdir / f"seed{seed}.txt"
    if rfile.exists():
        return f"skip {cfg}/{inst[:16]}/s{seed}"
    cnf = SRC / (inst + ".cnf")
    odir = OUT / "_work" / f"{cfg}_{idx}"
    odir.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, **CONFIGS[cfg], "SAT_SEED": str(seed), "SAT_STATS_JSON": "on"}
    cmd = ["taskset", "-c", str(core), "bash", "-c",
           f'ulimit -v {MEM_MB*1024}; exec timeout {TIMEOUT} "{SOLVER}" "{cnf}" "{odir}"']
    t0 = time.time()
    try:
        p = subprocess.run(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           text=True, timeout=TIMEOUT + 30)
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
        conflicts, props, decs = g("conflicts"), g("propagations"), g("decisions")
    except subprocess.TimeoutExpired:
        dt = time.time() - t0; res = "TIMEOUT"; conflicts = props = decs = "NA"
    rfile.write_text(f"{cfg}\t{inst}\t{seed}\t{res}\t{dt:.1f}\t{conflicts}\t{props}\t{decs}\n")
    return f"{cfg}/{inst[:16]}/s{seed} {res} {dt:.0f}s conf={conflicts}"

done = 0
with ThreadPoolExecutor(max_workers=len(CORES)) as ex:
    for msg in ex.map(run, enumerate(jobs)):
        done += 1
        if not msg.startswith("skip"):
            print(f"[{done}/{len(jobs)}] {msg}", flush=True)

agg = OUT / "results.tsv"
with open(agg, "w") as out:
    out.write("config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\n")
    for cfg in CONFIGS:
        for inst in INSTANCES:
            for seed in SEEDS:
                rf = OUT / cfg / inst / f"seed{seed}.txt"
                if rf.exists():
                    out.write(rf.read_text())
(OUT / "DONE").write_text("hardsweep complete\n")
print(f"[hardsweep] DONE -> {agg}", flush=True)
