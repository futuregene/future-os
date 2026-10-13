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
