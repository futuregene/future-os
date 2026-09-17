const HELP: &str = "Usage: futureos [--help]

Start the graphical Desktop.
For phone remote access without GUI dependencies, run futureos-headless instead.";

#[cfg(not(test))]
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(true) => {
            println!("{HELP}");
            std::process::ExitCode::SUCCESS
        }
        Ok(false) => {
            #[cfg(feature = "gui")]
            {
                configure_environment();
                match futureos_lib::run() {
                    Ok(()) => std::process::ExitCode::SUCCESS,
                    Err(error) => {
                        eprintln!("FutureOS: {error}");
                        std::process::ExitCode::FAILURE
                    }
                }
            }
            #[cfg(not(feature = "gui"))]
            {
                eprintln!("This build has no graphical interface. Run futureos-headless instead.");
                std::process::ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("{error}\n\n{HELP}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Validate before starting any GUI, store or Agent; true requests help only.
fn parse_args(args: &[String]) -> Result<bool, String> {
    let mut help = false;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => help = true,
            "--headless" => {
                return Err(
                    "The --headless option has been removed. Run futureos-headless instead.".into(),
                );
            }
            "--no-qr" | "--re-pair" => {
                return Err(format!("Use futureos-headless {arg} instead."));
            }
            // macOS Finder can pass its process serial number at launch.
            value if value.starts_with("-psn_") && args.len() == 1 => {}
            _ => return Err(format!("Unknown option: {arg}")),
        }
    }
    Ok(help)
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
    fn parse(args: &[&str]) -> Result<bool, String> {
        super::parse_args(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn graphical_launch_and_help() {
        assert_eq!(parse(&[]), Ok(false));
        assert_eq!(parse(&["-psn_123"]), Ok(false));
        assert_eq!(parse(&["--help"]), Ok(true));
        assert_eq!(parse(&["-h"]), Ok(true));
        assert!(super::HELP.contains("Usage: futureos [--help]"));
    }

    #[test]
    fn headless_options_are_rejected_even_with_help() {
        for flag in ["--headless", "--no-qr", "--re-pair"] {
            assert!(parse(&[flag]).unwrap_err().contains("futureos-headless"));
            assert!(parse(&["--help", flag]).is_err());
        }
        assert!(parse(&["--unknown"]).is_err());
        assert!(parse(&["-psn_123", "--headless"]).is_err());
    }

    #[test]
    fn configure_environment_is_available() {
        // Do not detach the test runner's console.
        let _: fn() = super::configure_environment;
    }
}
