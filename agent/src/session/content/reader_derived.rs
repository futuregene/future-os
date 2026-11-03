//! Disposable derived original-text encodings. SQL remains the sole durable
//! source; cache hits reauthorize the exact entry/field range before serving.
use super::*;
use crate::session::history::fragment_cache::{self, Fragment, FragmentCache};
use std::sync::{Arc, LazyLock};

static CACHE: LazyLock<parking_lot::Mutex<FragmentCache>> = LazyLock::new(Default::default);
static WORK: parking_lot::Mutex<Usage> = parking_lot::Mutex::new(Usage {
    jobs: 0,
    bytes: 0,
    waiting: 0,
});
static AVAILABLE: parking_lot::Condvar = parking_lot::Condvar::new();
const MAX_JOBS: usize = 2;
const MAX_BYTES: usize = 256 * 1024 * 1024;
struct Usage {
    jobs: usize,
    bytes: usize,
    waiting: usize,
}
struct Work {
    bytes: usize,
}
impl Work {
    fn acquire(source: usize) -> Result<Self> {
        // Typed shell facts / string-only JSON repair use no retained generic
        // JSON tree. Charge conservative overlapping raw/decoded/output buffers.
        let bytes = source
            .checked_mul(16)
            .and_then(|bytes| bytes.checked_add(64 * 1024))
            .context("derived read size overflow")?;
        ensure!(
            bytes <= MAX_BYTES,
            "derived content exceeds the read-work memory budget"
        );
        let mut usage = WORK.lock();
        ensure!(
            usage.waiting < 32,
            "derived read-work queue is full; retry the read"
        );
        usage.waiting += 1;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while usage.jobs >= MAX_JOBS || usage.bytes + bytes > MAX_BYTES {
            if AVAILABLE.wait_until(&mut usage, deadline).timed_out()
                && (usage.jobs >= MAX_JOBS || usage.bytes + bytes > MAX_BYTES)
            {
                usage.waiting -= 1;
                anyhow::bail!("derived read-work admission timed out; retry the read");
            }
        }
        usage.waiting -= 1;
        usage.jobs += 1;
        usage.bytes += bytes;
        Ok(Self { bytes })
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        let mut usage = WORK.lock();
        usage.jobs -= 1;
        usage.bytes -= self.bytes;
        AVAILABLE.notify_all();
    }
}

impl Snapshot {
    pub(super) fn derived_text(
        &self,
        store: &SqliteStore,
        entry: &Entry,
        reference: &Reference,
        arguments: bool,
    ) -> Result<Arc<Fragment>> {
        self.verify_reference(store, entry, reference)?;
        let key = self.render_key(
            entry,
            &json!([
                "original-text-v1",
                reference.field,
                reference.range.id,
                reference.range.offset,
                reference.range.length,
                reference.encoding.name(),
                arguments
            ])
            .to_string(),
        );
        if let Some(body) = CACHE.lock().get(&key) {
            return Ok(body);
        }
        let _work = Work::acquire(usize::try_from(reference.range.length)?)?;
        let raw = self.body(store, entry, reference)?;
        let text = match reference.encoding {
            Encoding::ToolArgumentsStringV1 => {
                ensure!(
                    arguments,
                    "repaired argument encoding in a non-argument field"
                );
                let mut value = Value::String(raw);
                future_rpc::tool_arguments::repair_string_v1(&mut value);
                serde_json::to_string(&value)?
            }
            Encoding::ShellSummary => {
                ensure!(!arguments, "shell summary encoding in an argument field");
                let result: future_rpc::shell_result::ShellResult = serde_json::from_str(&raw)?;
                future_rpc::shell_result::model_summary(&result)
            }
            _ => anyhow::bail!("not a derived original-text encoding"),
        };
        let body = fragment_cache::text(&text)?;
        self.verify_reference(store, entry, reference)?;
        CACHE.lock().insert(key, body.clone());
        Ok(body)
    }
}

#[cfg(test)]
#[path = "reader_derived_tests.rs"]
mod tests;
