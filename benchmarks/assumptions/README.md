# Baselines for the assumptions branch

Epic `SAT-playground-bp3`, step 0 (bead `bp3.1`). The rule of the branch is
"off means off": with no assumptions the solver must do what main's solver
does, step for step. These files are what "main's solver does" means, and the
tools to check a build against it.

## The frozen build of main

| | |
|---|---|
| main at the branch point | `5451c7b` (no solver change since `6b8a200`) |
| frozen build | `solver/13-j22-ctlsize-snapshot/target/release/sat-solver` in the main checkout (untracked, binary only) |
| sha256 | `be9a2b056d23f4443f436cfa77c1d45e9a0025f4210f5b426d8bcf9e07550e04` |

Built again in this worktree at the branch point, the binary has the same
sha256, so the frozen folder is main's build exactly. (A build of this crate
does not depend on the folder it is built in.)

## The tables

Made by `tools/as_baseline.py run`. Each run is the binary as `run.sh` calls
it, with a proof path, plus `-s` and `--seed=0`, under a work limit
(`SAT_LIMIT_TICKS`) so that it ends at the same point however busy the host
is.

| file | set | cells | work limit | answers |
|---|---|---|---|---|
| `main-5451c7b-stock.tsv` | stock path (structure passes and engines off, the options `tools/parity.py` uses) on `tests/cnf` | 9 | none | 4 SAT, 5 UNSAT |
| `main-5451c7b-stockmed.tsv` | stock path on the 100 medium cells | 100 | 2G | 18 SAT, 12 UNSAT, 70 at the limit |
| `main-5451c7b-default.tsv` | shipped defaults on the 100 medium cells | 100 | 10G | 26 SAT, 37 UNSAT, 37 at the limit |
| `main-5451c7b-split.tsv` | shipped defaults with the control split on (`--ctlsplit=1`) on the 100 medium cells and the three other oisc cells of 2025 | 103 | 10G | 26 SAT, 38 UNSAT, 39 at the limit |

The last set is there because step 5 rebuilds the split: the one-stage split
must stay what it is. Of the four oisc cells, sll-8 is decided by the split
within the limit (5.28G work); the other three stop at 10G.

Columns, per cell:

- `exit`, `status`: the exit code and the answer.
- `workclock`: the solver's work-clock line (ticks of every kind, conflicts,
  decisions, propagations).
- `counters_sha`: a hash of the `-s` counters, the ones `parity.py` compares.
- `text_sha`: a hash of the whole output with times, memory and paths taken
  out. The report rows of the search are in it.
- `model_sha`: a hash of the model lines.
- `proof_bytes`, `proof_sha`: the DRAT file. `pbp_*`: the VeriPB file.
  `derived_*`: the derived clauses. The files are hashed and deleted; the
  four sets wrote 312 DRAT files (107 GB) and 170 VeriPB files.
- `maxrss_kb`, `wall_s`: peak memory and wall time. Not exact, so `compare`
  only reports them.

`tools/as_baseline.py compare A B` checks every exact field of every cell and
shows the first differing lines of the output when one differs.

## Is the comparison itself repeatable?

Yes, after two fixes. Main's build was run twice over the first three sets,
and a third time later (the tables here are the third pass; it equals the
first two in every exact field).

| set | cells | identical in the first comparison | after the fixes |
|---|---|---|---|
| stock | 9 | 9 | 9 |
| stockmed | 100 | 95 | 100 |
| default | 100 | 86 | 100 |

What differed was never the search:

- 19 cells differed only in the spacing of the report's header lines. They
  are laid out by the width of the seconds and MB columns.
- 3 cells (the lockchart ones) differed only in `walk_ms`, a wall time on the
  free-choice walk's exit line.

Both are now taken out before hashing. So a difference that `compare` finds
between two builds is a real one.

One more thing was wrong at first: peak memory. A child process keeps its
parent's size at the fork as its own maximum, so every small run showed the
tool's own few hundred MB. The tool now reads the solver's own high-water
mark from `/proc`, every 20 ms. That is a sample, not an exact number:
between main's build and the branch's build (same search, cell by cell) the
middle cell's ratio is 1.000 and nine cells in ten are within 0.986 to
1.024, with single cells off by up to 17% either way. `compare` lists a cell
only when the two differ by more than 5% and 16 MB, and such a note is
something to look at, not a failure.

## Wall time: the noise floor

`tools/as_pairtime.py` runs two builds on the same cell at the same moment,
each on its own physical core, swaps the cores every repetition, and reports
the mean ratio. The cells are the 20 of `benchmarks/discriminating`, each
stopped at 100,000 conflicts, four repetitions. With main's build against a
copy of itself this is the noise floor: how far from 1.000 the ratio is when
nothing changed.

| main against a copy of itself | mean of the 20 cells | slowest and fastest cell | single repetitions |
|---|---|---|---|
| quiet host | 1.0007 | 0.9984 to 1.0033 | 0.9873 to 1.0192 |
| 28 other solver runs on the host | 1.0007 | 0.9970 to 1.0048 | 0.9748 to 1.0339 |

So a mean within about 0.001 of 1, with every cell within 0.005, is "no
difference". One repetition alone can be off by 3%; that is why there are
four, with the cores swapped. CPU time gives the same means (1.0003 and
1.0007). Measured 2026-11-15, logs in
`log/as-pairtime-2026-11-15/` of the main checkout (`aa-quiet`, `aa-load`).

## How to check a build

    F=/home/bojji/code/SAT-playground/solver/13-j22-ctlsize-snapshot/target/release/sat-solver
    B=solver/13-kissat-rs/target/release/sat-solver
    for set in stock stockmed default split; do
      python3 tools/as_baseline.py run --solver $B --set $set --out /some/log/new-$set
      python3 tools/as_baseline.py compare benchmarks/assumptions/main-5451c7b-$set.tsv /some/log/new-$set
    done
    python3 solver/13-kissat-rs/tools/parity.py --corpus default --phases
    python3 tools/as_pairtime.py --a $F --b $B --out /some/log/pair-quiet
    python3 tools/as_pairtime.py --a $F --b $B --load 28 --out /some/log/pair-load

The four sets take about 52 minutes at 32 jobs. A timing run takes about 15
minutes on a quiet host and 25 under load, and needs the host to itself.

## What the branch's builds showed

| build | the four sets | wall time over main's, quiet host |
|---|---|---|
| step 2 (`753e377c`) | identical on the three sets that existed then | 0.9961 (cells 0.9795 to 1.0047) |
| steps 2 and 5, final (`b59ec0e8`) | identical: 9, 100, 100 and 103 cells | 1.0046 (cells 0.9939 to 1.0151) |

Step 2's build under 28 other runs: 0.9985 (0.9911 to 1.0058). The final
build's 0.5% is outside the noise floor and is open work for the performance
gate (bead `bp3.8`; solver README, "Assumptions", "Speed").

## When main moves

When another change to main's solver lands, the branch merges main, the
frozen build and these tables are made again from the new main, and the check
is repeated against them (bead `bp3.1`).
