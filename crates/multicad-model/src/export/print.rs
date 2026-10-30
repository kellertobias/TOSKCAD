//! Print packages: the finished Parts a user chose, optionally with the
//! removable FDM aids serving them, tessellated from current exact solids
//! into one 3MF (one object and one identity build item per Part, in world
//! millimeters) and, on request, one binary STL per Part. The package keeps
//! model coordinates with +Z as the build direction. Nothing here slices,
//! arranges a plate, drives a printer or asserts printability.
use super::{Exporter, checkpoint, failure, mesh};
use crate::Model;
use multicad_core::{
    commands::Actor,
    document::{Document, EntityId},
    exchange::*,
    model,
    project_file::ContentHash,
};
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Upper bound on one written package file.
pub const MAX_PRINT_FILE_BYTES: usize = 512 * 1024 * 1024;
/// Distance treated as resting on the build plane, in millimeters.
const PLANE_MM: f64 = 1e-6;

struct Built {
    preview: PrintPreview,
    generated: Vec<Vec<u8>>,
}
fn io(e: impl ToString) -> ExportError {
    failure(ErrorKind::Io, None, e)
}
/// A filename fragment from a Part name: path separators, reserved and
/// control characters become `_`, so a name never leaves the directory.
fn file_fragment(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    if cleaned.is_empty() {
        "part".into()
    } else {
        cleaned
    }
}
fn stl_suffixes(names: &[&str]) -> Vec<String> {
    let mut used = BTreeSet::new();
    names
        .iter()
        .map(|name| {
            let base = file_fragment(name);
            let mut candidate = format!("-{base}.stl");
            let mut n = 2;
            while !used.insert(candidate.to_lowercase()) {
                candidate = format!("-{base}-{n}.stl");
                n += 1;
            }
            candidate
        })
        .collect()
}
fn process_record(
    document: &Document,
    parameters: &multicad_core::parameters::EvaluatedParameters,
    id: EntityId,
) -> Result<PrintProcess, String> {
    let resolved = model::read_process(document, id)
        .and_then(|d| d.resolve(parameters))
        .map_err(|e| e.to_string())?;
    Ok(PrintProcess {
        id,
        name: document
            .get(id)
            .map_or_else(|| id.to_string(), |e| e.name.clone()),
        layer_height_mm: resolved.layer_height_mm,
        line_width_mm: resolved.line_width_mm,
        min_layers: resolved.min_layers,
        min_lines: resolved.min_lines,
        support_gap_mm: resolved.support_gap_mm,
        fit_clearance_mm: resolved.fit_clearance_mm,
    })
}
/// FDM aid inspection findings for the aids serving the chosen Parts, and
/// plain build-plane facts about the packaged objects.
fn findings(
    document: &Document,
    model: &Model,
    selection: &PrintSelection,
    objects: &[PrintObject],
) -> Vec<PrintFinding> {
    let mut out = vec![];
    let finished: BTreeSet<_> = selection
        .parts
        .iter()
        .filter(|p| p.role == PrintRole::Finished)
        .map(|p| p.part)
        .collect();
    let packaged: BTreeSet<_> = objects.iter().map(|o| o.part).collect();
    match model.fdm_aids(document) {
        Ok(inspections) => {
            for aid in inspections {
                let target = model::read_aid(document, aid.feature)
                    .map(|a| a.target)
                    .ok();
                let included = packaged.contains(&aid.part);
                if !included && !target.is_some_and(|t| finished.contains(&t)) {
                    continue;
                }
                let subject = if included {
                    aid.name.clone()
                } else {
                    format!("{} (left out)", aid.name)
                };
                for issue in aid.issues {
                    out.push(PrintFinding {
                        part: Some(aid.part),
                        subject: subject.clone(),
                        message: issue,
                    });
                }
            }
        }
        Err(e) => out.push(PrintFinding {
            part: e.entity,
            subject: "FDM aid inspection".into(),
            message: e.message,
        }),
    }
    for object in objects {
        let z = object.bounds_mm[0][2];
        if z < -PLANE_MM {
            out.push(PrintFinding {
                part: Some(object.part),
                subject: object.name.clone(),
                message: format!(
                    "Extends {:.3} mm below the build plane (z = 0); the package keeps model coordinates",
                    -z
                ),
            });
        }
    }
    if let Some(lowest) = objects
        .iter()
        .map(|o| o.bounds_mm[0][2])
        .min_by(f64::total_cmp)
        && lowest > PLANE_MM
    {
        out.push(PrintFinding {
            part: None,
            subject: "Package".into(),
            message: format!(
                "Lowest point is z = {lowest:.3} mm; no packaged Part rests on the build plane"
            ),
        });
    }
    out
}
/// Read a generated file back and compare it with what was tessellated and
/// with the exact solids' volume, up to the chord deviation.
fn verify(
    file: &PrintFile,
    bytes: &[u8],
    objects: &[&PrintObject],
    quality: &MeshQuality,
) -> Result<f64, ExportError> {
    let (read, single) = match file.kind {
        PrintFileKind::ThreeMf => (mesh::read_three_mf(bytes), false),
        PrintFileKind::Stl => (mesh::read_stl(bytes), true),
    };
    let read = read.map_err(io)?;
    let mesh_volume: f64 = objects.iter().map(|o| o.mesh_volume_mm3).sum();
    let exact: f64 = objects.iter().map(|o| o.volume_mm3).sum();
    let allowance: f64 = objects
        .iter()
        .map(|o| {
            let [a, b] = o.bounds_mm;
            let [x, y, z] = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
            2. * (x * y + y * z + z * x) * quality.chord_deviation_mm * 1.5 + o.volume_mm3 * 1e-6
        })
        .sum();
    let relative = if single { 1e-5 } else { 1e-6 };
    let structure = match file.kind {
        PrintFileKind::ThreeMf => read.objects == file.objects && read.items == file.objects,
        PrintFileKind::Stl => true,
    };
    if !structure
        || (read.unit_scale - 1.).abs() > 1e-12
        || read.triangles != file.triangles
        || (read.volume_mm3() - mesh_volume).abs() > mesh_volume.abs() * relative + 1e-6
        || (read.volume_mm3() - exact).abs() > allowance
    {
        return Err(failure(
            ErrorKind::Invalid,
            file.part,
            format!(
                "Readback of the generated {} differs from its objects, millimeter unit, triangles or volume",
                match file.kind {
                    PrintFileKind::ThreeMf => "3MF",
                    PrintFileKind::Stl => "STL",
                }
            ),
        ));
    }
    Ok(read.volume_mm3())
}
impl Exporter {
    fn print_provider(&self) -> String {
        format!("print-3mf-stl.1;{}", self.provider())
    }
    fn build_print(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Built, ExportError> {
        if actor.id.trim().is_empty() || actor.id.len() > 256 {
            return Err(failure(
                ErrorKind::Invalid,
                None,
                "Invalid print actor identity",
            ));
        }
        checkpoint(interrupt)?;
        let mut selection = resolve_print(document, revision, request)?;
        let parameters = multicad_core::parameters::evaluate(document)
            .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        let mut processes = vec![];
        let mut found = vec![];
        for id in &selection.processes {
            match process_record(document, &parameters, *id) {
                Ok(process) => processes.push(process),
                Err(message) => found.push(PrintFinding {
                    part: None,
                    subject: document
                        .get(*id)
                        .map_or_else(|| id.to_string(), |e| e.name.clone()),
                    message: format!("Process record cannot be resolved: {message}"),
                }),
            }
        }
        let mut objects = vec![];
        let mut meshes = vec![];
        if !selection.parts.is_empty() {
            let model = self
                .compiler
                .compile_interruptible(document, revision, interrupt)
                .map_err(|e| failure(ErrorKind::Invalid, e.entity, e.message))?;
            if model.snapshot().stamp.source_hash != selection.stamp.source_hash {
                return Err(failure(
                    ErrorKind::Stale,
                    None,
                    "Native model source differs from the print selection",
                ));
            }
            let mut unavailable = vec![];
            for part in &selection.parts {
                checkpoint(interrupt)?;
                let solid = model
                    .snapshot()
                    .bodies
                    .get(&part.part)
                    .and_then(|producer| model.solid(*producer));
                let Some(solid) = solid else {
                    unavailable.push(PrintSkip {
                        entity: part.part,
                        name: part.name.clone(),
                        reason: if model.snapshot().consumed_bodies.contains(&part.part) {
                            "Consumed by a later operation; package the resulting Part".into()
                        } else {
                            "No current unsuppressed solid".into()
                        },
                    });
                    continue;
                };
                let invalid = |e: String| failure(ErrorKind::Invalid, Some(part.part), e);
                let bounds = solid.bounds().map_err(|e| invalid(e.to_string()))?;
                let volume = solid.volume_mm3().map_err(|e| invalid(e.to_string()))?;
                let object =
                    mesh::tessellate(solid, &request.quality, &part.name).map_err(invalid)?;
                objects.push(PrintObject {
                    part: part.part,
                    name: part.name.clone(),
                    role: part.role.clone(),
                    bounds_mm: [bounds.min, bounds.max],
                    volume_mm3: volume,
                    triangles: object.triangles.len(),
                    mesh_volume_mm3: mesh::volume(&object.vertices, &object.triangles),
                });
                meshes.push(object);
            }
            selection
                .parts
                .retain(|p| !unavailable.iter().any(|u| u.entity == p.part));
            selection.skipped.extend(unavailable);
            found.extend(findings(document, &model, &selection, &objects));
        }
        let blocked = if objects.is_empty() {
            Some(match &request.source {
                PrintSource::Parts { parts } if parts.is_empty() => {
                    "Select finished Parts before entering Export, or choose Parts by tag".into()
                }
                PrintSource::Tag { tag } => {
                    format!("No world Part with a current solid carries the tag “{tag}”")
                }
                PrintSource::Parts { .. } => {
                    "None of the chosen entities is a world Part with a current solid".into()
                }
            })
        } else {
            None
        };
        let mut files = vec![];
        let mut generated = vec![];
        if blocked.is_none() {
            let items: Vec<_> = (0..meshes.len()).map(mesh::BuildItem::identity).collect();
            files.push(PrintFile {
                kind: PrintFileKind::ThreeMf,
                part: None,
                suffix: ".3mf".into(),
                objects: meshes.len(),
                triangles: objects.iter().map(|o| o.triangles).sum(),
            });
            generated.push(mesh::three_mf(&meshes, &items, LengthUnit::Millimeter));
            if request.stl_per_part {
                let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
                for ((object, mesh_object), suffix) in
                    objects.iter().zip(&meshes).zip(stl_suffixes(&names))
                {
                    checkpoint(interrupt)?;
                    generated.push(mesh::stl(
                        std::slice::from_ref(mesh_object),
                        &[mesh::BuildItem::identity(0)],
                        LengthUnit::Millimeter,
                        &object.name,
                    ));
                    files.push(PrintFile {
                        kind: PrintFileKind::Stl,
                        part: Some(object.part),
                        suffix,
                        objects: 1,
                        triangles: object.triangles,
                    });
                }
            }
            if let Some(large) = generated
                .iter()
                .position(|b| b.len() > MAX_PRINT_FILE_BYTES)
            {
                return Err(failure(
                    ErrorKind::Budget,
                    files[large].part,
                    "A print package file exceeds 512 MiB; coarsen the mesh quality or choose fewer Parts",
                ));
            }
        }
        let provider = self.print_provider();
        let plan = serde_json::to_vec(&(
            &provider,
            selection.stamp.source_hash,
            request,
            &objects,
            &files,
            &selection.skipped,
            generated
                .iter()
                .map(|b| ContentHash::of(b))
                .collect::<Vec<_>>(),
        ))
        .map_err(|e| failure(ErrorKind::Invalid, None, e))?;
        Ok(Built {
            preview: PrintPreview {
                requested_by: actor.clone(),
                key: PrintKey {
                    source: selection.stamp.clone(),
                    plan_hash: ContentHash::of(&plan),
                },
                provider,
                request: request.clone(),
                selection,
                objects,
                files,
                processes,
                findings: found,
                blocked,
            },
            generated,
        })
    }
    /// Resolve and tessellate the package without writing anything.
    pub fn print_preview(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintPreview, ExportError> {
        Ok(self
            .build_print(document, revision, request, actor, interrupt)?
            .preview)
    }
    /// Rebuild the package from the snapshot, require the previewed plan,
    /// verify every generated file by reading it back, and only then replace
    /// the chosen 3MF (and its STL siblings) atomically. A stale source, a
    /// changed plan, a failed evaluation or a failed readback writes nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn write_print(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: &Actor,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<PrintReceipt, ExportError> {
        expected.source.check(document, revision)?;
        let (directory, stem) = package_target(path)?;
        let built = self.build_print(document, revision, request, actor, interrupt)?;
        if built.preview.key != *expected {
            return Err(failure(
                ErrorKind::Stale,
                None,
                "Print package plan changed; preview again before writing",
            ));
        }
        if let Some(reason) = &built.preview.blocked {
            return Err(failure(ErrorKind::Empty, None, reason));
        }
        let preview = &built.preview;
        let mut volumes = vec![];
        for (file, bytes) in preview.files.iter().zip(&built.generated) {
            let objects: Vec<&PrintObject> = match file.part {
                Some(part) => preview.objects.iter().filter(|o| o.part == part).collect(),
                None => preview.objects.iter().collect(),
            };
            volumes.push(verify(file, bytes, &objects, &request.quality)?);
        }
        checkpoint(interrupt)?;
        let targets: Vec<PathBuf> = preview
            .files
            .iter()
            .map(|f| directory.join(f.filename(&stem)))
            .collect();
        for target in &targets {
            if let Ok(meta) = fs::symlink_metadata(target)
                && (!meta.is_file() || meta.file_type().is_symlink())
            {
                return Err(io(format!(
                    "{} exists and is not a regular file; nothing was written",
                    target.display()
                )));
            }
        }
        let mut staged: Vec<PathBuf> = vec![];
        let cleanup = |staged: &[PathBuf]| {
            for path in staged {
                let _ = fs::remove_file(path);
            }
        };
        for (target, bytes) in targets.iter().zip(&built.generated) {
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let temp = directory.join(format!(".{name}.{}.partial", EntityId::new()));
            let written = (|| -> std::io::Result<()> {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                file.write_all(bytes)?;
                file.sync_all()
            })();
            if let Err(e) = written {
                let _ = fs::remove_file(&temp);
                cleanup(&staged);
                return Err(io(format!(
                    "Could not stage {name}: {e}; nothing was written"
                )));
            }
            staged.push(temp);
        }
        if let Err(e) = checkpoint(interrupt) {
            cleanup(&staged);
            return Err(e);
        }
        let mut files = vec![];
        for (index, (temp, target)) in staged.iter().zip(&targets).enumerate() {
            if let Err(e) = fs::rename(temp, target) {
                cleanup(&staged[index..]);
                let done: Vec<_> = files
                    .iter()
                    .map(|f: &PrintWritten| f.path.clone())
                    .collect();
                return Err(io(format!(
                    "Could not replace {}: {e}; already replaced: {}",
                    target.display(),
                    if done.is_empty() {
                        "none".into()
                    } else {
                        done.join(", ")
                    }
                )));
            }
            let file = &preview.files[index];
            let bytes = &built.generated[index];
            let on_disk = fs::read(target).map_err(io)?;
            if on_disk != *bytes {
                return Err(io(format!(
                    "{} differs from the verified package bytes after writing",
                    target.display()
                )));
            }
            files.push(PrintWritten {
                path: target.to_string_lossy().into_owned(),
                kind: file.kind,
                bytes: bytes.len() as u64,
                content_hash: ContentHash::of(bytes),
                objects: file.objects,
                triangles: file.triangles,
                mesh_volume_mm3: volumes[index],
            });
        }
        Ok(PrintReceipt {
            actor: actor.clone(),
            key: expected.clone(),
            files,
        })
    }
}
/// The existing directory and file stem of a chosen absolute `.3mf` path.
fn package_target(path: &Path) -> Result<(PathBuf, String), ExportError> {
    let invalid = |m: &str| failure(ErrorKind::Invalid, None, m);
    if !path.is_absolute() {
        return Err(invalid("Choose an absolute 3MF file path"));
    }
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("3mf"))
    {
        return Err(invalid("A print package file must end in .3mf"));
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid("Name the 3MF print package"))?;
    let directory = path
        .parent()
        .filter(|p| p.is_dir())
        .ok_or_else(|| invalid("The 3MF package folder does not exist"))?;
    Ok((directory.to_owned(), stem))
}

enum Job {
    Preview {
        document: Document,
        revision: u64,
        request: PrintRequest,
        actor: Actor,
    },
    Write {
        document: Document,
        revision: u64,
        request: PrintRequest,
        expected: PrintKey,
        path: PathBuf,
        actor: Actor,
    },
}
#[derive(Clone)]
struct Info {
    id: EntityId,
    document: EntityId,
    revision: u64,
    actor: Actor,
    write: bool,
}
impl Info {
    fn reply(&self, result: PrintResult) -> PrintReply {
        PrintReply {
            id: self.id,
            document: self.document,
            revision: self.revision,
            actor: self.actor.clone(),
            result,
        }
    }
    fn failed(&self, message: &str) -> PrintReply {
        let error = failure(ErrorKind::Io, None, message);
        self.reply(if self.write {
            PrintResult::Write(Err(error))
        } else {
            PrintResult::Preview(Err(error))
        })
    }
}
struct Queued {
    info: Info,
    job: Job,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct Slots {
    pending: VecDeque<Queued>,
    active: Option<(Info, Arc<AtomicBool>)>,
    replies: VecDeque<PrintReply>,
    closed: bool,
}
struct Shared {
    slots: Mutex<Slots>,
    wake: Condvar,
    stop: AtomicBool,
}
struct Exit(Arc<Shared>);
impl Drop for Exit {
    fn drop(&mut self) {
        let mut slots = self.0.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.closed {
            return;
        }
        slots.closed = true;
        self.0.stop.store(true, Ordering::Release);
        let message = "Print worker exited unexpectedly; accepted jobs failed";
        if let Some((info, _)) = slots.active.take() {
            slots.replies.push_back(info.failed(message));
        }
        while let Some(job) = slots.pending.pop_front() {
            slots.replies.push_back(job.info.failed(message));
        }
        self.0.wake.notify_all();
    }
}
/// A bounded FIFO owner thread for print previews and writes. Native models
/// are built and dropped on this thread only; the UI thread polls replies.
pub struct PrintWorker {
    shared: Arc<Shared>,
}
const MAX_OUTSTANDING: usize = 4;
impl PrintWorker {
    pub fn new(exporter: Exporter) -> Result<Self, String> {
        Self::spawn(move |job, interrupt| match job {
            Job::Preview {
                document,
                revision,
                request,
                actor,
            } => PrintResult::Preview(
                exporter
                    .print_preview(&document, revision, &request, &actor, Some(interrupt))
                    .map(Box::new),
            ),
            Job::Write {
                document,
                revision,
                request,
                expected,
                path,
                actor,
            } => PrintResult::Write(exporter.write_print(
                &document,
                revision,
                &request,
                &expected,
                &path,
                &actor,
                Some(interrupt),
            )),
        })
    }
    fn spawn(
        handler: impl Fn(Job, &dyn Fn() -> bool) -> PrintResult + Send + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("multicad-print".into())
            .spawn(move || {
                let _exit = Exit(Arc::clone(&worker));
                loop {
                    let queued = {
                        let mut slots = worker.slots.lock().unwrap_or_else(|e| e.into_inner());
                        while slots.pending.is_empty() && !slots.closed {
                            slots = worker.wake.wait(slots).unwrap_or_else(|e| e.into_inner());
                        }
                        if slots.closed {
                            return;
                        }
                        let queued = slots.pending.pop_front().expect("pending print job");
                        slots.active = Some((queued.info.clone(), Arc::clone(&queued.cancel)));
                        queued
                    };
                    let interrupt = || {
                        worker.stop.load(Ordering::Acquire) || queued.cancel.load(Ordering::Acquire)
                    };
                    let result = handler(queued.job, &interrupt);
                    let mut slots = worker.slots.lock().unwrap_or_else(|e| e.into_inner());
                    slots.active = None;
                    if !slots.closed {
                        slots.replies.push_back(queued.info.reply(result));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { shared })
    }
    fn enqueue(
        &self,
        job: Job,
        info: impl FnOnce(EntityId) -> Info,
    ) -> Result<EntityId, ExportError> {
        let mut slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        if slots.closed {
            return Err(failure(ErrorKind::Io, None, "Print worker is closed"));
        }
        if slots.pending.len() + slots.replies.len() + usize::from(slots.active.is_some())
            >= MAX_OUTSTANDING
        {
            return Err(failure(
                ErrorKind::Budget,
                None,
                "Print queue is full; four accepted jobs are still outstanding",
            ));
        }
        let id = EntityId::new();
        slots.pending.push_back(Queued {
            info: info(id),
            job,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.shared.wake.notify_one();
        Ok(id)
    }
    pub fn preview(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        let info = |id| Info {
            id,
            document: document.id(),
            revision,
            actor: actor.clone(),
            write: false,
        };
        self.enqueue(
            Job::Preview {
                document: document.clone(),
                revision,
                request: request.clone(),
                actor: actor.clone(),
            },
            info,
        )
    }
    pub fn write(
        &self,
        document: &Document,
        revision: u64,
        request: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        let info = |id| Info {
            id,
            document: document.id(),
            revision,
            actor: actor.clone(),
            write: true,
        };
        self.enqueue(
            Job::Write {
                document: document.clone(),
                revision,
                request: request.clone(),
                expected: expected.clone(),
                path: path.to_owned(),
                actor: actor.clone(),
            },
            info,
        )
    }
    pub fn poll(&self) -> Option<PrintReply> {
        self.shared
            .slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replies
            .pop_front()
    }
    pub fn pending(&self) -> bool {
        let slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.active.is_some() || !slots.pending.is_empty() || !slots.replies.is_empty()
    }
    pub fn cancel(&self, id: EntityId) -> bool {
        let slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        let flag = slots
            .active
            .as_ref()
            .filter(|(info, _)| info.id == id)
            .map(|(_, flag)| flag)
            .or_else(|| {
                slots
                    .pending
                    .iter()
                    .find(|j| j.info.id == id)
                    .map(|j| &j.cancel)
            });
        if let Some(flag) = flag {
            flag.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
}
impl Drop for PrintWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let mut slots = self.shared.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.closed = true;
        slots.pending.clear();
        slots.replies.clear();
        self.shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn stl_names_stay_in_the_folder_and_are_unique() {
        assert_eq!(
            stl_suffixes(&["Lid", "lid", "../etc/passwd", "  ", "a:b"]),
            [
                "-Lid.stl",
                "-lid-2.stl",
                "-_etc_passwd.stl",
                "-part.stl",
                "-a_b.stl"
            ]
        );
    }
    #[test]
    fn package_target_requires_an_absolute_3mf_in_an_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(package_target(Path::new("relative.3mf")).is_err());
        assert!(package_target(&dir.path().join("plate.stl")).is_err());
        assert!(package_target(&dir.path().join("missing/plate.3mf")).is_err());
        let (folder, stem) = package_target(&dir.path().join("plate.3MF")).unwrap();
        assert_eq!((folder.as_path(), stem.as_str()), (dir.path(), "plate"));
    }
    #[test]
    fn worker_is_bounded_and_reports_cancellation() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = Mutex::new(rx);
        let worker = PrintWorker::spawn(move |_, interrupt| {
            let _ = rx.lock().unwrap().recv();
            PrintResult::Preview(if interrupt() {
                Err(failure(ErrorKind::Cancelled, None, "cancelled"))
            } else {
                Err(failure(ErrorKind::Empty, None, "controlled"))
            })
        })
        .unwrap();
        let document = Document::new("Bounded print").unwrap();
        let actor = Actor::local(multicad_core::commands::Channel::Ui);
        let request = PrintRequest::default();
        let ids: Vec<_> = (0..4)
            .map(|_| {
                worker
                    .preview(&document, 0, &request, actor.clone())
                    .unwrap()
            })
            .collect();
        assert_eq!(
            worker
                .preview(&document, 0, &request, actor.clone())
                .unwrap_err()
                .kind,
            ErrorKind::Budget
        );
        assert!(worker.cancel(ids[0]) && worker.cancel(ids[3]));
        for _ in 0..4 {
            tx.send(()).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut replies = vec![];
        while replies.len() < 4 {
            assert!(Instant::now() < deadline);
            match worker.poll() {
                Some(r) => replies.push(r),
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        assert_eq!(replies.iter().map(|r| r.id).collect::<Vec<_>>(), ids);
        let kinds: Vec<_> = replies
            .iter()
            .map(|r| match &r.result {
                PrintResult::Preview(Err(e)) => e.kind.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ErrorKind::Cancelled,
                ErrorKind::Empty,
                ErrorKind::Empty,
                ErrorKind::Cancelled
            ]
        );
        assert!(!worker.pending());
    }
}
