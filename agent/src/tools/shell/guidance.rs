/// Shell execution rules are injected into the execution section of the
/// default prompt. They describe tool use, not the user's reply format.
pub(in crate::tools) fn guidelines() -> Vec<&'static str> {
    let mut rules = vec![
        "Choose commands to suit the task. Each call starts a fresh shell in the project workspace with the inherited environment; cd, variables and functions do not carry across calls. Keep scripts needing shared shell context in one command.",
        "The tool reports the whole process status, not each operation inside a script. Interpret it with the output and intended effect: exit 0 may hide an earlier failure, and a later query's nonzero exit does not prove an earlier action failed. Printed status text does not replace the tool's process status.",
        "Preserve errors needed to assess required operations. Expected no-match or false-condition results may be normal; tolerating an error does not make that operation successful.",
        "Before retrying, inspect possible partial effects. If replay is unsafe, submit only the remaining necessary work as a new call; do not repeat completed independent calls.",
        "Follow a user-requested script or command-line method using shell. Scripted writes and redirection use the same sandbox and approval checks.",
    ];
    #[cfg(not(target_os = "windows"))]
    rules.push("Prefer write/edit for ordinary file writes; use shell redirection, heredocs, tee, or cat > file only when they are more appropriate for the task.");
    #[cfg(target_os = "windows")]
    rules.push("Prefer write/edit for ordinary file writes; use PowerShell redirection (> or Out-File) only when it is more appropriate for the task. On Windows PowerShell 5.1 these default to UTF-16 with a BOM; pass -Encoding utf8 if another tool must read the file.");
    rules
}
