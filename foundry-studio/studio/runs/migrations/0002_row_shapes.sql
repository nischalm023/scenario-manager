-- What a row may say, stated where it is stored.
--
-- Both of these were true of every row the server writes and neither was
-- written down, so a row from somewhere else — a hand-edited registry, a
-- future writer, a restore of a database from another build — could say
-- something the server has no answer for. A run id that is not one key segment
-- names no object in storage; an attempt numbered zero is a caller that lost
-- count, and the layout has no attempt 0 to put a recording under.

ALTER TABLE attempts DROP CONSTRAINT IF EXISTS attempts_n_counts_from_one;
ALTER TABLE attempts ADD CONSTRAINT attempts_n_counts_from_one CHECK (n >= 1);

-- The same alphabet `runstore`'s keys are drawn from: letters, digits, `.`,
-- `_` and `-`, and never a name a filesystem reads as a directory of its own.
ALTER TABLE runs DROP CONSTRAINT IF EXISTS runs_id_is_one_key_segment;
ALTER TABLE runs ADD CONSTRAINT runs_id_is_one_key_segment
    CHECK (id ~ '^[A-Za-z0-9._-]+$' AND id <> '.' AND id <> '..');
