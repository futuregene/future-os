CREATE INDEX IF NOT EXISTS idx_review_snapshots_thread ON review_snapshots(thread_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_run ON artifacts(run_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_thread ON artifacts(thread_id);
CREATE INDEX IF NOT EXISTS idx_approval_assessments_request ON approval_assessments(approval_request_id);
CREATE INDEX IF NOT EXISTS idx_threads_parent_session ON threads(parent_session_id);
CREATE INDEX IF NOT EXISTS idx_threads_effective_session ON threads(COALESCE(NULLIF(TRIM(agent_session_id), ''), id));
CREATE INDEX IF NOT EXISTS idx_agent_delete_outbox_pending ON agent_delete_outbox(requested_at, session_id);
