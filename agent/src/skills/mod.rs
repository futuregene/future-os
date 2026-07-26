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

/// Predefined skill directories (matching Go internal/skills/skills.go).
/// Both are global, user-level locations — project/cwd-relative skill
/// directories are intentionally not supported (see `global_skill_dirs`).
pub const APP_SKILLS_DIR: &str = "~/.future/agent/skills/";
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

/// The global (user-level) skill directories. Project/cwd-relative skill
/// dirs are intentionally NOT scanned: every caller must see the same skill
/// set regardless of the session's working directory, which also keeps the
/// `discover_skills_cached` cache key stable across call sites.
pub fn global_skill_dirs() -> Vec<String> {
    vec![APP_SKILLS_DIR.to_string(), AGENTS_SKILLS_DIR.to_string()]
}

// ─── Cached skills discovery ──────────────────────────────────────────────

/// How long the cached skills list stays fresh before a refresh is triggered.
const SKILLS_CACHE_TTL_SECS: u64 = 60;

/// Global cached skills list, refreshed lazily on access.
static SKILLS_CACHE: std::sync::RwLock<Option<(std::time::Instant, Vec<Skill>)>> =
    std::sync::RwLock::new(None);

/// Serialises cache refreshes so only one thread does the I/O work.
static REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Returns a cached skills list, refreshing when older than
/// SKILLS_CACHE_TTL_SECS. Fast path is lock-free for concurrent readers;
/// slow path serialises file I/O with a dedicated refresh mutex so multiple
/// concurrent prompts never block each other on the write lock.
///
/// All call sites are expected to pass `global_skill_dirs()`; the cache is
/// global and does NOT key on `dirs`, so mixing different dir lists across
/// call sites would return stale results for the minority list.
pub fn discover_skills_cached(dirs: &[String]) -> Vec<Skill> {
    // Fast path: read lock, check TTL — multiple readers OK.
    {
        let cache = SKILLS_CACHE.read().unwrap();
        if let Some((ref ts, ref skills)) = *cache {
            if ts.elapsed().as_secs() < SKILLS_CACHE_TTL_SECS {
                return skills.clone();
            }
        }
    }
    // Slow path: one thread refreshes; others wait on the refresh lock
    // then double-check (the first thread may have already refreshed).
    let _refresh = REFRESH_LOCK.lock().unwrap();
    {
        let cache = SKILLS_CACHE.read().unwrap();
        if let Some((ref ts, ref skills)) = *cache {
            if ts.elapsed().as_secs() < SKILLS_CACHE_TTL_SECS {
                return skills.clone();
            }
        }
    }
    // File I/O outside any lock — only one thread reaches here.
    let skills = discover_skills(dirs).unwrap_or_default();
    *SKILLS_CACHE.write().unwrap() = Some((std::time::Instant::now(), skills.clone()));
    skills
}

/// Invalidate the skills cache so the next access triggers a refresh.
pub fn invalidate_skills_cache() {
    *SKILLS_CACHE.write().unwrap() = None;
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

    #[test]
    fn extract_name_falls_back_to_filename() {
        let content = "---\nother: value\n---\n# Body";
        let path = std::path::Path::new("/path/to/my-skill/SKILL.md");
        assert_eq!(extract_name(content, path).unwrap(), "SKILL");
    }

    #[test]
    fn extract_yaml_value_quoted_strings() {
        assert_eq!(
            extract_yaml_value("name: \"quoted value\"", "name").as_deref(),
            Some("quoted value")
        );
        assert_eq!(
            extract_yaml_value("name: 'single quoted'", "name").as_deref(),
            Some("single quoted")
        );
    }

    #[test]
    fn extract_yaml_value_no_colon_returns_none() {
        assert!(extract_yaml_value("nocolon", "name").is_none());
    }

    #[test]
    fn frontmatter_with_newline_separator() {
        let content = "---\nname: skill\ndescription: desc\n---\n# Body";
        let fm = frontmatter(content).unwrap();
        assert!(fm.contains("name: skill"));
    }

    #[test]
    fn frontmatter_without_newline_separator() {
        let content = "---name: skill---";
        let fm = frontmatter(content).unwrap();
        assert_eq!(fm, "name: skill");
    }

    #[test]
    fn frontmatter_no_separator_returns_none() {
        assert!(frontmatter("no frontmatter here").is_none());
    }

    #[test]
    fn yaml_block_scalar_style_literal() {
        assert_eq!(yaml_block_scalar_style("|  "), Some('|'));
        assert_eq!(yaml_block_scalar_style(">  "), Some('>'));
        assert_eq!(yaml_block_scalar_style("|"), Some('|'));
        assert!(yaml_block_scalar_style("plain").is_none());
        assert!(yaml_block_scalar_style("").is_none());
    }

    #[test]
    fn yaml_block_scalar_with_modifiers() {
        assert_eq!(yaml_block_scalar_style("|2"), Some('|'));
        assert_eq!(yaml_block_scalar_style(">-"), Some('>'));
        assert_eq!(yaml_block_scalar_style("|+"), Some('|'));
        assert!(yaml_block_scalar_style("|x").is_none());
    }

    #[test]
    fn extract_yaml_block_scalar_literal() {
        // Lines must have MORE leading spaces than parent (parent has 2, lines have 4)
        let lines = ["    line one", "    line two", "  stop"];
        let result = extract_yaml_block_scalar(&lines, "  description: |", '|');
        assert_eq!(result, "line one\nline two");
    }

    #[test]
    fn extract_yaml_block_scalar_folded() {
        // Lines with more indent than parent
        let lines = [
            "    line one",
            "    line two",
            "    ",
            "    line three",
            "  stop",
        ];
        let result = extract_yaml_block_scalar(&lines, "  description: >", '>');
        assert_eq!(result, "line one line two\n\nline three");
    }

    #[test]
    fn fold_yaml_block_lines_joins_consecutive() {
        let lines: Vec<String> = vec![
            "first line".to_string(),
            "continued".to_string(),
            "".to_string(),
            "new paragraph".to_string(),
        ];
        let result = fold_yaml_block_lines(&lines);
        assert!(result.contains("first line continued"));
        assert!(result.contains("\n\nnew paragraph"));
    }

    #[test]
    fn strip_indent_basic() {
        assert_eq!(strip_indent("    indented", 4), "indented");
        assert_eq!(strip_indent("noindent", 0), "noindent");
        assert_eq!(strip_indent("  ", 2), "");
    }

    #[test]
    fn leading_spaces_counts_correctly() {
        assert_eq!(leading_spaces("  two"), 2);
        assert_eq!(leading_spaces("zero"), 0);
    }

    #[test]
    fn resolve_collisions_renames_duplicates() {
        let skills = vec![
            Skill {
                name: "my-skill".to_string(),
                description: "first".to_string(),
                name_zh: None,
                description_zh: None,
                version: Some("1.0".to_string()),
                location: "/a".to_string(),
                disable_model_invocation: false,
            },
            Skill {
                name: "my-skill".to_string(),
                description: "second".to_string(),
                name_zh: None,
                description_zh: None,
                version: Some("2.0".to_string()),
                location: "/b".to_string(),
                disable_model_invocation: false,
            },
            Skill {
                name: "other".to_string(),
                description: "unique".to_string(),
                name_zh: None,
                description_zh: None,
                version: None,
                location: "/c".to_string(),
                disable_model_invocation: false,
            },
        ];
        let resolved = resolve_collisions(skills);
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].name, "my-skill");
        assert_eq!(resolved[1].name, "my-skill_2");
        assert_eq!(resolved[2].name, "other");
    }

    #[test]
    fn parse_skill_reads_file() {
        let dir = std::env::temp_dir().join(format!(
            "future_skill_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let skill_md = dir.join("SKILL.md");
        let content = r#"---
name: test-skill
description: A test skill for unit tests
version: "1.0.0"
disable_model_invocation: true
---

# Test Skill
This is a test skill body.
"#;
        std::fs::write(&skill_md, content).unwrap();

        let skill = parse_skill(&skill_md).unwrap().unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.description, "A test skill for unit tests");
        assert_eq!(skill.version.as_deref(), Some("1.0.0"));
        assert!(skill.disable_model_invocation);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_skills_finds_and_parses() {
        let dir = std::env::temp_dir().join(format!(
            "future_skills_discover_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skills_subdir = dir.join("test-skill");
        std::fs::create_dir_all(&skills_subdir).unwrap();
        std::fs::write(
            skills_subdir.join("SKILL.md"),
            "---\nname: discovered-skill\ndescription: Found via discovery\n---\n# Body\n",
        )
        .unwrap();

        let discovered = discover_skills(&[dir.to_string_lossy().to_string()]).unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "discovered-skill");
        assert_eq!(discovered[0].description, "Found via discovery");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discover_skills_empty_nonexistent_dir() {
        let discovered = discover_skills(&["/no/such/dir/skills".to_string()]).unwrap();
        assert!(discovered.is_empty());
    }
}
