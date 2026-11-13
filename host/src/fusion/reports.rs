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
    validate: impl Fn(&str) -> Result<()>,
) -> Result<Report> {
    let check = |text: &str| -> Result<()> {
        let mut errors = vec![];
        match forwarded_bytes(phase, text) {
            Ok(size) if capacity > 0 && size > capacity => errors.push(format!(
                "Final report occupies {size} UTF-8 bytes in downstream input; capacity is {capacity}")),
            Err(error) => errors.push(error.to_string()),
            _ => {},
        }
        if let Err(error) = validate(text) {
            errors.push(error.to_string());
        }
        ensure!(errors.is_empty(), "{}", errors.join("\n"));
        Ok(())
    };
    let original = session.generate(message, false).await?;
    let first_error = match check(&original) {
        Ok(()) => {
            return Ok(Report {
                text: original,
                validation_errors: vec![],
                generation: 0,
            })
        }
        Err(error) => error.to_string(),
    };
    let message = format!(
        "You have ONE report repair opportunity. Revise ONLY your last final report using the existing work in this conversation. Do not repeat the task, investigate again, call tools or add new facts. Fix the reported fields; preserve the current answer and other valid content. Return a complete replacement report in the same required format. Quotes must copy visible source wording and belong to the named participant; typography differences are allowed, paraphrases are not quotes. No commentary about the revision. Validation feedback: {first_error}. Final report capacity: {capacity} UTF-8 bytes in its downstream representation; this is not a thinking-token budget."
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
        Ok(()) => Ok(Report {
            text: repaired,
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
                Ok(())
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
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(answer.text.contains("consensus"));
        assert_eq!(session.requests.len(), 2);
        assert!(session.requests[1].1);
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
