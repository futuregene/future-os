"""Compression-pressure test: does verbatim retention beat summary-only when the
assistant content is large enough that a summary MUST lose detail?

The previous fixture had 142 tokens of assistant text — trivially summarisable, so
both rules tied. Here the agent's own messages carry N distinct, precise
commitments (a code per decision). The summary has a real budget, so specifics must
be dropped; verbatim retention keeps all of them.

Both projections contain the SAME summary (generated from the full history), so the
only difference is whether the assistant prose is retained beside it.
"""
import argparse, json, pathlib, random, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM
import abc_cache_shapes as shapes
import abc_external_strategies as external

INSTRUCTION = (
    "Summarize the conversation above for another agent that will continue this work. Read all "
    "of it, including every assistant message. STRICT LIMIT: your entire summary must be at most "
    "200 words. It will be truncated if longer, so choose what to keep deliberately. Prefer the "
    "decisions and exact identifiers that matter most. Do not mention compaction.")


def build(n_facts, stage):
    """Assistant prose carrying n_facts distinct decision codes, plus tool state."""
    rng = random.Random(7000 + stage + n_facts)
    records, order = [], 0

    def rec(role, kind, ident, **extra):
        nonlocal order
        order += 1
        return {"id": ident, "role": role, "kind": kind, "order": order, **extra}

    records.append(rec("user", "text", f"p{stage}-u", text=(
        f"Stage {stage}: continue the ORION migration. Never deploy without approval. "
        f"Report only what evidence establishes." + (
            " Correction: the throughput target is now 900 req/s." if stage >= 2 else ""))))

    codes = []
    lines = [f"STAGE_REPORT_{stage}. Recording my decisions for this stage."]
    for i in range(n_facts):
        code = "DEC_" + rng.randbytes(4).hex()
        codes.append(code)
        component = ["parser", "adapter", "cache", "router", "serializer", "scheduler"][i % 6]
        lines.append(
            f"DECISION {i:03d} [{code}]: I am choosing the {component} variant {i % 7} because "
            f"the alternative deadlocks on split frames and would force a rewrite later. "
            f"Confidence {60 + i % 40}%. Recorded so a later agent does not relitigate this.")
    records.append(rec("assistant", "text", f"p{stage}-a1", text="\n".join(lines)))

    version = "ver_" + rng.randbytes(6).hex()
    for i in range(3):
        path = ["config.snapshot", "validation.log", "trace.log"][i]
        call = f"p{stage}-call-{i}"
        header = f"PROJECT=ORION; STAGE={stage}; PATH={path}; diagnostic data.\n"
        if i == 0:
            header += f"VERSION_C{stage:03d}={version}\n"
        if i == 1:
            header += "VALIDATION=PASS_LINUX_ONLY\nBLOCKER=WAIT_USER_APPROVAL\nDEPLOYMENT=NOT_DEPLOYED\n"
        body = header + "\n".join(f"{j:04d} DEBUG counters-only" for j in range(400)) + "\nREAD_STATUS=COMPLETED.\n"
        records.extend([rec("assistant", "tool_call", call, call=call, path=path),
                        rec("tool", "tool_result", f"p{stage}-result-{i}", call=call, path=path,
                            text=body, error=False)])
    return records, codes, version


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--facts", type=int, nargs="*", default=[40, 160, 400])
    ap.add_argument("--budget", type=float, default=6.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "pressure-calls.json"
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

    rows = []
    for n_facts in args.facts:
        records, codes, version = build(n_facts, 0)
        assistant_chars = sum(len(r["text"]) for r in records if r["kind"] == "text" and r["role"] == "assistant")
        full = shapes.to_messages(records)
        summary = (call(f"pressure__n{n_facts}__summary",
                        full + [{"role": "user", "content": INSTRUCTION}], 8192).get("text") or "").strip()
        if not summary:
            raise RuntimeError("empty summary")
        print(json.dumps({"facts": n_facts, "assistant_chars": assistant_chars,
                          "assistant_tokens": assistant_chars // 4,
                          "summary_tokens": external.estimate_tokens(summary),
                          "ratio": round(assistant_chars / 4 / max(external.estimate_tokens(summary), 1), 1)}),
              flush=True)

        # ask for a sampled subset of the decision codes
        sample = codes[::max(1, n_facts // 10)][:10]
        question = (
            "Return one JSON object with a key `codes` containing an array of these decisions as "
            "objects {\"index\": <int>, \"code\": \"<the DEC_ code>\"}. Give the exact code for "
            "decision indices: " + ", ".join(str(codes.index(c)) for c in sample) + ". "
            "Use \"UNKNOWN\" for any you cannot establish exactly; never guess.")

        for rule in ("ours-verbatim", "codex-summary-only"):
            kept = [r for r in records if r["kind"] == "text" and (rule == "ours-verbatim" or r["role"] == "user")]
            body = ("<archived-conversation>\n" + shapes._plain(kept) +
                    f"\n\n<state-summary>\n{summary}\n</state-summary>\n</archived-conversation>\n" + question)
            parsed = call(f"pressure__n{n_facts}__{rule}", [
                {"role": "system", "content": CLOSED_SYSTEM.replace(
                    " No tools or external evidence are available in this condition.", "")},
                {"role": "user", "content": body}], 4096, reuse=False)
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
            got = 0
            for entry in (answer or {}).get("codes", []):
                try:
                    idx = int(entry.get("index"))
                except (TypeError, ValueError):
                    continue
                if 0 <= idx < len(codes) and str(entry.get("code", "")).strip() == codes[idx]:
                    got += 1
            rows.append({"facts": n_facts, "rule": rule, "asked": len(sample), "exact": got,
                         "cost": ledger.rows[-1].get("charged", 0)})
            print(json.dumps({"facts": n_facts, "rule": rule, "exact": f'{got}/{len(sample)}',
                              "cost": round(ledger.rows[-1].get("charged", 0), 4)}), flush=True)

    print(f'\n{"assistant facts":>16s} {"ours-verbatim":>16s} {"codex-summary":>16s}')
    for n_facts in args.facts:
        a = next(r for r in rows if r["facts"] == n_facts and r["rule"] == "ours-verbatim")
        b = next(r for r in rows if r["facts"] == n_facts and r["rule"] == "codex-summary-only")
        left = f'{a["exact"]}/{a["asked"]}'
        right = f'{b["exact"]}/{b["asked"]}'
        print(f'{n_facts:>16d} {left:>16s} {right:>16s}')

    pathlib.Path(args.root / "pressure-results.json").write_text(
        json.dumps(rows, ensure_ascii=False, indent=2) + "\n")
    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
