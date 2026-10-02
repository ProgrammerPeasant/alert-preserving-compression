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
        // Merge the two sorted sequences `ts` and `ts + offset`.
        let mut out = Vec::with_capacity(2 * ts.len());
        let (mut i, mut j) = (0, 0);
        while i < ts.len() || j < ts.len() {
            let t = if j == ts.len() || (i < ts.len() && ts[i] <= ts[j] + offset) {
                i += 1;
                ts[i - 1]
            } else {
                j += 1;
                ts[j - 1] + offset
            };
            if out.last() != Some(&t) {
                out.push(t);
            }
        }
        Some(out)
    }
}

impl Rule {
    /// Supports at sorted instants `ts_eval`, by two pointers in `O(n + m)`.
    /// Equal to `ts_eval.map(|t| self.support(ts, t))`.
    pub fn supports(&self, ts: &[i64], ts_eval: &[i64]) -> Vec<Range<usize>> {
        assert!(
            ts_eval.windows(2).all(|w| w[0] <= w[1]),
            "evaluation instants must be sorted"
        );
        let offset = match self.agg {
            Agg::Last => DEFAULT_LOOKBACK_MS,
            _ => self.range_ms,
        };
        let (mut lo, mut hi) = (0, 0);
        ts_eval
            .iter()
            .map(|&t| {
                while hi < ts.len() && ts[hi] <= t {
                    hi += 1;
                }
                while lo < ts.len() && ts[lo] <= t - offset {
                    lo += 1;
                }
                match self.agg {
                    Agg::Last if hi > lo => hi - 1..hi,
                    Agg::Last => hi..hi,
                    _ => lo.min(hi)..hi,
                }
            })
            .collect()
    }

    /// `f(X)(t)` at every sorted instant of `ts_eval` in amortised `O(n + m)`
    /// (sliding-window aggregation; [`Rule::eval`] costs `O(window)` per
    /// instant). Bitwise equal to [`Rule::eval`] for `last`, `min/max` and
    /// `rate`; for `sum/avg_over_time` the window is summed in a different
    /// order, which stays within the same `(n - 1) u sum|x|` rounding bound.
    pub fn eval_many(&self, ts: &[i64], vals: &[f64], ts_eval: &[i64]) -> Vec<Option<f64>> {
        let supports = self.supports(ts, ts_eval);
        let mut out = Vec::with_capacity(ts_eval.len());
        match self.agg {
            Agg::Last => out.extend(
                supports
                    .iter()
                    .map(|r| vals.get(r.start).filter(|_| !r.is_empty()).copied()),
            ),
            Agg::Rate => {
                let resets = reset_prefix(vals);
                for (r, &t) in supports.iter().zip(ts_eval) {
                    out.push(rate_in(ts, vals, &resets, r.clone(), t - self.range_ms, t));
                }
            }
            agg => {
                let (op, id): (fn(f64, f64) -> f64, f64) = match agg {
                    Agg::MinOverTime => (f64::min, f64::INFINITY),
                    Agg::MaxOverTime => (f64::max, f64::NEG_INFINITY),
                    _ => (|a, b| a + b, 0.0),
                };
                let mut w = SlidingWindow::new(op, id);
                let mut prev = 0..0;
                for r in &supports {
                    w.slide(vals, &prev, r);
                    prev = r.clone();
                    out.push((!r.is_empty()).then(|| match agg {
                        Agg::AvgOverTime => w.get() / r.len() as f64,
                        _ => w.get(),
                    }));
                }
            }
        }
        out
    }

    /// Conditions at sorted instants, via [`Rule::eval_many`].
    pub fn conditions_many(&self, ts: &[i64], vals: &[f64], ts_eval: &[i64]) -> Vec<bool> {
        self.eval_many(ts, vals, ts_eval)
            .into_iter()
            .map(|v| v.is_some_and(|v| self.cmp.holds(v, self.threshold)))
            .collect()
    }
}

/// `resets[i]` = number of `j < i`, `j >= 1`, with `v[j] < v[j - 1]`.
pub fn reset_prefix(v: &[f64]) -> Vec<u32> {
    let mut out = Vec::with_capacity(v.len() + 1);
    let mut c = 0;
    out.push(0);
    for i in 0..v.len() {
        if i > 0 && v[i] < v[i - 1] {
            c += 1;
        }
        out.push(c);
    }
    out
}

/// [`extrapolated_rate`] on the support `r`, skipping the reset scan when the
/// prefix counts show the window has no reset.
pub fn rate_in(
    ts: &[i64],
    v: &[f64],
    resets: &[u32],
    r: Range<usize>,
    start: i64,
    end: i64,
) -> Option<f64> {
    let n = r.len();
    if n < 2 {
        return None;
    }
    if resets[r.end] != resets[r.start + 1] {
        return extrapolated_rate(&ts[r.clone()], &v[r], start, end);
    }
    let (ts, v) = (&ts[r.clone()], &v[r]);
    let result = v[n - 1] - v[0];
    let g = RateGeometry::from_ends(ts[0], ts[n - 1], n, start, end);
    Some(finish_rate(result, v[0], &g))
}

/// Queue aggregate for an associative `op` (two-stacks algorithm): push at the
/// back, pop at the front, query in amortised `O(1)`.
pub struct SlidingWindow {
    op: fn(f64, f64) -> f64,
    id: f64,
    /// Suffix aggregates of the front part; the last entry covers all of it.
    front: Vec<f64>,
    back: Vec<f64>,
    back_agg: f64,
}

impl SlidingWindow {
    pub fn new(op: fn(f64, f64) -> f64, id: f64) -> Self {
        SlidingWindow {
            op,
            id,
            front: Vec::new(),
            back: Vec::new(),
            back_agg: id,
        }
    }

    pub fn push(&mut self, x: f64) {
        self.back.push(x);
        self.back_agg = if self.back.len() == 1 {
            x
        } else {
            (self.op)(self.back_agg, x)
        };
    }

    pub fn pop(&mut self) {
        if self.front.is_empty() {
            let mut acc = None;
            for &x in self.back.iter().rev() {
                let a = acc.map_or(x, |a| (self.op)(x, a));
                self.front.push(a);
                acc = Some(a);
            }
            self.back.clear();
            self.back_agg = self.id;
        }
        self.front.pop().expect("pop from empty window");
    }

    pub fn get(&self) -> f64 {
        match (self.front.last(), self.back.is_empty()) {
            (Some(&f), true) => f,
            (Some(&f), false) => (self.op)(f, self.back_agg),
            (None, _) => self.back_agg,
        }
    }

    pub fn len(&self) -> usize {
        self.front.len() + self.back.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Move the window over `vals` from range `prev` to range `next`; both
    /// ends must be non-decreasing.
    pub fn slide(&mut self, vals: &[f64], prev: &Range<usize>, next: &Range<usize>) {
        self.slide_map(vals, prev, next, |x| x);
    }

    /// [`SlidingWindow::slide`] over `map(vals[i])`.
    pub fn slide_map(
        &mut self,
        vals: &[f64],
        prev: &Range<usize>,
        next: &Range<usize>,
        map: impl Fn(f64) -> f64,
    ) {
        if next.start >= prev.end {
            *self = SlidingWindow::new(self.op, self.id);
            for &x in &vals[next.clone()] {
                self.push(map(x));
            }
            return;
        }
        for _ in prev.start..next.start {
            self.pop();
        }
        for &x in &vals[prev.end..next.end] {
            self.push(map(x));
        }
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
    Some(finish_rate(result, v[0], &g))
}

fn finish_rate(result: f64, first: f64, g: &RateGeometry) -> f64 {
    let mut to_start = g.to_start;
    let to_zero = if result > 0.0 && first >= 0.0 {
        g.sampled * (first / result)
    } else {
        to_start
    };
    if to_zero < to_start {
        to_start = to_zero;
    }
    result * (g.sampled + to_start + g.to_end) / g.sampled / g.range
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
        Self::from_ends(ts[0], ts[ts.len() - 1], ts.len(), start, end)
    }

    /// Geometry of `n` samples spanning `[first, last]`.
    pub fn from_ends(first: i64, last: i64, n: usize, start: i64, end: i64) -> Self {
        let sampled = (last - first) as f64 / 1000.0;
        let avg = sampled / (n - 1) as f64;
        let threshold = avg * 1.1;
        let mut to_start = (first - start) as f64 / 1000.0;
        let mut to_end = (end - last) as f64 / 1000.0;
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
    fn eval_many_matches_eval() {
        let ts: Vec<i64> = (0..300).map(|i| i * 7_000 + (i * i) % 5_000).collect();
        let vals: Vec<f64> = (0..300)
            .map(|i| {
                if i % 37 == 0 {
                    1.0
                } else {
                    (i as f64 * 0.7).sin() * 100.0 + i as f64
                }
            })
            .collect();
        let probe: Vec<i64> = (-20..400).map(|k| k * 5_500).collect();
        for agg in [
            Agg::Last,
            Agg::AvgOverTime,
            Agg::SumOverTime,
            Agg::MinOverTime,
            Agg::MaxOverTime,
            Agg::Rate,
        ] {
            for range in [1_000, 30_000, 300_000] {
                let r = Rule::new("r", agg, range, Cmp::Gt, 0.0);
                let fast = r.eval_many(&ts, &vals, &probe);
                let sup = r.supports(&ts, &probe);
                for (k, &t) in probe.iter().enumerate() {
                    assert_eq!(sup[k], r.support(&ts, t));
                    let slow = r.eval(&ts, &vals, t);
                    match (fast[k], slow) {
                        (Some(a), Some(b)) => assert!(
                            a == b || (a - b).abs() <= 1e-12 * b.abs().max(1.0),
                            "{agg:?} {range} t={t}: {a} vs {b}"
                        ),
                        (a, b) => assert_eq!(a, b, "{agg:?} {range} t={t}"),
                    }
                }
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
