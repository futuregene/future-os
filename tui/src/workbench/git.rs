//! The one git read the workbench does: which branch a workspace is on.
//!
//! The header names the session's workspace, and a branch is half of that name
//! for anyone who works in a repository of worktrees: `/Users/x/checkout` says
//! which checkout, `dev-<user>` says which line of work. The read is
//! deliberately the smallest possible one — `git rev-parse --abbrev-ref HEAD`
//! in the session's own directory — because the surface must not pay for a git
//! library, a repository scan or a config parse to draw one line.
//!
//! Three rules, all of them about not letting a repository annoy the reader:
//!
//! * **A deadline.** [`spawn_with_timeout`] kills the child after
//!   [`BRANCH_TIMEOUT`]; a git that is waiting on a lock (another session's
//!   `git rebase`) must not freeze the frame that asked.
//! * **No output, no branch.** Not a repository, no commits yet, `git` not
//!   installed, a permission error — every one of those is an empty string, and
//!   the header degrades to the bare cwd it already draws.
//! * **Once per workspace.** [`BranchCache`] re-reads only when the session's
//!   cwd changes; `ctrl+k`, a `usage` event and every redraw in between reuse
//!   the cached name instead of spawning a process.

use std::path::Path;
use std::time::Duration;

/// How long a branch read may take before it is killed and reported as "no
/// branch". Sized for a cold `git` on a loaded machine, not for a network
/// filesystem: a hang here would hold the frame that asked for it.
pub const BRANCH_TIMEOUT: Duration = Duration::from_millis(600);

/// The branch checked out in `cwd`, or `""` when there is none to name.
///
/// A detached HEAD is named by its short commit (`@9e4a47d2a`): "HEAD" on its
/// own is a git implementation detail, and the reader wants to know *where* the
/// checkout is, not that it is not on a branch.
pub fn branch(cwd: &Path) -> String {
    let (code, stdout, _) = run(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]);
    if code != 0 {
        return String::new();
    }
    let name = stdout.trim();
    if name.is_empty() {
        return String::new();
    }
    if name != "HEAD" {
        return name.to_string();
    }
    let (code, stdout, _) = run(cwd, &["rev-parse", "--short", "HEAD"]);
    if code != 0 {
        return String::new();
    }
    let sha = stdout.trim();
    if sha.is_empty() {
        String::new()
    } else {
        format!("@{sha}")
    }
}

/// `git -C <cwd> <args…>`, bounded by [`BRANCH_TIMEOUT`]. A failure to spawn or
/// a timeout is reported as a non-zero code with no output, which every caller
/// already treats as "no branch".
fn run(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let mut full = vec!["-C".to_string(), cwd.display().to_string()];
    full.extend(args.iter().map(|arg| (*arg).to_string()));
    match crate::subprocess::spawn_with_timeout(Path::new("git"), &full, BRANCH_TIMEOUT) {
        Ok(result) => result,
        Err(_) => (1, String::new(), String::new()),
    }
}

/// The branch of one workspace, installed once and then only read.
///
/// The surface asks for the header on every frame, so a `git` spawn per frame
/// would show up as a stutter on a large repository. It also asks *right after a
/// switch*, which is the worst moment to wait for a process — and a read can take
/// up to [`BRANCH_TIMEOUT`] on a repository another session has locked. So this
/// type never reads: it holds the last answer, says when a workspace still needs
/// one, and the caller reads on its own thread.
///
/// Two rules, both about not letting a repository annoy the reader:
///
/// * **No read here.** [`Self::get`] answers from memory and never spawns; the
///   frame that names the workspace cannot wait for a process. The read itself
///   belongs to [`crate::workbench::live::LocalReads`], which also decides *when*
///   one is owed (a workspace move) and drops an answer for a workspace the
///   reader has already left.
/// * **No stale name.** A workspace that has not been read yet has no branch to
///   name — an empty string, never the previous workspace's branch.
#[derive(Debug, Default)]
pub struct BranchCache {
    cwd: String,
    branch: String,
    /// Whether a workspace has ever been installed: `""` is a real answer (a
    /// terminal outside a repository), so it cannot double as "unset".
    loaded: bool,
}

impl BranchCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The branch of `cwd`, or nothing while its read is still running.
    pub fn get(&self, cwd: &str) -> &str {
        if self.loaded && self.cwd == cwd {
            &self.branch
        } else {
            ""
        }
    }

    /// Install a finished read for `cwd`. An empty `cwd` names no workspace and
    /// installs nothing.
    pub fn install(&mut self, cwd: &str, branch: String) {
        if cwd.is_empty() {
            return;
        }
        self.cwd = cwd.to_string();
        self.branch = branch;
        self.loaded = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace that has not been read yet names no branch — an empty string,
    /// never the previous workspace's.
    #[test]
    fn a_workspace_that_was_never_read_names_no_branch() {
        let mut cache = BranchCache::new();
        assert_eq!(cache.get("/w/one"), "");
        cache.install("/w/one", "dev-huichen".to_string());
        assert_eq!(cache.get("/w/one"), "dev-huichen");
        assert_eq!(cache.get("/w/one"), "dev-huichen", "a cache, not one shot");
        assert_eq!(
            cache.get("/w/two"),
            "",
            "and nothing for a workspace it has not been told about"
        );
        assert_eq!(cache.get("/w/one"), "dev-huichen", "which it still holds");
    }

    /// A workspace that is not a repository, a read that failed, and a session
    /// with no cwd all end at the same place: no branch, no panic, and no stale
    /// name left over from the previous workspace.
    #[test]
    fn a_failed_read_is_an_empty_branch_not_a_stale_one() {
        let mut cache = BranchCache::new();
        cache.install("/w/repo", "main".to_string());
        assert_eq!(cache.get("/w/repo"), "main");
        cache.install("/w/plain", String::new());
        assert_eq!(cache.get("/w/plain"), "", "not a repository, no branch");
        assert_eq!(cache.get("/w/repo"), "", "and not the previous one either");
        // No workspace at all: nothing to name.
        assert_eq!(cache.get(""), "");
        cache.install("", "ignored".to_string());
        assert_eq!(cache.get(""), "", "an install for no workspace is ignored");
        assert_eq!(cache.get("/w/plain"), "");
    }

    /// The real read, in this repository: the branch the checkout is on, and
    /// nothing at all for a directory that is not a repository.
    #[test]
    fn a_real_repository_names_its_branch() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let here = here.parent().expect("the crate sits inside the repository");
        let name = branch(here);
        assert!(
            !name.is_empty(),
            "the worktree this test runs in is a checkout: {name:?}"
        );
        assert!(!name.contains('\n'), "a branch name is one line: {name:?}");
        let outside = tempfile::tempdir().expect("a temp dir");
        assert_eq!(branch(outside.path()), "");
    }
}
