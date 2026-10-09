//! Remote-host **client**: this desktop connecting *out* to another desktop's
//! FutureOS, the way a phone does.
//!
//! Two roles live in this crate and must stay separate:
//!
//! | | `remote` (+`remote_host`) | `remote_peer` (here) |
//! |---|---|---|
//! | role | host: phones pair *into* this machine | client: this machine pairs *out* |
//! | credential file | `~/.future/remote_pairing.json` | `~/.future/remote_peers.json` |
//! | owns | the bridge, phone commands, host catalog | this desktop's view of a remote host |
//!
//! **The remote host's sessions are not this machine's sessions.** They are
//! never written to the desktop store (`~/.future/app/app.db`) and never read
//! from the local agent: the remote agent is the only authority for its own
//! transcript, and a local row would be a second copy of it. The client keeps
//! its own snapshot of the remote catalogue and a bounded timeline cache —
//! both projections, both disposable.
//!
//! The wire protocol is the same one the phone speaks (Remote v2): the shared
//! `packages/remote-crypto` crate provides Noise (`XXpsk0` for the initial
//! pairing, `IK` for every reconnect) and the AEAD record layer, and the
//! JSON command/reply shapes are the ones `remote_host::business` serves.
//! Nothing about the protocol is desktop-specific, so a desktop pairs with any
//! host the phone can pair with.

pub(crate) mod creds;
pub(crate) mod link;
pub(crate) mod platform;
pub(crate) mod runtime;
pub(crate) mod session;

pub use self::runtime::{PeerEvent, PeerSummary};

#[cfg(test)]
mod tests;
