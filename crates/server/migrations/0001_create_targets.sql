-- Registered fuzz targets, synced from the contracts repo's manifest by the
-- targets loader (see src/targets.rs) rather than hand-edited.
CREATE TABLE targets (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    contract_name     TEXT NOT NULL,
    methods           JSONB NOT NULL DEFAULT '[]',
    available_invariants JSONB NOT NULL DEFAULT '[]',
    fuzz_target_name  TEXT NOT NULL,
    known_buggy       BOOLEAN NOT NULL DEFAULT FALSE,
    added_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
