"""How much does the judge disagree with itself?

The continuation exam's judge was re-run on identical answer pairs (the answers are cached;
only the verdict was regenerated). Comparing the two verdicts on the same inputs measures
the instrument's noise floor, which decides whether the tiny arm differences it reports mean
anything.
"""
import json, pathlib, collections, re

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
rows = json.loads((ROOT / "continuation-calls.json").read_text())

# every judge verdict ever recorded for the same question
by_question = collections.defaultdict(list)
for r in rows:
    ident = r["id"]
    base = ident.split("#")[0]
    if not base.startswith("judge__"):
        continue
    m = re.search(r"\{.*\}", r.get("text") or "", re.S)
    if not m:
        continue
    try:
        verdict = json.loads(m.group(0))
    except json.JSONDecodeError:
        continue
    if "A" not in verdict or "B" not in verdict:
        continue
    by_question[base].append((ident, verdict))

repeated = {k: v for k, v in by_question.items() if len(v) > 1}
print(f"questions with a repeated judge verdict: {len(repeated)} of {len(by_question)}")

spread = []
for question, verdicts in repeated.items():
    for label in ("A", "B"):
        vals = [v[1].get(label) for v in verdicts if isinstance(v[1].get(label), int)]
        if len(vals) > 1:
            spread.append(max(vals) - min(vals))
if spread:
    print(f"\nabsolute score spread on identical inputs (0-3 scale):")
    print(f"  mean {sum(spread)/len(spread):.2f}   max {max(spread)}")
    print(f"  identical: {sum(1 for s in spread if s == 0)}/{len(spread)}")
    for s in sorted(set(spread)):
        print(f"  differs by {s}: {spread.count(s)}")

print("\nverdicts for the same question across runs:")
for question, verdicts in list(repeated.items())[:8]:
    shown = [(ident.split('#')[-1] if '#' in ident else 'first',
              v.get("A"), v.get("B"), v.get("better")) for ident, v in verdicts]
    print(f"  {question.replace('judge__', ''):22s} {shown}")

# What does that noise floor imply for the arms?
results = json.loads((ROOT / "continuation-results.json").read_text())
for arm in ("summary", "no-summary"):
    vals = [r["scores"][arm] for r in results if arm in r["scores"]]
    n = len(vals)
    mean = sum(vals) / n
    var = sum((v - mean) ** 2 for v in vals) / max(n - 1, 1)
    stderr = (var / n) ** 0.5
    print(f'\n{arm:12s} n={n}  mean {mean:.2f}  stderr {stderr:.2f}  '
          f'95% CI [{mean - 1.96*stderr:.2f}, {mean + 1.96*stderr:.2f}]')

wins = sum(1 for r in results if r["better"] == "summary")
losses = sum(1 for r in results if r["better"] == "no-summary")
ties = sum(1 for r in results if r["better"] == "tie")
print(f'\npaired: summary {wins}, no-summary {losses}, tie {ties}')
print(f'  of {wins+losses} decided, summary share {100*wins/max(wins+losses,1):.0f}%')
