//! Run-scoped adapter for the shared, durable Host consultation queue.
use crate::types::{AgentTool, FunctionDef, ToolDef};
use anyhow::{ensure, Context, Result};
use future_fusion::{ConsultationInput, Parent, Snapshot, Store};
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone)]
pub(crate) enum ContextScope {
    Parent {
        parent: Parent,
        workspace: PathBuf,
        store: Store,
        sandbox: Arc<crate::sandbox::ResolvedSandbox>,
        cancelled: Arc<AtomicBool>,
    },
    Inner(Arc<Snapshot>),
}

tokio::task_local! { static CONTEXT: ContextScope; }

pub(crate) async fn with_context<F: Future>(context: Option<ContextScope>, future: F) -> F::Output {
    match context {
        Some(context) => CONTEXT.scope(context, future).await,
        None => future.await,
    }
}

pub(crate) fn available() -> bool {
    settings_enabled().unwrap_or(false)
        && Store::existing(&crate::utils::future_home()).is_ok_and(|s| s.host_available())
}

pub(crate) fn settings_enabled() -> Result<bool> {
    let home = crate::utils::future_home();
    if !home.join("fusion/fusion.db").exists() {
        return Ok(true);
    }
    Ok(Store::existing(&home)?.settings()?.enabled)
}

pub(crate) fn query_settings() -> Result<Value> {
    Ok(serde_json::to_value(
        Store::open(&crate::utils::future_home())?.settings()?,
    )?)
}

pub(crate) fn update_settings(state: &crate::rpc::AppState, raw: &str) -> Result<Value> {
    ensure!(raw.len() <= 16 * 1024, "Fusion settings exceed size limit");
    let mut patch: future_fusion::SettingsPatch = serde_json::from_str(raw)?;
    if let Some(config) = &mut patch.configuration {
        config.validate()?;
        let registry = state.model_registry.read();
        for profile in config.panel.iter_mut().chain([&mut config.moderator]) {
            let (model, _) = registry
                .resolve_for_request(&profile.model)
                .with_context(|| format!("Fusion model unavailable: {}", profile.model))?;
            ensure!(
                model
                    .allowed_thinking_levels()
                    .contains(&profile.thinking_level),
                "Unsupported thinking level for {}: {}",
                profile.model,
                profile.thinking_level
            );
            ensure!(
                model.output.iter().any(|kind| kind == "text"),
                "Fusion requires a text model"
            );
            profile.model = format!("{}/{}", model.provider, model.id);
            if model.max_tokens > 0 {
                profile.max_output_tokens = profile.max_output_tokens.min(model.max_tokens);
                profile.retry_max_output_tokens =
                    profile.retry_max_output_tokens.min(model.max_tokens);
            }
        }
    }
    let settings = Store::open(&crate::utils::future_home())?.update_settings(patch)?;
    state
        .control_plane
        .global_events_broadcaster()
        .broadcast(crate::rpc::SseEvent::new(
            "fusion_settings_changed",
            json!({"revision":settings.revision}),
        ));
    Ok(serde_json::to_value(settings)?)
}

fn tool(
    name: &str,
    description: &str,
    parameters: Value,
    handler: crate::types::ToolHandler,
) -> AgentTool {
    AgentTool {
        def: ToolDef {
            tool_type: "function".into(),
            function: FunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        },
        handler,
        guidelines: vec![],
    }
}

pub(crate) fn main_tool() -> AgentTool {
    tool(
        "fusion",
        future_fusion::TOOL_DESCRIPTION,
        json!({"type":"object","additionalProperties":false,"required":["question","goal","background","constraints"],"properties":{
            "question":{"type":"string","description":"One focused decision, not the whole task"},"goal":{"type":"string"},"background":{"type":"string","description":"Confirmed facts, relevant investigation, assumptions and unknowns"},"constraints":{"type":"array","items":{"type":"string"}},"options":{"type":"array","items":{"type":"string"}},"evidence":{"type":"array","items":{"type":"string"}},"read_scope":{"type":"array","items":{"type":"string"},"description":"Only necessary workspace-relative files/directories. Omit for conceptual decisions."}
        }}),
        invoke,
    )
}

fn invoke(args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        let context = CONTEXT
            .try_with(Clone::clone)
            .context("Fusion unavailable outside an explicitly enabled parent run")?;
        let ContextScope::Parent {
            mut parent,
            workspace,
            store,
            sandbox,
            cancelled,
        } = context
        else {
            anyhow::bail!("Recursive Fusion is prohibited")
        };
        ensure!(
            store.host_available(),
            "Fusion host is unavailable; start the FutureOS host"
        );
        parent.tool_call_id = crate::tools::current_tool_call_id();
        ensure!(
            !parent.tool_call_id.is_empty(),
            "Missing Fusion tool call identity"
        );
        let input: ConsultationInput = serde_json::from_value(args)?;
        input.validate()?;
        let (run, created) = store.prepare(parent, input, &workspace)?;
        let mut guard = InvocationGuard {
            store: store.clone(),
            id: run.id.clone(),
            armed: true,
        };
        if created {
            let root = PathBuf::from(&run.snapshot_root);
            let scopes = run.input.read_scope.clone();
            let flag = cancelled.clone();
            let captured = tokio::task::spawn_blocking(move || {
                Snapshot::capture(&workspace, &root, &scopes, &flag, |path| {
                    !sandbox.enabled()
                        || sandbox.evaluate(path, crate::sandbox::rules::Op::Read)
                            == crate::sandbox::rules::Decision::Allow
                })
            })
            .await?;
            if let Err(error) = captured {
                store.fail(&run.id, None, &format!("Snapshot failed: {error:#}"))?;
                return Err(error);
            }
            store.ready(&run.id)?;
        }
        let timeout_ms = run
            .configuration
            .as_ref()
            .map(|config| config.run_timeout_secs)
            .unwrap_or(future_fusion::RUN_TIMEOUT_SECS)
            .saturating_mul(1000);
        let remaining_ms = timeout_ms.saturating_sub(
            future_fusion::now_ms()
                .saturating_sub(run.created_at)
                .max(0) as u64,
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(remaining_ms);
        loop {
            if cancelled.load(Ordering::Acquire) {
                store.cancel(&run.id)?;
                anyhow::bail!("Fusion cancelled");
            }
            let current = store.get(&run.id)?;
            if let Some(result) = current.result {
                ensure!(
                    matches!(current.status.as_str(), "delivering" | "completed"),
                    "Fusion result belongs to an interrupted invocation"
                );
                guard.armed = false;
                return Ok(serde_json::to_string(&result)?);
            }
            ensure!(
                !current.terminal(),
                "Fusion {}: {}",
                current.status,
                current.error.as_deref().unwrap_or("no final result")
            );
            if tokio::time::Instant::now() >= deadline || !store.host_available() {
                store.fail(
                    &run.id,
                    None,
                    "Fusion host disappeared or consultation timed out",
                )?;
                anyhow::bail!(
                    "Fusion host disappeared or consultation timed out; no conclusion was produced"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
}

// Aborting the parent task also cancels its accepted consultation. The worker
// observes the durable state and aborts only the child runs it owns.
struct InvocationGuard {
    store: Store,
    id: String,
    armed: bool,
}
impl Drop for InvocationGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.store.cancel(&self.id);
        }
    }
}

fn query(name: &'static str, args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    Box::pin(async move {
        let context = CONTEXT
            .try_with(Clone::clone)
            .context("Read-only Fusion context missing")?;
        let ContextScope::Inner(snapshot) = context else {
            anyhow::bail!("Fusion file tools are only available inside a consultation")
        };
        tokio::task::spawn_blocking(move || snapshot.query(name, args)).await?
    })
}
fn read(args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    query("read", args)
}
fn ls(args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    query("ls", args)
}
fn glob(args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    query("glob", args)
}
fn grep(args: Value) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> {
    query("grep", args)
}

pub(crate) fn inner_tools() -> Vec<AgentTool> {
    vec![
        tool(
            "read",
            "Read lines from this consultation's immutable authorized file snapshot.",
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}}),
            read,
        ),
        tool(
            "ls",
            "List directories in the authorized snapshot; no commands are executed.",
            json!({"type":"object","properties":{"path":{"type":"string"},"depth":{"type":"integer"},"limit":{"type":"integer"}}}),
            ls,
        ),
        tool(
            "glob",
            "Find snapshot files by glob pattern.",
            json!({"type":"object","required":["pattern"],"properties":{"pattern":{"type":"string"},"root":{"type":"string"},"limit":{"type":"integer"}}}),
            glob,
        ),
        tool(
            "grep",
            "Search UTF-8 snapshot contents by a bounded regular expression.",
            json!({"type":"object","required":["pattern"],"properties":{"pattern":{"type":"string"},"root":{"type":"string"},"glob":{"type":"string"},"context":{"type":"integer"},"limit":{"type":"integer"}}}),
            grep,
        ),
    ]
}

pub(crate) struct InnerExecution {
    pub snapshot: Snapshot,
    pub settings: future_fusion::StepSettings,
    pub file_tools: bool,
}

pub(crate) fn inner_execution(
    meta: &Value,
    session: &str,
    agent_run: &str,
    model: &str,
    thinking: &str,
) -> Result<Option<InnerExecution>> {
    let Some(id) = meta.get("fusionRunId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let store = Store::existing(&crate::utils::future_home())?;
    let run = store.get(id)?;
    ensure!(run.status == "running", "Fusion is not active");
    let step_id = meta["stepId"]
        .as_str()
        .context("Fusion step identity is missing")?;
    let step = store
        .steps(id)?
        .into_iter()
        .find(|step| step.id == step_id)
        .context("Fusion step not found")?;
    ensure!(
        step.session_id == session && step.agent_run_id == agent_run && step.model == model,
        "Fusion child execution differs from recorded step"
    );
    let settings = step
        .settings
        .context("Fusion generation settings are missing; upgrade Host and Agent together")?;
    ensure!(
        settings.thinking_level == thinking,
        "Fusion thinking level differs from configured policy; fallback prohibited"
    );
    ensure!(
        settings.max_output_tokens > 0,
        "Invalid Fusion output limit"
    );
    Ok(Some(InnerExecution {
        snapshot: Snapshot::load(Path::new(&run.snapshot_root))?,
        settings,
        file_tools: !run.input.read_scope.is_empty(),
    }))
}

pub(crate) fn record_delivery(result: &str) -> Result<()> {
    if let Ok(value) = serde_json::from_str::<future_fusion::Conclusion>(result) {
        let ContextScope::Parent {
            mut parent, store, ..
        } = CONTEXT
            .try_with(Clone::clone)
            .context("Fusion delivery context missing")?
        else {
            anyhow::bail!("Invalid Fusion delivery context")
        };
        parent.tool_call_id = crate::tools::current_tool_call_id();
        // The tool ID task-local ends before persistence; verify the run/session
        // source as well as the result's deterministic consultation ID.
        let run = store.get(&value.fusion_run_id)?;
        ensure!(
            run.parent.database_id == parent.database_id
                && run.parent.session_id == parent.session_id
                && run.parent.run_id == parent.run_id,
            "Fusion delivery source mismatch"
        );
        store.delivered(&value.fusion_run_id)?;
    }
    Ok(())
}

/// Bound each inner completion without changing any live session settings.
pub(crate) struct LimitedProvider {
    pub provider: Arc<dyn crate::types::LLMProvider>,
    pub max_output_tokens: i32,
}
#[async_trait::async_trait]
impl crate::types::LLMProvider for LimitedProvider {
    async fn stream_model(
        &self,
        request: crate::llm::schema::ModelRequest,
    ) -> Result<tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>> {
        self.provider
            .stream_model_with_output_limit(request, self.max_output_tokens)
            .await
    }
}

pub(crate) fn query_status(state: &crate::rpc::AppState) -> Result<Value> {
    let identity = state.session_manager.storage()?.storage_identity()?;
    let settings = Store::open(&crate::utils::future_home())?.settings()?;
    Ok(json!({
        "available": available(), "enabled": settings.enabled,
        "database_id": format!("{}:{}", identity.database_id, identity.restore_generation),
        "panel_models": settings.configuration.panel.iter().map(|role| &role.model).collect::<Vec<_>>(),
        "moderator_model": settings.configuration.moderator.model,
        "generation_policy_version": future_fusion::GENERATION_POLICY_VERSION,
    }))
}

pub(crate) fn query_discussion(
    state: &crate::rpc::AppState,
    cmd: &crate::rpc::RpcCommand,
) -> Result<Value> {
    ensure!(
        !cmd.session_id.is_empty(),
        "Fusion reads require the parent session"
    );
    ensure!(
        state.session_manager.contains(&cmd.session_id)?,
        "Parent session not found"
    );
    let store = Store::existing(&crate::utils::future_home())?;
    let tool = cmd.tool_call_id.as_deref().unwrap_or_default();
    let run = if cmd.run_id.is_empty() && tool.is_empty() {
        store.latest_for_session(&cmd.session_id)?
    } else {
        ensure!(
            !cmd.run_id.is_empty() && !tool.is_empty(),
            "Provide both parent run and tool call ID"
        );
        store.for_tool(&cmd.session_id, &cmd.run_id, tool)?
    };
    let Some(run) = run else {
        return Ok(json!({"pending":true}));
    };
    let identity = state.session_manager.storage()?.storage_identity()?;
    ensure!(
        run.parent
            .database_id
            .starts_with(&format!("{}:", identity.database_id)),
        "Fusion belongs to a different Agent database"
    );
    let mut discussion = store.discussion(&run.id)?;
    if !cmd.entry_id.is_empty() {
        let step = discussion
            .steps
            .into_iter()
            .find(|s| s.id == cmd.entry_id)
            .context("Step does not belong to this consultation")?;
        let mut value = serde_json::to_value(&step)?;
        if !step.session_id.is_empty() {
            let session = state.session_manager.load(&step.session_id)?;
            let messages = crate::session::entries_to_agent_messages(&session.entries, false);
            let transcript = messages
                .iter()
                .map(|m| {
                    let content = m
                        .content
                        .iter()
                        .filter(|b| !matches!(b, crate::types::ContentBlock::Reasoning { .. }))
                        .collect::<Vec<_>>();
                    json!({"role":m.role,"content":content})
                })
                .collect::<Vec<_>>();
            let encoded = serde_json::to_string(&transcript)?;
            let offset = cmd.offset.unwrap_or(0).max(0) as usize;
            ensure!(
                offset <= encoded.len() && encoded.is_char_boundary(offset),
                "Invalid discussion byte offset"
            );
            let mut end = (offset + 32 * 1024).min(encoded.len());
            while !encoded.is_char_boundary(end) {
                end -= 1;
            }
            value["transcript"] = json!(&encoded[offset..end]);
            value["next_offset"] = json!(end);
            value["total_bytes"] = json!(encoded.len());
        }
        return Ok(value);
    }
    for step in &mut discussion.steps {
        step.input.clear();
    }
    Ok(serde_json::to_value(discussion)?)
}

/// Avoid hydrating child message bodies during catalogue enumeration.
pub(crate) fn child_sessions() -> std::collections::HashSet<String> {
    Store::existing(&crate::utils::future_home())
        .and_then(|s| s.child_sessions())
        .unwrap_or_default()
        .into_iter()
        .collect()
}
