//! Authored during implementation; run only in the final validation phase.
use super::*;

fn old(text: &str) -> String {
    crate::session::projection::truncate_visible(&future_rpc::title::plain_title_text(text), 40)
}

#[test]
fn bounded_preview_preserves_shared_markdown_and_whitespace_rules() {
    let parts = [
        "",
        "  ",
        "word",
        "中🙂",
        "[reference](futureos://session/s)",
        "![image](./image.png)",
        "[docs](path(with(parens)).md)",
        "[angle](<futureos://session/s>)",
        "[ordinary](<https://example.com>)",
        "[unfinished](futureos://session/s",
        "items[0]",
        "[[label](url)",
        "[first][second](futureos://session/s)",
        "[only](futureos://session/s) [more](futureos://session/t)",
        "[angle]( <futureos://session/s> )",
        "[not a reference](< futureos://session/s>)",
    ];
    for first in parts {
        for second in parts {
            let text = format!("{first}\n {second} trailing topic");
            assert_eq!(from_str(&text), old(&text), "{text}");
        }
    }
    for text in parts {
        assert_eq!(from_str(text), old(text), "{text}");
    }
}

#[test]
fn paged_preview_handles_long_labels_targets_and_small_unicode_pages() {
    for text in [
        format!(
            "[{}](futureos://session/{}) 这个会话的主题",
            "other".repeat(20000),
            "id".repeat(40000)
        ),
        format!("[{}](futureos://session/s)", "中🙂".repeat(10000)),
        format!("{} topic", "[".repeat(100000)),
        format!("[label]({}) real text", "url(with(inner)).".repeat(10000)),
    ] {
        let mut largest = 0;
        let actual = paged(text.len(), |start, budget| {
            largest = largest.max(budget);
            let mut end = (start + budget.min(13)).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Ok(text[start..end].to_owned())
        })
        .unwrap();
        assert_eq!(actual, old(&text));
        assert!(largest <= PAGE_BYTES);
    }
}

#[test]
fn ordinary_large_messages_stop_after_the_preview_and_zero_width_output_is_capped() {
    let text = format!("actual topic {}", "unneeded content".repeat(100000));
    let mut bytes = 0;
    let result = paged(text.len(), |start, budget| {
        bytes += budget;
        Ok(text[start..start + budget].to_owned())
    })
    .unwrap();
    assert_eq!(result, old(&text));
    assert!(bytes <= PAGE_BYTES);
    let text = "\u{200b}".repeat(100000);
    let result = from_str(&text);
    assert!(result.len() <= MAX_PREVIEW_BYTES && !result.is_empty());
    assert!(result.chars().all(|ch| ch == '\u{200b}'));
}
