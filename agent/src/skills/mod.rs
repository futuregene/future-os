//! Skills discovery — 1:1 compatible with Go internal/skills/

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub location: String,
    #[serde(rename = "disableModelInvocation", default)]
    pub disable_model_invocation: bool,
}

pub const USER_SKILLS_DIR: &str = "~/.openclaw/skills";
pub const PROJECT_SKILLS_DIR: &str = ".openclaw/skills";
pub const AGENTS_SKILLS_DIR: &str = "~/.agents/skills";

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
    // First non-empty line that looks like a name
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Ok(trimmed.to_string());
    }
    Ok(path.file_stem().unwrap_or_default().to_string_lossy().to_string())
}

fn extract_description(content: &str) -> String {
    let mut found_first_line = false;
    let mut lines = vec![];
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            if found_first_line {
                break; // Stop at second heading
            }
            found_first_line = true;
            continue;
        }
        if found_first_line && lines.len() < 3 {
            lines.push(trimmed.to_string());
        }
    }
    lines.join(" ").trim().to_string()
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
