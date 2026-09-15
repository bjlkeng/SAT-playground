#!/usr/bin/env bash
# Tracked copy of the launcher run on 2026-09-15 (the live copy and its console log are
# under log/rl-step0/, which git ignores). Run detached:
#   setsid nohup bash tools/rl_step0_sweeps.sh > log/rl-step0/launcher-$(date +%Y%m%d-%H%M%S).log 2>&1 < /dev/null &
# Report each sweep with: python3 tools/rl_sweep_report.py log/abtest-rl-step0-stage1-<ts>
# RL plan step 0 (beads SAT-playground-p9m.5.2 / .5.3 / .5.4): constant-knob
# baseline sweeps on sat-comp-2025-medium, stock v each knob at half and double.
# Sweep 1 = stage-1 knobs (intervals, restart margin, sweep effort), 18 arms.
# Sweep 2 = stage-2 knobs (per-pass effort, reduce fraction), 16 arms, queued
# right after. Decisions 2026-09-15: one 18-arm sweep, 1800 s, no proofs,
# stage 2 queued. kissat 4.0.4 has no reducefraction; its reduce fraction is
# reducelow/reducehigh (500/900 per mille), halved together here. The stage-1
# "sweep effort 0 = skip" arm is --sweep=0 (the pass off): --sweepeffort=0 would
# still set up dense watches and touch the delay counters (Codex review 2026-09-15).
set -uo pipefail
cd /home/bojji/code/SAT-playground || exit 1
FROZEN="$HOME/.cache/sat13-step0/13-kissat-rs-frozen"   # frozen binary: rebuilds during the sweep cannot change it
COMMON=(--solver "$FROZEN" --suite sat-comp-2025-medium --seeds 1 --jobs 32 --mem-mb 16000 --timeout 1800 --no-verify)
S1=(--arm 'base:'
    --arm 'probeint50:SAT_EXTRA_ARGS=--probeint=50'         --arm 'probeint200:SAT_EXTRA_ARGS=--probeint=200'
    --arm 'eliminateint250:SAT_EXTRA_ARGS=--eliminateint=250' --arm 'eliminateint1000:SAT_EXTRA_ARGS=--eliminateint=1000'
    --arm 'reduceint500:SAT_EXTRA_ARGS=--reduceint=500'     --arm 'reduceint2000:SAT_EXTRA_ARGS=--reduceint=2000'
    --arm 'rephaseint500:SAT_EXTRA_ARGS=--rephaseint=500'   --arm 'rephaseint2000:SAT_EXTRA_ARGS=--rephaseint=2000'
    --arm 'reorderint5000:SAT_EXTRA_ARGS=--reorderint=5000' --arm 'reorderint20000:SAT_EXTRA_ARGS=--reorderint=20000'
    --arm 'modeint500:SAT_EXTRA_ARGS=--modeint=500'         --arm 'modeint2000:SAT_EXTRA_ARGS=--modeint=2000'
    --arm 'restartmargin5:SAT_EXTRA_ARGS=--restartmargin=5' --arm 'restartmargin20:SAT_EXTRA_ARGS=--restartmargin=20'
    --arm 'sweepoff:SAT_EXTRA_ARGS=--sweep=0'                 --arm 'sweepeffort50:SAT_EXTRA_ARGS=--sweepeffort=50'
    --arm 'sweepeffort200:SAT_EXTRA_ARGS=--sweepeffort=200')
S2=(--arm 'base:'
    --arm 'vivifyeffort50:SAT_EXTRA_ARGS=--vivifyeffort=50'         --arm 'vivifyeffort200:SAT_EXTRA_ARGS=--vivifyeffort=200'
    --arm 'eliminateeffort50:SAT_EXTRA_ARGS=--eliminateeffort=50'   --arm 'eliminateeffort200:SAT_EXTRA_ARGS=--eliminateeffort=200'
    --arm 'backboneeffort10:SAT_EXTRA_ARGS=--backboneeffort=10'     --arm 'backboneeffort40:SAT_EXTRA_ARGS=--backboneeffort=40'
    --arm 'factoreffort25:SAT_EXTRA_ARGS=--factoreffort=25'         --arm 'factoreffort100:SAT_EXTRA_ARGS=--factoreffort=100'
    --arm 'forwardeffort50:SAT_EXTRA_ARGS=--forwardeffort=50'       --arm 'forwardeffort200:SAT_EXTRA_ARGS=--forwardeffort=200'
    --arm 'transitiveeffort10:SAT_EXTRA_ARGS=--transitiveeffort=10' --arm 'transitiveeffort40:SAT_EXTRA_ARGS=--transitiveeffort=40'
    --arm 'walkeffort25:SAT_EXTRA_ARGS=--walkeffort=25'             --arm 'walkeffort100:SAT_EXTRA_ARGS=--walkeffort=100'
    --arm 'reducefrachalf:SAT_EXTRA_ARGS=--reducelow=250 --reducehigh=450')
echo "binary $(sha256sum "$FROZEN/target/release/sat-solver" | cut -c1-16) host $(hostname) $(nproc) cpus"
echo "sweep 1 (stage 1, ${#S1[@]} args) start $(date)"
python3 tools/feature_ablation.py --name rl-step0-stage1 "${COMMON[@]}" "${S1[@]}"
echo "sweep 1 exit $? at $(date)"
echo "sweep 2 (stage 2) start $(date)"
python3 tools/feature_ablation.py --name rl-step0-stage2 "${COMMON[@]}" "${S2[@]}"
echo "sweep 2 exit $? at $(date)"
echo ALL_SWEEPS_DONE
