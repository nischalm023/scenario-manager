-- What an attempt's recording holds, written down where the attempt is.
--
-- It was read off the recording itself, which meant fetching a 1.5 MB object
-- per run to answer "how long is it?" — and `list`, which every browser asks
-- every second, could not answer at all. The three numbers are known at the
-- moment the attempt ends and the file is still on this machine, so they are
-- written then and read back with the row.
--
-- NULL means no recording: an attempt still queued or running, one that failed
-- before it wrote anything, or one a restart swept to `partial`.

ALTER TABLE attempts ADD COLUMN IF NOT EXISTS recording_live_edge_ms BIGINT;
ALTER TABLE attempts ADD COLUMN IF NOT EXISTS recording_frames       BIGINT;
ALTER TABLE attempts ADD COLUMN IF NOT EXISTS recording_size_bytes   BIGINT;
