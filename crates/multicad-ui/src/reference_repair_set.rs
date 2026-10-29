//! Staged repair of several broken sketch references. Rows stay uncommitted
//! until a background native check passes the complete candidate; Apply then
//! commits every row as one undoable transaction. Any source, Session or
//! resource change discards the whole set.
use crate::viewport::ReferenceEvaluation;
use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{PreparationSnapshot, Prepared, Session},
    constraint_sketch::{CurveKind, source::Source},
    document::{Document, EntityData, EntityId},
    project_file::ProjectResources,
    references::{ReferenceReport, Resolution, TopologyKey, TopologyKind, TopologySnapshot},
    sketch_external::{self, RepairRow, RepairSet, RepairStamp, RepairStep},
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

/// The native verdict on a staged set. `prepared` exists only when every row
/// staged, the full model rebuilt, every driving reference resolved and every
/// affected sketch solved.
pub struct RepairSetCheck {
    pub rows: Vec<RepairRow>,
    pub sketches: Vec<EntityId>,
    pub blockers: Vec<String>,
    pub prepared: Option<Prepared>,
}
impl RepairSetCheck {
    pub fn can_apply(&self) -> bool {
        self.prepared.is_some() && self.blockers.is_empty()
    }
}
/// Runs the complete native check of a set, off the UI thread, honouring the
/// cancellation flag. The composition root owns the native model.
pub type RepairSetChecker = Arc<
    dyn Fn(
            PreparationSnapshot,
            RepairSet,
            String,
            Arc<AtomicBool>,
        ) -> Result<RepairSetCheck, String>
        + Send
        + Sync,
>;

/// A sketch reference whose current binding does not resolve.
#[derive(Clone, Debug, PartialEq)]
pub struct BrokenReference {
    pub sketch: EntityId,
    pub reference: EntityId,
    /// Feature-source references keep their topology kind; occurrence
    /// references are staged for removal here and rebound through the API.
    pub kind: Option<TopologyKind>,
    pub status: String,
}
/// Own sketch items that use a reference's output geometry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dependents {
    pub constraints: Vec<EntityId>,
    pub curves: Vec<EntityId>,
}

fn status(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Resolved { .. } => "Resolved".into(),
        Resolution::Missing => "Referenced geometry was removed".into(),
        Resolution::MissingSource => "Source geometry is unavailable".into(),
        Resolution::Ambiguous { candidates } => {
            format!("Ambiguous · {} candidates", candidates.len())
        }
        Resolution::Stale { .. } => "Stale geometry".into(),
        Resolution::UnavailableFrame => "Planar frame is unavailable".into(),
        Resolution::Invalid { message } => message.clone(),
    }
}
fn name(document: &Document, id: EntityId) -> String {
    document
        .get(id)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| format!("Missing {id}"))
}

/// Every unresolved feature or occurrence sketch reference in `report`, plus
/// every reference of a sketch in `failed` whose evaluation fails although its
/// binding resolves, such as an intersection that no longer reaches the plane.
pub fn broken_references(
    document: &Document,
    report: &ReferenceReport,
    failed: &BTreeSet<EntityId>,
) -> Vec<BrokenReference> {
    let mut out = vec![];
    for sketch in document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
        .map(|e| e.id)
    {
        for binding in sketch_external::bindings(document, sketch).unwrap_or_default() {
            let failing = report
                .entries
                .iter()
                .find(|d| d.address.owner == binding.reference)
                .filter(|d| !matches!(d.resolution, Resolution::Resolved { .. }));
            let status = match failing {
                Some(diagnostic) => status(&diagnostic.resolution),
                None if failed.contains(&sketch) => "Consumer sketch fails to evaluate".into(),
                None => continue,
            };
            out.push(BrokenReference {
                sketch,
                reference: binding.reference,
                kind: Some(binding.intent.kind),
                status,
            });
        }
        for request in sketch_external::occurrence_bindings(document, sketch).unwrap_or_default() {
            let status = match report.external_errors.get(&request.query.reference) {
                Some(error) => error.clone(),
                None if failed.contains(&sketch) => "Consumer sketch fails to evaluate".into(),
                None => continue,
            };
            out.push(BrokenReference {
                sketch,
                reference: request.query.reference,
                kind: None,
                status,
            });
        }
    }
    out
}

/// Constraints and curves of `sketch` that use the output geometry of
/// `reference`. Removal must name them explicitly.
pub fn dependents(document: &Document, sketch: EntityId, reference: EntityId) -> Dependents {
    let outputs: BTreeSet<EntityId> =
        if sketch_external::is_occurrence_reference(document, reference) {
            sketch_external::read_occurrence(document, reference)
                .map(|r| r.output.all().into_iter().collect())
                .unwrap_or_default()
        } else {
            sketch_external::read(document, reference)
                .map(|b| b.definition.output.all().into_iter().collect())
                .unwrap_or_default()
        };
    let Some(EntityData::Sketch { definition, .. }) = document.get(sketch).map(|e| &e.data) else {
        return Dependents::default();
    };
    let Ok(source) = Source::from_definition(definition) else {
        return Dependents::default();
    };
    let points = |kind: &CurveKind| match kind {
        CurveKind::Line { start, end } => vec![*start, *end],
        CurveKind::Circle { center, .. } => vec![*center],
        CurveKind::Arc {
            center, start, end, ..
        } => vec![*center, *start, *end],
        CurveKind::BSpline { poles, .. } => poles.clone(),
    };
    let curves: Vec<_> = source
        .sketch
        .curves
        .iter()
        .filter(|c| !outputs.contains(&c.id) && points(&c.kind).iter().any(|p| outputs.contains(p)))
        .map(|c| c.id)
        .collect();
    let removed: BTreeSet<_> = outputs.iter().chain(&curves).copied().collect();
    let constraints = source
        .sketch
        .constraints
        .iter()
        .filter(|c| c.kind.references().iter().any(|id| removed.contains(id)))
        .map(|c| c.id)
        .collect();
    Dependents {
        constraints,
        curves,
    }
}

struct Check {
    steps: Vec<RepairStep>,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<RepairSetCheck, String>>,
}
#[derive(Default)]
struct Draft {
    sketch: Option<EntityId>,
    reference: Option<EntityId>,
    kind: Option<TopologyKind>,
    source: Option<EntityId>,
    selected: Option<TopologyKey>,
}

#[derive(Default)]
pub struct RepairSetPanel {
    checker: Option<RepairSetChecker>,
    stamp: Option<RepairStamp>,
    steps: Vec<RepairStep>,
    draft: Draft,
    check: Option<Check>,
    result: Option<(Vec<RepairStep>, RepairSetCheck)>,
    message: Option<(bool, String)>,
}
impl RepairSetPanel {
    pub fn set_checker(&mut self, checker: RepairSetChecker) {
        self.checker = Some(checker);
    }
    pub fn has_checker(&self) -> bool {
        self.checker.is_some()
    }
    pub fn has_pending(&self) -> bool {
        !self.steps.is_empty()
    }
    pub fn checking(&self) -> bool {
        self.check.is_some()
    }
    pub fn steps(&self) -> &[RepairStep] {
        &self.steps
    }
    pub fn result(&self) -> Option<&RepairSetCheck> {
        self.result
            .as_ref()
            .filter(|(steps, _)| steps == &self.steps)
            .map(|(_, r)| r)
    }
    pub fn message(&self) -> Option<&(bool, String)> {
        self.message.as_ref()
    }
    /// Drop every staged row without touching the document.
    pub fn discard(&mut self) {
        if let Some(check) = self.check.take() {
            check.cancel.store(true, Ordering::Relaxed);
        }
        self.stamp = None;
        self.steps.clear();
        self.draft = Draft::default();
        self.result = None;
    }
    fn invalidate(&mut self) {
        if let Some(check) = self.check.take() {
            check.cancel.store(true, Ordering::Relaxed);
        }
        self.result = None;
    }
    /// Discard the set if the document, revision, source or resources moved.
    pub fn synchronize(&mut self, session: &Session, resources: &ProjectResources) -> bool {
        let Some(stamp) = &self.stamp else {
            return true;
        };
        if stamp
            .check(
                session.document(),
                session.revision(),
                Some(&resources.fingerprint()),
            )
            .is_err()
        {
            self.discard();
            self.message = Some((
                true,
                "Source or resources changed; the staged repairs were discarded".into(),
            ));
            return false;
        }
        true
    }
    fn stage(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        rows: Vec<RepairStep>,
    ) -> Result<(), String> {
        self.synchronize(session, resources);
        if self.stamp.is_none() {
            self.stamp = Some(
                RepairStamp::current(
                    session.document(),
                    session.revision(),
                    Some(resources.fingerprint()),
                )
                .map_err(|e| e.to_string())?,
            );
        }
        if self.steps.len() + rows.len() > sketch_external::MAX_REPAIR_STEPS {
            return Err("Too many staged repairs".into());
        }
        let staged: BTreeSet<_> = self
            .steps
            .iter()
            .filter_map(RepairStep::reference)
            .collect();
        if rows
            .iter()
            .filter_map(RepairStep::reference)
            .any(|r| staged.contains(&r))
        {
            return Err("This reference already has a staged repair; unstage it first".into());
        }
        self.steps.extend(rows);
        self.draft = Draft::default();
        self.message = None;
        self.invalidate();
        Ok(())
    }
    /// Stage a replacement of a feature-source reference by an element of a
    /// current source feature's topology.
    pub fn stage_rebind(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        sketch: EntityId,
        reference: EntityId,
        source: EntityId,
        selected: TopologyKey,
    ) -> Result<(), String> {
        self.stage(
            session,
            resources,
            vec![RepairStep::Rebind {
                sketch,
                reference,
                source,
                selected,
            }],
        )
    }
    /// Stage removal of a reference together with the explicitly listed own
    /// constraints and curves that use its output geometry.
    pub fn stage_removal(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        sketch: EntityId,
        reference: EntityId,
        dependents: &Dependents,
    ) -> Result<(), String> {
        let mut rows = vec![];
        if !dependents.constraints.is_empty() || !dependents.curves.is_empty() {
            rows.push(RepairStep::RemoveSketchItems {
                sketch,
                constraints: dependents.constraints.clone(),
                curves: dependents.curves.clone(),
                points: vec![],
            });
        }
        rows.push(RepairStep::Remove { sketch, reference });
        self.stage(session, resources, rows)
    }
    pub fn unstage(&mut self, index: usize) {
        if index < self.steps.len() {
            self.steps.remove(index);
            self.invalidate();
            if self.steps.is_empty() {
                self.stamp = None;
            }
        }
    }
    /// Check the complete candidate natively on a background thread.
    pub fn request_check(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        if !self.synchronize(session, resources) {
            return Err("The staged repairs were discarded".into());
        }
        let stamp = self.stamp.clone().ok_or("Stage at least one repair")?;
        let checker = self
            .checker
            .clone()
            .ok_or("Native model checks are unavailable")?;
        self.invalidate();
        let set = RepairSet {
            stamp,
            steps: self.steps.clone(),
        };
        let snapshot = session.preparation_snapshot();
        let fingerprint = resources.fingerprint();
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, receiver) = mpsc::channel();
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("reference-repair-set".into())
            .spawn(move || {
                let result = checker(snapshot, set, fingerprint, flag.clone());
                if !flag.load(Ordering::Relaxed) {
                    let _ = send.send(result);
                }
            })
            .map_err(|e| e.to_string())?;
        self.check = Some(Check {
            steps: self.steps.clone(),
            cancel,
            receiver,
        });
        self.message = None;
        Ok(())
    }
    /// Adopt a finished check if it still matches the staged rows and source.
    pub fn poll(&mut self, session: &Session, resources: &ProjectResources) -> bool {
        if !self.synchronize(session, resources) {
            return false;
        }
        let Some(check) = &self.check else {
            return false;
        };
        match check.receiver.try_recv() {
            Ok(result) => {
                let check = self.check.take().expect("checked");
                if check.steps != self.steps {
                    return true;
                }
                match result {
                    Ok(prepared) => self.result = Some((check.steps, prepared)),
                    Err(error) => self.message = Some((true, error)),
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.check = None;
                self.message = Some((true, "The repair check stopped unexpectedly".into()));
                true
            }
        }
    }
    pub fn can_apply(&self, session: &Session, resources: &ProjectResources) -> bool {
        self.stamp.as_ref().is_some_and(|s| {
            s.check(
                session.document(),
                session.revision(),
                Some(&resources.fingerprint()),
            )
            .is_ok()
        }) && self.check.is_none()
            && self.result().is_some_and(|r| {
                r.can_apply()
                    && r.prepared
                        .as_ref()
                        .is_some_and(|p| session.validate_preview(p).is_ok())
            })
    }
    /// Commit every staged row as one transaction.
    pub fn apply(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        if !self.can_apply(session, resources) {
            return Err("Check the complete staged set before Apply".into());
        }
        let (_, result) = self.result.take().expect("checked result");
        let rows = self.steps.len();
        session
            .commit(result.prepared.expect("checked can_apply"))
            .map_err(|e| e.to_string())?;
        self.discard();
        self.message = Some((
            false,
            format!(
                "{rows} staged repair row{} applied · one undoable source change",
                if rows == 1 { "" } else { "s" }
            ),
        ));
        Ok(())
    }

    fn row_label(document: &Document, step: &RepairStep) -> String {
        match step {
            RepairStep::Rebind {
                sketch,
                reference,
                source,
                selected,
            } => format!(
                "{} · {} → {} {:?} {}",
                name(document, *sketch),
                name(document, *reference),
                name(document, *source),
                selected.kind,
                selected.ordinal.saturating_add(1)
            ),
            RepairStep::RebindOccurrence { request } => format!(
                "{} · rebind occurrence reference",
                name(document, request.query.sketch)
            ),
            RepairStep::Remove { sketch, reference } => format!(
                "{} · remove {}",
                name(document, *sketch),
                name(document, *reference)
            ),
            RepairStep::RemoveSketchItems {
                sketch,
                constraints,
                curves,
                points,
            } => format!(
                "{} · remove {} constraints, {} curves, {} points",
                name(document, *sketch),
                constraints.len(),
                curves.len(),
                points.len()
            ),
            RepairStep::SourceEdit { label, .. } => label.clone(),
        }
    }

    fn draft_editor(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        snapshots: &std::collections::BTreeMap<EntityId, TopologySnapshot>,
    ) {
        let (Some(sketch), Some(reference)) = (self.draft.sketch, self.draft.reference) else {
            return;
        };
        let document = session.document();
        ui.separator();
        ui.label(format!(
            "Staging {} in {}",
            name(document, reference),
            name(document, sketch)
        ));
        if let Some(kind) = self.draft.kind {
            let sources: Vec<_> = snapshots
                .keys()
                .copied()
                .filter(|id| {
                    matches!(
                        document.get(*id).map(|e| &e.data),
                        Some(EntityData::Feature { .. })
                    )
                })
                .collect();
            egui::ComboBox::from_id_salt("repair-set-source")
                .selected_text(
                    self.draft
                        .source
                        .map(|id| name(document, id))
                        .unwrap_or_else(|| "Replacement source".into()),
                )
                .show_ui(ui, |ui| {
                    for id in sources {
                        if ui
                            .selectable_label(self.draft.source == Some(id), name(document, id))
                            .clicked()
                        {
                            self.draft.source = Some(id);
                            self.draft.selected = None;
                        }
                    }
                });
            if let Some(snapshot) = self.draft.source.and_then(|id| snapshots.get(&id)) {
                egui::ScrollArea::vertical()
                    .id_salt("repair-set-candidates")
                    .max_height(120.)
                    .show(ui, |ui| {
                        for element in snapshot.elements.iter().filter(|e| e.key.kind == kind) {
                            let label =
                                format!("{:?} {}", kind, element.key.ordinal.saturating_add(1));
                            let response = ui
                                .selectable_label(self.draft.selected == Some(element.key), label);
                            let response = match &element.geometry {
                                Some(g) => response.on_hover_text(format!("{g:?}")),
                                None => response,
                            };
                            if response.clicked() {
                                self.draft.selected = Some(element.key);
                            }
                        }
                    });
            }
            let ready = self.draft.source.is_some() && self.draft.selected.is_some();
            if ui
                .add_enabled(ready, egui::Button::new("Stage replacement"))
                .on_disabled_hover_text("Choose a source and a current element of the same kind")
                .clicked()
                && let Err(error) = self.stage_rebind(
                    session,
                    resources,
                    sketch,
                    reference,
                    self.draft.source.expect("ready"),
                    self.draft.selected.expect("ready"),
                )
            {
                self.message = Some((true, error));
            }
        } else {
            ui.small(
                "Occurrence references are rebound through the reference API; stage removal here.",
            );
        }
        let dependents = dependents(document, sketch, reference);
        let removal = if dependents.constraints.is_empty() && dependents.curves.is_empty() {
            "Stage removal".to_string()
        } else {
            format!(
                "Stage removal with {} dependent constraints and {} curves",
                dependents.constraints.len(),
                dependents.curves.len()
            )
        };
        if ui
            .button(removal)
            .on_hover_text("Removes the reference geometry and the listed own items that use it")
            .clicked()
            && let Err(error) =
                self.stage_removal(session, resources, sketch, reference, &dependents)
        {
            self.message = Some((true, error));
        }
        if ui.button("Cancel").clicked() {
            self.draft = Draft::default();
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        evaluation: &ReferenceEvaluation,
        failed: &BTreeSet<EntityId>,
    ) {
        let current = self.synchronize(session, resources);
        if self.poll(session, resources) {
            ui.ctx().request_repaint();
        }
        let broken = broken_references(session.document(), &evaluation.report, failed);
        if broken.is_empty() && !self.has_pending() && self.message.is_none() {
            return;
        }
        ui.separator();
        ui.strong("Staged reference repairs");
        if let Some((error, message)) = &self.message {
            ui.colored_label(
                if *error {
                    Color32::LIGHT_RED
                } else {
                    Color32::LIGHT_BLUE
                },
                message,
            );
        }
        if !current {
            return;
        }
        let staged: BTreeSet<_> = self
            .steps
            .iter()
            .filter_map(RepairStep::reference)
            .collect();
        for item in &broken {
            let label = format!(
                "{} · {} · {}",
                name(session.document(), item.sketch),
                name(session.document(), item.reference),
                item.status
            );
            ui.horizontal(|ui| {
                ui.label(label);
                if staged.contains(&item.reference) {
                    ui.small("staged");
                } else if ui.small_button("Stage…").clicked() {
                    self.draft = Draft {
                        sketch: Some(item.sketch),
                        reference: Some(item.reference),
                        kind: item.kind,
                        ..Default::default()
                    };
                }
            });
        }
        self.draft_editor(ui, session, resources, &evaluation.snapshots);
        if self.steps.is_empty() {
            return;
        }
        ui.separator();
        let rows = self.result().map(|r| r.rows.clone());
        let mut unstage = None;
        for (index, step) in self.steps.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{}. {}",
                    index + 1,
                    Self::row_label(session.document(), step)
                ));
                if ui.small_button("Unstage").clicked() {
                    unstage = Some(index);
                }
            });
            if let Some(row) = rows.as_ref().and_then(|r| r.get(index)) {
                if !row.old.is_empty() {
                    ui.small(format!("Was: {} · Now: {}", row.old, row.new));
                }
                ui.small(format!("Affects {} entities", row.affected.len()));
                if let Some(error) = &row.error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
            }
        }
        if let Some(index) = unstage {
            self.unstage(index);
        }
        if let Some(result) = self.result() {
            for blocker in &result.blockers {
                ui.colored_label(Color32::LIGHT_YELLOW, blocker);
            }
            if result.can_apply() {
                ui.colored_label(
                    Color32::LIGHT_GREEN,
                    "Complete model rebuilds and every reference resolves",
                );
            }
        }
        ui.horizontal(|ui| {
            if self.checking() {
                ui.spinner();
                ui.small("Checking the complete model…");
            } else if ui
                .add_enabled(!self.steps.is_empty(), egui::Button::new("Check repairs"))
                .clicked()
                && let Err(error) = self.request_check(session, resources)
            {
                self.message = Some((true, error));
            }
            let ready = self.can_apply(session, resources);
            if ui
                .add_enabled(ready, egui::Button::new("Apply all"))
                .on_disabled_hover_text(
                    "Every row must stage and the complete model must rebuild with all references resolved",
                )
                .clicked()
                && let Err(error) = self.apply(session, resources)
            {
                self.message = Some((true, error));
            }
            if ui.button("Discard all").clicked() {
                self.discard();
                self.message = Some((false, "Staged repairs discarded · document unchanged".into()));
            }
        });
        if self.checking() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
#[cfg(test)]
#[path = "reference_repair_set_tests.rs"]
mod tests;
