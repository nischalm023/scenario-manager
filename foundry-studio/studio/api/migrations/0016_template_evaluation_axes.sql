-- Spec 05 section 6 and spec 03 section 3.6: an evaluation's stage draws the axes of the frame its template's wall
-- knobs name, and its text says which way they point, so each evaluation keeps that frame's name and directions beside
-- its floor. An evaluation stored before this keeps none and draws none.

ALTER TABLE template_evaluations ADD COLUMN axes JSONB;
