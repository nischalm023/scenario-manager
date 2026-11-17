-- Create site writes a site in one changeset, which world-proto-server applies whole or refuses whole (scopes phase
-- 3, world-proto 092d695): nothing of a refused act is left to clean up or to keep off the site list, and the
-- made-from record is written once, `made`, in the transaction that marks the draft `created` and binds its thread
-- (spec 05 section 4.4). Migration 0007's states for a staged Create site, `creating` and `abandoned`, go with the
-- root's cleanup changeset and the index that found abandoned roots; `updating` and `partial` stay for update
-- (spec 05 section 4.5) until its task settles them. 0007 may have run on a developer's database, so this alters it.

DROP INDEX site_made_from_abandoned;

-- Dropping the column drops its own CHECK and the table's CHECK that names it (`state <> 'abandoned' OR ...`).
ALTER TABLE site_made_from DROP COLUMN cleaned_root_seq;

ALTER TABLE site_made_from
    DROP CONSTRAINT site_made_from_state_check,
    ADD CONSTRAINT site_made_from_state_check CHECK (state IN ('updating', 'made', 'updated', 'partial'));
