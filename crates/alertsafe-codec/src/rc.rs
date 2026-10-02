//! Adaptive binary range coder (LZMA-style carry-less encoder with a cached
//! carry byte) and the bit-sink abstraction shared by both stream formats.
//!
//! Every adaptive decision is coded under a context `ctx`: an index into a
//! table of 15-bit probabilities that both sides update identically. Bits
//! with no useful statistics (float payloads) go through [`Sink::raw`], which
//! the range coder codes at exactly one bit per bit. [`BitWriter`] and
//! [`BitReader`] implement the same traits and ignore contexts, so code
//! written against [`Sink`]/[`Source`] serves the fixed-width format too.
//! [`CostSink`] runs the models without producing output and accumulates the
//! ideal code length, which the encoder uses to compare block plans.

use std::sync::OnceLock;

use crate::bits::{BitReader, BitWriter};

const PROB_BITS: u32 = 15;
const PROB_ONE: u32 = 1 << PROB_BITS;
const MOVE_BITS: u32 = 5;
const TOP: u32 = 1 << 24;
/// Bypass bits are coded in chunks of at most this many bits.
const RAW_CHUNK: u32 = 16;

pub trait Sink {
    /// Codes `b` under the adaptive context `ctx`.
    fn bit(&mut self, ctx: usize, b: bool);
    /// Codes the low `n` bits of `v` (`n <= 64`) without modelling.
    fn raw(&mut self, v: u64, n: u32);
}

pub trait Source {
    fn bit(&mut self, ctx: usize) -> bool;
    fn raw(&mut self, n: u32) -> u64;
}

impl Sink for BitWriter {
    fn bit(&mut self, _ctx: usize, b: bool) {
        self.write(b as u64, 1);
    }
    fn raw(&mut self, v: u64, n: u32) {
        self.write(v, n);
    }
}

impl Source for BitReader {
    fn bit(&mut self, _ctx: usize) -> bool {
        self.read(1).expect("truncated stream") == 1
    }
    fn raw(&mut self, n: u32) -> u64 {
        self.read(n).expect("truncated stream")
    }
}

/// Probabilities (of a zero bit) for every context.
#[derive(Clone)]
pub struct Models(Vec<u16>);

impl Models {
    pub fn new(n: usize) -> Self {
        Models(vec![(PROB_ONE / 2) as u16; n])
    }

    #[inline]
    fn update(p: &mut u16, b: bool) {
        if b {
            *p -= *p >> MOVE_BITS;
        } else {
            *p += ((PROB_ONE - *p as u32) >> MOVE_BITS) as u16;
        }
    }
}

pub struct RcEncoder {
    models: Models,
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    out: Vec<u8>,
}

impl RcEncoder {
    pub fn new(models: Models, out: Vec<u8>) -> Self {
        RcEncoder {
            models,
            low: 0,
            range: u32::MAX,
            cache: 0,
            cache_size: 1,
            out,
        }
    }

    pub fn models(&self) -> &Models {
        &self.models
    }

    pub fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        self.out
    }

    fn shift_low(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut temp = self.cache;
            loop {
                self.out.push(temp.wrapping_add(carry));
                temp = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.cache_size += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    #[inline]
    fn normalize(&mut self) {
        while self.range < TOP {
            self.range <<= 8;
            self.shift_low();
        }
    }
}

impl Sink for RcEncoder {
    #[inline]
    fn bit(&mut self, ctx: usize, b: bool) {
        let p = &mut self.models.0[ctx];
        let bound = (self.range >> PROB_BITS) * *p as u32;
        if b {
            self.low += bound as u64;
            self.range -= bound;
        } else {
            self.range = bound;
        }
        Models::update(p, b);
        self.normalize();
    }

    fn raw(&mut self, v: u64, n: u32) {
        let mut left = n;
        while left > 0 {
            let k = left.min(RAW_CHUNK);
            left -= k;
            let chunk = (v >> left) & ((1u64 << k) - 1);
            self.range >>= k;
            self.low += chunk * self.range as u64;
            self.normalize();
        }
    }
}

pub struct RcDecoder<'a> {
    models: Models,
    bytes: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
}

impl<'a> RcDecoder<'a> {
    pub fn new(models: Models, bytes: &'a [u8]) -> Self {
        let mut d = RcDecoder {
            models,
            bytes,
            pos: 0,
            range: u32::MAX,
            code: 0,
        };
        for _ in 0..5 {
            d.code = (d.code << 8) | d.next_byte() as u32;
        }
        d
    }

    #[inline]
    fn next_byte(&mut self) -> u8 {
        let b = self.bytes.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }

    #[inline]
    fn normalize(&mut self) {
        while self.range < TOP {
            self.range <<= 8;
            self.code = (self.code << 8) | self.next_byte() as u32;
        }
    }
}

impl Source for RcDecoder<'_> {
    #[inline]
    fn bit(&mut self, ctx: usize) -> bool {
        let p = &mut self.models.0[ctx];
        let bound = (self.range >> PROB_BITS) * *p as u32;
        let b = self.code >= bound;
        if b {
            self.code -= bound;
            self.range -= bound;
        } else {
            self.range = bound;
        }
        Models::update(p, b);
        self.normalize();
        b
    }

    fn raw(&mut self, n: u32) -> u64 {
        let mut v = 0u64;
        let mut left = n;
        while left > 0 {
            let k = left.min(RAW_CHUNK);
            left -= k;
            self.range >>= k;
            let q = (self.code / self.range).min((1 << k) - 1);
            self.code -= q * self.range;
            v = (v << k) | q as u64;
            self.normalize();
        }
        v
    }
}

/// Ideal code length of the decisions fed to it, in bits, under the same
/// adaptive models as the range coder (which stays within a few bytes of it).
#[derive(Clone)]
pub struct CostSink {
    models: Models,
    pub bits: f64,
}

impl CostSink {
    pub fn new(models: Models) -> Self {
        CostSink { models, bits: 0.0 }
    }
}

/// `-log2(p / PROB_ONE)` sampled at `p = (i + 0.5) * 8`.
fn cost_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..PROB_ONE >> 3)
            .map(|i| -(((i as f64 + 0.5) * 8.0) / PROB_ONE as f64).log2() as f32)
            .collect()
    })
}

impl Sink for CostSink {
    #[inline]
    fn bit(&mut self, ctx: usize, b: bool) {
        let p = &mut self.models.0[ctx];
        let p_b = if b { PROB_ONE - *p as u32 } else { *p as u32 };
        self.bits += cost_table()[(p_b >> 3) as usize] as f64;
        Models::update(p, b);
    }

    fn raw(&mut self, _v: u64, n: u32) {
        self.bits += n as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_mixed_bits_and_raw() {
        let mut items = Vec::new();
        let mut s = 0x1234_5678_9abc_def0u64;
        for i in 0..20_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            // Skewed adaptive bits on a few contexts, interleaved with raw runs.
            let ctx = (s % 7) as usize;
            items.push((ctx, (s >> 20) % 16 < ctx as u64, s, (i % 65) as u32));
        }
        let mut enc = RcEncoder::new(Models::new(8), Vec::new());
        let mut cost = CostSink::new(Models::new(8));
        for &(ctx, b, v, n) in &items {
            enc.bit(ctx, b);
            enc.raw(v, n);
            cost.bit(ctx, b);
            cost.raw(v, n);
        }
        let bytes = enc.finish();
        let mut dec = RcDecoder::new(Models::new(8), &bytes);
        for &(ctx, b, v, n) in &items {
            assert_eq!(dec.bit(ctx), b);
            let mask = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
            assert_eq!(dec.raw(n), v & mask);
        }
        let actual = bytes.len() as f64 * 8.0;
        assert!(
            (actual - cost.bits).abs() < 64.0 + 1e-3 * cost.bits,
            "{actual} vs {}",
            cost.bits
        );
    }

    #[test]
    fn skewed_bits_cost_far_below_one_bit() {
        let mut enc = RcEncoder::new(Models::new(1), Vec::new());
        for i in 0..100_000 {
            enc.bit(0, i % 1000 == 0);
        }
        let bytes = enc.finish();
        assert!(bytes.len() * 8 < 100_000 / 20, "{} bytes", bytes.len());
    }
}
