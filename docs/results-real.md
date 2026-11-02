# Results (real Prometheus series)

First run on real data, 2026-11-02. 13 series, 7 days of raw 15 s scrapes
(40 320 samples each, no gaps), fetched with `scripts/fetch_real.py` from
the public demo servers: prometheus.demo.prometheus.io (node_exporter,
Prometheus, Caddy; retention 31 d) and demo.promlabs.com (a demo service).
The demo servers keep only 31 days, so the exact interval cannot be fetched
again later; the fetched data are committed as
`datasets/prom-demo-2026-11-02.tar.xz` (1.1 MB, sha256 `5a4d1c34…01ad6`).
The interval is recorded in its `manifest.txt`.

Reproduce:

```
tar -C data -xJf datasets/prom-demo-2026-11-02.tar.xz   # or: scripts/fetch_real.py data/real 7
cargo run --release -p alertsafe-bench -- h8 data/real
cargo run --release -p alertsafe-bench -- export DIR --real data/real
uv run baselines/safeguards/run.py DIR --only NAME     # per dataset; prod variants skipped
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
then no lossy codec may move those samples at all.

**Findings.**

- **`prod`: the deployed thresholds cost almost nothing.** Rule-aware
  coding takes 0.09 bits/value on average (0.43 in Prometheus-sized chunks
  of 120 samples, where the per-chunk stream overhead dominates). Uniform
  bounds tuned on the same data do as well (0.11), because a threshold
  that is never approached constrains nothing. This is the common case in
  production, and it says nothing about the method.
- **`q95` / `q50`: 3.5× / 2.0× below a tuned uniform bound** (mean 2.17 vs
  7.63 and 3.01 vs 6.05 bits/value), and 4.4× / 2.4× with the optimistic
  budget (κ = 1024: 1.73 and 2.55). That is well below the synthetic
  4.5–24×. Real series hover more and are more discrete than the
  generators.
- **Compression Safeguards beat us on real data**, by a lot on gauges.
  SZ3+SG best (smallest zero-mismatch point of its sweep): gauges 0.15 /
  0.29 bits/value vs our 0.89 / 1.84 (κ); counters 2.13 / 2.65 vs 2.26 /
  2.99. We are smaller on only 5 of 13 datasets per variant: bursty or
  slow counters (`pgmajfault` 1.9–2.2×, `disk_io` 2.2–5.6×, `demo_api_500`
  1.3×), `fs_root_avail`, and the near-constant `engine_queries`. On
  `node_load1` SG takes 0.25 bits/value at ε = range: SZ3 sends almost
  nothing, and the safeguard corrections carry all the information.
- **The safeguards' `rate` heuristic still breaks on real counters, but
  rarely**: `demo_cpu_seconds` (7/13 and 5/13 sweep points) and
  `prometheus_samples_appended@q95` (2/13). These series have no missed
  scrapes. On `requests-gappy` (synthetic, 8 % missed scrapes) it broke at
  11/13 points.
- **Why we lose: the step is set per block.** One quantizer exponent covers
  a block of 128 samples, so the tightest budget in the block (a sample
  next to θ) sets the step for all 128 samples, or that sample escapes at
  64 bits. SG corrects only the samples whose QoI actually changed, one by
  one, and codes the corrections losslessly. The optimistic budget (H7)
  narrows the gap on counters but not on gauges (`node_load1@q50`: 4.83 vs
  0.70), because its repair zeroes or halves whole supports.
- **Coder fixes found on these data** (in this change, rerun on synthetic
  in `results.md`):
  1. The coded size is not monotone in the budget. On `node_cpu0_idle@prod`
     the analytic budget gave 1.01 bits/value and half of it gave 0.48. The
     candidate exponents now include two finer grids below each budget's
     own (`FINER_STEPS`). Any finer step is admissible, and the exact
     block cost decides.
  2. `compress_with` also encodes the analytic budget scaled by 1/2, 1/4
     and 1/8 (`Options::scales`), each sound by Theorem 1, and keeps the
     smallest. This also makes κ > 1 never worse than κ = 1. Before, it was
     worse on `pgmajfault@q50` (4.27 vs 1.69) and `demo_api_500` (4.71 vs
     3.57). Cost: end-to-end `compress` goes from 24–43 to 5–9 MB/s (H3).
  3. `tuned` in the bench returned a mismatching sweep point when even the
     smallest nonzero uniform bound changed a decision. The sweep now
     starts at ε = 0.
- **Lossless real gauges are expensive**: 54.6 bits/value for `node_load1`
  (two-decimal values) and 55.6 for `node_timex_offset`. XOR coding
  (Gorilla-style) cannot use a decimal lattice. An ALP-style decimal mode
  would help both the lossless and the escape paths.


## H8: real Prometheus series (13 series, 7.0 days, raw 15s scrapes)

Sources: `prom-demo` = prometheus.demo.prometheus.io (node_exporter, Prometheus, Caddy),
`promlabs` = demo.promlabs.com (demo service). Rules: `prod` = upstream node-/prometheus-mixin
thresholds restricted to one series; `q95` / `q50` = θ recalibrated to the 95th percentile / median
of the rule's aggregate (see `real.rs`). Same columns as H1, plus H7 (κ=1024) and
Prometheus-sized chunks of 120. Zero changed decisions in every rule-aware cell (asserted).

| series | kind | rules | firings | lossless | uniform-safe | uniform-tuned | rule-aware | κ=1024 | rule-aware /120 | κ /120 | ×lossless | ×tuned | ×tuned (κ) | exact samples |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| node_load1 | gauge (prom-demo) | prod | 0 | 54.56 | 0.00 | 0.06 | 0.00 | 0.00 | 0.26 | 0.26 | 10998.6 | 13.0 | 14.2 | 0.00% |
| node_load1 | gauge (prom-demo) | q95 | 0 | 54.56 | 5.02 | 3.19 | 2.04 | 2.00 | 2.40 | 1.76 | 26.8 | 1.6 | 1.6 | 0.00% |
| node_load1 | gauge (prom-demo) | q50 | 8 | 54.56 | 5.02 | 5.02 | 4.85 | 4.83 | 4.62 | 4.58 | 11.3 | 1.0 | 1.0 | 0.00% |
| node_memory_available | gauge (prom-demo) | prod | 0 | 15.61 | 0.01 | 0.01 | 0.01 | 0.01 | 0.34 | 0.34 | 1919.2 | 0.7 | 0.9 | 0.00% |
| node_memory_available | gauge (prom-demo) | q95 | 2 | 15.61 | 11.33 | 12.33 | 1.68 | 1.63 | 2.17 | 1.51 | 9.3 | 7.3 | 7.6 | 0.00% |
| node_memory_available | gauge (prom-demo) | q50 | 80 | 15.61 | 11.33 | 10.33 | 3.40 | 3.38 | 3.90 | 3.36 | 4.6 | 3.0 | 3.1 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | prod | 0 | 13.13 | 0.00 | 0.00 | 0.00 | 0.00 | 0.33 | 0.33 | 2757.1 | 1.0 | 1.0 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | q95 | 12 | 13.13 | 0.91 | 1.48 | 0.07 | 0.05 | 0.43 | 0.35 | 189.1 | 21.4 | 27.1 | 0.00% |
| node_fs_root_avail | gauge (prom-demo) | q50 | 34 | 13.13 | 3.17 | 1.48 | 0.15 | 0.13 | 0.49 | 0.34 | 85.4 | 9.6 | 11.4 | 0.00% |
| node_timex_offset | gauge (prom-demo) | prod | 0 | 55.62 | 0.00 | 0.01 | 0.00 | 0.00 | 0.20 | 0.20 | 12188.0 | 1.2 | 1.2 | 0.00% |
| node_timex_offset | gauge (prom-demo) | q95 | 32 | 55.62 | 2.92 | 2.13 | 0.84 | 0.76 | 1.06 | 0.79 | 66.3 | 2.5 | 2.8 | 0.00% |
| node_timex_offset | gauge (prom-demo) | q50 | 185 | 55.62 | 5.05 | 4.16 | 0.89 | 0.87 | 1.06 | 0.95 | 62.2 | 4.7 | 4.8 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | prod | 0 | 0.02 | 0.01 | 0.01 | 0.01 | 0.01 | 0.14 | 0.14 | 4.2 | 1.1 | 1.1 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | q95 | 0 | 0.02 | 0.02 | 0.01 | 0.02 | 0.01 | 0.15 | 0.14 | 1.1 | 0.5 | 1.0 | 0.00% |
| prometheus_engine_queries | gauge (prom-demo) | q50 | 0 | 0.02 | 0.02 | 0.01 | 0.02 | 0.01 | 0.15 | 0.14 | 1.1 | 0.5 | 1.0 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | prod | 0 | 7.78 | 0.01 | 0.01 | 0.00 | 0.00 | 0.33 | 0.27 | 1632.9 | 1.1 | 1.1 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | q95 | 0 | 7.78 | 3.08 | 1.54 | 1.17 | 0.95 | 1.59 | 1.40 | 6.6 | 1.3 | 1.6 | 0.00% |
| node_vmstat_pgmajfault | counter (prom-demo) | q50 | 21 | 7.78 | 7.75 | 1.54 | 1.60 | 1.60 | 2.31 | 2.31 | 4.9 | 1.0 | 1.0 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | prod | 0 | 39.43 | 0.01 | 0.01 | 0.01 | 0.00 | 0.40 | 0.27 | 5230.1 | 1.3 | 2.2 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | q95 | 0 | 39.43 | 5.95 | 3.96 | 0.93 | 0.61 | 1.55 | 1.04 | 42.6 | 4.3 | 6.5 | 0.00% |
| node_disk_io_weighted_sda | counter (prom-demo) | q50 | 0 | 39.43 | 9.02 | 10.02 | 2.49 | 1.56 | 2.92 | 1.89 | 15.8 | 4.0 | 6.4 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | prod | 0 | 40.88 | 0.72 | 0.72 | 0.58 | 0.48 | 1.17 | 1.17 | 70.7 | 1.2 | 1.5 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | q95 | 1 | 40.88 | 8.17 | 7.16 | 3.45 | 2.48 | 4.34 | 3.07 | 11.9 | 2.1 | 2.9 | 0.00% |
| node_cpu0_idle | counter (prom-demo) | q50 | 73 | 40.88 | 10.14 | 9.16 | 5.68 | 4.37 | 6.56 | 4.66 | 7.2 | 1.6 | 2.1 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | prod | 0 | 22.34 | 0.20 | 0.20 | 0.20 | 0.20 | 0.87 | 0.87 | 112.6 | 1.0 | 1.0 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | q95 | 0 | 22.34 | 11.07 | 10.11 | 4.55 | 3.27 | 5.54 | 3.96 | 4.9 | 2.2 | 3.1 | 0.00% |
| prometheus_samples_appended | counter (prom-demo) | q50 | 146 | 22.34 | 11.07 | 10.11 | 7.22 | 5.93 | 8.24 | 6.03 | 3.1 | 1.4 | 1.7 | 0.00% |
| caddy_requests | counter (prom-demo) | q95 | 46 | 13.79 | 7.17 | 5.16 | 2.77 | 2.08 | 3.27 | 2.33 | 5.0 | 1.9 | 2.5 | 0.00% |
| caddy_requests | counter (prom-demo) | q50 | 135 | 13.79 | 7.17 | 5.16 | 2.32 | 1.92 | 2.77 | 2.16 | 6.0 | 2.2 | 2.7 | 0.00% |
| demo_api_foo_200 | counter (promlabs) | q95 | 0 | 17.15 | 12.54 | 6.26 | 4.34 | 3.24 | 5.36 | 3.53 | 4.0 | 1.4 | 1.9 | 0.00% |
| demo_api_foo_200 | counter (promlabs) | q50 | 336 | 17.15 | 7.99 | 6.26 | 3.17 | 2.35 | 4.05 | 2.74 | 5.4 | 2.0 | 2.7 | 0.00% |
| demo_api_foo_500 | counter (promlabs) | q95 | 1 | 9.76 | 9.76 | 4.14 | 3.31 | 3.31 | 4.16 | 4.16 | 2.9 | 1.2 | 1.2 | 0.00% |
| demo_api_foo_500 | counter (promlabs) | q50 | 387 | 9.76 | 9.76 | 4.14 | 3.37 | 3.37 | 4.27 | 4.27 | 2.9 | 1.2 | 1.2 | 0.00% |
| demo_cpu_seconds | counter (promlabs) | q95 | 52 | 41.71 | 12.35 | 41.71 | 2.99 | 2.11 | 3.24 | 2.24 | 13.9 | 13.9 | 19.8 | 0.15% |
| demo_cpu_seconds | counter (promlabs) | q50 | 313 | 41.71 | 13.22 | 11.32 | 4.00 | 2.80 | 4.68 | 2.96 | 10.4 | 2.8 | 4.0 | 0.00% |

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
| prod | 9 | 27.71 | 0.11 | 0.11 | 0.09 | 0.08 | 0.45 | 0.43 |
| q95 | 13 | 25.52 | 6.94 | 7.63 | 2.17 | 1.73 | 2.71 | 2.02 |
| q50 | 13 | 25.52 | 7.74 | 6.05 | 3.01 | 2.55 | 3.54 | 2.80 |


## H6: Compression Safeguards baseline (real Prometheus series)

`SZ3` is SZ3 with an absolute error bound; `SZ3+SG` wraps it in safeguards translated from the rules
(see `baselines/safeguards/run.py`); both are swept over the SZ3 bound. `tuned` = largest bound with no
changed decision and none below it; `SZ3+SG best` = smallest output with zero state mismatches, if any.
`rule-aware κ` is the optimistic budget with a posteriori repair (H7).
Mismatches are counted by the reference evaluator of this repository.

| dataset | lossless (ours) | uniform-tuned (ours) | rule-aware (ours) | rule-aware κ=1024 (ours) | SZ3 tuned | SZ3+SG best | SZ3+SG ε / range | SZ3+SG points with mismatches | SZ3+SG encode, s (mean) | ×SZ3+SG |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| node_load1@q95 | 54.56 | 3.19 | 2.04 | 2.00 | 7.09 | 0.25 | 1.0e0 | 0/13 | 0.0 | 0.1 |
| node_load1@q50 | 54.56 | 5.02 | 4.85 | 4.83 | 7.09 | 0.70 | 1.0e0 | 0/13 | 0.0 | 0.1 |
| node_memory_available@q95 | 15.61 | 12.33 | 1.68 | 1.63 | 15.81 | 0.19 | 1.0e0 | 0/13 | 0.0 | 0.1 |
| node_memory_available@q50 | 15.61 | 10.33 | 3.40 | 3.38 | 19.32 | 0.45 | 1.0e0 | 0/13 | 0.0 | 0.1 |
| node_fs_root_avail@q95 | 13.13 | 1.48 | 0.07 | 0.05 | 2.03 | 0.11 | 1.0e0 | 0/13 | 0.0 | 2.0 |
| node_fs_root_avail@q50 | 13.13 | 1.48 | 0.15 | 0.13 | 3.55 | 0.15 | 6.2e-2 | 0/13 | 0.0 | 1.1 |
| node_timex_offset@q95 | 55.62 | 2.13 | 0.84 | 0.76 | 3.08 | 0.17 | 1.0e0 | 0/13 | 0.0 | 0.2 |
| node_timex_offset@q50 | 55.62 | 4.16 | 0.89 | 0.87 | 12.80 | 0.10 | 1.0e0 | 0/13 | 0.0 | 0.1 |
| prometheus_engine_queries@q95 | 0.02 | 0.01 | 0.02 | 0.01 | 0.04 | 0.04 | 2.5e-1 | 0/13 | 0.3 | 4.0 |
| prometheus_engine_queries@q50 | 0.02 | 0.01 | 0.02 | 0.01 | 0.04 | 0.04 | 2.5e-1 | 0/13 | 0.3 | 4.0 |
| node_vmstat_pgmajfault@q95 | 7.78 | 1.54 | 1.17 | 0.95 | – | 1.79 | 1.6e-2 | 0/13 | 3.2 | 1.9 |
| node_vmstat_pgmajfault@q50 | 7.78 | 1.54 | 1.60 | 1.60 | – | 3.60 | 1.6e-2 | 0/13 | 3.2 | 2.2 |
| node_disk_io_weighted_sda@q95 | 39.43 | 3.96 | 0.93 | 0.61 | 16.73 | 3.43 | 2.4e-4 | 0/13 | 2.9 | 5.6 |
| node_disk_io_weighted_sda@q50 | 39.43 | 10.02 | 2.49 | 1.56 | – | 3.38 | 2.4e-4 | 0/13 | 3.1 | 2.2 |
| node_cpu0_idle@q95 | 40.88 | 7.16 | 3.45 | 2.48 | – | 1.16 | 3.9e-3 | 0/13 | 0.6 | 0.5 |
| node_cpu0_idle@q50 | 40.88 | 9.16 | 5.68 | 4.37 | – | 2.79 | 9.5e-7 | 0/13 | 0.5 | 0.6 |
| prometheus_samples_appended@q95 | 22.34 | 10.11 | 4.55 | 3.27 | – | 1.66 | 9.5e-7 | 2/13 | 2.1 | 0.5 |
| prometheus_samples_appended@q50 | 22.34 | 10.11 | 7.22 | 5.93 | – | 2.97 | 9.5e-7 | 0/13 | 2.1 | 0.5 |
| caddy_requests@q95 | 13.79 | 5.16 | 2.77 | 2.08 | 8.31 | 1.38 | 3.9e-3 | 0/13 | 6.8 | 0.7 |
| caddy_requests@q50 | 13.79 | 5.16 | 2.32 | 1.92 | 8.31 | 1.54 | 3.9e-3 | 0/13 | 6.2 | 0.8 |
| demo_api_foo_200@q95 | 17.15 | 6.26 | 4.34 | 3.24 | – | 2.02 | 2.5e-1 | 0/13 | 6.8 | 0.6 |
| demo_api_foo_200@q50 | 17.15 | 6.26 | 3.17 | 2.35 | – | 1.15 | 2.5e-1 | 0/13 | 6.8 | 0.5 |
| demo_api_foo_500@q95 | 9.76 | 4.14 | 3.31 | 3.31 | – | 4.27 | 3.9e-3 | 0/13 | 6.9 | 1.3 |
| demo_api_foo_500@q50 | 9.76 | 4.14 | 3.37 | 3.37 | – | 4.33 | 3.9e-3 | 0/13 | 7.2 | 1.3 |
| demo_cpu_seconds@q95 | 41.71 | 41.71 | 2.99 | 2.11 | – | 1.30 | 1.5e-5 | 7/13 | 5.7 | 0.6 |
| demo_cpu_seconds@q50 | 41.71 | 11.32 | 4.00 | 2.80 | – | 1.40 | 1.5e-5 | 5/13 | 5.8 | 0.5 |

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
| 9.5e-7 | 10.50 | 8 | 10.53 | 0 | 0.0 |
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
| 1.0e0 | 0.03 | 131 | 0.06 | 0 | 3.8 |

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
| 1.0e0 | 0.03 | 131 | 0.06 | 0 | 3.7 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_vmstat_pgmajfault@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.54 | 19396 | 5.84 | 0 | 3.5 |
| 2.4e-7 | 4.47 | 18889 | 5.99 | 0 | 3.2 |
| 9.5e-7 | 4.44 | 19684 | 5.88 | 0 | 3.5 |
| 3.8e-6 | 3.87 | 19770 | 5.45 | 0 | 2.8 |
| 1.5e-5 | 1.81 | 19443 | 3.91 | 0 | 3.4 |
| 6.1e-5 | 0.71 | 1597 | 3.04 | 0 | 3.0 |
| 2.4e-4 | 0.29 | 1362 | 2.60 | 0 | 3.5 |
| 9.8e-4 | 0.11 | 2089 | 2.42 | 0 | 3.2 |
| 3.9e-3 | 0.06 | 1679 | 2.17 | 0 | 3.6 |
| 1.6e-2 | 0.06 | 1210 | 1.79 | 0 | 2.1 |
| 6.2e-2 | 0.04 | 1002 | 1.93 | 0 | 3.2 |
| 2.5e-1 | 0.04 | 1002 | 1.93 | 0 | 2.8 |
| 1.0e0 | 0.04 | 1002 | 1.93 | 0 | 3.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_vmstat_pgmajfault@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.54 | 18516 | 7.83 | 0 | 3.4 |
| 2.4e-7 | 4.47 | 17916 | 8.57 | 0 | 3.1 |
| 9.5e-7 | 4.44 | 19300 | 9.17 | 0 | 3.5 |
| 3.8e-6 | 3.87 | 19735 | 8.97 | 0 | 2.9 |
| 1.5e-5 | 1.81 | 18802 | 8.28 | 0 | 3.5 |
| 6.1e-5 | 0.71 | 6984 | 8.29 | 0 | 3.1 |
| 2.4e-4 | 0.29 | 16431 | 9.61 | 0 | 3.6 |
| 9.8e-4 | 0.11 | 17431 | 9.53 | 0 | 3.3 |
| 3.9e-3 | 0.06 | 18171 | 9.11 | 0 | 3.6 |
| 1.6e-2 | 0.06 | 9752 | 3.60 | 0 | 2.2 |
| 6.2e-2 | 0.04 | 19898 | 8.29 | 0 | 3.2 |
| 2.5e-1 | 0.04 | 19898 | 8.29 | 0 | 2.8 |
| 1.0e0 | 0.04 | 19898 | 8.29 | 0 | 3.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_disk_io_weighted_sda@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 16.73 | 0 | 16.73 | 0 | 0.0 |
| 2.4e-7 | 8.54 | 1493 | 8.91 | 0 | 3.2 |
| 9.5e-7 | 5.26 | 3536 | 6.68 | 0 | 2.5 |
| 3.8e-6 | 2.23 | 733 | 5.16 | 0 | 2.8 |
| 1.5e-5 | 1.14 | 66 | 4.61 | 0 | 3.5 |
| 6.1e-5 | 0.70 | 149 | 3.84 | 0 | 3.5 |
| 2.4e-4 | 0.35 | 597 | 3.43 | 0 | 3.6 |
| 9.8e-4 | 0.21 | 4926 | 5.77 | 0 | 3.2 |
| 3.9e-3 | 0.07 | 1689 | 4.17 | 0 | 3.3 |
| 1.6e-2 | 0.04 | 1008 | 3.50 | 0 | 2.9 |
| 6.2e-2 | 0.04 | 1008 | 3.50 | 0 | 3.3 |
| 2.5e-1 | 0.04 | 1008 | 3.50 | 0 | 2.8 |
| 1.0e0 | 0.04 | 1008 | 3.50 | 0 | 3.3 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_disk_io_weighted_sda@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 16.73 | 2 | 16.75 | 0 | 3.2 |
| 2.4e-7 | 8.54 | 792 | 8.91 | 0 | 2.9 |
| 9.5e-7 | 5.26 | 2312 | 6.68 | 0 | 2.5 |
| 3.8e-6 | 2.23 | 480 | 5.16 | 0 | 1.8 |
| 1.5e-5 | 1.14 | 95 | 4.61 | 0 | 3.3 |
| 6.1e-5 | 0.70 | 366 | 3.81 | 0 | 3.2 |
| 2.4e-4 | 0.35 | 1638 | 3.38 | 0 | 3.4 |
| 9.8e-4 | 0.21 | 19098 | 5.67 | 0 | 3.2 |
| 3.9e-3 | 0.07 | 19852 | 4.02 | 0 | 3.5 |
| 1.6e-2 | 0.04 | 20129 | 3.39 | 0 | 3.2 |
| 6.2e-2 | 0.04 | 20129 | 3.39 | 0 | 3.6 |
| 2.5e-1 | 0.04 | 20129 | 3.39 | 0 | 3.4 |
| 1.0e0 | 0.04 | 20129 | 3.39 | 0 | 3.6 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_cpu0_idle@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.49 | 31 | 4.61 | 0 | 0.6 |
| 2.4e-7 | 2.42 | 117 | 2.67 | 0 | 0.5 |
| 9.5e-7 | 0.98 | 327 | 1.45 | 0 | 0.5 |
| 3.8e-6 | 0.50 | 733 | 1.38 | 0 | 0.5 |
| 1.5e-5 | 0.16 | 1213 | 1.49 | 0 | 0.7 |
| 6.1e-5 | 0.09 | 1363 | 1.34 | 0 | 0.6 |
| 2.4e-4 | 0.06 | 1061 | 1.25 | 0 | 0.6 |
| 9.8e-4 | 0.05 | 1008 | 1.16 | 0 | 0.4 |
| 3.9e-3 | 0.05 | 1008 | 1.16 | 0 | 0.4 |
| 1.6e-2 | 0.05 | 1008 | 1.18 | 0 | 0.5 |
| 6.2e-2 | 0.05 | 1008 | 1.16 | 0 | 0.7 |
| 2.5e-1 | 0.05 | 1008 | 1.16 | 0 | 0.7 |
| 1.0e0 | 0.03 | 20155 | 1.63 | 0 | 0.5 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>node_cpu0_idle@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 4.49 | 332 | 4.91 | 0 | 0.5 |
| 2.4e-7 | 2.42 | 1299 | 3.47 | 0 | 0.6 |
| 9.5e-7 | 0.98 | 3266 | 2.79 | 0 | 0.5 |
| 3.8e-6 | 0.50 | 4929 | 2.95 | 0 | 0.5 |
| 1.5e-5 | 0.16 | 8002 | 2.97 | 0 | 0.6 |
| 6.1e-5 | 0.09 | 9020 | 3.08 | 0 | 0.7 |
| 2.4e-4 | 0.06 | 9053 | 2.98 | 0 | 0.6 |
| 9.8e-4 | 0.05 | 9398 | 2.90 | 0 | 0.6 |
| 3.9e-3 | 0.05 | 9398 | 2.90 | 0 | 0.4 |
| 1.6e-2 | 0.05 | 9147 | 2.87 | 0 | 0.4 |
| 6.2e-2 | 0.05 | 9398 | 2.90 | 0 | 0.5 |
| 2.5e-1 | 0.05 | 9398 | 2.90 | 0 | 0.7 |
| 1.0e0 | 0.03 | 16774 | 3.26 | 0 | 0.7 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_samples_appended@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 2.96 | 98 | 3.32 | 0 | 3.4 |
| 2.4e-7 | 1.22 | 480 | 2.11 | 0 | 2.3 |
| 9.5e-7 | 0.32 | 954 | 1.66 | 0 | 2.4 |
| 3.8e-6 | 0.10 | 1730 | 2.07 | 0 | 2.2 |
| 1.5e-5 | 0.06 | 1362 | 1.91 | 0 | 2.4 |
| 6.1e-5 | 0.04 | 1000 | 1.87 | 0 | 2.2 |
| 2.4e-4 | 0.04 | 1000 | 1.87 | 0 | 2.4 |
| 9.8e-4 | 0.04 | 1000 | 1.87 | 0 | 2.1 |
| 3.9e-3 | 0.05 | 18825 | 2.75 | 9 | 1.9 |
| 1.6e-2 | 0.05 | 19841 | 2.68 | 9 | 2.0 |
| 6.2e-2 | 0.04 | 1000 | 1.87 | 0 | 1.5 |
| 2.5e-1 | 0.04 | 1000 | 1.87 | 0 | 1.5 |
| 1.0e0 | 0.04 | 1000 | 1.87 | 0 | 1.2 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>prometheus_samples_appended@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 2.96 | 630 | 3.87 | 0 | 3.1 |
| 2.4e-7 | 1.22 | 2358 | 3.30 | 0 | 3.3 |
| 9.5e-7 | 0.32 | 5054 | 2.97 | 0 | 1.9 |
| 3.8e-6 | 0.10 | 8724 | 3.37 | 0 | 2.1 |
| 1.5e-5 | 0.06 | 10106 | 3.48 | 0 | 1.7 |
| 6.1e-5 | 0.04 | 9790 | 3.50 | 0 | 2.3 |
| 2.4e-4 | 0.04 | 9790 | 3.50 | 0 | 2.0 |
| 9.8e-4 | 0.04 | 9790 | 3.50 | 0 | 2.5 |
| 3.9e-3 | 0.05 | 16481 | 4.65 | 0 | 1.9 |
| 1.6e-2 | 0.05 | 17380 | 4.58 | 0 | 2.0 |
| 6.2e-2 | 0.04 | 9790 | 3.50 | 0 | 1.9 |
| 2.5e-1 | 0.04 | 9790 | 3.50 | 0 | 1.5 |
| 1.0e0 | 0.04 | 9790 | 3.50 | 0 | 1.5 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>caddy_requests@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.31 | 0 | 8.31 | 0 | 0.0 |
| 2.4e-7 | 6.05 | 11 | 6.12 | 0 | 7.4 |
| 9.5e-7 | 3.61 | 48 | 3.77 | 0 | 7.6 |
| 3.8e-6 | 1.68 | 214 | 2.03 | 0 | 7.6 |
| 1.5e-5 | 0.69 | 4539 | 1.96 | 0 | 7.5 |
| 6.1e-5 | 0.24 | 2659 | 1.88 | 0 | 7.5 |
| 2.4e-4 | 0.12 | 2690 | 1.60 | 0 | 7.3 |
| 9.8e-4 | 0.07 | 3077 | 1.43 | 0 | 7.3 |
| 3.9e-3 | 0.05 | 3254 | 1.38 | 0 | 7.3 |
| 1.6e-2 | 0.06 | 20789 | 1.54 | 0 | 7.1 |
| 6.2e-2 | 0.04 | 2021 | 1.40 | 0 | 7.0 |
| 2.5e-1 | 0.04 | 2021 | 1.40 | 0 | 7.0 |
| 1.0e0 | 0.04 | 2021 | 1.40 | 0 | 7.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>caddy_requests@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.31 | 0 | 8.31 | 0 | 0.0 |
| 2.4e-7 | 6.05 | 50 | 6.15 | 0 | 7.5 |
| 9.5e-7 | 3.61 | 187 | 3.81 | 0 | 7.6 |
| 3.8e-6 | 1.68 | 886 | 2.16 | 0 | 7.5 |
| 1.5e-5 | 0.69 | 9066 | 2.08 | 0 | 7.5 |
| 6.1e-5 | 0.24 | 6116 | 2.03 | 0 | 7.3 |
| 2.4e-4 | 0.12 | 6112 | 1.74 | 0 | 6.5 |
| 9.8e-4 | 0.07 | 6285 | 1.57 | 0 | 5.6 |
| 3.9e-3 | 0.05 | 6896 | 1.54 | 0 | 6.3 |
| 1.6e-2 | 0.06 | 22065 | 1.71 | 0 | 6.8 |
| 6.2e-2 | 0.04 | 7467 | 1.56 | 0 | 6.3 |
| 2.5e-1 | 0.04 | 7467 | 1.56 | 0 | 5.9 |
| 1.0e0 | 0.04 | 7467 | 1.56 | 0 | 6.4 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_200@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.61 | 9 | 5.69 | 0 | 7.6 |
| 2.4e-7 | 3.71 | 45 | 3.90 | 0 | 7.6 |
| 9.5e-7 | 3.30 | 168 | 3.79 | 0 | 7.6 |
| 3.8e-6 | 1.58 | 701 | 2.62 | 0 | 7.5 |
| 1.5e-5 | 0.63 | 4292 | 2.61 | 0 | 7.4 |
| 6.1e-5 | 0.36 | 2644 | 2.46 | 0 | 7.0 |
| 2.4e-4 | 0.14 | 6578 | 2.66 | 0 | 5.9 |
| 9.8e-4 | 0.06 | 4340 | 2.46 | 0 | 6.1 |
| 3.9e-3 | 0.04 | 2011 | 2.28 | 0 | 6.5 |
| 1.6e-2 | 0.04 | 2011 | 2.28 | 0 | 6.7 |
| 6.2e-2 | 0.04 | 2011 | 2.28 | 0 | 6.4 |
| 2.5e-1 | 0.03 | 21161 | 2.02 | 0 | 5.6 |
| 1.0e0 | 0.04 | 2011 | 2.28 | 0 | 6.5 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_200@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 5.61 | 7 | 5.64 | 0 | 7.5 |
| 2.4e-7 | 3.71 | 34 | 3.77 | 0 | 6.6 |
| 9.5e-7 | 3.30 | 111 | 3.44 | 0 | 5.6 |
| 3.8e-6 | 1.58 | 311 | 1.86 | 0 | 6.1 |
| 1.5e-5 | 0.63 | 3841 | 1.67 | 0 | 5.2 |
| 6.1e-5 | 0.36 | 1623 | 1.60 | 0 | 6.8 |
| 2.4e-4 | 0.14 | 10041 | 1.84 | 0 | 7.4 |
| 9.8e-4 | 0.06 | 19733 | 1.60 | 0 | 7.2 |
| 3.9e-3 | 0.04 | 21874 | 1.39 | 0 | 7.3 |
| 1.6e-2 | 0.04 | 21874 | 1.39 | 0 | 7.0 |
| 6.2e-2 | 0.04 | 21874 | 1.39 | 0 | 7.1 |
| 2.5e-1 | 0.03 | 23616 | 1.15 | 0 | 7.0 |
| 1.0e0 | 0.04 | 21874 | 1.39 | 0 | 7.1 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_500@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 6.34 | 11875 | 7.91 | 0 | 7.5 |
| 2.4e-7 | 6.14 | 14101 | 7.78 | 0 | 7.7 |
| 9.5e-7 | 6.32 | 10979 | 7.90 | 0 | 7.6 |
| 3.8e-6 | 4.28 | 11429 | 6.11 | 0 | 7.5 |
| 1.5e-5 | 2.21 | 18091 | 5.72 | 0 | 7.5 |
| 6.1e-5 | 0.87 | 17941 | 6.13 | 0 | 7.3 |
| 2.4e-4 | 0.22 | 6638 | 5.78 | 0 | 7.2 |
| 9.8e-4 | 0.05 | 2278 | 4.28 | 0 | 6.9 |
| 3.9e-3 | 0.04 | 2094 | 4.27 | 0 | 6.3 |
| 1.6e-2 | 0.04 | 2094 | 4.27 | 0 | 5.6 |
| 6.2e-2 | 0.04 | 2094 | 4.27 | 0 | 6.5 |
| 2.5e-1 | 0.04 | 2094 | 4.27 | 0 | 6.3 |
| 1.0e0 | 0.04 | 2094 | 4.27 | 0 | 5.7 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_api_foo_500@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 6.34 | 17533 | 7.94 | 0 | 7.6 |
| 2.4e-7 | 6.14 | 19895 | 7.80 | 0 | 7.6 |
| 9.5e-7 | 6.32 | 16088 | 7.92 | 0 | 7.6 |
| 3.8e-6 | 4.28 | 16700 | 6.16 | 0 | 7.5 |
| 1.5e-5 | 2.21 | 22704 | 5.76 | 0 | 7.5 |
| 6.1e-5 | 0.87 | 22058 | 6.15 | 0 | 7.3 |
| 2.4e-4 | 0.22 | 16377 | 5.80 | 0 | 7.2 |
| 9.8e-4 | 0.05 | 25665 | 4.34 | 0 | 7.3 |
| 3.9e-3 | 0.04 | 25665 | 4.33 | 0 | 7.2 |
| 1.6e-2 | 0.04 | 25665 | 4.33 | 0 | 7.0 |
| 6.2e-2 | 0.04 | 25665 | 4.33 | 0 | 7.0 |
| 2.5e-1 | 0.04 | 25665 | 4.33 | 0 | 7.0 |
| 1.0e0 | 0.04 | 25665 | 4.33 | 0 | 5.4 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_cpu_seconds@q95</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.17 | 8 | 8.22 | 0 | 5.7 |
| 2.4e-7 | 3.96 | 29 | 4.06 | 0 | 5.7 |
| 9.5e-7 | 2.17 | 87 | 2.39 | 0 | 6.5 |
| 3.8e-6 | 1.39 | 308 | 1.84 | 1 | 7.2 |
| 1.5e-5 | 0.46 | 1123 | 1.30 | 0 | 6.9 |
| 6.1e-5 | 0.20 | 2219 | 1.53 | 1 | 6.8 |
| 2.4e-4 | 0.10 | 3353 | 1.57 | 0 | 5.9 |
| 9.8e-4 | 0.07 | 3978 | 1.55 | 0 | 5.4 |
| 3.9e-3 | 0.05 | 2011 | 1.38 | 1 | 6.1 |
| 1.6e-2 | 0.04 | 2011 | 1.39 | 1 | 4.6 |
| 6.2e-2 | 0.04 | 2011 | 1.39 | 1 | 4.7 |
| 2.5e-1 | 0.04 | 2011 | 1.39 | 1 | 4.5 |
| 1.0e0 | 0.04 | 2011 | 1.39 | 1 | 4.0 |

</details>

<details><summary>SZ3 / SZ3+SG sweep, <code>demo_cpu_seconds@q50</code></summary>

| ε / range | SZ3 bits/value | SZ3 mismatches | SZ3+SG bits/value | SZ3+SG mismatches | SZ3+SG encode, s |
|---:|---:|---:|---:|---:|---:|
| 6.0e-8 | 8.17 | 24 | 8.22 | 0 | 6.2 |
| 2.4e-7 | 3.96 | 24 | 4.06 | 0 | 5.7 |
| 9.5e-7 | 2.17 | 145 | 2.40 | 0 | 5.8 |
| 3.8e-6 | 1.39 | 940 | 1.88 | 0 | 6.6 |
| 1.5e-5 | 0.46 | 3134 | 1.40 | 0 | 7.2 |
| 6.1e-5 | 0.20 | 7045 | 1.58 | 0 | 6.8 |
| 2.4e-4 | 0.10 | 14619 | 1.55 | 0 | 6.6 |
| 9.8e-4 | 0.07 | 15492 | 1.48 | 0 | 5.8 |
| 3.9e-3 | 0.05 | 18423 | 1.27 | 3 | 5.2 |
| 1.6e-2 | 0.04 | 18911 | 1.24 | 3 | 6.3 |
| 6.2e-2 | 0.04 | 18911 | 1.24 | 3 | 4.6 |
| 2.5e-1 | 0.04 | 18911 | 1.24 | 3 | 4.2 |
| 1.0e0 | 0.04 | 18911 | 1.24 | 3 | 3.9 |

</details>
