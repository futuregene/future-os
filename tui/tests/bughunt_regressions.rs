use future_tui::components::autocomplete::{
    AttachmentProvider, AutocompleteProvider, FilePathProvider,
};
use future_tui::components::input::Input;
use future_tui::components::markdown::MarkdownRenderer;
use future_tui::stdin_buffer::{StdinBuffer, StdinEvent};
use future_tui::tui::Component;
use future_tui::utils::strip_ansi_codes;

#[test]
fn unicode_meta_keys_and_completion_offsets_are_safe() {
    for text in ["中", "é", "🎉"] {
        let mut buffer = StdinBuffer::new();
        let input = format!("\x1b{text}");
        assert_eq!(
            buffer.process_bytes(input.as_bytes()),
            vec![StdinEvent::Data(input.clone())]
        );
        let mut editor = Input::new();
        let path = format!("./{text}");
        editor.set_value(&path, None);
        assert_eq!(editor.cursor_byte(), path.len());
        let file = FilePathProvider::new(None);
        let ctx = file.r#match(&path, editor.cursor_byte()).unwrap();
        assert_eq!(ctx.token, path);
        for offset in 0..path.len() {
            let _ = file.r#match(&path, offset);
        }
        let attachment = AttachmentProvider::default();
        for offset in 0..input.len() {
            let _ = attachment.r#match(&input, offset);
        }
    }
}

#[test]
fn decorated_unicode_input_keeps_source_offsets_on_character_boundaries() {
    for text in [
        "你\x1b[31m好\x1b[0m世界",
        "你\x1b[31m好世界\x1b[0m",
        "a\x1b[31m中文\x1b[0m测试",
        "\t中文测试",
        "你 \t 世界",
        "a\u{200d}你世界",
    ] {
        for width in 3..16 {
            let mut input = Input::new();
            input.set_value(text, None);
            input.render(width);
            for key in ["up", "up", "down", "down"] {
                input.handle_key(key);
                input.render(width);
                assert!(
                    text.is_char_boundary(input.cursor_byte()),
                    "{text:?} width={width}"
                );
            }
        }
    }
}

#[test]
fn wrapped_cursor_accounts_for_discarded_spaces_and_newlines() {
    let mut input = Input::new();
    input.set_value("aaaa bbbb cccc", Some(7));
    input.render(9);
    input.handle_key("up");
    assert_eq!(input.cursor(), 2);
    input.set_value("aaaa\nbbbb", Some(7));
    input.render(9);
    input.handle_key("up");
    assert_eq!(input.cursor(), 2);
}

#[test]
fn markdown_preserves_nested_headings_and_blocks_control_entities() {
    let mut md = MarkdownRenderer::new();
    for text in ["- # heading\n- tail", "- item\n  > # heading\n  > body"] {
        let output = md.render_text(text, 60).join("\n");
        assert!(strip_ansi_codes(&output).contains("heading"));
    }
    for text in ["&#27;[2Jhello", "&#x1b;]0;title&#7;", "a\u{9b}2Jb"] {
        let output = md.render_text(text, 60).join("\n");
        assert!(!output.contains("\x1b[2J"));
        assert!(!output.contains("\x1b]0;"));
        assert!(!output.contains('\x07'));
        assert!(!output.contains('\u{9b}'));
    }
    assert_eq!(md.render_text("   ", 60).len(), 1);
    assert!(md.render_text("", 60).is_empty());
}

#[test]
fn list_code_borders_fit_content_width_without_spurious_rows() {
    let text = "- item\n\n  ```\n  code\n  ```";
    for width in [8, 24, 40, 60, 80] {
        let mut md = MarkdownRenderer::new();
        let lines: Vec<String> = md
            .render_text(text, width)
            .iter()
            .map(|s| strip_ansi_codes(s))
            .collect();
        let borders: Vec<_> = lines.iter().filter(|s| s.contains('─')).collect();
        assert_eq!(
            borders.len(),
            2,
            "borders split at width {width}: {lines:?}"
        );
        assert!(borders
            .iter()
            .all(|s| s.trim().chars().count() == (width - 2).min(60)));
        assert!(!lines.iter().any(|s| s == " "));
    }
}

#[test]
fn paragraph_leading_whitespace_is_preserved_only_at_top_level() {
    let mut md = MarkdownRenderer::new();
    let lines: Vec<String> = md
        .render_text("   a\n\n b", 60)
        .iter()
        .map(|s| strip_ansi_codes(s))
        .collect();
    assert_eq!(lines, ["   a", "", " b"]);
    assert_eq!(
        strip_ansi_codes(&md.render_text("\tcode", 60)[0]),
        "   code"
    );
    assert_eq!(
        strip_ansi_codes(&md.render_text("- a\n\n  b", 60)[1]),
        "  b"
    );
}

#[test]
fn osc_urls_reject_controls_while_image_protocol_lines_stay_intact() {
    use future_tui::terminal_image::hyperlink;
    for url in [
        "https://x/\x1b\\\x1b]0;title\x07",
        "https://x/\nnext",
        "https://x/\u{9c}",
    ] {
        assert_eq!(hyperlink("label", url), "label");
    }
    let mut md = MarkdownRenderer::new();
    // This goes through cmark's destination/entity parser before OSC rendering.
    let output = md
        .render_text("[click](<https://x/&#27;]0;injected&#7;>)", 80)
        .join("\n");
    assert!(!output.contains("\x1b]0;injected"));
    assert!(!output.contains('\x07'));
    assert!(strip_ansi_codes(&output).contains("click"));
    let image = "\x1b_Ga=T,f=100;YQ==\x1b\\";
    assert!(md.render_text(image, 80)[0].contains(image));
}

#[test]
fn file_completion_tracks_current_session_working_directory() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("first.txt"), "a").unwrap();
    std::fs::write(second.path().join("second.txt"), "b").unwrap();
    let mut provider = FilePathProvider::new(Some(first.path().to_string_lossy().into_owned()));
    let ctx = provider.r#match("./", 2).unwrap();
    assert!(provider
        .get_completions(&ctx)
        .iter()
        .any(|i| i.value.contains("first.txt")));
    provider.update_state(&second.path().to_string_lossy(), &[], &[]);
    let items = provider.get_completions(&ctx);
    assert!(items.iter().any(|i| i.value.contains("second.txt")));
    assert!(!items.iter().any(|i| i.value.contains("first.txt")));
}
