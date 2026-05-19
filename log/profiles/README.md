# Solver 11 Profile Artifacts

`tools/profile_solver11.sh` writes one directory per run:

```text
log/profiles/<date>-<instance>-<config_hash>/
  command.txt
  env.txt
  stats.jsonl
  perf-stat.txt
  perf-record.txt or unavailable.txt
  notes.md
```

Profile artifacts are generated evidence, so they are ignored by Git. Link the relevant artifact directory from milestone or bead notes when a performance decision depends on propagation, watch-list, occurrence-list, proof-throughput, or allocation-speed claims.

The tracked README exists so future agents know where profile evidence belongs. Do not commit large `perf.data` files or per-run profile directories unless the user explicitly asks for an exact artifact bundle.
