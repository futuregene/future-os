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
        "[Image #1]",
        "[Pasted Content 2000 chars]",
        "**bold** and `code`",
        "- a bullet",
        "> a quote",
        "1. one",
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

/// The session list of a TUI conversation showed `[Image #1]现在 tui 中…`:
/// a marker the composer wrote named the conversation.
#[test]
fn composer_placeholders_and_markdown_are_flattened_away() {
    assert_eq!(
        from_str("[Image #1]现在 tui 中显示展示思考中"),
        "现在 tui 中显示展示思考中"
    );
    assert_eq!(from_str("[Image #1][Image #2]"), "");
    assert_eq!(from_str("[Pasted Content 2000 chars] 帮我看看"), "帮我看看");
    assert_eq!(from_str("## 修复 **登录** 超时"), "修复 登录 超时");
    assert_eq!(from_str("- 第一条\n- 第二条"), "第一条 第二条");
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
