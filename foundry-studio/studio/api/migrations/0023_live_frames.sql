-- Live frames (26 October 2026): the streaming server also carries the run's worldview picture, one locked segment at
-- a time, uploads each to the run-object store the moment it arrives and keeps a copy on its own disk for replay.
-- These columns on the stream's row (migration 0022's `attempt_streams`) say what is where. The final recording is not
-- here: the runner sends it to the API's door, and `attempts.object_key` names it. The server writes them under its
-- own upsert, beside the stats writer's revisioned columns, and refuses to start without them, naming this
-- migration, as it does for 0022's tables. Every `_sim_ms` is simtime, every `_at` wall clock.
ALTER TABLE attempt_streams
    -- The scenario block's object key under the attempt's `live/` prefix once uploaded; NULL until then.
    ADD COLUMN scenario_key      TEXT,
    -- How many locked segments are uploaded, in order from segment 0, so segments 0 to segments_uploaded - 1 are in
    -- the store and nothing past them is.
    ADD COLUMN segments_uploaded INTEGER     NOT NULL DEFAULT 0 CHECK (segments_uploaded >= 0),
    -- The simtime instant the locked picture reaches: the end of the newest uploaded segment, 0 before the first.
    ADD COLUMN locked_sim_ms     BIGINT      NOT NULL DEFAULT 0 CHECK (locked_sim_ms >= 0),
    -- When the server's disk copy was written, NULL once the sweeper deleted it; and when a viewer last opened the
    -- attempt's picture, which the sweeper's age reads first.
    ADD COLUMN cached_at         TIMESTAMPTZ,
    ADD COLUMN last_opened_at    TIMESTAMPTZ;
