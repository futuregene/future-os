use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

use super::{reply, reply_unit};

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "upload_init" => {
            match crate::remote_host::files::init_upload(
                &cmd.name,
                &cmd.transfer_name,
                &cmd.mime_type,
                &cmd.kind,
                cmd.original_size,
                cmd.transfer_size,
            ) {
                Ok(data) => {
                    reply(
                        sink,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "upload_complete" => match crate::remote_host::files::complete_upload(&cmd.transfer_id) {
            Ok(data) => {
                reply(
                    sink,
                    true,
                    serde_json::to_value(data).unwrap_or(Value::Null),
                    None,
                )
                .await
            }
            Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
        },
        "upload_cancel" => {
            reply_unit(
                sink,
                crate::remote_host::files::cancel_upload(&cmd.transfer_id),
            )
            .await
        }
        "list_session_files" => {
            let session_id = cmd.session_id.clone();
            let file_path = cmd.file_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::remote_host::session_files::list(&session_id, &file_path)
            })
            .await;
            match result {
                Ok(Ok(data)) => reply(sink, true, json!(data), None).await,
                Ok(Err(error)) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "download_prepare" => {
            match crate::remote_host::files::prepare_download_variant(
                &cmd.session_id,
                &cmd.file_path,
                &cmd.mode,
                Some(&cmd.name),
            )
            .await
            {
                Ok(data) => {
                    reply(
                        sink,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "download_cancel" => {
            crate::remote_host::files::cancel_download(&cmd.transfer_id);
            reply(sink, true, json!({}), None).await;
        }
        _ => unreachable!("transfer handler received {}", cmd.cmd_type),
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    /// The transfer family is a closed set of six commands; a name from another
    /// family must not be answered with a plausible-looking reply.
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
