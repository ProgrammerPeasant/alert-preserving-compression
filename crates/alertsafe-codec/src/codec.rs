//! Block codec with a per-sample error budget.
//!
//! Values are split into blocks of [`BLOCK`] samples. Each block is coded
//! either losslessly (Gorilla XOR) or as closed-loop DPCM on a power-of-two
//! grid `q = 2^e`: the encoder predicts from the previous *reconstructed*
//! value, so errors never accumulate, and it checks every reconstructed value
//! against the budget. A sample whose check fails is escaped and stored raw.
//! The guarantee `|y_i - x_i| <= eps_i` is therefore enforced on the actual
//! floating-point reconstruction, not derived from an idealized bound.
//!
//! In [`Mode::Counter`] the quantizer rounds towards `-inf` relative to the
//! previous reconstruction with a non-negative step count, so the output is
//! non-decreasing wherever the input is and `y_i <= x_i` always: no spurious
//! counter resets can appear, at the cost of one bit of precision.

use crate::bits::{BitReader, BitWriter};

pub const BLOCK: usize = 128;
const E_MIN: i32 = -126;
const E_MAX: i32 = 126;
/// Largest step count coded in a lossy block; beyond it the sample escapes.
const K_MAX: f64 = (1u64 << 50) as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Gauge,
    Counter,
}

#[derive(Clone, Debug)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    /// Exact payload size in bits (the byte vector is padded).
    pub bits: usize,
    pub n: usize,
}

impl Encoded {
    pub fn bits_per_value(&self) -> f64 {
        self.bits as f64 / self.n.max(1) as f64
    }
}

/// Encodes `vals` so that the decoded value `y_i` satisfies
/// `|y_i - vals[i]| <= budget[i]`. A budget of `0` (or a non-finite value)
/// forces an exact sample; `f64::INFINITY` leaves the sample unconstrained.
pub fn encode(vals: &[f64], budget: &[f64], mode: Mode) -> Encoded {
    assert_eq!(vals.len(), budget.len());
    let mut w = BitWriter::new();
    w.write((mode == Mode::Counter) as u64, 1);
    w.write(vals.len() as u64, 32);
    let mut y_prev = 0.0f64;
    let mut xor = XorState::default();
    for (xs, eps) in vals.chunks(BLOCK).zip(budget.chunks(BLOCK)) {
        let lossless_bits = 1 + xor.cost(xs);
        let best = candidate_exponents(eps, mode)
            .into_iter()
            .flat_map(|e| [false, true].map(|delta| plan_lossy(xs, eps, y_prev, e, delta, mode)))
            .min_by_key(|p| p.bits);
        match best {
            Some(p) if p.bits < lossless_bits => {
                p.write(&mut w);
                y_prev = p.y_last;
                xor.reset(y_prev);
            }
            _ => {
                w.write(0, 1);
                xor.reset(y_prev);
                for &x in xs {
                    xor.put(&mut w, x);
                }
                y_prev = *xs.last().unwrap();
            }
        }
    }
    let bits = w.bit_len();
    Encoded {
        bytes: w.into_bytes(),
        bits,
        n: vals.len(),
    }
}

pub fn decode(bytes: &[u8]) -> Vec<f64> {
    let mut r = BitReader::new(bytes);
    // Mode bit: the decoder is mode-agnostic, only the encoder differs.
    r.read(1).expect("truncated header");
    let n = r.read(32).expect("truncated header") as usize;
    let mut out = Vec::with_capacity(n);
    let mut y_prev = 0.0f64;
    let mut xor = XorState::default();
    while out.len() < n {
        let len = BLOCK.min(n - out.len());
        if r.read(1).unwrap() == 0 {
            xor.reset(y_prev);
            for _ in 0..len {
                out.push(xor.get(&mut r));
            }
        } else {
            let e = r.read(8).unwrap() as i32 - 128;
            let q = 2f64.powi(e);
            let delta = r.read(1).unwrap() == 1;
            let width = r.read(7).unwrap() as u32;
            let esc = (1u64 << width) - 1;
            let mut k_prev = 0i64;
            for _ in 0..len {
                let code = r.read(width).unwrap();
                let y = if code == esc {
                    k_prev = 0;
                    f64::from_bits(r.read(64).unwrap())
                } else {
                    let d = unzigzag(code);
                    let k = if delta { k_prev + d } else { d };
                    k_prev = k;
                    y_prev + k as f64 * q
                };
                out.push(y);
                y_prev = y;
            }
        }
        y_prev = *out.last().unwrap();
    }
    out
}

/// Candidate grid exponents: for each budget, the largest `e` with a
/// provably admissible step (`q <= 2 eps` for rounding, `q <= eps` for the
/// counter floor quantizer).
fn candidate_exponents(eps: &[f64], mode: Mode) -> Vec<i32> {
    let scale = if mode == Mode::Gauge { 2.0 } else { 1.0 };
    let mut es: Vec<i32> = eps
        .iter()
        .filter(|&&e| e > 0.0)
        .map(|&e| {
            let s = e * scale;
            if s.is_finite() {
                (s.log2().floor() as i32).clamp(E_MIN, E_MAX)
            } else {
                E_MAX
            }
        })
        .collect();
    es.sort_unstable();
    es.dedup();
    es
}

enum Code {
    Int(u64),
    Escape(f64),
}

struct LossyPlan {
    e: i32,
    delta: bool,
    width: u32,
    codes: Vec<Code>,
    bits: usize,
    y_last: f64,
}

impl LossyPlan {
    fn write(&self, w: &mut BitWriter) {
        w.write(1, 1);
        w.write((self.e + 128) as u64, 8);
        w.write(self.delta as u64, 1);
        w.write(self.width as u64, 7);
        let esc = (1u64 << self.width) - 1;
        for c in &self.codes {
            match c {
                Code::Int(z) => w.write(*z, self.width),
                Code::Escape(x) => {
                    w.write(esc, self.width);
                    w.write(x.to_bits(), 64);
                }
            }
        }
    }
}

fn plan_lossy(
    xs: &[f64],
    eps: &[f64],
    mut y_prev: f64,
    e: i32,
    delta: bool,
    mode: Mode,
) -> LossyPlan {
    let q = 2f64.powi(e);
    let mut codes = Vec::with_capacity(xs.len());
    let mut k_prev = 0i64;
    let mut max_z = 0u64;
    let mut escapes = 0usize;
    for (&x, &eb) in xs.iter().zip(eps) {
        match quantize(x, eb, y_prev, q, mode) {
            Some((k, y)) => {
                let z = zigzag(if delta { k - k_prev } else { k });
                max_z = max_z.max(z);
                codes.push(Code::Int(z));
                k_prev = k;
                y_prev = y;
            }
            None => {
                codes.push(Code::Escape(x));
                escapes += 1;
                k_prev = 0;
                y_prev = x;
            }
        }
    }
    let width = 64 - (max_z + 1).leading_zeros();
    let bits = 17 + xs.len() * width as usize + escapes * 64;
    LossyPlan {
        e,
        delta,
        width,
        codes,
        bits,
        y_last: y_prev,
    }
}

/// One closed-loop quantization step; `None` means the sample must escape.
fn quantize(x: f64, eps: f64, y_prev: f64, q: f64, mode: Mode) -> Option<(i64, f64)> {
    if eps.is_nan() || eps <= 0.0 || !x.is_finite() || !y_prev.is_finite() {
        return None;
    }
    let r = (x - y_prev) / q;
    let k = match mode {
        Mode::Gauge => r.round(),
        Mode::Counter => r.floor(),
    };
    if k.is_nan() || k.abs() > K_MAX || (mode == Mode::Counter && k < 0.0) {
        return None;
    }
    let y = y_prev + k * q;
    let ok = match mode {
        Mode::Gauge => (y - x).abs() <= eps,
        Mode::Counter => y <= x && x - y <= eps && y >= y_prev,
    };
    ok.then_some((k as i64, y))
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(z: u64) -> i64 {
    ((z >> 1) as i64) ^ -((z & 1) as i64)
}

/// Gorilla XOR coding (Pelkonen et al., VLDB 2015) of the value stream.
#[derive(Clone, Default)]
struct XorState {
    prev: u64,
    /// Current (leading, trailing) zero window, if one was emitted.
    window: Option<(u32, u32)>,
}

impl XorState {
    fn reset(&mut self, prev: f64) {
        self.prev = prev.to_bits();
        self.window = None;
    }

    fn put(&mut self, w: &mut BitWriter, x: f64) {
        let v = x.to_bits();
        let xor = v ^ self.prev;
        self.prev = v;
        if xor == 0 {
            w.write(0, 1);
            return;
        }
        w.write(1, 1);
        let lead = xor.leading_zeros().min(31);
        let trail = xor.trailing_zeros();
        match self.window {
            Some((l, t)) if lead >= l && trail >= t => {
                w.write(0, 1);
                w.write(xor >> t, 64 - l - t);
            }
            _ => {
                let len = 64 - lead - trail;
                w.write(1, 1);
                w.write(lead as u64, 5);
                w.write((len - 1) as u64, 6);
                w.write(xor >> trail, len);
                self.window = Some((lead, trail));
            }
        }
    }

    fn get(&mut self, r: &mut BitReader) -> f64 {
        if r.read(1).unwrap() == 1 {
            let xor = if r.read(1).unwrap() == 0 {
                let (l, t) = self.window.expect("XOR window reused before being set");
                r.read(64 - l - t).unwrap() << t
            } else {
                let lead = r.read(5).unwrap() as u32;
                let len = r.read(6).unwrap() as u32 + 1;
                let trail = 64 - lead - len;
                self.window = Some((lead, trail));
                r.read(len).unwrap() << trail
            };
            self.prev ^= xor;
        }
        f64::from_bits(self.prev)
    }

    /// Bits `put` would emit for `xs` starting from a reset state.
    fn cost(&self, xs: &[f64]) -> usize {
        let mut s = self.clone();
        s.window = None;
        let mut w = BitWriter::new();
        for &x in xs {
            s.put(&mut w, x);
        }
        w.bit_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| 0.5 + 0.3 * (i as f64 * 0.01).sin() + 1e-3 * ((i * 7919) % 101) as f64)
            .collect()
    }

    #[test]
    fn lossless_roundtrip_is_exact() {
        let xs = wave(1000);
        let enc = encode(&xs, &vec![0.0; xs.len()], Mode::Gauge);
        let ys = decode(&enc.bytes);
        assert!(xs.iter().zip(&ys).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn lossy_respects_budget_and_compresses() {
        let xs = wave(1000);
        let eps: Vec<f64> = (0..xs.len())
            .map(|i| if i % 97 == 0 { 0.0 } else { 1e-2 })
            .collect();
        let enc = encode(&xs, &eps, Mode::Gauge);
        let ys = decode(&enc.bytes);
        for i in 0..xs.len() {
            assert!((xs[i] - ys[i]).abs() <= eps[i], "sample {i}");
        }
        let lossless = encode(&xs, &vec![0.0; xs.len()], Mode::Gauge);
        assert!(
            enc.bits * 3 < lossless.bits,
            "{} vs {}",
            enc.bits,
            lossless.bits
        );
    }

    #[test]
    fn counter_mode_is_monotone_and_below() {
        let mut xs = Vec::new();
        let mut c = 1e6;
        for i in 0..2000 {
            c += (i % 13) as f64 * 3.7;
            xs.push(c);
        }
        let enc = encode(&xs, &vec![50.0; xs.len()], Mode::Counter);
        let ys = decode(&enc.bytes);
        assert!(ys.windows(2).all(|w| w[0] <= w[1]));
        assert!(xs.iter().zip(&ys).all(|(x, y)| y <= x && x - y <= 50.0));
    }

    #[test]
    fn unconstrained_and_special_values() {
        let xs = vec![1.0, f64::NAN, f64::INFINITY, -3.5, 1e300, 0.0];
        let eps = vec![f64::INFINITY, 1.0, 1.0, 0.0, 1.0, 1e-300];
        let ys = decode(&encode(&xs, &eps, Mode::Gauge).bytes);
        assert!(ys[1].is_nan() && ys[2] == f64::INFINITY && ys[3] == -3.5);
        assert!((ys[4] - 1e300).abs() <= 1.0 && (ys[5] - 0.0).abs() <= 1e-300);
    }
}
