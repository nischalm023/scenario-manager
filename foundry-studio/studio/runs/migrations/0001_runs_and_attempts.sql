-- The Crucible run registry: what exists, and how each try at it ended.
--
-- Two tables and no more. Nothing here is true only while a socket is open:
-- the server runs its simulations itself, so there is no runner to be alive or
-- dead and no lease for anyone to hold. What a restart finds in these rows is
-- what was true when the process died.

CREATE TABLE IF NOT EXISTS runs (
    id          TEXT        PRIMARY KEY,
    -- What to call the run in a listing. Not unique: two runs of the same spec
    -- are two runs.
    name        TEXT        NOT NULL,
    -- Where the submitted spec is stored, because a spec embeds a whole
    -- warehouse design and the registry holds no bytes a bucket holds better.
    spec_key    TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Newest first is the only order a listing is asked for.
CREATE INDEX IF NOT EXISTS runs_created_at ON runs (created_at DESC);

-- One row per try at a run, written when the run is submitted. A second row
-- appears only if a run is ever tried again, which nothing does yet; the table
-- is where it is because an attempt is what ends, and a restart has to be able
-- to say that the attempt it found running ended without an ending.
CREATE TABLE IF NOT EXISTS attempts (
    run_id      TEXT        NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    n           INTEGER     NOT NULL,
    -- queued | running | finished | cancelled | failed | partial.
    -- `partial` is what a restart writes over an attempt left `running`: the
    -- process died mid-run and nobody knows how far it got.
    state       TEXT        NOT NULL DEFAULT 'queued',
    started_at  TIMESTAMPTZ,
    ended_at    TIMESTAMPTZ,
    -- The simulator's own word for why it stopped, on an attempt that ended.
    stop_reason TEXT,
    -- Where the recording is stored, once it is. NULL until the upload
    -- returns, so a key in this column names an object that exists.
    object_key  TEXT,
    -- Why a failed attempt failed, in the simulator's words.
    error       TEXT,
    -- How far the run had got, last time anyone looked. Cleared by nothing: a
    -- finished run's last progress is what it finished at.
    progress_sim_ms  BIGINT NOT NULL DEFAULT 0,
    progress_picks   BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, n)
);

-- The queue is the `queued` attempts, oldest run first; this is the index the
-- worker pool's claim reads under FOR UPDATE SKIP LOCKED.
CREATE INDEX IF NOT EXISTS attempts_queued ON attempts (run_id) WHERE state = 'queued';

-- The restart sweep asks for every attempt left running.
CREATE INDEX IF NOT EXISTS attempts_running ON attempts (run_id) WHERE state = 'running';
