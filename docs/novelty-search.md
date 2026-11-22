# Novelty search (2026-11-22)

Question: does any compressor (paper, product or patent) guarantee that alert
decisions (threshold rules over windowed aggregates with hold durations) are
unchanged by lossy compression?

Answer so far: **no**. Nearest neighbours, all cited in `paper/sections/related.tex`:

| work | what it guarantees | difference |
|---|---|---|
| Compression Safeguards (Tyree et al., EGUsphere 2026) | pointwise / sign / stencil-QoI bounds by correction | closest; no alert automaton, `rate` extrapolation not expressible (see H6) |
| Ren et al., arXiv 2604.18801 (2026) | friends-of-friends cluster membership of particles (threshold predicates on distances), post-hoc correction | other domain; same "downstream decision" framing |
| Plato (Lin et al., PVLDB 2020) | deterministic error bounds on aggregate query results | bounds a number, not the side of a threshold |
| CAMEO (EDBT 2026), Shrink (BigData 2024), SZ3, TopoSZ, QoI (Jiao et al.) | ACF / L∞ / topology / QoI error | no discrete decisions |
| Grafana Adaptive Metrics, Chronosphere rollup rules | drop / aggregate series by usage | select series; Grafana skips metrics read by alerting rules |
| ibirfan/AAET (GitHub, 2026, no paper) | "alert-aware event-triggered" compression for base-station load; empirical only (0 missed high-load samples on synthetic data) | per-sample threshold, no guarantee, no windows/holds; not citable as publication |
| USPTO 12567870 "Lossy compression of time series data" | drops points based on changes in data | not rule-aware (full text not retrieved: 403) |

Queries run (web search, Crossref, arXiv API): alert-preserving / decision-preserving
lossy compression; threshold-crossing / exceedance-preserving compression;
query-aware lossy compression with aggregate guarantees; patents on lossy
compression with alert-threshold-dependent tolerance; Grafana / Chronosphere
cost controls.

Not done (bot-blocked or manual): DBLP and Google Scholar (anti-bot wall),
Google Patents full-text / CPC search (H03M7/30 with G06F11/30, G06F11/34),
Datadog / Splunk / Dynatrace patent portfolios. Worth a manual pass before
the journal version.
