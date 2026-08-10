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

/// `skills(command, args)` — port of the skills.ts command body (P2).
pub async fn skills(command: &str, args: &[String], out: &Output) -> Result<(), String> {
    match command {
        "list" => {
            list_skills(out).await;
        }
        "install-builtin" => {
            install_builtin_skills(out).await;
            // One notification per command, not per skill.
            crate::rpc::notify_agent_refresh_skills().await;
        }
        "update" => {
            update_skills(out).await?;
            crate::rpc::notify_agent_refresh_skills().await;
        }
        "install" => {
            let name = args.first().map(String::as_str);
            let Some(name) = name else {
                // No name given — install all builtin skills.
                install_builtin_skills(out).await;
                crate::rpc::notify_agent_refresh_skills().await;
                return Ok(());
            };
            let version_idx = args.iter().position(|a| a == "--version");
            let mut version = version_idx.and_then(|i| args.get(i + 1)).cloned();
            // Strip leading "v" if the user provided it (e.g. "v1.0" → "1.0")
            // to avoid a double "v" in output.
            if let Some(v) = &version {
                if let Some(stripped) = v.strip_prefix('v') {
                    version = Some(stripped.to_string());
                }
            }
            install_skill(name, version.as_deref(), out).await?;
            crate::rpc::notify_agent_refresh_skills().await;
        }
        "uninstall" => {
            let Some(name) = args.first().map(String::as_str) else {
                out.log_err(&format!("Usage: future skills {command} <skill-name>"));
                out.set_exit_code(1);
                return Ok(());
            };
            uninstall_skill(name, out).await?;
            crate::rpc::notify_agent_refresh_skills().await;
        }
        // The dispatch guards via is_skills_command; a direct caller with an
        // unknown subcommand gets an error rather than a panic.
        other => return Err(format!("Unknown skills command: {other}")),
    }
    Ok(())
}

// ── list / update / uninstall (P2 command bodies) ──────────────────────────

/// `listSkills()` — catalog table with installed versions.
async fn list_skills(out: &Output) {
    let platform_url = get_platform_url(None).await;

    let skills: Vec<SkillInfo> = match fetch_skills(&platform_url).await {
        Ok(skills) => skills,
        Err(err) => {
            out.log_err(&format!(
                "Failed to fetch skills from {platform_url}/client/v1/skills"
            ));
            out.log_err(&err);
            out.set_exit_code(1);
            return;
        }
    };

    if skills.is_empty() {
        out.log("No skills available.");
        return;
    }

    // Check which skills are installed.
    let mut installed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Ok(mut entries) = tokio::fs::read_dir(skills_dir()).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(ver) =
                read_skill_md_version(&skills_dir().join(&name).join("SKILL.md")).await
            {
                installed.insert(name, ver);
            }
        }
    }

    let id_width = skills
        .iter()
        .map(|s| s.id.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(12, 36);
    let ver_width = skills
        .iter()
        .map(|s| {
            let v = match &s.latest_version {
                Some(v) => format!("v{v}"),
                None => "—".to_string(),
            };
            v.chars().count()
        })
        .max()
        .unwrap_or(0)
        .max(10);
    let inst_width = skills
        .iter()
        .map(|s| {
            let marker = match installed.get(&s.id) {
                Some(v) => format!("v{v}"),
                None => "—".to_string(),
            };
            marker.chars().count()
        })
        .max()
        .unwrap_or(0)
        .max(9);
    const DESC_MAX: usize = 48;
    let desc_width = skills
        .iter()
        .map(|s| s.description.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(12, DESC_MAX);

    out.log(&format!(
        "  {} {} {} DESCRIPTION",
        pad("NAME", id_width),
        pad("LATEST", ver_width),
        pad("INSTALLED", inst_width)
    ));
    out.log(&format!(
        "  {} {} {} {}",
        "—".repeat(id_width),
        "—".repeat(ver_width),
        "—".repeat(inst_width),
        "—".repeat(desc_width)
    ));

    for s in &skills {
        let marker = match installed.get(&s.id) {
            Some(v) => format!("v{v}"),
            None => "—".to_string(),
        };
        let ver = match &s.latest_version {
            Some(v) => format!("v{v}"),
            None => "—".to_string(),
        };
        let desc: String = if s.description.chars().count() > DESC_MAX {
            let mut d: String = s.description.chars().take(DESC_MAX - 1).collect();
            d.push('…');
            d
        } else {
            s.description.clone()
        };
        out.log(&format!(
            "  {} {} {} {}",
            pad(&s.id, id_width),
            pad(&ver, ver_width),
            pad(&marker, inst_width),
            pad(&desc, desc_width)
        ));
    }
    out.log(&format!(
        "\n{} skills available. Use \"future skills install <name>\" to install.",
        skills.len()
    ));
}

/// `padEnd(s, width)` — JS padEnd (pad only when shorter; count chars).
fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

/// `updateSkills()` — upgrade all installed skills to their latest versions.
/// A catalog fetch failure propagates (the dispatch `catch` prints it and
/// exits 1), matching the TS `fetchSkills` throw path.
async fn update_skills(out: &Output) -> Result<(), String> {
    let platform_url = get_platform_url(None).await;
    out.log(&format!("Fetching skill catalog from {platform_url}..."));
    let skills: Vec<SkillInfo> = fetch_skills(&platform_url).await?;
    if skills.is_empty() {
        out.log("No skills available.");
        return Ok(());
    }

    let installed = get_installed_skill_ids().await;
    if installed.is_empty() {
        out.log("No skills installed.");
        return Ok(());
    }

    let mut updated = 0usize;
    let mut up_to_date = 0usize;

    for skill in &skills {
        if !installed.contains(&skill.id) {
            continue;
        }
        let Some(latest) = &skill.latest_version else {
            continue;
        };
        let skill_md_path = skills_dir().join(&skill.id).join("SKILL.md");
        let local_ver = read_skill_md_version(&skill_md_path).await;
        if local_ver.is_none() || local_ver.as_deref() == Some(latest.as_str()) {
            up_to_date += 1;
            continue;
        }

        out.log(&format!(
            "  {}: {} → {}",
            skill.id,
            local_ver.unwrap_or_default(),
            latest
        ));
        match install_skill(&skill.id, Some(latest.as_str()), out).await {
            Ok(()) => updated += 1,
            Err(err) => out.log_err(&format!("  Failed: {err}")),
        }
    }

    if updated == 0 {
        out.log(&format!("{up_to_date} skill(s) already up to date."));
    } else {
        out.log(&format!(
            "Updated {updated} skill(s), {up_to_date} already up to date."
        ));
    }
    Ok(())
}

/// `uninstallSkill(skillId)` — remove an installed skill.
async fn uninstall_skill(skill_id: &str, out: &Output) -> Result<(), String> {
    let dest = skills_dir().join(skill_id);
    if tokio::fs::metadata(&dest).await.is_err() {
        out.log(&format!("Skill \"{skill_id}\" is not installed."));
        return Ok(());
    }
    tokio::fs::remove_dir_all(&dest)
        .await
        .map_err(|e| e.to_string())?;
    out.log(&format!(
        "Uninstalled skill \"{skill_id}\" from {}.",
        skills_dir().display()
    ));
    Ok(())
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
        if let Err(err) = install_skill(&skill.id, Some(version.as_str()), out).await {
            out.log_err(&format!("  Failed to install {}: {err}", skill.id));
        }
    }

    out.log(&format!("Done. {} skills installed.", to_install.len()));
}

/// `installSkill(skillId, version?)` — download, unzip, flatten, print result.
///
/// With no version the latest is looked up from the catalog; metadata and
/// download failures print the raw error, set `process.exitCode = 1` and
/// return normally (TS behavior); write/unzip/flatten failures throw and are
/// reported by the caller with the `  Failed to install …` prefix.
async fn install_skill(skill_id: &str, version: Option<&str>, out: &Output) -> Result<(), String> {
    let platform_url = get_platform_url(None).await;
    let version = match version {
        Some(v) => v.to_string(),
        None => {
            // `installSkill(skillId)` without a version: look up the latest
            // from the catalog. Failures print + set exitCode 1 and return
            // normally (TS behavior).
            let skills = match fetch_skills(&platform_url).await {
                Ok(skills) => skills,
                Err(err) => {
                    out.log_err("Failed to fetch skill metadata.");
                    out.log_err(&err);
                    out.set_exit_code(1);
                    return Ok(());
                }
            };
            let Some(skill_meta) = skills.iter().find(|s| s.id == skill_id) else {
                out.log_err(&format!("Skill \"{skill_id}\" not found in catalog."));
                out.log_err("Run \"future skills list\" to see available skills.");
                out.set_exit_code(1);
                return Ok(());
            };
            let Some(latest) = &skill_meta.latest_version else {
                out.log_err(&format!("Skill \"{skill_id}\" has no versions available."));
                out.set_exit_code(1);
                return Ok(());
            };
            latest.clone()
        }
    };
    let dest = skills_dir().join(skill_id);
    let is_update = tokio::fs::metadata(&dest).await.is_ok();

    out.log(&format!("Downloading {skill_id} v{version}..."));
    let tmp_zip = std::env::temp_dir().join(format!("future-skill-{skill_id}-{version}.zip"));
    let zip_bytes = match download_skill_zip(&platform_url, skill_id, &version).await {
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

/// The platform unzip command. `#[cfg]` (not `cfg!`) so the off-platform
/// branch is never compiled into this target.
#[cfg(windows)]
fn unzip_command(zip_path: &Path, dest_dir: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-Command",
        &format!(
            "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
            zip_path.display(),
            dest_dir.display()
        ),
    ]);
    cmd
}

/// Unix unzip: `unzip -o <zip> -d <dest>`.
#[cfg(not(windows))]
fn unzip_command(zip_path: &Path, dest_dir: &Path) -> tokio::process::Command {
    let zip = zip_path.display().to_string();
    let dest = dest_dir.display().to_string();
    let mut cmd = tokio::process::Command::new("unzip");
    cmd.args(["-o", zip.as_str(), "-d", dest.as_str()]);
    cmd
}

/// `unzip(zipPath, destDir)` — system `unzip` (unix) or PowerShell
/// Expand-Archive (Windows).
async fn unzip(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let output = unzip_command(zip_path, dest_dir)
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

    // ── ZIP crafting (stored entries, no compression) ───────────────

    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        let mut crc = !0u32;
        for &b in data {
            crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        !crc
    }

    /// Minimal stored ZIP archive understood by the system `unzip`.
    fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        for (name, body) in entries {
            let crc = crc32(body.as_bytes());
            let offset = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(body.as_bytes());

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn crc32_known_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    // ── Shared helpers ──────────────────────────────────────────────

    /// Write auth.json pointing the platform URL at `base`.
    async fn point_platform_at(base: &str) {
        let path = crate::constants::auth_file();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            format!("{{\"future\": {{\"base_url\": \"{base}\"}}}}"),
        )
        .await
        .unwrap();
    }

    /// Catalog JSON body for the given (id, latest_version, description).
    fn catalog(rows: &[(&str, Option<&str>, &str)]) -> String {
        let skills: Vec<String> = rows
            .iter()
            .map(|(id, ver, desc)| match ver {
                Some(v) => format!(
                    "{{\"id\":\"{id}\",\"name\":\"{id}\",\"description\":\"{desc}\",\"latest_version\":\"{v}\"}}"
                ),
                None => format!(
                    "{{\"id\":\"{id}\",\"name\":\"{id}\",\"description\":\"{desc}\",\"latest_version\":null}}"
                ),
            })
            .collect();
        format!("{{\"skills\":[{}]}}", skills.join(","))
    }

    /// Install a local skill dir with a SKILL.md version under temp HOME.
    async fn plant_skill(id: &str, version: &str) {
        let dir = skills_dir().join(id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join("SKILL.md"),
            format!("---\nversion: {version}\n---\n"),
        )
        .await
        .unwrap();
    }

    // ── fetch_skills ────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_skills_success_and_normalization() {
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            "{\"skills\":[\
                {\"id\":\"a\",\"description\":\"d\",\"latest_version\":\"\"},\
                {\"id\":\"b\",\"latest_version\":\"1.0\"},\
                {\"malformed\": true}\
            ]}",
        )])
        .await;
        let skills = fetch_skills(&base).await.expect("fetch");
        // Every row parses (SkillInfo fields are all serde-default); the
        // empty-latest_version normalization is the observable behavior.
        assert_eq!(skills.len(), 3);
        assert_eq!(skills[0].id, "a");
        assert_eq!(skills[0].latest_version, None);
        assert_eq!(skills[1].latest_version.as_deref(), Some("1.0"));
        assert_eq!(skills[2].id, "");
    }

    #[tokio::test]
    async fn fetch_skills_error_variants() {
        // HTTP error status.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            500,
            "{}",
        )])
        .await;
        let err = fetch_skills(&base).await.unwrap_err();
        assert!(err.contains("Failed to fetch skills: 500"), "err: {err}");
        // Non-JSON body.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            "not json",
        )])
        .await;
        assert!(fetch_skills(&base).await.is_err());
        // Connection refused.
        let err = fetch_skills("http://127.0.0.1:1").await.unwrap_err();
        assert!(err.contains("Failed to fetch skills:"), "err: {err}");
        // Missing skills key → empty.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            "{}",
        )])
        .await;
        assert!(fetch_skills(&base).await.unwrap().is_empty());
    }

    // ── list ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_renders_catalog_with_installed_markers() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let long_desc = "d".repeat(60);
        let long_id = "future-skill-with-a-very-long-identifier-exceeding-36-chars";
        let body = catalog(&[
            ("future-alpha", Some("1.0"), "Alpha skill"),
            ("future-beta", None, &long_desc),
            (long_id, Some("2.0"), "Long id"),
        ]);
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &body,
        )])
        .await;
        point_platform_at(&base).await;
        plant_skill("future-alpha", "1.0").await;
        // A dir without SKILL.md is ignored by the installed scan.
        tokio::fs::create_dir_all(skills_dir().join("stray"))
            .await
            .unwrap();

        let (out, cap) = Output::memory();
        list_skills(&out).await;
        assert_eq!(out.exit_code(), 0);
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("NAME"), "stdout: {stdout}");
        assert!(stdout.contains("LATEST"), "stdout: {stdout}");
        assert!(stdout.contains("INSTALLED"), "stdout: {stdout}");
        assert!(stdout.contains("DESCRIPTION"), "stdout: {stdout}");
        // future-alpha: installed v1.0; future-beta: no version → "—".
        assert!(stdout.contains("future-alpha"), "stdout: {stdout}");
        assert!(stdout.contains("v1.0"), "stdout: {stdout}");
        // Long description truncated to 48 chars with an ellipsis.
        assert!(
            stdout.contains(&format!("{}…", "d".repeat(47))),
            "stdout: {stdout}"
        );
        // Long id rendered untruncated (width clamp caps at 36 but pad never truncates).
        assert!(stdout.contains(long_id), "stdout: {stdout}");
        assert!(stdout.contains("3 skills available."), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn list_empty_catalog_and_fetch_failure() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Empty catalog.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            "{\"skills\":[]}",
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        list_skills(&out).await;
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "No skills available.\n");

        // Fetch failure → stderr + exit code 1 (no Err — TS returns normally).
        point_platform_at("http://127.0.0.1:1").await;
        let (out, cap) = Output::memory();
        list_skills(&out).await;
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Failed to fetch skills from http://127.0.0.1:1/client/v1/skills"),
            "stderr: {stderr}"
        );
    }

    // ── install / download / unzip ──────────────────────────────────

    #[tokio::test]
    async fn install_skill_explicit_version_happy_path() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let zip = make_zip(&[
            ("future-x/", ""),
            ("future-x/SKILL.md", "---\nversion: 1.0\n---\n"),
        ]);
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::binary(
            "/client/v1/skills/future-x/versions/1.0/download",
            200,
            zip.clone(),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        install_skill("future-x", Some("1.0"), &out)
            .await
            .expect("install");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Downloading future-x v1.0..."),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("Installed skill \"future-x\" v1.0 →"),
            "stdout: {stdout}"
        );
        // Flattened: SKILL.md at the skill root (not nested under future-x/).
        let installed = skills_dir().join("future-x").join("SKILL.md");
        assert!(installed.exists());
        let content = tokio::fs::read_to_string(&installed).await.unwrap();
        assert!(content.contains("version: 1.0"));
    }

    #[tokio::test]
    async fn install_skill_update_replaces_existing() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        plant_skill("future-x", "0.9").await;
        let zip = make_zip(&[("SKILL.md", "---\nversion: 1.0\n---\n")]);
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::binary(
            "/client/v1/skills/future-x/versions/1.0/download",
            200,
            zip.clone(),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        install_skill("future-x", Some("1.0"), &out)
            .await
            .expect("update");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Updated skill \"future-x\" v1.0"),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn install_skill_catalog_lookup_paths() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Catalog fetch failure.
        point_platform_at("http://127.0.0.1:1").await;
        let (out, cap) = Output::memory();
        install_skill("future-x", None, &out).await.expect("ok");
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Failed to fetch skill metadata."),
            "stderr: {stderr}"
        );

        // Skill not in catalog.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &catalog(&[("future-y", Some("1.0"), "y")]),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        install_skill("future-x", None, &out).await.expect("ok");
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Skill \"future-x\" not found in catalog."),
            "stderr: {stderr}"
        );

        // In catalog but no versions.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &catalog(&[("future-x", None, "x")]),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        install_skill("future-x", None, &out).await.expect("ok");
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Skill \"future-x\" has no versions available."),
            "stderr: {stderr}"
        );

        // Found → installs the latest version from the catalog.
        let zip = make_zip(&[("SKILL.md", "---\nversion: 2.0\n---\n")]);
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/skills",
                200,
                &catalog(&[("future-x", Some("2.0"), "x")]),
            ),
            crate::test_server::HttpRoute::binary(
                "/client/v1/skills/future-x/versions/2.0/download",
                200,
                zip.clone(),
            ),
        ])
        .await;
        point_platform_at(&base).await;
        let (out, _) = Output::memory();
        install_skill("future-x", None, &out).await.expect("ok");
        assert_eq!(out.exit_code(), 0);
        assert!(skills_dir().join("future-x").join("SKILL.md").exists());
    }

    #[tokio::test]
    async fn download_skill_zip_error_variants() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // 404 → dedicated message.
        let base = crate::test_server::spawn_http(vec![]).await; // no routes → 404
        let err = download_skill_zip(&base, "future-x", "1.0")
            .await
            .unwrap_err();
        assert_eq!(err, "Skill version \"future-x@1.0\" not found.");
        // 500 → status message.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills/future-x/versions/1.0/download",
            500,
            "{}",
        )])
        .await;
        let err = download_skill_zip(&base, "future-x", "1.0")
            .await
            .unwrap_err();
        assert!(err.contains("Failed to download skill: 500"), "err: {err}");
        // Empty body.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills/future-x/versions/1.0/download",
            200,
            "",
        )])
        .await;
        let err = download_skill_zip(&base, "future-x", "1.0")
            .await
            .unwrap_err();
        assert_eq!(err, "Empty response body");
        // Network error.
        let err = download_skill_zip("http://127.0.0.1:1", "future-x", "1.0")
            .await
            .unwrap_err();
        assert!(err.contains("Network error:"), "err: {err}");
        // URL-encoding of exotic ids/versions reaches the path.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills/a%20b/versions/v%2F1/download",
            200,
            "eA==", // not a zip — just need bytes back
        )])
        .await;
        let bytes = download_skill_zip(&base, "a b", "v/1")
            .await
            .expect("download");
        assert_eq!(bytes, b"eA==");
    }

    #[tokio::test]
    async fn install_skill_download_failure_sets_exit_code() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![]).await; // 404
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        install_skill("future-x", Some("9.9"), &out)
            .await
            .expect("ok");
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Skill version \"future-x@9.9\" not found."),
            "stderr: {stderr}"
        );
    }

    #[tokio::test]
    async fn install_skill_bad_zip_propagates_err() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills/future-x/versions/1.0/download",
            200,
            "this is definitely not a zip file",
        )])
        .await;
        point_platform_at(&base).await;
        let (out, _) = Output::memory();
        let err = install_skill("future-x", Some("1.0"), &out)
            .await
            .unwrap_err();
        assert!(err.contains("unzip failed"), "err: {err}");
    }

    // ── flatten / rename / copy helpers ─────────────────────────────

    #[tokio::test]
    async fn flatten_single_subdir_moves_contents_up() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("pkg");
        tokio::fs::create_dir_all(&nested).await.unwrap();
        tokio::fs::write(nested.join("SKILL.md"), "x")
            .await
            .unwrap();
        flatten_single_subdir(dir.path()).await.unwrap();
        assert!(dir.path().join("SKILL.md").exists());
        assert!(!nested.exists());

        // Two entries → untouched.
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a"), "x").await.unwrap();
        tokio::fs::write(dir.path().join("b"), "x").await.unwrap();
        flatten_single_subdir(dir.path()).await.unwrap();
        assert!(dir.path().join("a").exists());

        // Single FILE (not dir) → untouched.
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("only.txt"), "x")
            .await
            .unwrap();
        flatten_single_subdir(dir.path()).await.unwrap();
        assert!(dir.path().join("only.txt").exists());

        // Missing dir → Ok (no-op).
        flatten_single_subdir(Path::new("/no/such/dir"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rename_across_device_fallback_on_missing_src() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("missing");
        let dest = dir.path().join("dest");
        // rename fails (no src) → copy fallback also fails → Err.
        assert!(rename_across_device(&src, &dest).await.is_err());
        // Happy path.
        let src = dir.path().join("real");
        tokio::fs::write(&src, "data").await.unwrap();
        rename_across_device(&src, &dest).await.unwrap();
        assert_eq!(tokio::fs::read_to_string(&dest).await.unwrap(), "data");
    }

    #[tokio::test]
    async fn copy_recursive_dir_tree() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        tokio::fs::create_dir_all(src.join("sub")).await.unwrap();
        tokio::fs::write(src.join("top.txt"), "1").await.unwrap();
        tokio::fs::write(src.join("sub").join("deep.txt"), "2")
            .await
            .unwrap();
        let dest = dir.path().join("dest");
        copy_recursive(&src, &dest).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(dest.join("top.txt"))
                .await
                .unwrap(),
            "1"
        );
        assert_eq!(
            tokio::fs::read_to_string(dest.join("sub").join("deep.txt"))
                .await
                .unwrap(),
            "2"
        );
        // Missing source → Err.
        assert!(copy_recursive(&dir.path().join("nope"), &dest)
            .await
            .is_err());
    }

    // ── uninstall / installed-ids ───────────────────────────────────

    #[tokio::test]
    async fn uninstall_skill_paths() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let (out, cap) = Output::memory();
        uninstall_skill("ghost", &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "Skill \"ghost\" is not installed.\n");

        plant_skill("future-x", "1.0").await;
        let (out, cap) = Output::memory();
        uninstall_skill("future-x", &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Uninstalled skill \"future-x\" from"),
            "stdout: {stdout}"
        );
        assert!(!skills_dir().join("future-x").exists());
    }

    #[tokio::test]
    async fn uninstall_remove_failure_is_err() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // A regular FILE at the skill path: metadata ok, remove_dir_all errs.
        tokio::fs::create_dir_all(skills_dir()).await.unwrap();
        tokio::fs::write(skills_dir().join("future-x"), "not a dir")
            .await
            .unwrap();
        let (out, _) = Output::memory();
        assert!(uninstall_skill("future-x", &out).await.is_err());
    }

    #[tokio::test]
    async fn get_installed_skill_ids_scans_skill_md() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Missing dir → empty.
        assert!(get_installed_skill_ids().await.is_empty());
        plant_skill("future-a", "1.0").await;
        plant_skill("future-b", "2.0").await;
        tokio::fs::create_dir_all(skills_dir().join("no-skill-md"))
            .await
            .unwrap();
        let ids = get_installed_skill_ids().await;
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("future-a") && ids.contains("future-b"));
    }

    // ── install-builtin / update / dispatch ─────────────────────────

    #[tokio::test]
    async fn install_builtin_full_flow() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let zip_a = make_zip(&[("SKILL.md", "---\nversion: 1.0\n---\n")]);
        let zip_b = make_zip(&[("SKILL.md", "---\nversion: 2.0\n---\n")]);
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/skills",
                200,
                &catalog(&[
                    ("future-a", Some("1.0"), "a"),
                    ("future-b", Some("2.0"), "b"),
                    ("future-skip", None, "no version"),
                    ("other-x", Some("1.0"), "not builtin"),
                ]),
            ),
            crate::test_server::HttpRoute::binary(
                "/client/v1/skills/future-a/versions/1.0/download",
                200,
                zip_a.clone(),
            ),
            crate::test_server::HttpRoute::binary(
                "/client/v1/skills/future-b/versions/2.0/download",
                200,
                zip_b.clone(),
            ),
        ])
        .await;
        point_platform_at(&base).await;
        plant_skill("future-b", "2.0").await; // already installed → skipped

        let (out, cap) = Output::memory();
        install_builtin_skills(&out).await;
        assert_eq!(out.exit_code(), 0);
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Installing 2 builtin skills (1 already installed)..."),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("Skipping future-skip — no version available."),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("Done. 2 skills installed."),
            "stdout: {stdout}"
        );
        assert!(skills_dir().join("future-a").join("SKILL.md").exists());
        // Non-builtin skill not installed.
        assert!(!skills_dir().join("other-x").exists());

        // Second run: future-skip still has no version, so it stays pending.
        let (out, cap) = Output::memory();
        install_builtin_skills(&out).await;
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Installing 1 builtin skills (2 already installed)..."),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn install_builtin_catalog_failure_and_empty() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        point_platform_at("http://127.0.0.1:1").await;
        let (out, cap) = Output::memory();
        install_builtin_skills(&out).await;
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Failed to fetch builtin skills."),
            "stderr: {stderr}"
        );

        // Catalog with no future-* skills.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &catalog(&[("other-x", Some("1.0"), "x")]),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        install_builtin_skills(&out).await;
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "No builtin skills available.\n");
    }

    #[tokio::test]
    async fn update_skills_flow() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        plant_skill("future-a", "0.9").await; // outdated
        plant_skill("future-b", "2.0").await; // up to date
        let zip = make_zip(&[("SKILL.md", "---\nversion: 1.0\n---\n")]);
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/skills",
                200,
                &catalog(&[
                    ("future-a", Some("1.0"), "a"),
                    ("future-b", Some("2.0"), "b"),
                    ("future-c", None, "no version installed? no"),
                ]),
            ),
            crate::test_server::HttpRoute::binary(
                "/client/v1/skills/future-a/versions/1.0/download",
                200,
                zip.clone(),
            ),
        ])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        update_skills(&out).await.expect("update");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Fetching skill catalog from"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("  future-a: 0.9 → 1.0"), "stdout: {stdout}");
        assert!(
            stdout.contains("Updated 1 skill(s), 1 already up to date."),
            "stdout: {stdout}"
        );
        // Actually upgraded on disk.
        let ver = read_skill_md_version(&skills_dir().join("future-a").join("SKILL.md")).await;
        assert_eq!(ver.as_deref(), Some("1.0"));
    }

    #[tokio::test]
    async fn update_skills_empty_states() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Fetch failure propagates as Err.
        point_platform_at("http://127.0.0.1:1").await;
        let (out, _) = Output::memory();
        assert!(update_skills(&out).await.is_err());

        // Empty catalog.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            "{\"skills\":[]}",
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        update_skills(&out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("No skills available."), "stdout: {stdout}");

        // Catalog non-empty but nothing installed.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &catalog(&[("future-a", Some("1.0"), "a")]),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        update_skills(&out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("No skills installed."), "stdout: {stdout}");

        // Installed but already at latest → "already up to date".
        plant_skill("future-a", "1.0").await;
        let (out, cap) = Output::memory();
        update_skills(&out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("1 skill(s) already up to date."),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn update_skills_install_failure_is_reported() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        plant_skill("future-a", "0.9").await;
        // Download serves a bad zip → install_skill errors → "Failed:" line.
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/skills",
                200,
                &catalog(&[("future-a", Some("1.0"), "a")]),
            ),
            crate::test_server::HttpRoute::json(
                "/client/v1/skills/future-a/versions/1.0/download",
                200,
                "not a zip",
            ),
        ])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        update_skills(&out).await.unwrap();
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("  Failed: "), "stderr: {stderr}");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // Nothing updated; up_to_date didn't count it either.
        assert!(
            stdout.contains("Updated 0 skill(s)")
                || stdout.contains("0 skill(s) already up to date"),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn update_skills_missing_skill_md_counts_up_to_date() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Installed id (dir with SKILL.md) whose version can't be read:
        // get_installed_skill_ids requires SKILL.md, so make it unreadable
        // as version (no frontmatter) → local_ver None → up_to_date.
        let dir = skills_dir().join("future-a");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("SKILL.md"), "# no frontmatter\n")
            .await
            .unwrap();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &catalog(&[("future-a", Some("1.0"), "a")]),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        update_skills(&out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("1 skill(s) already up to date."),
            "stdout: {stdout}"
        );
    }

    #[tokio::test]
    async fn dispatch_subcommands() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Agent refresh RPC goes nowhere (dead default addr) — best effort.
        let _grpc = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);

        // uninstall without a name → usage + exit code.
        let (out, cap) = Output::memory();
        skills("uninstall", &[], &out).await.unwrap();
        assert_eq!(out.exit_code(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert_eq!(stderr, "Usage: future skills uninstall <skill-name>\n");

        // Unknown subcommand (direct call) → error.
        let (out, _) = Output::memory();
        let err = skills("bogus", &[], &out).await.unwrap_err();
        assert!(err.contains("Unknown skills command"), "err: {err}");

        // install with --version strips a leading "v".
        let zip = make_zip(&[("SKILL.md", "---\nversion: 1.0\n---\n")]);
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::binary(
            "/client/v1/skills/future-x/versions/1.0/download",
            200,
            zip.clone(),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, _) = Output::memory();
        skills(
            "install",
            &[
                "future-x".to_string(),
                "--version".to_string(),
                "v1.0".to_string(),
            ],
            &out,
        )
        .await
        .unwrap();
        assert!(skills_dir().join("future-x").exists());

        // install WITHOUT a name → builtin install flow (the mock has no
        // catalog route → the failure message mentions builtin skills).
        let (out, cap) = Output::memory();
        skills("install", &[], &out).await.unwrap();
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("builtin"), "stderr: {stderr}");

        // uninstall happy path through the dispatch.
        let (out, _) = Output::memory();
        skills("uninstall", &["future-x".to_string()], &out)
            .await
            .unwrap();
        assert!(!skills_dir().join("future-x").exists());
    }

    // ── read_skill_md_version remaining edges ───────────────────────

    #[tokio::test]
    async fn read_skill_md_version_more_edges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        // metadata JSON with a NUMERIC version → stringified.
        tokio::fs::write(&path, "---\nmetadata: {\"version\": 2.5}\n---\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await.as_deref(), Some("2.5"));
        // metadata non-JSON starting with version: → parsed as yaml prefix.
        tokio::fs::write(&path, "---\nmetadata: version: 4.0 extra\n---\n")
            .await
            .unwrap();
        // not JSON → strip_prefix("version:") on the rest fails (starts with
        // "version: ..."? it does) → unquote.
        assert_eq!(
            read_skill_md_version(&path).await.as_deref(),
            Some("4.0 extra")
        );
        // metadata JSON without version, YAML block scan; comments skipped.
        tokio::fs::write(
            &path,
            "---\nmetadata:\n  # comment\n  version: '5.1'\n---\n",
        )
        .await
        .unwrap();
        assert_eq!(read_skill_md_version(&path).await.as_deref(), Some("5.1"));
        // Quoted direct version.
        tokio::fs::write(&path, "---\nversion: \"6.0\"\n---\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await.as_deref(), Some("6.0"));
        // Empty quoted version → None.
        tokio::fs::write(&path, "---\nversion: \"\"\n---\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await, None);
        // No closing frontmatter delimiter → None.
        tokio::fs::write(&path, "---\nversion: 1.0\n# no end\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await, None);
        // metadata JSON with empty-string version → falls through → None.
        tokio::fs::write(&path, "---\nmetadata: {\"version\": \"\"}\n---\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await, None);
    }

    // ── Remainder coverage ────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_install_builtin_update_and_bare_install() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let zip = make_zip(&[("SKILL.md", "---\nversion: 1.0\n---\n")]);
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json(
                "/client/v1/skills",
                200,
                &catalog(&[("future-a", Some("1.0"), "a")]),
            ),
            crate::test_server::HttpRoute::binary(
                "/client/v1/skills/future-a/versions/1.0/download",
                200,
                zip,
            ),
        ])
        .await;
        point_platform_at(&base).await;
        // notify_agent_refresh_skills is fire-and-forget: point at a dead
        // port so it fails fast and silently.
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);

        // install-builtin dispatch arm.
        let (out, cap) = Output::memory();
        skills("install-builtin", &[], &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Installing 1 builtin skills"), "{stdout}");

        // Second run: everything installed → early return line.
        let (out, cap) = Output::memory();
        skills("install-builtin", &[], &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("already installed"), "{stdout}");

        // Bare "install" (no name) == install-builtin.
        let (out, _cap) = Output::memory();
        skills("install", &[], &out).await.unwrap();

        // update dispatch arm (nothing to update).
        let (out, cap) = Output::memory();
        skills("update", &[], &out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("up to date") || stdout.contains("Up to date"),
            "{stdout}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_install_strips_leading_v() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let zip = make_zip(&[
            ("future-x/", ""),
            ("future-x/SKILL.md", "---\nversion: 1.0\n---\n"),
        ]);
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::binary(
            // The v-stripped version must be used in the download path.
            "/client/v1/skills/future-x/versions/1.0/download",
            200,
            zip,
        )])
        .await;
        point_platform_at(&base).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, cap) = Output::memory();
        skills(
            "install",
            &[
                "future-x".to_string(),
                "--version".to_string(),
                "v1.0".to_string(),
            ],
            &out,
        )
        .await
        .unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("Downloading future-x v1.0"), "{stdout}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_skips_catalog_entries_without_version() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Installed skill whose catalog entry has NO latest_version → skip.
        plant_skill("future-a", "0.9").await;
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &catalog(&[("future-a", None, "a")]),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        update_skills(&out).await.unwrap();
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("up to date") || stdout.contains("Up to date"),
            "{stdout}"
        );
    }

    #[tokio::test]
    async fn list_ignores_skills_without_version() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // One skill with a version, one whose SKILL.md has none (excluded
        // from the installed set), one dir with no SKILL.md at all.
        plant_skill("future-a", "1.0").await;
        let plain = skills_dir().join("future-plain");
        tokio::fs::create_dir_all(&plain).await.unwrap();
        tokio::fs::write(plain.join("SKILL.md"), "# no frontmatter\n")
            .await
            .unwrap();
        tokio::fs::create_dir_all(skills_dir().join("future-empty"))
            .await
            .unwrap();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            &catalog(&[("future-a", Some("1.0"), "a")]),
        )])
        .await;
        point_platform_at(&base).await;
        let (out, cap) = Output::memory();
        list_skills(&out).await;
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.contains("future-a"), "{stdout}");
        // The unversioned/SKILL.md-less dirs are not in the installed set.
        assert!(!stdout.contains("future-plain"), "{stdout}");
        assert!(!stdout.contains("future-empty"), "{stdout}");
    }

    #[tokio::test]
    async fn read_skill_md_version_numeric_and_yaml_edge_arms() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();

        // Metadata JSON with a NUMERIC version → f64 stringification.
        let path = dir.path().join("SKILL.md");
        tokio::fs::write(&path, "---\nname: x\nmetadata: {\"version\": 1.5}\n---\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await, Some("1.5".to_string()));

        // Metadata JSON with an EMPTY version → falls through to None.
        tokio::fs::write(&path, "---\nmetadata: {\"version\": \"\"}\n---\n")
            .await
            .unwrap();
        assert_eq!(read_skill_md_version(&path).await, None);

        // YAML metadata block: comment lines skipped; a non-indented line
        // terminates the block scan (no version anywhere → None).
        tokio::fs::write(
            &path,
            "---\nmetadata:\n  # comment\n  other: 1\nnext: 2\n---\n",
        )
        .await
        .unwrap();
        assert_eq!(read_skill_md_version(&path).await, None);

        // YAML block with the version before any terminator → found.
        tokio::fs::write(
            &path,
            "---\nmetadata:\n  # comment\n  version: '2.3'\n---\n",
        )
        .await
        .unwrap();
        assert_eq!(read_skill_md_version(&path).await, Some("2.3".to_string()));
    }

    #[tokio::test]
    async fn flatten_single_subdir_vanishing_entry_is_ok() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        // Single entry that is a BROKEN SYMLINK: metadata fails → Ok(()).
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                dir.path().join("missing-target"),
                dir.path().join("dangling"),
            )
            .unwrap();
            flatten_single_subdir(dir.path()).await.unwrap();
        }
    }

    #[tokio::test]
    async fn rename_across_device_fallback_copies_when_rename_fails() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        tokio::fs::create_dir_all(&src).await.unwrap();
        tokio::fs::write(src.join("f.txt"), "data").await.unwrap();
        // dest's PARENT does not exist → rename fails with ENOENT → the
        // copy+delete fallback runs (copy_recursive creates parents).
        let dest = dir.path().join("no-such-parent").join("dest");
        rename_across_device(&src, &dest).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(dest.join("f.txt")).await.unwrap(),
            "data"
        );
        assert!(!src.exists());
    }
}
