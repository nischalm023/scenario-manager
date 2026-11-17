-- A stored duration names its clock (7 November 2026). `metrics.json` declares the clock each duration or rate's hour
-- counts in, simtime or the wall clock, as the metric stream's descriptor does; this column keeps it, so the results
-- routes carry it and the page never reads the run's wall-clock figure as simtime. NULL for a count, a distance or a
-- ratio, and for every metric of an attempt stored before the column, which the page reads as simtime as it did.
ALTER TABLE attempt_metrics ADD COLUMN clock TEXT CHECK (clock IN ('sim', 'wall'));
