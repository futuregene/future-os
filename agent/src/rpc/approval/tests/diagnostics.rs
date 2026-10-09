use super::*;

#[test]
fn node_permission_paths_are_diagnostic_only() {
    for error in [
        "Error: EPERM: operation not permitted, open '/Users/example/Desktop/2.txt'",
        "Error: EPERM: 本地化错误, unlink '/Users/example/Desktop/2.txt'",
        "Error: EACCES: permission denied, open '/Users/example/Desktop/2.txt'",
    ] {
        assert_eq!(
            extract_blocked_paths_raw(error),
            vec!["/Users/example/Desktop/2.txt"]
        );
        assert!(extract_denial_paths(error, false).is_empty());
    }
    assert!(
        extract_blocked_paths_raw("Error: EPERM: operation not permitted, open '../file'")
            .is_empty()
    );
}
#[test]
fn extract_blocked_paths_from_gpg_denial() {
    let stderr = "\
[exit code: 128]
error: gpg failed to sign the data:
gpg: failed to create temporary file '/Users/x/.gnupg/.#lk0x001.host.9334': Operation not permitted
gpg: 密钥区块资源 '/Users/x/.gnupg/pubring.kbx': Operation not permitted
[GNUPG:] ERROR add_keyblock_resource 33587307";
    let paths = extract_blocked_paths(stderr);
    assert_eq!(
        paths,
        vec![
            "/Users/x/.gnupg/.#lk0x001.host.9334".to_string(),
            "/Users/x/.gnupg/pubring.kbx".to_string(),
        ]
    );
}

#[test]
fn extract_blocked_paths_unquoted_and_deduped() {
    let stderr = "touch: /etc/hosts: Operation not permitted\n\
                      touch: /etc/hosts: Operation not permitted";
    assert_eq!(
        extract_blocked_paths(stderr),
        vec!["/etc/hosts".to_string()]
    );
    // Non-denial lines are ignored.
    assert!(extract_blocked_paths("error[E0308]: mismatched types").is_empty());
}

#[test]
fn extract_blocked_paths_linux_diagnostics() {
    let stderr = "cat: /home/ace/.aws/config: Permission denied\n\
            touch: cannot touch '/etc/test file': Read-only file system\n\
            mkdir: cannot create directory ‘/opt/new dir’: Permission denied\n\
            /bin/bash: line 1: /etc/shell target: Read-only file system\n\
            cat: \"/home/ace/.ssh/config\": Permission denied\n\
            cat: /home/ace/.aws/config: Permission denied";
    assert_eq!(
        extract_blocked_paths_raw(stderr),
        vec![
            "/home/ace/.aws/config",
            "/etc/test file",
            "/opt/new dir",
            "/etc/shell target",
            "/home/ace/.ssh/config",
        ]
    );
    // Display improvements do not broaden persistent write-rule suggestions.
    assert!(extract_denial_paths(stderr, false).is_empty());
}

#[test]
fn extract_blocked_paths_linux_ignores_unknown_targets_and_other_failures() {
    for line in [
            "cat: config: Permission denied",
            "cat: '../config': Permission denied",
            "curl: https://example.com/private: Permission denied",
            "sandbox: Permission denied",
            "cat: /tmp/missing: No such file or directory",
            "future-linux-sandbox-helper: mount source is unavailable: /home/ace/.aws: No such file or directory (os error 2)",
        ] {
            assert!(extract_blocked_paths_raw(line).is_empty(), "{line}");
        }
}

#[test]
fn extract_blocked_paths_mixed_errors_deduplicate_and_preserve_suggestion_scope() {
    let stderr = "touch: /etc/one: Operation not permitted\n\
            touch: /etc/one: Read-only file system\n\
            cat: /etc/two: Permission denied\n\
            touch: /etc/three: Operation not permitted";
    assert_eq!(
        extract_blocked_paths_raw(stderr),
        vec!["/etc/one", "/etc/two", "/etc/three"]
    );
    assert_eq!(
        extract_denial_paths(stderr, false),
        vec!["/etc/one", "/etc/three"]
    );
}

#[test]
fn extract_blocked_paths_raw_quoted() {
    let stderr = "touch: /etc/test.txt: Operation not permitted";
    let paths = extract_blocked_paths_raw(stderr);
    assert_eq!(paths, vec!["/etc/test.txt"]);
}

#[test]
fn extract_blocked_paths_raw_multiple() {
    let stderr =
        "touch: /etc/a.txt: Operation not permitted\ntouch: /etc/b.txt: Operation not permitted";
    let paths = extract_blocked_paths_raw(stderr);
    assert_eq!(paths.len(), 2);
}

#[test]
fn extract_blocked_paths_raw_no_match() {
    let stderr = "some other error";
    assert!(extract_blocked_paths_raw(stderr).is_empty());
}

#[test]
fn shorten_home_replaces_home() {
    let _home_guard = crate::test_support::home_env_lock();
    let home = dirs::home_dir().unwrap().to_string_lossy().to_string();
    let path = format!("{}/some/file.txt", home);
    assert_eq!(shorten_home(&path), "~/some/file.txt");
}

#[test]
fn shorten_home_outside_home() {
    let home = crate::utils::home_dir();
    let sibling = format!("{}-sibling/file", home.display());
    assert_eq!(shorten_home(&sibling), sibling);
    assert_eq!(shorten_home("/etc/hosts"), "/etc/hosts");
}

// ─── quoted_path ───────────────────────────────────────────────────────

#[test]
fn quoted_path_finds_path() {
    assert_eq!(
        quoted_path("touch: '/etc/test.txt': Operation not permitted"),
        Some("/etc/test.txt".to_string())
    );
}

#[test]
fn quoted_path_no_path() {
    assert_eq!(quoted_path("no path here"), None);
}

// ─── absolute_path_token ───────────────────────────────────────────────

#[test]
fn absolute_path_token_finds() {
    assert_eq!(
        absolute_path_token("touch: /etc/test.txt: Operation not permitted"),
        Some("/etc/test.txt".to_string())
    );
}

#[test]
fn absolute_path_token_trims_colon() {
    assert_eq!(
        absolute_path_token("error: /path/to/file: something"),
        Some("/path/to/file".to_string())
    );
}

#[test]
fn absolute_path_token_none() {
    assert_eq!(absolute_path_token("no paths here"), None);
}

// ─── argument_write_preview ────────────────────────────────────────────

#[test]
fn escalation_save_suggestion_workspace_parent() {
    let ws = temp_ws("esc-save");
    let sandbox = enabled(&ws);
    let blocked = format!("{}/subdir/file.txt", ws);
    let suggestion = escalation_save_suggestion(&[blocked], &sandbox);
    assert!(suggestion.is_some());
    let s = suggestion.unwrap();
    assert_eq!(s["access"], "write");
    assert_eq!(s["action"], "allow");
}

#[test]
fn escalation_save_suggestion_empty_paths() {
    let ws = temp_ws("esc-empty");
    let sandbox = enabled(&ws);
    assert!(escalation_save_suggestion(&[], &sandbox).is_none());
}

#[test]
fn escalation_save_suggestion_secret_returns_none() {
    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("esc-secret");
    let sandbox = enabled(&ws);
    let home = dirs::home_dir().unwrap().to_string_lossy().to_string();
    let secret_path = format!("{home}/.ssh/id_rsa");
    assert!(escalation_save_suggestion(&[secret_path], &sandbox).is_none());
}

// ─── path_save_suggestion ──────────────────────────────────────────────

#[test]
fn extract_blocked_paths_skips_pathless_lines_and_caps_at_five() {
    // A denial line with no quoted/absolute token is skipped…
    let mut stderr = String::from("sandbox: deny(1) file-write: Operation not permitted\n");
    // …and the extractor stops after five paths.
    for i in 0..7 {
        stderr.push_str(&format!(
            "gpg: cannot open '/dev/null/path{i}': Operation not permitted\n"
        ));
    }
    let paths = extract_blocked_paths(&stderr);
    assert_eq!(paths.len(), 5);
    assert_eq!(paths[0], "/dev/null/path0");
    assert_eq!(paths[4], "/dev/null/path4");
}

#[test]
fn escalation_save_suggestion_workspace_root_and_subdir() {
    let ws = temp_ws("esc-glob");
    let sandbox = enabled(&ws);
    // Blocked path directly at the workspace root → bare "*". Paths are
    // canonicalized to match the resolved (symlink-free) workspace.
    let root_file = crate::sandbox::paths::canonicalize_lenient(&Path::new(&ws).join("note.txt"));
    let sug = escalation_save_suggestion(&[root_file.to_string_lossy().into_owned()], &sandbox)
        .expect("workspace-root path is persistable");
    assert_eq!(sug["path"], "*");
    // Blocked path in a workspace subdir → "sub/*".
    let sub_file =
        crate::sandbox::paths::canonicalize_lenient(&Path::new(&ws).join("build/out.bin"));
    let sug = escalation_save_suggestion(&[sub_file.to_string_lossy().into_owned()], &sandbox)
        .expect("workspace subdir path is persistable");
    assert_eq!(sug["path"], "build/*");
}

#[test]
fn quoted_path_skips_non_path_spans() {
    // First quoted span has no '/', so the scan skips ahead (and its
    // following outside span) to the next quoted span.
    assert_eq!(
        quoted_path("cmd 'not a path' then '/real/path'"),
        Some("/real/path".to_string())
    );
}
