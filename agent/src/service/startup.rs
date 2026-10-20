//! Shared model, engine, session-store and listener initialization.
use super::{AgentConfig, StartupCancellation};
use crate::{Engine, EngineConfig, Manager, ModelRegistry};
use anyhow::{Context, Result};
use chrono::Local;
use std::sync::Arc;

pub(super) async fn initialize(
    config: &AgentConfig,
    model_registry: Arc<parking_lot::RwLock<ModelRegistry>>,
    cancellation: &StartupCancellation,
    service_work: Arc<super::work::ServiceWork>,
    synced_state: Arc<parking_lot::RwLock<Option<crate::rpc::AppState>>>,
) -> Result<(crate::rpc::AppState, crate::grpc::AgentListener)> {
    cancellation.check()?;
    config.validate_environment()?;
    let cwd = crate::utils::home_dir().to_string_lossy().to_string();

    let all_models = model_registry.read().all_models();

    // Load settings
    let settings_path = std::path::PathBuf::from(crate::models::settings_path());
    let settings = match crate::config::load_settings(&settings_path) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(
                "Failed to load settings from {}: {}. Falling back to defaults.",
                settings_path.display(),
                error
            );
            crate::Settings::default()
        }
    };

    // Load auth store
    let auth_store = crate::AuthStore::load();

    // Resolve the *initial* model: the settings default, else the first model
    // that has credentials (a built-in key or an auth.json entry for its
    // provider). This is only a starting point — clients (GUI/TUI) set their own
    // model per session via the `set_model` RPC, which rebuilds the registry and
    // reloads auth.json, so the initial choice is not authoritative.
    //
    // A fresh install must serve without credentials or an initial model:
    // clients need the Agent to log in. set_model reloads committed auth and
    // resolves the actual target later; startup must not exit the process.
    let resolved_model = {
        // Prefer future/deepseek-v4-pro when the future provider is configured.
        let preferred = if auth_store.get("future").is_some()
            || all_models
                .iter()
                .any(|m| m.provider == "future" && !m.api_key.is_empty())
        {
            all_models
                .iter()
                .find(|m| m.provider == "future" && m.id == "deepseek-v4-pro")
                .map(|m| format!("{}/{}", m.provider, m.id))
        } else {
            None
        };
        preferred
            .or_else(crate::models::get_default_model)
            .or_else(|| {
                all_models
                    .iter()
                    .find(|m| !m.api_key.is_empty() || auth_store.get(&m.provider).is_some())
                    .map(|m| m.id.clone())
            })
            .unwrap_or_default()
    };
    if resolved_model.is_empty() {
        tracing::info!(
            "future-agent: no model configured yet — starting the gRPC server \
             anyway so a client can log in and pick a model. Add an API key via \
             'future auth login' or the desktop app, or configure a provider in \
             ~/.future/agent/models.json."
        );
    }

    // Resolve model config
    let model_config = model_registry.read().resolve(&resolved_model);

    let engine_model = model_config
        .as_ref()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| resolved_model.clone());

    // Resolve base URL: models.json > auth.json baseUrl > built-in default
    let base_url = model_config
        .as_ref()
        .filter(|m| !m.base_url.is_empty())
        .map(|m| m.base_url.clone())
        .or_else(|| {
            model_config
                .as_ref()
                .and_then(|m| auth_store.base_url(&m.provider))
        })
        .unwrap_or_default();
    let base_url = if base_url.is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        base_url
    };

    // Resolve API key from auth.json > model config
    let api_key = model_config
        .as_ref()
        .and_then(|m| auth_store.get(&m.provider))
        .or_else(|| {
            model_config
                .as_ref()
                .filter(|m| !m.api_key.is_empty())
                .map(|m| m.api_key.clone())
        })
        .unwrap_or_default();

    // Default thinking level (clients override per-session).

    // Honor each model's advertised output limit. Models without one retain
    // the existing reasoning/non-reasoning fallbacks.
    let max_tokens = model_config
        .as_ref()
        .map(crate::models::effective_max_tokens);

    // Build engine config from settings and model config. The turn limit is
    // opt-in: an absent or non-positive `maxTurns` is passed through as-is and
    // means unlimited (see `Loop::run_streaming_with_messages`), matching
    // `DEFAULT_MAX_TURNS` — no hidden floor is imposed here.
    let engine_config = EngineConfig {
        cwd: cwd.clone(),
        max_turns: settings.max_turns,
        thinking_level: "high".to_string(),
        compaction_reserve_tokens: settings.compaction_reserve_tokens(),
        compaction_keep_recent_tokens: settings.compaction_keep_recent_tokens(),
        ..EngineConfig::with_defaults()
    };

    // Build engine
    let engine = if let Some(model) = model_config.as_ref() {
        let target = crate::llm::schema::ResolvedModelTarget::from_model(
            model,
            api_key.clone(),
            None,
            max_tokens,
        )?;
        Engine::new_with_target(target, engine_config)?
    } else {
        Engine::new(
            &base_url,
            &api_key,
            &engine_model,
            engine_config,
            None,
            max_tokens,
        )?
    };
    let mut engine = engine.with_tools(crate::coding_tools());

    // Native IPC is the default; auto/empty remain local even through a
    // sidecar's inherited environment. Numeric binds opt into TCP.
    let tcp_bind = config
        .grpc_addr
        .as_deref()
        .filter(|addr| {
            !addr.trim().is_empty()
                && !addr
                    .trim()
                    .eq_ignore_ascii_case(future_rpc::transport::AUTO_ENDPOINT)
        })
        .map(super::config::parse_tcp_bind)
        .transpose()?;
    cancellation.check()?;
    // Discover skills (global user-level dirs only — project/cwd-relative
    // skill dirs are intentionally not scanned).
    let skill_dirs = crate::global_skill_dirs();
    let skills = crate::discover_skills_cached(&skill_dirs);
    let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

    // Load project context
    let agent_content = crate::prompt::load_project_context(&cwd, true).content;

    // Build system prompt
    let today = Local::now().format("%Y-%m-%d").to_string();
    let system_prompt = crate::prompt::build_prompt(&crate::prompt::PromptOptions {
        working_directory: cwd.clone(),
        date: today,
        tools: engine.tools.clone(),
        skills: skills.clone(),
        agent_content,
        ..Default::default()
    });
    engine.agent_loop.verbose = config.verbose;
    engine.agent_loop.system_prompt = system_prompt.clone();
    engine.agent_loop.config.system_prompt = system_prompt;

    cancellation.check()?;
    // Lock order is data root, then endpoint. Reserve before opening SQLite;
    // two different roots must never replace each other's local listener.
    #[cfg(unix)]
    let local_endpoint = if tcp_bind.is_none() {
        // A shared/network-mounted data root can make this reservation slow;
        // name the phase so a long wait reads as progress, not a hang.
        tracing::info!("Agent startup: reserving the local endpoint");
        Some(future_rpc::transport::reserve_local_endpoint().await?)
    } else {
        None
    };
    let manager = Arc::new(Manager::default_for(&cwd));
    manager
        .initialize_cancellable(cancellation)
        .context("initialize Agent SQLite storage")?;
    let imports = manager.import_records()?;
    let skipped = imports
        .iter()
        .filter(|record| record.status == "skipped")
        .count();
    if skipped > 0 {
        tracing::warn!(
            skipped,
            "legacy sessions skipped; use --migrate-sessions for the migration report"
        );
    }
    let approval_gate = crate::rpc::ApprovalGate::default();
    // Template for minting per-session agent loops.  Sessions no longer
    // share one global loop — each hydrated/created session gets an
    // independent copy so concurrent runs, model switches and aborts stay
    // session-local.  The template itself never runs prompts.
    // Set the model_registry on the template so all session loops inherit
    // the cached registry via independent_copy() — avoids ~15% CPU overhead
    // from re-deserialising the model catalog on every prompt.
    let mut template_loop = engine.agent_loop.independent_copy();
    template_loop.model_registry = Some(model_registry.clone());
    let loop_template = Arc::new(template_loop);

    // The agent starts with ZERO sessions.  There is no privileged
    // "default"/"current" session — clients (TUI, GUI, CLI, channels)
    // create or switch to sessions explicitly, and the agent hydrates
    // them on demand.  Settings that used to be applied to the startup
    // default session are applied per-session in cmd_new_session.
    let state = crate::rpc::AppState {
        agent_instance_id: format!("agent_{}", uuid::Uuid::new_v4().simple()),
        control_plane: std::sync::Arc::new(crate::rpc::ControlPlane::default()),
        data_root_id: future_rpc::connection::data_root_id(config.future_home())?,
        sessions: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
        queue_budget: Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        session_manager: manager,
        welcome_version: crate::utils::VERSION.to_string(),
        welcome_cwd: cwd.clone(),
        welcome_skills: Arc::new(parking_lot::RwLock::new(skill_names.clone())),
        welcome_exts: vec![],
        explicit_session: false,
        approval_gate,
        verbose: config.verbose,
        shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        service_work,
        model_registry: model_registry.clone(),
        loop_template,
    };

    cancellation.check()?;
    config.validate_environment()?;

    // Publish the built state to the sync callback registered in `prepare`
    // (before the cold registry build, so an early background fetch cannot
    // fire ahead of it). A fetch that already landed between those two points
    // found the holder empty and is recovered by rebuilding here.
    let early_sync = synced_state.read().is_none();
    *synced_state.write() = Some(state.clone());
    if early_sync {
        let _provider_guard = state
            .control_plane
            .provider_command_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *state.model_registry.write() = crate::models::Registry::new();
        state.reconcile_provider_references();
    }

    tracing::info!("Agent startup: binding the gRPC endpoint");
    let listener = match tcp_bind {
        Some((host, port)) => crate::grpc::AgentListener::bind_tcp(&host, port).await?,
        None => {
            #[cfg(unix)]
            {
                crate::grpc::AgentListener::Local(
                    local_endpoint
                        .expect("local endpoint reserved")
                        .bind()
                        .await?,
                )
            }
            #[cfg(not(unix))]
            {
                crate::grpc::AgentListener::bind_local().await?
            }
        }
    };
    cancellation.check()?;
    Ok((state, listener))
}
