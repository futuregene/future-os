"""Combination validation: sticky summary + overflow trigger + verbatim assistant.

Each ingredient has separate evidence but they have never run together:

  * sticky summary (carries the previous summary forward)  -> 72/72, matched Codex
  * overflow trigger (compact when the live context fills) -> made full-history summarisation bounded
  * verbatim assistant retention                           -> 10/10 vs 0/10 under 15x+ compression

The combined design (call it C3):

    live accumulation   : records accumulate; a compaction fires on overflow
    summary             : sticky -- reads [previous summary] + records since
    retained context    : protected user+assistant originals
                        + C's deterministic tool evidence
                        + recent tail
                        + the sticky summary

Validation has to show three things, not one:

  1. ACCURACY vs Codex on the pre-registered questionnaire, same interface, uncapped.
  2. NO REGRESSION from the ingredients individually (C, C+sticky, verbatim).
  3. EXACT-IDENTIFIER retention under compression pressure, where the
     summary-only rule collapsed to 0/10.

Pre-registered pass criteria (fixed before the run):
  P1  >= Codex accuracy on the six-probe comparison (delta >= 0)
  P2  zero probes lost, and `buried` >= 6/6 on those probes
  P3  exact-identifier retention >= 9/10 at 15x compression, where summary-only scored 0/10
  P4  no field regresses versus plain C by more than 1 probe
"""
import argparse, json, pathlib, random, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from abc_compaction_experiment import (
    Ledger, request_reserve, parse_sse, CLOSED_SYSTEM, QUESTION, EXTERNAL_LIMIT)
from assistant_coverage import build_fixture as assistant_fixture
import abc_cache_shapes as shapes
import abc_external_strategies as external

PROBE_STAGES = (0, 3, 7)
SUMMARY_INSTRUCTION = (
    "Summarize the entire conversation above into a handoff summary for another agent that "
    "will continue this work. Read all of it, including every assistant message. Use exactly "
    "this Markdown structure:\n\n"
    "## Objective\n- [unresolved objective, or (none)]\n\n"
    "## Important Details\n- [constraints, decisions and why, facts, or (none)]\n\n"
    "## Work State\n### Completed\n- [verified work, or (none)]\n\n### Active\n- [current work, or (none)]\n\n"
    "### Blocked\n- [blockers and unknowns, or (none)]\n\n## Next Move\n1. [concrete next action]\n\n"
    "## Relevant Files\n- [exact path and why it matters, or (none)]\n\n"
    "## Historical values (carry forward EVERY earlier stage, one row each)\n"
    "| Stage | Version | Trace token | Validation | Blocker |\n|---|---|---|---|---|\n"
    "| 0 | ... | ... | ... | ... |\n\n"
    "Rules: keep every section; the table must carry forward every earlier stage's version AND "
    "trace token even when this stage's conversation does not repeat them; terse bullets; preserve "
    "exact paths, symbols, numeric values and error strings; state explicitly what is NOT "
    "established rather than inferring it; do not mention compaction.")


def size(items):
    return len(json.dumps(shapes.to_messages(items))) // 4


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--tasks", nargs="*", default=[])
    ap.add_argument("--skip-chain", action="store_true")
    ap.add_argument("--budget", type=float, default=7.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "c3-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, messages, max_tokens, reuse=True):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior and reuse:
            return {"text": prior["text"], "error": prior.get("error")}
        body = {"model": args.model, "messages": messages, "stream": True,
                "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
        row = ledger.reserve(identity, args.model, request_reserve(len(json.dumps(body).encode()), max_tokens))
        began = time.monotonic()
        out = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                             capture_output=True, text=True, timeout=1200)
        parsed = parse_sse(out.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"))
        return parsed

    # ── part 1: the C3 chain over the two synthetic chains ────────────────────
    print("=== C3 chain ===", flush=True)
    if args.skip_chain:
        args.tasks = []
    for task in args.tasks:
        slug = args.model.split("/")[-1]
        summary, live, since = None, [], []
        for stage in range(8):
            data = json.loads((args.root / "data" / f"{task}-{stage}.json").read_text())
            fresh = data["archive"] + data["tail"] if stage == 0 else data["newly_covered"]
            since = since + fresh
            ident = f"{task}__{slug}__s{stage}__C3"

            def summarize(tag):
                base = ([{"kind": "text", "role": "user", "id": "prior-summary",
                          "text": f"<prior-summary>\n{summary}\n</prior-summary>"}] if summary else [])
                msgs = shapes.to_messages(base + live + since)
                for attempt in range(3):
                    suffix = "" if attempt == 0 else f"__retry{attempt}"
                    p = call(f"c3__{ident}__{tag}{suffix}",
                             msgs + [{"role": "user", "content": SUMMARY_INSTRUCTION}], 32768,
                             reuse=(attempt == 0))
                    text = (p.get("text") or "").strip()
                    if text:
                        return text, size(base + live + since)
                    if p.get("error"):
                        raise RuntimeError(f"{ident}: summary error at {tag}: {p['error'][:200]}")
                raise RuntimeError(f"{ident}: empty summary at {tag} after retries")

            # overflow trigger, exactly as the Codex arm uses it
            while live and size(live) + size(since) > EXTERNAL_LIMIT:
                summary, est = summarize(f"auto{len(live)}")
                live, since = [], []
                print(json.dumps({"auto_compact": ident, "live_est": est}), flush=True)
            live = live + since
            since = []

            if stage not in PROBE_STAGES:
                continue
            out = args.root / "C3" / "projections" / f"{ident}.json"
            out.parent.mkdir(parents=True, exist_ok=True)
            if not out.exists():
                if summary is None:
                    summary, est = summarize("boundary")
                    live = []
                # C's projection (protected originals + evidence + tail) + sticky summary
                base_text = json.loads((args.root / "C" / "projections" /
                                        f"{task}__{slug}__s{stage}__C.json").read_text())["text"]
                value = base_text.replace("<recent-history>",
                                          f"<state-summary>\n{summary}\n</state-summary>\n\n<recent-history>", 1)
                out.write_text(json.dumps({
                    "id": ident, "text": value, "summary": summary,
                    "context_tokens": external.estimate_tokens(value),
                    "summary_tokens": external.estimate_tokens(summary),
                    "design": "sticky summary + overflow trigger + verbatim assistant originals",
                }, ensure_ascii=False, indent=2) + "\n")
                print(json.dumps({"stage": ident, "ctx": external.estimate_tokens(value),
                                  "summary_tokens": external.estimate_tokens(summary)}), flush=True)
            else:
                summary = json.loads(out.read_text())["summary"]
                print(json.dumps({"stage": ident, "skipped": "exists"}), flush=True)

    # ── part 2: exact-identifier retention under pressure, with the same design ─
    print("\n=== C3 under compression pressure (exact identifiers) ===", flush=True)
    for n_facts in (160,):
        rng = random.Random(7000 + n_facts)
        records, order = [], 0

        def rec(role, kind, ident, **extra):
            nonlocal order
            order += 1
            return {"id": ident, "role": role, "kind": kind, "order": order, **extra}

        records.append(rec("user", "text", "p0-u",
                           text="Stage 0: continue the ORION migration. Never deploy without approval."))
        codes, lines = [], ["STAGE_REPORT_0. Recording my decisions."]
        for i in range(n_facts):
            code = "DEC_" + rng.randbytes(4).hex()
            codes.append(code)
            lines.append(f"DECISION {i:03d} [{code}]: choosing variant {i % 7} because the alternative "
                         f"deadlocks on split frames. Confidence {60 + i % 40}%.")
        records.append(rec("assistant", "text", "p0-a1", text="\n".join(lines)))

        full = shapes.to_messages(records)
        summary = (call(f"c3pressure__n{n_facts}__summary",
                        full + [{"role": "user", "content": SUMMARY_INSTRUCTION.replace(
                            "Summarize the entire conversation above", "Summarize the conversation above")}],
                        32768).get("text") or "").strip()
        if not summary:
            raise RuntimeError("empty summary in pressure test")
        ask = codes[::max(1, n_facts // 10)][:10]
        question = ("Return one JSON object with key `codes`: an array of {\"index\": <int>, \"code\": "
                    "\"<the DEC_ code>\"} for decision indices " +
                    ", ".join(str(codes.index(c)) for c in ask) + ". Use \"UNKNOWN\" if unsure; never guess.")

        # C3 keeps the assistant originals; that is the ingredient under test here.
        kept = [r for r in records if r["kind"] == "text"]
        body = ("<archived-conversation>\n" + shapes._plain(kept) +
                f"\n\n<state-summary>\n{summary}\n</state-summary>\n</archived-conversation>\n" + question)
        parsed = call(f"c3pressure__n{n_facts}__C3", [
            {"role": "system", "content": CLOSED_SYSTEM.replace(
                " No tools or external evidence are available in this condition.", "")},
            {"role": "user", "content": body}], 8192, reuse=False)
        text = parsed.get("text") or ""
        decoder, pos, answer = json.JSONDecoder(), 0, None
        while pos < len(text):
            start = text.find("{", pos)
            if start < 0:
                break
            try:
                val, end = decoder.raw_decode(text[start:])
            except json.JSONDecodeError:
                pos = start + 1
                continue
            if isinstance(val, dict) and "codes" in val:
                answer = val
                break
            pos = start + end
        got = sum(1 for e in (answer or {}).get("codes", [])
                  if str(e.get("code", "")).strip() == codes[int(e["index"])]
                  if isinstance(e.get("index"), (int, str)) and str(e.get("index", "")).isdigit())
        print(json.dumps({"n_facts": n_facts, "asked": len(ask), "exact": got,
                          "summary_tokens": external.estimate_tokens(summary),
                          "assistant_tokens": sum(len(r["text"]) for r in records
                                                  if r["kind"] == "text" and r["role"] == "assistant") // 4}),
              flush=True)

    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
