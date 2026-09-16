"""Six-chain comparison of three strategies: C, Codex, OpenCode.

Chains: 3 synthetic (export, analysis, pipeline) + 3 real sessions (ids from the
git-ignored config, so the repository holds no operator data).

Strategies:
  C        all protected user+assistant originals + deterministic tool evidence + tail
  codex    all user messages (bounded) + a whole-history summary
  opencode the summary + a retained recent tail

Every projection is scored with the same realistic exam (assistant-weighted, with
decoys), and every arm sees the same exam at the same stage.
"""
import argparse, json, pathlib, random, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM
import abc_cache_shapes as shapes
import realistic_exam as exam
from realistic_eval import load_real, real_to_messages, sanitize_messages

# Probe boundaries. Synthetic fixtures are cumulative per stage, so use their own
# stages; real chains have no stage structure and are sliced by fraction.
SYNTH_STAGES = (0, 3, 7)
REAL_FRACTIONS = (0.4, 0.7, 1.0)

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

# The real code expands the protected-original budget to at most 64K tokens.
PROTECTED_CHARS = 64_000 * 4


def clip(text, head, tail):
    if len(text) <= head + tail:
        return text
    return text[:head] + f"\n[... {len(text) - head - tail} characters omitted ...]\n" + text[-tail:]


def render_c(covered):
    """C: every protected original, a bounded evidence index, and the recent tail.

    The originals are NOT head-clipped: C retains all user and assistant text up to
    its protected budget, which is what distinguishes it from the summarising arms.
    Only the tool evidence and the tail are bounded.
    """
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
            + clip(shapes._plain(tail), 8000, 2000) + "\n</recent-history>"
            + "\n</archived-conversation>")


def render_codex(covered, summary):
    users = [r for r in covered if r["kind"] == "text" and r["role"] == "user"]
    return ("<archived-conversation>\n" + clip(shapes._plain(users), 20000, 2000)
            + f"\n\n<state-summary>\n{summary}\n</state-summary>\n</archived-conversation>")


def render_opencode(covered, summary):
    tail = covered[-24:]
    return ("<archived-conversation>\n"
            + f"<state-summary>\n{summary}\n</state-summary>"
            + "\n\n<recent-history>\n" + clip(shapes._plain(tail), 8000, 2000)
            + "\n</recent-history>\n</archived-conversation>")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, required=True)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=60.0)
    ap.add_argument("--only", nargs="*", default=None)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "sixchain-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, messages, max_tokens, thinking=False, attempts=3):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior:
            return prior["text"]
        for attempt in range(attempts):
            suffix = "" if attempt == 0 else f"__retry{attempt}"
            body = {"model": args.model, "messages": messages, "stream": True,
                    "max_tokens": max_tokens, "stream_options": {"include_usage": True}}
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
                          error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"))
            if parsed.get("error"):
                raise RuntimeError(f"{identity}: {parsed['error'][:200]}")
            if (parsed.get("text") or "").strip():
                return parsed["text"]
            # An empty completion means the output allowance was consumed; retrying
            # under a fresh identity is the only way to get a usable answer.
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
            stages.append(d["archive"] + d["tail"])
        if stages:
            chains[task] = stages
    for name, sid in cfg["chains"].items():
        records = load_real(sid)
        chains[name] = [records[:max(1, int(len(records) * f))] for f in REAL_FRACTIONS]

    if args.only:
        chains = {k: v for k, v in chains.items() if k in args.only}

    results = []
    for name, stages in chains.items():
        print(f"\n=== {name}: 3 boundaries, largest {len(stages[-1])} records ===", flush=True)
        for stage_index, covered in enumerate(stages):
            ident = f"six__{name}__s{stage_index}"

            bounded = [r for r in covered if r["kind"] == "text"]
            bounded += [r for r in covered if r["kind"] == "tool_result"][-6:]
            bounded.sort(key=lambda r: (r.get("position", 0), r.get("ordinal", 0)))
            is_real = bool(bounded) and "position" in bounded[0]
            msgs = sanitize_messages(real_to_messages(bounded) if is_real
                                     else shapes.to_messages(bounded))

            codex_summary = call(f"{ident}__codex_summary",
                                 msgs + [{"role": "user", "content": CODEX_PROMPT}], 16384).strip()
            opencode_summary = call(f"{ident}__opencode_summary",
                                    msgs + [{"role": "user", "content": OPENCODE_PROMPT}], 16384).strip()

            projections = {
                "C": render_c(covered),
                "codex": render_codex(covered, codex_summary),
                "opencode": render_opencode(covered, opencode_summary),
            }

            present, decoys = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
            body, p_truth, d_truth = exam.exam_body(present, decoys, random.Random(1000 + stage_index))

            for arm, value in projections.items():
                answer_text = call(f"{ident}__{arm}__probe",
                                   [{"role": "system", "content": CLOSED_SYSTEM.replace(
                                       " No tools or external evidence are available in this condition.", "")},
                                    {"role": "user", "content": value + "\n\n" + body}], 8192, thinking=True)
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
                           "proj_tokens": len(value) // 4,
                           "cost": ledger.rows[-1].get("charged", 0)})
                results.append(sc)
                print(f'   {arm:9s} hits {sc["hits"]}/{sc["of_present"]}  '
                      f'fp {sc["false_positives"]}/{sc["of_decoys"]}  proj~{sc["proj_tokens"]}tok', flush=True)

    pathlib.Path(args.root / "sixchain-results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print(f'\nledger: {len(ledger.rows)} requests, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
