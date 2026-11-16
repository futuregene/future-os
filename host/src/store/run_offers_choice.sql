-- Contract 9 adds the settlement flag introduced independently of archive
-- cleanup. Older databases receive it without rewriting any run history.
ALTER TABLE runs ADD COLUMN offers_choice INTEGER NOT NULL DEFAULT 0;
