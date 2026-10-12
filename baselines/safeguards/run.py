# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "compression-safeguards==1.0.0rc3",
#     "numcodecs-safeguards==0.1.0rc3",
#     "numcodecs-wasm-sz3==0.8.4",
# ]
# ///
"""H6 baseline: Compression Safeguards (Tyree et al., 2026) around SZ3.

Reads the datasets written by `alertsafe-bench export DIR`, translates every
alert rule into safeguards, and compresses each series with SZ3 alone and with
SZ3 wrapped in the safeguards (`SafeguardedCodec`), sweeping the SZ3 absolute
error bound. Every reconstruction is written to DIR/<dataset>/sweep/ and the
sizes to DIR/<dataset>/sweep.tsv; `alertsafe-bench h6 DIR` then checks the
decisions with the reference evaluator.

Translation (the most favourable one we could find for the baseline):

* `F(x[w]) cmp θ` for `avg/sum/min/max_over_time`: one `qoi_eb_stencil` per
  window length L (in samples) that the rule reads at any evaluation phase,
  over the L samples ending at the centre, with the comparison itself as the
  QoI and an absolute error bound of 0, so the 0/1 outcome is preserved.
  Windows truncated by the start or the end of the series are protected only
  where they occur (`select` with a late-bound selector), not everywhere.
* `last cmp θ`: `qoi_eb_pw` with QoI `x cmp θ`, bound 0.
* `rate`: not expressible. Prometheus extrapolates to the window boundaries,
  which depend on the evaluation instant, not on the samples. We use the
  closest stencil QoI: the reset-corrected increase over the window divided by
  the sampled interval (timestamps as a late-bound constant), compared with θ.
  This is a heuristic, so `h6` may count mismatches for it.
* `for` / `keep_firing_for` need no translation: equal conditions imply equal
  trajectories (Lemma 1). The baseline cannot exploit them either.

Usage: `uv run baselines/safeguards/run.py DIR [--only NAME]`
"""

import argparse
import json
import time
from pathlib import Path

import numpy as np
from numcodecs_safeguards import SafeguardedCodec
from numcodecs_wasm_sz3 import Sz3

CMP = {">": ">", ">=": ">=", "<": "<", "<=": "<="}


def stencil(qoi: str, before: int, after: int = 0) -> dict:
    return dict(
        kind="qoi_eb_stencil",
        qoi=qoi,
        neighbourhood=[dict(axis=0, before=before, after=after, boundary="valid")],
        type="abs",
        eb=0,
    )


def window_qoi(agg: str, cmp: str, th: str, n: int) -> str:
    if n == 1:
        return f"X[0] {cmp} {th}"
    if agg == "avg_over_time":
        return f"sum(X) / {n} {cmp} {th}"
    if agg == "sum_over_time":
        return f"sum(X) {cmp} {th}"
    upper = cmp in (">", ">=")
    # max(X) > θ  <=>  any(X > θ);  max(X) < θ  <=>  all(X < θ); dually for min.
    if agg == "max_over_time":
        return f"{'any' if upper else 'all'}(X {cmp} {th})"
    if agg == "min_over_time":
        return f"{'all' if upper else 'any'}(X {cmp} {th})"
    raise ValueError(agg)


def rate_qoi(cmp: str, th: str, n: int) -> str:
    if n < 2:
        # Prometheus returns no sample for fewer than two points: nothing to keep.
        return ""
    return (
        f'V["inc"] = X[{n - 1}] - X[0] + sum(where(X[1:] < X[:-1], X[:-1], 0));'
        f'return V["inc"] / (C["t"][{n - 1}] - C["t"][0]) {cmp} {th};'
    )


def safeguards_for(meta: dict, n: int) -> tuple[list[dict], dict]:
    sgs: list[dict] = []
    # Truncated windows occur at one position each: [0, L) at the start of
    # the series, [n - L, n) at its end. They are selected per position.
    edge: dict[int, list[dict]] = {}

    def at(pos: int, sg: dict) -> None:
        edge.setdefault(pos, []).append(sg)

    for r in meta["rules"]:
        cmp, th = CMP[r["cmp"]], repr(r["threshold"])
        if r["agg"] == "last":
            sgs.append(dict(kind="qoi_eb_pw", qoi=f"x {cmp} {th}", type="abs", eb=0))
            continue
        make = (
            (lambda L: rate_qoi(cmp, th, L))
            if r["agg"] == "rate"
            else (lambda L: window_qoi(r["agg"], cmp, th, L))
        )
        full = set(r["window_lengths"])
        for L in sorted(full):
            if q := make(L):
                sgs.append(stencil(q, L - 1))
        for L in r["head_lengths"]:
            if L not in full and (q := make(L)):
                at(L - 1, stencil(q, L - 1))
        for L in r["tail_lengths"]:
            if L not in full and (q := make(L)):
                at(n - L, stencil(q, 0, L - 1))
    consts: dict = {}
    # `select` chooses with `np.choose`, which takes at most 63 options.
    items = sorted(edge.items())
    for g in range(0, len(items), 62):
        selector = np.zeros(n, dtype=np.int64)
        options: list[dict] = [dict(kind="assume_safe")]
        for pos, here in items[g : g + 62]:
            options.append(here[0] if len(here) == 1 else dict(kind="all", safeguards=here))
            selector[pos] = len(options) - 1
        name = f"edge{g // 62}"
        sgs.append(dict(kind="select", selector=name, safeguards=options))
        consts[name] = selector
    return sgs, consts


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("dir", type=Path)
    ap.add_argument("--only")
    ap.add_argument("--kmin", type=int, default=6, help="smallest sweep exponent")
    ap.add_argument("--kmax", type=int, default=18)
    args = ap.parse_args()
    for d in sorted(p for p in args.dir.iterdir() if (p / "meta.json").exists()):
        meta = json.loads((d / "meta.json").read_text())
        if args.only and meta["name"] != args.only:
            continue
        x = np.fromfile(d / "vals.f64", dtype="<f8")
        ts = np.fromfile(d / "ts.i64", dtype="<i8")
        t = (ts - ts[0]).astype(np.float64) / 1000.0
        sgs, consts = safeguards_for(meta, len(x))
        if any(r["agg"] == "rate" for r in meta["rules"]):
            consts["t"] = t
        (d / "sweep").mkdir(exist_ok=True)
        rows = ["# k eps sz3_bytes sg_bytes sg_secs"]
        # Same grid as the uniform sweep of the Rust bench: range * 4^(k-18).
        for k in range(args.kmin, args.kmax + 1):
            eps = meta["range"] * 4.0 ** (k - 18)
            sz3 = Sz3(eb_mode="abs", eb_abs=eps)
            enc = sz3.encode(x)
            np.asarray(sz3.decode(enc), dtype="<f8").reshape(-1).tofile(
                d / "sweep" / f"{k}_sz3.f64"
            )
            codec = SafeguardedCodec(codec=sz3, safeguards=sgs, fixed_constants=consts)
            t0 = time.perf_counter()
            sg_enc = codec.encode(x)
            secs = time.perf_counter() - t0
            y = np.asarray(codec.decode(sg_enc), dtype="<f8").reshape(-1)
            y.tofile(d / "sweep" / f"{k}_sg.f64")
            sz3_bytes = len(bytes(enc))
            sg_bytes = len(bytes(sg_enc))
            rows.append(f"{k} {eps!r} {sz3_bytes} {sg_bytes} {secs:.3f}")
            print(
                f"{meta['name']:>9} k={k:2} eps/range=4^{k - 18:<3} "
                f"sz3={8 * sz3_bytes / len(x):6.3f} b/v  "
                f"sz3+sg={8 * sg_bytes / len(x):6.3f} b/v  ({secs:.1f}s)",
                flush=True,
            )
        (d / "sweep.tsv").write_text("\n".join(rows) + "\n")


if __name__ == "__main__":
    main()
