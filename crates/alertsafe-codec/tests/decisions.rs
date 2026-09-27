//! Property tests for the central invariant: rule decisions on the
//! reconstruction equal rule decisions on the original.

use alertsafe_codec::{alert_diff, budget, compress, decode, encode, flipped};
use alertsafe_rules::{grid, Agg, Cmp, Rule, Series};
use proptest::prelude::*;

fn timestamps(n: usize) -> impl Strategy<Value = Vec<i64>> {
    (0i64..100_000, prop::collection::vec(1i64..30_000, n)).prop_map(|(start, gaps)| {
        gaps.iter()
            .scan(start, |t, g| {
                *t += g;
                Some(*t)
            })
            .collect()
    })
}

fn gauge_series() -> impl Strategy<Value = Series> {
    (5usize..200).prop_flat_map(|n| {
        (
            timestamps(n),
            prop::collection::vec(-1.0f64..1.0, n),
            -3i32..7,
            any::<bool>(),
        )
            .prop_map(|(ts, steps, exp, coarse)| {
                let scale = 10f64.powi(exp);
                let mut v = 0.0;
                let vals = steps
                    .iter()
                    .map(|s| {
                        v += s * scale;
                        // Coarse values produce exact ties with thresholds.
                        if coarse {
                            (v / scale).round() * scale
                        } else {
                            v
                        }
                    })
                    .collect();
                Series::new(ts, vals)
            })
    })
}

fn counter_series() -> impl Strategy<Value = Series> {
    (5usize..200).prop_flat_map(|n| {
        (
            timestamps(n),
            prop::collection::vec((0.0f64..50.0, 0u8..40), n),
            0.0f64..1e6,
        )
            .prop_map(|(ts, incs, start)| {
                let mut c = start;
                let vals = incs
                    .iter()
                    .map(|&(inc, r)| {
                        c = if r == 0 { inc } else { c + inc.floor() };
                        c
                    })
                    .collect();
                Series::new(ts, vals)
            })
    })
}

fn cmp() -> impl Strategy<Value = Cmp> {
    prop_oneof![Just(Cmp::Gt), Just(Cmp::Ge), Just(Cmp::Lt), Just(Cmp::Le)]
}

/// A rule whose threshold is taken from the data, so crossings are common.
fn rule_for(series: &Series, agg: Agg) -> impl Strategy<Value = Rule> {
    let vals = series.vals.clone();
    let ts = series.ts.clone();
    (
        15_000i64..600_000,
        cmp(),
        0usize..vals.len(),
        -1.0f64..1.0,
        0i64..300_000,
        0i64..120_000,
    )
        .prop_map(move |(range, cmp, i, jitter, hold, keep)| {
            let mut probe = Rule::new("p", agg, range, cmp, 0.0);
            let base = probe.eval(&ts, &vals, ts[i]).unwrap_or(vals[i]);
            probe.threshold = if jitter.abs() < 0.2 {
                base
            } else {
                base * (1.0 + 0.1 * jitter)
            };
            probe.hold(hold).keep_firing(keep)
        })
}

fn eval_grid(series: &Series, step: i64, offset: i64) -> Vec<i64> {
    grid(
        series.ts[0] + offset,
        series.ts[series.len() - 1] + 600_000,
        step,
    )
}

fn gauge_case() -> impl Strategy<Value = (Series, Rule, i64, i64)> {
    gauge_series().prop_flat_map(|s| {
        let agg = prop_oneof![
            Just(Agg::Last),
            Just(Agg::AvgOverTime),
            Just(Agg::SumOverTime),
            Just(Agg::MinOverTime),
            Just(Agg::MaxOverTime)
        ];
        let s2 = s.clone();
        (
            Just(s),
            agg.prop_flat_map(move |a| rule_for(&s2, a)),
            5_000i64..60_000,
            0i64..60_000,
        )
    })
}

fn counter_case() -> impl Strategy<Value = (Series, Rule, i64, i64)> {
    counter_series().prop_flat_map(|s| {
        let s2 = s.clone();
        (
            Just(s),
            rule_for(&s2, Agg::Rate),
            5_000i64..60_000,
            0i64..60_000,
        )
    })
}

fn check_analytic(
    series: &Series,
    rule: &Rule,
    step: i64,
    offset: i64,
) -> Result<(), TestCaseError> {
    let g = eval_grid(series, step, offset);
    let rules = [rule.clone()];
    let plan = budget::plan(series, &rules, &g);
    let enc = encode(&series.vals, &plan.budget, plan.mode);
    let recon = series.with_values(decode(&enc.bytes));
    for i in 0..series.len() {
        let err = (recon.vals[i] - series.vals[i]).abs();
        prop_assert!(
            err <= plan.budget[i],
            "sample {i}: err {err} > budget {}",
            plan.budget[i]
        );
    }
    // Theorem 1: no repair needed, conditions agree at every protected instant.
    let flips = flipped(series, &recon, &rules, &g);
    prop_assert!(
        flips.is_empty(),
        "flipped at {:?}",
        flips.iter().map(|f| f.1).collect::<Vec<_>>()
    );
    // Corollary: identical state trajectories.
    prop_assert_eq!(alert_diff(series, &recon, &rules, &g).state_mismatches, 0);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn analytic_budget_preserves_gauge_decisions((s, r, step, off) in gauge_case()) {
        check_analytic(&s, &r, step, off)?;
    }

    #[test]
    fn analytic_budget_preserves_rate_decisions((s, r, step, off) in counter_case()) {
        check_analytic(&s, &r, step, off)?;
    }

    #[test]
    fn compress_preserves_trajectories_for_rule_sets(
        (s, r1, step, off) in gauge_case(),
        extra in prop_oneof![Just(Agg::AvgOverTime), Just(Agg::MaxOverTime)],
    ) {
        let g = eval_grid(&s, step, off);
        let r2 = Rule::new("extra", extra, r1.range_ms * 2, r1.cmp, r1.threshold * 0.9).hold(r1.for_ms);
        let rules = [r1, r2];
        let c = compress(&s, &rules, &g);
        let recon = s.with_values(decode(&c.encoded.bytes));
        prop_assert_eq!(alert_diff(&s, &recon, &rules, &g), Default::default());
    }
}
