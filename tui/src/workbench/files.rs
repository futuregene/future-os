//! The 文件 panel's tree, and what each file can be previewed as.
//!
//! The panel shows the session workspace as a tree: one row per visible entry,
//! directories first, their children indented beneath them once expanded. It is
//! *flat*, not recursive — [`FileTree::rows`] is the exact list the panel paints,
//! which is what makes a click resolve to a row instead of a path that has to be
//! looked up again.
//!
//! Two rules keep it usable on a real repository:
//!
//! * **A directory is read when it is opened**, never before. Scanning a tree
//!   eagerly is what makes a file browser stall on `node_modules`.
//! * **A listing is bounded** ([`MAX_ENTRIES_PER_DIR`]). A directory with ten
//!   thousand files says so and lists the first slice; the panel must not become
//!   a spinner.
//!
//! Nothing here reads file *contents* except [`preview_kind`], which sniffs the
//! first bytes to tell an image from text — the extension alone lies both ways.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How many entries one directory contributes to the panel.
pub const MAX_ENTRIES_PER_DIR: usize = 400;

/// The largest file [`preview_kind`] will call text. A preview of a 200 MB log
/// is not a preview.
pub const MAX_TEXT_PREVIEW_BYTES: u64 = 512 * 1024;

/// One painted row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNode {
    /// Absolute or workspace-relative path, exactly as the filesystem spelled it.
    pub path: PathBuf,
    /// The last path segment — what the row prints.
    pub name: String,
    /// Nesting depth: 0 for a child of the workspace root.
    pub depth: usize,
    pub kind: NodeKind,
    /// Directories only: is it open?
    pub expanded: bool,
    /// Its size in bytes, when one read told us (files only).
    pub size: u64,
    /// The directory held more entries than the panel lists.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    File,
}

impl FileNode {
    pub fn is_dir(&self) -> bool {
        self.kind == NodeKind::Dir
    }
}

/// What a preview can show for a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// A picture the terminal can draw as a thumbnail.
    Image,
    /// Markdown, rendered.
    Markdown,
    /// Any other readable text, shown as lines.
    Text,
    /// Nothing to show: a binary, or a file past the preview cap.
    Opaque,
}

impl PreviewKind {
    pub fn shows_something(self) -> bool {
        !matches!(self, Self::Opaque)
    }
}

/// The panel's tree: the visible rows plus the directories that are open.
///
/// `rows` is the *painted* list and stays in sync with `expanded` by
/// construction — every mutation goes through [`FileTree::open`],
/// [`FileTree::close`] and [`FileTree::rescan`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileTree {
    /// The workspace the tree describes.
    pub root: String,
    rows: Vec<FileNode>,
    expanded: BTreeSet<PathBuf>,
    /// The root directory held more entries than the panel lists.
    pub truncated: bool,
    /// Why the root could not be read, when it could not.
    pub error: Option<String>,
}

impl FileTree {
    /// Read `root`'s first level. Nothing else: children arrive as they open.
    pub fn scan(root: &str) -> Self {
        let mut tree = Self {
            root: root.to_string(),
            rows: Vec::new(),
            expanded: BTreeSet::new(),
            truncated: false,
            error: None,
        };
        if root.trim().is_empty() {
            return tree;
        }
        match read_dir(Path::new(root), 0) {
            Ok((nodes, truncated)) => {
                tree.rows = nodes;
                tree.truncated = truncated;
            }
            Err(error) => tree.error = Some(error),
        }
        tree
    }

    /// The rows the panel paints, in order.
    pub fn rows(&self) -> &[FileNode] {
        &self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The row at `index`.
    pub fn get(&self, index: usize) -> Option<&FileNode> {
        self.rows.get(index)
    }

    /// A tree built from rows in hand, without touching the filesystem.
    ///
    /// The demo and the screenshot scenarios need a believable tree that does
    /// not depend on whose checkout is rendered; the panel cannot tell the
    /// difference, which is what makes it a usable fixture.
    pub fn from_rows(root: &str, rows: Vec<FileNode>) -> Self {
        let expanded = rows
            .iter()
            .filter(|row| row.is_dir() && row.expanded)
            .map(|row| row.path.clone())
            .collect();
        Self {
            root: root.to_string(),
            rows,
            expanded,
            truncated: false,
            error: None,
        }
    }

    /// The row whose path is `path`.
    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|row| row.path == path)
    }

    /// The row whose path is the string `path`.
    pub fn index_of_str(&self, path: &str) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.path.to_string_lossy() == path)
    }

    /// Is `path` open?
    pub fn is_open(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Open a directory: read its children and splice them in beneath it.
    ///
    /// A path that is not a directory, or is already open, is left alone —
    /// opening is idempotent so a click and a key that both fire cannot fold a
    /// directory twice.
    pub fn open(&mut self, path: &Path) {
        let Some(index) = self.index_of(path) else {
            return;
        };
        if !self.rows[index].is_dir() || self.is_open(path) {
            return;
        }
        let depth = self.rows[index].depth + 1;
        let (children, truncated) = match read_dir(path, depth) {
            Ok(read) => read,
            Err(error) => {
                self.rows[index].truncated = false;
                self.error = Some(error);
                return;
            }
        };
        self.rows[index].expanded = true;
        self.rows[index].truncated = truncated;
        self.expanded.insert(path.to_path_buf());
        // Splice: everything from `index + 1` moves down by `children.len()`.
        let tail = self.rows.split_off(index + 1);
        self.rows.extend(children);
        self.rows.extend(tail);
    }

    /// Close a directory, dropping the whole subtree it had opened.
    pub fn close(&mut self, path: &Path) {
        let Some(index) = self.index_of(path) else {
            return;
        };
        if !self.is_open(path) {
            return;
        }
        self.expanded.remove(path);
        self.rows[index].expanded = false;
        let depth = self.rows[index].depth;
        let end = self.rows[index + 1..]
            .iter()
            .position(|row| row.depth <= depth)
            .map(|offset| index + 1 + offset)
            .unwrap_or(self.rows.len());
        let dropped: Vec<PathBuf> = self.rows[index + 1..end]
            .iter()
            .filter(|row| row.is_dir())
            .map(|row| row.path.clone())
            .collect();
        for gone in dropped {
            self.expanded.remove(&gone);
        }
        self.rows.drain(index + 1..end);
    }

    /// Open or close, whichever the row is not.
    pub fn toggle(&mut self, path: &Path) {
        if self.is_open(path) {
            self.close(path);
        } else {
            self.open(path);
        }
    }

    /// Re-read everything that was open, keeping the open set.
    ///
    /// This is what `r` in the panel runs: a build that wrote new files should
    /// not need a restart to appear.
    pub fn rescan(&mut self) {
        let open: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        let root = self.root.clone();
        let mut fresh = Self::scan(&root);
        for path in open {
            if fresh.index_of(&path).is_some() {
                fresh.open(&path);
            }
        }
        *self = fresh;
    }

    /// One line describing what the tree currently holds, for the panel heading.
    pub fn counts(&self) -> (usize, usize) {
        let dirs = self.rows.iter().filter(|row| row.is_dir()).count();
        (dirs, self.rows.len() - dirs)
    }
}

/// Read one directory: its entries sorted, directories first.
fn read_dir(path: &Path, depth: usize) -> Result<(Vec<FileNode>, bool), String> {
    let entries =
        std::fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut nodes: Vec<FileNode> = Vec::new();
    let mut truncated = false;
    for entry in entries.flatten() {
        if nodes.len() >= MAX_ENTRIES_PER_DIR {
            truncated = true;
            break;
        }
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            continue;
        };
        // `metadata` follows a symlink: a link to a directory navigates like the
        // directory it names, which is what a reader expects from a tree.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let kind = if meta.is_dir() {
            NodeKind::Dir
        } else {
            NodeKind::File
        };
        nodes.push(FileNode {
            path,
            name,
            depth,
            kind,
            expanded: false,
            size: if kind == NodeKind::File {
                meta.len()
            } else {
                0
            },
            truncated: false,
        });
    }
    nodes.sort_by(|a, b| {
        // Directories lead, then case-insensitive by name: the panel's order is
        // the reader's order, and re-deriving it per frame would be a second
        // ordering to keep in step.
        let by_kind = (b.kind == NodeKind::Dir).cmp(&(a.kind == NodeKind::Dir));
        by_kind.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok((nodes, truncated))
}

/// What a path can be previewed as: sniffed from its bytes, not its name.
///
/// An image is decided by magic bytes (`paste::sniff_image_format`), because a
/// text file named `shot.png` is not a picture. Everything else is text when it
/// decodes as UTF-8 and holds no NUL byte — the same rule `grep` uses — and
/// opaque when it is bigger than [`MAX_TEXT_PREVIEW_BYTES`].
pub fn preview_kind(path: &Path) -> PreviewKind {
    let Ok(meta) = std::fs::metadata(path) else {
        return PreviewKind::Opaque;
    };
    if !meta.is_file() {
        return PreviewKind::Opaque;
    }
    let head = read_head(path, 12);
    if let Some(_format) = crate::paste::sniff_image_format(&head) {
        return PreviewKind::Image;
    }
    if meta.len() > MAX_TEXT_PREVIEW_BYTES {
        return PreviewKind::Opaque;
    }
    let sample = read_head(path, MAX_TEXT_PREVIEW_BYTES as usize);
    if sample.contains(&0) || std::str::from_utf8(&sample).is_err() {
        return PreviewKind::Opaque;
    }
    let is_markdown = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown"));
    if is_markdown {
        PreviewKind::Markdown
    } else {
        PreviewKind::Text
    }
}

fn read_head(path: &Path, limit: usize) -> Vec<u8> {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buffer: Vec<u8> = Vec::with_capacity(limit.min(64 * 1024));
    let _ = file.take(limit as u64).read_to_end(&mut buffer);
    buffer
}

/// A short human size for a row's tail (`1.2k`, `3.4M`).
pub fn short_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("G", 1 << 30), ("M", 1 << 20), ("k", 1 << 10)];
    for (suffix, scale) in UNITS {
        if bytes >= scale {
            return format!("{:.1}{suffix}", bytes as f64 / scale as f64);
        }
    }
    format!("{bytes}b")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::create_dir(dir.path().join("src").join("deep")).unwrap();
        std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("src").join("deep").join("x.txt"), "x").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hello\n").unwrap();
        std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(dir.path().join("fake.png"), "not a picture\n").unwrap();
        dir
    }

    fn root_of(dir: &tempfile::TempDir) -> String {
        dir.path().to_string_lossy().to_string()
    }

    #[test]
    fn a_scan_lists_one_level_directories_first() {
        let dir = tree_fixture();
        let tree = FileTree::scan(&root_of(&dir));
        let names: Vec<&str> = tree.rows().iter().map(|row| row.name.as_str()).collect();
        // Directories lead; files follow, case-insensitively.
        assert_eq!(names[0], "src");
        assert!(names.contains(&"README.md"), "{names:?}");
        assert!(names.contains(&"notes.txt"), "{names:?}");
        // Only the first level: `main.rs` is inside `src`.
        assert!(!names.contains(&"main.rs"), "{names:?}");
        assert!(tree.rows().iter().all(|row| row.depth == 0));
    }

    #[test]
    fn opening_a_directory_splices_its_children_and_closing_drops_them() {
        let dir = tree_fixture();
        let mut tree = FileTree::scan(&root_of(&dir));
        let src = dir.path().join("src");
        let index = tree.index_of(&src).expect("src");
        tree.open(&src);
        assert!(tree.is_open(&src));
        assert!(tree.rows()[index].expanded);
        // Children sit directly under their directory, indented by one.
        assert_eq!(tree.rows()[index + 1].depth, 1);
        assert_eq!(tree.rows()[index + 1].name, "deep");
        assert_eq!(tree.rows()[index + 2].name, "main.rs");
        // …and nothing beyond them moved out of place.
        assert!(tree
            .rows()
            .iter()
            .any(|row| row.name == "README.md" && row.depth == 0));

        // Opening is idempotent: a click and a key cannot double the children.
        let before = tree.rows().len();
        tree.open(&src);
        assert_eq!(tree.rows().len(), before);

        // Closing removes the children (and the nested open set with them).
        tree.open(&dir.path().join("src").join("deep"));
        tree.close(&src);
        assert!(!tree.is_open(&src));
        assert!(!tree.is_open(&dir.path().join("src").join("deep")));
        assert!(tree.rows().iter().all(|row| row.depth == 0));
    }

    #[test]
    fn a_closed_parent_forgets_the_directories_it_had_open() {
        let dir = tree_fixture();
        let mut tree = FileTree::scan(&root_of(&dir));
        let src = dir.path().join("src");
        let deep = src.join("deep");
        tree.open(&src);
        tree.open(&deep);
        assert!(tree.is_open(&deep));
        tree.toggle(&src);
        assert!(!tree.is_open(&deep), "a hidden row cannot stay open");
        // Toggling back opens the parent only; the child stays closed until it
        // is opened again.
        tree.toggle(&src);
        assert!(tree.is_open(&src) && !tree.is_open(&deep));
    }

    #[test]
    fn a_rescan_keeps_what_was_open() {
        let dir = tree_fixture();
        let mut tree = FileTree::scan(&root_of(&dir));
        let src = dir.path().join("src");
        tree.open(&src);
        std::fs::write(src.join("new.rs"), "// new\n").unwrap();
        tree.rescan();
        assert!(tree.is_open(&src));
        assert!(
            tree.rows().iter().any(|row| row.name == "new.rs"),
            "{:?}",
            tree.rows()
        );
    }

    #[test]
    fn an_empty_or_missing_root_says_so_instead_of_panicking() {
        let tree = FileTree::scan("");
        assert!(tree.is_empty());
        assert_eq!(tree.error, None, "no workspace is not an error");
        let tree = FileTree::scan("/definitely/not/here/at/all");
        assert!(tree.is_empty());
        assert!(tree.error.is_some());
    }

    #[test]
    fn a_preview_kind_is_decided_by_bytes_and_the_cap() {
        let dir = tree_fixture();
        let png = dir.path().join("real.png");
        std::fs::write(&png, png_bytes()).unwrap();
        assert_eq!(preview_kind(&png), PreviewKind::Image);
        // A text file named `.png` is text, not a broken picture.
        assert_eq!(
            preview_kind(&dir.path().join("fake.png")),
            PreviewKind::Text
        );
        assert_eq!(
            preview_kind(&dir.path().join("README.md")),
            PreviewKind::Markdown
        );
        assert_eq!(
            preview_kind(&dir.path().join("notes.txt")),
            PreviewKind::Text
        );
        assert_eq!(
            preview_kind(&dir.path().join("blob.bin")),
            PreviewKind::Opaque
        );
        assert_eq!(preview_kind(&dir.path().join("src")), PreviewKind::Opaque);

        // Past the cap even a text file is not previewed.
        let big = dir.path().join("big.log");
        std::fs::write(&big, "y".repeat(MAX_TEXT_PREVIEW_BYTES as usize + 1)).unwrap();
        assert_eq!(preview_kind(&big), PreviewKind::Opaque);
    }

    #[test]
    fn a_size_reads_in_three_quarters_of_a_cell() {
        assert_eq!(short_size(999), "999b");
        assert_eq!(short_size(1024), "1.0k");
        assert_eq!(short_size(1024 * 1024 * 2), "2.0M");
        assert_eq!(short_size(3 * (1 << 30)), "3.0G");
    }

    /// A 1×1 PNG: the smallest real picture, enough for the sniffer.
    fn png_bytes() -> Vec<u8> {
        const B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(B64)
            .expect("fixture")
    }
}
