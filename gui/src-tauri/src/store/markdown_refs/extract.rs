//! Pure parsing of `futureos://` references and `futureos-*` fenced blocks out
//! of message markdown. No database access — see the parent module for
//! resolution, search and persistence.

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
        // `file` addresses an artifact by its filesystem path instead of its id,
        // so the model can reference a file it just wrote without knowing the
        // store-minted artifact id. See `resolve::get_file_artifact_in_workspace`.
        "approval" | "artifact" | "file" | "research" | "review" | "run" | "tool" => {
            Some(normalized)
        }
        _ => None,
    }
}

fn target_type_len(value: &str) -> usize {
    value.find('/').unwrap_or(value.len()).saturating_add(1)
}

fn percent_decode(value: &str) -> String {
    let mut output = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                output.push(hex);
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
[review](futureos://review/review_123), [research](futureos://research/research_123).

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
                    target_id: "research_123".to_string(),
                    target_type: "research".to_string(),
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
    fn extracts_file_references_by_percent_encoded_path() {
        let references = extract_markdown_references(
            "Wrote [test.txt](futureos://file/%2FUsers%2Ftao%2Fapp%2Ftest.txt).",
        );

        assert_eq!(
            references,
            vec![MarkdownObjectReference {
                target_id: "/Users/tao/app/test.txt".to_string(),
                target_type: "file".to_string(),
            }]
        );
    }
}
