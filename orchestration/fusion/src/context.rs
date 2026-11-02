//! Request-local context admission. Never silently truncate discussion material.
use anyhow::{ensure, Result};
pub const CONTEXT_POLICY_VERSION: u32 = 2;
pub const CONTEXT_MARGIN: usize = 8192;
pub fn guard_context(serialized_bytes: usize, window: usize, output_tokens: usize) -> Result<()> {
    ensure!(window > 0 && output_tokens > 0 && serialized_bytes.saturating_add(output_tokens).saturating_add(CONTEXT_MARGIN) <= window,
        "Fusion context capacity exceeded; request was not sent (input bytes={serialized_bytes}, output reserve={output_tokens}, window={window})");
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn capacity_guard_reserves_output_and_protocol_overhead() {
        assert!(super::guard_context(1000, 32768, 8000).is_ok());
        assert!(super::guard_context(30000, 32768, 8000).is_err());
        assert!(super::guard_context(0, 0, 1).is_err());
        assert!(super::guard_context(usize::MAX, 32768, 8000).is_err());
    }
}
