//! Privacy grading + redaction (G-4) — the three-tier projection privacy
//! mirroring LoopX `control_plane/runtime/public_safety.py` + the event
//! privacy taxonomy: `public_safe` / `local_private` / `private_pointer`.
//!
//! Conservative by default: content that cannot be safely classified
//! (empty / unknown) grades `private_pointer` — better to under-project than
//! leak. Public-safe projections redact private surfaces (local paths,
//! secret-like tokens, credential markers) to `[redacted-private-state]`.

use serde::{Deserialize, Serialize};

use crate::state::Goal;

pub const PUBLIC_PRIVACY: &str = "public_safe";
pub const LOCAL_PRIVATE_PRIVACY: &str = "local_private";
pub const PRIVATE_POINTER_PRIVACY: &str = "private_pointer";
pub const PRIVACY_VALUES: [&str; 3] = [
    PUBLIC_PRIVACY,
    LOCAL_PRIVATE_PRIVACY,
    PRIVATE_POINTER_PRIVACY,
];
pub const PUBLIC_BACKFILL_REDACTION: &str = "[redacted-private-state]";
/// The pointer emitted for private_pointer content in graded projections.
pub const PRIVATE_POINTER_TEMPLATE: &str = "[private-pointer:{todo_id}]";

/// Three-tier privacy level (LoopX `public_safe` / `local_private` /
/// `private_pointer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PrivacyLevel {
    #[serde(rename = "public_safe")]
    PublicSafe,
    #[serde(rename = "local_private")]
    LocalPrivate,
    #[serde(rename = "private_pointer")]
    PrivatePointer,
}

impl PrivacyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            PrivacyLevel::PublicSafe => PUBLIC_PRIVACY,
            PrivacyLevel::LocalPrivate => LOCAL_PRIVATE_PRIVACY,
            PrivacyLevel::PrivatePointer => PRIVATE_POINTER_PRIVACY,
        }
    }

    /// Whether this level redacts private surfaces before projection.
    pub fn redacts(self) -> bool {
        self == PrivacyLevel::PublicSafe
    }
}

impl std::str::FromStr for PrivacyLevel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "public_safe" | "public" => Ok(PrivacyLevel::PublicSafe),
            "local_private" | "local" => Ok(PrivacyLevel::LocalPrivate),
            "private_pointer" | "pointer" => Ok(PrivacyLevel::PrivatePointer),
            other => Err(format!(
                "unknown privacy level `{other}` (expected public_safe|local_private|private_pointer)"
            )),
        }
    }
}

// ── Private-surface detection ──────────────────────────────────────────────

/// Local-path surfaces (LoopX LOCAL_PATH_SURFACE_PATTERN, literal subset):
/// absolute HOME-relative paths and platform temp roots. Ordered longest
/// first so redaction replaces the most specific span.
const LOCAL_PATH_MARKERS: &[&str] = &[
    "/private/tmp/",
    "/var/folders/",
    "/Volumes/",
    "/Users/",
    "/tmp/",
    "C:\\Users\\",
    "~/.ssh",
];

/// Secret-like surfaces (LoopX SECRET_LIKE_SURFACE_PATTERN, literal subset):
/// bearer tokens, ak_/sk_ access-key shapes, `token=...`, `api_key`, and
/// credential file markers.
const SECRET_MARKERS: &[&str] = &[
    "BEGIN PRIVATE KEY",
    "BEGIN RSA PRIVATE KEY",
    "Authorization:",
    "authorization:",
    "api_key=",
    "api-key=",
    "apikey=",
    "token=",
    "secret=",
    "password=",
    "passwd=",
    "id_rsa",
    "auth.json",
    ".pem",
    "ak-",
    "sk-",
    ".ssh/",
];

/// All markers used for both classification and redaction.
fn all_markers() -> Vec<&'static str> {
    let mut markers = LOCAL_PATH_MARKERS.to_vec();
    markers.extend_from_slice(SECRET_MARKERS);
    markers
}

/// Whether text exposes a private surface (local path, secret-like token, or
/// a credential marker from the state-layer boundary scan).
pub fn contains_private_surface(text: &str) -> bool {
    if crate::state::boundary_scan_leaks(text)
        .iter()
        .any(|leak| !leak.is_empty())
    {
        return true;
    }
    all_markers().iter().any(|marker| text.contains(marker))
}

/// Classify free text (LoopX privacy taxonomy). Empty / unknown content
/// grades private_pointer (conservative).
pub fn classify_text(text: &str) -> PrivacyLevel {
    if text.trim().is_empty() {
        return PrivacyLevel::PrivatePointer;
    }
    if contains_private_surface(text) {
        PrivacyLevel::LocalPrivate
    } else {
        PrivacyLevel::PublicSafe
    }
}

/// Redact private surfaces for a public-safe projection. For local_private /
/// private_pointer the text is passed through unchanged (the caller decides
/// whether to project it at all).
pub fn redact(text: &str, level: PrivacyLevel) -> String {
    if !level.redacts() {
        return text.to_string();
    }
    let mut out = text.to_string();
    for marker in all_markers() {
        out = out.replace(marker, PUBLIC_BACKFILL_REDACTION);
    }
    out
}

// ── Goal grading ───────────────────────────────────────────────────────────

/// Per-todo privacy grade (which fields carried private surfaces).
#[derive(Debug, Clone, Serialize)]
pub struct PrivacyItem {
    pub todo_id: String,
    pub level: PrivacyLevel,
    pub redacted: bool,
    pub private_fields: Vec<String>,
}

/// Goal-wide privacy report.
#[derive(Debug, Clone, Serialize)]
pub struct GoalPrivacyReport {
    pub schema_version: String,
    pub goal_id: String,
    pub overall: PrivacyLevel,
    pub item_count: usize,
    pub public_safe_count: usize,
    pub local_private_count: usize,
    pub private_pointer_count: usize,
    pub items: Vec<PrivacyItem>,
}

pub const GOAL_PRIVACY_REPORT_SCHEMA_VERSION: &str = "goal_privacy_report_v0";

/// Grade every todo's text / evidence / note / gate question. `overall` is
/// the worst level seen (private_pointer > local_private > public_safe).
pub fn grade_goal(goal: &Goal) -> GoalPrivacyReport {
    let mut items = Vec::with_capacity(goal.todos.len());
    let mut public_safe_count = 0usize;
    let mut local_private_count = 0usize;
    let mut private_pointer_count = 0usize;
    for todo in &goal.todos {
        let mut worst = PrivacyLevel::PublicSafe;
        let mut private_fields: Vec<String> = vec![];
        let mut has_content = false;
        for (field, value) in [
            ("text", todo.text.as_str()),
            ("evidence", todo.evidence.as_deref().unwrap_or("")),
            ("note", todo.note.as_deref().unwrap_or("")),
            ("gate_question", todo.gate_question.as_deref().unwrap_or("")),
            (
                "monitor_target",
                todo.monitor_target.as_deref().unwrap_or(""),
            ),
        ] {
            if value.trim().is_empty() {
                continue; // empty fields are not private content
            }
            has_content = true;
            let level = classify_text(value);
            if level > worst {
                worst = level;
            }
            if level == PrivacyLevel::LocalPrivate {
                private_fields.push(field.to_string());
            }
        }
        // Unknown/empty content defaults private_pointer (conservative).
        if !has_content {
            worst = PrivacyLevel::PrivatePointer;
        }
        match worst {
            PrivacyLevel::PublicSafe => public_safe_count += 1,
            PrivacyLevel::LocalPrivate => local_private_count += 1,
            PrivacyLevel::PrivatePointer => private_pointer_count += 1,
        }
        items.push(PrivacyItem {
            todo_id: todo.id.clone(),
            level: worst,
            redacted: worst.redacts(),
            private_fields,
        });
    }
    let overall = if private_pointer_count > 0 {
        PrivacyLevel::PrivatePointer
    } else if local_private_count > 0 {
        PrivacyLevel::LocalPrivate
    } else {
        PrivacyLevel::PublicSafe
    };
    GoalPrivacyReport {
        schema_version: GOAL_PRIVACY_REPORT_SCHEMA_VERSION.to_string(),
        goal_id: goal.goal_id.clone(),
        overall,
        item_count: items.len(),
        public_safe_count,
        local_private_count,
        private_pointer_count,
        items,
    }
}

/// Whether the goal projects cleanly at public-safe (used by the G-6
/// migration bridge `public_boundary_clean` check).
pub fn privacy_boundary_clean(goal: &Goal) -> bool {
    grade_goal(goal).local_private_count == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_private_surfaces() {
        assert_eq!(classify_text("run the shell"), PrivacyLevel::PublicSafe);
        assert_eq!(
            classify_text("edit /Users/geilige/secret.txt"),
            PrivacyLevel::LocalPrivate
        );
        assert_eq!(
            classify_text("token=abc123def456"),
            PrivacyLevel::LocalPrivate
        );
        assert_eq!(
            classify_text("key at ~/.ssh/id_rsa"),
            PrivacyLevel::LocalPrivate
        );
        assert_eq!(classify_text(""), PrivacyLevel::PrivatePointer);
        assert_eq!(classify_text("   "), PrivacyLevel::PrivatePointer);
    }

    #[test]
    fn redact_replaces_private_spans_in_public_projection() {
        let text = "run in /Users/geilige/project with token=abc123";
        let redacted = redact(text, PrivacyLevel::PublicSafe);
        assert!(!redacted.contains("/Users/geilige"));
        assert!(!redacted.contains("token=abc123"));
        assert!(redacted.contains(PUBLIC_BACKFILL_REDACTION));
        // Local-private and pointer levels pass content through (caller decides).
        assert_eq!(redact(text, PrivacyLevel::LocalPrivate), text);
    }

    #[test]
    fn grade_goal_flags_private_todos() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(crate::state::Todo::advancement("t1", "public work"));
        goal.add(crate::state::Todo::advancement(
            "t2",
            "touch /Users/geilige/secret",
        ));
        let report = grade_goal(&goal);
        assert_eq!(report.overall, PrivacyLevel::LocalPrivate);
        assert_eq!(report.local_private_count, 1);
        assert_eq!(report.public_safe_count, 1);
        assert!(!privacy_boundary_clean(&goal));
        let item = report.items.iter().find(|i| i.todo_id == "t2").unwrap();
        assert!(item.private_fields.contains(&"text".to_string()));
    }

    #[test]
    fn unknown_content_is_conservatively_private_pointer() {
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.add(crate::state::Todo::advancement("t1", ""));
        let report = grade_goal(&goal);
        assert_eq!(report.overall, PrivacyLevel::PrivatePointer);
        assert_eq!(report.private_pointer_count, 1);
    }
}
