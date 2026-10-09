use crate::sandbox::diagnostics::{permission_diagnostic, PermissionDiagnostic};
use crate::sandbox::ResolvedSandbox;
use std::path::Path;
pub(super) fn extract_blocked_paths_raw(stderr: &str) -> Vec<String> {
    extract_denial_paths(stderr, true)
}

pub(super) fn extract_denial_paths(stderr: &str, include_linux_diagnostics: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let path = if line.contains("Operation not permitted") {
            quoted_path(line).or_else(|| absolute_path_token(line))
        } else if include_linux_diagnostics {
            // Additional formats improve diagnostics only. Do not broaden the
            // historical input for persistent write-rule suggestions.
            diagnostic_path(line)
        } else {
            None
        };
        let Some(raw) = path else {
            continue;
        };
        if !out.contains(&raw) {
            out.push(raw);
        }
        if out.len() >= 5 {
            break;
        }
    }
    out
}

/// Common coreutils/shell diagnostics: quoted targets (including spaces), or
/// `program: [line N:] /absolute/target: error`. Do not report `/bin/bash`
/// instead of the target, resolve relative names against an assumed cwd, or
/// treat URLs as filesystem paths. Localized/arbitrary program output remains
/// best-effort and may yield no target.
pub(super) fn diagnostic_path(line: &str) -> Option<String> {
    let diagnostic = permission_diagnostic(line)?;
    if matches!(
        diagnostic,
        PermissionDiagnostic::OperationNotPermitted | PermissionDiagnostic::PermissionDenied
    ) {
        if let Some(path) = quoted_path(line).filter(|path| path.starts_with('/')) {
            return Some(path);
        }
    }
    let lower = line.to_ascii_lowercase();
    let prefix = [
        "operation not permitted",
        "permission denied",
        "read-only file system",
        "device or resource busy",
    ]
    .iter()
    .filter_map(|error| lower.find(error))
    .min()
    .map(|index| &line[..index])?;
    for (open, close) in [('\'', '\''), ('"', '"'), ('‘', '’')] {
        let mut rest = prefix;
        while let Some((_, after_open)) = rest.split_once(open) {
            let Some((target, after_close)) = after_open.split_once(close) else {
                break;
            };
            if target.starts_with('/') {
                return Some(target.to_string());
            }
            rest = after_close;
        }
    }
    let target = prefix.trim().strip_suffix(':')?.trim();
    let target = target.rsplit(": ").next()?.trim();
    target.starts_with('/').then(|| target.to_string())
}

/// Shorten `$HOME` to `~` for display.
pub(super) fn shorten_home(path: &str) -> String {
    if let Some(home) = crate::utils::home_dir_opt() {
        if let Ok(rest) = std::path::Path::new(path).strip_prefix(home) {
            // Windows renders the tail with `\`; the shortened form is card
            // text and a saved-rule path, and tilde expansion accepts either
            // separator, so it is normalized to `/` for stable rules.
            #[cfg(windows)]
            let rest = rest.to_string_lossy().replace('\\', "/");
            #[cfg(not(windows))]
            let rest = rest.to_string_lossy().into_owned();
            return if rest.is_empty() {
                "~".to_string()
            } else {
                format!("~/{rest}")
            };
        }
    }
    path.to_string()
}

/// Card-friendly blocked paths (`$HOME` → `~`), for display only.
#[cfg(test)]
pub(super) fn extract_blocked_paths(stderr: &str) -> Vec<String> {
    extract_blocked_paths_raw(stderr)
        .iter()
        .map(|p| shorten_home(p))
        .collect()
}

/// "Allow in this workspace/chat" suggestion for an escalation: the parent
/// directory of the (first) blocked path. Returns `None` when any blocked path
/// is a secret (secrets stay one-time-only) or none is known — so the card
/// shows only "allow once" for those. Access is `write`: reads of non-secrets
/// are open, so a non-secret denial is a write.
pub(super) fn escalation_save_suggestion(
    raw_paths: &[String],
    sandbox: &ResolvedSandbox,
) -> Option<serde_json::Value> {
    if raw_paths.is_empty() {
        return None;
    }
    if raw_paths
        .iter()
        .any(|p| sandbox.is_secret_path(Path::new(p)))
    {
        return None;
    }
    let parent = Path::new(&raw_paths[0]).parent()?;
    let glob = match parent.strip_prefix(&sandbox.workspace) {
        Ok(rel) if rel.as_os_str().is_empty() => "*".to_string(),
        Ok(rel) => format!("{}/*", rel.to_string_lossy()),
        // Outside the workspace: keep it portable with `~` when under home.
        Err(_) => format!("{}/*", shorten_home(&parent.to_string_lossy())),
    };
    Some(serde_json::json!({
        "path": glob,
        "access": "write",
        "action": "allow",
    }))
}

/// First `'…'` or `"…"` span that looks like a path (contains `/`).
pub(super) fn quoted_path(line: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        let mut parts = line.split(quote);
        // parts alternate outside/inside the quote; index 1, 3, … are inside.
        parts.next();
        while let Some(inside) = parts.next() {
            if inside.contains('/') {
                return Some(inside.to_string());
            }
            parts.next(); // skip the following outside span
        }
    }
    None
}

/// First whitespace token that is an absolute path, trimming trailing `:`/`,`.
pub(super) fn absolute_path_token(line: &str) -> Option<String> {
    line.split_whitespace()
        .map(|tok| tok.trim_end_matches([':', ',']))
        .find(|tok| tok.starts_with('/') && tok.len() > 1)
        .map(str::to_string)
}
