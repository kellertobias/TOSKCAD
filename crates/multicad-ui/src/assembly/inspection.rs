use super::{
    AssemblySelection,
    clearance_view::{self, ClearanceMarker},
    widgets::{self, Fields},
};
use crate::{
    controls, egui,
    icons::{self, Icon},
    viewport::Scene,
};
use multicad_core::{
    assembly_inspection::*,
    commands::Session,
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{self, JointCoordinate, OccurrencePath},
    parameters::Unit,
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
pub(super) struct InspectionPanel {
    range: bool,
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
    fields: Fields,
    backend: Option<Box<dyn AssemblyInspectionBackend>>,
    expected: Option<(u64, AssemblyInspectionQuery)>,
    report: Option<AssemblyInspectionReport>,
    /// Current-pose result shown in the viewport (index into the report).
    selected: Option<usize>,
    reveal: bool,
    fit_requested: bool,
    pub error: Option<String>,
    pub pending: bool,
}
impl InspectionPanel {
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
            fields: Fields::default(),
            backend: factory.map(|f| f()),
            expected: None,
            report: None,
            selected: None,
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
        let Some(AssemblyInspectionReport::Current(report)) = &self.report else {
            return None;
        };
        let index = self.selected?;
        if report.stamp.document != session.document().id()
            || report.stamp.revision != session.revision()
        {
            return Some(Err(
                "The document changed after this result; Preview again".into()
            ));
        }
        Some(ClearanceMarker::build(report, index, scene))
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
    pub fn show(&mut self, ui: &mut egui::Ui, session: &Session, scene: Option<&Arc<Scene>>) {
        self.fields.start();
        let mut changed = false;
        ui.small(if self.range{"Samples only · no continuous collision, dynamics or safety guarantee between poses."}else{"Exact native solid overlap and finite-BRep minimum clearance at the current assembled pose."});
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
        if let Some(report) = &self.report {
            match report {
                AssemblyInspectionReport::Current(report) => {
                    show_pairs(ui, session.document(), report, Some(&mut self.selected));
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
                    ui.small("Sampled poses are not displayed; viewport witnesses show the current pose only.");
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
                                Ok(report) => show_pairs(ui, session.document(), report, None),
                                Err(error) => {
                                    ui.colored_label(
                                        egui::Color32::LIGHT_RED,
                                        format!("Unknown: {error}"),
                                    );
                                }
                            },
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
                    let (label, color) = clearance_view::relation_style(values.relation);
                    ui.colored_label(color, label);
                    ui.label(format!(
                        "Minimum clearance: {}",
                        crate::preferences::format_mm(values.minimum_clearance_mm)
                    ));
                    ui.label(format!(
                        "Overlap volume: {:.6} mm³",
                        values.overlap_volume_mm3
                    ));
                    ui.small(format!(
                        "{} / {} native witnesses · contact tolerance {} mm",
                        values.witnesses.len(),
                        values.total_solutions,
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
