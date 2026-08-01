//! Pigeonhole-counting extended-resolution refutation (SAT_PHP_REFUTE).
//!
//! Detects two relativized-pigeonhole clause shapes that are exponentially hard
//! for CDCL/resolution yet admit polynomial extended-resolution refutations:
//!
//! 1. Relativized PHP (the sat-comp `rphp` family): P pigeons each choose one
//!    of N resting places (P var-disjoint cover clauses), at most one pigeon
//!    per place (per-place binary cliques), an occupied place is "used"
//!    (`pigeon@place -> used(r)` binaries), a used place maps to one of H
//!    shared holes (`{-used(r)} ∪ holes(r)` clauses), and two used places
//!    cannot share a hole (`{-used(r), -used(r'), -hole(r,h), -hole(r',h)}`).
//! 2. Clique-coloring (the sat-comp `clqcl` family): P clique slots each
//!    choose one of N vertices, at most one slot per vertex, two chosen
//!    vertices force an edge literal (`{-slot_i@u, -slot_j@v, e(u,v)}`), every
//!    vertex is colored with one of H colors (unconditional cover clauses),
//!    and an edge forbids equal colors (`{-e(u,v), -color(u,h), -color(v,h)}`).
//!
//! Both reduce to one abstract interface: pigeon literals `a[p][r]` (pigeon p
//! sits at place r) and hole literals `hole[r][h]` (place r maps to hole h)
//! such that, by unit propagation over the ORIGINAL clauses:
//!
//! - (F1) `a[p][r]` with all of `hole[r][*]` false conflicts,
//! - (F2) `a[p][r]` and `a[p'][r]` (p != p') conflicts,
//! - (F3) `a[p][r]`, `a[p'][r']`, `hole[r][h]`, `hole[r'][h]` (p != p',
//!   r != r') conflicts.
//!
//! Detection verifies every clause needed for F1-F3 by exact lookup, which
//! makes it the soundness anchor: a formula passing detection with P > H is
//! genuinely UNSAT (any model would injectively map P pigeons through distinct
//! places into H < P holes). The proof engine then emits a DRAT refutation:
//! fresh-variable definitions `W[p][r][h] ~ a[p][r] & hole[r][h]` and
//! `G[p][h] ~ OR_r W[p][r][h]` (RAT on the leading fresh literal, the
//! drat-trim pivot convention), pairwise blocking lemmas at the W then G
//! level, per-pigeon covers of G, and an injective-assignment DFS over the
//! (H+1) x H matrix of G literals ending in the empty clause. Every
//! non-definition line is RUP against the clauses emitted before it, so the
//! stream is drat-trim-checkable and an aborted attempt can never invalidate
//! later proof lines. Detection is literal-based throughout, so it is
//! invariant under variable renaming, clause/literal reordering, and
//! per-variable polarity flips (the `rphp5_*_shuffled` cells exercise this).

use std::collections::{HashMap, HashSet};

/// Detected pigeonhole structure handed to the proof engine.
pub(crate) struct PhpStructure {
    /// `a[p][r]`: literal "pigeon p sits at place r" (P x N, place-aligned
    /// across pigeons).
    pub(crate) a: Vec<Vec<i32>>,
    /// `hole[r][h]`: literal "place r maps to hole h" (N x H, hole-aligned
    /// across places).
    pub(crate) hole: Vec<Vec<i32>>,
    /// Direct pigeonhole (SESSION 14c): `a[p][h]` maps pigeons straight to
    /// holes — no relay/place layer. `hole` is empty; holes() = places() =
    /// the cover width. Refutation runs the inductive reduction directly on
    /// the `a` literals (the formula's own covers + AMO cliques are the
    /// level-H axioms).
    pub(crate) direct: bool,
}

impl PhpStructure {
    pub(crate) fn pigeons(&self) -> usize {
        self.a.len()
    }
    pub(crate) fn places(&self) -> usize {
        if self.direct {
            self.a.first().map_or(0, |r| r.len())
        } else {
            self.hole.len()
        }
    }
    pub(crate) fn holes(&self) -> usize {
        if self.direct {
            self.a.first().map_or(0, |r| r.len())
        } else {
            self.hole.first().map_or(0, |h| h.len())
        }
    }
}

/// Production floor on the pigeon-cover width (N): real family members are
/// double-digit wide, and the floor lets ordinary formulas exit on the
/// histogram precheck alone.
pub(crate) const MIN_PLACES: usize = 10;
/// Clause-count cap for the pass: the family instances are tens of thousands
/// of clauses; anything bigger declines before gathering.
pub(crate) const MAX_CLAUSES: usize = 400_000;

/// Cheap histogram precheck shared by the parse-time and solve-time callers:
/// the pigeon covers must be the strictly longest clause length (>=
/// `MIN_PLACES`), 3-7 of them, with everything else short.
pub(crate) fn histogram_precheck(lengths: impl Iterator<Item = usize>) -> bool {
    let mut count = 0usize;
    let mut lmax = 0usize;
    let mut lmax_count = 0usize;
    let mut second = 0usize;
    for len in lengths {
        count += 1;
        if count > MAX_CLAUSES {
            return false;
        }
        if len > lmax {
            second = lmax;
            lmax = len;
            lmax_count = 1;
        } else if len == lmax {
            lmax_count += 1;
        } else if len > second {
            second = len;
        }
    }
    // SESSION 14c: cover count widened 7 -> 60 (clqcl_30_9_8/11_10 have 9/11
    // slot covers; harder-fphp-016-015 has 16 direct covers; rphp_p25_r25 has
    // 25 pigeon + 25 hole covers at the SAME length because N == H+1) and the
    // second-longest cap 8 -> 12 (the unconditional c-color covers reach
    // length c). Selectivity guard: the second-longest class must still sit
    // strictly below the covers; detection stays all-or-nothing exact
    // regardless (bench scan 2026-07-31: the widened precheck admits the 15
    // family cells plus 3 non-family cells, which pay one declining exact
    // scan each).
    lmax >= MIN_PLACES && (3..=60).contains(&lmax_count) && second <= 12
}

/// Parse-time probe: does this formula carry a refutable pigeonhole structure?
/// Used to hold frontend BVA off matching formulas (factoring rewrites the
/// clique-coloring adjacency ternaries and would hide the structure from the
/// solve-time refutation).
pub(crate) fn formula_matches(clauses: &[Vec<i32>]) -> bool {
    histogram_precheck(clauses.iter().map(Vec::len)) && detect(clauses, MIN_PLACES).is_some()
}

/// SAT_DEBUG_PHP=1: decline-point tracing for the detector (one-shot pass,
/// env check per call is fine). Used to decode near-miss family members.
fn php_dbg(msg: &str) {
    if std::env::var("SAT_DEBUG_PHP").is_ok() {
        eprintln!("c php_dbg {msg}");
    }
}

fn sorted_clause(lits: &[i32]) -> Vec<i32> {
    let mut v = lits.to_vec();
    v.sort_unstable();
    v
}

/// Shared clause lookup: a set of sorted clauses plus literal occurrence lists.
struct ClauseIndex<'a> {
    clauses: &'a [Vec<i32>],
    set: HashSet<Vec<i32>>,
    by_lit: HashMap<i32, Vec<usize>>,
}

impl<'a> ClauseIndex<'a> {
    fn new(clauses: &'a [Vec<i32>]) -> Self {
        let mut set = HashSet::with_capacity(clauses.len() * 2);
        let mut by_lit: HashMap<i32, Vec<usize>> = HashMap::new();
        for (idx, c) in clauses.iter().enumerate() {
            set.insert(sorted_clause(c));
            for &l in c {
                by_lit.entry(l).or_default().push(idx);
            }
        }
        Self {
            clauses,
            set,
            by_lit,
        }
    }

    fn contains(&self, lits: &[i32]) -> bool {
        self.set.contains(&sorted_clause(lits))
    }

    fn occurrences(&self, lit: i32) -> &[usize] {
        self.by_lit.get(&lit).map_or(&[], |v| v.as_slice())
    }
}

/// Try to detect either supported pigeonhole shape. `min_places` guards the
/// long-clause width (the production caller passes a double-digit floor so
/// ordinary formulas exit on the histogram precheck; tests pass small values).
pub(crate) fn detect(clauses: &[Vec<i32>], min_places: usize) -> Option<PhpStructure> {
    // Deduplicate while preserving first-occurrence order (duplicate clauses
    // would break the exact structural counting below).
    let mut seen: HashSet<Vec<i32>> = HashSet::with_capacity(clauses.len() * 2);
    let mut uniq: Vec<Vec<i32>> = Vec::with_capacity(clauses.len());
    for c in clauses {
        if c.is_empty() {
            return None;
        }
        if seen.insert(sorted_clause(c)) {
            uniq.push(c.clone());
        }
    }

    // Long-clause selection: the pigeon covers are the unique longest length,
    // small in count, and strictly longer than everything else (which must be
    // short). This is the cheap, highly selective shape filter.
    let lmax = uniq.iter().map(|c| c.len()).max()?;
    let long_idx: Vec<usize> = (0..uniq.len()).filter(|&i| uniq[i].len() == lmax).collect();
    let second = uniq
        .iter()
        .map(|c| c.len())
        .filter(|&l| l < lmax)
        .max()
        .unwrap_or(0);
    if lmax < min_places || !(3..=60).contains(&long_idx.len()) || second > 12 || lmax <= second {
        php_dbg(&format!(
            "decline long-select lmax={lmax} count={} second={second}",
            long_idx.len()
        ));
        return None;
    }

    // Partition the longest class (SESSION 14c): when N == H+1 the rphp
    // used->hole covers ({-s} ∪ per-place holes, length H+1) collide with the
    // pigeon covers (length N) — rphp_p25_r25 — and BOTH kinds are
    // var-disjoint, so var-sharing cannot separate them. The working oracle
    // is AMO-connectivity: with the first longest clause as the seed cover,
    // a longest clause is a pigeon cover iff EVERY seed literal has an
    // at-most-one binary partner inside it (pigeon covers are pairwise fully
    // AMO-linked place by place; a hole cover is reachable from at most one
    // seed literal via its {-a, s} binary). Families whose longest class is
    // pure pigeon covers partition to the identity.
    let index_all = ClauseIndex::new(&uniq);
    // A shuffled instance can put a HOLE cover first in the longest class, so
    // the partition retries every longest clause as the seed until one yields
    // a detecting pigeon set. Bounded (<=60 seeds, exact scans each) and
    // deterministic; families whose first seed works behave as before.
    for (seed_no, &seed) in long_idx.iter().enumerate() {
        let mut pigeon_idx: Vec<usize> = vec![seed];
        for &i in &long_idx {
            if i == seed {
                continue;
            }
            let fully_linked = uniq[seed].iter().all(|&l0| {
                uniq[i]
                    .iter()
                    .any(|&l| l != l0 && index_all.contains(&[-l0, -l]))
            });
            if fully_linked {
                pigeon_idx.push(i);
            }
        }
        let p_total = pigeon_idx.len();
        if !(3..=60).contains(&p_total) {
            continue;
        }
        // Pigeon literals: `a_info[lit] = (pigeon, position)`. All pigeon-cover
        // variables must be distinct (no var reuse across or within pigeons).
        let mut a_info: HashMap<i32, (usize, usize)> = HashMap::new();
        let mut a_vars: HashSet<i32> = HashSet::new();
        let mut reuse = false;
        let pigeon_clauses: Vec<&Vec<i32>> = pigeon_idx.iter().map(|&i| &uniq[i]).collect();
        'build: for (p, c) in pigeon_clauses.iter().enumerate() {
            for (pos, &l) in c.iter().enumerate() {
                if !a_vars.insert(l.abs()) {
                    reuse = true;
                    break 'build;
                }
                a_info.insert(l, (p, pos));
            }
        }
        if reuse {
            continue;
        }
        if let Some(st) = detect_with_pigeons(&uniq, &index_all, &pigeon_clauses, &a_info, &a_vars)
        {
            if seed_no > 0 {
                php_dbg(&format!("partition succeeded at seed {seed_no}"));
            }
            return Some(st);
        }
    }
    php_dbg("decline all-seeds");
    None
}

/// Detection past the pigeon-cover partition: place cliques, then the three
/// shape detectors in order (relay rphp, clique-coloring, direct php).
fn detect_with_pigeons(
    uniq: &[Vec<i32>],
    index: &ClauseIndex,
    pigeon_clauses: &[&Vec<i32>],
    a_info: &HashMap<i32, (usize, usize)>,
    a_vars: &HashSet<i32>,
) -> Option<PhpStructure> {
    let _ = uniq;
    let p_total = pigeon_clauses.len();
    let Some(cliques) = place_cliques(index, pigeon_clauses, a_info) else {
        php_dbg("decline place_cliques");
        return None;
    };
    if let Some(s) = detect_rphp(index, pigeon_clauses, a_info, a_vars, &cliques) {
        return Some(s);
    }
    php_dbg("rphp declined; trying clqcl");
    if let Some(st) = detect_clqcl(index, pigeon_clauses, a_vars, &cliques) {
        return Some(st);
    }
    php_dbg("clqcl declined; trying direct");
    // Direct pigeonhole (SESSION 14c, harder-fphp-016-015 class): P pigeon
    // covers over P x N pairwise-distinct variables whose columns form
    // verified per-hole AMO cliques (place_cliques above), with more pigeons
    // than holes. The covers + cliques ARE the counting core; extra clauses
    // only strengthen the formula, so P > N alone certifies UNSAT.
    let n = cliques.len();
    if p_total > n && n >= 2 {
        php_dbg(&format!("direct php matched pigeons={p_total} holes={n}"));
        let a: Vec<Vec<i32>> = (0..p_total)
            .map(|p| (0..n).map(|r| cliques[r][p]).collect())
            .collect();
        return Some(PhpStructure {
            a,
            hole: Vec::new(),
            direct: true,
        });
    }
    php_dbg("decline direct");
    None
}

/// Group the pigeon literals into N place cliques of size P: pigeon 0's r-th
/// literal defines place r, and each other pigeon contributes exactly one
/// literal connected to it by an at-most-one binary `{-x, -y}`. Verifies the
/// full pairwise clique and that the assignment is a perfect matching (every
/// pigeon literal used exactly once).
fn place_cliques(
    index: &ClauseIndex,
    pigeon_clauses: &[&Vec<i32>],
    a_info: &HashMap<i32, (usize, usize)>,
) -> Option<Vec<Vec<i32>>> {
    let p_total = pigeon_clauses.len();
    let n = pigeon_clauses[0].len();
    let mut used: HashSet<i32> = HashSet::new();
    let mut cliques: Vec<Vec<i32>> = Vec::with_capacity(n);
    for &lit0 in pigeon_clauses[0].iter() {
        // Partner per other pigeon via binaries containing -lit0.
        let mut partner: Vec<Option<i32>> = vec![None; p_total];
        partner[0] = Some(lit0);
        for &cid in index.occurrences(-lit0) {
            let c = &index.clauses[cid];
            if c.len() != 2 {
                continue;
            }
            let other = if c[0] == -lit0 { c[1] } else { c[0] };
            if other == -lit0 {
                continue;
            }
            if let Some(&(p2, _)) = a_info.get(&-other) {
                if p2 == 0 {
                    continue; // same-pigeon at-most-one (clqcl slot AMO)
                }
                match partner[p2] {
                    None => partner[p2] = Some(-other),
                    Some(existing) if existing == -other => {}
                    Some(existing) => {
                        php_dbg(&format!(
                            "place_cliques ambiguous lit0={lit0} p2={p2} existing={existing} new={}",
                            -other
                        ));
                        return None; // ambiguous partner
                    }
                }
            }
        }
        let clique: Vec<i32> = match (0..p_total).map(|p| partner[p]).collect::<Option<_>>() {
            Some(c) => c,
            None => {
                let missing: Vec<usize> =
                    (0..p_total).filter(|&p| partner[p].is_none()).collect();
                php_dbg(&format!(
                    "place_cliques missing-partner lit0={lit0} missing={missing:?}"
                ));
                return None;
            }
        };
        // Full pairwise clique of at-most-one binaries.
        for i in 0..p_total {
            for j in (i + 1)..p_total {
                if !index.contains(&[-clique[i], -clique[j]]) {
                    php_dbg(&format!(
                        "place_cliques missing-amo {} {}",
                        clique[i], clique[j]
                    ));
                    return None;
                }
            }
        }
        for &l in &clique {
            if !used.insert(l) {
                return None; // literal reused across places
            }
        }
        cliques.push(clique);
    }
    if used.len() != p_total * n {
        return None;
    }
    Some(cliques)
}

/// Relativized-PHP shape: each place clique shares a unique "used" literal
/// (`{-a, s}` binaries), each `s` has a unique hole-cover clause
/// (`{-s} ∪ holes`), and hole identity across places comes from the 4-clause
/// conflict constraints, aligned by union-find and then verified completely.
fn detect_rphp(
    index: &ClauseIndex,
    pigeon_clauses: &[&Vec<i32>],
    a_info: &HashMap<i32, (usize, usize)>,
    a_vars: &HashSet<i32>,
    cliques: &[Vec<i32>],
) -> Option<PhpStructure> {
    let p_total = pigeon_clauses.len();
    let n = cliques.len();
    // The place's "used" literal: the unique non-pigeon binary partner shared
    // by all P literals of the clique.
    let mut s_lits: Vec<i32> = Vec::with_capacity(n);
    let mut s_vars: HashSet<i32> = HashSet::new();
    for clique in cliques {
        let mut s_lit: Option<i32> = None;
        for &al in clique {
            let mut mine: Option<i32> = None;
            for &cid in index.occurrences(-al) {
                let c = &index.clauses[cid];
                if c.len() != 2 {
                    continue;
                }
                let other = if c[0] == -al { c[1] } else { c[0] };
                if other == -al || a_info.contains_key(&-other) {
                    continue;
                }
                match mine {
                    None => mine = Some(other),
                    Some(existing) if existing == other => {}
                    Some(_) => return None, // ambiguous used-literal
                }
            }
            let mine = mine?;
            match s_lit {
                None => s_lit = Some(mine),
                Some(existing) if existing == mine => {}
                Some(_) => return None, // clique members disagree
            }
        }
        let s_lit = s_lit?;
        if a_vars.contains(&s_lit.abs()) || !s_vars.insert(s_lit.abs()) {
            return None;
        }
        s_lits.push(s_lit);
    }

    // Hole cover per place: the unique clause containing -s_r whose other
    // literals mention no pigeon or used variable.
    let mut hole_sets: Vec<Vec<i32>> = Vec::with_capacity(n);
    let mut hole_owner: HashMap<i32, (usize, usize)> = HashMap::new(); // lit -> (place, pos)
    let mut hole_vars: HashSet<i32> = HashSet::new();
    let mut h_count: Option<usize> = None;
    for (r, &s_lit) in s_lits.iter().enumerate() {
        let mut cover: Option<Vec<i32>> = None;
        for &cid in index.occurrences(-s_lit) {
            let c = &index.clauses[cid];
            if c.len() < 3 {
                continue;
            }
            let rest: Vec<i32> = c.iter().copied().filter(|&l| l != -s_lit).collect();
            if rest.len() + 1 != c.len() {
                continue;
            }
            if rest
                .iter()
                .any(|&l| a_vars.contains(&l.abs()) || s_vars.contains(&l.abs()))
            {
                continue;
            }
            if cover.is_some() {
                return None; // ambiguous hole cover
            }
            cover = Some(rest);
        }
        let cover = cover?;
        match h_count {
            None => h_count = Some(cover.len()),
            Some(h) if h == cover.len() => {}
            Some(_) => return None,
        }
        for (pos, &l) in cover.iter().enumerate() {
            if a_vars.contains(&l.abs()) || s_vars.contains(&l.abs()) || !hole_vars.insert(l.abs())
            {
                return None;
            }
            hole_owner.insert(l, (r, pos));
        }
        hole_sets.push(cover);
    }
    let h = h_count?;
    if h < 2 || p_total <= h {
        return None;
    }

    // Hole alignment across places: union-find seeded by the 4-clause
    // conflicts {-s_r, -s_r', -b, -b'}.
    let neg_s: HashSet<i32> = s_lits.iter().map(|&s| -s).collect();
    let mut uf = UnionFind::new(n * h);
    for c in index.clauses.iter() {
        if c.len() != 4 {
            continue;
        }
        let s_hits = c.iter().filter(|l| neg_s.contains(l)).count();
        if s_hits != 2 {
            continue;
        }
        let nodes: Vec<usize> = c
            .iter()
            .filter_map(|&l| hole_owner.get(&-l).map(|&(r, pos)| r * h + pos))
            .collect();
        if nodes.len() != 2 {
            continue;
        }
        uf.union(nodes[0], nodes[1]);
    }
    let hole = align_holes(&mut uf, &hole_sets, n, h)?;

    // Completeness: every place pair conflicts on every hole.
    for r1 in 0..n {
        for r2 in (r1 + 1)..n {
            for hh in 0..h {
                if !index.contains(&[-s_lits[r1], -s_lits[r2], -hole[r1][hh], -hole[r2][hh]]) {
                    return None;
                }
            }
        }
    }

    let a: Vec<Vec<i32>> = (0..p_total)
        .map(|p| (0..n).map(|r| cliques[r][p]).collect())
        .collect();
    Some(PhpStructure { a, hole, direct: false })
}

/// Clique-coloring shape: chosen vertices force edge literals via ternary
/// adjacency clauses (or are directly blocked pairwise), every vertex has an
/// unconditional color cover, and edges forbid shared colors.
fn detect_clqcl(
    index: &ClauseIndex,
    pigeon_clauses: &[&Vec<i32>],
    a_vars: &HashSet<i32>,
    cliques: &[Vec<i32>],
) -> Option<PhpStructure> {
    let p_total = pigeon_clauses.len();
    let n = cliques.len();
    if n < 3 {
        return None;
    }
    // Per vertex pair {u, v}: either every slot configuration is directly
    // blocked by a binary, or a shared edge literal is forced by ternaries.
    let mut edge: HashMap<(usize, usize), i32> = HashMap::new();
    for u in 0..n {
        for v in (u + 1)..n {
            let mut candidates: Option<HashSet<i32>> = None;
            let mut any_open = false;
            for i in 0..p_total {
                for j in (i + 1)..p_total {
                    for &(x, y) in &[(cliques[u][i], cliques[v][j]), (cliques[v][i], cliques[u][j])]
                    {
                        if index.contains(&[-x, -y]) {
                            continue; // directly blocked configuration
                        }
                        any_open = true;
                        let mut cfg: HashSet<i32> = HashSet::new();
                        for &cid in index.occurrences(-x) {
                            let c = &index.clauses[cid];
                            if c.len() != 3 || !c.contains(&-y) {
                                continue;
                            }
                            for &l in c {
                                if l != -x && l != -y && !a_vars.contains(&l.abs()) {
                                    cfg.insert(l);
                                }
                            }
                        }
                        if cfg.is_empty() {
                            return None;
                        }
                        candidates = Some(match candidates {
                            None => cfg,
                            Some(prev) => prev.intersection(&cfg).copied().collect(),
                        });
                        if candidates.as_ref().is_some_and(HashSet::is_empty) {
                            return None;
                        }
                    }
                }
            }
            if any_open {
                let set = candidates?;
                let e = set.iter().copied().min_by_key(|l| (l.abs(), *l < 0))?;
                edge.insert((u, v), e);
            }
        }
    }

    // Colors per vertex: the literal set common to all incident edges' color
    // clauses. Needs at least two incident edges to separate a vertex's colors
    // from its neighbors'.
    let color_lits_touching = |e: i32| -> HashSet<i32> {
        let mut out = HashSet::new();
        for &cid in index.occurrences(-e) {
            let c = &index.clauses[cid];
            if c.len() != 3 {
                continue;
            }
            for &l in c {
                if l != -e && !a_vars.contains(&l.abs()) {
                    out.insert(-l);
                }
            }
        }
        out
    };
    let mut colors: Vec<Vec<i32>> = Vec::with_capacity(n);
    let mut color_owner: HashMap<i32, (usize, usize)> = HashMap::new();
    let mut color_vars: HashSet<i32> = HashSet::new();
    let mut h_count: Option<usize> = None;
    for u in 0..n {
        let partners: Vec<usize> = (0..n)
            .filter(|&v| v != u && edge.contains_key(&(u.min(v), u.max(v))))
            .collect();
        if partners.len() < 2 {
            return None;
        }
        let mut common: Option<HashSet<i32>> = None;
        for &v in &partners {
            let e = edge[&(u.min(v), u.max(v))];
            let touched = color_lits_touching(e);
            common = Some(match common {
                None => touched,
                Some(prev) => prev.intersection(&touched).copied().collect(),
            });
        }
        let common = common?;
        match h_count {
            None => h_count = Some(common.len()),
            Some(h) if h == common.len() => {}
            Some(_) => return None,
        }
        // Deterministic per-vertex order; hole alignment below rebuilds the
        // cross-vertex correspondence.
        let mut ordered: Vec<i32> = common.into_iter().collect();
        ordered.sort_unstable();
        // The unconditional color cover clause must exist (F1).
        if !index.contains(&ordered) {
            return None;
        }
        for (pos, &l) in ordered.iter().enumerate() {
            if a_vars.contains(&l.abs()) || !color_vars.insert(l.abs()) {
                return None;
            }
            color_owner.insert(l, (u, pos));
        }
        colors.push(ordered);
    }
    let h = h_count?;
    if h < 2 || p_total <= h {
        return None;
    }
    // Edge literals must be disjoint from color variables.
    for e in edge.values() {
        if color_vars.contains(&e.abs()) {
            return None;
        }
    }

    // Hole alignment via the color-conflict ternaries {-e, -c_u, -c_v}.
    let mut uf = UnionFind::new(n * h);
    for (&(u, v), &e) in edge.iter() {
        for &cid in index.occurrences(-e) {
            let c = &index.clauses[cid];
            if c.len() != 3 {
                continue;
            }
            let mut nodes: Vec<usize> = Vec::with_capacity(2);
            for &l in c {
                if l == -e {
                    continue;
                }
                let Some(&(w, pos)) = color_owner.get(&-l) else {
                    nodes.clear();
                    break;
                };
                if w != u && w != v {
                    nodes.clear();
                    break;
                }
                nodes.push(w * h + pos);
            }
            if nodes.len() == 2 {
                uf.union(nodes[0], nodes[1]);
            }
        }
    }
    let hole = align_holes(&mut uf, &colors, n, h)?;

    // Completeness: every open slot configuration has its adjacency ternary
    // with the chosen edge literal, and the edge conflicts on every hole.
    for u in 0..n {
        for v in (u + 1)..n {
            let e = edge.get(&(u, v)).copied();
            for i in 0..p_total {
                for j in (i + 1)..p_total {
                    for &(x, y) in &[(cliques[u][i], cliques[v][j]), (cliques[v][i], cliques[u][j])]
                    {
                        if index.contains(&[-x, -y]) {
                            continue;
                        }
                        let e = e?;
                        if !index.contains(&[-x, -y, e]) {
                            return None;
                        }
                    }
                }
            }
            if let Some(e) = e {
                for hh in 0..h {
                    if !index.contains(&[-e, -hole[u][hh], -hole[v][hh]]) {
                        return None;
                    }
                }
            }
        }
    }

    let a: Vec<Vec<i32>> = (0..p_total)
        .map(|p| (0..n).map(|r| cliques[r][p]).collect())
        .collect();
    Some(PhpStructure { a, hole, direct: false })
}

/// Collapse union-find classes over the (place, position) hole grid into an
/// N x H hole-literal matrix: exactly H classes, each holding exactly one
/// literal per place, hole index taken from place 0's position order.
fn align_holes(
    uf: &mut UnionFind,
    per_place: &[Vec<i32>],
    n: usize,
    h: usize,
) -> Option<Vec<Vec<i32>>> {
    let mut class_of_hole0: HashMap<usize, usize> = HashMap::new();
    for hh in 0..h {
        class_of_hole0.insert(uf.find(hh), hh);
    }
    if class_of_hole0.len() != h {
        return None;
    }
    let mut hole: Vec<Vec<i32>> = vec![vec![0; h]; n];
    for r in 0..n {
        let mut filled = 0usize;
        for pos in 0..h {
            let root = uf.find(r * h + pos);
            let &hh = class_of_hole0.get(&root)?;
            if hole[r][hh] != 0 {
                return None;
            }
            hole[r][hh] = per_place[r][pos];
            filled += 1;
        }
        if filled != h {
            return None;
        }
    }
    Some(hole)
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

/// Upper bound on emitted proof lines, for the checker-size guard.
/// Closing-stage line count: factorial DFS for H <= 8 (byte-identical proofs
/// for the banked family cells), polynomial inductive reduction above (the
/// missing factorial term let the 5M cap silently under-gate: H=10 measured
/// 10,389,802 DFS lines on clqcl_30_11_10, SESSION 14c).
fn closing_lines(h: u64) -> u64 {
    if h <= 8 {
        let mut dfs: u64 = 0;
        let mut falling: u64 = 1;
        for k in 0..h {
            falling = falling.saturating_mul(h - k);
            dfs = dfs.saturating_add(falling);
        }
        dfs
    } else {
        let mut total: u64 = 1; // the final empty clause
        let mut k = h;
        while k >= 2 {
            let pairs = k * (k - 1) / 2;
            total = total
                .saturating_add(6 * k * (k - 1)) // t/b definitions
                .saturating_add(k) // reduced covers
                .saturating_add(6 * pairs * (k - 1)); // AMO helper chain
            k -= 1;
        }
        total
    }
}

pub(crate) fn estimated_proof_lines(s: &PhpStructure) -> u64 {
    let h = s.holes() as u64;
    let n = s.places() as u64;
    let pp = (h + 1).min(s.pigeons() as u64);
    let pairs = pp * (pp - 1) / 2;
    if s.direct {
        return closing_lines(h) + 4096;
    }
    3 * pp * n * h + pp * h * (n + 1) + h * pairs * n * n + h * pairs * n + h * pairs + pp * (n + 1)
        + closing_lines(h)
        + 4096
}

/// Emit the extended-resolution DRAT refutation. `num_vars` is the highest
/// variable index in use; fresh definition variables start above it. The
/// structure must have pigeons > holes (guaranteed by `detect`).
pub(crate) fn refute_with_proof(s: &PhpStructure, num_vars: usize, emit: &mut dyn FnMut(&[i32])) {
    let n = s.places();
    let h = s.holes();
    let pp = (h + 1).min(s.pigeons());
    debug_assert!(s.pigeons() > h && pp == h + 1);
    let base = num_vars as i32;
    if s.direct {
        // Direct pigeonhole: the formula's own covers and per-hole AMO
        // cliques ARE the level-H axioms; no W/G layer is needed. Run the
        // inductive reduction straight on the pigeon literals.
        inductive_blocks(&|p, hh| s.a[p][hh], h, base, emit);
        return;
    }
    let w = |p: usize, r: usize, hh: usize| -> i32 { base + 1 + ((p * n + r) * h + hh) as i32 };
    let g = |p: usize, hh: usize| -> i32 { base + 1 + (pp * n * h) as i32 + (p * h + hh) as i32 };

    // W[p][r][h] <-> a[p][r] & hole[r][h]. The fresh literal leads each
    // definition clause (drat-trim checks RAT on the first literal); the
    // reverse-direction clauses resolve against the forward one into
    // tautologies, so each line is RAT at emission time.
    for p in 0..pp {
        for r in 0..n {
            for hh in 0..h {
                let wv = w(p, r, hh);
                emit(&[wv, -s.a[p][r], -s.hole[r][hh]]);
                emit(&[-wv, s.a[p][r]]);
                emit(&[-wv, s.hole[r][hh]]);
            }
        }
    }
    // G[p][h] <-> OR_r W[p][r][h].
    let mut long: Vec<i32> = Vec::with_capacity(n + 1);
    for p in 0..pp {
        for hh in 0..h {
            let gv = g(p, hh);
            for r in 0..n {
                emit(&[gv, -w(p, r, hh)]);
            }
            long.clear();
            long.push(-gv);
            for r in 0..n {
                long.push(w(p, r, hh));
            }
            emit(&long);
        }
    }
    // L1: no two pigeons occupy the same hole via any place pair. RUP: the W
    // definitions propagate both pigeon and hole literals, then the original
    // at-most-one / conflict clauses (F2 for r1 == r2, F3 otherwise) close it.
    for hh in 0..h {
        for p1 in 0..pp {
            for p2 in (p1 + 1)..pp {
                for r1 in 0..n {
                    for r2 in 0..n {
                        emit(&[-w(p1, r1, hh), -w(p2, r2, hh)]);
                    }
                }
            }
        }
    }
    // L2: lift one side to G (RUP via L1 and G's long definition).
    for hh in 0..h {
        for p1 in 0..pp {
            for p2 in (p1 + 1)..pp {
                for r in 0..n {
                    emit(&[-w(p1, r, hh), -g(p2, hh)]);
                }
            }
        }
    }
    // L3: pairwise hole exclusion at the G level.
    for hh in 0..h {
        for p1 in 0..pp {
            for p2 in (p1 + 1)..pp {
                emit(&[-g(p1, hh), -g(p2, hh)]);
            }
        }
    }
    // L4: a placed pigeon reaches some hole (RUP via the W/G definitions and
    // F1), per place.
    let mut cover: Vec<i32> = Vec::with_capacity(h + 1);
    for p in 0..pp {
        for r in 0..n {
            cover.clear();
            cover.push(-s.a[p][r]);
            cover.extend((0..h).map(|hh| g(p, hh)));
            emit(&cover);
        }
    }
    // L5: every pigeon reaches some hole (RUP via L4 and the pigeon's cover
    // clause).
    for p in 0..pp {
        let c: Vec<i32> = (0..h).map(|hh| g(p, hh)).collect();
        emit(&c);
    }
    // Close the counting core over the G matrix. The DFS scheme emits one
    // block per injective-assignment prefix (~e * H! lines) — fine up to
    // H ~ 8, infeasible beyond (H=10 measured 10.39M lines on
    // clqcl_30_11_10). Larger H uses the polynomial inductive
    // PHP(k) -> PHP(k-1) reduction (~3/4 * H^4 lines). Small H keeps the
    // DFS so every previously-banked family cell's proof stays
    // byte-identical.
    if h <= 8 {
        let mut sigma: Vec<usize> = Vec::with_capacity(h);
        let mut used = vec![false; h];
        dfs_blocks(&g, h, &mut sigma, &mut used, emit);
    } else {
        let fresh_base = base + (pp * n * h) as i32 + (pp * h) as i32;
        inductive_blocks(&|p, hh| g(p, hh), h, fresh_base, emit);
    }
}

/// Polynomial closure of the pigeonhole counting core (SESSION 14c): given
/// level-H literals `cur[p][h]` (p in 0..=H, h in 0..H) whose covers
/// `OR_h cur[p][h]` and per-hole at-most-one binaries `{-cur[p1][h],
/// -cur[p2][h]}` are already present (as axioms for the direct shape, as
/// L3/L5 lemmas for the relay shape), emit Cook's inductive reduction
/// PHP(k) -> PHP(k-1) down to the empty clause. Every line is RAT (fresh
/// definition vars, pivot first) or RUP in a single unit-propagation pass:
///
///   b[p][h] := cur[p][h] OR (cur[p][last_h] AND cur[last_p][h])
///
/// with per-level derived covers and AMOs; the AMO derivation goes through
/// the 6-lemma helper chain (t/cur cross pairs, then cur-vs-b, t-vs-b,
/// b-vs-b), each step RUP against the previous ones plus the level-k
/// axioms. Terminates at k=1: two unit covers + one AMO binary -> empty.
fn inductive_blocks(
    cur0: &dyn Fn(usize, usize) -> i32,
    h: usize,
    fresh_base: i32,
    emit: &mut dyn FnMut(&[i32]),
) {
    // cur[p][hh] literal matrix for the current level (k holes, k+1 pigeons).
    let mut cur: Vec<Vec<i32>> = (0..=h)
        .map(|p| (0..h).map(|hh| cur0(p, hh)).collect())
        .collect();
    let mut fresh = fresh_base;
    let mut k = h;
    while k >= 2 {
        let last_p = k; // pigeon index dropped by this reduction
        let last_h = k - 1; // hole index dropped by this reduction
        // Fresh t/b per surviving (pigeon, hole) cell.
        let mut t = vec![vec![0i32; last_h]; k];
        let mut b = vec![vec![0i32; last_h]; k];
        for (p, tp) in t.iter_mut().enumerate() {
            for (hh, tv) in tp.iter_mut().enumerate() {
                fresh += 1;
                *tv = fresh;
                let x = cur[p][last_h];
                let y = cur[last_p][hh];
                // t <-> x & y (RAT on t, pivot first).
                emit(&[*tv, -x, -y]);
                emit(&[-*tv, x]);
                emit(&[-*tv, y]);
            }
        }
        for (p, bp) in b.iter_mut().enumerate() {
            for (hh, bv) in bp.iter_mut().enumerate() {
                fresh += 1;
                *bv = fresh;
                // b <-> cur[p][hh] | t[p][hh] (RAT on b, pivot first).
                emit(&[*bv, -cur[p][hh]]);
                emit(&[*bv, -t[p][hh]]);
                emit(&[-*bv, cur[p][hh], t[p][hh]]);
            }
        }
        // Reduced covers: OR_hh b[p][hh] (RUP: b-defs force -cur/-t, the
        // level cover forces cur[p][last_h], the t-defs then force
        // -cur[last_p][hh] for all hh, last_p's cover forces
        // cur[last_p][last_h], and the last_h-column AMO closes it).
        for (p, bp) in b.iter().enumerate() {
            let cover: Vec<i32> = bp.clone();
            let _ = p;
            emit(&cover);
        }
        // Reduced AMOs via the helper chain, per hole and pigeon pair.
        for hh in 0..last_h {
            for p1 in 0..k {
                for p2 in (p1 + 1)..k {
                    // (ii) cur1 vs t2: t2 -> cur[p2][last_h] & cur[last_p][hh];
                    //      cur1 + column-hh AMO vs last_p closes.
                    emit(&[-cur[p1][hh], -t[p2][hh]]);
                    // (iii) symmetric.
                    emit(&[-t[p1][hh], -cur[p2][hh]]);
                    // (iv) t1 vs t2 via the last_h column AMO.
                    emit(&[-t[p1][hh], -t[p2][hh]]);
                    // (v) cur1 vs b2: level AMO gives -cur2, b2-def forces t2,
                    //     then (t2 ->) cur[last_p][hh] conflicts with cur1.
                    emit(&[-cur[p1][hh], -b[p2][hh]]);
                    // (vi) t1 vs b2: t1 forces cur[p1][last_h] + cur[last_p][hh],
                    //      column AMO gives -cur2, b2-def forces t2 ->
                    //      cur[p2][last_h], last_h column AMO closes.
                    emit(&[-t[p1][hh], -b[p2][hh]]);
                    // (vii) b1 vs b2 via (v) and (vi).
                    emit(&[-b[p1][hh], -b[p2][hh]]);
                }
            }
        }
        // Descend: the b matrix is the next level's cur (k-1 holes, k pigeons).
        cur = b;
        k -= 1;
    }
    // k == 1: covers are the units cur[0][0], cur[1][0]; with the AMO
    // binary the empty clause is RUP.
    emit(&[]);
}

fn dfs_blocks(
    g: &dyn Fn(usize, usize) -> i32,
    h: usize,
    sigma: &mut Vec<usize>,
    used: &mut Vec<bool>,
    emit: &mut dyn FnMut(&[i32]),
) {
    for hh in 0..h {
        if !used[hh] {
            sigma.push(hh);
            used[hh] = true;
            dfs_blocks(g, h, sigma, used, emit);
            used[hh] = false;
            sigma.pop();
        }
    }
    let clause: Vec<i32> = sigma
        .iter()
        .enumerate()
        .map(|(p, &hh)| -g(p, hh))
        .collect();
    emit(&clause);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic LCG for shuffle/sign-flip robustness tests.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Build a relativized-PHP instance: p pigeons, n places, h holes.
    /// Variables: a[p][r], s[r], b[r][h], densely numbered.
    fn build_rphp(p: usize, n: usize, h: usize) -> Vec<Vec<i32>> {
        let a = |pi: usize, r: usize| (pi * n + r + 1) as i32;
        let s = |r: usize| (p * n + r + 1) as i32;
        let b = |r: usize, hh: usize| (p * n + n + r * h + hh + 1) as i32;
        let mut cls: Vec<Vec<i32>> = Vec::new();
        for pi in 0..p {
            cls.push((0..n).map(|r| a(pi, r)).collect());
        }
        for r in 0..n {
            for p1 in 0..p {
                for p2 in (p1 + 1)..p {
                    cls.push(vec![-a(p1, r), -a(p2, r)]);
                }
                cls.push(vec![-a(p1, r), s(r)]);
            }
            let mut cover = vec![-s(r)];
            cover.extend((0..h).map(|hh| b(r, hh)));
            cls.push(cover);
        }
        for r1 in 0..n {
            for r2 in (r1 + 1)..n {
                for hh in 0..h {
                    cls.push(vec![-s(r1), -s(r2), -b(r1, hh), -b(r2, hh)]);
                }
            }
        }
        cls
    }

    /// Build a clique-coloring instance with a free (existential) complete
    /// edge relation: p slots, n vertices, h colors.
    fn build_clqcl(p: usize, n: usize, h: usize) -> Vec<Vec<i32>> {
        let x = |pi: usize, v: usize| (pi * n + v + 1) as i32;
        let c = |v: usize, hh: usize| (p * n + v * h + hh + 1) as i32;
        let e = |u: usize, v: usize| {
            let (u, v) = (u.min(v), u.max(v));
            (p * n + n * h + (u * n + v) + 1) as i32
        };
        let mut cls: Vec<Vec<i32>> = Vec::new();
        for pi in 0..p {
            cls.push((0..n).map(|v| x(pi, v)).collect());
        }
        // Slot AMO (same pigeon, different vertices) and vertex AMO.
        for pi in 0..p {
            for u in 0..n {
                for v in (u + 1)..n {
                    cls.push(vec![-x(pi, u), -x(pi, v)]);
                }
            }
        }
        for v in 0..n {
            for p1 in 0..p {
                for p2 in (p1 + 1)..p {
                    cls.push(vec![-x(p1, v), -x(p2, v)]);
                }
            }
        }
        // Adjacency: chosen slots force the edge (both orientations).
        for i in 0..p {
            for j in (i + 1)..p {
                for u in 0..n {
                    for v in 0..n {
                        if u != v {
                            cls.push(vec![-x(i, u), -x(j, v), e(u, v)]);
                        }
                    }
                }
            }
        }
        // Color covers and edge color conflicts.
        for v in 0..n {
            cls.push((0..h).map(|hh| c(v, hh)).collect());
        }
        for u in 0..n {
            for v in (u + 1)..n {
                for hh in 0..h {
                    cls.push(vec![-e(u, v), -c(u, hh), -c(v, hh)]);
                }
            }
        }
        cls
    }

    fn max_var(clauses: &[Vec<i32>]) -> usize {
        clauses
            .iter()
            .flat_map(|c| c.iter().map(|l| l.unsigned_abs() as usize))
            .max()
            .unwrap_or(0)
    }

    /// Random variable renaming + per-variable polarity flip + clause/literal
    /// order shuffle: the invariances detection must survive.
    fn shuffle_flip(clauses: &[Vec<i32>], seed: u64) -> Vec<Vec<i32>> {
        let nv = max_var(clauses);
        let mut rng = Lcg(seed);
        let mut perm: Vec<i32> = (1..=nv as i32).collect();
        for i in (1..perm.len()).rev() {
            perm.swap(i, rng.below(i + 1));
        }
        let flip: Vec<bool> = (0..=nv).map(|_| rng.next() & 1 == 1).collect();
        let mut out: Vec<Vec<i32>> = clauses
            .iter()
            .map(|c| {
                let mut nc: Vec<i32> = c
                    .iter()
                    .map(|&l| {
                        let v = l.unsigned_abs() as usize;
                        let mapped = perm[v - 1];
                        let s = (l > 0) ^ flip[v];
                        if s {
                            mapped
                        } else {
                            -mapped
                        }
                    })
                    .collect();
                for i in (1..nc.len()).rev() {
                    nc.swap(i, rng.below(i + 1));
                }
                nc
            })
            .collect();
        for i in (1..out.len()).rev() {
            out.swap(i, rng.below(i + 1));
        }
        out
    }

    /// Self-contained RUP check (mirrors sweep.rs's test helper).
    fn is_rup(clauses: &[Vec<i32>], c: &[i32]) -> bool {
        let mut assign: HashMap<i32, bool> = HashMap::new();
        for &l in c {
            let v = l.abs();
            let want = l < 0;
            if let Some(&e) = assign.get(&v) {
                if e != want {
                    return true;
                }
            }
            assign.insert(v, want);
        }
        loop {
            let mut changed = false;
            for clause in clauses {
                let mut unassigned: Vec<i32> = Vec::new();
                let mut satisfied = false;
                for &l in clause {
                    match assign.get(&l.abs()) {
                        Some(&val) => {
                            if (val && l > 0) || (!val && l < 0) {
                                satisfied = true;
                                break;
                            }
                        }
                        None => unassigned.push(l),
                    }
                }
                if satisfied {
                    continue;
                }
                if unassigned.is_empty() {
                    return true;
                }
                if unassigned.len() == 1 {
                    let l = unassigned[0];
                    assign.insert(l.abs(), l > 0);
                    changed = true;
                }
            }
            if !changed {
                return false;
            }
        }
    }

    /// DRAT line check: RUP, or RAT on the first literal (resolvents against
    /// all clauses containing its negation are tautological or RUP).
    fn is_rat_or_rup(clauses: &[Vec<i32>], c: &[i32]) -> bool {
        if is_rup(clauses, c) {
            return true;
        }
        let Some(&pivot) = c.first() else {
            return false;
        };
        clauses.iter().filter(|d| d.contains(&-pivot)).all(|d| {
            let mut res: Vec<i32> = c.iter().copied().filter(|&l| l != pivot).collect();
            res.extend(d.iter().copied().filter(|&l| l != -pivot));
            if res.iter().any(|&l| res.contains(&-l)) {
                return true;
            }
            is_rup(clauses, &res)
        })
    }

    fn check_full_proof(clauses: &[Vec<i32>]) {
        let s = detect(clauses, 4).expect("structure must be detected");
        let nv = max_var(clauses);
        let mut acc: Vec<Vec<i32>> = clauses.to_vec();
        let mut lines: Vec<Vec<i32>> = Vec::new();
        refute_with_proof(&s, nv, &mut |c| lines.push(c.to_vec()));
        assert!(lines.len() as u64 <= estimated_proof_lines(&s));
        for (i, line) in lines.iter().enumerate() {
            assert!(
                is_rat_or_rup(&acc, line),
                "proof line {i} is neither RUP nor RAT: {line:?}"
            );
            acc.push(line.clone());
        }
        assert_eq!(lines.last().unwrap().len(), 0, "proof must end empty");
    }

    #[test]
    fn detects_rphp_plain() {
        let cls = build_rphp(4, 6, 3);
        let s = detect(&cls, 4).expect("rphp detected");
        assert_eq!((s.pigeons(), s.places(), s.holes()), (4, 6, 3));
    }

    #[test]
    fn detects_rphp_shuffled_and_flipped() {
        let cls = shuffle_flip(&build_rphp(4, 6, 3), 0x5eed);
        let s = detect(&cls, 4).expect("shuffled rphp detected");
        assert_eq!((s.pigeons(), s.places(), s.holes()), (4, 6, 3));
    }

    #[test]
    fn detects_clqcl_plain() {
        let cls = build_clqcl(4, 6, 3);
        let s = detect(&cls, 4).expect("clqcl detected");
        assert_eq!((s.pigeons(), s.places(), s.holes()), (4, 6, 3));
    }

    #[test]
    fn detects_clqcl_shuffled_and_flipped() {
        let cls = shuffle_flip(&build_clqcl(4, 6, 3), 0xabcd);
        let s = detect(&cls, 4).expect("shuffled clqcl detected");
        assert_eq!((s.pigeons(), s.places(), s.holes()), (4, 6, 3));
    }

    #[test]
    fn declines_satisfiable_variant() {
        // As many holes as pigeons: satisfiable, must not detect.
        assert!(detect(&build_rphp(3, 6, 3), 4).is_none());
        assert!(detect(&build_clqcl(3, 6, 3), 4).is_none());
    }

    #[test]
    fn declines_missing_conflict_clause() {
        let mut cls = build_rphp(4, 6, 3);
        let victim = cls
            .iter()
            .position(|c| c.len() == 4)
            .expect("has a conflict clause");
        cls.remove(victim);
        assert!(detect(&cls, 4).is_none());

        let mut cls = build_clqcl(4, 6, 3);
        let victim = cls
            .iter()
            .position(|c| c.len() == 3 && c.iter().all(|&l| l < 0))
            .expect("has a color conflict clause");
        cls.remove(victim);
        assert!(detect(&cls, 4).is_none());
    }

    #[test]
    fn declines_unstructured_formula() {
        let cls: Vec<Vec<i32>> = vec![
            (1..=10).collect(),
            (11..=20).collect(),
            (21..=30).collect(),
            vec![-1, -11],
            vec![-2, -12],
        ];
        assert!(detect(&cls, 4).is_none());
    }

    #[test]
    fn rphp_proof_is_valid_drat() {
        check_full_proof(&build_rphp(4, 6, 3));
    }

    #[test]
    fn rphp_shuffled_proof_is_valid_drat() {
        check_full_proof(&shuffle_flip(&build_rphp(4, 6, 3), 0xfeed));
    }

    #[test]
    fn clqcl_proof_is_valid_drat() {
        check_full_proof(&build_clqcl(4, 6, 3));
    }

    #[test]
    fn extra_pigeons_still_refute() {
        // Six pigeons into three holes: proof uses only the first four.
        check_full_proof(&build_rphp(6, 7, 3));
    }

    /// Direct pigeonhole (SESSION 14c, harder-fphp class): p pigeons straight
    /// into h holes, per-hole AMO across pigeons plus per-pigeon AMO (the
    /// "functional" extras fphp carries; they must not confuse detection).
    fn build_direct_php(p: usize, h: usize) -> Vec<Vec<i32>> {
        let x = |pi: usize, hh: usize| (pi * h + hh + 1) as i32;
        let mut cls: Vec<Vec<i32>> = Vec::new();
        for pi in 0..p {
            cls.push((0..h).map(|hh| x(pi, hh)).collect());
        }
        for hh in 0..h {
            for p1 in 0..p {
                for p2 in (p1 + 1)..p {
                    cls.push(vec![-x(p1, hh), -x(p2, hh)]);
                }
            }
        }
        for pi in 0..p {
            for h1 in 0..h {
                for h2 in (h1 + 1)..h {
                    cls.push(vec![-x(pi, h1), -x(pi, h2)]);
                }
            }
        }
        cls
    }

    #[test]
    fn detects_direct_php_plain() {
        let s = detect(&build_direct_php(5, 4), 4).expect("direct php detected");
        assert!(s.direct);
        assert_eq!((s.pigeons(), s.places(), s.holes()), (5, 4, 4));
    }

    #[test]
    fn detects_direct_php_shuffled_and_flipped() {
        let cls = shuffle_flip(&build_direct_php(5, 4), 0xd1ec7);
        let s = detect(&cls, 4).expect("shuffled direct php detected");
        assert!(s.direct);
        assert_eq!((s.pigeons(), s.places(), s.holes()), (5, 4, 4));
    }

    #[test]
    fn direct_php_proof_is_valid_drat() {
        check_full_proof(&build_direct_php(5, 4));
    }

    #[test]
    fn direct_php_shuffled_proof_is_valid_drat() {
        check_full_proof(&shuffle_flip(&build_direct_php(5, 4), 0xfab));
    }

    /// N == H+1 collision (rphp_p25_r25 class): the used->hole covers have
    /// the SAME length as the pigeon covers, so detection must partition the
    /// longest class by AMO connectivity before building the pigeon set.
    #[test]
    fn detects_rphp_hole_cover_length_collision() {
        let cls = build_rphp(4, 4, 3);
        let s = detect(&cls, 4).expect("collision rphp detected");
        assert!(!s.direct);
        assert_eq!((s.pigeons(), s.places(), s.holes()), (4, 4, 3));
    }

    #[test]
    fn rphp_hole_cover_collision_proof_is_valid_drat() {
        check_full_proof(&build_rphp(4, 4, 3));
    }

    #[test]
    fn rphp_hole_cover_collision_shuffled_proof_is_valid_drat() {
        check_full_proof(&shuffle_flip(&build_rphp(4, 4, 3), 0xc0de));
    }

    /// H > 8 routes the closing stage through the polynomial inductive
    /// reduction instead of the factorial DFS (SESSION 14c); both the relay
    /// and direct shapes must stay drat-trim-valid there.
    #[test]
    fn inductive_relay_proof_is_valid_drat() {
        check_full_proof(&build_rphp(11, 12, 10));
    }

    #[test]
    fn inductive_direct_proof_is_valid_drat() {
        check_full_proof(&build_direct_php(11, 10));
    }

    /// End-to-end drat-trim verification when the checker binary is present
    /// (mirrors the gauss.rs test-harness pattern).
    fn drat_trim_verify(clauses: &[Vec<i32>]) {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let drat = std::env::var("DRAT_TRIM")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| manifest.join("../../tools/checkers/drat-trim/drat-trim"));
        if !drat.exists() {
            eprintln!("drat-trim not found at {drat:?}; skipping proof check");
            return;
        }
        let s = detect(clauses, 4).expect("detected");
        let nv = max_var(clauses);
        let mut proof = String::new();
        refute_with_proof(&s, nv, &mut |c| {
            for &l in c {
                proof.push_str(&l.to_string());
                proof.push(' ');
            }
            proof.push_str("0\n");
        });
        let mut cnf = format!("p cnf {} {}\n", nv, clauses.len());
        for c in clauses {
            for &l in c {
                cnf.push_str(&l.to_string());
                cnf.push(' ');
            }
            cnf.push_str("0\n");
        }
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        // Unique per call: tests sharing (pid, nv) — e.g. the plain and
        // shuffled rphp tests — otherwise race on the same paths under the
        // parallel test runner (observed as a flaky s NOT VERIFIED).
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let cnf_path = dir.join(format!("php_cnf_{pid}_{nv}_{seq}.cnf"));
        let proof_path = dir.join(format!("php_proof_{pid}_{nv}_{seq}.drat"));
        std::fs::write(&cnf_path, cnf).unwrap();
        std::fs::write(&proof_path, proof).unwrap();
        let out = std::process::Command::new(&drat)
            .arg(&cnf_path)
            .arg(&proof_path)
            .output()
            .expect("failed to run drat-trim");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let _ = std::fs::remove_file(&cnf_path);
        let _ = std::fs::remove_file(&proof_path);
        assert!(
            stdout.contains("s VERIFIED"),
            "drat-trim must verify the proof: {stdout}"
        );
    }

    #[test]
    fn drat_trim_verifies_rphp() {
        drat_trim_verify(&build_rphp(4, 6, 3));
    }

    #[test]
    fn drat_trim_verifies_rphp_shuffled() {
        drat_trim_verify(&shuffle_flip(&build_rphp(4, 6, 3), 0x1234));
    }

    #[test]
    fn drat_trim_verifies_clqcl() {
        drat_trim_verify(&build_clqcl(4, 6, 3));
    }
}
