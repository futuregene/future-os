use super::*;
use std::process::Command;

/// A repository whose history is built by the test itself, so the expected
/// picture is a property of the fixture rather than of this checkout.
struct Repo {
    dir: tempfile::TempDir,
    /// The instant the next `git` command commits at, in seconds since the epoch.
    ///
    /// The read is `git log --date-order`, and a date order is only *total* when
    /// the instants are distinct: `--date-order` has to break a tie between two
    /// commits committed in the same second itself, and it breaks it from the rev
    /// set the walk started at rather than from the commits alone. A fixture that
    /// let every commit land in the same second therefore read its eight commits
    /// in one order as one page and in another order as two — `8ad8075` and
    /// `ebd6258` swapped — which passed on Git for macOS and Linux and failed on
    /// Git for Windows (`Rust Windows test / host-core`, run 44937037098).
    ///
    /// So every command commits at a distinct second, and the order the pages are
    /// compared in is a property of the history instead of of the tie-break.
    clock: std::cell::Cell<i64>,
}

/// The first instant the fixture commits at: 2020-09-13T12:26:40Z, far enough
/// from any test's `now` that nothing reads as "just now".
const FIRST_INSTANT: i64 = 1_600_000_000;

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = Self {
            dir,
            clock: std::cell::Cell::new(FIRST_INSTANT),
        };
        repo.git(&["init", "--initial-branch=main"]);
        repo.git(&["config", "user.email", "reader@example.com"]);
        repo.git(&["config", "user.name", "Reader"]);
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The instant one command commits at, one second after the command before
    /// it. Applied to *every* command, so `commit`, `commit -m` and the merge
    /// `git merge` makes itself all advance the clock, and no two commits in the
    /// fixture can share a second.
    fn instant(&self) -> String {
        let at = self.clock.get();
        self.clock.set(at + 1);
        format!("@{at} +0000")
    }

    fn git(&self, args: &[&str]) -> String {
        let instant = self.instant();
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(args)
            .env("GIT_AUTHOR_DATE", &instant)
            .env("GIT_COMMITTER_DATE", &instant)
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
        list(&self.path().display().to_string(), "").unwrap()
    }

    /// One page of the history, as a reader asking for more would get it.
    fn page(&self, cursor: &str, take: usize) -> GraphListing {
        page(&self.path().display().to_string(), cursor, take).unwrap()
    }

    /// Commit one file with the author and committer named separately, the way a
    /// squash merge or a rebase leaves them.
    fn commit_as(
        &self,
        author: &str,
        author_email: &str,
        committer: &str,
        committer_email: &str,
        subject: &str,
    ) {
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
        let instant = self.instant();
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(["commit", "-m", subject])
            .env("GIT_AUTHOR_DATE", &instant)
            .env("GIT_COMMITTER_DATE", &instant)
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", author_email)
            .env("GIT_COMMITTER_NAME", committer)
            .env("GIT_COMMITTER_EMAIL", committer_email)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git commit as {author}/{committer}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The fixture's history is totally ordered: every commit has an instant of its
/// own.
///
/// [`the_second_page_is_the_tail_of_one_bigger_read`] compares two orders, and
/// `--date-order` can only reproduce one across a seam when no two commits share
/// a second — the tie-break is the walk's, not the commits' (see `Repo::clock`).
/// That disagreement showed up on Git for Windows and on neither other platform,
/// so the property the paging assertion rests on is asserted here, where every
/// platform runs it and a fixture that stopped pinning its dates would fail at
/// home rather than in one CI job.
#[test]
fn the_fixture_gives_every_commit_an_instant_of_its_own() {
    let repo = Repo::new();
    repo.commit("base");
    repo.git(&["checkout", "-b", "side"]);
    repo.commit("side work");
    repo.git(&["checkout", "main"]);
    repo.commit("main work");
    // A merge commits too, and it is git making the commit rather than the
    // fixture, so it is the one a pinned `commit` would miss.
    repo.git(&["merge", "--no-ff", "-m", "merge side", "side"]);
    let stamps = repo.git(&["log", "--all", "--format=%ct"]);
    let mut instants: Vec<&str> = stamps.lines().map(str::trim).collect();
    instants.sort_unstable();
    instants.dedup();
    assert_eq!(
        instants.len(),
        4,
        "two commits share a committer second, so the date order has a tie: {stamps:?}"
    );
}

/// A directory that is not a repository, with a `.git`-less parent chain: the
/// one answer the phone draws nothing for.
#[test]
fn a_plain_directory_is_not_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let listing = list(&dir.path().display().to_string(), "").unwrap();
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
    let listing = list(&nested.display().to_string(), "").unwrap();
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

/// A lane keeps its column, ten tips are ten lanes, and the picture still stops
/// at the window: the phone draws every row's gutter at one width, so a history
/// that keeps opening lanes would squeeze every subject on the page.
#[test]
fn the_picture_stops_widening_past_the_window() {
    let repo = Repo::new();
    repo.commit("base");
    for n in 0..10 {
        repo.git(&["checkout", "-q", "-b", &format!("lane-{n}"), "main"]);
        repo.commit(&format!("lane {n} work"));
    }
    let listing = repo.listing();
    let window = 2 * LANE_WINDOW;
    assert!(
        listing.commits.len() >= 11,
        "the ten tips and their base: {}",
        listing.commits.len()
    );
    for row in &listing.commits {
        assert!(
            row.graph.chars().count() <= window,
            "a ten-lane history drew a {}-cell gutter, past the window: {:?}",
            row.graph.chars().count(),
            row.graph
        );
        // A commit sits on an even cell, and past the window it is still drawn —
        // in the bundle's column, at the window's edge.
        if let Some(at) = row
            .graph
            .chars()
            .position(|cell| cell == 'o' || cell == 'M')
        {
            assert_eq!(at % 2, 0, "{:?}", row.graph);
            assert!(at <= window - 2, "a dot past the window: {:?}", row.graph);
        }
    }
    // The window is used, not merely respected: ten lanes fill all six columns.
    assert!(
        listing
            .commits
            .iter()
            .any(|row| row.graph.chars().count() == window),
        "no row filled the window: {:?}",
        listing
            .commits
            .iter()
            .map(|row| &row.graph)
            .collect::<Vec<_>>()
    );
}

/// Pages join into one picture: reading the log two commits at a time must draw
/// exactly what reading it whole draws. The layout of a page is a fold over the
/// rows above it, so a page cut out and laid out on its own would start every
/// lane at column 0 and break the drawing at the seam.
/// A page continues where the last one stopped: the cursor is a position in the
/// log, so walking it must reproduce one uninterrupted read exactly — the same
/// commits, in the same order, with the same lanes, and the pages must not
/// overlap or skip.
///
/// This is the property the cursor exists for. Reading the log from the top on
/// every page (with `--skip`, or by asking for `skip + take` and dropping the
/// first `skip`) also happens to produce these rows; what it cannot do is keep
/// doing it at the hundredth page without reading the whole history each time.
/// The assertion is on the sequence, so a cursor that is ignored — every page
/// answering with the first one — fails it on the duplicate rows.
#[test]
fn pages_concatenate_into_the_single_page_drawing() {
    let repo = Repo::new();
    repo.commit("base");
    repo.git(&["checkout", "-b", "side"]);
    repo.commit("side work");
    repo.git(&["checkout", "main"]);
    repo.commit("main work");
    repo.git(&["merge", "--no-ff", "-m", "merge side", "side"]);

    let whole = repo.listing();
    let mut paged = Vec::new();
    let mut cursor = String::new();
    loop {
        let page = repo.page(&cursor, 2);
        assert!(page.commits.len() <= 2, "a page overran its size");
        let last = !page.truncated;
        cursor = page.cursor.clone();
        paged.extend(page.commits.into_iter().map(|row| (row.short, row.graph)));
        if last {
            break;
        }
        assert!(!cursor.is_empty(), "a truncated page must carry a cursor");
    }
    let whole: Vec<(String, String)> = whole
        .commits
        .into_iter()
        .map(|row| (row.short, row.graph))
        .collect();
    assert_eq!(paged.len(), whole.len());
    assert_eq!(paged, whole);
}

/// A page read from a cursor is exactly the tail of the read above it: page two
/// holds the commits one bigger read lists after page one's last row, in that
/// order.
///
/// This is the assertion a rewritten cursor has to keep: if the continuation
/// ignores the cursor (answers with the first page again) the rows repeat, and
/// if it walks from a narrower set than the page above left it (dropping the
/// branches the log had not reached yet) commits go missing. Both show up here
/// as a sequence that is not the tail.
#[test]
fn the_second_page_is_the_tail_of_one_bigger_read() {
    let repo = Repo::new();
    repo.commit("base");
    // Two long branches that do not merge, one opening after the other: at a cut
    // this leaves a branch tip the pages above have never shown, which is the
    // case a cursor made only of the open lanes loses.
    for (branch, count) in [("alpha", 3), ("beta", 3)] {
        repo.git(&["checkout", "-b", branch, "main"]);
        for at in 0..count {
            repo.commit(&format!("{branch} {at}"));
        }
    }
    repo.git(&["checkout", "main"]);
    repo.commit("main head");

    let whole: Vec<String> = repo
        .page("", 20)
        .commits
        .into_iter()
        .map(|row| row.short)
        .collect();
    assert_eq!(whole.len(), 8, "base, three, three and the head: {whole:?}");

    let first = repo.page("", 3);
    assert_eq!(first.commits.len(), 3);
    let second = repo.page(&first.cursor, 3);
    assert_eq!(second.commits.len(), 3);
    let got: Vec<String> = first
        .commits
        .iter()
        .chain(second.commits.iter())
        .map(|row| row.short.clone())
        .collect();
    assert_eq!(got, whole[..6], "page two is not the tail of one read");

    // The same cursor twice is the same page: a client retries a page that
    // failed, and a retry must not advance the reader's position.
    let again = repo.page(&first.cursor, 3);
    let once: Vec<String> = second.commits.iter().map(|row| row.short.clone()).collect();
    let twice: Vec<String> = again.commits.iter().map(|row| row.short.clone()).collect();
    assert_eq!(once, twice);
}

/// Paging a linear history: each page says whether another one follows, and the
/// last one answers with no cursor at all.
#[test]
fn every_page_says_whether_another_one_follows() {
    let repo = Repo::new();
    for subject in ["one", "two", "three", "four", "five"] {
        repo.commit(subject);
    }
    let first = repo.page("", 3);
    assert_eq!(first.commits.len(), 3);
    assert!(first.truncated);
    assert!(!first.cursor.is_empty());
    assert_eq!(first.commits[0].subject, "five");

    let second = repo.page(&first.cursor, 3);
    assert_eq!(second.commits.len(), 2);
    assert!(!second.truncated);
    assert!(
        second.cursor.is_empty(),
        "the last page has nothing to continue to"
    );
    assert_eq!(second.commits[1].subject, "one");
    // The header of a page is the repository's, not the page's, so the second
    // page carries it too: the client says what it is reading once, and the
    // answer does not get thinner as it pages.
    assert_eq!(second.repo_name, first.repo_name);

    // Every row of the history is padded the same, whichever page carries it.
    let widths: Vec<usize> = first
        .commits
        .iter()
        .chain(second.commits.iter())
        .map(|row| row.graph.chars().count())
        .collect();
    assert!(widths.iter().all(|width| *width == widths[0]));
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

/// A row names the commit's *author*, not its committer.
///
/// They differ whenever someone else lands a change: a squash merge on GitHub
/// rewrites the committer to "GitHub", a rebase to whoever ran it. Reading the
/// committer made a whole page of real work read as "GitHub" (measured on this
/// repository: every PR-merged commit there has author `Hui Chen` and committer
/// `GitHub`).
#[test]
fn a_row_names_the_author_not_the_committer() {
    let repo = Repo::new();
    repo.commit("written by the author, landed by someone else");
    // Commit once more under a different committer, the way a squash merge does.
    repo.commit_as(
        "Author Person",
        "author@example.com",
        "GitHub",
        "noreply@github.com",
        "squashed",
    );

    let listing = repo.listing();
    let squashed = listing
        .commits
        .iter()
        .find(|row| row.subject == "squashed")
        .expect("the squashed commit is in the log");
    assert_eq!(squashed.who, "Author Person");
    assert_ne!(squashed.who, "GitHub", "the committer is not the author");
}

/// The detail lists every ref on the commit, not git's shortened decoration.
#[test]
fn the_detail_lists_every_ref_on_the_commit() {
    let repo = Repo::new();
    repo.commit("base");
    repo.commit("tagged and branched");
    let head = repo.listing().commits[0].short.clone();
    // More refs than `%D` keeps on one line: two branches, a remote-tracking
    // branch and two tags all point at the same commit.
    repo.git(&["branch", "feature/a"]);
    repo.git(&["branch", "feature/b"]);
    repo.git(&["tag", "v1.0"]);
    repo.git(&["tag", "v1.1"]);
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);

    let detail = detail(&repo.path().display().to_string(), &head).unwrap();
    assert_eq!(detail.refs.branches, vec!["feature/a", "feature/b", "main"]);
    assert_eq!(detail.refs.remotes, vec!["origin/main"]);
    assert_eq!(detail.refs.tags, vec!["v1.0", "v1.1"]);
    // HEAD is on this commit, so the checkout is named too.
    assert!(detail.refs.head);
}

/// A commit no ref points at answers with empty lists rather than an error.
#[test]
fn the_detail_of_a_commit_no_ref_points_at_lists_nothing() {
    let repo = Repo::new();
    repo.commit("old");
    let older = repo.listing().commits[0].short.clone();
    repo.commit("newer");

    let detail = detail(&repo.path().display().to_string(), &older).unwrap();
    assert!(
        detail.refs.branches.is_empty(),
        "{:?}",
        detail.refs.branches
    );
    assert!(detail.refs.remotes.is_empty());
    assert!(detail.refs.tags.is_empty());
    // The older commit is not what is checked out.
    assert!(!detail.refs.head);
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
