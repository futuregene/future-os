//! `future skills` — catalogue presentation and a one-shot frontend for the
//! host-local Agent SkillManager. All mutations and version decisions happen
//! in the manager shared with Agent RPC.

use crate::output::Output;
use crate::utils::platform::get_platform_url;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// `SKILLS_DIR` from skills.ts — `~/.future/agent/skills`.
///
/// Shares the agent's `~/.future/agent` root (`HOME`, then `USERPROFILE`):
/// `dirs::home_dir()` reads the Windows token profile and ignores a redirected
/// `HOME`, so installs landed outside the tree the agent discovers — and test
/// runs wrote skill fixtures into the developer's real skills directory.
pub fn skills_dir() -> PathBuf {
    future_agent::utils::default_config_dir().join("skills")
}

/// Reject path components before any network or filesystem side effect. Both
/// catalogue IDs and explicit CLI arguments are untrusted, on every platform.
fn validate_skill_component(value: &str, label: &str) -> Result<(), String> {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if value.is_empty()
        || value.len() > 128
        || value.contains("..")
        || value.ends_with('.')
        || reserved
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        return Err(format!("Invalid skill {label}: {value:?}"));
    }
    Ok(())
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
    /// Chinese description, when the catalogue carries one (`description_zh`).
    /// Emitted as `descriptionZh` by `skills list --json`; the table ignores it.
    #[serde(default)]
    pub description_zh: String,
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
    /// Catalogue classification for skills in the repository's builtin/ directory.
    /// Missing metadata must not turn a name prefix into an implicit opt-in.
    #[serde(default)]
    pub builtin: bool,
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
            // `future skills list --json` — the machine-readable catalogue the
            // TUI parses (it shells out to this binary; see the module docs of
            // tui/src/skills_cli.rs). Flags are scanned anywhere after `list`.
            if args.iter().any(|a| a == "--json") {
                list_skills_json(out).await;
            } else {
                list_skills(out).await;
            }
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
            let version = version_idx.and_then(|i| args.get(i + 1)).cloned();
            // Strip leading "v" if the user provided it (e.g. "v1.0" → "1.0")
            // to avoid a double "v" in output.
            let version = version.map(|v| v.strip_prefix('v').map(str::to_string).unwrap_or(v));
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

    // Check which skills are installed (a missing skills dir just means
    // nothing is installed).
    let installed = match installed_skill_versions().await {
        Ok(installed) => installed,
        Err(error) => {
            out.log_err(&error);
            out.set_exit_code(1);
            return;
        }
    };
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

/// The manager's reconciled host-local snapshot is the version source for
/// the table and machine-readable TUI catalogue.
async fn installed_skill_versions() -> Result<std::collections::HashMap<String, String>, String> {
    tokio::task::spawn_blocking(|| {
        future_agent::skills::manager::SkillManager::local()?.list_installed()
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
    .map(|skills| {
        skills
            .into_iter()
            .filter_map(|skill| skill.version.map(|version| (skill.id, version)))
            .collect()
    })
}

/// One catalogue row of `future skills list --json`. The wire names are
/// camelCase — that is the contract the TUI parses, and `installedVersion` is
/// what it uses to flag an upgradable skill. The CLI's own [`SkillInfo`] mirrors
/// the platform's snake_case instead, hence the explicit renames.
#[derive(serde::Serialize)]
struct SkillRowJson {
    id: String,
    name: String,
    #[serde(rename = "latestVersion")]
    latest_version: Option<String>,
    #[serde(rename = "installedVersion")]
    installed_version: Option<String>,
    description: String,
    #[serde(rename = "descriptionZh")]
    description_zh: String,
}

/// The whole `future skills list --json` document: `{skills, count}` in that
/// order.
#[derive(serde::Serialize)]
struct SkillCatalogueJson {
    skills: Vec<SkillRowJson>,
    count: usize,
}

/// `listSkills()` with `--json` — same catalogue and installed scan as the
/// table, emitted as a single JSON document on stdout (the TUI cannot link this
/// crate: `future-cli` embeds `future-tui`). Nothing else goes to stdout — no
/// table, no separator, no progress text — and failures follow the table path's
/// contract: empty stdout, message on stderr, exit code 1.
async fn list_skills_json(out: &Output) {
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

    let installed = match installed_skill_versions().await {
        Ok(installed) => installed,
        Err(error) => {
            out.log_err(&error);
            out.set_exit_code(1);
            return;
        }
    };
    let rows: Vec<SkillRowJson> = skills
        .into_iter()
        .map(|skill| SkillRowJson {
            // Computed first: `installed.get` borrows `skill.id`, which the
            // struct literal below moves.
            installed_version: installed.get(&skill.id).cloned(),
            id: skill.id,
            name: skill.name,
            latest_version: skill.latest_version,
            description: skill.description,
            description_zh: skill.description_zh,
        })
        .collect();
    let count = rows.len();
    let catalogue = SkillCatalogueJson {
        skills: rows,
        count,
    };
    let json = serde_json::to_string(&catalogue).expect("catalogue rows are plain data");
    out.log(&json);
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

/// Upgrade managed skills and install builtin skills that have never been seen.
async fn update_skills(out: &Output) -> Result<(), String> {
    let result = tokio::task::spawn_blocking(|| {
        future_agent::skills::manager::SkillManager::local()?.sync(true)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    out.log(&format!(
        "Installed {} new builtin skill(s), upgraded {} skill(s), skipped {}.",
        result.installed.len(),
        result.upgraded.len(),
        result.skipped.len()
    ));
    for failure in result.failed {
        out.log_err(&failure);
        out.set_exit_code(1);
    }
    Ok(())
}

async fn uninstall_skill(skill_id: &str, out: &Output) -> Result<(), String> {
    validate_skill_component(skill_id, "id")?;
    let id = skill_id.to_string();
    let removed = tokio::task::spawn_blocking(move || {
        future_agent::skills::manager::SkillManager::local()?.uninstall(&id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    if removed {
        out.log(&format!("Uninstalled skill \"{skill_id}\"."));
    } else {
        out.log(&format!("Skill \"{skill_id}\" is not installed."));
    }
    Ok(())
}

// ── Remote API ─────────────────────────────────────────────────────────────

const SKILL_HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn skill_http_client(timeout: std::time::Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

/// `fetchSkills(platformUrl)` — GET {platform}/client/v1/skills.
pub async fn fetch_skills(platform_url: &str) -> Result<Vec<SkillInfo>, String> {
    let url = format!("{platform_url}/client/v1/skills");
    let response = skill_http_client(SKILL_HTTP_TIMEOUT)?
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

// ── Builtin bootstrap and explicit install ───────────────────────────────

/// Add catalogued builtins with no installation or install/uninstall history.
pub async fn install_builtin_skills(out: &Output) {
    let result = tokio::task::spawn_blocking(|| {
        future_agent::skills::manager::SkillManager::local()?.install_missing_builtins()
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|value| value.map_err(|error| error.to_string()));
    match result {
        Ok(result) => {
            out.log(&format!(
                "Done. {} builtin skills installed.",
                result.installed.len()
            ));
            for failure in result.failed {
                out.log_err(&failure);
                out.set_exit_code(1);
            }
        }
        Err(error) => {
            out.log_err(&format!("Failed to install builtin skills: {error}"));
            out.set_exit_code(1);
        }
    }
}

async fn install_skill(skill_id: &str, version: Option<&str>, out: &Output) -> Result<(), String> {
    validate_skill_component(skill_id, "id")?;
    if let Some(version) = version {
        validate_skill_component(version, "version")?;
    }
    let version = match version {
        Some(version) => version.to_string(),
        None => {
            let platform = get_platform_url(None).await;
            fetch_skills(&platform)
                .await?
                .into_iter()
                .find(|skill| skill.id == skill_id)
                .and_then(|skill| skill.latest_version)
                .ok_or_else(|| format!("Skill \"{skill_id}\" has no available version."))?
        }
    };
    let id = skill_id.to_string();
    let selected = version.clone();
    tokio::task::spawn_blocking(move || {
        future_agent::skills::manager::SkillManager::local()?.install(&id, &selected)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    out.log(&format!("Installed skill \"{skill_id}\" v{version}."));
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
                        if let Some(version) = as_string.filter(|v| !v.is_empty()) {
                            return Some(version);
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
    let stripped = match val.chars().next() {
        Some(quote @ ('"' | '\'')) => val.get(1..)?.strip_suffix(quote)?,
        _ => val,
    };
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_skill_commands() {
        assert!(is_skills_command(Some("install")));
        assert!(is_skills_command(Some("update")));
        assert!(!is_skills_command(Some("unknown")));
    }

    #[test]
    fn components_cannot_escape_skill_directory() {
        for value in ["../x", "a/b", "", "CON", "with space"] {
            assert!(validate_skill_component(value, "id").is_err());
        }
    }
}
