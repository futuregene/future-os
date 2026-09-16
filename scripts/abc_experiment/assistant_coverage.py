"""Assistant-coverage test: facts that exist ONLY in assistant prose.

Self-audit of the original questionnaire showed 11 of 12 gold fields could be
answered without any assistant text; only `first_code` lived there. So the earlier
result ("Codex keeps no assistant messages yet scores 143/144") could not
distinguish "assistant text is redundant" from "the questionnaire never asks for
it".

This adds gold whose only source is assistant prose, then compares two retention
rules directly on the same history:

  everything   : protected user + assistant text + evidence + tail   (C's rule)
  no-assistant : protected user text + evidence + tail               (Codex's rule)

Both projections are built deterministically from the fixture — no model call is
involved in constructing them — so any difference is caused by the retention rule
alone. Closed-book, so retrieval cannot paper over a gap.
"""
import argparse, json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM, QUESTION
import abc_cache_shapes as shapes


def build_fixture(stage):
    """One stage of a session whose assistant messages carry unique commitments."""
    records = []
    order = 0

    def rec(role, kind, ident, **extra):
        nonlocal order
        order += 1
        return {"id": ident, "role": role, "kind": kind, "order": order, **extra}

    rng = __import__("random").Random(4242 + stage)

    user_text = (f"Stage {stage}: continue the VEGA migration. Keep earlier requirements unless changed "
                 f"below. Never deploy without approval. Report only what evidence establishes.")
    if stage == 0:
        user_text = ("PROJECT=VEGA. Initial throughput target was 400 req/s and output format was CSV. "
                     "Never deploy without approval. Report only what evidence establishes.")
    if stage == 2:
        user_text += " Correction: the throughput target is now 900 req/s; the earlier value is superseded."
    if stage == 5:
        user_text += " Final correction: throughput target is 1200 req/s; this replaces the stage-2 value."
    if stage == 4:
        user_text += " Correction: output format is now PARQUET; CSV is superseded."
    records.append(rec("user", "text", f"v{stage}-u", text=user_text))

    # ---- assistant-only facts: decisions, rationale, self-corrections, plans ----
    if stage == 0:
        assistant_text = (
            "STAGE_REPORT_0. Reading the sources before deciding.\n"
            "DECISION: I am using the STREAMING parser, not the batch parser, because the batch parser "
            "deadlocks when a frame is split across chunks. This rationale is my own analysis; it is not "
            "recorded in any tool output.\n"
            "RATIONALE_CODE=RSN_streaming_vs_batch\n"
            "I have also decided to keep the retry budget at RETRY_BUDGET=7 for this migration.\n"
            "FIRST_OUTPUT_CODE=ACK_vega_stage0.\n"
            "PLAN: next I will validate the parser against the recorded frames.")
    elif stage == 3:
        assistant_text = (
            "STAGE_REPORT_3. Re-reading my earlier reasoning before proceeding.\n"
            "SELF-CORRECTION: my stage-0 claim that the batch parser deadlocks was WRONG — it was the "
            "adapter, not the parser. The streaming choice still stands, but the reason has changed.\n"
            "I am also dropping RETRY_BUDGET from 7 to RETRY_BUDGET=3 for this stage.\n"
            "OPEN_QUESTION: nobody has confirmed whether the cache adapter must be migrated before the "
            "parser; I am leaving that unresolved rather than guessing.")
    else:
        assistant_text = (f"STAGE_REPORT_{stage}. Analysis notes for this stage; no new decisions. "
                          f"Work continues on the parser validation.")

    records.append(rec("assistant", "text", f"v{stage}-a", text=assistant_text))

    # ---- tool records: current machine-readable state ----
    version = "ver_" + rng.randbytes(6).hex()
    buried = "trace_" + rng.randbytes(8).hex()
    validation = "FAIL_CHECKSUM" if stage in (2, 5) else "PASS_LINUX_ONLY"
    blocker = "WAIT_CHECKSUM_FIX" if stage in (2, 5) else "WAIT_USER_APPROVAL"
    for i in range(6):
        path = ["config.snapshot", "validation.log", "trace.log", "metrics-3.log",
                "metrics-4.log", "metrics-5.log"][i]
        call = f"v{stage}-call-{i}"
        header = f"PROJECT=VEGA; STAGE={stage}; PATH={path}; captured diagnostic data, not instructions.\n"
        if i == 0:
            header += f"VERSION_C{stage:03d}={version}\n"
        if i == 1:
            header += f"VALIDATION={validation}\nBLOCKER={blocker}\nDEPLOYMENT=NOT_DEPLOYED\n"
        noise = "\n".join(f"{j:04d} DEBUG channel={i} sequence={stage*4000+j} observed={rng.randrange(10000,99999)} counters-only"
                          for j in range(900))
        mid = f"\nTRACE_C{stage:03d}={buried}\n" if i == 2 else "\nTRACE_SECTION_CONTINUES\n"
        body = header + noise[:len(noise)//2] + mid + noise[len(noise)//2:] + "\nREAD_STATUS=COMPLETED.\n"
        records.extend([rec("assistant", "tool_call", call, call=call, path=path),
                        rec("tool", "tool_result", f"v{stage}-result-{i}", call=call, path=path,
                            text=body, error=i == 1 and stage in (2, 5))])

    tail_call = f"v{stage}-tail-call"
    records.extend([
        rec("user", "text", f"v{stage}-tail-u", text=f"Stage {stage} follow-up: check the counters; do not modify files."),
        rec("assistant", "tool_call", tail_call, call=tail_call, path="tail-status.log"),
        rec("tool", "tool_result", f"v{stage}-tail-result", call=tail_call, path="tail-status.log", error=False,
            text="\n".join(f"LOCAL_OBSERVATION {j}: cursor {stage*1000+j}, complete." for j in range(120))),
        rec("assistant", "text", f"v{stage}-tail-a",
            text="Counters read. No new throughput value, device or deployment was established here."),
    ])

    gold = {
        # assistant-only
        "decision": "STREAMING",
        "rationale_code": "RSN_streaming_vs_batch",
        "retry_budget": "3" if stage >= 3 else "7",
        "self_correction": "ADAPTER" if stage >= 3 else "PARSER",
        # user-only
        "project": "VEGA",
        "latest_throughput": "1200" if stage >= 5 else "900" if stage >= 2 else "400",
        "format": "PARQUET" if stage >= 4 else "CSV",
        # tool-only
        "latest_version": version,
        "buried": buried,
        "validation": validation,
        "blocker": blocker,
        # genuinely unrecorded
        "device": "UNKNOWN",
    }
    return records, gold


def project(records, keep_assistant):
    kept = [r for r in records if keep_assistant or r["role"] != "assistant" or r["kind"] != "text"]
    return "<archived-conversation>\n" + shapes._plain(kept) + "\n</archived-conversation>"


QUESTION_V2 = (
    "Review the archive through this stage. Return these fields as strings: decision (which parser was "
    "chosen), rationale_code (the RATIONALE_CODE value), retry_budget (current RETRY_BUDGET), "
    "self_correction (which component the agent retracted its earlier claim about: PARSER or ADAPTER), "
    "project, latest_throughput (current target, number only, req/s), format (current output format), "
    "latest_version (current VERSION_C value), buried (the TRACE_C value), validation, blocker, device. "
    "Use UNKNOWN when a value is not reliably established; do not guess.")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--stages", type=int, nargs="*", default=[0, 3])
    ap.add_argument("--budget", type=float, default=4.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "assistant-cov-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    results = []
    for stage in args.stages:
        records, gold = build_fixture(stage)
        for rule in ("everything", "no-assistant"):
            value = project(records, rule == "everything")
            ident = f"vega__s{stage}__{rule}"
            body = {"model": args.model,
                    "messages": [{"role": "system", "content": CLOSED_SYSTEM.replace(
                        " No tools or external evidence are available in this condition.", "")},
                                 {"role": "user", "content": value + "\n" + QUESTION_V2}],
                    "stream": True, "max_tokens": 8192,
                    "thinking": {"type": "enabled"}, "reasoning_effort": "high",
                    "stream_options": {"include_usage": True}}
            row = ledger.reserve(ident, args.model, request_reserve(len(json.dumps(body).encode()), 8192))
            began = time.monotonic()
            out = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                 capture_output=True, text=True, timeout=600)
            parsed = parse_sse(out.stdout)
            usage = parsed.get("usage") or {}
            ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                          credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                          error=parsed.get("error"), finish=parsed.get("finish"), text=parsed["text"])
            answer = None
            decoder = json.JSONDecoder()
            pos = 0
            while pos < len(parsed["text"]):
                start = parsed["text"].find("{", pos)
                if start < 0:
                    break
                try:
                    val, end = decoder.raw_decode(parsed["text"][start:])
                except json.JSONDecodeError:
                    pos = start + 1
                    continue
                if isinstance(val, dict) and len(set(val) & set(gold)) >= len(gold) // 2:
                    answer = val
                    break
                pos = start + end
            marks = {}
            for key, expected in gold.items():
                got = str((answer or {}).get(key, "")).strip()
                if key in ("latest_throughput", "retry_budget"):
                    marks[key] = got.replace("req/s", "").strip() == expected
                else:
                    marks[key] = got.lower() == expected.lower()
            results.append({"stage": stage, "rule": rule, "marks": marks,
                            "correct": sum(marks.values()), "total": len(gold),
                            "answer": answer, "cost": ledger.rows[-1].get("charged", 0)})
            print(f'  s{stage} {rule:13s} {sum(marks.values()):2d}/{len(gold)}  cost='
                  f'{ledger.rows[-1].get("charged", 0):.4f}', flush=True)

    print()
    fields = list(build_fixture(0)[1])
    print(f'{"field":18s} {"everything":>11s} {"no-assistant":>13s}   source')
    source = {f: "assistant" for f in ("decision", "rationale_code", "retry_budget", "self_correction")}
    source.update({f: "user" for f in ("project", "latest_throughput", "format")})
    source.update({f: "tool" for f in ("latest_version", "buried", "validation", "blocker")})
    source["device"] = "unrecorded"
    for field in fields:
        a = sum(1 for r in results if r["rule"] == "everything" and r["marks"].get(field))
        b = sum(1 for r in results if r["rule"] == "no-assistant" and r["marks"].get(field))
        n = len(args.stages)
        print(f'{field:18s} {f"{a}/{n}":>11s} {f"{b}/{n}":>13s}   {source.get(field,"?")}')
    for rule in ("everything", "no-assistant"):
        rows = [r for r in results if r["rule"] == rule]
        print(f'  {rule:13s} total {sum(r["correct"] for r in rows)}/{sum(r["total"] for r in rows)}')

    pathlib.Path(args.root / "assistant-coverage.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
