//! STEP and KiCad import Inspector. The chosen file is read once on a worker
//! through the host's native reader; the transaction is then prepared and its
//! model built on a second worker against an immutable snapshot of the
//! session. Apply commits exactly that checked candidate as one undoable
//! transaction. Cancel, a failed read or a failed check leave the document
//! unchanged.
//!
//! The same Inspector updates an existing import: the reimport plan (matched,
//! new and missing solids; moved, unchanged, added and missing components) is
//! shown before Apply, and missing items are kept, never dropped.
mod inspector;
#[cfg(test)]
pub(crate) mod tests;

use multicad_core::{
    commands::{Actor, Channel, Prepared, Session},
    document::{Document, EntityId},
    model::imports::{self, KicadImport, KicadUpdatePlan, StepFile},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

/// The reader's defaults: keep mounting holes and skip signal pads; envelopes
/// of footprints without a model are 5 mm high.
pub const DEFAULT_MIN_HOLE_MM: f64 = 2.;
pub const DEFAULT_HEIGHT_MM: f64 = 5.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    Step,
    Kicad,
}
impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Step => "STEP",
            Self::Kicad => "KiCad board",
        }
    }
    fn accepts(self, file_name: &str) -> bool {
        let lower = file_name.to_ascii_lowercase();
        match self {
            Self::Step => lower.ends_with(".step") || lower.ends_with(".stp"),
            Self::Kicad => lower.ends_with(".kicad_pcb"),
        }
    }
}

/// A file read natively by the host.
#[derive(Clone, Debug)]
pub enum ReadFile {
    Step(StepFile),
    Kicad(KicadImport),
}

/// Native reading and model building, supplied by the composition root. All
/// methods run on worker threads, never on the window thread.
pub trait MechanicalImportBackend: Send + Sync {
    fn read_step(&self, path: &Path) -> Result<StepFile, String>;
    /// Read a board and every footprint model that can be found.
    fn read_kicad(&self, path: &Path, min_hole_mm: f64) -> Result<KicadImport, String>;
    /// Build the complete candidate model; an error blocks Apply.
    fn verify(&self, document: &Document, revision: u64, cancel: &AtomicBool)
    -> Result<(), String>;
}
pub type MechanicalImportBackendRef = Arc<dyn MechanicalImportBackend>;

/// Create a new import, or update an existing one (a STEP source or a board
/// import feature).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    New,
    Update(EntityId),
}

/// Existing imports of a kind, by entity and name.
pub fn existing(document: &Document, kind: SourceKind) -> Vec<(EntityId, String)> {
    let entities: Vec<_> = match kind {
        SourceKind::Step => imports::step_sources(document).collect(),
        SourceKind::Kicad => imports::boards(document).collect(),
    };
    entities
        .into_iter()
        .map(|e| (e.id, e.name.clone()))
        .collect()
}

/// The import an entity belongs to: a STEP source, a STEP body feature or its
/// Part, a board feature or its Part, or a component feature or its Part.
pub fn import_of(document: &Document, id: EntityId) -> Option<(SourceKind, EntityId)> {
    for e in imports::step_sources(document) {
        if e.id == id {
            return Some((SourceKind::Step, id));
        }
    }
    for e in imports::step_bodies(document) {
        if let Ok(body) = imports::read_step_body(document, e.id)
            && (e.id == id || body.output.body == id)
        {
            return Some((SourceKind::Step, body.source));
        }
    }
    for e in imports::boards(document) {
        if let Ok(board) = imports::read_board(document, e.id)
            && (e.id == id || board.output.body == id)
        {
            return Some((SourceKind::Kicad, e.id));
        }
    }
    for e in imports::components(document) {
        if let Ok(c) = imports::read_component(document, e.id)
            && (e.id == id || c.output.body == id)
        {
            return Some((SourceKind::Kicad, c.board));
        }
    }
    None
}

/// The import to update from a selection, or the only import in the document.
pub fn update_target(
    document: &Document,
    selection: impl IntoIterator<Item = EntityId>,
) -> Result<(SourceKind, EntityId), String> {
    let found: BTreeSet<_> = selection
        .into_iter()
        .filter_map(|id| import_of(document, id))
        .collect();
    match found.len() {
        1 => return Ok(*found.iter().next().expect("one")),
        n if n > 1 => return Err("Select Parts of only one import to update".into()),
        _ => {}
    }
    let all: Vec<_> = existing(document, SourceKind::Step)
        .into_iter()
        .map(|(id, _)| (SourceKind::Step, id))
        .chain(
            existing(document, SourceKind::Kicad)
                .into_iter()
                .map(|(id, _)| (SourceKind::Kicad, id)),
        )
        .collect();
    match all.as_slice() {
        [one] => Ok(*one),
        [] => Err("The document has no STEP or KiCad import to update".into()),
        _ => Err("Select a Part of the STEP or KiCad import to update".into()),
    }
}

/// What Apply will do, from the checked transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    StepNew {
        parts: usize,
    },
    StepUpdate {
        /// (stored solid label, new solid index, geometry changed).
        matched: Vec<(String, usize, bool)>,
        added: Vec<usize>,
        /// Stored solids not found again; kept with their last geometry.
        missing: Vec<String>,
    },
    KicadNew {
        /// (reference, linked STEP model).
        components: Vec<(String, bool)>,
        /// (reference, model path) of models that were not found.
        missing_models: Vec<(String, String)>,
        unsupported: Vec<String>,
    },
    KicadUpdate {
        plan: KicadUpdatePlan,
        /// Models of added components that were not found.
        missing_models: Vec<(String, String)>,
        unsupported: Vec<String>,
    },
}
#[derive(Clone, Debug)]
pub struct Checked {
    pub report: Report,
    prepared: Prepared,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Applied { label: String, summary: String },
    Cancelled,
}

type Job<T> = Receiver<Result<T, String>>;

fn spawn<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<Job<T>, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| Err("The import worker failed unexpectedly".into()));
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}
fn take<T>(rx: &Job<T>) -> Option<Result<T, String>> {
    match rx.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => {
            Some(Err("The import worker stopped without a result".into()))
        }
    }
}

/// Inputs a check depends on; a check is current only while they match.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    revision: u64,
    generation: u64,
}
// One check per open import; boxing the finished candidate buys nothing.
#[allow(clippy::large_enum_variant)]
enum Check {
    Idle,
    Running {
        key: Key,
        rx: Job<Checked>,
        cancel: Arc<AtomicBool>,
    },
    Done {
        key: Key,
        result: Result<Checked, String>,
    },
}
impl Check {
    fn stop(&mut self) {
        if let Self::Running { cancel, .. } = self {
            cancel.store(true, Ordering::Relaxed);
        }
        *self = Self::Idle;
    }
}

pub(crate) struct Ready {
    kind: SourceKind,
    path: PathBuf,
    file_name: String,
    file: Arc<ReadFile>,
    target: Target,
    /// The hole size the board was read with, and the edited value.
    min_hole_mm: f64,
    min_hole_edit: f64,
    /// False while the hole size is still being dragged or typed.
    hole_settled: bool,
    default_height_mm: f64,
    /// Selected footprint references (KiCad only).
    selected: BTreeSet<String>,
    generation: u64,
    check: Check,
    reread: Option<Job<ReadFile>>,
    reread_error: Option<String>,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Reading {
        kind: SourceKind,
        path: PathBuf,
        target: Target,
        rx: Job<ReadFile>,
    },
    Ready(Box<Ready>),
}

#[derive(Default)]
pub struct MechanicalImport {
    state: State,
    error: Option<String>,
    backend: Option<MechanicalImportBackendRef>,
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| "The selected file has no readable name".into())
}
fn read(
    backend: &MechanicalImportBackendRef,
    kind: SourceKind,
    path: PathBuf,
    min_hole_mm: f64,
) -> Result<Job<ReadFile>, String> {
    let backend = Arc::clone(backend);
    spawn("multicad-import-read", move || match kind {
        SourceKind::Step => backend.read_step(&path).map(ReadFile::Step),
        SourceKind::Kicad => backend.read_kicad(&path, min_hole_mm).map(ReadFile::Kicad),
    })
}

impl MechanicalImport {
    pub fn active(&self) -> bool {
        !matches!(self.state, State::Idle)
    }
    /// A file read or a check is running.
    pub fn pending(&self) -> bool {
        match &self.state {
            State::Idle => false,
            State::Reading { .. } => true,
            State::Ready(r) => r.reread.is_some() || matches!(r.check, Check::Running { .. }),
        }
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn kind(&self) -> Option<SourceKind> {
        match &self.state {
            State::Idle => None,
            State::Reading { kind, .. } => Some(*kind),
            State::Ready(r) => Some(r.kind),
        }
    }
    pub fn target(&self) -> Option<Target> {
        match &self.state {
            State::Idle => None,
            State::Reading { target, .. } => Some(*target),
            State::Ready(r) => Some(r.target),
        }
    }
    pub fn file(&self) -> Option<&ReadFile> {
        match &self.state {
            State::Ready(r) => Some(&r.file),
            _ => None,
        }
    }
    /// The current check result, when it matches the options and revision.
    pub fn checked(&self) -> Option<&Result<Checked, String>> {
        match &self.state {
            State::Ready(r) => match &r.check {
                Check::Done { key, result } if key.generation == r.generation => Some(result),
                _ => None,
            },
            _ => None,
        }
    }
    pub fn report(&self) -> Option<&Report> {
        self.checked()?.as_ref().ok().map(|c| &c.report)
    }
    pub fn cancel(&mut self) {
        if let State::Ready(r) = &mut self.state {
            r.check.stop();
        }
        self.state = State::Idle;
        self.error = None;
    }
    /// Start reading a picked file on a worker. Nothing changes until Apply.
    pub fn begin(
        &mut self,
        backend: MechanicalImportBackendRef,
        kind: SourceKind,
        path: PathBuf,
        target: Target,
    ) -> Result<(), String> {
        let name = file_name(&path)?;
        if !kind.accepts(&name) {
            return Err(match kind {
                SourceKind::Step => "Choose a .step or .stp file".into(),
                SourceKind::Kicad => "Choose a .kicad_pcb file".into(),
            });
        }
        self.cancel();
        let rx = read(&backend, kind, path.clone(), DEFAULT_MIN_HOLE_MM)?;
        self.backend = Some(backend);
        self.state = State::Reading {
            kind,
            path,
            target,
            rx,
        };
        Ok(())
    }
    /// Collect worker results and start the check when its inputs changed.
    pub fn poll(&mut self, session: &Session) {
        if let State::Reading {
            kind,
            path,
            target,
            rx,
        } = &self.state
        {
            match take(rx) {
                None => {}
                Some(Err(error)) => {
                    self.state = State::Idle;
                    self.error = Some(error);
                }
                Some(Ok(file)) => {
                    let selected = match &file {
                        ReadFile::Kicad(k) => k
                            .board
                            .footprints
                            .iter()
                            .map(|f| f.reference.clone())
                            .collect(),
                        ReadFile::Step(_) => BTreeSet::new(),
                    };
                    let target = match target {
                        Target::Update(id)
                            if !existing(session.document(), *kind)
                                .iter()
                                .any(|(e, _)| e == id) =>
                        {
                            Target::New
                        }
                        other => *other,
                    };
                    let file_name = file_name(path).unwrap_or_default();
                    self.error = None;
                    self.state = State::Ready(Box::new(Ready {
                        kind: *kind,
                        path: path.clone(),
                        file_name,
                        file: Arc::new(file),
                        target,
                        min_hole_mm: DEFAULT_MIN_HOLE_MM,
                        min_hole_edit: DEFAULT_MIN_HOLE_MM,
                        hole_settled: true,
                        default_height_mm: DEFAULT_HEIGHT_MM,
                        selected,
                        generation: 0,
                        check: Check::Idle,
                        reread: None,
                        reread_error: None,
                    }));
                }
            }
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let State::Ready(ready) = &mut self.state else {
            return;
        };
        if let Some(rx) = &ready.reread
            && let Some(result) = take(rx)
        {
            ready.reread = None;
            match result {
                Ok(file) => {
                    if let ReadFile::Kicad(k) = &file {
                        let known: BTreeSet<_> =
                            k.board.footprints.iter().map(|f| &f.reference).collect();
                        ready.selected.retain(|r| known.contains(r));
                    }
                    ready.file = Arc::new(file);
                    ready.reread_error = None;
                    ready.generation += 1;
                }
                Err(error) => {
                    ready.min_hole_edit = ready.min_hole_mm;
                    ready.reread_error = Some(error);
                }
            }
        }
        if ready.reread.is_some() {
            return;
        }
        if let Check::Running { rx, key, .. } = &ready.check
            && let Some(result) = take(rx)
        {
            ready.check = Check::Done {
                key: key.clone(),
                result,
            };
        }
        let key = Key {
            revision: session.revision(),
            generation: ready.generation,
        };
        let current = match &ready.check {
            Check::Idle => false,
            Check::Running { key: k, .. } | Check::Done { key: k, .. } => *k == key,
        };
        if !current {
            ready.check.stop();
            let cancel = Arc::new(AtomicBool::new(false));
            match ready.start_check(backend, session, Arc::clone(&cancel)) {
                Ok(rx) => ready.check = Check::Running { key, rx, cancel },
                Err(error) => {
                    ready.check = Check::Done {
                        key,
                        result: Err(error),
                    }
                }
            }
        }
    }
    /// Changes options through the same path as the Inspector controls.
    pub(crate) fn edit(&mut self, change: impl FnOnce(&mut Ready)) {
        if let State::Ready(ready) = &mut self.state {
            change(ready);
            ready.after_edit(self.backend.as_ref());
        }
    }
    pub fn set_target(&mut self, target: Target) {
        self.edit(|r| r.target = target);
    }
    pub fn set_components(&mut self, references: impl IntoIterator<Item = String>) {
        self.edit(|r| r.selected = references.into_iter().collect());
    }
    pub fn set_default_height(&mut self, height_mm: f64) {
        self.edit(|r| r.default_height_mm = height_mm);
    }
    /// Re-read the board with another minimum hole size on the worker.
    pub fn set_min_hole(&mut self, min_hole_mm: f64) {
        self.edit(|r| {
            r.min_hole_edit = min_hole_mm;
            r.hole_settled = true;
        });
    }
    pub fn can_apply(&self, session: &Session) -> bool {
        matches!(&self.state, State::Ready(r)
            if r.reread.is_none()
            && matches!(&r.check, Check::Done { key, result: Ok(c) }
                if key.generation == r.generation
                && key.revision == session.revision()
                && c.prepared.base_revision() == session.revision()))
    }
    /// Commit the checked candidate as one undoable transaction.
    pub fn apply(&mut self, session: &mut Session) -> Result<Outcome, String> {
        let State::Ready(ready) = &self.state else {
            return Err("Choose a STEP or KiCad file first".into());
        };
        if ready.reread.is_some() {
            return Err("Wait for the board to be read again".into());
        }
        let checked = match &ready.check {
            Check::Done { key, result } if key.generation == ready.generation => {
                result.as_ref().map_err(|e| e.clone())?
            }
            _ => return Err("Wait for the import check to finish".into()),
        };
        if checked.prepared.base_revision() != session.revision() {
            return Err("The document changed; the import is being checked again".into());
        }
        let label = checked.prepared.label().to_owned();
        let summary = summary(&checked.report);
        session
            .commit(checked.prepared.clone())
            .map_err(|e| e.to_string())?;
        self.state = State::Idle;
        self.error = None;
        Ok(Outcome::Applied { label, summary })
    }
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}
fn summary(report: &Report) -> String {
    match report {
        Report::StepNew { parts } => plural(*parts, "Part"),
        Report::StepUpdate {
            matched,
            added,
            missing,
        } => format!(
            "{} matched, {} new, {} missing (kept)",
            matched.len(),
            added.len(),
            missing.len()
        ),
        Report::KicadNew {
            components,
            missing_models,
            ..
        } => format!(
            "board and {}, {}",
            plural(components.len(), "component"),
            plural(missing_models.len(), "missing model")
        ),
        Report::KicadUpdate { plan, .. } => format!(
            "{} moved, {} unchanged, {} added, {} missing (kept)",
            plan.moved.len(),
            plan.unchanged.len(),
            plan.added.len(),
            plan.missing.len()
        ),
    }
}

impl Ready {
    fn after_edit(&mut self, backend: Option<&MechanicalImportBackendRef>) {
        let hole_changed =
            self.kind == SourceKind::Kicad && (self.min_hole_edit - self.min_hole_mm).abs() > 1e-9;
        if hole_changed && !self.hole_settled {
            return;
        }
        self.generation += 1;
        if let (true, Some(backend)) = (hole_changed, backend) {
            self.check.stop();
            match read(backend, self.kind, self.path.clone(), self.min_hole_edit) {
                Ok(rx) => {
                    self.min_hole_mm = self.min_hole_edit;
                    self.reread = Some(rx);
                    self.reread_error = None;
                }
                Err(error) => {
                    self.min_hole_edit = self.min_hole_mm;
                    self.reread_error = Some(error);
                }
            }
        }
    }
    fn kicad_import(&self, import: &KicadImport) -> KicadImport {
        let all = import
            .board
            .footprints
            .iter()
            .all(|f| self.selected.contains(&f.reference));
        KicadImport {
            components: (!all).then(|| self.selected.iter().cloned().collect()),
            default_height_mm: self.default_height_mm,
            ..import.clone()
        }
    }
    fn start_check(
        &self,
        backend: MechanicalImportBackendRef,
        session: &Session,
        cancel: Arc<AtomicBool>,
    ) -> Result<Job<Checked>, String> {
        let snapshot = session.preparation_snapshot();
        let file = Arc::clone(&self.file);
        let target = self.target;
        let kicad = match &*self.file {
            ReadFile::Kicad(k) => Some(self.kicad_import(k)),
            ReadFile::Step(_) => None,
        };
        spawn("multicad-import-check", move || {
            let session = snapshot.session();
            let revision = session.revision();
            let actor = Actor::local(Channel::Ui);
            let e = |e: multicad_core::commands::CommandError| e.to_string();
            let (report, prepared) = match (&*file, kicad, target) {
                (ReadFile::Step(file), _, Target::New) => {
                    let p =
                        imports::prepare_step_import(session, revision, actor, file).map_err(e)?;
                    (
                        Report::StepNew {
                            parts: p.model.outputs.len(),
                        },
                        p.model.prepared,
                    )
                }
                (ReadFile::Step(file), _, Target::Update(source)) => {
                    let old = imports::read_step_source(session.document(), source)
                        .map_err(|e| e.to_string())?;
                    let (plan, p) =
                        imports::prepare_step_update(session, revision, actor, source, file)
                            .map_err(e)?;
                    let label = |id: &str| {
                        old.solid(id).map_or(id.to_owned(), |s| {
                            if s.name.trim().is_empty() {
                                s.id.clone()
                            } else {
                                format!("{} ({})", s.name, s.id)
                            }
                        })
                    };
                    (
                        Report::StepUpdate {
                            matched: plan
                                .matched
                                .iter()
                                .map(|(id, i, changed)| (label(id), *i, *changed))
                                .collect(),
                            added: plan.added.clone(),
                            missing: plan.missing.iter().map(|id| label(id)).collect(),
                        },
                        p.model.prepared,
                    )
                }
                (ReadFile::Kicad(_), Some(import), Target::New) => {
                    let p = imports::prepare_kicad_import(session, revision, actor, &import)
                        .map_err(e)?;
                    (
                        Report::KicadNew {
                            components: p
                                .components
                                .iter()
                                .map(|(r, _, m)| (r.clone(), m.is_some()))
                                .collect(),
                            missing_models: p.missing_models.clone(),
                            unsupported: p.unsupported.clone(),
                        },
                        p.model.prepared,
                    )
                }
                (ReadFile::Kicad(_), Some(import), Target::Update(board)) => {
                    let (plan, p) =
                        imports::prepare_kicad_update(session, revision, actor, board, &import)
                            .map_err(e)?;
                    let missing_models = import
                        .board
                        .footprints
                        .iter()
                        .filter(|f| plan.added.contains(&f.reference))
                        .filter(|f| !import.models.iter().any(|(r, _)| *r == f.reference))
                        .filter_map(|f| {
                            f.models
                                .first()
                                .map(|m| (f.reference.clone(), m.path.clone()))
                        })
                        .collect();
                    (
                        Report::KicadUpdate {
                            plan,
                            missing_models,
                            unsupported: import.board.unsupported.clone(),
                        },
                        p.prepared,
                    )
                }
                (ReadFile::Kicad(_), None, _) => unreachable!("a board always has options"),
            };
            if cancel.load(Ordering::Relaxed) {
                return Err("Check cancelled".into());
            }
            backend
                .verify(prepared.document(), revision + 1, &cancel)
                .map_err(|e| format!("The imported model does not build: {e}"))?;
            Ok(Checked { report, prepared })
        })
    }
}
