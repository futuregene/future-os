//! The graphical entry point's command line.
//!
//! `futureos` shows the window; the flags it accepts are about what that window
//! should do, not about how remote access is implemented. Two callers read them:
//! the process that is starting, and — when another Desktop already owns the data
//! directory — the running instance, which receives the launching `argv` through
//! the single-instance plugin. Both go through this module so the two can never
//! disagree about what a flag means.
//!
//! This is deliberately not the headless entry point's parser: `--no-qr` exists
//! because a terminal has to print the invitation, and a window does not.

use std::io::IsTerminal;

pub const HELP: &str = "Usage: futureos [--re-pair] [--help]

Start the graphical Desktop.

  --re-pair   Revoke the saved pairing and issue a fresh invitation; the Remote
              page shows the new code and QR. Pairing a new device replaces the
              one that was connected, so it is cut off as soon as this runs —
              whether or not the new code is ever scanned.
  --help      Show this help without starting Desktop.

To disconnect a paired device without pairing another one, use the Remote page's
Unpair button instead. For phone remote access without GUI dependencies, run
futureos-headless.";

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
        for arg in args {
            match arg.as_str() {
                "--help" | "-h" => help = true,
                "--re-pair" => options.re_pair = true,
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
        }
        Ok(if help {
            Launch::Help
        } else {
            Launch::Gui(options)
        })
    }
}

/// Whether a forwarded command line asks for a re-pair.
///
/// This reads the `argv` the single-instance plugin hands to the running
/// instance, which **includes `argv[0]`** — so it looks for the flag anywhere
/// rather than at a fixed index. `argv[0]` is a path and can never be the flag
/// itself, and the launching process has already rejected anything unrecognized,
/// so a plain membership test is the whole check.
pub fn requests_re_pair(argv: &[String]) -> bool {
    argv.iter().any(|arg| arg == "--re-pair")
}

/// Refuse the flag without a terminal.
///
/// `--re-pair` revokes a live pairing the moment it runs, which is the one
/// destructive thing a flag on this entry point does, and a script is where such
/// a command ends up unattended. Requiring an interactive terminal is the same
/// rule the headless entry point applies to its own `--re-pair`; here it also
/// keeps the refusal legible, since the window that would explain the outcome is
/// exactly what a redirected launch does not have.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Launch, String> {
        Launch::parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
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

    /// The plugin forwards `argv[0]` with the flags, so the check cannot assume a
    /// leading flag.
    #[test]
    fn a_forwarded_argv_is_read_with_its_program_name() {
        let argv = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert!(requests_re_pair(&argv(&[
            "/Applications/FutureOS.app/Contents/MacOS/futureos",
            "--re-pair"
        ])));
        assert!(requests_re_pair(&argv(&["futureos", "--re-pair"])));
        assert!(!requests_re_pair(&argv(&["/usr/local/bin/futureos"])));
        assert!(!requests_re_pair(&argv(&["futureos", "--help"])));
        // A path containing the flag's text is not the flag.
        assert!(!requests_re_pair(&argv(&["/tmp/--re-pair"])));
    }

    #[test]
    fn a_re_pair_without_a_terminal_is_refused_by_name() {
        let error = require_terminal_with(false).expect_err("no terminal");
        assert!(error.contains("--re-pair"), "{error}");
        assert!(error.contains("interactive terminal"), "{error}");
        assert!(error.contains("futureos-headless --re-pair"), "{error}");
        assert_eq!(require_terminal_with(true), Ok(()));
    }
}
