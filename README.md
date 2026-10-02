# alert-preserving-compression

**Alert-preserving lossy compression of monitoring metrics: formal model,
algorithm, and Rust implementation.**

Bachelor's thesis project (Software Engineering, HSE University, defence in
2027). Status: early prototype. The formal core and the codec work end to end
on synthetic data; real traces, the remote-write proxy, and the Prometheus
oracle are next.

## The question

Can metrics be compressed lossily so that alerting rules make **exactly the
same decisions** on the reconstruction as on the original data?

Lossless float compressors (Gorilla, Chimp, Elf, ALP) are close to their
limit. Lossy compressors give many times more, but SRE teams avoid them
because nothing tells them whether an alert will go missing or appear out of
nowhere. Existing lossy methods bound the *values* (pointwise error, ACF,
topology). This project bounds the *decisions*: the alert state trajectory
(`Inactive` / `Pending` / `Firing`, with `for` and `keep_firing_for`) must
not change.

The key observation: a rule only needs precision where its aggregate is close
to the threshold. Far from the threshold the data can be quantized very
coarsely. Near it, the budget drops to zero on its own.

## Formal core

Full statements and proofs: [`docs/formal-model.md`](docs/formal-model.md).

**Model.** A series is $X = ((s_i, x_i))_{i=1}^n$ with strictly increasing
timestamps. A value-only compressor keeps the timestamps and outputs
$Y = ((s_i, y_i))$. A rule $R = (F, w, \bowtie, \theta, h, k)$ is the PromQL
alert `F(x[w]) ⋈ θ` with `for: h` and `keep_firing_for: k`. At time $t$ it
reads the support $S_R(t) = \{\, i : t - w < s_i \le t \,\}$. Its condition is

$$
c_R(X)(t) = \big[\, S_R(t) \ne \emptyset \wedge F(x_{S_R(t)}) \bowtie \theta \,\big],
$$

and its decision $D_R(X) = M_{h,k}(T, (c_R(X)(t))_{t \in T})$ is the run of
the Prometheus alert automaton over the evaluation grid $T$.

**Criterion.** $C$ is *decision-preserving* for a rule set $\mathcal{R}$ if
$D_R(X) = D_R(C(X))$ for all $R \in \mathcal{R}$. It is *phase-robust* if
this holds for every grid $T$. A $\delta$-relaxation matches firing
episodes up to a time shift $\delta$.

**Lemma 1 (automaton).** Equal conditions on $T$ imply equal trajectories:
the automaton never reads values.

**Lemma 2 (phase robustness).** For `last` and `*_over_time`, $c_R(X)(t)$ is
piecewise constant between the critical instants
$E = \{s_i\} \cup \{s_i + w\}$. Protecting $E$ protects every evaluation
interval and phase.

**Theorem 1 (sufficient condition).** Let $F$ be $L$-Lipschitz in the sup
norm on its support ($L = 1$ for `last`, `avg/min/max_over_time`; $L = |S|$
for `sum_over_time`). Let $m_R(t) = |F(x_{S_R(t)}) - \theta|$ be the margin
and let the per-sample budget be

$$
\varepsilon_i = \min_{R \in \mathcal{R}}\ \min_{t \in P_R :\ i \in S_R(t)} \frac{m_R(t)}{L_R(t)} .
$$

If $|y_i - x_i| \le \varepsilon_i$ for all $i$ (strictly where the
comparison must stay strict), then every condition, and therefore every
trajectory, is preserved.

This corrects a first draft that bounded the error "at time $t$" by
$m(t)/L$. Errors live on samples and margins on evaluations. One sample feeds
every evaluation whose window contains it, so the budget is a minimum over
overlapping windows. Two consequences follow. An encoder needs a lookahead of
one window. And the budget is zero exactly where some window sits on the
threshold.

**Proposition 1 (`rate` and counters).** Counter-reset detection is a
comparison, so no symmetric error bound preserves `rate`. A closed-loop floor
quantizer ($y_i \le x_i$, order-preserving), together with exact samples
around true resets, keeps the reset set unchanged. On that set `rate` is
Lipschitz with $L \approx (2 + r)/W$. A discontinuity remains at a first
value of $0$ (Prometheus' zero-point extrapolation). The code uses a sound
interval enclosure of the exact Prometheus formula instead of the global
constant.

**Proposition 2 (limits).** A positive budget exists iff the input lies in
the interior of its condition's level set. Such a budget does not exist for
`==`, `changes()`, `floor/round`, or `topk`-style selection. It exists for
ratios and `histogram_quantile` only locally. `absent()` is preserved
trivially.

**What the hypotheses reduce to** (formal model §8). If an alert fires, the
largest *guaranteed* uniform bound is pinned at the noise floor. So H1
against it holds almost by construction, and the informative baseline is a
uniform bound *tuned* on the data. High-resolution quantization theory
predicts the saving as $\frac{1}{n}\sum_i \log_2(\varepsilon_i/\varepsilon_u)$
bits per sample. H2 is a monotonicity statement in the margin distribution.

## Algorithm

1. **Budget** (`alertsafe-codec::budget`). For every rule, protected instant
   $t$ ($E$ from Lemma 2, or the grid for `rate`) and support, compute the
   tolerance $m/L$, minus a floating-point slack. For `rate`, bisect over an
   interval enclosure, seeded with the Lipschitz estimate. Take the minimum
   per sample. In counter mode, keep samples around resets exact. Supports
   are monotone in $t$, so window aggregates slide (two-stacks queue) and the
   per-sample minimum is a sliding minimum over instants: $O(n)$ per rule.
2. **Codec** (`alertsafe-codec::codec`). Blocks of 128 samples. For each
   block, pick the cheapest of lossless Gorilla XOR and closed-loop DPCM on a
   grid $q = 2^e$ (rounding for gauges, floor for counters). DPCM codes
   either the step counts or their deltas, with an escape to raw float64.
   Every decision and residual is coded with an adaptive binary range coder
   (`alertsafe-codec::rc`; zero flag, sign and unary bit length under a
   context of the previous residual's magnitude), and block plans are
   compared by their exact adaptive cost. A fixed-width format remains for
   ablation. The encoder checks every reconstructed value against its
   budget, so the bound holds for the actual floating-point output.
3. **Verify and repair** (`alertsafe-codec::compress`). Decode, re-evaluate
   every protected condition with the reference semantics
   (`alertsafe-rules`), and zero the budget on the support of any condition
   that flipped. The analytic budget has needed no repairs so far. The loop
   makes the guarantee end to end with respect to the reference evaluator.

## Results so far (synthetic, 7 days at 15 s scrape)

Bits per value, timestamps excluded. Every rule-aware row has zero changed
decisions. Full tables, sweeps and caveats: [`docs/results.md`](docs/results.md).

| dataset | rules | lossless (Gorilla XOR) | uniform, guaranteed | uniform, tuned on data | **rule-aware** |
|---|---|---:|---:|---:|---:|
| cpu | `avg_over_time[5m] > 0.8 for 10m`, `max_over_time[1m] > 0.97 for 2m` | 53.96 | 11.59 | 2.55 | **0.36** |
| latency | `avg_over_time[5m] > 0.5 for 5m`, `x > 1 for 2m keep_firing_for 5m` | 55.24 | 7.23 | 5.20 | **0.23** |
| memory | `x > 4.5e9 for 15m`, `avg_over_time[30m] > 4e9 for 30m` | 49.91 | 12.98 | 6.94 | **0.74** |
| requests | `rate[5m] > 700 for 5m`, `rate[5m] < 20 for 10m` | 19.68 | 2.18 | 2.18 | **0.49** |

All columns share the entropy-coded format. The synthetic data support H1
(4.5 to 22× over a uniform bound tuned on the data, 4.5 to 32× over the
guaranteed one) and H2 (the rate grows with time spent near thresholds).
Entropy coding cut the rule-aware rate 3 to 6× against fixed-width codes
(1.4 to 2.2 bits per value). For H3, decode runs at about 2.5 GB/s and
encode at 110 to 200 MB/s per core. The $O(n)$ budget runs at about 70 MB/s
and the full verified `compress` at 25 to 45 MB/s. None of this counts as
evidence until real traces and open rule sets are in.

## Novelty and positioning

The search is summarised in [`docs/related-work.md`](docs/related-work.md). No work found uses equality of
alert-state trajectories as the correctness criterion for lossy compression.

The closest work is **Compression Safeguards** (Tyree et al., 2026). Its
`sign` and `qoi_eb_stencil` safeguards can express a single window-average
threshold test on a regular grid. It has no temporal alert semantics, no
irregular timestamps or phase robustness, and no PromQL `rate` or counter
resets, and it enforces requirements by corrections rather than an analytic
budget. It will be a baseline.

The Lipschitz step itself is standard quantity-of-interest error propagation
(Liu et al., PVLDB 2022). The contribution lies in the problem statement, the
temporal and PromQL-specific analysis, the applicability boundary, and the
evaluation.

## Repository layout

```
crates/
  alertsafe-rules/   decision operator D_R: supports, aggregates, Prometheus rate,
                     alert automaton, critical instants, trajectory diff
  alertsafe-codec/   budget (Theorem 1, Prop. 1), block codec, range coder,
                     verify-and-repair; tests/decisions.rs: property tests of
                     the invariant; tests/codec.rs: round-trip properties
  alertsafe-bench/   seeded synthetic datasets and experiments for H1-H3
docs/
  formal-model.md    definitions, lemmas, theorem, propositions, proofs
  related-work.md    novelty check and positioning
  results.md         benchmark output
```

## Usage

```sh
cargo test --release                              # unit + property tests (10000 random cases)
cargo run --release -p alertsafe-bench -- all 7   # H1, H2, H3 on 7 synthetic days
```

```rust
use alertsafe_codec::{compress, decode};
use alertsafe_rules::{grid, Agg, Cmp, Rule, Series};

let series = Series::new(ts, vals);
let rules = [Rule::new("HighCPU", Agg::AvgOverTime, 300_000, Cmp::Gt, 0.8).hold(600_000)];
let eval = grid(ts[0], *ts.last().unwrap(), 30_000);
let c = compress(&series, &rules, &eval);
let restored = decode(&c.encoded.bytes);   // same alert trajectories as `series`
```

## Roadmap

| Period | Deliverable |
|---|---|
| Sep–Oct 2026 | Literature review, positioning against Compression Safeguards and QoI work, topic approval |
| Nov–Dec 2026 | Proofs finalised; hold-aware and δ-relaxed budgets; `quantile_over_time`, ratios, `sum by` across series |
| Jan–Feb 2027 | ~~O(n) sliding-window budget~~, ~~entropy-coded residuals~~ (done Oct 2026); remote-write proxy (tokio); oracle on a reference Prometheus via remote-read |
| Mar 2027 | Real traces (cluster metrics, Alibaba/Google traces, AIOps KPI sets) with kube-prometheus and Awesome Prometheus Alerts rules; baselines: Gorilla, Chimp, ALP, SZ3, Serf, CAMEO, downsampling, Compression Safeguards |
| Apr–Jun 2027 | Thesis text, workshop paper, defence |

## Hypotheses

- **H1.** With zero changed firings, the rule-aware budget compresses
  several times better than a uniform error bound with the same guarantee.
  The thesis will also compare against a uniform bound tuned on the data.
- **H2.** The gain grows with the share of time a series spends far from
  its thresholds.
- **H3.** The codec is fast enough for the metrics write path. Because the
  budget needs one window of lookahead, the realistic deployment points are
  a buffered remote-write proxy or TSDB block compaction.

## License

MIT. Author: Bogdan Serykh.
