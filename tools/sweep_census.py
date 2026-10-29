#!/usr/bin/env python3
"""Simulation-guided sweeping census (bead SAT-playground-skz.10).

Measures, on one miter that reads as a circuit by first literal (the
Netlist of tools/divsqrt_census.py), what a global SAT-sweeping pass would
find and what it would cost:

  classes   the circuit is simulated on random inputs (bit-parallel, 2,048
            patterns by default); signals with the same or the complementary
            values form a class; a class's shallowest member is its
            representative.  A class is a candidate, not a proof.
  pairs     each other member is proven equal to the representative by the
            solver binary, in depth order, with the pairs proven so far
            added as binary clauses.  A pair is first tried on a window
            (the two signals' cones, breadth first, cut at a gate budget:
            the signals below the cut are free), then on wider windows, the
            whole cone last.  A window is a subset of the circuit's clauses
            plus proven pairs, so an UNSAT answer is a proof; a model of a
            cut window may be spurious, so it only widens the window.  A
            model of the whole cone is a real counterexample: the circuit
            is simulated on it and every class it separates splits (a
            proven pair that differs on it stops the run: the solver or
            the reading is wrong).
  final     the miter with every proven pair added, on the stock path.

Subcommands:
  sweep FILE --solver BIN --out DIR [--jobs N] [--patterns P]
        [--ladder 100,1000,10000] [--conflicts 2000,10000,50000,250000]
        [--limits 60,60,120,900] [--whole-gates G] [--whole-fails F]
        [--final-limit S] [--mem-mb M]
        the ladder lists the window gate budgets; the whole cone is the last
        rung, with the last conflict and time limits.  A whole cone over
        --whole-gates gates is not tried ('toobig'); once --whole-fails
        whole-cone calls have failed no further whole cone is started
        (the calls already running finish; the pairs after them are
        'stopped') and the sweep ends with the level.  Writes
        summary.json, proven.json and progress.log under DIR.
  report FILE DIR...
        per rung the pairs, calls, seconds and conflicts (a check killed
        before it printed its counters is counted in missing_counters and
        left out of the totals, never counted as zero); the proven pairs
        by kind (joining the two sides, inside one side, touching shared
        logic) and by level band; the compared pair; the final solve.

The solver runs the stock path (every structure pass off) under the time
limit and --mem-mb (default 16,000); a crash, an UNKNOWN with no budget
set, or an answer with the wrong exit code stops the run (run_solver of
tools/divsqrt_census.py); a timeout, a memory-out or the solver's own
conflict budget is a failed check.  The 2026 suite is not analysed (plan
section 8: it is the holdout).

Measured 2026-10-28 on hhyp_cec_multi_2 (solver README, "Global sweeping:
the measurement"): 49,469 pairs proven, 97% of them in the lowest 100
levels at 3 conflicts each, the rest tens of thousands of conflicts each,
the chain stalled at level 369 of 1,418, the miter unchanged.
"""
from __future__ import annotations

import argparse
import json
import random
import sys
import tempfile
import time
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from divsqrt_census import MEM_MB, Netlist, miter_sides, refuse_holdout, solve, write_cnf  # noqa: E402

PATTERNS = 2048


def levels_of(net):
    level = {v: 0 for v in net.inputs}
    level.update({v: 0 for v in net.const})
    for v in net.order:
        level[v] = 1 + max(level[u] for u in net.gates[v][0])
    return level


class Sweep:
    def __init__(self, path, args, tmp):
        self.net = Netlist(path)
        self.args = args
        self.tmp = Path(tmp)
        self.root, self.a, self.b, _ = miter_sides(self.net)
        self.level = levels_of(self.net)
        self.ladder = [int(x) for x in args.ladder.split(",")] + [None]
        self.conflicts = [int(x) for x in args.conflicts.split(",")]
        self.limits = [float(x) for x in args.limits.split(",")]
        if not len(self.ladder) == len(self.conflicts) == len(self.limits):
            sys.exit("--ladder needs one fewer entry than --conflicts and --limits (the whole cone is the last rung)")
        rnd = random.Random(args.seed)
        self.width = args.patterns
        self.mask = (1 << self.width) - 1
        self.val = self.net.simulate({v: rnd.getrandbits(self.width) for v in self.net.inputs}, self.mask)
        self.proven = []          # (x, y) literals, x == y, in the order proven
        self.parent = {}          # var -> (parent var, 1 when opposite to it): the proven links
        self.whole_calls = 0
        self.whole_fails = 0
        self.log = open(self.tmp / "progress.log", "a")

    def canon(self, v):
        s = self.val[v]
        return (s ^ self.mask, 1) if s & 1 else (s, 0)

    def find(self, v):
        """(root, parity): v equals its root, or its complement when parity is 1."""
        par = 0
        while v in self.parent:
            u, q = self.parent[v]
            par ^= q
            v = u
        return v, par

    def link(self, x, y):
        """Record x == y (literals) among the proven links."""
        rx, px = self.find(abs(x))
        ry, py = self.find(abs(y))
        if rx == ry:
            return
        self.parent[ry] = (rx, px ^ py ^ (1 if (x < 0) != (y < 0) else 0))

    def related(self, a, b):
        """None, or 'equal' / 'complement' once the proven links connect a and b."""
        ra, pa = self.find(a)
        rb, pb = self.find(b)
        if ra != rb:
            return None
        return "complement" if pa ^ pb else "equal"

    def cone_size(self, x, y):
        net = self.net
        seen = {abs(x), abs(y)}
        stack = list(seen)
        n = 0
        while stack:
            v = stack.pop()
            if v not in net.gates:
                continue
            n += 1
            for u in net.gates[v][0]:
                if u not in seen:
                    seen.add(u)
                    stack.append(u)
        return n

    def window(self, x, y, budget):
        """The clauses of the cones of |x| and |y|, breadth first, up to
        `budget` gates (None: all), the constants among the signals reached,
        the proven pairs inside, and 'x and y differ', renamed densely.
        Returns (clauses, back, cut gates, gates)."""
        net = self.net
        seen = {abs(x), abs(y)}
        queue = [abs(x), abs(y)]
        gates_in, cut = [], 0
        qi = 0
        while qi < len(queue):
            v = queue[qi]
            qi += 1
            if v not in net.gates:
                continue
            if budget is not None and len(gates_in) >= budget:
                cut += 1
                continue
            gates_in.append(v)
            for u in net.gates[v][0]:
                if u not in seen:
                    seen.add(u)
                    queue.append(u)
        dense, back = {}, [0]

        def name(lit):
            v = abs(lit)
            if v not in dense:
                back.append(v)
                dense[v] = len(back) - 1
            return -dense[v] if lit < 0 else dense[v]

        clauses = []
        for v in gates_in:
            for i in net.groups[v]:
                clauses.append(tuple(name(lit) for lit in net.clauses[i]))
        for v in seen:
            if v in net.const:
                clauses.append((name(v if net.const[v] else -v),))
        for p, q in self.proven:
            if abs(p) in seen and abs(q) in seen:
                clauses.append((name(-p), name(q)))
                clauses.append((name(p), name(-q)))
        clauses.append((name(x), name(y)))
        clauses.append((name(-x), name(-y)))
        return clauses, back, cut, len(gates_in)

    def prove(self, x, y, tag):
        """Up the ladder.  status: proven (with the rung), differs (with the
        model's true inputs), toobig, or the failed whole-cone check's
        status in lower case (budget, timeout, memout)."""
        out = dict(status="unknown", rung=None, calls=0, seconds=0.0, conflicts=0, work=0, missing=0, gates=0, cone=None, model=None)
        for k, budget in enumerate(self.ladder):
            if budget is None:
                if self.whole_fails >= self.args.whole_fails:
                    out["status"] = "stopped"   # the cap was reached by a check before this one
                    return out
                out["cone"] = self.cone_size(x, y)
                if out["cone"] > self.args.whole_gates:
                    out["status"] = "toobig"
                    return out
                self.whole_calls += 1
            clauses, back, cut, gates = self.window(x, y, budget)
            f = self.tmp / f"w{tag}.cnf"
            write_cnf(f, len(back) - 1, clauses)
            st, el, confl, model, work = solve(self.args.solver, f, self.limits[k], extra=[f"--conflicts={self.conflicts[k]}"], mem_mb=self.args.mem_mb)
            f.unlink()
            out["calls"] += 1
            out["seconds"] += el
            if confl < 0 or work < 0:
                out["missing"] += 1   # killed before its counters were printed: left out of the totals
            else:
                out["conflicts"] += confl
                out["work"] += work
            out["gates"] = gates
            if st == "UNSATISFIABLE":
                out["status"], out["rung"] = "proven", ("whole" if budget is None else str(budget))
                return out
            if st == "SATISFIABLE" and cut == 0:
                # no cut gate: the window is the whole cone and the model a
                # real counterexample (the cone's inputs as it sets them)
                out["status"] = "differs"
                out["model"] = {back[t] for t in model if t < len(back)}
                return out
            if budget is None:
                out["status"] = st.lower()
                self.whole_fails += 1
                return out
        return out

    def run(self):
        net = self.net
        by_level = defaultdict(list)
        for v in net.order:
            if v not in net.roots:
                by_level[self.level[v]].append(v)
        reps = {}
        for v in net.inputs:
            reps.setdefault(self.canon(v)[0], v)
        stats = Counter()
        cost = defaultdict(lambda: dict(pairs=0, calls=0, seconds=0.0, conflicts=0, work=0, missing_counters=0))
        cones = []
        const_cands = 0
        compared = None
        top_level = 0
        t0 = time.time()
        levels = sorted(by_level)
        if not levels:
            print("no gate but the root: no pair to prove", file=self.log, flush=True)
        for lv in levels:
            pending = []
            for y in by_level[lv]:
                if self.val[y] in (0, self.mask):
                    const_cands += 1   # a constant signal: a unit, not a pair
                else:
                    pending.append(y)
            rounds = 0
            while pending and rounds < 8:
                rounds += 1
                pairs = []
                for y in pending:
                    sig, pol = self.canon(y)
                    r = reps.get(sig)
                    if r is None:
                        reps[sig] = y
                        continue
                    pairs.append((r if self.canon(r)[1] == pol else -r, y))
                pending = []
                if not pairs:
                    break
                with ThreadPoolExecutor(self.args.jobs) as ex:
                    results = list(ex.map(lambda t: self.prove(pairs[t][0], pairs[t][1], f"{lv}_{t}"), range(len(pairs))))
                patterns = []
                for (xl, y), res in zip(pairs, results):
                    stats[res["status"]] += 1
                    key = res["rung"] if res["status"] == "proven" else res["status"]
                    c = cost[key]
                    c["pairs"] += 1
                    for f in ("calls", "seconds", "conflicts", "work"):
                        c[f] += res[f]
                    c["missing_counters"] += res["missing"]
                    if res["cone"] is not None:
                        cones.append((res["cone"], res["status"]))
                    if res["status"] == "proven":
                        self.proven.append((xl, y))
                        self.link(xl, y)
                        top_level = max(top_level, lv)
                        if compared is None and self.related(self.a, self.b) is not None:
                            # through this pair, directly or through a shared representative
                            compared = dict(level=lv, relation=self.related(self.a, self.b), rung=res["rung"],
                                            direct={abs(xl), y} == {self.a, self.b}, seconds=round(res["seconds"], 2), conflicts=res["conflicts"])
                    elif res["status"] == "differs":
                        patterns.append(res["model"])
                        pending.append(y)   # re-keyed after the split
                for on in patterns:
                    one = net.simulate({v: (1 if v in on else 0) for v in net.inputs}, 1)
                    for v in self.val:
                        self.val[v] = (self.val[v] << 1) | (one[v] & 1)
                    self.width += 1
                    self.mask = (self.mask << 1) | 1
                if patterns:
                    # appending bits only splits classes; the representatives stay
                    reps = {self.canon(r)[0]: r for r in reps.values()}
                    for (xl, y), res in zip(pairs, results):
                        if res["status"] == "proven" and self.canon(y)[0] != self.canon(abs(xl))[0]:
                            raise RuntimeError(f"proven pair ({xl}, {y}) differs on a counterexample: the solver or the reading is wrong")
            if self.whole_fails >= self.args.whole_fails:
                print(f"level {lv}: stopped after {self.whole_fails} failed whole-cone calls", file=self.log, flush=True)
                stats["stopped_at_level"] = lv
                break
            if lv % 50 == 0 or lv == levels[-1]:
                print(f"level {lv}/{levels[-1]}: proven {len(self.proven)}, {dict(stats)}, whole-cone calls {self.whole_calls}, {time.time() - t0:.0f} s", file=self.log, flush=True)
        return dict(
            cell=self.net.path.name, gates=len(net.gates), inputs=len(net.inputs), levels=levels[-1] if levels else 0,
            root=self.root, compared_signals=[self.a, self.b], patterns=self.width,
            stats=dict(stats), cost={k: dict(v, seconds=round(v["seconds"], 1)) for k, v in cost.items()},
            const_cands=const_cands, proven=len(self.proven), top_level=top_level, compared=compared,
            whole_calls=self.whole_calls,
            cones=dict(proven=sorted(c for c, s in cones if s == "proven"), other=sorted((c, s) for c, s in cones if s != "proven")),
            seconds=round(time.time() - t0, 1))

    def final(self):
        net = self.net
        n_vars = max([net.header_vars] + [abs(lit) for c in net.clauses for lit in c])
        extra = [c for p, q in self.proven for c in ((-p, q), (p, -q))]
        f = self.tmp / "miter_with_pairs.cnf"
        write_cnf(f, n_vars, list(net.clauses) + extra)
        st, el, confl, _, work = solve(self.args.solver, f, self.args.final_limit, mem_mb=self.args.mem_mb)
        f.unlink()
        return dict(result=st, seconds=round(el, 1), conflicts=confl, work=work, added=len(extra))


def cmd_sweep(args):
    path = Path(args.file)
    refuse_holdout([path])
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix="sweep-", dir=out))
    try:
        sw = Sweep(path, args, tmp)
    except ValueError as e:
        sys.exit(f"{path.name}: {e}")
    print(f"read {len(sw.net.gates)} gates, {len(sw.net.inputs)} inputs, root {sw.root} = cmp({sw.a}, {sw.b}), {max(sw.level.values())} levels", file=sw.log, flush=True)
    summary = sw.run()
    summary["final"] = sw.final()
    with open(out / "proven.json", "w") as f:
        json.dump(sw.proven, f)
    with open(out / "summary.json", "w") as f:
        json.dump(summary, f, indent=1)
    sw.log.close()
    (tmp / "progress.log").rename(out / "progress.log")
    tmp.rmdir()
    print(json.dumps(summary))
    return 0


def cmd_report(args):
    path = Path(args.file)
    refuse_holdout([path])
    net = Netlist(path)
    _, a, b, _ = miter_sides(net)
    ca, cb = net.cone(a), net.cone(b)
    level = levels_of(net)
    for run in args.dirs:
        d = json.load(open(Path(run) / "summary.json"))
        proven = json.load(open(Path(run) / "proven.json"))
        print(f"== {run}: {d['proven']} proven, {d['stats']}, sweep {d['seconds']} s, {d['patterns']} patterns, {d['const_cands']} constant signals")
        for k, v in sorted(d["cost"].items(), key=lambda kv: -kv[1]["pairs"]):
            miss = f"  {v['missing_counters']} checks without counters" if v.get("missing_counters") else ""
            print(f"   {k:>8}: pairs {v['pairs']:6d}  calls {v['calls']:6d}  seconds {v['seconds']:8.1f}  conflicts {v['conflicts']:10d} ({v['conflicts'] / max(1, v['pairs']):8.1f} a pair){miss}")
        kinds = Counter()
        bands = Counter()
        top = 0   # the highest level among the pairs (the probes before this tool saved no top_level)
        for x, y in proven:
            x, y = abs(x), abs(y)
            sides = {("A" if v in ca else "") + ("B" if v in cb else "") for v in (x, y)}
            if sides == {"A", "B"}:
                kinds["joining the sides"] += 1
            elif sides == {"A"} or sides == {"B"}:
                kinds[f"inside {sides.pop()}"] += 1
            else:
                kinds["touching shared logic"] += 1
            top = max(top, level[x], level[y])
            bands[max(level[x], level[y]) // 100] += 1
        print("   proven pairs:", ", ".join(f"{n} {k}" for k, n in sorted(kinds.items())), f"; highest level {d.get('top_level', top)}")
        print("   by level band (x100):", " ".join(f"{k}:{bands[k]}" for k in sorted(bands)))
        print(f"   compared pair: {d['compared']}")
        print(f"   final miter + pairs: {d['final']}")
        c = d.get("cones", dict(proven=[], other=[]))   # run 1 of the probes recorded no cone sizes
        if c["proven"] or c["other"]:
            pr = c["proven"]
            other = Counter(s for _, s in c["other"])
            skipped = other.pop("toobig", 0) + other.pop("stopped", 0)
            print(f"   whole-cone calls: {len(pr)} proven (cones of {pr[0] if pr else '-'} to {pr[-1] if pr else '-'} gates, median {pr[len(pr) // 2] if pr else '-'}), "
                  + ", ".join(f"{n} {s}" for s, n in sorted(other.items())) + f"; {skipped} cones not tried (too big or after the stop)")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("sweep")
    s.add_argument("file")
    s.add_argument("--solver", required=True)
    s.add_argument("--out", required=True)
    s.add_argument("--jobs", type=int, default=6)
    s.add_argument("--patterns", type=int, default=PATTERNS)
    s.add_argument("--seed", type=int, default=1)
    s.add_argument("--ladder", default="100,1000,10000", help="window gate budgets, smallest first")
    s.add_argument("--conflicts", default="2000,10000,50000,250000", help="conflict limit per rung, the whole cone last")
    s.add_argument("--limits", default="60,60,120,900", help="seconds per rung, the whole cone last")
    s.add_argument("--whole-gates", type=int, default=60_000, help="largest cone (gates) solved whole")
    s.add_argument("--whole-fails", type=int, default=150, help="stop after this many failed whole-cone calls")
    s.add_argument("--final-limit", type=float, default=1800)
    s.add_argument("--mem-mb", type=int, default=MEM_MB)
    r = sub.add_parser("report")
    r.add_argument("file")
    r.add_argument("dirs", nargs="+")
    args = ap.parse_args(argv)
    return {"sweep": cmd_sweep, "report": cmd_report}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
