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

/// Validate a skill id before it is ever joined onto a filesystem path. The id
/// comes from the (unauthenticated) platform catalogue, so an id like `../x` or
/// an absolute path would let install/uninstall escape the skills directory and
/// `remove_dir_all` an arbitrary target. Allow only a conservative slug charset.
fn is_skill_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        // `.` is allowed for versioned names but never as a path-traversal `..`.
        && !id.contains("..")
}

fn ensure_skill_id_ok(id: &str) -> Result<(), AppError> {
    if is_skill_id_ok(id) {
        Ok(())
    } else {
        Err(AppError::Message(format!("Invalid skill id: {id:?}")))
    }
}

/// Join a validated skill id onto a scope directory and assert the result stays
/// inside that scope — defence in depth on top of [`ensure_skill_id_ok`].
fn skill_dir_in_scope(scope: SkillScope, id: &str) -> Result<PathBuf, AppError> {
    ensure_skill_id_ok(id)?;
    join_skill_dir(scope.dir()?, id)
}

/// Join `id` onto `base`, refusing any result that escapes `base` (defence in
/// depth: [`ensure_skill_id_ok`] already rejects separators and `..`, so a future
/// relaxation of that validator must still be unable to traverse out of scope).
fn join_skill_dir(base: PathBuf, id: &str) -> Result<PathBuf, AppError> {
    let dest = base.join(id);
    if dest.parent() != Some(base.as_path()) {
        return Err(AppError::Message(format!("Invalid skill id: {id:?}")));
    }
    Ok(dest)
}

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
    #[serde(default, alias = "name_zh")]
    pub name_zh: String,
    #[serde(default, alias = "description_zh")]
    pub description_zh: String,
    #[serde(default)]
    pub category: String,
    #[serde(default, alias = "category_zh")]
    pub category_zh: String,
    #[serde(default, alias = "latest_version")]
    pub latest_version: Option<String>,
}

fn http_client() -> reqwest::Client {
    // `Client::builder().timeout().build()` only fails for an invalid config;
    // the default config here is constant, so a failure is an invariant break.
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("default reqwest client config cannot fail to build")
}

/// The platform skill catalogue (`GET /client/v1/skills`). Unauthenticated, like
/// the CLI's `fetchSkills`.
pub async fn list_available_skills() -> Result<Vec<SkillInfo>, AppError> {
    #[derive(Deserialize)]
    struct CatalogueResponse {
        #[serde(default)]
        skills: Vec<SkillInfo>,
    }

    let url = format!(
        "{}/client/v1/skills",
        crate::future_platform::current_platform_url()
    );
    let response = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Failed to fetch skill list: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "Failed to fetch skill list (HTTP {})",
            response.status().as_u16()
        )));
    }
    let parsed: CatalogueResponse = response
        .json()
        .await
        .map_err(|error| AppError::Message(format!("Failed to parse skill list: {error}")))?;
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
    let dest = skill_dir_in_scope(SkillScope::App, &id)?;
    // `version` is interpolated into the URL path below — hold it to the same
    // slug charset as `id` so `/../`, `?` or `#` can't reroute the request to
    // another endpoint on the platform host.
    if !is_skill_id_ok(&version) {
        return Err(AppError::Message(format!(
            "Invalid skill version: {version:?}."
        )));
    }
    let url = format!(
        "{}/client/v1/skills/{}/versions/{}/download",
        crate::future_platform::current_platform_url(),
        id,
        version
    );
    let response = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Failed to download skill: {error}")))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::Message(format!(
            "Skill version not found: {id}@{version}."
        )));
    }
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "Skill download failed (HTTP {})",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::Message(format!("Failed to read skill data: {error}")))?;

    // Unzip + filesystem work is blocking; keep it off the async runtime.
    tokio::task::spawn_blocking(move || extract_skill_zip(&bytes, &dest))
        .await
        .map_err(|error| AppError::Message(format!("Install task failed: {error}")))?
}

/// Remove skill `id` from every scope it's installed in. Returns whether any
/// directory was removed.
pub fn uninstall_skill(id: &str) -> Result<bool, AppError> {
    ensure_skill_id_ok(id)?;
    let mut removed = false;
    for scope in SCOPES {
        let dest = skill_dir_in_scope(scope, id)?;
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
        .map_err(|error| AppError::Message(format!("Skill package is not a valid zip: {error}")))?;
    archive
        .extract(dest)
        .map_err(|error| AppError::Message(format!("Failed to extract skill: {error}")))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_and_absolute_ids() {
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "../../Documents",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "foo/../bar",
            "with space",
            "emoji😀",
        ] {
            assert!(!is_skill_id_ok(bad), "should reject {bad:?}");
            assert!(ensure_skill_id_ok(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn accepts_slug_ids() {
        for ok in ["core", "rare-disease", "gene_variant", "skill.v2", "a1b2"] {
            assert!(is_skill_id_ok(ok), "should accept {ok:?}");
        }
    }

    #[test]
    fn skill_dir_in_scope_stays_inside_base() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-dir");
        let base = SkillScope::App.dir().unwrap();
        let dest = skill_dir_in_scope(SkillScope::App, "core").unwrap();
        assert_eq!(dest, base.join("core"));
        assert!(skill_dir_in_scope(SkillScope::App, "../escape").is_err());
    }

    #[test]
    fn join_skill_dir_refuses_traversal() {
        // The defence-in-depth join check fires when the id (bypassing
        // ensure_skill_id_ok) still escapes the base directory.
        let base = PathBuf::from("/scope");
        assert!(join_skill_dir(base.clone(), "../escape").is_err());
        assert_eq!(
            join_skill_dir(base.clone(), "core").unwrap(),
            base.join("core")
        );
    }

    #[test]
    fn global_scope_dir_is_home_rooted() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-global");
        let dir = SkillScope::Global.dir().unwrap();
        assert!(dir.ends_with(".agents/skills"));
    }

    fn write_skill(dir: &Path, id: &str, frontmatter: &str) {
        std::fs::create_dir_all(dir.join(id)).unwrap();
        std::fs::write(dir.join(id).join("SKILL.md"), frontmatter).unwrap();
    }

    #[test]
    fn installed_versions_empty_when_no_scopes_exist() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-empty");
        // Fresh HOME → neither scope dir exists → read_dir fails → skipped.
        assert!(installed_versions().is_empty());
    }

    #[test]
    fn installed_versions_scans_both_scopes() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-installed");
        let app = SkillScope::App.dir().unwrap();
        let global = SkillScope::Global.dir().unwrap();
        write_skill(&app, "foo", "---\nversion: \"1.2.3\"\n---\n");
        write_skill(&app, "noversion", "no frontmatter");
        write_skill(&global, "bar", "---\nversion: '4.5.6'\n---\n");
        std::fs::write(app.join("notadir"), "x").unwrap();

        let versions = installed_versions();
        assert_eq!(versions.get("foo").unwrap().as_deref(), Some("1.2.3"));
        assert_eq!(versions.get("noversion").unwrap().as_deref(), None);
        assert_eq!(versions.get("bar").unwrap().as_deref(), Some("4.5.6"));
        assert!(!versions.contains_key("notadir"));
    }

    #[test]
    fn uninstall_removes_from_all_scopes() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-uninstall");
        let app = SkillScope::App.dir().unwrap();
        write_skill(&app, "foo", "x");

        assert!(uninstall_skill("foo").unwrap());
        assert!(!app.join("foo").exists());
        assert!(!uninstall_skill("foo").unwrap()); // not there anymore
        assert!(uninstall_skill("../evil").is_err());
    }

    fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (name, data) in files {
                writer.start_file(*name, options).unwrap();
                std::io::Write::write_all(&mut writer, data).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn extract_skill_zip_writes_and_flattens() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-extract");
        let dest = SkillScope::App.dir().unwrap().join("installed");
        std::fs::create_dir_all(&dest).unwrap(); // pre-existing → cleared on install
        let bytes = make_zip(&[
            ("wrapper/SKILL.md", b"---\nversion: 1.0.0\n---\n"),
            ("wrapper/lib/util.js", b"// code"),
        ]);
        extract_skill_zip(&bytes, &dest).unwrap();
        assert!(dest.join("SKILL.md").exists());
        assert!(dest.join("lib/util.js").exists());
        assert!(!dest.join("wrapper").exists());
    }

    #[test]
    fn extract_skill_zip_rejects_non_zip() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-badzip");
        let dest = SkillScope::App.dir().unwrap().join("bad");
        assert!(extract_skill_zip(b"not a zip", &dest).is_err());
    }

    #[test]
    fn flatten_single_subdir_flattens_one_level() {
        let dir = std::env::temp_dir().join(format!("futureos-flatten-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("wrapper")).unwrap();
        std::fs::write(dir.join("wrapper/SKILL.md"), "x").unwrap();
        flatten_single_subdir(&dir).unwrap();
        assert!(dir.join("SKILL.md").exists());
        assert!(!dir.join("wrapper").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flatten_single_subdir_noops_when_not_a_single_dir() {
        let dir = std::env::temp_dir().join(format!("futureos-flatten2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        flatten_single_subdir(&dir).unwrap(); // empty → no-op
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        flatten_single_subdir(&dir).unwrap(); // one non-dir entry → no-op
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_skill_md_version_parses_frontmatter() {
        let dir = std::env::temp_dir().join(format!("futureos-skillmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");

        assert_eq!(read_skill_md_version(&dir.join("missing.md")), None);

        std::fs::write(&path, "no frontmatter here").unwrap();
        assert_eq!(read_skill_md_version(&path), None);

        std::fs::write(&path, "---\nversion: 1.0.0\n").unwrap();
        assert_eq!(read_skill_md_version(&path), None); // unterminated

        std::fs::write(&path, "---\nversion: \"1.2.3\"\n---\n").unwrap();
        assert_eq!(read_skill_md_version(&path).as_deref(), Some("1.2.3"));

        std::fs::write(&path, "---\n# comment\nversion: 2.0.0\n---\n").unwrap();
        assert_eq!(read_skill_md_version(&path).as_deref(), Some("2.0.0"));

        std::fs::write(&path, "---\nversion: \"\"\n---\n").unwrap();
        assert_eq!(read_skill_md_version(&path), None); // empty version

        std::fs::write(&path, "---\nname: my-skill\n---\n").unwrap();
        assert_eq!(read_skill_md_version(&path), None); // no version line

        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── async catalogue + install against a mock HTTP server ─────────────

    /// Thread-based mock HTTP server answering each request with the next
    /// canned (status, content-type, body) response. Returns the base URL.
    fn mock_http_server(responses: Vec<(u16, &'static str, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for (status, content_type, body) in responses {
                let (mut stream, _) = listener.accept().expect("mock accept");
                let mut sink = [0u8; 8192];
                let _ = stream.read(&mut sink);
                let header = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn point_auth_at(url: &str) {
        crate::auth_store::set_future_base_url(&format!("{url}/api")).unwrap();
    }

    #[tokio::test]
    async fn list_available_skills_parses_catalogue() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-list");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            br#"{"skills":[{"id":"core","name":"Core","latest_version":"1.0.0"}]}"#.to_vec(),
        )]);
        point_auth_at(&url);
        let skills = list_available_skills().await.unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "core");
        assert_eq!(skills[0].latest_version.as_deref(), Some("1.0.0"));
    }

    #[tokio::test]
    async fn list_available_skills_http_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-list-500");
        let url = mock_http_server(vec![(500, "application/json", b"{}".to_vec())]);
        point_auth_at(&url);
        let err = list_available_skills().await.unwrap_err();
        assert!(err.to_string().contains("HTTP 500"));
    }

    #[tokio::test]
    async fn list_available_skills_parse_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-list-badjson");
        let url = mock_http_server(vec![(200, "application/json", b"not json".to_vec())]);
        point_auth_at(&url);
        let err = list_available_skills().await.unwrap_err();
        assert!(err.to_string().contains("Failed to parse"));
    }

    #[tokio::test]
    async fn list_available_skills_network_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-list-net");
        crate::auth_store::set_future_base_url("http://127.0.0.1:1/api").unwrap();
        let err = list_available_skills().await.unwrap_err();
        assert!(err.to_string().contains("Failed to fetch"));
    }

    #[tokio::test]
    async fn install_skill_rejects_bad_version() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-install-badver");
        let err = install_skill("core".to_string(), "../evil".to_string())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Invalid skill version"));
    }

    #[tokio::test]
    async fn install_skill_version_not_found() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-install-404");
        let url = mock_http_server(vec![(404, "application/json", b"{}".to_vec())]);
        point_auth_at(&url);
        let err = install_skill("core".to_string(), "9.9.9".to_string())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn install_skill_http_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-install-500");
        let url = mock_http_server(vec![(500, "application/json", b"{}".to_vec())]);
        point_auth_at(&url);
        let err = install_skill("core".to_string(), "1.0.0".to_string())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("download failed"));
    }

    #[tokio::test]
    async fn install_skill_downloads_and_extracts() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-install-ok");
        let zip_bytes = make_zip(&[("skill/SKILL.md", b"---\nversion: 1.0.0\n---\n")]);
        let url = mock_http_server(vec![(200, "application/zip", zip_bytes)]);
        point_auth_at(&url);
        install_skill("core".to_string(), "1.0.0".to_string())
            .await
            .unwrap();
        let dest = SkillScope::App.dir().unwrap().join("core");
        assert!(dest.join("SKILL.md").exists());
    }

    #[tokio::test]
    async fn install_skill_network_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("skills-install-net");
        crate::auth_store::set_future_base_url("http://127.0.0.1:1/api").unwrap();
        let err = install_skill("core".to_string(), "1.0.0".to_string())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to download"));
    }
}
