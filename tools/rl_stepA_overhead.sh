#!/usr/bin/env bash
# RL plan step A.11 (bead SAT-playground-p9m.6.11): what does the policy
# plumbing cost when it does nothing? Two arms on sat-comp-2025-medium, same
# binary, simultaneous start on 32 pinned cores (16 + 16), 1800 s, 16 GB, no
# proofs (the cross-arm SAT/UNSAT check is the oracle):
#   base       policy off
#   policylog  policy on with the stock action, the raw-state log written per
#              cell into the run's scratch ({odir}, deleted after the run),
#              observe() at every observation epoch, the static-feature pass,
#              and the wall horizon the harness would pass at a gate.
# The work clock W must be identical per cell (the parity claim, checked by
# tools/compare_full_runs.py-style TSV diff below); the wall difference is the
# overhead. Quiet host only. Run detached:
#   mkdir -p log/rl-stepA && setsid nohup bash tools/rl_stepA_overhead.sh > log/rl-stepA/overhead-launcher-$(date +%Y%m%d-%H%M%S).log 2>&1 < /dev/null &
set -uo pipefail
cd /home/bojji/code/SAT-playground || exit 1
SRC=solver/13-kissat-rs
FROZEN="$HOME/.cache/sat13-stepA/13-kissat-rs-frozen"   # a rebuild during the run cannot change it
mkdir -p "$FROZEN/target/release"
cp "$SRC/run.sh" "$FROZEN/" && cp "$SRC/target/release/sat-solver" "$FROZEN/target/release/sat-solver" || exit 1
# The harness runs build.sh in every solver directory first; the frozen copy
# has the binary and no sources, so its build is a no-op (as in step 0).
printf '#!/usr/bin/env bash\n# Frozen copy of solver 13 for the step A.11 overhead run; nothing to build.\nexit 0\n' > "$FROZEN/build.sh"
echo "binary $(sha256sum "$FROZEN/target/release/sat-solver" | cut -c1-16) host $(hostname) $(nproc) cpus load $(cut -d' ' -f1-3 /proc/loadavg)"
echo "start $(date)"
python3 tools/feature_ablation.py --name rl-stepA11-overhead \
  --solver "$FROZEN" --suite sat-comp-2025-medium --seeds 1 --jobs 32 --mem-mb 16000 --timeout 1800 --no-verify \
  --arm 'base:' \
  --arm 'policylog:SAT_POLICY=stock,SAT_POLICY_LOG={odir}/policy.log,SAT_WALL_LIMIT=1800'
echo "exit $? at $(date)"
echo OVERHEAD_RUN_DONE
