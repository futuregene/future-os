use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_FILES: usize = 1024;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const QUERY_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub path: String,
    pub bytes: usize,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub workspace: String,
    pub root: String,
    pub files: Vec<SnapshotFile>,
    pub directories: Vec<String>,
    pub excluded: Vec<String>,
    pub capture_started_at: i64,
    pub capture_finished_at: i64,
}

impl Snapshot {
    pub fn capture(
        workspace: &Path,
        root: &Path,
        scopes: &[String],
        cancelled: &AtomicBool,
        permitted: impl Fn(&Path) -> bool,
    ) -> Result<Self> {
        ensure!(
            !root.exists(),
            "Snapshot already exists without a committed manifest; preserve it for diagnosis"
        );
        let workspace = workspace.canonicalize()?;
        std::fs::create_dir_all(root)?;
        let mut snapshot = Self {
            workspace: workspace.to_str().context("Non-Unicode workspace")?.into(),
            root: root.to_str().context("Non-Unicode snapshot")?.into(),
            files: vec![],
            directories: vec![String::new()],
            excluded: vec![],
            capture_started_at: crate::now_ms(),
            capture_finished_at: 0,
        };
        let mut candidates = BTreeSet::new();
        let mut directories = BTreeSet::new();
        for scope in scopes {
            let scope = Path::new(scope);
            let scope = if scope.is_absolute() {
                scope.to_path_buf()
            } else {
                workspace.join(scope)
            };
            for ancestor in scope.ancestors().take_while(|p| p.starts_with(&workspace)) {
                ensure!(
                    !special_link(&std::fs::symlink_metadata(ancestor)?),
                    "Fusion cannot capture paths through symlinks or reparse points"
                );
            }
            let canonical = scope.canonicalize().context("Read scope does not exist")?;
            ensure!(
                canonical.starts_with(&workspace),
                "Fusion read scope escapes the authorized workspace"
            );
            ensure!(
                !special_link(&std::fs::symlink_metadata(&scope)?),
                "Fusion does not follow scope links"
            );
            for entry in walkdir::WalkDir::new(&canonical)
                .follow_links(false)
                .sort_by_file_name()
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0
                        || !matches!(
                            e.file_name().to_str(),
                            Some(".git" | "node_modules" | "target" | ".worktrees")
                        )
                })
            {
                ensure!(
                    !cancelled.load(Ordering::Acquire),
                    "Fusion snapshot cancelled"
                );
                let entry = entry?;
                let metadata = std::fs::symlink_metadata(entry.path())?;
                ensure!(
                    !special_link(&metadata),
                    "Fusion cannot capture symlinks or reparse points: {}",
                    entry.path().display()
                );
                let relative = entry.path().strip_prefix(&workspace)?;
                let logical = logical_path(relative)?;
                ensure!(
                    permitted(entry.path()),
                    "Fusion read denied by parent policy: {logical}"
                );
                if metadata.is_dir() {
                    directories.insert(logical);
                } else if metadata.is_file() {
                    candidates.insert(entry.path().to_path_buf());
                } else {
                    bail!("Fusion cannot capture special file {logical}");
                }
                ensure!(
                    candidates.len() <= MAX_FILES && directories.len() <= MAX_FILES,
                    "Fusion snapshot exceeds 1024 files/directories; narrow read_scope"
                );
            }
        }
        let mut total = 0;
        for source in candidates {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "Fusion snapshot cancelled"
            );
            let relative = source.strip_prefix(&workspace)?;
            let logical = logical_path(relative)?;
            let metadata = std::fs::symlink_metadata(&source)?;
            ensure!(
                metadata.is_file() && !special_link(&metadata),
                "File changed type during snapshot"
            );
            ensure!(
                metadata.len() <= MAX_FILE_BYTES as u64,
                "File exceeds 1 MiB: {logical}"
            );
            let bytes = read_bounded(&source, MAX_FILE_BYTES)?;
            ensure!(
                bytes.len() <= MAX_FILE_BYTES,
                "File grew beyond snapshot bound"
            );
            total += bytes.len();
            ensure!(
                total <= MAX_TOTAL_BYTES,
                "Snapshot exceeds 16 MiB; narrow read_scope"
            );
            if std::str::from_utf8(&bytes).is_err() || bytes.contains(&0) {
                snapshot
                    .excluded
                    .push(format!("{logical}: binary or non-UTF-8"));
                continue;
            }
            let destination = root.join("files").join(relative);
            std::fs::create_dir_all(destination.parent().context("Snapshot parent missing")?)?;
            write_synced(&destination, &bytes)?;
            snapshot.files.push(SnapshotFile {
                path: logical,
                bytes: bytes.len(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            });
        }
        // Detect observed capture drift rather than claiming an atomic filesystem copy.
        for file in &snapshot.files {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "Fusion snapshot cancelled"
            );
            let source = workspace.join(&file.path);
            let metadata = std::fs::symlink_metadata(&source)?;
            ensure!(
                metadata.is_file()
                    && !special_link(&metadata)
                    && metadata.len() <= MAX_FILE_BYTES as u64,
                "File changed during snapshot"
            );
            let current = read_bounded(&source, MAX_FILE_BYTES)?;
            ensure!(
                format!("{:x}", Sha256::digest(current)) == file.sha256,
                "File changed during snapshot: {}",
                file.path
            );
        }
        for file in &snapshot.files {
            let mut parent = Path::new(&file.path).parent();
            while let Some(path) = parent {
                directories.insert(logical_path(path)?);
                parent = path.parent();
            }
        }
        snapshot.directories.extend(directories);
        snapshot.directories.sort();
        snapshot.directories.dedup();
        snapshot.capture_finished_at = crate::now_ms();
        let manifest = root.join("manifest.tmp");
        write_synced(&manifest, &serde_json::to_vec(&snapshot)?)?;
        std::fs::rename(manifest, root.join("manifest.json"))?;
        #[cfg(unix)]
        std::fs::File::open(root)?.sync_all()?;
        Ok(snapshot)
    }

    pub fn load(root: &Path) -> Result<Self> {
        let bytes = read_bounded(&root.join("manifest.json"), 1024 * 1024)?;
        ensure!(
            bytes.len() <= 1024 * 1024,
            "Snapshot manifest exceeds bound"
        );
        let value: Self = serde_json::from_slice(&bytes)?;
        ensure!(
            Path::new(&value.root).canonicalize()? == root.canonicalize()?,
            "Snapshot source mismatch"
        );
        Ok(value)
    }

    fn logical(&self, path: &str) -> Result<String> {
        let p = Path::new(path);
        let p = if p.is_absolute() {
            p.strip_prefix(&self.workspace)
                .or_else(|_| p.strip_prefix(&self.root))
                .context("Path outside Fusion snapshot")?
        } else {
            p
        };
        logical_path(p)
    }

    fn read_text(&self, path: &str) -> Result<String> {
        let file = self
            .files
            .iter()
            .find(|f| f.path == path)
            .context("File not in this consultation's authorized snapshot")?;
        let target = Path::new(&self.root).join("files").join(path);
        let metadata = std::fs::symlink_metadata(&target)?;
        ensure!(
            metadata.is_file() && !special_link(&metadata),
            "Invalid snapshot file"
        );
        ensure!(
            target
                .canonicalize()?
                .starts_with(Path::new(&self.root).canonicalize()?),
            "Snapshot path escaped"
        );
        let bytes = read_bounded(&target, MAX_FILE_BYTES)?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == file.sha256,
            "Snapshot content changed"
        );
        Ok(String::from_utf8(bytes)?)
    }

    pub fn query(&self, tool: &str, args: Value) -> Result<String> {
        ensure!(
            crate::INNER_TOOL_NAMES.contains(&tool),
            "Fusion tool denied: {tool}"
        );
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 400) as usize;
        let mut truncated = false;
        let mut results = vec![];
        match tool {
            "read" => {
                let path = self.logical(args["path"].as_str().context("read requires path")?)?;
                let text = self.read_text(&path)?;
                let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
                let lines = text.lines().collect::<Vec<_>>();
                for (n, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
                    results.push(json!({"line":n+1,"text":line}));
                }
                truncated = lines.len() > offset - 1 + results.len();
                bounded(
                    json!({"path":path,"sha256":self.files.iter().find(|f|f.path==path).map(|f|&f.sha256),"lines":results,"truncated":truncated}),
                )
            }
            "ls" => {
                let root = self.logical(args["path"].as_str().unwrap_or("."))?;
                ensure!(
                    self.directories.contains(&root),
                    "Directory not in snapshot"
                );
                let depth = args["depth"].as_u64().unwrap_or(1).clamp(1, 8) as usize;
                let prefix = if root.is_empty() {
                    String::new()
                } else {
                    format!("{root}/")
                };
                let items = self
                    .directories
                    .iter()
                    .filter(|p| !p.is_empty())
                    .map(|p| (p, true))
                    .chain(self.files.iter().map(|f| (&f.path, false)))
                    .filter(|(p, _)| {
                        p.starts_with(&prefix)
                            && **p != root
                            && p[prefix.len()..].split('/').count() <= depth
                    })
                    .collect::<BTreeSet<_>>();
                for (path, directory) in items {
                    if results.len() == limit {
                        truncated = true;
                        break;
                    }
                    results.push(json!({"path":path,"kind":if directory{"directory"}else{"file"}}));
                }
                bounded(
                    json!({"entries":results,"truncated":truncated,"snapshot":self.capture_finished_at,"excluded":self.excluded}),
                )
            }
            "glob" | "grep" => {
                let root = self.logical(args["root"].as_str().unwrap_or("."))?;
                ensure!(
                    self.directories.contains(&root) || self.files.iter().any(|f| f.path == root),
                    "Search root not in snapshot"
                );
                let pattern = if tool == "glob" {
                    args["pattern"].as_str().context("glob requires pattern")?
                } else {
                    args["glob"].as_str().unwrap_or("**")
                };
                ensure!(pattern.len() <= 1024, "Glob is too large");
                let matcher = globset::GlobBuilder::new(pattern)
                    .literal_separator(false)
                    .build()?
                    .compile_matcher();
                let regex = if tool == "grep" {
                    let pattern = args["pattern"].as_str().context("grep requires pattern")?;
                    ensure!(pattern.len() <= 2048, "Pattern too large");
                    Some(
                        regex::RegexBuilder::new(pattern)
                            .size_limit(1024 * 1024)
                            .build()?,
                    )
                } else {
                    None
                };
                let context = args["context"].as_u64().unwrap_or(0).min(5) as usize;
                'files: for file in &self.files {
                    if !(root.is_empty()
                        || file.path == root
                        || file.path.starts_with(&format!("{root}/")))
                        || !matcher.is_match(&file.path)
                    {
                        continue;
                    }
                    if let Some(regex) = &regex {
                        let text = self.read_text(&file.path)?;
                        let lines = text.lines().collect::<Vec<_>>();
                        for (n, line) in lines.iter().enumerate() {
                            if regex.is_match(line) {
                                if results.len() == limit {
                                    truncated = true;
                                    break 'files;
                                }
                                results.push(json!({"path":file.path,"line":n+1,"text":line,"context":lines[n.saturating_sub(context)..(n+context+1).min(lines.len())],"sha256":file.sha256}));
                            }
                        }
                    } else {
                        if results.len() == limit {
                            truncated = true;
                            break;
                        }
                        results.push(json!({"path":file.path,"sha256":file.sha256}));
                    }
                }
                bounded(json!({"matches":results,"truncated":truncated,"excluded":self.excluded}))
            }
            _ => bail!("Fusion tool denied"),
        }
    }
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "Fusion cannot read a special file"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "File exceeds snapshot bound");
    Ok(bytes)
}
fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn bounded(value: Value) -> Result<String> {
    let text = serde_json::to_string(&value)?;
    ensure!(
        text.len() <= QUERY_BYTES,
        "Query exceeds 48 KiB; narrow the query or reduce limit/context"
    );
    Ok(text)
}
fn logical_path(path: &Path) -> Result<String> {
    let mut parts = vec![];
    for part in path.components() {
        match part {
            Component::Normal(p) => parts.push(p.to_str().context("Non-Unicode snapshot path")?),
            Component::CurDir => {}
            _ => bail!("Parent traversal and absolute paths are not allowed here"),
        }
    }
    Ok(parts.join("/"))
}
fn special_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_policy_binary_and_manifest_filename() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("w");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("manifest.json"), "user document").unwrap();
        std::fs::write(workspace.join("binary"), [0, 1, 2]).unwrap();
        assert!(Snapshot::capture(
            &workspace,
            &dir.path().join("denied"),
            &["manifest.json".into()],
            &AtomicBool::new(false),
            |_| false
        )
        .is_err());
        assert!(Snapshot::capture(
            &workspace,
            &dir.path().join("escaped"),
            &["..".into()],
            &AtomicBool::new(false),
            |_| true
        )
        .is_err());
        let snap = Snapshot::capture(
            &workspace,
            &dir.path().join("s"),
            &["manifest.json".into(), "binary".into()],
            &AtomicBool::new(false),
            |_| true,
        )
        .unwrap();
        assert_eq!(snap.excluded.len(), 1);
        assert!(snap
            .query("read", json!({"path":"manifest.json"}))
            .unwrap()
            .contains("user document"));
        assert!(snap.query("read", json!({"path":"binary"})).is_err());
        for name in ["write", "edit", "shell", "fusion", "new_session"] {
            assert!(snap.query(name, json!({})).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symbolic_links() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("w");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(dir.path().join("secret"), "secret").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret"), workspace.join("link")).unwrap();
        assert!(Snapshot::capture(
            &workspace,
            &dir.path().join("s"),
            &["link".into()],
            &AtomicBool::new(false),
            |_| true
        )
        .is_err());
    }
    #[test]
    fn snapshot_is_fixed_and_queries_cannot_escape() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("a.rs"), "first\nsecond\n").unwrap();
        let snapshot = Snapshot::capture(
            &workspace,
            &dir.path().join("snapshot"),
            &["a.rs".into()],
            &AtomicBool::new(false),
            |_| true,
        )
        .unwrap();
        std::fs::write(workspace.join("a.rs"), "changed").unwrap();
        assert!(snapshot
            .query("read", json!({"path":"a.rs"}))
            .unwrap()
            .contains("first"));
        assert!(snapshot.query("read", json!({"path":"../secret"})).is_err());
        assert!(snapshot.query("shell", json!({"command":"ls"})).is_err());
        assert!(snapshot
            .query("grep", json!({"pattern":"second"}))
            .unwrap()
            .contains("a.rs"));
    }
}
