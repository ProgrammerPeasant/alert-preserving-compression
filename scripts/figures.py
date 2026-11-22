# /// script
# requires-python = ">=3.10"
# dependencies = ["matplotlib>=3.8"]
# ///
"""Paper figures (paper/figures/*.pdf) from the result tables in docs/ and
the budget dumps in paper/figures/data/.

    uv run scripts/figures.py

Budget dumps (one-off, from the 7-day snapshot):

    alertsafe-bench budget data/real node_load1@q95 \\
        paper/figures/data/budget-node_load1@q95.csv
"""

import csv
import math
import os
import re
import sys
from pathlib import Path

import matplotlib

matplotlib.use("pdf")
import matplotlib.pyplot as plt  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "paper" / "figures"
DOCS = ROOT / "docs"

# Categorical slots 1-3 (validated: CVD dE >= 9, normal-vision dE >= 27);
# slot 3 is below 3:1 on white, so every series also differs in marker shape
# and is named in a legend.
BLUE, ORANGE, AQUA = "#2a78d6", "#eb6834", "#1baf7a"
INK, INK2, GRID = "#0b0b0b", "#52514e", "#e4e3df"
OURS, SG, UNIFORM = BLUE, ORANGE, AQUA

plt.rcParams.update(
    {
        "font.family": "serif",
        "font.serif": ["Latin Modern Roman", "CMU Serif", "DejaVu Serif"],
        "mathtext.fontset": "cm",
        "font.size": 8,
        "axes.titlesize": 8,
        "axes.labelsize": 8,
        "xtick.labelsize": 7,
        "ytick.labelsize": 7,
        "legend.fontsize": 7,
        "axes.edgecolor": INK2,
        "axes.labelcolor": INK,
        "xtick.color": INK2,
        "ytick.color": INK2,
        "text.color": INK,
        "axes.spines.top": False,
        "axes.spines.right": False,
        "axes.grid": True,
        "grid.color": GRID,
        "grid.linewidth": 0.5,
        "axes.axisbelow": True,
        "legend.frameon": False,
        "pdf.fonttype": 42,
        "savefig.bbox": "tight",
        "savefig.pad_inches": 0.02,
    }
)
PREVIEW = os.environ.get("FIG_PNG")
WIDTH = 6.3  # inches, text width of the 11pt A4 article


def save(fig, name: str):
    fig.savefig(OUT / f"{name}.pdf")
    if PREVIEW:  # FIG_PNG=DIR: PNG copies for a quick look
        fig.savefig(Path(PREVIEW) / f"{name}.png", dpi=200)


def md_table(path: Path, header_start: str, nth: int = 0) -> list[list[str]]:
    """Rows of the `nth` markdown table whose header starts with `header_start`."""
    lines = path.read_text().splitlines()
    hits = [i for i, l in enumerate(lines) if l.startswith(header_start)]
    i = hits[nth]
    rows = []
    for l in lines[i + 2 :]:
        if not l.startswith("|"):
            break
        rows.append([c.strip() for c in l.strip().strip("|").split("|")])
    return rows


def num(s: str) -> float:
    try:
        return float(s)
    except ValueError:
        return math.nan


# --- Figure: budget along a series -------------------------------------------


def fig_budget(name: str, start_h: float, hours: float, ylim: tuple[float, float]):
    path = OUT / "data" / f"budget-{name}.csv"
    with open(path) as f:
        lines = f.readlines()
    rules = [l.split() for l in lines if l.startswith("# rule")]
    rows = list(csv.DictReader(l for l in lines if not l.startswith("#")))
    t0 = int(rows[0]["t_ms"])
    t, x, lo, hi, y = [], [], [], [], []
    for r in rows:
        h = (int(r["t_ms"]) - t0) / 3.6e6
        if not start_h <= h <= start_h + hours:
            continue
        xv = float(r["x"])
        t.append((h - start_h) * 60)
        x.append(xv)
        lo.append(max(xv - float(r["down"]), ylim[0] - 1))
        hi.append(min(xv + float(r["up"]), ylim[1] + 1))
        y.append(float(r["y"]))
    theta = float(rules[0][4])

    fig, ax = plt.subplots(figsize=(WIDTH, 2.1))
    ax.fill_between(t, lo, hi, step="mid", color=BLUE, alpha=0.18, lw=0,
                    label="admissible interval $[x-d,\\ x+u]$")
    ax.plot(t, x, color=INK, lw=0.8, label="original $x$")
    ax.step(t, y, where="mid", color=ORANGE, lw=1.0, label="reconstruction $y$")
    ax.axhline(theta, color=INK2, lw=0.8, ls=(0, (4, 2)))
    ax.annotate(f"$\\theta = {theta:g}$", (t[-1], theta), xytext=(2, 2),
                textcoords="offset points", ha="right", va="bottom", color=INK2)
    ax.set_ylim(*ylim)
    ax.set_xlim(t[0], t[-1])
    ax.set_xlabel("time, min")
    ax.set_ylabel(name.split("@")[0])
    ax.legend(loc="upper left", ncols=3, bbox_to_anchor=(0, 1.16))
    save(fig, "budget")
    plt.close(fig)


# --- Figure: size vs Compression Safeguards ----------------------------------


def h6_rows(nth: int):
    rows = md_table(DOCS / "results-real.md", "| dataset | lossless (ours) | uniform-tuned", nth)
    return [(r[0], num(r[2]), num(r[4]), num(r[6])) for r in rows]


def fig_sg():
    panels = [("7 days", h6_rows(0)), ("30 days", h6_rows(1))]
    names = [r[0] for r in panels[0][1]]
    n = len(names)
    fig, axes = plt.subplots(1, 2, figsize=(WIDTH, 4.3), sharey=True)
    for ax, (title, rows) in zip(axes, panels):
        assert [r[0] for r in rows] == names
        for i, (_, uni, ours, sg) in enumerate(rows):
            yy = n - 1 - i
            lo_, hi_ = min(ours, sg), max(ours, sg)
            ax.plot([lo_, hi_], [yy, yy], color=GRID if ours <= sg else "#c9c8c2",
                    lw=2.0, solid_capstyle="round", zorder=1)
        yy = [n - 1 - i for i in range(n)]
        ax.scatter([r[1] for r in rows], yy, marker="s", s=12, color=UNIFORM,
                   label="uniform bound, tuned", zorder=2, lw=0)
        ax.scatter([r[3] for r in rows], yy, marker="D", s=14, color=SG,
                   label="SZ3 + Safeguards, best", zorder=3, lw=0)
        ax.scatter([r[2] for r in rows], yy, marker="o", s=16, color=OURS,
                   label="ours ($\\kappa = 1024$)", zorder=4, lw=0)
        wins = sum(1 for r in rows if r[2] < r[3])
        ax.set_title(f"{title}: ours smaller on {wins} of {n}", loc="left")
        ax.set_xscale("log")
        ax.set_xlim(0.005, 20)
        ax.set_xlabel("bits per value (log)")
        ax.grid(axis="y", visible=False)
    axes[0].set_yticks(range(n))
    axes[0].set_yticklabels(list(reversed(names)))
    axes[0].set_ylim(-0.7, n - 0.3)
    h, l = axes[0].get_legend_handles_labels()
    order = [2, 1, 0]
    fig.legend([h[i] for i in order], [l[i] for i in order], loc="lower center",
               ncols=3, bbox_to_anchor=(0.55, -0.04))
    fig.subplots_adjust(bottom=0.13, wspace=0.08)
    save(fig, "sg")
    plt.close(fig)


# --- Figure: gain vs closeness to the threshold (H2) --------------------------


def fig_h2():
    rows = md_table(DOCS / "results.md", "| θ | incidents/day")
    fig, ax = plt.subplots(figsize=(WIDTH / 2, 2.1))
    styles = {"0.5": (AQUA, "s"), "2": (BLUE, "o"), "6": (ORANGE, "D")}
    for inc, (color, marker) in styles.items():
        pts = [(num(r[3].rstrip("%")), num(r[6])) for r in rows if r[1] == inc]
        ax.scatter(*zip(*pts), color=color, marker=marker, s=14, lw=0,
                   label=f"{inc} incidents/day")
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("samples with budget < 0.01, %")
    ax.set_ylabel("bits per value")
    ax.legend(loc="upper left")
    save(fig, "h2")
    plt.close(fig)


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    which = set(sys.argv[1:]) or {"budget", "sg", "h2"}
    if "budget" in which:
        fig_budget("node_load1@q95", start_h=69.0, hours=2.0, ylim=(0.0, 1.6))
    if "sg" in which:
        fig_sg()
    if "h2" in which:
        fig_h2()
    print("written:", ", ".join(sorted(p.name for p in OUT.glob("*.pdf"))))
