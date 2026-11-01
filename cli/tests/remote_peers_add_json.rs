//! `future remote peers add --json`.
//!
//! This arm has no test in the lib's own test binary, and the reason is a
//! property of the command rather than of the test: `add` shuts the peer runtime
//! down when it is done, so one process holds one pairing, and the lib binary
//! already spends its one on the same command's text report
//! (`add_says_which_machine_it_paired_and_who_connects_it`). A second pairing in
//! that process fails with `remote_client_stopped`. Hence a binary of its own —
//! `tests/support/mod.rs` holds the bridge both of them build.
//!
//! What is asserted is the document a script reads: the machine that was
//! paired, that it answered, and the one fact the text report carries in a
//! sentence — whether a Desktop is running here to keep it connected. And that
//! the whole of stdout is *one* document: this is the branch that returns
//! straight after printing, unlike `remote peers remove --json`'s Desktop-removed
//! path (see the note in `docs/testing/coverage-waivers-2026-11/cli.md`).

mod support;

use serde_json::Value;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_json_prints_the_machine_it_paired_and_who_keeps_it_connected() {
    let served = support::Served::start("cli-peers-add-json").await;
    let invitation = served.invitation("refresh-token-add-json");

    let (code, stdout, stderr) =
        support::run(&["remote", "peers", "add", &invitation, "--json"]).await;

    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    // The whole of stdout, not a document cut out of it: this branch returns
    // immediately after printing, so anything after the document is a defect.
    let document: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("one whole JSON document was expected ({error}): {stdout}"));

    assert_eq!(document["desktopRunning"], Value::Bool(false), "{document}");
    let paired = &document["paired"];
    assert_eq!(
        paired["desktopId"],
        Value::String(served.desktop_id.clone()),
        "{document}"
    );
    assert_eq!(paired["connected"], Value::Bool(true), "{document}");
    // The pairing the claim granted is the one on disk, so the row a script
    // stores is the row the next start reads.
    let stored = served.paired_book();
    let pair_id = stored["peers"][0]["pairId"]
        .as_str()
        .unwrap_or_else(|| panic!("the pairing was stored: {stored}"))
        .to_string();
    assert_eq!(paired["pairId"], Value::String(pair_id), "{document}");
    assert_eq!(
        stored["peers"][0]["desktopId"],
        Value::String(served.desktop_id.clone()),
        "{stored}"
    );

    served.finish().await;
}
