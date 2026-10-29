//! `--help` text.
//!
//! Rewritten when the workbench replaced the inline UI: the previous text was a
//! verbatim port of the TypeScript CLI's `printHelp()` — complete with
//! `node dist/index.js` in every example — which no longer described anything
//! that runs. The options themselves are unchanged; the commands and the
//! interactive keys are now the workbench's.

pub fn help_text() -> &'static str {
    "future-tui — the FutureOS terminal UI

Usage: future tui [options] [@files...] [messages...]
       future-tui [options] [@files...] [messages...]

With no mode flag the terminal UI opens (the full-screen workbench). A bare
message starts a session with it as the first prompt.

Options:
  --grpc-addr <addr>    Explicit TCP gRPC address (default: auto — per-user local IPC)
  --session <id>        Connect to a specific session
  --continue, -c        Continue the most recently active session
  --fork <id>           Fork from a session (print mode)
  --print, -p           Non-interactive mode: process prompt and exit
  --model <model>       Model to use (supports model:thinking format, e.g. sonnet:max)
  --models <patterns>   Model patterns for Ctrl+P cycling (comma-separated, supports globs)
  --provider <provider>  Provider to use
  --api-key <key>       API key (overrides env vars)
  --list-models [search] List available models (with optional search)
  --thinking <level>    Thinking level: default (provider default), off, minimal,
                        low, medium, high, xhigh, max
  --system-prompt <text> Set system prompt
  --append-system-prompt <text> Append to system prompt
  --tools, -t <tools>  Comma-separated tool names to enable
  --no-tools, -nt       Disable all tools
  --no-builtin-tools, -nbt Disable built-in tools (keep extensions)
  --no-session          Ephemeral mode (don't save session)
  --mode <mode>        Output mode: text, json (default: text)
  --prompt-template <path> Load a prompt template file
  --no-prompt-templates, -np Disable prompt templates
  --no-context-files, -nc  Disable AGENTS.md / CLAUDE.md / GEMINI.md discovery
  --offline             Disable startup network operations
  --verbose             Show detailed startup information
  --skill <path>        Load a skill file or directory
  --no-skills, -ns      Disable skills discovery
  --version, -v         Show version number
  --help, -h            Show this help

Keys (interactive):
  enter                 Send the prompt
  tab / shift+tab       Step through links and the rows that open a call or a
                        thought (enter opens the one under the cursor)
  ctrl+b                Sidebar: focus / show / hide (full-screen when narrow)
  ctrl+c                Stop the running turn
  ctrl+n                New session
  ctrl+e                Final answer only, or the whole process
  ctrl+k                Refresh sessions and usage
  /                     Command palette
  ?                     Keys and commands
  ctrl+x                Quit

  Mouse: click a tab or a row, drag the divider, wheel to scroll.

Examples:
  # Interactive session
  future tui

  # Continue where you left off
  future tui --continue

  # One question, non-interactive
  future tui -p --model deepseek-flash \"what does this error mean?\"

  # Review a file with the message
  future tui -p @src/main.rs \"find the bug\"

  # List models
  future tui --list-models
"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_mentions_key_flags() {
        let h = help_text();
        assert!(h.contains("--grpc-addr"));
        assert!(h.contains("--list-models [search]"));
        assert!(h.contains("--no-skills, -ns"));
        assert!(h.contains("future tui --list-models"));
    }

    /// The help must describe what runs. Two regressions this pins: the old
    /// text's `node dist/index.js` examples (nothing in this repository runs
    /// Node), and an option this build no longer has.
    #[test]
    fn help_describes_this_build() {
        let h = help_text();
        assert!(!h.contains("node dist/index.js"), "{h}");
        for gone in ["--legacy", "--classic", "--resume"] {
            assert!(!h.contains(gone), "{gone} is not a flag any more");
        }
        // The interactive keys are the workbench's.
        for key in ["ctrl+b", "ctrl+n", "ctrl+e", "ctrl+x", "ctrl+k"] {
            assert!(h.contains(key), "{key} missing from the key table");
        }
        assert!(h.starts_with("future-tui"));
    }
}
