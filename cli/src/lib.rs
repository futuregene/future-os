//! future-cli — Rust port of the TypeScript `future` CLI.
//!
//! Goal: byte-identical argument parsing, help text, output, and exit codes.
//! `dispatch` is the port of `cli/src/index.ts` `main()`; command modules port
//! `cli/src/commands/*`.

pub mod browser;
pub mod commands;
pub mod constants;
pub mod help;
pub mod output;
pub mod rpc;
#[cfg(test)]
pub mod test_env;
#[cfg(test)]
pub mod test_server;
pub mod types;
pub mod utils;
pub mod version;

pub use output::Output;

use std::future::Future;

/// Sentinel returned by commands that have already written their error output
/// and only need to force exit code 1 — the port of `process.exit(1)` inside
/// command bodies (agent/models/session). `catch` recognises it and skips the
/// generic `console.error` step so nothing is double-printed.
pub const HANDLED_EXIT: &str = "\u{0}handled-exit";

/// Port of `cli/src/index.ts` `main()`.
///
/// `args` is the full argv (without the program name), i.e. what Node's
/// `process.argv.slice(2)` yields. Returns the process exit code.
pub async fn dispatch(args: &[String], out: &Output) -> i32 {
    // const [group, command, ...rest] = args;
    let group = args.first().map(String::as_str);
    let command = args.get(1).map(String::as_str);
    let rest: &[String] = args.get(2..).unwrap_or(&[]);

    // if (group === "--version" || group === "-v" || group === "version")
    if matches!(group, Some("--version" | "-v" | "version")) {
        out.log(&format!("future v{}", version::VERSION));
        return 0;
    }

    // if (group === "init")
    if group == Some("init") {
        if command == Some("--help") || command == Some("-h") {
            out.log(help::INIT_HELP);
            return 0;
        }
        if let Some(cmd) = command {
            out.log_err(&format!("Unknown argument: {cmd}\n"));
            out.log_err("Usage: future init");
            return 1;
        }
        return catch(out, commands::init::init_command(out)).await;
    }

    // if (group === "auth" && (!command || command === "--help" || command === "-h"))
    if group == Some("auth")
        && (command.is_none() || command == Some("--help") || command == Some("-h"))
    {
        out.log(help::AUTH_GROUP_HELP);
        return 0;
    }

    // if (group === "auth" && command === "login")
    if group == Some("auth") && command == Some("login") {
        if rest.iter().any(|a| a == "--help" || a == "-h") {
            out.log(help::AUTH_LOGIN_HELP);
            return 0;
        }
        // const urlIdx = rest.indexOf("--url");
        // if (urlIdx !== -1 && urlIdx + 1 < rest.length) { urlOverride = rest[urlIdx + 1]; }
        // else { const urlEq = rest.find(a => a.startsWith("--url=")); urlOverride = urlEq?.slice("--url=".length); }
        let url_override: Option<String> = match rest.iter().position(|a| a == "--url") {
            Some(i) if i + 1 < rest.len() => Some(rest[i + 1].clone()),
            _ => rest
                .iter()
                .find(|a| a.starts_with("--url="))
                .map(|a| a["--url=".len()..].to_string()),
        };
        return catch(out, commands::auth::login(url_override, out)).await;
    }

    // if (group === "auth" && command === "status")
    if group == Some("auth") && command == Some("status") {
        if rest.iter().any(|a| a == "--help" || a == "-h") {
            out.log(help::AUTH_STATUS_HELP);
            return 0;
        }
        return catch(out, commands::auth::status(out)).await;
    }

    // if (group === "auth" && command === "credential")
    if group == Some("auth") && command == Some("credential") {
        if rest.iter().any(|a| a == "--help" || a == "-h") {
            out.log(help::AUTH_CREDENTIAL_HELP);
            return 0;
        }
        let json_flag = rest.iter().any(|a| a == "--json");
        return catch(out, commands::auth::credential(json_flag, out)).await;
    }

    // if (group === "auth" && command === "logout")
    if group == Some("auth") && command == Some("logout") {
        if rest.iter().any(|a| a == "--help" || a == "-h") {
            out.log(help::AUTH_LOGOUT_HELP);
            return 0;
        }
        return catch(out, commands::auth::logout(out)).await;
    }

    // if (group === "auth") — unknown subcommand: show group help
    if group == Some("auth") {
        out.log_err(&format!(
            "Unknown command: {}\n",
            command.unwrap_or("undefined")
        ));
        out.log(help::AUTH_GROUP_HELP_UNKNOWN);
        return 0;
    }

    // if (group === "tools" && (!command || command === "--help" || command === "-h"))
    if group == Some("tools")
        && (command.is_none() || command == Some("--help") || command == Some("-h"))
    {
        out.log(help::TOOLS_GROUP_HELP);
        return 0;
    }

    // if (group === "tools" && isToolsCommand(command))
    if group == Some("tools") && commands::tools::is_tools_command(command) {
        let cmd = command.expect("is_tools_command implies a command");
        return catch(out, commands::tools::tools(cmd, rest, out)).await;
    }

    // if (group === "tools") — unknown subcommand
    if group == Some("tools") {
        out.log_err(&format!(
            "Unknown command: {}\n",
            command.unwrap_or("undefined")
        ));
        out.log(help::TOOLS_GROUP_HELP);
        return 0;
    }

    // if (group === "skills" && (!command || command === "--help" || command === "-h"))
    if group == Some("skills")
        && (command.is_none() || command == Some("--help") || command == Some("-h"))
    {
        out.log(help::SKILLS_GROUP_HELP);
        return 0;
    }

    // if (group === "skills" && isSkillsCommand(command))
    if group == Some("skills") && commands::skills::is_skills_command(command) {
        let cmd = command.expect("is_skills_command implies a command");
        return catch(out, commands::skills::skills(cmd, rest, out)).await;
    }

    // if (group === "skills") — unknown subcommand
    if group == Some("skills") {
        out.log_err(&format!(
            "Unknown command: {}\n",
            command.unwrap_or("undefined")
        ));
        out.log(help::SKILLS_GROUP_HELP);
        return 0;
    }

    // if (group === "account" && (!command || command === "--help" || command === "-h"))
    if group == Some("account")
        && (command.is_none() || command == Some("--help") || command == Some("-h"))
    {
        out.log(help::ACCOUNT_GROUP_HELP);
        return 0;
    }

    // if (group === "account" && isAccountCommand(command))
    if group == Some("account") && commands::account::is_account_command(command) {
        let cmd = command.expect("is_account_command implies a command");
        return catch(out, commands::account::account(cmd, rest, out)).await;
    }

    // if (group === "account") — unknown subcommand
    if group == Some("account") {
        out.log_err(&format!(
            "Unknown command: {}\n",
            command.unwrap_or("undefined")
        ));
        out.log(help::ACCOUNT_GROUP_HELP);
        return 0;
    }

    // if (group === "run") — args.slice(1) is everything after "run"
    if group == Some("run") {
        return catch(
            out,
            commands::run::run_command(args.get(1..).unwrap_or(&[]), out),
        )
        .await;
    }

    // if (group === "models")
    if group == Some("models") {
        if command == Some("--help")
            || command == Some("-h")
            || rest.iter().any(|a| a == "--help" || a == "-h")
        {
            out.log(help::MODELS_HELP);
            return 0;
        }
        // command === "--json" ? [command, ...rest] : rest
        let models_args: Vec<String> = if command == Some("--json") {
            std::iter::once("--json".to_string())
                .chain(rest.iter().cloned())
                .collect()
        } else {
            rest.to_vec()
        };
        return catch(out, commands::models::models(&models_args, out)).await;
    }

    // if (group === "session")
    if group == Some("session") {
        return catch(out, commands::session::session(command, rest, out)).await;
    }

    // if (group === "doctor")
    if group == Some("doctor") {
        return catch(out, commands::doctor::doctor(out)).await;
    }

    // printHelp();
    out.log(help::MAIN_HELP);
    0
}

/// Port of `main().catch(...)`: a rejected command promise becomes
/// `console.error(error.message)` on stderr with exit code 1. The final exit
/// code is the max of the command result and any `process.exitCode` set
/// during the run (`installBuiltinSkills` sets it on catalog failure and
/// continues), exactly like Node.
async fn catch<F>(out: &Output, fut: F) -> i32
where
    F: Future<Output = Result<(), String>>,
{
    let code = match fut.await {
        Ok(()) => 0,
        Err(msg) if msg == HANDLED_EXIT => 1,
        Err(msg) => {
            out.log_err(&msg);
            1
        }
    };
    code.max(out.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the dispatch with captured output; returns (exit_code, stdout, stderr).
    async fn run(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (out, cap) = Output::memory();
        let code = dispatch(&args, &out).await;
        let stdout = String::from_utf8(cap.out.lock().expect("poisoned").clone()).unwrap();
        let stderr = String::from_utf8(cap.err.lock().expect("poisoned").clone()).unwrap();
        (code, stdout, stderr)
    }

    #[tokio::test]
    async fn version_flags() {
        for flag in ["--version", "-v", "version"] {
            let (code, stdout, stderr) = run(&[flag]).await;
            assert_eq!(code, 0);
            assert_eq!(stdout, format!("future v{}\n", version::VERSION));
            assert_eq!(stderr, "");
        }
    }

    #[tokio::test]
    async fn no_args_prints_main_help() {
        let (code, stdout, stderr) = run(&[]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, format!("{}\n", help::MAIN_HELP));
        assert_eq!(stderr, "");
    }

    #[tokio::test]
    async fn unknown_group_prints_main_help() {
        let (code, stdout, _) = run(&["bogus"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, format!("{}\n", help::MAIN_HELP));
    }

    #[tokio::test]
    async fn init_help_and_unknown_arg() {
        let (code, stdout, _) = run(&["init", "--help"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, format!("{}\n", help::INIT_HELP));

        let (code, stdout, stderr) = run(&["init", "foo"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert_eq!(stderr, "Unknown argument: foo\n\nUsage: future init\n");
    }

    #[tokio::test]
    async fn auth_group_help_variants() {
        // Plain group help.
        let (code, stdout, stderr) = run(&["auth"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, format!("{}\n", help::AUTH_GROUP_HELP));
        assert_eq!(stderr, "");

        // Unknown subcommand: error on stderr + the OTHER group-help variant.
        let (code, stdout, stderr) = run(&["auth", "bogus"]).await;
        assert_eq!(code, 0);
        assert_eq!(stderr, "Unknown command: bogus\n\n");
        assert_eq!(stdout, format!("{}\n", help::AUTH_GROUP_HELP_UNKNOWN));
        assert_ne!(help::AUTH_GROUP_HELP, help::AUTH_GROUP_HELP_UNKNOWN);
    }

    #[tokio::test]
    async fn auth_login_url_parsing_reaches_login() {
        // All three forms route into login; with an unreachable --url the
        // device-code POST fails fast with a Network error and exit code 1.
        // Isolated HOME: login reads ~/.future/agent/auth.json, and the
        // shared env lock prevents other env-mutating tests from racing us.
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        for args in [
            &["auth", "login", "--url", "http://127.0.0.1:1"][..],
            &["auth", "login", "--url=http://127.0.0.1:1"][..],
            &["auth", "login", "--url", "http://127.0.0.1:1", "extra"][..],
        ] {
            let (code, _, stderr) = run(args).await;
            assert_eq!(code, 1);
            assert!(
                stderr.contains("Network error"),
                "args={args:?} stderr={stderr:?}"
            );
        }
    }

    #[tokio::test]
    async fn all_help_outputs_match_help_constants() {
        // Golden: every --help/-h path must print exactly the ported help
        // text (verified byte-identical against the TS CLI in the diff
        // battery) on stdout with exit 0 and empty stderr.
        let cases: &[(&[&str], &str)] = &[
            (&["init", "--help"], help::INIT_HELP),
            (&["init", "-h"], help::INIT_HELP),
            (&["auth", "--help"], help::AUTH_GROUP_HELP),
            (&["auth", "-h"], help::AUTH_GROUP_HELP),
            (&["auth", "login", "--help"], help::AUTH_LOGIN_HELP),
            (&["auth", "login", "-h"], help::AUTH_LOGIN_HELP),
            (&["auth", "status", "--help"], help::AUTH_STATUS_HELP),
            (
                &["auth", "credential", "--help"],
                help::AUTH_CREDENTIAL_HELP,
            ),
            (&["auth", "logout", "--help"], help::AUTH_LOGOUT_HELP),
            (&["account", "--help"], help::ACCOUNT_GROUP_HELP),
            (&["skills", "--help"], help::SKILLS_GROUP_HELP),
            (&["tools", "--help"], help::TOOLS_GROUP_HELP),
            (&["models", "--help"], help::MODELS_HELP),
            (&["models", "-h"], help::MODELS_HELP),
            (&["session", "--help"], commands::session::SESSION_HELP),
            (&["session", "-h"], commands::session::SESSION_HELP),
        ];
        for (args, expected) in cases {
            let (code, stdout, stderr) = run(args).await;
            assert_eq!(code, 0, "args {args:?}");
            assert_eq!(stdout, format!("{expected}\n"), "args {args:?}");
            assert_eq!(stderr, "", "args {args:?}");
        }
    }

    #[tokio::test]
    async fn tools_skills_account_predicates() {
        assert!(commands::tools::is_tools_command(Some("list")));
        assert!(!commands::tools::is_tools_command(Some("bogus")));
        assert!(!commands::tools::is_tools_command(None));
        assert!(commands::skills::is_skills_command(Some("install-builtin")));
        assert!(!commands::skills::is_skills_command(Some("bogus")));
        assert!(commands::account::is_account_command(Some("balance")));
        assert!(!commands::account::is_account_command(Some("bogus")));
    }
}
