/// Shell execution rules are injected into the execution section of the
/// default prompt. They describe tool use, not the user's reply format.
pub(in crate::tools) fn guidelines() -> Vec<&'static str> {
    let mut rules = vec![
        "Choose commands to suit the task. Each call starts a fresh shell in the project workspace with the inherited environment; cd, variables and functions do not carry across calls. Keep scripts needing shared shell context in one command.",
        "The tool reports the whole process status, not each operation inside a script. Interpret it with the output and intended effect: exit 0 may hide an earlier failure, and a later query's nonzero exit does not prove an earlier action failed. Printed status text does not replace the tool's process status.",
        "Preserve errors needed to assess required operations. Expected no-match or false-condition results may be normal; tolerating an error does not make that operation successful.",
        "Sandbox failures return directly; the host never automatically replays a command or asks to replay it. Inspect existing files and other partial effects before recovery. Submit only the remaining necessary work as a new call. If completion is unknown, verify it first; approval does not establish safe replay.",
        "Follow a user-requested script or command-line method using shell. Scripted writes and redirection use the same sandbox and approval checks.",
    ];
    // Waiting and backgrounding: rules the model cannot infer from the schema
    // (each call is a fresh shell with no job handle, and an unredirected
    // stream blocks the call). Order: liveness, background, bound.
    #[cfg(not(target_os = "windows"))]
    rules.push("Wait on the task's liveness, not on the clock: `pid=<pid>; until ! kill -0 \"$pid\" 2>/dev/null; do sleep 5; done`, or `until ! pgrep -f \"<name>\" >/dev/null; do sleep 5; done`. Keep each sleep ~10 s or less and let the command return as soon as the condition holds — never `sleep 600; tail -3 log`.");
    #[cfg(target_os = "windows")]
    rules.push("Wait on the task's liveness, not on the clock: start it with Start-Process -RedirectStandardOutput out.log -RedirectStandardError err.log -PassThru, write $p.Id to a file, then poll Get-Process -Id <id> -ErrorAction SilentlyContinue until it fails. Keep each Start-Sleep ~10 s or less and let the command return the moment the condition holds; never name the id $pid, which PowerShell reserves.");
    #[cfg(not(target_os = "windows"))]
    rules.push("To start work in the background, redirect both streams: `cmd > log 2>&1 &` returns at once, while `> log &` leaves stderr on the pipe and blocks this whole call until the child exits. Record the pid (echo $! > run.pid) and check it later with kill -0 — the next call is a fresh shell, so $! is gone.");
    #[cfg(target_os = "windows")]
    rules.push("To start work in the background, redirect both streams: an unredirected stream makes the call block until the child exits. Record the process id in a file and check it later with Get-Process -Id — the next call is a fresh shell, so the id has to come from that file.");
    rules.push("Bound every wait: cap the number of probes and print how long you have waited so far (`echo \"waited ${secs}s\"`) instead of re-sleeping because a progress counter is still rising.");
    #[cfg(not(target_os = "windows"))]
    rules.push("With an active macOS/Linux sandbox, declare known extra filesystem access in shell.additional_permissions before execution: read/write arrays of {path, scope, reason}, up to 8 literal targets total. file is one existing regular file; subtree is an existing directory and descendants. For a new file request its existing parent directory. Example: additional_permissions: {read: [{path: '/path/config.env', scope: 'file', reason: 'Read the configuration required by this task'}]}. The application validates and approves the actual command and targets, then executes once with temporary permissions inside the sandbox. This is not a session allowance and cannot override deny rules. Do not combine with escalated. Linux may reject write-without-read grants or a child grant inside an unreadable directory because its mount backend cannot enforce them; use an approved native file tool for the specific file. Do not broaden permissions silently.");
    #[cfg(target_os = "windows")]
    rules.push("With active Windows write protection, declare extra writes before execution using shell.additional_permissions.write: an array of {path, scope, reason}, at most 8 targets. file is an existing regular file; subtree is an existing directory and descendants. For a new file request its existing parent directory. There is no shell read-permission grant on this backend; use read for protected reads. Explicit ask carveouts may be impossible to reopen under Windows ACLs; use an approved write/edit tool instead. Never treat approval as permission to drop write protection or replay earlier work.");
    #[cfg(not(target_os = "windows"))]
    rules.push("Prefer write/edit for ordinary file writes; use shell redirection, heredocs, tee, or cat > file only when they are more appropriate for the task.");
    #[cfg(target_os = "windows")]
    rules.push("Prefer write/edit for ordinary file writes; use PowerShell redirection (> or Out-File) only when it is more appropriate for the task. On Windows PowerShell 5.1 these default to UTF-16 with a BOM; pass -Encoding utf8 if another tool must read the file.");
    rules
}
