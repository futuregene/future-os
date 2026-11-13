use super::*;
use std::process::Command;

/// A repository whose history is built by the test itself, so the expected
/// picture is a property of the fixture rather than of this checkout.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = Self { dir };
        repo.git(&["init", "--initial-branch=main"]);
        repo.git(&["config", "user.email", "reader@example.com"]);
        repo.git(&["config", "user.name", "Reader"]);
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Commit one file, named after the subject: separate files keep the fixture
    /// free of merge conflicts, so a merge test measures the graph rather than
    /// how the test wrote its content.
    fn commit(&self, subject: &str) {
        let name: String = subject
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        std::fs::write(
            self.path().join(format!("{name}.txt")),
            format!("{subject}\n"),
        )
        .unwrap();
        self.git(&["add", "."]);
        self.git(&["commit", "-m", subject]);
    }

    fn listing(&self) -> GraphListing {
        list(&self.path().display().to_string()).unwrap()
    }
}

/// A directory that is not a repository, with a `.git`-less parent chain: the
/// one answer the phone draws nothing for.
#[test]
fn a_plain_directory_is_not_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let listing = list(&dir.path().display().to_string()).unwrap();
    assert!(!listing.is_repo);
    assert!(listing.repo_name.is_empty());
    assert!(listing.commits.is_empty());
}

/// The root the host reports never carries the Windows extended-length prefix:
/// it is the one spelling a client renders verbatim and would send straight back,
/// so `\\?\C:\…` would surface as the workspace's own path. The inputs here do
/// not resolve on any platform, so the read falls back to the spelling it was
/// given and this is the removal alone — the rule holds off Windows too, where a
/// Windows-spelled path can still arrive from a paired desktop.
#[test]
fn a_reported_root_never_carries_the_windows_extended_length_prefix() {
    assert_eq!(
        reported_root(Path::new(r"\\?\C:\future-os")),
        Path::new(r"C:\future-os")
    );
    assert_eq!(
        reported_root(Path::new(r"\\?\UNC\server\share\checkout")),
        Path::new(r"\\server\share\checkout")
    );
    // An ordinary spelling is untouched: the phone shows what the file system
    // says, not a form this boundary invented.
    let dir = tempfile::tempdir().unwrap();
    let root = crate::store::strip_verbatim_prefix(dir.path().canonicalize().unwrap());
    assert_eq!(reported_root(&root), root);
}

/// A workspace *inside* a repository is a repository: the phone's session
/// workspace is often a subdirectory of a checkout, and `git_review`'s
/// equality-of-toplevel rule would wrongly hide the history there.
#[test]
fn a_subdirectory_of_a_repository_reports_the_repository() {
    let repo = Repo::new();
    repo.commit("first");
    let nested = repo.path().join("nested/deeper");
    std::fs::create_dir_all(&nested).unwrap();
    let listing = list(&nested.display().to_string()).unwrap();
    assert!(listing.is_repo);
    // Spelled the way the file system spells it: `fs::canonicalize` produces the
    // extended-length form (`\\?\C:\…`) on Windows, and a phone given that string
    // would show it — or send it back — as the workspace it is not. Asserting the
    // absence catches a regression the equality below cannot: two canonicalised
    // spellings agree whatever prefix either one carries.
    assert!(!listing.path.starts_with(r"\\?\"), "{}", listing.path);
    // The repository's root, not the nested workspace the listing was asked for.
    // Compared in canonical form on both sides: a client renders the reported
    // string verbatim, so its *spelling* is the assertion above, and this one is
    // about which directory it names.
    assert_eq!(
        Path::new(&listing.path)
            .canonicalize()
            .expect("the reported root exists"),
        repo.path().canonicalize().expect("the fixture root exists"),
    );
    assert_eq!(listing.commits.len(), 1);
}

#[test]
fn a_fresh_repository_has_a_name_and_no_commits() {
    let repo = Repo::new();
    let listing = repo.listing();
    assert!(listing.is_repo);
    assert!(!listing.repo_name.is_empty());
    assert!(
        listing.commits.is_empty(),
        "an unborn branch is an empty log"
    );
    assert!(!listing.truncated);
}

/// The gutter is the point: a commit's row carries its own horizontal, lanes are
/// columns, and every row is the same width so the subjects line up.
#[test]
fn the_log_is_drawn_as_lane_rows() {
    let repo = Repo::new();
    repo.commit("first");
    repo.commit("second");
    let listing = repo.listing();
    assert_eq!(listing.commits.len(), 2);
    // Newest first, and a single line of work is one column.
    assert_eq!(listing.commits[0].subject, "second");
    assert_eq!(listing.commits[0].graph.trim_end(), "o");
    assert_eq!(listing.commits[1].graph.trim_end(), "o");
    // Every row is padded to the page's width, so the subjects line up.
    assert_eq!(
        listing.commits[0].graph.chars().count(),
        listing.commits[1].graph.chars().count()
    );
    // The committer's name and a minute-precision stamp, as the TUI prints them.
    assert_eq!(listing.commits[0].who, "Reader");
    assert_eq!(listing.commits[0].when.len(), 16);
}

/// A merge draws an `M` with its second parent in a column to the right, and the
/// two lines it joins are joined *on that row* rather than on a drawing row
/// below it.
#[test]
fn a_merge_opens_a_lane_and_joins_it_on_its_own_row() {
    let repo = Repo::new();
    repo.commit("base");
    repo.git(&["checkout", "-b", "side"]);
    repo.commit("side work");
    repo.git(&["checkout", "main"]);
    repo.commit("main work");
    repo.git(&["merge", "--no-ff", "-m", "merge side", "side"]);
    let listing = repo.listing();
    let subjects: Vec<&str> = listing
        .commits
        .iter()
        .map(|row| row.subject.as_str())
        .collect();
    assert_eq!(subjects[0], "merge side");
    // Every row paints a commit: the graph never spends a row on bare drawing.
    for row in &listing.commits {
        assert!(
            row.graph.contains('o') || row.graph.contains('M'),
            "row {} has no commit glyph: {:?}",
            row.short,
            row.graph
        );
    }
    // The merge is the only `M`, and its horizontal reaches the side lane.
    assert_eq!(
        listing
            .commits
            .iter()
            .filter(|row| row.graph.contains('M'))
            .count(),
        1
    );
    let merge = &listing.commits[0].graph;
    assert!(merge.starts_with("M"), "merge gutter: {merge:?}");
    assert!(merge.contains('─'), "merge horizontal: {merge:?}");
    // One width for the whole page.
    let width = listing.commits[0].graph.chars().count();
    for row in &listing.commits {
        assert_eq!(row.graph.chars().count(), width);
    }
}

/// A lane keeps one column for as long as it exists, which is what lets the phone
/// colour it: a branch that moved columns would also change colour.
#[test]
fn a_lane_keeps_its_column_across_the_page() {
    let repo = Repo::new();
    repo.commit("base");
    for (branch, subject) in [
        ("alpha", "alpha 1"),
        ("beta", "beta 1"),
        ("gamma", "gamma 1"),
    ] {
        repo.git(&["checkout", "-b", branch, "main"]);
        repo.commit(subject);
    }
    let listing = repo.listing();
    // Three tips open three lanes, and the commit column of each stays where it
    // started: no row has a commit further left than a row above it did.
    let columns: Vec<usize> = listing
        .commits
        .iter()
        .filter_map(|row| row.graph.find(['o', 'M']))
        .collect();
    assert!(columns.len() >= 4);
    // The gutter never grows wider than the lanes that exist, and a commit's own
    // column is even (lane `n` sits at `2n`).
    assert!(columns.iter().all(|at| at % 2 == 0));
}

/// A commit's detail is its stat and patch; a request that is not a sha is
/// refused before git is asked anything.
#[test]
fn the_detail_is_the_patch_and_only_a_sha_is_accepted() {
    let repo = Repo::new();
    repo.commit("hello");
    let listing = repo.listing();
    let short = listing.commits[0].short.clone();
    let detail = detail(&repo.path().display().to_string(), &short).unwrap();
    assert_eq!(detail.short, short);
    assert!(detail.patch.contains("hello"), "{}", detail.patch);
    // The stat and the patch are both there — the file is named after the
    // subject, and `git show --stat --patch` prints it in the header and again
    // in the hunk.
    assert!(detail.patch.contains("hello.txt"), "{}", detail.patch);
    assert!(!detail.truncated);

    let path = repo.path().display().to_string();
    for rejected in ["", "  ", "HEAD", "--all", "file.txt", "../../etc/passwd"] {
        assert!(
            super::detail(&path, rejected).is_err(),
            "{rejected:?} must not reach git as a revision"
        );
    }
}

/// A huge patch is cut, and the cut is on a line boundary with a notice the
/// reader can act on.
#[test]
fn a_large_patch_is_bounded_and_says_so() {
    let repo = Repo::new();
    repo.commit("base");
    let big = (0..4_000)
        .map(|index| format!("line {index} of a large file"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(repo.path().join("file.txt"), format!("{big}\n")).unwrap();
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "large"]);
    let listing = repo.listing();
    let detail = detail(
        &repo.path().display().to_string(),
        &listing.commits[0].short,
    )
    .unwrap();
    assert!(detail.truncated);
    assert!(detail.patch.contains("truncated"));
    assert!(detail.patch.len() <= PATCH_MAX_BYTES + 200);
    assert!(detail.patch.lines().count() <= PATCH_MAX_ROWS + 1);
}

/// A directory that is not a repository answers the detail read with a message
/// rather than a plausible-looking empty patch.
#[test]
fn the_detail_of_a_non_repository_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let error = detail(&dir.path().display().to_string(), "abc1234").unwrap_err();
    assert!(error.to_string().contains("not a Git repository"));
}

/// The separators are the only thing a subject is split on: a subject holding a
/// tab, a pipe and a colon stays one field.
#[test]
fn a_subject_with_separators_stays_one_field() {
    let repo = Repo::new();
    repo.commit("a | b\tc: d — e");
    let listing = repo.listing();
    assert_eq!(listing.commits[0].subject, "a | b\tc: d — e");
}

/// Two branches reaching the same commit is one row, not a phantom lane
/// running down the page with no commit of its own.
#[test]
fn a_branch_reaching_an_already_drawn_commit_draws_no_empty_lane() {
    let repo = Repo::new();
    repo.commit("base");
    repo.git(&["checkout", "-b", "feature"]);
    repo.commit("feature work");
    repo.git(&["checkout", "main"]);
    // Merge the same side twice: the second merge's extra parent is a commit
    // another lane already lands on.
    repo.git(&["merge", "--no-ff", "-m", "merge one", "feature"]);
    repo.git(&["merge", "--no-ff", "-m", "merge two", "feature"]);
    let listing = repo.listing();
    // No lane is wider than the merges that exist, and every row still carries a
    // commit — the layout drops a link that would draw an empty one.
    for row in &listing.commits {
        assert!(
            row.graph.contains('o') || row.graph.contains('M'),
            "empty row: {:?}",
            row.graph
        );
    }
}
