# Formal model

This document states the model, the correctness criterion, and the results
that the code relies on, with proofs. Section numbers are referenced from the
source (`budget.rs`, `lib.rs`, `alertsafe-rules`).

## 1. Series, rules, decisions

**Series.** A series is a finite sequence $X = ((s_i, x_i))_{i=1}^{n}$ with
strictly increasing timestamps $s_i \in \mathbb{Z}$ (ms) and values
$x_i \in \mathbb{R}$. A *value-only* compressor $C$ keeps timestamps and
changes values: $C(X) = Y = ((s_i, y_i))$, with pointwise error
$e_i = y_i - x_i$.

**Rules.** A threshold rule is $R = (F, w, \bowtie, \theta, h, k)$: an
aggregate $F$, a range $w$, a comparison
$\bowtie \in \{>, \ge, <, \le\}$, a threshold $\theta$, a hold duration $h$
(`for`) and $k$ (`keep_firing_for`). This is the PromQL alert
`expr: F(x[w]) ⋈ θ` over one series.

**Support.** At evaluation time $t$ the aggregate reads the samples with
index in the support

$$
S_R(t) = \{\, i : t - w < s_i \le t \,\}
$$

(range selectors are left-open in Prometheus 3). For an instant selector
(`last`) the support is the single last sample in $(t - \lambda, t]$, with
lookback $\lambda = 5\,\mathrm{min}$.

**Aggregate and condition.**
$f_R(X)(t) = F\big((x_i)_{i \in S_R(t)}\big)$ when $S_R(t) \neq \emptyset$;
the condition is

$$
c_R(X)(t) = \big[\, S_R(t) \neq \emptyset \;\wedge\; f_R(X)(t) \bowtie \theta \,\big].
$$

**Decision operator.** For an evaluation grid $T = (t_1 < t_2 < \dots)$ the
alert state trajectory is

$$
D_R(X) = M_{h,k}\big(T, (c_R(X)(t_j))_j\big) \in \{\mathsf{Inactive}, \mathsf{Pending}, \mathsf{Firing}\}^{|T|}
$$

where $M_{h,k}$ is the deterministic automaton of `rules/alerting.go`:
a true condition moves `Inactive → Pending` (recording `activeAt`),
`Pending → Firing` once $t - \mathit{activeAt} \ge h$; a false condition
resets to `Inactive`, except that a `Firing` alert stays `Firing` while
$t - \mathit{keepFiringSince} < k$. Implemented in
`alertsafe_rules::trajectory`.

## 2. Correctness criterion

**Definition 1 (decision preservation).** $C$ is *decision-preserving* for a
rule set $\mathcal{R}$ on $X$ over grid $T$ if

$$
\forall R \in \mathcal{R}:\; D_R(X) = D_R(C(X)).
$$

It is *phase-robust* if this holds for every grid $T \subset \mathbb{R}$.

**Definition 2 ($\delta$-relaxation).** Let $\mathrm{fires}(D)$ be the set
of firing episodes (maximal runs of `Firing`). $C$ is
*$\delta$-decision-preserving* if there is a bijection between
$\mathrm{fires}(D_R(X))$ and $\mathrm{fires}(D_R(C(X)))$ such that matched
episodes' start times differ by at most $\delta$. Definition 1 is the case
$\delta = 0$ with, additionally, equal `Pending` periods. The code implements
$\delta = 0$; §7 discusses $\delta > 0$.

## 3. From decisions to conditions

**Lemma 1 (automaton).** If $c_R(X)(t) = c_R(Y)(t)$ for all $t \in T$ then
$D_R(X) = D_R(Y)$.

*Proof.* $M_{h,k}$ reads only $T$ and the condition sequence; its transition
function does not depend on values. Equal inputs give equal runs. $\square$

The converse fails: `for` filters short condition flips. Lemma 1 is therefore
where $\delta > 0$ or a hold-aware criterion can buy extra budget (§7).

**Lemma 2 (phase robustness).** Let $F$ depend on $t$ only through $S_R(t)$
(true for `last`, `avg/sum/min/max_over_time`). Let
$E = \{s_i\} \cup \{s_i + w\}$ ($s_i + \lambda$ for `last`), sorted as
$e_1 < e_2 < \dots$. Then $S_R(t)$, hence $f_R(X)(t)$ and $c_R(X)(t)$, are
constant on each $[e_j, e_{j+1})$ and empty for $t < e_1$.

*Proof.* $i \in S_R(t) \iff s_i \le t < s_i + w$, so membership of $i$ can
change only at $t = s_i$ or $t = s_i + w$. $\square$

**Corollary.** For such rules, $c_R(X)(e) = c_R(Y)(e)$ for all $e \in E$
implies equal conditions at every real $t$, hence (Lemma 1) equal
trajectories for every evaluation interval and phase. This is what
`Rule::critical_instants` and `budget::instants` implement. `rate` depends on
$t$ continuously through extrapolation and is protected on the given grid
only.

## 4. Sufficient condition (Theorem 1)

**Margin and tolerance.** For $(R, t)$ with $S_R(t) \neq \emptyset$, the
margin is $m_R(t) = |f_R(X)(t) - \theta|$. Let $L_R(t)$ be a Lipschitz
constant of $F$ on $\mathbb{R}^{|S_R(t)|}$ in the sup norm:

$$
|F(x + e) - F(x)| \le L_R(t)\, \|e\|_\infty .
$$

| $F$ | $L$ | reason |
|---|---|---|
| `last` | 1 | identity on one sample |
| `avg_over_time` | 1 | mean of a perturbation is at most its max |
| `min_over_time`, `max_over_time` | 1 | order statistics are 1-Lipschitz in $\ell_\infty$ |
| `sum_over_time` | $\lvert S_R(t)\rvert$ | triangle inequality, tight |
| `quantile_over_time` | 1 | order statistic (not implemented yet) |

Define the tolerance $\tau_R(t) = m_R(t) / L_R(t)$ and the per-sample budget

$$
\varepsilon_i = \min \{\, \tau_R(t) : R \in \mathcal{R},\ t \in P_R,\ i \in S_R(t) \,\}
\qquad (\min \emptyset = +\infty),
$$

where $P_R$ is the protected instant set ($E$ of Lemma 2, or $T$ for `rate`).

**Theorem 1.** Let the comparison be *tight* at $(R,t)$ when preserving
$c_R(X)(t)$ needs a strict inequality: $c$ true with $\bowtie \in \{>,<\}$, or
$c$ false with $\bowtie \in \{\ge,\le\}$. If for all $i$

$$
|y_i - x_i| \le \varepsilon_i, \quad \text{with } |y_i - x_i| < \tau_R(t)
\text{ whenever } (R,t) \text{ is tight and } i \in S_R(t),
$$

then $c_R(Y)(t) = c_R(X)(t)$ for all $R \in \mathcal{R}$, $t \in P_R$;
consequently $D_R(Y) = D_R(X)$ on every grid (on $T$ for `rate`).

*Proof.* Fix $(R, t)$ with non-empty support ($S_R(t)$ depends on timestamps
only, so emptiness is preserved). By the Lipschitz bound,
$|f_R(Y)(t) - f_R(X)(t)| \le L_R(t) \max_{i \in S_R(t)} |e_i| \le L_R(t)\,\tau_R(t) = m_R(t)$,
strictly when tight. Take $\bowtie = {>}$ (others are symmetric). If
$c$ is true, $f_R(X)(t) = \theta + m$ with $m > 0$ and the bound is strict,
so $f_R(Y)(t) > \theta$. If $c$ is false, $f_R(X)(t) = \theta - m$ and
$f_R(Y)(t) \le \theta$. Lemma 2 and Lemma 1 finish the proof. $\square$

**Remarks.**

1. *Correction to the original formulation.* The draft stated
   $\varepsilon(t) < |f(X)(t) - \theta| / L$ per time point. Errors live on
   samples, margins on evaluations, and one sample feeds every evaluation
   whose window contains it; the correct budget is the minimum over those
   evaluations, as above.
2. *Lookahead.* $\varepsilon_i$ depends on evaluations up to $s_i + w$, so a
   streaming encoder needs a lookahead of $\max_R w_R$. Inline on the write
   path this means buffering one window; at TSDB block compaction (2h blocks)
   the whole future is known. A causal variant must bound the future margin
   and is strictly weaker.
3. *Margin zero.* $m_R(t) = 0$ (value exactly on the threshold) forces exact
   samples; this is the automatic "lossless near the threshold" of the draft.
4. *Floating point.* The code subtracts a rounding slack from $m$
   (Higham's $\gamma_n$ bound for sums) and uses `next_down` for strictness,
   and then verifies the actual reconstruction against the reference
   evaluator, zeroing budgets on any evaluation that still flips
   (verify-and-repair, `lib.rs`). The loop terminates: an evaluation with an
   all-zero-budget support reads identical values.
5. *Multiple rules and series.* Rules compose by the minimum. An aggregate
   across $N$ series (`sum by`) has $L = N$ in the joint sup norm; per-series
   budgets must satisfy $\sum_j \varepsilon^{(j)} \le m$, which couples the
   series and requires the encoder to see the whole group.

## 5. `rate` and counters (Proposition 1)

Prometheus' `extrapolatedRate` over a window with $n \ge 2$ samples, first
value $x_f$, last value $x_l$, reset correction $\rho = \sum_{x_i < x_{i-1}} x_{i-1}$,
increase $\Delta = x_l - x_f + \rho$, sampled interval $S$, range $W$, and
timestamp-only extrapolation distances $d_s, d_e$ (after the 1.1-average
threshold), computes

$$
\mathrm{rate} = \frac{\Delta\,(S + \min(d_s, d_z) + d_e)}{S\,W},
\qquad d_z = S\,\frac{x_f}{\Delta}\ \text{ if } \Delta > 0 \wedge x_f \ge 0,\ \text{else } d_z = +\infty .
$$

**Proposition 1.**

(a) *Resets are discontinuities.* The reset indicator $[x_i < x_{i-1}]$ is
a comparison, so an arbitrarily small error can create or remove a reset and
change $\Delta$ by a whole counter value. No positive symmetric budget
preserves `rate` in general.

(b) *Order-preserving quantizer.* Quantize in closed loop with a floor
step: $y_i = y_{i-1} + q\,\lfloor (x_i - y_{i-1})/q \rfloor$, falling back to
the exact value when the step count is negative. Then $y_i \le x_i$,
$x_i - y_i < q$, and $x_i \ge x_{i-1} \Rightarrow y_i \ge y_{i-1}$. Keeping
the two samples around every true reset exact makes the reset set of $Y$
equal to that of $X$.

(c) *Lipschitz on the reset-preserving set.* With the reset set fixed and
$x_f - \varepsilon \ge 0$, the map $(\Delta, x_f) \mapsto \mathrm{rate}$ is
Lipschitz, because $\Delta\min(d_s, d_z) = \min(\Delta d_s, S x_f)$ and
$|\min(a,b) - \min(a',b')| \le \max(|a-a'|, |b-b'|)$. With $r$ resets in the
window, $|\delta\Delta| \le (2 + r)\varepsilon$ and $|\delta x_f| \le \varepsilon$, so

$$
L_{\mathrm{rate}} = \frac{(2 + r)(S + d_e) + \max\big((2 + r)\, d_s,\ S\big)}{S\,W} \approx \frac{2 + r}{W}\ \text{ for densely sampled windows.}
$$

(d) *Residual discontinuity.* At $x_f = 0$ the zero-point branch switches
off: if $y_f < 0 \le x_f$ the extrapolation jumps by $\Delta d_s / (S W)$.
This happens only for counters within $\varepsilon$ of zero.

*Proof sketch.* (a) Example: $x = (10, 20, 20)$ has no reset; $y = (10, 20, 20 - \eta)$
has one and $\Delta$ jumps from $10$ to $\approx 30$ for any $\eta > 0$.
(b) By induction: $x_i - y_{i-1} \ge x_{i-1} - y_{i-1} \ge 0$, so the step
count is non-negative, and floor gives the error bound. (c) Direct from the
formula and the min inequality. (d) Direct. $\square$

The implementation (`budget::rate_enclosure`) does not use the global
constant; it computes a sound interval enclosure of the formula over the box
$y_i \in [x_i - \varepsilon, x_i]$ (one-sided, thanks to (b)), covering the
branch in (d), and finds the largest safe $\varepsilon$ by bisection. This is
tighter than (c) and handles the discontinuity.

## 6. Where no positive budget exists (Proposition 2)

For a single evaluation, a positive budget exists at $x$ iff $x$ lies in the
interior of $\{\,z : c(z) = c(x)\,\}$ in $\ell_\infty$. It fails whenever
the condition's boundary passes through $x$:

| construct | why | status |
|---|---|---|
| `==`, `!=` on floats | condition set has empty interior at firing points | exact only |
| `changes()`, `resets()` | count equalities/orderings of adjacent samples; constant input sits on the boundary | exact, or order-preserving quantizer for `resets` |
| `floor`, `round`, `ceil` inside `expr` | discontinuous $F$ | exact near jump points |
| `topk`, `bottomk`, label-selecting aggregations | which series is selected is discontinuous | out of scope |
| `histogram_quantile` | continuous piecewise-linear in bucket counts except where the rank lands on an empty bucket | needs separate analysis |
| ratios $a/b$ | locally Lipschitz only: $\lvert a'/b' - a/b\rvert \le (\varepsilon_a + \lvert a/b\rvert \varepsilon_b)/(\lvert b\rvert - \varepsilon_b)$ for $\varepsilon_b < \lvert b\rvert$ | interval enclosure, like `rate` |
| `absent()`, `absent_over_time()` | depends on timestamps only | trivially preserved |
| boolean gauges (`up == 0`) | values in {0,1}, threshold 0 or 1 → margin 0 | exact (but cheap) |

*Proof of the criterion.* If $x$ is interior, some $\ell_\infty$ ball
$B(x, r)$ with $r > 0$ stays in the set, and $\varepsilon = r$ works. If not,
every ball contains a $z$ with $c(z) \ne c(x)$, and a compressor that happens
to output $z$ violates the criterion. $\square$

## 7. Beyond Theorem 1 (open, planned)

1. **Hold-aware budget.** Lemma 1 is only sufficient: with `for: h`, a
   condition may flip on a `Pending` run that never reaches $h$ without
   changing $D_R$ (it changes `Pending` periods, which Definition 1 counts,
   but not firings). Under a "firings only" criterion the budget on such
   runs can exceed the margin.
2. **$\delta > 0$.** Allowing firing times to move by $\delta$ lets the
   encoder spend budget near crossings. A natural candidate is a tolerance
   defined over $[t - \delta, t + \delta]$; the difficulty is that shifted
   crossings interact with $h$ and $k$, so the bijection in Definition 2 must
   be argued on the automaton, not per evaluation.
3. **Tightness.** The budget is per-evaluation uniform over the support. An
   optimal allocation solves, for each evaluation, a knapsack-like problem
   $\sum_i a_i \varepsilon_i \le m$ (for linear $F$) that trades error
   between samples; the min-composition above is a feasible but not optimal
   point.

## 8. What the hypotheses reduce to

Let $b(\varepsilon)$ be the bits per sample of the codec at error
$\varepsilon$. For DPCM on a grid $q = 2\varepsilon$, high-resolution theory
gives $b(\varepsilon) \approx h(\text{innovation}) - \log_2 q$.

**H1.** Any uniform bound with the same guarantee must satisfy
$\varepsilon_u \le \min_i \varepsilon_i$. If an alert ever fires, $f$ crosses
$\theta$ between two consecutive protected instants, so
$\min_i \varepsilon_i$ is at most the per-event change of $f$ (a single
sample's contribution, e.g. $\sigma / |S|$ for an average): the uniform
guaranteed bound is pinned to the noise floor. The predicted saving is

$$
b(\varepsilon_u) - \overline{b(\varepsilon_i)} \approx \frac{1}{n} \sum_i \log_2 \frac{\varepsilon_i}{\varepsilon_u}.
$$

So H1 against a *guaranteed* uniform bound is close to true by construction
whenever alerts fire; the informative comparison is against a uniform bound
*tuned on the data* (no guarantee), which the benchmark reports separately.
The high-resolution formula over-predicts at low rates (below ~2 bits per
sample, where fixed-width codes saturate at about 1 bit per sample).

**H2.** With $\varepsilon_i \propto m_R$, the per-sample rate
$\max(0, h - \log_2 2\varepsilon_i)$ decreases with the margin, so the rate
falls (and the gain over lossless grows) as the share of time spent near
$\theta$ shrinks. This is a monotonicity statement that the benchmark checks
by sweeping $\theta$ and the incident frequency.

**H3.** The codec itself is linear-time. Computed directly, the budget costs
$O(\sum_{t} |S_R(t)|)$. Because the instants are sorted and both ends of
$S_R(t)$ are non-decreasing in $t$, it costs $O(n + |P_R|)$ per rule: the
supports follow by two pointers, `sum/avg/min/max` over a sliding support by
the two-stacks queue aggregate (amortised $O(1)$; its summation tree keeps
the $(|S| - 1)u\sum|x_i|$ rounding bound used for the slack), and the
instants whose support contains sample $i$ form a contiguous run
$[a_i, b_i)$ with $a_i, b_i$ non-decreasing, so
$\varepsilon_i = \min_{k \in [a_i, b_i)} \tau_k$ is a sliding minimum
(monotone deque). `rate` adds a bisection per instant, seeded with the
Lipschitz estimate of Proposition 1(c) and stopped at relative precision
$2^{-10}$, about a dozen enclosure evaluations. Remark 2 of §4 implies that
the natural deployment point is compaction or a buffered proxy, not a
zero-latency inline hop.
