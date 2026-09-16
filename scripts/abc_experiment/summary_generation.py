"""Does the summary help recover what the originals dropped?

The recognition exam and the deterministic containment checks both say the summary is
redundant on sessions where nothing is dropped. This asks the question where a summary
could actually earn its cost: *generation* of facts that the originals no longer carry.

Protocol, per boundary and window:

  * compress with C, with and without the summary (same code path, same projection budget)
  * find the distinctive values that lived in assistant blocks the projection dropped
  * ask the model to list every exact identifier, path, version and number it can find
  * score how many of those dropped-specific values it produces

This is generation, not recognition: the question does not name the values, so a projection
that lost them cannot recover them by matching. Scoring is exact string containment, so no
model judge is involved.
"""
import argparse, json, pathlib, random, re, subprocess, sys, time

WT = pathlib.Path("/Users/geilige/future-os/.worktrees/session-history-a47313")
sys.path.insert(0, str(WT / "scripts"))
sys.path.insert(0, str(WT / "scripts/abc_experiment"))
from abc_compaction_experiment import Ledger, request_reserve, parse_sse

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
DRIVER = pathlib.Path("/Users/geilige/future-os/target/debug/examples/abc_c3_probe")
FROZEN = ROOT / "frozen-sessions"
FACTIONS = (0.4, 0.7, 1.0)

# Values worth asking for: identifiers, versions, paths, sizes, counts. Prose is excluded
# because a question like this cannot ask for a sentence without leaking it.
VALUE = re.compile(
    r"\b(?:"
    r"[0-9a-f]{6,40}|"                       # hashes and ids
    r"ver_[0-9a-f]+|"
    r"RSN_[A-Za-z_]+|"
    r"ACK_[0-9a-f]+|"
    r"trace_[0-9a-f]+|"
    r"[\w./-]+\.(?:rs|ts|py|md|json|toml|log|mjs|tsx)|"   # file paths
    r"\d+(?:\.\d+)?\s?(?:MiB|MB|GiB|GB|KB|B)|"            # sizes
    r"\d[\d,]{2,}\s?(?:项|个|条|次|tests?)"                # counts
    r")\b")


def covered(name, index):
    manifest = json.loads((FROZEN / "manifest.json").read_text())
    records = json.loads(pathlib.Path(manifest[name]["path"]).read_text())["records"]
    return records[:max(1, int(len(records) * FACTIONS[index]))]


def compress(records, window, summary, label):
    tmp = ROOT / "gen" / f"{label}-{window}-{int(summary)}.json"
    tmp.parent.mkdir(parents=True, exist_ok=True)
    tmp.write_text(json.dumps(records, ensure_ascii=False) + "\n")
    argv = [str(DRIVER), "--records", str(tmp), "--model", "future/deepseek-flash",
            "--window", str(window)]
    if not summary:
        argv.append("--no-summary")
    out = subprocess.run(argv, capture_output=True, text=True, timeout=1200)
    if out.returncode != 0:
        return None
    return json.loads(out.stdout.strip().splitlines()[-1])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--window", type=int, default=32_000)
    ap.add_argument("--bridge", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, default=10.0)
    ap.add_argument("--chains", nargs="*", default=["real-yt", "real-visual", "real-stream"])
    args = ap.parse_args()

    ledger = Ledger(ROOT, args.budget)
    ledger.path = ROOT / f"gen-{args.window}-calls.json"
    ledger.rows = json.loads(ledger.path.read_text()) if ledger.path.exists() else []

    def ask(identity, prompt, attempts=3):
        prior = next((r for r in ledger.rows if r["id"] == identity
                      and r.get("state") == "finished" and r.get("text")), None)
        if prior:
            return prior["text"]
        for attempt in range(attempts):
            body = {"model": args.model, "stream": True, "max_tokens": 4096,
                    "stream_options": {"include_usage": True},
                    "messages": [{"role": "user", "content": prompt}]}
            row = ledger.reserve(identity + ("" if attempt == 0 else f"__retry{attempt}"),
                                 args.model, request_reserve(len(json.dumps(body).encode()), 4096))
            began = time.monotonic()
            out = subprocess.run([str(args.bridge)],
                                 input=json.dumps({"model": args.model, "body": body}),
                                 capture_output=True, text=True, timeout=1200)
            if out.returncode != 0:
                ledger.settle(row, error=f"exit {out.returncode}: {out.stderr[-200:]}")
                continue
            parsed = parse_sse(out.stdout)
            usage = parsed.get("usage") or {}
            ledger.settle(row, input_tokens=usage.get("prompt_tokens"),
                          output_tokens=usage.get("completion_tokens"),
                          credit_cost=usage.get("credit_cost"),
                          seconds=round(time.monotonic() - began, 3),
                          error=parsed.get("error"), text=parsed["text"], finish=parsed.get("finish"))
            if (parsed.get("text") or "").strip():
                return parsed["text"]
        return ""

    INSTRUCTION = (
        "You are continuing work on an engineering session. From the material available to "
        "you, list every EXACT identifier, file path, version, size and count you can find, "
        "one per line. Do not summarise prose and do not explain. Only list values that are "
        "actually present in your material.")

    results = []
    for chain in args.chains:
        for index in range(3):
            records = covered(chain, index)
            phrase_values = set()
            for r in records:
                if r["kind"] == "text" and r["role"] == "assistant":
                    phrase_values |= set(VALUE.findall(r.get("text", "")))
            for use_summary in (True, False):
                identity = f"{chain}__s{index}__{'s' if use_summary else 'n'}"
                payload = compress(records, args.window, use_summary, identity)
                if payload is None:
                    continue
                proj = "\n\n".join(m["text"] for m in payload["projection"])
                missing = sorted(v for v in phrase_values if v not in proj)
                if not missing:
                    continue
                answer = ask(f"gen__{identity}__r", proj + "\n\n" + INSTRUCTION)
                listed = set(VALUE.findall(answer or ""))
                recovered = sum(1 for v in missing if v in (answer or ""))
                # Control: values that ARE in the projection. Without it a low recovery
                # rate could mean the task is impossible rather than the summary unhelpful.
                available = sorted(v for v in phrase_values if v in proj)
                found_present = sum(1 for v in available if v in (answer or ""))
                results.append({"chain": chain, "stage": index,
                                "arm": "summary" if use_summary else "no-summary",
                                "missing": len(missing), "recovered": recovered,
                                "available": len(available), "found_present": found_present,
                                "listed": len(listed), "proj_tokens": len(proj) // 4})
                print(f'  {chain:12s} s{index} {"with summary" if use_summary else "no summary":13s} '
                      f'missing {len(missing):>3d} recovered {recovered:>3d} '
                      f'| present {len(available):>3d} found {found_present:>3d} '
                      f'({100*found_present/max(len(available),1):.0f}%)  proj {len(proj)//4}tok',
                      flush=True)

    out = ROOT / f"gen-{args.window}-results.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print()
    for arm in ("summary", "no-summary"):
        sel = [r for r in results if r["arm"] == arm]
        m = sum(r["missing"] for r in sel)
        rec = sum(r["recovered"] for r in sel)
        av = sum(r["available"] for r in sel)
        fp = sum(r["found_present"] for r in sel)
        print(f'  {arm:12s} dropped {m:>5d} recovered {rec:>3d} ({100*rec/max(m,1):5.1f}%)   '
              f'| present {av:>5d} found {fp:>4d} ({100*fp/max(av,1):5.1f}%)')
    print(f'ledger: {len(ledger.rows)} calls, CNY '
          f'{sum(r.get("charged", r["reserved"]) for r in ledger.rows):.4f}')


if __name__ == "__main__":
    main()
