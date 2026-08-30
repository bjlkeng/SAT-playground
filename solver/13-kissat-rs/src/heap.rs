// Port of src/heap.h + src/heap.c + src/inlineheap.h (kissat 4.0.4).
//
// Binary max-heap over variable indices with a score array and a position
// array.  Exact traversal/tie behavior of the C code is preserved: bubble-up
// stops on `score[parent] >= idx_score`, bubble-down prefers the sibling
// only on `sibling_score > child_score`, and stops on
// `child_score <= idx_score`.
//
// PORT NOTES:
//  - Per the porting convention for this cluster, all heap operations take
//    &mut Heap / &Heap directly (the C kissat* first argument is only used
//    for LOG and allocation, both irrelevant here).
//  - kissat_resize_heap: C nrealloc leaves the new tail of `pos`
//    uninitialized (kissat_enlarge_heap later memsets it to 0xff); the port
//    fills it with DISCONTAIN (the same 0xff bytes) immediately.  For an
//    untainted heap C deallocates and callocs `score` (fresh zeros) —
//    replicated by allocating a fresh zeroed Vec.
//  - kissat_release_heap: RELEASE_STACK + DEALLOC + memset 0 == reset to the
//    default (empty) value.
//  - kissat_check_heap / kissat_dump_heap are !NDEBUG only: not ported.

/// DISCONTAIN (heap.h).
pub const DISCONTAIN: u32 = u32::MAX;

/// DISCONTAINED (IDX): `(int) (IDX) < 0`.
#[inline]
pub fn discontained(pos: u32) -> bool {
    (pos as i32) < 0
}

// HEAP_CHILD / HEAP_PARENT.
#[inline]
fn heap_child(pos: u32) -> u32 {
    debug_assert!(pos < (1u32 << 31));
    2 * pos + 1
}

#[inline]
fn heap_parent(pos: u32) -> u32 {
    debug_assert!(pos > 0);
    (pos - 1) / 2
}

/// C `struct heap`.
#[derive(Default)]
pub struct Heap {
    pub tainted: bool,
    pub vars: u32,
    pub size: u32,
    pub stack: Vec<u32>,   // unsigneds stack
    pub score: Vec<f64>,   // double *score (len == size)
    pub pos: Vec<u32>,     // unsigned *pos  (len == size)
}

impl Heap {
    pub fn new() -> Self {
        Heap::default()
    }
}

/// kissat_heap_contains.
#[inline]
pub fn heap_contains(heap: &Heap, idx: u32) -> bool {
    idx < heap.vars && !discontained(heap.pos[idx as usize])
}

/// kissat_get_heap_pos.
#[inline]
pub fn get_heap_pos(heap: &Heap, idx: u32) -> u32 {
    if idx < heap.vars {
        heap.pos[idx as usize]
    } else {
        DISCONTAIN
    }
}

/// kissat_get_heap_score.
#[inline]
pub fn get_heap_score(heap: &Heap, idx: u32) -> f64 {
    if idx < heap.vars {
        heap.score[idx as usize]
    } else {
        0.0
    }
}

/// kissat_empty_heap.
#[inline]
pub fn empty_heap(heap: &Heap) -> bool {
    heap.stack.is_empty()
}

/// kissat_size_heap.
#[inline]
pub fn size_heap(heap: &Heap) -> usize {
    heap.stack.len()
}

/// kissat_max_heap.
#[inline]
pub fn max_heap(heap: &Heap) -> u32 {
    debug_assert!(!empty_heap(heap));
    heap.stack[0]
}

/// kissat_max_score_on_heap.
pub fn max_score_on_heap(heap: &Heap) -> f64 {
    if !heap.tainted {
        return 0.0;
    }
    debug_assert!(heap.vars > 0);
    let mut res = heap.score[0];
    for i in 1..heap.vars as usize {
        let s = heap.score[i];
        res = if res < s { s } else { res }; // MAX (res, *p)
    }
    res
}

/// kissat_release_heap.
pub fn release_heap(heap: &mut Heap) {
    *heap = Heap::default(); // RELEASE_STACK + DEALLOC + memset 0
}

/// kissat_resize_heap (grow-only).
pub fn resize_heap(heap: &mut Heap, new_size: u32) {
    let old_size = heap.size;
    if old_size >= new_size {
        return;
    }
    // pos: kissat_nrealloc keeps contents; new tail filled with DISCONTAIN
    // (C leaves it uninitialized until enlarge_heap's 0xff memset).
    heap.pos.resize(new_size as usize, DISCONTAIN);
    if heap.tainted {
        heap.score.resize(new_size as usize, 0.0); // nrealloc, keep scores
    } else {
        if old_size > 0 {
            // DEALLOC (heap->score, old_size);
        }
        heap.score = vec![0.0; new_size as usize]; // kissat_calloc
    }
    heap.size = new_size;
}

/// kissat_rescale_heap.
pub fn rescale_heap(heap: &mut Heap, factor: f64) {
    for i in 0..heap.vars as usize {
        heap.score[i] *= factor;
    }
}

/// kissat_enlarge_heap.
pub fn enlarge_heap(heap: &mut Heap, new_vars: u32) {
    let old_vars = heap.vars;
    debug_assert!(old_vars < new_vars);
    debug_assert!(new_vars <= heap.size);
    // memset (heap->pos + old_vars, 0xff, delta * sizeof (unsigned)):
    for p in &mut heap.pos[old_vars as usize..new_vars as usize] {
        *p = DISCONTAIN;
    }
    heap.vars = new_vars;
    if heap.tainted {
        for s in &mut heap.score[old_vars as usize..new_vars as usize] {
            *s = 0.0;
        }
    }
}

/// kissat_bubble_up (inlineheap.h).
pub fn bubble_up(heap: &mut Heap, idx: u32) {
    let mut idx_pos = heap.pos[idx as usize];
    let idx_score = heap.score[idx as usize];
    while idx_pos != 0 {
        let parent_pos = heap_parent(idx_pos);
        let parent = heap.stack[parent_pos as usize];
        if heap.score[parent as usize] >= idx_score {
            break;
        }
        heap.stack[idx_pos as usize] = parent;
        heap.pos[parent as usize] = idx_pos;
        idx_pos = parent_pos;
    }
    heap.stack[idx_pos as usize] = idx;
    heap.pos[idx as usize] = idx_pos;
}

/// kissat_bubble_down (inlineheap.h).
pub fn bubble_down(heap: &mut Heap, idx: u32) {
    let end = heap.stack.len() as u32;
    let mut idx_pos = heap.pos[idx as usize];
    let idx_score = heap.score[idx as usize];
    loop {
        let mut child_pos = heap_child(idx_pos);
        if child_pos >= end {
            break;
        }
        let mut child = heap.stack[child_pos as usize];
        let mut child_score = heap.score[child as usize];
        let sibling_pos = child_pos + 1;
        if sibling_pos < end {
            let sibling = heap.stack[sibling_pos as usize];
            let sibling_score = heap.score[sibling as usize];
            if sibling_score > child_score {
                child = sibling;
                child_pos = sibling_pos;
                child_score = sibling_score;
            }
        }
        if child_score <= idx_score {
            break;
        }
        heap.stack[idx_pos as usize] = child;
        heap.pos[child as usize] = idx_pos;
        idx_pos = child_pos;
    }
    heap.stack[idx_pos as usize] = idx;
    heap.pos[idx as usize] = idx_pos;
}

// HEAP_IMPORT (IDX).
#[inline]
fn heap_import(heap: &mut Heap, idx: u32) {
    debug_assert!(idx < u32::MAX - 1);
    if heap.vars <= idx {
        enlarge_heap(heap, idx + 1);
    }
}

/// kissat_push_heap.
pub fn push_heap(heap: &mut Heap, idx: u32) {
    debug_assert!(!heap_contains(heap, idx));
    heap_import(heap, idx);
    heap.pos[idx as usize] = heap.stack.len() as u32;
    heap.stack.push(idx);
    bubble_up(heap, idx);
}

/// kissat_pop_heap: remove `idx` (not necessarily the maximum).
pub fn pop_heap(heap: &mut Heap, idx: u32) {
    debug_assert!(heap_contains(heap, idx));
    let last = heap.stack.pop().unwrap(); // POP_STACK
    heap.pos[last as usize] = DISCONTAIN;
    if last == idx {
        return;
    }
    let idx_pos = heap.pos[idx as usize];
    heap.pos[idx as usize] = DISCONTAIN;
    heap.stack[idx_pos as usize] = last; // POKE_STACK
    heap.pos[last as usize] = idx_pos;
    bubble_up(heap, last);
    bubble_down(heap, last);
}

/// kissat_pop_max_heap.
pub fn pop_max_heap(heap: &mut Heap) -> u32 {
    debug_assert!(!empty_heap(heap));
    let idx = heap.stack[0];
    debug_assert!(heap.pos[idx as usize] == 0);
    let last = heap.stack.pop().unwrap();
    heap.pos[last as usize] = DISCONTAIN;
    if last == idx {
        return idx;
    }
    heap.pos[idx as usize] = DISCONTAIN;
    heap.stack[0] = last;
    heap.pos[last as usize] = 0;
    bubble_down(heap, last);
    idx
}

/// kissat_adjust_heap: make sure `idx` fits (resize by doubling, then
/// enlarge vars).
pub fn adjust_heap(heap: &mut Heap, idx: u32) {
    let new_vars = idx + 1;
    let old_vars = heap.vars;
    if new_vars <= old_vars {
        return;
    }
    let old_size = heap.size;
    if idx >= old_size {
        let mut new_size: u64 = if old_size != 0 { 2 * old_size as u64 } else { 1 };
        while idx as u64 >= new_size {
            new_size *= 2;
        }
        debug_assert!(new_size < DISCONTAIN as u64);
        resize_heap(heap, new_size as u32);
    }
    enlarge_heap(heap, idx + 1);
}

/// kissat_update_heap: set a new score and restore the heap property.
pub fn update_heap(heap: &mut Heap, idx: u32, new_score: f64) {
    let old_score = get_heap_score(heap, idx);
    if old_score == new_score {
        return;
    }
    heap_import(heap, idx);
    heap.score[idx as usize] = new_score;
    if !heap.tainted {
        heap.tainted = true;
    }
    if !heap_contains(heap, idx) {
        return;
    }
    if new_score > old_score {
        bubble_up(heap, idx);
    } else {
        bubble_down(heap, idx);
    }
}
