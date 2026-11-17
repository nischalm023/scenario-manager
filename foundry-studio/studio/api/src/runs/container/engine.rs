//! Within `container`, `engine` is the container engine's command-line dialect, docker's and podman's alike: which
//! engine is on `PATH`, the `run` line an attempt's container starts with, its stack and its deadline among its
//! labels, how what `inspect` and `wait` print reads (a container's status, its deadline or its exit, an image's
//! build and source), and how the engine's refusals read in words. The invoker decides when to ask what; this only
//! spells it.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use chrono::DateTime;
use run_invocation::{JobRequest, JobStatus};
use rustc_hash::FxHashMap;
use serde::Deserialize;

use super::ContainerSetup;
use super::watch::Deadline;

/// The label every runner container carries, by which a next API process finds them.
pub(super) const RUNNER_LABEL: &str = "agent-studio=runner";
/// The label naming the stack whose API started a container, when its configuration names one.
pub(super) const STACK_LABEL: &str = "agent-studio/stack";
/// The label naming the source a runner image was built from, which `scripts/source-identity.sh` spells.
const SOURCE_LABEL: &str = "agent-studio/source";
/// Where the image's runner reads its spec and writes its output: the Dockerfile's folders.
const SPEC_IN_CONTAINER: &str = "/var/lib/simrunner/spec";
const OUT_IN_CONTAINER: &str = "/var/lib/simrunner/out";
/// The host as a container reaches it, on Docker Desktop natively and on Linux through `--add-host`.
pub const HOST_ALIAS: &str = "host.docker.internal";
/// The labels a container keeps its deadline in, so the next API process holds an adopted one to it: the instant it
/// falls (Unix seconds) and the allowance counted to it (seconds).
const DEADLINE_AT_LABEL: &str = "agent-studio/deadline-at";
const DEADLINE_SECS_LABEL: &str = "agent-studio/deadline-secs";

/// The first container engine on `PATH`, `docker` before `podman`, or the one `named`.
pub fn find_engine(named: Option<&Path>) -> Option<PathBuf> {
    if let Some(named) = named {
        return Some(named.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    for name in ["docker", "podman"] {
        if let Some(found) = env::split_paths(&path).map(|dir| dir.join(name)).find(|file| file.is_file()) {
            return Some(found);
        }
    }
    None
}

/// `docker run`'s arguments for one attempt: detached, named and labelled with its stack and its deadline, the
/// environment file, the host alias, the spec folder read-only and the output folder mounted at the image's paths, the
/// resources, the user, the image the attempt's request names, and the runner's own arguments after it.
pub(super) fn run_args(
    setup: &ContainerSetup,
    request: &JobRequest,
    deadline: &Deadline,
    spec: &Path,
    out: &Path,
    environment: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = ["run", "-d", "--name", request.name.as_str(), "--label", RUNNER_LABEL]
        .into_iter()
        .map(str::to_owned)
        .collect();
    for (label, value) in &request.labels {
        args.extend(["--label".to_owned(), format!("{label}={value}")]);
    }
    if let Some(stack) = &setup.stack {
        args.extend(["--label".to_owned(), format!("{STACK_LABEL}={stack}")]);
    }
    args.extend(["--label".to_owned(), format!("{DEADLINE_AT_LABEL}={}", deadline.at.timestamp())]);
    args.extend(["--label".to_owned(), format!("{DEADLINE_SECS_LABEL}={}", deadline.allowance.as_secs())]);
    args.extend(["--env-file".to_owned(), environment.display().to_string()]);
    if setup.host_alias {
        args.extend(["--add-host".to_owned(), format!("{HOST_ALIAS}:host-gateway")]);
    }
    args.extend(["-v".to_owned(), format!("{}:{SPEC_IN_CONTAINER}:ro", spec.display())]);
    args.extend(["-v".to_owned(), format!("{}:{OUT_IN_CONTAINER}", out.display())]);
    args.extend(["--cpus".to_owned(), request.resources.cpu_millis.0.div_ceil(1_000).to_string()]);
    if let Some(memory) = request.resources.memory_mib {
        args.extend(["--memory".to_owned(), format!("{}m", memory.0)]);
    }
    if let Some(user) = &setup.user {
        args.extend(["--user".to_owned(), user.clone()]);
    }
    args.push(request.image.clone());
    args.extend([
        "--job".to_owned(),
        format!("{SPEC_IN_CONTAINER}/job.json"),
        "--out".to_owned(),
        OUT_IN_CONTAINER.to_owned(),
    ]);
    args
}

/// What the engine's `inspect` line says of a container: created or paused is pending, running is running, exited
/// or dead is ended with its exit code, or out of memory when the engine killed it for that.
pub(super) fn status_of(line: &str) -> JobStatus {
    let mut words = line.split_whitespace();
    let (state, code, oom) = (words.next().unwrap_or(""), words.next().unwrap_or("0"), words.next() == Some("true"));
    match state {
        "running" => JobStatus::Running,
        "created" | "restarting" | "paused" => JobStatus::Pending { reason: Some(state.to_owned()) },
        "exited" | "dead" | "removing" => JobStatus::Ended {
            reason: Some(if oom { format!("OOMKilled (exit {code})") } else { format!("exit {code}") }),
        },
        other => JobStatus::Unknown { reason: format!("the engine says the container is {other:?}") },
    }
}

/// `inspect`'s format for the labels a container keeps its deadline in, which `deadline_of` reads back.
pub(super) fn deadline_format() -> String {
    format!(
        r#"{{{{index .Config.Labels "{DEADLINE_AT_LABEL}"}}}} {{{{index .Config.Labels "{DEADLINE_SECS_LABEL}"}}}}"#
    )
}

/// The deadline in an `inspect` line `deadline_format` printed; `None` when the container carries none.
pub(super) fn deadline_of(line: &str) -> Option<Deadline> {
    let mut words = line.split_whitespace();
    let at = DateTime::from_timestamp(words.next()?.parse().ok()?, 0)?;
    let allowance = Duration::from_secs(words.next()?.parse().ok()?);
    Some(Deadline { allowance, at })
}

/// What the engine knows of a runner image: its id, which a container of it runs whatever its reference names later,
/// the build `studio/Dockerfile` baked into its environment for dessim's harness, and the source it was built from,
/// uncommitted changes and all, which its label keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakedImage {
    pub id: String,
    pub commit: String,
    pub branch: String,
    pub source: String,
}

/// The part of `image inspect`'s answer read here.
#[derive(Deserialize)]
struct Inspected {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Config")]
    config: InspectedConfig,
}

#[derive(Deserialize)]
struct InspectedConfig {
    #[serde(rename = "Env", default)]
    env: Vec<String>,
    #[serde(rename = "Labels", default)]
    labels: Option<FxHashMap<String, String>>,
}

/// One image as `image inspect` answered for it; the words of what it lacks.
pub(super) fn baked_of(inspected: &[u8]) -> Result<BakedImage, String> {
    let images: Vec<Inspected> =
        serde_json::from_slice(inspected).map_err(|fault| format!("the engine's inspect does not read: {fault}"))?;
    let image = images.into_iter().next().ok_or("the engine's inspect named no image")?;
    let baked = |name: &str| {
        let line = image.config.env.iter().find_map(|line| line.strip_prefix(name)?.strip_prefix('='));
        line.map(str::to_owned)
            .ok_or_else(|| format!("it has no {name}, which studio/Dockerfile's simrunner target bakes"))
    };
    let (commit, branch) = (baked("DESVIS_RUN_COMMIT")?, baked("DESVIS_RUN_BRANCH")?);
    let labels = image.config.labels.unwrap_or_default();
    let source = labels.get(SOURCE_LABEL).filter(|source| !source.is_empty()).cloned().ok_or_else(|| {
        format!("it has no {SOURCE_LABEL} label, which dev.sh's build gives it from scripts/source-identity.sh")
    })?;
    Ok(BakedImage { id: image.id, commit, branch, source })
}

/// Whether the engine's answer says no such container exists.
pub(super) fn absent(answer: &Output) -> bool {
    let words = String::from_utf8_lossy(&answer.stderr).to_ascii_lowercase();
    !answer.status.success() && (words.contains("no such container") || words.contains("no such object"))
}

/// What the engine said when it refused, in its own last line, or its exit status when it said nothing.
pub(super) fn engine_words(answer: &Output) -> String {
    let stderr = String::from_utf8_lossy(&answer.stderr);
    let line = stderr.lines().last().unwrap_or("").trim();
    if line.is_empty() { format!("the engine answered {}", answer.status) } else { line.to_owned() }
}

/// The container's exit in words, as `settle` reads them, when `docker wait` came back with one: its exit status, or
/// that the container is gone, which it can only be once it has stopped. `None` when the engine could not say, its
/// daemon restarting say, which is no exit at all.
pub(super) fn exit_of(waited: &Output) -> Option<String> {
    if waited.status.success() {
        let said = String::from_utf8_lossy(&waited.stdout);
        return Some(match said.trim().parse::<i32>() {
            Ok(code) => format!("the runner exited with status {code}"),
            Err(_) => format!("the runner's container exited, and the engine's wait said {:?}", said.trim()),
        });
    }
    absent(waited).then(|| "the runner's container was removed before its exit was read".to_owned())
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::time::Duration;

    use run_invocation::{AttemptKey, CpuMillis, JobName, JobRequest, JobStatus, MemoryMib, Resources, Token};
    use rustc_hash::FxHashMap;
    use studio_protocol::{AttemptNumber, Endpoint, FORMAT, JobSpec};

    use super::*;

    fn request() -> JobRequest {
        let key = AttemptKey { run: "01KZAVPQNES65HV4EZADPCS7E8".parse().unwrap(), n: AttemptNumber(2) };
        JobRequest {
            name: JobName::of(&key),
            image: "registry.example/studio-simrunner@sha256:0f0f".to_owned(),
            spec: JobSpec {
                v: FORMAT,
                run: key.run.clone(),
                attempt: key.n,
                studio: Endpoint { endpoint: "http://host.docker.internal:3101".to_owned() },
                streaming: None,
            },
            token: Token::mint(),
            deadline: Duration::from_secs(60),
            resources: Resources { cpu_millis: CpuMillis(1_500), memory_mib: Some(MemoryMib(2_048)) },
            labels: FxHashMap::from_iter([("agent-studio/run".to_owned(), key.run.to_string())]),
        }
    }

    /// A deadline of an hour, falling at a fixed instant.
    fn deadline() -> Deadline {
        Deadline { allowance: Duration::from_secs(3_600), at: DateTime::from_timestamp(1_793_080_000, 0).unwrap() }
    }

    /// The token rides only in the environment file; the spec is read-only; the image is the one the attempt's
    /// request names, and the runner's arguments follow it.
    #[test]
    fn the_run_line_runs_the_attempts_image_and_keeps_the_token_off_it() {
        let mut setup = ContainerSetup::new("docker".into(), "/o".into(), "/out".into());
        setup.host_alias = true;
        let request = request();
        let args = run_args(
            &setup,
            &request,
            &deadline(),
            Path::new("/o/runs/x/attempts/2"),
            Path::new("/out/job"),
            Path::new("/out/job.env"),
        );
        let line = args.join(" ");
        assert!(!line.contains(request.token.expose()), "{line}");
        assert!(
            line.starts_with("run -d --name run-01kzavpqnes65hv4ezadpcs7e8-2 --label agent-studio=runner"),
            "{line}"
        );
        assert!(
            line.contains("-v /o/runs/x/attempts/2:/var/lib/simrunner/spec:ro -v /out/job:/var/lib/simrunner/out"),
            "{line}"
        );
        let image = "--cpus 2 --memory 2048m registry.example/studio-simrunner@sha256:0f0f";
        assert!(
            line.ends_with(&format!("{image} --job /var/lib/simrunner/spec/job.json --out /var/lib/simrunner/out")),
            "{line}"
        );
        assert!(line.contains("--env-file /out/job.env --add-host host.docker.internal:host-gateway"), "{line}");
    }

    /// A container keeps its stack and its deadline in its labels, and what `inspect` prints of the deadline reads
    /// back as it, so an API process that adopts it holds it to the deadline it started with; a container without them
    /// has none.
    #[test]
    fn a_container_keeps_its_stack_and_its_deadline_in_its_labels() {
        let mut setup = ContainerSetup::new("docker".into(), "/o".into(), "/out".into());
        let out = (Path::new("/out/job"), Path::new("/out/job.env"));
        let line = run_args(&setup, &request(), &deadline(), Path::new("/o/a"), out.0, out.1).join(" ");
        assert!(!line.contains("agent-studio/stack"), "no stack named, none labelled: {line}");
        setup.stack = Some("5eed1e55".to_owned());
        let line = run_args(&setup, &request(), &deadline(), Path::new("/o/a"), out.0, out.1).join(" ");
        let labels = "--label agent-studio/stack=5eed1e55 --label agent-studio/deadline-at=1793080000 \
                      --label agent-studio/deadline-secs=3600";
        assert!(line.contains(labels), "{line}");
        assert_eq!(deadline_of("1793080000 3600\n"), Some(deadline()));
        assert_eq!(deadline_of(" \n"), None, "an unlabelled container has no deadline to keep");
        assert_eq!(deadline_format().matches("index .Config.Labels").count(), 2);
    }

    /// An image is what the engine says of it: its id, the build baked into its environment and the source its label
    /// names, which an image dev.sh did not build from `studio/Dockerfile`'s simrunner target lacks.
    #[test]
    fn an_image_is_its_id_the_build_baked_into_it_and_its_source() {
        let inspected = br#"[{"Id": "sha256:4b1d", "Config": {"Env": ["PATH=/usr/bin", "DESVIS_RUN_BRANCH=main",
            "DESVIS_RUN_COMMIT=c0ffee", "DESVIS_RUN_DIRTY=true"], "Labels": {"agent-studio/source": "c0ffee+5eed"}}}]"#;
        let baked = baked_of(inspected).expect("a runner image");
        let expected = BakedImage {
            id: "sha256:4b1d".to_owned(),
            commit: "c0ffee".to_owned(),
            branch: "main".to_owned(),
            source: "c0ffee+5eed".to_owned(),
        };
        assert_eq!(baked, expected);
        let unbaked = baked_of(br#"[{"Id": "sha256:b0b0", "Config": {"Env": ["PATH=/bin"]}}]"#).expect_err("no build");
        assert!(unbaked.contains("DESVIS_RUN_COMMIT"), "{unbaked}");
        let unlabelled = br#"[{"Id": "sha256:b0b0", "Config": {"Env": ["DESVIS_RUN_BRANCH=main",
            "DESVIS_RUN_COMMIT=c0ffee"], "Labels": null}}]"#;
        let unlabelled = baked_of(unlabelled).expect_err("no source");
        assert!(unlabelled.contains("agent-studio/source"), "{unlabelled}");
    }

    /// Only an exit status or a container gone is an exit; an engine that could not be asked says nothing of one.
    #[test]
    fn only_an_exit_status_or_a_container_gone_is_an_exit() {
        let answer = |code: i32, stdout: &str, stderr: &str| Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        };
        assert_eq!(exit_of(&answer(0, "3\n", "")).as_deref(), Some("the runner exited with status 3"));
        let gone = answer(1, "", "Error response from daemon: No such container: run-x-1");
        assert_eq!(exit_of(&gone).as_deref(), Some("the runner's container was removed before its exit was read"));
        let unreachable = answer(1, "", "Cannot connect to the Docker daemon at unix:///var/run/docker.sock.");
        assert_eq!(exit_of(&unreachable), None);
    }

    #[test]
    fn the_engines_states_read_as_the_loops_statuses() {
        assert_eq!(status_of("running 0 false"), JobStatus::Running);
        assert_eq!(status_of("created 0 false"), JobStatus::Pending { reason: Some("created".to_owned()) });
        assert_eq!(status_of("exited 3 false"), JobStatus::Ended { reason: Some("exit 3".to_owned()) });
        assert_eq!(status_of("exited 137 true"), JobStatus::Ended { reason: Some("OOMKilled (exit 137)".to_owned()) });
        assert!(matches!(status_of("limbo 0 false"), JobStatus::Unknown { .. }));
    }
}
