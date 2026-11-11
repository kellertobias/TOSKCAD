use super::{
    AssemblySelection,
    clearance_view::{self, ClearanceMarker},
    widgets::{self, Fields},
};
use crate::{
    controls, egui,
    icons::{self, Icon},
    viewport::{Scene, SceneBackend, SceneReply, SceneStamp, VisibilityFilter},
};
use multicad_core::{
    assembly_inspection::*,
    commands::{Prepared, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{self, JointCoordinate, OccurrencePath},
    parameters::Unit,
    project_file::ProjectResources,
};
use std::sync::Arc;
pub trait AssemblyInspectionBackend {
    fn request(
        &mut self,
        document: &Document,
        query: &AssemblyInspectionQuery,
    ) -> Result<u64, String>;
    fn poll(&mut self) -> Option<AssemblyInspectionReply>;
    fn cancel(&mut self);
}
pub type AssemblyInspectionBackendFactory =
    Arc<dyn Fn() -> Box<dyn AssemblyInspectionBackend> + Send + Sync>;
struct SampleView {
    index: usize,
    stamp: SceneStamp,
    prepared: Prepared,
    scene: Option<Arc<Scene>>,
    pending: bool,
}
pub(super) struct InspectionPanel {
    range: bool,
    saved_study: bool,
    catalog: Vec<(OccurrencePath, String)>,
    a: Option<OccurrencePath>,
    b: Option<OccurrencePath>,
    pairs: Vec<OccurrencePair>,
    joint: Option<EntityId>,
    coordinate: JointCoordinate,
    start: Dimension,
    end: Dimension,
    step: Dimension,
    tolerance: f64,
    required: Dimension,
    fields: Fields,
    backend: Option<Box<dyn AssemblyInspectionBackend>>,
    expected: Option<(u64, AssemblyInspectionQuery)>,
    report: Option<AssemblyInspectionReport>,
    /// Current-pose result shown in the viewport (index into the report).
    selected: Option<usize>,
    selected_sample: Option<usize>,
    sample_view: Option<SampleView>,
    reset_sample_backend: bool,
    reveal: bool,
    fit_requested: bool,
    pub error: Option<String>,
    pub pending: bool,
}
impl InspectionPanel {
    pub(super) fn seed_study(&mut self, study: SavedMotionStudy) {
        self.saved_study = true;
        self.a = study.pairs.first().map(|pair| pair.a.clone());
        self.b = study.pairs.first().map(|pair| pair.b.clone());
        self.pairs = study.pairs;
        self.joint = Some(study.joint);
        self.coordinate = study.coordinate;
        self.start = study.start;
        self.end = study.end;
        self.step = study.step;
        self.tolerance = study.contact_tolerance_mm;
        self.required = widgets::literal(study.required_clearance_mm, "mm");
    }

    pub fn new(
        range: bool,
        document: &Document,
        scene: Option<&Arc<Scene>>,
        selection: &AssemblySelection,
        factory: Option<&AssemblyInspectionBackendFactory>,
    ) -> Self {
        let catalog = scene
            .and_then(|s| s.assembly_evaluation())
            .map(|info| {
                info.leaves
                    .iter()
                    .map(|leaf| (leaf.path.clone(), path_name(document, &leaf.path)))
                    .collect()
            })
            .unwrap_or_default();
        let pairs = selection
            .occurrences
            .windows(2)
            .take(1)
            .map(|p| OccurrencePair {
                a: p[0].clone(),
                b: p[1].clone(),
            })
            .collect();
        Self {
            range,
            saved_study: false,
            catalog,
            a: selection.occurrences.first().cloned(),
            b: selection.occurrences.get(1).cloned(),
            pairs,
            joint: selection.joint,
            coordinate: JointCoordinate::Z,
            start: widgets::literal(0., "mm"),
            end: widgets::literal(100., "mm"),
            step: widgets::literal(10., "mm"),
            tolerance: 1e-7,
            required: widgets::literal(0., "mm"),
            fields: Fields::default(),
            backend: factory.map(|f| f()),
            expected: None,
            report: None,
            selected: None,
            selected_sample: None,
            sample_view: None,
            reset_sample_backend: false,
            reveal: false,
            fit_requested: false,
            error: None,
            pending: false,
        }
    }
    /// Viewport evidence for the selected current-pose result. `None` when no
    /// result is selected; an error when the viewed model is not the evaluated
    /// one, so stale markers are never drawn.
    pub fn marker(
        &self,
        session: &Session,
        scene: &Scene,
    ) -> Option<Result<ClearanceMarker, String>> {
        let report = match self.report.as_ref()? {
            AssemblyInspectionReport::Current(report) => {
                if let Err(error) = report.stamp.check(session.document(), session.revision()) {
                    return Some(Err(error));
                }
                report
            }
            AssemblyInspectionReport::Range(report) => {
                if !self.sample_view.as_ref().is_some_and(|view| {
                    Some(view.index) == self.selected_sample
                        && view.scene.is_some()
                        && view.stamp == scene.stamp()
                }) {
                    return Some(Err(
                        "Wait for the selected sampled pose's native display".into()
                    ));
                }
                if let Err(error) = report.stamp.check(session.document(), session.revision()) {
                    return Some(Err(error));
                }
                report
                    .samples
                    .get(self.selected_sample?)?
                    .result
                    .as_ref()
                    .ok()?
            }
        };
        let required = match self.required_mm(session) {
            Ok(value) => value,
            Err(error) => return Some(Err(error)),
        };
        Some(
            ClearanceMarker::build(report, self.selected?, scene).map(|mut marker| {
                marker.required_minimum_mm = Some(required);
                marker
            }),
        )
    }
    fn required_mm(&self, session: &Session) -> Result<f64, String> {
        let value = self
            .required
            .value_in(Unit::Millimeter, Some(session.parameters()))?;
        if !value.is_finite() || value < 0. {
            return Err("Required minimum clearance must be finite and nonnegative".into());
        }
        Ok(value)
    }
    pub fn sample_pending(&self) -> bool {
        self.sample_view
            .as_ref()
            .is_some_and(|view| view.pending && Some(view.index) == self.selected_sample)
    }
    pub fn display_scene(&self, session: &Session) -> Option<Arc<Scene>> {
        let AssemblyInspectionReport::Range(report) = self.report.as_ref()? else {
            return None;
        };
        report
            .stamp
            .check(session.document(), session.revision())
            .ok()?;
        let view = self.sample_view.as_ref()?;
        if Some(view.index) != self.selected_sample {
            return None;
        }
        view.scene.clone()
    }
    pub fn sync_sample(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        backend: &mut dyn SceneBackend,
        generation: &mut u64,
    ) -> Result<(), String> {
        let result = self.request_sample_scene(session, resources, backend, generation);
        if result.is_err() {
            // A failed submission is retried only after an explicit selection.
            self.selected_sample = None;
            self.selected = None;
            self.sample_view = None;
        }
        result
    }
    fn request_sample_scene(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        backend: &mut dyn SceneBackend,
        generation: &mut u64,
    ) -> Result<(), String> {
        if std::mem::take(&mut self.reset_sample_backend) {
            backend.cancel();
        }
        if self.selected_sample.is_none() && self.sample_view.is_none() {
            return Ok(());
        }
        if self
            .sample_view
            .as_ref()
            .is_some_and(|view| Some(view.index) == self.selected_sample)
        {
            return Ok(());
        }
        backend.cancel();
        self.sample_view = None;
        let Some(index) = self.selected_sample else {
            return Ok(());
        };
        let Some(AssemblyInspectionReport::Range(report)) = &self.report else {
            return Err("No sampled inspection is available".into());
        };
        report.stamp.check(session.document(), session.revision())?;
        let sample = report.samples.get(index).ok_or("Sample no longer exists")?;
        let pair_report = sample.result.as_ref().map_err(|e| e.to_string())?;
        let candidate = report.query.prepare_sample(session.document(), index)?;
        let candidate_stamp =
            InspectionStamp::capture(candidate.prepared.document(), session.revision())?;
        if Some(candidate_stamp) != sample.candidate_stamp || candidate_stamp != pair_report.stamp {
            return Err(
                "Sample display source differs from the inspected pose; Preview again".into(),
            );
        }
        *generation = generation
            .checked_add(1)
            .ok_or("Sample preview generation limit reached")?;
        let stamp = SceneStamp {
            document: candidate_stamp.document,
            revision: candidate_stamp.revision,
            generation: *generation,
        };
        backend.request_evaluation(
            candidate.prepared.document(),
            resources,
            stamp,
            &Default::default(),
        )?;
        self.sample_view = Some(SampleView {
            index,
            stamp,
            prepared: candidate.prepared,
            scene: None,
            pending: true,
        });
        self.error = None;
        Ok(())
    }
    pub fn accept_scene(&mut self, session: &Session, reply: SceneReply) {
        let Some(view) = &self.sample_view else {
            return;
        };
        if !view.pending || view.stamp != reply.stamp || Some(view.index) != self.selected_sample {
            return;
        }
        let result = (|| {
            let Some(AssemblyInspectionReport::Range(report)) = &self.report else {
                return Err("Sample report was cleared".into());
            };
            report.stamp.check(session.document(), session.revision())?;
            let pair = report
                .samples
                .get(view.index)
                .ok_or("Sample no longer exists")?
                .result
                .as_ref()
                .map_err(|e| e.to_string())?;
            let scene = reply.result?;
            if scene.stamp() != view.stamp || !clearance_view::scene_matches(&pair.stamp, &scene) {
                return Err("Native display does not match the selected sampled pose".into());
            }
            super::proof::validate(view.prepared.document(), session.revision(), &scene)?;
            VisibilityFilter::default().apply(&scene, view.prepared.document(), session.revision())
        })();
        match result {
            Ok(scene) => {
                let view = self.sample_view.as_mut().expect("matching sample");
                view.pending = false;
                view.scene = Some(scene);
            }
            Err(error) => {
                self.sample_view = None;
                self.selected_sample = None;
                self.selected = None;
                self.error = Some(error);
            }
        }
    }
    pub fn take_fit(&mut self) -> bool {
        std::mem::take(&mut self.fit_requested)
    }
    pub fn reveal(&self) -> bool {
        self.reveal
    }
    pub fn invalidate(&mut self) {
        if let Some(b) = &mut self.backend {
            b.cancel();
        }
        self.expected = None;
        self.report = None;
        self.selected = None;
        self.selected_sample = None;
        self.sample_view = None;
        self.reset_sample_backend = true;
        self.fit_requested = false;
        self.pending = false;
        self.error = None;
    }
    pub fn request(&mut self, session: &Session) -> Result<(), String> {
        self.invalidate();
        let query = self.query(session)?;
        let request = self
            .backend
            .as_mut()
            .ok_or("The native assembly inspection provider is unavailable")?
            .request(session.document(), &query)?;
        self.expected = Some((request, query));
        self.pending = true;
        self.error = None;
        Ok(())
    }
    fn query(&self, session: &Session) -> Result<AssemblyInspectionQuery, String> {
        self.required_mm(session)?;
        if let Some(e) = &self.fields.error {
            return Err(e.clone());
        }
        let stamp = InspectionStamp::capture(session.document(), session.revision())?;
        let query = if self.range {
            let joint = self.joint.ok_or("Choose a driven relationship")?;
            let (source, _) =
                model::read_joint(session.document(), joint).map_err(|e| e.to_string())?;
            if !source.parameters.kind.allows(self.coordinate) {
                return Err("This relationship locks the selected coordinate".into());
            }
            let unit = if self.coordinate == JointCoordinate::Rotation {
                Unit::Radian
            } else {
                Unit::Millimeter
            };
            AssemblyInspectionQuery::Range(MotionRangeQuery {
                expected: stamp,
                pairs: self.pairs.clone(),
                contact_tolerance_mm: self.tolerance,
                joint,
                coordinate: self.coordinate,
                start: self.start.value_in(unit, Some(session.parameters()))?,
                end: self.end.value_in(unit, Some(session.parameters()))?,
                step: self.step.value_in(unit, Some(session.parameters()))?,
            })
        } else {
            AssemblyInspectionQuery::Current(PairQuery {
                expected: stamp,
                pairs: self.pairs.clone(),
                contact_tolerance_mm: self.tolerance,
            })
        };
        query.validate()?;
        for pair in match &query {
            AssemblyInspectionQuery::Current(q) => &q.pairs,
            AssemblyInspectionQuery::Range(q) => &q.pairs,
        } {
            if [&pair.a, &pair.b]
                .into_iter()
                .any(|p| !self.catalog.iter().any(|(known, _)| known == p))
            {
                return Err(
                    "A selected physical occurrence is unavailable in the current native model"
                        .into(),
                );
            }
        }
        Ok(query)
    }
    pub fn poll(&mut self, session: &Session) {
        for _ in 0..16 {
            let Some(reply) = self.backend.as_mut().and_then(|b| b.poll()) else {
                break;
            };
            let Some((request, query)) = &self.expected else {
                continue;
            };
            if *request != reply.request {
                continue;
            }
            let result = (|| {
                query
                    .stamp()
                    .check(session.document(), session.revision())?;
                if reply.stamp != query.stamp()
                    || reply.query != *query
                    || self.query(session)? != *query
                {
                    return Err(
                        "Native inspection reply belongs to different source or inputs".into(),
                    );
                }
                let report = reply.result?;
                report.validate_against(query)?;
                Ok(report)
            })();
            self.pending = false;
            self.expected = None;
            match result {
                Ok(report) => {
                    // Show the first known current-pose result immediately.
                    self.selected = match &report {
                        AssemblyInspectionReport::Current(r) => {
                            r.pairs.iter().position(|p| p.result.is_ok())
                        }
                        AssemblyInspectionReport::Range(_) => None,
                    };
                    self.report = Some(report);
                    self.error = None;
                }
                Err(error) => {
                    self.report = None;
                    self.error = Some(error);
                }
            }
        }
    }
    fn show_inputs(&mut self, ui: &mut egui::Ui, session: &Session) {
        self.fields.start();
        let mut changed = false;
        if self.catalog.is_empty() {
            ui.label("Wait for a current native model with physical Part occurrences, then reopen this tool.");
        }
        changed |= path_choice(
            ui,
            "inspection-a",
            "First occurrence",
            &mut self.a,
            &self.catalog,
        );
        changed |= path_choice(
            ui,
            "inspection-b",
            "Second occurrence",
            &mut self.b,
            &self.catalog,
        );
        if ui
            .add_enabled(
                self.a.is_some() && self.b.is_some(),
                egui::Button::new("Add pair"),
            )
            .clicked()
            && let (Some(a), Some(b)) = (&self.a, &self.b)
        {
            let pair = OccurrencePair {
                a: a.clone(),
                b: b.clone(),
            };
            if let Err(error) = pair.validate() {
                self.error = Some(error);
            } else if self
                .pairs
                .iter()
                .any(|p| (p.a == pair.a && p.b == pair.b) || (p.a == pair.b && p.b == pair.a))
            {
                self.error = Some("This pair is already listed".into());
            } else if self.pairs.len() >= MAX_INSPECTION_PAIRS {
                self.error = Some("At most 256 explicit pairs are supported".into());
            } else {
                self.pairs.push(pair);
                changed = true;
            }
        }
        let mut remove = None;
        for (index, pair) in self.pairs.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{} / {}",
                    path_name(session.document(), &pair.a),
                    path_name(session.document(), &pair.b)
                ));
                if icons::mini_button(ui, Icon::Trash, "Remove pair").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.pairs.remove(index);
            changed = true;
        }
        self.fields.dimension(
            ui,
            session.document(),
            "required-clearance",
            "Required minimum clearance",
            &mut self.required,
            "mm",
        );
        ui.small("Inspection requirement only · does not move parts or add a relationship");
        controls::inspector_collapsing(ui, "inspection-tolerance", "Contact tolerance", |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut self.tolerance)
                        .speed(1e-7)
                        .range(1e-7..=1e-3)
                        .suffix(" mm"),
                )
                .on_hover_text(
                    "Contact classification tolerance only. Boolean overlap uses zero added fuzz.",
                )
                .changed();
        });
        if self.range {
            let joints = session
                .document()
                .entities()
                .filter_map(|e| {
                    model::read_joint(session.document(), e.id)
                        .ok()
                        .filter(|(s, _)| s.parameters.kind != model::JointKind::Fixed)
                        .map(|_| (e.id, e.name.clone()))
                })
                .collect::<Vec<_>>();
            if widgets::choose(
                ui,
                "range-joint",
                "Driven relationship",
                &mut self.joint,
                &joints,
                None,
            ) {
                changed = true;
            }
            if let Some(joint) = self.joint
                && let Ok((source, _)) = model::read_joint(session.document(), joint)
            {
                let allowed = [
                    JointCoordinate::X,
                    JointCoordinate::Y,
                    JointCoordinate::Z,
                    JointCoordinate::Rotation,
                ]
                .into_iter()
                .filter(|c| source.parameters.kind.allows(*c))
                .collect::<Vec<_>>();
                let previous = self.coordinate;
                if !allowed.contains(&self.coordinate)
                    && let Some(c) = allowed.first()
                {
                    self.coordinate = *c;
                }
                ui.label("Coordinate");
                controls::inspector_combo(ui, "range-coordinate")
                    .selected_text(coordinate_name(self.coordinate))
                    .show_ui(ui, |ui| {
                        for coordinate in allowed {
                            if ui
                                .selectable_value(
                                    &mut self.coordinate,
                                    coordinate,
                                    coordinate_name(coordinate),
                                )
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    });
                if previous != self.coordinate {
                    let unit = if self.coordinate == JointCoordinate::Rotation {
                        "deg"
                    } else {
                        "mm"
                    };
                    self.start = widgets::literal(0., unit);
                    self.end = widgets::literal(90., unit);
                    self.step = widgets::literal(10., unit);
                    self.fields.reset();
                    changed = true;
                }
            }
            let unit = if self.coordinate == JointCoordinate::Rotation {
                "deg"
            } else {
                "mm"
            };
            for (key, label, value) in [
                ("range-start", "Start", &mut self.start),
                ("range-end", "End (included)", &mut self.end),
                ("range-step", "Explicit step", &mut self.step),
            ] {
                self.fields
                    .dimension(ui, session.document(), key, label, value, unit);
            }
            if let Ok(AssemblyInspectionQuery::Range(query)) = self.query(session)
                && let Ok(values) = query.sample_values()
            {
                ui.small(format!(
                    "{} samples · {} pairs · final interval may be shorter",
                    values.len(),
                    query.pairs.len()
                ));
            }
        }
        changed |= self.fields.changed;
        if changed {
            self.invalidate();
            self.error = None;
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, session: &Session, scene: Option<&Arc<Scene>>) {
        ui.small(if self.range{"Samples only · no continuous collision, dynamics or safety guarantee between poses."}else{"Exact native solid overlap and finite-BRep minimum clearance at the current assembled pose."});
        if self.saved_study {
            let unit = if self.coordinate == JointCoordinate::Rotation {
                Unit::Degree
            } else {
                Unit::Millimeter
            };
            let value =
                |dimension: &Dimension| dimension.value_in(unit, Some(session.parameters()));
            if let [Ok(start), Ok(end), Ok(step)] = [&self.start, &self.end, &self.step].map(value)
            {
                let suffix = if unit == Unit::Degree { "°" } else { " mm" };
                ui.label(format!(
                    "{start}{suffix} to {end}{suffix} · step {step}{suffix}"
                ));
            }
            if let Some(joint) = self.joint.and_then(|id| session.document().get(id)) {
                ui.small(&joint.name);
            }
            for pair in &self.pairs {
                ui.small(format!(
                    "{} / {}",
                    path_name(session.document(), &pair.a),
                    path_name(session.document(), &pair.b)
                ));
            }
        } else {
            self.show_inputs(ui, session);
        }
        if self.backend.is_none() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Native assembly inspection provider unavailable",
            );
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.pending {
            ui.spinner();
            ui.label("Inspecting exact native solids…");
        }
        let required = self.required_mm(session).unwrap_or(0.);
        if let Some(report) = &self.report {
            match report {
                AssemblyInspectionReport::Current(report) => {
                    show_pairs(
                        ui,
                        session.document(),
                        report,
                        Some(&mut self.selected),
                        required,
                    );
                    if self.selected.is_some() {
                        let hash = report.stamp.source_hash.to_string();
                        ui.small(format!(
                            "Evaluated revision {} · source {}",
                            report.stamp.revision,
                            hash.get(..12).unwrap_or(&hash)
                        ));
                        let status = scene.map(|scene| {
                            report
                                .stamp
                                .check(session.document(), session.revision())
                                .and_then(|_| {
                                    ClearanceMarker::build(
                                        report,
                                        self.selected.unwrap_or_default(),
                                        scene,
                                    )
                                })
                        });
                        match status {
                            Some(Ok(marker)) => {
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .button("Fit result")
                                        .on_hover_text(
                                            "Frame both occurrences and the native witnesses · camera only, the model is unchanged",
                                        )
                                        .clicked()
                                    {
                                        self.fit_requested = true;
                                    }
                                    ui.checkbox(&mut self.reveal, "Reveal targets")
                                        .on_hover_text(
                                            "Outline both occurrences over occluding or hidden geometry",
                                        );
                                });
                                for (side, hidden) in marker.hidden().into_iter().enumerate() {
                                    if hidden {
                                        ui.small(format!(
                                            "{} occurrence is hidden in the viewport · outlined from its native placement",
                                            if side == 0 { "First" } else { "Second" }
                                        ));
                                    }
                                }
                            }
                            Some(Err(error)) => {
                                ui.colored_label(
                                    egui::Color32::LIGHT_YELLOW,
                                    format!("Viewport markers hidden: {error}"),
                                );
                            }
                            None => {
                                ui.small("Viewport markers need the current native model view");
                            }
                        }
                    }
                }
                AssemblyInspectionReport::Range(report) => {
                    ui.label(format!(
                        "{} explicit samples · samples only",
                        report.samples.len()
                    ));
                    ui.small("Select a sampled pose to display its actual native geometry. The saved assembly is unchanged.");
                    if let Some(sample) = self
                        .selected_sample
                        .and_then(|index| report.samples.get(index))
                    {
                        ui.label(format!(
                            "Sample preview: {} {} · saved pose unchanged",
                            sample.value,
                            if report.unit == SampleUnit::Radians {
                                "rad"
                            } else {
                                "mm"
                            }
                        ));
                    }
                    if self.sample_pending() {
                        ui.spinner();
                        ui.label("Generating selected sampled pose…");
                    }
                    if self.selected_sample.is_some()
                        && ui.button("Return to current pose").clicked()
                    {
                        self.selected_sample = None;
                        self.selected = None;
                    }
                    let mut next_sample = self.selected_sample;
                    let mut next_pair = self.selected;
                    ui.small(format!(
                        "Actual intervals: {:?} {}",
                        report.actual_intervals,
                        if report.unit == SampleUnit::Radians {
                            "rad"
                        } else {
                            "mm"
                        }
                    ));
                    for (i, sample) in report.samples.iter().enumerate() {
                        controls::inspector_collapsing(
                            ui,
                            format!("range-result/{i}"),
                            &format!(
                                "{} {}",
                                sample.value,
                                if report.unit == SampleUnit::Radians {
                                    "rad"
                                } else {
                                    "mm"
                                }
                            ),
                            |ui| match &sample.result {
                                Ok(report) => {
                                    if ui
                                        .selectable_label(
                                            next_sample == Some(i),
                                            "View sampled pose",
                                        )
                                        .clicked()
                                    {
                                        next_sample = Some(i);
                                        next_pair =
                                            report.pairs.iter().position(|p| p.result.is_ok());
                                    }
                                    let mut selected = if next_sample == Some(i) {
                                        next_pair
                                    } else {
                                        None
                                    };
                                    show_pairs(
                                        ui,
                                        session.document(),
                                        report,
                                        Some(&mut selected),
                                        required,
                                    );
                                    if selected.is_some()
                                        && (next_sample != Some(i) || selected != next_pair)
                                    {
                                        next_sample = Some(i);
                                        next_pair = selected;
                                    }
                                    if next_sample == Some(i) {
                                        next_pair = selected;
                                    }
                                }
                                Err(error) => {
                                    ui.colored_label(
                                        egui::Color32::LIGHT_RED,
                                        format!("Unknown: {error}"),
                                    );
                                }
                            },
                        );
                    }
                    self.selected_sample = next_sample;
                    self.selected = next_pair;
                    if self.display_scene(session).is_some() {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Fit result").clicked() {
                                self.fit_requested = true;
                            }
                            ui.checkbox(&mut self.reveal, "Reveal targets");
                        });
                        ui.small(
                            "Sample preview only · close this tool to restore the current pose",
                        );
                    }
                }
            }
        }
    }
}
fn coordinate_name(coordinate: JointCoordinate) -> &'static str {
    match coordinate {
        JointCoordinate::X => "Local X",
        JointCoordinate::Y => "Local Y",
        JointCoordinate::Z => "Local Z",
        JointCoordinate::Rotation => "About local Z",
    }
}
fn path_choice(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    selected: &mut Option<OccurrencePath>,
    catalog: &[(OccurrencePath, String)],
) -> bool {
    ui.label(label);
    let mut changed = false;
    controls::inspector_combo(ui, id)
        .selected_text(
            selected
                .as_ref()
                .and_then(|s| {
                    catalog
                        .iter()
                        .find(|(p, _)| p == s)
                        .map(|(_, n)| n.as_str())
                })
                .unwrap_or("Choose physical occurrence…"),
        )
        .show_ui(ui, |ui| {
            for (path, name) in catalog {
                if ui
                    .selectable_label(selected.as_ref() == Some(path), name)
                    .clicked()
                {
                    changed |= selected.as_ref() != Some(path);
                    *selected = Some(path.clone());
                    ui.close();
                }
            }
        });
    changed
}
fn path_name(document: &Document, path: &OccurrencePath) -> String {
    std::iter::once(path.root)
        .chain(path.members.iter().copied())
        .map(|id| widgets::name(document, id))
        .collect::<Vec<_>>()
        .join(" / ")
}
fn show_pairs(
    ui: &mut egui::Ui,
    document: &Document,
    report: &PairReport,
    mut selected: Option<&mut Option<usize>>,
    required: f64,
) {
    for (index, pair) in report.pairs.iter().enumerate() {
        ui.push_id(index, |ui| {
            let name = format!(
                "{} / {}",
                path_name(document, &pair.pair.a),
                path_name(document, &pair.pair.b)
            );
            match selected.as_deref_mut() {
                Some(selected) if pair.result.is_ok() => {
                    let active = *selected == Some(index);
                    if ui
                        .selectable_label(active, name)
                        .on_hover_text("Show this pair's witnesses and distance in the viewport")
                        .clicked()
                    {
                        *selected = (!active).then_some(index);
                    }
                }
                _ => {
                    ui.label(name);
                }
            }
            match &pair.result {
                Err(error) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("Unknown: {error}"));
                }
                Ok(values) => {
                    let (label, color) = clearance_view::clearance_status(
                        values.relation,
                        values.minimum_clearance_mm,
                        required,
                    );
                    ui.colored_label(color, label);
                    ui.label(format!(
                        "Required minimum clearance: {}",
                        crate::preferences::format_mm(required)
                    ));
                    ui.small(clearance_view::relation_style(values.relation).0);
                    ui.label(format!(
                        "Measured minimum clearance: {}",
                        crate::preferences::format_mm(values.minimum_clearance_mm)
                    ));
                    ui.label(format!(
                        "Overlap volume: {:.6} mm³",
                        values.overlap_volume_mm3
                    ));
                    let witnesses = match values.total_solutions {
                        Some(total) => format!(
                            "{} / {total} closest-point witnesses",
                            values.witnesses.len()
                        ),
                        None => format!(
                            "{} certified witnesses · total not enumerated",
                            values.witnesses.len()
                        ),
                    };
                    ui.small(format!(
                        "{witnesses} · contact tolerance {} mm",
                        values.tolerance_mm
                    ));
                }
            }
            ui.separator();
        });
    }
}

#[cfg(test)]
#[path = "inspection_native_tests.rs"]
mod native_tests;

#[cfg(test)]
#[path = "clearance_view_tests.rs"]
mod clearance_tests;
