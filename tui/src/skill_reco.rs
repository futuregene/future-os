//! Skill-recommendation budget for the TUI.
//!
//! The client owns the trigger rules, so each client keeps its own daily
//! budget. This file is the TUI's: `~/.future/tui/skill_reco.json`.
//!
//! Why a separate file rather than `settings.json`: this is rolled-over,
//! per-day state (written on every recommendation), while `settings.json` holds
//! preferences a human edits. Mixing them would make a settings write clobber
//! the day's usage and vice versa.
//!
//! Semantics (PRD v1.6 §7):
//! - the daily limit counts **recommendations shown**, not calls — a call that
//!   recommends nothing leaves no record and does not consume the budget;
//! - one skill is never shown twice in a day;
//! - one message is never evaluated twice.
//!
//! The budget is per client by design: desktop/mobile/TUI each hold their own,
//! so one user's daily total across all clients is not capped at three.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Recommendations shown per local day.
pub const DAILY_LIMIT: u32 = 3;
/// Shortest draft worth asking about: 30 UTF-8 bytes, i.e. 10 汉字 or about 30
/// ASCII characters (PRD v1.6 §3).
pub const MIN_QUERY_BYTES: usize = 30;
/// Longest draft worth asking about; longer ones are sent unrecommended rather
/// than truncated (the call's cost is dominated by the candidate list).
pub const MAX_QUERY_CHARS: usize = 2000;
/// Candidates per call. A Choice accepts at most 255 options and the "none of
/// these" option takes one, so 254 skills is the hard ceiling; a longer
/// catalogue is truncated rather than chunked (the desktop client has the same
/// limit and a chunking path this one does not need yet).
pub const MAX_CANDIDATES: usize = 254;

/// One day's record. Only the current day is kept: the file is rewritten with a
/// fresh record when the day rolls over (nothing older is ever read).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecoDay {
    /// Local calendar day (`YYYY-MM-DD`) this record belongs to.
    pub day: String,
    /// Skill ids already recommended today.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Hashes of the messages that already produced a recommendation today.
    #[serde(default)]
    pub messages: Vec<String>,
}

impl SkillRecoDay {
    /// Number of recommendations shown today (the budget's unit).
    pub fn count(&self) -> u32 {
        self.skills.len() as u32
    }

    /// True when the day's recommendations are used up.
    pub fn exhausted(&self) -> bool {
        self.count() >= DAILY_LIMIT
    }

    /// True when this skill was already recommended today.
    pub fn already_recommended(&self, skill: &str) -> bool {
        self.skills.iter().any(|s| s == skill)
    }

    /// True when this message already produced a recommendation today.
    pub fn already_evaluated(&self, message_hash: &str) -> bool {
        self.messages.iter().any(|m| m == message_hash)
    }

    /// Record a shown recommendation.
    pub fn record(&mut self, skill: &str, message_hash: &str) {
        self.skills.push(skill.to_string());
        self.messages.push(message_hash.to_string());
    }
}

/// Local calendar day.
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// This client's budget file, derived from the TUI settings path so both live
/// under `~/.future/tui/` (and follow the same home resolution).
pub fn path_for(settings_path: &Path) -> std::path::PathBuf {
    settings_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("skill_reco.json")
}

/// Stable, non-security hash of a draft. Used only to recognise a message that
/// already produced a recommendation today, so collisions merely suppress a
/// recommendation; the full text is never stored.
pub fn message_hash(text: &str) -> String {
    // FNV-1a over the UTF-8 bytes.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.trim().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The stored record, or a fresh one when the file is missing, unreadable or
/// belongs to another day.
///
/// Every failure degrades to "no budget used yet": a corrupt file must never
/// make the feature refuse to work, and the worst case (an extra recommendation
/// today) is bounded by the budget being rewritten on the next record.
pub fn load_at(path: &Path) -> SkillRecoDay {
    let Ok(text) = std::fs::read_to_string(path) else {
        return fresh();
    };
    match serde_json::from_str::<SkillRecoDay>(&text) {
        Ok(mut day) if day.day == today() => {
            // Defensive: a hand-edited file could exceed the limit.
            day.skills.truncate(DAILY_LIMIT as usize);
            day
        }
        _ => fresh(),
    }
}

/// Load, record one shown recommendation, and write back (best effort).
///
/// The write is not atomic (no temp+rename): the file is disposable state, and
/// the only consequence of losing it is one extra recommendation.
pub fn record_at(path: &Path, skill: &str, message_hash: &str) -> SkillRecoDay {
    let mut day = load_at(path);
    day.record(skill, message_hash);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&day) {
        let _ = std::fs::write(path, json);
    }
    day
}

fn fresh() -> SkillRecoDay {
    SkillRecoDay {
        day: today(),
        skills: Vec::new(),
        messages: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tui-skill-reco-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("skill_reco.json")
    }

    #[test]
    fn a_missing_file_is_an_empty_today() {
        let path = temp_path("missing");
        let day = load_at(&path);
        assert_eq!(day.day, today());
        assert_eq!(day.count(), 0);
        assert!(!day.exhausted());
    }

    #[test]
    fn recording_accumulates_until_the_limit() {
        let path = temp_path("accumulate");
        assert!(!load_at(&path).exhausted());
        record_at(&path, "future-web", &message_hash("first"));
        record_at(&path, "future-paper", &message_hash("second"));
        let day = load_at(&path);
        assert_eq!(day.count(), 2);
        assert!(!day.exhausted(), "two of three is not exhausted");
        record_at(&path, "future-slides", &message_hash("third"));
        assert!(load_at(&path).exhausted());
    }

    #[test]
    fn the_same_skill_is_recognised_and_the_same_message_too() {
        let path = temp_path("dedupe");
        record_at(&path, "future-web", &message_hash("asked about a website"));
        let day = load_at(&path);
        assert!(day.already_recommended("future-web"));
        assert!(!day.already_recommended("future-paper"));
        assert!(day.already_evaluated(&message_hash("asked about a website")));
        assert!(!day.already_evaluated(&message_hash("something else")));
    }

    /// A day boundary resets the budget without any cleanup step: the record is
    /// simply not "today" any more.
    #[test]
    fn a_record_from_another_day_is_ignored() {
        let path = temp_path("rollover");
        let stale = SkillRecoDay {
            day: "2020-01-01".to_string(),
            skills: vec!["future-web".to_string(), "future-paper".to_string()],
            messages: vec!["abc".to_string()],
        };
        std::fs::write(&path, serde_json::to_string(&stale).unwrap()).unwrap();
        let day = load_at(&path);
        assert_eq!(
            day.count(),
            0,
            "yesterday's usage does not spend today's budget"
        );
        assert_eq!(day.day, today());
    }

    #[test]
    fn a_corrupt_file_degrades_to_an_empty_today() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "not json at all").unwrap();
        let day = load_at(&path);
        assert_eq!(day.count(), 0);
        assert_eq!(day.day, today());
        // And it recovers: the next record rewrites the file.
        record_at(&path, "future-web", &message_hash("hello"));
        assert_eq!(load_at(&path).count(), 1);
    }

    #[test]
    fn a_hand_edited_file_cannot_exceed_the_limit() {
        let path = temp_path("handedited");
        let over = SkillRecoDay {
            day: today(),
            skills: vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            messages: vec!["1".into(), "2".into(), "3".into(), "4".into(), "5".into()],
        };
        std::fs::write(&path, serde_json::to_string(&over).unwrap()).unwrap();
        let day = load_at(&path);
        assert_eq!(day.count(), DAILY_LIMIT);
        assert!(day.exhausted());
    }

    #[test]
    fn the_budget_file_sits_beside_the_settings_file() {
        let path = path_for(Path::new("/home/u/.future/tui/settings.json"));
        assert_eq!(
            path,
            std::path::PathBuf::from("/home/u/.future/tui/skill_reco.json")
        );
        // A bare filename with no directory must not panic.
        let bare = path_for(Path::new("settings.json"));
        assert!(bare.ends_with("skill_reco.json"));
    }

    #[test]
    fn the_message_hash_is_stable_ignores_surrounding_space_and_is_not_the_text() {
        assert_eq!(message_hash("hello"), message_hash("hello"));
        assert_eq!(message_hash("  hello  "), message_hash("hello"));
        assert_ne!(message_hash("hello"), message_hash("hello!"));
        assert_ne!(message_hash("单细胞测序"), message_hash("single cell"));
        let hash = message_hash("something private");
        assert_eq!(hash.len(), 16);
        assert!(
            !hash.contains("private"),
            "the hash must not leak the draft: {hash}"
        );
    }
}
