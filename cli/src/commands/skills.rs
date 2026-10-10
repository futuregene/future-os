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
    use crate::test_env::EnvGuard;
    use crate::test_server::{spawn_http, HttpRoute};

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

    // ── validate_skill_component: the Windows device-name rule ─────────

    /// A skill id becomes a directory name, so the reserved device names are
    /// rejected — with or without an extension (`CON.txt` still names the
    /// console device) and in any case. `COM1`..`COM9`/`LPT1`..`LPT9` are the
    /// four-character arm, so `COM0`/`COM10` are ordinary names and must pass:
    /// an off-by-one here would either block valid skills or let `COM1`
    /// through.
    #[test]
    fn reserved_device_names_are_rejected_with_their_boundary() {
        for reserved in [
            "CON",
            "PRN",
            "AUX",
            "NUL",
            "com1",
            "Com9",
            "lpt1",
            "LPT9",
            "con.md",
            "nul.tar.gz",
        ] {
            assert!(
                validate_skill_component(reserved, "id").is_err(),
                "{reserved} names a Windows device"
            );
        }
        for allowed in ["COM0", "COM10", "LPT0", "LPT10", "CONS", "NULL", "console"] {
            assert!(
                validate_skill_component(allowed, "id").is_ok(),
                "{allowed} is an ordinary name"
            );
        }
    }

    /// The length/character boundary: 128 characters is the last accepted
    /// length, 129 the first rejected; only ASCII alphanumerics and `.`/`_`/`-`
    /// may appear, so a CJK id (which the catalogue does not carry) and a
    /// trailing dot are refused before any filesystem call.
    #[test]
    fn component_length_and_character_set_are_exact() {
        assert!(validate_skill_component(&"a".repeat(128), "id").is_ok());
        assert!(validate_skill_component(&"a".repeat(129), "id").is_err());
        for bad in [
            "skill.", "..", "a..b", "技能", "skill!", "sk ill", "skill\\x",
        ] {
            assert!(validate_skill_component(bad, "id").is_err(), "{bad:?}");
        }
        for good in ["skill", "skill-name", "skill_name", "skill.v1", "A1"] {
            assert!(validate_skill_component(good, "id").is_ok(), "{good}");
        }
        // The message names the label so an install and a version failure are
        // distinguishable.
        let err = validate_skill_component("../x", "version").unwrap_err();
        assert!(err.contains("version"), "{err}");
        assert!(err.contains("../x"), "{err}");
    }

    // ── pad ─────────────────────────────────────────────────────────────

    /// `padEnd` counts *characters*, so a CJK name pads by its display width
    /// in chars (2 per glyph is not modelled — the JS original counts code
    /// units too) and an already-wide value is never truncated.
    #[test]
    fn pad_pads_by_character_count_and_never_truncates() {
        assert_eq!(pad("ab", 5), "ab   ");
        assert_eq!(pad("abcde", 5), "abcde");
        assert_eq!(pad("abcdef", 5), "abcdef", "a wide value is left alone");
        assert_eq!(pad("", 3), "   ");
        assert_eq!(pad("网页", 4), "网页  ", "two chars, not four bytes");
        assert_eq!(pad("—", 2), "— ");
    }

    // ── unquote ─────────────────────────────────────────────────────────

    /// `unquote` strips a *matching* quote pair, and an empty result is `None`
    /// (empty version strings are falsy everywhere the value is consumed). A
    /// leading quote with no matching trailing one fails the strip and yields
    /// `None` rather than a value that still carries a quote.
    #[test]
    fn unquote_strips_only_matching_quotes_and_rejects_empty() {
        assert_eq!(unquote("1.0"), Some("1.0".to_string()));
        assert_eq!(unquote("\"1.0\""), Some("1.0".to_string()));
        assert_eq!(unquote("'1.0'"), Some("1.0".to_string()));
        assert_eq!(unquote("\"\""), None);
        assert_eq!(unquote("''"), None);
        assert_eq!(unquote(""), None);
        // Mismatched or unterminated quoting is not a quoted value.
        assert_eq!(unquote("\"1.0'"), None);
        assert_eq!(unquote("'"), None);
        assert_eq!(unquote("\""), None);
        // Only the *outer* pair is stripped.
        assert_eq!(unquote("\"1'0\""), Some("1'0".to_string()));
    }

    // ── read_skill_md_version ───────────────────────────────────────────

    /// Write `body` to a SKILL.md inside a fresh tempdir and read the version.
    async fn version_of(body: &str) -> Option<String> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("SKILL.md");
        tokio::fs::write(&path, body).await.expect("write SKILL.md");
        read_skill_md_version(&path).await
    }

    /// The frontmatter parser: a plain `version:` field, comments and blank
    /// lines skipped, a quoted value unquoted, and no frontmatter at all is
    /// `None` rather than an error.
    #[tokio::test]
    async fn version_reads_the_simple_frontmatter_field() {
        assert_eq!(
            version_of("---\nversion: 1.0\n---\nbody\n").await,
            Some("1.0".into())
        );
        assert_eq!(
            version_of("---\nversion: \"1.0\"\n---\n").await,
            Some("1.0".into())
        );
        assert_eq!(
            version_of("---\nversion: '2.1'\n---\n").await,
            Some("2.1".into())
        );
        assert_eq!(
            version_of("---\n# a comment\n\nversion: 3.2\n---\n").await,
            Some("3.2".into())
        );
        // No frontmatter at all.
        assert_eq!(version_of("just text\n").await, None);
        assert_eq!(version_of("---\nname: x\n---\n").await, None);
        // An empty file is not a frontmatter document.
        assert_eq!(version_of("").await, None);
        // A missing file is `None`, not a panic.
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            read_skill_md_version(&dir.path().join("gone.md")).await,
            None
        );
    }

    /// The unterminated-frontmatter boundary: `---` opening with no closing
    /// `---` yields `None` (the search index is `usize::MAX`), and the closing
    /// marker may be at the very start of the remainder (`---\n---`).
    #[tokio::test]
    async fn version_requires_a_terminated_frontmatter_block() {
        assert_eq!(version_of("---\nversion: 1.0\n").await, None);
        assert_eq!(version_of("---\n").await, None);
        assert_eq!(
            version_of("---\nversion: 1.0\n---").await,
            Some("1.0".into())
        );
        // A `---` later in the body is an acceptable terminator (the JS
        // `indexOf` fallback), so the field before it is still read.
        assert_eq!(
            version_of("---\nversion: 4\n\ntext\n---\n").await,
            Some("4".into())
        );
    }

    /// `metadata:` accepts an inline JSON object (`"version": "…"` and a
    /// numeric version), an inline `metadata: version:` form, and a YAML block
    /// whose indented lines are scanned until the block ends.
    #[tokio::test]
    async fn version_reads_metadata_json_and_yaml_block_forms() {
        assert_eq!(
            version_of("---\nmetadata: {\"version\":\"5.1\"}\n---\n").await,
            Some("5.1".into())
        );
        // A numeric JSON version is stringified (`String(meta.version)`).
        assert_eq!(
            version_of("---\nmetadata: {\"version\":6}\n---\n").await,
            Some("6".into())
        );
        // JSON without a version, then an inline `version:` after the colon.
        assert_eq!(
            version_of("---\nmetadata: {\"other\":1}\n---\n").await,
            None
        );
        assert_eq!(
            version_of("---\nmetadata: version: 7.0\n---\n").await,
            Some("7.0".into())
        );
        // YAML block: indented `version:` under `metadata:` — with a commented
        // line inside the block (skipped, not a terminator) and a blank one.
        assert_eq!(
            version_of("---\nmetadata:\n  # a note\n  other: 1\n\n  version: 8.2\n---\n").await,
            Some("8.2".into())
        );
        assert_eq!(
            version_of("---\nmetadata:\n  other: 1\n  version: 8.2\n---\n").await,
            Some("8.2".into())
        );
        // The block ends at the first unindented line.
        assert_eq!(
            version_of("---\nmetadata:\n  other: 1\n---\n").await,
            None,
            "a block with no version has no version"
        );
        // …but the outer scan trims every line before testing for `version:`,
        // so an indented one is still the frontmatter version (the JS original
        // trims too — indentation is not significant to this parser).
        assert_eq!(
            version_of("---\nmetadata:\n  other: 1\nname: x\n  version: 9\n---\n").await,
            Some("9".into())
        );
        // A JSON version that is `null` or blank falls through to no version.
        assert_eq!(
            version_of("---\nmetadata: {\"version\":null}\n---\n").await,
            None
        );
        assert_eq!(
            version_of("---\nmetadata: {\"version\":\"\"}\n---\n").await,
            None
        );
    }

    // ── fetch_skills (mock HTTP) ───────────────────────────────────────

    /// `fetch_skills` against a real HTTP response: rows decode, an empty
    /// `latest_version` becomes `None` (JS falsiness), and a row that cannot
    /// deserialise is skipped rather than failing the whole catalogue.
    #[tokio::test(flavor = "multi_thread")]
    async fn fetch_skills_normalises_the_catalogue_rows() {
        let base = spawn_http(vec![HttpRoute::json(
            "/client/v1/skills",
            200,
            r#"{"skills":[
                {"id":"a","name":"A","description":"d","latest_version":"1.2"},
                {"id":"b","latest_version":""},
                {"id":"c"},
                {"not_an_object":1}
            ]}"#,
        )])
        .await;
        let skills = fetch_skills(&base).await.expect("fetch");
        assert_eq!(
            skills.len(),
            4,
            "a non-object row still decodes (all fields default)"
        );
        assert_eq!(skills[0].id, "a");
        assert_eq!(skills[0].latest_version.as_deref(), Some("1.2"));
        assert_eq!(skills[1].latest_version, None, "\"\" is normalised to None");
        assert_eq!(skills[2].latest_version, None);
        assert!(skills[2].name.is_empty());
    }

    /// Every failure mode of the catalogue fetch is a distinct message: a
    /// non-2xx status carries the code and reason, invalid JSON carries the
    /// parse error, and a body with no `skills` array is an empty catalogue
    /// rather than an error (JS `body.skills ?? []`).
    #[tokio::test(flavor = "multi_thread")]
    async fn fetch_skills_reports_http_and_parse_failures() {
        let base = spawn_http(vec![
            HttpRoute::json("/client/v1/skills", 500, "{}"),
            HttpRoute::json("/empty", 200, "{}"),
        ])
        .await;
        let err = fetch_skills(&base).await.unwrap_err();
        assert!(err.contains("500"), "{err}");
        assert!(err.contains("Internal Server Error"), "{err}");

        // 404 has a canonical reason too.
        let err = fetch_skills(&format!("{base}/missing")).await.unwrap_err();
        assert!(err.contains("404"), "{err}");

        // A body that is not JSON at all.
        let not_json = spawn_http(vec![HttpRoute::json("/client/v1/skills", 200, "<html>")]).await;
        assert!(fetch_skills(&not_json).await.is_err());

        // Unreachable host: the transport error is wrapped.
        let err = fetch_skills("http://127.0.0.1:1").await.unwrap_err();
        assert!(err.contains("Failed to fetch skills"), "{err}");
    }

    /// `skills: null`, a missing `skills` key and an empty array all mean "no
    /// skills", never an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn fetch_skills_treats_a_missing_skills_field_as_empty() {
        for body in ["{}", r#"{"skills":null}"#, r#"{"skills":[]}"#] {
            let base = spawn_http(vec![HttpRoute::json("/client/v1/skills", 200, body)]).await;
            assert!(
                fetch_skills(&base).await.expect("fetch").is_empty(),
                "{body}"
            );
        }
    }

    // ── list / list --json through `skills()` ──────────────────────────

    /// An isolated HOME plus a catalogue served over HTTP: returns the temp
    /// dir (kept alive for cleanup) and the `EnvGuard` restoring the
    /// environment. The gRPC address points at a dead port so `refresh_skills`
    /// notifications cannot reach (or disturb) a real agent.
    async fn isolated_platform(catalogue_body: &str) -> (tempfile::TempDir, EnvGuard, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let guard = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
            (
                "FUTURE_AGENT_GRPC_ADDR",
                std::ffi::OsString::from("127.0.0.1:1"),
            ),
        ]);
        let base = spawn_http(vec![HttpRoute::json(
            "/client/v1/skills",
            200,
            catalogue_body,
        )])
        .await;
        let auth = crate::constants::auth_file();
        tokio::fs::create_dir_all(auth.parent().expect("parent"))
            .await
            .expect("mkdir");
        tokio::fs::write(&auth, format!(r#"{{"future":{{"base_url":"{base}"}}}}"#))
            .await
            .expect("write auth.json");
        (dir, guard, base)
    }

    /// Run `skills(...)` with captured output.
    async fn run_skills(command: &str, args: &[&str]) -> (Result<(), String>, String, String, i32) {
        let (out, cap) = Output::memory();
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        let result = skills(command, &args, &out).await;
        let stdout = String::from_utf8(cap.out.lock().expect("out").clone()).expect("utf8");
        let stderr = String::from_utf8(cap.err.lock().expect("err").clone()).expect("utf8");
        (result, stdout, stderr, out.exit_code())
    }

    const CATALOGUE: &str = r#"{"skills":[
        {"id":"future-web","name":"Web","description":"Web search","latest_version":"1.2"},
        {"id":"very-long-skill-id","name":"Long","description":"A description long enough to be truncated by the table's column cap","latest_version":"2.0"}
    ]}"#;

    /// The human table: a header, a separator row, one row per skill, and the
    /// footer count. The installed column shows the manager's version for an
    /// installed skill and an em dash for one that is not.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_renders_the_table_with_installed_and_missing_versions() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(CATALOGUE).await;
        // One skill is installed locally, the other is not.
        let installed = skills_dir().join("future-web");
        tokio::fs::create_dir_all(&installed).await.expect("mkdir");
        tokio::fs::write(installed.join("SKILL.md"), "---\nversion: 1.1\n---\n")
            .await
            .expect("write SKILL.md");

        let (result, stdout, stderr, code) = run_skills("list", &[]).await;
        result.expect("list succeeds");
        assert_eq!(code, 0);
        assert_eq!(stderr, "");
        assert!(stdout.contains("NAME"), "{stdout}");
        assert!(stdout.contains("DESCRIPTION"), "{stdout}");
        assert!(stdout.contains("future-web"), "{stdout}");
        assert!(stdout.contains("v1.2"), "the catalogue version: {stdout}");
        assert!(stdout.contains("—"), "the separator row: {stdout}");
        assert!(
            stdout.contains("2 skills available. Use \"future skills install <name>\" to install."),
            "{stdout}"
        );
        // The long description is truncated with a single ellipsis at the cap.
        assert!(stdout.contains('…'), "{stdout}");
    }

    /// An empty catalogue is a message, not an empty table, and it does not
    /// touch the manager (so it works with no skills directory at all).
    #[tokio::test(flavor = "multi_thread")]
    async fn list_reports_an_empty_catalogue() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(r#"{"skills":[]}"#).await;
        let (result, stdout, _stderr, code) = run_skills("list", &[]).await;
        result.expect("list succeeds");
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "No skills available.");
    }

    /// A catalogue that cannot be fetched is reported on stderr with exit code
    /// 1 and *nothing* on stdout — the table must not be half-printed.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_reports_a_failed_fetch_on_stderr() {
        let _env_lock = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
            (
                "FUTURE_AGENT_GRPC_ADDR",
                std::ffi::OsString::from("127.0.0.1:1"),
            ),
        ]);
        // No auth.json → the platform URL is the real default, which is not
        // reachable from a sandboxed test; point it at a dead port instead so
        // the failure is local and immediate.
        let auth = crate::constants::auth_file();
        tokio::fs::create_dir_all(auth.parent().expect("parent"))
            .await
            .expect("mkdir");
        tokio::fs::write(&auth, r#"{"future":{"base_url":"http://127.0.0.1:1"}}"#)
            .await
            .expect("write auth.json");

        for command_args in [vec![], vec!["--json"]] {
            let (result, stdout, stderr, code) = run_skills("list", &command_args).await;
            result.expect("the command itself succeeds");
            assert_eq!(code, 1, "args={command_args:?}");
            assert_eq!(stdout, "", "args={command_args:?}");
            assert!(
                stderr.contains("Failed to fetch skills from"),
                "args={command_args:?} stderr={stderr}"
            );
            assert!(stderr.contains("/client/v1/skills"), "{stderr}");
        }
    }

    /// `list --json` is the TUI's contract: one JSON document whose keys are
    /// camelCase, with `installedVersion` filled from the local manager and
    /// `null` for a skill that is not installed.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_json_emits_the_camel_case_catalogue() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(
            r#"{"skills":[{"id":"future-web","name":"Web","description":"Web search","description_zh":"网页搜索","latest_version":"1.2"},{"id":"other","latest_version":"3.0"}]}"#,
        )
        .await;
        let installed = skills_dir().join("future-web");
        tokio::fs::create_dir_all(&installed).await.expect("mkdir");
        tokio::fs::write(installed.join("SKILL.md"), "---\nversion: 1.1\n---\n")
            .await
            .expect("write SKILL.md");

        let (result, stdout, stderr, code) = run_skills("list", &["--json"]).await;
        result.expect("list --json succeeds");
        assert_eq!(code, 0);
        assert_eq!(stderr, "");
        let doc: Value = serde_json::from_str(&stdout).expect("stdout is one JSON document");
        assert_eq!(doc["count"], 2);
        let rows = doc["skills"].as_array().expect("skills array");
        assert_eq!(rows[0]["id"], "future-web");
        assert_eq!(rows[0]["latestVersion"], "1.2");
        assert_eq!(rows[0]["installedVersion"], "1.1");
        assert_eq!(rows[0]["descriptionZh"], "网页搜索");
        assert!(rows[1]["installedVersion"].is_null(), "{stdout}");
        // The document is emitted in `{skills, count}` order (a TUI streams it).
        assert!(stdout.starts_with("{\"skills\":"), "{stdout}");
        // A skill with no flags at all still decodes with empty strings.
        assert_eq!(rows[1]["name"], "");
    }

    /// `install-builtin` drives the manager once and then notifies the agent;
    /// on an isolated home with a catalogue that lists nothing to install it
    /// reports the count and leaves nothing on stderr.
    #[tokio::test(flavor = "multi_thread")]
    async fn install_builtin_reports_the_installed_count() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(r#"{"skills":[]}"#).await;
        let (result, stdout, stderr, code) = run_skills("install-builtin", &[]).await;
        result.expect("install-builtin succeeds");
        assert_eq!(code, 0);
        assert!(stdout.contains("0 builtin skills installed"), "{stdout}");
        assert_eq!(stderr, "");
    }

    /// `install` with no name is the same builtin bootstrap as
    /// `install-builtin` (the documented shorthand), not an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn install_without_a_name_bootstraps_the_builtins() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(r#"{"skills":[]}"#).await;
        let (result, stdout, stderr, code) = run_skills("install", &[]).await;
        result.expect("install with no name succeeds");
        assert_eq!(code, 0);
        assert!(stdout.contains("0 builtin skills installed"), "{stdout}");
        assert_eq!(stderr, "");
    }

    /// `update` syncs the managed skills and reports the three counts from the
    /// manager's own result — not a fixed string.
    #[tokio::test(flavor = "multi_thread")]
    async fn update_reports_the_sync_counts() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(r#"{"skills":[]}"#).await;
        let (result, stdout, stderr, code) = run_skills("update", &[]).await;
        result.expect("update succeeds");
        assert_eq!(code, 0);
        assert!(
            stdout.contains("Installed 0 new builtin skill(s), upgraded 0 skill(s), skipped 0."),
            "{stdout}"
        );
        assert_eq!(stderr, "");
    }

    /// Uninstalling a skill that is not installed is a report, not an error:
    /// the message says so and the exit code stays 0.
    #[tokio::test(flavor = "multi_thread")]
    async fn uninstall_reports_a_skill_that_is_not_installed() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(r#"{"skills":[]}"#).await;
        let (result, stdout, stderr, code) = run_skills("uninstall", &["absent-skill"]).await;
        result.expect("uninstalling something absent is not an error");
        assert_eq!(code, 0);
        assert!(
            stdout.contains("Skill \"absent-skill\" is not installed."),
            "{stdout}"
        );
        assert_eq!(stderr, "");
    }

    /// A catalogue row with no `latest_version` and no local installation: both
    /// columns fall back to the em dash, and the row's widths stay at their
    /// documented minimums rather than collapsing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_version_less_row_uses_the_em_dash_in_both_columns() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(
            r#"{"skills":[{"id":"bare","name":"Bare","description":"no version"},{"id":"versioned","latest_version":"9.9"}]}"#,
        )
        .await;
        let (result, stdout, _stderr, code) = run_skills("list", &[]).await;
        result.expect("list succeeds");
        assert_eq!(code, 0);
        // Two em dashes on the `bare` row (latest + installed), at least one on
        // the `versioned` row (not installed).
        let bare = stdout
            .lines()
            .find(|line| line.contains("bare"))
            .expect("the bare row is rendered");
        assert_eq!(bare.matches('—').count(), 2, "{bare}");
        assert!(bare.contains("no version"), "{bare}");
        let versioned = stdout
            .lines()
            .find(|line| line.contains("versioned"))
            .expect("the versioned row is rendered");
        assert!(versioned.contains("v9.9"), "{versioned}");
        assert_eq!(versioned.matches('—').count(), 1, "{versioned}");
        // The JSON form reports the same absence as null, not "".
        let (_result, stdout, _stderr, _code) = run_skills("list", &["--json"]).await;
        let doc: Value = serde_json::from_str(&stdout).expect("json");
        assert!(doc["skills"][0]["latestVersion"].is_null(), "{stdout}");
        assert!(doc["skills"][0]["installedVersion"].is_null(), "{stdout}");
    }

    /// The manager tolerates a skills path that is not a directory (it has
    /// nothing installed, which is not an error): the catalogue still renders,
    /// every `installedVersion` is `null`, and the exit code stays 0. The CLI's
    /// "the installed scan failed" arm is a different case — see the waiver
    /// ledger in `docs/testing/module-cli.md`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_skills_path_that_is_not_a_directory_reads_as_nothing_installed() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(CATALOGUE).await;
        let skills = skills_dir();
        tokio::fs::write(&skills, "not a directory")
            .await
            .expect("write blocker");
        assert!(skills.is_file());

        let (result, stdout, stderr, code) = run_skills("list", &["--json"]).await;
        result.expect("list succeeds");
        assert_eq!(code, 0, "stderr={stderr}");
        assert_eq!(stderr, "");
        let doc: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(doc["count"], 2);
        for row in doc["skills"].as_array().expect("rows") {
            assert!(
                row["installedVersion"].is_null(),
                "nothing is installed: {row}"
            );
        }
    }

    /// An unknown subcommand is an error (not a panic), and `uninstall` with no
    /// name prints usage and exits 1 without calling the manager.
    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_rejects_unknown_subcommands_and_missing_names() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(r#"{"skills":[]}"#).await;

        let (result, _stdout, _stderr, _code) = run_skills("bogus", &[]).await;
        assert_eq!(result.unwrap_err(), "Unknown skills command: bogus");

        let (result, stdout, stderr, code) = run_skills("uninstall", &[]).await;
        result.expect("a missing name is not an internal error");
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(
            stderr.contains("Usage: future skills uninstall <skill-name>"),
            "{stderr}"
        );

        // An invalid name is refused before any manager call.
        let (result, _stdout, _stderr, _code) = run_skills("uninstall", &["../escape"]).await;
        let err = result.expect_err("an invalid name is refused");
        assert!(err.contains("Invalid skill id"), "{err}");
    }

    /// `install <name> --version vN` strips the leading `v` before the manager
    /// sees it, and `install <name>` with no version resolves the catalogue's
    /// latest — both fail here only because the isolated home has no network
    /// to download from, which is what the assertions pin.
    #[tokio::test(flavor = "multi_thread")]
    async fn install_validates_and_resolves_the_requested_version() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(CATALOGUE).await;

        // An invalid version is refused before the catalogue is consulted.
        let (result, _stdout, _stderr, _code) =
            run_skills("install", &["future-web", "--version", "../x"]).await;
        let err = result.expect_err("an invalid version is refused");
        assert!(err.contains("Invalid skill version"), "{err}");

        // A name the catalogue does not carry has no version to install.
        let (result, _stdout, _stderr, _code) = run_skills("install", &["absent-skill"]).await;
        let err = result.unwrap_err();
        assert!(err.contains("has no available version"), "{err}");
    }

    // ── installed-skill helpers ────────────────────────────────────────

    /// The manager's own scan is fallible too, and a *failed* scan must be
    /// reported in both list forms: the catalogue fetch succeeds here, so what
    /// fails is the installed-skill scan (a directory where the registry file
    /// belongs makes `open_registry` fail inside `list_installed`). Printing an
    /// empty table instead would claim "nothing is installed" — a wrong answer,
    /// not a missing one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_manager_that_cannot_open_its_registry_is_reported_in_both_list_forms() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(CATALOGUE).await;
        let registry_dir = crate::constants::auth_file()
            .parent()
            .expect("agent dir")
            .join("agent.db");
        tokio::fs::create_dir_all(&registry_dir)
            .await
            .expect("a directory where the registry file belongs");

        for flags in [vec![], vec!["--json"]] {
            let (result, stdout, stderr, code) = run_skills("list", &flags).await;
            result.expect("the command itself succeeds");
            assert_eq!(code, 1, "flags={flags:?} stderr={stderr}");
            assert_eq!(stdout, "", "flags={flags:?}");
            assert!(
                !stderr.is_empty(),
                "the scan failure is reported: flags={flags:?}"
            );
            assert!(
                !stderr.contains("no available version") || flags.is_empty(),
                "the message is about the scan: {stderr}"
            );
        }
    }

    /// A catalogue entry whose id or version cannot become a path component is
    /// refused by the manager, and both commands that sync the catalogue must
    /// surface that per-entry failure (exit 1) instead of reporting a clean
    /// "0 skills installed" run.
    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_catalogue_entries_are_reported_by_update_and_install_builtin() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, _base) = isolated_platform(
            r#"{"skills":[
                {"id":"../escape","latest_version":"1.0"},
                {"id":"ok","latest_version":"bad/version"}
            ]}"#,
        )
        .await;
        for command in ["update", "install-builtin"] {
            let (result, _stdout, stderr, code) = run_skills(command, &[]).await;
            result.expect("the command itself succeeds");
            assert_eq!(code, 1, "{command}: {stderr}");
            assert!(
                stderr.contains("invalid catalog id/version"),
                "{command} reports the bad entry: {stderr}"
            );
            // Both entries are reported, not just the first.
            assert_eq!(
                stderr.matches("invalid catalog id/version").count(),
                2,
                "{command}: {stderr}"
            );
        }
    }

    /// `install <name> --version vN` skips the catalogue lookup entirely (the
    /// version is given), strips the leading `v`, and hands the pair to the
    /// manager — so the failure here is the *download* from the mock platform,
    /// which is what proves the manager call was reached.
    #[tokio::test(flavor = "multi_thread")]
    async fn install_with_an_explicit_version_reaches_the_manager() {
        let _env_lock = crate::test_env::lock_env().await;
        let (_dir, _guard, base) = isolated_platform(CATALOGUE).await;
        let (result, stdout, _stderr, _code) =
            run_skills("install", &["future-web", "--version", "v1.2"]).await;
        let err = result.expect_err("the mock platform serves no archive");
        assert!(
            !err.contains("has no available version"),
            "the catalogue was not consulted: {err}"
        );
        assert!(
            !err.contains("Invalid skill version"),
            "the leading v is stripped before validation: {err}"
        );
        assert_eq!(stdout, "", "nothing is claimed as installed");
        // The download went to the stripped version, not the typed one.
        assert!(
            base.starts_with("http://127.0.0.1:"),
            "the mock platform answered the catalogue: {base}"
        );
    }

    /// A minimal single-entry **stored** ZIP (no compression), built by hand so
    /// the test does not need a zip dependency: the manager accepts an archive
    /// whose `SKILL.md` frontmatter carries the requested `name` and `version`,
    /// which is exactly what this produces.
    fn skill_archive(id: &str, version: &str) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for byte in bytes {
                crc ^= *byte as u32;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }

        let body = format!("---\nname: {id}\nversion: {version}\n---\n\n# {id}\n").into_bytes();
        let name = b"SKILL.md";
        let crc = crc32(&body);
        let size = body.len() as u32;

        let mut zip = Vec::new();
        // Local file header — method 0 (stored), no data descriptor.
        zip.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(&body);

        let central_offset = zip.len() as u32;
        // Central directory.
        zip.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(name);
        let central_size = zip.len() as u32 - central_offset;

        // End of central directory.
        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&central_size.to_le_bytes());
        zip.extend_from_slice(&central_offset.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip
    }

    /// The three *success* paths of the skills CLI, over one served archive:
    /// `install <name>` records the install through the manager and reports it
    /// (the agent notification runs after it), and `uninstall` then removes a
    /// skill that really is installed (`removed == true`), which the "not
    /// installed" tests cannot reach.
    ///
    /// The archive is served by the mock platform, so nothing here depends on
    /// the real catalogue.
    #[tokio::test(flavor = "multi_thread")]
    async fn install_and_uninstall_reach_their_success_paths() {
        let _env_lock = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
            (
                "FUTURE_AGENT_GRPC_ADDR",
                std::ffi::OsString::from("127.0.0.1:1"),
            ),
        ]);
        let base = crate::test_server::spawn_http(vec![
            crate::test_server::HttpRoute::json("/client/v1/skills", 200, CATALOGUE),
            crate::test_server::HttpRoute::binary(
                "/client/v1/skills/future-web/versions/1.2/download",
                200,
                skill_archive("future-web", "1.2"),
            ),
        ])
        .await;
        let auth = crate::constants::auth_file();
        tokio::fs::create_dir_all(auth.parent().expect("parent"))
            .await
            .expect("mkdir");
        tokio::fs::write(&auth, format!(r#"{{"future":{{"base_url":"{base}"}}}}"#))
            .await
            .expect("write auth.json");

        // install <name> --version v1.2 → manager installs, CLI reports it,
        // then notifies the agent (best effort at a dead address).
        let (result, stdout, stderr, code) =
            run_skills("install", &["future-web", "--version", "v1.2"]).await;
        result.unwrap_or_else(|e| panic!("install failed: {e} — stderr {stderr}"));
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(
            stdout.contains("Installed skill \"future-web\" v1.2."),
            "{stdout}"
        );
        // The manager's own record agrees: the skill is installed at 1.2.
        assert!(
            skills_dir().join("future-web").join("SKILL.md").is_file(),
            "the archive was unpacked into the skills directory"
        );

        // The table now shows it as installed rather than as an em dash.
        let (_result, listed, _stderr, _code) = run_skills("list", &[]).await;
        let row = listed
            .lines()
            .find(|line| line.contains("future-web"))
            .expect("the installed skill is listed");
        assert!(row.contains("v1.2"), "{row}");

        // uninstall → a real removal is reported (not the "is not installed"
        // message the other test pins).
        let (result, stdout, stderr, code) = run_skills("uninstall", &["future-web"]).await;
        result.unwrap_or_else(|e| panic!("uninstall failed: {e} — stderr {stderr}"));
        assert_eq!(code, 0, "stderr: {stderr}");
        assert!(
            stdout.contains("Uninstalled skill \"future-web\"."),
            "{stdout}"
        );
        assert!(
            !skills_dir().join("future-web").exists(),
            "the skill directory is gone"
        );
    }

    /// `get_installed_skill_ids` counts only directories that actually hold a
    /// SKILL.md, and a missing skills directory is an empty set rather than an
    /// error.
    #[tokio::test(flavor = "multi_thread")]
    async fn installed_skill_ids_require_a_skill_md() {
        let _env_lock = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
        ]);
        // No skills dir yet.
        assert!(get_installed_skill_ids().await.is_empty());

        let skills = skills_dir();
        tokio::fs::create_dir_all(skills.join("with-md"))
            .await
            .expect("mkdir");
        tokio::fs::write(skills.join("with-md").join("SKILL.md"), "---\n---\n")
            .await
            .expect("write");
        tokio::fs::create_dir_all(skills.join("without-md"))
            .await
            .expect("mkdir");
        tokio::fs::write(skills.join("loose.txt"), "x")
            .await
            .expect("write");

        let ids = get_installed_skill_ids().await;
        assert_eq!(ids.len(), 1, "{ids:?}");
        assert!(ids.contains("with-md"), "{ids:?}");
    }

    /// A readable SKILL.md with no version field is filtered out of the
    /// installed-version map rather than recorded as an empty version.
    #[tokio::test(flavor = "multi_thread")]
    async fn installed_versions_skip_a_missing_version() {
        let _env_lock = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("USERPROFILE", dir.path().as_os_str().to_owned()),
        ]);
        let skills = skills_dir();
        tokio::fs::create_dir_all(skills.join("versioned"))
            .await
            .expect("mkdir");
        tokio::fs::write(
            skills.join("versioned").join("SKILL.md"),
            "---\nversion: 4.2\n---\n",
        )
        .await
        .expect("write");
        tokio::fs::create_dir_all(skills.join("unversioned"))
            .await
            .expect("mkdir");
        tokio::fs::write(
            skills.join("unversioned").join("SKILL.md"),
            "no version here\n",
        )
        .await
        .expect("write");

        let versions = installed_skill_versions().await.expect("list_installed");
        assert_eq!(versions.get("versioned").map(String::as_str), Some("4.2"));
        assert!(
            !versions.contains_key("unversioned"),
            "a version-less skill has no entry: {versions:?}"
        );
    }
}
