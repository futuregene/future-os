//! `future goal ui --open` (`webui/server.rs:88, 102-104`): the dashboard URL is
//! handed to the platform opener, and the drive asserts on *what was opened*
//! rather than paying for it — the opener on PATH is a stub inside this test's
//! fixture directory, never the operator's real browser.
//!
//! In its own test binary because it mutates `PATH`, which is process-global
//! (`tests/executor_env_drive.rs` is the precedent), and behind an RAII guard
//! because a leaked `PATH` fails every later test in the same binary — measured
//! at 27 unrelated failures from one such leak (FLAKES.md 4c / item 27). The two
//! tests also take a lock, because both mutate the same global.
//!
//! `PATH` is set to the fixture directory *alone*, not merely prepended: the real
//! opener (`/usr/bin/open`, `xdg-open`) then cannot be reached at all, so a
//! regression that ignores the stub cannot open a browser on the operator's
//! desktop — and the negative control keeps its meaning, because the only
//! process it could start is the recorder.
//!
//! Coverage note: the positive arm is Unix-only. On Windows the opener is
//! `rundll32 url.dll,FileProtocolHandler …` — a real PE that this unit's write
//! set cannot replace with a recorder, and the real opener must never be
//! launched, so Windows runs the negative control only. The module record says
//! so; the measured number for this row is the macOS one (§11.1).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use future_goal::store::Store;
use future_goal::webui::run_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The name `open_in_browser` resolves on this platform.
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(all(unix, not(target_os = "macos")))]
const OPENER: &str = "xdg-open";

/// Serialises the two tests, which both replace the same process-global.
static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Replaces `PATH` with one directory and restores it on drop — including while
/// a test is unwinding, which is the half FLAKES.md 4c is about.
struct PathGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    saved: Option<std::ffi::OsString>,
}

impl PathGuard {
    fn only(dir: &Path) -> Self {
        let _lock = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", dir);
        Self { _lock, saved }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        match &self.saved {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
}

/// The fixture opener: records its first argument (the URL) and exits. A stub,
/// so the assertion is on the record and no browser is ever asked to run.
#[cfg(unix)]
fn write_opener_stub(dir: &Path, record: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let stub = dir.join(OPENER);
    let mut file = std::fs::File::create(&stub).unwrap();
    file.write_all(
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" > '{}'\n",
            record.display()
        )
        .as_bytes(),
    )
    .unwrap();
    drop(file);
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    stub
}

/// Wait for the opener's record to appear, then return its single line.
async fn opened_url(record: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = std::fs::read_to_string(record) {
            let text = text.trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the platform opener was never asked to open anything ({} absent after 10s)",
            record.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// `http://127.0.0.1:<port>/`, with the port parsed out — the URL shape
/// `run_server` builds must be exact, not merely non-empty.
fn parse_dashboard_url(url: &str) -> u16 {
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .unwrap_or_else(|| panic!("the opener must receive the loopback dashboard URL: {url}"));
    let port = rest
        .strip_suffix('/')
        .unwrap_or_else(|| panic!("the dashboard URL must end in a single `/`: {url}"));
    port.parse()
        .unwrap_or_else(|_| panic!("the dashboard URL must carry a numeric port: {url}"))
}

/// `--open` must hand the opener the URL of the port THIS server actually bound,
/// once, and that port must answer the dashboard's HTTP surface.
#[cfg(unix)]
#[tokio::test]
async fn open_browser_hands_the_platform_opener_the_live_dashboard_url() {
    let fixture = tempfile::tempdir().unwrap();
    let record = fixture.path().join("opened.txt");
    write_opener_stub(fixture.path(), &record);
    // `PATH` = the fixture alone: only the recorder is reachable as `open`.
    let _path = PathGuard::only(fixture.path());

    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().to_string_lossy().into_owned();
    Store::open(&root_path).unwrap();
    // Port 0: the OS picks one and `run_server` rewrites `port` from the bound
    // listener, so the URL in the record cannot be predicted by the test — it
    // has to come from the server.
    let server = tokio::spawn(async move { run_server(root_path, 0, true).await });

    let url = opened_url(&record).await;
    let port = parse_dashboard_url(&url);
    assert_eq!(
        std::fs::read_to_string(&record).unwrap().lines().count(),
        1,
        "the opener is asked once per server start: {url}"
    );

    // The recorded port is this server's own: it answers the dashboard there.
    let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap_or_else(|e| panic!("the recorded URL must name the live server port: {url} ({e})"));
    client
        .write_all(b"GET /api/overview HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(
        String::from_utf8_lossy(&response).contains("200 OK"),
        "the URL handed to the opener must serve the dashboard"
    );

    server.abort();
    let _ = server.await;
}

/// The control: a server that cannot bind returns from `run_server` before the
/// `open_browser` block, so the opener is never spawned. Because `PATH` holds
/// only the recorder, a regression that opened a browser anyway would write the
/// record and fail here.
#[tokio::test]
async fn a_server_that_cannot_bind_never_reaches_the_opener() {
    let fixture = tempfile::tempdir().unwrap();
    let record = fixture.path().join("opened.txt");
    #[cfg(unix)]
    write_opener_stub(fixture.path(), &record);
    let _path = PathGuard::only(fixture.path());

    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().to_string_lossy().into_owned();
    Store::open(&root_path).unwrap();
    // Hold the port, then ask `run_server` for it with `open_browser = true`.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(
        run_server(root_path, port, true).await.is_err(),
        "a taken port must make the bind fail"
    );
    assert!(
        !record.exists(),
        "the opener runs only after a successful bind; nothing may be opened when the bind fails"
    );
}
