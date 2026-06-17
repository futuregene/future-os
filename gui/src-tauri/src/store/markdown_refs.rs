use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;

use super::initialize_app_store;
use super::models::*;
use super::support::{
    approval_request_from_row, artifact_from_row, create_id, now_millis,
    research_resource_from_row, review_changeset_from_row, run_from_row, tool_call_from_row,
};

pub fn resolve_markdown_references(
    input: ResolveMarkdownReferencesInput,
) -> Result<Vec<ResolvedMarkdownReference>, String> {
    initialize_app_store()?;
    let workspace_id = input.workspace_id.trim().to_string();
    if workspace_id.is_empty() {
        return Err("workspace id is required to resolve markdown references.".to_string());
    }
    let conn = super::support::connect()?;
    Ok(input
        .references
        .into_iter()
        .map(|reference| resolve_markdown_reference(&conn, &workspace_id, reference))
        .collect())
}

pub fn search_reference_targets(
    input: SearchReferenceTargetsInput,
) -> Result<Vec<ReferenceTargetSearchResult>, String> {
    initialize_app_store()?;
    let conn = super::support::connect()?;
    let query = input.query.unwrap_or_default().trim().to_ascii_lowercase();
    let limit = input.limit.unwrap_or(12).clamp(1, 30) as usize;
    let mut results = Vec::new();

    search_artifact_targets(&conn, &input.workspace_id, &query, &mut results)?;
    search_run_targets(&conn, &input.workspace_id, &query, &mut results)?;
    search_tool_targets(&conn, &input.workspace_id, &query, &mut results)?;
    search_approval_targets(&conn, &input.workspace_id, &query, &mut results)?;
    search_review_targets(&conn, &input.workspace_id, &query, &mut results)?;
    search_research_targets(&conn, &input.workspace_id, &query, &mut results)?;

    results.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.title.cmp(&right.title))
    });
    results.truncate(limit);
    Ok(results)
}

fn resolve_markdown_reference(
    conn: &Connection,
    workspace_id: &str,
    reference: MarkdownReferenceInput,
) -> ResolvedMarkdownReference {
    let target_type = reference.target_type.trim().to_ascii_lowercase();
    let target_id = reference.target_id.trim().to_string();

    if target_id.is_empty() {
        return missing_reference(target_type, target_id, "reference id is empty");
    }

    match target_type.as_str() {
        "artifact" => match get_artifact_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(artifact)) => resolved_reference(target_type, target_id, artifact),
            Ok(None) => missing_reference(target_type, target_id, "artifact was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "run" => match get_run_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(run)) => resolved_reference(target_type, target_id, run),
            Ok(None) => missing_reference(target_type, target_id, "run was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "tool" => match get_tool_call_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(tool)) => resolved_reference(target_type, target_id, tool),
            Ok(None) => missing_reference(target_type, target_id, "tool call was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "approval" => match get_approval_request_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(approval)) => resolved_reference(target_type, target_id, approval),
            Ok(None) => missing_reference(target_type, target_id, "approval request was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "review" => match get_review_changeset_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(review)) => resolved_reference(target_type, target_id, review),
            Ok(None) => missing_reference(target_type, target_id, "review changeset was not found"),
            Err(error) => failed_reference(target_type, target_id, error),
        },
        "research" => match get_research_resource_in_workspace(conn, workspace_id, &target_id) {
            Ok(Some(resource)) => resolved_reference(target_type, target_id, resource),
            Ok(None) => {
                missing_reference(target_type, target_id, "research resource was not found")
            }
            Err(error) => failed_reference(target_type, target_id, error),
        },
        _ => missing_reference(
            target_type,
            target_id,
            "reference type is not supported yet",
        ),
    }
}

fn search_artifact_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, type, path, summary, updated_at
             FROM artifacts
             WHERE workspace_id = ?1 AND deleted_at IS NULL
             ORDER BY updated_at DESC
             LIMIT 80",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let artifact_type: String = row.get(2)?;
            let path: Option<String> = row.get(3)?;
            let summary: Option<String> = row.get(4)?;
            let updated_at: i64 = row.get(5)?;
            let search_text = compact_search_text(
                &[&title, &artifact_type],
                &[path.as_ref(), summary.as_ref()],
            );
            Ok(ReferenceTargetSearchResult {
                target_type: "artifact".to_string(),
                target_id: id,
                title,
                subtitle: path.or(Some(artifact_type)),
                search_text: Some(search_text),
                updated_at,
            })
        })
        .map_err(|error| error.to_string())?;
    collect_matching_targets(rows, query, results)
}

fn search_run_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.status, r.model_id, r.error_message, r.updated_at
             FROM runs r
             JOIN threads t ON t.id = r.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY r.updated_at DESC
             LIMIT 80",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let id: String = row.get(0)?;
            let status: String = row.get(1)?;
            let model_id: Option<String> = row.get(2)?;
            let error_message: Option<String> = row.get(3)?;
            let updated_at: i64 = row.get(4)?;
            let title = format!("Run {}", short_id(&id));
            let search_text = compact_search_text(
                &[&id, &status],
                &[model_id.as_ref(), error_message.as_ref()],
            );
            Ok(ReferenceTargetSearchResult {
                target_type: "run".to_string(),
                target_id: id,
                title,
                subtitle: model_id.or(Some(status)),
                search_text: Some(search_text),
                updated_at,
            })
        })
        .map_err(|error| error.to_string())?;
    collect_matching_targets(rows, query, results)
}

fn search_tool_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT tc.id, tc.name, tc.kind, tc.status, tc.input, tc.created_at
             FROM tool_calls tc
             JOIN runs r ON r.id = tc.run_id
             JOIN threads t ON t.id = r.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY tc.created_at DESC
             LIMIT 80",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let status: String = row.get(3)?;
            let input: Option<String> = row.get(4)?;
            let updated_at: i64 = row.get(5)?;
            let search_text = compact_search_text(&[&name, &kind, &status], &[input.as_ref()]);
            Ok(ReferenceTargetSearchResult {
                target_type: "tool".to_string(),
                target_id: id,
                title: name,
                subtitle: Some(format!("{kind} · {status}")),
                search_text: Some(search_text),
                updated_at,
            })
        })
        .map_err(|error| error.to_string())?;
    collect_matching_targets(rows, query, results)
}

fn search_approval_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.title, a.kind, a.status, a.summary, a.requested_action, a.updated_at
             FROM approval_requests a
             JOIN threads t ON t.id = a.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY a.updated_at DESC
             LIMIT 80",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let status: String = row.get(3)?;
            let summary: Option<String> = row.get(4)?;
            let requested_action: Option<String> = row.get(5)?;
            let updated_at: i64 = row.get(6)?;
            let search_text = compact_search_text(
                &[&title, &kind, &status],
                &[summary.as_ref(), requested_action.as_ref()],
            );
            Ok(ReferenceTargetSearchResult {
                target_type: "approval".to_string(),
                target_id: id,
                title,
                subtitle: Some(format!("{kind} · {status}")),
                search_text: Some(search_text),
                updated_at,
            })
        })
        .map_err(|error| error.to_string())?;
    collect_matching_targets(rows, query, results)
}

fn search_review_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.title, c.status, c.summary, c.files_changed,
                    c.additions, c.deletions, c.updated_at
             FROM review_changesets c
             JOIN threads t ON t.id = c.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY c.updated_at DESC
             LIMIT 80",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let status: String = row.get(2)?;
            let summary: Option<String> = row.get(3)?;
            let files_changed: i64 = row.get(4)?;
            let additions: i64 = row.get(5)?;
            let deletions: i64 = row.get(6)?;
            let updated_at: i64 = row.get(7)?;
            let subtitle = format!("{status} · {files_changed} files · +{additions} -{deletions}");
            let search_text =
                compact_search_text(&[&title, &status, &subtitle], &[summary.as_ref()]);
            Ok(ReferenceTargetSearchResult {
                target_type: "review".to_string(),
                target_id: id,
                title,
                subtitle: Some(subtitle),
                search_text: Some(search_text),
                updated_at,
            })
        })
        .map_err(|error| error.to_string())?;
    collect_matching_targets(rows, query, results)
}

fn search_research_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.title, r.type, r.source_uri, r.summary, r.updated_at
             FROM research_resources r
             JOIN research_collections c ON c.id = r.collection_id
             WHERE c.workspace_id = ?1
             ORDER BY r.updated_at DESC
             LIMIT 80",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let resource_type: String = row.get(2)?;
            let source_uri: Option<String> = row.get(3)?;
            let summary: Option<String> = row.get(4)?;
            let updated_at: i64 = row.get(5)?;
            let search_text = compact_search_text(
                &[&title, &resource_type],
                &[source_uri.as_ref(), summary.as_ref()],
            );
            Ok(ReferenceTargetSearchResult {
                target_type: "research".to_string(),
                target_id: id,
                title,
                subtitle: source_uri.or(Some(resource_type)),
                search_text: Some(search_text),
                updated_at,
            })
        })
        .map_err(|error| error.to_string())?;
    collect_matching_targets(rows, query, results)
}

fn collect_matching_targets(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<ReferenceTargetSearchResult>,
    >,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), String> {
    for row in rows {
        let result = row.map_err(|error| error.to_string())?;
        if reference_matches(&result, query) {
            results.push(result);
        }
    }
    Ok(())
}

fn reference_matches(result: &ReferenceTargetSearchResult, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    [
        result.target_type.as_str(),
        result.target_id.as_str(),
        result.title.as_str(),
        result.subtitle.as_deref().unwrap_or_default(),
        result.search_text.as_deref().unwrap_or_default(),
    ]
    .join("\n")
    .to_ascii_lowercase()
    .contains(query)
}

fn compact_search_text(required: &[&str], optional: &[Option<&String>]) -> String {
    required
        .iter()
        .map(|value| (*value).to_string())
        .chain(
            optional
                .iter()
                .filter_map(|value| value.map(|text| text.to_string())),
        )
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn resolved_reference<T: serde::Serialize>(
    target_type: String,
    target_id: String,
    value: T,
) -> ResolvedMarkdownReference {
    ResolvedMarkdownReference {
        target_type,
        target_id,
        status: "resolved".to_string(),
        data: serde_json::to_value(value).ok(),
        error: None,
    }
}

fn missing_reference(
    target_type: String,
    target_id: String,
    error: &str,
) -> ResolvedMarkdownReference {
    ResolvedMarkdownReference {
        target_type,
        target_id,
        status: "missing".to_string(),
        data: None,
        error: Some(error.to_string()),
    }
}

fn failed_reference(
    target_type: String,
    target_id: String,
    error: String,
) -> ResolvedMarkdownReference {
    ResolvedMarkdownReference {
        target_type,
        target_id,
        status: "missing".to_string(),
        data: None,
        error: Some(error),
    }
}

pub fn sync_message_markdown_references(
    conn: &Connection,
    message_id: &str,
    thread_id: &str,
    content: &str,
) -> Result<(), String> {
    let references = extract_markdown_references(content);
    conn.execute(
        "DELETE FROM object_references
         WHERE source_type = 'message' AND source_id = ?1",
        params![message_id],
    )
    .map_err(|error| error.to_string())?;

    if references.is_empty() {
        return Ok(());
    }

    let workspace_id: String = conn
        .query_row(
            "SELECT workspace_id FROM threads WHERE id = ?1",
            params![thread_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    let now = now_millis();
    for reference in references {
        if let Some(target) = resolve_reference_target_metadata(conn, &reference, &workspace_id)? {
            let reference_target_id =
                upsert_reference_target(conn, &reference, target, &workspace_id, now)?;
            conn.execute(
                "INSERT INTO object_references (
                     id, source_type, source_id, reference_target_id, created_at
                 ) VALUES (?1, 'message', ?2, ?3, ?4)",
                params![
                    create_id("object_ref"),
                    message_id,
                    reference_target_id,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
struct MarkdownObjectReference {
    target_id: String,
    target_type: String,
}

#[derive(Debug)]
struct ReferenceTargetMetadata {
    search_text: Option<String>,
    subtitle: Option<String>,
    title: String,
}

fn extract_markdown_references(content: &str) -> Vec<MarkdownObjectReference> {
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
        "approval" | "artifact" | "research" | "review" | "run" | "tool" => Some(normalized),
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

fn resolve_reference_target_metadata(
    conn: &Connection,
    reference: &MarkdownObjectReference,
    workspace_id: &str,
) -> Result<Option<ReferenceTargetMetadata>, String> {
    match reference.target_type.as_str() {
        "artifact" => conn
            .query_row(
                "SELECT title, type, path, summary
                 FROM artifacts
                 WHERE id = ?1
                   AND workspace_id = ?2
                   AND deleted_at IS NULL",
                params![reference.target_id, workspace_id],
                |row| {
                    let title: String = row.get(0)?;
                    let artifact_type: String = row.get(1)?;
                    let path: Option<String> = row.get(2)?;
                    let summary: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [Some(title.clone()), path.clone(), summary.clone()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        subtitle: path.or(Some(artifact_type)),
                        title,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string()),
        "run" => conn
            .query_row(
                "SELECT id, status, model_id, error_message
                 FROM runs
                 WHERE id = ?1
                   AND thread_id IN (
                     SELECT id FROM threads WHERE workspace_id = ?2
                   )",
                params![reference.target_id, workspace_id],
                |row| {
                    let id: String = row.get(0)?;
                    let status: String = row.get(1)?;
                    let model_id: Option<String> = row.get(2)?;
                    let error_message: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [
                                Some(id.clone()),
                                Some(status.clone()),
                                model_id.clone(),
                                error_message.clone(),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ),
                        subtitle: model_id.or(Some(status)),
                        title: format!("Run {}", short_id(&id)),
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string()),
        "tool" => conn
            .query_row(
                "SELECT tc.name, tc.kind, tc.status, tc.input
                 FROM tool_calls tc
                 JOIN runs r ON r.id = tc.run_id
                 JOIN threads t ON t.id = r.thread_id
                 WHERE tc.id = ?1 AND t.workspace_id = ?2",
                params![reference.target_id, workspace_id],
                |row| {
                    let name: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let status: String = row.get(2)?;
                    let input: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [
                                Some(name.clone()),
                                Some(kind.clone()),
                                Some(status.clone()),
                                input,
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ),
                        subtitle: Some(format!("{kind} · {status}")),
                        title: name,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string()),
        "approval" => conn
            .query_row(
                "SELECT title, kind, status, summary, requested_action
                 FROM approval_requests
                 WHERE id = ?1
                   AND thread_id IN (
                     SELECT id FROM threads WHERE workspace_id = ?2
                   )",
                params![reference.target_id, workspace_id],
                |row| {
                    let title: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let status: String = row.get(2)?;
                    let summary: Option<String> = row.get(3)?;
                    let requested_action: Option<String> = row.get(4)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [
                                Some(title.clone()),
                                Some(kind.clone()),
                                Some(status.clone()),
                                summary,
                                requested_action,
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ),
                        subtitle: Some(format!("{kind} · {status}")),
                        title,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string()),
        "review" => conn
            .query_row(
                "SELECT title, status, summary, files_changed, additions, deletions
                 FROM review_changesets
                 WHERE id = ?1
                   AND thread_id IN (
                     SELECT id FROM threads WHERE workspace_id = ?2
                   )",
                params![reference.target_id, workspace_id],
                |row| {
                    let title: String = row.get(0)?;
                    let status: String = row.get(1)?;
                    let summary: Option<String> = row.get(2)?;
                    let files_changed: i64 = row.get(3)?;
                    let additions: i64 = row.get(4)?;
                    let deletions: i64 = row.get(5)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [Some(title.clone()), Some(status.clone()), summary]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        subtitle: Some(format!(
                            "{status} · {files_changed} files · +{additions} -{deletions}"
                        )),
                        title,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string()),
        "research" => conn
            .query_row(
                "SELECT r.title, r.type, r.source_uri, r.summary
                 FROM research_resources r
                 JOIN research_collections c ON c.id = r.collection_id
                 WHERE r.id = ?1 AND c.workspace_id = ?2",
                params![reference.target_id, workspace_id],
                |row| {
                    let title: String = row.get(0)?;
                    let resource_type: String = row.get(1)?;
                    let source_uri: Option<String> = row.get(2)?;
                    let summary: Option<String> = row.get(3)?;
                    Ok(ReferenceTargetMetadata {
                        search_text: Some(
                            [Some(title.clone()), source_uri.clone(), summary]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        subtitle: source_uri.or(Some(resource_type)),
                        title,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string()),
        _ => Ok(None),
    }
}

fn get_artifact_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ArtifactRecord>, String> {
    conn.query_row(
        "SELECT id, workspace_id, thread_id, run_id, title, type, path, content,
                content_storage, summary, created_at, updated_at, deleted_at
         FROM artifacts
         WHERE id = ?1 AND workspace_id = ?2 AND deleted_at IS NULL",
        params![id, workspace_id],
        artifact_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn get_run_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<RunRecord>, String> {
    conn.query_row(
        "SELECT r.id, r.thread_id, r.trigger_message_id, r.status, r.model_provider, r.model_id,
                r.started_at, r.ended_at, r.error_message, r.created_at, r.updated_at
         FROM runs r
         JOIN threads t ON t.id = r.thread_id
         WHERE r.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        run_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn get_tool_call_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ToolCallRecord>, String> {
    conn.query_row(
        "SELECT tc.id, tc.run_id, tc.name, tc.kind, tc.input, tc.status,
                tc.started_at, tc.ended_at, tc.created_at
         FROM tool_calls tc
         JOIN runs r ON r.id = tc.run_id
         JOIN threads t ON t.id = r.thread_id
         WHERE tc.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        tool_call_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn get_approval_request_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ApprovalRequestRecord>, String> {
    conn.query_row(
        "SELECT a.id, a.thread_id, a.run_id, a.tool_call_id, a.kind, a.status,
                a.title, a.summary, a.risk_level, a.requested_action, a.decision_note,
                a.decided_at, a.created_at, a.updated_at
         FROM approval_requests a
         JOIN threads t ON t.id = a.thread_id
         WHERE a.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        approval_request_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn get_review_changeset_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ReviewChangesetRecord>, String> {
    conn.query_row(
        "SELECT c.id, c.thread_id, c.run_id, c.tool_call_id, c.title, c.summary, c.status,
                c.files_changed, c.additions, c.deletions, c.created_at, c.updated_at
         FROM review_changesets c
         JOIN threads t ON t.id = c.thread_id
         WHERE c.id = ?1 AND t.workspace_id = ?2",
        params![id, workspace_id],
        review_changeset_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn get_research_resource_in_workspace(
    conn: &Connection,
    workspace_id: &str,
    id: &str,
) -> Result<Option<ResearchResourceRecord>, String> {
    conn.query_row(
        "SELECT r.id, r.collection_id, c.workspace_id, r.source_artifact_id, r.title,
                r.type, r.source_uri, r.content, r.content_storage, r.summary,
                r.metadata, r.created_at, r.updated_at
         FROM research_resources r
         JOIN research_collections c ON c.id = r.collection_id
         WHERE r.id = ?1 AND c.workspace_id = ?2",
        params![id, workspace_id],
        research_resource_from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn upsert_reference_target(
    conn: &Connection,
    reference: &MarkdownObjectReference,
    metadata: ReferenceTargetMetadata,
    workspace_id: &str,
    now: i64,
) -> Result<String, String> {
    let existing_id: Option<String> = conn
        .query_row(
            "SELECT id
             FROM reference_targets
             WHERE target_type = ?1
               AND target_id = ?2
               AND scope = 'workspace'
               AND workspace_id = ?3
             LIMIT 1",
            params![reference.target_type, reference.target_id, workspace_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if let Some(existing_id) = existing_id {
        conn.execute(
            "UPDATE reference_targets
             SET title = ?1, subtitle = ?2, search_text = ?3, updated_at = ?4
             WHERE id = ?5",
            params![
                metadata.title,
                metadata.subtitle,
                metadata.search_text,
                now,
                existing_id
            ],
        )
        .map_err(|error| error.to_string())?;
        return Ok(existing_id);
    }

    let id = create_id("ref_target");
    conn.execute(
        "INSERT INTO reference_targets (
             id, target_type, target_id, scope, workspace_id, title, subtitle,
             search_text, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'workspace', ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id,
            reference.target_type,
            reference.target_id,
            workspace_id,
            metadata.title,
            metadata.subtitle,
            metadata.search_text,
            now
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(id)
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::INITIAL_SCHEMA;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(INITIAL_SCHEMA)
            .expect("initialize test schema");
        conn
    }

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
    fn syncs_message_references_into_reference_tables() {
        let conn = test_conn();
        seed_workspace_artifact(&conn);

        sync_message_markdown_references(
            &conn,
            "msg_test",
            "thread_test",
            "[Poem](futureos://artifact/artifact_test)",
        )
        .expect("sync markdown references");

        let target_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM reference_targets", [], |row| {
                row.get(0)
            })
            .expect("count reference targets");
        let object_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM object_references", [], |row| {
                row.get(0)
            })
            .expect("count object references");
        let title: String = conn
            .query_row("SELECT title FROM reference_targets", [], |row| row.get(0))
            .expect("load target title");

        assert_eq!(target_count, 1);
        assert_eq!(object_count, 1);
        assert_eq!(title, "Poem");
    }

    #[test]
    fn searches_workspace_reference_targets_from_objects() {
        let conn = test_conn();
        seed_workspace_artifact(&conn);

        let mut results = vec![];
        search_artifact_targets(&conn, "ws_test", "poem", &mut results)
            .expect("search artifact targets");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_type, "artifact");
        assert_eq!(results[0].target_id, "artifact_test");
        assert_eq!(results[0].title, "Poem");
    }

    #[test]
    fn resolves_references_with_workspace_scope_and_deleted_filter() {
        let conn = test_conn();
        seed_workspace_artifact(&conn);
        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws_other', 'Other', 'temporary', '/tmp/other', 'active', 1, 1)",
            [],
        )
        .expect("insert other workspace");
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 created_at, updated_at
             ) VALUES (
                 'thread_other', 'ws_other', 'chat', 'Other', 'active', 0, 0, 1, 1
             )",
            [],
        )
        .expect("insert other thread");
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, type, created_at, updated_at
             ) VALUES (
                 'artifact_other', 'ws_other', 'thread_other', 'Other Poem', 'document', 1, 1
             )",
            [],
        )
        .expect("insert other artifact");
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, type, created_at, updated_at, deleted_at
             ) VALUES (
                 'artifact_deleted', 'ws_test', 'thread_test', 'Deleted Poem',
                 'document', 1, 1, 2
             )",
            [],
        )
        .expect("insert deleted artifact");

        let resolved = [
            ("artifact_test", "resolved"),
            ("artifact_other", "missing"),
            ("artifact_deleted", "missing"),
        ]
        .into_iter()
        .map(|(target_id, expected_status)| {
            let resolved = resolve_markdown_reference(
                &conn,
                "ws_test",
                MarkdownReferenceInput {
                    target_id: target_id.to_string(),
                    target_type: "artifact".to_string(),
                },
            );
            (resolved.status, expected_status)
        })
        .collect::<Vec<_>>();

        assert_eq!(
            resolved,
            vec![
                ("resolved".to_string(), "resolved"),
                ("missing".to_string(), "missing"),
                ("missing".to_string(), "missing"),
            ]
        );
    }

    #[test]
    fn percent_decodes_utf8_reference_ids() {
        assert_eq!(percent_decode("%E8%AF%97"), "诗");
        assert_eq!(percent_decode("%E0%A4%A"), "%E0%A4%A");
    }

    fn seed_workspace_artifact(conn: &Connection) {
        conn.execute(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws_test', 'Test', 'temporary', '/tmp/test', 'active', 1, 1)",
            [],
        )
        .expect("insert workspace");
        conn.execute(
            "INSERT INTO threads (
                 id, workspace_id, mode, title, status, pinned, readonly,
                 created_at, updated_at
             ) VALUES (
                 'thread_test', 'ws_test', 'chat', 'Thread', 'active', 0, 0, 1, 1
             )",
            [],
        )
        .expect("insert thread");
        conn.execute(
            "INSERT INTO artifacts (
                 id, workspace_id, thread_id, title, type, path, summary,
                 created_at, updated_at
             ) VALUES (
                 'artifact_test', 'ws_test', 'thread_test', 'Poem', 'document',
                 'poem.md', 'Saved poem', 1, 1
             )",
            [],
        )
        .expect("insert artifact");
    }
}
