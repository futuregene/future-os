//! Rooms: one pairing's namespace on the relay.
//!
//! A *room* is what lets the broker answer "may this connection touch
//! `p.<pair>.>`?" without an account system: the desktop creates the room with
//! a request signed by its pairing NKey, the phone joins it with the one-time
//! token carried inside the invitation, and the relay remembers only what it
//! needs to answer that question.
//!
//! The registry is persisted as one JSON file (when the relay is configured
//! with a path) so a restart does not cost a re-pair. The file holds no secret:
//! pair ids, public NKeys, device names, timestamps and the *hash* of an
//! invitation token. A deployment that deletes it makes every phone re-pair,
//! which is also the only way to revoke a device the desktop cannot reach.

use future_agent::console;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A device the phone/host proved it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub public_key: String,
    pub name: String,
    pub paired_at: i64,
    /// SHA-256 of the refresh credential issued at join. The official platform
    /// stores the same thing, and for the same reason: a stolen room file must
    /// not hand out working credentials.
    pub refresh_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub pair_id: String,
    /// The desktop's NATS user public key. It owns the room and is the only
    /// caller allowed to rotate the join token or delete it.
    pub host_public_key: String,
    /// The desktop's installation id, also the reply-inbox namespace.
    pub host_device_id: String,
    pub host_name: String,
    /// Hex SHA-256 of the invitation token. The token itself never reaches the
    /// relay: the desktop hashes it and sends the digest.
    ///
    /// `None` means this room has no open invitation: the desktop is already
    /// paired and only needs credentials for the device it has.
    pub join_hash: Option<String>,
    /// The device that consumed the invitation, if any. A join token is
    /// single-use for *a device*: the same device may retry (a lost response
    /// must not cost a rescan), a different one may not.
    pub join_consumed_by: Option<String>,
    pub devices: BTreeMap<String, Device>,
    pub created_at: i64,
    pub invitation_expires_at: i64,
    pub expires_at: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomError {
    Unknown,
    Expired,
    InvitationExpired,
    HostMismatch,
    JoinInvalid,
    JoinUsed,
    DeviceUnknown,
    RefreshInvalid,
    BadIdentifier,
}

impl RoomError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unknown => "room_unknown",
            Self::Expired => "room_expired",
            Self::InvitationExpired => "invitation_expired",
            Self::HostMismatch => "room_owned_by_another_host",
            Self::JoinInvalid => "join_invalid",
            Self::JoinUsed => "join_used",
            Self::DeviceUnknown => "device_unknown",
            Self::RefreshInvalid => "invalid_remote_credential",
            Self::BadIdentifier => "identifier_invalid",
        }
    }

    pub fn status(self) -> u16 {
        match self {
            Self::Unknown | Self::Expired => 404,
            Self::InvitationExpired | Self::JoinUsed | Self::HostMismatch => 409,
            Self::JoinInvalid
            | Self::DeviceUnknown
            | Self::RefreshInvalid
            | Self::BadIdentifier => 403,
        }
    }
}

pub fn hash_join(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Identifiers end up inside NATS subjects and inside the Noise prologue.
///
/// A dot would create a new subject level, `*`/`>` would widen a subscription,
/// and an empty segment would make the prologue ambiguous — so the relay is
/// strict where the clients are permissive.
pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

#[derive(Debug)]
pub struct JoinOutcome {
    pub pair_id: String,
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomDescription {
    pub pair_id: String,
    /// The desktop's NATS user key. Public material; it is what the route
    /// compares the caller's signature against.
    pub host_public_key: String,
    pub host_device_id: String,
    pub host_name: String,
    pub devices: Vec<DeviceDescription>,
    pub invitation_open: bool,
    pub invitation_expires_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDescription {
    pub device_id: String,
    pub name: String,
    pub paired_at: i64,
}

pub struct RoomStore {
    rooms: Mutex<HashMap<String, Room>>,
    /// `None` keeps the registry in memory only (tests). A served relay always
    /// sets this, so a restart is not a re-pair.
    path: Option<PathBuf>,
    /// When the registry was last written. Token rotation touches `lastSeen`
    /// and `expiresAt` on every call; persisting that per call would turn many
    /// idle pairings into a write storm, so it is flushed hourly.
    last_flush: Mutex<i64>,
}

impl Default for RoomStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomStore {
    pub fn new() -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
            path: None,
            last_flush: Mutex::new(0),
        }
    }

    /// Open (or create) the persisted registry.
    ///
    /// A corrupt file is an error rather than an empty registry: silently
    /// starting empty would invalidate every pairing at once, which is exactly
    /// the failure this file exists to prevent.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let rooms = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<Vec<Room>>(&bytes)
                .map_err(|error| anyhow::anyhow!("parse {}: {error}", path.display()))?
                .into_iter()
                .map(|room| (room.pair_id.clone(), room))
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => {
                return Err(anyhow::anyhow!("read {}: {error}", path.display()));
            }
        };
        Ok(Self {
            rooms: Mutex::new(rooms),
            path: Some(path.to_path_buf()),
            last_flush: Mutex::new(0),
        })
    }

    pub fn len(&self) -> usize {
        self.rooms.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write the registry back. Best effort: a disk failure must not fail the
    /// authentication that triggered it, but it is loud, because the next
    /// restart would otherwise silently invalidate pairings.
    fn flush(&self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let rooms = self.rooms.lock().unwrap();
        let mut keys: Vec<&String> = rooms.keys().collect();
        keys.sort();
        let ordered: Vec<&Room> = keys.iter().map(|key| &rooms[*key]).collect();
        let outcome = serde_json::to_vec_pretty(&ordered)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                let tmp = path.with_extension("tmp");
                std::fs::write(&tmp, bytes).map_err(|error| error.to_string())?;
                std::fs::rename(&tmp, &path).map_err(|error| error.to_string())
            });
        if let Err(error) = outcome {
            console::eprintln!(
                "future-relay: could not persist {}: {error}",
                path.display()
            );
            return;
        }
        *self.last_flush.lock().unwrap() = now_stamp();
    }

    /// Persist only if the last write is old enough. `lastSeen`/`expiresAt`
    /// drift is not worth a write per credential rotation.
    fn flush_throttled(&self, now: i64) {
        if self.path.is_none() {
            return;
        }
        let due = {
            let last = *self.last_flush.lock().unwrap();
            now - last >= 3_600
        };
        if due {
            self.flush();
        }
    }

    /// Create or refresh the room of one desktop.
    ///
    /// Re-registering is how a desktop survives a relay restart, so it is
    /// idempotent: the same host key keeps the same room, its paired devices
    /// and its join token. A *different* host key for the same pair id is
    /// refused rather than silently taking the namespace over.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &self,
        pair_id: &str,
        host_public_key: &str,
        host_device_id: &str,
        host_name: &str,
        join_hash: Option<&str>,
        now: i64,
        invitation_ttl: i64,
        room_ttl: i64,
    ) -> Result<Room, RoomError> {
        // Only the identifiers that become part of a subject (or of the Noise
        // prologue) are constrained. A host name is a label a person typed, and
        // the shipping clients send things like `FutureOS Desktop (DESKTOP-1)`;
        // refusing that would reject every real device.
        if !valid_identifier(pair_id)
            || !valid_identifier(host_device_id)
            || host_public_key.is_empty()
            || join_hash.is_some_and(|hash| !is_sha256_hex(hash))
        {
            return Err(RoomError::BadIdentifier);
        }
        let join_hash = join_hash.map(|hash| hash.to_ascii_lowercase());
        let room = {
            let mut rooms = self.rooms.lock().unwrap();
            match rooms.entry(pair_id.to_string()) {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    let room = occupied.get_mut();
                    if room.host_public_key != host_public_key {
                        return Err(RoomError::HostMismatch);
                    }
                    room.host_device_id = host_device_id.to_string();
                    room.host_name = host_name.to_string();
                    // `None` keeps the invitation exactly as it was: a desktop
                    // that restarts to refresh a credential must not reopen a
                    // QR code the user already scanned.
                    if let Some(hash) = join_hash {
                        room.join_hash = Some(hash);
                        room.join_consumed_by = None;
                        room.invitation_expires_at = now + invitation_ttl;
                    }
                    room.expires_at = (now + room_ttl).max(room.expires_at);
                    room.last_seen = now;
                    room.clone()
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    let room = Room {
                        pair_id: pair_id.to_string(),
                        host_public_key: host_public_key.to_string(),
                        host_device_id: host_device_id.to_string(),
                        host_name: host_name.to_string(),
                        join_hash,
                        join_consumed_by: None,
                        devices: BTreeMap::new(),
                        created_at: now,
                        invitation_expires_at: now + invitation_ttl,
                        expires_at: now + room_ttl,
                        last_seen: now,
                    };
                    vacant.insert(room.clone());
                    room
                }
            }
        };
        self.flush();
        Ok(room)
    }

    /// Consume the invitation for one device.
    #[allow(clippy::too_many_arguments)]
    pub fn join(
        &self,
        pair_id: &str,
        join_token: &str,
        device_id: &str,
        device_public_key: &str,
        device_name: &str,
        refresh_hash: &str,
        now: i64,
        room_ttl: i64,
    ) -> Result<JoinOutcome, RoomError> {
        if !valid_identifier(device_id) {
            return Err(RoomError::BadIdentifier);
        }
        let outcome = {
            let mut rooms = self.rooms.lock().unwrap();
            let room = rooms.get_mut(pair_id).ok_or(RoomError::Unknown)?;
            if now >= room.expires_at {
                rooms.remove(pair_id);
                return Err(RoomError::Expired);
            }
            if now >= room.invitation_expires_at {
                return Err(RoomError::InvitationExpired);
            }
            let expected = room.join_hash.as_deref().ok_or(RoomError::JoinInvalid)?;
            if hash_join(join_token) != expected {
                return Err(RoomError::JoinInvalid);
            }
            match room.join_consumed_by.as_deref() {
                // A lost join response must not cost a rescan: the same device
                // may present the same invitation again. It still has to prove
                // the matching public key, so this is not a replay oracle for
                // anyone else.
                Some(consumed) => {
                    let bound = room.devices.get(consumed);
                    if consumed != device_id
                        || bound.map(|device| device.public_key.as_str()) != Some(device_public_key)
                    {
                        return Err(RoomError::JoinUsed);
                    }
                }
                None => {
                    room.join_consumed_by = Some(device_id.to_string());
                }
            }
            room.devices.insert(
                device_id.to_string(),
                Device {
                    public_key: device_public_key.to_string(),
                    name: device_name.to_string(),
                    paired_at: now,
                    refresh_hash: refresh_hash.to_string(),
                },
            );
            room.last_seen = now;
            room.expires_at = (now + room_ttl).max(room.expires_at);
            JoinOutcome {
                pair_id: room.pair_id.clone(),
                device_id: device_id.to_string(),
            }
        };
        self.flush();
        Ok(outcome)
    }

    /// A request authenticated as `public_key` wants a credential for a room.
    ///
    /// Returns the device id the caller speaks as, which is what the subject
    /// permissions need for reply inboxes.
    pub fn authorize(
        &self,
        pair_id: &str,
        public_key: &str,
        role: crate::relay::jwt::Role,
        now: i64,
        room_ttl: i64,
    ) -> Result<String, RoomError> {
        let device_id = {
            let mut rooms = self.rooms.lock().unwrap();
            let room = rooms.get_mut(pair_id).ok_or(RoomError::Unknown)?;
            if now >= room.expires_at {
                rooms.remove(pair_id);
                return Err(RoomError::Expired);
            }
            room.last_seen = now;
            room.expires_at = (now + room_ttl).max(room.expires_at);
            match role {
                crate::relay::jwt::Role::Bridge => {
                    if room.host_public_key != public_key {
                        return Err(RoomError::HostMismatch);
                    }
                    room.host_device_id.clone()
                }
                crate::relay::jwt::Role::Client => room
                    .devices
                    .iter()
                    .find(|(_, device)| device.public_key == public_key)
                    .map(|(id, _)| id.clone())
                    .ok_or(RoomError::DeviceUnknown)?,
            }
        };
        self.flush_throttled(now);
        Ok(device_id)
    }

    /// A client role presenting a stored refresh credential.
    ///
    /// The device id and public key must match the device the room remembers,
    /// so a refresh token copied out of one pairing cannot be replayed against
    /// another. The caller is still not authenticated for NATS by this alone:
    /// the resulting JWT is bound to `public_key`, and only the seed can answer
    /// the broker's nonce challenge.
    pub fn authorize_refresh(
        &self,
        pair_id: &str,
        device_id: &str,
        device_public_key: &str,
        refresh_token: &str,
        now: i64,
        room_ttl: i64,
    ) -> Result<JoinOutcome, RoomError> {
        let outcome = {
            let mut rooms = self.rooms.lock().unwrap();
            let room = rooms.get_mut(pair_id).ok_or(RoomError::Unknown)?;
            if now >= room.expires_at {
                rooms.remove(pair_id);
                return Err(RoomError::Expired);
            }
            let device = room
                .devices
                .get(device_id)
                .ok_or(RoomError::DeviceUnknown)?;
            if device.public_key != device_public_key
                || device.refresh_hash != hash_join(refresh_token)
            {
                return Err(RoomError::RefreshInvalid);
            }
            room.last_seen = now;
            room.expires_at = (now + room_ttl).max(room.expires_at);
            JoinOutcome {
                pair_id: room.pair_id.clone(),
                device_id: device_id.to_string(),
            }
        };
        self.flush_throttled(now);
        Ok(outcome)
    }

    /// The room an invitation token belongs to, if any.
    ///
    /// Claims are rare (one per pairing) and the token is only stored hashed,
    /// so the lookup is a scan rather than an index. A relay with enough rooms
    /// for that to matter would also want a database, which this deliberately
    /// is not.
    pub fn room_for_join(&self, join_token: &str) -> Option<String> {
        let digest = hash_join(join_token);
        let now = now_stamp();
        let rooms = self.rooms.lock().unwrap();
        rooms
            .values()
            .find(|room| {
                room.join_hash.as_deref() == Some(digest.as_str())
                    && now < room.expires_at
                    && now < room.invitation_expires_at
            })
            .map(|room| room.pair_id.clone())
    }

    /// Remove one device (a client unpairing itself). The invitation stays
    /// closed; a new device needs a new invitation from the desktop.
    pub fn forget_device(&self, pair_id: &str, device_id: &str) {
        {
            let mut rooms = self.rooms.lock().unwrap();
            if let Some(room) = rooms.get_mut(pair_id) {
                room.devices.remove(device_id);
            }
        }
        self.flush();
    }

    /// Delete a room. Only the host key that owns it may.
    pub fn revoke(&self, pair_id: &str, host_public_key: &str) -> Result<(), RoomError> {
        {
            let mut rooms = self.rooms.lock().unwrap();
            match rooms.get(pair_id) {
                None => return Ok(()),
                Some(room) if room.host_public_key != host_public_key => {
                    return Err(RoomError::HostMismatch)
                }
                Some(_) => {
                    rooms.remove(pair_id);
                }
            }
        }
        self.flush();
        Ok(())
    }

    /// Drop rooms nobody has touched. Called on a timer, never on a request
    /// path, so a long list cannot slow an authentication down.
    pub fn purge(&self, now: i64) -> usize {
        let dropped = {
            let mut rooms = self.rooms.lock().unwrap();
            let before = rooms.len();
            rooms.retain(|_, room| now < room.expires_at);
            before - rooms.len()
        };
        if dropped > 0 {
            self.flush();
        }
        dropped
    }

    /// A snapshot for diagnostics. Never includes the join hash or a credential.
    pub fn describe(&self, pair_id: &str, now: i64) -> Option<RoomDescription> {
        let rooms = self.rooms.lock().unwrap();
        let room = rooms.get(pair_id)?;
        Some(RoomDescription {
            pair_id: room.pair_id.clone(),
            host_public_key: room.host_public_key.clone(),
            host_device_id: room.host_device_id.clone(),
            host_name: room.host_name.clone(),
            devices: room
                .devices
                .iter()
                .map(|(id, device)| DeviceDescription {
                    device_id: id.clone(),
                    name: device.name.clone(),
                    paired_at: device.paired_at,
                })
                .collect(),
            invitation_open: room.join_hash.is_some()
                && room.join_consumed_by.is_none()
                && now < room.invitation_expires_at,
            invitation_expires_at: room.invitation_expires_at,
            expires_at: room.expires_at,
        })
    }
}

fn now_stamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;
    const JOIN: &str = "join-token-0123456789";
    const REFRESH: &str = "refresh-hash-0123456789";

    fn store() -> RoomStore {
        RoomStore::new()
    }

    fn register(store: &RoomStore) -> Room {
        register_with(store, Some(JOIN))
    }

    fn register_with(store: &RoomStore, join: Option<&str>) -> Room {
        store
            .register(
                "PAIR1",
                "UHOST",
                "desktop_a",
                "GUI",
                join.map(hash_join).as_deref(),
                NOW,
                300,
                7_776_000,
            )
            .unwrap()
    }

    #[test]
    fn a_host_registers_and_re_registers_the_same_room() {
        let store = store();
        let first = register(&store);
        assert_eq!(first.pair_id, "PAIR1");
        let again = store
            .register(
                "PAIR1",
                "UHOST",
                "desktop_a",
                "GUI",
                None,
                NOW + 10,
                300,
                7_776_000,
            )
            .unwrap();
        assert_eq!(again.created_at, first.created_at);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn a_second_host_key_cannot_take_a_pair_namespace() {
        let store = store();
        register(&store);
        assert_eq!(
            store
                .register(
                    "PAIR1",
                    "UOTHER",
                    "desktop_b",
                    "GUI",
                    None,
                    NOW,
                    300,
                    7_776_000
                )
                .unwrap_err(),
            RoomError::HostMismatch
        );
    }

    #[test]
    fn the_invitation_is_single_use_per_device_and_idempotent_for_that_device() {
        let store = store();
        register(&store);
        store
            .join(
                "PAIR1", JOIN, "dev_a", "UDEVA", "Pixel", REFRESH, NOW, 7_776_000,
            )
            .unwrap();
        // Same device, same key: a lost response is recoverable.
        store
            .join(
                "PAIR1",
                JOIN,
                "dev_a",
                "UDEVA",
                "Pixel",
                REFRESH,
                NOW + 5,
                7_776_000,
            )
            .unwrap();
        // Another device: refused.
        assert_eq!(
            store
                .join("PAIR1", JOIN, "dev_b", "UDEVB", "iPhone", REFRESH, NOW, 7_776_000)
                .unwrap_err(),
            RoomError::JoinUsed
        );
        // Same device id, different key: refused.
        assert_eq!(
            store
                .join("PAIR1", JOIN, "dev_a", "UOTHER", "Pixel", REFRESH, NOW, 7_776_000)
                .unwrap_err(),
            RoomError::JoinUsed
        );
    }

    #[test]
    fn a_wrong_join_token_is_refused() {
        let store = store();
        register(&store);
        assert_eq!(
            store
                .join(
                    "PAIR1",
                    "not-the-token-0123456",
                    "dev_a",
                    "UDEVA",
                    "Pixel",
                    REFRESH,
                    NOW,
                    1
                )
                .unwrap_err(),
            RoomError::JoinInvalid
        );
    }

    /// A desktop that restarts to refresh a credential must not reopen the QR
    /// code a user already scanned.
    #[test]
    fn re_registering_without_a_hash_keeps_the_invitation_closed() {
        let store = store();
        register(&store);
        store
            .join(
                "PAIR1", JOIN, "dev_a", "UDEVA", "Pixel", REFRESH, NOW, 7_776_000,
            )
            .unwrap();
        register_with(&store, None);
        assert_eq!(
            store
                .join("PAIR1", JOIN, "dev_b", "UDEVB", "iPhone", REFRESH, NOW, 7_776_000)
                .unwrap_err(),
            RoomError::JoinUsed
        );
    }

    #[test]
    fn an_expired_invitation_does_not_expire_the_pairing() {
        let store = store();
        register(&store);
        assert_eq!(
            store
                .join(
                    "PAIR1",
                    JOIN,
                    "dev_a",
                    "UDEVA",
                    "Pixel",
                    REFRESH,
                    NOW + 301,
                    7_776_000
                )
                .unwrap_err(),
            RoomError::InvitationExpired
        );
        // The host can still refresh its credential for the same room.
        assert_eq!(
            store
                .authorize(
                    "PAIR1",
                    "UHOST",
                    crate::relay::jwt::Role::Bridge,
                    NOW + 301,
                    7_776_000
                )
                .unwrap(),
            "desktop_a"
        );
    }

    #[test]
    fn authorization_is_role_scoped() {
        let store = store();
        register(&store);
        store
            .join(
                "PAIR1", JOIN, "dev_a", "UDEVA", "Pixel", REFRESH, NOW, 7_776_000,
            )
            .unwrap();
        assert_eq!(
            store
                .authorize("PAIR1", "UHOST", crate::relay::jwt::Role::Bridge, NOW, 1)
                .unwrap(),
            "desktop_a"
        );
        assert_eq!(
            store
                .authorize("PAIR1", "UDEVA", crate::relay::jwt::Role::Client, NOW, 1)
                .unwrap(),
            "dev_a"
        );
        // The host is not a client, and the client is not the bridge.
        assert_eq!(
            store
                .authorize("PAIR1", "UHOST", crate::relay::jwt::Role::Client, NOW, 1)
                .unwrap_err(),
            RoomError::DeviceUnknown
        );
        assert_eq!(
            store
                .authorize("PAIR1", "UDEVA", crate::relay::jwt::Role::Bridge, NOW, 1)
                .unwrap_err(),
            RoomError::HostMismatch
        );
    }

    #[test]
    fn only_the_host_may_delete_and_deleting_is_idempotent() {
        let store = store();
        register(&store);
        assert_eq!(
            store.revoke("PAIR1", "UOTHER").unwrap_err(),
            RoomError::HostMismatch
        );
        store.revoke("PAIR1", "UHOST").unwrap();
        assert!(store.is_empty());
        store.revoke("PAIR1", "UHOST").unwrap();
    }

    #[test]
    fn purge_drops_only_rooms_past_their_ttl() {
        let store = store();
        register(&store);
        assert_eq!(store.purge(NOW + 1), 0);
        assert_eq!(store.purge(NOW + 7_776_001), 1);
        assert!(store.is_empty());
    }

    /// The shipping clients describe their device with the machine name, which
    /// contains spaces and parentheses. That is a label, not a subject level.
    #[test]
    fn a_display_name_is_free_text_but_the_device_id_is_not() {
        let store = store();
        register_with(&store, Some(JOIN));
        let long_name = "FutureOS Desktop (unmer-desktop-01)";
        store
            .join(
                "PAIR1", JOIN, "dev_a", "UDEVA", long_name, REFRESH, NOW, 7_776_000,
            )
            .expect("a machine name is a valid label");
        store
            .register(
                "PAIR2",
                "UHOST2",
                "desktop_b",
                long_name,
                None,
                NOW,
                300,
                7_776_000,
            )
            .expect("the host name is a label too");
        assert_eq!(
            store
                .join("PAIR1", JOIN, "dev.a", "UDEVA", "x", REFRESH, NOW, 7_776_000)
                .unwrap_err(),
            RoomError::BadIdentifier,
            "the device id does become a subject segment"
        );
    }

    #[test]
    fn identifiers_that_would_widen_a_subject_are_refused() {
        for bad in ["PAIR.1", "PAIR*", "PAIR>", "", "PAIR 1"] {
            assert!(!valid_identifier(bad), "{bad:?} must not be an identifier");
        }
        assert!(valid_identifier("PAIR1"));
        assert!(valid_identifier("desktop_a-1"));
        assert_eq!(
            store()
                .register("PAIR.1", "UHOST", "desktop_a", "GUI", None, NOW, 1, 1)
                .unwrap_err(),
            RoomError::BadIdentifier
        );
    }

    /// A relay restart must not cost a re-pair: the registry comes back from
    /// disk and the phone's existing credential still belongs to a device.
    #[test]
    fn a_restart_keeps_paired_devices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rooms.json");
        {
            let store = RoomStore::open(&path).unwrap();
            register(&store);
            store
                .join(
                    "PAIR1", JOIN, "dev_a", "UDEVA", "Pixel", REFRESH, NOW, 7_776_000,
                )
                .unwrap();
        }
        let reopened = RoomStore::open(&path).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(
            reopened
                .authorize(
                    "PAIR1",
                    "UDEVA",
                    crate::relay::jwt::Role::Client,
                    NOW + 10,
                    7_776_000
                )
                .unwrap(),
            "dev_a"
        );
        assert_eq!(
            reopened
                .authorize(
                    "PAIR1",
                    "UHOST",
                    crate::relay::jwt::Role::Bridge,
                    NOW + 10,
                    7_776_000
                )
                .unwrap(),
            "desktop_a"
        );
        // The invitation is still closed: the persisted room remembers that it
        // was consumed.
        assert_eq!(
            reopened
                .join("PAIR1", JOIN, "dev_b", "UDEVB", "iPhone", REFRESH, NOW, 7_776_000)
                .unwrap_err(),
            RoomError::JoinUsed
        );
    }

    #[test]
    fn a_corrupt_registry_is_an_error_not_an_empty_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rooms.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(RoomStore::open(&path).is_err());
    }

    #[test]
    fn the_persisted_file_holds_no_token_or_key_material() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rooms.json");
        let store = RoomStore::open(&path).unwrap();
        register(&store);
        store
            .join(
                "PAIR1", JOIN, "dev_a", "UDEVA", "Pixel", REFRESH, NOW, 7_776_000,
            )
            .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains(JOIN), "the join token must never be stored");
        assert!(raw.contains("UDEVA"), "public keys are not secret");
    }
}
