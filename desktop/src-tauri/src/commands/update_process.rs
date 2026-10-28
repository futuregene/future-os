//! A prepared native helper owns its execution environment before services stop.
//! EOF before COMMIT cancels; after COMMIT the helper completes independently.
use future_host::console;
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "windows"))]
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::Duration,
};

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Completion {
    pub ok: bool,
    pub restored: bool,
    pub diagnostic: String,
}
impl Completion {
    pub(super) fn success() -> Self {
        Self {
            ok: true,
            restored: false,
            diagnostic: String::new(),
        }
    }
}
#[cfg(not(target_os = "windows"))]
pub(super) struct PreparedProcess {
    child: Child,
    input: Option<ChildStdin>,
    output: Option<BufReader<ChildStdout>>,
    committed: bool,
}
#[cfg(not(target_os = "windows"))]
impl PreparedProcess {
    pub(super) fn start(mut command: Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Cannot start update preparation: {e}"))?;
        let input = child.stdin.take();
        let output = child.stdout.take().ok_or("Update helper has no output")?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            let mut line = String::new();
            let result = reader.read_line(&mut line).map(|_| (line, reader));
            let _ = tx.send(result);
        });
        let mut prepared = Self {
            child,
            input,
            output: None,
            committed: false,
        };
        let (line, output) = rx
            .recv_timeout(Duration::from_secs(300))
            .map_err(|_| "Update preparation timed out; no installation was started")?
            .map_err(|e| e.to_string())?;
        if line.trim() != "READY" {
            let status = prepared.child.try_wait().map_err(|e| e.to_string())?;
            if status.is_some_and(|status| status.code() == Some(126)) {
                return Err("Authorization cancelled; current services are still running".into());
            }
            return Err(format!(
                "Update preparation was cancelled or refused: {}",
                line.trim()
            ));
        }
        prepared.output = Some(output);
        Ok(prepared)
    }
    pub(super) fn commit(mut self) -> Result<Completion, String> {
        if let Err(error) = self
            .input
            .as_mut()
            .ok_or("Update helper input closed")?
            .write_all(b"COMMIT\n")
        {
            // A partial command has no newline and cannot authorize install.
            return Ok(Completion {
                ok: false,
                restored: true,
                diagnostic: format!("Update commit was not delivered: {error}"),
            });
        }
        self.committed = true;
        self.input.take();
        let mut line = String::new();
        self.output
            .as_mut()
            .ok_or("Update helper output closed")?
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let completion: Completion = serde_json::from_str(line.trim())
            .map_err(|e| format!("Update helper result is unknown: {e}; {}", line.trim()))?;
        let status = self.child.wait().map_err(|e| e.to_string())?;
        if completion.ok && !status.success() {
            return Err(format!("Update helper exited with {status}"));
        }
        Ok(completion)
    }
}
#[cfg(not(target_os = "windows"))]
impl Drop for PreparedProcess {
    fn drop(&mut self) {
        // Closing stdin also cancels a privileged helper the user cannot kill.
        self.input.take();
        if !self.committed && self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}
#[cfg(not(target_os = "windows"))]
pub(super) fn await_commit() -> Result<bool, String> {
    console::println!("READY");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(line == "COMMIT\n")
}
#[cfg(not(target_os = "windows"))]
pub(super) fn complete(result: &Completion) -> Result<(), String> {
    console::println!(
        "{}",
        serde_json::to_string(result).map_err(|e| e.to_string())?
    );
    std::io::stdout().flush().map_err(|e| e.to_string())
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::*;
    fn fixture(marker: &std::path::Path) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(r#"printf 'READY\n'; IFS= read -r request || exit 0; test "$request" = COMMIT || exit 1; printf installed > "$1"; printf '{"ok":true,"restored":false,"diagnostic":""}\n'"#)
            .arg("update-fixture").arg(marker);
        command
    }
    #[test]
    fn prepared_helper_needs_explicit_commit_before_writing_program_files() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("installed");
        let prepared = PreparedProcess::start(fixture(&marker)).unwrap();
        assert!(!marker.exists());
        assert!(prepared.commit().unwrap().ok);
        assert_eq!(std::fs::read(marker).unwrap(), b"installed");
    }
    #[test]
    fn dropping_prepared_helper_cancels_without_installing() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("installed");
        drop(PreparedProcess::start(fixture(&marker)).unwrap());
        assert!(!marker.exists());
    }
}

#[cfg(not(target_os = "windows"))]
pub(super) fn agent_lease(record: &std::path::Path) -> Result<std::fs::File, String> {
    use fs2::FileExt;
    let home = record
        .ancestors()
        .nth(4)
        .ok_or("Invalid update record location")?;
    if record.file_name().and_then(|s| s.to_str()) != Some("result.json")
        || record
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            != Some("updates")
    {
        return Err("Invalid update record location".into());
    }
    let path = home.join("agent/agent-instance.lock");
    std::fs::create_dir_all(path.parent().ok_or("Invalid Agent lock location")?)
        .map_err(|e| e.to_string())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.try_lock_exclusive().map_err(|_| {
        "Another Agent started before installation; no program files were changed".to_owned()
    })?;
    Ok(file)
}
