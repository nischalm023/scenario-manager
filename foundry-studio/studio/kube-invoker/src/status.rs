//! Within `kube-invoker`, `status` is the reading of one Job and its pods into the loops' `JobStatus`, done without a
//! cluster: its pods say whether a runner may still run, or what keeps it from starting, and once none can, the Job's
//! conditions and counts say whether it ended and how.

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{ContainerStatus, Pod};
use run_invocation::JobStatus;

/// What `job`, listed with its `pods`, is. While a pod has not reached a terminal phase a runner may still run, so the
/// Job is running or pending whatever its conditions and counts say, and even when it is gone, since a Job deleted in
/// the background goes before its pods (spec 08 section 3.4: a seat frees only once no pod of its Job runs). With no
/// such pod: absent when the Job is gone; ended once Kubernetes says so (a failure told in the Job's reason and the
/// pod's termination, `DeadlineExceeded: OOMKilled (exit 137)`) or its pods all ended; pending otherwise.
pub fn status_of(job: Option<&Job>, pods: &[Pod]) -> JobStatus {
    let live: Vec<&Pod> = pods.iter().filter(|pod| !ended(pod)).collect();
    if live.iter().any(|pod| containers(pod).any(runs)) {
        return JobStatus::Running;
    }
    if !live.is_empty() {
        return JobStatus::Pending { reason: live.iter().find_map(|pod| held_back(pod)) };
    }
    let Some(job) = job else { return JobStatus::Absent };
    let status = job.status.as_ref();
    let conditions = status.and_then(|status| status.conditions.as_deref()).unwrap_or_default();
    let holds = |kind: &str| conditions.iter().find(|condition| condition.type_ == kind && condition.status == "True");
    let count = |count: Option<i32>| count.unwrap_or_default() >= 1;
    if holds("Complete").is_some() || count(status.and_then(|status| status.succeeded)) {
        return JobStatus::Ended { reason: None };
    }
    if let Some(failed) = holds("Failed") {
        return JobStatus::Ended { reason: Some(failure(failed.reason.as_deref(), pods)) };
    }
    // With no restarts and one pod, a pod that reached a terminal phase is the Job's ending, which its conditions
    // say a moment later.
    if count(status.and_then(|status| status.failed)) || !pods.is_empty() {
        return JobStatus::Ended { reason: termination(pods) };
    }
    JobStatus::Pending { reason: None }
}

/// A failed Job's reason, `DeadlineExceeded` or `BackoffLimitExceeded`, with its pod's termination when there is one.
fn failure(reason: Option<&str>, pods: &[Pod]) -> String {
    match (reason, termination(pods)) {
        (Some(reason), Some(detail)) => format!("{reason}: {detail}"),
        (Some(reason), None) => reason.to_owned(),
        (None, Some(detail)) => detail,
        (None, None) => "Failed".to_owned(),
    }
}

/// The first container that ended other than cleanly, in words: `OOMKilled (exit 137)`, `Error (exit 1)`, `exit 2`.
fn termination(pods: &[Pod]) -> Option<String> {
    pods.iter().flat_map(containers).filter_map(|container| container.state.as_ref()?.terminated.as_ref()).find_map(
        |ended| match (ended.reason.as_deref(), ended.exit_code) {
            (None | Some("Completed"), 0) => None,
            (None, code) => Some(format!("exit {code}")),
            (Some(reason), 0) => Some(reason.to_owned()),
            (Some(reason), code) => Some(format!("{reason} (exit {code})")),
        },
    )
}

/// What keeps `pod` from running: that it could not be scheduled, else what its container waits on
/// (`ImagePullBackOff`, `CreateContainerConfigError`, `ContainerCreating`), each with the platform's message.
fn held_back(pod: &Pod) -> Option<String> {
    let conditions = pod.status.as_ref()?.conditions.as_deref().unwrap_or_default();
    let unscheduled = conditions
        .iter()
        .find(|condition| condition.type_ == "PodScheduled" && condition.status == "False")
        .and_then(|condition| words(condition.reason.as_deref(), condition.message.as_deref()));
    unscheduled.or_else(|| {
        containers(pod)
            .filter_map(|container| container.state.as_ref()?.waiting.as_ref())
            .find_map(|waiting| words(waiting.reason.as_deref(), waiting.message.as_deref()))
    })
}

fn words(reason: Option<&str>, message: Option<&str>) -> Option<String> {
    match (reason, message) {
        (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
        (Some(text), None) | (None, Some(text)) => Some(text.to_owned()),
        (None, None) => None,
    }
}

/// Whether `pod` reached a terminal phase, after which no container of it runs again (it is never restarted).
fn ended(pod: &Pod) -> bool {
    matches!(pod.status.as_ref().and_then(|status| status.phase.as_deref()), Some("Succeeded" | "Failed"))
}

fn containers(pod: &Pod) -> impl Iterator<Item = &ContainerStatus> {
    pod.status.as_ref().and_then(|status| status.container_statuses.as_deref()).unwrap_or_default().iter()
}

fn runs(container: &ContainerStatus) -> bool {
    container.state.as_ref().is_some_and(|state| state.running.is_some())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn job(status: Value) -> Job {
        serde_json::from_value(json!({ "metadata": { "name": "run-x-1" }, "status": status })).unwrap()
    }

    fn pod(status: Value) -> Pod {
        serde_json::from_value(json!({ "metadata": { "name": "run-x-1-abcde" }, "status": status })).unwrap()
    }

    fn terminated(reason: &str, exit_code: i32) -> Pod {
        pod(json!({
            "phase": "Failed",
            "containerStatuses": [{ "name": "simrunner", "image": "x", "imageID": "x", "ready": false,
                "restartCount": 0, "state": { "terminated": { "reason": reason, "exitCode": exit_code } } }]
        }))
    }

    /// A pod deleted while its container still runs, which it does until its termination grace runs out.
    fn terminating() -> Pod {
        serde_json::from_value(json!({
            "metadata": { "name": "run-x-1-abcde", "deletionTimestamp": "2026-10-22T10:05:00Z" },
            "status": { "phase": "Running", "containerStatuses": [{ "name": "simrunner", "image": "x",
                "imageID": "x", "ready": true, "restartCount": 0,
                "state": { "running": { "startedAt": "2026-10-22T10:00:00Z" } } }] }
        }))
        .unwrap()
    }

    #[test]
    fn no_job_is_absent() {
        assert_eq!(status_of(None, &[]), JobStatus::Absent);
        assert_eq!(status_of(None, &[terminated("Error", 143)]), JobStatus::Absent, "its one pod ended");
    }

    /// A Job deleted in the background goes before its pod, which runs out its grace: the seat must not free yet.
    #[test]
    fn a_pod_still_terminating_after_its_job_went_keeps_it_running() {
        assert_eq!(status_of(None, &[terminating()]), JobStatus::Running);
    }

    /// The Job's conditions and counts can say it ended while its pod still runs out its termination grace, past its
    /// deadline say: the pod decides, and the Job ends once no pod of it can run.
    #[test]
    fn a_job_said_to_have_ended_runs_while_a_pod_of_it_does() {
        let failed = job(json!({ "failed": 1, "conditions": [{ "type": "Failed", "status": "True",
            "reason": "DeadlineExceeded" }] }));
        assert_eq!(status_of(Some(&failed), &[terminating()]), JobStatus::Running);
        assert_eq!(status_of(Some(&job(json!({ "succeeded": 1 }))), &[terminating()]), JobStatus::Running);
        let unscheduled = pod(json!({ "phase": "Pending" }));
        assert_eq!(status_of(Some(&failed), &[unscheduled]), JobStatus::Pending { reason: None });
        let ended = status_of(Some(&failed), &[terminated("Error", 143)]);
        assert_eq!(ended, JobStatus::Ended { reason: Some("DeadlineExceeded: Error (exit 143)".to_owned()) });
    }

    #[test]
    fn an_active_job_without_pods_is_pending_for_no_said_reason() {
        assert_eq!(status_of(Some(&job(json!({ "active": 1 }))), &[]), JobStatus::Pending { reason: None });
    }

    #[test]
    fn an_unschedulable_pod_is_pending_with_the_schedulers_words() {
        let pod = pod(json!({
            "phase": "Pending",
            "conditions": [{ "type": "PodScheduled", "status": "False", "reason": "Unschedulable",
                "message": "0/3 nodes are available: 3 Insufficient cpu." }]
        }));
        let status = status_of(Some(&job(json!({ "active": 1 }))), &[pod]);
        let reason = Some("Unschedulable: 0/3 nodes are available: 3 Insufficient cpu.".to_owned());
        assert_eq!(status, JobStatus::Pending { reason });
    }

    #[test]
    fn an_image_that_will_not_pull_is_pending_with_the_kubelets_words() {
        let pod = pod(json!({
            "phase": "Pending",
            "conditions": [{ "type": "PodScheduled", "status": "True" }],
            "containerStatuses": [{ "name": "simrunner", "image": "x", "imageID": "", "ready": false,
                "restartCount": 0, "state": { "waiting": { "reason": "ImagePullBackOff",
                "message": "Back-off pulling image \"ghcr.io/example/simrunner:test\"" } } }]
        }));
        let status = status_of(Some(&job(json!({ "active": 1 }))), &[pod]);
        let reason = Some("ImagePullBackOff: Back-off pulling image \"ghcr.io/example/simrunner:test\"".to_owned());
        assert_eq!(status, JobStatus::Pending { reason });
    }

    #[test]
    fn a_running_container_is_running() {
        let pod = pod(json!({
            "phase": "Running",
            "containerStatuses": [{ "name": "simrunner", "image": "x", "imageID": "x", "ready": true,
                "restartCount": 0, "state": { "running": { "startedAt": "2026-10-22T10:00:00Z" } } }]
        }));
        assert_eq!(status_of(Some(&job(json!({ "active": 1 }))), &[pod]), JobStatus::Running);
    }

    #[test]
    fn a_complete_job_ended_for_no_reason() {
        let job = job(json!({ "succeeded": 1, "conditions": [{ "type": "Complete", "status": "True" }] }));
        assert_eq!(status_of(Some(&job), &[terminated("Completed", 0)]), JobStatus::Ended { reason: None });
        let job = self::job(json!({ "succeeded": 1 }));
        assert_eq!(status_of(Some(&job), &[]), JobStatus::Ended { reason: None });
    }

    #[test]
    fn a_job_past_its_deadline_ended_for_that_reason() {
        let job = job(json!({ "failed": 1, "conditions": [{ "type": "Failed", "status": "True",
            "reason": "DeadlineExceeded", "message": "Job was active longer than specified deadline" }] }));
        let status = status_of(Some(&job), &[]);
        assert_eq!(status, JobStatus::Ended { reason: Some("DeadlineExceeded".to_owned()) });
    }

    #[test]
    fn a_failed_job_names_its_pods_termination_too() {
        let job = job(json!({ "failed": 1, "conditions": [{ "type": "Failed", "status": "True",
            "reason": "BackoffLimitExceeded" }] }));
        let status = status_of(Some(&job), &[terminated("OOMKilled", 137)]);
        assert_eq!(status, JobStatus::Ended { reason: Some("BackoffLimitExceeded: OOMKilled (exit 137)".to_owned()) });
        let JobStatus::Ended { reason: Some(reason) } = status else { panic!("ended") };
        assert!(reason.contains("BackoffLimitExceeded") && reason.contains("OOMKilled"));
    }

    #[test]
    fn a_failed_pod_before_the_jobs_conditions_is_already_an_ending() {
        let status = status_of(Some(&job(json!({ "active": 0 }))), &[terminated("Error", 2)]);
        assert_eq!(status, JobStatus::Ended { reason: Some("Error (exit 2)".to_owned()) });
    }
}
