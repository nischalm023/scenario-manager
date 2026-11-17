-- Spec 05 section 3.7's template tables, the rest of its made-from record, and spec 01 section 3.3's approvals, which
-- the agent loop (task 3) has not made yet and Create site needs, since the person's press is their approval (spec 01
-- section 3.10, rule 6). A template draft holds knob values over one template read at one changeset per scope; its
-- evaluations are numbered without gaps under the draft's row lock; the ids it gives each source key stay for the
-- draft's life; a thumbnail is one template state's default evaluation. Every `*_at` column is wall clock.

CREATE TABLE approvals (
    -- A ULID, minted at random.
    id             TEXT        PRIMARY KEY,
    -- The turn's conversation, or the one a press came from; NULL for a press with no conversation attached.
    conversation   TEXT        REFERENCES conversations (id),
    -- NULL for a button, which no turn's tool call asked for.
    turn           TEXT,
    call           TEXT,
    tool           TEXT        NOT NULL,
    class          TEXT        NOT NULL
                   CHECK (class IN ('read', 'draft', 'page', 'cancel', 'apply', 'submit', 'destroy', 'publish')),
    input          JSONB       NOT NULL,
    -- SHA-256 of the input's canonical JSON.
    input_digest   TEXT        NOT NULL,
    summary        TEXT        NOT NULL,
    facts          JSONB       NOT NULL,
    via            TEXT        NOT NULL CHECK (via IN ('card', 'button')),
    state          TEXT        NOT NULL CHECK (state IN ('pending', 'approved', 'denied', 'expired')),
    requested_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_by     TEXT        REFERENCES people (subject),
    decided_at     TIMESTAMPTZ,
    reason         TEXT,
    execution      TEXT        CHECK (execution IN ('not_started', 'running', 'ok', 'failed', 'interrupted', 'unknown')),
    -- What the act answered once it ended, and the sentence of a failed or unknown execution.
    outcome        JSONB,
    -- Spec 04's ChangesetDraft exactly as sent, bases and prologue included, written before the first send.
    sent           JSONB,
    unattributable JSONB,
    attribution    JSONB,
    CHECK (via <> 'button' OR (turn IS NULL AND call IS NULL AND state = 'approved')),
    CHECK ((state = 'approved') = (execution IS NOT NULL))
);

-- An approval's input never changes once written, nor its `sent` once set.
CREATE FUNCTION approvals_keep_what_was_approved() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.input IS DISTINCT FROM OLD.input OR (OLD.sent IS NOT NULL AND NEW.sent IS DISTINCT FROM OLD.sent) THEN
        RAISE EXCEPTION 'an approval keeps the input it approved and the changeset it sent';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER approvals_input_and_sent_fixed BEFORE UPDATE ON approvals
    FOR EACH ROW EXECUTE FUNCTION approvals_keep_what_was_approved();

CREATE TABLE template_drafts (
    -- A ULID, minted at random.
    id               TEXT        PRIMARY KEY,
    -- The New site conversation, which Create site binds to the made site; NULL with no conversation attached.
    conversation     TEXT        REFERENCES conversations (id),
    mode             TEXT        NOT NULL DEFAULT 'create' CHECK (mode IN ('create', 'update')),
    -- The template's root scope, template/site/{t}, and its tag (NULL until world-proto-server has tags).
    template         TEXT        NOT NULL,
    tag              TEXT,
    -- Each template scope's changeset as the draft read it: { scope: seq }.
    template_seqs    JSONB       NOT NULL,
    -- Update mode's site, each made scope's head read and the site's prologue; NULL in create mode.
    site             TEXT,
    site_seqs        JSONB,
    site_prologue    JSONB,
    -- The made document's id: a random v4 minted once per draft (the site's own on update).
    doc              TEXT        NOT NULL,
    -- The template's title and knob declarations at template_seqs, which never change.
    title            TEXT        NOT NULL,
    declarations     JSONB       NOT NULL,
    -- { key: value } for every knob, and { key: { by, turn, author, revision } } saying who set each and when.
    knobs            JSONB       NOT NULL,
    set_by           JSONB       NOT NULL,
    revision         INTEGER     NOT NULL DEFAULT 1 CHECK (revision >= 1),
    -- The newest evaluation's number; numbers are given in order under this row's lock.
    latest_n         INTEGER     NOT NULL DEFAULT 0 CHECK (latest_n >= 0),
    state            TEXT        NOT NULL DEFAULT 'open'
                     CHECK (state IN ('open', 'writing', 'created', 'updated', 'abandoned', 'expired')),
    -- The write in progress or last ended: its approval and evaluation, then what it made or the problem refusing it.
    write_approval   TEXT        REFERENCES approvals (id),
    write_evaluation INTEGER,
    write_result     JSONB,
    write_problem    JSONB,
    -- A person's knob changes since the last knob.changed entry: [{ knob, from, to }].
    unjournaled      JSONB       NOT NULL DEFAULT '[]',
    created_by       TEXT        NOT NULL REFERENCES people (subject),
    at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    used_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Seven days after last use (SPEC.md:267).
    expires_at       TIMESTAMPTZ NOT NULL DEFAULT now() + interval '7 days',
    CHECK (state <> 'writing' OR (write_approval IS NOT NULL AND write_evaluation IS NOT NULL))
);

CREATE TABLE template_draft_ids (
    draft      TEXT    NOT NULL REFERENCES template_drafts (id),
    -- A source key (spec 05 section 3.5) and the id the draft first gave it, never deleted while the draft lives.
    key        TEXT    NOT NULL,
    id         TEXT    NOT NULL,
    evaluation INTEGER NOT NULL,
    PRIMARY KEY (draft, key),
    UNIQUE (draft, id)
);

CREATE TABLE template_evaluations (
    draft                 TEXT        NOT NULL REFERENCES template_drafts (id),
    n                     INTEGER     NOT NULL CHECK (n >= 1),
    -- The draft's revision whose knob values this evaluation reads.
    revision              INTEGER     NOT NULL,
    knobs                 JSONB       NOT NULL,
    state                 TEXT        NOT NULL DEFAULT 'in-progress'
                          CHECK (state IN ('in-progress', 'ready', 'failed', 'superseded')),
    -- Whether a person's knob change started it, so its end journals knob.changed.
    by_person             BOOLEAN     NOT NULL DEFAULT false,
    prologue              JSONB,
    engine                JSONB,
    census                JSONB,
    changes_from_template JSONB,
    problems              JSONB       NOT NULL DEFAULT '[]',
    notes                 JSONB       NOT NULL DEFAULT '[]',
    diff                  JSONB,
    -- The run-object store's keys, under template-drafts/{draft}/{n}/.
    plan_key              TEXT,
    floor_key             TEXT,
    floor_bytes           BIGINT,
    floor_version         INTEGER,
    sketch_key            TEXT,
    started_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at           TIMESTAMPTZ,
    PRIMARY KEY (draft, n),
    CHECK (state IN ('in-progress', 'superseded') OR finished_at IS NOT NULL)
);

CREATE TABLE template_thumbnails (
    -- A ULID, minted at random; the object prefix template-thumbnails/{id}/.
    id            TEXT        PRIMARY KEY,
    -- The template state the default evaluation was made at: its root, its scopes' changesets, the engine's
    -- version and the generators the registry ran, so a new changeset or engine makes a new row.
    template      TEXT        NOT NULL,
    template_seqs JSONB       NOT NULL,
    engine        TEXT        NOT NULL,
    generators    JSONB       NOT NULL,
    state         TEXT        NOT NULL DEFAULT 'evaluating' CHECK (state IN ('evaluating', 'ready', 'failed')),
    census        JSONB,
    problems      JSONB       NOT NULL DEFAULT '[]',
    sketch_key    TEXT,
    floor_key     TEXT,
    at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (template, template_seqs, engine, generators)
);

-- Spec 05 section 3.7's columns of the made-from record, beside migration 0007's site, state and cleanup changeset:
-- the act; the template, its tag and the changesets it was read at; the knob values; the changesets the act wrote;
-- the draft, evaluation and approval; the author.
ALTER TABLE site_made_from
    ADD COLUMN act           TEXT CHECK (act IN ('create', 'update')),
    ADD COLUMN template      TEXT,
    ADD COLUMN tag           TEXT,
    ADD COLUMN template_seqs JSONB,
    ADD COLUMN knobs         JSONB,
    ADD COLUMN site_seqs     JSONB,
    ADD COLUMN draft         TEXT REFERENCES template_drafts (id),
    ADD COLUMN evaluation    INTEGER,
    ADD COLUMN approval      TEXT REFERENCES approvals (id),
    ADD COLUMN author        TEXT REFERENCES people (subject);
