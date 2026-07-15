//! Skills discovery — 1:1 compatible with Go internal/skills/

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub name_zh: Option<String>,
    pub description_zh: Option<String>,
    pub version: Option<String>,
    pub location: String,
    #[serde(rename = "disableModelInvocation", default)]
    pub disable_model_invocation: bool,
}

/// Predefined skill directories (matching Go internal/skills/skills.go)
pub const APP_SKILLS_DIR: &str = "~/.future/agent/skills/";
pub const PROJECT_SKILLS_DIR: &str = ".future/agent/skills/";
pub const AGENTS_SKILLS_DIR: &str = "~/.agents/skills/";

/// DiscoverSkills finds all skills in the given directories.
/// Earlier directories take priority; duplicates (by name) are skipped.
pub fn discover_skills(dirs: &[String]) -> Result<Vec<Skill>> {
    let mut skills = vec![];
    let mut seen = std::collections::HashSet::new();
    for dir in dirs {
        let expanded = shellexpand::tilde(dir);
        let path = Path::new(&*expanded);
        if !path.exists() {
            continue;
        }
        for entry in WalkDir::new(path)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                let skill_md = entry_path.join("SKILL.md");
                if skill_md.exists() {
                    if let Some(skill) = parse_skill(&skill_md)? {
                        if seen.insert(skill.name.clone()) {
                            skills.push(skill);
                        }
                    }
                }
            }
        }
    }
    Ok(skills)
}

fn parse_skill(skill_md: &Path) -> Result<Option<Skill>> {
    let content = std::fs::read_to_string(skill_md)?;
    let name = extract_name(&content, skill_md)?;
    let description = extract_description(&content);
    let name_zh = extract_frontmatter_field(&content, "name_zh");
    let description_zh = extract_frontmatter_field(&content, "description_zh");
    let version = extract_frontmatter_field(&content, "version");
    // Normalize to forward slashes so the path survives transport through
    // the system prompt without backslash escape-sequence corruption
    // (e.g. \f, \a interpreted by the model on non-Windows hosts).
    let location = skill_md
        .to_string_lossy()
        .replace('\\', "/")
        // Strip the Windows extended-length prefix (\\?\) if present.
        .trim_start_matches("//?/")
        .to_string();
    let disable =
        content.contains("disableModelInvocation") || content.contains("disable_model_invocation");

    Ok(Some(Skill {
        name,
        description,
        name_zh,
        description_zh,
        version,
        location,
        disable_model_invocation: disable,
    }))
}

fn extract_name(content: &str, path: &Path) -> Result<String> {
    // Delegate to extract_frontmatter_field which handles block scalars (>, |).
    if let Some(name) = extract_frontmatter_field(content, "name") {
        return Ok(name);
    }
    // Fallback to filename
    Ok(path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string())
}

fn extract_yaml_value(line: &str, key: &str) -> Option<String> {
    let prefix = format!("{}:", key);
    if !line.starts_with(&prefix) && !line.trim_start().starts_with(&prefix) {
        return None;
    }

    // Find the colon
    let colon_idx = line.find(':')?;
    let value = line[colon_idx + 1..].trim();

    // Handle quoted values
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_string());
    }
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_string());
    }

    // Unquoted value
    Some(value.to_string())
}

fn extract_description(content: &str) -> String {
    extract_frontmatter_field(content, "description").unwrap_or_default()
}

fn extract_frontmatter_field(content: &str, key: &str) -> Option<String> {
    let frontmatter = frontmatter(content)?;
    let lines: Vec<&str> = frontmatter.lines().collect();

    for (index, line) in lines.iter().enumerate() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with('#') {
            continue;
        }

        if let Some(val) = extract_yaml_value(trimmed_line, key) {
            if let Some(style) = yaml_block_scalar_style(&val) {
                return Some(extract_yaml_block_scalar(&lines[index + 1..], line, style));
            }
            return Some(val);
        }
    }

    None
}

fn frontmatter(content: &str) -> Option<&str> {
    let trimmed = content.trim_start_matches(['\r', '\n']);
    if !trimmed.starts_with("---") {
        return None;
    }

    let rest = &trimmed[3..];
    let end_idx = rest.find("\n---").or_else(|| rest.find("---"))?;
    Some(&rest[..end_idx])
}

fn yaml_block_scalar_style(value: &str) -> Option<char> {
    let value = value.trim();
    let mut chars = value.chars();
    let style = chars.next()?;
    if !matches!(style, '>' | '|') {
        return None;
    }
    let modifiers = chars.as_str().trim();
    if modifiers
        .chars()
        .all(|ch| matches!(ch, '+' | '-') || ch.is_ascii_digit())
    {
        Some(style)
    } else {
        None
    }
}

fn extract_yaml_block_scalar(lines: &[&str], parent_line: &str, style: char) -> String {
    let parent_indent = leading_spaces(parent_line);
    let mut block_lines = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            block_lines.push(*line);
            continue;
        }
        if leading_spaces(line) <= parent_indent {
            break;
        }
        block_lines.push(*line);
    }

    let content_indent = block_lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| leading_spaces(line))
        .min()
        .unwrap_or(parent_indent + 1);

    let stripped: Vec<String> = block_lines
        .into_iter()
        .map(|line| strip_indent(line, content_indent).to_string())
        .collect();

    if style == '|' {
        return stripped.join("\n").trim().to_string();
    }

    fold_yaml_block_lines(&stripped).trim().to_string()
}

fn fold_yaml_block_lines(lines: &[String]) -> String {
    let mut result = String::new();
    for line in lines {
        if line.trim().is_empty() {
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push('\n');
            continue;
        }
        if !result.is_empty() && !result.ends_with('\n') {
            result.push(' ');
        }
        result.push_str(line.trim_end());
    }
    result
}

fn strip_indent(line: &str, indent: usize) -> &str {
    let mut byte_index = 0;
    for (count, (index, ch)) in line.char_indices().enumerate() {
        if count >= indent || ch != ' ' {
            byte_index = index;
            break;
        }
        byte_index = index + ch.len_utf8();
    }
    if indent == 0 {
        line
    } else if byte_index >= line.len() {
        ""
    } else {
        &line[byte_index..]
    }
}

fn leading_spaces(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == ' ').count()
}

/// ResolveCollisionsWithDiagnostics resolves skill name collisions.
pub fn resolve_collisions(skills: Vec<Skill>) -> Vec<Skill> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut result = vec![];

    for mut skill in skills {
        let count = seen.entry(skill.name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            skill.name = format!("{}_{}", skill.name, count);
        }
        result.push(skill);
    }
    result
}

use std::collections::HashMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_folded_description_from_frontmatter() {
        let content = r#"---
name: future-deep-research
version: 2.5.0
description: >
  Research across the web,
  papers, and local documents.
  Produce a cited report.
metadata:
  requires:
    bins: ["future"]
---

# Body
"#;

        assert_eq!(
            extract_frontmatter_field(content, "description").as_deref(),
            Some("Research across the web, papers, and local documents. Produce a cited report.")
        );
        assert_eq!(
            extract_frontmatter_field(content, "version").as_deref(),
            Some("2.5.0")
        );
    }

    #[test]
    fn extracts_literal_block_description_from_frontmatter() {
        let content = r#"---
name: docs
description: |
  First line.
  Second line.
version: "1.0.0"
---
"#;

        assert_eq!(
            extract_frontmatter_field(content, "description").as_deref(),
            Some("First line.\nSecond line.")
        );
    }
}
