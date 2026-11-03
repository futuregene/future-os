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
    /// A scheduled task was created, changed, deleted or asked to run — by a
    /// phone over Remote, or by any other process sharing this home. The
    /// conversation list did not change, so this is its own signal: a panel
    /// listening for `ThreadsUpdated` would re-list every conversation to learn
    /// that one task's switch moved.
    TasksUpdated,
    /// One conversation's activity stamp advanced. Carries the value so a GUI
    /// can patch that row instead of re-listing every conversation.
    ThreadActivity {
        thread_id: &'a str,
        last_message_at: i64,
    },
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
    /// The desktop-side `takeover` moved control to another device. The
    /// windowed desktop activates its main window on receipt; a headless host
    /// has no presentation and ignores it.
    ControlClaimed,
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

/// Emit only when a presentation sink is attached. The headless host never
/// installs one, so this is a compile-time no-op there.
pub fn publish_to_gui(event: Event<'_>) {
    if SINK.get().is_some() {
        publish(event);
    }
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
pub fn emit_tasks_updated() {
    publish(Event::TasksUpdated);
}

/// The desktop-side `takeover` moved control; the windowed desktop presents
/// its main window. Headless runs no-op (no presentation sink).
pub fn emit_control_claimed() {
    publish_to_gui(Event::ControlClaimed);
}
pub fn emit_thread_activity(thread_id: &str, last_message_at: i64) {
    publish(Event::ThreadActivity {
        thread_id,
        last_message_at,
    });
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
        publish_to(None, Event::TasksUpdated);
        publish_to(None, Event::ControlClaimed);
        publish_to(
            None,
            Event::ThreadActivity {
                thread_id: "thread",
                last_message_at: 1_000,
            },
        );
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
    fn gui_only_events_are_dropped_without_a_sink() {
        // Headless host: no sink, ControlClaimed must not panic or be forwarded.
        publish_to_gui(Event::ControlClaimed);
    }

    /// The producer side of that gate: raising a control claim goes through
    /// `publish_to_gui`, so a headless host drops it *at the gate* instead of
    /// reaching a presentation that does not exist — and raising one is never
    /// what attaches a sink (`events::attach` belongs to the entry point, and
    /// it is process-global and one-shot, so whether a sink exists here depends
    /// on which other test ran first: the invariant that holds either way is
    /// that this call does not change it).
    #[test]
    fn raising_a_control_claim_does_not_attach_a_presentation() {
        let attached = has_presentation();
        emit_control_claimed();
        assert_eq!(
            has_presentation(),
            attached,
            "raising the claim must not attach a presentation sink"
        );
    }

    #[test]
    fn presentation_receives_the_activity_stamp_it_must_patch() {
        publish_to(
            Some(|event| {
                let Event::ThreadActivity {
                    thread_id,
                    last_message_at,
                } = event
                else {
                    panic!("expected a thread activity update")
                };
                assert_eq!(thread_id, "thread");
                assert_eq!(last_message_at, 1_700_000_000_000);
            }),
            Event::ThreadActivity {
                thread_id: "thread",
                last_message_at: 1_700_000_000_000,
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
