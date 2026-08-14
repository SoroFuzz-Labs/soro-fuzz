-- Requires Postgres 13+ for the built-in gen_random_uuid() (see
-- docker-compose.yml, which pins a 16.x image).
--
-- Note this enum's initial state is 'queued', not 'pending' — that's
-- run_status's word for the same moment (see 0003). Two different enums,
-- two different vocabularies, straight from docs/api-contract.md; don't
-- "fix" them to match each other.
CREATE TYPE campaign_status AS ENUM ('queued', 'running', 'completed', 'cancelled', 'failed');

CREATE TABLE campaigns (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_id         TEXT NOT NULL REFERENCES targets(id),
    name              TEXT NOT NULL,
    invariant_ids     JSONB NOT NULL DEFAULT '[]',
    time_budget_secs  INTEGER NOT NULL,
    -- Mirrors the status of this campaign's one run (see 0003); the worker
    -- updates both together rather than the API deriving one from the other.
    status            campaign_status NOT NULL DEFAULT 'queued',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX campaigns_status_idx ON campaigns (status);
CREATE INDEX campaigns_target_id_idx ON campaigns (target_id);
