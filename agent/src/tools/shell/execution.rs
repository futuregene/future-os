use super::super::*;
use super::*;

/// Pre-execution escalation: the model explicitly asked for unsandboxed
/// execution. Returns the decision-driven outcome to propagate, or None when
/// no escalation channel is registered (caller falls through to a normal
/// sandboxed run).
pub(crate) async fn pre_execution_escalation(
    escalation: &Option<crate::sandbox::EscalationRequester>,
    command: &str,
    timeout_secs: u64,
    justification: &str,
    sandbox: &ResolvedSandbox,
) -> Option<Result<String>> {
    let requester = escalation.as_ref()?;
    let request = EscalationRequest {
        trigger: crate::sandbox::EscalationTrigger::ModelRequest,
        command: command.to_string(),
        justification: justification.to_string(),
        failure_summary: String::new(),
    };
    Some(match requester(&request) {
        EscalationDecision::Approved => {
            record_approval(
                true,
                "Approved retry may repeat partial effects within this execution unit.",
            );
            spawn_shell(command, timeout_secs, sandbox, true, None).await
        }
        EscalationDecision::Cancelled(note) => {
            record_approval_end("cancelled", &note);
            Err(anyhow!("Shell approval cancelled: {note}"))
        }
        EscalationDecision::ContextInvalidated(note) => {
            record_approval_end("context_invalidated", &note);
            Err(anyhow!("Shell approval context invalidated: {note}"))
        }
        EscalationDecision::Denied(note) => {
            record_approval(false, &note);
            Err(anyhow!(
            "Escalated execution was not approved{}. Run the command inside the sandbox instead, or explain to the user why it needs these permissions.",
            if note.is_empty() { String::new() } else { format!(": {note}") }
        ))
        }
    })
}

/// Post-hoc escalation: the sandboxed run failed with a sandbox-denial
/// signature. Returns the outcome to propagate (approved → unsandboxed
/// re-run; denied → annotated original output), or None when the failure
/// doesn't look like a sandbox denial / no escalation channel exists.
pub(crate) async fn post_hoc_escalation(
    escalation: &Option<crate::sandbox::EscalationRequester>,
    sandbox: &ResolvedSandbox,
    command: &str,
    timeout_secs: u64,
    result: &str,
    retry: ShellRetry,
) -> Option<Result<String>> {
    let requester = escalation.as_ref()?;
    let fact = latest_attempt();
    #[cfg(test)]
    let fact = fact.or_else(|| {
        Some(ShellAttempt {
            status: ShellStatus::Exited,
            exit_code: shell_result_exit_code(result),
            output: result.into(),
            duration_ms: 0,
            output_truncated: false,
            escalated: false,
        })
    });
    let fact = fact?;
    let exit_code = fact.exit_code?;
    let tail = bounded(&fact.output, 2000);
    if cancelled()
        || duration(timeout_secs).is_zero()
        || fact.status != ShellStatus::Exited
        || retry == ShellRetry::Blocked
        || exit_code == 0
        || !crate::sandbox::looks_like_sandbox_denial(sandbox, exit_code, &tail)
    {
        return None;
    }
    let request = EscalationRequest {
        trigger: crate::sandbox::EscalationTrigger::SandboxFailure,
        command: command.to_string(),
        justification: String::new(),
        failure_summary: bounded(&tail, 2000),
    };
    Some(match requester(&request) {
        EscalationDecision::Approved => {
            record_approval(
                true,
                "Approved retry may repeat partial effects within this execution unit.",
            );
            spawn_shell(command, timeout_secs, sandbox, true, None).await
        }
        EscalationDecision::Cancelled(note) => {
            record_approval_end("cancelled", &note);
            Err(anyhow!("Shell approval cancelled: {note}"))
        }
        EscalationDecision::ContextInvalidated(note) => {
            record_approval_end("context_invalidated", &note);
            Err(anyhow!("Shell approval context invalidated: {note}"))
        }
        EscalationDecision::Denied(note) => {
            record_approval(false, &note);
            Ok(format!(
            "{result}\n[sandbox] The command appears to have been blocked by the sandbox; running it without the sandbox was not approved{}.",
            if note.is_empty() { String::new() } else { format!(": {note}") }
        ))
        }
    })
}

#[cfg(test)]
pub(crate) async fn run_shell(
    command: &str,
    timeout_secs: u64,
    escalated: bool,
    justification: &str,
) -> Result<String> {
    FACTS
        .scope(
            RefCell::new(ExecutionFacts::default()),
            run_shell_with_capability(command, timeout_secs, escalated, justification, None),
        )
        .await
}

pub(crate) async fn run_shell_with_capability(
    command: &str,
    timeout_secs: u64,
    escalated: bool,
    justification: &str,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
) -> Result<String> {
    // Defense-in-depth: reject obviously destructive commands before they
    // reach the OS.  The sandbox provides the primary enforcement boundary;
    // this is a loud, fast-fail layer that catches the most egregious patterns.
    reject_dangerous_command(command)?;

    // On Windows, cmd.exe strips double quotes when processing arguments to
    // npm-generated .cmd wrappers (like the `future` CLI). This corrupts
    // --args JSON that contains commas in string values. Rewrite such
    // commands to pipe JSON through --stdin via a temp file.
    let command_owned =
        cmd_exe_rewrite::rewrite_future_tools_args(command).unwrap_or_else(|| command.to_string());
    let command: &str = &command_owned;

    let sandbox = TOOL_SCOPE
        .try_with(|scope| scope.sandbox.clone())
        .unwrap_or_default();
    let escalation = TOOL_SCOPE
        .try_with(|scope| scope.escalation.clone())
        .unwrap_or(None);

    // Model explicitly requested escalated permissions: approve BEFORE running.
    // Only honored when the command would actually run sandboxed — in degraded
    // or full-access modes the pre-execution approval flow already covered it,
    // and escalating would double-prompt the user.
    if escalated && sandbox.wraps_shell() {
        // No escalation channel: fall through to a normal sandboxed run.
        #[allow(clippy::single_match)]
        // match keeps each edge's region on its arm line; an if-let whose body always diverges leaves a phantom zero-count region on its closing brace
        match pre_execution_escalation(&escalation, command, timeout_secs, justification, &sandbox)
            .await
        {
            Some(outcome) => return outcome,
            None => {}
        }
    }

    let sandboxed = sandbox.wraps_shell();
    if sandboxed {
        if let Ok(Some(notify)) = TOOL_SCOPE.try_with(|scope| scope.on_sandboxed.clone()) {
            notify(command);
        }
    }
    let mut retry = ShellRetry::ClassifyOutput;
    let result = spawn_shell_with_report(
        command,
        timeout_secs,
        &sandbox,
        false,
        approved_capability,
        &mut retry,
    )
    .await?;

    // Post-hoc escalation: only when the failure narrowly looks like a sandbox
    // denial (conservative heuristic — ordinary failures go back to the model).
    if sandboxed && retry != ShellRetry::Blocked {
        #[allow(clippy::single_match)]
        // match keeps each edge's region on its arm line; an if-let whose body always diverges leaves a phantom zero-count region on its closing brace
        match post_hoc_escalation(&escalation, &sandbox, command, timeout_secs, &result, retry)
            .await
        {
            Some(outcome) => return outcome,
            None => {}
        }
    }

    Ok(result)
}

/// Extract the exit code and output tail from a formatted run_shell result, for
/// the sandbox-denial heuristic. Exit code is now at the end as "[exit: N]".
#[cfg(test)]
pub(crate) fn parse_result_failure(result: &str) -> (i32, String) {
    let exit_code = shell_result_exit_code(result).unwrap_or(0);
    let mut tail_start = result.len().saturating_sub(2000);
    while !result.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let tail = result[tail_start..].to_string();
    (exit_code, tail)
}

/// Spawn a shell command (sandbox-wrapped unless `escalated`) and wait for it
/// with timeout + interrupt handling. Returns the formatted combined output.
#[cfg(windows)]
pub(crate) async fn spawn_windows_restricted_shell(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    cwd: &Path,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
) -> Result<String> {
    let mut env_overrides = vec![(
        std::ffi::OsString::from("PWD"),
        cwd.as_os_str().to_os_string(),
    )];
    if let Some(path) = path_with_own_dir(std::env::current_exe()) {
        env_overrides.push((std::ffi::OsString::from("PATH"), path.into()));
    }
    let mut child = crate::sandbox::windows::runner::spawn(
        sandbox,
        command,
        cwd,
        &env_overrides,
        approved_capability,
    )
    .map_err(|error| anyhow!("Failed to initialize Windows write protection: {error}"))?;
    mark_started();
    let mut stdout = tokio::fs::File::from_std(
        child
            .take_stdout()
            .ok_or_else(|| anyhow!("Failed to capture restricted stdout"))?,
    );
    let mut stderr = tokio::fs::File::from_std(
        child
            .take_stderr()
            .ok_or_else(|| anyhow!("Failed to capture restricted stderr"))?,
    );
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_chunk = [0; 8192];
    let mut stderr_chunk = [0; 8192];
    let interrupt_flag = TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.clone())
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
    let timeout = duration(timeout_secs);

    enum Completion {
        Exit(std::io::Result<u32>),
        Timeout,
        Interrupted,
    }
    let completion = tokio::select! {
        result = tokio::time::timeout(timeout, async {
            let (out, err, exit) = tokio::join!(
                read_shell_output(&mut stdout, &mut stdout_bytes, &mut stdout_chunk),
                read_shell_output(&mut stderr, &mut stderr_bytes, &mut stderr_chunk),
                child.wait()
            );
            out.map_err(std::io::Error::other)?;
            err.map_err(std::io::Error::other)?;
            exit
        }) => match result {
            Ok(exit) => Completion::Exit(exit),
            Err(_) => Completion::Timeout,
        },
        _ = wait_for_interrupt(interrupt_flag) => Completion::Interrupted,
    };
    if matches!(&completion, Completion::Timeout | Completion::Interrupted) {
        child.terminate();
        let _ = child.wait().await;
    }
    let mut combined = stdout_bytes;
    if !stderr_bytes.is_empty() {
        append_output(&mut combined, b"\n");
        append_output(&mut combined, &stderr_bytes);
    }
    let combined = crate::sandbox::decode_restricted_shell_output(&combined);

    match &completion {
        Completion::Exit(Ok(exit)) => {
            record_process(ShellStatus::Exited, Some(*exit as i32), &combined)
        }
        Completion::Timeout => record_process(ShellStatus::TimedOut, None, &combined),
        Completion::Interrupted => record_process(ShellStatus::Cancelled, None, &combined),
        Completion::Exit(Err(error)) => record_process(
            ShellStatus::ExecutionFailed,
            None,
            &format!("{combined}\n{error}"),
        ),
    }
    match completion {
        Completion::Exit(exit) => {
            let exit = exit.map_err(|error| anyhow!("Restricted shell wait failed: {error}"))?;
            Ok(format_shell_output(&combined, combined.len(), exit as i32))
        }
        Completion::Timeout if combined.is_empty() => Err(anyhow!(
            "Shell command timed out after {} seconds (no output captured)",
            timeout_secs.max(1)
        )),
        Completion::Timeout => Err(anyhow!(
            "Shell command timed out after {} seconds.\nPartial output ({} total):\n{}",
            timeout_secs.max(1),
            human_size(combined.len()),
            format_shell_output(&combined, combined.len(), -1),
        )),
        Completion::Interrupted => Err(anyhow!("Shell command interrupted by abort")),
    }
}

pub(crate) async fn spawn_shell(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    escalated: bool,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
) -> Result<String> {
    spawn_shell_with_report(
        command,
        timeout_secs,
        sandbox,
        escalated,
        approved_capability,
        &mut ShellRetry::ClassifyOutput,
    )
    .await
}

/// Internal evidence, never deserialized from command output or model input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellRetry {
    ClassifyOutput,
    Blocked,
}

pub(crate) async fn spawn_shell_with_report(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    escalated: bool,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
    retry: &mut ShellRetry,
) -> Result<String> {
    let start = Instant::now();
    let (outcome, fact) = STARTED
        .scope(
            RefCell::new(false),
            TRUNCATED.scope(
                RefCell::new(false),
                PROCESS.scope(RefCell::new(None), async {
                    let outcome = if cancelled() {
                        record_process(ShellStatus::Cancelled, None, "Cancelled before execution");
                        Err(anyhow!(
                            "Shell command interrupted by abort before execution"
                        ))
                    } else if duration(timeout_secs).is_zero() {
                        record_process(
                            ShellStatus::TimedOut,
                            None,
                            "Total execution budget exhausted",
                        );
                        Err(anyhow!("Total execution budget exhausted"))
                    } else {
                        // Bound the size of the task-local orchestration
                        // future independently of platform pipe state.
                        Box::pin(spawn_shell_impl(
                            command,
                            timeout_secs,
                            sandbox,
                            escalated,
                            approved_capability,
                            retry,
                        ))
                        .await
                    };
                    let mut fact = PROCESS
                        .with(|slot| slot.borrow_mut().take())
                        .unwrap_or_else(|| ShellAttempt {
                            status: if cancelled() {
                                ShellStatus::Cancelled
                            } else if duration(timeout_secs).is_zero() {
                                ShellStatus::TimedOut
                            } else if STARTED.with(|flag| *flag.borrow()) {
                                ShellStatus::ExecutionFailed
                            } else {
                                ShellStatus::LaunchFailed
                            },
                            exit_code: None,
                            output: bounded(
                                &outcome
                                    .as_ref()
                                    .err()
                                    .map(ToString::to_string)
                                    .unwrap_or_default(),
                                limit(),
                            ),
                            output_truncated: false,
                            duration_ms: 0,
                            escalated,
                        });
                    fact.duration_ms = start.elapsed().as_millis() as u64;
                    fact.escalated = escalated;
                    (outcome, fact)
                }),
            ),
        )
        .await;
    let _ = FACTS.try_with(|facts| facts.borrow_mut().attempts.push(fact));
    outcome
}

async fn spawn_shell_impl(
    command: &str,
    timeout_secs: u64,
    sandbox: &ResolvedSandbox,
    escalated: bool,
    approved_capability: Option<&crate::sandbox::windows_request::ApprovedWriteCapability>,
    retry: &mut ShellRetry,
) -> Result<String> {
    let cwd = active_workspace()?;
    #[cfg(windows)]
    if !escalated && sandbox.wraps_shell() {
        return spawn_windows_restricted_shell(
            command,
            timeout_secs,
            sandbox,
            &cwd,
            approved_capability,
        )
        .await;
    }
    #[cfg(not(windows))]
    let _ = approved_capability;
    // Unix: wrap in a subshell to merge stderr into stdout, preserving the
    // original interleaving order that separate pipes lose. Internal
    // redirections in the user's command are respected inside the subshell;
    // only the subshell's own stderr (empty after the merge) goes to /dev/null.
    #[cfg(not(windows))]
    let merged_cmd = format!("( {} ) 2>&1", command);
    // Windows: `( … ) 2>&1` is a bash-ism — PowerShell's `( … )` rejects
    // multi-statement commands. The PowerShell wrapper built by
    // `sandbox::shell_invocation` does the stderr merge and exit-code capture
    // itself, so the command passes through unmodified.
    #[cfg(windows)]
    let merged_cmd = command.to_string();
    // Preparation can scan a large workspace before a child exists. Let Abort
    // cancel that scan as well as the process execution below.
    let interrupt_flag = TOOL_SCOPE
        .try_with(|scope| scope.interrupt_flag.clone())
        .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
    // Keep directory I/O off the async executor so it can process the Abort
    // request even on a single-worker runtime. The worker only prepares a
    // request: dropping this future can never launch the user command later.
    #[cfg(target_os = "linux")]
    let preparation = {
        let sandbox = sandbox.clone();
        let cwd = cwd.clone();
        let cancelled = interrupt_flag.clone();
        let deadline = Instant::now() + duration(timeout_secs);
        let worker = tokio::task::spawn_blocking(move || {
            sandbox.prepare_shell_for_cwd_with_cancel(&merged_cmd, escalated, &cwd, &|| {
                cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline
            })
        });
        match tokio::time::timeout(duration(timeout_secs), worker).await {
            Ok(outcome) => outcome
                .map_err(|error| anyhow!("Failed to initialize OS sandbox worker: {error}"))?,
            Err(_) => {
                record_process(
                    ShellStatus::TimedOut,
                    None,
                    "Sandbox preparation exceeded execution budget",
                );
                return Err(anyhow!("Sandbox preparation exceeded execution budget"));
            }
        }
    };
    #[cfg(not(target_os = "linux"))]
    let preparation = sandbox.prepare_shell_for_cwd(&merged_cmd, escalated, &cwd);
    let prepared =
        preparation.map_err(|error| anyhow!("Failed to initialize OS sandbox: {error}"))?;
    if interrupt_flag.load(Ordering::Relaxed) {
        return Err(anyhow!(
            "Shell command interrupted by abort before execution"
        ));
    }
    let report_digest = prepared.boundary.policy_digest.clone();
    let expects_report =
        prepared.boundary.backend == crate::sandbox::backend::ShellBackend::LinuxBubblewrap;
    if expects_report {
        *retry = ShellRetry::Blocked;
    }
    let (mut child, mut report_reader) = prepared
        .into_command_with_report()
        .map_err(|error| anyhow!("Failed to initialize OS sandbox request transport: {error}"))?;
    #[cfg(windows)]
    let _ = (&report_digest, &mut report_reader);
    child.current_dir(&cwd).env("PWD", &cwd);
    // Prepend the agent binary's directory to PATH so bundled tools in the
    // same directory are discoverable by shell commands. (map + discard: a
    // lone if-let closing brace here collected a phantom zero-count region.)
    let _ = path_with_own_dir(std::env::current_exe()).map(|path| child.env("PATH", path));
    child.stdout(std::process::Stdio::piped());
    // Plain Unix shells merge in the subshell. Linux sandbox helpers instead
    // dup stderr to stdout before exec (PreparedShell::into_command), preserving
    // initialization errors emitted before that subshell even exists.
    // Windows: PowerShell's own failures (a parse error in the
    // -Command string never executes the 2>&1 merge) surface only on the
    // process's stderr — capture it so those errors aren't silently dropped.
    #[cfg(not(windows))]
    child.stderr(std::process::Stdio::null());
    #[cfg(windows)]
    child.stderr(std::process::Stdio::piped());
    child.kill_on_drop(true);
    // Run the shell as the leader of its own process group so abort/timeout can kill
    // the whole tree. kill_on_drop alone only SIGKILLs the shell itself, leaving
    // grandchildren (e.g. a `sleep` spawned by the command) running as orphans.
    // sandbox-exec execs its child, so the group covers the wrapped tree too.
    #[cfg(unix)]
    child.process_group(0);

    let mut spawned = child
        .spawn()
        .map_err(|e| anyhow!("Failed to run shell command: {}", e))?;
    mark_started();
    #[cfg(unix)]
    let pgid = spawned.id().map(|id| id as i32);
    #[cfg(windows)]
    let job = {
        let job = crate::sandbox::windows::Job::create().ok();
        if let (Some(job), Some(pid)) = (&job, spawned.id()) {
            let _ = job.assign(pid);
        }
        job
    };

    #[cfg(windows)]
    let mut stderr = spawned.stderr.take();
    #[cfg(windows)]
    let mut stderr_output = Vec::new();
    // Read stdout incrementally — on timeout we keep whatever was captured.
    let mut stdout = spawned
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stdout"))?;
    let mut output_buf = Vec::new();
    let mut read_buf = [0u8; 8192];
    let timeout_dur = duration(timeout_secs);

    // On Windows, EOF is not proof of success. Include process exit in the
    // same deadline, and keep kill-on-close armed on every failure path.
    #[cfg(windows)]
    {
        let result = tokio::select! {
            result = tokio::time::timeout(timeout_dur, async {
                let (out, err, status) = tokio::join!(
                    read_shell_output(&mut stdout, &mut output_buf, &mut read_buf),
                    async {
                        if let Some(err) = &mut stderr {
                            let mut chunk = [0; 8192];
                            read_shell_output(err, &mut stderr_output, &mut chunk).await?;
                        }
                        Ok::<(), anyhow::Error>(())
                    },
                    spawned.wait()
                );
                out?; err?;
                status.map_err(|error| anyhow!("Failed to wait for shell: {error}"))
            }) => result,
            _ = wait_for_interrupt(interrupt_flag.clone()) => {
                if let Some(job) = &job {
                    job.terminate();
                }
                if !stderr_output.is_empty() { append_output(&mut output_buf, b"\n"); append_output(&mut output_buf, &stderr_output); }
                record_process(ShellStatus::Cancelled, None, &String::from_utf8_lossy(&output_buf));
                return Err(anyhow!("Shell command interrupted by abort"));
            }
        };

        if !stderr_output.is_empty() {
            append_output(&mut output_buf, b"\n");
            append_output(&mut output_buf, &stderr_output);
        }

        match result {
            Ok(Ok(status)) => {
                if let Some(job) = &job {
                    job.disarm();
                }
                let combined = String::from_utf8_lossy(&output_buf);
                record_process(ShellStatus::Exited, status.code(), &combined);
                Ok(format_shell_output(
                    &combined,
                    combined.len(),
                    status.code().unwrap_or(-1),
                ))
            }
            Ok(Err(e)) => {
                record_process(
                    ShellStatus::ExecutionFailed,
                    None,
                    &format!("{}\n{e}", String::from_utf8_lossy(&output_buf)),
                );
                Err(e)
            }
            Err(_elapsed) => {
                if let Some(job) = &job {
                    job.terminate();
                }
                let _ = spawned.kill().await;
                let combined = String::from_utf8_lossy(&output_buf);
                let combined = if expects_report {
                    std::borrow::Cow::Owned(crate::sandbox::linux::report::untrusted_output(
                        &combined,
                    ))
                } else {
                    combined
                };
                let total = combined.len();
                record_process(ShellStatus::TimedOut, None, &combined);
                if total == 0 {
                    Err(anyhow!(
                        "Shell command timed out after {} seconds (no output captured)",
                        timeout_secs.max(1)
                    ))
                } else {
                    spawned.kill().await.ok();
                    Ok(format_shell_output(&combined, total, -1))
                }
            }
        }
    }

    #[cfg(not(windows))]
    let read_result = tokio::select! {
        result = tokio::time::timeout(timeout_dur, async {
            read_shell_output(&mut stdout, &mut output_buf, &mut read_buf).await?;
            // Channel a wait() failure through the same error edge as read
            // failures so the match below needs no OS-failure-only arm.
            spawned
                .wait()
                .await
                .map_err(|e| anyhow!("Failed to run shell command: {e}"))
        }) => result,
        _ = wait_for_interrupt(interrupt_flag.clone()) => {
            kill_process_group(pgid);
            record_process(ShellStatus::Cancelled, None, &String::from_utf8_lossy(&output_buf));
                return Err(anyhow!("Shell command interrupted by abort"));
        }
    };

    #[cfg(not(windows))]
    {
        // `outcome?` keeps the (injection-proof) read/wait error edge on the
        // same line as the success pattern — no unreachable match arm.
        let status = match read_result {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                kill_process_group(pgid);
                record_process(
                    ShellStatus::ExecutionFailed,
                    None,
                    &format!("{}\n{error}", String::from_utf8_lossy(&output_buf)),
                );
                return Err(error);
            }
            Err(_elapsed) => {
                // Timeout — kill process tree, drain remaining pipe content.
                kill_process_group(pgid);
                // Drain whatever the process wrote before the kill took effect.
                let _ = tokio::time::timeout(
                    Duration::from_millis(200),
                    drain_shell_output(&mut stdout, &mut output_buf, &mut read_buf),
                )
                .await;
                let combined = String::from_utf8_lossy(&output_buf);
                let combined = if expects_report {
                    std::borrow::Cow::Owned(crate::sandbox::linux::report::untrusted_output(
                        &combined,
                    ))
                } else {
                    combined
                };
                let total = combined.len();
                record_process(ShellStatus::TimedOut, None, &combined);
                if total == 0 {
                    return Err(anyhow!(
                        "Shell command timed out after {} seconds (no output captured)",
                        timeout_secs.max(1)
                    ));
                }
                let formatted = format_shell_output(&combined, total, -1);
                return Err(anyhow!(
                    "Shell command timed out after {} seconds.\nPartial output ({} total):\n{}",
                    timeout_secs.max(1),
                    human_size(total),
                    formatted,
                ));
            }
        };
        // Normal completion. On unix a successful command never kills the
        // process group, so intentionally detached grandchildren survive.
        // Drain leftover bytes (rare: process exited but pipe still has data).
        let _ = tokio::time::timeout(
            Duration::from_millis(200),
            drain_shell_output(&mut stdout, &mut output_buf, &mut read_buf),
        )
        .await;
        let combined = String::from_utf8_lossy(&output_buf);
        let exit_code = status.code().unwrap_or(-1);
        record_process(ShellStatus::Exited, status.code(), &combined);
        if expects_report {
            // Never parse command-printed markers as helper evidence. Only the
            // per-spawn anonymous channel may suppress post-hoc escalation.
            let mut output = crate::sandbox::linux::report::untrusted_output(&combined);
            let report = report_reader
                .as_mut()
                .zip(report_digest.as_deref())
                .ok_or_else(|| anyhow!("missing helper report channel"))
                .and_then(|(file, digest)| {
                    crate::sandbox::linux::report::HelperReport::read(file, digest)
                });
            match report {
                Ok(report) => {
                    *retry = if report.events.is_empty() {
                        ShellRetry::ClassifyOutput
                    } else {
                        ShellRetry::Blocked
                    };
                    // Format/truncate command text before appending verified
                    // reports so they cannot be lost to command-output volume.
                    output = format_shell_output(&output, output.len(), exit_code);
                    for event in &report.events {
                        output.push('\n');
                        output.push_str(&crate::sandbox::linux::violation::marker(event));
                    }
                }
                Err(_) => {
                    output = format_shell_output(&output, output.len(), exit_code);
                    output.push_str("\n[sandbox] Helper report unavailable or invalid. Detection results are unknown; automatic unsandboxed retry is disabled. The command's exit status is unchanged.");
                }
            }
            record_process(ShellStatus::Exited, status.code(), &output);
            return Ok(output);
        }
        Ok(format_shell_output(&combined, combined.len(), exit_code))
    }
}

/// Prepend the agent binary's own directory to the inherited PATH, so tools
/// bundled next to the binary are discoverable by shell commands. Returns
/// None when the binary's location can't be determined (no PATH prepend).
pub(crate) fn path_with_own_dir(exe: std::io::Result<std::path::PathBuf>) -> Option<String> {
    let exe = exe.ok()?;
    let dir = exe.parent()?;
    let existing = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    Some(format!("{}{}{}", dir.display(), sep, existing))
}

/// Read the child's stdout into `buf` until EOF; a read error aborts the run.
/// Extracted from spawn_shell so the error arm is directly testable with a
/// failing reader (a real pipe read failure has no reliable injection point).
pub(crate) async fn read_shell_output<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    chunk: &mut [u8],
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    loop {
        match reader.read(chunk).await {
            Ok(0) => return Ok(()),
            Ok(n) => append_output(buf, &chunk[..n]),
            Err(e) => return Err(anyhow!("Failed to read shell output: {}", e)),
        }
    }
}

/// Drain leftover pipe bytes after process exit/kill; EOF or a read error
/// (the kill racing the pipe) both end the drain silently.
#[cfg(not(windows))]
pub(crate) async fn drain_shell_output<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    chunk: &mut [u8],
) {
    use tokio::io::AsyncReadExt;
    loop {
        match reader.read(chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => append_output(buf, &chunk[..n]),
        }
    }
}

/// Drop the CLIXML noise Windows PowerShell serializes onto its stderr when it
/// is a redirected pipe (each block starts with a `#< CLIXML` marker line
/// followed by a `<Objs …>…</Objs>` XML payload). Line-based so it never eats
/// genuine error text. No-op when there is no CLIXML marker.
#[cfg(all(windows, test))]
pub(crate) fn strip_powershell_clixml(text: &str) -> String {
    if !text.contains("#< CLIXML") {
        return text.to_string();
    }
    text.lines()
        .filter(|line| {
            let t = line.trim_start();
            !(t.starts_with("#< CLIXML")
                || (t.starts_with("<Objs") && t.contains("schemas.microsoft.com/powershell")))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format shell output with truncation info and exit code footer.
/// Kept out of the hot path so the timeout branch can reuse it.
pub(crate) fn format_shell_output(raw: &str, total_bytes: usize, exit_code: i32) -> String {
    const MAX_KEEP: usize = 500_000;

    let body = if total_bytes > MAX_KEEP {
        let truncated = total_bytes - MAX_KEEP;
        // Keep the LAST MAX_KEEP bytes (most relevant output is at the end).
        let start = raw.ceil_char_boundary(raw.len().saturating_sub(MAX_KEEP));
        format!(
            "[output: {} total, showing last {}; {} truncated]\n{}",
            human_size(total_bytes),
            human_size(MAX_KEEP),
            human_size(truncated),
            &raw[start..],
        )
    } else if TRUNCATED.try_with(|flag| *flag.borrow()).unwrap_or(false) {
        // Bounded pipe capture has already discarded the head. Keep the
        // legacy truncation notice without inventing a total byte count.
        format!(
            "[output truncated during capture; showing last {}]\n{}",
            human_size(raw.len()),
            raw,
        )
    } else {
        raw.to_string()
    };

    let footer = if exit_code >= 0 {
        format!("[exit: {}]", exit_code)
    } else {
        "[exit: signal]".to_string()
    };

    let result = format!("{}\n{}", body, footer);
    // The footer ("[exit: …]") is always non-empty, so trim_end can never
    // yield an empty string — the untrimmed fallback was dead by construction.
    result.trim_end().to_string()
}

pub(crate) fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
