//! Within `wps-client`, `cache` is spec 04 section 3.11's two caches: exports under the changesets they were read at,
//! and floors under the exact bytes they were compiled from. Each computes a key once however many ask for it at
//! once, never changes what a key holds, and evicts the least recently used entries past its byte budget.
//!
//! The cache owns each computation and runs it on the runtime, so no caller's leaving restarts or loses it, and its
//! one outcome, failure included, is shared with every waiter: the floor's compile (reviews-impl/astra-t12.md finding
//! 5) and the export's read and check alike.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::future::Future;
use std::hash::Hash;
use std::mem;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use rustc_hash::FxHashMap;
use studio_protocol::{DeclaredId, ScopePath, Seq};
use tokio::sync::watch::{self, Receiver};

use crate::error::WpsError;
use crate::snapshot::CutExport;

/// The default budget of each cache: 256 MiB.
pub const DEFAULT_BUDGET: usize = 256 * 1024 * 1024;

/// An export's key: the cut's changesets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportKey {
    pub seqs: BTreeMap<ScopePath, Seq>,
}

/// A floor's key: the export's sha256, the compiler's identity, and the digest of a draft applied in memory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FloorKey {
    pub document: String,
    pub compiler: String,
    pub draft: Option<String>,
}

pub type ExportCache = Flight<ExportKey, Arc<CutExport>>;
pub type FloorCache = Flight<FloorKey, Bytes>;

struct Held<V> {
    value: V,
    bytes: usize,
    used: u64,
}

/// What an owned computation's waiters read: nothing until it ends, then its outcome.
type Outcome<V, E> = Receiver<Option<Result<V, E>>>;

struct State<K, V> {
    held: FxHashMap<K, Held<V>>,
    /// The computations running, by key and by the error type their waiters read, each an [`Outcome`] behind `Any`
    /// with the clock it started at, which names it when it is retired.
    owned: FxHashMap<(K, TypeId), (u64, Box<dyn Any + Send + Sync>)>,
    bytes: usize,
    clock: u64,
}

/// A single-flight cache under a byte budget.
pub struct Flight<K, V> {
    state: Arc<Mutex<State<K, V>>>,
    budget: usize,
    size: fn(&V) -> usize,
}

impl<K: Clone + Eq + Hash, V: Clone> Flight<K, V> {
    pub fn new(budget: usize, size: fn(&V) -> usize) -> Self {
        let state = State { held: FxHashMap::default(), owned: FxHashMap::default(), bytes: 0, clock: 0 };
        Self { state: Arc::new(Mutex::new(state)), budget, size }
    }

    /// The bytes held now.
    pub fn bytes(&self) -> usize {
        self.state.lock().expect("a cache's lock").bytes
    }

    pub fn contains(&self, key: &K) -> bool {
        self.state.lock().expect("a cache's lock").held.contains_key(key)
    }

    /// What `key` holds, if anything, counted as a use.
    pub fn held(&self, key: &K) -> Option<V> {
        let mut state = self.state.lock().expect("a cache's lock");
        state.clock += 1;
        let clock = state.clock;
        let held = state.held.get_mut(key)?;
        held.used = clock;
        Some(held.value.clone())
    }

    /// Keep `value` under `key`, computed by its caller, unless something already is.
    pub fn keep(&self, key: K, value: V) {
        let mut state = self.state.lock().expect("a cache's lock");
        state.clock += 1;
        hold(&mut state, key, value, self.budget, self.size);
    }

    /// What `key` holds, or the outcome of the one computation the cache owns for it: the caller that finds none
    /// running calls `compute` and the cache runs its future on the runtime to the end, whoever stops waiting; every
    /// caller waiting reads that one outcome, an error included, and a value is held though nobody waits any more.
    /// A computation that ends without an outcome, its task panicked or dropped, answers `E::from(ComputeLost)`.
    pub async fn get_or_spawn<E, F, Fut>(&self, key: K, compute: F) -> Result<V, E>
    where
        K: Send + Sync + 'static,
        V: Send + Sync + 'static,
        E: Clone + From<WpsError> + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>> + Send + 'static,
    {
        let (mut outcome, started) = {
            let mut state = self.state.lock().expect("a cache's lock");
            state.clock += 1;
            let clock = state.clock;
            if let Some(held) = state.held.get_mut(&key) {
                held.used = clock;
                return Ok(held.value.clone());
            }
            let flight = (key.clone(), TypeId::of::<E>());
            match state.owned.get(&flight).and_then(|(_, outcome)| outcome.downcast_ref::<Outcome<V, E>>()) {
                Some(outcome) => (outcome.clone(), None),
                None => {
                    let (tell, outcome) = watch::channel(None);
                    state.owned.insert(flight.clone(), (clock, Box::new(outcome.clone())));
                    (outcome, Some((tell, Retire { state: Arc::clone(&self.state), flight, started: clock })))
                }
            }
        };
        if let Some((tell, retire)) = started {
            let computation = compute();
            let (budget, size) = (self.budget, self.size);
            tokio::spawn(async move {
                let outcome = computation.await;
                if let Ok(value) = &outcome {
                    let mut state = retire.state.lock().expect("a cache's lock");
                    hold(&mut state, key, value.clone(), budget, size);
                }
                tell.send_replace(Some(outcome));
                drop(retire);
            });
        }
        let said = outcome.wait_for(Option::is_some).await.ok().and_then(|said| said.clone());
        said.unwrap_or_else(|| Err(E::from(WpsError::ComputeLost)))
    }
}

/// Keep `value` under `key` unless something already is, then evict past the budget.
fn hold<K: Clone + Eq + Hash, V>(state: &mut State<K, V>, key: K, value: V, budget: usize, size: fn(&V) -> usize) {
    if state.held.contains_key(&key) {
        return;
    }
    let bytes = size(&value);
    let used = state.clock;
    state.bytes += bytes;
    state.held.insert(key, Held { value, bytes, used });
    while state.bytes > budget && !state.held.is_empty() {
        let oldest = state.held.iter().min_by_key(|(_, held)| held.used).map(|(key, _)| key.clone());
        let Some(oldest) = oldest else { break };
        if let Some(gone) = state.held.remove(&oldest) {
            state.bytes -= gone.bytes;
        }
    }
}

/// Retires an owned computation from the cache when its task ends, however it ends, after its waiters were told.
struct Retire<K: Eq + Hash, V> {
    state: Arc<Mutex<State<K, V>>>,
    flight: (K, TypeId),
    started: u64,
}

impl<K: Eq + Hash, V> Drop for Retire<K, V> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.owned.get(&self.flight).is_some_and(|(started, _)| *started == self.started) {
            state.owned.remove(&self.flight);
        }
    }
}

pub fn export_cache(budget: usize) -> ExportCache {
    Flight::new(budget, |export| export_bytes(export))
}

/// What an export keeps in memory: its bytes, and each of its holders, an entry and its scope's path.
pub fn export_bytes(export: &CutExport) -> usize {
    let entry = mem::size_of::<(DeclaredId, ScopePath)>();
    export.bytes.len() + export.holders.values().map(|scope| entry + scope.as_str().len()).sum::<usize>()
}

pub fn floor_cache(budget: usize) -> FloorCache {
    Flight::new(budget, Bytes::len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A computation whose task panics tells each waiter so, in the waiter's own error type, and leaves no flight
    /// behind: the next caller computes afresh.
    #[tokio::test]
    async fn a_computation_that_ends_without_an_outcome_is_retired() {
        let floors = floor_cache(1 << 20);
        let key = FloorKey { document: "ab".to_owned(), compiler: "floor1@test".to_owned(), draft: None };
        let lost = floors.get_or_spawn::<WpsError, _, _>(key.clone(), || async { panic!("a compile that panics") });
        assert_eq!(lost.await, Err(WpsError::ComputeLost));
        let found = floors.get_or_spawn::<WpsError, _, _>(key.clone(), || async { Ok(Bytes::from_static(b"floor")) });
        assert_eq!(found.await.expect("computed afresh"), Bytes::from_static(b"floor"));
        assert!(floors.contains(&key));
    }
}
