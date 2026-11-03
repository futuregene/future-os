//! Bounded title extraction over paged UTF-8. Uses the shared title grammar
//! (links/images contribute labels; conversation references and composer
//! placeholders are addressing; markdown syntax is flattened away by
//! [`future_rpc::title::flatten_markup`]), without allocating all source
//! characters or the complete rendered text.
use anyhow::{ensure, Context, Result};

const PAGE_BYTES: usize = 4096;
/// Columns of title kept after the syntax is flattened away.
const MAX_COLUMNS: usize = 40;
/// The scan keeps more than the title's final width: emphasis marks, bullets
/// and code fences are dropped after the scan, and text that was cut off here
/// cannot be recovered by flattening it.
const SCAN_COLUMNS: usize = MAX_COLUMNS * 4;
// Zero-width scalars must not turn a 40-column preview into an unlimited value.
const MAX_PREVIEW_BYTES: usize = 1024;
const SESSION_REFERENCE: &str = "futureos://session/";
/// A composer placeholder (`[Image #1]`, `[Pasted Content 2000 chars]`) is
/// shorter than this, and the probe stops at the first character that could not
/// belong to one.
const PLACEHOLDER_PROBE_CHARS: usize = 48;

struct Output {
    text: String,
    columns: usize,
    limit: usize,
    space: bool,
    closed: bool,
}
impl Output {
    fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            columns: 0,
            limit,
            space: false,
            closed: false,
        }
    }
    fn character(&mut self, ch: char) {
        if self.closed {
            return;
        }
        // A line break stays a line break: flattening a title needs to know
        // where a line started to tell a heading or a bullet from prose, and
        // the marks themselves are only dropped after the scan.
        if ch == '\n' {
            self.space = false;
            if !self.text.is_empty() && !self.text.ends_with('\n') {
                self.text.push('\n');
            }
            return;
        }
        if ch.is_whitespace() {
            self.space = !self.text.is_empty();
            return;
        }
        if self.space {
            self.space = false;
            self.push(' ');
        }
        self.push(ch);
    }
    fn push(&mut self, ch: char) {
        if self.closed {
            return;
        }
        let columns = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if self.columns + columns > self.limit
            || self.text.len() + ch.len_utf8() > MAX_PREVIEW_BYTES
        {
            self.closed = true;
            return;
        }
        self.columns += columns;
        self.text.push(ch);
    }
}

#[derive(Clone, Copy)]
struct Target {
    end: usize,
    reference: bool,
}
#[derive(Clone, Copy)]
struct LabelScan {
    from: usize,
    end: Option<usize>,
    target: Option<Option<Target>>,
}
struct Text<F> {
    load: F,
    len: usize,
    start: usize,
    page: String,
    label: Option<LabelScan>,
}
impl<F: FnMut(usize, usize) -> Result<String>> Text<F> {
    fn character(&mut self, at: usize) -> Result<Option<char>> {
        if at >= self.len {
            return Ok(None);
        }
        if self.page.is_empty() || at < self.start || at >= self.start + self.page.len() {
            self.start = at;
            self.page = (self.load)(at, PAGE_BYTES.min(self.len - at))?;
            ensure!(
                !self.page.is_empty()
                    && self.page.len() <= PAGE_BYTES
                    && at + self.page.len() <= self.len,
                "invalid title source page"
            );
        }
        Ok(Some(
            self.page
                .get(at - self.start..)
                .context("title offset is not UTF-8")?
                .chars()
                .next()
                .context("empty title character")?,
        ))
    }
    /// Byte length of a composer placeholder (`[Image #1]`,
    /// `[Pasted Content 2000 chars]`) starting at `from`, read through the page
    /// and recognised by the shared rule rather than by a second copy of it.
    fn placeholder(&mut self, from: usize) -> Result<Option<usize>> {
        let mut probe = String::new();
        let mut at = from;
        let mut characters = 0;
        while characters < PLACEHOLDER_PROBE_CHARS {
            let Some(ch) = self.character(at)? else { break };
            probe.push(ch);
            at += ch.len_utf8();
            characters += 1;
        }
        Ok(future_rpc::title::placeholder_len(&probe))
    }
    fn label_end(&mut self, from: usize) -> Result<Option<usize>> {
        if let Some(scan) = self.label {
            if from >= scan.from && scan.end.is_none_or(|end| from <= end) {
                return Ok(scan.end);
            }
        }
        let mut at = from;
        let mut end = None;
        while let Some(ch) = self.character(at)? {
            if ch == ']' {
                end = Some(at);
                break;
            }
            at += ch.len_utf8();
        }
        self.label = Some(LabelScan {
            from,
            end,
            target: None,
        });
        Ok(end)
    }
    fn target(&mut self, label_end: usize) -> Result<Option<Target>> {
        if let Some(target) = self.label.and_then(|scan| scan.target) {
            return Ok(target);
        }
        let target = self.parse_target(label_end)?;
        self.label
            .as_mut()
            .context("missing title label scan")?
            .target = Some(target);
        Ok(target)
    }
    fn parse_target(&mut self, label_end: usize) -> Result<Option<Target>> {
        if self.character(label_end + 1)? != Some('(') {
            return Ok(None);
        }
        let mut at = label_end + 2;
        let mut depth = 1usize;
        let mut prefix = String::new();
        let mut seen = false;
        let mut last = None;
        while let Some(ch) = self.character(at)? {
            match ch {
                '(' => {
                    depth = depth
                        .checked_add(1)
                        .context("title target nesting overflow")?
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let reference = prefix.starts_with(SESSION_REFERENCE)
                            || (prefix.starts_with(&format!("<{SESSION_REFERENCE}"))
                                && last == Some('>'));
                        return Ok(Some(Target {
                            end: at + 1,
                            reference,
                        }));
                    }
                }
                _ => {}
            }
            if seen || !ch.is_whitespace() {
                seen = true;
                if prefix.len() < SESSION_REFERENCE.len() + 2 {
                    prefix.push(ch);
                }
            }
            if !ch.is_whitespace() {
                last = Some(ch);
            }
            at += ch.len_utf8();
        }
        Ok(None)
    }
}

/// `load` returns at most its byte budget, starts on the requested UTF-8
/// boundary and may stop a few bytes early to preserve the trailing character.
/// Source identity/ownership checks belong to the caller on every page.
pub(crate) fn paged(
    len: usize,
    load: impl FnMut(usize, usize) -> Result<String>,
) -> Result<String> {
    let mut source = Text {
        load,
        len,
        start: 0,
        page: String::new(),
        label: None,
    };
    let mut primary = Output::new(SCAN_COLUMNS);
    let mut fallback = Output::new(SCAN_COLUMNS);
    let mut at = 0usize;
    while let Some(ch) = source.character(at)? {
        let bracket = if ch == '!' && source.character(at + 1)? == Some('[') {
            Some(at + 1)
        } else {
            (ch == '[').then_some(at)
        };
        if let Some(bracket) = bracket {
            // A composer placeholder names nothing a reader can use, so it is
            // skipped before it can spend any of the width budget.
            if let Some(length) = source.placeholder(bracket)? {
                at = bracket + length;
                continue;
            }
            if let Some(end) = source.label_end(bracket + 1)? {
                if let Some(target) = source.target(end)? {
                    let mut label = bracket + 1;
                    while label < end
                        && ((!target.reference && !primary.closed) || !fallback.closed)
                    {
                        let ch = source
                            .character(label)?
                            .context("title label ended early")?;
                        if !target.reference {
                            primary.character(ch);
                        }
                        fallback.character(ch);
                        label += ch.len_utf8();
                    }
                    at = target.end;
                    if primary.closed {
                        break;
                    }
                    continue;
                }
            }
        }
        primary.character(ch);
        fallback.character(ch);
        if primary.closed {
            break;
        }
        at += ch.len_utf8();
    }
    let scanned = if primary.text.is_empty() {
        fallback.text
    } else {
        primary.text
    };
    // Markdown marks and any placeholder the scan did not reach are dropped
    // here, before the width the title is shown in is applied: a cut taken
    // first would leave half a `**` or half a placeholder behind.
    Ok(crate::session::projection::truncate_visible(
        &future_rpc::title::flatten_markup(&scanned),
        MAX_COLUMNS,
    ))
}

pub(crate) fn from_str(text: &str) -> String {
    paged(text.len(), |start, budget| {
        let mut end = (start + budget).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Ok(text[start..end].to_owned())
    })
    .expect("a valid string supplies valid title pages")
}

#[cfg(test)]
#[path = "title_preview_tests.rs"]
mod tests;
