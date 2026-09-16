"""Realistic evaluation: 3 synthetic chains + 3 real sessions.

Exam weighting follows the measured distribution of real follow-up turns (~80%
reference the agent's own output). Two retention rules are compared with the SAME
sticky summary, so the only difference is what each rule retains:

  C3           protected user AND assistant originals + tool evidence + tail + summary
  summary-only user messages + summary        (Codex's retention rule)

Real sessions are read from the local Agent database, used only in the git-ignored
research directory, and never written into the repository.
"""
import argparse, json, pathlib, random, sqlite3, subprocess, sys, time, re

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM
import abc_cache_shapes as shapes
import abc_external_strategies as external
import realistic_exam as exam

STAGES = (0, 3, 7)
SUMMARY_INSTRUCTION = (
    "Summarize the conversation above into a handoff summary for another agent that will "
    "continue this work. Read all of it. Keep: the objective, decisions and their reasons, "
    "exact file paths, commit hashes, PR numbers, version strings, sizes, test counts and any "
    "value the agent reported. Also record what was NOT established. Use terse bullets and "
    "preserve values verbatim. Do not mention compaction.")


def load_synthetic(root, task):
    out = []
    for stage in range(8):
        d = json.loads((root / "data" / f"{task}-{stage}.json").read_text())
        out.append(d["archive"] + d["tail"])
    return out


def load_real(session):
    DB = pathlib.Path.home() / ".future" / "agent" / "agent.db"
    con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    rows = con.execute("""
        SELECT e.position, e.entry_type AS role, mb.kind, mb.text, mb.ordinal,
               mb.tool_call_id, mb.tool_name, mb.arguments_json
        FROM entries e JOIN message_blocks mb
          ON mb.session_id=e.session_id AND mb.entry_position=e.position
        WHERE e.session_id=? AND mb.kind IN ('text','tool_call','tool_result')
        ORDER BY e.position, mb.ordinal""", (session,)).fetchall()
    out = []
    for r in rows:
        rec = {"role": r["role"], "kind": r["kind"], "text": r["text"] or "",
               "position": r["position"]}
        # `to_messages` expects the fixture shape: call ids on tool calls and on
        # the matching results, plus a path when the call carried one.
        if r["kind"] == "tool_call":
            rec["call"] = r["tool_call_id"] or f"call-{r['position']}-{r['ordinal']}"
            rec["tool"] = r["tool_name"] or "tool"
            try:
                args = json.loads(r["arguments_json"] or "{}")
            except json.JSONDecodeError:
                args = {}
            # arguments_json is sometimes a JSON string rather than an object.
            if not isinstance(args, dict):
                args = {}
            rec["path"] = args.get("path") or args.get("file_path") or ""
        elif r["kind"] == "tool_result":
            rec["call"] = r["tool_call_id"] or ""
        out.append(rec)

    # Drop tool results with no call, then tool calls with no result. Keeping either
    # half alone makes the array invalid for the provider.
    calls = {r["call"] for r in out if r["kind"] == "tool_call" and r.get("call")}
    results = {r["call"] for r in out if r["kind"] == "tool_result" and r.get("call")}
    dropped_calls = calls - results
    out = [r for r in out
           if not (r["kind"] == "tool_result" and r.get("call") not in calls)
           and not (r["kind"] == "tool_call" and r.get("call") in dropped_calls)]
    if dropped_calls or (results - calls):
        print(f'    [load_real {session[:20]}] dropped {len(dropped_calls)} dangling tool calls, '
              f'{len(results - calls)} orphan results', flush=True)
    return out


def real_to_messages(records):
    """Group by position: one assistant message per entry, calls included.

    A real assistant entry can hold several tool calls, and their results arrive in
    later entries. Emitting each call as its own assistant message produced
    `assistant(callA), assistant(callB), tool(resultA), ...`, which a provider
    rejects; the valid shape is one assistant message carrying all the calls,
    immediately followed by the matching tool messages.
    """
    by_pos = {}
    order = []
    for r in records:
        key = r.get("position", -1)
        if key not in by_pos:
            by_pos[key] = []
            order.append(key)
        by_pos[key].append(r)

    messages = []
    for position in order:
        group = by_pos[position]
        role = group[0]["role"]
        if role == "tool":
            for r in group:
                if r["kind"] == "tool_result":
                    messages.append({"role": "tool", "tool_call_id": r.get("call", ""),
                                     "content": r["text"]})
            continue
        text = "\n".join(r["text"] for r in group if r["kind"] == "text" and r["text"])
        if role != "assistant":
            if text:
                messages.append({"role": "user", "content": text})
            continue
        calls = [{"id": r["call"], "type": "function",
                  "function": {"name": r.get("tool", "tool"),
                               "arguments": json.dumps({"path": r.get("path", "")})}}
                 for r in group if r["kind"] == "tool_call" and r.get("call")]
        if calls:
            messages.append({"role": "assistant", "content": text or None, "tool_calls": calls})
        elif text:
            messages.append({"role": "assistant", "content": text})
    return messages


def sanitize_messages(messages):
    """Make the array valid: every tool message answered, every call responded to."""
    # 1. keep only tool messages whose call is declared by an earlier assistant
    declared = set()
    kept = []
    for m in messages:
        if m.get("role") == "assistant" and m.get("tool_calls"):
            declared |= {c["id"] for c in m["tool_calls"]}
        if m.get("role") == "tool":
            if m.get("tool_call_id") in declared:
                kept.append(m)
            continue
        kept.append(m)

    # 2. keep only tool_calls whose results survived the filter above
    answered = {m["tool_call_id"] for m in kept if m.get("role") == "tool"}
    out = []
    for m in kept:
        if m.get("role") == "assistant" and m.get("tool_calls"):
            remaining = [c for c in m["tool_calls"] if c["id"] in answered]
            if not remaining:
                # an assistant whose calls all lost their results becomes plain text
                if (m.get("content") or "").strip():
                    out.append({"role": "assistant", "content": m["content"]})
                continue
            m = dict(m, tool_calls=remaining)
        out.append(m)
    return out


def stage_boundaries(records):
    """Equal thirds of a session, so synthetic and real chains use the same schedule."""
    n = len(records)
    return [max(1, int(n * f)) for f in (0.4, 0.7, 1.0)]


def tokens_of(records):
    return sum(len(r.get("text", "")) for r in records) // 4


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=100.0)
    ap.add_argument("--only", nargs="*", default=None)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "realistic-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, messages, max_tokens):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior:
            return prior["text"]
        body = {"model": args.model, "messages": messages, "stream": True,
                "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
        row = ledger.reserve(identity, args.model, request_reserve(len(json.dumps(body).encode()), max_tokens))
        began = time.monotonic()
        out = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                             capture_output=True, text=True, timeout=1200)
        if out.returncode != 0:
            # A rejected request must not look like an empty answer.
            ledger.settle(row, error=f"bridge exit {out.returncode}: {out.stderr[-400:]}")
            raise RuntimeError(f"{identity}: bridge exit {out.returncode}: {out.stderr[-400:]}")
        parsed = parse_sse(out.stdout)
        usage = parsed.get("usage") or {}
        ledger.settle(row, input_tokens=usage.get("prompt_tokens"), output_tokens=usage.get("completion_tokens"),
                      credit_cost=usage.get("credit_cost"), seconds=round(time.monotonic() - began, 3),
                      error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"))
        if parsed.get("error"):
            raise RuntimeError(f"{identity}: {parsed['error'][:200]}")
        if not (parsed.get("text") or "").strip():
            raise RuntimeError(f"{identity}: empty completion (finish={parsed.get('finish')})")
        return parsed["text"]

    # Session ids live in a git-ignored file so the repository holds no operator data.
    cfg = json.loads((args.root / "real-sessions.json").read_text())
    real = cfg["chains"]
    chains = {}
    for task in ("export", "analysis", "pipeline"):   # 3 synthetic + 3 real
        p = args.root / "data" / f"{task}-7.json"
        if p.exists():
            chains[task] = load_synthetic(args.root, task)
    for name, sid in real.items():
        chains[name] = [load_real(sid)]

    if args.only:
        chains = {k: v for k, v in chains.items() if k in args.only}

    results = []
    for name, chain in chains.items():
        records = chain[-1] if isinstance(chain, list) and isinstance(chain[0], list) else chain
        bounds = stage_boundaries(records)
        print(f"\n=== {name}: {len(records)} records, ~{tokens_of(records)} tokens ===", flush=True)
        summary = None
        for probe_index, upto in enumerate(bounds):
            covered = records[:upto]
            fresh = covered if summary is None else covered[prev_covered:]
            prev_covered = len(covered)

            # one sticky summary per stage, used by BOTH rules
            ident = f"{name}__s{probe_index}"
            base = ([{"kind": "text", "role": "user", "id": "prior-summary",
                      "text": f"<prior-summary>\n{summary}\n</prior-summary>"}] if summary else [])
            # Bound the summariser input: text records plus the most recent tool
            # results. A late synthetic stage holds megabytes of counters, and both
            # retention rules receive the SAME summary, so this favours neither.
            bounded = [r for r in fresh if r["kind"] == "text"]
            bounded += [r for r in fresh if r["kind"] == "tool_result"][-6:]
            bounded.sort(key=lambda r: (r.get("position", 0), r.get("ordinal", 0)))
            is_real = bool(bounded) and "position" in bounded[0]
            if is_real and base:
                prior = {"role": "user", "kind": "text", "position": -1,
                         "text": base[0]["text"]}
                msgs = sanitize_messages(real_to_messages([prior] + bounded))
            elif is_real:
                msgs = sanitize_messages(real_to_messages(bounded))
            else:
                msgs = sanitize_messages(shapes.to_messages(base + bounded))
            if msgs:
                text = call(f"realistic__{ident}__summary",
                            msgs + [{"role": "user", "content": SUMMARY_INSTRUCTION}], 32768).strip()
                if text:
                    summary = text

            # the two projections
            assistant_kept = [r for r in covered if r["kind"] == "text"]
            user_kept = [r for r in covered if r["kind"] == "text" and r["role"] == "user"]
            tool_kept = [r for r in covered if r["kind"] == "tool_result"][-4:]
            tail = covered[-20:]

            def render(kept, with_evidence, with_tail):
                parts = [shapes._plain(kept)]
                if with_evidence and tool_kept:
                    parts.append("<tool-evidence>\n" + shapes._plain(tool_kept) + "\n</tool-evidence>")
                if with_tail:
                    parts.append("<recent-history>\n" + shapes._plain(tail) + "\n</recent-history>")
                if summary:
                    parts.append(f"<state-summary>\n{summary}\n</state-summary>")
                return "<archived-conversation>\n" + "\n\n".join(p for p in parts if p) + "\n</archived-conversation>"

            projections = {
                "C3-verbatim": render(assistant_kept, True, True),
                "summary-only": render(user_kept, False, False),
            }

            present, decoys = exam.build_exam(records, upto, random.Random(9000 + probe_index))
            body, present_truth, decoy_truth = exam.exam_body(present, decoys, random.Random(1000 + probe_index))

            print(f"  stage {probe_index+1}: covered ~{tokens_of(covered)} tok, "
                  f"{len(present)} present / {len(decoys)} decoys", flush=True)
            for rule, value in projections.items():
                answer_text = call(f"realistic__{ident}__{rule}",
                                   [{"role": "system", "content": CLOSED_SYSTEM.replace(
                                       " No tools or external evidence are available in this condition.", "")},
                                    {"role": "user", "content": value + "\n\n" + body}], 8192)
                decoder, pos, answer = json.JSONDecoder(), 0, None
                while pos < len(answer_text or ""):
                    start = (answer_text or "").find("{", pos)
                    if start < 0:
                        break
                    try:
                        val, end = decoder.raw_decode(answer_text[start:])
                    except json.JSONDecodeError:
                        pos = start + 1
                        continue
                    if isinstance(val, dict) and "appeared" in val:
                        answer = val
                        break
                    pos = start + end
                sc = exam.score(answer, present_truth, decoy_truth)
                sc.update({"chain": name, "stage": probe_index, "rule": rule,
                           "projection_tokens": len(value) // 4,
                           "cost": ledger.rows[-1].get("charged", 0)})
                results.append(sc)
                print(f'    {rule:14s} hits {sc["hits"]}/{sc["of_present"]}  '
                      f'false-pos {sc["false_positives"]}/{sc["of_decoys"]}  net {sc["net"]:+d}  '
                      f'proj~{sc["projection_tokens"]}tok', flush=True)

    pathlib.Path(args.root / "realistic-results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
