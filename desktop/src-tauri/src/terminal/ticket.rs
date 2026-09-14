//! One-time tickets for the WebSocket connect path.
//!
//! The control routes authenticate with the process secret in a header, which a
//! browser cannot attach to a WebSocket handshake. Instead the client asks for a
//! short-lived ticket over the authenticated control route and passes it in the
//! connect URL; the ticket is consumed on use and is bound to the exact session
//! it was issued for. This is opencode's connect-token flow.
//!
//! A ticket is *not* an authorization boundary on its own — it only proves the
//! caller could reach the authenticated control route moments ago. It is scoped,
//! single-use and expiring, so a leaked URL cannot be replayed against another
//! session or after the fact.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Tickets live one minute: long enough for a page's connect round-trip, short
/// enough that a captured URL is worthless.
pub const TICKET_TTL: Duration = Duration::from_secs(60);
/// Outstanding tickets; issuing prunes expired entries first.
pub const TICKET_CAPACITY: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketScope {
    pub terminal_id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedTicket {
    pub ticket: String,
    pub expires_in: u64,
}

struct Entry {
    scope: TicketScope,
    expires_at: Instant,
}

pub struct TicketStore {
    entries: Mutex<HashMap<String, Entry>>,
    ttl: Duration,
}

impl Default for TicketStore {
    fn default() -> Self {
        Self::new(TICKET_TTL)
    }
}

impl TicketStore {
    pub fn new(ttl: Duration) -> Self {
        TicketStore {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Issue a ticket for one session. Returns `None` when the store is full of
    /// unexpired tickets — failing closed beats evicting a live ticket.
    pub fn issue(&self, scope: TicketScope) -> Option<IssuedTicket> {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| entry.expires_at > now);
        if entries.len() >= TICKET_CAPACITY {
            return None;
        }
        let ticket = random_ticket();
        entries.insert(
            ticket.clone(),
            Entry {
                scope,
                expires_at: now + self.ttl,
            },
        );
        Some(IssuedTicket {
            ticket,
            expires_in: self.ttl.as_secs().max(1),
        })
    }

    /// Redeem a ticket for exactly the session it was issued for. Single use:
    /// a second attempt (or a mismatched scope) fails.
    pub fn consume(&self, ticket: &str, scope: &TicketScope) -> bool {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap();
        let Some(entry) = entries.get(ticket) else {
            return false;
        };
        if entry.expires_at <= now {
            // Drop it on the way out: an expired ticket is dead weight and must
            // never make the store look full to `issue`.
            entries.remove(ticket);
            return false;
        }
        if &entry.scope != scope {
            return false;
        }
        entries.remove(ticket);
        true
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

fn random_ticket() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| {
            let value: u8 = rng.gen_range(0..16);
            char::from_digit(value as u32, 16).unwrap_or('0')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(terminal: &str) -> TicketScope {
        TicketScope {
            terminal_id: terminal.to_string(),
            thread_id: "thread-1".to_string(),
        }
    }

    #[test]
    fn a_ticket_is_single_use() {
        let store = TicketStore::default();
        let issued = store.issue(scope("t1")).expect("issue");
        assert!(store.consume(&issued.ticket, &scope("t1")));
        assert!(
            !store.consume(&issued.ticket, &scope("t1")),
            "a replayed ticket must be rejected"
        );
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn a_ticket_is_bound_to_its_session() {
        let store = TicketStore::default();
        let issued = store.issue(scope("t1")).expect("issue");
        assert!(
            !store.consume(&issued.ticket, &scope("t2")),
            "a ticket for t1 must not open t2"
        );
        // The mismatch must not consume it either.
        assert!(store.consume(&issued.ticket, &scope("t1")));
    }

    #[test]
    fn an_unknown_ticket_is_rejected() {
        let store = TicketStore::default();
        assert!(!store.consume("deadbeef", &scope("t1")));
    }

    #[test]
    fn an_expired_ticket_is_rejected() {
        let store = TicketStore::new(Duration::from_millis(1));
        let issued = store.issue(scope("t1")).expect("issue");
        std::thread::sleep(Duration::from_millis(20));
        assert!(!store.consume(&issued.ticket, &scope("t1")));
        assert_eq!(
            store.len(),
            0,
            "expired tickets are dropped on the next use"
        );
    }

    #[test]
    fn tickets_are_unique_and_hex() {
        let store = TicketStore::default();
        let a = store.issue(scope("t1")).expect("issue");
        let b = store.issue(scope("t1")).expect("issue");
        assert_ne!(a.ticket, b.ticket);
        assert_eq!(a.ticket.len(), 32);
        assert!(a.ticket.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(a.expires_in >= 1);
    }

    #[test]
    fn issue_prunes_expired_entries_before_the_capacity_check() {
        let store = TicketStore::new(Duration::from_millis(1));
        for _ in 0..TICKET_CAPACITY {
            store.issue(scope("t1")).expect("issue");
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(
            store.issue(scope("t1")).is_some(),
            "expired tickets must not keep the store full"
        );
    }
}
