//! Keep the workbench bilingual: a failing test when a piece of chrome is a
//! string literal instead of a row in `workbench/i18n.rs`.
//!
//! `i18n.rs` makes a *half-translated key* uncompilable. It says nothing about a
//! string that never went through the table, and that is how the surface drifts
//! back to one language: every branch that adds a panel adds its own labels, and
//! a hardcoded literal compiles and looks finished in whichever language its
//! author happened to write.
//!
//! So this test reads the sources and looks for two shapes of literal:
//!
//! 1. **Anything with a CJK character, anywhere under `tui/src/`.** The chrome is
//!    authored in Chinese here, so a Chinese literal is a chrome literal that
//!    skipped the table, whichever module it sits in. Content is untouched: a
//!    conversation, a session title or a path is runtime data, never a literal.
//! 2. **An English sentence drawn by the surface** (`SURFACE`): two whitespace-
//!    separated tokens holding three or more letters, in a literal that is not a
//!    path, an identifier, an escape sequence or a format skeleton. Deliberately
//!    conservative — an alarm every week gets switched off, which is worse than a
//!    miss. Its blind spots are known and listed in
//!    `docs/tui/i18n-homeless-strings.md`: a one-word label (`queued`, `Manual`)
//!    is not a sentence and slips through.
//!
//! Test code is not scanned: it never draws a frame, and its literals are
//! expectations *about* the table rather than text on screen.
//!
//! # What to do when it fails
//!
//! Add a variant to `Key` and a row to `pair()` in
//! `tui/src/workbench/i18n.rs`, then draw it with `t(lang, Key::…)`. A literal
//! that is genuinely not chrome gets a row in `allowlist.rs` saying why, or a
//! same-line `// i18n: <reason>` marker; `BUDGET` in that file counts both.
//!
//! # Checking another tree
//!
//! `FUTURE_I18N_GUARD_ROOT=<tree>/tui/src cargo test -p future-tui --test
//! i18n_guard -- --nocapture` runs the same rules over another checkout, which
//! is how the branches waiting to merge were measured before this landed. Only
//! the violation rule applies to a tree that is not this one: the allowance
//! baseline describes the file's own tree.

mod allowlist;

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use allowlist::{Why, ALLOW, BUDGET, FILES};

/// The modules that draw the full-screen surface — where a sentence in English
/// is a sentence a reader sees. `workbench/` is the surface itself; the rest are
/// the modules it pulls display text out of (`chat_area` draws the
/// conversation, `themes` and `approval_mode` name the picker's rows,
/// `session_rows` labels the history's compaction).
const SURFACE: &[&str] = &[
    "workbench/",
    "components/chat_area.rs",
    "session_rows.rs",
    "themes.rs",
    "approval_mode.rs",
];

/// The table itself: its literals *are* the two languages, so it is not scanned.
const TABLE: &str = "workbench/i18n.rs";

/// Points the scan at another tree's `src` directory (see the module doc).
const ROOT_ENV: &str = "FUTURE_I18N_GUARD_ROOT";

/// The comment marker that exempts one literal in place, reason included:
/// `"the frame must be h rows" // i18n: an invariant's panic, never drawn`. A
/// marker without a reason is reported rather than honoured.
const MARKER: &str = "i18n:";

#[test]
fn chrome_is_drawn_from_the_table() {
    let overridden = std::env::var_os(ROOT_ENV).map(PathBuf::from);
    let root = match &overridden {
        Some(dir) => dir.clone(),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
    };

    let scan = scan(&root);
    assert!(
        scan.files > 0,
        "no Rust sources under {} — is the path right?",
        root.display()
    );
    print_summary(&scan, &root);

    let mut problems = String::new();

    let violations = scan.violations();
    if !violations.is_empty() {
        problems.push_str(&format!(
            "{} string literal(s) are chrome that never reached the table:\n\n",
            violations.len()
        ));
        for candidate in violations {
            problems.push_str(&format!(
                "  {}:{}: {:?}\n      {}\n",
                candidate.path,
                candidate.line,
                candidate.literal,
                candidate.rule.because(),
            ));
        }
        problems.push_str(
            "\n  Add a row to `pair()` and a variant to `Key` in\n  \
             tui/src/workbench/i18n.rs, then draw it with `t(lang, Key::…)`.\n  \
             If it is not chrome, add a row to tui/tests/i18n_guard/allowlist.rs\n  \
             with the reason, or put `// i18n: <reason>` on the line.\n",
        );
    }

    if overridden.is_none() {
        if !scan.stale_rows.is_empty() {
            problems.push_str(&format!(
                "\n{} allowlist row(s) match nothing any more — delete them \
                 (and lower BUDGET):\n",
                scan.stale_rows.len()
            ));
            for row in &scan.stale_rows {
                problems.push_str(&format!("  {}: {:?}\n", row.path, row.literal));
            }
        }
        if !scan.stale_files.is_empty() {
            problems.push_str(&format!(
                "\n{} file exemption(s) cover a file with nothing to exempt — \
                 delete them (and lower BUDGET):\n",
                scan.stale_files.len()
            ));
            for file in &scan.stale_files {
                problems.push_str(&format!("  {}\n", file.path));
            }
        }
        if !scan.unreasoned_markers.is_empty() {
            problems.push_str(&format!(
                "\n{} `{MARKER}` marker(s) carry no reason — an exemption has to \
                 say why it is not chrome:\n",
                scan.unreasoned_markers.len()
            ));
            for (path, line) in &scan.unreasoned_markers {
                problems.push_str(&format!("  {path}:{line}\n"));
            }
        }
        let exemptions = scan.exemptions();
        if exemptions != BUDGET {
            let (direction, verb) = if exemptions > BUDGET {
                ("raised", "Raise")
            } else {
                ("closed", "Lower")
            };
            problems.push_str(&format!(
                "\n{exemptions} exemptions, but BUDGET says {BUDGET}. An exemption \
                 was {direction}.\n  {verb} `BUDGET` in tui/tests/i18n_guard/allowlist.rs \
                 to {exemptions} — and if it went up,\n  make sure a reviewer agrees the \
                 new one is not chrome.\n"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "the workbench's chrome is drifting out of the table\n{problems}"
    );
}

/// One literal the rules picked up.
struct Candidate {
    path: String,
    line: usize,
    literal: String,
    rule: Rule,
}

/// Which rule caught it — the two fail for different reasons, so the message
/// says which one fired.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// A CJK character in a literal outside the table.
    Chinese,
    /// An English sentence drawn by the surface.
    Sentence,
}

impl Rule {
    fn because(self) -> &'static str {
        match self {
            Self::Chinese => {
                "a Chinese literal: chrome is written in Chinese in this tree, so this is \
                 chrome that skipped the table"
            }
            Self::Sentence => {
                "an English sentence the surface draws: a reader who picked 中文 sees English"
            }
        }
    }
}

/// What one pass over the tree found.
struct Scan {
    files: usize,
    /// String literals outside test code, in every scanned file.
    literals: usize,
    /// Everything the rules picked up, exempt or not.
    candidates: Vec<Candidate>,
    /// Parallel to `candidates`: the reason each was exempt, if it was.
    exempt: Vec<Option<String>>,
    /// Allowlist rows that matched nothing.
    stale_rows: Vec<&'static allowlist::Allow>,
    /// File exemptions whose file had nothing to exempt.
    stale_files: Vec<&'static allowlist::FileExempt>,
    /// `// i18n:` markers with an empty reason.
    unreasoned_markers: Vec<(String, usize)>,
    /// How many allowlist rows matched at least one literal.
    rows_used: usize,
    /// How many file exemptions covered a file with candidates.
    files_used: usize,
    /// How many literals were exempted by an in-source marker.
    markers_used: usize,
}

impl Scan {
    /// The number of exemptions the tree's own mechanisms add up to — the
    /// quantity [`BUDGET`] pins.
    fn exemptions(&self) -> usize {
        self.rows_used + self.files_used + self.markers_used
    }

    /// The candidates nothing exempted.
    fn violations(&self) -> Vec<&Candidate> {
        self.candidates
            .iter()
            .zip(&self.exempt)
            .filter(|(_, exempt)| exempt.is_none())
            .map(|(candidate, _)| candidate)
            .collect()
    }
}

fn scan(root: &Path) -> Scan {
    let paths = rust_files(root);
    let mut scan = Scan {
        files: paths.len(),
        literals: 0,
        candidates: Vec::new(),
        exempt: Vec::new(),
        stale_rows: Vec::new(),
        stale_files: Vec::new(),
        unreasoned_markers: Vec::new(),
        rows_used: 0,
        files_used: 0,
        markers_used: 0,
    };
    let mut row_used = vec![false; ALLOW.len()];
    let mut file_used = vec![false; FILES.len()];

    for path in &paths {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative == TABLE {
            continue;
        }
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            // A source file that is not UTF-8 is not this test's business.
            Err(_) => continue,
        };
        let lexed = lex(&source);
        let tests = test_regions(&source, &lexed.mask);

        for literal in &lexed.strings {
            if tests
                .iter()
                .any(|region| region.contains(&literal.content.start))
            {
                continue;
            }
            let raw = &source[literal.content.clone()];
            scan.literals += 1;
            let Some(rule) = rule_for(&relative, raw) else {
                continue;
            };
            let line = line_of(&lexed.newlines, literal.content.start);
            let candidate = Candidate {
                path: relative.clone(),
                line,
                literal: raw.to_string(),
                rule,
            };

            // 1. A whole file that is content rather than chrome.
            // 2. A marker on the line: the exemption travels with the code.
            // 3. The allowlist.
            let exempt = if let Some(index) = FILES.iter().position(|file| file.path == relative) {
                file_used[index] = true;
                Some(FILES[index].why.to_string())
            } else if let Some(marker) = marker_on_line(&lexed, &source, literal.content.clone()) {
                match marker {
                    Some(reason) => {
                        scan.markers_used += 1;
                        Some(reason)
                    }
                    // `// i18n:` with nothing after it: not an exemption.
                    None => {
                        scan.unreasoned_markers.push((relative.clone(), line));
                        None
                    }
                }
            } else if let Some(index) = ALLOW
                .iter()
                .position(|row| row.path == candidate.path && row.literal == candidate.literal)
            {
                row_used[index] = true;
                Some(why_text(&ALLOW[index].why).to_string())
            } else {
                None
            };
            scan.candidates.push(candidate);
            scan.exempt.push(exempt);
        }
    }

    scan.rows_used = row_used.iter().filter(|used| **used).count();
    scan.files_used = file_used.iter().filter(|used| **used).count();
    scan.stale_rows = ALLOW
        .iter()
        .zip(&row_used)
        .filter(|(_, used)| !**used)
        .map(|(row, _)| row)
        .collect();
    scan.stale_files = FILES
        .iter()
        .zip(&file_used)
        .filter(|(_, used)| !**used)
        .map(|(file, _)| file)
        .collect();
    scan
}

fn why_text(why: &Why) -> &'static str {
    match why {
        Why::NotChrome(text) | Why::KnownGap(text) => text,
    }
}

/// The rule that catches this literal, if any.
fn rule_for(path: &str, literal: &str) -> Option<Rule> {
    if literal.chars().any(is_cjk) {
        return Some(Rule::Chinese);
    }
    if is_surface(path) && prose_like(literal) {
        return Some(Rule::Sentence);
    }
    None
}

fn is_surface(path: &str) -> bool {
    SURFACE.iter().any(|entry| match entry.strip_suffix('/') {
        Some(dir) => path.starts_with(dir),
        None => path == *entry,
    })
}

/// A CJK character: Han (either plane), kana, Hangul, or the CJK/fullwidth
/// punctuation and forms around them.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x2E80..=0x2EFF
        | 0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFFEF
        | 0x20000..=0x2FA1F)
}

/// Whether a literal reads like a sentence someone typed for a reader.
///
/// Everything here is a way of *not* being a sentence, tuned so the shapes that
/// fill this tree are quiet: escape sequences, paths and wire keys, identifiers
/// and module paths, and format skeletons that are nothing but holes and spaces.
/// What is left — `No models to choose from yet`, `no workspace yet` — is prose.
fn prose_like(literal: &str) -> bool {
    if literal.contains('\\') || literal.contains('\n') || literal.contains('\t') {
        return false;
    }
    if literal.contains('/') || literal.contains('_') || literal.contains("::") {
        return false;
    }
    let words = without_holes(literal);
    let words = words
        .split_whitespace()
        .filter(|word| word.chars().filter(char::is_ascii_alphabetic).count() >= 3)
        .count();
    words >= 2
}

/// The literal with its `{}` / `{error}` / `{tps:.0}` holes replaced by spaces,
/// so a format skeleton does not read as prose.
fn without_holes(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    let mut depth = 0usize;
    for ch in literal.chars() {
        match ch {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// The files under `root`, sorted, with `*.rs` only.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// 1-based line number of a byte offset.
fn line_of(newlines: &[usize], offset: usize) -> usize {
    newlines.partition_point(|start| *start < offset) + 1
}

/// The `// i18n:` marker on this literal's line, if there is one: `Some(Some(reason))`
/// for a marker with a reason, `Some(None)` for one without.
fn marker_on_line(lexed: &Lexed, source: &str, literal: Range<usize>) -> Option<Option<String>> {
    let line = line_of(&lexed.newlines, literal.end);
    let comment = lexed.line_comments.iter().find(|comment| {
        comment.start >= literal.end && line_of(&lexed.newlines, comment.start) == line
    })?;
    let text = source[comment.start + 2..comment.end].trim();
    let reason = text.strip_prefix(MARKER)?.trim();
    Some(if reason.is_empty() {
        None
    } else {
        Some(reason.to_string())
    })
}

// ─── reading Rust without a parser ────────────────────────────────────────
//
// Enough of a lexer to tell a string literal from a comment and a doc comment
// from chrome: no macro expansion, no AST. The one structural question it has to
// answer is where a `#[cfg(test)]` item ends, and that is braces counted with
// strings and comments skipped.

struct Lexed {
    /// String literals, by the extent of their *content* (between the quotes).
    strings: Vec<Str>,
    /// `//` comments, for the in-source markers.
    line_comments: Vec<Comment>,
    /// One byte per source byte: true inside a comment or a string/char literal,
    /// delimiters included. Everything structural is read through it.
    mask: Vec<bool>,
    /// Byte offsets of `\n`, for line numbers.
    newlines: Vec<usize>,
}

struct Str {
    content: Range<usize>,
}

struct Comment {
    start: usize,
    end: usize,
}

fn lex(source: &str) -> Lexed {
    let bytes = source.as_bytes();
    let mut lexed = Lexed {
        strings: Vec::new(),
        line_comments: Vec::new(),
        mask: vec![false; bytes.len()],
        newlines: Vec::new(),
    };
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lexed.newlines.push(i);
                i += 1;
            }
            // A line comment runs to the newline, which stays code.
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                lexed.line_comments.push(Comment { start, end: i });
                lexed.mask[start..i].fill(true);
            }
            // Block comments nest in Rust.
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let start = i;
                let mut depth = 0usize;
                while i < bytes.len() {
                    if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                lexed.mask[start..i].fill(true);
            }
            b'"' => {
                let start = i;
                i += 1;
                let content_start = i;
                let mut content_end = bytes.len();
                while i < bytes.len() {
                    match bytes[i] {
                        // An escape hides its next byte, including a quote.
                        b'\\' => i = (i + 2).min(bytes.len()),
                        b'"' => {
                            content_end = i;
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                lexed.strings.push(Str {
                    content: content_start..content_end.max(content_start),
                });
                lexed.mask[start..i].fill(true);
            }
            _ if raw_open(bytes, i).is_some() => {
                let (quote, hashes) = raw_open(bytes, i).expect("just matched");
                let start = i;
                let content_start = quote + 1;
                let mut content_end = bytes.len();
                let mut end = bytes.len();
                let mut j = content_start;
                while j < bytes.len() {
                    if bytes[j] == b'"' && hashes_at(bytes, j + 1, hashes) {
                        content_end = j;
                        end = j + 1 + hashes;
                        break;
                    }
                    j += 1;
                }
                lexed.strings.push(Str {
                    content: content_start..content_end,
                });
                lexed.mask[start..end].fill(true);
                i = end;
            }
            b'\'' => {
                i = char_literal_end(bytes, i);
            }
            _ => i += 1,
        }
    }
    lexed
}

/// Where a raw string opens, if one does here: `(index of the opening quote,
/// number of `#`s)`.
fn raw_open(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    // `r"…"`, `r#"…"#`, and their byte cousins `br#"…"#`.
    let mut j = i;
    if bytes.get(j) == Some(&b'b') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'r') {
        return None;
    }
    // …but not the tail of an identifier (`bar`, `future_r`).
    if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        return None;
    }
    let mut k = j + 1;
    let mut hashes = 0;
    while bytes.get(k) == Some(&b'#') {
        hashes += 1;
        k += 1;
    }
    if bytes.get(k) == Some(&b'"') {
        Some((k, hashes))
    } else {
        None
    }
}

fn hashes_at(bytes: &[u8], at: usize, count: usize) -> bool {
    at + count <= bytes.len() && bytes[at..at + count].iter().all(|byte| *byte == b'#')
}

/// The byte after a `'` at `i`: a char literal's closing quote (a char literal
/// can hold a CJK character or a brace, so it must not be read as code), or just
/// past the `'` when it opens a lifetime.
fn char_literal_end(bytes: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    if bytes.get(j) == Some(&b'\\') {
        j += 1;
        if bytes.get(j) == Some(&b'u') && bytes.get(j + 1) == Some(&b'{') {
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }
            j += 1;
        } else if bytes.get(j) == Some(&b'x') {
            j += 3;
        } else {
            j += 1;
        }
    } else {
        // One character, which may be several bytes.
        j += 1;
        while j < bytes.len() && bytes[j] & 0xC0 == 0x80 {
            j += 1;
        }
    }
    if bytes.get(j) == Some(&b'\'') {
        j + 1
    } else {
        // A lifetime: the quote is punctuation.
        i + 1
    }
}

/// The byte ranges of `#[cfg(…test…)]` items: modules and functions that only
/// exist for tests, and therefore never draw a frame.
fn test_regions(source: &str, mask: &[bool]) -> Vec<Range<usize>> {
    let bytes = source.as_bytes();
    let mut regions = Vec::new();
    let mut from = 0;
    while let Some(offset) = source[from..].find("#[cfg(") {
        let at = from + offset;
        from = at + 1;
        if mask.get(at).copied().unwrap_or(true) {
            continue;
        }
        let open = at + "#[cfg".len();
        let Some(close) = matching(bytes, mask, open, b'(', b')') else {
            continue;
        };
        if !mentions_test(source, mask, open + 1..close) {
            continue;
        }
        // Past the attribute's own `]`.
        let mut after = close + 1;
        while after < bytes.len() && (mask[after] || bytes[after].is_ascii_whitespace()) {
            after += 1;
        }
        if bytes.get(after) != Some(&b']') {
            continue;
        }
        let Some(body) = gated_item_body(source, mask, after + 1) else {
            continue;
        };
        from = body.end;
        regions.push(at..body.end);
    }
    regions
}

/// `all(test, windows)` and `any(test, coverage)` gate tests too; a `"test-utils"`
/// *string* does not (string contents are masked out before tokenising).
fn mentions_test(source: &str, mask: &[bool], range: Range<usize>) -> bool {
    let bytes = source.as_bytes();
    let mut i = range.start;
    while i < range.end {
        if mask[i] || !is_ident_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < range.end && !mask[i] && is_ident_byte(bytes[i]) {
            i += 1;
        }
        if &source[start..i] == "test" {
            return true;
        }
    }
    false
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The body of the item an attribute belongs to — `Some` only for `mod` and
/// `fn`, because those are the only two that wrap code. An attribute on a field
/// (`#[cfg(test)] hooks: TestHooks,`) or a statement is not a region: a literal
/// there is still reported, and a false report is fixed by an allowlist row.
fn gated_item_body(source: &str, mask: &[bool], from: usize) -> Option<Range<usize>> {
    let bytes = source.as_bytes();
    let mut i = from;
    loop {
        while i < bytes.len() && (mask[i] || bytes[i].is_ascii_whitespace()) {
            i += 1;
        }
        // Further attributes between `#[cfg(test)]` and the item.
        if bytes.get(i) == Some(&b'#') && bytes.get(i + 1) == Some(&b'[') && !mask[i] {
            i = matching(bytes, mask, i + 1, b'[', b']')? + 1;
            continue;
        }
        break;
    }
    loop {
        while i < bytes.len() && (mask[i] || bytes[i].is_ascii_whitespace()) {
            i += 1;
        }
        let word = word_at(bytes, mask, i);
        match &source[word.clone()] {
            "pub" | "unsafe" | "async" | "const" | "default" => {
                i = word.end;
                // `pub(crate)`
                let mut k = i;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if bytes.get(k) == Some(&b'(') {
                    i = matching(bytes, mask, k, b'(', b')')? + 1;
                }
            }
            // `extern "C" fn`: the string between is masked, so skip it whole.
            "extern" => {
                i = word.end;
                while i < bytes.len() && (mask[i] || bytes[i].is_ascii_whitespace()) {
                    i += 1;
                }
            }
            "mod" | "fn" => {
                let mut k = word.end;
                while k < bytes.len() {
                    if mask[k] {
                        k += 1;
                        continue;
                    }
                    if bytes[k] == b'{' {
                        let close = matching(bytes, mask, k, b'{', b'}')?;
                        return Some(k..close + 1);
                    }
                    // `mod tests;` has no body here.
                    if bytes[k] == b';' {
                        return None;
                    }
                    k += 1;
                }
                return None;
            }
            _ => return None,
        }
    }
}

fn word_at(bytes: &[u8], mask: &[bool], at: usize) -> Range<usize> {
    let mut i = at;
    while i < bytes.len() && !mask[i] && is_ident_byte(bytes[i]) {
        i += 1;
    }
    at..i
}

/// The index of the `close` that matches the `open` at `at`.
fn matching(bytes: &[u8], mask: &[bool], at: usize, open: u8, close: u8) -> Option<usize> {
    if bytes.get(at) != Some(&open) {
        return None;
    }
    let mut depth = 0usize;
    let mut i = at;
    while i < bytes.len() {
        if mask[i] {
            i += 1;
            continue;
        }
        if bytes[i] == open {
            depth += 1;
        } else if bytes[i] == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// The summary line, plus every candidate with what happened to it. `cargo test`
/// hides it unless the run asks for output (`-- --nocapture`) or the test fails.
fn print_summary(scan: &Scan, root: &Path) {
    let known_gaps = ALLOW
        .iter()
        .filter(|row| matches!(row.why, Why::KnownGap(_)))
        .count();
    println!(
        "i18n guard ({}): {} files, {} literals outside tests, {} chrome candidates, \
         {} exemptions → {} allowlist rows ({} known gaps, {} not chrome), {} files, \
         {} markers",
        root.display(),
        scan.files,
        scan.literals,
        scan.candidates.len(),
        scan.exemptions(),
        scan.rows_used,
        known_gaps,
        ALLOW.len() - known_gaps,
        scan.files_used,
        scan.markers_used,
    );
    for (candidate, exempt) in scan.candidates.iter().zip(&scan.exempt) {
        let status = match exempt {
            Some(why) => format!("exempt ({why})"),
            None => "VIOLATION".to_string(),
        };
        println!(
            "  {}:{}: {:?} — {status}",
            candidate.path, candidate.line, candidate.literal
        );
    }
}

/// The scanner's own limits, pinned. Without these a change to a rule could
/// turn the check into one that never fires and nothing would notice — the
/// whole tree passing is exactly what a broken scanner also looks like.
mod the_scanner_itself {
    use super::*;

    #[test]
    fn a_sentence_reads_as_a_sentence() {
        for sentence in [
            "No models to choose from yet",
            "Could not export: {error}",
            "no workspace yet",
            "One Dark",
            "Could not load older rows: {error}",
        ] {
            assert!(prose_like(sentence), "{sentence:?} is drawn prose");
        }
    }

    #[test]
    fn a_path_an_id_a_skeleton_and_an_escape_are_not_sentences() {
        for quiet in [
            "\\x1b[?25l",                    // an escape sequence
            "{tps:.0} tok/s",                // a skeleton with a unit
            "{border}{body}{border}",        // nothing but holes
            "launchpad.session.{index}",     // a hit region's id
            "cache_read_tokens",             // a wire key
            "~/future-os/.worktrees/mobile", // a path
            "Queued",                        // one word
            "Manual",                        // one word
            "{point} {app} {version}",       // three holes, no words
        ] {
            assert!(!prose_like(quiet), "{quiet:?} is not drawn prose");
        }
    }

    #[test]
    fn cjk_is_recognised_and_the_typographic_dots_are_not() {
        for ch in ['待', '办', 'あ', '한', '、', '，', '（'] {
            assert!(is_cjk(ch), "{ch:?} is CJK");
        }
        // These are in the table's English column and must never count as CJK,
        // or the guard would report the table's own text.
        for ch in ['·', '→', '…', '⏎', '↑', 'a', ' '] {
            assert!(!is_cjk(ch), "{ch:?} is not CJK");
        }
    }

    #[test]
    fn a_string_that_looks_like_code_is_still_a_string() {
        let source = r####"
fn draw() {
    let a = "not // a comment";
    let b = r#"raw "quoted" text"#;
    let c = '"';
    // a comment with "quotes", 'quotes' and {braces}
    let d = "待办";
}
"####;
        let literals: Vec<&str> = lex(source)
            .strings
            .iter()
            .map(|literal| &source[literal.content.clone()])
            .collect();
        // The `'"'` char literal contributes nothing — and, the point of the
        // case, does not start a string that would swallow `let d` below it.
        assert_eq!(
            literals,
            vec!["not // a comment", "raw \"quoted\" text", "待办"]
        );
    }

    #[test]
    fn a_cfg_test_module_is_skipped_and_a_cfg_test_field_is_not() {
        let source = r#"
struct State {
    #[cfg(test)] hooks: u8,
}

#[cfg(all(test, windows))]
fn helper() {
    let inside = "test only";
}

fn draw() {
    let chrome = "chrome";
}
"#;
        let lexed = lex(source);
        let regions = test_regions(source, &lexed.mask);
        assert_eq!(regions.len(), 1, "one test item: {regions:?}");
        let skipped: Vec<&str> = lexed
            .strings
            .iter()
            .filter(|literal| {
                regions
                    .iter()
                    .any(|region| region.contains(&literal.content.start))
            })
            .map(|literal| &source[literal.content.clone()])
            .collect();
        // The `#[cfg(test)]` field's literal is still checked: a field attribute
        // does not wrap code, so nothing is skipped for it.
        assert_eq!(skipped, vec!["test only"]);
    }

    #[test]
    fn a_marker_exempts_only_its_own_line() {
        let source = r#"
fn draw() {
    let a = "chrome"; // i18n: an invariant's panic, never drawn
    let b = "chrome";
}
"#;
        let lexed = lex(source);
        let marked: Vec<bool> = lexed
            .strings
            .iter()
            .map(|literal| marker_on_line(&lexed, source, literal.content.clone()).is_some())
            .collect();
        assert_eq!(marked, vec![true, false]);
    }

    #[test]
    fn a_marker_without_a_reason_is_reported() {
        let source = r#"
fn draw() {
    let a = "chrome"; // i18n:
}
"#;
        let lexed = lex(source);
        let marker = marker_on_line(&lexed, source, lexed.strings[0].content.clone());
        assert_eq!(marker, Some(None));
    }
}
