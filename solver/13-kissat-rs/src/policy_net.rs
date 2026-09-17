// Not in kissat. The learned policy: weights-file loader, forward pass and
// the margin over stock (plan §6.4, §7 item 8; step A.10, bead
// SAT-playground-p9m.6.10).
//
// `SAT_POLICY=<file>` loads a flat little-endian file written by
// tools/rl/policy_net.py (the same script is the reference forward pass
// and the exporter for a trained PyTorch model):
//
//   bytes 0-15   "SAT13POLICYNET\0\0"
//   u32          format version (1)
//   u32          n_in, the observation length (policy_obs::names().len())
//   u64          layout hash: FNV-1a 64 of the observation names joined by
//                "\n", so a file trained on another layout is refused
//   u32 u32 u32  hidden sizes h1, h2 and the number of heads (8)
//   per head     u32 kind (0 = a head on the trunk, 1 = linear on the
//                normalized input, 2 = reserved for tree rankers), u32
//                n_out, u32 stock index
//   f32 arrays   mean[n_in], std[n_in], W1[h1 x n_in], b1[h1],
//                W2[h2 x h1], b2[h2], then per head W[n_out x h2] (kind 0)
//                or W[n_out x n_in] (kind 1) and b[n_out]
//
// Matrices are row-major, one row per output. The heads are, in order:
// probe, eliminate, reduce, rephase, reorder (5-way interval menus, stock
// at index 2), mode (3-way, stock 1), restart margin (3-way, stock 1),
// sweep effort (4-way, stock 2); the loader checks each against the
// menus in policy.rs.
//
// Arithmetic. Weights are f32 in the file; the forward pass runs in f64
// with plain multiply-then-add in a fixed order (no FMA: Rust never
// contracts), so the scores are the same on every host and equal the
// pure-Python reference bit for bit. Normalization is (x - mean) / std
// with a non-positive std treated as 1.
//
// Acting (plan §6.4). Per head the entry with the highest score is taken
// only when its score beats the stock entry's by `SAT_POLICY_MARGIN`
// (log-odds under a pairwise-logistic ranker; default 1, `inf` = always
// stock); the action is then masked like any other (policy::mask). An
// infinite margin must pass parity, which is the loader's plumbing check.

use crate::policy::{
    Action, Effort, Timer, INTERVAL_MENU, INTERVAL_STOCK, MARGIN_MENU, MARGIN_STOCK, MODE_MENU,
    MODE_STOCK, SWEEP_MENU, SWEEP_STOCK,
};

pub const MAGIC: &[u8; 16] = b"SAT13POLICYNET\0\0";
pub const FORMAT: u32 = 1;
pub const N_HEADS: usize = 8;

pub const HEAD_NAMES: [&str; N_HEADS] = [
    "probe", "eliminate", "reduce", "rephase", "reorder", "mode", "margin", "sweep",
];

/// (menu size, stock index) per head, from the menus in policy.rs.
pub fn head_menus() -> [(usize, usize); N_HEADS] {
    [
        (INTERVAL_MENU.len(), INTERVAL_STOCK),
        (INTERVAL_MENU.len(), INTERVAL_STOCK),
        (INTERVAL_MENU.len(), INTERVAL_STOCK),
        (INTERVAL_MENU.len(), INTERVAL_STOCK),
        (INTERVAL_MENU.len(), INTERVAL_STOCK),
        (MODE_MENU.len(), MODE_STOCK),
        (MARGIN_MENU.len(), MARGIN_STOCK),
        (SWEEP_MENU.len(), SWEEP_STOCK),
    ]
}

/// FNV-1a 64 of the observation names joined by "\n".
pub fn layout_hash(names: &[String]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut first = true;
    for n in names {
        if !first {
            h ^= b'\n' as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        first = false;
        for b in n.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

#[derive(Clone, Debug)]
pub struct Head {
    pub kind: u32,
    pub n_out: usize,
    pub stock: usize,
    /// Row-major, n_out rows of h2 (kind 0) or n_in (kind 1) weights.
    pub w: Vec<f32>,
    pub b: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub path: String,
    pub n_in: usize,
    pub h1: usize,
    pub h2: usize,
    pub layout_hash: u64,
    pub mean: Vec<f32>,
    pub std: Vec<f32>,
    pub w1: Vec<f32>,
    pub b1: Vec<f32>,
    pub w2: Vec<f32>,
    pub b2: Vec<f32>,
    pub heads: Vec<Head>,
    /// Scratch for the forward pass; allocated once at load.
    pub x: Vec<f64>,
    pub a1: Vec<f64>,
    pub a2: Vec<f64>,
    /// The scores of the last forward pass, head by head (35 values).
    pub scores: Vec<f64>,
    /// Offsets of each head's scores in `scores`.
    pub offsets: [usize; N_HEADS],
    pub n_scores: usize,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    path: &'a str,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8], String> {
        if self.pos + n > self.data.len() {
            return Err(format!(
                "SAT_POLICY='{}': truncated file (needs {} more bytes for {})",
                self.path,
                self.pos + n - self.data.len(),
                what
            ));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u32(&mut self, what: &str) -> Result<u32, String> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self, what: &str) -> Result<u64, String> {
        let b = self.take(8, what)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn f32s(&mut self, n: usize, what: &str) -> Result<Vec<f32>, String> {
        let b = self.take(4 * n, what)?;
        let mut v = Vec::with_capacity(n);
        for c in b.chunks_exact(4) {
            let x = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            if !x.is_finite() {
                return Err(format!("SAT_POLICY='{}': non-finite value in {}", self.path, what));
            }
            v.push(x);
        }
        Ok(v)
    }
}

/// Parse a weights file. `names` are the observation entry names of this
/// binary, which the file's layout hash and length must match.
pub fn parse(path: &str, data: &[u8], names: &[String]) -> Result<Net, String> {
    let mut r = Reader { data, pos: 0, path };
    if r.take(16, "magic")? != MAGIC {
        return Err(format!("SAT_POLICY='{}': not a policy weights file (bad magic)", path));
    }
    let version = r.u32("version")?;
    if version != FORMAT {
        return Err(format!(
            "SAT_POLICY='{}': format {} but this binary reads format {}",
            path, version, FORMAT
        ));
    }
    let n_in = r.u32("n_in")? as usize;
    let hash = r.u64("layout hash")?;
    let expected = layout_hash(names);
    if n_in != names.len() || hash != expected {
        return Err(format!(
            "SAT_POLICY='{}': trained on another observation layout ({} inputs, hash {:016x}) than this binary's ({} inputs, hash {:016x})",
            path, n_in, hash, names.len(), expected
        ));
    }
    let h1 = r.u32("h1")? as usize;
    let h2 = r.u32("h2")? as usize;
    let n_heads = r.u32("n_heads")? as usize;
    if n_heads != N_HEADS {
        return Err(format!("SAT_POLICY='{}': {} heads, expected {}", path, n_heads, N_HEADS));
    }
    let limit = 1usize << 24;
    if h1 == 0 || h2 == 0 || h1 > limit || h2 > limit {
        return Err(format!("SAT_POLICY='{}': bad hidden sizes {} x {}", path, h1, h2));
    }
    let menus = head_menus();
    let mut shapes = Vec::with_capacity(N_HEADS);
    for i in 0..N_HEADS {
        let kind = r.u32("head kind")?;
        let n_out = r.u32("head size")? as usize;
        let stock = r.u32("head stock index")? as usize;
        if kind == 2 {
            return Err(format!(
                "SAT_POLICY='{}': head {} is a tree ranker, which this binary cannot run yet (plan step E.3)",
                path, HEAD_NAMES[i]
            ));
        }
        if kind > 1 {
            return Err(format!("SAT_POLICY='{}': head {} has unknown kind {}", path, HEAD_NAMES[i], kind));
        }
        if (n_out, stock) != menus[i] {
            return Err(format!(
                "SAT_POLICY='{}': head {} has {} entries with stock at {}, this binary's menu has {} with stock at {}",
                path, HEAD_NAMES[i], n_out, stock, menus[i].0, menus[i].1
            ));
        }
        shapes.push((kind, n_out, stock));
    }
    let mean = r.f32s(n_in, "mean")?;
    let mut std = r.f32s(n_in, "std")?;
    for s in std.iter_mut() {
        if *s <= 0.0 {
            *s = 1.0;
        }
    }
    let w1 = r.f32s(h1 * n_in, "W1")?;
    let b1 = r.f32s(h1, "b1")?;
    let w2 = r.f32s(h2 * h1, "W2")?;
    let b2 = r.f32s(h2, "b2")?;
    let mut heads = Vec::with_capacity(N_HEADS);
    let mut offsets = [0usize; N_HEADS];
    let mut n_scores = 0;
    for (i, (kind, n_out, stock)) in shapes.into_iter().enumerate() {
        let width = if kind == 0 { h2 } else { n_in };
        let w = r.f32s(n_out * width, "head weights")?;
        let b = r.f32s(n_out, "head bias")?;
        offsets[i] = n_scores;
        n_scores += n_out;
        heads.push(Head { kind, n_out, stock, w, b });
    }
    if r.pos != data.len() {
        return Err(format!(
            "SAT_POLICY='{}': {} trailing bytes after the weights",
            path,
            data.len() - r.pos
        ));
    }
    Ok(Net {
        path: path.to_string(),
        n_in,
        h1,
        h2,
        layout_hash: hash,
        mean,
        std,
        w1,
        b1,
        w2,
        b2,
        heads,
        x: vec![0.0; n_in],
        a1: vec![0.0; h1],
        a2: vec![0.0; h2],
        scores: vec![0.0; n_scores],
        offsets,
        n_scores,
    })
}

/// Load a weights file for this binary's observation layout.
pub fn load(path: &str) -> Result<Net, String> {
    let data = std::fs::read(path).map_err(|e| format!("SAT_POLICY='{}': cannot read: {}", path, e))?;
    let names = crate::policy_obs::names();
    parse(path, &data, &names)
}

impl Net {
    /// Run the net on `obs` (the raw observation, f32 as logged), leaving
    /// the scores in `self.scores`. Allocation-free.
    pub fn forward(&mut self, obs: &[f32]) {
        debug_assert_eq!(obs.len(), self.n_in);
        let n_in = self.n_in;
        for i in 0..n_in {
            self.x[i] = (obs[i] as f64 - self.mean[i] as f64) / self.std[i] as f64;
        }
        for j in 0..self.h1 {
            let row = &self.w1[j * n_in..(j + 1) * n_in];
            let mut acc = self.b1[j] as f64;
            for i in 0..n_in {
                acc += row[i] as f64 * self.x[i];
            }
            self.a1[j] = if acc > 0.0 { acc } else { 0.0 };
        }
        let h1 = self.h1;
        for j in 0..self.h2 {
            let row = &self.w2[j * h1..(j + 1) * h1];
            let mut acc = self.b2[j] as f64;
            for i in 0..h1 {
                acc += row[i] as f64 * self.a1[i];
            }
            self.a2[j] = if acc > 0.0 { acc } else { 0.0 };
        }
        for (k, head) in self.heads.iter().enumerate() {
            let (input, width): (&[f64], usize) = if head.kind == 0 {
                (&self.a2, self.h2)
            } else {
                (&self.x, n_in)
            };
            let base = self.offsets[k];
            for j in 0..head.n_out {
                let row = &head.w[j * width..(j + 1) * width];
                let mut acc = head.b[j] as f64;
                for i in 0..width {
                    acc += row[i] as f64 * input[i];
                }
                self.scores[base + j] = acc;
            }
        }
    }

    /// The chosen entry per head from the last scores: the best entry
    /// when it beats stock by more than `margin`, else stock.
    pub fn choose(&self, margin: f64) -> [usize; N_HEADS] {
        let mut out = [0usize; N_HEADS];
        for (k, head) in self.heads.iter().enumerate() {
            let s = &self.scores[self.offsets[k]..self.offsets[k] + head.n_out];
            let mut best = head.stock;
            for j in 0..head.n_out {
                if s[j] > s[best] {
                    best = j;
                }
            }
            out[k] = if best != head.stock && s[best] - s[head.stock] > margin {
                best
            } else {
                head.stock
            };
        }
        out
    }
}

/// The action for a choice of menu entries.
pub fn action_from_choice(choice: &[usize; N_HEADS]) -> Action {
    let mut act = Action::default();
    for (k, t) in [Timer::Probe, Timer::Eliminate, Timer::Reduce, Timer::Rephase, Timer::Reorder]
        .iter()
        .enumerate()
    {
        act.interval_mult[*t as usize] = INTERVAL_MENU[choice[k]];
    }
    act.interval_mult[Timer::Mode as usize] = MODE_MENU[choice[5]];
    act.restart_margin = MARGIN_MENU[choice[6]];
    act.effort_mult[Effort::Sweep as usize] = SWEEP_MENU[choice[7]];
    act
}

/// Parse `SAT_POLICY_MARGIN`: a non-negative number or `inf`.
pub fn parse_margin(v: &str) -> Result<f64, String> {
    let t = v.trim();
    if t.eq_ignore_ascii_case("inf") || t.eq_ignore_ascii_case("infinity") {
        return Ok(f64::INFINITY);
    }
    t.parse::<f64>()
        .ok()
        .filter(|m| !m.is_nan() && *m >= 0.0)
        .ok_or_else(|| format!("SAT_POLICY_MARGIN='{}': expected a number >= 0 or inf", v))
}

/// Header text for the log: the file, sizes and margin.
pub fn header_json(net: &Net, margin: f64) -> String {
    let mut s = String::with_capacity(256);
    s.push_str("{\"path\":");
    crate::policy_log::json_escape_into(&net.path, &mut s);
    s.push_str(&format!(
        ",\"n_in\":{},\"hidden\":[{},{}],\"layout_hash\":\"{:016x}\",\"margin\":{},\"heads\":[",
        net.n_in,
        net.h1,
        net.h2,
        net.layout_hash,
        if margin.is_infinite() { "\"inf\"".to_string() } else { format!("{}", margin) }
    ));
    for (k, h) in net.heads.iter().enumerate() {
        if k > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"name\":\"{}\",\"kind\":{},\"n_out\":{},\"stock\":{}}}",
            HEAD_NAMES[k], h.kind, h.n_out, h.stock
        ));
    }
    s.push_str("]}");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a deterministic net for the current layout (a seeded LCG,
    /// biased so stock starts ahead as plan §6.4 asks); returns the bytes.
    fn fixture_bytes(names: &[String], h1: usize, h2: usize, seed: u64, kind1_heads: bool) -> Vec<u8> {
        let mut rng = seed;
        let mut next = || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((rng >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        };
        let n_in = names.len();
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT.to_le_bytes());
        out.extend_from_slice(&(n_in as u32).to_le_bytes());
        out.extend_from_slice(&layout_hash(names).to_le_bytes());
        out.extend_from_slice(&(h1 as u32).to_le_bytes());
        out.extend_from_slice(&(h2 as u32).to_le_bytes());
        out.extend_from_slice(&(N_HEADS as u32).to_le_bytes());
        let menus = head_menus();
        for (k, (n, st)) in menus.iter().enumerate() {
            let kind: u32 = if kind1_heads && k % 2 == 1 { 1 } else { 0 };
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&(*n as u32).to_le_bytes());
            out.extend_from_slice(&(*st as u32).to_le_bytes());
        }
        let mut push = |v: f32, out: &mut Vec<u8>| out.extend_from_slice(&v.to_le_bytes());
        for _ in 0..n_in {
            push(next() as f32 * 0.5, &mut out);
        }
        for _ in 0..n_in {
            push((1.0 + next().abs()) as f32, &mut out);
        }
        for _ in 0..h1 * n_in {
            push((next() * 0.2) as f32, &mut out);
        }
        for _ in 0..h1 {
            push((next() * 0.1) as f32, &mut out);
        }
        for _ in 0..h2 * h1 {
            push((next() * 0.3) as f32, &mut out);
        }
        for _ in 0..h2 {
            push((next() * 0.1) as f32, &mut out);
        }
        for (k, (n, st)) in menus.iter().enumerate() {
            let kind = if kind1_heads && k % 2 == 1 { 1 } else { 0 };
            let width = if kind == 0 { h2 } else { n_in };
            for _ in 0..n * width {
                push((next() * 0.3) as f32, &mut out);
            }
            for j in 0..*n {
                push(if j == *st { 3.0 } else { next() as f32 * 0.5 }, &mut out);
            }
        }
        out
    }

    fn write_fixture(dir: &std::path::Path, names: &[String], kind1: bool) -> (std::path::PathBuf, Net) {
        let bytes = fixture_bytes(names, 32, 16, 42, kind1);
        let path = dir.join(if kind1 { "mixed.bin" } else { "net.bin" });
        std::fs::write(&path, &bytes).unwrap();
        let net = parse(path.to_str().unwrap(), &bytes, names).expect("parse fixture");
        (path, net)
    }

    #[test]
    fn forward_matches_the_python_reference_to_the_bit() {
        let names = crate::policy_obs::names();
        let dir = std::env::temp_dir().join(format!("sat13-policy-net-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/rl/policy_net.py");
        for kind1 in [false, true] {
            let (path, mut net) = write_fixture(&dir, &names, kind1);
            assert_eq!(net.n_scores, 5 * 5 + 3 + 3 + 4);
            // Eight inputs: zeros, a one-hot, and six random vectors in
            // the range the observation uses (log counts to ~20).
            let mut rng = 7u64;
            let mut cases: Vec<Vec<f32>> = vec![vec![0.0; names.len()]];
            let mut one = vec![0.0f32; names.len()];
            one[3] = 1.0;
            cases.push(one);
            for _ in 0..6 {
                let mut v = Vec::with_capacity(names.len());
                for _ in 0..names.len() {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    v.push(((rng >> 11) as f64 / (1u64 << 53) as f64 * 20.0) as f32);
                }
                cases.push(v);
            }
            let cases_path = dir.join("cases.txt");
            let mut text = String::new();
            for c in &cases {
                let line: Vec<String> = c.iter().map(|x| format!("{:?}", x)).collect();
                text.push_str(&line.join(" "));
                text.push('\n');
            }
            std::fs::write(&cases_path, text).unwrap();
            let out = std::process::Command::new("python3")
                .arg(script)
                .arg("--forward")
                .arg(&path)
                .arg(&cases_path)
                .output()
                .expect("python3 tools/rl/policy_net.py");
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(out.status.success(), "{}{}", stdout, String::from_utf8_lossy(&out.stderr));
            let mut lines = stdout.lines();
            let mut torch_lines: Vec<&str> = Vec::new();
            let mut ref_lines: Vec<&str> = Vec::new();
            let mut torch_available = false;
            for l in lines.by_ref() {
                if l == "torch unavailable" {
                    torch_available = false;
                    torch_lines.clear();
                } else if l.starts_with("python ") {
                    ref_lines.push(&l[7..]);
                } else if l.starts_with("torch ") {
                    torch_available = true;
                    torch_lines.push(&l[6..]);
                }
            }
            assert_eq!(ref_lines.len(), cases.len(), "{}", stdout);
            let mut worst_torch: f64 = 0.0;
            for (k, c) in cases.iter().enumerate() {
                net.forward(c);
                let expect: Vec<f64> = ref_lines[k].split_whitespace().map(|s| s.parse().unwrap()).collect();
                assert_eq!(expect.len(), net.n_scores);
                for (j, (a, b)) in net.scores.iter().zip(expect.iter()).enumerate() {
                    assert!(
                        a.to_bits() == b.to_bits() || (a - b).abs() <= 1e-12 * a.abs().max(1.0),
                        "kind1 {} case {} score {}: rust {:e} python {:e}",
                        kind1,
                        k,
                        j,
                        a,
                        b
                    );
                }
                if torch_available {
                    // PyTorch evaluates the same float32 weights in float64
                    // (tools/rl/policy_net.py): only summation order differs.
                    let t: Vec<f64> = torch_lines[k].split_whitespace().map(|s| s.parse().unwrap()).collect();
                    assert_eq!(t.len(), net.n_scores);
                    for (a, b) in net.scores.iter().zip(t.iter()) {
                        let d = (a - b).abs() / a.abs().max(1.0);
                        assert!(d <= 1e-6, "torch differs: {} v {}", a, b);
                        worst_torch = worst_torch.max(d);
                    }
                }
            }
            // The biased heads choose stock at the default margin and at
            // an infinite one; a negative-margin choice is the plain argmax.
            net.forward(&cases[0]);
            let stock: Vec<usize> = head_menus().iter().map(|m| m.1).collect();
            assert_eq!(net.choose(1.0).to_vec(), stock);
            assert_eq!(net.choose(f64::INFINITY).to_vec(), stock);
            assert_eq!(action_from_choice(&net.choose(f64::INFINITY)), crate::policy::STOCK);
            eprintln!(
                "policy_net fixture kind1={}: {} cases matched the python reference; torch {}",
                kind1,
                cases.len(),
                if torch_available { format!("max relative diff {:e}", worst_torch) } else { "not installed".to_string() }
            );
        }
        // Inference cost, once: microseconds per forward pass.
        let (_, mut net) = write_fixture(&dir, &names, false);
        let bytes = fixture_bytes(&names, 128, 64, 1, false);
        let mut big = parse("big", &bytes, &names).unwrap();
        let obs = vec![0.5f32; names.len()];
        let t0 = std::time::Instant::now();
        for _ in 0..2000 {
            big.forward(&obs);
        }
        let per = t0.elapsed().as_secs_f64() * 1e6 / 2000.0;
        net.forward(&obs);
        eprintln!("policy_net forward 150->128->64 (actual {} inputs): {:.1} us per pass", names.len(), per);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loader_refuses_bad_files() {
        let names = crate::policy_obs::names();
        let good = fixture_bytes(&names, 8, 4, 3, false);
        assert!(parse("f", &good, &names).is_ok());
        // Wrong magic, wrong version, wrong layout, truncated, trailing.
        let mut bad = good.clone();
        bad[0] = b'X';
        assert!(parse("f", &bad, &names).unwrap_err().contains("bad magic"));
        let mut bad = good.clone();
        bad[16] = 9;
        assert!(parse("f", &bad, &names).unwrap_err().contains("format 9"));
        let mut other = names.clone();
        other[0] = "renamed".to_string();
        assert!(parse("f", &good, &other).unwrap_err().contains("another observation layout"));
        let bad = &good[..good.len() - 3];
        assert!(parse("f", bad, &names).unwrap_err().contains("truncated"));
        let mut bad = good.clone();
        bad.push(0);
        assert!(parse("f", &bad, &names).unwrap_err().contains("trailing"));
        // A reserved tree head.
        let mut bad = good.clone();
        let head0 = 16 + 4 + 4 + 8 + 12;
        bad[head0..head0 + 4].copy_from_slice(&2u32.to_le_bytes());
        assert!(parse("f", &bad, &names).unwrap_err().contains("tree ranker"));
        // A head whose menu does not match.
        let mut bad = good.clone();
        bad[head0 + 4..head0 + 8].copy_from_slice(&4u32.to_le_bytes());
        assert!(parse("f", &bad, &names).unwrap_err().contains("entries with stock"));
        // NaN weights.
        let mut bad = good.clone();
        let mean0 = head0 + 12 * N_HEADS;
        bad[mean0..mean0 + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(parse("f", &bad, &names).unwrap_err().contains("non-finite"));
        assert_eq!(parse_margin("inf").unwrap(), f64::INFINITY);
        assert_eq!(parse_margin(" 0.5 ").unwrap(), 0.5);
        assert!(parse_margin("-1").is_err());
        assert!(parse_margin("nan").is_err());
    }
}
