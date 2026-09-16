"""C with Codex's exact schedule: sticky summaries, overflow-triggered compaction.

The previous attempt failed at stage 7 because I compacted only at probe boundaries,
so four stages of records (~1M tokens) accumulated and exceeded the window. That is
precisely why Codex's design works: it compacts whenever the context fills, which is
what keeps the live history bounded in the first place.

This version uses the same trigger constant the Codex arm uses
(`EXTERNAL_LIMIT`, calibrated from the provider's own rejection), so both chains
compact at the same points. The only difference is what a compaction retains:

    Codex : user messages + summary            (assistant text and tool output dropped)
    C     : C's projection + summary            (proof: user text, evidence, tail)

Probe stages force a compaction if the trigger did not fire, so the boundary state
is comparable across arms.
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import (
    Ledger, request_reserve, parse_sse, EXTERNAL_LIMIT)
import abc_cache_shapes as shapes
import abc_external_strategies as external

PROBE_STAGES = (0, 3, 7)
INSTRUCTION = (
    "Summarize the entire conversation above into a handoff summary for another agent that "
    "will continue this work. Read all of it. Use exactly this Markdown structure:\n\n"
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--models", nargs="*", default=None)
    ap.add_argument("--tasks", nargs="*", default=["export", "analysis"])
    ap.add_argument("--budget", type=float, default=12.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "csched-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, model, messages, max_tokens):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior:
            return ({"text": prior["text"], "error": prior.get("error")},
                    (prior.get("input_tokens") or 0, prior.get("cache_hit") or 0, prior.get("charged", 0)))
        body = {"model": model, "messages": messages, "stream": True,
                "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
        row = ledger.reserve(identity, model, request_reserve(len(json.dumps(body).encode()), max_tokens))
        began = time.monotonic()
        result = subprocess.run([str(args.bridge)], input=json.dumps({"model": model, "body": body}),
                                capture_output=True, text=True, timeout=1200)
        parsed = parse_sse(result.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), cache_hit=usage.get("prompt_cache_hit_tokens"),
                      text=parsed["text"], finish=parsed.get("finish"))
        return parsed, (usage.get("prompt_tokens") or 0, usage.get("prompt_cache_hit_tokens") or 0,
                        ledger.rows[-1].get("charged", 0))

    def summarize(model, ident_suffix, live, kind):
        messages = shapes.to_messages(live)
        est = len(json.dumps(messages)) // 4
        attempt = 0
        while True:
            suffix = "" if attempt == 0 else f"__retry{attempt}"
            parsed, (total, hit, cost) = call(f"{ident_suffix}__{kind}{suffix}", model,
                                              messages + [{"role": "user", "content": INSTRUCTION}], 32768)
            if parsed.get("error"):
                raise RuntimeError(f"{ident_suffix}: summary failed: {parsed['error'][:200]}")
            if parsed["text"].strip():
                break
            attempt += 1
            if attempt > 2:
                raise RuntimeError(f"{ident_suffix}: empty after retries")
        return parsed["text"].strip(), est, total, hit, cost

    for task in args.tasks:
        for model in ("future/deepseek-flash", "future/glm-5.3-flash"):
            if args.models and model not in args.models:
                continue
            slug = model.split("/")[-1]
            summary, live = None, []
            for stage in range(8):
                data = json.loads((args.root / "data" / f"{task}-{stage}.json").read_text())
                fresh = data["archive"] + data["tail"] if stage == 0 else data["newly_covered"]
                ident = f"{task}__{slug}__s{stage}__Csched"

                # Codex's trigger: compact before the appended records would overflow.
                def size(items):
                    return len(json.dumps(shapes.to_messages(items))) // 4

                events = []
                while live and size(live) + size(fresh) > EXTERNAL_LIMIT:
                    base = [{"kind": "text", "role": "user", "id": "prior-summary",
                             "text": f"<prior-summary>\n{summary}\n</prior-summary>"}] if summary else []
                    summary, est, total, hit, cost = summarize(model, f"csched__{ident}__auto{len(events)}",
                                                               base + live, "summary")
                    events.append({"stage": stage, "kind": "auto", "live_est": est, "input": total,
                                   "hit": hit, "cost": cost})
                    live = []
                    print(json.dumps({"auto_compact": ident, "live_est": est, "cost": round(cost, 4)}), flush=True)
                live = live + fresh

                if stage not in PROBE_STAGES:
                    continue
                out = args.root / "Csched" / "projections" / f"{ident}.json"
                out.parent.mkdir(parents=True, exist_ok=True)
                if out.exists():
                    summary = json.loads(out.read_text())["summary"]
                    print(json.dumps({"stage": ident, "skipped": "exists"}), flush=True)
                    continue
                if not events:
                    base = [{"kind": "text", "role": "user", "id": "prior-summary",
                             "text": f"<prior-summary>\n{summary}\n</prior-summary>"}] if summary else []
                    summary, est, total, hit, cost = summarize(model, f"csched__{ident}__boundary",
                                                               base + live, "summary")
                    events.append({"stage": stage, "kind": "boundary", "live_est": est, "input": total,
                                   "hit": hit, "cost": cost})
                    live = []

                base_text = json.loads((args.root / "C" / "projections" /
                                        f"{task}__{slug}__s{stage}__C.json").read_text())["text"]
                value = base_text.replace("<recent-history>",
                                          f"<state-summary>\n{summary}\n</state-summary>\n\n<recent-history>", 1)
                out.write_text(json.dumps({
                    "id": ident, "text": value, "summary": summary,
                    "context_tokens": external.estimate_tokens(value),
                    "summary_tokens": external.estimate_tokens(summary),
                    "compactions": events, "summary_shape": "sticky chain, Codex trigger",
                }, ensure_ascii=False, indent=2) + "\n")
                print(json.dumps({"stage": ident, "compactions": [e["kind"] for e in events],
                                  "ctx": external.estimate_tokens(value),
                                  "cost": round(sum(e["cost"] for e in events), 4)}), flush=True)

    spend = sum(r.get("charged", r["reserved"]) for r in ledger.rows)
    print(f"\ncsched ledger: {len(ledger.rows)} requests, CNY {spend:.4f}")


if __name__ == "__main__":
    main()
