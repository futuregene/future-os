"""Six-chain comparison WITH retrieval: the same exam, now open-book.

The closed-book run measured what each retention rule preserves on its own
(C 122/176, Codex 116/176, OpenCode 104/176). This measures what each strategy can
still recover when the agent can look things up in the original archive.

Each strategy gets the lookup interface that matches its own design, so the question
answered is "how good is each agent at its job", not "which lookup engine is best":

  C        the archive CLI (future session history search/get)
  codex    the window/item interface, as the Codex arm has been modelled throughout
  opencode the filesystem (glob / grep / read over the working tree)

A miss is expensive here: the archive has to hold the material the projection
dropped, which is exactly what a compaction strategy chooses to discard.
"""
import argparse, json, os, pathlib, random, sqlite3, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, QUESTION
import abc_retrieval as R
import abc_cache_shapes as shapes
import realistic_exam as exam
from realistic_eval import load_real, real_to_messages, sanitize_messages

SYNTH_STAGES = (0, 3, 7)
REAL_FRACTIONS = (0.4, 0.7, 1.0)
PROTECTED_CHARS = 64_000 * 4

CODEX_PROMPT = (
    "Create a detailed summary of the conversation so far. Focus on the user's explicit requests, "
    "your actions, key technical details (file paths, function names, versions, exact values) and "
    "any errors you hit and how you fixed them. Preserve the agent's own decisions and the reasons "
    "for them. Output only terse bullet points. Do not mention compaction."
)
OPENCODE_PROMPT = (
    "Provide a detailed but concise summary of our conversation above. Focus on information that "
    "would be helpful for continuing the conversation, including what we did, what we're doing, "
    "which files are we modifying, what is left to do, current work and which next steps are we "
    "planning to take. Include exact paths, identifiers, versions and values."
)


def text_of(result):
    """Plain text from a call() result (which may be parsed or a raw string)."""
    if isinstance(result, str):
        return result
    return (result.get("text") or "")


def clip(text, head, tail):
    if len(text) <= head + tail:
        return text
    return text[:head] + f"\n[... {len(text) - head - tail} characters omitted ...]\n" + text[-tail:]


def render_c(covered):
    kept = [r for r in covered if r["kind"] == "text"]
    tools = [r for r in covered if r["kind"] == "tool_result"][-6:]
    tail = covered[-24:]
    blocks = "\n\n".join(clip(shapes._plain([r]), 380, 100) for r in tools)
    originals = shapes._plain(kept)
    if len(originals) > PROTECTED_CHARS:
        originals = clip(originals, PROTECTED_CHARS // 2, PROTECTED_CHARS // 2)
    return ("<archived-conversation>\n<protected-originals>\n" + originals
            + "\n</protected-originals>\n\n<deterministic-evidence>\n" + blocks
            + "\n</deterministic-evidence>\n\n<recent-history>\n"
            + clip(shapes._plain(tail), 8000, 2000) + "\n</recent-history>\n</archived-conversation>")


def render_codex(covered, summary):
    users = [r for r in covered if r["kind"] == "text" and r["role"] == "user"]
    return ("<archived-conversation>\n" + clip(shapes._plain(users), 20000, 2000)
            + f"\n\n<state-summary>\n{summary}\n</state-summary>\n</archived-conversation>")


def render_opencode(covered, summary):
    tail = covered[-24:]
    return ("<archived-conversation>\n" + f"<state-summary>\n{summary}\n</state-summary>"
            + "\n\n<recent-history>\n" + clip(shapes._plain(tail), 8000, 2000)
            + "\n</recent-history>\n</archived-conversation>")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--binary", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=40.0)
    ap.add_argument("--only", nargs="*", default=None)
    ap.add_argument("--modes", nargs="*", default=["ours", "codex", "opencode"])
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "sixopen-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, messages, max_tokens, tools=None, thinking=False, attempts=3):
        # Cache the tool calls as well as the text. Returning only the text made a
        # replayed probe look like a final answer, so a retrieval step was scored as
        # the response and the cell came out as zero.
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior:
            return {"text": prior["text"], "calls": prior.get("calls") or [],
                    "usage": None, "finish": prior.get("finish"), "error": None}
        for attempt in range(attempts):
            suffix = "" if attempt == 0 else f"__retry{attempt}"
            body = {"model": args.model, "messages": messages, "stream": True,
                    "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
            if tools:
                body["tools"] = tools
            if thinking:
                body.update({"thinking": {"type": "enabled"}, "reasoning_effort": "high"})
            row = ledger.reserve(identity + suffix, args.model,
                                 request_reserve(len(json.dumps(body).encode()), max_tokens))
            began = time.monotonic()
            out = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                 capture_output=True, text=True, timeout=1200)
            if out.returncode != 0:
                ledger.settle(row, error=f"bridge exit {out.returncode}: {out.stderr[-300:]}")
                raise RuntimeError(f"{identity}: bridge exit {out.returncode}: {out.stderr[-300:]}")
            parsed = parse_sse(out.stdout)
            usage = parsed.get("usage") or {}
            ledger.settle(row, input_tokens=usage.get("prompt_tokens"),
                          output_tokens=usage.get("completion_tokens"),
                          credit_cost=usage.get("credit_cost"),
                          seconds=round(time.monotonic() - began, 3),
                          error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"),
                          calls=parsed["calls"])
            if parsed.get("error"):
                raise RuntimeError(f"{identity}: {parsed['error'][:200]}")
            if (parsed.get("text") or "").strip() or parsed.get("calls"):
                return parsed
            # an empty completion means the output allowance was consumed; retry
        raise RuntimeError(f"{identity}: empty completion after {attempts} attempts")

    cfg = json.loads((args.root / "real-sessions.json").read_text())
    chains = {}
    for task in ("export", "analysis", "pipeline"):
        stages = []
        for stage in SYNTH_STAGES:
            p = args.root / "data" / f"{task}-{stage}.json"
            if not p.exists():
                stages = []
                break
            d = json.loads(p.read_text())
            stages.append((d["archive"] + d["tail"], d["source_session"]))
        if stages:
            chains[task] = stages
    for name, sid in cfg["chains"].items():
        records = load_real(sid)
        chains[name] = [(records[:max(1, int(len(records) * f))], sid) for f in REAL_FRACTIONS]
    if args.only:
        chains = {k: v for k, v in chains.items() if k in args.only}

    results = []
    # interface -> the strategy it is used to probe
    STRATEGY_FOR_MODE = {"ours": "C", "codex": "codex", "opencode": "opencode"}
    strategies = [STRATEGY_FOR_MODE[m] for m in args.modes]
    synthetic = {"export", "analysis", "pipeline"}
    print(f"[diag] chains={list(chains)} modes={args.modes} only={args.only}", flush=True)
    for _n, _s in chains.items():
        print(f"[diag] {_n}: {len(_s)} stages", flush=True)
    for name, stages in chains.items():
        skipped = sorted(m for m in args.modes if m == "ours" and name in synthetic)
        note = f"  (ours unavailable for synthetic chain: no session in the Agent database)" if skipped else ""
        print(f"\n=== {name} ==={note}", flush=True)
        for stage_index, (covered, archive_session) in enumerate(stages):
            ident = f"sixopen__{name}__s{stage_index}"

            bounded = [r for r in covered if r["kind"] == "text"]
            bounded += [r for r in covered if r["kind"] == "tool_result"][-6:]
            bounded.sort(key=lambda r: (r.get("position", 0), r.get("ordinal", 0)))
            is_real = bool(bounded) and "position" in bounded[0]
            msgs = sanitize_messages(real_to_messages(bounded) if is_real
                                     else shapes.to_messages(bounded))

            projections = {}
            if "C" in strategies:
                projections["C"] = render_c(covered)
            if "codex" in strategies:
                codex_summary = text_of(call(f"{ident}__codex_summary",
                                             msgs + [{"role": "user", "content": CODEX_PROMPT}], 16384))
                if not codex_summary.strip():
                    raise RuntimeError(f"{ident}: empty codex summary")
                projections["codex"] = render_codex(covered, codex_summary)
            if "opencode" in strategies:
                opencode_summary = text_of(call(f"{ident}__opencode_summary",
                                                msgs + [{"role": "user", "content": OPENCODE_PROMPT}], 16384))
                if not opencode_summary.strip():
                    raise RuntimeError(f"{ident}: empty opencode summary")
                projections["opencode"] = render_opencode(covered, opencode_summary)
            if not projections:
                raise SystemExit(f"no strategy for modes {args.modes}")

            present, decoys = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
            body, p_truth, d_truth = exam.exam_body(present, decoys, random.Random(1000 + stage_index))

            windows = (R.codex_windows(args.root, name, stage_index)
                       if name in ("export", "analysis", "pipeline")
                       else R.codex_windows_from_records(covered))

            for arm, value in projections.items():
                # `arm` is the strategy; the interface is whichever mode selects it.
                mode = next(m for m, s in STRATEGY_FOR_MODE.items() if s == arm)
                if mode not in args.modes:
                    continue
                if mode == "ours" and name in synthetic:
                    continue
                workspace = None
                if mode == "opencode":
                    workspace = pathlib.Path(f"/tmp/sixopen-ws-{name}-{stage_index}")
                    import shutil as _sh
                    if workspace.exists():
                        _sh.rmtree(workspace)
                    if name in ("export", "analysis", "pipeline"):
                        R.materialize_workspace(args.root, name, SYNTH_STAGES[stage_index], workspace)
                    else:
                        R.materialize_workspace_from_records(covered, workspace)
                guide = R.guide(mode, session_id=archive_session, workspace=str(workspace or ""))
                tools = (R.codex_tools() if mode == "codex"
                         else R.our_tools(archive_session) if mode == "ours" else R.opencode_tools())
                system = ("You are answering questions about an archived engineering session. "
                          "You may look things up." + guide)
                conversation = [{"role": "system", "content": system},
                                {"role": "user", "content": value + "\n\n" + body}]

                returned, calls_made, answer_text = 0, 0, ""
                print(f"   [{arm}] starting retrieval loop", flush=True)
                for step in range(R.RUNAWAY_GUARD):
                    parsed = call(f"{ident}__{arm}__r{step}", conversation, 8192, tools=tools,
                                  thinking=True, attempts=2)
                    if isinstance(parsed, str):
                        # defensive: a cached row without parsed calls must not be
                        # mistaken for a finished answer
                        answer_text = parsed
                        break
                    calls_made += 1
                    if not parsed.get("calls"):
                        answer_text = parsed["text"]
                        break
                    conversation.append({"role": "assistant", "content": parsed["text"] or None,
                                         "tool_calls": parsed["calls"]})
                    for c in parsed["calls"]:
                        name_ = c["function"]["name"]
                        try:
                            arguments = json.loads(c["function"]["arguments"] or "{}")
                        except json.JSONDecodeError:
                            arguments = {}
                        room = max(0, R.RETRIEVAL_BYTES - returned)
                        env = dict(os.environ)
                        env["FUTURE_AGENT_GRPC_ADDR"] = os.environ.get("FUTURE_AGENT_GRPC_ADDR", "")
                        if mode == "codex":
                            out, size = R.codex_dispatch(name_, arguments, windows, room)
                        elif mode == "ours":
                            out, size = R.our_dispatch(arguments.get("command", ""), archive_session,
                                                       args.binary, env, str(workspace or args.root))
                        else:
                            # OpenCode's tools are filesystem reads over the tree
                            out, size = R.opencode_dispatch(name_, arguments, workspace, room)
                        returned += min(size, room)
                        conversation.append({"role": "tool", "tool_call_id": c["id"],
                                             "content": out[:room]})

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
                sc = exam.score(answer, p_truth, d_truth)
                sc.update({"chain": name, "stage": stage_index, "arm": arm,
                           "proj_tokens": len(value) // 4, "calls": calls_made,
                           "bytes": returned, "status": "answered" if answer else "no_answer"})
                results.append(sc)
                print(f'   {arm:9s} hits {sc["hits"]}/{sc["of_present"]} fp {sc["false_positives"]} '
                      f'calls={calls_made} bytes={returned}', flush=True)
                if workspace and workspace.exists():
                    import shutil as _sh
                    _sh.rmtree(workspace, ignore_errors=True)

    out = args.root / "sixopen-results.json"
    merged = {}
    if out.exists():
        for row in json.loads(out.read_text()):
            merged[(row["chain"], row["stage"], row["arm"])] = row
    for row in results:
        merged[(row["chain"], row["stage"], row["arm"])] = row
    out.write_text(json.dumps(list(merged.values()), ensure_ascii=False, indent=2) + "\n")
    print(f"results merged: {len(merged)} rows on disk", flush=True)
    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
