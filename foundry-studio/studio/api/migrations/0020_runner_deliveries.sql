-- What a runner delivers once its run is over, and when (spec 08 section 3.5): the bytes of the one recording an
-- attempt's run store holds, and the instant the runner's own report wrote the ending. A recording's SHA-256 is written
-- when the first upload of it begins storing it, so from then on a recording of other bytes is refused, and a retry of
-- the same bytes stores and answers the same one; the recording is accepted once `object_key` names it, which happens
-- only while the attempt is queued or running, whether or not a report follows, and no ending written later clears it.
-- `reported_at` lets the reconciler leave the Job of an attempt its runner reported to exit by itself. Every `*_at`
-- column is wall clock.
ALTER TABLE attempts
    ADD COLUMN recording_sha256 TEXT,
    ADD COLUMN reported_at      TIMESTAMPTZ,
    ADD CHECK (reported_at IS NULL OR ended_at IS NOT NULL);
