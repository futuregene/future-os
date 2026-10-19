-- Frozen skills-only bootstrap DDL from 9a6ec1af:agent/src/skills/registry.rs.
CREATE TABLE IF NOT EXISTS skills (
    name TEXT PRIMARY KEY NOT NULL,
    version TEXT,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)),
    installed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);
