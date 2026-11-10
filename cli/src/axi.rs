//! Agent-ergonomic output: the small vocabulary an agent-facing command shares.
//!
//! These helpers implement the parts of AXI (Agent eXperience Interface,
//! <https://axi.md/> — ten design principles for CLI tools an agent drives) that
//! the `future` CLI can adopt without breaking the text output humans and
//! scripts already read:
//!
//! | principle | helper |
//! |---|---|
//! | 1 token-efficient output | [`toon_rows`] — TOON instead of JSON |
//! | 2 minimal default schemas | the row fields each command passes |
//! | 4 pre-computed aggregates | the `count:` line [`count_line`] prints |
//! | 5 definitive empty states | [`empty_state`] |
//! | 6 structured errors, exit codes | [`EXIT_USAGE`], [`usage`] |
//! | 8 content first | [`bin_path`], [`tilde`] for the home view |
//! | 9 contextual disclosure | [`hints`] — the `help[N]:` block |
//!
//! The rule that keeps this safe: hints and counts are part of the *text*
//! view only. `--json` and `--format toon` print data and nothing else, because
//! the moment a machine-readable stream grows a sentence, every consumer has to
//! parse around it.

use crate::flags::Flags;
use crate::output::Output;
use std::path::Path;

/// Exit code for a usage error: an unknown flag, an unknown value, a missing
/// argument. Distinct from 1 (the command ran and failed) so a caller can tell
/// "I asked wrong" from "it did not work" — AXI principle 6, and the same
/// contract an unknown *command* already had.
pub const EXIT_USAGE: i32 = 2;

/// How a command renders its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The human table, with a `count:` line and a `help[N]:` block.
    Text,
    /// The unchanged pretty JSON document.
    Json,
    /// TOON: the same rows without braces, quotes and repeated keys.
    Toon,
}

impl Format {
    /// `--format <text|json|toon>`, with `--json` kept as its own long-standing
    /// alias. Giving both a `--json` and a conflicting `--format` is an error
    /// rather than a silent precedence rule.
    pub fn resolve(flags: &Flags, default: Format) -> Result<Self, String> {
        let explicit = match flags.value("--format") {
            None => None,
            Some(raw) => Some(match raw {
                "text" => Format::Text,
                "json" => Format::Json,
                "toon" => Format::Toon,
                other => return Err(format!("--format must be text, json or toon: {other:?}")),
            }),
        };
        match (explicit, flags.has("--json")) {
            (Some(chosen), true) if chosen != Format::Json => {
                Err("--json conflicts with --format; pass only one of them".to_string())
            }
            (Some(chosen), _) => Ok(chosen),
            (None, true) => Ok(Format::Json),
            (None, false) => Ok(default),
        }
    }

    /// True when the output is data rather than a view.
    pub fn is_data(self) -> bool {
        !matches!(self, Format::Text)
    }
}

/// Does this error text describe a usage mistake rather than a failure?
///
/// Groups that have not been converted to [`usage`] still raise a plain
/// `Err("Usage: …")` string, and the dispatcher's `catch` used to turn every one
/// of those into exit 1 — while `docs/wiki/{en,zh}/CLI.md` promises 2. Classifying
/// here fixes the whole surface at once; converting a group to [`usage`] later
/// keeps the same answer, so the two paths agree instead of drifting.
pub fn is_usage_error(message: &str) -> bool {
    let first = message.lines().next().unwrap_or("").trim_start();
    first.starts_with("Usage: ")
        || first.starts_with("usage: ")
        || first.starts_with("Unknown argument")
        || first.starts_with("unexpected argument")
        || first.starts_with("missing ")
        || first.contains("unknown option")
        || first.starts_with("expected at most")
        || first.starts_with("unknown mcp command")
        || first.starts_with("unknown setting")
        || first.starts_with("Invalid data command")
        // A destructive command missing its confirmation is a usage mistake too.
        || first.contains("re-run with --yes")
}

/// True when the command line asks for help.
///
/// Every group checks this **before** it touches anything: `future skills update
/// --help` used to run the update, `future auth logout --help` used to log the
/// user out, and `future channel --help` used to start the bridge. A help
/// request is a read, and must never reach a command body.
pub fn wants_help(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--help" || arg == "-h")
}

/// [`usage`] for code that is not inside a command future (the dispatcher, where
/// the exit code is returned rather than carried by an error).
pub fn usage_exit(out: &Output, detail: &str, usage: &str) -> i32 {
    out.log_err(detail);
    out.log_err(usage);
    out.set_exit_code(EXIT_USAGE);
    out.exit_code()
}

/// Report a usage error: the detail on stderr, the usage line and where the full
/// reference is, then the exit code that says "you asked wrong".
///
/// Returns the [`crate::HANDLED_EXIT`] sentinel so a command can write
/// `return Err(axi::usage(out, &e, USAGE))` and keep the error text out of the
/// generic `console.error` path.
pub fn usage(out: &Output, detail: &str, usage: &str) -> String {
    out.log_err(detail);
    out.log_err(usage);
    out.set_exit_code(EXIT_USAGE);
    crate::HANDLED_EXIT.to_string()
}

/// The `help[N]:` block after a command's own output — concrete next commands,
/// with runtime values left as `<placeholders>`. Agent-readable: the count is
/// there so a reader knows how many suggestions follow.
pub fn hints(out: &Output, lines: &[&str]) {
    if lines.is_empty() {
        return;
    }
    out.log(&format!("\nhelp[{}]:", lines.len()));
    for line in lines {
        out.log(&format!("  {line}"));
    }
}

/// `count: N` — the aggregate a caller would otherwise compute by counting
/// lines. Printed above a text table, never in a data format.
pub fn count_line(out: &Output, label: &str, count: usize) {
    out.log(&format!("count: {count}{label}"));
}

/// An empty result, stated rather than left blank: "no output" and "the command
/// failed silently" must not look the same to a reader that cannot ask.
pub fn empty_state(out: &Output, message: &str) {
    out.log("count: 0");
    out.log(message);
}

/// TOON: `name[N]{a,b,c}:` then one comma-separated row per item.
///
/// Roughly 40% fewer tokens than the equivalent JSON (no braces, no quotes, and
/// the field names appear once instead of once per row). A value is quoted only
/// when it has to be — a comma, a quote, a newline, or surrounding space —
/// which is also what keeps numbers and enums bare.
pub fn toon_rows(name: &str, fields: &[&str], rows: &[Vec<String>]) -> String {
    let mut text = format!("{name}[{}]{{{}}}:\n", rows.len(), fields.join(","));
    for row in rows {
        text.push(' ');
        let cells: Vec<String> = row.iter().map(|cell| toon_value(cell)).collect();
        text.push_str(&cells.join(","));
        text.push('\n');
    }
    text.trim_end().to_string()
}

/// `key: value` for a scalar that accompanies a TOON block.
pub fn toon_scalar(key: &str, value: &str) -> String {
    format!("{key}: {}", toon_value(value))
}

fn toon_value(raw: &str) -> String {
    let needs_quotes = raw.is_empty()
        || raw.starts_with(' ')
        || raw.ends_with(' ')
        || raw.chars().any(|c| matches!(c, ',' | '"' | '\n' | '\r'));
    if !needs_quotes {
        return raw.to_string();
    }
    let escaped = raw.replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The executable an agent is talking to, with the home directory rendered as
/// `~` — AXI principle 8: a home view names the binary that produced it, so a
/// stale install is visible instead of mysterious.
pub fn bin_path() -> String {
    match std::env::current_exe() {
        Ok(path) => tilde(&path),
        Err(_) => "future".to_string(),
    }
}

/// Render `path` with the user's home directory as `~`.
pub fn tilde(path: &Path) -> String {
    let rendered = path.display().to_string();
    if let Some(home) = user_home() {
        let prefix = home.display().to_string();
        if rendered == prefix {
            return "~".to_string();
        }
        if let Some(rest) = rendered.strip_prefix(&(prefix + std::path::MAIN_SEPARATOR_STR)) {
            return format!("~{}{rest}", std::path::MAIN_SEPARATOR);
        }
    }
    rendered
}

/// The user's home, resolved the way the FutureOS home does (`HOME`, then
/// `USERPROFILE`, then the platform profile): a client that shortened against
/// `dirs::home_dir()` alone would print a different `~` than the paths it
/// derived from the same environment.
fn user_home() -> Option<std::path::PathBuf> {
    [std::env::var_os("HOME"), std::env::var_os("USERPROFILE")]
        .into_iter()
        .flatten()
        .map(std::path::PathBuf::from)
        .find(|path| !path.as_os_str().is_empty() && path.is_absolute())
        .or_else(dirs::home_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(values: &[&str]) -> Flags {
        let args: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        crate::flags::parse_flags(&args, &["--format"], &[], &["--json"]).unwrap()
    }

    #[test]
    fn format_defaults_and_aliases() {
        assert_eq!(
            Format::resolve(&flags(&[]), Format::Text).unwrap(),
            Format::Text
        );
        assert_eq!(
            Format::resolve(&flags(&["--json"]), Format::Text).unwrap(),
            Format::Json
        );
        assert_eq!(
            Format::resolve(&flags(&["--format", "toon"]), Format::Text).unwrap(),
            Format::Toon
        );
        // `--format json` and `--json` agree, so both together is fine.
        assert_eq!(
            Format::resolve(&flags(&["--format", "json", "--json"]), Format::Text).unwrap(),
            Format::Json
        );
    }

    #[test]
    fn format_rejects_an_unknown_value_and_a_conflict() {
        let error = Format::resolve(&flags(&["--format", "yaml"]), Format::Text).unwrap_err();
        assert!(error.contains("text, json or toon"), "{error}");
        let error =
            Format::resolve(&flags(&["--format", "toon", "--json"]), Format::Text).unwrap_err();
        assert!(error.contains("conflicts"), "{error}");
    }

    #[test]
    fn toon_quotes_only_what_needs_quoting() {
        let rows = vec![
            vec![
                "42".to_string(),
                "Fix login bug".to_string(),
                "open".to_string(),
            ],
            vec!["43".to_string(), "Add a,b".to_string(), "open".to_string()],
            vec!["44".to_string(), "say \"hi\"".to_string(), "".to_string()],
        ];
        assert_eq!(
            toon_rows("issues", &["number", "title", "state"], &rows),
            "issues[3]{number,title,state}:\n 42,Fix login bug,open\n 43,\"Add a,b\",open\n 44,\"say \\\"hi\\\"\",\"\""
        );
    }

    #[test]
    fn toon_empty_block_is_definitive() {
        assert_eq!(toon_rows("sessions", &["id"], &[]), "sessions[0]{id}:");
    }

    #[test]
    fn hints_render_a_counted_block() {
        let (out, captured) = Output::memory();
        hints(
            &out,
            &["Run `future task show <id>`", "Run `future task runs <id>`"],
        );
        let text = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
        assert_eq!(
            text,
            "\nhelp[2]:\n  Run `future task show <id>`\n  Run `future task runs <id>`\n"
        );
    }

    #[test]
    fn usage_sets_the_usage_exit_code() {
        let (out, captured) = Output::memory();
        let sentinel = usage(&out, "unknown option: --jsno", "Usage: future x [--json]");
        assert_eq!(sentinel, crate::HANDLED_EXIT);
        assert_eq!(out.exit_code(), EXIT_USAGE);
        let err = String::from_utf8(captured.err.lock().unwrap().clone()).unwrap();
        assert!(err.contains("unknown option: --jsno"), "{err}");
        assert!(err.contains("Usage: future x"), "{err}");
    }

    #[test]
    fn tilde_shortens_the_home_prefix_only() {
        let home = user_home().expect("home");
        let inside = home.join("x").join("future");
        assert!(tilde(&inside).starts_with("~/"), "{}", tilde(&inside));
        assert_eq!(tilde(&home), "~");
        // A sibling directory that merely shares a prefix is left alone.
        let sibling = std::path::PathBuf::from(format!("{}2/x", home.display()));
        assert_eq!(tilde(&sibling), sibling.display().to_string());
    }
}
