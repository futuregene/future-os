"""Resolve a contradiction: earlier I measured 99.9% prefix-cache hits, now 0-2%.

Hypothesis to test: the 99.9% earlier was inherited from a *previous identical
request* made minutes earlier in the same session (cache_probe sent that exact
message array three times before shape_prime_test measured it), not from the
prime/summary pair itself.

Test, on one array:
  A) send it once, then send the identical array again  -> does repetition cache?
  B) send it once, then send it with an instruction appended -> does appending cache?
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse
import abc_cache_shapes as shapes


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=6.0)
    ap.add_argument("--token", default="", help="identity token, so repeats under a new name still run")
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "cache-diag-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    data = json.loads((args.root / "data" / "export-0.json").read_text())
    messages = shapes.to_messages(data["archive"] + data["tail"])
    print(f"array: {len(messages)} messages, ~{sum(len(json.dumps(m)) for m in messages)//4} tokens by chars/4", flush=True)

    def send(name, msgs, max_tokens=1024):
        ident = f"diag__{args.token}__{name}"
        body = {"model": args.model, "messages": msgs, "stream": True,
                "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
        row = ledger.reserve(ident, args.model, request_reserve(len(json.dumps(body).encode()), max_tokens))
        began = time.monotonic()
        result = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                capture_output=True, text=True, timeout=900)
        parsed = parse_sse(result.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), cache_hit=usage.get("prompt_cache_hit_tokens"))
        total = usage.get("prompt_tokens") or 0
        hit = usage.get("prompt_cache_hit_tokens") or 0
        print(f'  {name:14s} input={total:>7d} hit={hit:>7d} ({100*hit/max(total,1):5.1f}%) '
              f'CNY {ledger.rows[-1].get("charged", 0):.4f}', flush=True)
        return usage

    print("\nA) same array twice", flush=True)
    send("a1", messages)
    send("a2", messages)

    print("\nB) array, then array + appended instruction", flush=True)
    send("b1", messages)
    send("b2", messages + [{"role": "user", "content": "Summarize the conversation above."}])

    spend = sum(r.get("charged", r["reserved"]) for r in ledger.rows)
    print(f"\ndiag ledger: {len(ledger.rows)} requests, CNY {spend:.4f}")


if __name__ == "__main__":
    main()
