//! Pure parsing of references out of message markdown: `futureos://` id-based
//! links, `futureos-*` fenced blocks, and plain local-file path links
//! (`[name](/abs/path)`, `[name](<./rel path>)`, or the `[/abs/path]` shortcut).
//! No database access — see the parent module for resolution, search and
//! persistence.

use std::collections::HashSet;

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub(super) struct MarkdownObjectReference {
    pub(super) target_id: String,
    pub(super) target_type: String,
}

pub(super) fn extract_markdown_references(content: &str) -> Vec<MarkdownObjectReference> {
    let mut seen = HashSet::new();
    let mut references = vec![];

    for reference in extract_futureos_links(content)
        .into_iter()
        .chain(extract_futureos_fences(content))
        .chain(extract_local_file_links(content))
    {
        if seen.insert(reference.clone()) {
            references.push(reference);
        }
    }

    references
}

fn extract_futureos_links(content: &str) -> Vec<MarkdownObjectReference> {
    let mut references = vec![];
    let mut remaining = content;

    while let Some(start) = remaining.find("futureos://") {
        let after_scheme = &remaining[start + "futureos://".len()..];
        let Some((target_type, rest)) = after_scheme.split_once('/') else {
            break;
        };
        let target_type = normalize_target_type(target_type);
        let Some(target_type) = target_type else {
            remaining = &after_scheme[target_type_len(after_scheme)..];
            continue;
        };

        let raw_target_id = rest
            .split(|character: char| {
                character == ')'
                    || character == ']'
                    || character == ' '
                    || character == '\n'
                    || character == '\t'
                    || character == '?'
                    || character == '#'
            })
            .next()
            .unwrap_or_default();
        let target_id = raw_target_id.trim();

        if !target_id.is_empty() {
            references.push(MarkdownObjectReference {
                target_id: percent_decode(target_id),
                target_type,
            });
        }

        remaining = &rest[raw_target_id.len()..];
    }

    references
}

fn extract_futureos_fences(content: &str) -> Vec<MarkdownObjectReference> {
    let mut references = vec![];
    let mut lines = content.lines();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        let Some(target_type) = trimmed
            .strip_prefix("```futureos-")
            .and_then(normalize_target_type)
        else {
            continue;
        };

        let mut target_id = String::new();
        for body_line in lines.by_ref() {
            let body = body_line.trim();
            if body == "```" {
                break;
            }
            if let Some(value) = body.strip_prefix("id:") {
                target_id = value.trim().to_string();
            }
        }

        if !target_id.is_empty() {
            references.push(MarkdownObjectReference {
                target_id,
                target_type,
            });
        }
    }

    references
}

fn normalize_target_type(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        // `file` is intentionally absent: local files arrive as plain path links
        // (see `extract_local_file_links`), not via the `futureos://` scheme,
        // which now carries only id-based objects.
        "approval" | "artifact" | "review" | "run" | "tool" => Some(normalized),
        _ => None,
    }
}

/// Extract `file` references from plain markdown links whose destination is a
/// local path — the counterpart to the frontend's `localFilePath` classifier.
/// Recognizes inline `[label](dest)` / `[label](<dest>)` and the `[dest]`
/// shortcut; image links (`![alt](src)`) are skipped.
fn extract_local_file_links(content: &str) -> Vec<MarkdownObjectReference> {
    let mut references = vec![];
    let bytes = content.as_bytes();
    let mut search = 0;

    while let Some(rel) = content[search..].find('[') {
        let bracket = search + rel;
        search = bracket + 1;

        // Skip image links: `![alt](src)`.
        if bracket > 0 && bytes[bracket - 1] == b'!' {
            continue;
        }

        let after = &content[bracket + 1..];
        let Some(close) = after.find(']') else {
            break;
        };
        let label = &after[..close];
        let rest = &after[close + 1..];

        let dest = if let Some(inner) = rest.strip_prefix('(') {
            link_destination(inner)
        } else {
            // Shortcut `[dest]`: the bracket content itself is the destination.
            Some(label)
        };

        if let Some(path) = dest.and_then(local_file_path) {
            references.push(MarkdownObjectReference {
                target_id: path,
                target_type: "file".to_string(),
            });
        }
    }

    references
}

/// The destination slice of an inline link, starting just after the opening
/// `(`. Handles the angle-bracket form `<...>` (allows spaces) and the bare
/// form (ends at the first `)` or whitespace).
fn link_destination(inner: &str) -> Option<&str> {
    if let Some(rest) = inner.strip_prefix('<') {
        rest.find('>').map(|end| &rest[..end])
    } else {
        let end = inner
            .find(|character: char| character == ')' || character.is_whitespace())
            .unwrap_or(inner.len());
        Some(&inner[..end])
    }
}

/// Classify a link destination as a local filesystem path, returning the
/// normalized path. Mirrors the frontend `localFilePath` (see
/// `gui/src/features/markdown/localPath.ts`) — keep the two in sync.
fn local_file_path(href: &str) -> Option<String> {
    let raw = href.trim();
    if raw.is_empty() {
        return None;
    }

    // `file://` URI — decode to its plain path.
    if raw
        .as_bytes()
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"file://"))
    {
        let after = &raw[7..];
        let path = match after.find('/') {
            Some(index) => &after[index..],
            None => after,
        };
        let decoded = percent_decode(path);
        return (!decoded.is_empty()).then_some(decoded);
    }

    // Any other explicit URL scheme (http:, https:, mailto:, futureos:, …) is
    // not a local path. A scheme needs two-plus chars, so a Windows drive letter
    // (`C:`) falls through to the drive check below.
    if let Some(colon) = raw.find(':') {
        let scheme = &raw[..colon];
        let is_scheme = scheme.len() >= 2
            && scheme.chars().enumerate().all(|(index, character)| {
                if index == 0 {
                    character.is_ascii_alphabetic()
                } else {
                    character.is_ascii_alphanumeric()
                        || character == '+'
                        || character == '.'
                        || character == '-'
                }
            });
        if is_scheme {
            return None;
        }
    }

    // POSIX absolute.
    if raw.starts_with('/') {
        return Some(raw.to_string());
    }

    // Windows UNC (`\\server\share`).
    if raw.starts_with("\\\\") {
        return Some(raw.to_string());
    }

    // Windows drive absolute (`C:\` or `C:/`).
    let bytes = raw.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return Some(raw.to_string());
    }

    // Explicit relative (`./x`, `../x`, or backslash forms). Strip a single
    // leading `./`; `../` is preserved.
    if let Some(rest) = raw.strip_prefix("./").or_else(|| raw.strip_prefix(".\\")) {
        return Some(rest.to_string());
    }
    if raw.starts_with("../") || raw.starts_with("..\\") {
        return Some(raw.to_string());
    }

    None
}

fn target_type_len(value: &str) -> usize {
    value.find('/').unwrap_or(value.len()).saturating_add(1)
}

fn percent_decode(value: &str) -> String {
    // Decode on raw bytes only: an `&str` slice like `value[index+1..index+3]`
    // panics when `%` is followed by a multi-byte character (non-char-boundary
    // slice), and this input is agent-produced markdown — it must never unwind.
    fn hex_digit(byte: u8) -> Option<u8> {
        (byte as char).to_digit(16).map(|digit| digit as u8)
    }
    let mut output = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
            {
                output.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_inline_and_fenced_references_once() {
        let references = extract_markdown_references(
            r#"
See [plan](futureos://artifact/artifact_123) and [run](futureos://run/run_456).
Duplicate [plan again](futureos://artifact/artifact_123).
Other objects: [tool](futureos://tool/tool_123), [approval](futureos://approval/approval_123),
[review](futureos://review/review_123).

```futureos-artifact
id: artifact_789
view: card
```

```futureos-run
id: run_456
view: timeline
```
"#,
        );

        assert_eq!(
            references,
            vec![
                MarkdownObjectReference {
                    target_id: "artifact_123".to_string(),
                    target_type: "artifact".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "run_456".to_string(),
                    target_type: "run".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "tool_123".to_string(),
                    target_type: "tool".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "approval_123".to_string(),
                    target_type: "approval".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "review_123".to_string(),
                    target_type: "review".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "artifact_789".to_string(),
                    target_type: "artifact".to_string(),
                },
            ]
        );
    }

    #[test]
    fn percent_decodes_utf8_reference_ids() {
        assert_eq!(percent_decode("%E8%AF%97"), "诗");
        assert_eq!(percent_decode("%E0%A4%A"), "%E0%A4%A");
    }

    #[test]
    fn percent_decode_survives_percent_before_multibyte_char() {
        // Regression: `%` directly followed by a multi-byte character used to
        // slice the &str off a char boundary and panic.
        assert_eq!(percent_decode("%诗"), "%诗");
        assert_eq!(percent_decode("%E8诗"), "%E8诗");
        assert_eq!(percent_decode("诗%"), "诗%");
    }

    #[test]
    fn extracts_file_references_from_plain_path_links() {
        let references = extract_markdown_references(
            "Wrote [test.txt](/Users/tao/app/test.txt) and [poem.txt](./poem.txt), \
             plus [notes.txt](</Users/tao/My Docs/notes.txt>) and see [/Users/tao/x.log].",
        );

        assert_eq!(
            references,
            vec![
                MarkdownObjectReference {
                    target_id: "/Users/tao/app/test.txt".to_string(),
                    target_type: "file".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "poem.txt".to_string(),
                    target_type: "file".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "/Users/tao/My Docs/notes.txt".to_string(),
                    target_type: "file".to_string(),
                },
                MarkdownObjectReference {
                    target_id: "/Users/tao/x.log".to_string(),
                    target_type: "file".to_string(),
                },
            ]
        );
    }

    #[test]
    fn ignores_remote_links_and_images() {
        let references = extract_markdown_references(
            "Docs at [site](https://example.com/page), mail [me](mailto:a@b.com), \
             image ![chart](/Users/tao/chart.png).",
        );

        assert_eq!(references, vec![]);
    }

    #[test]
    fn classifies_windows_paths_as_local_files() {
        assert_eq!(
            local_file_path("C:/Users/tao/report.txt"),
            Some("C:/Users/tao/report.txt".to_string())
        );
        assert_eq!(
            local_file_path("C:\\Users\\tao\\report.txt"),
            Some("C:\\Users\\tao\\report.txt".to_string())
        );
        assert_eq!(
            local_file_path("\\\\server\\share\\file.txt"),
            Some("\\\\server\\share\\file.txt".to_string())
        );
        assert_eq!(local_file_path("poem2.txt"), None);
        assert_eq!(local_file_path("https://example.com"), None);
    }
}
