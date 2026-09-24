#!/usr/bin/env python3
"""rl_collect.py — the RL scheduler's collection harness for solver 13.

Plan: plan/rl-scheduler-solver13-plan.md §5 (data collection), §7 items 6c,
6d and 7, §15 item 6. Bead SAT-playground-p9m.7.1 (step B.1).

feature_ablation.py times an A/B and throws the scratch away. This tool
keeps what the RL plan needs: the raw-state policy log of every run, the
stdout with the answer, a per-cell record, and a manifest, and it knows
about fork children and tick budgets.

One invocation is one PASS over a job table. A job is one solver run on
one cell: a flavour (stock / fork / random / jitter), the solver seed, a
work-clock budget (SAT_LIMIT_TICKS) or a wall limit, and the policy
environment that shapes the run. The stock pass builds its table from a
suite; later passes (round 0) hand in a table built from the per-cell
budgets. Everything the solver needs is passed through the environment it
already reads (solver/13-kissat-rs/README.md, "Environment").

    # the stock traces (plan §5.2): policy on with the stock action, logging
    # on, wall-limited, proofs off, one run per cell
    python3 tools/rl_collect.py stock --suite sat-comp-2025 --name stock2025 \
        --timeout 1800 --jobs 32 --mem-mb 16000 \
        --oracle log/solver13-full-accept3-20260905-164713/results.csv

    # a job table (round 0): cells with tick budgets and fork branch points
    python3 tools/rl_collect.py table jobs.tsv --name round0 --jobs 32

    # progress of a running or finished pass
    python3 tools/rl_collect.py status log/rl-stock2025-<timestamp>

Run directory `log/rl-<name>-<timestamp>/`:
    manifest.json       the pass: binary and its sha256, suite, git head,
                        host, limits, cores, environment, job table, times
    bin/sat-solver      a frozen copy of the binary (a rebuild during the
                        pass cannot change what runs; the review loop
                        rebuilds target/release, see the solver README)
    jobs.tsv            the job table as run (resume reads it back)
    logs/<stem>.<tag>.log        the policy log (+ .b<D>.<k>[.out|.err] for
                                 fork children, written by the solver)
    out/<stem>.<tag>.out|.err    the run's stdout (answer, model, -s block,
                                 `c workclock` line) and stderr
    cells/<stem>.<tag>.json      one record per finished job (the resume key)
    results.tsv         every record, one row per job, rewritten as the
                        pass runs and at the end
    DONE                written when every job has a record

Process accounting. Every job is its own session and process group
(`setsid`), pinned with `taskset` to as many cores as it has processes
(1 + the live children of a fork parent), run under `ulimit -v` per
process and `timeout -k 30 <wall>`. Killing the pass (SIGINT/SIGTERM, or
`kill <pid>` of the collector) sends SIGTERM to every live group so each
solver prints its `c workclock` line and seals its log, waits a grace
period, SIGKILLs what is left, and checks that nothing survived. The
32-process cap (CLAUDE.md "Benchmarking operations") is enforced on
processes, not parents: a fork parent with 4 live children takes 5 slots.

Correctness (plan §7 item 6c). Every SAT answer, parent or child, has its
model checked by tools/verify_sat.py against the CNF before the scratch
copy is deleted. Every parent and child of one branch point that answers
must agree (SAT v UNSAT); `--oracle` adds known statuses from earlier
runs. A disagreement is a solver bug: the pass stops admitting jobs,
finishes the live ones, and exits 3. Proofs are always off.

Budgets. A wall-limited job (the stock pass) records TIMEOUT as a result.
A tick-budgeted job (round 0) records `s UNKNOWN` at the budget as its
result; its wall limit is a safety cap only (2x the expected wall by
default) and hitting it is an ANOMALY in the record, not a result.

Job table (TSV, header row; `stock` writes one too). Columns:
    stem         instance stem (the file name without .cnf.xz)
    tag          a label unique per (stem, tag) in the pass: stock, fork3, ...
    flavour      stock | fork | random | jitter | off | net
                 (off: the policy off and no log, the plain solver; net: the
                 learned policy, env carries SAT_POLICY=<weights file> and
                 SAT_POLICY_MARGIN; both are arms of the three-arm gates,
                 plan section 8)
    seed         the solver's --seed (every flavour uses the paired stock
                 run's seed, plan §5.4)
    limit_ticks  work-clock budget (SAT_LIMIT_TICKS), or empty = wall-limited
    wall_s       the wall limit (wall-limited) or the safety cap (budgeted)
    procs        processes the job may have alive (1 + SAT_POLICY_BRANCH_JOBS)
    peak_rss_mb  the cell's peak RSS from the stock trace (memory
                 admission for fork jobs), or empty
    env          extra environment, `KEY=VALUE` pairs separated by spaces;
                 SAT_EXTRA_ARGS=<kissat options> goes on the command line
                 instead, several options separated by commas
                 (SAT_EXTRA_ARGS=--reducelow=250,--reducehigh=450)
                 (SAT_POLICY_BRANCH=3:reduce,9:mode SAT_POLICY_BRANCH_JOBS=4);
                 a value may hold `;` (SAT_POLICY_BRANCH_ACTIONS=probe=0|4;reduce=2)
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from feature_ablation import death_note, numa_balanced_cores, parse_workclock  # noqa: E402

SOLVER_DIR = ROOT / "solver" / "13-kissat-rs"
VERIFY_SAT = ROOT / "tools" / "verify_sat.py"
FLAVOURS = ("stock", "fork", "random", "jitter", "off", "net")
PROC_CAP = 32            # CLAUDE.md: at most 32 concurrent solver processes
KILL_GRACE_S = 30        # timeout -k and the shutdown grace: let the solver seal its log
TSV_COLUMNS = ("tag", "stem", "flavour", "seed", "result", "wall_s", "exit", "limit_ticks",
               "ticks", "eliminate_resolutions", "work", "conflicts", "rows", "footer",
               "peak_rss_mb", "verify", "oracle", "children", "agree", "anomaly", "failed", "note")
JOB_COLUMNS = ("stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs", "peak_rss_mb", "env")
SOLVED = {"SAT", "UNSAT"}


# ---------------------------------------------------------------------------
# small helpers
# ---------------------------------------------------------------------------

def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def git_head() -> str:
    try:
        return subprocess.run(["git", "rev-parse", "--short=12", "HEAD"], cwd=ROOT, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout.strip()
    except Exception:
        return ""


def resolve_suite(name: str) -> Path:
    p = Path(name)
    if p.is_absolute():
        return p
    for cand in (ROOT / name, ROOT / "benchmarks" / name):
        if cand.is_dir():
            return cand
    raise SystemExit(f"suite not found: {name}")


def suite_stems(suite: Path) -> list[str]:
    stems = sorted(p.name[:-len(".cnf.xz")] for p in suite.glob("*.cnf.xz"))
    if not stems:
        raise SystemExit(f"no *.cnf.xz in {suite}")
    return stems


def cnf_source(suite: Path, stem: str) -> Path:
    for suffix in (".cnf.xz", ".cnf.gz", ".cnf"):
        p = suite / (stem + suffix)
        if p.is_file() or p.is_symlink():
            return p
    raise RuntimeError(f"missing CNF for {stem} in {suite}")   # a worker's harness error, not an exit


def parse_env(spec: str) -> dict[str, str]:
    """`K=V K=V` to a dict. Whitespace separates items, so a value may hold
    commas, colons, bars and semicolons (SAT_POLICY_BRANCH=3:reduce,9:mode,
    SAT_POLICY_BRANCH_ACTIONS=probe=0|4;reduce=0.5|2); no solver variable
    takes a value with a space."""
    out: dict[str, str] = {}
    if not spec:
        return out
    for item in spec.split():
        item = item.strip()
        if not item:
            continue
        if "=" not in item:
            raise SystemExit(f"bad env item {item!r} (expected KEY=VALUE)")
        k, v = item.split("=", 1)
        out[k.strip()] = v.strip()
    return out


def status_of(out_path: Path) -> tuple[str, str]:
    """(SAT | UNSAT | UNKNOWN | NONE, the `c workclock` line) from a solver stdout file."""
    res, wc = "NONE", ""
    try:
        with open(out_path, "r", errors="replace") as f:
            for ln in f:
                if ln.startswith("s "):
                    word = ln.split(None, 1)[1].strip() if len(ln.split()) > 1 else ""
                    res = {"SATISFIABLE": "SAT", "UNSATISFIABLE": "UNSAT"}.get(word, "UNKNOWN")
                elif ln.startswith("c workclock "):
                    wc = ln
    except FileNotFoundError:
        pass
    return res, wc


def log_summary(path: Path) -> dict:
    """Header policy block and the footer of a policy log without reading its rows.

    Reads the two header lines and the tail of the file. `footer` is None
    when the log has no footer (cut off: a write failure or a kill the
    handler could not finish); `rows` then comes from the file size.
    """
    info: dict = {"rows": None, "footer": None, "header": None, "size": None}
    try:
        size = path.stat().st_size
        info["size"] = size
        with open(path, "rb") as f:
            magic = f.readline()
            if not magic.startswith(b"SAT13POLICYLOG"):
                info["error"] = "not a policy log"
                return info
            header = json.loads(f.readline())
            info["header"] = {k: header.get(k) for k in ("pid", "policy", "branch", "k_res")}
            row_bytes = int(header["row_bytes"])
            body_start = f.tell()
            tail_len = min(size - body_start, max(1 << 16, row_bytes + (1 << 15)))
            f.seek(size - tail_len)
            tail = f.read(tail_len)
        # The footer is the last line, a JSON object after the sentinel row.
        end = tail.rfind(b"}\n")
        start = tail.rfind(b'{"result"', 0, end + 1 if end >= 0 else len(tail))
        if end >= 0 and start >= 0:
            try:
                info["footer"] = json.loads(tail[start:end + 1])
                info["rows"] = int(info["footer"].get("rows", 0))
                return info
            except json.JSONDecodeError:
                pass
        info["rows"] = max(0, (size - body_start) // row_bytes)
    except (OSError, ValueError, KeyError) as e:
        info["error"] = str(e)
    return info


def oracle_load(paths: list[str]) -> dict[str, dict]:
    """Known SAT/UNSAT statuses and times per stem from earlier runs.

    Reads run_kissat_full.sh `results.csv` (instance,result,time_s,...) and
    feature_ablation.py `results.tsv` (config, instance, seed, result,
    time_s, ...). A TIMEOUT/UNKNOWN teaches nothing about the status but
    keeps its time for ordering.
    """
    out: dict[str, dict] = {}
    for p in paths:
        path = Path(p)
        if not path.is_file():
            raise SystemExit(f"--oracle {p}: not a file")
        with open(path, newline="") as f:
            dialect = "excel-tab" if path.suffix == ".tsv" else "excel"
            for row in csv.DictReader(f, dialect=dialect):
                stem = row.get("instance", "").strip()
                if not stem:
                    continue
                res = row.get("result", "").strip().upper()
                res = {"SATISFIABLE": "SAT", "UNSATISFIABLE": "UNSAT"}.get(res, res)
                try:
                    t = float(row.get("time_s", "nan"))
                except ValueError:
                    t = float("nan")
                rec = out.setdefault(stem, {"status": None, "time_s": None, "sources": []})
                rec["sources"].append(str(path))
                if res in SOLVED:
                    if rec["status"] and rec["status"] != res:
                        raise SystemExit(f"oracles disagree on {stem}: {rec['status']} v {res} ({path})")
                    rec["status"] = res
                if t == t:
                    rec["time_s"] = max(rec["time_s"] or 0.0, t)
    return out


# ---------------------------------------------------------------------------
# jobs
# ---------------------------------------------------------------------------

class Aborted(Exception):
    """A job stopped before its solver was launched (the pass was shutting down)."""


BAD_VERIFY = ("FAIL", "checker-timeout", "checker-error")   # an unverified SAT answer never passes the gate


class Job:
    def __init__(self, row: dict):
        self.stem = row["stem"].strip()
        self.tag = row["tag"].strip()
        self.flavour = row["flavour"].strip()
        if self.flavour not in FLAVOURS:
            raise SystemExit(f"job {self.stem}/{self.tag}: flavour {self.flavour!r} not in {FLAVOURS}")
        self.seed = int(row.get("seed", "0") or 0)
        lt = (row.get("limit_ticks") or "").strip()
        self.limit_ticks = int(lt) if lt else None
        self.wall_s = int(float(row["wall_s"]))
        self.procs = int(row.get("procs") or 1)
        pr = (row.get("peak_rss_mb") or "").strip()
        self.peak_rss_mb = float(pr) if pr else None
        self.env = parse_env(row.get("env") or "")
        if self.flavour == "fork" and "SAT_POLICY_BRANCH" not in self.env:
            raise SystemExit(f"job {self.stem}/{self.tag}: a fork job needs SAT_POLICY_BRANCH in env")
        if self.flavour == "fork" and self.limit_ticks is None:
            raise SystemExit(f"job {self.stem}/{self.tag}: a fork job needs limit_ticks (children stop on it)")
        if self.flavour == "fork":
            self.procs = max(self.procs, 1 + int(self.env.get("SAT_POLICY_BRANCH_JOBS", "4")))
        if self.flavour == "net" and not self.env.get("SAT_POLICY"):
            raise SystemExit(f"job {self.stem}/{self.tag}: a net job needs SAT_POLICY=<weights file> in env")
        if self.flavour == "off" and any(k.startswith("SAT_POLICY") for k in self.env):
            raise SystemExit(f"job {self.stem}/{self.tag}: an off job runs the plain solver and cannot carry SAT_POLICY* env")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+", self.tag):
            raise SystemExit(f"job {self.stem}/{self.tag}: tag must be [A-Za-z0-9_.-]+")

    @property
    def key(self) -> str:
        return f"{self.stem}.{self.tag}"

    def row(self) -> dict:
        return {"stem": self.stem, "tag": self.tag, "flavour": self.flavour, "seed": self.seed,
                "limit_ticks": "" if self.limit_ticks is None else self.limit_ticks,
                "wall_s": self.wall_s, "procs": self.procs,
                "peak_rss_mb": "" if self.peak_rss_mb is None else self.peak_rss_mb,
                "env": " ".join(f"{k}={v}" for k, v in self.env.items())}

    def solver_env(self, run: "Run") -> dict[str, str]:
        """The SAT_* environment for this job (plan §7 item 6; solver README table)."""
        # `off` is the plain solver: no policy, no log (the record comes from
        # stdout: the `s` line and the `c workclock` line)
        env = {} if self.flavour == "off" else {"SAT_POLICY_LOG": str(run.log_path(self))}
        if self.flavour == "stock":
            env["SAT_POLICY"] = "stock"
        elif self.flavour == "random":
            env["SAT_POLICY"] = "random"
        elif self.flavour == "jitter":
            env["SAT_POLICY"] = "jitter"
        # fork: the parent's policy is whatever the env says (stock when unset);
        # net: SAT_POLICY is the weights file the table's env names
        if self.limit_ticks is not None:
            env["SAT_LIMIT_TICKS"] = str(self.limit_ticks)
        else:
            env["SAT_WALL_LIMIT"] = str(self.wall_s)
        if run.epochs:
            env["SAT_POLICY_EPOCH_TICKS"] = run.epochs
        env.update(self.env)   # the table's env wins (an explicit SAT_POLICY_HORIZON, a branch spec)
        return env


def read_jobs(path: Path) -> list[Job]:
    with open(path, newline="") as f:
        rows = list(csv.DictReader(f, dialect="excel-tab"))
    missing = [c for c in JOB_COLUMNS if rows and c not in rows[0]]
    if missing:
        raise SystemExit(f"{path}: job table lacks columns {missing}")
    jobs = [Job(r) for r in rows]
    keys = [j.key for j in jobs]
    dup = {k for k in keys if keys.count(k) > 1}
    if dup:
        raise SystemExit(f"{path}: duplicate (stem, tag): {sorted(dup)[:5]}")
    return jobs


def write_jobs(path: Path, jobs: list[Job]) -> None:
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=JOB_COLUMNS, dialect="excel-tab")
        w.writeheader()
        for j in jobs:
            w.writerow(j.row())


def stock_jobs(stems: list[str], seed: int, timeout: int) -> list[Job]:
    return [Job({"stem": s, "tag": "stock", "flavour": "stock", "seed": str(seed), "limit_ticks": "",
                 "wall_s": str(timeout), "procs": "1", "peak_rss_mb": "", "env": ""}) for s in stems]


# ---------------------------------------------------------------------------
# the pass
# ---------------------------------------------------------------------------

class Run:
    def __init__(self, run_dir: Path, suite: Path, args, jobs: list[Job], resume: bool):
        self.dir = run_dir
        self.suite = suite
        self.jobs = jobs
        self.mem_mb = int(args.mem_mb)
        self.proc_cap = int(args.jobs)
        self.epochs = getattr(args, "epochs", "") or ""
        self.verify = not getattr(args, "no_verify", False)
        self.on_contradiction = getattr(args, "on_contradiction", "stop")
        self.mem_total_mb = int(getattr(args, "mem_total_mb", 0) or 0)
        self.oracle = oracle_load(getattr(args, "oracle", None) or [])
        # the scratch is always a directory of this pass's own, even under a
        # user-supplied root, because the pass deletes it at the end
        scratch_root = Path(getattr(args, "scratch", "") or "/tmp")
        self.scratch = scratch_root / f"rl-collect-{run_dir.name}-{os.getpid()}"
        self.lock = threading.Lock()
        self.write_lock = threading.Lock()     # results.tsv: one writer at a time
        self.n_admitted = 0                    # admitted and not yet finished (launched or still decompressing)
        self.stop = threading.Event()          # a signal: kill the live jobs, exit 130
        self.drain = threading.Event()         # a contradiction: admit nothing more, let the live jobs finish
        self.contradiction = threading.Event()
        self.live: dict[str, subprocess.Popen] = {}
        self.killed: set[str] = set()      # jobs the shutdown ended or the harness failed on: no record, a resume reruns them
        self.harness_errors = 0
        self.records: dict[str, dict] = {}
        explicit = [int(c) for c in (getattr(args, "cores", "") or "").split(",") if c.strip()]
        self.cores = explicit[:self.proc_cap] if explicit else numa_balanced_cores(self.proc_cap)
        if len(self.cores) < self.proc_cap:
            raise SystemExit(f"only {len(self.cores)} cores for --jobs {self.proc_cap}")
        self.free_cores = list(self.cores)
        self.mem_used_mb = 0.0
        for d in ("bin", "logs", "out", "cells"):
            (run_dir / d).mkdir(parents=True, exist_ok=True)
        self.scratch.mkdir(parents=True, exist_ok=True)
        self.binary = run_dir / "bin" / "sat-solver"
        src = Path(getattr(args, "binary", "") or SOLVER_DIR / "target" / "release" / "sat-solver")
        if resume:
            if not self.binary.is_file():
                raise SystemExit(f"resume: {self.binary} is missing")
        else:
            if not src.is_file():
                raise SystemExit(f"binary not found: {src} (run build.sh first)")
            shutil.copy2(src, self.binary)
            self.binary.chmod(0o755)
        self.binary_sha = sha256_of(self.binary)
        self.manifest_path = run_dir / "manifest.json"
        if resume:
            m = json.loads(self.manifest_path.read_text())
            if m.get("binary_sha256") != self.binary_sha:
                raise SystemExit(f"resume: bin/sat-solver sha {self.binary_sha[:16]} differs from the manifest")
            self.manifest = m
            self.manifest.setdefault("resumed", []).append(time.strftime("%Y-%m-%d %H:%M:%S"))
        else:
            self.manifest = {
                "name": run_dir.name, "created": time.strftime("%Y-%m-%d %H:%M:%S"),
                "suite": str(suite), "binary": str(src), "binary_sha256": self.binary_sha,
                "git_head": git_head(), "host": socket.gethostname(), "nproc": os.cpu_count(),
                "jobs_cap": self.proc_cap, "cores": self.cores, "mem_mb": self.mem_mb,
                "mem_total_mb": self.mem_total_mb, "epochs": self.epochs or None,
                "verify": self.verify, "oracle": getattr(args, "oracle", None) or [],
                "n_jobs": len(jobs), "argv": sys.argv, "proofs": "off",
            }
        self.write_manifest()
        for j in jobs:
            rec = run_dir / "cells" / f"{j.key}.json"
            if rec.is_file():
                try:
                    self.records[j.key] = json.loads(rec.read_text())
                except json.JSONDecodeError:
                    rec.unlink()
                    continue
                if self.records[j.key].get("contradiction"):
                    # a resumed pass keeps its failure: it can never end in DONE
                    self.contradiction.set()

    # paths ---------------------------------------------------------------
    def log_path(self, job: Job) -> Path:
        return self.dir / "logs" / f"{job.key}.log"

    def out_path(self, job: Job) -> Path:
        return self.dir / "out" / f"{job.key}.out"

    def err_path(self, job: Job) -> Path:
        return self.dir / "out" / f"{job.key}.err"

    def write_manifest(self) -> None:
        tmp = self.manifest_path.with_suffix(".json.tmp")
        tmp.write_text(json.dumps(self.manifest, indent=1, sort_keys=True) + "\n")
        os.replace(tmp, self.manifest_path)

    # one job ----------------------------------------------------------------
    def materialize(self, job: Job, core_list: str) -> Path:
        src = cnf_source(self.suite, job.stem)
        src = src.resolve() if src.is_symlink() else src
        dst = self.scratch / f"{job.key}.cnf"
        if src.suffix == ".xz":
            with open(dst, "wb") as out:
                subprocess.run(["taskset", "-c", core_list, "xz", "-dc", str(src)], stdout=out, check=True)
        elif src.suffix == ".gz":
            with open(dst, "wb") as out:
                subprocess.run(["taskset", "-c", core_list, "gzip", "-dc", str(src)], stdout=out, check=True)
        else:
            return src
        return dst

    def run_job(self, job: Job, cores: list[int]) -> None:
        core_list = ",".join(str(c) for c in cores)
        rec: dict = {"stem": job.stem, "tag": job.tag, "flavour": job.flavour, "seed": job.seed,
                     "limit_ticks": job.limit_ticks, "wall_s": job.wall_s, "procs": job.procs,
                     "cores": cores, "started": time.strftime("%Y-%m-%d %H:%M:%S"),
                     "log": str(self.log_path(job).relative_to(self.dir)),
                     "out": str(self.out_path(job).relative_to(self.dir)),
                     "note": "", "anomaly": None, "children": [], "agree": "n/a", "verify": "skip",
                     "oracle": "none"}
        cnf = None
        p = None
        harness_error = False
        try:
            cnf = self.materialize(job, core_list)
            env_job = job.solver_env(self)
            rec["env"] = env_job
            base_env = {k: v for k, v in os.environ.items() if not k.startswith("SAT_")}
            env = {**base_env, **env_job}
            # SAT_EXTRA_ARGS in a job's env are kissat options for the command
            # line (as run.sh and feature_ablation.py pass them), not
            # environment; the table's env column has no spaces, so several
            # options are separated by commas
            extra = [a for a in env.pop("SAT_EXTRA_ARGS", "").replace(",", " ").split() if a]
            argv = [str(self.binary), "-s", f"--seed={job.seed}", *extra, str(cnf)]
            rec["argv"] = argv
            inner = f"ulimit -v {self.mem_mb * 1024}; exec timeout -k {KILL_GRACE_S} {job.wall_s} " \
                    + " ".join(shlex.quote(a) for a in argv)
            cmd = ["taskset", "-c", core_list, "bash", "-c", inner]
            # remove stale outputs of an earlier attempt (a resume after a kill)
            for old in (self.out_path(job), self.err_path(job), self.log_path(job)):
                if old.exists():
                    old.unlink()
            for old in self.dir.joinpath("logs").glob(f"{job.key}.log.b*"):
                old.unlink()
            with open(self.out_path(job), "wb") as fo, open(self.err_path(job), "wb") as fe:
                # launch and register under the lock the shutdown takes: a
                # worker that was still decompressing when the stop came must
                # not start a solver after the shutdown's snapshot
                with self.lock:
                    if self.stop.is_set():
                        self.killed.add(job.key)
                        raise Aborted()
                    t0 = time.time()
                    p = subprocess.Popen(cmd, stdout=fo, stderr=fe, env=env, start_new_session=True)
                    self.live[job.key] = p
                try:
                    p.wait(timeout=job.wall_s + KILL_GRACE_S + 120)
                except subprocess.TimeoutExpired:
                    rec["note"] = "collector killed the group: timeout(1) did not end it"
                    self.kill_group(p)
                    p.wait()
                rec["wall_s"] = round(time.time() - t0, 3)
                rec["exit"] = p.returncode
            with self.lock:
                self.live.pop(job.key, None)
                killed = job.key in self.killed
            self.reap_group(p, rec)
            if not killed:      # a job the shutdown ended gets no record, so nothing to verify
                self.finish_record(job, rec, cnf, core_list)
        except Aborted:
            rec["note"] = "aborted before launch (shutdown)"
        except Exception as e:  # a harness bug must not pass as collected work
            harness_error = True
            print(f"  HARNESS ERROR on {job.key}: {e!r} (no record; a resume reruns it)", flush=True)
            with self.lock:
                self.harness_errors += 1
                self.killed.add(job.key)           # no record, like a killed job
                if p is not None:
                    self.live.pop(job.key, None)
        finally:
            if cnf is not None and cnf.parent == self.scratch:
                try:
                    cnf.unlink()
                except OSError:
                    pass
            rec["ended"] = time.strftime("%Y-%m-%d %H:%M:%S")
            with self.lock:
                killed = job.key in self.killed
                if not killed:
                    self.records[job.key] = rec
                    path = self.dir / "cells" / f"{job.key}.json"
                    tmp = path.with_suffix(".json.tmp")
                    tmp.write_text(json.dumps(rec, indent=1, sort_keys=True) + "\n")
                    os.replace(tmp, path)
                self.free_cores.extend(cores)
                self.mem_used_mb -= self.mem_charge(job)
                self.n_admitted -= 1
            if killed:
                print(f"  aborted {job.key} (no record; a resume reruns it)", flush=True)
            else:
                self.report(job, rec)

    def finish_record(self, job: Job, rec: dict, cnf: Path, core_list: str = "") -> None:
        res, wc_line = status_of(self.out_path(job))
        wc = parse_workclock(wc_line) if wc_line else {}
        rec["workclock"] = wc
        ls = log_summary(self.log_path(job)) if job.flavour != "off" else {}
        ft = ls.get("footer") or {}
        capped = rec["exit"] in (124, 137)
        # An `off` job writes no log, so nothing proves its output was complete
        # when the cap fired; with no children to keep the group alive, a cap
        # on it means the solver itself was still running, so it is a TIMEOUT.
        answered = (res in SOLVED and job.flavour != "off" and ft.get("reason") == "solve"
                    and ft.get("result") == {"SAT": "SATISFIABLE", "UNSAT": "UNSATISFIABLE"}[res])
        if capped and answered:
            # timeout(1) expired after the solver had answered and sealed its log:
            # a fork parent whose children kept the group alive. The answer
            # stands (and is checked below); the cap is an anomaly of the job.
            rec["anomaly"] = "wall_cap_after_answer"
        elif capped:
            # timeout(1) expired (137: it had to SIGKILL). Whatever the handler
            # printed, the wall limit ended this run: TIMEOUT for a wall-limited
            # job, an anomaly for a budgeted one (its result should have been
            # the budget's `s UNKNOWN`).
            res = "TIMEOUT"
            if job.limit_ticks is not None:
                rec["anomaly"] = "wall_cap"
        elif res == "NONE":
            res = f"NONE_rc{rec['exit']}"
        elif rec["exit"] not in (0, 10, 20):
            rec["note"] = (rec["note"] + f"; exit rc={rec['exit']} after the status line").strip("; ")
        rec["result"] = res
        abnormal_exit = rec["exit"] not in (0, 10, 20) and not capped
        # `s UNKNOWN` is only honest at the work budget (CLAUDE.md "Correctness
        # is absolute"): a budgeted run must have reached its limit, in the
        # units of the binary that ran it (the `c workclock` line), and a
        # wall-limited run has no budget to stop on at all
        rec["premature"] = False
        if res == "UNKNOWN" and rec["exit"] == 0:
            w_end = int(wc["work"]) if wc.get("work", "NA") != "NA" else None
            if job.limit_ticks is None or w_end is None or w_end < job.limit_ticks:
                rec["premature"] = True
                rec["note"] = (rec["note"] + f"; premature UNKNOWN: work {w_end} below the budget {job.limit_ticks}").strip("; ")
        note = death_note(self.err_path(job).read_text(errors="replace"))
        if note:
            rec["note"] = (rec["note"] + "; " + note).strip("; ")
        # no answer and no wall expiry: an out-of-memory abort under the ulimit
        # is an honest resource stop (priced as unsolved); anything else (a
        # panic, a usage error, a signal) is a crash and fails the gate
        rec["crash"] = False
        oom = bool(re.search(r"memory allocation of .* failed|out of memory", note or "", re.I))
        if (res.startswith("NONE") or abnormal_exit) and not oom:
            rec["crash"] = True
            rec["note"] = (rec["note"] + f"; crash: exit {rec['exit']}" + (" with an answer" if not res.startswith("NONE") else " without an answer")).strip("; ")
        rec["oom"] = oom
        # the log
        rec["log_rows"] = ls.get("rows")
        rec["log_footer"] = None if job.flavour == "off" else ls.get("footer") is not None
        rec["log_reason"] = ft.get("reason")
        rec["log_result"] = ft.get("result")
        rec["peak_rss_mb"] = round(ft["peak_rss_bytes"] / 2**20, 1) if ft.get("peak_rss_bytes") else None
        rec["log_work"] = ft.get("work")
        rec["log_conflicts"] = ft.get("conflicts")
        rec["static"] = bool(ft.get("static"))
        if ls.get("error"):
            rec["note"] = (rec["note"] + f"; log: {ls['error']}").strip("; ")
        if res in SOLVED and rec["log_footer"] is False:
            rec["note"] = (rec["note"] + "; log has no footer").strip("; ")
        # the model
        if self.verify and res == "SAT":
            rec["verify"] = self.verify_model(cnf, self.out_path(job), job.wall_s, core_list)
        elif res == "SAT":
            rec["verify"] = "off"
        # the oracle
        known = self.oracle.get(job.stem, {}).get("status")
        if known and res in SOLVED:
            rec["oracle"] = "agree" if known == res else f"CONTRADICTION(oracle={known})"
        elif known:
            rec["oracle"] = f"known={known}"
        # children (fork mode): every child's answer, model and log
        answers = {res} if res in SOLVED else set()
        for child_log in sorted(self.dir.joinpath("logs").glob(f"{job.key}.log.b*")):
            name = child_log.name
            if name.endswith(".out") or name.endswith(".err"):
                continue
            m = re.search(r"\.b(\d+)\.(\d+)$", name)
            if not m:
                continue
            cres, cwc = status_of(child_log.with_name(name + ".out"))
            cls = log_summary(child_log)
            cft = cls.get("footer") or {}
            hb = (cls.get("header") or {}).get("branch")
            hb = hb if isinstance(hb, dict) else {}
            child = {"branch": int(m.group(1)), "k": int(m.group(2)), "result": cres,
                     "log": str(child_log.relative_to(self.dir)), "rows": cls.get("rows"),
                     "footer": bool(cft), "work": cft.get("work"),
                     "conflicts": cft.get("conflicts"), "reason": cft.get("reason"),
                     "peak_rss_mb": round(cft["peak_rss_bytes"] / 2**20, 1) if cft.get("peak_rss_bytes") else None,
                     "decision": hb.get("decision"), "knob": hb.get("knob"), "entry": hb.get("entry"),
                     "verify": "skip", "anomaly": None}
            cnote = death_note(child_log.with_name(name + ".err").read_text(errors="replace")) if child_log.with_name(name + ".err").is_file() else ""
            c_oom = bool(re.search(r"memory allocation of .* failed|out of memory", cnote, re.I))
            child["note"] = cnote
            child["crash"] = False
            if not cft and capped:
                child["anomaly"] = "wall_cap"         # timeout -k SIGKILLed it before it could seal its log
            elif not cft:
                child["anomaly"] = "cut"              # no footer: the child's log was cut off
                child["crash"] = not c_oom            # an out-of-memory abort is an honest stop; anything else is not
            elif capped and cft.get("reason") == "signal":
                child["anomaly"] = "wall_cap"         # ended by the job's safety cap, not by its budget
            elif cft.get("reason") != "solve" and not c_oom:
                # a signal with no cap on the job: a panic or a fault in the child
                child["crash"] = True
            if cnote and not c_oom and cres == "NONE":
                child["crash"] = True
            child["oom"] = c_oom
            if child["crash"]:
                child["note"] = (f"child crash: exit reason {cft.get('reason') or 'no footer'}; " + cnote).strip("; ")
            completed = cft.get("reason") == "solve" and cft.get("result") == {"SAT": "SATISFIABLE", "UNSAT": "UNSATISFIABLE", "UNKNOWN": "UNKNOWN"}.get(cres)
            if cres == "SAT" and not completed:
                child["verify"] = "interrupted"       # the cap cut the model output: not an answer, not a bug
                child["anomaly"] = child["anomaly"] or "wall_cap"
            elif cres == "SAT":
                child["verify"] = self.verify_model(cnf, child_log.with_name(name + ".out"), job.wall_s, core_list) if self.verify else "off"
            child["premature"] = (cres == "UNKNOWN" and completed and job.limit_ticks is not None
                                  and cft.get("work") is not None and int(cft["work"]) < job.limit_ticks)
            if child["premature"]:
                child["note"] = f"premature UNKNOWN: work {cft.get('work')} below the budget {job.limit_ticks}"
            if cres in SOLVED and completed:
                answers.add(cres)
            if known and cres in SOLVED and cres != known:
                child["oracle"] = f"CONTRADICTION(oracle={known})"
            rec["children"].append(child)
        rec["children_expected"] = len(ft.get("branches") or []) if isinstance(ft.get("branches"), list) else None
        rec["children_missing"] = 0
        if job.flavour == "fork":
            fails = [c for c in rec["children"] if c["verify"] in BAD_VERIFY]
            rec["agree"] = "FAIL" if len(answers) > 1 or fails else "ok"
            # every child the parent's footer lists must have left a log; one
            # that died before opening it is a failure, not a missing sample
            if rec["children_expected"] is not None and rec["children_expected"] > len(rec["children"]):
                rec["children_missing"] = rec["children_expected"] - len(rec["children"])
                rec["note"] = (rec["note"] + f"; {rec['children_missing']} child log(s) missing of {rec['children_expected']} forked").strip("; ")
        bad = (rec["verify"] in BAD_VERIFY or rec["agree"] == "FAIL" or rec["premature"] or rec["crash"]
               or rec["children_missing"] > 0
               or rec["oracle"].startswith("CONTRADICTION")
               or any(c.get("oracle", "").startswith("CONTRADICTION") or c.get("premature") or c.get("crash")
                      for c in rec["children"]))
        rec["failed"] = bool(bad)
        if bad:
            rec["contradiction"] = True
            self.contradiction.set()
            if self.on_contradiction == "stop":
                self.drain.set()

    def verify_model(self, cnf: Path, out_path: Path, wall_s: int, core_list: str = "") -> str:
        """tools/verify_sat.py on the run's stdout, pinned to the job's own cores."""
        pin = ["taskset", "-c", core_list] if core_list else []
        try:
            p = subprocess.run([*pin, sys.executable, str(VERIFY_SAT), str(cnf), str(out_path)],
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
                               timeout=max(600, 2 * wall_s))
            return "ok" if p.returncode == 0 else "FAIL"
        except subprocess.TimeoutExpired:
            return "checker-timeout"
        except Exception:
            return "checker-error"

    # process groups -------------------------------------------------------
    @staticmethod
    def group_alive(p: subprocess.Popen) -> bool:
        try:
            os.killpg(p.pid, 0)
            return True
        except ProcessLookupError:
            return False
        except PermissionError:
            return True

    def kill_group(self, p: subprocess.Popen) -> None:
        """SIGTERM the job's group (the solver seals its log), then SIGKILL what is left."""
        try:
            os.killpg(p.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        deadline = time.time() + KILL_GRACE_S
        while time.time() < deadline and self.group_alive(p):
            time.sleep(0.2)
        if self.group_alive(p):
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def reap_group(self, p: subprocess.Popen, rec: dict) -> None:
        """After the leader exited: nothing of its group may survive (fork children)."""
        if self.group_alive(p):
            deadline = time.time() + 5
            while time.time() < deadline and self.group_alive(p):
                time.sleep(0.1)
        if self.group_alive(p):
            rec["note"] = (rec["note"] + "; orphaned children killed by the collector").strip("; ")
            self.kill_group(p)

    def shutdown(self) -> None:
        with self.lock:
            live = list(self.live.values())
            self.killed.update(self.live.keys())
        for p in live:
            try:
                os.killpg(p.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.time() + KILL_GRACE_S
        while time.time() < deadline and any(self.group_alive(p) for p in live):
            time.sleep(0.2)
        for p in live:
            if self.group_alive(p):
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        time.sleep(0.5)
        left = [p.pid for p in live if self.group_alive(p)]
        if left:
            print(f"[rl_collect] WARNING: process groups still alive after SIGKILL: {left}", flush=True)
        else:
            print(f"[rl_collect] shutdown: {len(live)} live job group(s) ended, nothing left", flush=True)

    # admission -----------------------------------------------------------
    def mem_charge(self, job: Job) -> float:
        if self.mem_total_mb <= 0:
            return 0.0
        per = job.peak_rss_mb * 1.5 if job.peak_rss_mb else float(self.mem_mb)
        return job.procs * min(per, float(self.mem_mb))

    def report(self, job: Job, rec: dict) -> None:
        done = len(self.records)
        wc = rec.get("workclock") or {}
        flags = []
        if rec.get("verify") not in ("skip", "ok", "off"):
            flags.append(f"verify={rec['verify']}")
        if rec.get("oracle", "none").startswith("CONTRADICTION"):
            flags.append(rec["oracle"])
        if rec.get("agree") == "FAIL":
            flags.append("SIBLINGS DISAGREE")
        if rec.get("anomaly"):
            flags.append(f"anomaly={rec['anomaly']}")
        if rec.get("premature") or any(c.get("premature") for c in rec.get("children") or []):
            flags.append("PREMATURE UNKNOWN")
        if rec.get("crash") or any(c.get("crash") for c in rec.get("children") or []):
            flags.append("CRASH")
        if rec.get("log_footer", True) is False:
            flags.append("no-footer")
        kids = f" children={len(rec['children'])}" if rec.get("children") else ""
        print(f"  [{done}/{len(self.jobs)}] {job.key[:48]:<48} {rec.get('result', '?'):<9} "
              f"{rec.get('wall_s', 0):8.1f}s work={wc.get('work', 'NA')} rows={rec.get('log_rows')}"
              f"{kids} {' '.join(flags)}", flush=True)
        if rec.get("contradiction"):
            print(f"    *** CORRECTNESS ALERT: {job.key}: {rec.get('verify')} {rec.get('oracle')} "
                  f"agree={rec.get('agree')} — a solver bug; {'stopping the pass' if self.on_contradiction == 'stop' else 'continuing (--on-contradiction continue)'}",
                  flush=True)
        if done % 10 == 0 or done == len(self.jobs):
            self.write_results()

    def write_results(self) -> None:
        with self.write_lock:
            self._write_results()

    def _write_results(self) -> None:
        with self.lock:
            recs = [self.records[j.key] for j in self.jobs if j.key in self.records]
        tmp = self.dir / "results.tsv.tmp"
        with open(tmp, "w") as f:
            f.write("\t".join(TSV_COLUMNS) + "\n")
            for r in sorted(recs, key=lambda r: (r["tag"], r["stem"])):
                wc = r.get("workclock") or {}
                row = {
                    "tag": r["tag"], "stem": r["stem"], "flavour": r["flavour"], "seed": r["seed"],
                    "result": r.get("result", "?"), "wall_s": f"{r.get('wall_s', 0):.3f}", "exit": r.get("exit"),
                    "limit_ticks": "" if r.get("limit_ticks") is None else r["limit_ticks"],
                    "ticks": wc.get("ticks", "NA"), "eliminate_resolutions": wc.get("eliminate_resolutions", "NA"),
                    "work": wc.get("work", "NA"), "conflicts": wc.get("conflicts", "NA"),
                    "rows": "" if r.get("log_rows") is None else r.get("log_rows"),
                    "footer": "" if r.get("log_footer") is None else int(bool(r.get("log_footer"))),
                    "peak_rss_mb": "" if r.get("peak_rss_mb") is None else r["peak_rss_mb"],
                    "verify": r.get("verify"), "oracle": r.get("oracle"),
                    "children": len(r.get("children") or []), "agree": r.get("agree"),
                    "anomaly": r.get("anomaly") or "", "failed": int(bool(r.get("failed"))),
                    "note": (r.get("note") or "").replace("\t", " "),
                }
                f.write("\t".join(str(row[c]) for c in TSV_COLUMNS) + "\n")
        os.replace(tmp, self.dir / "results.tsv")

    # the loop ---------------------------------------------------------------
    def execute(self) -> int:
        pending = [j for j in self.jobs if j.key not in self.records]
        print(f"[rl_collect] {self.dir}: {len(self.jobs)} jobs, {len(self.records)} done already, "
              f"{len(pending)} to run; cap {self.proc_cap} processes on cores {self.cores[0]}..{self.cores[-1]}; "
              f"binary {self.binary_sha[:16]}", flush=True)
        self.manifest["started"] = time.strftime("%Y-%m-%d %H:%M:%S")
        self.write_manifest()

        def on_signal(signum, _frame):
            print(f"[rl_collect] signal {signum}: stopping", flush=True)
            self.stop.set()

        signal.signal(signal.SIGINT, on_signal)
        signal.signal(signal.SIGTERM, on_signal)
        threads: list[threading.Thread] = []
        shut = False
        try:
            while pending or any(t.is_alive() for t in threads):
                if self.stop.is_set():
                    if not shut:
                        self.shutdown()
                        shut = True
                    pending.clear()
                    time.sleep(0.2)
                    continue
                if self.drain.is_set() and pending:
                    print(f"[rl_collect] correctness failure: {len(pending)} pending job(s) dropped, "
                          f"{len(self.live)} live job(s) finish", flush=True)
                    pending.clear()
                started = False
                with self.lock:
                    for j in list(pending):
                        if j.procs > len(self.free_cores):
                            continue
                        # the memory budget counts every admitted job, launched or
                        # still decompressing; a job is admitted over budget only
                        # when nothing else is admitted (it must run at some point)
                        if (self.mem_total_mb and self.mem_used_mb + self.mem_charge(j) > self.mem_total_mb
                                and self.n_admitted > 0):
                            continue
                        cores = [self.free_cores.pop(0) for _ in range(j.procs)]
                        self.mem_used_mb += self.mem_charge(j)
                        self.n_admitted += 1
                        pending.remove(j)
                        t = threading.Thread(target=self.run_job, args=(j, cores), daemon=True)
                        t.start()
                        threads.append(t)
                        started = True
                        break
                if not started:
                    time.sleep(0.5)
            for t in threads:
                t.join()
        finally:
            self.write_results()
            self.manifest["ended"] = time.strftime("%Y-%m-%d %H:%M:%S")
            self.manifest["n_done"] = len(self.records)
            self.manifest["contradiction"] = self.contradiction.is_set()
            self.write_manifest()
            shutil.rmtree(self.scratch, ignore_errors=True)
        n_done = len(self.records)
        if self.contradiction.is_set():
            print(f"[rl_collect] *** CORRECTNESS FAILURE in {self.dir}: see results.tsv (verify/oracle/agree)", flush=True)
            return 3
        if self.stop.is_set():
            print(f"[rl_collect] stopped with {n_done}/{len(self.jobs)} done; resume with: "
                  f"python3 tools/rl_collect.py resume {self.dir}", flush=True)
            return 130
        if self.harness_errors:
            print(f"[rl_collect] {self.harness_errors} job(s) hit a harness error and have no record; "
                  f"fix the cause and resume: python3 tools/rl_collect.py resume {self.dir}", flush=True)
            return 2
        (self.dir / "DONE").write_text(f"{n_done} jobs\n")
        print(f"[rl_collect] DONE -> {self.dir}", flush=True)
        summarize(self.dir)
        return 0


# ---------------------------------------------------------------------------
# commands
# ---------------------------------------------------------------------------

def summarize(run_dir: Path) -> None:
    tsv = run_dir / "results.tsv"
    if not tsv.is_file():
        print("no results.tsv yet")
        return
    with open(tsv) as f:
        rows = list(csv.DictReader(f, dialect="excel-tab"))
    n = len(rows)
    counts: dict[str, int] = {}
    for r in rows:
        counts[r["result"]] = counts.get(r["result"], 0) + 1
    bad = [r for r in rows if r.get("failed", "0") == "1" or r["verify"] not in ("ok", "skip", "off")
           or r["oracle"].startswith("CONTRADICTION") or r["agree"] == "FAIL"]
    nofoot = [r for r in rows if r["footer"] not in ("1", "")]      # "" = an off job, which writes no log
    anomalies = [r for r in rows if r["anomaly"]]
    manifest = json.loads((run_dir / "manifest.json").read_text()) if (run_dir / "manifest.json").is_file() else {}
    total = manifest.get("n_jobs", n)
    print(f"{run_dir}: {n}/{total} jobs done; " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    print(f"  correctness failures: {len(bad)}; logs without footer: {len(nofoot)}; anomalies: {len(anomalies)}; "
          f"DONE: {(run_dir / 'DONE').is_file()}")
    for r in bad[:10]:
        print(f"  FAIL {r['stem']}.{r['tag']}: verify={r['verify']} oracle={r['oracle']} agree={r['agree']}")
    for r in nofoot[:5]:
        print(f"  no footer: {r['stem']}.{r['tag']} result={r['result']} note={r['note']}")


def order_jobs(jobs: list[Job], oracle: dict[str, dict], how: str) -> list[Job]:
    if how == "longest":
        def t(j: Job) -> float:
            rec = oracle.get(j.stem)
            return -(rec["time_s"] if rec and rec.get("time_s") is not None else float(j.wall_s))
        return sorted(jobs, key=lambda j: (t(j), j.key))
    return sorted(jobs, key=lambda j: j.key)


def select_cells(stems: list[str], args) -> list[str]:
    sel = getattr(args, "cells", None)
    if sel:
        p = Path(sel)
        if p.is_file():
            wanted = [ln.strip() for ln in p.read_text().splitlines() if ln.strip() and not ln.startswith("#")]
        else:
            wanted = [s.strip() for s in sel.split(",") if s.strip()]
        missing = [w for w in wanted if w not in stems]
        if missing:
            raise SystemExit(f"--cells: not in the suite: {missing[:5]}")
        stems = [s for s in stems if s in set(wanted)]
    if getattr(args, "limit", 0):
        stems = stems[:args.limit]
    return stems


def add_common(ap: argparse.ArgumentParser) -> None:
    ap.add_argument("--name", required=True, help="run directory log/rl-<name>-<timestamp>")
    ap.add_argument("--jobs", type=int, default=PROC_CAP, help=f"process cap (default {PROC_CAP}, the host rule)")
    ap.add_argument("--mem-mb", type=int, default=16000, help="ulimit -v per process in MB (default 16000)")
    ap.add_argument("--mem-total-mb", type=int, default=0,
                    help="memory admission budget for the pass (0 = off): a job charges procs x 1.5 x its peak_rss_mb (or --mem-mb)")
    ap.add_argument("--binary", default="", help="solver binary to freeze (default solver/13-kissat-rs/target/release/sat-solver)")
    ap.add_argument("--epochs", default="", help="SAT_POLICY_EPOCH_TICKS for every job (X_o[,X_d])")
    ap.add_argument("--oracle", action="append", default=[], help="results.csv/.tsv with known statuses (repeatable)")
    ap.add_argument("--no-verify", action="store_true", help="skip the SAT model check")
    ap.add_argument("--on-contradiction", choices=("stop", "continue"), default="stop")
    ap.add_argument("--order", choices=("longest", "name"), default="longest",
                    help="job order: longest known time first (oracle times; default) or by name")
    ap.add_argument("--scratch", default="", help="root for the pass's scratch dir <root>/rl-collect-<run>-<pid> (default /tmp)")
    ap.add_argument("--dry-run", action="store_true", help="write the run dir and the job table, run nothing")
    ap.add_argument("--cores", default="", help="pin to these cores instead of the socket-balanced order (tests next to a sweep)")


def cmd_stock(args) -> int:
    suite = resolve_suite(args.suite)
    stems = select_cells(suite_stems(suite), args)
    jobs = stock_jobs(stems, args.seed, args.timeout)
    return launch(args, suite, jobs)


def cmd_table(args) -> int:
    suite = resolve_suite(args.suite)
    jobs = read_jobs(Path(args.table))
    stems = set(suite_stems(suite))
    unknown = [j.stem for j in jobs if j.stem not in stems]
    if unknown:
        raise SystemExit(f"job table: stems not in {suite}: {unknown[:5]}")
    if getattr(args, "cells", None) or getattr(args, "limit", 0):
        keep = set(select_cells(sorted({j.stem for j in jobs}), args))
        jobs = [j for j in jobs if j.stem in keep]
    return launch(args, suite, jobs)


def launch(args, suite: Path, jobs: list[Job]) -> int:
    if args.jobs > PROC_CAP:
        raise SystemExit(f"--jobs {args.jobs} exceeds the {PROC_CAP}-process cap (CLAUDE.md)")
    if any(j.procs > args.jobs for j in jobs):
        raise SystemExit("a job needs more processes than --jobs allows")
    live = live_solvers()
    if live and not args.dry_run:
        print(f"[rl_collect] WARNING: {len(live)} solver/bench process(es) already running on this host:", flush=True)
        for ln in live[:8]:
            print("   ", ln, flush=True)
        if not getattr(args, "force", False):
            raise SystemExit("refusing to start next to a live sweep (pass --force to override)")
    ts = time.strftime("%Y-%m-%d-%H-%M-%S")
    run_dir = ROOT / "log" / f"rl-{args.name}-{ts}"
    run_dir.mkdir(parents=True)
    oracle = oracle_load(args.oracle)
    jobs = order_jobs(jobs, oracle, args.order)
    write_jobs(run_dir / "jobs.tsv", jobs)
    run = Run(run_dir, suite, args, jobs, resume=False)
    run.manifest["suite_name"] = suite.name
    run.manifest["n_cells"] = len({j.stem for j in jobs})
    run.manifest["flavours"] = sorted({j.flavour for j in jobs})
    run.write_manifest()
    if args.dry_run:
        print(f"[rl_collect] dry run: {len(jobs)} jobs written to {run_dir / 'jobs.tsv'}")
        return 0
    print(f"[rl_collect] pid {os.getpid()} -> {run_dir}", flush=True)
    return run.execute()


def cmd_resume(args) -> int:
    run_dir = Path(args.run_dir).resolve()
    manifest = json.loads((run_dir / "manifest.json").read_text())
    suite = Path(manifest["suite"])
    jobs = read_jobs(run_dir / "jobs.tsv")
    ns = argparse.Namespace(**{k: manifest.get(k) for k in ("mem_mb", "mem_total_mb", "epochs", "oracle")})
    ns.jobs = args.jobs or manifest["jobs_cap"]
    ns.no_verify = not manifest.get("verify", True)
    ns.on_contradiction = args.on_contradiction
    ns.scratch = ""
    ns.binary = ""
    # the saved core allocation (disjoint from a concurrent arm's) unless overridden
    ns.cores = args.cores or ",".join(str(c) for c in (manifest.get("cores") or []))
    if ns.jobs > PROC_CAP:
        raise SystemExit(f"--jobs {ns.jobs} exceeds the {PROC_CAP}-process cap")
    wide = [j.key for j in jobs if j.procs > ns.jobs]
    if wide:
        raise SystemExit(f"--jobs {ns.jobs} is below the {max(j.procs for j in jobs)} processes of {len(wide)} job(s), "
                         f"e.g. {wide[0]}; they could never start")
    live = live_solvers()
    if live and not args.force:
        for ln in live[:8]:
            print("   ", ln, flush=True)
        raise SystemExit("refusing to resume next to a live sweep (pass --force to override)")
    run = Run(run_dir, suite, ns, jobs, resume=True)
    print(f"[rl_collect] pid {os.getpid()} resumes {run_dir}", flush=True)
    return run.execute()


def live_solvers() -> list[str]:
    """Command lines of solver or sweep processes on the host, other than this one.

    Looks at what each process IS (its first token after taskset/timeout/
    nohup/setsid wrappers), not at what its command line mentions, so a
    shell that merely quotes this tool's name is not a hit.
    """
    try:
        out = subprocess.run(["ps", "-eo", "pid,args"], text=True, stdout=subprocess.PIPE).stdout
    except Exception:
        return []
    me = str(os.getpid())
    wrappers = {"taskset", "timeout", "nohup", "setsid", "env", "nice"}
    hits = []
    for ln in out.splitlines()[1:]:
        pid, _, cmd = ln.strip().partition(" ")
        if pid == me or " -c " in cmd[:12] or "bash -c" in cmd or "sh -c" in cmd:
            continue
        toks = cmd.split()
        i = 0
        while i < len(toks) and (Path(toks[i]).name in wrappers or (i > 0 and toks[i - 1] in ("-c", "-k", "-s") and Path(toks[i - 2]).name in wrappers) or re.fullmatch(r"[0-9,-]+", toks[i]) and i > 0):
            i += 1
        if i >= len(toks):
            continue
        exe = Path(toks[i]).name
        arg = Path(toks[i + 1]).name if i + 1 < len(toks) else ""
        if exe in ("sat-solver", "kissat"):
            hits.append(f"{pid} {cmd[:120]}")
        elif exe.startswith("python") and arg in ("feature_ablation.py", "rl_collect.py"):
            if arg == "rl_collect.py" and "status" in toks[i + 2:i + 3]:
                continue
            hits.append(f"{pid} {cmd[:120]}")
        elif exe in ("bash", "sh") and arg in ("run_kissat_full.sh", "run_kissat_medium.sh", "run_bench_reference.sh", "bench_reference.sh"):
            hits.append(f"{pid} {cmd[:120]}")
    return hits


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("stock", help="stock traces: one logged stock run per cell, wall-limited")
    s.add_argument("--suite", required=True)
    s.add_argument("--timeout", type=int, default=1800)
    s.add_argument("--seed", type=int, default=0)
    s.add_argument("--cells", default="", help="comma list or a file of stems to run (default all)")
    s.add_argument("--limit", type=int, default=0, help="first N cells only (tests)")
    s.add_argument("--force", action="store_true", help="start even if solver processes are live")
    add_common(s)
    s.set_defaults(fn=cmd_stock)

    t = sub.add_parser("table", help="run a job table (round 0: budgets, forks, random runs)")
    t.add_argument("table")
    t.add_argument("--suite", required=True)
    t.add_argument("--cells", default="")
    t.add_argument("--limit", type=int, default=0)
    t.add_argument("--force", action="store_true")
    add_common(t)
    t.set_defaults(fn=cmd_table)

    r = sub.add_parser("resume", help="continue a stopped pass: jobs with a record are skipped")
    r.add_argument("run_dir")
    r.add_argument("--jobs", type=int, default=0)
    r.add_argument("--cores", default="", help="override the saved core allocation")
    r.add_argument("--on-contradiction", choices=("stop", "continue"), default="stop")
    r.add_argument("--force", action="store_true")
    r.set_defaults(fn=cmd_resume)

    st = sub.add_parser("status", help="progress and correctness summary of a pass")
    st.add_argument("run_dir")
    st.set_defaults(fn=lambda a: (summarize(Path(a.run_dir)), 0)[1])

    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
