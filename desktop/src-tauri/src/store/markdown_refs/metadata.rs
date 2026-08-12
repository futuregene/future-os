//! Display metadata builders shared by [`super::sync`] (which caches it into
//! `reference_targets`) and [`super::search`] (which surfaces it in the
//! `@`-mention pick list). Each object kind has one builder so the `title` /
//! `subtitle` formulas and the `search_text` flattening stay in a single place
//! instead of being written twice with subtle drift.

use super::short_id;

/// The cached display metadata for one referenced object.
pub(super) struct ReferenceMetadata {
    pub title: String,
    pub subtitle: Option<String>,
    pub search_text: Option<String>,
}

pub(super) fn artifact_metadata(
    title: String,
    artifact_type: String,
    path: Option<String>,
    summary: Option<String>,
) -> ReferenceMetadata {
    let search_text = compact_search_text(
        &[&title, &artifact_type],
        &[path.as_ref(), summary.as_ref()],
    );
    ReferenceMetadata {
        subtitle: path.or(Some(artifact_type)),
        search_text: Some(search_text),
        title,
    }
}

pub(super) fn run_metadata(
    id: &str,
    status: String,
    model_id: Option<String>,
    error_message: Option<String>,
) -> ReferenceMetadata {
    let search_text =
        compact_search_text(&[id, &status], &[model_id.as_ref(), error_message.as_ref()]);
    ReferenceMetadata {
        title: format!("Run {}", short_id(id)),
        subtitle: model_id.or(Some(status)),
        search_text: Some(search_text),
    }
}

pub(super) fn approval_metadata(
    title: String,
    kind: String,
    status: String,
    summary: Option<String>,
    requested_action: Option<String>,
) -> ReferenceMetadata {
    let search_text = compact_search_text(
        &[&title, &kind, &status],
        &[summary.as_ref(), requested_action.as_ref()],
    );
    ReferenceMetadata {
        subtitle: Some(format!("{kind} · {status}")),
        search_text: Some(search_text),
        title,
    }
}

pub(super) fn review_metadata(
    title: String,
    status: String,
    summary: Option<String>,
    files_changed: i64,
    additions: i64,
    deletions: i64,
) -> ReferenceMetadata {
    let subtitle = format!("{status} · {files_changed} files · +{additions} -{deletions}");
    let search_text = compact_search_text(&[&title, &status, &subtitle], &[summary.as_ref()]);
    ReferenceMetadata {
        subtitle: Some(subtitle),
        search_text: Some(search_text),
        title,
    }
}

/// Join the non-empty required and present-optional fields with newlines into
/// the single blob the `@`-mention substring search runs against.
pub(super) fn compact_search_text(required: &[&str], optional: &[Option<&String>]) -> String {
    required
        .iter()
        .map(|value| (*value).to_string())
        .chain(
            optional
                .iter()
                .filter_map(|value| value.map(|text| text.to_string())),
        )
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_metadata_prefers_model_subtitle_and_flattens_search_text() {
        let meta = run_metadata(
            "run_1234567890",
            "failed".to_string(),
            Some("gpt-x".to_string()),
            Some("boom".to_string()),
        );
        assert_eq!(meta.title, "Run run_1234");
        assert_eq!(meta.subtitle.as_deref(), Some("gpt-x"));
        assert_eq!(
            meta.search_text.as_deref(),
            Some("run_1234567890\nfailed\ngpt-x\nboom")
        );
    }

    #[test]
    fn run_metadata_falls_back_to_status_subtitle() {
        let meta = run_metadata("run_abcdefgh", "completed".to_string(), None, None);
        assert_eq!(meta.title, "Run run_abcd");
        assert_eq!(meta.subtitle.as_deref(), Some("completed"));
        assert_eq!(
            meta.search_text.as_deref(),
            Some("run_abcdefgh\ncompleted")
        );
    }

    #[test]
    fn approval_metadata_combines_kind_and_status() {
        let meta = approval_metadata(
            "Deploy".to_string(),
            "shell".to_string(),
            "pending".to_string(),
            Some("Ship it".to_string()),
            Some("deploy --prod".to_string()),
        );
        assert_eq!(meta.title, "Deploy");
        assert_eq!(meta.subtitle.as_deref(), Some("shell · pending"));
        assert_eq!(
            meta.search_text.as_deref(),
            Some("Deploy\nshell\npending\nShip it\ndeploy --prod")
        );
    }

    #[test]
    fn review_metadata_summarizes_the_diff() {
        let meta = review_metadata(
            "Changes".to_string(),
            "ready".to_string(),
            None,
            3,
            10,
            2,
        );
        assert_eq!(meta.title, "Changes");
        assert_eq!(meta.subtitle.as_deref(), Some("ready · 3 files · +10 -2"));
        assert_eq!(
            meta.search_text.as_deref(),
            Some("Changes\nready\nready · 3 files · +10 -2")
        );
    }

    #[test]
    fn compact_search_text_drops_empty_and_absent_fields() {
        assert_eq!(compact_search_text(&["a", " ", "b"], &[None]), "a\nb");
        let present = "x".to_string();
        assert_eq!(
            compact_search_text(&[], &[Some(&present)]),
            "x"
        );
    }
}
