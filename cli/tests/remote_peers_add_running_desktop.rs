//! `future remote peers add` with a Desktop running here.
//!
//! The pairing is the same real one as the lib's `add` test and the JSON one in
//! `remote_peers_add_json.rs`; what changes is the last line of the report. A
//! pairing made by this command is taken over by the Desktop's own supervisor, so
//! the sentence a reader gets is the opposite of the "no Desktop is running" one —
//! and saying the wrong half would send someone looking for a window that is
//! already open, or leave them waiting for a connection a live process had
//! already made.
//!
//! Its own binary for the same reason as the JSON file (`tests/support/mod.rs`):
//! `add` shuts the peer runtime down, so one process holds one pairing.

mod support;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_with_a_desktop_running_says_who_keeps_it_connected() {
    let served = support::Served::start("cli-peers-add-desktop").await;
    // Held for the whole test: this is what `desktop_running()` reads.
    let _desktop = support::RunningDesktop::claim(&served.future_home());
    let invitation = served.invitation("refresh-token-add-desktop");

    let (code, stdout, stderr) = support::run(&["remote", "peers", "add", &invitation]).await;

    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains(&served.desktop_id), "{stdout}");
    assert!(stdout.contains("Connected to it now."), "{stdout}");
    assert!(
        stdout.contains("The Desktop running here keeps it connected from now on."),
        "{stdout}"
    );
    assert!(
        !stdout.contains("No Desktop is running"),
        "a running Desktop must not be reported as absent: {stdout}"
    );

    served.finish().await;
}
