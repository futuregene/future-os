//! Skill marketplace + local install/uninstall for the GUI's Skills panel.
//!
//! Reimplements the CLI's skill logic (`cli/src/commands/skills.ts`) in Rust,
//! independent of the CLI: the *available* list comes from the Future platform
//! (`GET {platform}/client/v1/skills`); install downloads and unpacks a version
//! zip into a local skill directory; uninstall removes it. The *installed* list
//! shown in the UI comes from the agent's `get_commands` (see
//! [`crate::agent_bridge::list_installed_skills`]), not from here — this module
//! only supplies version enrichment and the filesystem mutations.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Install scopes mirroring the CLI's home-rooted ones. The GUI panel is a
/// global manager, so the cwd-relative `project` scope isn't offered; install
/// always targets `app`, and uninstall sweeps every scope.
#[derive(Debug, Clone, Copy)]
enum SkillScope {
    App,
    Global,
}

const SCOPES: [SkillScope; 2] = [SkillScope::App, SkillScope::Global];

impl SkillScope {
    fn dir(self) -> Result<PathBuf, AppError> {
        match self {
            // ~/.future/agent/skills — the canonical app scope.
            SkillScope::App => Ok(crate::auth_store::agent_dir()?.join("skills")),
            // ~/.agents/skills — shared with other agent tooling.
            SkillScope::Global => {
                let home =
                    crate::home_dir().ok_or("HOME/USERPROFILE environment variable is not set.")?;
                Ok(PathBuf::from(home).join(".agents").join("skills"))
            }
        }
    }
}

/// One entry from the platform skill catalogue. Snake-case `latest_version` from
/// the server is accepted via an alias while the struct serializes camelCase to
/// the frontend.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, alias = "latest_version")]
    pub latest_version: Option<String>,
}

fn platform_url() -> String {
    let auth = Value::Object(crate::auth_store::read().unwrap_or_default());
    crate::agent_providers::resolve_future_platform_url(&auth)
}

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| AppError::Message(format!("无法创建 HTTP 客户端：{error}")))
}

/// The platform skill catalogue (`GET /client/v1/skills`). Unauthenticated, like
/// the CLI's `fetchSkills`.
pub async fn list_available_skills() -> Result<Vec<SkillInfo>, AppError> {
    #[derive(Deserialize)]
    struct CatalogueResponse {
        #[serde(default)]
        skills: Vec<SkillInfo>,
    }

    let url = format!("{}/client/v1/skills", platform_url());
    let response = http_client()?
        .get(&url)
        .send()
        .await
        .map_err(|error| AppError::Message(format!("获取技能列表失败：{error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "获取技能列表失败（HTTP {}）",
            response.status().as_u16()
        )));
    }
    let parsed: CatalogueResponse = response
        .json()
        .await
        .map_err(|error| AppError::Message(format!("解析技能列表失败：{error}")))?;
    Ok(parsed.skills)
}

/// Map of installed skill id → version, scanned across scopes. The id is the
/// install directory name (equal to the catalogue id and the SKILL.md `name`).
/// Used to enrich the agent-sourced installed list and to flag catalogue items.
pub fn installed_versions() -> BTreeMap<String, Option<String>> {
    let mut versions = BTreeMap::new();
    for scope in SCOPES {
        let Ok(dir) = scope.dir() else { continue };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            let version = read_skill_md_version(&path.join("SKILL.md"));
            // First scope wins (app before global), matching discovery order.
            versions.entry(id).or_insert(version);
        }
    }
    versions
}

/// Download and unpack skill `id`@`version` into the app scope.
pub async fn install_skill(id: String, version: String) -> Result<(), AppError> {
    let url = format!(
        "{}/client/v1/skills/{}/versions/{}/download",
        platform_url(),
        id,
        version
    );
    let response = http_client()?
        .get(&url)
        .send()
        .await
        .map_err(|error| AppError::Message(format!("下载技能失败：{error}")))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::Message(format!(
            "未找到技能版本 {id}@{version}。"
        )));
    }
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "下载技能失败（HTTP {}）",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::Message(format!("读取技能数据失败：{error}")))?;

    let dest = SkillScope::App.dir()?.join(&id);
    // Unzip + filesystem work is blocking; keep it off the async runtime.
    tokio::task::spawn_blocking(move || extract_skill_zip(&bytes, &dest))
        .await
        .map_err(|error| AppError::Message(format!("安装任务失败：{error}")))?
}

/// Remove skill `id` from every scope it's installed in. Returns whether any
/// directory was removed.
pub fn uninstall_skill(id: &str) -> Result<bool, AppError> {
    let mut removed = false;
    for scope in SCOPES {
        let dest = scope.dir()?.join(id);
        if dest.is_dir() {
            std::fs::remove_dir_all(&dest)?;
            removed = true;
        }
    }
    Ok(removed)
}

fn extract_skill_zip(bytes: &[u8], dest: &Path) -> Result<(), AppError> {
    // Fresh install/update: clear any prior contents so removed files don't linger.
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::create_dir_all(dest)?;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|error| AppError::Message(format!("技能包不是有效的 zip：{error}")))?;
    archive
        .extract(dest)
        .map_err(|error| AppError::Message(format!("解压技能失败：{error}")))?;

    // Some zips wrap everything in a single top-level directory; flatten it so
    // SKILL.md lands at the skill root (matches the CLI).
    flatten_single_subdir(dest)?;
    Ok(())
}

fn flatten_single_subdir(dir: &Path) -> Result<(), AppError> {
    let entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    if entries.len() != 1 || !entries[0].path().is_dir() {
        return Ok(());
    }
    let single = entries[0].path();
    for child in std::fs::read_dir(&single)?.filter_map(Result::ok) {
        let target = dir.join(child.file_name());
        let _ = std::fs::remove_dir_all(&target);
        let _ = std::fs::remove_file(&target);
        // Same skills dir → same filesystem, so a plain rename suffices.
        std::fs::rename(child.path(), &target)?;
    }
    std::fs::remove_dir_all(&single)?;
    Ok(())
}

/// Extract the `version:` field from a SKILL.md YAML frontmatter block, if any.
fn read_skill_md_version(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let after = text.trim_start().strip_prefix("---")?;
    let end = after.find("\n---")?;
    for line in after[..end].lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix("version:") {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}
