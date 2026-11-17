-- A partial copy says it is partial (6 November 2026). An ended attempt with no final recording is read from the live
-- segments its streaming server uploaded (migration 0023), and those stop short when that server died with uploads
-- still queued. This column on the stream's row says the server's upload queue drained after the attempt ended: every
-- segment the server took is in the store, and `segments_uploaded` counts them all. It stays NULL until then, and for
-- good when the server died first, so a reader of the segments ends them saying the recording is incomplete. The
-- server writes it under its own upsert, as it writes 0023's columns, and refuses to start without it. Wall clock.
ALTER TABLE attempt_streams ADD COLUMN uploads_finished_at TIMESTAMPTZ;
