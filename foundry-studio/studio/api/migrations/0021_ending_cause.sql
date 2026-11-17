-- A typed cause beside an ending's words, for the endings counted apart from the rest: a Job that could not be created,
-- which the deployment's run counts read (`GET /api/runs/stats`, spec 08 section 3.7). The words stay what a person
-- reads, and no count matches them. The endings written before this migration are known by the words the dispatcher
-- began them with, which were its only mark.
ALTER TABLE attempts
    ADD COLUMN ending_cause TEXT CHECK (ending_cause IN ('job-not-created')),
    ADD CHECK (ending_cause IS NULL OR ended_at IS NOT NULL);

UPDATE attempts SET ending_cause = 'job-not-created'
    WHERE ended_by = 'platform' AND starts_with(stop_reason, 'its Job could not be created');
