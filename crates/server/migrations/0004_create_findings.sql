-- `sequence` stores the decoded, already-display-shaped FindingStep[] (see
-- docs/api-contract.md) — index, method, args, authorizedBy, advanceTimeSecs,
-- outcome per step — so the API layer can deserialize it directly instead of
-- re-decoding raw command bytes on every read.
CREATE TABLE findings (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id                      UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    invariant                   TEXT NOT NULL,
    message                     TEXT NOT NULL,
    step_index                  INTEGER NOT NULL,
    sequence                    JSONB NOT NULL,
    shrunk_input                BYTEA,
    fuzz_target_name            TEXT NOT NULL,
    artifact_path               TEXT NOT NULL,
    requires_sanitizer_thread   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX findings_run_id_idx ON findings (run_id);
