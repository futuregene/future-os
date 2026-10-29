//! Usage accounting for one model request.
//!
//! A request reports its usage more than once — the Future gateway sends a
//! coarse synthesized frame and then the provider's real one, which also
//! arrives nested in the finish chunk and again at top level. Counting per
//! chunk is what once made a single request look like two, so the counters move
//! once, from the merged total, when the stream ends.

use super::super::Loop;
impl Loop {
    /// Fold one usage block from the model stream into this call's total.
    ///
    /// Nothing is added to the session counters here. A single request can
    /// report its usage more than once — the Future gateway sends a coarse
    /// synthesized frame and then the provider's real one (which also reaches
    /// us twice: nested in the finish chunk, and again at top level in the
    /// trailing empty-choices chunk) — so counting per block is what made one
    /// request look like two. The call's counters are updated once, from the
    /// merged total, when the stream ends (`charge_request_usage`).
    pub(super) fn process_usage_event(
        &self,
        u: &crate::types::Usage,
        total_usage: &mut Option<crate::types::Usage>,
    ) {
        use std::sync::atomic::Ordering;
        // The context size is a point-in-time gauge, not a cumulative counter:
        // the latest block that reports a prompt (here, the real one, since a
        // coarse frame's prompt is an estimate) describes what this call sent.
        self.last_prompt_tokens
            .store(u.prompt_tokens + u.completion_tokens, Ordering::Relaxed);
        match total_usage {
            Some(total) => total.merge_later_chunk(u),
            None => *total_usage = Some(u.clone()),
        }
    }

    /// Charge one model request: its tokens once, its cost once, both taken
    /// from the call's final merged usage rather than from each progressive
    /// chunk. This is the only place a call's tokens reach the session
    /// counters, mirroring what `charge_cost_split` already did for cost.
    pub(super) fn charge_request_usage(&self, u: &crate::types::Usage) {
        add_request_usage(
            &self.cumulative_input_tokens,
            &self.cumulative_output_tokens,
            &self.cumulative_cache_read_tokens,
            &self.cumulative_cache_write_tokens,
            u,
        );
        let cost = u.credit_cost.unwrap_or_else(|| self.estimate_usage_cost(u));
        if cost > 0.0 {
            *self.cumulative_cost.lock() += cost;
        }
        self.charge_cost_split(u);
    }

    /// Estimate the cost of a single request's final usage when the upstream
    /// API did not report an authoritative `credit_cost` (most non-Future
    /// providers). Resolves the model's per-1M-token prices from the shared
    /// registry and returns zero when the model is unknown or unpriced.
    pub(super) fn estimate_usage_cost(&self, usage: &crate::types::Usage) -> f64 {
        let model_ref = if self.model_ref.is_empty() {
            self.model.as_str()
        } else {
            self.model_ref.as_str()
        };
        estimate_usage_cost_with(self.model_registry.as_ref(), model_ref, usage)
    }

    /// Add this request's per-category amounts to the session's running split.
    ///
    /// Priced with the rates of the model that served *this* request, which is
    /// what keeps a session that switches models honest: the tokens of each
    /// request are billed at the price in force when it ran. The provider's own
    /// `credit_cost` (when present) is a single number with no breakdown, so the
    /// split is always our price-list estimate and the total stays authoritative.
    /// An unpriced model contributes nothing, leaving that request out of the split.
    pub(super) fn charge_cost_split(&self, usage: &crate::types::Usage) {
        let model_ref = if self.model_ref.is_empty() {
            self.model.as_str()
        } else {
            self.model_ref.as_str()
        };
        let parts = usage_cost_split_with(self.model_registry.as_ref(), model_ref, usage);
        if parts.iter().any(|amount| *amount > 0.0) {
            self.cumulative_cost_split.lock().add(parts);
        }
    }
}

/// Add one model request's usage to a session's cumulative counters.
///
/// `usage` must be the request's *final* usage — the block left after folding
/// its progressive chunks with [`crate::types::Usage::merge_later_chunk`] — not
/// one chunk on its own: the counters are the session's record of how much one
/// request cost, and a request that reports its usage twice (or a coarse
/// estimate before the real one) must still add up to itself.
///
/// Every caller therefore charges exactly one call: the run loop once per
/// stream, the compaction summary observer once per attempt.
pub(crate) fn add_request_usage(
    input: &std::sync::atomic::AtomicI64,
    output: &std::sync::atomic::AtomicI64,
    cache_read: &std::sync::atomic::AtomicI64,
    cache_write: &std::sync::atomic::AtomicI64,
    usage: &crate::types::Usage,
) {
    use std::sync::atomic::Ordering;
    input.fetch_add(usage.prompt_tokens, Ordering::Relaxed);
    output.fetch_add(usage.completion_tokens, Ordering::Relaxed);
    if let Some(read) = usage.cache_read_tokens {
        cache_read.fetch_add(read, Ordering::Relaxed);
    }
    if let Some(write) = usage.cache_write_tokens {
        cache_write.fetch_add(write, Ordering::Relaxed);
    }
    // `prompt_tokens` already contains the cached subset, so no single request
    // can be served more cached bytes than it had prompt. A block that says
    // otherwise mixed a coarse count with a detailed one, or was counted twice:
    // report it rather than inflating the cache figures into a rate above 100%.
    if usage.cached_tokens_exceed_prompt() {
        tracing::warn!(
            prompt_tokens = usage.prompt_tokens,
            cache_read = usage.cache_read_tokens.unwrap_or(0),
            cache_write = usage.cache_write_tokens.unwrap_or(0),
            "usage reports more cached bytes than prompt tokens for one request"
        );
        debug_assert!(
            !usage.cached_tokens_exceed_prompt(),
            "cache_read + cache_write exceeds prompt_tokens for one request: {usage:?}"
        );
    }
}

/// Estimate a request's cost from the model's per-1M-token prices, for providers
/// that do not report an authoritative `credit_cost`. Shared so auxiliary requests
/// (a compaction summary, for example) are priced exactly like a model call.
pub(crate) fn estimate_usage_cost_with(
    registry: Option<&std::sync::Arc<parking_lot::RwLock<crate::models::Registry>>>,
    model_ref: &str,
    usage: &crate::types::Usage,
) -> f64 {
    let Some(model) = registry.and_then(|registry| registry.read().resolve(model_ref)) else {
        return 0.0;
    };
    model.cost.estimate(
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.cache_read_tokens.unwrap_or(0),
        usage.cache_write_tokens.unwrap_or(0),
    )
}

/// The per-category amounts (in yuan) of one request, priced with `model_ref`'s
/// rates; all-zero when the model is unknown or unpriced. The counterpart of
/// [`estimate_usage_cost_with`], used to accumulate a session's split request by
/// request rather than re-pricing its totals at whatever model it runs today.
pub(crate) fn usage_cost_split_with(
    registry: Option<&std::sync::Arc<parking_lot::RwLock<crate::models::Registry>>>,
    model_ref: &str,
    usage: &crate::types::Usage,
) -> [f64; 4] {
    let Some(model) = registry.and_then(|registry| registry.read().resolve(model_ref)) else {
        return [0.0; 4];
    };
    model.cost.parts(
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.cache_read_tokens.unwrap_or(0),
        usage.cache_write_tokens.unwrap_or(0),
    )
}
