// Port of src/inlinequeue.h (kissat 4.0.4).
//
// VMTF queue operations over solver.links / solver.queue.  See queue.rs for
// the Links/Queue types and queue.c functions.
//
// PORT NOTES:
//  - kissat_update_queue takes (solver, links, idx) in C with links always
//    == solver->links; the port takes (solver, idx).  Likewise
//    kissat_enqueue_links(solver, i, links, queue) becomes (solver, i)
//    (it can call reassign_queue_stamps, which needs the solver).
//    kissat_dequeue_links only touches links + queue and keeps the C shape
//    (callable under split borrows).
//  - VALUE (LIT (idx)) is solver.values[crate::literal::lit(idx)].

use crate::internal::Solver;
use crate::queue::{disconnected, Links, Queue, DISCONNECT};

/// kissat_update_queue: cache `idx` (with its stamp) as the search position.
#[inline]
pub fn update_queue(solver: &mut Solver, idx: u32) {
    debug_assert!(!disconnected(idx));
    let stamp = solver.links[idx as usize].stamp;
    solver.queue.search.idx = idx;
    solver.queue.search.stamp = stamp;
}

/// kissat_enqueue_links: append `i` at the tail and stamp it (reassigning
/// all stamps first if the stamp counter would wrap).
pub fn enqueue_links(solver: &mut Solver, i: u32) {
    debug_assert!(disconnected(solver.links[i as usize].prev));
    debug_assert!(disconnected(solver.links[i as usize].next));
    let j = solver.queue.last; // const unsigned j = p->prev = queue->last
    solver.links[i as usize].prev = j;
    solver.queue.last = i;
    if disconnected(j) {
        solver.queue.first = i;
    } else {
        let l = &mut solver.links[j as usize];
        debug_assert!(disconnected(l.next));
        l.next = i;
    }
    if solver.queue.stamp == u32::MAX {
        crate::queue::reassign_queue_stamps(solver);
        debug_assert!(solver.links[i as usize].stamp == solver.queue.stamp);
    } else {
        solver.queue.stamp += 1; // p->stamp = ++queue->stamp
        solver.links[i as usize].stamp = solver.queue.stamp;
    }
}

/// kissat_dequeue_links: unlink `i` (C signature preserved: links + queue
/// only, usable under split borrows).
pub fn dequeue_links(i: u32, links: &mut [Links], queue: &mut Queue) {
    let (j, k) = {
        let l = &mut links[i as usize];
        let j = l.prev;
        let k = l.next;
        l.prev = DISCONNECT;
        l.next = DISCONNECT;
        (j, k)
    };
    if disconnected(j) {
        debug_assert!(queue.first == i);
        queue.first = k;
    } else {
        let p = &mut links[j as usize];
        debug_assert!(p.next == i);
        p.next = k;
    }
    if disconnected(k) {
        debug_assert!(queue.last == i);
        queue.last = j;
    } else {
        let n = &mut links[k as usize];
        debug_assert!(n.prev == i);
        n.prev = j;
    }
}

/// kissat_enqueue.
pub fn enqueue(solver: &mut Solver, idx: u32) {
    debug_assert!(idx < solver.vars);
    {
        let l = &mut solver.links[idx as usize];
        l.prev = DISCONNECT;
        l.next = DISCONNECT;
    }
    enqueue_links(solver, idx);
    if solver.values[crate::literal::lit(idx) as usize] == 0 {
        // !VALUE (LIT (idx))
        update_queue(solver, idx);
    }
}

/// kissat_dequeue.
pub fn dequeue(solver: &mut Solver, idx: u32) {
    debug_assert!(idx < solver.vars);
    if solver.queue.search.idx == idx {
        let l = solver.links[idx as usize];
        let mut search = l.next;
        if search == DISCONNECT {
            search = l.prev;
        }
        if search == DISCONNECT {
            solver.queue.search.idx = DISCONNECT;
            solver.queue.search.stamp = 0;
        } else {
            update_queue(solver, search);
        }
    }
    dequeue_links(idx, &mut solver.links, &mut solver.queue);
}

/// kissat_move_to_front.
pub fn move_to_front(solver: &mut Solver, idx: u32) {
    if idx == solver.queue.last {
        debug_assert!(disconnected(solver.links[idx as usize].next));
        return;
    }
    debug_assert!(idx < solver.vars);
    let tmp = solver.values[crate::literal::lit(idx) as usize]; // VALUE (LIT (idx))
    if tmp != 0 && solver.queue.search.idx == idx {
        let prev = solver.links[idx as usize].prev;
        if !disconnected(prev) {
            update_queue(solver, prev);
        } else {
            let next = solver.links[idx as usize].next;
            debug_assert!(!disconnected(next));
            update_queue(solver, next);
        }
    }
    dequeue_links(idx, &mut solver.links, &mut solver.queue);
    enqueue_links(solver, idx);
    if tmp == 0 {
        update_queue(solver, idx);
    }
}
