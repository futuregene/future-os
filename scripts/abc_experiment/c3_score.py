"""Score the real C3 projections, closed book and open book, on all six chains.

The projections come from the production Rust path (`abc_c3_probe`), so these numbers
describe what the runtime actually commits. Closed book uses the same exam protocol as
before; open book additionally gives the arm its own lookup interface.

Reports, per arm and chain: hits, false positives, lookups used, and cost.
"""
import argparse, json, os, pathlib, random, subprocess, sys, time

WT = pathlib.Path("/Users/geilige/future-os/.worktrees/session-history-a47313")
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM
import abc_retrieval as R
import realistic_exam as exam

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
FROZEN = ROOT / "frozen-sessions"


def covered_sets():
    out = {}
    for task in ("export", "analysis", "pipeline"):
        for stage in (0, 3, 7):
            p = ROOT / "data" / f"{task}-{stage}.json"
            if p.exists():
                d = json.loads(p.read_text())
                # The fixture names the session it corresponds to, and that session is
                # written into the isolated database, so the archive CLI can read it.
                # Passing None here left the model with an empty session id and every
                # synthetic lookup came back empty.
                out[f"{task}__s{stage}"] = (d["archive"] + d["tail"], d["source_session"])
    manifest = json.loads((FROZEN / "manifest.json").read_text())
    for name, meta in manifest.items():
        records = json.loads(pathlib.Path(meta["path"]).read_text())["records"]
        for index, frac in enumerate((0.4, 0.7, 1.0)):
            out[f"{name}__s{index}"] = (records[:max(1, int(len(records) * frac))], meta["session"])
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, default=ROOT)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--binary", type=pathlib.Path, required=True)
    ap.add_argument("--projdir", default="C3proj")
    ap.add_argument("--prefix", default="",
                    help="projection filename prefix, e.g. codex__")
    ap.add_argument("--tag", default="c3")
    ap.add_argument("--mode", choices=["closed", "open"], default="closed")
    ap.add_argument("--interface", default="ours")
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=25.0)
    ap.add_argument("--retag-suffix", default="",
                    help="rename existing rows with this suffix so they re-run")
    argparse.ArgumentParser().parse_args([])
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / f"score-{args.tag}-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []
    if args.retag_suffix:
        renamed = 0
        for row in ledger.rows:
            if not row["id"].endswith(args.retag_suffix):
                row["id"] = row["id"] + args.retag_suffix
                row["note"] = "superseded: the CLI lookup fell back to an agent that " \
                              "predates search_session_history, so it returned nothing"
                renamed += 1
        ledger.save()
        print(f"retired {renamed} rows with {args.retag_suffix}")

    def call(identity, messages, max_tokens, tools=None, thinking=False, attempts=3):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior:
            return {"text": prior["text"], "calls": prior.get("calls") or [],
                    "finish": prior.get("finish")}
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
                                 capture_output=True, text=True, timeout=1800)
            if out.returncode != 0:
                ledger.settle(row, error=f"exit {out.returncode}: {out.stderr[-300:]}")
                raise RuntimeError(out.stderr[-300:])
            parsed = parse_sse(out.stdout)
            usage = parsed.get("usage") or {}
            ledger.settle(row, input_tokens=usage.get("prompt_tokens"),
                          output_tokens=usage.get("completion_tokens"),
                          credit_cost=usage.get("credit_cost"),
                          seconds=round(time.monotonic() - began, 3), error=parsed.get("error"),
                          text=parsed["text"], finish=parsed.get("finish"), calls=parsed["calls"])
            if (parsed.get("text") or "").strip() or parsed.get("calls"):
                return parsed
        raise RuntimeError(f"{identity}: empty after retries")

    sets = covered_sets()
    results = []
    for identity, (covered, session) in sets.items():
        proj_path = args.root / args.projdir / "projections" / f"{args.prefix}{identity}.json"
        if not proj_path.exists():
            continue
        projection = json.loads(proj_path.read_text())
        value = projection["text"]
        chain, stage_index = identity.rsplit("__s", 1)
        stage_index = int(stage_index)
        present, decoys = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
        body, p_truth, d_truth = exam.exam_body(present, decoys, random.Random(1000 + stage_index))

        calls_made, returned = 0, 0
        workspace = None
        if args.mode == "closed":
            answer_text = call(f"{args.tag}__{identity}__closed",
                               [{"role": "system", "content": CLOSED_SYSTEM.replace(
                                   " No tools or external evidence are available in this condition.", "")},
                                {"role": "user", "content": value + "\n\n" + body}], 8192,
                               thinking=True)["text"]
        else:
            # Each interface needs its own substrate. Passing an empty window list to
            # the Codex interface, or no working tree to OpenCode, makes every lookup
            # fail silently and the run measures a broken tool rather than retrieval.
            task, stage_for_fixture = (chain, stage_index) if chain in (
                "export", "analysis", "pipeline") else (None, None)
            windows, workspace = [], None
            if args.interface == "codex":
                windows = (R.codex_windows(args.root, task, stage_for_fixture)
                           if task else R.codex_windows_from_records(covered))
            elif args.interface == "opencode":
                workspace = pathlib.Path(f"/tmp/c3-open-ws-{identity}")
                import shutil as _sh
                if workspace.exists():
                    _sh.rmtree(workspace)
                if task:
                    R.materialize_workspace(args.root, task, stage_for_fixture, workspace)
                else:
                    R.materialize_workspace_from_records(covered, workspace)
            guide = R.guide(args.interface, session_id=session or "",
                            workspace=str(workspace or ""))
            tools = R.codex_tools() if args.interface == "codex" else (
                R.our_tools(session) if args.interface == "ours" else R.opencode_tools())
            conversation = [{"role": "system", "content":
                             "You are answering questions about an archived engineering session. "
                             "You may look things up." + guide},
                            {"role": "user", "content": value + "\n\n" + body}]
            answer_text = ""
            for step in range(R.RUNAWAY_GUARD):
                parsed = call(f"{args.tag}__{identity}__r{step}", conversation, 8192,
                              tools=tools, thinking=True, attempts=2)
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
                    env = dict(os.environ)
                    if args.interface == "codex":
                        out, size = R.codex_dispatch(name_, arguments, windows, 32768)
                    elif args.interface == "opencode":
                        out, size = R.opencode_dispatch(name_, arguments, workspace, 32768)
                    else:
                        out, size = R.our_dispatch(arguments.get("command", ""), session or "",
                                                   args.binary, env, str(WT))
                    returned += size
                    conversation.append({"role": "tool", "tool_call_id": c["id"],
                                         "content": out[:32768]})

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
        if workspace is not None and workspace.exists():
            import shutil as _sh
            _sh.rmtree(workspace, ignore_errors=True)
        sc = exam.score(answer, p_truth, d_truth)
        sc.update({"chain": chain, "stage": stage_index, "identity": identity,
                   "calls": calls_made, "bytes": returned,
                   "projection_tokens": projection["context_tokens"],
                   "summary_present": "Model handoff summary" in value})
        results.append(sc)
        print(f'  {identity:18s} {sc["hits"]:>3d}/{sc["of_present"]:<3d} fp={sc["false_positives"]} '
              f'calls={calls_made} proj={projection["context_tokens"]}tok', flush=True)

    out = args.root / f"score-{args.tag}-{args.mode}-{args.interface}.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    h = sum(r["hits"] for r in results)
    n = sum(r["of_present"] for r in results)
    print(f'\n{args.tag} {args.mode}/{args.interface}: {h}/{n} = {100*h/max(n,1):.1f}%, '
          f'fp={sum(r["false_positives"] for r in results)}, '
          f'mean calls={sum(r["calls"] for r in results)/max(len(results),1):.2f}')
    print(f'ledger: {len(ledger.rows)} calls, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
