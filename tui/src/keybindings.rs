//! KeybindingManager — configurable key-to-action dispatch with conflict
//! detection. 1:1 port of `tui/src/keybindings.ts`.
//!
//! The TS implementation uses `Symbol`-based IDs for stable references and
//! supports user-level overrides via a flat config map. Rust substitutes a
//! monotonically increasing `usize` ID (Symbols are opaque by design; an
//! integer handle provides the same remove/update semantics).

use std::collections::HashMap;

/// Contexts a binding can be scoped to (TS `KeybindingContext`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeybindingContext {
    Global,
    Editor,
    Overlay,
    Autocomplete,
}

/// Unique opaque ID returned when registering a binding.
pub type KeybindingId = usize;

pub struct KeybindingEntry {
    pub id: KeybindingId,
    pub key: String,
    pub action: Box<dyn FnMut() -> bool>, // returns true if consumed
    pub description: String,
    pub context: Option<KeybindingContext>,
}

/// User-level overrides: a flat map from key ID to the new action description.
/// Applied on top of programmatic bindings. "key" → "" removes the binding
/// entirely.
pub type UserOverrideMap = HashMap<String, String>;

pub struct KeybindingManager {
    bindings: HashMap<String, Vec<KeybindingEntry>>,
    overrides: UserOverrideMap,
    next_id: KeybindingId,
}

impl Default for KeybindingManager {
    fn default() -> Self {
        Self::new()
    }
}

impl KeybindingManager {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            overrides: HashMap::new(),
            next_id: 0,
        }
    }

    /// Register a keybinding. Returns a unique ID that can be used to remove
    /// or update this specific binding later.
    pub fn add(
        &mut self,
        key: &str,
        action: Box<dyn FnMut() -> bool>,
        description: &str,
        context: Option<KeybindingContext>,
    ) -> KeybindingId {
        let id = self.next_id;
        self.next_id += 1;
        let entry = KeybindingEntry {
            id,
            key: key.to_string(),
            action,
            description: description.to_string(),
            context,
        };
        self.bindings
            .entry(key.to_string())
            .or_default()
            .push(entry);
        id
    }

    /// Remove a specific binding by its ID. If `id` is None, removes all
    /// bindings for the key.
    pub fn remove(&mut self, key: &str, id: Option<KeybindingId>) -> bool {
        let Some(id) = id else {
            return self.bindings.remove(key).is_some();
        };
        let Some(entries) = self.bindings.get_mut(key) else {
            return false;
        };
        let Some(idx) = entries.iter().position(|e| e.id == id) else {
            return false;
        };
        entries.remove(idx);
        if entries.is_empty() {
            self.bindings.remove(key);
        }
        true
    }

    /// Apply user-level keybinding overrides (e.g. from
    /// ~/.future/tui/keybindings.json). Matching is by description.
    pub fn apply_overrides(&mut self, overrides: UserOverrideMap) {
        self.overrides = overrides;
    }

    /// Dispatch a key to registered bindings. Runs all matching bindings in
    /// registration order; stops on the first that returns true. Returns true
    /// if any binding consumed the key.
    pub fn dispatch(&mut self, key: &str, context: Option<KeybindingContext>) -> bool {
        // Check user overrides first: if key is mapped to "", skip entirely
        let override_desc = self.overrides.get(key).cloned();
        if override_desc.as_deref() == Some("") {
            return false;
        }

        let Some(entries) = self.bindings.get_mut(key) else {
            return false;
        };
        if entries.is_empty() {
            return false;
        }

        for entry in entries {
            // If user has an override for this key, only fire the matching
            // description
            if let Some(od) = &override_desc {
                if &entry.description != od {
                    continue;
                }
            }
            if let (Some(ctx), Some(entry_ctx)) = (context, entry.context) {
                if entry_ctx != ctx {
                    continue;
                }
            }
            if (entry.action)() {
                return true;
            }
        }
        false
    }

    /// Return all registered entries.
    pub fn get_bindings(&self) -> Vec<&KeybindingEntry> {
        let mut all: Vec<&KeybindingEntry> = Vec::new();
        for entries in self.bindings.values() {
            all.extend(entries.iter());
        }
        all
    }

    /// Return bindings that have more than one entry for the same key,
    /// excluding those resolved by user overrides.
    pub fn get_conflicts(&self) -> Vec<(&str, Vec<&KeybindingEntry>)> {
        let mut conflicts: Vec<(&str, Vec<&KeybindingEntry>)> = Vec::new();
        let mut keys: Vec<&String> = self.bindings.keys().collect();
        keys.sort();
        for key in keys {
            let entries = &self.bindings[key];
            let override_desc = self.overrides.get(key);
            if let Some(od) = override_desc {
                if od.is_empty() {
                    continue; // unbind — no conflict
                }
                let resolved = entries.iter().filter(|e| &e.description == od).count();
                if resolved == 1 {
                    continue; // resolved by override
                }
            }
            if entries.len() > 1 {
                conflicts.push((key, entries.iter().collect()));
            }
        }
        conflicts
    }

    /// Flattened map: key → descriptions (for help display).
    pub fn get_binding_map(&self) -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        let mut keys: Vec<&String> = self.bindings.keys().collect();
        keys.sort();
        for key in keys {
            let entries = &self.bindings[key];
            let override_desc = self.overrides.get(key);
            let mut visible: Vec<&KeybindingEntry> = entries.iter().collect();
            if let Some(od) = override_desc {
                if od.is_empty() {
                    continue;
                }
                visible = entries.iter().filter(|e| &e.description == od).collect();
            }
            if !visible.is_empty() {
                map.insert(
                    key.clone(),
                    visible.iter().map(|e| e.description.clone()).collect(),
                );
            }
        }
        map
    }

    /// Find a binding by its ID.
    pub fn find_by_id(&self, id: KeybindingId) -> Option<&KeybindingEntry> {
        for entries in self.bindings.values() {
            if let Some(found) = entries.iter().find(|e| e.id == id) {
                return Some(found);
            }
        }
        None
    }

    /// Get the user override map.
    pub fn get_overrides(&self) -> UserOverrideMap {
        self.overrides.clone()
    }

    /// Remove all bindings.
    pub fn clear(&mut self) {
        self.bindings.clear();
        self.overrides.clear();
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn add_and_dispatch_consumed_action() {
        let mut km = KeybindingManager::new();
        km.add("ctrl+p", Box::new(|| true), "cycle model", None);
        assert!(km.dispatch("ctrl+p", None));
    }

    #[test]
    fn dispatch_returns_false_for_unbound_key() {
        let mut km = KeybindingManager::new();
        assert!(!km.dispatch("ctrl+x", None));
    }

    #[test]
    fn dispatch_stops_at_first_consuming_action() {
        let mut km = KeybindingManager::new();
        let calls = Rc::new(Cell::new(0));
        let c1 = Rc::clone(&calls);
        let c2 = Rc::clone(&calls);
        km.add(
            "a",
            Box::new(move || {
                c1.set(c1.get() + 1);
                true
            }),
            "first",
            None,
        );
        km.add(
            "a",
            Box::new(move || {
                c2.set(c2.get() + 1);
                true
            }),
            "second",
            None,
        );
        km.dispatch("a", None);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn dispatch_tries_next_action_when_not_consumed() {
        let mut km = KeybindingManager::new();
        let calls = Rc::new(Cell::new(0));
        let c1 = Rc::clone(&calls);
        let c2 = Rc::clone(&calls);
        km.add(
            "a",
            Box::new(move || {
                c1.set(c1.get() + 1);
                false
            }),
            "first",
            None,
        );
        km.add(
            "a",
            Box::new(move || {
                c2.set(c2.get() + 1);
                true
            }),
            "second",
            None,
        );
        assert!(km.dispatch("a", None));
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn remove_by_id_removes_specific_binding() {
        let mut km = KeybindingManager::new();
        let id1 = km.add("a", Box::new(|| true), "one", None);
        let _id2 = km.add("a", Box::new(|| true), "two", None);
        assert!(km.remove("a", Some(id1)));
        assert_eq!(km.get_bindings().len(), 1);
        assert_eq!(km.get_bindings()[0].description, "two");
    }

    #[test]
    fn remove_all_for_key_when_no_id() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        km.add("a", Box::new(|| true), "two", None);
        assert!(km.remove("a", None));
        assert!(!km.dispatch("a", None));
    }

    #[test]
    fn remove_missing_returns_false() {
        let mut km = KeybindingManager::new();
        assert!(!km.remove("nope", None));
        let id = km.add("a", Box::new(|| true), "x", None);
        assert!(!km.remove("a", Some(id + 100)));
    }

    #[test]
    fn context_filtering() {
        let mut km = KeybindingManager::new();
        km.add(
            "enter",
            Box::new(|| true),
            "submit",
            Some(KeybindingContext::Editor),
        );
        // Without context: entry.context is Some(Editor), caller has no context
        // → TS: `if (context && entry.context && entry.context !== context)` —
        // caller context undefined → no filter → fires.
        assert!(km.dispatch("enter", None));
        // With a mismatched context: filtered out.
        assert!(!km.dispatch("enter", Some(KeybindingContext::Overlay)));
        // With the matching context: fires.
        assert!(km.dispatch("enter", Some(KeybindingContext::Editor)));
    }

    #[test]
    fn unbind_override_skips_dispatch() {
        let mut km = KeybindingManager::new();
        km.add("ctrl+p", Box::new(|| true), "cycle model", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("ctrl+p".into(), String::new());
        km.apply_overrides(overrides);
        assert!(!km.dispatch("ctrl+p", None));
    }

    #[test]
    fn description_override_keeps_only_matching_binding() {
        let mut km = KeybindingManager::new();
        km.add("ctrl+r", Box::new(|| true), "browse sessions", None);
        km.add("ctrl+r", Box::new(|| false), "other action", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("ctrl+r".into(), "browse sessions".into());
        km.apply_overrides(overrides);
        assert!(km.dispatch("ctrl+r", None));
    }

    #[test]
    fn conflicts_reported_without_override() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        km.add("a", Box::new(|| true), "two", None);
        let conflicts = km.get_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "a");
    }

    #[test]
    fn conflicts_resolved_by_override_are_skipped() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        km.add("a", Box::new(|| true), "two", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("a".into(), "one".into());
        km.apply_overrides(overrides);
        assert!(km.get_conflicts().is_empty());
    }

    #[test]
    fn binding_map_lists_descriptions_per_key() {
        let mut km = KeybindingManager::new();
        km.add("ctrl+p", Box::new(|| true), "cycle model", None);
        km.add("tab", Box::new(|| true), "autocomplete", None);
        let map = km.get_binding_map();
        assert_eq!(
            map.get("ctrl+p").map(|v| v.as_slice()),
            Some(&["cycle model".to_string()][..])
        );
        assert_eq!(
            map.get("tab").map(|v| v.as_slice()),
            Some(&["autocomplete".to_string()][..])
        );
    }

    #[test]
    fn find_by_id_locates_registered_entry() {
        let mut km = KeybindingManager::new();
        let id = km.add("a", Box::new(|| true), "x", None);
        let found = km.find_by_id(id).unwrap();
        assert_eq!(found.key, "a");
        assert_eq!(found.description, "x");
    }

    #[test]
    fn clear_removes_everything() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "x", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("b".into(), String::new());
        km.apply_overrides(overrides);
        km.clear();
        assert!(km.get_bindings().is_empty());
        assert!(km.get_overrides().is_empty());
    }
}
