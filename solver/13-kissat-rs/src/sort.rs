// Port of src/sort.c + src/sort.h + src/rank.h (kissat 4.0.4).
//
// PORT NOTES:
//  - kissat_sort_literals exists twice in C: the plain version (linked for
//    clause.c) reads solver->values / solver->assigned, and the INLINE_SORT
//    variant (compiled into watch.c via `#define INLINE_SORT` + `#include
//    "sort.c"`) takes values/assigned explicitly.  `sort_literals` mirrors
//    the former, `sort_literals_inline` the latter; both share one body.
//  - SORT / QUICK_SORT / INSERTION_SORT / PARTITION (sort.h) and RADIX_SORT /
//    RADIX_STACK (rank.h) are C macros; here they are generic functions with
//    identical loop structure, pivot selection, explicit work stack
//    (solver->sorter passed as `sorter`), pass order and stability.
//  - The START/STOP (sort) and START/STOP (radix) profile hooks (both level
//    4) live inside the C macros.  The generics cannot take &mut Solver
//    (rank/less closures typically borrow solver fields), so callers with a
//    solver in scope must hoist the hooks around the call:
//      profile::start_checked(solver, Prof::radix);
//      ... radix_sort (...) ...
//      profile::stop_checked(solver, Prof::radix);
//    (vector.rs defrag_vectors does exactly this.)
//  - RADIX_SORT arithmetic is performed in u64 regardless of the C RTYPE;
//    for RTYPE == unsigned the executed passes are bit-identical because
//    i_radix never reaches bits beyond RadixRank::BITS and all rank values
//    fit the type's width.  SHIFT <<= 8 uses wrapping shift (C unsigned
//    wrap-around on the final, unused increment).
//  - The radix TMP buffer is allocated lazily on the first scatter pass (as
//    in C); its initial contents are irrelevant (fully overwritten), a copy
//    of the input is used to stay safe.

use crate::internal::{Assigned, Solver};
use crate::value::Value;

/*------------------------------------------------------------------------*/
// sort.c

// move_smallest_literal_to_front (static).
fn move_smallest_literal_to_front(
    values: &[Value],
    assigned: &[Assigned],
    satisfied_is_enough: bool,
    start: u32,
    size: u32,
    lits: &mut [u32],
) -> Value {
    debug_assert!(1 < size);
    debug_assert!(start < size);

    let a = lits[start as usize];

    let mut u = values[a as usize];
    if u == 0 || (u > 0 && satisfied_is_enough) {
        return u;
    }

    let mut pos: u32 = 0;
    let mut best = a;

    {
        let i = crate::literal::idx(a);
        let mut k = if u != 0 {
            assigned[i as usize].level
        } else {
            u32::MAX
        };

        for idx in (start + 1)..size {
            let b = lits[idx as usize];
            let v = values[b as usize];

            if v == 0 || (v > 0 && satisfied_is_enough) {
                best = b;
                pos = idx;
                u = v;
                break;
            }

            let j = crate::literal::idx(b);
            let l = if v != 0 {
                assigned[j as usize].level
            } else {
                u32::MAX
            };

            let better;
            if u < 0 && v > 0 {
                better = true;
            } else if u > 0 && v < 0 {
                better = false;
            } else if u < 0 {
                debug_assert!(v < 0);
                better = k < l;
            } else {
                debug_assert!(u > 0);
                debug_assert!(v > 0);
                debug_assert!(!satisfied_is_enough);
                better = k > l;
            }

            if !better {
                continue;
            }

            best = b;
            pos = idx;
            u = v;
            k = l;
        }
    }

    if pos == 0 {
        return u;
    }

    lits[start as usize] = best;
    lits[pos as usize] = a;

    u
}

/// kissat_sort_literals, INLINE_SORT variant (watch.c): explicit values and
/// assigned arrays.
pub fn sort_literals_inline(values: &[Value], assigned: &[Assigned], size: u32, lits: &mut [u32]) {
    let u = move_smallest_literal_to_front(values, assigned, false, 0, size, lits);
    if size > 2 {
        move_smallest_literal_to_front(values, assigned, u >= 0, 1, size, lits);
    }
}

/// kissat_sort_literals, plain variant (clause.c): reads solver->values and
/// solver->assigned.  `lits` must not alias solver storage (clause.rs takes
/// solver->clause out before calling).
pub fn sort_literals(solver: &Solver, size: u32, lits: &mut [u32]) {
    sort_literals_inline(&solver.values, &solver.assigned, size, lits);
}

/*------------------------------------------------------------------------*/
// sort.h — SORT / QUICK_SORT / INSERTION_SORT / PARTITION

pub const QUICK_SORT_LIMIT: usize = 10;

// GREATER_SWAP (TYPE, A[p], A[q], LESS)
#[inline]
fn greater_swap<T: Copy, F: Fn(&T, &T) -> bool>(a: &mut [T], p: usize, q: usize, less: &F) {
    if less(&a[q], &a[p]) {
        a.swap(p, q);
    }
}

fn quick_sort<T: Copy, F: Fn(&T, &T) -> bool>(sorter: &mut Vec<usize>, a: &mut [T], less: &F) {
    let n = a.len();
    debug_assert!(n != 0);
    debug_assert!(sorter.is_empty());

    let mut l: usize = 0;
    let mut r: usize = n - 1;

    if r - l <= QUICK_SORT_LIMIT {
        return;
    }

    loop {
        let m = l + (r - l) / 2;

        a.swap(m, r - 1);

        greater_swap(a, l, r - 1, less);
        greater_swap(a, l, r, less);
        greater_swap(a, r - 1, r, less);

        // PARTITION (TYPE, l + 1, r - 1, a, less):
        let i;
        {
            let l_partition = l + 1;
            let mut i_quick_sort = l_partition - 1;
            let mut j_partition = r - 1;
            let pivot = a[j_partition];
            loop {
                loop {
                    i_quick_sort += 1;
                    if !less(&a[i_quick_sort], &pivot) {
                        break;
                    }
                }
                // while (LESS (PIVOT, A[--J])) if (J == L) break;
                loop {
                    j_partition -= 1;
                    if !less(&pivot, &a[j_partition]) {
                        break;
                    }
                    if j_partition == l_partition {
                        break;
                    }
                }
                if i_quick_sort >= j_partition {
                    break;
                }
                a.swap(i_quick_sort, j_partition);
            }
            a.swap(i_quick_sort, r - 1); // SWAP (A[I], A[R]) with R == r - 1
            i = i_quick_sort;
        }
        debug_assert!(l < i);
        debug_assert!(i <= r);

        let ll;
        let rr;
        if i - l < r - i {
            ll = i + 1;
            rr = r;
            r = i - 1;
        } else {
            ll = l;
            rr = i - 1;
            l = i + 1;
        }
        if r - l > QUICK_SORT_LIMIT {
            debug_assert!(rr - ll > QUICK_SORT_LIMIT);
            sorter.push(ll);
            sorter.push(rr);
        } else if rr - ll > QUICK_SORT_LIMIT {
            l = ll;
            r = rr;
        } else if !sorter.is_empty() {
            r = sorter.pop().unwrap();
            l = sorter.pop().unwrap();
        } else {
            break;
        }
    }
}

fn insertion_sort<T: Copy, F: Fn(&T, &T) -> bool>(a: &mut [T], less: &F) {
    let n = a.len();
    let l: usize = 0;
    let r: usize = n - 1;

    let mut i = r;
    while i > l {
        greater_swap(a, i - 1, i, less);
        i -= 1;
    }

    let mut i = l + 2;
    while i <= r {
        let pivot = a[i];
        let mut j = i;
        while less(&pivot, &a[j - 1]) {
            a[j] = a[j - 1];
            j -= 1;
        }
        a[j] = pivot;
        i += 1;
    }
}

/// SORT (TYPE, N, A, LESS): kissat's quicksort (explicit work stack, median
/// pivot with sentinel swaps, QUICK_SORT_LIMIT 10) followed by insertion
/// sort.  `sorter` is solver->sorter (must be empty on entry, left empty).
/// START/STOP (sort) (level 4) hoisted to callers — see module PORT NOTES.
pub fn sort<T: Copy, F: Fn(&T, &T) -> bool>(sorter: &mut Vec<usize>, a: &mut [T], less: F) {
    let n = a.len();
    if n == 0 {
        return;
    }
    quick_sort(sorter, a, &less);
    insertion_sort(a, &less);
    debug_assert!(sorter.is_empty());
}

/// SORT_STACK (TYPE, S, LESS).
pub fn sort_stack<T: Copy, F: Fn(&T, &T) -> bool>(sorter: &mut Vec<usize>, s: &mut [T], less: F) {
    if s.len() <= 1 {
        return;
    }
    sort(sorter, s, less);
}

/*------------------------------------------------------------------------*/
// rank.h — RADIX_SORT / RADIX_STACK

/// The C RTYPE of a RADIX_SORT instantiation: an unsigned integer type
/// whose width bounds the number of 8-bit passes.
pub trait RadixRank: Copy {
    const BITS: u32;
    fn to_u64(self) -> u64;
}

impl RadixRank for u32 {
    const BITS: u32 = 32;
    #[inline]
    fn to_u64(self) -> u64 {
        self as u64
    }
}

impl RadixRank for u64 {
    const BITS: u32 = 64;
    #[inline]
    fn to_u64(self) -> u64 {
        self
    }
}

impl RadixRank for usize {
    const BITS: u32 = usize::BITS;
    #[inline]
    fn to_u64(self) -> u64 {
        self as u64
    }
}

// The scatter pass of RADIX_SORT (stable counting-sort step).
fn radix_scatter<V: Copy, R: RadixRank, F: Fn(&V) -> R>(
    src: &[V],
    dst: &mut [V],
    count: &mut [usize; 256],
    i_radix: u32,
    rank: &F,
) {
    for x in src {
        let r = rank(x).to_u64();
        let m = ((r >> i_radix) & 0xff) as usize;
        let pos = count[m];
        count[m] += 1;
        dst[pos] = *x;
    }
}

/// RADIX_SORT (VTYPE, RTYPE, N, V, RANK): stable LSB-first radix sort in
/// 8-bit passes with kissat's bounded-byte and already-sorted pass skips.
/// START/STOP (radix) (level 4) hoisted to callers — see module PORT NOTES.
pub fn radix_sort<V: Copy, R: RadixRank, F: Fn(&V) -> R>(v: &mut [V], rank: F) {
    let n_radix = v.len();
    if n_radix <= 1 {
        return;
    }

    const LENGTH_RADIX: u32 = 8;
    let mask_radix: u64 = (1u64 << LENGTH_RADIX) - 1;

    let mut count_radix = [0usize; 256];

    let mut tmp_radix: Vec<V> = Vec::new(); // allocated lazily (kissat_malloc)
    let mut tmp_allocated = false;

    let mut c_in_a = true; // C_RADIX == A_RADIX (v); else C_RADIX == B (tmp)

    let mut mlower_radix: usize = 0;
    let mut mupper_radix: usize = mask_radix as usize;

    let mut bounded_radix = false;
    let mut upper_radix: u64 = 0;
    let mut lower_radix: u64 = if R::BITS >= 64 {
        !0u64
    } else {
        (1u64 << R::BITS) - 1
    }; // ~UPPER of RTYPE
    let mut shift_radix: u64 = mask_radix;

    let mut i_radix: u32 = 0;
    while i_radix < R::BITS {
        'pass: {
            if bounded_radix && (lower_radix & shift_radix) == (upper_radix & shift_radix) {
                break 'pass;
            }

            // memset (COUNT + MLOWER, 0, (MUPPER - MLOWER + 1) * ...):
            for c in &mut count_radix[mlower_radix..=mupper_radix] {
                *c = 0;
            }

            let mut sorted_radix = true;
            let mut last_radix: u64 = 0;

            {
                let src: &[V] = if c_in_a { &*v } else { &tmp_radix };
                for x in src {
                    let r_radix = rank(x).to_u64();
                    if !bounded_radix {
                        lower_radix &= r_radix;
                        upper_radix |= r_radix;
                    }
                    let s_radix = r_radix >> i_radix;
                    let m_radix = s_radix & mask_radix;
                    if sorted_radix && last_radix > m_radix {
                        sorted_radix = false;
                    } else {
                        last_radix = m_radix;
                    }
                    count_radix[m_radix as usize] += 1;
                }
            }

            mlower_radix = ((lower_radix >> i_radix) & mask_radix) as usize;
            mupper_radix = ((upper_radix >> i_radix) & mask_radix) as usize;

            if !bounded_radix {
                bounded_radix = true;
                if (lower_radix & shift_radix) == (upper_radix & shift_radix) {
                    break 'pass;
                }
            }

            if sorted_radix {
                break 'pass;
            }

            let mut pos_radix: usize = 0;
            for j_radix in mlower_radix..=mupper_radix {
                let delta_radix = count_radix[j_radix];
                count_radix[j_radix] = pos_radix;
                pos_radix += delta_radix;
            }

            if !tmp_allocated {
                debug_assert!(c_in_a);
                tmp_radix.extend_from_slice(v); // contents fully overwritten
                tmp_allocated = true;
            }

            if c_in_a {
                radix_scatter(v, &mut tmp_radix, &mut count_radix, i_radix, &rank);
            } else {
                radix_scatter(&tmp_radix, v, &mut count_radix, i_radix, &rank);
            }
            c_in_a = !c_in_a; // C_RADIX = D_RADIX
        }
        i_radix += LENGTH_RADIX;
        shift_radix = shift_radix.wrapping_shl(LENGTH_RADIX); // C unsigned wrap
    }

    if !c_in_a {
        v.copy_from_slice(&tmp_radix); // memcpy (A, B, ...)
    }
    // kissat_free (TMP): drop.
}

/// RADIX_STACK (VTYPE, RTYPE, S, RANK).
pub fn radix_stack<V: Copy, R: RadixRank, F: Fn(&V) -> R>(s: &mut [V], rank: F) {
    radix_sort::<V, R, F>(s, rank);
}
