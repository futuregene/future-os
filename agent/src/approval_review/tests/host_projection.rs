//! Host projection of tool results: what the reviewer is allowed to see.

use crate::approval_review::tool_evidence::result_summary;

/// A 3-byte character straddling the 2048-byte tail window. The slice must move
/// forward to a char boundary instead of panicking, and the summary must report
/// the full body length, not the window.
#[test]
fn the_tail_window_is_moved_to_a_char_boundary_without_losing_the_length() {
    let body = format!("a{}", "€".repeat(1000));
    assert_eq!(body.len(), 3001);
    // 3001 - 2048 = 953: inside the third byte of a character, so the start has
    // to advance to the next boundary.
    assert!(!body.is_char_boundary(953));
    let summary = result_summary("shell", true, &body);
    assert_eq!(summary["output_bytes"], serde_json::json!(3001));
    assert_eq!(summary["result_status"], serde_json::json!("error"));
    assert_eq!(summary["content_policy"], serde_json::json!("body_omitted"));
    assert_eq!(
        summary["diagnostic_category"],
        serde_json::json!("other_error")
    );
    assert!(
        summary["exit_code"].is_null(),
        "a body without a shell trailer has no exit code"
    );

    // The same body as a success carries no diagnostic category at all.
    let summary = result_summary("shell", false, &body);
    assert_eq!(summary["result_status"], serde_json::json!("completed"));
    assert!(summary["diagnostic_category"].is_null());
}

#[test]
fn an_error_tail_is_classified_into_one_fixed_category() {
    for (tail, expected) in [
        ("cat: /w/secret: Permission denied", "permission_denied"),
        ("cp: /w/out: Read-only file system", "read_only_filesystem"),
        ("cat: /w/typo: No such file or directory", "not_found"),
        ("curl: (28) operation timed out", "timeout"),
        ("segfault while rendering", "other_error"),
    ] {
        let summary = result_summary("shell", true, tail);
        assert_eq!(
            summary["diagnostic_category"],
            serde_json::json!(expected),
            "{tail}"
        );
    }

    // The exit code comes from the shell's own trailer, for shell results only.
    let summary = result_summary("shell", true, "boom [exit: 3]");
    assert_eq!(summary["exit_code"], serde_json::json!(3));
    assert_eq!(
        summary["diagnostic_category"],
        serde_json::json!("other_error")
    );
    assert!(result_summary("read", true, "boom [exit: 3]")["exit_code"].is_null());
    assert!(result_summary("shell", true, "boom [exit: not-a-number]")["exit_code"].is_null());
}
