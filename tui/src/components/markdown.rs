//! Markdown renderer — 1:1 port of `tui/src/components/markdown.ts`.
//!
//! The TS renderer consumes a `marked` token tree (block tokens with inline
//! `tokens` children). We reproduce the same tree shape from a
//! `pulldown-cmark` event stream, then apply the identical rendering logic:
//! stylePrefix re-application, blank-line rules, the table column-width
//! algorithm, quote borders, and code fences.
//!
//! The adapter compensates for two parser differences:
//!   - `marked` emits explicit `space` tokens between blocks separated by
//!     blank line(s); pulldown-cmark does not. We insert them from line
//!     spans (see `insert_spaces`). pulldown-cmark also folds trailing blank
//!     lines after a list into the list's range (marked assigns them to the
//!     parent level), so list spans are trimmed of trailing blank lines.
//!   - `marked` emits `def` tokens (link reference definitions, which render
//!     nothing but still separate blocks); pulldown-cmark consumes them
//!     silently. We re-insert `Def` blocks from a source line scan.
//!
//! Known accepted divergence (documented in the parity README): link
//! reference definitions nested inside blockquotes/list items keep their
//! surrounding blank-line spacing only at the top level.

use std::rc::Rc;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;

use crate::terminal_image::{hyperlink, is_image_line};
use crate::theme::{
    bold as theme_bold, dim as theme_dim, fg as theme_fg, italic as theme_italic,
    underline as theme_underline,
};
use crate::tui::{Component, RESET};
use crate::utils::{
    apply_background_to_line, strip_ansi_codes, visible_width, wrap_text_with_ansi,
};

// ─── Token tree (marked-shaped) ────────────────────────────────────────────

/// Block-level tokens, mirroring the `marked` block token types the TS
/// renderer handles.
#[derive(Debug, Clone, PartialEq)]
enum MdBlock {
    Heading {
        depth: usize,
        inline: Vec<MdInline>,
    },
    Paragraph {
        inline: Vec<MdInline>,
    },
    Code {
        text: String,
        lang: String,
    },
    Blockquote {
        blocks: Vec<MdBlock>,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<Vec<MdBlock>>,
    },
    Hr,
    Html {
        raw: String,
    },
    Space,
    Table {
        header: Vec<Vec<MdInline>>,
        rows: Vec<Vec<Vec<MdInline>>>,
        raw: String,
    },
    /// Block-level text token (tight list items; marked's `text` token with
    /// inline `tokens`).
    Text {
        inline: Vec<MdInline>,
    },
    /// Link reference definition (`[id]: url`) — renders nothing.
    Def,
}

impl MdBlock {
    /// `token.type` — used for the `nextType` checks in renderToken.
    fn type_name(&self) -> &'static str {
        match self {
            MdBlock::Heading { .. } => "heading",
            MdBlock::Paragraph { .. } => "paragraph",
            MdBlock::Code { .. } => "code",
            MdBlock::Blockquote { .. } => "blockquote",
            MdBlock::List { .. } => "list",
            MdBlock::Hr => "hr",
            MdBlock::Html { .. } => "html",
            MdBlock::Space => "space",
            MdBlock::Table { .. } => "table",
            MdBlock::Text { .. } => "text",
            MdBlock::Def => "def",
        }
    }
}

/// Inline tokens, mirroring `marked`'s inline token types.
#[derive(Debug, Clone, PartialEq)]
enum MdInline {
    Text {
        text: String,
    },
    Strong {
        inline: Vec<MdInline>,
    },
    Em {
        inline: Vec<MdInline>,
    },
    Codespan {
        text: String,
    },
    Del {
        inline: Vec<MdInline>,
    },
    Link {
        href: String,
        inline: Vec<MdInline>,
    },
    Br,
    Html {
        raw: String,
    },
    /// Renders nothing (`case "image": break`).
    Image,
    /// Task-list checkbox — renders nothing in the TUI renderer.
    Checkbox,
}

// ─── Theme ────────────────────────────────────────────────────────────────

/// Style function type used across the theme (Rc'd so themes can be cloned).
pub type StyleFn = Rc<dyn Fn(&str) -> String>;
pub type HighlightFn = Rc<dyn Fn(&str, Option<&str>) -> Vec<String>>;

/// Style functions used by the renderer. `Rc<dyn Fn>` mirrors the TS
/// closures; the defaults are built from the theme helpers.
#[derive(Clone)]
pub struct MarkdownTheme {
    pub heading: StyleFn,
    pub link: StyleFn,
    pub link_url: StyleFn,
    pub code: StyleFn,
    pub code_block: StyleFn,
    pub code_block_border: StyleFn,
    pub quote: StyleFn,
    pub quote_border: StyleFn,
    pub hr: StyleFn,
    pub list_bullet: StyleFn,
    pub bold: StyleFn,
    pub italic: StyleFn,
    pub strikethrough: StyleFn,
    pub underline: StyleFn,
    pub highlight_code: Option<HighlightFn>,
    pub code_block_indent: Option<String>,
}

impl std::fmt::Debug for MarkdownTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MarkdownTheme {{ .. }}")
    }
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        MarkdownTheme {
            heading: Rc::new(|t| theme_fg(221, &theme_bold(t))),
            link: Rc::new(|t| theme_fg(117, t)),
            link_url: Rc::new(|t| theme_fg(245, t)),
            code: Rc::new(|t| theme_fg(151, t)),
            code_block: Rc::new(|t| theme_fg(143, &theme_dim(t))),
            code_block_border: Rc::new(|t| theme_fg(244, &theme_dim(t))),
            quote: Rc::new(|t| theme_fg(244, &theme_italic(t))),
            quote_border: Rc::new(|t| theme_fg(244, t)),
            hr: Rc::new(|t| theme_fg(244, t)),
            list_bullet: Rc::new(|t| theme_fg(151, t)),
            bold: Rc::new(theme_bold),
            italic: Rc::new(theme_italic),
            strikethrough: Rc::new(|t| theme_fg(244, t)),
            underline: Rc::new(theme_underline),
            highlight_code: None,
            code_block_indent: None,
        }
    }
}

/// Partial theme overrides — mirrors the TS `Partial<MarkdownTheme>`
/// constructor parameter (`{...defaults, ...theme}`).
#[derive(Default)]
pub struct MarkdownThemePartial {
    pub heading: Option<StyleFn>,
    pub link: Option<StyleFn>,
    pub link_url: Option<StyleFn>,
    pub code: Option<StyleFn>,
    pub code_block: Option<StyleFn>,
    pub code_block_border: Option<StyleFn>,
    pub quote: Option<StyleFn>,
    pub quote_border: Option<StyleFn>,
    pub hr: Option<StyleFn>,
    pub list_bullet: Option<StyleFn>,
    pub bold: Option<StyleFn>,
    pub italic: Option<StyleFn>,
    pub strikethrough: Option<StyleFn>,
    pub underline: Option<StyleFn>,
    pub highlight_code: Option<HighlightFn>,
    pub code_block_indent: Option<String>,
}

impl MarkdownTheme {
    /// `{...defaults, ...theme}` — apply partial overrides.
    fn with_partial(mut self, partial: MarkdownThemePartial) -> Self {
        if let Some(v) = partial.heading {
            self.heading = v;
        }
        if let Some(v) = partial.link {
            self.link = v;
        }
        if let Some(v) = partial.link_url {
            self.link_url = v;
        }
        if let Some(v) = partial.code {
            self.code = v;
        }
        if let Some(v) = partial.code_block {
            self.code_block = v;
        }
        if let Some(v) = partial.code_block_border {
            self.code_block_border = v;
        }
        if let Some(v) = partial.quote {
            self.quote = v;
        }
        if let Some(v) = partial.quote_border {
            self.quote_border = v;
        }
        if let Some(v) = partial.hr {
            self.hr = v;
        }
        if let Some(v) = partial.list_bullet {
            self.list_bullet = v;
        }
        if let Some(v) = partial.bold {
            self.bold = v;
        }
        if let Some(v) = partial.italic {
            self.italic = v;
        }
        if let Some(v) = partial.strikethrough {
            self.strikethrough = v;
        }
        if let Some(v) = partial.underline {
            self.underline = v;
        }
        if let Some(v) = partial.highlight_code {
            self.highlight_code = Some(v);
        }
        if let Some(v) = partial.code_block_indent {
            self.code_block_indent = Some(v);
        }
        self
    }
}

/// Mirrors the TS `DefaultTextStyle` (unused by ChatArea, but part of the
/// renderer API — the parity harness exercises it).
#[derive(Clone, Default)]
pub struct DefaultTextStyle {
    pub color: Option<StyleFn>,
    pub bg_color: Option<StyleFn>,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
}

/// Style application context for inline rendering — owns its data (no
/// borrows of `self`, so `render_inline_tokens(&mut self, ..)` can run
/// while the context is alive).
struct InlineStyleContext {
    apply_text: StyleFn,
    style_prefix: String,
}

// ─── Parser (pulldown-cmark → marked-shaped tree) ─────────────────────────

/// A block's line span `(first_line, last_line)` — inclusive line numbers
/// of the source lines the block occupies.
type LineSpan = (usize, usize);

struct MdParser<'a> {
    src: &'a str,
    line_starts: Vec<usize>,
    events: Vec<(Event<'a>, std::ops::Range<usize>)>,
    pos: usize,
}

impl<'a> MdParser<'a> {
    fn new(src: &'a str) -> Self {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        let events: Vec<(Event<'a>, std::ops::Range<usize>)> =
            Parser::new_ext(src, opts).into_offset_iter().collect();
        let mut line_starts = vec![0usize];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        MdParser {
            src,
            line_starts,
            events,
            pos: 0,
        }
    }

    fn line_of(&self, byte: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= byte) - 1
    }

    /// Line `line`'s content (without the trailing newline).
    fn line_content(&self, line: usize) -> &'a str {
        let start = self.line_starts[line];
        let end = if line + 1 < self.line_starts.len() {
            self.line_starts[line + 1].saturating_sub(1)
        } else {
            self.src.len()
        };
        &self.src[start..end]
    }

    /// Blank-line test: empty/whitespace after stripping `>` blockquote
    /// markers (`>`, `> `, `  > `, nested `>>`). Real blank lines never
    /// start with `>`, and blockquote marker lines do, so stripping is safe
    /// in both contexts.
    fn is_blank_line(&self, line: usize) -> bool {
        let content = self.line_content(line);
        let mut t = content;
        loop {
            let mut rest = t;
            let mut spaces = 0;
            while spaces < 3 && rest.starts_with(' ') {
                rest = &rest[1..];
                spaces += 1;
            }
            if rest.starts_with('>') {
                rest = &rest[1..];
                if rest.starts_with(' ') {
                    rest = &rest[1..];
                }
                t = rest;
            } else {
                break;
            }
        }
        t.trim().is_empty()
    }

    /// Parse a block list until `stop` (the container's End event).
    /// `container` = `(first_line, last_content_line)` used for space
    /// insertion; pass `(0, 0)` with `insert_spaces = false` when the
    /// container span is not yet known (callers then run `insert_spaces`
    /// themselves). `insert_defs` only runs for the top level.
    fn parse_blocks(
        &mut self,
        stop: Option<TagEnd>,
        container: LineSpan,
        insert_spaces: bool,
        insert_defs: bool,
    ) -> (Vec<MdBlock>, Vec<LineSpan>) {
        let mut blocks: Vec<MdBlock> = Vec::new();
        let mut spans: Vec<LineSpan> = Vec::new();

        while self.pos < self.events.len() {
            let (ev, range) = &self.events[self.pos];
            match ev {
                Event::End(tag) => {
                    // Break on the container's End (or any End at the top
                    // level, where none should remain), consuming it.
                    if stop.as_ref().is_none_or(|s| tag == s) {
                        self.pos += 1;
                        break;
                    }
                    self.pos += 1;
                }
                Event::Start(_) => {
                    if let Some((block, span)) = self.parse_block_start() {
                        blocks.push(block);
                        spans.push(span);
                    }
                }
                Event::Text(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::Code(_)
                | Event::InlineHtml(_)
                | Event::TaskListMarker(_) => {
                    // Bare inline events (tight list items, no paragraph
                    // tags) — collect into a block Text token.
                    let (b, s) = self.parse_bare_text_block(range.clone());
                    blocks.push(b);
                    spans.push(s);
                }
                Event::Rule => {
                    let span = (self.line_of(range.start), self.line_of(range.end - 1));
                    blocks.push(MdBlock::Hr);
                    spans.push(span);
                    self.pos += 1;
                }
                Event::Html(h) => {
                    let raw = h.to_string();
                    let span = (self.line_of(range.start), self.line_of(range.end - 1));
                    blocks.push(MdBlock::Html { raw });
                    spans.push(span);
                    self.pos += 1;
                }
                other => {
                    let _ = other;
                    self.pos += 1;
                }
            }
        }

        if insert_defs {
            self.insert_defs(&mut blocks, &mut spans, container);
        }
        if insert_spaces {
            self.insert_spaces(&mut blocks, &mut spans, container);
        }
        (blocks, spans)
    }

    /// Parse one `Start(tag)` block at `self.pos` (consuming through its
    /// matching End event).
    fn parse_block_start(&mut self) -> Option<(MdBlock, LineSpan)> {
        let (ev, range) = &self.events[self.pos];
        let start_byte = range.start;
        let tag = match ev {
            Event::Start(t) => t.clone(),
            _ => return None,
        };
        match tag {
            Tag::HtmlBlock => {
                let end_byte = self.find_container_end(TagEnd::HtmlBlock);
                let raw = self.src[start_byte..end_byte].to_string();
                self.pos += 1;
                while self.pos < self.events.len() {
                    if matches!(self.events[self.pos].0, Event::End(TagEnd::HtmlBlock)) {
                        self.pos += 1;
                        break;
                    }
                    self.pos += 1;
                }
                let span = (self.line_of(start_byte), self.line_of(end_byte - 1));
                Some((MdBlock::Html { raw }, span))
            }
            Tag::Paragraph => {
                let inline = self.parse_inline_until(TagEnd::Paragraph);
                let end_byte = self.events[self.pos - 1].1.end;
                // pulldown trims trailing whitespace from its Text events;
                // marked keeps it (a trailing `text` token). Restore it from
                // the paragraph's source slice.
                let mut inline = inline;
                let para_src = self.src[start_byte..end_byte].trim_end_matches(['\n', '\r']);
                let ws_len = para_src.len() - para_src.trim_end_matches([' ', '\t']).len();
                if ws_len > 0 {
                    self.push_text(&mut inline, para_src[para_src.len() - ws_len..].to_string());
                }
                let span = (self.line_of(start_byte), self.line_of(end_byte - 1));
                Some((MdBlock::Paragraph { inline }, span))
            }
            Tag::Heading { level, .. } => {
                let depth = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    pulldown_cmark::HeadingLevel::H3 => 3,
                    pulldown_cmark::HeadingLevel::H4 => 4,
                    pulldown_cmark::HeadingLevel::H5 => 5,
                    pulldown_cmark::HeadingLevel::H6 => 6,
                };
                let inline = self.parse_inline_until(TagEnd::Heading(level));
                let end_byte = self.events[self.pos - 1].1.end;
                let span = (self.line_of(start_byte), self.line_of(end_byte - 1));
                Some((MdBlock::Heading { depth, inline }, span))
            }
            Tag::CodeBlock(kind) => {
                let (lang, fenced) = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => (lang.to_string(), true),
                    pulldown_cmark::CodeBlockKind::Indented => (String::new(), false),
                };
                self.pos += 1;
                let mut text = String::new();
                let end_byte = loop {
                    match &self.events[self.pos].0 {
                        Event::End(TagEnd::CodeBlock) => {
                            let e = self.events[self.pos].1.end;
                            self.pos += 1;
                            break e;
                        }
                        Event::Text(t) => {
                            text.push_str(t);
                            self.pos += 1;
                        }
                        _ => {
                            self.pos += 1;
                        }
                    }
                };
                // marked: fenced code drops ONE trailing newline (the one
                // before the closing fence); indented code drops ALL
                // trailing blank lines (marked's `Y` helper).
                if fenced {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                } else {
                    while text.ends_with('\n') {
                        text.pop();
                    }
                    while text.ends_with(' ') || text.ends_with('\t') {
                        text.pop();
                    }
                }
                let span = (self.line_of(start_byte), self.line_of(end_byte - 1));
                Some((MdBlock::Code { text, lang }, span))
            }
            Tag::BlockQuote(_) => {
                let end_byte = self.find_container_end(TagEnd::BlockQuote(None));
                let container = (
                    self.line_of(start_byte),
                    self.line_of(end_byte.saturating_sub(1)),
                );
                self.pos += 1;
                let (mut blocks, mut spans) =
                    self.parse_blocks(Some(TagEnd::BlockQuote(None)), (0, 0), false, false);
                self.insert_spaces(&mut blocks, &mut spans, container);
                let span = (self.line_of(start_byte), self.line_of(end_byte - 1));
                Some((MdBlock::Blockquote { blocks }, span))
            }
            Tag::List(start) => {
                let ordered = start.is_some();
                let start_num = start;
                let end_byte = self.find_container_end(TagEnd::List(start.is_some()));
                self.pos += 1;
                let mut items: Vec<Vec<MdBlock>> = Vec::new();
                let mut item_spans: Vec<LineSpan> = Vec::new();
                while self.pos < self.events.len() {
                    match &self.events[self.pos].0 {
                        Event::Start(Tag::Item) => {
                            let is = self.events[self.pos].1.start;
                            let item_end = self.find_container_end(TagEnd::Item);
                            let item_container =
                                (self.line_of(is), self.line_of(item_end.saturating_sub(1)));
                            self.pos += 1;
                            let (mut item_blocks, mut item_block_spans) =
                                self.parse_blocks(Some(TagEnd::Item), (0, 0), false, false);
                            self.insert_spaces(
                                &mut item_blocks,
                                &mut item_block_spans,
                                item_container,
                            );
                            let item_span = (self.line_of(is), self.line_of(item_end - 1));
                            items.push(item_blocks);
                            item_spans.push(item_span);
                        }
                        Event::End(TagEnd::List(_)) => {
                            self.pos += 1;
                            break;
                        }
                        _ => {
                            self.pos += 1;
                        }
                    }
                }
                // pulldown folds trailing blank lines after the last item
                // into the list range; marked assigns them to the parent
                // level. Trim them so the parent sees the blank gap.
                let mut span = (self.line_of(start_byte), self.line_of(end_byte - 1));
                while span.1 > span.0 && self.is_blank_line(span.1) {
                    span.1 -= 1;
                }
                Some((
                    MdBlock::List {
                        ordered,
                        start: start_num,
                        items,
                    },
                    span,
                ))
            }
            Tag::Table(_) => {
                self.pos += 1;
                let mut header: Vec<Vec<MdInline>> = Vec::new();
                let mut rows: Vec<Vec<Vec<MdInline>>> = Vec::new();
                let end_byte = loop {
                    match &self.events[self.pos].0 {
                        Event::Start(Tag::TableHead) => {
                            self.pos += 1;
                            loop {
                                match &self.events[self.pos].0 {
                                    Event::Start(Tag::TableCell) => {
                                        let cell = self.parse_inline_until(TagEnd::TableCell);
                                        header.push(cell);
                                    }
                                    Event::End(TagEnd::TableHead) => {
                                        self.pos += 1;
                                        break;
                                    }
                                    _ => {
                                        self.pos += 1;
                                    }
                                }
                            }
                        }
                        Event::Start(Tag::TableRow) => {
                            self.pos += 1;
                            let mut row: Vec<Vec<MdInline>> = Vec::new();
                            loop {
                                match &self.events[self.pos].0 {
                                    Event::Start(Tag::TableCell) => {
                                        let cell = self.parse_inline_until(TagEnd::TableCell);
                                        row.push(cell);
                                    }
                                    Event::End(TagEnd::TableRow) => {
                                        self.pos += 1;
                                        break;
                                    }
                                    _ => {
                                        self.pos += 1;
                                    }
                                }
                            }
                            rows.push(row);
                        }
                        Event::End(TagEnd::Table) => {
                            let e = self.events[self.pos].1.end;
                            self.pos += 1;
                            break e;
                        }
                        _ => {
                            self.pos += 1;
                        }
                    }
                };
                let raw = self.src[start_byte..end_byte].to_string();
                let span = (self.line_of(start_byte), self.line_of(end_byte - 1));
                Some((MdBlock::Table { header, rows, raw }, span))
            }
            _ => {
                // Unexpected block tag — skip through its matching End.
                self.pos += 1;
                while self.pos < self.events.len() {
                    if matches!(self.events[self.pos].0, Event::End(_)) {
                        self.pos += 1;
                        break;
                    }
                    self.pos += 1;
                }
                None
            }
        }
    }

    /// Scan forward from `self.pos` to find the byte range end of the
    /// matching `stop` End event (handles nesting). `self.pos` must point at
    /// the Start event; it is left unchanged.
    fn find_container_end(&self, stop: TagEnd) -> usize {
        let mut depth = 0usize;
        let mut pos = self.pos;
        let start_tag = match &self.events[pos].0 {
            Event::Start(t) => t.clone(),
            _ => return 0,
        };
        while pos < self.events.len() {
            match &self.events[pos].0 {
                Event::Start(t) => {
                    if *t == start_tag {
                        depth += 1;
                    }
                }
                Event::End(t) => {
                    if *t == stop {
                        if depth <= 1 {
                            return self.events[pos].1.end;
                        }
                        depth -= 1;
                    } else if *t == start_tag.to_end() {
                        depth = depth.saturating_sub(1);
                    }
                }
                _ => {}
            }
            pos += 1;
        }
        0
    }

    /// Collect inline events until the matching End event. Consumes the
    /// Start event at `self.pos` too.
    fn parse_inline_until(&mut self, stop: TagEnd) -> Vec<MdInline> {
        self.pos += 1; // consume Start
        let mut tokens: Vec<MdInline> = Vec::new();

        loop {
            if self.pos >= self.events.len() {
                break;
            }
            let (ev, _) = &self.events[self.pos];
            match ev {
                Event::End(tag) => {
                    if *tag == stop {
                        self.pos += 1;
                        break;
                    }
                    self.pos += 1;
                }
                Event::Text(t) => {
                    self.push_text(&mut tokens, t.to_string());
                    self.pos += 1;
                }
                Event::SoftBreak => {
                    self.push_text(&mut tokens, "\n".to_string());
                    self.pos += 1;
                }
                Event::HardBreak => {
                    tokens.push(MdInline::Br);
                    self.pos += 1;
                }
                Event::Code(c) => {
                    tokens.push(MdInline::Codespan {
                        text: c.to_string(),
                    });
                    self.pos += 1;
                }
                Event::InlineHtml(h) => {
                    tokens.push(MdInline::Html { raw: h.to_string() });
                    self.pos += 1;
                }
                Event::TaskListMarker(_) => {
                    tokens.push(MdInline::Checkbox);
                    self.pos += 1;
                }
                Event::Start(tag) => match tag {
                    Tag::Emphasis => {
                        let inner = self.parse_inline_until(TagEnd::Emphasis);
                        tokens.push(MdInline::Em { inline: inner });
                    }
                    Tag::Strong => {
                        let inner = self.parse_inline_until(TagEnd::Strong);
                        tokens.push(MdInline::Strong { inline: inner });
                    }
                    Tag::Strikethrough => {
                        let inner = self.parse_inline_until(TagEnd::Strikethrough);
                        tokens.push(MdInline::Del { inline: inner });
                    }
                    Tag::Link { dest_url, .. } => {
                        let href = dest_url.to_string();
                        let inner = self.parse_inline_until(TagEnd::Link);
                        tokens.push(MdInline::Link {
                            href,
                            inline: inner,
                        });
                    }
                    Tag::Image { .. } => {
                        let _inner = self.parse_inline_until(TagEnd::Image);
                        tokens.push(MdInline::Image);
                    }
                    _ => {
                        // Nested block inside inline (shouldn't happen).
                        self.pos += 1;
                        while self.pos < self.events.len() {
                            if matches!(self.events[self.pos].0, Event::End(_)) {
                                self.pos += 1;
                                break;
                            }
                            self.pos += 1;
                        }
                    }
                },
                _ => {
                    self.pos += 1;
                }
            }
        }

        tokens
    }

    /// Append text, merging consecutive text runs (marked merges them).
    fn push_text(&self, tokens: &mut Vec<MdInline>, text: String) {
        if let Some(MdInline::Text { text: last }) = tokens.last_mut() {
            last.push_str(&text);
        } else {
            tokens.push(MdInline::Text { text });
        }
    }

    /// Bare text at block level (tight list items) — wrap the inline events
    /// into a block Text token. `first_range` is the range of the first
    /// event (the one at `self.pos`).
    fn parse_bare_text_block(
        &mut self,
        first_range: std::ops::Range<usize>,
    ) -> (MdBlock, LineSpan) {
        let start_byte = first_range.start;
        let mut inline: Vec<MdInline> = Vec::new();
        let mut end_byte = first_range.end;
        loop {
            if self.pos >= self.events.len() {
                break;
            }
            let (ev, range) = &self.events[self.pos];
            match ev {
                Event::Text(t) => {
                    self.push_text(&mut inline, t.to_string());
                    end_byte = range.end;
                    self.pos += 1;
                }
                Event::SoftBreak => {
                    self.push_text(&mut inline, "\n".to_string());
                    end_byte = range.end;
                    self.pos += 1;
                }
                Event::HardBreak => {
                    inline.push(MdInline::Br);
                    end_byte = range.end;
                    self.pos += 1;
                }
                Event::Code(c) => {
                    inline.push(MdInline::Codespan {
                        text: c.to_string(),
                    });
                    end_byte = range.end;
                    self.pos += 1;
                }
                Event::InlineHtml(h) => {
                    inline.push(MdInline::Html { raw: h.to_string() });
                    end_byte = range.end;
                    self.pos += 1;
                }
                Event::TaskListMarker(_) => {
                    inline.push(MdInline::Checkbox);
                    end_byte = range.end;
                    self.pos += 1;
                }
                Event::Start(tag) => match tag {
                    Tag::Emphasis => {
                        let inner = self.parse_inline_until(TagEnd::Emphasis);
                        inline.push(MdInline::Em { inline: inner });
                        end_byte = self.events[self.pos - 1].1.end;
                    }
                    Tag::Strong => {
                        let inner = self.parse_inline_until(TagEnd::Strong);
                        inline.push(MdInline::Strong { inline: inner });
                        end_byte = self.events[self.pos - 1].1.end;
                    }
                    Tag::Strikethrough => {
                        let inner = self.parse_inline_until(TagEnd::Strikethrough);
                        inline.push(MdInline::Del { inline: inner });
                        end_byte = self.events[self.pos - 1].1.end;
                    }
                    Tag::Link { dest_url, .. } => {
                        let href = dest_url.to_string();
                        let inner = self.parse_inline_until(TagEnd::Link);
                        inline.push(MdInline::Link {
                            href,
                            inline: inner,
                        });
                        end_byte = self.events[self.pos - 1].1.end;
                    }
                    Tag::Image { .. } => {
                        let _inner = self.parse_inline_until(TagEnd::Image);
                        inline.push(MdInline::Image);
                        end_byte = self.events[self.pos - 1].1.end;
                    }
                    _ => break,
                },
                _ => break,
            }
        }
        let span = (self.line_of(start_byte), self.line_of(end_byte - 1));
        (MdBlock::Text { inline }, span)
    }

    /// Insert `Def` blocks for link reference definition lines that aren't
    /// covered by any block's line span (i.e. are real top-level defs, not
    /// inline paragraph content).
    fn insert_defs(
        &self,
        blocks: &mut Vec<MdBlock>,
        spans: &mut Vec<LineSpan>,
        container: LineSpan,
    ) {
        static DEF_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let def_re = DEF_RE.get_or_init(|| Regex::new(r"^ {0,3}\[[^\]\n]+\]:[ \t]*\S").unwrap());

        let mut defs: Vec<(usize, LineSpan)> = Vec::new();
        for line in container.0..=container.1 {
            if !def_re.is_match(self.line_content(line)) {
                continue;
            }
            let covered = spans.iter().any(|&(fs, ls)| fs <= line && line <= ls);
            if !covered {
                defs.push((line, (line, line)));
            }
        }
        if defs.is_empty() {
            return;
        }
        let mut i = 0;
        for (line, span) in defs {
            while i < blocks.len() && spans[i].1 < line {
                i += 1;
            }
            blocks.insert(i, MdBlock::Def);
            spans.insert(i, span);
            i += 1;
        }
    }

    /// Insert `Space` tokens between blocks separated by blank line(s), and
    /// after the last block when the container ends with blank line(s).
    /// `container` = `(first_line, last_content_line)` (inclusive); blank
    /// lines at the container's own last content line never count (they are
    /// terminators, matching marked's per-container lexing).
    fn insert_spaces(
        &self,
        blocks: &mut Vec<MdBlock>,
        spans: &mut Vec<LineSpan>,
        container: LineSpan,
    ) {
        let mut out: Vec<MdBlock> = Vec::with_capacity(blocks.len() * 2);
        let mut out_spans: Vec<LineSpan> = Vec::with_capacity(blocks.len() * 2);

        let blank = |line: usize| -> bool { line < container.1 && self.is_blank_line(line) };

        for i in 0..blocks.len() {
            out.push(std::mem::replace(&mut blocks[i], MdBlock::Space));
            out_spans.push(spans[i]);

            let last_line = spans[i].1;
            let next_first = blocks
                .get(i + 1)
                .map(|_| spans[i + 1].0)
                .unwrap_or(container.1);
            let mut has_blank = false;
            for l in last_line + 1..=next_first {
                if blank(l) {
                    has_blank = true;
                    break;
                }
            }
            if has_blank {
                out.push(MdBlock::Space);
                out_spans.push((0, 0));
            }
        }

        *blocks = out;
        *spans = out_spans;
    }
}

// ─── Markdown Renderer ────────────────────────────────────────────────────

pub struct MarkdownRenderer {
    theme: MarkdownTheme,
    default_text_style: Option<DefaultTextStyle>,
    default_style_prefix: Option<String>,
    padding_x: usize,
    padding_y: usize,

    // Render cache
    cached_text: Option<String>,
    cached_width: Option<usize>,
    cached_lines: Option<Vec<String>>,

    text: String,
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        Self {
            theme: MarkdownTheme::default(),
            default_text_style: None,
            default_style_prefix: None,
            padding_x: 0,
            padding_y: 0,
            cached_text: None,
            cached_width: None,
            cached_lines: None,
            text: String::new(),
        }
    }

    /// Constructor with partial theme overrides (`{...defaults, ...theme}`).
    pub fn with_theme(theme: MarkdownThemePartial) -> Self {
        let mut r = Self::new();
        r.theme = MarkdownTheme::default().with_partial(theme);
        r
    }

    /// Constructor with theme overrides + default text style.
    pub fn with_theme_and_style(
        theme: MarkdownThemePartial,
        default_text_style: Option<DefaultTextStyle>,
    ) -> Self {
        let mut r = Self::with_theme(theme);
        r.default_text_style = default_text_style;
        r
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.invalidate();
    }

    pub fn set_padding(&mut self, x: usize, y: Option<usize>) {
        self.padding_x = x;
        if let Some(y) = y {
            self.padding_y = y;
        }
        self.invalidate();
    }

    pub fn invalidate(&mut self) {
        self.cached_text = None;
        self.cached_width = None;
        self.cached_lines = None;
    }

    /// Legacy dual API: `render(text, width)` — the `Component::render`
    /// variant renders the internal `text` field.
    pub fn render_text(&mut self, text: &str, width: usize) -> Vec<String> {
        self.render_impl(text, width)
    }

    fn render_impl(&mut self, effective_text: &str, max_width: usize) -> Vec<String> {
        if let (Some(lines), Some(cached_text), Some(cached_width)) =
            (&self.cached_lines, &self.cached_text, &self.cached_width)
        {
            if cached_text.as_str() == effective_text && *cached_width == max_width {
                return lines.clone();
            }
        }

        let content_width = std::cmp::max(1, max_width.saturating_sub(self.padding_x * 2));
        let normalized_text = effective_text.replace('\t', "   ");

        let mut rendered_lines: Vec<String> = Vec::new();

        if is_image_line(effective_text) {
            // Passthrough Kitty image lines unmodified.
            rendered_lines.push(effective_text.to_string());
        } else {
            let mut parser = MdParser::new(&normalized_text);
            let container = (0, parser.line_of(normalized_text.len()));
            let (blocks, _spans) = parser.parse_blocks(None, container, true, true);
            let style_ctx = self.get_default_inline_style_context();

            for (i, block) in blocks.iter().enumerate() {
                let next_type = blocks.get(i + 1).map(|b| b.type_name());
                rendered_lines.extend(self.render_block(
                    block,
                    content_width,
                    next_type,
                    &style_ctx,
                ));
            }
        }

        // Word-wrap all lines via wrap_text_with_ansi.
        let mut wrapped_lines: Vec<String> = Vec::new();
        for line in rendered_lines {
            wrapped_lines.extend(wrap_text_with_ansi(&line, content_width));
        }

        // Apply background to lines if default bgColor is set.
        if let Some(style) = &self.default_text_style {
            if let Some(bg_color_fn) = &style.bg_color {
                let sentinel = "\u{0}";
                let styled = bg_color_fn(sentinel);
                let bg_num = extract_bg_num(&styled).unwrap_or(235);
                for line in wrapped_lines.iter_mut() {
                    *line = apply_background_to_line(line, content_width, bg_num as i16);
                }
            }
        }

        // Apply paddingY (empty lines before/after).
        let px = " ".repeat(self.padding_x);
        let mut padded_lines: Vec<String> = Vec::new();
        for _ in 0..self.padding_y {
            padded_lines.push(String::new());
        }
        for line in wrapped_lines {
            padded_lines.push(format!("{px}{line}{px}"));
        }
        for _ in 0..self.padding_y {
            padded_lines.push(String::new());
        }

        self.cached_text = Some(effective_text.to_string());
        self.cached_width = Some(max_width);
        self.cached_lines = Some(padded_lines.clone());
        padded_lines
    }

    // ─── Default text style ─────────────────────────────────────────────

    fn apply_default_style(&self, text: &str) -> String {
        let Some(style) = &self.default_text_style else {
            return text.to_string();
        };
        let mut styled = text.to_string();
        if let Some(color) = &style.color {
            styled = color(&styled);
        }
        if style.bold {
            styled = (self.theme.bold)(&styled);
        }
        if style.italic {
            styled = (self.theme.italic)(&styled);
        }
        if style.strikethrough {
            styled = (self.theme.strikethrough)(&styled);
        }
        if style.underline {
            styled = (self.theme.underline)(&styled);
        }
        styled
    }

    fn get_default_style_prefix(&mut self) -> String {
        if self.default_text_style.is_none() {
            return String::new();
        }
        if let Some(prefix) = &self.default_style_prefix {
            return prefix.clone();
        }
        let sentinel = "\u{0}";
        let style = self.default_text_style.as_ref().expect("guarded above");
        let mut styled = sentinel.to_string();
        if let Some(color) = &style.color {
            styled = color(&styled);
        }
        if style.bold {
            styled = (self.theme.bold)(&styled);
        }
        if style.italic {
            styled = (self.theme.italic)(&styled);
        }
        if style.strikethrough {
            styled = (self.theme.strikethrough)(&styled);
        }
        if style.underline {
            styled = (self.theme.underline)(&styled);
        }
        let prefix = match styled.find(sentinel) {
            Some(idx) => styled[..idx].to_string(),
            None => String::new(),
        };
        self.default_style_prefix = Some(prefix.clone());
        prefix
    }

    fn get_style_prefix(style_fn: &dyn Fn(&str) -> String) -> String {
        let sentinel = "\u{0}";
        let styled = style_fn(sentinel);
        match styled.find(sentinel) {
            Some(idx) => styled[..idx].to_string(),
            None => String::new(),
        }
    }

    fn get_default_inline_style_context(&mut self) -> InlineStyleContext {
        let apply_text: Rc<dyn Fn(&str) -> String> = if self.default_text_style.is_some() {
            let theme = self.theme.clone();
            let style = self.default_text_style.clone();
            Rc::new(move |text: &str| apply_default_style_fn(&theme, &style, text))
        } else {
            Rc::new(|t: &str| t.to_string())
        };
        let style_prefix = self.get_default_style_prefix();
        InlineStyleContext {
            apply_text,
            style_prefix,
        }
    }

    // ─── Block token rendering ───────────────────────────────────────────

    fn render_block(
        &mut self,
        token: &MdBlock,
        width: usize,
        next_type: Option<&str>,
        style_ctx: &InlineStyleContext,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();

        match token {
            MdBlock::Heading { depth, inline } => {
                let heading = self.theme.heading.clone();
                let bold = self.theme.bold.clone();
                let underline = self.theme.underline.clone();
                let apply_text: Rc<dyn Fn(&str) -> String> = if *depth == 1 {
                    Rc::new(move |t: &str| {
                        let u = underline(t);
                        let b = bold(&u);
                        heading(&b)
                    })
                } else {
                    Rc::new(move |t: &str| {
                        let b = bold(t);
                        heading(&b)
                    })
                };
                let style_prefix = {
                    let heading = self.theme.heading.clone();
                    let bold = self.theme.bold.clone();
                    let underline = self.theme.underline.clone();
                    if *depth == 1 {
                        Self::get_style_prefix(&move |t: &str| {
                            let u = underline(t);
                            let b = bold(&u);
                            heading(&b)
                        })
                    } else {
                        Self::get_style_prefix(&move |t: &str| {
                            let b = bold(t);
                            heading(&b)
                        })
                    }
                };
                let heading_ctx = InlineStyleContext {
                    apply_text,
                    style_prefix,
                };
                let heading_text = self.render_inline_tokens(inline, &heading_ctx);
                lines.push(heading_text);
                if next_type.is_some() && next_type != Some("space") {
                    lines.push(String::new());
                }
            }

            MdBlock::Paragraph { inline } => {
                let text = self.render_inline_tokens(inline, style_ctx);
                lines.push(text);
                if next_type.is_some() && next_type != Some("list") && next_type != Some("space") {
                    lines.push(String::new());
                }
            }

            MdBlock::Code { text, lang } => {
                let indent = self
                    .theme
                    .code_block_indent
                    .clone()
                    .unwrap_or_else(|| "  ".to_string());
                let border_line = (self.theme.code_block_border)(&"─".repeat(width.min(60)));
                lines.push(border_line.clone());
                if let Some(highlight) = &self.theme.highlight_code {
                    for hl_line in highlight(text, Some(lang)) {
                        lines.push(format!("{indent}{hl_line}"));
                    }
                } else {
                    for code_line in text.split('\n') {
                        lines.push(format!("{indent}{}", (self.theme.code_block)(code_line)));
                    }
                }
                lines.push(border_line);
                if next_type.is_some() && next_type != Some("space") {
                    lines.push(String::new());
                }
            }

            MdBlock::Blockquote { blocks } => {
                // Use raw fg+italic for quote text so internal ANSI resets
                // don't clear the style. (Hardcoded 244 in TS — the themed
                // quote/quoteBorder fields are unused by the renderer.)
                let quote_style_prefix = "\x1b[3m\x1b[38;5;244m";
                let quote_content_width = std::cmp::max(1, width.saturating_sub(2));

                let mut rendered_quote: Vec<String> = Vec::new();
                for (i, qt) in blocks.iter().enumerate() {
                    let qnext = blocks.get(i + 1).map(|b| b.type_name());
                    rendered_quote.extend(self.render_block(
                        qt,
                        quote_content_width,
                        qnext,
                        style_ctx,
                    ));
                }

                // Trim trailing empty lines.
                while rendered_quote.last().is_some_and(|l| l.is_empty()) {
                    rendered_quote.pop();
                }

                // Border uses raw fg (no RESET) so it flows into the styled
                // quote text.
                let border_raw = "\x1b[38;5;244m│ ";
                for ql in rendered_quote {
                    let styled_line =
                        format!("{quote_style_prefix}{}{RESET}", reapply_quote_style(&ql));
                    let wrapped = wrap_text_with_ansi(&styled_line, quote_content_width);
                    for wl in wrapped {
                        lines.push(format!("{border_raw}{wl}"));
                    }
                }
                if next_type.is_some() && next_type != Some("space") {
                    lines.push(String::new());
                }
            }

            MdBlock::List {
                ordered,
                start,
                items,
            } => {
                lines.extend(self.render_list(*ordered, *start, items, 0, style_ctx));
            }

            MdBlock::Hr => {
                lines.push((self.theme.hr)(&"─".repeat(width.min(80))));
                if next_type.is_some() && next_type != Some("space") {
                    lines.push(String::new());
                }
            }

            MdBlock::Html { raw } => {
                lines.push(self.apply_default_style(raw.trim()));
            }

            MdBlock::Space => {
                lines.push(String::new());
            }

            MdBlock::Table { header, rows, raw } => {
                lines.extend(self.render_table(header, rows, raw, width, next_type, style_ctx));
            }

            MdBlock::Text { inline } => {
                let text = self.render_inline_tokens(inline, style_ctx);
                lines.push(text);
            }

            MdBlock::Def => {
                // Renders nothing.
            }
        }

        lines
    }

    // ─── Inline token rendering ──────────────────────────────────────────

    fn render_inline_tokens(
        &mut self,
        tokens: &[MdInline],
        style_ctx: &InlineStyleContext,
    ) -> String {
        let mut result = String::new();
        let style_prefix = style_ctx.style_prefix.as_str();

        for token in tokens {
            match token {
                MdInline::Text { text } => {
                    result.push_str(&apply_text_nl(&*style_ctx.apply_text, text));
                }
                MdInline::Strong { inline } => {
                    let inner = self.render_inline_tokens(inline, style_ctx);
                    result.push_str(&(self.theme.bold)(&inner));
                    result.push_str(style_prefix);
                }
                MdInline::Em { inline } => {
                    let inner = self.render_inline_tokens(inline, style_ctx);
                    result.push_str(&(self.theme.italic)(&inner));
                    result.push_str(style_prefix);
                }
                MdInline::Codespan { text } => {
                    result.push_str(&(self.theme.code)(text));
                    result.push_str(style_prefix);
                }
                MdInline::Del { inline } => {
                    let inner = self.render_inline_tokens(inline, style_ctx);
                    result.push_str(&(self.theme.strikethrough)(&inner));
                    result.push_str(style_prefix);
                }
                MdInline::Link { href, inline } => {
                    let link_text = self.render_inline_tokens(inline, style_ctx);
                    let underlined = (self.theme.underline)(&link_text);
                    let styled_link = (self.theme.link)(&underlined);
                    result.push_str(&hyperlink(&styled_link, href));
                    result.push_str(style_prefix);
                }
                MdInline::Br => {
                    result.push('\n');
                }
                MdInline::Html { raw } => {
                    result.push_str(&apply_text_nl(&*style_ctx.apply_text, raw));
                }
                MdInline::Image => {}
                MdInline::Checkbox => {}
            }
        }

        // Strip trailing stylePrefix (will be re-added by parent if needed).
        while !style_prefix.is_empty() && result.ends_with(style_prefix) {
            result.truncate(result.len() - style_prefix.len());
        }

        result
    }

    // ─── List rendering ──────────────────────────────────────────────────

    fn render_list(
        &mut self,
        ordered: bool,
        start: Option<u64>,
        items: &[Vec<MdBlock>],
        depth: usize,
        style_ctx: &InlineStyleContext,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let indent = "  ".repeat(depth);
        let start_number = start.unwrap_or(1) as usize;

        let is_nested = |line: &str| -> bool {
            static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
            let re = RE.get_or_init(|| Regex::new(r"^\s+\x1b\[[0-9;]*m[-\d]").unwrap());
            re.is_match(line)
        };

        for (i, item) in items.iter().enumerate() {
            let bullet = if ordered {
                format!("{}. ", start_number + i)
            } else {
                "- ".to_string()
            };
            let item_lines = self.render_list_item(item, depth, style_ctx);

            if !item_lines.is_empty() {
                let first_line = &item_lines[0];
                let is_nested_list = is_nested(first_line);

                if is_nested_list {
                    lines.push(first_line.clone());
                } else {
                    lines.push(format!(
                        "{indent}{}{first_line}",
                        (self.theme.list_bullet)(&bullet)
                    ));
                }

                for line in &item_lines[1..] {
                    let is_nested_line = is_nested(line);
                    if is_nested_line {
                        lines.push(line.clone());
                    } else {
                        lines.push(format!("{indent}  {line}"));
                    }
                }
            } else {
                lines.push(format!("{indent}{}", (self.theme.list_bullet)(&bullet)));
            }
        }

        lines
    }

    fn render_list_item(
        &mut self,
        tokens: &[MdBlock],
        parent_depth: usize,
        style_ctx: &InlineStyleContext,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();

        for token in tokens {
            match token {
                MdBlock::List {
                    ordered,
                    start,
                    items,
                } => {
                    lines.extend(self.render_list(
                        *ordered,
                        *start,
                        items,
                        parent_depth + 1,
                        style_ctx,
                    ));
                }
                MdBlock::Text { inline } => {
                    let text = self.render_inline_tokens(inline, style_ctx);
                    lines.push(text);
                }
                MdBlock::Paragraph { inline } => {
                    let text = self.render_inline_tokens(inline, style_ctx);
                    lines.push(text);
                }
                MdBlock::Code { text, lang } => {
                    let indent = self
                        .theme
                        .code_block_indent
                        .clone()
                        .unwrap_or_else(|| "  ".to_string());
                    let border_line = (self.theme.code_block_border)(&"─".repeat(60));
                    lines.push(border_line.clone());
                    if let Some(highlight) = &self.theme.highlight_code {
                        for hl_line in highlight(text, Some(lang)) {
                            lines.push(format!("{indent}{hl_line}"));
                        }
                    } else {
                        for code_line in text.split('\n') {
                            lines.push(format!("{indent}{}", (self.theme.code_block)(code_line)));
                        }
                    }
                    lines.push(border_line);
                }
                MdBlock::Html { raw } => {
                    // TS: renderInlineTokens([htmlToken]) — html tokens have
                    // `text` (the raw html) → applyTextNl(raw).
                    lines.push(apply_text_nl(&*style_ctx.apply_text, raw));
                }
                MdBlock::Blockquote { blocks } => {
                    // TS: renderInlineTokens([blockquote]) — the default
                    // case renders `token.text` = the marker-stripped raw
                    // content (marked's blockquote token `text` field).
                    let text = blockquote_raw_text(blocks);
                    if !text.is_empty() {
                        lines.push(apply_text_nl(&*style_ctx.apply_text, &text));
                    }
                }
                _ => {
                    // space/hr/def/table render nothing inline.
                }
            }
        }

        lines
    }

    // ─── Table rendering (with cell wrapping) ────────────────────────────

    fn get_longest_word_width(&self, text: &str, max_width: Option<usize>) -> usize {
        let mut longest = 0usize;
        for word in text.split_whitespace() {
            longest = longest.max(visible_width(word));
        }
        match max_width {
            Some(m) => longest.min(m),
            None => longest,
        }
    }

    fn wrap_cell_text(&self, text: &str, max_width: usize) -> Vec<String> {
        wrap_text_with_ansi(text, std::cmp::max(1, max_width))
    }

    fn render_table(
        &mut self,
        header: &[Vec<MdInline>],
        rows: &[Vec<Vec<MdInline>>],
        raw: &str,
        available_width: usize,
        next_type: Option<&str>,
        style_ctx: &InlineStyleContext,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let num_cols = header.len();
        if num_cols == 0 {
            return lines;
        }

        let border_overhead = 3 * num_cols + 1;
        let avail_for_cells = available_width as isize - border_overhead as isize;
        if avail_for_cells < num_cols as isize {
            // Too narrow — fall back to raw text.
            let mut fallback = if !raw.is_empty() {
                wrap_text_with_ansi(raw, available_width)
            } else {
                Vec::new()
            };
            if next_type.is_some() && next_type != Some("space") {
                fallback.push(String::new());
            }
            return fallback;
        }

        // Gather cell text.
        let mut render_cell =
            |inline: &Vec<MdInline>| -> String { self.render_inline_tokens(inline, style_ctx) };
        let mut header_texts: Vec<String> = Vec::with_capacity(header.len());
        for h in header {
            header_texts.push(render_cell(h));
        }
        let mut row_texts: Vec<Vec<String>> = Vec::with_capacity(rows.len());
        for row in rows {
            let mut r: Vec<String> = Vec::with_capacity(row.len());
            for cell in row {
                r.push(render_cell(cell));
            }
            row_texts.push(r);
        }

        // Natural widths (minimum to show all content without wrapping).
        let mut natural_widths: Vec<usize> = vec![0; num_cols];
        let mut min_word_widths: Vec<usize> = vec![1; num_cols];
        let max_unbroken_word = 30;

        for i in 0..num_cols {
            natural_widths[i] = visible_width(&strip_ansi_codes(&header_texts[i]));
            min_word_widths[i] = std::cmp::max(
                1,
                self.get_longest_word_width(
                    &strip_ansi_codes(&header_texts[i]),
                    Some(max_unbroken_word),
                ),
            );
        }
        for row in &row_texts {
            for i in 0..num_cols {
                if i >= row.len() {
                    continue;
                }
                let cell_text = &row[i];
                natural_widths[i] =
                    natural_widths[i].max(visible_width(&strip_ansi_codes(cell_text)));
                min_word_widths[i] =
                    min_word_widths[i].max(self.get_longest_word_width(
                        &strip_ansi_codes(cell_text),
                        Some(max_unbroken_word),
                    ));
            }
        }

        // Ensure min word widths don't exceed available.
        let mut min_col_widths = min_word_widths.clone();
        let mut min_total: usize = min_col_widths.iter().sum();

        if min_total as isize > avail_for_cells {
            // Shrink to fit.
            min_col_widths = vec![1; num_cols];
            let remaining = avail_for_cells - num_cols as isize;
            if remaining > 0 {
                // total_weight is always > 0 here: min_total > avail ≥
                // num_cols ⇒ Σ(w-1) = min_total - num_cols > 0.
                let total_weight = min_word_widths
                    .iter()
                    .map(|w| w.saturating_sub(1))
                    .sum::<usize>()
                    .max(1);
                for i in 0..num_cols {
                    let weight = min_word_widths[i].saturating_sub(1);
                    let add =
                        (weight as f64 / total_weight as f64 * remaining as f64).floor() as usize;
                    min_col_widths[i] += add;
                }
                let allocated: usize = min_col_widths.iter().sum();
                // JS distributes `leftover = remaining - allocated` one cell
                // at a time; here it is always ≤ 0 (each floor loses < 1, so
                // Σadd > remaining - num_cols), making the loop a no-op.
                debug_assert!(remaining - allocated as isize <= 0);
            }
            min_total = min_col_widths.iter().sum();
        }

        // Determine final column widths.
        let total_natural: usize = natural_widths.iter().sum();
        let col_widths: Vec<usize> = if total_natural + border_overhead <= available_width {
            (0..num_cols)
                .map(|i| natural_widths[i].max(min_col_widths[i]))
                .collect()
        } else {
            // Grow proportionally. total_grow > 0 here: if every natural
            // width equaled its min, the first branch would have matched
            // (natural == min ⇒ total_natural + border ≤ avail + border).
            let total_grow: usize = natural_widths
                .iter()
                .enumerate()
                .map(|(i, w)| w.saturating_sub(min_col_widths[i]))
                .sum::<usize>()
                .max(1);
            let extra = std::cmp::max(0, avail_for_cells - min_total as isize) as usize;
            let mut cols: Vec<usize> = min_col_widths
                .iter()
                .enumerate()
                .map(|(i, min_w)| {
                    let delta = natural_widths[i].saturating_sub(*min_w);
                    let grow = (delta as f64 / total_grow as f64 * extra as f64).floor() as usize;
                    min_w + grow
                })
                .collect();
            // Distribute remainder.
            // JS: `let remaining = availForCells - sum;` — can be negative,
            // in which case the distribution loop is skipped. Signed
            // arithmetic matches JS (usize would underflow).
            let mut remaining = avail_for_cells - cols.iter().sum::<usize>() as isize;
            for i in 0..num_cols {
                if remaining <= 0 {
                    break;
                }
                if cols[i] < natural_widths[i] {
                    cols[i] += 1;
                    remaining -= 1;
                }
            }
            cols
        };

        // Top border.
        let widths_str = col_widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┬─");
        lines.push(format!("┌─{widths_str}─┐"));

        // Header (with wrapping).
        let header_cell_lines: Vec<Vec<String>> = header_texts
            .iter()
            .enumerate()
            .map(|(i, t)| self.wrap_cell_text(t, col_widths[i]))
            .collect();
        let header_row_count = header_cell_lines.iter().map(|c| c.len()).max().unwrap_or(0);
        for li in 0..header_row_count {
            let parts: Vec<String> = header_cell_lines
                .iter()
                .enumerate()
                .map(|(ci, cl)| {
                    let t = cl.get(li).cloned().unwrap_or_default();
                    let pad = " ".repeat(col_widths[ci].saturating_sub(visible_width(&t)));
                    (self.theme.bold)(&format!("{t}{pad}"))
                })
                .collect();
            lines.push(format!("│ {} │", parts.join(" │ ")));
        }

        // Separator.
        let sep_widths = col_widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┼─");
        lines.push(format!("├─{sep_widths}─┤"));

        // Rows (with wrapping).
        for (ri, row) in row_texts.iter().enumerate() {
            let row_cell_lines: Vec<Vec<String>> = row
                .iter()
                .enumerate()
                .map(|(i, t)| self.wrap_cell_text(t, col_widths[i]))
                .collect();
            let row_line_count = row_cell_lines.iter().map(|c| c.len()).max().unwrap_or(0);

            for li in 0..row_line_count {
                let parts: Vec<String> = row_cell_lines
                    .iter()
                    .enumerate()
                    .map(|(ci, cl)| {
                        let t = cl.get(li).cloned().unwrap_or_default();
                        let pad = " ".repeat(col_widths[ci].saturating_sub(visible_width(&t)));
                        format!("{t}{pad}")
                    })
                    .collect();
                lines.push(format!("│ {} │", parts.join(" │ ")));
            }

            if ri < row_texts.len() - 1 {
                lines.push(format!(
                    "├─{}─┤",
                    col_widths
                        .iter()
                        .map(|w| "─".repeat(*w))
                        .collect::<Vec<_>>()
                        .join("─┼─")
                ));
            }
        }

        // Bottom border.
        lines.push(format!(
            "└─{}─┘",
            col_widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┴─")
        ));

        if next_type.is_some() && next_type != Some("space") {
            lines.push(String::new());
        }
        lines
    }
}

impl Component for MarkdownRenderer {
    fn render(&mut self, width: usize) -> Vec<String> {
        let text = self.text.clone();
        self.render_impl(&text, width)
    }

    fn handle_input(&mut self, _data: &str) {}

    fn invalidate(&mut self) {
        MarkdownRenderer::invalidate(self);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Re-apply quote style after every ANSI reset (`\x1b[0m` or `\x1b[m`).
fn reapply_quote_style(line: &str) -> String {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\x1b\[0?m").unwrap());
    re.replace_all(line, "\x1b[0m\x1b[3m\x1b[38;5;244m")
        .into_owned()
}

/// `styled.match(/48;5;(\d+)/)` → bg number.
fn extract_bg_num(styled: &str) -> Option<usize> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"48;5;(\d+)").unwrap());
    re.captures(styled)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<usize>().ok())
}

/// `applyTextNl` — split on `\n`, style each line, rejoin.
fn apply_text_nl(apply_text: &dyn Fn(&str) -> String, text: &str) -> String {
    text.split('\n')
        .map(apply_text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// `applyDefaultStyle` as a standalone fn (owned theme + style).
fn apply_default_style_fn(
    theme: &MarkdownTheme,
    style: &Option<DefaultTextStyle>,
    text: &str,
) -> String {
    let Some(style) = style else {
        return text.to_string();
    };
    let mut styled = text.to_string();
    if let Some(color) = &style.color {
        styled = color(&styled);
    }
    if style.bold {
        styled = (theme.bold)(&styled);
    }
    if style.italic {
        styled = (theme.italic)(&styled);
    }
    if style.strikethrough {
        styled = (theme.strikethrough)(&styled);
    }
    if style.underline {
        styled = (theme.underline)(&styled);
    }
    styled
}

/// marked's blockquote token `text` field: content lines with `> ` markers
/// stripped, trailing empty lines dropped, joined with `\n`.
fn blockquote_raw_text(blocks: &[MdBlock]) -> String {
    // The blockquote's blocks don't retain the raw source; reconstruct the
    // marker-stripped text from a serialized approximation. This mirrors
    // marked's `text` field for the common single-paragraph case.
    fn render_inline_text(inline: &[MdInline]) -> String {
        let mut s = String::new();
        for t in inline {
            match t {
                MdInline::Text { text } => s.push_str(text),
                MdInline::Codespan { text } => s.push_str(text),
                MdInline::Strong { inline }
                | MdInline::Em { inline }
                | MdInline::Del { inline } => {
                    s.push_str(&render_inline_text(inline));
                }
                MdInline::Link { inline, .. } => s.push_str(&render_inline_text(inline)),
                MdInline::Br => s.push('\n'),
                MdInline::Html { raw } => s.push_str(raw),
                MdInline::Image | MdInline::Checkbox => {}
            }
        }
        s
    }
    let mut lines: Vec<String> = Vec::new();
    for b in blocks {
        match b {
            MdBlock::Paragraph { inline } => lines.push(render_inline_text(inline)),
            MdBlock::Text { inline } => lines.push(render_inline_text(inline)),
            MdBlock::Code { text, .. } => {
                for l in text.split('\n') {
                    lines.push(l.to_string());
                }
            }
            _ => {}
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    fn render(md: &str, width: usize) -> Vec<String> {
        let mut r = MarkdownRenderer::new();
        r.render_text(md, width)
    }

    fn plain(md: &str, width: usize) -> Vec<String> {
        render(md, width)
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect()
    }

    #[test]
    fn heading_renders_and_trails_blank_line() {
        let lines = plain("# Head", 40);
        assert_eq!(lines, vec!["Head"]);
        let lines = plain("# Head\n\ntext", 40);
        assert_eq!(lines, vec!["Head", "", "text"]);
    }

    #[test]
    fn heading_no_blank_line_between() {
        // heading followed directly by a paragraph still gets a blank line
        // (marked renderer behavior: nextType != "space").
        let lines = plain("# Head\ntext", 40);
        assert_eq!(lines, vec!["Head", "", "text"]);
    }

    #[test]
    fn paragraphs_separated_by_blank_line() {
        let lines = plain("a\n\nb", 40);
        assert_eq!(lines, vec!["a", "", "b"]);
    }

    #[test]
    fn multiple_blank_lines_collapse_to_one() {
        let lines = plain("a\n\n\n\nb", 40);
        assert_eq!(lines, vec!["a", "", "b"]);
    }

    #[test]
    fn soft_break_splits_into_two_wrapped_lines() {
        // wrapTextWithAnsi splits on \n (TS parity: "a\xb1b[0m", "b\xb1b[0m").
        let lines = render("a\nb", 40);
        assert_eq!(lines, vec!["a\x1b[0m", "b\x1b[0m"]);
    }

    #[test]
    fn hard_break_renders_newline() {
        let lines = render("a  \nb", 40);
        assert_eq!(lines, vec!["a\x1b[0m", "b\x1b[0m"]);
    }

    #[test]
    fn strong_em_codespan_styles() {
        let lines = render("**b** *i* `c`", 40);
        assert_eq!(
            lines[0],
            "\x1b[1mb\x1b[m \x1b[3mi\x1b[m \x1b[38;5;151mc\x1b[m\x1b[0m"
        );
    }

    #[test]
    fn strikethrough_renders() {
        let lines = render("~~gone~~", 40);
        assert_eq!(lines[0], "\x1b[38;5;244mgone\x1b[m\x1b[0m");
    }

    #[test]
    fn strict_strikethrough_rejects_space_after_open() {
        // marked's StrictStrikethroughTokenizer: "~~ strike~~" is NOT a del.
        let lines = plain("~~ strike~~", 40);
        assert_eq!(lines[0], "~~ strike~~");
    }

    #[test]
    fn link_renders_hyperlink() {
        let lines = render("[x](https://e.com)", 40);
        assert_eq!(
            lines[0],
            "\x1b]8;;https://e.com\x1b\\\x1b[38;5;117m\x1b[4mx\x1b[m\x1b[m\x1b]8;;\x1b\\\x1b[0m"
        );
    }

    #[test]
    fn image_renders_nothing() {
        let lines = plain("![alt](img.png)", 40);
        assert_eq!(lines, vec![""]);
    }

    #[test]
    fn list_bullets() {
        let lines = plain("- a\n- b", 40);
        assert_eq!(lines, vec!["- a", "- b"]);
    }

    #[test]
    fn ordered_list_numbers() {
        let lines = plain("1. first\n2. second", 40);
        assert_eq!(lines, vec!["1. first", "2. second"]);
    }

    #[test]
    fn list_then_paragraph_blank_line() {
        let lines = plain("- a\n- b\n\npara", 40);
        assert_eq!(lines, vec!["- a", "- b", "", "para"]);
    }

    #[test]
    fn nested_list_indentation() {
        let lines = plain("- top\n  - inner\n- bottom", 40);
        assert_eq!(lines, vec!["- top", "  - inner", "- bottom"]);
    }

    #[test]
    fn loose_list_blank_lines() {
        let lines = plain("- a\n\n- b", 40);
        assert_eq!(lines, vec!["- a", "- b"]);
    }

    #[test]
    fn task_list_renders_without_checkbox() {
        let lines = plain("- [x] done\n- [ ] todo", 40);
        assert_eq!(lines, vec!["- done", "- todo"]);
    }

    #[test]
    fn code_block_borders_and_indent() {
        let lines = plain("```ts\nconst x = 1;\n```", 40);
        assert_eq!(lines[0], "─".repeat(40));
        assert_eq!(lines[1], "  const x = 1;");
        assert_eq!(lines[2], "─".repeat(40));
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn code_block_no_blank_between_blocks() {
        let lines = plain("```\na\n```\n\npara", 40);
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], "para");
    }

    #[test]
    fn indented_code_block() {
        // border + "  " indent + styled code line
        let lines = plain("    code\n    line two", 40);
        assert_eq!(
            lines,
            vec![
                "─".repeat(40),
                "  code".to_string(),
                "  line two".to_string(),
                "─".repeat(40)
            ]
        );
    }

    #[test]
    fn blockquote_border_and_style() {
        let lines = render("> quote", 40);
        assert_eq!(
            lines[0],
            "\x1b[38;5;244m│ \x1b[3m\x1b[38;5;244mquote\x1b[m\x1b[0m\x1b[0m"
        );
    }

    #[test]
    fn blockquote_trailing_empty_marker_no_blank() {
        let lines = plain("> q\n>", 40);
        assert_eq!(lines, vec!["│ q"]);
    }

    #[test]
    fn blockquote_trailing_markers_trimmed() {
        // Two trailing empty markers DO create a space token inside marked,
        // but the renderer trims trailing empty lines of the quote.
        let lines = plain("> q\n>\n>", 40);
        assert_eq!(lines, vec!["│ q"]);
    }

    #[test]
    fn nested_blockquote() {
        // Each blockquote level adds its own "│ " border.
        let lines = plain(">> nested", 40);
        assert_eq!(lines, vec!["│ │ nested"]);
    }

    #[test]
    fn hr_renders_line() {
        let lines = plain("a\n\n---\n\nb", 40);
        assert_eq!(lines[0], "a");
        assert_eq!(lines[1], "");
        assert_eq!(lines[2], "─".repeat(40));
        assert_eq!(lines[3], "");
        assert_eq!(lines[4], "b");
    }

    #[test]
    fn hr_width_capped_at_80() {
        let lines = plain("---", 200);
        assert_eq!(visible_width(&lines[0]), 80);
    }

    #[test]
    fn html_block_trimmed() {
        // paragraph pushes a blank line before the html block (nextType !=
        // space/list); the html raw is trimmed.
        let lines = plain("para\n<hr>", 40);
        assert_eq!(lines, vec!["para", "", "<hr>"]);
    }

    #[test]
    fn inline_html_raw() {
        let lines = plain("a <span>b</span>", 40);
        assert_eq!(lines, vec!["a <span>b</span>"]);
    }

    #[test]
    fn escaped_punctuation_literal() {
        let lines = plain("\\*escaped\\*", 40);
        assert_eq!(lines, vec!["*escaped*"]);
    }

    #[test]
    fn table_renders_borders() {
        let lines = plain("| a | b |\n|---|---|\n| 1 | 2 |", 40);
        assert_eq!(lines[0], "┌───┬───┐");
        assert_eq!(lines[1], "│ a │ b │");
        assert_eq!(lines[2], "├───┼───┤");
        assert_eq!(lines[3], "│ 1 │ 2 │");
        assert_eq!(lines[4], "└───┴───┘");
    }

    #[test]
    fn table_too_narrow_falls_back_to_raw() {
        // width 8: availForCells = 8-7 = 1 < 2 cols → raw fallback wrapped
        let lines = render("| a | b |\n|---|---|\n| 1 | 2 |", 8);
        assert!(!lines.is_empty());
        assert!(strip_ansi_codes(&lines.join("\n")).contains("a | b"));
    }

    #[test]
    fn table_column_width_algorithm() {
        let lines = plain(
            "| long header one | long header two |\n|---|---|\n| 1 | 2 |",
            40,
        );
        let expected = format!("┌─{}─┬─{}─┐", "─".repeat(15), "─".repeat(15));
        assert_eq!(lines[0], expected);
        assert!(lines.join("\n").contains("long header one"));
    }

    #[test]
    fn table_with_wrapping_cells() {
        let lines = plain(
            "| short |\n|---|\n| a very long word that will not fit and wraps around |",
            20,
        );
        let joined = lines.join("\n");
        assert!(joined.contains("wraps"));
        assert!(lines.iter().all(|l| visible_width(l) <= 20));
    }

    #[test]
    fn setext_heading() {
        let lines = plain("setext\n======", 40);
        assert_eq!(lines, vec!["setext"]);
    }

    #[test]
    fn reference_link_definition_renders_nothing_but_separates() {
        // def renders nothing; the surrounding spaces still appear (marked
        // gives TWO blank lines here: space before and after the def).
        let lines = plain("a\n\n[x]: https://e.com\n\nb", 40);
        assert_eq!(lines, vec!["a", "", "", "b"]);
    }

    #[test]
    fn reference_link_resolves() {
        let lines = render("see [x] here\n\n[x]: https://e.com\n", 40);
        let joined = lines.join("\n");
        assert!(joined.contains("\x1b]8;;https://e.com\x1b\\"));
    }

    #[test]
    fn autolink_renders() {
        let lines = render("<https://auto.com>", 40);
        assert!(lines[0].contains("\x1b]8;;https://auto.com\x1b\\"));
    }

    #[test]
    fn image_line_passthrough() {
        let kitty = "\x1b_Ga=T,f=100;QUJD\x1b\\";
        let lines = render(kitty, 40);
        // Passthrough line, then the outer wrap appends its RESET.
        assert_eq!(lines, vec![format!("{kitty}\x1b[0m")]);
    }

    #[test]
    fn tabs_replaced_with_three_spaces() {
        let lines = plain("a\tb", 40);
        assert_eq!(lines, vec!["a   b"]);
    }

    #[test]
    fn word_wrap_applies() {
        let lines = plain("aaaa bbbb cccc", 10);
        assert_eq!(lines, vec!["aaaa bbbb", "cccc"]);
    }

    #[test]
    fn width_change_invalidates_cache() {
        let mut r = MarkdownRenderer::new();
        let w40 = r.render_text("aaa bbb ccc", 40);
        let w10 = r.render_text("aaa bbb ccc", 10);
        assert_ne!(w40, w10);
        let w40b = r.render_text("aaa bbb ccc", 40);
        assert_eq!(w40, w40b);
    }

    #[test]
    fn set_text_and_component_render() {
        let mut r = MarkdownRenderer::new();
        r.set_text("hello");
        let lines = Component::render(&mut r, 40);
        assert_eq!(strip_ansi_codes(&lines[0]), "hello");
    }

    #[test]
    fn padding_applied() {
        let mut r = MarkdownRenderer::new();
        r.set_padding(2, Some(1));
        let lines = r.render_text("hi", 40);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "");
        assert_eq!(lines[2], "");
        // "  " + "hi\x1b[0m" + "  " (padding after the wrap reset)
        assert_eq!(lines[1], "  hi\x1b[0m  ");
    }

    #[test]
    fn default_text_style_applied() {
        let mut r = MarkdownRenderer::new();
        r.default_text_style = Some(DefaultTextStyle {
            bold: true,
            ..Default::default()
        });
        let lines = r.render_text("hi", 40);
        assert_eq!(lines[0], "\x1b[1mhi\x1b[m\x1b[0m");
    }

    #[test]
    fn default_style_prefix_restored_after_inline_styles() {
        let mut r = MarkdownRenderer::new();
        r.default_text_style = Some(DefaultTextStyle {
            bold: true,
            ..Default::default()
        });
        let lines = r.render_text("a `c` b", 40);
        let s = strip_ansi_codes(&lines[0]);
        assert_eq!(s, "a c b");
        assert!(lines[0].contains("\x1b[1m"));
    }

    #[test]
    fn trailing_blank_lines_at_eof() {
        let lines = plain("text\n\n", 40);
        assert_eq!(lines, vec!["text", ""]);
    }

    #[test]
    fn single_trailing_newline_no_blank() {
        let lines = plain("text\n", 40);
        assert_eq!(lines, vec!["text"]);
    }

    #[test]
    fn list_trailing_blank_line() {
        let lines = plain("- a\n\n", 40);
        assert_eq!(lines, vec!["- a", ""]);
    }

    #[test]
    fn quote_between_quotes() {
        let lines = plain("> q\n\n> q2", 40);
        assert_eq!(lines, vec!["│ q", "", "│ q2"]);
    }

    #[test]
    fn two_lists_with_blank_between() {
        let lines = plain("- item one\n- item two\n\n1. first\n2. second", 40);
        assert_eq!(
            lines,
            vec!["- item one", "- item two", "", "1. first", "2. second"]
        );
    }

    #[test]
    fn multi_paragraph_list_item_no_blank() {
        // marked renderListItem has no `space` case: the space token inside
        // an item falls through to the else branch and renders nothing.
        let lines = plain("- a\n\n  b", 40);
        assert_eq!(lines, vec!["- a", "  b"]);
    }

    #[test]
    fn heading_levels_bold_and_underline() {
        // h1: heading(bold(underline(t))) → fg, bold, bold, underline,
        // 4 resets (TS parity — verified byte-identical by the harness).
        let h1 = render("# h1", 40);
        assert_eq!(
            h1[0],
            "\x1b[38;5;221m\x1b[1m\x1b[1m\x1b[4mh1\x1b[m\x1b[m\x1b[m\x1b[m\x1b[0m"
        );
        let h2 = render("## h2", 40);
        assert_eq!(
            h2[0],
            "\x1b[38;5;221m\x1b[1m\x1b[1mh2\x1b[m\x1b[m\x1b[m\x1b[0m"
        );
    }

    #[test]
    fn emphasis_across_line_break() {
        // Strong spans the soft break; wrap splits mid-style (TS parity).
        let lines = render("para **bold\ncontinues** here", 40);
        assert_eq!(
            lines,
            vec!["para \x1b[1mbold\x1b[0m", "continues\x1b[m here\x1b[0m"]
        );
    }

    #[test]
    fn br_token_in_middle() {
        let lines = render("a  \nb", 40);
        assert_eq!(lines, vec!["a\x1b[0m", "b\x1b[0m"]);
    }

    #[test]
    fn blockquote_in_list_item_renders_raw_text() {
        // marked renderListItem else-branch renders token.text (marker
        // stripped raw) for a blockquote token.
        let lines = plain("- > quote\n- b", 40);
        assert_eq!(lines, vec!["- quote", "- b"]);
    }

    #[test]
    fn thinking_theme_renders() {
        // mdThinking-style theme: everything mapped to thinkingText (244).
        let tc: u8 = 244;
        let think_fg = move |s: &str| theme_fg(tc, s);
        let partial = MarkdownThemePartial {
            heading: Some(Rc::new(think_fg)),
            link: Some(Rc::new(think_fg)),
            code: Some(Rc::new(think_fg)),
            list_bullet: Some(Rc::new(think_fg)),
            strikethrough: Some(Rc::new(think_fg)),
            code_block: Some(Rc::new(move |s: &str| theme_fg(tc, &theme_dim(s)))),
            code_block_border: Some(Rc::new(move |s: &str| theme_fg(tc, &theme_dim(s)))),
            ..Default::default()
        };
        let mut r = MarkdownRenderer::with_theme(partial);
        let lines = r.render_text("# Plan\n\n`code` and **bold**\n\n```\nfn x()\n```\n", 40);
        let s = lines.join("\n");
        assert!(s.contains("\x1b[38;5;244m"));
        assert!(!s.contains("\x1b[38;5;151m"));
        assert!(!s.contains("\x1b[38;5;221m"));
    }

    #[test]
    fn default_theme_styles_links_and_blockquotes() {
        // Default theme closures for link/quote/quote_border.
        let lines = render("[text](https://example.com)\n\n> a quote", 40);
        let s = lines.join("\n");
        assert!(s.contains("text"));
        assert!(s.contains("a quote"));
        assert!(s.contains("\x1b[38;5;117m")); // link fg
    }

    #[test]
    fn default_theme_parity_styles_produce_output() {
        // link_url/quote/quote_border exist for TS shape parity — the
        // renderer deliberately never invokes them (quote text uses raw
        // fg+italic so resets can be re-applied). Call them once here so
        // their bodies aren't dead code.
        let theme = MarkdownTheme::default();
        assert!((theme.link_url)("x").contains("x"));
        assert!((theme.quote)("x").contains("x"));
        assert!((theme.quote_border)("x").contains("x"));
    }

    #[test]
    fn empty_input_renders_empty() {
        let lines = plain("", 40);
        assert_eq!(lines, Vec::<String>::new());
    }

    #[test]
    fn paragraph_keeps_trailing_whitespace() {
        // pulldown-cmark trims trailing whitespace from Text events; marked
        // keeps it (a trailing text token). Parity harness caught the loss.
        let lines = plain("Hello ", 40);
        assert_eq!(lines, vec!["Hello "]);
        let lines = plain("x  y  ", 40);
        assert_eq!(lines, vec!["x  y  "]);
        // Paragraph followed by a blank line: source slice ends with \n —
        // only the spaces are restored, not the newline.
        let lines = plain("a \n\nb", 40);
        assert_eq!(lines, vec!["a ", "", "b"]);
    }

    #[test]
    fn h1_style_composition_order_matches_marked() {
        // TS: theme.heading(theme.bold(theme.underline(t))) — heading (fg+bold)
        // wraps bold which wraps underline. A reversed composition (heading
        // wrapping underline wrapping bold) diverges in ANSI order.
        let lines = render("# Head", 40);
        assert_eq!(
            lines[0],
            "\x1b[38;5;221m\x1b[1m\x1b[1m\x1b[4mHead\x1b[m\x1b[m\x1b[m\x1b[m\x1b[0m"
        );
        // depth-2 heading: heading(bold(t)).
        let lines = render("## Sub", 40);
        assert_eq!(
            lines[0],
            "\x1b[38;5;221m\x1b[1m\x1b[1mSub\x1b[m\x1b[m\x1b[m\x1b[0m"
        );
    }

    // ─── structural coverage ──────────────────────────────────────────

    fn render_plain(md: &str) -> Vec<String> {
        let mut r = MarkdownRenderer::new();
        r.render_text(md, 80)
            .iter()
            .map(|l| strip_ansi_codes(l))
            .collect()
    }

    #[test]
    fn type_name_covers_every_block_variant() {
        assert_eq!(
            MdBlock::Heading {
                depth: 1,
                inline: vec![]
            }
            .type_name(),
            "heading"
        );
        assert_eq!(
            MdBlock::Paragraph { inline: vec![] }.type_name(),
            "paragraph"
        );
        assert_eq!(
            MdBlock::Code {
                text: String::new(),
                lang: String::new()
            }
            .type_name(),
            "code"
        );
        assert_eq!(
            MdBlock::Blockquote { blocks: vec![] }.type_name(),
            "blockquote"
        );
        assert_eq!(
            MdBlock::List {
                ordered: false,
                start: None,
                items: vec![]
            }
            .type_name(),
            "list"
        );
        assert_eq!(MdBlock::Hr.type_name(), "hr");
        assert_eq!(MdBlock::Html { raw: String::new() }.type_name(), "html");
        assert_eq!(MdBlock::Space.type_name(), "space");
        assert_eq!(
            MdBlock::Table {
                header: vec![],
                rows: vec![],
                raw: String::new()
            }
            .type_name(),
            "table"
        );
        assert_eq!(MdBlock::Text { inline: vec![] }.type_name(), "text");
        assert_eq!(MdBlock::Def.type_name(), "def");
    }

    #[test]
    fn markdown_theme_debug_is_brief() {
        let theme = MarkdownTheme::default();
        assert_eq!(format!("{theme:?}"), "MarkdownTheme { .. }");
    }

    #[test]
    fn with_partial_applies_every_override() {
        let f: StyleFn = std::rc::Rc::new(|s: &str| format!("B{s}B"));
        let partial = MarkdownThemePartial {
            heading: Some(f.clone()),
            link: Some(f.clone()),
            link_url: Some(f.clone()),
            code: Some(f.clone()),
            code_block: Some(f.clone()),
            code_block_border: Some(f.clone()),
            quote: Some(f.clone()),
            quote_border: Some(f.clone()),
            hr: Some(f.clone()),
            list_bullet: Some(f.clone()),
            bold: Some(f.clone()),
            italic: Some(f.clone()),
            strikethrough: Some(f.clone()),
            underline: Some(f.clone()),
            highlight_code: Some(std::rc::Rc::new(|text: &str, _lang| {
                vec![format!("HL:{text}")]
            })),
            code_block_indent: Some(">>>".to_string()),
        };
        let theme = MarkdownTheme::default().with_partial(partial);
        assert_eq!((theme.bold)("x"), "BxB");
        assert_eq!((theme.italic)("x"), "BxB");
        assert_eq!((theme.strikethrough)("x"), "BxB");
        assert_eq!((theme.underline)("x"), "BxB");
        assert_eq!(theme.highlight_code.unwrap()("x", None), vec!["HL:x"]);
        assert_eq!(theme.code_block_indent.as_deref(), Some(">>>"));
    }

    #[test]
    fn renders_all_heading_depths() {
        let plain = render_plain("# H1\n\n## H2\n\n### H3\n\n#### H4\n\n##### H5\n\n###### H6");
        for h in ["H1", "H2", "H3", "H4", "H5", "H6"] {
            assert!(plain.iter().any(|l| l.contains(h)), "missing {h}");
        }
    }

    #[test]
    fn renders_indented_and_fenced_code() {
        // Fenced with language.
        let plain = render_plain("```rust\nfn main() {}\n```");
        assert!(plain.iter().any(|l| l.contains("fn main() {}")));
        // Indented code block (trailing blanks are trimmed).
        let plain = render_plain("para\n\n    let x = 1;\n\n");
        assert!(plain.iter().any(|l| l.contains("let x = 1;")));
    }

    #[test]
    fn renders_code_with_custom_highlighter_and_indent() {
        let mut r = MarkdownRenderer::with_theme(MarkdownThemePartial {
            highlight_code: Some(std::rc::Rc::new(|text: &str, lang| {
                vec![format!("[{lang:?}] {text}")]
            })),
            code_block_indent: Some(">>".to_string()),
            ..Default::default()
        });
        let lines = r.render_text("```rs\nx\n```", 80);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi_codes(l)).collect();
        assert!(plain.iter().any(|l| l.contains(">>") && l.contains("x")));
    }

    #[test]
    fn renders_blockquotes_tasklists_and_nested_lists() {
        let plain = render_plain(
            "> quoted *em*\n>\n> more\n\n- [ ] todo\n- [x] done\n\n1. first\n2. second\n",
        );
        assert!(plain.iter().any(|l| l.contains("quoted")));
        assert!(plain.iter().any(|l| l.contains("todo")));
        assert!(plain.iter().any(|l| l.contains("done")));
        assert!(plain.iter().any(|l| l.contains("first")));
        assert!(plain.iter().any(|l| l.contains("second")));
        // Nested list inside a list item.
        let plain = render_plain("- outer\n  - inner\n");
        assert!(plain.iter().any(|l| l.contains("inner")));
    }

    #[test]
    fn renders_tables_hr_html_and_defs() {
        let plain = render_plain(
            "| A | B |\n|---|---|\n| 1 | 2 |\n\n---\n\n<div>block</div>\n\n[x]: http://ref\n\nafter",
        );
        assert!(plain.iter().any(|l| l.contains('A')));
        assert!(plain.iter().any(|l| l.contains('1')));
        assert!(plain.iter().any(|l| l.contains("─")));
        assert!(plain.iter().any(|l| l.contains("div")));
        assert!(plain.iter().any(|l| l.contains("after")));
        // The link definition itself renders nothing.
        assert!(!plain.iter().any(|l| l.contains("http://ref")));
    }

    #[test]
    fn renders_inline_features() {
        let plain = render_plain(
            "has **bold** *em* ~~gone~~ `code` [lnk](http://x) <span>inline</span> ![img](i.png) and  \na break",
        );
        let joined = plain.join("\n");
        for needle in ["bold", "em", "gone", "code", "lnk", "span", "break"] {
            assert!(joined.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn default_text_style_applies_color_bg_and_attributes() {
        use crate::theme::{bg_raw, fg_raw};
        let style = DefaultTextStyle {
            color: Some(std::rc::Rc::new(|s: &str| fg_raw(196, s))),
            bg_color: Some(std::rc::Rc::new(|s: &str| bg_raw(17, s))),
            bold: true,
            italic: true,
            strikethrough: true,
            underline: true,
        };
        let mut r =
            MarkdownRenderer::with_theme_and_style(MarkdownThemePartial::default(), Some(style));
        let lines = r.render_text("styled text", 40);
        let joined = lines.join("\n");
        assert!(joined.contains("38;5;196"));
        assert!(joined.contains("48;5;17"));
        assert!(strip_ansi_codes(&joined).contains("styled text"));
    }

    #[test]
    fn renderer_component_trait_and_caching() {
        let mut r = MarkdownRenderer::default();
        r.set_text("cached body");
        let a = Component::render(&mut r, 60);
        let b = r.render(60); // cache hit
        assert_eq!(a, b);
        r.set_padding(2, Some(1));
        let c = r.render(60);
        assert!(c.len() >= a.len()); // padding adds lines
        r.invalidate();
        Component::handle_input(&mut r, "ignored");
        assert!(r.as_any().downcast_ref::<MarkdownRenderer>().is_some());
        assert!(r.as_any_mut().downcast_mut::<MarkdownRenderer>().is_some());
    }

    #[test]
    fn helpers_cover_their_branches() {
        // reapply_quote_style re-arms after every reset form.
        assert_eq!(
            reapply_quote_style("a\x1b[0mb\x1b[mc"),
            "a\x1b[0m\x1b[3m\x1b[38;5;244mb\x1b[0m\x1b[3m\x1b[38;5;244mc"
        );
        // extract_bg_num finds / misses the bg number.
        assert_eq!(extract_bg_num("\x1b[48;5;17mx"), Some(17));
        assert_eq!(extract_bg_num("\x1b[38;5;17mx"), None);
        assert_eq!(extract_bg_num("48;5;abc"), None);
        // apply_text_nl styles per line.
        let out = apply_text_nl(&|s: &str| format!("<{s}>"), "a\nb");
        assert_eq!(out, "<a>\n<b>");
        // apply_default_style_fn: None style passes through; all flags apply.
        let theme = MarkdownTheme::default();
        assert_eq!(apply_default_style_fn(&theme, &None, "plain"), "plain");
        let style = DefaultTextStyle {
            color: Some(std::rc::Rc::new(|s: &str| format!("C{s}C"))),
            bg_color: None,
            bold: true,
            italic: true,
            strikethrough: true,
            underline: true,
        };
        let out = apply_default_style_fn(&theme, &Some(style), "x");
        assert!(out.contains('C'));
        assert!(strip_ansi_codes(&out.replace('C', "")).contains('x'));
    }

    #[test]
    fn blockquote_raw_text_reconstructs_content() {
        let blocks = vec![
            MdBlock::Paragraph {
                inline: vec![
                    MdInline::Text {
                        text: "hello ".to_string(),
                    },
                    MdInline::Strong {
                        inline: vec![MdInline::Text {
                            text: "bold".into(),
                        }],
                    },
                    MdInline::Codespan {
                        text: "code".into(),
                    },
                    MdInline::Br,
                    MdInline::Em {
                        inline: vec![MdInline::Text { text: "em".into() }],
                    },
                    MdInline::Del {
                        inline: vec![MdInline::Text { text: "del".into() }],
                    },
                    MdInline::Link {
                        inline: vec![MdInline::Text { text: "lnk".into() }],
                        href: String::new(),
                    },
                    MdInline::Html {
                        raw: "<b>raw</b>".into(),
                    },
                    MdInline::Image,
                    MdInline::Checkbox,
                ],
            },
            MdBlock::Text {
                inline: vec![MdInline::Text { text: "txt".into() }],
            },
            MdBlock::Code {
                text: "c1\nc2".into(),
                lang: String::new(),
            },
            MdBlock::Hr, // ignored
        ];
        let text = blockquote_raw_text(&blocks);
        assert!(text.contains("hello boldcode"));
        assert!(text.contains("em"));
        assert!(text.contains("del"));
        assert!(text.contains("lnk"));
        assert!(text.contains("<b>raw</b>"));
        assert!(text.contains("txt"));
        assert!(text.contains("c1\nc2"));
        assert!(!text.ends_with('\n'));
    }

    // ─── white-box render drives ──────────────────────────────────────

    fn bare_ctx(r: &mut MarkdownRenderer) -> InlineStyleContext {
        r.get_default_inline_style_context()
    }

    #[test]
    fn render_block_text_def_space_and_spacers() {
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        // Top-level Text block.
        let out = r.render_block(
            &MdBlock::Text {
                inline: vec![MdInline::Text {
                    text: "tight".into(),
                }],
            },
            60,
            None,
            &ctx,
        );
        assert_eq!(strip_ansi_codes(&out[0]), "tight");
        // Def renders nothing; Space renders a blank line.
        assert!(r.render_block(&MdBlock::Def, 60, None, &ctx).is_empty());
        let ctx = bare_ctx(&mut r);
        let space = r.render_block(&MdBlock::Space, 60, None, &ctx);
        assert!(space.iter().all(|l| l.trim().is_empty()));
        // Code block followed by a non-space block gets a spacer line.
        let ctx = bare_ctx(&mut r);
        let code = MdBlock::Code {
            text: "x = 1".into(),
            lang: String::new(),
        };
        let with_next = r.render_block(&code, 60, Some("paragraph"), &ctx);
        let ctx = bare_ctx(&mut r);
        let without_next = r.render_block(&code, 60, None, &ctx);
        assert_eq!(with_next.len(), without_next.len() + 1);
        // …but not before a space block.
        let ctx = bare_ctx(&mut r);
        let before_space = r.render_block(&code, 60, Some("space"), &ctx);
        assert_eq!(before_space.len(), without_next.len());
        // Hr with a following block.
        let ctx = bare_ctx(&mut r);
        let hr = r.render_block(&MdBlock::Hr, 60, Some("paragraph"), &ctx);
        assert!(hr.last().unwrap().is_empty());
        // Html block uses the default style.
        let ctx = bare_ctx(&mut r);
        let html = r.render_block(&MdBlock::Html { raw: "<br>".into() }, 60, None, &ctx);
        assert!(strip_ansi_codes(&html[0]).contains("<br>"));
    }

    #[test]
    fn render_list_item_all_content_kinds() {
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        let item: Vec<MdBlock> = vec![
            MdBlock::Code {
                text: "item code".into(),
                lang: "rs".into(),
            },
            MdBlock::Html {
                raw: "<i>x</i>".into(),
            },
            MdBlock::Blockquote {
                blocks: vec![MdBlock::Paragraph {
                    inline: vec![MdInline::Text {
                        text: "q in item".into(),
                    }],
                }],
            },
            MdBlock::Hr, // renders nothing inline
            MdBlock::Table {
                header: vec![],
                rows: vec![],
                raw: String::new(),
            }, // nothing
            MdBlock::List {
                ordered: false,
                start: None,
                items: vec![vec![MdBlock::Text {
                    inline: vec![MdInline::Text {
                        text: "nested".into(),
                    }],
                }]],
            },
        ];
        let lines = r.render_list_item(&item, 0, &ctx);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi_codes(l)).collect();
        assert!(plain.iter().any(|l| l.contains("item code")));
        assert!(plain.iter().any(|l| l.contains("<i>x</i>")));
        assert!(plain.iter().any(|l| l.contains("q in item")));
        assert!(plain.iter().any(|l| l.contains("nested")));
    }

    #[test]
    fn render_table_narrow_fallback_and_shrink() {
        // Too narrow for borders → raw text fallback (+ spacer when a
        // non-space block follows).
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        let header = vec![vec![MdInline::Text { text: "A".into() }]];
        let rows = vec![vec![vec![MdInline::Text { text: "1".into() }]]];
        let narrow = r.render_table(
            &header,
            &rows,
            "| A |\n|---|\n| 1 |",
            2,
            Some("paragraph"),
            &ctx,
        );
        assert!(narrow.iter().any(|l| l.contains('A')));
        assert!(narrow.last().unwrap().is_empty());

        // A long unbroken word forces the shrink-to-fit path.
        let long = "x".repeat(100);
        let header2 = vec![
            vec![MdInline::Text { text: long.clone() }],
            vec![MdInline::Text { text: "B".into() }],
        ];
        let rows2 = vec![vec![
            vec![MdInline::Text { text: long }],
            vec![MdInline::Text { text: "2".into() }],
        ]];
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        let lines = r.render_table(&header2, &rows2, "", 30, None, &ctx);
        assert!(!lines.is_empty());
        // A table with an empty header renders nothing.
        let ctx = bare_ctx(&mut r);
        assert!(r.render_table(&[], &[], "", 40, None, &ctx).is_empty());
    }

    #[test]
    fn render_table_multi_row_and_ragged() {
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        let header = vec![
            vec![MdInline::Text { text: "A".into() }],
            vec![MdInline::Text { text: "B".into() }],
        ];
        // Second row has fewer cells than the header (ragged → skip arm).
        let rows = vec![
            vec![
                vec![MdInline::Text { text: "1".into() }],
                vec![MdInline::Text { text: "2".into() }],
            ],
            vec![vec![MdInline::Text { text: "3".into() }]],
        ];
        let lines = r.render_table(&header, &rows, "", 60, Some("paragraph"), &ctx);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi_codes(l)).collect();
        // Row separator between rows.
        assert!(plain.iter().any(|l| l.contains('├')));
        assert!(plain.iter().any(|l| l.contains('1')));
        assert!(plain.iter().any(|l| l.contains('3')));
        // Spacer before the next non-space block.
        assert!(plain.last().unwrap().is_empty());
        // get_longest_word_width with no cap.
        assert_eq!(r.get_longest_word_width("a bbbb cc", None), 4);
        assert_eq!(r.get_longest_word_width("a bbbb cc", Some(2)), 2);
    }

    #[test]
    fn parser_synthetic_event_skip_arms() {
        use pulldown_cmark::CowStr;

        fn parser_with(events: Vec<(Event<'static>, std::ops::Range<usize>)>) -> MdParser<'static> {
            MdParser {
                // Padding so synthetic byte ranges stay in bounds.
                src: "                                                                ",
                line_starts: vec![0],
                events,
                pos: 0,
            }
        }
        fn text_ev(s: &str) -> (Event<'static>, std::ops::Range<usize>) {
            (
                Event::Text(CowStr::Boxed(s.to_string().into_boxed_str())),
                0..1,
            )
        }
        let rng = || 0..1;

        // parse_blocks: End that does not match the stop is skipped.
        let mut p = parser_with(vec![
            (Event::End(TagEnd::Paragraph), rng()),
            (Event::End(TagEnd::Item), rng()),
        ]);
        let (blocks, _) = p.parse_blocks(Some(TagEnd::Item), (0, 0), false, false);
        assert!(blocks.is_empty());

        // parse_blocks: an unrecognized top-level event hits `other`.
        let mut p = parser_with(vec![(
            Event::FootnoteReference(CowStr::Boxed("x".to_string().into_boxed_str())),
            rng(),
        )]);
        let (blocks, _) = p.parse_blocks(None, (0, 0), false, false);
        assert!(blocks.is_empty());

        // parse_block_start: an unexpected block tag is skipped to its End.
        let mut p = parser_with(vec![
            (
                Event::Start(Tag::FootnoteDefinition(CowStr::Boxed(
                    "x".to_string().into_boxed_str(),
                ))),
                rng(),
            ),
            text_ev("inside"),
            (Event::End(TagEnd::FootnoteDefinition), rng()),
        ]);
        let (blocks, _) = p.parse_blocks(None, (0, 0), false, false);
        assert!(blocks.is_empty());

        // Code block: stray non-text events are skipped.
        let mut p = parser_with(vec![
            (
                Event::Start(Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Fenced(
                    CowStr::Boxed("rs".to_string().into_boxed_str()),
                ))),
                rng(),
            ),
            (Event::SoftBreak, rng()),
            text_ev("code"),
            (Event::End(TagEnd::CodeBlock), rng()),
        ]);
        let (blocks, _) = p.parse_blocks(None, (0, 0), false, false);
        assert!(matches!(blocks[0], MdBlock::Code { .. }));

        // Indented code: trailing spaces/tabs are trimmed.
        let mut p = parser_with(vec![
            (
                Event::Start(Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Indented)),
                rng(),
            ),
            text_ev("code  \t\n\n"),
            (Event::End(TagEnd::CodeBlock), rng()),
        ]);
        let (blocks, _) = p.parse_blocks(None, (0, 0), false, false);
        let debug = format!("{:?}", blocks[0]);
        assert!(debug.starts_with("Code"));
        assert!(debug.contains("text: \"code\""));

        // List: stray events between items are skipped.
        let mut p = parser_with(vec![
            (Event::Start(Tag::List(None)), rng()),
            (Event::SoftBreak, rng()),
            (Event::End(TagEnd::List(false)), rng()),
        ]);
        let (blocks, _) = p.parse_blocks(None, (0, 0), false, false);
        assert!(matches!(blocks[0], MdBlock::List { .. }));

        // Table: stray events in head/row/top loops are skipped.
        let mut p = parser_with(vec![
            (Event::Start(Tag::Table(vec![])), rng()),
            (Event::Start(Tag::TableHead), rng()),
            (Event::SoftBreak, rng()),
            (Event::End(TagEnd::TableHead), rng()),
            (Event::Start(Tag::TableRow), rng()),
            (Event::Rule, rng()),
            (Event::End(TagEnd::TableRow), rng()),
            (Event::HardBreak, rng()),
            (Event::End(TagEnd::Table), rng()),
        ]);
        let (blocks, _) = p.parse_blocks(None, (0, 0), false, false);
        assert!(matches!(blocks[0], MdBlock::Table { .. }));

        // parse_inline_until: running out of events breaks; a non-matching
        // End is skipped.
        let mut p = parser_with(vec![
            (Event::Start(Tag::Emphasis), rng()),
            (Event::End(TagEnd::Paragraph), rng()),
            text_ev("x"),
        ]);
        let tokens = p.parse_inline_until(TagEnd::Emphasis);
        assert_eq!(tokens.len(), 1);

        // parse_inline_until: a nested BLOCK start is skipped to its End.
        let mut p = parser_with(vec![
            (Event::Start(Tag::Emphasis), rng()),
            (
                Event::Start(Tag::Heading {
                    level: pulldown_cmark::HeadingLevel::H2,
                    id: None,
                    classes: vec![],
                    attrs: vec![],
                }),
                rng(),
            ),
            text_ev("inner"),
            (
                Event::End(TagEnd::Heading(pulldown_cmark::HeadingLevel::H2)),
                rng(),
            ),
            text_ev("after"),
            (Event::End(TagEnd::Emphasis), rng()),
        ]);
        let tokens = p.parse_inline_until(TagEnd::Emphasis);
        assert_eq!(tokens.len(), 1); // only "after" — the heading was skipped

        // parse_inline_until: an unrecognized event is skipped in place.
        let mut p = parser_with(vec![
            (Event::Start(Tag::Emphasis), rng()),
            (Event::Rule, rng()),
            text_ev("x"),
            (Event::End(TagEnd::Emphasis), rng()),
        ]);
        let tokens = p.parse_inline_until(TagEnd::Emphasis);
        assert_eq!(tokens.len(), 1);

        // parse_bare_text_block: all inline event kinds.
        let mut p = parser_with(vec![
            (Event::TaskListMarker(false), rng()),
            (
                Event::Code(CowStr::Boxed("c".to_string().into_boxed_str())),
                rng(),
            ),
            (
                Event::InlineHtml(CowStr::Boxed("<b>".to_string().into_boxed_str())),
                rng(),
            ),
            (Event::SoftBreak, rng()),
            (Event::HardBreak, rng()),
            (Event::Start(Tag::Emphasis), rng()),
            text_ev("em"),
            (Event::End(TagEnd::Emphasis), rng()),
            (Event::Start(Tag::Strong), rng()),
            text_ev("st"),
            (Event::End(TagEnd::Strong), rng()),
            (Event::Start(Tag::Strikethrough), rng()),
            text_ev("del"),
            (Event::End(TagEnd::Strikethrough), rng()),
            (
                Event::Start(Tag::Link {
                    link_type: pulldown_cmark::LinkType::Inline,
                    dest_url: CowStr::Boxed("http://x".to_string().into_boxed_str()),
                    title: CowStr::Boxed(String::new().into_boxed_str()),
                    id: CowStr::Boxed(String::new().into_boxed_str()),
                }),
                rng(),
            ),
            text_ev("lnk"),
            (Event::End(TagEnd::Link), rng()),
            (
                Event::Start(Tag::Image {
                    link_type: pulldown_cmark::LinkType::Inline,
                    dest_url: CowStr::Boxed("i.png".to_string().into_boxed_str()),
                    title: CowStr::Boxed(String::new().into_boxed_str()),
                    id: CowStr::Boxed(String::new().into_boxed_str()),
                }),
                rng(),
            ),
            (Event::End(TagEnd::Image), rng()),
            // …ends at end-of-events (no stop).
        ]);
        let (block, _) = p.parse_bare_text_block(0..1);
        let debug = format!("{:?}", block);
        assert!(debug.starts_with("Text"));

        // parse_bare_text_block: breaks on an unhandled Start and on Rule.
        let mut p = parser_with(vec![
            text_ev("before"),
            (
                Event::Start(Tag::Heading {
                    level: pulldown_cmark::HeadingLevel::H1,
                    id: None,
                    classes: vec![],
                    attrs: vec![],
                }),
                rng(),
            ),
        ]);
        let (block, _) = p.parse_bare_text_block(0..1);
        assert!(matches!(block, MdBlock::Text { .. }));
        let mut p = parser_with(vec![text_ev("before"), (Event::Rule, rng())]);
        let (block, _) = p.parse_bare_text_block(0..1);
        assert!(matches!(block, MdBlock::Text { .. }));

        // is_blank_line with blockquote markers.
        let p = MdParser::new("> \n");
        assert!(p.is_blank_line(0));
        let p = MdParser::new("  > quoted\n");
        assert!(!p.is_blank_line(0));
    }

    #[test]
    fn render_inline_strips_trailing_style_prefix() {
        use crate::theme::fg_raw;
        let style = DefaultTextStyle {
            color: Some(std::rc::Rc::new(|s: &str| fg_raw(99, s))),
            bg_color: None,
            bold: false,
            italic: false,
            strikethrough: false,
            underline: false,
        };
        let mut r =
            MarkdownRenderer::with_theme_and_style(MarkdownThemePartial::default(), Some(style));
        // An image-only inline run renders just the style prefix, which is
        // then stripped back off.
        let ctx = bare_ctx(&mut r);
        let out = r.render_inline_tokens(&[MdInline::Image], &ctx);
        assert_eq!(out, "");
    }

    #[test]
    fn component_invalidate_override_and_blockquote_trailing_pop() {
        let mut r = MarkdownRenderer::new();
        r.set_text("x");
        let _ = r.render(40);
        Component::invalidate(&mut r);
        assert!(r.cached_lines.is_none());

        // blockquote_raw_text drops trailing empty lines.
        let blocks = vec![
            MdBlock::Paragraph {
                inline: vec![MdInline::Text {
                    text: "real".into(),
                }],
            },
            MdBlock::Paragraph { inline: vec![] },
        ];
        assert_eq!(blockquote_raw_text(&blocks), "real");
    }

    #[test]
    fn parser_white_box_edges() {
        let p = MdParser::new("plain text");
        // find_container_end at a non-Start event → 0.
        let mut p2 = MdParser::new("plain text");
        // pos 1 is the Text event inside the paragraph.
        p2.pos = 1;
        assert_eq!(p2.find_container_end(TagEnd::Paragraph), 0);
        // …and an unmatched stop scans to the end → 0.
        let mut p3 = MdParser::new("plain text");
        p3.pos = 0;
        assert_eq!(p3.find_container_end(TagEnd::CodeBlock), 0);
        // parse_block_start at a non-Start event → None.
        assert!(matches!(p2.events[p2.pos].0, Event::Text(_)));
        assert!(p2.parse_block_start().is_none());
        drop(p);
    }

    #[test]
    fn parser_skip_arms_via_exotic_inputs() {
        // Exercise the skip/`other` arms; assertions are loose — the point
        // is driving the parser paths without panicking.
        let cases = [
            "- a\n\n>\n",                         // blank-ish blockquote marker after a list
            "- [ ] task\n\n- [x] done\n\npara\n", // loose task list
            "<div>\nblock\n</div>\n\npara\n",     // html block
            "text\n<span>inline</span>\nmore\n",  // inline html mid-paragraph
            "<!-- comment -->\n\npara\n",
            "##\n",                        // empty heading
            "```\n\n```\n",                // empty fenced code
            "    indented\n\n\t tabbed\n", // indented code with tab line
        ];
        for md in cases {
            let mut r = MarkdownRenderer::new();
            let _ = r.render_text(md, 60);
        }
    }

    #[test]
    fn default_style_prefix_cache_and_sentinel_dropping_style() {
        use crate::theme::fg_raw;
        let style = DefaultTextStyle {
            color: Some(std::rc::Rc::new(|s: &str| fg_raw(99, s))),
            bg_color: None,
            bold: false,
            italic: false,
            strikethrough: false,
            underline: false,
        };
        let mut r =
            MarkdownRenderer::with_theme_and_style(MarkdownThemePartial::default(), Some(style));
        // Two renders: the second hits the cached style prefix.
        let _ = r.render_text("<b>hi</b>", 40);
        let _ = r.render_text("<b>hi again</b>", 40);

        // A color fn that DROPS the sentinel → empty prefix (find → None).
        let style = DefaultTextStyle {
            color: Some(std::rc::Rc::new(|_s: &str| "CONSTANT".to_string())),
            bg_color: None,
            bold: true,
            italic: false,
            strikethrough: false,
            underline: false,
        };
        let mut r =
            MarkdownRenderer::with_theme_and_style(MarkdownThemePartial::default(), Some(style));
        let lines = r.render_text("<u>html</u>", 40);
        assert!(!lines.is_empty());
    }

    #[test]
    fn task_list_markers_in_tight_and_loose_lists() {
        let plain = render_plain("- [ ] open\n- [x] closed\n");
        assert!(plain.join("").contains("open"));
        let plain = render_plain("- [ ] open\n\n- [x] closed\n");
        assert!(plain.join("").contains("closed"));
    }

    #[test]
    fn default_style_applies_to_html_blocks() {
        use crate::theme::fg_raw;
        let style = DefaultTextStyle {
            color: Some(std::rc::Rc::new(|s: &str| fg_raw(99, s))),
            bg_color: None,
            bold: true,
            italic: true,
            strikethrough: true,
            underline: true,
        };
        let mut r =
            MarkdownRenderer::with_theme_and_style(MarkdownThemePartial::default(), Some(style));
        let lines = r.render_text("<div>\nblock html\n</div>", 60);
        let joined = lines.join("\n");
        assert!(joined.contains("38;5;99"));
        assert!(strip_ansi_codes(&joined).contains("block html"));
    }

    #[test]
    fn get_style_prefix_none_when_sentinel_dropped() {
        // A heading style that drops the sentinel yields an empty prefix.
        let mut r = MarkdownRenderer::with_theme(MarkdownThemePartial {
            heading: Some(std::rc::Rc::new(|_s: &str| "CONSTANT".to_string())),
            ..Default::default()
        });
        let lines = r.render_text("# title", 60);
        // The style fn swallows the text — the output is its constant.
        assert!(strip_ansi_codes(&lines.join("\n")).contains("CONSTANT"));
    }

    #[test]
    fn blockquote_spacer_before_next_block() {
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        let quote = MdBlock::Blockquote {
            blocks: vec![MdBlock::Paragraph {
                inline: vec![MdInline::Text { text: "q".into() }],
            }],
        };
        let with_next = r.render_block(&quote, 60, Some("paragraph"), &ctx);
        let ctx = bare_ctx(&mut r);
        let without = r.render_block(&quote, 60, None, &ctx);
        assert_eq!(with_next.len(), without.len() + 1);
    }

    #[test]
    fn render_list_with_nested_list_first_line() {
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        // An item whose first content block is itself a list → the first
        // line is pushed without a bullet prefix.
        let items = vec![vec![
            MdBlock::List {
                ordered: false,
                start: None,
                items: vec![vec![MdBlock::Text {
                    inline: vec![MdInline::Text {
                        text: "deep".into(),
                    }],
                }]],
            },
            MdBlock::Text {
                inline: vec![MdInline::Text {
                    text: "tail".into(),
                }],
            },
        ]];
        let lines = r.render_list(false, None, &items, 0, &ctx);
        let plain: Vec<String> = lines.iter().map(|l| strip_ansi_codes(l)).collect();
        assert!(plain.iter().any(|l| l.contains("deep")));
        assert!(plain.iter().any(|l| l.contains("tail")));
    }

    #[test]
    fn highlight_code_inside_list_item() {
        let mut r = MarkdownRenderer::with_theme(MarkdownThemePartial {
            highlight_code: Some(std::rc::Rc::new(|text: &str, _lang| {
                vec![format!("HL {text}")]
            })),
            ..Default::default()
        });
        let ctx = bare_ctx(&mut r);
        let item = vec![MdBlock::Code {
            text: "fn x()".into(),
            lang: "rs".into(),
        }];
        let lines = r.render_list_item(&item, 0, &ctx);
        assert!(strip_ansi_codes(&lines.join("\n")).contains("HL fn x()"));
    }

    #[test]
    fn table_narrow_fallback_with_empty_raw() {
        let mut r = MarkdownRenderer::new();
        let ctx = bare_ctx(&mut r);
        let header = vec![vec![MdInline::Text { text: "A".into() }]];
        let rows = vec![vec![vec![MdInline::Text { text: "1".into() }]]];
        // Empty raw + narrow → empty fallback (no spacer without a next).
        assert!(r.render_table(&header, &rows, "", 2, None, &ctx).is_empty());
    }

    #[test]
    fn table_width_distribution_edges() {
        let mut r = MarkdownRenderer::new();
        // Shrink with zero remaining (avail == num_cols after borders).
        let header = vec![
            vec![MdInline::Text { text: "aaa".into() }],
            vec![MdInline::Text { text: "bbb".into() }],
        ];
        let rows = vec![vec![
            vec![MdInline::Text { text: "ccc".into() }],
            vec![MdInline::Text { text: "ddd".into() }],
        ]];
        let ctx = bare_ctx(&mut r);
        let lines = r.render_table(&header, &rows, "", 9, None, &ctx);
        assert!(!lines.is_empty());

        // Grow with zero total_grow (all cells single words at min width).
        let header = vec![
            vec![MdInline::Text { text: "aa".into() }],
            vec![MdInline::Text { text: "b".into() }],
        ];
        let rows = vec![vec![
            vec![MdInline::Text { text: "cc".into() }],
            vec![MdInline::Text { text: "d".into() }],
        ]];
        let ctx = bare_ctx(&mut r);
        let lines = r.render_table(&header, &rows, "", 9, None, &ctx);
        assert!(!lines.is_empty());

        // Remainder distribution: cols grown below natural with spare room.
        let header = vec![
            vec![MdInline::Text {
                text: "aa bb".into(),
            }],
            vec![MdInline::Text {
                text: "cc dd".into(),
            }],
        ];
        let rows = vec![vec![
            vec![MdInline::Text {
                text: "ee ff".into(),
            }],
            vec![MdInline::Text {
                text: "gg hh".into(),
            }],
        ]];
        let ctx = bare_ctx(&mut r);
        let lines = r.render_table(&header, &rows, "", 14, None, &ctx);
        assert!(!lines.is_empty());
    }

    #[test]
    fn synthetic_html_block_event() {
        use pulldown_cmark::CowStr;
        let mut p = MdParser {
            src: "                                                                ",
            line_starts: vec![0],
            events: vec![(
                Event::Html(CowStr::Boxed("<div>raw</div>".to_string().into_boxed_str())),
                0..1,
            )],
            pos: 0,
        };
        let (blocks, _) = p.parse_blocks(None, (0, 0), false, false);
        assert!(matches!(&blocks[0], MdBlock::Html { raw } if raw.contains("raw")));
    }

    #[test]
    fn trailing_prefix_stripped_after_styled_token() {
        use crate::theme::fg_raw;
        let style = DefaultTextStyle {
            color: Some(std::rc::Rc::new(|s: &str| fg_raw(99, s))),
            bg_color: None,
            bold: false,
            italic: false,
            strikethrough: false,
            underline: false,
        };
        let mut r =
            MarkdownRenderer::with_theme_and_style(MarkdownThemePartial::default(), Some(style));
        // A run ending in Strong pushes the prefix last → stripped.
        let ctx = bare_ctx(&mut r);
        let out = r.render_inline_tokens(
            &[MdInline::Strong {
                inline: vec![MdInline::Text { text: "b".into() }],
            }],
            &ctx,
        );
        assert!(!out.ends_with(&ctx.style_prefix));
        assert!(strip_ansi_codes(&out).contains('b'));
    }
}
