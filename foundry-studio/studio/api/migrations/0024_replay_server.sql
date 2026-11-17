-- One streaming server per ended attempt (6 November 2026), so a page's picture and charts, and every later page,
-- read the attempt through one server's single download of it. A watch ticket names the attempt's own server while
-- its heartbeat is fresh. Once that server is gone and the store holds the attempt's picture, the first ticket picks a
-- live server and records it here, and every later ticket names that one while its heartbeat stays fresh; a ticket
-- finding it gone too records another. NULL until a ticket needed one. Like `streaming_server`, it names a server's
-- row without a foreign key, since a dead server's row is deleted.
ALTER TABLE attempts ADD COLUMN replay_server TEXT;
