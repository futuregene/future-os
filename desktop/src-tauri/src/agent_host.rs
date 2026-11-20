//! Entry-owned native runtime driver for the single shared Agent owner.
#[cfg(feature = "gui")]
pub(crate) use future_host::agent_host::Phase;

pub(crate) fn host() -> &'static future_host::agent_host::AgentHost {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        future_host::agent_host::configure_driver(drive)
            .expect("entry installs its Agent driver once");
    });
    future_host::agent_host::host()
}

fn drive(
    host: &future_host::agent_host::AgentHost,
    configured: &str,
) -> Result<(), crate::AppError> {
    let probe = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let selected = probe.block_on(host.select(configured));
    drop(probe);
    host.run_selected(selected?, |prepared| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("agent-runtime")
            // Not a small number: this pool serves the Agent's RPC *and* every
            // run's model stream, so an undersized one turns a loaded Agent into
            // an unresponsive one. See `AGENT_RUNTIME_WORKER_THREADS`.
            .worker_threads(future_host::agent_host::AGENT_RUNTIME_WORKER_THREADS)
            .max_blocking_threads(32)
            .thread_stack_size(2 * 1024 * 1024)
            .build()?;
        let result = runtime.block_on(host.serve(prepared));
        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
        result
    })
}
