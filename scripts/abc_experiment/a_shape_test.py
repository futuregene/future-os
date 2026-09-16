"""Measure the A-style prefix-preserving summary shape across consecutive stages.

A is supposed to be cheap because it reads only the clipped material. That is
true per token, but it re-serialises the material every stage, so the prefix never
repeats and nothing is cached; measured on the 33 real A summaries the hit rate was
2.8%. Here the two shapes are compared at stage 1 and then at stage 3, where the
stage-1 blocks would still be a valid prefix.
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
    ap.add_argument("--task", default="export")
    ap.add_argument("--budget", type=float, default=6.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "shape-test-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def send(identity, messages):
        body = {"model": args.model, "messages": messages, "stream": True, "max_tokens": 2048,
                "stream_options": {"include_usage": True}}
        row = ledger.reserve(identity, args.model, request_reserve(len(json.dumps(body).encode()), 2048))
        began = time.monotonic()
        result = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                capture_output=True, text=True, timeout=900)
        parsed = parse_sse(result.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), cache_hit=usage.get("prompt_cache_hit_tokens"))
        return usage

    collected = []
    for stage in (0, 3):
        data = json.loads((args.root / "data" / f"{args.task}-{stage}.json").read_text())
        prior = json.loads((args.root / "data" / f"{args.task}-{max(stage - 1, 0)}.json").read_text())
        records = data["protected"] + data["newly_covered"]
        msgs = shapes.a_style_messages(records, "Summarize the conversation above as a handoff summary.",
                                       protected=data["protected"][:4],
                                       previous_summary="STAGE-PLACEHOLDER-SUMMARY" if stage else None)
        usage = send(f"shape__a_s{stage}", msgs)
        total = usage.get("prompt_tokens") or 0
        hit = usage.get("prompt_cache_hit_tokens") or 0
        collected.append({"stage": stage, "input": total, "hit": hit})
        print(f'  stage {stage}: input={total} hit={hit} ({100*hit/max(total,1):5.1f}%) '
              f'CNY {ledger.rows[-1].get("charged", 0):.4f}', flush=True)

    print(json.dumps(collected))
    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
