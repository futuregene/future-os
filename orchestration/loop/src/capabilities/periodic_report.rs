//! Periodic Report capability (LoopX: periodic-report — recurring heartbeat
//! or monitor work with a cadence profile).
//!
//! G-25 deepening: the P3 stub accepted only the literal words
//! hourly/daily/weekly; the deepened version parses cadence profiles
//! (`hourly` / `daily` / `weekly` / `every-Nh` / `every-Nd`), a scope
//! (`project:` / `team:` / `audience:`), and proposes:
//!
//! - a MONITOR todo with the cadence-derived due time when the profile is
//!   complete (recurring observation);
//! - a successor draft step for the report body;
//! - a clarify successor when the cadence is missing or ambiguous.

use super::{monitor_todo, successor_todo, Capability, TypedProposal};

pub struct PeriodicReportCapability;

#[derive(Debug, Clone, Default)]
pub struct ReportProfile {
    pub cadence: String,
    pub scope: String,
    pub audience: String,
    pub notes: String,
}

pub fn parse_report_profile(input: &str) -> ReportProfile {
    let mut p = ReportProfile::default();
    let mut free: Vec<String> = vec![];
    for line in input.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once(':') {
            match key.trim().to_lowercase().as_str() {
                "cadence" => p.cadence = value.trim().to_string(),
                "scope" => p.scope = value.trim().to_string(),
                "audience" => p.audience = value.trim().to_string(),
                "notes" => p.notes = value.trim().to_string(),
                _ => free.push(trimmed.to_string()),
            }
        } else if !trimmed.is_empty() {
            free.push(trimmed.to_string());
        }
    }
    if p.cadence.is_empty() {
        p.cadence = free.join(" ");
    }
    p
}

/// Parse a cadence token into (class, due_seconds). Supported: hourly (3600),
/// daily (86400), weekly (604800), every-Nh / every-Nd, and the literal
/// words. Returns None when ambiguous.
pub fn cadence_due_secs(cadence: &str) -> Option<(String, u64)> {
    let c = cadence.trim().to_lowercase();
    if c.is_empty() {
        return None;
    }
    if c.contains("hourly") || c.contains("hour") || c == "1h" || c.contains("每小时") {
        return Some(("hourly".to_string(), 3600));
    }
    if c.contains("daily") || c.contains("day") || c == "1d" || c.contains("每天") {
        return Some(("daily".to_string(), 86400));
    }
    if c.contains("weekly") || c.contains("week") || c.contains("每周") {
        return Some(("weekly".to_string(), 604800));
    }
    // every-Nh / every-Nd
    if let Some(rest) = c.strip_prefix("every-").or_else(|| c.strip_prefix("every")) {
        let rest = rest.trim();
        if let Some(n) = rest
            .strip_suffix('h')
            .and_then(|n| n.trim().parse::<u64>().ok())
        {
            if n > 0 {
                return Some((format!("every-{n}h"), n * 3600));
            }
        }
        if let Some(n) = rest
            .strip_suffix('d')
            .and_then(|n| n.trim().parse::<u64>().ok())
        {
            if n > 0 {
                return Some((format!("every-{n}d"), n * 86400));
            }
        }
    }
    None
}

impl Capability for PeriodicReportCapability {
    fn name(&self) -> &'static str {
        "periodic_report"
    }
    fn describe(&self) -> &'static str {
        "propose a recurring report monitor from a cadence/scope profile (hourly/daily/weekly/every-N)"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let profile = parse_report_profile(input);
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("no report profile provided")];
        }
        let Some((class, due_secs)) = cadence_due_secs(&profile.cadence) else {
            return vec![TypedProposal::successor(
                successor_todo(
                    "report",
                    "Clarify the report cadence (hourly/daily/weekly/every-Nh/every-Nd) and scope before scheduling.",
                ),
                "profile lacks a parseable cadence",
            )];
        };
        let scope = if profile.scope.is_empty() {
            "the configured scope".to_string()
        } else {
            format!("scope: {}", profile.scope)
        };
        let mut proposals = vec![TypedProposal::monitor(
            monitor_todo(
                "report",
                &format!(
                    "Draft the periodic report ({class}, due in {due_secs}s) for {scope}; include progress, blockers, and evidence refs.",
                ),
                due_secs,
            ),
            &format!("report monitor on a {class} cadence"),
        )];
        proposals.push(TypedProposal::successor(
            successor_todo(
                "report",
                "Collect the report inputs (progress, blockers, evidence refs) for the recurring report.",
            ),
            "report input collection",
        ));
        proposals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::ProposalKind;

    #[test]
    fn complete_profile_yields_monitor_and_collect() {
        let cap = PeriodicReportCapability;
        let proposals = cap.propose("cadence: weekly\nscope: loopx project\naudience: maintainers");
        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].kind, ProposalKind::Monitor);
        let monitor = proposals[0].todo.as_ref().unwrap();
        assert!(monitor.text.contains("weekly"));
        assert!(monitor.resume_when.is_some());
        assert_eq!(proposals[1].kind, ProposalKind::SuccessorTodo);
    }

    #[test]
    fn every_n_cadence_parses() {
        assert_eq!(
            cadence_due_secs("every-2h"),
            Some(("every-2h".to_string(), 7200))
        );
        assert_eq!(
            cadence_due_secs("every-3d"),
            Some(("every-3d".to_string(), 259200))
        );
        assert_eq!(
            cadence_due_secs("daily"),
            Some(("daily".to_string(), 86400))
        );
        assert_eq!(
            cadence_due_secs("hourly"),
            Some(("hourly".to_string(), 3600))
        );
        assert_eq!(cadence_due_secs(""), None);
        assert_eq!(cadence_due_secs("sometimes"), None);
    }

    #[test]
    fn missing_cadence_clarifies() {
        let cap = PeriodicReportCapability;
        let proposals = cap.propose("scope: project");
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        assert!(proposals[0].todo.as_ref().unwrap().text.contains("Clarify"));
    }

    #[test]
    fn free_text_cadence_detected() {
        let cap = PeriodicReportCapability;
        let proposals = cap.propose("send a daily report for the project");
        assert_eq!(proposals[0].kind, ProposalKind::Monitor);
        assert!(proposals[0].todo.as_ref().unwrap().text.contains("daily"));
    }
}
