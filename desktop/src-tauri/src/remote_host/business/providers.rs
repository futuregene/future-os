//! Provider and model configuration for a paired phone.
//!
//! Every write goes through the same `agent_providers` command paths the
//! desktop Settings dialog uses, so a phone is subject to exactly the same
//! validation, catalog-collision checks and agent RPC writes — including the
//! rules that keep the account provider (`future`) uneditable and every
//! built-in provider undeletable. Nothing here reads an API key back out: the
//! view carries only `hasApiKey`, so a key can be set or cleared from the phone
//! but never displayed.
//!
//! Payloads travel in the shared `IncomingCmd` fields: `provider_id` selects a
//! custom provider to delete, and `provider` carries one whole write — either a
//! built-in update (`agent_providers::UpdateBuiltinProviderInput`, atomic key +
//! Base URL) or a custom-provider upsert
//! (`agent_providers::UpsertCustomProviderInput`), both in the same camelCase
//! shape the desktop forms send.

use crate::agent_providers::{
    delete_custom_provider, list_agent_providers, update_builtin_provider, upsert_custom_provider,
    ProvidersView,
};
use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::Value;

use super::reply;

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "list_providers" => reply_view(sink, list_agent_providers().await).await,
        "update_builtin_provider" => match parse(cmd.provider.clone()) {
            Ok(input) => reply_view(sink, update_builtin_provider(input).await).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error)).await,
        },
        "upsert_custom_provider" => match parse(cmd.provider.clone()) {
            Ok(input) => reply_view(sink, upsert_custom_provider(input).await).await,
            Err(error) => reply(sink, false, Value::Null, Some(&error)).await,
        },
        "delete_custom_provider" => {
            reply_view(sink, delete_custom_provider(cmd.provider_id.clone()).await).await;
        }
        _ => unreachable!("provider handler received {}", cmd.cmd_type),
    }
}

/// Reject a malformed payload before it reaches the agent. Field-level rules
/// (lengths, id charset, model caps) stay in `agent_providers::validate`, which
/// is authoritative for both clients.
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "The provider payload could not be read.".to_string())
}

async fn reply_view(sink: &dyn ReplySink, result: Result<ProvidersView, crate::AppError>) {
    match result {
        Ok(view) => match serde_json::to_value(view) {
            Ok(value) => reply(sink, true, value, None).await,
            Err(_) => {
                reply(
                    sink,
                    false,
                    Value::Null,
                    Some("Could not serialize the provider configuration."),
                )
                .await
            }
        },
        Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    /// Only the four provider commands the dispatcher routes here may reach this
    /// handler. Anything else is a routing bug and must panic rather than reply.
    #[tokio::test]
    #[should_panic(expected = "handler received")]
    async fn a_command_from_another_family_is_not_answered() {
        let sink = crate::remote::test_support::RecordingSink::default();
        let cmd = IncomingCmd {
            cmd_type: "get_settings".into(),
            ..Default::default()
        };
        execute(&cmd, &sink).await;
    }
}
