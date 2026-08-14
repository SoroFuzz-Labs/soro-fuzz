-- Backend-internal; not part of the frontend-facing API. A worker claims a
-- row by atomically setting claimed_by/claimed_at (see store::claim_job),
-- which is what makes a re-claimed/retried job idempotent rather than a
-- source of duplicate findings.
CREATE TABLE job_queue (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id  UUID NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    run_id       UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    claimed_by   TEXT,
    claimed_at   TIMESTAMPTZ,
    attempts     INTEGER NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Partial index over unclaimed jobs so the worker's claim query (WHERE
-- claimed_by IS NULL ORDER BY created_at) doesn't scan claimed history.
CREATE INDEX job_queue_unclaimed_idx ON job_queue (created_at) WHERE claimed_by IS NULL;
