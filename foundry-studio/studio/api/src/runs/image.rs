//! Within `runs`, `image` is the runner image a plan names (ruling D78), which exists so that every run says which
//! runner build it ran and which closed values that build accepts. There is no registry yet (rulings D109 and D110):
//! the one image is the one `STUDIO_RUNNER_IMAGE` names, the default, its `accepts` this build's own, and `GET
//! /api/runner-images` answers it alone (spec 08 section 3.5). Locally the engine says what the image is, its id, the
//! commit and branch built into it and the source it was built from, which must be the source this checkout holds,
//! uncommitted changes and all, for those `accepts` to hold: a stale runner would run old simulator code under this
//! build's checks. In a cluster the deployment pins the image the build that made this API's image made, so it names
//! that build's commit and branch.

use std::path::{Path, PathBuf};
use std::process::Command;

use axum::Json;
use axum::extract::State;
use scenario_spec::{COMPILER, this_build_accepts};
use studio_protocol::{RunnerImage, RunnerImages, WallClock};

use crate::AppState;

use super::container::BakedImage;

/// The one runner image in a cluster: `reference` as the deployment pins it, out of the build that made this API's
/// image, so with that build's `commit` and `branch`.
pub fn cluster_image(reference: &str, commit: &str, branch: &str) -> RunnerImage {
    image(reference.to_owned(), reference, commit, branch)
}

/// The one runner image locally: `reference` as the engine knows it, `baked`, refused unless it was built from
/// `source`, the source this checkout holds, whose closed values are this build's. Plans name the image's id, so an
/// image rebuilt under the same reference later runs no plan this one was named in.
pub fn local_image(reference: &str, baked: &BakedImage, source: &str) -> Result<RunnerImage, String> {
    if baked.source != source {
        return Err(format!(
            "the runner image {reference} was built from the source {}, and this checkout holds {source}: \
             ./dev.sh start rebuilds it",
            baked.source
        ));
    }
    Ok(image(baked.id.clone(), reference, &baked.commit, &baked.branch))
}

/// The source the checkout this process runs in holds, as `scripts/source-identity.sh` spells it for dev.sh and for
/// the runner image's label alike: its commit, and a hash of its uncommitted changes when it has any.
pub fn checkout_source() -> Result<String, String> {
    source_in(Path::new("."))
}

/// The source the checkout that holds `dir` holds, as its own `scripts/source-identity.sh` spells it.
fn source_in(dir: &Path) -> Result<String, String> {
    let unknown = |why: String| format!("this checkout's source cannot be named, as a local runner image must: {why}");
    let top = Command::new("git").args(["rev-parse", "--show-toplevel"]).current_dir(dir).output();
    let top = top.map_err(|fault| unknown(fault.to_string()))?;
    if !top.status.success() {
        return Err(unknown("git names no checkout here".to_owned()));
    }
    let top = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
    let script = top.join("scripts/source-identity.sh");
    let answer = Command::new("sh").arg(&script).current_dir(&top).output().map_err(|f| unknown(f.to_string()))?;
    let source = String::from_utf8_lossy(&answer.stdout).trim().to_owned();
    if answer.status.success() && !source.is_empty() {
        Ok(source)
    } else {
        Err(unknown(String::from_utf8_lossy(&answer.stderr).trim().to_owned()))
    }
}

fn image(digest: String, reference: &str, commit: &str, branch: &str) -> RunnerImage {
    RunnerImage {
        digest,
        tag: tag_of(reference).to_owned(),
        commit: commit.to_owned(),
        branch: branch.to_owned(),
        registered_at: WallClock::now(),
        compiler: COMPILER.to_owned(),
        accepts: this_build_accepts(),
        default: true,
    }
}

/// The tag `reference` names, if any: what follows the last `:` of its last path segment, its digest set aside, so
/// `studio-simrunner:dev` is `dev`, `localhost:5000/studio-simrunner` has none, and neither has
/// `…/studio-simrunner@sha256:…`.
fn tag_of(reference: &str) -> &str {
    let named = reference.split_once('@').map_or(reference, |(named, _)| named);
    let last = named.rsplit_once('/').map_or(named, |(_, last)| last);
    last.split_once(':').map_or("", |(_, tag)| tag)
}

/// `GET /api/runner-images`: the registered images; the one image, or none when no invoker is set up.
pub async fn list(State(state): State<AppState>) -> Json<RunnerImages> {
    Json(RunnerImages { items: state.runs.image.iter().map(|image| (**image).clone()).collect() })
}

#[cfg(test)]
mod tests {
    use std::{env, fs, process};

    use super::*;

    #[test]
    fn a_tag_is_the_last_segments_and_a_digest_is_no_tag() {
        assert_eq!(tag_of("studio-simrunner:dev"), "dev");
        assert_eq!(tag_of("us-docker.pkg.dev/greymatter-development/apps/studio-simrunner@sha256:0f0f"), "");
        assert_eq!(tag_of("registry.example/apps/studio-simrunner:v3@sha256:0f0f"), "v3");
        assert_eq!(tag_of("localhost:5000/studio-simrunner"), "");
    }

    /// A local image names what the engine says of it, its id and the build baked into it, and only an image of the
    /// source this checkout holds is taken: neither another commit's nor the same commit's with other uncommitted
    /// changes, since its closed values are this build's.
    #[test]
    fn a_local_image_is_what_the_engine_says_and_only_of_this_checkouts_source() {
        let baked = BakedImage {
            id: "sha256:4b1d".to_owned(),
            commit: "c0ffee".to_owned(),
            branch: "main".to_owned(),
            source: "c0ffee".to_owned(),
        };
        let image = local_image("studio-simrunner:dev-5eed", &baked, "c0ffee").expect("this checkout's build");
        assert_eq!((image.digest.as_str(), image.tag.as_str()), ("sha256:4b1d", "dev-5eed"));
        assert_eq!((image.commit.as_str(), image.branch.as_str()), ("c0ffee", "main"));
        for other in ["decade", "c0ffee+5eed"] {
            let refused = local_image("studio-simrunner:dev-5eed", &baked, other).expect_err("another source's build");
            assert!(refused.contains("the source c0ffee,") && refused.contains(&format!("holds {other}")), "{refused}");
        }
    }

    /// This checkout's source is named as dev.sh names it: its commit, then a hash of its uncommitted changes when it
    /// has any.
    #[test]
    fn this_checkouts_source_begins_with_its_commit() {
        let head = Command::new("git").args(["rev-parse", "HEAD"]).output().expect("git answers").stdout;
        let source = checkout_source().expect("this checkout's source");
        assert!(source.starts_with(String::from_utf8_lossy(&head).trim()), "{source}");
    }

    /// A checkout with uncommitted changes names another source than its commit's, whatever file the change is in, so
    /// an image built from the commit alone is refused; and once the change is reverted the source is the commit's
    /// again, so an image built while the change stood is refused in its turn and the next start rebuilds.
    #[test]
    fn a_change_names_another_source_and_its_revert_the_commits_again() {
        let repository = env::temp_dir().join(format!("studio-source-{}-{}", process::id(), line!()));
        fs::create_dir_all(repository.join("scripts")).unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/source-identity.sh");
        fs::copy(script, repository.join("scripts/source-identity.sh")).unwrap();
        fs::write(repository.join("simulator.rs"), "fn run() {}\n").unwrap();
        let git = |args: &[&str]| {
            let answer = Command::new("git").args(args).current_dir(&repository).output().expect("git runs");
            assert!(answer.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&answer.stderr));
            String::from_utf8_lossy(&answer.stdout).trim().to_owned()
        };
        git(&["init", "-q"]);
        git(&["add", "."]);
        git(&["-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "a commit"]);
        let commit = git(&["rev-parse", "HEAD"]);
        assert_eq!(source_in(&repository).unwrap(), commit, "a clean tree is its commit");
        fs::write(repository.join("simulator.rs"), "fn run() { stale() }\n").unwrap();
        let edited = source_in(&repository).unwrap();
        assert!(edited.starts_with(&format!("{commit}+")), "{edited}");
        fs::write(repository.join("new.rs"), "fn new() {}\n").unwrap();
        let added = source_in(&repository).unwrap();
        assert!(added.starts_with(&format!("{commit}+")) && added != edited, "an untracked file is a change: {added}");
        fs::remove_file(repository.join("new.rs")).unwrap();
        git(&["checkout", "-q", "--", "simulator.rs"]);
        assert_eq!(source_in(&repository).unwrap(), commit, "the change reverted, the commit's source again");
        fs::remove_dir_all(&repository).unwrap();
    }
}
