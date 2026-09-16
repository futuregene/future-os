"""Model production: the conversation was already sent as normal turns.

A compaction request reuses a prefix that the session has already paid for. The
earlier `a_shape_test.py` sent each prefix once, so everything was cold - which is
not what happens in a real session. Here each stage is sent twice: once as the
ordinary turn (priming) and once with the summarization instruction appended.

Two shapes are compared at the same stages and on the same history:
  flattened  - A's original shape: one user message containing the clipped material
  preserved  - the conversation as real messages, instruction at the tail
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse
import abc_cache_shapes as shapes
import abc_external_strategies as external

INSTRUCTION = ("Summarize the conversation above as a handoff summary so another agent can "
               "continue: current progress and key decisions, important constraints, what "
               "remains, and critical data needed to continue. Be concise and structured.")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--task", default="export")
    ap.add_argument("--stages", type=int, nargs="*", default=[0, 3])
    ap.add_argument("--budget", type=float, default=8.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "shape-prime-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def send(identity, messages, max_tokens=1024):
        body = {"model": args.model, "messages": messages, "stream": True,
                "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
        row = ledger.reserve(identity, args.model, request_reserve(len(json.dumps(body).encode()), max_tokens))
        began = time.monotonic()
        result = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                capture_output=True, text=True, timeout=900)
        parsed = parse_sse(result.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), cache_hit=usage.get("prompt_cache_hit_tokens"))
        return usage

    rows = []
    for stage in args.stages:
        data = json.loads((args.root / "data" / f"{args.task}-{stage}.json").read_text())
        records = data["archive"] + data["tail"]

        # ---- preserved prefix: prime with the plain conversation, then summarize
        conv = shapes.to_messages(records)
        send(f"prime__conv_s{stage}", conv, 64)                      # the ordinary turn
        usage = send(f"summ__preserved_s{stage}", conv + [{"role": "user", "content": INSTRUCTION}])
        total = usage.get("prompt_tokens") or 0
        hit = usage.get("prompt_cache_hit_tokens") or 0
        cost = ledger.rows[-1].get("charged", 0)
        print(f'  preserved s{stage}: input={total:>7d} hit={hit:>7d} ({100*hit/max(total,1):5.1f}%) CNY {cost:.4f}', flush=True)
        rows.append({"shape": "preserved", "stage": stage, "input": total, "hit": hit, "cost": cost})

        # ---- flattened: A's original shape (one user message), primed the same way
        flat = [{"role": "user", "content": external.plain(records)[:200_000]}]
        send(f"prime__flat_s{stage}", flat, 64)
        usage = send(f"summ__flat_s{stage}", flat + [{"role": "user", "content": INSTRUCTION}])
        total = usage.get("prompt_tokens") or 0
        hit = usage.get("prompt_cache_hit_tokens") or 0
        cost = ledger.rows[-1].get("charged", 0)
        print(f'  flattened s{stage}: input={total:>7d} hit={hit:>7d} ({100*hit/max(total,1):5.1f}%) CNY {cost:.4f}', flush=True)
        rows.append({"shape": "flattened", "stage": stage, "input": total, "hit": hit, "cost": cost})

    print("\n" + json.dumps(rows))
    print(f'ledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
