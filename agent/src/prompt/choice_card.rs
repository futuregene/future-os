//! Whether a reply ends by offering the reader a choice.
//!
//! The card itself is rendered by the clients (`packages/markdown` parses the
//! fence, each surface draws it), but "this conversation's last finished reply
//! ended with a question" is a fact about the *run*, and the run is where it has
//! to be recorded: a rail row, a phone's session list and a peer's catalogue all
//! badge such a conversation without opening it, and none of them holds the
//! other conversations' text.
//!
//! So this is the Agent's reading of its own reply, applied once at run commit
//! and carried on `agent_end`. The rules are the renderer's
//! (`parseChoiceCard` in `packages/markdown/src/parseFutureMarkdown.ts`): a
//! closing `futureos-choice` fence with 2–6 option rows, each an id in
//! `[a-z0-9-]` (≤ 24 characters) and a non-empty label. A block that fails them
//! is drawn as the code block it is, so a reply carrying one must not badge a
//! conversation as "needs your answer". The bounds are the renderer's, not the
//! prompt's: the prompt asks for 2–4, the client draws up to six so a writer that
//! slipped by a row still gets a card (see `CHOICE_MAX_OPTIONS`).
//!
//! Kept small on purpose: the reply text is already in memory at commit time and
//! a wrong `false` only omits a badge, so this reads the tail of the message and
//! stops, rather than re-implementing a Markdown parser.

/// Fence language of a choice card; every other fence keeps its old meaning.
const CHOICE_FENCE: &str = "futureos-choice";
/// §3: fewer than two rows is not a card, and seven rows are a plan. The upper
/// bound matches `CHOICE_MAX_OPTIONS` in `packages/markdown/src/parseFutureMarkdown.ts`:
/// the badge may only promise a card the client will actually draw.
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 6;
/// §3: an option id is ASCII `[a-z0-9-]`, at most 24 characters.
const MAX_OPTION_ID: usize = 24;

/// Does `text` close with a `futureos-choice` card?
///
/// True only when the *last* fence in the reply is a choice card and nothing but
/// whitespace follows it — the design's "the card closes the reply, alone"
/// (`docs/design/choice-cards.zh-CN.md` §3). A card in the middle of a reply
/// belongs to a question that has already been answered, and a conversation
/// whose last reply merely *quotes* a card (inside a fenced document) must not
/// read as one either.
pub fn reply_offers_choice(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let mut open: Option<Fence> = None;
    // The closing line index of the last fence that ended a choice card, or
    // `None` once a later fence ended as something else.
    let mut card_end: Option<usize> = None;

    for (index, line) in lines.iter().enumerate() {
        match &open {
            None => {
                if let Some(fence) = read_fence(line, index) {
                    open = Some(fence);
                }
            }
            Some(fence) => {
                if closes(line, fence) {
                    let is_choice = fence.info.eq_ignore_ascii_case(CHOICE_FENCE);
                    card_end = (is_choice
                        && card_body_offers_choice(&lines[fence.body_start..index]))
                    .then_some(index);
                    open = None;
                }
            }
        }
    }

    // Only whitespace may follow the closing fence: the card is the reply's last
    // word, and a card quoted mid-reply is not a question to answer.
    card_end.is_some_and(|end| lines[end + 1..].iter().all(|line| line.trim().is_empty()))
}

/// One fence delimiter, with the line its body starts on.
struct Fence {
    ch: char,
    len: usize,
    info: String,
    body_start: usize,
}

/// ` ```lang ` or `~~~lang`, up to three spaces in. Follows CommonMark closely
/// enough for a closing fence: a backtick fence's info string may not contain a
/// backtick, so `` `a `b` `` is inline code rather than a fence.
fn read_fence(line: &str, index: usize) -> Option<Fence> {
    let (start, ch, run) = fence_run(line)?;
    let info = line[start + run..].trim().to_string();
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some(Fence {
        ch,
        len: run,
        info,
        body_start: index + 1,
    })
}

/// The `(start byte, delimiter, run length)` of a fence line, or `None`.
fn fence_run(line: &str) -> Option<(usize, char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = rest.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let run = rest
        .chars()
        .take_while(|candidate| *candidate == ch)
        .count();
    if run < 3 {
        return None;
    }
    Some((indent, ch, run))
}

/// Whether `line` ends the fence `open` — the same delimiter, at least as long,
/// and nothing after it.
fn closes(line: &str, open: &Fence) -> bool {
    fence_run(line).is_some_and(|(index, ch, run)| {
        ch == open.ch && run >= open.len && line[index + run..].trim().is_empty()
    })
}

/// The card's own field rules, applied to a fence's body.
///
/// A `key: value` line is a field (the renderer's `parseDirectiveEntries`); the
/// question may be inferred from the first option, so only the option rows are
/// checked. Rows that lose their id or label are dropped, exactly as the renderer
/// drops them, and the card survives only with two of them left.
fn card_body_offers_choice(body: &[&str]) -> bool {
    let options: Vec<&str> = body
        .iter()
        .filter_map(|line| directive_entry(line))
        .filter(|(key, _)| *key == "option")
        .map(|(_, value)| value)
        .collect();
    if options.len() < MIN_OPTIONS || options.len() > MAX_OPTIONS {
        return false;
    }

    let mut seen: Vec<&str> = Vec::new();
    for raw in options {
        let mut parts = raw.split('|');
        let id = parts.next().unwrap_or("").trim();
        let label = parts.next().unwrap_or("").trim();
        if !valid_option_id(id) || label.is_empty() || seen.contains(&id) {
            continue;
        }
        seen.push(id);
    }
    seen.len() >= MIN_OPTIONS
}

/// `key: value`, with the key non-empty (the renderer skips a line whose colon
/// leads it, and a line with no colon at all).
fn directive_entry(line: &str) -> Option<(&str, &str)> {
    let separator = line.find(':')?;
    if separator == 0 {
        return None;
    }
    let key = line[..separator].trim();
    if key.is_empty() {
        return None;
    }
    Some((key, line[separator + 1..].trim()))
}

fn valid_option_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_OPTION_ID
        && id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::{reply_offers_choice, CHOICE_FENCE};

    fn card(body: &str) -> String {
        format!("```{CHOICE_FENCE}\n{body}\n```\n")
    }

    const TWO: &str =
        "title: 接下来做哪一件？\noption: forest | 重画森林图\noption: flow | 补一张实验流程图";

    #[test]
    fn a_reply_closing_with_a_card_offers_a_choice() {
        assert!(reply_offers_choice(&format!(
            "Here is the report.\n\n{}",
            card(TWO)
        )));
        // Trailing blank lines do not move the card off the end.
        assert!(reply_offers_choice(&format!("{}\n\n", card(TWO))));
    }

    #[test]
    fn a_card_that_is_not_last_does_not() {
        let text = format!("{}\n\nAnything else I can help with?", card(TWO));
        assert!(!reply_offers_choice(&text));
    }

    #[test]
    fn six_rows_are_a_card_and_seven_are_a_plan() {
        let rows = |count: usize| {
            (0..count)
                .map(|index| format!("option: id{index} | label {index}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(reply_offers_choice(&card(&rows(2))));
        assert!(reply_offers_choice(&card(&rows(4))));
        // A one-row slip still draws as a card on every client (2–6 rows).
        assert!(reply_offers_choice(&card(&rows(5))));
        assert!(reply_offers_choice(&card(&rows(6))));
        assert!(!reply_offers_choice(&card(&rows(7))));
        assert!(!reply_offers_choice(&card(&rows(1))));
    }

    #[test]
    fn rows_without_a_usable_id_are_dropped_like_the_renderer_drops_them() {
        // Two usable rows survive one bad one.
        assert!(reply_offers_choice(&card(&format!(
            "{TWO}\noption: Forest | no id charset"
        ))));
        // Only one usable row is left: not a card.
        assert!(!reply_offers_choice(&card(
            "title: T\noption: forest | one\noption: Forest | two"
        )));
        // A duplicate id is dropped, not counted twice: two rows naming the same
        // option are one row, which is not enough to be a card.
        assert!(!reply_offers_choice(&card(
            "title: T\noption: forest | one\noption: forest | two"
        )));
        // A duplicate beside two real rows leaves those two: still a card.
        assert!(reply_offers_choice(&card(&format!(
            "{TWO}\noption: forest | again"
        ))));
    }

    #[test]
    fn the_language_is_matched_exactly_and_case_insensitively() {
        assert!(!reply_offers_choice(&format!(
            "```{CHOICE_FENCE}-x\n{TWO}\n```\n"
        )));
        assert!(reply_offers_choice(&format!(
            "```FUTUREOS-CHOICE\n{TWO}\n```\n"
        )));
        assert!(reply_offers_choice(&format!(
            "```futureos-choice \n{TWO}\n```\n"
        )));
    }

    #[test]
    fn a_card_quoted_inside_a_document_fence_is_not_one() {
        // The renderer reads the outer fence as a Markdown document and draws the
        // inner fence as its text; nothing there is a card to answer.
        let quoted = format!("```markdown\n{TWO}\n```\n");
        assert!(!reply_offers_choice(&format!(
            "Quoting the file:\n\n{quoted}"
        )));
    }

    #[test]
    fn an_unterminated_fence_is_not_a_card() {
        // A reply cut off mid-card: the run is not a clean finish anyway, and a
        // half-written fence draws as a code block.
        assert!(!reply_offers_choice(&format!("```{CHOICE_FENCE}\n{TWO}\n")));
    }

    #[test]
    fn a_card_without_a_title_still_counts() {
        // The renderer falls back to the first option's label, so the title can
        // never be the reason a card disappears.
        assert!(reply_offers_choice(&card(
            "option: forest | 重画森林图\noption: flow | 补一张实验流程图"
        )));
    }

    #[test]
    fn ordinary_replies_and_empty_text_offer_nothing() {
        assert!(!reply_offers_choice(""));
        assert!(!reply_offers_choice("```rust\nfn main() {}\n```\n"));
        assert!(!reply_offers_choice("Which one?\n\n- a\n- b\n"));
    }
}
