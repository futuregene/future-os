#!/usr/bin/env python3
"""Closed-book retention comparison at production call shape.

What this run is, and how it differs from the earlier ones:

* **The Rust arms are called the way the runtime calls them.** The window comes from the
  model registry, the output reservation from `effective_max_tokens`, `set_request_budget`
  runs with the session's system prompt and the real tool definitions, and the trigger and
  phase are the ones an ordinary pre-turn compaction uses. The earlier runs passed a
  simulated 128K window, no budget and an empty tool list, so their budgets were not the
  ones production computes.
* **The system prompt is captured, not invented.** `capture_shape.py` records what a real
  turn sends (see `shape/`), and the driver derives the budget prompt and the outgoing
  prompt from it through the same function production uses. A frozen session's own prompt is
  rebuilt per turn from its working directory and is not in the journal, so it cannot be
  replayed; this is a real production prompt standing in for it.
* **Compaction cadence follows the production trigger.** New history accumulates until the
  economic trigger is reached, instead of an arbitrary 64K. The boundary set itself stays
  forced at the three scored points: this compares retention rules, not trigger timing.
* **It adds the deployed strategy as an arm.** `main` runs the algorithm the released code
  contains (a recursive model summary with protected originals and a recent tail), built from
  that checkout with `abc_main_probe.rs`, so "is the new strategy better than what ships" is
  answerable from the same data.

Closed book only. Every model call is reserved in a ledger before it is sent and settled from
the provider's reported usage afterwards.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path[:0] = [str(HERE), str(HERE.parent)]
import four_arm_rerun as base  # noqa: E402  inputs, questionnaire, schedule, Calls, probe
import fidelity_rerun as fid  # noqa: E402  corrected Codex budgeting + AI SDK lowering
import abc_external_strategies as ext  # noqa: E402
import realistic_exam as exam  # noqa: E402

ARMS = ("deterministic", "summarized", "main", "codex", "opencode")
SEED = 20260917
OUTPUT_TOKENS = 8192
# The exam's own system prompt, identical for every arm; the compaction policies' prompts
# belong to the policies and are therefore different by design.
SYSTEM = base.SYSTEM

load, save, sha, immutable = base.load, base.save, base.sha, base.immutable


def limits(driver, model):
    """What the registry and the budget rules imply, straight from the driver."""
    raw = subprocess.check_output([str(driver), "--model", model, "--print-limits"], text=True)
    return json.loads(raw.strip().splitlines()[-1])


def safe_cuts(records, cuts):
    """Move scored boundaries back to a safe endpoint, then re-check.

    A boundary may not separate a tool call from its result, and may not fall inside one
    stored message. `four_arm_rerun.schedule` enforces this for the chunk-based boundaries
    but appends the forced scored cuts unconditionally, which is how a cut lands mid-pair
    and the shared normalizer then (correctly) refuses it. This is the rule
    `fidelity_rerun.safe_plan` applies; applied here so both cadences obey it.
    """
    def position(record, index):
        return record.get("position", record.get("order", index))

    safe = []
    pending = set()
    for index, record in enumerate(records):
        if record["kind"] == "tool_call":
            pending.add(record["call"])
        elif record["kind"] == "tool_result":
            pending.discard(record["call"])
        end = index + 1
        complete = end == len(records) or position(record, index) != position(records[end], end)
        if complete and not pending:
            safe.append(end)
    moved = [max(s for s in safe if s <= cut) for cut in cuts]
    if len(set(moved)) != len(set(cuts)):
        raise ValueError(f"scored boundaries collapsed onto one endpoint: {moved}")
    return moved


def sdk_select(messages, sdk_root, window, max_output):
    """Tail selection from the pinned AI SDK, at this run's window."""
    request = {"messages": messages, "sdkRoot": str(sdk_root),
               "window": window, "maxOutput": max_output}
    raw = subprocess.check_output(
        ["node", str(HERE / "opencode_fidelity.mjs")], input=json.dumps(request), text=True)
    return json.loads(raw)


def shape_paths(shape):
    return shape / "system-prompt.txt", shape / "tools.json"


def project(calls, args, arm, records, fresh, previous, identity):
    """Produce one arm's projection for the history up to this boundary."""
    path = calls.root / "inputs" / f"{sha(records)}.json"
    immutable(path, records)
    checkpoint = previous.get("checkpoint") if previous else None

    if arm in ("deterministic", "summarized"):
        prompt_file, tools_file = shape_paths(args.shape)
        # `manual` bypasses the economic trigger, so a forced run compacts at every
        # boundary exactly like the external arms do; `automatic` is what production does.
        trigger = "manual" if args.force_compaction else "automatic"
        argv = [str(args.driver), "--records", str(path), "--model", calls.model,
                "--strategy", arm, "--trigger", trigger, "--phase", "preturn",
                "--thinking-level", "off",
                "--base-system-prompt-file", str(prompt_file),
                "--tools-file", str(tools_file),
                "--session-id", f"experiment-{identity.split('-')[0]}"]
        if checkpoint:
            argv += ["--previous", json.dumps(checkpoint)]

        def parse(stdout):
            p = json.loads(stdout.strip().splitlines()[-1])
            p["text"] = "\n\n".join(m["text"] for m in p["projection"])
            p["summary_used"] = (
                (p.get("checkpoint") or {}).get("algorithm_version") == "summarized-evidence-v1"
            )
            # A model-free strategy reports no usage; the summarised one reports the real
            # charge, and a failed summary call falls back and bills nothing.
            usage = p["usage"]
            cost = usage["cost"] if usage["input_tokens"] or p["model_requests"] == 0 else None
            return p, cost

        request = {"arm": arm, "records_sha256": sha(records), "previous": checkpoint,
                   "trigger": trigger,
                   "shape": sha(Path(shape_paths(args.shape)[0]).read_bytes())}
        reserve = 0 if arm == "deterministic" else 3 * (args.window * 5 + args.output_tokens * 20) / 1e6
        return calls.execute(identity, request, argv, reserve, parse)

    if arm == "main":
        # Production compacts PreTurn on the first turn and MidTurn afterwards; the
        # difference changes whether a whole active turn may be covered.
        first = previous is None
        argv = [str(args.main_driver), "--records", str(path), "--model", calls.model,
                "--thinking-level", "off",
                "--trigger", "manual" if args.force_compaction else "automatic",
                "--phase", "standalone" if args.force_compaction
                else ("preturn" if first else "midturn")]
        if checkpoint:
            argv += ["--previous", json.dumps(checkpoint)]

        def parse_main(stdout):
            p = json.loads(stdout.strip().splitlines()[-1])
            p["text"] = "\n\n".join(m["text"] for m in p["projection"])
            usage = p["usage"]
            cost = usage["cost"] if usage["input_tokens"] or p["model_requests"] == 0 else None
            return p, cost

        request = {"arm": arm, "records_sha256": sha(records), "previous": checkpoint,
                   "force": args.force_compaction}
        return calls.execute(identity, request, argv, 3 * (args.window * 5 + args.output_tokens * 20) / 1e6,
                             parse_main)

    if arm == "codex":
        # Codex's own policy: its pinned base instructions, its recursive live state, and
        # its byte-based user protection with middle truncation (v3's corrected path).
        live = (previous["live"] if previous else []) + fresh
        messages = [{"role": "system", "content": (args.sources / "codex-base.md").read_text()},
                    *fid.to_chat(live),
                    {"role": "user", "content": (args.sources / "codex-compact.md").read_text()}]
        p = calls.model_call(identity, messages, cap=args.output_tokens)
        summary = p["text"].strip()
        if not summary:
            raise RuntimeError(f"empty Codex summary: {identity}")
        kept = fid.codex_keep(live, summary)
        return {"text": fid.render(kept), "live": kept, "summary": summary,
                "finish": p.get("finish"), "usage": p.get("usage")}

    # OpenCode's own policy: its dedicated summary system prompt, and tail selection
    # computed by the pinned AI SDK rather than a hand-written serializer.
    live = (previous["live"] if previous else []) + fresh
    cap = min(args.output_tokens, 32000) or 32000
    selection = sdk_select(live, args.sdk_root, args.window, cap)
    prompt = ext.opencode_summary_request(selection["head"],
                                         previous.get("summary") if previous else None)
    p = calls.model_call(identity, [{"role": "system", "content": (args.sources / "opencode-system.txt").read_text()},
                                    {"role": "user", "content": prompt}], cap=cap)
    summary = p["text"].strip()
    if not summary:
        raise RuntimeError(f"empty OpenCode summary: {identity}")
    tail = selection["tailMessages"]
    return {"text": "[Context compaction summary]: " + summary + "\n\n" + fid.render(tail),
            "live": tail, "summary": summary, "finish": p.get("finish"),
            "usage": p.get("usage"), "max_output": cap, "selection": selection}


def ledger_costs(root):
    """Compaction spend separated from scoring spend, per arm and in total.

    The two are not the same thing: scoring is what the exam costs every arm equally,
    while compaction is what the retention policy costs to run. Reporting one number would
    hide the strategy that summarises with a model behind the one that does not.
    """
    path = root / "ledger.json"
    rows = load(path) if path.exists() else {}
    out = {"compaction": {}, "scoring": {}, "compaction_calls": {}, "scoring_calls": {}}
    for row in rows.values():
        identity = row.get("identity", "")
        cost = row.get("charged", row["reserved"])
        if identity.endswith("-compact"):
            bucket, calls = "compaction", "compaction_calls"
        elif identity.endswith("-closed"):
            bucket, calls = "scoring", "scoring_calls"
        else:
            continue
        arm = identity.rsplit("-", 2)[-2]
        out[bucket][arm] = out[bucket].get(arm, 0.0) + cost
        out[calls][arm] = out[calls].get(arm, 0) + 1
    out["compaction_total"] = sum(out["compaction"].values())
    out["scoring_total"] = sum(out["scoring"].values())
    return out


# Which arm's summary request can reuse the prefix the session's last turn sent.
#
# A prefix is compared from token zero, so a strategy that substitutes its own summariser
# prompt cannot share the session's prefix at all — however small its request is. That makes
# this a structural property of the request each strategy builds, readable from the code,
# and not something these runs can measure: the recorded cache counters are contaminated by
# arm and run ordering (see PRODUCTION_SHAPE_PROTOCOL.md).
CACHE_ELIGIBLE = {
    "deterministic": (False, "no model call at all"),
    "summarized": (True, "passes the session's own system prompt and tool definitions and the "
                         "live conversation as real messages; the deployed path measured "
                         "99.8 % cache read in production"),
    "main": (False, "substitutes its own SUMMARY_SYSTEM_PROMPT constant, so token 0 differs "
                    "from every request the session sent"),
    "codex": (True, "reuses its base instructions and appends its instruction last; in its "
                    "own deployment those are the session's system prompt, though this "
                    "harness supplies a stand-in"),
    "opencode": (False, "sends a dedicated compaction system prompt, which cannot share the "
                        "session's prefix"),
}
# Modelled hit rate for an eligible request. The deployed path measured 99.8 %
# (212 548 of 212 911 tokens) on an isolated agent; 98 % is the conservative stand-in.
MODELLED_CACHE_HIT = 0.98


def bill(rates, prompt, completion, cache_read):
    """`Cost::estimate`: prompt already includes the cached subset."""
    uncached = max(prompt - cache_read, 0)
    return (uncached * rates["input"] + cache_read * rates["cache_read"]
            + completion * rates["output"]) / 1e6


def cache_cost(root, rates, hit=MODELLED_CACHE_HIT):
    """Reprice the recorded compaction calls cold, and with a cache-served prefix.

    Both views are reported. The cold figure is what the ledger actually charged and is the
    upper bound; the modelled figure is what a request that reuses the session's prefix
    costs, which is the number the cache-friendliness argument is about.
    """
    ledger = load(root / "ledger.json") if (root / "ledger.json").exists() else {}
    # Seed every arm: a model-free strategy has no calls to price, and reporting it as
    # absent would read as "unknown" rather than "free".
    per_arm = {arm: {"calls": 0, "prompt": 0, "completion": 0, "cold": 0.0, "cached": 0.0}
               for arm in ARMS}
    for row in ledger.values():
        identity = row.get("identity", "")
        if not identity.endswith("-compact"):
            continue
        artifact = row.get("artifact")
        if not artifact:
            continue
        try:
            response = load(root / artifact)
        except (OSError, json.JSONDecodeError):
            continue
        usage = response.get("usage") or {}
        try:
            # The external arms report provider usage; the Rust drivers report the aggregate
            # their wrapper observed. Same quantities, different keys.
            prompt = int(usage.get("prompt_tokens") or usage.get("input_tokens") or 0)
            completion = int(usage.get("completion_tokens") or usage.get("output_tokens") or 0)
        except (TypeError, ValueError):
            continue
        if prompt == 0:
            continue
        arm = identity.rsplit("-", 2)[-2]
        entry = per_arm.setdefault(arm, {"calls": 0, "prompt": 0, "completion": 0,
                                         "cold": 0.0, "cached": 0.0})
        entry["calls"] += 1
        entry["prompt"] += prompt
        entry["completion"] += completion
        entry["cold"] += bill(rates, prompt, completion, 0)
        eligible = CACHE_ELIGIBLE.get(arm, (False, ""))[0]
        entry["cached"] += (bill(rates, prompt, completion, int(prompt * hit))
                            if eligible else bill(rates, prompt, completion, 0))
    for arm, entry in per_arm.items():
        eligible, why = CACHE_ELIGIBLE.get(arm, (False, "unknown"))
        entry["cache_eligible"] = eligible
        entry["reason"] = why
        entry["cold_cny"] = round(entry.pop("cold"), 6)
        entry["cached_cny"] = round(entry.pop("cached"), 6)
    return {"assumed_hit": hit, "arms": per_arm}


def compacted(arm, projection):
    """Did this arm actually replace the history with a projection?

    The Rust arms report a checkpoint, which is the direct evidence. Codex's policy always
    summarises at a boundary; OpenCode does too unless it cannot, which surfaces as a refused
    call rather than as a missing checkpoint.
    """
    if arm in ("deterministic", "summarized", "main"):
        return bool(projection.get("checkpoint"))
    if arm == "codex":
        return not projection.get("refused")
    return not projection.get("refused")


def stage_end(root, chain, stage):
    """The record index a scored stage sits at, from the schedule."""
    boundaries = load(root / "schedule.json")[chain]
    cut = load(root / "corpus" / f"{chain}.json")["cuts"][stage]
    return max(b for b in boundaries if b <= cut)


def report(root, chains):
    config = load(root / "manifest.json")
    rows = [load(p) for p in sorted((root / "scores").glob("*.json"))]
    projections = [load(p) for p in (root / "projections").glob("*-compact.json")]
    costs = ledger_costs(root)
    expected = len(chains) * 3 * len(ARMS)

    # One row per (chain, boundary): the shared input size, and what each arm sent instead.
    per_boundary = []
    for chain in chains:
        ends = sorted({p["end"] for p in projections if p["chain"] == chain})
        for end in ends:
            arms = {}
            raw = None
            for arm in ARMS:
                p = next((p for p in projections
                          if p["chain"] == chain and p["arm"] == arm and p["end"] == end), None)
                if p is None:
                    continue
                raw = raw or p.get("estimated_before")
                after = p.get("projection_tokens")
                arms[arm] = {
                    "compacted": compacted(arm, p),
                    "projection_tokens": after,
                    "ratio": (after / raw) if raw and after else None,
                }
            for arm in ARMS:
                if arm not in arms:
                    continue
                matches = [r for r in rows if r["chain"] == chain and r["arm"] == arm
                           and stage_end(root, chain, r["stage"]) == end]
                if matches:
                    arms[arm]["hits"] = matches[0]["hits"]
                    arms[arm]["of_present"] = matches[0]["of_present"]
                    arms[arm]["false_positives"] = matches[0]["false_positives"]
            per_boundary.append({"chain": chain, "end": end, "raw_tokens": raw, "arms": arms})

    def aggregate(selected):
        """Aggregate a set of boundaries, restricted to those where the arm compacted.

        A boundary where an arm sent its whole history is excluded from the token and ratio
        figures — including it is exactly how "no compaction" masquerades as the best
        retention policy — but its quality is still counted, and `boundaries_total` and
        `compacted` record the difference.
        """
        out = {}
        for arm in ARMS:
            all_xs = [b["arms"][arm] for b in selected if arm in b["arms"]]
            xs = [x for x in all_xs if x.get("compacted")]
            if not all_xs:
                continue
            scored = [x for x in all_xs if x.get("hits") is not None]
            ratios = [x["ratio"] for x in xs if x.get("ratio")]
            out[arm] = {
                "boundaries_total": len(all_xs),
                "compacted": len(xs),
                "quality_boundaries": len(scored),
                "hits": sum(x["hits"] for x in scored),
                "of_present": sum(x["of_present"] for x in scored),
                "false_positives": sum(x["false_positives"] for x in scored),
                "median_projection_tokens": statistics.median(
                    x["projection_tokens"] for x in xs if x["projection_tokens"]),
                "median_compression_ratio": statistics.median(ratios) if ratios else None,
                "compaction_calls": costs["compaction_calls"].get(arm, 0),
                "compaction_cny": round(costs["compaction"].get(arm, 0.0), 6),
                "scoring_cny": round(costs["scoring"].get(arm, 0.0), 6),
            }
        return out

    refusal = {}
    for arm in ARMS:
        xs = [r for r in rows if r["arm"] == arm]
        refusal[arm] = {"scored": len(xs), "invalid": sum(not x["valid_answer"] for x in xs),
                        "refused": sum(1 for x in xs if x.get("refused"))}

    # Cache-aware cost. The cold figure is what the ledger charged; the modelled one is what
    # a request reusing the session's prefix costs. The projection itself is re-sent every
    # later turn, so it is priced per turn as well — that recurring cost is what a smaller
    # projection actually buys down.
    rates = config["limits"].get("rates_per_million",
                                 {"input": 1.0, "output": 4.0,
                                  "cache_read": 0.02, "cache_write": 0.0})
    cache = cache_cost(root, rates)
    # The baseline the per-turn column is read against: sending no projection at all, i.e.
    # the raw history. Without it the recurring figure has nothing to compare to.
    raw_sizes = sorted(b["raw_tokens"] for b in per_boundary if b.get("raw_tokens"))
    if raw_sizes:
        raw_median = statistics.median(raw_sizes)
        cache["no_compaction_baseline"] = {
            "median_raw_tokens": raw_median,
            "per_turn_cny": round(
                bill(rates, raw_median, 0, int(raw_median * MODELLED_CACHE_HIT)), 6),
            "per_hundred_turns_cny": round(
                bill(rates, raw_median, 0, int(raw_median * MODELLED_CACHE_HIT)) * 100, 4),
        }
    for arm, entry in cache["arms"].items():
        toks = sorted(p["projection_tokens"] for p in projections
                      if p["arm"] == arm and p.get("projection_tokens"))
        if toks:
            median = statistics.median(toks)
            entry["median_projection_tokens"] = median
            entry["per_turn_cny"] = round(
                bill(rates, median, 0, int(median * MODELLED_CACHE_HIT)), 6)
            entry["per_hundred_turns_cny"] = round(entry["per_turn_cny"] * 100, 4)
            baseline = cache.get("no_compaction_baseline", {}).get("per_turn_cny")
            if baseline and entry["per_turn_cny"]:
                entry["per_turn_vs_no_compaction"] = round(baseline / entry["per_turn_cny"], 1)
    cache["rates_per_million"] = rates

    # The headline comparison the question asks for: post-compaction against
    # post-compaction, so only boundaries where every arm committed a checkpoint.
    fair = [b for b in per_boundary if all(b["arms"].get(a, {}).get("compacted") for a in ARMS)]
    result = {
        "complete": len(rows) == expected,
        "expected": expected,
        "scored": len(rows),
        "call_shape": config.get("call_shape"),
        "force_compaction": config.get("force_compaction"),
        "limits": config.get("limits"),
        "chunk_tokens": config.get("chunk_tokens"),
        "all_boundaries": aggregate(per_boundary),
        "refusals": refusal,
        "fair_boundaries": len(fair),
        "fair_arms": aggregate(fair),
        "per_boundary": per_boundary,
        "cache_cost": cache,
        "compaction_spend_cny": round(costs["compaction_total"], 6),
        "scoring_spend_cny": round(costs["scoring_total"], 6),
        "spent_or_reserved": round(costs["compaction_total"] + costs["scoring_total"], 6),
    }
    save(root / "report.json", result)
    print(json.dumps({k: v for k, v in result.items() if k != "per_boundary"}, indent=2),
          flush=True)
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--source", type=Path, default=Path.home() / "compact-exp")
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--driver", type=Path, required=True, help="abc_strategy_probe binary")
    ap.add_argument("--main-driver", type=Path, required=True, help="abc_main_probe binary")
    ap.add_argument("--main-commit", required=True,
                    help="the commit the --main-driver was built from (`git log -1` in that "
                         "checkout). The harness cannot infer it from a binary path, and without "
                         "it the run's `main` arm is not reproducible.")
    ap.add_argument("--bridge", type=Path, required=True)
    ap.add_argument("--shape", type=Path, required=True, help="capture_shape.py output")
    ap.add_argument("--sources", type=Path, default=Path.home() / "compact-exp" / "fidelity-sources")
    ap.add_argument("--sdk-root", type=Path, default=Path.home() / "compact-exp" / "fidelity-sdk")
    ap.add_argument("--model", default="future/deepseek-flash")
    ap.add_argument("--budget", type=float, required=True,
                    help="total admission ceiling, including spend already recorded by "
                         "earlier runs (--prior-spend)")
    ap.add_argument("--prior-spend", type=float, default=41.1569124,
                    help="CNY already spent by the v2 and v3 runs; the ceiling covers both")
    ap.add_argument("--output-tokens", type=int, default=OUTPUT_TOKENS,
                    help="generation cap for the summary requests")
    ap.add_argument("--chains", nargs="+",
                    default=["export", "analysis", "pipeline", "real-yt", "real-visual", "real-stream"])
    ap.add_argument("--force-compaction", action="store_true",
                    help="force every arm to compact at every boundary (manual trigger for "
                         "the Rust arms), so the comparison is post-compaction against "
                         "post-compaction instead of mixed with un-compacted history")
    ap.add_argument("--prepare-only", action="store_true")
    ap.add_argument("--report-only", action="store_true")
    args = ap.parse_args()
    args.output = args.output.resolve()
    args.driver = args.driver.resolve()
    args.main_driver = args.main_driver.resolve()
    args.bridge = args.bridge.resolve()
    args.shape = args.shape.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if args.report_only:
        report(args.output, args.chains)
        return

    limit = limits(args.driver, args.model)
    args.window = limit["window"]
    # Cadence: new history accumulates until the economic trigger, as in production.
    chunk = limit["effective_trigger"]
    data = base.inputs(args.source, args.chains)
    code_paths = [Path(__file__), HERE / "four_arm_rerun.py", HERE / "fidelity_rerun.py",
                  HERE / "opencode_fidelity.mjs", HERE / "capture_shape.py",
                  HERE.parent / "abc_external_strategies.py", HERE / "realistic_exam.py",
                  HERE.parent / "abc_compaction_experiment.py"]
    config = {
        "version": 4,
        "seed": SEED,
        "call_shape": "production",
        "force_compaction": args.force_compaction,
        "limits": limit,
        "chunk_tokens": chunk,
        "output_tokens": args.output_tokens,
        "model": args.model,
        "budget": args.budget,
        "arms": list(ARMS),
        "chains": args.chains,
        "input_hashes": {n: sha(d) for n, d in data.items()},
        "code_hashes": {str(p.relative_to(REPO)): sha(p.read_bytes()) for p in code_paths},
        "driver_sha256": sha(args.driver.read_bytes()),
        "main_driver_sha256": sha(args.main_driver.read_bytes()),
        "main_commit": args.main_commit,
        "bridge_sha256": sha(args.bridge.read_bytes()),
        "shape": json.loads((args.shape / "shape.json").read_text()),
        "shape_sha256": sha(args.shape.joinpath("system-prompt.txt").read_bytes()),
        "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        "prior_spend": args.prior_spend,
        "sampling": "one draw per arm/boundary; no significance claim",
        "open_book": "not run; closed-book only",
    }
    immutable(args.output / "manifest.json", config)
    fingerprint = sha(config)
    calls = fid.Calls(args.output, args.budget, args.model, args.bridge, fingerprint,
                      previous_spend=args.prior_spend)
    jobs = {}
    for chain, d in data.items():
        cuts = safe_cuts(d["records"], d["cuts"])
        d["cuts"] = cuts
        immutable(args.output / "corpus" / f"{chain}.json", d)
        ends = base.schedule(d["records"], cuts, chunk=chunk)
        jobs[chain] = ends
        for stage, cut in enumerate(cuts):
            immutable(args.output / "questions" / f"{chain}-{stage}.json",
                      base.questionnaire(d["records"][:cut], stage))
    immutable(args.output / "schedule.json", jobs)
    print(json.dumps({"limits": limit, "chunk_tokens": chunk,
                      "boundaries": {n: len(v) for n, v in jobs.items()},
                      "paid_compactions": sum(map(len, jobs.values())) * 4}), flush=True)
    if args.prepare_only:
        return
    rng = random.Random(SEED)
    for chain, d in data.items():
        states = {a: None for a in ARMS}
        consumed = 0
        for step, end in enumerate(jobs[chain]):
            fresh = fid.normalize(args.output, args.driver, calls.model,
                                  d["records"][consumed:end])
            arms = list(ARMS)
            rng.shuffle(arms)
            for arm in arms:
                identity = f"{chain}-{step}-{arm}-compact"
                path = args.output / "projections" / f"{identity}.json"
                if path.exists():
                    p = load(path)
                else:
                    # The Rust arms replay the stored records; the external arms need the
                    # provider-shaped message array the shared normalizer produces, which is
                    # also where any text/tool loss would be caught before it is paid for.
                    try:
                        p = project(calls, args, arm, d["records"][:end], fresh,
                                    states[arm], identity)
                    except Exception as error:
                        # A policy that refuses to compact has told us something: the
                        # deployed algorithm hard-fails when the model's summary does not
                        # match its required structure. Record it and score the empty
                        # projection rather than losing the boundary.
                        p = {"text": "", "refused": True, "error": str(error)[:400],
                             "usage": {}, "model_requests": 0}
                    p.update(input_sha256=sha(d["records"][:end]), previous_sha256=sha(states[arm]),
                             arm=arm, chain=chain, end=end)
                    # One yardstick for every arm: the harness's estimator over the text
                    # the next request carries. The Rust arms additionally report the
                    # planner's own estimate in `estimated_after`.
                    p["projection_tokens"] = base.tokens(p["text"])
                    immutable(path, p)
                if p["input_sha256"] != sha(d["records"][:end]) or p["previous_sha256"] != sha(states[arm]):
                    raise ValueError("projection chain mismatch")
                states[arm] = p
            consumed = end
            if end in d["cuts"]:
                stage = d["cuts"].index(end)
                q = load(args.output / "questions" / f"{chain}-{stage}.json")
                arms = list(ARMS)
                rng.shuffle(arms)
                for arm in arms:
                    identity = f"{chain}-{stage}-{arm}-closed"
                    path = args.output / "scores" / f"{identity}.json"
                    if not path.exists():
                        sc = base.probe(calls, identity, states[arm], q)
                        immutable(path, dict(sc, chain=chain, stage=stage, arm=arm))
                    elif load(path)["projection_sha256"] != sha(states[arm]["text"]):
                        raise ValueError("score/projection mismatch")
                report(args.output, args.chains)
    report(args.output, args.chains)


if __name__ == "__main__":
    main()
