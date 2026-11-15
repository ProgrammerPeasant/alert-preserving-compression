# Ablation

Produced on 2026-11-15 by `alertsafe-bench ablation 7 --real data/real` and
`alertsafe-bench ablation 30 --real data/real30` (16 threads; 36 s and
~4 min). Every configuration is the end-to-end `compress_with` with the
options named in the row (`Options::{one_sided, steps, scales, slack}`), and
zero changed decisions is asserted in every cell. Real series use the `q95`
and `q50` thresholds (`prod` never fires, see `results-real.md`). The
synthetic block of the 30-day run uses 30-day synthetic series.

## Findings

- **One-sided budget and lazy steps together halve the rate or better.**
  From the Theorem 1 box with round-to-nearest to the full analytic codec
  the geometric mean of the size ratio is 2.30× (synthetic), 2.51× / 2.46×
  (real q95 / q50, 7 d) and 2.34×, 2.51× / 2.40× (30 d). The stream format
  and the decoder are unchanged.
- **The two only work together.** One-sided bounds with round-to-nearest
  gain little (2.30 → 2.19, 2.51 → 2.11) and can lose: the grid is chosen
  from the width `down + up` of the admissible interval, but nearest
  rounding spends it symmetrically, so a very asymmetric interval forces
  escapes or finer grids (`node_memory_available@q50` 3.40 → 4.09 bits per
  value; 30-day q50 mean 1.95 → 2.36). Lazy steps on the box
  (`rule-aware − one-sided`) recover most of the gain (1.22–1.38× above the
  full codec); one-sided bounds add the rest. On counters the one-sided
  plan changes nothing (`rate` budgets sit on window ends and the floor
  quantizer is one-sided anyway), and all of the counter gain (1.0–1.9× per dataset)
  comes from lazy steps, which keep the prediction on the free inner
  samples (Proposition 1(e)).
- **Scale search is redundant.** Encoding the budget × ½, ¼, ⅛ as well
  (`Options::scales`, added 2026-11-02) is smaller on 1 of 31 datasets by at
  most 0.006 bits per value, with or without the other components: the
  `FINER_STEPS` grid candidates added at the same time already offer the
  exponents of ×½ and ×¼ per block. It costs four encodes (see H3). The
  default is kept for now so committed tables stay reproducible.
- **Optimistic budget.** κ = 1024 on top of the full analytic codec saves a
  further 5–19 % (geometric mean 0.81–0.95). Without the one-sided budget,
  κ is *worse* than the one-sided analytic codec on real data (1.06–1.12):
  repair works on whole supports, so it cannot recover the directional
  freedom.
- **Entropy coding.** Fixed-width residual codes on the same plan are
  5.9–8.5× larger (the ~1 bit/value floor).

## 7 days
Synthetic, 7 days:

| configuration | cpu | latency | memory | requests | requests-gappy |
|---|---:|---:|---:|---:|---:|
| box, nearest (Thm 1) | 0.32 | 0.27 | 0.70 | 0.49 | 3.52 |
| + one-sided | 0.26 | 0.26 | 0.70 | 0.49 | 3.52 |
| + lazy steps | 0.07 | 0.08 | 0.21 | 0.47 | 2.92 |
| + scale search (= rule-aware) | 0.07 | 0.08 | 0.21 | 0.47 | 2.92 |
| + κ=1024 (= κ) | 0.05 | 0.07 | 0.14 | 0.46 | 2.72 |
| box, nearest + scale search | 0.32 | 0.27 | 0.70 | 0.49 | 3.52 |
| rule-aware − one-sided | 0.14 | 0.11 | 0.38 | 0.47 | 2.92 |
| rule-aware − lazy steps | 0.26 | 0.26 | 0.70 | 0.49 | 3.52 |
| κ − one-sided | 0.06 | 0.07 | 0.21 | 0.46 | 2.72 |
| rule-aware, fixed-width codes | 1.31 | 1.30 | 1.62 | 2.17 | 6.73 |

<details><summary>Real series (data/real), q95 / q50 thresholds</summary>

| configuration | node_load1@q95 | node_load1@q50 | node_memory_available@q95 | node_memory_available@q50 | node_fs_root_avail@q95 | node_fs_root_avail@q50 | node_timex_offset@q95 | node_timex_offset@q50 | prometheus_engine_queries@q95 | prometheus_engine_queries@q50 | node_vmstat_pgmajfault@q95 | node_vmstat_pgmajfault@q50 | node_disk_io_weighted_sda@q95 | node_disk_io_weighted_sda@q50 | node_cpu0_idle@q95 | node_cpu0_idle@q50 | prometheus_samples_appended@q95 | prometheus_samples_appended@q50 | caddy_requests@q95 | caddy_requests@q50 | demo_api_foo_200@q95 | demo_api_foo_200@q50 | demo_api_foo_500@q95 | demo_api_foo_500@q50 | demo_cpu_seconds@q95 | demo_cpu_seconds@q50 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| box, nearest (Thm 1) | 2.06 | 4.85 | 1.68 | 3.40 | 0.07 | 0.15 | 0.85 | 0.89 | 0.02 | 0.02 | 1.08 | 1.54 | 0.85 | 1.50 | 2.78 | 4.72 | 3.64 | 6.05 | 2.12 | 2.13 | 3.61 | 2.58 | 3.19 | 3.33 | 2.13 | 2.65 |
| + one-sided | 1.20 | 1.39 | 1.31 | 4.09 | 0.05 | 0.09 | 0.47 | 0.31 | 0.01 | 0.01 | 1.08 | 1.54 | 0.85 | 1.50 | 2.78 | 4.72 | 3.64 | 6.05 | 2.12 | 2.13 | 3.61 | 2.58 | 3.19 | 3.33 | 2.13 | 2.65 |
| + lazy steps | 0.20 | 0.75 | 0.13 | 0.37 | 0.01 | 0.02 | 0.12 | 0.08 | 0.01 | 0.01 | 0.79 | 1.54 | 0.71 | 1.39 | 2.02 | 3.51 | 2.47 | 4.45 | 1.38 | 1.39 | 3.02 | 1.56 | 2.68 | 2.90 | 1.21 | 1.42 |
| + scale search (= rule-aware) | 0.20 | 0.75 | 0.13 | 0.37 | 0.01 | 0.02 | 0.12 | 0.08 | 0.01 | 0.01 | 0.79 | 1.54 | 0.71 | 1.39 | 2.02 | 3.51 | 2.47 | 4.45 | 1.38 | 1.39 | 3.02 | 1.56 | 2.68 | 2.90 | 1.21 | 1.42 |
| + κ=1024 (= κ) | 0.20 | 0.75 | 0.13 | 0.37 | 0.01 | 0.02 | 0.12 | 0.08 | 0.01 | 0.01 | 0.74 | 1.54 | 0.61 | 1.33 | 1.31 | 2.95 | 2.25 | 4.22 | 1.30 | 1.31 | 2.82 | 1.43 | 2.56 | 2.79 | 1.11 | 1.25 |
| box, nearest + scale search | 2.06 | 4.85 | 1.68 | 3.40 | 0.07 | 0.15 | 0.85 | 0.89 | 0.02 | 0.02 | 1.08 | 1.54 | 0.85 | 1.50 | 2.78 | 4.72 | 3.64 | 6.05 | 2.12 | 2.13 | 3.61 | 2.58 | 3.19 | 3.33 | 2.13 | 2.65 |
| rule-aware − one-sided | 0.21 | 0.75 | 0.17 | 0.43 | 0.03 | 0.07 | 0.22 | 0.12 | 0.02 | 0.02 | 0.79 | 1.54 | 0.71 | 1.39 | 2.02 | 3.51 | 2.47 | 4.45 | 1.38 | 1.39 | 3.02 | 1.56 | 2.68 | 2.90 | 1.21 | 1.42 |
| rule-aware − lazy steps | 0.73 | 1.14 | 1.31 | 4.09 | 0.05 | 0.09 | 0.47 | 0.31 | 0.01 | 0.01 | 1.08 | 1.54 | 0.85 | 1.50 | 2.78 | 4.72 | 3.64 | 6.05 | 2.12 | 2.13 | 3.61 | 2.58 | 3.19 | 3.33 | 2.13 | 2.65 |
| κ − one-sided | 0.21 | 0.75 | 0.17 | 0.43 | 0.03 | 0.07 | 0.20 | 0.12 | 0.01 | 0.01 | 0.74 | 1.54 | 0.61 | 1.33 | 1.31 | 2.95 | 2.25 | 4.22 | 1.30 | 1.31 | 2.82 | 1.43 | 2.56 | 2.79 | 1.11 | 1.25 |
| rule-aware, fixed-width codes | 1.86 | 2.13 | 1.56 | 2.06 | 1.17 | 1.21 | 1.70 | 1.90 | 1.02 | 1.02 | 2.99 | 4.94 | 3.57 | 5.16 | 6.51 | 8.95 | 9.01 | 11.89 | 3.84 | 3.71 | 6.77 | 4.74 | 5.55 | 5.74 | 4.20 | 4.97 |

</details>

Scale search (`+ scale search` vs `+ lazy steps`) is smaller on 1 of 31 datasets, by at most 0.0044 bits per value.

Summary: mean bits per value and geometric mean of the ratio to `rule-aware` (+ scale search):

| configuration | synthetic mean (5) | synthetic ÷ rule-aware | real q95 mean (13) | real q95 ÷ rule-aware | real q50 mean (13) | real q50 ÷ rule-aware |
|---|---:|---:|---:|---:|---:|---:|
| box, nearest (Thm 1) | 1.06 | 2.30 | 1.85 | 2.51 | 2.60 | 2.46 |
| + one-sided | 1.05 | 2.19 | 1.73 | 2.11 | 2.34 | 1.93 |
| + lazy steps | 0.75 | 1.00 | 1.13 | 1.00 | 1.49 | 1.00 |
| + scale search (= rule-aware) | 0.75 | 1.00 | 1.13 | 1.00 | 1.49 | 1.00 |
| + κ=1024 (= κ) | 0.69 | 0.81 | 1.01 | 0.92 | 1.39 | 0.95 |
| box, nearest + scale search | 1.06 | 2.30 | 1.85 | 2.51 | 2.60 | 2.46 |
| rule-aware − one-sided | 0.80 | 1.38 | 1.15 | 1.23 | 1.50 | 1.22 |
| rule-aware − lazy steps | 1.05 | 2.19 | 1.69 | 2.03 | 2.32 | 1.90 |
| κ − one-sided | 0.70 | 0.93 | 1.03 | 1.06 | 1.40 | 1.10 |
| rule-aware, fixed-width codes | 2.63 | 7.57 | 3.83 | 7.31 | 4.49 | 5.94 |

## 30 days

Synthetic, 30 days:

| configuration | cpu | latency | memory | requests | requests-gappy |
|---|---:|---:|---:|---:|---:|
| box, nearest (Thm 1) | 0.37 | 0.43 | 0.71 | 0.53 | 3.69 |
| + one-sided | 0.31 | 0.40 | 0.68 | 0.53 | 3.69 |
| + lazy steps | 0.08 | 0.11 | 0.22 | 0.49 | 3.09 |
| + scale search (= rule-aware) | 0.08 | 0.11 | 0.22 | 0.49 | 3.09 |
| + κ=1024 (= κ) | 0.06 | 0.09 | 0.17 | 0.47 | 2.88 |
| box, nearest + scale search | 0.37 | 0.43 | 0.71 | 0.53 | 3.69 |
| rule-aware − one-sided | 0.14 | 0.17 | 0.37 | 0.49 | 3.09 |
| rule-aware − lazy steps | 0.31 | 0.40 | 0.68 | 0.53 | 3.69 |
| κ − one-sided | 0.08 | 0.11 | 0.18 | 0.47 | 2.88 |
| rule-aware, fixed-width codes | 1.36 | 1.45 | 1.63 | 2.20 | 6.89 |

<details><summary>Real series (data/real30), q95 / q50 thresholds</summary>

| configuration | node_load1@q95 | node_load1@q50 | node_memory_available@q95 | node_memory_available@q50 | node_fs_root_avail@q95 | node_fs_root_avail@q50 | node_timex_offset@q95 | node_timex_offset@q50 | prometheus_engine_queries@q95 | prometheus_engine_queries@q50 | node_vmstat_pgmajfault@q95 | node_vmstat_pgmajfault@q50 | node_disk_io_weighted_sda@q95 | node_disk_io_weighted_sda@q50 | node_cpu0_idle@q95 | node_cpu0_idle@q50 | prometheus_samples_appended@q95 | prometheus_samples_appended@q50 | caddy_requests@q95 | caddy_requests@q50 | demo_api_foo_200@q95 | demo_api_foo_200@q50 | demo_api_foo_500@q95 | demo_api_foo_500@q50 | demo_cpu_seconds@q95 | demo_cpu_seconds@q50 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| box, nearest (Thm 1) | 2.36 | 4.72 | 0.53 | 0.98 | 0.08 | 0.17 | 0.80 | 0.83 | 0.01 | 0.01 | 1.09 | 1.59 | 0.88 | 1.45 | 2.26 | 3.37 | 1.17 | 1.41 | 1.86 | 2.31 | 3.58 | 2.56 | 3.20 | 3.34 | 2.04 | 2.61 |
| + one-sided | 1.71 | 10.54 | 0.43 | 1.01 | 0.04 | 0.09 | 0.45 | 0.34 | 0.01 | 0.01 | 1.09 | 1.59 | 0.88 | 1.45 | 2.26 | 3.37 | 1.17 | 1.41 | 1.86 | 2.31 | 3.58 | 2.56 | 3.20 | 3.34 | 2.04 | 2.61 |
| + lazy steps | 0.20 | 0.83 | 0.05 | 0.11 | 0.01 | 0.02 | 0.08 | 0.07 | 0.01 | 0.01 | 0.78 | 1.58 | 0.74 | 1.33 | 1.73 | 2.66 | 1.02 | 1.15 | 1.26 | 1.52 | 2.96 | 1.57 | 2.68 | 2.86 | 1.14 | 1.42 |
| + scale search (= rule-aware) | 0.20 | 0.83 | 0.05 | 0.11 | 0.01 | 0.02 | 0.08 | 0.06 | 0.01 | 0.01 | 0.78 | 1.58 | 0.74 | 1.33 | 1.73 | 2.66 | 1.02 | 1.15 | 1.26 | 1.52 | 2.96 | 1.57 | 2.68 | 2.86 | 1.14 | 1.42 |
| + κ=1024 (= κ) | 0.20 | 0.83 | 0.05 | 0.11 | 0.01 | 0.02 | 0.08 | 0.06 | 0.01 | 0.01 | 0.78 | 1.57 | 0.65 | 1.28 | 1.08 | 2.36 | 0.92 | 1.08 | 1.14 | 1.42 | 2.79 | 1.41 | 2.56 | 2.75 | 1.05 | 1.23 |
| box, nearest + scale search | 2.36 | 4.72 | 0.53 | 0.98 | 0.08 | 0.17 | 0.80 | 0.83 | 0.01 | 0.01 | 1.09 | 1.59 | 0.88 | 1.45 | 2.26 | 3.37 | 1.17 | 1.41 | 1.86 | 2.31 | 3.58 | 2.56 | 3.20 | 3.34 | 2.04 | 2.61 |
| rule-aware − one-sided | 0.22 | 0.83 | 0.06 | 0.13 | 0.03 | 0.06 | 0.20 | 0.11 | 0.01 | 0.01 | 0.78 | 1.58 | 0.74 | 1.33 | 1.73 | 2.66 | 1.02 | 1.15 | 1.26 | 1.52 | 2.96 | 1.57 | 2.68 | 2.86 | 1.14 | 1.42 |
| rule-aware − lazy steps | 1.35 | 9.13 | 0.43 | 1.01 | 0.04 | 0.09 | 0.45 | 0.32 | 0.01 | 0.01 | 1.09 | 1.59 | 0.88 | 1.45 | 2.26 | 3.37 | 1.17 | 1.41 | 1.86 | 2.31 | 3.58 | 2.56 | 3.20 | 3.34 | 2.04 | 2.61 |
| κ − one-sided | 0.22 | 0.83 | 0.06 | 0.13 | 0.03 | 0.06 | 0.18 | 0.11 | 0.01 | 0.01 | 0.78 | 1.57 | 0.65 | 1.28 | 1.08 | 2.36 | 0.92 | 1.08 | 1.14 | 1.42 | 2.79 | 1.41 | 2.56 | 2.75 | 1.05 | 1.23 |
| rule-aware, fixed-width codes | 1.93 | 2.13 | 1.29 | 1.47 | 1.17 | 1.21 | 1.65 | 1.85 | 1.02 | 1.02 | 3.01 | 4.92 | 3.59 | 5.16 | 5.78 | 7.86 | 4.46 | 4.88 | 3.60 | 4.11 | 6.74 | 4.68 | 5.51 | 5.74 | 4.04 | 4.98 |

</details>

Scale search (`+ scale search` vs `+ lazy steps`) is smaller on 1 of 31 datasets, by at most 0.0057 bits per value.

Summary: mean bits per value and geometric mean of the ratio to `rule-aware` (+ scale search):

| configuration | synthetic mean (5) | synthetic ÷ rule-aware | real q95 mean (13) | real q95 ÷ rule-aware | real q50 mean (13) | real q50 ÷ rule-aware |
|---|---:|---:|---:|---:|---:|---:|
| box, nearest (Thm 1) | 1.15 | 2.34 | 1.53 | 2.51 | 1.95 | 2.40 |
| + one-sided | 1.12 | 2.21 | 1.44 | 2.17 | 2.36 | 2.25 |
| + lazy steps | 0.80 | 1.00 | 0.97 | 1.00 | 1.16 | 1.01 |
| + scale search (= rule-aware) | 0.80 | 1.00 | 0.97 | 1.00 | 1.16 | 1.00 |
| + κ=1024 (= κ) | 0.73 | 0.84 | 0.87 | 0.93 | 1.09 | 0.95 |
| box, nearest + scale search | 1.15 | 2.34 | 1.53 | 2.51 | 1.95 | 2.40 |
| rule-aware − one-sided | 0.85 | 1.32 | 0.99 | 1.24 | 1.17 | 1.22 |
| rule-aware − lazy steps | 1.12 | 2.21 | 1.41 | 2.13 | 2.25 | 2.21 |
| κ − one-sided | 0.74 | 0.92 | 0.88 | 1.10 | 1.10 | 1.12 |
| rule-aware, fixed-width codes | 2.71 | 6.87 | 3.37 | 8.48 | 3.85 | 6.92 |
