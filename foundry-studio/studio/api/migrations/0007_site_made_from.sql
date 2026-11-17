-- Spec 05 section 3's `site_made_from`, the record of each act that made or updated a site from a template, created
-- ahead of Create site with only what `GET /api/sites` reads (spec 01 section 3.6): a site whose act was abandoned
-- after a stage failed stays off the list, since its root may hold a stage's prologue though nothing usable is there,
-- until a later act's stage moves the root past where the cleanup left it. Create site and update add the template,
-- knob, changeset, draft, approval and author columns with the code that writes them. Every `*_at` column is wall
-- clock.

CREATE TABLE site_made_from (
    -- A ULID, minted at random.
    id               TEXT        PRIMARY KEY,
    -- The site's root scope path, customer/{c}/site/{s}.
    site             TEXT        NOT NULL,
    state            TEXT        NOT NULL
                     CHECK (state IN ('creating', 'updating', 'made', 'updated', 'abandoned', 'partial')),
    -- The root's head once an abandoned act's cleanup ended: the root's cleanup changeset, or the head it stood at
    -- when the cleanup wrote nothing there.
    cleaned_root_seq BIGINT      CHECK (cleaned_root_seq >= 0),
    at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (state <> 'abandoned' OR cleaned_root_seq IS NOT NULL)
);

CREATE INDEX site_made_from_abandoned ON site_made_from (site) WHERE state = 'abandoned';
