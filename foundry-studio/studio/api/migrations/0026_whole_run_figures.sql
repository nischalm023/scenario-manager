-- A Status total answers over any window (7 November 2026). The end block's Status figures, the run's total picks
-- among them, are the whole run's by definition and store no running values, so an additive one had no value over a
-- report window that starts after a warm-up. `metrics.json` now marks each such figure (`whole_run`); this column keeps
-- the mark, and the results routes answer a marked metric's value over any window, marked by the whole run's. An
-- attempt stored before the mark keeps FALSE, and answers as it did.
ALTER TABLE attempt_metrics ADD COLUMN whole_run BOOLEAN NOT NULL DEFAULT FALSE;
