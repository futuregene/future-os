"""Uncapped probes on a small sample, to measure how many rounds are really needed.

Runs a few probes with the 5-call cap removed and the corrected Codex contract
(cursor + selectable snippet placement). The goal is a defensible estimate for a
full uncapped re-run, not a result: the sample is deliberately tiny.
"""
import argparse, json, pathlib, subprocess, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM, QUESTION
import abc_retrieval as R


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--binary", type=pathlib.Path, required=True)
    ap.add_argument("--arm", default="C")
    ap.add_argument("--mode", default="codex")
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--task", default="export")
    ap.add_argument("--stage", type=int, default=3)
    ap.add_argument("--budget", type=float, default=6.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "uncapped-test-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    ident = f'{args.task}__{args.model.split("/")[-1]}__s{args.stage}__{args.arm}'
    projection = json.loads((args.root / args.arm / "projections" / f"{ident}.json").read_text())
    data = json.loads((args.root / "data" / f"{args.task}-{args.stage}.json").read_text())
    sid = data["source_session"]
    windows = R.codex_windows(args.root, args.task, args.stage)

    system = (CLOSED_SYSTEM.replace(" No tools or external evidence are available in this condition.", "")
              + R.guide(args.mode, session_id=sid))
    tools = R.codex_tools() if args.mode == "codex" else R.our_tools(sid)
    messages = [{"role": "user", "content": f'<archived-conversation>\n{projection["text"]}\n</archived-conversation>\n'
                                            + QUESTION.format(stage=args.stage)}]

    returned, rounds = 0, 0
    for step in range(R.RUNAWAY_GUARD):
        body = {"model": args.model, "messages": [{"role": "system", "content": system}] + messages,
                "tools": tools, "stream": True, "max_tokens": 8192,
                "thinking": {"type": "enabled"}, "reasoning_effort": "high",
                "stream_options": {"include_usage": True}}
        row = ledger.reserve(f"uncapped__{args.mode}__{step}", args.model,
                             request_reserve(len(json.dumps(body).encode())))
        result = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                capture_output=True, text=True, timeout=600)
        parsed = parse_sse(result.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), error=parsed.get("error"))
        rounds += 1
        if parsed.get("error"):
            print(f"  error: {parsed['error'][:200]}")
            break
        if not parsed["calls"]:
            answer = parsed["text"]
            print(f"  finished after {rounds} rounds, {returned} bytes retrieved; "
                  f"answer chars={len(answer)}")
            break
        assistant = {"role": "assistant", "content": parsed["text"] or None, "tool_calls": parsed["calls"]}
        messages.append(assistant)
        for call in parsed["calls"]:
            name = call["function"]["name"]
            arguments = json.loads(call["function"]["arguments"] or "{}")
            room = max(0, R.RETRIEVAL_BYTES - returned)
            if args.mode == "codex":
                out, size = R.codex_dispatch(name, arguments, windows, room)
            else:
                out, size = R.our_dispatch(arguments["command"], sid, args.binary,
                                           __import__("os").environ.copy(), args.root)
            returned += min(size, room)
            print(f'    r{rounds} {name:26s} {json.dumps(arguments, ensure_ascii=False)[:70]:72s} -> {size}')
            messages.append({"role": "tool", "tool_call_id": call["id"], "content": out[:room]})
    else:
        print(f"  RUNAWAY GUARD tripped at {rounds} rounds")

    print(f'ledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
