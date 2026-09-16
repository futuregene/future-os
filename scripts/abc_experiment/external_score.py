import os as _os
import pathlib as _pathlib
import subprocess as _subprocess


def _checkout():
    """The checkout this script lives in (…/<checkout>/scripts/abc_experiment/x.py)."""
    return _pathlib.Path(__file__).resolve().parents[2]


def _main_checkout():
    """The main checkout, which owns the shared .future directory.

    `--git-common-dir` resolves to <main>/.git even when running from a worktree, so the
    research directory is found without depending on any absolute path.
    """
    try:
        out = _subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=_pathlib.Path(__file__).resolve().parent, capture_output=True, text=True,
            timeout=30)
        if out.returncode == 0 and out.stdout.strip():
            return _pathlib.Path(out.stdout.strip()).parent
    except Exception:
        pass
    return _checkout()


def _research():
    """The experiment root: fixtures, frozen sessions, ledgers and results.

    Deliberately outside any repository -- it holds real session data and large ledgers
    that must never be committed. `ABC_ROOT` overrides the default.
    """
    override = _os.environ.get("ABC_ROOT")
    if override:
        return _pathlib.Path(override)
    return _pathlib.Path.home() / "compact-exp"


def require(path, what, how=""):
    """Return `path` or stop immediately with an explanation.

    Inputs used to be skipped when absent, so a run without them produced a partial result
    that looked complete. Failing here is the difference between "the numbers are wrong"
    and "the numbers are missing".
    """
    path = _pathlib.Path(path)
    if path.exists():
        return path
    raise SystemExit(
        f"missing {what}:\n  {path}\n"
        + (f"  {how}\n" if how else "")
        + "  Set ABC_ROOT to the experiment root, or see "
          "scripts/abc_experiment/README.md."
    )


WORKTREE = _checkout()
REPO = _main_checkout()
ROOT = _research()

"""Generate and score Codex/OpenCode projections over the frozen sessions.

Both arms replace history with a summary, so their projections are built here from the
frozen records with each strategy's own selection rule (transcribed from upstream), then
scored closed book with the same exam. Interface variation belongs to the open-book
pass; this pass fixes the exam and varies only the retention rule.
"""
import argparse, json, pathlib, random, subprocess, sys, time

WT = WORKTREE
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM
import abc_cache_shapes as shapes
import realistic_exam as exam
from realistic_eval import real_to_messages, sanitize_messages

ROOT = ROOT
FROZEN = ROOT / "frozen-sessions"

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


def clip(text, head, tail):
    if len(text) <= head + tail:
        return text
    return text[:head] + f"\n[... {len(text) - head - tail} characters omitted ...]\n" + text[-tail:]


def render_codex(covered, summary):
    users = [r for r in covered if r["kind"] == "text" and r["role"] == "user"]
    return ("<archived-conversation>\n" + clip(shapes._plain(users), 20000, 2000)
            + f"\n\n<state-summary>\n{summary}\n</state-summary>\n</archived-conversation>")


def render_opencode(covered, summary):
    tail = covered[-24:]
    return ("<archived-conversation>\n" + f"<state-summary>\n{summary}\n</state-summary>"
            + "\n\n<recent-history>\n" + clip(shapes._plain(tail), 8000, 2000)
            + "\n</recent-history>\n</archived-conversation>")


def covered_sets():
    out = {}
    for task in ("export", "analysis", "pipeline"):
        for stage in (0, 3, 7):
            p = ROOT / "data" / f"{task}-{stage}.json"
            if p.exists():
                d = json.loads(p.read_text())
                out[f"{task}__s{stage}"] = d["archive"] + d["tail"]
    manifest = json.loads(require(FROZEN / "manifest.json", "frozen real sessions",
                                "Run freeze_sessions.py against your own Agent database first.").read_text())
    for name, meta in manifest.items():
        records = json.loads((FROZEN / meta["path"]).read_text())["records"]
        for index, frac in enumerate((0.4, 0.7, 1.0)):
            out[f"{name}__s{index}"] = records[:max(1, int(len(records) * frac))]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, default=ROOT)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--arms", nargs="*", default=["codex", "opencode"])
    ap.add_argument("--budget", type=float, default=15.0)
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "score-external-calls.json"
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
                          text=parsed["text"], finish=parsed.get("finish"))
            if (parsed.get("text") or "").strip():
                return parsed["text"]
        raise RuntimeError(f"{identity}: empty after retries")

    sets = covered_sets()
    outdir = args.root / "ExternalProj" / "projections"
    outdir.mkdir(parents=True, exist_ok=True)
    results = []
    for identity, covered in sets.items():
        stage_index = int(identity.rsplit("__s", 1)[1])
        bounded = [r for r in covered if r["kind"] == "text"]
        bounded += [r for r in covered if r["kind"] == "tool_result"][-6:]
        bounded.sort(key=lambda r: (r.get("position", 0), r.get("ordinal", 0)))
        is_real = bool(bounded) and "position" in bounded[0]
        msgs = sanitize_messages(real_to_messages(bounded) if is_real
                                 else shapes.to_messages(bounded))

        present, decoys = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
        body, p_truth, d_truth = exam.exam_body(present, decoys, random.Random(1000 + stage_index))

        for arm in args.arms:
            prompt = CODEX_PROMPT if arm == "codex" else OPENCODE_PROMPT
            cap = 16384 if arm == "codex" else 4096
            target = outdir / f"{arm}__{identity}.json"
            if target.exists():
                value = json.loads(target.read_text())["text"]
            else:
                summary = call(f"ext__{arm}__{identity}__summary",
                               msgs + [{"role": "user", "content": prompt}], cap).strip()
                if not summary:
                    print(f"  {arm} {identity}: EMPTY SUMMARY", flush=True)
                    continue
                value = (render_codex(covered, summary) if arm == "codex"
                         else render_opencode(covered, summary))
                target.write_text(json.dumps({"id": identity, "arm": arm, "text": value,
                                              "context_tokens": len(value) // 4,
                                              "summary_tokens": len(summary) // 4},
                                             ensure_ascii=False, indent=2) + "\n")
            answer_text = call(f"extscore__{arm}__{identity}__probe",
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
            sc.update({"chain": identity.rsplit("__s", 1)[0], "stage": stage_index,
                       "arm": arm, "identity": identity, "calls": 0,
                       "projection_tokens": len(value) // 4})
            results.append(sc)
            print(f'  {arm:9s} {identity:18s} {sc["hits"]:>3d}/{sc["of_present"]:<3d} '
                  f'fp={sc["false_positives"]} proj={len(value)//4}tok', flush=True)

    out = args.root / "score-external-closed.json"
    merged = {f'{r["arm"]}__{r["identity"]}': r for r in
              (json.loads(out.read_text()) if out.exists() else [])}
    for r in results:
        merged[f'{r["arm"]}__{r["identity"]}'] = r
    out.write_text(json.dumps(list(merged.values()), ensure_ascii=False, indent=2) + "\n")
    for arm in args.arms:
        rows = [r for r in merged.values() if r["arm"] == arm]
        h = sum(r["hits"] for r in rows)
        n = sum(r["of_present"] for r in rows)
        print(f'{arm}: {h}/{n} = {100*h/max(n,1):.1f}% over {len(rows)} probes')
    print(f'ledger: {len(ledger.rows)} calls, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
