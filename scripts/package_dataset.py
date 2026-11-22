#!/usr/bin/env python3
"""Package the real-series snapshots (data/real, data/real30) for Zenodo.

    scripts/package_dataset.py OUT.tar.gz

Writes a tarball with both snapshots, the fetch script, a README describing
the format and the per-series statistics, a CSV export of every series and
SHA256SUMS. Standard library only.
"""

import hashlib
import io
import struct
import sys
import tarfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SNAPSHOTS = [("real-7d", ROOT / "data" / "real"), ("real-30d", ROOT / "data" / "real30")]
TOP = "alertsafe-prometheus-snapshots"


def read(path: Path, fmt: str) -> list:
    raw = path.read_bytes()
    return list(struct.unpack(f"<{len(raw) // 8}{fmt}", raw))


def series_of(manifest: str) -> list[tuple[str, str, str, str, list[str]]]:
    out = []
    for line in manifest.splitlines():
        f = line.split()
        if f and f[0] == "series":
            out.append((f[1], f[2], f[3], " ".join(f[4:]), []))
        elif f and f[0] == "rule":
            out[-1][4].append(" ".join(f[1:]))
    return out


def iso(ms: int) -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ms / 1000))


def main(out: Path) -> None:
    files: dict[str, bytes] = {}
    tables = []
    for name, src in SNAPSHOTS:
        manifest = (src / "manifest.txt").read_text()
        files[f"{name}/manifest.txt"] = manifest.encode()
        header = [l for l in manifest.splitlines() if l.startswith("#")]
        rows = []
        for s, kind, source, selector, rules in series_of(manifest):
            for ext in ("ts", "vals"):
                files[f"{name}/{s}.{ext}"] = (src / f"{s}.{ext}").read_bytes()
            ts = read(src / f"{s}.ts", "q")
            vals = read(src / f"{s}.vals", "d")
            assert len(ts) == len(vals) and all(a < b for a, b in zip(ts, ts[1:]))
            csv = io.StringIO()
            csv.write("timestamp_ms,value\n")
            for t, v in zip(ts, vals):
                csv.write(f"{t},{v!r}\n")
            files[f"{name}/csv/{s}.csv"] = csv.getvalue().encode()
            rows.append(
                f"| `{s}` | {kind} | {source} | `{selector}` | {len(ts)} | "
                f"{iso(ts[0])} | {iso(ts[-1])} | {len(rules)} |"
            )
        tables.append((name, header, rows))
    files["fetch_real.py"] = (ROOT / "scripts" / "fetch_real.py").read_bytes()

    readme = [
        "# Real Prometheus series for alert-preserving lossy compression",
        "",
        "Raw samples of 13 monitoring time series from the public Prometheus demo",
        "servers, used in the evaluation of *Alert-Preserving Lossy Compression of",
        "Monitoring Time Series* (B. Serykh, HSE University, 2026). Code:",
        "<https://github.com/ProgrammerPeasant/alert-preserving-compression>.",
        "",
        "Two snapshots of the same series: `real-7d` (7 days) and `real-30d` (30 days),",
        "every sample as scraped (no resampling), fetched with `fetch_real.py`",
        "(included) through range-vector queries. The demo servers keep 31 days of",
        "data, so the snapshots cannot be fetched again.",
        "",
        "Sources: `prom-demo` = <https://prometheus.demo.prometheus.io> (node_exporter,",
        "Prometheus, Caddy), `promlabs` = <https://demo.promlabs.com> (demo service).",
        "",
        "## Format",
        "",
        "Per series `NAME`:",
        "",
        "- `NAME.ts`: little-endian `int64`, Unix time in milliseconds, strictly increasing;",
        "- `NAME.vals`: little-endian `float64`, same length;",
        "- `csv/NAME.csv`: the same data as text (`timestamp_ms,value`; values in",
        "  shortest round-trip form, so parsing them gives the exact `float64`).",
        "",
        "`manifest.txt` lists the series and the alerting rules applied to them:",
        "",
        "    series NAME KIND SOURCE SELECTOR",
        "    rule   NAME AGG RANGE_S CMP THRESHOLD FOR_S KEEP_S ORIGIN",
        "",
        "`KIND` is `gauge` or `counter`. Rules are the upstream node-mixin /",
        "prometheus-mixin alerts restricted to one series; constant denominators",
        "(memory size, filesystem size, CPU count, query limit) are folded into",
        "`THRESHOLD`. `ORIGIN`: `mixin` (as upstream), `mixin-folded` (algebraically",
        "rewritten), `adapted` (adapted to a single series). Series without rule",
        "lines get generic `rate` rules in the evaluation. Header comments record",
        "the fetch time, the interval and the folded constants.",
        "",
        "Reading in Python:",
        "",
        "    import numpy as np",
        "    ts = np.fromfile('real-7d/node_load1.ts', '<i8')",
        "    x = np.fromfile('real-7d/node_load1.vals', '<f8')",
        "",
    ]
    for name, header, rows in tables:
        readme += [f"## {name}", ""]
        readme += [f"    {h}" for h in header] + [""]
        readme += [
            "| series | kind | source | selector | samples | first | last | rules |",
            "|---|---|---|---|---:|---|---|---:|",
            *rows,
            "",
        ]
    readme += [
        "## License",
        "",
        "This compilation is released under CC BY 4.0. The underlying values are",
        "operational metrics of public demo servers run by the Prometheus project and",
        "PromLabs; `fetch_real.py` is MIT-licensed (see the code repository).",
        "",
        "`SHA256SUMS` covers every other file in the archive.",
        "",
    ]
    files["README.md"] = "\n".join(readme).encode()
    sums = "".join(
        f"{hashlib.sha256(b).hexdigest()}  {p}\n" for p, b in sorted(files.items())
    )
    files["SHA256SUMS"] = sums.encode()

    mtime = time.time()
    with tarfile.open(out, "w:gz") as tar:
        for p, b in sorted(files.items()):
            info = tarfile.TarInfo(f"{TOP}/{p}")
            info.size, info.mtime, info.mode = len(b), mtime, 0o644
            tar.addfile(info, io.BytesIO(b))
    print(f"{out}: {len(files)} files, {out.stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(Path(sys.argv[1]))
