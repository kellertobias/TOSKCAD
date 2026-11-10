//! Same-owner evidence for exact native Intersect(None), never a connection gate.
//!
//! Callers still enumerate every current physical component and validate source,
//! pose, phase, engagement and query budgets. No native handle, address, mesh or
//! approximate geometry enters this bounded store. Completed proofs publish only
//! after the enclosing full compile's final cancellation checkpoint succeeds.
use multicad_core::project_file::ContentHash;
use multicad_kernel::{GeometryError, GeometryPolicy, Solid};
use serde::Serialize;
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::{Rc, Weak},
};

const MAX_ENTRIES: usize = 256;
const MAX_BYTES: usize = 1024 * 1024;
const FORMAT: &[u8] = b"multicad.native-intersection-none/parallel/1";

#[derive(Clone, Debug, Default, Serialize)]
pub struct NativeIntersectionProofStats {
    pub entries: usize,
    pub retained_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub native_queries: u64,
    pub published: u64,
    pub evicted: u64,
    pub stale_publications: u64,
    pub key_failures: u64,
    pub unavailable: bool,
}
struct Entry {
    key: ContentHash,
    used: u64,
}
struct State {
    entries: Vec<Entry>,
    epoch: Option<u64>,
    clock: u64,
    capacity: usize,
    stats: NativeIntersectionProofStats,
}
#[derive(Clone)]
pub(crate) struct ProofStore(Rc<RefCell<State>>);
impl ProofStore {
    pub(crate) fn new(bytes: usize, entries: usize) -> Self {
        Self(Rc::new(RefCell::new(State {
            entries: Vec::new(),
            epoch: Some(0),
            clock: 0,
            capacity: entries
                .min(MAX_ENTRIES)
                .min(bytes.min(MAX_BYTES) / size_of::<Entry>()),
            stats: Default::default(),
        })))
    }
    pub(crate) fn stats(&self) -> NativeIntersectionProofStats {
        self.0
            .try_borrow()
            .map(|state| {
                let mut stats = state.stats.clone();
                stats.entries = state.entries.len();
                stats.retained_bytes = state.entries.len() * size_of::<Entry>();
                stats
            })
            .unwrap_or(NativeIntersectionProofStats {
                unavailable: true,
                ..Default::default()
            })
    }
    pub(crate) fn clear(&self) {
        let mut state = self.0.borrow_mut();
        state.entries.clear();
        state.epoch = state.epoch.and_then(|epoch| epoch.checked_add(1));
    }
    fn epoch(&self) -> Option<u64> {
        self.0.try_borrow().ok()?.epoch
    }
    fn enabled(&self) -> bool {
        self.0
            .try_borrow()
            .is_ok_and(|state| state.capacity > 0 && state.epoch.is_some())
    }
    fn lookup(&self, epoch: u64, key: ContentHash) -> bool {
        let Ok(mut state) = self.0.try_borrow_mut() else {
            return false;
        };
        if state.epoch != Some(epoch) {
            return false;
        }
        state.clock = state.clock.saturating_add(1);
        let clock = state.clock;
        if let Some(entry) = state.entries.iter_mut().find(|entry| entry.key == key) {
            entry.used = clock;
            state.stats.hits = state.stats.hits.saturating_add(1);
            true
        } else {
            state.stats.misses = state.stats.misses.saturating_add(1);
            false
        }
    }
    fn native_query(&self) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            state.stats.native_queries = state.stats.native_queries.saturating_add(1);
        }
    }
    fn key_failure(&self) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            state.stats.key_failures = state.stats.key_failures.saturating_add(1);
        }
    }
}
struct Pending {
    depth: usize,
    store: Weak<RefCell<State>>,
    epoch: u64,
    key: ContentHash,
}
impl Pending {
    fn publish(self) {
        let Some(store) = self.store.upgrade() else {
            return;
        };
        let Ok(mut state) = store.try_borrow_mut() else {
            return;
        };
        if state.epoch != Some(self.epoch) {
            state.stats.stale_publications = state.stats.stale_publications.saturating_add(1);
            return;
        }
        if state.capacity == 0 {
            return;
        }
        state.clock = state.clock.saturating_add(1);
        let clock = state.clock;
        if let Some(entry) = state.entries.iter_mut().find(|entry| entry.key == self.key) {
            entry.used = clock;
            return;
        }
        if state.entries.len() >= state.capacity {
            let oldest = state
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(index, _)| index)
                .expect("nonempty bounded store");
            state.entries.swap_remove(oldest);
            state.stats.evicted = state.stats.evicted.saturating_add(1);
        }
        if state.entries.try_reserve(1).is_err() {
            return;
        }
        state.entries.push(Entry {
            key: self.key,
            used: clock,
        });
        state.stats.published = state.stats.published.saturating_add(1);
    }
}
struct Frame {
    store: Option<(ProofStore, u64)>,
}
#[derive(Default)]
struct Publications {
    frames: Vec<Frame>,
    pending: Vec<Pending>,
}
thread_local! { static FRAMES: RefCell<Publications> = const { RefCell::new(Publications { frames: Vec::new(), pending: Vec::new() }) }; }
fn stage(state: &mut Publications, mut proof: Pending) {
    let Some(depth) = state.frames.len().checked_sub(1) else {
        return;
    };
    if state.pending.iter().any(|old| {
        old.epoch == proof.epoch && old.key == proof.key && Weak::ptr_eq(&old.store, &proof.store)
    }) {
        return;
    }
    // A single allocation bounds both live values and spare capacity through
    // arbitrarily nested compile frames. Depth tags provide child rollback.
    let limit = MAX_ENTRIES.min(MAX_BYTES / size_of::<Pending>());
    if state.pending.len() >= limit {
        state.pending.remove(0);
    }
    proof.depth = depth;
    if state.pending.try_reserve(1).is_ok() {
        state.pending.push(proof);
    }
}
/// Thread-affine transaction/barrier for nested native evaluators. Even a
/// source-scoped compile creates a barrier, then discards its child's proofs.
pub(crate) struct PublicationScope(bool, PhantomData<Rc<()>>);
impl PublicationScope {
    pub(crate) fn begin(store: Option<ProofStore>) -> Self {
        let store = store.and_then(|store| store.epoch().map(|epoch| (store, epoch)));
        FRAMES.with(|state| state.borrow_mut().frames.push(Frame { store }));
        Self(true, PhantomData)
    }
    pub(crate) fn finish(mut self, success: bool) {
        let ready = FRAMES.with(|state| {
            let mut state = state.borrow_mut();
            state
                .frames
                .pop()
                .expect("native intersection compile scope");
            let depth = state.frames.len();
            self.0 = false;
            if !success {
                state.pending.retain(|proof| proof.depth < depth);
                if depth == 0 {
                    state.pending = Vec::new();
                }
                return vec![];
            }
            if depth == 0 {
                std::mem::take(&mut state.pending)
            } else {
                for proof in &mut state.pending {
                    if proof.depth == depth {
                        proof.depth -= 1;
                    }
                }
                vec![]
            }
        });
        for proof in ready {
            proof.publish();
        }
    }
}
impl Drop for PublicationScope {
    fn drop(&mut self) {
        if self.0 {
            FRAMES.with(|state| {
                let mut state = state.borrow_mut();
                state.frames.pop();
                let depth = state.frames.len();
                state.pending.retain(|proof| proof.depth < depth);
                if depth == 0 {
                    state.pending = Vec::new();
                }
            });
        }
    }
}
fn policy_bits(policy: GeometryPolicy) -> [u64; 4] {
    // Exhaustive destructuring makes a future policy addition a compile error.
    let GeometryPolicy {
        linear_tolerance_mm,
        boolean_fuzzy_mm,
        minimum_feature_mm,
        maximum_coordinate_mm,
    } = policy;
    [
        linear_tolerance_mm,
        boolean_fuzzy_mm,
        minimum_feature_mm,
        maximum_coordinate_mm,
    ]
    .map(f64::to_bits)
}
fn key(a: &Solid, b: &Solid, requested: GeometryPolicy) -> Result<ContentHash, GeometryError> {
    let a_hash = a.exact_geometry_digest()?;
    let b_hash = b.exact_geometry_digest()?;
    let mut bytes = [0_u8; FORMAT.len() + 64 + 12 * 8];
    bytes[..FORMAT.len()].copy_from_slice(FORMAT);
    let mut offset = FORMAT.len();
    for digest in [a_hash, b_hash] {
        bytes[offset..offset + 32].copy_from_slice(&digest.0);
        offset += 32;
    }
    for policy in [a.geometry_policy(), b.geometry_policy(), requested] {
        for value in policy_bits(policy) {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            offset += 8;
        }
    }
    Ok(ContentHash::of(&bytes))
}
/// Exactly the existing parallel native Intersect predicate. True means None;
/// nonempty results and errors are never cached. With no current full compile
/// store this performs the ordinary query without serializing a cache key.
pub fn no_intersection(
    a: &Solid,
    b: &Solid,
    policy: GeometryPolicy,
) -> Result<bool, GeometryError> {
    let context = FRAMES.with(|frames| frames.try_borrow().ok()?.frames.last()?.store.clone());
    let cache_key = context
        .as_ref()
        .filter(|(store, _)| store.enabled())
        .and_then(|(store, _)| match key(a, b, policy) {
            Ok(key) => Some(key),
            Err(_) => {
                store.key_failure();
                None
            }
        });
    if let (Some((store, epoch)), Some(key)) = (&context, cache_key)
        && store.lookup(*epoch, key)
    {
        return Ok(true);
    }
    if let Some((store, _)) = &context {
        store.native_query();
    }
    let empty = a.intersect_parallel(b, policy)?.is_none();
    if empty
        && cache_key.is_some()
        && let Some((store, epoch)) = context
    {
        // OCCT may freeze Free(false) as non-geometric bookkeeping even for a
        // non-destructive query. Retain only the exact currently held operands
        // AFTER the completed None, never a lossy comparison or a flag alias.
        match key(a, b, policy) {
            Ok(key) => FRAMES.with(|frames| {
                stage(
                    &mut frames.borrow_mut(),
                    Pending {
                        depth: 0,
                        store: Rc::downgrade(&store.0),
                        epoch,
                        key,
                    },
                );
            }),
            Err(_) => store.key_failure(),
        }
    }
    Ok(empty)
}

#[cfg(all(test, feature = "occt"))]
mod tests;
