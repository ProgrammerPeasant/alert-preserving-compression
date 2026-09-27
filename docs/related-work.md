# Related work and novelty check

Last checked: 2026-09-27 (web search: alert-preserving / decision-preserving /
query-aware / QoI-preserving lossy compression, time-series lossy compression
2025–2026, Compression Safeguards documentation, Prometheus downsampling,
observability vendor patents). Every item must still be read in full before
it is cited in the thesis.

## Closest work and how this project differs

| Work | What it guarantees | Overlap | Difference |
|---|---|---|---|
| **Compression Safeguards** (Tyree et al., EGUsphere preprint 2026; `compression-safeguards` on PyPI) | Declarative safety requirements wrapped around any compressor: pointwise error bounds, `sign` preservation relative to an offset, error bounds on quantities of interest over stencils (`qoi_eb_stencil`), logical combinators; enforced by computing corrections | **High.** `sign` with an offset preserves `x ⋈ θ` per sample; a `qoi_eb_stencil` over a 1-D time axis with a mean stencil and a sign/comparison expression can in principle preserve one window-average threshold decision on a regular grid | No temporal decision semantics (hold `for`, `keep_firing_for`, `Pending`/`Firing`); no irregular timestamps / left-open windows / unknown evaluation phase (Lemma 2); no PromQL `rate` extrapolation or counter-reset semantics (Proposition 1); no multi-rule min-composition as a budget; correction-based rather than an analytic budget; no streaming/lookahead analysis; aimed at gridded scientific data |
| QoI-preserving lossy compression (Liu et al., PVLDB 16(4), 2022) and follow-ups (FZ-VIS 2026, TOPIQ 2026) | Error bounds on derived quantities of interest, via error propagation to pointwise bounds | **Medium.** The margin/Lipschitz step of Theorem 1 is an instance of QoI error propagation | Threshold *decisions* rather than QoI error; temporal automaton; monitoring semantics |
| Topology/feature-preserving compression (TopoSZ, topological-guarantees framework 2025, cluster preservation 2026, spatiotemporally adaptive feature-preserving compression 2024) | Preservation of critical points, contour trees, clusters, detected features; adaptive error bounds near features | **Medium.** Level-set preservation at level θ is sign preservation of `x − θ`; adaptive bounds near features resemble the non-uniform budget | Features are static geometric objects; no time-window aggregates or alert state |
| CAMEO (EDBT 2026) | Bounded deviation of ACF/PACF under line simplification | Low | Statistical property, not discrete decisions |
| Error-bounded time-series compressors (SZ3, Serf, Machete, Cadence 2026, VIREL 2026, Shrink) | Pointwise error bound | Baselines | Uniform bound; no decision guarantee |
| Muñiz-Cuza et al. (EDBT 2024) | Empirical impact of error-bounded compression on forecasting | Low | Names other analytics as open |
| Grafana Adaptive Metrics / Adaptive Telemetry | Aggregates series not used in alerts or dashboards | Low | Series selection, no per-value guarantee |
| US patent 12567870 "Lossy compression of time series data" | Drops points when the series reaches a size threshold (from search snippet; full text not retrieved) | Low, verify | Size-driven thinning, no decision criterion (to confirm) |

No work found that uses equality of *alert state trajectories* of monitoring
rules as the correctness criterion of lossy compression.

## Consequences for the claimed novelty

1. **Keep:** the problem statement (decision preservation for monitoring
   rules with temporal semantics), phase robustness (Lemma 2), the analysis
   of PromQL-specific operators (`rate` extrapolation, counter resets:
   Proposition 1), the applicability boundary (Proposition 2), the
   $\delta$ / hold-aware relaxation (§7 of the formal model), and the
   empirical study on real rule sets.
2. **Downgrade:** "sufficient condition via Lipschitz aggregates" is standard
   QoI error propagation. Present it as an instantiation, and put the
   weight on what is specific: per-sample budget as a minimum over
   overlapping windows, lookahead, strictness/ties, floating-point soundness.
3. **Add a baseline:** Compression Safeguards (`sign` + `qoi_eb_stencil`
   wrapping SZ3 or ZFP) on regularly resampled series. This is the most
   likely reviewer objection; the thesis should show where it cannot
   express the rule (hold, `rate`, irregular timestamps) and how it compares
   where it can.

## Still to search before topic approval

DBLP / Google Scholar (2024–2026): "alert-aware compression",
"decision-preserving compression", "rule-aware downsampling",
"SLO-aware telemetry"; Google Patents full text for Datadog, Grafana Labs,
Chronosphere, Splunk, Dynatrace, New Relic; VLDB/SIGMOD/ICDE 2025–2026
proceedings for "query-aware lossy".

## Links

- Compression Safeguards: https://egusphere.copernicus.org/preprints/2026/egusphere-2026-4266/ , https://github.com/juntyr/compression-safeguards
- QoI-preserving lossy compression: https://dl.acm.org/doi/10.14778/3574245.3574255
- FZ-VIS: https://arxiv.org/abs/2608.08386 ; TOPIQ: https://arxiv.org/abs/2608.26912
- Spatiotemporally adaptive feature-preserving compression: https://arxiv.org/abs/2401.03317
- CAMEO: https://arxiv.org/abs/2501.14432
- Cadence: https://arxiv.org/abs/2609.06008 ; VIREL: https://arxiv.org/abs/2607.22433 ; Shrink: https://arxiv.org/abs/2503.13246
- Machete (TACO): https://dl.acm.org/doi/10.1145/3767158
- EDBT 2024 forecasting study: https://openproceedings.org/2024/conf/edbt/paper-102.pdf
- US 12567870: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/12567870
- Prometheus source used for the model: `promql/functions.go` (`extrapolatedRate`), `rules/alerting.go`
