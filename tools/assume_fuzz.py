#!/usr/bin/env python3
"""Fuzz harness and oracle for solving under assumptions (epic
SAT-playground-bp3, plan/assumptions-plan.md section 11).

It makes small formulas and lists of questions ("is the formula satisfiable
with these literals true?"), runs the solver, and checks every answer
without trusting the solver's own code:

  the right answer   by truth table when the variables are few, else from
                     solvers the branch cannot touch (the C kissat, CaDiCaL,
                     and a frozen build of main), each run fresh with the
                     assumptions as unit clauses; they must agree
  a model            satisfies every clause and every assumption
  a failed core      is a subset of the question's literals, and the formula
                     plus the core is unsatisfiable (by the same oracle)
  a proof            passes tools/proof_gate.py with the core as units and
                     the DRAT file cut at the answer's `p` line

Modes:
  selftest   the harness against hand-made answers, right and wrong: every
             wrong one must be caught (a wrong status, a wrong model, a
             model that breaks an assumption, a core that is satisfiable
             with the formula, a core that is not a subset, a broken proof)
  units      validation on a solver without assumptions: one fresh run per
             question, the assumptions written as unit clauses
  run        the solver's test mode (--inccnf=1): one run per formula, every
             question answered by the same solver
  split      the control split in two stages (--ctlsplit=1 --ctlsplitassume=1)
             on a plain formula with a random control set and random limits:
             the answer against the oracle, the model, the proof through the
             gate

usage:
  assume_fuzz.py selftest
  assume_fuzz.py units --solver BIN [--count 200] [--seed 1] [--proof] [--cross]
  assume_fuzz.py run   --solver BIN [--count 200] [--seed 1] [--proof] [--cross]
                       [--options '--assumeconflicts=1'] [--limited]
  assume_fuzz.py split --solver BIN [--count 200] [--seed 1] [--proof]
"""

import argparse
import os
import random
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import proof_gate  # noqa: E402

KISSAT = os.path.join(ROOT, "benchmarks", "reference-solvers", "kissat-latest", "build", "kissat")
CADICAL = os.path.join(ROOT, "benchmarks", "reference-solvers", "cadical", "build", "cadical")
TABLE_MAX = 20  # variables up to which the truth table is the oracle


# ---------------------------------------------------------------------------
# Formulas
# ---------------------------------------------------------------------------

class Formula:
    def __init__(self, nvars, clauses, special, kind):
        self.nvars = nvars          # the largest variable, used or not
        self.clauses = clauses      # lists of non-zero ints
        self.special = special      # variables worth asking about
        self.kind = kind

    def dimacs(self, units=()):
        lines = [f"p cnf {self.nvars} {len(self.clauses) + len(units)}"]
        lines += [" ".join(map(str, c)) + " 0" for c in self.clauses]
        lines += [f"{u} 0" for u in units]
        return "\n".join(lines) + "\n"

    def inccnf(self, questions):
        lines = ["p inccnf"]
        lines += [" ".join(map(str, c)) + " 0" for c in self.clauses]
        lines += ["a " + " ".join(map(str, q)) + (" 0" if q else "0") for q in questions]
        return "\n".join(lines) + "\n"


def random_clause(rng, n, k):
    vs = rng.sample(range(1, n + 1), min(k, n))
    return [v if rng.random() < 0.5 else -v for v in vs]


def gen_formula(rng, hard=0.0):
    """A small formula with planted features that stress a freeze: gate
    outputs, equivalent variables, pure variables, variables fixed by a unit,
    and variables the formula never uses."""
    small = rng.random() < 0.6
    n = rng.randint(5, 14) if small else rng.randint(21, 48)
    kind = rng.choice(["3sat", "3sat", "mixed", "gates", "equiv", "easy"])
    if hard and rng.random() < hard:
        # big enough for hundreds of conflicts, so restarts, clause
        # reduction and inprocessing happen inside a solve
        n = rng.randint(60, 130)
        kind = "hard"
    clauses = []
    special = []
    nxt = n + 1  # the next new variable

    if kind == "easy":
        ratio = rng.uniform(1.5, 3.2)
    elif kind == "hard":
        ratio = rng.uniform(4.1, 4.4)
    else:
        ratio = rng.uniform(3.4, 4.8)
    for _ in range(int(n * ratio)):
        k = 3 if kind != "mixed" else rng.choice([2, 2, 3, 3, 3, 4, 5])
        clauses.append(random_clause(rng, n, k))

    if kind == "gates" or rng.random() < 0.35:
        for _ in range(rng.randint(1, 4)):
            g = nxt
            nxt += 1
            a, b = (rng.choice([1, -1]) * rng.randint(1, g - 1) for _ in range(2))
            shape = rng.choice(["and", "xor", "ite"])
            if abs(a) == abs(b):
                shape = "and"
            if shape == "and":       # g <-> a and b
                clauses += [[-g, a], [-g, b], [g, -a, -b]]
            elif shape == "xor":     # g <-> a xor b
                clauses += [[-g, a, b], [-g, -a, -b], [g, -a, b], [g, a, -b]]
            else:                    # g <-> if s then a else b
                s = rng.choice([1, -1]) * rng.randint(1, g - 1)
                clauses += [[-g, -s, a], [-g, s, b], [g, -s, -a], [g, s, -b]]
            special.append(g)
            if rng.random() < 0.5:   # use the gate somewhere
                clauses.append([rng.choice([g, -g])] + random_clause(rng, n, 2))
    if kind == "equiv" or rng.random() < 0.3:
        for _ in range(rng.randint(1, 3)):
            x = rng.randint(1, nxt - 1)
            y = nxt
            nxt += 1
            sign = rng.choice([1, -1])  # y <-> sign * x
            clauses += [[-y, sign * x], [y, -sign * x]]
            special += [x, y]
    if rng.random() < 0.3:           # a pure variable: one polarity only
        p = nxt
        nxt += 1
        for _ in range(rng.randint(1, 3)):
            clauses.append([p] + random_clause(rng, n, 2))
        special.append(p)
    if rng.random() < 0.3:           # a variable fixed by a unit clause
        u = rng.randint(1, nxt - 1)
        clauses.append([rng.choice([u, -u])])
        special.append(u)
    if rng.random() < 0.3:           # a variable the formula never uses
        special.append(nxt)
        nxt += 1
    rng.shuffle(clauses)
    return Formula(nxt - 1, clauses, special, kind)


def gen_questions(rng, f, plain=0.08):
    qs = []
    pool = list(range(1, f.nvars + 1)) + f.special * 4
    for _ in range(rng.randint(3, 9)):
        if rng.random() < plain:
            qs.append([])
            continue
        k = rng.choice([1, 1, 2, 2, 3, 4, 6, max(1, f.nvars // 3)])
        q = [rng.choice([1, -1]) * rng.choice(pool) for _ in range(k)]
        if rng.random() < 0.15:      # the same literal twice
            q.insert(rng.randrange(len(q) + 1), rng.choice(q))
        if rng.random() < 0.10:      # a literal with its negation
            q.insert(rng.randrange(len(q) + 1), -rng.choice(q))
        qs.append(q)
    if rng.random() < 0.3:           # a question asked again
        qs.append(list(rng.choice(qs)))
    return qs


# ---------------------------------------------------------------------------
# The oracle
# ---------------------------------------------------------------------------

class OracleFailure(Exception):
    pass


class Table:
    """Truth table as one big integer: bit i is assignment i (bit v-1 of i
    is variable v). Exact, and too simple to be wrong in an interesting way."""

    def __init__(self, f):
        n = f.nvars
        self.all = (1 << (1 << n)) - 1
        self.true = [0] * (n + 1)
        for v in range(1, n + 1):
            half = 1 << (v - 1)
            block = ((1 << half) - 1) << half          # 2^(v-1) zeros, then ones
            self.true[v] = block * (self.all // ((1 << (2 * half)) - 1))
        models = self.all
        for c in f.clauses:
            s = 0
            for l in c:
                s |= self.lit(l)
            models &= s
        self.models = models

    def lit(self, l):
        return self.true[l] if l > 0 else self.all ^ self.true[-l]

    def sat(self, lits):
        m = self.models
        for l in lits:
            m &= self.lit(l)
        return m != 0


def run_exit(cmd, timeout=120):
    p = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=timeout)
    return p.returncode


class Oracle:
    def __init__(self, f, workdir, frozen=None, cross=False):
        self.f = f
        self.workdir = workdir
        self.table = Table(f) if f.nvars <= TABLE_MAX else None
        self.cross = cross
        self.solvers = [("kissat", [KISSAT, "-q", "-n"]), ("cadical", [CADICAL, "-q", "-n"])]
        if frozen:
            self.solvers.append(("frozen-main", [frozen, "-q", "-n"]))
        self.memo = {}

    def external(self, lits):
        path = os.path.join(self.workdir, "oracle.cnf")
        with open(path, "w") as fh:
            fh.write(self.f.dimacs(lits))
        codes = {name: run_exit(cmd + [path]) for name, cmd in self.solvers}
        if set(codes.values()) not in ({10}, {20}):
            raise OracleFailure(f"the oracle solvers do not agree under {lits}: {codes}")
        return next(iter(codes.values())) == 10

    def sat(self, lits):
        key = tuple(sorted(set(lits)))
        if key not in self.memo:
            if self.table is not None:
                res = self.table.sat(key)
                if self.cross and self.external(key) != res:
                    raise OracleFailure(f"truth table and oracle solvers disagree under {list(key)}")
            else:
                res = self.external(key)
            self.memo[key] = res
        return self.memo[key]


# ---------------------------------------------------------------------------
# Checking answers
# ---------------------------------------------------------------------------

def parse_inccnf_output(text):
    """The answer blocks of the test mode: one dict per `q` line."""
    answers = []
    cur = None
    for line in text.splitlines():
        if line.startswith("q "):
            tok = line.split()
            cur = {"number": int(tok[1]), "status": tok[2], "model": [], "core": None, "proof_bytes": None,
                   "model_done": False}
            answers.append(cur)
        elif cur is None:
            continue
        elif line.startswith("v "):
            for t in line.split()[1:]:
                if t == "0":
                    cur["model_done"] = True
                else:
                    cur["model"].append(int(t))
        elif line.startswith("f "):
            tok = line.split()[1:]
            if not tok or tok[-1] != "0":
                cur["core"] = "malformed"
            else:
                cur["core"] = [int(t) for t in tok[:-1]]
        elif line.startswith("p "):
            cur["proof_bytes"] = int(line.split()[1])
    return answers


def check_answer(f, oracle, question, ans, limited=False, cnf_path=None, proof_path=None):
    """Reasons why this answer is wrong; empty when it is right."""
    bad = []
    status = ans["status"]
    if status == "UNKNOWN":
        if not limited:
            bad.append("UNKNOWN without a limit")
        return bad
    if status not in ("SATISFIABLE", "UNSATISFIABLE"):
        return [f"status {status!r}"]
    truth = oracle.sat(question)
    if (status == "SATISFIABLE") != truth:
        return [f"answered {status} but the oracle says {'SATISFIABLE' if truth else 'UNSATISFIABLE'}"]
    if status == "SATISFIABLE":
        if not ans.get("model_done", True):
            bad.append("the model line has no closing 0")
        value = {}
        for l in ans["model"]:
            if value.get(abs(l), l > 0) != (l > 0):
                bad.append(f"the model gives variable {abs(l)} both values")
            value[abs(l)] = l > 0
        for c in f.clauses:
            if not any(value.get(abs(l)) == (l > 0) for l in c):
                bad.append(f"the model does not satisfy the clause {c}")
                break
        for l in question:
            if value.get(abs(l)) != (l > 0):
                bad.append(f"the model breaks the assumption {l}")
                break
        return bad
    core = ans["core"]
    if core is None or core == "malformed":
        return ["UNSATISFIABLE without a well-formed core line"]
    left = list(question)
    for l in core:
        if l in left:
            left.remove(l)
        else:
            bad.append(f"the core literal {l} is not among the assumptions")
    if bad:
        return bad
    if oracle.sat(core):
        return [f"the core {core} is satisfiable together with the formula"]
    if proof_path is not None:
        verdict = proof_gate.check_unsat(cnf_path, proof_path, timeout=300, units=core,
                                         proof_bytes=ans["proof_bytes"])
        if verdict != "ok":
            bad.append(f"the proof does not check under the core {core}: {verdict}")
    return bad


# ---------------------------------------------------------------------------
# Running a solver
# ---------------------------------------------------------------------------

def solve_units(solver, f, question, workdir, want_proof, options):
    """One fresh run, the assumptions as unit clauses. Returns an answer as
    the test mode would give it, without a core."""
    cnf = os.path.join(workdir, "units.cnf")
    with open(cnf, "w") as fh:
        fh.write(f.dimacs(question))
    proof = os.path.join(workdir, "units.proof")
    for p in (proof, proof + ".pbp", proof + ".derived.cnf"):
        if os.path.exists(p):
            os.remove(p)
    cmd = [solver] + options + [cnf] + ([proof] if want_proof else [])
    p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=300)
    status = "none"
    model = []
    done = False
    for line in p.stdout.splitlines():
        if line.startswith("s "):
            status = line[2:].strip()
        elif line.startswith("v "):
            for t in line.split()[1:]:
                if t == "0":
                    done = True
                else:
                    model.append(int(t))
    ans = {"status": status, "model": model, "model_done": done, "core": None, "proof_bytes": None}
    return ans, cnf, proof


def check_units_case(solver, f, questions, workdir, frozen, want_proof, cross, options):
    oracle = Oracle(f, workdir, frozen, cross)
    bad = []
    for i, q in enumerate(questions):
        ans, cnf, proof = solve_units(solver, f, q, workdir, want_proof, options)
        status = ans["status"]
        truth = oracle.sat(q)
        if status not in ("SATISFIABLE", "UNSATISFIABLE"):
            bad.append(f"question {i + 1} {q}: status {status!r}")
            continue
        if (status == "SATISFIABLE") != truth:
            bad.append(f"question {i + 1} {q}: answered {status}, the oracle says otherwise")
            continue
        if status == "SATISFIABLE":
            for why in check_answer(f, oracle, q, ans):
                bad.append(f"question {i + 1} {q}: {why}")
        elif want_proof:
            verdict = proof_gate.check_unsat(cnf, proof, timeout=300)
            if verdict != "ok":
                bad.append(f"question {i + 1} {q}: the proof does not check: {verdict}")
    return bad


# Named option sets for the `run` mode.  The formulas are small, so with the
# solver's own intervals most passes would never fire inside a solve; "often"
# makes every interval tiny.  A set marked limited gives each question a
# conflict limit, so UNKNOWN is an allowed answer there.
OFTEN = ("--eliminateinit=0 --eliminateint=10 --probeinit=0 --probeint=2 --reduceinit=2 --reduceint=2 "
         "--rephaseinit=10 --rephaseint=10 --reorderinit=0 --reorderint=1 --modeinit=10 --modeint=10 "
         "--compactlim=0 --randecinit=0 --randecint=0")
PRESETS = {
    "default": ("", False),
    "often": (OFTEN, False),
    "often-stable": (OFTEN + " --stable=2", False),
    "often-focused": (OFTEN + " --stable=0", False),
    "often-nochrono": (OFTEN + " --chrono=0", False),
    "often-noreuse": (OFTEN + " --restartreusetrail=0", False),
    "often-tumble0": (OFTEN + " --tumble=0", False),
    "often-fastel": (OFTEN + " --fastel=1", False),
    "often-noluck": (OFTEN + " --lucky=0", False),
    "limit0": ("--assumeconflicts=0", True),
    "limit1": (OFTEN + " --assumeconflicts=1", True),
    "limit5": (OFTEN + " --assumeconflicts=5", True),
    "ticks1": ("--assumeticks=1", True),
}

STAT_LINE = __import__("re").compile(r"^c ([a-z_0-9]+):\s+(\d+)")
COUNTED = ("conflicts", "decisions", "restarts", "reductions", "rephased", "switched", "probings",
           "eliminations", "eliminated", "substituted", "vivifications", "searches", "units")


def check_run_case(solver, f, questions, workdir, frozen, want_proof, cross, options, limited, stats=None):
    oracle = Oracle(f, workdir, frozen, cross)
    icnf = os.path.join(workdir, "case.icnf")
    with open(icnf, "w") as fh:
        fh.write(f.inccnf(questions))
    cnf = os.path.join(workdir, "case.cnf")
    with open(cnf, "w") as fh:
        fh.write(f.dimacs())
    proof = os.path.join(workdir, "case.proof")
    for p in (proof, proof + ".pbp", proof + ".derived.cnf"):
        if os.path.exists(p):
            os.remove(p)
    cmd = [solver, "--inccnf=1"] + options + [icnf] + ([proof] if want_proof else [])
    p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=600)
    if p.returncode != 0:
        return [f"the solver exited with {p.returncode}: {p.stdout.strip().splitlines()[-1:]}"]
    answers = parse_inccnf_output(p.stdout)
    counters = {}
    for line in p.stdout.splitlines():
        if line.startswith("c assume "):
            for tok in line.split()[2:]:
                k, _, v = tok.partition("=")
                counters["assume_" + k] = int(v)
        else:
            m = STAT_LINE.match(line)
            if m and m.group(1) in COUNTED:
                counters[m.group(1)] = int(m.group(2))
    bad = []
    if stats is not None:
        stats.append(counters)
    # A question that comes back UNKNOWN must have used its limit: one with
    # no work at all was stopped by something left over from the question
    # before (review round 1: a stop flag raised in preprocessing).  The
    # solver prints what each question cost unless it runs with -q.
    work = {}
    for line in p.stdout.splitlines():
        if line.startswith("c question "):
            tok = line.split()
            work[int(tok[2])] = int(tok[4])
    for ans in answers:
        if ans["status"] == "UNKNOWN" and work.get(ans["number"]) == 0 and "--assumeconflicts=0" not in options:
            bad.append(f"question {ans['number']}: UNKNOWN with no work done")
    if len(answers) != len(questions):
        bad.append(f"{len(answers)} answers for {len(questions)} questions")
    for i, (q, ans) in enumerate(zip(questions, answers)):
        if ans["number"] != i + 1:
            bad.append(f"answer {i + 1} is numbered {ans['number']}")
        if want_proof and ans["proof_bytes"] is None:
            bad.append(f"question {i + 1}: no `p` line although a proof is written")
        for why in check_answer(f, oracle, q, ans, limited, cnf, proof if want_proof else None):
            bad.append(f"question {i + 1} {q}: {why}")
    return bad


STOCK = ("--structsym=0 --structparity=0 --structcount=0 --structarith=0 --structsort=0 --structdiv=0 "
         "--structdd=0 --gauss=0 --card=0 --structpoints=0 --freewalk=0").split()


def check_split_case(solver, f, rng, workdir, frozen, want_proof, cross, options, stats=None):
    """One run of the split on the plain formula. The control set, the bits a
    level, the levels and both limits are drawn at random, so that stage 1
    answers everything in some runs and nothing in others, cases are split
    again, and some splits give up and leave the search to finish."""
    oracle = Oracle(f, workdir, frozen, cross)
    cnf = os.path.join(workdir, "case.cnf")
    with open(cnf, "w") as fh:
        fh.write(f.dimacs())
    proof = os.path.join(workdir, "case.proof")
    for p in (proof, proof + ".pbp", proof + ".derived.cnf"):
        if os.path.exists(p):
            os.remove(p)
    used = sorted({abs(l) for c in f.clauses for l in c})
    count = min(len(used), rng.choice([2, 3, 4, 6, 9]))
    control = rng.sample(used, count)
    two_stage = rng.random() < 0.85
    opts = ["--ctlsplit=1", f"--ctlsplitassume={1 if two_stage else 0}",
            f"--ctlsplitbits={rng.choice([1, 2, 3])}", f"--ctlsplitlevels={rng.choice([1, 2, 3])}",
            "--ctlsplitratio=100000", f"--ctlsplitassumeload={rng.choice([1, 20, 100, 100000])}",
            f"--ctlsplitcase={rng.choice([2, 20, 200, 100000])}"]
    if rng.random() < 0.5:
        opts = STOCK + opts
    env = dict(os.environ)
    env["SAT_CTL_VARS"] = ",".join(map(str, control))
    cmd = [solver] + opts + options + [cnf] + ([proof] if want_proof else [])
    p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=600, env=env)
    status = "none"
    model = []
    done = False
    line = ""
    for out in p.stdout.splitlines():
        if out.startswith("s "):
            status = out[2:].strip()
        elif out.startswith("v "):
            for t in out.split()[1:]:
                if t == "0":
                    done = True
                else:
                    model.append(int(t))
        elif out.startswith("c control ctlsplit="):
            line = out
    if stats is not None and line:
        counters = {}
        for tok in line.split()[2:]:
            k, _, v = tok.partition("=")
            if v.isdigit():
                counters["split_" + k] = int(v)
            elif k == "status":
                counters["status_" + v] = 1
        stats.append(counters)
    what = f"control {control} options {' '.join(opts[-7:])}: "
    truth = oracle.sat([])
    if status not in ("SATISFIABLE", "UNSATISFIABLE"):
        return [what + f"status {status!r} (exit {p.returncode})"]
    if (status == "SATISFIABLE") != truth:
        return [what + f"answered {status}, the oracle says otherwise ({line})"]
    if status == "SATISFIABLE":
        ans = {"status": status, "model": model, "model_done": done, "core": None, "proof_bytes": None}
        return [what + why for why in check_answer(f, oracle, [], ans)]
    if want_proof:
        verdict = proof_gate.check_unsat(cnf, proof, timeout=300)
        if verdict != "ok":
            return [what + f"the proof does not check: {verdict} ({line})"]
    return []


def cmd_fuzz(args):
    if getattr(args, "preset", None):
        names = sorted(PRESETS) if args.preset == "all" else args.preset.split(",")
        worst = 0
        for name in names:
            if name not in PRESETS:
                raise SystemExit(f"unknown preset {name!r}; known: {', '.join(sorted(PRESETS))}")
            opts, limited = PRESETS[name]
            one = argparse.Namespace(**vars(args))
            one.preset = None
            one.options = (opts + " " + args.options).strip()
            one.limited = limited or args.limited
            print(f"--- preset {name}: {one.options}")
            worst = max(worst, cmd_fuzz(one))
        return worst
    solver = os.path.abspath(args.solver)
    for path, what in ((solver, "solver"), (KISSAT, "C kissat"), (CADICAL, "CaDiCaL")):
        if not os.access(path, os.X_OK):
            raise SystemExit(f"missing {what}: {path} (in a worktree, run tools/worktree_links.sh; "
                             f"CaDiCaL: tools/build_reference_solvers.sh cadical)")
    frozen = os.path.abspath(args.frozen) if args.frozen else None
    if frozen and not os.access(frozen, os.X_OK):
        raise SystemExit(f"missing frozen main build: {frozen}")
    options = args.options.split() if args.options else []
    base = tempfile.mkdtemp(prefix="assume-fuzz-")
    stats = {"formulas": 0, "questions": 0, "failed": 0, "oracle": 0}
    counters = []   # per run of the test mode: the solver's own counts

    def one(i):
        rng = random.Random(args.seed * 1_000_003 + i)
        f = gen_formula(rng, args.hard)
        qs = gen_questions(rng, f, args.plain)
        wd = os.path.join(base, f"w{i % args.jobs}-{i}")
        os.makedirs(wd)
        try:
            if args.cmd == "units":
                bad = check_units_case(solver, f, qs, wd, frozen, args.proof, args.cross, options)
            elif args.cmd == "split":
                bad = check_split_case(solver, f, rng, wd, frozen, args.proof, args.cross, options, counters)
            else:
                bad = check_run_case(solver, f, qs, wd, frozen, args.proof, args.cross, options, args.limited,
                                     counters)
            kind = "bad" if bad else "ok"
        except OracleFailure as e:
            bad, kind = [str(e)], "oracle"
        except subprocess.TimeoutExpired:
            bad, kind = ["a run timed out"], "bad"
        if kind == "ok":
            shutil.rmtree(wd, ignore_errors=True)
        return i, f, qs, bad, kind, wd

    with ThreadPoolExecutor(max_workers=args.jobs) as ex:
        for i, f, qs, bad, kind, wd in ex.map(one, range(args.count)):
            stats["formulas"] += 1
            stats["questions"] += len(qs)
            if kind == "oracle":
                stats["oracle"] += 1
            elif kind == "bad":
                stats["failed"] += 1
            if bad:
                print(f"{'ORACLE' if kind == 'oracle' else 'FAIL'} formula {i} (seed {args.seed}, {f.kind}, "
                      f"{f.nvars} variables, {len(f.clauses)} clauses), kept in {wd}")
                for why in bad[:6]:
                    print(f"    {why}")
                sys.stdout.flush()
    asked = f"{stats['questions']} questions, " if args.cmd != "split" else ""
    print(f"{args.cmd}: {stats['formulas']} formulas, {asked}"
          f"{stats['failed']} formulas with a wrong answer, {stats['oracle']} oracle failures "
          f"(seed {args.seed}, proofs {'checked' if args.proof else 'not checked'}, "
          f"oracles: truth table up to {TABLE_MAX} variables, kissat, cadical"
          f"{', frozen main' if frozen else ''}{', cross-checked' if args.cross else ''})")
    if counters:
        total = {}
        for c in counters:
            for k, v in c.items():
                total[k] = total.get(k, 0) + v
        if total:
            print("the solver's own counts, summed: " + " ".join(f"{k}={total[k]}" for k in sorted(total)))
        else:
            print("the solver's own counts: none printed (run without -q and with -s to see them)")
    if not stats["failed"] and not stats["oracle"]:
        shutil.rmtree(base, ignore_errors=True)
    return 1 if stats["failed"] or stats["oracle"] else 0


# ---------------------------------------------------------------------------
# Selftest: the harness against right and wrong answers
# ---------------------------------------------------------------------------

def cmd_selftest(_args):
    checks = 0

    def expect(bad, needle, what):
        nonlocal checks
        checks += 1
        if needle is None:
            assert not bad, f"{what}: a right answer was refused: {bad}"
        else:
            assert any(needle in b for b in bad), f"{what}: not caught (got {bad})"

    with tempfile.TemporaryDirectory(prefix="assume-selftest-") as d:
        # the plan's worked example: a=1 b=2 c=3 d=4 x=5 y=6
        f = Formula(6, [[-1, 5], [-2, -5, 6], [-3, -6]], [], "toy")
        cnf = os.path.join(d, "toy.cnf")
        open(cnf, "w").write(f.dimacs())
        oracle = Oracle(f, d)
        q = [1, 2, 4, 3]
        good_proof = os.path.join(d, "good.drat")
        open(good_proof, "w").write("-1 -2 -3 0\n")
        size = os.path.getsize(good_proof)

        def unsat(core, pb=size, proof=good_proof):
            return check_answer(f, oracle, q, {"status": "UNSATISFIABLE", "model": [], "core": core,
                                               "proof_bytes": pb}, cnf_path=cnf, proof_path=proof)

        expect(unsat([1, 2, 3]), None, "the right core")
        expect(unsat([1, 2, 3], pb=0), None, "the right core, the proof cut to nothing (propagation is enough)")
        expect(unsat([1, 3]), "satisfiable together", "a core that is satisfiable with the formula")
        expect(unsat([1, 2, 3, 5]), "not among the assumptions", "a core that is not a subset")
        expect(unsat([1, 2, 3, 3]), "not among the assumptions", "a core literal used more often than assumed")
        expect(check_answer(f, oracle, q, {"status": "SATISFIABLE", "model": [1, 2, 3, 4, 5, 6], "core": None,
                                           "proof_bytes": None}), "the oracle says", "a wrong status (SAT for UNSAT)")
        q2 = [1, 2]
        model = [1, 2, -3, 4, 5, 6]
        expect(check_answer(f, oracle, q2, {"status": "SATISFIABLE", "model": model, "core": None,
                                            "proof_bytes": None}), None, "a right model")
        expect(check_answer(f, oracle, q2, {"status": "SATISFIABLE", "model": [1, 2, -3, 4, 5, -6], "core": None,
                                            "proof_bytes": None}), "does not satisfy", "a wrong model")
        expect(check_answer(f, oracle, q2, {"status": "SATISFIABLE", "model": [1, -2, -3, 4, 5, 6], "core": None,
                                            "proof_bytes": None}), "breaks the assumption",
               "a model that breaks an assumption")
        expect(check_answer(f, oracle, q2, {"status": "SATISFIABLE", "model": [1, 2, -3, 4, 5], "core": None,
                                            "proof_bytes": None}), "does not satisfy",
               "a model that leaves a needed variable out")
        expect(check_answer(f, oracle, q2, {"status": "UNSATISFIABLE", "model": [], "core": [1, 2],
                                            "proof_bytes": None}), "the oracle says", "a wrong status (UNSAT for SAT)")
        expect(check_answer(f, oracle, q2, {"status": "UNKNOWN", "model": [], "core": None,
                                            "proof_bytes": None}), "without a limit", "UNKNOWN with no limit set")
        expect(check_answer(f, oracle, q2, {"status": "UNKNOWN", "model": [], "core": None,
                                            "proof_bytes": None}, limited=True), None, "UNKNOWN under a limit")

        # a formula that needs a lemma: propagation alone does not refute it
        g = Formula(3, [[1, 2], [1, -2], [-1, 3], [-1, -3]], [], "toy2")
        gcnf = os.path.join(d, "toy2.cnf")
        open(gcnf, "w").write(g.dimacs())
        goracle = Oracle(g, d)
        lemma = os.path.join(d, "lemma.drat")
        open(lemma, "w").write("1 0\n")
        wrong = os.path.join(d, "wrong.drat")
        # two "lemmas" that contradict each other on a fresh variable: the
        # classic way to fake a refutation; the second is not a consequence
        open(wrong, "w").write("4 0\n-4 0\n")
        binary = os.path.join(d, "lemma.bdrat")
        open(binary, "wb").write(b"a\x02\x00")

        def empty_core(proof, pb):
            return check_answer(g, goracle, [], {"status": "UNSATISFIABLE", "model": [], "core": [],
                                                 "proof_bytes": pb}, cnf_path=gcnf, proof_path=proof)

        expect(empty_core(lemma, 4), None, "a proof with the lemma it needs")
        expect(empty_core(binary, 3), None, "the same proof in binary")
        expect(empty_core(lemma, 0), "does not check", "a proof cut before its lemma")
        expect(empty_core(wrong, 9), "does not check", "a proof with a lemma that does not follow")
        expect(empty_core(lemma, 99), "does not check", "a `p` line past the end of the proof")

        # a core next to a structure proof is refused: that proof keeps the
        # formula's satisfiability, not the formula's under the assumptions
        shutil.copy(good_proof, os.path.join(d, "side.drat"))
        open(os.path.join(d, "side.drat.pbp"), "w").write("pseudo-Boolean proof version 3.0\nf 3 ;\n")
        open(os.path.join(d, "side.drat.derived.cnf"), "w").write("p cnf 6 0\n")
        expect(unsat([1, 2, 3], proof=os.path.join(d, "side.drat")), "together with a structure proof",
               "a core next to a structure proof")

        # the output parser
        text = "c x\nq 1 UNSATISFIABLE\nf 1 2 0\np 37\nq 2 SATISFIABLE\nv 1 -2\nv 3 0\np 37\nq 3 UNKNOWN\nq 4 UNSATISFIABLE\nf 0\n"
        a = parse_inccnf_output(text)
        assert [x["status"] for x in a] == ["UNSATISFIABLE", "SATISFIABLE", "UNKNOWN", "UNSATISFIABLE"]
        assert a[0]["core"] == [1, 2] and a[0]["proof_bytes"] == 37
        assert a[1]["model"] == [1, -2, 3] and a[1]["model_done"]
        assert a[3]["core"] == []
        checks += 4

        # truth table against the oracle solvers, on fresh random formulas
        rng = random.Random(7)
        agreed = 0
        for _ in range(60):
            h = gen_formula(rng)
            if h.nvars > TABLE_MAX:
                continue
            o = Oracle(h, d, cross=True)
            for hq in gen_questions(rng, h):
                o.sat(hq)   # raises OracleFailure on any disagreement
                agreed += 1
        assert agreed > 100, agreed
        # and the formula writer round-trips through the gate's own reader
        again = list(proof_gate.iter_clauses(cnf))
        assert again == f.clauses, again
    print(f"selftest ok: {checks} checks of right and wrong answers, "
          f"{agreed} questions where the truth table, kissat and cadical agree")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("selftest").set_defaults(func=cmd_selftest)
    for name in ("units", "run", "split"):
        p = sub.add_parser(name)
        p.add_argument("--solver", required=True)
        p.add_argument("--count", type=int, default=200)
        p.add_argument("--seed", type=int, default=1)
        p.add_argument("--jobs", type=int, default=8)
        p.add_argument("--proof", action="store_true", help="write and check proofs")
        p.add_argument("--cross", action="store_true",
                       help="also ask the oracle solvers where the truth table decides")
        p.add_argument("--frozen", default=os.environ.get("SAT_FROZEN_MAIN", ""),
                       help="a frozen build of main as a third oracle solver")
        p.add_argument("--options", default="", help="extra solver options")
        p.add_argument("--limited", action="store_true",
                       help="run mode: the options set a per-question limit, so UNKNOWN is allowed")
        p.add_argument("--preset", default="",
                       help="run mode: named option sets, comma-separated, or 'all' "
                            f"({', '.join(sorted(PRESETS))})")
        p.add_argument("--hard", type=float, default=0.0,
                       help="share of formulas that are large random 3-SAT at the threshold")
        p.add_argument("--plain", type=float, default=0.08,
                       help="share of questions with no assumptions")
        p.set_defaults(func=cmd_fuzz)
    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
