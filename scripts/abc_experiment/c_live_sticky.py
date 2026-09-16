"""Faithful Codex-shaped chaining for C (clean rewrite).

Codex's live history after compaction is [user messages] + [summary], so each new
summary reads the previous one and values accumulate: its stage-4 summary already
lists 4 trace tokens, stage 8 lists 8. The first C+live attempt rebuilt every
summary from C's projection, so its own previous summary was never carried forward
and the stage-0 value vanished by stage 4. This chains properly:

    live(0) = archive + tail                       -> summary_0
    live(N) = [summary_{N-1}] + records since      -> summary_N
    projection(N) = C's projection + summary_N

Between probe stages the records accumulate, because a real session would have
compacted there too; the summary is regenerated only at probe boundaries to keep
the run affordable, which is recorded as a limitation.
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse
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
    ap.add_argument("--no-prime", action="store_true")
    ap.add_argument("--budget", type=float, default=25.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "clivesticky-calls.json"
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

    for task in ("export", "analysis"):
        for model in ("future/deepseek-flash", "future/glm-5.3-flash"):
            if args.models and model not in args.models:
                continue
            slug = model.split("/")[-1]
            previous_summary = None
            since_last = []          # records accumulated between probe boundaries
            for stage in range(8):
                data = json.loads((args.root / "data" / f"{task}-{stage}.json").read_text())
                if stage == 0:
                    since_last = data["archive"] + data["tail"]
                else:
                    since_last = since_last + data["newly_covered"]
                if stage not in PROBE_STAGES:
                    continue

                ident = f"{task}__{slug}__s{stage}__Clivesty"
                out = args.root / "Clivesty" / "projections" / f"{ident}.json"
                out.parent.mkdir(parents=True, exist_ok=True)
                if out.exists():
                    previous_summary = json.loads(out.read_text())["summary"]
                    since_last = []
                    print(json.dumps({"stage": ident, "skipped": "exists"}), flush=True)
                    continue

                if previous_summary is None:
                    live = since_last
                    provenance = "archive (first boundary)"
                else:
                    live = [{"kind": "text", "role": "user", "id": f"prior-summary-{stage}",
                             "text": f"<prior-summary>\n{previous_summary}\n</prior-summary>"}] + since_last
                    provenance = f"prior summary + {len(since_last)} records since"

                messages = shapes.to_messages(live)
                est = len(json.dumps(messages)) // 4
                if not args.no_prime:
                    prime, _ = call(f"clivesty__{ident}__prime", model, messages, 1024)
                    if prime.get("error"):
                        raise RuntimeError(f"{ident}: prime failed: {prime['error'][:200]}")
                attempt = 0
                while True:
                    suffix = "" if attempt == 0 else f"__retry{attempt}"
                    parsed, (total, hit, cost) = call(f"clivesty__{ident}__summary{suffix}", model,
                                                      messages + [{"role": "user", "content": INSTRUCTION}], 32768)
                    if parsed.get("error"):
                        raise RuntimeError(f"{ident}: summary failed: {parsed['error'][:200]}")
                    if parsed["text"].strip():
                        break
                    attempt += 1
                    if attempt > 2:
                        raise RuntimeError(f"{ident}: empty after retries")
                summary = parsed["text"].strip()
                previous_summary = summary
                since_last = []

                base = json.loads((args.root / "C" / "projections" / f"{task}__{slug}__s{stage}__C.json").read_text())["text"]
                value = base.replace("<recent-history>",
                                     f"<state-summary>\n{summary}\n</state-summary>\n\n<recent-history>", 1)
                out.write_text(json.dumps({
                    "id": ident, "text": value, "summary": summary,
                    "context_tokens": external.estimate_tokens(value),
                    "summary_tokens": external.estimate_tokens(summary),
                    "summary_input": provenance, "live_material_est_tokens": est,
                    "summary_request": {"input": total, "cache_hit": hit, "cost": cost},
                    "summary_shape": "sticky chain (Codex-style)",
                }, ensure_ascii=False, indent=2) + "\n")
                print(json.dumps({"stage": ident, "live_est": est, "summary_tokens": external.estimate_tokens(summary),
                                  "ctx": external.estimate_tokens(value), "input": total,
                                  "hit_pct": round(100 * hit / max(total, 1), 1), "cost": round(cost, 5)}), flush=True)

    spend = sum(r.get("charged", r["reserved"]) for r in ledger.rows)
    print(f"\nclivesticky ledger: {len(ledger.rows)} requests, CNY {spend:.4f}")


if __name__ == "__main__":
    main()
