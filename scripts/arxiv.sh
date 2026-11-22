#!/bin/sh
# Source tarball for arXiv: main.tex, sections, figure PDFs and a prebuilt
# main.bbl (arXiv does not run BibTeX when a .bbl is present).
#
#     scripts/arxiv.sh [OUT.tar.gz]     (default: alertsafe-arxiv.tar.gz)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
out=$(realpath "${1:-alertsafe-arxiv.tar.gz}")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp -r "$root/paper/main.tex" "$root/paper/refs.bib" "$root/paper/sections" "$tmp/"
mkdir "$tmp/figures" && cp "$root"/paper/figures/*.pdf "$tmp/figures/"
(cd "$tmp" && tectonic --keep-intermediates main.tex >/dev/null)
(cd "$tmp" && tar czf "$out" main.tex main.bbl sections/*.tex figures/*.pdf)
echo "$out: $(tar tzf "$out" | wc -l) files"
