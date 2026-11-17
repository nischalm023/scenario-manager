-- The site form of a journal append (spec 01 section 3.17's last paragraph) looks a run event's dedup key up across
-- every conversation of its site, archived ones included, so the entry lands once wherever Dismiss has moved the
-- site's live conversation. This index finds a key's entries without reading the site's whole journal.

CREATE INDEX journal_by_dedup_key ON journal (dedup_key) WHERE dedup_key IS NOT NULL;
