# Results (synthetic data)

Produced by `cargo run --release -p alertsafe-bench -- all 7` on one core of a
desktop CPU (Linux 6.18, rustc 1.93). Data are seeded synthetic series from
`crates/alertsafe-bench/src/gen.rs`; they are placeholders for real cluster
traces and open rule sets (see the roadmap in the README). Treat the numbers
as a smoke test of the method, not as evidence for the thesis.

**Reading the numbers.**

- All columns use the same entropy-coded format (adaptive binary range
  coding of block decisions and DPCM residuals, `alertsafe-codec::rc`), so
  baselines and the rule-aware codec differ only in the error budget.
  `rule-aware (fixed)` is the ablation with fixed-width residual codes.
- H1 (one stream per series): rule-aware is 4.5–34× smaller than the
  largest uniform bound with the same guarantee and 4.5–24× smaller than a
  uniform bound tuned on the same data to zero mismatches (which has no
  guarantee on other data).
- Entropy coding removes the ~1 bit/value floor of fixed-width codes: the
  rule-aware rate drops from 1.4–2.2 to 0.22–0.73 bits per value (3–6×).
  Far from thresholds the residuals are almost all zero and cost ~0.01 bit
  each; the rate is dominated by the samples near thresholds. Uniform bounds
  gain much less (e.g. `cpu` tuned 3.80 → 2.55), so the rule-aware advantage
  grows. The high-resolution prediction of the gain over uniform-safe
  (formal model, §8) still over-predicts: by 2.6–3.4 bits on the gauge
  datasets and by 6.5 on `requests`, where the uniform-safe bound is
  already cheap (2.18 bits per value).
- H2: the rule-aware rate grows with the share of samples near the threshold
  (0.08 → 2.98 bits per value as `near` goes 0.1% → 10.2%), so the gain over
  lossless falls from ~700× to 18×. The gain over the tuned uniform bound is
  noisy because the tuned bound itself jumps between sweep points.
- H3: the entropy-coded decode runs at 2.3–2.6 GB/s (fixed: 3.5 GB/s) and
  encode at 100–185 MB/s (fixed: 140–290 MB/s); block plans are compared by
  their exact adaptive cost, which costs encode speed. The `O(n)` budget runs
  at ~70 MB/s and the end-to-end verified `compress` at 26–44 MB/s.
- H4: Prometheus cuts chunks at 120 samples, and every chunk must decode on
  its own. Short streams cost more for three reasons. Each has a fixed
  overhead, now 3–4 bytes (format byte, LEB128 length, a minimal coder
  flush; it was 10). Each starts the models from p = 1/2. And each restates
  an absolute first value. At 120 samples the rule-aware rate is 0.53–1.02
  bits per value (whole series: 0.22–0.73), and the gain over a tuned
  uniform bound under the same chunking is 3.1–11×. The chunking costs the
  rule-aware codec relatively more because its payload is so small. Before
  this change the 120-sample rates were 0.66–1.10 on gauges and 2.86 on
  `requests`. Two fixes brought them down. Model rates now start at 1/2 and
  slow to 1/32 (a count-based estimate), which leaves long streams
  unchanged. And counter chunks now continue the previous chunk's
  reconstruction: without that, the output could drop at a chunk boundary
  and `rate` saw a spurious reset (250 repairs; see formal model, Prop. 1).
- H5: the uncapped rule-aware reconstruction is only good for the rules. Its
  RMSE is 7–17% of the value range on gauges and the maximum error reaches
  45%. Capping the budget at the tuned uniform ε (`Options::max_error`)
  costs within 0–5% of that uniform bound, so once a pointwise bound tighter
  than the rule budget is required, rule awareness adds a guarantee rather
  than a rate gain. At a fixed ε = 10⁻³ of the range the uniform bound
  changes decisions on `memory` (17 states) and `requests` (1021 states).
  The capped rule-aware codec keeps every decision for 0.03–0.15 extra bits
  per value.
- The analytic budget needed zero repairs on every dataset and chunk size.

## H1: rule-aware vs. uniform error bound (7 days, 15s scrape, 30s eval)

All rule-aware rows have zero changed decisions (asserted). `uniform-safe` is the largest uniform bound
that satisfies the same sufficient condition; `uniform-tuned` is the largest uniform bound with zero
state mismatches found by sweeping on the same data (no guarantee elsewhere).

| dataset | firings | lossless | uniform-safe | uniform-tuned | rule-aware | rule-aware (fixed) | ×lossless | ×safe | ×tuned | predicted Δbits (safe→aware) | measured Δbits | exact samples | repairs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 8 | 53.96 | 11.57 | 2.55 | 0.36 | 1.53 | 150.2 | 32.2 | 7.1 | 13.81 | 11.21 | 0.00% | 0 |
| latency | 11 | 55.24 | 7.22 | 5.20 | 0.22 | 1.38 | 256.6 | 33.5 | 24.1 | 10.43 | 7.01 | 0.00% | 0 |
| memory | 9 | 49.91 | 12.96 | 6.93 | 0.73 | 1.91 | 68.3 | 17.7 | 9.5 | 15.62 | 12.23 | 0.00% | 0 |
| requests | 4 | 19.68 | 2.18 | 2.17 | 0.48 | 2.20 | 40.6 | 4.5 | 4.5 | 8.24 | 1.69 | 0.01% | 0 |

Values are bits per value (timestamps excluded). Raw float64 is 64 bits. All columns except
`rule-aware (fixed)` use the entropy-coded format.

<details><summary>Uniform-bound sweep, <code>cpu</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
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
| 6.1e-5 | 10.57 | 0 | 0 | 0 |
| 2.4e-4 | 8.57 | 0 | 0 | 0 |
| 9.8e-4 | 6.55 | 0 | 0 | 0 |
| 3.9e-3 | 4.52 | 0 | 0 | 0 |
| 1.6e-2 | 2.55 | 0 | 0 | 0 |
| 6.2e-2 | 1.11 | 11 | 0 | 60 |
| 2.5e-1 | 0.32 | 184 | 7 | 390 |
| 1.0e0 | 0.20 | 4847 | 213 | 1080 |

</details>

<details><summary>Uniform-bound sweep, <code>latency</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 31.26 | 0 | 0 | 0 |
| 5.8e-11 | 29.26 | 0 | 0 | 0 |
| 2.3e-10 | 27.26 | 0 | 0 | 0 |
| 9.3e-10 | 25.25 | 0 | 0 | 0 |
| 3.7e-9 | 23.25 | 0 | 0 | 0 |
| 1.5e-8 | 21.25 | 0 | 0 | 0 |
| 6.0e-8 | 19.25 | 0 | 0 | 0 |
| 2.4e-7 | 17.24 | 0 | 0 | 0 |
| 9.5e-7 | 15.24 | 0 | 0 | 0 |
| 3.8e-6 | 13.24 | 0 | 0 | 0 |
| 1.5e-5 | 11.24 | 0 | 0 | 0 |
| 6.1e-5 | 9.23 | 0 | 0 | 0 |
| 2.4e-4 | 7.22 | 0 | 0 | 0 |
| 9.8e-4 | 5.20 | 0 | 0 | 0 |
| 3.9e-3 | 3.19 | 1 | 0 | 0 |
| 1.6e-2 | 1.48 | 11 | 0 | 210 |
| 6.2e-2 | 0.39 | 23 | 1 | 330 |
| 2.5e-1 | 0.06 | 74 | 5 | 330 |
| 1.0e0 | 0.01 | 130 | 5 | 660 |

</details>

<details><summary>Uniform-bound sweep, <code>memory</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
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
| 1.5e-5 | 10.96 | 0 | 0 | 0 |
| 6.1e-5 | 8.95 | 0 | 0 | 0 |
| 2.4e-4 | 6.93 | 0 | 0 | 0 |
| 9.8e-4 | 4.90 | 17 | 0 | 240 |
| 3.9e-3 | 2.88 | 25 | 0 | 30 |
| 1.6e-2 | 1.24 | 253 | 1 | 1170 |
| 6.2e-2 | 0.34 | 1347 | 0 | 5280 |
| 2.5e-1 | 0.07 | 2725 | 3 | 12540 |
| 1.0e0 | 0.05 | 14407 | 5 | 106530 |

</details>

<details><summary>Uniform-bound sweep, <code>requests</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 19.37 | 0 | 0 | 0 |
| 5.8e-11 | 17.76 | 0 | 0 | 0 |
| 2.3e-10 | 15.77 | 0 | 0 | 0 |
| 9.3e-10 | 13.78 | 0 | 0 | 0 |
| 3.7e-9 | 11.78 | 0 | 0 | 0 |
| 1.5e-8 | 9.79 | 0 | 0 | 0 |
| 6.0e-8 | 7.80 | 0 | 0 | 0 |
| 2.4e-7 | 5.79 | 0 | 0 | 0 |
| 9.5e-7 | 3.79 | 0 | 0 | 0 |
| 3.8e-6 | 2.17 | 0 | 0 | 0 |
| 1.5e-5 | 1.40 | 2 | 0 | 30 |
| 6.1e-5 | 0.69 | 4 | 0 | 30 |
| 2.4e-4 | 0.57 | 41 | 1 | 270 |
| 9.8e-4 | 0.44 | 1031 | 0 | 270 |
| 3.9e-3 | 0.19 | 11277 | 160 | 390 |
| 1.6e-2 | 0.07 | 20143 | 231 | 1200 |
| 6.2e-2 | 0.02 | 20163 | 58 | 6210 |
| 2.5e-1 | 0.01 | 20189 | 14 | 29010 |
| 1.0e0 | 0.01 | 20189 | 4 | 213660 |

</details>

## H2: gain vs. distance from the threshold (`cpu`, rule `HighCPU` only, 7 days)

`near` is the share of samples whose budget is below 0.01 (i.e. the 5m average is within 0.01 of θ).

| θ | incidents/day | firings | near | lossless | uniform-tuned | rule-aware | ×lossless | ×tuned |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0.55 | 0.5 | 10 | 7.6% | 53.92 | 7.55 | 2.10 | 25.7 | 3.6 |
| 0.65 | 0.5 | 1 | 0.6% | 53.92 | 3.51 | 0.49 | 109.5 | 7.1 |
| 0.75 | 0.5 | 1 | 0.2% | 53.92 | 0.67 | 0.14 | 384.9 | 4.8 |
| 0.85 | 0.5 | 1 | 0.1% | 53.92 | 0.67 | 0.10 | 561.4 | 7.0 |
| 0.95 | 0.5 | 1 | 0.1% | 53.92 | 7.55 | 0.08 | 700.4 | 98.1 |
| 0.55 | 2 | 20 | 8.3% | 53.75 | 9.55 | 2.23 | 24.1 | 4.3 |
| 0.65 | 2 | 12 | 2.0% | 53.75 | 5.53 | 0.97 | 55.6 | 5.7 |
| 0.75 | 2 | 10 | 1.7% | 53.75 | 3.52 | 0.64 | 84.6 | 5.5 |
| 0.85 | 2 | 7 | 1.1% | 53.75 | 1.70 | 0.42 | 127.5 | 4.0 |
| 0.95 | 2 | 4 | 0.9% | 53.75 | 1.70 | 0.32 | 166.3 | 5.3 |
| 0.55 | 6 | 39 | 10.2% | 53.44 | 9.52 | 2.98 | 17.9 | 3.2 |
| 0.65 | 6 | 25 | 4.3% | 53.44 | 5.53 | 1.53 | 34.9 | 3.6 |
| 0.75 | 6 | 21 | 3.5% | 53.44 | 3.54 | 1.21 | 44.1 | 2.9 |
| 0.85 | 6 | 12 | 2.8% | 53.44 | 5.53 | 0.94 | 56.8 | 5.9 |
| 0.95 | 6 | 7 | 1.6% | 53.44 | 1.72 | 0.59 | 90.6 | 2.9 |

## H3: single-core throughput (MB/s of raw float64 input, best of 5)

| dataset | samples | budget (naive) | budget | encode (rule-aware) | decode | encode (fixed) | decode (fixed) | encode (lossless) | end-to-end `compress` |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 40320 | 31 | 71 | 99 | 2635 | 139 | 3543 | 402 | 26 |
| requests | 40320 | 48 | 71 | 184 | 2284 | 291 | 3501 | 513 | 44 |

## H4: independently decodable chunks (7 days)

Every chunk is a separate stream (own header, coder flush, models reset to p = 1/2). The budget
is planned and verified on the whole series; `compress` asserts zero changed decisions.

| dataset | chunk | lossless | uniform-tuned | rule-aware | ×lossless | ×tuned | repairs |
|---|---:|---:|---:|---:|---:|---:|---:|
| cpu | 120 | 57.99 | 3.14 | 0.59 | 98.6 | 5.3 | 0 |
| cpu | 480 | 55.11 | 2.74 | 0.39 | 139.7 | 6.9 | 0 |
| cpu | 1920 | 54.29 | 2.60 | 0.33 | 163.7 | 7.8 | 0 |
| cpu | whole | 53.96 | 2.55 | 0.36 | 150.2 | 7.1 | 0 |
| latency | 120 | 58.87 | 5.92 | 0.53 | 111.0 | 11.2 | 0 |
| latency | 480 | 56.25 | 5.44 | 0.35 | 158.9 | 15.4 | 0 |
| latency | 1920 | 55.53 | 5.26 | 0.27 | 208.9 | 19.8 | 0 |
| latency | whole | 55.24 | 5.20 | 0.22 | 256.6 | 24.1 | 0 |
| memory | 120 | 56.85 | 7.73 | 1.02 | 55.7 | 7.6 | 0 |
| memory | 480 | 51.68 | 7.21 | 0.80 | 64.8 | 9.0 | 0 |
| memory | 1920 | 50.32 | 7.01 | 0.73 | 68.5 | 9.5 | 0 |
| memory | whole | 49.91 | 6.93 | 0.73 | 68.3 | 9.5 | 0 |
| requests | 120 | 27.02 | 3.13 | 1.02 | 26.5 | 3.1 | 0 |
| requests | 480 | 21.22 | 2.46 | 0.65 | 32.7 | 3.8 | 0 |
| requests | 1920 | 19.97 | 2.26 | 0.53 | 38.0 | 4.3 | 0 |
| requests | whole | 19.68 | 2.17 | 0.48 | 40.6 | 4.5 | 0 |

## H5: signal fidelity (7 days, 120-sample chunks)

Errors are relative to the value range of the series. `rule-aware ≤ ε` caps every sample's
budget at ε on top of the rule budget (`Options::max_error`) and keeps every decision
(asserted); `uniform ε` rows keep decisions only where the mismatch column says so.

| dataset | codec | bits/value | RMSE / range | max error / range | state mismatches |
|---|---|---:|---:|---:|---:|
| cpu | uniform ε = 1.6e-2 | 3.14 | 4.6e-3 | 8.0e-3 | 0 |
| cpu | uniform ε = 1.0e-3 | 6.29 | 5.8e-4 | 9.9e-4 | 0 |
| cpu | rule-aware | 0.59 | 1.7e-1 | 4.5e-1 | 0 |
| cpu | rule-aware ≤ 1.6e-2 | 3.28 | 4.5e-3 | 8.0e-3 | 0 |
| cpu | rule-aware ≤ 1.0e-3 | 6.35 | 5.7e-4 | 9.9e-4 | 0 |
| latency | uniform ε = 9.8e-4 | 5.92 | 2.9e-4 | 5.0e-4 | 0 |
| latency | uniform ε = 1.0e-3 | 4.89 | 5.7e-4 | 9.9e-4 | 0 |
| latency | rule-aware | 0.53 | 6.5e-2 | 1.3e-1 | 0 |
| latency | rule-aware ≤ 9.8e-4 | 5.94 | 2.9e-4 | 5.0e-4 | 0 |
| latency | rule-aware ≤ 1.0e-3 | 4.92 | 5.7e-4 | 9.9e-4 | 0 |
| memory | uniform ε = 2.4e-4 | 7.73 | 9.6e-5 | 1.7e-4 | 0 |
| memory | uniform ε = 1.0e-3 | 5.71 | 3.9e-4 | 6.7e-4 | 17 |
| memory | rule-aware | 1.02 | 8.4e-2 | 2.6e-1 | 0 |
| memory | rule-aware ≤ 2.4e-4 | 7.82 | 9.5e-5 | 1.7e-4 | 0 |
| memory | rule-aware ≤ 1.0e-3 | 5.86 | 3.8e-4 | 6.7e-4 | 0 |
| requests | uniform ε = 3.8e-6 | 3.13 | 1.5e-6 | 2.6e-6 | 0 |
| requests | uniform ε = 1.0e-3 | 0.97 | 3.9e-4 | 6.8e-4 | 1021 |
| requests | rule-aware | 1.02 | 5.2e-4 | 1.4e-3 | 0 |
| requests | rule-aware ≤ 3.8e-6 | 3.13 | 1.5e-6 | 2.6e-6 | 0 |
| requests | rule-aware ≤ 1.0e-3 | 1.09 | 3.3e-4 | 6.8e-4 | 0 |
