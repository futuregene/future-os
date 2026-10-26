//! Foreground CLI application entry backed by the shared GUI-free Host library.
mod agent;

use std::process::Stdio;
use std::time::Duration;

use future_host::{future_login, remote, Error as AppError};

/// Worker threads for this entry's runtime and its embedded Agent's runtime.
///
/// Two workers left no margin. The session loop, the readiness probe, the
/// remote bridge and the store all share the host runtime, and synchronous file
/// I/O on the data root (session database, credential store, log file — an NFS
/// home on a server) can park a worker in an uninterruptible write. With both
/// workers parked the runtime cannot poll the probe's deadline, and a healthy
/// Agent is then reported as "handshake timed out". Five keeps the loop and the
/// probe running while several blocking operations are in flight.
const RUNTIME_WORKER_THREADS: usize = 5;

pub const HELP: &str = "Usage: future host [--relay URL [--relay-ca PEM]] [--help]

Run this machine as a FutureOS host in this terminal; Ctrl+C stops it.
It serves this machine's Agent, local runs and tasks; signing in to FutureOS
additionally serves the paired remote desktops (a phone, or another computer)
and reuses an existing pairing. With --relay, no Future account is needed at all.

  --relay URL Print an invitation that points at a self-hosted relay
              (future-relay) instead of the Future platform. The address is
              remembered for later launches; pass an empty value to go back.
  --relay-ca  PEM file with the relay's CA, when its certificate is not issued
              by a public CA.
  --help      Show this help without starting Host or Agent.

No Future account is required to run this entry: a signed-out machine serves
everything local with its own providers, and its remote desktop stays closed
until an account signs in. Sign in from another terminal with `future auth
login` (it prints the authorization link and user code) — this entry picks the
account up and starts serving the remote desktop without a restart. A different
login, or a logout, while it is running closes remote access and asks for a
restart. With --relay there is no account at all.

Signed out, everything that needs the platform is refused together, while nothing
local is: `future remote devices add` cannot mint an invitation (the platform
issues those, so it asks you to run `future auth login` first), and the remote
desktop switch can be turned on without effect. Sessions, runs, tasks and your
own model providers keep working throughout — a refusal there is not a broken
host.

Pair a device with `future remote devices add` — from another terminal, while
this entry is running: the invitation's link and QR code are printed in the
terminal that asked, never here. Dropping one or choosing which one is in
control is the same group (`future remote devices remove|use`).

The entry serves the machine's remote desktop, and the remote desktop switch is
shared with the window's Remote page: with the switch off, it starts (and stays)
idle instead of exiting, so `future remote enable` turns it back on without a
restart.";

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
    future_host::environment::prepare_process();
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    future_host::cli_tool::prepare_path_for(&executable).map_err(|e| e.to_string())?;
    let _guard = future_host::instance::InstanceGuard::acquire().map_err(|e| e.to_string())?;
    // Applied before anything reads the pairing store: a `--relay` on this
    // launch decides which relay the invitation it prints will point at.
    future_host::remote::relay::apply_launch_settings(
        options.relay.as_deref(),
        options.relay_ca.as_deref(),
    );
    // A read-only install or a full disk must not stop a headless host: the
    // terminal still reports what happens, and the log is a convenience, not a
    // prerequisite.
    if let Err(error) = future_host::environment::start_logging() {
        future_host::environment::report(&format!("host log unavailable: {error}"));
    }
    // Say where diagnostics land before any Agent work starts — after the data
    // root is final, because hydration above can still change it. The Agent logs
    // each startup phase to that file as well as this terminal, and on a host
    // that kills the process for exceeding a memory or per-user CPU budget the
    // file is the only surviving evidence.
    eprintln!(
        "FutureOS: startup progress is logged to {} and mirrored in this terminal.",
        future_host::environment::log_file_path().display()
    );
    prepare_terminal();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(RUNTIME_WORKER_THREADS)
        .max_blocking_threads(32)
        .thread_stack_size(2 * 1024 * 1024)
        .build()
        .map_err(|e| e.to_string())?;
    future_host::runtime::set(runtime.handle().clone());
    let result = runtime.block_on(async {
        let signals = ShutdownSignals::new()?;
        let mut agent = agent::Agent::default();
        let (result, cause) = until_shutdown(signals.wait(), session(&mut agent)).await;
        // Cleanup is captured rather than `?`-ed, so a failed shutdown cannot
        // discard the reason the serving loop ended — see `exit_outcome`.
        let cleanup = agent.shutdown(cause.reason()).await;
        exit_outcome(result, cleanup)
    });
    // Do not let a stuck blocking task make Ctrl+C wait forever.
    runtime.shutdown_timeout(Duration::from_secs(2));
    result.map_err(|e| e.to_string()).map_err(with_log_hint)
}

/// The one report a foreground run leaves behind: why the serving loop ended and
/// whether the Agent then shut down cleanly.
///
/// Both halves are reported when both failed. The shutdown used to be `?`-ed
/// before the loop's own result was read, so a cleanup that ran out of its budget
/// replaced the whole report: the operator saw "the Agent failed to stop" and
/// learned nothing about what stopped the run in the first place — measured on a
/// loaded host, that was the only line left of a run that quit because the Agent
/// stopped answering. Neither half alone is actionable.
///
/// Exit status is unchanged: a failure in either half is a failure of the entry,
/// and a run where both succeeded is still `Ok`.
fn exit_outcome(
    loop_result: Result<(), AppError>,
    cleanup: Result<(), AppError>,
) -> Result<(), AppError> {
    match (loop_result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        // One half failed: it is the report on its own, unchanged, so the ordinary
        // single-failure message gains no sentence it did not have before.
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        // Cause first: it is why the entry stopped at all. The cleanup failure
        // follows as a complication of stopping, not as the reason for it.
        (Err(loop_error), Err(cleanup_error)) => {
            Err(format!("{loop_error}\nThe Agent also failed to shut down: {cleanup_error}").into())
        }
    }
}

/// A failure message this entry point cannot act on is still one the operator
/// must be able to finish: name the log that holds the preceding startup phases
/// whenever one was written.
fn with_log_hint(message: String) -> String {
    let log = future_host::environment::log_file_path();
    if log.is_file() {
        format!(
            "{message}\nThe Agent's last startup phase and any failure are in {}.",
            log.display()
        )
    } else {
        message
    }
}

/// What ended this entry's foreground run.
///
/// A login node can take the process away without the operator asking: an SSH
/// session closes, a scheduler reclaims the node, a supervisor sends SIGTERM.
/// Every one of those arrives on the same shutdown path, so without this the log
/// of "the host stopped at 13:17" is identical for all of them and the cause is
/// only inferable afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopCause {
    /// The serving loop returned on its own — a reported failure, not a signal.
    Session,
    /// Ctrl+C in the foreground terminal.
    Interrupt,
    /// SIGTERM: `systemctl stop`, a scheduler, or a process supervisor.
    #[cfg(unix)]
    Terminate,
    /// SIGHUP: the terminal closed or the SSH session went away.
    #[cfg(unix)]
    Hangup,
    /// The console window was closed (Windows).
    #[cfg(windows)]
    Close,
}

impl StopCause {
    /// The shared stop operation's reason, so a cleanup that runs out of its
    /// budget reports which signal it was finishing.
    fn reason(self) -> &'static str {
        match self {
            Self::Session => "headless_exit",
            Self::Interrupt => "headless_sigint",
            #[cfg(unix)]
            Self::Terminate => "headless_sigterm",
            #[cfg(unix)]
            Self::Hangup => "headless_sighup",
            #[cfg(windows)]
            Self::Close => "headless_console_close",
        }
    }

    /// What to call it on the terminal, where the operator reads the cause once.
    fn describe(self) -> &'static str {
        match self {
            Self::Session => "an internal stop",
            Self::Interrupt => "SIGINT (Ctrl+C)",
            #[cfg(unix)]
            Self::Terminate => "SIGTERM",
            #[cfg(unix)]
            Self::Hangup => "SIGHUP (the terminal or SSH session ended)",
            #[cfg(windows)]
            Self::Close => "a console-close event",
        }
    }
}

/// Whichever comes first, together with what ended the run. The signal arm
/// carries its own cause so the cleanup that follows can record it; the session
/// arm is this entry's own loop returning, which is not a signal at all.
async fn until_shutdown(
    shutdown: impl std::future::Future<Output = StopCause>,
    session: impl std::future::Future<Output = Result<(), AppError>>,
) -> (Result<(), AppError>, StopCause) {
    tokio::select! {
        biased;
        cause = shutdown => (Ok(()), cause),
        result = session => (result, StopCause::Session),
    }
}

async fn session(agent: &mut agent::Agent) -> Result<(), AppError> {
    eprintln!("FutureOS host - foreground mode. Press Ctrl+C to stop.");
    // Fail on corrupt credentials rather than silently replacing the account.
    future_host::auth_store::read()?;
    // The Future account is what the *remote desktop* runs on; it is not a
    // prerequisite for this entry point. A machine signed out — one that only
    // has its own providers configured, or that will be signed in from another
    // terminal in a moment — serves local runs, tasks and the Agent normally and
    // keeps the remote desktop closed until an account appears. Signing in is
    // `future auth login`'s job; this run adopts it without a restart (see
    // [`account_action`]).
    //
    // A self-hosted relay is the other answer to "which account": it has none.
    let relay = future_host::remote::relay::endpoint();
    future_host::store::initialize_app_store()?;
    // Leftover convergence walks the data tree and no command depends on it, so
    // it must not sit between the operator and the Agent starting.
    std::thread::spawn(future_host::store::reconcile_after_startup);
    ensure_agent_running(agent).await?;
    install_channel_runtime();
    // A key the platform already refused is not offered again. It is the one
    // credential this run knows to be dead, so adopting it would only repeat a
    // rejection; a *different* key is a new login. Filled in below and never
    // afterwards: a rejection that arrives as a remote phase while an account is
    // pinned still stops the run, exactly as it always did.
    let mut refused: Option<String> = None;
    let mut account = if let Some(url) = relay.as_deref() {
        eprintln!(
            "Self-hosted relay ({url}). No Future account is used for phone pairing; \
             existing pairings keep using the relay that issued them."
        );
        None
    } else {
        // Exactly the check this entry made before it served a signed-in machine:
        // a key the platform still accepts is reused silently, a key it refuses is
        // named as such, and a transport failure is not a logout. Only the step
        // that used to follow a refusal — this entry's own device-code login — is
        // gone: signing in is `future auth login`'s job now, and this run waits for
        // it (see [`account_action`]) instead of replacing itself with a login
        // prompt.
        match future_login::future_api_key() {
            Ok(key) => match future_login::fetch_profile().await {
                Ok(_) => {
                    let account = Account::pin(key);
                    eprintln!("Platform account signed in ({}).", account.platform);
                    refresh_catalogue().await;
                    Some(account)
                }
                Err(AppError::Remote {
                    status: 401 | 403, ..
                }) => {
                    eprintln!("Platform authorization is no longer valid. Please sign in again.");
                    refused = Some(key);
                    None
                }
                // Unchanged: a network failure while verifying is not a logout and
                // is not silently downgraded to "signed out" either.
                Err(error) => return Err(error),
            },
            Err(_) => {
                eprintln!(
                    "Not signed in to FutureOS: serving local runs, tasks and the Agent. The \
                     remote desktop stays closed until an account signs in — run `future auth \
                     login` from another terminal and this entry starts serving it without a \
                     restart."
                );
                None
            }
        }
    };
    // The window's fixed-interval maintenance (app updates and the balance
    // event need a WebView; see `scheduler::start_headless`).
    future_host::scheduler::start_headless();
    // Built-in skills are installed by the window's post-login onboarding. A
    // server never renders that flow, so it runs the same CLI step itself.
    future_host::runtime::spawn(bootstrap_builtin_skills());
    // Same background projections, approvals and persistence paths as Desktop.
    // Headless has no active-thread UI hint, so its compatibility monitor
    // observes only sessions the Agent reports as actively streaming.
    future_host::agent_bridge::spawn_streaming_observer_monitor();
    future_host::agent_bridge::spawn_provider_config_observer();
    future_host::agent_bridge::spawn_session_events_observer();
    future_host::agent_bridge::spawn_session_discovery();
    future_host::agent_bridge::spawn_delete_outbox_worker();
    future_host::agent_bridge::spawn_active_run_watchdog();
    // User-defined tasks run here too: headless desktop has no webview, so the
    // executor's notifications downgrade to ledger-only (the run rows are the
    // signal). This is what lets a server box run tasks with no GUI attached.
    future_host::tasks::start_headless();
    future_host::runtime::spawn(async {
        future_host::agent_bridge::reconcile_interrupted_runs().await;
        future_host::agent_bridge::reconcile_pending_approvals().await;
    });

    // The switch is the user's decision and this entry point obeys it: a machine
    // whose remote desktop was turned off (in the window, or with `future remote
    // disable`) must not come back serving just because a server restarted. A
    // fresh install defaults to on, so the ordinary launch is unchanged.
    let switch_on = remote_switch_on();
    // `enable`, not `start`: a start mints an invitation when nothing is paired,
    // and printing that code is a decision the operator makes explicitly by
    // running `future remote devices add` — not something a host prints the
    // moment it is launched. This entry therefore serves the devices already in
    // the pairing book and grows the book only on request.
    let mut status = if !switch_on {
        eprintln!(
            "The remote desktop switch is off, so this entry serves nothing yet. It stays up as \
             the machine's control endpoint: turn the switch on with `future remote enable` \
             (no restart needed), or in the window's Remote page."
        );
        remote::status()
    } else if account.is_none() && relay.is_none() {
        // Signed out and no relay: nothing to serve the remote desktop with, and
        // that is not a fault. The loop below turns it on the moment an account
        // signs in.
        remote::status()
    } else {
        eprintln!("Preparing the remote connection...");
        remote::enable().await?
    };
    // `future remote devices add` asks this process for an invitation: only a
    // process that can serve the new pairing may mint one, and a server has no
    // window to print it in — the terminal that asked does. Started after the
    // remote entry, so a request that arrives during startup is answered by a
    // Desktop that is actually serving rather than refused as "off".
    future_host::remote::control::spawn_host();
    let mut previous_state = String::new();
    let mut was_paired = false;
    let mut said_idle = false;
    let mut said_unpaired = false;
    let mut said_signed_out = false;
    loop {
        agent.check_running()?;
        // The Agent may live in another process (an external `future agent`, or a
        // Desktop that started it), in which case `check_running` — which only
        // reads this host's own phase — stays silent forever. Probing the
        // endpoint is what notices that it died, and starting a replacement (or
        // waiting for the process that holds the lock) is what keeps this entry
        // serving instead of answering every RPC with a transport error.
        agent.supervise().await?;
        // The account this run serves is pinned (`Account`). Relay mode has none
        // to watch; a *different* key closes remote access instead of leaving the
        // old account's ingress open until its JWT expires; and a login that
        // appears while this run was local-only is adopted right here — that is
        // what makes `future auth login` under a running `future host` take
        // effect without a restart.
        if relay.is_none() {
            let stored = future_login::future_api_key().ok();
            let platform = future_host::future_platform::current_platform_url();
            match account_action(
                account.as_ref(),
                stored.as_deref(),
                &platform,
                refused.as_deref(),
            ) {
                AccountAction::Keep => {}
                AccountAction::Close => return Err(
                    "Platform login changed or was removed. Remote access is closing; restart to \
                     authorize again."
                        .into(),
                ),
                AccountAction::Adopt => {
                    let key = stored.expect("adoption needs a stored key");
                    let adopted = Account::pin(key);
                    eprintln!(
                        "Platform account signed in ({}): starting the remote desktop for it.",
                        adopted.platform
                    );
                    account = Some(adopted);
                    previous_state.clear();
                    said_signed_out = false;
                    status = if remote_switch_on() {
                        remote::enable().await?
                    } else {
                        remote::status()
                    };
                    refresh_catalogue().await;
                }
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
        // What this run has to serve the remote desktop with: an account, or a
        // relay that supplies its own. Nothing else can authorize a device.
        let serving = relay.is_some() || account.is_some();
        // The three terminal-looking phases are not one state, and collapsing
        // them would hide the difference the operator has to act on:
        //
        // * `Failed(AccountAuthorization)` **without** an account is this run's
        //   ordinary signed-out state (the remote desktop was switched on by
        //   `future remote enable` while nothing could serve it). It is not a
        //   fault: say it once and keep serving locally until a login appears.
        // * The very same phase **with** an account pinned means the platform
        //   refused this machine's credential. That is a fault, and a signed-in
        //   run stops on it exactly as it always has.
        // * `Revoked` and every other `Failed` need a person either way, and they
        //   stop the run with the reason named — unchanged.
        match phase_action(serving, status.phase, status.reason) {
            PhaseAction::Keep => {}
            PhaseAction::NeedsAccount => {
                if !said_signed_out {
                    said_signed_out = true;
                    eprintln!(
                        "The remote desktop needs a FutureOS account: sign in with `future auth \
                         login` (from another terminal) and this entry starts serving it without a \
                         restart. Everything local keeps working meanwhile."
                    );
                }
            }
            PhaseAction::Stop => {
                return Err(format!("Remote connection stopped ({:?}). Check platform login/network, then restart; use `future remote devices` to pair or drop a device.", status.reason).into());
            }
        }
        if serving {
            said_signed_out = false;
        }
        // A stop is only a fault while the switch says on. Turning the switch off
        // is the user's decision: a server told to stop serving stays up and keeps
        // answering control requests — that is what lets `future remote enable`
        // turn it back on without a restart — instead of exiting as if it broke.
        let switch_on = remote_switch_on();
        let idle = matches!(status.phase, remote::RemotePhase::Stopped) && !switch_on;
        if idle {
            was_paired = false;
            if !said_idle {
                eprintln!(
                    "Remote desktop is off (the switch is off). Turn it on with `future remote \
                     enable` — this entry keeps running and serves again without a restart."
                );
                said_idle = true;
            }
        } else {
            said_idle = false;
        }
        if matches!(status.phase, remote::RemotePhase::Stopped) && switch_on && serving {
            return Err(format!("Remote connection stopped ({:?}). Check platform login/network, then restart; use `future remote devices` to pair or drop a device.", status.reason).into());
        }
        let paired = future_host::remote::pairing::load_creds_checked()?.is_some();
        if paired && !was_paired && status.phase == remote::RemotePhase::Ready {
            eprintln!("Phone pairing saved. Remote access is open; connect with the paired FutureOS app. Ctrl+C closes this entry.");
            was_paired = true;
        }
        // Nothing paired and nothing to serve is the ordinary first launch: say
        // which command pairs a device instead of minting a code nobody asked
        // for. `Enabled` is exactly "on, serving nothing yet" — the switch is on
        // and the book is empty.
        if paired {
            said_unpaired = false;
        } else if !said_unpaired && status.phase == remote::RemotePhase::Enabled {
            eprintln!(
                "No device is paired yet. Pair one from another terminal with `future remote \
                 devices add` — this entry keeps running and starts serving as soon as a phone \
                 claims the invitation."
            );
            said_unpaired = true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        status = remote::status();
    }
}

/// How long the owned Agent may take before the wait is worth explaining.
const SLOW_STARTUP_NOTICE: Duration = Duration::from_secs(30);

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
        &future_host::environment::log_file_path(),
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

/// The Future account a run serves: the key the platform issued and the platform
/// it was issued for, pinned when the run adopted it.
///
/// Pinned rather than re-read for its own value: a *different* key on disk is a
/// different account, and this run closes its remote access instead of silently
/// serving it (the old account's ingress must not stay open until its JWT
/// expires). A run that has not adopted one has no account — the ordinary
/// local-only state on a machine whose providers are its own.
struct Account {
    key: String,
    platform: String,
}

impl Account {
    fn pin(key: String) -> Self {
        Self {
            key,
            platform: future_host::future_platform::current_platform_url(),
        }
    }
}

/// What the supervision loop must do with the account on this iteration.
#[derive(Debug, PartialEq, Eq)]
enum AccountAction {
    /// Nothing changed: keep serving what is pinned.
    Keep,
    /// No account is pinned and a usable one is stored: adopt it and serve it.
    Adopt,
    /// A pinned account changed or disappeared: close remote access.
    Close,
}

/// Decide from the pinned account, the key stored on disk and the platform this
/// process resolves which of the three transitions applies.
///
/// `rejected` is the key the platform already refused: it is *stored* but not
/// offered again, so a refused credential cannot turn into a per-second retry
/// loop. Any other key is a new login and is adopted.
fn account_action(
    pinned: Option<&Account>,
    stored_key: Option<&str>,
    platform: &str,
    rejected: Option<&str>,
) -> AccountAction {
    let available = stored_key.filter(|key| Some(*key) != rejected);
    match (pinned, available) {
        (Some(pinned), Some(key)) if pinned.key == key && pinned.platform == platform => {
            AccountAction::Keep
        }
        (Some(_), _) => AccountAction::Close,
        (None, Some(_)) => AccountAction::Adopt,
        (None, None) => AccountAction::Keep,
    }
}

/// Fetch the platform's model catalogue for the account this run just adopted.
///
/// Best-effort: the Agent's own provider list stays valid without it, and a
/// server whose platform is unreachable must not stop over a catalogue refresh.
async fn refresh_catalogue() {
    if let Err(error) = future_host::scheduler::refresh_future_models_now().await {
        eprintln!(
            "Model catalog refresh failed; existing configured models remain available: {error}"
        );
    }
}

/// What one supervision iteration must do about the remote phase it just read.
#[derive(Debug, PartialEq, Eq)]
enum PhaseAction {
    /// Not terminal for this run: serving, connecting, reconnecting, idle.
    Keep,
    /// Signed out and the remote desktop has nothing to serve it with. Say so
    /// once and keep running locally; a login can still arrive.
    NeedsAccount,
    /// Stop the run, naming the phase's reason.
    Stop,
}

/// Decide what a remote phase means for this run.
///
/// `serving` is whether this run has anything to authorize a device with — a
/// pinned Future account, or a relay that supplies its own. It is the *only*
/// thing that changes the answer, and it must: `Failed(AccountAuthorization)`
/// on a signed-out run is "nothing to serve with yet", while the same phase on a
/// signed-in run is the platform refusing this machine's credential. `Revoked`
/// and every other `Failed` stop the run either way.
fn phase_action(
    serving: bool,
    phase: remote::RemotePhase,
    reason: Option<remote::RemoteFailureReason>,
) -> PhaseAction {
    use remote::{RemoteFailureReason, RemotePhase};
    match phase {
        RemotePhase::Failed
            if !serving && reason == Some(RemoteFailureReason::AccountAuthorization) =>
        {
            PhaseAction::NeedsAccount
        }
        RemotePhase::Failed | RemotePhase::Revoked => PhaseAction::Stop,
        // `Stopped` is not terminal here: the switch can turn it back on from
        // another terminal, and the loop reports the idle state itself.
        RemotePhase::Stopped
        | RemotePhase::Enabled
        | RemotePhase::Connecting
        | RemotePhase::Ready
        | RemotePhase::Reconnecting
        | RemotePhase::Refreshing => PhaseAction::Keep,
    }
}

/// The remote desktop switch, as the user last set it. Defaults to on: a fresh
/// install serves the paired devices, and only an explicit `disable` opts out.
fn remote_switch_on() -> bool {
    future_host::store::get_app_settings()
        .map(|settings| settings.auto_connect_remote)
        .unwrap_or(true)
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
    let executable = match maintenance_executable() {
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
    future_host::agent_bridge::refresh_skills().await;
}

fn prepare_terminal() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, GetStdHandle, SetConsoleMode, SetConsoleOutputCP,
            ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
        };
        let _ = SetConsoleOutputCP(65001);
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
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

    async fn wait(mut self) -> StopCause {
        #[cfg(unix)]
        let cause = tokio::select! {
            _ = self.interrupt.recv() => StopCause::Interrupt,
            _ = self.terminate.recv() => StopCause::Terminate,
            _ = self.hangup.recv() => StopCause::Hangup,
        };
        #[cfg(windows)]
        let cause = tokio::select! {
            _ = self.interrupt.recv() => StopCause::Interrupt,
            _ = self.close.recv() => StopCause::Close,
        };
        // Before the first cleanup stage: several of the lines that follow look
        // like faults (a cancelled run, a rebuilt remote generation, a cleanup
        // deadline), and none of them say why the entry decided to stop. Mirrored
        // into the host log, because a detached terminal keeps no scrollback.
        future_host::environment::report(&format!(
            "received {}; stopping. This is a clean shutdown request, not a crash: cleanup stages \
             follow and a failure in them is reported as this exit's error.",
            cause.describe()
        ));
        cause
    }
}

fn install_channel_runtime() -> future_host::agent_client::RuntimeInstall {
    future_host::agent_client::provide_channel_runtime(
        "agent-channel-runtime",
        agent_channel_driver,
    )
}

fn stop(reason: &'static str) -> Result<(), AppError> {
    future_host::shutdown::begin(reason, move |operation| {
        let mut services = future_host::shutdown::ServiceStop::prepare(operation, reason, || {})
            .map_err(|error| error.to_string())?;
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => Some(runtime),
            Err(error) => {
                services.runtime_unavailable(error);
                None
            }
        };
        if let Some(runtime) = &runtime {
            runtime.block_on(services.disconnect());
        }
        if let Some(runtime) = runtime {
            runtime.shutdown_timeout(operation.remaining(Duration::from_secs(1)));
        }
        services.finish().map_err(|error| error.to_string())
    })
    .wait()
    .map_err(Into::into)
}

fn agent_channel_driver() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name("agent-channel")
        .build()
        .expect("build agent channel runtime")
}

#[cfg(not(test))]
fn maintenance_executable() -> Result<std::path::PathBuf, AppError> {
    std::env::current_exe().map_err(Into::into)
}
#[cfg(test)]
fn maintenance_executable() -> Result<std::path::PathBuf, AppError> {
    future_host::test_support::configure_entry_fixture();
    future_host::cli_tool::resolve_for("skills.init")
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

    /// How a supervisor takes the host away without the operator asking:
    /// `systemctl stop` / a scheduler (SIGTERM) where there are unix signals, the
    /// console-close event where there are not. Which one exists differs by
    /// platform, and each must reach the cleanup as the cause and as its own
    /// reason — naming the signal is the whole point of it.
    #[cfg(unix)]
    const SUPERVISOR_STOP: (StopCause, &str) = (StopCause::Terminate, "headless_sigterm");
    #[cfg(windows)]
    const SUPERVISOR_STOP: (StopCause, &str) = (StopCause::Close, "headless_console_close");

    #[tokio::test]
    async fn interrupt_drops_pending_startup_before_it_can_complete() {
        let completed = std::sync::atomic::AtomicBool::new(false);
        let (stop, reason) = SUPERVISOR_STOP;
        let (result, cause) = until_shutdown(async { stop }, async {
            completed.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        })
        .await;
        result.unwrap();
        // The signal is what the cleanup records as the stop reason, so a
        // cleanup deadline names the signal instead of a generic exit.
        assert_eq!(cause, stop);
        assert_eq!(cause.reason(), reason);
        assert!(!completed.load(std::sync::atomic::Ordering::Relaxed));
    }

    /// A cleanup failure must not erase why the serving loop ended.
    ///
    /// Both statements are needed to act: the loop's reason says what stopped the
    /// entry, and the cleanup failure says the Agent did not follow it down. Only
    /// reporting the second (what `?` on the shutdown did) leaves an operator
    /// looking at a shutdown problem that is a symptom, not the cause.
    #[test]
    fn a_failed_shutdown_reports_the_loop_cause_as_well() {
        let error = exit_outcome(
            Err("Remote connection stopped (Revoked)".into()),
            Err("Agent shutdown deadline exceeded during admitted commands".into()),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("Remote connection stopped (Revoked)"),
            "the loop's own reason must survive a failed cleanup: {error}"
        );
        assert!(
            error.contains("Agent shutdown deadline exceeded during admitted commands"),
            "the cleanup failure must still be reported: {error}"
        );
    }

    /// The control: a cleanup that succeeded leaves the message as it was.
    ///
    /// The loop's reason is reported alone, with no cleanup sentence appended —
    /// otherwise every ordinary exit would claim a shutdown problem it did not
    /// have.
    #[test]
    fn a_clean_shutdown_adds_nothing_to_the_loop_cause() {
        let error = exit_outcome(Err("setup failed".into()), Ok(()))
            .unwrap_err()
            .to_string();
        assert_eq!(error, "setup failed");
        assert!(
            !error.contains("failed to shut down"),
            "a clean shutdown must not be mentioned: {error}"
        );
    }

    /// A run whose cleanup failed but whose loop ended normally still fails, and
    /// says what went wrong in the cleanup.
    #[test]
    fn a_failed_cleanup_alone_is_still_a_failure() {
        let error = exit_outcome(Ok(()), Err("Agent failed to stop".into()))
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Agent failed to stop");
    }

    /// Only a run where both halves succeeded is a success.
    #[test]
    fn both_halves_succeeding_is_success() {
        assert!(exit_outcome(Ok(()), Ok(())).is_ok());
    }

    #[tokio::test]
    async fn startup_errors_are_not_hidden_as_success() {
        let (result, cause) =
            until_shutdown(std::future::pending(), async { Err("setup failed".into()) }).await;
        // A serving loop that returned is not a signal, and must not be reported
        // as one: the operator reads the entry's own error.
        assert_eq!(cause, StopCause::Session);
        assert_eq!(cause.reason(), "headless_exit");
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
        let _home = future_host::auth_store::test_support::HomeGuard::new("headless-log-hint");
        let log = future_host::environment::log_file_path();
        assert_eq!(with_log_hint("failed".into()), "failed");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "FutureOS: host starting\n").unwrap();
        let hinted = with_log_hint("failed".into());
        assert!(hinted.starts_with("failed\n"), "{hinted}");
        assert!(hinted.contains(&log.display().to_string()), "{hinted}");
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
        let _lock = future_host::TEST_HOME_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let _env = crate::test_env::EnvGuard::set(&[
            ("PATH", directory.as_os_str().to_owned()),
            ("FUTURE_AGENT_GRPC_ADDR", "http://[::1".into()),
        ]);
        body()
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
            future_host::cli_tool::resolve_for("skills.init").map_err(|error| error.to_string())
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
            future_host::cli_tool::resolve_for("skills.init").map_err(|error| error.to_string())
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

    fn pinned(key: &str, platform: &str) -> Account {
        Account {
            key: key.to_string(),
            platform: platform.to_string(),
        }
    }

    /// Signed out is a stable state, not a transition: a host that has no
    /// account keeps serving locally and must not re-decide every second.
    #[test]
    fn a_signed_out_run_keeps_its_local_only_state() {
        assert_eq!(
            account_action(None, None, "https://future-os.cn", None),
            AccountAction::Keep
        );
    }

    /// The login that appears under a running `future host` is adopted — this is
    /// the whole point of serving signed out: `future auth login` in another
    /// terminal takes effect without a restart.
    #[test]
    fn a_login_that_appears_is_adopted() {
        assert_eq!(
            account_action(None, Some("fresh"), "https://future-os.cn", None),
            AccountAction::Adopt
        );
    }

    /// A pinned account is never silently replaced, and a logout closes remote
    /// access: the old account's ingress must not outlive its credential.
    #[test]
    fn a_changed_or_removed_login_closes_remote_access() {
        let account = pinned("first", "https://future-os.cn");
        assert_eq!(
            account_action(Some(&account), Some("first"), "https://future-os.cn", None),
            AccountAction::Keep
        );
        assert_eq!(
            account_action(Some(&account), Some("second"), "https://future-os.cn", None),
            AccountAction::Close
        );
        assert_eq!(
            account_action(Some(&account), None, "https://future-os.cn", None),
            AccountAction::Close
        );
        // The same key against another platform is a different account.
        assert_eq!(
            account_action(
                Some(&account),
                Some("first"),
                "https://test.future-os.cn",
                None
            ),
            AccountAction::Close
        );
    }

    /// The signed-out state that has an account to say nothing about is still
    /// *serving*: every non-terminal phase must leave the run alone, or a local
    /// host would exit on its own once a second.
    #[test]
    fn ordinary_phases_never_stop_a_run() {
        use remote::RemotePhase;
        for serving in [true, false] {
            for phase in [
                RemotePhase::Stopped,
                RemotePhase::Enabled,
                RemotePhase::Connecting,
                RemotePhase::Ready,
                RemotePhase::Reconnecting,
                RemotePhase::Refreshing,
            ] {
                assert_eq!(
                    phase_action(serving, phase, None),
                    PhaseAction::Keep,
                    "{phase:?} with serving={serving} must not stop the run"
                );
            }
        }
    }

    /// The one phase whose meaning depends on whether this run has anything to
    /// serve with. Signed out it is "nothing to serve with yet" and the run keeps
    /// working locally; signed in it is the platform refusing this machine's
    /// credential, which stops the run exactly as it always has.
    #[test]
    fn account_authorization_is_fatal_only_when_an_account_is_serving() {
        use remote::{RemoteFailureReason, RemotePhase};
        assert_eq!(
            phase_action(
                false,
                RemotePhase::Failed,
                Some(RemoteFailureReason::AccountAuthorization)
            ),
            PhaseAction::NeedsAccount
        );
        assert_eq!(
            phase_action(
                true,
                RemotePhase::Failed,
                Some(RemoteFailureReason::AccountAuthorization)
            ),
            PhaseAction::Stop
        );
    }

    /// `Revoked` and the remaining failures need a person either way: neither the
    /// signed-out tolerance nor a relay may swallow them, so they are not folded
    /// into "not signed in".
    #[test]
    fn revocation_and_other_failures_still_stop_the_run() {
        use remote::{RemoteFailureReason, RemotePhase};
        for serving in [true, false] {
            assert_eq!(
                phase_action(
                    serving,
                    RemotePhase::Revoked,
                    Some(RemoteFailureReason::CredentialRevoked)
                ),
                PhaseAction::Stop
            );
            for reason in [
                RemoteFailureReason::Network,
                RemoteFailureReason::RemoteServer,
                RemoteFailureReason::CredentialExpired,
                RemoteFailureReason::ServiceAuthorization,
                RemoteFailureReason::Protocol,
                RemoteFailureReason::GenerationUnhealthy,
                RemoteFailureReason::Local,
                RemoteFailureReason::SystemSleep,
            ] {
                assert_eq!(
                    phase_action(serving, RemotePhase::Failed, Some(reason)),
                    PhaseAction::Stop,
                    "{reason:?} with serving={serving} must still stop the run"
                );
            }
        }
    }

    /// A `Failed` without an account-authorization reason is not the signed-out
    /// state, whatever `serving` says: only that exact pair is tolerated.
    #[test]
    fn a_failure_without_an_account_reason_is_not_tolerated() {
        use remote::{RemoteFailureReason, RemotePhase};
        assert_eq!(
            phase_action(false, RemotePhase::Failed, None),
            PhaseAction::Stop
        );
        assert_eq!(
            phase_action(false, RemotePhase::Failed, Some(RemoteFailureReason::Local)),
            PhaseAction::Stop
        );
    }

    /// A key the platform refused is stored but not offered again, so a dead
    /// credential cannot become a per-second retry loop; a *different* key is a
    /// new login and is adopted.
    #[test]
    fn a_rejected_key_is_not_adopted_again_but_a_new_one_is() {
        assert_eq!(
            account_action(
                None,
                Some("refused"),
                "https://future-os.cn",
                Some("refused")
            ),
            AccountAction::Keep
        );
        assert_eq!(
            account_action(None, Some("fresh"), "https://future-os.cn", Some("refused")),
            AccountAction::Adopt
        );
    }
}
