"""Isolate whether the provider caches a large prefix at all.

The first run was ambiguous: the 258K conversation reported cache_hit=0 on a
repeat, yet its billed cost (¥0.27) was far below a full miss (¥0.67 for the
first call). This test sends the *identical* message array three times so the
only variable is a warm versus cold cache, and prints the full usage object the
provider returns.
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse
from cache_test import to_messages, INSTRUCTION


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, required=True)
    parser.add_argument("--bridge", type=pathlib.Path, required=True)
    parser.add_argument("--model", default="future/deepseek-flash")
    parser.add_argument("--stage", type=int, default=0)
    parser.add_argument("--budget", type=float, default=8.0)
    args = parser.parse_args()

    data = json.loads((args.root / "data" / f"export-{args.stage}.json").read_text())
    messages = to_messages(data["archive"] + data["tail"])
    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "cache-test-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def send(identity, msgs):
        body = {"model": args.model, "messages": msgs, "stream": True, "max_tokens": 2048,
                "stream_options": {"include_usage": True}}
        row = ledger.reserve(identity, args.model, request_reserve(len(json.dumps(body).encode()), 2048))
        began = time.monotonic()
        result = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                capture_output=True, text=True, timeout=900)
        parsed = parse_sse(result.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), cache_hit=usage.get("prompt_cache_hit_tokens"),
                      usage_raw=usage)
        return usage, parsed

    print("A: identical conversation, sent three times\n", flush=True)
    for label in ("a1", "a2", "a3"):
        usage, _ = send(f"cachebig__{label}", messages)
        total = usage.get("prompt_tokens") or 0
        hit = usage.get("prompt_cache_hit_tokens") or 0
        miss = usage.get("prompt_cache_miss_tokens")
        cost = (ledger.rows[-1].get("charged"))
        print(f'  {label}: input={total} hit={hit} miss={miss} ({100*hit/max(total,1):5.1f}%) CNY {cost:.4f}', flush=True)

    print("\nB: same conversation plus an instruction at the tail\n", flush=True)
    usage, _ = send("cachebig__tail", messages + [{"role": "user", "content": INSTRUCTION}])
    total = usage.get("prompt_tokens") or 0
    hit = usage.get("prompt_cache_hit_tokens") or 0
    print(f'  tail: input={total} hit={hit} ({100*hit/max(total,1):5.1f}%) CNY {ledger.rows[-1].get("charged"):.4f}', flush=True)
    print("\n  full usage:", json.dumps(usage, ensure_ascii=False), flush=True)

    spend = sum(r.get("charged", r["reserved"]) for r in ledger.rows)
    print(f"\nledger: {len(ledger.rows)} requests, CNY {spend:.4f}")


if __name__ == "__main__":
    main()
