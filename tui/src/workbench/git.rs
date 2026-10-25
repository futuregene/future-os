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

/// The branch of one workspace, re-read only when the workspace changes.
///
/// The surface asks for the header on every frame and for the local facts on
/// every state refresh; a `git` spawn per frame would show up as a stutter on a
/// large repository, and the answer only changes when the reader moves.
#[derive(Debug, Default)]
pub struct BranchCache {
    cwd: String,
    branch: String,
    /// Whether a workspace has ever been asked about: `""` is a real answer
    /// (a terminal outside a repository), so it cannot double as "unset".
    loaded: bool,
}

impl BranchCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The branch for `cwd`, reading it only if this is a different workspace
    /// than the last answer was for. `""` asks nothing and answers nothing.
    pub fn for_cwd(&mut self, cwd: &str) -> &str {
        self.for_cwd_with(cwd, branch)
    }

    /// The same cache with the reader injected, so the "one read per workspace"
    /// rule is a unit test instead of a process count.
    pub fn for_cwd_with<F>(&mut self, cwd: &str, read: F) -> &str
    where
        F: FnOnce(&Path) -> String,
    {
        if cwd.is_empty() {
            self.cwd.clear();
            self.branch.clear();
            self.loaded = false;
            return &self.branch;
        }
        let changed = !self.loaded || self.cwd != cwd;
        if changed {
            // A failed read is "no branch", not the previous workspace's
            // branch: a header that keeps naming the old branch after a move is
            // worse than one that names none.
            self.branch = read(Path::new(cwd));
            self.cwd = cwd.to_string();
            self.loaded = true;
        }
        &self.branch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// One read per workspace: the second and third refreshes at the same cwd
    /// spawn nothing, and a move reads again.
    #[test]
    fn the_branch_is_read_once_per_cwd() {
        let calls = Cell::new(0);
        let mut cache = BranchCache::new();
        let read = |path: &Path| {
            calls.set(calls.get() + 1);
            assert_eq!(path, Path::new("/w/one"));
            "dev-huichen".to_string()
        };
        assert_eq!(cache.for_cwd_with("/w/one", read), "dev-huichen");
        assert_eq!(
            cache.for_cwd_with("/w/one", |_| unreachable!("cached")),
            "dev-huichen"
        );
        assert_eq!(
            cache.for_cwd_with("/w/one", |_| unreachable!("cached")),
            "dev-huichen"
        );
        assert_eq!(calls.get(), 1);
        // A worktree move: a different workspace, a second read.
        assert_eq!(
            cache.for_cwd_with("/w/two", |path| {
                assert_eq!(path, Path::new("/w/two"));
                "fix/workbench-defects".to_string()
            }),
            "fix/workbench-defects"
        );
        assert_eq!(calls.get(), 1);
    }

    /// A workspace that is not a repository, a read that failed, and a session
    /// with no cwd all end at the same place: no branch, no panic, and no stale
    /// name left over from the previous workspace.
    #[test]
    fn a_failed_read_is_an_empty_branch_not_a_stale_one() {
        let mut cache = BranchCache::new();
        assert_eq!(
            cache.for_cwd_with("/w/repo", |_| "main".to_string()),
            "main"
        );
        assert_eq!(cache.for_cwd_with("/w/plain", |_| String::new()), "");
        // Back to a repository reads again rather than replaying the empty one.
        assert_eq!(
            cache.for_cwd_with("/w/repo", |_| "main".to_string()),
            "main"
        );
        // No workspace at all: nothing to name, and the next real one reads.
        assert_eq!(cache.for_cwd_with("", |_| unreachable!("no cwd")), "");
        assert_eq!(
            cache.for_cwd_with("/w/repo", |_| "main".to_string()),
            "main"
        );
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
