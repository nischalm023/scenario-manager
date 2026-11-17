-- The streaming servers (ruling D174): a run's events streamed live by its runner to one of several streaming
-- servers, which register themselves here with where a runner and a viewer reach them, and say they are alive every
-- 2 s of wall clock, a beat that writes the row again if it was deleted. When it dispatches an attempt, the
-- dispatcher picks one whose heartbeat is within STUDIO_STREAMING_STALE_SECS (10 s) by the database's clock, at
-- random, and records it on the attempt (`attempts.streaming_server`); a row whose heartbeat stopped 10 minutes ago
-- is a dead server's, and the next beat of any server deletes it. `open_streams` counts the streams the server holds
-- open, for whoever reads the table.
CREATE TABLE streaming_servers (
    id           TEXT        PRIMARY KEY,
    runner_url   TEXT        NOT NULL,
    viewer_url   TEXT        NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL,
    open_streams INTEGER     NOT NULL DEFAULT 0 CHECK (open_streams >= 0),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE attempts RENAME COLUMN worldview_server TO streaming_server;

-- One row per attempt a streaming server streamed, its stream's own facts, written as they change at most every 5 s
-- of wall clock and at once when the stream closes: how far the run's progress reached in simtime and its counters by
-- name as the runner's heartbeat reads them (the shape of `attempts.progress_values`), and when the stream closed and
-- who closed it, its runner with its ending or the server, the runner lost past its grace or the server stopping. How
-- the attempt ended is the API's, on `attempts`. `revision` orders the writes: a row takes a write only with a newer
-- revision, or from another server process, which counts its own; `server_id` is the process that holds the stream,
-- which a watch ticket names.
CREATE TABLE attempt_streams (
    run_id          TEXT        NOT NULL,
    n               INTEGER     NOT NULL,
    server_id       TEXT        NOT NULL,
    revision        BIGINT      NOT NULL CHECK (revision >= 0),
    opened_at       TIMESTAMPTZ NOT NULL,
    written_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    sim_ms          BIGINT      NOT NULL CHECK (sim_ms >= 0),
    progress_values JSONB       NOT NULL,
    closed_at       TIMESTAMPTZ,
    closed_by       TEXT        CHECK (closed_by IN ('runner', 'server')),
    CHECK ((closed_at IS NULL) = (closed_by IS NULL)),
    PRIMARY KEY (run_id, n),
    FOREIGN KEY (run_id, n) REFERENCES attempts (run_id, n)
);

-- The runners' door's rule for a runner's token, kept once for both its readers, the door and a streaming server
-- admitting a runner: a token opens its attempt while the attempt's Job is not known stopped.
CREATE VIEW live_runner_tokens AS
    SELECT t.token_sha256, t.run, t.n
    FROM runner_tokens t JOIN attempts a ON a.run_id = t.run AND a.n = t.n
    WHERE a.job_stopped_at IS NULL;

-- A viewer's ticket to watch one attempt's stream, minted by the API for a person who asked
-- (`POST /api/runs/{run}/watch`) and presented to the streaming server's external port, which admits it by its SHA-256
-- until it expires, by the database's clock (`live_watch_tickets`). Each mint deletes the expired ones.
CREATE TABLE stream_watch_tickets (
    token_sha256 TEXT        PRIMARY KEY,
    run_id       TEXT        NOT NULL,
    n            INTEGER     NOT NULL,
    subject      TEXT        NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (run_id, n) REFERENCES attempts (run_id, n)
);

CREATE VIEW live_watch_tickets AS
    SELECT token_sha256, run_id, n FROM stream_watch_tickets WHERE expires_at > now();
