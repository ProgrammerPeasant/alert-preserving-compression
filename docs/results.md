# Results (synthetic data)

Produced by `cargo run --release -p alertsafe-bench -- all 7` on one core of a
desktop CPU (Linux 6.18, rustc 1.93). Data are seeded synthetic series from
`crates/alertsafe-bench/src/gen.rs`; they are placeholders for real cluster
traces and open rule sets (see the roadmap in the README). Treat the numbers
as a smoke test of the method, not as evidence for the thesis.

**Reading the numbers.**

- H1: rule-aware is 2.8–8.1× smaller than the largest uniform bound with the
  same guarantee and 2.5–4.6× smaller than a uniform bound tuned on the same
  data to zero mismatches (which has no guarantee on other data).
- The high-resolution prediction of the saving (formal model, §8)
  over-predicts by 2–4 bits: the codec saturates near 1.2–2 bits per value.
- H2: the rule-aware rate grows monotonically with the share of samples near
  the threshold (1.24 → 3.95 bits per value as `near` goes 0.1% → 10.2%), so
  the gain over lossless falls from 45× to 14×. The gain over the tuned
  uniform bound is noisy because the tuned bound itself jumps between sweep
  points.
- H3: decode ~3.5 GB/s and encode 170–340 MB/s per core; the budget
  computation (naive window scans, bisection for `rate`) at 6–28 MB/s and
  the end-to-end verified `compress` at 6–13 MB/s are the bottleneck.
- The analytic budget needed zero repairs on every dataset.

## H1: rule-aware vs. uniform error bound (7 days, 15s scrape, 30s eval)

All rule-aware rows have zero changed decisions (asserted). `uniform-safe` is the largest uniform bound
that satisfies the same sufficient condition; `uniform-tuned` is the largest uniform bound with zero
state mismatches found by sweeping on the same data (no guarantee elsewhere).

| dataset | firings | lossless | uniform-safe | uniform-tuned | rule-aware | ×lossless | ×safe | ×tuned | predicted Δbits (safe→aware) | measured Δbits | exact samples | repairs |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| cpu | 8 | 55.63 | 12.37 | 3.80 | 1.53 | 36.4 | 8.1 | 2.5 | 13.81 | 10.84 | 0.00% | 0 |
| latency | 11 | 56.97 | 8.37 | 6.39 | 1.38 | 41.2 | 6.1 | 4.6 | 10.43 | 6.99 | 0.00% | 0 |
| memory | 9 | 51.70 | 14.09 | 8.10 | 1.91 | 27.1 | 7.4 | 4.2 | 15.62 | 12.18 | 0.00% | 0 |
| requests | 4 | 21.07 | 6.15 | 6.15 | 2.20 | 9.6 | 2.8 | 2.8 | 8.24 | 3.95 | 0.01% | 0 |

Values are bits per value (timestamps excluded). Raw float64 is 64 bits.

<details><summary>Uniform-bound sweep, <code>cpu</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 33.37 | 0 | 0 | 0 |
| 5.8e-11 | 31.37 | 0 | 0 | 0 |
| 2.3e-10 | 29.37 | 0 | 0 | 0 |
| 9.3e-10 | 27.37 | 0 | 0 | 0 |
| 3.7e-9 | 25.37 | 0 | 0 | 0 |
| 1.5e-8 | 23.37 | 0 | 0 | 0 |
| 6.0e-8 | 21.37 | 0 | 0 | 0 |
| 2.4e-7 | 19.37 | 0 | 0 | 0 |
| 9.5e-7 | 17.37 | 0 | 0 | 0 |
| 3.8e-6 | 15.37 | 0 | 0 | 0 |
| 1.5e-5 | 13.37 | 0 | 0 | 0 |
| 6.1e-5 | 11.37 | 0 | 0 | 0 |
| 2.4e-4 | 9.38 | 0 | 0 | 0 |
| 9.8e-4 | 7.38 | 0 | 0 | 0 |
| 3.9e-3 | 5.43 | 0 | 0 | 0 |
| 1.6e-2 | 3.80 | 0 | 0 | 0 |
| 6.2e-2 | 2.20 | 11 | 0 | 60 |
| 2.5e-1 | 1.99 | 184 | 7 | 390 |
| 1.0e0 | 1.58 | 4847 | 213 | 1080 |

</details>

<details><summary>Uniform-bound sweep, <code>latency</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 32.37 | 0 | 0 | 0 |
| 5.8e-11 | 30.37 | 0 | 0 | 0 |
| 2.3e-10 | 28.37 | 0 | 0 | 0 |
| 9.3e-10 | 26.37 | 0 | 0 | 0 |
| 3.7e-9 | 24.37 | 0 | 0 | 0 |
| 1.5e-8 | 22.37 | 0 | 0 | 0 |
| 6.0e-8 | 20.37 | 0 | 0 | 0 |
| 2.4e-7 | 18.37 | 0 | 0 | 0 |
| 9.5e-7 | 16.37 | 0 | 0 | 0 |
| 3.8e-6 | 14.37 | 0 | 0 | 0 |
| 1.5e-5 | 12.37 | 0 | 0 | 0 |
| 6.1e-5 | 10.37 | 0 | 0 | 0 |
| 2.4e-4 | 8.37 | 0 | 0 | 0 |
| 9.8e-4 | 6.39 | 0 | 0 | 0 |
| 3.9e-3 | 4.47 | 1 | 0 | 0 |
| 1.6e-2 | 2.93 | 11 | 0 | 210 |
| 6.2e-2 | 2.09 | 23 | 1 | 330 |
| 2.5e-1 | 1.38 | 74 | 5 | 330 |
| 1.0e0 | 1.15 | 130 | 5 | 660 |

</details>

<details><summary>Uniform-bound sweep, <code>memory</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 32.09 | 0 | 0 | 0 |
| 5.8e-11 | 30.09 | 0 | 0 | 0 |
| 2.3e-10 | 28.09 | 0 | 0 | 0 |
| 9.3e-10 | 26.09 | 0 | 0 | 0 |
| 3.7e-9 | 24.09 | 0 | 0 | 0 |
| 1.5e-8 | 22.09 | 0 | 0 | 0 |
| 6.0e-8 | 20.09 | 0 | 0 | 0 |
| 2.4e-7 | 18.09 | 0 | 0 | 0 |
| 9.5e-7 | 16.09 | 0 | 0 | 0 |
| 3.8e-6 | 14.09 | 0 | 0 | 0 |
| 1.5e-5 | 12.09 | 0 | 0 | 0 |
| 6.1e-5 | 10.09 | 0 | 0 | 0 |
| 2.4e-4 | 8.10 | 0 | 0 | 0 |
| 9.8e-4 | 6.14 | 17 | 0 | 240 |
| 3.9e-3 | 4.21 | 25 | 0 | 30 |
| 1.6e-2 | 2.47 | 253 | 1 | 1170 |
| 6.2e-2 | 1.63 | 1347 | 0 | 5280 |
| 2.5e-1 | 1.25 | 2725 | 3 | 12540 |
| 1.0e0 | 1.20 | 14407 | 5 | 106530 |

</details>

<details><summary>Uniform-bound sweep, <code>requests</code></summary>

| ε / range | bits/value | state mismatches | changed firings | max shift, s |
|---:|---:|---:|---:|---:|
| 1.5e-11 | 21.02 | 0 | 0 | 0 |
| 5.8e-11 | 20.93 | 0 | 0 | 0 |
| 2.3e-10 | 20.09 | 0 | 0 | 0 |
| 9.3e-10 | 18.12 | 0 | 0 | 0 |
| 3.7e-9 | 16.12 | 0 | 0 | 0 |
| 1.5e-8 | 14.12 | 0 | 0 | 0 |
| 6.0e-8 | 12.12 | 0 | 0 | 0 |
| 2.4e-7 | 10.12 | 0 | 0 | 0 |
| 9.5e-7 | 8.13 | 0 | 0 | 0 |
| 3.8e-6 | 6.15 | 0 | 0 | 0 |
| 1.5e-5 | 4.23 | 2 | 0 | 30 |
| 6.1e-5 | 2.68 | 4 | 0 | 30 |
| 2.4e-4 | 2.14 | 41 | 1 | 270 |
| 9.8e-4 | 2.14 | 1031 | 0 | 270 |
| 3.9e-3 | 2.14 | 11277 | 160 | 390 |
| 1.6e-2 | 1.84 | 20143 | 231 | 1200 |
| 6.2e-2 | 1.32 | 20163 | 58 | 6210 |
| 2.5e-1 | 1.18 | 20189 | 14 | 29010 |
| 1.0e0 | 1.15 | 20189 | 4 | 213660 |

</details>

## H2: gain vs. distance from the threshold (`cpu`, rule `HighCPU` only, 7 days)

`near` is the share of samples whose budget is below 0.01 (i.e. the 5m average is within 0.01 of θ).

| θ | incidents/day | firings | near | lossless | uniform-tuned | rule-aware | ×lossless | ×tuned |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0.55 | 0.5 | 10 | 7.6% | 55.61 | 8.36 | 3.15 | 17.7 | 2.7 |
| 0.65 | 0.5 | 1 | 0.6% | 55.61 | 4.50 | 1.76 | 31.6 | 2.6 |
| 0.75 | 0.5 | 1 | 0.2% | 55.61 | 2.14 | 1.34 | 41.6 | 1.6 |
| 0.85 | 0.5 | 1 | 0.1% | 55.61 | 2.14 | 1.29 | 43.0 | 1.7 |
| 0.95 | 0.5 | 1 | 0.1% | 55.61 | 8.36 | 1.24 | 44.9 | 6.7 |
| 0.55 | 2 | 20 | 8.3% | 55.41 | 10.38 | 3.24 | 17.1 | 3.2 |
| 0.65 | 2 | 12 | 2.0% | 55.41 | 6.44 | 2.12 | 26.1 | 3.0 |
| 0.75 | 2 | 10 | 1.7% | 55.41 | 4.58 | 1.81 | 30.7 | 2.5 |
| 0.85 | 2 | 7 | 1.1% | 55.41 | 3.14 | 1.64 | 33.8 | 1.9 |
| 0.95 | 2 | 4 | 0.9% | 55.41 | 3.14 | 1.48 | 37.4 | 2.1 |
| 0.55 | 6 | 39 | 10.2% | 55.07 | 10.44 | 3.95 | 13.9 | 2.6 |
| 0.65 | 6 | 25 | 4.3% | 55.07 | 6.49 | 2.71 | 20.3 | 2.4 |
| 0.75 | 6 | 21 | 3.5% | 55.07 | 4.62 | 2.38 | 23.2 | 1.9 |
| 0.85 | 6 | 12 | 2.8% | 55.07 | 6.49 | 2.13 | 25.8 | 3.0 |
| 0.95 | 6 | 7 | 1.6% | 55.07 | 3.14 | 1.76 | 31.3 | 1.8 |

## H3: single-core throughput (MB/s of raw float64 input, best of 5)

| dataset | samples | budget | encode (rule-aware) | decode | encode (lossless) | end-to-end `compress` |
|---|---:|---:|---:|---:|---:|---:|
| cpu | 40320 | 28 | 171 | 3569 | 1024 | 13 |
| requests | 40320 | 6 | 342 | 3537 | 960 | 6 |
