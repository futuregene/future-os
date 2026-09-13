//! Foreground Desktop without a window or WebView. Business and remote
//! protocol implementations are the same ones used by the graphical Desktop.
mod agent;

use std::io::{IsTerminal, Write};
use std::time::Duration;

use crate::{future_login, remote, AppError};

pub const HELP: &str = "Usage: futureos [--headless [--no-qr] [--re-pair]]

  --headless  Run Desktop's mobile remote entry in this terminal; Ctrl+C stops it.
              Guide platform login and phone pairing when needed; reuse an existing pairing.
  --no-qr     Print authorization/pairing links instead of terminal QR codes.
  --re-pair   Explicitly revoke the saved phone pairing and create a new invitation.
  --help      Show this help without starting Desktop or Agent.

Login uses your phone/computer browser; phone pairing uses the FutureOS app.
First-time setup requires an interactive terminal. No background service is installed.";

#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    Gui,
    Headless(Options),
    Help,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub no_qr: bool,
    pub re_pair: bool,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Launch, String> {
        if args.iter().any(|arg| arg == "--help" || arg == "-h") {
            return Ok(Launch::Help);
        }
        if args.is_empty() {
            return Ok(Launch::Gui);
        }
        let mut headless = false;
        let mut options = Self::default();
        for arg in args {
            match arg.as_str() {
                "--headless" => headless = true,
                "--no-qr" => options.no_qr = true,
                "--re-pair" => options.re_pair = true,
                // macOS Finder can pass its process serial number at launch.
                value if value.starts_with("-psn_") && args.len() == 1 => return Ok(Launch::Gui),
                _ => return Err(format!("Unknown option: {arg}")),
            }
        }
        if !headless {
            return Err("--no-qr and --re-pair require --headless.".into());
        }
        Ok(Launch::Headless(options))
    }
}

/// A single process-lifetime runtime; no Tauri Builder, window, plugin or
/// event loop is constructed on this path.
pub fn run(options: Options) -> Result<(), String> {
    let _guard = crate::instance::InstanceGuard::acquire().map_err(|e| e.to_string())?;
    crate::install_rustls_provider();
    prepare_terminal();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    crate::runtime::set(runtime.handle().clone());
    let result = runtime.block_on(async {
        let signals = ShutdownSignals::new()?;
        let mut agent = agent::Agent::default();
        let result = until_shutdown(signals.wait(), session(&options, &mut agent)).await;
        eprintln!("Stopping remote access...");
        remote::stop_gracefully("headless_exit").await;
        agent.shutdown().await;
        result
    });
    // Do not let a stuck blocking task make Ctrl+C wait forever.
    runtime.shutdown_timeout(Duration::from_secs(2));
    result.map_err(|e| e.to_string())
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

async fn session(options: &Options, agent: &mut agent::Agent) -> Result<(), AppError> {
    eprintln!("FutureOS headless Desktop - foreground mode. Press Ctrl+C to stop.");
    // Fail on corrupt credentials rather than silently replacing the account.
    crate::auth_store::read()?;
    crate::future_platform::apply_channel_environment_default()?;
    let paired = crate::remote_host::pairing::load_creds_checked()?.is_some();
    if !paired || options.re_pair || future_login::future_api_key().is_err() {
        require_terminal()?;
    }
    crate::store::initialize_app_store()?;
    agent.ensure_running().await?;
    ensure_login(options).await?;
    let account_key = future_login::future_api_key()?;
    let platform = crate::future_platform::current_platform_url();
    eprintln!("Platform login authorized ({platform}).");
    // Fresh servers have no WebView to trigger the usual initial model sync.
    if let Err(error) = crate::agent_bridge::sync_future_models().await {
        eprintln!(
            "Model catalog refresh failed; existing configured models remain available: {error}"
        );
    }

    if options.re_pair {
        require_terminal()?;
        eprintln!("Revoking the saved phone pairing (--re-pair).");
        remote::unpair().await?;
    }
    // Same background projections, approvals and persistence paths as Desktop.
    crate::agent_bridge::seed_observers_from_store();
    crate::agent_bridge::spawn_provider_config_observer();
    crate::agent_bridge::spawn_session_events_observer();
    crate::agent_bridge::spawn_session_discovery();
    crate::agent_bridge::spawn_delete_outbox_worker();
    crate::agent_bridge::spawn_active_run_watchdog();
    tokio::spawn(async {
        crate::agent_bridge::import_missing_sessions().await;
        crate::agent_bridge::reconcile_interrupted_runs().await;
        crate::agent_bridge::reconcile_pending_approvals().await;
    });

    eprintln!("Preparing the remote connection...");
    let mut status = remote::start(remote::RemoteStartInput {}).await?;
    let mut shown_invitation = None;
    let mut invitation_expiry = None;
    let mut previous_state = String::new();
    let mut was_paired = false;
    loop {
        agent.check_running()?;
        // Logout/account/environment changes never leave an old account's
        // remote ingress open until its JWT eventually expires.
        if future_login::future_api_key().ok().as_deref() != Some(account_key.as_str())
            || crate::future_platform::current_platform_url() != platform
        {
            return Err("Platform login changed or was removed. Remote access is closing; restart to authorize again.".into());
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
            return Err(format!("Remote connection stopped ({:?}). Check platform login/network, then restart; use --re-pair only to replace a phone pairing.", status.reason).into());
        }
        invitation_expiry = status.pairing_code_expires_at.or(invitation_expiry);
        if let Some(link) = pairing_link(&status) {
            require_terminal()?;
            if shown_invitation.as_ref() != Some(&link) {
                print_invitation(
                    "PHONE PAIRING - scan in the FutureOS app, or paste this complete link",
                    &link,
                    options,
                )?;
                eprintln!("Single-use invitation; expires at Unix time {}. Do not share it or save it in logs.", status.pairing_code_expires_at.unwrap_or_default());
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

fn require_terminal() -> Result<(), AppError> {
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        Ok(())
    } else {
        Err("Login/pairing requires an interactive terminal. Run `futureos --headless` as the same user first; authorization links are not written to redirected logs.".into())
    }
}

async fn ensure_login(options: &Options) -> Result<(), AppError> {
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
        options,
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
    mut interval: Duration,
    expires: Duration,
    mut poll: F,
) -> Result<(), AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<future_login::FutureLoginPoll, AppError>>,
{
    tokio::time::timeout(expires, async {
        loop {
            tokio::time::sleep(interval).await;
            let response = poll().await?;
            match response.status.as_str() {
                "authorized" => return Ok(()),
                "pending" => {}
                "slow_down" => interval = interval.saturating_add(Duration::from_secs(5)),
                "denied" => return Err("Platform authorization was denied.".into()),
                "expired" => {
                    return Err("Platform authorization expired. Restart to try again.".into())
                }
                _ => return Err("Platform authorization failed. Restart to try again.".into()),
            }
        }
    })
    .await
    .map_err(|_| AppError::from("Platform authorization expired. Restart to try again."))?
}

fn print_invitation(label: &str, link: &str, options: &Options) -> Result<(), AppError> {
    let columns = terminal_size::terminal_size().map(|(width, _)| usize::from(width.0));
    let output = invitation_text(label, link, !options.no_qr, columns);
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn invitation_text(label: &str, link: &str, qr: bool, columns: Option<usize>) -> String {
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

fn pairing_link(status: &remote::RemoteStatus) -> Option<String> {
    if status.phase != remote::RemotePhase::Ready || !status.agent_available {
        return None;
    }
    let code = status.pairing_code.as_ref()?;
    if !status
        .pairing_code_expires_at
        .is_some_and(|expiry| expiry > unix_seconds())
    {
        return None;
    }
    let mut url = reqwest::Url::parse("futureos://remote/pair").expect("constant pairing URI");
    url.query_pairs_mut()
        .append_pair("code", code)
        .append_pair("desktopId", &status.desktop_id)
        .append_pair("desktopKey", &status.desktop_public_key);
    Some(url.to_string())
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
    fn launch_is_opt_in_and_qr_is_default() {
        assert_eq!(parse(&[]).unwrap(), Launch::Gui);
        assert_eq!(
            parse(&["--headless"]).unwrap(),
            Launch::Headless(Options::default())
        );
        assert_eq!(
            parse(&["--headless", "--re-pair", "--no-qr"]).unwrap(),
            Launch::Headless(Options {
                no_qr: true,
                re_pair: true
            })
        );
        assert!(parse(&["--re-pair"]).is_err());
        assert!(parse(&["--headles"]).is_err());
        assert_eq!(parse(&["--help"]).unwrap(), Launch::Help);
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
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(11));
    }

    #[tokio::test(start_paused = true)]
    async fn login_denial_expiry_and_http_failure_do_not_authorize() {
        for status in ["denied", "expired", "error"] {
            assert!(
                wait_for_login(Duration::from_secs(1), Duration::from_secs(5), || async {
                    Ok(future_login::FutureLoginPoll {
                        status: status.into(),
                        message: None,
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
            })
        })
        .await
        .unwrap_err();
        assert!(expired.to_string().contains("expired"));
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
            phase: remote::RemotePhase::Ready,
            reason: None,
            recovery: None,
            agent_available: true,
            nats_url: String::new(),
            pair_id: "pair".into(),
            pairing_code: Some("a+b&c".into()),
            pairing_code_expires_at: Some(unix_seconds() + 300),
            desktop_id: "desktop_test".into(),
            desktop_public_key: "UTEST".into(),
            web_url: None,
            web_lan_url: None,
            warning_code: None,
        };
        let link = pairing_link(&status).unwrap();
        let url = reqwest::Url::parse(&link).unwrap();
        let fields = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(fields["code"], "a+b&c");
        assert_eq!(fields["desktopId"], "desktop_test");
        assert_eq!(fields["desktopKey"], "UTEST");
        assert!(qrcode::QrCode::new(link.as_bytes()).is_ok());
        status.phase = remote::RemotePhase::Reconnecting;
        assert!(pairing_link(&status).is_none());
        status.phase = remote::RemotePhase::Ready;
        status.pairing_code_expires_at = Some(unix_seconds() - 1);
        assert!(pairing_link(&status).is_none());
    }
}
