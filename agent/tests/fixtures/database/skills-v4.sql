-- Frozen skills-only bootstrap DDL from 9d4f8a5e:agent/src/skills/registry.rs.
CREATE TABLE IF NOT EXISTS skills (
    name TEXT PRIMARY KEY NOT NULL,
    version TEXT,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)),
    installed_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS skill_installations (
    location TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('app','global')),
    source TEXT NOT NULL CHECK(source IN ('managed','external')),
    version TEXT,
    package_sha256 TEXT,
    observed_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS skill_installations_name ON skill_installations(name);
CREATE TABLE IF NOT EXISTS skill_operations (
    name TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('install','uninstall')),
    version TEXT,
    phase TEXT NOT NULL CHECK(phase IN ('prepared','replaced')),
    started_at_ms INTEGER NOT NULL
);
