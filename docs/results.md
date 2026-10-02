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
- H1: rule-aware is 4.5–32× smaller than the largest uniform bound with the
  same guarantee and 4.5–22× smaller than a uniform bound tuned on the same
  data to zero mismatches (which has no guarantee on other data).
- Entropy coding removes the ~1 bit/value floor of fixed-width codes: the
  rule-aware rate drops from 1.4–2.2 to 0.23–0.74 bits per value (3–6×).
  Far from thresholds the residuals are almost all zero and cost ~0.01 bit
  each; the rate is dominated by the samples near thresholds. Uniform bounds
  gain much less (e.g. `cpu` tuned 3.80 → 2.55), so the rule-aware advantage
  grows. The high-resolution prediction of the gain over uniform-safe
  (formal model, §8) still over-predicts: by 2.6–3.4 bits on the gauge
  datasets and by 6.5 on `requests`, where the uniform-safe bound is
  already cheap (2.18 bits per value).
- H2: the rule-aware rate grows with the share of samples near the threshold
  (0.08 → 2.99 bits per value as `near` goes 0.1% → 10.2%), so the gain over
  lossless falls from ~650× to 18×. The gain over the tuned uniform bound is
  noisy because the tuned bound itself jumps between sweep points.
- H3: the entropy-coded decode runs at 2.5–2.8 GB/s (fixed: 3.4 GB/s) and
  encode at 110–200 MB/s (fixed: 170–320 MB/s); block plans are compared by
  their exact adaptive cost, which costs encode speed. The `O(n)` budget runs
  at ~70 MB/s and the end-to-end verified `compress` at 25–45 MB/s.
- Stream overhead is 10 bytes (5 header, 5 coder flush) per series, i.e.
  0.67 bits per value on a 120-sample Prometheus chunk; amortized away here.
- The analytic budget needed zero repairs on every dataset.

## H1: rule-aware vs. uniform error bound (7 days, 15s scrape, 30s eval)

All rule-aware rows have zero changed decisions (asserted). `uniform-safe` is the largest uniform bound
that satisfies the same sufficient condition; `uniform-tuned` is the largest uniform bound with zero
state mismatches found by sweeping on the same data (no guarantee elsewhere).

| dataset | firings | lossless | uniform-safe | uniform-tuned | rule-aware | rule-aware (fixed) | ×lossless | ×safe | ×tuned | predicted Δbits (safe→aware) | measured Δbits | exact samples | repairs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 8 | 53.96 | 11.59 | 2.55 | 0.36 | 1.53 | 148.0 | 31.8 | 7.0 | 13.81 | 11.22 | 0.00% | 0 |
| latency | 11 | 55.24 | 7.23 | 5.20 | 0.23 | 1.38 | 238.6 | 31.2 | 22.5 | 10.43 | 7.00 | 0.00% | 0 |
| memory | 9 | 49.91 | 12.98 | 6.94 | 0.74 | 1.91 | 67.4 | 17.5 | 9.4 | 15.62 | 12.24 | 0.00% | 0 |
| requests | 4 | 19.68 | 2.18 | 2.18 | 0.49 | 2.20 | 40.2 | 4.5 | 4.5 | 8.24 | 1.69 | 0.01% | 0 |

Values are bits per value (timestamps excluded). Raw float64 is 64 bits. All columns except
`rule-aware (fixed)` use the entropy-coded format.

<details><summary>Uniform-bound sweep, <code>cpu</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 32.63 | 0 | 0 | 0 |
| 5.8e-11 | 30.62 | 0 | 0 | 0 |
| 2.3e-10 | 28.62 | 0 | 0 | 0 |
| 9.3e-10 | 26.61 | 0 | 0 | 0 |
| 3.7e-9 | 24.61 | 0 | 0 | 0 |
| 1.5e-8 | 22.60 | 0 | 0 | 0 |
| 6.0e-8 | 20.60 | 0 | 0 | 0 |
| 2.4e-7 | 18.60 | 0 | 0 | 0 |
| 9.5e-7 | 16.59 | 0 | 0 | 0 |
| 3.8e-6 | 14.59 | 0 | 0 | 0 |
| 1.5e-5 | 12.59 | 0 | 0 | 0 |
| 6.1e-5 | 10.59 | 0 | 0 | 0 |
| 2.4e-4 | 8.58 | 0 | 0 | 0 |
| 9.8e-4 | 6.56 | 0 | 0 | 0 |
| 3.9e-3 | 4.53 | 0 | 0 | 0 |
| 1.6e-2 | 2.55 | 0 | 0 | 0 |
| 6.2e-2 | 1.11 | 11 | 0 | 60 |
| 2.5e-1 | 0.32 | 184 | 7 | 390 |
| 1.0e0 | 0.21 | 4847 | 213 | 1080 |

</details>

<details><summary>Uniform-bound sweep, <code>latency</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 31.30 | 0 | 0 | 0 |
| 5.8e-11 | 29.29 | 0 | 0 | 0 |
| 2.3e-10 | 27.29 | 0 | 0 | 0 |
| 9.3e-10 | 25.28 | 0 | 0 | 0 |
| 3.7e-9 | 23.28 | 0 | 0 | 0 |
| 1.5e-8 | 21.27 | 0 | 0 | 0 |
| 6.0e-8 | 19.26 | 0 | 0 | 0 |
| 2.4e-7 | 17.26 | 0 | 0 | 0 |
| 9.5e-7 | 15.26 | 0 | 0 | 0 |
| 3.8e-6 | 13.26 | 0 | 0 | 0 |
| 1.5e-5 | 11.25 | 0 | 0 | 0 |
| 6.1e-5 | 9.25 | 0 | 0 | 0 |
| 2.4e-4 | 7.23 | 0 | 0 | 0 |
| 9.8e-4 | 5.20 | 0 | 0 | 0 |
| 3.9e-3 | 3.20 | 1 | 0 | 0 |
| 1.6e-2 | 1.49 | 11 | 0 | 210 |
| 6.2e-2 | 0.39 | 23 | 1 | 330 |
| 2.5e-1 | 0.07 | 74 | 5 | 330 |
| 1.0e0 | 0.01 | 130 | 5 | 660 |

</details>

<details><summary>Uniform-bound sweep, <code>memory</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 31.02 | 0 | 0 | 0 |
| 5.8e-11 | 29.01 | 0 | 0 | 0 |
| 2.3e-10 | 27.01 | 0 | 0 | 0 |
| 9.3e-10 | 25.00 | 0 | 0 | 0 |
| 3.7e-9 | 23.00 | 0 | 0 | 0 |
| 1.5e-8 | 20.99 | 0 | 0 | 0 |
| 6.0e-8 | 18.99 | 0 | 0 | 0 |
| 2.4e-7 | 16.98 | 0 | 0 | 0 |
| 9.5e-7 | 14.98 | 0 | 0 | 0 |
| 3.8e-6 | 12.98 | 0 | 0 | 0 |
| 1.5e-5 | 10.97 | 0 | 0 | 0 |
| 6.1e-5 | 8.96 | 0 | 0 | 0 |
| 2.4e-4 | 6.94 | 0 | 0 | 0 |
| 9.8e-4 | 4.91 | 17 | 0 | 240 |
| 3.9e-3 | 2.89 | 25 | 0 | 30 |
| 1.6e-2 | 1.25 | 253 | 1 | 1170 |
| 6.2e-2 | 0.34 | 1347 | 0 | 5280 |
| 2.5e-1 | 0.07 | 2725 | 3 | 12540 |
| 1.0e0 | 0.05 | 14407 | 5 | 106530 |

</details>

<details><summary>Uniform-bound sweep, <code>requests</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 19.49 | 0 | 0 | 0 |
| 5.8e-11 | 17.78 | 0 | 0 | 0 |
| 2.3e-10 | 15.79 | 0 | 0 | 0 |
| 9.3e-10 | 13.79 | 0 | 0 | 0 |
| 3.7e-9 | 11.80 | 0 | 0 | 0 |
| 1.5e-8 | 9.80 | 0 | 0 | 0 |
| 6.0e-8 | 7.82 | 0 | 0 | 0 |
| 2.4e-7 | 5.80 | 0 | 0 | 0 |
| 9.5e-7 | 3.80 | 0 | 0 | 0 |
| 3.8e-6 | 2.18 | 0 | 0 | 0 |
| 1.5e-5 | 1.53 | 2 | 0 | 30 |
| 6.1e-5 | 0.75 | 4 | 0 | 30 |
| 2.4e-4 | 0.58 | 41 | 1 | 270 |
| 9.8e-4 | 0.45 | 1031 | 0 | 270 |
| 3.9e-3 | 0.19 | 11277 | 160 | 390 |
| 1.6e-2 | 0.08 | 20143 | 231 | 1200 |
| 6.2e-2 | 0.03 | 20163 | 58 | 6210 |
| 2.5e-1 | 0.02 | 20189 | 14 | 29010 |
| 1.0e0 | 0.01 | 20189 | 4 | 213660 |

</details>

## H2: gain vs. distance from the threshold (`cpu`, rule `HighCPU` only, 7 days)

`near` is the share of samples whose budget is below 0.01 (i.e. the 5m average is within 0.01 of θ).

| θ | incidents/day | firings | near | lossless | uniform-tuned | rule-aware | ×lossless | ×tuned |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0.55 | 0.5 | 10 | 7.6% | 53.92 | 7.56 | 2.11 | 25.5 | 3.6 |
| 0.65 | 0.5 | 1 | 0.6% | 53.92 | 3.52 | 0.50 | 108.6 | 7.1 |
| 0.75 | 0.5 | 1 | 0.2% | 53.92 | 0.68 | 0.14 | 386.6 | 4.9 |
| 0.85 | 0.5 | 1 | 0.1% | 53.92 | 0.68 | 0.10 | 533.9 | 6.7 |
| 0.95 | 0.5 | 1 | 0.1% | 53.92 | 7.56 | 0.08 | 648.6 | 91.0 |
| 0.55 | 2 | 20 | 8.3% | 53.75 | 9.56 | 2.24 | 23.9 | 4.3 |
| 0.65 | 2 | 12 | 2.0% | 53.75 | 5.54 | 0.97 | 55.3 | 5.7 |
| 0.75 | 2 | 10 | 1.7% | 53.75 | 3.52 | 0.62 | 86.7 | 5.7 |
| 0.85 | 2 | 7 | 1.1% | 53.75 | 1.71 | 0.42 | 128.0 | 4.1 |
| 0.95 | 2 | 4 | 0.9% | 53.75 | 1.71 | 0.32 | 165.5 | 5.3 |
| 0.55 | 6 | 39 | 10.2% | 53.44 | 9.54 | 2.99 | 17.9 | 3.2 |
| 0.65 | 6 | 25 | 4.3% | 53.44 | 5.54 | 1.55 | 34.5 | 3.6 |
| 0.75 | 6 | 21 | 3.5% | 53.44 | 3.54 | 1.22 | 43.7 | 2.9 |
| 0.85 | 6 | 12 | 2.8% | 53.44 | 5.54 | 0.94 | 56.6 | 5.9 |
| 0.95 | 6 | 7 | 1.6% | 53.44 | 1.72 | 0.59 | 90.1 | 2.9 |

## H3: single-core throughput (MB/s of raw float64 input, best of 5)

| dataset | samples | budget (naive) | budget | encode (rule-aware) | decode | encode (fixed) | decode (fixed) | encode (lossless) | end-to-end `compress` |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 40320 | 31 | 69 | 112 | 2778 | 166 | 3464 | 399 | 25 |
| requests | 40320 | 46 | 68 | 196 | 2490 | 324 | 3425 | 504 | 44 |
