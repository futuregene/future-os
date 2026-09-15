"""Measure whether "original context + instruction at the tail" hits the prefix cache.

The hypothesis: a compaction request that reuses the live conversation as its
prefix (Codex's shape) should hit the provider's prefix cache, while our A-style
request — which clips the material and re-serialises it — should not. If true, a
summary can be both informed by the whole history *and* cheap.

Design (one chain, one stage, two requests each):
  prime    : the conversation as a real message array            (fills the cache)
  tail     : the same array + a summarisation instruction appended
  clipped  : an A-style clipped excerpt, sent twice

Usage is read from the provider's own `prompt_cache_hit_tokens`.
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse

INSTRUCTION = ("Summarize the conversation above into a handoff summary: current progress and key "
               "decisions, important context and constraints, what remains to be done, and any "
               "critical data needed to continue. Be concise and structured.")


def to_messages(records):
    """The conversation as a real chat message array (what an agent actually sends)."""
    messages = []
    for record in records:
        if record["kind"] == "tool_call":
            messages.append({"role": "assistant", "content": None,
                             "tool_calls": [{"id": record["call"], "type": "function",
                                             "function": {"name": "read",
                                                          "arguments": json.dumps({"path": record["path"]})}}]})
        elif record["kind"] == "tool_result":
            messages.append({"role": "tool", "tool_call_id": record["call"], "content": record["text"]})
        else:
            messages.append({"role": record["role"], "content": record["text"]})
    return messages


def clipped_excerpt(records, char_budget=60_000):
    """A-style: flatten to text and cut, which changes every byte of the prefix."""
    import abc_external_strategies as external
    text = external.plain(records)
    return text[:char_budget]


def call(args, ledger, identity, model, messages):
    body = {"model": model, "messages": messages, "stream": True, "max_tokens": 4096,
            "stream_options": {"include_usage": True}}
    row = ledger.reserve(identity, model, request_reserve(len(json.dumps(body).encode()), 4096))
    began = time.monotonic()
    result = subprocess.run([str(args.bridge)], input=json.dumps({"model": model, "body": body}),
                            capture_output=True, text=True, timeout=600)
    parsed = parse_sse(result.stdout)
    usage = parsed.get("usage") or {}
    ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                  credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                  error=parsed.get("error"), cache_hit=usage.get("prompt_cache_hit_tokens"))
    return usage, parsed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, required=True)
    parser.add_argument("--bridge", type=pathlib.Path, required=True)
    parser.add_argument("--model", default="future/deepseek-flash")
    parser.add_argument("--task", default="export")
    parser.add_argument("--stage", type=int, default=0)
    parser.add_argument("--budget", type=float, default=6.0)
    args = parser.parse_args()

    data = json.loads((args.root / "data" / f"{args.task}-{args.stage}.json").read_text())
    records = data["archive"] + data["tail"]
    args.ledger = args.root / "cache-test-calls.json"
    ledger = Ledger(args.root, args.budget)
    ledger.path = args.ledger
    ledger.rows = json.loads(args.ledger.read_text()) if args.ledger.exists() else []

    messages = to_messages(records)
    print(f"conversation: {len(records)} records -> {len(messages)} messages", flush=True)

    print("\n[1] prime the cache with the plain conversation", flush=True)
    usage, _ = call(args, ledger, "cache__prime", args.model, messages)
    print(f'    input={usage.get("prompt_tokens")} cache_hit={usage.get("prompt_cache_hit_tokens")}', flush=True)

    print("\n[2] same conversation + instruction at the tail", flush=True)
    usage, _ = call(args, ledger, "cache__tail", args.model, messages + [{"role": "user", "content": INSTRUCTION}])
    hit = usage.get("prompt_cache_hit_tokens") or 0
    total = usage.get("prompt_tokens") or 1
    print(f'    input={total} cache_hit={hit} ({100 * hit / total:.1f}%)', flush=True)

    print("\n[3] an A-style clipped excerpt (different bytes in the prefix)", flush=True)
    excerpt = clipped_excerpt(records)
    clipped = [{"role": "user", "content": excerpt}]
    call(args, ledger, "cache__clipped_prime", args.model, clipped)
    usage, _ = call(args, ledger, "cache__clipped_again", args.model, clipped + [{"role": "user", "content": INSTRUCTION}])
    hit2 = usage.get("prompt_cache_hit_tokens") or 0
    total2 = usage.get("prompt_tokens") or 1
    print(f'    input={total2} cache_hit={hit2} ({100 * hit2 / total2:.1f}%)', flush=True)

    spend = sum(r.get("charged", r["reserved"]) for r in ledger.rows)
    print(f"\nledger: {len(ledger.rows)} requests, CNY {spend:.4f}")


if __name__ == "__main__":
    main()
