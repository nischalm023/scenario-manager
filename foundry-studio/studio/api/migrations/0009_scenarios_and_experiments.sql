-- Spec 07 section 3.1's tables: scenarios, their versions and the experiments prepared from them. A scenario states
-- the work a site must do, as a workload spec the generator makes orders and stock from on demand (ruling D96: no
-- generated data is stored anywhere) and the run controls; an experiment fixes, once, the runs one submission would
-- launch. The transactions that write these rows keep spec 07's invariants (one open version, the latest; an edit
-- under the version row's lock); the triggers below refuse what no transaction may do: change a sealed version, or
-- change an experiment at all. A run names its experiment in spec 08's tables. Every `*_at` column is wall clock.

CREATE TABLE scenarios (
    -- A ULID, minted at random.
    id             TEXT        PRIMARY KEY,
    project        TEXT        NOT NULL REFERENCES projects (id),
    name           TEXT        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    description    TEXT        NOT NULL DEFAULT '' CHECK (char_length(description) <= 2000),
    -- The site's root scope path, customer/{c}/site/{s}: the conversation's site at creation (ruling D94), never
    -- changed.
    site_scope     TEXT        NOT NULL,
    latest_version INTEGER     NOT NULL DEFAULT 1 CHECK (latest_version >= 1),
    created_by     TEXT        NOT NULL REFERENCES people (subject),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The last change to the scenario or any of its versions, which the site's list shows; not in spec 07's table.
    changed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_by     TEXT        REFERENCES people (subject),
    deleted_at     TIMESTAMPTZ,
    CHECK ((deleted_by IS NULL) = (deleted_at IS NULL))
);

-- A name is unique among the site's undeleted scenarios, compared case-insensitively.
CREATE UNIQUE INDEX scenarios_one_name_per_site ON scenarios (site_scope, lower(name)) WHERE deleted_at IS NULL;
CREATE INDEX scenarios_by_site ON scenarios (site_scope, changed_at DESC);

CREATE TABLE scenario_versions (
    scenario     TEXT        NOT NULL REFERENCES scenarios (id),
    version      INTEGER     NOT NULL CHECK (version >= 1),
    state        TEXT        NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'sealed')),
    -- Grows by one with each accepted edit of an open version.
    revision     INTEGER     NOT NULL DEFAULT 1 CHECK (revision >= 1),
    -- Spec 07 section 3.2's workload spec and run controls: parameters and seeds, never generated data.
    workload     JSONB       NOT NULL,
    run_controls JSONB       NOT NULL,
    -- Per JSON pointer, the last change of that field: who, in which turn, when and at which revision.
    set_by       JSONB       NOT NULL,
    created_by   TEXT        NOT NULL REFERENCES people (subject),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    sealed_at    TIMESTAMPTZ,
    -- The experiment whose submission sealed this version; its reference arrives with that table, below.
    sealed_by    TEXT,
    PRIMARY KEY (scenario, version),
    CHECK ((state = 'sealed') = (sealed_at IS NOT NULL)),
    CHECK ((sealed_at IS NULL) = (sealed_by IS NULL))
);

-- At most one open version per scenario; the transactions open only the latest.
CREATE UNIQUE INDEX scenario_versions_one_open ON scenario_versions (scenario) WHERE state = 'open';

CREATE FUNCTION scenario_versions_sealed_never_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' OR OLD.state = 'sealed' THEN
        RAISE EXCEPTION 'a sealed scenario version never changes, and no version is removed';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER scenario_versions_sealed_never_change BEFORE UPDATE OR DELETE ON scenario_versions
    FOR EACH ROW EXECUTE FUNCTION scenario_versions_sealed_never_change();

CREATE TABLE experiments (
    -- A ULID, minted at random.
    id            TEXT        PRIMARY KEY,
    scenario      TEXT        NOT NULL,
    version       INTEGER     NOT NULL,
    revision      INTEGER     NOT NULL CHECK (revision >= 1),
    -- The site's live conversation when it was prepared (ruling D94), if one was open.
    conversation  TEXT        REFERENCES conversations (id),
    prepared_by   TEXT        NOT NULL REFERENCES people (subject),
    -- The agent's turn that prepared it; its reference to turns arrives with the agent loop's migration.
    prepared_turn TEXT,
    prepared_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The varied field and its values, kept for display; NULL for a single run and its repeats.
    sweep         JSONB,
    -- The expanded variants, derived from `sweep` and never authored: what runs.
    variants      JSONB       NOT NULL,
    -- The simulation seeds, run_controls.seed then the next, one per repeat; each exact in JSON.
    seeds         BIGINT[]    NOT NULL CHECK (
        cardinality(seeds) BETWEEN 1 AND 10 AND 0 <= ALL (seeds) AND 9007199254740991 >= ALL (seeds)
    ),
    run_overrides JSONB       NOT NULL DEFAULT '{}',
    FOREIGN KEY (scenario, version) REFERENCES scenario_versions (scenario, version)
);

CREATE INDEX experiments_by_version ON experiments (scenario, version);

ALTER TABLE scenario_versions ADD CONSTRAINT scenario_versions_sealed_by_fkey
    FOREIGN KEY (sealed_by) REFERENCES experiments (id);

-- Spec 08 needs an experiment unchangeable once a plan names it, so none ever changes.
CREATE FUNCTION experiments_never_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'an experiment never changes after it is created';
END;
$$;

CREATE TRIGGER experiments_never_change BEFORE UPDATE OR DELETE ON experiments
    FOR EACH ROW EXECUTE FUNCTION experiments_never_change();
