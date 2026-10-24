//! Alert-preserving lossy compression of monitoring metrics.
//!
//! [`compress`] derives a per-sample error budget from a rule set
//! ([`budget::plan`]), encodes the series with it ([`codec::encode`]), and
//! verifies the reconstruction against the reference rule semantics of
//! `alertsafe-rules`, zeroing the budget on the support of any evaluation
//! whose condition changed (verify-and-repair). The analytic budget is sound
//! in exact arithmetic (see `docs/formal-model.md`); the repair loop turns it
//! into an end-to-end guarantee against the reference evaluator, including
//! floating-point effects. [`Options::slack`] starts from a multiple of the
//! analytic budget instead and lets the same loop shrink it where needed
//! (the analytic budget is the floor of that repair). Verification uses the
//! sliding-window evaluator
//! [`alertsafe_rules::Rule::eval_many`], with a margin for the summation
//! order of `sum/avg` (see [`flipped`]); the property tests additionally
//! check every protected instant with the per-instant reference
//! [`alertsafe_rules::Rule::eval`].

pub mod bits;
pub mod budget;
pub mod codec;
pub mod rc;

#[cfg(kani)]
mod proofs;

use alertsafe_rules::{Agg, Diff, Rule, Series, SlidingWindow};

pub use budget::{plan, plan_naive, Plan};
pub use codec::{decode, encode, encode_chunk, encode_with, Coding, Encoded, Mode};

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Samples per independently decodable stream. Prometheus TSDB cuts
    /// chunks at 120 samples; `usize::MAX` codes the series as one stream.
    pub chunk: usize,
    /// Upper bound on every sample's error on top of the rule budget, for
    /// consumers other than the alert rules (dashboards, ad-hoc queries).
    /// Lowering a budget never breaks the guarantee.
    pub max_error: f64,
    /// Optimistic factor `κ ≥ 1` on the analytic budget. The analytic budget
    /// covers the worst case of all errors at once; real errors mostly cancel
    /// in window aggregates, so a larger budget changes few evaluations. The
    /// repair loop halves the budget on the support of each flipped one,
    /// never below the analytic budget (which cannot flip it), and zeroes it
    /// there as the last resort. `1.0` is the analytic budget alone.
    pub slack: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            chunk: usize::MAX,
            max_error: f64::INFINITY,
            slack: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Compressed {
    /// Consecutive streams of at most [`Options::chunk`] samples each.
    pub chunks: Vec<Encoded>,
    pub plan: Plan,
    /// Evaluations whose condition flipped and were repaired, summed over
    /// rounds. Zero with the analytic budget (Theorem 1) unless a
    /// floating-point effect escaped its slack.
    pub repairs: usize,
    /// Encode-and-verify rounds after the first one.
    pub rounds: usize,
}

impl Compressed {
    pub fn bits(&self) -> usize {
        self.chunks.iter().map(|c| c.bits).sum()
    }

    pub fn bits_per_value(&self) -> f64 {
        self.bits() as f64 / self.plan.budget.len().max(1) as f64
    }

    pub fn decode(&self) -> Vec<f64> {
        self.chunks.iter().flat_map(|c| decode(&c.bytes)).collect()
    }
}

/// Rule-aware compression of `series`: every rule's condition is identical on
/// the original and the reconstruction at every protected instant (see
/// [`budget::plan`]), hence the alert state trajectories are identical.
pub fn compress(series: &Series, rules: &[Rule], grid: &[i64]) -> Compressed {
    compress_with(series, rules, grid, Options::default())
}

/// [`compress`] with chunking, an additional error cap and an optimistic
/// budget. The budget is still planned and verified on the whole series:
/// windows span chunk boundaries, so the encoder needs lookahead of the
/// longest window.
pub fn compress_with(series: &Series, rules: &[Rule], grid: &[i64], opts: Options) -> Compressed {
    assert!(opts.slack >= 1.0, "slack must be at least 1");
    let mut plan = budget::plan(series, rules, grid);
    for b in &mut plan.budget {
        // Written so that a NaN budget (exact sample) stays NaN.
        if *b > opts.max_error {
            *b = opts.max_error;
        }
    }
    // Repair floor: a support within it cannot flip (Theorem 1).
    let floor = plan.budget.clone();
    for b in plan.budget.iter_mut().filter(|b| **b > 0.0) {
        if *b * opts.slack < opts.max_error {
            *b *= opts.slack;
        } else if *b < opts.max_error {
            *b = opts.max_error;
        }
    }
    let (mut repairs, mut rounds) = (0, 0);
    loop {
        let chunks = encode_chunked(&series.vals, &plan.budget, plan.mode, opts.chunk);
        let c = Compressed {
            chunks,
            plan,
            repairs,
            rounds,
        };
        let recon = series.with_values(c.decode());
        let flips = flipped(series, &recon, rules, grid);
        if flips.is_empty() {
            return c;
        }
        plan = c.plan;
        repairs += flips.len();
        rounds += 1;
        // Decide on the budgets this round was encoded with: supports overlap.
        let prev = plan.budget.clone();
        for (rule, t) in flips {
            let sup = rule.support(&series.ts, t);
            let relaxed = sup.clone().any(|i| prev[i] > floor[i]);
            for i in sup {
                plan.budget[i] = if relaxed {
                    // An infinite budget (infinite slack) goes to the floor.
                    let half = if prev[i].is_finite() {
                        prev[i] * 0.5
                    } else {
                        floor[i]
                    };
                    plan.budget[i].min(half.max(floor[i]))
                } else {
                    0.0
                };
            }
        }
    }
}

/// Codes `vals` as consecutive independent streams of at most `chunk`
/// samples, each continuing the previous one (see [`encode_chunk`]).
pub fn encode_chunked(vals: &[f64], budget: &[f64], mode: Mode, chunk: usize) -> Vec<Encoded> {
    let mut floor = f64::NEG_INFINITY;
    vals.chunks(chunk)
        .zip(budget.chunks(chunk))
        .map(|(xs, eps)| {
            let e = encode_chunk(xs, eps, mode, Coding::Entropy, floor);
            floor = e.last;
            e
        })
        .collect()
}

/// Protected evaluations `(rule, t)` whose condition differs between the
/// original and the reconstruction, or may differ under another evaluator.
///
/// `sum/avg_over_time` are evaluated with a sliding sum here and summed in
/// another order by the reference evaluator (and by Prometheus, which uses
/// Kahan summation). Two summation orders of `n` values differ by at most
/// `2 (n - 1) u sum |y_i|`, so an evaluation of the reconstruction counts as
/// preserved only if it lies on the original's side of θ by more than that,
/// or reads samples bitwise equal to the original's. The analytic budget
/// leaves a margin of at least twice this bound (see [`budget`]), so it is never
/// flagged; an optimistic one ([`Options::slack`]) can land closer to θ.
pub fn flipped<'a>(
    orig: &Series,
    recon: &Series,
    rules: &'a [Rule],
    grid: &[i64],
) -> Vec<(&'a Rule, i64)> {
    // `changed[i]`: samples before `i` whose reconstruction is not bitwise exact.
    let changed: Vec<usize> = std::iter::once(0)
        .chain(orig.vals.iter().zip(&recon.vals).scan(0, |c, (x, y)| {
            *c += usize::from(x.to_bits() != y.to_bits());
            Some(*c)
        }))
        .collect();
    rules
        .iter()
        .flat_map(|rule| {
            let ts_eval = budget::instants(rule, orig, grid);
            let a = rule.conditions_many(&orig.ts, &orig.vals, &ts_eval);
            let b = rule.eval_many(&recon.ts, &recon.vals, &ts_eval);
            let supports = rule.supports(&orig.ts, &ts_eval);
            let mut abs_sum = SlidingWindow::new(|a, b| a + b, 0.0);
            let mut prev = 0..0;
            let flips: Vec<bool> = (0..ts_eval.len())
                .map(|k| {
                    let cond = b[k].is_some_and(|v| rule.cmp.holds(v, rule.threshold));
                    if cond != a[k] {
                        return true;
                    }
                    let (Agg::SumOverTime | Agg::AvgOverTime, Some(v)) = (rule.agg, b[k]) else {
                        return false;
                    };
                    let r = &supports[k];
                    abs_sum.slide_map(&recon.vals, &prev, r, f64::abs);
                    prev = r.clone();
                    if changed[r.end] == changed[r.start] {
                        return false;
                    }
                    let n = r.len() as f64;
                    let order = match rule.agg {
                        Agg::AvgOverTime => (n + 2.0) * f64::EPSILON * abs_sum.get() / n,
                        _ => (n + 1.0) * f64::EPSILON * abs_sum.get(),
                    };
                    // An overflowing sum may be finite in another order.
                    let safe = v.is_finite() && (v - rule.threshold).abs() > order;
                    !safe
                })
                .collect();
            ts_eval
                .into_iter()
                .zip(flips)
                .filter(|&(_, f)| f)
                .map(move |(t, _)| (rule, t))
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
