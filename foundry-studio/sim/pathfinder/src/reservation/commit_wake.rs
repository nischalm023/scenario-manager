//! Within `reservation`, `commit_wake` is how an await on a held cell learns
//! that the table has changed that cell: every commit takes a new count,
//! stamps it on each cell it changes and wakes the awaits registered on those
//! cells. `reserve_path` awaits here between the windows of a journey.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use desgraph::CellId;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

#[derive(Default)]
pub(super) struct CommitWake {
    stamps: Mutex<Stamps>,
}

/// Which commit last changed each cell, and the awaits registered on each
/// cell. A commit wakes every await on the cells it changes; which woken
/// robot then reaches a cell first is decided by the order its executor polls
/// the woken futures in, not here.
#[derive(Default)]
struct Stamps {
    count: u64,
    stamped: FxHashMap<CellId, u64>,
    registered: FxHashMap<CellId, Vec<(AwaitId, Waker)>>,
    next_await: u64,
}

/// One await's registrations, so it can take them back from every cell it
/// awaited once it resolves or is dropped.
#[derive(Clone, Copy, PartialEq, Eq)]
struct AwaitId(u64);

impl Stamps {
    fn withdraw(&mut self, cells: &[CellId], id: AwaitId) {
        for cell in cells {
            if let Some(waiting) = self.registered.get_mut(cell) {
                waiting.retain(|(registered, _)| *registered != id);
                if waiting.is_empty() {
                    self.registered.remove(cell);
                }
            }
        }
    }
}

impl CommitWake {
    pub(super) fn count(&self) -> u64 {
        self.stamps.lock().count
    }

    /// One commit changing `cells`: a new count, stamped on each, and the
    /// awaits on them woken outside the lock.
    pub(super) fn touch(&self, cells: impl IntoIterator<Item = CellId>) {
        let woken: Vec<Waker> = {
            let stamps = &mut *self.stamps.lock();
            stamps.count += 1;
            let count = stamps.count;
            let mut woken = Vec::new();
            for cell in cells {
                stamps.stamped.insert(cell, count);
                woken.extend(stamps.registered.remove(&cell).into_iter().flatten().map(|(_, waker)| waker));
            }
            woken
        };
        for waker in woken {
            waker.wake();
        }
    }

    /// Resolves once a commit counted after `seen` has changed any of
    /// `cells`. A level-triggered caller reads a spurious wake as nothing new.
    pub(super) fn touching<'a>(&'a self, cells: &'a [CellId], seen: u64) -> CommitTouching<'a> {
        CommitTouching { wake: self, cells, seen, id: None }
    }
}

/// An await on a commit touching any of `cells`, registered on each of them
/// while it is pending and withdrawn from all of them when it resolves or is
/// dropped, so a cell's registrations are only the awaits still pending.
pub(super) struct CommitTouching<'a> {
    wake: &'a CommitWake,
    cells: &'a [CellId],
    seen: u64,
    id: Option<AwaitId>,
}

impl Future for CommitTouching<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        let stamps = &mut *this.wake.stamps.lock();
        let id = *this.id.get_or_insert_with(|| {
            stamps.next_await += 1;
            AwaitId(stamps.next_await)
        });
        if this.cells.iter().any(|cell| stamps.stamped.get(cell).is_some_and(|&at| at > this.seen)) {
            stamps.withdraw(this.cells, id);
            this.id = None;
            return Poll::Ready(());
        }
        for &cell in this.cells {
            let waiting = stamps.registered.entry(cell).or_default();
            match waiting.iter_mut().find(|(registered, _)| *registered == id) {
                Some((_, waker)) => waker.clone_from(cx.waker()),
                None => waiting.push((id, cx.waker().clone())),
            }
        }
        Poll::Pending
    }
}

impl Drop for CommitTouching<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.wake.stamps.lock().withdraw(self.cells, id);
        }
    }
}
