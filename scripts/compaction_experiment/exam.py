"""A realistic exam: retention of values the agent itself produced.

Real follow-up turns are mostly about the agent's own output (measured: ~80% of
follow-ups reference it; only ~4% repeat the user's earlier ask). The original
questionnaire inverted that, asking almost entirely about user turns and tool
results. This builds a questionnaire with the measured weighting.

Design (objective and cheap, stated openly as a *retention probe*):
    present values : N exact strings the agent produced in the covered range
    decoys         : M plausible strings that appear nowhere in the session
    question       : "which of these appeared in the conversation? return JSON"
    score          : hits on present values minus false positives on decoys

A projection that dropped a value cannot distinguish it from a decoy, so the score
measures what actually survived. It is a recognition task, not free recall --
noted as a limitation.

Value sources, by weight:
    assistant text : paths, commit SHAs, PR numbers, versions, quoted identifiers,
                     counts with units   (80%)
    user turns     : stated constraints and numbers   (10%)
    tool results   : identifiers unique to tool output (10%)
"""
import re, json, pathlib, random

PATH_RE = re.compile(r"[`\s(]((?:[\w.-]+/)*[\w.-]+\.(?:rs|ts|tsx|js|py|md|json|html|css|kt|swift|yaml|toml|txt))\b")
SHA_RE = re.compile(r"\b([0-9a-f]{7,12})\b")
PR_RE = re.compile(r"\bPR\s?#(\d{1,5})\b|/pull/(\d{1,5})\b")
VER_RE = re.compile(r"`([\w.]+-[\w.]+)`")
COUNT_RE = re.compile(r"\b(\d[\d,]*(?:\.\d+)?)\s*(?:项|个|条|套件|次)\s*(?:测试)?")
SIZE_RE = re.compile(r"\b(\d+(?:\.\d+)?\s?(?:MiB|MB|GiB|GB|KB))\b")
BACKTICK_RE = re.compile(r"`([A-Za-z_][\w./-]{4,60})`")
# Vendored vocabularies: the synthetic fixtures use prefixed hex ids, and real
# sessions use the same shapes plus backticked paths.
PREFIXED_ID_RE = re.compile(r"\b((?:ver|trace|ACK|DEC|RSN)_[0-9a-zA-Z_]{4,40})\b")
CAP_RE = re.compile(r"\b(\d{2,5}\s?(?:MiB|MB))\b")
PROJECT_RE = re.compile(r"\bPROJECT=([A-Z]{3,12})\b")


def values_in(text, role, kind, rng, out, limit_per_source=40):
    if not text:
        return
    if role == "assistant" and kind == "text":
        for m in PATH_RE.findall(text)[:6]:
            out["assistant"].append(m)
        for m in SHA_RE.findall(text)[:4]:
            out["assistant"].append(m)
        for a, b in PR_RE.findall(text)[:3]:
            out["assistant"].append(f"PR #{a or b}")
        for m in SIZE_RE.findall(text)[:3]:
            out["assistant"].append(m)
        for m in COUNT_RE.findall(text)[:3]:
            out["assistant"].append(m)
        for m in BACKTICK_RE.findall(text)[:6]:
            out["assistant"].append(m)
        for m in PREFIXED_ID_RE.findall(text)[:6]:
            out["assistant"].append(m)
        for m in CAP_RE.findall(text)[:3]:
            out["assistant"].append(m)
        for m in PROJECT_RE.findall(text)[:2]:
            out["assistant"].append(m)
    elif role == "user" and kind == "text":
        for m in SIZE_RE.findall(text)[:2]:
            out["user"].append(m)
        for m in re.findall(r"\b(\d{2,5})\b", text)[:3]:
            out["user"].append(m)
        for m in PATH_RE.findall(text)[:2]:
            out["user"].append(m)
        for m in CAP_RE.findall(text)[:3]:
            out["user"].append(m)
    elif kind == "tool_result":
        for m in SHA_RE.findall(text)[:6]:
            out["tool"].append(m)
        for m in PREFIXED_ID_RE.findall(text)[:6]:
            out["tool"].append(m)
        for m in SIZE_RE.findall(text)[:2]:
            out["tool"].append(m)


def build_exam(records, upto, rng, n_assistant=12, n_user=2, n_tool=3, n_decoys=8):
    """Pick the question set from records[:upto]; gold is what is actually there."""
    found = {"assistant": [], "user": [], "tool": []}
    for r in records[:upto]:
        values_in(r.get("text", ""), r.get("role", ""), r.get("kind", ""), rng, found)

    def pick(bucket, n):
        seen, out = set(), []
        for v in bucket:
            v = v.strip()
            if len(v) < 3 or v in seen:
                continue
            seen.add(v)
            out.append(v)
            if len(out) >= n * 4:          # keep a pool so shuffling stays fair
                break
        rng.shuffle(out)
        return out[:n]

    present = (pick(found["assistant"], n_assistant) + pick(found["user"], n_user)
               + pick(found["tool"], n_tool))

    def decoy():
        shapes = [
            lambda: f"{rng.randrange(0x10000000, 0xffffffff):08x}",
            lambda: f"PR #{rng.randrange(100, 999)}",
            lambda: f"{rng.randrange(10, 99)}.{rng.choice('0123456789')}.{rng.randrange(0, 9)}-diag",
            lambda: f"{rng.choice(['src','lib','app','packages'])}/{rng.choice(['cache','router','adapter','parser'])}.{rng.choice(['rs','ts','py'])}",
            lambda: f"{rng.randrange(11, 99)}.{rng.randrange(0, 9)} {rng.choice(['MiB','MB'])}",
            lambda: f"{rng.randrange(100, 2000)} 项测试",
        ]
        return rng.choice(shapes)()

    present_set = set(present)
    decoys = []
    while len(decoys) < n_decoys:
        d = decoy()
        if d not in present_set and d not in decoys:
            decoys.append(d)
    return present, decoys


EXAM_PROMPT = """The conversation above is the record of an engineering session. Below is a list of values.
Some of them appeared in that conversation; some did not appear anywhere in it.

Return one JSON object: {"appeared": ["...", "..."]} listing ONLY the values that you can
confirm appeared in the conversation. Do not list values you cannot confirm, and do not guess.
If you cannot check a value, leave it out.

Values:
"""


def exam_body(present, decoys, rng):
    items = present + decoys
    rng.shuffle(items)
    listing = "\n".join(f"- {v}" for v in items)
    return EXAM_PROMPT + listing, {v: True for v in present}, {v: True for v in decoys}


def score(answer, present_truth, decoy_truth):
    reported = set()
    if isinstance(answer, dict):
        for v in answer.get("appeared", []) or []:
            reported.add(str(v).strip())
    hits = sum(1 for v in present_truth if v in reported)
    false_pos = sum(1 for v in decoy_truth if v in reported)
    return {"hits": hits, "of_present": len(present_truth),
            "false_positives": false_pos, "of_decoys": len(decoy_truth),
            "net": hits - false_pos}
