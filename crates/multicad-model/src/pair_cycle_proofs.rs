//! Application-owned, detached evidence for completed native gear cycles.
//! Native objects and current-pose witnesses never enter this store.
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const DEFAULT_ENTRIES: usize = 64;
const DEFAULT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize)]
pub struct PairCycleProofStats {
    pub entries: usize,
    pub retained_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub published: u64,
    pub evicted: u64,
    pub stale_publications: u64,
    pub unavailable: bool,
}
struct Entry {
    phases: Arc<[f64]>,
    used: u64,
}
struct State {
    entries: BTreeMap<String, Entry>,
    epoch: Option<u64>,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
    stats: PairCycleProofStats,
}
/// Only exact keys and immutable phase angles cross native worker boundaries.
/// A new application/server owns a new store; this is never project data.
#[derive(Clone)]
pub struct PairCycleProofStore(Arc<Mutex<State>>);
impl Default for PairCycleProofStore {
    fn default() -> Self {
        Self::new(DEFAULT_ENTRIES, DEFAULT_BYTES)
    }
}
impl PairCycleProofStore {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self(Arc::new(Mutex::new(State {
            entries: BTreeMap::new(),
            epoch: Some(0),
            clock: 0,
            max_entries: max_entries.min(DEFAULT_ENTRIES),
            max_bytes: max_bytes.min(DEFAULT_BYTES),
            stats: Default::default(),
        })))
    }
    pub fn stats(&self) -> PairCycleProofStats {
        self.0
            .lock()
            .map(|s| s.stats.clone())
            .unwrap_or(PairCycleProofStats {
                unavailable: true,
                ..Default::default()
            })
    }
    /// In-flight evaluations from the previous epoch cannot repopulate it.
    /// Clearing a native EvaluationCache does not clear an injected app store.
    pub fn clear(&self) {
        if let Ok(mut s) = self.0.lock() {
            s.entries.clear();
            s.stats.entries = 0;
            s.stats.retained_bytes = 0;
            s.epoch = s.epoch.and_then(|e| e.checked_add(1));
        }
    }
    fn epoch(&self) -> Option<u64> {
        self.0.lock().ok()?.epoch
    }
    fn lookup(&self, key: &str) -> Option<Vec<f64>> {
        let mut s = self.0.lock().ok()?;
        s.clock = s.clock.saturating_add(1);
        let clock = s.clock;
        let phases = s.entries.get_mut(key).map(|e| {
            e.used = clock;
            e.phases.to_vec()
        });
        if phases.is_some() {
            s.stats.hits += 1;
        } else {
            s.stats.misses += 1;
        }
        phases
    }
    fn publish(&self, proof: PendingProof) {
        let Ok(mut s) = self.0.lock() else {
            return;
        };
        if s.epoch != Some(proof.epoch) {
            s.stats.stale_publications += 1;
            return;
        }
        let bytes = proof.bytes();
        if s.max_entries == 0 || bytes > s.max_bytes {
            return;
        }
        if let Some(old) = s.entries.remove(&proof.key) {
            s.stats.retained_bytes -= proof.key.len() + old.phases.len() * size_of::<f64>();
        }
        while s.entries.len() >= s.max_entries || s.stats.retained_bytes > s.max_bytes - bytes {
            let Some(oldest) = s
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            let old = s.entries.remove(&oldest).expect("selected entry");
            s.stats.retained_bytes -= oldest.len() + old.phases.len() * size_of::<f64>();
            s.stats.evicted += 1;
        }
        s.clock = s.clock.saturating_add(1);
        let used = s.clock;
        s.entries.insert(
            proof.key,
            Entry {
                phases: proof.phases,
                used,
            },
        );
        s.stats.entries = s.entries.len();
        s.stats.retained_bytes += bytes;
        s.stats.published += 1;
    }
}

thread_local! {
    static CURRENT: RefCell<Option<PairCycleProofStore>> = const { RefCell::new(None) };
    static PUBLICATIONS: RefCell<Vec<Vec<PendingProof>>> = const { RefCell::new(Vec::new()) };
}
/// Explicitly inject a store into synchronous consumers and worker creation.
/// Worker constructors capture a clone; no process-global store is allocated.
pub fn with_pair_cycle_proofs<R>(store: PairCycleProofStore, run: impl FnOnce() -> R) -> R {
    let _scope = ProofOwner::enter(Some(store));
    run()
}
/// The explicit scope's detached handle, for transferring into a new owner.
pub fn current_pair_cycle_proofs() -> Option<PairCycleProofStore> {
    CURRENT.with(|s| s.borrow().clone())
}
pub(crate) struct ProofOwner(Option<PairCycleProofStore>);
impl ProofOwner {
    pub(crate) fn enter(store: Option<PairCycleProofStore>) -> Self {
        Self(CURRENT.with(|s| s.replace(store)))
    }
}
impl Drop for ProofOwner {
    fn drop(&mut self) {
        CURRENT.with(|s| {
            s.replace(self.0.take());
        });
    }
}
struct PendingProof {
    store: PairCycleProofStore,
    epoch: u64,
    key: String,
    phases: Arc<[f64]>,
}
impl PendingProof {
    fn bytes(&self) -> usize {
        self.key.len() + self.phases.len() * size_of::<f64>()
    }
}
fn append_bounded(target: &mut Vec<PendingProof>, proofs: impl IntoIterator<Item = PendingProof>) {
    let mut bytes: usize = target.iter().map(PendingProof::bytes).sum();
    for proof in proofs {
        let size = proof.bytes();
        if target.len() < DEFAULT_ENTRIES && size <= DEFAULT_BYTES.saturating_sub(bytes) {
            bytes += size;
            target.push(proof);
        }
    }
}
pub(crate) struct CycleTransaction {
    store: Option<PairCycleProofStore>,
    epoch: Option<u64>,
}
impl CycleTransaction {
    pub(crate) fn new(store: Option<PairCycleProofStore>) -> Self {
        let epoch = store.as_ref().and_then(PairCycleProofStore::epoch);
        Self { store, epoch }
    }
    pub(crate) fn lookup(&self, key: &str) -> Option<Vec<f64>> {
        self.store.as_ref()?.lookup(key)
    }
    pub(crate) fn stage(&mut self, key: String, phases: Vec<f64>) {
        if let (Some(store), Some(epoch)) = (&self.store, self.epoch) {
            PUBLICATIONS.with(|slot| {
                if let Some(pending) = slot.borrow_mut().last_mut() {
                    append_bounded(
                        pending,
                        [PendingProof {
                            store: store.clone(),
                            epoch,
                            key,
                            phases: phases.into(),
                        }],
                    );
                }
            });
        }
    }
}
/// Nested template compiles join their outer compile's success boundary.
/// Failure or unwinding drops new receipts, including successful child work.
pub(crate) struct PublicationScope(bool);
impl PublicationScope {
    pub(crate) fn begin() -> Self {
        PUBLICATIONS.with(|s| s.borrow_mut().push(Vec::new()));
        Self(true)
    }
    pub(crate) fn finish(mut self, success: bool) {
        let publish = PUBLICATIONS.with(|s| {
            let mut stack = s.borrow_mut();
            let pending = stack.pop().expect("compile publication scope");
            self.0 = false;
            if !success {
                return Vec::new();
            }
            if let Some(parent) = stack.last_mut() {
                append_bounded(parent, pending);
                Vec::new()
            } else {
                pending
            }
        });
        // Never hold a TLS borrow or the store lock during native callbacks.
        for proof in publish {
            proof.store.clone().publish(proof);
        }
    }
}
impl Drop for PublicationScope {
    fn drop(&mut self) {
        if self.0 {
            PUBLICATIONS.with(|s| {
                s.borrow_mut().pop();
            });
        }
    }
}

#[cfg(test)]
pub(crate) fn pending_entries() -> usize {
    PUBLICATIONS.with(|s| s.borrow().iter().map(Vec::len).sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stage(store: &PairCycleProofStore, key: &str) {
        CycleTransaction::new(Some(store.clone())).stage(key.into(), vec![0., 1.]);
    }
    fn complete(store: &PairCycleProofStore, key: &str) {
        let scope = PublicationScope::begin();
        stage(store, key);
        scope.finish(true);
    }
    #[test]
    fn nested_success_waits_for_outer_success_and_failure_or_panic_discards() {
        let store = PairCycleProofStore::default();
        let outer = PublicationScope::begin();
        complete(&store, "child");
        assert_eq!(store.stats().entries, 0);
        outer.finish(false);
        assert_eq!(store.stats().entries, 0);
        let outer = PublicationScope::begin();
        complete(&store, "child");
        stage(&store, "parent");
        outer.finish(true);
        assert_eq!(store.stats().entries, 2);
        let _ = std::panic::catch_unwind(|| {
            let _outer = PublicationScope::begin();
            complete(&store, "abandoned");
            panic!("abandon outer compile");
        });
        assert!(store.lookup("abandoned").is_none());
        complete(&store, "after unwind");
        assert_eq!(store.stats().entries, 3);
        let outer = PublicationScope::begin();
        let child = PublicationScope::begin();
        stage(&store, "failed child");
        child.finish(false);
        outer.finish(true);
        assert!(store.lookup("failed child").is_none());
    }
    #[test]
    fn clear_refuses_inflight_completion_and_new_store_never_imports_old_entries() {
        let store = PairCycleProofStore::default();
        complete(&store, "old");
        let scope = PublicationScope::begin();
        let mut transaction = CycleTransaction::new(Some(store.clone()));
        store.clear();
        transaction.stage("inflight".into(), vec![0.]);
        scope.finish(true);
        assert_eq!(store.stats().entries, 0);
        assert_eq!(store.stats().stale_publications, 1);
        complete(&store, "fresh");
        assert!(store.lookup("fresh").is_some());
        let other = PairCycleProofStore::default();
        assert!(other.lookup("fresh").is_none());
    }
    #[test]
    fn count_bytes_lru_and_replacement_are_bounded() {
        let store = PairCycleProofStore::new(2, 40);
        complete(&store, "a");
        complete(&store, "b");
        assert!(store.lookup("a").is_some());
        complete(&store, "c");
        assert!(store.lookup("b").is_none());
        assert!(store.lookup("a").is_some());
        assert_eq!(store.stats().retained_bytes, 34);
        complete(&store, "a");
        assert_eq!(store.stats().retained_bytes, 34);
        assert_eq!(store.stats().evicted, 1);
        complete(&store, &"oversized".repeat(40));
        assert_eq!(store.stats().entries, 2);
        assert_eq!(store.stats().retained_bytes, 34);
        let disabled = PairCycleProofStore::new(0, 40);
        complete(&disabled, "a");
        assert_eq!(disabled.stats().entries, 0);
        let bytes = PairCycleProofStore::new(64, 20);
        complete(&bytes, "a");
        complete(&bytes, "b");
        assert_eq!(bytes.stats().entries, 1);
        assert!(bytes.lookup("a").is_none());
    }
    #[test]
    fn pending_child_and_parent_share_one_bounded_accumulator() {
        let store = PairCycleProofStore::default();
        let outer = PublicationScope::begin();
        for i in 0..64 {
            complete(&store, &format!("child{i}"));
        }
        stage(&store, "parent overflow");
        PUBLICATIONS.with(|s| assert_eq!(s.borrow().last().unwrap().len(), 64));
        assert_eq!(store.stats().entries, 0);
        outer.finish(true);
        assert_eq!(store.stats().entries, 64);
        assert!(store.lookup("parent overflow").is_none());
        store.clear();
        let scope = PublicationScope::begin();
        stage(&store, &"x".repeat(DEFAULT_BYTES));
        PUBLICATIONS.with(|s| assert!(s.borrow().last().unwrap().is_empty()));
        scope.finish(true);
        assert_eq!(store.stats().entries, 0);
    }
    #[test]
    fn concurrent_misses_are_private_until_success_and_clear_is_a_barrier() {
        let store = PairCycleProofStore::default();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let scope = PublicationScope::begin();
                    let mut tx = CycleTransaction::new(Some(store.clone()));
                    assert!(tx.lookup("same").is_none());
                    tx.stage("same".into(), vec![0., 1.]);
                    barrier.wait();
                    barrier.wait();
                    scope.finish(true);
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(store.stats().entries, 0);
        store.clear();
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(store.stats().entries, 0);
        assert_eq!(store.stats().stale_publications, 2);
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || complete(&store, "same"))
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(store.stats().entries, 1);
        assert_eq!(store.stats().retained_bytes, 20);
    }
    #[test]
    fn explicit_owner_scope_restores_on_nested_return_and_unwind() {
        assert!(current_pair_cycle_proofs().is_none());
        let a = PairCycleProofStore::default();
        with_pair_cycle_proofs(a.clone(), || {
            complete(&current_pair_cycle_proofs().unwrap(), "a");
            let _ = std::panic::catch_unwind(|| {
                with_pair_cycle_proofs(PairCycleProofStore::default(), || panic!("nested"))
            });
            assert!(current_pair_cycle_proofs().unwrap().lookup("a").is_some());
        });
        assert!(current_pair_cycle_proofs().is_none());
    }
    #[test]
    fn poisoned_store_loses_reuse_without_accepting_evidence() {
        let store = PairCycleProofStore::default();
        let copy = store.clone();
        let _ = std::thread::spawn(move || {
            let _lock = copy.0.lock().unwrap();
            panic!("poison");
        })
        .join();
        complete(&store, "not accepted");
        assert!(store.lookup("not accepted").is_none());
        assert!(store.stats().unavailable);
        store.clear();
    }
}
