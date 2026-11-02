use super::State;
use std::{fmt, sync::Arc};
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    fmt::{
        format::{Format, Json, Writer},
        FmtContext, FormatEvent, FormatFields,
    },
    registry::LookupSpan,
};

pub(super) struct EventFormat {
    json: Format<Json>,
    state: Arc<State>,
}

impl EventFormat {
    pub(super) fn new(state: Arc<State>) -> Self {
        Self {
            json: Format::default().json(),
            state,
        }
    }
}

impl<S, N> FormatEvent<S, N> for EventFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut json = String::new();
        self.json.format_event(ctx, Writer::new(&mut json), event)?;
        let common = serde_json::json!({
            "component": self.state.component(event.metadata()).name(),
            "pid": std::process::id(),
            "process_instance_id": self.state.instance,
            "version": self.state.version,
        })
        .to_string();
        // Both fragments are encoded by standard JSON serializers. Keep the
        // subscriber's structured fields/spans intact, including JSON escaping.
        writer.write_str(common.trim_end_matches('}'))?;
        writer.write_char(',')?;
        writer.write_str(json.strip_prefix('{').ok_or(fmt::Error)?)
    }
}
