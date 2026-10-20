use tauri::Emitter;

/// Forward a whitelisted event to the webview as `agent-event`. Both owner
/// identities are injected so the frontend can reject stale listeners during
/// a thread switch instead of trusting session identity alone.
#[cfg(feature = "gui")]
pub(crate) fn forward_settings_event<R: tauri::Runtime>(
    app_handle: Option<&tauri::AppHandle<R>>,
    session_id: &str,
    thread_id: &str,
    event_type: &str,
    data: &str,
) {
    if let Some(app_handle) = app_handle {
        emit_settings_event(app_handle, session_id, thread_id, event_type, data);
    }
}

/// Emit the enriched payload via an injectable `Emitter`. Generic over
/// `Runtime` so unit tests can drive the emit body with a mock app handle
/// (the process-global `APP_HANDLE` is `None` outside a running Tauri app).
#[cfg(feature = "gui")]
fn emit_settings_event<R: tauri::Runtime>(
    app_handle: &tauri::AppHandle<R>,
    session_id: &str,
    thread_id: &str,
    event_type: &str,
    data: &str,
) {
    if let Some(payload) =
        future_host::agent_bridge::settings_event_payload(session_id, thread_id, event_type, data)
    {
        let _ = app_handle.emit("agent-event", &payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "gui")]
    #[test]
    fn forward_settings_event_emits_or_skips_without_a_handle() {
        // No handle → no emit (the `if let Some` body is skipped).
        forward_settings_event(
            None::<&tauri::AppHandle<tauri::test::MockRuntime>>,
            "sess",
            "thread",
            "model_changed",
            "{}",
        );
        // A mock handle → the emit body runs (valid + malformed payloads).
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let handle = app.handle();
        forward_settings_event(
            Some(handle),
            "sess",
            "thread",
            "model_changed",
            r#"{"model":"k3"}"#,
        );
        forward_settings_event(Some(handle), "sess", "thread", "model_changed", "not json");
    }

    #[cfg(feature = "gui")]
    #[test]
    fn emit_settings_event_emits_through_a_mock_handle() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let handle = app.handle();
        // Valid object payload → emits; malformed JSON → no emit.
        emit_settings_event(
            handle,
            "sess",
            "thread",
            "model_changed",
            r#"{"model":"k3"}"#,
        );
        emit_settings_event(handle, "sess", "thread", "model_changed", "not json");
    }
}
