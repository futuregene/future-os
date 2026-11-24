//! Accept only the final report; repair it in the original working session.
use anyhow::{ensure, Result};
use future_fusion::{failure, failure_kind, FailureKind};
use serde_json::Value;
use std::future::Future;

/// A neutral capacity bound: it must not broaden any stage into investigation.
pub(super) fn capacity_instruction(bytes: usize) -> String {
    format!("Final report capacity: {bytes} UTF-8 bytes including its actual forwarding representation. This is an upper bound, not a target length; include only material needed for this stage. It applies only to the final report, not to working generation or intermediate material. Follow this stage's scope and format. Oversized reports will be revised in this same session without repeating the task.")
}

pub(super) trait ReportSession {
    fn generate(
        &mut self,
        message: String,
        report_only: bool,
    ) -> impl Future<Output = Result<String>> + Send;
}

/// Only the selected public report is forwarded; failed validation is explicit.
#[derive(Debug)]
pub(super) struct Report {
    pub text: String,
    pub validation_errors: Vec<String>,
    pub generation: u32,
}

/// One repair in the original session, shared by format, citations and capacity.
/// Participant reports remain useful as attributed, unvalidated material. They
/// are packed against downstream context before being sent to the moderator.
pub(super) async fn collect(
    session: &mut impl ReportSession,
    message: String,
    phase: &str,
    capacity: usize,
    prepare: impl Fn(&str) -> Result<String>,
) -> Result<Report> {
    let check = |text: &str| -> Result<String> {
        let text = prepare(text)?;
        let size = forwarded_bytes(phase, &text)?;
        ensure!(
            capacity == 0 || size <= capacity,
            "Final report occupies {size} UTF-8 bytes in downstream input; capacity is {capacity}"
        );
        Ok(text)
    };
    // Agent owns output continuation inside the accepted child run. An
    // output-limit terminal here has already used its available allowance.
    let original = match session.generate(message, false).await {
        Ok(text) => text,
        Err(error)
            if phase == "synthesizing" && failure_kind(&error) == FailureKind::InvalidOutput =>
        {
            let text = session.generate(
                "The previous generation did not return a usable final report. Submit the complete report in the original format using your existing work and the original sources. Do not restart the discussion or add evidence.".into(),
                true,
            ).await?;
            return Ok(Report {
                text: check(&text).map_err(|error| {
                    failure(
                        FailureKind::InvalidOutput,
                        format!("Report validation failed after empty-response recovery: {error}"),
                    )
                })?,
                validation_errors: vec![],
                generation: 1,
            });
        }
        Err(error) => return Err(error),
    };
    let first_error = match check(&original) {
        Ok(text) => {
            return Ok(Report {
                text,
                validation_errors: vec![],
                generation: 0,
            })
        }
        Err(error) => error.to_string(),
    };
    let citation_repair = match phase {
        "issue_mapping" => "For a participant citation, copy a short exact quote from the named participant’s visible response. If the visible response is a JSON object, quote_path may select a relevant substantive string such as /answer or /support. For a plain-text response, supply quote.",
        "reviewing" => "In a mapping correction, use a short exact source_quote or select your own report with source_path /original_analysis. A nested field such as /original_analysis/answer is available when your report is JSON. For another participant, cite only their visible quote under moderator_map.",
        "synthesizing" => "For disposition citation errors, fix only the failing basis entry. Prefer a source_path to a relevant text field of the named source. For participant reports, /response selects the report itself. Otherwise copy an exact quote from that source.",
        _ => "",
    };
    let message = format!(
        "You have ONE report repair opportunity. Revise ONLY your last final report using the existing work in this conversation. Do not repeat the task, investigate again, call tools or add new facts. Fix ALL listed fields, not only the first error. For formatting errors, return one JSON object with escaped string values. {citation_repair} Validation error messages are diagnostics, not source evidence. Preserve unaffected dispositions. Preserve valid content and the current answer unless the reported source error changes its support; then correct the inference and resulting answer explicitly. Return a complete replacement report in the same required format. Participant quotations must copy the named participant’s visible wording; typography differences are allowed, paraphrases are not quotes. Evidence citations must identify a supplied source, its version and a matching original excerpt. If a verified_fact cannot cite a source, qualify it as an unverified_claim rather than inventing a reference. No commentary about the revision. Validation feedback: {first_error}. Final report capacity: {capacity} UTF-8 bytes in its downstream representation; this is not a thinking-token budget."
    );
    let repaired = match session.generate(message, true).await {
        Ok(text) => text,
        Err(error)
            if matches!(phase, "analyzing" | "reviewing")
                && matches!(
                    failure_kind(&error),
                    FailureKind::InvalidOutput
                        | FailureKind::OutputLimit
                        | FailureKind::PanelTimeout
                )
                && !original.trim().is_empty() =>
        {
            return Ok(Report {
                text: original,
                generation: 0,
                validation_errors: vec![future_fusion::bounded_text(
                    &format!("{first_error}\nReport repair did not complete: {error}"),
                    2048,
                )],
            });
        }
        Err(error) => return Err(error),
    };
    match check(&repaired) {
        Ok(text) => Ok(Report {
            text,
            validation_errors: vec![],
            generation: 1,
        }),
        Err(error) if matches!(phase, "analyzing" | "reviewing") => {
            // Prefer the repaired answer; an empty repair must not discard an
            // existing public answer. Never pretend it passed source checks.
            let (text, generation) = if repaired.trim().is_empty() {
                (original, 0)
            } else {
                (repaired, 1)
            };
            ensure!(
                !text.trim().is_empty(),
                "Empty Fusion report after one repair"
            );
            Ok(Report {
                text,
                validation_errors: vec![future_fusion::bounded_text(&error.to_string(), 2048)],
                generation,
            })
        }
        Err(error) => Err(failure(
            FailureKind::InvalidOutput,
            format!("Report validation failed after one repair: {error}"),
        )),
    }
}

/// UTF-8-safe forwarding only, retaining full originals in the child session.
pub(super) fn fit_text(text: &str, capacity: usize) -> Result<String> {
    if forwarded_bytes("analyzing", text)? <= capacity {
        return Ok(text.to_owned());
    }
    const NOTICE: &str = "\n[Report shortened to fit context; remaining material is retained in the discussion record.]";
    ensure!(
        forwarded_bytes("analyzing", NOTICE)? <= capacity,
        "No room for a usable report"
    );
    let mut low = 0;
    let mut high = text.len();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let mut end = mid;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = format!("{}{NOTICE}", &text[..end]);
        if forwarded_bytes("analyzing", &candidate)? <= capacity {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    while !text.is_char_boundary(low) {
        low -= 1;
    }
    Ok(format!("{}{NOTICE}", &text[..low]))
}

/// Count actual model-visible forwarding, not HTTP transport escaping. Reports
/// in structured source packets are strings; a map is inserted as a JSON object.
pub(super) fn forwarded_bytes(phase: &str, text: &str) -> Result<usize> {
    match phase {
        "analyzing" | "reviewing" | "synthesizing" => Ok(serde_json::to_vec(text)?.len()),
        "issue_mapping" => {
            Ok(serde_json::to_vec(&future_fusion::parse_json::<Value>(text)?)?.len())
        }
        _ => Ok(text.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct WorkingSession {
        requests: Vec<(String, bool)>,
        responses: VecDeque<String>,
    }
    impl ReportSession for WorkingSession {
        async fn generate(&mut self, message: String, report_only: bool) -> Result<String> {
            self.requests.push((message, report_only));
            Ok(self.responses.pop_front().expect("unexpected regeneration"))
        }
    }
    struct ExhaustedSession {
        requests: usize,
    }
    impl ReportSession for ExhaustedSession {
        async fn generate(&mut self, _message: String, _report_only: bool) -> Result<String> {
            self.requests += 1;
            Err(failure(FailureKind::OutputLimit, "exhausted"))
        }
    }
    #[tokio::test]
    async fn output_limit_ends_step_without_a_second_host_submission() {
        for phase in ["analyzing", "issue_mapping", "reviewing", "synthesizing"] {
            let mut session = ExhaustedSession { requests: 0 };
            let error = collect(&mut session, "task".into(), phase, 100, |text| {
                Ok(text.to_owned())
            })
            .await
            .unwrap_err();
            assert_eq!(failure_kind(&error), FailureKind::OutputLimit);
            assert_eq!(session.requests, 1);
        }
    }
    #[tokio::test]
    async fn missing_synthesis_gets_one_report_only_attempt_with_validation() {
        struct MissingSynthesis(Vec<bool>);
        impl ReportSession for MissingSynthesis {
            async fn generate(&mut self, _: String, report_only: bool) -> Result<String> {
                self.0.push(report_only);
                if report_only {
                    Ok("Recovered report".into())
                } else {
                    Err(failure(
                        FailureKind::InvalidOutput,
                        "Missing Fusion model response",
                    ))
                }
            }
        }
        let mut session = MissingSynthesis(vec![]);
        let error = collect(&mut session, "Task".into(), "synthesizing", 100, |_| {
            anyhow::bail!("Invalid citation")
        })
        .await
        .unwrap_err();
        assert_eq!(session.0, [false, true]);
        assert!(error.to_string().contains("Invalid citation"));
    }

    #[tokio::test]
    async fn oversized_report_is_revised_without_repeating_investigation() {
        let mut session = WorkingSession {
            requests: vec![],
            responses: ["调查结果。".repeat(100), "基于已读文件的结论。".into()].into(),
        };
        let answer = collect(
            &mut session,
            "Original investigation".into(),
            "analyzing",
            100,
            |text| {
                ensure!(text.contains("结论"), "Missing answer");
                Ok(text.to_owned())
            },
        )
        .await
        .unwrap();
        assert_eq!(answer.text, "基于已读文件的结论。");
        assert_eq!(session.requests.len(), 2);
        assert!(!session.requests[0].1);
        assert!(session.requests[1].1);
        assert!(!session.requests[1].0.contains("Original investigation"));
        assert!(session.requests[1].0.contains("Do not repeat"));
    }
    #[tokio::test]
    async fn a_structural_repair_also_reuses_the_same_session() {
        let mut session = WorkingSession {
            requests: vec![],
            responses: [
                "invalid".into(),
                r#"{"consensus":[],"issues":[],"excluded":[]}"#.into(),
            ]
            .into(),
        };
        let answer = collect(
            &mut session,
            "Organize existing sources".into(),
            "issue_mapping",
            100,
            |text| {
                let _: Value = serde_json::from_str(text)?;
                Ok(text.to_owned())
            },
        )
        .await
        .unwrap();
        assert!(answer.text.contains("consensus"));
        assert_eq!(session.requests.len(), 2);
        assert!(session.requests[1].1);
        assert!(session.requests[1].0.contains("quote_path"));
        assert!(session.requests[1].0.contains("short exact quote"));
        assert!(!session.requests[1].0.contains("/response"));
    }
    #[tokio::test]
    async fn stops_after_one_repair_and_retains_unvalidated_public_report() {
        let mut session = WorkingSession {
            requests: vec![],
            responses: ["Initial answer".into(), "Useful revised answer".into()].into(),
        };
        let report = collect(&mut session, "Review".into(), "reviewing", 1000, |_| {
            anyhow::bail!("responses[0].quote does not match P1")
        })
        .await
        .unwrap();
        assert_eq!(session.requests.len(), 2);
        assert_eq!(report.text, "Useful revised answer");
        assert_eq!(report.generation, 1);
        assert!(!report.validation_errors.is_empty());
        assert!(session.requests[1].0.contains("ONE report repair"));
        assert!(session.requests[1]
            .0
            .contains("source_path /original_analysis"));
        assert!(!session.requests[1].0.contains("/response"));
    }
    #[tokio::test]
    async fn empty_repair_keeps_original_and_invalid_map_never_becomes_a_map() {
        let mut session = WorkingSession {
            requests: vec![],
            responses: ["Existing answer".into(), "".into()].into(),
        };
        let report = collect(&mut session, "Review".into(), "reviewing", 1000, |_| {
            anyhow::bail!("Invalid quote")
        })
        .await
        .unwrap();
        assert_eq!(report.text, "Existing answer");
        assert_eq!(report.generation, 0);
        let mut session = WorkingSession {
            requests: vec![],
            responses: ["invalid".into(), "still invalid".into()].into(),
        };
        let error = collect(
            &mut session,
            "Organize".into(),
            "issue_mapping",
            1000,
            |_| anyhow::bail!("Invalid map"),
        )
        .await
        .unwrap_err();
        assert_eq!(failure_kind(&error), FailureKind::InvalidOutput);
        assert_eq!(session.requests.len(), 2);
    }
    #[test]
    fn forwarding_fits_actual_escaped_bytes_and_preserves_utf8() {
        let original = "中文\n\"quoted\"".repeat(1000);
        let retained = fit_text(&original, 1024).unwrap();
        assert!(forwarded_bytes("reviewing", &retained).unwrap() <= 1024);
        assert!(retained.contains("shortened to fit context"));
        assert_eq!(fit_text("完整报告", 100).unwrap(), "完整报告");
    }

    #[test]
    fn actual_json_cost_does_not_apply_a_blanket_sixfold_factor() {
        let text = "中文\n\"quoted\"";
        let bytes = forwarded_bytes("analyzing", text).unwrap();
        assert_eq!(bytes, serde_json::to_string(text).unwrap().len());
        assert!(bytes < text.len() * 2);
        let map = " { \"consensus\": [] } ";
        assert_eq!(
            forwarded_bytes("issue_mapping", map).unwrap(),
            r#"{"consensus":[]}"#.len()
        );
        assert_eq!(
            forwarded_bytes("synthesizing", text).unwrap(),
            serde_json::to_string(text).unwrap().len()
        );
    }
}
