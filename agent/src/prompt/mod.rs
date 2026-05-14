//! Prompt building — 1:1 compatible with Go internal/prompt/

use crate::skills::Skill;
use crate::types::AgentTool;

/// BuildPrompt produces a fully assembled system prompt from the given options.
pub fn build_prompt(opts: &PromptOptions) -> String {
    let mut sections = vec![];

    // 1. Identity / custom prompt
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

    // 4. Skills XML injection
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

    // 5. Date and working directory
    if !opts.date.is_empty() || !opts.working_directory.is_empty() {
        let mut info = vec![];
        if !opts.date.is_empty() {
            info.push(format!("Current date: {}", opts.date));
        }
        if !opts.working_directory.is_empty() {
            info.push(format!("Working directory: {}", opts.working_directory));
        }
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
    pub append_prompt: String,
}

fn build_identity_section(_opts: &PromptOptions) -> String {
    // Base identity + behavioral guidelines
    // In Go: buildIdentitySection()
    r#"You are FutureAgent, a highly capable AI coding assistant."#.to_string()
}

fn format_skills_section(skills: &[&Skill]) -> String {
    let mut xml = String::from("<skills>\n");
    for skill in skills {
        xml.push_str(&format!(
            "  <skill name=\"{}\" location=\"{}\">\n    {}\n  </skill>\n",
            skill.name,
            skill.location,
            skill.description.replace('\n', " ")
        ));
    }
    xml.push_str("</skills>");
    xml
}

fn has_tool(tools: &[AgentTool], name: &str) -> bool {
    tools.iter().any(|t| t.def.function.name == name)
}
