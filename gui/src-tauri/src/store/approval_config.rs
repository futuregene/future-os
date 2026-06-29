#![allow(dead_code)]
//! Approval configuration CRUD operations (stub)
//!
//! These functions provide infrastructure for future sandbox configuration,
//! approval policy configuration, and approval rules. They are not yet
//! exposed as Tauri commands and are not called by the agent or GUI.
//!
//! Future work: wire these up to Settings UI and agent policy evaluation.

use rusqlite::{params, OptionalExtension};

use super::records::*;

use super::connect;

// ─── Sandbox Configuration ─────────────────────────────────────────────

pub fn get_sandbox_config(
    workspace_id: Option<&str>,
) -> Result<Option<SandboxConfigRecord>, crate::AppError> {
    let conn = connect()?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(SandboxConfigRecord {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            mode: row.get(2)?,
            writable_roots: row.get(3)?,
            network_access: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    };
    // The `None` branch has no `?1` placeholder, so it must bind no parameter.
    match workspace_id {
        Some(ws) => conn.query_row(
            "SELECT id, workspace_id, mode, writable_roots, network_access, created_at, updated_at FROM sandbox_config WHERE workspace_id = ?1",
            params![ws],
            map_row,
        ),
        None => conn.query_row(
            "SELECT id, workspace_id, mode, writable_roots, network_access, created_at, updated_at FROM sandbox_config WHERE workspace_id IS NULL",
            [],
            map_row,
        ),
    }
    .optional()
    .map_err(crate::AppError::from)
}

pub fn upsert_sandbox_config(config: &SandboxConfigRecord) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute(
        "INSERT INTO sandbox_config (id, workspace_id, mode, writable_roots, network_access, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
           mode = excluded.mode,
           writable_roots = excluded.writable_roots,
           network_access = excluded.network_access,
           updated_at = excluded.updated_at",
        params![
            config.id,
            config.workspace_id,
            config.mode,
            config.writable_roots,
            config.network_access,
            config.created_at,
            config.updated_at,
        ],
    )
    ?;
    Ok(())
}

// ─── Approval Policy Configuration ─────────────────────────────────────

pub fn get_approval_policy_config(
    workspace_id: Option<&str>,
) -> Result<Option<ApprovalPolicyConfigRecord>, crate::AppError> {
    let conn = connect()?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(ApprovalPolicyConfigRecord {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            policy: row.get(2)?,
            reviewer: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    };
    // The `None` branch has no `?1` placeholder, so it must bind no parameter.
    match workspace_id {
        Some(ws) => conn.query_row(
            "SELECT id, workspace_id, policy, reviewer, created_at, updated_at FROM approval_policy_config WHERE workspace_id = ?1",
            params![ws],
            map_row,
        ),
        None => conn.query_row(
            "SELECT id, workspace_id, policy, reviewer, created_at, updated_at FROM approval_policy_config WHERE workspace_id IS NULL",
            [],
            map_row,
        ),
    }
    .optional()
    .map_err(crate::AppError::from)
}

pub fn upsert_approval_policy_config(
    config: &ApprovalPolicyConfigRecord,
) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute(
        "INSERT INTO approval_policy_config (id, workspace_id, policy, reviewer, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
           policy = excluded.policy,
           reviewer = excluded.reviewer,
           updated_at = excluded.updated_at",
        params![
            config.id,
            config.workspace_id,
            config.policy,
            config.reviewer,
            config.created_at,
            config.updated_at,
        ],
    )
    ?;
    Ok(())
}

// ─── Approval Rules ────────────────────────────────────────────────────

pub fn list_approval_rules(
    workspace_id: Option<&str>,
) -> Result<Vec<ApprovalRuleRecord>, crate::AppError> {
    let conn = connect()?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(ApprovalRuleRecord {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            scope: row.get(2)?,
            match_kind: row.get(3)?,
            match_value: row.get(4)?,
            decision: row.get(5)?,
            enabled: row.get(6)?,
            created_at: row.get(7)?,
            expires_at: row.get(8)?,
        })
    };
    // The `None` branch has no `?1` placeholder, so it must bind no parameter.
    let query = match workspace_id {
        Some(_) => "SELECT id, workspace_id, scope, match_kind, match_value, decision, enabled, created_at, expires_at FROM approval_rules WHERE workspace_id = ?1 ORDER BY created_at",
        None => "SELECT id, workspace_id, scope, match_kind, match_value, decision, enabled, created_at, expires_at FROM approval_rules WHERE workspace_id IS NULL ORDER BY created_at",
    };
    let mut stmt = conn.prepare(query)?;
    let rows = match workspace_id {
        Some(ws) => stmt.query_map(params![ws], map_row)?,
        None => stmt.query_map([], map_row)?,
    };
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(crate::AppError::from)
}

pub fn insert_approval_rule(rule: &ApprovalRuleRecord) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute(
        "INSERT INTO approval_rules (id, workspace_id, scope, match_kind, match_value, decision, enabled, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            rule.id,
            rule.workspace_id,
            rule.scope,
            rule.match_kind,
            rule.match_value,
            rule.decision,
            rule.enabled,
            rule.created_at,
            rule.expires_at,
        ],
    )
    ?;
    Ok(())
}

pub fn delete_approval_rule(id: &str) -> Result<(), crate::AppError> {
    let conn = connect()?;
    conn.execute("DELETE FROM approval_rules WHERE id = ?1", params![id])?;
    Ok(())
}
