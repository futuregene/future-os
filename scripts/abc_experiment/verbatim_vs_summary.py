"""The decisive test: does keeping assistant text VERBATIM beat only summarising it?

Both designs preserve assistant information, but differently:

  ours  : assistant prose is retained verbatim (protected originals) AND summarised
  codex : assistant prose is dropped from the retained history, and only survives
          inside the summary -- but its summary *reads* the whole history, so the
          information does pass through once

So on a questionnaire that asks for assistant-only facts, both should get *some*
of them. The difference should appear on EXACTNESS: verbatim wording, precise
codes, exact numbers -- the things a second, lossy compression degrades.

Isolation: one summary is generated from the full history (assistant included) and
is placed in BOTH projections. The only difference is the retained history beside
it. Closed-book, so retrieval cannot equalise them.
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM
from assistant_coverage import build_fixture
import abc_cache_shapes as shapes
import abc_external_strategies as external


def source_records(records):
    """Split the fixture the way each rule needs it.

    ours  : user + assistant text, plus tool records (evidence/tail follow C's rule)
    codex : user messages only, plus the summary -- no assistant text, no tool output
    """
    ours = [r for r in records if r["kind"] == "text"]
    codex = [r for r in records if r["kind"] == "text" and r["role"] == "user"]
    return ours, codex


SUMMARY_INSTRUCTION = (
    "Summarize the entire conversation above into a handoff summary for another agent. "
    "Read all of it, including every assistant message. Preserve exact codes, numbers and "
    "identifiers verbatim. Cover: the objective, decisions taken and the reason for each, "
    "any earlier claim that was retracted, open questions, current state, and next steps. "
    "State explicitly what is not established. Do not mention compaction.")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--stages", type=int, nargs="*", default=[0, 3])
    ap.add_argument("--budget", type=float, default=8.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "verbatim-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, messages, max_tokens, reuse=True):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior and reuse:
            return {"text": prior["text"], "error": prior.get("error")}
        body = {"model": args.model, "messages": messages, "stream": True,
                "max_tokens": max_tokens, "stream_options": {"include_usage": True},
                "thinking": {"type": "enabled"}, "reasoning_effort": "high"}
        row = ledger.reserve(identity, args.model, request_reserve(len(json.dumps(body).encode()), max_tokens))
        began = time.monotonic()
        out = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                             capture_output=True, text=True, timeout=900)
        parsed = parse_sse(out.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"))
        return parsed

    QUESTION = (
        "Review the archive through this stage and answer as one JSON object with string values.\n"
        "Answer these about the AGENT'S OWN stated positions -- its decisions, its reasons, its "
        "retractions -- not about tool output:\n"
        "  decision        : which parser the agent chose to use\n"
        "  rationale_code  : the RATIONALE_CODE identifier the agent wrote\n"
        "  retry_budget    : the current RETRY_BUDGET value\n"
        "  self_correction : which component the agent retracted its earlier claim about\n"
        "  open_question   : what the agent said was still unresolved\n"
        "  exact_phrase    : copy verbatim the sentence where the agent first explained why it "
        "chose that parser (the wording matters)\n"
        "Also: project, latest_throughput (number only), format, latest_version, buried, validation, "
        "blocker, device.\n"
        "Use UNKNOWN when a value is not reliably established; never guess.")

    results = []
    for stage in args.stages:
        records, gold = build_fixture(stage)
        # one summary from the FULL history, reused by both projections
        full = shapes.to_messages(records)
        parsed = call(f"verbatim__s{stage}__summary", full + [{"role": "user", "content": SUMMARY_INSTRUCTION}], 8192)
        summary = (parsed.get("text") or "").strip()
        if not summary:
            raise RuntimeError(f"stage {stage}: empty summary")
        print(json.dumps({"stage": stage, "summary_tokens": external.estimate_tokens(summary)}), flush=True)

        ours_text, codex_text = source_records(records)
        for rule, kept in (("ours-verbatim", ours_text), ("codex-summary-only", codex_text)):
            body = "<archived-conversation>\n" + shapes._plain(kept) + \
                   f"\n\n<state-summary>\n{summary}\n</state-summary>\n</archived-conversation>\n" + QUESTION
            parsed = call(f"verbatim__s{stage}__{rule}", [
                {"role": "system", "content": CLOSED_SYSTEM.replace(
                    " No tools or external evidence are available in this condition.", "")},
                {"role": "user", "content": body}], 8192, reuse=False)
            text = parsed.get("text") or ""
            answer, decoder, pos = None, json.JSONDecoder(), 0
            while pos < len(text):
                start = text.find("{", pos)
                if start < 0:
                    break
                try:
                    val, end = decoder.raw_decode(text[start:])
                except json.JSONDecodeError:
                    pos = start + 1
                    continue
                if isinstance(val, dict) and len(set(val) & set(gold)) >= 4:
                    answer = val
                    break
                pos = start + end
            answer = answer or {}
            marks = {}

            def norm(s):
                return " ".join(str(s).lower().split())

            for key, expected in gold.items():
                marks[key] = norm(answer.get(key)) == norm(expected)
            # assistant-specific: decisions must be exact; the verbatim phrase is scored by
            # whether the agent's own reasoning wording survives
            marks["exact_phrase"] = (
                "batch parser deadlocks" in norm(answer.get("exact_phrase"))
                or ("deadlock" in norm(answer.get("exact_phrase")) and "batch" in norm(answer.get("exact_phrase")))
            )
            marks["open_question"] = "cache adapter" in norm(answer.get("open_question")) if stage >= 3 else True
            results.append({"stage": stage, "rule": rule, "marks": marks,
                            "correct": sum(marks.values()), "total": len(marks),
                            "answer": answer, "cost": ledger.rows[-1].get("charged", 0)})
            print(json.dumps({"stage": stage, "rule": rule, "score": f'{sum(marks.values())}/{len(marks)}',
                              "cost": round(ledger.rows[-1].get("charged", 0), 4)}), flush=True)

    fields = list(results[0]["marks"])
    print(f'\n{"field":20s} {"ours-verbatim":>14s} {"codex-summary":>14s}   source')
    src = {f: "assistant" for f in ("decision", "rationale_code", "retry_budget", "self_correction",
                                    "open_question", "exact_phrase")}
    src.update({f: "user" for f in ("project", "latest_throughput", "format")})
    src.update({f: "tool" for f in ("latest_version", "buried", "validation", "blocker")})
    src["device"] = "unrecorded"
    for f in fields:
        a = sum(1 for r in results if r["rule"] == "ours-verbatim" and r["marks"].get(f))
        b = sum(1 for r in results if r["rule"] == "codex-summary-only" and r["marks"].get(f))
        n = len(args.stages)
        print(f'{f:20s} {f"{a}/{n}":>14s} {f"{b}/{n}":>14s}   {src.get(f, "?")}')
    for rule in ("ours-verbatim", "codex-summary-only"):
        rows = [r for r in results if r["rule"] == rule]
        print(f'  {rule:20s} total {sum(r["correct"] for r in rows)}/{sum(r["total"] for r in rows)}')

    pathlib.Path(args.root / "verbatim-results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
