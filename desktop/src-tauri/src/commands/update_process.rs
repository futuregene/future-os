//! A prepared native helper owns its execution environment before services stop.
//! EOF before COMMIT cancels; after COMMIT the helper completes independently.
#[cfg(not(target_os = "windows"))]
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

    /// The three ways a helper can refuse before authorising anything are three different
    /// answers, and only one of them is the user cancelling authorization.
    #[test]
    fn a_helper_that_does_not_announce_ready_is_reported_with_when_it_was_cancelled() {
        let script = |body: &'static str| {
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(body);
            command
        };
        // Spoke, but not READY: the refusal carries its own first line.
        let refused = PreparedProcess::start(script("printf 'DENIED\\n'"))
            .err()
            .expect("a helper that does not announce READY is refused");
        assert_eq!(
            refused, "Update preparation was cancelled or refused: DENIED",
            "the diagnostic carries what the helper actually said"
        );
        // Exited 126 before saying anything: the authorization dialog's own code, so the
        // parent may report cancellation instead of a fault.
        let cancelled = PreparedProcess::start(script("exit 126"))
            .err()
            .expect("exit 126 is a refusal");
        assert_eq!(
            cancelled, "Authorization cancelled; current services are still running",
            "126 is the dialog's cancellation, not a generic failure"
        );
    }

    /// A helper that stops reading between `READY` and `COMMIT` leaves nobody to write to.
    /// That is a failed commit — reported as one, not as a panic and not as a success.
    #[test]
    fn a_helper_that_stops_reading_before_the_commit_is_a_failed_commit() {
        // The child closes **its** stdin and stays alive, so the parent's `COMMIT` meets a
        // pipe with no readers — which is what makes this arm deterministic. A child that
        // merely exits after `READY` races the parent: the write then succeeds into the pipe
        // buffer and the failure moves to the completion read instead (measured in the full
        // suite, where the first version of this case failed exactly that way).
        let prepared = PreparedProcess::start({
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg("printf 'READY\\n'; exec 0<&-; sleep 5");
            command
        })
        .expect("READY arrives before the helper stops reading");
        let completion = prepared.commit().expect("a failed write is still a result");
        assert!(!completion.ok);
        assert!(completion.restored, "nothing was replaced");
        assert!(
            completion
                .diagnostic
                .contains("Update commit was not delivered"),
            "the diagnostic names the lost commit, got {:?}",
            completion.diagnostic
        );
    }

    /// A completion that claims success while the helper exits non-zero is the one shape the
    /// parent may not accept: the claim and the process disagree about what happened.
    #[test]
    fn a_successful_completion_from_a_helper_that_failed_is_refused() {
        let prepared = PreparedProcess::start({
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(
                "printf 'READY\\n'; IFS= read -r request || exit 0; \
                 printf '{\"ok\":true,\"restored\":false,\"diagnostic\":\"\"}\\n'; exit 3",
            );
            command
        })
        .expect("READY");
        let error = prepared
            .commit()
            .expect_err("a claimed success from a failed helper is refused");
        assert!(
            error.contains("Update helper exited with"),
            "the refusal names the exit status, got {error:?}"
        );
    }

    /// `agent_lease` derives the Agent's home from the record it was handed, and refuses a
    /// path that is not `<home>/app/updates/<id>/result.json` rather than writing a lock file
    /// somewhere unexpected.
    #[test]
    fn the_agent_lease_refuses_a_record_path_that_is_not_an_update_record() {
        let root = tempfile::tempdir().unwrap();
        for record in [
            root.path().join("result.json"),
            root.path().join("elsewhere/app/notes/result.json"),
        ] {
            let error = agent_lease(&record).expect_err("not an update record");
            assert_eq!(error, "Invalid update record location");
        }
        assert!(
            !root.path().join("agent").exists(),
            "a refused path must not have created the Agent lock directory"
        );
    }

    /// The refusal the installation depends on: a **live** holder of the Agent instance lock.
    /// The lock is what stops a swap of program files under a running Agent, so the arm has to
    /// be observed against a real holder rather than an unusable path — a path-only test would
    /// pass even if `try_lock_exclusive` never ran.
    ///
    /// `flock(2)` locks belong to the open file description, so two descriptors on one file in
    /// one process are two holders; no child process is needed to model "another Agent".
    #[test]
    fn the_agent_lease_refuses_while_another_agent_holds_the_instance_lock() {
        use fs2::FileExt;
        let root = tempfile::tempdir().unwrap();
        let record = root.path().join("app/updates/update-lease/result.json");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, b"{}").unwrap();
        let lock_path = root.path().join("agent/agent-instance.lock");
        std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        holder.lock_exclusive().unwrap();

        let error = agent_lease(&record).expect_err("a running Agent holds the instance lock");
        assert_eq!(
            error,
            "Another Agent started before installation; no program files were changed"
        );
        assert!(
            std::fs::read_dir(root.path().join("agent"))
                .unwrap()
                .any(|entry| entry.unwrap().path() == lock_path),
            "the refusal is about the lock this path names, not about a path it invented"
        );

        // The same call succeeds once the holder lets go: the arm above was the lock, not the
        // fixture.
        holder.unlock().unwrap();
        let lease = agent_lease(&record).expect("no holder, so the lease is available");
        assert_eq!(lease.metadata().unwrap().len(), 0);
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
