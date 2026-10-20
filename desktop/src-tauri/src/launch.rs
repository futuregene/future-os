//! The graphical entry point's command line.
//!
//! `futureos` shows the window; the flags it accepts are about what that window
//! should do, not about how remote access is implemented. Managing the devices
//! that pair into this machine lives in the `future` CLI (`future remote
//! devices`), which asks a *running* Desktop for an invitation — so there is no
//! launch flag for it here.
//!
//! What remains is small on purpose: `--relay`/`--relay-ca` (which relay *this
//! launch* points new invitations at, applied before anything reads the pairing
//! store), the refusal of the other binary's `--headless`, Finder's process
//! serial number, and `--help`.

use std::io::IsTerminal;

pub const HELP: &str = "Usage: futureos [--relay URL [--relay-ca PEM]] [--help]

Start the graphical Desktop.

  --relay URL Print an invitation that points at a self-hosted relay
              (future-relay) instead of the Future platform. It applies to this
              launch only: run again without it to use the Future platform.
  --relay-ca  PEM file with the relay's CA, when its certificate is not issued
              by a public CA.
  --help      Show this help without starting Desktop.

Pairing, unpairing and choosing which device drives this machine all live in the
window's Remote page, and in the terminal via `future remote devices` (list /
add / remove / use). That command asks a running Desktop, so start it first.
For phone remote access without GUI dependencies, run future headless.";

/// The parsed command line. `Help` is not a mode of running: it means the process
/// prints [`HELP`] and exits without touching any state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    Gui,
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
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--help" | "-h" => help = true,
                // Validated here, applied in `apply_relay_from_argv`. Both entry
                // points parse their argv before that runs, and the setting is
                // process-wide.
                "--relay" | "--relay-ca" => {
                    index += 1;
                    let flag = args[index - 1].clone();
                    args.get(index)
                        .ok_or_else(|| format!("{flag} needs a value (see --help)"))?;
                }
                // Named, not generic: these are the headless entry point's flags,
                // and "unknown option" would leave the user guessing which binary
                // they wanted.
                "--headless" => {
                    return Err(
                        "The --headless option has been removed. Run future headless instead."
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
        Ok(if help { Launch::Help } else { Launch::Gui })
    }
}

/// Apply `--relay` / `--relay-ca` from this process's command line.
///
/// Runs once, before anything reads the pairing store: an invitation minted by
/// this launch has to know which relay it points at. `future headless` reads
/// the same flags from its own parser; applying them twice is harmless because
/// the values are identical.
pub(crate) fn apply_relay_from_argv() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut relay = None;
    let mut relay_ca = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--relay" => {
                index += 1;
                relay = args.get(index).cloned();
            }
            "--relay-ca" => {
                index += 1;
                relay_ca = args.get(index).cloned();
            }
            _ => {}
        }
        index += 1;
    }
    apply_relay_settings(relay.as_deref(), relay_ca.as_deref())
}

/// Apply the relay flags to this process, if any were given.
///
/// Both entry points call this before the remote runtime starts, so the choice
/// means the same thing whether it arrived through the GUI's command line or
/// through `future headless`. Nothing is written down: these flags decide
/// where *this launch* mints its invitations, and a launch that names no relay
/// uses the Future platform for new pairings. Existing pairings keep the relay
/// that issued them either way.
pub fn apply_relay_settings(relay: Option<&str>, relay_ca: Option<&str>) {
    future_host::remote::relay::apply_launch_settings(relay, relay_ca)
}

/// Resolve the environment the Desktop itself will use.
///
/// Login-shell hydration may supply `FUTURE_HOME`, which moves the data directory
/// the ownership check in `instance` reads — so it runs first, in the same order
/// the headless entry point uses. Idempotent: the Desktop calls it again at
/// startup.
pub fn prepare_host_environment() -> Result<(), String> {
    crate::host_environment::initialize().map_err(|error| error.to_string())
}

/// Whether this process is attached to a terminal on both ends.
///
/// Asked by the paths that print an invitation or a login link: a script is where
/// such output ends up unattended, and half of it (the QR, the link) is worthless
/// in a log. Kept here because it is the one terminal question both binaries ask.
pub fn interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::test_support::HomeGuard;

    fn parse(args: &[&str]) -> Result<Launch, String> {
        Launch::parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_bare_launch_shows_the_window() {
        assert_eq!(parse(&[]), Ok(Launch::Gui));
        // Finder's process serial number is the whole command line or nothing.
        assert_eq!(parse(&["-psn_123"]), Ok(Launch::Gui));
        assert!(parse(&["-psn_123", "--relay", "https://relay.example"]).is_err());
    }

    #[test]
    fn help_alone_prints_without_starting() {
        assert_eq!(parse(&["--help"]), Ok(Launch::Help));
        assert_eq!(parse(&["-h"]), Ok(Launch::Help));
        assert_eq!(
            parse(&["--help", "--relay", "https://relay.example"]),
            Ok(Launch::Help)
        );
        assert!(HELP.contains("Usage: futureos"));
        assert!(HELP.contains("future remote devices"));
        assert!(HELP.contains("future headless"));
    }

    /// The two flags this entry point no longer has are refused by name, with the
    /// replacement — an unknown-option error would leave the user guessing.
    #[test]
    fn the_removed_pairing_flags_say_what_replaced_them() {
        for args in [
            vec!["--re-pair"],
            vec!["--re-pair", "--no-qr"],
            vec!["--no-qr"],
        ] {
            let error = parse(&args).expect_err("no longer a launch flag");
            assert!(error.contains("Unknown option"), "{args:?}: {error}");
            assert!(!error.trim().is_empty());
        }
    }

    /// `--headless` belongs to the other binary and must keep saying so, even next
    /// to `--help` (the launch that would otherwise print help and exit
    /// successfully, hiding the mistake).
    #[test]
    fn the_headless_flag_is_refused_and_named() {
        for args in [vec!["--headless"], vec!["--help", "--headless"]] {
            let error = parse(&args).expect_err("not this entry point");
            assert!(error.contains("future headless"), "{args:?}: {error}");
        }
        assert!(parse(&["--repair"]).is_err());
    }

    /// The relay flags take a value, so a missing one is a usage error rather than
    /// a silently empty endpoint.
    #[test]
    fn relay_flags_take_a_value() {
        assert_eq!(
            parse(&["--relay", "https://relay.example"]),
            Ok(Launch::Gui)
        );
        assert_eq!(parse(&["--relay-ca", "/etc/ca.pem"]), Ok(Launch::Gui));
        let error = parse(&["--relay"]).expect_err("a value is required");
        assert!(error.contains("--relay"), "{error}");
        assert!(error.contains("needs"), "{error}");
        assert!(parse(&["--relay-ca"]).is_err());
    }

    /// Both launch stages hydrate the host environment, and the Desktop hydrates
    /// again at startup (`lib.rs` calls `initialize` itself), so the wrapper must
    /// be idempotent: the second call answers exactly what the first one did
    /// instead of re-running the login-shell harvest or failing on a second pass.
    #[test]
    fn prepare_host_environment_is_idempotent() {
        // The harvest mutates this process's PATH and adds missing vars; keep it
        // away from the developer's real HOME.
        let _home = HomeGuard::new("launch-host-environment");
        let first = prepare_host_environment();
        let second = prepare_host_environment();
        assert_eq!(first, second);
    }
}
