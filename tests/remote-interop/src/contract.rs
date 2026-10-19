//! Shared input/expectation validation. Unknown versions/kinds fail explicitly.
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Suite {
    pub flow_ids: FlowIds,
    pub version: u32,
    pub file_bytes: Vec<u8>,
    pub event: Event,
    pub cases: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowIds {
    pub first_pair: String,
    pub event: String,
    pub transfer: String,
    pub reconnect: String,
    pub shutdown: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub data: String,
    pub run_id: String,
    pub idx: i64,
    pub epoch: i64,
    pub event_id: String,
    pub timestamp: String,
    pub session_idx: i64,
    pub run_sequence: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub kind: String,
    pub steps: Vec<Step>,
    pub expected_executions: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub command: Value,
    #[serde(default, deserialize_with = "present")]
    pub data: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub error: Option<String>,
}

fn present<'de, D, T>(input: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(input).map(Some)
}

pub fn suite() -> anyhow::Result<Suite> {
    let suite: Suite = serde_json::from_str(include_str!(
        "../../../packages/remote-client/fixtures/client.json"
    ))?;
    anyhow::ensure!(suite.version == 1, "unsupported Client fixture version");
    anyhow::ensure!(!suite.file_bytes.is_empty(), "empty fixture file");
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    anyhow::ensure!(
        [
            suite.event.idx,
            suite.event.epoch,
            suite.event.session_idx,
            suite.event.run_sequence
        ]
        .iter()
        .all(|value| value.unsigned_abs() <= MAX_SAFE_INTEGER),
        "shared cursor exceeds JavaScript integer precision"
    );
    let mut ids = BTreeSet::new();
    for id in [
        &suite.flow_ids.first_pair,
        &suite.flow_ids.event,
        &suite.flow_ids.transfer,
        &suite.flow_ids.reconnect,
        &suite.flow_ids.shutdown,
    ] {
        anyhow::ensure!(
            id.starts_with("client.") && ids.insert(id.clone()),
            "invalid/duplicate Client flow ID"
        );
    }
    for case in &suite.cases {
        anyhow::ensure!(
            ids.insert(case.id.clone()) && case.id.starts_with("client."),
            "invalid/duplicate Client case ID"
        );
        anyhow::ensure!(case.kind == "request", "unsupported Client case kind");
        anyhow::ensure!(
            case.expected_executions <= MAX_SAFE_INTEGER,
            "shared count exceeds JavaScript integer precision"
        );
        anyhow::ensure!(!case.steps.is_empty(), "empty Client case");
        let request_id = case.steps[0].command["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing command ID"))?;
        for step in &case.steps {
            anyhow::ensure!(
                step.command["id"].as_str() == Some(request_id),
                "inconsistent request identity"
            );
            anyhow::ensure!(step.command["type"].is_string(), "missing command kind");
            anyhow::ensure!(
                step.data.is_some() != step.error.is_some(),
                "ambiguous step expectation"
            );
        }
    }
    Ok(suite)
}
