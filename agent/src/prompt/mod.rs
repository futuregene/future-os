//! Prompt building — 1:1 compatible with internal/prompt/

use crate::skills::Skill;
use crate::types::AgentTool;

// ─── Public API ─────────────────────────────────────────────────────────────

/// BuildPrompt produces a fully assembled system prompt from the given options.
/// Section ordering matches 's BuildPrompt():
///   1. Identity (who you are + tool list + behavior rules)
///   2. Skills (available capabilities — only if read tool is present)
///   3. Project context (AGENTS.md / CLAUDE.md / GEMINI.md)
///   4. Workspace memory (FUTURE.md)
///   5. Append prompt (user override — placed late so it can override earlier rules)
///   6. Environment (date, cwd, platform)
pub fn build_prompt(opts: &PromptOptions) -> String {
    let mut sections = vec![];

    // 1. Identity
    if !opts.custom_prompt.is_empty() {
        sections.push(opts.custom_prompt.clone());
    } else {
        sections.push(build_identity_section(opts));
    }

    // 2. Skills XML — capabilities before project-specific rules, so the model
    //    knows what it can do before reading constraints.
    if !opts.skills.is_empty() && has_tool(&opts.tools, "read") {
        let visible: Vec<_> = opts
            .skills
            .iter()
            .filter(|s| !s.disable_model_invocation)
            .collect();
        if !visible.is_empty() {
            sections.push(format_skills_section(&visible));
        }
    }

    // 3. Project context (AGENTS.md / CLAUDE.md)
    if !opts.agent_content.is_empty() {
        sections.push(format!(
            "# Project Context\n\nProject-specific instructions and guidelines:\n\n{}",
            opts.agent_content.trim()
        ));
    }

    // 4. Workspace memory (FUTURE.md) — always present so the model knows about
    //    the feature even before the file exists. FUTURE.md is an INDEX, not a
    //    log: details live in topic files under .future/memory/ and are read on
    //    demand. The loader enforces the caps below (lint_memory_index); the
    //    advertised numbers are the same constants, so text and enforcement
    //    cannot drift apart.
    {
        let mut part = format!(
            "# Workspace Memory\n\n\
             FUTURE.md in the working directory is your memory INDEX, loaded here. \
             Details live in topic files under `.future/memory/` — read one when an \
             index line looks relevant. The index is capped ({MEMORY_INDEX_MAX_ENTRIES} \
             entries / {MEMORY_INDEX_MAX_LINES} lines / {MEMORY_INDEX_MAX_KB}KB) and \
             linted at load time; overflow and malformed lines get truncated with a \
             warning.\n\n\
             Entry format — every entry ends with its last-confirmed date:\n\
             - [Title](.future/memory/<topic>.md) — one-line hook, ≤100 chars (2026-08-12)\n\
             - [user] short fact the user explicitly asked remembered (2026-08-12)\n\n\
             To save: write detail to `.future/memory/<topic>.md` (title + \
             self-contained), then add ONE index line. A short user-requested fact \
             may go inline as a [user] entry. Prefer updating an existing \
             entry/topic file over adding new ones.\n\n\
             Record only when — the user asks you to remember; the user corrects or \
             confirms a non-obvious approach; you find a durable preference, \
             convention, or toolchain gotcha — AND both gates pass: (1) future-me \
             could NOT re-derive it from the repo in under a minute; (2) I can say \
             why it still matters in 3 months. Never record in-progress work, \
             goal/task status, PR/activity lists, or fix narratives — even if \
             asked; save only the surprising, non-derivable part. Closing a goal \
             or PR is NOT a memory event.\n\n\
             Freshness: verify any entry older than ~7 days before relying on it; \
             refresh its date if still valid. When a memory turns stale, exit it: \
             delete BOTH the index line and its topic file (unless another entry \
             links to it), and tell the user in one line what you removed. If the \
             index is full, evict oldest-date first. Memory writes go only to \
             FUTURE.md / .future/memory/ — never to CLAUDE.md, AGENTS.md, or \
             GEMINI.md.",
        );
        if !opts.memory_content.is_empty() {
            part.push_str("\n\n");
            part.push_str(opts.memory_content.trim());
        }
        sections.push(part);
    }

    // 5. Append prompt — placed late so user overrides can take precedence
    //    over earlier rules without being diluted by metadata.
    if !opts.append_prompt.is_empty() {
        sections.push(opts.append_prompt.clone());
    }

    // 6. Environment: date, working directory, host platform, and session info
    //    — always included so the model generates platform-appropriate shell
    //    commands, paths, and can self-identify its own session.
    {
        let mut info = vec!["# Environment".to_string(), String::new()];
        if !opts.date.is_empty() {
            info.push(format!("Current date: {}", opts.date));
        }
        if !opts.working_directory.is_empty() {
            info.push(format!(
                "Current working directory: {}",
                opts.working_directory
            ));
            info.push(
                "When looking for a file, search within the current working directory \
                 first; only widen the search to the rest of the filesystem if it is \
                 clearly not there. Avoid scanning the entire filesystem up front."
                    .to_string(),
            );
        }
        if !opts.session_id.is_empty() {
            info.push(format!("Current session ID: {}", opts.session_id));
            info.push(
                "You can reference this session ID when you need to identify or \
                 report which conversation you are part of. This is your own \
                 session — you are self-aware of this identifier."
                    .to_string(),
            );
        }
        if !opts.model.is_empty() {
            info.push(format!("Current model: {}", opts.model));
            info.push(
                "This is the model (provider/model) you are running as — \
                 reference it when asked which model you are."
                    .to_string(),
            );
        }
        if !opts.thinking_level.is_empty() {
            info.push(format!("Thinking level: {}", opts.thinking_level));
        }
        info.push(os_hint());
        sections.push(info.join("\n"));
    }

    sections.join("\n\n")
}

// ─── Memory index lint ──────────────────────────────────────────────────────

/// Hard caps for the FUTURE.md memory index. The workspace-memory prompt
/// section advertises these exact constants via format! captures, so the
/// documented caps and the enforced caps cannot drift apart.
pub const MEMORY_INDEX_MAX_ENTRIES: usize = 30;
pub const MEMORY_INDEX_MAX_LINES: usize = 100;
pub const MEMORY_INDEX_MAX_KB: usize = 12;
pub const MEMORY_INDEX_MAX_BYTES: usize = MEMORY_INDEX_MAX_KB * 1024;

/// Validate and bound the FUTURE.md memory index before injection into the
/// system prompt. Returns the (possibly truncated) content; when any issue is
/// found, a `> WARNING:` block is appended so the model sees it on every load
/// and can repair the file. Checks, in order:
///   - size: truncate to MEMORY_INDEX_MAX_LINES / MEMORY_INDEX_MAX_BYTES
///     (line-truncate first, then byte-truncate at the last newline so we
///     never cut mid-line — long single lines are a real failure mode);
///   - format: content lines must be `- [Title](path) — hook (YYYY-MM-DD)` or
///     `- [user] fact (YYYY-MM-DD)` (blank lines and `#`/`>` lines are
///     structural and pass);
///   - links: link targets must exist under `base_dir`;
///   - count: more than MEMORY_INDEX_MAX_ENTRIES entries.
pub fn lint_memory_index(raw: &str, base_dir: &std::path::Path) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut warnings: Vec<String> = Vec::new();

    let line_count = trimmed.lines().count();
    let byte_count = trimmed.len();
    let mut content = trimmed.to_string();
    if line_count > MEMORY_INDEX_MAX_LINES || byte_count > MEMORY_INDEX_MAX_BYTES {
        let mut cut = trimmed
            .lines()
            .take(MEMORY_INDEX_MAX_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        if cut.len() > MEMORY_INDEX_MAX_BYTES {
            let mut end = MEMORY_INDEX_MAX_BYTES;
            while !cut.is_char_boundary(end) {
                end -= 1;
            }
            match cut[..end].rfind('\n') {
                Some(nl) => cut.truncate(nl),
                None => cut.truncate(end),
            }
        }
        warnings.push(format!(
            "truncated to {MEMORY_INDEX_MAX_LINES} lines / {MEMORY_INDEX_MAX_BYTES} bytes \
             (was {line_count} lines / {byte_count} bytes) — keep entries to one line; \
             move detail into .future/memory/ topic files"
        ));
        content = cut;
    }

    let mut entries = 0usize;
    let mut malformed: Vec<usize> = Vec::new();
    let mut dead: Vec<String> = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') || line.starts_with('>') {
            continue;
        }
        entries += 1;
        match classify_entry(line) {
            EntryKind::Link(target) => {
                if !base_dir.join(target).exists() {
                    dead.push(target.to_string());
                }
            }
            EntryKind::Inline => {}
            EntryKind::Malformed => malformed.push(idx + 1),
        }
    }
    if !malformed.is_empty() {
        let at: Vec<String> = malformed.iter().map(|n| n.to_string()).collect();
        warnings.push(format!(
            "{} malformed line(s) at {} — expected `- [Title](.future/memory/<file>.md) — hook (YYYY-MM-DD)` or `- [user] fact (YYYY-MM-DD)`",
            malformed.len(),
            at.join(", ")
        ));
    }
    if !dead.is_empty() {
        warnings.push(format!(
            "dead link(s): {} — repair the target or exit the entry (delete the line and its topic file)",
            dead.join(", ")
        ));
    }
    if entries > MEMORY_INDEX_MAX_ENTRIES {
        warnings.push(format!(
            "{entries} entries exceed the {MEMORY_INDEX_MAX_ENTRIES}-entry cap — the index is a map, not a ledger; evict oldest-date first"
        ));
    }

    if warnings.is_empty() {
        return content;
    }
    let mut out = content;
    out.push_str("\n\n> WARNING: FUTURE.md index issues detected at load — please repair:\n");
    for w in &warnings {
        out.push_str("> - ");
        out.push_str(w);
        out.push('\n');
    }
    out
}

enum EntryKind<'a> {
    Link(&'a str),
    Inline,
    Malformed,
}

/// Classify one content line as a link entry (yielding its link target), an
/// inline `[user]` entry, or malformed. Every valid entry ends with its
/// last-confirmed date `(YYYY-MM-DD)`. `- [user] ` (space after the bracket)
/// cannot collide with a link entry titled "user" — that would be `[user](`.
fn classify_entry(line: &str) -> EntryKind<'_> {
    if !ends_with_date(line) {
        return EntryKind::Malformed;
    }
    if line.starts_with("- [user] ") {
        return EntryKind::Inline;
    }
    let Some(rest) = line.strip_prefix("- [") else {
        return EntryKind::Malformed;
    };
    let Some(open) = rest.find("](") else {
        return EntryKind::Malformed;
    };
    let after_open = &rest[open + 2..];
    let Some(close) = after_open.find(')') else {
        return EntryKind::Malformed;
    };
    let target = &after_open[..close];
    if target.is_empty() || !after_open[close + 1..].contains(" — ") {
        return EntryKind::Malformed;
    }
    EntryKind::Link(target)
}

/// `(YYYY-MM-DD)` at end of line — shape check only (digits and dashes in
/// the right positions), not a calendar validation.
fn ends_with_date(line: &str) -> bool {
    let b = line.as_bytes();
    let n = b.len();
    // "(YYYY-MM-DD)" is exactly 12 bytes.
    if n < 12 || b[n - 12] != b'(' || b[n - 1] != b')' {
        return false;
    }
    let d = &b[n - 11..n - 1];
    d.iter().enumerate().all(|(i, c)| {
        if i == 4 || i == 7 {
            *c == b'-'
        } else {
            c.is_ascii_digit()
        }
    })
}

#[derive(Debug, Clone, Default)]
pub struct PromptOptions {
    pub custom_prompt: String,
    pub working_directory: String,
    pub date: String,
    pub tools: Vec<AgentTool>,
    pub skills: Vec<Skill>,
    pub agent_content: String,
    /// Workspace memory (FUTURE.md). Injected as its own section, separate from
    /// `agent_content` (project context), so memory and human-authored project
    /// instructions never shadow each other. Expected to be an index (details
    /// live in `.future/memory/`) — pass it through [`lint_memory_index`]
    /// first so bloat and malformed entries surface as a visible warning.
    pub memory_content: String,
    pub append_prompt: String,
    pub prompt_guidelines: Vec<String>,
    /// Session ID — injected into the environment section so the model can
    /// self-identify and reference its own conversation.
    pub session_id: String,
    /// Model id (provider/model) — injected into the environment section so
    /// the model can self-report which model it runs as.
    pub model: String,
    /// Thinking level — injected into the environment section so the model
    /// knows its reasoning mode.
    pub thinking_level: String,
}

// ─── Identity Section ───────────────────────────────────────────────────────

fn build_identity_section(opts: &PromptOptions) -> String {
    let mut parts = vec![];

    // Identity
    parts.push("You are an expert coding assistant operating inside FutureAgent, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.".to_string());

    // Tool list
    let tools_list = if opts.tools.is_empty() {
        "(none)".to_string()
    } else {
        opts.tools
            .iter()
            .map(|t| {
                format!(
                    "- {}: {}",
                    t.def.function.name,
                    first_sentence(&t.def.function.description)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    parts.push("Available tools:".to_string());
    parts.push(tools_list);
    parts.push("In addition to the tools above, you may have access to other custom tools depending on the project.".to_string());

    // Dynamic tool guidelines
    let tool_names: Vec<&str> = opts
        .tools
        .iter()
        .map(|t| t.def.function.name.as_str())
        .collect();
    let mut guidelines: Vec<String> = build_dynamic_tool_guidelines(&tool_names);
    // PromptGuidelines from opts
    for g in &opts.prompt_guidelines {
        guidelines.push(g.clone());
    }
    // Per-tool guidelines
    for g in opts.tools.iter().flat_map(|t| t.guidelines.iter()) {
        guidelines.push(g.clone());
    }
    // Default behavioral guidelines (always appended last)
    guidelines.push("Be concise in your responses".to_string());
    guidelines.push("Show file paths clearly when working with files".to_string());
    guidelines.push("Write ordinary responses in standard Markdown. To reference a file you created or edited on disk, use a normal Markdown link whose destination is the file path from the write tool result: [name](<path>). Wrap the path in angle brackets so paths with spaces work, and write it verbatim (an absolute path keeps its leading slash; a workspace-relative path MUST start with ./ — e.g. [notes.txt](<./notes.txt>), never [notes.txt](<notes.txt>)). Use forward slashes even on Windows. Do NOT percent-encode the path or use any custom URL scheme.".to_string());
    // Minimal link mode: application-object references (futureos:// links and
    // futureos-* fenced embeds) are disabled while we trial the simplest link
    // set. The GUI no longer renders them (see desktop parseFutureMarkdown.ts), so we
    // don't instruct the model to emit them. File links above are unaffected.
    // To restore, uncomment the two guidelines below.
    // guidelines.push("Only use an id-based reference — [label](futureos://artifact/<id>), [label](futureos://run/<id>), [label](futureos://tool/<id>), [label](futureos://approval/<id>), or [label](futureos://review/<id>) — when you actually have that object's id from earlier in the conversation or tool results. NEVER invent or guess an id; if you don't have one (e.g. a file you just wrote), use a plain [name](<path>) file link instead. Prefer a reference over pasting long stdout, full diffs, or large file contents inline.".to_string());
    // guidelines.push("For block-level FutureOS objects, use fenced directives with language names such as `futureos-artifact`, `futureos-run`, `futureos-tool`, `futureos-approval`, or `futureos-review`, and include id and view fields. Do not embed long stdout, full diffs, or large file contents directly in the assistant message when an object reference is available.".to_string());
    // The default guidelines above guarantee the list is non-empty.
    let deduped = dedup(guidelines);
    let lines: Vec<String> = deduped.iter().map(|g| format!("- {}", g)).collect();
    parts.push("Guidelines:".to_string());
    parts.push(lines.join("\n"));

    parts.join("\n\n")
}

fn build_dynamic_tool_guidelines(tool_names: &[&str]) -> Vec<String> {
    let has_shell = tool_names.contains(&"shell");

    let mut guidelines = vec![];

    if has_shell {
        // Platform-matched examples: the same tool speaks bash on Unix and
        // PowerShell 5.1 on Windows (see sandbox::shell_invocation).
        #[cfg(not(target_os = "windows"))]
        guidelines.push(
            "Use the shell tool for command-line exploration such as ls, rg, and find; but to read a known file's contents use the read tool, not cat. Prefer write/edit tools for ordinary file writes."
                .to_string(),
        );
        #[cfg(target_os = "windows")]
        guidelines.push(
            "Use the shell tool (PowerShell) for command-line exploration such as Get-ChildItem and Select-String; but to read a known file's contents use the read tool, not Get-Content. Prefer write/edit tools for ordinary file writes."
                .to_string(),
        );
    }

    guidelines
}

// ─── Skills Section ─────────────────────────────────────────────────────────

/// Formats skills with lead-in text + <available_skills> XML block.
/// Matches 's formatSkillsSection() exactly.
fn format_skills_section(skills: &[&Skill]) -> String {
    let mut sb = String::new();
    sb.push_str("# Available Skills\n\n");
    sb.push_str("The following skills provide specialized instructions for specific tasks.\n");
    sb.push_str(
        "Use the read tool to load a skill's file when the task matches its description.\n",
    );
    sb.push_str("When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.\n");
    sb.push_str("A `/<skill-name>` token in a user message (at the start or mid-sentence) is an explicit invocation: load and follow that skill's SKILL.md, and treat the rest of the message as its input.\n");
    sb.push('\n');
    sb.push_str("<available_skills>\n");
    for s in skills {
        sb.push_str("  <skill>\n");
        sb.push_str(&format!("    <name>{}</name>\n", escape_xml(&s.name)));
        sb.push_str(&format!(
            "    <description>{}</description>\n",
            escape_xml(&s.description)
        ));
        sb.push_str(&format!(
            "    <location>{}</location>\n",
            escape_xml(&s.location)
        ));
        sb.push_str("  </skill>\n");
    }
    sb.push_str("</available_skills>");
    sb
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn has_tool(tools: &[AgentTool], name: &str) -> bool {
    tools.iter().any(|t| t.def.function.name == name)
}

fn first_sentence(desc: &str) -> String {
    if let Some(idx) = desc.find('.') {
        desc[..=idx].to_string()
    } else {
        desc.to_string()
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn dedup(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = vec![];
    for item in items {
        let lower = item.to_lowercase();
        if seen.insert(lower) {
            result.push(item);
        }
    }
    result
}

/// Returns an OS platform hint so the model generates platform-appropriate
/// shell commands (e.g. `dir` vs `ls`, path separators, package managers).
fn os_hint() -> String {
    os_hint_for(
        std::env::consts::OS,
        crate::sandbox::shell_display_name(),
        crate::sandbox::shell_is_legacy_bash(),
        crate::sandbox::shell_supports_chain_operators(),
    )
}

/// `os_hint` with platform and shell facts injected, so every platform arm is
/// testable from any host (`std::env::consts::OS` is a compile-time constant —
/// without injection the other arms are dead code on the test host).
fn os_hint_for(os: &str, shell: &str, legacy_bash: bool, supports_chaining: bool) -> String {
    let skills_hint = "Skill files are located under the user's home directory \
        at .agents/skills/<name>/SKILL.md. When creating a new skill, \
        construct the path by joining the home directory with this relative path \
        using the correct path separator for this platform.";

    match os {
        "macos" => {
            // Name the shell actually resolved at runtime ($SHELL — often zsh on
            // macOS). bash and zsh share command-line syntax, so no separate
            // syntax rules are needed — only the accurate name.
            let legacy_note = if legacy_bash {
                " IMPORTANT: This is bash 3.2 — do NOT use bash 4+ features: \
                 no associative arrays (declare -A), no globstar \
                 (**), no ${var,,}/${var^^}, no mapfile/readarray. Use \
                 POSIX-compatible syntax only."
            } else {
                ""
            };
            format!(
                "Host platform: macOS. Shell commands are interpreted by {shell} \
                 (POSIX shell syntax); macOS command-line tools (BSD variants) apply.\
                 {legacy_note} \
                 {skills_hint} (Example: ~/.agents/skills/my-skill/SKILL.md)"
            )
        }
        "windows" => {
            // The interpreter is resolved at runtime (pwsh 7 when present, else
            // Windows PowerShell 5.1); only pwsh 7 supports `&&`/`||`, so the
            // chaining guidance tracks the actual shell rather than guessing.
            let chaining = if supports_chaining {
                "chain commands with `;`, `&&`, or `||`"
            } else {
                // PowerShell 5.1 rejects `&&`/`||` at parse time. `;` runs the
                // next command unconditionally; to run one ONLY if the previous
                // succeeded, use `cmd1; if ($?) { cmd2 }`.
                "chain commands with `;` (run-if-previous-succeeded is \
                 `cmd1; if ($?) { cmd2 }`); never use `&&` or `||` — this \
                 PowerShell version rejects them at parse time"
            };
            format!(
                "Host platform: Windows. Shell commands are interpreted by \
                 {shell} — NOT cmd and NOT bash. Use PowerShell syntax only: \
                 {chaining}, environment variables as $env:VAR (never %VAR%), \
                 path separators \\ (not /). \
                 {skills_hint} (Example: $env:USERPROFILE\\.agents\\skills\\my-skill\\SKILL.md)"
            )
        }
        "linux" => {
            let legacy_note = if legacy_bash {
                " IMPORTANT: This is bash 3.x — do NOT use bash 4+ features: \
                 no associative arrays (declare -A), no globstar \
                 (**), no ${var,,}/${var^^}, no mapfile/readarray. Use \
                 POSIX-compatible syntax only."
            } else {
                ""
            };
            format!(
                "Host platform: Linux. Shell commands are interpreted by {shell} \
                 (POSIX shell syntax).{legacy_note} \
                 {skills_hint} (Example: ~/.agents/skills/my-skill/SKILL.md)"
            )
        }
        other => format!("Host platform: {other}. {skills_hint}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_reports_model_and_thinking_level() {
        let prompt = build_prompt(&PromptOptions {
            session_id: "sess-123".to_string(),
            model: "future/deepseek-v4-flash".to_string(),
            thinking_level: "high".to_string(),
            ..Default::default()
        });
        assert!(prompt.contains("Current session ID: sess-123"));
        assert!(prompt.contains("Current model: future/deepseek-v4-flash"));
        assert!(prompt.contains("Thinking level: high"));
    }

    #[test]
    fn workspace_memory_is_a_separate_layer_from_project_context() {
        let prompt = build_prompt(&PromptOptions {
            agent_content: "Use 2-space indent.".to_string(),
            memory_content: "User prefers pnpm over npm.".to_string(),
            ..Default::default()
        });

        // Both layers present, each under its own heading — neither shadows the other.
        assert!(prompt.contains("# Project Context"));
        assert!(prompt.contains("Use 2-space indent."));
        assert!(prompt.contains("# Workspace Memory"));
        assert!(prompt.contains("User prefers pnpm over npm."));
        assert!(prompt.contains("FUTURE.md"));
    }

    #[test]
    fn workspace_memory_section_present_even_when_empty() {
        let prompt = build_prompt(&PromptOptions {
            agent_content: "Project rules.".to_string(),
            ..Default::default()
        });
        // Section header and operational rules always appear so the model
        // knows about FUTURE.md before the file exists.
        assert!(prompt.contains("# Project Context"));
        assert!(prompt.contains("# Workspace Memory"));
        assert!(prompt.contains("FUTURE.md"));
    }

    #[test]
    fn memory_section_advertises_the_enforced_caps() {
        // The prompt text must quote the same constants the loader enforces,
        // or the model would be told limits that differ from reality.
        let prompt = build_prompt(&PromptOptions::default());
        let expected = format!(
            "{} entries / {} lines / {}KB",
            MEMORY_INDEX_MAX_ENTRIES, MEMORY_INDEX_MAX_LINES, MEMORY_INDEX_MAX_KB
        );
        assert!(prompt.contains(&expected));
    }

    // ─── lint_memory_index ──────────────────────────────────────────────

    #[test]
    fn lint_empty_index_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(lint_memory_index("", dir.path()), "");
        assert_eq!(lint_memory_index("  \n \n", dir.path()), "");
    }

    #[test]
    fn lint_clean_index_passes_through_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join(".future/memory");
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(mem.join("a.md"), "# A\n").unwrap();
        let index = "# FUTURE.md — Index\n\n\
                     - [A](.future/memory/a.md) — read when doing a (2026-08-12)\n\
                     - [user] prefers pnpm over npm (2026-08-01)\n";
        let out = lint_memory_index(index, dir.path());
        assert_eq!(out, index.trim());
        assert!(!out.contains("WARNING"));
    }

    #[test]
    fn lint_flags_malformed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let index = "- [A](.future/memory/a.md) — missing date\n\
                     prose that is not an entry\n\
                     - [user] also missing a date\n";
        let out = lint_memory_index(index, dir.path());
        assert!(out.contains("WARNING"));
        assert!(out.contains("3 malformed line(s) at 1, 2, 3"));
    }

    #[test]
    fn lint_flags_dead_links() {
        let dir = tempfile::tempdir().unwrap();
        let index = "- [Gone](.future/memory/gone.md) — was deleted (2026-08-12)\n";
        let out = lint_memory_index(index, dir.path());
        assert!(out.contains("dead link(s): .future/memory/gone.md"));
    }

    #[test]
    fn lint_warns_when_over_entry_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = String::new();
        for i in 0..=MEMORY_INDEX_MAX_ENTRIES {
            index.push_str(&format!("- [user] fact number {i} (2026-08-12)\n"));
        }
        let out = lint_memory_index(&index, dir.path());
        assert!(out.contains("exceed the 30-entry cap"));
        assert!(out.contains("evict oldest-date first"));
    }

    #[test]
    fn lint_truncates_overlong_index_at_line_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = String::new();
        for i in 0..(MEMORY_INDEX_MAX_LINES + 20) {
            index.push_str(&format!("- [user] fact {i} (2026-08-12)\n"));
        }
        let out = lint_memory_index(&index, dir.path());
        assert!(out.contains("truncated to 100 lines"));
        // Only the cap's worth of lines survive (plus the warning block).
        assert!(!out.contains("fact 119"));
        assert!(out.contains("fact 99"));
    }

    #[test]
    fn lint_truncates_oversize_index_at_byte_cap_on_line_boundary() {
        let dir = tempfile::tempdir().unwrap();
        // CJK hooks: 3 bytes each, so the byte cap lands mid-codepoint and the
        // char-boundary backoff runs; truncation still ends on a line boundary.
        let hook = "界".repeat(46);
        let mut index = String::new();
        for _ in 0..95 {
            index.push_str(&format!("- [user] {hook} (2026-08-12)\n"));
        }
        assert!(index.len() > MEMORY_INDEX_MAX_BYTES);
        assert!(index.lines().count() <= MEMORY_INDEX_MAX_LINES);
        let out = lint_memory_index(&index, dir.path());
        assert!(out.contains("truncated to 100 lines / 12288 bytes"));
        // Byte-truncated at a newline: the content portion ends with a full line.
        let content = out.split("\n\n> WARNING").next().unwrap();
        assert!(content.ends_with("(2026-08-12)"));
    }

    #[test]
    fn lint_structural_lines_are_not_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join(".future/memory");
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(mem.join("a.md"), "# A\n").unwrap();
        let index = "# Title\n\n## Section\n\n> a quote line\n\n\
                     - [A](.future/memory/a.md) — only real entry (2026-08-12)\n";
        let out = lint_memory_index(index, dir.path());
        assert!(!out.contains("WARNING"));
    }

    #[test]
    fn lint_byte_truncates_single_huge_line_without_newline() {
        let dir = tempfile::tempdir().unwrap();
        // No newline before the byte cap → fall back to a raw byte cut.
        let index = "x".repeat(MEMORY_INDEX_MAX_BYTES + 500);
        let out = lint_memory_index(&index, dir.path());
        assert!(out.contains("truncated to 100 lines / 12288 bytes"));
        let content = out.split("\n\n> WARNING").next().unwrap();
        assert_eq!(content.len(), MEMORY_INDEX_MAX_BYTES);
    }

    #[test]
    fn classify_entry_distinguishes_user_inline_from_user_titled_link() {
        assert!(matches!(
            classify_entry("- [user] prefers pnpm (2026-08-12)"),
            EntryKind::Inline
        ));
        assert!(matches!(
            classify_entry("- [user](.future/memory/u.md) — hook (2026-08-12)"),
            EntryKind::Link(".future/memory/u.md")
        ));
        // Missing the hook separator after the link target.
        assert!(matches!(
            classify_entry("- [A](.future/memory/a.md) (2026-08-12)"),
            EntryKind::Malformed
        ));
        // Empty link target.
        assert!(matches!(
            classify_entry("- [A]() — hook (2026-08-12)"),
            EntryKind::Malformed
        ));
        // No date suffix.
        assert!(matches!(
            classify_entry("- [user] prefers pnpm"),
            EntryKind::Malformed
        ));
    }

    #[test]
    fn ends_with_date_checks_shape_not_calendar() {
        assert!(ends_with_date("x (2026-08-12)"));
        assert!(ends_with_date("x (9999-99-99)")); // shape only
        assert!(!ends_with_date("x (2026-8-12)"));
        assert!(!ends_with_date("x (2026-08-12) trailing"));
        assert!(!ends_with_date("x (2026/08/12)"));
        assert!(!ends_with_date("(2026-08-1")); // too short overall
    }

    #[test]
    fn build_prompt_with_custom_prompt() {
        let prompt = build_prompt(&PromptOptions {
            custom_prompt: "You are a custom assistant.".to_string(),
            ..Default::default()
        });
        assert!(prompt.contains("You are a custom assistant."));
        // Should not have the default identity section
        assert!(!prompt.contains("You are an expert coding assistant"));
    }

    #[test]
    fn build_prompt_with_append() {
        let prompt = build_prompt(&PromptOptions {
            append_prompt: "EXTRA: Always use TypeScript.".to_string(),
            ..Default::default()
        });
        assert!(prompt.contains("EXTRA: Always use TypeScript."));
    }

    #[test]
    fn build_prompt_with_date_and_cwd() {
        let prompt = build_prompt(&PromptOptions {
            date: "2026-07-23".to_string(),
            working_directory: "/Users/test/project".to_string(),
            ..Default::default()
        });
        assert!(prompt.contains("Current date: 2026-07-23"));
        assert!(prompt.contains("Current working directory: /Users/test/project"));
    }

    #[test]
    fn build_prompt_with_skills() {
        let skill = crate::skills::Skill {
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            name_zh: None,
            description_zh: None,
            version: Some("1.0".to_string()),
            location: "/path/to/skill".to_string(),
            disable_model_invocation: false,
        };
        let tool = crate::tools::read_tool();
        let prompt = build_prompt(&PromptOptions {
            skills: vec![skill],
            tools: vec![tool],
            ..Default::default()
        });
        assert!(prompt.contains("test-skill"));
        assert!(prompt.contains("<available_skills>"));
    }

    #[test]
    fn has_tool_finds_matching() {
        let tools = crate::tools::coding_tools();
        assert!(has_tool(&tools, "shell"));
        assert!(has_tool(&tools, "read"));
        assert!(!has_tool(&tools, "nonexistent"));
    }

    #[test]
    fn first_sentence_truncates_at_period() {
        assert_eq!(first_sentence("Hello world. Rest"), "Hello world.");
        assert_eq!(first_sentence("No period"), "No period");
    }

    #[test]
    fn escape_xml_escapes_all() {
        assert_eq!(
            escape_xml("<tag>\"quoted\"&'single'</tag>"),
            "&lt;tag&gt;&quot;quoted&quot;&amp;&apos;single&apos;&lt;/tag&gt;"
        );
    }

    #[test]
    fn dedup_removes_case_insensitive_duplicates() {
        let items = vec![
            "First".to_string(),
            "FIRST".to_string(),
            "first".to_string(),
            "second".to_string(),
        ];
        let result = dedup(items);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "First");
        assert_eq!(result[1], "second");
    }

    #[test]
    fn os_hint_covers_every_platform_arm() {
        // macOS: legacy-bash note present/absent.
        let mac_legacy = os_hint_for("macos", "bash", true, true);
        assert!(mac_legacy.contains("Host platform: macOS"));
        assert!(mac_legacy.contains("bash 3.2"));
        let mac_modern = os_hint_for("macos", "zsh", false, true);
        assert!(mac_modern.contains("Host platform: macOS"));
        assert!(!mac_modern.contains("bash 3.2"));
        // Windows: chaining guidance tracks pwsh vs Windows PowerShell 5.1.
        let win_pwsh = os_hint_for("windows", "PowerShell 7 (pwsh)", false, true);
        assert!(win_pwsh.contains("Host platform: Windows"));
        assert!(win_pwsh.contains("&&"));
        let win_legacy = os_hint_for("windows", "Windows PowerShell 5.1", false, false);
        assert!(win_legacy.contains("Host platform: Windows"));
        assert!(win_legacy.contains("if ($?)"));
        assert!(win_legacy.contains("never use `&&`"));
        // Linux: legacy-bash note present/absent.
        let linux_legacy = os_hint_for("linux", "bash", true, true);
        assert!(linux_legacy.contains("Host platform: Linux"));
        assert!(linux_legacy.contains("bash 3.x"));
        let linux_modern = os_hint_for("linux", "bash", false, true);
        assert!(linux_modern.contains("Host platform: Linux"));
        assert!(!linux_modern.contains("bash 3.x"));
        // Unknown OS: bare fallback.
        let other = os_hint_for("freebsd", "sh", false, true);
        assert!(other.starts_with("Host platform: freebsd."));
    }

    #[test]
    fn os_hint_returns_non_empty() {
        let hint = os_hint();
        assert!(!hint.is_empty());
        assert!(hint.contains("Host platform"));
    }

    #[test]
    fn format_skills_section_produces_xml() {
        let skill = crate::skills::Skill {
            name: "my-skill".to_string(),
            description: "Does things".to_string(),
            name_zh: None,
            description_zh: None,
            version: None,
            location: "/home/.agents/skills/my-skill/SKILL.md".to_string(),
            disable_model_invocation: false,
        };
        let xml = format_skills_section(&[&skill]);
        assert!(xml.contains("<available_skills>"));
        assert!(xml.contains("my-skill"));
        assert!(xml.contains("Does things"));
    }

    #[test]
    fn build_dynamic_tool_guidelines_returns_vec() {
        let guidelines = build_dynamic_tool_guidelines(&["shell", "read", "write", "edit"]);
        assert!(!guidelines.is_empty());
    }

    #[test]
    fn build_prompt_without_skills_or_tools() {
        let prompt = build_prompt(&PromptOptions::default());
        // Should still contain workspace memory and environment sections
        assert!(prompt.contains("# Workspace Memory"));
        assert!(prompt.contains("# Environment"));
    }
}
