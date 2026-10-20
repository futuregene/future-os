//! `future remote devices add` — a terminal asking the running Desktop for a
//! pairing invitation.
//!
//! A device can only be *added* by the process that will serve it: the invitation
//! carries a single-use secret and the pair-scoped credential, and the bridge for
//! that pairing has to be up when the phone scans. That process is whichever
//! Desktop owns this data directory — the window, or `futureos-headless` — so the
//! request travels to it as a file and the answer comes back as one.
//!
//! This is the same one-shot hand-off the launcher's old `--re-pair` performed
//! through the single-instance plugin, with two differences that are the reason
//! it moved here: it works for a headless Desktop too (which has no plugin), and
//! it adds one device instead of revoking every device and restarting.
//!
//! The files are a request/reply pair in `~/.future/run/remote-control/`:
//!
//! * `*.request.json` — `{"op": "add-device", "reply": "<absolute path>"}`;
//! * the reply goes to the path the *asker* named, which it created for itself as
//!   a temporary file — so two terminals asking at once cannot read each other's
//!   invitation, and a process that exits takes its answer path with it.
//!
//! A request is claimed by renaming it: exactly one Desktop can act on it, and a
//! request nobody could still be waiting for (older than [`ABANDONED_AFTER`]) is
//! deleted instead of being answered into a file whose reader is gone.
//!
//! # Trust boundary
//!
//! Anything running as this user can write a request, exactly as it can already
//! edit the pairing book and read the NKey inside it. So the channel grants no
//! power that the file system does not: it is a smaller interface to the same
//! authority, and the directory is created `0700`-under-the-user's data root only
//! so that other users cannot reach it at all.
//!
//! What this deliberately is *not* is a general control API. Removing a device and
//! choosing the controller are changes to the pairing book alone, which is why
//! `future remote devices remove|use` edit it directly and need nothing running.
//! Adding one is the only operation that needs code executed inside the Desktop.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Where requests wait (`~/.future/run/remote-control`).
const DIRECTORY: &str = "remote-control";
/// How often the host looks for a new request. Short enough that a terminal feels
/// answered immediately; the whole round trip is one platform request anyway.
const POLL: Duration = Duration::from_millis(200);
/// How old a request may be before its asker cannot still be waiting.
///
/// The terminal gives up after a minute, so anything older than this is a request
/// whose writer died or whose Desktop was not running when it was written. Two
/// minutes leaves a restart of the Desktop a chance to answer a request that is
/// still live without ever answering one nobody reads.
const ABANDONED_AFTER: Duration = Duration::from_secs(120);
/// Suffix that marks a file as a request. Anything else in the directory is not
/// ours (including a claim, below).
const REQUEST_SUFFIX: &str = ".request.json";

/// The only operation there is. Named rather than implied so a newer terminal
/// asking for something this Desktop does not know gets an explicit refusal
/// instead of a silence it would have to time out on.
const OP_ADD_DEVICE: &str = "add-device";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Request {
    #[serde(default)]
    pub(crate) op: String,
    /// Absolute path the asker is polling for its answer.
    pub(crate) reply: String,
}

/// What a terminal is told once the request has been handled.
///
/// One shape for both outcomes — the invitation when there is one, and otherwise
/// why there is not — so a caller cannot print "added" for a request that issued
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    /// The device the invitation is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pair_id: Option<String>,
    /// The complete `futureos://remote/pair` invitation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) link: Option<String>,
    /// Unix seconds after which `link` no longer works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<i64>,
    /// Why there is no invitation, when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    /// Invitations forgotten on the platform so this one could be minted.
    ///
    /// Carried out to the terminal because the alternative is another silent
    /// success: the user asked for one code and four of their earlier ones
    /// disappeared to make it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) cleared: Vec<String>,
}

impl Reply {
    /// The answer for a request that produced this status.
    ///
    /// The invitation is extracted by the same rule the headless entry point
    /// prints by (`headless::invitation_link`), not a second opinion: a link that
    /// rule refuses (expired, or not a v2 invitation) is not one to hand a
    /// terminal either.
    ///
    /// Deliberately not `headless::pairing_link`: that also requires the Agent
    /// availability probe, which starts with the bridge and cannot have answered
    /// yet. A freshly minted code is valid and scannable regardless, and gating
    /// the answer on the probe reported "no invitation" for one the user could
    /// already use.
    pub(crate) fn of_status(status: &crate::remote::RemoteStatus) -> Self {
        match crate::headless::invitation_link(status) {
            Some(link) => Self {
                pair_id: Some(status.pair_id.clone()).filter(|pair_id| !pair_id.is_empty()),
                link: Some(link),
                expires_at: status.pairing_code_expires_at,
                error: None,
                cleared: status.cleared_invitations.clone(),
            },
            None => Self::of_error(format!(
                "the invitation was not issued (remote start ended in {:?}{})",
                status.phase,
                status
                    .reason
                    .map(|reason| format!(" ({reason:?})"))
                    .unwrap_or_default()
            )),
        }
    }

    pub(crate) fn of_error(message: impl Into<String>) -> Self {
        Self {
            pair_id: None,
            link: None,
            expires_at: None,
            error: Some(message.into()),
            cleared: Vec::new(),
        }
    }
}

/// `~/.future/run/remote-control`.
///
/// Transient by design, beside the agent's own socket: a request names a reply
/// path its asker created for itself, so one that survives a restart is stale in
/// a way a data file would not be — which is why this lives under `run/` and is
/// never part of the data directory a backup restores.
pub(crate) fn directory() -> Result<PathBuf, crate::AppError> {
    let root = crate::store::future_dir()?;
    Ok(root.join("run").join(DIRECTORY))
}

/// Serve requests for this process's lifetime.
///
/// Started by both Desktops — the window once its backend is up, the headless
/// entry before it begins supervising — so a terminal's `add` is answered by
/// whichever one owns the data directory. Best effort: a data directory that
/// cannot be prepared is reported once, and the Desktop's own work continues.
pub(crate) fn spawn_host() {
    crate::runtime::spawn(async {
        let directory = match directory() {
            Ok(directory) => directory,
            Err(error) => {
                eprintln!("remote: cannot serve device requests: {error}");
                return;
            }
        };
        if let Err(error) = std::fs::create_dir_all(&directory) {
            eprintln!(
                "remote: cannot serve device requests in {}: {error}",
                directory.display()
            );
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Owner-only: the requests name a reply path, and the answers are
            // invitations. The data root is already this user's, but the
            // directory is what other accounts would have to traverse.
            let _ = std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
        }
        // A request from a previous run (the Desktop was restarting) may still be
        // worth answering; one nobody can be waiting for is not.
        for path in requests(&directory) {
            if abandoned(&path) {
                let _ = std::fs::remove_file(&path);
            }
        }
        loop {
            for path in requests(&directory) {
                serve_one(&path).await;
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

/// The request files in `directory`, oldest name first (the names carry millis).
fn requests(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(REQUEST_SUFFIX))
        })
        .collect();
    paths.sort();
    paths
}

fn abandoned(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > ABANDONED_AFTER)
}

/// Take ownership of a request by renaming it.
///
/// The rename is the claim: two desktops (a restart overlapping the previous
/// process, or a test double) cannot both perform the same request, and a claim
/// left behind by a crash is inert — only `*.request.json` is ever served.
fn claim(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let claimed = path.with_file_name(format!("{name}.{}.claim", std::process::id()));
    std::fs::rename(path, &claimed).ok().map(|()| claimed)
}

async fn serve_one(path: &Path) {
    if abandoned(path) {
        let _ = std::fs::remove_file(path);
        return;
    }
    let Some(claimed) = claim(path) else {
        return;
    };
    let parsed: Result<Request, String> = std::fs::read_to_string(&claimed)
        .map_err(|error| error.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()));
    let request = match parsed {
        Ok(request) => request,
        Err(error) => {
            // Nothing can be written back: without a readable request there is no
            // reply path. The asker times out and says so.
            eprintln!(
                "remote: ignoring an unreadable device request ({}): {error}",
                path.display()
            );
            let _ = std::fs::remove_file(&claimed);
            return;
        }
    };
    let reply = answer(&request, perform(&request.op)).await;
    if let Err(error) = write_reply(Path::new(&request.reply), &reply) {
        eprintln!("remote: cannot answer a device request: {error}");
    }
    let _ = std::fs::remove_file(&claimed);
}

/// What the request asks for, as a future the host can run.
fn perform(
    op: &str,
) -> impl std::future::Future<Output = Result<crate::remote::RemoteStatus, crate::AppError>> {
    let op = op.to_string();
    async move {
        match op.as_str() {
            OP_ADD_DEVICE => {
                // A Desktop with the remote switch off serves nobody, so an
                // invitation it minted would not work. Saying so is the honest
                // answer; turning the switch on is the operator's decision.
                if matches!(
                    crate::remote::status().phase,
                    crate::remote::RemotePhase::Stopped
                ) {
                    return Err(crate::AppError::Message(
                        "the remote desktop is off in the Desktop. Turn it on there (Remote \
                         page, or the window's Remote desktop switch) and try again."
                            .to_string(),
                    ));
                }
                crate::remote::pair_new_device().await
            }
            other => Err(crate::AppError::Message(format!(
                "this Desktop does not know the `{other}` request; update the FutureOS Desktop \
                 or the `future` CLI so the two match"
            ))),
        }
    }
}

/// Turn the operation's outcome into the answer the terminal prints.
///
/// Split from [`serve_one`] with the operation injected, so every arm — including
/// a request this Desktop does not know and one that failed — is reachable without
/// a platform, a broker or a paired device.
pub(crate) async fn answer(
    request: &Request,
    result: impl std::future::Future<Output = Result<crate::remote::RemoteStatus, crate::AppError>>,
) -> Reply {
    // Checked before waiting on anything: a request written by a newer terminal
    // for an operation this Desktop does not implement should be refused at once,
    // not after the pairing attempt it never asked for.
    if request.op != OP_ADD_DEVICE {
        return Reply::of_error(format!(
            "this Desktop does not know the `{}` request; update the FutureOS Desktop or the \
             `future` CLI so the two match",
            request.op
        ));
    }
    match result.await {
        Ok(status) => Reply::of_status(&status),
        Err(error) => Reply::of_error(error.to_string()),
    }
}

/// Write the answer where the asker is polling.
///
/// Atomic and `0600` like the rest of the pairing material: a half-written answer
/// would read as "not yet", and the file names an invitation.
fn write_reply(path: &Path, reply: &Reply) -> Result<(), String> {
    let value = serde_json::to_value(reply).map_err(|error| error.to_string())?;
    crate::config_io::write_json_atomic(path, &value, true).map_err(|error| error.to_string())
}

/// Read an answer, if one is there yet.
///
/// Empty or unparsable means "not yet", not "failed": the writer is another
/// process, and a poll can land before it has finished. Failure is reported
/// through [`Reply::error`], so this never has to guess.
#[cfg(test)]
pub(crate) fn read_reply(path: &Path) -> Option<Reply> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{RemoteFailureReason, RemotePhase, RemoteStatus};

    /// An isolated `~/.future`, with the app directory the control channel lives
    /// in already created.
    struct Root {
        dir: tempfile::TempDir,
    }

    impl Root {
        fn new() -> Self {
            Root {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn control(&self) -> PathBuf {
            let path = self.dir.path().join("run").join(DIRECTORY);
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn write_request(&self, name: &str, request: &Request) -> PathBuf {
            let path = self.control().join(name);
            std::fs::write(&path, serde_json::to_string(request).unwrap()).unwrap();
            path
        }
    }

    fn status(phase: RemotePhase, link: Option<&str>) -> RemoteStatus {
        RemoteStatus {
            devices: Vec::new(),
            client: None,
            phase,
            reason: None,
            recovery: None,
            agent_available: true,
            nats_url: String::new(),
            pair_id: "pair_AAAA".into(),
            pairing_code: link.map(str::to_string),
            pairing_code_expires_at: Some(i64::MAX),
            desktop_id: "desktop_test".into(),
            desktop_public_key: "UTEST".into(),
            web_url: None,
            web_lan_url: None,
            warning_code: None,
            cleared_invitations: Vec::new(),
        }
    }

    #[tokio::test]
    async fn an_add_device_request_answers_with_the_scannable_invitation() {
        let request = Request {
            op: OP_ADD_DEVICE.into(),
            reply: "/tmp/unused".into(),
        };
        let link =
            "futureos://remote/pair?v=2&code=a&desktopId=d&desktopKey=U&secureKey=K&secret=S";
        let reply = answer(&request, async {
            Ok(status(RemotePhase::Ready, Some(link)))
        })
        .await;
        assert_eq!(reply.error, None);
        assert_eq!(reply.link.as_deref(), Some(link));
        assert_eq!(reply.pair_id.as_deref(), Some("pair_AAAA"));
        assert_eq!(reply.expires_at, Some(i64::MAX));
    }

    #[tokio::test]
    async fn a_failed_add_device_reports_the_reason_instead_of_a_link() {
        let request = Request {
            op: OP_ADD_DEVICE.into(),
            reply: "/tmp/unused".into(),
        };
        let reply = answer(&request, async {
            Err(crate::AppError::Message("pairing_limit_reached".into()))
        })
        .await;
        assert!(reply.link.is_none(), "{reply:?}");
        assert_eq!(reply.error.as_deref(), Some("pairing_limit_reached"));
    }

    /// A start that ended without an invitation answers with *why*, never with a
    /// "successful" reply a terminal would print as done.
    #[tokio::test]
    async fn a_status_without_an_invitation_is_an_error_reply() {
        let request = Request {
            op: OP_ADD_DEVICE.into(),
            reply: "/tmp/unused".into(),
        };
        let refused = RemoteStatus {
            phase: RemotePhase::Failed,
            reason: Some(RemoteFailureReason::Network),
            ..status(RemotePhase::Failed, None)
        };
        let reply = answer(&request, async { Ok(refused) }).await;
        assert!(reply.link.is_none(), "{reply:?}");
        let error = reply.error.expect("a status without a code must say so");
        assert!(error.contains("Failed"), "{error}");
        assert!(error.contains("Network"), "{error}");
    }

    /// A request this Desktop does not implement is refused before the operation
    /// runs at all — a newer terminal must not silently pair a device it did not
    /// ask for.
    #[tokio::test]
    async fn an_unknown_operation_is_refused_without_performing_anything() {
        let request = Request {
            op: "remove-everything".into(),
            reply: "/tmp/unused".into(),
        };
        let mut performed = false;
        let reply = answer(&request, async {
            performed = true;
            Ok(status(
                RemotePhase::Ready,
                Some("futureos://remote/pair?v=2"),
            ))
        })
        .await;
        assert!(!performed, "the operation must not run");
        let error = reply
            .error
            .expect("an unknown op must answer with a reason");
        assert!(error.contains("remove-everything"), "{error}");
        assert!(error.contains("FutureOS Desktop"), "{error}");
    }

    #[test]
    fn a_request_round_trips_through_the_wire_shape() {
        let request: Request =
            serde_json::from_str(r#"{"op":"add-device","reply":"/tmp/x.json"}"#).unwrap();
        assert_eq!(request.op, OP_ADD_DEVICE);
        assert_eq!(request.reply, "/tmp/x.json");
        // An omitted `op` is not guessed at.
        let missing: Request = serde_json::from_str(r#"{"reply":"/tmp/x.json"}"#).unwrap();
        assert_eq!(missing.op, "");

        let reply = Reply {
            pair_id: Some("pair_A".into()),
            link: Some("futureos://remote/pair?v=2".into()),
            expires_at: Some(7),
            error: None,
            cleared: Vec::new(),
        };
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["pairId"], "pair_A");
        assert_eq!(value["expiresAt"], 7);
        assert!(value.get("error").is_none(), "absent error is omitted");
    }

    #[test]
    fn only_request_files_are_listed_oldest_first() {
        let root = Root::new();
        let first = root.write_request(
            "0001.request.json",
            &Request {
                op: OP_ADD_DEVICE.into(),
                reply: "/tmp/a".into(),
            },
        );
        let second = root.write_request(
            "0002.request.json",
            &Request {
                op: OP_ADD_DEVICE.into(),
                reply: "/tmp/b".into(),
            },
        );
        std::fs::write(root.control().join("0003.request.json.12.claim"), b"").unwrap();
        std::fs::write(root.control().join("notes.txt"), b"").unwrap();
        assert_eq!(requests(&root.control()), vec![first, second]);
    }

    /// The rename is the claim: a second host that looks at the same directory
    /// finds nothing left to serve, so one request is performed once.
    #[test]
    fn a_request_can_be_claimed_only_once() {
        let root = Root::new();
        let path = root.write_request(
            "0001.request.json",
            &Request {
                op: OP_ADD_DEVICE.into(),
                reply: "/tmp/a".into(),
            },
        );
        let claimed = claim(&path).expect("the first host claims it");
        assert!(claimed.is_file());
        assert!(!path.exists());
        assert_eq!(claim(&path), None, "the second host finds nothing");
        assert!(
            requests(&root.control()).is_empty(),
            "a claim is not a request"
        );
    }

    #[tokio::test]
    async fn serving_one_request_answers_it_and_removes_every_trace() {
        let root = Root::new();
        let reply_path = root.dir.path().join("answer.json");
        let path = root.write_request(
            "0001.request.json",
            &Request {
                op: OP_ADD_DEVICE.into(),
                reply: reply_path.display().to_string(),
            },
        );
        // The injected operation cannot be reached through `serve_one`, so this
        // drives the real one: no remote is running here, which is itself an arm
        // (the request is answered, not left hanging).
        serve_one(&path).await;

        let reply = read_reply(&reply_path).expect("the asker's file must be written");
        assert!(reply.error.is_some(), "{reply:?}");
        assert!(!path.exists(), "the request must be consumed");
        assert!(
            requests(&root.control()).is_empty(),
            "the claim must be cleaned up too"
        );
    }

    #[tokio::test]
    async fn an_unreadable_request_is_dropped_without_a_reply_path() {
        let root = Root::new();
        let path = root.control().join("0001.request.json");
        std::fs::write(&path, b"{ not json").unwrap();
        serve_one(&path).await;
        assert!(!path.exists());
        assert!(requests(&root.control()).is_empty());
    }

    /// A request whose asker is long gone is not answered: the Desktop that starts
    /// later must not act on it (and must not write an invitation into a file
    /// whose reader has exited).
    #[tokio::test]
    async fn an_abandoned_request_is_deleted_instead_of_served() {
        let root = Root::new();
        let reply_path = root.dir.path().join("answer.json");
        let path = root.write_request(
            "0001.request.json",
            &Request {
                op: OP_ADD_DEVICE.into(),
                reply: reply_path.display().to_string(),
            },
        );
        let old = SystemTime::now() - ABANDONED_AFTER - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();

        serve_one(&path).await;

        assert!(!path.exists(), "the stale request must be removed");
        assert!(!reply_path.exists(), "and must not be answered");
    }

    #[test]
    fn the_reply_is_written_atomically_with_no_leftovers() {
        let root = Root::new();
        let path = root.dir.path().join("answer.json");
        write_reply(
            &path,
            &Reply {
                pair_id: None,
                link: Some("futureos://remote/pair?v=2".into()),
                expires_at: None,
                error: None,
                cleared: Vec::new(),
            },
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["link"], "futureos://remote/pair?v=2");
        let left: Vec<_> = std::fs::read_dir(root.dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "answer.json")
            .collect();
        assert!(left.is_empty(), "a temp file escaped: {left:?}");
    }

    /// Both entry points call the same `spawn_host`, so this is the only place the
    /// directory is derived from the data root.
    #[test]
    fn the_channel_lives_under_the_transient_run_directory() {
        let _guard = crate::auth_store::test_support::HomeGuard::new("remote-control-dir");
        let root = crate::store::future_dir().unwrap();
        assert_eq!(directory().unwrap(), root.join("run").join(DIRECTORY));
        // Not in the data directory a backup restores.
        assert_ne!(
            directory().unwrap().parent(),
            Some(root.join("app").as_path())
        );
    }
}
