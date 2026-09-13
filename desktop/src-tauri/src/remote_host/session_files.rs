//! Read-only, session-rooted directory browsing for paired mobile clients.
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Listing {
    root_path: String,
    path: String,
    entries: Vec<Entry>,
}

pub(super) fn list(session_id: &str, requested: &str) -> Result<Listing, crate::AppError> {
    if session_id.is_empty() {
        return Err("A session is required.".to_string().into());
    }
    let thread = crate::store::find_thread_by_agent_session(session_id)?
        .ok_or_else(|| "Session workspace is unavailable.".to_string())?;
    let root = crate::agent_bridge::workspace_path_for_thread(&thread.id)?;
    list_under_root(Path::new(&root), requested)
}

fn list_under_root(root: &Path, requested: &str) -> Result<Listing, crate::AppError> {
    let root = root.canonicalize()?;
    // Empty path means the session root, never the desktop process's cwd.
    let path = root.join(requested).canonicalize()?;
    if !path.starts_with(&root) {
        return Err("Directory is outside the session workspace."
            .to_string()
            .into());
    }
    crate::commands::ensure_path_allowed(&path)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path)? {
        let Ok(entry) = entry else { continue };
        // Do not expose symlinks/junctions leading outside the session or to
        // protected credentials. Each navigation is independently checked.
        let Ok(target) = entry.path().canonicalize() else {
            continue;
        };
        if !target.starts_with(&root) || crate::commands::ensure_path_allowed(&target).is_err() {
            continue;
        }
        let Ok(meta) = target.metadata() else {
            continue;
        };
        if !meta.is_dir() && !meta.is_file() {
            continue;
        }
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(Listing {
        root_path: root.to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_session_root_and_children_but_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("z.txt"), "hello").unwrap();
        std::fs::write(root.join("A.txt"), "a").unwrap();
        std::fs::write(root.join(".hidden"), "h").unwrap();
        let listing = list_under_root(&root, "").unwrap();
        assert_eq!(listing.path, root.canonicalize().unwrap().to_string_lossy());
        assert_eq!(
            listing
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["sub", ".hidden", "A.txt", "z.txt"]
        );
        assert!(listing.entries[0].is_dir);
        assert_eq!(listing.entries[3].size, 5);
        assert!(list_under_root(&root, "sub").unwrap().entries.is_empty());
        assert!(list_under_root(&root, &listing.entries[0].path).is_ok());
        assert!(list_under_root(&root, "..").is_err());
        assert!(list_under_root(&root, &dir.path().to_string_lossy()).is_err());
        assert!(list_under_root(&root, "missing").is_err());
        assert!(list_under_root(&root, "A.txt").is_err());
        assert!(list("", "").is_err());
    }

    #[test]
    fn protected_credentials_are_not_listed() {
        let _home = crate::remote::test_support::HomeGuard::new("session-file-credentials");
        let home = crate::home_dir().unwrap();
        let root = Path::new(&home).join(".future").join("agent");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("auth.json"), "secret").unwrap();
        std::fs::write(root.join("notes.txt"), "public").unwrap();
        let listing = list_under_root(&root, "").unwrap();
        assert!(listing
            .entries
            .iter()
            .all(|entry| entry.name != "auth.json"));
        assert!(listing
            .entries
            .iter()
            .any(|entry| entry.name == "notes.txt"));
        assert!(list_under_root(&root, "auth.json").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("escape")).unwrap();
        assert!(list_under_root(&root, "escape").is_err());
        assert!(list_under_root(&root, "").unwrap().entries.is_empty());
    }
}
