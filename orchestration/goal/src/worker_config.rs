//! Provenance of the model + thinking level a worker run actually uses.
//!
//! ## The failure this exists to prevent
//!
//! `--model` / `--thinking-level` are per-run flags with no goal-level
//! default, so relaunching a worker means retyping the whole
//! `future goal run …` line — and a line gets pasted again and again. Measured
//! on one real goal: thirteen dispatches carried the same
//! `--model … --thinking-level high --max-turns 8`, only the first few were a
//! different model, and nothing re-judged the choice in between. Worse, an
//! explicit flag was applied silently, so no log line and no ledger field said
//! "this worker left the supervisor's configuration".
//!
//! Two things follow, and both live here:
//!
//! 1. **The default is the supervisor's configuration.** The session that
//!    supervises a goal is the one whose model/thinking level the operator
//!    actually chose for this work, so a worker inherits it unless a flag says
//!    otherwise. Only when the goal has no live supervisor session does a
//!    fresh worker fall back to the agent's own defaults.
//! 2. **Every run records which of those happened.** [`WorkerConfig`] is
//!    stamped into the run header (`model`, `thinking_level`, `model_source`)
//!    and printed on the console, so "why is this worker on that model" is
//!    answerable from the run log alone.
//!
//! Resolution is a pure function ([`resolve`]) so all four cases are testable
//! without an agent: explicit override, inherited from the supervisor, agent
//! default, and a resumed session that keeps the configuration it already has.

use serde::Serialize;

/// Which decision produced the configuration a run used.
///
/// Serialized lowercase into the run header. `Session` is the resumed case: the
/// run applied no configuration at all, so the values belong to the session it
/// resumed (a relaunch of in-flight work must not rewrite a conversation's
/// model underneath it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    /// Inherited from the goal's supervising session.
    Supervisor,
    /// The caller passed `--model` and/or `--thinking-level`.
    Override,
    /// No supervisor configuration available: the agent's own defaults stand.
    Default,
    /// A resumed session keeps the configuration it already had.
    Session,
}

impl ConfigSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigSource::Supervisor => "supervisor",
            ConfigSource::Override => "override",
            ConfigSource::Default => "default",
            ConfigSource::Session => "session",
        }
    }
}

/// The configuration a run used, as recorded in its header and printed on the
/// console. `model` / `thinking_level` are the values read back from the agent
/// after applying the decision, not merely what the caller intended.
///
/// Serialized field names match the run header (`model`, `thinking_level`,
/// `model_source`) by construction, so there is one spelling of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerConfig {
    pub model: String,
    pub thinking_level: String,
    #[serde(rename = "model_source")]
    pub source: ConfigSource,
}

impl WorkerConfig {
    /// A configuration whose values could not be read back from the agent.
    /// Recorded as `unknown` rather than an empty string so a reader can tell
    /// "not queried" apart from "the agent reports nothing".
    pub fn unknown(source: ConfigSource) -> Self {
        Self {
            model: "unknown".to_string(),
            thinking_level: "unknown".to_string(),
            source,
        }
    }
}

impl Default for WorkerConfig {
    /// "Nothing was decided": agent defaults, values unknown. Used where a run
    /// exists but no dispatch resolved its configuration (tests that drive the
    /// executor directly), so the header still carries all three fields.
    fn default() -> Self {
        Self::unknown(ConfigSource::Default)
    }
}

/// What [`resolve`] decided: what to send, and which case decided it.
///
/// `None` means "send nothing" — an inherited value must not be re-sent when
/// there is nothing to inherit, and the agent's default must stay reachable
/// without this layer guessing what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub set_model: Option<String>,
    pub set_thinking: Option<String>,
    pub source: ConfigSource,
}

impl Resolution {
    fn new(model: Option<&str>, thinking: Option<&str>, source: ConfigSource) -> Self {
        Self {
            set_model: model.map(str::to_string),
            set_thinking: thinking.map(str::to_string),
            source,
        }
    }
}

/// Decide the worker's configuration for this run.
///
/// - `explicit_model` / `explicit_thinking`: the `--model` / `--thinking-level`
///   flags. Either one present makes the run an [`ConfigSource::Override`];
///   the flag that is *absent* still inherits from the supervisor, because
///   picking a model should not silently reset the thinking level.
/// - `supervisor`: the supervising session's `(model, thinking_level)`, when
///   the goal has one and it could be read.
/// - `resumed`: the run resumes an existing session instead of minting one. A
///   resumed session is never reconfigured from the supervisor — only explicit
///   flags touch it.
///
/// Nothing is resolved to a hard-coded model or level: the point of inheriting
/// is to use the operator's choice, and when there is none the agent's own
/// default is the correct answer, not a guess made here.
pub fn resolve(
    explicit_model: Option<&str>,
    explicit_thinking: Option<&str>,
    supervisor: Option<(&str, &str)>,
    resumed: bool,
) -> Resolution {
    let explicit_model = explicit_model.filter(|v| !v.trim().is_empty());
    let explicit_thinking = explicit_thinking.filter(|v| !v.trim().is_empty());
    let overridden = explicit_model.is_some() || explicit_thinking.is_some();

    if resumed {
        // Relaunching in-flight work: the conversation keeps the configuration
        // it already has, so only explicit flags touch it. Inheriting here would
        // rewrite a running conversation's model because the supervisor happens
        // to be on a different one — the opposite of "resume where you were".
        let source = if overridden {
            ConfigSource::Override
        } else {
            ConfigSource::Session
        };
        return Resolution::new(explicit_model, explicit_thinking, source);
    }

    let source = if overridden {
        ConfigSource::Override
    } else if supervisor.is_some() {
        ConfigSource::Supervisor
    } else {
        ConfigSource::Default
    };
    let model = explicit_model.or_else(|| supervisor.map(|(model, _)| model));
    let thinking = explicit_thinking.or_else(|| supervisor.map(|(_, level)| level));
    Resolution::new(model, thinking, source)
}

/// The one console line a dispatch prints about its configuration.
///
/// An override is the only case that must be visible, and it carries the
/// supervisor's values inline: "worker left the supervisor's configuration" is
/// exactly the fact a pasted command line hides.
pub fn console_line(
    agent_label: &str,
    config: &WorkerConfig,
    supervisor: Option<(&str, &str)>,
) -> String {
    let base = format!(
        "   ⚙ worker {}: model={} thinking={}",
        agent_label, config.model, config.thinking_level
    );
    match config.source {
        ConfigSource::Supervisor => format!("{base} (inherited from supervisor)"),
        ConfigSource::Override => match supervisor {
            Some((model, level)) => {
                format!("{base} (override; supervisor was {model} / {level})")
            }
            None => format!("{base} (override; this goal has no supervisor config)"),
        },
        ConfigSource::Default => {
            format!("{base} (no supervisor config — agent default)")
        }
        ConfigSource::Session => format!("{base} (resumed session keeps its own config)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUP: Option<(&str, &str)> = Some(("future/sup-model", "medium"));

    #[test]
    fn explicit_flags_override_the_supervisor() {
        let r = resolve(Some("future/other"), Some("high"), SUP, false);
        assert_eq!(r.source, ConfigSource::Override);
        assert_eq!(r.set_model.as_deref(), Some("future/other"));
        assert_eq!(r.set_thinking.as_deref(), Some("high"));
        assert_eq!(r.source.as_str(), "override");
    }

    #[test]
    fn no_flags_inherit_the_supervisor_configuration() {
        let r = resolve(None, None, SUP, false);
        assert_eq!(r.source, ConfigSource::Supervisor);
        assert_eq!(r.set_model.as_deref(), Some("future/sup-model"));
        assert_eq!(r.set_thinking.as_deref(), Some("medium"));
        assert_eq!(r.source.as_str(), "supervisor");
    }

    #[test]
    fn no_flags_and_no_supervisor_fall_back_to_the_agent_default() {
        let r = resolve(None, None, None, false);
        assert_eq!(r.source, ConfigSource::Default);
        // Nothing is sent: guessing the agent's default here would be a lie.
        assert_eq!(r.set_model, None);
        assert_eq!(r.set_thinking, None);
        assert_eq!(r.source.as_str(), "default");
    }

    #[test]
    fn an_explicit_model_still_inherits_the_supervisor_thinking_level() {
        let r = resolve(Some("future/other"), None, SUP, false);
        assert_eq!(r.source, ConfigSource::Override);
        assert_eq!(r.set_model.as_deref(), Some("future/other"));
        assert_eq!(r.set_thinking.as_deref(), Some("medium"));
    }

    #[test]
    fn an_explicit_thinking_level_still_inherits_the_supervisor_model() {
        let r = resolve(None, Some("max"), SUP, false);
        assert_eq!(r.source, ConfigSource::Override);
        assert_eq!(r.set_model.as_deref(), Some("future/sup-model"));
        assert_eq!(r.set_thinking.as_deref(), Some("max"));
    }

    #[test]
    fn a_resumed_session_keeps_its_own_configuration() {
        let r = resolve(None, None, SUP, true);
        assert_eq!(r.source, ConfigSource::Session);
        assert_eq!(r.set_model, None);
        assert_eq!(r.set_thinking, None);
    }

    #[test]
    fn a_resumed_session_still_obeys_explicit_flags() {
        let r = resolve(Some("future/other"), None, SUP, true);
        assert_eq!(r.source, ConfigSource::Override);
        assert_eq!(r.set_model.as_deref(), Some("future/other"));
        // A resumed session has no inheritance source: leave its level alone.
        assert_eq!(r.set_thinking, None);
    }

    #[test]
    fn empty_flag_values_count_as_absent() {
        let r = resolve(Some("  "), Some(""), SUP, false);
        assert_eq!(r.source, ConfigSource::Supervisor);
        assert_eq!(r.set_model.as_deref(), Some("future/sup-model"));
    }

    fn cfg(source: ConfigSource) -> WorkerConfig {
        WorkerConfig {
            model: "future/m".to_string(),
            thinking_level: "medium".to_string(),
            source,
        }
    }

    #[test]
    fn the_console_line_names_an_override_and_the_supervisor_it_left() {
        let line = console_line("audio-1", &cfg(ConfigSource::Override), SUP);
        assert!(line.contains("override"), "{line}");
        assert!(line.contains("future/sup-model"), "{line}");
        assert!(line.contains("medium"), "{line}");
    }

    #[test]
    fn the_console_line_reports_an_override_without_a_supervisor() {
        let line = console_line("audio-1", &cfg(ConfigSource::Override), None);
        assert!(line.contains("override"), "{line}");
        assert!(line.contains("no supervisor config"), "{line}");
    }

    #[test]
    fn inherited_and_default_lines_do_not_claim_an_override() {
        for (source, expected) in [
            (ConfigSource::Supervisor, "inherited from supervisor"),
            (ConfigSource::Default, "agent default"),
            (ConfigSource::Session, "resumed session"),
        ] {
            let line = console_line("audio-1", &cfg(source), SUP);
            assert!(line.contains(expected), "{line}");
            assert!(!line.contains("override"), "{line}");
        }
    }

    #[test]
    fn the_header_fields_are_the_three_the_acceptance_requires() {
        for source in [
            ConfigSource::Supervisor,
            ConfigSource::Override,
            ConfigSource::Default,
            ConfigSource::Session,
        ] {
            let value = serde_json::to_value(cfg(source)).expect("serializable");
            assert_eq!(value["model"], "future/m");
            assert_eq!(value["thinking_level"], "medium");
            assert_eq!(value["model_source"], source.as_str());
        }
    }
}
