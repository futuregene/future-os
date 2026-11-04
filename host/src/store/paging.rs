//! Keyset pagination for the conversation and workspace lists.
//!
//! The sidebar used to load every row (see `list_threads`/`list_workspaces`);
//! this module answers one page at a time for the three list kinds the rail
//! draws — the chat section, one workspace's conversations, and the workspace
//! list itself — plus the archived view.
//!
//! **Why a cursor and not an offset**: the list is continuously reordered by
//! `last_message_at` pushes, so an offset page would duplicate or drop rows
//! between requests. The cursor encodes the sort key itself
//! `(activity_ts, id)` and is compared as a row value, which tolerates
//! concurrent reordering — the same property `get_session_entries_page`
//! relies on.
//!
//! **Page by roots, not by rows**: a fork child's activity stamp is usually
//! newer than its parent's, so a row-level page could return the child while
//! its parent sits in a later page and the tree could not be drawn. A page is
//! therefore a set of *root* conversations (those whose parent is outside the
//! visible set), and each page's roots are followed by one query for all of
//! their descendants. One page = several complete fork trees.

use rusqlite::params;

use super::db::*;
use super::threads::{thread_from_row, THREAD_COLUMNS};
use super::util::qualify_columns;
use super::workspaces::workspace_from_row;
use super::WorkspaceRecord;

/// Which list a page belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadScope {
    /// The chat section: every live `mode = 'chat'` conversation that is not
    /// hidden by an archived workspace or shelved itself.
    Chat,
    /// One workspace's conversations, as the active tree draws it: only the
    /// conversations that are themselves active.
    Workspace(String),
    /// One *archived* workspace's contents, for browsing before a restore:
    /// every live conversation it holds, whatever its own status.
    ArchivedWorkspace(String),
    /// The recent view's global stream: every live conversation, regardless of
    /// section. The view splits active from finished itself; the *active*
    /// stretch reads whole from [`super::list_active_threads`].
    Recent,
    /// The archived view's conversation section: trees shelved by the user,
    /// whose workspace is not itself archived (those belong under the
    /// workspace heading, not here).
    Archived,
    /// *Every* shelved conversation, whichever half shelved it: the user
    /// shelved the conversation, or shelved the workspace holding it.
    ///
    /// The two halves are drawn apart in the archived view (a workspace's
    /// contents hang under its heading), but one machine hands both to a paired
    /// client as one set of rows marked `archived`, so reading them back needs a
    /// scope that says the same thing.
    Shelved,
}

impl ThreadScope {
    /// Parse the IPC spelling (`chat`, `recent`, `archived`, `workspace:<id>`,
    /// `workspace-archived:<id>`).
    pub fn parse(value: &str) -> Result<Self, crate::Error> {
        let value = value.trim();
        match value {
            "" | "chat" => Ok(Self::Chat),
            "recent" => Ok(Self::Recent),
            "archived" => Ok(Self::Archived),
            "shelved" => Ok(Self::Shelved),
            other => {
                if let Some(id) = other.strip_prefix("workspace-archived:") {
                    if !id.trim().is_empty() {
                        return Ok(Self::ArchivedWorkspace(id.trim().to_string()));
                    }
                }
                if let Some(id) = other.strip_prefix("workspace:") {
                    if !id.trim().is_empty() {
                        return Ok(Self::Workspace(id.trim().to_string()));
                    }
                }
                Err(format!("Unknown conversation scope: {value}").into())
            }
        }
    }
}

/// One page of conversation rows, plus the cursor for the next one.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsPage {
    pub threads: Vec<super::ThreadRecord>,
    /// `None` when this page is the last one in the scope.
    pub next_cursor: Option<String>,
    /// How many roots the whole scope holds, not just this page.
    ///
    /// A cursor answers "what comes next", never "how much is left", so a
    /// reader offering to show the rest has no honest number without this. It
    /// counts *roots* under the same filter the page does (pinned rows are
    /// outside every page, so they are outside this count too).
    pub total_roots: i64,
}

/// One page of workspace rows, plus the cursor for the next one.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacesPage {
    pub workspaces: Vec<WorkspaceRecord>,
    pub next_cursor: Option<String>,
}

/// A cursor is `<activity-stamp>:<row-id>`; ids are generated without colons,
/// so the split is unambiguous, and an unparsable value falls back to the first
/// page rather than failing a list the reader can see.
type Cursor = (i64, String);

fn encode_cursor(stamp: i64, id: &str) -> String {
    format!("{stamp}:{id}")
}

fn decode_cursor(raw: &str) -> Option<Cursor> {
    let (stamp, id) = raw.split_once(':')?;
    Some((stamp.parse().ok()?, id.to_string()))
}

/// The activity stamp the lists are ordered by — identical to the ordering the
/// unpaged `list_threads` used, so switching to pages changes nothing the reader
/// can see.
fn thread_activity(alias: &str) -> String {
    format!("COALESCE({alias}.last_message_at, {alias}.updated_at, {alias}.created_at)")
}

/// The `EXISTS` fragment that decides a row is a *root*: no conversation that
/// is visible in this scope is its parent. A missing or hidden parent promotes
/// its children, exactly as `threadParentIds` does in the rail.
fn root_predicate(filter: &str) -> String {
    format!(
        "(t.parent_session_id IS NULL OR NOT EXISTS (
             SELECT 1 FROM threads p JOIN workspaces pw ON pw.id = p.workspace_id
              WHERE p.deleted_at IS NULL
                AND COALESCE(NULLIF(TRIM(p.agent_session_id), ''), p.id) = t.parent_session_id
                AND {filter_with}
         ))",
        filter_with = qualify_filter(filter, "p", "pw")
    )
}

/// Rewrite a filter written over `t`/`w` for another alias pair, so the same
/// one definition answers "is this row visible here" and "is its parent visible
/// here".
fn qualify_filter(filter: &str, thread_alias: &str, workspace_alias: &str) -> String {
    filter
        .replace("t.", &format!("{thread_alias}."))
        .replace("w.", &format!("{workspace_alias}."))
}

/// The rows a scope admits, as a single SQL fragment over `threads t` joined to
/// `workspaces w`. Pinned rows are excluded here: they are fetched whole by the
/// rail's shortcut section and never take part in a page.
fn thread_scope_filter(scope: &ThreadScope) -> String {
    match scope {
        ThreadScope::Chat => "t.status = 'active' AND t.deleted_at IS NULL
             AND t.mode = 'chat' AND w.archived_at IS NULL"
            .to_string(),
        ThreadScope::Workspace(id) => format!(
            "t.status = 'active' AND t.deleted_at IS NULL
             AND t.workspace_id = '{}' AND w.archived_at IS NULL",
            id.replace('\'', "''")
        ),
        ThreadScope::ArchivedWorkspace(id) => format!(
            "t.deleted_at IS NULL AND t.workspace_id = '{}'",
            id.replace('\'', "''")
        ),
        ThreadScope::Recent => {
            "t.status = 'active' AND t.deleted_at IS NULL AND w.archived_at IS NULL".to_string()
        }
        ThreadScope::Archived => {
            "t.status = 'archived' AND t.deleted_at IS NULL AND w.archived_at IS NULL".to_string()
        }
        ThreadScope::Shelved => "t.deleted_at IS NULL
             AND (t.status = 'archived' OR w.archived_at IS NOT NULL)"
            .to_string(),
    }
}

/// One page of conversation trees.
///
/// `limit` counts *roots* (clamped 1..=100); the returned rows also include each
/// root's visible descendants, so the row count can exceed `limit` while the
/// page stays bounded by tree size.
pub fn list_threads_page(
    scope: &ThreadScope,
    cursor: Option<&str>,
    limit: i64,
) -> Result<ThreadsPage, crate::Error> {
    let limit = limit.clamp(1, 100);
    let conn = connect()?;
    let filter = thread_scope_filter(scope);
    let activity = thread_activity("t");
    let cursor = cursor.and_then(decode_cursor);

    // Root rows only, ordered newest activity first. The cursor repeats the
    // ordering expression because a `SELECT` alias is not addressable in
    // `WHERE`.
    let cursor_clause = if cursor.is_some() {
        format!("AND ({activity}, t.id) < (?1, ?2)")
    } else {
        String::new()
    };
    let roots_sql = format!(
        "SELECT {columns}, {activity} AS activity
         FROM threads t JOIN workspaces w ON w.id = t.workspace_id
         WHERE {filter} AND t.pinned = 0 AND {root_predicate}
           {cursor_clause}
         ORDER BY activity DESC, t.id DESC
         LIMIT {limit}",
        columns = qualify_columns("t", THREAD_COLUMNS),
        root_predicate = root_predicate(&filter)
    );

    let mut stmt = conn.prepare(&roots_sql)?;
    let rows: Vec<(super::ThreadRecord, i64)> = match &cursor {
        Some((stamp, id)) => stmt
            .query_map(params![stamp, id], |row| {
                Ok((thread_from_row(row)?, row.get::<_, i64>("activity")?))
            })?
            .collect::<rusqlite::Result<_>>()?,
        None => stmt
            .query_map([], |row| {
                Ok((thread_from_row(row)?, row.get::<_, i64>("activity")?))
            })?
            .collect::<rusqlite::Result<_>>()?,
    };

    let next_cursor = if rows.len() as i64 == limit {
        rows.last()
            .map(|(thread, stamp)| encode_cursor(*stamp, &thread.id))
    } else {
        None
    };

    let root_ids: Vec<String> = rows.iter().map(|(thread, _)| thread.id.clone()).collect();
    let mut threads: Vec<super::ThreadRecord> =
        rows.into_iter().map(|(thread, _)| thread).collect();
    threads.extend(descendants_of(&conn, &root_ids, &filter)?);
    // The scope's whole root count — what a "show the rest" control can honestly
    // promise. Same filter, same root rule, no cursor: it describes the list, not
    // the page.
    let total_roots: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*)
               FROM threads t JOIN workspaces w ON w.id = t.workspace_id
              WHERE {filter} AND t.pinned = 0 AND {root_predicate}",
            root_predicate = root_predicate(&filter)
        ),
        [],
        |row| row.get(0),
    )?;
    Ok(ThreadsPage {
        threads,
        next_cursor,
        total_roots,
    })
}

/// Every visible descendant of `root_ids`, resolved through the Agent session
/// lineage (the same relation `thread_delete_closure` walks). `UNION` dedupes
/// and terminates on a malformed cycle.
fn descendants_of(
    conn: &rusqlite::Connection,
    root_ids: &[String],
    filter: &str,
) -> Result<Vec<super::ThreadRecord>, crate::Error> {
    if root_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = root_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "WITH RECURSIVE subtree(id, session_key) AS (
             SELECT t.id, COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id)
               FROM threads t WHERE t.id IN ({placeholders})
           UNION
             SELECT t.id, COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id)
               FROM threads t JOIN subtree s ON t.parent_session_id = s.session_key
              WHERE t.deleted_at IS NULL
         )
         SELECT {columns}
           FROM threads t JOIN workspaces w ON w.id = t.workspace_id
          WHERE t.id IN (SELECT id FROM subtree)
            AND t.id NOT IN ({placeholders})
            AND {filter}
          ORDER BY {activity} DESC, t.id DESC",
        columns = qualify_columns("t", THREAD_COLUMNS),
        activity = thread_activity("t")
    );
    let mut stmt = conn.prepare(&sql)?;
    let args: Vec<&dyn rusqlite::ToSql> = root_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .chain(root_ids.iter().map(|id| id as &dyn rusqlite::ToSql))
        .collect();
    let rows = stmt.query_map(args.as_slice(), thread_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The pinned conversations of a scope, whole. Pins are hand-picked shortcuts
/// and naturally few; they lead the list rather than being paged.
pub fn list_pinned_threads(scope: &ThreadScope) -> Result<Vec<super::ThreadRecord>, crate::Error> {
    let conn = connect()?;
    let filter = thread_scope_filter(scope);
    let sql = format!(
        "SELECT {columns} FROM threads t JOIN workspaces w ON w.id = t.workspace_id
         WHERE {filter} AND t.pinned = 1
         ORDER BY {activity} DESC, t.id DESC",
        columns = qualify_columns("t", THREAD_COLUMNS),
        activity = thread_activity("t")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], thread_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Whether a workspace still has any conversation the user has not shelved —
/// the fact the rail uses to decide whether an archived workspace group has
/// anything to browse.
pub fn workspace_has_live_threads(workspace_id: &str) -> Result<bool, crate::Error> {
    let conn = connect()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM threads
         WHERE workspace_id = ?1 AND deleted_at IS NULL AND status = 'active'",
        params![workspace_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// One page of the workspace list.
///
/// `archived` (the default `false`) is every user workspace that is *not*
/// archived; `true` lists the shelved ones by `archived_at`. Temporary chat
/// workspaces are containers of a conversation, not list items, so they never
/// appear here. Pinned workspaces are excluded — like pinned conversations they
/// lead the list whole ([`list_pinned_workspaces`]).
pub fn list_workspaces_page(
    archived: bool,
    cursor: Option<&str>,
    limit: i64,
) -> Result<WorkspacesPage, crate::Error> {
    let limit = limit.clamp(1, 100);
    let conn = connect()?;
    let (where_clause, stamp) = if archived {
        (
            "kind = 'user' AND archived_at IS NOT NULL AND deleted_at IS NULL",
            "COALESCE(archived_at, updated_at)",
        )
    } else {
        (
            "kind = 'user' AND archived_at IS NULL AND deleted_at IS NULL AND pinned = 0",
            "COALESCE(last_opened_at, updated_at)",
        )
    };
    let cursor = cursor.and_then(decode_cursor);
    let cursor_clause = if cursor.is_some() {
        format!("AND ({stamp}, id) < (?1, ?2)")
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT id, name, kind, path, description, pinned, cleanup_status,
                cleanup_requested_at, cleaned_at, last_opened_at, created_at,
                updated_at, archived_at, deleted_at, {stamp} AS stamp
           FROM workspaces
          WHERE {where_clause} {cursor_clause}
          ORDER BY stamp DESC, id DESC
          LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(WorkspaceRecord, i64)> = match &cursor {
        Some((stamp, id)) => stmt
            .query_map(params![stamp, id], |row| {
                Ok((workspace_from_row(row)?, row.get::<_, i64>("stamp")?))
            })?
            .collect::<rusqlite::Result<_>>()?,
        None => stmt
            .query_map([], |row| {
                Ok((workspace_from_row(row)?, row.get::<_, i64>("stamp")?))
            })?
            .collect::<rusqlite::Result<_>>()?,
    };
    let next_cursor = if rows.len() as i64 == limit {
        rows.last()
            .map(|(workspace, stamp)| encode_cursor(*stamp, &workspace.id))
    } else {
        None
    };
    Ok(WorkspacesPage {
        workspaces: rows.into_iter().map(|(workspace, _)| workspace).collect(),
        next_cursor,
    })
}

/// The pinned user workspaces, whole (they lead the rail's workspace section).
pub fn list_pinned_workspaces() -> Result<Vec<WorkspaceRecord>, crate::Error> {
    let conn = connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, path, description, pinned, cleanup_status,
                cleanup_requested_at, cleaned_at, last_opened_at, created_at,
                updated_at, archived_at, deleted_at
           FROM workspaces
          WHERE kind = 'user' AND archived_at IS NULL AND deleted_at IS NULL AND pinned = 1
          ORDER BY COALESCE(last_opened_at, updated_at) DESC, id DESC",
    )?;
    let rows = stmt.query_map([], workspace_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Server-side search over conversation titles and workspace names, split in
/// two so archived results can be shown as their own section (client-side
/// filtering cannot see rows that paging has not loaded).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchResults {
    pub active: Vec<super::ThreadRecord>,
    pub archived: Vec<super::ThreadRecord>,
}

pub fn search_threads(query: &str, limit: i64) -> Result<ThreadSearchResults, crate::Error> {
    let limit = limit.clamp(1, 200);
    let pattern = format!(
        "%{}%",
        query
            .trim()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let conn = connect()?;
    let sql = format!(
        "SELECT {columns} FROM threads t JOIN workspaces w ON w.id = t.workspace_id
         WHERE t.deleted_at IS NULL
           AND (t.title LIKE ?1 ESCAPE '\\' OR w.name LIKE ?1 ESCAPE '\\')
           AND {{partition}}
         ORDER BY {activity} DESC, t.id DESC
         LIMIT {limit}",
        columns = qualify_columns("t", THREAD_COLUMNS),
        activity = thread_activity("t")
    );
    let run = |partition: &str| -> Result<Vec<super::ThreadRecord>, crate::Error> {
        let mut stmt = conn.prepare(&sql.replace("{partition}", partition))?;
        let rows = stmt.query_map(params![pattern], thread_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    };
    Ok(ThreadSearchResults {
        active: run("t.status = 'active' AND w.archived_at IS NULL")?,
        archived: run("(t.status = 'archived' OR w.archived_at IS NOT NULL)")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::db::test_support::guarded_conn;

    fn seed(
        conn: &rusqlite::Connection,
        workspace: &str,
        id: &str,
        updated: i64,
        parent: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO threads (id, workspace_id, mode, title, status, pinned,
                 readonly, agent_session_id, parent_session_id, created_at, updated_at)
             VALUES (?1, ?2, 'chat', ?1, 'active', 0, 0, ?1, ?3, 1, ?4)",
            params![id, workspace, parent, updated],
        )
        .unwrap();
    }

    fn workspace(conn: &rusqlite::Connection, id: &str, archived: bool) {
        conn.execute(
            "INSERT INTO workspaces (id, name, kind, path, created_at, updated_at, archived_at)
             VALUES (?1, ?1, 'user', '/tmp/' || ?1, 1, 1, ?2)",
            params![id, if archived { Some(5) } else { None }],
        )
        .unwrap();
    }

    #[test]
    fn a_chat_page_returns_rows_ordered_by_activity_with_a_cursor() {
        let (_home, conn) = guarded_conn("page_chat");
        workspace(&conn, "ws", false);
        for (id, updated) in [("a", 10), ("b", 30), ("c", 20)] {
            seed(&conn, "ws", id, updated, None);
        }
        drop(conn);

        let page = list_threads_page(&ThreadScope::Chat, None, 2).expect("first page");
        let ids: Vec<&str> = page.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"]);
        let cursor = page.next_cursor.expect("a full page has a cursor");

        let next = list_threads_page(&ThreadScope::Chat, Some(&cursor), 2).expect("second page");
        let ids: Vec<&str> = next.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
        assert!(next.next_cursor.is_none(), "the last page has no cursor");
    }

    #[test]
    fn a_page_carries_a_roots_whole_fork_tree() {
        let (_home, conn) = guarded_conn("page_tree");
        workspace(&conn, "ws", false);
        seed(&conn, "ws", "root", 10, None);
        // The child's stamp is newer than its root's, which is exactly the row
        // that row-level paging would strand in a later page.
        seed(&conn, "ws", "child", 50, Some("root"));
        seed(&conn, "ws", "grandchild", 60, Some("child"));
        drop(conn);

        let page = list_threads_page(&ThreadScope::Chat, None, 1).expect("page");
        let ids: Vec<&str> = page.threads.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"root"), "{ids:?}");
        assert!(ids.contains(&"child"), "{ids:?}");
        assert!(ids.contains(&"grandchild"), "{ids:?}");
    }

    /// A page reports the scope's whole size, not its own.
    ///
    /// The recent view's "show the other N" is read from this: a cursor answers
    /// "there is more", never "how much", so a count taken from the page would
    /// name the loaded part instead of the list's remainder.
    #[test]
    fn a_page_reports_the_scopes_whole_root_count() {
        let (_home, conn) = guarded_conn("page_total");
        workspace(&conn, "ws", false);
        workspace(&conn, "other", false);
        for (id, updated) in [("a", 10), ("b", 30), ("c", 20)] {
            seed(&conn, "ws", id, updated, None);
        }
        // A fork's child is not a root, so it is not part of the count.
        seed(&conn, "ws", "child", 40, Some("a"));
        // Another workspace's conversation still belongs to the recent stream.
        seed(&conn, "other", "elsewhere", 15, None);
        drop(conn);

        let first = list_threads_page(&ThreadScope::Recent, None, 2).expect("first page");
        let ids: Vec<&str> = first.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"], "the two newest roots");
        assert_eq!(
            first.total_roots, 4,
            "every root of the scope, across workspaces"
        );
        let cursor = first.next_cursor.expect("a full page has a cursor");

        let last = list_threads_page(&ThreadScope::Recent, Some(&cursor), 2).expect("second page");
        let ids: Vec<&str> = last.threads.iter().map(|t| t.id.as_str()).collect();
        // The rest of the roots, and the fork child that rides with its root.
        assert_eq!(ids, vec!["elsewhere", "a", "child"]);
        // A full page carries a cursor even when the list ends there (the host
        // cannot know without asking again), so the end is the empty page.
        if let Some(cursor) = last.next_cursor {
            let empty =
                list_threads_page(&ThreadScope::Recent, Some(&cursor), 2).expect("beyond the end");
            assert!(empty.threads.is_empty());
            assert_eq!(empty.total_roots, 4);
        }
        // The count describes the list, so it is the same on every page of it.
        assert_eq!(last.total_roots, 4);
        // And a narrower scope counts its own rows only.
        let chat = list_threads_page(&ThreadScope::Chat, None, 50).expect("chat page");
        assert_eq!(chat.total_roots, 4);
        let one = list_threads_page(&ThreadScope::Workspace("other".into()), None, 50)
            .expect("workspace page");
        assert_eq!(one.total_roots, 1);
    }

    #[test]
    fn an_archived_workspace_hides_its_conversations_from_the_default_lists() {
        let (_home, conn) = guarded_conn("page_archived_ws");
        workspace(&conn, "open", false);
        workspace(&conn, "shelved", true);
        seed(&conn, "open", "visible", 10, None);
        seed(&conn, "shelved", "hidden", 20, None);
        drop(conn);

        let page = list_threads_page(&ThreadScope::Chat, None, 50).expect("page");
        let ids: Vec<&str> = page.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["visible"]);
    }

    #[test]
    fn an_archived_page_lists_shelved_conversations() {
        let (_home, conn) = guarded_conn("page_archived");
        workspace(&conn, "ws", false);
        seed(&conn, "ws", "live", 10, None);
        seed(&conn, "ws", "old", 20, None);
        conn.execute(
            "UPDATE threads SET status = 'archived' WHERE id = 'old'",
            [],
        )
        .unwrap();
        drop(conn);

        let page = list_threads_page(&ThreadScope::Archived, None, 50).expect("page");
        let ids: Vec<&str> = page.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["old"]);
    }

    /// An archived workspace's contents are browsable while the workspace is
    /// shelved — its conversations keep their own status.
    #[test]
    fn an_archived_workspaces_page_lists_everything_it_holds() {
        let (_home, conn) = guarded_conn("page_archived_ws_contents");
        workspace(&conn, "shelved", true);
        seed(&conn, "shelved", "one", 10, None);
        seed(&conn, "shelved", "two", 20, None);
        drop(conn);

        let page = list_threads_page(&ThreadScope::ArchivedWorkspace("shelved".into()), None, 50)
            .expect("page");
        assert_eq!(page.threads.len(), 2);
        assert!(
            page.threads.iter().all(|thread| thread.status == "active"),
            "a workspace archive never rewrites member status"
        );
        // …and those rows stay out of the archived *conversation* section, so
        // the view shows each conversation exactly once.
        let archived = list_threads_page(&ThreadScope::Archived, None, 50).expect("archived");
        assert!(archived.threads.is_empty());
    }

    /// The shelved scope is both halves of the archive at once.
    ///
    /// The archived *view* draws them apart — a workspace's contents under its
    /// heading, a user-shelved conversation in its own section — but one machine
    /// hands both to a paired client as one set of rows marked `archived`, and
    /// this is the scope that reads them back the same way.
    #[test]
    fn a_shelved_page_lists_both_halves_of_the_archive() {
        let (_home, conn) = guarded_conn("page_shelved");
        workspace(&conn, "open", false);
        workspace(&conn, "closed", true);
        seed(&conn, "open", "live", 10, None);
        seed(&conn, "open", "shelved", 20, None);
        seed(&conn, "closed", "inside", 30, None);
        conn.execute(
            "UPDATE threads SET status = 'archived' WHERE id = 'shelved'",
            [],
        )
        .unwrap();
        drop(conn);

        let page = list_threads_page(&ThreadScope::Shelved, None, 50).expect("page");
        let mut ids: Vec<&str> = page
            .threads
            .iter()
            .map(|thread| thread.id.as_str())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["inside", "shelved"]);
        // Neither half is the active list.
        let live = list_threads_page(&ThreadScope::Recent, None, 50).expect("recent");
        let live: Vec<&str> = live
            .threads
            .iter()
            .map(|thread| thread.id.as_str())
            .collect();
        assert_eq!(live, vec!["live"]);
    }

    #[test]
    fn a_workspace_scope_pages_only_that_workspace() {
        let (_home, conn) = guarded_conn("page_ws_scope");
        workspace(&conn, "one", false);
        workspace(&conn, "two", false);
        seed(&conn, "one", "a", 10, None);
        seed(&conn, "two", "b", 20, None);
        drop(conn);

        let page =
            list_threads_page(&ThreadScope::Workspace("one".into()), None, 50).expect("page");
        let ids: Vec<&str> = page.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    #[test]
    fn pinned_rows_are_not_paged_and_are_listed_whole() {
        let (_home, conn) = guarded_conn("page_pinned");
        workspace(&conn, "ws", false);
        seed(&conn, "ws", "a", 10, None);
        seed(&conn, "ws", "pin", 30, None);
        conn.execute("UPDATE threads SET pinned = 1 WHERE id = 'pin'", [])
            .unwrap();
        drop(conn);

        let page = list_threads_page(&ThreadScope::Chat, None, 50).expect("page");
        assert!(page.threads.iter().all(|t| !t.pinned), "pins stay out");
        let pinned = list_pinned_threads(&ThreadScope::Chat).expect("pinned");
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].id, "pin");
    }

    #[test]
    fn workspace_pages_split_active_from_archived() {
        let (_home, conn) = guarded_conn("page_workspaces");
        workspace(&conn, "open", false);
        workspace(&conn, "shelved", true);
        drop(conn);

        let active = list_workspaces_page(false, None, 50).expect("active");
        assert_eq!(active.workspaces.len(), 1);
        assert_eq!(active.workspaces[0].id, "open");

        let archived = list_workspaces_page(true, None, 50).expect("archived");
        assert_eq!(archived.workspaces.len(), 1);
        assert_eq!(archived.workspaces[0].id, "shelved");
    }

    #[test]
    fn pinned_workspaces_lead_and_are_not_paged() {
        let (_home, conn) = guarded_conn("page_ws_pinned");
        workspace(&conn, "open", false);
        workspace(&conn, "pin", false);
        conn.execute("UPDATE workspaces SET pinned = 1 WHERE id = 'pin'", [])
            .unwrap();
        drop(conn);

        let page = list_workspaces_page(false, None, 50).expect("page");
        assert!(page.workspaces.iter().all(|workspace| !workspace.pinned));
        let pinned = list_pinned_workspaces().expect("pinned");
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].id, "pin");
    }

    #[test]
    fn search_splits_active_from_archived_results() {
        let (_home, conn) = guarded_conn("page_search");
        workspace(&conn, "ws", false);
        seed(&conn, "ws", "live-alpha", 10, None);
        seed(&conn, "ws", "old-alpha", 20, None);
        conn.execute(
            "UPDATE threads SET status = 'archived' WHERE id = 'old-alpha'",
            [],
        )
        .unwrap();
        drop(conn);

        let results = search_threads("alpha", 50).expect("search");
        assert_eq!(results.active.len(), 1);
        assert_eq!(results.active[0].id, "live-alpha");
        assert_eq!(results.archived.len(), 1);
        assert_eq!(results.archived[0].id, "old-alpha");
    }

    #[test]
    fn an_unknown_scope_is_refused() {
        assert!(ThreadScope::parse("nope").is_err());
        assert_eq!(
            ThreadScope::parse("workspace:ws-1").unwrap(),
            ThreadScope::Workspace("ws-1".into())
        );
        assert_eq!(
            ThreadScope::parse("archived").unwrap(),
            ThreadScope::Archived
        );
    }

    #[test]
    fn a_malformed_cursor_starts_over_instead_of_failing() {
        let (_home, conn) = guarded_conn("page_bad_cursor");
        workspace(&conn, "ws", false);
        seed(&conn, "ws", "a", 10, None);
        drop(conn);

        let page = list_threads_page(&ThreadScope::Chat, Some("garbage"), 50).expect("page");
        assert_eq!(page.threads.len(), 1);
    }
}
