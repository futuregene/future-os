//! No WebView notifications in the server build. Agent persistence and mobile
//! event mirroring remain active in their shared modules.
pub(crate) fn emit_review_updated(_thread_id: &str) {}
pub(crate) fn emit_remote_activity(_thread_id: &str) {}
pub(crate) fn emit_threads_updated() {}
pub(crate) fn emit_thread_runtime_updated(
    _thread_id: String,
    _run_id: String,
    _status: String,
    _reset_projection: bool,
) {
}
pub(crate) fn emit_approvals_updated(_thread_id: &str, _approval_request_id: &str) {}
