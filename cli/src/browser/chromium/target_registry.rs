//! Target session registry — port of
//! `cli/src/browser/chromium/chromium-target-registry.ts`.
//!
//! Maps CDP targetId ↔ sessionId with idempotent unified cleanup
//! (`Target.detachedFromTarget` and `Target.targetDestroyed` may both fire
//! for the same target).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// `AttachedTarget`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachedTarget {
    pub target_id: String,
    pub session_id: String,
    pub r#type: String,
}

/// `TargetSessionRegistry`.
#[derive(Default)]
pub struct TargetSessionRegistry {
    by_target_id: HashMap<String, AttachedTarget>,
    by_session_id: HashMap<String, AttachedTarget>,
}

impl TargetSessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, target: AttachedTarget) {
        self.by_target_id
            .insert(target.target_id.clone(), target.clone());
        self.by_session_id.insert(target.session_id.clone(), target);
    }

    /// Remove by sessionId. Returns None if already removed.
    pub fn detach_by_session_id(&mut self, session_id: &str) -> Option<AttachedTarget> {
        let target = self.by_session_id.remove(session_id)?;
        self.by_target_id.remove(&target.target_id);
        Some(target)
    }

    /// Remove by targetId. Returns None if already removed.
    pub fn detach_by_target_id(&mut self, target_id: &str) -> Option<AttachedTarget> {
        let target = self.by_target_id.remove(target_id)?;
        self.by_session_id.remove(&target.session_id);
        Some(target)
    }

    pub fn get_by_target_id(&self, target_id: &str) -> Option<&AttachedTarget> {
        self.by_target_id.get(target_id)
    }

    pub fn get_by_session_id(&self, session_id: &str) -> Option<&AttachedTarget> {
        self.by_session_id.get(session_id)
    }

    pub fn get_attached_page_ids(&self) -> Vec<String> {
        self.by_target_id.keys().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.by_target_id.clear();
        self.by_session_id.clear();
    }
}

/// `Arc<Mutex<TargetSessionRegistry>>` alias used by the connection.
pub type SharedTargetRegistry = Arc<Mutex<TargetSessionRegistry>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn target(t: &str, s: &str) -> AttachedTarget {
        AttachedTarget {
            target_id: t.to_string(),
            session_id: s.to_string(),
            r#type: "page".to_string(),
        }
    }

    #[test]
    fn add_and_retrieve_by_both_keys() {
        let mut r = TargetSessionRegistry::new();
        r.add(target("t1", "s1"));
        assert_eq!(r.get_by_target_id("t1"), Some(&target("t1", "s1")));
        assert_eq!(r.get_by_session_id("s1"), Some(&target("t1", "s1")));
    }

    #[test]
    fn detach_by_session_id_is_idempotent() {
        let mut r = TargetSessionRegistry::new();
        r.add(target("t1", "s1"));
        assert!(r.detach_by_session_id("s1").is_some());
        assert!(r.detach_by_session_id("s1").is_none());
        assert!(r.get_by_target_id("t1").is_none());
        assert!(r.get_by_session_id("s1").is_none());
    }

    #[test]
    fn detach_by_target_id_is_idempotent() {
        let mut r = TargetSessionRegistry::new();
        r.add(target("t1", "s1"));
        assert!(r.detach_by_target_id("t1").is_some());
        assert!(r.detach_by_target_id("t1").is_none());
    }

    #[test]
    fn detach_by_one_key_removes_both_mappings() {
        let mut r = TargetSessionRegistry::new();
        r.add(target("t1", "s1"));
        r.detach_by_session_id("s1");
        assert!(r.detach_by_target_id("t1").is_none());
    }

    #[test]
    fn get_attached_page_ids_returns_all_target_ids() {
        let mut r = TargetSessionRegistry::new();
        r.add(target("t1", "s1"));
        r.add(target("t2", "s2"));
        let mut ids = r.get_attached_page_ids();
        ids.sort();
        assert_eq!(ids, vec!["t1".to_string(), "t2".to_string()]);
    }

    #[test]
    fn clear_removes_everything() {
        let mut r = TargetSessionRegistry::new();
        r.add(target("t1", "s1"));
        r.clear();
        assert!(r.get_by_target_id("t1").is_none());
        assert!(r.get_attached_page_ids().is_empty());
    }
}
