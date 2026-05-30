import random, subprocess, time, os, sys

BIN = "/tmp/sat-worktrees/lucky-70h/solver/11-kissat-port/target/release/sat-solver"
SRC = "/tmp/bship.cnf"  # decompressed battleship-16-31-sat

lines = open(SRC).read().splitlines()
pline = next(l for l in lines if l.startswith("p "))
clauses = []
for l in lines:
    s = l.strip()
    if not s or s.startswith("c") or s.startswith("p"):
        continue
    clauses.append(s.split())  # includes trailing 0

def shuffle(seed):
    rnd = random.Random(seed)
    cl = [c[:] for c in clauses]
    for c in cl:
        body = c[:-1]
        rnd.shuffle(body)
        c[:] = body + ["0"]
    rnd.shuffle(cl)
    path = f"/tmp/bship_s{seed}.cnf"
    open(path, "w").write(pline + "\n" + "\n".join(" ".join(c) for c in cl) + "\n")
    return path

def run(path, lucky):
    env = os.environ.copy()
    env["SAT_LUCKY"] = "on" if lucky else "off"
    t0 = time.time()
    out = subprocess.run([BIN, path, f"/tmp/luckyproof_{lucky}"], env=env,
                         capture_output=True, text=True)
    dt = time.time() - t0
    sline = next((l for l in out.stdout.splitlines() if l.startswith("s ")), "?")
    return sline, dt

print(f"{'seed':>5} {'lucky-ON':>20} {'lucky-OFF':>20}")
on_times, off_times = [], []
for seed in [1, 2, 3, 4, 5]:
    p = shuffle(seed)
    s_on, t_on = run(p, True)
    s_off, t_off = run(p, False)
    on_times.append(t_on); off_times.append(t_off)
    print(f"{seed:>5}  {s_on.split()[1][:5]:>6} {t_on:7.2f}s     {s_off.split()[1][:5]:>6} {t_off:7.2f}s")
print(f"\nlucky-ON  mean={sum(on_times)/len(on_times):.2f}s  max={max(on_times):.2f}s")
print(f"lucky-OFF mean={sum(off_times)/len(off_times):.2f}s  max={max(off_times):.2f}s")
print(f"per-seed gap (off-on) mean = {sum(o-n for o,n in zip(off_times,on_times))/5:.2f}s")
