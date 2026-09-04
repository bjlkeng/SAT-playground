// UVec<T>: a Vec<T> whose `[]` indexing is unchecked in release builds and
// debug-asserted otherwise.  kissat indexes its var-/lit-indexed arrays
// (values, marks, assigned, flags, links, watches, phases, the trail and
// the shared watch stack) through raw pointers with no bounds check under
// NDEBUG; the port indexes the same arrays through Vec `[]` at ~800 sites,
// and on cache-resident instances those checks were the bulk of a +27%
// branch overhead over the C (perf 2026-09-03).  Wrapping the fields keeps
// every call site's syntax and turns them all unchecked at once; the wrapper
// derefs to the Vec for everything else (len, resize, push, iter, ...).
//
// Range indexing (`&v[a..b]`) stays CHECKED: those sites are rare, cold, and
// a wrong range there would be a slice-length bug, not a per-element one.

use std::ops::{Deref, DerefMut, Index, IndexMut, Range, RangeFrom, RangeFull, RangeTo};

#[derive(Clone, Default, Debug)]
pub struct UVec<T>(pub Vec<T>);

impl<T> UVec<T> {
    #[inline]
    pub fn new() -> Self {
        UVec(Vec::new())
    }
}

impl<T> From<Vec<T>> for UVec<T> {
    #[inline]
    fn from(v: Vec<T>) -> Self {
        UVec(v)
    }
}

impl<T> Deref for UVec<T> {
    type Target = Vec<T>;
    #[inline(always)]
    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T> DerefMut for UVec<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.0
    }
}

impl<T> Index<usize> for UVec<T> {
    type Output = T;
    #[inline(always)]
    fn index(&self, i: usize) -> &T {
        debug_assert!(i < self.0.len(), "UVec index {} out of range {}", i, self.0.len());
        unsafe { self.0.get_unchecked(i) }
    }
}

impl<T> IndexMut<usize> for UVec<T> {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut T {
        debug_assert!(i < self.0.len(), "UVec index {} out of range {}", i, self.0.len());
        unsafe { self.0.get_unchecked_mut(i) }
    }
}

macro_rules! checked_range_index {
    ($($r:ty),*) => {$(
        impl<T> Index<$r> for UVec<T> {
            type Output = [T];
            #[inline(always)]
            fn index(&self, r: $r) -> &[T] {
                &self.0[r]
            }
        }
        impl<T> IndexMut<$r> for UVec<T> {
            #[inline(always)]
            fn index_mut(&mut self, r: $r) -> &mut [T] {
                &mut self.0[r]
            }
        }
    )*};
}
checked_range_index!(Range<usize>, RangeFrom<usize>, RangeTo<usize>, RangeFull);

impl<'a, T> IntoIterator for &'a UVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut UVec<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl<T> FromIterator<T> for UVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        UVec(Vec::from_iter(iter))
    }
}
