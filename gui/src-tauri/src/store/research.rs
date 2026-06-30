use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::artifacts::get_artifact;
use super::db::*;
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchCollectionRecord {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchResourceRecord {
    pub id: String,
    pub collection_id: String,
    pub workspace_id: String,
    pub source_artifact_id: Option<String>,
    pub title: String,
    pub resource_type: String,
    pub source_uri: Option<String>,
    pub content: Option<String>,
    pub content_storage: Option<String>,
    pub summary: Option<String>,
    pub metadata: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Column list for `research_collection_from_row`, in struct order.
pub(super) const RESEARCH_COLLECTION_COLUMNS: &str =
    "id, workspace_id, name, description, created_at, updated_at";

pub(super) fn research_collection_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ResearchCollectionRecord> {
    Ok(ResearchCollectionRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

/// Column list for `research_resource_from_row`, in struct order. Aliases are
/// baked in because every `SELECT` joins `research_resources r` onto
/// `research_collections c` and pulls `workspace_id` from the collection — so
/// unlike the other tables this list can't be re-prefixed with a single alias.
pub(super) const RESEARCH_RESOURCE_COLUMNS: &str =
    "r.id, r.collection_id, c.workspace_id, r.source_artifact_id, r.title, \
     r.resource_type, r.source_uri, r.content, r.content_storage, r.summary, \
     r.metadata, r.created_at, r.updated_at";

pub(super) fn research_resource_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ResearchResourceRecord> {
    Ok(ResearchResourceRecord {
        id: row.get(0)?,
        collection_id: row.get(1)?,
        workspace_id: row.get(2)?,
        source_artifact_id: row.get(3)?,
        title: row.get(4)?,
        resource_type: row.get(5)?,
        source_uri: row.get(6)?,
        content: row.get(7)?,
        content_storage: row.get(8)?,
        summary: row.get(9)?,
        metadata: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

pub fn list_research_resources(
    workspace_id: &str,
) -> Result<Vec<ResearchResourceRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {RESEARCH_RESOURCE_COLUMNS}
             FROM research_resources r
             JOIN research_collections c ON c.id = r.collection_id
             WHERE c.workspace_id = ?1
             ORDER BY r.created_at DESC"
    ))?;
    let rows = stmt.query_map(params![workspace_id], research_resource_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn promote_artifact_to_research(
    artifact_id: &str,
) -> Result<ResearchResourceRecord, crate::AppError> {
    let artifact = loaded(get_artifact(artifact_id)?, "Artifact")?;
    if artifact.deleted_at.is_some() {
        return Err("deleted artifacts cannot be added to Research."
            .to_string()
            .into());
    }

    let collection = get_or_create_default_research_collection(&artifact.workspace_id)?;
    let conn = connect()?;
    let existing = conn
        .query_row(
            &format!(
                "SELECT {RESEARCH_RESOURCE_COLUMNS}
             FROM research_resources r
             JOIN research_collections c ON c.id = r.collection_id
             WHERE r.source_artifact_id = ?1
               AND c.workspace_id = ?2
             LIMIT 1"
            ),
            params![artifact.id, artifact.workspace_id],
            research_resource_from_row,
        )
        .optional()?;
    if let Some(resource) = existing {
        return Ok(resource);
    }

    let id = create_id("research");
    let now = now_millis();
    conn.execute(
        "INSERT INTO research_resources (
             id, collection_id, source_artifact_id, title, resource_type, source_uri,
             content, content_storage, summary, metadata, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
        params![
            id,
            collection.id,
            artifact.id,
            artifact.title,
            artifact.artifact_type,
            artifact.path,
            artifact.content,
            artifact.content_storage,
            artifact.summary,
            None::<String>,
            now
        ],
    )?;

    loaded(get_research_resource(&id)?, "Created research resource")
}

fn get_research_resource(id: &str) -> Result<Option<ResearchResourceRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {RESEARCH_RESOURCE_COLUMNS}
         FROM research_resources r
         JOIN research_collections c ON c.id = r.collection_id
         WHERE r.id = ?1"
        ),
        params![id],
        research_resource_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

fn get_or_create_default_research_collection(
    workspace_id: &str,
) -> Result<ResearchCollectionRecord, crate::AppError> {
    let conn = connect()?;
    let existing = conn
        .query_row(
            &format!(
                "SELECT {RESEARCH_COLLECTION_COLUMNS}
             FROM research_collections
             WHERE workspace_id = ?1 AND name = 'Research'
             LIMIT 1"
            ),
            params![workspace_id],
            research_collection_from_row,
        )
        .optional()?;
    if let Some(collection) = existing {
        return Ok(collection);
    }

    let id = create_id("research_collection");
    let now = now_millis();
    conn.execute(
        "INSERT INTO research_collections (
             id, workspace_id, name, description, created_at, updated_at
         ) VALUES (?1, ?2, 'Research', ?3, ?4, ?4)",
        params![
            id,
            workspace_id,
            Some("Default research resources".to_string()),
            now
        ],
    )?;

    conn.query_row(
        &format!("SELECT {RESEARCH_COLLECTION_COLUMNS} FROM research_collections WHERE id = ?1"),
        params![id],
        research_collection_from_row,
    )
    .map_err(crate::AppError::from)
}
