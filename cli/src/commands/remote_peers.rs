//! `future remote peers` — the remote desktops *this* machine connects out to.
//!
//! The twin of [`super::remote`], and deliberately a separate group: that one
//! answers "who may drive me" (phones and other computers paired *into* this
//! machine's remote desktop), this one answers "which machines do I drive"
//! (the client role, `~/.future/remote_peers.json`). The two books, the two
//! runtimes and the two sets of credentials are independent, and one command
//! group holding both would invite exactly the mistake the two halves exist to
//! prevent: reading one status where the other was meant.
//!
//! # What is authoritative
//!
//! * **The pairing itself** is a short claim plus a Noise handshake with the
//!   other machine, which needs nothing running on this one: `add` performs the
//!   same pairing the window's "Connect another computer" box does, stores the
//!   credential in the book, and connects once so the answer is "it works"
//!   rather than "it was written down". A running Desktop picks the new pairing
//!   up on its next liveness pass and keeps it connected from then on.
//! * **The book** (`~/.future/remote_peers.json`) is what `list`, `rename` and
//!   the fallback paths read and write. It is parsed as a [`serde_json::Value`]
//!   so the fields this command does not model (the NKey seed, the pinned
//!   handshake identity, whatever a newer Desktop adds) survive an edit, and so
//!   nothing here can print a credential by accident.
//! * **A live connection** exists only inside the Desktop process (the window,
//!   or `future host`): connect, disconnect and the user's own "stopped"
//!   mark are all in that process's memory. So `connect`, `disconnect`,
//!   `takeover` and `remove` ask the running Desktop through the same
//!   request/reply files `future remote devices` uses ([`super::remote`]
//!   documents that channel), and say plainly when no Desktop is running to
//!   answer.

use super::remote::{
    ask, desktop_running, future_dir, one_device, positional, request_desktop_naming, write_json,
};
use crate::help;
use crate::output::Output;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The client-role credential book (`~/.future/remote_peers.json`).
const BOOK_FILE: &str = "remote_peers.json";

/// The Desktop's own names for the client-role requests
/// (`host/src/remote/control.rs`).
const OP_PEER_STATE: &str = "peer-state";
const OP_PEER_CONNECT: &str = "peer-connect";
const OP_PEER_DISCONNECT: &str = "peer-disconnect";
const OP_PEER_TAKEOVER: &str = "peer-takeover";
const OP_PEER_REMOVE: &str = "peer-remove";

/// How long a read waits for the running Desktop.
///
/// The answer is the client runtime's own list, from memory, so a Desktop that
/// is there answers within one poll. This covers one busy with another request,
/// and it is deliberately short: a pure query must not hang for the half minute
/// a connection attempt may legitimately take.
#[cfg(not(test))]
const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// A fake answers in the same millisecond; this only has to outlast a poll or
/// two so the "silent Desktop" cases stay reachable without a real wait.
#[cfg(test)]
const STATE_TIMEOUT: Duration = Duration::from_millis(150);

/// How long `connect`, `disconnect` and `remove` wait for the Desktop.
///
/// A disconnect and a removal are local work in that process and answer at
/// once; a connect is a real handshake with another machine over the broker, so
/// the deadline has to cover that rather than the round trip to the file.
#[cfg(not(test))]
const PEER_TIMEOUT: Duration = Duration::from_secs(30);
/// A fake answers in the same millisecond; see [`STATE_TIMEOUT`].
#[cfg(test)]
const PEER_TIMEOUT: Duration = Duration::from_millis(400);

/// `future remote peers [list|add|connect|disconnect|takeover|rename|remove]`.
pub async fn peers(args: &[String], out: &Output) -> Result<(), String> {
    let (action, rest) = match args.first().map(String::as_str) {
        None => ("list", args),
        Some("--help" | "-h") => {
            out.log(help::REMOTE_PEERS_HELP);
            return Ok(());
        }
        Some("list" | "add" | "connect" | "disconnect" | "takeover" | "rename" | "remove") => {
            (args[0].as_str(), &args[1..])
        }
        // A flag before any action belongs to the default one, so
        // `future remote peers --json` is a list and `--bogus` is reported by the
        // action that would have taken it, not as an unknown command.
        Some(flag) if flag.starts_with('-') => ("list", args),
        Some(argument) => {
            return Err(format!(
                "Unknown argument: {argument}\n\n{}",
                help::REMOTE_PEERS_USAGE
            ))
        }
    };
    if rest
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        out.log(match action {
            "list" => help::REMOTE_PEERS_LIST_HELP,
            "add" => help::REMOTE_PEERS_ADD_HELP,
            "connect" | "disconnect" => help::REMOTE_PEERS_CONNECT_HELP,
            "takeover" => help::REMOTE_PEERS_TAKEOVER_HELP,
            "rename" => help::REMOTE_PEERS_RENAME_HELP,
            _ => help::REMOTE_PEERS_REMOVE_HELP,
        });
        return Ok(());
    }
    match action {
        "list" => list(rest, out).await,
        "add" => add(rest, out).await,
        "connect" => reach(true, rest, out).await,
        "disconnect" => reach(false, rest, out).await,
        "takeover" => takeover(rest, out).await,
        "rename" => rename(rest, out),
        _ => remove(rest, out).await,
    }
}

// -- The client-role book ---------------------------------------------------

/// One remote desktop this machine is paired with.
///
/// Only the fields a terminal reads: the credential material is never a
/// [`Peer`], so no printing path can reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Peer {
    desktop_id: String,
    pair_id: String,
    name: Option<String>,
    icon: Option<String>,
    /// What the running Desktop said about this pairing, when it answered.
    /// `None` is honestly unknown: the book cannot know whether a connection is
    /// up, because connections live in that process.
    live: Option<Live>,
}

/// The live half of one peer, as the Desktop reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Live {
    connected: bool,
    stopped: bool,
    error: Option<String>,
    agent_available: bool,
    /// `active` while this machine drives it, `standby` while another paired
    /// device does; `None` when the host said nothing (not connected, or an
    /// older build).
    control: Option<String>,
    /// Who holds it while this machine is in `standby`, as that machine named
    /// the holder — possibly not at all.
    holder: Option<Holder>,
}

/// The device driving a remote desktop, as that machine named it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Holder {
    name: Option<String>,
    /// A phone, a tablet, a computer: what the holder called itself. Kept for the
    /// JSON a script reads; the printed row only has room for the name.
    kind: Option<String>,
}

impl Peer {
    fn from_value(value: &Value, file: &Path) -> Result<Self, String> {
        let object = value.as_object().ok_or_else(|| {
            format!(
                "{}: every entry in `peers` must be a JSON object.",
                file.display()
            )
        })?;
        let text = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let desktop_id = text("desktopId").ok_or_else(|| {
            format!(
                "{}: an entry in `peers` has no `desktopId`, so there is no machine to name.",
                file.display()
            )
        })?;
        Ok(Self {
            desktop_id,
            pair_id: text("pairId").unwrap_or_default(),
            name: text("name"),
            icon: text("icon"),
            live: None,
        })
    }

    fn from_live(value: &Value) -> Option<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|value| !value.is_empty())
        };
        Some(Self {
            desktop_id: text("desktopId")?,
            pair_id: text("pairId").unwrap_or_default(),
            name: text("name"),
            icon: text("icon"),
            live: Some(Live {
                connected: value
                    .get("connected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                stopped: value
                    .get("stopped")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                error: text("error"),
                agent_available: value
                    .get("agentAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                control: text("control"),
                holder: value
                    .get("holder")
                    .and_then(Value::as_object)
                    .map(|holder| {
                        let field = |key: &str| {
                            holder
                                .get(key)
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .filter(|value| !value.is_empty())
                        };
                        Holder {
                            name: field("name"),
                            kind: field("kind"),
                        }
                    }),
            }),
        })
    }

    /// The name to show: the user's own, else the machine id.
    fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.desktop_id)
    }

    /// Whether this machine may drive it, as far as the Desktop said.
    ///
    /// `false` only for an explicit `standby`: a host that said nothing (an
    /// older build, or no connection) is not evidence that somebody else holds
    /// it, and "taken over" would be a claim this side cannot make.
    fn held(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|live| live.connected && live.control.as_deref() == Some("standby"))
    }
}

/// The parsed book, plus the document to write back.
struct Book {
    path: PathBuf,
    /// The whole file, so an edit keeps every field this command does not model.
    document: Value,
    peers: Vec<Peer>,
}

impl Book {
    fn empty(path: PathBuf) -> Self {
        Self {
            path,
            document: json!({ "peers": [] }),
            peers: Vec::new(),
        }
    }

    fn load() -> Result<Self, String> {
        let path = future_dir()?.join(BOOK_FILE);
        if !path.is_file() {
            return Ok(Self::empty(path));
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        let document: Value = serde_json::from_str(&text).map_err(|error| {
            format!(
                "{} is not valid JSON ({error}). Fix or remove it; this command will not \
                 replace a pairing file it cannot read.",
                path.display()
            )
        })?;
        if !document.is_object() {
            return Err(format!(
                "The root of {} must be a JSON object.",
                path.display()
            ));
        }
        let peers = match document.get("peers") {
            Some(Value::Array(peers)) => peers
                .iter()
                .map(|peer| Peer::from_value(peer, &path))
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
            Some(_) => {
                return Err(format!(
                    "{}: `peers` must be an array of paired machines.",
                    path.display()
                ))
            }
        };
        Ok(Self {
            path,
            document,
            peers,
        })
    }

    /// Which row the user means: an exact id first (case-insensitively, with the
    /// `desktop_` prefix optional), then the 1-based row number this command
    /// prints, then a unique id prefix.
    fn position(&self, selector: &str) -> Result<usize, String> {
        let wanted = normalized(selector);
        if let Some(index) = self
            .peers
            .iter()
            .position(|peer| normalized(&peer.desktop_id) == wanted)
        {
            return Ok(index);
        }
        if let Some(index) = selector
            .trim()
            .strip_prefix('#')
            .unwrap_or(selector.trim())
            .parse::<usize>()
            .ok()
            .filter(|index| *index >= 1 && *index <= self.peers.len())
        {
            return Ok(index - 1);
        }
        let matches: Vec<usize> = self
            .peers
            .iter()
            .enumerate()
            .filter(|(_, peer)| normalized(&peer.desktop_id).starts_with(&wanted))
            .map(|(index, _)| index)
            .collect();
        match matches.as_slice() {
            [only] => Ok(*only),
            [] => Err(format!(
                "No remote desktop matches `{selector}`. Run `future remote peers` to see the \
                 paired machines."
            )),
            _ => Err(format!(
                "`{selector}` matches {} machines. Give a longer prefix.",
                matches.len()
            )),
        }
    }

    /// Re-read the book, apply `edit` to what is there now, and write it.
    ///
    /// The copy in hand was read before work that can hold a command for half a
    /// minute, and the Desktop writes this same file meanwhile (it stores a new
    /// pairing as it connects one, and refreshes credentials in place). Writing
    /// a stale copy back would drop whatever landed in between, so every edit is
    /// re-applied to the file as it is now.
    fn edit(&mut self, edit: impl FnOnce(&mut Self) -> Result<(), String>) -> Result<(), String> {
        let mut fresh = Self::load()?;
        edit(&mut fresh)?;
        fresh.save()?;
        *self = fresh;
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        write_json(&self.path, &self.document)
    }
}

/// Compare ids the way a person copies them: case-insensitively, and with the
/// `desktop_` prefix the platform adds treated as decoration.
fn normalized(desktop_id: &str) -> String {
    let trimmed = desktop_id.trim();
    trimmed
        .strip_prefix("desktop_")
        .unwrap_or(trimmed)
        .to_ascii_uppercase()
}

// -- list -------------------------------------------------------------------

/// `future remote peers [list] [--json]`
async fn list(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json"], help::REMOTE_PEERS_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    if let Some(extra) = values.first() {
        return Err(format!(
            "`list` takes no machine: it lists them all (drop `{extra}`).\n\n{}",
            help::REMOTE_PEERS_USAGE
        ));
    }
    let book = Book::load()?;
    let observed = observe(out).await;
    let rows = merged(&book, &observed);
    if json_flag {
        out.log(&pretty(json!({
            "desktopRunning": observed.running,
            "answered": observed.answered,
            "peers": rows.iter().map(row_document).collect::<Vec<_>>(),
        }))?);
        return Ok(());
    }
    if rows.is_empty() {
        out.log("This machine is not paired with any remote desktop.");
        out.log("Pair one with `future remote peers add <invitation>`: on the other machine open");
        out.log("Remote -> the connection to another computer, copy its invitation, and paste it");
        out.log("here.");
        return Ok(());
    }
    out.log(&format!(
        "{} {} paired with this machine:",
        rows.len(),
        if rows.len() == 1 {
            "remote desktop is"
        } else {
            "remote desktops are"
        }
    ));
    out.log("");
    let ids: Vec<&str> = rows.iter().map(|peer| peer.desktop_id.as_str()).collect();
    let id_width = ids.iter().map(|id| id.len()).max().unwrap_or(0).max(7);
    let names: Vec<&str> = rows
        .iter()
        .map(|peer| peer.name.as_deref().unwrap_or("-"))
        .collect();
    let name_width = names
        .iter()
        .map(|name| name.len())
        .max()
        .unwrap_or(0)
        .max(4);
    out.log(&format!(
        "  {:>2}  {:<id_width$}  {:<name_width$}  {}",
        "#", "DESKTOP", "NAME", "STATE"
    ));
    for (index, peer) in rows.iter().enumerate() {
        out.log(&format!(
            "  {:>2}  {:<id_width$}  {:<name_width$}  {}",
            index + 1,
            peer.desktop_id,
            names[index],
            state_of(peer, observed.running)
        ));
        if let Some(error) = peer.live.as_ref().and_then(|live| live.error.as_deref()) {
            out.log(&format!("      last error: {error}"));
        }
    }
    out.log("");
    if !observed.running {
        out.log("No Desktop is running, so nothing is connected right now: a paired desktop");
        out.log("connects when one starts.");
    } else if !observed.answered {
        out.log("The running Desktop did not answer, so the STATE column is unknown.");
    } else {
        out.log(
            "`future remote peers connect|disconnect|takeover <id|#>` changes one; `remove` \
             unpairs it.",
        );
    }
    Ok(())
}

/// What one row says about the connection, or that it cannot say.
fn state_of(peer: &Peer, running: bool) -> String {
    if peer.held() {
        // A machine answering every heartbeat while another paired device drives
        // it is connected on the wire and useless in fact: "connected" one line
        // above a list of commands that all come back `superseded` is the wording
        // this column exists to avoid.
        return match peer
            .live
            .as_ref()
            .and_then(|live| live.holder.as_ref())
            .and_then(|holder| holder.name.as_deref())
        {
            Some(name) => format!("taken over by {name}"),
            None => "taken over".to_string(),
        };
    }
    match &peer.live {
        Some(live) if live.connected => "connected".to_string(),
        Some(live) if live.stopped => "disconnected".to_string(),
        Some(_) => "not connected".to_string(),
        // No answer from a Desktop that is not running is not "unknown": a
        // connection lives in that process, so there is none.
        None if !running => "not connected".to_string(),
        None => "unknown".to_string(),
    }
}

/// One row as a script reads it: the whole row, and `null` where the book cannot
/// answer.
fn row_document(peer: &Peer) -> Value {
    json!({
        "desktopId": peer.desktop_id,
        "pairId": peer.pair_id,
        "name": peer.name,
        "icon": peer.icon,
        "connected": peer.live.as_ref().map(|live| live.connected),
        "stopped": peer.live.as_ref().map(|live| live.stopped),
        "error": peer.live.as_ref().and_then(|live| live.error.clone()),
        "agentAvailable": peer.live.as_ref().map(|live| live.agent_available),
        // The same two fields the window reads off `PeerSummary.control` and
        // `.holder`, so a script can tell "connected" from "connected but held".
        "control": peer.live.as_ref().and_then(|live| live.control.clone()),
        "holder": peer
            .live
            .as_ref()
            .and_then(|live| live.holder.as_ref())
            .map(|holder| json!({ "name": holder.name, "kind": holder.kind })),
    })
}

/// The book's rows, with whatever the Desktop answered attached by machine id.
///
/// A row the Desktop knows but the book does not is kept: the two are read a
/// moment apart, and a pairing made in between is real.
fn merged(book: &Book, observed: &Observed) -> Vec<Peer> {
    let mut rows: Vec<Peer> = book
        .peers
        .iter()
        .map(|peer| {
            let mut row = peer.clone();
            row.live = observed
                .peers
                .iter()
                .find(|live| live.desktop_id == peer.desktop_id)
                .and_then(|live| live.live.clone());
            row
        })
        .collect();
    for extra in &observed.peers {
        if !rows.iter().any(|row| row.desktop_id == extra.desktop_id) {
            rows.push(extra.clone());
        }
    }
    rows
}

/// What the running Desktop said, and whether it said anything.
#[derive(Debug, Default)]
struct Observed {
    running: bool,
    answered: bool,
    peers: Vec<Peer>,
}

/// Ask the running Desktop for its client-role state.
///
/// A read that cannot be answered is not an error: the book is still true about
/// *what is paired*, and the caller says which half is missing.
async fn observe(out: &Output) -> Observed {
    let running = desktop_running();
    let mut observed = Observed {
        running,
        answered: false,
        peers: Vec::new(),
    };
    if !running {
        return observed;
    }
    match request_desktop_naming(
        OP_PEER_STATE,
        None,
        None,
        "the list shows the stored pairings only",
        STATE_TIMEOUT,
    )
    .await
    {
        Ok(reply) => {
            if let Some(error) = reply.get("error").and_then(Value::as_str) {
                out.log_err(&format!(
                    "note: the running Desktop did not report its connections ({error}); the \
                     list shows the stored pairings only."
                ));
                return observed;
            }
            observed.answered = true;
            observed.peers = reply
                .get("peers")
                .and_then(Value::as_array)
                .map(|peers| peers.iter().filter_map(Peer::from_live).collect())
                .unwrap_or_default();
        }
        Err(reason) => out.log_err(&format!(
            "note: the running Desktop did not answer ({reason}); the list shows the stored \
             pairings only."
        )),
    }
    observed
}

// -- add --------------------------------------------------------------------

/// `future remote peers add <invitation> [--json]`
///
/// The pairing runs here rather than in the Desktop, unlike every other action
/// in this group: it is a claim against the platform plus one Noise handshake
/// with the other machine, and nothing local has to be serving for it. That is
/// what makes this command useful on a machine whose Desktop is not open - pair
/// now, and the Desktop connects it on its next start.
async fn add(args: &[String], out: &Output) -> Result<(), String> {
    let mut values = positional(args, &["--json"], help::REMOTE_PEERS_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let invitation = match values.len() {
        1 => values.remove(0),
        0 => {
            return Err(format!(
                "Which invitation? Paste the complete `futureos://remote/pair?...` link from the \
                 other machine.\n\n{}",
                help::REMOTE_PEERS_USAGE
            ))
        }
        count => {
            return Err(format!(
                "One invitation at a time, please: that is {count} arguments. Quote the whole \
                 link if it contains spaces.\n\n{}",
                help::REMOTE_PEERS_USAGE
            ))
        }
    };
    // The peer runtime spawns its supervisors through the shared host runtime,
    // which refuses to start without an installed handle ("entry point must
    // install the host runtime"). In the Desktop that is the app's runtime; in
    // this process it is the runtime this command is awaited on.
    install_runtime();
    let paired = future_host::remote_peer::runtime::pair_with_emitter(&invitation, None)
        .await
        .map_err(|error| describe_pairing_error(&error))?;
    // Connecting once is what makes this answer "it works"; saying goodbye is
    // what keeps the other machine's Remote page from drawing a client that is
    // gone for the length of a lease. The Desktop takes over from here.
    let peer = Peer {
        desktop_id: paired.desktop_id.clone(),
        pair_id: paired.pair_id.clone(),
        name: paired.name.clone(),
        icon: paired.icon.clone(),
        live: Some(Live {
            connected: paired.connected,
            stopped: paired.stopped,
            error: paired.error.clone(),
            agent_available: paired.agent_available,
            // A freshly paired machine is driven by this one, and the handshake
            // has just said so; a state this build does not know is left out
            // rather than guessed at.
            control: paired
                .control
                .and_then(|control| serde_json::to_value(control).ok())
                .and_then(|control| control.as_str().map(str::to_string)),
            holder: None,
        }),
    };
    let _ = future_host::remote_peer::runtime::shutdown(Duration::from_secs(3)).await;
    let running = desktop_running();
    if json_flag {
        out.log(&pretty(json!({
            "desktopRunning": running,
            "paired": row_document(&peer),
        }))?);
        return Ok(());
    }
    out.log(&format!(
        "Paired with {}{}.",
        peer.desktop_id,
        peer.name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default()
    ));
    match &peer.live {
        Some(live) if live.connected => out.log("Connected to it now."),
        Some(live) => out.log(&format!(
            "The pairing is stored, but the connection did not come up{}. It will be retried in \
             the background.",
            live.error
                .as_deref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )),
        None => (),
    }
    if running {
        out.log("The Desktop running here keeps it connected from now on.");
    } else {
        out.log("No Desktop is running: the pairing is stored, and it connects when one starts");
        out.log("(the window; a host serves devices but does not drive other desktops).");
    }
    Ok(())
}

/// Turn a pairing failure into a sentence that says what to do about it.
///
/// The link failures carry `PA002` support codes and machine-readable text
/// (`pairing_link_expired`, ...), because both the phone and this desktop name
/// the same fault the same way. A terminal has room to explain, and the fix for
/// every link failure is the same errand on the other machine, so this does.
fn describe_pairing_error(error: &future_host::Error) -> String {
    // A platform-minted invitation is claimed with this machine's account, so
    // "not signed in" arrives as an account fault rather than a link fault. The
    // command that fixes it is not visible from here otherwise.
    if matches!(error, future_host::Error::NotSignedIn) {
        return "This machine is not signed in to Future OS, and that invitation was minted by \
                the Future platform, so claiming it needs an account: run `future auth login`, \
                then paste the invitation again. (An invitation from a self-hosted relay needs \
                no account.)"
            .to_string();
    }
    if let future_host::Error::Remote { code, message, .. } = error {
        let advice = match message.as_str() {
            "pairing_link_expired" => Some(
                "The invitation has expired. Invitations are short-lived and single-use: ask the \
                 other machine for a fresh one and paste that.",
            ),
            "pairing_link_malformed"
            | "pairing_link_invalid_field"
            | "pairing_link_unsupported_version" => Some(
                "That is not a usable `futureos://remote/pair` link. Copy the whole thing, \
                 including everything after `?`; a link that lost a parameter cannot be repaired \
                 here.",
            ),
            _ => None,
        };
        if let Some(advice) = advice {
            return format!(
                "{advice} ({}, {message})",
                code.as_deref().unwrap_or("PA002")
            );
        }
    }
    format!("Pairing failed: {error}")
}

// -- connect / disconnect ---------------------------------------------------

/// `future remote peers connect|disconnect <id|#> [--json]`
async fn reach(connect: bool, args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json"], help::REMOTE_PEERS_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let selector = one_device(values, help::REMOTE_PEERS_USAGE)?;
    let book = Book::load()?;
    if book.peers.is_empty() {
        return Err(
            "This machine is not paired with any remote desktop. Add one with `future remote \
             peers add <invitation>`."
                .to_string(),
        );
    }
    let peer = book.peers[book.position(&selector)?].clone();
    let verb = if connect { "connect" } else { "disconnect" };
    if !desktop_running() {
        return Err(format!(
            "No Desktop is running for {} - a connection lives in that process, so there is \
             nothing to {verb} here. Start the Desktop (the window) and run this again; a paired \
             machine is connected by the Desktop as it starts{}.",
            future_dir()?.display(),
            if connect {
                ", so an unstarted one is not a fact about the pairing"
            } else {
                ""
            }
        ));
    }
    let op = if connect {
        OP_PEER_CONNECT
    } else {
        OP_PEER_DISCONNECT
    };
    let reply = request_desktop_naming(
        op,
        None,
        Some(&peer.desktop_id),
        &format!(
            "the machine is still {}",
            if connect { "disconnected" } else { "connected" }
        ),
        PEER_TIMEOUT,
    )
    .await?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(format!("The Desktop could not {verb} it: {error}"));
    }
    let peers = peers_of(&reply);
    let row = peers
        .iter()
        .find(|candidate| candidate.desktop_id == peer.desktop_id)
        .cloned()
        .unwrap_or(peer.clone());
    let live = row.live.clone().unwrap_or_default();
    if json_flag {
        out.log(&pretty(json!({
            "desktopRunning": true,
            "answered": true,
            "peers": peers.iter().map(row_document).collect::<Vec<_>>(),
        }))?);
    }
    if connect {
        if live.connected {
            out.log(&format!("{} is connected.", row.label()));
        } else {
            out.log(&format!(
                "{} is still not connected{}.",
                row.label(),
                live.error
                    .as_deref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            ));
            // The machine did not end up how it was asked, so the exit code says
            // so - the same rule `future remote enable` follows for a start that
            // came up off.
            out.set_exit_code(1);
        }
    } else if live.connected {
        out.log(&format!(
            "{} is still connected: the Desktop did not drop it.",
            row.label()
        ));
        out.set_exit_code(1);
    } else {
        out.log(&format!(
            "{} is disconnected. The pairing stays; `future remote peers connect {}` brings it \
             back, and this Desktop will not reconnect it on its own until then.",
            row.label(),
            row.desktop_id
        ));
    }
    Ok(())
}

// -- takeover ---------------------------------------------------------------

/// `future remote peers takeover <id|#> [--json]`
///
/// Takes the machine back from whichever paired device is driving it. The move
/// happens on the *other* machine — it is the same `takeover` command a phone
/// sends — so this asks the running Desktop, which owns the one live connection
/// that can carry it, and prints the host's own answer: a machine this one
/// already drove answers `tookOver: false`, which is a result, not a failure.
///
/// The reply's peer list is what the Desktop holds *now*, which is a presence
/// tick behind the move: the host announces control on its own channel, and
/// this side learns the new state from the next tick. So the sentence about the
/// move comes from the answer, not from re-reading the row.
async fn takeover(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json"], help::REMOTE_PEERS_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let selector = one_device(values, help::REMOTE_PEERS_USAGE)?;
    let book = Book::load()?;
    if book.peers.is_empty() {
        return Err(
            "This machine is not paired with any remote desktop. Add one with `future remote \
             peers add <invitation>`."
                .to_string(),
        );
    }
    let peer = book.peers[book.position(&selector)?].clone();
    if !desktop_running() {
        return Err(format!(
            "No Desktop is running for {} - the connection that would carry the takeover lives \
             in that process, and so does this machine's own control state. Start the Desktop \
             (the window) and run this again.",
            future_dir()?.display()
        ));
    }
    let reply = request_desktop_naming(
        OP_PEER_TAKEOVER,
        None,
        Some(&peer.desktop_id),
        "the machine was not taken over",
        PEER_TIMEOUT,
    )
    .await?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(format!("The Desktop could not take it over: {error}"));
    }
    let peers = peers_of(&reply);
    let row = peers
        .iter()
        .find(|candidate| candidate.desktop_id == peer.desktop_id)
        .cloned()
        .unwrap_or(peer.clone());
    // A Desktop that answered without the field (an older build) moved control:
    // it answers an error for every way the move can fail.
    let moved = reply
        .get("tookOver")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if json_flag {
        out.log(&pretty(json!({
            "desktopRunning": true,
            "answered": true,
            "tookOver": moved,
            "peers": peers.iter().map(row_document).collect::<Vec<_>>(),
        }))?);
        return Ok(());
    }
    if moved {
        out.log(&format!(
            "{} is now driven by this machine: the device that had it was told to stand by.",
            row.label()
        ));
    } else {
        out.log(&format!(
            "{} was already driven by this machine, so nothing moved.",
            row.label()
        ));
    }
    Ok(())
}

/// The peer rows in a Desktop answer.
fn peers_of(reply: &Value) -> Vec<Peer> {
    reply
        .get("peers")
        .and_then(Value::as_array)
        .map(|peers| peers.iter().filter_map(Peer::from_live).collect())
        .unwrap_or_default()
}

// -- rename -----------------------------------------------------------------

/// `future remote peers rename <id|#> <name> [--json]`
///
/// Book only, and that is the whole truth: the name is this machine's private
/// label for the other one, never sent anywhere, and the Desktop reads it from
/// the same file on every list.
fn rename(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json", "--clear"], help::REMOTE_PEERS_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let clear = args.iter().any(|argument| argument == "--clear");
    let mut values = values;
    let selector = match values.len() {
        0 => {
            return Err(format!(
                "Which machine? Give its id or its row number.\n\n{}",
                help::REMOTE_PEERS_USAGE
            ))
        }
        _ => values.remove(0),
    };
    let name = values.join(" ");
    if clear && !name.trim().is_empty() {
        return Err(format!(
            "--clear goes back to the machine id, so it cannot be given a name too. Give one or \
             the other.\n\n{}",
            help::REMOTE_PEERS_USAGE
        ));
    }
    if !clear && name.trim().is_empty() {
        return Err(format!(
            "What should it be called? Give a name, or --clear to go back to the machine id.\n\n{}",
            help::REMOTE_PEERS_USAGE
        ));
    }
    let mut book = Book::load()?;
    let desktop_id = book.peers[book.position(&selector)?].desktop_id.clone();
    let wanted = if clear {
        None
    } else {
        Some(name.trim().to_string())
    };
    book.edit(|book| {
        let Some(record) = book
            .document
            .get_mut("peers")
            .and_then(Value::as_array_mut)
            .and_then(|peers| {
                peers.iter_mut().find(|peer| {
                    peer.get("desktopId").and_then(Value::as_str) == Some(desktop_id.as_str())
                })
            })
        else {
            return Err(format!(
                "{desktop_id} is no longer paired with this machine: the book changed while this \
                 ran."
            ));
        };
        let object = record.as_object_mut().ok_or_else(|| {
            "The pairing book holds an entry that is not a JSON object.".to_string()
        })?;
        match &wanted {
            Some(name) => {
                object.insert("name".to_string(), json!(name));
            }
            None => {
                object.remove("name");
            }
        }
        // Re-read through the loader so the in-memory rows match the file.
        Ok(())
    })?;
    if json_flag {
        let book = Book::load()?;
        out.log(&pretty(json!({
            "desktopRunning": desktop_running(),
            "peers": book.peers.iter().map(row_document).collect::<Vec<_>>(),
        }))?);
        return Ok(());
    }
    out.log(&match &wanted {
        Some(name) => format!("{desktop_id} is called `{name}` here from now on."),
        None => format!("{desktop_id} is shown by its machine id again."),
    });
    Ok(())
}

// -- remove -----------------------------------------------------------------

/// `future remote peers remove <id|#> [--yes] [--json]`
async fn remove(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json", "--yes"], help::REMOTE_PEERS_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let yes = args.iter().any(|argument| argument == "--yes");
    let selector = one_device(values, help::REMOTE_PEERS_USAGE)?;
    let book = Book::load()?;
    if book.peers.is_empty() {
        return Err("This machine is not paired with any remote desktop.".to_string());
    }
    let peer = book.peers[book.position(&selector)?].clone();
    if !yes {
        let remaining = book.peers.len() - 1;
        let confirmed = ask(&format!(
            "Remove the pairing with {}? This machine stops connecting to it; {} other pairing{} \
             stay{}.",
            peer.label(),
            remaining,
            if remaining == 1 { "" } else { "s" },
            if remaining == 1 { "s" } else { "" }
        ))?;
        if !confirmed {
            out.log_err("Cancelled. Nothing was changed.");
            return Ok(());
        }
    }
    let mut answered = false;
    if desktop_running() {
        match request_desktop_naming(
            OP_PEER_REMOVE,
            None,
            Some(&peer.desktop_id),
            "it is still paired",
            PEER_TIMEOUT,
        )
        .await
        {
            Ok(reply) if reply.get("removed").and_then(Value::as_bool) == Some(true) => {
                if json_flag {
                    out.log(&pretty(json!({
                        "desktopRunning": true,
                        "answered": true,
                        "removed": peer.desktop_id,
                        "peers": peers_of(&reply).iter().map(row_document).collect::<Vec<_>>(),
                    }))?);
                }
                out.log(&format!(
                    "{} is no longer paired with this machine.",
                    peer.label()
                ));
                return Ok(());
            }
            Ok(reply) => {
                answered = true;
                let reason = reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("it answered without removing it");
                out.log_err(&format!(
                    "note: the running Desktop did not remove it ({reason}); removing it here, so \
                     it is gone at the next start."
                ));
            }
            Err(reason) => out.log_err(&format!(
                "note: the running Desktop did not answer ({reason}); removing it here, so it is \
                 gone at the next start."
            )),
        }
    }
    // Local removal, and the platform told where it can be reached. The book is
    // what decides whether this machine ever connects to it again, so it is
    // written first and the revoke is best effort - exactly the contract the
    // window's own unpair follows.
    install_runtime();
    match future_host::remote_peer::runtime::unpair(&peer.desktop_id).await {
        Ok(note) => {
            if let Some(note) = note {
                out.log_err(&format!(
                    "warning: the pairing is gone here, but the platform could not be told \
                     ({note}). The other machine keeps it listed until its credential expires, \
                     and nothing here will connect to it."
                ));
            }
        }
        Err(error) => {
            return Err(format!(
                "Could not remove the pairing: {error}. The book is unchanged."
            ))
        }
    }
    let book = Book::load()?;
    if json_flag {
        out.log(&pretty(json!({
            "desktopRunning": desktop_running(),
            "answered": answered,
            "removed": peer.desktop_id,
            "peers": book.peers.iter().map(row_document).collect::<Vec<_>>(),
        }))?);
        return Ok(());
    }
    out.log(&format!(
        "{} is no longer paired with this machine.",
        peer.label()
    ));
    Ok(())
}

// -- shared ----------------------------------------------------------------

/// Give the shared host runtime its task owner before the client role runs here.
///
/// See the note in [`add`]: the peer runtime spawns through
/// `future_host::runtime`, which panics with no installed handle, and in this
/// process the handle is the runtime the command is awaited on.
fn install_runtime() {
    future_host::runtime::set(tokio::runtime::Handle::current());
}

fn pretty(value: Value) -> Result<String, String> {
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
