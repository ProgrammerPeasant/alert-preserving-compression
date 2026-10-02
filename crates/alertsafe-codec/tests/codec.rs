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
