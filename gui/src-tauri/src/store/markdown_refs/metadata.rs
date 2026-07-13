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

pub(super) fn tool_metadata(
    name: String,
    kind: String,
    status: String,
    input: Option<String>,
) -> ReferenceMetadata {
    let search_text = compact_search_text(&[&name, &kind, &status], &[input.as_ref()]);
    ReferenceMetadata {
        subtitle: Some(format!("{kind} · {status}")),
        search_text: Some(search_text),
        title: name,
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
