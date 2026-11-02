#!/usr/bin/env python3
"""Fetch raw samples of real Prometheus series for the H8 experiment.

Pulls every sample (no resampling) of a fixed set of series from the public
Prometheus demo servers through range-vector instant queries
(`metric[1d]` at day boundaries; selectors are left-open, so the pieces tile
the interval exactly). Writes, per series, raw little-endian `i64`
millisecond timestamps (`NAME.ts`) and `f64` values (`NAME.vals`), plus a
`manifest.txt` that `alertsafe-bench h8 DIR` reads:

    series NAME KIND SOURCE SELECTOR
    rule   NAME AGG RANGE_S CMP THRESHOLD FOR_S KEEP_S ORIGIN

Rules are the upstream node-mixin / prometheus-mixin alerts the demo server
runs, restricted to their single-series part. Constant denominators
(MemTotal, filesystem size, CPU count, query concurrency limit) are folded
into the threshold, and they are checked to be constant over the interval.
`ORIGIN` says whether the rule is upstream as is (`mixin`), algebraically
rewritten (`mixin-folded`), or adapted to one series (`adapted`).

Standard library only. Usage: scripts/fetch_real.py OUT_DIR [days]
"""

import json
import struct
import sys
import time
import urllib.parse
import urllib.request
from decimal import Decimal
from pathlib import Path

PROM_DEMO = "https://prometheus.demo.prometheus.io"
PROMLABS = "https://demo.promlabs.com"
DAY = 86400


def query(base, expr, at=None):
    params = {"query": expr}
    if at is not None:
        params["time"] = str(at)
    url = f"{base}/api/v1/query?{urllib.parse.urlencode(params)}"
    for attempt in range(4):
        try:
            with urllib.request.urlopen(url, timeout=120) as r:
                # Values are Go's shortest round-trip decimal strings; parse
                # timestamps exactly so milliseconds are not rounded.
                d = json.loads(r.read(), parse_float=Decimal)
            break
        except Exception as e:  # noqa: BLE001 - retry any transport error
            if attempt == 3:
                raise
            print(f"  retry {expr!r}: {e}", file=sys.stderr)
            time.sleep(2 + 4 * attempt)
    if d["status"] != "success":
        raise RuntimeError(f"{expr}: {d}")
    return d["data"]["result"]


def scalar(base, expr, at):
    res = query(base, expr, at)
    if len(res) != 1:
        raise RuntimeError(f"{expr}: expected one series, got {len(res)}")
    return float(res[0]["value"][1])


def constant(base, expr, end, days):
    """Value of `expr`, asserting it did not change over the interval."""
    lo = scalar(base, f"min_over_time(({expr})[{days}d:1m])", end)
    hi = scalar(base, f"max_over_time(({expr})[{days}d:1m])", end)
    if lo != hi:
        raise RuntimeError(f"{expr} not constant: {lo} .. {hi}")
    return lo


def raw_samples(base, selector, end, days):
    ts, vals = [], []
    for k in range(days, 0, -1):
        at = end - (k - 1) * DAY
        res = query(base, f"{selector}[1d]", at)
        if len(res) != 1:
            raise RuntimeError(f"{selector}: expected one series, got {len(res)}")
        for t, v in res[0]["values"]:
            ts.append(int(Decimal(t) * 1000))
            vals.append(float(v))
    assert all(a < b for a, b in zip(ts, ts[1:])), selector
    return ts, vals


def main():
    out = Path(sys.argv[1])
    days = int(sys.argv[2]) if len(sys.argv) > 2 else 7
    out.mkdir(parents=True, exist_ok=True)
    # A fixed, hour-aligned end keeps every query on the same interval.
    end = int(time.time()) // 3600 * 3600 - 3600

    node = '{job="node"}'
    ncpu = scalar(PROM_DEMO, 'count(node_cpu_seconds_total{job="node",mode="idle"})', end)
    mem_total = constant(PROM_DEMO, f"node_memory_MemTotal_bytes{node}", end, days)
    fs_size = constant(PROM_DEMO, 'node_filesystem_size_bytes{job="node",mountpoint="/"}', end, days)
    qmax = constant(PROM_DEMO, 'prometheus_engine_queries_concurrent_max{job="prometheus"}', end, days)

    m = 60
    # (name, kind, base, selector, [rule ...]); rule = (name, agg, range_s,
    # cmp, threshold, for_s, keep_s, origin).
    series = [
        ("node_load1", "gauge", PROM_DEMO, f"node_load1{node}", [
            ("NodeSystemSaturation", "last", 0, ">", 2 * ncpu, 15 * m, 0, "mixin-folded"),
        ]),
        ("node_memory_available", "gauge", PROM_DEMO, f"node_memory_MemAvailable_bytes{node}", [
            ("NodeMemoryHighUtilization", "last", 0, "<", 0.10 * mem_total, 15 * m, 0, "mixin-folded"),
        ]),
        ("node_fs_root_avail", "gauge", PROM_DEMO,
         'node_filesystem_avail_bytes{job="node",mountpoint="/"}', [
            ("NodeFilesystemAlmostOutOfSpace", "last", 0, "<", 0.05 * fs_size, 30 * m, 0, "mixin-folded"),
            ("NodeFilesystemAlmostOutOfSpaceCrit", "last", 0, "<", 0.03 * fs_size, 30 * m, 0, "mixin-folded"),
        ]),
        ("node_timex_offset", "gauge", PROM_DEMO, f"node_timex_offset_seconds{node}", [
            ("NodeClockSkewDetected", "last", 0, ">", 0.05, 10 * m, 0, "adapted"),
            ("NodeClockSkewDetectedNeg", "last", 0, "<", -0.05, 10 * m, 0, "adapted"),
        ]),
        ("prometheus_engine_queries", "gauge", PROM_DEMO,
         'prometheus_engine_queries{job="prometheus"}', [
            ("PrometheusHighQueryLoad", "avg_over_time", 5 * m, ">", 0.8 * qmax, 15 * m, 0, "mixin-folded"),
        ]),
        ("node_vmstat_pgmajfault", "counter", PROM_DEMO, f"node_vmstat_pgmajfault{node}", [
            ("NodeMemoryMajorPagesFaults", "rate", 5 * m, ">", 500.0, 15 * m, 0, "mixin"),
        ]),
        ("node_disk_io_weighted_sda", "counter", PROM_DEMO,
         'node_disk_io_time_weighted_seconds_total{job="node",device="sda"}', [
            ("NodeDiskIOSaturation", "rate", 5 * m, ">", 10.0, 30 * m, 0, "mixin"),
        ]),
        ("node_cpu0_idle", "counter", PROM_DEMO,
         'node_cpu_seconds_total{job="node",cpu="0",mode="idle"}', [
            ("NodeCPUHighUsage", "rate", 2 * m, "<", 0.10, 15 * m, 0, "adapted"),
        ]),
        ("prometheus_samples_appended", "counter", PROM_DEMO,
         'prometheus_tsdb_head_samples_appended_total{job="prometheus",type="float"}', [
            ("PrometheusNotIngestingSamples", "rate", 5 * m, "<=", 0.0, 10 * m, 0, "adapted"),
        ]),
        ("node_net_rx_bytes", "counter", PROM_DEMO,
         'node_network_receive_bytes_total{job="node",device="eth0"}', []),
        ("caddy_requests", "counter", PROM_DEMO, None, []),
        ("demo_api_foo_200", "counter", PROMLABS,
         'demo_api_request_duration_seconds_count{instance="demo-service-0:10000",method="GET",path="/api/foo",status="200"}', []),
        ("demo_api_foo_500", "counter", PROMLABS,
         'demo_api_request_duration_seconds_count{instance="demo-service-0:10000",method="GET",path="/api/foo",status="500"}', []),
        ("demo_cpu_seconds", "counter", PROMLABS,
         'process_cpu_seconds_total{instance="demo-service-0:10000"}', []),
    ]

    lines = [
        f"# fetched {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}, "
        f"interval ({end - days * DAY}, {end}] unix s, {days} days",
        f"# ncpu={ncpu:g} mem_total={mem_total:.17g} fs_size={fs_size:.17g} query_max={qmax:g}",
    ]
    for name, kind, base, sel, rules in series:
        if sel is None:
            # Busiest single caddy request counter, whatever its labels are.
            top = query(base, "topk(1, caddy_http_requests_total)", end)
            if not top:
                print(f"skip {name}: no caddy counter", file=sys.stderr)
                continue
            lab = {k: v for k, v in top[0]["metric"].items() if k != "__name__"}
            sel = "caddy_http_requests_total{" + ",".join(
                f'{k}="{v}"' for k, v in sorted(lab.items())) + "}"
        try:
            ts, vals = raw_samples(base, sel, end, days)
        except RuntimeError as e:
            print(f"skip {name}: {e}", file=sys.stderr)
            continue
        (out / f"{name}.ts").write_bytes(struct.pack(f"<{len(ts)}q", *ts))
        (out / f"{name}.vals").write_bytes(struct.pack(f"<{len(vals)}d", *vals))
        gaps = sum(1 for a, b in zip(ts, ts[1:]) if b - a > 22_500)
        print(f"{name}: {len(ts)} samples, {gaps} gaps > 1.5 scrape, "
              f"range [{min(vals):.6g}, {max(vals):.6g}]")
        src = "prom-demo" if base == PROM_DEMO else "promlabs"
        lines.append(f"series {name} {kind} {src} {sel.replace(' ', '')}")
        for r in rules:
            rn, agg, rng, cmp, th, fr, keep, origin = r
            lines.append(f"rule {rn} {agg} {rng} {cmp} {th!r} {fr} {keep} {origin}")
    (out / "manifest.txt").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
