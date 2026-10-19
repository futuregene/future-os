//! `future remote devices` — the devices paired to this machine's remote
//! desktop, from a terminal.
//!
//! The window's Remote page manages the same list through Tauri commands; a
//! headless server has no window, and until now its only knob was
//! `futureos-headless --re-pair`, which revokes *every* device and issues a
//! replacement invitation. That answers "switch phones" and nothing else: a
//! machine with several paired clients could not say who they were, could not
//! drop one of them, and could not choose which one drives it.
//!
//! # What is authoritative
//!
//! The pairing book on disk (`~/.future/remote_pairings.json`, or the
//! pre-multi-device `remote_pairing.json` read as a book with one slot) is what
//! the Desktop itself reads at start and on every command path. That makes it
//! the honest thing for this command to read and edit, and it means the command
//! needs nothing running.
//!
//! The document is parsed and rewritten as a [`serde_json::Value`] rather than a
//! mirrored struct: the fields this command does not understand (the NKey seed,
//! the handshake identity, whatever a newer Desktop adds) must survive an edit
//! untouched. A typed copy here would silently drop them.
//!
//! # What a running Desktop does with an edit
//!
//! - **Remove** is effective immediately in the only sense that matters: the
//!   device is revoked at the platform (or its relay room is deleted) *before*
//!   the local slot is dropped, so even a Desktop that still holds the old
//!   credential in memory cannot keep serving it — its next credential refresh
//!   is refused and it forgets the device itself. Revoking first is also why a
//!   lost local write is harmless: the credential is already dead.
//! - **Use** is persisted for the next start. Control is an in-memory pointer in
//!   the running process (a device taking over must move it too), so a running
//!   Desktop keeps its current controller until it restarts.
//!
//! A revoke that cannot be delivered is queued where the Desktop already looks
//! (`remote_pending_revokes.json`), so stopping here never loses it.

use crate::constants::{auth_file, DEFAULT_PLATFORM_URL};
use crate::help;
use crate::output::Output;
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// Everything below the group, and above the three actions.
async fn devices(args: &[String], out: &Output) -> Result<(), String> {
    let (action, rest) = match args.first().map(String::as_str) {
        None => ("list", args),
        Some("--help" | "-h") => {
            out.log(help::REMOTE_HELP);
            return Ok(());
        }
        Some("list" | "remove" | "use") => (args[0].as_str(), &args[1..]),
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
            "remove" => help::REMOTE_REMOVE_HELP,
            _ => help::REMOTE_USE_HELP,
        });
        return Ok(());
    }
    match action {
        "list" => list(rest, out),
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

// ── remove ──────────────────────────────────────────────────────────────────

async fn remove(args: &[String], out: &Output) -> Result<(), String> {
    let values = positional(args, &["--json", "--yes"], help::REMOTE_USAGE)?;
    let json_flag = args.iter().any(|argument| argument == "--json");
    let yes = args.iter().any(|argument| argument == "--yes");
    let selector = one_device(values, help::REMOTE_USAGE)?;
    let mut book = Book::load()?;
    if book.devices.is_empty() {
        return Err("No devices are paired to this machine.".to_string());
    }
    let index = book.position(&selector)?;
    let device = book.devices[index].clone();
    if !yes && !confirmed(&device, book.devices.len() - 1)? {
        out.log_err("Cancelled. Nothing was changed.");
        return Ok(());
    }
    // Revoke before forgetting locally: a device whose slot write is lost can
    // still only reconnect with a credential the platform has just refused,
    // and the Desktop retires such a slot on its next start.
    let warning = revoke(&device).await;
    book.remove(index);
    book.save()?;
    if let Some(warning) = &warning {
        out.log_err(warning);
    }
    let running = desktop_running();
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
        out.log(
            "A Desktop is running; it stops serving this device on its next credential refresh.",
        );
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
    if !stdin_is_terminal() {
        return Err(format!(
            "Refusing to unpair {} without confirmation from a terminal. Re-run with --yes \
             if that is what you mean.",
            display_pair_id(&device.pair_id)
        ));
    }
    eprint!(
        "Unpair {}? It loses remote access to this machine; {} {} paired. [y/N] ",
        display_pair_id(&device.pair_id),
        remaining,
        if remaining == 1 {
            "device stays"
        } else {
            "devices stay"
        }
    );
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
    let response = client()?
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
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("Cannot create an HTTP client: {error}"))
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
