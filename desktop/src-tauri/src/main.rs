#[cfg(not(test))]
use futureos_lib::launch::{self, Launch};

#[cfg(not(test))]
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(result) = futureos_lib::dispatch_agent_helper(&args) {
        return match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    match Launch::parse(&args) {
        Ok(Launch::Help) => {
            println!("{}", launch::HELP);
            std::process::ExitCode::SUCCESS
        }
        Ok(Launch::Gui(options)) => {
            if options.re_pair {
                // Checked first, before any state is touched: the flag revokes a
                // live pairing, so a launch that cannot ask a human first must not
                // begin.
                if let Err(error) = launch::require_terminal() {
                    eprintln!("{error}\n\n{}", launch::HELP);
                    return std::process::ExitCode::FAILURE;
                }
                // Hydration can move the data directory (a login shell may supply
                // FUTURE_HOME), so it happens before asking who owns it. A failure
                // here is not fatal — the launch below reports it properly.
                let _ = launch::prepare_host_environment();
                if launch::another_desktop_is_running() {
                    // Another Desktop holds the pairing, so this process cannot
                    // perform the re-pair — but it can be the one that stays and
                    // prints, which is the whole point of asking from a shell.
                    return match launch::forward_re_pair() {
                        Ok(reply) => {
                            use std::io::Write;
                            let _ = std::io::stdout()
                                .write_all(launch::invitation_notice(&reply).as_bytes());
                            let _ = std::io::stdout().flush();
                            std::process::ExitCode::SUCCESS
                        }
                        Err(message) => {
                            eprintln!("{message}");
                            std::process::ExitCode::FAILURE
                        }
                    };
                }
            }
            start_desktop(options)
        }
        // Only ever spawned by the branch above: this process carries the request
        // to the running Desktop and exits at the hand-off, so it must not touch
        // the pairing itself — the reply travels back to the process that asked.
        Ok(Launch::ForwardRePair { .. }) => start_desktop(launch::LaunchOptions::default()),
        Err(error) => {
            eprintln!("{error}\n\n{}", launch::HELP);
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(test))]
fn start_desktop(options: launch::LaunchOptions) -> std::process::ExitCode {
    #[cfg(feature = "gui")]
    {
        configure_environment();
        match futureos_lib::run_with(options) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("FutureOS: {error}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(feature = "gui"))]
    {
        let _ = options;
        eprintln!("This build has no graphical interface. Run futureos-headless instead.");
        std::process::ExitCode::FAILURE
    }
}

/// Suppress macOS activity logs for graphical launches. Windows console
/// detachment happens only after the GUI has acquired its instance lock.
#[cfg(any(feature = "gui", test))]
fn configure_environment() {
    #[cfg(target_os = "macos")]
    std::env::set_var("OS_ACTIVITY_MODE", "disable");
}

#[cfg(test)]
mod tests {
    #[test]
    fn configure_environment_is_available() {
        // Do not detach the test runner's console.
        let _: fn() = super::configure_environment;
    }
}
