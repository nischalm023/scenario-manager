//! Within `kube-invoker`, `job` is the two objects one `JobRequest` becomes, built without a cluster: the Job whose one
//! pod runs the runner image, its environment the token, the thread count and the deployment's runner settings and
//! nothing else, and the Secret holding the attempt's job spec and its token, which the pod mounts and the Job owns.
//! The invoker sets their namespace.

use std::collections::BTreeMap; // ordered: k8s-openapi keeps labels, data and resource amounts in B-trees.
use std::time::Duration;

use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, EnvVarSource, KeyToPath, PodSpec, PodTemplateSpec, ResourceRequirements, Secret,
    SecretKeySelector, SecretVolumeSource, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use run_invocation::JobRequest;
use studio_protocol::{THREADS_VAR, TOKEN_VAR};

use crate::KubeSetup;

/// The label every object here carries besides the request's own.
const APP_LABEL: (&str, &str) = ("app.kubernetes.io/name", "studio-simrunner");
/// The Secret's two keys: the job spec, mounted as a file, and the token, set as an environment variable.
const SPEC_KEY: &str = "job.json";
const TOKEN_KEY: &str = "token";
/// The Secret is mounted as a folder of its own beside the runner's writable working directory, and the runner is
/// told both paths outright, so nothing overlays the image's own `/var/lib/simrunner`.
const SPEC_DIR: &str = "/var/lib/simrunner/spec";
const OUT_DIR: &str = "/var/lib/simrunner/out";

/// The Secret of `request`'s name: its job spec and its token, the token's only place in the cluster, owned from its
/// creation by the Job whose uid is `job_uid`, so the garbage collector deletes it with that Job.
pub fn secret_of(request: &JobRequest, job_uid: &str) -> Secret {
    let spec = serde_json::to_string_pretty(&request.spec).expect("a job spec is plain data and serializes");
    let data = [(SPEC_KEY.to_owned(), spec), (TOKEN_KEY.to_owned(), request.token.expose().to_owned())];
    let owner = OwnerReference {
        api_version: "batch/v1".to_owned(),
        kind: "Job".to_owned(),
        name: request.name.to_string(),
        uid: job_uid.to_owned(),
        ..OwnerReference::default()
    };
    Secret {
        metadata: ObjectMeta { owner_references: Some(vec![owner]), ..metadata(request) },
        type_: Some("Opaque".to_owned()),
        string_data: Some(BTreeMap::from(data)),
        ..Secret::default()
    }
}

/// The Job of `request`: one pod, never restarted, the runner image over the Secret's spec and token, requesting
/// exactly what it is limited to. Its environment is named variable by variable, never a Secret's keys wholesale.
pub fn job_of(request: &JobRequest, setup: &KubeSetup) -> Job {
    let name = request.name.to_string();
    let cpu_millis = request.resources.cpu_millis.0;
    let args = vec!["--job".into(), format!("{SPEC_DIR}/{SPEC_KEY}"), "--out".into(), OUT_DIR.into()];
    let token = SecretKeySelector { name: name.clone(), key: TOKEN_KEY.to_owned(), optional: None };
    let mut env = vec![
        EnvVar {
            name: TOKEN_VAR.to_owned(),
            value_from: Some(EnvVarSource { secret_key_ref: Some(token), ..EnvVarSource::default() }),
            value: None,
        },
        EnvVar { name: THREADS_VAR.to_owned(), value: Some(cpu_millis.div_ceil(1000).to_string()), value_from: None },
    ];
    env.extend(setup.passed.iter().map(|setting| EnvVar {
        name: setting.name().to_owned(),
        value: Some(setting.value().to_owned()),
        value_from: None,
    }));
    let mut amounts = BTreeMap::from([("cpu".to_owned(), Quantity(format!("{cpu_millis}m")))]);
    if let Some(memory) = request.resources.memory_mib {
        amounts.insert("memory".to_owned(), Quantity(format!("{}Mi", memory.0)));
    }
    let container = Container {
        name: "simrunner".to_owned(),
        image: Some(request.image.clone()),
        args: Some(args),
        env: Some(env),
        resources: Some(ResourceRequirements { requests: Some(amounts.clone()), limits: Some(amounts), claims: None }),
        volume_mounts: Some(vec![VolumeMount {
            name: "spec".to_owned(),
            mount_path: SPEC_DIR.to_owned(),
            read_only: Some(true),
            ..VolumeMount::default()
        }]),
        ..Container::default()
    };
    let spec_volume = SecretVolumeSource {
        secret_name: Some(name),
        items: Some(vec![KeyToPath { key: SPEC_KEY.to_owned(), path: SPEC_KEY.to_owned(), mode: None }]),
        ..SecretVolumeSource::default()
    };
    let pod = PodSpec {
        containers: vec![container],
        restart_policy: Some("Never".to_owned()),
        termination_grace_period_seconds: Some(seconds(setup.termination_grace)),
        service_account_name: setup.service_account.clone(),
        // The runner never calls the Kubernetes API, so its pod holds no token for it.
        automount_service_account_token: Some(false),
        volumes: Some(vec![Volume { name: "spec".to_owned(), secret: Some(spec_volume), ..Volume::default() }]),
        ..PodSpec::default()
    };
    let template = PodTemplateSpec {
        metadata: Some(ObjectMeta { labels: Some(labels(request)), ..ObjectMeta::default() }),
        spec: Some(pod),
    };
    let spec = JobSpec {
        backoff_limit: Some(0),
        active_deadline_seconds: Some(seconds(request.deadline)),
        ttl_seconds_after_finished: Some(i32::try_from(setup.ttl_after_finished.as_secs()).unwrap_or(i32::MAX)),
        template,
        ..JobSpec::default()
    };
    Job { metadata: metadata(request), spec: Some(spec), status: None }
}

fn metadata(request: &JobRequest) -> ObjectMeta {
    ObjectMeta { name: Some(request.name.to_string()), labels: Some(labels(request)), ..ObjectMeta::default() }
}

fn labels(request: &JobRequest) -> BTreeMap<String, String> {
    let mut labels: BTreeMap<_, _> = request.labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    labels.insert(APP_LABEL.0.to_owned(), APP_LABEL.1.to_owned());
    labels
}

fn seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use run_invocation::{AttemptKey, CpuMillis, JobName, JobRequest, MemoryMib, Resources, RunnerSetting, Token};
    use rustc_hash::FxHashMap;
    use studio_protocol::{AttemptNumber, Endpoint, FORMAT, JobSpec};

    use super::*;

    const RUN: &str = "01KZAVPQNES65HV4EZADPCS7E8";

    fn request(memory_mib: Option<u32>) -> JobRequest {
        let key = AttemptKey { run: RUN.parse().expect("a ULID"), n: AttemptNumber(1) };
        let spec = JobSpec {
            v: FORMAT,
            run: key.run.clone(),
            attempt: key.n,
            studio: Endpoint { endpoint: "http://studio:3101".to_owned() },
            streaming: None,
        };
        let labels = FxHashMap::from_iter([
            ("agent-studio/attempt".to_owned(), "1".to_owned()),
            ("agent-studio/run".to_owned(), RUN.to_owned()),
        ]);
        JobRequest {
            name: JobName::of(&key),
            image: "ghcr.io/example/simrunner:test".to_owned(),
            spec,
            token: Token::mint(),
            deadline: Duration::from_secs(21_600),
            resources: Resources { cpu_millis: CpuMillis(1500), memory_mib: memory_mib.map(MemoryMib) },
            labels,
        }
    }

    fn container(job: &Job) -> &Container {
        &job.spec.as_ref().unwrap().template.spec.as_ref().unwrap().containers[0]
    }

    fn env<'a>(job: &'a Job, name: &str) -> Option<&'a EnvVar> {
        container(job).env.as_ref().unwrap().iter().find(|var| var.name == name)
    }

    #[test]
    fn the_job_is_named_labelled_bounded_and_never_restarted() {
        let request = request(None);
        let setup = KubeSetup::new("studio");
        let job = job_of(&request, &setup);
        assert_eq!(job.metadata.name.as_deref(), Some("run-01kzavpqnes65hv4ezadpcs7e8-1"));
        let labels = job.metadata.labels.as_ref().unwrap();
        assert_eq!(labels["app.kubernetes.io/name"], "studio-simrunner");
        assert_eq!(labels["agent-studio/run"], RUN);
        assert_eq!(labels["agent-studio/attempt"], "1");
        let spec = job.spec.as_ref().unwrap();
        assert_eq!(spec.backoff_limit, Some(0));
        assert_eq!(spec.active_deadline_seconds, Some(21_600));
        assert_eq!(spec.ttl_seconds_after_finished, Some(3600));
        assert_eq!(spec.template.metadata.as_ref().unwrap().labels.as_ref(), Some(labels));
        let pod = spec.template.spec.as_ref().unwrap();
        assert_eq!(pod.restart_policy.as_deref(), Some("Never"));
        assert_eq!(pod.termination_grace_period_seconds, Some(90));
        assert_eq!(pod.service_account_name, None);
        assert_eq!(pod.automount_service_account_token, Some(false), "the runner holds no Kubernetes token");
        assert_eq!(container(&job).image.as_deref(), Some("ghcr.io/example/simrunner:test"));
        assert_eq!(container(&job).command, None);
        let args = container(&job).args.as_ref().unwrap();
        assert_eq!(args, &["--job", "/var/lib/simrunner/spec/job.json", "--out", "/var/lib/simrunner/out"]);
    }

    #[test]
    fn requests_equal_limits_with_and_without_memory() {
        let job = job_of(&request(Some(2048)), &KubeSetup::new("studio"));
        let resources = container(&job).resources.as_ref().unwrap();
        assert_eq!(resources.requests, resources.limits);
        let limits = resources.limits.as_ref().unwrap();
        assert_eq!(limits["cpu"].0, "1500m");
        assert_eq!(limits["memory"].0, "2048Mi");

        let job = job_of(&request(None), &KubeSetup::new("studio"));
        let limits = container(&job).resources.as_ref().unwrap().limits.as_ref().unwrap();
        assert_eq!(limits["cpu"].0, "1500m");
        assert!(!limits.contains_key("memory"));
    }

    /// The pod's environment is the token by reference, the thread count and the deployment's runner settings, each
    /// named, and nothing else: no Secret's keys come in wholesale, so a Secret holding the API's database URL has no
    /// way into a runner (ruling D174).
    #[test]
    fn the_environment_is_the_token_reference_the_threads_and_the_settings_and_nothing_else() {
        let mut setup = KubeSetup::new("studio");
        setup.passed = vec![RunnerSetting::new("DES_HEARTBEAT", "3600").expect("a runner setting")];
        setup.service_account = Some("simrunner".to_owned());
        let request = request(None);
        let job = job_of(&request, &setup);
        let token = env(&job, "STUDIO_RUNNER_TOKEN").unwrap();
        assert_eq!(token.value, None);
        let key = token.value_from.as_ref().unwrap().secret_key_ref.as_ref().unwrap();
        assert_eq!((key.name.as_str(), key.key.as_str()), (request.name.as_str(), "token"));
        assert_eq!(env(&job, "STUDIO_RUNNER_THREADS").unwrap().value.as_deref(), Some("2"));
        assert_eq!(env(&job, "DES_HEARTBEAT").unwrap().value.as_deref(), Some("3600"));
        let names: Vec<&str> = container(&job).env.as_ref().unwrap().iter().map(|var| var.name.as_str()).collect();
        assert_eq!(names, ["STUDIO_RUNNER_TOKEN", "STUDIO_RUNNER_THREADS", "DES_HEARTBEAT"]);
        assert_eq!(container(&job).env_from, None, "no Secret's keys reach the runner wholesale");
        let pod = job.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
        assert_eq!(pod.service_account_name.as_deref(), Some("simrunner"));
        let volume = pod.volumes.as_ref().unwrap()[0].secret.as_ref().unwrap();
        assert_eq!(volume.secret_name.as_deref(), Some(request.name.as_str()));
        assert_eq!(volume.items.as_ref().unwrap()[0].key, "job.json");
    }

    #[test]
    fn the_token_is_in_the_secret_and_nowhere_in_the_job() {
        let request = request(None);
        let job = serde_json::to_string(&job_of(&request, &KubeSetup::new("studio"))).unwrap();
        assert!(!job.contains(request.token.expose()));
        let secret = secret_of(&request, "a-job-uid");
        assert_eq!(secret.metadata.name.as_deref(), Some(request.name.as_str()));
        assert_eq!(secret.type_.as_deref(), Some("Opaque"));
        let data = secret.string_data.as_ref().unwrap();
        assert_eq!(data["token"], request.token.expose());
        let spec: JobSpec = serde_json::from_str(&data["job.json"]).unwrap();
        assert_eq!(spec, request.spec);
        assert_eq!(secret.metadata.labels.as_ref().unwrap()["app.kubernetes.io/name"], "studio-simrunner");
    }

    /// The Secret is the Job's from its creation, so nothing but the Job's own deletion takes it away and no Secret
    /// outlives its Job.
    #[test]
    fn the_secret_is_owned_by_its_job_from_its_creation() {
        let request = request(None);
        let owners = secret_of(&request, "a-job-uid").metadata.owner_references.unwrap();
        assert_eq!(owners.len(), 1);
        let owner = &owners[0];
        assert_eq!((owner.api_version.as_str(), owner.kind.as_str()), ("batch/v1", "Job"));
        assert_eq!((owner.name.as_str(), owner.uid.as_str()), (request.name.as_str(), "a-job-uid"));
    }
}
