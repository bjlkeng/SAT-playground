//! Minimal FxHash (the rustc-hash algorithm): a fast, non-cryptographic hasher for
//! short structured keys (literal tuples, small literal vectors). Used by the gate
//! congruence extraction/closure path, where std's SipHash dominates the wall of
//! every closure round (ibm root closure: 1.3M gates hashed per round, ~4.8s/round
//! measured 2026-07-18). Determinism note: unlike std's RandomState (randomly
//! seeded per process), this hasher is FIXED-seed, so hash-map iteration order is
//! deterministic across runs — a strict subset of the run-to-run variation the
//! solver already tolerates (outcomes are byte-reproducible across processes with
//! random SipHash seeds; the congruence pipeline is insensitive to gate hash
//! order, verified on the armed-cell identity screens).
//!
//! Algorithm (public domain / MIT-Apache dual per rustc-hash): per machine word
//! `hash = (hash.rotate_left(5) ^ word).wrapping_mul(K)` with K = 0x51_7c_c1_b7_27_22_0a_95.

use std::hash::{BuildHasherDefault, Hasher};

const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Default)]
pub(crate) struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline(always)]
    fn add_to_hash(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let mut chunk = [0u8; 8];
            chunk.copy_from_slice(&bytes[..8]);
            self.add_to_hash(u64::from_le_bytes(chunk));
            bytes = &bytes[8..];
        }
        if bytes.len() >= 4 {
            let mut chunk = [0u8; 4];
            chunk.copy_from_slice(&bytes[..4]);
            self.add_to_hash(u64::from(u32::from_le_bytes(chunk)));
            bytes = &bytes[4..];
        }
        for &b in bytes {
            self.add_to_hash(u64::from(b));
        }
    }

    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(u64::from(i));
    }

    #[inline(always)]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(u64::from(i));
    }

    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash(u64::from(i));
    }

    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }

    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.add_to_hash(i as u64);
    }

    #[inline(always)]
    fn write_i32(&mut self, i: i32) {
        self.add_to_hash(i as u32 as u64);
    }

    #[inline(always)]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub(crate) type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub(crate) type FxHashMap<K, V> = std::collections::HashMap<K, V, FxBuildHasher>;
pub(crate) type FxHashSet<T> = std::collections::HashSet<T, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_across_instances() {
        let mut a = FxHashMap::<(u8, Vec<i32>), i32>::default();
        let mut b = FxHashMap::<(u8, Vec<i32>), i32>::default();
        for i in 0..1000i32 {
            a.insert((0, vec![i, -i, i * 3]), i);
            b.insert((0, vec![i, -i, i * 3]), i);
        }
        let ka: Vec<_> = a.keys().cloned().collect();
        let kb: Vec<_> = b.keys().cloned().collect();
        assert_eq!(ka, kb, "fixed-seed hasher must iterate identically");
    }

    #[test]
    fn set_membership_matches_std() {
        let mut fx = FxHashSet::<(i32, i32)>::default();
        let mut std_set = std::collections::HashSet::new();
        for i in 0..1000i32 {
            fx.insert((i, -i));
            std_set.insert((i, -i));
        }
        for i in -100..1100i32 {
            assert_eq!(fx.contains(&(i, -i)), std_set.contains(&(i, -i)));
        }
    }
}
