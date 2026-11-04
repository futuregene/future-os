//! Embedded terminals over the control plane.
//!
//! A paired desktop drives the *same* shells the person in front of the machine
//! sees: the registry is the host's, scoped to a conversation, and this module
//! is only the translation between its id space and the wire.
//!
//! ## Why commands and not a stream
//!
//! The local webview attaches over a WebSocket because both sides are on one
//! loopback and the transport is free. A paired desktop is not: every byte here
//! crosses the relay inside the sealed envelope the rest of the control plane
//! already uses. So the client *pulls*: `terminal_read` returns the bytes after
//! the cursor it last applied, which is the same absolute-offset contract the
//! loopback attachment has (`Attachment::attach(cursor)`), and `terminal_write`
//! pushes one keystroke batch. Nothing new is needed on the wire — no subject,
//! no lane, no binary frame — and an interactive shell over a relay is the one
//! place that shows the cost: an echo waits for a round trip, so this transport
//! is honest about being for "look at the build", not for `vim`.
//!
//! ## Scope
//!
//! A terminal belongs to a conversation, and a remote client names that
//! conversation by its **agent session id** (`IncomingCmd::session_id`), which
//! is what every other command here takes. The registry is keyed by thread, so
//! the two are mapped in one place — [`thread_for`] — and an unknown session is
//! refused with the same code its other commands use.
//!
//! ## What a paired device may do
//!
//! Anything this trait allows, which is: spawn a shell, type into it, resize it,
//! and kill it. That is deliberately wider than the agent's own surface — a
//! prompt passes through approval rules, a shell does not. The gate that stays
//! is control: `commands.rs` refuses every business command from a standby
//! device before it reaches a handler, so only the device the user handed the
//! machine to can open or type into a shell. See
//! `docs/internals/desktop/embedded-terminal.md` §Remote.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use future_remote_protocol::IncomingCmd;
use future_remote_server::services::ReplySink;
use serde_json::{json, Value};

use super::{prompt::UNKNOWN_SESSION_CODE, reply};
use crate::terminal_service::{self, TerminalError, TerminalInfo, TerminalService};

/// Largest page one `terminal_read` returns. The client's poll interval, not
/// this number, is what bounds a shell's output rate; the cap only keeps one
/// reply (and its base64 expansion) inside the wire budget when a command
/// floods — the client asks again from the returned cursor immediately.
pub(crate) const READ_PAGE_BYTES: usize = 64 * 1024;

/// Largest keyboard batch one `terminal_write` accepts. A paste larger than
/// this is split by the client; anything bigger arriving at once is refused
/// rather than pushed into a PTY in one unbounded write.
pub(crate) const WRITE_MAX_BYTES: usize = 256 * 1024;

pub(crate) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "terminal_list" => {
            let result = thread_for(&cmd.session_id).map(|thread| {
                let terminals = service().map(|service| service.list(&thread));
                (thread, terminals)
            });
            match result {
                Ok((_, Some(terminals))) => {
                    reply(
                        sink,
                        true,
                        json!({ "terminals": terminals.iter().map(info).collect::<Vec<_>>() }),
                        None,
                    )
                    .await;
                }
                // No registry in this process: the handshake should not have
                // advertised the capability, and saying "none" would read as a
                // conversation with no terminals rather than a machine without
                // the feature.
                Ok((_, None)) => refusal(sink, &TerminalError::Unavailable).await,
                Err(error) => refusal_text(sink, &error).await,
            }
        }
        "terminal_create" => match thread_for(&cmd.session_id) {
            Err(error) => refusal_text(sink, &error).await,
            Ok(thread) => {
                let result = service()
                    .ok_or(TerminalError::Unavailable)
                    .and_then(|service| {
                        let cols = dimension(cmd.cols);
                        let rows = dimension(cmd.rows);
                        let title = trimmed(&cmd.title);
                        service.create(&thread, title, cols, rows)
                    });
                result_reply(
                    sink,
                    result,
                    |terminal| json!({ "terminal": info(terminal) }),
                )
                .await;
            }
        },
        "terminal_update" => {
            let result = service()
                .ok_or(TerminalError::Unavailable)
                .and_then(|service| {
                    let title = trimmed(&cmd.title);
                    let size = (cmd.cols > 0 && cmd.rows > 0)
                        .then(|| (dimension(cmd.cols), dimension(cmd.rows)));
                    service.update(&cmd.terminal_id, title, size)
                });
            result_reply(
                sink,
                result,
                |terminal| json!({ "terminal": info(terminal) }),
            )
            .await;
        }
        "terminal_delete" => {
            let result = service()
                .ok_or(TerminalError::Unavailable)
                .and_then(|service| service.delete(&cmd.terminal_id));
            unit_reply(sink, result).await;
        }
        "terminal_read" => {
            let result = service()
                .ok_or(TerminalError::Unavailable)
                .and_then(|service| service.read(&cmd.terminal_id, cmd.terminal_cursor));
            match result {
                Ok(page) => {
                    // The page is bounded here as well as in the service: a
                    // command that floods (`cat` a big file) may hand back the
                    // whole retained tail, and one reply has to stay inside the
                    // wire budget. The cursor is therefore recomputed from what
                    // actually travels, so the client's next ask resumes exactly
                    // after the last byte it received.
                    let take = page.bytes.len().min(READ_PAGE_BYTES);
                    reply(
                        sink,
                        true,
                        json!({
                            "data": URL_SAFE_NO_PAD.encode(&page.bytes[..take]),
                            // Absolute end offset *after these bytes*, which is
                            // what the client stores and asks from next time.
                            "cursor": page.start + take as u64,
                            // Absolute offset of the first returned byte: a
                            // client that asked from further back than the
                            // retained tail is told so instead of shown a hole.
                            "start": page.start,
                            "status": page.status,
                            "exitCode": page.exit_code,
                        }),
                        None,
                    )
                    .await;
                }
                Err(error) => refusal(sink, &error).await,
            }
        }
        "terminal_write" => {
            let result = decode(&cmd.terminal_data).and_then(|bytes| {
                if bytes.len() > WRITE_MAX_BYTES {
                    return Err(TerminalError::Unusable(format!(
                        "TERMINAL_INPUT_TOO_LARGE: {} bytes in one write",
                        bytes.len()
                    )));
                }
                service()
                    .ok_or(TerminalError::Unavailable)
                    .and_then(|service| service.write(&cmd.terminal_id, &bytes))
            });
            unit_reply(sink, result).await;
        }
        _ => {
            reply(
                sink,
                false,
                Value::Null,
                Some("Unsupported terminal command"),
            )
            .await
        }
    }
}

/// The conversation's thread id, from the session id the wire names it by.
fn thread_for(session_id: &str) -> Result<String, crate::Error> {
    if session_id.trim().is_empty() {
        return Err("A session is required.".to_string().into());
    }
    crate::store::find_thread_by_agent_session(session_id)?
        .map(|thread| thread.id)
        .ok_or_else(|| {
            format!("{UNKNOWN_SESSION_CODE}: this conversation is not on this desktop").into()
        })
}

fn service() -> Option<&'static std::sync::Arc<dyn TerminalService>> {
    terminal_service::installed()
}

/// A viewport dimension from the wire: 0 means "not stated", and the registry
/// clamps the rest.
fn dimension(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn decode(value: &str) -> Result<Vec<u8>, TerminalError> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|error| TerminalError::Unusable(format!("TERMINAL_INPUT_INVALID: {error}")))
}

fn info(terminal: &TerminalInfo) -> Value {
    json!({
        "id": terminal.id,
        "threadId": terminal.thread_id,
        "title": terminal.title,
        "cwd": terminal.cwd,
        "status": terminal.status,
        "exitCode": terminal.exit_code,
        "cols": terminal.cols,
        "rows": terminal.rows,
    })
}

async fn result_reply<T>(
    sink: &dyn ReplySink,
    result: Result<T, TerminalError>,
    body: impl FnOnce(&T) -> Value,
) {
    match result {
        Ok(value) => reply(sink, true, body(&value), None).await,
        Err(error) => refusal(sink, &error).await,
    }
}

async fn refusal(sink: &dyn ReplySink, error: &TerminalError) {
    reply(sink, false, Value::Null, Some(&error.message())).await;
}

/// A command with nothing to return.
async fn unit_reply(sink: &dyn ReplySink, result: Result<(), TerminalError>) {
    match result {
        Ok(()) => reply(sink, true, json!({}), None).await,
        Err(error) => refusal(sink, &error).await,
    }
}

async fn refusal_text(sink: &dyn ReplySink, error: &crate::Error) {
    reply(sink, false, Value::Null, Some(&error.to_string())).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::{HomeGuard, RecordingSink};
    use crate::terminal_service::{TerminalPage, TerminalService};
    use std::sync::{Mutex, OnceLock};

    /// A registry with no PTY: it records what the wire asked it to do, so the
    /// translation is checked without spawning a shell. The shells themselves
    /// are covered by the Desktop's own terminal tests.
    #[derive(Default)]
    struct RecordingTerminals {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingTerminals {
        fn note(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn terminal(thread_id: &str) -> TerminalInfo {
        TerminalInfo {
            id: "term_1".into(),
            thread_id: thread_id.into(),
            title: "Terminal".into(),
            cwd: "/tmp".into(),
            status: "running".into(),
            exit_code: None,
            cols: 80,
            rows: 24,
        }
    }

    impl TerminalService for RecordingTerminals {
        fn list(&self, thread_id: &str) -> Vec<TerminalInfo> {
            self.note(format!("list:{thread_id}"));
            vec![terminal(thread_id)]
        }

        fn create(
            &self,
            thread_id: &str,
            title: Option<&str>,
            cols: u16,
            rows: u16,
        ) -> Result<TerminalInfo, TerminalError> {
            self.note(format!(
                "create:{thread_id}:{}:{cols}x{rows}",
                title.unwrap_or("-")
            ));
            Ok(terminal(thread_id))
        }

        fn update(
            &self,
            id: &str,
            title: Option<&str>,
            size: Option<(u16, u16)>,
        ) -> Result<TerminalInfo, TerminalError> {
            self.note(format!(
                "update:{id}:{}:{}",
                title.unwrap_or("-"),
                size.map(|(cols, rows)| format!("{cols}x{rows}"))
                    .unwrap_or_else(|| "-".into())
            ));
            Ok(terminal("thread_1"))
        }

        fn delete(&self, id: &str) -> Result<(), TerminalError> {
            self.note(format!("delete:{id}"));
            Ok(())
        }

        fn read(&self, id: &str, cursor: i64) -> Result<TerminalPage, TerminalError> {
            self.note(format!("read:{id}:{cursor}"));
            Ok(TerminalPage {
                bytes: b"hello".to_vec(),
                cursor: 5,
                start: 0,
                status: "running".into(),
                exit_code: None,
            })
        }

        fn write(&self, id: &str, bytes: &[u8]) -> Result<(), TerminalError> {
            self.note(format!("write:{id}:{}", String::from_utf8_lossy(bytes)));
            Ok(())
        }
    }

    /// Installing is process-wide, so these tests share one recording registry
    /// and assert on the calls it collected.
    fn registry() -> &'static std::sync::Arc<RecordingTerminals> {
        static INSTALLED: OnceLock<std::sync::Arc<RecordingTerminals>> = OnceLock::new();
        INSTALLED.get_or_init(|| {
            let registry = std::sync::Arc::new(RecordingTerminals::default());
            let dynamic: std::sync::Arc<dyn TerminalService> = registry.clone();
            assert!(terminal_service::install(dynamic));
            registry
        })
    }

    /// Every handler resolves the conversation first, so each test seeds the
    /// thread its session id names.
    fn fixture(test: &str) -> HomeGuard {
        let home = HomeGuard::new(test);
        crate::remote::test_support::init_store();
        crate::remote::test_support::thread_for_session("session-terminal");
        registry();
        home
    }

    fn command(cmd_type: &str) -> IncomingCmd {
        IncomingCmd {
            cmd_type: cmd_type.into(),
            session_id: "session-terminal".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_list_names_the_conversations_terminals() {
        let _home = fixture("terminal-list");
        let sink = RecordingSink::default();
        execute(&command("terminal_list"), &sink).await;
        let data = sink.ok_data();
        let terminals = data["terminals"].as_array().unwrap().clone();
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0]["id"], json!("term_1"));
        assert_eq!(terminals[0]["status"], json!("running"));
        assert_eq!(terminals[0]["cols"], json!(80));
        assert!(registry()
            .calls()
            .iter()
            .any(|call| call.starts_with("list:")));
    }

    #[tokio::test]
    async fn a_create_carries_the_title_and_viewport_verbatim() {
        let _home = fixture("terminal-create");
        let sink = RecordingSink::default();
        let mut cmd = command("terminal_create");
        cmd.title = "Build".into();
        cmd.cols = 120;
        cmd.rows = 40;
        execute(&cmd, &sink).await;
        assert_eq!(sink.ok_data()["terminal"]["title"], json!("Terminal"));
        let call = registry()
            .calls()
            .into_iter()
            .find(|call| call.starts_with("create:"))
            .expect("the registry was asked to create");
        assert!(call.ends_with(":Build:120x40"), "{call}");
    }

    #[tokio::test]
    async fn a_resize_and_rename_travel_as_one_update() {
        let _home = fixture("terminal-update");
        let sink = RecordingSink::default();
        let mut cmd = command("terminal_update");
        cmd.terminal_id = "term_1".into();
        cmd.cols = 100;
        cmd.rows = 30;
        execute(&cmd, &sink).await;
        sink.ok_data();
        assert!(registry()
            .calls()
            .iter()
            .any(|call| call == "update:term_1:-:100x30"));
    }

    #[tokio::test]
    async fn a_read_returns_the_page_and_the_cursor_to_ask_from() {
        let _home = fixture("terminal-read");
        let sink = RecordingSink::default();
        let mut cmd = command("terminal_read");
        cmd.terminal_id = "term_1".into();
        cmd.terminal_cursor = 7;
        execute(&cmd, &sink).await;
        let data = sink.ok_data();
        assert_eq!(data["data"], json!(URL_SAFE_NO_PAD.encode(b"hello")));
        assert_eq!(data["cursor"], json!(5));
        assert_eq!(data["start"], json!(0));
        assert!(registry()
            .calls()
            .iter()
            .any(|call| call == "read:term_1:7"));
    }

    #[tokio::test]
    async fn a_write_forwards_the_decoded_bytes_verbatim() {
        let _home = fixture("terminal-write");
        let sink = RecordingSink::default();
        let mut cmd = command("terminal_write");
        cmd.terminal_id = "term_1".into();
        cmd.terminal_data = URL_SAFE_NO_PAD.encode(b"pwd\r");
        execute(&cmd, &sink).await;
        sink.ok_data();
        assert!(registry()
            .calls()
            .iter()
            .any(|call| call == "write:term_1:pwd\r"));
    }

    #[tokio::test]
    async fn an_unknown_conversation_is_refused_by_code() {
        let _home = fixture("terminal-unknown-session");
        let sink = RecordingSink::default();
        let mut cmd = command("terminal_list");
        cmd.session_id = "session-that-is-not-here".into();
        execute(&cmd, &sink).await;
        assert!(
            sink.error_text().starts_with(UNKNOWN_SESSION_CODE),
            "{}",
            sink.error_text()
        );
    }

    #[tokio::test]
    async fn a_write_larger_than_one_batch_is_refused_instead_of_pushed() {
        let _home = fixture("terminal-write-large");
        let sink = RecordingSink::default();
        let before = registry().calls().len();
        let mut cmd = command("terminal_write");
        cmd.terminal_id = "term_1".into();
        cmd.terminal_data = URL_SAFE_NO_PAD.encode(vec![b'a'; WRITE_MAX_BYTES + 1]);
        execute(&cmd, &sink).await;
        assert!(
            sink.error_text().starts_with("TERMINAL_INPUT_TOO_LARGE"),
            "{}",
            sink.error_text()
        );
        assert_eq!(
            registry().calls().len(),
            before,
            "nothing reached the shell"
        );
    }
}
