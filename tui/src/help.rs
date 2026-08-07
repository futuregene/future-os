//! Help text — verbatim port of `printHelp()` in `tui/src/index.ts`.
//!
//! `console.log` of the template literal below emits exactly this string plus
//! one trailing newline. `help_text()` returns the string; the caller prints it
//! with a newline so the bytes match the TS output.

pub fn help_text() -> &'static str {
    "future-tui TUI

Usage: node dist/index.js [options] [@files...] [messages...]

Options:
  --grpc-addr <addr>    gRPC server address (default: localhost:50051)
  --session <id>        Connect to specific session
  --continue, -c        Continue most recent session
  --resume, -r          Resume a session (show picker)
  --fork <id>           Fork from a session
  --print, -p           Non-interactive mode: process prompt and exit
  --model <model>       Model to use (supports model:thinking format)
  --models <patterns>   Model patterns for Ctrl+P cycling (comma-separated, supports globs)
  --provider <provider>  Provider to use
  --api-key <key>       API key (overrides env vars)
  --list-models [search] List available models (with optional search)
  --thinking <level>    Thinking level: off, minimal, low, medium, high, xhigh
  --system-prompt <text> Set system prompt
  --append-system-prompt <text> Append to system prompt
  --tools, -t <tools>  Comma-separated tool names to enable
  --no-tools, -nt       Disable all tools
  --no-builtin-tools, -nbt Disable built-in tools (keep extensions)
  --no-session          Ephemeral mode (don't save session)
  --mode <mode>        Output mode: text, json (default: text)
  --prompt-template <path> Load a prompt template file
  --no-prompt-templates, -np Disable prompt templates
  --no-context-files, -nc  Disable AGENTS.md and CLAUDE.md discovery
  --offline             Disable startup network operations
  --verbose             Show detailed startup information
  --skill <path>        Load a skill file or directory
  --no-skills, -ns      Disable skills discovery
  --version, -v         Show version number
  --help, -h            Show this help

Examples:
  # Interactive mode
  node dist/index.js

  # With specific model
  node dist/index.js --model deepseek-v4-flash

  # Model with thinking level (model:thinking format)
  node dist/index.js --model sonnet:high

  # List models
  node dist/index.js --list-models

  # List models with search
  node dist/index.js --list-models deepseek

  # Non-interactive with thinking level
  node dist/index.js -p --thinking high \"Solve this problem\"

  # Enable only read and bash tools
  node dist/index.js --tools read,shell -p \"Review this code\"

  # JSON output mode
  node dist/index.js --mode json -p \"What is 2+2?\"
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
        assert!(h.contains("node dist/index.js --mode json"));
    }

    #[test]
    fn help_matches_ts_length_contract() {
        // The TS help is a single console.log of a template literal; this
        // guards against accidental drift (e.g. a dropped line). Exact
        // byte-identity with the TS output is verified by the diff harness
        // (P4), which prints help_text() + "\n" and compares.
        let h = help_text();
        assert!(h.starts_with("future-tui TUI\n"));
        assert!(h.ends_with("What is 2+2?\"\n"));
        assert_eq!(h.lines().count(), 58);
    }
}
