//! Round-trip properties of both stream formats on adversarial inputs:
//! mixed magnitudes, special values, zero and infinite budgets, block
//! boundaries.

use alertsafe_codec::{decode, encode_with, Coding, Mode};
use proptest::prelude::*;

fn value() -> impl Strategy<Value = f64> {
    prop_oneof![
        8 => -1e3f64..1e3,
        2 => any::<f64>(),
        1 => Just(f64::NAN),
        1 => Just(f64::INFINITY),
        1 => Just(0.0),
    ]
}

fn budget() -> impl Strategy<Value = f64> {
    prop_oneof![
        6 => (-12i32..4).prop_map(|e| 10f64.powi(e)),
        2 => Just(0.0),
        1 => Just(f64::INFINITY),
        1 => Just(f64::NAN),
    ]
}

fn check(xs: &[f64], eps: &[f64], mode: Mode) -> Result<(), TestCaseError> {
    for coding in [Coding::Fixed, Coding::Entropy] {
        let ys = decode(&encode_with(xs, eps, mode, coding).bytes);
        prop_assert_eq!(ys.len(), xs.len());
        for (i, ((&x, &y), &e)) in xs.iter().zip(&ys).zip(eps).enumerate() {
            if e.is_nan() || e <= 0.0 || !x.is_finite() {
                prop_assert_eq!(x.to_bits(), y.to_bits(), "{:?} sample {}", coding, i);
            } else {
                prop_assert!((x - y).abs() <= e, "{:?} sample {}", coding, i);
                if mode == Mode::Counter {
                    prop_assert!(y <= x, "{:?} sample {}", coding, i);
                }
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn gauge_roundtrip(pairs in prop::collection::vec((value(), budget()), 0..400)) {
        let (xs, eps): (Vec<f64>, Vec<f64>) = pairs.into_iter().unzip();
        check(&xs, &eps, Mode::Gauge)?;
    }

    #[test]
    fn counter_roundtrip(
        steps in prop::collection::vec((0.0f64..1e4, budget()), 0..400),
        start in 0.0f64..1e9,
    ) {
        let mut c = start;
        let (xs, eps): (Vec<f64>, Vec<f64>) = steps
            .into_iter()
            .map(|(s, e)| {
                c += s;
                (c, e)
            })
            .unzip();
        check(&xs, &eps, Mode::Counter)?;
    }
}

/// Fixed cost of a stream: a Prometheus chunk of 120 samples that the budget
/// leaves unconstrained must cost a few bytes, not a few bits per sample.
#[test]
fn short_stream_overhead() {
    let xs: Vec<f64> = (0..120).map(|i| 0.5 + 1e-3 * i as f64).collect();
    let enc = encode_with(&xs, &vec![f64::INFINITY; 120], Mode::Gauge, Coding::Entropy);
    assert!(enc.bytes.len() <= 6, "{} bytes", enc.bytes.len());
    assert_eq!(decode(&enc.bytes).len(), 120);
    let empty = encode_with(&[], &[], Mode::Gauge, Coding::Entropy);
    assert!(empty.bytes.len() <= 2, "{:?}", empty.bytes);
    assert!(decode(&empty.bytes).is_empty());
}

/// Regression (proptest-found): a lossy block ending on a zero step from
/// `-0.0` must leave encoder and decoder with the same reference value, bit
/// for bit, or the next XOR-coded block decodes with flipped signs.
#[test]
fn negative_zero_at_block_end() {
    let mut xs = vec![0.0; 135];
    let mut eps = vec![1.0; 135];
    eps[121] = f64::INFINITY;
    (xs[126], eps[126]) = (-0.0, 0.0);
    (xs[127], eps[127]) = (-641.8553619710056, f64::INFINITY);
    (xs[128], eps[128]) = (6948036.381076522, 1000.0);
    (xs[129], eps[129]) = (1.4343853832831938e45, f64::INFINITY);
    xs[131] = f64::NAN;
    for coding in [Coding::Fixed, Coding::Entropy] {
        let ys = decode(&encode_with(&xs, &eps, Mode::Gauge, coding).bytes);
        assert!(ys[131].is_nan());
        for i in (0..xs.len()).filter(|&i| i != 131) {
            assert!(
                (xs[i] - ys[i]).abs() <= eps[i],
                "{coding:?} sample {i}: {}",
                ys[i]
            );
        }
    }
}
