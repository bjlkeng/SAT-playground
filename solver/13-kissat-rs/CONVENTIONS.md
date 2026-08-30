# solver 13 port conventions (BINDING for every module)

Reference: `benchmarks/reference-solvers/kissat-latest/src` (kissat 4.0.4,
built `gcc -O3 -DNDEBUG`, i.e. NDEBUG defined; LOGGING/METRICS/COVERAGE/
CHECKING/EMBEDDED/QUIET/NOPTIONS/NPROOFS **not** defined). Port exactly what
that build compiles: skip `#ifndef NDEBUG`-only and LOGGING/METRICS-only code;
KEEP code under `!defined(NPROOFS)` (proofs are in) and `#ifndef QUIET`
(profiles/reports are in). `LOG`/`LOGCLS`/`assert` → omit (or debug_assert).

## Goal

Faithful transliteration: same algorithms, same data layouts where they affect
trajectory (watch order, clause order, sort tie-breaks, PRNG, tick counting),
same option names/defaults, same statistics. Oracle: identical `-s` statistics
vs the reference binary at fixed `--conflicts=N` limits.

## Naming / layout

- One Rust module per C file, same stem: `analyze.c` → `src/analyze.rs`.
- Functions: drop the `kissat_` prefix, keep the rest verbatim:
  `kissat_sort_deduced` → `pub fn sort_deduced(...)` in its module.
  Static (file-local) C functions keep their names as private fns.
- All solver state lives in `pub struct Solver` (in `internal.rs`), fields
  named exactly as in `struct kissat` (C `import` field → `import_`, C
  `clause` field → `clause`, etc.; rename only Rust keywords with trailing
  underscore: `export` → `export_`, `struct` → n/a, `mode.last` fine).
- Functions take `solver: &mut Solver` as first arg (free functions, module-
  qualified), mirroring C call sites: `analyze::analyze(solver, conflict_ref)`.

## Types

- C `unsigned` → `u32`; `int` → `i32`; `uint64_t`/`size_t` counters → `u64`
  (use `usize` only for actual indexing); `double` → `f64`; `bool` → `bool`.
- `INVALID_LIT`/`INVALID_IDX`/`INVALID_REF` → `u32::MAX` (`INVALID` const).
  `INVALID_LEVEL = u32::MAX`, `MAX_ARENA` etc. per arena.h/reference.h.
- Literals: `lit = 2*idx + negated`, `NOT(lit) = lit ^ 1`, `IDX(lit) = lit>>1`,
  `SGN(lit) = lit & 1` — see literal.h. Externals are `i32`.
- `STACK(T)` → `Vec<T>`. kissat stack ops map: `PUSH_STACK`→`push`,
  `POP_STACK`→`pop().unwrap()`, `PEEK_STACK`→`v[i]`, `TOP_STACK`→`*v.last()`,
  `CLEAR_STACK`→`clear`, `SIZE_STACK`→`len`, `RESIZE_STACK(s,n)`→`truncate`.
  Do NOT replicate the C growth policy — capacity never affects semantics.
- `value` = `i8` (-1,0,1). `mark` = `i8`. `flags` bitfields → plain `bool`
  fields in a `Flags` struct (matches flags.h semantics).
- References into the arena: `type Reference = u32`.
- Watch words: vectors hold `u32` words exactly as kissat (`vector.h`); a
  watch is 1 word (binary: tagged lit) or 2 words (blocking-lit word + ref).
  Port `watch.h` unions as helper fns over u32 words — bit layout identical.

## Structural rules

- Keep C control flow recognizable: same loop structure, same early exits,
  same order of operations. Where C uses `goto`, restructure minimally
  (labeled break / small helper) without changing order of effects.
- Pointer iteration → index iteration. Where C caches `end` pointers and the
  container can move (arena during GC), mirror kissat's re-derivation points.
- Aliasing (e.g. propsearch mutates values while walking a watch list): use
  raw-slice unsafe or index juggling, but effects must occur in C order.
- Sorting: port `sort.c`/`rank.h` (radix sort with identical passes and
  stability) — never call std sort where kissat radix-sorts; tie-break order
  feeds schedules and hence trajectory.
- PRNG (`random.h`): port bit-exactly, same seeding, same call sites.
- Ticks: port `kissat_cache_lines` (utilities) exactly; increment ticks at
  exactly kissat's call sites. Ticks drive mode switching and effort limits.
- Statistics: `INC(name)`/`ADD(name,n)` → `solver.statistics.name += 1/n`.
  METRICS-only counters may be omitted (they are `#ifdef METRICS`).
- Options: `GET_OPTION(name)` → `solver.options.name` (i32). `GET1K` etc per
  options.h. Exact defaults/min/max from options.h OPTIONS table.
- Proof hooks: every `ADD_/DELETE_..._TO_PROOF`/`kissat_add_*`/`kissat_delete_*`
  proof call site must be preserved (proof.rs may stub early, but call sites
  stay so DRAT stays valid when enabled).
- `#ifndef QUIET` report/verbose code: keep (report.c prints drive nothing,
  but `-s` stats and `-v` reports are the parity oracle; profiles keep time
  accounting out of the search path).
- NO behavioral "improvements". If kissat has a quirk, port the quirk. Mark
  genuinely impossible-in-Rust spots with `// PORT NOTE:` comments.

## What NOT to port

- allocate.c (malloc wrappers) — use Vec. error.c → panic/eprintln helpers.
- attribute/cover/keatures/kommon headers — compiler glue.
- main.c/application.c/handle.c get a thin Rust equivalent in main.rs
  (kissat CLI: options, `<cnf> [proof]`, exit 10/20/0, signal alarm handling
  minimal). witness printing per witness.c with 4096-char v-lines.
- dump.c (debug), testing harness files.

## Process

- Modules land non-compiling while siblings are missing; integration and
  `mod` wiring happen in `main.rs`/`lib` pass by the integrator. Do not
  reorganize other modules to make yours compile.
- Every ported module starts with a header comment:
  `// Port of src/<file>.c (kissat 4.0.4).` plus any PORT NOTEs.
