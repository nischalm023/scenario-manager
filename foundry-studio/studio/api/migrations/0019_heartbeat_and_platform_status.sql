-- Watching a run while it goes (22 October 2026): when its runner was last heard, a heartbeat every 5 s of wall
-- clock from the runner's start, and what the platform last said of its Job in words (`pending: ImagePullBackOff`,
-- `running`, `ended: DeadlineExceeded`), written by the reconciler each time it asks. Both read off the row, never
-- live; a running attempt whose heartbeat is older than 30 s reads as stalled.
ALTER TABLE attempts
    ADD COLUMN heartbeat_at    TIMESTAMPTZ,
    ADD COLUMN platform_status TEXT;
