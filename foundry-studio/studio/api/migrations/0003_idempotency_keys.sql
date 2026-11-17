-- Idempotency keys (spec 01 section 3.13, after draft-ietf-httpapi-idempotency-key-header-07): one row per person and
-- key, holding a digest of the first request (method, route and canonical body) and, once it completed, its answer,
-- which a repeat with the same digest is given again. A 5xx answer is never stored, so its retry executes again.
-- Rows are purged 24 hours of wall clock after created_at.

CREATE TABLE idempotency_keys (
    subject        TEXT        NOT NULL REFERENCES people (subject),
    key            TEXT        NOT NULL CHECK (key ~ '^[A-Za-z0-9_-]{1,255}$'),
    method         TEXT        NOT NULL,
    route          TEXT        NOT NULL,
    -- SHA-256 of the method, the route and the canonical body, in hex.
    request_digest TEXT        NOT NULL,
    state          TEXT        NOT NULL CHECK (state IN ('in_flight', 'done')),
    status         INTEGER     CHECK (status BETWEEN 100 AND 599),
    -- The first answer as it was sent, its media type and its bytes, so a repeat is answered byte for byte.
    content_type   TEXT,
    response       BYTEA,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (subject, key),
    CHECK (state = 'in_flight' OR (status IS NOT NULL AND response IS NOT NULL))
);

CREATE INDEX idempotency_keys_by_age ON idempotency_keys (created_at);
