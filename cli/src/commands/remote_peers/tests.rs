//! Tests for `future remote peers`.
//!
//! Two halves, tested separately because they answer different questions: the
//! book on disk (what is paired) and the running Desktop (what is connected).
//! Every test runs against a throwaway home, so a machine's real pairings are
//! never read, written or revoked.

use super::*;
use crate::output::Output;
use crate::test_env::EnvGuard;
use serde_json::json;

async fn run(args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (out, capture) = Output::memory();
    let code = crate::dispatch(&args, &out).await;
    let stdout = String::from_utf8(capture.out.lock().unwrap().clone()).unwrap();
    let stderr = String::from_utf8(capture.err.lock().unwrap().clone()).unwrap();
    (code, stdout, stderr)
}

/// A throwaway home, with `FUTURE_HOME` cleared so an ambient one cannot
/// redirect the test at the real installation.
struct Home {
    dir: tempfile::TempDir,
    _home: EnvGuard,
    _future_home: EnvGuard,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);
        let future_home = EnvGuard::remove(&["FUTURE_HOME"]);
        Home {
            dir,
            _home: home,
            _future_home: future_home,
        }
    }

    fn future(&self) -> PathBuf {
        self.dir.path().join(".future")
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::create_dir_all(self.future()).unwrap();
        std::fs::write(self.future().join(name), text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.future().join(name)).unwrap()
    }

    fn exists(&self, name: &str) -> bool {
        self.future().join(name).exists()
    }
}

/// A user NKey seed, as the pairing stores one: the revoke path parses it, so a
/// placeholder would test a different failure than the one meant.
fn seed() -> String {
    nkeys::KeyPair::new_user().seed().unwrap()
}

const ONE: &str = "desktop_AAAABBBBCCCCDDDD";
const TWO: &str = "desktop_EEEEFFFFGGGGHHHH";

/// A book with two pairings: one named, one not, both with usable-shaped
/// credentials.
fn book(token_url: &str) -> Value {
    json!({
        "peers": [
            {
                "pairId": "pair_ONE",
                "desktopId": ONE,
                "deviceId": "dev_1",
                "nkeySeed": seed(),
                "userJwt": "jwt-one",
                "refreshToken": "refresh-one",
                "natsUrl": "nats://10.0.0.1:4222",
                "natsWsUrl": "wss://10.0.0.1/ws",
                "jwtExpiresAt": 4_000_000_000i64,
                "tokenUrl": token_url,
                "name": "work",
            },
            {
                "pairId": "pair_TWO",
                "desktopId": TWO,
                "deviceId": "dev_1",
                "nkeySeed": seed(),
                "userJwt": "jwt-two",
                "refreshToken": "refresh-two",
                "natsUrl": "nats://10.0.0.2:4222",
                "natsWsUrl": "wss://10.0.0.2/ws",
                "jwtExpiresAt": 4_000_000_000i64,
                "tokenUrl": token_url,
            }
        ]
    })
}

const PLATFORM_TOKEN: &str = "https://future-os.cn/client/v1/remote/auth/token";

/// Hold the Desktop's own lock for as long as this lives, so the commands that
/// ask a Desktop have one to ask - the same lock file `instance.rs` holds.
struct RunningDesktop {
    _file: std::fs::File,
}

impl RunningDesktop {
    fn claim(home: &Home) -> Self {
        let app = home.future().join("app");
        std::fs::create_dir_all(&app).unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(app.join("desktop.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&file).unwrap();
        RunningDesktop { _file: file }
    }
}

/// Answer the next request the way the Desktop's control host does, and hand
/// back what was asked so a test can judge it.
fn answer_requests(home: &Home, answer: Value) -> std::sync::Arc<std::sync::Mutex<Vec<Value>>> {
    let directory = home.future().join("run").join("remote-control");
    std::fs::create_dir_all(&directory).unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    tokio::spawn(async move {
        loop {
            let mut entries: Vec<_> = std::fs::read_dir(&directory)
                .map(|entries| entries.filter_map(Result::ok).collect())
                .unwrap_or_default();
            entries.sort_by_key(|entry| entry.file_name());
            if let Some(entry) = entries.into_iter().find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".request.json")
            }) {
                let request: Value =
                    serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
                std::fs::write(
                    request["reply"].as_str().expect("a reply path"),
                    answer.to_string(),
                )
                .unwrap();
                recorded.lock().unwrap().push(request);
                let _ = std::fs::remove_file(entry.path());
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    seen
}

/// One peer row as the Desktop reports it.
fn live_row(desktop_id: &str, connected: bool) -> Value {
    json!({
        "desktopId": desktop_id,
        "pairId": if desktop_id == ONE { "pair_ONE" } else { "pair_TWO" },
        "name": "work",
        "connected": connected,
        "stopped": false,
        "agentAvailable": connected,
    })
}

/// A live row another paired device is driving, as the Desktop reports it.
fn held_row(desktop_id: &str, holder: Option<&str>) -> Value {
    let mut row = live_row(desktop_id, true);
    row["control"] = json!("standby");
    if let Some(name) = holder {
        row["holder"] = json!({ "pairId": "pair_phone", "name": name, "kind": "phone" });
    }
    row
}

// -- list -------------------------------------------------------------------

/// "Connected" alone would claim a machine usable whose every command comes back
/// `superseded`: the row says who has it, and the JSON says it too.
#[tokio::test]
async fn list_names_the_device_a_machine_is_held_by() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(
        &home,
        json!({ "peers": [held_row(ONE, Some("phone")), held_row(TWO, None)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("taken over by phone"), "{stdout}");
    assert!(stdout.contains("taken over"), "{stdout}");
    // Neither held row is reported as plain "connected": that word is what a
    // reader takes as usable.
    for line in stdout.lines() {
        if line.contains(ONE) || line.contains(TWO) {
            assert!(
                line.contains("taken over"),
                "a held machine must not read as connected: {line}"
            );
        }
    }

    // A second ask needs a second answerer: the fake Desktop serves one request
    // and is gone, which is also the "silent Desktop" the JSON path must not
    // invent state for.
    answer_requests(
        &home,
        json!({ "peers": [held_row(ONE, Some("phone")), held_row(TWO, None)] }),
    );
    let (code, stdout, stderr) = run(&["remote", "peers", "--json"]).await;
    assert_eq!(code, 0, "{stderr}");
    let value: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["peers"][0]["control"], "standby");
    assert_eq!(value["peers"][0]["holder"]["name"], "phone");
    assert_eq!(value["peers"][0]["holder"]["kind"], "phone");
    // A holder that never said who it is leaves the object out of the row.
    assert!(value["peers"][1]["holder"].is_null(), "{value}");
}

#[tokio::test]
async fn list_reads_the_book_and_never_prints_a_credential() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    let book = book(PLATFORM_TOKEN);
    let secret = book["peers"][0]["nkeySeed"].as_str().unwrap().to_string();
    home.write(BOOK_FILE, &book.to_string());

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("2 remote desktops are paired"), "{stdout}");
    assert!(stdout.contains(ONE), "{stdout}");
    assert!(stdout.contains(TWO), "{stdout}");
    assert!(stdout.contains("work"), "{stdout}");
    // No Desktop was asked, and the book cannot know: that is the sentence.
    assert!(stdout.contains("No Desktop is running"), "{stdout}");
    assert!(stdout.contains("not connected"), "{stdout}");
    // Credential material stays in the file.
    assert!(!stdout.contains(&secret), "the seed was printed: {stdout}");
    assert!(!stdout.contains("jwt-one"), "{stdout}");
    assert!(!stdout.contains("refresh-one"), "{stdout}");
}

#[tokio::test]
async fn list_reports_the_state_the_running_desktop_answered_with() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    let seen = answer_requests(
        &home,
        json!({ "peers": [live_row(ONE, true), live_row(TWO, false)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("connected"), "{stdout}");
    assert!(stdout.contains("not connected"), "{stdout}");
    let posed = seen.lock().unwrap();
    assert_eq!(posed[0]["op"], "peer-state", "{}", posed[0]);
    assert!(posed[0].get("desktopId").is_none(), "{}", posed[0]);
}

#[tokio::test]
async fn list_says_the_state_is_unknown_when_the_desktop_stays_silent() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("unknown"), "{stdout}");
    assert!(stderr.contains("did not answer"), "{stderr}");
}

/// A Desktop that reported a fault is not the same as one that stayed silent:
/// the note carries the Desktop's own reason, and the stored pairings are still
/// listed — a broken Desktop does not empty the list.
#[tokio::test]
async fn list_notes_a_desktop_that_reports_a_fault() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(&home, json!({ "error": "the Remote page is not open" }));

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains("did not report its connections"),
        "{stderr}"
    );
    assert!(
        stderr.contains("the Remote page is not open"),
        "the Desktop's own words: {stderr}"
    );
    assert!(stdout.contains(ONE), "{stdout}");
    assert!(stdout.contains(TWO), "{stdout}");
}

#[tokio::test]
async fn list_json_keeps_unknown_state_null_rather_than_guessing() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, stdout, stderr) = run(&["remote", "peers", "list", "--json"]).await;
    assert_eq!(code, 0, "{stderr}");
    let value: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["desktopRunning"], false);
    assert_eq!(value["answered"], false);
    assert_eq!(value["peers"][0]["desktopId"], ONE);
    assert_eq!(value["peers"][0]["name"], "work");
    assert!(value["peers"][0]["connected"].is_null(), "{value}");
    assert!(value["peers"][0]["stopped"].is_null(), "{value}");
    assert!(value["peers"][0]["agentAvailable"].is_null(), "{value}");
    // Control is the Desktop's to report as well: the book cannot know whether
    // another device is driving the machine.
    assert!(value["peers"][0]["control"].is_null(), "{value}");
    assert!(value["peers"][0]["holder"].is_null(), "{value}");
}

/// A book with one pairing is singular in the sentence, and a row the Desktop
/// reported with an error says so: the machine is listed and not usable, which
/// is the case a reader most needs spelled out.
#[tokio::test]
async fn list_one_pairing_is_singular_and_carries_the_last_error() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    let mut book = book(PLATFORM_TOKEN);
    book["peers"].as_array_mut().unwrap().truncate(1);
    home.write(BOOK_FILE, &book.to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(
        &home,
        json!({ "peers": [{
            "desktopId": ONE,
            "pairId": "pair_ONE",
            "connected": false,
            "stopped": false,
            "agentAvailable": false,
            "error": "handshake timed out",
        }] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("1 remote desktop is paired"), "{stdout}");
    assert!(!stdout.contains("remote desktops are"), "{stdout}");
    assert!(
        stdout.contains("last error: handshake timed out"),
        "{stdout}"
    );
}

#[tokio::test]
async fn an_empty_book_says_so_and_points_at_add() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("not paired with any remote desktop"),
        "{stdout}"
    );
    assert!(stdout.contains("future remote peers add"), "{stdout}");
    // A read invents nothing on disk.
    assert!(!home.exists(BOOK_FILE), "{}", home.future().display());
}

// -- connect / disconnect ---------------------------------------------------

#[tokio::test]
async fn connect_names_the_machine_it_asks_about() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    let seen = answer_requests(&home, json!({ "peers": [live_row(ONE, true)] }));

    let (code, stdout, stderr) = run(&["remote", "peers", "connect", "AAAA"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("is connected"), "{stdout}");
    let posed = seen.lock().unwrap();
    assert_eq!(posed[0]["op"], "peer-connect", "{}", posed[0]);
    assert_eq!(posed[0]["desktopId"], ONE, "{}", posed[0]);
    assert!(posed[0].get("pairId").is_none(), "{}", posed[0]);
}

#[tokio::test]
async fn connect_reports_a_failed_attempt_and_exits_one() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(
        &home,
        json!({ "peers": [{
            "desktopId": ONE,
            "pairId": "pair_ONE",
            "connected": false,
            "stopped": false,
            "agentAvailable": false,
            "error": "handshake timed out",
        }] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "connect", "#1"]).await;
    assert_eq!(code, 1, "it did not connect: {stdout}");
    assert!(stdout.contains("still not connected"), "{stdout}");
    assert!(stdout.contains("handshake timed out"), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[tokio::test]
async fn disconnect_reports_the_state_the_desktop_answered_with() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    let seen = answer_requests(
        &home,
        json!({ "peers": [{
            "desktopId": ONE,
            "pairId": "pair_ONE",
            "connected": false,
            "stopped": true,
            "agentAvailable": false,
        }] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "disconnect", ONE]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("is disconnected"), "{stdout}");
    assert!(stdout.contains("The pairing stays"), "{stdout}");
    let posed = seen.lock().unwrap();
    assert_eq!(posed[0]["op"], "peer-disconnect", "{}", posed[0]);
    assert_eq!(posed[0]["desktopId"], ONE, "{}", posed[0]);
}

#[tokio::test]
async fn connect_without_a_desktop_says_there_is_nothing_to_act_on() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, _, stderr) = run(&["remote", "peers", "connect", ONE]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("No Desktop is running"), "{stderr}");
    // Nothing was asked of a Desktop that is not there, and the book is intact.
    assert!(!home.exists("run"), "{}", home.future().display());
}

/// Nothing is paired, so there is no machine to act on: the refusal is the same
/// for every action that takes one, and each names the command that pairs one.
#[tokio::test]
async fn an_action_on_an_empty_book_names_the_paired_command() {
    let _guard = crate::test_env::lock_env().await;
    let _home = Home::new();

    for (args, expected) in [
        (
            vec!["remote", "peers", "connect", ONE],
            "future remote peers add",
        ),
        (
            vec!["remote", "peers", "disconnect", "#1"],
            "future remote peers add",
        ),
        (
            vec!["remote", "peers", "remove", ONE, "--yes"],
            "not paired with any remote desktop",
        ),
    ] {
        let (code, _, stderr) = run(&args).await;
        assert_eq!(code, 1, "{args:?}");
        assert!(stderr.contains(expected), "{args:?} → {stderr}");
    }
}

/// Disconnect's own note: unlike connect, it has no reason to add that an
/// unstarted Desktop says nothing about the pairing — the machine is paired and
/// being connected is the pairing working.
#[tokio::test]
async fn disconnect_without_a_desktop_keeps_its_own_words() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, _, stderr) = run(&["remote", "peers", "disconnect", ONE]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("nothing to disconnect here"), "{stderr}");
    assert!(
        !stderr.contains("not a fact about the pairing"),
        "connect's clause does not belong in disconnect's sentence: {stderr}"
    );
    // The book is untouched by a Desktop that is not there.
    assert!(!home.exists("run"), "{}", home.future().display());
}

/// A Desktop that refuses in its own words is quoted, not summarised: the
/// refusal is the only thing that says what to change.
#[tokio::test]
async fn connect_reports_a_refusal_in_the_desktops_own_words() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(&home, json!({ "error": "superseded" }));

    let (code, _, stderr) = run(&["remote", "peers", "connect", ONE]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("could not connect it"), "{stderr}");
    assert!(stderr.contains("superseded"), "{stderr}");
}

/// `--json` prints what the Desktop answered, not the row the book guessed.
#[tokio::test]
async fn connect_json_carries_the_list_the_desktop_returned() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(
        &home,
        json!({ "peers": [live_row(ONE, true), live_row(TWO, false)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "connect", "AAAA", "--json"]).await;
    assert_eq!(code, 0, "{stderr}");
    // The document comes first and the sentence a reader needs after it, so the
    // parse stops at the document rather than at the end of stdout.
    let mut documents = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
    let value = documents.next().expect("a document").unwrap();
    assert_eq!(value["desktopRunning"], true);
    assert_eq!(value["answered"], true);
    assert_eq!(value["peers"][0]["desktopId"], ONE);
    assert_eq!(value["peers"][0]["connected"], true);
    assert_eq!(value["peers"][1]["connected"], false);
    assert!(
        stdout.contains("is connected."),
        "--json does not take the sentence away: {stdout}"
    );
}

/// A Desktop that kept the connection did not do what it was asked, so the exit
/// code says so even though it answered.
#[tokio::test]
async fn disconnect_says_when_the_desktop_kept_it_connected() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(&home, json!({ "peers": [live_row(ONE, true)] }));

    let (code, stdout, stderr) = run(&["remote", "peers", "disconnect", ONE]).await;
    assert_eq!(code, 1, "{stdout} {stderr}");
    assert!(
        stdout.contains("is still connected: the Desktop did not drop it"),
        "{stdout}"
    );
    assert!(stderr.is_empty(), "{stderr}");
}

#[tokio::test]
async fn a_machine_that_is_not_paired_is_refused_with_the_list_hint() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);

    let (code, _, stderr) = run(&["remote", "peers", "connect", "ZZZZ"]).await;
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("No remote desktop matches"), "{stderr}");
    assert!(stderr.contains("future remote peers"), "{stderr}");
}

// -- takeover ---------------------------------------------------------------

#[tokio::test]
async fn takeover_names_the_machine_and_reports_that_control_moved() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    let seen = answer_requests(
        &home,
        json!({ "tookOver": true, "peers": [live_row(ONE, true)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "takeover", "AAAA"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("is now driven by this machine"), "{stdout}");
    let posed = seen.lock().unwrap();
    assert_eq!(posed[0]["op"], "peer-takeover", "{}", posed[0]);
    assert_eq!(posed[0]["desktopId"], ONE, "{}", posed[0]);
    // The two roles' identifiers travel in their own fields, always.
    assert!(posed[0].get("pairId").is_none(), "{}", posed[0]);
}

/// A machine this one already drove answers that nothing moved. That is a
/// result, not a refusal, and the command must not exit 1 over it.
#[tokio::test]
async fn takeover_says_nothing_moved_when_it_already_had_control() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(
        &home,
        json!({ "tookOver": false, "peers": [live_row(ONE, true)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "takeover", "#1"]).await;
    assert_eq!(code, 0, "{stdout} {stderr}");
    assert!(
        stdout.contains("already driven by this machine"),
        "{stdout}"
    );
    assert!(stdout.contains("nothing moved"), "{stdout}");
}

#[tokio::test]
async fn takeover_json_carries_the_move_and_the_list() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(
        &home,
        json!({ "tookOver": true, "peers": [live_row(ONE, true)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "takeover", ONE, "--json"]).await;
    assert_eq!(code, 0, "{stderr}");
    let value: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["tookOver"], true);
    assert_eq!(value["answered"], true);
    assert_eq!(value["peers"][0]["desktopId"], ONE);
}

/// A refusal is the Desktop's own sentence, not a cheerful exit 0: the machine
/// stayed with the device that had it.
#[tokio::test]
async fn takeover_reports_a_refusal_by_name() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(&home, json!({ "error": "superseded" }));

    let (code, _, stderr) = run(&["remote", "peers", "takeover", ONE]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("could not take it over"), "{stderr}");
    assert!(stderr.contains("superseded"), "{stderr}");
}

#[tokio::test]
async fn takeover_without_a_desktop_says_there_is_nothing_to_act_on() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, _, stderr) = run(&["remote", "peers", "takeover", ONE]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("No Desktop is running"), "{stderr}");
    // Nothing was asked of a Desktop that is not there.
    assert!(!home.exists("run"), "{}", home.future().display());
}

// -- rename -----------------------------------------------------------------

#[tokio::test]
async fn rename_writes_the_label_and_keeps_the_credential() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let before: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    let secret = before["peers"][1]["nkeySeed"].as_str().unwrap().to_string();

    let (code, stdout, stderr) = run(&["remote", "peers", "rename", "EEEE", "laptop"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("laptop"), "{stdout}");
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    assert_eq!(after["peers"][1]["name"], "laptop");
    // Everything this command does not model survives the edit, including the
    // credentials it must never lose.
    assert_eq!(after["peers"][1]["nkeySeed"], secret);
    assert_eq!(after["peers"][1]["refreshToken"], "refresh-two");
    assert_eq!(after["peers"][0]["name"], "work");
}

#[tokio::test]
async fn rename_clear_goes_back_to_the_machine_id() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, _, stderr) = run(&["remote", "peers", "rename", "#1", "--clear"]).await;
    assert_eq!(code, 0, "{stderr}");
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    assert!(after["peers"][0].get("name").is_none(), "{after}");

    let (code, _, stderr) = run(&["remote", "peers", "rename", "#1"]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("What should it be called"), "{stderr}");
}

#[tokio::test]
async fn a_row_number_and_a_case_insensitive_prefix_name_the_same_machine() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, _, stderr) = run(&["remote", "peers", "rename", "#2", "first"]).await;
    assert_eq!(code, 0, "{stderr}");
    let (code, _, stderr) = run(&["remote", "peers", "rename", "eeee", "second"]).await;
    assert_eq!(code, 0, "{stderr}");
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    assert_eq!(after["peers"][1]["name"], "second");
    assert_eq!(after["peers"][0]["name"], "work");
}

/// Both halves of the instruction are refused when only one is given: a rename
/// with nothing to name asks which machine, and `--clear` beside a name is two
/// answers to one question.
#[tokio::test]
async fn rename_names_what_it_needs_before_touching_the_book() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    let before = book(PLATFORM_TOKEN);
    home.write(BOOK_FILE, &before.to_string());

    let (code, _, stderr) = run(&["remote", "peers", "rename"]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("Which machine?"), "{stderr}");

    let (code, _, stderr) = run(&["remote", "peers", "rename", ONE, "laptop", "--clear"]).await;
    assert_eq!(code, 1);
    assert!(
        stderr.contains("--clear goes back to the machine id"),
        "{stderr}"
    );
    // Neither refusal wrote anything.
    assert_eq!(home.read(BOOK_FILE), before.to_string());
}

/// The JSON form re-reads the book after the edit, so a script sees the label it
/// just set rather than the one from before.
#[tokio::test]
async fn rename_json_prints_the_book_it_left_behind() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, stdout, stderr) =
        run(&["remote", "peers", "rename", "EEEE", "laptop", "--json"]).await;
    assert_eq!(code, 0, "{stderr}");
    let value: Value = serde_json::from_str(&stdout).unwrap();
    // No Desktop was asked, and the document says which half is missing.
    assert_eq!(value["desktopRunning"], false);
    assert_eq!(value["peers"][1]["desktopId"], TWO);
    assert_eq!(value["peers"][1]["name"], "laptop");
    assert_eq!(value["peers"][0]["name"], "work");
}

// -- remove -----------------------------------------------------------------

#[tokio::test]
async fn remove_asks_the_desktop_first_and_takes_its_answer() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    let seen = answer_requests(
        &home,
        json!({ "removed": true, "peers": [live_row(TWO, false)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "remove", "AAAA", "--yes"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("no longer paired"), "{stdout}");
    let posed = seen.lock().unwrap();
    assert_eq!(posed[0]["op"], "peer-remove", "{}", posed[0]);
    assert_eq!(posed[0]["desktopId"], ONE, "{}", posed[0]);
}

#[tokio::test]
async fn remove_without_yes_refuses_a_prompt_nobody_saw() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());

    let (code, _, stderr) = run(&["remote", "peers", "remove", ONE]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("Refusing to unpair"), "{stderr}");
    // Nothing was removed: the book still has both.
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    assert_eq!(after["peers"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn remove_with_no_desktop_unpairs_here_and_revokes_where_it_can() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    let server = crate::test_server::spawn_http(vec![
        crate::test_server::HttpRoute::json("/client/v1/remote/auth/token", 200, "{}"),
        crate::test_server::HttpRoute::json("/client/v1/remote/pair/revoke", 200, "{}"),
    ])
    .await;
    let token_url = format!("{server}/client/v1/remote/auth/token");
    home.write(BOOK_FILE, &book(&token_url).to_string());

    let (code, stdout, stderr) = run(&["remote", "peers", "remove", TWO, "--yes"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("no longer paired"), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    let remaining = after["peers"].as_array().unwrap();
    assert_eq!(remaining.len(), 1, "{after}");
    assert_eq!(remaining[0]["desktopId"], ONE, "{after}");
}

/// The Desktop removed it, and the JSON says so with the book it left behind:
/// this command must not remove it a second time and then report a pairing that
/// is already gone.
#[tokio::test]
async fn remove_json_reports_the_desktop_that_did_it() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, &book(PLATFORM_TOKEN).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(
        &home,
        json!({ "removed": true, "peers": [live_row(TWO, false)] }),
    );

    let (code, stdout, stderr) = run(&["remote", "peers", "remove", ONE, "--yes", "--json"]).await;
    assert_eq!(code, 0, "{stderr}");
    // This branch prints the JSON *and* the closing sentence, unlike the paths
    // that return straight after the document, so the answer is read from the
    // front of stdout rather than from all of it.
    let value: Value = serde_json::Deserializer::from_str(&stdout)
        .into_iter::<Value>()
        .next()
        .expect("a JSON document was printed")
        .unwrap();
    assert_eq!(value["removed"], ONE);
    assert_eq!(value["answered"], true);
    assert_eq!(value["desktopRunning"], true);
    let peers = value["peers"].as_array().unwrap();
    assert_eq!(peers.len(), 1, "{value}");
    assert_eq!(peers[0]["desktopId"], TWO, "{value}");
}

/// The Desktop answered and declined — it has no such pairing, or a newer build
/// refused. This command takes it out here anyway and names the Desktop's own
/// reason, so the reader can tell "not connected" from "refused".
#[tokio::test]
async fn remove_takes_the_pairing_out_here_when_the_desktop_declines() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    let server = crate::test_server::spawn_http(vec![
        crate::test_server::HttpRoute::json("/client/v1/remote/auth/token", 200, "{}"),
        crate::test_server::HttpRoute::json("/client/v1/remote/pair/revoke", 200, "{}"),
    ])
    .await;
    let token_url = format!("{server}/client/v1/remote/auth/token");
    home.write(BOOK_FILE, &book(&token_url).to_string());
    let _desktop = RunningDesktop::claim(&home);
    answer_requests(&home, json!({ "error": "superseded" }));

    let (code, stdout, stderr) = run(&["remote", "peers", "remove", TWO, "--yes"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("no longer paired"), "{stdout}");
    assert!(stderr.contains("did not remove it"), "{stderr}");
    assert!(
        stderr.contains("superseded"),
        "the Desktop's own reason: {stderr}"
    );
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    assert_eq!(after["peers"].as_array().unwrap().len(), 1, "{after}");
}

/// The local pairing goes first, always: a platform that could not be told is a
/// warning that says what the other machine still shows, not a failure — and the
/// JSON still reports the book this machine is left with.
#[tokio::test]
async fn remove_says_when_the_platform_could_not_be_told() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    let server = crate::test_server::spawn_http(vec![
        crate::test_server::HttpRoute::json("/client/v1/remote/auth/token", 200, "{}"),
        crate::test_server::HttpRoute::json("/client/v1/remote/pair/revoke", 500, "{}"),
    ])
    .await;
    let token_url = format!("{server}/client/v1/remote/auth/token");
    home.write(BOOK_FILE, &book(&token_url).to_string());

    let (code, stdout, stderr) = run(&["remote", "peers", "remove", TWO, "--yes", "--json"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("could not be told"), "{stderr}");
    assert!(stderr.contains("credential expires"), "{stderr}");
    let value: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["removed"], TWO);
    assert_eq!(value["answered"], false);
    assert_eq!(value["desktopRunning"], false);
    let peers = value["peers"].as_array().unwrap();
    assert_eq!(peers.len(), 1, "{value}");
    assert_eq!(peers[0]["desktopId"], ONE, "{value}");
}

// -- help and arguments -----------------------------------------------------

#[tokio::test]
async fn the_group_and_each_action_have_their_own_help() {
    let _guard = crate::test_env::lock_env().await;
    let _home = Home::new();

    let (code, stdout, _) = run(&["remote", "peers", "--help"]).await;
    assert_eq!(code, 0);
    assert!(stdout.contains("future remote peers"), "{stdout}");
    assert!(
        stdout.contains("peer-state") || stdout.contains("client role"),
        "{stdout}"
    );
    for action in [
        "list",
        "add",
        "connect",
        "disconnect",
        "takeover",
        "rename",
        "remove",
    ] {
        let (code, stdout, _) = run(&["remote", "peers", action, "--help"]).await;
        assert_eq!(code, 0, "{action}");
        assert!(stdout.contains(action), "{action}: {stdout}");
    }
    // The group is named by `future remote --help` too, or nobody finds it.
    let (code, stdout, _) = run(&["remote", "--help"]).await;
    assert_eq!(code, 0);
    assert!(stdout.contains("future remote peers"), "{stdout}");
}

#[tokio::test]
async fn add_explains_what_it_wants_before_touching_the_network() {
    let _guard = crate::test_env::lock_env().await;
    let _home = Home::new();

    let (code, _, stderr) = run(&["remote", "peers", "add"]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("Which invitation?"), "{stderr}");

    let (code, _, stderr) = run(&["remote", "peers", "add", "a", "b"]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("One invitation at a time"), "{stderr}");
}

/// A platform-minted invitation is claimed with this machine's account. The host
/// reports that as the typed `NotSignedIn` fault, and this is the only place with
/// room to turn it into the command that fixes it: "Not signed in to FutureOS."
/// on its own sends the reader looking at the link instead of the account.
#[test]
fn a_signed_out_pairing_names_the_login_command() {
    let message = describe_pairing_error(&future_host::Error::NotSignedIn);
    assert!(message.contains("future auth login"), "{message}");
    assert!(message.contains("self-hosted relay"), "{message}");
    // Every other failure keeps its own advice: the login hint must not become
    // the generic answer for a broken link.
    let other = describe_pairing_error(&future_host::Error::Message("boom".into()));
    assert!(!other.contains("future auth login"), "{other}");
}

#[tokio::test]
async fn an_unknown_action_is_a_usage_error() {
    let _guard = crate::test_env::lock_env().await;
    let _home = Home::new();

    let (code, _, stderr) = run(&["remote", "peers", "poke"]).await;
    assert_eq!(code, 1);
    assert!(stderr.contains("Unknown argument: poke"), "{stderr}");
    assert!(stderr.contains("future remote peers"), "{stderr}");
}

// -- the book on disk -------------------------------------------------------

/// Every way the pairing file can be wrong is refused with the file named, and
/// none of them is repaired by overwriting it: this file holds credentials, so a
/// command that cannot read it must not replace it.
#[tokio::test]
async fn an_unusable_pairing_file_is_named_and_left_alone() {
    let _guard = crate::test_env::lock_env().await;
    for (contents, expected) in [
        ("{ not json", "is not valid JSON"),
        ("[]", "The root of"),
        (r#"{"peers": "one"}"#, "`peers` must be an array"),
        (r#"{"peers": ["desktop_AAAA"]}"#, "must be a JSON object"),
        (
            r#"{"peers": [{"pairId": "pair_ONE"}]}"#,
            "has no `desktopId`",
        ),
    ] {
        let home = Home::new();
        home.write(BOOK_FILE, contents);
        let (code, _, stderr) = run(&["remote", "peers"]).await;
        assert_eq!(code, 1, "{contents}");
        assert!(stderr.contains(expected), "{contents} → {stderr}");
        // A pairing file that could not be read is still the only copy of those
        // credentials: the refusal must leave it byte for byte as it was.
        assert_eq!(home.read(BOOK_FILE), contents);
    }
}

/// A book with no `peers` key at all is an empty book, not a broken one: an
/// installation that has never paired anything, or a file another tool wrote.
#[tokio::test]
async fn a_book_without_a_peer_list_is_empty_rather_than_broken() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    home.write(BOOK_FILE, "{}");

    let (code, stdout, stderr) = run(&["remote", "peers"]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("not paired with any remote desktop"),
        "{stdout}"
    );
}

/// Two machines that share the start of their id cannot both answer to it: the
/// refusal names how many matched and asks for more, instead of picking one.
#[tokio::test]
async fn an_ambiguous_prefix_asks_for_a_longer_one() {
    let _guard = crate::test_env::lock_env().await;
    let home = Home::new();
    let mut book = book(PLATFORM_TOKEN);
    book["peers"][1]["desktopId"] = json!("desktop_AAAACCCCDDDDEEEE");
    home.write(BOOK_FILE, &book.to_string());

    let (code, _, stderr) = run(&["remote", "peers", "rename", "aaaa", "first"]).await;
    assert_eq!(code, 1);
    assert!(
        stderr.contains("`aaaa` matches 2 machines. Give a longer prefix."),
        "{stderr}"
    );
    // Nothing was renamed by a selector that did not name one machine.
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    assert_eq!(after["peers"][0]["name"], json!("work"));
    assert!(after["peers"][1].get("name").is_none(), "{after}");

    // The longer prefix does name one.
    let (code, _, stderr) = run(&["remote", "peers", "rename", "aaaacccc", "second"]).await;
    assert_eq!(code, 0, "{stderr}");
    let after: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
    assert_eq!(after["peers"][1]["name"], json!("second"));
}

/// `list` is a list, so a machine given to it is a mistake worth saying out
/// loud rather than one row silently answered.
#[tokio::test]
async fn list_refuses_to_be_given_a_machine() {
    let _guard = crate::test_env::lock_env().await;
    let _home = Home::new();

    let (code, _, stderr) = run(&["remote", "peers", "list", ONE]).await;
    assert_eq!(code, 1);
    assert!(
        stderr.contains("`list` takes no machine: it lists them all (drop "),
        "{stderr}"
    );
    assert!(stderr.contains(ONE), "{stderr}");
}

/// A link failure is the user's only clue, so each one that has a fix names it —
/// and a failure with no known fix keeps its own words instead of borrowing
/// advice that does not apply.
#[test]
fn each_link_fault_names_what_to_do_about_it() {
    let remote = |code: Option<&str>, message: &str| future_host::Error::Remote {
        status: 400,
        code: code.map(str::to_string),
        message: message.to_string(),
    };

    let expired = describe_pairing_error(&remote(Some("PA002"), "pairing_link_expired"));
    assert!(expired.contains("The invitation has expired"), "{expired}");
    assert!(expired.contains("PA002"), "{expired}");
    assert!(expired.contains("pairing_link_expired"), "{expired}");

    for message in [
        "pairing_link_malformed",
        "pairing_link_invalid_field",
        "pairing_link_unsupported_version",
    ] {
        let refused = describe_pairing_error(&remote(Some("PA002"), message));
        assert!(
            refused.contains("not a usable `futureos://remote/pair`"),
            "{refused}"
        );
        assert!(refused.contains(message), "{refused}");
    }

    // An unknown fault is not given advice that might be wrong for it...
    let unknown = describe_pairing_error(&remote(None, "something_else"));
    assert!(unknown.starts_with("Pairing failed: "), "{unknown}");
    // ...and a link fault that arrived without a support code still names the
    // default one a person would be asked for.
    let uncoded = describe_pairing_error(&remote(None, "pairing_link_expired"));
    assert!(uncoded.contains("PA002"), "{uncoded}");
}
