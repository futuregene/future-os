"""Durable baseline records, outcome-selected challenges and single-arm execution."""

import shutil
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from datasets import ROOT, digest, load_frozen_questions, read_json, write_json
from future_runner import BASE_MODEL, BASE_THINKING, prompt_for
from scoring import score


def load_baseline(directory, complete=True, verify_grades=True):
    directory = Path(directory)
    manifest = read_json(directory / "manifest.json")
    if (directory / "INVALID.md").exists():
        raise ValueError("Invalid baseline exercise")
    if (
        manifest.get("arms") != ["single"]
        or manifest["repeats"] != 1
        or manifest.get("execution_profile") != "daily"
        or manifest["configuration"]["baseline"]
        != {"model": BASE_MODEL, "thinking_level": BASE_THINKING}
    ):
        raise ValueError("Requires a one-pass DeepSeek Flash max single-arm baseline")
    suite = manifest["suite"]
    if suite["sources_lock_sha256"] != digest(read_json(ROOT / "sources.lock.json")):
        raise ValueError("Baseline source lock changed")
    # Transport/retry code can evolve while the saved answers remain the baseline.
    # Verify the actual recorded prompt below, rather than the entire runner file.
    for name in ("scoring.py",) if verify_grades else ():
        if manifest["harness_sha256"].get(name) != digest((ROOT / name).read_bytes()):
            raise ValueError("Baseline prompt/scoring implementation changed: " + name)
    # Validate the saved question snapshot without depending on a mutable suite path.
    bundle = read_json(directory / "frozen-questions.json")
    if (
        digest((directory / "frozen-questions.json").read_bytes())
        != manifest["question_bundle_sha256"]
    ):
        raise ValueError("Baseline question snapshot hash mismatch")
    if bundle["suite_sha256"] != digest(suite):
        raise ValueError("Baseline suite mismatch")
    cases = {row["id"]: row for row in bundle["cases"]}
    planned = {row["id"]: row for row in suite["cases"]}
    if len(cases) != len(bundle["cases"]) or set(cases) != set(planned):
        raise ValueError("Baseline question IDs mismatch")
    for identifier, case in cases.items():
        if any(
            digest(case[key]) != planned[identifier][key + "_sha256"]
            for key in ("question", "answer")
        ):
            raise ValueError("Baseline question content mismatch")
    rows = {}
    for path in sorted(directory.glob("r*-*/result.json")):
        row = read_json(path)
        identifier = row["case_id"]
        if (
            identifier not in cases
            or identifier in rows
            or row["arm"] != "single"
            or not path.parent.name.startswith("r0-")
        ):
            raise ValueError("Unexpected or duplicate baseline result")
        if row["status"] == "completed":
            if (
                row.get("effective_parent_model") != BASE_MODEL
                or row.get("effective_parent_thinking") != BASE_THINKING
            ):
                raise ValueError("Baseline effective model/effort mismatch")
            if verify_grades and row["grade"] != score(cases[identifier], row["text"]):
                raise ValueError("Baseline score differs from frozen answer/scorer")
            if (path.parent / "prompt.txt").read_text(encoding="utf-8") != prompt_for(
                cases[identifier]
            ):
                raise ValueError("Baseline prompt differs from current protocol")
        rows[identifier] = (row, path)
    if complete and set(rows) != set(cases):
        raise ValueError(f"Baseline is incomplete: {len(rows)}/{len(cases)} results")
    return manifest, bundle, cases, rows


def regrade_baseline(source, out):
    """A new, traceable grade revision; never replace the original run evidence."""
    source, out = Path(source), Path(out)
    manifest, _, cases, rows = load_baseline(
        source, complete=False, verify_grades=False
    )
    if out.exists():
        raise ValueError("Regraded output already exists")
    shutil.copytree(source, out)
    shutil.copyfile(source / "manifest.json", out / "manifest-before-regrading.json")
    (out / "harness").rename(out / "harness-before-regrading")
    harness = out / "harness"
    harness.mkdir()
    files = list(ROOT.glob("*.py")) + [
        ROOT / "requirements.txt",
        ROOT / "sources.lock.json",
    ]
    for path in files:
        shutil.copyfile(path, harness / path.name)
    manifest["harness_sha256"] = {
        path.name: digest(path.read_bytes()) for path in files
    }
    manifest["regrading"] = {
        "source_manifest_sha256": digest(read_json(source / "manifest.json")),
        "reason": "accept equivalent ZebraLogic JSON object rows; no question or model output changes",
        "original_result_sha256": {},
    }
    for identifier, (row, path) in rows.items():
        destination = out / path.relative_to(source)
        shutil.copyfile(path, destination.with_name("result-before-regrading.json"))
        manifest["regrading"]["original_result_sha256"][identifier] = digest(
            path.read_bytes()
        )
        if row["status"] == "completed":
            row["grade"] = row["outer_grade"] = score(cases[identifier], row["text"])
        write_json(destination, row)
    write_json(out / "manifest.json", manifest)
    from reporting import generate

    generate(out)
    print(
        f"Regraded {len(rows)} retained responses without model calls: {out}",
        flush=True,
    )


def freeze_challenge(baseline, out, seed, wrong_count=14, correct_count=6):
    if wrong_count < 0 or correct_count < 0 or wrong_count + correct_count == 0:
        raise ValueError("Challenge counts must be nonnegative with a positive total")
    out = Path(out)
    if out.exists() or out.with_suffix(".cases.json").exists():
        raise ValueError("Frozen suite already exists; use a new path")
    manifest, bundle, cases, rows = load_baseline(baseline)
    cohorts = {False: [], True: []}
    for identifier, (row, _) in rows.items():
        # Service failures and invalid answer formats are reported, but are not
        # reasoning mistakes. Never fill wrong-answer slots with those records.
        if row["status"] == "completed" and row["grade"]["parsed"]:
            cohorts[row["grade"]["correct"]].append(identifier)
    selected = []
    for correct, count in ((False, wrong_count), (True, correct_count)):
        available = sorted(
            cohorts[correct], key=lambda identifier: digest([seed, identifier])
        )
        if len(available) < count:
            raise ValueError(
                f"Need {count} {'correct' if correct else 'wrong'} parsed answers; have {len(available)}. Keep the baseline; do not replace questions or retry answers."
            )
        selected += available[:count]
    selected.sort(key=lambda identifier: digest([seed, "order", identifier]))
    planned = {row["id"]: row for row in manifest["suite"]["cases"]}
    suite = {
        "schema": 1,
        "purpose": "baseline-outcome-selected development challenge; not representative capability evaluation",
        "seed": seed,
        "sources_lock_sha256": manifest["suite"]["sources_lock_sha256"],
        "selection": {
            "kind": "baseline_outcomes",
            "wrong": wrong_count,
            "correct": correct_count,
            "baseline_manifest_sha256": digest(manifest),
            "baseline_suite_sha256": digest(manifest["suite"]),
            "baseline_result_sha256": {
                identifier: digest(rows[identifier][1].read_bytes())
                for identifier in selected
            },
            "eligibility": "completed and parsed only; SHA256(seed, id) within each outcome; no Fusion outputs used",
        },
        "cases": [planned[identifier] for identifier in selected],
    }
    write_json(out, suite)
    write_json(
        out.with_suffix(".cases.json"),
        {
            "schema": 1,
            "suite_sha256": digest(suite),
            "sources_lock_sha256": suite["sources_lock_sha256"],
            "upstream_files": bundle["upstream_files"],
            "cases": [cases[identifier] for identifier in selected],
        },
    )
    load_frozen_questions(out, suite)
    print(f"Frozen {wrong_count} wrong + {correct_count} correct: {out}", flush=True)


def reference_for(baseline, suite, cases):
    manifest, _, original_cases, rows = load_baseline(baseline)
    selection = suite.get("selection", {})
    if selection.get("kind") != "baseline_outcomes" or selection[
        "baseline_manifest_sha256"
    ] != digest(manifest):
        raise ValueError("Challenge was not frozen from this baseline")
    wrong_count, correct_count = selection["wrong"], selection["correct"]
    if (
        not isinstance(wrong_count, int)
        or not isinstance(correct_count, int)
        or wrong_count < 0
        or correct_count < 0
        or wrong_count + correct_count == 0
    ):
        raise ValueError("Invalid challenge counts")
    total = wrong_count + correct_count
    if (
        len(suite["cases"]) != total
        or len({row["id"] for row in suite["cases"]}) != total
    ):
        raise ValueError(f"Challenge must contain {total} distinct questions")
    if (
        sum(rows[item["id"]][0]["grade"]["correct"] for item in suite["cases"])
        != correct_count
    ):
        raise ValueError(
            f"Challenge must contain exactly {wrong_count} wrong and {correct_count} correct"
        )
    records = {}
    for planned in suite["cases"]:
        identifier = planned["id"]
        row, path = rows[identifier]
        if (
            row["status"] != "completed"
            or not row["grade"].get("parsed")
            or original_cases[identifier] != cases[identifier]
            or digest(path.read_bytes())
            != selection["baseline_result_sha256"][identifier]
        ):
            raise ValueError("Frozen baseline result/content changed: " + identifier)
        records[identifier] = {"result_sha256": digest(path.read_bytes())}
    return {
        "manifest_sha256": digest(manifest),
        "source_started_at_utc": manifest["started_at_utc"],
        "records": records,
    }


def validate_resume(directory, suite, configuration, args, arms, reference):
    manifest = read_json(Path(directory) / "manifest.json")
    if (Path(directory) / "INVALID.md").exists():
        raise ValueError("Cannot resume an invalid exercise")
    for key, value in {
        "suite": suite,
        "arms": arms,
        "repeats": args.repeats,
        "timeout_seconds": args.timeout,
        "baseline_reference": reference,
        "binary_sha256": digest(args.binary.read_bytes()),
        "http_502_retries": args.http_502_retries,
        "http_502_retry_delay": args.http_502_retry_delay,
    }.items():
        if manifest.get(key) != value:
            raise ValueError("Resume manifest mismatch: " + key)
    if (
        manifest["configuration"]["baseline"] != configuration["baseline"]
        or manifest["configuration"]["catalogue"] != configuration["catalogue"]
    ):
        raise ValueError("Resume model catalogue/configuration mismatch")
    if (
        "fusion" in arms
        and manifest["configuration"]["fusion"] != configuration["fusion"]
    ):
        raise ValueError("Resume Fusion configuration mismatch")
    for name, expected in manifest["harness_sha256"].items():
        if digest((ROOT / name).read_bytes()) != expected:
            raise ValueError("Resume harness changed: " + name)


def execute(client, args, suite, cases, configuration, arms):
    from reporting import generate

    if args.baseline:
        _, _, _, rows = load_baseline(args.baseline)
        for planned in suite["cases"]:
            row, path = rows[planned["id"]]
            destination = args.out / f"r0-{row['case_id'].replace(':', '-')}-single"
            destination.mkdir(exist_ok=True)
            if not (destination / "result.json").exists():
                shutil.copyfile(path, destination / "baseline-result-original.json")
                write_json(
                    destination / "result.json",
                    {
                        **row,
                        "reused_baseline": True,
                        "baseline_result_sha256": digest(path.read_bytes()),
                    },
                )

    def question(repeat, planned):
        if "fusion" in arms and digest(
            client.call(["fusion", "settings", "--json"])["configuration"]
        ) != digest(configuration["fusion"]["configuration"]):
            raise ValueError("Fusion configuration changed during the run")
        for arm in arms:
            directory = args.out / f"r{repeat}-{planned['id'].replace(':', '-')}-{arm}"
            print(f"Running {planned['id']} repeat {repeat + 1}: {arm}", flush=True)
            already_recorded = (directory / "result.json").exists()
            if arm == "single":
                from retry import run_single

                result = run_single(
                    client,
                    cases[planned["id"]],
                    directory,
                    args.timeout,
                    args.http_502_retries,
                    args.http_502_retry_delay,
                    args.resume,
                )
            else:
                result = client.run_case(
                    cases[planned["id"]],
                    arm,
                    directory,
                    args.timeout,
                    resume=args.resume,
                )
            print(
                f"  {planned['id']} {arm}: {result['status']}; correct={result['grade']['correct']}; seconds={result['elapsed_seconds']:.1f}"
                + (f"; {result['error']}" if result.get("error") else ""),
                flush=True,
            )

            if (
                result["status"] != "completed"
                and not already_recorded
                and result.get("http_status") != 502
            ):
                raise ValueError(
                    "Baseline/model execution failed; accepted questions retained. Inspect the recorded error before --resume: "
                    + planned["id"]
                )

    # Keep a bounded submission window. A protocol failure stops new submissions;
    # accepted work is allowed to finish and all results remain on disk.
    work = iter(
        (repeat, planned)
        for repeat in range(args.repeats)
        for planned in suite["cases"]
    )
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        pending = {}
        for _ in range(args.jobs):
            item = next(work, None)
            if item is not None:
                pending[pool.submit(question, *item)] = item
        while pending:
            done = next(as_completed(pending))
            pending.pop(done)
            try:
                done.result()
            finally:
                generate(args.out)
            item = next(work, None)
            if item is not None:
                pending[pool.submit(question, *item)] = item
    generate(args.out)
