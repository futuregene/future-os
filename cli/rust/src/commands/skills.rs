//! `future skills` — port of cli/src/commands/skills.ts.
//!
//! P1 status: the command entry (`skills`) stays a stub until P2, but the
//! helpers the P1 commands depend on are fully ported: `fetchSkills`,
//! `installBuiltinSkills` (used by `future init`), `getInstalledSkillIds`,
//! `readSkillMdVersion` (used by `future doctor`), plus the internal
//! download/unzip/flatten machinery.

use crate::output::Output;
use crate::utils::platform::get_platform_url;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// `SKILLS_DIR` from skills.ts — `~/.future/agent/skills`.
pub fn skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".future")
        .join("agent")
        .join("skills")
}

/// `SkillInfo` from skills.ts.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub price: String,
    #[serde(default)]
    pub formats: String,
    #[serde(default)]
    pub limit: String,
    #[serde(default)]
    pub latest_version: Option<String>,
}

/// `isSkillsCommand(command)` — type-guard port; `undefined` is not a command.
pub fn is_skills_command(command: Option<&str>) -> bool {
    matches!(
        command,
        Some("list" | "install" | "uninstall" | "install-builtin" | "update")
    )
}

/// `skills(command, args)` (P2 — command bodies ported then).
pub async fn skills(command: &str, _args: &[String], _out: &Output) -> Result<(), String> {
    Err(not_implemented(&format!("skills {command}")))
}

fn not_implemented(what: &str) -> String {
    format!("`future {what}` is not implemented yet in the Rust CLI (P2)")
}

// ── Remote API ─────────────────────────────────────────────────────────────

/// `fetchSkills(platformUrl)` — GET {platform}/client/v1/skills.
pub async fn fetch_skills(platform_url: &str) -> Result<Vec<SkillInfo>, String> {
    let url = format!("{platform_url}/client/v1/skills");
    let response = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch skills: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch skills: {} {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("")
        ));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    // `return body.skills ?? [];` — also normalise empty latest_version to
    // None (JS treats "" as falsy throughout skills.ts).
    Ok(body
        .get("skills")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| serde_json::from_value::<SkillInfo>(s.clone()).ok())
                .map(|mut skill| {
                    if skill.latest_version.as_deref() == Some("") {
                        skill.latest_version = None;
                    }
                    skill
                })
                .collect()
        })
        .unwrap_or_default())
}

/// `downloadSkillZip(platformUrl, skillId, version)` — download to a temp zip.
async fn download_skill_zip(
    platform_url: &str,
    skill_id: &str,
    version: &str,
) -> Result<Vec<u8>, String> {
    let url = format!(
        "{platform_url}/client/v1/skills/{}/versions/{}/download",
        urlencode(skill_id),
        urlencode(version)
    );
    let response = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    if !response.status().is_success() {
        if response.status().as_u16() == 404 {
            return Err(format!("Skill version \"{skill_id}@{version}\" not found."));
        }
        return Err(format!(
            "Failed to download skill: {} {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("")
        ));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("Empty response body".to_string());
    }
    Ok(bytes.to_vec())
}

fn urlencode(s: &str) -> String {
    // `encodeURIComponent` — encode everything except unreserved chars.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── installBuiltinSkills (used by `future init`) ───────────────────────────

/// `installBuiltinSkills()` — install every catalog skill whose id starts
/// with `future-`. On catalog failure it reports to stderr, sets
/// `process.exitCode = 1` (via [`Output::set_exit_code`]) and returns
/// normally — exactly like the TS, which sets `process.exitCode` and lets
/// `init` continue linking command links.
pub async fn install_builtin_skills(out: &Output) {
    let platform_url = get_platform_url(None).await;
    let skills: Vec<SkillInfo> = match fetch_skills(&platform_url).await {
        Ok(skills) => skills
            .into_iter()
            .filter(|s| s.id.starts_with("future-"))
            .collect(),
        Err(err) => {
            out.log_err("Failed to fetch builtin skills.");
            out.log_err(&err);
            out.set_exit_code(1);
            return;
        }
    };

    if skills.is_empty() {
        out.log("No builtin skills available.");
        return;
    }

    let installed = get_installed_skill_ids().await;
    let to_install: Vec<&SkillInfo> = skills
        .iter()
        .filter(|s| !installed.contains(&s.id))
        .collect();

    if to_install.is_empty() {
        out.log(&format!(
            "All {} builtin skills are already installed.",
            skills.len()
        ));
        return;
    }

    let skipped = skills.len() - to_install.len();
    out.log(&format!(
        "Installing {} builtin skills{}...",
        to_install.len(),
        if skipped > 0 {
            format!(" ({skipped} already installed)")
        } else {
            String::new()
        }
    ));

    for skill in &to_install {
        let Some(version) = &skill.latest_version else {
            out.log(&format!("  Skipping {} — no version available.", skill.id));
            continue;
        };
        if let Err(err) = install_skill(&skill.id, version, out).await {
            out.log_err(&format!("  Failed to install {}: {err}", skill.id));
        }
    }

    out.log(&format!("Done. {} skills installed.", to_install.len()));
}

/// `installSkill(skillId, version)` — download, unzip, flatten, print result.
///
/// Download failures print the raw error, set `process.exitCode = 1` and
/// return normally (TS behavior); write/unzip/flatten failures throw and are
/// reported by the caller with the `  Failed to install …` prefix.
async fn install_skill(skill_id: &str, version: &str, out: &Output) -> Result<(), String> {
    let platform_url = get_platform_url(None).await;
    let dest = skills_dir().join(skill_id);
    let is_update = tokio::fs::metadata(&dest).await.is_ok();

    out.log(&format!("Downloading {skill_id} v{version}..."));
    let tmp_zip = std::env::temp_dir().join(format!("future-skill-{skill_id}-{version}.zip"));
    let zip_bytes = match download_skill_zip(&platform_url, skill_id, version).await {
        Ok(bytes) => bytes,
        Err(err) => {
            out.log_err(&err);
            out.set_exit_code(1);
            return Ok(());
        }
    };
    let write_result = tokio::fs::write(&tmp_zip, zip_bytes).await;
    let result: Result<(), String> = async {
        write_result.map_err(|e| e.to_string())?;
        if is_update {
            tokio::fs::remove_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::create_dir_all(&dest)
            .await
            .map_err(|e| e.to_string())?;
        unzip(&tmp_zip, &dest).await?;
        flatten_single_subdir(&dest).await?;
        Ok(())
    }
    .await;
    // `finally { rm(tmpZip, { force: true }) }`
    let _ = tokio::fs::remove_file(&tmp_zip).await;
    result?;

    out.log(&format!(
        "{} skill \"{skill_id}\" v{version} → {}",
        if is_update { "Updated" } else { "Installed" },
        dest.display()
    ));
    Ok(())
}

// ── Installed-skill helpers (used by `future doctor`) ──────────────────────

/// `getInstalledSkillIds()` — ids of skill dirs containing a SKILL.md.
pub async fn get_installed_skill_ids() -> HashSet<String> {
    let mut ids = HashSet::new();
    let Ok(entries) = tokio::fs::read_dir(skills_dir()).await else {
        return ids;
    };
    let mut entries = entries;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if tokio::fs::metadata(skills_dir().join(&name).join("SKILL.md"))
            .await
            .is_ok()
        {
            ids.insert(name);
        }
    }
    ids
}

/// `readSkillMdVersion(skillMdPath)` — YAML frontmatter `version` field.
pub async fn read_skill_md_version(skill_md_path: &Path) -> Option<String> {
    let text = tokio::fs::read_to_string(skill_md_path).await.ok()?;
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = &trimmed[3..];
    // `Math.max(rest.indexOf("\\n---"), rest.indexOf("---"))` — Option::max
    // mirrors Math.max over -1-when-missing (None).
    let end_idx = rest
        .find("\n---")
        .max(rest.find("---"))
        .unwrap_or(usize::MAX);
    if end_idx == usize::MAX {
        return None;
    }
    let frontmatter = &rest[..end_idx];
    let lines: Vec<&str> = frontmatter.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // `^version:\s*(.+)$`
        if let Some(v) = t.strip_prefix("version:") {
            return unquote(v.trim());
        }
        // `^metadata:\s*(.*)$`
        if let Some(meta_rest) = t.strip_prefix("metadata:") {
            let meta_rest = meta_rest.trim();
            if !meta_rest.is_empty() {
                // JSON first: `if (meta.version) return String(meta.version)`.
                if let Ok(meta) = serde_json::from_str::<Value>(meta_rest) {
                    if let Some(v) = meta.get("version") {
                        let as_string = v
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| v.as_f64().map(|n| format!("{n}")));
                        if let Some(version) = as_string {
                            if !version.is_empty() {
                                return Some(version);
                            }
                        }
                    }
                } else if let Some(v) = meta_rest.strip_prefix("version:") {
                    return unquote(v.trim());
                }
            }
            // YAML block: scan indented lines for `version:`.
            for sub in lines.iter().skip(i + 1) {
                let sub_trimmed = sub.trim();
                if sub_trimmed.starts_with('#') {
                    continue;
                }
                if !sub.starts_with(' ') && !sub.starts_with('\t') {
                    break;
                }
                if let Some(v) = sub_trimmed.strip_prefix("version:") {
                    return unquote(v.trim());
                }
            }
        }
    }
    None
}

/// `unquote(val)` — strip matching single/double quotes; empty results are
/// `None` (JS `unquote` returns `val || ""`, and empty is falsy everywhere
/// the version is consumed).
fn unquote(val: &str) -> Option<String> {
    let stripped = if (val.starts_with('"') && val.ends_with('"'))
        || (val.starts_with('\'') && val.ends_with('\''))
    {
        &val[1..val.len().saturating_sub(1)]
    } else {
        val
    };
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

// ── unzip / flatten ────────────────────────────────────────────────────────

/// `unzip(zipPath, destDir)` — system `unzip` (unix) or PowerShell
/// Expand-Archive (Windows).
async fn unzip(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    if cfg!(windows) {
        let output = tokio::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    zip_path.display(),
                    dest_dir.display()
                ),
            ])
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "unzip failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    } else {
        let zip = zip_path.display().to_string();
        let dest = dest_dir.display().to_string();
        let output = tokio::process::Command::new("unzip")
            .args(["-o", zip.as_str(), "-d", dest.as_str()])
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "unzip failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
}

/// `flattenSingleSubdir(dir)` — if the dir contains exactly one subdirectory
/// and nothing else, move its contents up one level.
async fn flatten_single_subdir(dir: &Path) -> Result<(), String> {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return Ok(());
    };
    let mut names = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        names.push(entry.file_name());
    }
    if names.len() != 1 {
        return Ok(());
    }
    let single = dir.join(&names[0]);
    let Ok(meta) = tokio::fs::metadata(&single).await else {
        return Ok(());
    };
    if !meta.is_dir() {
        return Ok(());
    }
    let mut children = tokio::fs::read_dir(&single)
        .await
        .map_err(|e| e.to_string())?;
    let mut child_names = Vec::new();
    while let Ok(Some(entry)) = children.next_entry().await {
        child_names.push(entry.file_name());
    }
    for child in child_names {
        let src = single.join(&child);
        let dest = dir.join(&child);
        let _ = tokio::fs::remove_dir_all(&dest).await;
        let _ = tokio::fs::remove_file(&dest).await;
        rename_across_device(&src, &dest).await?;
    }
    let _ = tokio::fs::remove_dir_all(&single).await;
    Ok(())
}

/// `renameAcrossDevice(src, dest)` — rename with copy+delete fallback.
async fn rename_across_device(src: &Path, dest: &Path) -> Result<(), String> {
    match tokio::fs::rename(src, dest).await {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_recursive(src, dest).await?;
            let _ = tokio::fs::remove_dir_all(src).await;
            let _ = tokio::fs::remove_file(src).await;
            Ok(())
        }
    }
}

/// Recursive copy (tokio has no built-in).
async fn copy_recursive(src: &Path, dest: &Path) -> Result<(), String> {
    let meta = tokio::fs::metadata(src).await.map_err(|e| e.to_string())?;
    if meta.is_dir() {
        tokio::fs::create_dir_all(dest)
            .await
            .map_err(|e| e.to_string())?;
        let mut entries = tokio::fs::read_dir(src).await.map_err(|e| e.to_string())?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            Box::pin(copy_recursive(&entry.path(), &dest.join(entry.file_name()))).await?;
        }
    } else {
        tokio::fs::copy(src, dest)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_skill_md_version_direct_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        tokio::fs::write(&path, "---\nversion: 1.2.3\n---\n# Hi\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await.as_deref(), Some("1.2.3"));
    }

    #[tokio::test]
    async fn read_skill_md_version_metadata_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        tokio::fs::write(&path, "---\nmetadata: {\"version\": \"2.0\"}\n---\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await.as_deref(), Some("2.0"));
    }

    #[tokio::test]
    async fn read_skill_md_version_metadata_yaml_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        tokio::fs::write(
            &path,
            "---\nmetadata:\n  version: \"3.1\"\n  author: x\n---\n",
        )
        .await
        .unwrap();
        assert_eq!(read_skill_md_version(&path).await.as_deref(), Some("3.1"));
    }

    #[tokio::test]
    async fn read_skill_md_version_no_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        tokio::fs::write(&path, "# no frontmatter\n").await.unwrap();
        assert_eq!(read_skill_md_version(&path).await, None);
    }

    #[tokio::test]
    async fn read_skill_md_version_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_skill_md_version(&dir.path().join("nope.md")).await,
            None
        );
    }

    #[tokio::test]
    async fn urlencode_behavior() {
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("v1.0"), "v1.0");
    }
}
