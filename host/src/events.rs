//! Presentation outlet for committed backend changes. Remote transport keeps
//! its own Agent event subscription; attaching a GUI only adds presentation.
use std::sync::OnceLock;

pub enum Event<'a> {
    CloseThreadTerminals(&'a str),
    AgentSettings {
        session_id: &'a str,
        thread_id: &'a str,
        event_type: &'a str,
        data: &'a str,
    },
    ReviewUpdated(&'a str),
    RemoteActivity(&'a str),
    ThreadsUpdated,
    ThreadRuntimeUpdated {
        thread_id: String,
        run_id: String,
        status: String,
        reset_projection: bool,
    },
    ApprovalsUpdated {
        thread_id: &'a str,
        approval_request_id: &'a str,
    },
    Notification {
        name: &'a str,
        payload: serde_json::Value,
    },
}

pub type Sink = for<'a> fn(Event<'a>);
static SINK: OnceLock<Sink> = OnceLock::new();

/// Attach one entry point before starting backend producers. A headless host
/// leaves this unbound; persistence and Remote delivery do not depend on it.
pub fn attach(sink: Sink) -> Result<(), Sink> {
    SINK.set(sink)
}

/// Whether the entry has presentation resources that need synchronous cleanup.
pub(crate) fn has_presentation() -> bool {
    SINK.get().is_some()
}

pub fn publish(event: Event<'_>) {
    publish_to(SINK.get().copied(), event);
}

fn publish_to(sink: Option<Sink>, event: Event<'_>) {
    if let Some(sink) = sink {
        sink(event);
    }
}

pub fn emit_review_updated(thread_id: &str) {
    publish(Event::ReviewUpdated(thread_id));
}
pub fn emit_remote_activity(thread_id: &str) {
    publish(Event::RemoteActivity(thread_id));
}
pub fn emit_threads_updated() {
    publish(Event::ThreadsUpdated);
}
pub fn emit_thread_runtime_updated(
    thread_id: String,
    run_id: String,
    status: String,
    reset_projection: bool,
) {
    publish(Event::ThreadRuntimeUpdated {
        thread_id,
        run_id,
        status,
        reset_projection,
    });
}
pub fn emit_approvals_updated(thread_id: &str, approval_request_id: &str) {
    publish(Event::ApprovalsUpdated {
        thread_id,
        approval_request_id,
    });
}
pub fn notify(name: &str, payload: serde_json::Value) {
    publish(Event::Notification { name, payload });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_presentation_accepts_every_backend_notification() {
        publish_to(None, Event::ReviewUpdated("thread"));
        publish_to(None, Event::RemoteActivity("thread"));
        publish_to(None, Event::ThreadsUpdated);
        publish_to(
            None,
            Event::ThreadRuntimeUpdated {
                thread_id: "thread".into(),
                run_id: "run".into(),
                status: "completed".into(),
                reset_projection: true,
            },
        );
        publish_to(
            None,
            Event::ApprovalsUpdated {
                thread_id: "thread",
                approval_request_id: "approval",
            },
        );
        publish_to(
            None,
            Event::Notification {
                name: "app_settings_changed",
                payload: serde_json::json!({}),
            },
        );
    }

    #[test]
    fn presentation_receives_the_committed_runtime_identity_and_reset() {
        publish_to(
            Some(|event| {
                let Event::ThreadRuntimeUpdated {
                    thread_id,
                    run_id,
                    status,
                    reset_projection,
                } = event
                else {
                    panic!("expected runtime update")
                };
                assert_eq!(thread_id, "thread");
                assert_eq!(run_id, "run");
                assert_eq!(status, "cancelled");
                assert!(reset_projection);
            }),
            Event::ThreadRuntimeUpdated {
                thread_id: "thread".into(),
                run_id: "run".into(),
                status: "cancelled".into(),
                reset_projection: true,
            },
        );
    }
}
