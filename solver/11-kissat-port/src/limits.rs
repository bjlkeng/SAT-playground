//! Limit-check boundary.
//!
//! Existing solver-10 code has scattered conflict/restart counters but no
//! unified limit object yet. Task 0.3 introduces parsed limit fields; later
//! tasks route conflict, propagation, tick, wall-clock, RSS, learned-lit,
//! binary-clause, extension-byte, and proof-byte checks through this module.
