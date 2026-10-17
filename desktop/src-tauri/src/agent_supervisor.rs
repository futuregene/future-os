//! GUI adapter over the shared in-process Agent host. External Agents retain
//! their ownership; successful full cleanup is required before relaunch.
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);
static QUIT_DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub phase: String,
    pub desktop_version: String,
    pub agent_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shutdown: Option<crate::desktop_shutdown::StopSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn start_agent_supervision<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), crate::AppError> {
    ensure_agent_running(app)
}
pub fn ensure_agent_running<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
) -> Result<(), crate::AppError> {
    #[cfg(not(test))]
    {
        crate::agent_host::host().start(crate::agent_bridge::raw_agent_addr())
    }
    #[cfg(test)]
    {
        Ok(())
    } // Mock Tauri apps must never launch a real default-home Agent.
}
pub async fn ensure_agent_ready_for_login<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> bool {
    if !matches!(
        tokio::task::spawn_blocking(move || ensure_agent_running(&app)).await,
        Ok(Ok(()))
    ) {
        return false;
    }
    crate::agent_bridge::get_agent_readiness().await.is_ok()
}
pub async fn agent_status() -> AgentStatus {
    let mut status = AgentStatus {
        phase: "unavailable".into(),
        desktop_version: crate::build_info::VERSION.into(),
        agent_version: None,
        shutdown: crate::desktop_shutdown::snapshot(),
        error: None,
    };
    if let Some(shutdown) = &status.shutdown {
        status.phase = "stopping".into();
        if shutdown.state == "failed" {
            status.error = Some(shutdown.errors.join("; "));
        }
        return status;
    }
    if crate::desktop_shutdown::is_stopping() {
        status.phase = "stopping".into();
        return status;
    }
    match crate::agent_host::host().phase() {
        crate::agent_host::Phase::Starting => {
            status.phase = "starting".into();
            return status;
        }
        crate::agent_host::Phase::Failed(error) => {
            status.error = Some(error);
        }
        _ => {}
    }
    match crate::agent_bridge::get_agent_readiness().await {
        Ok(Some(info)) if status.error.is_none() => {
            status.phase = "ready".into();
            status.agent_version = Some(info.version);
            status.error = None;
        }
        Err(crate::AppError::AgentIncompatible {
            agent_version,
            reason,
        }) => {
            status.phase = "incompatible".into();
            status.agent_version = agent_version;
            status.error = Some(reason);
        }
        Err(error) if status.error.is_none() => status.error = Some(error.to_string()),
        _ => {}
    }
    status
}

/// The invoke signature remains stable. Recovery requires a completed native
/// stop; a status poll never replaces a failed or externally owned service.
pub async fn agent_status_with_recovery<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
) -> AgentStatus {
    agent_status().await
}

#[derive(Debug, PartialEq, Eq)]
pub enum QuitDecision {
    Proceed,
    Confirm { open_dialog: bool },
}
fn active_work_count() -> Result<usize, crate::AppError> {
    if matches!(
        crate::agent_host::host().phase(),
        crate::agent_host::Phase::Stopped
    ) {
        return Ok(0);
    }
    if matches!(
        crate::agent_host::host().phase(),
        crate::agent_host::Phase::Starting
    ) {
        return Err("Agent is still starting".into());
    }
    match crate::agent_host::host().activity()? {
        Some(activity) => Ok(activity.running
            + activity.queued
            + activity.approvals
            + activity.compactions
            + activity.commands
            + activity.background),
        None => Ok(crate::desktop_shutdown::owned_count()),
    }
}
pub fn on_close_requested() -> QuitDecision {
    quit_decision(
        QUIT_CONFIRMED.load(Ordering::Acquire),
        QUIT_DIALOG_OPEN.load(Ordering::Acquire),
        active_work_count(),
    )
}
fn quit_decision(
    confirmed: bool,
    dialog_open: bool,
    activity: Result<usize, crate::AppError>,
) -> QuitDecision {
    if confirmed {
        return QuitDecision::Proceed;
    }
    if dialog_open {
        return QuitDecision::Confirm { open_dialog: false };
    }
    match activity {
        Ok(0) => QuitDecision::Proceed,
        _ => QuitDecision::Confirm { open_dialog: true },
    }
}
pub fn confirm_quit<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    QUIT_DIALOG_OPEN.store(true, Ordering::Release);
    let message = if crate::agent_host::host().owns_agent() {
        "Agent work is still active or cannot be checked. Quitting stops all connected clients' work, including CLI conversations. Quit anyway?"
    } else {
        "Desktop still has submitted work. Quitting cancels only this Desktop's submitted runs; the external Agent and other clients keep running. Quit anyway?"
    };
    let mut dialog = app
        .dialog()
        .message(message)
        .title("Quit FutureOS?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Force quit".into(),
            "Keep running".into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |confirmed| {
        QUIT_DIALOG_OPEN.store(false, Ordering::Release);
        if confirmed {
            request_quit(app, true);
        }
    });
}

static QUIT_PENDING: AtomicBool = AtomicBool::new(false);

/// Intercept ordinary exits while the event loop can still render progress.
/// This waiter is deliberately outside the task registry it must shut down.
pub(crate) fn request_quit<R: tauri::Runtime>(app: tauri::AppHandle<R>, force: bool) {
    if QUIT_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(operation) = crate::desktop_shutdown::begin_quit() else {
        QUIT_PENDING.store(false, Ordering::Release);
        return;
    };
    let fallback = app.clone();
    if let Err(error) = std::thread::Builder::new().name("desktop-quit".into()).spawn(move || {
        let result = operation.wait();
        if let Err(error) = result {
            eprintln!("FutureOS exit cleanup failed: {error}");
            if !force {
                app.dialog().message(format!("FutureOS could not complete cleanup: {error}. Exit the process anyway?"))
                    .title("Cleanup failed")
                    .kind(MessageDialogKind::Error)
                    .buttons(MessageDialogButtons::OkCancelCustom("Exit anyway".into(), "Keep open".into()))
                    .show(move |confirmed| {
                        QUIT_PENDING.store(false, Ordering::Release);
                        if confirmed { QUIT_CONFIRMED.store(true, Ordering::Release); app.exit(0); }
                    });
                return;
            }
        }
        QUIT_CONFIRMED.store(true, Ordering::Release);
        app.exit(0);
    }) {
        QUIT_PENDING.store(false, Ordering::Release);
        fallback.dialog().message(format!("Cannot start exit waiter: {error}. Cleanup status remains available."))
            .kind(MessageDialogKind::Error).show(|_| {});
    }
}

pub(crate) fn authorize_exit() {
    QUIT_CONFIRMED.store(true, Ordering::Release);
}
pub(crate) fn exit_authorized() -> bool {
    QUIT_CONFIRMED.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_or_busy_activity_requires_confirmation() {
        assert_eq!(
            quit_decision(false, false, Err("unknown".into())),
            QuitDecision::Confirm { open_dialog: true }
        );
        assert_eq!(
            quit_decision(false, false, Ok(1)),
            QuitDecision::Confirm { open_dialog: true }
        );
        assert_eq!(quit_decision(false, false, Ok(0)), QuitDecision::Proceed);
        assert_eq!(
            quit_decision(false, true, Ok(1)),
            QuitDecision::Confirm { open_dialog: false }
        );
        assert_eq!(quit_decision(true, false, Ok(1)), QuitDecision::Proceed);
    }
    #[test]
    fn mock_gui_does_not_start_an_agent() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        ensure_agent_running(app.handle()).unwrap();
    }

    /// The GUI entry point is not a second, weaker path to the same decision:
    /// it routes through `ensure_agent_running`, so a mock app cannot start a
    /// real Agent through it either.
    #[test]
    fn supervision_through_the_gui_entry_point_is_also_safe_for_a_mock_app() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        start_agent_supervision(app.handle()).unwrap();
    }

    /// `authorize_exit` is what an already-confirmed quit reads: the close
    /// handler must proceed *without* reopening the confirmation dialog, and the
    /// decision must not have to consult the Agent to know that.
    #[test]
    fn an_authorized_exit_proceeds_without_a_dialog() {
        authorize_exit();
        assert!(exit_authorized());
        assert_eq!(on_close_requested(), QuitDecision::Proceed);
    }

    /// The recovery status is the same status: it never replaces a service it
    /// cannot own, so a poll through it cannot disagree with the plain poll.
    #[tokio::test]
    async fn the_recovery_poll_reports_the_same_phase_as_the_plain_poll() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let recovered = agent_status_with_recovery(app.handle().clone()).await;
        let plain = agent_status().await;
        assert_eq!(recovered.phase, plain.phase);
        assert_eq!(recovered.desktop_version, crate::build_info::VERSION);
        assert_eq!(recovered.agent_version, plain.agent_version);
        assert_eq!(recovered.error, plain.error);
    }

    /// The status type is the webview's contract: every field is serialized in
    /// camelCase and absent optionals are omitted rather than sent as null.
    #[test]
    fn the_status_serialization_matches_the_webview_contract() {
        let status = AgentStatus {
            phase: "unavailable".into(),
            desktop_version: "1.2.3".into(),
            agent_version: None,
            shutdown: None,
            error: None,
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["phase"], "unavailable");
        assert_eq!(value["desktopVersion"], "1.2.3");
        assert!(
            value["agentVersion"].is_null(),
            "the webview reads the field as JSON, so an unknown version is null: {value}"
        );
        assert!(
            value.get("shutdown").is_none(),
            "an absent shutdown snapshot is omitted, not null: {value}"
        );
        assert!(
            value.get("error").is_none(),
            "an absent error is omitted, not null: {value}"
        );
        let busy = AgentStatus {
            phase: "incompatible".into(),
            desktop_version: "1.2.3".into(),
            agent_version: Some("9.9.9".into()),
            shutdown: None,
            error: Some("protocol mismatch".into()),
        };
        let value = serde_json::to_value(&busy).unwrap();
        assert_eq!(value["agentVersion"], "9.9.9");
        assert_eq!(value["error"], "protocol mismatch");
    }
}
