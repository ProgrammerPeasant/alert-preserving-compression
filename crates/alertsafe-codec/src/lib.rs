//! Alert-preserving lossy compression of monitoring metrics.
//!
//! [`compress`] derives a per-sample error budget from a rule set
//! ([`budget::plan`]), encodes the series with it ([`codec::encode`]), and
//! verifies the reconstruction against the reference rule semantics of
//! `alertsafe-rules`, zeroing the budget on the support of any evaluation
//! whose condition changed (verify-and-repair). The analytic budget is sound
//! in exact arithmetic (see `docs/formal-model.md`); the repair loop turns it
//! into an end-to-end guarantee against the reference evaluator, including
//! floating-point effects. Verification uses the sliding-window evaluator
//! [`alertsafe_rules::Rule::eval_many`]; the property tests additionally
//! check every protected instant with the per-instant reference
//! [`alertsafe_rules::Rule::eval`].

pub mod bits;
pub mod budget;
pub mod codec;
pub mod rc;

use alertsafe_rules::{Diff, Rule, Series};

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
}

impl Default for Options {
    fn default() -> Self {
        Options {
            chunk: usize::MAX,
            max_error: f64::INFINITY,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Compressed {
    /// Consecutive streams of at most [`Options::chunk`] samples each.
    pub chunks: Vec<Encoded>,
    pub plan: Plan,
    /// Evaluations whose condition flipped under the analytic budget and were
    /// repaired. Zero whenever the analytic budget alone was sufficient.
    pub repairs: usize,
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

/// [`compress`] with chunking and an additional error cap. The budget is
/// still planned and verified on the whole series: windows span chunk
/// boundaries, so the encoder needs lookahead of the longest window.
pub fn compress_with(series: &Series, rules: &[Rule], grid: &[i64], opts: Options) -> Compressed {
    let mut plan = budget::plan(series, rules, grid);
    for b in &mut plan.budget {
        // Written so that a NaN budget (exact sample) stays NaN.
        if *b > opts.max_error {
            *b = opts.max_error;
        }
    }
    let mut repairs = 0;
    loop {
        let chunks = encode_chunked(&series.vals, &plan.budget, plan.mode, opts.chunk);
        let c = Compressed {
            chunks,
            plan,
            repairs,
        };
        let recon = series.with_values(c.decode());
        let flips = flipped(series, &recon, rules, grid);
        if flips.is_empty() {
            return c;
        }
        plan = c.plan;
        repairs += flips.len();
        for (rule, t) in flips {
            for b in &mut plan.budget[rule.support(&series.ts, t)] {
                *b = 0.0;
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
            let ts_eval = budget::instants(rule, orig, grid);
            let a = rule.conditions_many(&orig.ts, &orig.vals, &ts_eval);
            let b = rule.conditions_many(&recon.ts, &recon.vals, &ts_eval);
            ts_eval
                .into_iter()
                .zip(a.into_iter().zip(b))
                .filter(|(_, (a, b))| a != b)
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
