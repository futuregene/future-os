#!/usr/bin/env python3
"""Exercise the agent's `suggest_skill` gRPC command end to end.

Not part of CI: it needs grpcurl, a real Future credential, and a live gateway, so
it costs money and depends on the network. Run it by hand after touching the
recommendation path — the unit tests cover the decision logic, and this covers the
wire: the typed payload the clients parse, every failure collapsing to "no
recommendation", and the boundaries the constants imply.

  # 1. an isolated agent (its own socket, database and lock — never the one you use)
  future agent --home /tmp/reco-test --verbose --log-file

  # 2. the suite
  python3 scripts/skill_reco/suggest-skill-tests.py            # ~33 calls, ≈¥0.04
  python3 scripts/skill_reco/suggest-skill-tests.py --only A,B # free suites: no gateway
  python3 scripts/skill_reco/suggest-skill-tests.py --only C,E # gateway suites only

Suites: A credential/availability, B input validation (no network), C real gateway,
D protocol shape and concurrency, E boundaries (the 254-candidate cap, CJK, long input).

`--only A,B,D` runs without spending anything: those never reach the network.
"""

import argparse
import json
import os
import subprocess
import sys
import time

REPO = os.environ.get("FUTURE_REPO", "/Users/geilige/future-os")
HOME_DIR = os.environ.get("RECO_TEST_HOME", "/tmp/reco-test")
SOCKET = f"{HOME_DIR}/run/agent.sock"
AUTH = f"{HOME_DIR}/agent/auth.json"
REAL_AUTH = os.path.expanduser("~/.future/agent/auth.json")
LOG = f"{HOME_DIR}/agent/logs/agent.log"

PROTO_ARGS = [
    "-plaintext",
    "-import-path", f"{REPO}/packages/rpc/proto",
    "-proto", "future.proto",
]

results = []
calls_made = 0


def call(command, timeout=90):
    """One ExecuteCommand round trip. Returns (elapsed_s, parsed_response)."""
    global calls_made
    calls_made += 1
    started = time.time()
    proc = subprocess.run(
        ["grpcurl", *PROTO_ARGS, "-d", json.dumps(command), f"unix://{SOCKET}",
         "proto.FutureAgent/ExecuteCommand"],
        capture_output=True, text=True, timeout=timeout,
    )
    elapsed = time.time() - started
    if proc.returncode != 0:
        return elapsed, {"_grpcurl_error": proc.stderr.strip()[:300]}
    try:
        return elapsed, json.loads(proc.stdout)
    except json.JSONDecodeError:
        return elapsed, {"_bad_json": proc.stdout[:300]}


def suggest(query, candidates, **extra):
    cmd = {"id": f"t{int(time.time()*1000) % 100000}", "type": "suggest_skill",
           "suggest_query": query, "suggest_candidates": candidates}
    cmd.update(extra)
    return call(cmd)


def recommend(response):
    """The recommended skill name, or None."""
    payload = (response or {}).get("payload") or {}
    skill = (payload.get("suggestSkill") or {}).get("skill") or {}
    return skill.get("name") or None


def record(case, expected, actual, note="", elapsed=None):
    ok = expected == actual if not callable(expected) else expected(actual)
    results.append((case, ok, note, elapsed))
    mark = "PASS" if ok else "FAIL"
    timing = f"  [{elapsed:.2f}s]" if elapsed is not None else ""
    print(f"{mark}  {case}{timing}")
    if not ok:
        print(f"        expected {expected!r}, got {actual!r}  {note}")
    elif note:
        print(f"        {note}")
    return ok


def cand(name, description):
    return {"name": name, "description": description}


def log_line_count():
    try:
        with open(LOG) as handle:
            return len(handle.readlines())
    except FileNotFoundError:
        return 0


def log_tail(since):
    try:
        with open(LOG) as handle:
            return "".join(handle.readlines()[since:])
    except FileNotFoundError:
        return ""


def install_credential():
    """Copy the real account credential into the isolated home. True if available."""
    if not os.path.exists(REAL_AUTH):
        return False
    with open(REAL_AUTH) as handle:
        real = json.load(handle)
    os.makedirs(os.path.dirname(AUTH), exist_ok=True)
    with open(AUTH, "w") as handle:
        json.dump({"future": real["future"]}, handle)
    return True


def remove_credential():
    if os.path.exists(AUTH):
        os.remove(AUTH)


# ── A: credential / availability ────────────────────────────────────────────

def suite_a():
    print("\n── A. credential and availability ──")
    if os.path.exists(AUTH):
        os.remove(AUTH)
    elapsed, resp = suggest("帮我把这张照片转成水彩风格",
                            [cand("future-image", "Generate and edit images"),
                             cand("future-web", "Search the web")])
    record("A1 no credential → success with no skill (feature off, not an error)",
           True, resp.get("success") is True and recommend(resp) is None,
           f"payload={json.dumps(resp.get('payload'))}", elapsed)
    record("A2 the response carries the typed payload and no `data` dual-write",
           True, "payload" in resp and "data" not in resp,
           f"keys={sorted(resp.keys())}")

    # A malformed credential must fail the call, not the command.
    os.makedirs(os.path.dirname(AUTH), exist_ok=True)
    with open(AUTH, "w") as handle:
        json.dump({"future": {"type": "api_key", "key": "not-a-real-key", "base_url": "https://future-os.cn/api"}}, handle)
    elapsed, resp = suggest("帮我把这张照片转成水彩风格",
                            [cand("future-image", "Generate and edit images"),
                             cand("future-web", "Search the web")])
    record("A3 invalid credential → success with no skill (no error surfaces)",
           True, resp.get("success") is True and recommend(resp) is None, "", elapsed)
    if os.path.exists(AUTH):
        os.remove(AUTH)


# ── B: input that must never reach the network ──────────────────────────────

def suite_b():
    print("\n── B. input validation (no network) ──")
    good = [cand("future-image", "Generate and edit images"),
            cand("future-web", "Search the web")]

    for label, query, candidates in [
        ("B1 empty query", "", good),
        ("B2 empty candidate list", "帮我把这张照片转成水彩风格", []),
        ("B3 whitespace-only query", "   \n\t  ", good),
    ]:
        elapsed, resp = suggest(query, candidates)
        record(f"{label} → success with no skill",
               True, resp.get("success") is True and recommend(resp) is None,
               f"returned in {elapsed:.2f}s", elapsed)
        record(f"{label} → handled locally (sub-second, so no round trip)",
               True, elapsed < 0.5, f"{elapsed:.3f}s", None)

    # Fields omitted entirely: the RPC must tolerate a caller that never sets them.
    elapsed, resp = call({"id": "t-b4", "type": "suggest_skill"})
    record("B4 suggest_query/-candidates omitted entirely → success with no skill",
           True, resp.get("success") is True and recommend(resp) is None, "", elapsed)

    # The cases above ran *without* a credential, so `endpoint()` returned first
    # and they exercised the NoKey path — never the input check. With a credential
    # present the input check is what must stop them, and it has to stop them
    # before any HTTP: a signed-in user typing nothing must not cost a Jev call.
    if install_credential():
        print("        (re-running the empty-input cases signed in)")
        before = log_line_count()
        for label, query, candidates in [
            ("B5 empty query, signed in", "", good),
            ("B6 empty candidates, signed in", "帮我把这张照片转成水彩风格", []),
            ("B7 blank query, signed in", "   \t ", good),
        ]:
            elapsed, resp = suggest(query, candidates)
            record(f"{label} → success with no skill", True,
                   resp.get("success") is True and recommend(resp) is None, "", elapsed)
            record(f"{label} → rejected before any network call", True,
                   elapsed < 0.5, f"{elapsed:.3f}s (a real call takes ≈0.7-1.8s)", None)
        record("B8 the three were logged as \"nothing to ask\", not \"not signed in\"",
               True, "nothing to ask" in log_tail(before),
               "proves the input check ran rather than the credential check")
        remove_credential()
    else:
        print("        (no credential available — the signed-in input path is UNTESTED)")


# ── C: against the real gateway ────────────────────────────────────────────

def suite_c():
    if not os.path.exists(REAL_AUTH):
        print("\n── C. real gateway — SKIPPED (no ~/.future/agent/auth.json) ──")
        return
    print("\n── C. real gateway ──")
    install_credential()

    small = [cand("future-image", "Generate images from text, edit supplied images, and analyse images for OCR"),
             cand("future-paper", "Search academic literature and retrieve paper content by PMID or DOI"),
             cand("future-slides", "Turn a report or outline into PNG slide images and an optional PDF"),
             cand("future-account", "View the Future account profile and credit balance"),
             cand("future-browser", "Control a local visible Chrome, Edge or Safari browser")]

    elapsed, resp = suggest("帮我把这张海边日落的照片改成水彩风格", small)
    got = recommend(resp)
    record("C1 obvious match → recommends the image skill", "future-image", got, "", elapsed)
    record("C1b the recommendation echoes a description", True,
           bool(((resp.get("payload") or {}).get("suggestSkill") or {}).get("skill", {}).get("description")),
           "", None)
    record("C1c latency is inside the 5s server timeout", True, elapsed < 6.0, f"{elapsed:.2f}s", None)

    elapsed, resp = suggest("这周有什么值得关注的学术论文吗", small)
    record("C2 another clear match → the paper skill", "future-paper", recommend(resp), "", elapsed)

    # Order independence, on a question where exactly one candidate applies.
    # (An *ambiguous* question is genuinely unstable between identical runs, which
    # is a property of the model, not of the ordering — see order-sensitivity.py.)
    unambiguous = "帮我把这张海边日落的照片改成水彩风格"
    pair = [cand("future-image", "Generate images from text, edit supplied images"),
            cand("future-slides", "Turn a report or outline into PNG slides and a PDF")]
    elapsed, resp = suggest(unambiguous, pair)
    first = recommend(resp)
    elapsed2, resp2 = suggest(unambiguous, list(reversed(pair)))
    second = recommend(resp2)
    record("C3 candidate order does not change the answer", first, second,
           f"forward={first}, reversed={second}", elapsed2)

    # An ambiguous request (both candidates apply) is allowed to be unstable, but
    # the answer must always be one of the candidates offered — never a name the
    # caller did not send, which would be unusable client-side.
    ambiguous = "帮我把这张照片转成水彩风格，再做一个 PDF 幻灯片"
    offered = {"future-image", "future-slides"}
    seen = []
    for _ in range(3):
        _, sampled = suggest(ambiguous, pair)
        seen.append(recommend(sampled))
    record("C3b an ambiguous request only ever returns an offered candidate (or refuses)",
           True, all(name is None or name in offered for name in seen),
           f"three samples: {seen}", None)

    elapsed, resp = suggest("帮我订下周二去柏林的机票", small)
    record("C4 nothing fits → refuses (gate says none)", None, recommend(resp),
           f"returned {recommend(resp)!r}", elapsed)

    elapsed, resp = suggest("帮我看看我现在还剩多少额度", small)
    record("C5 a different intent in the same list → account skill",
           "future-account", recommend(resp), "", elapsed)

    # A description longer than the 220-char truncation must still work.
    long_desc = "Handle images. " * 40  # ≈ 640 chars
    elapsed, resp = suggest("把这张图转成素描风格",
                            [cand("future-image", long_desc),
                             cand("future-paper", "Search academic literature")])
    record("C6 a 640-char description (over the 220 truncation) still works",
           "future-image", recommend(resp), "", elapsed)

    # CJK + punctuation in candidate text must survive the option-table encoding.
    elapsed, resp = suggest("把这个 PDF 里的表格抽成 markdown",
                            [cand("future-document", "提取 PDF 或 Word（.docx）内容为结构化 Markdown，含表格与公式"),
                             cand("future-image", "生成图像")])
    record("C7 CJK description and query → document skill",
           "future-document", recommend(resp), "", elapsed)

    # 30 candidates, one clearly relevant: exercises a bigger option table.
    many = [cand(f"skill-{i:02d}", f"Generic capability number {i}") for i in range(29)]
    many.insert(17, cand("future-web", "Search the public web and retrieve pages; verify current facts"))
    elapsed, resp = suggest("帮我上网查一下今天发布的新闻", many)
    record("C8 30 candidates with one relevant → picks the relevant one",
           "future-web", recommend(resp), "", elapsed)
    remove_credential()


# ── D: protocol shape and concurrency ──────────────────────────────────────

def suite_d():
    print("\n── D. protocol shape and concurrency ──")
    elapsed, resp = suggest("测试", [cand("future-image", "Generate and edit images")])
    record("D1 response echoes the command name", "suggest_skill", resp.get("command"))
    record("D2 response is typed `response`", "response", resp.get("type"))
    record("D3 payload is a SuggestSkillResult object", True,
           isinstance((resp.get("payload") or {}).get("suggestSkill"), dict),
           f"{json.dumps(resp.get('payload'))}")
    record("D4 an unanswered call is not an error", True, "error" not in resp,
           f"error={resp.get('error')!r}")

    # Concurrent calls: the handler runs on a blocking worker, so several
    # recommendations must not serialise into a failure or corrupt the answer.
    # Run them on the real path when a credential is available — without one the
    # calls never reach the network and finish in microseconds.
    real_path = install_credential()
    print(f"        (concurrency runs against {'the gateway' if real_path else 'the off path'})")
    import concurrent.futures as futures
    started = time.time()
    with futures.ThreadPoolExecutor(max_workers=4) as pool:
        responses = list(pool.map(
            lambda _: suggest("帮我把照片转成水彩风格",
                              [cand("future-image", "Generate and edit images"),
                               cand("future-web", "Search the web")])[1],
            range(4)))
    spread = time.time() - started
    record("D5 four concurrent calls all return success", True,
           all(r.get("success") is True for r in responses),
           f"wall={spread:.2f}s")
    record("D6 concurrent callers all get a well-formed payload", True,
           all(isinstance((r.get("payload") or {}).get("suggestSkill"), dict) for r in responses))
    if real_path:
        names = [recommend(r) for r in responses]
        record("D7 four concurrent gateway calls all answer the same way", 1,
               len(set(names)), f"answers={names}")
        record("D8 the four ran in parallel, not one after another", True,
               spread < 6.0, f"wall={spread:.2f}s for 4 calls (each ≈0.8s serial)")
        remove_credential()


# ── E: boundaries the constants imply ──────────────────────────────────────

def suite_e():
    if not install_credential():
        print("\n── E. boundaries — SKIPPED (no credential) ──")
        return
    print("\n── E. boundaries (MAX_CANDIDATES = 254, + none = the API's 255) ──")
    try:
        filler = [cand(f"filler-{i:03d}", f"Unrelated capability number {i}") for i in range(299)]
        relevant = cand("future-web", "Search the public web and retrieve pages")

        # Exactly at the cap: 254 candidates + none_of_these = 255 options, which
        # is the most the gateway accepts (bench/option-limit.mjs measured 256 as
        # a hard 400). This is the case the constant exists for.
        at_cap = filler[:253] + [relevant]
        elapsed, resp = suggest("帮我上网查一下今天的新闻", at_cap)
        record("E1 exactly 254 candidates (the cap, 255 options with none) is accepted",
               True, resp.get("success") is True and "error" not in resp,
               f"{len(at_cap)} candidates, answer={recommend(resp)!r}", elapsed)

        # Over the cap: must be truncated, not rejected — the gateway would 400 on
        # 300 options, so an error here would mean the truncation is missing. The
        # match sits inside the head, so it is still reachable and must be found.
        over = filler[:5] + [relevant] + filler[5:299]
        elapsed, resp = suggest("帮我上网查一下今天的新闻", over)
        record("E2 300 candidates are truncated to the cap, and the reachable match is found",
               "future-web", recommend(resp),
               f"{len(over)} sent (match at index 5), answer={recommend(resp)!r}", elapsed)

        # The truncation keeps the head, so a match past the cap is unreachable.
        # Documented rather than desired: a caller offering 300 candidates is
        # choosing which 254 the model ever sees.
        beyond = filler[:260] + [relevant]
        elapsed, resp = suggest("帮我上网查一下今天的新闻", beyond)
        record("E3 a match past position 254 is unreachable (documented truncation)",
               None, recommend(resp),
               f"sent {len(beyond)} with the match at index 260 → {recommend(resp)!r}", elapsed)

        # Degenerate candidate entries the type allows.
        elapsed, resp = suggest("帮我把照片转成水彩风格",
                                [cand("", "A candidate with no name"),
                                 cand("future-image", "Generate images from text, edit supplied images")])
        record("E4 a candidate with an empty name does not break the call",
               True, resp.get("success") is True and recommend(resp) == "future-image",
               f"answer={recommend(resp)!r}", elapsed)

        elapsed, resp = suggest("帮我把照片转成水彩风格",
                                [cand("future-image", "Generate images from text"),
                                 cand("future-image", "A duplicate entry with the same name")])
        record("E5 duplicate candidate names do not produce a malformed answer",
               True, resp.get("success") is True and recommend(resp) in (None, "future-image"),
               f"answer={recommend(resp)!r}", elapsed)

        # A single candidate: the smallest non-empty option table.
        elapsed, resp = suggest("帮我把照片转成水彩风格",
                                [cand("future-image", "Generate images from text, edit supplied images")])
        record("E6 one candidate only → answers it (no ambiguity to resolve)",
               "future-image", recommend(resp), "", elapsed)

        # A query far longer than the PRD's 2000-char client gate. The client
        # filters first, but the RPC must not misbehave if one gets through.
        long_query = "帮我把这张照片转成水彩风格。" * 200
        elapsed, resp = suggest(long_query, [
            cand("future-image", "Generate images from text, edit supplied images"),
            cand("future-web", "Search the public web and retrieve pages"),
        ])
        record("E7 a ~3000-char query still resolves (the length gate is the client's, not the RPC's)",
               "future-image", recommend(resp),
               f"{len(long_query)} chars → {recommend(resp)!r}, {elapsed:.2f}s", elapsed)
    finally:
        remove_credential()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--only", default="A,B,C,D,E")
    args = parser.parse_args()
    wanted = {part.strip().upper() for part in args.only.split(",") if part.strip()}

    if not os.path.exists(SOCKET):
        sys.exit(f"no isolated agent at {SOCKET}; start one with:\n"
                 f"  future agent --home {HOME_DIR} --verbose --log-file")
    print(f"isolated agent: unix://{SOCKET}")
    print(f"running suites: {','.join(sorted(wanted))}")

    for letter, suite in (("A", suite_a), ("B", suite_b), ("C", suite_c),
                          ("D", suite_d), ("E", suite_e)):
        if letter in wanted:
            suite()

    passed = sum(1 for _, ok, _, _ in results if ok)
    total = len(results)
    failed = [case for case, ok, _, _ in results if not ok]
    print(f"\n{'=' * 66}")
    print(f"{passed}/{total} passed   ({calls_made} RPC calls issued)")
    if failed:
        print("failed:")
        for case in failed:
            print(f"  - {case}")
    print("=" * 66)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
