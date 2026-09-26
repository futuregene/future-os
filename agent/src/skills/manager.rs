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
        fs::create_dir_all(&self.agent_dir).context("create Agent state directory")?;
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(self.agent_dir.join(".skills.lock"))
            .context("open skill installation lock")?;
        lock.lock_exclusive()
            .context("acquire skill installation lock")?;
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
            self.recover(db)
                .context("recover pending skill operation")?;
            self.reconcile(db)
                .context("scan installed skills before install")?;
            if let Err(error) = self.install_locked(db, id, version) {
                self.recover(db)
                    .context("recover failed skill installation")?;
                return Err(error);
            }
            self.reconcile(db)
                .context("scan installed skills after install")?;
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
        let bytes = self
            .download(id, version)
            .context("download skill archive")?;
        let package_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let staging = tempfile::Builder::new()
            .prefix(".skill-install-")
            .tempdir_in(&self.agent_dir)
            .context("create skill staging directory")?;
        let candidate = staging.path().join("candidate");
        fs::create_dir(&candidate).context("create skill staging candidate")?;
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
        archive
            .extract(&candidate)
            .context("extract skill archive")?;
        flatten(&candidate).context("flatten skill archive")?;
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
        let mut file = File::create(candidate.join(RECEIPT)).context("create skill receipt")?;
        serde_json::to_writer(&mut file, &receipt)?;
        file.sync_all().context("sync skill receipt")?;
        // Windows cannot rename the candidate directory while this child file
        // is still open without delete sharing.
        drop(file);

        let app = self.app_dir();
        fs::create_dir_all(&app).context("create installed skills directory")?;
        let dest = app.join(id);
        let backup = self.agent_dir.join(format!(".skill-{id}.previous"));
        db.execute(
            "INSERT INTO skill_operations(name,kind,version,phase,started_at_ms) VALUES(?1,'install',?2,'prepared',?3)
             ON CONFLICT(name) DO UPDATE SET kind='install',version=excluded.version,phase='prepared',started_at_ms=excluded.started_at_ms",
            params![id,version,now_ms()],
        )?;
        if backup.exists() {
            fs::remove_dir_all(&backup).context("remove previous skill backup")?;
        }
        if dest.exists() {
            fs::rename(&dest, &backup).context("back up existing skill")?;
        }
        if let Err(error) = fs::rename(&candidate, &dest) {
            if backup.exists() {
                fs::rename(&backup, &dest)
                    .context("restore existing skill after install failure")?;
            }
            db.execute("DELETE FROM skill_operations WHERE name=?1", [id])?;
            return Err(error).context("publish staged skill");
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
                fs::remove_dir_all(&dest).context("remove failed skill installation")?;
            }
            if backup.exists() {
                fs::rename(&backup, &dest)
                    .context("restore existing skill after finalization failure")?;
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

    // ── Catalogue-driven sync: what each entry is classified as ────────────

    /// One pass over the catalogue has four outcomes and the operator has to be
    /// able to tell them apart: installed, upgraded, skipped (nothing to do),
    /// failed (with the reason). A catalogue entry with no usable version is
    /// *skipped*, an entry whose id/version could never be a directory name is
    /// *failed*, and `install_missing_builtins` never upgrades.
    #[test]
    fn sync_classifies_skipped_failed_and_never_upgrades_on_explicit_bootstrap() {
        let root = tempfile::tempdir().unwrap();
        let catalog = r#"{"skills":[
            {"id":"future-no-version","latest_version":null,"builtin":true},
            {"id":"future-empty-version","latest_version":"","builtin":true},
            {"id":"bad id","latest_version":"1.0.0","builtin":true},
            {"id":"future-bad-version","latest_version":"1.0.0..1","builtin":true},
            {"id":"future-older","latest_version":"1.0.0","builtin":true},
            {"id":"future-new","latest_version":"2.0.0","builtin":true}
        ]}"#;
        let url = platform(catalog, Download::Bytes(package("future-new", "2.0.0")));
        let manager = manager(root.path(), url);
        // An older managed install must not be touched by the explicit
        // bootstrap path, and must not be re-installed either.
        let dest = manager.app_dir().join("future-older");
        fs::create_dir_all(&dest).unwrap();
        fs::write(
            dest.join("SKILL.md"),
            "---\nname: future-older\nversion: 0.9.0\n---\n",
        )
        .unwrap();
        fs::write(
            dest.join(RECEIPT),
            serde_json::to_vec(&Receipt {
                id: "future-older".into(),
                version: "0.9.0".into(),
                package_sha256: "digest".into(),
            })
            .unwrap(),
        )
        .unwrap();
        manager.list_installed().unwrap();

        let result = manager.install_missing_builtins().unwrap();
        assert_eq!(result.installed, vec!["future-new".to_string()]);
        assert!(result.upgraded.is_empty(), "{result:?}");
        for skipped in ["future-no-version", "future-empty-version", "future-older"] {
            assert!(
                result.skipped.contains(&skipped.to_string()),
                "{skipped} was not skipped: {result:?}"
            );
        }
        assert_eq!(result.failed.len(), 2, "{result:?}");
        assert!(
            result
                .failed
                .iter()
                .all(|reason| reason.ends_with(": invalid catalog id/version")),
            "the failure must name the entry and the reason: {:?}",
            result.failed
        );
        assert!(result
            .failed
            .iter()
            .any(|reason| reason.starts_with("bad id")));
    }

    /// An install that fails in the middle of a sync is reported against the
    /// entry that caused it, and the recovery pass leaves no pending operation
    /// behind — the next sync has to behave as if the attempt never happened.
    #[test]
    fn a_failed_install_mid_sync_is_reported_and_leaves_no_pending_operation() {
        let root = tempfile::tempdir().unwrap();
        let catalog =
            r#"{"skills":[{"id":"future-gone","latest_version":"1.0.0","builtin":true}]}"#;
        let url = platform(catalog, Download::Status("404 Not Found"));
        let manager = manager(root.path(), url);
        let result = manager.sync(true).unwrap();
        assert!(result.installed.is_empty());
        assert_eq!(result.failed.len(), 1, "{result:?}");
        assert!(
            result.failed[0].starts_with("future-gone: "),
            "the failure must name the entry: {:?}",
            result.failed
        );
        assert!(
            manager.list_installed().unwrap().is_empty(),
            "a failed download must not be recorded as installed"
        );
        let db = registry::open_registry(&manager.db_path()).unwrap();
        let pending: i64 = db
            .query_row("SELECT count(*) FROM skill_operations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(pending, 0, "recovery must clear the pending operation");
    }

    // ── Archive validation: what a hostile package cannot do ───────────────

    #[test]
    fn an_archive_with_an_unsafe_path_is_refused() {
        let bytes = archive_with(|archive| {
            archive
                .start_file(
                    "../escape/SKILL.md",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive
                .write_all(b"---\nname: future-x\nversion: 1.0.0\n---\n")
                .unwrap();
        });
        install_rejects(bytes, "skill archive has an unsafe path");
    }

    #[test]
    fn an_archive_with_a_symlink_entry_is_refused() {
        let mut bytes = archive_with(|archive| {
            archive
                .start_file("SKILL.md", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive
                .write_all(b"---\nname: future-x\nversion: 1.0.0\n---\n")
                .unwrap();
        });
        // The zip writer records a DOS creator, so a reader would never look at
        // the external attributes. Patch the central directory to what a Unix
        // `zip -y` writes: host 3 (Unix) and mode 0120777 (a symbolic link).
        patch_u16(&mut bytes, b"PK\x01\x02", 4, 0x031e);
        patch_u32(&mut bytes, b"PK\x01\x02", 38, 0o120777 << 16);
        install_rejects(bytes, "symbolic link");
    }

    /// The entry-count and uncompressed-size caps are what stop a zip bomb from
    /// filling the disk before the identity check ever runs.
    #[test]
    fn an_archive_with_too_many_entries_is_refused() {
        let bytes = archive_with(|archive| {
            for index in 0..4097 {
                archive
                    .start_file(
                        format!("entry-{index}"),
                        zip::write::SimpleFileOptions::default(),
                    )
                    .unwrap();
            }
        });
        install_rejects(bytes, "too many entries");
    }

    /// The uncompressed-size guard reads the size the archive *declares*, so a
    /// bomb that lies about its expansion is refused before anything is written
    /// to disk. The sizes are patched in both the local header and the central
    /// directory, because either is a valid source for that declared size.
    #[test]
    fn an_archive_that_declares_more_than_the_extraction_limit_is_refused() {
        let mut bytes = package("future-x", "1.0.0");
        let declared = (MAX_EXTRACTED_BYTES + 1) as u32;
        patch_u32(&mut bytes, b"PK\x03\x04", 22, declared);
        patch_u32(&mut bytes, b"PK\x01\x02", 24, declared);
        install_rejects(bytes, "exceeds 256 MiB uncompressed");
    }

    #[test]
    fn an_archive_without_skill_md_is_refused() {
        let bytes = archive_with(|archive| {
            archive
                .start_file("readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(b"nothing here").unwrap();
        });
        install_rejects(bytes, "no SKILL.md");
    }

    /// A single wrapping directory is normal for a release archive, so it is
    /// unwrapped; two top-level directories are ambiguous and are left alone
    /// (which then fails the SKILL.md check rather than picking one at random).
    #[test]
    fn a_single_wrapping_directory_is_unwrapped_but_two_are_not() {
        let root = tempfile::tempdir().unwrap();
        let wrapped = archive_with(|archive| {
            archive
                .start_file(
                    "future-x/SKILL.md",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive
                .write_all(b"---\nname: future-x\nversion: 1.0.0\n---\n")
                .unwrap();
            archive
                .start_file(
                    "future-x/reference.md",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive.write_all(b"docs").unwrap();
        });
        let dir = root.path().join("wrapped");
        extract(&wrapped, &dir);
        flatten(&dir).unwrap();
        // The nested files moved up with SKILL.md, and the wrapper is gone.
        assert!(dir.join("SKILL.md").is_file());
        assert!(dir.join("reference.md").is_file());
        assert!(!dir.join("future-x").exists());

        let ambiguous = archive_with(|archive| {
            for name in ["one/readme.md", "two/readme.md"] {
                archive
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                archive.write_all(b"docs").unwrap();
            }
        });
        let dir = root.path().join("ambiguous");
        extract(&ambiguous, &dir);
        flatten(&dir).unwrap();
        assert!(
            dir.join("one").is_dir() && dir.join("two").is_dir(),
            "an ambiguous layout is left exactly as it was"
        );
        assert!(
            !dir.join("SKILL.md").exists(),
            "nothing is moved, so the SKILL.md check fails cleanly instead of \
             guessing which directory was meant"
        );
    }

    /// A leftover `.skill-<id>.previous` from an interrupted replacement is
    /// cleaned up before the new version is published, so the stale copy cannot
    /// shadow the install or accumulate on disk.
    #[test]
    fn a_stale_backup_directory_is_cleaned_up_before_publishing() {
        let root = tempfile::tempdir().unwrap();
        let url = platform("{}", Download::Bytes(package("future-x", "1.0.0")));
        let manager = manager(root.path(), url);
        let backup = manager.agent_dir.join(".skill-future-x.previous");
        fs::create_dir_all(backup.join("stale")).unwrap();
        fs::write(backup.join("stale/SKILL.md"), "---\nname: future-x\n---\n").unwrap();
        manager.install("future-x", "1.0.0").unwrap();
        assert!(manager.app_dir().join("future-x/SKILL.md").is_file());
        assert!(
            !backup.exists(),
            "the previous attempt's directory must not be left behind"
        );
    }

    /// A failure while the receipt is being committed must not leave a
    /// half-published skill behind: the previous install comes back, and the
    /// pending operation is cleared so the next attempt starts clean.
    #[test]
    fn a_finalization_failure_restores_the_previous_install() {
        let root = tempfile::tempdir().unwrap();
        let first = server("{}", package("future-x", "1.0.0"), 1);
        let installed = manager(root.path(), first);
        installed.install("future-x", "1.0.0").unwrap();

        // Force `finish_install`'s first statement to fail: the phase update is
        // the last thing that can go wrong before the registry is rewritten.
        let db = registry::open_registry(&installed.db_path()).unwrap();
        db.execute_batch(
            "CREATE TRIGGER refuse_replaced BEFORE UPDATE ON skill_operations
             WHEN NEW.phase='replaced' BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();
        drop(db);

        let second = server("{}", package("future-x", "2.0.0"), 1);
        let replaced = manager(root.path(), second);
        let error = replaced.install("future-x", "2.0.0").unwrap_err();
        assert!(
            format!("{error:#}").contains("injected"),
            "the injected failure must surface: {error:#}"
        );

        // The published v1.0.0 directory is back, and nothing is left pending.
        let published = installed.list_installed().unwrap();
        assert_eq!(published.len(), 1, "{published:?}");
        assert_eq!(published[0].version.as_deref(), Some("1.0.0"));
        assert!(!installed
            .agent_dir
            .join(".skill-future-x.previous")
            .exists());
        let db = registry::open_registry(&installed.db_path()).unwrap();
        let pending: i64 = db
            .query_row("SELECT count(*) FROM skill_operations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(pending, 0);
    }

    /// An uninstall removes the directory *and* reports that it did, which is
    /// what the CLI prints; a tombstone is kept so a sync cannot resurrect it.
    #[test]
    fn uninstall_removes_the_directory_and_reports_it() {
        let root = tempfile::tempdir().unwrap();
        let url = server("{}", package("future-x", "1.0.0"), 1);
        let manager = manager(root.path(), url);
        manager.install("future-x", "1.0.0").unwrap();
        assert!(manager.app_dir().join("future-x").is_dir());
        assert!(
            manager.uninstall("future-x").unwrap(),
            "a directory was removed"
        );
        assert!(!manager.app_dir().join("future-x").exists());
        assert!(
            !manager.uninstall("future-x").unwrap(),
            "a second uninstall removes nothing"
        );
        assert!(manager.list_installed().unwrap().is_empty());
    }

    /// Recovery has two outcomes for an interrupted install: finish it when the
    /// staged directory and its receipt agree, otherwise put the backup back.
    #[test]
    fn recovery_clears_the_backup_of_a_completed_replacement() {
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
        // The replaced install is still sitting in the backup slot.
        let backup = manager.agent_dir.join(".skill-future-x.previous");
        fs::create_dir_all(backup.join("stale")).unwrap();
        fs::write(backup.join("SKILL.md"), "---\nname: future-x\n---\n").unwrap();
        manager
            .locked(|db| {
                db.execute("INSERT INTO skill_operations(name,kind,version,phase,started_at_ms) VALUES('future-x','install','2.0.0','replaced',1)", [])?;
                Ok(())
            })
            .unwrap();

        assert_eq!(
            manager.list_installed().unwrap()[0].version.as_deref(),
            Some("2.0.0")
        );
        assert!(
            !backup.exists(),
            "the replaced install is dead once the new one is committed"
        );
        assert!(dest.join("SKILL.md").is_file());
    }

    #[test]
    fn recovery_restores_the_backup_when_the_staged_install_never_landed() {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "http://localhost".into());
        let dest = manager.app_dir().join("future-x");
        // The backup holds the only good copy: the staged directory is gone (or
        // was never a complete install).
        let backup = manager.agent_dir.join(".skill-future-x.previous");
        fs::create_dir_all(&backup).unwrap();
        fs::write(
            backup.join("SKILL.md"),
            "---\nname: future-x\nversion: 1.0.0\n---\n",
        )
        .unwrap();
        let stale = dest.join("leftover");
        fs::create_dir_all(&stale).unwrap();
        manager
            .locked(|db| {
                db.execute("INSERT INTO skill_operations(name,kind,version,phase,started_at_ms) VALUES('future-x','install','2.0.0','prepared',1)", [])?;
                Ok(())
            })
            .unwrap();

        let installed = manager.list_installed().unwrap();
        assert_eq!(installed.len(), 1, "{installed:?}");
        assert_eq!(installed[0].version.as_deref(), Some("1.0.0"));
        assert!(dest.join("SKILL.md").is_file());
        assert!(!stale.exists(), "the half-written directory is discarded");
        assert!(!backup.exists());
    }

    /// Reconciliation adopts what is on disk and ignores everything that is not
    /// a skill directory, so a stray file or a directory without a SKILL.md
    /// cannot become an "installed skill".
    #[test]
    fn reconciliation_ignores_files_and_directories_without_a_skill_md() {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "http://localhost".into());
        let app = manager.app_dir();
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join("loose-file.md"), b"not a skill").unwrap();
        fs::create_dir_all(app.join("no-manifest")).unwrap();
        let listed = manager.list_installed().unwrap();
        assert!(listed.is_empty(), "{listed:?}");
    }

    /// The same skill can exist in both scopes; the app install wins and the
    /// duplicate row is dropped rather than listed twice.
    #[test]
    fn an_app_install_shadows_the_global_install_of_the_same_skill() {
        let root = tempfile::tempdir().unwrap();
        let manager = manager(root.path(), "http://localhost".into());
        for (dir, version) in [
            (manager.app_dir(), "2.0.0"),
            (manager.global_dir.clone(), "1.0.0"),
        ] {
            // Reconciliation scans one level down: the skill is a directory
            // under the scope root that contains a SKILL.md.
            fs::create_dir_all(dir.join("future-x")).unwrap();
            fs::write(
                dir.join("future-x/SKILL.md"),
                format!("---\nname: future-x\nversion: {version}\n---\n"),
            )
            .unwrap();
        }
        let listed = manager.list_installed().unwrap();
        assert_eq!(listed.len(), 1, "one row per skill id: {listed:?}");
        assert_eq!(listed[0].scope, "app");
        assert_eq!(listed[0].version.as_deref(), Some("2.0.0"));
    }

    // ── Download limits ──

    /// A download over the cap is refused. The declared length is enough on its
    /// own — nothing is read from a body the gateway already said is too big.
    #[test]
    fn a_download_with_an_oversized_declared_length_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let url = platform(
            "{}",
            Download::Stream {
                declared: Some(MAX_DOWNLOAD_BYTES + 1),
                streamed: 0,
            },
        );
        let manager = manager(root.path(), url);
        let error = manager.install("future-x", "1.0.0").unwrap_err();
        assert!(
            format!("{error:#}").contains("skill download exceeds 64 MiB"),
            "unexpected error: {error:#}"
        );
    }

    /// A body with no declared length is still capped while it is read, so a
    /// gateway that streams forever cannot exhaust memory either.
    #[test]
    fn a_download_that_streams_past_the_cap_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let url = platform(
            "{}",
            Download::Stream {
                declared: None,
                streamed: MAX_DOWNLOAD_BYTES + 1,
            },
        );
        let manager = manager(root.path(), url);
        let error = manager.install("future-x", "1.0.0").unwrap_err();
        assert!(
            format!("{error:#}").contains("skill download exceeds 64 MiB"),
            "unexpected error: {error:#}"
        );
    }

    // ── Harness ────────────────────────────────────────────────────────────

    /// What the package endpoint should do for one test.
    enum Download {
        /// Serve these package bytes.
        Bytes(Vec<u8>),
        /// Answer with this status instead of a package.
        Status(&'static str),
        /// Declare `declared` (when given) and then stream `streamed` bytes.
        Stream {
            declared: Option<u64>,
            streamed: u64,
        },
    }

    /// Both platform endpoints, in one server that keeps answering until the
    /// test ends: the catalogue is served by path, everything else is the
    /// package download.
    fn platform(catalogue: &str, download: Download) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let catalogue = catalogue.as_bytes().to_vec();
        std::thread::spawn(move || loop {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            serve_connection(stream, &catalogue, &download);
        });
        address
    }

    /// Answer one client: the catalogue for a catalogue request, `download` for
    /// everything else. A client that cannot be read is dropped without a reply,
    /// which must not end the server.
    ///
    /// Generic over the connection so the two "this client is gone" failures the
    /// loop swallows can be driven by a connection that fails on demand instead
    /// of by trying to race a real socket into erroring.
    fn serve_connection(mut stream: impl Read + Write, catalogue: &[u8], download: &Download) {
        let mut request = [0u8; 4096];
        let count = match stream.read(&mut request) {
            Ok(count) => count,
            // Nothing to answer: the peer went away before it said anything.
            Err(_) => return,
        };
        if String::from_utf8_lossy(&request[..count]).contains("/client/v1/skills ") {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                catalogue.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(catalogue);
            return;
        }
        match download {
            Download::Bytes(bytes) => {
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(bytes);
            }
            Download::Status(status) => {
                let body = b"gone";
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            }
            Download::Stream { declared, streamed } => {
                // No declared length means the client reads until the socket
                // closes, which is how a chunked body looks to reqwest.
                let length = match declared {
                    Some(length) => format!("Content-Length: {length}\r\n"),
                    None => String::new(),
                };
                let _ = stream.write_all(
                    format!("HTTP/1.1 200 OK\r\n{length}Connection: close\r\n\r\n").as_bytes(),
                );
                let chunk = vec![b'z'; 1 << 20];
                let mut left = *streamed;
                while left > 0 {
                    let take = left.min(chunk.len() as u64) as usize;
                    if stream.write_all(&chunk[..take]).is_err() {
                        break;
                    }
                    left -= take as u64;
                }
            }
        }
    }

    /// A connection that fails on demand, for the two "the client is gone"
    /// failures `serve_connection` has to swallow: a request that cannot be
    /// read, and a body write that dies mid-stream. `writes` records every write
    /// attempt, which is what the tests assert on.
    struct FailingConnection {
        unreadable: bool,
        writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Read for FailingConnection {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.unreadable {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "client went away",
                ));
            }
            let request = b"GET /package HTTP/1.1\r\n\r\n";
            let take = request.len().min(buf.len());
            buf[..take].copy_from_slice(&request[..take]);
            Ok(take)
        }
    }

    impl Write for FailingConnection {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let attempt = self
                .writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if attempt == 0 {
                return Ok(buf.len());
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "client went away",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A client whose request cannot be read is dropped without an answer — and,
    /// because this is one connection of a server loop, without ending the
    /// server. The write counter is the assertion: an unread request must not
    /// produce a single write attempt.
    #[test]
    fn a_client_whose_request_cannot_be_read_is_dropped_without_an_answer() {
        let mut connection = FailingConnection {
            unreadable: true,
            writes: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        serve_connection(&mut connection, b"{}", &Download::Bytes(Vec::new()));
        assert_eq!(
            connection.writes.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a request that could not be read must not be answered"
        );
    }

    /// A client that goes away while the body is being streamed ends the pump:
    /// the failed write breaks the loop, so the harness does not keep pushing
    /// chunks into a dead socket. The write counter separates "broke out" (the
    /// header write, then one failed body write) from "kept going" (one write
    /// per remaining chunk).
    #[test]
    fn a_write_failure_mid_body_stops_the_stream_pump() {
        let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        serve_connection(
            &mut FailingConnection {
                unreadable: false,
                writes: writes.clone(),
            },
            b"[]",
            &Download::Stream {
                declared: None,
                streamed: 8 * 1024 * 1024,
            },
        );
        assert_eq!(
            writes.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the header write plus exactly one failed body write"
        );
    }

    /// Run the install pipeline against `bytes` and assert it is refused with
    /// `needle` in the reason, and that nothing was recorded.
    fn install_rejects(bytes: Vec<u8>, needle: &str) {
        let root = tempfile::tempdir().unwrap();
        let url = platform("{}", Download::Bytes(bytes));
        let manager = manager(root.path(), url);
        let error = manager.install("future-x", "1.0.0").unwrap_err();
        assert!(
            format!("{error:#}").contains(needle),
            "expected {needle:?} in: {error:#}"
        );
        assert!(
            manager.list_installed().unwrap().is_empty(),
            "a refused archive must not be recorded"
        );
    }

    /// Build a zip with `write`, for the archive-shape tests that are not about
    /// the package identity.
    fn archive_with(
        write: impl FnOnce(&mut zip::ZipWriter<&mut std::io::Cursor<Vec<u8>>>),
    ) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut cursor);
            write(&mut archive);
            archive.finish().unwrap();
        }
        cursor.into_inner()
    }

    /// Overwrite a 32-bit little-endian field of every record carrying
    /// `signature`, `offset` bytes into that record.
    fn patch_u32(bytes: &mut [u8], signature: &[u8; 4], offset: usize, value: u32) {
        let mut index = 0;
        while let Some(found) = bytes[index..]
            .windows(4)
            .position(|window| window == signature)
        {
            let at = index + found + offset;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            index = at + 4;
        }
    }

    /// Overwrite a 16-bit little-endian field of every record carrying
    /// `signature`, `offset` bytes into that record.
    fn patch_u16(bytes: &mut [u8], signature: &[u8; 4], offset: usize, value: u16) {
        let mut index = 0;
        while let Some(found) = bytes[index..]
            .windows(4)
            .position(|window| window == signature)
        {
            let at = index + found + offset;
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
            index = at + 2;
        }
    }

    fn extract(bytes: &[u8], into: &Path) {
        fs::create_dir_all(into).unwrap();
        zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .unwrap()
            .extract(into)
            .unwrap();
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
