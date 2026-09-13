#[cfg(not(test))]
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match futureos_lib::headless::Options::parse(&args) {
        Ok(futureos_lib::headless::Launch::Help) => {
            println!("{}", futureos_lib::headless::HELP);
            std::process::ExitCode::SUCCESS
        }
        Ok(futureos_lib::headless::Launch::Headless(options)) => {
            match futureos_lib::headless::run(options) {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("FutureOS: {error}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Ok(futureos_lib::headless::Launch::Gui) => {
            #[cfg(feature = "gui")]
            {
                configure_environment();
                futureos_lib::run();
                std::process::ExitCode::SUCCESS
            }
            #[cfg(not(feature = "gui"))]
            {
                eprintln!("This build has no graphical interface. Run with --headless.");
                std::process::ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("{error}\n\n{}", futureos_lib::headless::HELP);
            std::process::ExitCode::FAILURE
        }
    }
}

/// The executable uses the console subsystem so shells wait for --headless
/// and Ctrl+C reaches it. Ordinary GUI launches detach their console instead.
#[cfg(any(feature = "gui", test))]
fn configure_environment() {
    #[cfg(target_os = "windows")]
    unsafe {
        let _ = windows::Win32::System::Console::FreeConsole();
    }
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
