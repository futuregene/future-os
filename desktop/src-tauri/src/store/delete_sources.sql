CREATE TABLE IF NOT EXISTS agent_delete_sources (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES agent_delete_outbox(session_id) ON DELETE CASCADE,
    database_id TEXT NOT NULL CHECK(length(database_id)>0)
);
-- Only existing ownership evidence may qualify an old pending intent.
INSERT INTO agent_delete_sources(session_id,database_id)
SELECT o.session_id,MIN(s.database_id) FROM agent_delete_outbox o
JOIN thread_sources s ON s.session_id=o.session_id
GROUP BY o.session_id HAVING COUNT(DISTINCT s.database_id)=1
ON CONFLICT(session_id) DO NOTHING;
