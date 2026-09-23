//! Transport plumbing shared by channel providers.
//!
//! Channels talk to their platform over HTTP, a WebSocket, an inbound webhook,
//! or a raw line protocol. Each of those has one recurring concern that is the
//! same on every platform — retrying an HTTP call without hammering a rate
//! limit, keeping a socket up across a flaky network, verifying a webhook
//! signature, folding long text into platform-sized pieces — so it lives here
//! instead of being re-derived per provider.

pub mod cipher;
pub mod http;
pub mod signature;
pub mod text;
pub mod webhook;
pub mod ws;

pub use http::{ErrorClass, HttpResponse, RetryPolicy};
pub use text::{chunk, is_blank, truncate, LengthUnit};
pub use ws::{Backoff, Socket};
