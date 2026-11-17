//! Within `container`, `watch` is a runner container's life once it is started or adopted, which exists so that no
//! container runs past its deadline, is settled before it exits, or loses what its runner printed: it is stopped at its
//! deadline, asked again until the engine says it stopped; its exit is waited on, asked again while the engine cannot
//! answer; what it left is settled, what its runner printed kept and only then the container removed, each asked again
//! until done. A stopped API process's containers are adopted at the deadlines their labels keep, and one keeping none
//! has its attempt ended and is removed, the rest adopted all the same; one whose attempt's seat that process had
//! freed has what its runner printed kept and is removed now, finishing the clean-up that process left unfinished.

use std::fs::{File, create_dir_all};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use desunits::SimTime;
use run_invocation::{AttemptKey, Fault, HeldAttempt, JobName, RunStore};
use studio_protocol::{EndedBy, Ending, RunState};
use tokio::task::AbortHandle;
use tokio::time::sleep;

use super::super::store::PgRunStore;
use super::ContainerInvoker;
use super::engine::{RUNNER_LABEL, STACK_LABEL, absent, deadline_format, deadline_of, engine_words, exit_of};
use super::settle::{Exit, settle};

/// How long an engine command that did not answer waits to be asked again, at first and at most (wall clock).
const AGAIN: Duration = Duration::from_secs(1);
const AGAIN_MOST: Duration = Duration::from_secs(30);

/// A container's wall-clock deadline: the allowance it was given, and the instant that falls at, which the container
/// keeps in its labels so that the next API process holds it to the same instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Deadline {
    pub(super) allowance: Duration,
    pub(super) at: DateTime<Utc>,
}

impl Deadline {
    pub(super) fn from_now(allowance: Duration) -> Self {
        let at = TimeDelta::from_std(allowance).ok().and_then(|allowance| Utc::now().checked_add_signed(allowance));
        Self { allowance, at: at.unwrap_or(DateTime::<Utc>::MAX_UTC) }
    }

    /// What is left of it, nothing once it has passed.
    fn left(&self) -> Duration {
        (self.at - Utc::now()).to_std().unwrap_or_default()
    }
}

/// A container this process watches: its deadline, whether that passed, and the task that stops it there, which goes
/// once the container has exited, so it never stops a later container of the name.
#[derive(Debug, Clone)]
pub(super) struct Watched {
    deadline: Deadline,
    deadline_passed: bool,
    stopper: Option<AbortHandle>,
}

impl Watched {
    pub(super) fn new(deadline: Deadline) -> Self {
        Self { deadline, deadline_passed: false, stopper: None }
    }
}

/// What a container of a held attempt says of its deadline, as an adopting process reads it.
enum Kept {
    Deadline(Deadline),
    NoDeadline,
    /// The container went between the listing and the reading; the reconciler meets it absent.
    Gone,
}

impl ContainerInvoker {
    /// Stop the container at its deadline, and settle what it left when it exits.
    pub(super) fn watch(&self, name: JobName, key: AttemptKey, deadline: Deadline) {
        let (invoker, stopping) = (self.clone(), name.clone());
        let stopper = tokio::spawn(async move {
            sleep(deadline.left()).await;
            invoker.deadline_passed(&stopping).await;
        });
        if let Some(watched) = self.watched.lock().expect("the watched containers' lock").get_mut(&name) {
            watched.stopper = Some(stopper.abort_handle());
        }
        let invoker = self.clone();
        tokio::spawn(async move {
            let words = invoker.wait(&name).await;
            invoker.exited(&name, &key, &words).await;
        });
    }

    /// The deadline of the container of `name` passed while it ran: that is marked, and the container stopped, asked
    /// again (`AGAIN` doubling to `AGAIN_MOST`) until the engine says it stopped or is gone. Its seat stays held all the
    /// while, since its status is the engine's.
    async fn deadline_passed(&self, name: &JobName) {
        let watched = self
            .watched
            .lock()
            .expect("the watched containers' lock")
            .get_mut(name)
            .map(|watched| watched.deadline_passed = true)
            .is_some();
        if !watched {
            return;
        }
        let grace = self.setup.stop_grace.as_secs().to_string();
        let (mut again, mut told) = (AGAIN, false);
        loop {
            let why = match self.run(&["stop", "-t", &grace, name.as_str()]).await {
                Ok(answer) if answer.status.success() || absent(&answer) => return,
                Ok(answer) => engine_words(&answer),
                Err(fault) => fault.to_string(),
            };
            if !told {
                eprintln!("studio-api: {name} ran past its deadline and was not stopped, and is asked again: {why}");
                told = true;
            }
            sleep(again).await;
            again = (again * 2).min(AGAIN_MOST);
        }
    }

    /// The container of `name`'s exit in words, once the engine says it exited or is gone. An engine that cannot answer
    /// is asked again, `AGAIN` apart and doubling to `AGAIN_MOST`, for as long as it takes: settling on its silence
    /// would end, and then remove, a container that still runs.
    async fn wait(&self, name: &JobName) -> String {
        let (mut again, mut told) = (AGAIN, false);
        loop {
            let why = match self.run(&["wait", name.as_str()]).await {
                Ok(answer) => match exit_of(&answer) {
                    Some(exit) => return exit,
                    None => engine_words(&answer),
                },
                Err(fault) => fault.to_string(),
            };
            if !told {
                eprintln!("studio-api: {name}'s container could not be waited on, and is asked again: {why}");
                told = true;
            }
            sleep(again).await;
            again = (again * 2).min(AGAIN_MOST);
        }
    }

    /// The container of `name` exited, as `words` tell: what it left settled; then no longer watched, which lets the
    /// reconciler see the exit; then what its runner printed kept and the container removed.
    async fn exited(&self, name: &JobName, key: &AttemptKey, words: &str) {
        let out = self.setup.out.join(name.as_str());
        let watched = self.watched.lock().expect("the watched containers' lock").get(name).cloned();
        let (deadline, deadline_passed) =
            watched.map_or((Duration::ZERO, false), |watched| (watched.deadline.allowance, watched.deadline_passed));
        let exit = Exit { words, deadline, deadline_passed };
        settle(&PgRunStore::new(self.app.clone()), key, &out, &exit, self.setup.store_again).await;
        let unwatched = self.watched.lock().expect("the watched containers' lock").remove(name);
        if let Some(stopper) = unwatched.and_then(|watched| watched.stopper) {
            stopper.abort();
        }
        self.app.runs.wakes.reconcile();
        self.clean_up(name).await;
    }

    /// What the runner of `name` printed kept beside its folder, and only then its container removed, each asked of the
    /// engine again (`AGAIN` doubling to `AGAIN_MOST`) until it is done: until then the container holds the only copy
    /// of that log, and nothing but this removes it.
    async fn clean_up(&self, name: &JobName) {
        let (mut again, mut kept, mut told) = (AGAIN, false, false);
        loop {
            let fault = if kept {
                match self.remove(name).await {
                    Ok(()) => return,
                    Err(fault) => format!("its container was not removed: {fault}"),
                }
            } else {
                match self.keep_log(name).await {
                    Ok(()) => {
                        kept = true;
                        continue;
                    }
                    Err(fault) => format!("what its runner printed was not kept: {fault}"),
                }
            };
            if !told {
                eprintln!("studio-api: {name} exited, and {fault}; it is asked again");
                told = true;
            }
            sleep(again).await;
            again = (again * 2).min(AGAIN_MOST);
        }
    }

    /// What the runner of `name` printed, both streams, kept in `{name}.log` beside its output folder; nothing to keep
    /// when its container went first.
    async fn keep_log(&self, name: &JobName) -> Result<(), Fault> {
        create_dir_all(&self.setup.out)?;
        let log = File::create(self.setup.out.join(format!("{name}.log")))?;
        let kept = self.engine().args(["logs", name.as_str()]).stdout(log.try_clone()?).stderr(log).status().await?;
        if kept.success() {
            return Ok(());
        }
        if absent(&self.run(&["inspect", name.as_str()]).await?) {
            eprintln!("studio-api: {name}'s container went before what its runner printed was kept");
            return Ok(());
        }
        Err(format!("the engine's logs answered {kept}").into())
    }

    /// The container of `name` removed, or already gone.
    async fn remove(&self, name: &JobName) -> Result<(), Fault> {
        let answer = self.run(&["rm", "-f", name.as_str()]).await?;
        if answer.status.success() || absent(&answer) { Ok(()) } else { Err(engine_words(&answer).into()) }
    }

    /// Adopt the containers a stopped API process of this stack left, before the loops start (spec 08 section 3.4: a
    /// seat frees only once its Job is known stopped): every attempt holding a seat whose container the engine knows is
    /// watched until it exits and stopped at the deadline it started with, at once when that has passed. A container
    /// whose attempt's seat is freed exited and was settled, and only what its runner printed and its removal can be
    /// left, which this process finishes. Each adopted Job's name is answered.
    pub async fn adopt(&self) -> Result<Vec<JobName>, Fault> {
        let mut filters = vec![format!("label={RUNNER_LABEL}")];
        filters.extend(self.setup.stack.iter().map(|stack| format!("label={STACK_LABEL}={stack}")));
        let mut listing = vec!["ps", "-a", "--format", "{{.Names}}"];
        for filter in &filters {
            listing.extend(["--filter", filter.as_str()]);
        }
        let listed = self.run(&listing).await?;
        if !listed.status.success() {
            return Err(format!("the engine would not list the runners' containers: {}", engine_words(&listed)).into());
        }
        let names = String::from_utf8_lossy(&listed.stdout);
        let known: Vec<&str> = names.lines().map(str::trim).filter(|name| !name.is_empty()).collect();
        let store = PgRunStore::new(self.app.clone());
        let mut adopted = Vec::new();
        for attempt in store.held().await? {
            if !known.contains(&attempt.job.as_str()) {
                continue;
            }
            match self.kept(&attempt.job).await? {
                Kept::Deadline(deadline) => {
                    self.watched
                        .lock()
                        .expect("the watched containers' lock")
                        .insert(attempt.job.clone(), Watched::new(deadline));
                    self.watch(attempt.job.clone(), attempt.key.clone(), deadline);
                    adopted.push(attempt.job);
                }
                Kept::NoDeadline => self.end_without_deadline(&attempt).await?,
                Kept::Gone => {}
            }
        }
        for name in store.freed(&known).await? {
            eprintln!("studio-api: {name} exited, and what its runner printed is kept and its container removed now");
            let invoker = self.clone();
            tokio::spawn(async move { invoker.clean_up(&name).await });
        }
        Ok(adopted)
    }

    /// The deadline the container of `name` started with, as its labels keep it.
    async fn kept(&self, name: &JobName) -> Result<Kept, Fault> {
        let answer = self.run(&["inspect", "-f", &deadline_format(), name.as_str()]).await?;
        if absent(&answer) {
            return Ok(Kept::Gone);
        }
        if !answer.status.success() {
            return Err(format!("the engine would not read {name}'s deadline: {}", engine_words(&answer)).into());
        }
        Ok(deadline_of(&String::from_utf8_lossy(&answer.stdout)).map_or(Kept::NoDeadline, Kept::Deadline))
    }

    /// The container of `attempt` keeps no deadline in its labels, so no process can hold it to one: only a build older
    /// than those labels starts such a container. Its attempt ends saying so, and what its runner printed is kept and
    /// the container removed, after the adoption, which goes on.
    async fn end_without_deadline(&self, attempt: &HeldAttempt) -> Result<(), Fault> {
        eprintln!(
            "studio-api: {}'s container keeps no deadline, so it is not adopted: attempt {} ends and the container goes",
            attempt.job, attempt.key
        );
        PgRunStore::new(self.app.clone()).end(&attempt.key, &found_without_deadline()).await?;
        let (invoker, name) = (self.clone(), attempt.job.clone());
        tokio::spawn(async move { invoker.clean_up(&name).await });
        Ok(())
    }
}

/// The ending of an attempt whose container was found keeping no deadline: failed, by the platform, saying so.
fn found_without_deadline() -> Ending {
    Ending {
        state: RunState::Failed,
        ended_by: EndedBy::Platform,
        stop_reason:
            "its container was found without its deadline, so it was removed rather than left to run unbounded"
                .to_owned(),
        stop_detail: None,
        sim_ms: SimTime::ZERO,
    }
}
