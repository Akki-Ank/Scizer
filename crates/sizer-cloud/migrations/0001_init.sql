-- API keys: per-caller auth, assigned out of band (no signup flow -- see
-- docs/ARCHITECTURE.md "Cloud mode is opt-in, not core"). Only the SHA-256
-- hash is stored; the raw key is shown once at creation time (sizer-cloud-keygen).
CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_hash TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    quota_bytes_per_day BIGINT NOT NULL DEFAULT 5368709120,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at TIMESTAMPTZ
);

-- Jobs: one row per compress/decompress/recompress request. `status` is a
-- plain TEXT + CHECK rather than a Postgres ENUM so adding a new status
-- later is a migration that only touches this constraint, not an
-- ALTER TYPE across every dependent column.
CREATE TABLE jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    api_key_id UUID NOT NULL REFERENCES api_keys(id),
    codec_domain TEXT NOT NULL,
    codec_name TEXT NOT NULL,
    operation TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'queued', 'running', 'succeeded', 'failed', 'expired')),
    input_key TEXT NOT NULL,
    output_key TEXT,
    input_bytes BIGINT,
    output_bytes BIGINT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_jobs_api_key_id ON jobs (api_key_id);
CREATE INDEX idx_jobs_status ON jobs (status);
CREATE INDEX idx_jobs_expires_at ON jobs (expires_at);
