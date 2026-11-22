# Preprint draft

`latexmk -pdf main.tex` (TeX Live: amsmath, cleveref, hyperref, booktabs,
microtype, lmodern; bibliography style `plainurl`). Not yet compiled: no TeX
toolchain on the machine it was written on; labels, citations and brace
balance were checked by script only.

Every number comes from `docs/results.md`, `docs/results-real.md` and
`docs/ablation.md` (2026-11-15).

## Open before submission

- Compile and fix layout (table widths: `tab:real` is wide for 11pt A4).
- `\todo{}` markers: affiliation/e-mail/date, Zenodo DOI for the dataset
  snapshots, novelty search (DBLP/Scholar, vendor patents).
- `refs.bib`: entries marked VERIFY come from search snippets; check every
  entry against the published version (and read each cited paper in full).
- Figures: none yet (candidates: budget along a series near θ; size vs
  share of time near θ from H2; ablation bars).
