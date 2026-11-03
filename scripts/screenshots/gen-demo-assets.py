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


def check_tick_labels_do_not_collide(fig, ax, minimum_gap: float = 4.0) -> None:
    """Fail the run when the chart's y tick labels touch each other.

    The transcript renders this plate ~768 css px wide and the README shows the
    whole frame at 900, so a stack of labels in the source is unreadable in both.
    Measuring beats eyeballing: a label's window extent is what the renderer will
    draw, and two of them overlapping is the defect — the same way the screenshot
    scenarios assert geometry instead of trusting the picture.
    """
    fig.canvas.draw()
    renderer = fig.canvas.get_renderer()
    boxes = []
    for label in ax.get_yticklabels():
        text = label.get_text()
        if not text:
            continue
        box = label.get_window_extent(renderer)
        boxes.append((text, box))
    for index, (text_a, box_a) in enumerate(boxes):
        for text_b, box_b in boxes[index + 1:]:
            overlap_x = min(box_a.x1, box_b.x1) - max(box_a.x0, box_b.x0)
            overlap_y = min(box_a.y1, box_b.y1) - max(box_a.y0, box_b.y0)
            if overlap_x > 0 and overlap_y > 0:
                raise SystemExit(
                    f"parser-throughput: the tick labels {text_a!r} and {text_b!r} overlap "
                    f"by {overlap_x:.1f}x{overlap_y:.1f} px — give the plate more height, or drop a row"
                )
    gaps = [
        max(box_a.y0, box_b.y0) - min(box_a.y1, box_b.y1)
        for index, (_, box_a) in enumerate(boxes)
        for _, box_b in boxes[index + 1:]
    ]
    closest = min(gaps) if gaps else 0.0
    if closest < minimum_gap:
        raise SystemExit(
            f"parser-throughput: the closest pair of tick labels is {closest:.1f} px apart, "
            f"under the {minimum_gap:.0f} px this chart needs to stay readable at 768 css px"
        )
    label_height = boxes[0][1].height if boxes else 0.0
    print(f"parser-throughput: {len(boxes)} tick labels of {label_height:.0f} px, "
          f"closest pair {closest:.1f} px apart")


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

    # The parser session's benchmark: the two bounds (`<=` then `<`) against the
    # same three cases the transcript's table lists, as the agent's plot script
    # would draw them.
    #
    # Drawn on the app's own dark surface rather than on white: this figure sits
    # inside a dark transcript, and a white plate in the middle of it reads as a
    # screenshot of something else. Its labels are the case expressions and the
    # unit — the same words the transcript's table uses, because a chart of
    # numbers is data, not prose.
    #
    # The plate is as tall as the labels need, not as tall as the bars happen to
    # be: four rows on a 190 px plate put 23 px labels 32 px apart, which reads as
    # a stack. Three rows on 240 px leaves them ~60 px apart, and the check at the
    # bottom of this block fails the run if a later edit closes that gap again.
    cases = ["((((1)))) × 64", "f(a, b, c, d)", "1+2+"]
    before = [11.70, 3.210, 0.412]
    after = [12.41, 3.194, 0.408]

    fig, ax = plt.subplots(figsize=(6.4, 1.15), dpi=200)
    fig.patch.set_facecolor("#0d1117")
    ax.set_facecolor("#0d1117")
    y = np.arange(len(cases))
    height = 0.34
    ax.barh(y + height / 2, before, height, color="#3d4b5f", label="before")
    ax.barh(y - height / 2, after, height, color="#4c9aff", label="after")
    # Only the delta, right after the longer of the two bars: the absolute
    # numbers are in the transcript's table, and a long label at this render
    # width ran into the axes edge.
    for index, (old, new) in enumerate(zip(before, after)):
        delta = (new - old) / old * 100
        ax.text(max(old, new) + 0.35, index, f"{delta:+.1f}%",
                va="center", fontsize=8, color="#c9d1d9")
    ax.set_yticks(y, cases, fontsize=9, color="#c9d1d9")
    ax.set_xlabel("µs/op, median of 5", fontsize=8.5, color="#8b949e")
    ax.set_xlim(0, 16)
    ax.grid(axis="x", color="#21262d", lw=0.8)
    ax.set_axisbelow(True)
    for spine in ("top", "right", "left"):
        ax.spines[spine].set_visible(False)
    ax.spines["bottom"].set_color("#30363d")
    ax.tick_params(axis="both", length=0, colors="#8b949e", labelsize=8)
    legend = ax.legend(loc="lower right", frameon=False, fontsize=7.5)
    for text in legend.get_texts():
        text.set_color("#8b949e")
    fig.tight_layout()
    check_tick_labels_do_not_collide(fig, ax)
    fig.savefig(out / "parser-throughput.png", facecolor=fig.get_facecolor())
    plt.close(fig)

    print("wrote", out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
