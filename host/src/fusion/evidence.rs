//! Source receipts describe what was supplied, never what a claim proves.
use anyhow::{ensure, Context, Result};
use future_fusion::{ConsultationInput, Snapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) const POLICY: &str = "Source manifest is backend-generated availability metadata, not an additional user requirement. request identifies the preserved request (or the consultation question when unavailable); evidence:N identifies a zero-based supplied evidence item. The supplied_evidence status describes only the optional evidence array; data may already be present in request. The history_summary, prior assistant statements and participant reports are attributed context, not source-verified observations. File metadata means captured, not read; use the authorized snapshot tools before citing file contents. not_supplied and excluded content do not mean an empty dataset or a query with no matches. Do not apply a business empty-result default to unavailable input. If the task depends on missing material, identify that gap and preserve useful conditional results. Tasks that do not need external evidence may proceed normally. Source validation checks location, version and quotation only: it does not establish truth, independence, exhaustive coverage or entailment. A verified_fact remains a participant's source-checked claim; evaluate its support and counterexamples before adopting it.";

/// Project the durable input into the roles visible to the panel. In a closed
/// task the preserved request already supplies the entire discussion question.
pub(super) fn prompt_input(input: &ConsultationInput) -> Value {
    let mut discussion_task = json!({
        "scope": if input.answer_only { "full_user_request" } else { "focused_task" },
        "goal": input.goal,
        "constraints": input.constraints,
    });
    if !input.answer_only {
        discussion_task["question"] = json!(input.question);
    }
    json!({
        "parent_request": input.parent_request,
        "discussion_task": discussion_task,
        "history_summary": input.background,
        "evidence": input.evidence,
        "read_scope": input.read_scope,
    })
}

pub(super) fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub(super) struct Sources<'a> {
    input: &'a ConsultationInput,
    snapshot: Option<&'a Snapshot>,
}

impl<'a> Sources<'a> {
    pub fn new(input: &'a ConsultationInput, snapshot: Option<&'a Snapshot>) -> Self {
        Self { input, snapshot }
    }

    fn inline(&self, id: &str) -> Option<&str> {
        if id == "request" {
            return Some(
                self.input
                    .parent_request
                    .as_deref()
                    .unwrap_or(&self.input.question),
            );
        }
        let index = id.strip_prefix("evidence:")?.parse::<usize>().ok()?;
        // Do not accept aliases such as evidence:00 for the published ID.
        (id == format!("evidence:{index}"))
            .then(|| self.input.evidence.get(index).map(String::as_str))?
    }

    pub fn manifest(&self) -> Value {
        let mut inline = vec![];
        for id in std::iter::once("request".to_owned())
            .chain((0..self.input.evidence.len()).map(|n| format!("evidence:{n}")))
        {
            let text = self.inline(&id).expect("published inline source");
            inline.push(
                json!({"source_id":id,"sha256":digest(text),"bytes":text.len(),
                "availability":if text.trim().is_empty() {"not_supplied"} else {"provided"}}),
            );
        }
        let snapshot = match self.snapshot {
            Some(snapshot) => json!({
                "availability":if !snapshot.excluded.is_empty() {"partial"}
                    else if snapshot.files.is_empty() {"captured_no_text_files"} else {"captured"},
                "text_files":snapshot.files.len(), "excluded_files":snapshot.excluded.len(),
                "content_delivery":"authorized_tools_only; capture does not mean read",
                "reference_format":"snapshot:<path returned by read>; use its sha256 and one-based line range"
            }),
            None => json!({"availability":"not_loaded","content_delivery":"unavailable"}),
        };
        let mut manifest = json!({"version":1,"inline_sources":inline,
            "supplied_evidence":if self.input.evidence.iter().any(|s| !s.trim().is_empty()) {"provided"} else {"not_supplied"},
            "file_scope":if self.input.read_scope.is_empty() {"not_requested"} else {"requested"},
            "snapshot":snapshot});
        if let Some(snapshot) = self.snapshot {
            let captured = snapshot.user_images.len();
            let unavailable = snapshot.unavailable_user_images;
            if captured > 0 || unavailable > 0 {
                manifest["user_images"] = json!({
                    "captured": captured,
                    "unavailable": unavailable,
                    "content_delivery": "native attachments for roles that support images; a count alone does not expose image content",
                    "use": "Use visible details as visual observations and separate them from inference. Do not claim to have seen missing images or treat another role's description as independently verified. If an unseen detail changes the answer, identify the gap. Images are outside the text-quotation citation protocol."
                });
            }
        }
        manifest
    }

    pub fn inline_version(&self, id: &str) -> Option<String> {
        self.inline(id)
            .filter(|text| !text.trim().is_empty())
            .map(digest)
    }

    pub fn validate(&self, reference: &Reference) -> Result<()> {
        let citation = match reference {
            Reference::Citation(citation) => citation,
            Reference::Legacy(label) => anyhow::bail!(
                "verified_fact requires source_id, sha256 and quote, not a free-form reference {:?}; use unverified_claim if no source can be cited",
                future_fusion::bounded_text(label, 160)
            ),
        };
        ensure!(
            !citation.quote.trim().is_empty(),
            "Source quotation is empty"
        );
        if let Some(text) = self.inline(&citation.source_id) {
            ensure!(
                citation.start_line.is_none() && citation.end_line.is_none(),
                "Inline references do not accept file line ranges"
            );
            ensure!(!text.trim().is_empty(), "Source was not supplied");
            ensure!(
                citation.sha256 == digest(text),
                "Source version does not match {}",
                citation.source_id
            );
            ensure!(
                super::quotes::contains(text, &citation.quote),
                "Quotation is absent from source {}",
                citation.source_id
            );
            return Ok(());
        }
        let path = citation.source_id.strip_prefix("snapshot:").context(
            "Unknown source_id; cite request, evidence:N, or an authorized snapshot:<path>",
        )?;
        let snapshot = self
            .snapshot
            .context("No file snapshot is available for this citation")?;
        let file = snapshot
            .files
            .iter()
            .find(|file| file.path == path)
            .context("Source is not an included file in this consultation's snapshot")?;
        ensure!(
            citation.sha256 == file.sha256,
            "Snapshot source version mismatch"
        );
        let start = citation
            .start_line
            .context("Snapshot citation requires start_line")?;
        let end = citation
            .end_line
            .context("Snapshot citation requires end_line")?;
        ensure!(
            start > 0 && end >= start && end - start < 400,
            "Snapshot citation requires a one-based range of at most 400 lines"
        );
        // Reuse the authorized query path, including bounds and content-hash
        // checks. Never read an arbitrary path supplied in a model report.
        let page: Value = serde_json::from_str(&snapshot.query(
            "read",
            json!({"path":path,"offset":start,"limit":end-start+1}),
        )?)?;
        let lines = page["lines"].as_array().context("Missing snapshot lines")?;
        ensure!(
            lines.len() == end - start + 1,
            "Citation range extends beyond the source"
        );
        let text = lines
            .iter()
            .filter_map(|line| line["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        ensure!(
            super::quotes::contains(&text, &citation.quote),
            "Quotation is absent from the cited snapshot range"
        );
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Reference {
    Citation(Citation),
    // Old reports and unverified claims remain parseable. A legacy label is
    // never silently promoted to a source-checked fact in a new review.
    Legacy(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Citation {
    source_id: String,
    sha256: String,
    quote: String,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn input() -> ConsultationInput {
        serde_json::from_value(
            json!({"question":"Summarize the supplied records", "goal":"Summarize",
            "background":"An earlier assistant says all rows are empty.",
            "parent_request":"Use the records, if supplied.","evidence":["Observed count: 7."]}),
        )
        .unwrap()
    }
    fn citation(source: &str, text: &str, quote: &str) -> Reference {
        serde_json::from_value(json!({"source_id":source,"sha256":digest(text),"quote":quote}))
            .unwrap()
    }
    #[test]
    fn panel_prompt_keeps_the_focused_task_separate_from_the_user_request() {
        let mut input = input();
        let focused = prompt_input(&input);
        assert_eq!(focused["parent_request"], "Use the records, if supplied.");
        assert_eq!(focused["discussion_task"]["scope"], "focused_task");
        assert_eq!(
            focused["discussion_task"]["question"],
            "Summarize the supplied records"
        );
        assert_eq!(focused["history_summary"], input.background);

        input.answer_only = true;
        input.question = input.parent_request.clone().unwrap();
        let closed = prompt_input(&input);
        assert_eq!(closed["parent_request"], focused["parent_request"]);
        assert_eq!(closed["discussion_task"]["scope"], "full_user_request");
        assert!(closed["discussion_task"].get("question").is_none());
    }

    #[test]
    fn inline_references_bind_identity_version_and_original_text() {
        let input = input();
        let sources = Sources::new(&input, None);
        sources
            .validate(&citation("evidence:0", &input.evidence[0], "count: 7."))
            .unwrap();
        for reference in [
            citation("nonexistent", &input.evidence[0], "count: 7."),
            citation("evidence:00", &input.evidence[0], "count: 7."),
            citation("evidence:0", "another version", "count: 7."),
            citation("evidence:0", &input.evidence[0], "count: 0."),
            citation("background", &input.background, "all rows are empty"),
            Reference::Legacy("a fabricated source label".into()),
        ] {
            assert!(sources.validate(&reference).is_err());
        }
    }
    #[test]
    fn availability_does_not_manufacture_an_empty_business_result() {
        let mut input = input();
        input.evidence = vec![" ".into()];
        let manifest = Sources::new(&input, None).manifest();
        assert_eq!(manifest["supplied_evidence"], "not_supplied");
        assert_eq!(manifest["inline_sources"][0]["source_id"], "request");
        assert_eq!(manifest["inline_sources"][0]["availability"], "provided");
        assert_eq!(manifest["file_scope"], "not_requested");
        assert_eq!(manifest["snapshot"]["availability"], "not_loaded");
        assert!(manifest.get("user_images").is_none());
        // Missing external evidence is not a blanket rejection of a valid task.
        input.validate().unwrap();
    }
    #[test]
    fn captured_empty_scope_is_not_a_business_query_result() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let snapshot = Snapshot::capture(
            &workspace,
            &root.path().join("snapshot"),
            &[".".into()],
            &AtomicBool::new(false),
            |_| true,
        )
        .unwrap();
        let mut input = input();
        input.evidence.clear();
        input.read_scope = vec![".".into()];
        let manifest = Sources::new(&input, Some(&snapshot)).manifest();
        assert_eq!(manifest["supplied_evidence"], "not_supplied");
        assert_eq!(manifest["file_scope"], "requested");
        assert_eq!(
            manifest["snapshot"]["availability"],
            "captured_no_text_files"
        );
        assert!(manifest.get("query_result").is_none());
    }

    #[test]
    fn image_receipts_publish_availability_without_payload_or_path() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let image = "data:image/png;base64,AQID";
        let user_images =
            future_fusion::UserImages::from_candidates([Some(image), None], |value| {
                value.map(str::to_owned)
            });
        let snapshot = Snapshot::capture_with_images(
            &workspace,
            &root.path().join("snapshot"),
            &[],
            &user_images,
            &AtomicBool::new(false),
            |_| true,
        )
        .unwrap();
        let input = input();
        let manifest = Sources::new(&input, Some(&snapshot)).manifest();
        assert_eq!(manifest["user_images"]["captured"], 1);
        assert_eq!(manifest["user_images"]["unavailable"], 1);
        assert!(!manifest.to_string().contains(image));
        assert!(!manifest.to_string().contains(&snapshot.root));
    }

    #[test]
    fn file_citations_are_limited_to_unchanged_authorized_ranges() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("data.txt"), "header\ncount: 7\nend\n").unwrap();
        std::fs::write(workspace.join("binary.bin"), [0, 1]).unwrap();
        let snapshot = Snapshot::capture(
            &workspace,
            &root.path().join("snapshot"),
            &[".".into()],
            &AtomicBool::new(false),
            |_| true,
        )
        .unwrap();
        let input = input();
        let sources = Sources::new(&input, Some(&snapshot));
        assert_eq!(sources.manifest()["snapshot"]["availability"], "partial");
        assert_eq!(sources.manifest()["snapshot"]["excluded_files"], 1);
        let reference = |path: &str, start, end| {
            serde_json::from_value::<Reference>(json!({
            "source_id":format!("snapshot:{path}"),"sha256":snapshot.files[0].sha256,
            "quote":"count: 7","start_line":start,"end_line":end}))
            .unwrap()
        };
        sources.validate(&reference("data.txt", 2, 2)).unwrap();
        for item in [
            reference("data.txt", 1, 1),
            reference("data.txt", 2, 8),
            reference("../workspace/data.txt", 2, 2),
            reference("binary.bin", 1, 1),
        ] {
            assert!(sources.validate(&item).is_err());
        }
        std::fs::write(
            root.path().join("snapshot/files/data.txt"),
            "header\ncount: 0\nend\n",
        )
        .unwrap();
        assert!(sources.validate(&reference("data.txt", 2, 2)).is_err());
    }
}
