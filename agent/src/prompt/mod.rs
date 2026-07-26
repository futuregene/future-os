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
    //    the feature even before the file exists. Operational rules live here
    //    instead of duplicating them in the guidelines section.
    {
        let mut part = String::from(
            "# Workspace Memory\n\n\
             You maintain a workspace memory file named FUTURE.md in the working \
             directory. Its content is loaded here — treat it as authoritative: \
             preferences, conventions, commands, and facts worth remembering \
             across sessions.\n\n\
             Record a memory when the user explicitly asks you to remember something.\n\n\
             Also record proactively when these events happen:\n\
             - The user corrects your approach (\"no, not that\", \"don't do X\", \
             \"use Y instead\"). Save what you learned and why.\n\
             - The user confirms a non-obvious choice you made was correct (\"yes \
             exactly\", \"that's right\", \"perfect\"). Corrections are easy to \
             notice; confirmations are quieter — watch for them.\n\
             - You learn a durable, high-value fact: a research question the user \
             is investigating, a data source location, an output format preference \
             (citation style, language, file format), or a preferred skill/tool \
             for a recurring task.\n\n\
             Do not record: one-off tasks, transient state, secrets, or anything \
             that can be re-derived by reading the project files. Use the write or \
             edit tool; keep entries concise; update stale entries instead of \
             duplicating; aim under ~200 lines. When you write to memory, tell the \
             user in one line what you recorded. Memory may only be written to \
             FUTURE.md — never to CLAUDE.md, AGENTS.md, or GEMINI.md.",
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

    // 6. Environment: date, working directory, and host platform — always
    //    included so the model generates platform-appropriate shell commands
    //    and paths.
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
        info.push(os_hint());
        sections.push(info.join("\n"));
    }

    sections.join("\n\n")
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
    /// instructions never shadow each other.
    pub memory_content: String,
    pub append_prompt: String,
    pub prompt_guidelines: Vec<String>,
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
    // set. The GUI no longer renders them (see gui parseFutureMarkdown.ts), so we
    // don't instruct the model to emit them. File links above are unaffected.
    // To restore, uncomment the two guidelines below.
    // guidelines.push("Only use an id-based reference — [label](futureos://artifact/<id>), [label](futureos://run/<id>), [label](futureos://tool/<id>), [label](futureos://approval/<id>), or [label](futureos://review/<id>) — when you actually have that object's id from earlier in the conversation or tool results. NEVER invent or guess an id; if you don't have one (e.g. a file you just wrote), use a plain [name](<path>) file link instead. Prefer a reference over pasting long stdout, full diffs, or large file contents inline.".to_string());
    // guidelines.push("For block-level FutureOS objects, use fenced directives with language names such as `futureos-artifact`, `futureos-run`, `futureos-tool`, `futureos-approval`, or `futureos-review`, and include id and view fields. Do not embed long stdout, full diffs, or large file contents directly in the assistant message when an object reference is available.".to_string());
    let deduped = dedup(guidelines);
    if !deduped.is_empty() {
        let lines: Vec<String> = deduped.iter().map(|g| format!("- {}", g)).collect();
        parts.push("Guidelines:".to_string());
        parts.push(lines.join("\n"));
    }

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
    let skills_hint = "Skill files are located under the user's home directory \
        at .agents/skills/<name>/SKILL.md. When creating a new skill, \
        construct the path by joining the home directory with this relative path \
        using the correct path separator for this platform.";

    match std::env::consts::OS {
        "macos" => {
            // Name the shell actually resolved at runtime ($SHELL — often zsh on
            // macOS). bash and zsh share command-line syntax, so no separate
            // syntax rules are needed — only the accurate name.
            let shell = crate::sandbox::shell_display_name();
            let legacy_note = if crate::sandbox::shell_is_legacy_bash() {
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
            let shell = crate::sandbox::shell_display_name();
            let chaining = if crate::sandbox::shell_supports_chain_operators() {
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
            let shell = crate::sandbox::shell_display_name();
            let legacy_note = if crate::sandbox::shell_is_legacy_bash() {
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
