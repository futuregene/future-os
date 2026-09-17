//! Standalone server entrypoint; never ship this binary linked to GUI libraries.
#[cfg(feature = "gui")]
compile_error!("futureos-headless requires --no-default-features --features headless");

use futureos_lib::headless::{Launch, Options, HELP};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match Options::parse(&args) {
        Ok(Launch::Help) => {
            println!("{HELP}");
            return std::process::ExitCode::SUCCESS;
        }
        Ok(Launch::Headless(options)) => futureos_lib::headless::run(options),
        Err(error) => Err(format!("{error}\n\n{HELP}")),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("FutureOS: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
