//! Session export: the whole conversation as one self-contained HTML file.
//!
//! The file has to stand on its own — an attachment, a mail, a folder on a
//! machine with no FutureOS install — so everything lives inside it: the
//! stylesheet, the few lines of script behind the fold buttons, and the
//! document itself. Nothing is fetched from a CDN, and the pictures the
//! session already carries are embedded as `data:` URLs.
//!
//! Two things make the transcript readable rather than a dump:
//!
//! - **Markdown is rendered**, because that is how the model wrote it. Raw
//!   HTML inside a message is shown as text instead of being executed, and
//!   `javascript:` links are neutralised — an export is a document, not a
//!   script host.
//! - **Thinking and tool calls start folded.** A long run is mostly tool
//!   chatter; folding it leaves the questions and the answers as the outline,
//!   with the machinery one click (or "Expand all") away.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use pulldown_cmark::{html as cmark_html, Event, Options, Parser, Tag};
use serde_json::Value;

use crate::types::{AgentMessage, ContentBlock};

/// Render one session to a complete HTML document.
///
/// `messages` is the session's own history (`AgentMessage` blocks), not the
/// flattened LLM projection: the blocks are what keep thinking, a tool call
/// and its result in the places the reader expects them.
pub(crate) fn generate_session_html(
    session_id: &str,
    model: &str,
    cwd: &str,
    messages: &[AgentMessage],
) -> String {
    // Tool results by call id, so a call renders its input and output together
    // even though the result arrives in a later message.
    let results = collect_tool_results(messages);
    let calls: HashSet<&str> = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolCall { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let tool_calls = calls.len();
    let visible_messages = messages
        .iter()
        .filter(|message| !message.content.is_empty())
        .count();
    let exported = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    let mut html = String::with_capacity(16 * 1024 + messages.len() * 1024);
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"utf-8\">\n");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    let _ = writeln!(
        html,
        "<title>FutureAgent session {}</title>",
        escape_html(session_id)
    );
    html.push_str("<style>\n");
    push_styles(&mut html);
    html.push_str("</style>\n</head>\n<body id=\"top\">\n");

    // ─── the head: who, where, how much ───────────────────────────────
    html.push_str("<header class=\"page-head\">\n<div class=\"brand\">\n");
    html.push_str("<span class=\"mark\" aria-hidden=\"true\">F</span>\n<div>\n");
    html.push_str("<h1>FutureAgent session</h1>\n");
    let _ = write!(
        html,
        "<p class=\"sub\">{} {}",
        visible_messages,
        plural(visible_messages, "message", "messages")
    );
    if tool_calls > 0 {
        let _ = write!(
            html,
            " · {} {}",
            tool_calls,
            plural(tool_calls, "tool call", "tool calls")
        );
    }
    let _ = writeln!(html, " · exported {exported}</p>");
    html.push_str("</div>\n</div>\n");

    html.push_str("<dl class=\"meta\">\n");
    push_meta(&mut html, "Session", session_id);
    push_meta(&mut html, "Model", model);
    push_meta(&mut html, "Workspace", cwd);
    html.push_str("</dl>\n</header>\n");

    // ─── the toolbar: the folds and the theme ─────────────────────────
    html.push_str("<div class=\"toolbar\" role=\"toolbar\">\n");
    html.push_str("<button type=\"button\" data-action=\"expand\">Expand all</button>\n");
    html.push_str("<button type=\"button\" data-action=\"collapse\">Collapse all</button>\n");
    html.push_str("<span class=\"spacer\"></span>\n");
    html.push_str("<span class=\"toolbar-note\">Thinking and tool calls start folded</span>\n");
    html.push_str("<button type=\"button\" data-action=\"theme\">Theme</button>\n");
    html.push_str("</div>\n");

    // ─── the conversation ────────────────────────────────────────────
    html.push_str("<main class=\"transcript\">\n");
    if visible_messages == 0 {
        html.push_str("<p class=\"empty\">This session has no messages yet.</p>\n");
    } else {
        render_transcript(messages, &results, &calls, &mut html);
    }
    html.push_str("</main>\n");

    let _ = writeln!(
        html,
        "<footer class=\"page-foot\">Exported by FutureAgent on {exported} · <a href=\"#top\">Back to top</a></footer>"
    );
    html.push_str("<script>\n");
    html.push_str(SCRIPT);
    html.push_str("</script>\n</body>\n</html>\n");
    html
}

fn push_meta(html: &mut String, label: &str, value: &str) {
    let _ = writeln!(
        html,
        "<div><dt>{}</dt><dd>{}</dd></div>",
        escape_html(label),
        escape_html(value)
    );
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 {
        one
    } else {
        many
    }
}

/// Every tool result in the session, by the call it answers.
///
/// A result whose call is not in the history (a fork cut through the middle of
/// a turn, a compacted prefix) is simply not in this map: its own message
/// renders it on its own page instead of dropping it.
fn collect_tool_results(messages: &[AgentMessage]) -> HashMap<&str, (&str, bool)> {
    let mut results = HashMap::new();
    for message in messages {
        for block in &message.content {
            if let ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } = block
            {
                results
                    .entry(tool_call_id.as_str())
                    .or_insert((content.as_str(), *is_error));
            }
        }
    }
    results
}

/// One user message is one card; everything the assistant does until the next
/// user message is one card — thinking, prose and every tool call in order.
///
/// The tools are the reason for grouping: one question with fifteen tool
/// calls would otherwise be fifteen cards, and the answer buried between them.
fn render_transcript(
    messages: &[AgentMessage],
    results: &HashMap<&str, (&str, bool)>,
    calls: &HashSet<&str>,
    html: &mut String,
) {
    let mut assistant = String::new();
    for message in messages {
        match message.role.as_str() {
            "user" => {
                flush_assistant(&mut assistant, html);
                render_user_turn(message, html);
            }
            "assistant" | "tool" => {
                render_assistant_blocks(message, results, calls, &mut assistant);
            }
            other => {
                flush_assistant(&mut assistant, html);
                render_other_turn(other, message, html);
            }
        }
    }
    flush_assistant(&mut assistant, html);
}

fn flush_assistant(body: &mut String, html: &mut String) {
    if body.trim().is_empty() {
        body.clear();
        return;
    }
    html.push_str("<section class=\"turn assistant\">\n");
    html.push_str("<div class=\"turn-role\"><span class=\"role-dot\"></span>Assistant</div>\n");
    html.push_str("<div class=\"turn-body\">\n");
    html.push_str(body);
    html.push_str("</div>\n</section>\n");
    body.clear();
}

fn render_user_turn(message: &AgentMessage, html: &mut String) {
    html.push_str("<section class=\"turn user\">\n");
    html.push_str("<div class=\"turn-role\"><span class=\"role-dot\"></span>You</div>\n");
    html.push_str("<div class=\"turn-body\">\n");
    for block in &message.content {
        match block {
            ContentBlock::Text { text } if !text.trim().is_empty() => {
                render_markdown(text, html);
            }
            ContentBlock::Image { image_url } => render_image(image_url.url.as_deref(), html),
            _ => {}
        }
    }
    render_attachment_chips(message, html);
    html.push_str("</div>\n</section>\n");
}

/// A role the surface does not know (an older protocol's `system` entry, an
/// imported transcript): keep it, labelled as itself, instead of dropping it.
fn render_other_turn(role: &str, message: &AgentMessage, html: &mut String) {
    let label: String = role
        .chars()
        .take(32)
        .collect::<String>()
        .to_uppercase()
        .chars()
        .take(32)
        .collect();
    html.push_str("<section class=\"turn other\">\n");
    let _ = writeln!(
        html,
        "<div class=\"turn-role\"><span class=\"role-dot\"></span>{}</div>",
        escape_html(&label)
    );
    html.push_str("<div class=\"turn-body\">\n");
    for block in &message.content {
        if let ContentBlock::Text { text } = block {
            if !text.trim().is_empty() {
                render_markdown(text, html);
            }
        }
    }
    html.push_str("</div>\n</section>\n");
}

fn render_assistant_blocks(
    message: &AgentMessage,
    results: &HashMap<&str, (&str, bool)>,
    calls: &HashSet<&str>,
    html: &mut String,
) {
    let fallback_name = if message.name.is_empty() {
        "Tool"
    } else {
        message.name.as_str()
    };
    for block in &message.content {
        match block {
            ContentBlock::Text { text } if !text.trim().is_empty() => {
                render_markdown(text, html);
            }
            ContentBlock::Reasoning { text, .. } if !text.trim().is_empty() => {
                render_thinking(text, html);
            }
            ContentBlock::ToolCall { id, name, args, .. } => {
                render_tool_call(name, args, results.get(id.as_str()).copied(), html);
            }
            ContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                // A result whose call is on the page is already inside that
                // call's card; rendering it again would print it twice.
                if !calls.contains(tool_call_id.as_str()) {
                    render_standalone_result(fallback_name, content, *is_error, html);
                }
            }
            ContentBlock::Image { image_url } => render_image(image_url.url.as_deref(), html),
            _ => {}
        }
    }
}

/// Thinking, folded. The character count is the only thing visible, so a long
/// deliberation does not push the answer off the first screen.
fn render_thinking(text: &str, html: &mut String) {
    let chars = text.chars().count();
    html.push_str("<details class=\"fold thinking\">\n");
    let _ = writeln!(
        html,
        "<summary><span class=\"chev\"></span><span class=\"thinking-label\">Thinking</span>\
         <span class=\"muted\">{chars} chars</span></summary>"
    );
    html.push_str("<div class=\"fold-body md\">\n");
    push_markdown(text, html);
    html.push_str("</div>\n</details>\n");
}

/// A tool call and its result, folded into one card. The summary names the
/// tool and the argument that says what it acted on (`command`, `path`, …), so
/// the folded list still reads as the shape of the worked turn.
fn render_tool_call(name: &str, args: &Value, result: Option<(&str, bool)>, html: &mut String) {
    let failed = result.is_some_and(|(_, is_error)| is_error);
    let _ = write!(
        html,
        "<details class=\"fold tool{}\">\n<summary><span class=\"chev\"></span>\
         <span class=\"tool-name\">{}</span>",
        if failed { " error" } else { "" },
        escape_html(name)
    );
    if let Some(hint) = tool_hint(args) {
        let _ = write!(
            html,
            "<span class=\"tool-hint\">{}</span>",
            escape_html(&hint)
        );
    }
    if failed {
        html.push_str("<span class=\"status\">error</span>");
    }
    html.push_str("</summary>\n<div class=\"fold-body\">\n");
    let _ = writeln!(
        html,
        "<div class=\"panel-label\">Input</div><pre class=\"block\">{}</pre>",
        escape_html(&pretty_args(args))
    );
    if let Some((content, _)) = result {
        html.push_str("<div class=\"panel-label\">Output</div>\n");
        push_tool_output(content, html);
    }
    html.push_str("</div>\n</details>\n");
}

fn render_standalone_result(name: &str, content: &str, is_error: bool, html: &mut String) {
    let _ = write!(
        html,
        "<details class=\"fold tool{}\">\n<summary><span class=\"chev\"></span>\
         <span class=\"tool-name\">{}</span><span class=\"tool-hint\">result</span>",
        if is_error { " error" } else { "" },
        escape_html(name)
    );
    if is_error {
        html.push_str("<span class=\"status\">error</span>");
    }
    html.push_str(
        "</summary>\n<div class=\"fold-body\">\n<div class=\"panel-label\">Output</div>\n",
    );
    push_tool_output(content, html);
    html.push_str("</div>\n</details>\n");
}

fn push_tool_output(content: &str, html: &mut String) {
    if content.trim().is_empty() {
        html.push_str("<p class=\"muted\">No output recorded.</p>\n");
    } else {
        let _ = writeln!(
            html,
            "<pre class=\"block out\">{}</pre>",
            escape_html(content)
        );
    }
}

/// The files and pictures the prompt carried. Pictures that survived as image
/// blocks are already on the page; the rest stay as chips with their path.
fn render_attachment_chips(message: &AgentMessage, html: &mut String) {
    let Some(attachments) = message
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("attachments"))
        .and_then(Value::as_array)
    else {
        return;
    };
    let has_picture = message
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::Image { .. }));
    let mut chips = String::new();
    for attachment in attachments {
        let kind = attachment
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("file");
        if kind == "image" && has_picture {
            continue;
        }
        let path = attachment
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let name = attachment
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| path.rsplit(['/', '\\']).next().unwrap_or(path));
        let _ = write!(
            chips,
            "<span class=\"chip\" title=\"{}\"><span class=\"chip-kind\">{}</span>{}</span>",
            escape_html(path),
            escape_html(kind),
            escape_html(name)
        );
    }
    if !chips.is_empty() {
        let _ = writeln!(html, "<div class=\"chips\">{chips}</div>");
    }
}

/// A picture the file can carry, or a labelled path when it cannot. Remote
/// URLs become links instead of `<img>`: an export opened offline should not
/// start fetching things by itself.
fn render_image(url: Option<&str>, html: &mut String) {
    let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) else {
        return;
    };
    if url.starts_with("data:image/") {
        let _ = writeln!(
            html,
            "<figure class=\"figure\"><img src=\"{}\" alt=\"Attached image\" loading=\"lazy\"></figure>",
            escape_html(url)
        );
    } else if url.starts_with("http://") || url.starts_with("https://") {
        let _ = writeln!(
            html,
            "<p class=\"chip\"><span class=\"chip-kind\">image</span><a href=\"{0}\" rel=\"noreferrer\">{0}</a></p>",
            escape_html(url)
        );
    } else {
        let _ = writeln!(
            html,
            "<p class=\"chip\"><span class=\"chip-kind\">image</span>{}</p>",
            escape_html(url)
        );
    }
}

// ─── markdown ─────────────────────────────────────────────────────────────

fn render_markdown(source: &str, html: &mut String) {
    html.push_str("<div class=\"md\">\n");
    push_markdown(source, html);
    html.push_str("</div>\n");
}

/// Markdown with the model's own syntax: tables, task lists and strikethrough
/// included. Raw HTML is turned into text (visible, never executed) and unsafe
/// link schemes are dropped to `#`.
fn push_markdown(source: &str, html: &mut String) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(source, options).map(|event| match event {
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) if is_unsafe_link(&dest_url) => Event::Start(Tag::Link {
            link_type,
            dest_url: "#".into(),
            title,
            id,
        }),
        other => other,
    });
    cmark_html::push_html(html, parser);
}

fn is_unsafe_link(url: &str) -> bool {
    let url = url.trim_start().to_ascii_lowercase();
    url.starts_with("javascript:") || url.starts_with("vbscript:") || url.starts_with("data:")
}

// ─── tool arguments ───────────────────────────────────────────────────────

fn argument_object(args: &Value) -> Option<serde_json::Map<String, Value>> {
    match args {
        Value::Object(map) => Some(map.clone()),
        Value::String(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|value| value.as_object().cloned()),
        _ => None,
    }
}

fn pretty_args(args: &Value) -> String {
    match args {
        Value::String(raw) => serde_json::from_str::<Value>(raw.trim())
            .ok()
            .and_then(|value| serde_json::to_string_pretty(&value).ok())
            .unwrap_or_else(|| raw.clone()),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

/// The argument that says what a call acted on, from the keys the tools use.
fn tool_hint(args: &Value) -> Option<String> {
    const KEYS: [&str; 9] = [
        "command",
        "path",
        "file_path",
        "filePath",
        "pattern",
        "query",
        "url",
        "description",
        "name",
    ];
    let object = argument_object(args)?;
    let value = KEYS
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())?;
    Some(one_line(value, 96))
}

fn one_line(text: &str, limit: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(limit).collect();
    if collapsed.chars().count() > limit {
        out.push('…');
    }
    out
}

// ─── escaping ─────────────────────────────────────────────────────────────

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ─── the stylesheet ───────────────────────────────────────────────────────

const LIGHT_VARS: &str = "
  color-scheme: light;
  --font-sans: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, \"Helvetica Neue\", \"PingFang SC\", \"Hiragino Sans GB\", \"Microsoft YaHei\", sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, \"SF Mono\", Menlo, Consolas, \"Liberation Mono\", monospace;
  --bg: #eef1f5;
  --surface: #ffffff;
  --surface-2: #f3f5f8;
  --text: #1b1f24;
  --muted: #5c6672;
  --line: #dde2e8;
  --line-strong: #c6cfd9;
  --accent: #2f6feb;
  --user-bg: #eef4ff;
  --user-line: #c8dbff;
  --assistant-role: #0f766e;
  --assistant-line: #7dd3c7;
  --tool-bg: #f4f7fb;
  --thinking-bg: #fbf7ea;
  --thinking-line: #e8d49a;
  --thinking-ink: #7a5b12;
  --code-bg: #f2f4f7;
  --danger: #b42318;
  --danger-soft: #f3c0bb;
  --shadow: 0 1px 2px rgba(16, 24, 40, .05), 0 10px 30px -14px rgba(16, 24, 40, .18);
";

const DARK_VARS: &str = "
  color-scheme: dark;
  --bg: #0b0e14;
  --surface: #151a22;
  --surface-2: #202834;
  --text: #e7ebf1;
  --muted: #a3aebc;
  --line: #2a3340;
  --line-strong: #3d4a5c;
  --accent: #8fb2ff;
  --user-bg: #111a2b;
  --user-line: #2b4266;
  --assistant-role: #5eead4;
  --assistant-line: #2c5f59;
  --tool-bg: #1a2230;
  --thinking-bg: #211a0d;
  --thinking-line: #4d3f1f;
  --thinking-ink: #f0ce7f;
  --code-bg: #0a0f16;
  --danger: #ff8a80;
  --danger-soft: #5a2320;
  --shadow: none;
";

fn push_styles(html: &mut String) {
    let _ = writeln!(html, ":root{{{LIGHT_VARS}}}");
    let _ = writeln!(html, ":root[data-theme=\"dark\"]{{{DARK_VARS}}}");
    let _ = writeln!(
        html,
        "@media (prefers-color-scheme: dark){{:root:not([data-theme=\"light\"]){{{DARK_VARS}}}}}"
    );
    html.push_str(CSS);
}

const CSS: &str = r##"
*, *::before, *::after { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font: 15px/1.65 var(--font-sans);
  overflow-wrap: break-word;
}
.page-head, .toolbar, .transcript, .page-foot {
  width: min(920px, calc(100% - 40px));
  margin-inline: auto;
}
.page-head { padding-top: 44px; }
.brand { display: flex; align-items: center; gap: 14px; }
.mark {
  flex: none;
  width: 38px;
  height: 38px;
  border-radius: 12px;
  background: linear-gradient(135deg, var(--accent), #8b5cf6);
  color: #fff;
  display: grid;
  place-items: center;
  font: 700 18px/1 var(--font-sans);
}
h1 { margin: 0; font-size: 23px; line-height: 1.2; letter-spacing: -.01em; }
.sub { margin: 5px 0 0; color: var(--muted); font-size: 13px; }
a { color: var(--accent); text-underline-offset: 2px; }
.meta {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
  gap: 12px 28px;
  margin: 22px 0 0;
}
.meta dt {
  font-size: 10px;
  letter-spacing: .1em;
  text-transform: uppercase;
  color: var(--muted);
  margin-bottom: 3px;
}
.meta dd { margin: 0; font: 13px/1.45 var(--font-mono); overflow-wrap: anywhere; }
.toolbar {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 22px;
  margin-bottom: 22px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface);
  box-shadow: var(--shadow);
}
.toolbar button {
  appearance: none;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--surface-2);
  color: var(--text);
  padding: 6px 12px;
  font: 500 12px/1 var(--font-sans);
  cursor: pointer;
}
.toolbar button:hover { background: var(--surface); border-color: var(--line-strong); }
.toolbar button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.toolbar .spacer { flex: 1; }
.toolbar-note { color: var(--muted); font-size: 12px; }
.transcript { padding-bottom: 24px; }
.turn {
  position: relative;
  margin: 0 0 18px;
  padding: 15px 20px 17px 24px;
  border: 1px solid var(--line);
  border-radius: 14px;
  background: var(--surface);
  box-shadow: var(--shadow);
}
.turn::before {
  content: "";
  position: absolute;
  left: 0;
  top: 14px;
  bottom: 14px;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background: var(--line);
}
.turn.user { background: var(--user-bg); border-color: var(--user-line); }
.turn.user::before { background: var(--accent); }
.turn.assistant::before { background: var(--assistant-line); }
.turn.other::before { background: var(--line-strong); }
.turn-role {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 9px;
  font: 650 11px/1 var(--font-sans);
  letter-spacing: .09em;
  text-transform: uppercase;
  color: var(--muted);
}
.role-dot { width: 7px; height: 7px; border-radius: 50%; background: currentColor; }
.turn.user .turn-role { color: var(--accent); }
.turn.assistant .turn-role { color: var(--assistant-role); }
.turn-body > :first-child { margin-top: 0; }
.turn-body > :last-child { margin-bottom: 0; }
.md > :first-child { margin-top: 0; }
.md > :last-child { margin-bottom: 0; }
.md p, .md ul, .md ol, .md pre, .md table, .md blockquote { margin: 0 0 12px; }
.md h1, .md h2, .md h3, .md h4, .md h5, .md h6 {
  margin: 22px 0 10px;
  line-height: 1.28;
  font-weight: 650;
}
.md h1 { font-size: 1.5em; }
.md h2 { font-size: 1.3em; }
.md h3 { font-size: 1.15em; }
.md h4, .md h5, .md h6 { font-size: 1em; }
.md ul, .md ol { padding-left: 24px; }
.md li + li { margin-top: 4px; }
.md li > p { margin: 0; }
.md code {
  font-family: var(--font-mono);
  font-size: .89em;
  background: var(--code-bg);
  border: 1px solid var(--line);
  border-radius: 6px;
  padding: 1px 5px;
}
.md pre {
  background: var(--code-bg);
  border: 1px solid var(--line);
  border-radius: 10px;
  padding: 12px 14px;
  overflow: auto;
}
.md pre code {
  display: block;
  background: none;
  border: 0;
  padding: 0;
  font-size: 13px;
  line-height: 1.55;
  white-space: pre;
}
.md blockquote {
  border-left: 3px solid var(--line-strong);
  padding: 2px 0 2px 14px;
  color: var(--muted);
}
.md table {
  display: block;
  width: 100%;
  overflow: auto;
  border-collapse: collapse;
}
.md th, .md td { border: 1px solid var(--line); padding: 6px 10px; text-align: left; }
.md th { background: var(--surface-2); font-weight: 600; }
.md img { max-width: 100%; height: auto; border-radius: 8px; }
.md hr { border: 0; border-top: 1px solid var(--line); margin: 20px 0; }
.md input[type="checkbox"] { margin-right: 6px; }
.md .footnote-definition { color: var(--muted); font-size: .92em; }
details.fold {
  margin: 12px 0;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--tool-bg);
  overflow: hidden;
}
details.fold > summary {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 8px 12px;
  cursor: pointer;
  list-style: none;
  user-select: none;
  font: 500 13px/1.45 var(--font-mono);
  color: var(--muted);
}
details.fold > summary::-webkit-details-marker { display: none; }
details.fold > summary:hover { background: var(--surface-2); }
details.fold[open] > summary { border-bottom: 1px solid var(--line); }
.chev {
  flex: none;
  width: 0;
  height: 0;
  border-left: 5px solid currentColor;
  border-top: 4px solid transparent;
  border-bottom: 4px solid transparent;
  transition: transform .15s ease;
}
details[open] > summary .chev { transform: rotate(90deg); }
details.thinking { background: var(--thinking-bg); border-color: var(--thinking-line); }
details.thinking > summary { font-family: var(--font-sans); color: var(--thinking-ink); }
details.thinking > summary .muted { color: var(--thinking-ink); opacity: .72; }
.thinking-label { font-weight: 600; }
.tool-name { color: var(--text); font-weight: 600; }
.tool-hint {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-sans);
}
.muted { color: var(--muted); font-size: 12px; margin-left: auto; }
.status {
  flex: none;
  padding: 2px 7px;
  border: 1px solid var(--danger-soft);
  border-radius: 999px;
  color: var(--danger);
  font: 600 10px/1.4 var(--font-sans);
  letter-spacing: .06em;
  text-transform: uppercase;
}
details.fold > .fold-body { padding: 12px; }
details.fold > .fold-body.md { font-size: 14px; color: var(--muted); }
details.fold > .fold-body.md > :last-child { margin-bottom: 0; }
.panel-label {
  margin: 0 0 6px;
  font: 600 10px/1.4 var(--font-sans);
  letter-spacing: .1em;
  text-transform: uppercase;
  color: var(--muted);
}
.panel-label + .block { margin-bottom: 12px; }
.panel-label:not(:first-child) { margin-top: 2px; }
pre.block {
  margin: 0 0 12px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--code-bg);
  overflow: auto;
  max-height: 60vh;
  font: 12.5px/1.55 var(--font-mono);
  white-space: pre;
}
pre.block:last-child { margin-bottom: 0; }
.chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 10px; }
.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  padding: 3px 9px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--surface-2);
  color: var(--text);
  font: 12px/1.5 var(--font-sans);
  overflow-wrap: anywhere;
}
.chip-kind {
  color: var(--muted);
  font-size: 10px;
  letter-spacing: .08em;
  text-transform: uppercase;
}
.figure { margin: 12px 0; }
.figure img {
  max-width: 100%;
  height: auto;
  border: 1px solid var(--line);
  border-radius: 10px;
}
.empty {
  padding: 48px 20px;
  border: 1px dashed var(--line-strong);
  border-radius: 14px;
  color: var(--muted);
  text-align: center;
}
.page-foot {
  padding: 24px 0 56px;
  border-top: 1px solid var(--line);
  color: var(--muted);
  font-size: 12px;
}
@media print {
  body { background: #fff; color: #000; }
  .toolbar { display: none; }
  .turn { box-shadow: none; break-inside: avoid; }
}
"##;

/// The whole script: fold buttons, the theme toggle, and printing unfolded.
const SCRIPT: &str = r##"
(function () {
  var root = document.documentElement;
  var themeKey = 'futureos-export-theme';
  try {
    var saved = localStorage.getItem(themeKey);
    if (saved === 'light' || saved === 'dark') root.setAttribute('data-theme', saved);
  } catch (error) { /* some file:// origins refuse storage */ }

  function folds() { return document.querySelectorAll('details.fold'); }
  function setAll(open) {
    var list = folds();
    for (var i = 0; i < list.length; i++) list[i].open = open;
  }
  function dark() {
    var chosen = root.getAttribute('data-theme');
    if (chosen) return chosen === 'dark';
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
  }

  var buttons = document.querySelectorAll('[data-action]');
  for (var i = 0; i < buttons.length; i++) {
    buttons[i].addEventListener('click', function () {
      var action = this.getAttribute('data-action');
      if (action === 'expand') { setAll(true); return; }
      if (action === 'collapse') { setAll(false); return; }
      if (action === 'theme') {
        var next = dark() ? 'light' : 'dark';
        root.setAttribute('data-theme', next);
        try { localStorage.setItem(themeKey, next); } catch (error) {}
      }
    });
  }

  window.addEventListener('beforeprint', function () { setAll(true); });
})();
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentMessage, ContentBlock, ImageUrlData};

    fn user(text: &str) -> AgentMessage {
        AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text(text)],
            ..Default::default()
        }
    }

    fn assistant(text: &str) -> AgentMessage {
        AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::text(text)],
            ..Default::default()
        }
    }

    fn tool_result(call_id: &str, content: &str, is_error: bool) -> AgentMessage {
        AgentMessage {
            role: "tool".to_string(),
            name: "shell".to_string(),
            content: vec![ContentBlock::ToolResult {
                tool_call_id: call_id.to_string(),
                content: content.to_string(),
                is_error,
            }],
            ..Default::default()
        }
    }

    fn tool_call(id: &str, name: &str, args: Value) -> ContentBlock {
        ContentBlock::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            args,
            provider_metadata: Default::default(),
        }
    }

    fn render(messages: &[AgentMessage]) -> String {
        generate_session_html("sess-123", "gpt-4o", "/tmp/test", messages)
    }

    #[test]
    fn the_title_and_the_metadata_name_the_session() {
        let html = render(&[]);
        assert!(html.contains("<title>FutureAgent session sess-123</title>"));
        assert!(html.contains("gpt-4o"));
        assert!(html.contains("/tmp/test"));
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<body id=\"top\">"));
        assert!(html.contains("</body>"));
    }

    #[test]
    fn an_empty_session_says_so_instead_of_painting_an_empty_transcript() {
        let html = render(&[]);
        assert!(html.contains("This session has no messages yet."));
        assert!(!html.contains("class=\"turn"));
    }

    #[test]
    fn the_document_is_one_file_with_no_external_assets() {
        let html = render(&[user("hi"), assistant("hello")]);
        assert!(html.contains("<style>"));
        assert!(html.contains("<script>"));
        assert!(!html.contains("<link"));
        assert!(!html.contains(" src=\"http"));
    }

    #[test]
    fn markdown_becomes_html() {
        let html = render(&[assistant(
            "**bold** and `code`\n\n- one\n- two\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n```rust\nfn main() {}\n```",
        )]);
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        assert!(html.contains("<code>code</code>"), "{html}");
        assert!(html.contains("<li>one</li>"), "{html}");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("language-rust"), "{html}");
        assert!(html.contains("fn main() {}"), "{html}");
    }

    #[test]
    fn raw_html_in_a_message_is_shown_as_text_not_executed() {
        let html = render(&[user("<script>alert('xss')</script>")]);
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn unsafe_link_schemes_are_neutralised() {
        let html = render(&[assistant("[x](javascript:alert(1))")]);
        assert!(!html.contains("href=\"javascript:"), "{html}");
    }

    #[test]
    fn thinking_is_folded_and_labelled() {
        let message = AgentMessage {
            role: "assistant".to_string(),
            content: vec![
                ContentBlock::reasoning("weighing the options", Default::default()),
                ContentBlock::text("the answer"),
            ],
            ..Default::default()
        };
        let html = render(&[message]);
        assert!(html.contains("<details class=\"fold thinking\">"), "{html}");
        assert!(html.contains("Thinking"), "{html}");
        assert!(html.contains("weighing the options"), "{html}");
        // Folded by default: no `open` attribute.
        assert!(!html.contains("class=\"fold thinking\" open"), "{html}");
    }

    #[test]
    fn a_tool_call_and_its_result_are_one_folded_card() {
        let messages = vec![
            user("run the tests"),
            AgentMessage {
                role: "assistant".to_string(),
                content: vec![
                    ContentBlock::text("running them"),
                    tool_call(
                        "call-1",
                        "shell",
                        serde_json::json!({"command": "cargo test"}),
                    ),
                ],
                ..Default::default()
            },
            tool_result("call-1", "test result: ok. 3 passed", false),
        ];
        let html = render(&messages);
        assert!(html.contains("<details class=\"fold tool\">"), "{html}");
        assert!(html.contains("cargo test"), "{html}");
        assert!(html.contains("test result: ok. 3 passed"), "{html}");
        // The result appears once, inside the call's card.
        assert_eq!(html.matches("test result: ok").count(), 1, "{html}");
        assert!(
            html.contains("<div class=\"panel-label\">Input</div>"),
            "{html}"
        );
        assert!(
            html.contains("<div class=\"panel-label\">Output</div>"),
            "{html}"
        );
    }

    #[test]
    fn a_failed_tool_call_is_marked() {
        let messages = vec![
            AgentMessage {
                role: "assistant".to_string(),
                content: vec![tool_call(
                    "c1",
                    "shell",
                    serde_json::json!({"command": "false"}),
                )],
                ..Default::default()
            },
            tool_result("c1", "exit 1", true),
        ];
        let html = render(&messages);
        assert!(html.contains("fold tool error"), "{html}");
        assert!(
            html.contains("<span class=\"status\">error</span>"),
            "{html}"
        );
    }

    #[test]
    fn a_result_without_its_call_still_renders() {
        let html = render(&[tool_result("orphan", "kept output", false)]);
        assert!(html.contains("kept output"), "{html}");
        assert!(html.contains("<details class=\"fold tool\">"), "{html}");
    }

    #[test]
    fn a_run_of_assistant_messages_is_one_card() {
        let messages = vec![
            user("question"),
            assistant("first"),
            AgentMessage {
                role: "assistant".to_string(),
                content: vec![tool_call("c1", "read", serde_json::json!({"path": "a.rs"}))],
                ..Default::default()
            },
            tool_result("c1", "file body", false),
            assistant("final"),
        ];
        let html = render(&messages);
        assert_eq!(
            html.matches("<section class=\"turn assistant\">").count(),
            1,
            "{html}"
        );
        assert_eq!(
            html.matches("<section class=\"turn user\">").count(),
            1,
            "{html}"
        );
        // The argument names what the folded row acted on.
        assert!(html.contains("tool-hint"), "{html}");
        assert!(html.contains("a.rs"), "{html}");
    }

    #[test]
    fn an_unknown_role_gets_its_own_labelled_card() {
        let messages = vec![
            AgentMessage {
                role: "system".to_string(),
                content: vec![ContentBlock::text("setup notes")],
                ..Default::default()
            },
            user("hi"),
        ];
        let html = render(&messages);
        assert!(html.contains(">SYSTEM</div>"), "{html}");
        assert!(html.contains("setup notes"), "{html}");
        // It is its own card, not a second role row inside an assistant one.
        assert!(!html.contains("turn assistant"), "{html}");
    }

    #[test]
    fn anything_textual_is_escaped() {
        let html = generate_session_html(
            "</title><script>session</script>",
            "<img src=x onerror=alert(1)>",
            "<script>cwd</script>",
            &[AgentMessage {
                role: "assistant".to_string(),
                content: vec![
                    ContentBlock::text("<b>hello</b>"),
                    ContentBlock::ToolCall {
                        id: "\"><script>id</script>".to_string(),
                        name: "<script>tool</script>".to_string(),
                        args: serde_json::json!({"command": "<script>arg</script>"}),
                        provider_metadata: Default::default(),
                    },
                ],
                ..Default::default()
            }],
        );
        assert!(!html.contains("<script>session"));
        assert!(!html.contains("<img src=x"));
        assert!(!html.contains("<script>cwd"));
        assert!(!html.contains("<script>tool"));
        assert!(!html.contains("<script>arg"));
        assert!(!html.contains("<b>hello</b>"));
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn tool_arguments_are_pretty_printed() {
        let message = AgentMessage {
            role: "assistant".to_string(),
            content: vec![tool_call(
                "c1",
                "shell",
                serde_json::json!({"command": "ls", "timeout": 30}),
            )],
            ..Default::default()
        };
        let html = render(&[message]);
        assert!(
            html.contains("&quot;command&quot;: &quot;ls&quot;"),
            "{html}"
        );
    }

    #[test]
    fn an_attached_picture_is_embedded() {
        let message = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::image("data:image/png;base64,AAAA")],
            ..Default::default()
        };
        let html = render(&[message]);
        assert!(
            html.contains("<img src=\"data:image/png;base64,AAAA\""),
            "{html}"
        );
    }

    #[test]
    fn a_file_attachment_becomes_a_chip() {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "attachments".to_string(),
            serde_json::json!([{"kind": "file", "path": "/tmp/report.pdf", "name": "report.pdf"}]),
        );
        let message = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::text("see attached")],
            metadata: Some(metadata),
            ..Default::default()
        };
        let html = render(&[message]);
        assert!(html.contains("report.pdf"), "{html}");
        assert!(html.contains("chip-kind"), "{html}");
    }

    #[test]
    fn a_picture_without_a_source_leaves_the_page_alone() {
        // A provider may echo an image block with no URL, or with one that is
        // only whitespace; neither is a figure.
        let missing = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Image {
                image_url: ImageUrlData { url: None },
            }],
            ..Default::default()
        };
        let blank = AgentMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::image("   ")],
            ..Default::default()
        };
        let html = render(&[missing, blank]);
        assert!(!html.contains("<img"), "{html}");
        assert!(!html.contains("<figure"), "{html}");
        assert_eq!(html.matches("turn user").count(), 2, "{html}");
    }

    #[test]
    fn a_remote_picture_becomes_a_link_and_a_local_one_a_chip() {
        let messages = vec![
            AgentMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::image("https://example.com/a.png")],
                ..Default::default()
            },
            AgentMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::image("http://example.com/b.png")],
                ..Default::default()
            },
            AgentMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::image("/var/fixtures/pic.png")],
                ..Default::default()
            },
        ];
        let html = render(&messages);
        assert!(
            html.contains(
                "<a href=\"https://example.com/a.png\" rel=\"noreferrer\">https://example.com/a.png</a>"
            ),
            "{html}"
        );
        assert!(html.contains("href=\"http://example.com/b.png\""), "{html}");
        // An export opened offline must not fetch anything by itself.
        assert!(!html.contains("<img src=\"http"), "{html}");
        assert!(
            html.contains("<span class=\"chip-kind\">image</span>/var/fixtures/pic.png"),
            "{html}"
        );
    }

    #[test]
    fn a_picture_inside_an_answer_is_embedded_in_its_card() {
        let message = AgentMessage {
            role: "assistant".to_string(),
            content: vec![
                ContentBlock::text("here it is:"),
                ContentBlock::image("data:image/png;base64,BBBB"),
            ],
            ..Default::default()
        };
        let html = render(&[user("show me"), message]);
        assert!(
            html.contains("<img src=\"data:image/png;base64,BBBB\""),
            "{html}"
        );
        assert_eq!(html.matches("turn assistant").count(), 1, "{html}");
    }

    #[test]
    fn a_blank_block_renders_nothing_at_all() {
        // Guards on both the user and the assistant path drop a whitespace-only
        // text block instead of painting an empty paragraph.
        let messages = vec![
            AgentMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::text("  \n ")],
                ..Default::default()
            },
            AgentMessage {
                role: "assistant".to_string(),
                content: vec![
                    ContentBlock::text("   "),
                    ContentBlock::reasoning("   ", Default::default()),
                ],
                ..Default::default()
            },
        ];
        let html = render(&messages);
        // No markdown block and no folded thinking panel were opened.
        assert!(!html.contains("<div class=\"md\">"), "{html}");
        assert!(!html.contains("fold thinking"), "{html}");
        assert!(html.contains("turn user"), "{html}");
    }

    #[test]
    fn a_user_turn_keeps_only_prose_and_pictures() {
        // A prompt that also carries a tool result or a reasoning block (an
        // imported transcript) renders as a question, not as tool chrome.
        let message = AgentMessage {
            role: "user".to_string(),
            content: vec![
                ContentBlock::reasoning("internal note", Default::default()),
                ContentBlock::text("what is in a.rs?"),
                ContentBlock::tool_result("c9", "leaked tool body", false),
            ],
            ..Default::default()
        };
        let html = render(&[message]);
        assert!(html.contains("what is in a.rs?"), "{html}");
        assert!(!html.contains("internal note"), "{html}");
        assert!(!html.contains("leaked tool body"), "{html}");
        assert!(!html.contains("fold tool"), "{html}");
    }

    #[test]
    fn tool_arguments_given_as_a_json_string_still_name_their_target() {
        // Some providers echo the raw argument text instead of an object.
        let message = AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::ToolCall {
                id: "c1".to_string(),
                name: "shell".to_string(),
                args: Value::String(r#"{"command":"cargo test"}"#.to_string()),
                provider_metadata: Default::default(),
            }],
            ..Default::default()
        };
        let html = render(&[message]);
        assert!(
            html.contains("<span class=\"tool-hint\">cargo test</span>"),
            "{html}"
        );
        // The input panel shows the parsed object, not the raw JSON text.
        assert!(
            html.contains("&quot;command&quot;: &quot;cargo test&quot;"),
            "{html}"
        );
        assert!(!html.contains(r#"{\&quot;command\&quot;"#), "{html}");
    }

    #[test]
    fn string_arguments_that_are_not_json_are_shown_verbatim() {
        let message = AgentMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::ToolCall {
                id: "c1".to_string(),
                name: "shell".to_string(),
                args: Value::String("half a command".to_string()),
                provider_metadata: Default::default(),
            }],
            ..Default::default()
        };
        let html = render(&[message]);
        assert!(html.contains("half a command"), "{html}");
        // No key to name the target with, so the summary carries no hint (the
        // stylesheet's `.tool-hint` rule is not a rendered hint).
        assert!(!html.contains("<span class=\"tool-hint\">"), "{html}");
    }

    #[test]
    fn non_object_arguments_have_no_hint_but_still_render() {
        for args in [serde_json::json!([1, 2, 3]), serde_json::json!(7)] {
            let message = AgentMessage {
                role: "assistant".to_string(),
                content: vec![tool_call("c1", "shell", args.clone())],
                ..Default::default()
            };
            let html = render(&[message]);
            assert!(
                !html.contains("<span class=\"tool-hint\">"),
                "{args} {html}"
            );
            let expected = serde_json::to_string_pretty(&args).expect("pretty prints");
            assert!(html.contains(&escape_html(&expected)), "{args} {html}");
        }
    }

    #[test]
    fn a_long_argument_is_folded_to_one_truncated_line() {
        let command = format!("echo {}\nsecond line", "y".repeat(120));
        let message = AgentMessage {
            role: "assistant".to_string(),
            content: vec![tool_call(
                "c1",
                "shell",
                serde_json::json!({"command": command}),
            )],
            ..Default::default()
        };
        let html = render(&[message]);
        let hint = html
            .split("<span class=\"tool-hint\">")
            .nth(1)
            .and_then(|rest| rest.split("</span>").next())
            .expect("the hint is rendered");
        assert!(hint.ends_with('…'), "{hint}");
        assert_eq!(hint.chars().count(), 97, "{hint}");
        assert!(!hint.contains('\n'), "{hint}");
        // The input panel still carries the whole argument.
        assert!(html.contains(&"y".repeat(120)), "{html}");
    }

    #[test]
    fn an_orphan_error_result_is_marked_as_failed() {
        let html = render(&[tool_result("gone", "exit 2", true)]);
        assert!(html.contains("fold tool error"), "{html}");
        assert!(
            html.contains("<span class=\"status\">error</span>"),
            "{html}"
        );
        assert!(html.contains("exit 2"), "{html}");
    }

    #[test]
    fn a_result_with_no_output_says_so() {
        let html = render(&[tool_result("gone", "  \n ", false)]);
        assert!(
            html.contains("<p class=\"muted\">No output recorded.</p>"),
            "{html}"
        );
        assert!(!html.contains("block out"), "{html}");
    }

    #[test]
    fn a_picture_attachment_is_not_repeated_as_a_chip() {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "attachments".to_string(),
            serde_json::json!([
                {"kind": "image", "path": "/tmp/diagram.png", "name": "diagram.png"},
                {"kind": "file", "path": "/tmp/report.pdf"}
            ]),
        );
        let message = AgentMessage {
            role: "user".to_string(),
            content: vec![
                ContentBlock::text("see both"),
                ContentBlock::image("data:image/png;base64,CCCC"),
            ],
            metadata: Some(metadata),
            ..Default::default()
        };
        let html = render(&[message]);
        // The picture is already a figure on the page; the chip list keeps only
        // what has no other rendering, and falls back to the file name.
        assert!(!html.contains("diagram.png"), "{html}");
        assert!(html.contains(">file</span>report.pdf</span>"), "{html}");
    }

    #[test]
    fn the_counts_in_the_head_describe_the_session() {
        let messages = vec![
            user("one"),
            AgentMessage {
                role: "assistant".to_string(),
                content: vec![tool_call("c1", "shell", serde_json::json!({}))],
                ..Default::default()
            },
        ];
        let html = render(&messages);
        assert!(html.contains("2 messages"), "{html}");
        assert!(html.contains("1 tool call"), "{html}");
    }
}
