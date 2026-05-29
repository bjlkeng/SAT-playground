# ARCHIVED search-feature efficacy results (pre-2026-05-29 re-evaluation)

**DO NOT CONSULT the material in this directory unless the user EXPLICITLY asks for it.**

These are the solver-11 search-feature efficacy verdicts/tables that existed before the
2026-05-29 fresh re-evaluation (tracked by bead SAT-playground-gbc). They were found to be
contaminated by measurement artifacts (host contention, cold-cache/first-run slowness, the
~7.5% same-binary warming variance, and single-run noise — see bd memory
`profiling-suite-longinstance-variance` and closed beads 2dd / 18.3 / 18.12 / 59l).

They are retained only as historical provenance. Any current statement about whether a search
feature helps or hurts aggregate PAR-2 must come from the FRESH re-evaluation, NOT from here.

Contents:
- `FEATURES.pre-reeval.csv` / `FEATURES.pre-reeval.md` — the feature ledger as of archival.
- `README-validation-archived.md` — the README "Validation" section (search-feature tables).
