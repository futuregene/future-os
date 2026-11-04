//! Read-only, session-rooted directory browsing for paired mobile clients.
//!
//! # Cost model
//!
//! A directory is answered **one page at a time** ([`list_page`]) because the
//! per-entry work is filesystem work: the phone's file panel opens whatever
//! directory the agent was working in, and a workspace holding `node_modules`
//! or a build output tree has thousands of plain entries. The measured
//! baseline was ≈0.046 ms per entry (5,000 files → 229 ms; see
//! `docs/testing/remote-latency/04-scaling.md` §(f)), which crossed the 100 ms
//! budget at ≈2,200 entries.
//!
//! Two syscall groups were paid for *every* entry and are now paid only where
//! they can change the answer:
//!
//! * `Path::canonicalize` (a full resolution, readlink chain included) is only
//!   needed for a **symlink**, the only entry kind whose name can leave the
//!   session root — and whether an entry is one is decided by its one `lstat`,
//!   not by the directory's type byte. `DirEntry::file_type` gives that byte
//!   (`d_type`) for the ordering pass, which therefore needs no syscall at all.
//! * `crate::files::ensure_path_allowed` canonicalizes the path it is given
//!   plus each credential root, so it is only called for names that can
//!   actually name a protected credential (see [`may_be_credential`]) or for a
//!   resolved symlink target, whose *target* name is what the denylist matches.
//!
//! What remains per returned entry is one `lstat` (`symlink_metadata`) for its
//! kind and size. That is bounded by the page size, not by the directory.
//!
//! # What still scales with the directory
//!
//! The listing is alphabetical, so a page has to see every name in the
//! directory: one `readdir` pass, with no per-entry syscall and no per-entry
//! allocation beyond the name itself. Measured on this machine, release build:
//! that pass is ≈1.6–2.4 ms for 5,000 entries against ≈0.02 ms for ten, and the
//! whole first page at 5,000 entries is ≈2.4–3.5 ms (versus 0.05 ms for ten and
//! 229 ms before this change). The residual is the price of the ordering — an
//! unordered page would cost ≈page-size only — and it is ~20× cheaper per entry
//! than the 0.046 ms/entry baseline.
//!
//! # Security property (unchanged)
//!
//! An entry that resolves outside the session root, or to a protected
//! credential path, is still skipped rather than reported — the root check for
//! symlinks is the one thing paging must never weaken, and a symlink whose
//! target escapes the root is still dropped (and a symlink loop still fails to
//! resolve instead of being followed).
//!
//! # Wire shape
//!
//! The reply keeps `rootPath`, `path` and `entries` exactly as they were and
//! adds `hasMore` + `nextCursor`. **What the command surface answers today is
//! the complete listing** (`list` → [`Span::All`]): the request has no cursor
//! field yet, so a caller that cannot page must not be truncated — the phone and
//! desktop panels render `entries` and nothing else, and a half-filled panel is
//! a defect, not an optimisation. Paging is ready behind [`list_page`] for the
//! caller that supplies a cursor or a limit: it answers [`DEFAULT_PAGE_SIZE`]
//! entries and a `nextCursor`, and every reply already carries `hasMore` /
//! `nextCursor` so a client can adopt it without a second host change.
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Entries in a page when the caller names no size. 200 keeps the reply well
/// inside one transport frame and its cost (one `lstat` each) under 10 ms even
/// on a cold cache.
const DEFAULT_PAGE_SIZE: usize = 200;

/// Ceiling for a caller-chosen page. A client that asks for the whole directory
/// gets the cost this paging exists to bound, so the ceiling is not optional.
const MAX_PAGE_SIZE: usize = 1_000;

/// Names that can address a FutureOS credential file, so they must go through
/// [`crate::files::ensure_path_allowed`] instead of being waved through.
///
/// Mirrors `files::is_protected_credential`, which returns false for every
/// other name (it early-returns unless the name is one of these, or
/// `config.json` directly in `~/.future/channels`). [`may_be_credential`] is
/// deliberately a *superset* of that set — `.json` names are included even when
/// they are not credentials today — because a missed name leaks a secret while
/// an extra check only costs one `canonicalize` for an entry that is on the
/// page anyway. `protected_credentials_are_not_listed` and
/// `credential_gate_covers_the_authoritative_denylist` fail if this drifts.
const CREDENTIAL_NAMES: &[&str] = &[
    "auth.json",
    "models.json",
    "remote_pairing.json",
    "remote_pairings.json",
    "config.json",
];

/// `canonicalize` calls made to resolve an entry (never the root or the listed
/// directory, which happen once per listing). Only a symlink reaches this, so
/// it is zero for a directory of plain files — the property
/// `plain_files_are_never_resolved` pins.
static ENTRY_RESOLUTIONS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
fn entry_resolutions() -> usize {
    ENTRY_RESOLUTIONS.load(Ordering::Relaxed)
}

fn resolve_entry(path: &Path) -> std::io::Result<PathBuf> {
    ENTRY_RESOLUTIONS.fetch_add(1, Ordering::Relaxed);
    path.canonicalize()
}

fn may_be_credential(name: &str) -> bool {
    CREDENTIAL_NAMES.contains(&name) || name.ends_with(".json")
}

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
    /// Additive: at least one more listable entry follows this page.
    has_more: bool,
    /// Additive: the cursor that continues this listing, `null` when the page
    /// is the last one. It is the last returned entry's `name`.
    next_cursor: Option<String>,
}

/// Windows `Path::canonicalize` returns the extended-length spelling
/// (`\\?\D:\...`). The paired phone renders these paths verbatim, so that
/// spelling would surface as the session directory instead of the `D:\...`
/// every Windows tool prints. Normalize it at this boundary, mirroring the
/// agent's sandbox path model (`agent/src/sandbox/paths.rs`);
/// `\\?\UNC\server\share` becomes `\\server\share`.
///
/// Purely textual: a canonical POSIX path never carries the prefix, so this is
/// a no-op off Windows.
fn ordinary_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => text.into_owned(),
    }
}

/// A listing span: every listable entry, or one page of them.
///
/// The command surface takes no cursor yet — `business::transfers` calls
/// [`list`], which passes [`Span::All`] — so a caller that cannot page keeps
/// getting the **complete** listing. Truncating it would be a defect, not an
/// optimisation: the phone and desktop panels render `entries` and nothing else.
/// Paging is opt-in for a caller that supplies a cursor or a limit
/// ([`list_page`]).
enum Span<'a> {
    All,
    Page {
        after: Option<&'a str>,
        limit: usize,
    },
}

impl<'a> Span<'a> {
    /// One page continuing after `after`, its size clamped so a client cannot
    /// ask for the whole directory back and restore the cost paging removed.
    ///
    /// `allow(dead_code)`: reachable only through [`list_page`], which waits on a
    /// request field that does not exist yet (see the module docs); the tests
    /// exercise both.
    #[allow(dead_code)]
    fn page(after: Option<&'a str>, limit: usize) -> Self {
        // 0 is what a numeric field defaults to on the wire, so it means "the
        // default page" rather than a one-entry page.
        let limit = if limit == 0 { DEFAULT_PAGE_SIZE } else { limit };
        Span::Page {
            after,
            limit: limit.clamp(1, MAX_PAGE_SIZE),
        }
    }
}

/// What the command surface asks for today. Split out so a test can pin the
/// decision itself: the day a client sends a cursor, this is the one line that
/// changes, and `a_directory_above_the_page_size_is_not_truncated_through_list`
/// is what fails if it changes by accident.
fn command_span() -> Span<'static> {
    Span::All
}

pub(super) fn list(session_id: &str, requested: &str) -> Result<Listing, crate::Error> {
    resolve_and_list(session_id, requested, command_span())
}

/// Page-aware entry point. The caller forwards the client's cursor here; [`list`]
/// is the unbounded listing and keeps the old signature for callers that do not
/// page. `limit` is clamped into `1..=MAX_PAGE_SIZE`, so a client that asks for
/// the whole directory gets a page.
///
/// A page is named by `after` — the previous reply's `nextCursor` — which names
/// an entry rather than an index, so entries inserted or removed between two
/// pages shift indices but not the entry the client last saw. An unknown cursor
/// (the named entry was deleted, or it was filtered out of the listing) resumes
/// where it *would* have sorted.
///
/// `allow(dead_code)`: the command surface still calls [`list`], because the
/// request carries no cursor field yet. This is the entry point that caller
/// switches to once it does (the schema, TS types and both clients are a later
/// change), and it is what the module's paging tests drive.
#[allow(dead_code)]
pub(super) fn list_page(
    session_id: &str,
    requested: &str,
    after: Option<&str>,
    limit: usize,
) -> Result<Listing, crate::Error> {
    resolve_and_list(session_id, requested, Span::page(after, limit))
}

fn resolve_and_list(
    session_id: &str,
    requested: &str,
    span: Span<'_>,
) -> Result<Listing, crate::Error> {
    if session_id.is_empty() {
        return Err("A session is required.".to_string().into());
    }
    let thread = crate::store::find_thread_by_agent_session(session_id)?
        .ok_or_else(|| "Session workspace is unavailable.".to_string())?;
    let root = crate::agent_bridge::workspace_path_for_thread(&thread.id)?;
    list_under_root(Path::new(&root), requested, span)
}

/// One directory entry as the directory itself describes it: the name and kind
/// come from `readdir` (`d_type`), with no per-entry syscall.
struct Candidate {
    /// The raw name, used to build the path — a name that is not valid UTF-8
    /// must still reach `lstat` intact.
    raw: OsString,
    /// The name the client sees, matching the pre-paging `to_string_lossy`.
    display: String,
    /// `display` lowercased, the primary sort key.
    folded: String,
    /// `d_type` says directory. `false` for a symlink even when it points at a
    /// directory: resolving it is exactly the per-entry cost paging removes, so
    /// a directory symlink is ordered with the files. It still reports
    /// `isDir: true` (its target's kind), because that is what the client needs
    /// to navigate it. Ordering only — [`describe`] re-derives the kind from
    /// `lstat` before anything is reported or dropped.
    is_dir: bool,
}

impl Candidate {
    /// Directories first, then case-insensitive name, then name — the order
    /// this listing has always produced. The leading `!is_dir` makes the tuple
    /// sort ascending with directories first, so [`resume_index`] can probe it.
    fn sort_key(&self) -> (bool, &str, &str) {
        (!self.is_dir, self.folded.as_str(), self.display.as_str())
    }
}

fn resume_index(candidates: &[Candidate], cursor: &str) -> usize {
    if let Some(index) = candidates.iter().position(|item| item.display == cursor) {
        return index + 1;
    }
    // The cursor's entry is gone (deleted between pages, or filtered out of the
    // listing). Resume where it would have sorted, treating an unknown entry as
    // a non-directory: the pages then neither repeat nor skip its neighbours.
    let folded = cursor.to_lowercase();
    let probe = (true, folded.as_str(), cursor);
    candidates
        .iter()
        .position(|item| item.sort_key() > probe)
        .unwrap_or(candidates.len())
}

fn list_under_root(root: &Path, requested: &str, span: Span<'_>) -> Result<Listing, crate::Error> {
    let root = root.canonicalize()?;
    // Empty path means the session root, never the desktop process's cwd.
    let path = root.join(requested).canonicalize()?;
    if !path.starts_with(&root) {
        return Err("Directory is outside the session workspace."
            .to_string()
            .into());
    }
    crate::files::ensure_path_allowed(&path)?;

    // One readdir pass, ordered and sliced before any entry is resolved. The
    // sort is what gives the cursor a stable meaning, so it runs over the whole
    // directory even though only one page is described: names and `d_type` are
    // already in memory from `readdir`.
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(&path)? {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let raw = entry.file_name();
        let display = raw.to_string_lossy().into_owned();
        let folded = display.to_lowercase();
        candidates.push(Candidate {
            raw,
            display,
            folded,
            is_dir: file_type.is_dir(),
        });
    }
    candidates.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    let (after, limit) = match span {
        Span::All => (None, None),
        Span::Page { after, limit } => (after, Some(limit)),
    };
    let start = match after {
        None => 0,
        Some(cursor) => resume_index(&candidates, cursor),
    };
    let mut entries = Vec::new();
    let mut has_more = false;
    for candidate in candidates.iter().skip(start) {
        let Some(entry) = describe(&root, &path, candidate) else {
            continue;
        };
        // A page describes one entry past itself: that is what proves another
        // page exists, instead of promising one and answering it empty.
        if limit == Some(entries.len()) {
            has_more = true;
            break;
        }
        entries.push(entry);
    }
    let next_cursor = if has_more {
        entries.last().map(|entry| entry.name.clone())
    } else {
        None
    };
    Ok(Listing {
        root_path: ordinary_path(&root),
        path: ordinary_path(&path),
        entries,
        has_more,
        next_cursor,
    })
}

/// The entry the client is allowed to see, or `None` when it must be skipped.
fn describe(root: &Path, dir: &Path, candidate: &Candidate) -> Option<Entry> {
    let path = dir.join(&candidate.raw);
    // One `lstat` decides kind and size. It is the authority for *whether this
    // is a symlink*, not `d_type` from the readdir pass: ordering may trust the
    // directory's own type byte because a wrong answer there only misplaces an
    // entry, while a wrong answer here would skip the root check.
    let meta = path.symlink_metadata().ok()?;
    let (is_dir, size) = if meta.file_type().is_symlink() {
        // The one entry kind whose name can resolve elsewhere: follow it, and
        // drop it if the target leaves the session root or names a protected
        // credential. A dangling or looping link fails to resolve and is
        // dropped here too. The target's own metadata decides kind and size, as
        // it always has.
        let target = resolve_entry(&path).ok()?;
        if !target.starts_with(root) || crate::files::ensure_path_allowed(&target).is_err() {
            return None;
        }
        let meta = target.metadata().ok()?;
        if !meta.is_dir() && !meta.is_file() {
            return None;
        }
        (meta.is_dir(), if meta.is_dir() { 0 } else { meta.len() })
    } else {
        // Not a symlink, so the canonical form of `path` is `path` itself: the
        // root check the old `canonicalize` performed is decided by
        // construction (the name comes from reading `dir`, which
        // `list_under_root` already proved is inside the root).
        if !meta.is_dir() && !meta.is_file() {
            // A FIFO, socket or device is not something the phone can open.
            return None;
        }
        if may_be_credential(&candidate.display)
            && crate::files::ensure_path_allowed(&path).is_err()
        {
            return None;
        }
        (meta.is_dir(), if meta.is_dir() { 0 } else { meta.len() })
    };
    Some(Entry {
        name: candidate.display.clone(),
        path: ordinary_path(&path),
        is_dir,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One bounded page of the session root, as a paging client would ask for
    /// it.
    fn first(root: &Path) -> Listing {
        list_under_root(root, "", Span::page(None, DEFAULT_PAGE_SIZE)).unwrap()
    }

    fn page(root: &Path, after: &str, limit: usize) -> Listing {
        list_under_root(root, "", Span::page(Some(after), limit)).unwrap()
    }

    /// The same directory, listed with an explicit page size and no cursor.
    fn opening(root: &Path, limit: usize) -> Listing {
        list_under_root(root, "", Span::page(None, limit)).unwrap()
    }

    /// The complete listing — the span [`list`] answers, which is what every
    /// client on the wire today gets.
    fn everything(root: &Path) -> Listing {
        list_under_root(root, "", Span::All).unwrap()
    }

    /// Opening a directory (any span): only success or the refusal matters.
    fn open(root: &Path, requested: &str) -> Result<Listing, crate::Error> {
        list_under_root(root, requested, Span::page(None, DEFAULT_PAGE_SIZE))
    }

    fn names(listing: &Listing) -> Vec<&str> {
        listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    #[test]
    fn windows_extended_length_prefix_is_normalized_away() {
        // These are the spellings `Path::canonicalize` produces on Windows and
        // the phone would otherwise render verbatim.
        assert_eq!(
            ordinary_path(Path::new(r"\\?\D:\future-os\src")),
            r"D:\future-os\src"
        );
        assert_eq!(
            ordinary_path(Path::new(r"\\?\UNC\server\share\dir")),
            r"\\server\share\dir"
        );
        // Ordinary spellings are untouched: Windows paths that were never
        // canonicalized, and every POSIX path.
        assert_eq!(ordinary_path(Path::new(r"D:\future-os")), r"D:\future-os");
        assert_eq!(ordinary_path(Path::new("/home/me/work")), "/home/me/work");
    }

    #[test]
    fn lists_session_root_and_children_but_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("z.txt"), "hello").unwrap();
        std::fs::write(root.join("A.txt"), "a").unwrap();
        std::fs::write(root.join(".hidden"), "h").unwrap();
        let listing = first(&root);
        // The phone renders these strings verbatim, so a Windows `\\?\`
        // extended-length prefix must never survive the boundary. Asserting the
        // absence (rather than equality with `canonicalize`) is what makes this
        // catch a regression: comparing two canonicalized spellings is
        // self-consistent and would pass either way.
        assert!(!listing.path.starts_with(r"\\?\"));
        assert!(!listing.root_path.starts_with(r"\\?\"));
        assert!(listing
            .entries
            .iter()
            .all(|entry| !entry.path.starts_with(r"\\?\")));
        assert_eq!(
            Path::new(&listing.path).canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
        assert_eq!(names(&listing), ["sub", ".hidden", "A.txt", "z.txt"]);
        assert!(listing.entries[0].is_dir);
        assert_eq!(listing.entries[3].size, 5);
        assert!(!listing.has_more);
        assert_eq!(listing.next_cursor, None);
        assert!(open(&root, "sub").unwrap().entries.is_empty());
        // A directory opened from the listing round-trips: the phone sends the
        // path it was shown (no extended-length prefix) back to this endpoint.
        assert!(open(&root, &listing.entries[0].path).is_ok());
        assert!(open(&root, "..").is_err());
        assert!(open(&root, &dir.path().to_string_lossy()).is_err());
        assert!(open(&root, "missing").is_err());
        assert!(open(&root, "A.txt").is_err());
        assert!(list("", "").is_err());
        assert!(list_page("", "", None, DEFAULT_PAGE_SIZE).is_err());
    }

    /// An empty directory is an empty last page, not an error and not a promise
    /// of more.
    #[test]
    fn an_empty_directory_answers_an_empty_last_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let listing = first(&root);
        assert!(listing.entries.is_empty());
        assert!(!listing.has_more);
        assert_eq!(listing.next_cursor, None);
        // Paging past the end stays empty rather than wrapping or erroring.
        let beyond = page(&root, "nothing-here", DEFAULT_PAGE_SIZE);
        assert!(beyond.entries.is_empty());
        assert!(!beyond.has_more);
    }

    /// The paging contract: a directory larger than one page is answered in
    /// pages that concatenate to the whole listing, in the same order, with the
    /// cursor naming the last entry the client saw.
    #[test]
    fn a_large_directory_pages_without_repeating_or_skipping() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(root.join("zdir")).unwrap();
        std::fs::create_dir_all(root.join("adir")).unwrap();
        let total = 25;
        for index in 0..total {
            std::fs::write(root.join(format!("file-{index:03}.txt")), "x").unwrap();
        }
        let whole = first(&root);
        assert_eq!(whole.entries.len(), total + 2);
        assert!(!whole.has_more);
        assert_eq!(whole.next_cursor, None);

        let mut walked: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let listing = match &cursor {
                None => first(&root),
                Some(cursor) => page(&root, cursor, 7),
            };
            pages += 1;
            assert!(pages <= total + 3, "paging did not terminate");
            walked.extend(names(&listing).into_iter().map(str::to_string));
            match listing.next_cursor {
                Some(next) => {
                    assert!(listing.has_more);
                    // The cursor is the last entry of the page it came from.
                    assert_eq!(
                        Some(next.clone()),
                        listing.entries.last().map(|e| e.name.clone())
                    );
                    assert!(listing.entries.len() <= 7);
                    cursor = Some(next);
                }
                None => {
                    assert!(!listing.has_more);
                    break;
                }
            }
        }
        // Same entries, same order as the single-shot listing: directories
        // first, then case-insensitive name.
        let expected: Vec<String> = names(&whole).into_iter().map(str::to_string).collect();
        assert_eq!(walked, expected);
        assert_eq!(walked.len(), total + 2);
        assert_eq!(&walked[..2], ["adir", "zdir"]);
    }

    /// The first page of a directory far larger than a page is a page: the
    /// answer's size and work are bounded by the page, not by the directory.
    #[test]
    fn a_page_is_bounded_by_the_page_size_not_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let total = 1_200;
        for index in 0..total {
            std::fs::write(root.join(format!("f{index:04}.txt")), "x").unwrap();
        }
        let listing = first(&root);
        assert_eq!(listing.entries.len(), DEFAULT_PAGE_SIZE);
        assert!(listing.has_more);
        assert_eq!(
            listing.next_cursor.as_deref(),
            Some(listing.entries.last().unwrap().name.as_str())
        );
        // The second page continues where the first stopped.
        let second = page(&root, listing.next_cursor.as_deref().unwrap(), 50);
        assert_eq!(second.entries.len(), 50);
        assert_eq!(second.entries[0].name, "f0200.txt");
        assert_eq!(second.entries[49].name, "f0249.txt");
        // A caller-chosen page is clamped, so nobody can ask for the whole
        // directory back and restore the old cost.
        let huge = opening(&root, usize::MAX);
        assert_eq!(huge.entries.len(), MAX_PAGE_SIZE);
        assert!(huge.has_more);
    }

    /// A directory above the page size is still answered **completely** to a
    /// caller that cannot page: 300 entries in, 300 entries out, no cursor and
    /// no `hasMore`, exactly as before this module learned to page. The same
    /// directory through the bounded span is two pages. This is the regression a
    /// first cut of the paging introduced (`list` answering page 1 and stranding
    /// entries 201..), and the guard that keeps it from coming back before the
    /// cursor is on the wire.
    #[test]
    fn a_directory_above_the_page_size_is_not_truncated_through_list() {
        assert!(
            matches!(command_span(), Span::All),
            "list() must answer every entry while the request carries no cursor"
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let total = 300;
        for index in 0..total {
            std::fs::write(root.join(format!("f{index:03}.txt")), "x").unwrap();
        }

        let complete = everything(&root);
        assert_eq!(complete.entries.len(), total);
        assert!(!complete.has_more);
        assert_eq!(complete.next_cursor, None);
        assert_eq!(complete.entries[0].name, "f000.txt");
        assert_eq!(complete.entries[total - 1].name, "f299.txt");

        let bounded = first(&root);
        assert_eq!(bounded.entries.len(), DEFAULT_PAGE_SIZE);
        assert!(bounded.has_more);
        assert_eq!(bounded.next_cursor.as_deref(), Some("f199.txt"));
        // A limit of 0 is what a numeric field defaults to on the wire, so it
        // means "the default page", not a one-entry page.
        let absent_limit = opening(&root, 0);
        assert_eq!(absent_limit.entries.len(), DEFAULT_PAGE_SIZE);
        assert!(absent_limit.has_more);
        // The remainder is reachable through the cursor, so nothing is stranded
        // for a caller that does page.
        let rest = page(
            &root,
            bounded.next_cursor.as_deref().unwrap(),
            DEFAULT_PAGE_SIZE,
        );
        assert_eq!(rest.entries.len(), total - DEFAULT_PAGE_SIZE);
        assert!(!rest.has_more);
        assert_eq!(rest.next_cursor, None);
        let mut walked = names(&bounded)
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        walked.extend(names(&rest).into_iter().map(str::to_string));
        assert_eq!(walked, names(&complete));
    }

    /// The cost property, pinned as a syscall count rather than a stopwatch: on
    /// a directory of plain files no entry is resolved at all (the old code
    /// canonicalized every one), whatever the directory's size.
    #[test]
    fn plain_files_are_never_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let total: usize = 4_000;
        for index in 0..total {
            std::fs::write(root.join(format!("f{index:04}.txt")), "x").unwrap();
        }
        let before = entry_resolutions();
        let listing = first(&root);
        assert_eq!(listing.entries.len(), DEFAULT_PAGE_SIZE);
        assert_eq!(
            entry_resolutions() - before,
            0,
            "a page of plain files must not resolve any entry"
        );
        // ...and the same holds for the page that walks the rest of the
        // directory: the work stays per-page, not per-directory.
        let mut cursor = listing.next_cursor.clone();
        let mut pages = 1;
        while let Some(next) = cursor {
            let listing = page(&root, &next, DEFAULT_PAGE_SIZE);
            pages += 1;
            cursor = listing.next_cursor;
        }
        // `sub` sorts first, so the directory holds `total + 1` entries.
        assert_eq!(pages, (total + 1).div_ceil(DEFAULT_PAGE_SIZE));
        assert_eq!(entry_resolutions() - before, 0);
    }

    #[test]
    fn protected_credentials_are_not_listed() {
        let _home = crate::remote::test_support::HomeGuard::new("session-file-credentials");
        let home = crate::home_dir().unwrap();
        let root = Path::new(&home).join(".future").join("agent");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("auth.json"), "secret").unwrap();
        std::fs::write(root.join("notes.txt"), "public").unwrap();
        let listing = first(&root);
        assert!(listing
            .entries
            .iter()
            .all(|entry| entry.name != "auth.json"));
        assert!(listing
            .entries
            .iter()
            .any(|entry| entry.name == "notes.txt"));
        assert!(open(&root, "auth.json").is_err());
    }

    /// The cheap gate must never wave through a name the authoritative denylist
    /// rejects: paging skips `ensure_path_allowed` for ordinary names, which is
    /// only sound while every credential name is in [`CREDENTIAL_NAMES`] (or
    /// ends in `.json`). If `files::is_protected_credential` grows a name this
    /// test's second assertion catches it for these spellings.
    #[test]
    fn credential_gate_covers_the_authoritative_denylist() {
        let _home = crate::remote::test_support::HomeGuard::new("session-file-gate");
        let home = crate::home_dir().unwrap();
        let future = Path::new(&home).join(".future");
        let cases = [
            (future.join("agent").join("auth.json"), "auth.json"),
            (future.join("agent").join("models.json"), "models.json"),
            (future.join("agent-app").join("auth.json"), "auth.json"),
            (future.join("channels").join("config.json"), "config.json"),
            (future.join("remote_pairing.json"), "remote_pairing.json"),
        ];
        for (path, name) in cases {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "secret").unwrap();
            assert!(may_be_credential(name), "{name} must reach the real check");
            assert!(
                crate::files::ensure_path_allowed(&path).is_err(),
                "{name} is denied by the authoritative check"
            );
        }
        // The names the gate skips are ones the authoritative check allows, so
        // skipping it changes no answer.
        let plain = future.join("agent").join("notes.txt");
        std::fs::write(&plain, "public").unwrap();
        assert!(!may_be_credential("notes.txt"));
        assert!(crate::files::ensure_path_allowed(&plain).is_ok());
        // A directory holding a protected credential still lists its siblings
        // and drops the credential, through the gate.
        let listing = first(&future.join("agent"));
        assert!(names(&listing).contains(&"notes.txt"));
        assert!(!names(&listing).contains(&"auth.json"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("escape")).unwrap();
        assert!(open(&root, "escape").is_err());
        assert!(first(&root).entries.is_empty());
    }

    /// A symlink that stays inside the root is listed, and reports its target's
    /// kind and size — the boundary is the root, not the entry kind.
    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_the_root_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("real.txt"), "hello").unwrap();
        std::os::unix::fs::symlink(root.join("sub"), root.join("sub-link")).unwrap();
        let listing = first(&root);
        assert_eq!(names(&listing), ["sub", "sub-link"]);
        // The link resolves to a directory, so the phone can open it even
        // though `d_type` (which orders the page) calls it a symlink.
        let link = &listing.entries[1];
        assert!(link.is_dir);
        assert_eq!(link.size, 0);
        assert_eq!(link.name, "sub-link");
        assert!(open(&root, &link.path).is_ok());
    }

    /// A symlink loop fails to resolve instead of being followed: the entry is
    /// skipped and the listing still answers.
    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_does_not_hang() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("real.txt"), "hello").unwrap();
        std::os::unix::fs::symlink(root.join("loop-b"), root.join("loop-a")).unwrap();
        std::os::unix::fs::symlink(root.join("loop-a"), root.join("loop-b")).unwrap();
        std::os::unix::fs::symlink(root.join("self"), root.join("self")).unwrap();
        let listing = first(&root);
        assert_eq!(names(&listing), ["real.txt"]);
        // The loop is only skipped as an entry; opening it as a directory is
        // still refused rather than followed.
        assert!(open(&root, "loop-a").is_err());
    }

    /// An entry the boundary cannot describe is skipped, not reported: a
    /// dangling symlink resolves to nothing, and a FIFO is neither a file nor a
    /// directory, so neither is something the phone can open. Skipping one must
    /// not cost the listing its readable neighbours — a working directory that
    /// holds a pipe is ordinary.
    #[cfg(unix)]
    #[test]
    fn entries_that_are_neither_files_nor_directories_are_skipped() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("real.txt"), "hello").unwrap();
        // A symlink whose target does not exist: `canonicalize` fails.
        std::os::unix::fs::symlink(root.join("gone"), root.join("dangling")).unwrap();
        // A FIFO: metadata says neither `is_file` nor `is_dir`.
        let fifo = root.join("pipe");
        let path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);

        let listing = first(&root);
        let names: Vec<&str> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["real.txt"],
            "only the readable file is listed: {names:?}"
        );
        assert_eq!(listing.entries[0].size, 5);
    }

    /// Skipped entries must not eat into a page: a page is a page of *listable*
    /// entries, so a directory whose first entries are escapes still fills.
    #[cfg(unix)]
    #[test]
    fn skipped_entries_do_not_shorten_a_page() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        // 10 escapes sort before the readable files by name.
        for index in 0..10 {
            std::os::unix::fs::symlink(&outside, root.join(format!("a-escape-{index}"))).unwrap();
        }
        for index in 0..5 {
            std::fs::write(root.join(format!("z-file-{index}.txt")), "x").unwrap();
        }
        let listing = page(&root, "a-escape-0", 5);
        assert_eq!(listing.entries.len(), 5);
        assert!(!listing.has_more);
        assert!(names(&listing)
            .iter()
            .all(|name| name.starts_with("z-file-")));
    }

    /// Manual evidence for the acceptance measurement (5,000 files, first page
    /// ≤ 20 ms, plus what the complete listing costs). Ignored because a
    /// wall-clock assertion in CI flakes; run with
    /// `cargo test --release -p future-host --lib plain_directory_first_page_cost -- --ignored --nocapture`.
    ///
    /// The audit's second criterion — "10 entries vs 5,000 entries differ by
    /// ≤ 3×" — is **not** asserted here because no ordered listing can meet it:
    /// this listing is alphabetical, so a page must read every name in the
    /// directory, and merely iterating 5,000 directory entries (no `lstat`, no
    /// name copy: measured below as [`scan_floor`]) already costs ≈1.6 ms
    /// against ≈0.05 ms for a 10-entry directory — a 30× floor before any work
    /// is done. What is asserted is the budget itself (20 ms; the 100 ms
    /// command budget with 5× margin) and that the page costs little more than
    /// reading the names, i.e. the per-entry resolution cost that made this
    /// command 229 ms at this size is gone.
    #[test]
    #[ignore]
    fn plain_directory_first_page_cost() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small");
        std::fs::create_dir(&small).unwrap();
        for index in 0..10 {
            std::fs::write(small.join(format!("f{index}.txt")), "x").unwrap();
        }
        let big = dir.path().join("big");
        std::fs::create_dir(&big).unwrap();
        for index in 0..5_000 {
            std::fs::write(big.join(format!("f{index:05}.txt")), "x").unwrap();
        }
        let measure = |root: &Path, expected: usize| {
            let mut samples = Vec::new();
            for _ in 0..20 {
                let started = std::time::Instant::now();
                let listing = first(root);
                assert_eq!(listing.entries.len(), expected);
                samples.push(started.elapsed());
            }
            samples.sort();
            samples[samples.len() / 2]
        };
        let small_ms = measure(&small, 10).as_secs_f64() * 1_000.0;
        let big_ms = measure(&big, DEFAULT_PAGE_SIZE).as_secs_f64() * 1_000.0;
        let floor_ms = scan_floor(&big);
        // What a client on today's wire actually gets: the complete listing,
        // because the request carries no cursor yet. Its cost is the interesting
        // number: one readdir, one `lstat` per entry, one sort, no canonicalize
        // and no credential check per entry.
        let mut complete_samples = Vec::new();
        for _ in 0..10 {
            let started = std::time::Instant::now();
            let listing = everything(&big);
            assert_eq!(listing.entries.len(), 5_000);
            assert!(!listing.has_more);
            complete_samples.push(started.elapsed());
        }
        complete_samples.sort();
        let complete_ms = complete_samples[complete_samples.len() / 2].as_secs_f64() * 1_000.0;
        println!(
            "first page: 10 entries {small_ms:.3} ms, 5,000 entries {big_ms:.3} ms\n\
             readdir floor for the same 5,000 entries: {floor_ms:.3} ms (\
             ratio to the 10-entry page: {:.1}x)\n\
             complete 5,000-entry listing (what list() answers today): {complete_ms:.3} ms",
            big_ms / small_ms
        );
        assert!(big_ms <= 20.0, "5,000-file first page took {big_ms:.2} ms");
        // The page describes 200 entries and orders the directory, but it must
        // not pay the old per-entry resolution again: little more than the
        // readdir pass the ordering alone requires.
        assert!(
            big_ms <= floor_ms * 2.5,
            "5,000-entry first page {big_ms:.2} ms vs {floor_ms:.2} ms readdir floor"
        );
        // Even the untruncated listing must stay far inside the 100 ms command
        // budget: the canonicalize work that made it 229 ms is gone.
        assert!(
            complete_ms <= 40.0,
            "complete 5,000-entry listing took {complete_ms:.2} ms"
        );
    }

    /// Iterating a directory without touching any entry beyond its name — the
    /// floor any alphabetical page must pay.
    #[cfg(test)]
    fn scan_floor(root: &Path) -> f64 {
        let mut samples = Vec::new();
        for _ in 0..20 {
            let started = std::time::Instant::now();
            let mut seen = 0;
            for entry in std::fs::read_dir(root).unwrap() {
                if entry.is_ok() {
                    seen += 1;
                }
            }
            assert!(seen > 0);
            samples.push(started.elapsed());
        }
        samples.sort();
        samples[samples.len() / 2].as_secs_f64() * 1_000.0
    }
}
