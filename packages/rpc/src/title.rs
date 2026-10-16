//! The title a conversation is shown by when nothing named it.
//!
//! An unnamed session falls back to its first user message: the Agent writes it
//! into `session_info.session_name`, the Desktop derives a thread title from it,
//! and the TUI lists it. That message is markdown *source*, and a title is not a
//! place to draw markdown. A composer writes a reference to another
//! conversation as `[手机截图镜像修复](futureos://session/<id>)`, so a message
//! that begins with one produced a title reading
//! `[手机截图镜像修复](futureos://sessio…` — brackets, a URL and no topic.
//!
//! The rule lives here — in the crate the Agent and every client share — because
//! one message must not produce a different title on the phone than on the
//! desktop.

/// The URL prefix that marks a link as a reference to another conversation.
const SESSION_REFERENCE_URL: &str = "futureos://session/";

/// Plain text of a user message, for a title or list preview derived from it.
///
/// Two transformations, in order:
///
/// 1. A reference to another conversation is *addressing*, not prose, so it
///    contributes nothing. Its label is the other conversation's title, and
///    keeping it named this conversation after that one — two identical rows in
///    one list, where the reader cannot tell which one they are looking at.
/// 2. Every other link contributes its label and an image its alt text, so the
///    title reads as the sentence that was written.
///
/// A message that is *only* a reference keeps the label after all: naming a
/// conversation after the one it points at beats naming it `[`. Whitespace is
/// collapsed, because a title is one line whatever the message's line breaks
/// were and a dropped link must not leave the spaces that framed it behind.
pub fn plain_title_text(text: &str) -> String {
    let without_references = render(text, true);
    if !without_references.is_empty() {
        return without_references;
    }
    // Nothing but a reference (or nothing at all): the labels are all we have.
    render(text, false)
}

/// `drop_references` decides what a conversation reference contributes: its
/// label, or nothing.
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
    rendered.split_whitespace().collect::<Vec<_>>().join(" ")
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
        // Brackets without a target, an array index, an emphasis pair — none of
        // these are links, and a title must not eat them.
        assert_eq!(plain_title_text("[pending] 待办"), "[pending] 待办");
        assert_eq!(plain_title_text("items[0] = 3"), "items[0] = 3");
        assert_eq!(plain_title_text("**加粗** 一下"), "**加粗** 一下");
        assert_eq!(plain_title_text("a_b_c 命名"), "a_b_c 命名");
        // An unclosed link is not a link either.
        assert_eq!(
            plain_title_text("[标题](futureos://session/s-3"),
            "[标题](futureos://session/s-3"
        );
    }

    #[test]
    fn whitespace_collapses_to_one_line() {
        assert_eq!(plain_title_text("  第一行\n\n第二行  "), "第一行 第二行");
        assert_eq!(plain_title_text("   "), "");
        assert_eq!(plain_title_text(""), "");
    }
}
