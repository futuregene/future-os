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

"""A deterministic rubric for the continuation exam.

The free-form judge could not resolve the arms: its self-disagreement on identical inputs was
0.97 on a 0-3 scale, five times the effect being measured. This replaces it with a scorer that
has no variance at all.

Two deterministic measures per answer, both against the reference answer that the real session
actually gave:

  content recall   fraction of the reference's content terms (ASCII words + CJK bigrams)
                   that the candidate also contains. This is ROUGE-style coverage and is the
                   standard way to score a summary without a judge.
  referent recall  fraction of the reference's concrete referents (paths, ids, versions,
                   sizes, counts) that the candidate also produces.

What it gives up: neither can tell whether an answer captured the *reasoning* rather than its
wording. That is the trade the judge's noise floor forced -- an exact instrument answering a
narrower question beats a rich one whose answers are not reproducible.

The answers are generated here rather than reused, because the earlier run's prompt let the
model emit tool calls that nothing executed; those answers were truncated into `<tool_calls>`
markup and scored as if they were empty.
"""
import argparse, json, math, pathlib, re, subprocess, sys, time, collections

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse
from continuation_exam import turns, compress

ROOT = ROOT
FROZEN = ROOT / "frozen-sessions"

ENTITY = re.compile(
    r"\b(?:"
    r"[0-9a-f]{6,40}"
    r"|ver_[0-9a-f]+"
    r"|(?:ACK|RSN|trace)_[A-Za-z0-9_]+"
    r"|[A-Za-z0-9_][\w./-]*\.(?:rs|ts|py|md|json|toml|log|mjs|tsx|yml|yaml|css|html|pdf)"
    r"|\d+(?:\.\d+)?\s?(?:MiB|MB|GiB|GB|KB|B)"
    r"|PR\s?#\d+"
    r"|\d[\d,]{2,}"
    r")\b")
STOP_ENTITIES = {"1,000", "2,000", "100"}
ASCII_WORD = re.compile(r"[A-Za-z][A-Za-z0-9_.\-]{1,}")
CJK = re.compile(r"[\u3400-\u9fff]")
# Tool markup that must never count as answer content.
TOOL_MARKUP = re.compile(r"<tool_calls>.*?</tool_calls>|<tool\s+name=.*?</tool>", re.S)

STOP_EN = {
    "the", "and", "for", "that", "this", "with", "from", "have", "has", "had", "not", "but",
    "are", "was", "were", "will", "would", "can", "could", "should", "you", "your", "our",
    "its", "it's", "they", "them", "their", "there", "here", "then", "than", "when", "what",
    "which", "while", "into", "onto", "over", "under", "about", "after", "before", "because",
    "been", "being", "does", "did", "doing", "done", "each", "also", "more", "most", "some",
    "any", "all", "only", "just", "like", "use", "used", "using", "make", "made", "need",
    "needs", "let", "lets", "get", "got", "one", "two", "three", "yes", "no", "now", "new",
    "old", "same", "other", "such", "very", "well", "still", "even", "back", "out", "off",
    "i'll", "we'll", "it's", "don't", "doesn't", "isn't", "aren't", "won't", "can't",
}


def strip_markup(text):
    return TOOL_MARKUP.sub(" ", text or "")


def content_terms(text):
    """ASCII words plus CJK bigrams: the standard bag for mixed Chinese/English text."""
    terms = []
    for word in ASCII_WORD.findall(text or ""):
        word = word.lower().strip(".-_")
        if len(word) > 2 and word not in STOP_EN:
            terms.append(word)
    cjk = "".join(CJK.findall(text or ""))
    terms.extend(cjk[i:i + 2] for i in range(max(0, len(cjk) - 1)))
    return terms


def entities(text):
    return {e.strip() for e in ENTITY.findall(text or "")} - STOP_ENTITIES


def score(candidate, reference, question):
    cand = strip_markup(candidate)
    ref_terms = set(content_terms(reference))
    cand_terms = set(content_terms(cand))
    ref_ents = entities(reference)
    cand_ents = entities(cand)
    q_ents = entities(question)
    return {
        "content_total": len(ref_terms),
        "content_hits": len(ref_terms & cand_terms),
        "content_recall": len(ref_terms & cand_terms) / len(ref_terms) if ref_terms else None,
        "content_f1": (2 * len(ref_terms & cand_terms)
                       / (len(ref_terms) + len(cand_terms))
                       if (ref_terms and cand_terms) else 0.0),
        "ref_total": len(ref_ents),
        "ref_hits": len(ref_ents & cand_ents),
        "ref_recall": len(ref_ents & cand_ents) / len(ref_ents) if ref_ents else None,
        "q_total": len(q_ents),
        "q_hits": len(q_ents & cand_ents),
        "q_recall": len(q_ents & cand_ents) / len(q_ents) if q_ents else None,
        "chars": len(cand),
        "nonempty": bool(cand.strip()),
        "had_tool_markup": bool(TOOL_MARKUP.search(candidate or "")),
    }


def question_set():
    manifest = json.loads((FROZEN / "manifest.json").read_text())
    out = []
    for name, meta in manifest.items():
        records = json.loads(pathlib.Path(meta["path"]).read_text())["records"]
        for turn in turns(records):
            out.append({"session": name, "cut": turn["cut"],
                        "question": turn["question"], "reference": turn["reference"],
                        "records": records[:turn["cut"]]})
    return out


ANSWER_SYSTEM = (
    "You are continuing an engineering session. You have a record of the work so far. Answer "
    "the user's question directly and concretely from that record. You have NO tools and "
    "cannot read, run or fetch anything: answer in prose only, and never emit tool calls. If "
    "the record does not establish something, say so.")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=10.0)
    ap.add_argument("--only", nargs="*", default=None)
    ap.add_argument("--salt", default="",
                    help="regenerate answers under a new cache key, to measure answer variance")
    args = ap.parse_args()

    questions = question_set()
    if args.only:
        questions = [q for q in questions if q["session"] in args.only]

    ledger = Ledger(ROOT, args.budget)
    ledger.path = ROOT / "rubric-answers-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def answer(key, arm, projection, question):
        ident = f"rub__{args.salt}__{key}__{arm}" if args.salt else f"rub__{key}__{arm}"
        prior = next((r for r in ledger.rows if r["id"] == ident
                      and r.get("state") == "finished" and r.get("text")), None)
        if prior:
            return prior["text"]
        messages = [{"role": "system", "content": ANSWER_SYSTEM},
                    {"role": "user", "content":
                     projection + "\n\n<user-question>\n" + question + "\n</user-question>"}]
        text = ""
        for attempt, extra in enumerate(({}, {"thinking": {"type": "disabled"}})):
            body = {"model": args.model, "messages": messages, "stream": True,
                    "max_tokens": 8192, "stream_options": {"include_usage": True}}
            body.update(extra)
            row = ledger.reserve(ident + ("" if attempt == 0 else "__nothink"),
                                 args.model,
                                 request_reserve(len(json.dumps(body).encode()), 8192))
            began = time.monotonic()
            out = subprocess.run([str(args.bridge)],
                                 input=json.dumps({"model": args.model, "body": body}),
                                 capture_output=True, text=True, timeout=1800)
            if out.returncode != 0:
                ledger.settle(row, error=f"exit {out.returncode}")
                continue
            parsed = parse_sse(out.stdout)
            usage = parsed.get("usage") or {}
            ledger.settle(row, input_tokens=usage.get("prompt_tokens"),
                          output_tokens=usage.get("completion_tokens"),
                          credit_cost=usage.get("credit_cost"),
                          seconds=round(time.monotonic() - began, 3),
                          text=parsed.get("text"), finish=parsed.get("finish"))
            text = parsed.get("text") or ""
            if text.strip():
                break
        return text

    results = []
    for q in questions:
        key = f'{q["session"]}-{q["cut"]}'
        entry = {"session": q["session"], "cut": q["cut"],
                 "question": q["question"][:100]}
        for arm in ("summary", "no-summary"):
            projection = compress(q["records"], f"{key}-{arm}", arm == "summary")
            if projection is None:
                continue
            text = answer(key, arm, projection, q["question"])
            entry[arm] = score(text, q["reference"], q["question"])
            entry[arm]["answer_head"] = strip_markup(text)[:200]
        if "summary" in entry and "no-summary" in entry:
            results.append(entry)
        print(f'  {key:20s} content recall summary={entry.get("summary", {}).get("content_recall")} '
              f'no-summary={entry.get("no-summary", {}).get("content_recall")}', flush=True)

    scored = [r for r in results if r["summary"]["content_total"] >= 20]
    print(f"\nquestions answered {len(results)}, with enough reference content {len(scored)}")

    def mean(arm, field):
        vals = [r[arm][field] for r in scored if r[arm].get(field) is not None]
        return sum(vals) / len(vals) if vals else float("nan"), len(vals)

    print(f'{"":12s} {"content recall":>15s} {"content F1":>11s} {"referent rec":>13s} '
      f'{"question rec":>13s} {"chars":>7s}')
    for arm in ("summary", "no-summary"):
        cr, _ = mean(arm, "content_recall")
        f1, _ = mean(arm, "content_f1")
        rr, _ = mean(arm, "ref_recall")
        qr, _ = mean(arm, "q_recall")
        ch, _ = mean(arm, "chars")
        print(f'{arm:12s} {cr:>15.3f} {f1:>11.3f} {rr:>13.3f} {qr:>13.3f} {ch:>7.0f}')

    markup = sum(1 for r in results if r["summary"]["had_tool_markup"]
                 or r["no-summary"]["had_tool_markup"])
    print(f'\nanswers that still emitted tool markup: {markup}')
    print(f'non-empty: summary {sum(1 for r in results if r["summary"]["nonempty"])}/{len(results)}, '
          f'no-summary {sum(1 for r in results if r["no-summary"]["nonempty"])}/{len(results)}')

    for field, label in (("content_recall", "content recall"), ("ref_recall", "referent recall")):
        wins = sum(1 for r in scored
                   if (r["summary"][field] or 0) > (r["no-summary"][field] or 0) + 1e-9)
        losses = sum(1 for r in scored
                     if (r["no-summary"][field] or 0) > (r["summary"][field] or 0) + 1e-9)
        ties = len(scored) - wins - losses
        n = wins + losses
        p = (sum(math.comb(n, k) for k in range(0, min(wins, losses) + 1)) / (2 ** n) * 2) if n else 1.0
        diffs = [(r["summary"][field] or 0) - (r["no-summary"][field] or 0) for r in scored]
        mean_d = sum(diffs) / len(diffs)
        var_d = sum((d - mean_d) ** 2 for d in diffs) / max(len(diffs) - 1, 1)
        stderr = (var_d / len(diffs)) ** 0.5
        print(f'\npaired by {label}: summary {wins}, no-summary {losses}, tie {ties}')
        print(f'  mean paired difference {mean_d:+.4f}, sd {var_d ** 0.5:.4f}, '
              f'95% CI [{mean_d - 1.96*stderr:+.4f}, {mean_d + 1.96*stderr:+.4f}]')
        # smallest difference this design could detect at 80% power
        detectable = 2.8 * var_d ** 0.5 / len(diffs) ** 0.5
        print(f'  detectable difference at 80% power: {detectable:.4f} '
              f'(observed {abs(mean_d):.4f})')
        if n:
            print(f'  sign test n={n}, p={min(p, 1.0):.3f} (two-sided)')

    out = ROOT / "continuation-rubric-results.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print(f'\nwrote {out}')
    print(f'ledger: {len(ledger.rows)} calls, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
