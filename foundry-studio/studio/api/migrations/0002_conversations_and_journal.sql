-- A site's conversations and their journal (spec 01 sections 3.3, 3.5 and 3.17). A conversation belongs to a site
-- (ruling D94): one live conversation per site and, before a site exists, one live new-site thread per person, whose
-- `site` stays NULL until Create site binds it. Dismissing a site's conversation archives it and opens the next;
-- dismissing a new-site thread discards it. The journal is a conversation's record, numbered in its site's one
-- sequence: a site's next conversation starts one past the last one's `last_seq`, so one number names one entry of
-- the site. Every `*_at` column is wall clock.

CREATE TABLE conversations (
    -- A ULID, minted at random.
    id               TEXT        PRIMARY KEY,
    -- Its reference to projects arrives with that table's migration.
    project          TEXT        NOT NULL,
    -- The site's root scope path, customer/{c}/site/{s}; NULL for a new-site thread until it binds.
    site             TEXT,
    state            TEXT        NOT NULL DEFAULT 'live' CHECK (state IN ('live', 'archived', 'discarded')),
    title            TEXT,
    created_by       TEXT        NOT NULL REFERENCES people (subject),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    bound_at         TIMESTAMPTZ,
    ended_at         TIMESTAMPTZ,
    ended_by         TEXT        REFERENCES people (subject),
    -- The journal counter: the first number this conversation holds and the last it wrote. A new-site thread starts
    -- at 1, a site's next conversation one past the last one's last_seq.
    first_seq        BIGINT      NOT NULL DEFAULT 1 CHECK (first_seq >= 1),
    last_seq         BIGINT      NOT NULL DEFAULT 0,
    -- What the agent loop last sent the model under (spec 01 section 3.7); NULL before its first request.
    prompt_version   TEXT,
    catalogue_digest TEXT,
    model            TEXT,
    -- The first model message whose thinking blocks are still sent back (spec 01 section 3.7).
    thinking_floor   INTEGER     NOT NULL DEFAULT 0,
    CHECK (last_seq >= first_seq - 1)
);

CREATE UNIQUE INDEX conversations_one_live_per_site ON conversations (site) WHERE state = 'live';
CREATE UNIQUE INDEX conversations_one_new_site_thread_per_person ON conversations (created_by)
    WHERE site IS NULL AND state = 'live';

CREATE TABLE journal (
    conversation TEXT        NOT NULL REFERENCES conversations (id),
    -- The entry's number, taken from conversations.last_seq in the transaction that inserts it, so numbers have no
    -- gaps.
    seq          BIGINT      NOT NULL CHECK (seq >= 1),
    at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The turn that wrote the entry, or NULL; its reference to turns arrives with the agent loop's migration.
    turn         TEXT,
    kind         TEXT        NOT NULL CHECK (kind IN (
        'message.user', 'turn.started', 'text.done', 'tool.started', 'tool.progress', 'tool.finished',
        'question.asked', 'question.answered', 'question.expired',
        'approval.requested', 'approval.decided', 'approval.expired', 'approval.noted', 'approval.execution',
        'draft.updated', 'draft.applied', 'draft.discarded', 'draft.stale',
        'panel.command', 'panel.event', 'run.status', 'submission.ended', 'run.finding',
        'turn.waiting', 'turn.resumed', 'turn.finished', 'system.note', 'conversation.changed'
    )),
    body         JSONB       NOT NULL,
    -- The tool call, question or approval the entry answers, or NULL.
    caused_by    TEXT,
    -- A writer's key for an entry that lands once however often it is reported, such as a run's status change.
    dedup_key    TEXT,
    PRIMARY KEY (conversation, seq)
);

CREATE UNIQUE INDEX journal_one_entry_per_dedup_key ON journal (conversation, dedup_key) WHERE dedup_key IS NOT NULL;

-- The journal takes inserts only: an entry, once written, is the record.
CREATE FUNCTION journal_refuses_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'the journal takes inserts only, so % is refused', TG_OP;
END;
$$;

CREATE TRIGGER journal_append_only BEFORE UPDATE OR DELETE OR TRUNCATE ON journal
    FOR EACH STATEMENT EXECUTE FUNCTION journal_refuses_change();
