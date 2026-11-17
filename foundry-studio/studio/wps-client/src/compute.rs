//! Within `wps-client`, `compute` is the door to spec 01 section 3.15's compute pool: the flagged export's parse and
//! closure check are CPU-bound and run there, never on a thread serving routes, streams or turns. The pool itself is
//! the API's; this crate states what it takes, and a stand-in on tokio's blocking threads for the runner, the seed
//! and tests.

use std::time::Duration;

use tokio::sync::oneshot;

use crate::error::WpsError;

/// A full queue: spec 01's `compute-busy`, with how long to wait in wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputeBusy {
    pub retry_after: Duration,
}

/// A pool that runs computations, or refuses one at once when its queue is full.
pub trait Compute: Send + Sync {
    fn submit(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy>;
}

/// Each computation on tokio's blocking threads, never refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockingCompute;

impl Compute for BlockingCompute {
    fn submit(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy> {
        tokio::task::spawn_blocking(job);
        Ok(())
    }
}

/// Run `work` on `pool` and wait for its answer.
pub async fn run<T: Send + 'static>(
    pool: &dyn Compute,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, WpsError> {
    let (answer, answered) = oneshot::channel();
    pool.submit(Box::new(move || {
        let _ = answer.send(work());
    }))
    .map_err(|busy| WpsError::ComputeBusy { retry_after: busy.retry_after })?;
    answered.await.map_err(|_| WpsError::ComputeLost)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool that drops every computation unrun, as a pool shutting down would.
    struct Dropping;

    impl Compute for Dropping {
        fn submit(&self, job: Box<dyn FnOnce() + Send>) -> Result<(), ComputeBusy> {
            drop(job);
            Ok(())
        }
    }

    /// The chore after world-proto b9d82a8 (kiQJVO): a computation that ends without an answer, dropped unrun or
    /// panicking, is `ComputeLost`, Agent Studio's own fault, and says so; before, it was `Invalid`, worded as a
    /// request world-proto-server refused as malformed.
    #[tokio::test]
    async fn a_lost_computation_is_its_own_kind_and_says_so() {
        let dropped = run(&Dropping, || 1).await.expect_err("dropped unrun");
        let panicked = run(&BlockingCompute, || -> u32 { panic!("a computation's own fault") }).await;
        assert_eq!((dropped.clone(), panicked.expect_err("panicked")), (WpsError::ComputeLost, WpsError::ComputeLost));
        let (sentence, problem) = (dropped.to_string(), dropped.api_problem());
        assert!(sentence.contains("ended without an answer") && !sentence.contains("malformed"), "{sentence}");
        assert_eq!((problem.type_, problem.status), ("internal", 500));
        assert!(!problem.detail.contains("world-proto-server"), "{}", problem.detail);
    }
}
