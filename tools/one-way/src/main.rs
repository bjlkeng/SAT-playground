//! one-way: from which sides does unit propagation see an implied clause?
//!
//! Unit propagation is the solver's cheap rule: when every literal of a
//! clause but one is false, the last one is true.  Take a clause that the
//! formula implies, say "not P or b".  It has two sides.  From P's side: set
//! P, and propagation sets b.  From b's side: set b false, and propagation
//! sets P false.  An encoding can give one side and not the other.  The
//! sorted-bit numbers of Z3 do that: setting a bit false turns the test P
//! off, and setting P on sets no bit (solver README, "Numbers stored as
//! sorted bits").  This tool counts such clauses with no knowledge of the
//! encoding.
//!
//! Two-literal clauses (x or y), over variables that are open at the root:
//!   two-way   both sides see it
//!   one-way   one side sees it, the other does not
//!   hidden    no side sees it, yet setting both literals false makes
//!             propagation fail (one probe shows it)
//! Three-literal clauses (x or y or z) that have no such two-literal part:
//!   seen from 3, 2 or 1 of their sides (a side: set two literals false,
//!   propagation sets the third), or from none though the three together fail.
//!
//! A small example.  s -> a, s -> b and the clause "not a or not b or m":
//! setting s sets a and b, and the long clause then sets m.  Setting m false
//! leaves "not a or not b" and sets nothing.  So "not s or m" is one-way.
//!
//!   one-way measure FILE [--all] [--seed N] [--sources N] [--targets N]
//!           [--probe-sources N] [--probe-pairs N] [--joint N] [--work N]
//!           [--kinds FILE] [--examples N] [--eq-vars N]
//!       one line "c oneway key=value ...".  --all counts every pair and
//!       triple (small formulas); otherwise sources, targets and pairs are
//!       sampled, with a seed, and the counts are estimates.
//!   one-way close FILE OUT [--max-add N] [--work N]
//!       FILE plus the missing side of every one-way root clause (below),
//!       each added at once so that the later ones can lean on it, pass
//!       after pass until nothing is added.  Every added clause follows by
//!       unit propagation from the clauses before it (RUP).
//!
//!   one-way equal FILE OUT [--max-add N] [--work N]
//!       FILE plus the missing halves of the hidden equal variables, and
//!       the literals that follow a variable set on and set off alike
//!       (true in every model: two two-literal clauses, then the unit).
//!
//! Equal variables.  Set a variable v on, then off.  A literal h that
//! follows it both times is equal to v in every model.  When h's own side
//! does not see it (h on does not set v, or h off does not set v off), the
//! equality is hidden: propagation computes h from v and not v from h.
//! The helpers of a count over sorted bits are such h, and v the bit.
//!
//! Root clauses.  One missing link makes many one-way pairs: everything
//! above it against everything below it.  So the tool also counts the
//! clauses that the others follow from.  When s sets m through a clause of
//! three or more, and s itself is the lowest literal above all that
//! clause's false literals, "not s or m" is a root clause (a hyper-binary
//! resolvent, in solver terms).  It is one-way when m false does not set s
//! false.  In the example above "not s or m" is the one root clause.
//!
//! FILE is DIMACS, plain or .xz.  The 2026 suite is never read.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::process::{Command, Stdio};

type Lit = u32; // 2 * variable + (1 when negative)

#[inline]
fn lit_of(x: i64) -> Lit {
    2 * (x.unsigned_abs() as u32) + (x < 0) as u32
}
#[inline]
fn dimacs(l: Lit) -> i64 {
    let v = (l >> 1) as i64;
    if l & 1 == 1 {
        -v
    } else {
        v
    }
}

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x2545_F491_4F6C_DD1D)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        ((self.next() >> 11) as u128 * n as u128 >> 53) as usize
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
    /// The first `k` places of `v` become a uniform sample of it.
    fn sample_front<T>(&mut self, v: &mut [T], k: usize) {
        for i in 0..k.min(v.len()) {
            let j = i + self.below(v.len() - i);
            v.swap(i, j);
        }
    }
}

/// Unit propagation: lists for the two-literal clauses, two watched
/// literals for the longer ones, one trail.
struct Prop {
    nv: usize,
    val: Vec<i8>,
    trail: Vec<Lit>,
    head: usize,
    bins: Vec<Vec<Lit>>,
    watch: Vec<Vec<u32>>,
    lits: Vec<Lit>,
    start: Vec<u32>,
    occ: Vec<Vec<u32>>,
    work: u64,
    n_bin: usize,
    /// By variable, for a literal on the trail: the literal it hangs under in
    /// the tree of implications (0 for an assumption), how deep it is, and
    /// whether a clause of three or more set it.  A literal set by "u -> m"
    /// hangs under u.  A literal set by a longer clause hangs under the
    /// lowest literal that all the clause's false literals hang under.
    parent: Vec<Lit>,
    depth: Vec<u32>,
    longr: Vec<bool>,
}

impl Prop {
    fn empty(nv: usize) -> Prop {
        let n = 2 * nv + 2;
        Prop {
            nv,
            val: vec![0; n],
            trail: Vec::new(),
            head: 0,
            bins: vec![Vec::new(); n],
            watch: vec![Vec::new(); n],
            lits: Vec::new(),
            start: vec![0],
            occ: Vec::new(),
            work: 0,
            n_bin: 0,
            parent: vec![0; nv + 1],
            depth: vec![0; nv + 1],
            longr: vec![false; nv + 1],
        }
    }

    #[inline]
    fn assign(&mut self, l: Lit, parent: Lit, long: bool) {
        self.val[l as usize] = 1;
        self.val[(l ^ 1) as usize] = -1;
        self.trail.push(l);
        let v = (l >> 1) as usize;
        self.parent[v] = parent;
        self.depth[v] = if parent == 0 { 0 } else { self.depth[(parent >> 1) as usize] + 1 };
        self.longr[v] = long;
    }

    /// The lowest literal that both `a` and `b` hang under, 0 when there is none.
    fn meet(&mut self, mut a: Lit, mut b: Lit) -> Lit {
        loop {
            if a == b {
                return a;
            }
            if a == 0 || b == 0 {
                return 0;
            }
            self.work += 1;
            if self.depth[(a >> 1) as usize] >= self.depth[(b >> 1) as usize] {
                a = self.parent[(a >> 1) as usize];
            } else {
                b = self.parent[(b >> 1) as usize];
            }
        }
    }

    fn add_bin(&mut self, a: Lit, b: Lit) {
        self.bins[(a ^ 1) as usize].push(b);
        self.bins[(b ^ 1) as usize].push(a);
        self.n_bin += 1;
    }

    fn add_long(&mut self, c: &[Lit]) {
        let id = (self.start.len() - 1) as u32;
        self.lits.extend_from_slice(c);
        self.start.push(self.lits.len() as u32);
        self.watch[c[0] as usize].push(id);
        self.watch[c[1] as usize].push(id);
    }

    fn n_long(&self) -> usize {
        self.start.len() - 1
    }

    /// True on a conflict.
    fn propagate(&mut self) -> bool {
        while self.head < self.trail.len() {
            let l = self.trail[self.head];
            self.head += 1;
            let n = self.bins[l as usize].len();
            self.work += n as u64 + 1;
            for i in 0..n {
                let x = self.bins[l as usize][i];
                match self.val[x as usize] {
                    1 => {}
                    -1 => return true,
                    _ => self.assign(x, l, false),
                }
            }
            let f = l ^ 1;
            let mut ws = std::mem::take(&mut self.watch[f as usize]);
            let (mut i, mut j) = (0, 0);
            let mut conflict = false;
            while i < ws.len() {
                let c = ws[i] as usize;
                i += 1;
                let (a, b) = (self.start[c] as usize, self.start[c + 1] as usize);
                self.work += 1;
                if self.lits[a] == f {
                    self.lits.swap(a, a + 1);
                }
                let other = self.lits[a];
                if self.val[other as usize] == 1 {
                    ws[j] = c as u32;
                    j += 1;
                    continue;
                }
                let mut moved = false;
                for k in a + 2..b {
                    self.work += 1;
                    let x = self.lits[k];
                    if self.val[x as usize] >= 0 {
                        self.lits.swap(a + 1, k);
                        self.watch[x as usize].push(c as u32);
                        moved = true;
                        break;
                    }
                }
                if moved {
                    continue;
                }
                ws[j] = c as u32;
                j += 1;
                if self.val[other as usize] == -1 {
                    conflict = true;
                    while i < ws.len() {
                        ws[j] = ws[i];
                        j += 1;
                        i += 1;
                    }
                    break;
                }
                let mut under = self.lits[a + 1] ^ 1;
                for k in a + 2..b {
                    if under == 0 {
                        break;
                    }
                    let u = self.lits[k] ^ 1;
                    under = self.meet(under, u);
                }
                self.assign(other, under, true);
            }
            ws.truncate(j);
            self.watch[f as usize] = ws;
            if conflict {
                return true;
            }
        }
        false
    }

    /// Sets `l` and propagates; true on a conflict.
    fn assume(&mut self, l: Lit) -> bool {
        match self.val[l as usize] {
            1 => false,
            -1 => true,
            _ => {
                self.assign(l, 0, false);
                self.propagate()
            }
        }
    }

    fn undo(&mut self, mark: usize) {
        while self.trail.len() > mark {
            let l = self.trail.pop().unwrap();
            self.val[l as usize] = 0;
            self.val[(l ^ 1) as usize] = 0;
        }
        self.head = mark;
    }

    /// What `s` sets by propagation, without `s`; false when it fails.
    fn implied(&mut self, s: Lit, out: &mut Vec<Lit>) -> bool {
        out.clear();
        if self.val[s as usize] != 0 {
            return self.val[s as usize] == 1; // set at the root already: nothing new follows
        }
        let mark = self.trail.len();
        let bad = self.assume(s);
        if !bad {
            out.extend_from_slice(&self.trail[mark + 1..]);
        }
        self.undo(mark);
        !bad
    }

    /// As `implied`, and in `roots` the literals that a clause of three or
    /// more set with nothing but `s` itself above all its false literals.
    /// "not s or m" for such an m is what a solver calls a hyper-binary
    /// resolvent; every other consequence of `s` follows from these and from
    /// two-literal clauses.
    fn implied_roots(&mut self, s: Lit, out: &mut Vec<Lit>, roots: &mut Vec<Lit>) -> bool {
        out.clear();
        roots.clear();
        if self.val[s as usize] != 0 {
            return self.val[s as usize] == 1;
        }
        let mark = self.trail.len();
        let bad = self.assume(s);
        if !bad {
            for t in mark + 1..self.trail.len() {
                let m = self.trail[t];
                out.push(m);
                let v = (m >> 1) as usize;
                if self.longr[v] && self.parent[v] == s {
                    roots.push(m);
                }
            }
        }
        self.undo(mark);
        !bad
    }
}

struct Formula {
    nv: usize,
    n_clauses: usize,
    clauses: Vec<Vec<Lit>>,
}

fn read_cnf(path: &str, max_clauses: usize) -> Result<Formula, String> {
    // the holdout is told by where the file really is, so a link into it counts
    let real = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
    if real.components().any(|c| c.as_os_str() == "sat-comp-2026") {
        return Err("the 2026 suite is held out and is never read".into());
    }
    let reader: Box<dyn Read> = if path.ends_with(".xz") {
        let child = Command::new("xz")
            .arg("-dc")
            .arg(path)
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("xz: {e}"))?;
        Box::new(child.stdout.unwrap())
    } else {
        Box::new(File::open(path).map_err(|e| format!("{path}: {e}"))?)
    };
    let mut rd = BufReader::with_capacity(1 << 20, reader);
    let mut nv = 0usize;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    let mut cur: Vec<Lit> = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = rd.read_until(b'\n', &mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let first = line.iter().copied().find(|c| !c.is_ascii_whitespace());
        match first {
            None | Some(b'c') | Some(b'%') => continue,
            Some(b'p') => {
                let text = String::from_utf8_lossy(&line);
                let mut it = text.split_whitespace().skip(2);
                nv = it.next().and_then(|t| t.parse().ok()).ok_or("bad p line")?;
                continue;
            }
            _ => {}
        }
        let mut i = 0;
        while i < line.len() {
            while i < line.len() && line[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= line.len() {
                break;
            }
            let negative = line[i] == b'-';
            if negative {
                i += 1;
            }
            let mut x: i64 = 0;
            let from = i;
            while i < line.len() && line[i].is_ascii_digit() {
                x = x * 10 + (line[i] - b'0') as i64;
                i += 1;
            }
            if i == from {
                return Err("not a DIMACS file".into());
            }
            if x == 0 {
                if clauses.len() >= max_clauses {
                    return Err("too-large".into());
                }
                clauses.push(std::mem::take(&mut cur));
            } else {
                nv = nv.max(x as usize);
                cur.push(lit_of(if negative { -x } else { x }));
            }
        }
    }
    let n_clauses = clauses.len();
    Ok(Formula { nv, n_clauses, clauses })
}

/// The formula after its own units: None on a root conflict.  Clauses that
/// the root satisfies are gone and root-false literals are cut, so a clause
/// here is as long as it is for propagation.
fn load(f: &Formula) -> Option<(Prop, usize)> {
    let mut p = Prop::empty(f.nv);
    let mut keep: Vec<Vec<Lit>> = Vec::with_capacity(f.clauses.len());
    for c in &f.clauses {
        let mut c = c.clone();
        c.sort_unstable();
        c.dedup();
        if c.windows(2).any(|w| w[0] ^ 1 == w[1]) {
            continue;
        }
        // as the root stands now: a clause it satisfies is gone for good, a
        // literal it has made false is cut, so no watch starts on a false literal
        if c.iter().any(|&l| p.val[l as usize] == 1) {
            continue;
        }
        c.retain(|&l| p.val[l as usize] == 0);
        match c.len() {
            0 => return None,
            1 => {
                if p.assume(c[0]) {
                    return None;
                }
            }
            2 => {
                p.add_bin(c[0], c[1]);
                keep.push(c);
            }
            _ => {
                p.add_long(&c);
                keep.push(c);
            }
        }
    }
    if p.propagate() {
        return None;
    }
    // second build, on what is left at the root
    let root: Vec<i8> = p.val.clone();
    let units = p.trail.len();
    let mut q = Prop::empty(f.nv);
    let mut long: Vec<Vec<Lit>> = Vec::new();
    for c in keep {
        if c.iter().any(|&l| root[l as usize] == 1) {
            continue;
        }
        let c: Vec<Lit> = c.into_iter().filter(|&l| root[l as usize] == 0).collect();
        match c.len() {
            0 | 1 => unreachable!("root propagation was complete"),
            2 => q.add_bin(c[0], c[1]),
            _ => long.push(c),
        }
    }
    q.occ = vec![Vec::new(); 2 * f.nv + 2];
    for c in &long {
        let id = q.n_long() as u32;
        for &l in c {
            q.occ[l as usize].push(id);
        }
        q.add_long(c);
    }
    // the root's values stay set, off the trail
    for l in 0..root.len() {
        q.val[l] = root[l];
    }
    Some((q, units))
}

fn active_literals(p: &Prop) -> Vec<Lit> {
    let mut used = vec![false; p.nv + 1];
    for l in 0..p.bins.len() {
        if !p.bins[l].is_empty() {
            used[l >> 1] = true;
        }
    }
    for &l in &p.lits {
        used[(l >> 1) as usize] = true;
    }
    let mut out = Vec::new();
    for v in 1..=p.nv {
        if used[v] && p.val[2 * v] == 0 {
            out.push(2 * v as Lit);
            out.push(2 * v as Lit + 1);
        }
    }
    out
}

#[derive(Default)]
struct Opts {
    all: bool,
    seed: u64,
    sources: usize,
    targets: usize,
    probe_sources: usize,
    probe_pairs: usize,
    joint: usize,
    work: u64,
    max_add: usize,
    max_clauses: usize,
    kinds: Option<String>,
    examples: usize,
    eq_vars: usize,
}

fn parse_opts(args: &[String]) -> Result<(Vec<String>, Opts), String> {
    let mut o = Opts {
        seed: 1,
        sources: 200_000,
        targets: 8,
        probe_sources: 2_000,
        probe_pairs: 8,
        joint: 4,
        work: 20_000_000_000,
        max_add: 5_000_000,
        max_clauses: 60_000_000,
        eq_vars: 400_000,
        ..Default::default()
    };
    let mut pos = Vec::new();
    let mut i = 0;
    fn num(args: &[String], i: &mut usize, name: &str) -> Result<u64, String> {
        *i += 1;
        args.get(*i).and_then(|t| t.parse().ok()).ok_or(format!("{name} needs a number"))
    }
    while i < args.len() {
        let a = args[i].clone();
        match a.as_str() {
            "--all" => o.all = true,
            "--seed" => o.seed = num(args, &mut i, &a)?,
            "--sources" => o.sources = num(args, &mut i, &a)? as usize,
            "--targets" => o.targets = num(args, &mut i, &a)? as usize,
            "--probe-sources" => o.probe_sources = num(args, &mut i, &a)? as usize,
            "--probe-pairs" => o.probe_pairs = num(args, &mut i, &a)? as usize,
            "--joint" => o.joint = num(args, &mut i, &a)? as usize,
            "--work" => o.work = num(args, &mut i, &a)?,
            "--max-add" => o.max_add = num(args, &mut i, &a)? as usize,
            "--max-clauses" => o.max_clauses = num(args, &mut i, &a)? as usize,
            "--examples" => o.examples = num(args, &mut i, &a)? as usize,
            "--eq-vars" => o.eq_vars = num(args, &mut i, &a)? as usize,
            "--kinds" => {
                i += 1;
                o.kinds = Some(args.get(i).ok_or("--kinds needs a file")?.clone());
            }
            _ if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ => pos.push(a.clone()),
        }
        i += 1;
    }
    Ok((pos, o))
}

/// Kinds of variables, for the tables by kind: lines "variable kind".
fn read_kinds(path: &str, nv: usize) -> Result<(Vec<u16>, Vec<String>), String> {
    let mut names: Vec<String> = vec!["other".into()];
    let mut kind = vec![0u16; nv + 1];
    let fh = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    for line in BufReader::new(fh).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let mut it = line.split_whitespace();
        let (Some(v), Some(k)) = (it.next(), it.next()) else { continue };
        let v: usize = v.parse().map_err(|_| "bad kinds line")?;
        let id = match names.iter().position(|n| n == k) {
            Some(id) => id,
            None => {
                names.push(k.to_string());
                names.len() - 1
            }
        };
        if v <= nv {
            kind[v] = id as u16;
        }
    }
    Ok((kind, names))
}

#[derive(Default)]
struct Counts {
    sources: u64,
    failed: u64,
    seen: f64,
    seen_max: usize,
    checked: u64,
    two_way_inst: f64,
    one_way: f64,
    hidden_unit: f64,
    roots: u64,
    roots_two_way: u64,
    roots_unit: u64,
    // equal variables
    eq_vars: u64,
    eq_two_way_inst: u64,
    eq_hidden: u64,
    eq_units: u64,
    eq_examples: Vec<(Lit, Lit)>,
    // the probe part
    probe_sources: u64,
    pairs: u64,
    pair_conflict_seen: u64,
    pair_hidden: u64,
    pairs_with_joint: u64,
    joint: f64,
    joint_checked: u64,
    improper: f64,
    t_inst: [f64; 4],
    t_none: u64,
    triples: u64,
    examples: Vec<(Lit, Lit)>,
    root_examples: Vec<(Lit, Lit)>,
}

/// The two-literal part: for a source `s`, every literal `m` that
/// propagation sets is one side of the clause "not s or m".  The other side
/// is seen when setting m false sets s false.
fn binary_sides(p: &mut Prop, order: &[Lit], o: &Opts, rng: &mut Rng, budget: u64, c: &mut Counts,
                kinds: Option<&(Vec<u16>, Vec<String>)>, by_kind: &mut Vec<f64>, roots_by_kind: &mut Vec<f64>) {
    let mut imp: Vec<Lit> = Vec::new();
    let mut roots: Vec<Lit> = Vec::new();
    let nk = kinds.map_or(0, |k| k.1.len());
    for &s in order {
        if !o.all && (c.sources as usize >= o.sources || p.work >= budget) {
            break;
        }
        c.sources += 1;
        if !p.implied_roots(s, &mut imp, &mut roots) {
            c.failed += 1;
            continue;
        }
        // the root clauses of this source: every one is looked at
        for &m in &roots {
            let mark = p.trail.len();
            let bad = p.assume(m ^ 1);
            let seen = !bad && p.val[(s ^ 1) as usize] == 1;
            p.undo(mark);
            if bad {
                c.roots_unit += 1;
            } else if seen {
                c.roots_two_way += 1;
            } else {
                c.roots += 1;
                if c.root_examples.len() < o.examples {
                    c.root_examples.push((s, m));
                }
                if let Some((kind, _)) = kinds {
                    roots_by_kind[kind[(s >> 1) as usize] as usize * nk + kind[(m >> 1) as usize] as usize] += 1.0;
                }
            }
        }
        c.seen += imp.len() as f64;
        c.seen_max = c.seen_max.max(imp.len());
        if imp.is_empty() {
            continue;
        }
        let k = if o.all { imp.len() } else { o.targets.min(imp.len()) };
        if k == 0 {
            continue;
        }
        if k < imp.len() {
            rng.sample_front(&mut imp, k);
        }
        let weight = imp.len() as f64 / k as f64;
        for &m in &imp[..k] {
            c.checked += 1;
            let mark = p.trail.len();
            let bad = p.assume(m ^ 1);
            let seen = !bad && p.val[(s ^ 1) as usize] == 1;
            p.undo(mark);
            if bad {
                c.hidden_unit += weight;
            } else if seen {
                c.two_way_inst += weight;
            } else {
                c.one_way += weight;
                if c.examples.len() < o.examples {
                    c.examples.push((s, m));
                }
                if let Some((kind, _)) = kinds {
                    by_kind[kind[(s >> 1) as usize] as usize * nk + kind[(m >> 1) as usize] as usize] += weight;
                }
            }
        }
    }
}

/// One variable set both ways.  A literal h that follows v when v is on, and
/// whose opposite follows when v is off, is equal to v in every model.  In
/// `out`: such h, with whether h's own side sees it too (h on sets v, h off
/// sets v off).  In `units`: the literals that follow both ways, which are
/// true in every model.  False when v or its opposite fails.
fn equal_to(p: &mut Prop, v: Lit, on: &mut Vec<Lit>, off: &mut Vec<Lit>, stamp: &mut [u32], epoch: u32,
            out: &mut Vec<(Lit, bool, bool)>, units: &mut u64, both: &mut Vec<Lit>) -> bool {
    out.clear();
    both.clear();
    if !p.implied(v, on) || !p.implied(v ^ 1, off) {
        return false;
    }
    for &l in off.iter() {
        stamp[l as usize] = epoch;
    }
    let mut eq: Vec<Lit> = Vec::new();
    for &h in on.iter() {
        if stamp[h as usize] == epoch {
            *units += 1;
            both.push(h);
        } else if stamp[(h ^ 1) as usize] == epoch {
            eq.push(h);
        }
    }
    for h in eq {
        // h's own side: h sets v, and h off sets v off
        let mark = p.trail.len();
        let bad = p.assume(h);
        let up = !bad && p.val[v as usize] == 1;
        p.undo(mark);
        let bad2 = p.assume(h ^ 1);
        let down = !bad2 && p.val[(v ^ 1) as usize] == 1;
        p.undo(mark);
        if bad || bad2 {
            continue; // h or its opposite fails: a unit one probe away, no equality to speak of
        }
        out.push((h, up, down));
    }
    true
}

/// The equal variables of a sample of variables.
fn equal_vars(p: &mut Prop, order: &[Lit], o: &Opts, budget: u64, c: &mut Counts) {
    let n = 2 * p.nv + 2;
    let mut stamp = vec![0u32; n];
    let mut done = vec![false; p.nv + 1];
    let (mut on, mut off, mut out, mut both) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut epoch = 0u32;
    for &l in order {
        let v = l & !1;
        if done[(v >> 1) as usize] {
            continue;
        }
        done[(v >> 1) as usize] = true;
        if !o.all && (c.eq_vars as usize >= o.eq_vars || p.work >= budget) {
            break;
        }
        c.eq_vars += 1;
        epoch += 1;
        if !equal_to(p, v, &mut on, &mut off, &mut stamp, epoch, &mut out, &mut c.eq_units, &mut both) {
            continue;
        }
        for &(h, up, down) in &out {
            if up && down {
                c.eq_two_way_inst += 1;
            } else {
                c.eq_hidden += 1;
                if c.eq_examples.len() < o.examples {
                    c.eq_examples.push((v, h));
                }
            }
        }
    }
}

/// The second literals to try with a first one `a1` whose propagation is on
/// the trail from `mark`: the literals that cut a clause which `a1` has cut
/// already.  With --all, every open literal after `a1`.
fn second_literals(p: &mut Prop, a1: Lit, mark: usize, active: &[Lit], o: &Opts, rng: &mut Rng,
                   stamp: &mut [u32], epoch: u32, out: &mut Vec<Lit>) {
    out.clear();
    if o.all {
        for &a2 in active {
            if a2 > a1 && (a2 >> 1) != (a1 >> 1) && p.val[a2 as usize] == 0 {
                out.push(a2);
            }
        }
        return;
    }
    let mut visits = 0usize;
    'scan: for t in mark..p.trail.len() {
        let f = p.trail[t] ^ 1;
        for i in 0..p.occ[f as usize].len() {
            let cl = p.occ[f as usize][i] as usize;
            visits += 1;
            if visits > 4096 {
                break 'scan;
            }
            let (a, b) = (p.start[cl] as usize, p.start[cl + 1] as usize);
            if p.lits[a..b].iter().any(|&x| p.val[x as usize] == 1) {
                continue;
            }
            for k in a..b {
                let x = p.lits[k];
                if p.val[x as usize] == 0 && stamp[(x ^ 1) as usize] != epoch {
                    stamp[(x ^ 1) as usize] = epoch;
                    out.push(x ^ 1);
                }
            }
        }
    }
    out.sort_unstable();
    let k = o.probe_pairs.min(out.len());
    rng.sample_front(out, k);
    out.truncate(k);
}

/// The probe part: pairs of literals set together.
fn probe(p: &mut Prop, order: &[Lit], active: &[Lit], o: &Opts, rng: &mut Rng, budget: u64, c: &mut Counts) {
    let n = 2 * p.nv + 2;
    let mut stamp = vec![0u32; n];
    let mut in1 = vec![0u32; n];
    let mut in2 = vec![0u32; n];
    let mut epoch = 0u32;
    let (mut i1, mut i2, mut seconds, mut joint) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for &a1 in order {
        if !o.all && (c.probe_sources as usize >= o.probe_sources || p.work >= budget) {
            break;
        }
        if !p.implied(a1, &mut i1) {
            continue;
        }
        c.probe_sources += 1;
        epoch += 1;
        let e1 = epoch;
        for &l in &i1 {
            in1[l as usize] = e1;
        }
        let mark = p.trail.len();
        p.assume(a1);
        second_literals(p, a1, mark, active, o, rng, &mut stamp, e1, &mut seconds);
        p.undo(mark);
        for si in 0..seconds.len() {
            let a2 = seconds[si];
            if !p.implied(a2, &mut i2) {
                continue;
            }
            epoch += 1;
            let e2 = epoch;
            for &l in &i2 {
                in2[l as usize] = e2;
            }
            c.pairs += 1;
            p.assume(a1);
            let mark1 = p.trail.len();
            let bad = p.assume(a2);
            joint.clear();
            if !bad {
                for t in mark1 + 1..p.trail.len() {
                    let m = p.trail[t];
                    if in2[m as usize] != e2 {
                        joint.push(m);
                    }
                }
            }
            p.undo(mark);
            if bad {
                // "not a1 or not a2": a1's side does not see it (a2 was open)
                if in2[(a1 ^ 1) as usize] == e2 {
                    c.pair_conflict_seen += 1;
                } else {
                    c.pair_hidden += 1;
                }
                continue;
            }
            if joint.is_empty() {
                continue;
            }
            c.pairs_with_joint += 1;
            c.joint += joint.len() as f64;
            let k = if o.all { joint.len() } else { o.joint.min(joint.len()) };
            if k == 0 {
                continue;
            }
            if k < joint.len() {
                rng.sample_front(&mut joint, k);
            }
            let weight = joint.len() as f64 / k as f64;
            for &m in &joint[..k] {
                // the clause "not a1 or not a2 or m", seen from m's side
                c.joint_checked += 1;
                let mut sides = 1usize;
                let mut proper = true;
                for (x, y) in [(a1, a2), (a2, a1)] {
                    p.assume(x);
                    let bad = p.assume(m ^ 1);
                    if bad {
                        proper = false;
                    } else if p.val[(y ^ 1) as usize] == 1 {
                        sides += 1;
                    }
                    p.undo(mark);
                    if !proper {
                        break;
                    }
                }
                if proper {
                    c.t_inst[sides] += weight;
                } else {
                    c.improper += weight;
                }
            }
        }
    }
}

/// With --all: the three-literal clauses that no side sees and that fail
/// when all three literals are false, with no two-literal part that fails.
fn hidden_triples(p: &mut Prop, active: &[Lit], c: &mut Counts) {
    let n = active.len();
    let mut imp = Vec::new();
    let mut ok = vec![false; 2 * p.nv + 2];
    for &l in active {
        ok[l as usize] = p.implied(l, &mut imp);
    }
    // pairs that fail together or where one sets the other are no part of a proper triple
    for i in 0..n {
        let a = active[i];
        if !ok[a as usize] {
            continue;
        }
        for j in i + 1..n {
            let b = active[j];
            if !ok[b as usize] || (a >> 1) == (b >> 1) {
                continue;
            }
            let mark = p.trail.len();
            p.assume(a);
            if p.val[b as usize] != 0 || p.assume(b) {
                p.undo(mark);
                continue;
            }
            let mark2 = p.trail.len();
            for k in j + 1..n {
                let d = active[k];
                if !ok[d as usize] || (d >> 1) == (a >> 1) || (d >> 1) == (b >> 1) {
                    continue;
                }
                c.triples += 1;
                if p.val[d as usize] != 0 {
                    continue; // set by a and b: a seen side, or a two-literal part
                }
                let bad = p.assume(d);
                p.undo(mark2);
                if !bad {
                    continue;
                }
                // a, b, d fail together and a, b do not set d's opposite.  The other sides:
                let mut none = true;
                for (x, y, z) in [(a, d, b), (b, d, a)] {
                    p.undo(mark);
                    p.assume(x);
                    let bad = p.val[y as usize] != 0 || p.assume(y);
                    if bad || p.val[(z ^ 1) as usize] == 1 {
                        none = false;
                    }
                    if !none {
                        break;
                    }
                }
                p.undo(mark);
                p.assume(a);
                p.assume(b);
                if none {
                    c.t_none += 1;
                }
            }
            p.undo(mark);
        }
    }
}

fn measure(path: &str, o: &Opts) -> Result<String, String> {
    let t0 = std::time::Instant::now();
    let f = match read_cnf(path, o.max_clauses) {
        Ok(f) => f,
        Err(e) if e == "too-large" => return Ok("c oneway status=too-large".into()),
        Err(e) => return Err(e),
    };
    let Some((mut p, units)) = load(&f) else {
        return Ok(format!("c oneway status=root-conflict vars={} clauses={}", f.nv, f.n_clauses));
    };
    let active = active_literals(&p);
    let kinds = match &o.kinds {
        Some(path) => Some(read_kinds(path, f.nv)?),
        None => None,
    };
    let nk = kinds.as_ref().map_or(0, |k| k.1.len());
    let mut by_kind = vec![0f64; nk * nk];
    let mut roots_by_kind = vec![0f64; nk * nk];
    let mut rng = Rng::new(o.seed);
    let mut order = active.clone();
    if !o.all {
        rng.shuffle(&mut order);
    }
    let mut c = Counts::default();
    binary_sides(&mut p, &order, o, &mut rng, o.work / 10 * 3, &mut c, kinds.as_ref(), &mut by_kind, &mut roots_by_kind);
    let work_a = p.work;
    equal_vars(&mut p, &order, o, o.work / 10 * 8, &mut c);
    probe(&mut p, &order, &active, o, &mut rng, o.work, &mut c);
    let all_triples = o.all && active.len() <= 400;
    if all_triples {
        hidden_triples(&mut p, &active, &mut c);
    }
    let status = if o.all {
        "all"
    } else if (c.sources as usize) < active.len() {
        "sampled"
    } else {
        "all-sources"
    };
    // from the sources looked at to every open literal
    let scale = if c.sources > 0 { active.len() as f64 / c.sources as f64 } else { 0.0 };
    let eq_scale = if c.eq_vars > 0 { (active.len() / 2) as f64 / c.eq_vars as f64 } else { 0.0 };
    let checked_w = c.two_way_inst + c.one_way + c.hidden_unit;
    let share = if checked_w > 0.0 { c.one_way / checked_w } else { 0.0 };
    let mut line = format!(
        "c oneway status={status} vars={} clauses={} units={units} active={} bin={} long={} sources={} failed={} \
         seen={:.0} seen_max={} checked={} two_way={:.1} one_way={:.1} hidden_unit={:.1} one_way_share={share:.6} \
         seen_all={:.0} two_way_all={:.0} one_way_all={:.0} roots={} roots_two_way={} roots_unit={} roots_all={:.0} \
         eq_vars={} eq_two_way={:.1} eq_hidden={} eq_units={} eq_hidden_all={:.0} \
         probe_sources={} pairs={} pair_one_way={} pair_hidden={} pairs_with_joint={} joint={:.0} joint_checked={} \
         improper={:.1} t1={:.2} t2={:.2} t3={:.2} t0={} triples={} work_a={work_a} work={} seconds={:.1}",
        f.nv,
        f.n_clauses,
        active.len() / 2,
        p.n_bin,
        p.n_long(),
        c.sources,
        c.failed,
        c.seen,
        c.seen_max,
        c.checked,
        c.two_way_inst / 2.0,
        c.one_way,
        c.hidden_unit,
        c.seen * scale,
        c.two_way_inst * scale / 2.0,
        c.one_way * scale,
        c.roots,
        c.roots_two_way,
        c.roots_unit,
        c.roots as f64 * scale,
        c.eq_vars,
        c.eq_two_way_inst as f64 / 2.0,
        c.eq_hidden,
        c.eq_units,
        c.eq_hidden as f64 * eq_scale,
        c.probe_sources,
        c.pairs,
        c.pair_conflict_seen,
        c.pair_hidden,
        c.pairs_with_joint,
        c.joint,
        c.joint_checked,
        c.improper,
        c.t_inst[1],
        c.t_inst[2] / 2.0,
        c.t_inst[3] / 3.0,
        if all_triples { c.t_none.to_string() } else { "NA".into() },
        c.triples,
        p.work,
        t0.elapsed().as_secs_f64(),
    );
    // some one-way clauses, as "c example s m": s sets m, and m false does not set s false
    let mut head = String::new();
    for &(s, m) in &c.examples {
        head.push_str(&format!("c example {} {}\n", dimacs(s), dimacs(m)));
    }
    for &(s, m) in &c.root_examples {
        head.push_str(&format!("c root {} {}\n", dimacs(s), dimacs(m)));
    }
    for &(v, h) in &c.eq_examples {
        head.push_str(&format!("c equal {} {}\n", dimacs(v), dimacs(h)));
    }
    line = head + &line;
    if let Some((_, names)) = &kinds {
        for a in 0..nk {
            for b in 0..nk {
                if by_kind[a * nk + b] > 0.0 {
                    line.push_str(&format!(" k:{}>{}={:.1}", names[a], names[b], by_kind[a * nk + b]));
                }
                if roots_by_kind[a * nk + b] > 0.0 {
                    line.push_str(&format!(" r:{}>{}={:.0}", names[a], names[b], roots_by_kind[a * nk + b]));
                }
            }
        }
    }
    Ok(line)
}

/// FILE plus the missing side of every one-way root clause, pass after pass
/// until a pass adds nothing.  A root clause "not s or m" is added when
/// setting m false does not set s false; it is added at once, so what
/// comes later can lean on it.  When no root clause is one-way, no
/// two-literal clause that propagation sees from one side is.
fn close(path: &str, out: &str, o: &Opts) -> Result<String, String> {
    let f = read_cnf(path, o.max_clauses)?;
    let Some((mut p, _units)) = load(&f) else {
        return Ok("c oneway-close status=root-conflict".into());
    };
    let active = active_literals(&p);
    let mut added: Vec<(Lit, Lit)> = Vec::new();
    let (mut imp, mut roots) = (Vec::new(), Vec::new());
    let (mut hidden_units, mut passes) = (0u64, 0u32);
    let mut failed: u64;
    let mut status = "closed";
    let mut by_pass: Vec<usize> = Vec::new();
    'all: loop {
        passes += 1;
        let before = added.len();
        failed = 0;
        for &s in &active {
            if p.work >= o.work {
                status = "stopped";
                break 'all;
            }
            if !p.implied_roots(s, &mut imp, &mut roots) {
                failed += 1;
                continue;
            }
            for &m in &roots {
                let mark = p.trail.len();
                let bad = p.assume(m ^ 1);
                let seen = !bad && p.val[(s ^ 1) as usize] == 1;
                p.undo(mark);
                if bad {
                    hidden_units += 1;
                } else if !seen {
                    if added.len() >= o.max_add {
                        status = "full";
                        break 'all;
                    }
                    p.add_bin(s ^ 1, m);
                    added.push((s ^ 1, m));
                }
            }
        }
        by_pass.push(added.len() - before);
        if added.len() == before {
            break;
        }
        if passes >= 20 {
            status = "passes";
            break;
        }
    }
    let fh = File::create(out).map_err(|e| format!("{out}: {e}"))?;
    let mut w = BufWriter::new(fh);
    let io = |e: std::io::Error| e.to_string();
    writeln!(w, "p cnf {} {}", f.nv, f.n_clauses + added.len()).map_err(io)?;
    for c in &f.clauses {
        for &l in c {
            write!(w, "{} ", dimacs(l)).map_err(io)?;
        }
        writeln!(w, "0").map_err(io)?;
    }
    for &(a, b) in &added {
        writeln!(w, "{} {} 0", dimacs(a), dimacs(b)).map_err(io)?;
    }
    w.flush().map_err(io)?;
    let by_pass: Vec<String> = by_pass.iter().map(|n| n.to_string()).collect();
    Ok(format!(
        "c oneway-close status={status} vars={} clauses={} active={} failed={failed} hidden_unit={hidden_units} added={} passes={passes} by_pass={} work={}",
        f.nv,
        f.n_clauses,
        active.len() / 2,
        added.len(),
        by_pass.join(","),
        p.work
    ))
}

/// FILE plus the missing halves of the hidden equal variables: for a
/// variable v and a literal h that follows v both ways, the clauses
/// "h -> v" and "not h -> not v" where propagation does not have them.
fn equal(path: &str, out: &str, o: &Opts) -> Result<String, String> {
    let f = read_cnf(path, o.max_clauses)?;
    let Some((mut p, _units)) = load(&f) else {
        return Ok("c oneway-equal status=root-conflict".into());
    };
    let active = active_literals(&p);
    let n = 2 * p.nv + 2;
    let mut stamp = vec![0u32; n];
    let (mut on, mut off, mut found, mut both) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    // an added clause: two literals, or one (the second is then 0)
    let mut added: Vec<(Lit, Lit)> = Vec::new();
    let mut unit_count = 0u64;
    let (mut epoch, mut passes, mut pairs) = (0u32, 0u32, 0u64);
    let mut units: u64;
    let mut status = "closed";
    let mut by_pass: Vec<usize> = Vec::new();
    'all: loop {
        passes += 1;
        let before = added.len();
        units = 0;
        for &l in &active {
            if l & 1 == 1 {
                continue;
            }
            if p.work >= o.work {
                status = "stopped";
                break 'all;
            }
            epoch += 1;
            if p.val[l as usize] != 0 {
                continue; // set by a unit found on the way
            }
            if !equal_to(&mut p, l, &mut on, &mut off, &mut stamp, epoch, &mut found, &mut units, &mut both) {
                continue;
            }
            // a literal u that follows l on and l off is true in every model.  "not l or u"
            // and "l or u" each follow by propagation, and u from the two.
            let mut fixed = false;
            for &u in &both {
                if p.val[u as usize] != 0 {
                    continue;
                }
                if added.len() + 3 > o.max_add {
                    status = "full";
                    break 'all;
                }
                p.add_bin(l ^ 1, u);
                p.add_bin(l, u);
                added.push((l ^ 1, u));
                added.push((l, u));
                added.push((u, 0));
                unit_count += 1;
                fixed = true;
                if p.assume(u) {
                    status = "refuted";
                    break 'all;
                }
            }
            if fixed {
                continue; // the root has changed: this variable is looked at again in the next pass
            }
            for &(h, up, down) in &found {
                if up && down {
                    continue;
                }
                if added.len() + 2 > o.max_add {
                    status = "full";
                    break 'all;
                }
                pairs += 1;
                if !up {
                    p.add_bin(h ^ 1, l);
                    added.push((h ^ 1, l));
                }
                if !down {
                    p.add_bin(h, l ^ 1);
                    added.push((h, l ^ 1));
                }
            }
        }
        by_pass.push(added.len() - before);
        if added.len() == before {
            break;
        }
        if passes >= 20 {
            status = "passes";
            break;
        }
    }
    let fh = File::create(out).map_err(|e| format!("{out}: {e}"))?;
    let mut w = BufWriter::new(fh);
    let io = |e: std::io::Error| e.to_string();
    writeln!(w, "p cnf {} {}", f.nv, f.n_clauses + added.len()).map_err(io)?;
    for c in &f.clauses {
        for &l in c {
            write!(w, "{} ", dimacs(l)).map_err(io)?;
        }
        writeln!(w, "0").map_err(io)?;
    }
    for &(a, b) in &added {
        if b == 0 {
            writeln!(w, "{} 0", dimacs(a)).map_err(io)?;
        } else {
            writeln!(w, "{} {} 0", dimacs(a), dimacs(b)).map_err(io)?;
        }
    }
    w.flush().map_err(io)?;
    let by_pass: Vec<String> = by_pass.iter().map(|n| n.to_string()).collect();
    let _ = units;
    Ok(format!(
        "c oneway-equal status={status} vars={} clauses={} active={} pairs={pairs} units={unit_count} added={} passes={passes} by_pass={} work={}",
        f.nv,
        f.n_clauses,
        active.len() / 2,
        added.len(),
        by_pass.join(","),
        p.work
    ))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run = || -> Result<String, String> {
        let (pos, o) = parse_opts(&args)?;
        match pos.first().map(|s| s.as_str()) {
            Some("measure") if pos.len() == 2 => measure(&pos[1], &o),
            Some("close") if pos.len() == 3 => close(&pos[1], &pos[2], &o),
            Some("equal") if pos.len() == 3 => equal(&pos[1], &pos[2], &o),
            _ => Err("usage: one-way measure FILE [options] | one-way close FILE OUT [options] | one-way equal FILE OUT [options]".into()),
        }
    };
    match run() {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("one-way: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formula(nv: usize, cls: &[&[i64]]) -> Formula {
        Formula { nv, n_clauses: cls.len(), clauses: cls.iter().map(|c| c.iter().map(|&x| lit_of(x)).collect()).collect() }
    }

    fn counts(f: &Formula) -> Counts {
        let (mut p, _) = load(f).unwrap();
        let active = active_literals(&p);
        let o = Opts { all: true, ..Default::default() };
        let mut rng = Rng::new(1);
        let mut c = Counts::default();
        let mut by = Vec::new();
        let mut by2 = Vec::new();
        binary_sides(&mut p, &active, &o, &mut rng, u64::MAX, &mut c, None, &mut by, &mut by2);
        probe(&mut p, &active, &active, &o, &mut rng, u64::MAX, &mut c);
        hidden_triples(&mut p, &active, &mut c);
        c
    }

    #[test]
    fn a_long_clause_gives_a_one_way_pair() {
        // s -> a, s -> b, (not a or not b or m): "not s or m" is seen from s only
        let f = formula(4, &[&[-1, 2], &[-1, 3], &[-2, -3, 4]]);
        let c = counts(&f);
        assert_eq!(c.one_way, 1.0);
        // the two clauses s -> a and s -> b are two-way
        assert_eq!(c.two_way_inst / 2.0, 2.0);
    }

    #[test]
    fn a_plain_clause_of_three_is_seen_from_every_side() {
        let f = formula(3, &[&[1, 2, 3]]);
        let c = counts(&f);
        assert_eq!(c.one_way, 0.0);
        assert_eq!(c.t_inst[3] / 3.0, 1.0);
        assert_eq!(c.t_inst[1] + c.t_inst[2], 0.0);
        assert_eq!(c.t_none, 0);
    }

    #[test]
    fn a_pair_that_only_a_probe_shows_is_hidden() {
        // (a or b or c), (a or b or not c): "a or b" is implied, and no side sees it
        let f = formula(3, &[&[1, 2, 3], &[1, 2, -3]]);
        let c = counts(&f);
        assert_eq!(c.pair_hidden, 1);
        assert_eq!(c.one_way, 0.0);
    }

    #[test]
    fn closing_adds_the_missing_side_once() {
        let f = formula(4, &[&[-1, 2], &[-1, 3], &[-2, -3, 4]]);
        let (mut p, _) = load(&f).unwrap();
        let mut imp = Vec::new();
        assert!(p.implied(lit_of(-4), &mut imp));
        assert!(!imp.contains(&lit_of(-1)));
        p.add_bin(lit_of(-1), lit_of(4));
        assert!(p.implied(lit_of(-4), &mut imp));
        assert!(imp.contains(&lit_of(-1)));
    }

    #[test]
    fn the_root_units_are_taken_out() {
        // unit a; (not a or b or c) becomes (b or c), a two-literal clause
        let f = formula(3, &[&[1], &[-1, 2, 3]]);
        let (p, units) = load(&f).unwrap();
        assert_eq!(units, 1);
        assert_eq!(p.n_bin, 1);
        assert_eq!(p.n_long(), 0);
        assert_eq!(active_literals(&p).len(), 4);
    }

    #[test]
    fn a_helper_that_follows_a_variable_both_ways_is_equal_to_it() {
        // g <-> h by two two-literal clauses: both sides see it, and it is found from both
        let f = formula(2, &[&[-1, 2], &[1, -2]]);
        let (mut p, _) = load(&f).unwrap();
        let active = active_literals(&p);
        let o = Opts { all: true, ..Default::default() };
        let mut c = Counts::default();
        equal_vars(&mut p, &active, &o, u64::MAX, &mut c);
        assert_eq!((c.eq_hidden, c.eq_two_way_inst), (0, 2));
        // g2 -> g1 and h <-> (g1 and g2) as (not g1 or not g2 or h), (g2 or not h): g2 on sets
        // g1 and h, g2 off sets h off, so h equals g2.  h on sets g2; h off leaves "not g1 or
        // not g2" and sets nothing: hidden from h's side
        let f = formula(3, &[&[-2, 1], &[-1, -2, 3], &[2, -3]]);
        let (mut p, _) = load(&f).unwrap();
        let active = active_literals(&p);
        let mut c = Counts::default();
        equal_vars(&mut p, &active, &o, u64::MAX, &mut c);
        assert_eq!((c.eq_hidden, c.eq_two_way_inst), (1, 0));
        // h <-> "at least two of g1, g2, g3" over sorted bits g3 -> g2 -> g1: h equals g2, and
        // h on sets no bit
        let f = formula(4, &[&[-2, 1], &[-3, 2], &[-1, -2, 4], &[-1, -3, 4], &[-2, -3, 4], &[1, 2, -4], &[1, 3, -4], &[2, 3, -4]]);
        let (mut p, _) = load(&f).unwrap();
        let active = active_literals(&p);
        let mut c = Counts::default();
        equal_vars(&mut p, &active, &o, u64::MAX, &mut c);
        assert_eq!(c.eq_hidden, 1);
    }
}
