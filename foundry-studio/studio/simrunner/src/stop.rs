//! Within `simrunner`, `stop` is the runner's one stop, which exists so that an attempt's ending says whose stop ended
//! it: a person's cancel, which attempt control reads off the attempt's row (`door::control`), or the platform's,
//! asked by SIGTERM or by a control route that refused the runner (`attempt`). The first stop asked reaches the
//! simulation and keeps its origin, which the ending reads. The platform's stop is kept apart, first or not, since it
//! abandons any step of the preparation still running even after a person's cancel, which the preparation would
//! otherwise finish first.

use std::future::pending;
use std::sync::Arc;

use dessim::sim_types::{StopReason, StopSignal};
use tokio::sync::watch::{Sender, channel};

#[derive(Clone, Copy)]
enum Origin {
    Person,
    Platform,
}

/// The runner's external stop: the first request's origin, latched before it reaches the harness, and the platform's
/// request in its words, whenever it came. The simulator's own completion or watchdog may win; classification still
/// reads the actual returned stop reason.
pub struct RunnerStop {
    pub(crate) signal: Arc<StopSignal>,
    first: Sender<Option<Origin>>,
    platform: Sender<Option<String>>,
}

impl RunnerStop {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { signal: StopSignal::new(), first: channel(None).0, platform: channel(None).0 })
    }

    /// A person's cancel, as the attempt's row asks it.
    pub fn cancel(&self) {
        self.request(StopReason::UserStop, Origin::Person);
    }

    /// The platform's stop, in its `words`: SIGTERM, or a control route that refused the runner. Its first words are
    /// kept whether or not a person's cancel came before it.
    pub fn platform_failure(&self, words: String) {
        self.platform.send_if_modified(|platform| {
            let first = platform.is_none();
            if first {
                *platform = Some(words.clone());
            }
            first
        });
        self.request(StopReason::Error(words), Origin::Platform);
    }

    fn request(&self, reason: StopReason, from: Origin) {
        self.first.send_if_modified(|origin| {
            let first = origin.is_none();
            if first {
                *origin = Some(from);
                self.signal.stop(reason);
            }
            first
        });
    }

    /// Only this runner's platform request sends Error through the external signal. Simulator failures return
    /// separately from execution; their presence never consults this origin, even if a control failure raced them.
    pub(crate) fn stopped_by_platform(&self, actual: &StopReason) -> bool {
        matches!(actual, StopReason::Error(_)) && matches!(*self.first.borrow(), Some(Origin::Platform))
    }

    /// Whether the first stop asked was a person's cancel.
    pub(crate) fn cancelled_first(&self) -> bool {
        matches!(*self.first.borrow(), Some(Origin::Person))
    }

    /// Once any stop is asked, a person's or the platform's.
    pub(crate) async fn requested(&self) {
        let _ = self.first.subscribe().wait_for(Option::is_some).await;
    }

    /// Once the platform has asked the stop, before or after a person's cancel: its words.
    pub(crate) async fn terminated(&self) -> String {
        let mut platform = self.platform.subscribe();
        loop {
            if let Some(words) = platform.borrow_and_update().clone() {
                return words;
            }
            // The sender is this stop's own, so the channel closes only once nothing can ask the stop any more.
            if platform.changed().await.is_err() {
                return pending().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;

    /// The platform's stop is heard whether it comes alone or after a person's cancel, in its first words; a cancel
    /// alone is a stop asked and terminates nothing.
    #[tokio::test]
    async fn the_platforms_stop_terminates_after_a_cancel_or_alone_and_any_stop_is_requested() {
        let cancelled = RunnerStop::new();
        cancelled.cancel();
        timeout(Duration::from_secs(1), cancelled.requested()).await.expect("a cancel is a stop asked");
        assert!(
            timeout(Duration::from_millis(200), cancelled.terminated()).await.is_err(),
            "a cancel terminates nothing"
        );
        for cancel_first in [false, true] {
            let stop = RunnerStop::new();
            if cancel_first {
                stop.cancel();
            }
            let waiting = tokio::spawn({
                let stop = stop.clone();
                async move { stop.terminated().await }
            });
            stop.platform_failure("terminated by the platform".to_owned());
            stop.platform_failure("the listener then refused".to_owned());
            let words = timeout(Duration::from_secs(1), waiting).await.expect("the platform's stop is heard").unwrap();
            assert_eq!(words, "terminated by the platform", "a cancel first: {cancel_first}");
            assert_eq!(stop.cancelled_first(), cancel_first);
        }
    }

    #[test]
    fn actual_completion_and_the_first_external_request_keep_their_origin() {
        let stop = RunnerStop::new();
        stop.signal.stop(StopReason::Complete);
        stop.platform_failure("the token was revoked".into());
        let actual = stop.signal.take_reason().unwrap();
        assert!(matches!(actual, StopReason::Complete));
        assert!(!stop.stopped_by_platform(&actual));

        let stop = RunnerStop::new();
        stop.cancel();
        stop.platform_failure("the listener then refused".into());
        let actual = stop.signal.take_reason().unwrap();
        assert!(matches!(actual, StopReason::UserStop));
        assert!(!stop.stopped_by_platform(&actual));

        let stop = RunnerStop::new();
        stop.platform_failure("the listener refused first".into());
        stop.cancel();
        let actual = stop.signal.take_reason().unwrap();
        assert!(stop.stopped_by_platform(&actual));
    }
}
