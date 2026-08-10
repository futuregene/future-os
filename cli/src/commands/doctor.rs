//! `future doctor` — 1:1 port of cli/src/commands/doctor.ts.
//!
//! Environment diagnostic: login, components, agent connectivity, config
//! files, providers/models, sessions, skills. Output uses the same ANSI
//! color codes as the TS version.

use crate::commands::auth::{get_future_auth_entry, load_auth_file, strip_api_suffix};
use crate::commands::skills::{
    fetch_skills, get_installed_skill_ids, read_skill_md_version, skills_dir,
};
use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use crate::utils::files::which;
use crate::utils::platform::get_platform_url;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

// ── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Issue,
}

struct CheckResult {
    name: String,
    status: Status,
    lines: Vec<String>,
}

// ── Colors ─────────────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";

/// `icon(status)`.
fn icon(status: Status) -> String {
    match status {
        Status::Ok => format!("{GREEN}[ok]{RESET}"),
        Status::Warn => format!("{YELLOW}[--]{RESET}"),
        Status::Issue => format!("{RED}[!!]{RESET}"),
    }
}

/// `colorName(status, text)`.
fn color_name(status: Status, text: &str) -> String {
    match status {
        Status::Ok => format!("{GREEN}{text}{RESET}"),
        Status::Warn => format!("{YELLOW}{text}{RESET}"),
        Status::Issue => format!("{RED}{text}{RESET}"),
    }
}

// ── Constants ──────────────────────────────────────────────────────────────

fn agent_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".future")
        .join("agent")
}
fn auth_file_path() -> PathBuf {
    agent_dir().join("auth.json")
}
fn models_file_path() -> PathBuf {
    agent_dir().join("models.json")
}
fn settings_file_path() -> PathBuf {
    agent_dir().join("settings.json")
}
fn sessions_dir_path() -> PathBuf {
    agent_dir().join("sessions")
}

// ── Entry ──────────────────────────────────────────────────────────────────

/// `doctor()`.
pub async fn doctor(out: &Output) -> Result<(), String> {
    // `console.log(\`${C.bold}Future Doctor${C.reset} — checking environment...\n\`)`
    out.log(&format!(
        "{BOLD}Future Doctor{RESET} — checking environment...\n"
    ));

    let mut results: Vec<CheckResult> = Vec::new();

    // 1. Login.
    results.push(check_login().await);
    // 2. Components.
    results.push(check_agent().await);
    results.push(check_component("future", "CLI").await);
    results.push(check_component("future-tui", "TUI").await);
    results.push(check_component("future-desktop", "Desktop").await);
    results.push(check_component("future-channel", "Channel bridge").await);
    // 3. Configuration.
    results.push(check_auth_config().await);
    results.push(check_models_config().await);
    results.push(check_settings_config().await);
    // 4. Providers & models.
    results.push(check_providers().await);
    // 5. Sessions.
    results.push(check_sessions().await);
    // 6. Skills.
    results.push(check_skills().await);

    print_results(&results, out);

    let issues = results.iter().filter(|r| r.status == Status::Issue).count();
    let warns = results.iter().filter(|r| r.status == Status::Warn).count();
    let problem_count = issues + warns;

    if problem_count == 0 {
        // `console.log(\`${GREEN}All checks passed.${C.reset}\n\`)`
        out.log(&format!("{GREEN}All checks passed.{RESET}\n"));
    }
    Ok(())
}

// ── 1. Login ───────────────────────────────────────────────────────────────

async fn check_login() -> CheckResult {
    let result = async {
        let auth = load_auth_file().await?;
        let entry = get_future_auth_entry(&auth);
        let Some(entry) = entry else {
            return Err(String::new());
        };
        let Some(key) = entry.key else {
            return Err(String::new());
        };
        if key.is_empty() {
            return Err(String::new());
        }
        // `entry.base_url ? entry.base_url.replace(/\/api\/?$/, "")
        //              : await getPlatformUrl().catch(() => "unknown")`
        let platform_url = match &entry.base_url {
            Some(base_url) => strip_api_suffix(base_url),
            None => get_platform_url(None).await,
        };
        Ok::<String, String>(platform_url)
    }
    .await;
    match result {
        Ok(platform_url) => CheckResult {
            name: "Login".to_string(),
            status: Status::Ok,
            lines: vec![format!("Logged in to {platform_url}")],
        },
        Err(_) => CheckResult {
            name: "Login".to_string(),
            status: Status::Warn,
            lines: vec!["Not logged in — run `future auth login`".to_string()],
        },
    }
}

// ── 2. Components ──────────────────────────────────────────────────────────

async fn check_component(bin: &str, label: &str) -> CheckResult {
    let bin_path = which(bin).await;
    let Some(bin_path) = bin_path else {
        return CheckResult {
            name: label.to_string(),
            status: Status::Warn,
            lines: vec![format!("{bin} not found on PATH — run `make install`")],
        };
    };
    let version = get_binary_version(&bin_path).await;
    let lines = vec![version
        .map(|v| format!("{bin_path}  {DIM}({v}){RESET}"))
        .unwrap_or_else(|| bin_path.clone())];
    CheckResult {
        name: label.to_string(),
        status: Status::Ok,
        lines,
    }
}

/// `getBinaryVersion(binPath)` — run `<bin> --version` with a 5s timeout and
/// pick the first plausible version line from stdout+stderr.
async fn get_binary_version(bin_path: &str) -> Option<String> {
    // Node's `execFile(..., { timeout: 5000 })` kills the child at the
    // deadline (SIGTERM) but still resolves with the output accumulated so
    // far — some binaries, like the GUI sidecar, print one line and never
    // exit. So: spawn with piped stdio, drain the pipes on background tasks
    // (preserving partial output), and on timeout kill + reap the child.
    let mut child = tokio::process::Command::new(bin_path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let stderr = child.stderr.take()?;
    let out_task = tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::new();
        let mut stdout = stdout;
        let _ = tokio::io::copy(&mut stdout, &mut buf).await;
        buf
    });
    let err_task = tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::new();
        let mut stderr = stderr;
        let _ = tokio::io::copy(&mut stderr, &mut buf).await;
        buf
    });

    // Poll exit; at the 5s deadline kill + reap (SIGKILL vs Node's SIGTERM —
    // output is already captured either way, so this only tightens cleanup).
    let deadline = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => {
                let _ = child.kill().await;
                let _ = child.wait().await; // reap; closes pipes → readers finish
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
            }
        }
    }
    let out_buf = out_task.await.ok()?;
    let err_buf = err_task.await.ok()?;

    let mut candidates: Vec<String> = String::from_utf8_lossy(&out_buf)
        .trim()
        .split('\n')
        .map(str::to_string)
        .collect();
    candidates.extend(
        String::from_utf8_lossy(&err_buf)
            .trim()
            .split('\n')
            .map(str::to_string),
    );
    candidates
        .into_iter()
        .find(|line| is_version_candidate(line))
}

/// `/^\d{4}-\d{2}-\d{2}T/` — a timestamp-prefixed line.
fn starts_with_iso_datetime(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 11
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4] == b'-'
        && bytes[5].is_ascii_digit()
        && bytes[6].is_ascii_digit()
        && bytes[7] == b'-'
        && bytes[8].is_ascii_digit()
        && bytes[9].is_ascii_digit()
        && bytes[10] == b'T'
}

/// `/\b(INFO|WARN|ERROR|DEBUG|TRACE)\b/` — a log-level word.
fn contains_log_level(line: &str) -> bool {
    for level in ["INFO", "WARN", "ERROR", "DEBUG", "TRACE"] {
        if line
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w == level)
        {
            return true;
        }
    }
    false
}

fn is_version_candidate(line: &str) -> bool {
    !line.is_empty() && !starts_with_iso_datetime(line) && !contains_log_level(line)
}

// ── 2b. Agent (binary + connectivity combined) ────────────────────────────

async fn check_agent() -> CheckResult {
    let grpc_addr = grpc_addr();
    let bin_path = which("future-agent").await;
    let mut lines: Vec<String> = Vec::new();

    if let Some(bin_path) = &bin_path {
        let version = get_binary_version(bin_path).await;
        lines.push(
            version
                .map(|v| format!("{bin_path}  {DIM}({v}){RESET}"))
                .unwrap_or_else(|| bin_path.clone()),
        );
    } else {
        lines.push("future-agent not found on PATH — run `make install`".to_string());
    }

    let client = RunClient::new(&grpc_addr);
    match client.get_state(None).await {
        Ok(state) => {
            // `state.version ?? "?"`
            let version = state.get("version").and_then(Value::as_str).unwrap_or("?");
            lines.push(format!(
                "Connected to {grpc_addr}  {DIM}(v{version}){RESET}"
            ));
            CheckResult {
                name: "Agent".to_string(),
                status: Status::Ok,
                lines,
            }
        }
        Err(_) => {
            if bin_path.is_none() {
                return CheckResult {
                    name: "Agent".to_string(),
                    status: Status::Warn,
                    lines,
                };
            }
            lines.push(format!(
                "{RED}Not running — start with: future-agent{RESET}"
            ));
            CheckResult {
                name: "Agent".to_string(),
                status: Status::Issue,
                lines,
            }
        }
    }
}

// ── 3. Configuration ──────────────────────────────────────────────────────

async fn check_auth_config() -> CheckResult {
    let path = auth_file_path();
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return CheckResult {
            name: "Auth config".to_string(),
            status: Status::Warn,
            lines: vec![format!(
                "{} not found — run `future auth login` or create manually",
                path.display()
            )],
        };
    }
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(parsed) => {
                // `Object.keys(raw).filter(k => v && typeof v === "object" && "key" in v)`
                let keys: Vec<&String> = parsed
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .filter(|(_, v)| v.is_object() && v.get("key").is_some())
                            .map(|(k, _)| k)
                            .collect()
                    })
                    .unwrap_or_default();
                let n = keys.len();
                CheckResult {
                    name: "Auth config".to_string(),
                    status: if n > 0 { Status::Ok } else { Status::Warn },
                    lines: vec![if n > 0 {
                        format!("{} — {n} provider key(s)", path.display())
                    } else {
                        format!("{} exists but no keys configured", path.display())
                    }],
                }
            }
            Err(_) => CheckResult {
                name: "Auth config".to_string(),
                status: Status::Issue,
                lines: vec![format!("{} exists but is not valid JSON", path.display())],
            },
        },
        Err(_) => CheckResult {
            name: "Auth config".to_string(),
            status: Status::Issue,
            lines: vec![format!("{} exists but is not valid JSON", path.display())],
        },
    }
}

async fn check_models_config() -> CheckResult {
    let path = models_file_path();
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return CheckResult {
            name: "Models config".to_string(),
            status: Status::Ok,
            lines: vec![format!(
                "{} not found (using built-in catalog)",
                path.display()
            )],
        };
    }
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(parsed) => {
                // `(raw.providers as Record<string, unknown>) ?? {}`
                let providers = parsed
                    .get("providers")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                // `Object.keys(providers).filter(id => id !== "future" && !isOverrideOnly(...))`
                let custom_ids: Vec<String> = providers
                    .iter()
                    .filter(|(id, config)| id.as_str() != "future" && !is_override_only(config))
                    .map(|(id, _)| id.clone())
                    .collect();
                CheckResult {
                    name: "Models config".to_string(),
                    status: Status::Ok,
                    lines: vec![
                        format!("{} exists", path.display()),
                        if !custom_ids.is_empty() {
                            format!("Custom providers: {}", custom_ids.join(", "))
                        } else {
                            "No custom providers defined".to_string()
                        },
                    ],
                }
            }
            Err(_) => CheckResult {
                name: "Models config".to_string(),
                status: Status::Issue,
                lines: vec![format!("{} exists but is not valid JSON", path.display())],
            },
        },
        Err(_) => CheckResult {
            name: "Models config".to_string(),
            status: Status::Issue,
            lines: vec![format!("{} exists but is not valid JSON", path.display())],
        },
    }
}

/// `isOverrideOnly(config)` from doctor.ts — JS truthiness semantics.
fn is_override_only(config: &Value) -> bool {
    let Some(c) = config.as_object() else {
        return false;
    };
    !js_truthy(c.get("name").unwrap_or(&Value::Null))
        && !js_truthy(c.get("api").unwrap_or(&Value::Null))
        && c.get("models")
            .and_then(Value::as_array)
            .is_none_or(|a| a.is_empty())
}

/// JS truthiness for JSON values (null/false/0/""/[] are falsy).
fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(_) => true,
    }
}

async fn check_settings_config() -> CheckResult {
    let path = settings_file_path();
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return CheckResult {
            name: "Agent settings".to_string(),
            status: Status::Ok,
            lines: vec![format!("{} not found (defaults apply)", path.display())],
        };
    }
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(_) => CheckResult {
                name: "Agent settings".to_string(),
                status: Status::Ok,
                lines: vec![format!("{} exists", path.display())],
            },
            Err(_) => CheckResult {
                name: "Agent settings".to_string(),
                status: Status::Issue,
                lines: vec![format!("{} exists but is not valid JSON", path.display())],
            },
        },
        Err(_) => CheckResult {
            name: "Agent settings".to_string(),
            status: Status::Issue,
            lines: vec![format!("{} exists but is not valid JSON", path.display())],
        },
    }
}

// ── 4. Providers ───────────────────────────────────────────────────────────

async fn check_providers() -> CheckResult {
    // `id → label`
    let mut all_providers: BTreeMap<String, String> = BTreeMap::new();

    // Collect from auth.json.
    if let Ok(raw) = tokio::fs::read_to_string(auth_file_path()).await {
        if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
            if let Some(obj) = parsed.as_object() {
                for (id, v) in obj {
                    if v.is_object() && v.get("key").is_some() {
                        all_providers.insert(id.clone(), "[key]".to_string());
                    }
                }
            }
        }
    }

    // Collect from models.json (custom providers).
    if let Ok(raw) = tokio::fs::read_to_string(models_file_path()).await {
        if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
            if let Some(providers) = parsed.get("providers").and_then(Value::as_object) {
                for (id, config) in providers {
                    if id == "future" || is_override_only(config) {
                        continue;
                    }
                    // `existing ? \`${existing} + custom\` : "custom"`
                    let label = match all_providers.get(id) {
                        Some(existing) => format!("{existing} + custom"),
                        None => "custom".to_string(),
                    };
                    all_providers.insert(id.clone(), label);
                }
            }
        }
    }

    if !all_providers.is_empty() {
        // `[...allProviders.entries()].sort((a, b) => a[0].localeCompare(b[0]))`
        let mut lines: Vec<String> = all_providers
            .iter()
            .map(|(id, label)| format!("  {id} {DIM}({label}){RESET}"))
            .collect();
        lines.insert(0, format!("{} provider(s) configured", all_providers.len()));
        CheckResult {
            name: "Providers".to_string(),
            status: Status::Ok,
            lines,
        }
    } else {
        CheckResult {
            name: "Providers".to_string(),
            status: Status::Warn,
            lines: vec![
                "No providers configured — run `future auth login` to get started.".to_string(),
            ],
        }
    }
}

// ── 5. Sessions ────────────────────────────────────────────────────────────

async fn check_sessions() -> CheckResult {
    let mut lines: Vec<String> = Vec::new();
    let sessions_dir = sessions_dir_path();

    if tokio::fs::metadata(&sessions_dir).await.is_ok() {
        match tokio::fs::read_dir(&sessions_dir).await {
            Ok(mut entries) => {
                let mut jsonl_count = 0;
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if entry.file_name().to_string_lossy().ends_with(".jsonl") {
                        jsonl_count += 1;
                    }
                }
                lines.push(format!(
                    "{jsonl_count} JSONL file(s) in {}",
                    sessions_dir.display()
                ));
            }
            Err(_) => lines.push(format!("Cannot read {}", sessions_dir.display())),
        }
    } else {
        lines.push("No session directory — no sessions created yet".to_string());
    }

    // Agent connectivity — failures ignored.
    let client = RunClient::new(&grpc_addr());
    if let Ok(data) = client.list_sessions().await {
        let n = data
            .get("sessions")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        if n > 0 {
            lines.push(format!("{n} session(s) tracked by agent"));
        }
    }

    CheckResult {
        name: "Sessions".to_string(),
        status: Status::Ok,
        lines,
    }
}

// ── 6. Skills ──────────────────────────────────────────────────────────────

async fn check_skills() -> CheckResult {
    let mut lines: Vec<String> = Vec::new();
    let installed = get_installed_skill_ids().await;
    let skills_dir = skills_dir();

    if installed.is_empty() {
        lines.push("No skills installed.".to_string());
        // `fs.existsSync(SKILLS_DIR) ? "" : \` ${C.dim}(directory not found)${C.reset}\``
        let marker = if tokio::fs::metadata(&skills_dir).await.is_ok() {
            String::new()
        } else {
            format!(" {DIM}(directory not found){RESET}")
        };
        lines.push(format!("  {}{}", skills_dir.display(), marker));
    }

    if !installed.is_empty() {
        let mut up_to_date: Vec<String> = Vec::new();
        let mut needs_update: Vec<String> = Vec::new();

        // `try { const platformUrl = await getPlatformUrl(); const allSkills =
        //      await fetchSkills(platformUrl); ... } catch { /* offline */ }`
        let catalog_result: Result<HashMap<String, crate::commands::skills::SkillInfo>, String> =
            async {
                let platform_url = get_platform_url(None).await;
                let all = fetch_skills(&platform_url).await?;
                Ok(all.into_iter().map(|s| (s.id.clone(), s)).collect())
            }
            .await;

        let mut ids: Vec<&String> = installed.iter().collect();
        ids.sort();

        match catalog_result {
            Ok(catalog) => {
                for id in ids {
                    let skill = catalog.get(id);
                    let local_ver =
                        read_skill_md_version(&skills_dir.join(id).join("SKILL.md")).await;
                    // `if (localVer && skill?.latest_version && localVer !== skill.latest_version)`
                    if let (Some(local), Some(skill)) = (local_ver.as_deref(), skill) {
                        if let Some(latest) = skill.latest_version.as_deref() {
                            if local != latest {
                                needs_update.push(format!("{id}: {local} {DIM}→{RESET} {latest}"));
                                continue;
                            }
                        }
                    }
                    let ver = local_ver.map(|v| format!(" (v{v})")).unwrap_or_default();
                    up_to_date.push(format!("{id}{ver}"));
                }
            }
            Err(_) => {
                // Offline — all up-to-date without version comparison.
                for id in ids {
                    let local_ver =
                        read_skill_md_version(&skills_dir.join(id).join("SKILL.md")).await;
                    let ver = local_ver.map(|v| format!(" (v{v})")).unwrap_or_default();
                    up_to_date.push(format!("{id}{ver}"));
                }
            }
        }

        if !up_to_date.is_empty() {
            lines.push(format!("  Up to date: {}", up_to_date.join(", ")));
        }
        if !needs_update.is_empty() {
            lines.push(format!("  Updates available: {}", needs_update.join(", ")));
            lines.push(format!(
                "  Run {BOLD}future skills update{RESET} to upgrade"
            ));
        }
    }

    CheckResult {
        name: "Skills".to_string(),
        status: if installed.is_empty() {
            Status::Warn
        } else {
            Status::Ok
        },
        lines,
    }
}

// ── Output ─────────────────────────────────────────────────────────────────

fn print_results(results: &[CheckResult], out: &Output) {
    for r in results {
        // `console.log(\`${icon(r.status)} ${colorName(r.status, r.name)}\`)`
        // — note the space between icon and name.
        out.log(&format!(
            "{} {}",
            icon(r.status),
            color_name(r.status, &r.name)
        ));
        for line in &r.lines {
            out.log(&format!("      {line}"));
        }
        out.log("");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;
    use crate::test_env::EnvGuard;
    use std::ffi::OsString;

    /// Isolated HOME + empty PATH + unreachable gRPC address.
    fn isolate_env() -> EnvGuard {
        let dir = tempfile::tempdir().unwrap();
        EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("PATH", dir.path().join("empty-bin").into_os_string()),
            ("FUTURE_AGENT_GRPC_ADDR", OsString::from("127.0.0.1:1")),
        ])
    }

    async fn run_doctor() -> (i32, String, String) {
        let (out, cap) = Output::memory();
        let result = doctor(&out).await;
        let code = if result.is_ok() { 0 } else { 1 };
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        (code, stdout, stderr)
    }

    #[tokio::test]
    async fn doctor_clean_environment_output() {
        let _guard = crate::test_env::lock_env().await;
        let _env = isolate_env();
        let (code, stdout, stderr) = run_doctor().await;
        assert_eq!(code, 0);
        assert_eq!(stderr, "");

        // Header + all 12 check sections present.
        assert!(stdout.starts_with("\u{1b}[1mFuture Doctor\u{1b}[0m — checking environment...\n"));
        assert!(stdout.contains("Not logged in — run `future auth login`"));
        assert!(stdout.contains("future-agent not found on PATH"));
        assert!(stdout.contains("future not found on PATH — run `make install`"));
        assert!(stdout.contains("future-tui not found on PATH"));
        assert!(stdout.contains("future-desktop not found on PATH"));
        assert!(stdout.contains("future-channel not found on PATH"));
        assert!(stdout.contains("Auth config"));
        assert!(stdout.contains("not found — run `future auth login` or create manually"));
        assert!(stdout.contains("Models config"));
        assert!(stdout.contains("(using built-in catalog)"));
        assert!(stdout.contains("Agent settings"));
        assert!(stdout.contains("(defaults apply)"));
        assert!(stdout.contains("No providers configured"));
        assert!(stdout.contains("No session directory — no sessions created yet"));
        assert!(stdout.contains("No skills installed."));
        assert!(stdout.contains("(directory not found)"));
        // Warns exist → no "All checks passed."
        assert!(!stdout.contains("All checks passed."));
    }

    #[tokio::test]
    async fn doctor_detects_agent_binary_not_running_as_issue() {
        let _guard = crate::test_env::lock_env().await;
        // Isolate HOME + gRPC address but keep the real PATH so `which` and
        // the fake agent script resolve.
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("FUTURE_AGENT_GRPC_ADDR", OsString::from("127.0.0.1:1")),
        ]);
        // Put a fake future-agent FIRST on PATH that answers --version.
        let bin_dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("fake-bin");
        tokio::fs::create_dir_all(&bin_dir).await.unwrap();
        let agent = bin_dir.join("future-agent");
        tokio::fs::write(&agent, "#!/bin/sh\necho \"future-agent v0.0.1\"\n")
            .await
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755))
                .await
                .unwrap();
        }
        let mut paths = vec![bin_dir.clone()];
        if let Some(p) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

        let (code, stdout, _) = run_doctor().await;
        assert_eq!(code, 0);
        assert!(
            stdout.contains("[!!]"),
            "expected an issue section: {stdout}"
        );
        assert!(stdout.contains("Not running — start with: future-agent"));
        // Version captured from the fake binary's --version output.
        assert!(stdout.contains("v0.0.1"), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn doctor_captures_partial_version_from_hanging_binary() {
        // execFile-parity: a binary that prints one line then never exits
        // (like the GUI sidecar when the agent is reachable) must still
        // contribute its line — the 5s timeout keeps partial output and
        // kills the child rather than dropping everything.
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("FUTURE_AGENT_GRPC_ADDR", OsString::from("127.0.0.1:1")),
        ]);
        let bin_dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("fake-bin");
        tokio::fs::create_dir_all(&bin_dir).await.unwrap();
        let desktop = bin_dir.join("future-desktop");
        tokio::fs::write(
            &desktop,
            "#!/bin/sh\necho \"FutureOS: agent already reachable at 127.0.0.1:50051\"\nsleep 30\n",
        )
        .await
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&desktop, std::fs::Permissions::from_mode(0o755))
                .await
                .unwrap();
        }
        let mut paths = vec![bin_dir.clone()];
        if let Some(p) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&p));
        }
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

        let (code, stdout, _) = run_doctor().await;
        assert_eq!(code, 0);
        assert!(
            stdout.contains("(FutureOS: agent already reachable at 127.0.0.1:50051)"),
            "expected partial --version output to survive the timeout: {stdout}"
        );
    }

    #[tokio::test]
    async fn doctor_reports_configured_providers() {
        let _guard = crate::test_env::lock_env().await;
        let _env = isolate_env();
        // auth.json with a future key → Login ok + provider listed.
        let home = std::env::var("HOME").unwrap();
        let auth_path = std::path::Path::new(&home)
            .join(".future")
            .join("agent")
            .join("auth.json");
        tokio::fs::create_dir_all(auth_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &auth_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "future": { "key": "k", "base_url": "https://x/api" },
                "openai": { "key": "sk-..." }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let (code, stdout, _) = run_doctor().await;
        assert_eq!(code, 0);
        assert!(stdout.contains("Logged in to https://x"));
        assert!(stdout.contains("2 provider(s) configured"));
        // Provider rows carry dim-wrapped labels, e.g. `  future \x1b[2m([key])\x1b[0m`.
        assert!(
            stdout.contains("future \u{1b}[2m([key])\u{1b}[0m"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("openai \u{1b}[2m([key])\u{1b}[0m"));
        // Auth config ok.
        assert!(stdout.contains("2 provider key(s)"));
    }

    #[tokio::test]
    async fn version_line_filters() {
        assert!(is_version_candidate("future-agent 0.0.1"));
        assert!(is_version_candidate("v1.2.3"));
        assert!(!is_version_candidate(""));
        assert!(!is_version_candidate("2026-08-06T12:00:00 INFO hello"));
        assert!(!is_version_candidate("INFO: starting"));
        assert!(!is_version_candidate("ERROR boom"));
        assert!(is_version_candidate("future 0.0.1568-479c8fee+local"));
    }

    // ── Full-house + remaining branches ─────────────────────────────

    /// Write a file under the isolated HOME's agent dir.
    async fn write_agent_file(rel: &str, body: &str) {
        let path = agent_dir().join(rel);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, body).await.unwrap();
    }

    #[tokio::test]
    async fn doctor_full_house_all_ok() {
        let _guard = crate::test_env::lock_env().await;
        let dir = tempfile::tempdir().unwrap();
        // gRPC mock: connected agent with version + two sessions.
        let mut agent = crate::test_server::MockAgent::default();
        agent
            .responses
            .insert("get_state".into(), "{\"version\":\"1.2.3\"}".into());
        agent.responses.insert(
            "list_sessions".into(),
            "{\"sessions\":[{\"id\":\"s1\"},{\"id\":\"s2\"}]}".into(),
        );
        let addr = crate::test_server::spawn_mock(agent).await;
        let _env = EnvGuard::set(&[
            ("HOME", dir.path().as_os_str().to_owned()),
            ("PATH", dir.path().join("empty-bin").into_os_string()),
            ("FUTURE_AGENT_GRPC_ADDR", OsString::from(addr)),
        ]);

        // auth.json: future key WITHOUT base_url → get_platform_url fallback.
        write_agent_file(
            "auth.json",
            "{\"future\": {\"key\": \"k\"}, \"custom-p\": {\"key\": \"x\"}}",
        )
        .await;
        // models.json: a custom provider, an override-only entry, "future".
        write_agent_file(
            "models.json",
            "{\"providers\": {\
                \"custom-p\": {\"name\": \"Custom\", \"models\": [{\"id\": \"m\"}]},\
                \"override-only\": {\"models\": []},\
                \"future\": {\"name\": \"Future\"}\
            }}",
        )
        .await;
        write_agent_file("settings.json", "{}").await;
        // Sessions dir with two JSONL + one non-JSONL file.
        write_agent_file("sessions/a.jsonl", "{}\n").await;
        write_agent_file("sessions/b.jsonl", "{}\n").await;
        write_agent_file("sessions/notes.txt", "x").await;
        // Skills: one up-to-date, one needing update — catalog via HTTP mock.
        let skills = skills_dir();
        tokio::fs::create_dir_all(skills.join("future-a")).await.unwrap();
        tokio::fs::write(skills.join("future-a/SKILL.md"), "---\nversion: 1.0\n---\n")
            .await
            .unwrap();
        tokio::fs::create_dir_all(skills.join("future-b")).await.unwrap();
        tokio::fs::write(skills.join("future-b/SKILL.md"), "---\nversion: 0.9\n---\n")
            .await
            .unwrap();
        let catalog = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/skills",
            200,
            "{\"skills\":[\
                {\"id\":\"future-a\",\"latest_version\":\"1.0\"},\
                {\"id\":\"future-b\",\"latest_version\":\"2.0\"}\
            ]}",
        )])
        .await;
        // Point the platform at the catalog mock (base_url in auth.json).
        write_agent_file(
            "auth.json",
            &format!(
                "{{\"future\": {{\"key\": \"k\", \"base_url\": \"{catalog}/api\"}}, \"custom-p\": {{\"key\": \"x\"}}}}"
            ),
        )
        .await;

        let (code, stdout, stderr) = run_doctor().await;
        assert_eq!(code, 0);
        assert!(stderr.is_empty());
        // Agent connected with version.
        assert!(stdout.contains("Connected to"), "stdout: {stdout}");
        assert!(stdout.contains("(v1.2.3)"), "stdout: {stdout}");
        // Login ok with the mock platform (base_url /api stripped).
        assert!(stdout.contains(&format!("Logged in to {catalog}")), "stdout: {stdout}");
        // Auth config: 2 provider keys.
        assert!(stdout.contains("2 provider key(s)"), "stdout: {stdout}");
        // Models config: custom provider listed, override-only + future hidden.
        assert!(stdout.contains("Custom providers: custom-p"), "stdout: {stdout}");
        assert!(!stdout.contains("override-only"), "stdout: {stdout}");
        // Settings exists.
        assert!(stdout.contains("settings.json exists"), "stdout: {stdout}");
        // Providers: custom-p merged label, future from key.
        assert!(stdout.contains("2 provider(s) configured"), "stdout: {stdout}");
        assert!(stdout.contains("custom-p \u{1b}[2m([key] + custom)\u{1b}[0m"), "stdout: {stdout}");
        // Sessions: 2 jsonl + agent-tracked count.
        assert!(stdout.contains("2 JSONL file(s)"), "stdout: {stdout}");
        assert!(stdout.contains("2 session(s) tracked by agent"), "stdout: {stdout}");
        // Skills: one up-to-date with version, one needs update.
        assert!(stdout.contains("Up to date: future-a (v1.0)"), "stdout: {stdout}");
        assert!(stdout.contains("Updates available: future-b: 0.9"), "stdout: {stdout}");
        assert!(stdout.contains("future skills update"), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn doctor_config_issue_variants() {
        let _guard = crate::test_env::lock_env().await;
        let _env = isolate_env();
        // auth.json invalid JSON → Issue; models.json invalid → Issue;
        // settings.json invalid → Issue.
        write_agent_file("auth.json", "{bad").await;
        write_agent_file("models.json", "{bad").await;
        write_agent_file("settings.json", "{bad").await;
        let (_, stdout, _) = run_doctor().await;
        assert_eq!(stdout.matches("exists but is not valid JSON").count(), 3, "stdout: {stdout}");

        // auth.json exists but has no keys → Warn line.
        write_agent_file("auth.json", "{\"future\": {\"base_url\": \"https://x\"}}").await;
        write_agent_file("models.json", "{}").await;
        write_agent_file("settings.json", "{}").await;
        let (_, stdout, _) = run_doctor().await;
        assert!(stdout.contains("exists but no keys configured"), "stdout: {stdout}");
        assert!(stdout.contains("No custom providers defined"), "stdout: {stdout}");
    }

    #[tokio::test]
    async fn doctor_skills_offline_and_unversioned() {
        let _guard = crate::test_env::lock_env().await;
        let _env = isolate_env();
        // Installed skill, no version in SKILL.md; catalog unreachable.
        let skills = skills_dir();
        tokio::fs::create_dir_all(skills.join("future-a")).await.unwrap();
        tokio::fs::write(skills.join("future-a/SKILL.md"), "# no frontmatter\n")
            .await
            .unwrap();
        let (_, stdout, _) = run_doctor().await;
        // Offline: listed up-to-date WITHOUT a version suffix.
        assert!(stdout.contains("Up to date: future-a\n"), "stdout: {stdout}");
        // Skills dir exists → no "(directory not found)" marker even though
        // installed is non-empty here (marker only in the empty branch).
        assert!(!stdout.contains("(directory not found)"), "stdout: {stdout}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn doctor_sessions_dir_unreadable() {
        let _guard = crate::test_env::lock_env().await;
        let _env = isolate_env();
        let sessions = sessions_dir_path();
        tokio::fs::create_dir_all(&sessions).await.unwrap();
        // chmod 000 → read_dir fails → "Cannot read" line.
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o000))
            .await
            .unwrap();
        let (_, stdout, _) = run_doctor().await;
        assert!(stdout.contains("Cannot read"), "stdout: {stdout}");
        // Restore so the tempdir can be cleaned up.
        tokio::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
    }

    #[test]
    fn js_truthy_and_override_only() {
        assert!(!js_truthy(&Value::Null));
        assert!(!js_truthy(&serde_json::json!(false)));
        assert!(js_truthy(&serde_json::json!(true)));
        assert!(!js_truthy(&serde_json::json!(0)));
        assert!(js_truthy(&serde_json::json!(1.5)));
        assert!(js_truthy(&serde_json::json!(-0.5)));
        assert!(!js_truthy(&serde_json::json!("")));
        assert!(js_truthy(&serde_json::json!("x")));
        assert!(!js_truthy(&serde_json::json!([])));
        assert!(js_truthy(&serde_json::json!([1])));
        assert!(js_truthy(&serde_json::json!({})));
        // NaN-free JSON: no NaN case exists for Value::Number.

        // is_override_only: non-object → false.
        assert!(!is_override_only(&serde_json::json!(5)));
        // Empty object → override-only.
        assert!(is_override_only(&serde_json::json!({})));
        // name/api/models presence flips it.
        assert!(!is_override_only(&serde_json::json!({"name": "N"})));
        assert!(!is_override_only(&serde_json::json!({"api": "a"})));
        assert!(!is_override_only(&serde_json::json!({"models": [{}]})));
        assert!(is_override_only(&serde_json::json!({"models": []})));
    }
}
