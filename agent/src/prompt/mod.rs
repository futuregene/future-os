//! Prompt building — 1:1 compatible with internal/prompt/

use crate::skills::Skill;
use crate::types::AgentTool;

// ─── Public API ─────────────────────────────────────────────────────────────

/// BuildPrompt produces a fully assembled system prompt from the given options.
/// Section ordering matches 's BuildPrompt():
///   1. Identity (identity + tools list + guidelines)
///   2. Append prompt
///   3. Project context (CLAUDE.md / AGENTS.md / GEMINI.md)
///   4. Workspace memory (FUTURE.md)
///   5. Skills XML (with lead-in text, only if read tool is available)
///   6. Date + working directory
pub fn build_prompt(opts: &PromptOptions) -> String {
    let mut sections = vec![];

    // 1. Identity
    if !opts.custom_prompt.is_empty() {
        sections.push(opts.custom_prompt.clone());
    } else {
        sections.push(build_identity_section(opts));
    }

    // 2. Append prompt
    if !opts.append_prompt.is_empty() {
        sections.push(opts.append_prompt.clone());
    }

    // 3. Project context (AGENTS.md / CLAUDE.md)
    if !opts.agent_content.is_empty() {
        sections.push(format!(
            "# Project Context\n\nProject-specific instructions and guidelines:\n\n{}",
            opts.agent_content.trim()
        ));
    }

    // 4. Workspace memory (FUTURE.md) — separate layer from project context.
    if !opts.memory_content.is_empty() {
        sections.push(format!(
            "# Workspace Memory\n\nThe following is the persistent memory for this \
             workspace, stored in FUTURE.md — notes you previously saved or the user \
             provided. Treat it as authoritative: preferences, conventions, \
             build/test/run commands, and facts worth remembering across sessions.\n\n{}",
            opts.memory_content.trim()
        ));
    }

    // 5. Skills XML (only if read tool is available)
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

    // 6. Date and working directory
    if !opts.date.is_empty() || !opts.working_directory.is_empty() {
        let mut info = vec![];
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
        sections.push(info.join("\n"));
    }

    // 7. Host platform — always included so the model generates
    //    platform-appropriate shell commands and paths.
    sections.push(os_hint());

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
    // guidelines.push("Only use an id-based reference — [label](futureos://artifact/<id>), [label](futureos://run/<id>), [label](futureos://tool/<id>), [label](futureos://approval/<id>), [label](futureos://review/<id>), or [label](futureos://research/<id>) — when you actually have that object's id from earlier in the conversation or tool results. NEVER invent or guess an id; if you don't have one (e.g. a file you just wrote), use a plain [name](<path>) file link instead. Prefer a reference over pasting long stdout, full diffs, or large file contents inline.".to_string());
    // guidelines.push("For block-level FutureOS objects, use fenced directives with language names such as `futureos-artifact`, `futureos-run`, `futureos-tool`, `futureos-approval`, `futureos-review`, or `futureos-research`, and include id and view fields. Do not embed long stdout, full diffs, or large file contents directly in the assistant message when an object reference is available.".to_string());
    let deduped = dedup(guidelines);
    if !deduped.is_empty() {
        let lines: Vec<String> = deduped.iter().map(|g| format!("- {}", g)).collect();
        parts.push("Guidelines:".to_string());
        parts.push(lines.join("\n"));
    }

    parts.join("\n\n")
}

fn build_dynamic_tool_guidelines(tool_names: &[&str]) -> Vec<String> {
    let has_bash = tool_names.contains(&"bash");

    let mut guidelines = vec![];

    if has_bash {
        guidelines.push(
            "Use bash for command-line exploration such as ls, rg, and find; prefer write/edit tools for ordinary file writes."
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
    match std::env::consts::OS {
        "macos" => "Host platform: macOS. Use macOS/zsh shell commands. "
            .to_string(),
        "windows" => "Host platform: Windows. Use Windows shell commands: "
            .to_string()
            + "dir (not ls), type (not cat), path separators \\ (not /). "
            + "PowerShell or cmd syntax is acceptable.",
        "linux" => "Host platform: Linux.".to_string(),
        other => format!("Host platform: {other}."),
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
        assert!(prompt.contains("stored in FUTURE.md"));
    }

    #[test]
    fn workspace_memory_section_omitted_when_empty() {
        let prompt = build_prompt(&PromptOptions {
            agent_content: "Project rules.".to_string(),
            ..Default::default()
        });
        assert!(prompt.contains("# Project Context"));
        assert!(!prompt.contains("# Workspace Memory"));
    }
}
