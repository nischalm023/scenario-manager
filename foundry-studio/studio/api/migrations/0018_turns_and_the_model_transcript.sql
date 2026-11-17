-- The agent loop's tables (spec 01 section 3.3; brief I1.03, which named this migration 0013 before that number was
-- taken; ruling D122): a conversation's turns with their lease and spend, the model's transcript as rig serializes
-- it, one row per request to the model keeping the provider's own document, the tool calls under ids the API mints
-- (a provider may send none), the questions the agent asks, and the provider a conversation began with, which it
-- keeps. The journal's `turn` and an approval's turn and call now name rows. Every `*_at` column is wall clock.

ALTER TABLE conversations
    ADD COLUMN provider TEXT CHECK (provider IN ('gemini', 'anthropic', 'vertex', 'scripted'));

CREATE TABLE turns (
    -- A ULID, minted at random.
    id                      TEXT        PRIMARY KEY,
    conversation            TEXT        NOT NULL REFERENCES conversations (id),
    -- The turn's ordinal in its conversation: 1, 2, …
    n                       INTEGER     NOT NULL CHECK (n >= 1),
    -- The sender's subject; a test writer's fixture may name a person the service never met.
    started_by              TEXT        NOT NULL,
    attended                BOOLEAN     NOT NULL,
    -- The browser tab whose message started the turn, which alone navigates on its panel commands; NULL unattended.
    browser_tab             TEXT,
    state                   TEXT        NOT NULL
                            CHECK (state IN ('running', 'waiting', 'done', 'failed', 'stopped', 'interrupted')),
    waiting_on              TEXT        CHECK (waiting_on IN ('question', 'approval')),
    waiting_id              TEXT,
    waiting_since           TIMESTAMPTZ,
    message_seq             BIGINT      NOT NULL,
    -- The journal number up to which context notes (panel events, run changes, a person's draft endings) reached
    -- the model; the next turn's notes start after it.
    notes_through           BIGINT,
    started_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at                TIMESTAMPTZ,
    end_reason              TEXT,
    budget                  TEXT,
    -- A running turn's lease: the process's instance id, renewed to 30 s ahead every 10 s; a waiting turn holds none.
    lease_owner             TEXT,
    lease_expires_at        TIMESTAMPTZ,
    model_calls             INTEGER     NOT NULL DEFAULT 0,
    input_tokens            BIGINT      NOT NULL DEFAULT 0,
    output_tokens           BIGINT      NOT NULL DEFAULT 0,
    cache_read_input_tokens BIGINT      NOT NULL DEFAULT 0,
    -- Wall-clock milliseconds spent running, waits excluded.
    active_ms               BIGINT      NOT NULL DEFAULT 0,
    -- The texts (the page line, then the words) of messages that ended a wait, kept until they reach the model after
    -- the results of their reply's calls, however many pauses come first.
    pending_words           JSONB       NOT NULL DEFAULT '[]',
    -- What the turn's last panel.show opened ({ panel, location, kin }), which its later panel commands without a
    -- location act on, kept across its waits (spec 03 section 3.3).
    panel_context           JSONB,
    UNIQUE (conversation, n),
    UNIQUE (id, conversation),
    CHECK ((state = 'waiting') = (waiting_on IS NOT NULL)),
    CHECK ((state IN ('running', 'waiting')) = (ended_at IS NULL))
);

-- At most one turn of a conversation runs or waits, whatever the code does (spec 01 section 8, statement 1).
CREATE UNIQUE INDEX turns_one_open_per_conversation ON turns (conversation) WHERE state IN ('running', 'waiting');

-- An entry's turn is one of its own conversation's. Rows written before this migration are not checked again.
ALTER TABLE journal ADD CONSTRAINT journal_turn_of_its_conversation
    FOREIGN KEY (turn, conversation) REFERENCES turns (id, conversation) NOT VALID;

CREATE TABLE model_messages (
    conversation TEXT        NOT NULL REFERENCES conversations (id),
    n            INTEGER     NOT NULL CHECK (n >= 1),
    role         TEXT        NOT NULL CHECK (role IN ('user', 'assistant')),
    -- rig's `Message` as it serializes, provider items (Gemini's thought signatures) included.
    content      JSONB       NOT NULL,
    turn         TEXT        REFERENCES turns (id),
    at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- For a compaction's summary, the last `n` it replaces: later requests send nothing before it.
    summary_of   INTEGER,
    PRIMARY KEY (conversation, n)
);

-- The transcript takes inserts only: a message, once sent, is what the model read.
CREATE FUNCTION model_messages_refuse_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'the model transcript takes inserts only, so % is refused', TG_OP;
END;
$$;

CREATE TRIGGER model_messages_append_only BEFORE UPDATE OR DELETE OR TRUNCATE ON model_messages
    FOR EACH STATEMENT EXECUTE FUNCTION model_messages_refuse_change();

CREATE TABLE model_calls (
    turn                        TEXT        NOT NULL REFERENCES turns (id),
    n                           INTEGER     NOT NULL CHECK (n >= 1),
    provider                    TEXT        NOT NULL,
    model                       TEXT        NOT NULL,
    -- `step` for the turn's own requests; `summary` for a budget's last request; `compaction` for a summary of
    -- earlier turns.
    purpose                     TEXT        NOT NULL CHECK (purpose IN ('step', 'summary', 'compaction')),
    started_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at                    TIMESTAMPTZ,
    stop_reason                 TEXT,
    input_tokens                BIGINT,
    output_tokens               BIGINT,
    cache_creation_input_tokens BIGINT,
    cache_read_input_tokens     BIGINT,
    provider_request_id         TEXT,
    -- A failed request's status and the provider's words, never a URL or a key.
    error                       JSONB,
    -- The provider's own document for the reply.
    raw                         JSONB,
    PRIMARY KEY (turn, n)
);

CREATE TABLE tool_calls (
    -- A ULID the API mints, the call's idempotency key.
    id           TEXT        PRIMARY KEY,
    conversation TEXT        NOT NULL REFERENCES conversations (id),
    turn         TEXT        NOT NULL REFERENCES turns (id),
    -- The request whose reply made the call, and its place among that reply's calls.
    model_call   INTEGER     NOT NULL,
    position     INTEGER     NOT NULL,
    -- rig's `ToolCall` as the model made it, which the call's result answers.
    wire_call    JSONB       NOT NULL,
    -- The catalogue name, or the name the model sent when no tool has it.
    tool         TEXT        NOT NULL,
    class        TEXT        NOT NULL,
    input        JSONB       NOT NULL,
    state        TEXT        NOT NULL
                 CHECK (state IN ('running', 'waiting', 'ok', 'failed', 'denied', 'stopped', 'interrupted')),
    result       JSONB,
    problem      JSONB,
    summary      TEXT        NOT NULL,
    started_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at     TIMESTAMPTZ,
    UNIQUE (turn, model_call, position)
);

CREATE TABLE questions (
    -- A ULID, minted at random.
    id          TEXT        PRIMARY KEY,
    turn        TEXT        NOT NULL REFERENCES turns (id),
    call        TEXT        NOT NULL REFERENCES tool_calls (id),
    items       JSONB       NOT NULL,
    state       TEXT        NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'answered', 'expired')),
    answers     JSONB,
    in_words    BOOLEAN     NOT NULL DEFAULT false,
    answered_by TEXT        REFERENCES people (subject),
    answered_at TIMESTAMPTZ,
    asked_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE approvals
    ADD CONSTRAINT approvals_turn_is_a_turn FOREIGN KEY (turn) REFERENCES turns (id),
    ADD CONSTRAINT approvals_call_is_a_call FOREIGN KEY (call) REFERENCES tool_calls (id);
