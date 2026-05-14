//! Skills discovery — 1:1 compatible with Go internal/skills/

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub location: String,
    #[serde(rename = "disableModelInvocation", default)]
    pub disable_model_invocation: bool,
}

/// Predefined skill directories (matching Go internal/skills/skills.go)
pub const USER_SKILLS_DIR: &str = "~/.future/agent/skills/";
pub const PROJECT_SKILLS_DIR: &str = ".future/agent/skills/";
pub const AGENTS_SKILLS_DIR: &str = "~/.agents/skills/";

/// DiscoverSkills finds all skills in the given directories.
pub fn discover_skills(dirs: &[String]) -> Result<Vec<Skill>> {
    let mut skills = vec![];
    for dir in dirs {
        let expanded = shellexpand::tilde(dir);
        let path = Path::new(&*expanded);
        if !path.exists() {
            continue;
        }
        for entry in WalkDir::new(path).max_depth(2).into_iter().filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                let skill_md = entry_path.join("SKILL.md");
                if skill_md.exists() {
                    if let Some(skill) = parse_skill(&skill_md)? {
                        skills.push(skill);
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
    let location = skill_md.to_string_lossy().to_string();
    let disable = content.contains("disableModelInvocation") || content.contains("disable_model_invocation");

    Ok(Some(Skill {
        name,
        description,
        location,
        disable_model_invocation: disable,
    }))
}

fn extract_name(content: &str, path: &Path) -> Result<String> {
    // Parse YAML frontmatter between --- markers (matching Go parseFrontmatter)
    let trimmed = content.trim_start_matches(|c| c == '\r' || c == '\n');
    if !trimmed.starts_with("---") {
        // No frontmatter, use filename
        return Ok(path.file_stem().unwrap_or_default().to_string_lossy().to_string());
    }
    
    let rest = &trimmed[3..]; // skip opening ---
    let end_idx = rest.find("\n---").or_else(|| rest.find("---"));
    if end_idx.is_none() {
        return Ok(path.file_stem().unwrap_or_default().to_string_lossy().to_string());
    }
    
    let frontmatter = &rest[..end_idx.unwrap()];
    
    for line in frontmatter.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with('#') {
            continue;
        }
        
        // Match "name: value" pattern
        if let Some(val) = extract_yaml_value(trimmed_line, "name") {
            return Ok(val);
        }
    }
    
    // Fallback to filename
    Ok(path.file_stem().unwrap_or_default().to_string_lossy().to_string())
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
    // Parse YAML frontmatter to extract description (matching Go parseFrontmatter)
    let trimmed = content.trim_start_matches(|c| c == '\r' || c == '\n');
    if !trimmed.starts_with("---") {
        return String::new();
    }
    
    let rest = &trimmed[3..];
    let end_idx = rest.find("\n---").or_else(|| rest.find("---"));
    if end_idx.is_none() {
        return String::new();
    }
    
    let frontmatter = &rest[..end_idx.unwrap()];
    
    for line in frontmatter.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with('#') {
            continue;
        }
        
        if let Some(val) = extract_yaml_value(trimmed_line, "description") {
            return val;
        }
    }
    
    String::new()
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
