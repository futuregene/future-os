//! A scripted agent for screenshots: a real gRPC server answering with a fixed
//! session, so `future tui` can be captured showing real transcript content
//! without a model, a network or a cent of credit.
//!
//! This is the TUI's equivalent of the desktop and mobile mock backends: the
//! *application* is the real binary, rendering real rows through the real code
//! path (`get_state` → `Live` → `WorkbenchData` → the frame; `get_session_entries`
//! → `session_rows::rows_to_messages` → `ChatArea`); only the data is scripted.
//! That is what makes a screenshot reproducible — the same command produces the
//! same picture on any machine — which a live model cannot.
//!
//! Usage (the screenshots harness starts it for you):
//!
//! ```text
//! cargo run -p future-tui --example demo_agent -- --port 5501
//! future tui --grpc-addr 127.0.0.1:5501
//! ```
//!
//! Three environment variables change what it answers, and a capture passes them
//! (see `scripts/screenshots/capture.py tui`):
//!
//! - `FUTURE_DEMO_LANG=en` — the session in English: the session list, the
//!   transcript, the streaming turn. Unset keeps the Chinese one.
//! - `FUTURE_DEMO_LONG=1` — extra turns until the transcript outruns a tall
//!   terminal's band, which is what a `scrolled` (or a deliberately full) frame
//!   needs.
//! - `FUTURE_DEMO_CWD=<path>` — the checkout the scripted session claims to sit
//!   in, for the one panel that reads a real repository.

use std::pin::Pin;

use futures_util::stream;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::transport::server::TcpIncoming;
use tonic::transport::Server;

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};

/// The session the demo agent pretends to have.
const SESSION_ID: &str = "demo-session";
const SESSION_CWD: &str = "/Users/you/future-os";
const MODEL: &str = "future/deepseek-flash";

/// The checkout the scripted session claims to sit in.
///
/// The fictional path is the default because a frame that names the machine it
/// was captured on is not a baseline. A surface that *reads* the checkout — the
/// git panel is the one — needs a real repository, so a capture can name one:
/// `FUTURE_DEMO_CWD=$PWD tui/scripts/shot.sh git`. Nothing changes unless it is
/// set, so every other scenario stays byte-identical across machines.
fn session_cwd() -> String {
    std::env::var("FUTURE_DEMO_CWD").unwrap_or_else(|_| SESSION_CWD.to_string())
}

/// The language the scripted *data* is in.
///
/// A capture pins the interface language (`capture.py tui --lang en`, which
/// writes it into the TUI's `settings.json`), but the chrome is only half of a
/// frame: a session list and a transcript are the agent's words, and a frame
/// whose sidebar says `Sessions` while its rows say `parser 的 off-by-one` is
/// neither language. So the harness passes the same choice here as
/// `FUTURE_DEMO_LANG`, and the scripted session is written in it — the English
/// payload is not a translation of the Chinese one, it is the session an
/// English reader would be looking at.
///
/// Unset or unknown keeps the Chinese payload, which is what every committed
/// frame was captured with; `system` is the TUI's own "follow the locale" and
/// has no data of its own, so it keeps them too.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DemoLang {
    Zh,
    En,
}

fn demo_lang() -> DemoLang {
    match std::env::var("FUTURE_DEMO_LANG")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "en" | "en-us" | "en_us" | "en-gb" | "en_gb" => DemoLang::En,
        _ => DemoLang::Zh,
    }
}

/// The scripted conversation: one exchange that reasons, reads a file, runs the
/// tests and answers. Written in the display projection's shape (`blocks`), so
/// it travels the same mapper a real session's journal does.
fn journal() -> Value {
    let mut entries = match demo_lang() {
        DemoLang::Zh => journal_zh(),
        DemoLang::En => journal_en(),
    };
    if std::env::var_os("FUTURE_DEMO_LONG").is_some() {
        long_turns(&mut entries);
    }
    json!({ "entries": entries })
}

fn journal_zh() -> Vec<Value> {
    vec![
        json!({
            "id": "e1", "kind": "user", "role": "user",
            "blocks": [{"kind": "text",
                "text": "parser 里有个 off-by-one，找出来并补一个回归测试"}],
        }),
        json!({
            "id": "e2", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "reasoning",
                "text": "先看 parse_expr 的边界条件，怀疑是最后一个 token 的处理。"}],
        }),
        json!({
            "id": "e3", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_read", "name": "read",
                        "arguments": {"path": "src/parser.rs"}}],
        }),
        json!({
            "id": "e4", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_read", "isError": false,
                "text": "142  fn parse_expr(&mut self) -> Result<Expr> {\n143      while self.peek() <= self.end { ... }\n"}],
        }),
        json!({
            "id": "e5", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_test", "name": "shell",
                        "arguments": {"command": "cargo test -p parser"}}],
        }),
        json!({
            "id": "e6", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_test", "isError": false,
                "text": "test result: ok. 40 passed; 0 failed\n"}],
        }),
        json!({
            "id": "e7", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text",
                "text": "`while self.peek() <= self.end` 在最后一个 token 上多读一次。改成 `<` 之后，41 个用例全过。"}],
        }),
    ]
}

/// The same session as an English reader would have it — and, deliberately, the
/// richest transcript the demo agent can draw: a heading, **bold** and *italic*
/// runs, inline code, a diff, a bordered table, a numbered list, a quote, and two
/// folded processes whose calls and thoughts a click opens.
///
/// Rows, not words, are what a frame is made of, so the shapes are one turn each
/// rather than the shortest thing that would parse: a panel is filled by content
/// a reader would recognise, and a screenshot exists to show what the surface can
/// draw.
fn journal_en() -> Vec<Value> {
    vec![
        json!({
            "id": "e1", "kind": "user", "role": "user",
            "blocks": [{"kind": "text",
                "text": "`1+2+` returns `3` instead of failing. There is an off-by-one in the expression parser — find it and add a regression test."}],
        }),
        json!({
            "id": "e2", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "reasoning",
                "text": "Start at `parse_expr`'s loop bound. If the cursor is compared with `<=`, \"at the end\" and \"past the end\" are the same state, and a trailing operator is accepted with an operand that is not there."}],
        }),
        json!({
            "id": "e3", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_read", "name": "read",
                        "arguments": {"path": "src/parser.rs", "offset": 138, "limit": 16}}],
        }),
        json!({
            "id": "e4", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_read", "isError": false,
                "text": "142  fn parse_expr(&mut self) -> Result<Expr> {\n143      let mut lhs = self.parse_atom()?;\n144      while self.peek() <= self.end {\n145          let op = self.next();\n146          let rhs = self.parse_atom()?;\n147          lhs = apply(lhs, op, rhs);\n148      }\n149      Ok(lhs)\n150  }\n"}],
        }),
        json!({
            "id": "e5", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_test", "name": "shell",
                        "arguments": {"command": "cargo test -p parser"}}],
        }),
        json!({
            "id": "e6", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_test", "isError": false,
                "text": "running 41 tests\ntest result: ok. 41 passed; 0 failed; 0 ignored\n"}],
        }),
        json!({
            "id": "e7", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text",
                "text": "### Diagnosis\n\n`parse_expr` compares the cursor with **`<=`**, so *\"at the end\"* and *\"past the end\"* are one state — the trailing operator is consumed and its missing operand becomes `lhs`.\n\n```rust\n// Stop *at* the last token, not one past it.\nwhile self.peek() < self.end {\n    let op = self.next();\n    lhs = apply(lhs, op, self.parse_atom()?);\n}\n```\n\n| Shape | Before | After |\n| --- | --- | --- |\n| 1+2+ | 3 | UnexpectedEof |\n| [1,2, | [1, 2] | UnexpectedEof |\n| f(a, | f(a) | UnexpectedEof |\n\nAll three are the **same mistake**, so one table-driven test covers them — see `src/parser/tests.rs`."}],
        }),
        json!({
            "id": "e8", "kind": "user", "role": "user",
            "blocks": [{"kind": "text",
                "text": "Does the same shape appear anywhere else? And land it as two commits — the bounds first, then the tests."}],
        }),
        json!({
            "id": "e9", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "reasoning",
                "text": "Search for the comparison itself rather than for `parse_expr`: the mistake is the bound, not the function."}],
        }),
        json!({
            "id": "e10", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_grep", "name": "shell",
                        "arguments": {"command": "rg -n 'peek\\(\\) <=' src/"}}],
        }),
        json!({
            "id": "e11", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_grep", "isError": false,
                "text": "src/parser.rs:144:        while self.peek() <= self.end {\nsrc/parser.rs:318:        while self.peek() <= self.end {\nsrc/args.rs:87:        while self.peek() <= self.end {\n"}],
        }),
        json!({
            "id": "e12", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "reasoning",
                "text": "Two more sites, both reachable from the same entry point. Bounds in one commit, the case table in the other, so reverting the tests cannot take the behaviour change with them — or leave a red suite in between."}],
        }),
        json!({
            "id": "e13", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_edit", "name": "edit",
                        "arguments": {"path": "src/args.rs", "old": "while self.peek() <= self.end {", "new": "while self.peek() < self.end {"}}],
        }),
        json!({
            "id": "e14", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_edit", "isError": false,
                "text": "patched src/args.rs:87\n1 file changed, 1 insertion(+), 1 deletion(-)\n"}],
        }),
        json!({
            "id": "e15", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "tool_call", "toolCallId": "call_commit", "name": "shell",
                        "arguments": {"command": "cargo test -p parser && git commit -am 'fix(parser): the loop bound'"}}],
        }),
        json!({
            "id": "e16", "kind": "tool", "role": "tool",
            "blocks": [{"kind": "tool_result", "toolCallId": "call_commit", "isError": false,
                "text": "test result: ok. 44 passed; 0 failed; 0 ignored\n[parser 6b1f0c2] fix(parser): the loop bound\n 3 files changed, 3 insertions(+), 3 deletions(-)\n"}],
        }),
        json!({
            "id": "e17", "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text",
                "text": "Landed in **two commits**, both green:\n\n1. `6b1f0c2` — the three bounds, *behaviour only*\n2. `d41e8aa` — the table-driven test, *coverage only*\n\n> `cargo test -p parser`: 44 passed, 0 failed, 0 ignored."}],
        }),
    ]
}

/// How many extra turns `FUTURE_DEMO_LONG=1` appends: enough rows to outrun the
/// transcript band of a tall terminal, which is what a `scrolled` capture needs.
const LONG_TURNS: usize = 16;

/// More exchanges after the tutorial's, for the one scenario that is *about*
/// scrolling (`scrolled`, which wheels up to reach the `↑ 更早` badge).
///
/// A conversation that fits the band has nowhere to scroll — the ceiling counts
/// the window as well as the rows (see `Workbench::scroll`) — so that shot needs
/// a transcript taller than the band. Opt-in rather than always: the other
/// scenarios' frames are byte-identical pictures, and rows nobody asked for
/// would change them.
fn long_turns(entries: &mut Vec<Value>) {
    match demo_lang() {
        DemoLang::Zh => long_turns_zh(entries),
        DemoLang::En => long_turns_en(entries),
    }
}

fn long_turns_zh(entries: &mut Vec<Value>) {
    for turn in 1..=LONG_TURNS {
        entries.push(json!({
            "id": format!("lu{turn}"), "kind": "user", "role": "user",
            "blocks": [{"kind": "text",
                "text": format!("第 {turn} 轮：这一层重试为什么不直接拿掉？")}],
        }));
        entries.push(json!({
            "id": format!("la{turn}"), "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text", "text": format!(
                "第 {turn} 轮：上游会在 45 秒静默断开，这一层把它挡在界面之外。\n日志里能查到 `retry attempt {turn}`，超时的那条没有收尾行。")}],
        }));
    }
}

/// The English long conversation: a reader working out why the retry layer
/// exists, one question at a time, with the log excerpt and the test that came
/// out of it.
///
/// Fixed turns rather than generated prose in a loop — a frame is compared run
/// to run, and a numbered sentence repeated sixteen times is a picture of a mock
/// rather than of a session. The round count comes from [`LONG_TURNS`] either
/// way, so both languages outrun the same band.
fn long_turns_en(entries: &mut Vec<Value>) {
    const TURNS: [(&str, &str); 8] = [
        (
            "why is this retry layer still here?",
            "Because the transport underneath it is silent rather than dead. The upstream stops writing after 45 seconds and never closes the socket, so nothing fires: no error, no EOF, no read timeout that we could lower. This layer is the only thing between that silence and a spinner nobody can explain.",
        ),
        (
            "is the 45 seconds ours or theirs?",
            "Theirs — it is the load balancer in front of the model gateway. Our client has a 5 second connect timeout and no read timeout at all, which is why a quiet connection looks, from inside the process, exactly like a slow answer.",
        ),
        (
            "what does the log say when it fires?",
            "Two lines, and the pair is the signal:\n\n```\n2026-10-29T09:41:12Z  retry attempt 3  session=demo  elapsed=45.0s\n2026-10-29T09:41:12Z  stream reopened     session=demo  attempt=1\n```\n\nThe attempt that timed out has no closing line: it is abandoned, not failed. Every deadline here should end with something — `finish_incomplete` is what the run record calls it.",
        ),
        (
            "could we just lower our timeout?",
            "For this path, yes: 20 seconds is already longer than a healthy first chunk. But that is a fix with a cost — a slow model on a busy machine would be cut off mid-answer — so it belongs behind the layer, as a budget the layer enforces, not as a socket option.",
        ),
        (
            "where does the attempt counter live?",
            "In the run record (`run_epoch`), not in the transport. That is deliberate: a reconnect that lands on the same run has to keep its epoch, and the counter is what says whether the reply the reader sees is still the one they asked for. Reset it on a new prompt, never on a new socket.",
        ),
        (
            "what happens to the partial reply?",
            "It stays, and that is the point of retrying rather than restarting. The transcript keeps the chunks that arrived, the activity row says `retrying`, and the new attempt is asked to continue instead of to answer again — repeating the opening paragraph is the one failure readers notice immediately.",
        ),
        (
            "do the tests cover the silent path?",
            "One does now. It fakes a stream that delivers two chunks and then goes quiet:\n\n- the run must not end\n- a reopen must arrive before the third read\n- the epoch must not change\n- the transcript must keep both chunks\n\nIt failed the first time for a boring reason — the fake held the sender, so the client saw a *closed* stream (an error) instead of a silent one. Silence has to be arranged with a task that never finishes, which is the same thing the demo agent below does on purpose.",
        ),
        (
            "is there a cheaper signal than a timer?",
            "Yes, when one exists — a keepalive from the server, or a byte count the gateway can report. There is none here today, so the timer is the honest answer: 45 seconds measured from the last byte rather than from the request, which is what makes it survive a slow first chunk and still catch a dead one.\n\nWhen the gateway grows a keepalive, this layer keeps its shape and loses its clock: the budget becomes \"three missed keepalives\" instead of \"45 seconds\". Until then, the number stays where it is and the log line stays paired.",
        ),
    ];
    for (index, (question, answer)) in TURNS.iter().enumerate() {
        let turn = index + 1;
        entries.push(json!({
            "id": format!("lu{turn}"), "kind": "user", "role": "user",
            "blocks": [{"kind": "text", "text": *question}],
        }));
        entries.push(json!({
            "id": format!("la{turn}"), "kind": "assistant", "role": "assistant",
            "blocks": [{"kind": "text", "text": *answer}],
        }));
    }
}

/// `get_state`: what the header, the status bar and the session list draw.
fn state() -> Value {
    let mut state = json!({
        "sessionId": SESSION_ID,
        "sessionName": session_name(),
        "cwd": session_cwd(),
        "model": MODEL,
        "thinkingLevel": "medium",
        "sandboxTier": "manual",
        "permissionLevel": "workspace",
        "isStreaming": false,
        "version": future_rpc::connection::EXPECTED_AGENT_VERSION,
        "skills": ["future-versioning", "future-peer-review", "future-web"],
        "contextFiles": ["AGENTS.md"],
        "contextTokens": 12_400,
        "contextWindow": 200_000,
        "contextPercent": 6.2,
        // `inputTokens` is the session's `prompt_tokens`, so the cached part is
        // inside it (88.0k + 2.1k of 92.0k → a 96% hit rate). A stand-in that
        // reports more cache than prompt would paint a footer no real session
        // can produce.
        "usage": {
            "inputTokens": 92_000, "outputTokens": 3_300,
            "cacheReadTokens": 88_000, "cacheWriteTokens": 2_100,
            "costCny": 0.0137
        }
    });
    // `FUTURE_DEMO_QUEUE=1` also reports a turn in flight and the prompts waiting
    // behind it, which is what the composer's queue band draws. Gated like
    // `FUTURE_DEMO_CWD`: a stand-in that always had a queue would put a band in
    // every other frame's baseline.
    if std::env::var_os("FUTURE_DEMO_QUEUE").is_some() {
        state["isStreaming"] = json!(true);
        state["activeRun"] = json!({
            "runId": "demo-run-live", "epoch": 1, "state": "running", "lastEventIdx": 0
        });
        // The prompts waiting behind the reply, in the frame's own language: a
        // queue band is a list of the reader's *words*, so a Chinese one in an
        // English frame reads as a mock (the same reason the transcript is
        // localized). One entry carries a newline on purpose — the band shows it
        // as `↵`, which is how a two-line prompt survives being one row.
        let queued: [(&str, &str); 3] = match demo_lang() {
            DemoLang::Zh => [
                ("顺手把 tui 的 clippy 警告也清掉", "2026-03-04T10:00:00Z"),
                (
                    "再看一下 diff 面板的滚动，别一滚就跳到底\n顺便补个回归测试",
                    "2026-03-04T10:00:04Z",
                ),
                ("跑完把两处改动的 sha 贴一下", "2026-03-04T10:00:09Z"),
            ],
            DemoLang::En => [
                ("Also clear the clippy warnings in `tui`", "2026-03-04T10:00:00Z"),
                (
                    "Then look at the diff panel's scrolling — it jumps to the\nbottom. Add a regression test for it.",
                    "2026-03-04T10:00:04Z",
                ),
                ("Paste both commit shas when you are done", "2026-03-04T10:00:09Z"),
            ],
        };
        state["queuedRuns"] = Value::Array(
            queued
                .iter()
                .enumerate()
                .map(|(index, (text, accepted))| {
                    json!({
                        "runId": format!("demo-run-queued-{}", index + 1),
                        "runSequence": index + 2,
                        "clientRequestId": format!("demo-request-queued-{}", index + 1),
                        "state": "queued",
                        "queuePosition": index + 1,
                        "acceptedAt": accepted,
                        "displayText": text,
                    })
                })
                .collect(),
        );
        state["queuedCount"] = json!(queued.len());
    }
    state
}

fn session_name() -> &'static str {
    match demo_lang() {
        DemoLang::Zh => "parser 的 off-by-one",
        // Title case, like every other English title here: a session name in a
        // rail is a title, and the TUI does not restyle what the agent sends.
        DemoLang::En => "Off-by-One in the Expression Parser",
    }
}

/// The sessions the rail lists.
///
/// The Chinese list is the three the committed frames were captured with. The
/// English one is longer on purpose: a rail three rows tall is a picture of a
/// mock, and the sidebar is half of the frame this payload exists for — a
/// working checkout files a dozen conversations under a couple of workspaces,
/// and that is what an English frame should show.
fn sessions() -> Value {
    let sessions = match demo_lang() {
        DemoLang::Zh => json!([
            {"id": SESSION_ID, "sessionName": session_name(), "cwd": session_cwd(),
             "model": MODEL, "updatedAtMs": now_ms(), "queryCount": 1, "isStreaming": false},
            {"id": "demo-older", "sessionName": "workbench 布局草图",
             "cwd": "/Users/you/future-os/.worktrees/ui", "model": MODEL,
             "updatedAtMs": now_ms() - 3_600_000, "queryCount": 12, "isStreaming": false},
            {"id": "demo-oldest", "sessionName": "nightly coverage run",
             "cwd": "/Users/you/future-os", "model": MODEL,
             "updatedAtMs": now_ms() - 86_400_000, "queryCount": 4, "isStreaming": false}
        ]),
        DemoLang::En => {
            // `(title, workspace, minutes ago, queries)`, newest first. Ages are
            // in minutes so the rail's own `12m` / `3h` / `2d` stamps come out of
            // the surface rather than being written here as text. A working
            // checkout files a railful under a couple of workspaces — three rows
            // of sessions next to a transcript the height of the band is a
            // picture of a mock, not of a session list.
            //
            // Titles are title case. A session name is a *title* in the rail, and
            // the surface prints whatever the agent sent, so the case has to come
            // from here.
            const HOME: &str = "/Users/you/future-os";
            const UI: &str = "/Users/you/future-os/.worktrees/ui";
            const NOTES: &str = "/Users/you/notes";
            const OTHERS: [(&str, &str, i64, u32); 64] = [
                ("Retry Layer Hides a Silent Stream", HOME, 8, 23),
                ("Rail: Fold a Workspace Shut", HOME, 26, 9),
                ("Palette Ranking for Short Queries", HOME, 51, 14),
                ("Coverage Gate for the Goal Plane", HOME, 95, 31),
                ("SQLite View Prepare Is 12ms", HOME, 140, 7),
                ("Workbench Layout Sketch", UI, 240, 12),
                ("48s of Silence, Then a Reopen", HOME, 300, 18),
                ("Split the Retry out of the Bridge", HOME, 420, 6),
                ("Keymap Audit: ctrl+g and ctrl+d", UI, 600, 11),
                ("Windows Named Pipe Smoke Test", HOME, 1_500, 4),
                ("Nightly Coverage Run", HOME, 2_100, 4),
                ("Notes on the Compaction Numbers", NOTES, 3_000, 3),
                ("Directory Layout Page", NOTES, 5_400, 2),
                ("The Palette Needs a Filter Row", UI, 6_200, 8),
                ("To-do Panel: Dedupe the Goals", HOME, 6_000, 5),
                ("Stream Events Arrive Twice", HOME, 6_432, 16),
                ("Tool Results as Transcript Rows", HOME, 6_896, 9),
                ("Mobile: Follow-up Opens a New Chat", HOME, 7_393, 21),
                ("Session Recency Is Never Updated", HOME, 7_925, 13),
                ("Delete a Batch of Sessions", HOME, 8_496, 4),
                ("The 45s Deadline Has No Closing Line", HOME, 9_109, 17),
                ("Cache Hit Rate Is 96 Percent", HOME, 9_765, 6),
                ("Cost per Run, in Credit Units", NOTES, 10_468, 3),
                ("One Commit per Parser, or Three", HOME, 11_223, 2),
                ("Guide: Where Each File Lives", NOTES, 12_031, 5),
                ("Screenshots: Pin the Rail Width", UI, 12_898, 7),
                ("A grpcurl Probe in a Scratch Home", HOME, 13_827, 8),
                ("The Goal Plane Forgets a Todo", HOME, 14_824, 11),
                ("What the Flaky Test Was Waiting For", HOME, 15_892, 9),
                ("Windows: The Lock File Is Not Atomic", HOME, 17_037, 6),
                ("One Prompt, Two Sessions", HOME, 18_264, 4),
                ("The Transcript Loses Its Last Row", HOME, 19_580, 12),
                ("Colour Contrast in the Light Theme", UI, 20_991, 9),
                ("Scroll: The Ceiling Counts the Window", HOME, 22_504, 7),
                ("Help Overlay Lists Verbs It Lacks", HOME, 24_125, 3),
                ("Typed RPC: Retire the JSON Fallback", HOME, 25_863, 15),
                ("Branch Protection Wants Main Merged", HOME, 27_727, 2),
                ("Coverage: The src-tauri Pair", HOME, 29_725, 8),
                ("Why the Wheel Jumps Two Lines", UI, 31_866, 5),
                ("Tool Call Arguments Are Truncated", HOME, 34_163, 10),
                ("Reading a 40MB Log Without Freezing", HOME, 36_624, 6),
                ("The Model Picker Remembers Nothing", HOME, 39_263, 4),
                ("Diff Review: Fold Long Hunks", UI, 42_092, 11),
                ("i18n: The Usage Panel in English", HOME, 45_125, 7),
                ("Sandbox Tier Names Are Misleading", NOTES, 48_376, 3),
                ("Keep the Worktree Until It Lands", HOME, 51_862, 2),
                ("Windows: The Pipe Name Is Per User", HOME, 55_598, 5),
                ("Two Agents on One Socket", HOME, 59_604, 3),
                ("The Composer Grows With the Draft", UI, 63_899, 9),
                ("Screenshots: A Wider Rail", UI, 68_503, 4),
                ("Markdown Tables Break on Wide Cells", HOME, 73_439, 6),
                ("The Diff Panel Reads the Index", HOME, 78_730, 7),
                ("Stale Locks in the Finish Lock", HOME, 84_403, 2),
                ("Coverage: Merge the Two Runs", HOME, 90_484, 8),
                ("A Session List That Fits", NOTES, 97_004, 5),
                ("Reading the Goal Plane's Ledger", HOME, 103_993, 11),
                ("One Prompt, Two Answers", HOME, 111_486, 3),
                ("Where the Context Window Grows", HOME, 119_519, 5),
                ("Screenshots: The Queue Band", UI, 128_131, 4),
                ("A Prompt That Is Two Lines", HOME, 137_363, 3),
                ("Sent Together, Not One by One", HOME, 147_260, 6),
                ("The Queue Survives a Reopen", HOME, 157_871, 2),
                ("The /queue List Has Its Own Rows", UI, 169_245, 5),
                ("Closing the Editor Keeps the Draft", HOME, 181_440, 3),
            ];
            // The session the frame opens is the newest row, so it cannot be in
            // the const table: its title comes from `session_name()`.
            let mut rows: Vec<(&str, &str, i64, u32)> = Vec::with_capacity(OTHERS.len() + 1);
            rows.push((session_name(), HOME, 0, 1));
            rows.extend(OTHERS);
            Value::Array(
                rows.iter()
                    .enumerate()
                    .map(|(index, (title, cwd, minutes, queries))| {
                        let id = match index {
                            0 => SESSION_ID.to_string(),
                            other => format!("demo-en-{}", other + 1),
                        };
                        json!({
                            "id": id, "sessionName": title, "cwd": cwd, "model": MODEL,
                            "updatedAtMs": now_ms() - minutes * 60_000,
                            "queryCount": queries, "isStreaming": false
                        })
                    })
                    .collect(),
            )
        }
    };
    json!({ "sessions": sessions })
}

fn models() -> Value {
    json!({"models": [
        {"id": "deepseek-flash", "label": "DeepSeek Flash", "provider": "future",
         "supportsImages": false, "thinkingLevel": "medium", "contextWindow": 200_000,
         "isDefault": true},
        {"id": "claude-sonnet-4", "label": "Claude Sonnet", "provider": "anthropic",
         "supportsImages": true, "thinkingLevel": "medium", "contextWindow": 200_000,
         "isDefault": false}
    ]})
}

/// What the scripted *live* turn says, one delta at a time.
///
/// A screenshot of a reply that is still arriving needs the agent to emit
/// events, and a real model cannot: the frame would differ per run and per
/// machine. So the tutorial's answer is a fixed list of deltas — the last one
/// deliberately stops mid-sentence, which is what a frame captured mid-stream
/// looks like. The surface lays it out through its ordinary path
/// (`Live::handle_event` → `ChatArea` → the frame); only the words are scripted.
const STREAM_DELTAS: [&str; 44] = [
    "先看边界：",
    "`parse_expr` 在循环里先消费了运算符，",
    "再检查 `peek()`，",
    "于是最后一个 token 永远没有机会触发收尾分支。\n\n",
    "复现也简单，",
    "输入 `1+2` 是对的，",
    "输入 `1+2+` 就少一个右操作数",
    "——",
    "因为循环在**先**前进后才判断端点。\n\n",
    "修法是把检查提到前进之前：",
    "\n\n```rust\n",
    "while let Some(tok) = lexer.peek() {\n",
    "    if tok.is_eof() { break; }\n",
    "    let op = lexer.next();\n",
    "    let rhs = parse_atom(lexer)?;\n",
    "    lhs = apply(lhs, op, rhs);\n",
    "}\n",
    "```\n\n",
    "回归测试就盯这一件事：",
    "一个以运算符结尾的表达式",
    "必须报 `UnexpectedEof`，",
    "而不是悄悄返回左边那一项。",
    "\n\n",
    "另外还有两处同样的写法，",
    "分别在 `parse_list` 和 `parse_args` 里；",
    "我先把这三处一起改掉，",
    "然后跑一遍 `cargo test -p parser`。",
    "\n\n",
    "测试这边我加了两条：",
    "一条钉住 `1+2+`，",
    "一条钉住 `[1,2,`",
    "和 `f(a,` 的空参数情况。",
    "\n\n",
    "顺便说一句，",
    "这类 off-by-one 都长成同一个形状：",
    "**先推进、后判断**。",
    "所以在 review 里看到 `next()` 出现在条件之前，",
    "就值得多看一眼，",
    "哪怕当下测试是绿的。",
    "\n\n",
    "要我把这三处的改动拆成两个 commit 吗？",
    "一个只做循环边界的调整，",
    "另一个补测试，",
    "这样 revert 的粒度",
];

/// The English streaming turn, delta for delta: the same answer, still arriving,
/// stopping mid-word on purpose. Used by `--lang en --scenario streaming`.
const STREAM_DELTAS_EN: [&str; 29] = [
    "Start with the bound: ",
    "`parse_expr` consumes the operator before it checks for the end of input, ",
    "so the last token never reaches the branch that would close the expression.\n\n",
    "It reproduces in one line — ",
    "`1+2` is fine, ",
    "`1+2+` comes back with a missing right-hand side. ",
    "The loop advanced **before** it looked.\n\n",
    "The fix is to check before the advance:\n\n```rust\n",
    "while let Some(tok) = lexer.peek() {\n",
    "    if tok.is_eof() { break; }\n",
    "    let op = lexer.next();\n",
    "    let rhs = parse_atom(lexer)?;\n",
    "    lhs = apply(lhs, op, rhs);\n",
    "}\n",
    "```\n\n",
    "The regression test pins exactly that: ",
    "an expression that ends on an operator ",
    "has to report `UnexpectedEof` ",
    "instead of quietly returning the left-hand side.\n\n",
    "There are two more sites with the same shape, ",
    "in `parse_list` and `parse_args`, ",
    "and I would fix all three together ",
    "before running `cargo test -p parser`.\n\n",
    "One thing worth saying about the class of bug: ",
    "it is always the same picture — ",
    "**advance, then check**. ",
    "So a `next()` sitting above a condition in review ",
    "is worth a second look, ",
    "even while the tests are green.",
];

/// One delta every this many milliseconds: slow enough that a screenshot catches
/// the reply in flight, fast enough that a capture does not wait on prose.
const STREAM_DELTA: std::time::Duration = std::time::Duration::from_millis(120);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// The demo agent's handle on its event stream.
///
/// `stream_events` is where the client's subscription lands, and a scripted turn
/// has to write into that very stream — so the sender is kept here instead of
/// being parked inside the function (see the note there). `None` before any
/// client subscribes: a prompt that arrives that early has nowhere to go, and
/// the surface is not watching anyway.
#[derive(Clone, Default)]
struct DemoAgent {
    events:
        std::sync::Arc<std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<StreamEvent>>>>,
}

impl DemoAgent {
    /// Emit the scripted turn on the subscribed stream: `agent_start`, then one
    /// `text_chunk` per [`STREAM_DELTAS`] entry, then nothing.
    ///
    /// Nothing after the last delta is the point — an `agent_end` would take the
    /// activity row away and leave a *finished* reply on screen, while a
    /// screenshot of streaming output needs the turn still in flight. The
    /// surface keeps drawing its spinner for as long as the capture runs.
    fn start_turn(&self, run_id: &str) {
        let Some(tx) = self.events.lock().ok().and_then(|slot| slot.clone()) else {
            return;
        };
        let run_id = run_id.to_string();
        let deltas: &[&str] = match demo_lang() {
            DemoLang::Zh => &STREAM_DELTAS,
            DemoLang::En => &STREAM_DELTAS_EN,
        };
        tokio::spawn(async move {
            let event = |kind: &str, data: Value| StreamEvent {
                r#type: kind.to_string(),
                session_id: SESSION_ID.to_string(),
                run_id: run_id.clone(),
                data: data.to_string(),
                ..Default::default()
            };
            if tx
                .send(event(
                    "agent_start",
                    json!({"model": MODEL, "sessionId": SESSION_ID}),
                ))
                .is_err()
            {
                return;
            }
            for delta in deltas {
                tokio::time::sleep(STREAM_DELTA).await;
                if tx
                    .send(event("text_chunk", json!({"text": delta})))
                    .is_err()
                {
                    return;
                }
            }
        });
    }
}

#[tonic::async_trait]
impl FutureAgent for DemoAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        let data: Value = match cmd.r#type.as_str() {
            // The connect handshake: the client refuses an agent whose version
            // or protocol differs, so answer it exactly.
            "get_agent_readiness" => json!({
                "version": future_rpc::connection::EXPECTED_AGENT_VERSION,
                "agentInstanceId": "demo-agent",
                "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId": "0".repeat(64),
                "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION
            }),
            "get_state" => state(),
            "list_sessions" => sessions(),
            "list_models" => models(),
            "get_session_entries" => journal(),
            // A launchpad submit creates the session it then prompts; answer with
            // the id the scripted session has so both halves name one session.
            "new_session" => json!({"sessionId": SESSION_ID}),
            // A prompt needs a model in a real agent. Here it starts the scripted
            // turn: the ack is what the surface expects from a running run, and
            // the deltas arrive on the event stream this agent was subscribed to.
            "prompt" => {
                let run_id = format!("demo-run-{}", now_ms());
                self.start_turn(&run_id);
                // `RunAck` reads snake_case (`run_id`, `run_epoch`,
                // `accepted_state`): a camelCase ack is what produced a
                // "Failed to send: missing field `run_id`" notice the first
                // time a scripted turn was screenshotted.
                json!({"run_id": run_id, "run_epoch": 1, "accepted_state": "running"})
            }
            _ => json!({}),
        };
        Ok(tonic::Response::new(RpcResponse {
            id: cmd.id,
            r#type: "response".into(),
            command: cmd.r#type.clone(),
            success: true,
            data: data.to_string(),
            error: String::new(),
            error_code: String::new(),
            error_data: String::new(),
            payload: None,
        }))
    }

    type StreamEventsStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        // One `ping` so the client's connected edge fires, then stay open: a
        // closed stream would put the surface in its reconnecting state, which
        // is not what a screenshot should show.
        //
        // "Stay open" has to be arranged explicitly. A sender bound in this
        // function is dropped the moment it returns, which closes the receiver
        // the returned stream is reading from — the client then sees a stream
        // that ended right after its first frame, flips to disconnected, and
        // every later call spends its connect wait before running. Measured:
        // the surface never painted at all. So the sender is parked in a task
        // that never finishes instead of living on the stack.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
        // The scripted turn writes into this same stream, so the sender is kept
        // on the service rather than only in the parked task below.
        if let Ok(mut slot) = self.events.lock() {
            *slot = Some(tx.clone());
        }
        tokio::spawn(async move {
            let _held = tx;
            std::future::pending::<()>().await;
        });
        let first = StreamEvent {
            r#type: "ping".into(),
            data: String::new(),
            ..Default::default()
        };
        let stream =
            stream::once(async move { Ok(first) }).chain(UnboundedReceiverStream::new(rx).map(Ok));
        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

#[tokio::main]
async fn main() {
    let mut port: u16 = 5501;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => match args.next().and_then(|value| value.parse().ok()) {
                Some(value) => port = value,
                None => {
                    eprintln!("demo_agent: --port needs a number");
                    std::process::exit(2);
                }
            },
            "--help" | "-h" => {
                println!(
                    "demo_agent — a scripted agent for TUI screenshots\n\
                     \n\
                     usage: demo_agent [--port N]\n\
                     \n\
                     Serves a fixed session on 127.0.0.1:N; point the TUI at it:\n\
                     \x20 future tui --grpc-addr 127.0.0.1:N"
                );
                return;
            }
            other => {
                eprintln!("demo_agent: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    // Port 0 is useful for tests: the OS picks one and the line below reports
    // it, which is how a caller learns where to connect.
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("demo_agent: cannot bind 127.0.0.1:{port}: {error}");
            std::process::exit(1);
        }
    };
    let addr = listener
        .local_addr()
        .expect("bound listener has an address");
    println!("demo_agent listening on {addr}");
    let incoming = TcpIncoming::from_listener(listener, true, None).expect("listener usable");
    let _ = Server::builder()
        .add_service(FutureAgentServer::new(DemoAgent::default()))
        .serve_with_incoming(incoming)
        .await;
}
