//! Reference semantics of Prometheus-style alerting rules over a single series.
//!
//! This crate is the decision operator `D_R` of the formal model: it maps a
//! series `X` and a rule `R` to the per-evaluation boolean condition and to the
//! alert state trajectory (`Inactive` / `Pending` / `Firing`). It follows
//! `promql/functions.go` (`extrapolatedRate`) and `rules/alerting.go` (hold
//! duration `for`, `keep_firing_for`) of Prometheus 3.x, restricted to one
//! float series without native histograms or start timestamps.

use std::ops::Range;

/// Instant-vector lookback delta (Prometheus default, 5m).
pub const DEFAULT_LOOKBACK_MS: i64 = 5 * 60 * 1000;

/// One float series: strictly increasing millisecond timestamps and values.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub ts: Vec<i64>,
    pub vals: Vec<f64>,
}

impl Series {
    pub fn new(ts: Vec<i64>, vals: Vec<f64>) -> Self {
        assert_eq!(
            ts.len(),
            vals.len(),
            "timestamps and values differ in length"
        );
        assert!(
            ts.windows(2).all(|w| w[0] < w[1]),
            "timestamps must be strictly increasing"
        );
        Series { ts, vals }
    }

    pub fn len(&self) -> usize {
        self.ts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ts.is_empty()
    }

    /// Same timestamps, different values (e.g. a reconstruction).
    pub fn with_values(&self, vals: Vec<f64>) -> Series {
        Series::new(self.ts.clone(), vals)
    }
}

/// Aggregate `f` applied to the samples a rule reads at evaluation time `t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    /// Instant selector `x`: last sample in `(t - lookback, t]`.
    Last,
    AvgOverTime,
    SumOverTime,
    MinOverTime,
    MaxOverTime,
    /// `rate(x[w])` on a counter, with Prometheus extrapolation.
    Rate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Gt,
    Ge,
    Lt,
    Le,
}

impl Cmp {
    pub fn holds(self, v: f64, threshold: f64) -> bool {
        match self {
            Cmp::Gt => v > threshold,
            Cmp::Ge => v >= threshold,
            Cmp::Lt => v < threshold,
            Cmp::Le => v <= threshold,
        }
    }

    pub fn is_strict(self) -> bool {
        matches!(self, Cmp::Gt | Cmp::Lt)
    }
}

/// Threshold rule `R = (f, w, cmp, θ, for, keep_firing_for)`, i.e. the PromQL
/// alert `expr: f(x[w]) cmp θ` with `for` and `keep_firing_for`.
#[derive(Clone, Debug)]
pub struct Rule {
    pub name: String,
    pub agg: Agg,
    /// Range `w` of the range selector; ignored for [`Agg::Last`].
    pub range_ms: i64,
    pub cmp: Cmp,
    pub threshold: f64,
    pub for_ms: i64,
    pub keep_firing_for_ms: i64,
}

impl Rule {
    pub fn new(name: impl Into<String>, agg: Agg, range_ms: i64, cmp: Cmp, threshold: f64) -> Self {
        Rule {
            name: name.into(),
            agg,
            range_ms,
            cmp,
            threshold,
            for_ms: 0,
            keep_firing_for_ms: 0,
        }
    }

    pub fn hold(mut self, for_ms: i64) -> Self {
        self.for_ms = for_ms;
        self
    }

    pub fn keep_firing(mut self, keep_firing_for_ms: i64) -> Self {
        self.keep_firing_for_ms = keep_firing_for_ms;
        self
    }

    /// Indices of the samples the aggregate reads at time `t` (its support).
    /// Range selectors are left-open, `(t - w, t]`, as in Prometheus 3.
    pub fn support(&self, ts: &[i64], t: i64) -> Range<usize> {
        let hi = ts.partition_point(|&s| s <= t);
        match self.agg {
            Agg::Last => {
                let lo = ts.partition_point(|&s| s <= t - DEFAULT_LOOKBACK_MS);
                if hi > lo {
                    hi - 1..hi
                } else {
                    hi..hi
                }
            }
            _ => ts.partition_point(|&s| s <= t - self.range_ms)..hi,
        }
    }

    /// `f(X)(t)`, or `None` when the rule produces no sample at `t`.
    pub fn eval(&self, ts: &[i64], vals: &[f64], t: i64) -> Option<f64> {
        let r = self.support(ts, t);
        if r.is_empty() {
            return None;
        }
        let v = &vals[r.clone()];
        match self.agg {
            Agg::Last => Some(v[0]),
            Agg::AvgOverTime => Some(v.iter().sum::<f64>() / v.len() as f64),
            Agg::SumOverTime => Some(v.iter().sum()),
            Agg::MinOverTime => Some(v.iter().copied().fold(f64::INFINITY, f64::min)),
            Agg::MaxOverTime => Some(v.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
            Agg::Rate => extrapolated_rate(&ts[r], v, t - self.range_ms, t),
        }
    }

    /// The boolean condition `c_R(X)(t)`: the alert expression returns a sample.
    pub fn condition(&self, ts: &[i64], vals: &[f64], t: i64) -> bool {
        self.eval(ts, vals, t)
            .is_some_and(|v| self.cmp.holds(v, self.threshold))
    }

    pub fn conditions(&self, series: &Series, grid: &[i64]) -> Vec<bool> {
        grid.iter()
            .map(|&t| self.condition(&series.ts, &series.vals, t))
            .collect()
    }

    /// Full decision operator `D_R(X)` on an evaluation grid.
    pub fn trajectory(&self, series: &Series, grid: &[i64]) -> Vec<AlertState> {
        trajectory(self, grid, &self.conditions(series, grid))
    }

    /// Instants at which the support of the aggregate can change. For every
    /// aggregate except `rate` the value `f(X)(t)` depends on `t` only through
    /// the support, so it is piecewise constant between consecutive instants
    /// returned here: checking them covers every real `t`, i.e. every
    /// evaluation interval and phase. Returns `None` for `rate`, whose
    /// extrapolation depends on `t` continuously.
    pub fn critical_instants(&self, ts: &[i64]) -> Option<Vec<i64>> {
        let offset = match self.agg {
            Agg::Rate => return None,
            Agg::Last => DEFAULT_LOOKBACK_MS,
            _ => self.range_ms,
        };
        let mut out: Vec<i64> = ts.iter().flat_map(|&s| [s, s + offset]).collect();
        out.sort_unstable();
        out.dedup();
        Some(out)
    }
}

/// `rate()` of a counter as computed by Prometheus `extrapolatedRate` for float
/// samples without start timestamps. `start` is exclusive, `end` inclusive.
pub fn extrapolated_rate(ts: &[i64], v: &[f64], start: i64, end: i64) -> Option<f64> {
    let n = v.len();
    if n < 2 {
        return None;
    }
    let mut result = v[n - 1] - v[0];
    for i in 1..n {
        if v[i] < v[i - 1] {
            result += v[i - 1];
        }
    }
    let g = RateGeometry::new(ts, start, end);
    let mut to_start = g.to_start;
    let to_zero = if result > 0.0 && v[0] >= 0.0 {
        g.sampled * (v[0] / result)
    } else {
        to_start
    };
    if to_zero < to_start {
        to_start = to_zero;
    }
    Some(result * (g.sampled + to_start + g.to_end) / g.sampled / g.range)
}

/// Timestamp-only quantities of `extrapolatedRate`, after the extrapolation
/// threshold has been applied (seconds).
#[derive(Clone, Copy, Debug)]
pub struct RateGeometry {
    pub sampled: f64,
    pub to_start: f64,
    pub to_end: f64,
    pub range: f64,
}

impl RateGeometry {
    pub fn new(ts: &[i64], start: i64, end: i64) -> Self {
        let n = ts.len();
        let sampled = (ts[n - 1] - ts[0]) as f64 / 1000.0;
        let avg = sampled / (n - 1) as f64;
        let threshold = avg * 1.1;
        let mut to_start = (ts[0] - start) as f64 / 1000.0;
        let mut to_end = (end - ts[n - 1]) as f64 / 1000.0;
        if to_start >= threshold {
            to_start = avg / 2.0;
        }
        if to_end >= threshold {
            to_end = avg / 2.0;
        }
        RateGeometry {
            sampled,
            to_start,
            to_end,
            range: (end - start) as f64 / 1000.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertState {
    Inactive,
    Pending,
    Firing,
}

/// Alert state machine of `rules/alerting.go` driven by per-evaluation
/// conditions `conds[i]` at times `grid[i]`.
pub fn trajectory(rule: &Rule, grid: &[i64], conds: &[bool]) -> Vec<AlertState> {
    assert_eq!(grid.len(), conds.len());
    let mut state = AlertState::Inactive;
    let mut active_at = 0i64;
    let mut keep_since: Option<i64> = None;
    let mut out = Vec::with_capacity(grid.len());
    for (&t, &c) in grid.iter().zip(conds) {
        if c {
            keep_since = None;
            if state == AlertState::Inactive {
                state = AlertState::Pending;
                active_at = t;
            }
        } else {
            let keep = state == AlertState::Firing
                && rule.keep_firing_for_ms > 0
                && t - *keep_since.get_or_insert(t) < rule.keep_firing_for_ms;
            if !keep {
                state = AlertState::Inactive;
                keep_since = None;
            }
        }
        if state == AlertState::Pending && t - active_at >= rule.for_ms {
            state = AlertState::Firing;
        }
        out.push(state);
    }
    out
}

/// One firing episode: first and last evaluation in state `Firing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Episode {
    pub fired_at: i64,
    pub last_firing: i64,
}

pub fn episodes(grid: &[i64], traj: &[AlertState]) -> Vec<Episode> {
    let mut out: Vec<Episode> = Vec::new();
    let mut open = false;
    for (&t, &s) in grid.iter().zip(traj) {
        match (s == AlertState::Firing, open) {
            (true, false) => {
                out.push(Episode {
                    fired_at: t,
                    last_firing: t,
                });
                open = true;
            }
            (true, true) => out.last_mut().unwrap().last_firing = t,
            (false, _) => open = false,
        }
    }
    out
}

/// Difference between the trajectory on original (`a`) and reconstructed
/// (`b`) data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Evaluations where the states differ.
    pub state_mismatches: usize,
    /// Episodes of `a` that no episode of `b` overlaps.
    pub missed: usize,
    /// Episodes of `b` that no episode of `a` overlaps.
    pub spurious: usize,
    /// Largest `|t_fire - t̂_fire|` over overlapping episode pairs.
    pub max_shift_ms: i64,
}

impl Diff {
    pub fn changed_firings(&self) -> usize {
        self.missed + self.spurious
    }

    pub fn merge(&mut self, o: Diff) {
        self.state_mismatches += o.state_mismatches;
        self.missed += o.missed;
        self.spurious += o.spurious;
        self.max_shift_ms = self.max_shift_ms.max(o.max_shift_ms);
    }
}

pub fn compare(grid: &[i64], a: &[AlertState], b: &[AlertState]) -> Diff {
    let state_mismatches = a.iter().zip(b).filter(|(x, y)| x != y).count();
    let (ea, eb) = (episodes(grid, a), episodes(grid, b));
    let overlaps =
        |x: &Episode, y: &Episode| x.fired_at <= y.last_firing && y.fired_at <= x.last_firing;
    let missed = ea
        .iter()
        .filter(|x| !eb.iter().any(|y| overlaps(x, y)))
        .count();
    let spurious = eb
        .iter()
        .filter(|y| !ea.iter().any(|x| overlaps(x, y)))
        .count();
    let max_shift_ms = ea
        .iter()
        .flat_map(|x| {
            eb.iter()
                .filter(move |y| overlaps(x, y))
                .map(move |y| (x.fired_at - y.fired_at).abs())
        })
        .max()
        .unwrap_or(0);
    Diff {
        state_mismatches,
        missed,
        spurious,
        max_shift_ms,
    }
}

/// Evaluation grid `start, start + step, ...` up to `end` inclusive.
pub fn grid(start: i64, end: i64, step: i64) -> Vec<i64> {
    (0..)
        .map(|i| start + i * step)
        .take_while(|&t| t <= end)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use AlertState::*;

    fn rule() -> Rule {
        Rule::new("r", Agg::Last, 0, Cmp::Gt, 1.0)
    }

    #[test]
    fn hold_duration_delays_firing() {
        let r = rule().hold(20);
        let g = grid(0, 50, 10);
        let t = trajectory(&r, &g, &[true, true, true, false, true, true]);
        assert_eq!(t, [Pending, Pending, Firing, Inactive, Pending, Pending]);
    }

    #[test]
    fn zero_hold_fires_immediately() {
        let t = trajectory(&rule(), &grid(0, 20, 10), &[false, true, false]);
        assert_eq!(t, [Inactive, Firing, Inactive]);
    }

    #[test]
    fn keep_firing_for_extends_firing() {
        let r = rule().keep_firing(20);
        let t = trajectory(
            &r,
            &grid(0, 50, 10),
            &[true, false, false, false, true, false],
        );
        assert_eq!(t, [Firing, Firing, Firing, Inactive, Firing, Firing]);
    }

    #[test]
    fn keep_firing_does_not_apply_to_pending() {
        let r = rule().hold(100).keep_firing(50);
        let t = trajectory(&r, &grid(0, 10, 10), &[true, false]);
        assert_eq!(t, [Pending, Inactive]);
    }

    #[test]
    fn window_is_left_open() {
        let s = Series::new(vec![0, 10, 20], vec![1.0, 2.0, 3.0]);
        let r = Rule::new("s", Agg::SumOverTime, 10, Cmp::Gt, 0.0);
        assert_eq!(r.eval(&s.ts, &s.vals, 20), Some(3.0));
        assert_eq!(r.eval(&s.ts, &s.vals, 25), Some(3.0));
    }

    #[test]
    fn rate_matches_hand_computation() {
        // 15s scrape, counter +15 per sample => 1/s. Window 60s aligned so that
        // extrapolation reaches both ends.
        let ts: Vec<i64> = (0..5).map(|i| 1_000 + i * 15_000).collect();
        let v: Vec<f64> = (0..5).map(|i| 100.0 + 15.0 * i as f64).collect();
        let r = extrapolated_rate(&ts, &v, 0, 62_000).unwrap();
        assert!((r - 1.0).abs() < 1e-12, "{r}");
    }

    #[test]
    fn rate_handles_reset() {
        let ts = vec![0, 15_000, 30_000];
        let v = vec![10.0, 20.0, 5.0];
        let r = extrapolated_rate(&ts, &v, -1, 30_000).unwrap();
        assert!(r > 0.0);
    }

    #[test]
    fn critical_instants_cover_all_phases() {
        let s = Series::new(vec![0, 7, 19, 30], vec![0.0, 5.0, 0.0, 5.0]);
        let r = Rule::new("a", Agg::AvgOverTime, 10, Cmp::Gt, 2.0);
        let crit = r.critical_instants(&s.ts).unwrap();
        for t in -5..45 {
            let k = crit.partition_point(|&c| c <= t);
            if k == 0 {
                assert!(!r.condition(&s.ts, &s.vals, t));
            } else {
                assert_eq!(
                    r.eval(&s.ts, &s.vals, t),
                    r.eval(&s.ts, &s.vals, crit[k - 1])
                );
            }
        }
    }

    #[test]
    fn compare_counts_missed_and_spurious() {
        let g = grid(0, 50, 10);
        let a = [Inactive, Firing, Firing, Inactive, Inactive, Inactive];
        let b = [Inactive, Inactive, Firing, Inactive, Firing, Inactive];
        let d = compare(&g, &a, &b);
        assert_eq!(
            (d.missed, d.spurious, d.max_shift_ms, d.state_mismatches),
            (0, 1, 10, 2)
        );
    }
}
