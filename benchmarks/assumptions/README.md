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

A fifth set, `full`, is every one of the 400 cells of 2025 under its own work
budget (column `B_cell` of `benchmarks/rl/cells_2025.tsv`: up to 102G, 23G for
the middle cell, 12.4 thousand G together). It is the last search check of the performance gate
and takes hours, so it has no table here: main's build and the branch's are
run one after the other and compared (`log/as-baseline-2026-11-15/main-full`
and `r6-full` in the main checkout). Run on 2026-11-16 for the reviewed build:
400 of 400 cells identical in every field (125 SAT, 187 UNSAT, 87 at their
budget, one at the memory cap; 335.9 GB of proof lines with equal hashes). In this set the proof goes through a
pipe and is hashed as it comes (`--proof pipe`), so no proof file of tens of
GB sits on the disk; the hash is the one the file would have (checked on the
`stock` and `default` sets against main's file-based tables).

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
difference" **for one binary run twice**. One repetition alone can be off by
3%; that is why there are four, with the cores swapped. CPU time gives the
same means (1.0003 and 1.0007). Measured 2026-11-15, logs in
`log/as-pairtime-2026-11-15/` of the main checkout (`aa-quiet`, `aa-load`).

**Two builds are further apart than that, with nothing wrong.** A rebuild
puts the code and the solver's fields at other addresses, and on this host's
processor that changes how much of a loop has to be decoded again (solver
README, "Assumptions", "Speed"). Measured 2026-11-16, the same 20 cells:

| pair, quiet host | mean of the 20 cells | slowest and fastest cell |
|---|---|---|
| main with one unused option added, against main | 0.9933 | 0.9782 to 1.0068 |
| two builds of the branch that differ in a few lines no run executes, each against main | 1.0046 and 0.9960 | 0.9752 to 1.0151 |

So for two builds the measure is about 0.7% in the mean and 3% on a cell,
either way. What tells a real slowdown from this is the instruction count:
`tools/as_counters.py` runs the builds under `perf stat` on the small cells
and prints exact counts of instructions, clock ticks and micro-operations,
and the share of the micro-operations that went through the decoder again.
Instructions that agree within 0.2% with a moved decoder share is placement.
(Measured: the reviewed build runs 0.01% to 0.13% more instructions than
main's on the four small cells; main with one unused option 0.00% to 0.06%
more. The share moves by up to ten points between such builds.)

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
    python3 tools/as_counters.py --builds main=$F,new=$B
    # the 400 cells under their own budgets, both builds, then the comparison
    python3 tools/as_baseline.py run --solver $F --set full --out /some/log/main-full --jobs 28 --wall-limit 20000
    python3 tools/as_baseline.py run --solver $B --set full --out /some/log/new-full --jobs 28 --wall-limit 20000
    python3 tools/as_baseline.py compare /some/log/main-full /some/log/new-full

The four sets take about 52 minutes at 32 jobs. A timing run takes about 15
minutes on a quiet host and 25 under load, and needs the host to itself; the
counters take two minutes. The `full` set took 2 h 13 min a build at 28 jobs.

For runs of the whole suite under a time limit (`tools/feature_ablation.py`),
`tools/as_report.py` puts the arms side by side: cells solved, PAR-2 in time
and in work, how the answers were checked, each arm against a baseline cell
by cell, and, when the runs were made under `perf stat`, instructions, clock
ticks and the decoder share on the cells where both ran the same search to an
answer. It reads a finished run's `results.tsv` or, while a run is going, the
`kept.tsv` of `--keep-lines 'c workclock '`.

## Two builds on the whole suite

What the 20 cells cannot show is the long run that waits on memory. For that
the two builds ran all 400 cells side by side (2026-11-16; solver README,
"Assumptions", "The paired 400-cell run"). How it was set up, in the main
checkout:

- `log/as-pair400-2026-11-16/launch.sh`: one call of `tools/feature_ablation.py`
  with four arms: main's build, the same file a second time, the branch's
  build, and main's code with the jump switch. The same file twice is the
  floor of that very run.
- `solver/13-j31-pair400-snapshot/`: frozen copies of the builds and a launcher
  that stands where the harness expects one binary. It starts the arm's build
  under `perf stat`, so every run leaves its instruction count, clock ticks
  and decoder share.
- `log/as-pair400-2026-11-16/run_pair.py`: starts the harness with a core list
  and keeps each job's answer checker on the job's own core. Without that the
  checkers land beside other jobs' solvers and the same build twice differs
  by tens of percent on a cell (bead `SAT-playground-209`).
- `tools/as_report.py` makes the tables.

A 400-cell run still cannot resolve one percent of summed time: on the long
cells two runs of one build differ by 0.7% typically and by more than 3% on
one cell in five, with the core and the neighbours a run gets. For that,
`tools/as_pairtime.py` takes any folder of cells and runs them to their
answer, a cell's two builds at the same moment on two neighbouring cores of
one socket, swapping cores for the second run:

    python3 tools/as_pairtime.py --a $F --b $B --out /some/log/tight \
      --suite /folder/of/long/cells --cells /file/longest-first.txt --conflicts 0 \
      --pairs 0-1,2-3,4-5,6-7,8-9,10-11,12-13,14-15,18-19,20-21,22-23,24-25,26-27,28-29,30-31,32-33 \
      --reps 2 --proof file --options=--seed=2

`--proof file` has every run write its proof, as the harness and a
competition do; then read the processor time, since the disk is in the wall
time. Without it only the search is timed.

## What the branch's builds showed

| build | the four sets | wall time over main's, quiet host |
|---|---|---|
| step 2 (`753e377c`) | identical on the three sets that existed then | 0.9961 (cells 0.9795 to 1.0047) |
| steps 2 and 5, before the last review fix (`b59ec0e8`) | identical: 9, 100, 100 and 103 cells | 1.0046 (cells 0.9939 to 1.0151) |
| steps 2 and 5, reviewed (`b3b6a491`, commit `0065a55`) | identical: 9, 100, 100 and 103 cells (the default set with the proof through a pipe); with two stages on, the 103 split cells equal the run of `b59ec0e8`, proofs included | 0.9960 (cells 0.9752 to 1.0139) |

Under 28 other runs: step 2's build 0.9985 (0.9911 to 1.0058), the reviewed
build 0.9966 (0.9750 to 1.0131). The 1.0046 of the middle build was looked
into before the work went on (bead `bp3.8`): it is placement, as the section
on wall time says, and the next build, which differs from it in a few lines
that no run without assumptions executes, landed on the other side of 1.

## When main moves

When another change to main's solver lands, the branch merges main, the
frozen build and these tables are made again from the new main, and the check
is repeated against them (bead `bp3.1`).
