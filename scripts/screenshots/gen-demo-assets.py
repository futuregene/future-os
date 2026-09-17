#!/usr/bin/env python3
"""Generate the demo figures shown inside captured conversations.

The harness serves these PNGs as if they were files the agent produced in the
demo workspace (`desktop/shot/assets/`, gitignored — they are generated output,
not fixtures). `scripts/screenshots/capture.py` runs this automatically when the
files are missing; run it by hand only after changing the pictures.

Requires matplotlib:

    pip install matplotlib && python3 scripts/screenshots/gen-demo-assets.py
"""

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
DEFAULT_OUT = ROOT / "desktop" / "shot" / "assets"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default=str(DEFAULT_OUT), help="output directory")
    args = parser.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    try:
        import matplotlib
    except ImportError:
        print("matplotlib is not installed; the demo figures cannot be generated")
        return 1

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    import numpy as np

    # Effect sizes from the two fictional papers, drawn as the agent's
    # "reproduced" plot.
    studies = ["Frank 2024\n(roulette, unknown odds)", "Dreher 2025\n(binary choice, known odds)"]
    effects = [0.41, -0.33]
    errors = [0.09, 0.07]

    fig, ax = plt.subplots(figsize=(6.4, 3.6), dpi=200)
    y = np.arange(len(studies))
    ax.barh(y, effects, xerr=errors, color=["#4f7cff", "#e2685f"], height=0.5, capsize=4)
    ax.set_yticks(y, studies, fontsize=9)
    ax.axvline(0, color="#8a8f98", lw=1)
    ax.set_xlabel("Effect size r (dopamine vs. risk taking)", fontsize=9)
    ax.set_title("Dopamine and risk taking: effect direction depends on task structure", fontsize=10)
    ax.set_xlim(-0.6, 0.65)
    ax.grid(axis="x", color="#e6e8ec", lw=0.8)
    ax.set_axisbelow(True)
    for spine in ("top", "right", "left"):
        ax.spines[spine].set_visible(False)
    ax.tick_params(axis="y", length=0)
    fig.tight_layout()
    fig.savefig(out / "effect-size.png", facecolor="white")
    plt.close(fig)

    # A forest plot for the second conversation.
    labels = ["Risk taking", "Loss aversion", "Exploration", "Reward learning", "Impulsivity"]
    est = [0.28, -0.19, 0.34, 0.11, -0.05]
    low = [0.12, -0.34, 0.18, -0.07, -0.21]
    high = [0.44, -0.04, 0.50, 0.29, 0.11]

    fig, ax = plt.subplots(figsize=(6.4, 3.4), dpi=200)
    y = np.arange(len(labels))[::-1]
    ax.errorbar(est, y, xerr=[np.array(est) - np.array(low), np.array(high) - np.array(est)],
                fmt="o", color="#3c5a9a", ecolor="#9aa9c4", elinewidth=2, capsize=3, ms=6)
    ax.set_yticks(y, labels, fontsize=9)
    ax.axvline(0, color="#c0c4cc", lw=1, ls="--")
    ax.set_xlabel("Standardized effect size (95% CI)", fontsize=9)
    ax.set_title("Dopamine measures and decision constructs", fontsize=10)
    ax.grid(axis="x", color="#eef0f4", lw=0.8)
    ax.set_axisbelow(True)
    for spine in ("top", "right", "left"):
        ax.spines[spine].set_visible(False)
    ax.tick_params(axis="y", length=0)
    fig.tight_layout()
    fig.savefig(out / "forest-plot.png", facecolor="white")
    plt.close(fig)

    print("wrote", out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
