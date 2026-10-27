//! Bounded background evaluator. Native handles never leave this worker;
//! receipts contain owned display/query data and revision-bound opaque keys.
use crate::{
    BooleanOperation, Bounds, Face, GeometryError, GeometryErrorKind, GeometryPolicy, Mesh,
    MeshOptions, PlaneFrame, Profile, RigidTransform, Solid,
};
use multicad_core::{
    document::EntityId,
    references::{Origin, TopologyElement, TopologySnapshot},
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::JoinHandle,
};
static WORKER_IDS: AtomicU64 = AtomicU64::new(1);
fn error(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "geometry worker",
        kind,
        message: message.into(),
    }
}
#[derive(Clone, Debug)]
pub enum GeometryOperation {
    Box {
        size_mm: [f64; 3],
    },
    NamedBox {
        size_mm: [f64; 3],
        feature: EntityId,
    },
    Cylinder {
        frame: PlaneFrame,
        radius_mm: f64,
        height_mm: f64,
    },
    Extrude {
        profile: Profile,
        height_mm: f64,
    },
    NamedExtrude {
        profile: Profile,
        height_mm: f64,
        feature: EntityId,
        curve_origins: Vec<Origin>,
    },
    Transform {
        source: usize,
        placement: RigidTransform,
    },
    Boolean {
        a: usize,
        b: usize,
        operation: BooleanOperation,
    },
    ReadStep {
        path: PathBuf,
    },
}
#[derive(Clone, Debug)]
pub struct GeometryProgram {
    pub revision: u64,
    pub policy: GeometryPolicy,
    pub operations: Vec<GeometryOperation>,
    /// Operation indices in requested part order. Empty results retain a slot.
    pub outputs: Vec<usize>,
    pub mesh: MeshOptions,
}
impl GeometryProgram {
    fn validate(&self) -> Result<(), GeometryError> {
        self.policy.validate()?;
        if self.operations.len() > 4096 || self.outputs.len() > 1024 {
            return Err(error(
                GeometryErrorKind::Limit,
                "Operation/output budget exceeded",
            ));
        }
        let mut outputs = BTreeSet::new();
        for output in &self.outputs {
            if *output >= self.operations.len() || !outputs.insert(*output) {
                return Err(error(
                    GeometryErrorKind::Input,
                    "Missing or duplicate output operation",
                ));
            }
        }
        let mut edges = 0usize;
        for (i, operation) in self.operations.iter().enumerate() {
            let references = match operation {
                GeometryOperation::Transform { source, .. } => vec![*source],
                GeometryOperation::Boolean { a, b, .. } => vec![*a, *b],
                GeometryOperation::Extrude { profile, .. }
                | GeometryOperation::NamedExtrude { profile, .. } => {
                    if profile.outer.len() > 4096
                        || profile.holes.len() > 128
                        || profile.holes.iter().any(|h| h.len() > 4096)
                    {
                        return Err(error(
                            GeometryErrorKind::Limit,
                            "Profile exceeds edge/hole budget",
                        ));
                    }
                    edges = edges.saturating_add(profile.outer.len());
                    for hole in &profile.holes {
                        edges = edges.saturating_add(hole.len());
                    }
                    vec![]
                }
                _ => vec![],
            };
            if let GeometryOperation::NamedExtrude { curve_origins, .. } = operation
                && curve_origins.len() > 4096
            {
                return Err(error(
                    GeometryErrorKind::Limit,
                    "Curve provenance budget exceeded",
                ));
            }
            if references.into_iter().any(|r| r >= i) {
                return Err(error(
                    GeometryErrorKind::Input,
                    "Operations must reference earlier results",
                ));
            }
        }
        if edges > 65536 {
            return Err(error(
                GeometryErrorKind::Limit,
                "Profile data budget exceeded",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolidKey {
    worker: u64,
    generation: u64,
    revision: u64,
    slot: usize,
}
impl SolidKey {
    pub fn revision(self) -> u64 {
        self.revision
    }
}
#[derive(Clone, Debug)]
pub struct SolidSnapshot {
    pub key: SolidKey,
    pub bounds: Bounds,
    pub faces: Vec<Face>,
    pub volume_mm3: f64,
    pub solid_count: u32,
    pub mesh: Mesh,
    pub topology: Vec<TopologyElement>,
}
impl SolidSnapshot {
    /// The compiler supplies the canonical output entity; local ordinals never
    /// identify that entity and cannot escape their evaluated revision.
    pub fn topology_snapshot(&self, owner: EntityId) -> TopologySnapshot {
        TopologySnapshot {
            owner,
            revision: self.key.revision,
            elements: self.topology.clone(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct GeometrySnapshot {
    pub revision: u64,
    pub solids: Vec<Option<SolidSnapshot>>,
}
#[derive(Default)]
struct Publication {
    generation: u64,
    revision: u64,
    closed: bool,
}
struct Guard {
    state: Arc<Mutex<Publication>>,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}
impl Guard {
    fn check(&self) -> Result<(), GeometryError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(error(
                GeometryErrorKind::Cancelled,
                "Geometry request cancelled",
            ));
        }
        let state = self
            .state
            .lock()
            .map_err(|_| error(GeometryErrorKind::Native, "Publication state poisoned"))?;
        self.check_state(&state)
    }
    fn check_state(&self, state: &Publication) -> Result<(), GeometryError> {
        if self.cancelled.load(Ordering::Acquire) || state.closed {
            return Err(error(
                GeometryErrorKind::Cancelled,
                "Geometry request cancelled or worker closed",
            ));
        }
        if state.generation != self.generation {
            return Err(error(
                GeometryErrorKind::Stale,
                "Geometry request was superseded",
            ));
        }
        Ok(())
    }
}
/// Poll with `try_take` from an event loop. `wait` is for headless callers/tests.
pub struct GeometryJob<T> {
    receiver: Receiver<Result<T, GeometryError>>,
    guard: Guard,
    consumed: bool,
}
impl<T> Drop for GeometryJob<T> {
    fn drop(&mut self) {
        self.guard.cancelled.store(true, Ordering::Release);
    }
}
impl<T> GeometryJob<T> {
    pub fn cancel(&self) {
        self.guard.cancelled.store(true, Ordering::Release);
    }
    pub fn try_take(&mut self) -> Result<Option<T>, GeometryError> {
        if self.consumed {
            return Err(error(
                GeometryErrorKind::Input,
                "Job result already consumed",
            ));
        }
        if let Err(error) = self.guard.check() {
            self.consumed = true;
            return Err(error);
        }
        match self.receiver.try_recv() {
            Ok(result) => {
                self.consumed = true;
                self.guard.check()?;
                result.map(Some)
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.consumed = true;
                Err(error(
                    GeometryErrorKind::Native,
                    "Geometry worker disconnected",
                ))
            }
        }
    }
    pub fn wait(self) -> Result<T, GeometryError> {
        self.guard.check()?;
        let result = self
            .receiver
            .recv()
            .map_err(|_| error(GeometryErrorKind::Native, "Geometry worker disconnected"))?;
        self.guard.check()?;
        result
    }
}
enum Command {
    Build {
        program: GeometryProgram,
        generation: u64,
        cancelled: Arc<AtomicBool>,
        reply: SyncSender<Result<GeometrySnapshot, GeometryError>>,
    },
    Step {
        key: SolidKey,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
        reply: SyncSender<Result<(), GeometryError>>,
    },
}
struct Cache {
    generation: u64,
    revision: u64,
    solids: Vec<Option<Solid>>,
}
pub struct GeometryWorker {
    id: u64,
    sender: Option<SyncSender<Command>>,
    state: Arc<Mutex<Publication>>,
    thread: Option<JoinHandle<()>>,
}
impl GeometryWorker {
    pub fn new() -> Result<Self, GeometryError> {
        let id = WORKER_IDS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| error(GeometryErrorKind::Limit, "Worker identity exhausted"))?;
        let (sender, receiver) = mpsc::sync_channel(4);
        let state = Arc::new(Mutex::new(Publication::default()));
        let publication = Arc::clone(&state);
        let thread = std::thread::Builder::new()
            .name("MultiCAD geometry".into())
            .spawn(move || run(id, receiver, publication))
            .map_err(|e| error(GeometryErrorKind::Native, e.to_string()))?;
        Ok(Self {
            id,
            sender: Some(sender),
            state,
            thread: Some(thread),
        })
    }
    pub fn submit(
        &self,
        program: GeometryProgram,
    ) -> Result<GeometryJob<GeometrySnapshot>, GeometryError> {
        program.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(GeometryErrorKind::Native, "Publication state poisoned"))?;
        if state.closed {
            return Err(error(
                GeometryErrorKind::Cancelled,
                "Geometry worker closed",
            ));
        }
        if program.revision < state.revision {
            return Err(error(
                GeometryErrorKind::Stale,
                "Document revision precedes the latest request",
            ));
        }
        let generation = state
            .generation
            .checked_add(1)
            .ok_or_else(|| error(GeometryErrorKind::Limit, "Generation exhausted"))?;
        let revision = program.revision;
        let (reply, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let command = Command::Build {
            program,
            generation,
            cancelled: Arc::clone(&cancelled),
            reply,
        };
        self.send(command)?;
        // The worker takes the same lock before publication, so it cannot see a
        // queued generation until this successful submission is recorded.
        state.generation = generation;
        state.revision = revision;
        drop(state);
        Ok(GeometryJob {
            receiver,
            guard: Guard {
                state: Arc::clone(&self.state),
                generation,
                cancelled,
            },
            consumed: false,
        })
    }
    /// Write exact B-rep geometry to a caller-owned staging path. The application
    /// must check its source revision again before publishing the staged file.
    pub fn write_step_to_staging(
        &self,
        key: SolidKey,
        expected_revision: u64,
        path: PathBuf,
    ) -> Result<GeometryJob<()>, GeometryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| error(GeometryErrorKind::Native, "Publication state poisoned"))?;
        if key.worker != self.id
            || key.revision != expected_revision
            || state.revision != expected_revision
            || state.generation != key.generation
        {
            return Err(error(
                GeometryErrorKind::Stale,
                "Foreign or stale exact geometry key",
            ));
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.send(Command::Step {
            key,
            path,
            cancelled: Arc::clone(&cancelled),
            reply,
        })?;
        Ok(GeometryJob {
            receiver,
            guard: Guard {
                state: Arc::clone(&self.state),
                generation: key.generation,
                cancelled,
            },
            consumed: false,
        })
    }
    fn send(&self, command: Command) -> Result<(), GeometryError> {
        self.sender
            .as_ref()
            .ok_or_else(|| error(GeometryErrorKind::Cancelled, "Geometry worker closed"))?
            .try_send(command)
            .map_err(|e| match e {
                TrySendError::Full(_) => error(
                    GeometryErrorKind::Busy,
                    "Geometry queue is full; retry after polling",
                ),
                TrySendError::Disconnected(_) => {
                    error(GeometryErrorKind::Native, "Geometry worker disconnected")
                }
            })
    }
}
impl Drop for GeometryWorker {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn run(id: u64, receiver: Receiver<Command>, state: Arc<Mutex<Publication>>) {
    let mut cache: Option<Cache> = None;
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Build {
                program,
                generation,
                cancelled,
                reply,
            } => {
                let guard = Guard {
                    state: Arc::clone(&state),
                    generation,
                    cancelled,
                };
                let result = compute(id, &program, &guard).and_then(|(candidate, snapshot)| {
                    let publication = state.lock().map_err(|_| {
                        error(GeometryErrorKind::Native, "Publication state poisoned")
                    })?;
                    guard.check_state(&publication)?;
                    let old = cache.replace(candidate);
                    drop(publication);
                    drop(old);
                    Ok(snapshot)
                });
                let _ = reply.send(result);
            }
            Command::Step {
                key,
                path,
                cancelled,
                reply,
            } => {
                let guard = Guard {
                    state: Arc::clone(&state),
                    generation: key.generation,
                    cancelled,
                };
                let result = guard.check().and_then(|()| {
                    let cached = cache
                        .as_ref()
                        .filter(|c| c.generation == key.generation && c.revision == key.revision)
                        .ok_or_else(|| {
                            error(GeometryErrorKind::Stale, "Exact geometry was not published")
                        })?;
                    let solid = cached
                        .solids
                        .get(key.slot)
                        .and_then(Option::as_ref)
                        .ok_or_else(|| error(GeometryErrorKind::Input, "Output has no solid"))?;
                    solid.write_step(&path)?;
                    guard.check()
                });
                let _ = reply.send(result);
            }
        }
    }
}
fn compute(
    id: u64,
    program: &GeometryProgram,
    guard: &Guard,
) -> Result<(Cache, GeometrySnapshot), GeometryError> {
    guard.check()?;
    let mut values: Vec<Option<Solid>> = vec![];
    for operation in &program.operations {
        guard.check()?;
        let get = |i: usize| {
            values.get(i).and_then(Option::as_ref).ok_or_else(|| {
                error(
                    GeometryErrorKind::Input,
                    "Operation references empty or missing material",
                )
            })
        };
        let solid = match operation {
            GeometryOperation::Box { size_mm } => Some(Solid::box_solid(*size_mm, program.policy)?),
            GeometryOperation::NamedBox { size_mm, feature } => {
                Some(Solid::box_named(*size_mm, *feature, program.policy)?)
            }
            GeometryOperation::Cylinder {
                frame,
                radius_mm,
                height_mm,
            } => Some(Solid::cylinder(
                *frame,
                *radius_mm,
                *height_mm,
                program.policy,
            )?),
            GeometryOperation::Extrude { profile, height_mm } => {
                Some(Solid::extrude_profile(profile, *height_mm, program.policy)?)
            }
            GeometryOperation::NamedExtrude {
                profile,
                height_mm,
                feature,
                curve_origins,
            } => Some(Solid::extrude_profile_named(
                profile,
                *height_mm,
                *feature,
                curve_origins,
                program.policy,
            )?),
            GeometryOperation::Transform { source, placement } => {
                Some(get(*source)?.transformed(*placement)?)
            }
            GeometryOperation::Boolean { a, b, operation } => {
                get(*a)?.boolean(get(*b)?, *operation, program.policy)?
            }
            GeometryOperation::ReadStep { path } => Some(Solid::read_step(path, program.policy)?),
        };
        values.push(solid);
    }
    let mut snapshots = vec![];
    let mut solids = vec![];
    let mut remaining_triangles = program.mesh.maximum_triangles;
    for (slot, index) in program.outputs.iter().enumerate() {
        guard.check()?;
        let solid = values[*index].take();
        let snapshot = if let Some(solid) = &solid {
            if remaining_triangles == 0 {
                return Err(error(
                    GeometryErrorKind::Limit,
                    "Model display triangle budget exhausted",
                ));
            }
            let mesh = solid.mesh(MeshOptions {
                maximum_triangles: remaining_triangles,
                ..program.mesh
            })?;
            remaining_triangles -= mesh.indices.len() as u32 / 3;
            Some(SolidSnapshot {
                key: SolidKey {
                    worker: id,
                    generation: guard.generation,
                    revision: program.revision,
                    slot,
                },
                bounds: solid.bounds()?,
                faces: solid.faces()?,
                volume_mm3: solid.volume_mm3()?,
                solid_count: solid.solid_count()?,
                mesh,
                topology: solid.provenance.clone(),
            })
        } else {
            None
        };
        solids.push(solid);
        snapshots.push(snapshot);
    }
    guard.check()?;
    Ok((
        Cache {
            generation: guard.generation,
            revision: program.revision,
            solids,
        },
        GeometrySnapshot {
            revision: program.revision,
            solids: snapshots,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_queue_is_nonblocking_and_does_not_supersede_the_last_accepted_job() {
        // A live, undrained receiver controls queue pressure without timing sleeps.
        let (sender, _receiver) = mpsc::sync_channel(4);
        let worker = GeometryWorker {
            id: 123,
            sender: Some(sender),
            state: Arc::new(Mutex::new(Publication::default())),
            thread: None,
        };
        let p = |revision| GeometryProgram {
            revision,
            policy: Default::default(),
            operations: vec![],
            outputs: vec![],
            mesh: Default::default(),
        };
        let mut accepted = vec![];
        for i in 1..=4 {
            accepted.push(worker.submit(p(i)).unwrap());
        }
        assert_eq!(
            worker.submit(p(5)).err().unwrap().kind,
            GeometryErrorKind::Busy
        );
        assert_eq!(worker.state.lock().unwrap().revision, 4);
        assert_eq!(worker.state.lock().unwrap().generation, 4);
        assert!(accepted[3].try_take().unwrap().is_none());
        assert_eq!(
            accepted[0].try_take().unwrap_err().kind,
            GeometryErrorKind::Stale
        );
    }
}
