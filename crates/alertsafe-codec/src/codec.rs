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
use crate::rc::{CostSink, Models, RcDecoder, RcEncoder, Sink, Source};

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
    /// Last reconstructed value (`0.0` for an empty stream): the `floor` of
    /// the next chunk in [`Mode::Counter`].
    pub last: f64,
}

impl Encoded {
    pub fn bits_per_value(&self) -> f64 {
        self.bits as f64 / self.n.max(1) as f64
    }
}

/// Residual coding of the stream. Both formats share block planning (grid
/// exponent, delta flag, escapes) and Gorilla XOR for lossless blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coding {
    /// Fixed-width zigzag codes per lossy block: at least one bit per value,
    /// branch-free decode. Kept as the ablation baseline.
    Fixed,
    /// Adaptive binary range coding of every decision, see [`crate::rc`].
    Entropy,
}

/// Encodes `vals` so that the decoded value `y_i` satisfies
/// `|y_i - vals[i]| <= budget[i]`. A budget of `0` (or a non-finite value)
/// forces an exact sample; `f64::INFINITY` leaves the sample unconstrained.
pub fn encode(vals: &[f64], budget: &[f64], mode: Mode) -> Encoded {
    encode_with(vals, budget, mode, Coding::Entropy)
}

pub fn encode_with(vals: &[f64], budget: &[f64], mode: Mode, coding: Coding) -> Encoded {
    encode_chunk(vals, budget, mode, coding, f64::NEG_INFINITY)
}

/// [`encode_with`] for a chunk that continues a series. In [`Mode::Counter`]
/// the reconstruction is additionally kept `>= floor`, the last
/// reconstructed value of the previous chunk, so the concatenated output is
/// non-decreasing across chunk boundaries too; a sample below `floor` (a
/// counter reset) is stored exactly. `floor` only constrains the encoder.
pub fn encode_chunk(
    vals: &[f64],
    budget: &[f64],
    mode: Mode,
    coding: Coding,
    floor: f64,
) -> Encoded {
    assert_eq!(vals.len(), budget.len());
    assert!(vals.len() <= u32::MAX as usize);
    match coding {
        Coding::Fixed => encode_fixed(vals, budget, mode, floor),
        Coding::Entropy => encode_entropy(vals, budget, mode, floor),
    }
}

/// The first bit of a stream selects the format: `0` fixed, `1` entropy.
pub fn decode(bytes: &[u8]) -> Vec<f64> {
    match bytes.first() {
        Some(b) if b & 0x80 != 0 => decode_entropy(bytes),
        _ => decode_fixed(bytes),
    }
}

fn encode_fixed(vals: &[f64], budget: &[f64], mode: Mode, mut floor: f64) -> Encoded {
    let mut w = BitWriter::new();
    w.write(0, 1);
    w.write((mode == Mode::Counter) as u64, 1);
    w.write(vals.len() as u64, 32);
    let mut y_prev = 0.0f64;
    let mut xor = XorState::default();
    for (xs, eps) in vals.chunks(BLOCK).zip(budget.chunks(BLOCK)) {
        let lossless_bits = 1 + xor.cost(xs);
        let best = candidate_exponents(eps, mode)
            .into_iter()
            .flat_map(|e| {
                [false, true].map(|delta| plan_lossy(xs, eps, y_prev, floor, e, delta, mode))
            })
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
        floor = y_prev;
    }
    let bits = w.bit_len();
    Encoded {
        bytes: w.into_bytes(),
        bits,
        n: vals.len(),
        last: y_prev,
    }
}

fn decode_fixed(bytes: &[u8]) -> Vec<f64> {
    let mut r = BitReader::new(bytes);
    r.read(1).expect("truncated header");
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

// Contexts of the entropy format.
const CTX_LOSSY: usize = 0;
const CTX_SAME_E: usize = 1;
const CTX_DELTA: usize = 2;
const CTX_XOR_SAME: usize = 3;
const CTX_XOR_NEW_WINDOW: usize = 4;
/// Escape flag, by whether the previous sample escaped.
const CTX_ESC: usize = 5;
/// Residual contexts are split by `delta * 4 + class` (see [`class`]).
const CTX_ZERO: usize = CTX_ESC + 2;
const CTX_SIGN: usize = CTX_ZERO + 8;
/// Top mantissa bit, by bit length.
const CTX_MANT: usize = CTX_SIGN + 8;
/// Unary bit length: 64 positions per residual context.
const CTX_LEN: usize = CTX_MANT + 65;
const N_CTX: usize = CTX_LEN + 8 * 64;

/// Magnitude class of the previous residual: 0, 1, 2–3, or larger/escape.
fn class(d: i64) -> usize {
    (64 - d.unsigned_abs().leading_zeros()).min(3) as usize
}

fn put_residual<S: Sink>(s: &mut S, d: i64, ctx: usize) {
    s.bit(CTX_ZERO + ctx, d != 0);
    if d == 0 {
        return;
    }
    s.bit(CTX_SIGN + ctx, d < 0);
    let m = d.unsigned_abs();
    let nb = 64 - m.leading_zeros();
    let len = CTX_LEN + ctx * 64;
    for j in 1..nb {
        s.bit(len + j as usize, true);
    }
    if nb < 64 {
        s.bit(len + nb as usize, false);
    }
    if nb >= 2 {
        s.bit(CTX_MANT + nb as usize, (m >> (nb - 2)) & 1 == 1);
        s.raw(m, nb - 2);
    }
}

fn get_residual<S: Source>(s: &mut S, ctx: usize) -> i64 {
    if !s.bit(CTX_ZERO + ctx) {
        return 0;
    }
    let neg = s.bit(CTX_SIGN + ctx);
    let len = CTX_LEN + ctx * 64;
    let mut nb = 1u32;
    while nb < 64 && s.bit(len + nb as usize) {
        nb += 1;
    }
    let mut m = 1u64;
    if nb >= 2 {
        m = (m << 1) | s.bit(CTX_MANT + nb as usize) as u64;
        m = (m << (nb - 2)) | s.raw(nb - 2);
    }
    let d = m as i64;
    if neg {
        -d
    } else {
        d
    }
}

fn put_lossy_entropy<S: Sink>(s: &mut S, p: &LossyPlan, e_prev: i32) {
    s.bit(CTX_LOSSY, true);
    s.bit(CTX_SAME_E, p.e == e_prev);
    if p.e != e_prev {
        s.raw((p.e + 128) as u64, 8);
    }
    s.bit(CTX_DELTA, p.delta);
    let mut cls = 0;
    let mut prev_esc = false;
    for c in &p.codes {
        match c {
            Code::Int(z) => {
                s.bit(CTX_ESC + prev_esc as usize, false);
                let d = unzigzag(*z);
                put_residual(s, d, p.delta as usize * 4 + cls);
                cls = class(d);
                prev_esc = false;
            }
            Code::Escape(x) => {
                s.bit(CTX_ESC + prev_esc as usize, true);
                s.raw(x.to_bits(), 64);
                cls = 3;
                prev_esc = true;
            }
        }
    }
}

fn encode_entropy(vals: &[f64], budget: &[f64], mode: Mode, mut floor: f64) -> Encoded {
    let mut out = vec![0x80 | (mode == Mode::Counter) as u8];
    put_varint(&mut out, vals.len() as u64);
    let mut rc = RcEncoder::new(Models::new(N_CTX), out);
    let mut y_prev = 0.0f64;
    let mut e_prev = 0i32;
    let mut xor = XorState::default();
    for (xs, eps) in vals.chunks(BLOCK).zip(budget.chunks(BLOCK)) {
        // Plans are compared by their exact adaptive cost from the current
        // model state, so the choice accounts for what the models learned.
        let mut lossless = CostSink::new(rc.models().clone());
        lossless.bit(CTX_LOSSY, false);
        let mut x = xor.clone();
        x.reset(y_prev);
        for &v in xs {
            x.put(&mut lossless, v);
        }
        let mut best: Option<(f64, LossyPlan)> = None;
        for e in candidate_exponents(eps, mode) {
            for delta in [false, true] {
                let p = plan_lossy(xs, eps, y_prev, floor, e, delta, mode);
                let mut c = CostSink::new(rc.models().clone());
                put_lossy_entropy(&mut c, &p, e_prev);
                if best.as_ref().is_none_or(|(b, _)| c.bits < *b) {
                    best = Some((c.bits, p));
                }
            }
        }
        match best {
            Some((bits, p)) if bits < lossless.bits => {
                put_lossy_entropy(&mut rc, &p, e_prev);
                e_prev = p.e;
                y_prev = p.y_last;
            }
            _ => {
                rc.bit(CTX_LOSSY, false);
                xor.reset(y_prev);
                for &v in xs {
                    xor.put(&mut rc, v);
                }
                y_prev = *xs.last().unwrap();
            }
        }
        floor = y_prev;
    }
    let bytes = rc.finish();
    Encoded {
        bits: bytes.len() * 8,
        bytes,
        n: vals.len(),
        last: y_prev,
    }
}

fn decode_entropy(bytes: &[u8]) -> Vec<f64> {
    let mut pos = 1;
    let n = get_varint(bytes, &mut pos) as usize;
    let mut r = RcDecoder::new(Models::new(N_CTX), &bytes[pos..]);
    let mut out = Vec::with_capacity(n);
    let mut y_prev = 0.0f64;
    let mut e_prev = 0i32;
    let mut xor = XorState::default();
    while out.len() < n {
        let len = BLOCK.min(n - out.len());
        if !r.bit(CTX_LOSSY) {
            xor.reset(y_prev);
            for _ in 0..len {
                out.push(xor.get(&mut r));
            }
        } else {
            let e = if r.bit(CTX_SAME_E) {
                e_prev
            } else {
                r.raw(8) as i32 - 128
            };
            e_prev = e;
            let q = 2f64.powi(e);
            let delta = r.bit(CTX_DELTA);
            let mut k_prev = 0i64;
            let mut cls = 0;
            let mut prev_esc = false;
            for _ in 0..len {
                let y = if r.bit(CTX_ESC + prev_esc as usize) {
                    k_prev = 0;
                    cls = 3;
                    prev_esc = true;
                    f64::from_bits(r.raw(64))
                } else {
                    let d = get_residual(&mut r, delta as usize * 4 + cls);
                    cls = class(d);
                    prev_esc = false;
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

/// LEB128.
fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(bytes: &[u8], pos: &mut usize) -> u64 {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *bytes.get(*pos).expect("truncated header");
        *pos += 1;
        v |= ((b & 0x7F) as u64) << shift;
        if b < 0x80 {
            return v;
        }
    }
    panic!("varint overflow")
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
    mut floor: f64,
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
        match quantize(x, eb, y_prev, floor, q, mode) {
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
        floor = y_prev;
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
/// In [`Mode::Counter`] the output must not fall below `floor` (the previous
/// reconstruction, or the previous chunk's last one; NaN means none).
fn quantize(x: f64, eps: f64, y_prev: f64, floor: f64, q: f64, mode: Mode) -> Option<(i64, f64)> {
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
    // Reconstruct from the integer the decoder sees: `k` may be `-0.0`, and
    // `-0.0 + -0.0 * q` keeps the sign the decoder's `-0.0 + 0.0` drops,
    // which would desynchronize the XOR reference of a following block.
    let k = k as i64;
    let y = y_prev + k as f64 * q;
    let ok = match mode {
        Mode::Gauge => (y - x).abs() <= eps,
        Mode::Counter => y <= x && x - y <= eps && y >= y_prev && (floor.is_nan() || y >= floor),
    };
    ok.then_some((k, y))
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

    fn put<S: Sink>(&mut self, w: &mut S, x: f64) {
        let v = x.to_bits();
        let xor = v ^ self.prev;
        self.prev = v;
        if xor == 0 {
            w.bit(CTX_XOR_SAME, false);
            return;
        }
        w.bit(CTX_XOR_SAME, true);
        let lead = xor.leading_zeros().min(31);
        let trail = xor.trailing_zeros();
        match self.window {
            Some((l, t)) if lead >= l && trail >= t => {
                w.bit(CTX_XOR_NEW_WINDOW, false);
                w.raw(xor >> t, 64 - l - t);
            }
            _ => {
                let len = 64 - lead - trail;
                w.bit(CTX_XOR_NEW_WINDOW, true);
                w.raw(lead as u64, 5);
                w.raw((len - 1) as u64, 6);
                w.raw(xor >> trail, len);
                self.window = Some((lead, trail));
            }
        }
    }

    fn get<S: Source>(&mut self, r: &mut S) -> f64 {
        if r.bit(CTX_XOR_SAME) {
            let xor = if !r.bit(CTX_XOR_NEW_WINDOW) {
                let (l, t) = self.window.expect("XOR window reused before being set");
                r.raw(64 - l - t) << t
            } else {
                let lead = r.raw(5) as u32;
                let len = r.raw(6) as u32 + 1;
                let trail = 64 - lead - len;
                self.window = Some((lead, trail));
                r.raw(len) << trail
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
        for coding in [Coding::Fixed, Coding::Entropy] {
            let xs = wave(1000);
            let enc = encode_with(&xs, &vec![0.0; xs.len()], Mode::Gauge, coding);
            let ys = decode(&enc.bytes);
            assert!(xs.iter().zip(&ys).all(|(a, b)| a.to_bits() == b.to_bits()));
        }
    }

    #[test]
    fn lossy_respects_budget_and_compresses() {
        for coding in [Coding::Fixed, Coding::Entropy] {
            let xs = wave(1000);
            let eps: Vec<f64> = (0..xs.len())
                .map(|i| if i % 97 == 0 { 0.0 } else { 1e-2 })
                .collect();
            let enc = encode_with(&xs, &eps, Mode::Gauge, coding);
            let ys = decode(&enc.bytes);
            for i in 0..xs.len() {
                assert!((xs[i] - ys[i]).abs() <= eps[i], "sample {i}");
            }
            let lossless = encode_with(&xs, &vec![0.0; xs.len()], Mode::Gauge, coding);
            assert!(
                enc.bits * 3 < lossless.bits,
                "{} vs {}",
                enc.bits,
                lossless.bits
            );
        }
    }

    #[test]
    fn counter_mode_is_monotone_and_below() {
        for coding in [Coding::Fixed, Coding::Entropy] {
            let mut xs = Vec::new();
            let mut c = 1e6;
            for i in 0..2000 {
                c += (i % 13) as f64 * 3.7;
                xs.push(c);
            }
            let enc = encode_with(&xs, &vec![50.0; xs.len()], Mode::Counter, coding);
            let ys = decode(&enc.bytes);
            assert!(ys.windows(2).all(|w| w[0] <= w[1]));
            assert!(xs.iter().zip(&ys).all(|(x, y)| y <= x && x - y <= 50.0));
        }
    }

    #[test]
    fn unconstrained_and_special_values() {
        for coding in [Coding::Fixed, Coding::Entropy] {
            let xs = vec![1.0, f64::NAN, f64::INFINITY, -3.5, 1e300, 0.0];
            let eps = vec![f64::INFINITY, 1.0, 1.0, 0.0, 1.0, 1e-300];
            let ys = decode(&encode_with(&xs, &eps, Mode::Gauge, coding).bytes);
            assert!(ys[1].is_nan() && ys[2] == f64::INFINITY && ys[3] == -3.5);
            assert!((ys[4] - 1e300).abs() <= 1.0 && (ys[5] - 0.0).abs() <= 1e-300);
        }
    }
}
