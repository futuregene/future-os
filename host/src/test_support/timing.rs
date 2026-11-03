//! Opt-in fixture timings, written outside HOME so fixture resets cannot erase them.
use crate::console;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

pub(crate) struct FixtureTiming(Option<Recording>);

struct Recording {
    path: PathBuf,
    fixture: &'static str,
    label: String,
    test: String,
    started: Instant,
    phase_started: Instant,
    phases_ms: BTreeMap<&'static str, f64>,
}

impl FixtureTiming {
    pub(crate) fn start(fixture: &'static str, label: &str) -> Self {
        let recording = std::env::var_os("FUTURE_HOST_FIXTURE_TIMINGS")
            .filter(|path| !path.is_empty())
            .map(|path| {
                let now = Instant::now();
                Recording {
                    path: PathBuf::from(path),
                    fixture,
                    label: label.to_string(),
                    test: std::thread::current()
                        .name()
                        .unwrap_or("unnamed")
                        .to_string(),
                    started: now,
                    phase_started: now,
                    phases_ms: BTreeMap::new(),
                }
            });
        Self(recording)
    }

    pub(crate) fn mark(&mut self, phase: &'static str) {
        if let Some(recording) = &mut self.0 {
            let now = Instant::now();
            recording.phases_ms.insert(
                phase,
                now.duration_since(recording.phase_started).as_secs_f64() * 1_000.0,
            );
            recording.phase_started = now;
        }
    }

    pub(crate) fn finish(&mut self) {
        self.mark("restore");
        let Some(recording) = self.0.take() else {
            return;
        };
        let wall_ms = recording.started.elapsed().as_secs_f64() * 1_000.0;
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .open(&recording.path)?;
            // Re-executed test children can share the same artifact. Keep one
            // complete JSON row per fixture even across processes.
            fs2::FileExt::lock_exclusive(&file)?;
            let row = serde_json::json!({
                "schema": 1,
                "pid": std::process::id(),
                "fixture": recording.fixture,
                "label": recording.label,
                "test": recording.test,
                "phases_ms": recording.phases_ms,
                "wall_ms": wall_ms,
            });
            writeln!(file, "{row}")
        })();
        if let Err(error) = result {
            console::eprintln!("Host fixture timing could not be recorded: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The opt-in artifact is the only thing this module exists for, and it is
    /// off unless `FUTURE_HOST_FIXTURE_TIMINGS` names a file — so every line
    /// below the `start` guard runs only under that switch. Held here so the
    /// timing rows a fixture run reports cannot rot unnoticed.
    ///
    /// `FUTURE_HOST_FIXTURE_TIMINGS` is process-global, so this test owns it for
    /// its own duration and puts it back; a concurrent fixture that also records
    /// only adds its own rows to the same file, which is what the format is for.
    #[test]
    fn the_artifact_is_written_only_under_the_switch() {
        let dir = std::env::temp_dir().join(format!(
            "future-host-timings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let artifact = dir.join("fixture-timings.jsonl");
        let previous = std::env::var_os("FUTURE_HOST_FIXTURE_TIMINGS");

        // Off: nothing is recorded and nothing is written.
        std::env::remove_var("FUTURE_HOST_FIXTURE_TIMINGS");
        let mut quiet = FixtureTiming::start("Quiet", "off");
        quiet.mark("load");
        quiet.finish();
        assert!(!artifact.exists(), "the switch is what writes the file");

        // A set-but-empty value is the same as unset (a launch script that passes
        // an empty default must not create a file named "").
        std::env::set_var("FUTURE_HOST_FIXTURE_TIMINGS", "");
        let mut empty = FixtureTiming::start("Quiet", "empty");
        empty.finish();

        std::env::set_var("FUTURE_HOST_FIXTURE_TIMINGS", &artifact);
        let mut timing = FixtureTiming::start("StoreInit", "initialize_app_store");
        timing.mark("open");
        timing.mark("migrate");
        timing.finish();
        // A second `finish` (a fixture that calls it from both success and the
        // error path) records nothing more.
        timing.finish();

        let text = std::fs::read_to_string(&artifact).expect("the artifact");
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("one JSON row per line")
            })
            .filter(|row| row["fixture"] == serde_json::json!("StoreInit"))
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "one row per fixture, not one per phase: {text}"
        );
        assert_eq!(rows[0]["schema"], serde_json::json!(1));
        assert_eq!(rows[0]["label"], serde_json::json!("initialize_app_store"));
        assert_eq!(rows[0]["pid"], serde_json::json!(std::process::id()));
        assert!(
            rows[0]["test"]
                .as_str()
                .is_some_and(|name| name.contains("the_artifact_is_written_only_under_the_switch")),
            "the row names the test it came from: {text}"
        );
        let phases = rows[0]["phases_ms"].as_object().expect("phases");
        assert!(phases.contains_key("open"), "{text}");
        assert!(phases.contains_key("migrate"), "{text}");
        assert!(
            phases.contains_key("restore"),
            "`finish` marks the restore phase itself: {text}"
        );
        assert!(
            rows[0]["wall_ms"].as_f64().is_some_and(|ms| ms >= 0.0),
            "{text}"
        );

        // A path whose directory does not exist is reported, not propagated: a
        // fixture must not fail because the artifact could not be written.
        let broken = dir.join("absent").join("timings.jsonl");
        std::env::set_var("FUTURE_HOST_FIXTURE_TIMINGS", &broken);
        let mut unwritable = FixtureTiming::start("StoreInit", "nowhere");
        unwritable.finish();
        assert!(!broken.exists());

        match previous {
            Some(value) => std::env::set_var("FUTURE_HOST_FIXTURE_TIMINGS", value),
            None => std::env::remove_var("FUTURE_HOST_FIXTURE_TIMINGS"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
