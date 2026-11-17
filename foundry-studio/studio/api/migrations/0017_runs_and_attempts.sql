-- Spec 08 section 3.2's tables (brief I1.09): a plan prepared and checked before anyone approves it, the submission
-- that executes one, its runs, and each run's attempts, a run's state being its latest attempt's; every change of an
-- attempt's state as a durable row the watcher hands on in order; the hash of each attempt's runner token; and what a
-- finished attempt's runner reported, its `metrics.json` and `series.json`, as rows. Every `*_at` column is wall
-- clock, every `_sim_ms` simtime.

CREATE TABLE plans (
    -- A ULID, minted at random; its runs' ULIDs are minted with it, inside `plan`.
    id            TEXT        PRIMARY KEY,
    experiment    TEXT        NOT NULL REFERENCES experiments (id),
    -- The site's live conversation when it was asked for, if one was open.
    conversation  TEXT        REFERENCES conversations (id),
    prepared_by   TEXT        NOT NULL REFERENCES people (subject),
    -- What the preparation is keyed by besides the experiment: the preparer's image by digest (NULL when none is
    -- registered, which fails the plan) and whether a developer chose it, and the changesets a press showed.
    runner_image  TEXT,
    runner_chosen TEXT        NOT NULL CHECK (runner_chosen IN ('default', 'developer')),
    shown_cut     JSONB,
    state         TEXT        NOT NULL DEFAULT 'preparing' CHECK (state IN ('preparing', 'ready', 'failed')),
    progress      JSONB       NOT NULL DEFAULT '{"worlds_done": 0, "worlds": 0}',
    problem       JSONB,
    plan          JSONB,
    -- SHA-256 of the plan's RFC 8785 canonical JSON without `queue`.
    plan_sha256   TEXT,
    started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    ready_at      TIMESTAMPTZ,
    -- The press that executes it, recorded before its execution (spec 01 section 3.10, rule 6), so a press repeated or
    -- racing answers that approval.
    approval      TEXT        UNIQUE REFERENCES approvals (id),
    CHECK ((state = 'ready') = (plan IS NOT NULL AND plan_sha256 IS NOT NULL AND ready_at IS NOT NULL)),
    CHECK ((state = 'failed') = (problem IS NOT NULL)),
    CHECK (approval IS NULL OR state = 'ready')
);

CREATE INDEX plans_preparing ON plans (experiment) WHERE state = 'preparing';

CREATE TABLE submissions (
    id            TEXT        PRIMARY KEY,
    -- The plan it executed, exactly as approved; one submission per experiment.
    plan          TEXT        NOT NULL UNIQUE REFERENCES plans (id),
    plan_sha256   TEXT        NOT NULL,
    experiment    TEXT        NOT NULL UNIQUE REFERENCES experiments (id),
    conversation  TEXT        REFERENCES conversations (id),
    approval      TEXT        NOT NULL UNIQUE REFERENCES approvals (id),
    submitted_by  TEXT        NOT NULL REFERENCES people (subject),
    submitted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The submission's attempt counts at which `submission.ended` was journaled (spec 08 section 3.6).
    ended_rounds  INTEGER[]   NOT NULL DEFAULT '{}'
);

CREATE TABLE runs (
    -- A ULID, minted at random with its plan: one key segment of the run store (runstore's layout).
    id               TEXT        PRIMARY KEY CHECK (id ~ '^[0-9A-HJKMNP-TV-Z]{26}$'),
    submission       TEXT        NOT NULL REFERENCES submissions (id),
    experiment       TEXT        NOT NULL REFERENCES experiments (id),
    -- The run's place in its submission and in the queue, from 1.
    position         INTEGER     NOT NULL CHECK (position >= 1),
    scenario         TEXT        NOT NULL,
    scenario_version INTEGER     NOT NULL,
    -- `[{ target, pointer, value, label, applied_to }]` in spec 07's shape; `[]` for a single run.
    variant          JSONB       NOT NULL,
    seed             BIGINT      NOT NULL CHECK (seed >= 0),
    duration_sim_ms  BIGINT      NOT NULL CHECK (duration_sim_ms > 0),
    report_window    JSONB       NOT NULL,
    -- Spec 04's `UnitCut` as the API answers it, and `{ scope: seq }` for every scope the check's export carried.
    cut              JSONB       NOT NULL,
    reads            JSONB       NOT NULL,
    -- The image by digest, and the check's digests.
    runner_image     TEXT        NOT NULL,
    expected         JSONB       NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (submission, position),
    FOREIGN KEY (scenario, scenario_version) REFERENCES scenario_versions (scenario, version)
);

-- Crucible's columns (studio/runs/migrations), `progress_picks` replaced by the discovered `progress_values`, and
-- spec 08's: the seat (`job` to `job_stopped_at`), the cancel request, who ended it, the runner's identity and what its
-- manifest said.
CREATE TABLE attempts (
    run_id                 TEXT        NOT NULL REFERENCES runs (id),
    n                      INTEGER     NOT NULL CHECK (n >= 1),
    state                  TEXT        NOT NULL DEFAULT 'queued'
                           CHECK (state IN ('queued', 'running', 'finished', 'cancelled', 'failed', 'partial')),
    -- The approval's instant; the queue's order with the run's position and n.
    requested_at           TIMESTAMPTZ NOT NULL,
    -- Recorded before the Job is created; a seat is held from here until `job_stopped_at`.
    job                    TEXT        UNIQUE,
    dispatched_at          TIMESTAMPTZ,
    -- The loops instance creating the Job (spec 08 section 3.4, step 5), claimed with the reservation or by the
    -- reconciler's re-creation and let go once every create of it has returned, and when that creation began.
    job_creator            TEXT,
    job_claimed_at         TIMESTAMPTZ,
    worldview_server       TEXT,
    job_stopped_at         TIMESTAMPTZ,
    cancel_requested_at    TIMESTAMPTZ,
    started_at             TIMESTAMPTZ,
    ended_at               TIMESTAMPTZ,
    stop_reason            TEXT,
    stop_detail            TEXT,
    error                  TEXT,
    ended_by               TEXT        CHECK (ended_by IN ('simulator', 'person', 'platform')),
    progress_sim_ms        BIGINT      NOT NULL DEFAULT 0,
    progress_values        JSONB       NOT NULL DEFAULT '[]',
    runner_commit          TEXT,
    runner_dirty           BOOLEAN,
    object_key             TEXT,
    recording_live_edge_ms BIGINT,
    recording_frames       BIGINT,
    recording_size_bytes   BIGINT,
    -- The manifest as handed in, stored in the run store, and what the API found comparing it with the plan.
    manifest_key           TEXT,
    manifest_sha256        TEXT,
    compile_sha256         TEXT,
    findings               JSONB       NOT NULL DEFAULT '[]',
    PRIMARY KEY (run_id, n),
    CHECK ((state IN ('queued', 'running')) = (ended_at IS NULL)),
    CHECK ((ended_at IS NULL) = (ended_by IS NULL)),
    CHECK ((job IS NULL) = (dispatched_at IS NULL)),
    CHECK ((job IS NULL) = (job_claimed_at IS NULL)),
    CHECK (job_creator IS NULL OR job IS NOT NULL),
    CHECK (job_stopped_at IS NULL OR job IS NOT NULL),
    CHECK ((manifest_key IS NULL) = (manifest_sha256 IS NULL))
);

-- The queue: waiting attempts, read under FOR UPDATE SKIP LOCKED in approval order.
CREATE INDEX attempts_waiting ON attempts (requested_at) WHERE state = 'queued' AND job IS NULL;
-- The seats: every attempt whose Job is not known stopped, ended or not.
CREATE INDEX attempts_holding ON attempts (run_id, n) WHERE job IS NOT NULL AND job_stopped_at IS NULL;

-- An attempt's state only moves forward, and an ending is never written over: the first ending wins, whoever writes.
CREATE FUNCTION attempts_state_moves_forward() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.state IS DISTINCT FROM OLD.state
       AND (OLD.state NOT IN ('queued', 'running') OR NEW.state = 'queued') THEN
        RAISE EXCEPTION 'attempt %/% is %, which does not move to %', OLD.run_id, OLD.n, OLD.state, NEW.state;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER attempts_state_moves_forward BEFORE UPDATE OF state ON attempts
    FOR EACH ROW EXECUTE FUNCTION attempts_state_moves_forward();

-- One row per change of an attempt's state, appended in the writer's own transaction, whoever writes. `tx` is that
-- transaction's id: the watcher hands rows on in (tx, id) order, and only from transactions older than every one
-- still running, so a row committed late behind a smaller id is never passed over.
CREATE TABLE attempt_transitions (
    id     BIGSERIAL   PRIMARY KEY,
    tx     BIGINT      NOT NULL DEFAULT (pg_current_xact_id()::text::bigint),
    run    TEXT        NOT NULL,
    n      INTEGER     NOT NULL,
    "from" TEXT,
    "to"   TEXT        NOT NULL,
    at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (run, n) REFERENCES attempts (run_id, n)
);

CREATE INDEX attempt_transitions_in_order ON attempt_transitions (tx, id);

CREATE FUNCTION attempts_record_transition() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'INSERT' OR NEW.state IS DISTINCT FROM OLD.state THEN
        INSERT INTO attempt_transitions (run, n, "from", "to")
        VALUES (NEW.run_id, NEW.n, CASE WHEN TG_OP = 'UPDATE' THEN OLD.state END, NEW.state);
        PERFORM pg_notify('studio_attempt_transitions', NEW.run_id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER attempts_record_transition AFTER INSERT OR UPDATE OF state ON attempts
    FOR EACH ROW EXECUTE FUNCTION attempts_record_transition();

-- The last transition the watcher handed to `RunEvents`, as (transaction, id): one row.
CREATE TABLE run_event_cursor (
    one             BOOLEAN PRIMARY KEY DEFAULT true CHECK (one),
    last_tx         BIGINT  NOT NULL DEFAULT 0,
    last_transition BIGINT  NOT NULL DEFAULT 0
);

INSERT INTO run_event_cursor DEFAULT VALUES;

-- Spec 01 section 3.16: the SHA-256 of each attempt's token, never the token; valid while its Job is not known stopped.
CREATE TABLE runner_tokens (
    run          TEXT        NOT NULL,
    n            INTEGER     NOT NULL,
    token_sha256 TEXT        NOT NULL UNIQUE,
    minted_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run, n),
    FOREIGN KEY (run, n) REFERENCES attempts (run_id, n)
);

-- The runner's `metrics.json` (the Worldview service's summary-metric sidecar in November): one row per metric the
-- attempt declared, in descriptor order, its final value NULL where the run never stated one.
CREATE TABLE attempt_metrics (
    run_id      TEXT             NOT NULL,
    n           INTEGER          NOT NULL,
    position    INTEGER          NOT NULL CHECK (position >= 1),
    metric      TEXT             NOT NULL,
    label       TEXT             NOT NULL,
    description TEXT             NOT NULL,
    unit        TEXT             NOT NULL
                CHECK (unit IN ('count', 'duration_ms', 'distance_mm', 'ratio', 'rate_per_hour')),
    aggregation TEXT             NOT NULL
                CHECK (aggregation IN ('additive', 'interval', 'ratio', 'snapshot_gauge', 'distinct_count')),
    denominator TEXT,
    value       DOUBLE PRECISION,
    basis       TEXT             NOT NULL CHECK (basis IN ('end_block', 'rebuilt')),
    PRIMARY KEY (run_id, n, metric),
    UNIQUE (run_id, n, position),
    FOREIGN KEY (run_id, n) REFERENCES attempts (run_id, n)
);

-- The runner's `series.json`: per metric, an accumulating one's running value at each slice end from the run's start,
-- or a gauge's readings at each slice that stated one; the instants in simtime, never thinned.
CREATE TABLE attempt_series (
    run_id    TEXT               NOT NULL,
    n         INTEGER            NOT NULL,
    metric    TEXT               NOT NULL,
    kind      TEXT               NOT NULL CHECK (kind IN ('running', 'readings')),
    at_sim_ms BIGINT[]           NOT NULL,
    value     DOUBLE PRECISION[] NOT NULL,
    CHECK (cardinality(at_sim_ms) = cardinality(value)),
    PRIMARY KEY (run_id, n, metric),
    FOREIGN KEY (run_id, n, metric) REFERENCES attempt_metrics (run_id, n, metric)
);
