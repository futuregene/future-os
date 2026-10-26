//! The side panels' *local* facts: the goal store's todos, the installed
//! skills, and the scheduled jobs.
//!
//! None of these travel over gRPC — they are files this machine owns — so the
//! workbench reads them where they live:
//!
//! * **Goal todos** come from the control plane's event store
//!   (`<root>/.future/goal/goals/<id>/events.jsonl`). [`goal_root`] resolves the
//!   store the way a reader has to (not the way the CLI's writer does): a
//!   session started in a task worktree still belongs to the checkout it was cut
//!   from, and `<cwd>/.future/goal` there holds nothing.
//! * **Skills** are the installed ones (`~/.future/agent/skills` and the shared
//!   `~/.agents/skills`), the same directories the agent discovers — the agent's
//!   `get_state` names them, and this is the fallback that fills in their
//!   descriptions.
//! * **Jobs** are the scheduled tasks in `<home>/tasks/tasks.db`, opened
//!   **read-only**: a panel must not create or migrate a store the host owns.
//!
//! Every reader is best-effort and total: a missing store, a half-written line
//! or an unreadable file yields fewer rows, never a panic and never a notice.
//! These are the sidebar, not the conversation.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::i18n::{t, Key, Language};
use super::state::{JobRow, SkillRow, TodoItem, TodoState};

// ─── goal store ─────────────────────────────────────────────────────────────

/// Where this workspace's goal store lives.
///
/// 1. `FUTURE_GOAL_ROOT` wins — the override the control plane itself honours.
/// 2. Otherwise the nearest ancestor holding `.future/goal/goals`. Walking up
///    (rather than only looking at `cwd`) crosses the worktree boundary: a
///    session in `.worktrees/<name>` is inside the checkout whose store holds
///    its goal, and that checkout is an ancestor.
/// 3. Failing that, the nearest ancestor that *is* a git work tree, for a
///    worktree checked out somewhere else entirely.
///
/// `None` when nothing looks like a store: a reader must never create one.
pub fn goal_root(cwd: &Path) -> Option<PathBuf> {
    if let Ok(root) = std::env::var("FUTURE_GOAL_ROOT") {
        if !root.is_empty() {
            return Some(PathBuf::from(root));
        }
    }
    if let Some(found) = nearest_store(cwd) {
        return Some(found);
    }
    let toplevel = git_toplevel(cwd)?;
    nearest_store(&toplevel)
}

fn nearest_store(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .map(|dir| dir.join(".future").join("goal"))
        .find(|candidate| candidate.join("goals").is_dir())
}

/// `git rev-parse --show-toplevel`, without ever letting git wedge the UI.
fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let args = vec![
        "-C".to_string(),
        cwd.display().to_string(),
        "rev-parse".to_string(),
        "--show-toplevel".to_string(),
    ];
    let (code, stdout, _stderr) = crate::subprocess::spawn_with_timeout(
        Path::new("git"),
        &args,
        std::time::Duration::from_secs(2),
    )
    .ok()?;
    if code != 0 {
        return None;
    }
    let path = stdout.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// The todos of the goal this workspace is working on.
///
/// The goal is the most recently active one that has not reached a terminal
/// state: a finished goal's todos are history, not this workspace's work. An
/// empty list is the honest answer for a workspace with no goal — the panel
/// says `no todos` rather than inventing them.
pub fn goal_todos(cwd: &Path) -> Vec<TodoItem> {
    let Some(root) = goal_root(cwd) else {
        return Vec::new();
    };
    let mut best: Option<Folded> = None;
    for id in goal_ids(&root) {
        let events = root.join("goals").join(&id).join("events.jsonl");
        let Ok(text) = std::fs::read_to_string(&events) else {
            continue;
        };
        let folded = fold_events(&text, &id);
        if folded.terminal {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|current| folded.last_ts >= current.last_ts)
        {
            best = Some(folded);
        }
    }
    best.map(|folded| folded.todos).unwrap_or_default()
}

/// A bounded list of files under `cwd`, for the `@` menu.
///
/// `git ls-files` when the directory is a repository — it is the fastest honest
/// answer and it already honours `.gitignore`, which is what keeps a menu of
/// candidates from filling with build output. Anything else (or a git that is
/// missing, or a listing too slow to be worth it) falls back to a short walk that
/// prunes the directories every project has.
pub fn workspace_files(cwd: &Path) -> Vec<String> {
    const FILE_LIMIT: usize = 4000;
    if cwd.as_os_str().is_empty() || !cwd.is_dir() {
        return Vec::new();
    }
    let args = vec![
        "-C".to_string(),
        cwd.display().to_string(),
        "ls-files".to_string(),
        "--cached".to_string(),
        "--others".to_string(),
        "--exclude-standard".to_string(),
    ];
    if let Ok((0, stdout, _)) = crate::subprocess::spawn_with_timeout(
        Path::new("git"),
        &args,
        std::time::Duration::from_secs(2),
    ) {
        let mut files: Vec<String> = stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(FILE_LIMIT)
            .map(str::to_string)
            .collect();
        if !files.is_empty() {
            files.sort();
            files.dedup();
            return files;
        }
    }
    let mut files = Vec::new();
    walk(cwd, cwd, 0, &mut files, FILE_LIMIT);
    files.sort();
    files
}

/// Directories a file menu never needs: version control, build output, package
/// caches. Pruned rather than listed and filtered, so the walk stays cheap.
const SKIP_DIRS: [&str; 10] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "coverage",
];

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>, limit: usize) {
    if depth > 8 || out.len() >= limit {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    for path in children {
        if out.len() >= limit {
            return;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            walk(root, &path, depth + 1, out, limit);
        } else if let Ok(relative) = path.strip_prefix(root) {
            out.push(relative.display().to_string());
        }
    }
}

fn goal_ids(root: &Path) -> Vec<String> {
    // The registry is the control plane's own index; the directory listing is
    // the fallback for a store whose registry was not written yet.
    let registry = std::fs::read_to_string(root.join("registry.json")).ok();
    let mut ids: Vec<String> = registry
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|value| value.as_array().cloned())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("goal_id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        if let Ok(entries) = std::fs::read_dir(root.join("goals")) {
            ids = entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
                .collect();
        }
    }
    ids.sort();
    ids
}

#[derive(Default)]
struct Folded {
    last_ts: i64,
    terminal: bool,
    todos: Vec<TodoItem>,
}

impl Folded {
    fn todo(&mut self, id: &str) -> Option<&mut TodoItem> {
        self.todos.iter_mut().find(|todo| todo.id == id)
    }
}

/// Fold one goal's `events.jsonl` into the list the panel prints.
///
/// Unknown event kinds are ignored on purpose: the goal plane grows event kinds
/// over time, and a reader that refuses to draw because it met a new one would
/// turn "the panel is behind" into "the panel is broken".
fn fold_events(text: &str, _goal_id: &str) -> Folded {
    let mut folded = Folded::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ts = event.get("ts").and_then(Value::as_i64).unwrap_or(0);
        folded.last_ts = folded.last_ts.max(ts);
        let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
        match kind {
            "goal_started" => folded.terminal = false,
            "goal_closed" | "goal_completed" | "goal_validated" | "goal_abandoned"
            | "goal_failed" => folded.terminal = true,
            "todo_added" => {
                let Some(todo) = event.get("todo") else {
                    continue;
                };
                let Some(id) = todo.get("id").and_then(Value::as_str) else {
                    continue;
                };
                folded.todos.retain(|known| known.id != id);
                folded.todos.push(item_from_todo_json(todo));
            }
            "todo_updated" => {
                let Some(id) = event.get("todo_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(item) = folded.todo(id) else {
                    continue;
                };
                if let Some(priority) = event.get("priority").and_then(Value::as_str) {
                    item.priority = priority.to_string();
                }
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    item.text = text.to_string();
                }
                if let Some(status) = event.get("status").and_then(Value::as_str) {
                    item.state = state_of(status, false);
                }
                if event
                    .get("blocks")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| !blocks.is_empty())
                {
                    item.state = TodoState::Blocked;
                }
            }
            "todo_claimed" => {
                let Some(id) = event.get("todo_id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(item) = folded.todo(id) {
                    if item.state == TodoState::Open {
                        item.state = TodoState::Active;
                    }
                }
            }
            "todo_released" => {
                let Some(id) = event.get("todo_id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(item) = folded.todo(id) {
                    if item.state == TodoState::Active {
                        item.state = TodoState::Open;
                    }
                }
            }
            "todo_completed" => {
                let Some(id) = event.get("todo_id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(item) = folded.todo(id) {
                    item.state = TodoState::Done;
                }
            }
            "todo_superseded" => {
                let Some(id) = event.get("todo_id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(item) = folded.todo(id) {
                    item.state = TodoState::Cancelled;
                }
            }
            _ => {}
        }
    }
    folded
}

fn item_from_todo_json(todo: &Value) -> TodoItem {
    let status = todo.get("status").and_then(Value::as_str).unwrap_or("open");
    let blocked = todo
        .get("blocked_by_gate")
        .is_some_and(|gate| !gate.is_null());
    let claimed = todo
        .get("claimed_by")
        .and_then(Value::as_str)
        .is_some_and(|who| !who.is_empty());
    let state = match state_of(status, blocked) {
        TodoState::Open if claimed => TodoState::Active,
        other => other,
    };
    TodoItem {
        id: todo
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        text: todo
            .get("text")
            .or_else(|| todo.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        priority: todo
            .get("priority")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        state,
    }
}

fn state_of(status: &str, blocked: bool) -> TodoState {
    if blocked || status.eq_ignore_ascii_case("blocked") {
        TodoState::Blocked
    } else {
        match status.to_ascii_lowercase().as_str() {
            "done" => TodoState::Done,
            "superseded" | "cancelled" | "canceled" => TodoState::Cancelled,
            _ => TodoState::Open,
        }
    }
}

// ─── skills ─────────────────────────────────────────────────────────────────

/// The installed skills: `~/.future/agent/skills` (app scope) first, then the
/// shared `~/.agents/skills`, deduplicated by name.
pub fn installed_skills() -> Vec<SkillRow> {
    let mut dirs = vec![crate::home::future_state_dir().join("agent").join("skills")];
    if let Some(home) = crate::home::home_dir() {
        dirs.push(home.join(".agents").join("skills"));
    }
    let mut skills: Vec<SkillRow> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if skills.iter().any(|known| known.name == name) {
                continue;
            }
            let description = std::fs::read_to_string(path.join("SKILL.md"))
                .map(|text| skill_description(&text))
                .unwrap_or_default();
            skills.push(SkillRow {
                name: name.to_string(),
                description,
            });
        }
    }
    skills
}

/// The `description:` field of a `SKILL.md` frontmatter block, on one line.
///
/// Covers the two shapes the installed skills use — an inline value and a
/// folded block (`description: >`) — and stops at the next key. A description
/// that cannot be read is empty, not an error: the panel still lists the skill.
fn skill_description(text: &str) -> String {
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    if first.trim() != "---" {
        return String::new();
    }
    let mut collecting = false;
    let mut parts: Vec<String> = Vec::new();
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let trimmed = line.trim_start();
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if collecting {
            if indented && !trimmed.is_empty() {
                parts.push(trimmed.to_string());
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            collecting = false;
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            let value = value.trim();
            if value.is_empty() || value == ">" || value == "|" || value == ">-" || value == "|-" {
                collecting = true;
            } else {
                parts.push(value.to_string());
                break;
            }
        }
    }
    let joined = parts.join(" ");
    let joined = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&joined, 96)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ─── scheduled jobs ─────────────────────────────────────────────────────────

/// The scheduled tasks this machine knows about, newest name order.
///
/// Read-only: the store is opened with `SQLITE_OPEN_READ_ONLY` and a missing
/// database is simply "no jobs". A panel must not create or migrate a store the
/// tasks host owns.
pub fn scheduled_jobs(lang: Language) -> Vec<JobRow> {
    let home = crate::home::future_state_dir();
    let Ok(Some(store)) = future_tasks::store::Store::open_existing_read_only(&home) else {
        return Vec::new();
    };
    let Ok(tasks) = store.list_tasks(false) else {
        return Vec::new();
    };
    tasks
        .into_iter()
        .map(|task| JobRow {
            name: task.name,
            state: t(
                lang,
                if !task.enabled {
                    Key::JobPaused
                } else if task.next_due_at.is_some() {
                    Key::JobScheduled
                } else {
                    Key::JobManual
                },
            )
            .to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_goal_is_folded_from_its_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let goal = dir.path().join(".future/goal/goals/goal_a");
        write(
            &goal.join("events.jsonl"),
            concat!(
                r#"{"kind":"goal_started","ts":10}"#,
                "\n",
                r#"{"kind":"todo_added","ts":11,"todo":{"id":"t1","text":"first","priority":"P0","status":"open"}}"#,
                "\n",
                r#"{"kind":"todo_added","ts":12,"todo":{"id":"t2","text":"second","priority":"P1","status":"open"}}"#,
                "\n",
                r#"{"kind":"todo_claimed","ts":13,"todo_id":"t1","agent_id":"w"}"#,
                "\n",
            ),
        );
        let todos = goal_todos(dir.path());
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].text, "first");
        assert_eq!(todos[0].priority, "P0");
        assert_eq!(todos[0].state, TodoState::Active);
        assert_eq!(todos[1].state, TodoState::Open);
    }

    #[test]
    fn completion_supersession_and_gates_are_whole_pieces() {
        let dir = tempfile::tempdir().unwrap();
        let goal = dir.path().join(".future/goal/goals/goal_b");
        write(
            &goal.join("events.jsonl"),
            concat!(
                r#"{"kind":"goal_started","ts":1}"#,
                "\n",
                r#"{"kind":"todo_added","ts":2,"todo":{"id":"a","text":"done","status":"open"}}"#,
                "\n",
                r#"{"kind":"todo_added","ts":3,"todo":{"id":"b","text":"gone","status":"open"}}"#,
                "\n",
                r#"{"kind":"todo_added","ts":4,"todo":{"id":"c","text":"waits","status":"blocked"}}"#,
                "\n",
                r#"{"kind":"todo_completed","ts":5,"todo_id":"a"}"#,
                "\n",
                r#"{"kind":"todo_superseded","ts":6,"todo_id":"b"}"#,
                "\n",
            ),
        );
        let todos = goal_todos(dir.path());
        let state = |id: &str| {
            todos
                .iter()
                .find(|todo| todo.id == id)
                .map(|todo| todo.state)
        };
        assert_eq!(state("a"), Some(TodoState::Done));
        assert_eq!(state("b"), Some(TodoState::Cancelled));
        assert_eq!(state("c"), Some(TodoState::Blocked));
        // Every state has a one-cell marker, which is what the panel's gutter
        // assumes (a two-cell glyph would shift the text on that row only).
        for state in [
            TodoState::Open,
            TodoState::Active,
            TodoState::Done,
            TodoState::Blocked,
            TodoState::Cancelled,
        ] {
            assert_eq!(crate::workbench::ansi::width(state.glyph()), 1, "{state:?}");
        }
    }

    #[test]
    fn a_finished_goal_is_not_this_workspaces_work() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join(".future/goal/goals/goal_old");
        write(
            &old.join("events.jsonl"),
            "{\"kind\":\"goal_started\",\"ts\":1}\n{\"kind\":\"todo_added\",\"ts\":2,\"todo\":{\"id\":\"o\",\"text\":\"old\",\"status\":\"open\"}}\n{\"kind\":\"goal_closed\",\"ts\":3}\n",
        );
        assert!(goal_todos(dir.path()).is_empty());
        // A live goal beside it wins even when it started earlier.
        let live = dir.path().join(".future/goal/goals/goal_new");
        write(
            &live.join("events.jsonl"),
            "{\"kind\":\"goal_started\",\"ts\":2}\n{\"kind\":\"todo_added\",\"ts\":4,\"todo\":{\"id\":\"n\",\"text\":\"new\",\"status\":\"open\"}}\n",
        );
        let todos = goal_todos(dir.path());
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "new");
    }

    #[test]
    fn a_worktree_reads_the_store_of_the_checkout_it_was_cut_from() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join(".future/goal/goals/goal_w");
        write(
            &store.join("events.jsonl"),
            "{\"kind\":\"goal_started\",\"ts\":1}\n{\"kind\":\"todo_added\",\"ts\":2,\"todo\":{\"id\":\"w\",\"text\":\"from the checkout\",\"status\":\"open\"}}\n",
        );
        let worktree = dir.path().join(".worktrees/feature");
        std::fs::create_dir_all(&worktree).unwrap();
        // `.worktrees/<name>` has no store of its own: walking up finds the
        // checkout's, which is the whole reason the reader looks above `cwd`.
        assert!(!worktree.join(".future/goal/goals").exists());
        assert_eq!(goal_todos(&worktree).len(), 1);
        assert_eq!(
            nearest_store(&worktree),
            Some(dir.path().join(".future/goal"))
        );
    }

    #[test]
    fn a_workspace_without_a_store_has_no_todos_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(goal_todos(dir.path()).is_empty());
        assert!(
            !dir.path().join(".future").exists(),
            "a reader created a store"
        );
    }

    /// The `@` menu's candidates come from the working directory: a bounded
    /// list, the same one for every reader of the same tree, and the directories
    /// every project has (build output, caches) are never in it.
    #[test]
    fn workspace_files_lists_the_tree_and_prunes_build_output() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("README.md"), "x");
        write(&dir.path().join("src/main.rs"), "x");
        write(&dir.path().join("src/nested/deep.rs"), "x");
        write(&dir.path().join("target/debug/junk"), "x");
        write(&dir.path().join("node_modules/pkg/index.js"), "x");
        write(&dir.path().join(".hidden"), "x");
        let files = workspace_files(dir.path());
        assert!(files.contains(&"README.md".to_string()), "{files:?}");
        assert!(files.contains(&"src/main.rs".to_string()), "{files:?}");
        assert!(
            files.contains(&"src/nested/deep.rs".to_string()),
            "the walk descends: {files:?}"
        );
        assert!(
            !files.iter().any(|file| file.starts_with("target/")),
            "build output is pruned: {files:?}"
        );
        assert!(
            !files.iter().any(|file| file.starts_with("node_modules/")),
            "{files:?}"
        );
        assert!(
            !files.iter().any(|file| file.starts_with('.')),
            "dotfiles stay out: {files:?}"
        );
        // Sorted, so the menu's order does not depend on the filesystem.
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
        // A directory that is not there is an empty list, not a panic.
        assert!(workspace_files(&dir.path().join("missing")).is_empty());
        assert!(workspace_files(Path::new("")).is_empty());
    }

    /// The fallback is only the fallback: inside a real repository the answer is
    /// `git ls-files`, so an ignored file never reaches the menu.
    #[test]
    fn a_repository_answers_with_its_tracked_and_unignored_files() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".gitignore"), "ignored.txt\n");
        write(&dir.path().join("kept.txt"), "x");
        write(&dir.path().join("ignored.txt"), "x");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .expect("run git")
        };
        let ok = git(&["init", "--quiet"]).status.success();
        assert!(ok, "git init failed");
        let files = workspace_files(dir.path());
        assert!(files.contains(&"kept.txt".to_string()), "{files:?}");
        assert!(
            !files.contains(&"ignored.txt".to_string()),
            "the ignore rules are what keep build output out: {files:?}"
        );
    }

    #[test]
    fn a_skill_description_is_one_line_from_either_frontmatter_shape() {
        let inline = "---\nname: a\ndescription: does a thing\n---\n# body\n";
        assert_eq!(skill_description(inline), "does a thing");
        let folded = "---\nname: b\ndescription: >\n  first half\n  second half\nother: x\n---\n";
        assert_eq!(skill_description(folded), "first half second half");
        assert_eq!(skill_description("no frontmatter"), "");
        let long = format!("---\ndescription: {}\n---\n", "字".repeat(400));
        assert!(skill_description(&long).chars().count() <= 96);
    }
}
