//! The `#[cfg(not(test))]` half of `App::host_sandbox_platform`.
//!
//! A unit test builds the library **with** `cfg(test)`, so it only ever sees the
//! macOS-pinned twin of `host_sandbox_platform`; an integration test links the
//! library *without* `cfg(test)`, so `SandboxPlatform::current()` is compiled
//! into this target and `/sandbox` reaches it through the public interactive
//! input path. Opening the panel needs no interactive session, no PTY and no
//! agent — only `App::start` needs those, and this test deliberately does not
//! start the app.

use std::sync::{Arc, Mutex};

use future_tui::app::{App, CliOptions, TerminalIo, UiCmd};
use future_tui::rpc::GrpcClient;

/// Records what the app writes, so the test can say how far the render path got.
struct RecordingTerminal {
    writes: Arc<Mutex<Vec<String>>>,
}

impl TerminalIo for RecordingTerminal {
    fn write(&self, data: &str) {
        self.writes.lock().unwrap().push(data.to_string());
    }
    fn columns(&self) -> u16 {
        100
    }
    fn rows(&self) -> u16 {
        30
    }
    fn hide_cursor(&self) {}
    fn show_cursor(&self) {}
    fn start(
        &mut self,
        _on_input: Box<dyn FnMut(String) + Send + 'static>,
        _on_resize: Box<dyn FnMut() + Send + 'static>,
    ) -> std::io::Result<()> {
        Ok(())
    }
    fn stop(&mut self) {}
    fn drain_input(&mut self, _max_ms: u64, _idle_ms: u64) {}
    fn set_exit_signal_callback(&mut self, _cb: Option<Box<dyn FnMut() + Send + 'static>>) {}
}

/// `/sandbox`, typed key by key exactly as an interactive session delivers it,
/// must open the tier/permission card on the *real* platform discovery path.
///
/// The observable is `App::next_deadline`: opening the card requests a repaint,
/// and a repaint needs a deadline, while a fresh app has none until something
/// asks to be redrawn (`overlay_stack`/`top_overlay_text` are crate-private, and
/// a painted card would need `App::start` to have run — which blocks until an
/// agent answers). The `UiCmd::Submit` the enter key emits is asserted too, so
/// the deadline cannot come from an unrelated repaint request.
#[tokio::test(flavor = "multi_thread")]
async fn the_sandbox_panel_opens_through_the_real_platform_twin() {
    let dir = tempfile::tempdir().expect("tempdir");
    let writes = Arc::new(Mutex::new(Vec::new()));
    let (op_tx, mut op_rx) = tokio::sync::mpsc::unbounded_channel();
    let (client, _events, _conn) = GrpcClient::new("127.0.0.1:1");
    let mut app = App::new(
        RecordingTerminal {
            writes: writes.clone(),
        },
        Arc::new(client),
        op_tx,
        &CliOptions::default(),
        dir.path().join("settings.json"),
    );

    let before = app.next_deadline();
    // One character per call: `handle_input` parses *keys*, and multi-character
    // plain text is the paste path.
    for ch in "/sandbox".chars() {
        app.handle_input(&ch.to_string());
    }
    app.handle_input("\r");

    // The enter key hands the line to the app loop rather than handling it
    // inline, so the test has to be that loop.
    let mut submit: Option<String> = None;
    while let Ok(cmd) = op_rx.try_recv() {
        if let UiCmd::Submit(value) = &cmd {
            submit = Some(value.clone());
        }
        app.handle_cmd(cmd);
    }
    assert_eq!(
        submit.as_deref(),
        Some("/sandbox"),
        "the enter key must emit the submitted line"
    );

    let after = app.next_deadline();
    assert!(
        after.is_some(),
        "opening the card must schedule the repaint (before={before:?}, after={after:?})"
    );
    // The card is painted only once `App::start` has set `running`, which waits
    // for a reachable agent; what this test can say is that the render path was
    // reached at all (nothing is written while no screen has been entered).
    assert!(
        writes.lock().unwrap().is_empty(),
        "no paint before the app is started"
    );
}
