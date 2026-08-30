// Port of src/profile.h + src/profile.c (kissat 4.0.4).
//
// PORT NOTE: C keeps `struct profiles` as 54 named `profile` fields and
// treats the struct as a `profile[]` array when printing, with a stack of
// `profile *` pointers. Rust represents the same layout as an array indexed
// by the `Prof` enum (variants in exact PROFS table order, C names verbatim)
// and a stack of `Prof` ids. `PROFILE (NAME)` -> `solver.profiles[Prof::NAME]`.
//
// PORT NOTE: the C macros START/STOP/STOP_SEARCH_AND_START_SIMPLIFIER/
// STOP_SIMPLIFIER_AND_RESUME_SEARCH (which wrap the calls in a
// GET_OPTION (profile) level check) are ported as the *_checked functions
// below; the bare functions mirror kissat_start/kissat_stop/... exactly.
//
// PORT NOTE: kissat_profiles_print uses INSERTION_SORT from sort.h; with
// size 0 the C macro underflows (`N - 1` on size_t) — that is unreachable
// in practice (search and simplify always qualify when profiling is on) and
// the Rust insertion sort below simply does nothing for size < 2.

use crate::internal::Solver;
use crate::resources;
use crate::format;

// PROFS table: PROF (name, level)
macro_rules! profs_table {
    ($m:ident) => {
        $m! {
            analyze: 3,
            backbone: 2,
            bump: 3,
            collect: 3,
            congruence: 2,
            decide: 4,
            deduce: 3,
            definition: 3,
            defrag: 3,
            dominate: 4,
            eliminate: 2,
            extend: 2,
            extract: 3,
            extractands: 3,
            extractbinaries: 3,
            extractites: 3,
            extractxors: 3,
            factor: 2,
            fastel: 2,
            focused: 2,
            forward: 4,
            lucky: 2,
            matching: 3,
            merge: 3,
            minimize: 3,
            parse: 1,
            preprocess: 2,
            probe: 2,
            propagate: 4,
            radix: 4,
            reduce: 2,
            reorder: 3,
            rephase: 3,
            restart: 3,
            search: 1,
            shrink: 3,
            simplify: 1,
            sort: 4,
            stable: 2,
            substitute: 2,
            subsume: 2,
            sweep: 2,
            sweepbackbone: 3,
            sweepequivalences: 3,
            total: 0,
            transitive: 2,
            vivify: 2,
            vivify0: 3,
            vivify1: 3,
            vivify2: 3,
            vivify3: 3,
            vivifysort: 4,
            walking: 2,
            warmup: 3,
        }
    };
}

macro_rules! define_profs {
    ($($name:ident: $level:expr,)*) => {
        /// Identifies one profile; variant order == PROFS table order.
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(usize)]
        pub enum Prof {
            $($name,)*
        }

        /// (name, level) in PROFS table order.
        pub const PROFS: &[(&str, i32)] = &[$((stringify!($name), $level),)*];
    };
}
profs_table!(define_profs);

pub const SIZE_PROFS: usize = PROFS.len(); // 54

#[derive(Clone, Copy, Default)]
pub struct Profile {
    pub level: i32,
    pub name: &'static str,
    pub entered: f64,
    pub time: f64,
}

pub struct Profiles {
    pub profs: [Profile; SIZE_PROFS],
    pub stack: Vec<Prof>,
}

impl std::ops::Index<Prof> for Profiles {
    type Output = Profile;
    fn index(&self, p: Prof) -> &Profile {
        &self.profs[p as usize]
    }
}

impl std::ops::IndexMut<Prof> for Profiles {
    fn index_mut(&mut self, p: Prof) -> &mut Profile {
        &mut self.profs[p as usize]
    }
}

impl Default for Profiles {
    fn default() -> Self {
        let mut res = Profiles {
            profs: [Profile::default(); SIZE_PROFS],
            stack: Vec::new(),
        };
        init_profiles(&mut res);
        res
    }
}

// kissat_init_profiles
pub fn init_profiles(profiles: &mut Profiles) {
    for (i, &(name, level)) in PROFS.iter().enumerate() {
        profiles.profs[i] = Profile {
            level,
            name,
            entered: 0.0,
            time: 0.0,
        };
    }
    profiles.stack.clear();
}

// static bool less_profile (profile *p, profile *q)
fn less_profile(p: &Profile, q: &Profile) -> bool {
    if p.time > q.time {
        return true;
    }
    if p.time < q.time {
        return false;
    }
    p.name < q.name // strcmp (p->name, q->name) < 0
}

// static void print_profile (kissat *solver, profile *p, double total)
fn print_profile(prefix: &str, p: &Profile, total: f64) {
    println!(
        "{}{:>14.2} {:>7.2} %  {}",
        prefix,
        p.time,
        format::percent(p.time, total),
        p.name
    );
}

// static double flush_profile (profile *profile, double now)
fn flush_profile(profile: &mut Profile, now: f64) -> f64 {
    let delta = now - profile.entered;
    profile.time += delta;
    profile.entered = now;
    delta
}

// static void flush_profiles (profiles *profiles, const double now)
fn flush_profiles(profiles: &mut Profiles, now: f64) {
    for i in 0..profiles.stack.len() {
        let p = profiles.stack[i];
        flush_profile(&mut profiles.profs[p as usize], now);
    }
}

// static void push_profile (kissat *solver, profile *profile, double now)
fn push_profile(profiles: &mut Profiles, prof: Prof, now: f64) {
    profiles[prof].entered = now;
    profiles.stack.push(prof);
}

// INSERTION_SORT (profile *, size, sorted, less_profile) from sort.h,
// ported with the same two phases (sentinel bubble pass + insertion).
fn insertion_sort_profiles(profs: &[Profile; SIZE_PROFS], sorted: &mut [usize]) {
    let n = sorted.len();
    if n < 2 {
        return;
    }
    let less = |p: usize, q: usize| less_profile(&profs[p], &profs[q]);
    let l = 0usize;
    let r = n - 1;
    let mut i = r;
    while i > l {
        // GREATER_SWAP: if (LESS (B, A)) swap
        if less(sorted[i], sorted[i - 1]) {
            sorted.swap(i - 1, i);
        }
        i -= 1;
    }
    for i in (l + 2)..=r {
        let pivot = sorted[i];
        let mut j = i;
        while less(pivot, sorted[j - 1]) {
            sorted[j] = sorted[j - 1];
            j -= 1;
        }
        sorted[j] = pivot;
    }
}

// kissat_profiles_print
pub fn profiles_print(solver: &mut Solver) {
    let now = resources::process_time();
    flush_profiles(&mut solver.profiles, now);
    let named = &solver.profiles;
    let mut sorted: Vec<usize> = Vec::with_capacity(SIZE_PROFS);
    for (i, p) in named.profs.iter().enumerate() {
        if p.level <= solver.options.profile
            && (i == Prof::search as usize
                || i == Prof::simplify as usize
                || (i != Prof::total as usize && p.time != 0.0))
        {
            sorted.push(i);
        }
    }
    insertion_sort_profiles(&named.profs, &mut sorted);
    let total = named.profs[Prof::total as usize].time;
    let prefix: &str = &solver.prefix;
    for &i in &sorted {
        print_profile(prefix, &named.profs[i], total);
    }
    println!("{}=============================================", prefix);
    print_profile(prefix, &named.profs[Prof::total as usize], total);
}

// kissat_start
pub fn start(solver: &mut Solver, prof: Prof) {
    let now = resources::process_time();
    push_profile(&mut solver.profiles, prof, now);
}

// kissat_stop
pub fn stop(solver: &mut Solver, prof: Prof) {
    debug_assert_eq!(*solver.profiles.stack.last().unwrap(), prof);
    solver.profiles.stack.pop().unwrap();
    let now = resources::process_time();
    flush_profile(&mut solver.profiles[prof], now);
}

// kissat_stop_search_and_start_simplifier
pub fn stop_search_and_start_simplifier(solver: &mut Solver, prof: Prof) {
    debug_assert!(solver.profiles[Prof::search].level <= solver.options.profile);
    let now = resources::process_time();
    while *solver.profiles.stack.last().unwrap() != Prof::search {
        let mode = solver.profiles.stack.pop().unwrap();
        debug_assert!(solver.profiles[Prof::search].level <= solver.profiles[mode].level);
        debug_assert!(if solver.stable {
            mode == Prof::stable
        } else {
            mode == Prof::focused
        });
        flush_profile(&mut solver.profiles[mode], now);
    }
    solver.profiles.stack.pop().unwrap(); // pops `search`
    debug_assert_eq!(
        solver.profiles[Prof::search].level,
        solver.profiles[Prof::simplify].level
    );
    debug_assert!(solver.profiles[Prof::simplify].level <= solver.profiles[prof].level);
    flush_profile(&mut solver.profiles[Prof::search], now);
    push_profile(&mut solver.profiles, Prof::simplify, now);
    if solver.profiles[prof].level <= solver.options.profile {
        push_profile(&mut solver.profiles, prof, now);
    }
}

// kissat_stop_simplifier_and_resume_search
pub fn stop_simplifier_and_resume_search(solver: &mut Solver, prof: Prof) {
    let top = solver.profiles.stack.pop().unwrap();
    let now = resources::process_time();
    let delta = flush_profile(&mut solver.profiles[Prof::simplify], now);
    solver.mode.entered += delta;
    if top == prof {
        flush_profile(&mut solver.profiles[prof], now);
        debug_assert_eq!(*solver.profiles.stack.last().unwrap(), Prof::simplify);
        solver.profiles.stack.pop().unwrap();
    } else {
        debug_assert_eq!(top, Prof::simplify);
        debug_assert!(solver.profiles[prof].level > solver.options.profile);
    }
    debug_assert!(solver.profiles[Prof::simplify].level <= solver.profiles[prof].level);
    push_profile(&mut solver.profiles, Prof::search, now);
    let mode = if solver.stable {
        Prof::stable
    } else {
        Prof::focused
    };
    debug_assert!(solver.profiles[Prof::search].level <= solver.profiles[mode].level);
    if solver.profiles[mode].level <= solver.options.profile {
        push_profile(&mut solver.profiles, mode, now);
    }
}

// kissat_time
pub fn time(solver: &mut Solver) -> f64 {
    let now = resources::process_time();
    flush_profiles(&mut solver.profiles, now);
    solver.profiles[Prof::total].time
}

/*------------------------------------------------------------------------*/
// The profile.h call-site macros (option-level gated):

// #define START(NAME)
pub fn start_checked(solver: &mut Solver, prof: Prof) {
    if solver.options.profile >= solver.profiles[prof].level {
        start(solver, prof);
    }
}

// #define STOP(NAME)
pub fn stop_checked(solver: &mut Solver, prof: Prof) {
    if solver.options.profile >= solver.profiles[prof].level {
        stop(solver, prof);
    }
}

// #define STOP_SEARCH_AND_START_SIMPLIFIER(NAME)
pub fn stop_search_and_start_simplifier_checked(solver: &mut Solver, prof: Prof) {
    if solver.options.profile >= solver.profiles[Prof::search].level {
        stop_search_and_start_simplifier(solver, prof);
    }
}

// #define STOP_SIMPLIFIER_AND_RESUME_SEARCH(NAME)
pub fn stop_simplifier_and_resume_search_checked(solver: &mut Solver, prof: Prof) {
    if solver.options.profile >= solver.profiles[Prof::search].level {
        stop_simplifier_and_resume_search(solver, prof);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profs_table_complete() {
        assert_eq!(SIZE_PROFS, 54);
        assert_eq!(Prof::analyze as usize, 0);
        assert_eq!(Prof::total as usize, 44);
        assert_eq!(Prof::warmup as usize, 53);
        assert_eq!(PROFS[Prof::search as usize], ("search", 1));
        assert_eq!(PROFS[Prof::simplify as usize], ("simplify", 1));
        assert_eq!(PROFS[Prof::total as usize], ("total", 0));
        assert_eq!(PROFS[Prof::propagate as usize], ("propagate", 4));
    }

    #[test]
    fn sort_matches_less_profile_order() {
        let mut profiles = Profiles::default();
        profiles.profs[Prof::search as usize].time = 5.0;
        profiles.profs[Prof::simplify as usize].time = 5.0;
        profiles.profs[Prof::walking as usize].time = 7.0;
        let mut sorted = vec![
            Prof::simplify as usize,
            Prof::walking as usize,
            Prof::search as usize,
        ];
        insertion_sort_profiles(&profiles.profs, &mut sorted);
        // time descending, then name ascending on ties
        assert_eq!(
            sorted,
            vec![
                Prof::walking as usize,
                Prof::search as usize,
                Prof::simplify as usize
            ]
        );
    }
}
