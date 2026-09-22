//! KeybindingManager — configurable key-to-action dispatch with conflict
//! detection. 1:1 port of `tui/src/keybindings.ts`.
//!
//! The TS implementation uses `Symbol`-based IDs for stable references and
//! supports user-level overrides via a flat config map. Rust substitutes a
//! monotonically increasing `usize` ID (Symbols are opaque by design; an
//! integer handle provides the same remove/update semantics).
//!
//! # Rebinding (`/keymap` and `~/.future/tui/keybindings.json`)
//!
//! An action's identity is its **description** — the same string
//! [`KeybindingManager::apply_overrides`] matches on, and the key that
//! `keybindings.json` stores. [`KeybindingManager::set_action_key`] moves every
//! entry carrying that description to the new key (several entries can share a
//! description: `ctrl+t` and `shift+tab` are both "Cycle thinking"), and
//! [`KeybindingEntry::default_key`] remembers where "restore defaults" goes.
//!
//! The file is a flat `{ "<description>": "<key>" }` object; `""` unbinds. Key
//! ids are validated with [`validate_binding_key`] before anything is applied —
//! `keys::parse_key` emits canonical camelCase ids (`pageDown`, `pageUp`), and
//! a hand-written `PageDown` in the file would otherwise register a binding no
//! terminal can ever trigger (the same silent-dead-key defect the pager had).
//!
//! With no file, nothing in this module moves a binding: the dispatch path is
//! unchanged, byte for byte.

use std::collections::HashMap;
use std::path::Path;

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
    /// The key [`KeybindingManager::add`] registered this entry with: where
    /// "restore defaults" puts it back, and what makes a file entry that only
    /// restates it a no-op.
    pub default_key: String,
}

/// The bucket an entry is parked in when it is deliberately bound to nothing.
///
/// `dispatch` is only ever called with a non-empty key (`keys::parse_key` never
/// returns one), so an entry parked here can be restored later but can never
/// fire. The read APIs ([`KeybindingManager::get_conflicts`],
/// [`KeybindingManager::get_binding_map`]) skip the bucket so a parked entry
/// does not look like a live binding.
pub const UNBOUND_KEY: &str = "";

/// One action's effective bindings — what the `/keymap` panel lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionBinding {
    /// The action's identity: the string the panel and `keybindings.json` key
    /// on, and therefore also the row's label.
    pub description: String,
    /// Every key the action currently answers to, in registration order and
    /// de-duplicated. Empty means the action is unbound.
    pub keys: Vec<String>,
    /// The keys this build registered the action with — "restore defaults".
    pub default_keys: Vec<String>,
}

/// Shown in place of a key for an action that is bound to nothing.
pub const UNBOUND_TEXT: &str = "(unbound)";

impl ActionBinding {
    /// `"ctrl+p"`, `"ctrl+t, shift+tab"`, `"(unbound)"`.
    pub fn key_text(&self) -> String {
        if self.keys.is_empty() {
            return UNBOUND_TEXT.to_string();
        }
        self.keys.join(", ")
    }

    /// Is the action on exactly the keys this build registered?
    pub fn is_default(&self) -> bool {
        self.keys == self.default_keys
    }

    /// Does the action answer on this key alone (`""` = bound to nothing)?
    pub fn is_bound_to(&self, key: &str) -> bool {
        if key.is_empty() {
            self.keys.is_empty()
        } else {
            self.keys.len() == 1 && self.keys[0] == key
        }
    }
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
            default_key: key.to_string(),
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
        // The parked bucket is not a key: an unbound action can never fire,
        // whatever a caller hands over (the app only ever passes ids
        // `keys::parse_key` returned, and those are never empty).
        if key == UNBOUND_KEY {
            return false;
        }

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
            if key == UNBOUND_KEY {
                continue; // parked, not bound to a key at all
            }
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
            if key == UNBOUND_KEY {
                continue; // parked, not bound to a key at all
            }
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

    // ─── Rebinding (the `/keymap` panel) ───────────────────────────────

    /// Every action, in registration order: its description, the keys it
    /// answers on now and the keys this build registered it with.
    ///
    /// One row per *description*: several entries can share one (the app binds
    /// "Cycle thinking" to both `ctrl+t` and `shift+tab`), and a description —
    /// not an entry — is what a user rebinds and what the file keys on.
    pub fn action_bindings(&self) -> Vec<ActionBinding> {
        // Walk the entries in registration order (ids are monotonic in `add`),
        // not in the map's bucket order: both the action list *and* the keys
        // listed inside one action have to be stable, and a `HashMap` iteration
        // would hand "Cycle thinking" its keys in whichever order it felt like.
        let mut entries: Vec<&KeybindingEntry> = self.bindings.values().flatten().collect();
        entries.sort_by_key(|entry| entry.id);
        let mut found: Vec<ActionBinding> = Vec::new();
        for entry in entries {
            match found
                .iter_mut()
                .find(|action| action.description == entry.description)
            {
                Some(action) => {
                    push_unique_key(&mut action.keys, &entry.key);
                    push_unique_key(&mut action.default_keys, &entry.default_key);
                }
                None => {
                    let mut keys = Vec::new();
                    let mut default_keys = Vec::new();
                    push_unique_key(&mut keys, &entry.key);
                    push_unique_key(&mut default_keys, &entry.default_key);
                    found.push(ActionBinding {
                        description: entry.description.clone(),
                        keys,
                        default_keys,
                    });
                }
            }
        }
        found
    }

    /// The descriptions bound to `key`, in dispatch order: the first one is the
    /// action `dispatch` actually runs (it stops at the first action that
    /// consumes the key). Used by the panel's conflict prompt.
    pub fn actions_on_key(&self, key: &str) -> Vec<String> {
        let Some(entries) = self.bindings.get(key) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for entry in entries {
            if !out.contains(&entry.description) {
                out.push(entry.description.clone());
            }
        }
        out
    }

    /// Every bound key with the actions that answer on it, in key order, each
    /// list in dispatch order (the first one wins).
    ///
    /// This is what the panel needs to answer "is this key free?" for a key
    /// that is not yet a conflict — the state *before* the user overrides it.
    pub fn key_owners(&self) -> Vec<(String, Vec<String>)> {
        let mut keys: Vec<&String> = self.bindings.keys().collect();
        keys.sort();
        let mut out = Vec::new();
        for key in keys {
            if key == UNBOUND_KEY {
                continue;
            }
            out.push((key.clone(), self.actions_on_key(key)));
        }
        out
    }

    /// Every key claimed by more than one *action*, in key order, with the
    /// actions in dispatch order (winner first).
    ///
    /// Distinct from [`Self::get_conflicts`], which counts entries: two entries
    /// of the same action on one key are not a conflict — they are one action
    /// that got rebound onto the key it already half-used.
    pub fn action_conflicts(&self) -> Vec<(String, Vec<String>)> {
        self.key_owners()
            .into_iter()
            .filter(|(_, actions)| actions.len() > 1)
            .collect()
    }

    /// Rebind one action: every entry with this description moves to `key`
    /// (`""` unbinds it).
    ///
    /// The moved action goes to the front of its new key's list, so a binding
    /// that overrides a conflict is the one `dispatch` runs first — the user
    /// asked for it explicitly (the panel says so before applying it). Returns
    /// `false` when no entry carries that description.
    pub fn set_action_key(&mut self, description: &str, key: &str) -> bool {
        let mut all = self.take_all();
        if !all.iter().any(|entry| entry.description == description) {
            self.rebuild_from(all, "");
            return false;
        }
        for entry in all.iter_mut() {
            if entry.description == description {
                entry.key = key.to_string();
            }
        }
        self.rebuild_from(all, description);
        true
    }

    /// Put one action back on the keys this build registered it with.
    /// Returns `false` when no entry carries that description.
    pub fn reset_action(&mut self, description: &str) -> bool {
        let mut all = self.take_all();
        let mut found = false;
        for entry in all.iter_mut() {
            if entry.description == description {
                entry.key = entry.default_key.clone();
                found = true;
            }
        }
        self.rebuild_from(all, "");
        found
    }

    /// Put every action back on the keys this build registered it with. The
    /// rebuild is by entry id, so the result is the state [`Self::add`] left
    /// behind — dispatch order included.
    pub fn reset_all(&mut self) {
        let mut all = self.take_all();
        for entry in all.iter_mut() {
            entry.key = entry.default_key.clone();
        }
        self.rebuild_from(all, "");
    }

    /// The file payload: every action whose keys differ from the registered
    /// ones, as `description → key` (`""` = unbound), sorted by description.
    ///
    /// An untouched manager returns nothing, so a TUI that never rebound
    /// anything writes an empty object instead of freezing today's defaults
    /// into the user's config.
    pub fn assignment_overrides(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .action_bindings()
            .into_iter()
            .filter(|action| !action.is_default())
            .map(|action| {
                let key = action.keys.first().cloned().unwrap_or_default();
                (action.description, key)
            })
            .collect();
        out.sort();
        out
    }

    /// Empty the map into a flat list (every entry, whatever bucket it is in).
    fn take_all(&mut self) -> Vec<KeybindingEntry> {
        let mut all = Vec::new();
        for (_, entries) in std::mem::take(&mut self.bindings) {
            all.extend(entries);
        }
        all
    }

    /// Rebuild the map from a flat list, keeping each key's entries in entry-id
    /// order (the order `add` produced, so `dispatch` stays first-match-wins in
    /// registration order). Entries whose description is `front` go first
    /// inside their bucket — the "the new binding wins" rule of
    /// [`Self::set_action_key`].
    fn rebuild_from(&mut self, mut entries: Vec<KeybindingEntry>, front: &str) {
        entries.sort_by_key(|entry| {
            let is_front = !front.is_empty() && entry.description == front;
            (usize::from(!is_front), entry.id)
        });
        let mut map: HashMap<String, Vec<KeybindingEntry>> = HashMap::new();
        for entry in entries {
            map.entry(entry.key.clone()).or_default().push(entry);
        }
        self.bindings = map;
    }
}

/// Append `key` unless it is empty (unbound) or already listed.
fn push_unique_key(keys: &mut Vec<String>, key: &str) {
    if key.is_empty() || keys.iter().any(|k| k == key) {
        return;
    }
    keys.push(key.to_string());
}

// ─── Key ids the terminal can actually send ───────────────────────────────
//
// A keybinding is dead weight the moment its id is not one `keys::parse_key`
// can return: the app parses raw bytes with `parse_key` and dispatches the id
// it gets back, so a binding spelled `PageDown` (or `esc`, or `ctrl+P`) would
// sit in `keybindings.json` and never fire — the exact class of defect the
// pager had (it matched `pagedown` while `parse_key` emits `pageDown`). The
// tables below are `keys.rs`'s id vocabulary, transcribed; the tests assert
// that the parser agrees by driving real byte sequences (`parse_key`) and
// feeding the result back through [`is_canonical_key_id`].

/// The base names `keys::parse_key` can emit (`key::*` constants, the legacy
/// sequence table and the Kitty functional-key names). Case matters: these are
/// the exact spellings the parser produces.
const NAMED_KEYS: [&str; 28] = [
    "escape",
    "tab",
    "enter",
    "space",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "pageUp",
    "pageDown",
    "up",
    "down",
    "left",
    "right",
    "clear",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

/// The modifier words `keys::parse_key` can emit, in any combination.
const MODIFIERS: [&str; 4] = ["shift", "ctrl", "alt", "super"];

/// Longest id accepted. The longest the parser can produce is
/// `"super+shift+ctrl+alt+f12"` (25); anything longer is not an id.
const MAX_KEY_ID_LEN: usize = 26;

/// Is `key` a key id the terminal can deliver to the dispatcher?
///
/// The accepted grammar is `[<modifier>+]*<base>` with the modifiers from
/// [`MODIFIERS`] and the base either one of [`NAMED_KEYS`] or a single
/// printable character. `""` (unbind) is *not* a valid id — check it before
/// calling this; [`validate_binding_key`] does.
pub fn is_canonical_key_id(key: &str) -> bool {
    if key.is_empty() || key.len() > MAX_KEY_ID_LEN {
        return false;
    }
    if key.chars().any(|c| c.is_control() || c == '\u{1b}') {
        // A raw escape sequence, not an id: the caller is holding bytes.
        return false;
    }
    let mut parts = key.split('+');
    // `split` yields at least one item for any string (the empty one is ruled
    // out above), so a base always exists. `expect` states that invariant
    // instead of an arm no input can reach.
    let base = parts.next_back().expect("split always yields one item");
    let has_modifier = parts.clone().next().is_some();
    if !parts.all(|modifier| MODIFIERS.contains(&modifier)) {
        return false;
    }
    is_canonical_base(base, has_modifier)
}

fn is_canonical_base(base: &str, has_modifier: bool) -> bool {
    if NAMED_KEYS.contains(&base) {
        return true;
    }
    let mut chars = base.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            // A single printable character. With a modifier, a letter is always
            // lowercase in the parser's output (`ctrl+p`, `shift+a`): an
            // uppercase one would be an id nothing ever dispatches (`ctrl+P`).
            c != '+' && !(has_modifier && c.is_ascii_uppercase())
        }
        _ => false,
    }
}

/// The canonical spelling of `key` when only its *case* is wrong, e.g.
/// `"Ctrl+PageDown"` → `"ctrl+PageDown"`… → `"ctrl+pageDown"`.
///
/// Used for the "did you mean" half of a rejection message: a user editing the
/// file by hand is the only source of a mis-cased id, and the case is exactly
/// what makes the binding silently dead.
pub fn canonical_key_suggestion(key: &str) -> Option<String> {
    let mut modifiers: Vec<String> = Vec::new();
    let mut parts: Vec<&str> = key.split('+').collect();
    let base = parts.pop()?;
    for part in parts {
        let lower = part.to_lowercase();
        if !MODIFIERS.contains(&lower.as_str()) || modifiers.contains(&lower) {
            return None;
        }
        modifiers.push(lower);
    }
    let canonical = NAMED_KEYS
        .iter()
        .find(|named| named.eq_ignore_ascii_case(base))?;
    modifiers.push(canonical.to_string());
    Some(modifiers.join("+"))
}

/// Why a key cannot be bound, or `None` when it can.
///
/// These keys never reach the keybinding dispatcher, so a binding on one would
/// be invisible — the panel refuses them instead of writing a dead entry.
pub fn reserved_key_reason(key: &str) -> Option<&'static str> {
    match key {
        "escape" => Some("escape closes a panel or clears the prompt before key dispatch"),
        "ctrl+c" => Some("ctrl+c arrives as the interrupt byte, before the key is ever parsed"),
        "shift+ctrl+d" => Some("shift+ctrl+d is reserved for the debug callback"),
        _ => None,
    }
}

/// Validate a key a user is binding an action to. `""` means "unbind" and is
/// always accepted.
pub fn validate_binding_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Ok(());
    }
    if !is_canonical_key_id(key) {
        let hint = match canonical_key_suggestion(key) {
            Some(suggestion) => format!(" — did you mean \"{suggestion}\"?"),
            None => String::new(),
        };
        return Err(format!("\"{key}\" is not a key id a terminal sends{hint}"));
    }
    if let Some(reason) = reserved_key_reason(key) {
        return Err(format!("\"{key}\" cannot be bound: {reason}"));
    }
    Ok(())
}

// ─── `~/.future/tui/keybindings.json` ─────────────────────────────────────

/// The keybinding-override file the TUI reads and writes. It lives beside
/// `settings.json` in `~/.future/tui/` (deliberately *not* inside it: it is a
/// separate, hand-editable file and the two never had a shared schema).
pub const KEYBINDINGS_FILE: &str = "keybindings.json";

/// A parsed `keybindings.json`. Never a failure: a file that cannot be read or
/// understood leaves the defaults in place and says why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeybindingsFile {
    /// `description → key` in file order (`""` = unbind).
    pub assignments: Vec<(String, String)>,
    /// Human-readable problems, shown by the `/keymap` panel.
    pub problems: Vec<String>,
}

/// Parse the *shape* of `keybindings.json`. Key ids are validated later, by
/// [`validate_binding_key`], because whether a key is acceptable can depend on
/// the action it is bound to (a file restating `Interrupt / exit` → `ctrl+c` is
/// a no-op, not an error).
pub fn parse_keybindings(text: &str) -> KeybindingsFile {
    let mut file = KeybindingsFile::default();
    if text.trim().is_empty() {
        return file;
    }
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            file.problems
                .push(format!("keybindings.json is not valid JSON ({err})"));
            return file;
        }
    };
    let Some(object) = value.as_object() else {
        file.problems
            .push("keybindings.json must be a JSON object of action → key".to_string());
        return file;
    };
    for (description, value) in object {
        match value.as_str() {
            Some(key) => file
                .assignments
                .push((description.clone(), key.to_string())),
            None => file.problems.push(format!(
                "\"{description}\": the binding must be a string key id"
            )),
        }
    }
    file
}

/// Read [`KEYBINDINGS_FILE`] from `path`. A missing file is the normal case
/// (defaults, nothing to report).
pub fn load_keybindings_file(path: &Path) -> KeybindingsFile {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_keybindings(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => KeybindingsFile::default(),
        Err(err) => KeybindingsFile {
            assignments: Vec::new(),
            problems: vec![format!(
                "keybindings.json could not be read ({err}) — using the default keys"
            )],
        },
    }
}

/// Serialize `assignments` as the file body (pretty-printed, trailing
/// newline). Insertion order is preserved, so pass a sorted list.
pub fn serialize_keybindings(assignments: &[(String, String)]) -> String {
    let mut object = serde_json::Map::new();
    for (description, key) in assignments {
        object.insert(description.clone(), serde_json::Value::String(key.clone()));
    }
    let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(object))
        .unwrap_or_else(|_| "{}".to_string());
    text.push('\n');
    text
}

/// Write `assignments` to `path`, creating `~/.future/tui/` when needed.
pub fn save_keybindings_file(path: &Path, assignments: &[(String, String)]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, serialize_keybindings(assignments))
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
        let first_id = km.add(
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
        // With the consumer gone, the second binding fires.
        km.remove("a", Some(first_id));
        km.dispatch("a", None);
        assert_eq!(calls.get(), 2);
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

    #[test]
    fn default_creates_empty_manager() {
        let km = KeybindingManager::default();
        assert!(km.get_bindings().is_empty());
        assert!(km.get_overrides().is_empty());
    }

    #[test]
    fn remove_without_id_removes_all_bindings_for_key() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        km.add("a", Box::new(|| true), "two", None);
        assert!(km.remove("a", None));
        assert!(km.get_bindings().is_empty());
        // Key already gone → false.
        assert!(!km.remove("a", None));
    }

    #[test]
    fn remove_with_id_reports_missing_key_and_missing_id() {
        let mut km = KeybindingManager::new();
        assert!(!km.remove("absent", Some(0)));
        let id = km.add("a", Box::new(|| true), "one", None);
        assert!(!km.remove("a", Some(id + 100)));
        assert!(km.remove("a", Some(id)));
        assert!(km.get_bindings().is_empty());
    }

    #[test]
    fn dispatch_skips_entries_with_mismatched_context() {
        let mut km = KeybindingManager::new();
        km.add(
            "a",
            Box::new(|| true),
            "editor-only",
            Some(KeybindingContext::Editor),
        );
        assert!(!km.dispatch("a", Some(KeybindingContext::Overlay)));
        assert!(km.dispatch("a", Some(KeybindingContext::Editor)));
    }

    #[test]
    fn dispatch_with_override_fires_only_matching_description() {
        let mut km = KeybindingManager::new();
        let calls = Rc::new(Cell::new(0));
        let c = Rc::clone(&calls);
        km.add(
            "a",
            Box::new(move || {
                c.set(c.get() + 1);
                true
            }),
            "first",
            None,
        );
        let c = Rc::clone(&calls);
        km.add(
            "a",
            Box::new(move || {
                c.set(c.get() + 10);
                true
            }),
            "second",
            None,
        );
        let mut overrides = UserOverrideMap::new();
        overrides.insert("a".into(), "second".into());
        km.apply_overrides(overrides.clone());
        assert!(km.dispatch("a", None));
        assert_eq!(calls.get(), 10);
        // Retargeting the override at the other description fires it instead.
        overrides.insert("a".into(), "first".into());
        km.apply_overrides(overrides);
        assert!(km.dispatch("a", None));
        assert_eq!(calls.get(), 11);
    }

    #[test]
    fn dispatch_continues_past_non_consuming_action() {
        let mut km = KeybindingManager::new();
        let calls = Rc::new(Cell::new(0));
        let c = Rc::clone(&calls);
        km.add(
            "a",
            Box::new(move || {
                c.set(c.get() + 1);
                false // does not consume — dispatch continues
            }),
            "first",
            None,
        );
        let c = Rc::clone(&calls);
        km.add(
            "a",
            Box::new(move || {
                c.set(c.get() + 10);
                true
            }),
            "second",
            None,
        );
        assert!(km.dispatch("a", None));
        assert_eq!(calls.get(), 11);
    }

    #[test]
    fn dispatch_with_empty_entry_vec_is_false() {
        // White-box: an empty entry vec (not reachable through the public
        // remove API, which prunes it) is handled defensively.
        let mut km = KeybindingManager::new();
        km.bindings.insert("a".to_string(), Vec::new());
        assert!(!km.dispatch("a", None));
    }

    #[test]
    fn conflicts_fall_through_when_override_matches_nothing() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        km.add("a", Box::new(|| true), "two", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("a".into(), "no-such-description".into());
        km.apply_overrides(overrides);
        // Override resolves to zero entries → the key is still a conflict.
        let conflicts = km.get_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "a");
    }

    #[test]
    fn binding_map_skips_override_with_no_visible_entries() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("a".into(), "no-such-description".into());
        km.apply_overrides(overrides);
        assert!(!km.get_binding_map().contains_key("a"));
    }

    #[test]
    fn conflicts_skip_unbound_and_override_resolved_keys() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        km.add("a", Box::new(|| true), "two", None);
        km.add("b", Box::new(|| true), "x", None);
        km.add("b", Box::new(|| true), "y", None);
        km.add("c", Box::new(|| true), "solo", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("a".into(), String::new()); // unbound — no conflict
        overrides.insert("b".into(), "x".into()); // resolves to exactly one
        km.apply_overrides(overrides);
        assert!(km.get_conflicts().is_empty());
        // A conflicting key with no override still shows up.
        km.add("d", Box::new(|| true), "p", None);
        km.add("d", Box::new(|| true), "q", None);
        let conflicts = km.get_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "d");
        assert_eq!(conflicts[0].1.len(), 2);
    }

    #[test]
    fn binding_map_hides_unbound_and_filters_by_override() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        km.add("a", Box::new(|| true), "two", None);
        km.add("b", Box::new(|| true), "gone", None);
        let mut overrides = UserOverrideMap::new();
        overrides.insert("a".into(), "two".into());
        overrides.insert("b".into(), String::new());
        km.apply_overrides(overrides);
        let map = km.get_binding_map();
        assert_eq!(
            map.get("a").map(Vec::as_slice),
            Some(&["two".to_string()][..])
        );
        assert!(!map.contains_key("b"));
    }

    #[test]
    fn find_by_id_scans_every_key() {
        let mut km = KeybindingManager::new();
        km.add("a", Box::new(|| true), "one", None);
        let id = km.add("b", Box::new(|| true), "two", None);
        assert_eq!(km.find_by_id(id).map(|e| e.key.as_str()), Some("b"));
        assert!(km.find_by_id(id + 100).is_none());
    }

    #[test]
    fn get_overrides_returns_installed_map() {
        let mut km = KeybindingManager::new();
        let mut overrides = UserOverrideMap::new();
        overrides.insert("a".into(), "x".into());
        km.apply_overrides(overrides.clone());
        assert_eq!(km.get_overrides(), overrides);
    }

    // ─── Rebinding ──────────────────────────────────────────────────────

    /// The app's global bindings, as `app.rs` registers them: two entries share
    /// one description ("Cycle thinking"), which is what makes the
    /// description-is-the-action rule testable here.
    fn app_like_manager() -> KeybindingManager {
        let mut km = KeybindingManager::new();
        km.add("ctrl+c", Box::new(|| true), "Interrupt / exit", None);
        km.add("ctrl+p", Box::new(|| true), "Cycle model", None);
        km.add("ctrl+t", Box::new(|| true), "Cycle thinking", None);
        km.add("shift+tab", Box::new(|| true), "Cycle thinking", None);
        km.add("ctrl+r", Box::new(|| true), "Browse sessions", None);
        km
    }

    fn action<'a>(bindings: &'a [ActionBinding], description: &str) -> &'a ActionBinding {
        bindings
            .iter()
            .find(|action| action.description == description)
            .expect("action is registered")
    }

    #[test]
    fn action_bindings_fold_entries_by_description_in_registration_order() {
        let km = app_like_manager();
        let bindings = km.action_bindings();
        let descriptions: Vec<&str> = bindings.iter().map(|a| a.description.as_str()).collect();
        assert_eq!(
            descriptions,
            vec![
                "Interrupt / exit",
                "Cycle model",
                "Cycle thinking",
                "Browse sessions"
            ]
        );
        // Both entries of "Cycle thinking" land on the one row.
        let thinking = action(&bindings, "Cycle thinking");
        assert_eq!(thinking.keys, vec!["ctrl+t", "shift+tab"]);
        assert_eq!(thinking.default_keys, vec!["ctrl+t", "shift+tab"]);
        assert_eq!(thinking.key_text(), "ctrl+t, shift+tab");
        assert!(thinking.is_default());
    }

    #[test]
    fn set_action_key_moves_every_entry_of_the_action() {
        let mut km = app_like_manager();
        assert!(km.set_action_key("Cycle thinking", "ctrl+y"));
        assert!(km.actions_on_key("ctrl+t").is_empty());
        assert!(km.actions_on_key("shift+tab").is_empty());
        assert_eq!(km.actions_on_key("ctrl+y"), vec!["Cycle thinking"]);
        let bindings = km.action_bindings();
        let thinking = action(&bindings, "Cycle thinking");
        assert_eq!(thinking.keys, vec!["ctrl+y"]);
        assert_eq!(thinking.default_keys, vec!["ctrl+t", "shift+tab"]);
        assert!(!thinking.is_default());
        assert!(km.dispatch("ctrl+y", None));
        assert!(!km.dispatch("ctrl+t", None), "the old key is free now");
    }

    #[test]
    fn set_action_key_unknown_description_returns_false() {
        let mut km = app_like_manager();
        assert!(!km.set_action_key("no such action", "ctrl+y"));
        assert!(km.actions_on_key("ctrl+y").is_empty());
        // The manager is left exactly as it was.
        assert_eq!(km.actions_on_key("ctrl+t"), vec!["Cycle thinking"]);
    }

    #[test]
    fn the_newly_bound_action_wins_a_conflicting_key() {
        let mut km = app_like_manager();
        // `Browse sessions` already owns ctrl+r; the user overrides knowingly.
        assert_eq!(km.action_conflicts().len(), 0);
        km.set_action_key("Cycle model", "ctrl+r");
        let conflicts = km.action_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "ctrl+r");
        assert_eq!(conflicts[0].1, vec!["Cycle model", "Browse sessions"]);
        // Dispatch order follows the conflict list: the override runs, the
        // shadowed action does not.
        let calls: Rc<std::cell::RefCell<Vec<&'static str>>> =
            Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut km = KeybindingManager::new();
        let c = Rc::clone(&calls);
        km.add(
            "ctrl+r",
            Box::new(move || {
                c.borrow_mut().push("browse");
                true
            }),
            "Browse sessions",
            None,
        );
        let c = Rc::clone(&calls);
        km.add(
            "ctrl+p",
            Box::new(move || {
                c.borrow_mut().push("model");
                true
            }),
            "Cycle model",
            None,
        );
        // Before the move, the key runs the action that owns it. (An assertion,
        // not just a call: this is the branch the override below shadows.)
        assert!(km.dispatch("ctrl+r", None));
        assert_eq!(*calls.borrow(), vec!["browse"]);
        calls.borrow_mut().clear();
        km.set_action_key("Cycle model", "ctrl+r");
        assert!(km.dispatch("ctrl+r", None));
        assert_eq!(*calls.borrow(), vec!["model"]);
        assert_eq!(
            km.actions_on_key("ctrl+r"),
            vec!["Cycle model", "Browse sessions"]
        );
    }

    #[test]
    fn unbinding_parks_the_entry_so_nothing_dispatches_it() {
        let mut km = app_like_manager();
        assert!(km.set_action_key("Cycle model", UNBOUND_KEY));
        assert!(!km.dispatch("ctrl+p", None));
        assert!(
            !km.dispatch(UNBOUND_KEY, None),
            "the parked bucket is inert"
        );
        let bindings = km.action_bindings();
        let model = action(&bindings, "Cycle model");
        assert!(model.keys.is_empty());
        assert_eq!(model.key_text(), UNBOUND_TEXT);
        assert!(!model.is_default());
        assert!(model.is_bound_to(UNBOUND_KEY));
        // A parked entry is not a live binding: no help row, no conflict.
        assert!(!km.get_binding_map().contains_key("ctrl+p"));
        assert!(km.get_conflicts().is_empty());
    }

    #[test]
    fn reset_action_restores_one_action_and_reports_unknown_descriptions() {
        let mut km = app_like_manager();
        km.set_action_key("Cycle model", "ctrl+y");
        km.set_action_key("Cycle thinking", "ctrl+u");
        assert!(km.reset_action("Cycle model"));
        let bindings = km.action_bindings();
        assert!(action(&bindings, "Cycle model").is_default());
        assert!(!action(&bindings, "Cycle thinking").is_default());
        assert!(!km.reset_action("no such action"));
    }

    #[test]
    fn reset_all_restores_the_registered_state_exactly() {
        let fresh = app_like_manager();
        let mut km = app_like_manager();
        km.set_action_key("Cycle model", "ctrl+r");
        km.set_action_key("Cycle thinking", UNBOUND_KEY);
        km.set_action_key("Browse sessions", "ctrl+p");
        km.reset_all();
        assert_eq!(km.get_binding_map(), fresh.get_binding_map());
        assert_eq!(km.action_conflicts(), fresh.action_conflicts());
        // Dispatch order too: the rebuilt map is in registration order.
        let fresh_actions: Vec<ActionBinding> = fresh.action_bindings();
        assert_eq!(km.action_bindings(), fresh_actions);
        assert_eq!(km.assignment_overrides(), Vec::new());
    }

    #[test]
    fn a_key_shared_by_two_entries_of_one_action_is_not_a_conflict() {
        let mut km = app_like_manager();
        // `shift+tab` is already one of "Cycle thinking"'s keys: rebinding the
        // action onto it collapses the two entries instead of reporting a
        // conflict with itself.
        km.set_action_key("Cycle thinking", "shift+tab");
        assert!(km.action_conflicts().is_empty());
        assert_eq!(km.actions_on_key("shift+tab"), vec!["Cycle thinking"]);
        assert!(km.dispatch("shift+tab", None));
    }

    #[test]
    fn a_rebound_action_is_listed_first_at_a_shared_key() {
        let mut km = app_like_manager();
        // Registered order puts "Cycle thinking" first at ctrl+t; the action
        // the user just moved there is the one that must win, so the conflict
        // list (and dispatch) lead with it and the shadowed action follows.
        km.set_action_key("Browse sessions", "ctrl+t");
        let conflicts = km.action_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "ctrl+t");
        assert_eq!(conflicts[0].1, vec!["Browse sessions", "Cycle thinking"]);
    }

    #[test]
    fn assignment_overrides_lists_only_changed_actions_in_key_order() {
        let mut km = app_like_manager();
        assert!(km.assignment_overrides().is_empty());
        km.set_action_key("Cycle model", "ctrl+y");
        km.set_action_key("Browse sessions", UNBOUND_KEY);
        assert_eq!(
            km.assignment_overrides(),
            vec![
                ("Browse sessions".to_string(), String::new()),
                ("Cycle model".to_string(), "ctrl+y".to_string()),
            ]
        );
    }

    // ─── Key id validation ──────────────────────────────────────────────

    #[test]
    fn canonical_key_ids_cover_every_id_the_parser_emits() {
        use crate::keys::parse_key;
        // The byte sequences are what a real terminal sends for these keys
        // (the same ones `keys.rs`' tests drive); `parse_key` is the only
        // producer of a key id in the whole TUI, so every id it returns for
        // them must be bindable.
        let sequences: [(&str, &str); 22] = [
            ("\x1b[A", "up"),
            ("\x1b[B", "down"),
            ("\x1b[C", "right"),
            ("\x1b[D", "left"),
            ("\x1b[1~", "home"),
            ("\x1b[4~", "end"),
            ("\x1b[5~", "pageUp"),
            ("\x1b[[6~", "pageDown"),
            ("\t", "tab"),
            ("\r", "enter"),
            (" ", "space"),
            ("\x7f", "backspace"),
            ("\x1b[2~", "insert"),
            ("\x1b[3~", "delete"),
            ("\x1b[Z", "shift+tab"),
            ("\x1b[15~", "f5"),
            ("\x1b[24~", "f12"),
            ("\x01", "ctrl+a"),
            ("\x1b[1;5C", "ctrl+right"),
            ("\x1b[1;2A", "shift+up"),
            ("\x1b[3;5~", "ctrl+delete"),
            ("\x1bb", "alt+left"),
        ];
        for (data, id) in sequences {
            assert_eq!(parse_key(data).as_deref(), Some(id), "parse_key({data:?})");
            assert!(is_canonical_key_id(id), "{id:?} must be bindable");
        }
        // Modifier combinations the parser can produce from Kitty CSI-u.
        for id in [
            "shift+ctrl+alt+f12",
            "super+a",
            "ctrl+pageUp",
            "shift+pageDown",
            "alt+space",
            "shift+clear",
            "ctrl+\\",
            "ctrl+-",
            "ctrl+]",
            "9",
        ] {
            assert!(is_canonical_key_id(id), "{id:?} must be bindable");
        }
    }

    #[test]
    fn canonical_key_ids_reject_spellings_no_terminal_sends() {
        for id in [
            "",         // unbind is not an id
            "PageDown", // the pitfall: the parser emits camelCase
            "pageup",
            "PageUp",
            "pagedown",
            "esc",
            "return",
            "ctrl+P",
            "Ctrl+p",
            "CTRL+p",
            "ctrl++a",
            "ctrl+",
            "+",
            "\x1b[1;5C", // raw bytes, not an id
            "\x1b",
            "ctrl+\t",
            "super+shift+ctrl+alt+f12+x",
            "nonsense",
        ] {
            assert!(!is_canonical_key_id(id), "{id:?} must be rejected");
        }
    }

    #[test]
    fn canonical_key_suggestion_fixes_the_case_only() {
        assert_eq!(
            canonical_key_suggestion("PageDown").as_deref(),
            Some("pageDown")
        );
        assert_eq!(
            canonical_key_suggestion("Ctrl+PageUp").as_deref(),
            Some("ctrl+pageUp")
        );
        assert_eq!(
            canonical_key_suggestion("SHIFT+ctrl+HOME").as_deref(),
            Some("shift+ctrl+home")
        );
        // Nothing useful to suggest for an unknown base or a bad modifier.
        assert_eq!(canonical_key_suggestion("nonsense"), None);
        assert_eq!(canonical_key_suggestion("meta+p"), None);
    }

    #[test]
    fn validate_binding_key_accepts_unbind_and_parser_ids() {
        assert_eq!(validate_binding_key(UNBOUND_KEY), Ok(()));
        assert_eq!(validate_binding_key("ctrl+p"), Ok(()));
        assert_eq!(validate_binding_key("pageDown"), Ok(()));
    }

    #[test]
    fn validate_binding_key_rejects_reserved_and_unreachable_keys() {
        let escape = validate_binding_key("escape").unwrap_err();
        assert!(escape.contains("cannot be bound"), "{escape}");
        let interrupt = validate_binding_key("ctrl+c").unwrap_err();
        assert!(interrupt.contains("interrupt byte"), "{interrupt}");
        let debug = validate_binding_key("shift+ctrl+d").unwrap_err();
        assert!(debug.contains("debug callback"), "{debug}");
        let mistyped = validate_binding_key("PageDown").unwrap_err();
        assert!(mistyped.contains("did you mean \"pageDown\""), "{mistyped}");
        let bytes = validate_binding_key("\x1b[1;5C").unwrap_err();
        assert!(bytes.contains("not a key id"), "{bytes}");
    }

    #[test]
    fn reserved_keys_are_the_ones_the_app_consumes_before_dispatch() {
        // A sanity link to app.rs: these are exactly the special cases in
        // `App::handle_key` / `handle_input_continue` that never reach
        // `KeybindingManager::dispatch`.
        assert!(reserved_key_reason("escape").is_some());
        assert!(reserved_key_reason("ctrl+c").is_some());
        assert!(reserved_key_reason("shift+ctrl+d").is_some());
        assert!(reserved_key_reason("ctrl+p").is_none());
        assert!(reserved_key_reason("enter").is_none());
    }

    // ─── keybindings.json ───────────────────────────────────────────────

    #[test]
    fn parse_reads_a_flat_object_in_file_order() {
        let file = parse_keybindings(r#"{"Cycle model":"ctrl+y","Scroll chat up":""}"#);
        assert_eq!(file.problems, Vec::<String>::new());
        assert_eq!(
            file.assignments,
            vec![
                ("Cycle model".to_string(), "ctrl+y".to_string()),
                ("Scroll chat up".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn parse_of_an_empty_or_missing_file_is_defaults_without_complaint() {
        assert_eq!(parse_keybindings(""), KeybindingsFile::default());
        assert_eq!(parse_keybindings("  \n\t"), KeybindingsFile::default());
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("keybindings.json");
        assert_eq!(load_keybindings_file(&missing), KeybindingsFile::default());
    }

    #[test]
    fn parse_reports_broken_shapes_instead_of_panicking() {
        let broken = parse_keybindings("{not json");
        assert_eq!(broken.assignments, Vec::<(String, String)>::new());
        assert_eq!(broken.problems.len(), 1);
        assert!(
            broken.problems[0].contains("not valid JSON"),
            "{:?}",
            broken.problems
        );

        let array = parse_keybindings("[]");
        assert!(array.problems[0].contains("must be a JSON object"));

        let number = parse_keybindings(r#"{"Cycle model": 5}"#);
        assert!(number.assignments.is_empty());
        assert!(
            number.problems[0].contains("must be a string"),
            "{:?}",
            number.problems
        );

        // A whole file of numbers is reported, not silently dropped.
        let nested = parse_keybindings(r#"{"Cycle model": {"key": "ctrl+y"}}"#);
        assert_eq!(nested.problems.len(), 1);
    }

    #[test]
    fn an_unreadable_file_is_reported_and_leaves_the_defaults_alone() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: `read_to_string` fails with
        // something other than NotFound.
        let file = load_keybindings_file(dir.path());
        assert!(file.assignments.is_empty());
        assert_eq!(file.problems.len(), 1);
        assert!(
            file.problems[0].contains("could not be read"),
            "{:?}",
            file.problems
        );
    }

    #[test]
    fn round_trip_through_the_file_keeps_every_assignment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(KEYBINDINGS_FILE);
        let assignments = vec![
            ("Browse sessions".to_string(), String::new()),
            ("Cycle model".to_string(), "ctrl+y".to_string()),
        ];
        save_keybindings_file(&path, &assignments).unwrap();
        assert!(path.exists());
        assert_eq!(load_keybindings_file(&path).assignments, assignments);
        // A missing parent directory is created rather than failing the save.
        assert!(path.parent().unwrap().is_dir());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, serialize_keybindings(&assignments));
        assert!(text.ends_with("}\n"), "{text}");
    }

    #[test]
    fn an_empty_assignment_list_writes_an_empty_object() {
        assert_eq!(serialize_keybindings(&[]), "{}\n");
        assert_eq!(parse_keybindings("{}\n").assignments, Vec::new());
    }

    /// The two ways a path can have no directory to create: an empty parent (a
    /// single-component relative path — a bare file name belongs in the current
    /// directory) and no parent at all (a filesystem root, and the empty path).
    /// Neither may reach `create_dir_all`, which would be asked to create `""`;
    /// the write is what reports the problem, and it reports it as an error
    /// rather than a panic.
    #[test]
    fn a_path_with_no_directory_to_create_skips_the_mkdir() {
        // The ordinary case is untouched: the parent is created on the way.
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("tui").join(KEYBINDINGS_FILE);
        save_keybindings_file(&nested, &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&nested).unwrap(), "{}\n");

        // `".."` is the empty-parent case, `"/"` the no-parent one (verified
        // against `Path::parent` on the pinned toolchain).
        for path in [Path::new(".."), Path::new("/")] {
            assert!(save_keybindings_file(path, &[]).is_err(), "{path:?}");
        }
    }
}
