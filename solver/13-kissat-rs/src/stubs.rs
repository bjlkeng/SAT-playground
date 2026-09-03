// TEMPORARY stub modules for core-wave engines referenced by the foundation.
// Each `pub mod` here is re-exported as a crate module from main.rs and is
// replaced wholesale by its real port (src/<name>.rs) when its wave lands.

// restart / import / resize / decide / reduce stubs removed: real ports
// landed as src/restart.rs, src/import.rs, src/resize.rs, src/decide.rs,
// src/reduce.rs (decision/GC wave).

// compact / preprocess / reorder / walk / krite stubs removed: real ports
// landed as src/compact.rs, src/preprocess.rs, src/reorder.rs, src/walk.rs,
// src/krite.rs (walk/preprocess wave), plus src/dense.rs (dense.c — hard
// dependency of walk.c, ported by the same wave).

// --- stubs added by the search/application wave ------------------------

// lucky stub removed: real port landed as src/lucky.rs (lucky/proprobe wave).

// probe stub removed: real port landed as src/probe.rs (probing wave),
// together with src/backbone.rs, src/transitive.rs and src/vivify.rs.

// --- stubs added by the probing wave (probe.c call sites) ---------------

// congruence / factor stubs removed: real ports landed as src/congruence.rs
// and src/factor.rs (congruence/factor wave).

// substitute stub removed: real port landed as src/substitute.rs
// (eliminate/BVE wave).

// sweep stub removed: real port landed as src/sweep.rs (sweep/kitten wave).

// fastel / eliminate stubs removed: real ports landed as src/fastel.rs and
// src/eliminate.rs (eliminate/BVE wave), together with src/resolve.rs,
// src/gates.rs, src/ands.rs, src/equivalences.rs, src/ifthenelse.rs,
// src/definition.rs, src/propdense.rs, src/forward.rs and src/substitute.rs.

// --- stubs added by the conflict-analysis wave -------------------------

// proprobe stub removed: real port landed as src/proprobe.rs (lucky/proprobe
// wave).

// promote stub removed: real port landed as src/promote.rs (decision/GC
// wave; same exact bodies).

// strengthen stub removed: real port landed as src/strengthen.rs.
