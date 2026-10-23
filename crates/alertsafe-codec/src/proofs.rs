//! Kani harnesses (`cargo kani -p alertsafe-codec`): bit-precise checks of the
//! floating-point side of the codec, which `docs/formal-model.md` argues in
//! exact arithmetic only.
//!
//! - Theorem 1 in `f64`: for every finite window of up to `N` samples, every
//!   threshold and comparison, any reconstruction within the per-instant
//!   tolerance [`tolerance`] (as the encoder checks it, `|y - x| <= tau`
//!   rounded) keeps the condition of `last`, `min/max/avg/sum_over_time`.
//!   This covers the rounding slack in `lipschitz_tolerance`. For
//!   `min/max_over_time` the monolithic statement is out of reach of the
//!   SAT solver, so it is proven in three parts (see [`scalar_tolerance`]).
//! - The quantizer's output is what the decoder reconstructs, bit for bit,
//!   and respects the budget (and, for counters, the floor quantizer's
//!   invariants of Proposition 1).
//! - The integer and header encodings are bijective.

use alertsafe_rules::{Agg, Cmp, Rule};

use crate::bits::{BitReader, BitWriter};
use crate::budget::{lipschitz_tolerance, tolerance};
use crate::codec::{get_varint, put_varint, quantize, unzigzag, zigzag, Mode};
use crate::rc::{Models, RcDecoder, RcEncoder, Sink, Source};

/// One evaluation at the last of `N` samples 1 s apart, window covering all.
fn check_condition_preserved<const N: usize>(agg: Agg, cmp: Cmp) {
    let x: [f64; N] = kani::any();
    let y: [f64; N] = kani::any();
    let th: f64 = kani::any();
    kani::assume(th.is_finite());
    for i in 0..N {
        kani::assume(x[i].is_finite() && y[i].is_finite());
    }
    let ts: [i64; N] = core::array::from_fn(|i| i as i64 * 1000);
    let t = ts[N - 1];
    let rule = Rule::new("r", agg, N as i64 * 1000, cmp, th);
    let (support, tau) = tolerance(&rule, &ts, &x, t).unwrap();
    assert_eq!(support, 0..N);
    for i in 0..N {
        kani::assume((y[i] - x[i]).abs() <= tau);
    }
    assert_eq!(rule.condition(&ts, &y, t), rule.condition(&ts, &x, t));
}

/// One harness per comparison: each is a separate (and much smaller)
/// floating-point SAT problem than a symbolic `Cmp`.
macro_rules! theorem1 {
    ($agg:ident, $n:literal, $unwind:literal: $($name:ident => $cmp:ident),+) => {$(
        #[kani::proof]
        #[kani::unwind($unwind)]
        fn $name() {
            check_condition_preserved::<$n>(Agg::$agg, Cmp::$cmp);
        }
    )+};
}

theorem1!(Last, 1, 2:
    theorem1_last_gt => Gt, theorem1_last_ge => Ge,
    theorem1_last_lt => Lt, theorem1_last_le => Le);
theorem1!(AvgOverTime, 2, 3:
    theorem1_avg_over_time_gt => Gt, theorem1_avg_over_time_ge => Ge,
    theorem1_avg_over_time_lt => Lt, theorem1_avg_over_time_le => Le);
theorem1!(SumOverTime, 2, 3:
    theorem1_sum_over_time_gt => Gt, theorem1_sum_over_time_ge => Ge,
    theorem1_sum_over_time_lt => Lt, theorem1_sum_over_time_le => Le);

// Theorem 1 for `last`, `min/max_over_time`, compositionally. The aggregate
// is 1-Lipschitz in the sup norm and evaluated exactly, so the statement
// factors through its value `v = f(x)`:
// (A) `scalar_tolerance`: any `y'` with `fl|y' - v| <= tau(v)` is on the
//     side of `th` that `v` is on, for `tau(v) = lipschitz_tolerance(v)`;
// (B) `extremum_lipschitz`: `fl|y_i - x_i| <= tau` for all `i` implies
//     `fl|f(y) - f(x)| <= tau`, for an arbitrary `tau` (no threshold, no
//     `next_down` chain: the expensive arithmetic of (A) is not repeated);
// (C) `tolerance_of_extremum`: `tolerance` returns the full support and
//     exactly `lipschitz_tolerance(f(x))` (its `abs_sum` is ignored).
// Then `c(y) = holds(f(y), th) = holds(f(x), th) = c(x)` by (C), (B), (A)
// with `y' = f(y)`, which is finite as the max/min of finite values.

/// The three aggregates share `lipschitz_tolerance`'s `(1, 0)` branch.
fn any_extremum_agg() -> Agg {
    match kani::any::<u8>() % 3 {
        0 => Agg::Last,
        1 => Agg::MinOverTime,
        _ => Agg::MaxOverTime,
    }
}

/// (A), for every window length `n`.
fn scalar_tolerance(cmp: Cmp) {
    let (v, y, th): (f64, f64, f64) = kani::any();
    kani::assume(v.is_finite() && y.is_finite() && th.is_finite());
    let n: usize = kani::any();
    kani::assume(n >= 1);
    let tau = lipschitz_tolerance(any_extremum_agg(), th, n, 0.0, v);
    kani::assume((y - v).abs() <= tau);
    assert_eq!(cmp.holds(y, th), cmp.holds(v, th));
}

#[kani::proof]
fn scalar_tolerance_gt() {
    scalar_tolerance(Cmp::Gt);
}

#[kani::proof]
fn scalar_tolerance_ge() {
    scalar_tolerance(Cmp::Ge);
}

#[kani::proof]
fn scalar_tolerance_lt() {
    scalar_tolerance(Cmp::Lt);
}

#[kani::proof]
fn scalar_tolerance_le() {
    scalar_tolerance(Cmp::Le);
}

/// The rule of [`check_condition_preserved`]: `N` samples 1 s apart, one
/// evaluation at the last, window covering all.
fn window_rule<const N: usize>(agg: Agg, cmp: Cmp, th: f64) -> (Rule, [i64; N], i64) {
    let ts: [i64; N] = core::array::from_fn(|i| i as i64 * 1000);
    (Rule::new("r", agg, N as i64 * 1000, cmp, th), ts, ts[N - 1])
}

/// (B). The threshold and comparison do not enter `eval`.
fn extremum_lipschitz<const N: usize>(agg: Agg) {
    let x: [f64; N] = kani::any();
    let y: [f64; N] = kani::any();
    let tau: f64 = kani::any();
    for i in 0..N {
        kani::assume(x[i].is_finite() && y[i].is_finite());
        kani::assume((y[i] - x[i]).abs() <= tau);
    }
    let (rule, ts, t) = window_rule::<N>(agg, Cmp::Gt, 0.0);
    let vx = rule.eval(&ts, &x, t).unwrap();
    let vy = rule.eval(&ts, &y, t).unwrap();
    assert!(vx.is_finite() && vy.is_finite());
    assert!((vy - vx).abs() <= tau);
}

#[kani::proof]
#[kani::unwind(2)]
fn extremum_lipschitz_last() {
    extremum_lipschitz::<1>(Agg::Last);
}

#[kani::proof]
#[kani::unwind(4)]
fn extremum_lipschitz_max_over_time() {
    extremum_lipschitz::<3>(Agg::MaxOverTime);
}

#[kani::proof]
#[kani::unwind(4)]
fn extremum_lipschitz_min_over_time() {
    extremum_lipschitz::<3>(Agg::MinOverTime);
}

// (B) in turn splits once more, since with `N = 3` it still times out in
// the final SAT query:
// (B1) `extremum_attained`: `max` is one of the samples, `x[k]`, and bounds
//      all of them (dually for `min`); comparisons only, no arithmetic;
// (B2) `extremum_pair`: with `y[j] = max y`, `x[k] = max x`, the error of
//      the maximum is at most the larger of the errors at `j` and `k`
//      (`y_j - x_k <= y_j - x_j` or `x_k - y_j <= x_k - y_k`, and rounding
//      is monotone). Then `fl|f(y) - f(x)| = fl|y_j - x_k| <= tau`.

/// (B1).
fn extremum_attained<const N: usize>(agg: Agg) {
    let x: [f64; N] = kani::any();
    for i in 0..N {
        kani::assume(x[i].is_finite());
    }
    let (rule, ts, t) = window_rule::<N>(agg, Cmp::Gt, 0.0);
    let v = rule.eval(&ts, &x, t).unwrap();
    assert!(x.iter().any(|a| a.to_bits() == v.to_bits()));
    for i in 0..N {
        match agg {
            Agg::MaxOverTime => assert!(x[i] <= v),
            _ => assert!(x[i] >= v),
        }
    }
}

#[kani::proof]
#[kani::unwind(4)]
fn extremum_attained_max_over_time() {
    extremum_attained::<3>(Agg::MaxOverTime);
}

#[kani::proof]
#[kani::unwind(4)]
fn extremum_attained_min_over_time() {
    extremum_attained::<3>(Agg::MinOverTime);
}

/// (B2): `y_j` and `x_k` are the extrema of `y` and `x`, hence
/// `y_j >= y_k` and `x_k >= x_j` for `max` (reversed for `min`).
fn extremum_pair(max: bool) {
    let (yj, yk, xj, xk): (f64, f64, f64, f64) = kani::any();
    kani::assume(yj.is_finite() && yk.is_finite() && xj.is_finite() && xk.is_finite());
    if max {
        kani::assume(yj >= yk && xk >= xj);
    } else {
        kani::assume(yj <= yk && xk <= xj);
    }
    assert!((yj - xk).abs() <= (yj - xj).abs().max((yk - xk).abs()));
}

#[kani::proof]
fn extremum_pair_max() {
    extremum_pair(true);
}

#[kani::proof]
fn extremum_pair_min() {
    extremum_pair(false);
}

/// The fact (B2) rests on: rounded subtraction is monotone in both
/// arguments (IEEE 754 rounding is monotone).
#[kani::proof]
fn sub_monotone() {
    let (a, a2, b, b2): (f64, f64, f64, f64) = kani::any();
    kani::assume(a.is_finite() && a2.is_finite() && b.is_finite() && b2.is_finite());
    kani::assume(a <= a2 && b2 <= b);
    assert!(a - b <= a2 - b2);
}

/// (C), bit for bit.
fn tolerance_of_extremum<const N: usize>(agg: Agg) {
    let x: [f64; N] = kani::any();
    let th: f64 = kani::any();
    kani::assume(th.is_finite());
    for i in 0..N {
        kani::assume(x[i].is_finite());
    }
    let (rule, ts, t) = window_rule::<N>(agg, Cmp::Gt, th);
    let (support, tau) = tolerance(&rule, &ts, &x, t).unwrap();
    assert_eq!(support, 0..N);
    let v = rule.eval(&ts, &x, t).unwrap();
    assert_eq!(
        tau.to_bits(),
        lipschitz_tolerance(agg, th, N, 0.0, v).to_bits()
    );
}

#[kani::proof]
#[kani::unwind(2)]
fn tolerance_of_extremum_last() {
    tolerance_of_extremum::<1>(Agg::Last);
}

#[kani::proof]
#[kani::unwind(4)]
fn tolerance_of_extremum_max_over_time() {
    tolerance_of_extremum::<3>(Agg::MaxOverTime);
}

#[kani::proof]
#[kani::unwind(4)]
fn tolerance_of_extremum_min_over_time() {
    tolerance_of_extremum::<3>(Agg::MinOverTime);
}

#[kani::proof]
fn quantize_matches_decoder_and_budget() {
    let (x, eps, y_prev, floor): (f64, f64, f64, f64) = kani::any();
    let e: i32 = kani::any();
    kani::assume((-60..=60).contains(&e));
    let q = 2f64.powi(e);
    let mode = if kani::any() {
        Mode::Gauge
    } else {
        Mode::Counter
    };
    if let Some((k, y)) = quantize(x, eps, y_prev, floor, q, mode) {
        // The decoder's reconstruction (`decode_*`), compared bitwise: a
        // differing sign of zero desynchronizes the XOR reference.
        assert_eq!((y_prev + k as f64 * q).to_bits(), y.to_bits());
        match mode {
            Mode::Gauge => {
                assert!((y - x).abs() <= eps);
            }
            Mode::Counter => {
                assert!(k >= 0 && y <= x && x - y <= eps && y >= y_prev);
                assert!(floor.is_nan() || y >= floor);
            }
        }
    }
}

#[kani::proof]
fn zigzag_roundtrip() {
    let v: i64 = kani::any();
    assert_eq!(unzigzag(zigzag(v)), v);
}

#[kani::proof]
#[kani::unwind(11)]
fn varint_roundtrip() {
    let v: u64 = kani::any();
    let mut out = Vec::new();
    put_varint(&mut out, v);
    let mut pos = 0;
    assert_eq!(get_varint(&out, &mut pos), v);
    assert_eq!(pos, out.len());
}

/// Encodes `items` (adaptive bits in either of two contexts, and raw
/// fields of the given widths), then decodes and compares.
fn check_range_coder<const K: usize>(widths: [u32; K]) {
    let items: [(bool, bool, u64); K] = kani::any();
    // Preallocated, so that no push takes the reallocation path.
    let mut enc = RcEncoder::new(Models::new(2), Vec::with_capacity(64));
    for (&(ctx, b, v), &n) in items.iter().zip(&widths) {
        enc.bit(ctx as usize, b);
        enc.raw(v, n);
    }
    let bytes = enc.finish();
    let mut dec = RcDecoder::new(Models::new(2), &bytes);
    for (&(ctx, b, v), &n) in items.iter().zip(&widths) {
        assert_eq!(dec.bit(ctx as usize), b);
        assert_eq!(dec.raw(n), v & ((1u64 << n) - 1));
    }
}

// `finish` scans 33 flush positions, hence the unwind bound. Raw widths are
// concrete: a symbolic width multiplies the paths through `normalize`.
#[kani::proof]
#[kani::unwind(34)]
fn range_coder_bit() {
    check_range_coder([0]);
}

#[kani::proof]
#[kani::unwind(34)]
fn range_coder_bits3() {
    check_range_coder([0, 0, 0]);
}

#[kani::proof]
#[kani::unwind(34)]
fn range_coder_raw8() {
    check_range_coder([8]);
}

#[kani::proof]
#[kani::unwind(34)]
fn range_coder_raw16() {
    check_range_coder([16]);
}

/// Two raw chunks (`RAW_CHUNK = 16`).
#[kani::proof]
#[kani::unwind(34)]
fn range_coder_raw20() {
    check_range_coder([20]);
}

#[kani::proof]
#[kani::unwind(34)]
fn range_coder_mixed() {
    check_range_coder([12, 3]);
}

// Bit I/O, compositionally: a single writer-to-reader harness over symbolic
// widths runs CBMC out of memory (values flow through a byte vector of
// symbolic length, read at symbolic indices). The specification is the
// stream as a `u128`, first bit most significant, zero past its end:
// - the writer emits exactly the leading `ceil(n / 8)` bytes of it,
// - the reader returns the fields of whatever bytes it is given,
// - truncating the specification to those bytes and cutting the fields
//   back out yields the written values.

fn mask(v: u64, n: u32) -> u64 {
    if n == 64 {
        v
    } else {
        v & ((1u64 << n) - 1)
    }
}

/// The stream of `v1` (`n1` bits) then `v2` (`n2` bits).
fn spec_stream(v1: u64, n1: u32, v2: u64, n2: u32) -> u128 {
    let place = |v: u64, off: u32, n: u32| {
        if n == 0 {
            0
        } else {
            (mask(v, n) as u128) << (128 - off - n)
        }
    };
    place(v1, 0, n1) | place(v2, n1, n2)
}

/// The `n` bits of `x` starting `off` bits from the top.
fn spec_field(x: u128, off: u32, n: u32) -> u64 {
    if n == 0 {
        0
    } else {
        ((x << off) >> (128 - n)) as u64
    }
}

/// `bytes` (at most 16) as the top of a `u128`, zero-padded.
fn spec_from_bytes(bytes: &[u8]) -> u128 {
    let mut b = [0u8; 16];
    for (d, s) in b.iter_mut().zip(bytes) {
        *d = *s;
    }
    u128::from_be_bytes(b)
}

#[kani::proof]
#[kani::unwind(17)]
fn bit_writer_matches_spec() {
    let (v1, v2): (u64, u64) = kani::any();
    let (n1, n2): (u32, u32) = kani::any();
    kani::assume(n1 <= 64 && n2 <= 64);
    let mut w = BitWriter::new();
    w.write(v1, n1);
    w.write(v2, n2);
    let bytes = w.into_bytes();
    let spec = spec_stream(v1, n1, v2, n2).to_be_bytes();
    assert_eq!(bytes.len(), (n1 + n2).div_ceil(8) as usize);
    for i in 0..bytes.len() {
        assert_eq!(bytes[i], spec[i]);
    }
}

#[kani::proof]
#[kani::unwind(17)]
fn bit_reader_matches_spec() {
    let buf: [u8; 16] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 16);
    let bytes = &buf[..len];
    let (n1, n2): (u32, u32) = kani::any();
    kani::assume(n1 <= 64 && n2 <= 64);
    kani::assume((n1 + n2) as usize <= bytes.len() * 8);
    let x = spec_from_bytes(bytes);
    let mut r = BitReader::new(bytes);
    assert_eq!(r.read(n1), Some(spec_field(x, 0, n1)));
    assert_eq!(r.read(n2), Some(spec_field(x, n1, n2)));
}

#[kani::proof]
#[kani::unwind(17)]
fn bit_spec_roundtrip() {
    let (v1, v2): (u64, u64) = kani::any();
    let (n1, n2): (u32, u32) = kani::any();
    kani::assume(n1 <= 64 && n2 <= 64);
    let spec = spec_stream(v1, n1, v2, n2);
    let len = (n1 + n2).div_ceil(8) as usize;
    let x = spec_from_bytes(&spec.to_be_bytes()[..len]);
    assert_eq!(spec_field(x, 0, n1), mask(v1, n1));
    assert_eq!(spec_field(x, n1, n2), mask(v2, n2));
}
