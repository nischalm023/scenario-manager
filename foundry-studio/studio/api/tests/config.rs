//! Spec 01 section 3.1, the configuration, and spec 04 section 3.12's WPS variables, read from a table of variables
//! rather than the process environment.

use std::path::Path;

use kube_invoker::KubeSetup;
use run_invocation::RunnerSetting;
use rustc_hash::FxHashMap;
use studio_api::config::{Config, ConfigError, Deployment, Effort, ModelTransport, ThinkingDisplay, WpsRoutes};
use studio_api::runs::cluster_setup;

fn read(vars: &[(&'static str, &str)]) -> Result<Config, ConfigError> {
    let vars: FxHashMap<&str, String> = vars.iter().map(|(var, value)| (*var, (*value).to_owned())).collect();
    Config::from_vars(|var| Ok(vars.get(var).cloned()))
}

/// The least a developer's machine needs: the database, the runner image and the model, here through a local key.
const LOCAL: [(&str, &str); 7] = [
    ("STUDIO_ENV", "local"),
    ("STUDIO_DATABASE_URL", "postgres://studio:hunter2@127.0.0.1/studio"),
    ("STUDIO_RUNNER_IMAGE", "studio-simrunner:dev"),
    ("STUDIO_MODEL_ID", "claude-sonnet-5-5"),
    ("STUDIO_MODEL_LABEL", "Claude Sonnet 5.5"),
    ("STUDIO_MODEL_TRANSPORT", "anthropic"),
    ("ANTHROPIC_API_KEY", "sk-ant-test-key"),
];

/// The least a deployment needs: the database, WPS, the runners' listener as runners reach it and the model through
/// Vertex AI.
const CLUSTER: [(&str, &str); 9] = [
    ("STUDIO_ENV", "cluster"),
    ("STUDIO_DATABASE_URL", "postgres://studio@db/studio"),
    ("STUDIO_WPS_URL", "http://world-proto-server:7413"),
    ("STUDIO_RUNNER_URL", "http://studio-api-runners:3101"),
    ("STUDIO_RUNNER_IMAGE", "us-docker.pkg.dev/greymatter-development/apps/studio-simrunner@sha256:0f0f"),
    ("STUDIO_MODEL_ID", "claude-sonnet-5-5"),
    ("STUDIO_MODEL_LABEL", "Claude Sonnet 5.5"),
    ("STUDIO_VERTEX_PROJECT", "agent-studio"),
    ("STUDIO_VERTEX_REGION", "global"),
];

fn with(
    base: &[(&'static str, &'static str)],
    more: &[(&'static str, &'static str)],
) -> Vec<(&'static str, &'static str)> {
    let mut vars: Vec<_> =
        base.iter().filter(|(var, _)| !more.iter().any(|(other, _)| other == var)).copied().collect();
    vars.extend_from_slice(more);
    vars
}

/// Spec 01 section 3.1's defaults, and spec 04 section 3.12's: WPS at its own local address, routes spelled `path`,
/// and no revision named, so the client's probe alone says what the server has landed.
#[test]
fn a_local_service_starts_with_spec_01s_defaults() {
    let config = read(&LOCAL).expect("a local service starts");
    assert_eq!(config.deployment, Deployment::Local);
    assert_eq!(
        (config.bind.to_string(), config.runner_bind.to_string()),
        ("127.0.0.1:3100".into(), "127.0.0.1:3101".into())
    );
    assert_eq!(config.identity.headers.subject.as_str(), "x-auth-request-user");
    assert_eq!(config.identity.headers.roles.as_str(), "x-auth-request-groups");
    assert_eq!(config.identity.developer_role, "studio-developer");
    assert!(config.identity.developers.is_empty() && config.identity.dev_identity.is_none());
    assert_eq!((config.wps.url.to_string(), config.wps.routes), ("http://127.0.0.1:7413/".into(), WpsRoutes::Path));
    assert_eq!(config.wps.revision, None);
    let built = read(&with(&LOCAL, &[("STUDIO_WPS_REVISION", " 60aa424 ")])).expect("a revision dev.sh names");
    assert_eq!(built.wps.revision.as_deref(), Some("60aa424"));
    assert_eq!((config.model.effort, config.model.thinking_display), (Effort::Medium, ThinkingDisplay::Omitted));
    assert_eq!(config.model.max_tokens.get(), 32_000);
    assert_eq!((config.compute.workers.get(), config.compute.queue.get()), (2, 64), "spec 01 section 3.15's pool");
    assert!(matches!(config.model.transport, ModelTransport::Anthropic { .. }));
}

/// Spec 01 section 8, statement 10: `STUDIO_DEVELOPERS`, the development identity (`STUDIO_DEV_IDENTITY` here) or
/// a key-based transport, `anthropic` or `gemini` (brief I1.03 section 2), with `STUDIO_ENV=cluster` refuses the start.
#[test]
fn a_cluster_refuses_local_identity_and_the_key_based_transports() {
    let config = read(&CLUSTER).expect("a deployment starts");
    assert!(matches!(config.model.transport, ModelTransport::Vertex { .. }));
    let refusals = [
        ("STUDIO_DEVELOPERS", "u-1"),
        ("STUDIO_DEV_IDENTITY", "u-1,u@example.com,U"),
        ("STUDIO_MODEL_TRANSPORT", "anthropic"),
        ("STUDIO_MODEL_TRANSPORT", "gemini"),
    ];
    for (var, value) in refusals {
        let keys = [("ANTHROPIC_API_KEY", "sk-ant-test-key"), ("STUDIO_GEMINI_API_KEY_FILE", "/nonexistent/keys.txt")];
        match read(&with(&CLUSTER, &[(var, value), keys[0], keys[1]])) {
            Err(ConfigError::RefusedInCluster { var: refused, .. }) => assert_eq!(refused, var),
            other => panic!("{var} with STUDIO_ENV=cluster: {other:?}"),
        }
    }
}

/// Spec 01 section 3.1: a value that does not parse refuses the start instead of guessing, and so does a required one
/// that is missing; spec 04 section 3.12: `STUDIO_WPS_ROUTES` accepts `path` alone, the retired `segments` refused.
#[test]
fn a_value_that_does_not_parse_or_is_missing_refuses_the_start() {
    let cases: [(&[(&str, &str)], &str); 14] = [
        (&[("STUDIO_BIND", "port 3100")], "Unreadable"),
        (&[("STUDIO_BIND", " ")], "Blank"),
        (&[("STUDIO_ENV", "staging")], "Unreadable"),
        (&[("STUDIO_IDENTITY_HEADERS", "x-user,x-email,x-name")], "Unreadable"),
        (&[("STUDIO_DEV_IDENTITY", "dev-1,dev@example.com")], "Unreadable"),
        (&[("STUDIO_WPS_URL", "ftp://world-proto-server")], "Unreadable"),
        (&[("STUDIO_WPS_ROUTES", "segments")], "Unreadable"),
        (&[("STUDIO_WPS_REVISION", "")], "Blank"),
        (&[("STUDIO_COMPUTE_WORKERS", "0")], "Unreadable"),
        (&[("STUDIO_COMPUTE_QUEUE", "-1")], "Unreadable"),
        (&[("STUDIO_MODEL_EFFORT", "extreme")], "Unreadable"),
        (&[("STUDIO_MODEL_MAX_TOKENS", "0")], "Unreadable"),
        (&[("STUDIO_MODEL_TRANSPORT", "vertex")], "RequiredWhen"),
        (&[("STUDIO_MODEL_TRANSPORT", "openai")], "Unreadable"),
    ];
    for (more, refusal) in cases {
        let outcome = read(&with(&LOCAL, more));
        let named = format!("{outcome:?}");
        assert!(named.starts_with(&format!("Err({refusal}")), "{more:?} gave {named}");
    }
    let without_env: Vec<_> = LOCAL.iter().filter(|(var, _)| *var != "STUDIO_ENV").copied().collect();
    assert!(matches!(read(&without_env), Err(ConfigError::Missing("STUDIO_ENV"))));
    let without_wps: Vec<_> = CLUSTER.iter().filter(|(var, _)| *var != "STUDIO_WPS_URL").copied().collect();
    assert!(matches!(read(&without_wps), Err(ConfigError::RequiredWhen { var: "STUDIO_WPS_URL", .. })));
}

/// Spec 08 section 6's run settings: five seats, a 6 h deadline, 15 min to start, 60 s of cancel grace and one core by
/// default, runners reaching a local service at its runners' listener; a cluster needs that address stated and
/// refuses the local runner's settings; a value that does not parse refuses the start.
#[test]
fn the_run_settings_take_spec_08s_defaults_and_refuse_what_does_not_parse() {
    let runs = read(&LOCAL).expect("a local service starts").runs;
    assert_eq!(runs.at_once.0, 5);
    assert_eq!((runs.deadline.as_secs(), runs.start_timeout.as_secs(), runs.cancel_grace.as_secs()), (21_600, 900, 60));
    assert_eq!(runs.streaming_stale_after.as_secs(), 10, "a streaming server is dead 10 s after its last heartbeat");
    assert_eq!((runs.cpu_millis.0, runs.memory_mib), (1_000, None));
    assert_eq!(runs.runner_url, "http://127.0.0.1:3101");
    assert_eq!((runs.runner_out, runs.runner_wps_url, runs.container_engine), (None, None, None));
    assert_eq!(runs.runner_image, "studio-simrunner:dev");
    let stated = read(&with(
        &LOCAL,
        &[
            ("STUDIO_RUNS_AT_ONCE", "2"),
            ("STUDIO_RUNNER_OUT_DIR", "/var/runs"),
            ("STUDIO_RUNNER_WPS_URL", "http://host.docker.internal:7413/"),
            ("STUDIO_CONTAINER_ENGINE", "/opt/bin/podman"),
        ],
    ))
    .expect("stated run settings");
    assert_eq!((stated.runs.at_once.0, stated.runs.runner_out.as_deref()), (2, Some(Path::new("/var/runs"))));
    assert_eq!(stated.runs.runner_wps_url.as_deref(), Some("http://host.docker.internal:7413"));
    assert_eq!(stated.runs.container_engine.as_deref(), Some(Path::new("/opt/bin/podman")));
    let without: Vec<_> = LOCAL.iter().filter(|(var, _)| *var != "STUDIO_RUNNER_IMAGE").copied().collect();
    assert!(matches!(read(&without), Err(ConfigError::Missing("STUDIO_RUNNER_IMAGE"))));
    match read(&with(&LOCAL, &[("STUDIO_RUNNER_BIN", "/opt/simrunner")])) {
        Err(ConfigError::Gone { var, .. }) => assert_eq!(var, "STUDIO_RUNNER_BIN"),
        other => panic!("the process invoker's binary, gone with it: {other:?}"),
    }
    for (var, value) in [
        ("STUDIO_RUNS_AT_ONCE", "0"),
        ("STUDIO_RUN_DEADLINE_SECS", "six hours"),
        ("STUDIO_RUNNER_CPU_MILLIS", "500"),
        ("STUDIO_RUNNER_URL", "ftp://runners"),
        ("STUDIO_RUNNER_WPS_URL", "wps:7413"),
    ] {
        let outcome = format!("{:?}", read(&with(&LOCAL, &[(var, value)])));
        assert!(outcome.starts_with("Err(Unreadable"), "{var}={value} gave {outcome}");
    }
    let without: Vec<_> = CLUSTER.iter().filter(|(var, _)| *var != "STUDIO_RUNNER_URL").copied().collect();
    assert!(matches!(read(&without), Err(ConfigError::RequiredWhen { var: "STUDIO_RUNNER_URL", .. })));
    for var in ["STUDIO_RUNNER_OUT_DIR", "STUDIO_CONTAINER_ENGINE", "STUDIO_CONTAINER_USER"] {
        match read(&with(&CLUSTER, &[(var, "x")])) {
            Err(ConfigError::RefusedInCluster { var: refused, .. }) => assert_eq!(refused, var),
            other => panic!("{var} in a cluster: {other:?}"),
        }
    }
    // The cluster's runner settings: the image required there too, the rest optional, none of them local.
    let without: Vec<_> = CLUSTER.iter().filter(|(var, _)| *var != "STUDIO_RUNNER_IMAGE").copied().collect();
    assert!(matches!(read(&without), Err(ConfigError::Missing("STUDIO_RUNNER_IMAGE"))));
    let cluster = read(&with(
        &CLUSTER,
        &[
            ("STUDIO_RUNNER_NAMESPACE", "studio"),
            ("STUDIO_RUNNER_SERVICE_ACCOUNT", "studio-runner"),
            ("STUDIO_RUNNER_PASSED", "DES_HEARTBEAT, DES_DIGEST"),
            ("DES_HEARTBEAT", "3600"),
            ("DES_DIGEST", "1"),
            ("STUDIO_BUILD_BRANCH", "main"),
        ],
    ))
    .expect("a cluster with its runner settings")
    .runs;
    assert!(cluster.runner_image.ends_with("@sha256:0f0f"), "{}", cluster.runner_image);
    assert_eq!(cluster.build_branch.as_deref(), Some("main"));
    assert_eq!(cluster.runner_namespace.as_deref(), Some("studio"));
    assert_eq!(cluster.runner_service_account.as_deref(), Some("studio-runner"));
    let passed: Vec<(&str, &str)> =
        cluster.runner_passed.iter().map(|setting| (setting.name(), setting.value())).collect();
    assert_eq!(passed, [("DES_HEARTBEAT", "3600"), ("DES_DIGEST", "1")], "each name with its value here");
    // A name this environment does not set is refused at the start, not when the first runner lacks it.
    let outcome = format!("{:?}", read(&with(&CLUSTER, &[("STUDIO_RUNNER_PASSED", "DES_HEARTBEAT")])));
    assert!(outcome.starts_with("Err(Unreadable") && outcome.contains("DES_HEARTBEAT is not"), "{outcome}");
    let outcome = format!("{:?}", read(&with(&CLUSTER, &[("STUDIO_RUNNER_PASSED", "DES_HEARTBEAT,,DES_DIGEST")])));
    assert!(outcome.starts_with("Err(Unreadable"), "{outcome}");
    for var in ["STUDIO_RUNNER_NAMESPACE", "STUDIO_RUNNER_SERVICE_ACCOUNT"] {
        match read(&with(&LOCAL, &[(var, "x")])) {
            Err(ConfigError::RefusedLocally { var: refused, .. }) => assert_eq!(refused, var),
            other => panic!("{var} locally: {other:?}"),
        }
    }
}

/// Ruling D174, the simulation runner has no access to Postgres, held by what a runner can be given at all: only the
/// runner's own settings, so a store credential and the API's database URL under its own name or another each refuse
/// the start rather than reach a runner, in a cluster and locally alike. No Secret's keys reach a runner either: the
/// Job names its environment variable by variable (`kube-invoker`'s `job` tests).
#[test]
fn no_database_url_or_store_credential_can_be_handed_to_a_runner() {
    let credentials = [
        ("GOOGLE_SERVICE_ACCOUNT_KEY", "{\"type\": \"service_account\"}"),
        ("STUDIO_DATABASE_URL", "postgres://studio@db/studio"),
        ("ANALYTICS_DB", "postgres://studio@db/studio"),
        ("PGPASSWORD", "hunter2"),
        ("CRUCIBLE_STORAGE_BUCKET", "studio-runs"),
    ];
    for base in [&LOCAL[..], &CLUSTER[..]] {
        for (name, value) in credentials {
            let vars = with(base, &[("STUDIO_RUNNER_PASSED", name), (name, value)]);
            let outcome = format!("{:?}", read(&vars));
            assert!(
                outcome.starts_with("Err(Unreadable") && outcome.contains("each one of DES_HEARTBEAT"),
                "{name} handed to runners gave {outcome}"
            );
        }
    }
}

/// Ruling D174 for a Secret: naming the API's own Secret in `STUDIO_RUNNER_SECRET` imports none of its keys into a
/// Job. The setup every Job is built from stays the runners' namespace and the settings `STUDIO_RUNNER_PASSED` names;
/// kube-invoker's own tests show that a Job's environment is those settings, its token and its thread count alone.
#[test]
fn a_secret_named_for_runners_reaches_no_job() {
    let vars = with(
        &CLUSTER,
        &[
            ("STUDIO_RUNNER_SECRET", "studio-api-credentials"),
            ("STUDIO_RUNNER_NAMESPACE", "studio-runs"),
            ("STUDIO_RUNNER_PASSED", "DES_HEARTBEAT"),
            ("DES_HEARTBEAT", "3600"),
        ],
    );
    let config = read(&vars).expect("a cluster service starts");
    let setup = cluster_setup(&config.runs).expect("the runners' namespace is named");
    let passed = vec![RunnerSetting::new("DES_HEARTBEAT", "3600").expect("a runner setting")];
    assert_eq!(setup, KubeSetup { passed, ..KubeSetup::new("studio-runs") });
}

/// Spec 01 section 3.1: the configuration names variables, never secret values; a key and the database URL's
/// password never reach Debug output.
#[test]
fn a_secret_never_reaches_debug_output() {
    let shown = format!("{:?}", read(&LOCAL).expect("a local service starts"));
    assert!(!shown.contains("hunter2") && !shown.contains("sk-ant-test-key"), "{shown}");
}
