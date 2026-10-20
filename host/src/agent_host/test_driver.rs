//! Runtime owner for the host test process; no production fallback.
pub(super) fn install() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| super::configure_driver(drive).expect("test driver installed once"));
}

fn drive(host: &super::AgentHost, configured: &str) -> Result<(), crate::Error> {
    let probe = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let selected = probe.block_on(host.select(configured));
    drop(probe);
    host.run_selected(selected?, |prepared| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("agent-runtime")
            .worker_threads(2)
            .max_blocking_threads(32)
            .thread_stack_size(2 * 1024 * 1024)
            .build()?;
        let result = runtime.block_on(host.serve(prepared));
        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
        result
    })
}
