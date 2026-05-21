//! Branching data structures that are safe to grow independently from the
//! monolithic solver while the solver 11 branch module boundary is staged.

#[derive(Clone, Debug)]
pub(crate) struct VmtfQueue {
    next: Vec<u32>,
    prev: Vec<u32>,
    head: u32,
    search: u32,
    stamp: Vec<u64>,
    stamp_counter: u64,
}

impl VmtfQueue {
    pub(crate) fn new(num_vars: usize, branch_order: &[u32]) -> Self {
        let mut queue = Self {
            next: vec![0; num_vars + 1],
            prev: vec![0; num_vars + 1],
            head: 0,
            search: 0,
            stamp: vec![0; num_vars + 1],
            stamp_counter: 0,
        };

        for &var in branch_order {
            queue.insert_new_head(var as usize);
        }
        queue.search = queue.head;
        queue
    }

    pub(crate) fn stamp_and_move_to_front(&mut self, var: usize) {
        if !self.valid_var(var) {
            return;
        }

        self.stamp_counter = self.stamp_counter.saturating_add(1);
        self.stamp[var] = self.stamp_counter;
        if self.head as usize == var {
            return;
        }

        self.detach(var);
        self.attach_head(var);
    }

    pub(crate) fn note_unassigned(&mut self, var: usize) {
        if !self.valid_var(var) {
            return;
        }

        let search = self.search as usize;
        if search == 0 || self.stamp[var] > self.stamp[search] {
            self.search = var as u32;
        }
    }

    pub(crate) fn reset_search_to_head(&mut self) {
        self.search = self.head;
    }

    pub(crate) fn pick<F>(&mut self, mut eligible: F) -> Option<usize>
    where
        F: FnMut(usize) -> bool,
    {
        let mut var = if self.search == 0 {
            self.head as usize
        } else {
            self.search as usize
        };

        while var != 0 {
            if eligible(var) {
                self.search = var as u32;
                return Some(var);
            }
            var = self.prev[var] as usize;
        }

        self.search = 0;
        None
    }

    fn insert_new_head(&mut self, var: usize) {
        debug_assert!(self.valid_var(var));
        debug_assert_eq!(self.next[var], 0);
        debug_assert_eq!(self.prev[var], 0);
        self.stamp_counter = self.stamp_counter.saturating_add(1);
        self.stamp[var] = self.stamp_counter;
        self.attach_head(var);
    }

    fn attach_head(&mut self, var: usize) {
        let old_head = self.head as usize;
        self.prev[var] = old_head as u32;
        self.next[var] = 0;
        if old_head != 0 {
            self.next[old_head] = var as u32;
        }
        self.head = var as u32;
    }

    fn detach(&mut self, var: usize) {
        let older = self.prev[var] as usize;
        let newer = self.next[var] as usize;

        if older != 0 {
            self.next[older] = newer as u32;
        }
        if newer != 0 {
            self.prev[newer] = older as u32;
        } else if self.head as usize == var {
            self.head = older as u32;
        }

        self.prev[var] = 0;
        self.next[var] = 0;
    }

    fn valid_var(&self, var: usize) -> bool {
        var > 0 && var < self.stamp.len()
    }

    #[cfg(test)]
    pub(crate) fn search_for_test(&self) -> usize {
        self.search as usize
    }

    #[cfg(test)]
    pub(crate) fn stamp_for_test(&self, var: usize) -> u64 {
        self.stamp[var]
    }
}
