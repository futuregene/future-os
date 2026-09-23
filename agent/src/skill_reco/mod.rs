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

/// Model id the gateway resolves (`jev` → `typesafe/jev-1.13-…`).
const JEV_MODEL: &str = "jev";
/// Fallback origin when the Future provider has no `base_url` configured.
const DEFAULT_FUTURE_BASE: &str = "https://future-os.cn/api";
/// The provider whose credential authenticates the call.
const FUTURE_PROVIDER: &str = "future";
/// Refusal gate: `none_of_these` probability at or above this means "no
/// recommendation" (caller-side, because Jev never self-refuses).
const NONE_GATE_THRESHOLD: f64 = 0.15;
/// Hard cap on candidates per call: a Choice accepts at most 255 options and
/// `none_of_these` occupies one of them (measured in bench/option-limit.mjs).
const MAX_CANDIDATES: usize = 254;
/// Per-call timeout. The client also races this RPC against its own deadline,
/// but the server-side timeout guarantees the HTTP call itself never hangs a
/// dispatcher worker indefinitely.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// The Jev option text is `name + " " + description`, truncated to this many
/// chars (the measured knee: shorter loses answers, longer buys nothing).
const DESC_CHARS: usize = 220;

const NONE_OPTION: &str = "none_of_these";

/// Where a call goes and what authenticates it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Endpoint {
    url: String,
    key: String,
    model: String,
}

/// Resolve the endpoint from the Future account: its credential authenticates
/// the call and its base URL hosts the gateway. Jev is served by the Future
/// provider and nowhere else, so there is deliberately no separate credential
/// to configure.
///
/// `None` means the feature is unavailable (not signed in), which the caller
/// treats the same as "no recommendation" — the feature is off, not broken.
fn endpoint() -> Option<Endpoint> {
    let auth = crate::auth::AuthStore::load();
    let key = auth.get(FUTURE_PROVIDER)?;
    let base = auth
        .base_url(FUTURE_PROVIDER)
        .unwrap_or_else(|| DEFAULT_FUTURE_BASE.to_string());
    Some(Endpoint {
        url: systemone_url(&base),
        key,
        model: JEV_MODEL.to_string(),
    })
}

/// `{base}/v1/systemone`, tolerating a base with or without a trailing slash or
/// an already-appended `/v1`.
fn systemone_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1/systemone") {
        return trimmed.to_string();
    }
    let origin = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    format!("{origin}/v1/systemone")
}

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
    Failed { reason: String },
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
        reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    });

/// Returns the single recommended skill, or `None` when the feature is
/// unavailable, the request fails/times out, or the gate refuses.
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
    let query_bytes = query.trim().len();
    let Some(endpoint) = endpoint() else {
        return (Outcome::NoKey, Attempt::default());
    };
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
    let response = match HTTP_CLIENT
        .post(&endpoint.url)
        .bearer_auth(&endpoint.key)
        .json(&request)
        .send()
    {
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
                },
                Attempt::default(),
            )
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

/// Apply the refusal gate and pick the top-1 skill, as an [`Outcome`].
fn decide(body: &JevResponse, candidates: &[SkillCandidate]) -> Outcome {
    let Some(probabilities) = body
        .answers
        .get("chunk_0")
        .and_then(|answer| answer.probabilities.as_ref())
    else {
        return Outcome::Failed {
            reason: "response had no chunk_0 probabilities".to_string(),
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
        };
    };
    // The model can only name options this request offered; a name outside that
    // set means a malformed answer, not a recommendation.
    if !candidates.iter().any(|candidate| &candidate.name == name) {
        return Outcome::Failed {
            reason: format!("response named an unknown option: {name}"),
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
        Outcome::Failed { reason } => tracing::warn!(
            reason = %reason,
            latency_ms,
            input_tokens,
            output_tokens,
            served_by = %served_by,
            "skill reco: call failed; the message is sent without a card"
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
            matches!(&unknown, Outcome::Failed { reason } if reason.contains("future-ghost")),
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
                },
                Attempt::default(),
                "whatever",
                elapsed,
            );
            log_outcome(&Outcome::NoKey, Attempt::default(), "whatever", elapsed);
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
            // Which backend answered, and the fallback when none reported.
            "served_by=typesafe/jev-1.13-20260917",
            "no Future account credential",
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
            Outcome::Failed { reason } => reason,
            other => panic!("expected a failure, got {other:?}"),
        };
        assert!(
            reason.contains("future-ghost"),
            "unhelpful reason: {reason}"
        );
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

    /// The gateway's URL is derived from the Future provider's base URL, which
    /// may or may not already carry a path suffix.
    #[test]
    fn the_systemone_url_is_derived_from_whatever_base_is_configured() {
        for base in [
            "https://future-os.cn/api",
            "https://future-os.cn/api/",
            "https://future-os.cn/api/v1",
            "https://future-os.cn/api/v1/systemone",
        ] {
            assert_eq!(
                systemone_url(base),
                "https://future-os.cn/api/v1/systemone",
                "base {base}"
            );
        }
        // A bare origin is fine too, and a staging host keeps its own path.
        assert_eq!(
            systemone_url("https://staging.example.com"),
            "https://staging.example.com/v1/systemone"
        );
        assert_eq!(
            systemone_url("https://staging.example.com/gw"),
            "https://staging.example.com/gw/v1/systemone"
        );
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

    /// With no credential the feature is simply off. This must not depend on the
    /// developer's own `auth.json`, so it runs against an isolated home.
    #[test]
    fn without_a_credential_the_feature_is_off() {
        let home = crate::test_support::TestHome::new();
        assert!(
            endpoint().is_none(),
            "a fresh home has no Future account credential"
        );
        assert!(suggest_skill("q", &[cand("a")]).is_none());
        drop(home);
    }

    /// A call goes to the Future account's gateway, authenticated by that
    /// account's credential — Jev has no separate key to configure. The base URL
    /// follows the provider entry, with the documented origin as the fallback.
    #[test]
    fn the_endpoint_is_the_future_accounts_gateway() {
        // The provider's own base URL.
        let home = crate::test_support::TestHome::new();
        write_auth(home.path(), "acct-key", Some("https://future-os.cn/api"));
        let resolved = endpoint().expect("the account credential is used");
        assert_eq!(resolved.key, "acct-key");
        assert_eq!(resolved.url, "https://future-os.cn/api/v1/systemone");
        assert_eq!(resolved.model, "jev");
        drop(home);

        // No base URL configured: the account is still usable, at the default
        // origin. A custom gateway (staging, self-hosted) keeps its own path.
        let home = crate::test_support::TestHome::new();
        write_auth(home.path(), "acct-key", None);
        assert_eq!(
            endpoint().expect("resolves").url,
            "https://future-os.cn/api/v1/systemone"
        );
        drop(home);

        let home = crate::test_support::TestHome::new();
        write_auth(
            home.path(),
            "acct-key",
            Some("https://staging.example.com/gw"),
        );
        assert_eq!(
            endpoint().expect("resolves").url,
            "https://staging.example.com/gw/v1/systemone"
        );
        drop(home);
    }

    /// Write the `auth.json` a signed-in install has, under an isolated home.
    fn write_auth(home: &std::path::Path, key: &str, base_url: Option<&str>) {
        let dir = home.join(".future/agent");
        std::fs::create_dir_all(&dir).expect("auth dir");
        let base = match base_url {
            Some(url) => format!(r#","base_url":"{url}""#),
            None => String::new(),
        };
        std::fs::write(
            dir.join("auth.json"),
            format!(r#"{{"future":{{"type":"api_key","key":"{key}"{base}}}}}"#),
        )
        .expect("write auth");
    }
}
