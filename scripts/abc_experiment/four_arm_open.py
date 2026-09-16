#!/usr/bin/env python3
"""Secondary common-archive condition on the primary run's exact projections.

Run only after four_arm_rerun.py completes. Reuses its shared cost ledger;
never materializes/runs the historical workspace or starts an agent.
"""
import argparse
import json
import os
from pathlib import Path
import random
import sys

import four_arm_rerun as base

BYTE_BUDGET = 32768
TOOL_BUDGET = 5
TOOLS = [
    {"type": "function", "function": {"name": "archive_search", "description": "Search the covered archive by a literal case-sensitive substring. Returns record IDs and centered snippets. No future records are accessible.", "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}}},
    {"type": "function", "function": {"name": "archive_read", "description": "Read a record by ID, optionally starting at a character offset. Returns a continuation cursor.", "parameters": {"type": "object", "properties": {"id": {"type": "string"}, "offset": {"type": "integer", "minimum": 0}}, "required": ["id"]}}},
]


def bounded_json(value, budget):
    """Refuse rather than return malformed JSON or exceed a UTF-8 budget."""
    text = json.dumps(value, ensure_ascii=False)
    if len(text.encode()) > budget:
        return ""
    return text


def archive_call(records, name, args, budget):
    if budget <= 0:
        return ""
    if name == "archive_search":
        query = args.get("query", "")
        if not isinstance(query, str) or not query:
            return bounded_json({"error": "nonempty query required"}, budget)
        found = []
        for index, record in enumerate(records):
            text = base.ext.plain([record])
            at = text.find(query)
            if at < 0:
                continue
            row = {"id": str(index), "source_id": record.get("id"), "role": record["role"],
                   "offset": max(0, at-160), "snippet": text[max(0,at-160):at+640]}
            if not bounded_json({"matches": found+[row]}, budget):
                break
            found.append(row)
            if len(found) == 8:
                break
        return bounded_json({"matches": found}, budget)
    if name == "archive_read":
        try:
            index = int(args.get("id", ""))
            offset = max(0, int(args.get("offset", 0)))
        except (ValueError, TypeError):
            return bounded_json({"error": "invalid id or offset"}, budget)
        if not 0 <= index < len(records):
            return bounded_json({"error": "unknown record"}, budget)
        text = base.ext.plain([records[index]])
        width = min(8192, max(0,len(text)-offset))
        while width >= 0:
            result = bounded_json({"id": str(index), "content": text[offset:offset+width],
                                   "next_offset": offset+width, "total_chars": len(text)}, budget)
            if result:
                return result
            if width == 0:
                break
            width //= 2
        return ""
    return bounded_json({"error": "unknown tool"}, budget)


def open_probe(calls, identity, projection, question, records):
    messages = [{"role": "system", "content": base.SYSTEM +
                 " You may query the original covered archive using the provided tools. "
                 "You have at most five tool invocations and 32768 returned UTF-8 bytes. "
                 "If the projection does not establish a candidate, search before deciding."},
                {"role": "user", "content": projection["text"] + "\n\n" + question["prompt"]}]
    remaining, executed, requested, turns, trace = BYTE_BUDGET, 0, 0, 0, []
    answer = None
    finish = None
    for step in range(TOOL_BUDGET+1):
        tools = TOOLS if executed < TOOL_BUDGET and remaining > 0 and step < TOOL_BUDGET else None
        p = calls.model_call(f"{identity}-turn{step}", messages, tools=tools)
        turns += 1
        finish = p.get("finish")
        if not p.get("calls"):
            answer = base.parse_answer(p["text"])
            break
        messages.append({"role": "assistant", "content": p["text"] or None, "tool_calls": p["calls"]})
        for call in p["calls"]:
            requested += 1
            function = call["function"]
            try:
                args = json.loads(function["arguments"])
                if not isinstance(args, dict):
                    args = {}
            except (ValueError, TypeError):
                args = {}
            if executed < TOOL_BUDGET and remaining > 0:
                response = archive_call(records, function["name"], args, remaining)
                executed += 1
            else:
                response = ""
            used = len(response.encode())
            assert used <= remaining
            remaining -= used
            trace.append({"name": function["name"], "args": args, "bytes": used})
            messages.append({"role": "tool", "tool_call_id": call["id"], "content": response})
    return {**base.exam.score(answer, dict.fromkeys(question["present"]), dict.fromkeys(question["decoys"])),
            "valid_answer": answer is not None, "answer": answer, "finish": finish,
            "projection_sha256": base.sha(projection["text"]), "question_sha256": base.sha(question),
            "model_turns": turns, "tool_invocations": executed, "tool_calls_requested": requested,
            "returned_bytes": BYTE_BUDGET-remaining, "trace": trace}


def aggregate(root):
    config = base.load(root/"manifest.json")
    rows = [base.load(p) for p in (root/"open-scores").glob("*.json")]
    result = {"complete": len(rows) == len(config["chains"])*12, "n": len(rows), "arms": {}}
    for arm in base.ARMS:
        xs = [x for x in rows if x["arm"] == arm]
        result["arms"][arm] = {k:sum(x[k] for x in xs) for k in
                               ("hits", "of_present", "false_positives", "model_turns", "tool_invocations", "returned_bytes")}
    base.save(root/"open-report.json", result)
    print(json.dumps(result, indent=2), flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--output", required=True, type=Path)
    ap.add_argument("--bridge", required=True, type=Path)
    ap.add_argument("--report-only", action="store_true")
    args = ap.parse_args()
    root = args.output.resolve()
    if args.report_only:
        aggregate(root)
        return
    config = base.load(root/"manifest.json")
    if not base.load(root/"report.json")["complete"]:
        raise RuntimeError("primary phase incomplete")
    if base.sha(args.bridge.read_bytes()) != config["bridge_sha256"]:
        raise RuntimeError("bridge changed")
    lock = root/"runner.lock"
    fd = os.open(lock, os.O_CREAT|os.O_EXCL|os.O_WRONLY, 0o600)
    os.write(fd, str(os.getpid()).encode())
    os.close(fd)
    try:
        protocol = {"base": base.sha(config), "tools": TOOLS, "tool_budget": TOOL_BUDGET,
                    "byte_budget": BYTE_BUDGET, "source_sha256": base.sha(Path(__file__).read_bytes()),
                    "runner_sha256": base.sha(Path(base.__file__).read_bytes())}
        base.immutable(root/"open-manifest.json", protocol)
        calls = base.Calls(root, config["budget"], config["model"], args.bridge.resolve(), base.sha(protocol))
        schedule = base.load(root/"schedule.json")
        rng = random.Random(base.SEED+1)
        for chain in config["chains"]:
            d = base.load(root/"corpus"/f"{chain}.json")
            for stage, cut in enumerate(d["cuts"]):
                step = schedule[chain].index(cut)
                q = base.load(root/"questions"/f"{chain}-{stage}.json")
                arms = list(base.ARMS)
                rng.shuffle(arms)
                for arm in arms:
                    p = base.load(root/"projections"/f"{chain}-{step}-{arm}-compact.json")
                    closed = base.load(root/"scores"/f"{chain}-{stage}-{arm}-closed.json")
                    if closed["projection_sha256"] != base.sha(p["text"]):
                        raise ValueError("closed/open projection mismatch")
                    identity = f"{chain}-{stage}-{arm}-open"
                    path = root/"open-scores"/f"{identity}.json"
                    if not path.exists():
                        result = open_probe(calls, identity, p, q, d["records"][:cut])
                        base.immutable(path, dict(result, chain=chain, stage=stage, arm=arm))
                aggregate(root)
        aggregate(root)
    finally:
        lock.unlink()


if __name__ == "__main__":
    main()
