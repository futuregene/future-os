-- Contract 7 adds the run's own record that its reply closed by offering the
-- reader a choice. The column reached `SCHEMA` before this contract existed, so
-- every database written until then — including the released contract 6 — is at
-- version 6 without it, while a fresh install (or a build that shipped the
-- column early) already has it; the runner skips this ALTER in the latter case
-- and keeps whatever marks were written.
ALTER TABLE runs ADD COLUMN offers_choice INTEGER NOT NULL DEFAULT 0;
