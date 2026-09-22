//! Sandbox tier and tool-permission settings for the TUI.
//!
//! Port of the desktop's approval-mode surface: the tier `<Select>` in
//! `desktop/src/features/settings/GeneralPage.tsx`, the availability probe in
//! `integrations/agent/useSandboxAvailability.ts` and the stable diagnostic
//! codes in `features/settings/linuxSandboxStatus.ts`.
//!
//! The agent owns both halves of the state — the policy
//! (`set_sandbox_policy`, tier `off` | `manual` | `sandbox`) and the probe
//! (`probe_sandbox`) — so this module is deliberately I/O-free: it holds the
//! last status the app learned, renders it, and reports *intent*
//! ([`SandboxAction`]) for the caller to turn into an RPC. Same contract as
//! [`crate::components::menu`]: `Moved` means "redraw", value-carrying actions
//! mean "act (then feed the result back through [`SandboxView::set_status`])".
//!
//! Design notes:
//!
//! * **The sandbox tier needs an available backend.** `sandbox` is disabled
//!   while the probe has not reported one and navigation skips the row; the
//!   other two tiers are always selectable. The gate is *availability*, not
//!   [`supports_sandbox`]: a Linux host with a working Bubblewrap reports
//!   `available` and may run sandboxed (that is why
//!   `approvalTier.description.sandboxLinux` exists), whereas
//!   [`supports_sandbox`] answers the narrower question "does this OS have a
//!   native sandbox at all".
//! * **Checking is not unavailable.** An unresolved probe (`resolved == false`)
//!   renders "checking the system sandbox…"; a resolved-but-unavailable probe
//!   renders the diagnostic [reason](reason_text) together with its code. The
//!   desktop distinguishes the two the same way (`sandboxChecking` vs
//!   `sandboxUnavailable`).
//! * **A downgrade is never hidden.** The agent answers
//!   `set_sandbox_policy { tier: "sandbox" }` with `requestedTier: "sandbox",
//!   tier: "manual"` when the probe says the sandbox is unavailable.
//!   [`SandboxStatus::fallback_notice`] turns that pair into an explicit
//!   "fell back to Manual" banner which [`SandboxView::render`] prints:
//!   reporting the request as applied would lie to the user.
//! * **Rendering is width-total.** Every row measures exactly `width` visible
//!   columns ([`crate::utils::visible_width`]) for any `width` including `0`
//!   and `1`, the row count never exceeds `height`, and no input panics.

use serde_json::{Map, Value};

use crate::theme::{bold, fg, Theme, DARK_THEME};
use crate::utils::{
    apply_background_to_line, truncate_to_width, visible_width, wrap_text_with_ansi,
    TruncateOptions,
};

// ─── Copy ──────────────────────────────────────────────────────────────────

/// Panel title.
const TITLE: &str = "Sandbox & permissions";
/// Heading of the approval-tier section (desktop `approvalTier.title`).
const TIER_HEADING: &str = "Approval mode";
/// Heading of the tool-permission section.
const PERMISSION_HEADING: &str = "Tool permissions";
/// Key legend rendered as the last row.
const KEY_LEGEND: &str = "↑/↓ move · enter applies · r re-checks · esc closes";
/// `off` description (desktop `approvalTier.description.off`).
const DESC_OFF: &str = "No prompts, no sandbox — everything runs.";
/// `manual` description (desktop `approvalTier.description.manual`).
const DESC_MANUAL: &str = "File access follows approval rules; allowlisted read-only commands run automatically, other commands ask. No OS sandbox.";
/// `sandbox` description on macOS.
const DESC_SANDBOX_MACOS: &str =
    "Commands run in the macOS sandbox and ask for approval when needed.";
/// `sandbox` description on Linux (desktop `approvalTier.description.sandboxLinux`).
const DESC_SANDBOX_LINUX: &str =
    "Commands run in the Linux sandbox and ask for approval when needed.";
/// `sandbox` description on Windows (desktop `approvalTier.description.sandboxWindows`).
const DESC_SANDBOX_WINDOWS: &str =
    "Commands run with Windows write protection and ask for approval when needed.";
/// `sandbox` description where no backend exists at all.
const DESC_SANDBOX_UNSUPPORTED: &str =
    "This platform has no sandbox backend, so the sandbox tier cannot be selected here.";
/// The agent is still probing the host.
const AVAILABILITY_CHECKING: &str = "Sandbox: checking the system sandbox…";
/// Generic unavailability text used when the code is missing or unknown.
const REASON_UNKNOWN: &str =
    "The system sandbox is unavailable. Run `future doctor` for details, then restart the agent.";
/// No backend exists on this platform (the desktop's `platform_unsupported`).
const REASON_PLATFORM_UNSUPPORTED: &str =
    "This platform has no sandbox backend, so the sandbox tier cannot be used here.";
/// A successful probe, for callers that ask for a reason anyway.
const REASON_AVAILABLE: &str = "The system sandbox is available.";
/// Stand-in diagnostic when the agent reported none (desktop does the same).
const CODE_FALLBACK: &str = "probe_failed";

// ─── Tier ──────────────────────────────────────────────────────────────────

/// The approval tier a session runs under (`SandboxPolicy.tier`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxTier {
    /// `"off"` — no approval, no sandbox, everything runs.
    Off,
    /// `"manual"` — approval rules on, no OS sandbox. The agent's default.
    Manual,
    /// `"sandbox"` — approval rules on, shell runs inside the OS sandbox.
    Sandbox,
}

impl SandboxTier {
    /// Display order of the picker, matching the desktop's `<Select>`:
    /// Manual, Sandboxed, Unrestricted.
    pub const ALL: [SandboxTier; 3] = [SandboxTier::Manual, SandboxTier::Sandbox, SandboxTier::Off];

    /// The wire string the agent stores (`"off"` | `"manual"` | `"sandbox"`).
    pub fn to_wire(self) -> &'static str {
        match self {
            SandboxTier::Off => "off",
            SandboxTier::Manual => "manual",
            SandboxTier::Sandbox => "sandbox",
        }
    }

    /// Parse a wire string, tolerating surrounding space and case.
    ///
    /// Returns `None` for anything else so the *caller* owns the fallback; the
    /// agent maps an unknown tier to `Manual` (`SandboxTier::parse`), which is
    /// what [`SandboxStatus::from_policy_response`] does too.
    pub fn from_wire(raw: &str) -> Option<SandboxTier> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Some(SandboxTier::Off),
            "manual" => Some(SandboxTier::Manual),
            "sandbox" => Some(SandboxTier::Sandbox),
            _ => None,
        }
    }
}

/// The label shown in the tier picker (desktop `approvalTier.*`).
pub fn tier_label(tier: SandboxTier) -> &'static str {
    match tier {
        SandboxTier::Off => "Unrestricted",
        SandboxTier::Manual => "Manual",
        SandboxTier::Sandbox => "Sandboxed",
    }
}

/// The long description of `tier` on `platform` (desktop
/// `approvalTier.description.*`, including the per-platform `sandbox` copies).
pub fn tier_description(tier: SandboxTier, platform: SandboxPlatform) -> &'static str {
    match tier {
        SandboxTier::Off => DESC_OFF,
        SandboxTier::Manual => DESC_MANUAL,
        SandboxTier::Sandbox => match platform {
            SandboxPlatform::Macos => DESC_SANDBOX_MACOS,
            SandboxPlatform::Linux => DESC_SANDBOX_LINUX,
            SandboxPlatform::Windows => DESC_SANDBOX_WINDOWS,
            SandboxPlatform::Other => DESC_SANDBOX_UNSUPPORTED,
        },
    }
}

// ─── Platform ──────────────────────────────────────────────────────────────

/// Where the TUI is running, for the platform-specific sandbox copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxPlatform {
    Macos,
    Linux,
    Windows,
    Other,
}

impl SandboxPlatform {
    /// Rust's `std::env::consts::OS` spellings.
    pub fn from_os(os: &str) -> SandboxPlatform {
        match os {
            "macos" => SandboxPlatform::Macos,
            "linux" => SandboxPlatform::Linux,
            "windows" => SandboxPlatform::Windows,
            _ => SandboxPlatform::Other,
        }
    }

    /// The host this binary was built for.
    pub fn current() -> SandboxPlatform {
        SandboxPlatform::from_os(std::env::consts::OS)
    }
}

/// `true` only for a platform with its own OS sandbox (macOS Seatbelt).
///
/// Deliberately *not* the gate for [`SandboxView::tier_options`]: Linux reaches
/// the sandbox tier through Bubblewrap (see the module note).
pub fn supports_sandbox(platform: SandboxPlatform) -> bool {
    matches!(platform, SandboxPlatform::Macos)
}

/// The tier a session should start on for `platform`.
///
/// macOS is sandboxable by construction (Seatbelt is part of the OS), so it
/// starts sandboxed; everywhere else the sandbox is a separate package that has
/// to be probed first, so the agent's own default — `manual` — applies.
pub fn platform_default_tier(platform: SandboxPlatform) -> SandboxTier {
    match platform {
        SandboxPlatform::Macos => SandboxTier::Sandbox,
        SandboxPlatform::Linux | SandboxPlatform::Windows | SandboxPlatform::Other => {
            SandboxTier::Manual
        }
    }
}

/// The platform-specific caveat shown under the tier list, or `None` when the
/// platform has nothing extra to say (macOS).
pub fn platform_note(platform: SandboxPlatform) -> Option<&'static str> {
    match platform {
        SandboxPlatform::Macos => None,
        SandboxPlatform::Linux => Some(
            "Sandboxing is native only to macOS: on Linux the agent runs it through the system Bubblewrap and falls back to Manual when Bubblewrap is missing, untrusted or too old.",
        ),
        SandboxPlatform::Windows => Some(
            "Windows has no OS sandbox: the sandbox tier is not used there, and commands run under FutureOS write protection instead.",
        ),
        SandboxPlatform::Other => {
            Some("No sandbox backend exists on this platform, so the sandbox tier is unavailable.")
        }
    }
}

// ─── Diagnostics ───────────────────────────────────────────────────────────

/// The stable diagnostics the agent's sandbox probes emit — the 12 keys of the
/// desktop's `LINUX_UNAVAILABLE_REASON_KEYS`.
pub const REASON_CODES: [&str; 12] = [
    "binary_missing",
    "path_rejected",
    "binary_invalid",
    "version_unreadable",
    "version_too_old",
    "required_feature_missing",
    "user_namespace_disabled",
    "proc_mount_restricted",
    "probe_timeout",
    "probe_failed",
    "binary_identity_changed",
    "probe_transport_error",
];

/// User-facing explanation of a probe diagnostic.
///
/// Never panics: a missing or unknown code falls back to [`REASON_UNKNOWN`].
/// The text is a sentence without the code — callers render the code next to it
/// (`Sandbox: unavailable — <reason> (diagnostic: <code>)`).
pub fn reason_text(code: Option<&str>) -> &'static str {
    match code {
        Some("binary_missing") => "Bubblewrap is not installed. Install Bubblewrap, then restart the agent.",
        Some("path_rejected") => "No trusted system Bubblewrap was found. Check PATH and confirm the bwrap file is owned by root, then restart the agent.",
        Some("binary_invalid") => "System Bubblewrap is not a valid executable. Reinstall it or repair its permissions, then restart the agent.",
        Some("version_unreadable") => "The Bubblewrap version could not be recognized. Reinstall or upgrade Bubblewrap, then restart the agent.",
        Some("version_too_old") => "Bubblewrap is older than version 0.9.0. Upgrade it, then restart the agent.",
        Some("required_feature_missing") => "This Bubblewrap package lacks features the sandbox requires. Upgrade or replace the system package, then restart the agent.",
        Some("user_namespace_disabled") => "Unprivileged user namespaces are disabled. Enable them, then restart the agent.",
        Some("proc_mount_restricted") => "The system does not allow the sandbox to mount a private /proc. Adjust the system or container policy, then restart the agent.",
        Some("probe_timeout") => "Bubblewrap detection timed out. Check the system, then restart the agent and try again.",
        Some("probe_failed") => "The sandbox check failed. Run `future doctor` for details, fix the issue, then restart the agent.",
        Some("binary_identity_changed") => "The Bubblewrap file changed after detection. Verify the system package, then restart the agent.",
        Some("probe_transport_error") => "The agent could not be reached to check the sandbox. Try again later or restart the agent.",
        Some("platform_unsupported") => REASON_PLATFORM_UNSUPPORTED,
        Some("available") => REASON_AVAILABLE,
        _ => REASON_UNKNOWN,
    }
}

// ─── Probe ─────────────────────────────────────────────────────────────────

/// The three states the desktop's `useSandboxAvailability` models with its
/// `resolved`/`available` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeState {
    /// No answer yet (`resolved == false`) — neither available nor unavailable.
    Checking,
    /// A backend is present and usable.
    Available,
    /// The probe came back without a usable backend.
    Unavailable,
}

/// Availability of the OS sandbox backend on this host.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SandboxProbe {
    /// The agent reported a usable sandbox backend.
    pub available: bool,
    /// The probe finished. `false` means "not checked yet", never "unusable".
    pub resolved: bool,
    /// The answer is authoritative (an RPC that came back) rather than a
    /// transport failure that may clear up on retry.
    pub definitive: bool,
    /// Stable diagnostic code (`binary_missing`, …).
    pub code: Option<String>,
    /// Backend name (`macos_seatbelt`, `bubblewrap`, `none`).
    pub backend: Option<String>,
    /// Absolute path of the backend binary, when the probe reported one.
    pub path: Option<String>,
    /// Backend version, when the probe reported one.
    pub version: Option<String>,
}

impl SandboxProbe {
    /// Nothing known yet — the desktop's pre-probe state.
    pub fn checking() -> Self {
        SandboxProbe::default()
    }

    /// A definitive, successful probe.
    pub fn available(backend: &str) -> Self {
        SandboxProbe {
            available: true,
            resolved: true,
            definitive: true,
            code: Some("available".to_string()),
            backend: Some(backend.to_string()),
            ..SandboxProbe::default()
        }
    }

    /// A definitive, failed probe.
    pub fn unavailable(code: &str, backend: &str) -> Self {
        SandboxProbe {
            available: false,
            resolved: true,
            definitive: true,
            code: Some(code.to_string()),
            backend: Some(backend.to_string()),
            ..SandboxProbe::default()
        }
    }

    /// Parse a `probe_sandbox` result.
    ///
    /// Tolerant by design: a payload that is not an object yields
    /// [`SandboxProbe::checking`], and `resolved`/`definitive` default to
    /// `true` because an RPC that returned *is* an answer.
    pub fn from_probe_response(value: &Value) -> Self {
        match value.as_object() {
            None => SandboxProbe::checking(),
            Some(map) => SandboxProbe {
                available: field_bool(map, &["available"]).unwrap_or(false),
                resolved: field_bool(map, &["resolved"]).unwrap_or(true),
                definitive: field_bool(map, &["definitive"]).unwrap_or(true),
                code: field_text(map, &["code"]),
                backend: field_text(map, &["backend"]),
                path: field_text(map, &["path"]),
                version: field_text(map, &["version"]),
            },
        }
    }

    /// Which of the three states the probe is in.
    pub fn state(&self) -> ProbeState {
        match (self.resolved, self.available) {
            (false, _) => ProbeState::Checking,
            (true, true) => ProbeState::Available,
            (true, false) => ProbeState::Unavailable,
        }
    }

    /// The diagnostic code to show, or the desktop's `probe_failed` stand-in.
    pub fn diagnostic_code(&self) -> &str {
        match self.code.as_deref() {
            Some(code) => code,
            None => CODE_FALLBACK,
        }
    }

    /// The user-facing reason the sandbox is unusable.
    pub fn reason(&self) -> &'static str {
        reason_text(self.code.as_deref())
    }

    /// `backend bubblewrap · /usr/bin/bwrap · 1.2`, omitting whatever the probe
    /// did not report (empty when it reported none of the three).
    pub fn detail(&self) -> String {
        let parts = [
            self.backend
                .as_deref()
                .map(|backend| format!("backend {backend}")),
            self.path.as_deref().map(str::to_string),
            self.version.as_deref().map(str::to_string),
        ];
        parts
            .into_iter()
            .flatten()
            .collect::<Vec<String>>()
            .join(" · ")
    }
}

// ─── Status ────────────────────────────────────────────────────────────────

/// Everything the settings screen knows about the sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxStatus {
    /// The tier the session is actually running under.
    pub tier: SandboxTier,
    /// The tier the last `set_sandbox_policy` asked for, when the app has seen
    /// its answer. `Some(Sandbox)` together with `tier == Manual` is the agent's
    /// silent downgrade.
    pub requested_tier: Option<SandboxTier>,
    /// Last known probe result.
    pub probe: SandboxProbe,
    /// Host platform, for the platform-specific copy.
    pub platform: SandboxPlatform,
}

impl SandboxStatus {
    /// The status before any RPC, mirroring the desktop's
    /// `initialAvailability()`: macOS is sandboxed by construction, an
    /// unsupported platform is definitively unsandboxable, and Linux/Windows
    /// have to ask the agent (so they render as "checking").
    pub fn new(platform: SandboxPlatform) -> Self {
        SandboxStatus {
            tier: platform_default_tier(platform),
            requested_tier: None,
            probe: initial_probe(platform),
            platform,
        }
    }

    /// Parse the `set_sandbox_policy` answer, which is where the downgrade is
    /// visible (`tier` + `requestedTier` + `sandboxAvailable`).
    ///
    /// Tolerant: a payload that is not an object yields [`SandboxStatus::new`],
    /// an unrecognised `tier` falls back to the platform default (the agent's
    /// own `parse` does the same), and an answer without `sandboxAvailable`
    /// leaves the probe unresolved instead of claiming unavailability.
    pub fn from_policy_response(platform: SandboxPlatform, value: &Value) -> Self {
        match value.as_object() {
            None => SandboxStatus::new(platform),
            Some(map) => {
                let tier_raw = field_text(map, &["tier"]);
                let tier = tier_raw
                    .as_deref()
                    .and_then(SandboxTier::from_wire)
                    .unwrap_or(platform_default_tier(platform));
                let requested_raw = field_text(map, &["requestedTier", "requested_tier"]);
                let requested_tier = requested_raw.as_deref().and_then(SandboxTier::from_wire);
                let probe = match field_bool(map, &["sandboxAvailable", "sandbox_available"]) {
                    Some(available) => SandboxProbe {
                        available,
                        resolved: true,
                        definitive: true,
                        code: field_text(map, &["sandboxCode", "sandbox_code"]),
                        backend: field_text(map, &["sandboxBackend", "sandbox_backend"]),
                        path: None,
                        version: None,
                    },
                    None => SandboxProbe::checking(),
                };
                SandboxStatus {
                    tier,
                    requested_tier,
                    probe,
                    platform,
                }
            }
        }
    }

    /// Which of the three probe states the status is in.
    pub fn probe_state(&self) -> ProbeState {
        self.probe.state()
    }

    /// `true` when the session asked for `sandbox` and did not get it. The agent
    /// downgrades silently to `manual`, so this is the only signal that the
    /// request was not honoured.
    pub fn is_fallback(&self) -> bool {
        self.requested_tier == Some(SandboxTier::Sandbox) && self.tier != SandboxTier::Sandbox
    }

    /// The banner explaining [`SandboxStatus::is_fallback`], or `None` when the
    /// requested tier was applied.
    pub fn fallback_notice(&self) -> Option<String> {
        if !self.is_fallback() {
            return None;
        }
        Some(format!(
            "Sandbox requested but unavailable — fell back to {}: approved or allowlisted commands run without an OS sandbox (diagnostic: {}).",
            tier_label(self.tier),
            self.probe.diagnostic_code()
        ))
    }
}

/// The pre-RPC probe state (desktop `initialAvailability`).
fn initial_probe(platform: SandboxPlatform) -> SandboxProbe {
    match platform {
        SandboxPlatform::Macos => SandboxProbe::available("macos_seatbelt"),
        SandboxPlatform::Linux | SandboxPlatform::Windows => SandboxProbe::checking(),
        SandboxPlatform::Other => SandboxProbe::unavailable("platform_unsupported", "none"),
    }
}

// ─── Permission level ──────────────────────────────────────────────────────

/// Tool-execution permission level (`set_permission_level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    /// `"all"` — every tool call runs without asking. The agent's default.
    All,
    /// `"workspace"` — tool calls go through the approval gate.
    Workspace,
    /// `"none"` — every tool call is denied.
    None,
}

impl PermissionKind {
    /// Display order, matching the wire vocabulary `all | workspace | none`.
    pub const ALL: [PermissionKind; 3] = [
        PermissionKind::All,
        PermissionKind::Workspace,
        PermissionKind::None,
    ];

    /// The wire string for `set_permission_level`.
    pub fn to_wire(self) -> &'static str {
        match self {
            PermissionKind::All => "all",
            PermissionKind::Workspace => "workspace",
            PermissionKind::None => "none",
        }
    }

    /// Parse a wire string, tolerating surrounding space and case.
    pub fn from_wire(raw: &str) -> Option<PermissionKind> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "all" => Some(PermissionKind::All),
            "workspace" => Some(PermissionKind::Workspace),
            "none" => Some(PermissionKind::None),
            _ => None,
        }
    }

    /// Label shown in the picker.
    pub fn label(self) -> &'static str {
        match self {
            PermissionKind::All => "All",
            PermissionKind::Workspace => "Workspace",
            PermissionKind::None => "None",
        }
    }

    /// What the agent does with a tool call at this level (see the `before_tool_call`
    /// hook in `agent/src/rpc/session_prompt.rs`).
    pub fn description(self) -> &'static str {
        match self {
            PermissionKind::All => "Every tool call runs without asking.",
            PermissionKind::Workspace => {
                "Tool calls ask for approval before they run; the grant is scoped to this workspace."
            }
            PermissionKind::None => "Every tool call is denied before it runs.",
        }
    }
}

/// The three-level permission picker, rendered under the tier list.
///
/// It owns its own highlight and is *focused* only while the enclosing
/// [`SandboxView`] has moved the selection into it, so the highlight bar is
/// never drawn in two sections at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionView {
    level: PermissionKind,
    index: usize,
    focused: bool,
    theme: Theme,
}

impl PermissionView {
    /// A picker whose highlight starts on `level`.
    pub fn new(level: PermissionKind) -> Self {
        PermissionView {
            level,
            index: PermissionView::index_of(level),
            focused: false,
            theme: DARK_THEME,
        }
    }

    /// Replace the applied level (after `set_permission_level` came back) and
    /// move the highlight to it.
    pub fn set_level(&mut self, level: PermissionKind) {
        self.level = level;
        self.index = PermissionView::index_of(level);
    }

    /// The applied level.
    pub fn level(&self) -> PermissionKind {
        self.level
    }

    /// The highlighted row.
    pub fn highlighted(&self) -> PermissionKind {
        PermissionKind::ALL
            .get(self.index)
            .copied()
            .unwrap_or(PermissionKind::All)
    }

    /// Apply the theme used for the rows.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
    }

    /// Whether the enclosing view has the selection in this section.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Move the highlight to the first row (used when the focus enters).
    fn focus_first(&mut self) {
        self.index = 0;
    }

    /// Navigation and the apply key, reporting the same [`SandboxAction`] shape
    /// as [`SandboxView::handle_key`]. `up` at the first row is [`SandboxAction::None`]:
    /// leaving the section is the enclosing view's decision.
    pub fn handle_key(&mut self, key: &str) -> SandboxAction {
        match key {
            "up" | "k" => match self.index.checked_sub(1) {
                Some(index) => {
                    self.index = index;
                    SandboxAction::Moved
                }
                None => SandboxAction::None,
            },
            "down" | "j" => {
                if self.index + 1 >= PermissionKind::ALL.len() {
                    return SandboxAction::None;
                }
                self.index += 1;
                SandboxAction::Moved
            }
            "enter" | " " => SandboxAction::PermissionSelected(self.highlighted()),
            _ => SandboxAction::None,
        }
    }

    /// One row per level. Empty for `width == 0`; every row is exactly `width`
    /// visible columns.
    pub fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        PermissionKind::ALL
            .iter()
            .map(|kind| self.row(*kind, width))
            .collect()
    }

    /// Index of `level` in the picker order.
    fn index_of(level: PermissionKind) -> usize {
        PermissionKind::ALL
            .iter()
            .position(|candidate| *candidate == level)
            .unwrap_or(0)
    }

    fn row(&self, kind: PermissionKind, width: usize) -> String {
        let highlighted = self.focused && self.highlighted() == kind;
        let marker = if highlighted { "›" } else { " " };
        let mut row = format!("{marker} {}", kind.label());
        row.push_str("  ");
        row.push_str(&fg(self.theme.dim as u8, kind.description()));
        if kind == self.level {
            row.push_str("  ");
            row.push_str(&fg(self.theme.dim as u8, "· current"));
        }
        // The description is what makes these rows longer than a terminal:
        // clip the assembled row with a visible ellipsis instead of letting
        // `fit_row` shear it mid-word (`… the grant is scope`).
        let row = truncate_to_width(
            &row,
            width,
            &TruncateOptions {
                ellipsis: true,
                pad: false,
            },
        );
        if highlighted {
            return selected_row(&row, width, &self.theme);
        }
        fg(self.theme.fg as u8, &row)
    }
}

// ─── Sandbox view ──────────────────────────────────────────────────────────

/// Which section of [`SandboxView`] owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Tiers,
    Permissions,
}

/// The sandbox settings screen.
///
/// A flat list of six rows — three tiers followed by three permission levels —
/// with exactly one highlight, so `↑`/`↓` walk the whole screen and `enter`
/// applies whatever is highlighted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxView {
    status: SandboxStatus,
    theme: Theme,
    section: Section,
    tier_index: usize,
    permission: PermissionView,
}

impl SandboxView {
    /// A view for `platform`, with the status the desktop starts from (no RPC
    /// has been sent yet) and the permission picker on the agent's default.
    pub fn new(platform: SandboxPlatform) -> Self {
        let status = SandboxStatus::new(platform);
        let tier_index = SandboxView::index_of(status.tier);
        SandboxView {
            status,
            theme: DARK_THEME,
            section: Section::Tiers,
            tier_index,
            permission: PermissionView::new(PermissionKind::All),
        }
    }

    /// Apply the theme used for every row.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
        self.permission.set_theme(theme);
    }

    /// Replace the status (a `set_sandbox_policy` answer, a probe, or a
    /// `get_state` reload). A highlight left on a row the new status disables
    /// moves to the nearest selectable tier.
    pub fn set_status(&mut self, status: SandboxStatus) {
        self.status = status;
        self.clamp_tier_index();
    }

    /// The current status.
    pub fn status(&self) -> &SandboxStatus {
        &self.status
    }

    /// The permission picker (level, highlight).
    pub fn permission(&self) -> &PermissionView {
        &self.permission
    }

    /// The applied permission level.
    pub fn permission_level(&self) -> PermissionKind {
        self.permission.level()
    }

    /// Set the applied permission level (after `set_permission_level` came back
    /// or a `get_state` reload).
    pub fn set_permission_level(&mut self, level: PermissionKind) {
        self.permission.set_level(level);
    }

    /// The picker rows with their selectability. `sandbox` is enabled only when
    /// the probe reports a usable backend.
    pub fn tier_options(&self) -> Vec<(SandboxTier, bool)> {
        SandboxTier::ALL
            .iter()
            .map(|tier| (*tier, self.tier_enabled(*tier)))
            .collect()
    }

    /// The highlighted tier. Never a disabled one: navigation skips those and
    /// [`SandboxView::set_status`] moves a stale highlight off them.
    pub fn highlighted_tier(&self) -> SandboxTier {
        SandboxTier::ALL
            .get(self.tier_index)
            .copied()
            .unwrap_or(SandboxTier::Manual)
    }

    /// Ask the caller to apply `tier`.
    ///
    /// A disabled tier — `sandbox` without an available backend — answers
    /// [`SandboxAction::None`] and leaves the highlight alone: the request is
    /// not queued, so the caller cannot mistake it for a pending mutation. (The
    /// keyboard cannot reach that row at all; this is the programmatic entry
    /// point.)
    pub fn select_tier(&mut self, tier: SandboxTier) -> SandboxAction {
        if !self.tier_enabled(tier) {
            return SandboxAction::None;
        }
        self.tier_index = SandboxView::index_of(tier);
        SandboxAction::TierSelected(tier)
    }

    /// `up`/`k`, `down`/`j` walk the six rows, `enter`/`space` applies the
    /// highlighted one, `r` asks for a fresh probe and `esc` closes.
    pub fn handle_key(&mut self, key: &str) -> SandboxAction {
        match key {
            "up" | "k" => self.move_up(),
            "down" | "j" => self.move_down(),
            "enter" | " " => self.activate(),
            "r" => SandboxAction::RefreshProbe,
            "esc" => SandboxAction::Cancelled,
            _ => SandboxAction::None,
        }
    }

    /// The panel at `width × height`: never more than `height` rows, each
    /// exactly `width` visible columns. Empty for `width == 0` or `height == 0`.
    pub fn render(&mut self, width: usize, height: usize) -> Vec<String> {
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let mut rows = vec![
            fg(self.theme.accent as u8, &bold(TITLE)),
            String::new(),
            self.heading_row(
                TIER_HEADING,
                &format!("current: {}", tier_label(self.status.tier)),
            ),
        ];
        rows.extend(self.tier_rows(width));
        rows.extend(wrapped_rows(
            tier_description(self.highlighted_tier(), self.status.platform),
            width,
            DESCRIPTION_INDENT,
        ));
        rows.extend(self.platform_rows(width));
        rows.extend(self.availability_rows(width));
        rows.extend(self.fallback_rows(width));
        rows.push(String::new());
        rows.push(self.heading_row(
            PERMISSION_HEADING,
            &format!("current: {}", self.permission.level().label()),
        ));
        rows.extend(self.permission.render(width));
        rows.push(fg(self.theme.dim as u8, &fit_hint_row(KEY_LEGEND, width)));
        rows.truncate(height);
        rows.into_iter().map(|row| fit_row(&row, width)).collect()
    }

    // ── sections ──

    fn move_up(&mut self) -> SandboxAction {
        match self.section {
            Section::Tiers => match self.step(false) {
                Some(index) => {
                    self.tier_index = index;
                    SandboxAction::Moved
                }
                None => SandboxAction::None,
            },
            Section::Permissions => self.move_up_in_permissions(),
        }
    }

    /// `up` inside the permission section: walk its rows, and leave the section
    /// (back into the tier list) from its first row.
    fn move_up_in_permissions(&mut self) -> SandboxAction {
        if self.permission.handle_key("up") == SandboxAction::Moved {
            return SandboxAction::Moved;
        }
        self.leave_permissions();
        SandboxAction::Moved
    }

    fn move_down(&mut self) -> SandboxAction {
        match self.section {
            Section::Tiers => match self.step(true) {
                Some(index) => {
                    self.tier_index = index;
                    SandboxAction::Moved
                }
                None => {
                    self.enter_permissions();
                    SandboxAction::Moved
                }
            },
            Section::Permissions => self.permission.handle_key("down"),
        }
    }

    fn activate(&mut self) -> SandboxAction {
        match self.section {
            Section::Tiers => self.select_tier(self.highlighted_tier()),
            Section::Permissions => self.permission.handle_key("enter"),
        }
    }

    /// Move the selection into the permission section (first row).
    fn enter_permissions(&mut self) {
        self.section = Section::Permissions;
        self.permission.focus_first();
        self.permission.set_focused(true);
    }

    /// Move the selection back up into the tier section, landing on its last
    /// selectable row so `↓` returns to where it left.
    fn leave_permissions(&mut self) {
        self.section = Section::Tiers;
        self.permission.set_focused(false);
        self.tier_index = self.last_enabled_tier();
    }

    // ── rows ──

    fn heading_row(&self, title: &str, hint: &str) -> String {
        format!(
            "{}  {}",
            fg(self.theme.accent as u8, &bold(title)),
            fg(self.theme.dim as u8, hint)
        )
    }

    fn tier_rows(&self, width: usize) -> Vec<String> {
        self.tier_options()
            .into_iter()
            .enumerate()
            .map(|(index, (tier, enabled))| self.tier_row(index, tier, enabled, width))
            .collect()
    }

    fn tier_row(&self, index: usize, tier: SandboxTier, enabled: bool, width: usize) -> String {
        let focused = self.section == Section::Tiers && index == self.tier_index;
        let marker = if focused { "›" } else { " " };
        let mut row = format!("{marker} {}", tier_label(tier));
        let suffix = self.tier_suffix(tier);
        if !suffix.is_empty() {
            row.push(' ');
            row.push_str(&suffix);
        }
        if tier == self.status.tier {
            row.push_str("  ");
            row.push_str(&fg(self.theme.dim as u8, "· current"));
        }
        if !enabled {
            return fg(self.theme.dim as u8, &row);
        }
        if focused {
            return selected_row(&row, width, &self.theme);
        }
        fg(self.theme.fg as u8, &row)
    }

    /// The `(checking)` / `(unavailable)` tag on the `sandbox` row, empty while
    /// the backend is usable (the desktop prints a plain "Sandboxed" then).
    fn tier_suffix(&self, tier: SandboxTier) -> String {
        match tier {
            SandboxTier::Sandbox => match self.status.probe_state() {
                ProbeState::Available => String::new(),
                ProbeState::Checking => fg(self.theme.dim as u8, "(checking)"),
                ProbeState::Unavailable => fg(self.theme.error as u8, "(unavailable)"),
            },
            SandboxTier::Off | SandboxTier::Manual => String::new(),
        }
    }

    fn platform_rows(&self, width: usize) -> Vec<String> {
        match platform_note(self.status.platform) {
            Some(note) => wrapped_rows(note, width, DESCRIPTION_INDENT)
                .into_iter()
                .map(|row| fg(self.theme.dim as u8, &row))
                .collect(),
            None => Vec::new(),
        }
    }

    fn availability_rows(&self, width: usize) -> Vec<String> {
        let probe = &self.status.probe;
        let (colour, text) = match probe.state() {
            ProbeState::Available => {
                let detail = probe.detail();
                let line = if detail.is_empty() {
                    "Sandbox: available".to_string()
                } else {
                    format!("Sandbox: available — {detail}")
                };
                (self.theme.success as u8, line)
            }
            ProbeState::Checking => (self.theme.dim as u8, AVAILABILITY_CHECKING.to_string()),
            ProbeState::Unavailable => (
                self.theme.error as u8,
                format!(
                    "Sandbox: unavailable — {} (diagnostic: {})",
                    probe.reason(),
                    probe.diagnostic_code()
                ),
            ),
        };
        wrapped_rows(&text, width, DESCRIPTION_INDENT)
            .into_iter()
            .map(|row| fg(colour, &row))
            .collect()
    }

    fn fallback_rows(&self, width: usize) -> Vec<String> {
        match self.status.fallback_notice() {
            Some(notice) => wrapped_rows(&format!("! {notice}"), width, 0)
                .into_iter()
                .map(|row| fg(self.theme.error as u8, &row))
                .collect(),
            None => Vec::new(),
        }
    }

    // ── tier selection ──

    /// `sandbox` needs an available backend; the other two tiers are always
    /// selectable.
    fn tier_enabled(&self, tier: SandboxTier) -> bool {
        match tier {
            SandboxTier::Sandbox => self.status.probe.available,
            SandboxTier::Off | SandboxTier::Manual => true,
        }
    }

    /// Index of `tier` in the picker order.
    fn index_of(tier: SandboxTier) -> usize {
        SandboxTier::ALL
            .iter()
            .position(|candidate| *candidate == tier)
            .unwrap_or(0)
    }

    /// The nearest selectable row in `direction`, skipping disabled tiers.
    fn step(&self, forward: bool) -> Option<usize> {
        let options = self.tier_options();
        let candidates: Vec<usize> = if forward {
            (self.tier_index + 1..options.len()).collect()
        } else {
            (0..self.tier_index).rev().collect()
        };
        candidates.into_iter().find(|index| {
            options
                .get(*index)
                .map(|(_, enabled)| *enabled)
                .unwrap_or(false)
        })
    }

    /// Index of the last selectable row (where `↑` lands when the selection
    /// comes back from the permission section).
    fn last_enabled_tier(&self) -> usize {
        self.tier_options()
            .iter()
            .rposition(|(_, enabled)| *enabled)
            .unwrap_or(0)
    }

    /// Keep the highlight on a selectable row after the status changed.
    fn clamp_tier_index(&mut self) {
        let options = self.tier_options();
        let valid = options
            .get(self.tier_index)
            .map(|(_, enabled)| *enabled)
            .unwrap_or(false);
        if valid {
            return;
        }
        self.tier_index = options
            .iter()
            .position(|(_, enabled)| *enabled)
            .unwrap_or(0);
    }
}

// ─── Actions ───────────────────────────────────────────────────────────────

/// What a key press asks the caller to do. Nothing here performs I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxAction {
    /// The key did nothing (or a nudge against a boundary).
    None,
    /// The highlight moved — redraw.
    Moved,
    /// Apply this approval tier (`set_sandbox_policy`).
    TierSelected(SandboxTier),
    /// Apply this permission level (`set_permission_level`).
    PermissionSelected(PermissionKind),
    /// Close the screen.
    Cancelled,
    /// Re-run `probe_sandbox`.
    RefreshProbe,
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Indent of wrapped prose rows (descriptions, notes, the fallback banner).
const DESCRIPTION_INDENT: usize = 2;

/// First non-empty string among `keys` (camelCase first, snake_case second).
fn field_text(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    let raw = keys.iter().find_map(|key| map.get(*key)?.as_str())?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// A boolean field, ignoring anything that is not a boolean.
fn field_bool(map: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_bool))
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

/// `content` on the theme's selection bar, exactly `width` columns wide.
///
/// The content is truncated *before* the background is applied, so a narrow
/// terminal can never cut an escape sequence in half.
fn selected_row(content: &str, width: usize, theme: &Theme) -> String {
    let styled = fg(theme.selected_fg as u8, content);
    let clipped = truncate_to_width(&styled, width, &TruncateOptions::default());
    apply_background_to_line(&clipped, width, theme.selected_bg)
}

/// A hint row (the key legend): like [`fit_row`], except a row too wide for the
/// pane ends in an ellipsis so the cut is visible (`esc closes` → `esc close…`)
/// rather than looking like a typo. The ellipsis takes the last column, so the
/// row still spans exactly `width`.
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

/// Wrap `text` into rows of at most `width` columns, each indented by `indent`.
///
/// Returns nothing when the indent leaves no room for text, so a one-column
/// terminal simply drops the prose instead of panicking.
fn wrapped_rows(text: &str, width: usize, indent: usize) -> Vec<String> {
    let inner = width.saturating_sub(indent);
    if inner == 0 {
        return Vec::new();
    }
    let pad = " ".repeat(indent);
    wrap_text_with_ansi(text, inner)
        .into_iter()
        .map(|line| format!("{pad}{line}"))
        .collect()
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;
    use serde_json::json;

    /// Every rendered row, ANSI stripped and padding removed.
    fn lines(view: &mut SandboxView, width: usize) -> Vec<String> {
        view.render(width, 200)
            .iter()
            .map(|row| strip_ansi_codes(row).trim_end().to_string())
            .collect()
    }

    /// The rendered panel as text, asserted against with `contains`.
    fn text(view: &mut SandboxView, width: usize) -> String {
        lines(view, width).join("\n")
    }

    /// The status the agent answers with when it refuses to sandbox.
    fn downgrade() -> Value {
        json!({
            "tier": "manual",
            "requestedTier": "sandbox",
            "sandboxAvailable": false,
            "sandboxCode": "binary_missing",
            "sandboxBackend": "none"
        })
    }

    /// A Linux view that just learned its sandbox request was downgraded.
    fn downgraded_view(platform: SandboxPlatform) -> SandboxView {
        let mut view = SandboxView::new(platform);
        view.set_status(SandboxStatus::from_policy_response(platform, &downgrade()));
        view
    }

    // ── tier ──

    #[test]
    fn tier_wire_round_trips_and_rejects_unknown_values() {
        assert_eq!(SandboxTier::Off.to_wire(), "off");
        assert_eq!(SandboxTier::Manual.to_wire(), "manual");
        assert_eq!(SandboxTier::Sandbox.to_wire(), "sandbox");
        for tier in SandboxTier::ALL {
            assert_eq!(SandboxTier::from_wire(tier.to_wire()), Some(tier));
        }
        assert_eq!(SandboxTier::from_wire("SAND-BOX"), None);
        assert_eq!(SandboxTier::from_wire(""), None);
        assert_eq!(
            SandboxTier::from_wire(" sandbox "),
            Some(SandboxTier::Sandbox)
        );
        assert_eq!(SandboxTier::from_wire("MANUAL"), Some(SandboxTier::Manual));
    }

    #[test]
    fn tier_labels_and_order_match_the_desktop_picker() {
        assert_eq!(
            SandboxTier::ALL,
            [SandboxTier::Manual, SandboxTier::Sandbox, SandboxTier::Off]
        );
        assert_eq!(tier_label(SandboxTier::Manual), "Manual");
        assert_eq!(tier_label(SandboxTier::Sandbox), "Sandboxed");
        assert_eq!(tier_label(SandboxTier::Off), "Unrestricted");
    }

    #[test]
    fn tier_descriptions_cover_every_tier_and_platform() {
        assert!(tier_description(SandboxTier::Off, SandboxPlatform::Macos).contains("No prompts"));
        assert!(
            tier_description(SandboxTier::Manual, SandboxPlatform::Macos)
                .contains("approval rules")
        );
        let macos = tier_description(SandboxTier::Sandbox, SandboxPlatform::Macos);
        let linux = tier_description(SandboxTier::Sandbox, SandboxPlatform::Linux);
        let windows = tier_description(SandboxTier::Sandbox, SandboxPlatform::Windows);
        let other = tier_description(SandboxTier::Sandbox, SandboxPlatform::Other);
        assert!(macos.contains("macOS sandbox"));
        assert!(linux.contains("Linux sandbox"));
        assert!(windows.contains("Windows write protection"));
        assert!(other.contains("no sandbox backend"));
        assert_ne!(macos, linux);
        assert_ne!(linux, windows);
        assert_ne!(windows, other);
    }

    // ── platform ──

    #[test]
    fn platform_parses_the_rust_os_names() {
        assert_eq!(SandboxPlatform::from_os("macos"), SandboxPlatform::Macos);
        assert_eq!(SandboxPlatform::from_os("linux"), SandboxPlatform::Linux);
        assert_eq!(
            SandboxPlatform::from_os("windows"),
            SandboxPlatform::Windows
        );
        assert_eq!(SandboxPlatform::from_os("freebsd"), SandboxPlatform::Other);
        assert_eq!(SandboxPlatform::from_os(""), SandboxPlatform::Other);
        assert_eq!(
            SandboxPlatform::current(),
            SandboxPlatform::from_os(std::env::consts::OS)
        );
    }

    #[test]
    fn only_macos_has_a_native_sandbox() {
        assert!(supports_sandbox(SandboxPlatform::Macos));
        assert!(!supports_sandbox(SandboxPlatform::Linux));
        assert!(!supports_sandbox(SandboxPlatform::Windows));
        assert!(!supports_sandbox(SandboxPlatform::Other));
    }

    #[test]
    fn the_default_tier_is_sandboxed_only_on_macos() {
        assert_eq!(
            platform_default_tier(SandboxPlatform::Macos),
            SandboxTier::Sandbox
        );
        assert_eq!(
            platform_default_tier(SandboxPlatform::Linux),
            SandboxTier::Manual
        );
        assert_eq!(
            platform_default_tier(SandboxPlatform::Windows),
            SandboxTier::Manual
        );
        assert_eq!(
            platform_default_tier(SandboxPlatform::Other),
            SandboxTier::Manual
        );
    }

    #[test]
    fn platform_notes_describe_the_alternatives() {
        assert_eq!(platform_note(SandboxPlatform::Macos), None);
        let linux = platform_note(SandboxPlatform::Linux).expect("Linux has a note");
        assert!(linux.contains("Bubblewrap"));
        assert!(linux.contains("macOS"));
        let windows = platform_note(SandboxPlatform::Windows).expect("Windows has a note");
        assert!(windows.contains("write protection"));
        let other = platform_note(SandboxPlatform::Other).expect("Other has a note");
        assert!(other.contains("No sandbox backend"));
    }

    // ── diagnostics ──

    #[test]
    fn every_reason_code_has_its_own_explanation() {
        assert_eq!(REASON_CODES.len(), 12);
        let mut seen: Vec<&'static str> = Vec::new();
        for code in REASON_CODES {
            let text = reason_text(Some(code));
            assert!(!text.is_empty());
            assert_ne!(text, REASON_UNKNOWN);
            assert_ne!(text, REASON_PLATFORM_UNSUPPORTED);
            assert_ne!(text, REASON_AVAILABLE);
            assert!(!seen.contains(&text));
            seen.push(text);
        }
        assert_eq!(seen.len(), REASON_CODES.len());
    }

    #[test]
    fn missing_and_unknown_codes_fall_back_without_panicking() {
        assert_eq!(reason_text(None), REASON_UNKNOWN);
        assert_eq!(reason_text(Some("")), REASON_UNKNOWN);
        assert_eq!(reason_text(Some("not_a_code")), REASON_UNKNOWN);
        assert_eq!(reason_text(Some("probe_timeout ")), REASON_UNKNOWN);
        assert_eq!(
            reason_text(Some("platform_unsupported")),
            REASON_PLATFORM_UNSUPPORTED
        );
        assert_eq!(reason_text(Some("available")), REASON_AVAILABLE);
    }

    // ── probe ──

    #[test]
    fn probe_constructors_report_their_three_states() {
        let checking = SandboxProbe::checking();
        assert_eq!(checking.state(), ProbeState::Checking);
        assert!(!checking.available);
        assert!(!checking.resolved);
        assert!(!checking.definitive);

        let available = SandboxProbe::available("macos_seatbelt");
        assert_eq!(available.state(), ProbeState::Available);
        assert!(available.resolved && available.definitive);
        assert_eq!(available.diagnostic_code(), "available");
        assert_eq!(available.reason(), REASON_AVAILABLE);

        let unavailable = SandboxProbe::unavailable("user_namespace_disabled", "bubblewrap");
        assert_eq!(unavailable.state(), ProbeState::Unavailable);
        assert_eq!(unavailable.diagnostic_code(), "user_namespace_disabled");
        assert_eq!(
            unavailable.reason(),
            reason_text(Some("user_namespace_disabled"))
        );

        // `available` without a finished probe is still "checking".
        let half = SandboxProbe {
            available: true,
            ..SandboxProbe::checking()
        };
        assert_eq!(half.state(), ProbeState::Checking);
    }

    #[test]
    fn probe_detail_lists_only_what_the_probe_reported() {
        assert_eq!(SandboxProbe::checking().detail(), "");
        assert_eq!(
            SandboxProbe::available("bubblewrap").detail(),
            "backend bubblewrap"
        );
        let full = SandboxProbe {
            path: Some("/usr/bin/bwrap".to_string()),
            version: Some("0.9.0".to_string()),
            ..SandboxProbe::available("bubblewrap")
        };
        assert_eq!(full.detail(), "backend bubblewrap · /usr/bin/bwrap · 0.9.0");
        let no_backend = SandboxProbe {
            backend: None,
            path: Some("/usr/bin/bwrap".to_string()),
            ..SandboxProbe::available("bubblewrap")
        };
        assert_eq!(no_backend.detail(), "/usr/bin/bwrap");
    }

    #[test]
    fn probe_parses_a_typed_response() {
        let probe = SandboxProbe::from_probe_response(&json!({
            "available": false,
            "resolved": true,
            "definitive": true,
            "code": "binary_invalid",
            "backend": "bubblewrap",
            "path": "/usr/bin/bwrap",
            "version": "0.9.0"
        }));
        assert_eq!(probe.state(), ProbeState::Unavailable);
        assert!(probe.definitive);
        assert_eq!(probe.diagnostic_code(), "binary_invalid");
        assert_eq!(probe.path.as_deref(), Some("/usr/bin/bwrap"));
        assert_eq!(probe.version.as_deref(), Some("0.9.0"));
        assert!(probe.reason().contains("not a valid executable"));

        // A successful RPC is authoritative about the two flags it omits.
        let sparse = SandboxProbe::from_probe_response(&json!({ "available": true }));
        assert_eq!(sparse.state(), ProbeState::Available);
        assert!(sparse.resolved && sparse.definitive);
        assert_eq!(sparse.diagnostic_code(), CODE_FALLBACK);

        assert_eq!(
            SandboxProbe::from_probe_response(&json!(null)),
            SandboxProbe::checking()
        );
        assert_eq!(
            SandboxProbe::from_probe_response(&json!("nope")),
            SandboxProbe::checking()
        );
    }

    // ── status ──

    #[test]
    fn a_fresh_status_mirrors_the_desktop_initial_state() {
        let macos = SandboxStatus::new(SandboxPlatform::Macos);
        assert_eq!(macos.tier, SandboxTier::Sandbox);
        assert_eq!(macos.probe_state(), ProbeState::Available);
        assert_eq!(macos.probe.backend.as_deref(), Some("macos_seatbelt"));
        assert_eq!(macos.probe.code.as_deref(), Some("available"));
        assert!(macos.probe.definitive);
        assert_eq!(macos.requested_tier, None);
        assert!(!macos.is_fallback());
        assert_eq!(macos.fallback_notice(), None);

        let linux = SandboxStatus::new(SandboxPlatform::Linux);
        assert_eq!(linux.tier, SandboxTier::Manual);
        assert_eq!(linux.probe_state(), ProbeState::Checking);
        assert_eq!(
            SandboxStatus::new(SandboxPlatform::Windows).probe_state(),
            ProbeState::Checking
        );

        let other = SandboxStatus::new(SandboxPlatform::Other);
        assert_eq!(other.tier, SandboxTier::Manual);
        assert_eq!(other.probe_state(), ProbeState::Unavailable);
        assert_eq!(other.probe.diagnostic_code(), "platform_unsupported");
        assert_eq!(other.probe.backend.as_deref(), Some("none"));
        assert_eq!(other.probe.reason(), REASON_PLATFORM_UNSUPPORTED);
    }

    #[test]
    fn the_agent_downgrade_is_reported_as_a_fallback() {
        let status = SandboxStatus::from_policy_response(SandboxPlatform::Linux, &downgrade());
        assert_eq!(status.tier, SandboxTier::Manual);
        assert_eq!(status.requested_tier, Some(SandboxTier::Sandbox));
        assert_eq!(status.probe_state(), ProbeState::Unavailable);
        assert_eq!(status.probe.diagnostic_code(), "binary_missing");
        assert!(status.is_fallback());
        let notice = status.fallback_notice().expect("a downgrade has a notice");
        assert!(notice.contains("Sandbox requested but unavailable"));
        assert!(notice.contains("fell back to Manual"));
        assert!(notice.contains("binary_missing"));
    }

    #[test]
    fn an_applied_sandbox_request_has_no_fallback_notice() {
        let status = SandboxStatus::from_policy_response(
            SandboxPlatform::Macos,
            &json!({
                "tier": "sandbox",
                "requestedTier": "sandbox",
                "sandboxAvailable": true,
                "sandboxCode": "available",
                "sandboxBackend": "macos_seatbelt"
            }),
        );
        assert_eq!(status.tier, SandboxTier::Sandbox);
        assert!(!status.is_fallback());
        assert_eq!(status.fallback_notice(), None);
        assert_eq!(status.probe_state(), ProbeState::Available);
    }

    #[test]
    fn a_policy_response_without_a_probe_stays_unresolved() {
        let status = SandboxStatus::from_policy_response(
            SandboxPlatform::Linux,
            &json!({ "tier": "manual", "requestedTier": "manual" }),
        );
        assert_eq!(status.probe_state(), ProbeState::Checking);
        assert_eq!(status.probe.code, None);
        assert!(!status.is_fallback());
    }

    #[test]
    fn a_malformed_policy_response_never_lies_about_the_state() {
        let fallback = SandboxStatus::new(SandboxPlatform::Linux);
        assert_eq!(
            SandboxStatus::from_policy_response(SandboxPlatform::Linux, &json!([1, 2])),
            fallback
        );
        // An unknown tier falls back to the platform default, like the agent.
        let bogus =
            SandboxStatus::from_policy_response(SandboxPlatform::Linux, &json!({ "tier": "??" }));
        assert_eq!(bogus.tier, SandboxTier::Manual);
        let bogus_macos =
            SandboxStatus::from_policy_response(SandboxPlatform::Macos, &json!({ "tier": "??" }));
        assert_eq!(bogus_macos.tier, SandboxTier::Sandbox);
        // snake_case keys and blank strings are tolerated.
        let snake = SandboxStatus::from_policy_response(
            SandboxPlatform::Linux,
            &json!({
                "tier": "manual",
                "requested_tier": "sandbox",
                "sandbox_available": false,
                "sandbox_code": "  ",
                "sandbox_backend": "none"
            }),
        );
        assert_eq!(snake.requested_tier, Some(SandboxTier::Sandbox));
        assert_eq!(snake.probe_state(), ProbeState::Unavailable);
        assert_eq!(snake.probe.code, None);
        let notice = snake.fallback_notice().expect("a downgrade has a notice");
        assert!(notice.contains("probe_failed"));
    }

    // ── permission level ──

    #[test]
    fn permission_kind_wire_values_and_copy() {
        assert_eq!(
            PermissionKind::ALL,
            [
                PermissionKind::All,
                PermissionKind::Workspace,
                PermissionKind::None
            ]
        );
        for kind in PermissionKind::ALL {
            assert_eq!(PermissionKind::from_wire(kind.to_wire()), Some(kind));
            assert!(!kind.label().is_empty());
            assert!(!kind.description().is_empty());
        }
        assert_eq!(
            PermissionKind::from_wire("NONE"),
            Some(PermissionKind::None)
        );
        assert_eq!(
            PermissionKind::from_wire(" workspace "),
            Some(PermissionKind::Workspace)
        );
        assert_eq!(PermissionKind::from_wire("auto"), None);
        assert_eq!(PermissionKind::All.label(), "All");
        assert_eq!(PermissionKind::Workspace.label(), "Workspace");
        assert_eq!(PermissionKind::None.label(), "None");
        assert!(PermissionKind::All.description().contains("without asking"));
        assert!(PermissionKind::Workspace
            .description()
            .contains("scoped to this workspace"));
        assert!(PermissionKind::None.description().contains("denied"));
    }

    #[test]
    fn permission_view_navigates_and_applies() {
        let mut view = PermissionView::new(PermissionKind::Workspace);
        assert_eq!(view.level(), PermissionKind::Workspace);
        assert_eq!(view.highlighted(), PermissionKind::Workspace);
        assert_eq!(view.handle_key("up"), SandboxAction::Moved);
        assert_eq!(view.highlighted(), PermissionKind::All);
        assert_eq!(view.handle_key("k"), SandboxAction::None);
        assert_eq!(view.handle_key("down"), SandboxAction::Moved);
        assert_eq!(view.handle_key("j"), SandboxAction::Moved);
        assert_eq!(view.highlighted(), PermissionKind::None);
        assert_eq!(view.handle_key("j"), SandboxAction::None);
        assert_eq!(
            view.handle_key("enter"),
            SandboxAction::PermissionSelected(PermissionKind::None)
        );
        assert_eq!(
            view.handle_key(" "),
            SandboxAction::PermissionSelected(PermissionKind::None)
        );
        assert_eq!(view.handle_key("x"), SandboxAction::None);
        assert_eq!(view.level(), PermissionKind::Workspace);
    }

    #[test]
    fn permission_view_level_and_focus_are_settable() {
        let mut view = PermissionView::new(PermissionKind::All);
        view.set_level(PermissionKind::None);
        assert_eq!(view.level(), PermissionKind::None);
        assert_eq!(view.highlighted(), PermissionKind::None);
        view.set_focused(true);
        view.focus_first();
        assert_eq!(view.highlighted(), PermissionKind::All);
    }

    #[test]
    fn permission_view_renders_three_rows_that_fit_the_width() {
        let mut view = PermissionView::new(PermissionKind::Workspace);
        let rows: Vec<String> = view
            .render(200)
            .iter()
            .map(|row| strip_ansi_codes(row).trim_end().to_string())
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], "  All  Every tool call runs without asking.");
        assert!(rows[1].starts_with("  Workspace  Tool calls ask for approval"));
        assert!(rows[1].ends_with("· current"));
        assert_eq!(rows[2], "  None  Every tool call is denied before it runs.");
        assert!(view.render(0).is_empty());

        view.set_focused(true);
        let focused = view.render(200);
        assert!(strip_ansi_codes(&focused[1]).starts_with("›"));
        assert!(focused[1].contains("\x1b[48;5;38m"));
        assert!(focused[1].contains("\x1b[38;5;255m"));
        assert!(strip_ansi_codes(&focused[0]).starts_with("  All"));
    }

    /// The permission rows carry a sentence, so on a normal 80-column pane they
    /// are longer than the row: the clip has to be *visible* (`… the gra…`)
    /// instead of shearing the sentence mid-word (`… the grant is scope`, which
    /// reads as a typo). Both the plain and the highlighted row are clipped the
    /// same way, and both still fill the pane exactly.
    #[test]
    fn narrow_permission_rows_end_in_an_ellipsis() {
        let mut view = PermissionView::new(PermissionKind::Workspace);
        let rows: Vec<String> = view
            .render(60)
            .iter()
            .map(|row| strip_ansi_codes(row).trim_end().to_string())
            .collect();
        assert_eq!(rows[0], "  All  Every tool call runs without asking.");
        assert_eq!(
            rows[1],
            "  Workspace  Tool calls ask for approval before they run; t…"
        );
        // The `· current` marker is the first thing to go, and the highlight
        // still spans the full width.
        assert_eq!(rows[2], "  None  Every tool call is denied before it runs.");
        view.set_focused(true);
        let focused = view.render(60);
        assert_eq!(visible_width(&focused[1]), 60);
        assert_eq!(
            strip_ansi_codes(&focused[1]).trim_end(),
            "› Workspace  Tool calls ask for approval before they run; t…"
        );
        assert!(focused[1].contains("\x1b[48;5;38m"));
    }

    // ── view: options and selection ──

    #[test]
    fn a_fresh_view_starts_on_the_platform_default() {
        let macos = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(macos.highlighted_tier(), SandboxTier::Sandbox);
        assert_eq!(macos.status().tier, SandboxTier::Sandbox);
        assert_eq!(macos.permission_level(), PermissionKind::All);

        let linux = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(linux.highlighted_tier(), SandboxTier::Manual);
        assert_eq!(linux.tier_options()[1], (SandboxTier::Sandbox, false));
    }

    #[test]
    fn the_sandbox_row_needs_an_available_backend() {
        let macos = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(
            macos.tier_options(),
            vec![
                (SandboxTier::Manual, true),
                (SandboxTier::Sandbox, true),
                (SandboxTier::Off, true)
            ]
        );

        let mut linux = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(
            linux.tier_options(),
            vec![
                (SandboxTier::Manual, true),
                (SandboxTier::Sandbox, false),
                (SandboxTier::Off, true)
            ]
        );

        // The gate is availability, not the platform: a working Bubblewrap
        // makes the tier selectable on Linux too.
        linux.set_status(SandboxStatus {
            tier: SandboxTier::Manual,
            requested_tier: None,
            probe: SandboxProbe::available("bubblewrap"),
            platform: SandboxPlatform::Linux,
        });
        assert_eq!(linux.tier_options()[1], (SandboxTier::Sandbox, true));
        assert_eq!(
            linux.handle_key("enter"),
            SandboxAction::TierSelected(SandboxTier::Manual)
        );
    }

    #[test]
    fn navigation_skips_the_disabled_sandbox_row() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(view.handle_key("down"), SandboxAction::Moved);
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
        assert_eq!(view.tier_options()[1], (SandboxTier::Sandbox, false));
        assert_eq!(view.handle_key("up"), SandboxAction::Moved);
        assert_eq!(view.highlighted_tier(), SandboxTier::Manual);
        assert_eq!(view.handle_key("up"), SandboxAction::None);
        assert_eq!(view.highlighted_tier(), SandboxTier::Manual);
        assert_eq!(view.handle_key("j"), SandboxAction::Moved);
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
        assert_eq!(view.handle_key("k"), SandboxAction::Moved);
        assert_eq!(view.highlighted_tier(), SandboxTier::Manual);
    }

    #[test]
    fn selecting_a_disabled_tier_does_nothing() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(view.select_tier(SandboxTier::Sandbox), SandboxAction::None);
        assert_eq!(view.highlighted_tier(), SandboxTier::Manual);
        assert_eq!(
            view.select_tier(SandboxTier::Off),
            SandboxAction::TierSelected(SandboxTier::Off)
        );
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);

        let mut macos = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(
            macos.select_tier(SandboxTier::Sandbox),
            SandboxAction::TierSelected(SandboxTier::Sandbox)
        );
        assert_eq!(macos.highlighted_tier(), SandboxTier::Sandbox);
    }

    #[test]
    fn enter_applies_whatever_is_highlighted() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(
            view.handle_key("enter"),
            SandboxAction::TierSelected(SandboxTier::Manual)
        );
        assert_eq!(view.handle_key("down"), SandboxAction::Moved);
        assert_eq!(
            view.handle_key(" "),
            SandboxAction::TierSelected(SandboxTier::Off)
        );
    }

    #[test]
    fn global_keys_work_from_both_sections() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(view.handle_key("r"), SandboxAction::RefreshProbe);
        assert_eq!(view.handle_key("esc"), SandboxAction::Cancelled);
        assert_eq!(view.handle_key("q"), SandboxAction::None);
        view.handle_key("down");
        view.handle_key("down");
        view.handle_key("down");
        assert_eq!(view.handle_key("r"), SandboxAction::RefreshProbe);
        assert_eq!(view.handle_key("esc"), SandboxAction::Cancelled);
    }

    #[test]
    fn the_selection_walks_into_and_out_of_the_permission_section() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(view.handle_key("down"), SandboxAction::Moved); // Off
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
        assert_eq!(view.handle_key("down"), SandboxAction::Moved); // into the permissions
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
        assert_eq!(
            view.handle_key("enter"),
            SandboxAction::PermissionSelected(PermissionKind::All)
        );
        assert_eq!(view.handle_key("down"), SandboxAction::Moved); // Workspace
        assert_eq!(
            view.handle_key("enter"),
            SandboxAction::PermissionSelected(PermissionKind::Workspace)
        );
        assert_eq!(view.handle_key("down"), SandboxAction::Moved); // None
        assert_eq!(view.handle_key("down"), SandboxAction::None);
        assert_eq!(
            view.handle_key("enter"),
            SandboxAction::PermissionSelected(PermissionKind::None)
        );
        assert_eq!(view.handle_key("k"), SandboxAction::Moved);
        assert_eq!(view.handle_key("k"), SandboxAction::Moved);
        assert_eq!(view.handle_key("k"), SandboxAction::Moved); // back to the tiers
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
        assert_eq!(view.handle_key("k"), SandboxAction::Moved);
        assert_eq!(view.highlighted_tier(), SandboxTier::Sandbox);
        assert_eq!(view.handle_key("k"), SandboxAction::Moved);
        assert_eq!(view.highlighted_tier(), SandboxTier::Manual);
        assert_eq!(view.handle_key("k"), SandboxAction::None);
    }

    /// Rows carrying the focus marker.
    fn marked(view: &mut SandboxView) -> usize {
        lines(view, 200)
            .iter()
            .filter(|row| row.contains('›'))
            .count()
    }

    #[test]
    fn only_the_focused_section_draws_a_marker() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(marked(&mut view), 1);
        assert!(lines(&mut view, 200)[4].starts_with("› Sandboxed"));
        view.handle_key("down");
        view.handle_key("down");
        assert_eq!(marked(&mut view), 1);
        assert!(lines(&mut view, 200)
            .iter()
            .any(|row| row.starts_with("› All")));
        assert!(!lines(&mut view, 200)
            .iter()
            .any(|row| row.starts_with("› Sandboxed")));
        view.handle_key("down");
        assert!(lines(&mut view, 200)
            .iter()
            .any(|row| row.starts_with("› Workspace")));
    }

    #[test]
    fn a_stale_highlight_moves_off_a_row_the_new_status_disables() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(view.highlighted_tier(), SandboxTier::Sandbox);
        view.set_status(SandboxStatus::new(SandboxPlatform::Linux));
        assert_eq!(view.highlighted_tier(), SandboxTier::Manual);
        assert_eq!(view.status().platform, SandboxPlatform::Linux);
        // A still-valid highlight is left alone.
        view.handle_key("down");
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
        view.set_status(SandboxStatus::new(SandboxPlatform::Linux));
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
        // The selection can sit in the permission section while the status
        // changes underneath it.
        view.set_status(SandboxStatus::new(SandboxPlatform::Macos));
        assert_eq!(view.highlighted_tier(), SandboxTier::Off);
    }

    #[test]
    fn permission_level_accessors_round_trip() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(view.permission().level(), PermissionKind::All);
        view.set_permission_level(PermissionKind::None);
        assert_eq!(view.permission_level(), PermissionKind::None);
        assert!(text(&mut view, 200).contains("Tool permissions  current: None"));
    }

    // ── rendering ──

    #[test]
    fn the_panel_renders_its_chrome_in_order() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        let rows = lines(&mut view, 200);
        assert_eq!(rows.len(), 14);
        assert_eq!(rows[0], TITLE);
        assert_eq!(rows[1], "");
        assert_eq!(rows[2], "Approval mode  current: Sandboxed");
        assert_eq!(rows[3], "  Manual");
        assert_eq!(rows[4], "› Sandboxed  · current");
        assert_eq!(rows[5], "  Unrestricted");
        assert_eq!(
            rows[6],
            "  Commands run in the macOS sandbox and ask for approval when needed."
        );
        assert_eq!(rows[7], "  Sandbox: available — backend macos_seatbelt");
        assert_eq!(rows[8], "");
        assert_eq!(rows[9], "Tool permissions  current: All");
        assert_eq!(
            rows[10],
            "  All  Every tool call runs without asking.  · current"
        );
        assert!(rows[11].starts_with("  Workspace  Tool calls ask for approval"));
        assert_eq!(
            rows[12],
            "  None  Every tool call is denied before it runs."
        );
        assert_eq!(rows[13], KEY_LEGEND);
    }

    /// The key legend is a hint row like the menus': on a pane too narrow for
    /// it the cut ends in an ellipsis, and the row still fills the pane.
    #[test]
    fn the_key_legend_is_ellipsized_on_a_narrow_pane() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        let rows = view.render(40, 60);
        let last = rows.last().unwrap().clone();
        assert_eq!(visible_width(&last), 40);
        assert_eq!(
            strip_ansi_codes(&last).trim_end(),
            "↑/↓ move · enter applies · r re-checks …"
        );
        // A pane with room for it keeps every word.
        let wide = view.render(120, 60).pop().unwrap();
        assert_eq!(visible_width(&wide), 120);
        assert_eq!(strip_ansi_codes(&wide).trim_end(), KEY_LEGEND);
    }

    #[test]
    fn checking_and_unavailable_read_differently() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        let waiting = text(&mut view, 200);
        assert!(waiting.contains("Sandboxed (checking)"));
        assert!(waiting.contains("Sandbox: checking the system sandbox…"));
        assert!(!waiting.contains("unavailable"));

        view.set_status(SandboxStatus {
            tier: SandboxTier::Manual,
            requested_tier: None,
            probe: SandboxProbe::unavailable("user_namespace_disabled", "bubblewrap"),
            platform: SandboxPlatform::Linux,
        });
        let failed = text(&mut view, 200);
        assert!(!failed.contains("checking the system sandbox"));
        assert!(failed.contains("(unavailable)"));
        assert!(
            failed.contains("Sandbox: unavailable — Unprivileged user namespaces are disabled.")
        );
        assert!(failed.contains("(diagnostic: user_namespace_disabled)"));
    }

    #[test]
    fn an_available_probe_reports_its_backend_details() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        view.set_status(SandboxStatus {
            tier: SandboxTier::Manual,
            requested_tier: None,
            probe: SandboxProbe {
                available: true,
                resolved: true,
                definitive: true,
                path: Some("/usr/bin/bwrap".to_string()),
                version: Some("0.9.0".to_string()),
                ..SandboxProbe::available("bubblewrap")
            },
            platform: SandboxPlatform::Linux,
        });
        let rows = lines(&mut view, 200);
        assert!(
            rows.iter()
                .any(|row| row
                    == "  Sandbox: available — backend bubblewrap · /usr/bin/bwrap · 0.9.0")
        );
        assert!(rows.iter().any(|row| row == "  Sandboxed"));
        assert!(!rows.iter().any(|row| row.contains("(checking)")));

        // No details at all still reads as available.
        let mut bare = SandboxView::new(SandboxPlatform::Linux);
        bare.set_status(SandboxStatus {
            tier: SandboxTier::Manual,
            requested_tier: None,
            probe: SandboxProbe {
                available: true,
                resolved: true,
                definitive: true,
                ..SandboxProbe::checking()
            },
            platform: SandboxPlatform::Linux,
        });
        let rows = text(&mut bare, 200);
        assert!(rows.contains("  Sandbox: available\n"));
        assert!(!rows.contains("Sandbox: available —"));
    }

    #[test]
    fn the_downgrade_banner_is_rendered_verbatim() {
        let mut view = downgraded_view(SandboxPlatform::Linux);
        let rows = text(&mut view, 200);
        assert!(rows.contains("! Sandbox requested but unavailable"));
        assert!(rows.contains("fell back to Manual"));
        assert!(rows.contains("(diagnostic: binary_missing)."));

        // A macOS view that gets the same answer moves its highlight too.
        let mut macos = downgraded_view(SandboxPlatform::Macos);
        assert_eq!(macos.highlighted_tier(), SandboxTier::Manual);
        assert!(text(&mut macos, 200).contains("Sandbox requested but unavailable"));
    }

    #[test]
    fn linux_and_windows_get_their_own_explanations() {
        let mut linux = SandboxView::new(SandboxPlatform::Linux);
        assert!(text(&mut linux, 200).contains("runs it through the system Bubblewrap"));

        let mut windows = SandboxView::new(SandboxPlatform::Windows);
        let rows = text(&mut windows, 200);
        assert!(rows.contains("Windows has no OS sandbox"));
        assert!(rows.contains("write protection"));

        let mut macos = SandboxView::new(SandboxPlatform::Macos);
        let rows = text(&mut macos, 200);
        assert!(!rows.contains("Windows has no OS sandbox"));
        assert!(!rows.contains("runs it through the system Bubblewrap"));

        let mut other = SandboxView::new(SandboxPlatform::Other);
        assert!(text(&mut other, 200).contains("No sandbox backend exists on this platform"));
    }

    #[test]
    fn long_prose_wraps_instead_of_being_lost() {
        let mut view = downgraded_view(SandboxPlatform::Linux);
        let narrow = lines(&mut view, 40);
        assert!(narrow.len() > 14, "the banner and descriptions wrap");
        assert!(narrow.iter().any(|row| row.contains("Sandbox requested")));
        assert!(narrow.iter().any(|row| row.starts_with("!")));
        assert!(narrow.iter().any(|row| row.contains("approval rules")));
    }

    #[test]
    fn render_truncates_to_the_requested_height() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(view.render(80, 1).len(), 1);
        assert_eq!(view.render(80, 3).len(), 3);
        assert_eq!(view.render(80, 0).len(), 0);
        assert_eq!(view.render(0, 10).len(), 0);
        assert_eq!(view.render(1, 1).len(), 1);
        assert!(view.render(1, 4).iter().all(|row| visible_width(row) == 1));
    }

    #[test]
    fn every_row_of_every_status_is_exactly_the_requested_width() {
        let cases = vec![
            SandboxView::new(SandboxPlatform::Macos),
            SandboxView::new(SandboxPlatform::Linux),
            SandboxView::new(SandboxPlatform::Windows),
            SandboxView::new(SandboxPlatform::Other),
            downgraded_view(SandboxPlatform::Macos),
            downgraded_view(SandboxPlatform::Linux),
        ];
        for mut view in cases {
            for width in 0..=60 {
                for height in 0..=18 {
                    let rows = view.render(width, height);
                    assert!(rows.len() <= height);
                    if width == 0 || height == 0 {
                        assert!(rows.is_empty());
                    }
                    for row in &rows {
                        assert_eq!(visible_width(row), width);
                    }
                }
            }
        }
    }

    #[test]
    fn set_theme_recolors_every_section() {
        let theme = Theme {
            accent: 100,
            dim: 101,
            fg: 102,
            error: 103,
            success: 104,
            selected_bg: 105,
            selected_fg: 106,
            ..DARK_THEME
        };
        let mut view = downgraded_view(SandboxPlatform::Linux);
        view.set_theme(&theme);
        view.set_permission_level(PermissionKind::Workspace);
        let rows = view.render(120, 200);
        let joined = rows.join("\n");
        assert!(joined.contains("\x1b[38;5;100m"));
        assert!(joined.contains("\x1b[38;5;101m"));
        assert!(joined.contains("\x1b[38;5;102m"));
        assert!(joined.contains("\x1b[38;5;103m"));
        assert!(joined.contains("\x1b[38;5;106m"));
        assert!(joined.contains("\x1b[48;5;105m"));

        let mut default = downgraded_view(SandboxPlatform::Linux);
        default.set_permission_level(PermissionKind::Workspace);
        assert_ne!(default.render(120, 200).join("\n"), joined);
    }

    #[test]
    fn the_selection_bar_never_cuts_an_escape_sequence() {
        // A focused row longer than the terminal: the truncation has to happen
        // before the background is applied, or the row would end mid-sequence.
        let theme = Theme {
            selected_fg: 106,
            selected_bg: 105,
            ..DARK_THEME
        };
        let row = selected_row("› a very long focused row that cannot fit", 10, &theme);
        assert_eq!(visible_width(&row), 10);
        assert!(row.contains("\x1b[48;5;105m"));
        assert_eq!(fit_row("abc", 0), "");
    }
}
