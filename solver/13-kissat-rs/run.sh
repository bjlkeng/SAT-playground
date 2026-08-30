#!/usr/bin/env bash
# run.sh <cnf_path> <output_dir> — SAT Competition interface (CLAUDE.md contract)
# plus the repo result.json/status.txt output contract expected by
# tools/smoke_test.sh. The binary itself is a faithful kissat CLI port; this
# wrapper maps the competition interface onto it.
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CNF="$1"
OUTDIR="${2:-}"

if [[ -z "$OUTDIR" ]]; then
  exec "$SCRIPT_DIR/target/release/sat-solver" "$CNF"
fi

mkdir -p "$OUTDIR"
STDOUT_TMP="$OUTDIR/solver_stdout.tmp"
"$SCRIPT_DIR/target/release/sat-solver" "$CNF" "$OUTDIR/proof.out" | tee "$STDOUT_TMP"
EXIT_CODE=${PIPESTATUS[0]}

SLINE=$(grep -m1 '^s ' "$STDOUT_TMP" || true)
case "$SLINE" in
  "s SATISFIABLE") STATUS=SAT ;;
  "s UNSATISFIABLE") STATUS=UNSAT ;;
  *) STATUS=UNKNOWN ;;
esac
# proof.out is only meaningful for UNSAT; drop it otherwise so validators
# never see a partial proof next to a SAT/UNKNOWN answer.
if [[ "$STATUS" != "UNSAT" ]]; then rm -f "$OUTDIR/proof.out"; fi
rm -f "$STDOUT_TMP"

printf '%s\n' "$STATUS" > "$OUTDIR/status.txt"
PROOF_JSON=null
[[ -f "$OUTDIR/proof.out" ]] && PROOF_JSON="\"$OUTDIR/proof.out\""
cat > "$OUTDIR/result.json" <<EOF
{
  "schema_version": 1,
  "status": "$STATUS",
  "exit_code": $EXIT_CODE,
  "status_file": "$OUTDIR/status.txt",
  "proof_file": $PROOF_JSON
}
EOF
exit 0
