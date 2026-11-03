//! Handler tests for the sessionless skill-management commands.
//!
//! `skills::handle` is the only RPC entry point that talks to the host-local
//! `SkillManager` (`~/.agents/skills` + `<future home>/agent/skills`), so the
//! tests redirect `$HOME` through [`crate::test_support::TestHome`] and drive
//! the real dispatcher: the assertions are about what lands in (and leaves)
//! the user's skill directories, not about the handler's return shape alone.

use crate::rpc::handle_command_internal;

use super::test_support::*;

/// Drop a hand-placed skill directory (no receipt, i.e. "external") whose
/// frontmatter carries the id and version the manager records.
fn write_skill(root: &std::path::Path, id: &str, version: &str) -> std::path::PathBuf {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {id}\nversion: {version}\ndescription: a {id} skill\n---\n\nBody.\n"),
    )
    .unwrap();
    dir
}

fn installed_rows(state: &crate::rpc::AppState) -> Vec<serde_json::Value> {
    let response = parse_response(&handle_command_internal(
        state,
        make_cmd("list_installed_skills"),
    ));
    assert_eq!(
        response["success"], true,
        "list_installed_skills failed: {response}"
    );
    response["data"].as_array().unwrap().clone()
}

/// Same-directory check that survives the `\\?\` verbatim spelling Windows
/// canonicalization produces.
fn same_dir(left: &str, right: &std::path::Path) -> bool {
    crate::sandbox::paths::canonicalize_lenient(std::path::Path::new(left))
        == crate::sandbox::paths::canonicalize_lenient(right)
}

#[test]
fn list_installed_skills_reports_both_scopes_from_disk() {
    let home = crate::test_support::TestHome::new();
    let state = make_app_state();

    // Directories placed by hand (the v3 → v4 upgrade path) are adopted on the
    // first read, so they must exist before the first list call.
    let global = write_skill(&home.path().join(".agents/skills"), "demo", "1.0.0");
    let app = write_skill(
        &home.path().join(".future/agent/skills"),
        "local-only",
        "2.0.0",
    );

    let rows = installed_rows(&state);
    assert_eq!(rows.len(), 2, "both scopes must be reported: {rows:?}");
    let by_id: std::collections::HashMap<&str, &serde_json::Value> = rows
        .iter()
        .map(|row| (row["id"].as_str().unwrap(), row))
        .collect();

    // Hand-placed directories are `external` for the shared scope and `app`
    // for the FutureOS-owned one; the version comes from frontmatter.
    assert_eq!(by_id["demo"]["scope"], "global");
    assert_eq!(by_id["demo"]["source"], "external");
    assert_eq!(by_id["demo"]["version"], "1.0.0");
    assert!(same_dir(
        by_id["demo"]["location"].as_str().unwrap(),
        &global
    ));
    assert_eq!(by_id["local-only"]["scope"], "app");
    assert_eq!(by_id["local-only"]["version"], "2.0.0");
    assert!(same_dir(
        by_id["local-only"]["location"].as_str().unwrap(),
        &app
    ));

    // A directory without SKILL.md is not an installed skill.
    std::fs::create_dir_all(home.path().join(".agents/skills/not-a-skill")).unwrap();
    assert_eq!(installed_rows(&state).len(), 2);
}

#[test]
fn sync_skills_without_auto_upgrade_reports_a_noop_and_refreshes_discovery() {
    let home = crate::test_support::TestHome::new();
    let state = make_app_state();
    write_skill(&home.path().join(".agents/skills"), "demo", "1.0.0");
    assert!(state.welcome_skills.read().is_empty());

    // `enabled` is the auto-upgrade flag; false means "do not talk to the
    // platform", which is what makes this path deterministic offline.
    let response = parse_response(&handle_command_internal(&state, make_cmd("sync_skills")));
    assert_eq!(response["success"], true, "{response}");
    for field in ["installed", "upgraded", "skipped", "failed"] {
        assert_eq!(
            response["data"][field],
            serde_json::json!([]),
            "a disabled sync must not touch anything: {response}"
        );
    }
    // ...but it still republishes discovery so the client's next get_state
    // shows the skill that is on disk.
    assert_eq!(*state.welcome_skills.read(), vec!["demo".to_string()]);
}

#[test]
fn uninstall_skill_removes_the_directory_and_is_idempotent() {
    let home = crate::test_support::TestHome::new();
    let state = make_app_state();
    let global = write_skill(&home.path().join(".agents/skills"), "demo", "1.0.0");
    assert_eq!(installed_rows(&state).len(), 1);

    let mut cmd = make_cmd("uninstall_skill");
    cmd.skill_id = "demo".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(response["data"]["removed"], true);
    assert!(
        !global.exists(),
        "uninstall must delete the skill directory"
    );
    assert!(installed_rows(&state).is_empty());

    // Uninstalling something that is not installed is a success with
    // `removed: false` — deleting a name twice must not be an error, and must
    // not resurrect a row.
    let mut cmd = make_cmd("uninstall_skill");
    cmd.skill_id = "demo".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(response["data"]["removed"], false);
    assert!(installed_rows(&state).is_empty());
}

/// The install command's *success* path end to end: the platform serves a real
/// package, the handler installs it, and the result is visible in the installed
/// list. The two existing install tests are failure paths (a rejected id and an
/// unreachable platform), so nothing proved the handler's success arm — or that
/// a successful install republishes discovery for the client's next get_state.
#[test]
fn install_skill_publishes_the_installed_skill_after_a_platform_download() {
    use std::io::{Read, Write};

    let home = crate::test_support::TestHome::new();

    // A minimal valid package: one SKILL.md whose frontmatter matches the
    // requested id and version (the manager verifies both).
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut cursor);
        archive
            .start_file("SKILL.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive
            .write_all(b"---\nname: demo\nversion: 1.0.0\n---\n\n# Demo\n")
            .unwrap();
        archive.finish().unwrap();
    }
    let bytes = cursor.into_inner();

    crate::test_support::ensure_fd_headroom();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let count = stream.read(&mut request).unwrap();
            let is_catalogue =
                String::from_utf8_lossy(&request[..count]).contains("/client/v1/skills ");
            let body = if is_catalogue {
                b"[]".to_vec()
            } else {
                bytes.clone()
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    std::fs::create_dir_all(home.path().join(".future/agent")).unwrap();
    std::fs::write(
        home.auth_path(),
        format!(r#"{{"future":{{"base_url":"{address}"}}}}"#),
    )
    .unwrap();
    let state = make_app_state();

    let mut cmd = make_cmd("install_skill");
    cmd.skill_id = "demo".into();
    cmd.skill_version = "1.0.0".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(
        response["success"], true,
        "install must succeed: {response}"
    );
    assert_eq!(response["data"], serde_json::json!({}));

    // The skill is on disk, recorded as managed, and republished for the
    // client's next get_state.
    let installed = home.path().join(".future/agent/skills/demo");
    assert!(
        installed.join("SKILL.md").exists(),
        "the package must be extracted into the installed skills directory"
    );
    let rows = installed_rows(&state);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], "demo");
    assert_eq!(rows[0]["version"], "1.0.0");
    assert_eq!(
        *state.welcome_skills.read(),
        vec!["demo".to_string()],
        "a successful install must republish discovery"
    );
}

#[test]
fn invalid_skill_identifiers_are_rejected_before_any_state_changes() {
    let home = crate::test_support::TestHome::new();
    let state = make_app_state();

    // Every one of these would otherwise be interpolated into a filesystem
    // path or a platform URL by the manager.
    for id in ["", "..", "a/b", "a\\b", "with space", ".hidden."] {
        for cmd_type in ["install_skill", "uninstall_skill"] {
            let mut cmd = make_cmd(cmd_type);
            cmd.skill_id = id.to_string();
            cmd.skill_version = "1.0.0".to_string();
            let response = parse_response(&handle_command_internal(&state, cmd));
            assert_eq!(
                response["success"], false,
                "{cmd_type}({id:?}) must be rejected: {response}"
            );
            assert!(
                response["error"]
                    .as_str()
                    .unwrap_or("")
                    .contains("invalid skill id or version"),
                "{cmd_type}({id:?}) must say why: {response}"
            );
        }
    }
    // A 129-byte id is over the documented 128-byte bound.
    let mut cmd = make_cmd("install_skill");
    cmd.skill_id = "x".repeat(129);
    cmd.skill_version = "1.0.0".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");

    // Nothing the rejected calls touched exists, and nothing was recorded.
    let app_dir = home.path().join(".future/agent/skills");
    let leftovers: Vec<std::ffi::OsString> = if app_dir.exists() {
        std::fs::read_dir(&app_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect()
    } else {
        Vec::new()
    };
    assert!(
        leftovers.is_empty(),
        "a rejected install left {leftovers:?} behind"
    );
    assert!(installed_rows(&state).is_empty());
}

#[test]
fn platform_backed_skill_commands_fail_closed_when_the_platform_is_unreachable() {
    let home = crate::test_support::TestHome::new();
    // Pin the platform base URL at a closed local port: a deterministic
    // "unreachable platform" instead of the real network.
    std::fs::create_dir_all(home.path().join(".future/agent")).unwrap();
    std::fs::write(
        home.auth_path(),
        r#"{"future":{"base_url":"http://127.0.0.1:9"}}"#,
    )
    .unwrap();
    let state = make_app_state();

    // The catalogue is served by the platform; when it cannot be reached the
    // command reports the failure instead of pretending to be empty.
    let response = parse_response(&handle_command_internal(
        &state,
        make_cmd("list_available_skills"),
    ));
    assert_eq!(response["success"], false, "{response}");

    let mut cmd = make_cmd("install_skill");
    cmd.skill_id = "demo".into();
    cmd.skill_version = "1.0.0".into();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], false, "{response}");
    assert!(!response["error"].as_str().unwrap().is_empty());
    // A failed download must not leave a half-installed skill behind...
    assert!(!home.path().join(".future/agent/skills/demo").exists());
    // ...nor claim one in the installation list.
    assert!(installed_rows(&state).is_empty());
}
