//! Frontier-sweep counting refutation for exactly-one bipartite cover
//! imbalance — the mutilated-chessboard class (SESSION 19).
//!
//! # Detected shape
//!
//! A formula is an *EO bipartite cover* when its clauses partition into:
//! - per-cell at-least-one clauses (all-positive literals), and
//! - the complete per-cell pairwise at-most-one binaries (all-negative),
//!
//! such that every variable occurs in exactly TWO cells, and the cell graph
//! (variables as edges) is connected and 2-colorable. Each variable is then a
//! potential "domino" covering one cell of each color, and the formula states
//! a perfect matching between the color classes. If the classes have unequal
//! sizes the formula is UNSAT by counting — exponentially hard for
//! resolution/CDCL (mutilated chessboard), polynomial in extended resolution.
//!
//! # The proof
//!
//! Sweep cells in a bandwidth-minimized order c_1..c_n. After cell i, an edge
//! is OPEN if exactly one endpoint has been swept; classify open TRUE edges by
//! the color of their seen endpoint: FB_i (seen-black) and FW_i (seen-white).
//! With delta_i = #black-swept − #white-swept, every model satisfies
//!
//!     FB_i − FW_i = delta_i                                   (invariant)
//!
//! because each cell's exactly-one contributes inc + dec = 1: at a black cell
//! the single true incident edge either OPENS (FB +1) or CLOSES a white-seen
//! open edge (FW −1), and either way the difference gains exactly the +1 that
//! delta gains. At the end the frontier is empty (FB_n = FW_n = 0) while
//! delta_n = imbalance ≥ 1 — contradiction.
//!
//! The DRAT encoding maintains, per boundary, unary sequential counters over
//! the statically-known open-edge lists (band-capped), and derives the
//! invariant as two families of level lemmas:
//!
//!     Bge[i][a] → Wge[i][a − delta_i]        (dir1: FB ≥ a ⇒ FW ≥ a − δ)
//!     Wge[i][b] → Bge[i][b + delta_i]        (dir2: FW ≥ b ⇒ FB ≥ b + δ)
//!
//! where a level lemma whose consequent exceeds the counter's list length is
//! emitted with the consequent dropped (count ≥ k is constant-false when the
//! list has < k elements). The dir2 instance at b = 0 on the LAST boundary is
//! then the empty-consequent, empty-antecedent clause — the empty clause —
//! because FW_n ≥ 0 is trivial and FB_n ≥ delta_n is constant-false over the
//! empty final frontier.
//!
//! Counter mechanics: appends (a cell's opening edges join its own color's
//! list) EXTEND the existing sequential-counter chain — free, no bridge.
//! Removals (a cell's closing edges leave the OTHER color's list) build a
//! fresh counter over the compacted list plus a per-prefix bridge battery
//! conditioned on dec_i (the OR of the closing edges): at most one closing
//! edge is true (the cell's AMO), so the removal shifts levels by exactly
//! dec_i. All definitions put the fresh literal first (drat-trim RAT pivot);
//! all lemmas are RUP against previously emitted lines.
//!
//! Size: with frontier width ≤ W and band ≤ B, the proof is
//! O(cells · W · B) lines — mchess_20 (W ≈ 21) lands well inside the
//! drat-trim envelope measured for the php engine (RAT-scan law).

use std::collections::HashMap;

/// Hard caps: decline anything that would blow the proof budget.
pub(crate) const MAX_CELLS: usize = 4096;
pub(crate) const MAX_FRONTIER: usize = 28;
/// Counter band: levels tracked per counter. The invariant chain needs
/// headroom of max |delta| plus the largest per-cell shift; the imbalance
/// itself is small (2 for mchess). Saturating above the band is sound: level
/// lemmas are only emitted for in-band levels.
pub(crate) const BAND: usize = 14;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Color {
    Black,
    White,
}

#[derive(Clone, Debug)]
pub(crate) struct SweepCell {
    pub(crate) color: Color,
    /// positive edge variable ids incident to this cell
    pub(crate) edges: Vec<i32>,
}

#[derive(Debug)]
pub(crate) struct SweepStructure {
    /// cells in sweep order
    pub(crate) cells: Vec<SweepCell>,
    /// black-cell surplus (#black − #white), > 0
    pub(crate) imbalance: i64,
    /// max open-edge count over all boundaries (for budget estimates)
    pub(crate) width: usize,
}

/// Detect the EO-bipartite-cover shape. Strict and all-or-nothing: any clause
/// that is not an all-positive cover clause or an all-negative binary makes
/// detection fail, so ordinary formulas decline after a cheap partition scan.
pub(crate) fn detect(clauses: &[Vec<i32>]) -> Option<SweepStructure> {
    let mut covers: Vec<Vec<i32>> = Vec::new();
    let mut amo: HashMap<(i32, i32), ()> = HashMap::new();
    for c in clauses {
        if c.is_empty() || c.len() > 8 {
            return None;
        }
        if c.iter().all(|&l| l > 0) {
            covers.push(c.clone());
        } else if c.len() == 2 && c.iter().all(|&l| l < 0) {
            let (a, b) = (-c[0].min(c[1]), -c[0].max(c[1]));
            // key sorted ascending by var
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            amo.insert((lo, hi), ());
        } else {
            return None;
        }
    }
    if covers.len() < 4 || covers.len() > MAX_CELLS {
        return None;
    }
    // Every var in exactly two covers; covers' AMO sets complete.
    let mut occ: HashMap<i32, Vec<usize>> = HashMap::new();
    for (ci, c) in covers.iter().enumerate() {
        let mut seen = c.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != c.len() {
            return None;
        }
        for &v in c {
            occ.entry(v).or_default().push(ci);
        }
        for x in 0..c.len() {
            for y in (x + 1)..c.len() {
                let (lo, hi) = if c[x] < c[y] { (c[x], c[y]) } else { (c[y], c[x]) };
                if !amo.contains_key(&(lo, hi)) {
                    return None;
                }
            }
        }
    }
    for cells in occ.values() {
        if cells.len() != 2 || cells[0] == cells[1] {
            return None;
        }
    }
    // 2-color the cell graph by BFS; require connected & bipartite.
    let n = covers.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for cells in occ.values() {
        adj[cells[0]].push(cells[1]);
        adj[cells[1]].push(cells[0]);
    }
    let mut color: Vec<Option<Color>> = vec![None; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    // Start from a minimum-degree cell (bandwidth heuristic: corner first).
    let start = (0..n).min_by_key(|&i| adj[i].len())?;
    color[start] = Some(Color::Black);
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(start);
    while let Some(u) = queue.pop_front() {
        order.push(u);
        // Deterministic neighbor order: sort by (degree, index) for a
        // Cuthill-McKee-flavored low-bandwidth sweep.
        let mut nb: Vec<usize> = adj[u].clone();
        nb.sort_unstable_by_key(|&v| (adj[v].len(), v));
        for v in nb {
            let want = match color[u].unwrap() {
                Color::Black => Color::White,
                Color::White => Color::Black,
            };
            match color[v] {
                None => {
                    color[v] = Some(want);
                    queue.push_back(v);
                }
                Some(c) if c != want => return None,
                _ => {}
            }
        }
    }
    if order.len() != n {
        return None; // disconnected: keep it simple, decline
    }
    // Candidate sweep orders: the ORIGINAL cover order first (generators
    // emit cells row-major, which has both low frontier width and small
    // running imbalance on grid instances), then the BFS order (robust to
    // shuffled inputs, at the cost of a diagonal wavefront). Score each by
    // (frontier width, max running |delta|); take the first feasible one.
    let orders: [Vec<usize>; 2] = [(0..n).collect(), order.clone()];
    let mut chosen: Option<(Vec<usize>, usize)> = None;
    for cand in &orders {
        let pos: HashMap<usize, usize> = cand.iter().enumerate().map(|(i, &c)| (c, i)).collect();
        let mut width = 0usize;
        let mut open = 0i64;
        let mut d: i64 = 0;
        let mut max_d = 0i64;
        for (i, &ci) in cand.iter().enumerate() {
            d += match color[ci].unwrap() {
                Color::Black => 1,
                Color::White => -1,
            };
            max_d = max_d.max(d.abs());
            for &v in &covers[ci] {
                let cs = &occ[&v];
                let other = if cs[0] == ci { cs[1] } else { cs[0] };
                if pos[&other] > i {
                    open += 1;
                } else {
                    open -= 1;
                }
            }
            width = width.max(open as usize);
        }
        if width <= MAX_FRONTIER && (max_d as usize) + 4 <= BAND {
            chosen = Some((cand.clone(), width));
            break;
        }
    }
    let (order, width) = chosen?;
    let blacks = color.iter().flatten().filter(|&&c| c == Color::Black).count() as i64;
    let whites = n as i64 - blacks;
    let mut imbalance = blacks - whites;
    let mut flip = false;
    if imbalance == 0 {
        return None; // balanced: no counting contradiction
    }
    if imbalance < 0 {
        imbalance = -imbalance;
        flip = true; // relabel so Black is the surplus color
    }
    // Build sweep cells in the chosen order.
    let mut cells: Vec<SweepCell> = order
        .iter()
        .map(|&ci| {
            let mut col = color[ci].unwrap();
            if flip {
                col = match col {
                    Color::Black => Color::White,
                    Color::White => Color::Black,
                };
            }
            SweepCell {
                color: col,
                edges: covers[ci].clone(),
            }
        })
        .collect();
    for c in cells.iter_mut() {
        c.edges.sort_unstable();
    }
    Some(SweepStructure {
        cells,
        imbalance,
        width,
    })
}

/// Proof-size estimate (lines), for the caller's budget gate.
pub(crate) fn estimated_proof_lines(s: &SweepStructure) -> u64 {
    // per cell: counter defs O(W·B·4) worst case + bridges O(W·B·2) + invariant O(B·2)
    (s.cells.len() as u64) * ((s.width as u64) * (BAND as u64) * 6 + (BAND as u64) * 4 + 16)
}

/// One sequential unary counter over an ordered list of edge literals.
/// `levels[j][a-1]` = fresh var for "at least a of the first j+1 list
/// elements are true", allocated only for a ≤ min(j+1, BAND).
struct Counter {
    list: Vec<i32>,
    levels: Vec<Vec<i32>>,
}

/// A counter level's status: constant-true (a = 0), constant-false (level
/// exceeds the list length), a tracked variable, or untracked (level beyond
/// the BAND cap but within the list — the saturating counter holds no var
/// for it, and no lemma may mention it).
#[derive(Clone, Copy, Debug)]
enum Lv {
    True,
    False,
    Var(i32),
    Untracked,
}

impl Lv {
    fn neg(self) -> Lv {
        match self {
            Lv::True => Lv::False,
            Lv::False => Lv::True,
            Lv::Var(v) => Lv::Var(-v),
            Lv::Untracked => Lv::Untracked,
        }
    }
}

impl Counter {
    fn empty() -> Self {
        Counter {
            list: Vec::new(),
            levels: Vec::new(),
        }
    }
    /// "count over the first `prefix` elements ≥ a".
    fn lv_at(&self, prefix: usize, a: i64) -> Lv {
        if a <= 0 {
            return Lv::True;
        }
        let a = a as usize;
        if a > prefix {
            return Lv::False;
        }
        if a > BAND {
            return Lv::Untracked;
        }
        Lv::Var(self.levels[prefix - 1][a - 1])
    }
    /// "count over the full list ≥ a".
    fn lv(&self, a: i64) -> Lv {
        self.lv_at(self.list.len(), a)
    }
}

/// Negate an optional counter level: None (constant-false) negates to the
/// constant-true sentinel Some(0) (vacuous clause); Some(0) negates to
/// constant-false None; a real var negates normally.
fn neg_opt(l: Option<i32>) -> Option<i32> {
    match l {
        None => Some(0),
        Some(0) => None,
        Some(v) => Some(-v),
    }
}

/// Emission context: allocates fresh vars, forwards clauses. When
/// `SWEEPCOUNT_ANNOTATE` is set (tests), every line's provenance tag is
/// collected in `tags` for drat-trim failure triage.
pub(crate) struct Emitter<'a> {
    next_var: i32,
    pub(crate) lines: u64,
    emit: &'a mut dyn FnMut(&[i32]),
    annotate: bool,
    pub(crate) tags: Vec<String>,
    tag: String,
}

impl<'a> Emitter<'a> {
    pub(crate) fn new(num_vars: usize, emit: &'a mut dyn FnMut(&[i32])) -> Self {
        Emitter {
            next_var: num_vars as i32 + 1,
            lines: 0,
            emit,
            annotate: std::env::var("SWEEPCOUNT_ANNOTATE").is_ok(),
            tags: Vec::new(),
            tag: String::new(),
        }
    }
    fn set_tag(&mut self, t: &str) {
        if self.annotate {
            self.tag = t.to_string();
        }
    }
    fn fresh(&mut self) -> i32 {
        let v = self.next_var;
        self.next_var += 1;
        v
    }
    fn clause(&mut self, lits: &[i32]) {
        self.lines += 1;
        if self.annotate {
            self.tags.push(format!("{} :: {:?}", self.tag, lits));
        }
        (self.emit)(lits);
    }
    /// Emit clause dropping `None` (constant-false) consequents and
    /// short-circuiting on sentinel-0 (constant-true) literals.
    fn clause_opt(&mut self, lits: &[Option<i32>]) {
        let mut out: Vec<i32> = Vec::with_capacity(lits.len());
        for l in lits {
            match l {
                Some(0) => return, // constant-true literal: clause vacuous
                Some(x) => out.push(*x),
                None => {} // constant-false literal: drop
            }
        }
        self.clause(&out);
    }

    /// Emit a lemma over counter levels: constant-true literal ⇒ vacuous
    /// (skip); constant-false ⇒ drop; Untracked ⇒ the lemma cannot be
    /// stated in the banded system — skip it entirely (sound: lemmas are
    /// derived facts, omitting one only weakens later RUP contexts, and the
    /// invariant chain never needs untracked levels when BAND ≥ the alive
    /// band; debug builds assert instead).
    fn lemma(&mut self, lits: &[Lv]) {
        let mut out: Vec<i32> = Vec::with_capacity(lits.len());
        for l in lits {
            match l {
                Lv::True => return,
                Lv::False => {}
                Lv::Var(v) => out.push(*v),
                Lv::Untracked => return,
            }
        }
        self.clause(&out);
    }

    /// Define `v ≡ OR(lits)`; returns v. Fresh literal leads each clause.
    fn define_or(&mut self, lits: &[i32]) -> i32 {
        let v = self.fresh();
        let mut long = Vec::with_capacity(lits.len() + 1);
        long.push(-v);
        long.extend_from_slice(lits);
        self.clause(&long);
        for &l in lits {
            self.clause(&[v, -l]);
        }
        v
    }

    /// Extend `c` by appending `edges`, defining new levels chain-style.
    fn counter_extend(&mut self, c: &mut Counter, edges: &[i32]) {
        for &e in edges {
            let j = c.list.len(); // prefix length before this element
            c.list.push(e);
            let cap = (j + 1).min(BAND);
            let mut row: Vec<i32> = Vec::with_capacity(cap);
            for a in 1..=cap {
                let v = self.fresh();
                // v ≡ p_a ∨ (p_{a−1} ∧ e); within cap, p_a is Var or False
                // and p_{a−1} is Var or True — never Untracked.
                let p_a = c.lv_at(j, a as i64);
                let p_am1 = c.lv_at(j, a as i64 - 1);
                // forward: (¬v ∨ p_a ∨ p_{a−1}) ; (¬v ∨ p_a ∨ e)
                match p_am1 {
                    Lv::True => {
                        // a == 1: v ≡ p_1 ∨ e
                        match p_a {
                            Lv::Var(pa) => self.clause(&[-v, pa, e]),
                            Lv::False => self.clause(&[-v, e]),
                            _ => unreachable!(),
                        }
                    }
                    Lv::Var(pm) => {
                        match p_a {
                            Lv::Var(pa) => {
                                self.clause(&[-v, pa, pm]);
                                self.clause(&[-v, pa, e]);
                            }
                            Lv::False => {
                                self.clause(&[-v, pm]);
                                self.clause(&[-v, e]);
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => unreachable!(),
                }
                // backward: (v ∨ ¬p_a) ; (v ∨ ¬p_{a−1} ∨ ¬e)
                if let Lv::Var(pa) = p_a {
                    self.clause(&[v, -pa]);
                }
                match p_am1 {
                    Lv::True => self.clause(&[v, -e]),
                    Lv::Var(pm) => self.clause(&[v, -pm, -e]),
                    _ => unreachable!(),
                }
                row.push(v);
            }
            c.levels.push(row);
            // M battery: level monotonicity at the new prefix
            // (¬v_{p,a} ∨ v_{p,a−1}), RUP ascending through the previous
            // prefix's M lemmas and this prefix's definitions.
            let p = c.list.len();
            for a in 2..=(p.min(BAND) as i64) {
                let hi = c.lv_at(p, a);
                let lo = c.lv_at(p, a - 1);
                self.lemma(&[hi.neg(), lo]);
            }
        }
    }

    /// Build a fresh counter over `list` (used after removals), no bridges.
    fn counter_build(&mut self, list: &[i32]) -> Counter {
        let mut c = Counter::empty();
        self.counter_extend(&mut c, list);
        c
    }
}

/// Emit the refutation. `emit` receives each DRAT line (added clause).
/// The final line is the empty clause.
pub(crate) fn refute_with_proof(
    s: &SweepStructure,
    num_vars: usize,
    emit: &mut dyn FnMut(&[i32]),
) -> Vec<String> {
    let mut em = Emitter::new(num_vars, emit);

    // Static open-edge bookkeeping: for every edge var, its two cell indices.
    let mut homes: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, c) in s.cells.iter().enumerate() {
        for &e in &c.edges {
            homes.entry(e).or_default().push(i);
        }
    }

    let mut fb = Counter::empty(); // seen-Black true open edges
    let mut fw = Counter::empty(); // seen-White true open edges
    let mut delta: i64 = 0;

    // Invariant lemma presence tracker: inv1[a] emitted ⇔ clause
    // (¬Bge_a ∨ Wge_{a−δ}) live for current boundary; inv2 similarly.
    // (We simply re-emit per boundary; RUP checks use the newest.)

    for (i, cell) in s.cells.iter().enumerate() {
        let (opening, closing): (Vec<i32>, Vec<i32>) = cell
            .edges
            .iter()
            .partition(|&&e| homes[&e].iter().any(|&h| h > i));
        // inc/dec definitions for this cell
        em.set_tag(&format!("cell{i} inc/dec defs"));
        let inc = if opening.is_empty() {
            None
        } else {
            Some(em.define_or(&opening))
        };
        let dec = if closing.is_empty() {
            None
        } else {
            Some(em.define_or(&closing))
        };
        // EO lemmas: (inc ∨ dec) — from the cover clause via OR defs;
        // (¬inc ∨ ¬dec) — via per-pair AMO axioms and helper lemmas.
        match (inc, dec) {
            (Some(iv), Some(dv)) => {
                em.clause(&[iv, dv]);
                for &a in &opening {
                    // (¬a ∨ ¬dec): a true forbids any closing edge (AMO), so
                    // dec's long def clause falsifies. RUP.
                    em.clause(&[-a, -dv]);
                }
                em.clause(&[-iv, -dv]);
            }
            (Some(iv), None) => em.clause(&[iv]),
            (None, Some(dv)) => em.clause(&[dv]),
            (None, None) => {
                // A cell with no edges at all: its cover clause is empty —
                // cannot happen (detect requires non-empty covers).
                unreachable!("empty cell");
            }
        }

        // Counter updates.
        let (own, other, own_is_b) = match cell.color {
            Color::Black => (&mut fb, &mut fw, true),
            Color::White => (&mut fw, &mut fb, false),
        };
        // 1. own-color: append opening edges (free chain extension), then
        //    emit the extend batteries E1-E3 relating pre and post levels.
        em.set_tag(&format!("cell{i} own-extend"));
        let pre_len = own.list.len();
        em.counter_extend(own, &opening);
        em.set_tag(&format!("cell{i} extend-batteries"));
        // E3 monotone: (¬pre_x ∨ post_x) — chain of backward defs.
        for x in 1..=(pre_len.min(BAND) as i64) {
            em.lemma(&[own.lv_at(pre_len, x).neg(), own.lv(x)]);
        }
        // E1 per-edge shift: (¬pre_x ∨ ¬e_k ∨ post_{x+1}); x = 0 row is the
        // bare (¬e_k ∨ post_1) form.
        for &e in &opening {
            for x in 0..=(pre_len.min(BAND) as i64) {
                em.lemma(&[own.lv_at(pre_len, x).neg(), Lv::Var(-e), own.lv(x + 1)]);
            }
        }
        // E2 inc lift: (¬pre_x ∨ ¬inc ∨ post_{x+1}) — RUP via E1 + inc long def.
        if let Some(iv) = inc {
            for x in 0..=(pre_len.min(BAND) as i64) {
                em.lemma(&[own.lv_at(pre_len, x).neg(), Lv::Var(-iv), own.lv(x + 1)]);
            }
        }
        // Reverse extend batteries (post → pre):
        //  H1 per appended edge e at position pos(e), per prefix q ≥ pos(e),
        //  per level x: (¬e ∨ ¬v_{q,x} ∨ v_{pre,x−1}) — with e true the cell
        //  AMO silences the other appended edges, so the walk from v_{q,x}
        //  down to the pre prefix loses exactly one.
        //  H0: (inc ∨ ¬post_x ∨ pre_x) — no appended true ⇒ count preserved.
        //  REV: (¬post_{x+1} ∨ pre_x) — assembled from H0/H1 + inc's defs.
        {
            let full = own.list.len();
            for (k, &e) in opening.iter().enumerate() {
                let pos = pre_len + k + 1;
                for q in pos..=full {
                    for x in 1..=(q.min(BAND) as i64) {
                        em.lemma(&[
                            Lv::Var(-e),
                            own.lv_at(q, x).neg(),
                            own.lv_at(pre_len, x - 1),
                        ]);
                    }
                }
            }
            if let Some(iv) = inc {
                for x in 1..=(full.min(BAND) as i64) {
                    em.lemma(&[Lv::Var(iv), own.lv(x).neg(), own.lv_at(pre_len, x)]);
                }
            }
            for x in 1..=(full.min(BAND) as i64 - 1) {
                em.lemma(&[own.lv(x + 1).neg(), own.lv_at(pre_len, x)]);
            }
        }
        // 2. other-color: remove closing edges → fresh counter + bridge.
        em.set_tag(&format!("cell{i} other-rebuild"));
        let old_other = std::mem::replace(other, Counter::empty());
        let new_list: Vec<i32> = old_other
            .list
            .iter()
            .copied()
            .filter(|e| !closing.contains(e))
            .collect();
        let new_other = em.counter_build(&new_list);
        em.set_tag(&format!("cell{i} bridge"));
        // Bridge battery over aligned prefixes (levels inner, prefix outer):
        //  D1 down:      (¬old_{j,a} ∨ new_{j',a−1})
        //  D2 preserve:  (dec ∨ ¬old_{j,a} ∨ new_{j',a})
        //  D3 converse:  (¬new_{j',a} ∨ old_{j,a})
        //  D4 up-shift:  (¬e_r ∨ ¬new_{j',a} ∨ old_{j,a+1})  per removed r ≤ j
        {
            let mut jn = 0usize;
            let mut removed_seen: Vec<i32> = Vec::new();
            for j in 1..=old_other.list.len() {
                let e = old_other.list[j - 1];
                let removed = closing.contains(&e);
                if removed {
                    removed_seen.push(e);
                }
                if !removed {
                    jn += 1;
                }
                // T battery: for each removed edge r positioned AFTER this
                // prefix, r true silences the removals inside the prefix (cell
                // AMO), so counts transfer 1:1: (¬r ∨ ¬old_{j,a} ∨ new_{j',a}).
                for &r in &closing {
                    let rpos = old_other.list.iter().position(|&x| x == r);
                    if let Some(rp) = rpos {
                        if rp + 1 > j {
                            for a in 1..=(j.min(BAND) as i64) {
                                em.lemma(&[
                                    Lv::Var(-r),
                                    old_other.lv_at(j, a).neg(),
                                    new_other.lv_at(jn, a),
                                ]);
                            }
                        }
                    }
                }
                // a = 0 row for D4: (¬e_r ∨ old_{j,1}).
                for &r in &removed_seen {
                    em.lemma(&[Lv::Var(-r), old_other.lv_at(j, 1)]);
                }
                for a in 1..=(j.min(BAND) as i64) {
                    let o = old_other.lv_at(j, a);
                    if removed_seen.is_empty() {
                        let nn = new_other.lv_at(jn, a);
                        em.lemma(&[o.neg(), nn]);
                        em.lemma(&[nn.neg(), o]);
                    } else {
                        let dl = dec.map(Lv::Var).unwrap_or(Lv::False);
                        em.lemma(&[o.neg(), new_other.lv_at(jn, a - 1)]);
                        em.lemma(&[dl, o.neg(), new_other.lv_at(jn, a)]);
                        em.lemma(&[new_other.lv_at(jn, a).neg(), o]);
                        for &r in &removed_seen {
                            em.lemma(&[
                                Lv::Var(-r),
                                new_other.lv_at(jn, a).neg(),
                                old_other.lv_at(j, a + 1),
                            ]);
                        }
                    }
                }
            }
            debug_assert_eq!(jn, new_list.len());
        }
        // D5 dec lift at full lists: (¬dec ∨ ¬new_a ∨ old_{a+1}) plus the
        // a = 0 row (¬dec ∨ old_1) — RUP via D4 + dec long def.
        if let Some(dv) = dec {
            em.lemma(&[Lv::Var(-dv), old_other.lv(1)]);
            for a in 1..=(BAND as i64) {
                em.lemma(&[
                    Lv::Var(-dv),
                    new_other.lv(a).neg(),
                    old_other.lv(a + 1),
                ]);
            }
        }
        *other = new_other;

        // delta update
        delta += if own_is_b { 1 } else { -1 };
        let _ = own;

        // 3. Invariant assembly for the new boundary — RUP chains through the
        //    previous invariant + extend batteries + bridge batteries.
        em.set_tag(&format!("cell{i} invariant d={delta}"));
        let (fb_r, fw_r) = (&fb, &fw);
        for lvl in 0..=(BAND as i64) {
            for dir in 0..2 {
                let (ant_c, cons_c, cons_lvl) = if dir == 0 {
                    (fb_r, fw_r, lvl - delta)
                } else {
                    (fw_r, fb_r, lvl + delta)
                };
                if cons_lvl < 0 || (lvl == 0 && cons_lvl == 0) {
                    continue;
                }
                let ant = if lvl == 0 { Lv::True } else { ant_c.lv(lvl) };
                em.lemma(&[ant.neg(), cons_c.lv(cons_lvl)]);
            }
        }
    }

    debug_assert_eq!(delta, s.imbalance);
    // The last boundary's dir2 b=0 lemma with empty FB list and δ ≥ 1 emitted
    // the empty clause via clause_opt (consequent constant-false, antecedent
    // constant-true).
    em.tags
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the mutilated m×m board CNF: cells (r,c), remove (0,0) and
    /// (m−1,m−1) (same color when m even). Returns (clauses, num_vars).
    fn mutilated_board(m: usize) -> (Vec<Vec<i32>>, usize) {
        let removed = |r: usize, c: usize| (r == 0 && c == 0) || (r == m - 1 && c == m - 1);
        let mut edge_id: HashMap<((usize, usize), (usize, usize)), i32> = HashMap::new();
        let mut next = 1i32;
        let mut cell_edges: HashMap<(usize, usize), Vec<i32>> = HashMap::new();
        for r in 0..m {
            for c in 0..m {
                if removed(r, c) {
                    continue;
                }
                for (dr, dc) in [(0usize, 1usize), (1, 0)] {
                    let (r2, c2) = (r + dr, c + dc);
                    if r2 >= m || c2 >= m || removed(r2, c2) {
                        continue;
                    }
                    let v = next;
                    next += 1;
                    edge_id.insert(((r, c), (r2, c2)), v);
                    cell_edges.entry((r, c)).or_default().push(v);
                    cell_edges.entry((r2, c2)).or_default().push(v);
                }
            }
        }
        let mut clauses = Vec::new();
        let mut keys: Vec<(usize, usize)> = cell_edges.keys().copied().collect();
        keys.sort_unstable(); // row-major, like real instance generators
        for k in keys {
            let edges = &cell_edges[&k];
            clauses.push(edges.clone());
            for i in 0..edges.len() {
                for j in (i + 1)..edges.len() {
                    clauses.push(vec![-edges[i], -edges[j]]);
                }
            }
        }
        (clauses, (next - 1) as usize)
    }

    #[test]
    fn detects_mutilated_4x4() {
        let (clauses, _nv) = mutilated_board(4);
        let s = detect(&clauses).expect("4x4 mutilated board must be detected");
        assert_eq!(s.imbalance, 2);
        assert_eq!(s.cells.len(), 14);
        assert!(s.width <= 8, "width {}", s.width);
    }

    #[test]
    fn declines_balanced_and_ordinary() {
        // Balanced 2x2 board (no removals): imbalance 0 → decline.
        let mut clauses: Vec<Vec<i32>> = Vec::new();
        // 2x2 full board: 4 cells, 4 edges — build by hand: cells A=(0,0),
        // B=(0,1), C=(1,0), D=(1,1); edges AB=1, AC=2, BD=3, CD=4.
        clauses.push(vec![1, 2]); // A
        clauses.push(vec![1, 3]); // B
        clauses.push(vec![2, 4]); // C
        clauses.push(vec![3, 4]); // D
        for c in [[1, 2], [1, 3], [2, 4], [3, 4]] {
            clauses.push(vec![-c[0], -c[1]]);
        }
        assert!(detect(&clauses).is_none(), "balanced board must decline");
        // Ordinary formula: mixed-polarity clause → decline.
        let ordinary = vec![vec![1, -2], vec![2, 3]];
        assert!(detect(&ordinary).is_none());
    }

    fn verify_board(m: usize) {
        let (clauses, nv) = mutilated_board(m);
        let s = detect(&clauses).unwrap_or_else(|| panic!("{m}x{m} board must detect"));
        run_verify(&clauses, nv, &s, m);
    }

    #[test]
    fn refutes_mutilated_8x8_drat_verified() {
        verify_board(8);
    }

    #[test]
    fn refutes_mutilated_20x20_drat_verified() {
        verify_board(20);
    }

    /// End-to-end: emit the proof for the 4x4 board and check it with
    /// drat-trim if available (skips silently otherwise).
    #[test]
    fn refutes_mutilated_4x4_drat_verified() {
        let (clauses, nv) = mutilated_board(4);
        let s = detect(&clauses).expect("detect");
        run_verify(&clauses, nv, &s, 4);
    }

    fn run_verify(clauses: &[Vec<i32>], nv: usize, s: &SweepStructure, tag: usize) {
        let mut proof: Vec<Vec<i32>> = Vec::new();
        let tags = refute_with_proof(s, nv, &mut |c| proof.push(c.to_vec()));
        assert!(
            proof.last().map(|c| c.is_empty()).unwrap_or(false),
            "proof must end with the empty clause (last: {:?}, lines {})",
            proof.last(),
            proof.len()
        );
        // Write DIMACS + DRAT, run drat-trim when present.
        let candidates = [
            "../../tools/checkers/drat-trim/drat-trim",
            "tools/checkers/drat-trim/drat-trim",
        ];
        let Some(drat_trim) = candidates
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.is_file())
        else {
            eprintln!("drat-trim not found; skipping verification");
            return;
        };
        let dir = std::env::temp_dir().join(format!(
            "sweepcount-test-{}-{}",
            std::process::id(),
            tag
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cnf_path = dir.join("board.cnf");
        let proof_path = dir.join("board.drat");
        let mut cnf = String::new();
        cnf.push_str(&format!("p cnf {} {}\n", nv, clauses.len()));
        for c in clauses {
            for l in c {
                cnf.push_str(&format!("{} ", l));
            }
            cnf.push_str("0\n");
        }
        std::fs::write(&cnf_path, cnf).unwrap();
        let mut dr = String::new();
        for c in &proof {
            for l in c {
                dr.push_str(&format!("{} ", l));
            }
            dr.push_str("0\n");
        }
        std::fs::write(&proof_path, dr).unwrap();
        if !tags.is_empty() {
            let map: String = tags
                .iter()
                .enumerate()
                .map(|(i, t)| format!("{} {}\n", i + 1, t))
                .collect();
            std::fs::write(dir.join("board.map"), map).unwrap();
        }
        let out = std::process::Command::new(&drat_trim)
            .arg(&cnf_path)
            .arg(&proof_path)
            .output()
            .expect("run drat-trim");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("s VERIFIED"),
            "drat-trim must verify the 4x4 proof; output:\n{}",
            stdout
        );
    }
}
