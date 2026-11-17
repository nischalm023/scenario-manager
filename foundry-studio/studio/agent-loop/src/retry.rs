//! Within the agent loop, `retry` is section 3.7's answer to a model that is unavailable: a 429, any 5xx or a lost
//! connection is asked again three times, the waits growing from 2 s and doubling (wall clock), each at least as long
//! as the provider asked; anything else is not asked again. The loop owns the retries, never rig, which retries
//! nothing itself.

use std::time::Duration;

use crate::backend::{ErrorClass, ModelError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrySchedule {
    pub first_wait: Duration,
    pub retries: u32,
}

impl Default for RetrySchedule {
    fn default() -> Self {
        Self { first_wait: Duration::from_secs(2), retries: 3 }
    }
}

impl RetrySchedule {
    /// The wait before asking again after `error` ended attempt `attempt` (0 for the first), or `None` when the
    /// request is not asked again.
    pub fn wait_after(&self, attempt: u32, error: &ModelError) -> Option<Duration> {
        if error.class != ErrorClass::Unavailable || attempt >= self.retries {
            return None;
        }
        let backoff = self.first_wait.saturating_mul(1 << attempt.min(16));
        Some(error.retry_after.map_or(backoff, |asked| asked.max(backoff)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unavailable(retry_after: Option<Duration>) -> ModelError {
        ModelError { class: ErrorClass::Unavailable, status: Some(429), words: "slow down".into(), retry_after }
    }

    /// Spec 01 section 3.7: three retries from 2 s, doubling, honouring a longer `retry-after`; a rejection none.
    #[test]
    fn three_retries_from_two_seconds_doubling() {
        let schedule = RetrySchedule::default();
        let waits: Vec<_> = (0..4).map(|n| schedule.wait_after(n, &unavailable(None))).collect();
        let secs = |s| Some(Duration::from_secs(s));
        assert_eq!(waits, [secs(2), secs(4), secs(8), None]);
        assert_eq!(schedule.wait_after(0, &unavailable(Some(Duration::from_secs(30)))), secs(30));
        let rejected = ModelError { class: ErrorClass::Rejected, ..unavailable(None) };
        assert_eq!(schedule.wait_after(0, &rejected), None);
    }
}
