# Results (synthetic data)

Produced by `cargo run --release -p alertsafe-bench -- h1|h2|h4|h5|h7 7`
(in parallel) and `h3 7` alone on one core of a desktop CPU (16 cores,
Linux 6.18, rustc 1.93); H6: see its section. Data are seeded synthetic
series from `crates/alertsafe-bench/src/gen.rs`; they are placeholders for
real cluster traces. Treat the numbers as a smoke test of the method, not as
evidence for the thesis. Real Prometheus series:
[`results-real.md`](results-real.md); ablation of the codec components:
[`ablation.md`](ablation.md).

Regenerated 2026-11-15 for the one-sided budget and lazy step choice
(commit `35ba3da`; Corollary 1 and Proposition 1(e) of `formal-model.md`).
The SZ3+SG side of H6 was rerun and is unchanged. Previous version
(2026-11-02, box budget): rule-aware 0.22–0.70 bits per value on the four
original datasets, 4.68 on `requests-gappy`.

**Reading the numbers.**

- All columns use the same entropy-coded format (adaptive binary range
  coding of block decisions and DPCM residuals, `alertsafe-codec::rc`), so
  baselines and the rule-aware codec differ only in the error budget.
  `rule-aware (fixed)` is the ablation with fixed-width residual codes. The
  uniform sweep takes, per point, the smaller safe output of nearest and
  lazy step choice (lazy biases errors, which a symmetric bound does not
  account for); H4/H5 reuse the step choice of the tuned point.
- H1 (one stream per series): rule-aware 0.07–0.21 bits per value on the
  gauges, 33–36× below a uniform bound tuned on the same data (which has no
  guarantee elsewhere) and 61–158× below the largest uniform bound with the
  same guarantee. On the counters the gain is 1.7× (`requests`) and 3.2×
  (`requests-gappy`); the tuned uniform bound gained from the lazy step
  choice too (`requests` 2.17 → 0.81).
- Entropy coding removes the ~1 bit/value floor of fixed-width codes:
  1.30–6.73 → 0.07–2.92 bits per value. The high-resolution prediction
  of the gain over uniform-safe (formal model, §8) over-predicts by 2.3–3.3
  bits on gauges and 6.7 on `requests`.
- H2: the rule-aware rate grows with the share of samples near the threshold
  (0.01 → 0.66 bits per value as `near` goes 0.1% → 10.2%); the gain over
  lossless falls from ~4 800× to 81×. The gain over the tuned uniform bound
  is noisy because the tuned bound jumps between sweep points.
- H3: decode 2.4–3.0 GB/s (fixed: 3.6 GB/s). Encode 30–32 MB/s (fixed:
  40–46): every grid is tried with four step strategies (it was 46–68 MB/s
  with two). The `O(n)` budget runs at 46–72 MB/s. The end-to-end verified
  `compress` runs at 4–6 MB/s, because by default it encodes four scalings
  of the budget (`Options::scales`); without that search it runs at
  14–20 MB/s, and the ablation shows the search no longer changes the size
  (smaller on 1 of 31 datasets by ≤ 0.006 bits per value).
- H4: in 120-sample chunks (Prometheus) the rule-aware rate is 0.29–0.48
  on gauges, 0.97 on `requests`, 3.64 on `requests-gappy` (whole series:
  0.07–2.92), 1.7–16× below the tuned uniform bound under the same
  chunking. Chunking costs the rule-aware codec relatively more because its
  payload is so small (fixed 3–4 bytes and model warm-up per chunk).
- H5: the uncapped reconstruction is only good for the rules, now more so:
  with lazy steps a sample free in one direction drifts until a bound stops
  it (RMSE 39% / 84% of the range on `cpu` / `memory`, max error up to 3.5×
  the range). Capping at the tuned uniform ε (`Options::max_error`) costs
  within 0–2% of that uniform bound (on `cpu` the cap is 14% cheaper:
  lazy steps). The tuned uniform bound, applied in 120-sample chunks
  instead of the whole-series encoding it was tuned on, changes decisions
  on `memory` (3 states) and `requests` (1): a bound tuned on one encoding
  does not even transfer to the same data. At ε = 10⁻³ of the range the
  uniform bound changes 17, 1020 and 10 655 states on `memory`,
  `requests`, `requests-gappy`; the capped rule-aware codec keeps every
  decision for 0.01–0.07 extra bits per value (and costs 3.64 instead of
  1.18 on `requests-gappy`, where the uniform bound is far from safe).
- The analytic budget needed zero repairs on every dataset and chunk size.
- H6 (Compression Safeguards): SZ3+SG still keeps every decision at every
  sweep point on the four original datasets, but now the **analytic budget
  alone is smaller on all five** (0.07 vs 0.16, 0.08 vs 0.20, 0.21 vs 0.57,
  0.47 vs 0.69, 2.92 vs 8.45). It was larger on the three gauges with the
  box budget; the one-sided budget and lazy steps removed the gap. κ = 1024
  is 1.5–4.1× below SZ3+SG. On `requests-gappy` SG's `rate` translation
  (no extrapolation) changes decisions at 11 of 13 sweep points. SG takes
  1–24 s per sweep point on 40k samples.
- H7: κ > 1 now gains little over the analytic budget (0–33%; κ = 1024:
  0.05–0.46 and 2.72 on `requests-gappy`), because the one-sided budget
  already leaves most samples free in one direction. Rounds grow with κ
  (up to 22, and 75 on `requests-gappy`). κ = 2–4 captures most of the
  gain with ≤ 15 rounds.
- The verifier must not trust its own summation order (see
  `lib.rs::flipped`): an optimistic budget can land a `sum/avg` evaluation
  within an ulp of θ, where summation orders disagree. The analytic budget
  keeps twice the bound on that difference and never trips it.
- `requests-gappy` (8% dropped scrapes, outages of 1–6 min, traffic around
  the threshold: 157 firings in 7 days) is the hard case: 2.92 bits per
  value, 3.2× below the tuned uniform bound, because most evaluations are
  near θ.

## H1: rule-aware vs. uniform error bound (7 days, 15s scrape, 30s eval)

All rule-aware rows have zero changed decisions (asserted). `uniform-safe` is the largest uniform bound
that satisfies the same sufficient condition; `uniform-tuned` is the largest uniform bound with zero
state mismatches found by sweeping on the same data (no guarantee elsewhere).

| dataset | firings | lossless | uniform-safe | uniform-tuned | rule-aware | rule-aware (fixed) | ×lossless | ×safe | ×tuned | predicted Δbits (safe→aware) | measured Δbits | exact samples | repairs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 8 | 53.96 | 11.57 | 2.55 | 0.07 | 1.31 | 739.0 | 158.4 | 34.9 | 13.81 | 11.49 | 0.00% | 0 |
| latency | 11 | 55.24 | 7.22 | 2.81 | 0.08 | 1.30 | 710.2 | 92.8 | 36.2 | 10.43 | 7.14 | 0.00% | 0 |
| memory | 9 | 49.91 | 12.96 | 6.92 | 0.21 | 1.62 | 236.4 | 61.4 | 32.8 | 15.62 | 12.75 | 0.00% | 0 |
| requests | 4 | 19.68 | 2.02 | 0.81 | 0.47 | 2.17 | 42.3 | 4.3 | 1.7 | 8.28 | 1.55 | 0.01% | 0 |
| requests-gappy | 157 | 21.76 | 11.46 | 9.46 | 2.92 | 6.73 | 7.5 | 3.9 | 3.2 | 12.58 | 8.54 | 0.02% | 0 |

Values are bits per value (timestamps excluded). Raw float64 is 64 bits. All columns except
`rule-aware (fixed)` use the entropy-coded format.

<details><summary>Uniform-bound sweep, <code>cpu</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 0.0e0 | 53.96 | 0 | 0 | 0 |
| 1.5e-11 | 32.59 | 0 | 0 | 0 |
| 5.8e-11 | 30.59 | 0 | 0 | 0 |
| 2.3e-10 | 28.59 | 0 | 0 | 0 |
| 9.3e-10 | 26.58 | 0 | 0 | 0 |
| 3.7e-9 | 24.58 | 0 | 0 | 0 |
| 1.5e-8 | 22.58 | 0 | 0 | 0 |
| 6.0e-8 | 20.58 | 0 | 0 | 0 |
| 2.4e-7 | 18.57 | 0 | 0 | 0 |
| 9.5e-7 | 16.57 | 0 | 0 | 0 |
| 3.8e-6 | 14.57 | 0 | 0 | 0 |
| 1.5e-5 | 12.57 | 0 | 0 | 0 |
| 6.1e-5 | 10.56 | 0 | 0 | 0 |
| 2.4e-4 | 8.55 | 0 | 0 | 0 |
| 9.8e-4 | 6.51 | 0 | 0 | 0 |
| 3.9e-3 | 4.52 | 0 | 0 | 0 |
| 1.6e-2 | 2.55 | 0 | 0 | 0 |
| 6.2e-2 | 0.47 | 22 | 0 | 30 |
| 2.5e-1 | 0.03 | 216 | 4 | 900 |
| 1.0e0 | 0.01 | 220 | 8 | 0 |

</details>

<details><summary>Uniform-bound sweep, <code>latency</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 0.0e0 | 55.24 | 0 | 0 | 0 |
| 1.5e-11 | 31.26 | 0 | 0 | 0 |
| 5.8e-11 | 29.26 | 0 | 0 | 0 |
| 2.3e-10 | 27.26 | 0 | 0 | 0 |
| 9.3e-10 | 25.25 | 0 | 0 | 0 |
| 3.7e-9 | 23.25 | 0 | 0 | 0 |
| 1.5e-8 | 21.25 | 0 | 0 | 0 |
| 6.0e-8 | 19.24 | 0 | 0 | 0 |
| 2.4e-7 | 17.24 | 0 | 0 | 0 |
| 9.5e-7 | 15.24 | 0 | 0 | 0 |
| 3.8e-6 | 13.24 | 0 | 0 | 0 |
| 1.5e-5 | 11.23 | 0 | 0 | 0 |
| 6.1e-5 | 9.22 | 0 | 0 | 0 |
| 2.4e-4 | 7.19 | 0 | 0 | 0 |
| 9.8e-4 | 5.08 | 0 | 0 | 0 |
| 3.9e-3 | 2.81 | 0 | 0 | 0 |
| 1.6e-2 | 0.82 | 4 | 1 | 0 |
| 6.2e-2 | 0.05 | 32 | 3 | 60 |
| 2.5e-1 | 0.01 | 130 | 6 | 90 |
| 1.0e0 | 0.01 | 257 | 11 | 0 |

</details>

<details><summary>Uniform-bound sweep, <code>memory</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 0.0e0 | 49.91 | 0 | 0 | 0 |
| 1.5e-11 | 30.98 | 0 | 0 | 0 |
| 5.8e-11 | 28.98 | 0 | 0 | 0 |
| 2.3e-10 | 26.98 | 0 | 0 | 0 |
| 9.3e-10 | 24.98 | 0 | 0 | 0 |
| 3.7e-9 | 22.97 | 0 | 0 | 0 |
| 1.5e-8 | 20.97 | 0 | 0 | 0 |
| 6.0e-8 | 18.97 | 0 | 0 | 0 |
| 2.4e-7 | 16.96 | 0 | 0 | 0 |
| 9.5e-7 | 14.96 | 0 | 0 | 0 |
| 3.8e-6 | 12.96 | 0 | 0 | 0 |
| 1.5e-5 | 10.95 | 0 | 0 | 0 |
| 6.1e-5 | 8.94 | 0 | 0 | 0 |
| 2.4e-4 | 6.92 | 0 | 0 | 0 |
| 9.8e-4 | 4.83 | 21 | 0 | 240 |
| 3.9e-3 | 2.61 | 27 | 0 | 30 |
| 1.6e-2 | 0.27 | 259 | 0 | 1980 |
| 6.2e-2 | 0.02 | 1196 | 0 | 5040 |
| 2.5e-1 | 0.01 | 1688 | 5 | 2940 |
| 1.0e0 | 0.00 | 5311 | 9 | 0 |

</details>

<details><summary>Uniform-bound sweep, <code>requests</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 0.0e0 | 19.68 | 0 | 0 | 0 |
| 1.5e-11 | 19.37 | 0 | 0 | 0 |
| 5.8e-11 | 17.76 | 0 | 0 | 0 |
| 2.3e-10 | 15.77 | 0 | 0 | 0 |
| 9.3e-10 | 13.77 | 0 | 0 | 0 |
| 3.7e-9 | 11.78 | 0 | 0 | 0 |
| 1.5e-8 | 9.78 | 0 | 0 | 0 |
| 6.0e-8 | 7.79 | 0 | 0 | 0 |
| 2.4e-7 | 5.73 | 0 | 0 | 0 |
| 9.5e-7 | 3.61 | 0 | 0 | 0 |
| 3.8e-6 | 1.78 | 0 | 0 | 0 |
| 1.5e-5 | 0.81 | 0 | 0 | 0 |
| 6.1e-5 | 0.59 | 6 | 0 | 30 |
| 2.4e-4 | 0.48 | 16 | 0 | 0 |
| 9.8e-4 | 0.44 | 1031 | 0 | 270 |
| 3.9e-3 | 0.18 | 11295 | 166 | 210 |
| 1.6e-2 | 0.07 | 20135 | 229 | 1740 |
| 6.2e-2 | 0.04 | 20163 | 94 | 2850 |
| 2.5e-1 | 0.02 | 20189 | 44 | 29010 |
| 1.0e0 | 0.01 | 20189 | 2 | 151050 |

</details>

<details><summary>Uniform-bound sweep, <code>requests-gappy</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 0.0e0 | 21.76 | 0 | 0 | 0 |
| 1.5e-11 | 21.31 | 0 | 0 | 0 |
| 5.8e-11 | 19.46 | 0 | 0 | 0 |
| 2.3e-10 | 17.46 | 0 | 0 | 0 |
| 9.3e-10 | 15.46 | 0 | 0 | 0 |
| 3.7e-9 | 13.46 | 0 | 0 | 0 |
| 1.5e-8 | 11.45 | 0 | 0 | 0 |
| 6.0e-8 | 9.46 | 0 | 0 | 0 |
| 2.4e-7 | 7.44 | 2 | 0 | 0 |
| 9.5e-7 | 5.35 | 8 | 0 | 60 |
| 3.8e-6 | 3.28 | 66 | 3 | 60 |
| 1.5e-5 | 2.28 | 404 | 8 | 3600 |
| 6.1e-5 | 1.40 | 1025 | 14 | 7020 |
| 2.4e-4 | 0.83 | 4364 | 78 | 15630 |
| 9.8e-4 | 0.58 | 13172 | 123 | 15120 |
| 3.9e-3 | 0.29 | 19745 | 157 | 12750 |
| 1.6e-2 | 0.12 | 25236 | 490 | 5250 |
| 6.2e-2 | 0.07 | 26187 | 278 | 4920 |
| 2.5e-1 | 0.04 | 26513 | 182 | 20040 |
| 1.0e0 | 0.02 | 26793 | 126 | 242190 |

</details>

## H2: gain vs. distance from the threshold (`cpu`, rule `HighCPU` only, 7 days)

`near` is the share of samples whose budget is below 0.01 (i.e. the 5m average is within 0.01 of θ).

| θ | incidents/day | firings | near | lossless | uniform-tuned | rule-aware | ×lossless | ×tuned |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0.55 | 0.5 | 10 | 7.6% | 53.92 | 7.55 | 0.52 | 104.3 | 14.6 |
| 0.65 | 0.5 | 1 | 0.6% | 53.92 | 3.51 | 0.04 | 1218.6 | 79.3 |
| 0.75 | 0.5 | 1 | 0.2% | 53.92 | 0.45 | 0.02 | 3529.1 | 29.2 |
| 0.85 | 0.5 | 1 | 0.1% | 53.92 | 0.06 | 0.01 | 4852.5 | 5.1 |
| 0.95 | 0.5 | 1 | 0.1% | 53.92 | 7.55 | 0.01 | 4313.3 | 604.2 |
| 0.55 | 2 | 20 | 8.3% | 53.75 | 9.55 | 0.56 | 95.6 | 17.0 |
| 0.65 | 2 | 12 | 2.0% | 53.75 | 5.53 | 0.14 | 391.4 | 40.3 |
| 0.75 | 2 | 10 | 1.7% | 53.75 | 3.52 | 0.12 | 447.0 | 29.3 |
| 0.85 | 2 | 7 | 1.1% | 53.75 | 1.70 | 0.08 | 692.8 | 22.0 |
| 0.95 | 2 | 4 | 0.9% | 53.75 | 1.70 | 0.06 | 915.1 | 29.0 |
| 0.55 | 6 | 39 | 10.2% | 53.44 | 9.52 | 0.66 | 81.0 | 14.4 |
| 0.65 | 6 | 25 | 4.3% | 53.44 | 5.53 | 0.29 | 182.2 | 18.9 |
| 0.75 | 6 | 21 | 3.5% | 53.44 | 3.54 | 0.25 | 212.6 | 14.1 |
| 0.85 | 6 | 12 | 2.8% | 53.44 | 5.53 | 0.19 | 286.2 | 29.6 |
| 0.95 | 6 | 7 | 1.6% | 53.44 | 1.72 | 0.10 | 543.0 | 17.4 |

## H3: single-core throughput (MB/s of raw float64 input, best of 5)

| dataset | samples | budget (naive) | budget | encode (rule-aware) | decode | encode (fixed) | decode (fixed) | encode (lossless) | end-to-end `compress` | `compress`, no scale search |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 40320 | 18 | 46 | 30 | 3038 | 40 | 3592 | 388 | 4 | 14 |
| requests | 40320 | 35 | 72 | 32 | 2373 | 46 | 3557 | 478 | 6 | 20 |

## H4: independently decodable chunks (7 days)

Every chunk is a separate stream (own header, coder flush, models reset to p = 1/2). The budget
is planned and verified on the whole series; `compress` asserts zero changed decisions.

| dataset | chunk | lossless | uniform-tuned | rule-aware | ×lossless | ×tuned | repairs |
|---|---:|---:|---:|---:|---:|---:|---:|
| cpu | 120 | 57.99 | 3.14 | 0.30 | 194.5 | 10.5 | 0 |
| cpu | 480 | 55.11 | 2.74 | 0.16 | 354.7 | 17.6 | 0 |
| cpu | 1920 | 54.29 | 2.60 | 0.09 | 581.0 | 27.8 | 0 |
| cpu | whole | 53.96 | 2.55 | 0.07 | 739.0 | 34.9 | 0 |
| latency | 120 | 58.87 | 3.42 | 0.29 | 203.2 | 11.8 | 0 |
| latency | 480 | 56.25 | 3.01 | 0.18 | 316.8 | 17.0 | 0 |
| latency | 1920 | 55.53 | 2.86 | 0.11 | 511.7 | 26.4 | 0 |
| latency | whole | 55.24 | 2.81 | 0.08 | 710.2 | 36.2 | 0 |
| memory | 120 | 56.85 | 7.71 | 0.48 | 117.8 | 16.0 | 0 |
| memory | 480 | 51.68 | 7.19 | 0.30 | 170.2 | 23.7 | 0 |
| memory | 1920 | 50.32 | 6.99 | 0.23 | 216.4 | 30.1 | 0 |
| memory | whole | 49.91 | 6.92 | 0.21 | 236.4 | 32.8 | 0 |
| requests | 120 | 27.02 | 1.61 | 0.97 | 27.9 | 1.7 | 0 |
| requests | 480 | 21.22 | 1.05 | 0.61 | 34.9 | 1.7 | 0 |
| requests | 1920 | 19.97 | 0.88 | 0.50 | 40.1 | 1.8 | 0 |
| requests | whole | 19.68 | 0.81 | 0.47 | 42.3 | 1.7 | 0 |
| requests-gappy | 120 | 28.68 | 10.69 | 3.64 | 7.9 | 2.9 | 0 |
| requests-gappy | 480 | 23.51 | 9.85 | 3.18 | 7.4 | 3.1 | 0 |
| requests-gappy | 1920 | 22.12 | 9.57 | 3.01 | 7.3 | 3.2 | 0 |
| requests-gappy | whole | 21.76 | 9.46 | 2.92 | 7.5 | 3.2 | 0 |

## H5: signal fidelity (7 days, 120-sample chunks)

Errors are relative to the value range of the series. `rule-aware ≤ ε` caps every sample's
budget at ε on top of the rule budget (`Options::max_error`) and keeps every decision
(asserted); `uniform ε` rows keep decisions only where the mismatch column says so.

| dataset | codec | bits/value | RMSE / range | max error / range | state mismatches |
|---|---|---:|---:|---:|---:|
| cpu | uniform ε = 1.6e-2 | 3.14 | 4.6e-3 | 8.0e-3 | 0 |
| cpu | uniform ε = 1.0e-3 | 6.29 | 5.8e-4 | 9.9e-4 | 0 |
| cpu | rule-aware | 0.30 | 3.9e-1 | 3.5e0 | 0 |
| cpu | rule-aware ≤ 1.6e-2 | 2.71 | 8.4e-3 | 1.6e-2 | 0 |
| cpu | rule-aware ≤ 1.0e-3 | 6.31 | 5.7e-4 | 1.0e-3 | 0 |
| latency | uniform ε = 3.9e-3 | 3.42 | 2.1e-3 | 3.9e-3 | 0 |
| latency | uniform ε = 1.0e-3 | 4.89 | 5.7e-4 | 9.9e-4 | 0 |
| latency | rule-aware | 0.29 | 6.9e-2 | 6.8e-1 | 0 |
| latency | rule-aware ≤ 3.9e-3 | 3.44 | 2.1e-3 | 3.9e-3 | 0 |
| latency | rule-aware ≤ 1.0e-3 | 4.90 | 5.7e-4 | 1.0e-3 | 0 |
| memory | uniform ε = 2.4e-4 | 7.71 | 1.1e-4 | 2.4e-4 | 3 |
| memory | uniform ε = 1.0e-3 | 5.71 | 3.9e-4 | 6.7e-4 | 17 |
| memory | rule-aware | 0.48 | 8.4e-1 | 1.3e0 | 0 |
| memory | rule-aware ≤ 2.4e-4 | 7.76 | 1.1e-4 | 2.4e-4 | 0 |
| memory | rule-aware ≤ 1.0e-3 | 5.74 | 4.9e-4 | 1.0e-3 | 0 |
| requests | uniform ε = 1.5e-5 | 1.61 | 8.6e-6 | 1.5e-5 | 1 |
| requests | uniform ε = 1.0e-3 | 0.93 | 3.1e-4 | 6.8e-4 | 1020 |
| requests | rule-aware | 0.97 | 6.4e-4 | 1.8e-3 | 0 |
| requests | rule-aware ≤ 1.5e-5 | 1.61 | 8.6e-6 | 1.5e-5 | 0 |
| requests | rule-aware ≤ 1.0e-3 | 1.00 | 3.4e-4 | 1.0e-3 | 0 |
| requests-gappy | uniform ε = 6.0e-8 | 10.69 | 2.8e-8 | 5.7e-8 | 0 |
| requests-gappy | uniform ε = 1.0e-3 | 1.18 | 4.2e-4 | 7.5e-4 | 10655 |
| requests-gappy | rule-aware | 3.64 | 6.9e-5 | 7.5e-4 | 0 |
| requests-gappy | rule-aware ≤ 6.0e-8 | 10.70 | 2.8e-8 | 5.7e-8 | 0 |
| requests-gappy | rule-aware ≤ 1.0e-3 | 3.64 | 6.9e-5 | 7.5e-4 | 0 |

## H6: Compression Safeguards baseline (7 days)

Reproduce with `alertsafe-bench export DIR 7`, `uv run baselines/safeguards/run.py DIR`,
`alertsafe-bench h6 DIR 7` (compression-safeguards 1.0.0rc3, numcodecs-safeguards
0.1.0rc3, SZ3 via numcodecs-wasm-sz3 0.8.4). Translation of each rule into
safeguards: module docstring of `run.py`.

`SZ3` is SZ3 with an absolute error bound; `SZ3+SG` wraps it in safeguards translated from the rules
(see `baselines/safeguards/run.py`); both are swept over the SZ3 bound. `tuned` = largest bound with no
changed decision and none below it; `SZ3+SG best` = smallest output with zero state mismatches, if any.
`rule-aware κ` is the optimistic budget with a posteriori repair (H7).
Mismatches are counted by the reference evaluator of this repository.

| dataset | lossless (ours) | uniform-tuned (ours) | rule-aware (ours) | rule-aware κ=1024 (ours) | SZ3 tuned | SZ3+SG best | SZ3+SG ε / range | SZ3+SG points with mismatches | SZ3+SG encode, s (mean) | ×SZ3+SG |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 53.96 | 2.55 | 0.07 | 0.05 | 3.72 | 0.16 | 1.0e0 | 0/13 | 2.0 | 3.3 |
| latency | 55.24 | 2.81 | 0.08 | 0.07 | 4.65 | 0.20 | 1.0e0 | 0/13 | 1.2 | 2.9 |
| memory | 49.91 | 6.92 | 0.21 | 0.14 | 9.11 | 0.57 | 1.0e0 | 0/13 | 24.3 | 4.1 |
| requests | 19.68 | 0.81 | 0.47 | 0.46 | 2.65 | 0.69 | 6.1e-5 | 0/13 | 3.9 | 1.5 |
| requests-gappy | 21.76 | 9.46 | 2.92 | 2.72 | 11.23 | 8.45 | 2.4e-7 | 11/13 | 18.5 | 3.1 |

Values are bits per value (timestamps excluded). `×SZ3+SG` = SZ3+SG best / rule-aware κ=1024.

<details><summary>SZ3 / SZ3+SG sweep, <code>cpu</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 59.93 | 0 | 59.93 | 0 | 0.0 |
| 2.4e-7 | 59.93 | 0 | 59.93 | 0 | 0.0 |
| 9.5e-7 | 59.93 | 0 | 59.93 | 0 | 0.0 |
| 3.8e-6 | 59.93 | 0 | 59.93 | 0 | 0.1 |
| 1.5e-5 | 20.05 | 0 | 20.05 | 0 | 0.0 |
| 6.1e-5 | 14.93 | 0 | 14.94 | 0 | 0.1 |
| 2.4e-4 | 8.93 | 0 | 8.96 | 0 | 5.6 |
| 9.8e-4 | 6.19 | 0 | 6.23 | 0 | 3.7 |
| 3.9e-3 | 3.72 | 0 | 3.79 | 0 | 3.3 |
| 1.6e-2 | 1.94 | 9 | 2.06 | 0 | 3.3 |
| 6.2e-2 | 0.67 | 20 | 0.83 | 0 | 3.1 |
| 2.5e-1 | 0.32 | 368 | 0.48 | 0 | 3.7 |
| 1.0e0 | 0.03 | 178 | 0.16 | 0 | 3.5 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>latency</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 59.95 | 0 | 59.95 | 0 | 0.0 |
| 2.4e-7 | 59.95 | 0 | 59.95 | 0 | 0.0 |
| 9.5e-7 | 59.95 | 0 | 59.95 | 0 | 0.0 |
| 3.8e-6 | 26.86 | 0 | 26.86 | 0 | 0.0 |
| 1.5e-5 | 16.82 | 0 | 16.83 | 0 | 0.0 |
| 6.1e-5 | 10.18 | 0 | 10.18 | 0 | 0.0 |
| 2.4e-4 | 7.10 | 0 | 7.10 | 0 | 0.0 |
| 9.8e-4 | 4.65 | 0 | 4.65 | 0 | 0.0 |
| 3.9e-3 | 2.52 | 1 | 2.59 | 0 | 2.9 |
| 1.6e-2 | 0.94 | 4 | 1.06 | 0 | 2.8 |
| 6.2e-2 | 0.25 | 16 | 0.45 | 0 | 3.4 |
| 2.5e-1 | 0.07 | 120 | 0.34 | 0 | 2.9 |
| 1.0e0 | 0.03 | 257 | 0.20 | 0 | 2.9 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>memory</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 59.40 | 0 | 59.40 | 0 | 1.0 |
| 2.4e-7 | 59.40 | 0 | 59.40 | 0 | 1.0 |
| 9.5e-7 | 59.40 | 0 | 59.40 | 0 | 0.9 |
| 3.8e-6 | 23.11 | 0 | 23.11 | 0 | 1.0 |
| 1.5e-5 | 13.03 | 0 | 13.03 | 0 | 0.9 |
| 6.1e-5 | 9.11 | 0 | 9.11 | 0 | 0.8 |
| 2.4e-4 | 6.51 | 12 | 6.62 | 0 | 52.6 |
| 9.8e-4 | 4.28 | 14 | 4.47 | 0 | 45.0 |
| 3.9e-3 | 2.28 | 55 | 2.70 | 0 | 44.4 |
| 1.6e-2 | 0.72 | 279 | 1.62 | 0 | 44.4 |
| 6.2e-2 | 0.26 | 845 | 0.81 | 0 | 44.0 |
| 2.5e-1 | 0.13 | 4284 | 0.58 | 0 | 40.1 |
| 1.0e0 | 0.06 | 12856 | 0.57 | 0 | 40.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>requests</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 7.03 | 0 | 7.03 | 0 | 0.0 |
| 2.4e-7 | 4.75 | 0 | 4.77 | 0 | 5.6 |
| 9.5e-7 | 2.65 | 0 | 2.67 | 0 | 4.8 |
| 3.8e-6 | 1.23 | 29 | 1.27 | 0 | 4.5 |
| 1.5e-5 | 0.58 | 145 | 0.69 | 0 | 3.8 |
| 6.1e-5 | 0.42 | 21 | 0.69 | 0 | 5.2 |
| 2.4e-4 | 0.26 | 140 | 0.90 | 0 | 4.3 |
| 9.8e-4 | 0.18 | 201 | 1.00 | 0 | 4.0 |
| 3.9e-3 | 0.09 | 215 | 1.15 | 0 | 4.5 |
| 1.6e-2 | 0.09 | 300 | 1.31 | 0 | 2.9 |
| 6.2e-2 | 0.05 | 20189 | 0.92 | 0 | 4.3 |
| 2.5e-1 | 0.04 | 20189 | 0.87 | 0 | 3.9 |
| 1.0e0 | 0.03 | 20189 | 0.85 | 0 | 3.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>requests-gappy</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 11.23 | 0 | 11.64 | 0 | 26.2 |
| 2.4e-7 | 7.58 | 6 | 8.45 | 0 | 23.3 |
| 9.5e-7 | 4.83 | 23 | 6.53 | 4 | 16.5 |
| 3.8e-6 | 3.03 | 200 | 6.89 | 61 | 18.0 |
| 1.5e-5 | 2.75 | 746 | 9.30 | 58 | 17.7 |
| 6.1e-5 | 1.48 | 4599 | 10.59 | 67 | 16.3 |
| 2.4e-4 | 0.47 | 8430 | 9.17 | 85 | 18.4 |
| 9.8e-4 | 0.27 | 9908 | 8.66 | 35 | 16.5 |
| 3.9e-3 | 0.21 | 11816 | 8.90 | 64 | 17.3 |
| 1.6e-2 | 0.12 | 12412 | 8.44 | 79 | 17.8 |
| 6.2e-2 | 0.07 | 26808 | 8.23 | 63 | 16.1 |
| 2.5e-1 | 0.05 | 26728 | 8.11 | 60 | 18.7 |
| 1.0e0 | 0.03 | 26793 | 8.05 | 47 | 17.2 |

</details>

## H7: optimistic budget with a posteriori repair (7 days)

Bits per value with the analytic budget scaled by κ, then repaired: on every flipped evaluation the
budget on its support is halved, never below the analytic one, and zeroed if already there. In
parentheses: repair rounds (one encode + verify each). Zero changed decisions everywhere (asserted).

whole series:

| dataset | κ=1 (analytic) | κ=2 | κ=4 | κ=16 | κ=64 | κ=1024 |
|---|---:|---:|---:|---:|---:|---:|
| cpu | 0.07 (0) | 0.06 (2) | 0.05 (2) | 0.05 (5) | 0.05 (7) | 0.05 (12) |
| latency | 0.08 (0) | 0.07 (2) | 0.06 (4) | 0.06 (8) | 0.06 (12) | 0.07 (19) |
| memory | 0.21 (0) | 0.18 (1) | 0.16 (2) | 0.14 (7) | 0.14 (11) | 0.14 (21) |
| requests | 0.47 (0) | 0.46 (14) | 0.46 (15) | 0.46 (15) | 0.46 (18) | 0.46 (22) |
| requests-gappy | 2.92 (0) | 2.80 (6) | 2.74 (10) | 2.75 (22) | 2.76 (30) | 2.72 (75) |

chunks of 120:

| dataset | κ=1 (analytic) | κ=2 | κ=4 | κ=16 | κ=64 | κ=1024 |
|---|---:|---:|---:|---:|---:|---:|
| cpu | 0.30 (0) | 0.29 (1) | 0.28 (3) | 0.28 (6) | 0.28 (9) | 0.28 (15) |
| latency | 0.29 (0) | 0.28 (3) | 0.27 (5) | 0.27 (8) | 0.27 (12) | 0.27 (18) |
| memory | 0.48 (0) | 0.45 (2) | 0.43 (4) | 0.41 (8) | 0.41 (10) | 0.41 (18) |
| requests | 0.97 (0) | 0.96 (14) | 0.96 (13) | 0.96 (15) | 0.96 (17) | 0.96 (25) |
| requests-gappy | 3.64 (0) | 3.42 (4) | 3.37 (6) | 3.34 (15) | 3.34 (27) | 3.32 (51) |
