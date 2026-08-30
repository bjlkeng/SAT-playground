// Port of src/queue.h + src/queue.c (kissat 4.0.4).
//
// VMTF doubly-linked queue over variable indices: solver.links[idx] holds
// prev/next/stamp, solver.queue holds first/last/stamp plus the last-search
// cache (search.idx/search.stamp).  The inline operations of inlinequeue.h
// (enqueue, dequeue, move_to_front, ...) live in inlinequeue.rs.
//
// PORT NOTES:
//  - kissat_check_queue is CHECK_QUEUE/!NDEBUG only: not ported.
//  - queue.h's LINK (IDX) macro is plain `solver.links[idx]` indexing here.

use crate::internal::Solver;

/// DISCONNECT (queue.h).
pub const DISCONNECT: u32 = u32::MAX;

/// DISCONNECTED (IDX): `(int) (IDX) < 0`.
#[inline]
pub fn disconnected(idx: u32) -> bool {
    (idx as i32) < 0
}

/// C `struct links`.
#[derive(Clone, Copy, Default)]
pub struct Links {
    pub prev: u32,
    pub next: u32,
    pub stamp: u32,
}

/// C `struct queue`'s anonymous `search` member.
#[derive(Clone, Copy, Default)]
pub struct Search {
    pub idx: u32,
    pub stamp: u32,
}

/// C `struct queue`.
#[derive(Clone, Copy, Default)]
pub struct Queue {
    pub first: u32,
    pub last: u32,
    pub stamp: u32,
    pub search: Search,
}

/// kissat_init_queue.
pub fn init_queue(solver: &mut Solver) {
    let queue = &mut solver.queue;
    queue.first = DISCONNECT;
    queue.last = DISCONNECT;
    debug_assert!(queue.stamp == 0);
    queue.search.idx = DISCONNECT;
    debug_assert!(queue.search.stamp == 0);
}

/// kissat_reset_search_of_queue.
pub fn reset_search_of_queue(solver: &mut Solver) {
    let last = solver.queue.last;
    debug_assert!(!disconnected(last));
    crate::inlinequeue::update_queue(solver, last);
}

/// kissat_reassign_queue_stamps: renumber all stamps 1..n in queue order
/// (called when the enqueue stamp would wrap).
pub fn reassign_queue_stamps(solver: &mut Solver) {
    crate::print::very_verbose(
        solver,
        format_args!("need to reassign enqueue stamps on queue"),
    );

    solver.queue.stamp = 0;

    let mut idx = solver.queue.first;
    while !disconnected(idx) {
        solver.queue.stamp += 1;
        let l = &mut solver.links[idx as usize];
        l.stamp = solver.queue.stamp;
        idx = l.next;
    }

    if !disconnected(solver.queue.search.idx) {
        solver.queue.search.stamp = solver.links[solver.queue.search.idx as usize].stamp;
    }
}
