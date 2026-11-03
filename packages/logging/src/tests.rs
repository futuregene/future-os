use super::*;

fn state(profile: Profile) -> Arc<State> {
    Arc::new(State {
        primary: Component::Desktop,
        profile,
        version: "0.0.2+test".into(),
        instance: "fixture-instance".into(),
        sinks: RwLock::new(HashMap::new()),
    })
}

#[test]
fn distribution_profiles_keep_dependency_noise_out_of_both_channels() {
    for (version, detailed) in [
        ("1.2.3", false),
        ("0.0.2+test", true),
        ("0.0.2+nightly", true),
        ("0.0.0-local", true),
    ] {
        let profile = Profile::for_version(version);
        let subscriber = tracing_subscriber::registry().with(EnvFilter::new(profile.filter()));
        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(
                tracing::enabled!(target: "future_agent::rpc", tracing::Level::DEBUG),
                detailed
            );
            assert!(tracing::enabled!(target: "future_host::application", tracing::Level::INFO));
            assert!(!tracing::enabled!(target: "h2", tracing::Level::INFO));
            assert!(!tracing::enabled!(target: "async_nats::connector", tracing::Level::INFO));
            assert!(tracing::enabled!(target: "rustls", tracing::Level::WARN));
        });
    }
}

#[test]
fn embedded_agent_and_desktop_have_independent_jsonl_files_and_flush_on_retirement() {
    let directory = tempfile::tempdir().unwrap();
    let state = state(Profile::Detailed);
    let desktop = Component::Desktop.path(directory.path());
    let agent = Component::Agent.path(directory.path());
    let desktop_guard = open_with_state(state.clone(), Component::Desktop, &desktop, 0).unwrap();
    let agent_guard = open_with_state(state.clone(), Component::Agent, &agent, 0).unwrap();
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .event_format(format::EventFormat::new(state.clone()))
        .with_writer(Router(state));
    tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
        let span = tracing::info_span!("operation", run_id = "run-fixture");
        let _entered = span.enter();
        tracing::debug!(target: "future_agent::rpc", event = "agent_fixture", count = 7, message = "escaped\n\"value\"");
        tracing::warn!(target: "futureos_lib::commands::history_transfer", event = "desktop_fixture", offset = 65536);
    });
    drop(agent_guard);
    drop(desktop_guard);
    for (path, component, own_event, other_event) in [
        (agent, "agent", "agent_fixture", "desktop_fixture"),
        (desktop, "desktop", "desktop_fixture", "agent_fixture"),
    ] {
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(contents.contains(own_event), "{contents}");
        assert!(!contents.contains(other_event), "{contents}");
        for line in contents.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["component"], component);
            assert_eq!(record["process_instance_id"], "fixture-instance");
            assert!(record["pid"].is_number());
            assert!(record["timestamp"].is_string());
        }
    }
}

#[test]
fn overlapping_process_owners_never_rotate_an_active_component_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = Component::Tui.path(directory.path());
    let first = open_with_state(state(Profile::Detailed), Component::Tui, &path, 0).unwrap();
    let second = open_with_state(state(Profile::Detailed), Component::Tui, &path, 0).unwrap();
    assert!(path.is_file());
    assert!(path
        .with_file_name(format!("tui.{}.log", std::process::id()))
        .is_file());
    assert!(!path.with_extension("log.1").exists());
    drop(second);
    drop(first);
}
