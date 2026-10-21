//! `future remote devices add|remove|use` — a terminal asking the running Desktop
//! to change what it serves.
//!
//! A device can only be *added* by the process that will serve it: the invitation
//! carries a single-use secret and the pair-scoped credential, and the bridge for
//! that pairing has to be up when the phone scans. That process is whichever
//! Desktop owns this data directory — the window, or `future headless` — so the
//! request travels to it as a file and the answer comes back as one.
//!
//! A device can only be *removed immediately* by that same process, for the mirror
//! reason: it owns the bridge, and a bridge nobody stopped keeps carrying the
//! device's commands until its credential comes up for renewal. Editing the book
//! from the terminal is still authoritative for the next start, so a removal falls
//! back to it when no Desktop answers — but while one is running, the device keeps
//! talking to a machine that has already forgotten it.
//!
//! *Choosing the controller* is the same shape one step further: the book holds
//! the decision, the running process holds the live pointer, and only a takeover
//! performed here moves the pointer without a restart.
//!
//! This is the same one-shot hand-off the launcher's old `--re-pair` performed
//! through the single-instance plugin, with two differences that are the reason
//! it moved here: it works for a headless Desktop too (which has no plugin), and
//! it adds one device instead of revoking every device and restarting.
//!
//! The files are a request/reply pair in `~/.future/run/remote-control/`:
//!
//! * `*.request.json` — `{"op": "add-device", "reply": "<absolute path>"}`,
//!   `{"op": "remove-device", "pairId": "<device>", "reply": "<absolute path>"}`,
//!   `{"op": "takeover", "pairId": "<device>", "reply": "<absolute path>"}`, or one
//!   of the client role's: `{"op": "peer-state", "reply": "..."}`,
//!   `{"op": "peer-connect|peer-disconnect|peer-takeover|peer-remove",
//!   "desktopId": "<remote>", "reply": "..."}`;
//! * the reply goes to the path the *asker* named, which it created for itself as
//!   a temporary file — so two terminals asking at once cannot read each other's
//!   invitation, and a process that exits takes its answer path with it.
//!
//! A request is claimed by renaming it: exactly one Desktop can act on it, and a
//! request nobody could still be waiting for (older than `ABANDONED_AFTER`) is
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
//! What this deliberately is *not* is a general control API.
//!
//! Three operations need code executed inside the Desktop, for one reason from
//! both sides: the running process is what serves the devices.
//!
//! * An invitation is only usable while the bridge for its pairing is up.
//! * A removal is only *immediate* if that bridge is stopped now. Editing the book
//!   from the terminal leaves a running bridge serving a device the book no longer
//!   lists, which goes quiet only when its credential comes up for renewal — up to
//!   the lifetime of the JWT it holds, minutes later.
//! * Choosing the controller is an in-memory pointer in the running process, and
//!   every command is checked against *that*. The book decides it for the next
//!   start, so a `use` that only edited it would take effect at a restart; asking
//!   the Desktop for a takeover is what moves the pointer now, and tells the
//!   previous controller to stand by exactly as a device taking over does.
//!
//! # The client role
//!
//! The same file serves `future remote peers` — the machines *this* one connects
//! out to — for the same single reason: the client runtime, its live connections
//! and the user's deliberate "stopped" marks all live in this process, and none of
//! them is on disk. `peer-state` is the read half; `peer-connect`,
//! `peer-disconnect` and `peer-remove` are the three changes, and each answers
//! with the list as it stands after the change. A peer is named by `desktopId`
//! rather than `pairId`, because both roles call their own identifier a pair id
//! and they mean different pairings.

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

/// The operations a terminal can ask for. Named rather than implied so a newer
/// terminal asking for something this Desktop does not know gets an explicit
/// refusal instead of a silence it would have to time out on.
const OP_ADD_DEVICE: &str = "add-device";
const OP_REMOVE_DEVICE: &str = "remove-device";
/// Read this Desktop's own runtime state, without changing anything.
///
/// It exists because the phase of the switch lives in this process and nowhere
/// on disk: `future remote devices` can always say what is paired, but only the
/// Desktop that owns the data directory can say whether it is serving.
const OP_STATUS: &str = "status";
/// Turn the switch on: the same `remote::enable()` the window's Remote page
/// calls, reached from a terminal.
const OP_ENABLE: &str = "enable";
/// Turn the switch off: the same graceful stop the window's switch performs,
/// which also stops the Desktop serving (and persists the switch).
const OP_DISABLE: &str = "disable";
/// Make the named device the one that drives this machine, now.
///
/// The book alone would only be read at the next start, because the live pointer
/// lives in this process and commands are checked against it. This is the same
/// move a device makes when it asks to take over
/// ([`crate::remote::takeover`]), reached from a terminal: the previous
/// controller is published `standby` and the new one `active`, and the pointer is
/// persisted so the next start agrees.
///
/// It names the same operation the window's Remote page calls ("let this
/// control") rather than inventing a second meaning for the word.
const OP_TAKEOVER: &str = "takeover";

/// The *client* role, from a terminal: the remote desktops this machine is
/// paired with as a controller (`~/.future/remote_peers.json`).
///
/// These are the mirror image of the operations above — "who do I connect to"
/// rather than "who is connected to me" — and they are asked of the Desktop for
/// the same single reason: the client runtime, the live connections and the
/// user's deliberate "stopped" marks all live in that process. Editing the book
/// from here is authoritative only for the next start.
///
/// `peer-state` reads the list without changing anything; the others name one
/// host by `desktopId` and answer with the list as it stands after the change,
/// so a terminal prints what happened instead of what it hoped. `peer-takeover`
/// also carries the host's own answer whether control moved, because this side
/// cannot see the pointer it asked the other machine to move.
const OP_PEER_STATE: &str = "peer-state";
const OP_PEER_CONNECT: &str = "peer-connect";
const OP_PEER_DISCONNECT: &str = "peer-disconnect";
const OP_PEER_TAKEOVER: &str = "peer-takeover";
const OP_PEER_REMOVE: &str = "peer-remove";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Request {
    #[serde(default)]
    pub(crate) op: String,
    /// Absolute path the asker is polling for its answer.
    pub(crate) reply: String,
    /// The device a removal or a takeover names. Absent for `add-device`, which
    /// creates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pair_id: Option<String>,
    /// The *remote* desktop a client-role operation names.
    ///
    /// Its own field rather than a second use of `pairId`: the two roles both
    /// call their own identifier a pair id, and they mean different pairings:
    /// a host request naming a peer's pair id would act on a device that has
    /// nothing to do with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) desktop_id: Option<String>,
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
    /// The Desktop stopped serving the device and forgot it.
    ///
    /// Its own field rather than "no `error`, so it worked": a removal has no
    /// invitation to show, and the terminal has to tell "this Desktop performed
    /// it" (stop now, nothing else to do) from "this Desktop does not know the
    /// request" (fall back to editing the book, which is slower but final).
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) removed: bool,
    /// This Desktop's runtime state, answering `status`.
    ///
    /// A structured snapshot rather than the raw `RemoteStatus`: the status
    /// carries the pending invitation (a single-use secret) and the desktop's
    /// key material, and an answer to "is it on" has no business carrying
    /// either. Everything a reader needs to decide *whether* it is on, and what
    /// is connected, survives the projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<StatusSnapshot>,
    /// Invitations forgotten on the platform so this one could be minted.
    ///
    /// Carried out to the terminal because the alternative is another silent
    /// success: the user asked for one code and four of their earlier ones
    /// disappeared to make it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) cleared: Vec<String>,
    /// The client role's paired hosts, answering `peer-state` and the operations
    /// that change one.
    ///
    /// Every peer answer carries the list as it stands *after* the operation,
    /// so a terminal prints what the Desktop actually holds instead of what it
    /// asked for. Empty for the host-role operations above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) peers: Vec<PeerSnapshot>,
    /// Answering `peer-takeover`: the host's own report whether this call moved
    /// control to this machine.
    ///
    /// Absent for every other request, and absent on a refusal (which is an
    /// `error`): `Some(false)` is "this machine already drove it", an answer
    /// rather than a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) took_over: Option<bool>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One paired device, as a terminal needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceSnapshot {
    pub(crate) pair_id: String,
    /// This device's own phase, spelled as the wire spells it (`ready`, …).
    pub(crate) phase: String,
    /// `active` or `standby`: whether this device may command the machine.
    pub(crate) control: String,
    /// The bridge for this pairing is connected.
    pub(crate) connected: bool,
    /// The client is actually there, as opposed to a bridge serving a pairing
    /// whose device was closed or lost. A `connected` bridge with no `present`
    /// client is what a page used to show as "in control" when nobody was.
    pub(crate) present: bool,
    /// Who is on the other end, as that client described itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) client: Option<ClientSnapshot>,
}

/// A client's own name for itself, as it declared it on `secure_ready`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientSnapshot {
    pub(crate) name: String,
    pub(crate) kind: String,
}

/// One remote desktop this machine is paired *with*, as a terminal needs it.
///
/// The client role's twin of [`DeviceSnapshot`], and deliberately not the same
/// struct: a peer has no `phase` or `present` here, because those describe a
/// bridge this machine serves. `control` and `holder` do belong, because they
/// answer whether this machine may drive *that* one — a standby peer answers
/// every heartbeat and refuses every command, which reads as "connected" and
/// works as nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerSnapshot {
    /// The remote machine's identity, and what a terminal names it by.
    pub(crate) desktop_id: String,
    /// This pairing, which is the remote side's id for it.
    pub(crate) pair_id: String,
    /// The user's local name for that machine, when they gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    /// The user's glyph for that machine, when they picked one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) icon: Option<String>,
    /// This Desktop holds a live connection to that machine right now.
    pub(crate) connected: bool,
    /// The user deliberately disconnected it, so the automatic reconnect pass
    /// leaves it alone until something asks for it by name.
    pub(crate) stopped: bool,
    /// Why the last connection attempt failed, cleared by a successful one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    /// The remote machine's own report that its agent is reachable.
    pub(crate) agent_available: bool,
    /// `active` while this machine drives it, `standby` while another paired
    /// device does. Absent when it is not connected, or when the host predates
    /// control arbitration and says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) control: Option<String>,
    /// Who holds it while this machine is in `standby`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) holder: Option<PeerHolder>,
}

/// The device driving a remote desktop, as that machine named it.
///
/// The client role's [`ClientSnapshot`]: every field is optional because a
/// controller that never declared an identity leaves the terminal with "another
/// device" rather than a name it invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerHolder {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pair_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
}

impl PeerSnapshot {
    pub(crate) fn of(peer: &crate::remote_peer::PeerSummary) -> Self {
        Self {
            desktop_id: peer.desktop_id.clone(),
            pair_id: peer.pair_id.clone(),
            name: peer.name.clone(),
            icon: peer.icon.clone(),
            connected: peer.connected,
            stopped: peer.stopped,
            error: peer.error.clone(),
            agent_available: peer.agent_available,
            // The wire's own spelling of the state, so a terminal and the
            // window read one vocabulary rather than two translations.
            control: peer.control.and_then(wire_name),
            holder: peer.holder.as_ref().map(|holder| PeerHolder {
                pair_id: holder.pair_id.clone(),
                name: holder.name.clone(),
                kind: holder.kind.clone(),
            }),
        }
    }
}

/// Where a retrying connection has got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoverySnapshot {
    pub(crate) attempt: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_attempts: Option<u64>,
    pub(crate) since: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) next_retry_at: Option<u64>,
}

/// This Desktop's runtime state, without the secrets a `RemoteStatus` carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusSnapshot {
    /// `stopped`, `enabled`, `connecting`, `ready`, `reconnecting`,
    /// `refreshing`, `failed` or `revoked`.
    pub(crate) phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery: Option<RecoverySnapshot>,
    pub(crate) agent_available: bool,
    /// Where this machine connects, with any userinfo removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) transport: Option<String>,
    /// The device in control, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) active: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) client: Option<ClientSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) devices: Vec<DeviceSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) warning_code: Option<String>,
    /// When the invitation this Desktop is showing lapses. The code itself is
    /// deliberately *not* carried: it is a single-use secret, and a terminal
    /// asking whether the switch is on has not asked to mint one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) invitation_expires_at: Option<i64>,
}

impl StatusSnapshot {
    pub(crate) fn of(status: &crate::remote::RemoteStatus) -> Self {
        Self {
            phase: wire_name(status.phase).unwrap_or_default(),
            reason: status.reason.and_then(wire_name),
            recovery: status.recovery.as_ref().map(|progress| RecoverySnapshot {
                attempt: progress.attempt,
                max_attempts: progress.max_attempts,
                since: progress.since,
                next_retry_at: progress.next_retry_at,
            }),
            agent_available: status.agent_available,
            transport: Some(strip_userinfo(&status.nats_url)).filter(|url| !url.is_empty()),
            active: status
                .devices
                .iter()
                .find(|device| matches!(device.control, crate::remote::DeviceControl::Active))
                .map(|device| device.pair_id.clone()),
            client: status.client.as_ref().map(client_snapshot),
            devices: status
                .devices
                .iter()
                .map(|device| DeviceSnapshot {
                    pair_id: device.pair_id.clone(),
                    phase: wire_name(device.phase).unwrap_or_default(),
                    control: wire_name(device.control).unwrap_or_default(),
                    connected: device.connected,
                    present: device.present,
                    client: device.client.as_ref().map(client_snapshot),
                })
                .collect(),
            warning_code: status.warning_code.clone(),
            invitation_expires_at: status.pairing_code_expires_at,
        }
    }
}

fn client_snapshot(client: &crate::remote::ClientIdentity) -> ClientSnapshot {
    ClientSnapshot {
        name: client.name.clone(),
        kind: client.kind.clone(),
    }
}

/// The name the wire uses for a status enum value (`ready`, `credential_expired`).
///
/// Taken from the value's own serialization rather than a second `match`, so a
/// phase or reason added upstream reaches a terminal without this file changing
/// and the two spellings cannot drift apart.
fn wire_name<T: Serialize>(value: T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
}

/// A transport URL with any userinfo removed.
///
/// A paired endpoint can carry a credential in the authority
/// (`tls://token@host:4222`). What a terminal needs is where this machine
/// connects — scheme, host and port — and the part before the `@` is a secret.
fn strip_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    match authority.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://{host}{tail}"),
        None => url.to_string(),
    }
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
        match super::invitation_link(status) {
            Some(link) => Self {
                pair_id: Some(status.pair_id.clone()).filter(|pair_id| !pair_id.is_empty()),
                link: Some(link),
                expires_at: status.pairing_code_expires_at,
                error: None,
                removed: false,
                cleared: status.cleared_invitations.clone(),
                peers: Vec::new(),
                took_over: None,
                status: None,
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

    /// The answer for a removal the Desktop performed.
    pub(crate) fn of_removed(pair_id: &str) -> Self {
        Self {
            pair_id: Some(pair_id.to_string()),
            removed: true,
            ..Self::default()
        }
    }

    pub(crate) fn of_error(message: impl Into<String>) -> Self {
        Self {
            pair_id: None,
            link: None,
            expires_at: None,
            error: Some(message.into()),
            removed: false,
            cleared: Vec::new(),
            peers: Vec::new(),
            took_over: None,
            status: None,
        }
    }

    /// The answer to `status`: this Desktop's state, with its secrets projected
    /// out (see [`StatusSnapshot`]).
    pub(crate) fn of_snapshot(status: &crate::remote::RemoteStatus) -> Self {
        Self {
            status: Some(StatusSnapshot::of(status)),
            ..Self::default()
        }
    }
}

/// `~/.future/run/remote-control`.
///
/// Transient by design, beside the agent's own socket: a request names a reply
/// path its asker created for itself, so one that survives a restart is stale in
/// a way a data file would not be — which is why this lives under `run/` and is
/// never part of the data directory a backup restores.
pub(crate) fn directory() -> Result<PathBuf, crate::Error> {
    let root = crate::store::future_dir()?;
    Ok(root.join("run").join(DIRECTORY))
}

/// Serve requests for this process's lifetime.
///
/// Started by both Desktops — the window once its backend is up, the headless
/// entry before it begins supervising — so a terminal's `add` is answered by
/// whichever one owns the data directory. Best effort: a data directory that
/// cannot be prepared is reported once, and the Desktop's own work continues.
pub fn spawn_host() {
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
    // The two roles answer different questions with different outcome types, so
    // the client-role operations take their own path instead of widening
    // `perform` into a union no host operation needs.
    let reply = if is_peer_op(&request.op) {
        answer_peer(&request, perform_peer(&request)).await
    } else {
        answer(&request, perform(&request)).await
    };
    if let Err(error) = write_reply(Path::new(&request.reply), &reply) {
        eprintln!("remote: cannot answer a device request: {error}");
    }
    let _ = std::fs::remove_file(&claimed);
}

/// What the request asks for, as a future the host can run.
fn perform(
    request: &Request,
) -> impl std::future::Future<Output = Result<crate::remote::RemoteStatus, crate::Error>> {
    let op = request.op.clone();
    let pair_id = request.pair_id.clone();
    async move {
        match op.as_str() {
            OP_ADD_DEVICE => {
                // A Desktop with the remote switch off serves nobody, so an
                // invitation it minted would not work. Saying so is the honest
                // answer; turning the switch on is the operator's decision — but
                // the sentence names the switch that can do it from any terminal,
                // not only the window this process may not have.
                if matches!(
                    crate::remote::status().phase,
                    crate::remote::RemotePhase::Stopped
                ) {
                    return Err(crate::Error::Message(
                        "the remote desktop is off. Turn it on with `future remote enable` (or \
                         the Remote page's switch, which stores the same setting) and try again."
                            .to_string(),
                    ));
                }
                crate::remote::pair_new_device().await
            }
            OP_REMOVE_DEVICE => {
                // The running supervisor owns every bridge: `unpair_slot` queues
                // the platform revoke, forgets the slot, stops serving that device
                // and tells it it was unpaired. That is the whole point of routing
                // this through here — a terminal editing the book alone leaves the
                // bridge alive until its credential comes up for renewal.
                let pair_id = pair_id
                    .filter(|pair_id| !pair_id.trim().is_empty())
                    .ok_or_else(|| {
                        crate::Error::Message(
                            "the removal names no device (`pairId` is missing)".to_string(),
                        )
                    })?;
                crate::remote::unpair_slot(&pair_id).await
            }
            // A read, not a change: the answer carries the supervisor's own
            // projection, so a terminal sees exactly what the window's Remote
            // page sees.
            OP_STATUS => Ok(crate::remote::status()),
            OP_TAKEOVER => {
                // The pointer a terminal can write on its own lives in the book,
                // and the book is read once, at start. A running Desktop keeps the
                // live pointer in memory and checks every command against it, so
                // handing the machine over here is what makes `future remote
                // devices use` take effect now instead of at the next restart.
                let pair_id = pair_id
                    .filter(|pair_id| !pair_id.trim().is_empty())
                    .ok_or_else(|| {
                        crate::Error::Message(
                            "the takeover names no device (`pairId` is missing)".to_string(),
                        )
                    })?;
                // With the switch off nothing is being served, so there is no
                // bridge to hand the machine to and no device to tell. Saying so
                // beats the `device_offline` refusal below, which would name the
                // symptom instead of the cause a terminal can undo.
                if matches!(
                    crate::remote::status().phase,
                    crate::remote::RemotePhase::Stopped
                ) {
                    return Err(crate::Error::Message(
                        "the remote desktop is off, so nobody is being served to hand the \
                         machine to. Turn it on with `future remote enable` (or the Remote \
                         page's switch) and the recorded controller takes over at once."
                            .to_string(),
                    ));
                }
                // A device with no runtime here cannot take control: there is no
                // bridge to send it commands, and moving the pointer to it would
                // strand the device that *is* connected. The window hides the
                // action for such a device; this is the same rule where the
                // pointer moves.
                match crate::remote::takeover(&pair_id).await {
                    Ok(status) => Ok(status),
                    Err(crate::Error::Message(code)) if code == "device_offline" => {
                        Err(crate::Error::Message(format!(
                            "{pair_id} is not connected to this Desktop, so there is no channel \
                             to hand it the machine"
                        )))
                    }
                    Err(error) => Err(error),
                }
            }
            // The switch, from a terminal. It is deliberately the same pair of
            // calls the window's switch makes: one machine, one meaning of
            // "on", and the preference is stored by the same rule — an enable
            // that came up failed is not remembered as on.
            OP_ENABLE => {
                let status = crate::remote::enable().await?;
                if crate::remote::is_on(status.phase) {
                    crate::remote::remember_switch(true);
                }
                Ok(status)
            }
            OP_DISABLE => {
                let status = crate::remote::stop_gracefully("user_disconnect").await;
                crate::remote::remember_switch(false);
                Ok(status)
            }
            other => Err(crate::Error::Message(format!(
                "this Desktop does not know the `{other}` request; update the FutureOS Desktop \
                 or the `future` CLI so the two match"
            ))),
        }
    }
}

/// Whether `op` belongs to the client role (see [`perform_peer`]).
pub(crate) fn is_peer_op(op: &str) -> bool {
    matches!(
        op,
        OP_PEER_STATE | OP_PEER_CONNECT | OP_PEER_DISCONNECT | OP_PEER_TAKEOVER | OP_PEER_REMOVE
    )
}

/// What a client-role request produced.
///
/// The list is every arm's answer, read after the change. `took_over` exists for
/// the one arm whose outcome lives on the *other* machine: this side asked, and
/// only that machine can say whether the pointer moved.
pub(crate) struct PeerOutcome {
    pub(crate) peers: Vec<crate::remote_peer::PeerSummary>,
    /// `Some(changed)` for a takeover only; `None` for every other request.
    pub(crate) took_over: Option<bool>,
}

impl PeerOutcome {
    /// A request that changes nothing about control.
    fn list(peers: Vec<crate::remote_peer::PeerSummary>) -> Self {
        Self {
            peers,
            took_over: None,
        }
    }
}

/// What a client-role request asks for, as a future this Desktop can run.
///
/// Every arm answers with the whole peer list, read after the change: a
/// terminal asking to connect a machine wants to know whether it worked, and
/// the list it would print is the same read. A host this Desktop does not know
/// is refused by name rather than silently ignored, because the terminal has a
/// stale book and needs to hear it.
///
/// `peer-state` is the read half: it changes nothing and exists because whether
/// a host is *connected* lives in this process and nowhere on disk.
fn perform_peer(
    request: &Request,
) -> impl std::future::Future<Output = Result<PeerOutcome, crate::Error>> {
    let op = request.op.clone();
    let desktop_id = request.desktop_id.clone();
    async move {
        if op == OP_PEER_STATE {
            return crate::remote_peer::runtime::list()
                .await
                .map(PeerOutcome::list);
        }
        let desktop_id = desktop_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                crate::Error::Message(format!(
                    "the `{op}` request names no remote desktop (`desktopId` is missing)"
                ))
            })?;
        let known = crate::remote_peer::runtime::list()
            .await?
            .iter()
            .any(|peer| peer.desktop_id == desktop_id);
        if !known {
            return Err(crate::Error::Message(format!(
                "`{desktop_id}` is not paired with this Desktop, so there is nothing to {}. Run \
                 `future remote peers` to see the paired machines.",
                match op.as_str() {
                    OP_PEER_CONNECT => "connect",
                    OP_PEER_DISCONNECT => "disconnect",
                    OP_PEER_TAKEOVER => "take control of",
                    _ => "remove",
                }
            )));
        }
        let mut took_over = None;
        match op.as_str() {
            // The same call the window's Connect makes, which is what clears a
            // deliberate stop: nothing else may, or the automatic pass would
            // undo the user's own answer on its next tick.
            OP_PEER_CONNECT => {
                crate::remote_peer::runtime::connect_with_emitter(&desktop_id, None).await?;
            }
            // Marks the host stopped in this process, which is the only place
            // that mark lives. The book is untouched: the pairing stays, and
            // the next Connect clears the mark.
            OP_PEER_DISCONNECT => {
                crate::remote_peer::runtime::disconnect(&desktop_id).await;
            }
            // Control comes back without a reconnection: the same `takeover`
            // command a phone sends, from this side of the pairing. It is sent
            // on the `list` lane because it names a pairing, not a
            // conversation. The host's answer says whether the pointer moved —
            // a machine this one already drives answers `changed: false`, which
            // is a result, not a refusal.
            OP_PEER_TAKEOVER => {
                let reply = crate::remote_peer::runtime::request(
                    &desktop_id,
                    serde_json::json!({ "type": "takeover" }),
                    "list",
                )
                .await?;
                // A host that predates the counter answers without `changed`.
                // The call only succeeds when this device now drives it, so
                // that is the honest reading of a silent field.
                took_over = Some(
                    reply
                        .get("changed")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true),
                );
            }
            // Local first, then a best-effort platform revoke: an unreachable
            // platform must not leave a host the user removed still paired here.
            // A revoke that fails is reported on this Desktop's own output; the
            // pairing is gone either way, and re-pairing is a new invitation.
            OP_PEER_REMOVE => {
                if let Some(note) = crate::remote_peer::runtime::unpair(&desktop_id).await? {
                    eprintln!(
                        "remote: removed the pairing for {desktop_id}, but the platform could \
                         not be told: {note}"
                    );
                }
            }
            other => {
                return Err(crate::Error::Message(format!(
                    "this Desktop does not know the `{other}` request; update the FutureOS \
                     Desktop or the `future` CLI so the two match"
                )))
            }
        }
        Ok(PeerOutcome {
            peers: crate::remote_peer::runtime::list().await?,
            took_over,
        })
    }
}

/// Turn a client-role outcome into the answer the terminal prints.
///
/// Split from [`serve_one`] with the outcome injected for the same reason
/// [`answer`] is: the failure arm must be reachable without a broker, a
/// platform or a paired machine.
pub(crate) async fn answer_peer(
    request: &Request,
    result: impl std::future::Future<Output = Result<PeerOutcome, crate::Error>>,
) -> Reply {
    match result.await {
        Ok(outcome) => Reply {
            peers: outcome.peers.iter().map(PeerSnapshot::of).collect(),
            // A removal has no row to show afterwards, and the terminal has to
            // tell "this Desktop dropped the pairing" from "it did not know the
            // request" (which is an `error`).
            removed: is_peer_removal(&request.op),
            took_over: outcome.took_over,
            ..Reply::default()
        },
        Err(error) => Reply::of_error(error.to_string()),
    }
}

fn is_peer_removal(op: &str) -> bool {
    op == OP_PEER_REMOVE
}

/// Turn the operation's outcome into the answer the terminal prints.
///
/// Split from [`serve_one`] with the operation injected, so every arm — including
/// a request this Desktop does not know and one that failed — is reachable without
/// a platform, a broker or a paired device.
pub(crate) async fn answer(
    request: &Request,
    result: impl std::future::Future<Output = Result<crate::remote::RemoteStatus, crate::Error>>,
) -> Reply {
    // Checked before waiting on anything: a request written by a newer terminal
    // for an operation this Desktop does not implement should be refused at once,
    // not after the pairing attempt it never asked for.
    let removal = match request.op.as_str() {
        OP_ADD_DEVICE => false,
        OP_REMOVE_DEVICE => true,
        // Everything that answers with the Desktop's own state, with no
        // invitation to extract and no removal to report: the switch's two
        // halves, the runtime snapshot, and the takeover that moves the
        // controller (its answer names the device now in control, which is what
        // the terminal verifies before saying the machine changed hands).
        OP_STATUS | OP_ENABLE | OP_DISABLE | OP_TAKEOVER => {
            return match result.await {
                Ok(status) => Reply::of_snapshot(&status),
                Err(error) => Reply::of_error(error.to_string()),
            }
        }
        _ => {
            return Reply::of_error(format!(
                "this Desktop does not know the `{}` request; update the FutureOS Desktop or the \
                 `future` CLI so the two match",
                request.op
            ))
        }
    };
    match result.await {
        // A removal answers with the device it stopped serving rather than with
        // the status's own subject: after the last device leaves there is no
        // pairing left to project, and the terminal asked about one device.
        Ok(_) if removal => Reply::of_removed(request.pair_id.as_deref().unwrap_or_default()),
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
            desktop_id: None,
            pair_id: None,
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
            desktop_id: None,
            pair_id: None,
        };
        let reply = answer(&request, async {
            Err(crate::Error::Message("pairing_limit_reached".into()))
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
            desktop_id: None,
            pair_id: None,
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
            desktop_id: None,
            pair_id: None,
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

    /// `status` answers from the supervisor's projection, and the answer a
    /// terminal reads carries no secret: not the pending invitation, not the
    /// credential the transport URL embeds.
    #[tokio::test]
    async fn a_status_request_answers_with_the_runtime_state_and_no_secrets() {
        let request = Request {
            op: OP_STATUS.into(),
            reply: "/tmp/unused".into(),
            desktop_id: None,
            pair_id: None,
        };
        let mut running = status(RemotePhase::Ready, Some("futureos://remote/pair?v=2"));
        running.nats_url = "tls://token:secret@relay.example:4222".into();
        running.client = Some(crate::remote::ClientIdentity {
            name: "iPhone".into(),
            kind: "phone".into(),
        });
        running.devices = vec![future_remote_server::types::RemoteDevice {
            pair_id: "pair_AAAA".into(),
            desktop_id: "desktop_test".into(),
            phase: RemotePhase::Ready,
            control: crate::remote::DeviceControl::Active,
            connected: true,
            present: true,
            client: None,
            pairing_code: None,
            pairing_code_expires_at: None,
        }];
        let reply = answer(&request, async { Ok(running) }).await;
        assert_eq!(reply.error, None);
        let snapshot = reply.status.clone().expect("a status answer");
        assert_eq!(snapshot.phase, "ready");
        assert_eq!(snapshot.active.as_deref(), Some("pair_AAAA"));
        assert_eq!(
            snapshot.transport.as_deref(),
            Some("tls://relay.example:4222"),
            "the transport keeps where, not who"
        );
        assert!(snapshot.agent_available);
        assert_eq!(
            snapshot.client.as_ref().map(|c| c.name.as_str()),
            Some("iPhone")
        );
        assert_eq!(snapshot.devices.len(), 1);
        assert!(snapshot.devices[0].connected);
        assert_eq!(snapshot.devices[0].control, "active");
        assert_eq!(snapshot.devices[0].phase, "ready");
        assert_eq!(snapshot.invitation_expires_at, Some(i64::MAX));

        let wire = serde_json::to_string(&reply).unwrap();
        assert!(!wire.contains("token"), "the transport credential: {wire}");
        assert!(!wire.contains("secret"), "the transport credential: {wire}");
        assert!(
            !wire.contains("futureos://remote/pair"),
            "the invitation: {wire}"
        );
        assert!(!wire.contains("pairingCode"), "{wire}");
    }

    /// A takeover answers with the Desktop's own state, and the answer names the
    /// device that now controls — which is what the terminal checks before it
    /// reports the machine as handed over.
    #[tokio::test]
    async fn a_takeover_answers_with_the_device_now_in_control() {
        let request = Request {
            op: OP_TAKEOVER.into(),
            reply: "/tmp/unused".into(),
            desktop_id: None,
            pair_id: Some("pair_BBBB".into()),
        };
        let device = |pair_id: &str, control| future_remote_server::types::RemoteDevice {
            pair_id: pair_id.to_string(),
            desktop_id: "desktop_test".into(),
            phase: RemotePhase::Ready,
            control,
            connected: true,
            present: true,
            client: None,
            pairing_code: None,
            pairing_code_expires_at: None,
        };
        let moved = RemoteStatus {
            devices: vec![
                device("pair_BBBB", crate::remote::DeviceControl::Active),
                device("pair_AAAA", crate::remote::DeviceControl::Standby),
            ],
            ..status(RemotePhase::Ready, None)
        };
        let reply = answer(&request, async { Ok(moved) }).await;
        assert_eq!(reply.error, None, "{reply:?}");
        assert!(reply.link.is_none(), "a takeover issues no invitation");
        assert!(!reply.removed, "and removes nothing");
        let snapshot = reply.status.expect("a snapshot");
        assert_eq!(snapshot.active.as_deref(), Some("pair_BBBB"));
        assert_eq!(
            snapshot
                .devices
                .iter()
                .find(|device| device.pair_id == "pair_AAAA")
                .map(|device| device.control.as_str()),
            Some("standby"),
            "the device that lost the machine is reported as standby"
        );
    }

    /// A takeover that names no device is refused before the pointer could move
    /// anywhere — handing the machine to nobody is not an operation.
    #[tokio::test]
    async fn a_takeover_without_a_device_is_refused() {
        let request = Request {
            op: OP_TAKEOVER.into(),
            reply: "/tmp/unused".into(),
            desktop_id: None,
            pair_id: None,
        };
        let error = perform(&request)
            .await
            .expect_err("a nameless takeover must not run");
        assert!(error.to_string().contains("names no device"), "{error}");
    }

    /// A takeover this Desktop refused says why and carries no snapshot: the
    /// terminal must not print "handed over" for a pointer that never moved.
    #[tokio::test]
    async fn a_refused_takeover_reports_the_reason_and_no_state() {
        let request = Request {
            op: OP_TAKEOVER.into(),
            reply: "/tmp/unused".into(),
            desktop_id: None,
            pair_id: Some("pair_BBBB".into()),
        };
        let reply = answer(&request, async {
            Err(crate::Error::Message(
                "pair_BBBB is not connected to this Desktop, so there is no channel to hand it \
                 the machine"
                    .into(),
            ))
        })
        .await;
        assert!(reply.status.is_none(), "{reply:?}");
        let error = reply.error.expect("a refusal must say why");
        assert!(error.contains("not connected"), "{error}");
    }

    /// The switch is reachable from a terminal: both operations reach the
    /// supervisor's own calls instead of being refused as unknown, and the answer
    /// is the resulting state.
    #[tokio::test]
    async fn the_switch_operations_are_recognized_by_this_desktop() {
        for op in [OP_ENABLE, OP_DISABLE] {
            let request = Request {
                op: op.into(),
                reply: "/tmp/unused".into(),
                desktop_id: None,
                pair_id: None,
            };
            let performed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = performed.clone();
            let reply = answer(&request, async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(status(RemotePhase::Stopped, None))
            })
            .await;
            assert!(
                performed.load(std::sync::atomic::Ordering::SeqCst),
                "{op} must reach the operation"
            );
            assert_eq!(reply.error, None, "{op}: {reply:?}");
            assert_eq!(
                reply.status.map(|status| status.phase),
                Some("stopped".into())
            );
        }
    }

    #[test]
    fn a_transport_url_loses_its_credential_and_nothing_else() {
        assert_eq!(
            strip_userinfo("tls://token@relay.example:4222"),
            "tls://relay.example:4222"
        );
        assert_eq!(
            strip_userinfo("tls://relay.example:4222"),
            "tls://relay.example:4222"
        );
        assert_eq!(strip_userinfo("nats://u:p@host:1/x"), "nats://host:1/x");
        // No scheme is nothing to redact: the string is returned as it came.
        assert_eq!(strip_userinfo("relay.example:4222"), "relay.example:4222");
        assert_eq!(strip_userinfo(""), "");
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
        assert_eq!(missing.pair_id, None);

        // A removal names its device on the same wire, and an add has no device
        // to name.
        let removal: Request = serde_json::from_str(
            r#"{"op":"remove-device","pairId":"pair_B","reply":"/tmp/y.json"}"#,
        )
        .unwrap();
        assert_eq!(removal.op, OP_REMOVE_DEVICE);
        assert_eq!(removal.pair_id.as_deref(), Some("pair_B"));
        assert_eq!(serde_json::to_value(&removal).unwrap()["pairId"], "pair_B");
        let add = serde_json::to_value(Request {
            op: OP_ADD_DEVICE.into(),
            reply: "/tmp/x.json".into(),
            desktop_id: None,
            pair_id: None,
        })
        .unwrap();
        assert!(
            add.get("pairId").is_none(),
            "an addition names no device: {add}"
        );

        let reply = Reply {
            pair_id: Some("pair_A".into()),
            link: Some("futureos://remote/pair?v=2".into()),
            expires_at: Some(7),
            error: None,
            removed: false,
            cleared: Vec::new(),
            peers: Vec::new(),
            took_over: None,
            status: None,
        };
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["pairId"], "pair_A");
        assert_eq!(value["expiresAt"], 7);
        assert!(value.get("error").is_none(), "absent error is omitted");
    }

    /// A removal answers with the device it stopped serving — no invitation, and
    /// not a failure. The terminal needs this arm told apart from "no reply".
    #[tokio::test]
    async fn a_removal_answers_with_the_device_it_stopped_serving() {
        let request = Request {
            op: OP_REMOVE_DEVICE.into(),
            reply: "/tmp/unused".into(),
            desktop_id: None,
            pair_id: Some("pair_GONE".into()),
        };
        // The status after a removal describes whoever is left, not the device
        // that just left; the reply must not report that one.
        let remaining = RemoteStatus {
            pair_id: "pair_OTHER".into(),
            ..status(RemotePhase::Ready, None)
        };
        let reply = answer(&request, async { Ok(remaining) }).await;
        assert_eq!(reply.error, None);
        assert!(reply.link.is_none(), "a removal issues no invitation");
        assert!(reply.removed, "{reply:?}");
        assert_eq!(reply.pair_id.as_deref(), Some("pair_GONE"));
        // And it is written as `removed`, not inferred from a missing error.
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["removed"], true);
        assert!(value.get("link").is_none());
    }

    /// A removal that failed says why and never claims it happened: the terminal
    /// would otherwise stop waiting and report a device it is still serving.
    #[tokio::test]
    async fn a_failed_removal_reports_the_reason_and_claims_nothing() {
        let request = Request {
            op: OP_REMOVE_DEVICE.into(),
            reply: "/tmp/unused".into(),
            desktop_id: None,
            pair_id: Some("pair_GONE".into()),
        };
        let reply = answer(&request, async {
            Err(crate::Error::Message("pairing_book_unreadable".into()))
        })
        .await;
        assert!(!reply.removed, "{reply:?}");
        assert_eq!(reply.error.as_deref(), Some("pairing_book_unreadable"));
    }

    /// A removal that names no device is refused before anything is stopped; the
    /// older wire shape (which a stale terminal could still write) is not a
    /// machine-wide stop.
    #[tokio::test]
    async fn a_removal_without_a_device_is_refused() {
        let request = Request {
            op: OP_REMOVE_DEVICE.into(),
            reply: "/tmp/unused".into(),
            desktop_id: None,
            pair_id: None,
        };
        let error = perform(&request)
            .await
            .expect_err("a nameless removal must not run");
        assert!(error.to_string().contains("names no device"), "{error}");
    }

    #[test]
    fn only_request_files_are_listed_oldest_first() {
        let root = Root::new();
        let first = root.write_request(
            "0001.request.json",
            &Request {
                op: OP_ADD_DEVICE.into(),
                reply: "/tmp/a".into(),
                desktop_id: None,
                pair_id: None,
            },
        );
        let second = root.write_request(
            "0002.request.json",
            &Request {
                op: OP_ADD_DEVICE.into(),
                reply: "/tmp/b".into(),
                desktop_id: None,
                pair_id: None,
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
                desktop_id: None,
                pair_id: None,
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
                desktop_id: None,
                pair_id: None,
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
                desktop_id: None,
                pair_id: None,
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
                removed: false,
                cleared: Vec::new(),
                peers: Vec::new(),
                took_over: None,
                status: None,
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

    // -- the client role (`future remote peers`) ---------------------------

    /// One peer as the client runtime reports it.
    fn peer(desktop_id: &str, connected: bool) -> crate::remote_peer::PeerSummary {
        crate::remote_peer::PeerSummary {
            desktop_id: desktop_id.into(),
            pair_id: "pair_1".into(),
            name: Some("work".into()),
            icon: None,
            connected,
            stopped: false,
            error: None,
            bridge_instance_id: None,
            features: Vec::new(),
            agent_available: connected,
            control: None,
            holder: None,
        }
    }

    fn peer_request(op: &str, desktop_id: Option<&str>) -> Request {
        Request {
            op: op.into(),
            reply: "/tmp/unused".into(),
            pair_id: None,
            desktop_id: desktop_id.map(str::to_string),
        }
    }

    /// The client-role requests are routed away from the host ones. Getting this
    /// wrong would send a `peer-connect` to a `match` that refuses it, or worse,
    /// let a host request reach code that acts on a peer.
    #[test]
    fn the_client_role_operations_are_told_apart_from_the_host_ones() {
        for op in [
            OP_PEER_STATE,
            OP_PEER_CONNECT,
            OP_PEER_DISCONNECT,
            OP_PEER_TAKEOVER,
            OP_PEER_REMOVE,
        ] {
            assert!(is_peer_op(op), "{op} is a client-role request");
        }
        for op in [
            OP_ADD_DEVICE,
            OP_REMOVE_DEVICE,
            OP_STATUS,
            OP_ENABLE,
            OP_DISABLE,
            OP_TAKEOVER,
            "peer",
            "",
        ] {
            assert!(!is_peer_op(op), "{op} is not a client-role request");
        }
    }

    /// The read half: the answer is the list, projected so no credential of any
    /// kind can ride out on it.
    #[tokio::test]
    async fn a_peer_state_request_answers_with_the_live_list() {
        let request = peer_request(OP_PEER_STATE, None);
        let reply = answer_peer(&request, async {
            Ok(PeerOutcome::list(vec![
                peer("desktop_A", true),
                peer("desktop_B", false),
            ]))
        })
        .await;
        assert_eq!(reply.error, None);
        assert!(!reply.removed, "a read removes nothing");
        assert_eq!(reply.took_over, None, "a read moves no control");
        assert_eq!(reply.peers.len(), 2);
        assert_eq!(reply.peers[0].desktop_id, "desktop_A");
        assert!(reply.peers[0].connected);
        assert_eq!(reply.peers[1].desktop_id, "desktop_B");
        assert!(!reply.peers[1].connected);
        assert_eq!(reply.peers[0].name.as_deref(), Some("work"));
    }

    /// A held machine says so, and says who holds it. "Connected" alone would
    /// claim a peer usable whose every command comes back `superseded`.
    #[test]
    fn a_held_peer_carries_its_control_and_holder() {
        let mut held = peer("desktop_A", true);
        held.control = Some(future_remote_client::session::ControlState::Standby);
        held.holder = Some(future_remote_client::session::Holder {
            pair_id: Some("pair_phone".into()),
            name: Some("phone".into()),
            kind: Some("phone".into()),
        });
        let snapshot = PeerSnapshot::of(&held);
        assert_eq!(snapshot.control.as_deref(), Some("standby"));
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(value["control"], "standby");
        assert_eq!(value["holder"]["name"], "phone");
        // A controller nobody described leaves the holder out entirely, so the
        // terminal falls back to "another device" instead of a stale name.
        held.control = Some(future_remote_client::session::ControlState::Active);
        held.holder = None;
        let value = serde_json::to_value(PeerSnapshot::of(&held)).unwrap();
        assert_eq!(value["control"], "active");
        assert!(value.get("holder").is_none(), "{value}");
    }

    /// A removal answers with `removed` even when the list it carries is empty:
    /// after the last pairing leaves there is no row left to say it worked.
    #[tokio::test]
    async fn a_peer_removal_says_it_happened_even_with_nothing_left_to_show() {
        let request = peer_request(OP_PEER_REMOVE, Some("desktop_A"));
        let reply = answer_peer(&request, async { Ok(PeerOutcome::list(Vec::new())) }).await;
        assert!(reply.removed);
        assert_eq!(reply.error, None);
        assert!(reply.peers.is_empty());
    }

    /// A takeover carries the host's own verdict, and only a takeover does: the
    /// terminal prints "already yours" for `false` instead of claiming control
    /// just changed hands, and it cannot make that distinction up on its own.
    #[tokio::test]
    async fn a_peer_takeover_answers_with_whether_control_moved() {
        for (changed, expected) in [(true, Some(true)), (false, Some(false))] {
            let request = peer_request(OP_PEER_TAKEOVER, Some("desktop_A"));
            let reply = answer_peer(&request, async move {
                Ok(PeerOutcome {
                    peers: vec![peer("desktop_A", true)],
                    took_over: Some(changed),
                })
            })
            .await;
            assert_eq!(reply.error, None);
            assert_eq!(reply.took_over, expected);
            assert!(!reply.removed, "a takeover removes nothing");
        }
    }

    /// A refusal is a sentence, not a silent empty list: a terminal has to be
    /// able to tell "nothing is paired" from "it did not happen".
    #[tokio::test]
    async fn a_refused_peer_request_carries_the_reason() {
        let request = peer_request(OP_PEER_CONNECT, Some("desktop_A"));
        let reply = answer_peer(&request, async {
            Err(crate::Error::Message(
                "`desktop_A` is not paired with this Desktop".into(),
            ))
        })
        .await;
        assert_eq!(
            reply.error.as_deref(),
            Some("`desktop_A` is not paired with this Desktop")
        );
        assert!(!reply.removed);
        assert!(reply.peers.is_empty());
    }

    /// The request carries the two roles' names in their own fields, and the
    /// answer carries the list under `peers` - the shape the terminal reads.
    #[test]
    fn a_peer_request_names_a_desktop_and_the_answer_carries_the_list() {
        let request = peer_request(OP_PEER_DISCONNECT, Some("desktop_A"));
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["desktopId"], "desktop_A");
        assert!(value.get("pairId").is_none(), "{value}");
        assert_eq!(value["op"], OP_PEER_DISCONNECT);

        let reply = Reply {
            peers: vec![PeerSnapshot::of(&peer("desktop_A", true))],
            ..Reply::default()
        };
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["peers"][0]["desktopId"], "desktop_A");
        assert_eq!(value["peers"][0]["connected"], true);
        // An empty list is omitted, so a host-role answer is byte-identical to
        // what it was before this channel learned the client role.
        let empty = serde_json::to_value(Reply::default()).unwrap();
        assert!(empty.get("peers").is_none(), "{empty}");
    }
}
