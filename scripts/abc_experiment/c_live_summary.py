"""C with a Codex-style summary input: the whole LIVE context, unclipped, bounded.

Codex's "full history" is not the archive — it is the *live* conversation, which
stays bounded only because compaction replaces it destructively. Reproducing that
shape for C means summarising:

    previous stage's C projection  +  everything recorded since

That is ~250-270K tokens at every stage (one stage's worth of records on top of a
bounded projection), instead of the 255K-2080K the raw archive grows to. It reads
everything the agent currently holds, without clipping, and it is bounded.

Generated as prime + append so the prefix repeats:

    prime : the live material as real chat messages
    ask   : the identical array with the instruction appended   -> warm prefix
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
    "Rules: keep every section; terse bullets; preserve exact paths, symbols, numeric values, "
    "versions and error strings; state explicitly what is NOT established rather than inferring "
    "it; when a value was superseded, give the current one and note it was superseded; "
    "do not mention compaction.")


def live_material(root, task, stage, model_slug):
    """Previous projection + everything since -- Codex's live-context shape."""
    if stage == 0:
        data = json.loads((root / "data" / f"{task}-0.json").read_text())
        return data["archive"] + data["tail"], "archive (first boundary)"
    prior = json.loads((root / "C" / "projections" /
                        f"{task}__{model_slug}__s{stage - 1}__C.json").read_text())["text"]
    data = json.loads((root / "data" / f"{task}-{stage}.json").read_text())
    return [{"kind": "text", "role": "user", "text": prior,
             "id": f"prior-projection-{stage}"}] + data["newly_covered"], "prior projection + new records"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--models", nargs="*", default=None)
    ap.add_argument("--no-prime", action="store_true")
    ap.add_argument("--budget", type=float, default=20.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "cfulllive-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, model, messages, max_tokens):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior:
            return ({"text": prior["text"], "error": prior.get("error"), "finish": prior.get("finish")},
                    (prior.get("input_tokens") or 0, prior.get("cache_hit") or 0, prior.get("charged", 0)))
        body = {"model": model, "messages": messages, "stream": True,
                "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
        row = ledger.reserve(identity, model, request_reserve(len(json.dumps(body).encode()), max_tokens))
        began = time.monotonic()
        result = subprocess.run([str(args.bridge)], input=json.dumps({"model": model, "body": body}),
                                capture_output=True, text=True, timeout=900)
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
            for stage in PROBE_STAGES:
                ident = f"{task}__{slug}__s{stage}__Clive"
                out = args.root / "Clive" / "projections" / f"{ident}.json"
                out.parent.mkdir(parents=True, exist_ok=True)
                if out.exists():
                    print(json.dumps({"stage": ident, "skipped": "exists"}), flush=True)
                    continue
                material, provenance = live_material(args.root, task, stage, slug)
                messages = shapes.to_messages(material)
                est = len(json.dumps(messages)) // 4

                if not args.no_prime:
                    prime, _ = call(f"clive__{ident}__prime", model, messages, 1024)
                    if prime.get("error"):
                        raise RuntimeError(f"{ident}: prime failed: {prime['error'][:200]}")

                attempt = 0
                while True:
                    suffix = "" if attempt == 0 else f"__retry{attempt}"
                    parsed, (total, hit, cost) = call(f"clive__{ident}__summary{suffix}", model,
                                                      messages + [{"role": "user", "content": INSTRUCTION}], 32768)
                    if parsed.get("error"):
                        raise RuntimeError(f"{ident}: summary failed: {parsed['error'][:200]}")
                    if parsed["text"].strip():
                        break
                    attempt += 1
                    if attempt > 2:
                        raise RuntimeError(f"{ident}: empty after retries")
                summary = parsed["text"].strip()

                base = json.loads((args.root / "C" / "projections" /
                                   f"{task}__{slug}__s{stage}__C.json").read_text())["text"]
                value = base.replace("<recent-history>",
                                     f'<state-summary>\n{summary}\n</state-summary>\n\n<recent-history>', 1)
                out.write_text(json.dumps({
                    "id": ident, "text": value, "summary": summary,
                    "context_tokens": external.estimate_tokens(value),
                    "summary_tokens": external.estimate_tokens(summary),
                    "summary_input": provenance, "live_material_est_tokens": est,
                    "summary_request": {"input": total, "cache_hit": hit, "cost": cost},
                }, ensure_ascii=False, indent=2) + "\n")
                print(json.dumps({"stage": ident, "live_est": est, "summary_tokens": external.estimate_tokens(summary),
                                  "ctx": external.estimate_tokens(value), "input": total, "hit_pct":
                                  round(100 * hit / max(total, 1), 1), "cost": round(cost, 5)}), flush=True)

    spend = sum(r.get("charged", r["reserved"]) for r in ledger.rows)
    print(f"\nclive ledger: {len(ledger.rows)} requests, CNY {spend:.4f}")


if __name__ == "__main__":
    main()
