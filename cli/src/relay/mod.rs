//! A self-hosted, account-free relay for FutureOS remote control.
//!
//! The official relay is a NATS deployment plus a platform service that mints
//! pair-scoped NATS user JWTs for signed-in accounts. This module keeps the
//! second half — the part the transport actually depends on — and drops the
//! first: no accounts, no billing, no enrollment. A desktop mints its own pair
//! id and invitation, creates a *room* on the relay with a signed request, and
//! the phone joins that room with the one-time token carried in the invitation.
//!
//! What does **not** change is the wire protocol: Noise `XXpsk0`/`IK`, the
//! `FRE2` record layer, subjects, reply binding and the JSON command shapes are
//! identical to the official relay's. Only how a client obtains its NATS
//! credential differs, which is why `future relay serve` can run next to the
//! official relay and a single desktop can serve both at once.
//!
//! The relay is not a trust anchor. It never sees the Noise static keys, the
//! invitation PSK or any application plaintext, and it cannot manufacture a
//! key: the invitation in the QR code is the only thing that binds the two
//! endpoints. See `docs/guide/self-hosted-relay.md` for the threat model.

pub mod cli;
pub mod config;
pub mod jwt;
pub mod room;
pub mod server;
pub mod sign;

pub use config::RelayConfig;
pub use server::{now, router, Relay};
