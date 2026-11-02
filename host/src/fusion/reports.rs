//! Accept only the final report; repair it in the original working session.
use anyhow::{ensure, Result};
use serde_json::Value;
use std::future::Future;

pub(super) trait ReportSession {
    fn generate(
        &mut self,
        message: String,
        report_only: bool,
    ) -> impl Future<Output = Result<String>> + Send;
}

/// The enclosing phase deadline/cancellation applies to the entire loop.
/// A repair never submits the original investigation task again.
pub(super) async fn collect(
    session: &mut impl ReportSession,
    mut message: String,
    phase: &str,
    capacity: usize,
    validate: impl Fn(&str) -> Result<()>,
) -> Result<String> {
    let mut report_only = false;
    loop {
        let text = session.generate(message, report_only).await?;
        let validation = (|| {
            let size = forwarded_bytes(phase, &text)?;
            ensure!(capacity == 0 || size <= capacity,
                "Final report occupies {size} UTF-8 bytes in downstream input; capacity is {capacity}");
            validate(&text)
        })();
        match validation {
            Ok(()) => return Ok(text),
            Err(error) => {
                report_only = true;
                message = format!(
                    "Revise ONLY your last final report using the existing work in this conversation. Do not repeat the task, investigate again, call tools or add new facts. Return a complete replacement report in the same required format, preserving the direct answer, essential evidence, material uncertainty and valid source references. No commentary about the revision. Validation feedback: {error}. Final report capacity: {capacity} UTF-8 bytes in its downstream representation; this is not a thinking-token budget."
                );
            }
        }
    }
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
        assert_eq!(answer, "基于已读文件的结论。");
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
        assert!(answer.contains("consensus"));
        assert_eq!(session.requests.len(), 2);
        assert!(session.requests[1].1);
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
