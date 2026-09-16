"""Continuation exam: answer the questions real users actually asked.

Every earlier exam asked for exact values, which is the task a summary is worst at. This
one uses what real follow-up turns actually ask about -- the agent's own decisions, reasons
and comparisons -- and it takes boundary, question and reference answer from the real
session, so nothing is synthesised:

  * boundary      the records before a real user turn
  * question      that user turn, verbatim
  * reference     the assistant turns that followed it, verbatim, i.e. what the session
                  actually established in answer to that question

Both arms compress the same boundary through the same code path, differing only in whether
the model summary is generated. The answer is produced with no tools, so this measures what
the projection alone supports.

Scoring is a blind paired judgement: for each question the judge sees the reference answer
and both candidate answers with arm labels shuffled, and returns an absolute score for each
plus which is better. Shuffling controls for position bias; the absolute scores let the two
arms be compared without a judge preference for length.
"""
import argparse, json, pathlib, random, re, subprocess, sys, time

WT = pathlib.Path("/Users/geilige/future-os/.worktrees/session-history-a47313")
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
FROZEN = ROOT / "frozen-sessions"
DRIVER = pathlib.Path("/Users/geilige/future-os/target/debug/examples/abc_c3_probe")

JUDGE_SYSTEM = (
    "You are grading whether an answer to a question about an engineering session matches "
    "what the session actually established. You are given the question, the reference answer "
    "(what the session really said), and two candidate answers labelled A and B in random "
    "order. Judge only substance: does the candidate state the same facts, reasons, "
    "comparisons and conclusions as the reference? Ignore length, formatting and confidence. "
    "A candidate that invents specific facts not in the reference scores lower than one that "
    "states less but states nothing false. Reply with JSON only: "
    '{"A": <0-3>, "B": <0-3>, "better": "A"|"B"|"tie", "why": "<one short sentence>"}. '
    "0 = missing or wrong, 1 = touches the topic but lacks the substance, "
    "2 = most of the substance, 3 = the substance of the reference."
)


def turns(records):
    """Index every user turn that has a substantial question and a following answer."""
    out = []
    for index, r in enumerate(records):
        if r["kind"] != "text" or r["role"] != "user":
            continue
        question = " ".join(r.get("text", "").split())
        if len(question) < 8:
            continue
        # the reference answer: assistant turns until the next user turn
        answer = []
        for later in records[index + 1:]:
            if later["kind"] == "text" and later["role"] == "user":
                break
            if later["kind"] == "text" and later["role"] == "assistant":
                answer.append(later.get("text", ""))
        joined = "\n".join(answer).strip()
        if len(joined) < 120:
            continue
        out.append({"cut": index, "question": question, "reference": joined[:4000]})
    return out


def compress(records, label, summary, bridge_unused=None):
    tmp = ROOT / "cont" / f"{label}-{int(summary)}.json"
    tmp.parent.mkdir(parents=True, exist_ok=True)
    tmp.write_text(json.dumps(records, ensure_ascii=False) + "\n")
    argv = [str(DRIVER), "--records", str(tmp), "--model", "future/deepseek-flash",
            "--window", "128000"]
    if not summary:
        argv.append("--no-summary")
    out = subprocess.run(argv, capture_output=True, text=True, timeout=1200)
    if out.returncode != 0:
        return None
    payload = json.loads(out.stdout.strip().splitlines()[-1])
    return "\n\n".join(m["text"] for m in payload["projection"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=12.0)
    ap.add_argument("--per-session", type=int, default=6)
    args = ap.parse_args()

    ledger = Ledger(ROOT, args.budget)
    ledger.path = ROOT / "continuation-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []
    retired = 0
    for row in ledger.rows:
        if row["id"].startswith("judge__") and not row["id"].endswith("#judge-no-think") \
                and not row["id"].endswith("#cap-too-low"):
            row["id"] += "#judge-no-think"
            retired += 1
        elif not row["id"].endswith("#cap-too-low"):
            row["id"] += "#cap-too-low"
            row["note"] = "superseded: the output cap was consumed by reasoning, " \
                          "so the call returned no text"
            retired += 1
    if retired:
        ledger.save()
        print(f"retired {retired} rows whose output cap was too low", flush=True)

    def call(identity, messages, max_tokens, system=None, attempts=3, think=True):
        prior = next((r for r in ledger.rows if r["id"] == identity
                      and r.get("state") == "finished" and r.get("text")), None)
        if prior:
            return prior["text"]
        full = ([{"role": "system", "content": system}] if system else []) + messages
        for attempt in range(attempts):
            body = {"model": args.model, "messages": full, "stream": True,
                    "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
            if not think:
                body["thinking"] = {"type": "disabled"}
            row = ledger.reserve(identity + ("" if attempt == 0 else f"__retry{attempt}"),
                                 args.model,
                                 request_reserve(len(json.dumps(body).encode()), max_tokens))
            began = time.monotonic()
            out = subprocess.run([str(args.bridge)],
                                 input=json.dumps({"model": args.model, "body": body}),
                                 capture_output=True, text=True, timeout=1800)
            if out.returncode != 0:
                ledger.settle(row, error=f"exit {out.returncode}: {out.stderr[-200:]}")
                continue
            parsed = parse_sse(out.stdout)
            usage = parsed.get("usage") or {}
            ledger.settle(row, input_tokens=usage.get("prompt_tokens"),
                          output_tokens=usage.get("completion_tokens"),
                          credit_cost=usage.get("credit_cost"),
                          seconds=round(time.monotonic() - began, 3),
                          error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"))
            if (parsed.get("text") or "").strip():
                return parsed["text"]
        return ""

    ANSWER_SYSTEM = (
        "You are continuing an engineering session. You have a record of the work so far. "
        "Answer the user's question about it directly and concretely, using only what your "
        "record supports. If the record does not establish something, say so.")

    manifest = json.loads((FROZEN / "manifest.json").read_text())
    results = []
    for name, meta in manifest.items():
        records = json.loads(pathlib.Path(meta["path"]).read_text())["records"]
        candidates = turns(records)
        if not candidates:
            continue
        # the last few, where the session has real history behind it
        chosen = candidates[-args.per_session:] if len(candidates) > args.per_session \
            else candidates
        for turn in chosen:
            key = f"{name}-{turn['cut']}"
            proj = {}
            for arm in ("summary", "no-summary"):
                text = compress(records[:turn["cut"]], f"{key}-{arm}", arm == "summary")
                if text is None:
                    print(f"  {key}: compression failed for {arm}")
                    continue
                proj[arm] = text
            if len(proj) < 2:
                continue
            answers = {}
            for arm, text in proj.items():
                answers[arm] = call(
                    f"cont__{key}__{arm}",
                    [{"role": "user", "content":
                      text + "\n\n<user-question>\n" + turn["question"] + "\n</user-question>"}],
                    8192, system=ANSWER_SYSTEM)

            # blind paired judgement, order shuffled
            order = ["summary", "no-summary"]
            random.Random(hash(key) & 0xffff).shuffle(order)
            labels = {"A": order[0], "B": order[1]}
            payload = {
                "question": turn["question"],
                "reference_answer": turn["reference"],
                "A": answers[order[0]][:4000],
                "B": answers[order[1]][:4000],
            }
            judged = call(f"judge__{key}",
                          [{"role": "user", "content": json.dumps(payload, ensure_ascii=False)}],
                          1024, system=JUDGE_SYSTEM, think=False)
            verdict = {}
            match = re.search(r'\{.*\}', judged or "", re.S)
            if match:
                try:
                    verdict = json.loads(match.group(0))
                except json.JSONDecodeError:
                    verdict = {}
            scored = {labels[k]: v for k, v in verdict.items() if k in labels and isinstance(v, int)}
            results.append({"session": name, "cut": turn["cut"],
                            "question": turn["question"][:120],
                            "scores": scored, "better": labels.get(str(verdict.get("better")), "tie")
                            if verdict.get("better") in labels else "tie",
                            "why": verdict.get("why", ""),
                            "proj_tokens": {k: len(v) // 4 for k, v in proj.items()}})
            print(f'  {name} cut={turn["cut"]:>4d}  summary={scored.get("summary", "?")} '
                  f'no-summary={scored.get("no-summary", "?")}  better={results[-1]["better"]}',
                  flush=True)

    out = ROOT / "continuation-results.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print()
    for arm in ("summary", "no-summary"):
        vals = [r["scores"][arm] for r in results if arm in r["scores"]]
        if vals:
            print(f'  {arm:12s} mean {sum(vals)/len(vals):.2f}  '
                  f'({sum(vals)}/{3*len(vals)})  max {max(vals)}')
    wins = sum(1 for r in results if r["better"] == "summary")
    losses = sum(1 for r in results if r["better"] == "no-summary")
    ties = sum(1 for r in results if r["better"] == "tie")
    print(f'  paired: summary wins {wins}, no-summary wins {losses}, ties {ties}')
    print(f'ledger: {len(ledger.rows)} calls, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
