"""Retry only confirmed HTTP 502 failures in single-model baseline generation."""

import shutil
import time
from pathlib import Path

from datasets import digest, read_json, write_json
from scoring import sum_usage


def run_single(
    client, case, directory, timeout, retries=3, retry_delay=2, resume=False
):
    directory = Path(directory)
    if (directory / "result.json").exists():
        if not resume:
            raise ValueError("Case result already exists")
        return read_json(directory / "result.json")
    if directory.exists() and not resume:
        raise ValueError("Case directory already exists")
    directory.mkdir(parents=True, exist_ok=True)
    (directory / ".gitignore").write_text("*\n", encoding="utf-8")
    state_path = directory / "retry-state.json"
    state = (
        read_json(state_path)
        if state_path.exists()
        else {"started_at_epoch": time.time()}
    )
    write_json(state_path, state)
    attempts = []
    for attempt in range(1, retries + 2):
        path = directory / "attempts" / f"attempt-{attempt}"
        if (
            path.exists()
            and not (path / "result.json").exists()
            and not (path / "ack.json").exists()
        ):
            raise ValueError(
                "Unacknowledged submission; inspect before resuming, never resubmit"
            )
        result = client.run_case(
            case, "single", path, timeout, resume=resume and path.exists()
        )
        attempts.append(
            {
                "attempt": attempt,
                "status": result["status"],
                "http_status": result.get("http_status"),
                "session_id": result.get("session_id"),
                "run_id": result.get("run_id"),
                "parent_usage": result.get("parent_usage", {}),
                "elapsed_seconds": result["elapsed_seconds"],
                "result_sha256": digest((path / "result.json").read_bytes()),
            }
        )
        retryable = result["status"] != "completed" and result.get("http_status") == 502
        if retryable and attempt <= retries:
            delay = min(30, retry_delay * 2 ** min(attempt - 1, 20))
            print(
                f"  {case['id']}: HTTP 502; retry {attempt}/{retries} after {delay:g}s; failed attempt retained",
                flush=True,
            )
            time.sleep(delay)
            continue
        # Keep the final accepted attempt in the traditional top-level layout
        # for reporting/audit; all original attempts remain immutable underneath.
        for source in path.iterdir():
            if source.is_file() and source.name not in ("result.json", ".gitignore"):
                shutil.copyfile(source, directory / source.name)
        result = {
            **result,
            "attempt_count": len(attempts),
            "http_502_retries": len(attempts) - 1,
            "attempts": attempts,
            "final_attempt_answer_seconds": result.get("answer_wall_seconds"),
            "elapsed_seconds": time.time() - state["started_at_epoch"],
            "parent_usage": sum_usage([item["parent_usage"] for item in attempts]),
            "usage_incomplete_possible": any(
                item["status"] != "completed" for item in attempts
            ),
        }
        if "answer_wall_seconds" in result:
            result["answer_wall_seconds"] = result["elapsed_seconds"]
        write_json(directory / "result.json", result)
        return result
    raise AssertionError("Unreachable retry state")
