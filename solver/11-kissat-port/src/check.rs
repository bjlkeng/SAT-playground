//! Invariant and debug-handle boundary.
//!
//! Solver 11 will introduce typed clause and binary references incrementally.
//! This debug-only table is the 0.1 scaffold for rejecting stale handles after
//! GC/rebuild style moves without wiring those handles into hot release paths.

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DebugClauseRef {
    slot: usize,
    generation: u32,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DebugBinaryClauseId {
    slot: usize,
    generation: u32,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebugGenerationError {
    UnknownSlot {
        slot: usize,
    },
    DeletedSlot {
        slot: usize,
    },
    StaleGeneration {
        slot: usize,
        expected: u32,
        actual: u32,
    },
}

#[cfg(any(test, debug_assertions))]
#[derive(Default)]
pub(crate) struct DebugGenerationTable {
    generations: Vec<u32>,
    live: Vec<bool>,
}

#[cfg(any(test, debug_assertions))]
impl DebugGenerationTable {
    pub(crate) fn allocate_clause_slot(&mut self) -> DebugClauseRef {
        let (slot, generation) = self.allocate_slot();
        DebugClauseRef { slot, generation }
    }

    pub(crate) fn allocate_binary_slot(&mut self) -> DebugBinaryClauseId {
        let (slot, generation) = self.allocate_slot();
        DebugBinaryClauseId { slot, generation }
    }

    pub(crate) fn delete_clause_slot(&mut self, handle: DebugClauseRef) {
        self.delete_slot(handle.slot);
    }

    pub(crate) fn delete_binary_slot(&mut self, handle: DebugBinaryClauseId) {
        self.delete_slot(handle.slot);
    }

    pub(crate) fn move_clause_slot(
        &mut self,
        handle: DebugClauseRef,
        new_slot: usize,
    ) -> DebugClauseRef {
        self.validate_clause_ref(handle)
            .expect("cannot move stale debug clause ref");
        self.delete_slot(handle.slot);
        let generation = self.activate_slot(new_slot);
        DebugClauseRef {
            slot: new_slot,
            generation,
        }
    }

    pub(crate) fn rewrite_clause_refs(
        &self,
        refs: &mut [DebugClauseRef],
        old: DebugClauseRef,
        new: DebugClauseRef,
    ) {
        for handle in refs {
            if *handle == old {
                *handle = new;
            }
        }
    }

    pub(crate) fn validate_clause_ref(
        &self,
        handle: DebugClauseRef,
    ) -> Result<(), DebugGenerationError> {
        self.validate_slot(handle.slot, handle.generation)
    }

    pub(crate) fn validate_binary_id(
        &self,
        handle: DebugBinaryClauseId,
    ) -> Result<(), DebugGenerationError> {
        self.validate_slot(handle.slot, handle.generation)
    }

    fn allocate_slot(&mut self) -> (usize, u32) {
        let slot = self.generations.len();
        self.generations.push(1);
        self.live.push(true);
        (slot, 1)
    }

    fn activate_slot(&mut self, slot: usize) -> u32 {
        if self.generations.len() <= slot {
            self.generations.resize(slot + 1, 0);
            self.live.resize(slot + 1, false);
        }
        let generation = self.generations[slot].wrapping_add(1).max(1);
        self.generations[slot] = generation;
        self.live[slot] = true;
        generation
    }

    fn delete_slot(&mut self, slot: usize) {
        if slot < self.live.len() {
            self.live[slot] = false;
        }
    }

    fn validate_slot(&self, slot: usize, generation: u32) -> Result<(), DebugGenerationError> {
        let Some(&actual) = self.generations.get(slot) else {
            return Err(DebugGenerationError::UnknownSlot { slot });
        };
        if !self.live.get(slot).copied().unwrap_or(false) {
            return Err(DebugGenerationError::DeletedSlot { slot });
        }
        if actual != generation {
            return Err(DebugGenerationError::StaleGeneration {
                slot,
                expected: generation,
                actual,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_clause_ref_generation_rejects_deleted_slot() {
        let mut table = DebugGenerationTable::default();
        let handle = table.allocate_clause_slot();

        table.delete_clause_slot(handle);

        assert_eq!(
            table.validate_clause_ref(handle),
            Err(DebugGenerationError::DeletedSlot { slot: 0 })
        );
    }

    #[test]
    fn test_gc_rewrite_visitor_updates_all_registered_clause_refs() {
        let mut table = DebugGenerationTable::default();
        let old = table.allocate_clause_slot();
        let mut registered_refs = vec![old, old];

        let new = table.move_clause_slot(old, 7);
        table.rewrite_clause_refs(&mut registered_refs, old, new);

        assert_eq!(registered_refs, vec![new, new]);
        assert!(table.validate_clause_ref(new).is_ok());
        assert_eq!(
            table.validate_clause_ref(old),
            Err(DebugGenerationError::DeletedSlot { slot: 0 })
        );
    }

    #[test]
    fn test_binary_clause_id_generation_rejects_deleted_binary_in_debug() {
        let mut table = DebugGenerationTable::default();
        let handle = table.allocate_binary_slot();

        table.delete_binary_slot(handle);

        assert_eq!(
            table.validate_binary_id(handle),
            Err(DebugGenerationError::DeletedSlot { slot: 0 })
        );
    }
}
