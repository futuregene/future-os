CREATE TABLE IF NOT EXISTS approval_assessments (
    id TEXT PRIMARY KEY,
    approval_request_id TEXT NOT NULL REFERENCES approval_requests(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    tool_call_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('approved','rejected','review_error','review_uncertain','cancelled','stale_request')),
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS approval_assessments_run ON approval_assessments(run_id, created_at);
