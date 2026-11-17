//! `kube-invoker` is `run-invocation`'s `Invoker` on Kubernetes (spec 08 section 3.5): each attempt's `JobRequest`
//! becomes one batch/v1 Job running the runner image, its job spec and token in a Secret of the Job's name that the
//! Job owns, and the Job's conditions and its pods are read back as the loops' `JobStatus`. `job` builds the two
//! objects and `status` reads them, both without a cluster; `invoker` is the three calls over a `kube::Client`.
//! Nothing here keeps state: the cluster is the record, so a second API process answers the same as the first. Every
//! duration here is wall clock.

mod invoker;
mod job;
mod status;

use std::fs::read_to_string;
use std::sync::Arc;
use std::time::Duration;

use kube::Client;
use run_invocation::{Fault, RunnerSetting};

/// Where a pod's service account mount names the pod's own namespace.
const NAMESPACE_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// The namespace this process's pod runs in, from its service account mount; none outside a cluster.
pub fn own_namespace() -> Option<String> {
    read_to_string(NAMESPACE_FILE).ok().map(|text| text.trim().to_owned()).filter(|text| !text.is_empty())
}

/// What every Job this invoker creates is told about its cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubeSetup {
    /// The namespace the Jobs and their Secrets are created in.
    pub namespace: String,
    /// The runner pod's service account; none means the namespace's default.
    pub service_account: Option<String>,
    /// The settings every runner pod is given besides its token and thread count; the API fills them from
    /// `STUDIO_RUNNER_PASSED`.
    pub passed: Vec<RunnerSetting>,
    /// How long a deleted pod has between its SIGTERM and its SIGKILL (wall clock): 90 s.
    pub termination_grace: Duration,
    /// How long an ended Job stays for `kubectl` before Kubernetes removes it (wall clock): an hour.
    pub ttl_after_finished: Duration,
}

impl KubeSetup {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            service_account: None,
            passed: Vec::new(),
            termination_grace: Duration::from_secs(90),
            ttl_after_finished: Duration::from_secs(3600),
        }
    }
}

/// The invoker: a client to the cluster and the setup every Job is built from.
#[derive(Clone)]
pub struct KubeInvoker {
    client: Client,
    setup: Arc<KubeSetup>,
}

impl KubeInvoker {
    pub fn new(client: Client, setup: KubeSetup) -> Self {
        Self { client, setup: Arc::new(setup) }
    }

    /// Over the cluster this process runs in, or the one `KUBECONFIG` (else `~/.kube/config`) names outside one.
    pub async fn from_env(setup: KubeSetup) -> Result<Self, Fault> {
        Ok(Self::new(Client::try_default().await?, setup))
    }
}
