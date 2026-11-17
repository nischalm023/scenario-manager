-- Spec 08 section 3.2's `metric_pins` (brief I1.10): a project's headline metrics as data, in pin order, at most
-- eight, so that no metric name is written into code (section 3.9, the headline rule: the project's pins, else the
-- agent's `show` command's metrics, else the first four a run declared). `PUT /api/projects/{p}/metric-pins`
-- replaces a project's list whole. The number 0011 was kept for this table; a database that applied 0012 to 0017
-- before it takes it next, since nothing before it reads it. `pinned_at` is wall clock.

CREATE TABLE metric_pins (
    project   TEXT        NOT NULL REFERENCES projects (id),
    position  INTEGER     NOT NULL CHECK (position BETWEEN 1 AND 8),
    -- A metric's name as a run declares it; a name no run of the project declared yet is kept, and shows nothing.
    metric    TEXT        NOT NULL CHECK (metric <> ''),
    pinned_by TEXT        NOT NULL REFERENCES people (subject),
    pinned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project, position),
    UNIQUE (project, metric)
);
