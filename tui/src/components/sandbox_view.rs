//! The TUI's approval-mode card.
//!
//! Port of the desktop's approval-mode surface: the tier `<Select>` in
//! `desktop/src/features/settings/GeneralPage.tsx` (and the composer dropdown
//! in `ComposerControls.tsx`, which adds the `auto` row), the availability
//! probe in `integrations/agent/useSandboxAvailability.ts` and the stable
//! diagnostic codes in `features/settings/linuxSandboxStatus.ts`.
//!
//! Four product modes share one picker (desktop `ApprovalTier`):
//! `manual`, `sandbox`, `auto` (the sandbox tier plus the model reviewer, so a
//! second model classifies each Ask request and approves the bounded ones) and
//! `off`. On the wire `auto` is not a tier: [`ApprovalMode::to_wire`] sends
//! `tier = sandbox, reviewer = model`.
//!
//! The card is deliberately mode-only: the tool-permission level is a separate
//! legacy setting the `/permission` command sets directly, not something the
//! mode picker needs to surface.
//!
//! The agent owns all halves of the state — the policy
//! (`set_sandbox_policy`), the probe (`probe_sandbox`) and the account gate
//! (`list_providers`'s `future` key presence) — so this module is deliberately
//! I/O-free: it holds the last status the app learned, renders it, and reports
//! *intent* ([`SandboxAction`]) for the caller to turn into an RPC. Same
//! contract as [`crate::components::menu`]: `Moved` means "redraw",
//! value-carrying actions mean "act (then feed the result back through
//! [`SandboxView::set_status`])".
//!
//! Design notes:
//!
//! * **The protected modes need their prerequisites.** `sandbox` is disabled
//!   while the probe has not reported a backend; `auto` additionally needs the
//!   Future account (`AccountState::SignedIn`). Navigation skips disabled
//!   rows. The gates are *availability*, not [`supports_sandbox`]: a Linux host
//!   with a working Bubblewrap reports `available` and may run sandboxed.
//! * **Checking is not unavailable.** An unresolved probe (`resolved == false`)
//!   renders "checking the system sandbox…"; a resolved-but-unavailable probe
//!   renders the diagnostic [reason](reason_text) together with its code. The
//!   `auto` row does the same for the account signal ("(sign in)").
//! * **A downgrade is never hidden.** The agent answers
//!   `set_sandbox_policy { tier: "sandbox" }` with `requestedTier: "sandbox",
//!   tier: "manual"` when the probe says the sandbox is unavailable, and
//!   `configuredMode: "auto", effectiveMode: "sandbox"` when the account is
//!   missing. [`SandboxStatus::fallback_notice`] turns those pairs into an
//!   explicit banner which [`SandboxView::render`] prints: reporting the
//!   request as applied would lie to the user.
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

/// Panel title (also the section heading — the card has one section).
const TITLE: &str = "Approval mode";
/// Key legend rendered as the last row.
const KEY_LEGEND: &str = "↑/↓ move · enter applies · r re-checks · esc closes";
/// `off` description (desktop `approvalTier.description.off`). Names the
/// bypass explicitly: the label (`Full access`, after Codex's capability
/// ladder) is friendlier than Claude Code's `Bypass permissions`, so the copy
/// has to carry the warning.
const DESC_OFF: &str =
    "No prompts, no sandbox — every command runs with full access to this machine.";
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
/// `auto` description (desktop `approvalTier.description.auto`).
const DESC_AUTO: &str = "Automatically reviews actions with sandbox protection.";
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

// ─── Approval mode ─────────────────────────────────────────────────────────

/// The product approval mode a session runs under — the four values the
/// desktop's `ApprovalTier` and the CLI's `--sandbox` share. `Auto` is not a
/// fourth tier on the wire: it is `tier = sandbox` plus `reviewer = model`
/// (the agent's `SandboxPolicy::mode()` spelling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalMode {
    /// `"manual"` — approval rules on, no OS sandbox, the human approves.
    Manual,
    /// `"sandbox"` — approval rules on, shell inside the OS sandbox, the human
    /// approves what the sandbox lets through.
    Sandbox,
    /// `"auto"` — the sandbox tier plus the model reviewer: a second model
    /// classifies each Ask request and approves the bounded ones. Needs an
    /// available sandbox backend *and* a signed-in Future account.
    Auto,
    /// `"off"` — no approval, no sandbox, everything runs. Shown as
    /// `Full access` (the top of Codex's capability ladder; Claude Code calls
    /// the same thing `Bypass permissions`).
    Off,
}

impl ApprovalMode {
    /// Display order of the picker, matching the desktop composer dropdown:
    /// Manual, Sandboxed, Automatic review, Full access.
    pub const ALL: [ApprovalMode; 4] = [
        ApprovalMode::Manual,
        ApprovalMode::Sandbox,
        ApprovalMode::Auto,
        ApprovalMode::Off,
    ];

    /// The canonical mode name — the wire value the agent stores and the
    /// spelling persisted in settings (`manual` | `sandbox` | `auto` | `off`).
    /// Deliberately separate from [`ApprovalMode::badge`] and
    /// [`ApprovalMode::label`]: the product label can be reworded without
    /// breaking a saved default, and `off` stays the contract.
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalMode::Manual => "manual",
            ApprovalMode::Sandbox => "sandbox",
            ApprovalMode::Auto => "auto",
            ApprovalMode::Off => "off",
        }
    }

    /// The `(tier, reviewer)` pair the agent's `set_sandbox_policy` carries.
    pub fn to_wire(self) -> (&'static str, &'static str) {
        match self {
            ApprovalMode::Manual => ("manual", "user"),
            ApprovalMode::Sandbox => ("sandbox", "user"),
            ApprovalMode::Auto => ("sandbox", "model"),
            ApprovalMode::Off => ("off", "user"),
        }
    }

    /// The mode string the agent round-trips (`SandboxPolicy::mode()`): the
    /// `effectiveMode` of a `set_sandbox_policy` answer, the `mode` of a
    /// `sandbox_policy_changed` event and — since the mode-aware read-back —
    /// `get_state.sandboxTier`.
    pub fn from_wire(raw: &str) -> Option<ApprovalMode> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "manual" => Some(ApprovalMode::Manual),
            "sandbox" => Some(ApprovalMode::Sandbox),
            "auto" => Some(ApprovalMode::Auto),
            "off" => Some(ApprovalMode::Off),
            _ => None,
        }
    }

    /// Parse a user-typed mode for `/mode`: the canonical names plus the
    /// display alias of the `Full access` label, so a user who reads the
    /// panel can type what they saw. Agent payloads use
    /// [`ApprovalMode::from_wire`] instead — a display alias is not a value
    /// the agent ever sends.
    pub fn from_user_input(raw: &str) -> Option<ApprovalMode> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "full" | "full access" | "full-access" | "full_access" | "unrestricted" => {
                Some(ApprovalMode::Off)
            }
            _ => ApprovalMode::from_wire(raw),
        }
    }

    /// The label shown in the mode picker (desktop `approvalTier.*`).
    pub fn label(self) -> &'static str {
        match self {
            ApprovalMode::Manual => "Manual",
            ApprovalMode::Sandbox => "Sandboxed",
            ApprovalMode::Auto => "Automatic review",
            ApprovalMode::Off => "Full access",
        }
    }

    /// The short footer badge (`[manual]` / `[sandbox]` / `[auto]` / `[full]`).
    /// A display spelling only — never persist it; use
    /// [`ApprovalMode::as_str`] for that.
    pub fn badge(self) -> &'static str {
        match self {
            ApprovalMode::Manual => "manual",
            ApprovalMode::Sandbox => "sandbox",
            ApprovalMode::Auto => "auto",
            ApprovalMode::Off => "full",
        }
    }

    /// The long description (desktop `approvalTier.description.*`); the
    /// `sandbox` copy is per-platform.
    pub fn description(self, platform: SandboxPlatform) -> &'static str {
        match self {
            ApprovalMode::Manual => DESC_MANUAL,
            ApprovalMode::Sandbox => match platform {
                SandboxPlatform::Macos => DESC_SANDBOX_MACOS,
                SandboxPlatform::Linux => DESC_SANDBOX_LINUX,
                SandboxPlatform::Windows => DESC_SANDBOX_WINDOWS,
                SandboxPlatform::Other => DESC_SANDBOX_UNSUPPORTED,
            },
            ApprovalMode::Auto => DESC_AUTO,
            ApprovalMode::Off => DESC_OFF,
        }
    }
}

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

    /// The backend reported by the connected Agent, independent of the TUI OS.
    pub fn from_backend(backend: &str) -> Option<Self> {
        match backend {
            "macos_seatbelt" => Some(Self::Macos),
            "linux_bubblewrap" | "bubblewrap" => Some(Self::Linux),
            "windows_restricted" => Some(Self::Windows),
            "none" => Some(Self::Other),
            _ => None,
        }
    }

    /// The host this binary was built for.
    pub fn current() -> SandboxPlatform {
        SandboxPlatform::from_os(std::env::consts::OS)
    }
}

/// `true` only for a platform with its own OS sandbox (macOS Seatbelt).
///
/// Deliberately *not* the gate for [`SandboxView::mode_options`]: Linux reaches
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

/// The platform-specific caveat shown under the mode list, or `None` when the
/// platform has nothing extra to say (macOS).
pub fn platform_note(platform: SandboxPlatform) -> Option<&'static str> {
    match platform {
        SandboxPlatform::Macos => None,
        SandboxPlatform::Linux => Some(
            "Linux uses system Bubblewrap, unlike macOS Seatbelt. The sandbox is unavailable when Bubblewrap is missing, untrusted or cannot provide basic write protection.",
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
        Some("version_too_old") => "This Bubblewrap package cannot provide the required protection. Upgrade it, then restart the agent.",
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

/// Whether the Future account credential the automatic reviewer needs is
/// present. The TUI's answer comes from `list_providers` (the `future`
/// provider's stored key); the agent's own `system_one::resolve` reads the
/// same `auth.json` entry, so the two agree unless one side is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccountState {
    /// Not asked yet — the `auto` row renders as "checking".
    #[default]
    Checking,
    /// A `future` key is configured.
    SignedIn,
    /// No `future` key.
    SignedOut,
}

impl AccountState {
    /// Parse the `hasApiKey` of the `future` provider entry.
    pub fn from_has_key(has_key: bool) -> Self {
        if has_key {
            AccountState::SignedIn
        } else {
            AccountState::SignedOut
        }
    }
}

/// Everything the settings screen knows about the sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxStatus {
    /// The mode the session is actually running under, when one is known —
    /// set by this client's own `set_sandbox_policy` answer, a
    /// `sandbox_policy_changed` event or (mode-aware agents) `get_state`.
    /// `None` means the session never chose a policy, which the agent treats
    /// as fully open — not the same thing as `Off`.
    pub mode: Option<ApprovalMode>,
    /// The mode the last `set_sandbox_policy` asked for, when the app has seen
    /// its answer. [`SandboxStatus::is_known_downgrade`] decides whether the
    /// difference from `mode` is one of the agent's silent downgrades.
    pub requested_mode: Option<ApprovalMode>,
    /// Last known probe result.
    pub probe: SandboxProbe,
    /// Whether the Future account the automatic reviewer needs is signed in.
    pub account: AccountState,
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
            mode: None,
            requested_mode: None,
            probe: initial_probe(platform),
            account: AccountState::Checking,
            platform,
        }
    }

    /// Parse the `set_sandbox_policy` answer, which is where the downgrade is
    /// visible (`effectiveMode` + `configuredMode` + `sandboxAvailable` +
    /// `fallback`).
    ///
    /// Tolerant: a payload that is not an object yields [`SandboxStatus::new`],
    /// an unrecognised `effectiveMode`/`tier` leaves the mode unknown (the
    /// panel then shows the platform default together with its caveat), and an
    /// answer without `sandboxAvailable` leaves the probe unresolved instead of
    /// claiming unavailability.
    pub fn from_policy_response(platform: SandboxPlatform, value: &Value) -> Self {
        match value.as_object() {
            None => SandboxStatus::new(platform),
            Some(map) => {
                // Prefer the explicit mode spelling; fall back to tier+reviewer
                // (pre-mode agents never said "auto" in `tier`, but they did
                // say `reviewer: "model"` on the sandbox tier). The two are
                // siblings, not a chain: a legacy answer always carries `tier`,
                // so folding it into the same `and_then` would shadow the
                // reviewer and silently report `auto` as `sandbox`.
                let mode = field_text(map, &["effectiveMode", "effective_mode"])
                    .and_then(|raw| ApprovalMode::from_wire(&raw))
                    .or_else(|| {
                        let tier = field_text(map, &["tier"])
                            .as_deref()
                            .and_then(SandboxTier::from_wire)?;
                        if tier == SandboxTier::Sandbox
                            && field_text(map, &["reviewer"]).as_deref() == Some("model")
                        {
                            return Some(ApprovalMode::Auto);
                        }
                        ApprovalMode::from_wire(tier.to_wire())
                    });
                let requested_mode = field_text(map, &["configuredMode", "configured_mode"])
                    .or_else(|| field_text(map, &["requestedTier", "requested_tier"]))
                    .and_then(|raw| ApprovalMode::from_wire(&raw));
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
                // The agent only reports `reviewer` after it applied the
                // account policy, so a model reviewer that survived means the
                // account was usable. `reviewer: "user"` says nothing on its
                // own — every non-auto policy answers that — so only an `auto`
                // request that came back downgraded proves the account is
                // missing. Anything else stays unresolved, which keeps a
                // `manual`/`sandbox` pick from overwriting the account answer
                // the panel already has.
                let account = match field_text(map, &["reviewer"]).as_deref() {
                    Some("model") => AccountState::SignedIn,
                    Some("user") if requested_mode == Some(ApprovalMode::Auto) => {
                        AccountState::SignedOut
                    }
                    _ => AccountState::Checking,
                };
                let platform = probe
                    .backend
                    .as_deref()
                    .and_then(SandboxPlatform::from_backend)
                    .unwrap_or(platform);
                SandboxStatus {
                    mode,
                    requested_mode,
                    probe,
                    account,
                    platform,
                }
            }
        }
    }

    /// Which of the three probe states the status is in.
    pub fn probe_state(&self) -> ProbeState {
        self.probe.state()
    }

    /// The mode the session runs under when one is known, else the tier this
    /// platform would default to (rendered with the caveat that it is a
    /// guess — see [`crate::app::sandbox_mode_caveat`]).
    pub fn effective_mode(&self) -> ApprovalMode {
        self.mode.unwrap_or_else(|| {
            ApprovalMode::from_wire(platform_default_tier(self.platform).to_wire())
                .unwrap_or(ApprovalMode::Manual)
        })
    }

    /// The agent's silent downgrades as `(requested, effective)` pairs:
    /// `auto` → `sandbox` when the account is missing, `auto`/`sandbox` →
    /// `manual` when the backend is unavailable. Nothing else counts — in
    /// particular a protected mode that was applied is not a downgrade just
    /// because a pre-mode answer spelled the request differently.
    pub fn is_known_downgrade(
        requested: Option<ApprovalMode>,
        effective: Option<ApprovalMode>,
    ) -> bool {
        matches!(
            (requested, effective),
            (Some(ApprovalMode::Auto), Some(ApprovalMode::Sandbox))
                | (Some(ApprovalMode::Auto), Some(ApprovalMode::Manual))
                | (Some(ApprovalMode::Sandbox), Some(ApprovalMode::Manual))
        )
    }

    /// `true` when the session asked for a protected mode and did not get it.
    /// The agent downgrades silently (`auto` → `sandbox` without an account,
    /// `sandbox`/`auto` → `manual` without a backend), so this is the only
    /// signal that the request was not honoured.
    pub fn is_fallback(&self) -> bool {
        SandboxStatus::is_known_downgrade(self.requested_mode, self.mode)
    }

    /// The banner explaining [`SandboxStatus::is_fallback`], or `None` when the
    /// requested mode was applied.
    pub fn fallback_notice(&self) -> Option<String> {
        if !self.is_fallback() {
            return None;
        }
        let requested = self.requested_mode?;
        if requested == ApprovalMode::Auto {
            // The agent's two auto downgrades: no account → sandbox (manual
            // approval), no backend → manual (allowlist only).
            return Some(match self.mode {
                Some(ApprovalMode::Sandbox) =>
                    "Automatic review needs a FutureOS account — running sandboxed with manual approval instead."
                        .to_string(),
                _ => format!(
                    "Automatic review requested but unavailable — fell back to {}: approved or allowlisted commands run without an OS sandbox (diagnostic: {}).",
                    self.effective_mode().label(),
                    self.probe.diagnostic_code()
                ),
            });
        }
        Some(format!(
            "Sandbox requested but unavailable — fell back to {}: approved or allowlisted commands run without an OS sandbox (diagnostic: {}).",
            self.effective_mode().label(),
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

// ─── Sandbox view ──────────────────────────────────────────────────────────

/// The approval-mode card.
///
/// A flat list of four rows with exactly one highlight, so `↑`/`↓` walk the
/// card and `enter` applies whatever is highlighted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxView {
    status: SandboxStatus,
    theme: Theme,
    mode_index: usize,
}

impl SandboxView {
    /// A view for `platform`, with the status the desktop starts from (no RPC
    /// has been sent yet).
    pub fn new(platform: SandboxPlatform) -> Self {
        let status = SandboxStatus::new(platform);
        let mode_index = SandboxView::index_of(status.effective_mode());
        SandboxView {
            status,
            theme: DARK_THEME,
            mode_index,
        }
    }

    /// Apply the theme used for every row.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
    }

    /// Replace the status (a `set_sandbox_policy` answer, a probe, or a
    /// `get_state` reload). A highlight left on a row the new status disables
    /// moves to the nearest selectable mode.
    pub fn set_status(&mut self, status: SandboxStatus) {
        self.status = status;
        self.clamp_mode_index();
    }

    /// The current status.
    pub fn status(&self) -> &SandboxStatus {
        &self.status
    }

    /// The picker rows with their selectability. `sandbox` needs an available
    /// backend; `auto` additionally needs a signed-in Future account.
    pub fn mode_options(&self) -> Vec<(ApprovalMode, bool)> {
        ApprovalMode::ALL
            .iter()
            .map(|mode| (*mode, self.mode_enabled(*mode)))
            .collect()
    }

    /// The highlighted mode. Never a disabled one: navigation skips those and
    /// [`SandboxView::set_status`] moves a stale highlight off them.
    pub fn highlighted_mode(&self) -> ApprovalMode {
        ApprovalMode::ALL
            .get(self.mode_index)
            .copied()
            .unwrap_or(ApprovalMode::Manual)
    }

    /// Ask the caller to apply `mode`.
    ///
    /// A disabled mode — `sandbox`/`auto` without an available backend, `auto`
    /// without a signed-in account — answers [`SandboxAction::None`] and leaves
    /// the highlight alone: the request is not queued, so the caller cannot
    /// mistake it for a pending mutation. (The keyboard cannot reach that row
    /// at all; this is the programmatic entry point.)
    pub fn select_mode(&mut self, mode: ApprovalMode) -> SandboxAction {
        if !self.mode_enabled(mode) {
            return SandboxAction::None;
        }
        self.mode_index = SandboxView::index_of(mode);
        SandboxAction::ModeSelected(mode)
    }

    /// `up`/`k`, `down`/`j` walk the four rows, `enter`/`space` applies the
    /// highlighted one, `r` asks for a fresh probe and `esc` closes.
    pub fn handle_key(&mut self, key: &str) -> SandboxAction {
        match key {
            "up" | "k" => self.move_up(),
            "down" | "j" => self.move_down(),
            "enter" | " " => self.select_mode(self.highlighted_mode()),
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
            self.heading_row(
                TITLE,
                &format!("current: {}", self.status.effective_mode().label()),
            ),
            String::new(),
        ];
        rows.extend(self.mode_rows(width));
        rows.extend(wrapped_rows(
            self.highlighted_mode().description(self.status.platform),
            width,
            DESCRIPTION_INDENT,
        ));
        rows.extend(self.platform_rows(width));
        rows.extend(self.availability_rows(width));
        rows.extend(self.fallback_rows(width));
        rows.push(fg(self.theme.dim as u8, &fit_hint_row(KEY_LEGEND, width)));
        rows.truncate(height);
        rows.into_iter().map(|row| fit_row(&row, width)).collect()
    }

    // ── navigation ──

    fn move_up(&mut self) -> SandboxAction {
        match self.step(false) {
            Some(index) => {
                self.mode_index = index;
                SandboxAction::Moved
            }
            None => SandboxAction::None,
        }
    }

    fn move_down(&mut self) -> SandboxAction {
        match self.step(true) {
            Some(index) => {
                self.mode_index = index;
                SandboxAction::Moved
            }
            None => SandboxAction::None,
        }
    }

    // ── rows ──

    fn heading_row(&self, title: &str, hint: &str) -> String {
        format!(
            "{}  {}",
            fg(self.theme.accent as u8, &bold(title)),
            fg(self.theme.dim as u8, hint)
        )
    }

    fn mode_rows(&self, width: usize) -> Vec<String> {
        self.mode_options()
            .into_iter()
            .enumerate()
            .map(|(index, (mode, enabled))| self.mode_row(index, mode, enabled, width))
            .collect()
    }

    fn mode_row(&self, index: usize, mode: ApprovalMode, enabled: bool, width: usize) -> String {
        let focused = index == self.mode_index;
        let marker = if focused { "›" } else { " " };
        let mut row = format!("{marker} {}", mode.label());
        let suffix = self.mode_suffix(mode);
        if !suffix.is_empty() {
            row.push(' ');
            row.push_str(&suffix);
        }
        if Some(mode) == self.status.mode {
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

    /// The `(checking)` / `(unavailable)` / `(sign in)` tag on the protected
    /// rows, empty while they are usable (the desktop prints a plain
    /// "Sandboxed" / "Automatic review" then).
    fn mode_suffix(&self, mode: ApprovalMode) -> String {
        match mode {
            ApprovalMode::Sandbox => match self.status.probe_state() {
                ProbeState::Available => String::new(),
                ProbeState::Checking => fg(self.theme.dim as u8, "(checking)"),
                ProbeState::Unavailable => fg(self.theme.error as u8, "(unavailable)"),
            },
            ApprovalMode::Auto => match (self.status.probe_state(), self.status.account) {
                (ProbeState::Checking, _) => fg(self.theme.dim as u8, "(checking)"),
                (ProbeState::Unavailable, _) => fg(self.theme.error as u8, "(unavailable)"),
                (ProbeState::Available, AccountState::Checking) => {
                    fg(self.theme.dim as u8, "(checking)")
                }
                (ProbeState::Available, AccountState::SignedIn) => String::new(),
                (ProbeState::Available, AccountState::SignedOut) => {
                    fg(self.theme.error as u8, "(sign in)")
                }
            },
            ApprovalMode::Off | ApprovalMode::Manual => String::new(),
        }
    }

    fn platform_rows(&self, width: usize) -> Vec<String> {
        if self.status.platform == SandboxPlatform::Other && !self.status.probe.resolved {
            return Vec::new();
        }
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
                let line = if probe.code.as_deref() == Some("available_write_only") {
                    format!("{line}. Upgrade system Bubblewrap and restart FutureOS to enable full sandbox protection.")
                } else {
                    line
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

    // ── mode selection ──

    /// `sandbox` needs an available backend; `auto` additionally needs a
    /// signed-in Future account; the other two modes are always selectable.
    fn mode_enabled(&self, mode: ApprovalMode) -> bool {
        match mode {
            ApprovalMode::Sandbox => self.status.probe.available,
            ApprovalMode::Auto => {
                self.status.probe.available && self.status.account == AccountState::SignedIn
            }
            ApprovalMode::Off | ApprovalMode::Manual => true,
        }
    }

    /// Index of `mode` in the picker order.
    fn index_of(mode: ApprovalMode) -> usize {
        ApprovalMode::ALL
            .iter()
            .position(|candidate| *candidate == mode)
            .unwrap_or(0)
    }

    /// The nearest selectable row in `direction`, skipping disabled modes.
    fn step(&self, forward: bool) -> Option<usize> {
        let options = self.mode_options();
        let candidates: Vec<usize> = if forward {
            (self.mode_index + 1..options.len()).collect()
        } else {
            (0..self.mode_index).rev().collect()
        };
        candidates.into_iter().find(|index| {
            options
                .get(*index)
                .map(|(_, enabled)| *enabled)
                .unwrap_or(false)
        })
    }

    /// Keep the highlight on a selectable row after the status changed.
    fn clamp_mode_index(&mut self) {
        let options = self.mode_options();
        let valid = options
            .get(self.mode_index)
            .map(|(_, enabled)| *enabled)
            .unwrap_or(false);
        if valid {
            return;
        }
        self.mode_index = options
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
    /// Apply this approval mode (`set_sandbox_policy`).
    ModeSelected(ApprovalMode),
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

    // ── mode ──

    #[test]
    fn mode_wire_round_trips_and_rejects_unknown_values() {
        for mode in ApprovalMode::ALL {
            assert_eq!(ApprovalMode::from_wire(mode.as_str()), Some(mode));
        }
        assert_eq!(ApprovalMode::from_wire("AUTO"), Some(ApprovalMode::Auto));
        assert_eq!(ApprovalMode::from_wire(" auto "), Some(ApprovalMode::Auto));
        assert_eq!(ApprovalMode::from_wire("auto-review"), None);
        assert_eq!(ApprovalMode::from_wire(""), None);
        // The display spelling is not the stored contract: `full` is a badge,
        // `off` is what is persisted and what the agent round-trips.
        assert_eq!(ApprovalMode::Off.as_str(), "off");
        assert_eq!(ApprovalMode::Off.badge(), "full");
        assert_eq!(ApprovalMode::from_wire("full"), None);
        // The user-facing spelling follows the label; the old label keeps
        // working so a muscle-memory `/mode unrestricted` is not a usage error.
        assert_eq!(
            ApprovalMode::from_user_input("full"),
            Some(ApprovalMode::Off)
        );
        assert_eq!(
            ApprovalMode::from_user_input("Full Access"),
            Some(ApprovalMode::Off)
        );
        assert_eq!(
            ApprovalMode::from_user_input("unrestricted"),
            Some(ApprovalMode::Off)
        );
        assert_eq!(
            ApprovalMode::from_user_input("auto"),
            Some(ApprovalMode::Auto)
        );
        assert_eq!(ApprovalMode::Auto.to_wire(), ("sandbox", "model"));
        assert_eq!(ApprovalMode::Sandbox.to_wire(), ("sandbox", "user"));
    }

    #[test]
    fn mode_labels_and_order_match_the_desktop_picker() {
        assert_eq!(
            ApprovalMode::ALL,
            [
                ApprovalMode::Manual,
                ApprovalMode::Sandbox,
                ApprovalMode::Auto,
                ApprovalMode::Off
            ]
        );
        assert_eq!(ApprovalMode::Manual.label(), "Manual");
        assert_eq!(ApprovalMode::Sandbox.label(), "Sandboxed");
        assert_eq!(ApprovalMode::Auto.label(), "Automatic review");
        assert_eq!(ApprovalMode::Off.label(), "Full access");
    }

    #[test]
    fn mode_descriptions_cover_every_mode_and_platform() {
        assert!(ApprovalMode::Off
            .description(SandboxPlatform::Macos)
            .contains("No prompts"));
        assert!(ApprovalMode::Manual
            .description(SandboxPlatform::Macos)
            .contains("approval rules"));
        assert!(ApprovalMode::Auto
            .description(SandboxPlatform::Macos)
            .contains("sandbox"));
        let macos = ApprovalMode::Sandbox.description(SandboxPlatform::Macos);
        let linux = ApprovalMode::Sandbox.description(SandboxPlatform::Linux);
        let windows = ApprovalMode::Sandbox.description(SandboxPlatform::Windows);
        let other = ApprovalMode::Sandbox.description(SandboxPlatform::Other);
        assert!(macos.contains("macOS sandbox"));
        assert!(linux.contains("Linux sandbox"));
        assert!(windows.contains("Windows write protection"));
        assert!(other.contains("no sandbox backend"));
        assert_ne!(macos, linux);
        assert_ne!(linux, windows);
        assert_ne!(windows, other);
    }

    // ── tier (legacy wire values) ──

    #[test]
    fn tier_wire_round_trips_and_rejects_unknown_values() {
        assert_eq!(SandboxTier::Off.to_wire(), "off");
        assert_eq!(SandboxTier::Manual.to_wire(), "manual");
        assert_eq!(SandboxTier::Sandbox.to_wire(), "sandbox");
        for tier in [SandboxTier::Off, SandboxTier::Manual, SandboxTier::Sandbox] {
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
        assert_eq!(macos.effective_mode(), ApprovalMode::Sandbox);
        assert_eq!(macos.probe_state(), ProbeState::Available);
        assert_eq!(macos.probe.backend.as_deref(), Some("macos_seatbelt"));
        assert_eq!(macos.probe.code.as_deref(), Some("available"));
        assert!(macos.probe.definitive);
        assert_eq!(macos.mode, None);
        assert!(!macos.is_fallback());
        assert_eq!(macos.fallback_notice(), None);

        let linux = SandboxStatus::new(SandboxPlatform::Linux);
        assert_eq!(linux.effective_mode(), ApprovalMode::Manual);
        assert_eq!(linux.probe_state(), ProbeState::Checking);
        assert_eq!(
            SandboxStatus::new(SandboxPlatform::Windows).probe_state(),
            ProbeState::Checking
        );

        let other = SandboxStatus::new(SandboxPlatform::Other);
        assert_eq!(other.effective_mode(), ApprovalMode::Manual);
        assert_eq!(other.probe_state(), ProbeState::Unavailable);
        assert_eq!(other.probe.diagnostic_code(), "platform_unsupported");
        assert_eq!(other.probe.backend.as_deref(), Some("none"));
        assert_eq!(other.probe.reason(), REASON_PLATFORM_UNSUPPORTED);
    }

    #[test]
    fn the_agent_downgrade_is_reported_as_a_fallback() {
        let status = SandboxStatus::from_policy_response(SandboxPlatform::Linux, &downgrade());
        assert_eq!(status.mode, Some(ApprovalMode::Manual));
        assert_eq!(status.requested_mode, Some(ApprovalMode::Sandbox));
        assert_eq!(status.probe_state(), ProbeState::Unavailable);
        assert_eq!(status.probe.diagnostic_code(), "binary_missing");
        assert!(status.is_fallback());
        let notice = status.fallback_notice().expect("a downgrade has a notice");
        assert!(notice.contains("Sandbox requested but unavailable"));
        assert!(notice.contains("fell back to Manual"));
        assert!(notice.contains("binary_missing"));
    }

    #[test]
    fn an_auto_downgrade_without_an_account_names_the_reason() {
        let status = SandboxStatus::from_policy_response(
            SandboxPlatform::Macos,
            &json!({
                "tier": "sandbox",
                "effectiveMode": "sandbox",
                "configuredMode": "auto",
                "reviewer": "user",
                "requestedTier": "sandbox",
                "sandboxAvailable": true,
                "sandboxBackend": "macos_seatbelt"
            }),
        );
        assert_eq!(status.mode, Some(ApprovalMode::Sandbox));
        assert_eq!(status.requested_mode, Some(ApprovalMode::Auto));
        assert!(status.is_fallback());
        // The downgrade itself is the proof: the reviewer was dropped for a
        // lack of account.
        assert_eq!(status.account, AccountState::SignedOut);
        let notice = status
            .fallback_notice()
            .expect("the auto downgrade has a notice");
        assert!(notice.contains("needs a FutureOS account"), "{notice}");
        assert!(notice.contains("manual approval"), "{notice}");
    }

    #[test]
    fn an_auto_answer_with_a_model_reviewer_is_applied_auto() {
        let status = SandboxStatus::from_policy_response(
            SandboxPlatform::Macos,
            &json!({
                "tier": "sandbox",
                "effectiveMode": "auto",
                "configuredMode": "auto",
                "reviewer": "model",
                "sandboxAvailable": true,
                "sandboxBackend": "macos_seatbelt"
            }),
        );
        assert_eq!(status.mode, Some(ApprovalMode::Auto));
        assert_eq!(status.requested_mode, Some(ApprovalMode::Auto));
        assert!(!status.is_fallback());
        assert_eq!(status.fallback_notice(), None);
        assert_eq!(status.account, AccountState::SignedIn);
    }

    #[test]
    fn a_legacy_answer_reports_auto_from_tier_and_reviewer() {
        // Pre-mode agents never spelled `auto` in `tier`; the reviewer is the
        // only signal, so it must not be shadowed by the tier parsing.
        let status = SandboxStatus::from_policy_response(
            SandboxPlatform::Macos,
            &json!({
                "tier": "sandbox",
                "requestedTier": "sandbox",
                "reviewer": "model",
                "sandboxAvailable": true,
                "sandboxBackend": "macos_seatbelt"
            }),
        );
        assert_eq!(status.mode, Some(ApprovalMode::Auto));
        assert_eq!(status.account, AccountState::SignedIn);
        assert_eq!(status.fallback_notice(), None);
    }

    #[test]
    fn a_non_auto_answer_leaves_the_account_signal_alone() {
        // Every non-auto policy answers `reviewer: "user"`; that is not a
        // statement about the account, so the panel keeps whatever
        // `list_providers` (or an earlier auto answer) taught it.
        for tier in ["manual", "sandbox", "off"] {
            let status = SandboxStatus::from_policy_response(
                SandboxPlatform::Macos,
                &json!({
                    "tier": tier,
                    "effectiveMode": tier,
                    "configuredMode": tier,
                    "reviewer": "user",
                    "sandboxAvailable": true,
                    "sandboxBackend": "macos_seatbelt"
                }),
            );
            assert_eq!(status.account, AccountState::Checking, "{tier}");
        }
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
        assert_eq!(status.mode, Some(ApprovalMode::Sandbox));
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
        // An unknown mode leaves the guess (the platform default) in place,
        // like the agent's own tier parse.
        let bogus =
            SandboxStatus::from_policy_response(SandboxPlatform::Linux, &json!({ "tier": "??" }));
        assert_eq!(bogus.mode, None);
        assert_eq!(bogus.effective_mode(), ApprovalMode::Manual);
        let bogus_macos =
            SandboxStatus::from_policy_response(SandboxPlatform::Macos, &json!({ "tier": "??" }));
        assert_eq!(bogus_macos.mode, None);
        assert_eq!(bogus_macos.effective_mode(), ApprovalMode::Sandbox);
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
        assert_eq!(snake.requested_mode, Some(ApprovalMode::Sandbox));
        assert_eq!(snake.probe_state(), ProbeState::Unavailable);
        assert_eq!(snake.probe.code, None);
        let notice = snake.fallback_notice().expect("a downgrade has a notice");
        assert!(notice.contains("probe_failed"));
    }

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

    // ── view: options and selection ──

    #[test]
    fn a_fresh_view_starts_on_the_platform_default() {
        let macos = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(macos.highlighted_mode(), ApprovalMode::Sandbox);
        assert_eq!(macos.status().effective_mode(), ApprovalMode::Sandbox);

        let linux = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(linux.highlighted_mode(), ApprovalMode::Manual);
        assert_eq!(linux.mode_options()[1], (ApprovalMode::Sandbox, false));
        assert_eq!(linux.mode_options()[2], (ApprovalMode::Auto, false));
    }

    #[test]
    fn the_sandbox_row_needs_an_available_backend() {
        let macos = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(
            macos.mode_options(),
            vec![
                (ApprovalMode::Manual, true),
                (ApprovalMode::Sandbox, true),
                (ApprovalMode::Auto, false),
                (ApprovalMode::Off, true)
            ]
        );

        let mut linux = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(
            linux.mode_options(),
            vec![
                (ApprovalMode::Manual, true),
                (ApprovalMode::Sandbox, false),
                (ApprovalMode::Auto, false),
                (ApprovalMode::Off, true)
            ]
        );

        // The gate is availability, not the platform: a working Bubblewrap
        // makes the sandbox tier selectable on Linux too — and a signed-in
        // account then unlocks auto on top of it.
        linux.set_status(SandboxStatus {
            mode: Some(ApprovalMode::Manual),
            requested_mode: None,
            probe: SandboxProbe::available("bubblewrap"),
            account: AccountState::SignedOut,
            platform: SandboxPlatform::Linux,
        });
        assert_eq!(linux.mode_options()[1], (ApprovalMode::Sandbox, true));
        assert_eq!(linux.mode_options()[2], (ApprovalMode::Auto, false));
        linux.set_status(SandboxStatus {
            mode: Some(ApprovalMode::Manual),
            requested_mode: None,
            probe: SandboxProbe::available("bubblewrap"),
            account: AccountState::SignedIn,
            platform: SandboxPlatform::Linux,
        });
        assert_eq!(linux.mode_options()[2], (ApprovalMode::Auto, true));
        assert_eq!(
            linux.handle_key("enter"),
            SandboxAction::ModeSelected(ApprovalMode::Manual)
        );
    }

    #[test]
    fn navigation_skips_the_disabled_protected_rows() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(view.handle_key("down"), SandboxAction::Moved);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Off);
        assert_eq!(view.mode_options()[1], (ApprovalMode::Sandbox, false));
        assert_eq!(view.mode_options()[2], (ApprovalMode::Auto, false));
        assert_eq!(view.handle_key("up"), SandboxAction::Moved);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Manual);
        assert_eq!(view.handle_key("up"), SandboxAction::None);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Manual);
        assert_eq!(view.handle_key("j"), SandboxAction::Moved);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Off);
        assert_eq!(view.handle_key("k"), SandboxAction::Moved);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Manual);
    }

    #[test]
    fn navigation_walks_through_auto_when_it_is_enabled() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        // Auto stays disabled while the account is unresolved…
        assert_eq!(view.mode_options()[2], (ApprovalMode::Auto, false));
        assert_eq!(view.handle_key("down"), SandboxAction::Moved);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Off);
        // …and takes its place in the order once it is.
        view.set_status(SandboxStatus {
            mode: Some(ApprovalMode::Sandbox),
            requested_mode: None,
            probe: SandboxProbe::available("macos_seatbelt"),
            account: AccountState::SignedIn,
            platform: SandboxPlatform::Macos,
        });
        assert_eq!(view.handle_key("up"), SandboxAction::Moved);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Auto);
        assert_eq!(
            view.handle_key("enter"),
            SandboxAction::ModeSelected(ApprovalMode::Auto)
        );
    }

    #[test]
    fn selecting_a_disabled_mode_does_nothing() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(view.select_mode(ApprovalMode::Sandbox), SandboxAction::None);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Manual);
        assert_eq!(
            view.select_mode(ApprovalMode::Off),
            SandboxAction::ModeSelected(ApprovalMode::Off)
        );
        assert_eq!(view.highlighted_mode(), ApprovalMode::Off);

        let mut macos = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(
            macos.select_mode(ApprovalMode::Sandbox),
            SandboxAction::ModeSelected(ApprovalMode::Sandbox)
        );
        assert_eq!(macos.highlighted_mode(), ApprovalMode::Sandbox);
    }

    #[test]
    fn enter_applies_whatever_is_highlighted() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        assert_eq!(
            view.handle_key("enter"),
            SandboxAction::ModeSelected(ApprovalMode::Manual)
        );
        assert_eq!(view.handle_key("down"), SandboxAction::Moved);
        assert_eq!(
            view.handle_key(" "),
            SandboxAction::ModeSelected(ApprovalMode::Off)
        );
    }

    #[test]
    fn global_keys_work_from_every_row() {
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

    /// Rows carrying the focus marker.
    fn marked(view: &mut SandboxView) -> usize {
        lines(view, 200)
            .iter()
            .filter(|row| row.contains('›'))
            .count()
    }

    #[test]
    fn exactly_one_row_draws_the_focus_marker() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(marked(&mut view), 1);
        assert!(lines(&mut view, 200)[3].starts_with("› Sandboxed"));
        view.handle_key("down");
        assert_eq!(marked(&mut view), 1);
        assert!(!lines(&mut view, 200)
            .iter()
            .any(|row| row.starts_with("› Sandboxed")));
    }

    #[test]
    fn a_stale_highlight_moves_off_a_row_the_new_status_disables() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        assert_eq!(view.highlighted_mode(), ApprovalMode::Sandbox);
        view.set_status(SandboxStatus::new(SandboxPlatform::Linux));
        assert_eq!(view.highlighted_mode(), ApprovalMode::Manual);
        assert_eq!(view.status().platform, SandboxPlatform::Linux);
        // A still-valid highlight is left alone.
        view.handle_key("down");
        assert_eq!(view.highlighted_mode(), ApprovalMode::Off);
        view.set_status(SandboxStatus::new(SandboxPlatform::Linux));
        assert_eq!(view.highlighted_mode(), ApprovalMode::Off);
    }

    // ── rendering ──

    #[test]
    fn the_panel_renders_its_chrome_in_order() {
        let mut view = SandboxView::new(SandboxPlatform::Macos);
        let rows = lines(&mut view, 200);
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[0], "Approval mode  current: Sandboxed");
        assert_eq!(rows[1], "");
        assert_eq!(rows[2], "  Manual");
        assert_eq!(rows[3], "› Sandboxed");
        assert_eq!(rows[4], "  Automatic review (checking)");
        assert_eq!(rows[5], "  Full access");
        assert_eq!(
            rows[6],
            "  Commands run in the macOS sandbox and ask for approval when needed."
        );
        assert_eq!(rows[7], "  Sandbox: available — backend macos_seatbelt");
        assert_eq!(rows[8], KEY_LEGEND);
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
            mode: Some(ApprovalMode::Manual),
            requested_mode: None,
            probe: SandboxProbe::unavailable("user_namespace_disabled", "bubblewrap"),
            account: AccountState::Checking,
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
    fn write_only_probe_keeps_modes_selectable_with_only_an_upgrade_hint() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        view.set_status(SandboxStatus {
            mode: Some(ApprovalMode::Sandbox),
            requested_mode: None,
            probe: SandboxProbe {
                code: Some("available_write_only".into()),
                ..SandboxProbe::available("linux_bubblewrap")
            },
            account: AccountState::SignedIn,
            platform: SandboxPlatform::Linux,
        });
        assert!(view.mode_enabled(ApprovalMode::Sandbox));
        assert!(view.mode_enabled(ApprovalMode::Auto));
        assert!(view.mode_suffix(ApprovalMode::Sandbox).is_empty());
        assert!(view.mode_suffix(ApprovalMode::Auto).is_empty());
        let rendered = text(&mut view, 240);
        assert!(rendered.contains("Upgrade system Bubblewrap"));
        assert!(!rendered.contains("(unavailable)"));
    }

    #[test]
    fn an_available_probe_reports_its_backend_details() {
        let mut view = SandboxView::new(SandboxPlatform::Linux);
        view.set_status(SandboxStatus {
            mode: Some(ApprovalMode::Manual),
            requested_mode: None,
            probe: SandboxProbe {
                available: true,
                resolved: true,
                definitive: true,
                path: Some("/usr/bin/bwrap".to_string()),
                version: Some("0.9.0".to_string()),
                ..SandboxProbe::available("bubblewrap")
            },
            account: AccountState::Checking,
            platform: SandboxPlatform::Linux,
        });
        let rows = lines(&mut view, 200);
        assert!(
            rows.iter()
                .any(|row| row
                    == "  Sandbox: available — backend bubblewrap · /usr/bin/bwrap · 0.9.0")
        );
        assert!(rows.iter().any(|row| row == "  Sandboxed"));
        // The sandbox row has no qualifier; the auto row still says
        // "(checking)" until the account signal resolves — a different
        // prerequisite, not the same probe.
        assert!(!rows.iter().any(|row| row.starts_with("  Sandboxed (")));
        assert!(rows
            .iter()
            .any(|row| row.starts_with("  Automatic review (")));

        // No details at all still reads as available.
        let mut bare = SandboxView::new(SandboxPlatform::Linux);
        bare.set_status(SandboxStatus {
            mode: Some(ApprovalMode::Manual),
            requested_mode: None,
            probe: SandboxProbe {
                available: true,
                resolved: true,
                definitive: true,
                ..SandboxProbe::checking()
            },
            account: AccountState::Checking,
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
        assert_eq!(macos.highlighted_mode(), ApprovalMode::Manual);
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
        let rows = view.render(120, 200);
        let joined = rows.join("\n");
        assert!(joined.contains("\x1b[38;5;100m"));
        assert!(joined.contains("\x1b[38;5;101m"));
        assert!(joined.contains("\x1b[38;5;102m"));
        assert!(joined.contains("\x1b[38;5;103m"));
        assert!(joined.contains("\x1b[38;5;106m"));
        assert!(joined.contains("\x1b[48;5;105m"));

        let mut default = downgraded_view(SandboxPlatform::Linux);
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
