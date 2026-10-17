//! The graphical entry point's command line, and how a `--re-pair` typed into a
//! terminal reaches the Desktop that owns the pairing.
//!
//! `futureos` shows the window; the flags it accepts are about what that window
//! should do, not about how remote access is implemented. Three readers share
//! this module so they can never disagree about what a flag means:
//!
//! * the process that is starting,
//! * the Desktop that already owns the data directory, which receives a
//!   forwarded `argv` from the single-instance plugin,
//! * the process waiting for that Desktop's answer.
//!
//! That last one is why this module owns a small file protocol: the plugin's
//! channel is one-way (the launching process exits as soon as the notification is
//! written), so a `--re-pair` started from an SSH session needs an explicit place
//! for the running Desktop to put the invitation it just issued.
//!
//! This is deliberately not the headless entry point's parser: `--no-qr` exists
//! because a terminal has to print the invitation, and a window does not.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const HELP: &str = "Usage: futureos [--re-pair] [--help]

Start the graphical Desktop.

  --re-pair   Revoke the saved pairing and issue a fresh invitation, whatever
              state the previous connection is in. The new code is printed on
              this terminal: by this process when it starts Desktop, or by the
              running Desktop answering through this process. Pairing a new
              device replaces the one that was connected, so that device is cut
              off as soon as this runs — whether or not the new code is ever
              scanned.
  --help      Show this help without starting Desktop.

To disconnect a paired device without pairing another one, use the Remote page's
Unpair button instead. For phone remote access without GUI dependencies, run
futureos-headless.";

/// The internal flag carrying where the answer goes. Only this entry point writes
/// it (see [`notifier_command`]): it is how the launching process is told the
/// outcome of a re-pair it cannot perform itself.
const REPLY_FLAG: &str = "--re-pair-reply";

/// How long a launcher waits for the Desktop's answer.
///
/// Sized for the slow path, not the common one (a second or two): the re-pair
/// revokes what is stored and then issues an invitation, which is one platform
/// round trip under a 20s request timeout, followed by the NATS connect and the
/// transport readiness window (10s).
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Between two reads of the answer file. Short enough that the common case feels
/// immediate, long enough that a file read twice a second is not work.
const REPLY_POLL: Duration = Duration::from_millis(100);

/// What this launch should do beyond showing the window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    /// `--re-pair`: replace the saved pairing with a freshly issued invitation.
    pub re_pair: bool,
}

/// The parsed command line. `Help` is not a mode of running: it means the process
/// prints [`HELP`] and exits without touching any state.
#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    Gui(LaunchOptions),
    Help,
    /// Internal: hand a `--re-pair` to the Desktop that already owns the data
    /// directory, and stop. Spawned by [`notifier_command`], never typed.
    ///
    /// The process that runs this never performs a re-pair itself — it cannot,
    /// because another Desktop holds the pairing and the data directory. Its only
    /// job is to deliver the request on the channel the running instance listens
    /// to, which is its own `argv`.
    ForwardRePair {
        reply: PathBuf,
    },
}

impl Launch {
    /// Parse the flags, which are everything after `argv[0]`.
    ///
    /// Errors rather than guessing: an unrecognized flag used to be a launch
    /// failure on this entry point, and silently ignoring one would turn a typo
    /// (`--repair`) into a Desktop that starts and does nothing the user asked.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut help = false;
        let mut options = LaunchOptions::default();
        let mut reply: Option<PathBuf> = None;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--help" | "-h" => help = true,
                "--re-pair" => options.re_pair = true,
                REPLY_FLAG => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| format!("{REPLY_FLAG} needs the path to write to"))?;
                    reply = Some(PathBuf::from(value));
                }
                // Named, not generic: these are the headless entry point's flags,
                // and "unknown option" would leave the user guessing which binary
                // they wanted.
                "--headless" => {
                    return Err(
                        "The --headless option has been removed. Run futureos-headless instead."
                            .into(),
                    );
                }
                "--no-qr" => {
                    return Err(
                        "Use futureos-headless --no-qr instead: the graphical Desktop shows the \
                         invitation in its own window and prints nothing."
                            .into(),
                    );
                }
                // macOS Finder can pass its process serial number at launch, and
                // only ever as the entire command line — so it is not accepted
                // alongside a flag, which would let a real mistake hide behind it.
                value if value.starts_with("-psn_") && args.len() == 1 => {}
                other => return Err(format!("Unknown option: {other}")),
            }
            index += 1;
        }
        if help {
            return Ok(Launch::Help);
        }
        // Asking for help revokes nothing, whatever else was typed; a reply path
        // means the caller is the launcher and is waiting for an answer.
        if let Some(reply) = reply {
            return Ok(Launch::ForwardRePair { reply });
        }
        Ok(Launch::Gui(options))
    }
}

/// What a forwarded `argv` asks the running Desktop to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Forwarded {
    /// A `--re-pair`, with the path to answer on when the launcher could name one.
    RePair { reply: Option<PathBuf> },
}

/// Read a re-pair request out of the `argv` the single-instance plugin delivers.
///
/// That `argv` **includes the program name**, which is why this is not simply a
/// membership test over every element: it is parsed as a command line with the
/// program name dropped, so a path that happens to contain the flag's text (or the
/// executable's own name) cannot look like a request. Every platform forwards
/// `std::env::args()` verbatim, so dropping the first element is right on all
/// three.
pub fn forwarded_request(argv: &[String]) -> Option<Forwarded> {
    let flags = argv.get(1..).unwrap_or_default();
    match Launch::parse(flags) {
        Ok(Launch::ForwardRePair { reply }) => Some(Forwarded::RePair { reply: Some(reply) }),
        // A launch that typed `--re-pair` itself still means the same thing here:
        // it only reaches a forwarded `argv` if the ownership check in the
        // launcher failed to notice this Desktop, and the user's intent is the
        // same either way.
        Ok(Launch::Gui(options)) if options.re_pair => Some(Forwarded::RePair { reply: None }),
        _ => None,
    }
}

/// Whether another Desktop already owns this data directory.
///
/// The lock is shared with the headless entry point (`instance.rs`), so this
/// answers "would a launch here become the Desktop, or join one that exists" —
/// which is exactly the question `--re-pair` has to settle before it decides
/// where the invitation will come from.
pub fn another_desktop_is_running() -> bool {
    match crate::instance::InstanceGuard::acquire() {
        Ok(guard) => {
            drop(guard);
            false
        }
        Err(_) => true,
    }
}

/// Resolve the environment the Desktop itself will use.
///
/// Login-shell hydration may supply `FUTURE_HOME`, which moves the data directory
/// the ownership check above reads — so it runs first, in the same order the
/// headless entry point uses. Idempotent: the Desktop calls it again at startup.
pub fn prepare_host_environment() -> Result<(), String> {
    crate::host_environment::initialize().map_err(|error| error.to_string())
}

/// Refuse `--re-pair` without a terminal.
///
/// It revokes a live pairing the moment it runs, which is the one destructive
/// thing a flag on this entry point does, and a script is where such a command
/// ends up unattended. Requiring an interactive terminal is the same rule the
/// headless entry point applies to its own `--re-pair`; here it also keeps the
/// refusal legible, since the terminal is where the new invitation is printed.
pub fn require_terminal() -> Result<(), String> {
    require_terminal_with(std::io::stdin().is_terminal() && std::io::stdout().is_terminal())
}

/// The formatting half, split out so both arms are testable without a TTY.
fn require_terminal_with(interactive: bool) -> Result<(), String> {
    if interactive {
        return Ok(());
    }
    Err(
        "--re-pair revokes the saved pairing, so it needs an interactive terminal. \
         Run it from a shell. To replace a pairing on a machine with no window, use \
         futureos-headless --re-pair."
            .into(),
    )
}

/// What a finished re-pair tells the terminal that asked for it.
///
/// One shape for both deliveries: the invitation when there is one, and otherwise
/// why there is not — so a caller cannot print "done" for a re-pair that issued
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    /// The complete `futureos://remote/pair` invitation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Unix seconds after which `link` no longer works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Why there is no invitation, when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    /// The answer for a re-pair that ended in this status.
    ///
    /// The invitation is extracted by the invitation rule the headless entry point
    /// applies to the code itself (`headless::invitation_link`), not a second
    /// opinion: a link that rule refuses (expired, or not a v2 invitation) is not
    /// one this entry point should hand a user either.
    ///
    /// Deliberately not `headless::pairing_link`: that also requires the Agent
    /// availability probe, which starts with the bridge and cannot have answered
    /// yet when a launch re-pairs. A freshly minted code is valid and visible in
    /// the window regardless, so gating the printed answer on the probe reported
    /// "no pairing invitation" for an invitation the user could already scan.
    pub fn of_status(status: &crate::remote::RemoteStatus) -> Self {
        match crate::headless::invitation_link(status) {
            Some(link) => Self {
                link: Some(link),
                expires_at: status.pairing_code_expires_at,
                error: None,
            },
            None => Self::of_error(format!(
                "the remote start ended in {:?}{}",
                status.phase,
                status
                    .reason
                    .map(|reason| format!(" ({reason:?})"))
                    .unwrap_or_default()
            )),
        }
    }

    /// The answer for a re-pair that failed outright.
    pub fn of_error(message: impl Into<String>) -> Self {
        Self {
            link: None,
            expires_at: None,
            error: Some(message.into()),
        }
    }
}

/// Write the answer for the process waiting on `path`.
pub fn write_reply(path: &Path, reply: &Reply) -> Result<(), String> {
    let value = serde_json::to_value(reply).map_err(|error| error.to_string())?;
    crate::config_io::write_json_atomic(path, &value, true).map_err(|error| error.to_string())
}

/// Read an answer, if one is there yet.
///
/// Empty or unparsable means "not yet", not "failed": the writer is another
/// process, and a poll can land before it has finished. Failure is reported
/// through [`Reply::error`], so this never has to guess.
pub fn read_reply(path: &Path) -> Option<Reply> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The text a launcher prints once the request has been answered.
pub fn invitation_notice(reply: &Reply) -> String {
    match (reply.link.as_deref(), reply.error.as_deref()) {
        (Some(link), _) => {
            let expiry = match reply.expires_at {
                Some(expiry) => format!("It expires at Unix time {expiry}"),
                None => "It expires shortly".to_string(),
            };
            format!(
                "\nPAIRING INVITATION - open the FutureOS app and scan or paste this complete \
                 link\n{link}\nSingle-use. {expiry}, and pairing a new device replaces the one \
                 that was connected, so that device is already being cut off. Do not share this \
                 link.\n"
            )
        }
        (None, Some(error)) => format!(
            "\nNo pairing invitation was issued: {error}.\nThe Remote page in the Desktop window \
             shows the current state.\n"
        ),
        (None, None) => {
            "\nNo pairing invitation was issued, and no reason was reported.\n".to_string()
        }
    }
}

/// The command that hands a `--re-pair` to the Desktop that owns the data
/// directory.
///
/// This process's own executable: the request travels in the arguments, which is
/// what the single-instance channel forwards, and that process dies as soon as the
/// hand-off is done. `stdin` is nulled because nothing on that path reads it — the
/// operator's terminal belongs to the process that stays and prints.
pub fn notifier_command(exe: &Path, reply: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command
        .arg("--re-pair")
        .arg(REPLY_FLAG)
        .arg(reply)
        .stdin(std::process::Stdio::null());
    command
}

/// Hand the request over and wait for the answer.
pub fn forward_re_pair() -> Result<Reply, String> {
    prepare_host_environment()?;
    let exe = std::env::current_exe().map_err(|error| {
        format!("Cannot locate this executable to hand the request over: {error}")
    })?;
    // The answer file is this process's own temporary file: the path travels to
    // the Desktop in the request, so two launchers cannot read each other's
    // invitation, and dropping the handle at the end removes whatever is left.
    let file = tempfile::Builder::new()
        .prefix("futureos-re-pair-")
        .suffix(".json")
        .tempfile()
        .map_err(|error| format!("Cannot create the answer file: {error}"))?;
    let mut notifier = notifier_command(&exe, file.path())
        .spawn()
        .map_err(|error| format!("Cannot start the hand-off process: {error}"))?;
    await_reply(file.path(), &mut notifier)
}

/// Wait for the Desktop that owns the data directory to answer.
pub fn await_reply(reply: &Path, notifier: &mut std::process::Child) -> Result<Reply, String> {
    let deadline = Instant::now() + REPLY_TIMEOUT;
    let mut attempt = || read_reply(reply);
    // A hand-off process that failed never delivered the request, so no answer is
    // coming: report that now instead of waiting out the whole timeout. It exits
    // 0 on the hand-off path itself, so success is not evidence either way — only a
    // *failure* is conclusive.
    let mut gave_up = || matches!(notifier.try_wait(), Ok(Some(status)) if !status.success());
    await_reply_with(deadline, REPLY_POLL, &mut attempt, &mut gave_up)
}

/// The loop, with its clock and both outcomes injected — so every arm is
/// reachable without a running Desktop or a real wait.
fn await_reply_with(
    deadline: Instant,
    interval: Duration,
    attempt: &mut dyn FnMut() -> Option<Reply>,
    gave_up: &mut dyn FnMut() -> bool,
) -> Result<Reply, String> {
    loop {
        if let Some(reply) = attempt() {
            return Ok(reply);
        }
        if gave_up() {
            return Err(
                "Another Desktop owns the data directory but did not accept the request. A \
                 headless Desktop does not listen for one: stop it and run \
                 `futureos-headless --re-pair`, or run this again once the graphical Desktop is \
                 up."
                .into(),
            );
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "The Desktop did not answer within {}s. Check that its window is open and \
                 responsive, then try again.",
                REPLY_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn parse(args: &[&str]) -> Result<Launch, String> {
        Launch::parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn a_bare_launch_shows_the_window() {
        assert_eq!(parse(&[]), Ok(Launch::Gui(LaunchOptions::default())));
        // Finder's process serial number is the whole command line or nothing.
        assert_eq!(
            parse(&["-psn_123"]),
            Ok(Launch::Gui(LaunchOptions::default()))
        );
        assert!(parse(&["-psn_123", "--re-pair"]).is_err());
    }

    #[test]
    fn re_pair_is_accepted_and_help_still_wins() {
        assert_eq!(
            parse(&["--re-pair"]),
            Ok(Launch::Gui(LaunchOptions { re_pair: true }))
        );
        // Asking for help revokes nothing, whatever else was typed.
        assert_eq!(parse(&["--re-pair", "--help"]), Ok(Launch::Help));
    }

    #[test]
    fn help_alone_prints_without_starting() {
        assert_eq!(parse(&["--help"]), Ok(Launch::Help));
        assert_eq!(parse(&["-h"]), Ok(Launch::Help));
        assert!(HELP.contains("Usage: futureos [--re-pair] [--help]"));
        assert!(HELP.contains("futureos-headless"));
    }

    /// The two headless flags must keep pointing at the other binary, and must do
    /// so even next to `--help` (the launch that would otherwise print help and
    /// exit successfully, hiding the mistake).
    #[test]
    fn headless_flags_are_refused_and_named() {
        for args in [
            vec!["--headless"],
            vec!["--no-qr"],
            vec!["--help", "--headless"],
            vec!["--help", "--no-qr"],
        ] {
            let error = parse(&args).expect_err("not this entry point");
            assert!(error.contains("futureos-headless"), "{args:?}: {error}");
        }
        assert!(parse(&["--repair"]).is_err());
    }

    #[test]
    fn a_re_pair_without_a_terminal_is_refused_by_name() {
        let error = require_terminal_with(false).expect_err("no terminal");
        assert!(error.contains("--re-pair"), "{error}");
        assert!(error.contains("interactive terminal"), "{error}");
        assert!(error.contains("futureos-headless --re-pair"), "{error}");
        assert_eq!(require_terminal_with(true), Ok(()));
    }

    /// The reply path takes a value, so the parser cannot treat it as another
    /// flag — and a missing value is a usage error, not a silent empty path that
    /// would leave the launcher waiting for an answer nobody can write.
    #[test]
    fn the_reply_flag_carries_a_path() {
        let parsed = parse(&["--re-pair", REPLY_FLAG, "/tmp/answer.json"]);
        assert_eq!(
            parsed,
            Ok(Launch::ForwardRePair {
                reply: PathBuf::from("/tmp/answer.json")
            })
        );
        let error = parse(&["--re-pair", REPLY_FLAG]).expect_err("a path is required");
        assert!(error.contains(REPLY_FLAG), "{error}");
        assert!(error.contains("needs"), "{error}");
    }

    #[test]
    fn another_desktop_is_running_reports_the_data_directory_owner() {
        let _home = HomeGuard::new("launch-owner");
        assert!(!another_desktop_is_running());
        let held = crate::instance::InstanceGuard::acquire().expect("take the lock");
        assert!(another_desktop_is_running());
        drop(held);
        assert!(!another_desktop_is_running());
    }

    /// The hand-off is only correct if the child's command line reads back as the
    /// internal mode: falling through to `Gui` would make the child start a second
    /// Desktop instead of delivering the request (and, with `re_pair`, re-pair
    /// locally and answer nobody).
    #[test]
    fn the_notifier_command_line_round_trips() {
        let command = notifier_command(Path::new("/opt/futureos"), Path::new("/tmp/a.json"));
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["--re-pair", REPLY_FLAG, "/tmp/a.json"]);
        assert_eq!(
            Launch::parse(&args),
            Ok(Launch::ForwardRePair {
                reply: PathBuf::from("/tmp/a.json")
            })
        );
        // What the plugin forwards is the whole `argv`, program name included.
        let forwarded = std::iter::once("/opt/futureos".to_string())
            .chain(args)
            .collect::<Vec<_>>();
        assert_eq!(
            forwarded_request(&forwarded),
            Some(Forwarded::RePair {
                reply: Some(PathBuf::from("/tmp/a.json"))
            })
        );
    }

    /// A `--re-pair` reaching a forwarded `argv` without a reply path still means
    /// "re-pair": the Desktop does the work and shows the invitation in its own
    /// window, it just has no terminal to answer.
    #[test]
    fn a_forwarded_re_pair_without_a_reply_path_is_still_a_request() {
        assert_eq!(
            forwarded_request(&argv(&["/opt/futureos", "--re-pair"])),
            Some(Forwarded::RePair { reply: None })
        );
        // The program name is not a flag, and nothing else is a request.
        assert_eq!(forwarded_request(&argv(&["/opt/futureos"])), None);
        assert_eq!(forwarded_request(&argv(&["--re-pair"])), None);
        assert_eq!(forwarded_request(&argv(&[])), None);
        // A path that merely contains the flag's text is not the flag.
        assert_eq!(forwarded_request(&argv(&["/tmp/--re-pair"])), None);
        assert_eq!(
            forwarded_request(&argv(&["/tmp/--re-pair", "--help"])),
            None,
            "a forwarded help must not re-pair"
        );
    }

    #[test]
    fn a_reply_survives_the_round_trip_between_processes() {
        let home = HomeGuard::new("launch-reply");
        let path = Path::new(&std::env::var("HOME").unwrap()).join("answer.json");
        // Nothing written yet is "not yet", not an error.
        assert_eq!(read_reply(&path), None);
        std::fs::write(&path, "").unwrap();
        assert_eq!(read_reply(&path), None, "an empty file is not an answer");

        let reply = Reply {
            link: Some("futureos://remote/pair?v=2&code=x".into()),
            expires_at: Some(1_700_000_000),
            error: None,
        };
        write_reply(&path, &reply).expect("write");
        assert_eq!(read_reply(&path), Some(reply));
        drop(home);
    }

    /// A re-pair reports the status the start just returned, before the Agent
    /// availability probe (which starts with the bridge) can have answered. That
    /// must not turn a freshly issued invitation into "nothing was issued": the
    /// code is printable the moment it is minted, and the window shows the same
    /// one.
    #[test]
    fn a_re_pair_reports_the_invitation_before_the_agent_is_probed() {
        let code = format!(
            "futureos://remote/pair?v=2&code=abc&desktopId=desktop_test&desktopKey=UTEST&secureKey={}&secret={}",
            "A".repeat(43),
            "B".repeat(43)
        );
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 300;
        let mut status = crate::remote::RemoteStatus {
            client: None,
            phase: crate::remote::RemotePhase::Ready,
            reason: None,
            recovery: None,
            nats_url: "nats://127.0.0.1:4222".into(),
            pair_id: "pair_new".into(),
            pairing_code: Some(code.clone()),
            pairing_code_expires_at: Some(expires_at),
            desktop_id: "desktop_test".into(),
            desktop_public_key: "UTEST".into(),
            web_url: None,
            web_lan_url: None,
            warning_code: None,
            agent_available: false,
        };
        let reply = Reply::of_status(&status);
        assert_eq!(reply.link.as_deref(), Some(code.as_str()), "{reply:?}");
        assert_eq!(reply.expires_at, Some(expires_at));
        assert_eq!(reply.error, None);
        assert!(invitation_notice(&reply).contains(&code));

        // The invitation rule still refuses a code that cannot be scanned — this
        // is about the readiness probe, not about printing expired invitations.
        status.pairing_code_expires_at = Some(expires_at - 1_000);
        assert!(Reply::of_status(&status).error.is_some());
    }

    #[test]
    fn the_notice_never_reports_success_without_an_invitation() {
        let issued = invitation_notice(&Reply {
            link: Some("futureos://remote/pair?v=2&code=x".into()),
            expires_at: Some(1_700_000_000),
            error: None,
        });
        assert!(
            issued.contains("futureos://remote/pair?v=2&code=x"),
            "{issued}"
        );
        assert!(issued.contains("1700000000"), "{issued}");
        assert!(issued.contains("Do not share"), "{issued}");

        let refused = invitation_notice(&Reply::of_error("the remote start ended in Failed"));
        assert!(
            refused.contains("No pairing invitation was issued"),
            "{refused}"
        );
        assert!(
            refused.contains("the remote start ended in Failed"),
            "{refused}"
        );
        assert!(!refused.contains("futureos://"), "{refused}");

        // Neither half present: still an explicit "nothing was issued".
        let empty = invitation_notice(&Reply::default());
        assert!(empty.contains("No pairing invitation"), "{empty}");
    }

    #[test]
    fn the_launcher_waits_until_the_desktop_answers() {
        let answer = Reply {
            link: Some("futureos://remote/pair?v=2&code=x".into()),
            expires_at: None,
            error: None,
        };
        let mut attempts = vec![None, None, Some(answer.clone())].into_iter();
        let mut attempt = move || attempts.next().flatten();
        let mut gave_up = || false;
        let got = await_reply_with(
            Instant::now() + Duration::from_secs(30),
            Duration::ZERO,
            &mut attempt,
            &mut gave_up,
        )
        .expect("the third poll answers");
        assert_eq!(got, answer);
    }

    #[test]
    fn a_hand_off_that_failed_is_not_waited_out() {
        let mut attempt = || None;
        let mut polls = 0;
        let mut gave_up = || {
            polls += 1;
            polls >= 2
        };
        let error = await_reply_with(
            Instant::now() + Duration::from_secs(30),
            Duration::ZERO,
            &mut attempt,
            &mut gave_up,
        )
        .expect_err("no answer is coming");
        assert!(error.contains("headless"), "{error}");
        assert!(error.contains("futureos-headless --re-pair"), "{error}");
    }

    #[test]
    fn a_silent_desktop_times_out_with_a_named_limit() {
        let mut attempt = || None;
        let mut gave_up = || false;
        let error = await_reply_with(Instant::now(), Duration::ZERO, &mut attempt, &mut gave_up)
            .expect_err("the deadline has passed");
        assert!(
            error.contains(&format!("{}s", REPLY_TIMEOUT.as_secs())),
            "{error}"
        );
        assert!(error.contains("window"), "{error}");
    }
}
