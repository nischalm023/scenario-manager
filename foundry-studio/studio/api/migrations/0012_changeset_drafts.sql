-- Spec 01 section 3.11's draft changesets, in the first cut a person's own (brief I1.11): an unapplied edit to a site,
-- built against the cut its person read and applied against that cut (ruling D43). A person holds at most one open
-- draft per site, shared by every panel of theirs, and only its creator changes, applies or discards it. Its events
-- go to the site's live conversation, found when each is written. The agent's drafts, a turn's, join this table with
-- task 3. Every `*_at` column is wall clock.

CREATE TABLE changeset_drafts (
    -- A ULID, minted at random.
    id           TEXT        PRIMARY KEY,
    -- The site's root scope, customer/{c}/site/{s}.
    site         TEXT        NOT NULL,
    created_by   TEXT        NOT NULL REFERENCES people (subject),
    -- Spec 04's UnitCut the draft was built on: every scope of the site at the changeset its person read.
    cut          JSONB       NOT NULL,
    -- `applying` while a press has claimed the revision it checked and its one changeset is being written.
    state        TEXT        NOT NULL
                             CHECK (state IN ('drafting', 'ready', 'applying', 'applied', 'discarded', 'stale')),
    -- Why a stale draft can no longer be applied (`head-moved`), with the scopes that moved.
    stale_reason JSONB,
    summary      TEXT        NOT NULL,
    -- The fold of every operation added, in order; its digest is SHA-256 of their canonical JSON.
    ops          JSONB       NOT NULL,
    ops_digest   TEXT        NOT NULL,
    revision     INTEGER     NOT NULL CHECK (revision >= 1),
    -- The last check's problems on the complete would-be site, or world-proto-server's refusal of the last press.
    problems     JSONB       NOT NULL,
    -- The press that claims it, and the one changeset body built at the claim from the checked revision, which every
    -- sending under the press's operation identifier sends unchanged; a definitive refusal releases both.
    approval     TEXT        REFERENCES approvals (id),
    apply_body   JSONB,
    -- Each written scope's new changeset once it is applied.
    applied_seqs JSONB,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((state = 'applied') = (applied_seqs IS NOT NULL)),
    CHECK ((state = 'stale') = (stale_reason IS NOT NULL)),
    CHECK (state <> 'applying' OR (approval IS NOT NULL AND apply_body IS NOT NULL))
);

-- One open draft per person and site, one being applied included.
CREATE UNIQUE INDEX changeset_drafts_one_open_per_person ON changeset_drafts (site, created_by)
    WHERE state IN ('drafting', 'ready', 'applying');

-- The drafts a start resumes: those a stopped process left claimed.
CREATE INDEX changeset_drafts_applying ON changeset_drafts (id) WHERE state = 'applying';
