//! Opt-in fixture timings, written outside HOME so fixture resets cannot erase them.
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
            eprintln!("Host fixture timing could not be recorded: {error}");
        }
    }
}
