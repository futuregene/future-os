"""Re-score the exam against the REAL C projections.

The earlier closed-book C column was produced from the Python approximation. The real
projections (from `prepare_evidence`, zero model calls to build) are now in
Creal/projections, so this runs the identical exam protocol against them and compares.

Same items (same seeds), same exam body, same system prompt and max_tokens as the
six-chain run, so the only change is the projection.
"""
import argparse, json, pathlib, random, subprocess, sys, time

WT = pathlib.Path("/Users/geilige/future-os/.worktrees/session-history-a47313")
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse, CLOSED_SYSTEM
import realistic_exam as exam

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=pathlib.Path, default=ROOT)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=5.0)
    ap.add_argument("--projdir", default="Creal")
    ap.add_argument("--tag", default="real")
    args = ap.parse_args()

    ledger = Ledger(args.root, args.budget)
    ledger.path = args.root / "c-real-score-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def call(identity, messages, max_tokens, attempts=3):
        prior = next((r for r in ledger.rows if r["id"] == identity and r.get("state") == "finished"
                      and r.get("text")), None)
        if prior:
            return prior["text"]
        for attempt in range(attempts):
            suffix = "" if attempt == 0 else f"__retry{attempt}"
            body = {"model": args.model, "messages": messages, "stream": True,
                    "max_tokens": max_tokens, "stream_options": {"include_usage": True},
                    "thinking": {"type": "enabled"}, "reasoning_effort": "high"}
            row = ledger.reserve(identity + suffix, args.model,
                                 request_reserve(len(json.dumps(body).encode()), max_tokens))
            began = time.monotonic()
            out = subprocess.run([str(args.bridge)], input=json.dumps({"model": args.model, "body": body}),
                                 capture_output=True, text=True, timeout=1200)
            if out.returncode != 0:
                ledger.settle(row, error=f"bridge exit {out.returncode}: {out.stderr[-300:]}")
                raise RuntimeError(out.stderr[-300:])
            parsed = parse_sse(out.stdout)
            usage = parsed.get("usage") or {}
            ledger.settle(row, input_tokens=usage.get("prompt_tokens"),
                          output_tokens=usage.get("completion_tokens"),
                          credit_cost=usage.get("credit_cost"),
                          seconds=round(time.monotonic() - began, 3),
                          error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"))
            if (parsed.get("text") or "").strip():
                return parsed["text"]
        raise RuntimeError(f"{identity}: empty after retries")

    # The same covered sets and seeds as the six-chain run.
    import abc_cache_shapes as shapes
    from realistic_eval import load_real
    covered_sets = {}
    for task in ("export", "analysis", "pipeline"):
        for stage in (0, 3, 7):
            d = json.loads((args.root / "data" / f"{task}-{stage}.json").read_text())
            covered_sets[f"{task}__s{stage}"] = d["archive"] + d["tail"]
    cfg = json.loads((args.root / "real-sessions.json").read_text())
    for name, sid in cfg["chains"].items():
        records = load_real(sid)
        for index, frac in enumerate((0.4, 0.7, 1.0)):
            covered_sets[f"{name}__s{index}"] = records[:max(1, int(len(records) * frac))]

    old = {}
    for row in json.loads((args.root / "sixchain-results.json").read_text()):
        if row["arm"] == "C":
            # map chain/stage back to the boundary identity used by Creal
            old[(row["chain"], row["stage"])] = row

    results = []
    for identity, covered in covered_sets.items():
        proj_path = args.root / args.projdir / "projections" / f"{identity}.json"
        if not proj_path.exists():
            continue
        value = json.loads(proj_path.read_text())["text"]
        chain, stage_index = identity.rsplit("__s", 1)
        stage_index = int(stage_index)
        present, decoys = exam.build_exam(covered, len(covered), random.Random(9000 + stage_index))
        body, p_truth, d_truth = exam.exam_body(present, decoys, random.Random(1000 + stage_index))
        answer_text = call(f"crealscore__{args.tag}__{identity}",
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
        sc = exam.score(answer, p_truth, d_truth)
        sc.update({"chain": chain, "stage": stage_index, "identity": identity})
        results.append(sc)
        prior = old.get((chain, stage_index))
        was = f'{prior["hits"]}/{prior["of_present"]}' if prior else "—"
        print(f'  {identity:18s} real {sc["hits"]}/{sc["of_present"]}  '
              f'approx {was}  fp {sc["false_positives"]}', flush=True)

    out = args.root / f"c-real-scores-{args.tag}.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    rh = sum(r["hits"] for r in results)
    rn = sum(r["of_present"] for r in results)
    ah = sum(v["hits"] for v in old.values())
    an = sum(v["of_present"] for v in old.values())
    print(f'\nreal C total: {rh}/{rn}   approximation total: {ah}/{an}')
    print(f'ledger: {len(ledger.rows)} calls, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
