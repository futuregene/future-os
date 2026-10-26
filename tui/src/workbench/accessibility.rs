//! Screen-reader-oriented output and terminal colour preferences.
//!
//! Linear reading is not a differently arranged alternate-screen buffer: its
//! transcript is appended to the terminal's native scrollback. Only one small
//! editable prompt is redrawn in place.

use crate::utils::{strip_ansi_codes, truncate_to_width, TruncateOptions};

/// Convert a message or streamed text to terminal-safe Unicode text.
/// Formatting escapes and controls are removed, never sent back to the tty.
pub fn plain(text: &str) -> String {
    strip_ansi_codes(text)
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}

/// Incremental native-scrollback writer. Incomplete lines wait for the next
/// delta, so a streaming reply is never repeated once per token.
#[derive(Default)]
pub struct LinearReader {
    session: String,
    pending: String,
    streamed: String,
}

impl LinearReader {
    /// Replay once on entry and after switching conversations. Empty rows and
    /// terminal styling do not become noise in a screen reader's buffer.
    pub fn sync_session(&mut self, session: &str, messages: &[String]) -> String {
        if self.session == session {
            return String::new();
        }
        self.session = session.to_string();
        self.pending.clear();
        self.streamed.clear();
        let body = messages
            .iter()
            .map(|row| plain(row))
            .filter(|row| !row.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        terminated(&body)
    }

    /// A message committed by a local action is not a stream delta.
    pub fn committed(&self, message: &str) -> String {
        terminated(&plain(message))
    }

    /// Accept only assistant text deltas. Tool events and spinner ticks do not
    /// manufacture a second, inaccessible live-status stream.
    pub fn event(
        &mut self,
        event: &crate::rpc::types::AgentEvent,
        lang: super::i18n::Language,
    ) -> String {
        match event.r#type.as_str() {
            "agent_start" => {
                self.pending.clear();
                self.streamed.clear();
                String::new()
            }
            "text_chunk" => {
                let text = plain(event.text());
                self.pending.push_str(&text);
                self.streamed.push_str(&text);
                self.take_lines()
            }
            "agent_end" => {
                let final_text = plain(event.text());
                // The final event repeats the streamed prefix. Append only
                // its unseen suffix, including when the last emitted chunk
                // ended in a newline and left `pending` empty.
                if let Some(suffix) = final_text.strip_prefix(&self.streamed) {
                    self.pending.push_str(suffix);
                }
                self.streamed.clear();
                terminated(&std::mem::take(&mut self.pending))
            }
            "approval_request" => {
                let title = event
                    .data
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| super::i18n::t(lang, super::i18n::Key::LinearTool).into());
                terminated(&super::i18n::render(
                    lang,
                    super::i18n::Key::LinearApproval,
                    &[&plain(&title)],
                ))
            }
            "error" => {
                let error = event
                    .data
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .filter(|error| !error.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        super::i18n::t(lang, super::i18n::Key::LinearUnknownError).into()
                    });
                terminated(&super::i18n::render(
                    lang,
                    super::i18n::Key::LinearError,
                    &[&plain(&error)],
                ))
            }
            _ => String::new(),
        }
    }

    /// Keep a partial streamed answer while an alternate-screen overlay is
    /// visible. The final event's prefix still matches `streamed`, so it is
    /// emitted once when the reader returns to native scrollback.
    pub fn take_partial(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }

    fn take_lines(&mut self) -> String {
        let end = self
            .pending
            .char_indices()
            .filter(|(_, ch)| *ch == '\n')
            .map(|(index, _)| index + 1)
            .next_back();
        end.map(|end| self.pending.drain(..end).collect())
            .unwrap_or_default()
    }

    /// The one redrawn line. Transcript history above it remains ordinary
    /// terminal scrollback and can be read with the terminal/screen reader's
    /// own navigation instead of FutureOS's alternate-screen shortcuts.
    pub fn prompt(&self, draft: &str, width: u16) -> String {
        let draft = plain(draft)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let label = "future> ";
        truncate_to_width(
            &format!("{label}{draft}"),
            width.max(1) as usize,
            &TruncateOptions {
                ellipsis: true,
                pad: false,
            },
        )
    }

    /// Clear just the prompt, append complete lines, then restore the prompt.
    pub fn frame(&self, output: &str, draft: &str, width: u16) -> String {
        let output = output.replace("\r\n", "\n").replace('\n', "\r\n");
        format!("\r\x1b[2K{output}{}", self.prompt(draft, width))
    }
}

fn terminated(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

/// `COLORFGBG` is the portable, query-free system light/dark hint supported by
/// terminals such as xterm. Unknown or malformed values keep the dark default.
pub fn system_theme_id(colorfgbg: Option<&str>) -> &'static str {
    let Some(background) = colorfgbg
        .and_then(|value| value.rsplit(';').next())
        .and_then(|value| value.trim().parse::<u8>().ok())
    else {
        return "dark";
    };
    // The conventional xterm default is 0 on 15. The xterm 256-colour cube's
    // entries 0..=7 are dark and 8..=15 are bright; high cube/grey entries are
    // light enough that a dark foreground is the useful system match.
    if background == 15 || background >= 231 || (8..=15).contains(&background) {
        "light"
    } else {
        "dark"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::types::AgentEvent;
    use serde_json::json;

    fn event(kind: &str, text: &str) -> AgentEvent {
        AgentEvent {
            r#type: kind.into(),
            data: json!({"text": text}),
            ..Default::default()
        }
    }

    fn read(reader: &mut LinearReader, event: &AgentEvent) -> String {
        reader.event(event, crate::workbench::i18n::Language::En)
    }

    #[test]
    fn replay_is_plain_once_per_session_and_streaming_never_repeats_lines() {
        let mut reader = LinearReader::default();
        let history = vec!["\x1b[31mYou: old\x1b[m".into(), "Assistant: earlier".into()];
        assert_eq!(
            reader.sync_session("a", &history),
            "You: old\nAssistant: earlier\n"
        );
        assert_eq!(reader.sync_session("a", &history), "");
        assert_eq!(reader.committed("You: new"), "You: new\n");
        assert_eq!(read(&mut reader, &event("text_chunk", "first")), "");
        assert_eq!(
            read(&mut reader, &event("text_chunk", " line\npartial")),
            "first line\n"
        );
        assert_eq!(read(&mut reader, &event("text_chunk", " answer")), "");
        assert_eq!(
            read(&mut reader, &event("agent_end", "partial answer")),
            "partial answer\n"
        );
        assert_eq!(
            reader.sync_session("b", &history),
            "You: old\nAssistant: earlier\n"
        );
    }

    #[test]
    fn final_only_answers_and_untrusted_controls_are_handled() {
        let mut reader = LinearReader::default();
        reader.sync_session("a", &[]);
        assert_eq!(read(&mut reader, &event("agent_end", "done")), "done\n");
        assert_eq!(
            read(&mut reader, &event("text_chunk", "first\n")),
            "first\n"
        );
        assert_eq!(
            read(&mut reader, &event("agent_end", "first\nsecond")),
            "second\n"
        );
        assert_eq!(read(&mut reader, &event("text_chunk", "stale")), "");
        assert_eq!(read(&mut reader, &event("agent_start", "")), "");
        assert_eq!(read(&mut reader, &event("text_chunk", "fresh")), "");
        assert_eq!(read(&mut reader, &event("agent_end", "fresh")), "fresh\n");
        assert_eq!(
            read(&mut reader, &event("text_chunk", "\x1b]52;c;bad\x07ok\n")),
            "ok\n"
        );
        assert!(reader.prompt("two\nlines\x07", 80).contains("two lines"));
        let frame = reader.frame("new\n", "draft", 80);
        assert!(frame.contains("new\r\nfuture> draft"));
        assert!(!frame.contains("\x1b[31m"));
    }

    #[test]
    fn an_interrupted_line_survives_an_overlay_and_finishes_only_once() {
        let mut reader = LinearReader::default();
        reader.sync_session("a", &[]);
        assert_eq!(read(&mut reader, &event("text_chunk", "partial")), "");
        assert_eq!(reader.take_partial(), "partial");
        let mut deferred = String::from("partial");
        assert_eq!(
            crate::workbench::run::emit_linear(
                read(&mut reader, &event("agent_end", "partial answer")),
                &mut deferred,
                false,
            ),
            ""
        );
        assert_eq!(
            crate::workbench::run::emit_linear(String::new(), &mut deferred, true),
            "partial answer\n"
        );
    }

    #[test]
    fn system_theme_uses_colorfgbg_and_has_safe_fallbacks() {
        assert_eq!(system_theme_id(Some("0;15")), "light");
        assert_eq!(system_theme_id(Some("15;0")), "dark");
        assert_eq!(system_theme_id(Some("0;255")), "light");
        assert_eq!(system_theme_id(Some("bad")), "dark");
        assert_eq!(system_theme_id(None), "dark");
    }
}
