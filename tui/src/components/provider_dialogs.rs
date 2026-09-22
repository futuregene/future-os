//! Provider & model configuration dialogs (`/providers`).
//!
//! Two state machines, both deliberately I/O-free so the app loop can drive
//! them synchronously (same contract as [`crate::components::menu`]):
//!
//! * [`ProviderListState`] — the floating `/providers` picker. It is a thin
//!   *policy* layer over [`MenuState`]: `MenuState` owns highlight, incremental
//!   search, the scroll window and the two tabs (built-in / custom), while this
//!   module maps the highlighted row plus an action key to a
//!   [`ProviderListAction`] the caller turns into an RPC.
//! * [`ProviderForm`] — the add/edit dialog, a tabbed field form with a nested
//!   editable model list. `ApiType` cycles over the three dialects the agent
//!   accepts; `Id` is locked while editing (the agent keys the mutation by id,
//!   so a renamed id would silently create a *second* provider — the desktop
//!   dialog disables the field for the same reason).
//!
//! Design notes:
//!
//! * **Nothing is validated until submit.** The form accepts any text and runs
//!   [`validate_provider_input`] on `enter`; a rejected submit leaves the
//!   message in [`ProviderForm::error`] and returns [`ProviderFormAction::None`]
//!   so the caller cannot mistake it for a queued mutation. Editing any field
//!   clears the message, so the error always describes the *current* input.
//! * **`api_key` empty means "unchanged".** The agent never sends a key back,
//!   so an edit form that prefilled one would have to invent it; blank stays
//!   blank and [`ProviderForm::to_input`] maps it to `api_key: None`
//!   (`ProviderInput::effective_api_key` does the same normalisation on the
//!   wire).
//! * **Model rows only edit their `id`.** `ProviderModelInput` keeps
//!   name/context window/limits/prices explicitly so a round-trip cannot reset
//!   them (`upsert_provider` replaces the whole model list), and the row keeps
//!   every one of those fields untouched — typing into a row edits its id (and
//!   keeps the name in sync while it still mirrors the id).
//! * **Rendering is width-total.** Every row [`ProviderForm::render`] and
//!   [`ProviderListState::render`] produce has exactly `width` visible columns,
//!   for any width including `0` and `1`, and never panics.
//!
//! Keys for the list — while the search row is empty, `a` add, `e` edit,
//! `k` set API key, `d` delete, `s` sync models, `r` reload auth, `/` search,
//! `enter` primary action, `esc` close. Once a search is active every printable
//! key goes to the filter until `esc` clears it (so a query may start with any
//! letter).

use crate::components::menu::{
    MenuAction, MenuItem, MenuOptions, MenuSection, MenuState, MenuTab, MENU_MAX_ROWS,
};
use crate::rpc::provider_types::{
    validate_provider_input, ProviderInfo, ProviderInput, ProviderModelInput, API_TYPES,
    MAX_PROVIDER_MODELS,
};
use crate::theme::{bold, fg, Theme, C};
use crate::tui::Component;
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

// ─── Shared helpers ────────────────────────────────────────────────────────

/// Tab ids of the provider picker (also the values a caller can compare on).
pub const PROVIDER_TAB_BUILTIN: &str = "builtin";
/// Tab id of the custom-provider tab.
pub const PROVIDER_TAB_CUSTOM: &str = "custom";

/// Model rows the form shows at once before it starts scrolling.
const MAX_VISIBLE_MODELS: usize = 6;
/// Longest mask rendered for a non-empty API key (never leaks its length).
const API_KEY_MASK_LEN: usize = 12;

/// A single printable key character, or `None` for anything with a modifier or
/// a multi-character name (`enter`, `shift+tab`, …).
fn printable(key: &str) -> Option<char> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if chars.next().is_some() || first.is_control() || key.contains('+') {
        return None;
    }
    Some(first)
}

/// Pad/truncate a row to exactly `width` visible columns.
fn fit_row(content: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let clipped = if visible_width(content) > width {
        truncate_to_width(content, width, &TruncateOptions::default())
    } else {
        content.to_string()
    };
    let padding = width.saturating_sub(visible_width(&clipped));
    if padding == 0 {
        clipped
    } else {
        format!("{clipped}{}", " ".repeat(padding))
    }
}

/// A hint row (the form's key legend): like [`fit_row`], except a row too wide
/// for the pane ends in an ellipsis so the cut is visible (`esc cancel` →
/// `esc cance…`) rather than looking like a typo. The ellipsis takes the last
/// column, so the row still spans exactly `width`.
fn fit_hint_row(content: &str, width: usize) -> String {
    let clipped = truncate_to_width(
        content,
        width,
        &TruncateOptions {
            ellipsis: true,
            pad: false,
        },
    );
    fit_row(&clipped, width)
}

/// Current visible width of a row (tests use it to assert the width contract).
#[cfg(test)]
fn row_width(row: &str) -> usize {
    visible_width(row)
}

// ─── Form: fields ──────────────────────────────────────────────────────────

/// Which mode a [`ProviderForm`] is in. `Add` sets the mutation's
/// `create_only` flag so a racing writer cannot silently be overwritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFormMode {
    Add,
    Edit,
}

impl ProviderFormMode {
    /// `true` for [`ProviderFormMode::Add`].
    pub fn is_add(self) -> bool {
        matches!(self, ProviderFormMode::Add)
    }
}

/// One editable field of the provider form, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderFormField {
    Id,
    Name,
    ApiType,
    BaseUrl,
    ApiKey,
    Models,
}

impl ProviderFormField {
    /// Tab order, used by `tab`/`shift+tab` and by `render`.
    pub const ALL: [ProviderFormField; 6] = [
        ProviderFormField::Id,
        ProviderFormField::Name,
        ProviderFormField::ApiType,
        ProviderFormField::BaseUrl,
        ProviderFormField::ApiKey,
        ProviderFormField::Models,
    ];

    /// Short label shown in the form row.
    pub fn label(self) -> &'static str {
        match self {
            ProviderFormField::Id => "ID",
            ProviderFormField::Name => "Name",
            ProviderFormField::ApiType => "API",
            ProviderFormField::BaseUrl => "Base URL",
            ProviderFormField::ApiKey => "API key",
            ProviderFormField::Models => "Models",
        }
    }

    /// The next field in tab order (wraps).
    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// The previous field in tab order (wraps).
    pub fn prev(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// What one key press did to the form.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderFormAction {
    /// The key was not handled, or a submit was rejected (message in
    /// [`ProviderForm::error`]).
    None,
    /// Form state changed (focus, text, model list) — redraw.
    Changed,
    /// `enter` on a valid form: the mutation to apply.
    Submit(ProviderInput),
    /// `escape`.
    Cancel,
}

// ─── Form: state ───────────────────────────────────────────────────────────

/// The add/edit provider dialog.
///
/// The text fields are public so the app (and the tests) can prefill or inspect
/// them without a setter per field; every mutation through
/// [`ProviderForm::handle_key`] keeps `dirty` accurate.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderForm {
    pub mode: ProviderFormMode,
    pub id: String,
    pub name: String,
    /// Wire field `api`; one of `API_TYPES` when it came from this UI.
    pub api_type: String,
    pub base_url: String,
    /// Empty means "keep the stored key" (`None` after `to_input`).
    pub api_key: String,
    pub models: Vec<ProviderModelInput>,
    pub field: ProviderFormField,
    /// Last validation failure, cleared by the next edit.
    pub error: Option<String>,
    /// Whether the user changed anything (a pristine form can be discarded
    /// without a confirmation).
    pub dirty: bool,
    /// Index of the highlighted model row (only meaningful when `field` is
    /// `Models`).
    pub model_index: usize,
    /// The provider being edited (always `Some` in `Edit` mode) — the id the
    /// mutation is keyed by, which `id` can never deviate from.
    pub original_id: Option<String>,
}

impl ProviderForm {
    /// An empty form for creating a provider.
    pub fn add() -> Self {
        Self {
            mode: ProviderFormMode::Add,
            id: String::new(),
            name: String::new(),
            api_type: API_TYPES[0].to_string(),
            base_url: String::new(),
            api_key: String::new(),
            models: Vec::new(),
            field: ProviderFormField::Id,
            error: None,
            dirty: false,
            model_index: 0,
            original_id: None,
        }
    }

    /// A form prefilled from an existing provider. The API key is left blank
    /// ("unchanged"); focus starts on `Name` because the id is locked.
    pub fn edit(info: &ProviderInfo) -> Self {
        let api_type = if info.api_type.trim().is_empty() {
            API_TYPES[0].to_string()
        } else {
            info.api_type.trim().to_string()
        };
        Self {
            mode: ProviderFormMode::Edit,
            id: info.id.clone(),
            name: info.name.clone(),
            api_type,
            base_url: info.base_url.clone(),
            api_key: String::new(),
            models: info.models.clone(),
            field: ProviderFormField::Name,
            error: None,
            dirty: false,
            model_index: 0,
            original_id: Some(info.id.clone()),
        }
    }

    /// `true` when the id field ignores edits (edit mode).
    pub fn is_id_locked(&self) -> bool {
        self.mode == ProviderFormMode::Edit
    }

    /// Move focus to the next field (wraps).
    pub fn tab_next(&mut self) {
        self.field = self.field.next();
        self.clamp_model_index();
    }

    /// Move focus to the previous field (wraps).
    pub fn tab_prev(&mut self) {
        self.field = self.field.prev();
        self.clamp_model_index();
    }

    /// The mutation this form describes.
    ///
    /// Model rows with a blank id are dropped — an empty row is scaffolding the
    /// user has not filled in yet, not a request to create a nameless model.
    pub fn to_input(&self) -> ProviderInput {
        let key = self.api_key.trim();
        ProviderInput {
            id: self.id.trim().to_string(),
            name: self.name.trim().to_string(),
            api_type: self.api_type.trim().to_string(),
            base_url: self.base_url.trim().to_string(),
            models: self
                .models
                .iter()
                .filter(|model| !model.id.trim().is_empty())
                .cloned()
                .collect(),
            api_key: if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            },
            clear_api_key: false,
            create_only: self.mode.is_add(),
        }
    }

    /// Validate and produce the submit action. On failure the message is stored
    /// in [`Self::error`] and the action is [`ProviderFormAction::None`].
    pub fn submit(&mut self) -> ProviderFormAction {
        let input = self.to_input();
        match validate_provider_input(&input) {
            Ok(()) => {
                self.error = None;
                ProviderFormAction::Submit(input)
            }
            Err(message) => {
                self.error = Some(message);
                ProviderFormAction::None
            }
        }
    }

    /// Dispatch one key. See the module docs for the key table.
    pub fn handle_key(&mut self, key: &str) -> ProviderFormAction {
        match key {
            "tab" => {
                self.tab_next();
                ProviderFormAction::Changed
            }
            "shift+tab" => {
                self.tab_prev();
                ProviderFormAction::Changed
            }
            "escape" => ProviderFormAction::Cancel,
            "enter" => self.submit(),
            "up" => self.move_focus(-1),
            "down" => self.move_focus(1),
            "backspace" | "ctrl+h" => {
                if self.delete_back() {
                    ProviderFormAction::Changed
                } else {
                    ProviderFormAction::None
                }
            }
            "left" => self.cycle_api_type(-1),
            "right" => self.cycle_api_type(1),
            "space" => self.space(),
            "ctrl+n" => {
                if self.add_model() {
                    ProviderFormAction::Changed
                } else {
                    ProviderFormAction::None
                }
            }
            "ctrl+d" | "delete" => {
                if self.remove_selected_model() {
                    ProviderFormAction::Changed
                } else {
                    ProviderFormAction::None
                }
            }
            "ctrl+t" => {
                if self.toggle_selected_model_thinking() {
                    ProviderFormAction::Changed
                } else {
                    ProviderFormAction::None
                }
            }
            other => match printable(other) {
                Some(ch) => {
                    if self.insert_char(ch) {
                        ProviderFormAction::Changed
                    } else {
                        ProviderFormAction::None
                    }
                }
                None => ProviderFormAction::None,
            },
        }
    }

    // ─── Model list editing ────────────────────────────────────────────────

    /// Append a blank model row, select it, and focus the model list.
    ///
    /// Refuses once the form already holds [`MAX_PROVIDER_MODELS`] rows — the
    /// same cap [`validate_provider_input`] applies on `enter` — so the limit is
    /// visible while editing instead of only when the mutation is submitted. As
    /// for a rejected submit, the reason is left in [`Self::error`].
    pub fn add_model(&mut self) -> bool {
        if self.models.len() >= MAX_PROVIDER_MODELS {
            self.error = Some("provider has too many models".to_string());
            return false;
        }
        self.models.push(ProviderModelInput::default());
        self.model_index = self.models.len() - 1;
        self.field = ProviderFormField::Models;
        self.dirty = true;
        self.error = None;
        true
    }

    /// Remove the selected model row (no-op on an empty list).
    pub fn remove_selected_model(&mut self) -> bool {
        if self.models.is_empty() {
            return false;
        }
        self.models.remove(self.model_index);
        self.clamp_model_index();
        self.dirty = true;
        self.error = None;
        true
    }

    /// Toggle `supports_images` on the selected row.
    pub fn toggle_selected_model_images(&mut self) -> bool {
        match self.models.get_mut(self.model_index) {
            Some(model) => {
                model.supports_images = !model.supports_images;
                self.dirty = true;
                self.error = None;
                true
            }
            None => false,
        }
    }

    /// Toggle `thinking` on the selected row.
    pub fn toggle_selected_model_thinking(&mut self) -> bool {
        match self.models.get_mut(self.model_index) {
            Some(model) => {
                model.thinking = !model.thinking;
                self.dirty = true;
                self.error = None;
                true
            }
            None => false,
        }
    }

    /// Select a model row by index (clamped); returns whether a row exists.
    pub fn select_model(&mut self, index: usize) -> bool {
        if self.models.is_empty() {
            return false;
        }
        self.model_index = index.min(self.models.len() - 1);
        true
    }

    // ─── Internals ─────────────────────────────────────────────────────────

    fn clamp_model_index(&mut self) {
        if self.models.is_empty() {
            self.model_index = 0;
        } else {
            self.model_index = self.model_index.min(self.models.len() - 1);
        }
    }

    /// `up`/`down`: inside the model list they move the row selection and only
    /// leave the field at its edges (so a form row is never unreachable); on
    /// every other field they move the focus directly.
    fn move_focus(&mut self, delta: isize) -> ProviderFormAction {
        if self.field == ProviderFormField::Models && !self.models.is_empty() {
            let last = self.models.len() - 1;
            if delta < 0 && self.model_index > 0 {
                self.model_index -= 1;
                return ProviderFormAction::Changed;
            }
            if delta > 0 && self.model_index < last {
                self.model_index += 1;
                return ProviderFormAction::Changed;
            }
        }
        if delta < 0 {
            self.tab_prev();
        } else {
            self.tab_next();
        }
        ProviderFormAction::Changed
    }

    fn cycle_api_type(&mut self, delta: isize) -> ProviderFormAction {
        if self.field != ProviderFormField::ApiType {
            return ProviderFormAction::None;
        }
        let count = API_TYPES.len() as isize;
        let current = API_TYPES
            .iter()
            .position(|api| *api == self.api_type)
            .map(|index| index as isize)
            .unwrap_or(0);
        let next = ((current + delta) % count + count) % count;
        self.api_type = API_TYPES[next as usize].to_string();
        self.dirty = true;
        self.error = None;
        ProviderFormAction::Changed
    }

    fn space(&mut self) -> ProviderFormAction {
        match self.field {
            ProviderFormField::Models => {
                if self.toggle_selected_model_images() {
                    ProviderFormAction::Changed
                } else {
                    ProviderFormAction::None
                }
            }
            ProviderFormField::ApiType => self.cycle_api_type(1),
            _ => {
                if self.insert_char(' ') {
                    ProviderFormAction::Changed
                } else {
                    ProviderFormAction::None
                }
            }
        }
    }

    fn insert_char(&mut self, ch: char) -> bool {
        match self.field {
            ProviderFormField::Id => {
                if self.is_id_locked() {
                    return false;
                }
                self.id.push(ch);
            }
            ProviderFormField::Name => self.name.push(ch),
            ProviderFormField::BaseUrl => self.base_url.push(ch),
            ProviderFormField::ApiKey => self.api_key.push(ch),
            // Cycle-only fields and the model list have their own keys.
            ProviderFormField::ApiType => return false,
            ProviderFormField::Models => {
                return self.push_model_char(ch);
            }
        }
        self.dirty = true;
        self.error = None;
        true
    }

    fn push_model_char(&mut self, ch: char) -> bool {
        let index = self.model_index;
        let Some(model) = self.models.get_mut(index) else {
            return false;
        };
        let previous = model.id.clone();
        model.id.push(ch);
        // The name mirrors the id until the user renames the model explicitly
        // (the agent falls back to the id for an empty name).
        if model.name.is_empty() || model.name == previous {
            model.name = model.id.clone();
        }
        self.dirty = true;
        self.error = None;
        true
    }

    fn delete_back(&mut self) -> bool {
        match self.field {
            ProviderFormField::Id => {
                if self.is_id_locked() || self.id.pop().is_none() {
                    return false;
                }
            }
            ProviderFormField::Name => {
                if self.name.pop().is_none() {
                    return false;
                }
            }
            ProviderFormField::BaseUrl => {
                if self.base_url.pop().is_none() {
                    return false;
                }
            }
            ProviderFormField::ApiKey => {
                if self.api_key.pop().is_none() {
                    return false;
                }
            }
            ProviderFormField::ApiType => return false,
            ProviderFormField::Models => {
                let index = self.model_index;
                let Some(model) = self.models.get_mut(index) else {
                    return false;
                };
                let previous = model.id.clone();
                if model.id.pop().is_none() {
                    return false;
                }
                // Keep the name in sync while it still mirrors the id.
                if model.name.is_empty() || model.name == previous {
                    model.name = model.id.clone();
                }
            }
        }
        self.dirty = true;
        self.error = None;
        true
    }

    // ─── Rendering ─────────────────────────────────────────────────────────

    fn title(&self) -> String {
        match self.mode {
            ProviderFormMode::Add => bold("Add provider"),
            ProviderFormMode::Edit => {
                let id = self.original_id.clone().unwrap_or_default();
                bold(&format!("Edit provider · {id}"))
            }
        }
    }

    fn placeholder(&self, text: &str) -> String {
        fg(C.dim_gray, text)
    }

    fn field_value_text(&self, field: ProviderFormField) -> String {
        match field {
            ProviderFormField::Id => {
                if self.id.is_empty() {
                    self.placeholder("required")
                } else if self.is_id_locked() {
                    format!("{} {}", self.id, self.placeholder("(fixed)"))
                } else {
                    self.id.clone()
                }
            }
            ProviderFormField::Name => {
                if self.name.is_empty() {
                    self.placeholder("optional")
                } else {
                    self.name.clone()
                }
            }
            ProviderFormField::ApiType => {
                format!("{} {}", self.api_type, self.placeholder("‹ ›"))
            }
            ProviderFormField::BaseUrl => {
                if self.base_url.is_empty() {
                    self.placeholder("required")
                } else {
                    self.base_url.clone()
                }
            }
            ProviderFormField::ApiKey => {
                if self.api_key.is_empty() {
                    if self.mode == ProviderFormMode::Edit {
                        self.placeholder("unchanged")
                    } else {
                        self.placeholder("optional")
                    }
                } else {
                    let mask_len = self.api_key.chars().count().min(API_KEY_MASK_LEN);
                    fg(C.green, &"•".repeat(mask_len))
                }
            }
            ProviderFormField::Models => {
                let count = self.models.len();
                if count == 0 {
                    self.placeholder("no models — ctrl+n adds one")
                } else {
                    format!("{count} configured")
                }
            }
        }
    }

    fn field_row(&self, field: ProviderFormField) -> String {
        let focused = self.field == field;
        let marker = if focused {
            fg(C.accent, "▸")
        } else {
            " ".to_string()
        };
        let label = if focused {
            fg(C.accent, &bold(field.label()))
        } else {
            fg(C.dim_gray, field.label())
        };
        format!("{marker} {label}: {}", self.field_value_text(field))
    }

    /// Model rows for the `Models` field, windowed around the selection.
    fn model_rows(&self) -> Vec<String> {
        if self.models.is_empty() {
            return Vec::new();
        }
        let total = self.models.len();
        let count = total.min(MAX_VISIBLE_MODELS);
        let start = self
            .model_index
            .saturating_sub(count / 2)
            .min(total.saturating_sub(count));
        let focused = self.field == ProviderFormField::Models;
        let mut rows = Vec::new();
        if start > 0 {
            rows.push(fg(C.dim_gray, &format!("      ↑ {start} more")));
        }
        for (offset, model) in self.models.iter().enumerate().skip(start).take(count) {
            let selected = offset == self.model_index;
            let marker = if selected && focused {
                fg(C.accent, "▸")
            } else if selected {
                "·".to_string()
            } else {
                " ".to_string()
            };
            let id = if model.id.is_empty() {
                self.placeholder("(id required)")
            } else {
                model.id.clone()
            };
            let mut flags = Vec::new();
            if model.supports_images {
                flags.push("images".to_string());
            }
            if model.thinking {
                flags.push("thinking".to_string());
            }
            let flag_text = if flags.is_empty() {
                String::new()
            } else {
                format!("  {}", fg(C.dim_gray, &flags.join(" · ")))
            };
            rows.push(format!(
                "    {marker} {id}{flag_text}  {}",
                fg(C.dim_gray, &format!("ctx {}", model.context_window))
            ));
        }
        let remaining = total - (start + count);
        if remaining > 0 {
            rows.push(fg(C.dim_gray, &format!("      ↓ {remaining} more")));
        }
        rows
    }

    /// The hint row for the focused field. `width` is the pane width: in edit
    /// mode the hint list is longer than a standard terminal, so it is cut with
    /// an ellipsis rather than mid-word.
    fn hint_line(&self, width: usize) -> String {
        let mut parts: Vec<&str> = match self.field {
            ProviderFormField::Models => {
                vec![
                    "ctrl+n add",
                    "ctrl+d remove",
                    "space images",
                    "ctrl+t thinking",
                ]
            }
            ProviderFormField::ApiType => vec!["←/→ cycle API type", "tab moves"],
            _ => vec!["tab/↑↓ move", "enter save", "esc cancel"],
        };
        if self.mode == ProviderFormMode::Edit {
            parts.push("blank key keeps the stored key");
        }
        fg(C.dim_gray, &fit_hint_row(&parts.join(" · "), width))
    }

    /// Render the form at `width`. Every row is exactly `width` columns wide.
    pub fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let mut rows = vec![self.title()];
        for field in ProviderFormField::ALL {
            rows.push(self.field_row(field));
            if field == ProviderFormField::Models {
                rows.extend(self.model_rows());
            }
        }
        if let Some(error) = &self.error {
            rows.push(fg(C.red, &format!("✗ {error}")));
        }
        rows.push(self.hint_line(width));
        rows.into_iter().map(|row| fit_row(&row, width)).collect()
    }
}

// ─── Overlay adapters ──────────────────────────────────────────────────────

/// The [`ProviderListState`] as an overlay [`Component`].
///
/// The list owns its menu; the adapter forwards the value-carrying
/// [`ProviderListAction`]s so the app layer (which talks to the agent and owns
/// the RPC write set) can act on them.
pub struct ProviderListOverlay {
    state: ProviderListState,
    on_action: Box<dyn FnMut(ProviderListAction)>,
}

impl ProviderListOverlay {
    pub fn new(state: ProviderListState, on_action: Box<dyn FnMut(ProviderListAction)>) -> Self {
        Self { state, on_action }
    }

    pub fn state(&self) -> &ProviderListState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut ProviderListState {
        &mut self.state
    }

    /// Forward the palette to the wrapped menu.
    pub fn set_theme(&mut self, theme: Theme) {
        self.state.set_theme(theme);
    }
}

impl Component for ProviderListOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.state.render(width, MENU_MAX_ROWS)
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.state.handle_key(data);
        if action != ProviderListAction::None {
            (self.on_action)(action);
        }
    }

    /// A live search takes the first escape (it clears the query and keeps the
    /// list open); the second one reaches the app layer and closes it. Same
    /// rule as [`crate::components::menu::MenuOverlay`], which this list is
    /// built on.
    fn wants_escape(&self) -> bool {
        self.state.is_searching()
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// The add/edit [`ProviderForm`] as an overlay [`Component`].
pub struct ProviderFormOverlay {
    form: ProviderForm,
    on_action: Box<dyn FnMut(ProviderFormAction)>,
}

impl ProviderFormOverlay {
    pub fn new(form: ProviderForm, on_action: Box<dyn FnMut(ProviderFormAction)>) -> Self {
        Self { form, on_action }
    }

    pub fn form(&self) -> &ProviderForm {
        &self.form
    }

    pub fn form_mut(&mut self) -> &mut ProviderForm {
        &mut self.form
    }
}

impl Component for ProviderFormOverlay {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.form.render(width)
    }

    fn handle_input(&mut self, data: &str) {
        let action = self.form.handle_key(data);
        if action != ProviderFormAction::None {
            (self.on_action)(action);
        }
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ─── List: actions ─────────────────────────────────────────────────────────

/// What one key press did to the provider list.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderListAction {
    /// Unhandled key (or a rejected action — see [`ProviderListState::notice`]).
    None,
    /// Highlight, filter, scroll or tab changed — redraw.
    Moved,
    /// `e`/`enter` on a custom provider.
    Edit(ProviderInfo),
    /// `d` on a custom provider.
    Delete(ProviderInfo),
    /// `k`/`enter` on a built-in provider (`set_auth`), or `k` anywhere.
    SetKey(ProviderInfo),
    /// `s` — `sync_future_models`.
    SyncModels,
    /// `r` — `reload_auth`.
    ReloadAuth,
    /// `a` — open the add form.
    Add,
    /// `escape`.
    Cancelled,
}

// ─── List: state ───────────────────────────────────────────────────────────

/// The `/providers` picker: two tabs (built-in catalog / custom), incremental
/// search and the action keys listed in the module docs.
pub struct ProviderListState {
    menu: MenuState,
    providers: Vec<ProviderInfo>,
    /// Qualified (`provider/model`) or bare default model id, for the
    /// `default` badge.
    default_model: String,
    /// One-line explanation for a rejected action (e.g. deleting a built-in).
    notice: Option<String>,
}

impl ProviderListState {
    /// A picker over `providers` (built-ins first — the caller passes what
    /// `list_providers` returned, which is already split).
    pub fn new(providers: Vec<ProviderInfo>) -> Self {
        Self::new_with_title("Providers", providers)
    }

    /// Same as [`Self::new`] with a custom overlay title.
    pub fn new_with_title(title: impl Into<String>, providers: Vec<ProviderInfo>) -> Self {
        let tabs = vec![
            MenuTab::new(PROVIDER_TAB_BUILTIN, "Built-in"),
            MenuTab::new(PROVIDER_TAB_CUSTOM, "Custom"),
        ];
        let options = MenuOptions::new(title, Vec::new())
            .with_tabs(tabs)
            .searchable(true)
            .max_visible(12)
            .with_footer_hints([
                ("enter", "open"),
                ("e", "edit"),
                ("k", "key"),
                ("d", "delete"),
                ("a", "add"),
                ("s", "sync"),
                ("r", "reload"),
                ("/", "search"),
                ("esc", "close"),
            ])
            .empty_text("No providers");
        let mut state = Self {
            menu: MenuState::new(options),
            providers: Vec::new(),
            default_model: String::new(),
            notice: None,
        };
        state.set_providers(providers);
        state
    }

    /// Replace the provider list (after a mutation or a refresh), keeping the
    /// active tab. The list's own providers are the source of the row data.
    pub fn set_providers(&mut self, providers: Vec<ProviderInfo>) {
        self.providers = providers;
        let (builtin, custom) = self.split();
        self.menu
            .set_sections_for_tab(0, vec![MenuSection::flat(builtin)]);
        self.menu
            .set_sections_for_tab(1, vec![MenuSection::flat(custom)]);
    }

    /// Set the qualified (`provider/model`) default model id. Only the prefix
    /// is used — the default model's *provider* gets the badge.
    pub fn set_default_model(&mut self, model_id: &str) {
        self.default_model = model_id.trim().to_string();
        let providers = std::mem::take(&mut self.providers);
        self.set_providers(providers);
    }

    /// Adopt a palette for the wrapped menu.
    pub fn set_theme(&mut self, theme: Theme) {
        self.menu.set_theme(theme);
    }

    /// The providers this list renders, in `list_providers` order.
    pub fn providers(&self) -> &[ProviderInfo] {
        &self.providers
    }

    /// Id of the default model as last set.
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// The last rejected-action explanation, if any.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Set (or clear) the notice row the caller wants surfaced.
    pub fn set_notice(&mut self, notice: Option<&str>) {
        self.notice = notice.map(str::to_string);
    }

    /// Active tab id ([`PROVIDER_TAB_BUILTIN`] / [`PROVIDER_TAB_CUSTOM`]).
    pub fn tab_id(&self) -> &str {
        self.menu
            .tabs()
            .get(self.menu.tab_index())
            .map(|tab| tab.id.as_str())
            .unwrap_or(PROVIDER_TAB_BUILTIN)
    }

    /// Switch to a tab by id; unknown ids are ignored.
    pub fn select_tab(&mut self, tab_id: &str) {
        if let Some(index) = self.menu.tabs().iter().position(|tab| tab.id == tab_id) {
            self.menu.set_tab_index(index);
        }
    }

    /// Current search query.
    pub fn filter(&self) -> &str {
        self.menu.filter()
    }

    /// `true` while the search row owns the printable keys.
    pub fn is_searching(&self) -> bool {
        self.menu.is_filtering() || !self.menu.filter().is_empty()
    }

    /// Rows matching the current search on the active tab.
    pub fn visible_len(&self) -> usize {
        self.menu.visible_len()
    }

    /// The highlighted provider, or `None` when the tab is empty.
    pub fn highlighted(&self) -> Option<ProviderInfo> {
        let item = self.menu.highlighted()?;
        self.find(&item.value)
    }

    /// Dispatch one key. See the module docs for the key table.
    pub fn handle_key(&mut self, key: &str) -> ProviderListAction {
        // Single-letter actions are only recognised while the search row is
        // idle; `/` (or any printable key) starts a search, after which every
        // printable key belongs to the query until `esc` clears it.
        if !self.is_searching() {
            match key {
                "a" | "ctrl+n" => {
                    self.notice = None;
                    return ProviderListAction::Add;
                }
                "s" | "ctrl+s" => {
                    self.notice = None;
                    return ProviderListAction::SyncModels;
                }
                "r" | "ctrl+r" => {
                    self.notice = None;
                    return ProviderListAction::ReloadAuth;
                }
                "k" | "ctrl+k" => {
                    if let Some(info) = self.highlighted() {
                        self.notice = None;
                        return ProviderListAction::SetKey(info);
                    }
                    return ProviderListAction::None;
                }
                "e" | "ctrl+e" => return self.edit_highlighted(),
                "d" | "ctrl+d" | "delete" => return self.delete_highlighted(),
                _ => {}
            }
        }

        match self.menu.handle_key(key) {
            MenuAction::Cancelled => {
                // The menu only cancels with an empty filter, so the caller can
                // treat this as "close the overlay".
                self.notice = None;
                ProviderListAction::Cancelled
            }
            MenuAction::Confirmed(values) => {
                // Every menu value is a provider id seeded by `set_providers`,
                // so an id that names no provider leaves the dialog unchanged —
                // the same "nothing to do" action as an unhandled key.
                let info = values.first().and_then(|id| self.find(id));
                info.map_or(ProviderListAction::None, |info| {
                    if info.builtin {
                        self.notice = Some(
                            "built-in provider: only its API key can be set from here".to_string(),
                        );
                        ProviderListAction::SetKey(info)
                    } else {
                        self.notice = None;
                        ProviderListAction::Edit(info)
                    }
                })
            }
            MenuAction::None => ProviderListAction::None,
            _ => {
                self.notice = None;
                ProviderListAction::Moved
            }
        }
    }

    /// Render at most `height` rows, each exactly `width` columns wide. The
    /// notice (if any) takes the last row.
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let notice = self.notice.clone();
        let body_height = if notice.is_some() {
            height.saturating_sub(1)
        } else {
            height
        };
        let mut rows = self.menu.render(width, body_height.max(1).min(height));
        if let Some(notice) = notice {
            rows.push(fit_row(&fg(C.yellow, &format!("! {notice}")), width));
        }
        rows.truncate(height);
        rows
    }

    // ─── Internals ─────────────────────────────────────────────────────────

    fn edit_highlighted(&mut self) -> ProviderListAction {
        let Some(info) = self.highlighted() else {
            return ProviderListAction::None;
        };
        if info.builtin {
            self.notice =
                Some("built-in providers cannot be edited — press k to set its key".to_string());
            return ProviderListAction::None;
        }
        self.notice = None;
        ProviderListAction::Edit(info)
    }

    fn delete_highlighted(&mut self) -> ProviderListAction {
        let Some(info) = self.highlighted() else {
            return ProviderListAction::None;
        };
        if info.builtin {
            self.notice = Some("built-in providers cannot be deleted".to_string());
            return ProviderListAction::None;
        }
        self.notice = None;
        ProviderListAction::Delete(info)
    }

    fn find(&self, id: &str) -> Option<ProviderInfo> {
        self.providers
            .iter()
            .find(|provider| provider.id == id)
            .cloned()
    }

    fn split(&self) -> (Vec<MenuItem>, Vec<MenuItem>) {
        let default_id = self.default_provider_id();
        let item = |provider: &ProviderInfo| {
            let mut badges = Vec::new();
            if default_id.as_deref() == Some(provider.id.as_str()) {
                badges.push("default".to_string());
            }
            badges.push(if provider.has_api_key {
                "key".to_string()
            } else {
                "no key".to_string()
            });
            badges.push(format!("{} models", provider.model_count));
            let description = if provider.base_url.trim().is_empty() {
                if provider.builtin {
                    "built-in catalog".to_string()
                } else {
                    "no base URL".to_string()
                }
            } else if provider.api_type.trim().is_empty() {
                provider.base_url.trim().to_string()
            } else {
                format!("{}  {}", provider.api_type.trim(), provider.base_url.trim())
            };
            MenuItem::new(provider.id.clone(), display_name(provider))
                .with_description(description)
                .with_badges(badges)
        };
        let builtin = self
            .providers
            .iter()
            .filter(|provider| provider.builtin)
            .map(item)
            .collect();
        let custom = self
            .providers
            .iter()
            .filter(|provider| !provider.builtin)
            .map(item)
            .collect();
        (builtin, custom)
    }

    /// The provider part of the default model id (`provider/model`), or `None`
    /// when the id is bare (a bare id names a model, not a provider).
    fn default_provider_id(&self) -> Option<String> {
        let (provider, _) = self.default_model.split_once('/')?;
        let provider = provider.trim();
        if provider.is_empty() {
            None
        } else {
            Some(provider.to_string())
        }
    }
}

/// Display name for a provider row (falls back to the id, like the agent).
fn display_name(provider: &ProviderInfo) -> String {
    let name = provider.name.trim();
    if name.is_empty() {
        provider.id.clone()
    } else {
        name.to_string()
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    fn plain(row: &str) -> String {
        strip_ansi_codes(row)
    }

    fn plain_rows(rows: &[String]) -> Vec<String> {
        rows.iter().map(|row| plain(row)).collect()
    }

    fn provider(id: &str, name: &str, builtin: bool) -> ProviderInfo {
        ProviderInfo {
            id: id.to_string(),
            name: name.to_string(),
            api_type: if builtin {
                String::new()
            } else {
                "openai-completions".to_string()
            },
            base_url: format!("https://{id}.test/v1"),
            models: if builtin {
                Vec::new()
            } else {
                vec![ProviderModelInput::new(&format!("{id}-large"), "Large")]
            },
            has_api_key: !builtin,
            builtin,
            model_count: if builtin { 12 } else { 1 },
        }
    }

    fn providers() -> Vec<ProviderInfo> {
        vec![
            provider("future", "Future", true),
            provider("acme", "Acme", false),
            provider("zeta", "Zeta", false),
        ]
    }

    fn typed(form: &mut ProviderForm, text: &str) {
        for ch in text.chars() {
            let action = form.handle_key(&ch.to_string());
            assert_ne!(action, ProviderFormAction::Cancel, "unexpected cancel");
        }
    }

    fn valid_form() -> ProviderForm {
        let mut form = ProviderForm::add();
        form.id = "acme".into();
        form.name = "Acme".into();
        form.base_url = "https://api.acme.test/v1".into();
        form.models = vec![ProviderModelInput::new("acme-large", "Acme Large")];
        form
    }

    // ─── Field order ───────────────────────────────────────────────────────

    #[test]
    fn field_order_wraps_in_both_directions() {
        assert_eq!(ProviderFormField::Id.next(), ProviderFormField::Name);
        assert_eq!(ProviderFormField::Models.next(), ProviderFormField::Id);
        assert_eq!(ProviderFormField::Id.prev(), ProviderFormField::Models);
        assert_eq!(ProviderFormField::ApiKey.prev(), ProviderFormField::BaseUrl);
        for field in ProviderFormField::ALL {
            assert_eq!(field.next().prev(), field);
        }
        assert_eq!(ProviderFormField::ALL.len(), 6);
    }

    #[test]
    fn field_labels_are_stable() {
        assert_eq!(ProviderFormField::BaseUrl.label(), "Base URL");
        assert_eq!(ProviderFormField::ApiType.label(), "API");
    }

    // ─── Add mode ──────────────────────────────────────────────────────────

    #[test]
    fn add_form_starts_pristine_and_targets_its_first_field() {
        let form = ProviderForm::add();
        assert_eq!(form.mode, ProviderFormMode::Add);
        assert_eq!(form.field, ProviderFormField::Id);
        assert!(!form.dirty);
        assert!(form.error.is_none());
        assert!(form.models.is_empty());
        assert_eq!(form.api_type, API_TYPES_CONTRACT_FIRST);
        assert_eq!(form.original_id, None);
        assert!(!form.is_id_locked());
    }

    /// The first `API_TYPES` entry — pinned so a reordering is a test failure.
    const API_TYPES_CONTRACT_FIRST: &str = "openai-completions";

    #[test]
    fn the_contract_api_type_is_the_first_entry() {
        assert_eq!(API_TYPES[0], API_TYPES_CONTRACT_FIRST);
        assert_eq!(API_TYPES.len(), 3);
    }

    #[test]
    fn tab_next_and_prev_walk_every_field_and_wrap() {
        let mut form = ProviderForm::add();
        for field in ProviderFormField::ALL.iter().skip(1) {
            form.tab_next();
            assert_eq!(form.field, *field);
        }
        form.tab_next();
        assert_eq!(form.field, ProviderFormField::Id);
        form.tab_prev();
        assert_eq!(form.field, ProviderFormField::Models);
    }

    #[test]
    fn typing_fills_the_focused_field_and_marks_the_form_dirty() {
        let mut form = ProviderForm::add();
        typed(&mut form, "acme-1");
        assert_eq!(form.id, "acme-1");
        assert!(form.dirty);
        form.tab_next();
        typed(&mut form, "Acme");
        assert_eq!(form.name, "Acme");
        form.tab_next();
        form.tab_next();
        typed(&mut form, "https://api.acme.test/v1");
        assert_eq!(form.base_url, "https://api.acme.test/v1");
        form.tab_next();
        typed(&mut form, "sk-123");
        assert_eq!(form.api_key, "sk-123");
    }

    #[test]
    fn unhandled_keys_report_none() {
        let mut form = ProviderForm::add();
        assert_eq!(form.handle_key("f5"), ProviderFormAction::None);
        assert_eq!(form.handle_key("ctrl+x"), ProviderFormAction::None);
        assert_eq!(form.handle_key("shift+tab"), ProviderFormAction::Changed);
    }

    #[test]
    fn tab_and_shift_tab_move_the_focus_through_handle_key() {
        let mut form = ProviderForm::add();
        assert_eq!(form.handle_key("tab"), ProviderFormAction::Changed);
        assert_eq!(form.field, ProviderFormField::Name);
        assert_eq!(form.handle_key("shift+tab"), ProviderFormAction::Changed);
        assert_eq!(form.field, ProviderFormField::Id);
    }

    #[test]
    fn backspace_deletes_in_every_text_field_and_ignores_empties() {
        let mut form = ProviderForm::add();
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::None);
        typed(&mut form, "abc");
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::Changed);
        assert_eq!(form.id, "ab");
        form.tab_next();
        assert_eq!(form.handle_key("ctrl+h"), ProviderFormAction::None);
        typed(&mut form, "xy");
        assert_eq!(form.handle_key("ctrl+h"), ProviderFormAction::Changed);
        assert_eq!(form.name, "x");
    }

    #[test]
    fn backspace_on_an_empty_base_url_key_or_api_picker_is_refused() {
        let mut form = ProviderForm::add();
        form.field = ProviderFormField::BaseUrl;
        typed(&mut form, "ab");
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::Changed);
        assert_eq!(form.base_url, "a");
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::Changed);
        assert!(form.base_url.is_empty());
        // Nothing left to delete: the edit is refused, and the form is not
        // marked dirty by the refused key.
        form.dirty = false;
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::None);
        assert!(!form.dirty);
        form.field = ProviderFormField::ApiKey;
        typed(&mut form, "k");
        assert_eq!(form.handle_key("ctrl+h"), ProviderFormAction::Changed);
        assert!(form.api_key.is_empty());
        form.dirty = false;
        assert_eq!(form.handle_key("ctrl+h"), ProviderFormAction::None);
        assert!(!form.dirty);
        // The API picker is a cycle field, so it has no text to delete at all.
        form.field = ProviderFormField::ApiType;
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::None);
        assert_eq!(form.api_type, API_TYPES_CONTRACT_FIRST);
        assert!(!form.dirty);
    }

    #[test]
    fn api_type_cycles_with_arrows_space_and_ignores_typing() {
        let mut form = ProviderForm::add();
        form.field = ProviderFormField::ApiType;
        let first = form.api_type.clone();
        assert_eq!(form.handle_key("right"), ProviderFormAction::Changed);
        assert_ne!(form.api_type, first);
        form.handle_key("right");
        form.handle_key("right");
        assert_eq!(form.api_type, first, "three API types cycle back");
        form.handle_key("left");
        assert_ne!(form.api_type, first, "left cycles backwards");
        // A printable key is ignored: the field is a cycle picker.
        let before = form.api_type.clone();
        assert_eq!(form.handle_key("x"), ProviderFormAction::None);
        assert_eq!(form.api_type, before);
        // The arrows only act on the focused field.
        form.field = ProviderFormField::Name;
        assert_eq!(form.handle_key("right"), ProviderFormAction::None);
    }

    #[test]
    fn space_cycles_the_api_type_and_is_refused_on_a_locked_id() {
        // The API field is a cycle picker, so `space` behaves like `right`.
        let mut form = ProviderForm::add();
        form.field = ProviderFormField::ApiType;
        assert_eq!(form.handle_key("space"), ProviderFormAction::Changed);
        assert_eq!(form.api_type, API_TYPES[1]);
        assert!(form.dirty);
        // In edit mode the id is fixed, so a space is refused instead of
        // appending to a value the agent would ignore.
        let mut locked = ProviderForm::edit(&provider("acme", "Acme", false));
        locked.field = ProviderFormField::Id;
        assert!(locked.is_id_locked());
        assert_eq!(locked.handle_key("space"), ProviderFormAction::None);
        assert_eq!(locked.id, "acme");
        assert!(!locked.dirty);
    }

    #[test]
    fn space_inserts_a_space_into_text_fields() {
        let mut form = ProviderForm::add();
        typed(&mut form, "Acme");
        assert_eq!(form.handle_key("space"), ProviderFormAction::Changed);
        assert_eq!(form.id, "Acme ");
    }

    // ─── Model rows ────────────────────────────────────────────────────────

    #[test]
    fn ctrl_n_appends_a_model_row_and_focuses_the_list() {
        let mut form = ProviderForm::add();
        form.field = ProviderFormField::Name;
        assert_eq!(form.handle_key("ctrl+n"), ProviderFormAction::Changed);
        assert_eq!(form.models.len(), 1);
        assert_eq!(form.field, ProviderFormField::Models);
        assert_eq!(form.model_index, 0);
        typed(&mut form, "acme-large");
        assert_eq!(form.models[0].id, "acme-large");
        assert_eq!(form.models[0].name, "acme-large", "name mirrors the id");
    }

    #[test]
    fn typing_after_a_rename_does_not_clobber_it() {
        let mut form = ProviderForm::add();
        form.add_model();
        typed(&mut form, "ab");
        form.models[0].name = "Custom Name".to_string();
        typed(&mut form, "c");
        assert_eq!(form.models[0].id, "abc");
        assert_eq!(form.models[0].name, "Custom Name");
    }

    #[test]
    fn backspace_on_a_model_row_shrinks_its_id_and_name_together() {
        let mut form = ProviderForm::add();
        form.add_model();
        typed(&mut form, "ab");
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::Changed);
        assert_eq!(form.models[0].id, "a");
        assert_eq!(form.models[0].name, "a");
        form.handle_key("backspace");
        assert_eq!(form.models[0].id, "");
        assert_eq!(form.models[0].name, "");
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::None);
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::None);
    }

    #[test]
    fn ctrl_d_removes_the_selected_row_and_no_ops_on_an_empty_list() {
        let mut form = ProviderForm::add();
        assert_eq!(form.handle_key("ctrl+d"), ProviderFormAction::None);
        form.add_model();
        form.add_model();
        typed(&mut form, "second");
        assert_eq!(form.model_index, 1);
        assert_eq!(form.handle_key("delete"), ProviderFormAction::Changed);
        assert_eq!(form.models.len(), 1);
        assert_eq!(form.model_index, 0, "selection clamps back");
        assert_eq!(form.handle_key("ctrl+d"), ProviderFormAction::Changed);
        assert!(form.models.is_empty());
        assert_eq!(form.model_index, 0);
    }

    #[test]
    fn toggles_flip_row_flags_and_ignore_an_empty_list() {
        let mut form = ProviderForm::add();
        form.field = ProviderFormField::Models;
        assert_eq!(form.handle_key("space"), ProviderFormAction::None);
        assert_eq!(form.handle_key("ctrl+t"), ProviderFormAction::None);
        form.add_model();
        assert!(!form.models[0].supports_images);
        assert!(form.models[0].thinking);
        assert_eq!(form.handle_key("space"), ProviderFormAction::Changed);
        assert!(form.models[0].supports_images);
        assert_eq!(form.handle_key("ctrl+t"), ProviderFormAction::Changed);
        assert!(!form.models[0].thinking);
    }

    #[test]
    fn up_and_down_walk_the_model_rows_then_leave_the_field() {
        let mut form = ProviderForm::add();
        form.add_model();
        form.add_model();
        form.add_model();
        form.select_model(0);
        assert_eq!(form.handle_key("down"), ProviderFormAction::Changed);
        assert_eq!(form.model_index, 1);
        // `up` walks the list backwards again.
        assert_eq!(form.handle_key("up"), ProviderFormAction::Changed);
        assert_eq!(form.model_index, 0);
        form.select_model(2);
        form.handle_key("down");
        assert_eq!(form.field, ProviderFormField::Id, "leaves at the last row");
        // Coming back: up from the first row leaves towards the other side.
        form.field = ProviderFormField::Models;
        form.select_model(0);
        form.handle_key("up");
        assert_eq!(form.field, ProviderFormField::ApiKey);
        // An empty list moves the focus instead of the selection.
        let mut empty = ProviderForm::add();
        empty.field = ProviderFormField::Models;
        assert_eq!(empty.handle_key("up"), ProviderFormAction::Changed);
        assert_eq!(empty.field, ProviderFormField::ApiKey);
    }

    #[test]
    fn select_model_clamps_and_reports_an_empty_list() {
        let mut form = ProviderForm::add();
        assert!(!form.select_model(3));
        form.add_model();
        form.add_model();
        assert!(form.select_model(99));
        assert_eq!(form.model_index, 1);
    }

    #[test]
    fn typing_or_deleting_without_a_model_row_is_refused() {
        // A form can focus the model list before any row exists (`down` walks
        // the fields); both edits must then be refused rather than panic.
        let mut form = ProviderForm::add();
        for _ in 0..5 {
            assert_eq!(form.handle_key("down"), ProviderFormAction::Changed);
        }
        assert_eq!(form.field, ProviderFormField::Models);
        assert!(form.models.is_empty());
        assert_eq!(form.handle_key("a"), ProviderFormAction::None);
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::None);
        assert!(form.models.is_empty());
        assert!(!form.dirty);
    }

    // ─── Validation arms ───────────────────────────────────────────────────

    #[test]
    fn submit_rejects_an_empty_provider_id() {
        let mut form = ProviderForm::add();
        form.base_url = "https://api.acme.test/v1".into();
        assert_eq!(form.submit(), ProviderFormAction::None);
        assert_eq!(form.error.as_deref(), Some("provider id is required"));
    }

    #[test]
    fn submit_rejects_an_id_with_illegal_characters() {
        let mut form = ProviderForm::add();
        form.id = "Acme Co!".into();
        form.base_url = "https://api.acme.test/v1".into();
        form.submit();
        assert!(form.error.as_deref().unwrap().contains("lowercase"));
    }

    #[test]
    fn submit_rejects_a_non_http_base_url() {
        let mut form = ProviderForm::add();
        form.id = "acme".into();
        form.base_url = "ftp://api.acme.test".into();
        form.submit();
        assert_eq!(
            form.error.as_deref(),
            Some("base URL must be a valid http/https address")
        );
        form.base_url = "not a url".into();
        form.submit();
        assert_eq!(
            form.error.as_deref(),
            Some("base URL must be a valid http/https address")
        );
        form.base_url = String::new();
        form.submit();
        assert_eq!(form.error.as_deref(), Some("base URL is required"));
    }

    #[test]
    fn submit_rejects_an_unsupported_api_type() {
        let mut form = valid_form();
        form.api_type = "gemini".into();
        form.submit();
        assert_eq!(form.error.as_deref(), Some("unsupported provider API type"));
        form.api_type = String::new();
        form.submit();
        assert_eq!(form.error.as_deref(), Some("provider API type is required"));
    }

    #[test]
    fn submit_rejects_duplicate_model_ids() {
        let mut form = valid_form();
        form.models = vec![
            ProviderModelInput::new("dup", "One"),
            ProviderModelInput::new("dup", "Two"),
        ];
        form.submit();
        assert_eq!(
            form.error.as_deref(),
            Some("provider model id is invalid or duplicated")
        );
    }

    #[test]
    fn submit_rejects_too_many_models() {
        let mut form = valid_form();
        form.models = (0..crate::rpc::provider_types::MAX_PROVIDER_MODELS + 1)
            .map(|index| ProviderModelInput::new(&format!("m{index}"), "M"))
            .collect();
        form.submit();
        assert_eq!(form.error.as_deref(), Some("provider has too many models"));
    }

    #[test]
    fn add_model_refuses_past_the_same_cap_submit_enforces() {
        let mut form = ProviderForm::add();
        form.models = (0..MAX_PROVIDER_MODELS)
            .map(|index| ProviderModelInput::new(&format!("m{index}"), "M"))
            .collect();
        assert_eq!(form.handle_key("ctrl+n"), ProviderFormAction::None);
        assert_eq!(form.models.len(), MAX_PROVIDER_MODELS, "no row is added");
        assert_eq!(
            form.error.as_deref(),
            Some("provider has too many models"),
            "the refusal says what the rejected submit would have said"
        );

        // One row below the cap the key still appends (and clears the message).
        form.models.pop();
        assert_eq!(form.handle_key("ctrl+n"), ProviderFormAction::Changed);
        assert_eq!(form.models.len(), MAX_PROVIDER_MODELS);
        assert!(form.error.is_none());
        assert_eq!(
            form.field,
            ProviderFormField::Models,
            "focus follows the row"
        );
    }

    #[test]
    fn submit_rejects_an_invalid_model_name_and_token_limits() {
        let mut form = valid_form();
        form.models[0].name = "bad\nname".into();
        form.submit();
        assert_eq!(
            form.error.as_deref(),
            Some("provider model name is invalid or too long")
        );

        let mut form = valid_form();
        form.models[0].context_window = 0;
        form.submit();
        assert_eq!(
            form.error.as_deref(),
            Some("provider model token limits must be positive")
        );

        let mut form = valid_form();
        form.models[0].max_tokens = 999_999;
        form.submit();
        assert_eq!(
            form.error.as_deref(),
            Some("provider model max tokens cannot exceed its context window")
        );
    }

    #[test]
    fn editing_clears_a_previous_error() {
        let mut form = ProviderForm::add();
        form.submit();
        assert!(form.error.is_some());
        form.field = ProviderFormField::Id;
        assert_eq!(form.handle_key("a"), ProviderFormAction::Changed);
        assert!(form.error.is_none(), "the message described stale input");
    }

    #[test]
    fn a_successful_submit_carries_the_create_flag_and_no_key() {
        let mut form = valid_form();
        assert!(!form.dirty);
        let input = form.to_input();
        assert_eq!(input.id, "acme");
        assert_eq!(input.api_type, "openai-completions");
        assert!(input.create_only);
        assert_eq!(input.api_key, None);
        assert_eq!(input.models.len(), 1);
        assert!(validate_provider_input(&input).is_ok());
        // `submit` hands back exactly the input it validated.
        assert_eq!(form.submit(), ProviderFormAction::Submit(input));
        assert!(form.error.is_none());
    }

    #[test]
    fn escape_cancels_without_validating() {
        let mut form = ProviderForm::add();
        assert_eq!(form.handle_key("escape"), ProviderFormAction::Cancel);
        assert!(form.error.is_none());
    }

    // ─── Edit mode ─────────────────────────────────────────────────────────

    #[test]
    fn edit_prefills_every_field_and_leaves_the_key_blank() {
        let info = provider("acme", "Acme", false);
        let form = ProviderForm::edit(&info);
        assert_eq!(form.mode, ProviderFormMode::Edit);
        assert_eq!(form.id, "acme");
        assert_eq!(form.name, "Acme");
        assert_eq!(form.api_type, "openai-completions");
        assert_eq!(form.base_url, "https://acme.test/v1");
        assert_eq!(form.api_key, "", "a blank key means 'unchanged'");
        assert_eq!(form.models, info.models);
        assert_eq!(form.original_id.as_deref(), Some("acme"));
        assert_eq!(form.field, ProviderFormField::Name);
        assert!(!form.dirty);
        assert!(form.is_id_locked());
    }

    #[test]
    fn edit_defaults_a_missing_api_type_and_focuses_name() {
        let mut info = provider("future", "Future", true);
        info.api_type = String::new();
        let form = ProviderForm::edit(&info);
        assert_eq!(form.api_type, API_TYPES_CONTRACT_FIRST);
        assert_eq!(form.field, ProviderFormField::Name);
    }

    #[test]
    fn edit_mode_ignores_id_edits_and_keeps_the_original_id() {
        let mut form = ProviderForm::edit(&provider("acme", "Acme", false));
        form.field = ProviderFormField::Id;
        assert_eq!(form.handle_key("x"), ProviderFormAction::None);
        assert_eq!(form.id, "acme");
        assert_eq!(form.handle_key("backspace"), ProviderFormAction::None);
        assert_eq!(form.id, "acme");
        assert!(!form.dirty);
        assert_eq!(form.handle_key("escape"), ProviderFormAction::Cancel);
    }

    #[test]
    fn edit_submits_create_only_false_and_only_the_typed_key() {
        let mut form = ProviderForm::edit(&provider("acme", "Acme", false));
        form.field = ProviderFormField::Name;
        typed(&mut form, "!");
        form.field = ProviderFormField::ApiKey;
        typed(&mut form, "sk-new");
        let input = form.to_input();
        assert!(!input.create_only);
        assert_eq!(input.api_key.as_deref(), Some("sk-new"));
        assert_eq!(input.id, "acme");
        assert_eq!(form.submit(), ProviderFormAction::Submit(input));

        // Untouched key → `None`, i.e. "leave the stored key alone".
        let mut untouched = ProviderForm::edit(&provider("acme", "Acme", false));
        untouched.base_url = "https://api.acme.test/v2".into();
        let input = untouched.to_input();
        assert_eq!(input.api_key, None);
        assert_eq!(input.effective_api_key(), None);
        assert_eq!(untouched.submit(), ProviderFormAction::Submit(input));
    }

    #[test]
    fn to_input_drops_rows_with_a_blank_id_and_trims_whitespace() {
        let mut form = valid_form();
        form.id = "  acme  ".into();
        form.base_url = "  https://api.acme.test/v1  ".into();
        form.models.push(ProviderModelInput::default());
        form.models
            .push(ProviderModelInput::new("  spaced  ", "Spaced"));
        form.api_key = "  sk-1  ".into();
        let input = form.to_input();
        assert_eq!(input.id, "acme");
        assert_eq!(input.base_url, "https://api.acme.test/v1");
        assert_eq!(input.models.len(), 2);
        assert_eq!(input.models[1].id, "  spaced  ");
        assert_eq!(input.api_key.as_deref(), Some("sk-1"));
        assert_eq!(form.models.len(), 3, "the form state is untouched");

        // A whitespace-only key is "unchanged", not a key made of spaces.
        form.api_key = "   ".into();
        assert_eq!(form.to_input().api_key, None);
    }

    // ─── Form rendering ────────────────────────────────────────────────────

    #[test]
    fn form_rows_are_exactly_width_columns_for_every_width() {
        for width in [0usize, 1, 2, 3, 5, 12, 40, 80, 200] {
            let form = valid_form();
            let rows = form.render(width);
            if width == 0 {
                assert!(rows.is_empty());
                continue;
            }
            assert!(!rows.is_empty());
            for row in &rows {
                assert_eq!(row_width(row), width, "width {width} row {:?}", plain(row));
            }
        }
    }

    #[test]
    fn form_render_lists_every_field_and_marks_the_focused_one() {
        let mut form = valid_form();
        form.field = ProviderFormField::BaseUrl;
        form.api_key = "sk-secret".into();
        let rows = plain_rows(&form.render(80));
        for field in ProviderFormField::ALL {
            let label = field.label();
            let message = format!("missing {label} in {rows:?}");
            assert!(rows.iter().any(|row| row.contains(label)), "{message}");
        }
        let focused = rows
            .iter()
            .find(|row| row.contains(ProviderFormField::BaseUrl.label()))
            .unwrap();
        assert!(focused.starts_with("▸"));
        assert!(
            focused.contains("https://api.acme.test/v1"),
            "the focused field shows its value: {focused:?}"
        );
        assert!(
            rows.iter().all(|row| !row.contains("sk-secret")),
            "the API key must never be echoed: {rows:?}"
        );
        assert!(rows.iter().any(|row| row.contains("••••")));
    }

    #[test]
    fn form_render_reports_placeholders_and_the_fixed_id() {
        let rows = plain_rows(&ProviderForm::add().render(80));
        assert!(rows.iter().any(|row| row.contains("required")));
        assert!(
            rows.iter()
                .any(|row| row.contains("no models — ctrl+n adds one")),
            "empty model list needs a hint: {rows:?}"
        );

        let edited = plain_rows(&ProviderForm::edit(&provider("acme", "Acme", false)).render(80));
        assert!(edited.iter().any(|row| row.contains("acme (fixed)")));
        assert!(edited.iter().any(|row| row.contains("unchanged")));
        assert!(edited
            .iter()
            .any(|row| row.contains("blank key keeps the stored key")));
    }

    #[test]
    fn form_render_shows_the_error_and_the_models_hint() {
        let mut form = valid_form();
        form.field = ProviderFormField::Models;
        form.models
            .push(ProviderModelInput::new("acme-mini", "Mini"));
        form.models[1].supports_images = true;
        form.models[1].thinking = false;
        form.submit();
        assert!(form.error.is_none());

        form.id = "BAD ID".into();
        form.submit();
        let rows = plain_rows(&form.render(120));
        assert!(rows.iter().any(|row| row.starts_with("✗ ")));
        assert!(rows.iter().any(|row| row.contains("acme-mini")));
        assert!(rows.iter().any(|row| row.contains("images")));
        assert!(rows.iter().any(|row| row.contains("ctrl+n add")));
        assert!(rows.iter().any(|row| row.contains("2 configured")));
    }

    #[test]
    fn model_rows_scroll_and_show_the_remaining_counts() {
        let mut form = ProviderForm::add();
        for index in 0..10 {
            form.add_model();
            typed(&mut form, &format!("m{index}"));
        }
        form.field = ProviderFormField::Models;
        form.select_model(9);
        let rows = plain_rows(&form.render(80));
        assert!(rows.iter().any(|row| row.contains("↑")), "rows: {rows:?}");
        assert!(rows.iter().any(|row| row.contains("m9")));
        assert!(
            rows.iter().all(|row| !row.contains("m0")),
            "the window follows the selection: {rows:?}"
        );
    }

    #[test]
    fn model_rows_report_the_rows_below_the_window() {
        let mut form = ProviderForm::add();
        for index in 0..8 {
            form.add_model();
            typed(&mut form, &format!("m{index}"));
        }
        form.field = ProviderFormField::Models;
        form.select_model(0);
        let rows = plain_rows(&form.render(80));
        assert!(rows.iter().any(|row| row.contains("↓ 2 more")), "{rows:?}");
        assert!(rows.iter().all(|row| !row.contains('↑')), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("m0")));
        assert!(rows.iter().all(|row| !row.contains("m7")), "{rows:?}");
    }

    #[test]
    fn model_rows_render_an_empty_id_placeholder() {
        let mut form = ProviderForm::add();
        form.add_model();
        let rows = plain_rows(&form.render(80));
        assert!(rows.iter().any(|row| row.contains("(id required)")));
    }

    #[test]
    fn model_rows_without_flags_render_only_the_id_and_context() {
        let mut form = ProviderForm::add();
        form.add_model();
        form.models[0] = ProviderModelInput::new("m", "M");
        form.models[0].thinking = false;
        form.field = ProviderFormField::Name; // unfocused list → "·" marker
        let rows = plain_rows(&form.render(80));
        let row = rows.iter().find(|row| row.contains("ctx ")).unwrap();
        assert_eq!(row.trim(), "· m  ctx 128000", "no flag text at all");
        // The same row with a flag set carries it between the id and the ctx.
        form.models[0].thinking = true;
        let rows = plain_rows(&form.render(80));
        let row = rows.iter().find(|row| row.contains("ctx ")).unwrap();
        assert_eq!(row.trim(), "· m  thinking  ctx 128000");
    }

    #[test]
    fn fit_row_of_a_zero_width_column_is_empty() {
        assert_eq!(fit_row("abc", 0), "");
        assert_eq!(fit_row("", 0), "");
        assert_eq!(fit_row("abc", 2), "ab");
        assert_eq!(fit_row("abc", 3), "abc");
    }

    #[test]
    fn the_hint_line_follows_the_focused_field() {
        let mut form = valid_form();
        form.field = ProviderFormField::ApiType;
        let rows = plain_rows(&form.render(90));
        let hint = rows.last().unwrap();
        assert!(hint.contains("←/→ cycle API type"), "{hint:?}");
        assert!(!hint.contains("tab/↑↓ move"), "{hint:?}");
        form.field = ProviderFormField::Models;
        let rows = plain_rows(&form.render(90));
        let hint = rows.last().unwrap();
        assert!(hint.contains("ctrl+n add"), "{hint:?}");
    }

    /// Editing an existing provider appends "blank key keeps the stored key" to
    /// the hint list, which no longer fits a standard pane: the cut has to end
    /// in an ellipsis, and the row still fills the pane.
    #[test]
    fn the_edit_mode_hint_line_is_ellipsized_when_it_does_not_fit() {
        let form = ProviderForm::edit(&provider("acme", "Acme", false));
        let rows = form.render(60);
        let hint = &rows[rows.len() - 1];
        assert_eq!(row_width(hint), 60);
        assert_eq!(
            plain(hint),
            "tab/↑↓ move · enter save · esc cancel · blank key keeps the…"
        );
        // A standard 80-column pane fits the whole hint — no stray ellipsis.
        let roomy = plain(&form.render(80).pop().unwrap());
        assert!(roomy.contains("the stored key"), "{roomy:?}");
        assert!(!roomy.contains('…'), "{roomy:?}");
        // With room for it the same hint keeps its last words.
        let wide = form.render(120);
        let hint = plain(wide.last().unwrap());
        assert!(hint.contains("blank key keeps the stored key"), "{hint:?}");
        assert!(!hint.contains('…'), "{hint:?}");
    }

    // ─── List: tabs and search ─────────────────────────────────────────────

    #[test]
    fn list_splits_providers_across_two_tabs() {
        let state = ProviderListState::new(providers());
        assert_eq!(state.providers().len(), 3);
        assert_eq!(state.tab_id(), PROVIDER_TAB_BUILTIN);
        assert_eq!(state.visible_len(), 1);
        assert_eq!(state.highlighted().unwrap().id, "future");
    }

    /// The list has no key handling of its own for the navigation keys it does
    /// not name: it forwards them to its menu, which is what makes the page
    /// keys work in `/providers` (the menu matches the camelCase ids
    /// `parse_key` emits). This is the survey's "not a defect — delegation"
    /// ruling, pinned down as a test.
    #[test]
    fn page_keys_reach_the_menu_the_list_delegates_to() {
        let many: Vec<ProviderInfo> = (0..20)
            .map(|i| provider(&format!("p{i:02}"), &format!("P{i:02}"), false))
            .collect();
        let mut state = ProviderListState::new(many);
        state.select_tab(PROVIDER_TAB_CUSTOM);
        assert_eq!(state.visible_len(), 20);
        // Page size comes from the last render's window.
        state.render(60, 5);
        let first = state.highlighted().unwrap().id.clone();
        assert_eq!(state.handle_key("pageDown"), ProviderListAction::Moved);
        let after_page = state.highlighted().unwrap().id.clone();
        assert_ne!(after_page, first, "pageDown moved past the first row");
        assert_eq!(state.handle_key("home"), ProviderListAction::Moved);
        assert_eq!(state.highlighted().unwrap().id, first);
        assert_eq!(state.handle_key("end"), ProviderListAction::Moved);
        assert_eq!(state.highlighted().unwrap().id, "p19");
    }

    #[test]
    fn tab_keys_cycle_between_builtin_and_custom() {
        let mut state = ProviderListState::new(providers());
        assert_eq!(state.handle_key("tab"), ProviderListAction::Moved);
        assert_eq!(state.tab_id(), PROVIDER_TAB_CUSTOM);
        assert_eq!(state.visible_len(), 2);
        assert_eq!(state.highlighted().unwrap().id, "acme");
        assert_eq!(state.handle_key("shift+tab"), ProviderListAction::Moved);
        assert_eq!(state.tab_id(), PROVIDER_TAB_BUILTIN);
        state.select_tab(PROVIDER_TAB_CUSTOM);
        assert_eq!(state.tab_id(), PROVIDER_TAB_CUSTOM);
        state.select_tab("nope");
        assert_eq!(state.tab_id(), PROVIDER_TAB_CUSTOM, "unknown tab ignored");
    }

    #[test]
    fn typing_filters_the_active_tab_case_insensitively() {
        let mut state = ProviderListState::new(providers());
        state.select_tab(PROVIDER_TAB_CUSTOM);
        for ch in "ZETA".chars() {
            state.handle_key(&ch.to_string());
        }
        assert!(state.is_searching());
        assert_eq!(state.filter(), "ZETA");
        assert_eq!(state.visible_len(), 1);
        assert_eq!(state.highlighted().unwrap().id, "zeta");
        // `esc` clears the filter and leaves the overlay open.
        assert_eq!(state.handle_key("escape"), ProviderListAction::Moved);
        assert!(!state.is_searching());
        assert_eq!(state.visible_len(), 2);
        assert_eq!(state.handle_key("escape"), ProviderListAction::Cancelled);
    }

    #[test]
    fn search_matches_the_description_too_and_can_be_emptied() {
        let mut state = ProviderListState::new(providers());
        state.select_tab(PROVIDER_TAB_CUSTOM);
        state.handle_key("/");
        assert!(state.is_searching());
        typed_into(&mut state, "acme.test");
        assert_eq!(state.visible_len(), 1);
        assert_eq!(state.highlighted().unwrap().id, "acme");
        assert_eq!(state.handle_key("backspace"), ProviderListAction::Moved);
        assert_eq!(state.filter(), "acme.tes");
        while !state.filter().is_empty() {
            state.handle_key("backspace");
        }
        assert_eq!(state.visible_len(), 2, "the unfiltered custom tab is back");
        assert!(!state.is_searching());
    }

    fn typed_into(state: &mut ProviderListState, text: &str) {
        for ch in text.chars() {
            state.handle_key(&ch.to_string());
        }
    }

    #[test]
    fn a_search_started_with_slash_may_contain_action_letters() {
        let mut state = ProviderListState::new(providers());
        state.handle_key("/");
        // `a`, `d`, `e`… are actions only while the search row is idle.
        for ch in "dead".chars() {
            assert_eq!(state.handle_key(&ch.to_string()), ProviderListAction::Moved);
        }
        assert_eq!(state.filter(), "dead");
        assert_eq!(state.visible_len(), 0, "no provider matches");
    }

    // ─── List: actions ─────────────────────────────────────────────────────

    #[test]
    fn enter_edits_a_custom_provider_and_sets_the_key_of_a_builtin() {
        let mut state = ProviderListState::new(providers());
        assert_eq!(
            state.handle_key("enter"),
            ProviderListAction::SetKey(provider("future", "Future", true))
        );
        assert!(state.notice().is_some(), "the caller is told why");

        state.select_tab(PROVIDER_TAB_CUSTOM);
        assert_eq!(
            state.handle_key("enter"),
            ProviderListAction::Edit(provider("acme", "Acme", false))
        );
        assert!(state.notice().is_none());
        assert_eq!(
            state.handle_key("e"),
            ProviderListAction::Edit(provider("acme", "Acme", false))
        );
        assert_eq!(
            state.handle_key("ctrl+e"),
            ProviderListAction::Edit(provider("acme", "Acme", false))
        );
    }

    #[test]
    fn editing_or_deleting_a_builtin_is_refused_with_a_notice() {
        let mut state = ProviderListState::new(providers());
        assert_eq!(state.handle_key("e"), ProviderListAction::None);
        assert!(state.notice().unwrap().contains("cannot be edited"));
        assert_eq!(state.handle_key("d"), ProviderListAction::None);
        assert!(state.notice().unwrap().contains("cannot be deleted"));
        // Navigation clears the notice again (`tab` always changes the tab,
        // unlike `down` on a single-row tab, which reports nothing moved).
        assert_eq!(state.handle_key("tab"), ProviderListAction::Moved);
        assert!(state.notice().is_none());
    }

    #[test]
    fn delete_removes_a_custom_provider() {
        let mut state = ProviderListState::new(providers());
        state.select_tab(PROVIDER_TAB_CUSTOM);
        assert_eq!(
            state.handle_key("d"),
            ProviderListAction::Delete(provider("acme", "Acme", false))
        );
        assert_eq!(
            state.handle_key("ctrl+d"),
            ProviderListAction::Delete(provider("acme", "Acme", false))
        );
        assert_eq!(state.handle_key("down"), ProviderListAction::Moved);
        assert_eq!(
            state.handle_key("delete"),
            ProviderListAction::Delete(provider("zeta", "Zeta", false))
        );
    }

    #[test]
    fn set_key_add_sync_and_reload_are_reachable_from_the_list() {
        let mut state = ProviderListState::new(providers());
        assert_eq!(
            state.handle_key("k"),
            ProviderListAction::SetKey(provider("future", "Future", true))
        );
        assert_eq!(state.handle_key("a"), ProviderListAction::Add);
        assert_eq!(state.handle_key("ctrl+n"), ProviderListAction::Add);
        assert_eq!(state.handle_key("s"), ProviderListAction::SyncModels);
        assert_eq!(state.handle_key("ctrl+s"), ProviderListAction::SyncModels);
        assert_eq!(state.handle_key("r"), ProviderListAction::ReloadAuth);
        assert_eq!(state.handle_key("ctrl+r"), ProviderListAction::ReloadAuth);
    }

    #[test]
    fn actions_on_an_empty_tab_report_none_instead_of_panicking() {
        let mut state = ProviderListState::new(Vec::new());
        assert!(state.highlighted().is_none());
        assert_eq!(state.handle_key("k"), ProviderListAction::None);
        assert_eq!(state.handle_key("e"), ProviderListAction::None);
        assert_eq!(state.handle_key("d"), ProviderListAction::None);
        assert_eq!(state.handle_key("enter"), ProviderListAction::None);
        assert_eq!(state.handle_key("a"), ProviderListAction::Add);
        assert!(state.notice().is_none());
    }

    #[test]
    fn an_unknown_key_reports_none_but_navigation_reports_moved() {
        let mut state = ProviderListState::new(providers());
        // Two rows on the custom tab, so a move actually changes the highlight
        // (a single-row tab reports nothing moved).
        state.select_tab(PROVIDER_TAB_CUSTOM);
        assert_eq!(state.handle_key("f5"), ProviderListAction::None);
        assert_eq!(state.handle_key("down"), ProviderListAction::Moved);
        assert_eq!(state.highlighted().unwrap().id, "zeta");
        assert_eq!(state.handle_key("j"), ProviderListAction::Moved);
        assert_eq!(state.highlighted().unwrap().id, "acme", "wraps to the top");
    }

    // ─── List: badges, refresh, rendering ───────────────────────────────────

    #[test]
    fn the_default_model_marks_its_provider() {
        let mut state = ProviderListState::new(providers());
        state.set_default_model("acme/acme-large");
        state.select_tab(PROVIDER_TAB_CUSTOM);
        let rows = plain_rows(&state.render(90, 10));
        let acme = rows.iter().find(|row| row.contains("Acme")).unwrap();
        assert!(acme.contains("default"), "row: {acme}");
        assert_eq!(state.default_model(), "acme/acme-large");

        // A bare model id names a model, not a provider: nothing is marked.
        state.set_default_model("acme-large");
        let rows = plain_rows(&state.render(90, 10));
        assert!(rows.iter().all(|row| !row.contains("default")));
        // A leading slash qualifies nothing either — the prefix is empty.
        state.set_default_model("/acme-large");
        let rows = plain_rows(&state.render(90, 10));
        assert!(rows.iter().all(|row| !row.contains("default")), "{rows:?}");
        state.set_default_model("");
        let rows = plain_rows(&state.render(90, 10));
        assert!(rows.iter().all(|row| !row.contains("default")));
    }

    #[test]
    fn rows_carry_key_and_model_count_badges() {
        let mut state = ProviderListState::new(providers());
        let builtin = plain_rows(&state.render(90, 10));
        assert!(builtin.iter().any(|row| row.contains("12 models")));
        assert!(builtin.iter().any(|row| row.contains("key")));
        state.select_tab(PROVIDER_TAB_CUSTOM);
        let custom = plain_rows(&state.render(90, 10));
        assert!(custom.iter().any(|row| row.contains("1 models")));
        assert!(
            custom.iter().any(|row| row.contains("openai-completions")),
            "custom rows show their API dialect: {custom:?}"
        );

        let mut keyless = ProviderListState::new(vec![{
            let mut plain = provider("plain", "Plain", true);
            plain.base_url = String::new();
            plain
        }]);
        let rows = plain_rows(&keyless.render(90, 10));
        assert!(rows.iter().any(|row| row.contains("no key")));
        assert!(rows.iter().any(|row| row.contains("built-in catalog")));
    }

    #[test]
    fn a_custom_provider_without_a_base_url_says_so() {
        let mut bare = provider("local", "Local", false);
        bare.base_url = String::new();
        let mut state = ProviderListState::new(vec![bare]);
        state.select_tab(PROVIDER_TAB_CUSTOM);
        let rows = plain_rows(&state.render(90, 10));
        assert!(rows.iter().any(|row| row.contains("no base URL")));
        assert!(rows.iter().all(|row| !row.contains("built-in catalog")));
    }

    #[test]
    fn a_provider_without_a_name_renders_its_id() {
        let mut bare = provider("bare", "", false);
        bare.base_url = "https://example.test/v1".into();
        let mut state = ProviderListState::new(vec![bare]);
        state.select_tab(PROVIDER_TAB_CUSTOM);
        let rows = plain_rows(&state.render(90, 6));
        // The row is "› key 1 models bare  openai-completions  …": the label
        // after the badges is the id, because the name is empty.
        assert!(
            rows.iter().any(|row| row.contains("1 models bare")),
            "{rows:?}"
        );
    }

    #[test]
    fn set_providers_refreshes_both_tabs_and_keeps_the_active_one() {
        let mut state = ProviderListState::new(providers());
        state.select_tab(PROVIDER_TAB_CUSTOM);
        state.set_providers(vec![provider("solo", "Solo", false)]);
        assert_eq!(state.tab_id(), PROVIDER_TAB_CUSTOM);
        assert_eq!(state.visible_len(), 1);
        assert_eq!(state.highlighted().unwrap().id, "solo");
        assert_eq!(state.providers().len(), 1);
    }

    #[test]
    fn set_notice_accepts_and_clears_a_caller_message() {
        let mut state = ProviderListState::new(providers());
        state.set_notice(Some("sync failed"));
        let rows = plain_rows(&state.render(60, 6));
        assert!(rows.iter().any(|row| row.contains("sync failed")));
        state.set_notice(None);
        let rows = plain_rows(&state.render(60, 6));
        assert!(rows.iter().all(|row| !row.contains("sync failed")));
    }

    #[test]
    fn list_rows_are_exactly_width_columns_and_respect_height() {
        let mut state = ProviderListState::new(providers());
        state.set_notice(Some(
            "a rather long notice that will not fit in a narrow overlay",
        ));
        for width in [1usize, 3, 10, 40, 120] {
            for height in [1usize, 2, 5, 20] {
                let rows = state.render(width, height);
                assert!(rows.len() <= height, "width {width} height {height}");
                for row in &rows {
                    assert_eq!(row_width(row), width, "row {:?}", plain(row));
                }
            }
        }
        assert!(state.render(0, 5).is_empty());
        assert!(state.render(40, 0).is_empty());
    }

    #[test]
    fn an_empty_list_renders_its_empty_text() {
        let mut state = ProviderListState::new(Vec::new());
        let rows = plain_rows(&state.render(40, 6));
        assert!(rows.iter().any(|row| row.contains("No providers")));
    }

    #[test]
    fn a_filter_with_no_match_renders_the_query() {
        let mut state = ProviderListState::new(providers());
        state.handle_key("/");
        typed_into(&mut state, "zzz");
        let rows = plain_rows(&state.render(40, 6));
        assert!(rows
            .iter()
            .any(|row| row.contains("No providers") && row.contains("zzz")));
    }

    // ─── Overlay adapters ──────────────────────────────────────────────

    /// The provider list claims `escape` exactly while its search is live, so
    /// the app layer hands the first escape to it (clear the query) instead of
    /// closing `/providers`.
    #[test]
    fn list_overlay_claims_escape_only_while_searching() {
        let mut overlay = ProviderListOverlay::new(
            ProviderListState::new(vec![
                provider("acme", "Acme", false),
                provider("zeta", "Zeta", false),
            ]),
            Box::new(|_| {}),
        );
        assert!(!overlay.wants_escape(), "idle list: escape closes it");

        // `z` is not a single-letter action, so it starts the search.
        overlay.handle_input("z");
        assert!(overlay.wants_escape());
        assert_eq!(overlay.state().filter(), "z");

        overlay.handle_input("escape");
        assert!(
            !overlay.wants_escape(),
            "the query is cleared: the next escape is the app's"
        );
        assert!(!overlay.state().is_searching());
        assert_eq!(overlay.state().filter(), "");
    }

    #[test]
    fn list_overlay_forwards_actions_and_renders_the_natural_height() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let seen: Rc<RefCell<Vec<ProviderListAction>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let mut overlay = ProviderListOverlay::new(
            ProviderListState::new(vec![provider("acme", "Acme", false)]),
            Box::new(move |action| sink.borrow_mut().push(action)),
        );
        let rows = overlay.render(70);
        assert!(!rows.is_empty());
        assert_eq!(overlay.state().providers().len(), 1);
        // Navigation and tab keys only redraw (`Moved`), they carry no value.
        overlay.handle_input("down");
        overlay.handle_input("tab");
        let moved: Vec<ProviderListAction> = seen.borrow().clone();
        assert!(
            moved
                .iter()
                .all(|action| *action == ProviderListAction::Moved),
            "{moved:?}"
        );
        seen.borrow_mut().clear();
        // `a` (add) carries a value.
        overlay.state_mut().select_tab(PROVIDER_TAB_CUSTOM);
        overlay.handle_input("a");
        assert_eq!(seen.borrow().as_slice(), &[ProviderListAction::Add]);
        overlay.handle_input("escape");
        assert_eq!(seen.borrow().len(), 2);
        assert_eq!(seen.borrow()[1], ProviderListAction::Cancelled);
        overlay.set_theme(crate::theme::Theme {
            selected_bg: 123,
            ..crate::theme::DARK_THEME
        });
        assert!(overlay
            .as_any()
            .downcast_ref::<ProviderListOverlay>()
            .is_some());
        assert!(overlay
            .as_any_mut()
            .downcast_mut::<ProviderListOverlay>()
            .is_some());
        overlay.invalidate();
    }

    #[test]
    fn form_overlay_forwards_submit_and_cancel() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let seen: Rc<RefCell<Vec<ProviderFormAction>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let mut form = ProviderForm::add();
        form.id = "acme".into();
        form.name = "Acme".into();
        form.base_url = "http://localhost:8080".into();
        let mut overlay =
            ProviderFormOverlay::new(form, Box::new(move |action| sink.borrow_mut().push(action)));
        assert!(!overlay.render(70).is_empty());
        assert!(!overlay.form().is_id_locked());
        overlay.handle_input("enter");
        assert!(matches!(seen.borrow()[0], ProviderFormAction::Submit(_)));
        overlay.handle_input("escape");
        assert_eq!(seen.borrow()[1], ProviderFormAction::Cancel);
        overlay.form_mut().name = "Renamed".into();
        assert_eq!(overlay.form().name, "Renamed");
        assert!(overlay
            .as_any()
            .downcast_ref::<ProviderFormOverlay>()
            .is_some());
        assert!(overlay
            .as_any_mut()
            .downcast_mut::<ProviderFormOverlay>()
            .is_some());
        overlay.invalidate();
    }
}
