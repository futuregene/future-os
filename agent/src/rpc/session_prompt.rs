use anyhow::Result;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[cfg(test)]
use super::prompt_helpers::build_user_message;
use super::prompt_helpers::{
    approve_tool_path_if_present, build_user_message_with_model_context, prepare_session_tool_call,
    run_event_to_sse,
};
use super::ServerSession;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScheduledPromptPayload {
    message: String,
    #[serde(default)]
    model_context: String,
    images: Vec<crate::types::ImageContent>,
    attachments: Vec<QueuedAttachmentSnapshot>,
    settings: ScheduledSettingsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueuedAttachmentSnapshot {
    attachment: crate::types::Attachment,
    bytes_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScheduledSettingsSnapshot {
    settings_schema_version: u32,
    model: String,
    thinking_level: String,
    cwd: String,
    auto_compaction: bool,
    auto_retry: bool,
    permission_level: String,
    sandbox_tier: Option<String>,
}

pub(super) struct AcceptedRunSnapshot {
    run_loop: crate::agent::Loop,
    settings: ScheduledSettingsSnapshot,
}

/// Textual parts of one user turn. `message` is the exact user-authored text;
/// `model_context` is a non-display sidecar that remains at user-role trust.
#[derive(Clone, Copy)]
pub(super) struct PromptText<'a> {
    message: &'a str,
    model_context: &'a str,
}

impl<'a> PromptText<'a> {
    pub(super) fn new(message: &'a str, model_context: &'a str) -> Self {
        Self {
            message,
            model_context,
        }
    }

    fn user_only(message: &'a str) -> Self {
        Self::new(message, "")
    }
}

/// Test-only hook fired by `start_next_scheduled` right after it peeks the
/// front queued run, keyed on that run id, so a test can reorder/drain the
/// queue before `prompt_internal` calls `start_next` (FIFO-mismatch / empty
/// dequeue defensive arms).
#[cfg(test)]
type SessionHook = Option<(String, Box<dyn Fn(&mut ServerSession) + Send>)>;

#[cfg(test)]
static SCHEDULED_DEQUEUE_HOOK: parking_lot::Mutex<SessionHook> = parking_lot::Mutex::new(None);

/// Test-only hook fired by `prompt_internal` right before `runtime.spawn`,
/// keyed on the accepted run id, so a test can occupy the task slot and
/// reach the spawn double-occupancy error arm.
#[cfg(test)]
static RUN_SPAWN_HOOK: parking_lot::Mutex<SessionHook> = parking_lot::Mutex::new(None);

/// Test-only flag: when set to a run id, `prompt_internal` fails right after
/// `start_next` made that run active (without finishing it), so the enqueue
/// error arm's `finish_active` branch is reachable.
#[cfg(test)]
static POST_START_FAIL_RUN: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

#[cfg(test)]
fn take_post_start_failure(run_id: &str) -> Option<String> {
    let mut slot = POST_START_FAIL_RUN.lock();
    if matches!(slot.as_deref(), Some(run) if run == run_id) {
        slot.take()
    } else {
        None
    }
}

impl ServerSession {
    #[cfg(test)]
    pub(super) fn scheduled_setting_summary(
        &self,
        run_id: &str,
    ) -> Option<(String, String, bool, String)> {
        self.scheduled_snapshots.get(run_id).map(|snapshot| {
            (
                snapshot.settings.model.clone(),
                snapshot.settings.thinking_level.clone(),
                snapshot.settings.auto_retry,
                snapshot.settings.permission_level.clone(),
            )
        })
    }

    #[cfg(test)]
    pub(super) fn scheduled_attachment_bytes(&self, run_id: &str) -> Option<Vec<Vec<u8>>> {
        let request = self
            .scheduler
            .queued()
            .into_iter()
            .find(|run| run.run_id == run_id)?;
        let payload: ScheduledPromptPayload = serde_json::from_value(request.payload).ok()?;
        payload
            .attachments
            .iter()
            .map(|snapshot| {
                base64::engine::general_purpose::STANDARD
                    .decode(&snapshot.bytes_base64)
                    .ok()
            })
            .collect()
    }

    pub fn enqueue_prompt(
        &mut self,
        msg: &str,
        images: &[crate::types::ImageContent],
        attachments: &[crate::types::Attachment],
        requested_run_id: Option<&str>,
        client_request_id: &str,
        busy_policy: crate::runtime::BusyPolicy,
    ) -> Result<crate::runtime::RunAck> {
        self.enqueue_prompt_with_model_context(
            PromptText::user_only(msg),
            images,
            attachments,
            requested_run_id,
            client_request_id,
            busy_policy,
        )
    }

    pub(super) fn enqueue_prompt_with_model_context(
        &mut self,
        prompt: PromptText<'_>,
        images: &[crate::types::ImageContent],
        attachments: &[crate::types::Attachment],
        requested_run_id: Option<&str>,
        client_request_id: &str,
        busy_policy: crate::runtime::BusyPolicy,
    ) -> Result<crate::runtime::RunAck> {
        // auth.json is authoritative. Refresh immediately before freezing the
        // run snapshot so a UI/catalog view and the actual request can never
        // disagree about which key is in use. Failure rejects admission rather
        // than silently running with stale credentials.
        self.reload_credentials()?;
        if self.deleting {
            return Err(crate::runtime::RunQueueError::Deleting.into());
        }
        if !self.scheduler.knows_request(client_request_id) {
            if let Some(run_id) = requested_run_id.filter(|run_id| !run_id.is_empty()) {
                let journal_exists = self
                    .session_manager
                    .run_data_path(&self.session_id)
                    .join(format!("{run_id}.jsonl"))
                    .exists();
                let transcript_exists = self
                    .session_manager
                    .load(&self.session_id)
                    .ok()
                    .is_some_and(|session| {
                        session.entries.iter().any(|entry| {
                            entry
                                .content
                                .as_ref()
                                .and_then(|content| content.get("run_id"))
                                .and_then(serde_json::Value::as_str)
                                == Some(run_id)
                        })
                    });
                if journal_exists || transcript_exists {
                    return Err(
                        crate::runtime::RunQueueError::DuplicateRunId(run_id.to_string()).into(),
                    );
                }
            }
        }
        if let Some(error) = self
            .broadcaster
            .persistence_error()
            .or_else(|| self.persistence.last_error())
        {
            return Err(crate::runtime::RunQueueError::PersistenceUnavailable(error).into());
        }
        if busy_policy == crate::runtime::BusyPolicy::RejectIfBusy
            && self.runtime.snapshot().is_some()
            && self.scheduler.active().is_none()
        {
            return Err(crate::runtime::RunQueueError::Busy.into());
        }
        let attachment_snapshots = attachments
            .iter()
            .map(|attachment| {
                let bytes = std::fs::read(&attachment.path).map_err(|error| {
                    crate::runtime::RunQueueError::AttachmentUnavailable {
                        path: attachment.path.clone(),
                        reason: error.to_string(),
                    }
                })?;
                Ok(QueuedAttachmentSnapshot {
                    attachment: attachment.clone(),
                    bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                })
            })
            .collect::<std::result::Result<Vec<_>, crate::runtime::RunQueueError>>()?;
        let settings = ScheduledSettingsSnapshot {
            settings_schema_version: 1,
            model: self.model.clone(),
            thinking_level: self.thinking_level.clone(),
            cwd: self.cwd.clone(),
            auto_compaction: self.auto_compaction,
            auto_retry: self.auto_retry,
            permission_level: self.permission_level.clone(),
            sandbox_tier: self
                .sandbox_policy
                .as_ref()
                .map(|policy| policy.tier.as_str().to_string()),
        };
        // Freeze the provider, model, tools and AgentConfig before accepting.
        // A queued run must not observe a later set_model/set_thinking/tools
        // command merely because it starts after that command.
        let run_loop = self
            .agent_loop
            .try_read()
            .map_err(|_| anyhow::anyhow!("session run configuration is busy"))?
            .independent_copy();
        let payload = serde_json::to_value(ScheduledPromptPayload {
            message: prompt.message.to_string(),
            model_context: prompt.model_context.to_string(),
            images: images.to_vec(),
            attachments: attachment_snapshots,
            settings: settings.clone(),
        })?;
        let (ack, superseded, superseded_active) = if busy_policy
            == crate::runtime::BusyPolicy::SupersedeSession
        {
            let (ack, cancelled, active) =
                self.scheduler
                    .supersede(client_request_id, requested_run_id, payload)?;
            (ack, cancelled, active)
        } else {
            (
                self.scheduler
                    .accept(client_request_id, requested_run_id, busy_policy, payload)?,
                Vec::new(),
                None,
            )
        };
        if ack.accepted_state == crate::runtime::RunAcceptedState::Existing {
            return Ok(ack);
        }
        for request in superseded {
            self.scheduled_snapshots.remove(&request.run_id);
        }
        self.scheduled_snapshots.insert(
            ack.run_id.clone(),
            AcceptedRunSnapshot { run_loop, settings },
        );
        if busy_policy == crate::runtime::BusyPolicy::SupersedeSession
            && (superseded_active.is_some() || self.runtime.snapshot().is_some())
        {
            if let Some(active) = self.runtime.snapshot() {
                let _ = self.runtime.request_abort(Some(&active.run_id));
            }
        }
        if self.runtime.snapshot().is_none() {
            if self.runtime.has_owned_task() {
                // The prior run has finalized its control lease but its task
                // monitor still owns the slot. Keep both the request and its
                // accepted execution snapshot queued; the completion wake will
                // start it after the task is fully gone.
                return Ok(ack);
            }
            return match self.start_next_scheduled() {
                Ok(running) => Ok(running),
                Err(error) => {
                    if self
                        .scheduler
                        .active()
                        .is_some_and(|(active, _)| active.run_id == ack.run_id)
                    {
                        let _ = self.scheduler.finish_active(&ack.run_id);
                    } else {
                        let _ = self.scheduler.cancel_queued(
                            &ack.run_id,
                            crate::runtime::QueuedCancellationReason::Cancelled,
                        );
                    }
                    self.scheduled_snapshots.remove(&ack.run_id);
                    Err(error)
                }
            };
        }
        Ok(ack)
    }

    pub(super) fn start_next_scheduled(&mut self) -> Result<crate::runtime::RunAck> {
        if let Some(error) = self
            .broadcaster
            .persistence_error()
            .or_else(|| self.persistence.last_error())
        {
            return Err(crate::runtime::RunQueueError::PersistenceUnavailable(error).into());
        }
        let request = self
            .scheduler
            .queued()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("there is no queued run to start"))?;
        let payload: ScheduledPromptPayload = serde_json::from_value(request.payload.clone())?;
        let snapshot = self
            .scheduled_snapshots
            .remove(&request.run_id)
            .ok_or_else(|| anyhow::anyhow!("accepted run snapshot is unavailable"))?;
        debug_assert_eq!(snapshot.settings.model, payload.settings.model);
        #[cfg(test)]
        {
            let mut slot = SCHEDULED_DEQUEUE_HOOK.lock();
            if matches!(slot.as_ref(), Some((sid, _)) if sid == &request.run_id) {
                if let Some((_, hook)) = slot.take() {
                    hook(self);
                }
            }
        }
        let materialized_attachments =
            self.materialize_queued_attachments(&request.run_id, &payload.attachments)?;
        let lease = self.prompt_internal(
            PromptText::new(&payload.message, &payload.model_context),
            &payload.images,
            &materialized_attachments,
            Some(&request.run_id),
            Some(&request.client_request_id),
            Some(&request),
            Some(snapshot),
        )?;
        Ok(crate::runtime::RunAck {
            run_id: lease.run_id,
            run_epoch: lease.epoch,
            accepted_state: crate::runtime::RunAcceptedState::Running,
            run_sequence: lease.run_sequence,
            queue_position: None,
        })
    }

    fn materialize_queued_attachments(
        &self,
        run_id: &str,
        snapshots: &[QueuedAttachmentSnapshot],
    ) -> Result<Vec<crate::types::Attachment>> {
        if snapshots.is_empty() {
            return Ok(Vec::new());
        }
        let directory = self
            .session_manager
            .run_data_path(&self.session_id)
            .join("attachments")
            .join(run_id);
        std::fs::create_dir_all(&directory)?;
        snapshots
            .iter()
            .enumerate()
            .map(|(index, snapshot)| {
                let bytes =
                    base64::engine::general_purpose::STANDARD.decode(&snapshot.bytes_base64)?;
                let extension = std::path::Path::new(&snapshot.attachment.name)
                    .extension()
                    .and_then(|value| value.to_str())
                    .filter(|value| {
                        !value.is_empty()
                            && value.len() <= 16
                            && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    });
                let file_name = extension
                    .map(|extension| format!("{index:04}.{extension}"))
                    .unwrap_or_else(|| format!("{index:04}.bin"));
                let path = directory.join(file_name);
                std::fs::write(&path, bytes)?;
                let mut attachment = snapshot.attachment.clone();
                attachment.path = path.to_string_lossy().to_string();
                Ok(attachment)
            })
            .collect()
    }

    pub fn prompt(
        &mut self,
        msg: &str,
        images: &[crate::types::ImageContent],
        attachments: &[crate::types::Attachment],
        requested_run_id: Option<&str>,
        client_request_id: Option<&str>,
    ) -> Result<crate::runtime::RunLease> {
        self.prompt_with_model_context(
            PromptText::user_only(msg),
            images,
            attachments,
            requested_run_id,
            client_request_id,
        )
    }

    fn prompt_with_model_context(
        &mut self,
        prompt: PromptText<'_>,
        images: &[crate::types::ImageContent],
        attachments: &[crate::types::Attachment],
        requested_run_id: Option<&str>,
        client_request_id: Option<&str>,
    ) -> Result<crate::runtime::RunLease> {
        if let Some(error) = self.broadcaster.persistence_error() {
            return Err(crate::runtime::RunQueueError::PersistenceUnavailable(error).into());
        }
        self.prompt_internal(
            prompt,
            images,
            attachments,
            requested_run_id,
            client_request_id,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prompt_internal(
        &mut self,
        prompt: PromptText<'_>,
        images: &[crate::types::ImageContent],
        attachments: &[crate::types::Attachment],
        requested_run_id: Option<&str>,
        client_request_id: Option<&str>,
        scheduled: Option<&crate::runtime::ScheduledRunRequest>,
        accepted_snapshot: Option<AcceptedRunSnapshot>,
    ) -> Result<crate::runtime::RunLease> {
        let accepted_settings = accepted_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.settings);
        let run_cwd = accepted_settings
            .map(|settings| settings.cwd.clone())
            .unwrap_or_else(|| self.cwd.clone());
        let run_model = accepted_settings
            .map(|settings| settings.model.clone())
            .unwrap_or_else(|| self.model.clone());
        let run_auto_compaction = accepted_settings
            .map(|settings| settings.auto_compaction)
            .unwrap_or(self.auto_compaction);
        let run_permission_level = accepted_settings
            .map(|settings| settings.permission_level.clone())
            .unwrap_or_else(|| self.permission_level.clone());
        let run_sandbox_policy = if let Some(settings) = accepted_settings {
            settings
                .sandbox_tier
                .as_deref()
                .map(|tier| crate::sandbox::SandboxPolicy {
                    tier: crate::sandbox::SandboxTier::parse(tier),
                })
        } else {
            self.sandbox_policy.clone()
        };

        let cwd_path = std::path::Path::new(&run_cwd);
        crate::utils::ensure_workspace_accessible(
            cwd_path,
            crate::utils::is_future_managed_dir(cwd_path),
        )?;
        let (system_prompt, verbose, mut run_loop) = if let Some(snapshot) = accepted_snapshot {
            let mut run_loop = snapshot.run_loop;
            self.swap_token_counters_into_loop(&mut run_loop);
            self.wire_auto_compaction(&mut run_loop, run_auto_compaction, &run_model);
            let system_prompt = self.build_system_prompt(&run_cwd, run_loop.tools.clone());
            run_loop.system_prompt = system_prompt.clone();
            run_loop.config.system_prompt = system_prompt.clone();
            (system_prompt, run_loop.verbose, run_loop)
        } else {
            let mut shared = self
                .agent_loop
                .try_write()
                .map_err(|_| anyhow::anyhow!("session run configuration is busy"))?;
            // Apply session-level settings at the run boundary. Updates made
            // after this snapshot are intentionally deferred to the next run
            // and cannot partially affect the accepted run.
            let thinking_budget = match self.thinking_level.as_str() {
                "minimal" => 2_000,
                "low" => 4_000,
                "medium" => 8_000,
                "high" => 16_000,
                "xhigh" => 24_000,
                _ => 0,
            };
            shared.config.thinking_budget = thinking_budget;
            shared
                .provider
                .update_thinking(&self.thinking_level, thinking_budget);
            self.swap_token_counters_into_loop(&mut shared);
            self.wire_auto_compaction(&mut shared, run_auto_compaction, &run_model);
            let system_prompt = self.build_system_prompt(&run_cwd, shared.tools.clone());
            shared.system_prompt = system_prompt.clone();
            shared.config.system_prompt = system_prompt.clone();

            // Freeze provider/tools/config in the same short critical section.
            // The shared Loop remains the next-run control plane and can be
            // updated immediately while this independent snapshot streams.
            let mut snapshot = shared.independent_copy();
            snapshot.context_manager = shared.context_manager.clone();
            snapshot.active_checkpoint = shared.active_checkpoint.clone();
            (system_prompt, shared.verbose, snapshot)
        };
        run_loop.cumulative_input_tokens = self.tokens_in.clone();
        run_loop.cumulative_output_tokens = self.tokens_out.clone();
        run_loop.cumulative_cache_read_tokens = self.tokens_cache_r.clone();
        run_loop.cumulative_cache_write_tokens = self.tokens_cache_w.clone();
        run_loop.cumulative_cost = self.cumulative_cost.clone();
        run_loop.last_prompt_tokens = self.last_prompt_tokens.clone();

        // Whether the active model accepts image input (catalog modalities).
        // Uses the cached registry from ServerSession to avoid ~15% CPU overhead
        // from re-deserialising the full model catalog on every prompt.
        let model_supports_images =
            crate::models::model_accepts_images_with(&self.model_registry.read(), &run_model);
        // Images are read + (down)encoded to base64 here, on the agent, from the
        // local path the GUI sent — the base64 never crosses the wire.
        let mut user_message = build_user_message_with_model_context(
            prompt.message,
            prompt.model_context,
            images,
            attachments,
            model_supports_images,
            &crate::utils::image_data_url_for_model,
        );

        // NOTE: No content-based dedup here. Idempotency for transport-level
        // retries is enforced atomically by SessionRuntime::begin.
        // A text-based dedup at this point only ever fires when NOT streaming —
        // i.e. exactly the cases that must run: retrying after a failed run,
        // or deliberately repeating a message ("continue", "yes", same text
        // with different attachments).
        let user_text = user_message.text();
        let user_display_text = user_message.display_text();

        // Accept the run before mutating messages or persistence. This is the
        // sole Idle -> Starting transition, so abort -> resend cannot create a
        // second task while the cancelled task is still unwinding.
        let run_lease = if let Some(request) = scheduled {
            self.runtime.begin_scheduled(
                &request.run_id,
                &request.client_request_id,
                request.run_sequence,
            )?
        } else {
            self.runtime.begin(requested_run_id, client_request_id)?
        };
        if let Some(request) = scheduled {
            match self.scheduler.start_next(run_lease.epoch) {
                Ok((started, _)) if started.run_id == request.run_id => {}
                Ok((started, _)) => {
                    let _ = self.runtime.begin_finalizing(&run_lease);
                    let _ = self.runtime.finish(&run_lease);
                    return Err(anyhow::anyhow!(
                        "scheduler FIFO mismatch: expected {}, started {}",
                        request.run_id,
                        started.run_id
                    ));
                }
                Err(error) => {
                    let _ = self.runtime.begin_finalizing(&run_lease);
                    let _ = self.runtime.finish(&run_lease);
                    return Err(error.into());
                }
            }
            self.scheduler.release_active_payload(&request.run_id);
            #[cfg(test)]
            if crate::rpc::session_prompt::take_post_start_failure(&request.run_id).is_some() {
                let _ = self.runtime.begin_finalizing(&run_lease);
                let _ = self.runtime.finish(&run_lease);
                return Err(anyhow::anyhow!("injected post-start failure"));
            }
        }
        self.broadcaster.start_run_with_sequence(
            run_lease.run_id.clone(),
            run_lease.epoch as i64,
            run_lease.run_sequence,
        );
        // This run starts with a clean persistence-error state. Compaction state
        // is durable and intentionally survives across runs via checkpoints.
        self.persistence.reset_error();
        run_loop
            .stream_incomplete
            .store(false, std::sync::atomic::Ordering::SeqCst);
        // Stamp run identity on the message itself, not just the journal
        // entry: the terminal history rewrite regenerates entries from the
        // in-memory messages, so identity injected only at the entry layer
        // would be lost there. Assistant and tool entries inherit the run id
        // at save time (see save_closure/user_msg_cb).
        {
            let metadata = user_message
                .metadata
                .get_or_insert_with(serde_json::Map::new);
            metadata.insert(
                "run_id".to_string(),
                serde_json::Value::String(run_lease.run_id.clone()),
            );
        }
        user_message.ensure_journal_entry_id();
        self.messages.write().push(user_message);

        // Log the user message so the run log shows the question alongside
        // the answer (thinking/output blocks already land via eprint_log!).
        if verbose {
            tracing::info!("[user] {user_text}");
        }

        // Persist immediately so the GUI can see the user message (and any
        // tool entries from earlier runs) during streaming. Without this, a
        // thread switch mid-stream loses the question until the run settles
        // because get_session_entries reads from disk.
        // Ephemeral sessions (--no-session) skip persistence entirely.
        if !self.ephemeral {
            if let Err(error) = self.persist_user_message(&run_lease) {
                self.messages.write().pop();
                let _ = self.runtime.begin_finalizing(&run_lease);
                let full_error = format!("Failed to persist accepted user message: {error:#}");
                self.broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "error".to_string(),
                    data: serde_json::json!({"error": &full_error}).to_string(),
                    ..Default::default()
                });
                self.broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "agent_end".to_string(),
                    data: serde_json::json!({
                        "type": "agent_end",
                        "error": &full_error,
                    })
                    .to_string(),
                    ..Default::default()
                });
                let _ = self.runtime.finish(&run_lease);
                if scheduled.is_some() {
                    let _ = self.scheduler.finish_active(&run_lease.run_id);
                }
                return Err(error);
            }
        }

        // Broadcast only after the durability boundary succeeds. Use
        // display_text (first text block only): text() also joins the
        // agent-injected attachment manifest, which observers would render as
        // a bogus extra bubble.
        self.broadcaster.broadcast(crate::rpc::SseEvent::new(
            "user_message",
            serde_json::json!({"text": user_display_text}),
        ));

        // Clone shared state for the background task
        let messages_arc = self.messages.clone();
        let initial_messages = messages_arc.read().clone();
        let broadcaster = self.broadcaster.clone();
        let runtime = self.runtime.clone();
        let task_lease = run_lease.clone();
        let session_manager = self.session_manager.clone();
        let session_persistence = self.persistence.clone();
        let session_id = self.session_id.clone();
        let session_cwd = run_cwd;
        // Session metadata remains the latest control-plane state. Execution
        // uses the accepted run snapshot above; allowing an older queued run
        // to overwrite newer session settings at terminal would be a rollback.
        let persisted_session_cwd = self.cwd.clone();
        let session_model = self.model.clone();
        let session_thinking = self.thinking_level.clone();
        let tokens_in = self.tokens_in.clone();
        let tokens_out = self.tokens_out.clone();
        let tokens_cache_r = self.tokens_cache_r.clone();
        let tokens_cache_w = self.tokens_cache_w.clone();
        let cumulative_cost = self.cumulative_cost.clone();
        let last_prompt = self.last_prompt_tokens.clone();
        let session_name = self.session_name.clone();
        let created_by = self.created_by.clone();
        let source_meta = self.source_meta.clone();
        let auto_compaction = self.auto_compaction;
        let approval_gate = self.approval_gate.clone();
        let is_ephemeral = self.ephemeral;

        // Resolve the sandbox boundary once per run: canonicalized writable
        // roots + platform availability. Shared by the approval closure (pre-
        // execution decisions), the shell wrapper, and write/edit boundary
        // checks so all layers agree. No explicit policy (every non-GUI client)
        // → dormant sandbox = legacy behavior. Session rules are cleared at run
        // start and shared into the sandbox so same-run "allow in this
        // workspace" injections take effect immediately (APPROVAL_PLAN §6.2).
        self.session_rules.lock().clear();
        let sandbox = Arc::new(match &run_sandbox_policy {
            Some(policy) => crate::sandbox::ResolvedSandbox::resolve_with_session(
                policy,
                &session_cwd,
                self.session_rules.clone(),
            ),
            None => crate::sandbox::ResolvedSandbox::disabled(&session_cwd),
        });

        // Build per-session StreamContext (callbacks) — these are session-
        // specific closures and must NOT be stored on the shared Loop.
        let save_messages = messages_arc.clone();
        let save_persistence = self.persistence.clone();
        let persisted_run_id = run_lease.run_id.clone();
        let save_closure: crate::agent::PersistCallback =
            Arc::new(move |msg: &mut crate::types::AgentMessage| {
                if is_ephemeral {
                    return;
                }
                msg.ensure_journal_entry_id();
                let mut persisted = msg.clone();
                // Every entry of this run carries its run identity — not just
                // assistant entries — so a message's home run never has to be
                // re-derived from journal position. Existing ids win for
                // legacy messages that are re-saved during compaction.
                let metadata = persisted.metadata.get_or_insert_with(serde_json::Map::new);
                metadata
                    .entry("run_id".to_string())
                    .or_insert_with(|| serde_json::Value::String(persisted_run_id.clone()));
                save_messages.write().push(persisted.clone());
                let entry = crate::session::agent_message_to_entry(&persisted);
                if let Err(error) = save_persistence.append(vec![entry]) {
                    tracing::error!("Failed to enqueue session entry: {error}");
                }
            });
        let checkpoint_persistence = self.persistence.clone();
        let checkpoint_callback: crate::agent::CheckpointCallback =
            Arc::new(move |checkpoint: &crate::compaction::ContextCheckpoint| {
                checkpoint_persistence
                    .commit_checkpoint(crate::session::checkpoint_to_entry(checkpoint))
            });
        let stream_ctx = crate::agent::StreamContext {
            // Use the bare model ID from the Loop — the LLM API expects just
            // the model name, not the "provider/model" display format stored
            // on ServerSession.
            model: run_loop.model.clone(),
            system_prompt,
            on_tool_result: Some(save_closure.clone()),
            save_callback: Some(save_closure),
            on_checkpoint: (!is_ephemeral).then_some(checkpoint_callback),
        };

        // Set approval/sandbox hooks on this session's Loop config (these
        // are not callbacks — they're tool-execution hooks on AgentConfig).
        let approval_gate_hook = approval_gate.clone();
        let approval_broadcaster = broadcaster.clone();
        let approval_session_id = session_id.clone();
        let approval_cwd = session_cwd.clone();
        let approval_sandbox = sandbox.clone();
        let permission_level = run_permission_level.clone();
        run_loop.config.before_tool_call =
            Some(Arc::new(
                move |tool_name, tool_id, arguments| match permission_level.as_str() {
                    "all" => {
                        approve_tool_path_if_present(&approval_cwd, tool_name, arguments);
                        None
                    }
                    "none" => Some(crate::types::ToolCallResult {
                        result: format!(
                            "Tool call `{tool_name}` denied: permission level is set to 'none'."
                        ),
                        is_error: true,
                    }),
                    _ => approval_gate_hook.request(
                        &approval_broadcaster,
                        &approval_session_id,
                        &approval_cwd,
                        tool_name,
                        tool_id,
                        arguments,
                        &approval_sandbox,
                    ),
                },
            ));
        let prepare_cwd = session_cwd.clone();
        run_loop.config.prepare_tool_call = Some(Arc::new(move |tool_name, arguments| {
            prepare_session_tool_call(&prepare_cwd, tool_name, arguments)
        }));

        // agent_start is emitted inside run_streaming_with_messages via on_event.

        // Clear any stale interrupt flag copied from the next-run control
        // plane. Each run owns its snapshot after this boundary.
        run_loop.clear_interrupt();
        let shared_interrupt_flag = run_loop.interrupt_flag();
        self.broadcaster
            .set_persistence_interrupt(shared_interrupt_flag.clone());

        // Create interrupt channel so abort() can stop the current stream
        let (interrupt_tx, interrupt_rx) = tokio::sync::mpsc::channel::<()>(1);
        // Capture both cancellation paths in the run-scoped lease. Abort can
        // now signal the task without making the session appear idle.
        let cancellation_installed = self.runtime.install_cancellation(
            &run_lease,
            interrupt_tx,
            shared_interrupt_flag.clone(),
        );
        debug_assert!(cancellation_installed);

        // Post-hoc escalation channel (SANDBOX_PLAN.md §2.6): lets run_shell
        // raise a `sandbox_escalation` approval after a sandbox denial without
        // the tools layer touching RPC internals. Blocks until the user decides.
        let escalation: crate::sandbox::EscalationRequester = {
            let gate = approval_gate.clone();
            let escalation_broadcaster = broadcaster.clone();
            let escalation_session_id = session_id.clone();
            let escalation_sandbox = sandbox.clone();
            Arc::new(move |request: &crate::sandbox::EscalationRequest| {
                gate.request_escalation(
                    &escalation_broadcaster,
                    &escalation_session_id,
                    request,
                    &escalation_sandbox,
                )
            })
        };

        // Sandboxed-execution notifier → `tool_sandboxed` run event (Run
        // Inspect shows which commands ran inside the OS sandbox).
        let on_sandboxed: crate::tools::SandboxedNotifier = {
            let sandbox_broadcaster = broadcaster.clone();
            Arc::new(move |command: &str| {
                sandbox_broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "tool_sandboxed".to_string(),
                    data: serde_json::json!({
                        "type": "tool_sandboxed",
                        "command": command,
                    })
                    .to_string(),
                    ..Default::default()
                });
            })
        };

        // Build the run future; SessionRuntime owns spawning, monitoring, and
        // the task slot. TODO(runtime-persistence): once terminal persistence
        // is an ordered writer command, move the remaining execution-input
        // assembly into a dedicated RunExecution value.
        let perm = run_permission_level;
        let scope_sandbox = sandbox.clone();
        let run_task = async move {
            // Anchors for per-reply metadata written at the save site: wall-clock
            // start and the session-cumulative output-token count before this
            // prompt ran. The delta/elapsed are attributed to the final assistant
            // entry so the GUI can show "time · N tokens" when reloading history.
            let run_start = std::time::Instant::now();
            let out_start = tokens_out.load(std::sync::atomic::Ordering::Relaxed);
            let result = crate::tools::with_tool_scope(
                crate::tools::ScopeOptions {
                    workspace: session_cwd.clone(),
                    permission_level: perm,
                    interrupt_flag: shared_interrupt_flag,
                    sandbox: scope_sandbox,
                    escalation: Some(escalation),
                    on_sandboxed: Some(on_sandboxed),
                },
                async {
                    let be = broadcaster.clone();
                    run_loop
                        .run_streaming_with_messages(
                            initial_messages,
                            &stream_ctx,
                            |_| {},
                            move |event| {
                                if let Some(event) = run_event_to_sse(event) {
                                    be.broadcast(event);
                                }
                            },
                            Some(interrupt_rx),
                        )
                        .await
                        .map(|(_, final_messages)| final_messages)
                },
            )
            .await;

            // This check fences shared messages, disk commits, and terminal
            // events. A completion from an obsolete epoch is never allowed to
            // mutate a newer run.
            let was_cancelled = runtime.snapshot().is_some_and(|active| {
                active.run_id == task_lease.run_id
                    && active.epoch == task_lease.epoch
                    && matches!(
                        active.phase,
                        crate::runtime::RunPhase::Cancelling
                            | crate::runtime::RunPhase::CancellationStuck
                    )
            });
            let stream_incomplete = run_loop
                .stream_incomplete
                .load(std::sync::atomic::Ordering::SeqCst);
            if let Some(error) = broadcaster.persistence_error() {
                let _ = runtime.mark_persistence_degraded(&task_lease, &error);
                return;
            }
            if !runtime.begin_finalizing(&task_lease) {
                return;
            }

            let run_error = match result {
                Ok(mut final_messages) => {
                    reconcile_run_identity(&mut final_messages, &task_lease.run_id);
                    // Update shared messages so next prompt includes the full context
                    *messages_arc.write() = final_messages;
                    None
                }
                Err(error) => Some(format!("{error:#}")),
            };
            let run_output_tokens =
                (tokens_out.load(std::sync::atomic::Ordering::Relaxed) - out_start).max(0);
            let run_duration_ms = run_start.elapsed().as_millis() as i64;

            // Every terminal path reaches the same durability boundary before
            // the runtime may return to Idle.
            //
            // Append-only is the fast path: the run's user/assistant/tool
            // entries were already appended during the run, so we only add a
            // run_terminal marker plus a refreshed session_info snapshot, then
            // commit at a durable boundary — O(this run), not O(full history).
            //
            // A full rewrite is reserved for the two cases that make the
            // in-memory history diverge from the appended JSONL: compaction
            // replacing the message list, or a mid-run append failure. commit_run
            // reports the latter by refusing to commit, and we then heal via a
            // full rewrite (which also applies the compacted history).
            let terminal_state = if was_cancelled {
                crate::session::RUN_STATE_CANCELLED
            } else if run_error.is_some() {
                crate::session::RUN_STATE_ERROR
            } else if stream_incomplete {
                crate::session::RUN_STATE_INCOMPLETE
            } else {
                crate::session::RUN_STATE_COMPLETED
            };
            // Clones for the blocking commit task: run_error and task_lease are
            // still needed after the task completes (terminal event dispatch).
            let commit_run_id = task_lease.run_id.clone();
            let commit_run_error = run_error.clone();
            let persistence_task = tokio::task::spawn_blocking(move || {
                if is_ephemeral {
                    return anyhow::Ok(());
                }
                use std::sync::atomic::Ordering;

                // Preserve parent_session_id from the existing session on disk.
                let parent_session_id = session_manager
                    .load(&session_id)
                    .map(|s| s.parent_session_id)
                    .unwrap_or_default();

                // Build the authoritative session_info snapshot that records the
                // session's cumulative metadata at this run boundary. Auto-
                // generate session_name from the first user message when not set
                // explicitly (matches first_message in list_sessions).
                let resolved_name = if session_name.is_empty() {
                    messages_arc
                        .read()
                        .iter()
                        .find(|m| m.role == "user")
                        .map(|m| m.display_text())
                        .map(|s| crate::session::truncate_visible(s.trim(), 40))
                        .unwrap_or_default()
                } else {
                    session_name
                };
                let total_cost = *cumulative_cost.lock();
                let mut info = serde_json::json!({
                    "cwd": persisted_session_cwd,
                    "tokens_in": tokens_in.load(Ordering::Relaxed),
                    "tokens_out": tokens_out.load(Ordering::Relaxed),
                    "tokens_cache_r": tokens_cache_r.load(Ordering::Relaxed),
                    "tokens_cache_w": tokens_cache_w.load(Ordering::Relaxed),
                    "last_prompt_tokens": last_prompt.load(Ordering::Relaxed),
                    "total_cost": total_cost,
                    "session_name": resolved_name,
                    "auto_compaction": auto_compaction,
                    "parent_session_id": parent_session_id,
                    "thinking_level": session_thinking,
                    "model": session_model,
                });
                if !created_by.is_empty() {
                    info["created_by"] = serde_json::Value::String(created_by);
                }
                if !source_meta.is_null() {
                    info["source_meta"] = source_meta;
                }
                let info_entry = crate::session::SessionEntry::session_info(
                    info,
                    session_model.clone(),
                    session_thinking.clone(),
                );
                let run_started = crate::session::SessionEntry::run_started_with_sequence(
                    &commit_run_id,
                    task_lease.epoch,
                    task_lease.run_sequence,
                );
                let terminal = crate::session::SessionEntry::run_terminal(
                    &commit_run_id,
                    terminal_state,
                    run_output_tokens,
                    run_duration_ms,
                    commit_run_error.as_deref(),
                );

                // Append-only fast path: terminal marker + refreshed session_info,
                // committed at a durable (fsync) boundary. commit_run is ordered
                // after every mid-run append, so it refuses if any of them failed;
                // in that case heal with a full rewrite.
                // Keep the terminal marker last. A crash before the fsync may
                // leave a missing/partial tail, but cannot expose a terminal
                // marker followed by an incomplete metadata record.
                match session_persistence.commit_run(vec![info_entry.clone(), terminal.clone()]) {
                    Ok(()) => anyhow::Ok(()),
                    Err(commit_error) => {
                        tracing::warn!(
                            run_id = %commit_run_id,
                            "append-only run commit refused ({commit_error}); healing with full rewrite"
                        );
                        let messages = messages_arc.read().clone();
                        let session = Self::build_rewrite_snapshot(
                            &session_manager,
                            &session_id,
                            &session_cwd,
                            &session_model,
                            &resolved_name,
                            &parent_session_id,
                            &messages,
                            info_entry,
                            run_started,
                            terminal,
                        );
                        session_persistence.rewrite_run_snapshot(session)?;
                        anyhow::Ok(())
                    }
                }
            });
            let persistence_error = match persistence_task.await {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(format!("{error:#}")),
                Err(error) => Some(format!("persistence task failed: {error}")),
            };
            if let Some(error) = persistence_error {
                let _ = runtime.mark_persistence_degraded(&task_lease, &error);
                tracing::error!(
                    run_id = %task_lease.run_id,
                    "Session persistence commit failed: {error}"
                );
                broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "persistence_error".to_string(),
                    data: serde_json::json!({"error": &error}).to_string(),
                    ..Default::default()
                });
                broadcaster.broadcast(crate::rpc::SseEvent {
                    event_type: "agent_end".to_string(),
                    data: serde_json::json!({
                        "type": "agent_end",
                        "state": crate::session::RUN_STATE_ERROR,
                        "error": format!("Session persistence failed: {error}"),
                        "usage": { "output_tokens": run_output_tokens },
                        "duration_ms": run_duration_ms
                    })
                    .to_string(),
                    ..Default::default()
                });
                return;
            }

            match run_error {
                None => {
                    // Carry this run's output-token total, wall-clock duration,
                    // and terminal state on the event so clients (notably the IM
                    // channel bridges and remote mobile/web clients) can show the
                    // stats and distinguish a cancellation from a clean completion
                    // the instant the run settles — without depending on having
                    // seen every streamed event (a late-joining client may only
                    // have the tail of the run's event ring).
                    let mut data = serde_json::json!({
                        "type": "agent_end",
                        "state": terminal_state,
                        "usage": { "output_tokens": run_output_tokens },
                        "duration_ms": run_duration_ms
                    });
                    if stream_incomplete {
                        data["reason"] = serde_json::Value::String("incomplete".to_string());
                    }
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: data.to_string(),
                        ..Default::default()
                    });
                }
                Some(full_error) => {
                    tracing::error!("Agent loop error: {}", full_error);
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "error".to_string(),
                        data: serde_json::json!({"error": &full_error}).to_string(),
                        ..Default::default()
                    });
                    broadcaster.broadcast(crate::rpc::SseEvent {
                        event_type: "agent_end".to_string(),
                        data: serde_json::json!({
                            "type": "agent_end",
                            "state": terminal_state,
                            "error": &full_error,
                            "usage": { "output_tokens": run_output_tokens },
                            "duration_ms": run_duration_ms
                        })
                        .to_string(),
                        ..Default::default()
                    });
                }
            }
        };
        #[cfg(test)]
        {
            let mut slot = RUN_SPAWN_HOOK.lock();
            if matches!(slot.as_ref(), Some((sid, _)) if sid == &run_lease.run_id) {
                if let Some((_, hook)) = slot.take() {
                    hook(self);
                }
            }
        }
        if let Err(error) = self.runtime.spawn(run_lease.clone(), run_task) {
            let _ = self.runtime.begin_finalizing(&run_lease);
            let full_error = format!("Failed to start accepted run task: {error:#}");
            self.broadcaster.broadcast(crate::rpc::SseEvent {
                event_type: "error".to_string(),
                data: serde_json::json!({"error": &full_error}).to_string(),
                ..Default::default()
            });
            self.broadcaster.broadcast(crate::rpc::SseEvent {
                event_type: "agent_end".to_string(),
                data: serde_json::json!({
                    "type": "agent_end",
                    "error": &full_error,
                })
                .to_string(),
                ..Default::default()
            });
            let _ = self.runtime.finish(&run_lease);
            if scheduled.is_some() {
                let _ = self.scheduler.finish_active(&run_lease.run_id);
            }
            return Err(error);
        }

        Ok(run_lease)
    }
    /// Build this run's system prompt: project context (CLAUDE.md/AGENTS.md/
    /// GEMINI.md), workspace memory (FUTURE.md), discovered skills, and the
    /// write/memory guidelines. Read fresh each run (cwd-scoped).
    /// Point the agent loop's cumulative token/cost counters at this session's
    /// shared atomics so streaming updates are tracked per-session.
    fn swap_token_counters_into_loop(&self, r#loop: &mut crate::agent::Loop) {
        r#loop.cumulative_input_tokens = self.tokens_in.clone();
        r#loop.cumulative_output_tokens = self.tokens_out.clone();
        r#loop.cumulative_cache_read_tokens = self.tokens_cache_r.clone();
        r#loop.cumulative_cache_write_tokens = self.tokens_cache_w.clone();
        r#loop.cumulative_cost = self.cumulative_cost.clone();
        r#loop.last_prompt_tokens = self.last_prompt_tokens.clone();
    }

    /// Install the journal-aware context manager and restore the latest durable
    /// checkpoint. Provider-limit recovery uses the same manager even when
    /// automatic threshold compaction is disabled.
    fn wire_auto_compaction(&self, r#loop: &mut crate::agent::Loop, enabled: bool, model: &str) {
        let context_window = self
            .model_registry
            .read()
            .resolve(model)
            .map(|m| m.context_window)
            .unwrap_or(1_000_000);
        let reserve_tokens = ((context_window as f64 * 0.1) as i32).max(16384);
        let keep_recent_tokens = ((context_window as f64 * 0.2) as i32).max(reserve_tokens);
        r#loop.context_manager = Some(crate::compaction::ContextManager {
            enabled,
            reserve_tokens,
            keep_recent_tokens,
            context_window,
            model: model.to_string(),
        });
        let checkpoint = self
            .session_manager
            .load(&self.session_id)
            .ok()
            .and_then(|session| crate::session::latest_context_checkpoint(&session.entries));
        *r#loop.active_checkpoint.lock() = checkpoint;
    }

    fn build_system_prompt(&self, cwd: &str, tools: Vec<crate::types::AgentTool>) -> String {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        // Discover skills so they appear in the system prompt's <available_skills> block.
        // Global user-level dirs only — identical for every session/cwd, which
        // keeps the skills cache correct regardless of which session refreshes it.
        let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());

        // Load project context (AGENTS.md / CLAUDE.md / GEMINI.md)
        let mut agent_content = String::new();
        for fname in &["AGENTS.md", "CLAUDE.md", "GEMINI.md"] {
            let p = std::path::Path::new(cwd).join(fname);
            if p.exists() {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    agent_content = content;
                    break;
                }
            }
        }

        // Load workspace memory (FUTURE.md) — a separate layer from project
        // context, read fresh each run (cwd only; workspace-scoped). The index
        // is linted and capped at load so bloat, malformed lines, and dead
        // links surface to the model as a repairable warning.
        let memory_path = std::path::Path::new(cwd).join("FUTURE.md");
        let memory_content = crate::prompt::lint_memory_index(
            &std::fs::read_to_string(&memory_path).unwrap_or_default(),
            std::path::Path::new(cwd),
        );

        crate::prompt::build_prompt(&crate::prompt::PromptOptions {
            working_directory: cwd.replace('\\', "/"),
            date: today,
            tools,
            skills,
            agent_content,
            memory_content,
            session_id: self.session_id.clone(),
            model: self.model.clone(),
            thinking_level: self.thinking_level.clone(),
            prompt_guidelines: vec![
                // The write-via-shell prohibition is platform-neutral, but its
                // examples must name the redirection forms the host's shell
                // actually has, or a PowerShell model won't map "don't use
                // `cat > file`" onto "don't use Out-File".
                {
                    #[cfg(not(target_os = "windows"))]
                    let forms = "`>`, `>>`, tee, heredocs, `cat > file`";
                    #[cfg(target_os = "windows")]
                    let forms = "`>`, `>>`, Out-File, Set-Content, Add-Content";
                    format!("When asked to create, save, write, or modify a file, ALWAYS use the write or edit tool — including for absolute paths and paths outside the current working directory (both tools accept any path). Do NOT use shell redirection ({forms}) to write files: shell file writes bypass file tracking and the approval flow. Reserve shell redirection for piping between commands, not for creating files. Only describe file changes after the tool succeeds.")
                },
            ],
            ..Default::default()
        })
    }

    /// Persist the just-pushed user message so the GUI sees it mid-stream.
    /// Uses append-only when the session file already exists (avoids a full
    /// rewrite); falls back to full save for a brand-new session that has no
    /// JSONL yet.  The session_info line (token counts, model, name) stays
    /// at its last-completed-run values — the final save at run end refreshes
    /// it. A failure rejects StartRun so memory and JSONL cannot diverge before
    /// the model begins producing side effects.
    fn persist_user_message(&self, run_lease: &crate::runtime::RunLease) -> Result<()> {
        // Use in-memory parent_session_id — avoids reading the entire session
        // file from disk just to get one field.
        let parent_session_id = self.parent_session_id.clone();
        let msgs = self.messages.read();
        // The run_started marker is persisted together with the user message so
        // the journal durably records that this run began. A run_started with no
        // matching run_terminal identifies a run interrupted by crash/restart.
        let run_started = crate::session::SessionEntry::run_started_with_sequence(
            &run_lease.run_id,
            run_lease.epoch,
            run_lease.run_sequence,
        );

        // Fast path: an existing session atomically closes any run left open by
        // a prior process restart and appends this run's user/start records.
        // Refuse the new run if that durability boundary fails: allowing the new
        // run_started through would hide the older open marker.
        if let Some(last_msg) = msgs.last() {
            let entry = crate::session::agent_message_to_entry(last_msg);
            if self.session_manager.find(&self.session_id).is_some() {
                self.session_manager.append_run_start(
                    &self.session_id,
                    entry,
                    run_started.clone(),
                )?;
                return Ok(());
            }
        }

        // Slow path: full save for a brand-new session.
        let mut entries: Vec<crate::session::SessionEntry> = msgs
            .iter()
            .map(crate::session::agent_message_to_entry)
            .collect();
        // Prepend session_info so token counts and other metadata survive
        // a crash — without this, a restarted session starts with zeroed
        // token counters and may skip needed compaction.
        {
            use std::sync::atomic::Ordering;
            let session_name = if !self.session_name.is_empty() {
                self.session_name.clone()
            } else {
                entries
                    .iter()
                    .find(|e| e.role == "user")
                    .and_then(|e| e.content.as_ref())
                    .map(|c| {
                        // agent_message_to_entry always serializes content as
                        // an array, so the legacy plain-string arm is
                        // unreachable here — handle the array shape directly.
                        c.as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                    .next()
                                    .unwrap_or("")
                                    .to_string()
                            })
                            .unwrap_or_default()
                    })
                    .map(|s| crate::session::truncate_visible(s.trim(), 40))
                    .unwrap_or_default()
            };
            let mut info = serde_json::json!({
                "cwd": self.cwd,
                "tokens_in": self.tokens_in.load(Ordering::Relaxed),
                "tokens_out": self.tokens_out.load(Ordering::Relaxed),
                "tokens_cache_r": self.tokens_cache_r.load(Ordering::Relaxed),
                "tokens_cache_w": self.tokens_cache_w.load(Ordering::Relaxed),
                "last_prompt_tokens": self.last_prompt_tokens.load(Ordering::Relaxed),
                "total_cost": *self.cumulative_cost.lock(),
                "session_name": session_name,
                "auto_compaction": self.auto_compaction,
                "parent_session_id": parent_session_id,
                "thinking_level": self.thinking_level.clone(),
                "model": self.model.clone(),
            });
            if !self.created_by.is_empty() {
                info["created_by"] = serde_json::Value::String(self.created_by.clone());
            }
            if !self.source_meta.is_null() {
                info["source_meta"] = self.source_meta.clone();
            }
            let info_entry = crate::session::SessionEntry::session_info(
                info,
                self.model.clone(),
                self.thinking_level.clone(),
            );
            entries.insert(0, info_entry);
        }
        // Record the run_started marker after the user message so the brand-new
        // session's journal also bounds this run (matches the fast path).
        entries.push(run_started);
        let session = crate::session::Session::snapshot(
            self.session_id.clone(),
            self.cwd.clone(),
            self.model.clone(),
            self.session_name.clone(),
            parent_session_id,
            entries,
        );
        self.session_manager.save(&session)?;
        Ok(())
    }

    /// Rebuild the complete session snapshot from the in-memory message list for
    /// a full rewrite. This is the O(history) path, reserved for runs where
    /// compaction replaced the message history (diverging from the appended
    /// JSONL) or where a mid-run append failed and the file must be healed.
    ///
    /// `info_entry` is the authoritative session_info snapshot (prepended).
    /// On-disk timestamps and prior-run token stats are preserved by index so a
    /// rewrite doesn't reset every message to "just now"; this run's output
    /// tokens and duration are attached to the final assistant entry.
    #[allow(clippy::too_many_arguments)]
    fn build_rewrite_snapshot(
        session_manager: &crate::session::Manager,
        session_id: &str,
        session_cwd: &str,
        session_model: &str,
        resolved_name: &str,
        parent_session_id: &str,
        messages: &[crate::types::AgentMessage],
        info_entry: crate::session::SessionEntry,
        run_started: crate::session::SessionEntry,
        terminal: crate::session::SessionEntry,
    ) -> crate::session::Session {
        use crate::session::SessionEntry;
        let mut entries: Vec<SessionEntry> = messages
            .iter()
            .map(crate::session::agent_message_to_entry)
            .collect();

        // The whole session is rebuilt from the in-memory message list, and
        // agent_message_to_entry re-stamps `now()` with zero token/duration.
        // Without preserving them, every reload shows all messages at the
        // current time ("just now") and drops earlier replies' token counts.
        // Messages only grow by appending, so the on-disk message entries align
        // by index with this prefix. Filter the old side to message entries only
        // (matching what the rebuild produces) so interleaved label/model_change
        // /run-marker entries can't shift the alignment.
        let is_message_entry = |t: &str| {
            matches!(
                t,
                crate::session::ENTRY_TYPE_USER
                    | crate::session::ENTRY_TYPE_ASSISTANT
                    | crate::session::ENTRY_TYPE_TOOL
                    | crate::session::ENTRY_TYPE_SYSTEM
            )
        };
        let old_session = session_manager.load(session_id).ok();
        let old_msg_entries: std::collections::HashMap<String, SessionEntry> = old_session
            .as_ref()
            .map(|session| {
                session
                    .entries
                    .iter()
                    .filter(|e| is_message_entry(&e.entry_type))
                    .map(|entry| (entry.id.clone(), entry.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for new_entry in &mut entries {
            if let Some(old_entry) = old_msg_entries.get(&new_entry.id) {
                new_entry.timestamp = old_entry.timestamp;
            }
            // NOTE: run output tokens + duration live in the run_terminal
            // marker's content (see the markers section below), not in an
            // assistant entry's block-array content — so there is nothing to
            // preserve/attach here (the old object-content arms were dead).
        }

        // Healing a failed append must retain every durable checkpoint. Insert
        // them in journal order immediately after their cutoff; chained
        // checkpoints may reference an earlier checkpoint entry. Legacy
        // checkpoints have no cutoff because their prefix was already removed.
        let checkpoints: Vec<SessionEntry> = old_session
            .as_ref()
            .map(|session| {
                session
                    .entries
                    .iter()
                    .filter(|entry| entry.entry_type == crate::session::ENTRY_TYPE_COMPACTION)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for checkpoint in checkpoints {
            let cutoff = checkpoint
                .content
                .as_ref()
                .and_then(|content| content.get("cutoff_entry_id"))
                .and_then(serde_json::Value::as_str);
            let insertion = cutoff
                .and_then(|cutoff| entries.iter().position(|entry| entry.id == cutoff))
                .map_or(0, |index| index + 1);
            entries.insert(insertion, checkpoint);
        }

        // A rewrite is a journal compaction/repair, not permission to erase run
        // history. Preserve existing lifecycle markers, ensure the current
        // start marker exists, and make the current terminal marker the final
        // durable record. Markers remain projection-invisible.
        let current_run_id = run_started
            .content
            .as_ref()
            .and_then(|content| content.get("run_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let mut markers: Vec<SessionEntry> = old_session
            .as_ref()
            .map(|session| {
                session
                    .entries
                    .iter()
                    .filter(|entry| crate::session::is_run_marker(&entry.entry_type))
                    .filter(|entry| {
                        // Compute both legs eagerly: the content chain is pure,
                        // and eager evaluation keeps both the terminal and
                        // non-terminal filter arms line-covered.
                        let is_current_terminal =
                            entry.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL;
                        let other_run = entry
                            .content
                            .as_ref()
                            .and_then(|content| content.get("run_id"))
                            .and_then(serde_json::Value::as_str)
                            != Some(current_run_id);
                        !is_current_terminal || other_run
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let has_current_start = markers.iter().any(|entry| {
            entry.entry_type == crate::session::ENTRY_TYPE_RUN_STARTED
                && entry
                    .content
                    .as_ref()
                    .and_then(|content| content.get("run_id"))
                    .and_then(serde_json::Value::as_str)
                    == Some(current_run_id)
        });
        if !has_current_start {
            markers.push(run_started);
        }

        entries.insert(0, info_entry);
        entries.extend(markers);
        entries.push(terminal);

        crate::session::Session::snapshot(
            session_id.to_string(),
            session_cwd.to_string(),
            session_model.to_string(),
            resolved_name.to_string(),
            parent_session_id.to_string(),
            entries,
        )
    }
}

/// Reconcile run identity across a run's final in-memory history before it
/// replaces the shared session messages (and before any terminal rewrite
/// regenerates journal entries from it). The run loop builds its own
/// un-stamped message copies (assistant/tool results), while the journal
/// entries persisted mid-run were stamped at save time — without this pass
/// the rewrite would diverge from them.
///
/// The sweep covers only this run's messages: it starts at the run's opening
/// user message, which was stamped before the run began, and never touches
/// earlier runs — their entries carry their own run identities (or, for
/// legacy journals, none at all, and they must stay that way). Existing ids
/// win: a message first journaled with a run id keeps it.
fn reconcile_run_identity(messages: &mut [crate::types::AgentMessage], run_id: &str) {
    let start = messages
        .iter()
        .position(|message| {
            message.role == "user"
                && message
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("run_id"))
                    .and_then(|value| value.as_str())
                    == Some(run_id)
        })
        .unwrap_or(messages.len());
    for message in &mut messages[start..] {
        let metadata = message.metadata.get_or_insert_with(serde_json::Map::new);
        metadata
            .entry("run_id".to_string())
            .or_insert_with(|| serde_json::Value::String(run_id.to_string()));
    }
    // The run's final assistant reply is always attributable to it, even when
    // a compaction rewrite cost the opening user message its stamp.
    if let Some(last_assistant) = messages
        .iter_mut()
        .rev()
        .find(|message| message.role == "assistant")
    {
        last_assistant
            .metadata
            .get_or_insert_with(serde_json::Map::new)
            .entry("run_id".to_string())
            .or_insert_with(|| serde_json::Value::String(run_id.to_string()));
    }
}

#[cfg(test)]
mod build_user_message_tests;
#[cfg(test)]
mod tests;
