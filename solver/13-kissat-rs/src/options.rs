// Port of src/options.h + src/options.c (kissat 4.0.4).
//
// Reference build flags (per CONVENTIONS.md): NDEBUG defined; LOGGING, QUIET,
// EMBEDDED, NOPTIONS, SAT, UNSAT all undefined. Consequently:
//   - DBGOPT `check`, LOGOPT `log`, EMBOPT `embedded` are compiled OUT and do
//     not appear in the table or the struct (158 options remain).
//   - NQTOPT `profile`, `quiet`, `statistics`, `verbose` are IN.
//   - TARGET_DEFAULT = 1, STABLE_DEFAULT = 1, RESTARTINT_DEFAULT = 1.
//   - Options are runtime-mutable i32 fields (the !NOPTIONS variant); the
//     NOPTIONS compile-time-constant variant is not ported.
//
// PORT NOTE: `kissat_options_print_value` is declared in options.h but never
// defined anywhere in the kissat 4.0.4 sources (dead declaration) — not ported.
// PORT NOTE: `check_table_sorted`, `check_ranges`, `check_name_length` are
// NDEBUG-off/`kissat_fatal` sanity checks over a compile-time table; they are
// covered here by unit tests instead of runtime checks.
// PORT NOTE: `format_value`/`format_count` below are a local transliteration of
// `kissat_format_value`/`format_count` from format.c (only the pieces
// options.c needs), producing byte-identical text; format.c's ring buffer is C
// memory management with no output effect and is not reproduced here.

// From options.h (also used by config.rs):
pub const TARGET_SAT: i32 = 2;
pub const TARGET_DEFAULT: i32 = 1;

pub const STABLE_DEFAULT: i32 = 1;
pub const STABLE_UNSAT: i32 = 0;

pub const RESTARTINT_DEFAULT: i32 = 1;
pub const RESTARTINT_SAT: i32 = 50;

/// C: `kissat_options_max_name_buffer_size`.
pub const OPTIONS_MAX_NAME_BUFFER_SIZE: usize = 32;

/// C: `struct opt` (the !NOPTIONS variant). `value` is the default value; the
/// runtime-mutable values live in `Options`.
pub struct OptionEntry {
    pub name: &'static str,
    pub value: i32,
    pub low: i32,
    pub high: i32,
    pub description: &'static str,
}

macro_rules! kissat_options {
    ($(($name:ident, $value:expr, $low:expr, $high:expr, $desc:expr),)*) => {
        /// C: `struct options` — one runtime-mutable `int` per option, in
        /// exact OPTIONS-table order.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Options {
            $(pub $name: i32,)*
        }

        impl Default for Options {
            /// C: `kissat_init_options` default assignment.
            fn default() -> Self {
                Options { $($name: $value,)* }
            }
        }

        /// C: `static const opt table[]` — sorted by `strcmp` on name
        /// (asserted by `check_table_sorted` in the C debug build).
        pub static OPTION_TABLE: &[OptionEntry] = &[
            $(OptionEntry {
                name: stringify!($name),
                value: $value,
                low: $low,
                high: $high,
                description: $desc,
            },)*
        ];

        impl Options {
            /// Field access by option name. C does this with pointer
            /// arithmetic (`(int *) options + (o - table)`); the name is the
            /// same unique key.
            fn value_by_name(&self, name: &str) -> Option<i32> {
                $(if name == stringify!($name) { return Some(self.$name); })*
                None
            }

            fn value_mut_by_name(&mut self, name: &str) -> Option<&mut i32> {
                $(if name == stringify!($name) { return Some(&mut self.$name); })*
                None
            }
        }
    };
}

kissat_options! {
    (ands, 1, 0, 1, "extract and eliminate and gates"),
    (backbone, 1, 0, 2, "binary clause backbone (2=eager)"),
    (backboneeffort, 20, 0, 100_000, "effort in per mille"),
    (backbonemaxrounds, 1000, 1, i32::MAX, "maximum backbone rounds"),
    (backbonerounds, 100, 1, i32::MAX, "backbone rounds limit"),
    (bigbigfraction, 990, 0, 1000, "big binary clause fraction per mille"),
    (bump, 1, 0, 1, "enable variable bumping"),
    (bumpreasons, 1, 0, 1, "bump reason side literals too"),
    (bumpreasonslimit, 10, 1, i32::MAX, "relative reason literals limit"),
    (bumpreasonsrate, 10, 1, i32::MAX, "decision rate limit"),
    (chrono, 1, 0, 1, "allow chronological backtracking"),
    (chronolevels, 100, 0, i32::MAX, "maximum jumped over levels"),
    (compact, 1, 0, 1, "enable compacting garbage collection"),
    (compactlim, 10, 0, 100, "compact inactive limit (in percent)"),
    (congruence, 1, 0, 1, "congruence closure on extracted gates"),
    (congruenceandarity, 1_000_000, 2, 50_000_000, "AND gate arity limit"),
    (congruenceands, 1, 0, 1, "extract AND gates for congruence closure"),
    (congruencebinaries, 1, 0, 1, "extract certain binary clauses"),
    (congruenceites, 1, 0, 1, "extract ITE gates for congruence closure"),
    (congruenceonce, 0, 0, 1, "congruence closure only initially"),
    (congruencexorarity, 4, 2, 20, "congruence XOR gate arity limit"),
    (congruencexorcounts, 2, 1, i32::MAX, "XOR counting rounds"),
    (congruencexors, 1, 0, 1, "extract XOR gates for congruence closure"),
    (decay, 50, 1, 200, "per mille scores decay"),
    (definitioncores, 2, 1, 100, "how many cores"),
    (definitions, 1, 0, 1, "extract general definitions"),
    (definitionticks, 1_000_000, 0, i32::MAX, "kitten ticks limits"),
    (defraglim, 75, 50, 100, "usable defragmentation limit in percent"),
    (defragsize, 1 << 18, 10, i32::MAX, "size defragmentation limit"),
    (eagersubsume, 4, 0, 4, "eagerly subsume previous learned clauses"),
    (eliminate, 1, 0, 1, "bounded variable elimination BVE"),
    (eliminatebound, 16, 0, 1 << 13, "maximum elimination bound"),
    (eliminateclslim, 100, 1, i32::MAX, "elimination clause size limit"),
    (eliminateeffort, 100, 0, 2000, "effort in per mille"),
    (eliminateinit, 500, 0, i32::MAX, "initial elimination interval"),
    (eliminateint, 500, 10, i32::MAX, "base elimination interval"),
    (eliminateocclim, 2000, 0, i32::MAX, "elimination occurrence limit"),
    (eliminaterounds, 2, 1, 10_000, "elimination rounds limit"),
    (emafast, 33, 10, 1_000_000, "fast exponential moving average window"),
    (emaslow, 100_000, 100, 1_000_000, "slow exponential moving average window"),
    (equivalences, 1, 0, 1, "extract and eliminate equivalence gates"),
    (extract, 1, 0, 1, "extract gates in variable elimination"),
    (factor, 1, 0, 1, "bounded variable addition"),
    (factorcandrounds, 2, 0, i32::MAX, "candidates reduction rounds"),
    (factordelay, 4, 0, 12, "delaying factor"),
    (factoreffort, 50, 0, 1_000_000, "bounded variable effort in per mille"),
    (factorhops, 3, 1, 10, "structural factoring heuristic hops"),
    (factoriniticks, 700, 1, 1_000_000, "initial ticks ticks in millions"),
    (factorsize, 5, 2, i32::MAX, "bounded variable addition clause size"),
    (factorstructural, 0, 0, 1, "structural bounded variable addition"),
    (fastel, 0, 0, 1, "initial fast variable elimination"),
    (fastelclslim, 100, 1, i32::MAX, "fast elimination clause length limit"),
    (fastelim, 8, 1, 1000, "fast elimination resolvents limit"),
    (fasteloccs, 100, 1, 1000, "fast elimination occurrence limit"),
    (fastelrounds, 4, 1, 1000, "fast elimination rounds"),
    (fastelsub, 1, 0, 1, "forward subsuming fast variable elimination"),
    (flushproof, 0, 0, 1, "flush proof lines immediately"),
    (focusedtiers, 1, 0, 1, "always used focused mode tiers"),
    (forcephase, 0, 0, 1, "force initial phase"),
    (forward, 1, 0, 1, "forward subsumption in BVE"),
    (forwardeffort, 100, 0, 1_000_000, "effort in per mille"),
    (ifthenelse, 1, 0, 1, "extract and eliminate if-then-else gates"),
    (incremental, 0, 0, 1, "enable incremental solving"),
    (jumpreasons, 1, 0, 1, "jump binary reasons"),
    (lucky, 1, 0, 1, "try some lucky assignments"),
    (luckyearly, 1, 0, 1, "lucky assignments before preprocessing"),
    (luckylate, 1, 0, 1, "lucky assignments after preprocessing"),
    (mineffort, 10, 0, i32::MAX, "minimum absolute effort in millions"),
    (minimize, 1, 0, 1, "learned clause minimization"),
    (minimizedepth, 1000, 1, 1_000_000, "minimization depth"),
    (minimizeticks, 1, 0, 1, "count ticks in minimize and shrink"),
    (modeinit, 1000, 10, 100_000_000, "initial focused conflicts limit"),
    (modeint, 1000, 10, 100_000_000, "focused conflicts interval"),
    (otfs, 1, 0, 1, "on-the-fly strengthening"),
    (phase, 1, 0, 1, "initial decision phase"),
    (phasesaving, 1, 0, 1, "enable phase saving"),
    (preprocess, 1, 0, 1, "initial preprocessing"),
    (preprocessbackbone, 1, 0, 1, "backbone preprocessing"),
    (preprocesscongruence, 1, 0, 1, "congruence preprocessing"),
    (preprocessfactor, 1, 0, 1, "variable addition preprocessing"),
    (preprocessprobe, 1, 0, 1, "probing preprocessing"),
    (preprocessrounds, 1, 1, i32::MAX, "initial preprocessing rounds"),
    (preprocessweep, 1, 0, 1, "sweep preprocessing"),
    (probe, 1, 0, 1, "enable probing"),
    (probeinit, 100, 0, i32::MAX, "initial probing interval"),
    (probeint, 100, 2, i32::MAX, "probing interval"),
    (proberounds, 2, 1, i32::MAX, "probing rounds"),
    (profile, 2, 0, 4, "profile level"),
    (promote, 1, 0, 1, "promote clauses"),
    (quiet, 0, 0, 1, "disable all messages"),
    (randec, 1, 0, 1, "random decisions"),
    (randecfocused, 1, 0, 1, "random decisions in focused mode"),
    (randecinit, 500, 0, i32::MAX, "random decisions interval"),
    (randecint, 500, 0, i32::MAX, "initial random decisions interval"),
    (randeclength, 10, 1, i32::MAX, "random conflicts length"),
    (randecstable, 0, 0, 1, "random decisions in stable mode"),
    (reduce, 1, 0, 1, "learned clause reduction"),
    (reducehigh, 900, 0, 1000, "high reduce fraction per mille"),
    (reduceinit, 1000, 2, 100_000, "initial reduce interval"),
    (reduceint, 1000, 2, 100_000, "base reduce interval"),
    (reducelow, 500, 0, 1000, "low reduce fraction per mille"),
    (reluctant, 1, 0, 1, "stable reluctant doubling restarting"),
    (reluctantint, 1 << 10, 2, 1 << 15, "reluctant interval"),
    (reluctantlim, 1 << 20, 0, 1 << 30, "reluctant limit (0=unlimited)"),
    (reorder, 2, 0, 2, "reorder decisions (1=stable-mode-only)"),
    (reorderinit, 10_000, 0, 100_000, "initial reorder interval"),
    (reorderint, 10_000, 1, 100_000, "base reorder interval"),
    (reordermaxsize, 100, 2, 256, "reorder maximum clause size"),
    (rephase, 1, 0, 1, "reinitialization of decision phases"),
    (rephaseinit, 1000, 10, 100_000, "initial rephase interval"),
    (rephaseint, 1000, 10, 100_000, "base rephase interval"),
    (restart, 1, 0, 1, "enable restarts"),
    (restartint, RESTARTINT_DEFAULT, 1, 10_000, "base restart interval"),
    (restartmargin, 10, 0, 25, "fast/slow margin in percent"),
    (restartreusetrail, 1, 0, 1, "restarts tries to reuse trail"),
    (seed, 0, 0, i32::MAX, "random seed"),
    (shrink, 3, 0, 3, "learned clauses (1=bin,2=lrg,3=rec)"),
    (simplify, 1, 0, 1, "enable probing and elimination"),
    (smallclauses, 100_000, 0, i32::MAX, "small clauses limit"),
    (stable, STABLE_DEFAULT, 0, 2, "enable stable search mode"),
    (statistics, 0, 0, 1, "print complete statistics"),
    (substitute, 1, 0, 1, "equivalent literal substitution"),
    (substituteeffort, 10, 1, 1000, "effort in per mille"),
    (substituterounds, 2, 1, 100, "maximum substitution rounds"),
    (subsumeclslim, 1000, 1, i32::MAX, "subsumption clause size limit"),
    (subsumeocclim, 1000, 0, i32::MAX, "subsumption occurrence limit"),
    (sweep, 1, 0, 1, "enable SAT sweeping"),
    (sweepclauses, 1024, 0, i32::MAX, "environment clauses"),
    (sweepcomplete, 0, 0, 1, "run SAT sweeping until completion"),
    (sweepdepth, 2, 0, i32::MAX, "environment depth"),
    (sweepeffort, 100, 0, 10_000, "effort in per mille"),
    (sweepfliprounds, 1, 0, i32::MAX, "flipping rounds"),
    (sweepmaxclauses, 32_768, 2, i32::MAX, "maximum environment clauses"),
    (sweepmaxdepth, 3, 1, i32::MAX, "maximum environment depth"),
    (sweepmaxvars, 8192, 2, i32::MAX, "maximum environment variables"),
    (sweeprand, 0, 0, 1, "randomize sweeping environment"),
    (sweepvars, 256, 0, i32::MAX, "environment variables"),
    (target, TARGET_DEFAULT, 0, 2, "target phases (1=stable,2=focused)"),
    (tier1, 2, 1, 100, "learned clause tier one glue limit"),
    (tier1relative, 500, 0, 1000, "relative tier one glue limit"),
    (tier2, 6, 1, 1000, "learned clause tier two glue limit"),
    (tier2relative, 900, 0, 1000, "relative tier two glue limit"),
    (transitive, 1, 0, 1, "transitive reduction of binary clauses"),
    (transitiveeffort, 20, 0, 2000, "effort in per mille"),
    (transitivekeep, 1, 0, 1, "keep transitivity candidates"),
    (tumble, 1, 0, 1, "tumbled external indices order"),
    (verbose, 0, 0, 3, "verbosity level"),
    (vivify, 1, 0, 1, "vivify clauses"),
    (vivifyeffort, 100, 0, 1000, "effort in per mille"),
    (vivifyfocusedtiers, 1, 0, 1, "use focused tier limits"),
    (vivifyirr, 3, 0, 100, "relative irredundant effort"),
    (vivifysort, 1, 0, 1, "sort vivification candidates"),
    (vivifytier1, 3, 0, 100, "relative tier1 effort"),
    (vivifytier2, 3, 0, 100, "relative tier2 effort"),
    (vivifytier3, 1, 0, 100, "relative tier3 effort"),
    (walkeffort, 50, 0, 1_000_000, "effort in per mille"),
    (walkinitially, 0, 0, 1, "initial local search"),
    (warmup, 1, 0, 1, "initialize phases by unit propagation"),
}

impl Options {
    /// C: `TIER1RELATIVE` — `GET_OPTION (tier1relative) / 1000.0`.
    #[inline]
    pub fn tier1_relative(&self) -> f64 {
        self.tier1relative as f64 / 1000.0
    }

    /// C: `TIER2RELATIVE` — `GET_OPTION (tier2relative) / 1000.0`.
    #[inline]
    pub fn tier2_relative(&self) -> f64 {
        self.tier2relative as f64 / 1000.0
    }
}

/// C: `GET1K_OPTION (NAME)` — `((int64_t) 1000) * GET_OPTION (NAME)`.
/// Call as `get1k_option (solver.options.name)`.
#[inline]
pub fn get1k_option(value: i32) -> i64 {
    1000i64 * value as i64
}

/// C: `kissat_init_options` (the runtime table checks are debug-only).
pub fn init_options(options: &mut Options) {
    *options = Options::default();
}

/// C: `kissat_options_has` — binary search over the sorted table, identical
/// probe sequence.
pub fn options_has(name: &str) -> Option<&'static OptionEntry> {
    let table = OPTION_TABLE;
    let mut l: usize = 0;
    let mut r: usize = table.len();
    while l + 1 < r {
        let m = l + (r - l) / 2;
        let o = &table[m];
        use core::cmp::Ordering;
        match name.cmp(o.name) {
            Ordering::Less => r = m,
            Ordering::Greater => l = m,
            Ordering::Equal => return Some(o),
        }
    }
    let o = &table[l];
    if o.name == name {
        Some(o)
    } else {
        None
    }
}

/// C: `kissat_options_get` — 0 for unknown names.
pub fn options_get(options: &Options, name: &str) -> i32 {
    match options_has(name) {
        Some(o) => options.value_by_name(o.name).unwrap(),
        None => 0,
    }
}

/// C: `kissat_options_set_opt` — clamps to `[low, high]`, returns the
/// PREVIOUS value (unchanged value short-circuits).
pub fn options_set_opt(options: &mut Options, o: &OptionEntry, mut value: i32) -> i32 {
    let p = options
        .value_mut_by_name(o.name)
        .expect("entry from OPTION_TABLE");
    let res = *p;
    if value == res {
        return res;
    }
    if value < o.low {
        value = o.low;
    }
    if value > o.high {
        value = o.high;
    }
    *p = value;
    res
}

/// C: `kissat_options_set` — 0 for unknown names, else `options_set_opt`.
/// (`kissat_set_option` in internal.c is a thin wrapper over this.)
pub fn options_set(options: &mut Options, name: &str, value: i32) -> i32 {
    match options_has(name) {
        Some(o) => options_set_opt(options, o, value),
        None => 0,
    }
}

fn peek_ch(b: &[u8], pos: usize) -> u8 {
    if pos < b.len() {
        b[pos]
    } else {
        0
    }
}

fn next_ch(b: &[u8], pos: &mut usize) -> u8 {
    let ch = peek_ch(b, *pos);
    *pos += 1;
    ch
}

/// C: `kissat_parse_option_value` — accepts `true`, `false`, decimal integers
/// with optional leading `-`, `<mantissa>e<digit>` (one exponent digit unless
/// the mantissa is 0) and `<base>^<exp>` (at most two exponent digits) forms,
/// with exact overflow rejection against `-(unsigned) INT_MIN`.
pub fn parse_option_value(val_str: &str) -> Option<i32> {
    if val_str == "true" {
        return Some(1);
    }
    if val_str == "false" {
        return Some(0);
    }
    let b = val_str.as_bytes();
    let mut pos: usize = 0;
    let mut sign: i32 = 1;
    let mut ch = next_ch(b, &mut pos);
    if ch == b'-' {
        sign = -1;
        ch = next_ch(b, &mut pos);
    }
    if !ch.is_ascii_digit() {
        // at least one digit
        return None;
    }
    const MAX: u32 = 2147483648; // C: -(unsigned) INT_MIN
    let mut res: u32 = (ch - b'0') as u32;
    loop {
        ch = next_ch(b, &mut pos);
        if !ch.is_ascii_digit() {
            break;
        }
        if MAX / 10 < res {
            return None;
        }
        res *= 10;
        let digit = (ch - b'0') as u32;
        if MAX - digit < res {
            return None;
        }
        res += digit;
        if res == 0 {
            return None; // invalid '00'
        }
    }
    if ch == b'e' {
        // parse '13e5' etc.
        ch = next_ch(b, &mut pos);
        if !ch.is_ascii_digit() {
            // at least one digit
            return None;
        }
        if res != 0 {
            if peek_ch(b, pos) != 0 {
                // exactly one digit
                return None;
            }
            let digit = (ch - b'0') as u32;
            for _ in 0..digit {
                if MAX / 10 < res {
                    return None;
                }
                res *= 10;
            }
        } else {
            // parse '0e123123123' etc.
            loop {
                ch = next_ch(b, &mut pos);
                if !ch.is_ascii_digit() {
                    break;
                }
            }
            if ch != 0 {
                return None;
            }
        }
    } else if ch == b'^' {
        // parse '2^11' etc.
        let base = res;
        ch = next_ch(b, &mut pos);
        if !ch.is_ascii_digit() {
            // at least one digit
            return None;
        }
        let mut exp: u32 = (ch - b'0') as u32;
        if base < 2 {
            // parse '0^123123123' etc.
            loop {
                ch = next_ch(b, &mut pos);
                if !ch.is_ascii_digit() {
                    break;
                }
            }
            if ch != 0 {
                return None;
            }
        } else {
            ch = next_ch(b, &mut pos);
            if ch.is_ascii_digit() {
                // parse '2^30' etc.
                if peek_ch(b, pos) != 0 {
                    // at most two digits
                    return None;
                }
                exp *= 10;
                let digit = (ch - b'0') as u32;
                exp += digit;
                if exp == 0 {
                    // '2^00' invalid
                    return None;
                }
            } else if ch != 0 {
                return None;
            }
        }
        if exp != 0 {
            for _ in 1..exp {
                // PORT NOTE: for base == 0 with exp >= 2 (e.g. '0^25') the C
                // code computes `max / base` with base 0 — an integer divide
                // by zero (SIGFPE on the reference build). Rust panics on the
                // same inputs; the quirk is preserved, not fixed.
                if MAX / base < res {
                    return None;
                }
                res *= base;
            }
        } else if base != 0 {
            res = 1; // parse '3^0'
        } else {
            return None; // '0^0' invalid
        }
    } else if ch != 0 {
        return None;
    }
    debug_assert!(res <= MAX);
    if sign > 0 && res == MAX {
        return None;
    }
    // C: `res *= sign` on an unsigned, then implicit conversion to int —
    // two's-complement negation; `-2147483648` maps to INT_MIN.
    Some(if sign < 0 {
        res.wrapping_neg() as i32
    } else {
        res as i32
    })
}

/// C: `kissat_parse_option_name` — matches `--<name>=<val>` (returning the
/// value substring) or `--no-<name>` (returning `"0"`); anything else is None.
pub fn parse_option_name<'a>(arg: &'a str, name: &str) -> Option<&'a str> {
    let a = arg.as_bytes();
    if a.len() < 2 || a[0] != b'-' || a[1] != b'-' {
        return None;
    }
    let rest = &arg[2..];
    if rest.as_bytes().starts_with(b"no-") {
        return if &rest[3..] == name { Some("0") } else { None };
    }
    let tail = rest.strip_prefix(name)?;
    let tb = tail.as_bytes();
    if tb.first() != Some(&b'=') {
        return None;
    }
    Some(&tail[1..])
}

/// C: `kissat_options_parse_arg` — validates `--<name>`, `--no-<name>`, and
/// `--<name>=<value>` against the option table. Returns `(name, value)`.
/// Unlike `options_set_opt` (which clamps), an explicit `=<value>` outside
/// `[low, high]` is REJECTED; bare `--<name>` requires `high >= 1` and
/// `--no-<name>` requires `low <= 0`.
pub fn options_parse_arg(arg: &str) -> Option<(&str, i32)> {
    let a = arg.as_bytes();
    if a.len() < 2 || a[0] != b'-' || a[1] != b'-' {
        return None;
    }
    let rest = &arg[2..];
    if let Some(eq) = rest.find('=') {
        let name = &rest[..eq];
        if name.len() >= OPTIONS_MAX_NAME_BUFFER_SIZE {
            return None;
        }
        let o = options_has(name)?;
        let value = parse_option_value(&rest[eq + 1..])?;
        if value < o.low || value > o.high {
            return None;
        }
        Some((name, value))
    } else if rest.as_bytes().starts_with(b"no-") {
        let name = &rest[3..];
        let o = options_has(name)?;
        if o.low > 0 {
            return None;
        }
        Some((name, 0))
    } else {
        let o = options_has(rest)?;
        if o.high < 1 {
            return None;
        }
        Some((rest, 1))
    }
}

// --- printing (usage / fuzzing lists), local port of the format.c pieces ---

/// C: `format_count` (format.c) — powers of two >= 128 print as `2^N`,
/// multiples of 1000 print as `<m>e<z>` with ALL trailing zeros stripped.
fn format_count(w: u64) -> String {
    if w >= 128 && (w & (w - 1)) == 0 {
        let mut l: u32 = 0;
        while (1u64 << l) != w {
            l += 1;
        }
        format!("2^{}", l)
    } else if w >= 1000 && w % 1000 == 0 {
        let mut w = w;
        let mut l: u32 = 0;
        while w % 10 == 0 {
            w /= 10;
            l += 1;
        }
        format!("{}e{}", w, l)
    } else {
        format!("{}", w)
    }
}

/// C: `kissat_format_value` (format.c).
fn format_value(boolean: bool, value: i32) -> String {
    if boolean && value != 0 {
        return "true".to_string();
    }
    if boolean {
        return "false".to_string();
    }
    if value == i32::MAX {
        return "INT_MAX".to_string();
    }
    if value == i32::MIN {
        return "INT_MIN".to_string();
    }
    if value < 0 {
        format!("-{}", format_count(value.unsigned_abs() as u64))
    } else {
        format_count(value as u64)
    }
}

/// C: `FORMAT_OPTION_LIMIT` — INT_MIN/INT_MAX limits print as `.`.
fn format_option_limit(value: i32) -> String {
    if value == i32::MIN || value == i32::MAX {
        ".".to_string()
    } else {
        format_value(false, value)
    }
}

/// C: `kissat_options_usage` — one `  --name=<range>  description [default]`
/// line per option (via `kissat_printf_usage`, `"  %-26s "` field).
pub fn options_usage() {
    for o in OPTION_TABLE {
        let b = o.low == 0 && o.high == 1;
        let buffer = if b {
            format!("--{}=<bool>", o.name)
        } else {
            format!(
                "--{}={}..{}",
                o.name,
                format_option_limit(o.low),
                format_option_limit(o.high)
            )
        };
        let val_str = format_value(b, o.value);
        println!("  {:<26} {} [{}]", buffer, o.description, val_str);
    }
}

/// C: `ignore_embedded_option_for_fuzzing` — with QUIET undefined only
/// `quiet` is skipped (`embedded` is compiled out of this build).
fn ignore_embedded_option_for_fuzzing(name: &str) -> bool {
    name == "quiet"
}

/// C: `kissat_print_embedded_option_list`.
pub fn print_embedded_option_list() {
    for o in OPTION_TABLE {
        if !ignore_embedded_option_for_fuzzing(o.name) {
            println!("c --{}={}", o.name, o.value);
        }
    }
}

/// C: `ignore_range_option_for_fuzzing` — `log`/`embedded` checks are
/// compiled out of this build; `quiet` plus four search options are skipped.
fn ignore_range_option_for_fuzzing(name: &str) -> bool {
    if name == "quiet" {
        return true;
    }
    if name == "reduce" {
        return true;
    }
    if name == "reluctant" {
        return true;
    }
    if name == "rephase" {
        return true;
    }
    if name == "restart" {
        return true;
    }
    false
}

/// C: `kissat_print_option_range_list` — `name low default high` lines.
pub fn print_option_range_list() {
    for o in OPTION_TABLE {
        if !ignore_range_option_for_fuzzing(o.name) {
            println!("{} {} {} {}", o.name, o.low, o.value, o.high);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_all_158_options() {
        assert_eq!(OPTION_TABLE.len(), 158);
    }

    #[test]
    fn table_sorted_and_ranges_valid() {
        // C: check_table_sorted + check_ranges.
        let mut prev: Option<&str> = None;
        for o in OPTION_TABLE {
            if let Some(p) = prev {
                assert!(p < o.name, "option '{}' before option '{}'", p, o.name);
            }
            prev = Some(o.name);
            assert!(o.low <= o.high, "{}", o.name);
            assert!(o.low <= o.value && o.value <= o.high, "{}", o.name);
            assert!(o.name.len() + 1 <= OPTIONS_MAX_NAME_BUFFER_SIZE);
        }
    }

    #[test]
    fn defaults_match_table() {
        let options = Options::default();
        for o in OPTION_TABLE {
            assert_eq!(options_get(&options, o.name), o.value, "{}", o.name);
        }
        // Spot-check the config.h-macro-derived defaults.
        assert_eq!(options.target, 1);
        assert_eq!(options.stable, 1);
        assert_eq!(options.restartint, 1);
        assert_eq!(options.reluctantint, 1024);
        assert_eq!(options.reluctantlim, 1_048_576);
        assert_eq!(options.defragsize, 262_144);
    }

    #[test]
    fn parse_value_forms() {
        assert_eq!(parse_option_value("true"), Some(1));
        assert_eq!(parse_option_value("false"), Some(0));
        assert_eq!(parse_option_value("0"), Some(0));
        assert_eq!(parse_option_value("-17"), Some(-17));
        assert_eq!(parse_option_value("13e5"), Some(1_300_000));
        assert_eq!(parse_option_value("0e123123123"), Some(0));
        assert_eq!(parse_option_value("2^11"), Some(2048));
        assert_eq!(parse_option_value("2^30"), Some(1 << 30));
        assert_eq!(parse_option_value("3^0"), Some(1));
        assert_eq!(parse_option_value("0^123123123"), Some(0));
        assert_eq!(parse_option_value("1^23"), Some(1));
        assert_eq!(parse_option_value("2147483647"), Some(i32::MAX));
        assert_eq!(parse_option_value("-2147483648"), Some(i32::MIN));
        assert_eq!(parse_option_value("2147483648"), None);
        assert_eq!(parse_option_value("00"), None);
        assert_eq!(parse_option_value("2^00"), None);
        assert_eq!(parse_option_value("0^0"), None);
        assert_eq!(parse_option_value("13e55"), None); // two exponent digits
        assert_eq!(parse_option_value("2^100"), None); // three exponent digits
        assert_eq!(parse_option_value(""), None);
        assert_eq!(parse_option_value("-"), None);
        assert_eq!(parse_option_value("x"), None);
        assert_eq!(parse_option_value("1x"), None);
    }

    #[test]
    fn parse_arg_forms() {
        assert_eq!(options_parse_arg("--chrono"), Some(("chrono", 1)));
        assert_eq!(options_parse_arg("--no-chrono"), Some(("chrono", 0)));
        assert_eq!(options_parse_arg("--chrono=true"), Some(("chrono", 1)));
        assert_eq!(options_parse_arg("--decay=200"), Some(("decay", 200)));
        assert_eq!(options_parse_arg("--decay=201"), None); // out of range: reject
        assert_eq!(options_parse_arg("--no-decay"), None); // low is 1
        assert_eq!(options_parse_arg("--unknown"), None);
        assert_eq!(options_parse_arg("-chrono"), None);
    }

    #[test]
    fn parse_name_forms() {
        assert_eq!(parse_option_name("--chrono=1", "chrono"), Some("1"));
        assert_eq!(parse_option_name("--no-chrono", "chrono"), Some("0"));
        assert_eq!(parse_option_name("--chrono", "chrono"), None);
        assert_eq!(parse_option_name("--chronolevels=5", "chrono"), None);
    }

    #[test]
    fn set_clamps_and_returns_previous() {
        let mut options = Options::default();
        assert_eq!(options_set(&mut options, "decay", 1000), 50);
        assert_eq!(options.decay, 200); // clamped to high
        assert_eq!(options_set(&mut options, "decay", -7), 200);
        assert_eq!(options.decay, 1); // clamped to low
        assert_eq!(options_set(&mut options, "bogus", 3), 0);
    }

    #[test]
    fn format_matches_kissat() {
        assert_eq!(format_value(false, 1000), "1e3");
        assert_eq!(format_value(false, 100_000), "1e5");
        assert_eq!(format_value(false, 1024), "2^10");
        assert_eq!(format_value(false, 50_000_000), "5e7");
        assert_eq!(format_value(false, 990), "990");
        assert_eq!(format_value(false, i32::MAX), "INT_MAX");
        assert_eq!(format_value(true, 1), "true");
        assert_eq!(format_value(true, 0), "false");
    }
}
