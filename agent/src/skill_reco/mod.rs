//! Jev-based skill recommendation.
//!
//! The agent exposes one RPC (`suggest_skill`) that, given the user's
//! first-turn text and a set of UNINSTALLED skill candidates, asks Jev
//! (TypeSafe System One) which single skill — if any — best matches. All
//! trigger logic (new session, first message, length cap, login/balance,
//! "user already picked a skill") lives in the calling client; this module
//! only performs the Jev call and the refusal gate.
//!
//! Design constraints carried over from the offline evaluation
//! (`demos/jev-skill-suggest/bench/REPORT.md`):
//! - Jev never self-refuses, so refusal is the caller's job: we add a
//!   `none_of_these` option and treat "its probability >= gate" as "no
//!   recommendation".
//! - A single Choice over the candidates is enough; the two-call verify
//!   stage was dropped for serving (it bought +2 questions for +0.3 s).
//! - The Choice option cap is 255 and `none_of_these` counts, so at most 254
//!   candidates per call (`bench/option-limit.mjs`).

use serde::{Deserialize, Serialize};

/// Jev System One endpoint.
const JEV_URL: &str = "https://api.typesafe.ai/v1/systemone";
/// Model id passed in each request.
const JEV_MODEL: &str = "jev-latest";
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
/// Environment variable holding the Jev API key. Absent key ⇒ silently
/// unavailable (returns None) so the feature is off by default.
const KEY_ENV: &str = "FUTURE_SKILL_RECO_JEV_KEY";

const NONE_OPTION: &str = "none_of_these";

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

/// Input/output token counts Jev reports for one call (used to price it).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct JevUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
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
    let (outcome, usage) = attempt(query, candidates);
    log_outcome(&outcome, usage, query, started.elapsed());
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
fn attempt(query: &str, candidates: &[SkillCandidate]) -> (Outcome, Option<JevUsage>) {
    let query_bytes = query.trim().len();
    let Some(key) = std::env::var(KEY_ENV)
        .ok()
        .filter(|key| !key.trim().is_empty())
    else {
        return (Outcome::NoKey, None);
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
            None,
        );
    }

    let request = build_request(query, candidates);
    let response = match HTTP_CLIENT
        .post(JEV_URL)
        .bearer_auth(key)
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
                None,
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
            None,
        );
    }
    let body: JevResponse = match response.json() {
        Ok(body) => body,
        Err(error) => {
            return (
                Outcome::Failed {
                    reason: format!("unreadable response: {error}"),
                },
                None,
            )
        }
    };
    let usage = body.usage;
    (decide(&body, candidates), usage)
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
    let cost = usage.input_tokens as f64 / 1_000_000.0 * USD_PER_MTOK_INPUT * USD_TO_CNY;
    // Round to 0.01 厘: the raw product carries float noise
    // (0.0026611200000000003) that only makes the log harder to read, while
    // coarser rounding would misreport the price.
    (cost * 10_000.0).round() / 10_000.0
}

/// Emit the attempt's outcome. `info` for real decisions and failures (the
/// default filter keeps those), `debug` for "the feature is not on / nothing to
/// ask", which would otherwise log on every single message.
fn log_outcome(
    outcome: &Outcome,
    usage: Option<JevUsage>,
    query: &str,
    elapsed: std::time::Duration,
) {
    let latency_ms = elapsed.as_millis() as u64;
    let (input_tokens, output_tokens, cost) = usage
        .map(|usage| (usage.input_tokens, usage.output_tokens, cost_cny(usage)))
        .unwrap_or((0, 0, 0.0));
    match outcome {
        Outcome::NoKey => tracing::debug!(
            "skill reco: FUTURE_SKILL_RECO_JEV_KEY is not set; recommendation is off"
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
            cost_cny = cost,
            query = %crate::session::truncate_visible(query.trim(), 40),
            "skill reco: recommended a skill"
        ),
        Outcome::Failed { reason } => tracing::warn!(
            reason = %reason,
            latency_ms,
            input_tokens,
            output_tokens,
            "skill reco: call failed; the message is sent without a card"
        ),
    }
}

/// Build the System One request body: one Choice over the candidates plus
/// `none_of_these`.
fn build_request(query: &str, candidates: &[SkillCandidate]) -> serde_json::Value {
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
        "model": JEV_MODEL,
        "questions": {
            "chunk_0": {
                "type": "choice",
                "question": "The request in `request` needs a skill from `criteria`. Which one, or does none of them help?",
                "criteria": {
                    "options": options,
                    "how_to_judge": "Pick the closest match if any is plausible, otherwise choose none_of_these. Choosing it is a normal answer here, not a fallback."
                }
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

        let usage = JevUsage {
            input_tokens: 8_800,
            output_tokens: 20,
        };
        let elapsed = std::time::Duration::from_millis(487);
        tracing::subscriber::with_default(subscriber, || {
            log_outcome(
                &Outcome::Recommended {
                    skill: "future-image".to_string(),
                    probability: 0.97,
                    none_probability: 0.01,
                },
                Some(usage),
                "帮我把这张照片转成水彩风格",
                elapsed,
            );
            log_outcome(
                &Outcome::Refused {
                    none_probability: 0.61,
                },
                Some(usage),
                "今天天气怎么样",
                elapsed,
            );
            log_outcome(
                &Outcome::Failed {
                    reason: "HTTP 402 Payment Required".to_string(),
                },
                None,
                "whatever",
                elapsed,
            );
            log_outcome(&Outcome::NoKey, None, "whatever", elapsed);
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
            "FUTURE_SKILL_RECO_JEV_KEY is not set",
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
    fn prices_a_call_from_its_input_tokens() {
        // 8.8k input tokens is the measured shape of a 141-skill catalogue:
        // about 2.7 厘 (0.0027 CNY) at $0.042/Mtok.
        let cost = cost_cny(JevUsage {
            input_tokens: 8_800,
            output_tokens: 20,
        });
        assert!((cost - 0.0027).abs() < 0.0002, "unexpected cost: {cost}");
        // Rounded only enough to strip float noise, so the log reads as a price
        // without misreporting it.
        assert_eq!(cost, 0.0027, "unexpected rounding: {cost}");
        // Output tokens are free, so they must not change the price.
        assert_eq!(
            cost,
            cost_cny(JevUsage {
                input_tokens: 8_800,
                output_tokens: 0,
            })
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

    #[test]
    fn build_request_has_none_option_and_state() {
        let cands = vec![cand("a"), cand("b")];
        let req = build_request("hello", &cands);
        let opts = &req["questions"]["chunk_0"]["criteria"]["options"];
        assert!(opts.get(NONE_OPTION).is_some());
        assert!(opts.get("a").is_some() && opts.get("b").is_some());
        assert_eq!(req["state"]["request"], serde_json::json!("hello"));
    }

    #[test]
    fn empty_query_or_candidates_returns_none() {
        assert!(suggest_skill("  ", &[cand("a")]).is_none());
        assert!(suggest_skill("q", &[]).is_none());
    }

    #[test]
    fn no_key_returns_none() {
        // Ensure the env key is absent for this test.
        std::env::remove_var(KEY_ENV);
        assert!(suggest_skill("q", &[cand("a")]).is_none());
    }
}
