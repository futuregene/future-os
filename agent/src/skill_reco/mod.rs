//! Jev-based skill recommendation.
//!
//! The agent exposes one RPC (`suggest_skill`) that, given the user's message
//! and a set of UNINSTALLED skill candidates, asks Jev which single skill — if
//! any — best matches. All trigger logic (any turn, length caps, login/balance,
//! daily budget, "user already picked a skill") lives in the calling client;
//! this module only performs the Jev call and the refusal gate.
//!
//! **Endpoint**: Jev is served through the Future account's own gateway,
//! `{future_base_url}/v1/systemone` (`https://future-os.cn/api` +
//! `/v1/systemone`), authenticated with the Future provider credential in
//! `auth.json` and billed to that account. There is no separate Jev
//! credential. The gateway's request shape differs from TypeSafe's public API
//! (see [`build_request`]).
//!
//! Design constraints measured during the offline evaluation:
//! - Jev never self-refuses, so refusal is the caller's job: we add a
//!   `none_of_these` option and treat "its probability >= gate" as "no
//!   recommendation".
//! - A single Choice over the candidates is enough; the two-call verify
//!   stage was dropped for serving (it bought +2 questions for +0.3 s).
//! - The Choice option cap is 255 and `none_of_these` counts, so at most 254
//!   candidates per call (`bench/option-limit.mjs`).

use serde::{Deserialize, Serialize};

/// Refusal gate: `none_of_these` probability at or above this means "no
/// recommendation" (caller-side, because Jev never self-refuses).
const NONE_GATE_THRESHOLD: f64 = 0.15;
/// Hard cap on candidates per call: a Choice accepts at most 255 options and
/// `none_of_these` occupies one of them (measured in bench/option-limit.mjs).
const MAX_CANDIDATES: usize = 254;
/// Per-call timeout, deliberately **inside** every client's own deadline
/// (`RECOMMEND_TIMEOUT_MS` = 3 s in the TUI, the desktop and the phone).
///
/// It was 5 s, which is longer than any client waits: the caller gave up at 3 s,
/// threw the answer away, and this call kept running — and being billed — for
/// two more seconds for a reply nobody could use. Ending it here instead means
/// every caller gets a real outcome inside its budget (a failure is *decided* at
/// 2.5 s rather than abandoned at 3 s) and no call outlives the client that
/// asked for it. The measured p95 is ≈1.4 s
/// (`docs/internals/skill_reco/evaluation.md`), so the tail still fits.
const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2500);
/// How long a failing recommender is left alone before it is asked again.
///
/// Recommendation is best-effort and its failures are invisible to the user by
/// design, so without this every later message after a persistent failure — a
/// spent account, a gateway that is down — paid the whole round-trip again for
/// an answer that could not have changed. 30 s collapses a burst of messages
/// into one call while still recovering on its own after a top-up or a gateway
/// restart.
const UNAVAILABLE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
/// How many [`FailureKind::Transient`] failures in a row mean "the recommender is
/// down" rather than "that one call was unlucky".
const FAILURE_STREAK_TO_OPEN: u32 = 2;
/// The Jev option text is `name + " " + description`, truncated to this many
/// chars (the measured knee: shorter loses answers, longer buys nothing).
const DESC_CHARS: usize = 220;

const NONE_OPTION: &str = "none_of_these";

#[cfg(test)]
use crate::system_one::JEV_MODEL;
use crate::system_one::{endpoint, Endpoint};

/// One skill candidate offered to Jev (also the recommendation shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCandidate {
    pub name: String,
    pub description: String,
}

/// How an attempt ended. Logged so an operator can tell "the call was never
/// made" apart from "the model declined" — the two look identical from the
/// client (no card either way).
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    /// No key configured, so the feature is off. Debug level: this would
    /// otherwise fire on every message.
    NoKey,
    /// Nothing to ask about (blank query, or no candidates to choose from).
    NoInput {
        query_bytes: usize,
        candidates: usize,
    },
    /// The model answered that nothing in the list fits.
    Refused { none_probability: f64 },
    /// The model picked a skill.
    Recommended {
        skill: String,
        probability: f64,
        none_probability: f64,
    },
    /// Transport, HTTP, or decoding failure — the caller sees "no
    /// recommendation" either way, so the reason only exists for this log.
    Failed { reason: String, kind: FailureKind },
    /// Not even attempted: the same account failed recently (see [`Memo`]), so a
    /// retry cannot produce a different answer. The caller sees the same "no
    /// recommendation" as a failure — this line is what tells the two apart in
    /// the log, and what accounts for the call that was not made.
    Skipped { label: String, retry_in_ms: u64 },
}

/// What a failure says about the *next* message, which is what decides whether
/// the recommender is worth asking again (see [`Memo`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    /// The account or the credential: a 401/402/403/429 from the gateway. The
    /// answer will not change per message, so one is enough to stand down —
    /// asking again only spends the round-trip the user is waiting on.
    Account,
    /// Could be a blip: a timeout, a refused connection, a 5xx, or an answer in
    /// a shape this build cannot read. Indistinguishable from "the recommender
    /// is down" until it repeats [`FAILURE_STREAK_TO_OPEN`] times.
    Transient,
}

impl FailureKind {
    /// The phrase the log repeats while this failure stands the recommender
    /// down. Short on purpose: it is emitted once per skipped message, and the
    /// full per-call reason (up to 300 columns of gateway body) is already in
    /// the line that recorded the failure.
    fn label(self) -> &'static str {
        match self {
            FailureKind::Account => "the account cannot produce a recommendation",
            FailureKind::Transient => "the recommender did not answer",
        }
    }
}

/// What a call actually consumed and produced. Both fields come from the
/// response, so an operator can tell a routed call from a direct one.
#[derive(Debug, Clone, Default, PartialEq)]
struct Attempt {
    /// Token counts and the charged amount, when the response carried `usage`.
    usage: Option<JevUsage>,
    /// The backend that answered (`typesafe/jev-1.13-…`), when reported.
    served_by: Option<String>,
}

/// Input/output token counts Jev reports for one call (used to price it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
struct JevUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    /// What the provider charged for this call, in USD. The gateway reports it,
    /// which beats estimating from a price list — Jev's rate changes upstream.
    #[serde(default)]
    cost: Option<f64>,
}

/// Shared blocking client. Initialize lazily on first use, on a blocking
/// dispatcher thread (never on a Tokio worker) — same pattern as
/// `models/future.rs`.
static HTTP_CLIENT: std::sync::LazyLock<reqwest::blocking::Client> =
    std::sync::LazyLock::new(|| {
        future_http::blocking::client_builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_else(|_| future_http::blocking::client())
    });

/// The recommender's recent health, for the life of this process.
///
/// One memo for the process rather than one per session: the failure it encodes
/// (a spent account, a gateway that is down) belongs to the account, and every
/// session on this agent would otherwise learn it again, one wasted call each.
#[derive(Default)]
struct Memo {
    /// The account that is standing down, and until when.
    entry: Option<Unavailable>,
    /// [`FailureKind::Transient`] failures since the last real answer.
    streak: u32,
}

/// One account's stand-down: who it is, why, and for how much longer.
struct Unavailable {
    /// The credential it belongs to. Another account has its own gateway, and a
    /// sign-in or a key change must not inherit this account's failure.
    key: String,
    label: String,
    until: std::time::Instant,
}

impl Memo {
    const fn new() -> Self {
        Self {
            entry: None,
            streak: 0,
        }
    }

    /// The reason to skip a call for `key`, with the time left on the stand-down.
    ///
    /// A pure read: an entry that has expired, or that belongs to another
    /// account, simply does not apply. Nothing is cleared here, because a
    /// message for another account must not discard this one's stand-down.
    fn suppressed(&self, key: &str, now: std::time::Instant) -> Option<(String, u64)> {
        let entry = self.entry.as_ref()?;
        if entry.until <= now || entry.key != key {
            return None;
        }
        Some((entry.label.clone(), (entry.until - now).as_millis() as u64))
    }

    /// Fold one attempt's outcome into the memo.
    fn note(&mut self, key: &str, outcome: &Outcome, now: std::time::Instant) {
        // Another account's failure (or answer) is a different series: the count
        // and the stand-down describe whichever credential is in play, and this
        // memo holds one of them.
        if self.entry.as_ref().is_some_and(|entry| entry.key != key) {
            self.entry = None;
            self.streak = 0;
        }
        let kind = match outcome {
            // The recommender answered — with a pick or with a refusal. It is
            // working, and either is a normal outcome.
            Outcome::Recommended { .. } | Outcome::Refused { .. } => {
                self.entry = None;
                self.streak = 0;
                return;
            }
            Outcome::Failed { kind, .. } => *kind,
            // Nothing was asked (or nothing was attempted), so nothing was
            // learned: the memo stays as it was.
            Outcome::NoKey | Outcome::NoInput { .. } | Outcome::Skipped { .. } => return,
        };
        match kind {
            FailureKind::Account => self.stand_down(key, kind.label(), now),
            FailureKind::Transient => {
                self.streak += 1;
                if self.streak >= FAILURE_STREAK_TO_OPEN {
                    self.stand_down(key, kind.label(), now);
                }
            }
        }
    }

    fn stand_down(&mut self, key: &str, label: &str, now: std::time::Instant) {
        self.entry = Some(Unavailable {
            key: key.to_string(),
            label: label.to_string(),
            until: now + UNAVAILABLE_TTL,
        });
    }
}

/// The process-wide memo, shared by every session on this agent.
fn memo() -> std::sync::MutexGuard<'static, Memo> {
    static MEMO: std::sync::Mutex<Memo> = std::sync::Mutex::new(Memo::new());
    // Poison-tolerant on purpose: a panic in an unrelated request must not turn
    // a best-effort recommendation into a permanently failing one.
    MEMO.lock().unwrap_or_else(|error| error.into_inner())
}

/// Returns the single recommended skill, or `None` when the feature is
/// unavailable, the request fails/times out, or the gate refuses.
///
/// The gate is the caller's (it owns the trigger rules); what lives here beyond
/// the call itself is only "this account just failed, do not spend the user's
/// next message on it" (see [`Memo`]).
///
/// Never panics and never returns an error to the caller: recommendation is
/// best-effort, so every failure mode collapses to "no recommendation" and the
/// client submits normally. That is exactly why each attempt is logged (see
/// [`Outcome`]) — the return value deliberately discards every distinction an
/// operator needs to debug it.
///
pub fn suggest_skill(query: &str, candidates: &[SkillCandidate]) -> Option<SkillCandidate> {
    let started = std::time::Instant::now();
    let (outcome, attempt) = attempt(query, candidates);
    log_outcome(&outcome, attempt, query, started.elapsed());
    match outcome {
        Outcome::Recommended { skill, .. } => candidates
            .iter()
            .find(|candidate| candidate.name == skill)
            .cloned(),
        _ => None,
    }
}

/// Runs one attempt and reports how it ended, with the token usage when the
/// call actually reached Jev.
fn attempt(query: &str, candidates: &[SkillCandidate]) -> (Outcome, Attempt) {
    let Some(endpoint) = endpoint() else {
        // Deliberately not memoized: no credential means no call either way, so
        // there is nothing to save, and a sign-in must be picked up by the very
        // next message rather than up to a TTL later.
        return (Outcome::NoKey, Attempt::default());
    };
    attempt_with(
        &HTTP_CLIENT,
        &endpoint,
        query,
        candidates,
        &mut memo(),
        std::time::Instant::now(),
    )
}

/// One attempt, gated and recorded by an explicit [`Memo`] and clock.
///
/// Split out of [`attempt`] for the same reason as [`attempt_call`]: what the
/// memo must do — stand down on an account failure, stand down after a streak,
/// lift on a real answer, expire on its own — is only testable by driving the
/// clock and the memo from the test rather than by waiting 30 s of wall time,
/// and the "no second call" half needs a gateway the test can count requests on.
fn attempt_with(
    client: &reqwest::blocking::Client,
    endpoint: &Endpoint,
    query: &str,
    candidates: &[SkillCandidate],
    memo: &mut Memo,
    now: std::time::Instant,
) -> (Outcome, Attempt) {
    if let Some((label, retry_in_ms)) = memo.suppressed(&endpoint.key, now) {
        return (Outcome::Skipped { label, retry_in_ms }, Attempt::default());
    }
    let (outcome, attempt) = attempt_call(client, endpoint, query, candidates);
    memo.note(&endpoint.key, &outcome, now);
    (outcome, attempt)
}

/// One attempt against an already-resolved endpoint, over a given client.
///
/// Split out of [`attempt`] for the same reason as [`resolve`]: every transport
/// outcome (a non-2xx body, an unreadable body, a refused connection, a
/// timeout) is decided here, and the only alternative to driving them through a
/// local socket is a live gateway, which no test may depend on.
fn attempt_call(
    client: &reqwest::blocking::Client,
    endpoint: &Endpoint,
    query: &str,
    candidates: &[SkillCandidate],
) -> (Outcome, Attempt) {
    let query_bytes = query.trim().len();
    // Defensive cap: the caller is expected to pre-truncate, but a Choice
    // would hard-400 above 255 options, so clamp here too.
    let candidates = &candidates[..candidates.len().min(MAX_CANDIDATES)];
    if query.trim().is_empty() || candidates.is_empty() {
        return (
            Outcome::NoInput {
                query_bytes,
                candidates: candidates.len(),
            },
            Attempt::default(),
        );
    }

    let request = build_request(query, candidates, &endpoint.model);
    let response = match crate::system_one::blocking_post(client, endpoint, &request) {
        Ok(response) => response,
        Err(error) => {
            return (
                Outcome::Failed {
                    reason: if error.is_timeout() {
                        format!("timeout after {}ms", TIMEOUT.as_millis())
                    } else if error.is_connect() {
                        format!("connection failed: {error}")
                    } else {
                        error.to_string()
                    },
                    // A transport failure may be a blip; the memo counts them.
                    kind: FailureKind::Transient,
                },
                Attempt::default(),
            );
        }
    };
    let status = response.status();
    if !status.is_success() {
        // The body carries Jev's own error (e.g. an invalid key), which is the
        // single most useful thing to log for a 4xx. Truncated: it is an
        // upstream payload, not something this code controls.
        let body = response.text().unwrap_or_default();
        return (
            Outcome::Failed {
                reason: format!(
                    "HTTP {status}: {}",
                    crate::session::truncate_visible(body.trim(), 300)
                ),
                kind: status_kind(status.as_u16()),
            },
            Attempt::default(),
        );
    }
    let body: JevResponse = match response.json() {
        Ok(body) => body,
        Err(error) => {
            return (
                Outcome::Failed {
                    reason: format!("unreadable response: {error}"),
                    kind: FailureKind::Transient,
                },
                Attempt::default(),
            )
        }
    };
    let attempt = Attempt {
        usage: body.usage,
        served_by: body.model.clone(),
    };
    (decide(&body, candidates), attempt)
}

/// The status classes that will not change per message: the account itself is
/// the problem (`401`/`403` a bad or revoked credential, `402` an unpaid or spent
/// one, `429` a rate-limited account), so one of them is enough to stand down.
/// Everything else is treated as possibly-transient, including a 5xx and a 400:
/// a request we get wrong would repeat just as persistently, but only a streak
/// says so, and a streak is what [`FAILURE_STREAK_TO_OPEN`] is for.
fn status_kind(status: u16) -> FailureKind {
    match status {
        401 | 402 | 403 | 429 => FailureKind::Account,
        _ => FailureKind::Transient,
    }
}

/// Apply the refusal gate and pick the top-1 skill, as an [`Outcome`].
fn decide(body: &JevResponse, candidates: &[SkillCandidate]) -> Outcome {
    let Some(probabilities) = body
        .answers
        .get("chunk_0")
        .and_then(|answer| answer.probabilities.as_ref())
    else {
        return Outcome::Failed {
            reason: "response had no chunk_0 probabilities".to_string(),
            kind: FailureKind::Transient,
        };
    };

    let none_probability = probabilities.get(NONE_OPTION).copied().unwrap_or(0.0);
    if none_probability >= NONE_GATE_THRESHOLD {
        return Outcome::Refused { none_probability };
    }

    let Some((name, probability)) = probabilities
        .iter()
        .filter(|(name, _)| name.as_str() != NONE_OPTION)
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
    else {
        return Outcome::Failed {
            reason: "response carried no candidate probabilities".to_string(),
            kind: FailureKind::Transient,
        };
    };
    // The model can only name options this request offered; a name outside that
    // set means a malformed answer, not a recommendation.
    if !candidates.iter().any(|candidate| &candidate.name == name) {
        return Outcome::Failed {
            reason: format!("response named an unknown option: {name}"),
            kind: FailureKind::Transient,
        };
    }
    Outcome::Recommended {
        skill: name.clone(),
        probability: *probability,
        none_probability,
    }
}

/// Price one call at the published rate. Output tokens are free on Jev, so this
/// is input-only; the rate is the documented $0.042/Mtok.
const USD_PER_MTOK_INPUT: f64 = 0.042;
const USD_TO_CNY: f64 = 7.2;

fn cost_cny(usage: JevUsage) -> f64 {
    // The gateway's own figure wins: it is what was actually charged, and it
    // stays right if the upstream price moves. The rate below is the documented
    // fallback for a response that omits `cost`.
    let cost = match usage.cost {
        Some(cost) => cost * USD_TO_CNY,
        None => usage.input_tokens as f64 / 1_000_000.0 * USD_PER_MTOK_INPUT * USD_TO_CNY,
    };
    // Round away float noise (0.0026611200000000003) without losing the value:
    // one call costs a fraction of a 厘, so the previous 0.01-厘 step reported
    // 0.000137 CNY as 0.0001 — a quarter of the price.
    (cost * 1_000_000.0).round() / 1_000_000.0
}

/// Emit the attempt's outcome. `info` for real decisions and failures (the
/// default filter keeps those), `debug` for "the feature is not on / nothing to
/// ask", which would otherwise log on every single message.
fn log_outcome(outcome: &Outcome, attempt: Attempt, query: &str, elapsed: std::time::Duration) {
    let latency_ms = elapsed.as_millis() as u64;
    let (input_tokens, output_tokens, cost) = attempt
        .usage
        .map(|usage| (usage.input_tokens, usage.output_tokens, cost_cny(usage)))
        .unwrap_or((0, 0, 0.0));
    // Which backend answered: a gateway can route the same request elsewhere,
    // and that is invisible in the reply's shape.
    let served_by = attempt.served_by.as_deref().unwrap_or("-");
    match outcome {
        Outcome::NoKey => tracing::debug!(
            "skill reco: not signed in (no Future account credential); recommendation is off"
        ),
        Outcome::NoInput {
            query_bytes,
            candidates,
        } => tracing::debug!(
            query_bytes,
            candidates,
            "skill reco: nothing to ask (blank query or no candidates)"
        ),
        Outcome::Refused { none_probability } => tracing::info!(
            query = %crate::session::truncate_visible(query.trim(), 40),
            none_probability,
            threshold = NONE_GATE_THRESHOLD,
            latency_ms,
            input_tokens,
            output_tokens,
            served_by = %served_by,
            cost_cny = cost,
            "skill reco: nothing in the list fits (no card shown)"
        ),
        Outcome::Recommended {
            skill,
            probability,
            none_probability,
        } => tracing::info!(
            skill = %skill,
            probability,
            none_probability,
            latency_ms,
            input_tokens,
            output_tokens,
            served_by = %served_by,
            cost_cny = cost,
            query = %crate::session::truncate_visible(query.trim(), 40),
            "skill reco: recommended a skill"
        ),
        Outcome::Failed { reason, kind } => tracing::warn!(
            reason = %reason,
            // Which failures will stand the recommender down, and which only
            // count towards a streak: the reason alone does not say.
            kind = ?kind,
            latency_ms,
            input_tokens,
            output_tokens,
            served_by = %served_by,
            "skill reco: call failed; the message is sent without a card"
        ),
        Outcome::Skipped { label, retry_in_ms } => tracing::warn!(
            reason = %label,
            retry_in_ms,
            "skill reco: skipped while the recommender stands down; the message is sent without a card"
        ),
    }
}

/// Build the System One request body: one Choice over the candidates plus
/// `none_of_these`.
///
/// The FutureOS gateway's shape differs from TypeSafe's public API, which the
/// evaluation used: the question goes in `instructions` (not `question`), and
/// `criteria` **is** the option map — nesting it under `options` is silently
/// taken as a choice named "options". Verified against the live gateway.
fn build_request(query: &str, candidates: &[SkillCandidate], model: &str) -> serde_json::Value {
    let mut options = serde_json::Map::new();
    for c in candidates {
        options.insert(c.name.clone(), serde_json::Value::String(option_text(c)));
    }
    options.insert(
        NONE_OPTION.to_string(),
        serde_json::Value::String("No skill in this list would help with the request".to_string()),
    );

    serde_json::json!({
        "state": { "request": query },
        "model": model,
        "questions": {
            "chunk_0": {
                "type": "choice",
                // The judging guidance rides in `instructions`: the gateway takes
                // no separate criteria text, and the load-bearing sentence
                // ("choosing none is a normal answer, not a fallback") must reach
                // the model (see the evaluation: dropping it collapsed refusal).
                "instructions": "The request in `request` needs a skill from `criteria`. Which one, or does none of them help? Pick the closest match if any is plausible, otherwise choose none_of_these. Choosing it is a normal answer here, not a fallback.",
                "criteria": options
            }
        }
    })
}

/// The option text Jev sees: name plus a one-line, length-capped description.
fn option_text(c: &SkillCandidate) -> String {
    let desc: String = c
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let truncated: String = desc.chars().take(DESC_CHARS).collect();
    format!("{} {}", c.name, truncated)
}

/// Apply the refusal gate and pick the top-1 skill from a Jev response.
///
/// Kept as the plain "what should we show" view over [`decide`]; the tests use
/// it, and `suggest_skill` needs the reasoning too, so both go through `decide`.
#[cfg(test)]
fn pick_from_response(body: &JevResponse, candidates: &[SkillCandidate]) -> Option<SkillCandidate> {
    match decide(body, candidates) {
        Outcome::Recommended { skill, .. } => candidates.iter().find(|c| c.name == skill).cloned(),
        _ => None,
    }
}

// ── Jev response decoding ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JevResponse {
    #[serde(default)]
    answers: std::collections::HashMap<String, JevAnswer>,
    /// Token counts for this call; absent on a response that omits `usage`.
    #[serde(default)]
    usage: Option<JevUsage>,
    /// The backend that answered, as the gateway names it
    /// (`typesafe/jev-1.13-20260917`).
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JevAnswer {
    #[serde(default)]
    probabilities: Option<std::collections::HashMap<String, f64>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn cand(name: &str) -> SkillCandidate {
        SkillCandidate {
            name: name.to_string(),
            description: format!("{name} does a thing"),
        }
    }

    fn response_with(probs: &[(&str, f64)]) -> JevResponse {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "chunk_0".to_string(),
            JevAnswer {
                probabilities: Some(probs.iter().map(|(k, v)| (k.to_string(), *v)).collect()),
            },
        );
        JevResponse {
            answers: map,
            usage: None,
            model: None,
        }
    }

    #[test]
    fn decide_reports_the_reason_not_just_the_answer() {
        let cands = vec![cand("future-web"), cand("future-paper")];

        // A pick carries the skill and both probabilities, so the log can show
        // why the card was shown.
        let picked = decide(
            &response_with(&[
                ("future-web", 0.7),
                ("future-paper", 0.2),
                (NONE_OPTION, 0.1),
            ]),
            &cands,
        );
        assert_eq!(
            picked,
            Outcome::Recommended {
                skill: "future-web".to_string(),
                probability: 0.7,
                none_probability: 0.1,
            }
        );

        // A refusal is distinguishable from a failure or from never calling.
        let refused = decide(
            &response_with(&[("future-web", 0.4), (NONE_OPTION, 0.6)]),
            &cands,
        );
        assert_eq!(
            refused,
            Outcome::Refused {
                none_probability: 0.6
            }
        );
    }

    #[test]
    fn decide_flags_a_malformed_response_as_failure() {
        let cands = vec![cand("future-web")];
        // No chunk_0 at all.
        assert!(matches!(
            decide(
                &JevResponse {
                    answers: std::collections::HashMap::new(),
                    usage: None,
                    model: None,
                },
                &cands,
            ),
            Outcome::Failed { .. }
        ));
        // A name that was never offered is a malformed answer, not a
        // recommendation for some skill outside the request.
        let unknown = decide(
            &response_with(&[("future-ghost", 0.9), (NONE_OPTION, 0.05)]),
            &cands,
        );
        assert!(
            matches!(&unknown, Outcome::Failed { reason, kind } if reason.contains("future-ghost") && *kind == FailureKind::Transient),
            "expected an unknown-option failure, got {unknown:?}"
        );
    }

    #[test]
    fn log_outcome_reports_the_decision_with_its_numbers() {
        // The log line is the only place a "why did nothing happen" question can
        // be answered (the return value is just None), so its content is pinned
        // here: a change that drops the skill, the probabilities or the price
        // would make the feature undebuggable without failing anything else.
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buffer = Capture::default();
        // The capture double is what the subscriber stores behind `Write`; its
        // `flush` is part of that contract and must succeed without emitting.
        let mut sink = buffer.clone();
        assert!(std::io::Write::flush(&mut sink).is_ok());
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let buffer = buffer.clone();
                move || buffer.clone()
            })
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .finish();

        let attempt = Attempt {
            usage: Some(JevUsage {
                input_tokens: 8_800,
                output_tokens: 20,
                cost: None,
            }),
            served_by: Some("typesafe/jev-1.13-20260917".to_string()),
        };
        let elapsed = std::time::Duration::from_millis(487);
        // `tracing` recomputes a callsite's interest from a *single* registered
        // dispatch when only one exists, so a concurrent test's scoped
        // subscriber (which has no debug level) can win that race and silence
        // this callsite for the rest of the process — which is why the `NoKey`
        // line below went missing in 2 runs in 5 whenever the whole module ran
        // in parallel. A process-wide debug-capable sink gives that
        // recomputation something to see, and the rebuild re-evaluates the
        // callsites that were already cached before it.
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .with_max_level(tracing::Level::DEBUG)
                .finish(),
        );
        tracing::callsite::rebuild_interest_cache();
        tracing::subscriber::with_default(subscriber, || {
            log_outcome(
                &Outcome::Recommended {
                    skill: "future-image".to_string(),
                    probability: 0.97,
                    none_probability: 0.01,
                },
                attempt.clone(),
                "帮我把这张照片转成水彩风格",
                elapsed,
            );
            log_outcome(
                &Outcome::Refused {
                    none_probability: 0.61,
                },
                attempt.clone(),
                "今天天气怎么样",
                elapsed,
            );
            log_outcome(
                &Outcome::Failed {
                    reason: "HTTP 402 Payment Required".to_string(),
                    kind: FailureKind::Account,
                },
                Attempt::default(),
                "whatever",
                elapsed,
            );
            log_outcome(
                &Outcome::Skipped {
                    label: FailureKind::Account.label().to_string(),
                    retry_in_ms: 21_400,
                },
                Attempt::default(),
                "whatever",
                elapsed,
            );
            log_outcome(&Outcome::NoKey, Attempt::default(), "whatever", elapsed);
            log_outcome(
                &Outcome::NoInput {
                    query_bytes: 6,
                    candidates: 0,
                },
                Attempt::default(),
                "  你好  ",
                elapsed,
            );
        });

        let logged = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        for expected in [
            "skill reco: recommended a skill",
            "skill=future-image",
            "probability=0.97",
            "none_probability=0.01",
            "latency_ms=487",
            "input_tokens=8800",
            "skill reco: nothing in the list fits (no card shown)",
            "threshold=0.15",
            "HTTP 402 Payment Required",
            "skill reco: call failed",
            // A skipped call is a third outcome, and the only one that explains
            // "why did nothing happen" *and* "why was nothing even asked": a
            // shorter line with no usage, but with the time left on the stand-down.
            "skill reco: skipped while the recommender stands down",
            "reason=the account cannot produce a recommendation",
            "retry_in_ms=21400",
            // Which backend answered, and the fallback when none reported.
            "served_by=typesafe/jev-1.13-20260917",
            "no Future account credential",
            // The "feature is off / nothing to ask" arm is debug-level, so it
            // would otherwise fire on every message; its fields still have to
            // be there when it does.
            "skill reco: nothing to ask (blank query or no candidates)",
            "query_bytes=6",
            "candidates=0",
        ] {
            assert!(
                logged.contains(expected),
                "log is missing {expected:?}:\n{logged}"
            );
        }
    }

    #[test]
    fn log_outcome_reports_the_reason_not_just_the_answer() {
        // Guards that a failure carries an operator-usable reason rather than a
        // bare "no recommendation".
        let reason = match decide(
            &response_with(&[("future-ghost", 0.9), (NONE_OPTION, 0.05)]),
            &[cand("future-web")],
        ) {
            Outcome::Failed { reason, .. } => reason,
            other => panic!("expected a failure, got {other:?}"),
        };
        assert!(
            reason.contains("future-ghost"),
            "unhelpful reason: {reason}"
        );
    }

    /// Which gateway statuses are worth asking again is a product decision:
    /// only the account-level ones stand the recommender down on the first hit,
    /// and everything else — including a 5xx and a 400 — counts as a blip that
    /// needs a streak. Asserted as a matrix so a reordered arm cannot pass.
    #[test]
    fn only_account_statuses_stand_the_recommender_down_immediately() {
        for status in [401u16, 402, 403, 429] {
            assert_eq!(status_kind(status), FailureKind::Account, "{status}");
        }
        for status in [400u16, 404, 408, 500, 502, 503, 504, 529] {
            assert_eq!(status_kind(status), FailureKind::Transient, "{status}");
        }
    }

    #[test]
    fn prices_a_call_from_its_input_tokens_when_the_gateway_omits_the_cost() {
        // 8.8k input tokens is the measured shape of a 141-skill catalogue:
        // about 2.7 厘 (0.0027 CNY) at $0.042/Mtok.
        let cost = cost_cny(JevUsage {
            input_tokens: 8_800,
            output_tokens: 20,
            cost: None,
        });
        assert!((cost - 0.0027).abs() < 0.0002, "unexpected cost: {cost}");
        // Rounded only enough to strip float noise (the raw product is
        // 0.0026611200000000003), so the log reads as a price without
        // misreporting it.
        assert_eq!(cost, 0.002661, "unexpected rounding: {cost}");
        // Output tokens are free, so they must not change the price.
        assert_eq!(
            cost,
            cost_cny(JevUsage {
                input_tokens: 8_800,
                output_tokens: 0,
                cost: None,
            })
        );
    }

    /// The gateway reports what it actually charged; that figure wins over the
    /// published rate (which only has to be right when `cost` is absent).
    #[test]
    fn prices_a_call_from_the_reported_cost_when_present() {
        // A live gateway response: 447 in / 54 out charged $1.8774e-05, exactly
        // the input-only rate. The log must show ~0.000135 CNY — the 0.01-厘
        // step this replaced would have printed 0.0001, a quarter of the price.
        let reported = cost_cny(JevUsage {
            input_tokens: 447,
            output_tokens: 54,
            cost: Some(1.8774e-05),
        });
        assert_eq!(reported, 0.000135, "unexpected rounding: {reported}");

        // A cost that disagrees with the rate must be believed, not averaged:
        // this is the number the provider billed.
        let billed = cost_cny(JevUsage {
            input_tokens: 8_800,
            output_tokens: 0,
            cost: Some(0.5),
        });
        assert_eq!(billed, 3.6, "the reported cost must win: {billed}");
    }

    #[test]
    fn picks_top1_when_gate_passes() {
        let cands = vec![cand("future-web"), cand("future-paper")];
        let body = response_with(&[
            ("future-web", 0.7),
            ("future-paper", 0.2),
            (NONE_OPTION, 0.1),
        ]);
        let pick = pick_from_response(&body, &cands);
        assert_eq!(pick.as_ref().map(|c| c.name.as_str()), Some("future-web"));
    }

    #[test]
    fn refuses_when_none_meets_gate() {
        let cands = vec![cand("future-web")];
        let body = response_with(&[("future-web", 0.5), (NONE_OPTION, 0.5)]);
        assert!(pick_from_response(&body, &cands).is_none());
    }

    #[test]
    fn gate_boundary_is_exclusive_below() {
        let cands = vec![cand("future-web")];
        // Just under the gate: still recommend.
        let body = response_with(&[("future-web", 0.9), (NONE_OPTION, 0.149)]);
        assert!(pick_from_response(&body, &cands).is_some());
        // Exactly at the gate: refuse.
        let body = response_with(&[("future-web", 0.85), (NONE_OPTION, 0.15)]);
        assert!(pick_from_response(&body, &cands).is_none());
    }

    #[test]
    fn missing_probabilities_returns_none() {
        let cands = vec![cand("future-web")];
        let body = JevResponse {
            answers: std::collections::HashMap::new(),
            usage: None,
            model: None,
        };
        assert!(pick_from_response(&body, &cands).is_none());
    }

    #[test]
    fn option_text_collapses_whitespace_and_truncates() {
        let c = SkillCandidate {
            name: "x".to_string(),
            description: format!("multi\n  line\t{}", "y".repeat(500)),
        };
        let text = option_text(&c);
        assert!(!text.contains('\n') && !text.contains('\t'));
        // "x " + at most DESC_CHARS chars.
        assert!(text.len() <= 2 + DESC_CHARS + 1);
    }

    /// The gateway's shape: `instructions` carries the question, and `criteria`
    /// **is** the option map. Nesting the options under an `options` key is not
    /// an error there — the model reads "options" as a choice name — so this
    /// pins the flat form.
    #[test]
    fn build_request_matches_the_gateway_shape() {
        let cands = vec![cand("a"), cand("b")];
        let req = build_request("hello", &cands, "jev");
        let question = &req["questions"]["chunk_0"];
        let opts = &question["criteria"];
        assert!(
            opts.get(NONE_OPTION).is_some(),
            "the none option is offered"
        );
        assert!(opts.get("a").is_some() && opts.get("b").is_some());
        assert!(
            opts.get("options").is_none(),
            "the option map must not be nested — the gateway would answer with a \
             literal choice named \"options\""
        );
        assert_eq!(question["type"], serde_json::json!("choice"));
        assert!(
            question["instructions"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "the question travels in `instructions`"
        );
        assert!(
            question.get("question").is_none(),
            "the official API's `question` field is not what this gateway reads"
        );
        // The load-bearing sentence from the evaluation must survive.
        let instructions = question["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("not a fallback"),
            "dropping this collapsed refusal in the evaluation: {instructions}"
        );
        assert_eq!(req["state"]["request"], serde_json::json!("hello"));
        assert_eq!(req["model"], serde_json::json!("jev"));
    }

    #[test]
    fn empty_query_or_candidates_returns_none() {
        assert!(suggest_skill("  ", &[cand("a")]).is_none());
        assert!(suggest_skill("q", &[]).is_none());
    }

    // ── Transport outcomes, driven against a local socket ──────────────────
    //
    // The live gateway cannot be part of a test: these five outcomes (a 2xx, a
    // 4xx carrying Jev's own error, a body that does not decode, a refused
    // connection, a silent peer) are what an operator has to tell apart in the
    // log, and each is decided by a branch in `attempt_call`.

    /// Read one HTTP request completely — headers plus the body `Content-Length`
    /// promises — so a test can assert what the call actually put on the wire.
    ///
    /// Generic over the reader (a real socket in the transport tests) so the
    /// framing rules below can be driven with a reader that hands the request
    /// over in chosen pieces, including a peer that hangs up mid-body.
    fn read_request(stream: &mut impl std::io::Read) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut expected = None;
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
            if expected.is_none() {
                if let Some(head) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&buf[..head]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    expected = Some(head + 4 + length);
                }
            }
            if expected.is_some_and(|total| buf.len() >= total) {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// The reader frames a request by its `Content-Length`, and every stopping
    /// rule is observable rather than assumed: headers split across reads are
    /// joined before the length is taken, a body that arrives after the length
    /// is already known is reassembled instead of being cut at the first read,
    /// and a peer that hangs up before the promised bytes exist ends the read
    /// with what it got instead of spinning on EOF.
    #[test]
    fn read_request_frames_a_request_across_reads_and_stops_at_eof() {
        /// Hands out the request in the given pieces, then EOF. A piece is
        /// drained across reads if the caller's buffer is smaller than it.
        struct Pieces {
            pieces: Vec<Vec<u8>>,
            next: usize,
            offset: usize,
        }
        impl std::io::Read for Pieces {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                while let Some(piece) = self.pieces.get(self.next) {
                    if self.offset < piece.len() {
                        let take = (piece.len() - self.offset).min(buf.len());
                        buf[..take].copy_from_slice(&piece[self.offset..self.offset + take]);
                        self.offset += take;
                        return Ok(take);
                    }
                    self.next += 1;
                    self.offset = 0;
                }
                Ok(0)
            }
        }
        fn pieces(pieces: Vec<&[u8]>) -> Pieces {
            Pieces {
                pieces: pieces.into_iter().map(<[u8]>::to_vec).collect(),
                next: 0,
                offset: 0,
            }
        }

        // The head arrives in two reads: the length cannot be known until the
        // second one completes it.
        let mut headers = pieces(vec![
            b"POST /v1/systemone HTTP/1.1\r\nContent-Len",
            b"gth: 4\r\n\r\nabcd",
        ]);
        let request = read_request(&mut headers);
        assert!(
            request.starts_with("POST /v1/systemone HTTP/1.1"),
            "{request:?}"
        );
        assert!(
            request.ends_with("abcd"),
            "a head split across reads must be reassembled: {request:?}"
        );

        // The length is known after the first read; the body arrives in a
        // second one, and the reader must keep going until it is complete.
        let mut split = pieces(vec![
            b"POST /v1/systemone HTTP/1.1\r\nContent-Length: 5\r\n\r\nab",
            b"cde",
        ]);
        let request = read_request(&mut split);
        assert!(
            request.ends_with("abcde"),
            "a body split across reads must be reassembled: {request:?}"
        );

        // The peer promised 64 bytes and hung up after 5: the reader returns
        // what it received rather than waiting for bytes that never come.
        let mut truncated = pieces(vec![b"POST / HTTP/1.1\r\nContent-Length: 64\r\n\r\nshort"]);
        let request = read_request(&mut truncated);
        assert!(
            request.ends_with("short"),
            "EOF before the promised length must end the read: {request:?}"
        );

        // A peer that says nothing at all is an empty request, not a hang.
        assert_eq!(read_request(&mut pieces(vec![])), "");
    }

    /// A one-shot HTTP server: it answers with `status` + `body` and reports the
    /// request it received back to the test.
    fn one_shot_server(
        status: &'static str,
        body: String,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let _ = sender.send(request);
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        });
        (address, receiver)
    }

    /// A peer that completes the TCP handshake and then says nothing, so only
    /// the client's own deadline can end the call.
    fn silent_server() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_secs(30));
        });
        address
    }

    fn gateway(url: &str) -> Endpoint {
        Endpoint {
            url: url.to_string(),
            key: "acct-key".to_string(),
            model: JEV_MODEL.to_string(),
        }
    }

    /// A gateway reply that picks `pick`, with the usage and attribution a live
    /// response carried.
    fn gateway_response(pick: &str) -> String {
        let mut probabilities = serde_json::Map::new();
        probabilities.insert(pick.to_string(), serde_json::json!(0.9));
        probabilities.insert(NONE_OPTION.to_string(), serde_json::json!(0.01));
        serde_json::json!({
            "answers": { "chunk_0": { "probabilities": probabilities } },
            "usage": { "input_tokens": 447, "output_tokens": 54, "cost": 1.8774e-05 },
            "model": "typesafe/jev-1.13-20260917",
        })
        .to_string()
    }

    /// A 2xx body is decoded, the pick and its billing are reported, and the
    /// request that produced them is the gateway's documented shape.
    #[test]
    fn a_successful_call_reports_the_pick_its_usage_and_what_it_sent() {
        let candidates = vec![cand("future-web"), cand("future-paper")];
        let (url, request) = one_shot_server("200 OK", gateway_response("future-web"));
        let (outcome, attempt) = attempt_call(
            &HTTP_CLIENT,
            &gateway(&format!("{url}/v1/systemone")),
            "帮我把这张照片转成水彩风格",
            &candidates,
        );
        assert_eq!(
            outcome,
            Outcome::Recommended {
                skill: "future-web".to_string(),
                probability: 0.9,
                none_probability: 0.01,
            }
        );
        let usage = attempt.usage.expect("the response reported usage");
        assert_eq!((usage.input_tokens, usage.output_tokens), (447, 54));
        assert_eq!(usage.cost, Some(1.8774e-05));
        assert_eq!(
            attempt.served_by.as_deref(),
            Some("typesafe/jev-1.13-20260917"),
            "which backend answered is invisible in the reply's shape"
        );

        let sent = request
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let (head, body) = sent.split_once("\r\n\r\n").expect("a body was sent");
        assert!(head.starts_with("POST /v1/systemone "), "{head}");
        assert!(
            head.lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer acct-key")),
            "the account credential must authenticate the call: {head}"
        );
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["model"], serde_json::json!("jev"));
        assert_eq!(
            body["state"]["request"],
            serde_json::json!("帮我把这张照片转成水彩风格")
        );
        assert_eq!(
            body["questions"]["chunk_0"]["type"],
            serde_json::json!("choice")
        );
    }

    /// Jev's own error text is the most useful thing in a 4xx, and it is an
    /// upstream payload: trimmed of surrounding whitespace and capped at 300
    /// display columns so one bad gateway cannot flood the log.
    #[test]
    fn a_non_success_status_reports_the_trimmed_truncated_upstream_body() {
        let (url, _request) =
            one_shot_server("402 Payment Required", format!("   {}   ", "x".repeat(500)));
        let (outcome, attempt) =
            attempt_call(&HTTP_CLIENT, &gateway(&url), "hello", &[cand("future-web")]);
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: format!("HTTP 402 Payment Required: {}", "x".repeat(300)),
                // A spent/unpaid account is not going to answer differently for
                // the user's next message, so one 402 stands the recommender
                // down (see `status_kind`).
                kind: FailureKind::Account,
            },
            "a day of log lines must not be one gateway error"
        );
        assert_eq!(attempt, Attempt::default(), "a 402 never reached Jev");
    }

    /// A 2xx whose body is not the gateway's JSON is a failure, not a pick: an
    /// HTML error page must not be mistaken for "no recommendation".
    #[test]
    fn an_unreadable_body_is_reported_instead_of_being_taken_as_an_answer() {
        let (url, _request) = one_shot_server("200 OK", "<html>gateway</html>".to_string());
        let (outcome, _) = attempt_call(&HTTP_CLIENT, &gateway(&url), "hello", &[cand("a")]);
        assert!(
            matches!(&outcome, Outcome::Failed { reason, .. }
                if reason.starts_with("unreadable response: ")),
            "{outcome:?}"
        );
    }

    /// Nothing listening is a connection failure, not a refusal by the model —
    /// the two look identical to the client (no card either way).
    #[test]
    fn a_refused_connection_is_reported_as_a_failure() {
        let url = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            drop(listener); // the port is now closed
            address
        };
        let (outcome, _) = attempt_call(&HTTP_CLIENT, &gateway(&url), "hello", &[cand("a")]);
        assert!(
            matches!(&outcome, Outcome::Failed { reason, .. }
                if reason.starts_with("connection failed: ")),
            "{outcome:?}"
        );
    }

    /// A peer that accepts the connection and never answers must not occupy a
    /// dispatcher thread: the client's deadline ends the call, and the failure
    /// reports that deadline rather than a generic transport error.
    #[test]
    fn a_silent_gateway_hits_the_client_deadline() {
        let client = future_http::blocking::client_builder()
            .timeout(std::time::Duration::from_millis(250))
            .build()
            .unwrap();
        let (outcome, _) = attempt_call(&client, &gateway(&silent_server()), "hello", &[cand("a")]);
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: format!("timeout after {}ms", TIMEOUT.as_millis()),
                // One timeout could be a blip: it takes a streak to stand down.
                kind: FailureKind::Transient,
            }
        );
    }

    /// The two promises this function makes to its caller: a blank query or an
    /// empty candidate list never reaches the wire, and the byte count it
    /// reports is the *trimmed* one (an ideographic space is whitespace too).
    #[test]
    fn a_blank_query_or_an_empty_candidate_list_never_reaches_the_wire() {
        let (url, request) = one_shot_server("200 OK", gateway_response("future-web"));
        let endpoint = gateway(&url);
        for (query, candidates) in [
            ("   ", vec![cand("a")]),
            ("\u{3000}\u{3000}", vec![cand("a")]),
            ("hello", vec![]),
        ] {
            let (outcome, attempt) = attempt_call(&HTTP_CLIENT, &endpoint, query, &candidates);
            assert_eq!(
                outcome,
                Outcome::NoInput {
                    query_bytes: query.trim().len(),
                    candidates: candidates.len(),
                },
                "query {query:?}"
            );
            assert_eq!(attempt, Attempt::default());
        }
        // Nothing was sent, so the server never answered: the receiver must time
        // out rather than yield a request.
        assert!(request
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err());
    }

    /// The defensive cap. A Choice holds at most 255 options and
    /// `none_of_these` is one of them, so 300 candidates must reach the gateway
    /// as 254 candidates + none — not as a request the gateway hard-400s.
    #[test]
    fn a_catalogue_larger_than_the_choice_limit_is_clamped_to_255_options() {
        let candidates: Vec<SkillCandidate> = (0..300)
            .map(|index| SkillCandidate {
                name: format!("skill-{index:03}"),
                description: "does a thing".to_string(),
            })
            .collect();
        let (url, request) = one_shot_server("200 OK", gateway_response("skill-000"));
        let (outcome, _) = attempt_call(
            &HTTP_CLIENT,
            &gateway(&format!("{url}/v1/systemone")),
            "hello",
            &candidates,
        );
        assert!(
            matches!(outcome, Outcome::Recommended { .. }),
            "{outcome:?}"
        );
        let sent = request
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_str(sent.split_once("\r\n\r\n").unwrap().1).unwrap();
        let options = body["questions"]["chunk_0"]["criteria"]
            .as_object()
            .expect("criteria is the option map");
        assert_eq!(options.len(), MAX_CANDIDATES + 1);
        assert!(options.contains_key("skill-000") && options.contains_key("skill-253"));
        assert!(
            !options.contains_key("skill-254"),
            "the 255th candidate would push the Choice past its option cap"
        );
        assert!(options.contains_key(NONE_OPTION));
    }

    /// Probabilities that name only `none_of_these`, below the gate, leave
    /// nothing to pick. That is a malformed answer, not a refusal — and the two
    /// are distinguishable in the log.
    #[test]
    fn probabilities_naming_only_none_are_a_failure_not_a_refusal() {
        let outcome = decide(
            &response_with(&[(NONE_OPTION, 0.05)]),
            &[cand("future-web")],
        );
        assert_eq!(
            outcome,
            Outcome::Failed {
                reason: "response carried no candidate probabilities".to_string(),
                kind: FailureKind::Transient,
            }
        );
    }

    /// The whole path a signed-in install takes: the credential store is read
    /// from an isolated `$HOME`, its `base_url` points at a local socket, the
    /// call is made, and the candidate the gateway picked is returned to the
    /// caller. This is the only test that goes through `suggest_skill` itself
    /// with an endpoint, so it is what pins the pick-mapping arm.
    #[test]
    fn a_signed_in_install_returns_the_candidate_the_gateway_picked() {
        let home = crate::test_support::TestHome::new();
        let (url, request) = one_shot_server("200 OK", gateway_response("future-web"));
        let auth_path = home.auth_path();
        std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_path,
            serde_json::json!({
                "future": {
                    "type": "api_key",
                    "key": "acct-key",
                    "base_url": url,
                }
            })
            .to_string(),
        )
        .unwrap();

        let picked = suggest_skill(
            "帮我把这张照片转成水彩风格",
            &[cand("future-web"), cand("future-paper")],
        );
        assert_eq!(
            picked.as_ref().map(|candidate| candidate.name.as_str()),
            Some("future-web")
        );
        // The account credential authenticated the call, and the gateway URL was
        // derived from the configured base rather than the built-in default.
        let sent = request
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let head = sent.split_once("\r\n\r\n").unwrap().0;
        assert!(head.starts_with("POST /v1/systemone "), "{head}");
        assert!(
            head.lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer acct-key")),
            "{head}"
        );
    }

    /// A URL reqwest cannot even build a request for is neither a timeout nor a
    /// refused connection, and the operator still gets the reason.
    #[test]
    fn a_request_that_cannot_be_built_is_reported_as_a_transport_failure() {
        let (outcome, attempt) =
            attempt_call(&HTTP_CLIENT, &gateway("http://["), "hello", &[cand("a")]);
        match outcome {
            Outcome::Failed { reason, .. } => {
                assert!(!reason.is_empty(), "a failure must carry a reason");
                assert!(
                    !reason.starts_with("timeout after")
                        && !reason.starts_with("connection failed"),
                    "a malformed URL is neither a timeout nor a connect failure: {reason}"
                );
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(attempt, Attempt::default());
    }

    // ── the stand-down (Memo) ─────────────────────────────────────────────

    fn failed(kind: FailureKind) -> Outcome {
        Outcome::Failed {
            reason: "whatever the gateway said".to_string(),
            kind,
        }
    }

    /// Which failures stand the recommender down, and how fast, is decided here —
    /// the clients cannot tell a 402 from a timeout, so nothing upstream of this
    /// can make the distinction.
    #[test]
    fn only_an_account_failure_stands_the_recommender_down_on_the_first_try() {
        let now = std::time::Instant::now();

        let mut account = Memo::new();
        account.note("acct", &failed(FailureKind::Account), now);
        assert_eq!(
            account.suppressed("acct", now),
            Some((FailureKind::Account.label().to_string(), 30_000)),
            "one 402 is enough: asking again cannot change the answer"
        );

        let mut transient = Memo::new();
        transient.note("acct", &failed(FailureKind::Transient), now);
        assert_eq!(
            transient.suppressed("acct", now),
            None,
            "one timeout could be a blip, and a blip is not worth skipping a message over"
        );
        assert_eq!(transient.streak, 1);
        transient.note("acct", &failed(FailureKind::Transient), now);
        assert_eq!(
            transient.suppressed("acct", now),
            Some((FailureKind::Transient.label().to_string(), 30_000)),
            "the second one in a row is a pattern"
        );
    }

    /// An answer — a card or a refusal — means the recommender works, so an older
    /// failure must not skip the next message, and a streak it interrupted starts
    /// over rather than resuming.
    #[test]
    fn an_answer_lifts_the_stand_down_and_resets_the_streak() {
        let now = std::time::Instant::now();
        let mut memo = Memo::new();
        memo.note("acct", &failed(FailureKind::Account), now);
        assert!(memo.suppressed("acct", now).is_some());

        memo.note(
            "acct",
            &Outcome::Refused {
                none_probability: 0.6,
            },
            now,
        );
        assert_eq!(memo.suppressed("acct", now), None, "a refusal is an answer");

        memo.note("acct", &failed(FailureKind::Transient), now);
        memo.note(
            "acct",
            &Outcome::Recommended {
                skill: "future-web".to_string(),
                probability: 0.9,
                none_probability: 0.01,
            },
            now,
        );
        memo.note("acct", &failed(FailureKind::Transient), now);
        assert_eq!(
            memo.suppressed("acct", now),
            None,
            "the streak must have been cleared by the card in between"
        );
    }

    /// The stand-down is bounded and per account: a top-up or a gateway restart
    /// must not need an agent restart, and another account's credential must not
    /// inherit this account's failure.
    #[test]
    fn the_stand_down_expires_and_belongs_to_one_account() {
        let now = std::time::Instant::now();
        let mut memo = Memo::new();
        memo.note("acct-a", &failed(FailureKind::Account), now);
        assert!(memo
            .suppressed("acct-a", now + UNAVAILABLE_TTL / 2)
            .is_some());
        assert_eq!(
            memo.suppressed("acct-b", now),
            None,
            "another account has its own state, and reading does not consume it"
        );

        // When the other account does fail, its failure is what the memo
        // carries — the previous account's entry is replaced, not kept beside it.
        memo.note("acct-b", &failed(FailureKind::Account), now);
        assert!(memo.suppressed("acct-b", now).is_some());
        assert_eq!(memo.suppressed("acct-a", now), None);

        // Expiry lifts it on its own, and the streak still needs two failures in
        // a row (an expiry is not an answer, so it does not clear the count).
        let later = now + UNAVAILABLE_TTL;
        assert_eq!(memo.suppressed("acct-b", later), None);
        let mut fresh = Memo::new();
        fresh.note("acct-b", &failed(FailureKind::Transient), later);
        assert_eq!(fresh.suppressed("acct-b", later), None);
    }

    /// What the memo buys: after a 402, the next message costs no round-trip at
    /// all. This gateway serves exactly one request, so a second call could only
    /// fail — `Skipped` is what proves none was made.
    #[test]
    fn a_spent_account_is_not_asked_again_for_the_next_message() {
        let (url, _request) = one_shot_server("402 Payment Required", "no credits".to_string());
        let endpoint = gateway(&url);
        let mut memo = Memo::new();
        let now = std::time::Instant::now();

        let (outcome, _) = attempt_with(
            &HTTP_CLIENT,
            &endpoint,
            "hello",
            &[cand("future-web")],
            &mut memo,
            now,
        );
        assert!(
            matches!(
                outcome,
                Outcome::Failed {
                    kind: FailureKind::Account,
                    ..
                }
            ),
            "a 402 is an account failure: {outcome:?}"
        );

        let (second, attempt) = attempt_with(
            &HTTP_CLIENT,
            &endpoint,
            "hello again",
            &[cand("future-web")],
            &mut memo,
            now,
        );
        assert_eq!(
            second,
            Outcome::Skipped {
                label: FailureKind::Account.label().to_string(),
                retry_in_ms: 30_000,
            },
            "the next message must not spend a round-trip on the same failure"
        );
        assert_eq!(
            attempt,
            Attempt::default(),
            "and certainly not a billed one"
        );
    }
}
