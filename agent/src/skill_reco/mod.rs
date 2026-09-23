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
/// client submits normally.
pub fn suggest_skill(query: &str, candidates: &[SkillCandidate]) -> Option<SkillCandidate> {
    let key = std::env::var(KEY_ENV)
        .ok()
        .filter(|k| !k.trim().is_empty())?;
    if query.trim().is_empty() || candidates.is_empty() {
        return None;
    }
    // Defensive cap: the caller is expected to pre-truncate, but a Choice
    // would hard-400 above 255 options, so clamp here too.
    let candidates = &candidates[..candidates.len().min(MAX_CANDIDATES)];

    let request = build_request(query, candidates);
    let response = HTTP_CLIENT
        .post(JEV_URL)
        .bearer_auth(key)
        .json(&request)
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: JevResponse = response.json().ok()?;
    pick_from_response(&body, candidates)
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
fn pick_from_response(body: &JevResponse, candidates: &[SkillCandidate]) -> Option<SkillCandidate> {
    let chunk = body.answers.get("chunk_0")?;
    let probs = chunk.probabilities.as_ref()?;

    let none_p = probs.get(NONE_OPTION).copied().unwrap_or(0.0);
    if none_p >= NONE_GATE_THRESHOLD {
        return None;
    }

    // Top-1 candidate by probability (excluding the none option).
    let best = probs
        .iter()
        .filter(|(k, _)| k.as_str() != NONE_OPTION)
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    candidates.iter().find(|c| &c.name == best.0).cloned()
}

// ── Jev response decoding ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JevResponse {
    #[serde(default)]
    answers: std::collections::HashMap<String, JevAnswer>,
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
        JevResponse { answers: map }
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
