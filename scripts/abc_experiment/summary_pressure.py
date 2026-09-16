import os as _os
import pathlib as _pathlib
import subprocess as _subprocess


def _checkout():
    """The checkout this script lives in (…/<checkout>/scripts/abc_experiment/x.py)."""
    return _pathlib.Path(__file__).resolve().parents[2]


def _main_checkout():
    """The main checkout, which owns the shared .future directory.

    `--git-common-dir` resolves to <main>/.git even when running from a worktree, so the
    research directory is found without depending on any absolute path.
    """
    try:
        out = _subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=_pathlib.Path(__file__).resolve().parent, capture_output=True, text=True,
            timeout=30)
        if out.returncode == 0 and out.stdout.strip():
            return _pathlib.Path(out.stdout.strip()).parent
    except Exception:
        pass
    return _checkout()


def _research():
    override = _os.environ.get("ABC_ROOT")
    if override:
        return _pathlib.Path(override)
    return _main_checkout() / ".future" / "research" / "abc-summary-a47313"


WORKTREE = _checkout()
REPO = _main_checkout()
ROOT = _research()

"""When does the model summary start to matter?

The value-recall exam showed the summary is a strict subset of C's projection, which makes
it redundant *as long as the protected originals are retained*. C retained every assistant
text block on every session measured, so that exam could not have detected the summary's
value even if it had one.

This varies the one thing that changes that: the context window. At a smaller window C must
drop assistant originals, and the summary becomes the only remaining carrier of whatever
they said.

Measured per assistant text block, using a distinctive phrase from the block rather than the
whole block (a summary paraphrases, so whole-block containment would understate it):

  kept verbatim   the phrase is in the originals
  carried by the  the phrase is in the summary but not the originals
  evidence
  lost            nowhere in the projection

Projection generation only -- no model calls.
"""
import json, pathlib, re, subprocess, sys, collections

ROOT = ROOT
DRIVER = REPO / "target" / "debug" / "examples" / "abc_c3_probe"
WINDOWS = [128_000, 32_000, 8_000, 4_000]
FROZEN = ROOT / "frozen-sessions"
FACTIONS = (0.4, 0.7, 1.0)


def distinctive(text, n=60):
    """A phrase worth searching for: the longest run of words with no sentence break."""
    best = ""
    for sentence in re.split(r"[。.!?\n]+", text):
        sentence = sentence.strip()
        if len(sentence) > len(best) and len(sentence) >= 24:
            best = sentence
    return best[:n] if best else ""


def corpus():
    out = []
    for task in ("export", "analysis", "pipeline"):
        for stage in (0, 3, 7):
            p = ROOT / "data" / f"{task}-{stage}.json"
            if not p.exists():
                continue
            d = json.loads(p.read_text())
            out.append((f"{task}__s{stage}", d["archive"] + d["tail"]))
    manifest = json.loads((FROZEN / "manifest.json").read_text())
    for name, meta in manifest.items():
        records = json.loads(pathlib.Path(meta["path"]).read_text())["records"]
        for index, frac in enumerate(FACTIONS):
            out.append((f"{name}__s{index}", records[:max(1, int(len(records) * frac))]))
    return out


def run(records, window, summary, label):
    tmp = ROOT / "pressure" / f"{label}-{window}-{int(summary)}.json"
    tmp.parent.mkdir(parents=True, exist_ok=True)
    tmp.write_text(json.dumps(records, ensure_ascii=False) + "\n")
    argv = [str(DRIVER), "--records", str(tmp), "--model", "future/deepseek-flash",
            "--window", str(window)]
    if not summary:
        argv.append("--no-summary")
    out = subprocess.run(argv, capture_output=True, text=True, timeout=900)
    if out.returncode != 0:
        return None
    return json.loads(out.stdout.strip().splitlines()[-1])


def partition(payload):
    """(whole projection, evidence half, summary half).

    The driver returns the projection as a message array; the saved study files flatten it
    with the same join, so this matches what was scored.
    """
    full = "\n\n".join(m["text"] for m in payload["projection"])
    slot = (payload.get("checkpoint") or {}).get("summary") or []
    slot_text = slot[0].get("text", "") if slot else ""
    mark = "Model handoff summary"
    if mark in slot_text:
        at = slot_text.find(mark)
        return full, slot_text[:at], slot_text[at:]
    return full, slot_text, ""


def blocks(records):
    return [r["text"] for r in records
            if r["kind"] == "text" and r["role"] == "assistant" and len(r.get("text", "")) >= 80]


print(f'{"window":>8s} {"arm":>13s} {"assistant blocks":>16s} {"kept":>6s} '
      f'{"by summary":>11s} {"by evidence":>12s} {"lost":>6s}')
totals = {}
for window in WINDOWS:
    for use_summary in (True, False):
        counts = collections.Counter()
        for label, records in corpus():
            payload = run(records, window, use_summary, label)
            if payload is None:
                continue
            full, evidence, summary = partition(payload)
            for text in blocks(records):
                phrase = distinctive(text)
                if not phrase:
                    continue
                counts["blocks"] += 1
                if phrase in full and phrase not in (evidence + summary):
                    counts["kept"] += 1
                elif phrase in summary:
                    counts["summary"] += 1
                elif phrase in evidence:
                    counts["evidence"] += 1
                else:
                    counts["lost"] += 1
        totals[(window, use_summary)] = counts
        arm = "C (summary)" if use_summary else "C (no summary)"
        print(f'{window:>8d} {arm:>13s} {counts["blocks"]:>16d} {counts["kept"]:>6d} '
              f'{counts["summary"]:>11d} {counts["evidence"]:>12d} {counts["lost"]:>6d}')

print("\nwhat the summary changes (assistant blocks whose phrase is nowhere verbatim)")
for window in WINDOWS:
    a, b = totals.get((window, True)), totals.get((window, False))
    if not a or not b:
        continue
    carried_a = a["summary"] + a["evidence"]
    carried_b = b["evidence"]
    print(f'  window {window:>7,d}: with summary {carried_a:>3d} carried, '
          f'{a["lost"]:>3d} lost   |   without {carried_b:>3d} carried, {b["lost"]:>3d} lost   '
          f'-> summary saves {b["lost"] - a["lost"]:+d}')
