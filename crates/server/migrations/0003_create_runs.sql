CREATE TYPE run_status AS ENUM ('pending', 'running', 'completed', 'cancelled', 'failed');

-- Exactly one run per campaign (POST /campaigns creates both in one
-- transaction, so `Campaign.runId` in the API is never null) — the UNIQUE
-- constraint makes that invariant enforceable rather than just conventional.
CREATE TABLE runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    campaign_id   UUID NOT NULL UNIQUE REFERENCES campaigns(id) ON DELETE CASCADE,
    status        run_status NOT NULL DEFAULT 'pending',
    iterations    BIGINT NOT NULL DEFAULT 0,
    started_at    TIMESTAMPTZ,
    finished_at   TIMESTAMPTZ,
    worker        TEXT,
    log_tail      TEXT
);
