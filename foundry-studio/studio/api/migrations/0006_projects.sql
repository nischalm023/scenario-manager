-- Projects (spec 01 section 3.3): what a conversation belongs to, and later a scenario. Brief I1.04 pulls the table
-- forward from task 24, which keeps its routes, because a conversation needs one; with STUDIO_ENV=local the service
-- makes a default project at start when none is the default. Every `*_at` column is wall clock.

CREATE TABLE projects (
    -- A ULID, minted at random.
    id              TEXT        PRIMARY KEY,
    name            TEXT        NOT NULL CHECK (name <> ''),
    -- A WPS customer slug, when the project is one customer's.
    customer        TEXT        CHECK (customer ~ '^[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?$'),
    -- A Foundry project's ObjectId, which a person enters and nothing mints (spec 03 section 3.7).
    foundry_project TEXT,
    -- Who made it; NULL for the default project the service makes at start.
    created_by      TEXT        REFERENCES people (subject),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    is_default      BOOLEAN     NOT NULL DEFAULT false
);

CREATE UNIQUE INDEX projects_one_default ON projects (is_default) WHERE is_default;

-- Migration 0002 left this reference for the table to arrive.
ALTER TABLE conversations ADD CONSTRAINT conversations_project_fkey FOREIGN KEY (project) REFERENCES projects (id);

-- A site's conversations newest first, by the first number each holds (spec 01 section 3.6's list).
CREATE INDEX conversations_by_site ON conversations (site, first_seq DESC) WHERE site IS NOT NULL;
