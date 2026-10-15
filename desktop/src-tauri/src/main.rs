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
            // Checked here, before any state is touched: the flag revokes a live
            // pairing, so a launch that cannot ask a human first must not begin.
            if options.re_pair {
                if let Err(error) = launch::require_terminal() {
                    eprintln!("{error}\n\n{}", launch::HELP);
                    return std::process::ExitCode::FAILURE;
                }
            }
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
        Err(error) => {
            eprintln!("{error}\n\n{}", launch::HELP);
            std::process::ExitCode::FAILURE
        }
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
