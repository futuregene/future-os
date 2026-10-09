use super::*;
use crate::sandbox::windows_request::PreparedWritePermissions;
#[test]
fn windows_capability_shape_uses_behavior_and_target_not_model_reason() {
    use crate::sandbox::windows_request::{
        ApprovalTarget, CapabilityApprovalSemantics, FrozenWriteTarget, WriteScope,
    };

    let ws = temp_ws("windows-capability-shape");
    let sandbox = enabled(&ws);
    let target = Path::new(&ws).join("release");
    std::fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let prepared = PreparedWritePermissions {
        command_hash: "hash-is-internal".to_string(),
        targets: vec![FrozenWriteTarget {
            normalized_path: target.clone(),
            scope: WriteScope::Subtree,
            untrusted_reason: "MODEL CONTROLLED TITLE".to_string(),
            decision: Decision::Ask,
        }],
        approval: Some(CapabilityApprovalSemantics {
            behavior: "manage_files",
            targets: vec![ApprovalTarget {
                path: target.to_string_lossy().into_owned(),
                scope: WriteScope::Subtree,
            }],
        }),
    };

    let shape = windows_capability_shape("build-release", &prepared, &sandbox);
    assert_eq!(shape.kind, "windows_write_capability");
    assert!(shape.title.contains(&target.to_string_lossy().to_string()));
    assert!(!shape.title.contains("MODEL CONTROLLED"));
    assert_eq!(shape.action["behavior"], "manage_files");
    assert_eq!(shape.action["targets"].as_array().unwrap().len(), 1);
    assert_eq!(shape.action["command"], "build-release");
    let action = shape.action.to_string();
    assert!(!action.contains("hash-is-internal"));
    assert!(!action.contains("MODEL CONTROLLED"));
    let suggestion = shape.save_suggestion.unwrap();
    assert_eq!(suggestion["rules"].as_array().unwrap().len(), 1);
    assert_eq!(
        suggestion["rules"][0]["path"],
        target.to_string_lossy().as_ref()
    );
}

#[test]
fn windows_capability_shape_file_scope_title() {
    use crate::sandbox::windows_request::{
        ApprovalTarget, CapabilityApprovalSemantics, FrozenWriteTarget, WriteScope,
    };

    let ws = temp_ws("windows-capability-file");
    let sandbox = enabled(&ws);
    let file = Path::new(&ws).join("notes.txt");
    std::fs::write(&file, "x").unwrap();
    let file = file.canonicalize().unwrap();
    let prepared = PreparedWritePermissions {
        command_hash: "hash".to_string(),
        targets: vec![FrozenWriteTarget {
            normalized_path: file.clone(),
            scope: WriteScope::File,
            untrusted_reason: "unused".to_string(),
            decision: Decision::Ask,
        }],
        approval: Some(CapabilityApprovalSemantics {
            behavior: "modify_file",
            targets: vec![ApprovalTarget {
                path: file.to_string_lossy().into_owned(),
                scope: WriteScope::File,
            }],
        }),
    };

    let shape = windows_capability_shape("edit-notes", &prepared, &sandbox);
    // File scope uses the "modify" verb (not "manage files in").
    assert!(shape.title.contains("modify"), "title: {}", shape.title);
    assert_eq!(
        shape.summary,
        "FutureOS needs write access to this location for the current command."
    );
    assert!(shape.save_suggestion.is_some());
}

#[test]
fn windows_capability_shape_multi_target_title_and_summary() {
    use crate::sandbox::windows_request::{
        ApprovalTarget, CapabilityApprovalSemantics, FrozenWriteTarget, WriteScope,
    };

    let ws = temp_ws("windows-capability-multi");
    let sandbox = enabled(&ws);
    let a = Path::new(&ws).join("release");
    let b = Path::new(&ws).join("cache");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let a = a.canonicalize().unwrap();
    let b = b.canonicalize().unwrap();
    let approval_targets = vec![
        ApprovalTarget {
            path: a.to_string_lossy().into_owned(),
            scope: WriteScope::Subtree,
        },
        ApprovalTarget {
            path: b.to_string_lossy().into_owned(),
            scope: WriteScope::Subtree,
        },
    ];
    let prepared = PreparedWritePermissions {
        command_hash: "hash".to_string(),
        targets: approval_targets
            .iter()
            .map(|t| FrozenWriteTarget {
                normalized_path: Path::new(&t.path).to_path_buf(),
                scope: t.scope,
                untrusted_reason: "unused".to_string(),
                decision: Decision::Ask,
            })
            .collect(),
        approval: Some(CapabilityApprovalSemantics {
            behavior: "manage_files",
            targets: approval_targets,
        }),
    };

    let shape = windows_capability_shape("build-release", &prepared, &sandbox);
    assert!(
        shape.title.contains("2 locations"),
        "title: {}",
        shape.title
    );
    assert_eq!(
        shape.summary,
        "FutureOS needs write access to all of these locations for the current command."
    );
}

#[test]
fn windows_capability_shape_secret_target_suppresses_suggestion() {
    use crate::sandbox::windows_request::{
        ApprovalTarget, CapabilityApprovalSemantics, FrozenWriteTarget, WriteScope,
    };

    let _home_guard = crate::test_support::home_env_lock();
    let ws = temp_ws("windows-capability-secret");
    let sandbox = enabled(&ws);
    // A secret path (~/.ssh) must suppress the persistable-rule suggestion.
    let secret = dirs::home_dir().unwrap().join(".ssh/id_rsa");
    let prepared = PreparedWritePermissions {
        command_hash: "hash".to_string(),
        targets: vec![FrozenWriteTarget {
            normalized_path: secret.clone(),
            scope: WriteScope::File,
            untrusted_reason: "unused".to_string(),
            decision: Decision::Ask,
        }],
        approval: Some(CapabilityApprovalSemantics {
            behavior: "modify_file",
            targets: vec![ApprovalTarget {
                path: secret.to_string_lossy().into_owned(),
                scope: WriteScope::File,
            }],
        }),
    };

    let shape = windows_capability_shape("touch-secret", &prepared, &sandbox);
    assert!(shape.save_suggestion.is_none());
}
