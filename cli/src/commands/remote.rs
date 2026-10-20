//! `future remote devices` — the devices paired to this machine's remote
//! desktop, from a terminal.
//!
//! The window's Remote page manages the same list through Tauri commands; a
//! headless server has no window, and its only knob used to be
//! `futureos-headless --re-pair`, which revoked *every* device and issued a
//! replacement invitation. That answered "switch phones" and nothing else: a
//! machine with several paired clients could not say who they were, could not
//! drop one of them, and could not choose which one drives it.
//!
//! # What is authoritative
//!
//! For everything except `add`, the pairing book on disk
//! (`~/.future/remote_pairings.json`, or the pre-multi-device
//! `remote_pairing.json` read as a book with one slot) is authoritative: it is
//! what the Desktop itself reads at start and on every command path. That makes
//! it the honest thing for this command to read and edit, and it means those
//! actions need nothing running.
//!
//! `add` and `remove` cannot work that way while a Desktop is running. An
//! invitation is only usable while the bridge for its pairing is connected, so it
//! has to be minted by the process that serves the machine — the window, or
//! `futureos-headless`. A removal is the mirror image: the bridge is what a device
//! talks to, and only that process can stop it. A credential is a bearer JWT, valid
//! until it lapses, so a book edited behind a running Desktop leaves the device
//! served for minutes after the user was told it was unpaired. Both requests go to
//! the Desktop as a file and come back as one (see [`request_desktop`]); with no
//! Desktop running, `remove` edits the book itself, which is what the next start
//! reads.
//!
//! The document is parsed and rewritten as a [`serde_json::Value`] rather than a
//! mirrored struct: the fields this command does not understand (the NKey seed,
//! the handshake identity, whatever a newer Desktop adds) must survive an edit
//! untouched. A typed copy here would silently drop them.
//!
//! # What a running Desktop does with an edit
//!
//! - **Remove** is asked of the Desktop first, and its answer is what makes the
//!   removal immediate: the supervisor revokes the credential, forgets the slot and
//!   stops serving that device now. Without it (no Desktop, or one too old to know
//!   the request) this command edits the book, which the next start reads — and the
//!   device keeps being served until its credential comes up for renewal, because
//!   the phone's JWT outlives the edit. Either way the revoke is ordered before the
//!   local slot is dropped, so a lost write cannot resurrect the device.
//! - **Use** is persisted for the next start. Control is an in-memory pointer in
//!   the running process (a device taking over must move it too), so a running
//!   Desktop keeps its current controller until it restarts.
//! - **Add** is the Desktop's own action: it mints the invitation and starts the
//!   bridge for it before answering, so the code works the moment it is printed.
//!   Existing devices stay paired. When the book is empty (a `remove --all` just
//!   ran, possibly while the old bridges were still connected), the Desktop takes
//!   its replace path first, which stops them before minting.
//!
//! A revoke that cannot be delivered is queued where the Desktop already looks
//! (`remote_pending_revokes.json`), so stopping here never loses it.

use crate::constants::{auth_file, DEFAULT_PLATFORM_URL};
use crate::help;
use crate::output::Output;
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The multi-device credential book (`~/.future/remote_pairings.json`).
const BOOK_FILE: &str = "remote_pairings.json";
/// The single-pairing file older builds wrote, still read as a book with one
/// slot so a downgrade or a not-yet-upgraded install keeps working.
const LEGACY_FILE: &str = "remote_pairing.json";
/// Revokes the Desktop (or this command) could not deliver, retried by the next
/// Desktop that runs. Same file and same shape as `pairing::queue_revoke`.
const PENDING_REVOKES_FILE: &str = "remote_pending_revokes.json";
/// The lock the Desktop holds for the lifetime of the process; holding it is
/// exactly "a Desktop owns this data directory" (`instance.rs`).
const DESKTOP_LOCK_FILE: &str = "desktop.lock";
/// The self-hosted relay this installation points at, and the CA it needs
/// (`relay.rs`'s `SETTINGS_FILE`).
const RELAY_SETTINGS_FILE: &str = "relay.json";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// `future remote <command> [args]`.
pub async fn remote(command: Option<&str>, rest: &[String], out: &Output) -> Result<(), String> {
    match command {
        None | Some("--help" | "-h") => {
            out.log(help::REMOTE_HELP);
            Ok(())
        }
        Some("devices") => devices(rest, out).await,
        Some(other) => Err(format!(
            "Unknown argument: {other}\n\n{}",
            help::REMOTE_USAGE
        )),
    }
}

/// Everything below the group, and above the four actions.
async fn devices(args: &[String], out: &Output) -> Result<(), String> {
    let (action, rest) = match args.first().map(String::as_str) {
        None => ("list", args),
        Some("--help" | "-h") => {
            out.log(help::REMOTE_HELP);
            return Ok(());
        }
        Some("list" | "add" | "remove" | "use") => (args[0].as_str(), &args[1..]),
        // A flag before any action belongs to the default one, so
        // `future remote devices --json` is a list (and `--bogus` is reported by
        // the action that would have taken it, not as an unknown command).
        Some(flag) if flag.starts_with('-') => ("list", args),
        Some(argument) => {
            return Err(format!(
                "Unknown argument: {argument}\n\n{}",
                help::REMOTE_USAGE
            ))
        }
    };
    if rest
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        out.log(match action {
            "list" => help::REMOTE_LIST_HELP,
            "add" => help::REMOTE_ADD_HELP,
            "remove" => help::REMOTE_REMOVE_HELP,
            _ => help::REMOTE_USE_HELP,
        });
        return Ok(());
    }
    match action {
        "list" => list(rest, out),
        "add" => add(rest, out).await,
        "remove" => remove(rest, out).await,
        _ => take_control(rest, out),
    }
}

// ── The pairing book ────────────────────────────────────────────────────────

/// `~/.future/` — the root the Desktop writes its pairing files into.
fn future_dir() -> Result<PathBuf, String> {
    future_app_settings::app_dir()
        .map_err(|error| error.to_string())?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "The Desktop data directory has no parent".to_string())
}

/// One paired device, with only the fields the terminal shows or needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Device {
    pair_id: String,
    desktop_id: Option<String>,
    /// The self-hosted relay that minted this pairing; `None` is the official
    /// platform, which is what every credential written before relays says by
    /// omission.
    control_url: Option<String>,
    /// Never printed or logged — kept because a relay pairing's room can only
    /// be deleted with a request signed by this key.
    nkey_seed: Option<String>,
}

impl Device {
    fn from_value(value: &Value, file: &Path) -> Result<Self, String> {
        let object = value.as_object().ok_or_else(|| {
            format!(
                "{}: every entry in `slots` must be a JSON object.",
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
        let pair_id = text("pairId").ok_or_else(|| {
            format!(
                "{}: an entry in `slots` has no `pairId`, so there is no device to name.",
                file.display()
            )
        })?;
        Ok(Self {
            pair_id,
            desktop_id: text("desktopId"),
            control_url: text("controlUrl"),
            nkey_seed: text("nkeySeed"),
        })
    }

    fn endpoint_label(&self) -> String {
        match self.control_url.as_deref() {
            Some(url) => format!("relay {url}"),
            None => "Future platform".to_string(),
        }
    }

    fn is_relay(&self) -> bool {
        self.control_url.is_some()
    }
}

/// The parsed pairing book, plus the document to write back.
struct Book {
    path: PathBuf,
    /// `remote_pairing.json`, which holds exactly one credential and is removed
    /// rather than rewritten.
    legacy: bool,
    /// The whole file, so an edit keeps every field this command does not model.
    document: Value,
    active: Option<String>,
    devices: Vec<Device>,
}

impl Book {
    fn empty(path: PathBuf) -> Self {
        Self {
            path,
            legacy: false,
            document: json!({ "version": 2, "active": Value::Null, "slots": [] }),
            active: None,
            devices: Vec::new(),
        }
    }

    fn load() -> Result<Self, String> {
        let future = future_dir()?;
        let book = future.join(BOOK_FILE);
        if book.is_file() {
            return Self::read(&book, false);
        }
        let legacy = future.join(LEGACY_FILE);
        if legacy.is_file() {
            return Self::read(&legacy, true);
        }
        Ok(Self::empty(book))
    }

    fn read(path: &Path, legacy: bool) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("Cannot read the pairing book {}: {error}", path.display()))?;
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
        // A pre-multi-device file *is* one credential: read it as a book with a
        // single slot, exactly as the Desktop does, and keep it in that shape on
        // disk until something removes the device.
        let slots = match (legacy, document.get("slots")) {
            (true, _) => vec![Device::from_value(&document, path)?],
            (false, Some(Value::Array(slots))) => slots
                .iter()
                .map(|slot| Device::from_value(slot, path))
                .collect::<Result<Vec<_>, _>>()?,
            (false, None) => Vec::new(),
            (false, Some(_)) => {
                return Err(format!(
                    "{}: `slots` must be an array of paired devices.",
                    path.display()
                ))
            }
        };
        let active = if legacy {
            slots.first().map(|slot| slot.pair_id.clone())
        } else {
            document
                .get("active")
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        Ok(Self {
            path: path.to_path_buf(),
            legacy,
            document,
            active,
            devices: slots,
        })
    }

    /// The device that drives the machine, by the same rule the runtime uses: the
    /// persisted pointer, else the only device there is (with one device there is
    /// no arbitration to do). A pointer naming a device that is no longer here
    /// controls nothing — the runtime clears it on the next start.
    fn controller(&self) -> Option<&Device> {
        match self.active.as_deref() {
            Some(pair_id) => self.devices.iter().find(|device| device.pair_id == pair_id),
            None => match self.devices.as_slice() {
                [only] => Some(only),
                _ => None,
            },
        }
    }

    /// Which row the user means: an exact id first (so a device whose id is all
    /// digits can still be named), then the 1-based row number this command
    /// prints, then a unique prefix.
    fn position(&self, selector: &str) -> Result<usize, String> {
        let wanted = normalized(selector);
        if let Some(index) = self
            .devices
            .iter()
            .position(|device| normalized(&device.pair_id) == wanted)
        {
            return Ok(index);
        }
        if let Some(index) = selector
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|index| *index >= 1 && *index <= self.devices.len())
        {
            return Ok(index - 1);
        }
        let matches: Vec<usize> = self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, device)| normalized(&device.pair_id).starts_with(&wanted))
            .map(|(index, _)| index)
            .collect();
        match matches.as_slice() {
            [only] => Ok(*only),
            [] => Err(format!(
                "No device matches `{selector}`. Run `future remote devices` to see the paired \
                 devices."
            )),
            _ => Err(format!(
                "`{selector}` matches {} devices. Give a longer prefix.",
                matches.len()
            )),
        }
    }

    /// Forget one device, in the same shape `pairing::remove_slot` leaves behind.
    fn remove(&mut self, index: usize) {
        let removed = self.devices.remove(index);
        if self.active.as_deref() == Some(removed.pair_id.as_str()) {
            self.active = None;
        }
        // The legacy file describes one credential and is never rewritten: it is
        // either that device (untouched) or gone.
        if self.legacy {
            return;
        }
        if let Some(slots) = self.document.get_mut("slots").and_then(Value::as_array_mut) {
            slots.retain(|slot| {
                slot.get("pairId").and_then(Value::as_str) != Some(removed.pair_id.as_str())
            });
        }
        if let Some(document) = self.document.as_object_mut() {
            document.insert(
                "active".to_string(),
                self.active.clone().map_or(Value::Null, Value::String),
            );
        }
    }

    fn set_controller(&mut self, pair_id: &str) {
        // A legacy file has one device, which controls by virtue of being alone;
        // there is nothing to write and its shape must stay one credential.
        if self.legacy {
            return;
        }
        self.active = Some(pair_id.to_string());
        if let Some(document) = self.document.as_object_mut() {
            document.insert("active".to_string(), json!(pair_id));
        }
    }

    /// Persist the book. An emptied book leaves no credential file behind — the
    /// same rule the Desktop applies, because "no devices" must not keep
    /// answering "there is a credential here".
    fn save(&self) -> Result<(), String> {
        if self.legacy || self.devices.is_empty() {
            for path in [&self.path, &self.path.with_file_name(LEGACY_FILE)] {
                if path.exists() {
                    std::fs::remove_file(path)
                        .map_err(|error| format!("Cannot remove {}: {error}", path.display()))?;
                }
            }
            return Ok(());
        }
        write_json(&self.path, &self.document)
    }
}

/// Compare ids the way a person copies them: case-insensitively, and with the
/// `pair_` prefix the platform sometimes adds treated as decoration — the same
/// normalisation the window's device list shows.
fn normalized(pair_id: &str) -> String {
    let trimmed = pair_id.trim();
    trimmed
        .strip_prefix("pair_")
        .unwrap_or(trimmed)
        .to_ascii_uppercase()
}

fn display_pair_id(pair_id: &str) -> String {
    normalized(pair_id)
}

/// Atomic JSON write, `0600`, exactly like the Desktop's config writer: a
/// pairing file is a credential, and a half-written one would make the next
/// start fail rather than pair.
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let serialized = format!(
        "{}\n",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    let directory = path.parent().ok_or_else(|| {
        format!(
            "Cannot write {}: it has no parent directory.",
            path.display()
        )
    })?;
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("Cannot create {}: {error}", directory.display()))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".remote-devices-")
        .suffix(".tmp")
        .tempfile_in(directory)
        .map_err(|error| format!("Cannot write beside {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Cannot restrict {}: {error}", path.display()))?;
    }
    temp.write_all(serialized.as_bytes())
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|error| format!("Cannot write {}: {error}", path.display()))?;
    temp.persist(path)
        .map_err(|error| format!("Cannot replace {}: {}", path.display(), error.error))?;
    Ok(())
}

// ── Is a Desktop holding these pairings right now? ──────────────────────────

/// Whether a Desktop (windowed or headless) owns this data directory.
///
/// Only asked to explain when a change takes effect — never to refuse one. A
/// missing lock file is "nothing is running" without creating it, so a
/// read-only `list` does not invent an app directory.
fn desktop_running() -> bool {
    let Ok(directory) = future_app_settings::app_dir() else {
        return false;
    };
    let path = directory.join(DESKTOP_LOCK_FILE);
    if !path.is_file() {
        return false;
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
    else {
        return false;
    };
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            false
        }
        Err(_) => true,
    }
}

// ── list ────────────────────────────────────────────────────────────────────

fn list(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json"], help::REMOTE_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    if let Some(extra) = values.first() {
        return Err(format!(
            "`list` takes no device: it lists them all (drop `{extra}`).\n\n{}",
            help::REMOTE_USAGE
        ));
    }
    let book = Book::load()?;
    let running = desktop_running();
    if json_flag {
        out.log(
            serde_json::to_string_pretty(&document(&book, running))
                .map_err(|error| error.to_string())?
                .as_str(),
        );
        return Ok(());
    }
    if book.devices.is_empty() {
        out.log("No devices are paired to this machine.");
        out.log("Start the Desktop (windowed or futureos-headless) to pair one.");
        return Ok(());
    }
    let controller = book.controller().map(|device| device.pair_id.clone());
    let ids: Vec<String> = book
        .devices
        .iter()
        .map(|device| display_pair_id(&device.pair_id))
        .collect();
    let id_width = ids.iter().map(String::len).max().unwrap_or(0).max(6);
    out.log(&format!(
        "{} {} paired to this machine:",
        book.devices.len(),
        if book.devices.len() == 1 {
            "device is"
        } else {
            "devices are"
        }
    ));
    out.log("");
    out.log(&format!(
        "  {:>2}  {:<id_width$}  {:<10}  {}",
        "#", "DEVICE", "CONTROL", "ENDPOINT"
    ));
    for (index, device) in book.devices.iter().enumerate() {
        let control = if controller.as_deref() == Some(device.pair_id.as_str()) {
            "in control"
        } else {
            "standby"
        };
        out.log(&format!(
            "  {:>2}  {:<id_width$}  {:<10}  {}",
            index + 1,
            ids[index],
            control,
            device.endpoint_label()
        ));
    }
    out.log("");
    out.log("The device in control is the one allowed to command this machine; the others stay");
    out.log("paired in standby.");
    if running {
        out.log("");
        out.log("A Desktop is running and is serving these pairings.");
    }
    Ok(())
}

/// The machine-readable document `--json` prints, and the same shape a mutation
/// prints for the state it leaves behind.
fn document(book: &Book, running: bool) -> Value {
    let controller = book.controller().map(|device| device.pair_id.clone());
    json!({
        "desktopRunning": running,
        "active": controller,
        "devices": book
            .devices
            .iter()
            .map(|device| json!({
                "pairId": device.pair_id,
                "desktopId": device.desktop_id,
                "control": if controller.as_deref() == Some(device.pair_id.as_str()) {
                    "active"
                } else {
                    "standby"
                },
                "endpoint": if device.is_relay() { "relay" } else { "platform" },
                "relayUrl": device.control_url,
            }))
            .collect::<Vec<_>>(),
    })
}

// ── add ─────────────────────────────────────────────────────────────────────

/// How long a running Desktop gets to answer.
///
/// Sized for the slow path: minting the invitation is one platform round trip
/// (20s request timeout), and the bridge for the new device is started before the
/// answer, so a slow network can hold this for a while. A terminal that gives up
/// first is no loss — the invitation expires on its own.
#[cfg(not(test))]
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Tests answer from an in-process fake, so the wait only has to be long enough
/// for two tasks on the same runtime to take turns.
#[cfg(test)]
const REPLY_TIMEOUT: Duration = Duration::from_millis(400);

/// Between two reads of the answer file.
#[cfg(not(test))]
const REPLY_POLL: Duration = Duration::from_millis(100);
#[cfg(test)]
const REPLY_POLL: Duration = Duration::from_millis(5);

/// The directory the running Desktop watches (`~/.future/run/remote-control`).
///
/// Under `run/` on purpose: this is transient runtime state, like the agent's own
/// socket beside it, and nothing here should ever be backed up or restored with
/// the data directory — a restored request would be answered by a Desktop whose
/// asking terminal is long gone.
const CONTROL_DIRECTORY: &str = "remote-control";
/// Suffix that marks a request, matching the Desktop's own rule.
const REQUEST_SUFFIX: &str = ".request.json";
/// Serializes `add` across terminals (`exclusive_add_slot`).
const ADD_LOCK_FILE: &str = "add.lock";

/// `future remote devices add` — mint an invitation for a device that is not
/// paired yet.
///
/// The request goes to the running Desktop rather than being performed here: an
/// invitation is only usable while a bridge for its pairing is connected, and
/// that bridge lives in the Desktop process that owns this data directory. So
/// this needs a Desktop running — the window, or `futureos-headless` — and on a
/// machine that has never paired, starting it already prints an invitation.
async fn add(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json"], help::REMOTE_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    if let Some(extra) = values.first() {
        return Err(format!(
            "`add` takes no argument: it issues an invitation, and the device joins by \
             scanning it (drop `{extra}`).\n\n{}",
            help::REMOTE_USAGE
        ));
    }
    if !desktop_running() {
        return Err(format!(
            "No Desktop is running for {} — a new device can only be paired by the process \
             that serves it. Start the Desktop (the window, or futureos-headless) and run this \
             again. A machine with no device at all prints an invitation as it starts.",
            future_dir()?.display()
        ));
    }
    // One invitation at a time. Two `add`s in flight would each mint a device —
    // both plausible, but the second is almost always a double-tap or a shell
    // retry, and on a platform that caps the number of devices it can cost the
    // slot the user meant for something else. Refusing is also the only honest
    // answer: the terminal that loses would otherwise wait out its budget for a
    // code nobody is looking at.
    let _one_at_a_time = exclusive_add_slot()?;
    let reply = request_invitation().await?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(format!("The Desktop did not issue an invitation: {error}"));
    }
    let link = reply
        .get("link")
        .and_then(Value::as_str)
        .ok_or_else(|| "The Desktop answered without an invitation.".to_string())?;
    let pair_id = reply.get("pairId").and_then(Value::as_str).unwrap_or("");
    let expires_at = reply.get("expiresAt").and_then(Value::as_i64);
    // Invitations the Desktop had to forget so this one could be minted. Said
    // out loud: the platform's per-desktop cap was full, and four codes the user
    // may still remember asking for are gone. Silence here would be the same
    // "nothing happened" this whole path exists to end.
    let cleared: Vec<&str> = reply
        .get("cleared")
        .and_then(Value::as_array)
        .map(|cleared| cleared.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if json_flag {
        out.log(
            serde_json::to_string_pretty(&json!({
                "pairId": pair_id,
                "link": link,
                "expiresAt": expires_at,
                "cleared": cleared,
            }))
            .map_err(|error| error.to_string())?
            .as_str(),
        );
        return Ok(());
    }
    out.write_out(&invitation_text(
        &format!(
            "PAIRING INVITATION{} - open the FutureOS app and scan or paste this complete link",
            if pair_id.is_empty() {
                String::new()
            } else {
                format!(" for {}", display_pair_id(pair_id))
            }
        ),
        link,
        terminal_columns(),
    ));
    out.log("");
    match expires_at {
        Some(expiry) => {
            let when = deadline_phrase(expiry, now_seconds())
                // The Desktop's clock is far enough out that no calendar can
                // name the day; the raw epoch at least says how far.
                .unwrap_or_else(|| format!("Unix time {expiry}"));
            out.log(&format!(
                "Single-use. It expires at {when}; the Desktop keeps serving the new device as \
                 soon as a phone scans it."
            ))
        }
        None => out.log(
            "Single-use. The Desktop keeps serving the new device as soon as a phone scans it.",
        ),
    }
    if !cleared.is_empty() {
        out.log(&format!(
            "\nThis desktop had no free device slot, so it cleared {} invitation{} no phone ever \
             claimed: {}.",
            cleared.len(),
            if cleared.len() == 1 { "" } else { "s" },
            cleared.join(", ")
        ));
    }
    Ok(())
}

/// A deadline as the reader's own clocks: the wall clock they can compare with
/// their watch, and the time left to act (`2026-10-20 09:45:32 (local time, in
/// 9m 58s)`). `None` for an instant no calendar can express.
///
/// Spelled the same way in `futureos-headless`, which prints an invitation of
/// its own — one wording, both entry points.
fn deadline_phrase(expiry: i64, now: i64) -> Option<String> {
    let clock = local_wall_clock(expiry)?;
    Some(match time_left(expiry, now) {
        Some(left) => format!("{clock} (local time, in {left})"),
        // Already passed, or this machine's clock is ahead of the Desktop's:
        // the deadline still names itself, and "in 0s" would only puzzle.
        None => format!("{clock} (local time)"),
    })
}

/// How long is left before `expiry`, in the largest unit worth naming:
/// `42s`, `9m 58s`, `2h 05m`. `None` once it has passed.
fn time_left(expiry: i64, now: i64) -> Option<String> {
    let left = expiry.checked_sub(now)?;
    if left <= 0 {
        return None;
    }
    let (hours, minutes, seconds) = (left / 3600, (left % 3600) / 60, left % 60);
    Some(if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    })
}

/// `SystemTime::now()` in whole seconds, the unit every deadline here uses.
fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

/// A deadline as the local clock the reader can look at, e.g.
/// `2026-10-20 09:45:32`, or `None` for an instant no calendar can express.
///
/// The zone is spelled out by the caller rather than taken from `%Z`: chrono
/// derives that from the platform, and it is inconsistent across Windows and
/// Unix. The range guard keeps `with_timezone` inside what the platform's time
/// zone database answers for.
fn local_wall_clock(unix: i64) -> Option<String> {
    let instant = chrono::DateTime::from_timestamp(unix, 0)
        .filter(|instant| (1970..=9999).contains(&chrono::Datelike::year(instant)))?;
    Some(
        instant
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    )
}

/// Hold the "one `add` at a time" lock for as long as this lives.
///
/// An advisory file lock, so a process that dies releases it: a stale lock would
/// otherwise refuse every later `add` until someone deleted a file by hand.
fn exclusive_add_slot() -> Result<std::fs::File, String> {
    let directory = control_directory()?;
    let path = directory.join(ADD_LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(file),
        Err(_) => Err(
            "Another `future remote devices add` is already waiting for an invitation. Finish \
             that one — it prints the code — or let it time out, then run this again."
                .to_string(),
        ),
    }
}

/// `~/.future/run/remote-control`, created if needed.
fn control_directory() -> Result<PathBuf, String> {
    let directory = future_dir()?.join("run").join(CONTROL_DIRECTORY);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Cannot create {}: {error}", directory.display()))?;
    Ok(directory)
}

/// What a timed-out request means, told apart by whether anything picked it up.
///
/// The two silences are different faults with different fixes, and a single
/// "did not answer" sends the user to the wrong one. A request still sitting in
/// the directory was never claimed: no process is watching it, which in practice
/// means the Desktop running for this data directory predates device requests —
/// it was started before the update and keeps serving from its old code until it
/// is restarted. A request that disappeared was claimed by a Desktop that then
/// failed or was killed mid-request. `unchanged` says what the silence left
/// behind, which is not the same sentence for an addition and a removal.
fn timeout_message(
    timeout: Duration,
    request_still_there: &dyn Fn() -> bool,
    unchanged: &str,
) -> String {
    if request_still_there() {
        format!(
            "No Desktop picked the request up within {}s — the one running for {} is probably an \
             older build that predates device requests (it was started before the update). \
             Restart it with the current version and try again; {unchanged}.",
            timeout.as_secs(),
            "this data directory",
        )
    } else {
        format!(
            "A Desktop took the request but did not answer within {}s. It may have failed or been \
             stopped while pairing; check its own output, then try again.",
            timeout.as_secs()
        )
    }
}

/// Ask the running Desktop to perform one device operation and return its answer.
///
/// `op` is the Desktop's own name for the operation (`add-device`,
/// `remove-device`); `pair_id` names the device for the operations about one.
/// `unchanged` completes the sentence about what the silence left behind: a
/// timeout that claimed "nothing was paired" after a removal would be a lie.
///
/// The reply path is a temporary file this process created for itself, so two
/// terminals asking at once cannot read each other's answer. Whether the request
/// was answered or not, both files are gone when this returns: a request left
/// behind would be served by the *next* Desktop into a file nobody reads.
async fn request_desktop(
    op: &str,
    pair_id: Option<&str>,
    unchanged: &str,
) -> Result<Value, String> {
    let directory = control_directory()?;
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let answer = std::env::temp_dir().join(format!(
        "future-remote-{op}-{started}-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&answer);
    // The name sorts by time, so the Desktop serves the oldest request first.
    let request = directory.join(format!(
        "{started:013}-{}{REQUEST_SUFFIX}",
        std::process::id()
    ));
    let mut body = json!({ "op": op, "reply": answer.display().to_string() });
    if let Some(pair_id) = pair_id {
        body["pairId"] = json!(pair_id);
    }
    write_json(&request, &body)?;
    let outcome = wait_for_answer(&answer, &|| request.exists(), &desktop_running, unchanged).await;
    let _ = std::fs::remove_file(&answer);
    let _ = std::fs::remove_file(&request);
    outcome
}

/// Ask the Desktop for an invitation.
async fn request_invitation() -> Result<Value, String> {
    request_desktop("add-device", None, "nothing was paired").await
}

/// Ask the Desktop to drop one device and stop serving it.
///
/// `Ok(true)` means it performed the removal (and wrote the book); `Ok(false)`
/// means it answered without doing so. `Err` is the reason it could not be
/// asked at all — an older Desktop, a Desktop that exited — and the caller
/// falls back to editing the book.
async fn ask_desktop_to_remove(device: &Device) -> Result<bool, String> {
    let reply = request_desktop(
        "remove-device",
        Some(&device.pair_id),
        "nothing was removed",
    )
    .await?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    Ok(reply
        .get("removed")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// Wait for the Desktop's answer, ending early if the Desktop stops.
///
/// `still_running` is injected so both outcomes are reachable without a real
/// Desktop or a real wait. A Desktop that exits mid-request is the one case the
/// timeout would otherwise *mislabel*: it would report "did not answer within
/// 60s" for a process that is gone, sending the user to look for a busy Desktop
/// that no longer exists.
async fn wait_for_answer(
    answer: &Path,
    request_still_there: &dyn Fn() -> bool,
    still_running: &dyn Fn() -> bool,
    unchanged: &str,
) -> Result<Value, String> {
    let deadline = Instant::now() + REPLY_TIMEOUT;
    loop {
        // A poll that lands before the writer has finished is "not yet", not a
        // failure: the answer is written atomically, so a parsed one is whole.
        if let Some(value) = std::fs::read_to_string(answer)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(timeout_message(
                REPLY_TIMEOUT,
                request_still_there,
                unchanged,
            ));
        }
        if !still_running() {
            return Err(format!(
                "The Desktop stopped before it answered. Start it again (the window, or \
                 futureos-headless) and re-run this; {unchanged}."
            ));
        }
        tokio::time::sleep(REPLY_POLL).await;
    }
}

/// Width of the terminal doing the printing, when it can be measured.
fn terminal_columns() -> Option<usize> {
    terminal_size::terminal_size().map(|(width, _)| usize::from(width.0))
}

/// The invitation block: a label, the complete link, and the same link as a QR
/// code when the terminal is wide enough to hold it.
///
/// The twin of the desktop entry point's renderer (`headless::invitation_text`):
/// it cannot be shared, because the CLI must not depend on the Desktop binary,
/// and the two must look and degrade the same way — a narrow terminal keeps the
/// link and says why the code is missing.
fn invitation_text(label: &str, link: &str, columns: Option<usize>) -> String {
    let mut output = format!("\n{label}\n{link}\n");
    match qrcode::QrCode::new(link.as_bytes()) {
        Ok(code) if columns.is_none_or(|width| code.width() + 8 <= width) => {
            use qrcode::render::unicode::Dense1x2;
            let image = code.render::<Dense1x2>().build();
            // Explicit black-on-white contrast on both light and dark terminal
            // themes. Reset every line so a log keeps its own colours.
            for line in image.lines() {
                output.push_str(&format!("\x1b[30;47m{line}\x1b[0m\n"));
            }
        }
        Ok(_) => output.push_str(
            "Terminal too narrow for this QR code; widen it and run this again, or use the link \
             above.\n",
        ),
        Err(_) => output.push_str("Cannot render a QR code; use the link above.\n"),
    }
    output
}

// ── remove ──────────────────────────────────────────────────────────────────

async fn remove(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json", "--yes", "--all"], help::REMOTE_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let yes = args.iter().any(|argument| argument == "--yes");
    let all = args.iter().any(|argument| argument == "--all");
    // Shape first, state second: a malformed command is a usage error whether or
    // not this machine has a device.
    let selector = if all {
        if let Some(extra) = values.first() {
            return Err(format!(
                "--all unpairs every device, so `{extra}` cannot also name one. Give one or \
                 the other.\n\n{}",
                help::REMOTE_USAGE
            ));
        }
        None
    } else {
        Some(one_device(values, help::REMOTE_USAGE)?)
    };
    let mut book = Book::load()?;
    if book.devices.is_empty() {
        return Err("No devices are paired to this machine.".to_string());
    }
    let Some(selector) = selector else {
        return remove_all(json_flag, yes, book, out).await;
    };
    let index = book.position(&selector)?;
    let device = book.devices[index].clone();
    if !yes && !confirmed(&device, book.devices.len() - 1)? {
        out.log_err("Cancelled. Nothing was changed.");
        return Ok(());
    }
    let running = desktop_running();
    // A running Desktop owns the bridge for this device. Asking it to drop the
    // device is what makes the removal take effect *now* — `unpair_slot` stops
    // serving it and tells it so — instead of at its next credential refresh,
    // which can be minutes away: the phone holds a bearer JWT, and until it
    // lapses nothing on the platform side can make the machine stop reading what
    // it publishes. No Desktop (or one too old to know the request) falls back to
    // editing the book, which is still authoritative for the next start.
    if running {
        match ask_desktop_to_remove(&device).await {
            // It wrote the book itself, so this process's copy is stale.
            Ok(true) => book = Book::load()?,
            Ok(false) => (),
            Err(reason) => out.log_err(&format!(
                "note: the running Desktop did not remove it ({reason}); removing it here, so \
                 it stops being served at its next credential refresh."
            )),
        }
    }
    let still_listed = book
        .devices
        .iter()
        .any(|paired| normalized(&paired.pair_id) == normalized(&device.pair_id));
    if still_listed {
        // Revoke before forgetting locally: a device whose slot write is lost can
        // still only reconnect with a credential the platform has just refused,
        // and the Desktop retires such a slot on its next start.
        let warning = revoke(&device).await;
        let index = book.position(&device.pair_id)?;
        book.remove(index);
        book.save()?;
        if let Some(warning) = &warning {
            out.log_err(warning);
        }
    }
    if json_flag {
        out.log(
            serde_json::to_string_pretty(&document(&book, running))
                .map_err(|error| error.to_string())?
                .as_str(),
        );
        return Ok(());
    }
    out.log(&format!(
        "Unpaired {}. {}",
        display_pair_id(&device.pair_id),
        match book.devices.len() {
            0 => "No devices remain paired to this machine.".to_string(),
            1 => "1 device remains paired.".to_string(),
            remaining => format!("{remaining} devices remain paired."),
        }
    ));
    if running {
        out.log(if still_listed {
            "A Desktop is running; it stops serving this device on its next credential refresh."
        } else {
            "A Desktop is running and has stopped serving it now."
        });
    }
    Ok(())
}

/// `future remote devices remove --all` — unpair everything.
///
/// The "replace this machine's pairing" flow, split into what it actually is:
/// dropping every device is revoking them (which works with no Desktop running),
/// and the fresh invitation is `future remote devices add` (which needs one). The
/// running Desktop stops serving the removed devices at their next credential
/// refresh; if one of them is still around by the next `add`, that command's own
/// cleanup stops the stale bridge before minting the replacement.
async fn remove_all(
    json_flag: bool,
    yes: bool,
    mut book: Book,
    out: &Output,
) -> Result<(), String> {
    let count = book.devices.len();
    let devices = book.devices.clone();
    if !yes
        && !ask(&format!(
            "Unpair all {count} {}? Each one loses remote access to this machine.",
            if count == 1 { "device" } else { "devices" }
        ))?
    {
        out.log_err("Cancelled. Nothing was changed.");
        return Ok(());
    }
    let mut warnings = Vec::new();
    let running = desktop_running();
    // Same reason as `remove`: the Desktop owns the bridges, so it is asked to
    // drop each device and takes effect now. Anything it did not handle is
    // revoked and dropped here — the book is authoritative either way.
    let mut stopped_by_desktop = 0usize;
    if running {
        for device in &devices {
            match ask_desktop_to_remove(device).await {
                Ok(true) => stopped_by_desktop += 1,
                Ok(false) => (),
                Err(reason) => out.log_err(&format!(
                    "note: the running Desktop did not remove {} ({reason}); removing it here.",
                    display_pair_id(&device.pair_id)
                )),
            }
        }
        // Whatever it removed is gone from the book it wrote; this process's copy
        // only has the devices left to handle.
        book = Book::load()?;
    }
    let untouched = book.devices.clone();
    for device in &untouched {
        if let Some(warning) = revoke(device).await {
            warnings.push(warning);
        }
    }
    // The book goes entirely: no slots, no pointer, and no credential file left
    // answering "this machine has a pairing" for a machine that has none.
    while !book.devices.is_empty() {
        book.remove(0);
    }
    book.save()?;
    for warning in &warnings {
        out.log_err(warning);
    }
    if json_flag {
        out.log(
            serde_json::to_string_pretty(&document(&book, running))
                .map_err(|error| error.to_string())?
                .as_str(),
        );
        return Ok(());
    }
    out.log(&format!(
        "Unpaired all {count} {}. No devices remain paired to this machine.",
        if count == 1 { "device" } else { "devices" }
    ));
    out.log("Run `future remote devices add` to pair the first one back.");
    if running {
        out.log(if stopped_by_desktop == count {
            "A Desktop is running and has stopped serving them now."
        } else {
            "A Desktop is running; it stops serving the rest at their next credential refresh."
        });
    }
    Ok(())
}

/// Tests run with cargo's inherited stdin, so a real prompt would block the
/// suite. This seam makes the "no terminal" branch reachable without one; it is
/// never set outside tests.
#[cfg(test)]
static STDIN_IS_NOT_A_TERMINAL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn stdin_is_terminal() -> bool {
    #[cfg(test)]
    if STDIN_IS_NOT_A_TERMINAL.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    std::io::stdin().is_terminal()
}

/// Ask before a security-relevant removal. A redirected stdin is refused
/// rather than read: the answer to a prompt nobody saw must not be "yes".
fn confirmed(device: &Device, remaining: usize) -> Result<bool, String> {
    ask(&format!(
        "Unpair {}? It loses remote access to this machine; {} {} paired.",
        display_pair_id(&device.pair_id),
        remaining,
        if remaining == 1 {
            "device stays"
        } else {
            "devices stay"
        }
    ))
}

/// The prompt itself, so the "all devices" question reads the same way.
fn ask(question: &str) -> Result<bool, String> {
    if !stdin_is_terminal() {
        return Err(format!(
            "Refusing to unpair without confirmation from a terminal: {question} Re-run with \
             --yes if that is what you mean."
        ));
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("Cannot read the answer: {error}"))?;
    Ok(is_yes(&answer))
}

/// What counts as consent: `y` or `yes`, in any case. Everything else — a bare
/// Enter included — is a no.
fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Revoke `device` where it was minted. Returns the line to print when the
/// removal could not be delivered now — never a failure of the command: the
/// local slot is what decides access to this machine, and the platform retry
/// queue covers the rest.
async fn revoke(device: &Device) -> Option<String> {
    if device.is_relay() {
        return match revoke_relay(device).await {
            Ok(()) => None,
            // No account, so no queue: a room that outlives its bridge serves a
            // namespace nobody listens on, and the relay prunes it on expiry.
            Err(error) => Some(format!(
                "warning: the relay did not confirm the removal ({error}). The device cannot \
                 reconnect: nothing here serves its room any more."
            )),
        };
    }
    let platform = platform_url();
    let key = match platform_api_key() {
        Ok(key) => key,
        Err(error) => {
            return Some(queue_revoke(&device.pair_id, &platform, &error));
        }
    };
    match revoke_platform(&platform, &key, &device.pair_id).await {
        Ok(()) => None,
        Err(error) => Some(queue_revoke(&device.pair_id, &platform, &error)),
    }
}

fn queue_revoke(pair_id: &str, platform: &str, reason: &str) -> String {
    match queue_pending_revoke(pair_id, platform) {
        Ok(()) => format!(
            "warning: the platform did not confirm the removal ({reason}). It is queued: the \
             next Desktop that runs retries it."
        ),
        Err(error) => format!(
            "warning: the platform did not confirm the removal ({reason}), and the retry could \
             not be queued either ({error})."
        ),
    }
}

/// Record the revoke where `pairing::retry_pending_revokes` looks for it.
///
/// A file that exists but cannot be read is an error rather than an empty queue:
/// replacing it with just this entry would drop revokes queued for other
/// devices, and those are the only record that they still need delivering.
fn queue_pending_revoke(pair_id: &str, platform: &str) -> Result<(), String> {
    let path = future_dir()?.join(PENDING_REVOKES_FILE);
    let mut pending = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| format!("{} is not a JSON object", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error.to_string()),
    };
    match pending.as_object_mut() {
        Some(map) => {
            map.insert(pair_id.to_string(), json!(platform));
        }
        None => return Err(format!("{} is not a JSON object.", path.display())),
    }
    write_json(&path, &pending)
}

async fn revoke_platform(platform: &str, key: &str, pair_id: &str) -> Result<(), String> {
    let response = client()?
        .post(format!("{platform}/client/v1/remote/pair/revoke"))
        .bearer_auth(key)
        .json(&json!({ "pair_id": pair_id }))
        .send()
        .await
        .map_err(|error| format!("revoke remote pairing: {error}"))?;
    // A pairing the platform no longer knows is the outcome the caller wanted.
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(response_error(response, "revoke remote pairing").await)
}

/// Delete this pairing's room at the relay that minted it.
///
/// Signed with the pairing's own NKey — the relay has no accounts, and the key
/// is the proof that this machine owns the room. The canonical string must stay
/// byte-identical to what `relay::sign` verifies with, which is why this reuses
/// the relay module's own signer rather than re-deriving it.
async fn revoke_relay(device: &Device) -> Result<(), String> {
    let base = device
        .control_url
        .as_deref()
        .ok_or_else(|| "the pairing names no relay".to_string())?;
    let seed = device
        .nkey_seed
        .as_deref()
        .ok_or("the pairing has no NKey, so a request signed for it cannot be produced")?;
    let key = nkeys::KeyPair::from_seed(seed)
        .map_err(|error| format!("read the pairing's NKey: {error}"))?;
    let path = format!("/v1/rooms/{}", device.pair_id);
    let (public_key, timestamp, signature) =
        crate::relay::sign::sign(&key, "DELETE", &path, crate::relay::now(), &[])?;
    let response = client_trusting(relay_ca_for(device).as_deref())?
        .delete(format!("{}{path}", base.trim_end_matches('/')))
        .header("x-future-public-key", public_key)
        .header("x-future-timestamp", timestamp)
        .header("x-future-signature", signature)
        .send()
        .await
        .map_err(|error| format!("delete the relay room: {error}"))?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(response_error(response, "delete the relay room").await)
}

/// The `{error, message}` body the platform and the relay both answer with.
async fn response_error(response: reqwest::Response, action: &str) -> String {
    let status = response.status();
    let body = response.json::<Value>().await.ok();
    let detail = body
        .as_ref()
        .and_then(|value| value.get("message"))
        .or_else(|| body.as_ref().and_then(|value| value.get("error")))
        .and_then(Value::as_str)
        .map(str::to_string);
    match detail {
        Some(detail) => format!("{action} failed (HTTP {}): {detail}", status.as_u16()),
        None => format!("{action} failed (HTTP {})", status.as_u16()),
    }
}

fn client() -> Result<reqwest::Client, String> {
    client_trusting(None)
}

/// An HTTP client that additionally trusts `ca`, a PEM bundle.
///
/// `None` is the platform client every other call uses. A self-hosted relay
/// commonly serves a certificate from a private CA, and without its root the
/// signed DELETE fails during the TLS handshake — the command would then report
/// a removal it could not deliver, on a machine where the room is the only thing
/// keeping the pairing alive.
fn client_trusting(ca: Option<&Path>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
    if let Some(ca) = ca {
        let pem = std::fs::read(ca)
            .map_err(|error| format!("Cannot read the relay CA {}: {error}", ca.display()))?;
        // `from_pem_bundle`, not `from_pem`: the single-certificate form only
        // *stores* the bytes for the rustls backend and is checked when the
        // client is built — or not at all — so a garbage file would be accepted
        // here and fail much later as an opaque TLS error during the revoke.
        let certificates = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|error| format!("{} is not a PEM certificate: {error}", ca.display()))?;
        if certificates.is_empty() {
            return Err(format!("{} holds no certificate.", ca.display()));
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|error| format!("Cannot create an HTTP client: {error}"))
}

/// The extra TLS root for this pairing's relay, if one is configured.
///
/// The Desktop applies the same two sources (`FUTURE_RELAY_CA`, then
/// `relay.json`'s `ca`) to its own NATS client, and this command must agree with
/// it or its requests never leave the machine on a private-CA relay.
///
/// The *environment* variable is an operator naming one CA for this invocation,
/// so it applies to whatever relay the pairing names. The *persisted* CA belongs
/// to the relay this installation currently points at, so it is used only for a
/// pairing on that same origin — a device left over from a previous relay must
/// not inherit that trust just because it is still in the book.
fn relay_ca_for(device: &Device) -> Option<PathBuf> {
    let base = device.control_url.as_deref()?;
    if let Ok(from_env) = std::env::var("FUTURE_RELAY_CA") {
        let path = from_env.trim();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let settings = relay_settings();
    let ca = settings.ca?;
    let configured = settings.url?;
    (origin(&configured) == origin(base)).then(|| PathBuf::from(ca))
}

/// `~/.future/relay.json` as this command needs it: the relay now in use, and
/// the CA that relay needs.
#[derive(Debug, Default)]
struct RelaySettings {
    url: Option<String>,
    ca: Option<String>,
}

fn relay_settings() -> RelaySettings {
    // The environment wins over the file, exactly as the Desktop resolves it.
    let from_env = std::env::var("FUTURE_RELAY_URL").ok();
    let env_url = from_env
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| url.trim_end_matches('/').to_string());
    let document = future_dir()
        .ok()
        .map(|future| future.join(RELAY_SETTINGS_FILE))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or(Value::Null);
    let field = |name: &str| {
        document
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    RelaySettings {
        url: env_url.or_else(|| field("url")),
        ca: field("ca"),
    }
}

/// `scheme://host[:port]` — what "the same relay" means. An unparsable URL is its
/// own origin, so a typo can only ever match itself.
fn origin(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => match (parsed.host_str(), parsed.port()) {
            (Some(host), Some(port)) => format!("{}://{host}:{port}", parsed.scheme()),
            (Some(host), None) => format!("{}://{host}", parsed.scheme()),
            (None, _) => url.trim_end_matches('/').to_string(),
        },
        Err(_) => url.trim_end_matches('/').to_string(),
    }
}

// ── use ─────────────────────────────────────────────────────────────────────

fn take_control(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json"], help::REMOTE_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let selector = one_device(values, help::REMOTE_USAGE)?;
    let mut book = Book::load()?;
    if book.devices.is_empty() {
        return Err("No devices are paired to this machine.".to_string());
    }
    let index = book.position(&selector)?;
    let pair_id = book.devices[index].pair_id.clone();
    let running = desktop_running();
    if book.active.as_deref() == Some(pair_id.as_str()) {
        if json_flag {
            out.log(
                serde_json::to_string_pretty(&document(&book, running))
                    .map_err(|error| error.to_string())?
                    .as_str(),
            );
        } else {
            out.log(&format!(
                "{} already drives this machine.",
                display_pair_id(&pair_id)
            ));
        }
        return Ok(());
    }
    book.set_controller(&pair_id);
    book.save()?;
    if json_flag {
        out.log(
            serde_json::to_string_pretty(&document(&book, running))
                .map_err(|error| error.to_string())?
                .as_str(),
        );
        return Ok(());
    }
    out.log(&format!(
        "{} now drives this machine.",
        display_pair_id(&pair_id)
    ));
    if running {
        out.log(
            "A Desktop is running: it keeps its current controller until it restarts, then this \
             takes effect.",
        );
    }
    Ok(())
}

// ── Little argument and environment helpers ─────────────────────────────────

/// The positional arguments, rejecting any option the action does not accept: a
/// typo must not silently do something else.
fn positional(args: &[String], allowed: &[&str], usage: &str) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    for argument in args {
        if allowed.contains(&argument.as_str()) {
            continue;
        }
        if argument.starts_with('-') {
            return Err(format!("Unknown option: {argument}\n\n{usage}"));
        }
        values.push(argument.clone());
    }
    Ok(values)
}

fn one_device(mut values: Vec<String>, usage: &str) -> Result<String, String> {
    match values.len() {
        1 => Ok(values.remove(0)),
        0 => Err(format!(
            "Which device? Give its id or its row number.\n\n{usage}"
        )),
        count => Err(format!(
            "One device at a time, please: {} names {count} devices.\n\n{usage}",
            values.join(" ")
        )),
    }
}

/// The signed-in Future account key, from the same `auth.json` the agent and
/// the Desktop use.
fn platform_api_key() -> Result<String, String> {
    let path = auth_file();
    let text = std::fs::read_to_string(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("not signed in ({} does not exist)", path.display())
        }
        _ => format!("cannot read {}: {error}", path.display()),
    })?;
    let auth: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
    auth.get("future")
        .and_then(|future| future.get("key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("no \"future\" API key in {}", path.display()))
}

/// The platform root this installation talks to, resolved exactly as the
/// Desktop resolves it (`future_platform::resolve_future_platform_url`): an
/// explicit `base_url` (with its `/api` suffix removed), then
/// `platform_base_url`, then the default. Getting this wrong would send a
/// revoke to the wrong environment.
fn platform_url() -> String {
    let auth: Value = std::fs::read_to_string(auth_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    platform_url_from(&auth)
}

fn platform_url_from(auth: &Value) -> String {
    let future = auth.get("future");
    let field = |name: &str| {
        future
            .and_then(|future| future.get(name))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    if let Some(base) = field("base_url") {
        let trimmed = base.trim_end_matches('/');
        let platform = trimmed.strip_suffix("/api").unwrap_or(trimmed);
        return platform.trim_end_matches('/').to_string();
    }
    if let Some(platform) = field("platform_base_url") {
        return platform.trim_end_matches('/').to_string();
    }
    DEFAULT_PLATFORM_URL.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;
    use crate::test_env::EnvGuard;
    use chrono::TimeZone;

    async fn run(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (out, capture) = Output::memory();
        let code = crate::dispatch(&args, &out).await;
        let stdout = String::from_utf8(capture.out.lock().unwrap().clone()).unwrap();
        let stderr = String::from_utf8(capture.err.lock().unwrap().clone()).unwrap();
        (code, stdout, stderr)
    }

    /// A throwaway home, with `FUTURE_HOME` cleared so an ambient one cannot
    /// redirect the test at the real installation.
    struct Home {
        dir: tempfile::TempDir,
        _home: EnvGuard,
        _future_home: EnvGuard,
    }

    impl Home {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let home = EnvGuard::set(&[("HOME", dir.path().as_os_str().to_owned())]);
            let future_home = EnvGuard::remove(&["FUTURE_HOME"]);
            Home {
                dir,
                _home: home,
                _future_home: future_home,
            }
        }

        fn future(&self) -> PathBuf {
            self.dir.path().join(".future")
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::create_dir_all(self.future()).unwrap();
            std::fs::write(self.future().join(name), text).unwrap();
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.future().join(name)).unwrap()
        }

        fn exists(&self, name: &str) -> bool {
            self.future().join(name).exists()
        }
    }

    /// A book with two devices: `AAAA` on the platform (out of control) and
    /// `BBBB` at `relay` (in control), both with seedless credentials.
    fn two_device_book(relay: &str) -> String {
        serde_json::to_string_pretty(&json!({
            "version": 2,
            "active": "pair_BBBB",
            "slots": [
                {
                    "handshakeVersion": 2,
                    "pairId": "pair_AAAA",
                    "desktopId": "desk-1",
                    "nkeySeed": "SUAAAA",
                    "userJwt": "jwt-a",
                    "natsUrl": "nats://a",
                },
                {
                    "handshakeVersion": 2,
                    "pairId": "pair_BBBB",
                    "desktopId": "desk-1",
                    "nkeySeed": "SUBBBB",
                    "userJwt": "jwt-b",
                    "controlUrl": relay,
                }
            ]
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn list_names_every_device_and_the_one_in_control() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));

        let (code, stdout, stderr) = run(&["remote", "devices"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(
            stdout.contains("2 devices are paired to this machine:"),
            "{stdout}"
        );
        assert!(stdout.contains("AAAA"), "{stdout}");
        assert!(stdout.contains("BBBB"), "{stdout}");
        assert!(stdout.contains("in control"), "{stdout}");
        assert!(stdout.contains("standby"), "{stdout}");
        assert!(stdout.contains("Future platform"), "{stdout}");
        assert!(
            stdout.contains("relay https://relay.example.com"),
            "{stdout}"
        );
        // The credential material is never printed.
        assert!(!stdout.contains("SUAAAA"), "{stdout}");
        assert!(!stdout.contains("jwt-a"), "{stdout}");
    }

    #[tokio::test]
    async fn list_json_is_the_document_the_app_uses() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));

        let (code, stdout, _) = run(&["remote", "devices", "list", "--json"]).await;
        assert_eq!(code, 0);
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["desktopRunning"], false);
        assert_eq!(value["active"], "pair_BBBB");
        assert_eq!(value["devices"][0]["control"], "standby");
        assert_eq!(value["devices"][0]["endpoint"], "platform");
        assert_eq!(value["devices"][1]["control"], "active");
        assert_eq!(value["devices"][1]["endpoint"], "relay");
        assert_eq!(value["devices"][1]["relayUrl"], "https://relay.example.com");
        // No seed survives into the machine-readable form either.
        assert!(!stdout.contains("SUAAAA"), "{stdout}");
    }

    #[tokio::test]
    async fn an_unpaired_machine_says_so_and_creates_nothing() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (code, stdout, _) = run(&["remote", "devices"]).await;
        assert_eq!(code, 0);
        assert!(
            stdout.contains("No devices are paired to this machine."),
            "{stdout}"
        );
        // Nothing was created just by looking: no app directory, no lock, no book.
        assert!(!home.future().join("app").exists());
        assert!(!home.exists(BOOK_FILE));
    }

    #[tokio::test]
    async fn a_legacy_single_pairing_reads_as_one_device_in_control() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(
            LEGACY_FILE,
            &json!({ "pairId": "pair_OLD", "desktopId": "desk-1", "nkeySeed": "S" }).to_string(),
        );

        let (code, stdout, _) = run(&["remote", "devices", "--json"]).await;
        assert_eq!(code, 0);
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["devices"][0]["pairId"], "pair_OLD");
        assert_eq!(value["active"], "pair_OLD");
        assert_eq!(value["devices"][0]["control"], "active");
    }

    #[tokio::test]
    async fn a_corrupt_book_is_an_error_not_an_empty_list() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, "{ not json");
        let (code, stdout, stderr) = run(&["remote", "devices"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(stderr.contains("is not valid JSON"), "{stderr}");
        // The file is left exactly as it was, not replaced by an empty book.
        assert_eq!(home.read(BOOK_FILE), "{ not json");
    }

    #[tokio::test]
    async fn use_persists_the_controller_and_keeps_unmodelled_fields() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));

        let (code, stdout, stderr) = run(&["remote", "devices", "use", "1"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("AAAA now drives this machine."), "{stdout}");

        let value: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
        assert_eq!(value["active"], "pair_AAAA");
        // Everything this command does not model survives the rewrite.
        assert_eq!(value["version"], 2);
        assert_eq!(value["slots"][0]["handshakeVersion"], 2);
        assert_eq!(value["slots"][0]["nkeySeed"], "SUAAAA");
        assert_eq!(value["slots"][1]["controlUrl"], "https://relay.example.com");
    }

    #[tokio::test]
    async fn use_accepts_a_stripped_or_partial_id() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        // `pair_` prefix omitted, lower case, and a prefix rather than the whole id.
        let (code, _, stderr) = run(&["remote", "devices", "use", "aaa"]).await;
        assert_eq!(code, 0, "{stderr}");
        let value: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
        assert_eq!(value["active"], "pair_AAAA");
    }

    #[tokio::test]
    async fn an_ambiguous_or_unknown_selector_is_refused() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(
            BOOK_FILE,
            &serde_json::to_string(&json!({
                "version": 2,
                "active": null,
                "slots": [
                    { "pairId": "pair_A111", "nkeySeed": "S" },
                    { "pairId": "pair_A222", "nkeySeed": "S" }
                ]
            }))
            .unwrap(),
        );
        let (code, _, stderr) = run(&["remote", "devices", "use", "ZZZ"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("No device matches `ZZZ`"), "{stderr}");
        let (code, _, stderr) = run(&["remote", "devices", "use", "9"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("No device matches `9`"), "{stderr}");
        let (code, _, stderr) = run(&["remote", "devices", "use", "A"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("matches 2 devices"), "{stderr}");
    }

    #[tokio::test]
    async fn use_does_nothing_when_the_device_already_controls() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let before = home.read(BOOK_FILE);
        let (code, stdout, _) = run(&["remote", "devices", "use", "2"]).await;
        assert_eq!(code, 0);
        assert!(stdout.contains("already drives this machine"), "{stdout}");
        assert_eq!(home.read(BOOK_FILE), before);
    }

    #[tokio::test]
    async fn remove_drops_one_device_and_leaves_the_rest_paired() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        // Not signed in and no platform reachable: the removal is still a local
        // fact, and the undeliverable revoke is queued rather than lost.
        let (code, stdout, stderr) = run(&["remote", "devices", "remove", "AAAA", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("Unpaired AAAA"), "{stdout}");
        assert!(stdout.contains("1 device remains paired."), "{stdout}");
        assert!(stderr.contains("queued"), "{stderr}");

        let value: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
        assert_eq!(value["slots"].as_array().unwrap().len(), 1);
        assert_eq!(value["slots"][0]["pairId"], "pair_BBBB");
        assert_eq!(value["active"], "pair_BBBB");
        let pending: Value = serde_json::from_str(&home.read(PENDING_REVOKES_FILE)).unwrap();
        assert_eq!(pending["pair_AAAA"], DEFAULT_PLATFORM_URL);
    }

    #[tokio::test]
    async fn removing_the_controller_clears_the_pointer_without_promoting_anyone() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        // A relay URL that is never reached: the seed is not a valid NKey, so the
        // signed request is refused before any network call.
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let (code, _, stderr) = run(&["remote", "devices", "remove", "BBBB", "--yes"]).await;
        assert_eq!(code, 0);
        assert!(stderr.contains("the relay did not confirm"), "{stderr}");
        let value: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
        assert!(value["active"].is_null(), "{value}");
        // With one device left it controls anyway — the runtime's own rule.
        let (_, stdout, _) = run(&["remote", "devices", "--json"]).await;
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["active"], "pair_AAAA");
        assert_eq!(value["devices"][0]["control"], "active");
    }

    #[tokio::test]
    async fn removing_the_last_device_removes_the_credential_files() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(
            LEGACY_FILE,
            &json!({ "pairId": "pair_OLD", "nkeySeed": "S" }).to_string(),
        );
        let (code, stdout, _) = run(&["remote", "devices", "remove", "OLD", "--yes"]).await;
        assert_eq!(code, 0);
        assert!(stdout.contains("No devices remain paired"), "{stdout}");
        assert!(!home.exists(LEGACY_FILE));
        assert!(!home.exists(BOOK_FILE));
    }

    #[tokio::test]
    async fn remove_refuses_a_redirected_stdin_without_yes() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let before = home.read(BOOK_FILE);
        STDIN_IS_NOT_A_TERMINAL.store(true, std::sync::atomic::Ordering::SeqCst);
        let (code, _, stderr) = run(&["remote", "devices", "remove", "AAAA"]).await;
        STDIN_IS_NOT_A_TERMINAL.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(code, 1);
        assert!(
            stderr.contains("without confirmation from a terminal"),
            "{stderr}"
        );
        assert!(stderr.contains("--yes"), "{stderr}");
        assert_eq!(home.read(BOOK_FILE), before);
    }

    /// A running Desktop is asked to drop the device, and its answer is what the
    /// terminal reports. This is the fix for "unpaired, but the phone still
    /// connects": editing the book alone leaves the bridge serving a device whose
    /// bearer credential stays valid for minutes.
    #[tokio::test]
    async fn remove_asks_the_running_desktop_to_stop_serving_the_device() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let _desktop = RunningDesktop::claim(&home);
        let seen = answer_removals(&home, 1);

        let (code, stdout, stderr) = run(&["remote", "devices", "remove", "AAAA", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("Unpaired AAAA"), "{stdout}");
        assert!(stdout.contains("1 device remains paired."), "{stdout}");
        assert!(
            stdout.contains("stopped serving it now"),
            "the removal was immediate, and the answer says so: {stdout}"
        );
        assert!(
            !stdout.contains("next credential refresh"),
            "the slow path is not what happened: {stdout}"
        );
        // The Desktop owns the revoke as well: no second attempt from here, and
        // nothing to queue.
        assert!(stderr.is_empty(), "{stderr}");
        assert!(!home.exists(PENDING_REVOKES_FILE));

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "one request, one answer");
        assert_eq!(seen[0]["op"], "remove-device");
        assert_eq!(seen[0]["pairId"], "pair_AAAA");
        // The other device is untouched, and it now controls by being alone.
        let value: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
        assert_eq!(value["slots"].as_array().unwrap().len(), 1);
        assert_eq!(value["slots"][0]["pairId"], "pair_BBBB");
    }

    /// A Desktop that cannot perform the removal — an older build, one that
    /// exited — is not a failure of the command: the book is still authoritative
    /// for the next start, so the device is revoked and dropped here. What the
    /// user is told changes, because what happened did.
    #[tokio::test]
    async fn a_desktop_that_cannot_remove_falls_back_to_editing_the_book() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let _desktop = RunningDesktop::claim(&home);
        let seen = answer_requests(
            &home,
            json!({ "error": "this Desktop does not know the `remove-device` request; update it" }),
        );

        let (code, stdout, stderr) = run(&["remote", "devices", "remove", "AAAA", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("Unpaired AAAA"), "{stdout}");
        assert!(
            stdout.contains("next credential refresh"),
            "the slow path has to be named as slow: {stdout}"
        );
        assert!(
            stderr.contains("did not remove it"),
            "and the reason it was slow is the Desktop's own: {stderr}"
        );

        assert_eq!(seen.lock().unwrap()[0]["op"], "remove-device");
        let value: Value = serde_json::from_str(&home.read(BOOK_FILE)).unwrap();
        assert_eq!(value["slots"].as_array().unwrap().len(), 1);
        assert_eq!(value["slots"][0]["pairId"], "pair_BBBB");
    }

    /// `--all` asks once per device, and says all of them stopped now.
    #[tokio::test]
    async fn remove_all_asks_the_running_desktop_for_each_device() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let _desktop = RunningDesktop::claim(&home);
        let seen = answer_removals(&home, 2);

        let (code, stdout, stderr) = run(&["remote", "devices", "remove", "--all", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(
            stdout.contains("No devices remain paired to this machine."),
            "{stdout}"
        );
        assert!(stdout.contains("stopped serving them now"), "{stdout}");
        assert!(!home.exists(BOOK_FILE), "the empty book leaves no file");
        let seen = seen.lock().unwrap();
        let ids: Vec<&str> = seen
            .iter()
            .map(|request| request["pairId"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(ids, ["pair_AAAA", "pair_BBBB"], "{seen:?}");
    }

    #[tokio::test]
    async fn an_undeliverable_platform_revoke_is_queued_for_the_next_desktop() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        // A platform that answers 503, and a signed-in account pointed at it.
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/remote/pair/revoke",
            503,
            "{\"error\":\"unavailable\",\"message\":\"try later\"}",
        )])
        .await;
        let auth = auth_file();
        std::fs::create_dir_all(auth.parent().unwrap()).unwrap();
        std::fs::write(
            &auth,
            json!({ "future": { "key": "sk-test", "base_url": format!("{base}/api") } })
                .to_string(),
        )
        .unwrap();
        home.write(
            BOOK_FILE,
            &json!({
                "version": 2,
                "active": null,
                "slots": [{ "pairId": "pair_ONLY", "nkeySeed": "S" }]
            })
            .to_string(),
        );

        let (code, _, stderr) = run(&["remote", "devices", "remove", "ONLY", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.contains("try later"), "{stderr}");
        assert!(stderr.contains("queued"), "{stderr}");
        let pending: Value = serde_json::from_str(&home.read(PENDING_REVOKES_FILE)).unwrap();
        assert_eq!(pending["pair_ONLY"], base);
    }

    #[tokio::test]
    async fn a_delivered_platform_revoke_leaves_no_queue_behind() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let base = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::json(
                "/client/v1/remote/pair/revoke",
                200,
                "{\"ok\":true}",
            )],
            Some(requests.clone()),
        )
        .await;
        let auth = auth_file();
        std::fs::create_dir_all(auth.parent().unwrap()).unwrap();
        std::fs::write(
            &auth,
            json!({ "future": { "key": "sk-test", "base_url": format!("{base}/api") } })
                .to_string(),
        )
        .unwrap();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));

        let (code, _, stderr) = run(&["remote", "devices", "remove", "AAAA", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        assert!(!home.exists(PENDING_REVOKES_FILE));
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(
            recorded[0].contains("authorization: Bearer sk-test"),
            "{}",
            recorded[0]
        );
        assert!(recorded[0].contains("pair_AAAA"), "{}", recorded[0]);
    }

    #[tokio::test]
    async fn a_relay_revoke_deletes_the_room_with_the_pairings_own_key() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let relay = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::json(
                "/v1/rooms/pair_BBBB",
                200,
                "{\"ok\":true}",
            )],
            Some(requests.clone()),
        )
        .await;
        let key = nkeys::KeyPair::new_user();
        let seed = key.seed().unwrap();
        let book = json!({
            "version": 2,
            "active": null,
            "slots": [{
                "pairId": "pair_BBBB",
                "desktopId": "desk-1",
                "nkeySeed": seed,
                "controlUrl": relay,
            }]
        });
        home.write(BOOK_FILE, &book.to_string());

        let (code, _, stderr) = run(&["remote", "devices", "remove", "BBBB", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        assert!(!home.exists(BOOK_FILE));
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(
            recorded[0].starts_with("DELETE /v1/rooms/pair_BBBB"),
            "{}",
            recorded[0]
        );
        assert!(
            recorded[0].contains(&format!("x-future-public-key: {}", key.public_key())),
            "{}",
            recorded[0]
        );
        assert!(
            recorded[0].contains("x-future-signature: "),
            "{}",
            recorded[0]
        );
    }

    // ── add ─────────────────────────────────────────────────────────────────

    /// Hold the Desktop's own lock for as long as this lives, so `add` has a
    /// Desktop to ask — the same file `instance::InstanceGuard` locks.
    struct RunningDesktop {
        _file: std::fs::File,
    }

    impl RunningDesktop {
        fn claim(home: &Home) -> Self {
            let app = home.future().join("app");
            std::fs::create_dir_all(&app).unwrap();
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(app.join(DESKTOP_LOCK_FILE))
                .unwrap();
            fs2::FileExt::lock_exclusive(&file).unwrap();
            RunningDesktop { _file: file }
        }
    }

    /// Answer the next request the way the Desktop's control host does, and give
    /// back the request that was posed so a test can judge it.
    fn answer_requests(home: &Home, answer: Value) -> std::sync::Arc<std::sync::Mutex<Vec<Value>>> {
        let directory = home.future().join("run").join(CONTROL_DIRECTORY);
        std::fs::create_dir_all(&directory).unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = seen.clone();
        tokio::spawn(async move {
            loop {
                let mut entries: Vec<_> = std::fs::read_dir(&directory)
                    .map(|entries| entries.filter_map(Result::ok).collect())
                    .unwrap_or_default();
                entries.sort_by_key(|entry| entry.file_name());
                if let Some(entry) = entries.into_iter().find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(REQUEST_SUFFIX)
                }) {
                    let request: Value =
                        serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap())
                            .unwrap();
                    std::fs::write(
                        request["reply"].as_str().expect("a reply path"),
                        answer.to_string(),
                    )
                    .unwrap();
                    recorded.lock().unwrap().push(request);
                    let _ = std::fs::remove_file(entry.path());
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        seen
    }

    /// Answer `remove-device` requests the way the running Desktop does: drop the
    /// device from the book, then reply `removed`.
    ///
    /// The order matters to the command under test — it re-reads the book after
    /// the answer to decide whether it still has to do the work here, so a fake
    /// that only wrote a reply would send every removal down the fallback path
    /// and test nothing.
    fn answer_removals(home: &Home, count: usize) -> std::sync::Arc<std::sync::Mutex<Vec<Value>>> {
        let directory = home.future().join("run").join(CONTROL_DIRECTORY);
        std::fs::create_dir_all(&directory).unwrap();
        let worked = home.future().join(BOOK_FILE);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = seen.clone();
        tokio::spawn(async move {
            let mut answered = 0;
            while answered < count {
                let mut entries: Vec<_> = std::fs::read_dir(&directory)
                    .map(|entries| entries.filter_map(Result::ok).collect())
                    .unwrap_or_default();
                entries.sort_by_key(|entry| entry.file_name());
                let Some(entry) = entries.into_iter().find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(REQUEST_SUFFIX)
                }) else {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                };
                let request: Value =
                    serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
                let pair_id = request["pairId"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a removal names its device: {request}"))
                    .to_string();
                if let Ok(text) = std::fs::read_to_string(&worked) {
                    let mut value: Value = serde_json::from_str(&text).unwrap();
                    if let Some(slots) = value["slots"].as_array_mut() {
                        slots.retain(|slot| slot["pairId"] != json!(pair_id));
                    }
                    if value["active"] == json!(pair_id) {
                        value["active"] = Value::Null;
                    }
                    if value["slots"].as_array().is_some_and(Vec::is_empty) {
                        let _ = std::fs::remove_file(&worked);
                    } else {
                        std::fs::write(&worked, value.to_string()).unwrap();
                    }
                }
                std::fs::write(
                    request["reply"].as_str().expect("a reply path"),
                    json!({ "pairId": pair_id, "removed": true }).to_string(),
                )
                .unwrap();
                recorded.lock().unwrap().push(request);
                let _ = std::fs::remove_file(entry.path());
                answered += 1;
            }
        });
        seen
    }

    const LINK: &str =
        "futureos://remote/pair?v=2&code=abc&desktopId=d&desktopKey=U&desktopSecret=S";

    /// The `Single-use.` line of an invitation, which carries the deadline.
    fn single_use_line(stdout: &str) -> &str {
        stdout
            .lines()
            .find(|line| line.contains("Single-use."))
            .unwrap_or_else(|| panic!("no single-use line: {stdout}"))
    }

    /// The `It expires at <clock> (local time)` clause of an invitation, parsed.
    fn printed_clock(stdout: &str) -> chrono::NaiveDateTime {
        let line = single_use_line(stdout);
        assert!(
            !line.contains("Unix time"),
            "a deadline a calendar can name is printed as a clock: {line}"
        );
        let clock = line
            .split("expires at ")
            .nth(1)
            // The clock ends where the parenthetical about zone and time left
            // begins: `2026-10-20 09:45:32 (local time, in 9m 58s)`.
            .and_then(|rest| rest.split(" (").next())
            .unwrap_or_else(|| panic!("no expiry in {line}"));
        chrono::NaiveDateTime::parse_from_str(clock, "%Y-%m-%d %H:%M:%S")
            .unwrap_or_else(|error| panic!("{clock:?} is not YYYY-MM-DD HH:MM:SS: {error}"))
    }

    #[test]
    fn a_deadline_no_calendar_can_name_keeps_the_epoch() {
        assert_eq!(
            local_wall_clock(1_800_000_000).map(|clock| clock.len()),
            Some(19)
        );
        // A Desktop whose clock is absurd keeps the invitation readable instead
        // of panicking in the formatter.
        assert_eq!(local_wall_clock(i64::MAX), None);
        assert_eq!(local_wall_clock(-1), None);
    }

    /// The time-left clause names the largest unit worth reading, and disappears
    /// rather than saying "in 0s" once the deadline is behind the reader's clock.
    #[test]
    fn the_time_left_is_readable_and_never_counts_down_to_zero() {
        assert_eq!(time_left(1_042, 1_000).as_deref(), Some("42s"));
        assert_eq!(time_left(1_598, 1_000).as_deref(), Some("9m 58s"));
        assert_eq!(time_left(1_000 + 7_500, 1_000).as_deref(), Some("2h 05m"));
        assert_eq!(time_left(1_000, 1_000), None, "a deadline now is gone");
        assert_eq!(time_left(999, 1_000), None, "a deadline past is gone");
        assert_eq!(deadline_phrase(i64::MAX, 0), None);
        // A machine whose clock runs ahead of the Desktop's still names the
        // deadline instead of printing "in 0s" — or panicking.
        let phrase = deadline_phrase(1_800_000_000, 1_900_000_000).expect("a clock");
        assert!(phrase.ends_with("(local time)"), "{phrase}");
    }

    #[tokio::test]
    async fn add_prints_the_invitation_the_desktop_answered_with() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        let seen = answer_requests(
            &home,
            json!({ "pairId": "pair_NEW", "link": LINK, "expiresAt": 1_800_000_000i64 }),
        );

        let (code, stdout, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        assert!(stdout.contains(LINK), "{stdout}");
        assert!(stdout.contains("for NEW"), "the device is named: {stdout}");
        // The deadline is a wall clock a person can compare with their own
        // watch, not an epoch; `--json` keeps the epoch for scripts. The zone
        // is the machine's, so the test reads the printed clock back instead of
        // hard-coding what CI's timezone would render.
        let clock = printed_clock(&stdout);
        assert_eq!(
            chrono::Local
                .from_local_datetime(&clock)
                .single()
                .expect("an unambiguous local time")
                .timestamp(),
            1_800_000_000,
            "the printed clock is the deadline itself: {stdout}"
        );
        // The time left to act is in the same clause, so nobody has to subtract
        // two clocks to learn how long they have.
        assert!(
            single_use_line(&stdout).contains("(local time, in "),
            "{stdout}"
        );
        // The request carried the operation and a reply path, and was cleaned up.
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "one request, one answer");
        assert_eq!(seen[0]["op"], "add-device");
        assert!(seen[0]["reply"]
            .as_str()
            .is_some_and(|path| path.ends_with(".json")));
        assert!(
            !std::fs::read_dir(home.future().join("run").join(CONTROL_DIRECTORY))
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(REQUEST_SUFFIX)),
            "a request must not outlive the command"
        );
    }

    /// A freshly minted invitation counts down the budget the phone actually
    /// has: "in 9m 58s" answers the question the epoch did not.
    #[tokio::test]
    async fn add_counts_down_the_time_left_to_scan() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        answer_requests(
            &home,
            json!({
                "pairId": "pair_NEW",
                "link": LINK,
                "expiresAt": now_seconds() + 600,
            }),
        );

        let (code, stdout, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 0, "{stderr}");
        let line = single_use_line(&stdout);
        let left = line
            .split("(local time, in ")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .unwrap_or_else(|| panic!("no time left in {line}"));
        // A ten-minute budget, read a moment after the Desktop answered.
        assert!(
            left == "10m 00s" || left.starts_with("9m "),
            "the countdown must match the invitation's budget: {line}"
        );
    }

    #[tokio::test]
    async fn add_json_prints_what_a_script_needs() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        answer_requests(
            &home,
            json!({ "pairId": "pair_NEW", "link": LINK, "expiresAt": 1_800_000_000i64 }),
        );

        let (code, stdout, stderr) = run(&["remote", "devices", "add", "--json"]).await;
        assert_eq!(code, 0, "{stderr}");
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["pairId"], "pair_NEW");
        assert_eq!(value["link"], LINK);
        assert_eq!(value["expiresAt"], 1_800_000_000i64);
    }

    /// A mint that had to clear room says so. The slots it freed belong to
    /// invitations the user asked for earlier, so a silent success would leave
    /// them wondering why a code that used to work no longer does.
    #[tokio::test]
    async fn add_reports_the_invitations_it_cleared_to_make_room() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        answer_requests(
            &home,
            json!({
                "pairId": "pair_NEW",
                "link": LINK,
                "expiresAt": 1_800_000_000i64,
                "cleared": ["pair_OLD_A", "pair_OLD_B"],
            }),
        );

        let (code, stdout, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("cleared 2 invitations"), "{stdout}");
        assert!(stdout.contains("pair_OLD_A, pair_OLD_B"), "{stdout}");
    }

    #[tokio::test]
    async fn add_json_carries_the_cleared_invitations() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        answer_requests(
            &home,
            json!({
                "pairId": "pair_NEW",
                "link": LINK,
                "expiresAt": 1_800_000_000i64,
                "cleared": ["pair_OLD_A"],
            }),
        );

        let (code, stdout, stderr) = run(&["remote", "devices", "add", "--json"]).await;
        assert_eq!(code, 0, "{stderr}");
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["cleared"], json!(["pair_OLD_A"]));
    }

    /// A refusal is the Desktop's answer, printed as such — not a timeout, and not
    /// a success with no invitation in it.
    #[tokio::test]
    async fn add_reports_the_desktops_refusal() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        let _answer = answer_requests(&home, json!({ "error": "pairing_limit_reached" }));

        let (code, stdout, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(stderr.contains("pairing_limit_reached"), "{stderr}");
        assert!(stderr.contains("did not issue an invitation"), "{stderr}");
    }

    /// The platform's own code reaches the user verbatim. The device cap is the
    /// case that matters: `pairing_limit_reached` is the only thing that tells
    /// them to unpair a device, and a generic "the action could not be completed"
    /// would leave them with no next step.
    #[tokio::test]
    async fn the_platforms_device_cap_is_passed_through_word_for_word() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        let _answer = answer_requests(&home, json!({ "error": "pairing_limit_reached" }));

        let (code, _, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 1);
        // Printed as the Desktop reported it, and nothing generic in front of it.
        assert_eq!(
            stderr.trim(),
            "The Desktop did not issue an invitation: pairing_limit_reached"
        );
    }

    /// Two `add`s must not both mint a device: the second is refused at once, with
    /// what to do about it, rather than queueing behind a code it never sees.
    #[tokio::test]
    async fn a_second_add_is_refused_while_one_is_waiting() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        // Hold the same lock the first `add` takes — exactly the state a second
        // terminal finds while the first is waiting.
        let first = exclusive_add_slot().expect("the first add takes the slot");

        let (code, stdout, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(
            stderr.contains("already waiting for an invitation"),
            "{stderr}"
        );
        assert!(stderr.contains("prints the code"), "{stderr}");
        // Refused before anything was written for the Desktop to act on.
        let requests = std::fs::read_dir(home.future().join("run").join(CONTROL_DIRECTORY))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .ends_with(REQUEST_SUFFIX)
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(requests, 0, "the refused add must not pose a request");

        // The slot is released with the process that held it, so the next add
        // proceeds instead of being refused forever.
        drop(first);
        assert!(exclusive_add_slot().is_ok());
    }

    /// A Desktop that exits while the request is pending is reported as that, not
    /// as a timeout — which would send the user looking for a process that is
    /// already gone, after a minute of waiting for nothing.
    #[tokio::test]
    async fn add_reports_a_desktop_that_exits_mid_request() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        let answer = home.dir.path().join("never-written.json");
        let directory = home.future().join("run").join(CONTROL_DIRECTORY);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("0001{REQUEST_SUFFIX}")),
            json!({ "op": "add-device", "reply": answer.display().to_string() }).to_string(),
        )
        .unwrap();

        // The Desktop's lock is released on drop; the probe is the same
        // `desktop_running` the command uses, injected so the wait can be cut short.
        drop(_desktop);
        let error = wait_for_answer(&answer, &|| true, &|| false, "nothing was paired")
            .await
            .expect_err("a Desktop that stopped must not look like a slow one");
        assert!(error.contains("stopped before it answered"), "{error}");
        assert!(!error.contains("did not answer within"), "{error}");
    }

    /// The two silences are different faults, and the message says which one this
    /// was. An unclaimed request means no process is watching the directory — in
    /// practice a Desktop started before the update, still serving from its old
    /// code — and the fix is to restart it; a claimed one means the Desktop took
    /// the request and then failed.
    #[tokio::test]
    async fn a_timeout_says_whether_anything_picked_the_request_up() {
        let home = tempfile::tempdir().unwrap();
        let answer = home.path().join("never-written.json");

        let unclaimed = wait_for_answer(&answer, &|| true, &|| true, "nothing was paired")
            .await
            .expect_err("an unclaimed request times out");
        assert!(
            unclaimed.contains("No Desktop picked the request up"),
            "{unclaimed}"
        );
        assert!(unclaimed.contains("older build"), "{unclaimed}");
        assert!(unclaimed.contains("Restart it"), "{unclaimed}");
        assert!(unclaimed.contains("nothing was paired"), "{unclaimed}");
        // Not the old, now-untrue advice: adds are serialized, so "busy pairing
        // another device" cannot be the explanation for this one.
        assert!(!unclaimed.contains("busy pairing"), "{unclaimed}");

        let claimed = wait_for_answer(&answer, &|| false, &|| true, "nothing was paired")
            .await
            .expect_err("a claimed request that never answers times out");
        assert!(
            claimed.contains("took the request but did not answer"),
            "{claimed}"
        );
        assert!(!claimed.contains("older build"), "{claimed}");
    }

    #[tokio::test]
    async fn add_without_a_desktop_says_what_to_start() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let (code, stdout, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert!(stderr.contains("No Desktop is running"), "{stderr}");
        assert!(stderr.contains("futureos-headless"), "{stderr}");
        // Nothing was written for a Desktop that is not there.
        assert!(!home.future().join("run").join(CONTROL_DIRECTORY).exists());
    }

    /// A Desktop that holds the lock but never answers is a timeout, not a hang.
    #[tokio::test]
    async fn add_gives_up_when_the_desktop_never_answers() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        let (code, _, stderr) = run(&["remote", "devices", "add"]).await;
        assert_eq!(code, 1);
        // Nothing claims the request in this fixture, which is the "an older
        // build is running" silence rather than "took it and gave up".
        assert!(
            stderr.contains("No Desktop picked the request up"),
            "{stderr}"
        );
        // The unanswered request does not stay behind for the next Desktop. The
        // add lock beside it is expected: it is held and released, never consumed.
        let left = std::fs::read_dir(home.future().join("run").join(CONTROL_DIRECTORY))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .ends_with(REQUEST_SUFFIX)
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(left, 0, "the request must be withdrawn on timeout");
    }

    #[tokio::test]
    async fn add_takes_no_device_argument() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        let (code, _, stderr) = run(&["remote", "devices", "add", "AAAA"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("takes no argument"), "{stderr}");
    }

    // ── remove --all ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn remove_all_unpairs_every_device_and_leaves_no_credential_file() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));

        let (code, stdout, stderr) = run(&["remote", "devices", "remove", "--all", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(
            stdout.contains("Unpaired all 2 devices"),
            "both are named: {stdout}"
        );
        assert!(stdout.contains("No devices remain paired"), "{stdout}");
        assert!(stdout.contains("future remote devices add"), "{stdout}");
        assert!(!home.exists(BOOK_FILE), "the book goes with the devices");
        assert!(!home.exists(LEGACY_FILE));
    }

    #[tokio::test]
    async fn remove_all_refuses_a_selector() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let before = home.read(BOOK_FILE);
        let (code, _, stderr) =
            run(&["remote", "devices", "remove", "--all", "AAAA", "--yes"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("cannot also name one"), "{stderr}");
        assert_eq!(home.read(BOOK_FILE), before);
    }

    #[tokio::test]
    async fn remove_all_asks_before_dropping_everything() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let before = home.read(BOOK_FILE);
        STDIN_IS_NOT_A_TERMINAL.store(true, std::sync::atomic::Ordering::SeqCst);
        let (code, _, stderr) = run(&["remote", "devices", "remove", "--all"]).await;
        STDIN_IS_NOT_A_TERMINAL.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(code, 1);
        assert!(
            stderr.contains("Unpair all 2 devices"),
            "the question names the scope: {stderr}"
        );
        assert!(stderr.contains("--yes"), "{stderr}");
        assert_eq!(home.read(BOOK_FILE), before);
    }

    /// The invitation block degrades the way the desktop's renderer does: the QR
    /// when the terminal holds it, the link alone otherwise, and always the link.
    #[test]
    fn the_invitation_block_draws_the_qr_or_keeps_the_link() {
        let wide = invitation_text("LABEL", LINK, Some(200));
        assert!(wide.contains("LABEL"), "{wide}");
        assert!(wide.contains(LINK), "{wide}");
        assert!(
            wide.contains('\u{2588}'),
            "a wide terminal gets the QR blocks: {wide}"
        );

        let narrow = invitation_text("LABEL", LINK, Some(20));
        assert!(narrow.contains(LINK), "{narrow}");
        assert!(narrow.contains("Terminal too narrow"), "{narrow}");
        assert!(!narrow.contains('\u{2588}'), "{narrow}");

        // An unmeasurable terminal is not a narrow one: the code is still drawn.
        let unknown = invitation_text("LABEL", LINK, None);
        assert!(!unknown.contains("Terminal too narrow"), "{unknown}");
        assert!(unknown.contains('\u{2588}'), "{unknown}");

        // More than the largest QR version can carry: the link is still printed.
        let huge = "x".repeat(4_000);
        let unrenderable = invitation_text("LABEL", &huge, Some(400));
        assert!(
            unrenderable.contains("Cannot render a QR code"),
            "{unrenderable}"
        );
        assert!(unrenderable.contains(&huge), "{unrenderable}");
    }

    #[tokio::test]
    async fn remove_all_json_prints_the_emptied_document() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let (code, stdout, stderr) =
            run(&["remote", "devices", "remove", "--all", "--yes", "--json"]).await;
        assert_eq!(code, 0, "{stderr}");
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert!(value["active"].is_null(), "{value}");
        assert_eq!(
            value["devices"].as_array().map(Vec::len),
            Some(0),
            "{value}"
        );
    }

    // ── relay (self-hosted) support ─────────────────────────────────────────

    fn relay_device(pair_id: &str, relay: &str) -> Device {
        Device {
            pair_id: pair_id.into(),
            desktop_id: Some("desk-1".into()),
            control_url: Some(relay.into()),
            nkey_seed: Some("S".into()),
        }
    }

    /// The persisted CA belongs to the relay this installation points at, so it
    /// is used for a pairing on that origin and nowhere else.
    #[tokio::test]
    async fn the_persisted_relay_ca_applies_only_to_its_own_relay() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        std::fs::create_dir_all(home.future()).unwrap();
        std::fs::write(
            home.future().join(RELAY_SETTINGS_FILE),
            json!({ "url": "https://relay.example.com", "ca": "/etc/relay-ca.pem" }).to_string(),
        )
        .unwrap();

        assert_eq!(
            relay_ca_for(&relay_device("pair_A", "https://relay.example.com")),
            Some(PathBuf::from("/etc/relay-ca.pem"))
        );
        // Same relay, spelled with a trailing slash and an explicit port default.
        assert_eq!(
            relay_ca_for(&relay_device("pair_A", "https://relay.example.com/")),
            Some(PathBuf::from("/etc/relay-ca.pem"))
        );
        // A device left over from another relay does not inherit that trust.
        assert_eq!(
            relay_ca_for(&relay_device("pair_B", "https://old.example.net")),
            None
        );
        assert_eq!(
            relay_ca_for(&relay_device("pair_C", "http://relay.example.com")),
            None,
            "a different scheme is a different origin"
        );
        assert_eq!(
            relay_ca_for(&relay_device("pair_D", "https://relay.example.com:8443")),
            None,
            "a different port is a different origin"
        );
    }

    /// The environment variable is the operator naming one CA for this
    /// invocation, so it applies to whatever relay the pairing names.
    #[tokio::test]
    async fn the_environment_ca_applies_to_any_relay() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let _ca = EnvGuard::set(&[("FUTURE_RELAY_CA", std::ffi::OsString::from("/env/ca.pem"))]);
        assert_eq!(
            relay_ca_for(&relay_device("pair_A", "https://anywhere.example")),
            Some(PathBuf::from("/env/ca.pem"))
        );
        // The environment also wins over the file, as it does in the Desktop.
        assert_eq!(
            relay_ca_for(&relay_device("pair_A", "https://relay.example.com")),
            Some(PathBuf::from("/env/ca.pem"))
        );
    }

    /// Nothing configured means the default trust store — not an error.
    #[tokio::test]
    async fn no_relay_ca_means_the_platform_trust_store() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        std::fs::create_dir_all(home.future()).unwrap();
        std::fs::write(
            home.future().join(RELAY_SETTINGS_FILE),
            json!({ "url": "https://relay.example.com" }).to_string(),
        )
        .unwrap();
        assert_eq!(
            relay_ca_for(&relay_device("pair_A", "https://relay.example.com")),
            None
        );
        // A platform pairing has no relay at all.
        let platform_device = Device {
            pair_id: "pair_P".into(),
            desktop_id: None,
            control_url: None,
            nkey_seed: None,
        };
        assert_eq!(relay_ca_for(&platform_device), None);
    }

    /// A configured CA that cannot be read, parsed, or even holds a certificate is
    /// reported here — not left to fail later as an opaque TLS error during the
    /// revoke, which is the one request that cannot be retried by hand.
    #[test]
    fn an_unusable_relay_ca_is_reported() {
        let home = tempfile::tempdir().unwrap();

        let missing = home.path().join("nope.pem");
        let error = client_trusting(Some(&missing)).expect_err("a missing CA file is an error");
        assert!(error.contains("Cannot read the relay CA"), "{error}");

        let empty = home.path().join("empty.pem");
        std::fs::write(&empty, b"").unwrap();
        let error = client_trusting(Some(&empty)).expect_err("an empty CA file is an error");
        assert!(error.contains("holds no certificate"), "{error}");

        // The mistake an operator actually makes: naming a key, a config, or the
        // wrong file. `from_pem` would have accepted it and failed at the TLS
        // handshake much later.
        let not_a_cert = home.path().join("not-a-cert.pem");
        std::fs::write(&not_a_cert, b"not a certificate").unwrap();
        let error = client_trusting(Some(&not_a_cert)).expect_err("a non-PEM file is an error");
        assert!(error.contains("holds no certificate"), "{error}");

        // A PEM block whose body is not DER: rejected as a certificate rather
        // than handed to rustls.
        let broken = home.path().join("broken.pem");
        std::fs::write(
            &broken,
            b"-----BEGIN CERTIFICATE-----\nnot base64\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let error = client_trusting(Some(&broken)).expect_err("a broken PEM is an error");
        assert!(error.contains("not a PEM certificate"), "{error}");
    }

    /// A real root certificate — the platform store this process already trusts —
    /// must still build a client. Guards the validation above from becoming so
    /// strict that every valid CA is refused; skipped where no bundle is readable.
    #[test]
    fn a_real_ca_bundle_builds_a_trusting_client() {
        let candidates = [
            "/etc/ssl/cert.pem",
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/pki/tls/certs/ca-bundle.crt",
        ];
        let Some(bundle) = candidates.iter().map(Path::new).find(|path| path.is_file()) else {
            eprintln!("no platform CA bundle on this host; nothing to check");
            return;
        };
        assert!(
            client_trusting(Some(bundle)).is_ok(),
            "{} must be accepted as a CA bundle",
            bundle.display()
        );
    }

    /// One machine can hold an official device and a self-hosted one at the same
    /// time, and `remove --all` sends each to the issuer that knows it.
    #[tokio::test]
    async fn remove_all_revokes_each_device_at_its_own_issuer() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let platform_requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let platform = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::json(
                "/client/v1/remote/pair/revoke",
                200,
                "{\"ok\":true}",
            )],
            Some(platform_requests.clone()),
        )
        .await;
        let relay_requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let relay = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::json(
                "/v1/rooms/pair_RELAY",
                200,
                "{\"ok\":true}",
            )],
            Some(relay_requests.clone()),
        )
        .await;
        let auth = auth_file();
        std::fs::create_dir_all(auth.parent().unwrap()).unwrap();
        std::fs::write(
            &auth,
            json!({ "future": { "key": "sk-test", "base_url": format!("{platform}/api") } })
                .to_string(),
        )
        .unwrap();
        let key = nkeys::KeyPair::new_user();
        home.write(
            BOOK_FILE,
            &json!({
                "version": 2,
                "active": null,
                "slots": [
                    { "pairId": "pair_PLATFORM", "nkeySeed": "S" },
                    { "pairId": "pair_RELAY", "controlUrl": relay, "nkeySeed": key.seed().unwrap() }
                ]
            })
            .to_string(),
        );

        let (code, _, stderr) = run(&["remote", "devices", "remove", "--all", "--yes"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.is_empty(), "both issuers confirmed: {stderr}");
        assert!(!home.exists(BOOK_FILE));

        let platform_requests = platform_requests.lock().unwrap();
        assert_eq!(
            platform_requests.len(),
            1,
            "the official device goes to the platform"
        );
        assert!(
            platform_requests[0].contains("pair_PLATFORM"),
            "{}",
            platform_requests[0]
        );
        let relay_requests = relay_requests.lock().unwrap();
        assert_eq!(
            relay_requests.len(),
            1,
            "the relay device goes to its relay"
        );
        assert!(
            relay_requests[0].starts_with("DELETE /v1/rooms/pair_RELAY"),
            "{}",
            relay_requests[0]
        );
        assert!(
            relay_requests[0].contains(&format!("x-future-public-key: {}", key.public_key())),
            "{}",
            relay_requests[0]
        );
        assert!(
            !relay_requests[0].contains("authorization:"),
            "the relay has no accounts, so no bearer token may be sent: {}",
            relay_requests[0]
        );
    }

    /// `add` asks the Desktop, so it needs no account of its own — which is what
    /// makes it work in relay mode, where there is no Future login at all.
    #[tokio::test]
    async fn add_needs_no_account() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let _desktop = RunningDesktop::claim(&home);
        let _answer = answer_requests(
            &home,
            json!({
                "pairId": "pair_RELAY",
                "link": "futureos://remote/pair?v=3&code=abc&desktopId=d&desktopKey=U&secureKey=K&secret=S",
                "expiresAt": 1_800_000_000i64,
            }),
        );
        // No auth.json anywhere: relay mode has no account to read.
        assert!(!auth_file().exists());

        let (code, stdout, stderr) = run(&["remote", "devices", "add", "--json"]).await;
        assert_eq!(code, 0, "{stderr}");
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["pairId"], "pair_RELAY");
        assert!(
            value["link"]
                .as_str()
                .is_some_and(|link| link.contains("v=3")),
            "a relay invitation is v3 and is passed through as-is: {value}"
        );
    }

    #[test]
    fn the_platform_url_matches_the_desktops_resolution_order() {
        assert_eq!(
            platform_url_from(&json!({})),
            DEFAULT_PLATFORM_URL.to_string()
        );
        assert_eq!(
            platform_url_from(&json!({ "future": { "base_url": "https://x.test/api" } })),
            "https://x.test"
        );
        assert_eq!(
            platform_url_from(&json!({ "future": { "base_url": "https://x.test/" } })),
            "https://x.test"
        );
        // `platform_base_url` is the Desktop's second choice, not noise.
        assert_eq!(
            platform_url_from(&json!({ "future": { "platform_base_url": "https://y.test/" } })),
            "https://y.test"
        );
        // An explicit base_url wins over platform_base_url.
        assert_eq!(
            platform_url_from(&json!({
                "future": { "base_url": "https://x.test/api", "platform_base_url": "https://y.test" }
            })),
            "https://x.test"
        );
    }

    #[test]
    fn ids_normalise_like_the_apps_display() {
        assert_eq!(normalized("pair_ab12cd"), "AB12CD");
        assert_eq!(normalized("  Ab12Cd "), "AB12CD");
    }

    #[tokio::test]
    async fn the_group_and_each_action_explain_themselves() {
        for args in [
            vec!["remote", "--help"],
            vec!["remote", "devices", "--help"],
            vec!["remote", "devices", "list", "--help"],
            vec!["remote", "devices", "add", "--help"],
            vec!["remote", "devices", "remove", "--help"],
            vec!["remote", "devices", "use", "--help"],
        ] {
            let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            let (out, captured) = Output::memory();
            let code = crate::dispatch(&args, &out).await;
            assert_eq!(code, 0, "{args:?}");
            let text = String::from_utf8(captured.out.lock().unwrap().clone()).unwrap();
            assert!(text.contains("Usage:"), "{args:?}: {text}");
        }
    }

    #[tokio::test]
    async fn an_unknown_action_or_option_is_refused() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (code, _, stderr) = run(&["remote", "bogus"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("Unknown argument: bogus"), "{stderr}");
        let (code, _, stderr) = run(&["remote", "devices", "list", "--bogus"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("Unknown option: --bogus"), "{stderr}");
        let (code, _, stderr) = run(&["remote", "devices", "use"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("Which device?"), "{stderr}");
        let (code, _, stderr) = run(&["remote", "devices", "remove", "1", "2"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("One device at a time"), "{stderr}");
    }

    #[tokio::test]
    async fn list_with_a_device_argument_is_a_mistake_not_a_filter() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        let (code, _, stderr) = run(&["remote", "devices", "list", "AAAA"]).await;
        assert_eq!(code, 1);
        assert!(stderr.contains("takes no device"), "{stderr}");
    }

    #[tokio::test]
    async fn mutating_an_unpaired_machine_is_an_error_not_a_silent_success() {
        let _guard = crate::test_env::lock_env().await;
        let _home = Home::new();
        for args in [
            vec!["remote", "devices", "remove", "1", "--yes"],
            vec!["remote", "devices", "use", "1"],
        ] {
            let (code, _, stderr) = run(&args).await;
            assert_eq!(code, 1, "{args:?}");
            assert!(
                stderr.contains("No devices are paired to this machine."),
                "{args:?}: {stderr}"
            );
        }
    }

    #[tokio::test]
    async fn a_book_with_an_unexpected_shape_is_refused_rather_than_rewritten() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        for (text, expected) in [
            ("[]", "must be a JSON object"),
            (r#"{"slots": {}}"#, "`slots` must be an array"),
            (r#"{"slots": ["not an object"]}"#, "must be a JSON object"),
            (r#"{"slots": [{"desktopId": "d"}]}"#, "has no `pairId`"),
        ] {
            home.write(BOOK_FILE, text);
            let (code, _, stderr) = run(&["remote", "devices"]).await;
            assert_eq!(code, 1, "{text}");
            assert!(stderr.contains(expected), "{text}: {stderr}");
            assert_eq!(home.read(BOOK_FILE), text);
        }
    }

    #[tokio::test]
    async fn a_desktop_holding_the_lock_is_reported_as_running() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(BOOK_FILE, &two_device_book("https://relay.example.com"));
        let app = home.future().join("app");
        std::fs::create_dir_all(&app).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(app.join(DESKTOP_LOCK_FILE))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();

        let (code, stdout, _) = run(&["remote", "devices", "--json"]).await;
        assert_eq!(code, 0);
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["desktopRunning"], true);

        fs2::FileExt::unlock(&lock).unwrap();
        let (_, stdout, _) = run(&["remote", "devices", "--json"]).await;
        let value: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value["desktopRunning"], false);
    }

    #[tokio::test]
    async fn a_platform_refusal_without_a_message_still_reports_the_status() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/client/v1/remote/pair/revoke",
            500,
            "not json",
        )])
        .await;
        let auth = auth_file();
        std::fs::create_dir_all(auth.parent().unwrap()).unwrap();
        std::fs::write(
            &auth,
            json!({ "future": { "key": "sk-test", "base_url": base } }).to_string(),
        )
        .unwrap();
        home.write(
            BOOK_FILE,
            &json!({ "version": 2, "active": null, "slots": [{ "pairId": "pair_ONE", "nkeySeed": "S" }] })
                .to_string(),
        );
        let (code, _, stderr) = run(&["remote", "devices", "remove", "ONE", "--yes"]).await;
        assert_eq!(code, 0);
        assert!(stderr.contains("HTTP 500"), "{stderr}");
        assert!(home.exists(PENDING_REVOKES_FILE));
    }

    #[tokio::test]
    async fn a_relay_that_refuses_the_delete_is_reported_and_the_slot_still_goes() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        let relay = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/v1/rooms/pair_RR",
            500,
            "{\"error\":\"boom\"}",
        )])
        .await;
        let key = nkeys::KeyPair::new_user();
        home.write(
            BOOK_FILE,
            &json!({
                "version": 2,
                "active": null,
                "slots": [{ "pairId": "pair_RR", "controlUrl": relay, "nkeySeed": key.seed().unwrap() }]
            })
            .to_string(),
        );
        let (code, _, stderr) = run(&["remote", "devices", "remove", "RR", "--yes"]).await;
        assert_eq!(code, 0);
        assert!(stderr.contains("did not confirm the removal"), "{stderr}");
        assert!(stderr.contains("boom"), "{stderr}");
        assert!(!home.exists(BOOK_FILE));
    }

    #[tokio::test]
    async fn a_corrupt_revoke_queue_is_reported_not_overwritten() {
        let _guard = crate::test_env::lock_env().await;
        let home = Home::new();
        home.write(PENDING_REVOKES_FILE, "{ broken");
        home.write(
            BOOK_FILE,
            &json!({ "version": 2, "active": null, "slots": [{ "pairId": "pair_ONE", "nkeySeed": "S" }] })
                .to_string(),
        );
        let (code, _, stderr) = run(&["remote", "devices", "remove", "ONE", "--yes"]).await;
        assert_eq!(code, 0);
        assert!(stderr.contains("could not be queued either"), "{stderr}");
        // Still broken, with this revoke not silently dropped into it.
        assert_eq!(home.read(PENDING_REVOKES_FILE), "{ broken");
        // The local unpair happened anyway.
        assert!(!home.exists(BOOK_FILE));
    }

    #[test]
    fn only_y_or_yes_is_consent() {
        for yes in ["y", "Y", "yes", "YES", " yes \n"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "n", "no", "nope", "yess", "1"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }
}
