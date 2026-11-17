-- Ruling D121, one renderer: the page's worldview viewer draws every floor, an evaluation's and a template's default
-- one included, so the API draws no SVG and keeps none. Migration 0008's two drawing-key columns go; a drawing an
-- earlier build stored stays in the run-object store, read by nothing.

ALTER TABLE template_evaluations DROP COLUMN sketch_key;

ALTER TABLE template_thumbnails DROP COLUMN sketch_key;
