#![allow(dead_code)]
//! Approval configuration CRUD operations (stub)
//!
//! These functions provide infrastructure for future sandbox configuration,
//! approval policy configuration, and approval rules. They are not yet
//! exposed as Tauri commands and are not called by the agent or GUI.
//!
//! Future work: wire these up to Settings UI and agent policy evaluation.

use rusqlite::{params, OptionalExtension};

use super::models::*;

use super::{connect, initialize_app_store};

// ─── Sandbox Configuration ─────────────────────────────────────────────

pub fn get_sandbox_config(
    workspace_id: Option<&str>,
) -> Result<Option<SandboxConfigRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let query = match workspace_id {
        Some(_) => "SELECT id, workspace_id, mode, writable_roots, network_access, created_at, updated_at FROM sandbox_config WHERE workspace_id = ?1",
        None => "SELECT id, workspace_id, mode, writable_roots, network_access, created_at, updated_at FROM sandbox_config WHERE workspace_id IS NULL",
    };
    conn.query_row(query, params![workspace_id], |row| {
        Ok(SandboxConfigRecord {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            mode: row.get(2)?,
            writable_roots: row.get(3)?,
            network_access: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })
    .optional()
    .map_err(|e| e.to_string())
}

pub fn upsert_sandbox_config(config: &SandboxConfigRecord) -> Result<(), String> {
    initialize_app_store()?;
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
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Approval Policy Configuration ─────────────────────────────────────

pub fn get_approval_policy_config(
    workspace_id: Option<&str>,
) -> Result<Option<ApprovalPolicyConfigRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let query = match workspace_id {
        Some(_) => "SELECT id, workspace_id, policy, reviewer, created_at, updated_at FROM approval_policy_config WHERE workspace_id = ?1",
        None => "SELECT id, workspace_id, policy, reviewer, created_at, updated_at FROM approval_policy_config WHERE workspace_id IS NULL",
    };
    conn.query_row(query, params![workspace_id], |row| {
        Ok(ApprovalPolicyConfigRecord {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            policy: row.get(2)?,
            reviewer: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })
    .optional()
    .map_err(|e| e.to_string())
}

pub fn upsert_approval_policy_config(config: &ApprovalPolicyConfigRecord) -> Result<(), String> {
    initialize_app_store()?;
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
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Approval Rules ────────────────────────────────────────────────────

pub fn list_approval_rules(workspace_id: Option<&str>) -> Result<Vec<ApprovalRuleRecord>, String> {
    initialize_app_store()?;
    let conn = connect()?;
    let query = match workspace_id {
        Some(_) => "SELECT id, workspace_id, scope, match_kind, match_value, decision, enabled, created_at, expires_at FROM approval_rules WHERE workspace_id = ?1 ORDER BY created_at",
        None => "SELECT id, workspace_id, scope, match_kind, match_value, decision, enabled, created_at, expires_at FROM approval_rules WHERE workspace_id IS NULL ORDER BY created_at",
    };
    let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
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
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn insert_approval_rule(rule: &ApprovalRuleRecord) -> Result<(), String> {
    initialize_app_store()?;
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
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_approval_rule(id: &str) -> Result<(), String> {
    initialize_app_store()?;
    let conn = connect()?;
    conn.execute("DELETE FROM approval_rules WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
