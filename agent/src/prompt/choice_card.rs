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
/// The same language with the spelling folded (`CHOICE_LANGUAGE` in
/// `packages/markdown/src/choiceTagBlocks.ts`).
const CHOICE_LANGUAGE: &str = "futureoschoice";
/// §3: fewer than two rows is not a card, and seven rows are a plan. The upper
/// bound matches `CHOICE_MAX_OPTIONS` in `packages/markdown/src/parseFutureMarkdown.ts`:
/// the badge may only promise a card the client will actually draw.
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 6;
/// §3: an option id is ASCII `[a-z0-9-]`, at most 24 characters.
const MAX_OPTION_ID: usize = 24;

/// Opening and closing tags of the form a model sometimes writes instead of the
/// fence (see `normalize_choice_tags`).
const OPEN_TAG: &str = "<futureos-choice>";
const CLOSE_TAG: &str = "</futureos-choice>";

/// Does `text` close with a `futureos-choice` card?
///
/// True only when the *last* fence in the reply is a choice card and nothing but
/// whitespace follows it — the design's "the card closes the reply, alone"
/// (`docs/design/choice-cards.zh-CN.md` §3). A card in the middle of a reply
/// belongs to a question that has already been answered, and a conversation
/// whose last reply merely *quotes* a card (inside a fenced document) must not
/// read as one either.
pub fn reply_offers_choice(text: &str) -> bool {
    let normalized = normalize_choice_tags(text);
    let lines: Vec<&str> = normalized.lines().collect();
    choice_card_ends(&lines)
        // Only whitespace may follow the closing fence: the card is the reply's
        // last word, and a card quoted mid-reply is not a question to answer.
        .last()
        .is_some_and(|end| lines[end + 1..].iter().all(|line| line.trim().is_empty()))
}

/// Does any block in the reply read as a card, at the end or not? `true` here
/// and `false` from `reply_offers_choice` is a card the reader can see but not
/// answer (rendered read-only by the "last card" rule); `false` for both, in a
/// reply that spells the card's language, is the case the warning in
/// `session_prompt.rs` is for.
pub fn contains_choice_card(text: &str) -> bool {
    let normalized = normalize_choice_tags(text);
    let lines: Vec<&str> = normalized.lines().collect();
    !choice_card_ends(&lines).is_empty()
}

/// Did the reply spell the card's language and end up with no card at all?
///
/// This is the guard against tolerance swallowing the next variant: the tag
/// form is normalized, but an unpaired tag, a body carrying a fence, a row list
/// that fails the field rules, and whatever a model invents next all reach the
/// reader as raw source. A *quotation* — the word inside inline code, or a tag
/// pair inside a ` ```markdown ` example — is not an attempt at a card and stays
/// quiet; anything else is exactly the failure this reports.
pub fn choice_syntax_without_card(text: &str) -> bool {
    mentions_choice_syntax(text) && !contains_choice_card(text)
}

/// Report a reply that spells the card's syntax and yields no card, and say
/// whether it did.
///
/// This is the guard rail's mouth: tolerance for one spelling (the tag pair)
/// must not swallow the next, so a reply that names `futureos-choice` and draws
/// nothing is logged at `warn` with a phrase that greps —
/// `choice card syntax without a card`. Called once per run from
/// `rpc/session_prompt.rs`, where the reply and its run id meet.
pub fn report_unrecognized_choice(run_id: &str, text: &str) -> bool {
    if !choice_syntax_without_card(text) {
        return false;
    }
    tracing::warn!(
        run_id = %run_id,
        "choice card syntax without a card: the reply spells `futureos-choice` but no card was parsed, so the reader gets its source (`agent/src/prompt/choice_card.rs`, `packages/markdown/src/choiceTagBlocks.ts`)"
    );
    true
}

/// Whether `text` names `futureos-choice` as syntax rather than quoting it:
/// outside every fenced block and every inline code span.
fn mentions_choice_syntax(text: &str) -> bool {
    let mut open: Option<Fence> = None;
    for (index, line) in text.lines().enumerate() {
        if let Some(fence) = &open {
            if closes(line, fence) {
                open = None;
            }
            continue;
        }
        if let Some(fence) = read_fence(line, index) {
            open = Some(fence);
            continue;
        }
        if strip_inline_code(line).contains(CHOICE_FENCE) {
            return true;
        }
    }
    false
}

/// `line` with its inline code spans removed: `` `futureos-choice` `` is a
/// mention, not a block. Unmatched backticks swallow the rest of the line, which
/// only ever makes the guard quieter, never wrong about a real block (a block's
/// tag is never inside a code span).
fn strip_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut span: Option<usize> = None;
    let mut run = 0usize;
    for ch in line.chars() {
        if ch == '`' {
            run += 1;
            continue;
        }
        if run > 0 {
            match span {
                None => span = Some(run),
                Some(width) if width == run => span = None,
                Some(_) => out.push_str(&"`".repeat(run)),
            }
            run = 0;
        }
        if span.is_none() {
            out.push(ch);
        }
    }
    out
}

/// Is `info` the card's fence language, spelling folded?
///
/// Case and the `-`/`_` between the two words are the ways a writer differs
/// without meaning anything different (`FutureOS-Choice`, `futureos_choice`);
/// the renderer folds them the same way (`isChoiceFence`), so one block cannot
/// draw a card on one reader and a code block on the other. Anything else still
/// has to match: `futureos-choice-x` is some other language.
fn is_choice_fence(info: &str) -> bool {
    let folded: String = info
        .trim()
        .chars()
        .filter(|ch| *ch != '-' && *ch != '_')
        .flat_map(char::to_lowercase)
        .collect();
    folded == CHOICE_LANGUAGE
}

/// Closing-fence line indices of every choice card in `lines`, in order.
fn choice_card_ends(lines: &[&str]) -> Vec<usize> {
    let mut open: Option<Fence> = None;
    let mut ends: Vec<usize> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        match &open {
            None => {
                if let Some(fence) = read_fence(line, index) {
                    open = Some(fence);
                }
            }
            Some(fence) => {
                if closes(line, fence) {
                    if is_choice_fence(&fence.info)
                        && card_body_offers_choice(&lines[fence.body_start..index])
                    {
                        ends.push(index);
                    }
                    open = None;
                }
            }
        }
    }
    ends
}

/// `text` with closed `<futureos-choice>` tag pairs rewritten into the fence
/// they stand for, which is the one shape both this reader and the renderer's
/// parser (`packages/markdown/src/choiceTagBlocks.ts`) accept.
///
/// A pair counts when both tags are present, at the same nesting depth as
/// everything else here: an opening tag inside another fence is a *quote* of the
/// form, not an attempt at a card. The pair may sit right under a prose line —
/// the renderer splits the paragraph there, and this reader only looks at where
/// the fence starts, so the two agree (2026-11-25). An opening tag that ends the
/// reply is read as the card whose closer was forgotten (`unclosed_to_end`).
/// Anything else unpaired is left exactly as written — `choice_syntax_without_card`
/// is what makes that case loud instead of silent.
///
/// The bare fence line immediately after a closing tag is the same malformed
/// emission's attempt at a fence closer (a live reply ended `</futureos-choice>`
/// then ` ``` `): it holds nothing, so it is dropped, and the card stays the
/// reply's last block instead of sitting behind an empty code block.
fn normalize_choice_tags(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut open: Option<Fence> = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some(fence) = &open {
            if closes(line, fence) {
                open = None;
            }
            out.push(line);
            index += 1;
            continue;
        }
        if let Some(fence) = read_fence(line, index) {
            open = Some(fence);
            out.push(line);
            index += 1;
            continue;
        }
        if is_tag(line, OPEN_TAG) {
            let close =
                find_close_tag(&lines, index + 1).or_else(|| unclosed_to_end(&lines, index + 1));
            if let Some(close) = close {
                out.push(OPEN_FENCE);
                out.extend(lines[index + 1..close].iter().copied());
                out.push("```");
                index = close + 1;
                if index < lines.len() && is_attempted_closer(lines[index]) {
                    index += 1;
                }
                continue;
            }
        }
        out.push(line);
        index += 1;
    }
    out.join("\n")
}

/// The opening fence written for a normalized tag pair.
const OPEN_FENCE: &str = "```futureos-choice";

fn is_tag(line: &str, tag: &str) -> bool {
    line.trim().eq_ignore_ascii_case(tag)
}

/// Index of the line closing a tag pair opened just before `from`, or `None`
/// when a fence, another opening tag or the end of the reply comes first.
fn find_close_tag(lines: &[&str], from: usize) -> Option<usize> {
    for (offset, line) in lines[from..].iter().enumerate() {
        if read_fence(line, from + offset).is_some() || is_tag(line, OPEN_TAG) {
            return None;
        }
        if is_tag(line, CLOSE_TAG) {
            return Some(from + offset);
        }
    }
    None
}

/// The end of the reply, read as the closer an opening tag never got.
///
/// A forgotten `</futureos-choice>` at the end of a reply has nothing left to
/// swallow, and a writer that ends with a list of `option:` rows meant a card
/// (the 2026-11-25 reply). The same shape mid-reply is *not* read: there the
/// missing closer would take whatever follows with it, so it stays text and the
/// guard reports it. A body that is not option rows stays text either way.
fn unclosed_to_end(lines: &[&str], from: usize) -> Option<usize> {
    for (offset, line) in lines[from..].iter().enumerate() {
        if read_fence(line, from + offset).is_some() || is_tag(line, OPEN_TAG) {
            return None;
        }
    }
    let rows = lines[from..]
        .iter()
        .filter(|line| directive_entry(line).is_some_and(|(key, _)| key == "option"))
        .count();
    if rows < MIN_OPTIONS || !lines[from..].iter().all(|line| is_card_field(line)) {
        return None;
    }
    Some(lines.len())
}

/// Is `line` one of the card's own fields (`title:`, `mode:`, `context:`, `hint:`,
/// `option:`) or blank?
///
/// The unclosed-tag case needs it: with nothing after the tag to mark the end of
/// the card, a body of *prose* is the tag the reply prints, and reading it as a
/// body would swallow the sentence the reader wrote next.
fn is_card_field(line: &str) -> bool {
    if line.trim().is_empty() {
        return true;
    }
    directive_entry(line).is_some_and(|(key, _)| {
        matches!(
            key.to_ascii_lowercase().as_str(),
            "title" | "mode" | "context" | "hint" | "option"
        )
    })
}

/// A fence run with nothing after it: what a writer meant as a closer.
fn is_attempted_closer(line: &str) -> bool {
    fence_run(line).is_some_and(|(start, _ch, run)| line[start + run..].trim().is_empty())
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
    use super::{
        choice_syntax_without_card, contains_choice_card, reply_offers_choice,
        report_unrecognized_choice, CHOICE_FENCE,
    };

    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
        type Writer = Sink;

        fn make_writer(&'a self) -> Sink {
            self.clone()
        }
    }

    /// Everything a `warn`-and-up subscriber writes while `closure` runs.
    fn logged(closure: impl FnOnce()) -> String {
        let sink = Sink::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .finish();
        tracing::subscriber::with_default(subscriber, closure);
        let bytes = sink.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

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

    /// The real reply that started this (2026-11-24): a closed tag pair written
    /// where the fence belongs, and the bare fence line it meant as a closer.
    const TAGGED_TAIL: &str = "还有一点：设计上「畸形块降级为代码块」。\n\n<futureos-choice>\ntitle: 这两处要现在改吗？\ncontext: 同模型两次都把围栏写成 XML 标签。\nmode: multi\noption: parser | 解析器和徽章都接受标签形式 | parseFutureMarkdown 加一条识别\noption: prompt | 提示词里补一句「必须围栏」 | 成本一行字\n</futureos-choice>\n```";

    #[test]
    fn a_closed_tag_pair_reads_as_the_card_it_stands_for() {
        assert!(reply_offers_choice(TAGGED_TAIL));
        assert!(contains_choice_card(TAGGED_TAIL));
        // A card was produced, so the guard rail is silent.
        assert!(!choice_syntax_without_card(TAGGED_TAIL));
    }

    #[test]
    fn the_tag_form_and_the_fence_form_read_the_same() {
        // The pair is normalized and then read by the fence rules, so the same
        // body cannot give two answers depending on how it was spelled.
        let body = TWO;
        let fenced = card(body);
        let tagged = format!("<futureos-choice>\n{body}\n</futureos-choice>\n");
        assert_eq!(reply_offers_choice(&fenced), reply_offers_choice(&tagged));
        assert_eq!(contains_choice_card(&fenced), contains_choice_card(&tagged));
        // …including the rules a card can fail: one row is not a card in either
        // spelling, and the guard says so about the tagged one.
        let one_row = "title: T\noption: forest | one";
        assert!(!reply_offers_choice(&card(one_row)));
        assert!(!reply_offers_choice(&format!(
            "<futureos-choice>\n{one_row}\n</futureos-choice>"
        )));
        assert!(choice_syntax_without_card(&format!(
            "<futureos-choice>\n{one_row}\n</futureos-choice>"
        )));
    }

    /// An opening tag at the end of the reply reads as a card: nothing follows
    /// it, and a body of option rows is a card the closer was forgotten from
    /// (the 2026-11-25 reply ended that way).
    #[test]
    fn an_opening_tag_that_ends_the_reply_reads_as_a_card() {
        let open_only = format!("还有一点：\n\n<futureos-choice>\n{TWO}");
        assert!(reply_offers_choice(&open_only));
        assert!(contains_choice_card(&open_only));
        assert!(!choice_syntax_without_card(&open_only));
    }

    #[test]
    fn an_unpaired_tag_with_something_after_it_is_not_a_card_and_is_reported() {
        // Mid-reply the missing closer would swallow what follows, so it stays
        // text — and the guard says so.
        let mid_reply = format!("<futureos-choice>\n{TWO}\n\n还有别的吗？");
        assert!(!reply_offers_choice(&mid_reply));
        assert!(!contains_choice_card(&mid_reply));
        assert!(choice_syntax_without_card(&mid_reply));

        // A body that is not option rows is the tag the reply prints, not an
        // attempt at a card.
        let prose_only = "<futureos-choice>\ntitle: T\n这里只是一段话";
        assert!(!contains_choice_card(prose_only));
        assert!(choice_syntax_without_card(prose_only));

        // A closing tag with no opening one is never a card.
        let close_only = format!("{TWO}\n</futureos-choice>\n");
        assert!(!reply_offers_choice(&close_only));
        assert!(choice_syntax_without_card(&close_only));
    }

    #[test]
    fn a_pair_attached_to_the_line_before_it_is_still_a_card() {
        // The 2026-11-25 reply wrote `…再问一遍：` and the tag on the next line,
        // no blank line between. The renderer splits the paragraph there; this
        // reader sees the fence wherever the line starts, so the two agree.
        let attached = format!("还有一点：\n<futureos-choice>\n{TWO}\n</futureos-choice>\n");
        assert!(reply_offers_choice(&attached));
        assert_eq!(
            contains_choice_card(&attached),
            contains_choice_card(&format!(
                "还有一点：\n\n<futureos-choice>\n{TWO}\n</futureos-choice>\n"
            ))
        );
    }

    #[test]
    fn the_language_folds_the_ways_a_writer_spells_it() {
        for info in ["FUTUREOS-CHOICE", "futureos_choice"] {
            assert!(
                reply_offers_choice(&format!("```{info}\n{TWO}\n```\n")),
                "{info}"
            );
        }
        // Anything past that is another language, not a card.
        assert!(!reply_offers_choice(&format!(
            "```futureos-choice-x\n{TWO}\n```\n"
        )));
    }

    #[test]
    fn the_word_alone_is_not_a_card() {
        // Prose naming the block: syntax, not a quotation — the guard fires.
        assert!(!contains_choice_card("Use a futureos-choice block here."));
        assert!(choice_syntax_without_card(
            "Use a futureos-choice block here."
        ));
        // Inline code is a quotation of the name, not an attempt at a block.
        assert!(!choice_syntax_without_card(
            "A `futureos-choice` block is a fence."
        ));
    }

    #[test]
    fn a_tag_pair_quoted_in_a_document_fence_is_content() {
        let quoted = format!("```markdown\n<futureos-choice>\n{TWO}\n</futureos-choice>\n```\n");
        assert!(!reply_offers_choice(&quoted));
        assert!(!contains_choice_card(&quoted));
        // Quoting the form is not failing to draw it.
        assert!(!choice_syntax_without_card(&quoted));
    }

    #[test]
    fn a_card_behind_an_attempted_closer_reads_as_the_last_block() {
        // The stray closer holds nothing and is dropped with the pair it belongs
        // to, so the card is still the reply's last block.
        assert!(reply_offers_choice(TAGGED_TAIL));
        // Real content after it is not dropped: the card is then mid-reply, which
        // is a card the reader can see (read-only) and not a failure to warn about.
        let mid_reply =
            format!("<futureos-choice>\n{TWO}\n</futureos-choice>\n```\n\nAny questions?");
        assert!(!reply_offers_choice(&mid_reply));
        assert!(contains_choice_card(&mid_reply));
        assert!(!choice_syntax_without_card(&mid_reply));
    }

    /// The guard's mouth, actually printed: the message a run leaves in the log
    /// when a reply names the card and draws none.
    #[test]
    fn the_guard_prints_a_greppable_warning() {
        // An unpaired tag mid-reply: the shape the reader still gets as source
        // (the reply ends with prose, so the missing closer is not forgiven).
        let mid_reply = format!("<futureos-choice>\n{TWO}\n\n还有别的吗？");
        let mut reported = false;
        let output = logged(|| {
            reported = report_unrecognized_choice("run-1", &mid_reply);
        });
        assert!(reported);
        assert!(
            output.contains("WARN") && output.contains("choice card syntax without a card"),
            "{output}"
        );
        assert!(output.contains("run_id=run-1"), "{output}");

        // A reply that draws its card is silent.
        let quiet = logged(|| {
            assert!(!report_unrecognized_choice("run-2", TAGGED_TAIL));
        });
        assert!(quiet.is_empty(), "{quiet}");
    }
}
