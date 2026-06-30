//! Searches workspace objects (artifacts, runs, tool calls, approvals, reviews,
//! research resources) and surfaces them as reference-target pick-list results
//! for the `@`-mention autocomplete. Each object kind is queried separately,
//! filtered by a case-insensitive substring match, then merged and ranked by
//! recency.

use rusqlite::{params, Connection};

use crate::store::db::connect;
use crate::store::records::{ReferenceTargetSearchResult, SearchReferenceTargetsInput};

use super::metadata::{
    approval_metadata, artifact_metadata, research_metadata, review_metadata, run_metadata,
    tool_metadata,
};

pub fn search_reference_targets(
    input: SearchReferenceTargetsInput,
) -> Result<Vec<ReferenceTargetSearchResult>, crate::AppError> {
    let conn = connect()?;
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

pub(super) fn search_artifact_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), crate::AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, title, type, path, summary, updated_at
             FROM artifacts
             WHERE workspace_id = ?1 AND deleted_at IS NULL
             ORDER BY updated_at DESC
             LIMIT 80",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        let id: String = row.get(0)?;
        let updated_at: i64 = row.get(5)?;
        let meta = artifact_metadata(row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?);
        Ok(ReferenceTargetSearchResult {
            target_type: "artifact".to_string(),
            target_id: id,
            title: meta.title,
            subtitle: meta.subtitle,
            search_text: meta.search_text,
            updated_at,
        })
    })?;
    collect_matching_targets(rows, query, results)
}

fn search_run_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), crate::AppError> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.status, r.model_id, r.error_message, r.updated_at
             FROM runs r
             JOIN threads t ON t.id = r.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY r.updated_at DESC
             LIMIT 80",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        let id: String = row.get(0)?;
        let updated_at: i64 = row.get(4)?;
        let meta = run_metadata(&id, row.get(1)?, row.get(2)?, row.get(3)?);
        Ok(ReferenceTargetSearchResult {
            target_type: "run".to_string(),
            target_id: id,
            title: meta.title,
            subtitle: meta.subtitle,
            search_text: meta.search_text,
            updated_at,
        })
    })?;
    collect_matching_targets(rows, query, results)
}

fn search_tool_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), crate::AppError> {
    let mut stmt = conn.prepare(
        "SELECT tc.id, tc.name, tc.kind, tc.status, tc.input, tc.created_at
             FROM tool_calls tc
             JOIN runs r ON r.id = tc.run_id
             JOIN threads t ON t.id = r.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY tc.created_at DESC
             LIMIT 80",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        let id: String = row.get(0)?;
        let updated_at: i64 = row.get(5)?;
        let meta = tool_metadata(row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?);
        Ok(ReferenceTargetSearchResult {
            target_type: "tool".to_string(),
            target_id: id,
            title: meta.title,
            subtitle: meta.subtitle,
            search_text: meta.search_text,
            updated_at,
        })
    })?;
    collect_matching_targets(rows, query, results)
}

fn search_approval_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), crate::AppError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.title, a.kind, a.status, a.summary, a.requested_action, a.updated_at
             FROM approval_requests a
             JOIN threads t ON t.id = a.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY a.updated_at DESC
             LIMIT 80",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        let id: String = row.get(0)?;
        let updated_at: i64 = row.get(6)?;
        let meta = approval_metadata(
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        );
        Ok(ReferenceTargetSearchResult {
            target_type: "approval".to_string(),
            target_id: id,
            title: meta.title,
            subtitle: meta.subtitle,
            search_text: meta.search_text,
            updated_at,
        })
    })?;
    collect_matching_targets(rows, query, results)
}

fn search_review_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), crate::AppError> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.title, c.status, c.summary, c.files_changed,
                    c.additions, c.deletions, c.updated_at
             FROM review_changesets c
             JOIN threads t ON t.id = c.thread_id
             WHERE t.workspace_id = ?1
             ORDER BY c.updated_at DESC
             LIMIT 80",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        let id: String = row.get(0)?;
        let updated_at: i64 = row.get(7)?;
        let meta = review_metadata(
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
        );
        Ok(ReferenceTargetSearchResult {
            target_type: "review".to_string(),
            target_id: id,
            title: meta.title,
            subtitle: meta.subtitle,
            search_text: meta.search_text,
            updated_at,
        })
    })?;
    collect_matching_targets(rows, query, results)
}

fn search_research_targets(
    conn: &Connection,
    workspace_id: &str,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), crate::AppError> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.title, r.type, r.source_uri, r.summary, r.updated_at
             FROM research_resources r
             JOIN research_collections c ON c.id = r.collection_id
             WHERE c.workspace_id = ?1
             ORDER BY r.updated_at DESC
             LIMIT 80",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        let id: String = row.get(0)?;
        let updated_at: i64 = row.get(5)?;
        let meta = research_metadata(row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?);
        Ok(ReferenceTargetSearchResult {
            target_type: "research".to_string(),
            target_id: id,
            title: meta.title,
            subtitle: meta.subtitle,
            search_text: meta.search_text,
            updated_at,
        })
    })?;
    collect_matching_targets(rows, query, results)
}

fn collect_matching_targets(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<ReferenceTargetSearchResult>,
    >,
    query: &str,
    results: &mut Vec<ReferenceTargetSearchResult>,
) -> Result<(), crate::AppError> {
    for row in rows {
        let result = row?;
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
