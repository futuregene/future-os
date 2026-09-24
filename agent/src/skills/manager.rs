//! The host-local skill manager. The CLI and the Agent RPC use this same
//! implementation; clients never patch the skill registry themselves.

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use reqwest::blocking::Client;
use rusqlite::{params, Connection, OptionalExtension};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::registry;

const MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
const RECEIPT: &str = ".future-install.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueSkill {
    pub id: String,
    #[serde(default, alias = "latest_version")]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub builtin: bool,
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
    #[serde(default, skip_deserializing)]
    pub upgrade_available: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstalledSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "nameZh")]
    pub name_zh: Option<String>,
    #[serde(rename = "descriptionZh")]
    pub description_zh: Option<String>,
    pub version: Option<String>,
    pub scope: String,
    pub source: String,
    pub location: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct SyncResult {
    pub installed: Vec<String>,
    pub upgraded: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Receipt {
    id: String,
    version: String,
    package_sha256: String,
}

pub struct SkillManager {
    agent_dir: PathBuf,
    global_dir: PathBuf,
    platform_url: String,
    client: Client,
}

impl SkillManager {
    pub fn local() -> Result<Self> {
        let future_home = crate::utils::future_home();
        let agent_dir = future_home.join("agent");
        let global_dir = crate::utils::home_dir().join(".agents").join("skills");
        let platform_url = platform_url(&agent_dir);
        Self::new(agent_dir, global_dir, platform_url)
    }

    pub fn new(agent_dir: PathBuf, global_dir: PathBuf, platform_url: String) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            agent_dir,
            global_dir,
            platform_url,
            client,
        })
    }

    fn app_dir(&self) -> PathBuf {
        self.agent_dir.join("skills")
    }
    fn db_path(&self) -> PathBuf {
        self.agent_dir.join("agent.db")
    }

    /// A file lock covers both Agent RPC and one-shot CLI processes. Operations
    /// also hold it while publishing their refreshed discovery snapshot.
    fn locked<T>(&self, action: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        fs::create_dir_all(&self.agent_dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(self.agent_dir.join(".skills.lock"))?;
        lock.lock_exclusive()?;
        let mut db = registry::open_registry(&self.db_path())?;
        let result = action(&mut db);
        let _ = lock.unlock();
        result
    }

    pub fn catalogue(&self) -> Result<Vec<CatalogueSkill>> {
        #[derive(Deserialize)]
        struct Response {
            #[serde(default)]
            skills: Vec<CatalogueSkill>,
        }
        let url = format!("{}/client/v1/skills", self.platform_url);
        Ok(self
            .client
            .get(url)
            .send()?
            .error_for_status()?
            .json::<Response>()?
            .skills)
    }

    pub fn catalogue_with_status(&self) -> Result<Vec<CatalogueSkill>> {
        let mut catalogue = self.catalogue()?;
        self.locked(|db| {
            self.recover(db)?;
            self.reconcile_once(db)?;
            for skill in &mut catalogue {
                let installed: Option<(String, Option<String>)> = db
                    .query_row(
                        "SELECT source,version FROM skill_installations WHERE name=?1
                     ORDER BY CASE scope WHEN 'app' THEN 0 ELSE 1 END LIMIT 1",
                        [&skill.id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                skill.upgrade_available = matches!(
                    (installed, skill.latest_version.as_deref()),
                    (Some((source, Some(current))), Some(latest))
                        if source == "managed" && newer(latest, &current)
                );
            }
            Ok(catalogue)
        })
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledSkill>> {
        self.locked(|db| {
            self.recover(db)?;
            self.reconcile_once(db)?;
            let mut statement = db.prepare(
                "SELECT name,version,scope,source,location FROM skill_installations
                 ORDER BY name, CASE scope WHEN 'app' THEN 0 ELSE 1 END",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut seen = std::collections::HashSet::new();
            Ok(rows
                .into_iter()
                .filter_map(|(id, version, scope, source, location)| {
                    if !seen.insert(id.clone()) {
                        return None;
                    }
                    let content = fs::read_to_string(Path::new(&location).join("SKILL.md")).ok()?;
                    Some(InstalledSkill {
                        name: id.clone(),
                        id,
                        version,
                        scope,
                        source,
                        location,
                        description: super::extract_frontmatter_field(&content, "description")
                            .unwrap_or_default(),
                        name_zh: super::extract_frontmatter_field(&content, "name_zh"),
                        description_zh: super::extract_frontmatter_field(
                            &content,
                            "description_zh",
                        ),
                    })
                })
                .collect())
        })
    }

    pub fn install(&self, id: &str, version: &str) -> Result<()> {
        validate_component(id)?;
        validate_component(version)?;
        self.locked(|db| {
            self.recover(db)?;
            self.reconcile(db)?;
            if let Err(error) = self.install_locked(db, id, version) {
                self.recover(db)?;
                return Err(error);
            }
            self.reconcile(db)?;
            super::invalidate_skills_cache();
            Ok(())
        })
    }

    pub fn uninstall(&self, id: &str) -> Result<bool> {
        validate_component(id)?;
        self.locked(|db| {
            self.recover(db)?;
            self.reconcile(db)?;
            db.execute("INSERT INTO skill_operations(name,kind,version,phase,started_at_ms)
                VALUES(?1,'uninstall',NULL,'prepared',?2)
                ON CONFLICT(name) DO UPDATE SET kind='uninstall',version=NULL,phase='prepared',started_at_ms=excluded.started_at_ms",
                params![id,now_ms()])?;
            let removed = self.finish_uninstall(db, id)?;
            self.reconcile(db)?;
            super::invalidate_skills_cache();
            Ok(removed)
        })
    }

    fn finish_uninstall(&self, db: &mut Connection, id: &str) -> Result<bool> {
        let mut statement = db.prepare("SELECT location FROM skill_installations WHERE name=?1")?;
        let paths = statement
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut removed = false;
        for location in paths {
            let path = Path::new(&location);
            if path.is_dir() {
                fs::remove_dir_all(path)?;
                removed = true;
            }
        }
        let tx = crate::session::database::begin_immediate(db)?;
        tx.execute(
            "INSERT INTO skills(name,version,deleted,installed_at_ms,updated_at_ms)
            VALUES(?1,NULL,1,NULL,?2)
            ON CONFLICT(name) DO UPDATE SET deleted=1,updated_at_ms=excluded.updated_at_ms",
            params![id, now_ms()],
        )?;
        tx.execute("DELETE FROM skill_installations WHERE name=?1", [id])?;
        tx.execute("DELETE FROM skill_operations WHERE name=?1", [id])?;
        tx.commit()?;
        Ok(removed)
    }

    /// One idempotent pass: upgrade managed installs and install catalogued
    /// builtins with no installation and no install/uninstall history.
    pub fn sync(&self, auto_upgrade: bool) -> Result<SyncResult> {
        if !auto_upgrade {
            return Ok(SyncResult::default());
        }
        self.sync_inner(true)
    }

    /// Explicit bootstrap adds only builtin skills that have never been seen.
    pub fn install_missing_builtins(&self) -> Result<SyncResult> {
        self.sync_inner(false)
    }

    fn sync_inner(&self, upgrade_existing: bool) -> Result<SyncResult> {
        let catalogue = self.catalogue()?;
        self.locked(|db| {
            self.recover(db)?;
            self.reconcile(db)?;
            let mut result = SyncResult::default();
            for skill in &catalogue {
                let Some(latest) = skill.latest_version.as_deref().filter(|v| !v.is_empty()) else {
                    result.skipped.push(skill.id.clone());
                    continue;
                };
                if validate_component(&skill.id).is_err() || validate_component(latest).is_err() {
                    result.failed.push(format!("{}: invalid catalog id/version", skill.id));
                    continue;
                }
                let installed: Option<(String, Option<String>)> = db.query_row(
                    "SELECT source,version FROM skill_installations WHERE name=?1 ORDER BY CASE scope WHEN 'app' THEN 0 ELSE 1 END LIMIT 1",
                    [skill.id.as_str()], |row| Ok((row.get(0)?, row.get(1)?)),
                ).optional()?;
                let prior: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM skills WHERE name=?1)",
                    [skill.id.as_str()], |row| row.get(0))?;
                let action = match installed {
                    Some((source, Some(current))) if upgrade_existing && source == "managed" && newer(latest, &current) => Some("upgrade"),
                    None if skill.builtin && !prior => Some("install"),
                    _ => None,
                };
                if let Some(action) = action {
                    match self.install_locked(db, &skill.id, latest) {
                        Ok(()) if action == "upgrade" => result.upgraded.push(skill.id.clone()),
                        Ok(()) => result.installed.push(skill.id.clone()),
                        Err(error) => {
                            self.recover(db)?;
                            result.failed.push(format!("{}: {error}", skill.id));
                        }
                    }
                } else {
                    result.skipped.push(skill.id.clone());
                }
            }
            self.reconcile(db)?;
            if !result.installed.is_empty() || !result.upgraded.is_empty() {
                super::invalidate_skills_cache();
            }
            Ok(result)
        })
    }

    fn install_locked(&self, db: &mut Connection, id: &str, version: &str) -> Result<()> {
        let bytes = self.download(id, version)?;
        let package_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let staging = tempfile::Builder::new()
            .prefix(".skill-install-")
            .tempdir_in(&self.agent_dir)?;
        let candidate = staging.path().join("candidate");
        fs::create_dir(&candidate)?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes))?;
        let mut extracted_bytes = 0u64;
        if archive.len() > 4096 {
            bail!("skill archive has too many entries");
        }
        for index in 0..archive.len() {
            let entry = archive.by_index(index)?;
            if entry.enclosed_name().is_none() {
                bail!("skill archive has an unsafe path");
            }
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                bail!("skill archive contains a symbolic link");
            }
            extracted_bytes = extracted_bytes.saturating_add(entry.size());
            if extracted_bytes > MAX_EXTRACTED_BYTES {
                bail!("skill archive exceeds 256 MiB uncompressed");
            }
        }
        archive.extract(&candidate)?;
        flatten(&candidate)?;
        let content = fs::read_to_string(candidate.join("SKILL.md"))
            .context("skill archive has no SKILL.md")?;
        let actual_id = super::extract_frontmatter_field(&content, "name");
        let actual_version = super::extract_package_version(&content);
        if actual_id.as_deref() != Some(id) || actual_version.as_deref() != Some(version) {
            bail!("skill package identity/version differs from requested {id}@{version}");
        }
        let receipt = Receipt {
            id: id.to_owned(),
            version: version.to_owned(),
            package_sha256,
        };
        let mut file = File::create(candidate.join(RECEIPT))?;
        serde_json::to_writer(&mut file, &receipt)?;
        file.sync_all()?;

        let app = self.app_dir();
        fs::create_dir_all(&app)?;
        let dest = app.join(id);
        let backup = self.agent_dir.join(format!(".skill-{id}.previous"));
        db.execute(
            "INSERT INTO skill_operations(name,kind,version,phase,started_at_ms) VALUES(?1,'install',?2,'prepared',?3)
             ON CONFLICT(name) DO UPDATE SET kind='install',version=excluded.version,phase='prepared',started_at_ms=excluded.started_at_ms",
            params![id,version,now_ms()],
        )?;
        if backup.exists() {
            fs::remove_dir_all(&backup)?;
        }
        if dest.exists() {
            fs::rename(&dest, &backup)?;
        }
        if let Err(error) = fs::rename(&candidate, &dest) {
            if backup.exists() {
                fs::rename(&backup, &dest)?;
            }
            db.execute("DELETE FROM skill_operations WHERE name=?1", [id])?;
            return Err(error.into());
        }
        let finalize = db
            .execute(
                "UPDATE skill_operations SET phase='replaced' WHERE name=?1",
                [id],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
            .and_then(|_| self.finish_install(db, &receipt, &dest));
        if let Err(error) = finalize {
            if dest.exists() {
                fs::remove_dir_all(&dest)?;
            }
            if backup.exists() {
                fs::rename(&backup, &dest)?;
            }
            let _ = db.execute("DELETE FROM skill_operations WHERE name=?1", [id]);
            return Err(error);
        }
        if backup.exists() {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(())
    }

    fn finish_install(&self, db: &mut Connection, receipt: &Receipt, dest: &Path) -> Result<()> {
        let tx = crate::session::database::begin_immediate(db)?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO skills(name,version,deleted,installed_at_ms,updated_at_ms) VALUES(?1,?2,0,?3,?3)
             ON CONFLICT(name) DO UPDATE SET version=excluded.version,deleted=0,
                 installed_at_ms=COALESCE(skills.installed_at_ms,excluded.installed_at_ms),
                 updated_at_ms=excluded.updated_at_ms",
            params![receipt.id,receipt.version,now],
        )?;
        tx.execute(
            "INSERT INTO skill_installations(location,name,scope,source,version,package_sha256,observed_at_ms)
             VALUES(?1,?2,'app','managed',?3,?4,?5)
             ON CONFLICT(location) DO UPDATE SET name=excluded.name,scope='app',source='managed',version=excluded.version,package_sha256=excluded.package_sha256,observed_at_ms=excluded.observed_at_ms",
            params![dest.to_string_lossy(),receipt.id,receipt.version,receipt.package_sha256,now],
        )?;
        tx.execute("DELETE FROM skill_operations WHERE name=?1", [&receipt.id])?;
        tx.commit()?;
        Ok(())
    }

    fn recover(&self, db: &mut Connection) -> Result<()> {
        let mut statement =
            db.prepare("SELECT name,kind,version FROM skill_operations ORDER BY started_at_ms")?;
        let pending = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (id, kind, version) in pending {
            if kind == "uninstall" {
                self.finish_uninstall(db, &id)?;
                continue;
            }
            let dest = self.app_dir().join(&id);
            let backup = self.agent_dir.join(format!(".skill-{id}.previous"));
            let receipt = read_receipt(&dest);
            if let Some(receipt) =
                receipt.filter(|r| r.id == id && Some(&r.version) == version.as_ref())
            {
                self.finish_install(db, &receipt, &dest)?;
                if backup.exists() {
                    let _ = fs::remove_dir_all(&backup);
                }
            } else {
                if backup.exists() {
                    if dest.exists() {
                        fs::remove_dir_all(&dest)?;
                    }
                    fs::rename(&backup, &dest)?;
                }
                db.execute("DELETE FROM skill_operations WHERE name=?1", [&id])?;
            }
        }
        Ok(())
    }

    fn reconcile(&self, db: &mut Connection) -> Result<()> {
        let previous = {
            let mut statement = db.prepare("SELECT location,source FROM skill_installations")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?;
            rows
        };
        let legacy = if previous.is_empty() {
            let mut statement = db.prepare("SELECT name FROM skills WHERE deleted=0")?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
            rows
        } else {
            std::collections::HashSet::new()
        };
        let tx = crate::session::database::begin_immediate(db)?;
        tx.execute("DELETE FROM skill_installations", [])?;
        // App installs take precedence in both the list and the historical
        // version row when an id also exists in the global scope.
        for (scope, root) in [("global", self.global_dir.clone()), ("app", self.app_dir())] {
            if !root.exists() {
                continue;
            }
            for entry in walkdir::WalkDir::new(root).min_depth(1).max_depth(1) {
                let entry = entry?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let md = path.join("SKILL.md");
                if !md.is_file() {
                    continue;
                }
                let content = fs::read_to_string(md)?;
                let id = super::extract_frontmatter_field(&content, "name")
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
                let declared = super::extract_package_version(&content);
                let receipt = read_receipt(path)
                    .filter(|r| r.id == id && Some(&r.version) == declared.as_ref());
                let prior_managed = previous
                    .get(path.to_string_lossy().as_ref())
                    .is_some_and(|s| s == "managed");
                let legacy_managed = legacy.contains(&id) && scope == "app";
                let (source, version, digest) = match receipt {
                    Some(r) if scope == "app" => {
                        ("managed", Some(r.version), Some(r.package_sha256))
                    }
                    _ if scope == "app" && (prior_managed || legacy_managed) => {
                        ("managed", declared, None)
                    }
                    _ => ("external", declared, None),
                };
                tx.execute("INSERT INTO skill_installations(location,name,scope,source,version,package_sha256,observed_at_ms)
                    VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![path.to_string_lossy(),id,scope,source,version,digest,now_ms()])?;
                tx.execute(
                    "INSERT INTO skills(name,version,deleted,installed_at_ms,updated_at_ms)
                    VALUES(?1,?2,0,?3,?3)
                    ON CONFLICT(name) DO UPDATE SET
                        version=excluded.version,deleted=0,updated_at_ms=excluded.updated_at_ms
                    WHERE skills.version IS NOT excluded.version OR skills.deleted != 0",
                    params![id, version, now_ms()],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO skills_meta(key,value) VALUES('reconciled','1')
             ON CONFLICT(key) DO UPDATE SET value='1'",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Reconcile at most once per database.
    ///
    /// Reconciling rewrites `skill_installations` wholesale, which made a
    /// read-only RPC ("list the skills") a writer of the session database —
    /// the contention behind "session persistence ... database is locked".
    /// It is still needed to adopt skill directories that exist on disk with no
    /// registry row (the v3 → v4 upgrade, or a directory placed by hand), so it
    /// runs once, and afterwards only on the mutating operations, where its cost
    /// is invisible next to the install itself.
    fn reconcile_once(&self, db: &mut Connection) -> Result<()> {
        let already: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM skills_meta WHERE key='reconciled')",
            [],
            |row| row.get(0),
        )?;
        if already {
            return Ok(());
        }
        self.reconcile(db)
    }

    fn download(&self, id: &str, version: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/client/v1/skills/{id}/versions/{version}/download",
            self.platform_url
        );
        let response = self.client.get(url).send()?.error_for_status()?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_DOWNLOAD_BYTES)
        {
            bail!("skill download exceeds 64 MiB");
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_DOWNLOAD_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
            bail!("skill download exceeds 64 MiB");
        }
        Ok(bytes)
    }
}

fn read_receipt(dir: &Path) -> Option<Receipt> {
    serde_json::from_slice(&fs::read(dir.join(RECEIPT)).ok()?).ok()
}

fn flatten(dir: &Path) -> Result<()> {
    if dir.join("SKILL.md").exists() {
        return Ok(());
    }
    let entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    if entries.len() != 1 || !entries[0].path().is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(entries[0].path())? {
        let entry = entry?;
        fs::rename(entry.path(), dir.join(entry.file_name()))?;
    }
    fs::remove_dir_all(entries[0].path())?;
    Ok(())
}

fn validate_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value.contains("..")
        || value.ends_with('.')
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        bail!("invalid skill id or version: {value:?}");
    }
    Ok(())
}

fn newer(latest: &str, current: &str) -> bool {
    match (Version::parse(latest), Version::parse(current)) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => false,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn platform_url(agent_dir: &Path) -> String {
    let auth = fs::read(agent_dir.join("auth.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let future = auth.as_ref().and_then(|value| value.get("future"));
    let raw = future
        .and_then(|value| {
            value
                .get("base_url")
                .or_else(|| value.get("platform_base_url"))
        })
        .and_then(serde_json::Value::as_str)
        .unwrap_or("https://future-os.cn");
    raw.trim_end_matches('/')
        .trim_end_matches("/api")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Write};
    use std::net::TcpListener;

    fn package(name: &str, version: &str) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut cursor);
            archive
                .start_file("SKILL.md", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive
                .write_all(
                    format!("---\nname: {name}\nversion: {version}\n---\n# Skill\n").as_bytes(),
                )
                .unwrap();
            archive.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn package_with_frontmatter(frontmatter: &str) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut cursor);
            archive
                .start_file("SKILL.md", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive
                .write_all(format!("---\n{frontmatter}---\n# Skill\n").as_bytes())
                .unwrap();
            archive.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn server(catalogue: &str, bytes: Vec<u8>, requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let catalogue = catalogue.as_bytes().to_vec();
        std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let count = stream.read(&mut request).unwrap();
                let is_catalogue =
                    String::from_utf8_lossy(&request[..count]).contains("/client/v1/skills ");
                let body = if is_catalogue { &catalogue } else { &bytes };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        address
    }

    fn manager(root: &Path, url: String) -> SkillManager {
        SkillManager::new(root.join("agent"), root.join("global"), url).unwrap()
    }

    #[test]
    fn install_records_the_verified_package_version() {
        let root = tempfile::tempdir().unwrap();
        let url = server("{}", package("future-x", "2.0.0"), 1);
        let manager = manager(root.path(), url);
        manager.install("future-x", "2.0.0").unwrap();
        let installed = manager.list_installed().unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].version.as_deref(), Some("2.0.0"));
        assert_eq!(installed[0].source, "managed");
        let db = registry::open_registry(&manager.db_path()).unwrap();
        let version: String = db
            .query_row(
                "SELECT version FROM skills WHERE name='future-x'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "2.0.0");
        assert_eq!(
            read_receipt(&manager.app_dir().join("future-x"))
                .unwrap()
                .version,
            version
        );
    }

    #[test]
    fn mismatched_package_never_replaces_or_records_a_skill() {
        let root = tempfile::tempdir().unwrap();
        let url = server("{}", package("future-x", "3.0.0"), 1);
        let manager = manager(root.path(), url);
        assert!(manager.install("future-x", "2.0.0").is_err());
        assert!(manager.list_installed().unwrap().is_empty());
        let db = registry::open_registry(&manager.db_path()).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM skills", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn install_accepts_metadata_version_when_top_level_version_is_missing_or_invalid() {
        for frontmatter in [
            "name: aeon\nmetadata: {\"version\": \"1.0\"}\n",
            "name: aeon\nversion: \"\"\nmetadata:\n  version: 1.0\n",
        ] {
            let root = tempfile::tempdir().unwrap();
            let url = server("{}", package_with_frontmatter(frontmatter), 1);
            let manager = manager(root.path(), url);
            manager.install("aeon", "1.0").unwrap();
        }
    }

    #[test]
    fn install_does_not_fall_back_when_top_level_version_is_valid_but_mismatched() {
        let root = tempfile::tempdir().unwrap();
        let url = server(
            "{}",
            package_with_frontmatter(
                "name: aeon\nversion: 2.0\nmetadata: {\"version\": \"1.0\"}\n",
            ),
            1,
        );
        let manager = manager(root.path(), url);
        assert!(manager.install("aeon", "1.0").is_err());
    }

    #[test]
    fn sync_installs_unseen_builtin_but_respects_tombstone() {
        let root = tempfile::tempdir().unwrap();
        let catalog = r#"{"skills":[{"id":"future-new","latest_version":"1.0.0","builtin":true},{"id":"future-deleted","latest_version":"1.0.0","builtin":true}]}"#;
        let url = server(catalog, package("future-new", "1.0.0"), 2);
        let manager = manager(root.path(), url);
        manager.locked(|db| {
            db.execute("INSERT INTO skills(name,version,deleted,installed_at_ms,updated_at_ms) VALUES('future-deleted',NULL,1,NULL,1)", [])?;
            Ok(())
        }).unwrap();
        let result = manager.sync(true).unwrap();
        assert_eq!(result.installed, vec!["future-new"]);
        assert!(result.skipped.contains(&"future-deleted".to_string()));
        assert_eq!(manager.list_installed().unwrap().len(), 1);
    }

    #[test]
    fn recovery_finishes_replaced_install_and_reconciles_legacy_version() {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "http://localhost".into());
        let dest = manager.app_dir().join("future-x");
        fs::create_dir_all(&dest).unwrap();
        fs::write(
            dest.join("SKILL.md"),
            "---\nname: future-x\nversion: 2.0.0\n---\n",
        )
        .unwrap();
        fs::write(
            dest.join(RECEIPT),
            serde_json::to_vec(&Receipt {
                id: "future-x".into(),
                version: "2.0.0".into(),
                package_sha256: "digest".into(),
            })
            .unwrap(),
        )
        .unwrap();
        manager.locked(|db| {
            db.execute("INSERT INTO skills(name,version,deleted,installed_at_ms,updated_at_ms) VALUES('future-x','1.0.0',0,1,1)", [])?;
            db.execute("INSERT INTO skill_operations(name,kind,version,phase,started_at_ms) VALUES('future-x','install','2.0.0','replaced',1)", [])?;
            Ok(())
        }).unwrap();
        assert_eq!(
            manager.list_installed().unwrap()[0].version.as_deref(),
            Some("2.0.0")
        );
        let db = registry::open_registry(&manager.db_path()).unwrap();
        let version: String = db
            .query_row(
                "SELECT version FROM skills WHERE name='future-x'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let pending: i64 = db
            .query_row("SELECT count(*) FROM skill_operations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, "2.0.0");
        assert_eq!(pending, 0);
    }

    #[test]
    fn recovery_finishes_uninstall_and_keeps_tombstone() {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "http://localhost".into());
        let dest = manager.app_dir().join("future-x");
        fs::create_dir_all(&dest).unwrap();
        fs::write(
            dest.join("SKILL.md"),
            "---\nname: future-x\nversion: 1.0.0\n---\n",
        )
        .unwrap();
        manager.list_installed().unwrap();
        manager.locked(|db| {
            db.execute("INSERT INTO skill_operations(name,kind,version,phase,started_at_ms) VALUES('future-x','uninstall',NULL,'prepared',1)", [])?;
            Ok(())
        }).unwrap();
        assert!(manager.list_installed().unwrap().is_empty());
        assert!(!dest.exists());
        let db = registry::open_registry(&manager.db_path()).unwrap();
        let deleted: i64 = db
            .query_row(
                "SELECT deleted FROM skills WHERE name='future-x'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(deleted, 1);
    }

    #[test]
    fn legacy_registered_install_stays_managed_but_new_directory_is_external() {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "http://localhost".into());
        for id in ["old", "manual"] {
            let dest = manager.app_dir().join(id);
            fs::create_dir_all(&dest).unwrap();
            fs::write(
                dest.join("SKILL.md"),
                format!("---\nname: {id}\nversion: 1.0.0\n---\n"),
            )
            .unwrap();
        }
        manager.locked(|db| {
            db.execute("INSERT INTO skills(name,version,deleted,installed_at_ms,updated_at_ms) VALUES('old','0.9.0',0,1,1)", [])?;
            Ok(())
        }).unwrap();
        let first = manager.list_installed().unwrap();
        assert_eq!(
            first.iter().find(|s| s.id == "old").unwrap().source,
            "managed"
        );
        assert_eq!(
            first.iter().find(|s| s.id == "manual").unwrap().source,
            "external"
        );
        let second = manager.list_installed().unwrap();
        assert_eq!(
            second.iter().find(|s| s.id == "manual").unwrap().source,
            "external"
        );
        let db = registry::open_registry(&manager.db_path()).unwrap();
        let version: String = db
            .query_row("SELECT version FROM skills WHERE name='old'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, "1.0.0");
    }

    #[test]
    fn sync_upgrades_managed_install_and_records_the_replaced_version() {
        let root = tempfile::tempdir().unwrap();
        let catalog = r#"{"skills":[{"id":"future-x","latest_version":"2.0.0","builtin":true}]}"#;
        let url = server(catalog, package("future-x", "2.0.0"), 2);
        let manager = manager(root.path(), url);
        let dest = manager.app_dir().join("future-x");
        fs::create_dir_all(&dest).unwrap();
        fs::write(
            dest.join("SKILL.md"),
            "---\nname: future-x\nversion: 1.0.0\n---\n",
        )
        .unwrap();
        fs::write(
            dest.join(RECEIPT),
            serde_json::to_vec(&Receipt {
                id: "future-x".into(),
                version: "1.0.0".into(),
                package_sha256: "old".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let result = manager.sync(true).unwrap();
        assert_eq!(result.upgraded, vec!["future-x"]);
        assert_eq!(
            manager.list_installed().unwrap()[0].version.as_deref(),
            Some("2.0.0")
        );
        let db = registry::open_registry(&manager.db_path()).unwrap();
        let version: String = db
            .query_row(
                "SELECT version FROM skills WHERE name='future-x'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "2.0.0");
    }

    #[test]
    fn catalogue_marks_only_managed_newer_installations_for_upgrade() {
        let root = tempfile::tempdir().unwrap();
        let catalog = r#"{"skills":[{"id":"managed","latest_version":"2.0.0"},{"id":"external","latest_version":"2.0.0"}]}"#;
        let url = server(catalog, Vec::new(), 1);
        let manager = manager(root.path(), url);
        for id in ["managed", "external"] {
            let dest = manager.app_dir().join(id);
            fs::create_dir_all(&dest).unwrap();
            fs::write(
                dest.join("SKILL.md"),
                format!("---\nname: {id}\nversion: 1.0.0\n---\n"),
            )
            .unwrap();
        }
        fs::write(
            manager.app_dir().join("managed").join(RECEIPT),
            serde_json::to_vec(&Receipt {
                id: "managed".into(),
                version: "1.0.0".into(),
                package_sha256: "digest".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let available = manager.catalogue_with_status().unwrap();
        assert!(
            available
                .iter()
                .find(|s| s.id == "managed")
                .unwrap()
                .upgrade_available
        );
        assert!(
            !available
                .iter()
                .find(|s| s.id == "external")
                .unwrap()
                .upgrade_available
        );
    }

    #[test]
    fn upgrade_order_uses_semver_including_prereleases() {
        assert!(newer("1.2.0", "1.2.0-rc.1"));
        assert!(newer("1.10.0", "1.9.0"));
        assert!(!newer("1.2.0", "1.2.0"));
        assert!(!newer("1.2", "1.1.0"));
        assert!(!newer("1.2.0", "1.1"));
    }
}

/// The registry shares `agent.db` with the session store, and the write lock it
/// takes there must not be needed by a read.
///
/// `list_installed` and `catalogue_with_status` used to run `reconcile` on every
/// call, which deletes and rewrites `skill_installations` — so "show me the
/// skills" was a writer of the session database. Logged as:
/// `Session persistence command failed: database is locked`, right after a burst
/// of `list_installed_skills` / `list_available_skills` from the desktop.
#[cfg(test)]
mod read_only_tests {
    use super::*;

    /// Hold the write lock, then read. A pure read succeeds; anything that writes
    /// blocks until `busy_timeout` (5 s) and then fails with "database is locked".
    #[test]
    fn listing_skills_does_not_need_the_write_lock() {
        let _home = crate::test_support::TestHome::new();
        let manager = SkillManager::local().unwrap();
        // The first call may reconcile once (creating the marker); the one under
        // test is a settled database, which is what the desktop hits per message.
        manager.list_installed().unwrap();
        manager.catalogue_with_status().unwrap();

        let holder = Connection::open(manager.db_path()).unwrap();
        holder
            .busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        holder
            .execute_batch(
                "BEGIN IMMEDIATE;
                 INSERT OR REPLACE INTO skills_meta(key,value) VALUES('held','1');",
            )
            .unwrap();

        let listed = manager.list_installed();
        let catalogue = manager.catalogue_with_status();
        holder.execute_batch("ROLLBACK").unwrap();

        assert!(
            listed.is_ok(),
            "listing installed skills must not write: {:?}",
            listed.err()
        );
        assert!(
            catalogue.is_ok(),
            "listing the catalogue must not write: {:?}",
            catalogue.err()
        );
    }
}
