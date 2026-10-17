//! The iMessage send path's *success* half, with the AppleScript runner
//! replaced by a stub.
//!
//! `IMessageSender::send_text` returns `Ok` only when `osascript` exits zero, so
//! covering that tail used to mean running the real thing against Messages.app —
//! which, unlike the module's other "either outcome" tests, sends a real iMessage
//! to a real handle. A `PATH` shim that exits zero reaches the product's own
//! success path with nothing sent, and is the ordinary way to make an external
//! tool succeed in a test.
//!
//! `PATH` is process-global, so this test owns its process: it is the only test
//! in this binary, and it puts the original value back before it asserts.

#![cfg(target_os = "macos")]

use future_channel::bridge::{ChatKind, ConversationRef, ProviderCtx};
use future_channel::providers::imessage;

/// Replace `osascript` with a stub for the duration of one send.
struct OsascriptShim {
    dir: std::path::PathBuf,
    original_path: String,
}

impl OsascriptShim {
    fn install() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "future-channel-osascript-shim-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create the shim directory");
        let exe = dir.join("osascript");
        std::fs::write(&exe, "#!/bin/sh\nexit 0\n").expect("write the stub");
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))
                .expect("make the stub executable");
        }
        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", dir.display(), original_path));
        Self { dir, original_path }
    }
}

impl Drop for OsascriptShim {
    fn drop(&mut self) {
        std::env::set_var("PATH", &self.original_path);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_zero_exit_osascript_is_a_successful_send() {
    let provider = imessage::provider();
    let ctx = ProviderCtx::offline(provider.definition());
    // Built before the shim so a failure here cannot be blamed on `PATH`.
    let sender = provider
        .sender(&ctx)
        .expect("macOS builds an iMessage sender");

    let conversation = ConversationRef {
        id: "+15551230000".into(),
        thread_id: None,
        kind: ChatKind::Direct,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let outcome = {
        let _shim = OsascriptShim::install();
        runtime.block_on(sender.send_text(&conversation, "coverage probe"))
    };

    // No id: the AppleScript transport reports the send, not a message id.
    assert_eq!(
        outcome.expect("a zero-exit osascript must be reported as success"),
        None
    );
}
