//! Spec 01 section 3.3, the service's tables as its embedded migrations make them, and the invariants the tables
//! keep themselves.

mod support;

use std::env;
use std::fs;
use std::path::Path;

use sqlx::migrate::Migrator;
use sqlx::{Error as SqlxError, PgPool};
use studio_protocol::JournalKind;

use support::Scratch;

/// The name of the constraint `outcome` broke, if it was one.
fn broken_constraint<T>(outcome: Result<T, SqlxError>) -> Option<String> {
    outcome.err()?.as_database_error()?.constraint().map(str::to_owned)
}

async fn person(pool: &PgPool, subject: &str) {
    sqlx::query("INSERT INTO people (subject, email, name) VALUES ($1, $1 || '@example.com', $1)")
        .bind(subject)
        .execute(pool)
        .await
        .expect("a person is recorded");
}

async fn conversation(pool: &PgPool, id: &str, site: Option<&str>, state: &str, by: &str) -> Result<(), SqlxError> {
    sqlx::query("INSERT INTO projects (id, name) VALUES ('p-1', 'Tests') ON CONFLICT DO NOTHING").execute(pool).await?;
    sqlx::query("INSERT INTO conversations (id, project, site, state, created_by) VALUES ($1, 'p-1', $2, $3, $4)")
        .bind(id)
        .bind(site)
        .bind(state)
        .bind(by)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Spec 01 section 3.3: the embedded migrations bring an empty database up to this build's tables, and a second run
/// finds nothing to apply.
#[tokio::test]
async fn the_migrations_apply_to_an_empty_schema_and_again_to_no_effect() {
    let scratch = scratch_or_skip!();
    studio_api::db::migrate(&scratch.pool).await.expect("a second run finds nothing to apply");
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = current_schema() \
         ORDER BY table_name",
    )
    .fetch_all(&scratch.pool)
    .await
    .expect("the schema lists its tables");
    // Every table of every migration the build embeds, 0008 on included, and how many migrations ran.
    assert_eq!(
        tables,
        [
            "_sqlx_migrations",
            "approvals",
            "attempt_metrics",
            "attempt_series",
            "attempt_streams",
            "attempt_transitions",
            "attempts",
            "changeset_drafts",
            "conversations",
            "experiments",
            "idempotency_keys",
            "journal",
            "live_runner_tokens",
            "live_watch_tickets",
            "metric_pins",
            "model_calls",
            "model_messages",
            "people",
            "plans",
            "projects",
            "questions",
            "run_event_cursor",
            "runner_tokens",
            "runs",
            "scenario_versions",
            "scenarios",
            "site_made_from",
            "stream_watch_tickets",
            "streaming_servers",
            "submissions",
            "template_draft_ids",
            "template_drafts",
            "template_evaluations",
            "template_thumbnails",
            "tool_calls",
            "turns"
        ]
    );
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&scratch.pool)
        .await
        .expect("the migrations are recorded");
    assert_eq!(applied, 27);
    scratch.close().await;
}

/// Spec 01 section 3.17: one live conversation per site and one live new-site thread per person, held by the
/// table's unique indexes (the half of section 8's statement 16 the database keeps).
#[tokio::test]
async fn a_site_holds_one_live_conversation_and_a_person_one_live_new_site_thread() {
    let scratch = scratch_or_skip!();
    let pool = &scratch.pool;
    person(pool, "u-1").await;
    let site = Some("customer/demo/site/lshape-6");
    conversation(pool, "01KN0TJZANP5N0PGJD8F7A3FD1", site, "live", "u-1").await.expect("the site's live conversation");
    let second = conversation(pool, "01KN0TJZANP5N0PGJD8F7A3FD2", site, "live", "u-1").await;
    assert_eq!(broken_constraint(second).as_deref(), Some("conversations_one_live_per_site"));
    conversation(pool, "01KN0TJZANP5N0PGJD8F7A3FD3", site, "archived", "u-1").await.expect("archived ones may be many");

    conversation(pool, "01KN0TJZANP5N0PGJD8F7A3FD4", None, "live", "u-1").await.expect("a new-site thread");
    let second = conversation(pool, "01KN0TJZANP5N0PGJD8F7A3FD5", None, "live", "u-1").await;
    assert_eq!(broken_constraint(second).as_deref(), Some("conversations_one_new_site_thread_per_person"));
    conversation(pool, "01KN0TJZANP5N0PGJD8F7A3FD6", None, "discarded", "u-1").await.expect("a discarded thread");
    scratch.close().await;
}

/// Spec 01 section 8, statement 1: the journal takes inserts only (a trigger); and section 3.5: it takes every kind
/// `studio-protocol` names and no other, one entry per dedup key.
#[tokio::test]
async fn the_journal_takes_each_kind_once_per_dedup_key_and_inserts_only() {
    let scratch = scratch_or_skip!();
    let pool = &scratch.pool;
    person(pool, "u-1").await;
    let id = "01KN0TJZANP5N0PGJD8F7A3FD1";
    conversation(pool, id, Some("customer/demo/site/lshape-6"), "live", "u-1").await.expect("a conversation");
    let entry = |seq: i64, kind: &'static str, dedup_key: Option<&'static str>| {
        sqlx::query("INSERT INTO journal (conversation, seq, kind, body, dedup_key) VALUES ($1, $2, $3, '{}', $4)")
            .bind(id)
            .bind(seq)
            .bind(kind)
            .bind(dedup_key)
            .execute(pool)
    };
    for (seq, kind) in (1..).zip(JournalKind::ALL) {
        entry(seq, kind.as_str(), None).await.unwrap_or_else(|fault| panic!("the journal takes {kind:?}: {fault}"));
    }
    let next = JournalKind::ALL.len() as i64 + 1;
    assert_eq!(broken_constraint(entry(next, "text.delta", None).await).as_deref(), Some("journal_kind_check"));
    entry(next, "run.status", Some("run.status:r:1:running")).await.expect("a deduplicated entry");
    let repeat = entry(next + 1, "run.status", Some("run.status:r:1:running")).await;
    assert_eq!(broken_constraint(repeat).as_deref(), Some("journal_one_entry_per_dedup_key"));

    for change in ["UPDATE journal SET body = '{\"changed\": true}'", "DELETE FROM journal", "TRUNCATE journal"] {
        let refused = sqlx::query(change).execute(pool).await.expect_err("the journal takes inserts only");
        assert!(refused.to_string().contains("the journal takes inserts only"), "{change}: {refused}");
    }
    scratch.close().await;
}

/// Spec 05 section 3.7 and spec 01 section 3.3 (migration 0008): the template tables and approvals exist; an
/// approval keeps the input it approved; a draft numbers each evaluation once.
#[tokio::test]
async fn the_template_tables_keep_their_invariants() {
    let scratch = scratch_or_skip!();
    let pool = &scratch.pool;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = current_schema() \
         AND table_name IN ('approvals', 'template_drafts', 'template_draft_ids', 'template_evaluations', \
             'template_thumbnails') ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .expect("the schema lists its tables");
    assert_eq!(tables.len(), 5, "{tables:?}");
    person(pool, "u-1").await;
    sqlx::query(
        "INSERT INTO approvals (id, tool, class, input, input_digest, summary, facts, via, state, execution) \
         VALUES ('a-1', 'site.create_from_template', 'apply', '{}', 'd', 's', '{}', 'button', 'approved', 'running')",
    )
    .execute(pool)
    .await
    .expect("a button's approval");
    let changed = sqlx::query("UPDATE approvals SET input = '{\"other\": 1}' WHERE id = 'a-1'").execute(pool).await;
    assert!(changed.expect_err("the input is fixed").to_string().contains("keeps the input it approved"));
    sqlx::query(
        "INSERT INTO template_drafts (id, template, template_seqs, doc, title, declarations, knobs, set_by, created_by) \
         VALUES ('d-1', 'template/site/l-shape', '{}', 'doc', 'L-Shape', '[]', '{}', '{}', 'u-1')",
    )
    .execute(pool)
    .await
    .expect("a draft");
    let evaluation =
        || sqlx::query("INSERT INTO template_evaluations (draft, n, revision, knobs) VALUES ('d-1', 1, 1, '{}')");
    evaluation().execute(pool).await.expect("evaluation 1");
    assert_eq!(broken_constraint(evaluation().execute(pool).await).as_deref(), Some("template_evaluations_pkey"));
    scratch.close().await;
}

/// Migration 0013: Create site writes a site in one changeset (spec 05 section 4.4), so a made-from record is never
/// `creating` or `abandoned` and keeps no cleanup changeset.
#[tokio::test]
async fn the_made_from_record_keeps_no_staged_state() {
    let scratch = scratch_or_skip!();
    let pool = &scratch.pool;
    let insert = |id: &'static str, state: &'static str| {
        sqlx::query("INSERT INTO site_made_from (id, site, state) VALUES ($1, 'customer/demo/site/a', $2)")
            .bind(id)
            .bind(state)
            .execute(pool)
    };
    for (id, state) in [("m-1", "creating"), ("m-2", "abandoned")] {
        assert_eq!(
            broken_constraint(insert(id, state).await).as_deref(),
            Some("site_made_from_state_check"),
            "{state}"
        );
    }
    insert("m-3", "made").await.expect("a made act");
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'site_made_from'",
    )
    .fetch_all(pool)
    .await
    .expect("the table lists its columns");
    assert!(!columns.iter().any(|column| column == "cleaned_root_seq"), "{columns:?}");
    scratch.close().await;
}

/// Migration 0015: one renderer (ruling D121), so neither an evaluation nor a thumbnail keeps a drawing's key; each
/// keeps its floor document's, which the page's viewer draws.
#[tokio::test]
async fn the_template_tables_keep_floors_and_no_drawings() {
    let scratch = scratch_or_skip!();
    let columns = |table: &'static str| {
        sqlx::query_scalar::<_, String>(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
        )
        .bind(table)
        .fetch_all(&scratch.pool)
    };
    let thumbnails = columns("template_thumbnails").await.expect("the thumbnails' columns");
    let wanted =
        ["id", "template", "template_seqs", "engine", "generators", "state", "census", "problems", "floor_key", "at"];
    assert_eq!(thumbnails, wanted);
    let evaluations = columns("template_evaluations").await.expect("the evaluations' columns");
    let keys: Vec<&str> = evaluations.iter().map(String::as_str).filter(|column| column.ends_with("_key")).collect();
    assert_eq!(keys, ["plan_key", "floor_key"]);
    scratch.close().await;
}

/// Migration 0016: an evaluation keeps the axes of the frame its template's wall knobs name (spec 05 section 6).
#[tokio::test]
async fn an_evaluation_keeps_its_frames_axes() {
    let scratch = scratch_or_skip!();
    let axes: Option<String> = sqlx::query_scalar(
        "SELECT data_type::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'template_evaluations' AND column_name = 'axes'",
    )
    .fetch_optional(&scratch.pool)
    .await
    .expect("the evaluations' columns");
    assert_eq!(axes.as_deref(), Some("jsonb"));
    scratch.close().await;
}

async fn scenario(pool: &PgPool, id: &str, name: &str) -> Result<(), SqlxError> {
    sqlx::query("INSERT INTO projects (id, name) VALUES ('p-1', 'Tests') ON CONFLICT DO NOTHING").execute(pool).await?;
    sqlx::query(
        "INSERT INTO scenarios (id, project, name, site_scope, created_by) \
         VALUES ($1, 'p-1', $2, 'customer/demo/site/lshape-6', 'u-1')",
    )
    .bind(id)
    .bind(name)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn version(pool: &PgPool, scenario: &str, version: i32, state: &str) -> Result<(), SqlxError> {
    let sealed = if state == "sealed" { "now()" } else { "NULL" };
    sqlx::query(&format!(
        "INSERT INTO scenario_versions (scenario, version, state, workload, run_controls, set_by, created_by, sealed_at) \
         VALUES ($1, $2, $3, '{{}}', '{{}}', '{{}}', 'u-1', {sealed})"
    ))
    .bind(scenario)
    .bind(version)
    .bind(state)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Spec 07 section 3.1's invariants the tables keep themselves: a name once per site among undeleted scenarios,
/// compared case-insensitively; one open version per scenario; a sealed version never changes, and no version is
/// removed; an experiment never changes.
#[tokio::test]
async fn scenario_names_open_versions_sealed_versions_and_experiments_keep_their_invariants() {
    let scratch = scratch_or_skip!();
    let pool = &scratch.pool;
    person(pool, "u-1").await;
    scenario(pool, "01KN0TJZANP5N0PGJD8F7A3FS1", "Peak hour").await.expect("a scenario");
    let twin = scenario(pool, "01KN0TJZANP5N0PGJD8F7A3FS2", "peak HOUR").await;
    assert_eq!(broken_constraint(twin).as_deref(), Some("scenarios_one_name_per_site"));
    sqlx::query("UPDATE scenarios SET deleted_by = 'u-1', deleted_at = now() WHERE id = '01KN0TJZANP5N0PGJD8F7A3FS1'")
        .execute(pool)
        .await
        .expect("a delete marks the row");
    scenario(pool, "01KN0TJZANP5N0PGJD8F7A3FS2", "Peak hour").await.expect("a deleted scenario's name is free");

    let id = "01KN0TJZANP5N0PGJD8F7A3FS2";
    version(pool, id, 1, "open").await.expect("version 1, open");
    assert_eq!(broken_constraint(version(pool, id, 2, "open").await).as_deref(), Some("scenario_versions_one_open"));
    sqlx::query(
        "INSERT INTO experiments (id, scenario, version, revision, prepared_by, variants, seeds) \
                 VALUES ('01KN0TJZANP5N0PGJD8F7A3FE1', $1, 1, 1, 'u-1', '[]', '{0}')",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("an experiment");
    sqlx::query(
        "UPDATE scenario_versions SET state = 'sealed', sealed_at = now(), sealed_by = '01KN0TJZANP5N0PGJD8F7A3FE1' \
                 WHERE scenario = $1 AND version = 1",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("a submission seals the open version");
    for change in [
        "UPDATE scenario_versions SET revision = 2",
        "DELETE FROM scenario_versions",
        "UPDATE experiments SET revision = 2",
        "DELETE FROM experiments",
    ] {
        let refused = sqlx::query(change).execute(pool).await.expect_err("refused");
        assert!(refused.to_string().contains("never change"), "{change}: {refused}");
    }
    scratch.close().await;
}

/// Migration 0021: the endings of Job creations that failed, written before an ending carried a typed cause, are given
/// that cause by the words the dispatcher began them with, which were their only mark; no other ending is.
#[tokio::test]
async fn the_creation_failures_written_before_their_cause_are_given_it() {
    let Some(scratch) = Scratch::bare().await else {
        return eprintln!("skipped: set {} to a PostgreSQL the tests may create schemas in", support::DATABASE_VAR);
    };
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let before = env::temp_dir().join(format!("studio-migrations-{}", support::ulid()));
    fs::create_dir_all(&before).expect("a folder for the migrations before 0021");
    for file in fs::read_dir(&migrations).expect("the migrations").map(|file| file.expect("a migration").path()) {
        let name = file.file_name().expect("a name").to_string_lossy().into_owned();
        if name.as_str() < "0021" {
            fs::copy(&file, before.join(&name)).expect("a migration copied");
        }
    }
    Migrator::new(before.as_path()).await.expect("the migrations before 0021").run(&scratch.pool).await.unwrap();
    let mut conn = scratch.pool.acquire().await.expect("a connection");
    // The rows alone, without the runs and submissions their keys name.
    sqlx::query("SET session_replication_role = replica").execute(&mut *conn).await.unwrap();
    for (run, ended_by, stop_reason) in [
        ("01KZAVPQNES65HV4EZADPCS7E1", "platform", "its Job could not be created: the webhook refused it"),
        ("01KZAVPQNES65HV4EZADPCS7E2", "platform", "the runner never started within 15 min"),
        ("01KZAVPQNES65HV4EZADPCS7E3", "simulator", "its Job could not be created, said a simulator"),
    ] {
        sqlx::query(
            "INSERT INTO attempts (run_id, n, state, requested_at, ended_at, ended_by, stop_reason) \
             VALUES ($1, 1, 'failed', now(), now(), $2, $3)",
        )
        .bind(run)
        .bind(ended_by)
        .bind(stop_reason)
        .execute(&mut *conn)
        .await
        .expect("an ended attempt");
    }
    drop(conn);
    studio_api::db::migrate(&scratch.pool).await.expect("0021 applies over them");
    let causes: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT run_id, ending_cause FROM attempts ORDER BY run_id")
            .fetch_all(&scratch.pool)
            .await
            .unwrap();
    let causes: Vec<Option<&str>> = causes.iter().map(|(_, cause)| cause.as_deref()).collect();
    assert_eq!(causes, [Some("job-not-created"), None, None]);
    fs::remove_dir_all(&before).expect("the copied migrations removed");
    scratch.close().await;
}
