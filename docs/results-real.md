# Results (real Prometheus series)

13 series of raw 15 s scrapes (no gaps) from the public demo servers
prometheus.demo.prometheus.io (node_exporter, Prometheus, Caddy; retention
31 d) and demo.promlabs.com (a demo service), fetched with
`scripts/fetch_real.py`. Two snapshots, both committed because the demo
servers keep only 31 days and the intervals cannot be fetched again:

| snapshot | interval (UTC) | samples / series | sha256 |
|---|---|---:|---|
| `datasets/prom-demo-2026-11-02.tar.xz` (1.1 MB) | 2026-10-26 13:00 – 2026-11-02 13:00 | 40 320 | `5a4d1c34…01ad6` |
| `datasets/prom-demo-2026-11-04-30d.tar.xz` (5.1 MB) | 2026-10-05 17:00 – 2026-11-04 17:00 | 172 800 | `c6f2d066…64d2fc` |

The exact interval is in each snapshot's `manifest.txt`. The tables below
were produced on 2026-11-14 by commit `35ba3da` (one-sided budget, lazy step
choice; Corollary 1 and Proposition 1(e) of `formal-model.md`).

Reproduce (the 30-day run takes about 15 min for H8 and 20 min for the
safeguards sweep with 8 parallel jobs):

```
tar -C data -xJf datasets/prom-demo-2026-11-02.tar.xz       # -> data/real
tar -C data -xJf datasets/prom-demo-2026-11-04-30d.tar.xz   # -> data/real30
cargo run --release -p alertsafe-bench -- h8 data/real      # or data/real30
cargo run --release -p alertsafe-bench -- export DIR --real data/real
ls DIR | grep -v prod | xargs -P 8 -I{} uv run baselines/safeguards/run.py DIR --only {}
cargo run --release -p alertsafe-bench -- h6 DIR --real data/real
```

**Rules.** The demo server runs the upstream node-mixin and prometheus-mixin
alerts (51 rules). Nine of them reduce to a single series once constant
denominators (MemTotal, filesystem size, CPU count, query concurrency
limit; all checked to be constant over the interval) are folded into θ.
Their thresholds as deployed are `prod`. On a healthy host they never fire
(load max 1.91 vs θ = 4; free memory min 23 % vs 10 %), so every series also
runs with θ recalibrated to the 95th percentile (`q95`, a busy alert) and
the median (`q50`, the aggregate crosses θ all the time) of the rule's own
aggregate. A recalibrated θ is placed midway between two distinct values of
the aggregate. Metrics are often discrete (load with two decimals, integer
counter increments), so a raw quantile coincides with many evaluations, and
then no lossy codec may move those samples at all. The 7-day and 30-day
runs recalibrate θ on their own data, so their thresholds differ.

## Findings

- **Zero changed decisions** in every rule-aware cell, both snapshots
  (asserted by the bench; 1 864 firings over the 7-day variants, 7 191 over
  the 30-day ones).
- **`prod`: the deployed thresholds cost almost nothing**, for every codec:
  0.07 bits/value rule-aware vs 0.09 for a tuned uniform bound (7 d). A
  threshold that is never approached constrains nothing. This is the
  common case in production, and it says nothing about the method.
- **`q95` / `q50`: 6.5× / 4.0× below a uniform bound tuned on the data**
  (7 d, mean 1.13 vs 7.35 and 1.49 vs 6.01 bits/value), **5.9× / 5.1× on
  30 d** (0.97 vs 5.72, 1.16 vs 5.91). With the optimistic budget
  (κ = 1024): 1.01 / 1.39 (7 d) and 0.87 / 1.09 (30 d). The tuned uniform
  bound has no guarantee; it is the largest bound that happened to keep
  every decision on these data.
- **Rule-aware κ is smaller than Compression Safeguards (SZ3+SG) on 19 of
  26 datasets in both snapshots**; geometric mean of the ratio κ / SG is
  0.61 (7 d) and 0.43 (30 d). Gauges: mean 0.17 vs 0.22 (7 d) and 0.14 vs
  0.89 (30 d); counters 1.84 vs 2.39 and 1.50 vs 2.32. The 30-day SG
  gauge mean is dominated by `node_load1` (1.79 / 4.92 vs our 0.20 /
  0.83).
- **Where SG is still smaller**: `prometheus_samples_appended` (0.56–0.60
  vs 0.92–1.08 on 30 d; 1.66–2.97 vs 2.25–4.22 on 7 d), `node_cpu0_idle`
  (≈ 1.0–1.15×), `demo_api_foo_200` (0.7–0.9×), and narrowly
  `node_load1@q50` (7 d: 0.70 vs 0.75) and `demo_cpu_seconds@q50` (30 d:
  1.17 vs 1.23). All but `node_load1` are dense counters under `rate`,
  where our budget applies only at window ends but the step is still
  shared by a 128-sample block.
- **SG's `rate` translation is heuristic and still breaks**: state
  mismatches at some sweep points of `demo_cpu_seconds` (7/13 and 5/13,
  7 d), `prometheus_samples_appended@q95` (2/13, 7 d), `caddy_requests@q95`
  (2/13, 30 d) and `demo_api_foo_500@q95` (1/13, 30 d). These series have
  no missed scrapes; on synthetic `requests-gappy` (8 % missed) it broke at
  11/13 points. SG's best point is reported only among mismatch-free ones,
  but a user cannot know in advance which bound is safe.
- **Speed**: SZ3+SG takes up to 10 s per sweep point on 7 d and up to 60 s
  on 30 d for counters. The whole 30-day H8 (all variants,
  uniform sweeps, κ, chunks) takes 13.5 min on one core.

## History

- 2026-11-02, first run (box budget of Theorem 1): q95 / q50 means 2.17 /
  3.01 bits/value, and SZ3+SG was smaller on 16 of 26 datasets (gauges
  0.15 / 0.29 vs our 0.89 / 1.84). Diagnosis: one quantizer step per
  128-sample block, so the tightest budget near θ sets the step for the
  whole block. The one-sided budget (most samples near θ are constrained
  in one direction only) and the lazy step choice (keep the prediction
  while it is on the admissible side) removed most of that gap without
  changing the format.
- Codec fixes found on these data in that run: coded size is not monotone
  in the budget (`FINER_STEPS` candidates), `Options::scales` (also encode
  the budget × ½, ¼, ⅛, keep the smallest), and the bench's `tuned()` sweep
  now starts at ε = 0.
- Lossless real gauges are expensive: 54.6 bits/value for `node_load1`
  (two-decimal values) and 55.6 for `node_timex_offset`. XOR coding cannot
  use a decimal lattice; an ALP-style decimal mode would help both the
  lossless and the escape paths.

## 7 days

### H8: real Prometheus series (13 series, 7.0 days, raw 15s scrapes)

Sources: `prom-demo` = prometheus.demo.prometheus.io (node_exporter, Prometheus, Caddy),
`promlabs` = demo.promlabs.com (demo service). Rules: `prod` = upstream node-/prometheus-mixin
thresholds restricted to one series; `q95` / `q50` = θ recalibrated to the 95th percentile / median
of the rule's aggregate (see `real.rs`). Same columns as H1, plus H7 (κ=1024) and
Prometheus-sized chunks of 120. Zero changed decisions in every rule-aware cell (asserted).

| series | kind | rules | firings | lossless | uniform-safe | uniform-tuned | rule-aware | κ=1024 | rule-aware /120 | κ /120 | ×lossless | ×tuned | ×tuned (κ) | exact samples |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| node_load1 | gauge (prom-demo) | prod | 0 | 54.56 | 0.00 | 0.00 | 0.00 | 0.00 | 0.21 | 0.21 | 10998.6 | 1.0 | 1.0 | 0.00% |
| node_load1 | gauge (prom-demo) | q95 | 0 | 54.56 | 5.01 | 3.19 | 0.20 | 0.20 | 0.42 | 0.42 | 277.2 | 16.2 | 16.2 | 0.00% |
| node_load1 | gauge (prom-demo) | q50 | 8 | 54.56 | 5.01 | 5.02 | 0.75 | 0.75 | 1.06 | 1.06 | 72.3 | 6.6 | 6.6 | 0.00% |
| node_memory_available | gauge (prom-demo) | prod | 0 | 15.61 | 0.01 | 0.00 | 0.00 | 0.00 | 0.33 | 0.33 | 3278.6 | 1.0 | 1.0 | 0.00% |
| node_memory_available | gauge (prom-demo) | q95 | 2 | 15.61 | 11.33 | 8.32 | 0.13 | 0.13 | 0.46 | 0.46 | 119.9 | 63.9 | 63.9 | 0.00% |
| node_memory_available | gauge (prom-demo) | q50 | 80 | 15.61 | 11.33 | 10.33 | 0.37 | 0.37 | 0.70 | 0.70 | 41.9 | 27.7 | 27.7 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | prod | 0 | 13.13 | 0.00 | 0.00 | 0.00 | 0.00 | 0.33 | 0.33 | 2757.1 | 1.0 | 1.0 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | q95 | 12 | 13.13 | 0.80 | 1.18 | 0.01 | 0.01 | 0.33 | 0.33 | 1181.6 | 105.8 | 105.8 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | q50 | 34 | 13.13 | 2.64 | 1.18 | 0.02 | 0.02 | 0.31 | 0.31 | 735.2 | 65.8 | 65.8 | 0.00% |
| node_timex_offset | gauge (prom-demo) | prod | 0 | 55.62 | 0.04 | 0.04 | 0.04 | 0.04 | 0.24 | 0.24 | 1566.1 | 1.0 | 1.0 | 0.00% |
| node_timex_offset | gauge (prom-demo) | q95 | 32 | 55.62 | 2.73 | 2.88 | 0.12 | 0.12 | 0.36 | 0.36 | 471.1 | 24.4 | 24.4 | 0.00% |
| node_timex_offset | gauge (prom-demo) | q50 | 185 | 55.62 | 5.03 | 4.16 | 0.08 | 0.08 | 0.37 | 0.37 | 715.1 | 53.5 | 53.5 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | prod | 0 | 0.02 | 0.01 | 0.01 | 0.01 | 0.01 | 0.14 | 0.14 | 4.2 | 1.0 | 1.0 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | q95 | 0 | 0.02 | 0.02 | 0.01 | 0.01 | 0.01 | 0.14 | 0.14 | 2.1 | 1.0 | 1.1 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | q50 | 0 | 0.02 | 0.02 | 0.01 | 0.01 | 0.01 | 0.14 | 0.14 | 2.1 | 1.0 | 1.1 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | prod | 0 | 7.78 | 0.01 | 0.01 | 0.00 | 0.00 | 0.33 | 0.27 | 1632.9 | 1.1 | 1.2 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | q95 | 0 | 7.78 | 3.08 | 1.54 | 0.79 | 0.74 | 1.47 | 1.43 | 9.9 | 2.0 | 2.1 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | q50 | 21 | 7.78 | 7.75 | 1.54 | 1.54 | 1.54 | 2.29 | 2.29 | 5.1 | 1.0 | 1.0 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | prod | 0 | 39.43 | 0.01 | 0.01 | 0.01 | 0.00 | 0.40 | 0.27 | 5230.1 | 1.0 | 1.6 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | q95 | 0 | 39.43 | 5.94 | 3.95 | 0.71 | 0.61 | 1.31 | 1.08 | 55.6 | 5.6 | 6.5 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | q50 | 0 | 39.43 | 9.02 | 10.02 | 1.39 | 1.33 | 2.07 | 2.00 | 28.5 | 7.2 | 7.5 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | prod | 0 | 40.88 | 0.45 | 0.52 | 0.35 | 0.35 | 1.17 | 1.17 | 116.8 | 1.5 | 1.5 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | q95 | 1 | 40.88 | 8.16 | 7.16 | 2.02 | 1.31 | 3.03 | 2.13 | 20.2 | 3.5 | 5.5 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | q50 | 73 | 40.88 | 10.12 | 9.15 | 3.51 | 2.95 | 4.60 | 3.93 | 11.6 | 2.6 | 3.1 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | prod | 0 | 22.34 | 0.19 | 0.20 | 0.20 | 0.20 | 0.86 | 0.86 | 109.2 | 1.0 | 1.0 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | q95 | 0 | 22.34 | 11.07 | 10.11 | 2.47 | 2.25 | 3.57 | 3.22 | 9.0 | 4.1 | 4.5 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | q50 | 146 | 22.34 | 11.07 | 10.11 | 4.45 | 4.22 | 5.64 | 5.37 | 5.0 | 2.3 | 2.4 | 0.00% |
| caddy_requests | counter (prom-demo) | q95 | 46 | 13.79 | 7.17 | 5.16 | 1.38 | 1.30 | 2.21 | 2.04 | 10.0 | 3.8 | 4.0 | 0.00% |
| caddy_requests | counter (prom-demo) | q50 | 135 | 13.79 | 7.17 | 5.16 | 1.39 | 1.31 | 2.21 | 2.01 | 10.0 | 3.7 | 3.9 | 0.00% |
| demo_api_foo_200 | counter (promlabs) | q95 | 0 | 17.15 | 12.54 | 6.26 | 3.02 | 2.82 | 4.02 | 3.76 | 5.7 | 2.1 | 2.2 | 0.00% |
| demo_api_foo_200 | counter (promlabs) | q50 | 336 | 17.15 | 7.99 | 6.07 | 1.56 | 1.43 | 2.55 | 2.22 | 11.0 | 3.9 | 4.2 | 0.00% |
| demo_api_foo_500 | counter (promlabs) | q95 | 1 | 9.76 | 9.76 | 4.14 | 2.68 | 2.56 | 3.66 | 3.54 | 3.6 | 1.5 | 1.6 | 0.00% |
| demo_api_foo_500 | counter (promlabs) | q50 | 387 | 9.76 | 9.76 | 4.14 | 2.90 | 2.79 | 3.82 | 3.70 | 3.4 | 1.4 | 1.5 | 0.00% |
| demo_cpu_seconds | counter (promlabs) | q95 | 52 | 41.71 | 12.28 | 41.71 | 1.21 | 1.11 | 2.03 | 1.80 | 34.6 | 34.6 | 37.6 | 0.01% |
| demo_cpu_seconds | counter (promlabs) | q50 | 313 | 41.71 | 13.21 | 11.32 | 1.42 | 1.25 | 2.33 | 2.02 | 29.3 | 8.0 | 9.0 | 0.00% |

<details><summary>Thresholds</summary>

| series | rules | rule | expression | for | firings |
|---|---|---|---|---:|---:|
| node_load1 | prod | NodeSystemSaturation | `Last(x) > 4e0` | 15m | 0 |
| node_load1 | q95 | NodeSystemSaturation | `Last(x) > 6.05e-1` | 15m | 0 |
| node_load1 | q50 | NodeSystemSaturation | `Last(x) > 1.65e-1` | 15m | 8 |
| node_memory_available | prod | NodeMemoryHighUtilization | `Last(x) < 4.1153413120000005e8` | 15m | 0 |
| node_memory_available | q95 | NodeMemoryHighUtilization | `Last(x) < 2.50175488e9` | 15m | 2 |
| node_memory_available | q50 | NodeMemoryHighUtilization | `Last(x) < 2.528796672e9` | 15m | 80 |
| node_fs_root_avail | prod | NodeFilesystemAlmostOutOfSpace | `Last(x) < 2.6295513088e9` | 30m | 0 |
| node_fs_root_avail | prod | NodeFilesystemAlmostOutOfSpaceCrit | `Last(x) < 1.57773078528e9` | 30m | 0 |
| node_fs_root_avail | q95 | NodeFilesystemAlmostOutOfSpace | `Last(x) < 3.3385668608e10` | 30m | 6 |
| node_fs_root_avail | q95 | NodeFilesystemAlmostOutOfSpaceCrit | `Last(x) < 3.3385668608e10` | 30m | 6 |
| node_fs_root_avail | q50 | NodeFilesystemAlmostOutOfSpace | `Last(x) < 3.3541894144e10` | 30m | 17 |
| node_fs_root_avail | q50 | NodeFilesystemAlmostOutOfSpaceCrit | `Last(x) < 3.3541894144e10` | 30m | 17 |
| node_timex_offset | prod | NodeClockSkewDetected | `Last(x) > 5e-2` | 10m | 0 |
| node_timex_offset | prod | NodeClockSkewDetectedNeg | `Last(x) < -5e-2` | 10m | 0 |
| node_timex_offset | q95 | NodeClockSkewDetected | `Last(x) > 3.43455e-5` | 10m | 14 |
| node_timex_offset | q95 | NodeClockSkewDetectedNeg | `Last(x) < -3.5524e-5` | 10m | 18 |
| node_timex_offset | q50 | NodeClockSkewDetected | `Last(x) > 4.7499999999999995e-8` | 10m | 95 |
| node_timex_offset | q50 | NodeClockSkewDetectedNeg | `Last(x) < 4.7499999999999995e-8` | 10m | 90 |
| prometheus_engine_queries | prod | PrometheusHighQueryLoad | `AvgOverTime(x[5m]) > 1.6e1` | 15m | 0 |
| prometheus_engine_queries | q95 | PrometheusHighQueryLoad | `AvgOverTime(x[5m]) > 2.5e-2` | 15m | 0 |
| prometheus_engine_queries | q50 | PrometheusHighQueryLoad | `AvgOverTime(x[5m]) > 2.5e-2` | 15m | 0 |
| node_vmstat_pgmajfault | prod | NodeMemoryMajorPagesFaults | `Rate(x[5m]) > 5e2` | 15m | 0 |
| node_vmstat_pgmajfault | q95 | NodeMemoryMajorPagesFaults | `Rate(x[5m]) > 1.5964912280701754e-1` | 15m | 0 |
| node_vmstat_pgmajfault | q50 | NodeMemoryMajorPagesFaults | `Rate(x[5m]) > 9.473717451756819e-2` | 15m | 21 |
| node_disk_io_weighted_sda | prod | NodeDiskIOSaturation | `Rate(x[5m]) > 1e1` | 30m | 0 |
| node_disk_io_weighted_sda | q95 | NodeDiskIOSaturation | `Rate(x[5m]) > 7.491052631582869e-2` | 30m | 0 |
| node_disk_io_weighted_sda | q50 | NodeDiskIOSaturation | `Rate(x[5m]) > 1.4973684210481383e-2` | 30m | 0 |
| node_cpu0_idle | prod | NodeCPUHighUsage | `Rate(x[2m]) < 1e-1` | 15m | 0 |
| node_cpu0_idle | q95 | NodeCPUHighUsage | `Rate(x[2m]) < 8.904285714296358e-1` | 15m | 1 |
| node_cpu0_idle | q50 | NodeCPUHighUsage | `Rate(x[2m]) < 9.179174121859681e-1` | 15m | 73 |
| prometheus_samples_appended | prod | PrometheusNotIngestingSamples | `Rate(x[5m]) <= 0e0` | 10m | 0 |
| prometheus_samples_appended | q95 | PrometheusNotIngestingSamples | `Rate(x[5m]) <= 1.1148894736842105e3` | 10m | 0 |
| prometheus_samples_appended | q50 | PrometheusNotIngestingSamples | `Rate(x[5m]) <= 1.1179140350877192e3` | 10m | 146 |
| caddy_requests | q95 | RateHigh | `Rate(x[5m]) > 4.082456140350877e0` | 5m | 33 |
| caddy_requests | q95 | RateLow | `Rate(x[5m]) < 2.3421052631578947e0` | 10m | 13 |
| caddy_requests | q50 | RateHigh | `Rate(x[5m]) > 3.4122807017543857e0` | 5m | 85 |
| caddy_requests | q50 | RateLow | `Rate(x[5m]) < 3.4122807017543857e0` | 10m | 50 |
| demo_api_foo_200 | q95 | RateHigh | `Rate(x[5m]) > 4.171396190238052e1` | 5m | 0 |
| demo_api_foo_200 | q95 | RateLow | `Rate(x[5m]) < 2.416846345351904e1` | 10m | 0 |
| demo_api_foo_200 | q50 | RateHigh | `Rate(x[5m]) > 3.054572120727717e1` | 5m | 168 |
| demo_api_foo_200 | q50 | RateLow | `Rate(x[5m]) < 3.054572120727717e1` | 10m | 168 |
| demo_api_foo_500 | q95 | RateHigh | `Rate(x[5m]) > 3.5438534318469916e-1` | 5m | 1 |
| demo_api_foo_500 | q95 | RateLow | `Rate(x[5m]) < 1.649125700225602e-1` | 10m | 0 |
| demo_api_foo_500 | q50 | RateHigh | `Rate(x[5m]) > 2.4561446599104717e-1` | 5m | 235 |
| demo_api_foo_500 | q50 | RateLow | `Rate(x[5m]) < 2.4561446599104717e-1` | 10m | 152 |
| demo_cpu_seconds | q95 | RateHigh | `Rate(x[5m]) > 1.288774190837269e-1` | 5m | 28 |
| demo_cpu_seconds | q95 | RateLow | `Rate(x[5m]) < 8.052659833898237e-2` | 10m | 24 |
| demo_cpu_seconds | q50 | RateHigh | `Rate(x[5m]) > 9.961420984984659e-2` | 5m | 163 |
| demo_cpu_seconds | q50 | RateLow | `Rate(x[5m]) < 9.961420984984659e-2` | 10m | 150 |

</details>

Mean bits per value over the series of each rule variant (every series has the same length):

| rules | series | lossless | uniform-safe | uniform-tuned | rule-aware | κ=1024 | rule-aware /120 | κ /120 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| prod | 9 | 27.71 | 0.08 | 0.09 | 0.07 | 0.07 | 0.45 | 0.42 |
| q95 | 13 | 25.52 | 6.91 | 7.35 | 1.13 | 1.01 | 1.77 | 1.59 |
| q50 | 13 | 25.52 | 7.70 | 6.01 | 1.49 | 1.39 | 2.16 | 2.01 |

### H6: Compression Safeguards baseline (real Prometheus series)

`SZ3` is SZ3 with an absolute error bound; `SZ3+SG` wraps it in safeguards translated from the rules
(see `baselines/safeguards/run.py`); both are swept over the SZ3 bound. `tuned` = largest bound with no
changed decision and none below it; `SZ3+SG best` = smallest output with zero state mismatches, if any.
`rule-aware κ` is the optimistic budget with a posteriori repair (H7).
Mismatches are counted by the reference evaluator of this repository.

| dataset | lossless (ours) | uniform-tuned (ours) | rule-aware (ours) | rule-aware κ=1024 (ours) | SZ3 tuned | SZ3+SG best | SZ3+SG ε / range | SZ3+SG points with mismatches | SZ3+SG encode, s (mean) | ×SZ3+SG |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| node_load1@q95 | 54.56 | 3.19 | 0.20 | 0.20 | 7.09 | 0.25 | 1.0e0 | 0/13 | 0.0 | 1.3 |
| node_load1@q50 | 54.56 | 5.02 | 0.75 | 0.75 | 7.09 | 0.70 | 1.0e0 | 0/13 | 0.0 | 0.9 |
| node_memory_available@q95 | 15.61 | 8.32 | 0.13 | 0.13 | 15.81 | 0.19 | 1.0e0 | 0/13 | 0.0 | 1.5 |
| node_memory_available@q50 | 15.61 | 10.33 | 0.37 | 0.37 | 19.32 | 0.45 | 1.0e0 | 0/13 | 0.0 | 1.2 |
| node_fs_root_avail@q95 | 13.13 | 1.18 | 0.01 | 0.01 | 2.03 | 0.11 | 1.0e0 | 0/13 | 0.0 | 9.7 |
| node_fs_root_avail@q50 | 13.13 | 1.18 | 0.02 | 0.02 | 3.55 | 0.15 | 6.2e-2 | 0/13 | 0.0 | 8.3 |
| node_timex_offset@q95 | 55.62 | 2.88 | 0.12 | 0.12 | 3.08 | 0.17 | 1.0e0 | 0/13 | 0.0 | 1.4 |
| node_timex_offset@q50 | 55.62 | 4.16 | 0.08 | 0.08 | 12.80 | 0.10 | 1.0e0 | 0/13 | 0.0 | 1.3 |
| prometheus_engine_queries@q95 | 0.02 | 0.01 | 0.01 | 0.01 | 0.04 | 0.04 | 2.5e-1 | 0/13 | 0.4 | 4.5 |
| prometheus_engine_queries@q50 | 0.02 | 0.01 | 0.01 | 0.01 | 0.04 | 0.04 | 2.5e-1 | 0/13 | 0.4 | 4.5 |
| node_vmstat_pgmajfault@q95 | 7.78 | 1.54 | 0.79 | 0.74 | – | 1.79 | 1.6e-2 | 0/13 | 3.2 | 2.4 |
| node_vmstat_pgmajfault@q50 | 7.78 | 1.54 | 1.54 | 1.54 | – | 3.60 | 1.6e-2 | 0/13 | 3.3 | 2.3 |
| node_disk_io_weighted_sda@q95 | 39.43 | 3.95 | 0.71 | 0.61 | 16.73 | 3.43 | 2.4e-4 | 0/13 | 3.0 | 5.7 |
| node_disk_io_weighted_sda@q50 | 39.43 | 10.02 | 1.39 | 1.33 | – | 3.38 | 2.4e-4 | 0/13 | 3.4 | 2.5 |
| node_cpu0_idle@q95 | 40.88 | 7.16 | 2.02 | 1.31 | – | 1.16 | 3.9e-3 | 0/13 | 0.7 | 0.9 |
| node_cpu0_idle@q50 | 40.88 | 9.15 | 3.51 | 2.95 | – | 2.79 | 9.5e-7 | 0/13 | 0.9 | 0.9 |
| prometheus_samples_appended@q95 | 22.34 | 10.11 | 2.47 | 2.25 | – | 1.66 | 9.5e-7 | 2/13 | 2.9 | 0.7 |
| prometheus_samples_appended@q50 | 22.34 | 10.11 | 4.45 | 4.22 | – | 2.97 | 9.5e-7 | 0/13 | 3.0 | 0.7 |
| caddy_requests@q95 | 13.79 | 5.16 | 1.38 | 1.30 | 8.31 | 1.38 | 3.9e-3 | 0/13 | 8.5 | 1.1 |
| caddy_requests@q50 | 13.79 | 5.16 | 1.39 | 1.31 | 8.31 | 1.54 | 3.9e-3 | 0/13 | 8.6 | 1.2 |
| demo_api_foo_200@q95 | 17.15 | 6.26 | 3.02 | 2.82 | – | 2.02 | 2.5e-1 | 0/13 | 9.1 | 0.7 |
| demo_api_foo_200@q50 | 17.15 | 6.07 | 1.56 | 1.43 | – | 1.15 | 2.5e-1 | 0/13 | 9.2 | 0.8 |
| demo_api_foo_500@q95 | 9.76 | 4.14 | 2.68 | 2.56 | – | 4.27 | 3.9e-3 | 0/13 | 9.8 | 1.7 |
| demo_api_foo_500@q50 | 9.76 | 4.14 | 2.90 | 2.79 | – | 4.33 | 3.9e-3 | 0/13 | 10.5 | 1.6 |
| demo_cpu_seconds@q95 | 41.71 | 41.71 | 1.21 | 1.11 | – | 1.30 | 1.5e-5 | 7/13 | 9.1 | 1.2 |
| demo_cpu_seconds@q50 | 41.71 | 11.32 | 1.42 | 1.25 | – | 1.40 | 1.5e-5 | 5/13 | 10.0 | 1.1 |

Values are bits per value (timestamps excluded). `×SZ3+SG` = SZ3+SG best / rule-aware κ=1024.

<details><summary>SZ3 / SZ3+SG sweep, <code>node_load1@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 10.44 | 0 | 10.44 | 0 | 0.0 |
| 2.4e-7 | 10.08 | 0 | 10.09 | 0 | 0.0 |
| 9.5e-7 | 8.33 | 0 | 8.33 | 0 | 0.0 |
| 3.8e-6 | 7.03 | 0 | 7.03 | 0 | 0.0 |
| 1.5e-5 | 7.40 | 0 | 7.40 | 0 | 0.0 |
| 6.1e-5 | 7.41 | 0 | 7.41 | 0 | 0.0 |
| 2.4e-4 | 7.39 | 0 | 7.39 | 0 | 0.0 |
| 9.8e-4 | 7.09 | 0 | 7.09 | 0 | 0.0 |
| 3.9e-3 | 4.86 | 17 | 4.92 | 0 | 0.0 |
| 1.6e-2 | 2.77 | 61 | 2.98 | 0 | 0.0 |
| 6.2e-2 | 1.07 | 272 | 1.77 | 0 | 0.0 |
| 2.5e-1 | 0.34 | 652 | 1.95 | 0 | 0.0 |
| 1.0e0 | 0.03 | 970 | 0.25 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_load1@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 10.44 | 0 | 10.44 | 0 | 0.0 |
| 2.4e-7 | 10.08 | 0 | 10.09 | 0 | 0.0 |
| 9.5e-7 | 8.33 | 0 | 8.33 | 0 | 0.0 |
| 3.8e-6 | 7.03 | 0 | 7.03 | 0 | 0.0 |
| 1.5e-5 | 7.40 | 0 | 7.40 | 0 | 0.0 |
| 6.1e-5 | 7.41 | 0 | 7.41 | 0 | 0.0 |
| 2.4e-4 | 7.39 | 0 | 7.39 | 0 | 0.0 |
| 9.8e-4 | 7.09 | 0 | 7.09 | 0 | 0.0 |
| 3.9e-3 | 4.86 | 203 | 5.41 | 0 | 0.0 |
| 1.6e-2 | 2.77 | 823 | 4.85 | 0 | 0.0 |
| 6.2e-2 | 1.07 | 3102 | 7.22 | 0 | 0.0 |
| 2.5e-1 | 0.34 | 6617 | 11.80 | 0 | 0.0 |
| 1.0e0 | 0.03 | 9614 | 0.70 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_memory_available@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 19.32 | 0 | 19.32 | 0 | 0.0 |
| 2.4e-7 | 19.32 | 0 | 19.32 | 0 | 0.0 |
| 9.5e-7 | 19.32 | 0 | 19.32 | 0 | 0.0 |
| 3.8e-6 | 15.81 | 0 | 15.81 | 0 | 0.0 |
| 1.5e-5 | 10.44 | 4 | 10.45 | 0 | 0.0 |
| 6.1e-5 | 7.14 | 10 | 7.17 | 0 | 0.0 |
| 2.4e-4 | 4.56 | 55 | 4.62 | 0 | 0.0 |
| 9.8e-4 | 2.47 | 122 | 2.55 | 0 | 0.0 |
| 3.9e-3 | 1.33 | 512 | 2.21 | 0 | 0.0 |
| 1.6e-2 | 0.39 | 1378 | 1.81 | 0 | 0.0 |
| 6.2e-2 | 0.04 | 20147 | 0.20 | 0 | 0.0 |
| 2.5e-1 | 0.06 | 1409 | 1.37 | 0 | 0.0 |
| 1.0e0 | 0.04 | 1004 | 0.19 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_memory_available@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 19.32 | 0 | 19.32 | 0 | 0.0 |
| 2.4e-7 | 19.32 | 0 | 19.32 | 0 | 0.0 |
| 9.5e-7 | 19.32 | 0 | 19.32 | 0 | 0.0 |
| 3.8e-6 | 15.81 | 5 | 15.83 | 0 | 0.0 |
| 1.5e-5 | 10.44 | 10 | 10.46 | 0 | 0.0 |
| 6.1e-5 | 7.14 | 51 | 7.20 | 0 | 0.0 |
| 2.4e-4 | 4.56 | 121 | 4.66 | 0 | 0.0 |
| 9.8e-4 | 2.47 | 82 | 2.55 | 0 | 0.0 |
| 3.9e-3 | 1.33 | 1640 | 4.18 | 0 | 0.0 |
| 1.6e-2 | 0.39 | 5966 | 4.12 | 0 | 0.0 |
| 6.2e-2 | 0.04 | 18555 | 0.45 | 0 | 0.0 |
| 2.5e-1 | 0.06 | 12611 | 1.70 | 0 | 0.0 |
| 1.0e0 | 0.04 | 10078 | 0.45 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_fs_root_avail@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.72 | 0 | 5.72 | 0 | 0.0 |
| 2.4e-7 | 5.78 | 0 | 5.78 | 0 | 0.0 |
| 9.5e-7 | 5.93 | 0 | 5.93 | 0 | 0.0 |
| 3.8e-6 | 3.55 | 0 | 3.55 | 0 | 0.0 |
| 1.5e-5 | 2.03 | 0 | 2.03 | 0 | 0.0 |
| 6.1e-5 | 0.79 | 2 | 0.80 | 0 | 0.0 |
| 2.4e-4 | 0.47 | 0 | 0.47 | 0 | 0.0 |
| 9.8e-4 | 0.37 | 70 | 0.39 | 0 | 0.0 |
| 3.9e-3 | 0.27 | 358 | 0.30 | 0 | 0.0 |
| 1.6e-2 | 0.23 | 400 | 0.28 | 0 | 0.0 |
| 6.2e-2 | 0.10 | 1480 | 0.13 | 0 | 0.0 |
| 2.5e-1 | 0.06 | 1998 | 0.13 | 0 | 0.0 |
| 1.0e0 | 0.04 | 2018 | 0.11 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_fs_root_avail@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.72 | 0 | 5.73 | 0 | 0.0 |
| 2.4e-7 | 5.78 | 0 | 5.78 | 0 | 0.0 |
| 9.5e-7 | 5.93 | 0 | 5.94 | 0 | 0.0 |
| 3.8e-6 | 3.55 | 0 | 3.55 | 0 | 0.0 |
| 1.5e-5 | 2.03 | 4 | 2.05 | 0 | 0.0 |
| 6.1e-5 | 0.79 | 16 | 0.80 | 0 | 0.0 |
| 2.4e-4 | 0.47 | 4 | 0.49 | 0 | 0.0 |
| 9.8e-4 | 0.37 | 244 | 0.40 | 0 | 0.0 |
| 3.9e-3 | 0.27 | 508 | 0.31 | 0 | 0.0 |
| 1.6e-2 | 0.23 | 1204 | 0.32 | 0 | 0.0 |
| 6.2e-2 | 0.10 | 5510 | 0.15 | 0 | 0.0 |
| 2.5e-1 | 0.06 | 7680 | 0.21 | 0 | 0.0 |
| 1.0e0 | 0.04 | 7522 | 0.16 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_timex_offset@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 14.66 | 0 | 14.66 | 0 | 0.0 |
| 2.4e-7 | 12.80 | 0 | 12.80 | 0 | 0.0 |
| 9.5e-7 | 10.50 | 0 | 10.50 | 0 | 0.0 |
| 3.8e-6 | 7.82 | 0 | 7.83 | 0 | 0.0 |
| 1.5e-5 | 3.08 | 0 | 3.08 | 0 | 0.0 |
| 6.1e-5 | 2.47 | 4 | 2.49 | 0 | 0.0 |
| 2.4e-4 | 1.66 | 12 | 1.70 | 0 | 0.0 |
| 9.8e-4 | 0.89 | 41 | 0.96 | 0 | 0.0 |
| 3.9e-3 | 0.47 | 80 | 0.55 | 0 | 0.0 |
| 1.6e-2 | 0.40 | 618 | 0.56 | 0 | 0.0 |
| 6.2e-2 | 0.12 | 1205 | 0.25 | 0 | 0.0 |
| 2.5e-1 | 0.04 | 1978 | 0.18 | 0 | 0.0 |
| 1.0e0 | 0.02 | 2017 | 0.17 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_timex_offset@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 14.66 | 0 | 14.66 | 0 | 0.0 |
| 2.4e-7 | 12.80 | 0 | 12.80 | 0 | 0.0 |
| 9.5e-7 | 10.50 | 8 | 10.53 | 0 | 0.1 |
| 3.8e-6 | 7.82 | 49 | 7.88 | 0 | 0.0 |
| 1.5e-5 | 3.08 | 25 | 3.12 | 0 | 0.0 |
| 6.1e-5 | 2.47 | 230 | 2.50 | 0 | 0.0 |
| 2.4e-4 | 1.66 | 1682 | 1.77 | 0 | 0.0 |
| 9.8e-4 | 0.89 | 5311 | 1.00 | 0 | 0.0 |
| 3.9e-3 | 0.47 | 11927 | 0.62 | 0 | 0.0 |
| 1.6e-2 | 0.40 | 12985 | 6.94 | 0 | 0.0 |
| 6.2e-2 | 0.12 | 21259 | 0.21 | 0 | 0.0 |
| 2.5e-1 | 0.04 | 21938 | 0.12 | 0 | 0.0 |
| 1.0e0 | 0.02 | 21982 | 0.10 | 0 | 0.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_engine_queries@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 2.4e-7 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 9.5e-7 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 3.8e-6 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 1.5e-5 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 6.1e-5 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 2.4e-4 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 9.8e-4 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 3.9e-3 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 1.6e-2 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 6.2e-2 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 2.5e-1 | 0.04 | 0 | 0.04 | 0 | 0.0 |
| 1.0e0 | 0.03 | 131 | 0.06 | 0 | 4.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_engine_queries@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 2.4e-7 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 9.5e-7 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 3.8e-6 | 0.04 | 0 | 0.05 | 0 | 0.0 |
| 1.5e-5 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 6.1e-5 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 2.4e-4 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 9.8e-4 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 3.9e-3 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 1.6e-2 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 6.2e-2 | 0.05 | 0 | 0.05 | 0 | 0.0 |
| 2.5e-1 | 0.04 | 0 | 0.04 | 0 | 0.0 |
| 1.0e0 | 0.03 | 131 | 0.06 | 0 | 4.2 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_vmstat_pgmajfault@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.54 | 19396 | 5.84 | 0 | 4.7 |
| 2.4e-7 | 4.47 | 18889 | 5.99 | 0 | 4.1 |
| 9.5e-7 | 4.44 | 19684 | 5.88 | 0 | 3.4 |
| 3.8e-6 | 3.87 | 19770 | 5.45 | 0 | 3.4 |
| 1.5e-5 | 1.81 | 19443 | 3.91 | 0 | 3.5 |
| 6.1e-5 | 0.71 | 1597 | 3.04 | 0 | 3.6 |
| 2.4e-4 | 0.29 | 1362 | 2.60 | 0 | 3.5 |
| 9.8e-4 | 0.11 | 2089 | 2.42 | 0 | 3.4 |
| 3.9e-3 | 0.06 | 1679 | 2.17 | 0 | 2.8 |
| 1.6e-2 | 0.06 | 1210 | 1.79 | 0 | 2.5 |
| 6.2e-2 | 0.04 | 1002 | 1.93 | 0 | 1.9 |
| 2.5e-1 | 0.04 | 1002 | 1.93 | 0 | 2.1 |
| 1.0e0 | 0.04 | 1002 | 1.93 | 0 | 2.2 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_vmstat_pgmajfault@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.54 | 18516 | 7.83 | 0 | 5.0 |
| 2.4e-7 | 4.47 | 17916 | 8.57 | 0 | 5.2 |
| 9.5e-7 | 4.44 | 19300 | 9.17 | 0 | 3.3 |
| 3.8e-6 | 3.87 | 19735 | 8.97 | 0 | 3.3 |
| 1.5e-5 | 1.81 | 18802 | 8.28 | 0 | 3.3 |
| 6.1e-5 | 0.71 | 6984 | 8.29 | 0 | 3.4 |
| 2.4e-4 | 0.29 | 16431 | 9.61 | 0 | 3.5 |
| 9.8e-4 | 0.11 | 17431 | 9.53 | 0 | 3.7 |
| 3.9e-3 | 0.06 | 18171 | 9.11 | 0 | 2.9 |
| 1.6e-2 | 0.06 | 9752 | 3.60 | 0 | 2.9 |
| 6.2e-2 | 0.04 | 19898 | 8.29 | 0 | 2.4 |
| 2.5e-1 | 0.04 | 19898 | 8.29 | 0 | 2.4 |
| 1.0e0 | 0.04 | 19898 | 8.29 | 0 | 2.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_disk_io_weighted_sda@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 16.73 | 0 | 16.73 | 0 | 0.0 |
| 2.4e-7 | 8.54 | 1493 | 8.91 | 0 | 3.5 |
| 9.5e-7 | 5.26 | 3536 | 6.68 | 0 | 3.1 |
| 3.8e-6 | 2.23 | 733 | 5.16 | 0 | 2.4 |
| 1.5e-5 | 1.14 | 66 | 4.61 | 0 | 3.6 |
| 6.1e-5 | 0.70 | 149 | 3.84 | 0 | 3.8 |
| 2.4e-4 | 0.35 | 597 | 3.43 | 0 | 3.1 |
| 9.8e-4 | 0.21 | 4926 | 5.77 | 0 | 3.3 |
| 3.9e-3 | 0.07 | 1689 | 4.17 | 0 | 3.2 |
| 1.6e-2 | 0.04 | 1008 | 3.50 | 0 | 3.1 |
| 6.2e-2 | 0.04 | 1008 | 3.50 | 0 | 3.2 |
| 2.5e-1 | 0.04 | 1008 | 3.50 | 0 | 3.2 |
| 1.0e0 | 0.04 | 1008 | 3.50 | 0 | 3.5 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_disk_io_weighted_sda@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 16.73 | 2 | 16.75 | 0 | 3.9 |
| 2.4e-7 | 8.54 | 792 | 8.91 | 0 | 3.7 |
| 9.5e-7 | 5.26 | 2312 | 6.68 | 0 | 3.6 |
| 3.8e-6 | 2.23 | 480 | 5.16 | 0 | 2.3 |
| 1.5e-5 | 1.14 | 95 | 4.61 | 0 | 3.8 |
| 6.1e-5 | 0.70 | 366 | 3.81 | 0 | 3.4 |
| 2.4e-4 | 0.35 | 1638 | 3.38 | 0 | 3.4 |
| 9.8e-4 | 0.21 | 19098 | 5.67 | 0 | 3.4 |
| 3.9e-3 | 0.07 | 19852 | 4.02 | 0 | 3.5 |
| 1.6e-2 | 0.04 | 20129 | 3.39 | 0 | 3.5 |
| 6.2e-2 | 0.04 | 20129 | 3.39 | 0 | 3.5 |
| 2.5e-1 | 0.04 | 20129 | 3.39 | 0 | 2.9 |
| 1.0e0 | 0.04 | 20129 | 3.39 | 0 | 2.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_cpu0_idle@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.49 | 31 | 4.61 | 0 | 0.8 |
| 2.4e-7 | 2.42 | 117 | 2.67 | 0 | 0.8 |
| 9.5e-7 | 0.98 | 327 | 1.45 | 0 | 0.7 |
| 3.8e-6 | 0.50 | 733 | 1.38 | 0 | 0.9 |
| 1.5e-5 | 0.16 | 1213 | 1.49 | 0 | 0.6 |
| 6.1e-5 | 0.09 | 1363 | 1.34 | 0 | 0.7 |
| 2.4e-4 | 0.06 | 1061 | 1.25 | 0 | 0.6 |
| 9.8e-4 | 0.05 | 1008 | 1.16 | 0 | 0.4 |
| 3.9e-3 | 0.05 | 1008 | 1.16 | 0 | 0.6 |
| 1.6e-2 | 0.05 | 1008 | 1.18 | 0 | 0.6 |
| 6.2e-2 | 0.05 | 1008 | 1.16 | 0 | 1.0 |
| 2.5e-1 | 0.05 | 1008 | 1.16 | 0 | 0.7 |
| 1.0e0 | 0.03 | 20155 | 1.63 | 0 | 0.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_cpu0_idle@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.49 | 332 | 4.91 | 0 | 0.7 |
| 2.4e-7 | 2.42 | 1299 | 3.47 | 0 | 0.9 |
| 9.5e-7 | 0.98 | 3266 | 2.79 | 0 | 0.9 |
| 3.8e-6 | 0.50 | 4929 | 2.95 | 0 | 1.1 |
| 1.5e-5 | 0.16 | 8002 | 2.97 | 0 | 0.9 |
| 6.1e-5 | 0.09 | 9020 | 3.08 | 0 | 1.0 |
| 2.4e-4 | 0.06 | 9053 | 2.98 | 0 | 1.0 |
| 9.8e-4 | 0.05 | 9398 | 2.90 | 0 | 0.9 |
| 3.9e-3 | 0.05 | 9398 | 2.90 | 0 | 1.1 |
| 1.6e-2 | 0.05 | 9147 | 2.87 | 0 | 1.1 |
| 6.2e-2 | 0.05 | 9398 | 2.90 | 0 | 1.2 |
| 2.5e-1 | 0.05 | 9398 | 2.90 | 0 | 0.5 |
| 1.0e0 | 0.03 | 16774 | 3.26 | 0 | 0.9 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_samples_appended@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 2.96 | 98 | 3.32 | 0 | 3.4 |
| 2.4e-7 | 1.22 | 480 | 2.11 | 0 | 3.5 |
| 9.5e-7 | 0.32 | 954 | 1.66 | 0 | 3.2 |
| 3.8e-6 | 0.10 | 1730 | 2.07 | 0 | 3.2 |
| 1.5e-5 | 0.06 | 1362 | 1.91 | 0 | 3.3 |
| 6.1e-5 | 0.04 | 1000 | 1.87 | 0 | 3.4 |
| 2.4e-4 | 0.04 | 1000 | 1.87 | 0 | 3.5 |
| 9.8e-4 | 0.04 | 1000 | 1.87 | 0 | 2.8 |
| 3.9e-3 | 0.05 | 18825 | 2.75 | 9 | 2.8 |
| 1.6e-2 | 0.05 | 19841 | 2.68 | 9 | 2.3 |
| 6.2e-2 | 0.04 | 1000 | 1.87 | 0 | 2.4 |
| 2.5e-1 | 0.04 | 1000 | 1.87 | 0 | 2.0 |
| 1.0e0 | 0.04 | 1000 | 1.87 | 0 | 1.2 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_samples_appended@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 2.96 | 630 | 3.87 | 0 | 4.3 |
| 2.4e-7 | 1.22 | 2358 | 3.30 | 0 | 3.4 |
| 9.5e-7 | 0.32 | 5054 | 2.97 | 0 | 3.3 |
| 3.8e-6 | 0.10 | 8724 | 3.37 | 0 | 3.6 |
| 1.5e-5 | 0.06 | 10106 | 3.48 | 0 | 3.3 |
| 6.1e-5 | 0.04 | 9790 | 3.50 | 0 | 3.2 |
| 2.4e-4 | 0.04 | 9790 | 3.50 | 0 | 2.9 |
| 9.8e-4 | 0.04 | 9790 | 3.50 | 0 | 3.0 |
| 3.9e-3 | 0.05 | 16481 | 4.65 | 0 | 2.6 |
| 1.6e-2 | 0.05 | 17380 | 4.58 | 0 | 2.6 |
| 6.2e-2 | 0.04 | 9790 | 3.50 | 0 | 2.3 |
| 2.5e-1 | 0.04 | 9790 | 3.50 | 0 | 2.4 |
| 1.0e0 | 0.04 | 9790 | 3.50 | 0 | 1.9 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>caddy_requests@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.31 | 0 | 8.31 | 0 | 0.0 |
| 2.4e-7 | 6.05 | 11 | 6.12 | 0 | 10.1 |
| 9.5e-7 | 3.61 | 48 | 3.77 | 0 | 9.3 |
| 3.8e-6 | 1.68 | 214 | 2.03 | 0 | 8.6 |
| 1.5e-5 | 0.69 | 4539 | 1.96 | 0 | 9.5 |
| 6.1e-5 | 0.24 | 2659 | 1.88 | 0 | 9.0 |
| 2.4e-4 | 0.12 | 2690 | 1.60 | 0 | 8.4 |
| 9.8e-4 | 0.07 | 3077 | 1.43 | 0 | 9.6 |
| 3.9e-3 | 0.05 | 3254 | 1.38 | 0 | 9.9 |
| 1.6e-2 | 0.06 | 20789 | 1.54 | 0 | 9.4 |
| 6.2e-2 | 0.04 | 2021 | 1.40 | 0 | 8.5 |
| 2.5e-1 | 0.04 | 2021 | 1.40 | 0 | 8.8 |
| 1.0e0 | 0.04 | 2021 | 1.40 | 0 | 9.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>caddy_requests@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.31 | 0 | 8.31 | 0 | 0.0 |
| 2.4e-7 | 6.05 | 50 | 6.15 | 0 | 11.1 |
| 9.5e-7 | 3.61 | 187 | 3.81 | 0 | 9.6 |
| 3.8e-6 | 1.68 | 886 | 2.16 | 0 | 9.7 |
| 1.5e-5 | 0.69 | 9066 | 2.08 | 0 | 9.3 |
| 6.1e-5 | 0.24 | 6116 | 2.03 | 0 | 9.6 |
| 2.4e-4 | 0.12 | 6112 | 1.74 | 0 | 9.4 |
| 9.8e-4 | 0.07 | 6285 | 1.57 | 0 | 9.6 |
| 3.9e-3 | 0.05 | 6896 | 1.54 | 0 | 9.5 |
| 1.6e-2 | 0.06 | 22065 | 1.71 | 0 | 9.0 |
| 6.2e-2 | 0.04 | 7467 | 1.56 | 0 | 9.2 |
| 2.5e-1 | 0.04 | 7467 | 1.56 | 0 | 8.4 |
| 1.0e0 | 0.04 | 7467 | 1.56 | 0 | 7.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_200@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.61 | 9 | 5.69 | 0 | 9.7 |
| 2.4e-7 | 3.71 | 45 | 3.90 | 0 | 9.4 |
| 9.5e-7 | 3.30 | 168 | 3.79 | 0 | 8.6 |
| 3.8e-6 | 1.58 | 701 | 2.62 | 0 | 9.4 |
| 1.5e-5 | 0.63 | 4292 | 2.61 | 0 | 9.2 |
| 6.1e-5 | 0.36 | 2644 | 2.46 | 0 | 8.8 |
| 2.4e-4 | 0.14 | 6578 | 2.66 | 0 | 9.6 |
| 9.8e-4 | 0.06 | 4340 | 2.46 | 0 | 9.9 |
| 3.9e-3 | 0.04 | 2011 | 2.28 | 0 | 9.6 |
| 1.6e-2 | 0.04 | 2011 | 2.28 | 0 | 8.4 |
| 6.2e-2 | 0.04 | 2011 | 2.28 | 0 | 8.8 |
| 2.5e-1 | 0.03 | 21161 | 2.02 | 0 | 8.9 |
| 1.0e0 | 0.04 | 2011 | 2.28 | 0 | 7.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_200@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.61 | 7 | 5.64 | 0 | 10.4 |
| 2.4e-7 | 3.71 | 34 | 3.77 | 0 | 9.8 |
| 9.5e-7 | 3.30 | 111 | 3.44 | 0 | 9.6 |
| 3.8e-6 | 1.58 | 311 | 1.86 | 0 | 9.5 |
| 1.5e-5 | 0.63 | 3841 | 1.67 | 0 | 9.8 |
| 6.1e-5 | 0.36 | 1623 | 1.60 | 0 | 9.7 |
| 2.4e-4 | 0.14 | 10041 | 1.84 | 0 | 9.7 |
| 9.8e-4 | 0.06 | 19733 | 1.60 | 0 | 9.6 |
| 3.9e-3 | 0.04 | 21874 | 1.39 | 0 | 8.9 |
| 1.6e-2 | 0.04 | 21874 | 1.39 | 0 | 8.9 |
| 6.2e-2 | 0.04 | 21874 | 1.39 | 0 | 7.8 |
| 2.5e-1 | 0.03 | 23616 | 1.15 | 0 | 7.9 |
| 1.0e0 | 0.04 | 21874 | 1.39 | 0 | 7.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_500@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 6.34 | 11875 | 7.91 | 0 | 10.8 |
| 2.4e-7 | 6.14 | 14101 | 7.78 | 0 | 9.6 |
| 9.5e-7 | 6.32 | 10979 | 7.90 | 0 | 9.6 |
| 3.8e-6 | 4.28 | 11429 | 6.11 | 0 | 9.4 |
| 1.5e-5 | 2.21 | 18091 | 5.72 | 0 | 9.8 |
| 6.1e-5 | 0.87 | 17941 | 6.13 | 0 | 9.6 |
| 2.4e-4 | 0.22 | 6638 | 5.78 | 0 | 9.2 |
| 9.8e-4 | 0.05 | 2278 | 4.28 | 0 | 8.9 |
| 3.9e-3 | 0.04 | 2094 | 4.27 | 0 | 10.8 |
| 1.6e-2 | 0.04 | 2094 | 4.27 | 0 | 11.5 |
| 6.2e-2 | 0.04 | 2094 | 4.27 | 0 | 11.9 |
| 2.5e-1 | 0.04 | 2094 | 4.27 | 0 | 9.1 |
| 1.0e0 | 0.04 | 2094 | 4.27 | 0 | 6.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_500@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 6.34 | 17533 | 7.94 | 0 | 10.4 |
| 2.4e-7 | 6.14 | 19895 | 7.80 | 0 | 11.1 |
| 9.5e-7 | 6.32 | 16088 | 7.92 | 0 | 10.7 |
| 3.8e-6 | 4.28 | 16700 | 6.16 | 0 | 10.7 |
| 1.5e-5 | 2.21 | 22704 | 5.76 | 0 | 10.2 |
| 6.1e-5 | 0.87 | 22058 | 6.15 | 0 | 10.4 |
| 2.4e-4 | 0.22 | 16377 | 5.80 | 0 | 10.9 |
| 9.8e-4 | 0.05 | 25665 | 4.34 | 0 | 11.6 |
| 3.9e-3 | 0.04 | 25665 | 4.33 | 0 | 11.9 |
| 1.6e-2 | 0.04 | 25665 | 4.33 | 0 | 12.1 |
| 6.2e-2 | 0.04 | 25665 | 4.33 | 0 | 10.1 |
| 2.5e-1 | 0.04 | 25665 | 4.33 | 0 | 6.4 |
| 1.0e0 | 0.04 | 25665 | 4.33 | 0 | 10.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_cpu_seconds@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.17 | 8 | 8.22 | 0 | 10.5 |
| 2.4e-7 | 3.96 | 29 | 4.06 | 0 | 9.8 |
| 9.5e-7 | 2.17 | 87 | 2.39 | 0 | 9.7 |
| 3.8e-6 | 1.39 | 308 | 1.84 | 1 | 9.5 |
| 1.5e-5 | 0.46 | 1123 | 1.30 | 0 | 9.6 |
| 6.1e-5 | 0.20 | 2219 | 1.53 | 1 | 9.5 |
| 2.4e-4 | 0.10 | 3353 | 1.57 | 0 | 8.8 |
| 9.8e-4 | 0.07 | 3978 | 1.55 | 0 | 8.7 |
| 3.9e-3 | 0.05 | 2011 | 1.38 | 1 | 9.3 |
| 1.6e-2 | 0.04 | 2011 | 1.39 | 1 | 7.5 |
| 6.2e-2 | 0.04 | 2011 | 1.39 | 1 | 8.6 |
| 2.5e-1 | 0.04 | 2011 | 1.39 | 1 | 8.8 |
| 1.0e0 | 0.04 | 2011 | 1.39 | 1 | 7.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_cpu_seconds@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.17 | 24 | 8.22 | 0 | 11.3 |
| 2.4e-7 | 3.96 | 24 | 4.06 | 0 | 10.7 |
| 9.5e-7 | 2.17 | 145 | 2.40 | 0 | 10.9 |
| 3.8e-6 | 1.39 | 940 | 1.88 | 0 | 10.8 |
| 1.5e-5 | 0.46 | 3134 | 1.40 | 0 | 11.1 |
| 6.1e-5 | 0.20 | 7045 | 1.58 | 0 | 10.8 |
| 2.4e-4 | 0.10 | 14619 | 1.55 | 0 | 10.6 |
| 9.8e-4 | 0.07 | 15492 | 1.48 | 0 | 10.6 |
| 3.9e-3 | 0.05 | 18423 | 1.27 | 3 | 10.4 |
| 1.6e-2 | 0.04 | 18911 | 1.24 | 3 | 9.9 |
| 6.2e-2 | 0.04 | 18911 | 1.24 | 3 | 8.5 |
| 2.5e-1 | 0.04 | 18911 | 1.24 | 3 | 7.8 |
| 1.0e0 | 0.04 | 18911 | 1.24 | 3 | 6.5 |

</details>

## 30 days

### H8: real Prometheus series (13 series, 30.0 days, raw 15s scrapes)

Sources: `prom-demo` = prometheus.demo.prometheus.io (node_exporter, Prometheus, Caddy),
`promlabs` = demo.promlabs.com (demo service). Rules: `prod` = upstream node-/prometheus-mixin
thresholds restricted to one series; `q95` / `q50` = θ recalibrated to the 95th percentile / median
of the rule's aggregate (see `real.rs`). Same columns as H1, plus H7 (κ=1024) and
Prometheus-sized chunks of 120. Zero changed decisions in every rule-aware cell (asserted).

| series | kind | rules | firings | lossless | uniform-safe | uniform-tuned | rule-aware | κ=1024 | rule-aware /120 | κ /120 | ×lossless | ×tuned | ×tuned (κ) | exact samples |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| node_load1 | gauge (prom-demo) | prod | 0 | 54.54 | 0.00 | 0.00 | 0.00 | 0.00 | 0.21 | 0.21 | 12804.6 | 1.0 | 1.0 | 0.00% |
| node_load1 | gauge (prom-demo) | q95 | 0 | 54.54 | 4.83 | 3.82 | 0.20 | 0.20 | 0.46 | 0.46 | 268.0 | 18.8 | 18.8 | 0.00% |
| node_load1 | gauge (prom-demo) | q50 | 31 | 54.54 | 4.83 | 3.82 | 0.83 | 0.83 | 1.15 | 1.15 | 65.4 | 4.6 | 4.6 | 0.00% |
| node_memory_available | gauge (prom-demo) | prod | 0 | 15.32 | 0.01 | 0.00 | 0.00 | 0.00 | 0.33 | 0.33 | 4354.2 | 1.0 | 1.0 | 0.00% |
| node_memory_available | gauge (prom-demo) | q95 | 13 | 15.32 | 9.61 | 8.61 | 0.05 | 0.05 | 0.37 | 0.37 | 330.9 | 186.0 | 186.0 | 0.00% |
| node_memory_available | gauge (prom-demo) | q50 | 141 | 15.32 | 10.63 | 10.63 | 0.11 | 0.11 | 0.41 | 0.41 | 143.9 | 99.8 | 99.8 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | prod | 0 | 13.11 | 0.00 | 0.00 | 0.00 | 0.00 | 0.33 | 0.33 | 3725.5 | 1.0 | 1.0 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | q95 | 38 | 13.11 | 2.65 | 1.13 | 0.01 | 0.01 | 0.33 | 0.33 | 1444.6 | 124.9 | 124.9 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | q50 | 154 | 13.11 | 3.23 | 3.23 | 0.02 | 0.02 | 0.31 | 0.31 | 755.0 | 186.0 | 186.0 | 0.00% |
| node_timex_offset | gauge (prom-demo) | prod | 0 | 55.29 | 0.04 | 0.04 | 0.04 | 0.04 | 0.25 | 0.25 | 1489.1 | 1.0 | 1.0 | 0.00% |
| node_timex_offset | gauge (prom-demo) | q95 | 143 | 55.29 | 5.17 | 2.20 | 0.08 | 0.08 | 0.35 | 0.35 | 660.2 | 26.3 | 26.3 | 0.00% |
| node_timex_offset | gauge (prom-demo) | q50 | 750 | 55.29 | 5.17 | 4.30 | 0.06 | 0.06 | 0.36 | 0.36 | 875.5 | 68.1 | 68.1 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | prod | 0 | 0.02 | 0.00 | 0.00 | 0.00 | 0.00 | 0.14 | 0.14 | 4.6 | 1.0 | 1.0 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | q95 | 0 | 0.02 | 0.01 | 0.01 | 0.01 | 0.01 | 0.14 | 0.14 | 2.2 | 1.1 | 1.1 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | q50 | 0 | 0.02 | 0.01 | 0.01 | 0.01 | 0.01 | 0.14 | 0.14 | 2.2 | 1.1 | 1.1 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | prod | 0 | 7.75 | 0.00 | 0.00 | 0.00 | 0.00 | 0.32 | 0.27 | 2091.4 | 1.0 | 1.1 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | q95 | 5 | 7.75 | 7.74 | 1.60 | 0.78 | 0.78 | 1.44 | 1.40 | 9.9 | 2.1 | 2.1 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | q50 | 98 | 7.75 | 7.75 | 1.60 | 1.58 | 1.57 | 2.34 | 2.32 | 4.9 | 1.0 | 1.0 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | prod | 0 | 40.25 | 0.01 | 0.01 | 0.01 | 0.00 | 0.39 | 0.22 | 6844.9 | 1.4 | 2.3 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | q95 | 0 | 40.25 | 9.04 | 8.04 | 0.74 | 0.65 | 1.33 | 1.12 | 54.3 | 10.8 | 12.4 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | q50 | 0 | 40.25 | 9.04 | 8.04 | 1.33 | 1.28 | 2.03 | 1.95 | 30.4 | 6.1 | 6.3 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | prod | 0 | 41.47 | 0.53 | 0.71 | 0.32 | 0.32 | 1.13 | 1.13 | 131.0 | 2.2 | 2.2 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | q95 | 4 | 41.47 | 7.65 | 6.66 | 1.73 | 1.08 | 2.73 | 1.85 | 24.0 | 3.9 | 6.1 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | q50 | 104 | 41.47 | 7.65 | 6.66 | 2.66 | 2.36 | 3.69 | 3.31 | 15.6 | 2.5 | 2.8 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | prod | 0 | 22.24 | 0.15 | 0.15 | 0.14 | 0.14 | 0.80 | 0.80 | 156.4 | 1.1 | 1.1 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | q95 | 41 | 22.24 | 11.07 | 10.11 | 1.02 | 0.92 | 1.82 | 1.73 | 21.9 | 10.0 | 11.0 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | q50 | 29 | 22.24 | 11.07 | 10.11 | 1.15 | 1.08 | 2.14 | 1.88 | 19.4 | 8.8 | 9.4 | 0.00% |
| caddy_requests | counter (prom-demo) | q95 | 171 | 13.63 | 12.88 | 5.02 | 1.26 | 1.14 | 2.09 | 1.90 | 10.9 | 4.0 | 4.4 | 0.00% |
| caddy_requests | counter (prom-demo) | q50 | 862 | 13.63 | 7.00 | 5.02 | 1.52 | 1.42 | 2.35 | 2.18 | 9.0 | 3.3 | 3.5 | 0.00% |
| demo_api_foo_200 | counter (promlabs) | q95 | 0 | 17.11 | 11.61 | 6.24 | 2.96 | 2.79 | 4.01 | 3.76 | 5.8 | 2.1 | 2.2 | 0.00% |
| demo_api_foo_200 | counter (promlabs) | q50 | 1440 | 17.11 | 7.97 | 6.24 | 1.57 | 1.41 | 2.53 | 2.20 | 10.9 | 4.0 | 4.4 | 0.00% |
| demo_api_foo_500 | counter (promlabs) | q95 | 3 | 9.74 | 9.74 | 4.14 | 2.68 | 2.56 | 3.64 | 3.51 | 3.6 | 1.5 | 1.6 | 0.00% |
| demo_api_foo_500 | counter (promlabs) | q50 | 1580 | 9.74 | 9.74 | 4.14 | 2.86 | 2.75 | 3.82 | 3.69 | 3.4 | 1.4 | 1.5 | 0.00% |
| demo_cpu_seconds | counter (promlabs) | q95 | 212 | 42.25 | 12.10 | 16.83 | 1.14 | 1.05 | 1.95 | 1.78 | 37.1 | 14.8 | 16.0 | 0.00% |
| demo_cpu_seconds | counter (promlabs) | q50 | 1372 | 42.25 | 13.04 | 13.05 | 1.42 | 1.23 | 2.32 | 2.02 | 29.8 | 9.2 | 10.6 | 0.00% |

<details><summary>Thresholds</summary>

| series | rules | rule | expression | for | firings |
|---|---|---|---|---:|---:|
| node_load1 | prod | NodeSystemSaturation | `Last(x) > 4e0` | 15m | 0 |
| node_load1 | q95 | NodeSystemSaturation | `Last(x) > 4.95e-1` | 15m | 0 |
| node_load1 | q50 | NodeSystemSaturation | `Last(x) > 1.35e-1` | 15m | 31 |
| node_memory_available | prod | NodeMemoryHighUtilization | `Last(x) < 4.1153413120000005e8` | 15m | 0 |
| node_memory_available | q95 | NodeMemoryHighUtilization | `Last(x) < 2.502017024e9` | 15m | 13 |
| node_memory_available | q50 | NodeMemoryHighUtilization | `Last(x) < 2.65792512e9` | 15m | 141 |
| node_fs_root_avail | prod | NodeFilesystemAlmostOutOfSpace | `Last(x) < 2.6295513088e9` | 30m | 0 |
| node_fs_root_avail | prod | NodeFilesystemAlmostOutOfSpaceCrit | `Last(x) < 1.57773078528e9` | 30m | 0 |
| node_fs_root_avail | q95 | NodeFilesystemAlmostOutOfSpace | `Last(x) < 3.3425186816e10` | 30m | 19 |
| node_fs_root_avail | q95 | NodeFilesystemAlmostOutOfSpaceCrit | `Last(x) < 3.3425186816e10` | 30m | 19 |
| node_fs_root_avail | q50 | NodeFilesystemAlmostOutOfSpace | `Last(x) < 3.3562318848e10` | 30m | 77 |
| node_fs_root_avail | q50 | NodeFilesystemAlmostOutOfSpaceCrit | `Last(x) < 3.3562318848e10` | 30m | 77 |
| node_timex_offset | prod | NodeClockSkewDetected | `Last(x) > 5e-2` | 10m | 0 |
| node_timex_offset | prod | NodeClockSkewDetectedNeg | `Last(x) < -5e-2` | 10m | 0 |
| node_timex_offset | q95 | NodeClockSkewDetected | `Last(x) > 4.3507e-5` | 10m | 72 |
| node_timex_offset | q95 | NodeClockSkewDetectedNeg | `Last(x) < -4.3494499999999995e-5` | 10m | 71 |
| node_timex_offset | q50 | NodeClockSkewDetected | `Last(x) > 5e-10` | 10m | 375 |
| node_timex_offset | q50 | NodeClockSkewDetectedNeg | `Last(x) < 5e-10` | 10m | 375 |
| prometheus_engine_queries | prod | PrometheusHighQueryLoad | `AvgOverTime(x[5m]) > 1.6e1` | 15m | 0 |
| prometheus_engine_queries | q95 | PrometheusHighQueryLoad | `AvgOverTime(x[5m]) > 2.5e-2` | 15m | 0 |
| prometheus_engine_queries | q50 | PrometheusHighQueryLoad | `AvgOverTime(x[5m]) > 2.5e-2` | 15m | 0 |
| node_vmstat_pgmajfault | prod | NodeMemoryMajorPagesFaults | `Rate(x[5m]) > 5e2` | 15m | 0 |
| node_vmstat_pgmajfault | q95 | NodeMemoryMajorPagesFaults | `Rate(x[5m]) > 1.7543982765655314e-1` | 15m | 5 |
| node_vmstat_pgmajfault | q50 | NodeMemoryMajorPagesFaults | `Rate(x[5m]) > 9.824595875895958e-2` | 15m | 98 |
| node_disk_io_weighted_sda | prod | NodeDiskIOSaturation | `Rate(x[5m]) > 1e1` | 30m | 0 |
| node_disk_io_weighted_sda | q95 | NodeDiskIOSaturation | `Rate(x[5m]) > 7.310000000002778e-2` | 30m | 0 |
| node_disk_io_weighted_sda | q50 | NodeDiskIOSaturation | `Rate(x[5m]) > 1.4949122807031432e-2` | 30m | 0 |
| node_cpu0_idle | prod | NodeCPUHighUsage | `Rate(x[2m]) < 1e-1` | 15m | 0 |
| node_cpu0_idle | q95 | NodeCPUHighUsage | `Rate(x[2m]) < 9.020429564805603e-1` | 15m | 4 |
| node_cpu0_idle | q50 | NodeCPUHighUsage | `Rate(x[2m]) < 9.39476190471933e-1` | 15m | 104 |
| prometheus_samples_appended | prod | PrometheusNotIngestingSamples | `Rate(x[5m]) <= 0e0` | 10m | 0 |
| prometheus_samples_appended | q95 | PrometheusNotIngestingSamples | `Rate(x[5m]) <= 1.066121052631579e3` | 10m | 41 |
| prometheus_samples_appended | q50 | PrometheusNotIngestingSamples | `Rate(x[5m]) <= 1.0888543859649124e3` | 10m | 29 |
| caddy_requests | q95 | RateHigh | `Rate(x[5m]) > 3.935087719298245e0` | 5m | 122 |
| caddy_requests | q95 | RateLow | `Rate(x[5m]) < 2.0070386708048886e0` | 10m | 49 |
| caddy_requests | q50 | RateHigh | `Rate(x[5m]) > 2.724484879642337e0` | 5m | 409 |
| caddy_requests | q50 | RateLow | `Rate(x[5m]) < 2.724484879642337e0` | 10m | 453 |
| demo_api_foo_200 | q95 | RateHigh | `Rate(x[5m]) > 4.1782529443162716e1` | 5m | 0 |
| demo_api_foo_200 | q95 | RateLow | `Rate(x[5m]) < 2.42052206840322e1` | 10m | 0 |
| demo_api_foo_200 | q50 | RateHigh | `Rate(x[5m]) > 3.0566666660887247e1` | 5m | 720 |
| demo_api_foo_200 | q50 | RateLow | `Rate(x[5m]) < 3.0566666660887247e1` | 10m | 720 |
| demo_api_foo_500 | q95 | RateHigh | `Rate(x[5m]) > 3.5438534318469916e-1` | 5m | 3 |
| demo_api_foo_500 | q95 | RateLow | `Rate(x[5m]) < 1.649125700225602e-1` | 10m | 0 |
| demo_api_foo_500 | q50 | RateHigh | `Rate(x[5m]) > 2.4561446599104717e-1` | 5m | 1001 |
| demo_api_foo_500 | q50 | RateLow | `Rate(x[5m]) < 2.4561446599104717e-1` | 10m | 579 |
| demo_cpu_seconds | q95 | RateHigh | `Rate(x[5m]) > 1.2091185644980215e-1` | 5m | 121 |
| demo_cpu_seconds | q95 | RateLow | `Rate(x[5m]) < 7.724561397442822e-2` | 10m | 91 |
| demo_cpu_seconds | q50 | RateHigh | `Rate(x[5m]) > 9.533350058542711e-2` | 5m | 725 |
| demo_cpu_seconds | q50 | RateLow | `Rate(x[5m]) < 9.533350058542711e-2` | 10m | 647 |

</details>

Mean bits per value over the series of each rule variant (every series has the same length):

| rules | series | lossless | uniform-safe | uniform-tuned | rule-aware | κ=1024 | rule-aware /120 | κ /120 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| prod | 9 | 27.77 | 0.08 | 0.10 | 0.06 | 0.06 | 0.43 | 0.41 |
| q95 | 13 | 25.59 | 8.01 | 5.72 | 0.97 | 0.87 | 1.59 | 1.44 |
| q50 | 13 | 25.59 | 7.47 | 5.91 | 1.16 | 1.09 | 1.82 | 1.69 |

### H6: Compression Safeguards baseline (real Prometheus series)

`SZ3` is SZ3 with an absolute error bound; `SZ3+SG` wraps it in safeguards translated from the rules
(see `baselines/safeguards/run.py`); both are swept over the SZ3 bound. `tuned` = largest bound with no
changed decision and none below it; `SZ3+SG best` = smallest output with zero state mismatches, if any.
`rule-aware κ` is the optimistic budget with a posteriori repair (H7).
Mismatches are counted by the reference evaluator of this repository.

| dataset | lossless (ours) | uniform-tuned (ours) | rule-aware (ours) | rule-aware κ=1024 (ours) | SZ3 tuned | SZ3+SG best | SZ3+SG ε / range | SZ3+SG points with mismatches | SZ3+SG encode, s (mean) | ×SZ3+SG |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| node_load1@q95 | 54.54 | 3.82 | 0.20 | 0.20 | 5.67 | 1.79 | 6.2e-2 | 0/13 | 0.0 | 8.8 |
| node_load1@q50 | 54.54 | 3.82 | 0.83 | 0.83 | 5.67 | 4.92 | 3.9e-3 | 0/13 | 0.0 | 5.9 |
| node_memory_available@q95 | 15.32 | 8.61 | 0.05 | 0.05 | 10.57 | 0.36 | 1.0e0 | 0/13 | 0.1 | 7.8 |
| node_memory_available@q50 | 15.32 | 10.63 | 0.11 | 0.11 | 14.78 | 0.76 | 1.0e0 | 0/13 | 0.1 | 7.1 |
| node_fs_root_avail@q95 | 13.11 | 1.13 | 0.01 | 0.01 | 3.12 | 0.08 | 1.0e0 | 0/13 | 0.1 | 8.9 |
| node_fs_root_avail@q50 | 13.11 | 3.23 | 0.02 | 0.02 | 3.12 | 0.13 | 1.0e0 | 0/13 | 0.1 | 7.4 |
| node_timex_offset@q95 | 55.29 | 2.20 | 0.08 | 0.08 | 3.55 | 0.38 | 3.9e-3 | 0/13 | 0.1 | 4.6 |
| node_timex_offset@q50 | 55.29 | 4.30 | 0.06 | 0.06 | 10.25 | 0.40 | 3.9e-3 | 0/13 | 0.1 | 6.3 |
| prometheus_engine_queries@q95 | 0.02 | 0.01 | 0.01 | 0.01 | 0.04 | 0.03 | 1.0e0 | 0/13 | 4.6 | 4.1 |
| prometheus_engine_queries@q50 | 0.02 | 0.01 | 0.01 | 0.01 | 0.04 | 0.03 | 1.0e0 | 0/13 | 4.6 | 4.1 |
| node_vmstat_pgmajfault@q95 | 7.75 | 1.60 | 0.78 | 0.78 | – | 2.02 | 3.9e-3 | 0/13 | 18.7 | 2.6 |
| node_vmstat_pgmajfault@q50 | 7.75 | 1.60 | 1.58 | 1.57 | – | 6.98 | 6.0e-8 | 0/13 | 18.7 | 4.4 |
| node_disk_io_weighted_sda@q95 | 40.25 | 8.04 | 0.74 | 0.65 | – | 3.44 | 6.1e-5 | 0/13 | 18.7 | 5.3 |
| node_disk_io_weighted_sda@q50 | 40.25 | 8.04 | 1.33 | 1.28 | – | 3.32 | 3.9e-3 | 0/13 | 19.4 | 2.6 |
| node_cpu0_idle@q95 | 41.47 | 6.66 | 1.73 | 1.08 | – | 1.05 | 9.8e-4 | 0/13 | 4.5 | 1.0 |
| node_cpu0_idle@q50 | 41.47 | 6.66 | 2.66 | 2.36 | – | 2.06 | 9.8e-4 | 0/13 | 4.4 | 0.9 |
| prometheus_samples_appended@q95 | 22.24 | 10.11 | 1.02 | 0.92 | – | 0.56 | 2.4e-7 | 0/13 | 16.9 | 0.6 |
| prometheus_samples_appended@q50 | 22.24 | 10.11 | 1.15 | 1.08 | – | 0.60 | 2.4e-7 | 0/13 | 17.6 | 0.6 |
| caddy_requests@q95 | 13.63 | 5.02 | 1.26 | 1.14 | – | 1.41 | 6.2e-2 | 2/13 | 59.9 | 1.2 |
| caddy_requests@q50 | 13.63 | 5.02 | 1.52 | 1.42 | – | 1.65 | 2.4e-4 | 0/13 | 58.2 | 1.2 |
| demo_api_foo_200@q95 | 17.11 | 6.24 | 2.96 | 2.79 | – | 2.08 | 3.9e-3 | 0/13 | 60.3 | 0.7 |
| demo_api_foo_200@q50 | 17.11 | 6.24 | 1.57 | 1.41 | – | 1.28 | 3.9e-3 | 0/13 | 60.0 | 0.9 |
| demo_api_foo_500@q95 | 9.74 | 4.14 | 2.68 | 2.56 | – | 4.11 | 9.8e-4 | 1/13 | 57.0 | 1.6 |
| demo_api_foo_500@q50 | 9.74 | 4.14 | 2.86 | 2.75 | – | 4.17 | 9.8e-4 | 0/13 | 59.7 | 1.5 |
| demo_cpu_seconds@q95 | 42.25 | 16.83 | 1.14 | 1.05 | – | 1.18 | 3.8e-6 | 0/13 | 58.6 | 1.1 |
| demo_cpu_seconds@q50 | 42.25 | 13.05 | 1.42 | 1.23 | – | 1.17 | 3.9e-3 | 0/13 | 59.8 | 1.0 |

Values are bits per value (timestamps excluded). `×SZ3+SG` = SZ3+SG best / rule-aware κ=1024.

<details><summary>SZ3 / SZ3+SG sweep, <code>node_load1@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 9.92 | 0 | 9.92 | 0 | 0.0 |
| 2.4e-7 | 7.70 | 0 | 7.70 | 0 | 0.0 |
| 9.5e-7 | 6.72 | 0 | 6.72 | 0 | 0.0 |
| 3.8e-6 | 6.26 | 0 | 6.26 | 0 | 0.0 |
| 1.5e-5 | 5.85 | 0 | 5.85 | 0 | 0.0 |
| 6.1e-5 | 5.83 | 0 | 5.84 | 0 | 0.0 |
| 2.4e-4 | 5.84 | 0 | 5.84 | 0 | 0.0 |
| 9.8e-4 | 5.67 | 0 | 5.67 | 0 | 0.0 |
| 3.9e-3 | 4.22 | 88 | 4.28 | 0 | 0.1 |
| 1.6e-2 | 2.36 | 415 | 2.62 | 0 | 0.1 |
| 6.2e-2 | 0.84 | 1677 | 1.79 | 0 | 0.1 |
| 2.5e-1 | 0.17 | 5355 | 1.91 | 0 | 0.1 |
| 1.0e0 | 0.02 | 8643 | 3.21 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_load1@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 9.92 | 0 | 9.92 | 0 | 0.0 |
| 2.4e-7 | 7.70 | 0 | 7.70 | 0 | 0.0 |
| 9.5e-7 | 6.72 | 0 | 6.72 | 0 | 0.0 |
| 3.8e-6 | 6.26 | 0 | 6.26 | 0 | 0.0 |
| 1.5e-5 | 5.85 | 0 | 5.85 | 0 | 0.0 |
| 6.1e-5 | 5.83 | 0 | 5.84 | 0 | 0.0 |
| 2.4e-4 | 5.84 | 0 | 5.84 | 0 | 0.0 |
| 9.8e-4 | 5.67 | 0 | 5.67 | 0 | 0.0 |
| 3.9e-3 | 4.22 | 1188 | 4.92 | 0 | 0.1 |
| 1.6e-2 | 2.36 | 5275 | 4.94 | 0 | 0.1 |
| 6.2e-2 | 0.84 | 19589 | 8.17 | 0 | 0.1 |
| 2.5e-1 | 0.17 | 57916 | 8.59 | 0 | 0.1 |
| 1.0e0 | 0.02 | 63807 | 17.93 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_memory_available@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 17.99 | 0 | 17.99 | 0 | 0.0 |
| 2.4e-7 | 18.59 | 0 | 18.59 | 0 | 0.1 |
| 9.5e-7 | 14.78 | 0 | 14.78 | 0 | 0.1 |
| 3.8e-6 | 10.57 | 0 | 10.57 | 0 | 0.1 |
| 1.5e-5 | 7.67 | 2 | 7.67 | 0 | 0.1 |
| 6.1e-5 | 5.43 | 0 | 5.44 | 0 | 0.1 |
| 2.4e-4 | 3.43 | 81 | 3.45 | 0 | 0.1 |
| 9.8e-4 | 1.72 | 514 | 1.76 | 0 | 0.1 |
| 3.9e-3 | 0.96 | 1077 | 1.28 | 0 | 0.1 |
| 1.6e-2 | 0.21 | 2669 | 0.70 | 0 | 0.1 |
| 6.2e-2 | 0.04 | 5676 | 0.38 | 0 | 0.1 |
| 2.5e-1 | 0.03 | 5900 | 0.37 | 0 | 0.1 |
| 1.0e0 | 0.01 | 5845 | 0.36 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_memory_available@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 17.99 | 0 | 17.99 | 0 | 0.1 |
| 2.4e-7 | 18.59 | 0 | 18.59 | 0 | 0.1 |
| 9.5e-7 | 14.78 | 0 | 14.78 | 0 | 0.0 |
| 3.8e-6 | 10.57 | 3 | 10.57 | 0 | 0.1 |
| 1.5e-5 | 7.67 | 7 | 7.67 | 0 | 0.1 |
| 6.1e-5 | 5.43 | 66 | 5.45 | 0 | 0.1 |
| 2.4e-4 | 3.43 | 301 | 3.47 | 0 | 0.1 |
| 9.8e-4 | 1.72 | 1935 | 1.81 | 0 | 0.1 |
| 3.9e-3 | 0.96 | 3704 | 2.28 | 0 | 0.1 |
| 1.6e-2 | 0.21 | 14122 | 1.21 | 0 | 0.1 |
| 6.2e-2 | 0.04 | 19316 | 0.78 | 0 | 0.1 |
| 2.5e-1 | 0.03 | 19521 | 0.80 | 0 | 0.1 |
| 1.0e0 | 0.01 | 19219 | 0.76 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_fs_root_avail@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.14 | 0 | 5.14 | 0 | 0.0 |
| 2.4e-7 | 4.95 | 0 | 4.95 | 0 | 0.0 |
| 9.5e-7 | 5.31 | 0 | 5.31 | 0 | 0.0 |
| 3.8e-6 | 3.12 | 0 | 3.12 | 0 | 0.0 |
| 1.5e-5 | 1.70 | 4 | 1.71 | 0 | 0.2 |
| 6.1e-5 | 0.72 | 8 | 0.72 | 0 | 0.1 |
| 2.4e-4 | 0.41 | 14 | 0.42 | 0 | 0.1 |
| 9.8e-4 | 0.30 | 150 | 0.31 | 0 | 0.1 |
| 3.9e-3 | 0.29 | 446 | 0.31 | 0 | 0.1 |
| 1.6e-2 | 0.15 | 2156 | 0.18 | 0 | 0.1 |
| 6.2e-2 | 0.09 | 6226 | 0.16 | 0 | 0.1 |
| 2.5e-1 | 0.04 | 9468 | 0.09 | 0 | 0.1 |
| 1.0e0 | 0.03 | 11982 | 0.08 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_fs_root_avail@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.14 | 0 | 5.14 | 0 | 0.0 |
| 2.4e-7 | 4.95 | 0 | 4.95 | 0 | 0.0 |
| 9.5e-7 | 5.31 | 0 | 5.31 | 0 | 0.0 |
| 3.8e-6 | 3.12 | 0 | 3.12 | 0 | 0.0 |
| 1.5e-5 | 1.70 | 6 | 1.71 | 0 | 0.1 |
| 6.1e-5 | 0.72 | 30 | 0.72 | 0 | 0.1 |
| 2.4e-4 | 0.41 | 4 | 0.42 | 0 | 0.1 |
| 9.8e-4 | 0.30 | 470 | 0.31 | 0 | 0.1 |
| 3.9e-3 | 0.29 | 2208 | 0.35 | 0 | 0.1 |
| 1.6e-2 | 0.15 | 7880 | 0.23 | 0 | 0.1 |
| 6.2e-2 | 0.09 | 26500 | 0.31 | 0 | 0.1 |
| 2.5e-1 | 0.04 | 52592 | 0.14 | 0 | 0.1 |
| 1.0e0 | 0.03 | 56240 | 0.13 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_timex_offset@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 12.50 | 0 | 12.50 | 0 | 0.0 |
| 2.4e-7 | 10.25 | 0 | 10.25 | 0 | 0.0 |
| 9.5e-7 | 4.76 | 0 | 4.76 | 0 | 0.0 |
| 3.8e-6 | 3.55 | 0 | 3.55 | 0 | 0.0 |
| 1.5e-5 | 2.41 | 7 | 2.41 | 0 | 0.1 |
| 6.1e-5 | 1.66 | 12 | 1.67 | 0 | 0.1 |
| 2.4e-4 | 1.01 | 86 | 1.03 | 0 | 0.1 |
| 9.8e-4 | 0.54 | 194 | 0.59 | 0 | 0.1 |
| 3.9e-3 | 0.30 | 1317 | 0.38 | 0 | 0.1 |
| 1.6e-2 | 0.27 | 3707 | 0.41 | 0 | 0.1 |
| 6.2e-2 | 0.12 | 17910 | 2.20 | 0 | 0.1 |
| 2.5e-1 | 0.03 | 20437 | 0.70 | 0 | 0.2 |
| 1.0e0 | 0.02 | 20601 | 0.67 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_timex_offset@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 12.50 | 0 | 12.50 | 0 | 0.0 |
| 2.4e-7 | 10.25 | 0 | 10.25 | 0 | 0.0 |
| 9.5e-7 | 4.76 | 1612 | 4.81 | 0 | 0.2 |
| 3.8e-6 | 3.55 | 1954 | 3.62 | 0 | 0.1 |
| 1.5e-5 | 2.41 | 2293 | 2.45 | 0 | 0.1 |
| 6.1e-5 | 1.66 | 5909 | 1.73 | 0 | 0.1 |
| 2.4e-4 | 1.01 | 13606 | 1.08 | 0 | 0.1 |
| 9.8e-4 | 0.54 | 33102 | 0.63 | 0 | 0.1 |
| 3.9e-3 | 0.30 | 59673 | 0.40 | 0 | 0.1 |
| 1.6e-2 | 0.27 | 68037 | 6.08 | 0 | 0.1 |
| 6.2e-2 | 0.12 | 92070 | 30.32 | 0 | 0.1 |
| 2.5e-1 | 0.03 | 94082 | 1.27 | 0 | 0.1 |
| 1.0e0 | 0.02 | 94763 | 1.13 | 0 | 0.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_engine_queries@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 0.03 | 0 | 0.03 | 0 | 0.3 |
| 2.4e-7 | 0.03 | 0 | 0.03 | 0 | 0.3 |
| 9.5e-7 | 0.03 | 0 | 0.03 | 0 | 0.3 |
| 3.8e-6 | 0.03 | 0 | 0.03 | 0 | 0.3 |
| 1.5e-5 | 0.03 | 0 | 0.04 | 0 | 0.3 |
| 6.1e-5 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 2.4e-4 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 9.8e-4 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 3.9e-3 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 1.6e-2 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 6.2e-2 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 2.5e-1 | 0.02 | 121 | 0.05 | 0 | 27.0 |
| 1.0e0 | 0.01 | 404 | 0.03 | 0 | 29.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_engine_queries@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 0.03 | 0 | 0.03 | 0 | 0.3 |
| 2.4e-7 | 0.03 | 0 | 0.03 | 0 | 0.2 |
| 9.5e-7 | 0.03 | 0 | 0.03 | 0 | 0.3 |
| 3.8e-6 | 0.03 | 0 | 0.03 | 0 | 0.3 |
| 1.5e-5 | 0.03 | 0 | 0.04 | 0 | 0.3 |
| 6.1e-5 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 2.4e-4 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 9.8e-4 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 3.9e-3 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 1.6e-2 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 6.2e-2 | 0.04 | 0 | 0.04 | 0 | 0.3 |
| 2.5e-1 | 0.02 | 121 | 0.05 | 0 | 27.0 |
| 1.0e0 | 0.01 | 404 | 0.03 | 0 | 29.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_vmstat_pgmajfault@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 3.24 | 49 | 4.61 | 0 | 23.5 |
| 2.4e-7 | 3.26 | 36 | 4.60 | 0 | 24.8 |
| 9.5e-7 | 3.38 | 83580 | 4.77 | 0 | 21.6 |
| 3.8e-6 | 1.51 | 88 | 3.66 | 0 | 18.1 |
| 1.5e-5 | 0.59 | 406 | 3.20 | 0 | 17.9 |
| 6.1e-5 | 0.24 | 6928 | 2.60 | 0 | 17.9 |
| 2.4e-4 | 0.08 | 8325 | 2.37 | 0 | 18.0 |
| 9.8e-4 | 0.03 | 6257 | 2.08 | 0 | 18.0 |
| 3.9e-3 | 0.02 | 4790 | 2.02 | 0 | 17.8 |
| 1.6e-2 | 0.02 | 6021 | 2.03 | 0 | 17.8 |
| 6.2e-2 | 0.01 | 6021 | 2.02 | 0 | 18.1 |
| 2.5e-1 | 0.01 | 6021 | 2.02 | 0 | 17.6 |
| 1.0e0 | 0.01 | 6021 | 2.02 | 0 | 12.3 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_vmstat_pgmajfault@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 3.24 | 6838 | 6.98 | 0 | 23.5 |
| 2.4e-7 | 3.26 | 1623 | 7.36 | 0 | 25.3 |
| 9.5e-7 | 3.38 | 80502 | 7.88 | 0 | 21.6 |
| 3.8e-6 | 1.51 | 19025 | 7.42 | 0 | 18.2 |
| 1.5e-5 | 0.59 | 44405 | 7.68 | 0 | 18.0 |
| 6.1e-5 | 0.24 | 62457 | 8.38 | 0 | 18.0 |
| 2.4e-4 | 0.08 | 70677 | 8.20 | 0 | 17.8 |
| 9.8e-4 | 0.03 | 82555 | 7.51 | 0 | 17.9 |
| 3.9e-3 | 0.02 | 83150 | 7.34 | 0 | 17.7 |
| 1.6e-2 | 0.02 | 84088 | 7.35 | 0 | 17.7 |
| 6.2e-2 | 0.01 | 84088 | 7.32 | 0 | 18.1 |
| 2.5e-1 | 0.01 | 84088 | 7.32 | 0 | 17.6 |
| 1.0e0 | 0.01 | 84088 | 7.32 | 0 | 12.3 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_disk_io_weighted_sda@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 7.14 | 7114 | 7.51 | 0 | 18.8 |
| 2.4e-7 | 4.39 | 16034 | 5.90 | 0 | 22.1 |
| 9.5e-7 | 1.94 | 1564 | 4.96 | 0 | 25.1 |
| 3.8e-6 | 1.05 | 557 | 4.46 | 0 | 18.6 |
| 1.5e-5 | 0.59 | 672 | 3.73 | 0 | 17.4 |
| 6.1e-5 | 0.30 | 2107 | 3.44 | 0 | 17.4 |
| 2.4e-4 | 0.15 | 17138 | 5.27 | 0 | 17.4 |
| 9.8e-4 | 0.04 | 6589 | 4.07 | 0 | 17.5 |
| 3.9e-3 | 0.02 | 4320 | 3.47 | 0 | 17.8 |
| 1.6e-2 | 0.02 | 4320 | 3.47 | 0 | 17.8 |
| 6.2e-2 | 0.02 | 4320 | 3.47 | 0 | 17.7 |
| 2.5e-1 | 0.02 | 4320 | 3.47 | 0 | 18.0 |
| 1.0e0 | 0.02 | 4320 | 3.47 | 0 | 17.2 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_disk_io_weighted_sda@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 7.14 | 4008 | 7.51 | 0 | 22.2 |
| 2.4e-7 | 4.39 | 10784 | 5.85 | 0 | 23.1 |
| 9.5e-7 | 1.94 | 1031 | 4.93 | 0 | 24.7 |
| 3.8e-6 | 1.05 | 432 | 4.46 | 0 | 22.3 |
| 1.5e-5 | 0.59 | 434 | 3.73 | 0 | 17.9 |
| 6.1e-5 | 0.30 | 7198 | 3.39 | 0 | 17.9 |
| 2.4e-4 | 0.15 | 82671 | 5.16 | 0 | 17.9 |
| 9.8e-4 | 0.04 | 85765 | 3.91 | 0 | 17.7 |
| 3.9e-3 | 0.02 | 86369 | 3.32 | 0 | 17.9 |
| 1.6e-2 | 0.02 | 86369 | 3.32 | 0 | 17.6 |
| 6.2e-2 | 0.02 | 86369 | 3.32 | 0 | 17.5 |
| 2.5e-1 | 0.02 | 86369 | 3.32 | 0 | 17.7 |
| 1.0e0 | 0.02 | 86369 | 3.32 | 0 | 17.7 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_cpu0_idle@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 2.07 | 365 | 2.24 | 0 | 4.9 |
| 2.4e-7 | 0.76 | 1029 | 1.16 | 0 | 4.7 |
| 9.5e-7 | 0.29 | 4398 | 1.40 | 0 | 4.7 |
| 3.8e-6 | 0.10 | 5420 | 1.22 | 0 | 4.0 |
| 1.5e-5 | 0.04 | 5274 | 1.09 | 0 | 4.1 |
| 6.1e-5 | 0.03 | 5506 | 1.07 | 0 | 3.2 |
| 2.4e-4 | 0.02 | 4333 | 1.05 | 0 | 4.0 |
| 9.8e-4 | 0.02 | 4333 | 1.05 | 0 | 5.1 |
| 3.9e-3 | 0.02 | 4333 | 1.05 | 0 | 4.8 |
| 1.6e-2 | 0.02 | 4333 | 1.05 | 0 | 5.0 |
| 6.2e-2 | 0.02 | 4333 | 1.05 | 0 | 4.2 |
| 2.5e-1 | 0.02 | 4333 | 1.05 | 0 | 4.7 |
| 1.0e0 | 0.02 | 4333 | 1.05 | 0 | 4.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_cpu0_idle@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 2.07 | 2693 | 2.69 | 0 | 4.5 |
| 2.4e-7 | 0.76 | 7785 | 2.13 | 0 | 4.7 |
| 9.5e-7 | 0.29 | 25518 | 2.53 | 0 | 4.6 |
| 3.8e-6 | 0.10 | 26604 | 2.15 | 0 | 4.5 |
| 1.5e-5 | 0.04 | 24793 | 2.12 | 0 | 4.7 |
| 6.1e-5 | 0.03 | 25626 | 2.13 | 0 | 4.5 |
| 2.4e-4 | 0.02 | 25359 | 2.06 | 0 | 4.4 |
| 9.8e-4 | 0.02 | 26228 | 2.06 | 0 | 4.2 |
| 3.9e-3 | 0.02 | 26228 | 2.06 | 0 | 3.6 |
| 1.6e-2 | 0.02 | 26228 | 2.06 | 0 | 3.3 |
| 6.2e-2 | 0.02 | 26228 | 2.06 | 0 | 5.0 |
| 2.5e-1 | 0.02 | 26228 | 2.06 | 0 | 4.9 |
| 1.0e0 | 0.02 | 26228 | 2.06 | 0 | 4.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_samples_appended@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 1.13 | 980 | 1.36 | 0 | 25.0 |
| 2.4e-7 | 0.26 | 1970 | 0.56 | 0 | 18.4 |
| 9.5e-7 | 0.07 | 5463 | 0.68 | 0 | 17.3 |
| 3.8e-6 | 0.03 | 5162 | 0.63 | 0 | 17.5 |
| 1.5e-5 | 0.02 | 4182 | 0.67 | 0 | 17.6 |
| 6.1e-5 | 0.02 | 4376 | 0.68 | 0 | 17.5 |
| 2.4e-4 | 0.02 | 4310 | 0.68 | 0 | 17.7 |
| 9.8e-4 | 0.02 | 4310 | 0.68 | 0 | 17.6 |
| 3.9e-3 | 0.02 | 4310 | 0.68 | 0 | 17.8 |
| 1.6e-2 | 0.02 | 5261 | 0.68 | 0 | 18.0 |
| 6.2e-2 | 0.02 | 4310 | 0.68 | 0 | 17.4 |
| 2.5e-1 | 0.02 | 4310 | 0.68 | 0 | 11.5 |
| 1.0e0 | 0.02 | 4310 | 0.68 | 0 | 6.5 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_samples_appended@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 1.13 | 1038 | 1.41 | 0 | 22.0 |
| 2.4e-7 | 0.26 | 2241 | 0.60 | 0 | 23.6 |
| 9.5e-7 | 0.07 | 6983 | 0.74 | 0 | 16.9 |
| 3.8e-6 | 0.03 | 5241 | 0.72 | 0 | 17.1 |
| 1.5e-5 | 0.02 | 5934 | 0.74 | 0 | 17.3 |
| 6.1e-5 | 0.02 | 6102 | 0.75 | 0 | 17.3 |
| 2.4e-4 | 0.02 | 6063 | 0.75 | 0 | 17.3 |
| 9.8e-4 | 0.02 | 6063 | 0.75 | 0 | 17.4 |
| 3.9e-3 | 0.02 | 6063 | 0.75 | 0 | 17.4 |
| 1.6e-2 | 0.02 | 5907 | 0.75 | 0 | 17.4 |
| 6.2e-2 | 0.02 | 6063 | 0.75 | 0 | 18.0 |
| 2.5e-1 | 0.02 | 6063 | 0.75 | 0 | 16.5 |
| 1.0e0 | 0.02 | 6063 | 0.75 | 0 | 11.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>caddy_requests@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.89 | 88 | 5.94 | 0 | 74.4 |
| 2.4e-7 | 3.79 | 269 | 3.92 | 0 | 71.5 |
| 9.5e-7 | 1.99 | 853 | 2.31 | 0 | 59.9 |
| 3.8e-6 | 0.65 | 14396 | 1.83 | 0 | 64.6 |
| 1.5e-5 | 0.22 | 10551 | 1.83 | 1 | 58.0 |
| 6.1e-5 | 0.07 | 11436 | 1.54 | 1 | 58.9 |
| 2.4e-4 | 0.03 | 12253 | 1.41 | 0 | 57.6 |
| 9.8e-4 | 0.02 | 11642 | 1.44 | 0 | 54.4 |
| 3.9e-3 | 0.02 | 8552 | 1.41 | 0 | 51.2 |
| 1.6e-2 | 0.02 | 8552 | 1.41 | 0 | 57.9 |
| 6.2e-2 | 0.02 | 8552 | 1.41 | 0 | 58.6 |
| 2.5e-1 | 0.02 | 8552 | 1.41 | 0 | 59.2 |
| 1.0e0 | 0.02 | 8552 | 1.41 | 0 | 52.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>caddy_requests@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.89 | 283 | 5.98 | 0 | 73.0 |
| 2.4e-7 | 3.79 | 1101 | 4.02 | 0 | 66.3 |
| 9.5e-7 | 1.99 | 6127 | 2.55 | 0 | 60.9 |
| 3.8e-6 | 0.65 | 30846 | 2.11 | 0 | 59.4 |
| 1.5e-5 | 0.22 | 30812 | 2.07 | 0 | 50.4 |
| 6.1e-5 | 0.07 | 38752 | 1.78 | 0 | 56.9 |
| 2.4e-4 | 0.03 | 43949 | 1.65 | 0 | 55.7 |
| 9.8e-4 | 0.02 | 53827 | 1.69 | 0 | 58.0 |
| 3.9e-3 | 0.02 | 61758 | 1.67 | 0 | 56.3 |
| 1.6e-2 | 0.02 | 61758 | 1.67 | 0 | 59.6 |
| 6.2e-2 | 0.02 | 61758 | 1.67 | 0 | 53.5 |
| 2.5e-1 | 0.02 | 61758 | 1.67 | 0 | 53.3 |
| 1.0e0 | 0.02 | 61758 | 1.67 | 0 | 53.7 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_200@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 3.55 | 208 | 3.72 | 0 | 77.7 |
| 2.4e-7 | 2.31 | 751 | 2.83 | 0 | 64.9 |
| 9.5e-7 | 1.37 | 6452 | 2.48 | 0 | 61.7 |
| 3.8e-6 | 0.51 | 19070 | 2.51 | 0 | 63.7 |
| 1.5e-5 | 0.17 | 14034 | 2.19 | 0 | 63.7 |
| 6.1e-5 | 0.07 | 18598 | 2.20 | 0 | 57.0 |
| 2.4e-4 | 0.03 | 14791 | 2.23 | 0 | 58.7 |
| 9.8e-4 | 0.02 | 8637 | 2.08 | 0 | 64.4 |
| 3.9e-3 | 0.02 | 8637 | 2.08 | 0 | 55.6 |
| 1.6e-2 | 0.02 | 8637 | 2.08 | 0 | 57.0 |
| 6.2e-2 | 0.02 | 8637 | 2.08 | 0 | 55.1 |
| 2.5e-1 | 0.02 | 8637 | 2.08 | 0 | 54.4 |
| 1.0e0 | 0.02 | 8637 | 2.08 | 0 | 49.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_200@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 3.55 | 95 | 3.59 | 0 | 75.4 |
| 2.4e-7 | 2.31 | 372 | 2.43 | 0 | 67.0 |
| 9.5e-7 | 1.37 | 1467 | 1.64 | 0 | 65.3 |
| 3.8e-6 | 0.51 | 18178 | 1.56 | 0 | 64.7 |
| 1.5e-5 | 0.17 | 8639 | 1.42 | 0 | 58.9 |
| 6.1e-5 | 0.07 | 24983 | 1.41 | 0 | 57.9 |
| 2.4e-4 | 0.03 | 89069 | 1.43 | 0 | 59.2 |
| 9.8e-4 | 0.02 | 93788 | 1.28 | 0 | 59.2 |
| 3.9e-3 | 0.02 | 93788 | 1.28 | 0 | 53.8 |
| 1.6e-2 | 0.02 | 93788 | 1.28 | 0 | 54.1 |
| 6.2e-2 | 0.02 | 93788 | 1.28 | 0 | 55.6 |
| 2.5e-1 | 0.02 | 93788 | 1.28 | 0 | 57.5 |
| 1.0e0 | 0.02 | 93788 | 1.28 | 0 | 51.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_500@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.11 | 460 | 6.61 | 0 | 72.8 |
| 2.4e-7 | 5.52 | 639 | 7.02 | 0 | 62.9 |
| 9.5e-7 | 3.89 | 47377 | 5.85 | 0 | 49.4 |
| 3.8e-6 | 1.95 | 1872 | 5.39 | 0 | 54.0 |
| 1.5e-5 | 0.77 | 76636 | 6.09 | 0 | 60.5 |
| 6.1e-5 | 0.17 | 26855 | 5.53 | 1 | 57.0 |
| 2.4e-4 | 0.03 | 10400 | 5.00 | 0 | 56.9 |
| 9.8e-4 | 0.01 | 8688 | 4.11 | 0 | 52.7 |
| 3.9e-3 | 0.01 | 8688 | 4.11 | 0 | 51.7 |
| 1.6e-2 | 0.01 | 8688 | 4.11 | 0 | 56.8 |
| 6.2e-2 | 0.01 | 8688 | 4.11 | 0 | 57.2 |
| 2.5e-1 | 0.01 | 8688 | 4.11 | 0 | 56.6 |
| 1.0e0 | 0.01 | 8688 | 4.11 | 0 | 53.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_500@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.11 | 1515 | 6.62 | 0 | 74.8 |
| 2.4e-7 | 5.52 | 4062 | 7.05 | 0 | 65.8 |
| 9.5e-7 | 3.89 | 69004 | 5.89 | 0 | 62.3 |
| 3.8e-6 | 1.95 | 10371 | 5.43 | 0 | 62.7 |
| 1.5e-5 | 0.77 | 94711 | 6.15 | 0 | 58.1 |
| 6.1e-5 | 0.17 | 78683 | 5.54 | 0 | 55.0 |
| 2.4e-4 | 0.03 | 109321 | 5.05 | 0 | 52.9 |
| 9.8e-4 | 0.01 | 109616 | 4.17 | 0 | 56.2 |
| 3.9e-3 | 0.01 | 109616 | 4.17 | 0 | 60.1 |
| 1.6e-2 | 0.01 | 109616 | 4.17 | 0 | 58.2 |
| 6.2e-2 | 0.01 | 109616 | 4.17 | 0 | 59.7 |
| 2.5e-1 | 0.01 | 109616 | 4.17 | 0 | 58.1 |
| 1.0e0 | 0.01 | 109616 | 4.17 | 0 | 52.4 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_cpu_seconds@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 3.70 | 80 | 3.77 | 0 | 75.1 |
| 2.4e-7 | 2.04 | 269 | 2.20 | 0 | 65.8 |
| 9.5e-7 | 0.96 | 1073 | 1.27 | 0 | 61.1 |
| 3.8e-6 | 0.36 | 5139 | 1.18 | 0 | 60.3 |
| 1.5e-5 | 0.14 | 9615 | 1.34 | 0 | 53.1 |
| 6.1e-5 | 0.09 | 11968 | 1.32 | 0 | 50.5 |
| 2.4e-4 | 0.03 | 12274 | 1.30 | 0 | 58.4 |
| 9.8e-4 | 0.03 | 11802 | 1.28 | 0 | 58.8 |
| 3.9e-3 | 0.02 | 8642 | 1.23 | 0 | 61.0 |
| 1.6e-2 | 0.02 | 8642 | 1.23 | 0 | 56.3 |
| 6.2e-2 | 0.02 | 8642 | 1.23 | 0 | 54.5 |
| 2.5e-1 | 0.02 | 8642 | 1.23 | 0 | 54.1 |
| 1.0e0 | 0.02 | 8642 | 1.23 | 0 | 52.8 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_cpu_seconds@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 3.70 | 388 | 3.78 | 0 | 75.3 |
| 2.4e-7 | 2.04 | 943 | 2.26 | 0 | 66.3 |
| 9.5e-7 | 0.96 | 3348 | 1.38 | 0 | 61.7 |
| 3.8e-6 | 0.36 | 13305 | 1.31 | 0 | 61.7 |
| 1.5e-5 | 0.14 | 32747 | 1.44 | 0 | 57.4 |
| 6.1e-5 | 0.09 | 40549 | 1.39 | 0 | 53.7 |
| 2.4e-4 | 0.03 | 75995 | 1.30 | 0 | 55.9 |
| 9.8e-4 | 0.03 | 79690 | 1.26 | 0 | 54.1 |
| 3.9e-3 | 0.02 | 82121 | 1.17 | 0 | 64.4 |
| 1.6e-2 | 0.02 | 82121 | 1.17 | 0 | 57.4 |
| 6.2e-2 | 0.02 | 82121 | 1.17 | 0 | 58.6 |
| 2.5e-1 | 0.02 | 82121 | 1.17 | 0 | 58.5 |
| 1.0e0 | 0.02 | 82121 | 1.17 | 0 | 53.0 |

</details>
