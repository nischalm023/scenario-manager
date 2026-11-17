//! Within `kube-invoker`, `invoker` is the `Invoker` trait over the cluster: `start` creates the Job, then the Secret
//! its pod mounts, owned by the Job from its creation; `stop` deletes the Job in the foreground, its pods and its Secret
//! with it; `status` reads the Job and its pods through `status_of`. Every call is one or a few requests, each answered
//! by the cluster and none remembered here.

use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Error, Resource};
use run_invocation::{Fault, Invoker, JobName, JobRequest, JobStatus};
use serde_json::json;

use crate::KubeInvoker;
use crate::job::{job_of, secret_of};
use crate::status::status_of;

impl KubeInvoker {
    fn api<K>(&self) -> Api<K>
    where
        K: Resource<Scope = NamespaceResourceScope>,
        K::DynamicType: Default,
    {
        Api::namespaced(self.client.clone(), &self.setup.namespace)
    }
}

impl Invoker for KubeInvoker {
    /// Idempotent by name. The Job goes first, so a Job the cluster refuses leaves no Secret behind; its pod waits for
    /// the Secret it mounts, which is then created owned by the Job. A Job already there is taken as this request's; a
    /// Secret already there is given this request's spec and token, since the reconciler's re-creation of an absent
    /// Job comes with a fresh token, and made the Job's own.
    async fn start(&self, request: JobRequest) -> Result<(), Fault> {
        let (secrets, jobs) = (self.api::<Secret>(), self.api::<Job>());
        let name = request.name.as_str();
        let job = match jobs.create(&PostParams::default(), &job_of(&request, &self.setup)).await {
            Ok(job) => job,
            Err(err) if already_exists(&err) => jobs.get(name).await?,
            Err(err) => return Err(err.into()),
        };
        let Some(uid) = job.metadata.uid else { return Err(format!("the Job {name} came back without a uid").into()) };
        let secret = secret_of(&request, &uid);
        match secrets.create(&PostParams::default(), &secret).await {
            Ok(_) => Ok(()),
            Err(err) if already_exists(&err) => {
                let refill = json!({
                    "metadata": { "ownerReferences": secret.metadata.owner_references },
                    "stringData": secret.string_data,
                });
                secrets.patch(name, &PatchParams::default(), &Patch::Merge(refill)).await?;
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Delete the Job in the foreground, so it stays, marked for deletion, until the garbage collector has deleted what
    /// it owns: its pods, once they have run out their termination grace, so a status asked in between still finds a
    /// pod that may run, and its Secret. A Job already gone is fine.
    async fn stop(&self, name: &JobName) -> Result<(), Fault> {
        gone(self.api::<Job>().delete(name.as_str(), &DeleteParams::foreground()).await)
    }

    /// The Job and its pods, the pods listed even when the Job is gone, since a Job deleted in the background goes
    /// before them.
    async fn status(&self, name: &JobName) -> Result<JobStatus, Fault> {
        let job = self.api::<Job>().get_opt(name.as_str()).await?;
        let selector = format!("batch.kubernetes.io/job-name={name}");
        let pods = self.api::<Pod>().list(&ListParams::default().labels(&selector)).await?;
        Ok(status_of(job.as_ref(), &pods.items))
    }
}

fn code(err: &Error) -> Option<u16> {
    match err {
        Error::Api(status) => Some(status.code),
        _ => None,
    }
}

fn already_exists(err: &Error) -> bool {
    code(err) == Some(409)
}

fn not_found(err: &Error) -> bool {
    code(err) == Some(404)
}

/// A deletion's answer with an object already gone counted as done.
fn gone<T>(answer: Result<T, Error>) -> Result<(), Fault> {
    match answer {
        Ok(_) => Ok(()),
        Err(err) if not_found(&err) => Ok(()),
        Err(err) => Err(err.into()),
    }
}
