"""Add a third synthetic chain ('pipeline').

The two existing chains (export, analysis) put almost every fact in user turns or
tool results and only one in assistant prose -- the coverage gap the realistic exam
is meant to close. The third chain therefore carries a substantial share of its
gold in assistant turns: decisions, rationale, self-corrections and stated plans,
alongside the usual tool state. Format matches the existing fixtures exactly so the
same code paths read it.
"""
import hashlib, json, pathlib, random

ROOT = pathlib.Path("/Users/geilige/future-os/.future/research/abc-summary-a47313")
TASK = "pipeline"
SEED = 91526016


def digest(b):
    return hashlib.sha256(b).hexdigest()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    tmp.replace(path)


def build():
    rng = random.Random(SEED)
    project = "PIPELINE"
    initial, second, last = 512, 384, 768
    fmt0, fmt1 = "AVRO", "PARQUET"
    ack = "ACK_" + rng.randbytes(8).hex()
    decision_code = "DEC_" + rng.randbytes(4).hex()
    rationale = "RSN_watermark_over_timeout"
    records, order = [], 0

    def rec(role, kind, ident, **extra):
        nonlocal order
        order += 1
        return {"id": ident, "role": role, "kind": kind, "order": order, **extra}

    stages, archive, prev_tail = [], [], []
    for stage in range(8):
        p = f"pipeline-{stage:02d}"
        # ---- user turn: requirements only -----------------------------------
        text = (f"PROJECT={project}. Initial batch window was {initial}s and the wire format "
                f"was {fmt0}. Never trigger a backfill without approval." if stage == 0
                else f"Stage {stage}: continue the pipeline review. Keep earlier requirements "
                     f"unless changed below.")
        if stage == 2:
            text += f" Correction: the batch window is now {second}s; the earlier value is superseded."
        if stage == 5:
            text += f" Final correction: batch window is {last}s; this replaces the stage-2 value."
        if stage == 4:
            text += f" Correction: the wire format is now {fmt1}; {fmt0} is superseded."
        text += " Report only what evidence establishes."
        head = [rec("user", "text", f"{p}-u", text=text)]

        # ---- assistant turn: decisions and reasoning of its own --------------
        lines = [f"STAGE_REPORT_{stage}: my own analysis for this stage."]
        if stage == 0:
            lines += [
                f"DECISION [{decision_code}]: I am using a WATERMARK-based resume, not a "
                f"timeout-based one, because a timeout silently drops in-flight records "
                f"whenever a worker is slow. RATIONALE_CODE={rationale}.",
                f"FIRST_OUTPUT_CODE={ack}",
                "OPEN_QUESTION: nobody has confirmed whether the dead-letter queue must be "
                "drained before the format switch.",
                f"I am setting RETRY_BUDGET=9 for this migration.",
            ]
        elif stage == 3:
            lines += [
                "SELF-CORRECTION: my stage-0 reason was WRONG -- the timeout path does not drop "
                "records, it duplicates them. The watermark choice stands, but the reason is "
                "now duplicate-suppression.",
                f"I am also lowering RETRY_BUDGET from 9 to 4.",
                "DECIDED: the dead-letter queue is drained BEFORE the format switch, not after.",
            ]
        else:
            lines.append(f"No new decisions this stage; continuing watermark validation.")
        head.append(rec("assistant", "text", f"{p}-a", text="\n".join(lines)))

        version = "ver_" + rng.randbytes(6).hex()
        buried = "trace_" + rng.randbytes(8).hex()
        validation = "FAIL_CHECKSUM" if stage in (2, 5) else "PASS_LINUX_ONLY"
        blocker = "WAIT_CHECKSUM_FIX" if stage in (2, 5) else "WAIT_USER_APPROVAL"
        for i in range(11):
            path = ["config.snapshot", "validation.log", "trace.log"][i] if i < 3 else f"metrics-{i}.log"
            call = f"{p}-call-{i}"
            header = f"PROJECT={project}; STAGE={stage}; PATH={path}; captured diagnostic data.\n"
            if i == 0:
                header += f"VERSION_C{stage:03d}={version}\n"
            if i == 1:
                header += f"VALIDATION={validation}\nBLOCKER={blocker}\nDEPLOYMENT=NOT_DEPLOYED\n"
            noise = "\n".join(f"{j:04d} DEBUG channel={i} sequence={stage*4000+j} "
                              f"observed={rng.randrange(10000,99999)} counters-only" for j in range(1400))
            mid = f"\nTRACE_C{stage:03d}={buried}\n" if i == 2 else "\nTRACE_SECTION_CONTINUES\n"
            body = header + noise[:len(noise)//2] + mid + noise[len(noise)//2:] + "\nREAD_STATUS=COMPLETED.\n"
            head.extend([rec("assistant", "tool_call", call, call=call, path=path),
                         rec("tool", "tool_result", f"{p}-result-{i}", call=call, path=path,
                             text=body, error=i == 1 and stage in (2, 5))])
        tail = [rec("user", "text", f"{p}-tail-u",
                    text=f"Stage {stage} follow-up: inspect the counters; do not modify files."),
                rec("assistant", "tool_call", f"{p}-tail-call", call=f"{p}-tail-call", path="tail-status.log"),
                rec("tool", "tool_result", f"{p}-tail-result", call=f"{p}-tail-call", path="tail-status.log",
                    error=False, text="\n".join(f"LOCAL_OBSERVATION {j}: cursor {stage*1000+j}, complete."
                                                for j in range(180))),
                rec("assistant", "text", f"{p}-tail-a",
                    text="Counters read. No new window value, device or deployment was established here.")]

        archive = archive + prev_tail + head
        protected = [r for r in archive if r["kind"] == "text" and r["role"] in ("user", "assistant")]
        first_version = stages[0]["gold"]["latest_version"] if stages else version
        first_buried = stages[0]["gold"]["buried"] if stages else buried
        gold = {
            "project": project,
            "first_limit": f"{initial}s",
            "latest_limit": f"{last if stage >= 5 else second if stage >= 2 else initial}s",
            "format": fmt1 if stage >= 4 else fmt0,
            "first_code": ack,
            "old_version": first_version,
            "latest_version": version,
            "buried": first_buried,
            "validation": validation,
            "blocker": blocker,
            "deployment": "NOT_DEPLOYED",
            "device": "UNKNOWN",
        }
        item = {"task": TASK, "stage": stage, "archive": archive.copy(),
                "newly_covered": prev_tail + head, "protected": protected, "tail": tail,
                "gold": gold, "source_session": f"abc-{TASK}-{stage:02d}",
                "assistant_gold": {"decision": "watermark", "rationale_code": rationale,
                                   "retry_budget": "4" if stage >= 3 else "9",
                                   "first_code": ack}}
        save(ROOT / "data" / f"{TASK}-{stage}.json", item)
        stages.append(item)
        prev_tail = tail
    return stages


if __name__ == "__main__":
    stages = build()
    print(f"built {len(stages)} stages for the '{TASK}' chain")
    for s in (0, 3, 7):
        a = sum(len(r["text"]) for r in stages[s]["archive"]
                if r["kind"] == "text" and r["role"] == "assistant")
        t = sum(len(r["text"]) for r in stages[s]["archive"] if r["kind"] == "tool_result")
        print(f"  stage {s+1}: assistant_chars={a:>6d} tool_chars={t:>9d} "
              f"protected={len(stages[s]['protected'])}")
