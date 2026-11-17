//! The invoker against a real cluster, a kind one will do, when `STUDIO_TEST_KUBECONFIG` names its kubeconfig and
//! `STUDIO_TEST_RUNNER_IMAGE` an image the cluster has that stands in for the runner, sleeping 20 s whatever its
//! arguments; each test returns at once without them. This builds one and hands it to a kind cluster:
//!
//! ```sh
//! printf 'FROM busybox:1.36\nENTRYPOINT ["sh", "-c", "sleep 20", "runner"]\n' | docker build -t studio-test-sleeper:1 -
//! kind load docker-image studio-test-sleeper:1
//! ```
//!
//! `STUDIO_TEST_NAMESPACE` (default `default`) is where the Jobs go. Every wait here is wall clock.

use std::env::var;
use std::time::{Duration, Instant};

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, ListParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};
use kube_invoker::{KubeInvoker, KubeSetup};
use run_invocation::{AttemptKey, CpuMillis, Invoker, JobName, JobRequest, JobStatus, Resources, Token};
use rustc_hash::FxHashMap;
use studio_protocol::{AttemptNumber, Endpoint, FORMAT, JobSpec};
use tokio::time::sleep;

const SKIP: &str = "needs STUDIO_TEST_KUBECONFIG and a cluster (kind)";

struct Cluster {
    client: Client,
    namespace: String,
    invoker: KubeInvoker,
    image: String,
}

/// The cluster `STUDIO_TEST_KUBECONFIG` names, and the image its Jobs run, or none, when the test has nothing to run
/// against.
async fn cluster() -> Option<Cluster> {
    let path = var("STUDIO_TEST_KUBECONFIG").ok()?;
    let image = var("STUDIO_TEST_RUNNER_IMAGE").ok()?;
    let kubeconfig = Kubeconfig::read_from(path).expect("the kubeconfig reads");
    let config = Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default()).await.expect("a config");
    let client = Client::try_from(config).expect("a client");
    let namespace = var("STUDIO_TEST_NAMESPACE").unwrap_or_else(|_| "default".to_owned());
    let mut setup = KubeSetup::new(namespace.clone());
    setup.ttl_after_finished = Duration::from_secs(60);
    // A shell as PID 1 ignores SIGTERM, so a stopped pod runs out this grace before its SIGKILL.
    setup.termination_grace = Duration::from_secs(5);
    let invoker = KubeInvoker::new(client.clone(), setup);
    Some(Cluster { client, namespace, invoker, image })
}

/// A request for attempt 1 of `run`, a ULID that differs per test so the tests' Jobs do not meet, running the
/// cluster's stand-in for the runner.
fn request(cluster: &Cluster, run: &str) -> JobRequest {
    let key = AttemptKey { run: run.parse().expect("a ULID"), n: AttemptNumber(1) };
    let spec = JobSpec {
        v: FORMAT,
        run: key.run.clone(),
        attempt: key.n,
        studio: Endpoint { endpoint: "http://studio:3101".to_owned() },
        streaming: None,
    };
    let labels = FxHashMap::from_iter([
        ("agent-studio/attempt".to_owned(), "1".to_owned()),
        ("agent-studio/run".to_owned(), run.to_owned()),
    ]);
    JobRequest {
        name: JobName::of(&key),
        image: cluster.image.clone(),
        spec,
        token: Token::mint(),
        deadline: Duration::from_secs(300),
        resources: Resources { cpu_millis: CpuMillis(100), memory_mib: None },
        labels,
    }
}

/// Poll `status` until `done` accepts it or `within` passes; the last status answered.
async fn until(
    invoker: &KubeInvoker,
    name: &JobName,
    within: Duration,
    done: impl Fn(&JobStatus) -> bool,
) -> JobStatus {
    let start = Instant::now();
    loop {
        let status = invoker.status(name).await.expect("a status");
        if done(&status) || start.elapsed() >= within {
            return status;
        }
        sleep(Duration::from_secs(1)).await;
    }
}

#[tokio::test]
#[ignore = "needs STUDIO_TEST_KUBECONFIG and a cluster (kind)"]
async fn starting_twice_yields_one_job() {
    let Some(cluster) = cluster().await else { return eprintln!("{SKIP}") };
    let request = request(&cluster, "01KZAVPQNES65HV4EZADPCS7E1");
    cluster.invoker.start(request.clone()).await.expect("the first start");
    cluster.invoker.start(request.clone()).await.expect("the second start");
    let jobs: Api<Job> = Api::namespaced(cluster.client.clone(), &cluster.namespace);
    let selector = format!("agent-studio/run={}", request.labels["agent-studio/run"]);
    let found = jobs.list(&ListParams::default().labels(&selector)).await.expect("a listing");
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].metadata.name.as_deref(), Some(request.name.as_str()));
    cluster.invoker.stop(&request.name).await.expect("the stop");
}

#[tokio::test]
#[ignore = "needs STUDIO_TEST_KUBECONFIG and a cluster (kind)"]
async fn a_job_is_pending_or_running_and_then_ends() {
    let Some(cluster) = cluster().await else { return eprintln!("{SKIP}") };
    let request = request(&cluster, "01KZAVPQNES65HV4EZADPCS7E2");
    cluster.invoker.start(request.clone()).await.expect("the start");
    let first = cluster.invoker.status(&request.name).await.expect("a status");
    assert!(matches!(first, JobStatus::Pending { .. } | JobStatus::Running), "{first:?}");
    let last = until(&cluster.invoker, &request.name, Duration::from_secs(60), JobStatus::stopped).await;
    assert_eq!(last, JobStatus::Ended { reason: None });
    cluster.invoker.stop(&request.name).await.expect("the stop");
}

/// Spec 08 section 3.4: a stopped Job reads as stopped only once no pod of it runs, so its seat never frees while the
/// pod runs out its termination grace; then the pod and the Secret go with the Job.
#[tokio::test]
#[ignore = "needs STUDIO_TEST_KUBECONFIG and a cluster (kind)"]
async fn a_stopped_job_ends_or_goes_and_its_secret_with_it() {
    let Some(cluster) = cluster().await else { return eprintln!("{SKIP}") };
    let request = request(&cluster, "01KZAVPQNES65HV4EZADPCS7E3");
    cluster.invoker.start(request.clone()).await.expect("the start");
    let started =
        until(&cluster.invoker, &request.name, Duration::from_secs(60), |status| *status == JobStatus::Running).await;
    assert_eq!(started, JobStatus::Running, "its pod runs, so the stop below meets a pod with a grace to run out");
    cluster.invoker.stop(&request.name).await.expect("the stop");
    let stopping = cluster.invoker.status(&request.name).await.expect("a status");
    assert!(!stopping.stopped(), "its pod runs out its grace, so the Job is not stopped yet: {stopping:?}");
    let last = until(&cluster.invoker, &request.name, Duration::from_secs(30), JobStatus::stopped).await;
    assert!(last.stopped(), "{last:?}");
    let pods: Api<Pod> = Api::namespaced(cluster.client.clone(), &cluster.namespace);
    let selector = format!("batch.kubernetes.io/job-name={}", request.name);
    let left = pods.list(&ListParams::default().labels(&selector)).await.expect("a listing");
    let ended = |pod: &Pod| {
        matches!(pod.status.as_ref().and_then(|status| status.phase.as_deref()), Some("Succeeded" | "Failed"))
    };
    assert!(left.items.iter().all(ended), "no pod of a stopped Job may run: {:?}", left.items);
    let secrets: Api<Secret> = Api::namespaced(cluster.client.clone(), &cluster.namespace);
    let start = Instant::now();
    while secrets.get_opt(request.name.as_str()).await.expect("a lookup").is_some() {
        assert!(start.elapsed() < Duration::from_secs(30), "the Secret stayed");
        sleep(Duration::from_secs(1)).await;
    }
}

/// A Job the cluster refuses leaves no Secret behind: the Job is created first, and its Secret only once it exists. The
/// refusal is the Job's alone, a pod template whose container names no image, which the Secret does not carry, so a
/// Secret created first would be left.
#[tokio::test]
#[ignore = "needs STUDIO_TEST_KUBECONFIG and a cluster (kind)"]
async fn a_job_the_cluster_refuses_leaves_no_secret() {
    let Some(cluster) = cluster().await else { return eprintln!("{SKIP}") };
    let mut request = request(&cluster, "01KZAVPQNES65HV4EZADPCS7E5");
    // The API server requires a container's image, in a Job's pod template as in a pod.
    request.image = String::new();
    cluster.invoker.start(request.clone()).await.expect_err("the cluster refuses the Job");
    let secrets: Api<Secret> = Api::namespaced(cluster.client.clone(), &cluster.namespace);
    assert!(secrets.get_opt(request.name.as_str()).await.expect("a lookup").is_none(), "no Secret is left");
}

#[tokio::test]
#[ignore = "needs STUDIO_TEST_KUBECONFIG and a cluster (kind)"]
async fn a_name_never_started_is_absent() {
    let Some(cluster) = cluster().await else { return eprintln!("{SKIP}") };
    let request = request(&cluster, "01KZAVPQNES65HV4EZADPCS7E4");
    assert_eq!(cluster.invoker.status(&request.name).await.expect("a status"), JobStatus::Absent);
}
