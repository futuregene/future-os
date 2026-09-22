//! Chat-based approval routing.
//!
//! When the agent wants to run a tool that the active permission level gates, it
//! parks the run and emits an approval request. On a terminal the user answers
//! in place; in a chat the answer arrives as an ordinary message, so the bridge
//! has to recognize it *before* deciding the message is a new prompt.
//!
//! Recognition is deliberately strict. A route is registered only while a
//! request is outstanding for that conversation, the reply must be a bare
//! yes/no (not a sentence that merely starts with one), and an unrecognized
//! message falls through to the agent untouched. A false positive here would
//! swallow a real question, which is a much worse failure than asking again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long an unanswered request stays claimable.
pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);

/// The longest message still treated as a bare yes/no.
const MAX_DECISION_LEN: usize = 12;

const AFFIRMATIVE: &[&str] = &[
    "y", "yes", "yeah", "yep", "ok", "okay", "sure", "approve", "approved", "allow", "allowed",
    "go", "proceed", "accept", "确认", "同意", "允许", "批准", "是", "好", "好的", "可以", "行",
];

const NEGATIVE: &[&str] = &[
    "n",
    "no",
    "nope",
    "deny",
    "denied",
    "reject",
    "rejected",
    "cancel",
    "stop",
    "abort",
    "never",
    "拒绝",
    "不同意",
    "取消",
    "否",
    "不要",
    "不行",
    "别",
];

/// An outstanding approval request for one conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRoute {
    pub session_id: String,
    pub request_id: String,
    pub tool_name: String,
}

/// Outstanding approval requests, keyed by conversation.
#[derive(Default)]
pub struct ApprovalRegistry {
    routes: Mutex<HashMap<String, (ApprovalRoute, Instant)>>,
}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember that `conversation` is waiting on `route`.
    pub fn insert(&self, conversation: &str, route: ApprovalRoute) {
        let mut routes = self.lock();
        routes.insert(conversation.to_string(), (route, Instant::now()));
    }

    /// Take the pending route for `conversation`, if its answer is `text`.
    ///
    /// Returns the route and the verdict. Returns `None` when nothing is
    /// pending, the request expired, or the message is not a bare yes/no — in
    /// which case the caller must treat it as an ordinary message.
    pub fn claim(&self, conversation: &str, text: &str) -> Option<(ApprovalRoute, bool)> {
        let decision = parse_decision(text)?;
        let mut routes = self.lock();
        let (route, created) = routes.get(conversation)?.clone();
        if created.elapsed() > DEFAULT_TTL {
            routes.remove(conversation);
            return None;
        }
        routes.remove(conversation);
        Some((route, decision))
    }

    /// The pending route, if any and not expired.
    pub fn peek(&self, conversation: &str) -> Option<ApprovalRoute> {
        let mut routes = self.lock();
        match routes.get(conversation) {
            Some((route, created)) if created.elapsed() <= DEFAULT_TTL => Some(route.clone()),
            Some(_) => {
                routes.remove(conversation);
                None
            }
            None => None,
        }
    }

    /// Drop a pending route (the run was superseded or cancelled).
    pub fn remove(&self, conversation: &str) {
        self.lock().remove(conversation);
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (ApprovalRoute, Instant)>> {
        self.routes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

/// Read a bare yes/no answer. `true` approves.
pub fn parse_decision(text: &str) -> Option<bool> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|ch| !matches!(ch, '.' | '!' | '?' | '。' | '！' | '？' | '~' | '～'))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.chars().count() > MAX_DECISION_LEN {
        return None;
    }
    // A trailing "please" / "thanks" / "了" still reads as a bare answer.
    let lowered = cleaned.to_lowercase();
    let raw_token = lowered.split_whitespace().next().unwrap_or("");
    let token = raw_token.trim_end_matches(['了', ',', '，']);
    let rest = lowered[raw_token.len().min(lowered.len())..]
        .trim()
        .trim_start_matches(['了', ',', '，'])
        .trim();
    let allowed_tail =
        rest.is_empty() || ["please", "pls", "thanks", "thank you", "thx"].contains(&rest);
    if !allowed_tail {
        return None;
    }
    if AFFIRMATIVE.contains(&token) {
        return Some(true);
    }
    if NEGATIVE.contains(&token) {
        return Some(false);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route() -> ApprovalRoute {
        ApprovalRoute {
            session_id: "s1".into(),
            request_id: "req_1".into(),
            tool_name: "shell".into(),
        }
    }

    #[test]
    fn bare_answers_in_both_languages_are_recognized() {
        for text in [
            "y", "YES", "ok", "approve", "go", "确认", "同意", "好的", "可以",
        ] {
            assert_eq!(parse_decision(text), Some(true), "{text}");
        }
        for text in ["n", "NO", "deny", "reject", "stop", "拒绝", "取消", "不要"] {
            assert_eq!(parse_decision(text), Some(false), "{text}");
        }
    }

    #[test]
    fn punctuation_and_a_polite_tail_are_tolerated() {
        assert_eq!(parse_decision("  Yes!  "), Some(true));
        assert_eq!(parse_decision("ok."), Some(true));
        assert_eq!(parse_decision("no?"), Some(false));
        assert_eq!(parse_decision("yes please"), Some(true));
        assert_eq!(parse_decision("好的了"), Some(true));
    }

    #[test]
    fn a_sentence_that_merely_starts_with_yes_is_not_an_answer() {
        assert_eq!(parse_decision("yes but first check the tests"), None);
        assert_eq!(parse_decision("no, use the other file instead"), None);
        assert_eq!(parse_decision("ok now do something else entirely"), None);
    }

    #[test]
    fn unrelated_or_empty_text_is_not_an_answer() {
        assert_eq!(parse_decision(""), None);
        assert_eq!(parse_decision("   "), None);
        assert_eq!(parse_decision("what is the status"), None);
        assert_eq!(parse_decision("please continue with step two"), None);
    }

    #[test]
    fn an_over_long_message_is_never_read_as_an_answer() {
        assert_eq!(parse_decision("yesyesyesyesyes"), None);
    }

    #[test]
    fn claim_consumes_the_route_once() {
        let registry = ApprovalRegistry::new();
        registry.insert("c1", route());
        let (claimed, decision) = registry.claim("c1", "yes").expect("claim");
        assert_eq!(claimed.request_id, "req_1");
        assert!(decision);
        assert!(
            registry.claim("c1", "yes").is_none(),
            "a route is single-use"
        );
        assert!(registry.is_empty());
    }

    #[test]
    fn a_non_answer_does_not_consume_the_route() {
        let registry = ApprovalRegistry::new();
        registry.insert("c1", route());
        assert!(registry.claim("c1", "what changed?").is_none());
        assert!(
            registry.peek("c1").is_some(),
            "route must survive a question"
        );
    }

    #[test]
    fn claim_requires_a_pending_route_for_that_conversation() {
        let registry = ApprovalRegistry::new();
        assert!(registry.claim("c1", "yes").is_none());
        registry.insert("c1", route());
        assert!(registry.claim("c2", "yes").is_none());
    }

    #[test]
    fn a_negative_answer_rejects() {
        let registry = ApprovalRegistry::new();
        registry.insert("c1", route());
        let (_, decision) = registry.claim("c1", "no").unwrap();
        assert!(!decision);
    }

    #[test]
    fn routes_can_be_dropped_or_read_without_consuming() {
        let registry = ApprovalRegistry::new();
        registry.insert("c1", route());
        assert_eq!(registry.len(), 1);
        assert!(registry.peek("c1").is_some());
        assert_eq!(registry.len(), 1, "peek must not consume");
        registry.remove("c1");
        assert!(registry.is_empty());
    }

    #[test]
    fn an_expired_route_is_not_claimable() {
        let registry = ApprovalRegistry::new();
        registry.insert("c1", route());
        {
            let mut routes = registry.lock();
            let (route, created) = routes.get_mut("c1").unwrap();
            *created = *created - DEFAULT_TTL - Duration::from_secs(1);
            let _ = route;
        }
        assert!(registry.claim("c1", "yes").is_none());
        assert!(registry.is_empty(), "an expired route is dropped");
    }
}
