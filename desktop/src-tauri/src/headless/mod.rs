//! Foreground Desktop without a window or WebView. Business and remote
//! protocol implementations are the same ones used by the graphical Desktop.
mod agent;

use std::io::{IsTerminal, Write};
use std::process::Stdio;
use std::time::Duration;

use crate::{future_login, remote, AppError};

pub const HELP: &str = "Usage: futureos-headless [--relay URL [--relay-ca PEM]] [--help]

Run Desktop's mobile remote entry in this terminal; Ctrl+C stops it.
Guide platform login and phone pairing when needed; reuse an existing pairing.
With --relay, no Future account is needed at all.

  --relay URL Print an invitation that points at a self-hosted relay
              (future-relay) instead of the Future platform. The address is
              remembered for later launches; pass an empty value to go back.
  --relay-ca  PEM file with the relay's CA, when its certificate is not issued
              by a public CA.
  --help      Show this help without starting Desktop or Agent.

Login uses your phone/computer browser; phone pairing uses the FutureOS app.
First-time setup requires an interactive terminal. No background service is installed.

Pair a device, drop one, or choose which one is in control with `future remote
devices` — from any terminal, while this entry is running.";

#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    Headless(Options),
    Help,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub relay: Option<String>,
    pub relay_ca: Option<String>,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Launch, String> {
        let mut help = false;
        let mut options = Self::default();
        let mut index = 0;
        while index < args.len() {
            let arg = args[index].as_str();
            match arg {
                "--help" | "-h" => help = true,
                "--relay" | "--relay-ca" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| format!("{arg} needs a value"))?;
                    if arg == "--relay" {
                        options.relay = Some(value.clone());
                    } else {
                        options.relay_ca = Some(value.clone());
                    }
                }
                value if value.starts_with("--relay=") => {
                    options.relay = Some(value["--relay=".len()..].to_string());
                }
                value if value.starts_with("--relay-ca=") => {
                    options.relay_ca = Some(value["--relay-ca=".len()..].to_string());
                }
                _ => return Err(format!("Unknown option: {arg}")),
            }
            index += 1;
        }
        if help {
            Ok(Launch::Help)
        } else {
            Ok(Launch::Headless(options))
        }
    }
}

/// A single process-lifetime runtime; no Tauri Builder, window, plugin or
/// event loop is constructed on this path.
pub fn run(options: Options) -> Result<(), String> {
    // Hydration may supply FUTURE_HOME; freeze it before selecting the lock.
    crate::host_environment::initialize().map_err(|e| e.to_string())?;
    let _guard = crate::instance::InstanceGuard::acquire().map_err(|e| e.to_string())?;
    // Applied before anything reads the pairing store: a `--relay` on this
    // launch decides which relay the invitation it prints will point at.
    crate::launch::apply_relay_settings(options.relay.as_deref(), options.relay_ca.as_deref())?;
    crate::host_environment::start_logging().map_err(|e| e.to_string())?;
    // Say where diagnostics land before any Agent work starts — after the data
    // root is final, because hydration above can still change it. The Agent logs
    // each startup phase to that file as well as this terminal, and on a host
    // that kills the process for exceeding a memory or per-user CPU budget the
    // file is the only surviving evidence.
    eprintln!(
        "FutureOS: startup progress is logged to {} and mirrored in this terminal.",
        crate::host_environment::log_file_path().display()
    );
    prepare_terminal();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .max_blocking_threads(32)
        .thread_stack_size(2 * 1024 * 1024)
        .build()
        .map_err(|e| e.to_string())?;
    crate::runtime::set(runtime.handle().clone());
    let result = runtime.block_on(async {
        let signals = ShutdownSignals::new()?;
        let mut agent = agent::Agent;
        let result = until_shutdown(signals.wait(), session(&mut agent)).await;
        agent.shutdown().await?;
        result
    });
    // Do not let a stuck blocking task make Ctrl+C wait forever.
    runtime.shutdown_timeout(Duration::from_secs(2));
    result.map_err(|e| e.to_string()).map_err(with_log_hint)
}

/// A failure message this entry point cannot act on is still one the operator
/// must be able to finish: name the log that holds the preceding startup phases
/// whenever one was written.
fn with_log_hint(message: String) -> String {
    let log = crate::host_environment::log_file_path();
    if log.is_file() {
        format!(
            "{message}\nThe Agent's last startup phase and any failure are in {}.",
            log.display()
        )
    } else {
        message
    }
}

async fn until_shutdown(
    shutdown: impl std::future::Future<Output = Result<(), AppError>>,
    session: impl std::future::Future<Output = Result<(), AppError>>,
) -> Result<(), AppError> {
    tokio::select! {
        biased;
        result = shutdown => result,
        result = session => result,
    }
}

async fn session(agent: &mut agent::Agent) -> Result<(), AppError> {
    eprintln!("FutureOS headless Desktop - foreground mode. Press Ctrl+C to stop.");
    // Fail on corrupt credentials rather than silently replacing the account.
    crate::auth_store::read()?;
    let paired = crate::remote_host::pairing::load_creds_checked()?.is_some();
    // A self-hosted relay is the whole answer to "which account": it has none.
    // The login gate exists because the Future platform mints the invitation,
    // and with `--relay` it no longer does.
    let relay = crate::remote::relay::endpoint();
    if relay.is_none() && (!paired || future_login::future_api_key().is_err()) {
        require_terminal()?;
    }
    if relay.is_some() {
        require_terminal()?;
    }
    crate::store::initialize_app_store()?;
    ensure_agent_running(agent).await?;
    crate::agent_bridge::install_channel_runtime();
    let account_key = if let Some(url) = relay.as_deref() {
        eprintln!(
            "Self-hosted relay ({url}). No Future account is used for phone pairing; \
             existing pairings keep using the relay that issued them."
        );
        None
    } else {
        ensure_login().await?;
        let key = future_login::future_api_key()?;
        let platform = crate::future_platform::current_platform_url();
        eprintln!("Platform login authorized ({platform}).");
        // Fresh servers have no WebView to trigger the usual initial model sync.
        // Through the scheduler so the automatic loop shares this run's clock, the
        // same way the window's `sync_future_models` command does.
        if let Err(error) = crate::scheduler::refresh_future_models_now().await {
            eprintln!(
                "Model catalog refresh failed; existing configured models remain available: {error}"
            );
        }
        Some(key)
    };
    let platform = crate::future_platform::current_platform_url();
    // The window's fixed-interval maintenance (app updates and the balance
    // event need a WebView; see `scheduler::start_headless`).
    crate::scheduler::start_headless();
    // Built-in skills are installed by the window's post-login onboarding. A
    // server never renders that flow, so it runs the same CLI step itself.
    crate::runtime::spawn(bootstrap_builtin_skills());
    // Same background projections, approvals and persistence paths as Desktop.
    // Headless has no active-thread UI hint, so its compatibility monitor
    // observes only sessions the Agent reports as actively streaming.
    crate::agent_bridge::spawn_streaming_observer_monitor();
    crate::agent_bridge::spawn_provider_config_observer();
    crate::agent_bridge::spawn_session_events_observer();
    crate::agent_bridge::spawn_session_discovery();
    crate::agent_bridge::spawn_delete_outbox_worker();
    crate::agent_bridge::spawn_active_run_watchdog();
    // User-defined tasks run here too: headless desktop has no webview, so the
    // executor's notifications downgrade to ledger-only (the run rows are the
    // signal). This is what lets a server box run tasks with no GUI attached.
    crate::tasks::start_headless();
    crate::runtime::spawn(async {
        crate::agent_bridge::reconcile_interrupted_runs().await;
        crate::agent_bridge::reconcile_pending_approvals().await;
    });

    eprintln!("Preparing the remote connection...");
    let mut status = remote::start(remote::RemoteStartInput {}).await?;
    // `future remote devices add` asks this process for an invitation: only a
    // process that can serve the new pairing may mint one, and a server has no
    // window to print it in — the terminal that asked does. Started after the
    // remote entry, so a request that arrives during startup is answered by a
    // Desktop that is actually serving rather than refused as "off".
    crate::remote::control::spawn_host();
    let mut shown_invitation = None;
    let mut invitation_expiry = None;
    let mut previous_state = String::new();
    let mut was_paired = false;
    loop {
        agent.check_running()?;
        // Logout/account/environment changes never leave an old account's
        // remote ingress open until its JWT eventually expires. There is no
        // account to watch in relay mode: the pairing's own credential and the
        // room registration are the whole authorization.
        if let Some(account_key) = account_key.as_deref() {
            if future_login::future_api_key().ok().as_deref() != Some(account_key)
                || crate::future_platform::current_platform_url() != platform
            {
                return Err("Platform login changed or was removed. Remote access is closing; restart to authorize again.".into());
            }
        }
        let state = format!(
            "{:?} (Agent {})",
            status.phase,
            if status.agent_available {
                "available"
            } else {
                "unavailable"
            }
        );
        if state != previous_state {
            eprintln!("Remote: {state}");
            previous_state = state;
        }
        if matches!(
            status.phase,
            remote::RemotePhase::Failed
                | remote::RemotePhase::Revoked
                | remote::RemotePhase::Stopped
        ) {
            return Err(format!("Remote connection stopped ({:?}). Check platform login/network, then restart; use `future remote devices` to pair or drop a device.", status.reason).into());
        }
        invitation_expiry = status.pairing_code_expires_at.or(invitation_expiry);
        if let Some(link) = pairing_link(&status) {
            require_terminal()?;
            if shown_invitation.as_ref() != Some(&link) {
                print_invitation(
                    "PHONE PAIRING - scan in the FutureOS app, or paste this complete link",
                    &link,
                )?;
                eprintln!(
                    "{}",
                    single_use_notice(status.pairing_code_expires_at, unix_seconds())
                );
                shown_invitation = Some(link);
            }
        }
        let paired = crate::remote_host::pairing::load_creds_checked()?.is_some();
        if paired && !was_paired && status.phase == remote::RemotePhase::Ready {
            eprintln!("Phone pairing saved. Remote access is open; connect with the paired FutureOS app. Ctrl+C closes this entry.");
            was_paired = true;
        }
        if !paired && invitation_expiry.is_some_and(|expiry| expiry <= unix_seconds()) {
            return Err(
                "Phone pairing invitation expired. Restart headless Desktop to request a new one."
                    .into(),
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        status = remote::status();
    }
}

/// How long the owned Agent may take before the wait is worth explaining.
const SLOW_STARTUP_NOTICE: Duration = Duration::from_secs(30);

/// The line under a printed invitation: single-use, and when it lapses.
///
/// The deadline is the reader's own clock and the time left to act
/// (`2026-10-20 09:45:32 (local time, in 9m 58s)`), not an epoch to convert by
/// hand. Spelled the same way as `future remote devices add`, which prints an
/// invitation of its own. An instant no calendar can name falls back to the
/// epoch — never to a wrong date, and never to a `0` for "no deadline given".
fn single_use_notice(expires_at: Option<i64>, now: i64) -> String {
    let Some(expiry) = expires_at else {
        return "Single-use invitation. Do not share it or save it in logs.".to_string();
    };
    let when = deadline_phrase(expiry, now).unwrap_or_else(|| format!("Unix time {expiry}"));
    format!("Single-use invitation; expires at {when}. Do not share it or save it in logs.")
}

/// A deadline as the reader's own clocks: the wall clock and the time left.
/// `None` for an instant no calendar can express.
fn deadline_phrase(expiry: i64, now: i64) -> Option<String> {
    let clock = local_wall_clock(expiry)?;
    Some(match time_left(expiry, now) {
        Some(left) => format!("{clock} (local time, in {left})"),
        // Already passed, or this machine's clock runs ahead of the Desktop's:
        // the deadline still names itself, and "in 0s" would only puzzle.
        None => format!("{clock} (local time)"),
    })
}

/// A deadline as the local clock, e.g. `2026-10-20 09:45:32`.
///
/// The zone is spelled out by the caller rather than taken from `%Z`: chrono
/// derives that from the platform, and it is inconsistent across Windows and
/// Unix. The range guard keeps `with_timezone` inside what the platform's time
/// zone database answers for.
fn local_wall_clock(unix: i64) -> Option<String> {
    let instant = chrono::DateTime::from_timestamp(unix, 0)
        .filter(|instant| (1970..=9999).contains(&chrono::Datelike::year(instant)))?;
    Some(
        instant
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    )
}

/// How long is left before `expiry`, in the largest unit worth naming:
/// `42s`, `9m 58s`, `2h 05m`. `None` once it has passed.
fn time_left(expiry: i64, now: i64) -> Option<String> {
    let left = expiry.checked_sub(now)?;
    if left <= 0 {
        return None;
    }
    let (hours, minutes, seconds) = (left / 3600, (left % 3600) / 60, left % 60);
    Some(if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    })
}

/// What the terminal says while the Agent is still initializing. On a cold or
/// network-mounted data root that wait is normal; a host enforcing a per-user
/// budget may instead kill the process without any error, so both readings of a
/// long startup are named here.
fn slow_startup_notice(log: &std::path::Path) -> String {
    format!(
        "The Agent is still starting: a cold or network-mounted data root can take minutes. \
Progress is logged as it happens. If this process disappears instead of reporting an error, \
the host killed it (memory or per-user budget); the phases it reached are in {}.",
        log.display()
    )
}

/// Initialize the owned Agent, explaining a long wait instead of leaving the
/// terminal silent for the whole of it.
async fn ensure_agent_running(agent: &mut agent::Agent) -> Result<(), AppError> {
    starting_with_notice(
        agent.ensure_running(),
        &crate::host_environment::log_file_path(),
    )
    .await
}

/// Wait for `start` while reporting at most once that the wait is long. The
/// remaining startup is never abandoned: the notice only explains the silence.
async fn starting_with_notice<T>(
    start: impl std::future::Future<Output = Result<T, AppError>>,
    log: &std::path::Path,
) -> Result<T, AppError> {
    let start = start;
    tokio::pin!(start);
    let notice = tokio::time::sleep(SLOW_STARTUP_NOTICE);
    tokio::pin!(notice);
    tokio::select! {
        result = start.as_mut() => result,
        _ = notice.as_mut() => {
            eprintln!("{}", slow_startup_notice(log));
            start.await
        }
    }
}

fn require_terminal() -> Result<(), AppError> {
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        Ok(())
    } else {
        Err("Login/pairing requires an interactive terminal. Run `futureos-headless` as the same user first; authorization links are not written to redirected logs.".into())
    }
}

/// Install the platform's built-in skills the way the window's post-login
/// onboarding does: `future init`, which is idempotent and needs no login. The
/// window spawns the bundled CLI through the Tauri shell plugin; the server has
/// no plugin, so the same CLI is spawned directly — and only after the Agent,
/// because the install writes the Agent's own skill database.
///
/// Best-effort by the same contract as the window's step: a missing CLI or a
/// failed install is reported and then ignored. The phone can still install
/// skills from its own Skills page, and the catalog fetch behind `future init`
/// is a network call this foreground process must not stall on.
async fn bootstrap_builtin_skills() {
    let executable = match crate::cli_tool::resolve_for("skills.init") {
        Ok(executable) => executable,
        Err(error) => {
            eprintln!("Skipping built-in skill setup: {error}");
            return;
        }
    };
    let output = tokio::process::Command::new(&executable)
        .arg("init")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("Could not run {} init: {error}", executable.display());
            return;
        }
    };
    // Both streams go to stderr with the window's `[skills]` prefix: stdout is
    // reserved for the authorization and pairing material this entry point
    // prints when it is attached to a terminal.
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        eprintln!("[skills] {line}");
    }
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        eprintln!("[skills] {line}");
    }
    if !output.status.success() {
        eprintln!(
            "FutureOS: skill bootstrap did not complete ({}).",
            output.status
        );
    }
    // The Agent caches skill discovery for prompt-time injection. The window
    // refreshes that cache on startup, so a server that just gained built-ins
    // must give the Agent the same signal or a run started from the phone would
    // not be offered them until the next restart.
    crate::agent_bridge::refresh_skills().await;
}

async fn ensure_login() -> Result<(), AppError> {
    if future_login::future_api_key().is_ok() {
        match future_login::fetch_profile().await {
            Ok(_) => return Ok(()),
            Err(AppError::Remote {
                status: 401 | 403, ..
            }) => {
                eprintln!("Platform authorization is no longer valid. Please sign in again.");
            }
            // Network failure is not logout and must never overwrite a key.
            Err(error) => return Err(error),
        }
    }
    require_terminal()?;
    let login = future_login::start_with_browser(false).await?;
    print_invitation(
        "PLATFORM LOGIN - scan with your phone camera/browser and authorize this server",
        &login.verification_uri_complete,
    )?;
    println!(
        "If requested by the web page, enter user code: {}",
        login.user_code
    );
    println!(
        "Authorization expires in {} seconds. Waiting for approval...",
        login.expires_in
    );
    std::io::stdout().flush()?;
    wait_for_login(
        Duration::from_secs(login.interval),
        Duration::from_secs(login.expires_in),
        || future_login::poll(&login.device_code),
    )
    .await
}

async fn wait_for_login<F, Fut>(
    interval: Duration,
    expires: Duration,
    mut poll: F,
) -> Result<(), AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<future_login::FutureLoginPoll, AppError>>,
{
    let last_poll_was_transient = std::cell::Cell::new(false);
    tokio::time::timeout(expires, async {
        let mut normal_interval = interval;
        let mut next_interval = interval;
        let mut transient_interval = Duration::from_secs(2);
        let max_transient_interval = Duration::from_secs(15);
        let mut malformed_responses = 0;
        loop {
            tokio::time::sleep(next_interval).await;
            let response = poll().await?;
            match response.status.as_str() {
                "authorized" => return Ok(()),
                "pending" => {
                    last_poll_was_transient.set(false);
                    next_interval = normal_interval;
                    transient_interval = Duration::from_secs(2);
                    malformed_responses = 0;
                }
                "slow_down" => {
                    last_poll_was_transient.set(false);
                    normal_interval = normal_interval.saturating_add(Duration::from_secs(5));
                    next_interval = normal_interval;
                    transient_interval = Duration::from_secs(2);
                    malformed_responses = 0;
                }
                "retry" => {
                    last_poll_was_transient.set(true);
                    let server_interval = response
                        .retry_after_seconds
                        .map(Duration::from_secs)
                        .unwrap_or_default();
                    next_interval = transient_interval.max(server_interval).min(expires);
                    transient_interval = transient_interval
                        .saturating_mul(2)
                        .min(max_transient_interval);
                }
                "malformed" => {
                    last_poll_was_transient.set(true);
                    malformed_responses += 1;
                    if malformed_responses >= 3 {
                        return Err(
                            "Platform authorization server returned invalid responses. Restart to try again."
                                .into(),
                        );
                    }
                    next_interval = transient_interval;
                    transient_interval = transient_interval
                        .saturating_mul(2)
                        .min(max_transient_interval);
                }
                "denied" => return Err("Platform authorization was denied.".into()),
                "expired" => {
                    return Err("Platform authorization expired. Restart to try again.".into())
                }
                _ => return Err("Platform authorization failed. Restart to try again.".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        if last_poll_was_transient.get() {
            AppError::from(
                "Platform authorization server is temporarily unavailable. Restart to try again.",
            )
        } else {
            AppError::from("Platform authorization expired. Restart to try again.")
        }
    })?
}

/// Print one invitation: the label, the complete link, and the same link as a QR
/// code when the terminal is wide enough to hold it.
fn print_invitation(label: &str, link: &str) -> Result<(), AppError> {
    let output = invitation_text(label, link, true, terminal_columns());
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

/// Width of the terminal doing the printing, when it can be measured.
pub(crate) fn terminal_columns() -> Option<usize> {
    terminal_size::terminal_size().map(|(width, _)| usize::from(width.0))
}

/// The invitation block both terminal entry points print: label, link, and
/// optionally the same link as a QR code.
///
/// The one renderer for invitation material in this crate: a narrow terminal
/// keeps the link and says why the QR is missing, which is why no flag is needed
/// to suppress the code — a redirected or narrow terminal already gets the link.
pub(crate) fn invitation_text(label: &str, link: &str, qr: bool, columns: Option<usize>) -> String {
    let mut output = format!("\n{label}\n{link}\n");
    if qr {
        match qrcode::QrCode::new(link.as_bytes()) {
            Ok(code) if columns.is_none_or(|width| code.width() + 8 <= width) => {
                use qrcode::render::unicode::Dense1x2;
                let image = code.render::<Dense1x2>().build();
                // Explicit black-on-white contrast on both light and dark
                // terminal themes. Reset every line to avoid tinting logs.
                for line in image.lines() {
                    output.push_str(&format!("\x1b[30;47m{line}\x1b[0m\n"));
                }
            }
            Ok(_) => output.push_str("Terminal too narrow for this QR code; widen it and restart, or use the link above.\n"),
            Err(_) => output.push_str("Cannot render a QR code; use the link above.\n"),
        }
    }
    output
}

fn prepare_terminal() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::*;
        let _ = SetConsoleOutputCP(65001);
        if let Ok(handle) = GetStdHandle(STD_OUTPUT_HANDLE) {
            let mut mode = CONSOLE_MODE::default();
            if GetConsoleMode(handle, &mut mode).is_ok() {
                let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }
}

/// The invitation a client can pair against, or `None` when there is not one to
/// show.
///
/// The Agent readiness gate belongs to this terminal: it prints the invitation
/// only for a bridge that can actually serve the phone. A terminal asking for a
/// new invitation (`future remote devices add`, via `remote::control`) uses
/// [`invitation_link`] instead, because the code itself is valid the moment it is
/// minted and the probe has not run yet.
pub fn pairing_link(status: &remote::RemoteStatus) -> Option<String> {
    if status.phase != remote::RemotePhase::Ready || !status.agent_available {
        return None;
    }
    invitation_link(status)
}

/// The invitation in a status, judged only by whether it can still be scanned.
///
/// Separate from [`pairing_link`] on purpose: `agent_available` is a probe that
/// starts with the bridge and answers on its own schedule. A device request reads
/// the status right after the pairing is minted — before that probe can have
/// answered — so gating the printed code on it reported "no pairing invitation"
/// for a code the window was showing and the phone could scan. Whether the Agent
/// is ready changes what a paired device can do, not whether the invitation
/// works.
pub fn invitation_link(status: &remote::RemoteStatus) -> Option<String> {
    let code = status.pairing_code.as_ref()?;
    if status
        .pairing_code_expires_at
        .is_none_or(|expiry| expiry <= unix_seconds())
    {
        return None;
    }
    // Desktop owns the complete v2 invitation, including the local-only PSK.
    // Do not wrap it in another `code` parameter or reconstruct a v1 invite.
    let url = reqwest::Url::parse(code).ok()?;
    // v3 is the same invitation, minted by a self-hosted relay instead of the
    // platform: the QR still carries the Noise static key and the one-time
    // secret, and the phone still verifies them. Only the claim endpoint moved.
    if url.scheme() != "futureos"
        || url.host_str() != Some("remote")
        || url.path() != "/pair"
        || !url
            .query_pairs()
            .any(|(key, value)| key == "v" && matches!(&*value, "2" | "3"))
    {
        return None;
    }
    Some(code.clone())
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

struct ShutdownSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hangup: tokio::signal::unix::Signal,
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
    #[cfg(windows)]
    close: tokio::signal::windows::CtrlClose,
}

impl ShutdownSignals {
    fn new() -> Result<Self, AppError> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
                hangup: signal(SignalKind::hangup())?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                interrupt: tokio::signal::windows::ctrl_c()?,
                close: tokio::signal::windows::ctrl_close()?,
            })
        }
    }

    async fn wait(mut self) -> Result<(), AppError> {
        #[cfg(unix)]
        tokio::select! {
            _ = self.interrupt.recv() => {},
            _ = self.terminate.recv() => {},
            _ = self.hangup.recv() => {},
        }
        #[cfg(windows)]
        tokio::select! {
            _ = self.interrupt.recv() => {},
            _ = self.close.recv() => {},
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Launch, String> {
        Options::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn standalone_launch_takes_only_the_relay_flags() {
        assert_eq!(parse(&[]).unwrap(), Launch::Headless(Options::default()));
        assert_eq!(
            parse(&["--relay", "https://relay.example"]).unwrap(),
            Launch::Headless(Options {
                relay: Some("https://relay.example".into()),
                relay_ca: None,
            })
        );
        assert_eq!(
            parse(&["--relay=https://relay.example", "--relay-ca=/tmp/ca.pem"]).unwrap(),
            Launch::Headless(Options {
                relay: Some("https://relay.example".into()),
                relay_ca: Some("/tmp/ca.pem".into()),
            })
        );
        for flag in [
            "--headless",
            "--headles",
            "--gui",
            "-psn_123",
            "--re-pair",
            "--no-qr",
        ] {
            assert!(
                parse(&[flag]).is_err(),
                "{flag} is not this entry point's option"
            );
            assert!(parse(&["--help", flag]).is_err());
        }
        assert_eq!(parse(&["--help"]).unwrap(), Launch::Help);
        assert_eq!(parse(&["-h"]).unwrap(), Launch::Help);
    }

    #[tokio::test]
    async fn interrupt_drops_pending_startup_before_it_can_complete() {
        let completed = std::sync::atomic::AtomicBool::new(false);
        until_shutdown(async { Ok(()) }, async {
            completed.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        })
        .await
        .unwrap();
        assert!(!completed.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn startup_errors_are_not_hidden_as_success() {
        let result =
            until_shutdown(std::future::pending(), async { Err("setup failed".into()) }).await;
        assert_eq!(result.unwrap_err().to_string(), "setup failed");
    }

    /// A long Agent startup must be explained, and must still be allowed to
    /// finish: the notice is not a timeout. Falsified by returning early from
    /// the notice branch or by dropping the pending startup.
    #[tokio::test(start_paused = true)]
    async fn a_long_agent_start_is_explained_and_still_completes() {
        let log = std::path::Path::new("/tmp/desktop.log");
        let started = tokio::time::Instant::now();
        let result = starting_with_notice(
            async {
                tokio::time::sleep(Duration::from_secs(45)).await;
                Ok("ready")
            },
            log,
        )
        .await;
        assert_eq!(result.unwrap(), "ready");
        assert_eq!(started.elapsed(), Duration::from_secs(45));
    }

    #[tokio::test(start_paused = true)]
    async fn a_fast_agent_start_is_not_delayed_by_the_notice() {
        let started = tokio::time::Instant::now();
        let result = starting_with_notice(
            async { Ok("ready") },
            std::path::Path::new("/tmp/desktop.log"),
        )
        .await;
        assert_eq!(result.unwrap(), "ready");
        assert!(started.elapsed() < SLOW_STARTUP_NOTICE);
    }

    /// The notice is the only place a host-side kill is named. It has to point
    /// at the log, because the process that gets SIGKILLed cannot report it.
    #[test]
    fn the_slow_startup_notice_points_at_the_log_and_a_host_kill() {
        let notice =
            slow_startup_notice(std::path::Path::new("/data/.future/app/logs/desktop.log"));
        assert!(
            notice.contains("/data/.future/app/logs/desktop.log"),
            "{notice}"
        );
        assert!(notice.contains("the host killed it"), "{notice}");
        assert!(notice.contains("network-mounted"), "{notice}");
    }

    /// The log hint is what makes an exited entry point actionable, and it must
    /// not claim a log that was never written.
    #[test]
    fn the_exit_hint_names_an_existing_log_only() {
        let _home = crate::auth_store::test_support::HomeGuard::new("headless-log-hint");
        let log = crate::host_environment::log_file_path();
        assert_eq!(with_log_hint("failed".into()), "failed");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "FutureOS: host starting\n").unwrap();
        let hinted = with_log_hint("failed".into());
        assert!(hinted.starts_with("failed\n"), "{hinted}");
        assert!(hinted.contains(&log.display().to_string()), "{hinted}");
    }

    #[tokio::test(start_paused = true)]
    async fn login_wait_honors_pending_slow_down_and_authorization() {
        let started = tokio::time::Instant::now();
        let mut replies = ["pending", "slow_down", "authorized"].into_iter();
        wait_for_login(Duration::from_secs(2), Duration::from_secs(60), || {
            let status = replies.next().unwrap().to_string();
            async move {
                Ok(future_login::FutureLoginPoll {
                    status,
                    message: None,
                    retry_after_seconds: None,
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(11));
    }

    #[tokio::test(start_paused = true)]
    async fn login_wait_retries_transient_failures_with_backoff() {
        let started = tokio::time::Instant::now();
        let mut replies = ["retry", "retry", "pending", "authorized"].into_iter();
        wait_for_login(Duration::from_secs(1), Duration::from_secs(60), || {
            let status = replies.next().unwrap().to_string();
            async move {
                Ok(future_login::FutureLoginPoll {
                    status,
                    message: None,
                    retry_after_seconds: None,
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(8));
    }

    #[tokio::test(start_paused = true)]
    async fn login_denial_expiry_and_http_failure_do_not_authorize() {
        for status in ["denied", "expired", "error"] {
            assert!(
                wait_for_login(Duration::from_secs(1), Duration::from_secs(5), || async {
                    Ok(future_login::FutureLoginPoll {
                        status: status.into(),
                        message: None,
                        retry_after_seconds: None,
                    })
                })
                .await
                .is_err()
            );
        }
        let expired = wait_for_login(Duration::from_secs(1), Duration::from_secs(3), || async {
            Ok(future_login::FutureLoginPoll {
                status: "pending".into(),
                message: None,
                retry_after_seconds: None,
            })
        })
        .await
        .unwrap_err();
        assert!(expired.to_string().contains("expired"));
        let unavailable =
            wait_for_login(Duration::from_secs(1), Duration::from_secs(3), || async {
                Ok(future_login::FutureLoginPoll {
                    status: "retry".into(),
                    message: None,
                    retry_after_seconds: None,
                })
            })
            .await
            .unwrap_err();
        assert!(unavailable.to_string().contains("temporarily unavailable"));
        assert!(
            wait_for_login(Duration::from_secs(1), Duration::from_secs(3), || async {
                Err("network failed".into())
            })
            .await
            .unwrap_err()
            .to_string()
            .contains("network")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn interrupt_cancels_an_already_waiting_authorization_request() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let poll_dropped = dropped.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        until_shutdown(
            async {
                rx.await.unwrap();
                Ok(())
            },
            wait_for_login(Duration::from_secs(1), Duration::from_secs(600), || {
                let guard = Dropped(poll_dropped.clone());
                let tx = tx.take().unwrap();
                async move {
                    let _guard = guard;
                    tx.send(()).unwrap();
                    std::future::pending().await
                }
            }),
        )
        .await
        .unwrap();
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn qr_and_link_share_the_same_invitation_with_narrow_terminal_fallback() {
        let link = "https://example.com/device?user_code=ABCD";
        let with_qr = invitation_text("PLATFORM LOGIN", link, true, Some(160));
        assert!(with_qr.contains(link));
        assert!(with_qr.contains('\u{2588}'));
        assert!(with_qr.contains("\x1b[30;47m"));
        let link_only = invitation_text("PLATFORM LOGIN", link, false, Some(160));
        assert!(!link_only.contains('\u{2588}'));
        assert!(link_only.contains(link));
        let narrow = invitation_text("PLATFORM LOGIN", link, true, Some(10));
        assert!(narrow.contains("too narrow"));
        assert!(!narrow.contains('\u{2588}'));
    }

    #[test]
    fn invitation_contains_all_mobile_identity_fields_and_is_ready_only() {
        let mut status = remote::RemoteStatus {
            devices: Vec::new(),
            client: None,
            phase: remote::RemotePhase::Ready,
            reason: None,
            recovery: None,
            agent_available: true,
            nats_url: String::new(),
            pair_id: "pair".into(),
            pairing_code: Some(format!("futureos://remote/pair?v=2&code=a%2Bb%26c&desktopId=desktop_test&desktopKey=UTEST&secureKey={}&secret={}", "A".repeat(43), "B".repeat(43))),
            pairing_code_expires_at: Some(unix_seconds() + 300),
            desktop_id: "desktop_test".into(),
            desktop_public_key: "UTEST".into(),
            web_url: None,
            web_lan_url: None,
            warning_code: None,
            cleared_invitations: Vec::new(),
        };
        let link = pairing_link(&status).unwrap();
        let url = reqwest::Url::parse(&link).unwrap();
        let fields = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(fields["code"], "a+b&c");
        assert_eq!(fields["desktopId"], "desktop_test");
        assert_eq!(fields["desktopKey"], "UTEST");
        assert_eq!(fields["v"], "2");
        assert_eq!(fields["secureKey"], "A".repeat(43));
        assert_eq!(fields["secret"], "B".repeat(43));
        assert_eq!(Some(&link), status.pairing_code.as_ref());
        assert!(qrcode::QrCode::new(link.as_bytes()).is_ok());
        status.phase = remote::RemotePhase::Reconnecting;
        assert!(pairing_link(&status).is_none());
        status.phase = remote::RemotePhase::Ready;
        // The readiness probe gates what this terminal prints for a bridge, not
        // whether the code is an invitation: a device request answers from a
        // status taken before the probe has run.
        status.agent_available = false;
        assert!(pairing_link(&status).is_none());
        assert_eq!(invitation_link(&status).as_deref(), Some(link.as_str()));
        status.agent_available = true;
        status.pairing_code_expires_at = Some(unix_seconds() - 1);
        assert!(pairing_link(&status).is_none());
        assert!(invitation_link(&status).is_none());
    }

    /// A `future` CLI fixture: answers `version --json` with a compatible
    /// capability contract (so discovery accepts it) and records the fact that
    /// `init` ran by touching a marker file.
    #[cfg(unix)]
    fn cli_fixture(
        directory: &std::path::Path,
        marker: &std::path::Path,
        init: &str,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let capabilities = future_rpc::cli_capabilities::CliCapabilities {
            commands: vec!["skills.init".into()],
            ..Default::default()
        };
        let reply = serde_json::json!({ "version": "1.2.3", "desktopCapabilities": capabilities })
            .to_string();
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join("future");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = version ]; then printf '%s' '{reply}'; exit 0; fi\n\
                 printf 'ran' > '{marker}'\n{init}\n",
                marker = marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Serialize the tests that replace the process-global `PATH` and the agent
    /// endpoint, and restore both.
    #[cfg(unix)]
    fn with_path_and_broken_agent<T>(directory: &std::path::Path, body: impl FnOnce() -> T) -> T {
        let _lock = crate::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let saved_path = std::env::var_os("PATH");
        let saved_addr = std::env::var_os("FUTURE_AGENT_GRPC_ADDR");
        std::env::set_var("PATH", directory.as_os_str());
        // Unparseable on purpose: `refresh_skills` must fail fast instead of
        // reaching for a real local Agent.
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = body();
        match saved_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match saved_addr {
            Some(value) => std::env::set_var("FUTURE_AGENT_GRPC_ADDR", value),
            None => std::env::remove_var("FUTURE_AGENT_GRPC_ADDR"),
        }
        result
    }

    /// The server has no Tauri shell plugin, so it spawns the same `future init`
    /// the window's onboarding uses — after the Agent, because the install
    /// writes the Agent's own skill database. The marker is the proof that the
    /// CLI really ran rather than that the call merely returned.
    #[cfg(unix)]
    #[test]
    fn the_builtin_skill_bootstrap_runs_the_cli() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("init-ran");
        let directory = root.path().join("bin");
        let cli = cli_fixture(&directory, &marker, "printf 'installed 2 skills\\n'");
        assert!(
            !marker.exists(),
            "the marker must only appear once `init` has run"
        );
        let outcome = with_path_and_broken_agent(&directory, || {
            crate::cli_tool::resolve_for("skills.init").map_err(|error| error.to_string())
        });
        match outcome {
            Ok(path) => assert_eq!(path, cli, "discovery must return the fixture CLI"),
            Err(error) => panic!("the fixture CLI must be accepted by discovery: {error}"),
        }
        with_path_and_broken_agent(&directory, || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(bootstrap_builtin_skills());
        });
        assert!(
            marker.exists(),
            "the bootstrap must have executed the CLI's `init`"
        );
    }

    /// A failed install is reported and then ignored: the phone can still
    /// install skills from its own page, and a server must not refuse to start
    /// because the catalogue fetch failed.
    #[cfg(unix)]
    #[test]
    fn a_failing_builtin_skill_bootstrap_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("init-ran");
        let directory = root.path().join("bin");
        cli_fixture(&directory, &marker, "printf 'boom\\n' >&2; exit 3");
        with_path_and_broken_agent(&directory, || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            // Must return normally: a non-zero exit may not become a startup
            // failure by any route.
            runtime.block_on(bootstrap_builtin_skills());
        });
        assert!(
            marker.exists(),
            "the CLI ran and failed, as the fixture says"
        );
    }

    /// A CLI that answers discovery but cannot be started is reported and
    /// skipped, exactly like a missing one: the bootstrap is best-effort and the
    /// phone can still install skills. The fixture leaves the CLI's own identity
    /// untouched (same bytes, size, mtime, inode) and removes only the program its
    /// `#!` line names — the one step between the probe and the spawn that the
    /// identity re-read does not pin.
    #[cfg(unix)]
    #[test]
    fn a_cli_whose_interpreter_vanished_after_discovery_is_reported_and_skipped() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("init-ran");
        let directory = root.path().join("bin");
        let interpreter = root.path().join("sh");
        std::os::unix::fs::symlink("/bin/sh", &interpreter).unwrap();
        let capabilities = future_rpc::cli_capabilities::CliCapabilities {
            commands: vec!["skills.init".into()],
            ..Default::default()
        };
        let reply = serde_json::json!({ "version": "1.2.3", "desktopCapabilities": capabilities })
            .to_string();
        std::fs::create_dir_all(&directory).unwrap();
        let cli = directory.join("future");
        std::fs::write(
            &cli,
            format!(
                "#!{interpreter}\nif [ \"$1\" = version ]; then printf '%s' '{reply}'; exit 0; fi\nprintf 'ran' > '{marker}'\n",
                interpreter = interpreter.display(),
                marker = marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

        let discovered = with_path_and_broken_agent(&directory, || {
            crate::cli_tool::resolve_for("skills.init").map_err(|error| error.to_string())
        });
        let identity_before = std::fs::metadata(&cli).unwrap();
        // The state this test is about: the CLI is byte-for-byte the file
        // discovery just accepted, and the program it needs is gone.
        std::fs::remove_file(&interpreter).unwrap();

        with_path_and_broken_agent(&directory, || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            // Must return normally: an unstartable CLI may not become a startup
            // failure by any route.
            runtime.block_on(bootstrap_builtin_skills());
        });

        let identity_after = std::fs::metadata(&cli).unwrap();
        let same_identity = identity_before.len() == identity_after.len()
            && identity_before.modified().unwrap() == identity_after.modified().unwrap()
            && std::os::unix::fs::MetadataExt::ino(&identity_before)
                == std::os::unix::fs::MetadataExt::ino(&identity_after);
        let accepted = format!("{discovered:?}");
        let ran = marker.exists();
        assert!(
            discovered.is_ok(),
            "discovery must accept the fixture before the interpreter is removed: {accepted}"
        );
        assert!(
            same_identity,
            "the CLI itself must be untouched, so only the spawn can notice"
        );
        assert!(
            !ran,
            "a CLI that cannot be started must never run its `init`"
        );
    }

    /// No usable CLI means the step is skipped with a diagnostic, not a panic
    /// and not a startup failure.
    #[cfg(unix)]
    #[test]
    fn a_missing_cli_skips_the_builtin_skill_bootstrap() {
        let empty = tempfile::tempdir().unwrap();
        with_path_and_broken_agent(empty.path(), || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(bootstrap_builtin_skills());
        });
    }

    /// A key the platform still accepts short-circuits the whole login flow:
    /// `ensure_login` must return without ever asking for a terminal, so a
    /// server that is already authorized starts unattended.
    #[test]
    fn an_accepted_platform_key_needs_no_terminal() {
        let _home = crate::auth_store::test_support::HomeGuard::new("headless-login-ok");
        let (url, server) = mock_http_server(200, r#"{"email":"operator@example.com"}"#);
        crate::auth_store::set_future_login("sekret", &format!("{url}/api")).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(ensure_login())
            .expect("an accepted key must not require an interactive terminal");
        let _ = server.join();
    }

    /// A platform that rejects the stored key is named as such, and the entry
    /// point then stops at the terminal requirement it cannot satisfy headless.
    #[test]
    fn a_rejected_platform_key_is_reported_as_expired_authorization() {
        let _home = crate::auth_store::test_support::HomeGuard::new("headless-login-401");
        let (url, server) = mock_http_server(401, r#"{"message":"expired"}"#);
        crate::auth_store::set_future_login("sekret", &format!("{url}/api")).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(ensure_login())
            .expect_err("a redirected login must fail");
        assert!(
            error.to_string().contains("interactive terminal"),
            "a rejected key falls through to the terminal requirement: {error}"
        );
        let _ = server.join();
    }

    /// A network failure while checking the profile is not a logout: the key
    /// must survive, and the error must reach the operator.
    #[test]
    fn a_network_failure_while_checking_the_platform_login_is_not_a_logout() {
        let _home = crate::auth_store::test_support::HomeGuard::new("headless-login-net");
        crate::auth_store::set_future_login("sekret", "http://127.0.0.1:1/api").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(ensure_login())
            .expect_err("an unreachable platform must be an error");
        assert!(
            error
                .to_string()
                .contains("Failed to fetch account profile"),
            "{error}"
        );
        assert_eq!(
            crate::auth_store::read()
                .unwrap()
                .get("future")
                .and_then(|future| future.get("key"))
                .and_then(|key| key.as_str()),
            Some("sekret"),
            "a network failure must never discard the stored key"
        );
    }

    /// Minimal single-response HTTP server for the profile check.
    fn mock_http_server(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            // Bounded accept: a request that never arrives must not leave this
            // thread (and the test's `join`) blocked for the whole run.
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut request = [0_u8; 4096];
                        let _ = stream.read(&mut request);
                        let response = format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
                }
            }
        });
        (url, handle)
    }

    /// Persistent malformed replies stop the wait instead of looping until the
    /// authorization code expires, and the message says why.
    #[tokio::test(start_paused = true)]
    async fn login_wait_gives_up_after_persistent_malformed_replies() {
        let error = wait_for_login(Duration::from_secs(1), Duration::from_secs(600), || async {
            Ok(future_login::FutureLoginPoll {
                status: "malformed".into(),
                message: None,
                retry_after_seconds: None,
            })
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("invalid responses"), "{error}");
    }

    /// A malformed reply is backed off like any transient failure — the next
    /// poll has to wait the doubled interval, not hammer the server.
    #[tokio::test(start_paused = true)]
    async fn login_wait_backs_off_after_a_malformed_reply() {
        let started = tokio::time::Instant::now();
        let mut replies = ["malformed", "authorized"].into_iter();
        wait_for_login(Duration::from_secs(1), Duration::from_secs(600), || {
            let status = replies.next().unwrap().to_string();
            async move {
                Ok(future_login::FutureLoginPoll {
                    status,
                    message: None,
                    retry_after_seconds: None,
                })
            }
        })
        .await
        .unwrap();
        // 1s (first interval) + 2s (the transient interval a malformed reply sets).
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    /// An invitation that cannot be encoded gets the link-only fallback instead
    /// of an error: the operator still has to be able to authorize.
    #[test]
    fn an_unencodable_invitation_falls_back_to_the_link() {
        let link = "x".repeat(8_000);
        let text = invitation_text("PLATFORM LOGIN", &link, true, Some(80));
        assert!(text.contains("Cannot render a QR code"), "{text}");
        assert!(text.contains(&link), "the link must still be printable");
    }

    /// A non-`v2` (or foreign) pairing code is not offered as an invitation: the
    /// terminal must not hand a user something the app cannot consume.
    #[test]
    fn pairing_link_rejects_codes_that_are_not_v2_invitations() {
        let mut status = remote::RemoteStatus {
            devices: Vec::new(),
            client: None,
            phase: remote::RemotePhase::Ready,
            reason: None,
            recovery: None,
            agent_available: true,
            nats_url: String::new(),
            pair_id: "pair".into(),
            pairing_code: Some("futureos://remote/pair?v=1&code=a".into()),
            pairing_code_expires_at: Some(unix_seconds() + 300),
            desktop_id: "desktop_test".into(),
            desktop_public_key: "UTEST".into(),
            web_url: None,
            web_lan_url: None,
            warning_code: None,
            cleared_invitations: Vec::new(),
        };
        assert!(pairing_link(&status).is_none(), "v1 is not offered");
        status.pairing_code = Some("futureos://elsewhere/pair?v=2".into());
        assert!(
            pairing_link(&status).is_none(),
            "a foreign host is not offered"
        );
        status.pairing_code = Some("https://example.com/device?v=2".into());
        assert!(
            pairing_link(&status).is_none(),
            "a foreign scheme is not offered"
        );
        status.pairing_code = Some("not a url".into());
        assert!(
            pairing_link(&status).is_none(),
            "an unparseable code is not offered"
        );
        status.pairing_code = None;
        assert!(pairing_link(&status).is_none(), "nothing to offer");
        status.pairing_code = Some("futureos://remote/pair?v=2&code=a".into());
        status.agent_available = false;
        assert!(
            pairing_link(&status).is_none(),
            "an unavailable Agent is not offered"
        );
    }

    /// The invitation writer is the one place the terminal prints pairing or
    /// authorization material, and it writes that material itself (the harness
    /// captures fd 1, so this asserts the call succeeds and the renderer's own
    /// rules are tested separately through `invitation_text`).
    #[test]
    fn print_invitation_writes_the_link_and_the_qr() {
        print_invitation("PLATFORM LOGIN", "https://example.com/device?code=ABCD")
            .expect("writing to the test's captured stdout must succeed");
    }

    /// The deadline under an invitation is the reader's own clock plus the time
    /// left — never a bare epoch, and never a `0` standing in for "no deadline".
    /// The zone is the machine's, so the test reads the printed clock back
    /// instead of naming CI's timezone.
    #[test]
    fn the_single_use_notice_prints_a_clock_and_the_time_left() {
        use chrono::TimeZone;

        let now = 1_800_000_000;
        // The printed clock is the instant the Desktop meant, rendered locally.
        let local = chrono::DateTime::from_timestamp(now + 600, 0)
            .unwrap()
            .with_timezone(&chrono::Local);
        assert_eq!(
            chrono::Local
                .from_local_datetime(&local.naive_local())
                .single()
                .expect("an unambiguous local time")
                .timestamp(),
            now + 600,
            "the formatter must not shift the instant"
        );
        let clock = local.format("%Y-%m-%d %H:%M:%S").to_string();

        let notice = single_use_notice(Some(now + 600), now);
        assert_eq!(
            notice,
            format!(
                "Single-use invitation; expires at {clock} (local time, in 10m 00s). Do not \
                 share it or save it in logs."
            )
        );

        // A terminal that starts a few seconds after the code was minted reads
        // the smaller budget, not the original one.
        assert!(single_use_notice(Some(now + 600), now + 3).contains("in 9m 57s"));
        // No deadline given is not a deadline of 0.
        assert_eq!(
            single_use_notice(None, now),
            "Single-use invitation. Do not share it or save it in logs."
        );
        // A Desktop clock set absurdly far out keeps the notice printable.
        assert_eq!(local_wall_clock(i64::MAX), None);
        assert_eq!(
            single_use_notice(Some(i64::MAX), now),
            format!(
                "Single-use invitation; expires at Unix time {}. Do not share it or save it in \
                 logs.",
                i64::MAX
            )
        );
        // A machine whose clock runs ahead of the Desktop's still names the
        // deadline rather than printing "in 0s".
        assert_eq!(
            single_use_notice(Some(now + 600), now + 660),
            format!(
                "Single-use invitation; expires at {clock} (local time). Do not share it or save \
                 it in logs."
            )
        );
    }
}
