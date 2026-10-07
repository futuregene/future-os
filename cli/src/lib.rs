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
pub mod test_cdp;
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
    //
    // `version` is both a bare form and a group with `--json`, so it must be
    // matched before the group dispatch below.
    if matches!(group, Some("--version" | "-v")) {
        out.log(&format!("future v{}", version::VERSION));
        return 0;
    }
    if group == Some("version") {
        let help_flag = command == Some("--help")
            || command == Some("-h")
            || rest.iter().any(|a| a == "--help" || a == "-h");
        if help_flag {
            out.log(help::VERSION_HELP);
            return 0;
        }
        let json_flag = command == Some("--json") || rest.iter().any(|a| a == "--json");
        let unknown = [command, rest.first().map(String::as_str)]
            .into_iter()
            .flatten()
            .find(|arg| !matches!(*arg, "--json" | "--help" | "-h"));
        if let Some(argument) = unknown {
            out.log_err(&format!("Unknown argument: {argument}\n"));
            out.log_err("Usage: future version [--json]");
            return 1;
        }
        if json_flag {
            out.log(version::build_info_json().trim_end());
        } else {
            out.log(&format!("future v{}", version::VERSION));
        }
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

    // `future config` — interactive model-provider setup, plus non-interactive
    // reads/writes of the global settings document.
    if group == Some("config") {
        match command {
            None => return catch(out, commands::configure::configure(out)).await,
            Some("--help" | "-h") => {
                out.log(help::CONFIG_HELP);
                return 0;
            }
            Some(sub) => {
                let args = rest.to_vec();
                let help_flag = args.iter().any(|a| a == "--help" || a == "-h");
                match sub {
                    "get" => {
                        if help_flag {
                            out.log(help::CONFIG_GET_HELP);
                            return 0;
                        }
                        return catch(out, async { commands::settings::get(&args, out) }).await;
                    }
                    "set" => {
                        if help_flag {
                            out.log(help::CONFIG_SET_HELP);
                            return 0;
                        }
                        return catch(out, async { commands::settings::set(&args, out) }).await;
                    }
                    argument => {
                        out.log_err(&format!("Unknown argument: {argument}\n"));
                        out.log_err("Usage: future config [get [<key>] | set <key> <value>]");
                        return 1;
                    }
                }
            }
        }
    }

    // if (group === "desktop") — the desktop app's own settings document.
    if group == Some("desktop") {
        return catch(out, async {
            commands::desktop::desktop(command, rest, out)
        })
        .await;
    }

    // if (group === "task") — reusable prompt + trigger + full-permission runs.
    if group == Some("task") {
        return catch(out, async { commands::task::task(command, rest, out) }).await;
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
        for flag in ["--version", "-v"] {
            let (code, stdout, stderr) = run(&[flag]).await;
            assert_eq!(code, 0);
            assert_eq!(stdout, format!("future v{}\n", version::VERSION));
            assert_eq!(stderr, "");
        }
        // The bare `version` form prints the same string as `--version`, so
        // existing callers keep working.
        let (code, stdout, stderr) = run(&["version"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, format!("future v{}\n", version::VERSION));
        assert_eq!(stderr, "");
    }

    /// `version --json` reports the build identity a support conversation (or an
    /// agent) needs, and the two forms agree on the version string.
    #[tokio::test]
    async fn version_json_reports_the_build_identity() {
        let (code, stdout, stderr) = run(&["version", "--json"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stderr, "");
        let info: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(info["version"], version::VERSION);
        assert_eq!(info["isRelease"], version::is_release(version::VERSION));
        assert!(info["buildTarget"].is_string(), "{stdout}");
        // Locally this is a real commit; a tarball build reports null instead.
        if let Some(commit) = info["gitCommit"].as_str() {
            assert_eq!(commit.len(), 40, "{stdout}");
        }
    }

    #[tokio::test]
    async fn version_help_and_unknown_argument() {
        for values in [
            vec!["version", "--help"],
            vec!["version", "-h"],
            vec!["version", "--json", "--help"],
        ] {
            let (code, stdout, stderr) = run(&values).await;
            assert_eq!(code, 0, "{values:?}");
            assert_eq!(stdout, format!("{}\n", help::VERSION_HELP), "{values:?}");
            assert_eq!(stderr, "", "{values:?}");
        }
        let (code, stdout, stderr) = run(&["version", "bogus"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            "Unknown argument: bogus\n\nUsage: future version [--json]\n"
        );
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
    async fn config_help_and_unknown_arg() {
        let (code, stdout, stderr) = run(&["config", "--help"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, format!("{}\n", help::CONFIG_HELP));
        assert_eq!(stderr, "");

        let (code, stdout, stderr) = run(&["config", "unexpected"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            "Unknown argument: unexpected\n\nUsage: future config [get [<key>] | set <key> <value>]\n"
        );
    }

    /// `config get` / `config set` are reachable from the dispatcher, print
    /// their own help without touching the file, and read the Agent's settings
    /// document from the FUTURE_HOME in effect.
    #[tokio::test]
    async fn config_get_and_set_dispatch() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env =
            crate::test_env::EnvGuard::set(&[("FUTURE_HOME", dir.path().as_os_str().to_owned())]);

        for (args, expected) in [
            (vec!["config", "get", "--help"], help::CONFIG_GET_HELP),
            (vec!["config", "set", "--help"], help::CONFIG_SET_HELP),
        ] {
            let (code, stdout, stderr) = run(&args).await;
            assert_eq!(code, 0, "{args:?}");
            assert_eq!(stdout, format!("{expected}\n"), "{args:?}");
            assert_eq!(stderr, "", "{args:?}");
        }

        let (code, stdout, stderr) = run(&["config", "get"]).await;
        assert_eq!(code, 0);
        assert!(stdout.contains("defaultPermissionLevel = all"), "{stdout}");
        assert_eq!(stderr, "");

        let (code, stdout, stderr) = run(&["config", "set", "maxTurns", "9"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("maxTurns = 9"), "{stdout}");

        let (code, stdout, stderr) = run(&["config", "get", "maxTurns"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, "9\n");
        assert_eq!(stderr, "");

        // A rejected write is reported as a command failure, not a traceback.
        let (code, stdout, stderr) = run(&["config", "set", "maxTurns", "-1"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(
            stderr.starts_with("maxTurns must be at least 0"),
            "{stderr}"
        );
    }

    #[tokio::test]
    async fn desktop_settings_dispatch_routes_reads_writes_and_help() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = crate::test_env::EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);

        // A bare `desktop` and the settings help both print the group help.
        for args in [
            &["desktop"][..],
            &["desktop", "--help"][..],
            &["desktop", "settings", "--help"][..],
        ] {
            let (code, stdout, stderr) = run(args).await;
            assert_eq!(code, 0, "{args:?}");
            assert_eq!(stdout, format!("{}\n", help::DESKTOP_HELP), "{args:?}");
            assert_eq!(stderr, "", "{args:?}");
        }

        // Reads report defaults before the desktop app has ever written one.
        let (code, stdout, stderr) = run(&["desktop", "settings"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("approvalTier = off"), "{stdout}");
        assert!(stdout.contains("bellOnComplete = true"), "{stdout}");

        // A write is visible to the next read, bare for scripting.
        let (code, _, stderr) =
            run(&["desktop", "settings", "set", "bellOnComplete", "false"]).await;
        assert_eq!(code, 0, "{stderr}");
        let (code, stdout, stderr) = run(&["desktop", "settings", "get", "bellOnComplete"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stdout, "false\n");

        // A rejected value is a command failure, not a traceback.
        let (code, stdout, stderr) =
            run(&["desktop", "settings", "set", "titleLanguage", "fr"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(
            stderr.contains("titleLanguage must be en or zh"),
            "{stderr}"
        );

        // An unknown subcommand reports the usage line.
        let (code, stdout, stderr) = run(&["desktop", "bogus"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(stderr.contains("Unknown argument: bogus"), "{stderr}");
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
            (&["config", "--help"], help::CONFIG_HELP),
            (&["config", "-h"], help::CONFIG_HELP),
            (&["desktop", "--help"], help::DESKTOP_HELP),
            (&["desktop", "-h"], help::DESKTOP_HELP),
            (&["desktop", "settings", "--help"], help::DESKTOP_HELP),
            (
                &["desktop", "settings", "get", "--help"],
                help::DESKTOP_GET_HELP,
            ),
            (
                &["desktop", "settings", "set", "--help"],
                help::DESKTOP_SET_HELP,
            ),
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
            (
                &["session", "set", "--help"],
                commands::session::SESSION_SET_HELP,
            ),
            (
                &["session", "set", "sess-1", "-h"],
                commands::session::SESSION_SET_HELP,
            ),
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

    // ── Dispatch routing into each command group ────────────────────

    // On Windows the host platform is `win32`, so `init` links nothing and
    // succeeds; the unix host reports the test binary as unusable.
    #[cfg(unix)]
    #[tokio::test]
    async fn init_dispatch_reaches_command() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // init runs against the test binary (not named "future") → exit 1.
        let (code, _, stderr) = run(&["init"]).await;
        assert_eq!(code, 1);
        assert!(
            stderr.contains("Cannot initialize command links"),
            "stderr: {stderr}"
        );
    }

    #[tokio::test]
    async fn tools_dispatch_and_unknown_subcommand() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_API_KEY", "FUTURE_API_TEST_KEY"]);
        // Known subcommand routes into tools() (local tools still render).
        let (code, stdout, _) = run(&["tools", "list"]).await;
        assert_eq!(code, 0);
        assert!(stdout.contains("tools available."), "stdout: {stdout}");
        // Unknown subcommand → stderr note + group help, exit 0.
        let (code, stdout, stderr) = run(&["tools", "bogus"]).await;
        assert_eq!(code, 0);
        assert_eq!(stderr, "Unknown command: bogus\n\n");
        assert_eq!(stdout, format!("{}\n", help::TOOLS_GROUP_HELP));
    }

    #[tokio::test]
    async fn skills_dispatch_and_unknown_subcommand() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _grpc = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        // skills uninstall without a name: sets exit code 1 via Output while
        // returning Ok — catch() merges it (process.exitCode semantics).
        let (code, _, stderr) = run(&["skills", "uninstall"]).await;
        assert_eq!(code, 1);
        assert!(
            stderr.contains("Usage: future skills uninstall"),
            "stderr: {stderr}"
        );
        // Unknown subcommand → group help.
        let (code, stdout, stderr) = run(&["skills", "bogus"]).await;
        assert_eq!(code, 0);
        assert_eq!(stderr, "Unknown command: bogus\n\n");
        assert_eq!(stdout, format!("{}\n", help::SKILLS_GROUP_HELP));
    }

    #[tokio::test]
    async fn account_dispatch_and_unknown_subcommand() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        // Known subcommand routes in; without auth it fails with exit 1.
        let (code, _, stderr) = run(&["account", "profile"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("No API key found"), "stderr: {stderr}");
        // Unknown subcommand → group help, exit 0.
        let (code, stdout, stderr) = run(&["account", "bogus"]).await;
        assert_eq!(code, 0);
        assert_eq!(stderr, "Unknown command: bogus\n\n");
        assert_eq!(stdout, format!("{}\n", help::ACCOUNT_GROUP_HELP));
    }

    #[tokio::test]
    async fn run_dispatch_no_prompt() {
        let (code, _, stderr) = run(&["run"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("No prompt provided."), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn models_json_flag_routing_and_rest_help() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        // --json immediately after the group is forwarded into models args.
        let (code, stdout, _) = run(&["models", "--json"]).await;
        assert_eq!(code, 1);
        assert!(stdout.starts_with("{\"error\":"), "stdout: {stdout}");
        // --help in the REST position still prints help.
        let (code, stdout, _) = run(&["models", "zzz", "--help"]).await;
        assert_eq!(code, 0);
        assert_eq!(stdout, format!("{}\n", help::MODELS_HELP));
        // A plain rest arg routes into the text-mode listing (agent down →
        // error exit).
        let (code, _, stderr) = run(&["models", "zzz"]).await;
        assert_eq!(code, 1);
        assert!(!stderr.is_empty(), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn session_and_doctor_dispatch() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        // session unknown subcommand → HANDLED_EXIT → 1.
        let (code, _, stderr) = run(&["session", "bogus", "id-1"]).await;
        assert_eq!(code, 1);
        assert!(
            stderr.contains("Unknown command: bogus"),
            "stderr: {stderr}"
        );
        // doctor runs to completion (isolated env → warns but exit 0).
        let dir = tempfile::tempdir().unwrap();
        let _path =
            crate::test_env::EnvGuard::set(&[("PATH", dir.path().as_os_str().to_os_string())]);
        let (code, stdout, _) = run(&["doctor"]).await;
        assert_eq!(code, 0);
        assert!(stdout.contains("Future Doctor"), "stdout: {stdout}");
    }
}
