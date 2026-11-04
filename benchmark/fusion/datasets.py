"""Pinned upstream downloads and deterministic, answer-independent sampling."""

import hashlib
import json
import re
import urllib.request
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


def digest(value):
    if not isinstance(value, bytes):
        value = json.dumps(value, sort_keys=True, ensure_ascii=False).encode()
    return hashlib.sha256(value).hexdigest()


def prepare(names):
    lock = read_json(ROOT / "sources.lock.json")
    previous = ROOT / "data" / "manifest.json"
    existing = read_json(previous) if previous.exists() else []
    cached = {(row["dataset"], row["file"]): row for row in existing}
    records = []
    for name in names:
        source = lock["sources"][name]
        for filename in source["files"]:
            if source["kind"] == "github":
                url = f"https://raw.githubusercontent.com/{source['repo']}/{source['revision']}/{filename}"
            else:
                url = f"https://huggingface.co/datasets/{source['repo']}/resolve/{source['revision']}/{filename}"
            path = ROOT / "data" / name / filename
            known = cached.get((name, filename), {})
            if not (
                path.exists()
                and known.get("url") == url
                and known.get("sha256") == digest(path.read_bytes())
            ):
                request = urllib.request.Request(
                    url, headers={"User-Agent": "FutureOS-Fusion-Benchmark/1"}
                )
                with urllib.request.urlopen(request, timeout=120) as response:
                    data = response.read()
                path.parent.mkdir(parents=True, exist_ok=True)
                temporary = path.with_suffix(path.suffix + ".part")
                temporary.write_bytes(data)
                temporary.replace(path)
            data = path.read_bytes()
            records.append(
                {
                    "dataset": name,
                    "file": filename,
                    "url": url,
                    "sha256": digest(data),
                    "bytes": len(data),
                }
            )
            print(f"prepared {name}/{filename}: {len(data)} bytes", flush=True)
    keys = {(r["dataset"], r["file"]) for r in records}
    write_json(
        previous,
        [r for r in existing if (r["dataset"], r["file"]) not in keys] + records,
    )


def load_cases(name):
    base = ROOT / "data" / name
    if name == "zebra":
        import pyarrow.parquet as pq

        rows = pq.read_table(base / "grid_mode/test-00000-of-00001.parquet").to_pylist()
        return [
            case(name, row["id"], row["puzzle"], row["solution"], "grid", row["size"])
            for row in rows
        ]
    if name == "bbeh":
        rows = read_json(base / "bbeh/mini/data.json")["examples"]
        return [
            case(
                name,
                str(i),
                row["input"],
                row["target"],
                "bbeh",
                row.get("task", "mini"),
            )
            for i, row in enumerate(rows)
        ]
    if name == "omni":
        rows = [
            json.loads(line)
            for line in (base / "test.jsonl").read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        # Deliberately narrow pilot: unambiguous integer answer, not a judge's opinion.
        return [
            case(
                name,
                str(i),
                row["problem"],
                row["answer"].strip(),
                "integer",
                str(row["difficulty"]),
            )
            for i, row in enumerate(rows)
            if re.fullmatch(r"-?\d+", row["answer"].strip())
        ]
    if name == "musr":
        cases = []
        for domain in ("murder_mystery", "object_placements", "team_allocation"):
            for i, row in enumerate(read_json(base / f"datasets/{domain}.json")):
                for j, question in enumerate(row["questions"]):
                    options = "\n".join(
                        f"{chr(65 + k)}. {value}"
                        for k, value in enumerate(question["choices"])
                    )
                    prompt = (
                        row["context"] + "\n\n" + question["question"] + "\n" + options
                    )
                    answer = question["answer"]
                    if not isinstance(answer, int) or not 0 <= answer < len(
                        question["choices"]
                    ):
                        raise ValueError(f"Invalid MuSR answer index: {domain}/{i}/{j}")
                    cases.append(
                        case(
                            name,
                            f"{domain}:{i}:{j}",
                            prompt,
                            chr(65 + answer),
                            "choice",
                            domain,
                        )
                    )
        return cases
    raise ValueError(
        f"{name}: raw data is installed; scoring adapter is not enabled in the pilot"
    )


def case(dataset, identifier, question, answer, scorer, stratum):
    return {
        "id": f"{dataset}:{identifier}",
        "dataset": dataset,
        "question": question,
        "answer": answer,
        "scorer": scorer,
        "stratum": str(stratum),
    }


def freeze_questions(suite_path, suite, rows):
    """Store inputs/references separately from the immutable selection manifest."""
    wanted = {row["id"] for row in suite["cases"]}
    bundle = {
        "schema": 1,
        "suite_sha256": digest(suite),
        "sources_lock_sha256": suite["sources_lock_sha256"],
        "upstream_files": [
            item
            for item in read_json(ROOT / "data/manifest.json")
            if item["dataset"] in {row["dataset"] for row in suite["cases"]}
        ],
        "cases": [row for row in rows if row["id"] in wanted],
    }
    path = Path(suite_path).with_suffix(".cases.json")
    write_json(path, bundle)
    load_frozen_questions(suite_path, suite)
    return path


def load_frozen_questions(suite_path, suite):
    path = Path(suite_path).with_suffix(".cases.json")
    bundle = read_json(path)
    if (
        bundle.get("schema") != 1
        or bundle.get("suite_sha256") != digest(suite)
        or bundle.get("sources_lock_sha256") != suite["sources_lock_sha256"]
        or bundle.get("sources_lock_sha256")
        != digest(read_json(ROOT / "sources.lock.json"))
    ):
        raise ValueError("Frozen question bundle provenance mismatch: " + str(path))
    rows = bundle["cases"]
    cases = {row["id"]: row for row in rows}
    if len(cases) != len(rows) or set(cases) != {row["id"] for row in suite["cases"]}:
        raise ValueError("Frozen question bundle IDs differ from suite")
    scorers = {"zebra": "grid", "bbeh": "bbeh", "omni": "integer", "musr": "choice"}
    for planned in suite["cases"]:
        row = cases[planned["id"]]
        if (
            row["dataset"] != planned["dataset"]
            or row["stratum"] != planned["stratum"]
            or row["scorer"] != scorers[row["dataset"]]
            or digest(row["question"]) != planned["question_sha256"]
            or digest(row["answer"]) != planned["answer_sha256"]
        ):
            raise ValueError("Frozen question content mismatch: " + planned["id"])
    return bundle, cases


def sample_cases(names, per_dataset, seed):
    selected = []
    for name in names:
        groups = defaultdict(list)
        for row in load_cases(name):
            groups[row["stratum"]].append(row)
        for rows in groups.values():
            rows.sort(key=lambda row: digest([seed, row["id"]]))
        # Round robin over strata; neither reference answers nor model outcomes enter selection.
        strata = sorted(groups, key=lambda group: digest([seed, name, group]))
        chosen = []
        while len(chosen) < per_dataset and any(groups.values()):
            for group in strata:
                if groups[group] and len(chosen) < per_dataset:
                    chosen.append(groups[group].pop(0))
        if len(chosen) != per_dataset:
            raise ValueError(f"{name}: only {len(chosen)} eligible cases")
        selected.extend(chosen)
    return selected
