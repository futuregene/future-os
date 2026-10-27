//! Bounded title extraction over paged UTF-8. Uses the shared title grammar
//! (links/images contribute labels; conversation references are addressing),
//! without allocating all source characters or the complete rendered text.
use anyhow::{ensure, Context, Result};

const PAGE_BYTES: usize = 4096;
const MAX_COLUMNS: usize = 40;
// Zero-width scalars must not turn a 40-column preview into an unlimited value.
const MAX_PREVIEW_BYTES: usize = 1024;
const SESSION_REFERENCE: &str = "futureos://session/";

#[derive(Default)]
struct Output {
    text: String,
    columns: usize,
    space: bool,
    closed: bool,
}
impl Output {
    fn character(&mut self, ch: char) {
        if self.closed {
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
        if self.columns + columns > MAX_COLUMNS
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
    let mut primary = Output::default();
    let mut fallback = Output::default();
    let mut at = 0usize;
    while let Some(ch) = source.character(at)? {
        let bracket = if ch == '!' && source.character(at + 1)? == Some('[') {
            Some(at + 1)
        } else {
            (ch == '[').then_some(at)
        };
        if let Some(bracket) = bracket {
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
    Ok(if primary.text.is_empty() {
        fallback.text
    } else {
        primary.text
    })
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
