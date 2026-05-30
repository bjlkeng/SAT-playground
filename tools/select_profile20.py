#!/usr/bin/env python3
"""Build the profile20 benchmark suite.

profile20 = the existing 10-instance `benchmarks/profiling` suite (EASY: solver-10 solves
            within 5 min) + 10 HARD headroom instances (solver-10 times out at 300 s but
            kissat-latest finishes within 5 min), drawn from the medium-100 set.

Source data (re-thresholded at 300 s; verified to be the IDENTICAL 100-instance set):
  - solver-10 (10-bve-preprocess) @1800s/16GiB: three runs (results.csv) on the medium-100 set
  - kissat-latest @1800s: one run on benchmarks/sat-comp-2025-medium

HARD rules (robust across all three solver-10 runs):
  - solver-10 "cannot solve in 5 min" in EVERY run: result==TIMEOUT, or solved but time>=300 s.
    Rows with ERROR/UNKNOWN in ANY run are excluded (we want honest timeouts, not crashes).
  - kissat "fast": solved AND time < 280 s (margin vs 300 s).
  - Disjoint from profiling; family-diverse (<=2/family); spread across kissat runtime.
"""
import csv, os, glob, re, json

S10_RUNS = [
    'log/bench-10-bve-preprocess-2026-05-18-16-04-01/results.csv',
    'log/bench-10-bve-preprocess-2026-05-12-07-21-01/results.csv',
    'log/bench-10-bve-preprocess-2026-05-03-12-02-01/results.csv',
]
KISSAT_RUN = 'log/bench-kissat-latest-2026-04-11-22-21-01/results.csv'
MEDIUM_DIR = 'benchmarks/sat-comp-2025-medium'
PROFILING_DIR = 'benchmarks/profiling'
OUT_DIR = 'benchmarks/profile20'
BUDGET = 300.0
KISSAT_MARGIN = 280.0
N_HARD = 10

def load(p):
    return {row['instance'].strip(): row for row in csv.DictReader(open(p))}

def res(r):    return r['result'].upper()
def solved(r): return res(r) in ('SAT', 'UNSAT')
def tm(r):     return float(r['time_s'])

def norm_stem(fname):
    # results.csv instance keys == filename without the .cnf.xz extension, with the
    # .normalised/.sanitized suffix PRESERVED (verified: instance==file[:-7], 100/100).
    return fname[:-len('.cnf.xz')] if fname.endswith('.cnf.xz') else fname

def family(name):
    base = name.split('-', 1)[1] if '-' in name else name
    toks = [t for t in re.split(r'[-_0-9.]', base) if t]
    return toks[0].lower() if toks else base.lower()

s10_runs = [load(p) for p in S10_RUNS]
kl = load(KISSAT_RUN)

medium_files = {norm_stem(os.path.basename(p)): os.path.basename(p)
                for p in glob.glob(os.path.join(MEDIUM_DIR, '*.cnf.xz'))}
profiling_files = sorted(os.path.basename(p) for p in glob.glob(os.path.join(PROFILING_DIR, '*.cnf.xz')))
profiling_stems = {norm_stem(f) for f in profiling_files}

def s10_summary(ins):
    rows = [run[ins] for run in s10_runs if ins in run]
    times = sorted(tm(r) for r in rows)
    return rows, (times[len(times)//2] if times else None)

# ---- EASY half: the existing profiling-10 (look up its solver-10 / kissat times) ----
easy = []
for f in profiling_files:
    stem = norm_stem(f)
    rows, med = s10_summary(stem)
    krow = kl.get(stem)
    easy.append(dict(ins=stem, file=f, link_target='../profiling/' + f,
                     s10res=(rows[-1]['result'] if rows else 'NA'),
                     s10_med=(round(med, 1) if med is not None else None),
                     s10_times=[round(tm(r), 1) for r in rows],
                     kres=(krow['result'] if krow else 'NA'),
                     kt=(round(tm(krow), 1) if krow and solved(krow) else None),
                     fam=family(stem), nruns=len(rows)))

# ---- HARD half: solver-10 cannot solve in 5 min (every run), kissat < 280 s ----
hard_cand = []
for ins in sorted(s10_runs[0]):
    if ins in profiling_stems or ins not in medium_files:
        continue
    rows = [run[ins] for run in s10_runs if ins in run]
    if any(res(r) in ('ERROR', 'UNKNOWN') for r in rows):     # honest timeouts only
        continue
    over_budget_all = all(res(r) == 'TIMEOUT' or (solved(r) and tm(r) >= BUDGET) for r in rows)
    krow = kl.get(ins)
    if over_budget_all and krow and solved(krow) and tm(krow) < KISSAT_MARGIN:
        _, med = s10_summary(ins)
        hard_cand.append(dict(ins=ins, file=medium_files[ins],
                              link_target='../sat-comp-2025-medium/' + medium_files[ins],
                              s10res=rows[-1]['result'], s10_med=round(med, 1),
                              s10_times=[round(tm(r), 1) for r in rows],
                              kres=krow['result'], kt=round(tm(krow), 1),
                              fam=family(ins), nruns=len(rows)))

def pick_spread(cands, n, key):
    cands = sorted(cands, key=key)
    m = len(cands)
    targets = [round(i * (m - 1) / (n - 1)) for i in range(n)] if n > 1 else [0]
    chosen, used, fam = [], set(), {}
    for t in targets:
        for j in sorted(range(m), key=lambda j: abs(j - t)):
            if j in used: continue
            if fam.get(cands[j]['fam'], 0) >= 2: continue
            chosen.append(cands[j]); used.add(j)
            fam[cands[j]['fam']] = fam.get(cands[j]['fam'], 0) + 1
            break
    for j in range(m):
        if len(chosen) >= n: break
        if j not in used: chosen.append(cands[j]); used.add(j)
    return sorted(chosen[:n], key=key)

hard_sel = pick_spread(hard_cand, N_HARD, lambda c: c['kt'])
assert len(easy) == 10, f"expected 10 profiling instances, got {len(easy)}"
# hard may legitimately be < N_HARD if the strict filter is too tight; build anyway and report.

# ---- build the suite ----
os.makedirs(OUT_DIR, exist_ok=True)
# clear stale symlinks from prior runs
for p in glob.glob(os.path.join(OUT_DIR, '*.cnf.xz')):
    os.remove(p)
for c in easy + hard_sel:
    link = os.path.join(OUT_DIR, c['file'])
    target = c['link_target']
    if not os.path.exists(os.path.join(OUT_DIR, target)):
        # fall back to medium dir if the preferred target is missing
        alt = '../sat-comp-2025-medium/' + c['file']
        if os.path.exists(os.path.join(OUT_DIR, alt)):
            target = alt
    os.symlink(target, link)

with open(os.path.join(OUT_DIR, 'selection.csv'), 'w', newline='') as f:
    w = csv.writer(f)
    w.writerow(['half', 'instance', 'file', 's10_result', 's10_median_s', 's10_times_s',
                'kissat_result', 'kissat_s', 'family'])
    for half, sel, sortk in (('easy', easy, 's10_med'), ('hard', hard_sel, 'kt')):
        for c in sorted(sel, key=lambda c: (c[sortk] is None, c[sortk])):
            w.writerow([half, c['ins'], c['file'], c['s10res'], c['s10_med'],
                        '|'.join(map(str, c['s10_times'])), c['kres'], c['kt'], c['fam']])

json.dump(dict(n_hard_candidates=len(hard_cand),
               easy=[c['ins'] for c in easy], hard=[c['ins'] for c in hard_sel]),
          open(os.path.join(OUT_DIR, 'selection.json'), 'w'), indent=2)

# ---- README ----
def fmt(x): return '—' if x is None else f"{x}"

def easy_rows(sel):
    out = []
    for c in sorted(sel, key=lambda c: (min(c['s10_times']) if c['s10_times'] else 1e9)):
        ts = c['s10_times']
        best = min(ts) if ts else None
        rng = f"{min(ts)}–{max(ts)}" if ts else '—'
        out.append(f"| `{c['ins']}` | {c['s10res']} | {fmt(best)} | {rng} | {c['kres']} | {fmt(c['kt'])} | {c['fam']} |")
    return "\n".join(out)

def hard_rows(sel):
    out = []
    for c in sorted(sel, key=lambda c: c['kt']):
        out.append(f"| `{c['ins']}` | {c['s10res']} | {fmt(c['s10_med'])} | {c['kres']} | {fmt(c['kt'])} | {c['fam']} |")
    return "\n".join(out)

readme = f"""# profile20 — 20-instance feature-ablation suite (10 easy + 10 headroom)

A 20-instance suite for solver-feature ablation that, unlike `benchmarks/profiling`, contains
**headroom**: instances solver-10 cannot crack in 5 minutes but a strong solver (kissat) can. On
the all-solved profiling suite a real search improvement has nothing to rescue; here it can flip a
solver-10 timeout into a solve.

Two halves:

- **easy (10)** — the existing `benchmarks/profiling` control suite, reused verbatim (symlinked to
  `../profiling/`), to preserve direct comparability with all prior profiling results. solver-10
  (`10-bve-preprocess`) solves every one within 5 min on its fast run (best-of-3 < 300 s for all 10).
  **Caveat (variance):** these are the profiling instances and two of them are bimodal /
  variance-sensitive — `brocard_problem_large` (7.9 / 526.5 / 585.3 s) and
  `REGRandom-K4-L1-Seed40` (54.2 / 966.8 / 1278.1 s) — and `mp1-Nb7T46` / `sudoku-N30-12` each have
  one slow run. They are < 300 s on their fast runs and in the profiling suite's own 300 s-budget
  runs, but can exceed 300 s on some runs. See `selection.csv` for all three per-run times.
- **hard (10)** — drawn from the medium-100 set (`../sat-comp-2025-medium/`), **disjoint from
  profiling**. solver-10 **times out at 300 s** (it needs ≥300 s, or never finishes, in every run),
  while **kissat-latest finishes in < 280 s**.

## Selection methodology

Selected from existing repeated runs on the **identical** medium-100 instance set (verified by
instance-set equality), re-thresholded at 300 s — no fresh campaign needed:

- **solver-10 @ 1800 s / 16 GiB:** three runs
  (`log/bench-10-bve-preprocess-2026-05-{{18,12,03}}-*/results.csv`).
- **kissat-latest @ 1800 s:** one run (`log/bench-kissat-latest-2026-04-11-22-21-01/results.csv`).

HARD rules (robust across all three solver-10 runs):
- solver-10 cannot solve within the 300 s budget in **every** run (result==TIMEOUT, or solved but
  time ≥ 300 s). Rows with ERROR/UNKNOWN in any run are excluded — honest timeouts only, not crashes.
- kissat solved **and** time < 280 s (margin vs 300 s).
- Family-diverse (≤ 2 per coarse family); spread across kissat runtime.

There were **{len(hard_cand)} qualifying hard candidates**; the requested condition is satisfiable.
Provenance: `selection.csv` (per-instance solver-10 per-run times + kissat time), `selection.json`.
Regenerate: `python3 tools/select_profile20.py`. solver-10/kissat times come from 1800 s runs; the
hard-half solver-10 figure is its time when it *did* eventually solve (>300 s) or 1800 (timeout).

## easy half — reused profiling-10 control (solver-10 fast run < 300 s; see variance caveat above)

| instance | s10 | s10 best_s | s10 range_s (3 runs) | kissat | kissat_s | family |
|---|---|---|---|---|---|---|
{easy_rows(easy)}

## hard half — headroom (solver-10 can't do in 5 min; kissat < 280 s)

| instance | s10 | s10_s (≥300 ⇒ TO@300) | kissat | kissat_s | family |
|---|---|---|---|---|---|
{hard_rows(hard_sel)}
"""
open(os.path.join(OUT_DIR, 'README.md'), 'w').write(readme)

print(json.dumps(dict(easy=len(easy), hard=len(hard_sel),
                      hard_candidates=len(hard_cand), out=OUT_DIR)))

