//! The title a conversation is shown by when nothing named it.
//!
//! An unnamed session falls back to its first user message: the Agent writes it
//! into `session_info.session_name`, the Desktop derives a thread title from it,
//! and the TUI lists it. That message is markdown *source*, and a title is not a
//! place to draw markdown. A composer writes a reference to another
//! conversation as `[手机截图镜像修复](futureos://session/<id>)`, so a message
//! that begins with one produced a title reading
//! `[手机截图镜像修复](futureos://sessio…` — brackets, a URL and no topic. An
//! image pasted into the TUI names its attachment `[Image #1]`, and a message
//! that was two screenshots named its conversation `[Image #1][Image #2]`.
//!
//! The rule lives here — in the crate the Agent and every client share — because
//! one message must not produce a different title on the phone than on the
//! desktop.

/// The URL prefix that marks a link as a reference to another conversation.
const SESSION_REFERENCE_URL: &str = "futureos://session/";

/// Plain text of a user message, for a title or list preview derived from it.
///
/// Three transformations, in order:
///
/// 1. A reference to another conversation is *addressing*, not prose, so it
///    contributes nothing. Its label is the other conversation's title, and
///    keeping it named this conversation after that one — two identical rows in
///    one list, where the reader cannot tell which one they are looking at.
/// 2. Every other link contributes its label and an image its alt text, so the
///    title reads as the sentence that was written.
/// 3. Markdown syntax and composer placeholders are removed and the text that
///    is left is kept — see [`flatten_markup`].
///
/// A message that is *only* a reference keeps the label after all: naming a
/// conversation after the one it points at beats naming it `[`.
pub fn plain_title_text(text: &str) -> String {
    let without_references = flatten_markup(&render(text, true));
    if !without_references.is_empty() {
        return without_references;
    }
    // Nothing but a reference (or nothing at all): the labels are all we have.
    flatten_markup(&render(text, false))
}

/// Markdown source read out as plain text: the words, none of the syntax.
///
/// A title is drawn as text, so nothing that only exists to be rendered may
/// survive in it. That covers two kinds of leftovers:
///
/// - **Composer placeholders**, which name nothing a reader can use: the TUI
///   writes `[Image #1]` where a pasted path became an attachment, and folds a
///   long paste into `[Pasted Content 2000 chars]`. A message that was one
///   screenshot used to name its conversation `[Image #1][Image #2]`.
/// - **Inline and block syntax**: emphasis, strong, strikethrough, code spans
///   and fences, headings, block quotes, list bullets and ordered-list numbers.
///   The text inside them stays; only the marks go, so `**加粗** 一下` reads
///   `加粗 一下`.
///
/// Delimiters are dropped only when they pair up, because a title may not eat
/// text that merely contains one: `2 * 3` and `a_b_c` keep their characters,
/// and an unclosed `**` stays as written. Whitespace is collapsed, because a
/// title is one line whatever the message's line breaks were.
pub fn flatten_markup(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let characters = without_placeholders(&characters);
    let characters = without_block_markers(&characters);
    let text: String = without_delimiters(&characters).into_iter().collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Byte length of a composer placeholder at the start of `text`.
///
/// Recognised exactly as the composers write them — `[Image #<n>]`,
/// `[Pasted Content <n> chars]` and `[Pasted Content <n> chars #<n>]` — so
/// ordinary bracketed prose (`[pending] 待办`) is never mistaken for one.
pub fn placeholder_len(text: &str) -> Option<usize> {
    let body = text.strip_prefix('[')?;
    if let Some(after) = body.strip_prefix("Image #") {
        let digits = digit_run(after);
        if digits > 0 {
            if let Some(tail) = after[digits..].strip_prefix(']') {
                return Some(text.len() - tail.len());
            }
        }
    }
    let after = body.strip_prefix("Pasted Content ")?;
    let digits = digit_run(after);
    if digits == 0 {
        return None;
    }
    let after = after[digits..].strip_prefix(" chars")?;
    if let Some(tail) = after.strip_prefix(']') {
        return Some(text.len() - tail.len());
    }
    let after = after.strip_prefix(" #")?;
    let digits = digit_run(after);
    if digits == 0 {
        return None;
    }
    let tail = after[digits..].strip_prefix(']')?;
    Some(text.len() - tail.len())
}

fn digit_run(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

/// Drop every composer placeholder, leaving the prose around it.
fn without_placeholders(characters: &[char]) -> Vec<char> {
    let mut kept = Vec::with_capacity(characters.len());
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '[' {
            let probe: String = characters[index..].iter().take(48).collect::<String>();
            if let Some(length) = placeholder_len(&probe) {
                // A placeholder of the same text is as long in characters as in
                // bytes here, every part of it being ASCII.
                index += length;
                continue;
            }
        }
        kept.push(characters[index]);
        index += 1;
    }
    kept
}

/// Drop the marks that only exist to open a block, keeping each line's text.
fn without_block_markers(characters: &[char]) -> Vec<char> {
    let mut kept = Vec::with_capacity(characters.len());
    let mut line_start = 0;
    let mut index = 0;
    while index <= characters.len() {
        let line_ends = index == characters.len() || characters[index] == '\n';
        if !line_ends {
            index += 1;
            continue;
        }
        let line = &characters[line_start..index];
        if !is_rule_or_fence(line) {
            let skip = block_marker_len(line);
            kept.extend_from_slice(&line[skip..]);
            if index < characters.len() {
                kept.push('\n');
            }
        }
        index += 1;
        line_start = index;
    }
    kept
}

/// A line that carries no text of its own: a horizontal rule, a setext
/// underline or a code fence.
fn is_rule_or_fence(line: &[char]) -> bool {
    let trimmed = trim(line);
    if trimmed.is_empty() {
        return false;
    }
    let fences = trimmed
        .iter()
        .take_while(|ch| **ch == '`' || **ch == '~')
        .count();
    if fences >= 3 {
        return trimmed[..fences].iter().all(|ch| *ch == trimmed[0]);
    }
    let marks = trimmed
        .iter()
        .filter(|ch| !ch.is_whitespace())
        .copied()
        .collect::<Vec<_>>();
    marks.len() >= 3
        && marks.iter().all(|ch| *ch == marks[0])
        && matches!(marks[0], '-' | '*' | '_' | '=')
}

/// How many characters of `line` only open a block and are not text.
fn block_marker_len(line: &[char]) -> usize {
    let indent = line.iter().take_while(|ch| ch.is_whitespace()).count();
    let rest = &line[indent..];
    let mark = |length: usize| indent + length;
    if let Some(heading) = heading_len(rest) {
        return mark(heading);
    }
    if rest.first() == Some(&'>') {
        let mut quote = 0;
        while rest.get(quote) == Some(&'>') {
            quote += 1;
            if rest.get(quote) == Some(&' ') {
                quote += 1;
            }
        }
        return mark(quote);
    }
    if matches!(rest.first(), Some('-' | '*' | '+')) && rest.get(1) == Some(&' ') {
        return mark(2);
    }
    let digits = rest.iter().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0
        && matches!(rest.get(digits), Some('.' | ')'))
        && rest.get(digits + 1) == Some(&' ')
    {
        return mark(digits + 2);
    }
    0
}

fn heading_len(line: &[char]) -> Option<usize> {
    let hashes = line.iter().take_while(|ch| **ch == '#').count();
    if (1..=6).contains(&hashes) && line.get(hashes).is_none_or(|ch| *ch == ' ') {
        return Some(if line.get(hashes) == Some(&' ') {
            hashes + 1
        } else {
            hashes
        });
    }
    None
}

fn trim(line: &[char]) -> &[char] {
    let start = line.iter().take_while(|ch| ch.is_whitespace()).count();
    let end = line.len()
        - line
            .iter()
            .rev()
            .take_while(|ch| ch.is_whitespace())
            .count();
    &line[start..end.max(start)]
}

/// Drop emphasis, strong, strikethrough and code delimiters that pair up.
///
/// Only paired delimiters go: `**b**` keeps `b`, while `2 * 3` and `a_b_c`
/// keep their characters, and an unclosed `**` stays as it was written.
fn without_delimiters(characters: &[char]) -> Vec<char> {
    let mut dropped = vec![false; characters.len()];
    let mut index = 0;
    while index < characters.len() {
        let ch = characters[index];
        if !matches!(ch, '*' | '_' | '~' | '`') {
            index += 1;
            continue;
        }
        let run = run_length(characters, index);
        if opens_emphasis(characters, index, run) {
            if let Some(close) = closing_delimiter(characters, index + run, ch, run) {
                let close_run = run_length(characters, close);
                dropped[index..index + run].fill(true);
                dropped[close..close + close_run].fill(true);
                index += run;
                continue;
            }
        }
        index += run;
    }
    characters
        .iter()
        .zip(dropped)
        .filter_map(|(ch, dropped)| (!dropped).then_some(*ch))
        .collect()
}

fn run_length(characters: &[char], from: usize) -> usize {
    characters[from..]
        .iter()
        .take_while(|ch| **ch == characters[from])
        .count()
}

fn opens_emphasis(characters: &[char], from: usize, run: usize) -> bool {
    let ch = characters[from];
    // `_` inside a word is a name (`a_b_c`), not emphasis; for every delimiter
    // the mark has to be followed by the text it marks.
    let intraword = ch == '_' && from > 0 && characters[from - 1].is_alphanumeric();
    !intraword
        && characters
            .get(from + run)
            .is_some_and(|next| !next.is_whitespace())
}

fn closing_delimiter(characters: &[char], from: usize, ch: char, run: usize) -> Option<usize> {
    let mut index = from;
    while index < characters.len() {
        if characters[index] != ch {
            index += 1;
            continue;
        }
        let length = run_length(characters, index);
        let closes = index > 0 && !characters[index - 1].is_whitespace();
        let intraword = ch == '_'
            && characters
                .get(index + length)
                .is_some_and(|next| next.is_alphanumeric());
        if closes && !intraword && length >= run {
            return Some(index);
        }
        index += length;
    }
    None
}

/// `drop_references` decides what a conversation reference contributes: its
/// label, or nothing. Line structure survives — a link never spans a line, and
/// [`flatten_markup`] still needs the message's lines to tell a heading or a
/// bullet from the middle of a sentence.
fn render(text: &str, drop_references: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut rendered = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        match inline_link(&chars, index) {
            Some((end, label, target)) => {
                if !(drop_references && is_conversation_reference(&target)) {
                    rendered.push_str(&label);
                }
                index = end;
            }
            None => {
                rendered.push(chars[index]);
                index += 1;
            }
        }
    }
    rendered
}

/// Whether a link target addresses another conversation in this app.
fn is_conversation_reference(target: &str) -> bool {
    // Both spellings of a target are in the wild: `[a](x)` and `[a](<x>)`.
    let target = target.trim();
    let bare = target
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(target);
    bare.starts_with(SESSION_REFERENCE_URL)
}

/// An inline markdown image (`![alt](target)`) or link (`[label](target)`)
/// starting at `from`: the index just past it, plus its label and its target.
///
/// `None` when no *complete* link starts there — text that merely contains
/// brackets is left exactly as it was, because a title may not silently eat it.
fn inline_link(chars: &[char], from: usize) -> Option<(usize, String, String)> {
    let mut start = from;
    if chars.get(start) == Some(&'!') {
        start += 1;
    }
    if chars.get(start) != Some(&'[') {
        return None;
    }
    let label_start = start + 1;
    let label_end = label_start + chars[label_start..].iter().position(|c| *c == ']')?;
    if chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    // Targets can carry parentheses of their own (`[x](file(1).md)`), so the
    // link ends at the `)` that closes the one that opened it.
    let target_start = label_end + 2;
    let mut depth = 1usize;
    let mut end = target_start;
    while end < chars.len() {
        match chars[end] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        end += 1;
    }
    if depth != 0 {
        return None; // unterminated — not a link
    }
    let label: String = chars[label_start..label_end].iter().collect();
    let target: String = chars[target_start..end].iter().collect();
    Some((end + 1, label, target))
}

#[cfg(test)]
mod tests {
    use super::plain_title_text;

    #[test]
    fn a_reference_is_dropped_and_the_question_stays() {
        assert_eq!(
            plain_title_text(
                "[手机截图镜像修复](futureos://session/20261016-142101-2a944dead697482d8a6adad9942fe403) 这个会话在流式回复中"
            ),
            "这个会话在流式回复中"
        );
    }

    #[test]
    fn a_lone_reference_is_named_after_the_conversation_it_points_at() {
        assert_eq!(
            plain_title_text("[手机截图镜像修复](futureos://session/s-1)"),
            "手机截图镜像修复"
        );
    }

    #[test]
    fn a_reference_inside_a_sentence_leaves_no_gap() {
        assert_eq!(
            plain_title_text("先看 [登录修复](futureos://session/s-9) 再说"),
            "先看 再说"
        );
    }

    #[test]
    fn other_links_keep_their_label_and_images_their_alt_text() {
        assert_eq!(
            plain_title_text("修一下 [这一行](https://example.com/x)"),
            "修一下 这一行"
        );
        assert_eq!(plain_title_text("![截图](./a.png) 崩了"), "截图 崩了");
    }

    #[test]
    fn a_target_may_be_angle_wrapped_or_contain_parentheses() {
        assert_eq!(
            plain_title_text("[看这个](<futureos://session/s-2>)"),
            "看这个"
        );
        assert_eq!(plain_title_text("[手册](docs(1).md) 怎么看"), "手册 怎么看");
    }

    #[test]
    fn text_that_is_not_a_link_is_untouched() {
        // Brackets without a target, an array index — neither is a link, and a
        // title must not eat them.
        assert_eq!(plain_title_text("[pending] 待办"), "[pending] 待办");
        assert_eq!(plain_title_text("items[0] = 3"), "items[0] = 3");
        // An unclosed link is not a link either.
        assert_eq!(
            plain_title_text("[标题](futureos://session/s-3"),
            "[标题](futureos://session/s-3"
        );
    }

    /// The reported shape: a TUI message whose text starts with the marker of a
    /// pasted screenshot named its session `[Image #1]现在 tui 中…`.
    #[test]
    fn a_composer_placeholder_names_nothing_and_is_dropped() {
        assert_eq!(
            plain_title_text("[Image #1]现在 tui 中显示展示思考中"),
            "现在 tui 中显示展示思考中"
        );
        assert_eq!(plain_title_text("[Image #1][Image #2]"), "");
        assert_eq!(
            plain_title_text("看 [Image #3] 这张图里的报错"),
            "看 这张图里的报错"
        );
        // Folded pastes are placeholders too.
        assert_eq!(
            plain_title_text("[Pasted Content 2000 chars] 帮我看看"),
            "帮我看看"
        );
        assert_eq!(
            plain_title_text("[Pasted Content 2000 chars #2] 帮我看看"),
            "帮我看看"
        );
        // Only the exact forms the composers write are placeholders.
        assert_eq!(plain_title_text("[Image #] 说明"), "[Image #] 说明");
        assert_eq!(plain_title_text("[image #1] 说明"), "[image #1] 说明");
    }

    #[test]
    fn markdown_syntax_is_flattened_and_its_text_kept() {
        assert_eq!(plain_title_text("**加粗** 一下"), "加粗 一下");
        assert_eq!(plain_title_text("__加粗__ 一下"), "加粗 一下");
        assert_eq!(plain_title_text("*斜体* 与 ~~删除线~~"), "斜体 与 删除线");
        assert_eq!(
            plain_title_text("看看 `cargo test` 的输出"),
            "看看 cargo test 的输出"
        );
        assert_eq!(plain_title_text("# 标题\n正文"), "标题 正文");
        assert_eq!(plain_title_text("### 修复登录"), "修复登录");
        assert_eq!(plain_title_text("> 引用的一句话"), "引用的一句话");
        assert_eq!(plain_title_text("- 第一条\n- 第二条"), "第一条 第二条");
        assert_eq!(plain_title_text("1. 第一步\n2) 第二步"), "第一步 第二步");
        assert_eq!(plain_title_text("---\n正文"), "正文");
        assert_eq!(
            plain_title_text("```rust\nfn main() {}\n```"),
            "fn main() {}"
        );
    }

    /// A delimiter the message never closes, and one that is part of a word or
    /// an expression, are text: dropping them would change what was written.
    #[test]
    fn unpaired_or_intraword_delimiters_are_text() {
        assert_eq!(plain_title_text("a_b_c 命名"), "a_b_c 命名");
        assert_eq!(plain_title_text("2 * 3 = 6"), "2 * 3 = 6");
        assert_eq!(plain_title_text("未闭合的 **加粗"), "未闭合的 **加粗");
        assert_eq!(plain_title_text("C# 与 ~/future-os"), "C# 与 ~/future-os");
    }

    #[test]
    fn whitespace_collapses_to_one_line() {
        assert_eq!(plain_title_text("  第一行\n\n第二行  "), "第一行 第二行");
        assert_eq!(plain_title_text("   "), "");
        assert_eq!(plain_title_text(""), "");
    }
}
