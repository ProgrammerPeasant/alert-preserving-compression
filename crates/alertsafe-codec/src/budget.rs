//! Rule-aware error budget (Theorem 1 of `docs/formal-model.md`).
//!
//! For every rule `R` and every relevant evaluation instant `t` we compute a
//! tolerance `tau_R(t)`: if every sample in the support of `f` at `t` is
//! reconstructed within `tau_R(t)`, the condition `f(X)(t) cmp θ` keeps its
//! truth value. The per-sample budget is the minimum tolerance over all
//! `(R, t)` whose support contains the sample:
//!
//! `eps_i = min { tau_R(t) : R in rules, i in supp_R(t) }`.
//!
//! For `last`, `avg/min/max_over_time` the tolerance is the margin
//! `|f(X)(t) - θ|` divided by the sup-norm Lipschitz constant of `f` (1, or
//! `n` for `sum_over_time`), minus a floating-point slack. For `rate` the
//! aggregate is not globally Lipschitz in the values (the zero-point
//! extrapolation divides by the increase), so the tolerance is found by
//! bisection over a sound interval enclosure of Prometheus' formula.

use std::collections::VecDeque;
use std::ops::Range;

use alertsafe_rules::{Agg, Cmp, RateGeometry, Rule, Series, SlidingWindow};

use crate::codec::Mode;

const EPS: f64 = f64::EPSILON;

#[derive(Clone, Debug)]
pub struct Plan {
    pub budget: Vec<f64>,
    pub mode: Mode,
}

/// Budget for `series` under `rules`. Rules other than `rate` are protected at
/// every instant (any evaluation interval and phase); `rate` rules are
/// protected on `grid`.
///
/// Runs in `O(n + m)` per rule (plus a constant-length bisection per `rate`
/// instant): supports are monotone in `t`, so window aggregates slide
/// ([`Rule::eval_many`]) and the instants whose support contains sample `i`
/// form a contiguous run, turning `min` over them into a sliding minimum.
/// [`plan_naive`] is the direct `O(n * window)` definition.
pub fn plan(series: &Series, rules: &[Rule], grid: &[i64]) -> Plan {
    let mut budget = vec![f64::INFINITY; series.len()];
    for rule in rules {
        let ts_eval = instants(rule, series, grid);
        let (supports, taus) = tolerances(rule, series, &ts_eval);
        min_over_supports(&mut budget, &supports, &taus);
    }
    finish(series, rules, budget)
}

/// Direct evaluation of the budget definition, one instant at a time.
pub fn plan_naive(series: &Series, rules: &[Rule], grid: &[i64]) -> Plan {
    let mut budget = vec![f64::INFINITY; series.len()];
    for rule in rules {
        for t in instants(rule, series, grid) {
            if let Some((support, tau)) = tolerance(rule, &series.ts, &series.vals, t) {
                for b in &mut budget[support] {
                    *b = b.min(tau);
                }
            }
        }
    }
    finish(series, rules, budget)
}

fn finish(series: &Series, rules: &[Rule], mut budget: Vec<f64>) -> Plan {
    let mode = if rules.iter().any(|r| r.agg == Agg::Rate) {
        Mode::Counter
    } else {
        Mode::Gauge
    };
    if mode == Mode::Counter {
        // A reset is detected by `y_i < y_{i-1}`: keep both samples exact so
        // that the reconstruction has exactly the same resets.
        for i in 1..series.len() {
            if series.vals[i] < series.vals[i - 1] {
                budget[i - 1] = 0.0;
                budget[i] = 0.0;
            }
        }
    }
    Plan { budget, mode }
}

/// Instants at which `rule` is protected, sorted.
pub fn instants(rule: &Rule, series: &Series, grid: &[i64]) -> Vec<i64> {
    rule.critical_instants(&series.ts).unwrap_or_else(|| {
        let mut g = grid.to_vec();
        g.sort_unstable();
        g
    })
}

/// Supports and tolerances `tau_R(t)` at sorted instants `ts_eval`
/// (`+inf` where the rule yields no sample).
pub fn tolerances(rule: &Rule, series: &Series, ts_eval: &[i64]) -> (Vec<Range<usize>>, Vec<f64>) {
    let (ts, x) = (&series.ts, &series.vals);
    let supports = rule.supports(ts, ts_eval);
    let values = rule.eval_many(ts, x, ts_eval);
    let th = rule.threshold;
    let mut abs_sum = SlidingWindow::new(|a, b| a + b, 0.0);
    let mut prev = 0..0;
    let taus = supports
        .iter()
        .zip(&values)
        .zip(ts_eval)
        .map(|((r, v), &t)| {
            let Some(v) = *v else {
                return f64::INFINITY;
            };
            if v.is_nan() || th.is_nan() {
                return 0.0;
            }
            match rule.agg {
                Agg::Rate => rate_tolerance(rule, &ts[r.clone()], &x[r.clone()], t, v),
                Agg::AvgOverTime | Agg::SumOverTime => {
                    abs_sum.slide_map(x, &prev, r, f64::abs);
                    prev = r.clone();
                    lipschitz_tolerance(rule.agg, th, r.len(), abs_sum.get(), v)
                }
                agg => lipschitz_tolerance(agg, th, r.len(), 0.0, v),
            }
        })
        .collect();
    (supports, taus)
}

/// `budget[i] = min(budget[i], min { taus[k] : i in supports[k] })` for
/// supports whose ends are non-decreasing in `k`, by a monotone deque.
pub fn min_over_supports(budget: &mut [f64], supports: &[Range<usize>], taus: &[f64]) {
    let mut deque: VecDeque<usize> = VecDeque::new();
    // Instants `k < next` have been pushed; instants with `end <= i` popped.
    let mut next = 0;
    for (i, b) in budget.iter_mut().enumerate() {
        while next < supports.len() && supports[next].start <= i {
            if !supports[next].is_empty() {
                while deque.back().is_some_and(|&j| taus[j] >= taus[next]) {
                    deque.pop_back();
                }
                deque.push_back(next);
            }
            next += 1;
        }
        while deque.front().is_some_and(|&j| supports[j].end <= i) {
            deque.pop_front();
        }
        if let Some(&j) = deque.front() {
            *b = b.min(taus[j]);
        }
    }
}

/// Support of `f` at `t` and the tolerance `tau_R(t)`, or `None` if the rule
/// yields no sample at `t` (which depends on timestamps only, so it is
/// preserved by any value-only compression).
pub fn tolerance(rule: &Rule, ts: &[i64], x: &[f64], t: i64) -> Option<(Range<usize>, f64)> {
    let support = rule.support(ts, t);
    let v = rule.eval(ts, x, t)?;
    let th = rule.threshold;
    if v.is_nan() || th.is_nan() {
        return Some((support, 0.0));
    }
    let tau = match rule.agg {
        Agg::Rate => rate_tolerance(rule, &ts[support.clone()], &x[support.clone()], t, v),
        agg => {
            let abs_sum = x[support.clone()].iter().map(|a| a.abs()).sum();
            lipschitz_tolerance(agg, th, support.len(), abs_sum, v)
        }
    };
    Some((support, tau))
}

/// Tolerance `margin / L - slack` of a Lipschitz aggregate over `n` samples
/// whose absolute values sum to `abs_sum` (only read for `sum/avg`).
fn lipschitz_tolerance(agg: Agg, th: f64, n: usize, abs_sum: f64, v: f64) -> f64 {
    let n = n as f64;
    // Sup-norm Lipschitz constant of f and a bound on the rounding error of
    // evaluating f on both the original and the reconstruction. `abs_sum` may
    // itself carry a relative error of `n u`, far inside the factor 4.
    let (lip, slack) = match agg {
        Agg::Last | Agg::MinOverTime | Agg::MaxOverTime => (1.0, 0.0),
        Agg::AvgOverTime => (1.0, 4.0 * (n + 2.0) * EPS * (abs_sum / n + th.abs())),
        Agg::SumOverTime => (n, 4.0 * (n + 1.0) * EPS * (abs_sum + th.abs())),
        Agg::Rate => unreachable!(),
    };
    // Two `next_down`s: one absorbs the rounding of `|v - th|`, one turns a
    // non-strict bound into a strict one.
    let avail = ((v - th).abs() - slack).next_down().next_down();
    if avail > 0.0 {
        (avail / lip).next_down()
    } else {
        0.0
    }
}

fn rate_tolerance(rule: &Rule, ts: &[i64], x: &[f64], t: i64, v: f64) -> f64 {
    let th = rule.threshold;
    let n = x.len();
    let geom = RateGeometry::new(ts, t - rule.range_ms, t);
    let (first, last) = (x[0], x[n - 1]);
    let (mut reset_sum, mut resets) = (0.0, 0.0);
    for i in 1..n {
        if x[i] < x[i - 1] {
            reset_sum += x[i - 1];
            resets += 1.0;
        }
    }
    let cond = rule.cmp.holds(v, th);
    let slack = 64.0 * n as f64 * EPS * (v.abs() + th.abs() + 1.0);
    // Decision is preserved if the whole enclosure lies on the side of θ that
    // `v` lies on, with a margin for rounding.
    let safe = |eps: f64| {
        let (lo, hi) = rate_enclosure(&geom, first, last, reset_sum, resets, eps);
        let above = lo > th + slack;
        let below = hi < th - slack;
        match (rule.cmp, cond) {
            (Cmp::Gt | Cmp::Ge, true) | (Cmp::Lt | Cmp::Le, false) => above,
            (Cmp::Gt | Cmp::Ge, false) | (Cmp::Lt | Cmp::Le, true) => below,
        }
    };
    if !safe(0.0) {
        return 0.0;
    }
    let cap = first.abs().max(last.abs()) + 1.0;
    if safe(cap) {
        return cap;
    }
    // `safe` is monotone (the enclosure widens with eps). Bracket the largest
    // safe eps geometrically around the Lipschitz estimate of Proposition 1,
    // then bisect to a relative precision of 2^-10: a finer budget cannot
    // change the 2^e quantization step by more than one notch in 1024.
    let guess = ((v - th).abs() * geom.sampled / (2.0 + resets)).clamp(f64::MIN_POSITIVE, cap);
    let (mut lo, mut hi) = if safe(guess) {
        let mut lo = guess;
        loop {
            let up = (2.0 * lo).min(cap);
            if !safe(up) {
                break (lo, up);
            }
            lo = up;
        }
    } else {
        let mut hi = guess;
        loop {
            let down = 0.5 * hi;
            if down < f64::MIN_POSITIVE {
                return 0.0;
            }
            if safe(down) {
                break (down, hi);
            }
            hi = down;
        }
    };
    while hi - lo > lo * (1.0 / 1024.0) {
        let mid = 0.5 * (lo + hi);
        if safe(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Interval enclosure of `extrapolatedRate` over all reconstructions with
/// `x_i - eps <= y_i <= x_i` (counter-mode quantizer), unchanged resets, and
/// unchanged timestamps.
pub fn rate_enclosure(
    g: &RateGeometry,
    first: f64,
    last: f64,
    reset_sum: f64,
    resets: f64,
    eps: f64,
) -> (f64, f64) {
    let res_lo = (last - eps) - first + reset_sum - resets * eps;
    let res_hi = last - (first - eps) + reset_sum;
    let (first_lo, first_hi) = (first - eps, first);
    // durationToStart = min(to_start, durationToZero) where durationToZero =
    // sampled * first / result applies only if result > 0 and first >= 0.
    let can_apply = res_hi > 0.0 && first_hi >= 0.0;
    let must_apply = res_lo > 0.0 && first_lo >= 0.0;
    let ds_lo = if can_apply {
        g.to_start.min(g.sampled * first_lo.max(0.0) / res_hi)
    } else {
        g.to_start
    };
    let ds_hi = if must_apply {
        g.to_start.min(g.sampled * first_hi / res_lo)
    } else {
        g.to_start
    };
    let factor = |ds: f64| (g.sampled + ds + g.to_end) / g.sampled / g.range;
    let (f_lo, f_hi) = (factor(ds_lo), factor(ds_hi));
    let corners = [res_lo * f_lo, res_lo * f_hi, res_hi * f_lo, res_hi * f_hi];
    let lo = corners.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = corners.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_over_lipschitz() {
        let r = Rule::new("a", Agg::AvgOverTime, 10, Cmp::Gt, 1.0);
        let (_, tau) = tolerance(&r, &[0, 5], &[1.5, 2.5], 5).unwrap();
        assert!(tau < 1.0 && tau > 1.0 - 1e-12);
        let s = Rule::new("s", Agg::SumOverTime, 10, Cmp::Gt, 1.0);
        let (_, tau) = tolerance(&s, &[0, 5], &[1.5, 2.5], 5).unwrap();
        assert!(tau < 1.5 && tau > 1.5 - 1e-12);
    }

    #[test]
    fn on_threshold_is_exact() {
        let r = Rule::new("a", Agg::Last, 0, Cmp::Ge, 2.0);
        assert_eq!(tolerance(&r, &[0], &[2.0], 0).unwrap().1, 0.0);
    }

    #[test]
    fn rate_enclosure_contains_samples() {
        let ts: Vec<i64> = (0..9).map(|i| 3_000 + i * 15_000).collect();
        let x: Vec<f64> = (0..9)
            .map(|i| 50.0 + 20.0 * i as f64 + (i % 3) as f64)
            .collect();
        let g = RateGeometry::new(&ts, 0, 125_000);
        let eps = 7.0;
        let (lo, hi) = rate_enclosure(&g, x[0], x[8], 0.0, 0.0, eps);
        for k in 0..200 {
            let y: Vec<f64> = x
                .iter()
                .enumerate()
                .map(|(i, v)| v - eps * (((i * 31 + k * 17) % 10) as f64 / 9.0))
                .collect();
            if y.windows(2).all(|w| w[0] <= w[1]) {
                let r = alertsafe_rules::extrapolated_rate(&ts, &y, 0, 125_000).unwrap();
                assert!(lo <= r && r <= hi, "{lo} <= {r} <= {hi}");
            }
        }
    }
}
