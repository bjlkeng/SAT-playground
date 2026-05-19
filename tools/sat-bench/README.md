# sat-bench

`sat-bench` is the Phase 1 command facade for solver 11 benchmark tooling. It provides the stable subcommand names from bead 0.5 while the Python implementations remain in place during the transition window.

Current subcommands:

- `sat-bench status-compare`
- `sat-bench validate-result`
- `sat-bench select-iter`
- `sat-bench compare`
- `sat-bench extract-hot`
- `sat-bench validate-plan`
- `sat-bench profile`

The first six subcommands delegate to the checked-in Python tools and preserve their arguments/output. `profile` is reserved for bead 0.5a and delegates to `tools/profile_solver11.sh` once that script exists.

Build:

```bash
cargo build --manifest-path tools/sat-bench/Cargo.toml --release
```
