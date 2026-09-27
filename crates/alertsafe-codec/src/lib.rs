//! Alert-preserving lossy compression of monitoring metrics.
//!
//! [`compress`] derives a per-sample error budget from a rule set
//! ([`budget::plan`]), encodes the series with it ([`codec::encode`]), and
//! verifies the reconstruction against the reference rule semantics of
//! `alertsafe-rules`, zeroing the budget on the support of any evaluation
//! whose condition changed (verify-and-repair). The analytic budget is sound
//! in exact arithmetic (see `docs/formal-model.md`); the repair loop turns it
//! into an end-to-end guarantee against the reference evaluator, including
//! floating-point effects.

pub mod bits;
pub mod budget;
pub mod codec;

use alertsafe_rules::{Diff, Rule, Series};

pub use budget::{plan, Plan};
pub use codec::{decode, encode, Encoded, Mode};

#[derive(Clone, Debug)]
pub struct Compressed {
    pub encoded: Encoded,
    pub plan: Plan,
    /// Evaluations whose condition flipped under the analytic budget and were
    /// repaired. Zero whenever the analytic budget alone was sufficient.
    pub repairs: usize,
}

/// Rule-aware compression of `series`: every rule's condition is identical on
/// the original and the reconstruction at every protected instant (see
/// [`budget::plan`]), hence the alert state trajectories are identical.
pub fn compress(series: &Series, rules: &[Rule], grid: &[i64]) -> Compressed {
    let mut plan = budget::plan(series, rules, grid);
    let mut repairs = 0;
    loop {
        let encoded = encode(&series.vals, &plan.budget, plan.mode);
        let recon = series.with_values(decode(&encoded.bytes));
        let flips = flipped(series, &recon, rules, grid);
        if flips.is_empty() {
            return Compressed {
                encoded,
                plan,
                repairs,
            };
        }
        repairs += flips.len();
        for (rule, t) in flips {
            for b in &mut plan.budget[rule.support(&series.ts, t)] {
                *b = 0.0;
            }
        }
    }
}

/// Protected evaluations `(rule, t)` whose condition differs between the
/// original and the reconstruction.
pub fn flipped<'a>(
    orig: &Series,
    recon: &Series,
    rules: &'a [Rule],
    grid: &[i64],
) -> Vec<(&'a Rule, i64)> {
    rules
        .iter()
        .flat_map(|rule| {
            budget::instants(rule, orig, grid)
                .into_iter()
                .filter(|&t| {
                    rule.condition(&orig.ts, &orig.vals, t)
                        != rule.condition(&recon.ts, &recon.vals, t)
                })
                .map(move |t| (rule, t))
        })
        .collect()
}

/// Alert-level comparison of original and reconstruction on `grid`.
pub fn alert_diff(orig: &Series, recon: &Series, rules: &[Rule], grid: &[i64]) -> Diff {
    let mut d = Diff::default();
    for rule in rules {
        d.merge(alertsafe_rules::compare(
            grid,
            &rule.trajectory(orig, grid),
            &rule.trajectory(recon, grid),
        ));
    }
    d
}
