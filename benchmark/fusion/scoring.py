"""Offline scorers. No model judges, execution feedback, or answer recovery calls."""

import ast
import re

from datasets import ROOT, digest, read_json


def extract(text):
    matches = list(re.finditer(r"(?im)^\s*(?:\*\*)?FINAL_ANSWER\s*:\s*(?:\*\*)?", text))
    if not matches:
        return None
    answer = text[matches[-1].end() :].strip()
    if answer.startswith("```"):
        answer = re.sub(r"^```(?:json)?\s*", "", answer)
        answer = answer.split("```")[0].strip()
    return answer


def bbeh_module():
    # Load only the definitions from the pinned official scorer, excluding its
    # executable examples. The source and its hash are retained in the manifest.
    path = ROOT / "third_party/bbeh/evaluate.py"
    source = read_json(path.parent / "source.json")
    pinned = read_json(ROOT / "sources.lock.json")["sources"]["bbeh"]
    if (
        source["repo"] != pinned["repo"]
        or source["revision"] != pinned["revision"]
        or digest(path.read_bytes()) != source["files"]["evaluate.py"]
    ):
        raise ValueError("Bundled BBEH scorer differs from the pinned source")
    tree = ast.parse(path.read_text(encoding="utf-8"))
    tree.body = [node for node in tree.body if isinstance(node, ast.FunctionDef)]
    namespace = {}
    exec(compile(tree, str(path), "exec"), namespace)
    return namespace


def score(case, text):
    answer = extract(text)
    if answer is None:
        return {
            "correct": False,
            "parsed": False,
            "prediction": None,
            "error": "missing FINAL_ANSWER marker",
        }
    try:
        if case["scorer"] == "grid":
            import json

            prediction = json.loads(answer)
            reference = case["answer"]
            if not isinstance(prediction, dict) or set(prediction) != {
                "header",
                "rows",
            }:
                raise ValueError("grid answer must contain header and rows")

            def canonical(grid):
                header, rows = grid["header"], grid["rows"]
                if (
                    not isinstance(header, list)
                    or not all(isinstance(column, str) for column in header)
                    or len(header) != len(set(header))
                    or not isinstance(rows, list)
                    or not rows
                ):
                    raise ValueError("invalid grid dimensions")
                values = []
                for row in rows:
                    if isinstance(row, dict) and set(row) == set(header):
                        row = [row[column] for column in header]
                    if not isinstance(row, list) or len(row) != len(header):
                        raise ValueError("grid row must match its header")
                    if any(not isinstance(value, (str, int)) for value in row):
                        raise ValueError("grid cells must be strings or integers")
                    values.append(tuple(sorted(zip(header, map(str, row)))))
                return sorted(values)

            correct = canonical(prediction) == canonical(reference)
        elif case["scorer"] == "integer":
            prediction = answer.splitlines()[0].strip()
            if not re.fullmatch(r"-?\d+", prediction):
                raise ValueError("integer answer required")
            correct = int(prediction) == int(case["answer"])
        elif case["scorer"] == "choice":
            prediction = answer.splitlines()[0].strip().upper().strip("(). ")
            if prediction not in "ABCDEFGHIJKLMNOPQRSTUVWXYZ" or len(prediction) != 1:
                raise ValueError("one option letter required")
            correct = prediction == case["answer"]
        elif case["scorer"] == "bbeh":
            prediction = answer.splitlines()[0].strip()
            correct = bbeh_module()["evaluate_correctness"](
                prediction, str(case["answer"])
            )
        else:
            raise ValueError(f"unsupported scorer {case['scorer']}")
        return {"correct": bool(correct), "parsed": True, "prediction": prediction}
    except (ValueError, KeyError, TypeError, IndexError) as error:
        return {
            "correct": False,
            "parsed": False,
            "prediction": answer,
            "error": str(error),
        }


def numeric_usage(value):
    """Preserve unknown fields; sum numeric leaves without inventing missing cost."""
    result = {}
    if isinstance(value, dict):
        for key, item in value.items():
            if isinstance(item, (int, float)) and not isinstance(item, bool):
                result[key] = item
            elif isinstance(item, dict):
                for subkey, subvalue in numeric_usage(item).items():
                    result[f"{key}.{subkey}"] = subvalue
    return result


def sum_usage(usages):
    totals = {}
    for usage in usages:
        for key, value in numeric_usage(usage).items():
            totals[key] = totals.get(key, 0) + value
    return totals
