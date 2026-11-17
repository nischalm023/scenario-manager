-- Create site's body (spec 05 section 4.4): the one changeset world-proto-server is sent for a draft's write, kept
-- before each sending, so that a write asked again after a lost answer or a restart sends the same body, which WPS
-- answers by replaying what it stored under the operation identifier. It is replaced only when WPS refused the body
-- before it, since a refused changeset stores nothing under its identifier; the plan the body is built from stays
-- with the approval, in `approvals.sent`, which never changes.

ALTER TABLE template_drafts ADD COLUMN write_body JSONB;
