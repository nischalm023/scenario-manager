-- Agent Studio's people: everyone the login gate has let through, keyed by the gate's stable subject (Keycloak's
-- `sub`); spec 01 section 3.3. The identity layer upserts a row on every request it reads, so email and name are the
-- gate's latest word. Every `*_at` column is wall clock.

CREATE TABLE people (
    subject       TEXT        PRIMARY KEY,
    email         TEXT        NOT NULL,
    name          TEXT        NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The runner image a developer chose, by digest, or NULL for the default (spec 01 section 3.16). Its reference
    -- to runner_images arrives with the registry's own migration.
    runner_image  TEXT
);
