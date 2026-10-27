//! Constraint drafts never mutate the document. The checked solver transaction
//! supplies the only Apply-capable preview; invalid drafts remain repairable.
use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{self as model, ConstraintKind as Kind, Dimension, source::Source},
    document::{Document, EntityId},
    references::ReferenceReport,
};
use multicad_solver::{SketchEdit, SolveState, prepare_edit};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Fix,
    Coincident,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Equal,
    Symmetric,
    Distance,
    Length,
    Radius,
    Diameter,
    Angle,
}
impl Tool {
    const ALL: [Self; 14] = [
        Self::Fix,
        Self::Coincident,
        Self::Horizontal,
        Self::Vertical,
        Self::Parallel,
        Self::Perpendicular,
        Self::Tangent,
        Self::Equal,
        Self::Symmetric,
        Self::Distance,
        Self::Length,
        Self::Radius,
        Self::Diameter,
        Self::Angle,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Fix => "Fix point",
            Self::Coincident => "Coincident",
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Tangent => "Tangent",
            Self::Equal => "Equal",
            Self::Symmetric => "Symmetry",
            Self::Distance => "Distance",
            Self::Length => "Length",
            Self::Radius => "Radius",
            Self::Diameter => "Diameter",
            Self::Angle => "Angle",
        }
    }
    fn kind(self, selection: &[EntityId], value: f64, unit: &str) -> Result<Kind, String> {
        let expected = match self {
            Self::Fix
            | Self::Horizontal
            | Self::Vertical
            | Self::Length
            | Self::Radius
            | Self::Diameter => 1,
            Self::Symmetric => 3,
            _ => 2,
        };
        if selection.len() != expected {
            return Err(format!(
                "{} needs {expected} selected entities (in order)",
                self.label()
            ));
        }
        let a = selection[0];
        let b = selection.get(1).copied().unwrap_or(a);
        let dimension = Dimension {
            value,
            unit: unit.into(),
        };
        Ok(match self {
            Self::Fix => Kind::Fix { point: a },
            Self::Coincident => Kind::Coincident { a, b },
            Self::Horizontal => Kind::Horizontal { line: a },
            Self::Vertical => Kind::Vertical { line: a },
            Self::Parallel => Kind::Parallel { a, b },
            Self::Perpendicular => Kind::Perpendicular { a, b },
            Self::Tangent => Kind::Tangent { a, b },
            Self::Equal => Kind::Equal { a, b },
            Self::Symmetric => Kind::Symmetric {
                a,
                b,
                about: selection[2],
            },
            Self::Distance => Kind::Distance {
                a,
                b,
                axis: model::Axis::Direct,
                dimension,
            },
            Self::Length => Kind::Length { line: a, dimension },
            Self::Radius => Kind::Radius {
                curve: a,
                dimension,
            },
            Self::Diameter => Kind::Diameter {
                curve: a,
                dimension,
            },
            Self::Angle => Kind::Angle {
                from: a,
                to: b,
                dimension,
            },
        })
    }
}
fn label(kind: &Kind) -> &'static str {
    match kind {
        Kind::Fix { .. } => "Fixed",
        Kind::Coincident { .. } => "Coincident",
        Kind::Horizontal { .. } => "Horizontal",
        Kind::Vertical { .. } => "Vertical",
        Kind::Parallel { .. } => "Parallel",
        Kind::Perpendicular { .. } => "Perpendicular",
        Kind::Tangent { .. } => "Tangent",
        Kind::Equal { .. } => "Equal",
        Kind::Symmetric { .. } => "Symmetry",
        Kind::Distance { .. } => "Distance",
        Kind::Length { .. } => "Length",
        Kind::Radius { .. } => "Radius",
        Kind::Diameter { .. } => "Diameter",
        Kind::Angle { .. } => "Angle",
    }
}

pub struct ConstraintEditor {
    owner: Option<EntityId>,
    revision: u64,
    draft: Option<Source>,
    result: Option<SketchEdit>,
    error: Option<String>,
    selection: Vec<EntityId>,
    last_element: Option<EntityId>,
    tool: Tool,
    value: f64,
    unit: String,
    evaluation: Option<ReferenceReport>,
}
impl Default for ConstraintEditor {
    fn default() -> Self {
        Self {
            owner: None,
            revision: 0,
            draft: None,
            result: None,
            error: None,
            selection: vec![],
            last_element: None,
            tool: Tool::Length,
            value: 10.,
            unit: "mm".into(),
            evaluation: None,
        }
    }
}
impl ConstraintEditor {
    pub fn set_evaluation(&mut self, evaluation: Option<ReferenceReport>) {
        if self.evaluation != evaluation {
            self.result = None;
            self.error = None;
        }
        self.evaluation = evaluation;
    }
    fn read_source(
        &self,
        session: &Session,
        id: EntityId,
    ) -> Result<
        (Source, multicad_core::sketch::Frame, EntityId),
        multicad_core::document::DocumentError,
    > {
        if let Some(report) = &self.evaluation {
            Source::read_evaluated(session.document(), id, session.revision(), report)
        } else {
            Source::read(session.document(), id)
        }
    }
    fn validate_frame(&self, session: &Session, id: EntityId) -> Result<(), String> {
        // The display facade also accepts a genuine legacy source during its
        // explicit upgrade. It never supplies a saved-datum attachment fallback.
        if let Some(report) = &self.evaluation {
            multicad_core::sketch::Sketch::read_evaluated(
                session.document(),
                id,
                session.revision(),
                report,
            )
        } else {
            multicad_core::sketch::Sketch::read(session.document(), id)
        }
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
    pub fn begin_dimension(
        &mut self,
        session: &Session,
        id: EntityId,
        element: EntityId,
    ) -> Result<(), String> {
        if self.dirty() {
            return Err("Apply or discard the current constraint draft first".into());
        }
        let (source, _, _) = self.read_source(session, id).map_err(|e| e.to_string())?;
        let curve = source
            .sketch
            .curves
            .iter()
            .find(|c| c.id == element)
            .ok_or("Select a curve to dimension")?;
        self.tool = match curve.kind {
            model::CurveKind::Line { .. } => Tool::Length,
            _ => Tool::Radius,
        };
        self.value = source
            .display_geometry()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|g| g.id() == element)
            .map(|g| match g {
                multicad_core::sketch::Geometry::Line { a, b, .. } => {
                    (b[0] - a[0]).hypot(b[1] - a[1])
                }
                multicad_core::sketch::Geometry::Circle { radius_mm, .. } => *radius_mm,
                multicad_core::sketch::Geometry::Arc { center, a, .. } => {
                    (a[0] - center[0]).hypot(a[1] - center[1])
                }
                _ => 0.,
            })
            .ok_or("Selected curve is unavailable")?;
        self.unit = "mm".into();
        self.selection = vec![element];
        self.last_element = Some(element);
        Ok(())
    }
    pub fn dimension_kind(&self, value: f64, unit: &str) -> Result<Kind, String> {
        self.tool.kind(&self.selection, value, unit)
    }
    pub fn dirty(&self) -> bool {
        self.draft.is_some()
    }
    pub fn discard(&mut self) {
        self.draft = None;
        self.result = None;
        self.error = None;
        self.owner = None;
        self.selection.clear();
        self.last_element = None;
    }
    pub fn preview_document(&self) -> Option<&Document> {
        if self.dirty() {
            self.result.as_ref()?.preview.as_ref().map(|p| p.document())
        } else {
            None
        }
    }
    pub fn prepare(&mut self, session: &Session, id: EntityId, source: Source) {
        if self.owner != Some(id) || !self.dirty() {
            self.revision = session.revision();
        }
        self.owner = Some(id);
        self.draft = Some(source.clone());
        self.error = None;
        if let Err(error) = self.validate_frame(session, id) {
            self.result = None;
            self.error = Some(error);
            return;
        }
        match prepare_edit(
            session,
            self.revision,
            Actor::local(Channel::Ui),
            id,
            source,
        ) {
            Ok(result) => self.result = Some(result),
            Err(e) => {
                self.result = None;
                self.error = Some(e.to_string());
            }
        }
    }
    pub fn apply(&mut self, session: &mut Session) -> Result<(), String> {
        self.validate_frame(
            session,
            self.owner
                .ok_or("Choose a sketch before applying constraints")?,
        )?;
        let preview = self
            .result
            .as_mut()
            .and_then(|r| r.preview.take())
            .ok_or("Resolve the draft before applying")?;
        session.commit(preview).map_err(|e| e.to_string())?;
        self.discard();
        Ok(())
    }
    pub fn affected(&self) -> BTreeSet<EntityId> {
        let Some(source) = &self.draft else {
            return BTreeSet::new();
        };
        let Some(result) = &self.result else {
            return BTreeSet::new();
        };
        source
            .sketch
            .constraints
            .iter()
            .filter(|c| {
                result.evaluation.diagnostics.conflicts.contains(&c.id)
                    || result.evaluation.diagnostics.unmet.contains(&c.id)
            })
            .flat_map(|c| c.kind.references())
            .collect()
    }
    pub fn selected(&self, id: EntityId) -> bool {
        self.selection.contains(&id)
    }
    pub fn free(&self, id: EntityId) -> bool {
        self.result
            .as_ref()
            .is_some_and(|r| r.evaluation.diagnostics.free_entities.contains(&id))
    }
    fn toggle(&mut self, id: EntityId) {
        if let Some(i) = self.selection.iter().position(|s| *s == id) {
            self.selection.remove(i);
        } else {
            self.selection.push(id);
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        id: EntityId,
        element: Option<EntityId>,
    ) {
        ui.separator();
        ui.strong("Constraints");
        if self.owner != Some(id) && self.dirty() {
            ui.colored_label(
                Color32::LIGHT_RED,
                "Apply or discard the other sketch draft first.",
            );
            if ui.button("Discard constraint draft").clicked() {
                self.discard();
            }
            return;
        }
        let read = self.read_source(session, id);
        if read.is_err() && !self.dirty() {
            let legacy=session.document().get(id).is_some_and(|e|matches!(&e.data,multicad_core::document::EntityData::Sketch {definition,..} if definition.type_id==multicad_core::sketch::TYPE&&definition.schema_version==1));
            if !legacy {
                if let Err(error) = &read {
                    ui.colored_label(Color32::LIGHT_RED, error.to_string());
                }
                return;
            }
            ui.small("This snapshot uses the earlier sketch source.");
            if ui.button("Enable constraints").on_hover_text("Convert rectangles to four constrained lines. Apply is undoable; Discard keeps the original source.").clicked() {
                match Source::from_legacy(session.document(),id) {Ok(source)=>self.prepare(session,id,source),Err(e)=>self.error=Some(e.to_string())}
            }
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            return;
        }
        if let Err(error) = &read {
            ui.colored_label(Color32::LIGHT_RED, error.to_string());
            if ui.button("Discard constraint draft").clicked() {
                self.discard();
            }
            return;
        }
        if self.owner != Some(id) || (!self.dirty() && self.revision != session.revision()) {
            self.owner = Some(id);
            self.revision = session.revision();
            self.result = None;
            self.error = None;
            self.selection.clear();
            self.last_element = None;
        }
        let mut source = if let Some(draft) = &self.draft {
            draft.clone()
        } else if let Ok((source, _, _)) = read {
            source
        } else {
            ui.colored_label(Color32::LIGHT_RED, "Sketch source is unavailable");
            return;
        };
        if self.last_element != element {
            self.last_element = element;
            self.selection = element.into_iter().collect();
        }
        if self.result.is_none() && self.error.is_none() {
            match prepare_edit(
                session,
                session.revision(),
                Actor::local(Channel::Ui),
                id,
                source.clone(),
            ) {
                Ok(r) => self.result = Some(r),
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if let Some(result) = &self.result {
            let d = &result.evaluation.diagnostics;
            let text = match d.state {
                SolveState::FullyConstrained => "Fully constrained".into(),
                SolveState::Underconstrained => {
                    format!("{} degrees of freedom", d.degrees_of_freedom)
                }
                SolveState::Conflict => format!("Conflicting constraints ({})", d.conflicts.len()),
                SolveState::Flipped => "Solution changed branch".into(),
                SolveState::NotConverged => "Solver did not converge".into(),
            };
            ui.colored_label(
                if result.evaluation.candidate.is_some() {
                    Color32::from_rgb(115, 205, 190)
                } else {
                    Color32::LIGHT_RED
                },
                text,
            );
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if self.dirty() && self.revision != session.revision() {
            ui.colored_label(
                Color32::LIGHT_RED,
                "Document changed. Discard this stale draft and retry.",
            );
        }
        ui.collapsing("Select geometry (in order)", |ui| {
            if ui.small_button("Clear selection").clicked() {
                self.selection.clear();
            }
            for (i, p) in source.sketch.points.iter().enumerate() {
                if ui
                    .selectable_label(
                        self.selected(p.id),
                        format!(
                            "P{} · {:.2}, {:.2}",
                            i + 1,
                            p.position_mm[0],
                            p.position_mm[1]
                        ),
                    )
                    .clicked()
                {
                    self.toggle(p.id);
                }
            }
            for (i, c) in source.sketch.curves.iter().enumerate() {
                let name = match c.kind {
                    model::CurveKind::Line { .. } => "Line",
                    model::CurveKind::Circle { .. } => "Circle",
                    model::CurveKind::Arc { .. } => "Arc",
                };
                if ui
                    .selectable_label(self.selected(c.id), format!("{name} {}", i + 1))
                    .clicked()
                {
                    self.toggle(c.id);
                }
            }
        });
        let mut changed = false;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("constraint-tool")
                .selected_text(self.tool.label())
                .width(125.)
                .show_ui(ui, |ui| {
                    for tool in Tool::ALL {
                        if ui
                            .selectable_value(&mut self.tool, tool, tool.label())
                            .clicked()
                        {
                            ui.close();
                        }
                    }
                });
            if ui
                .button("Add")
                .on_hover_text("Use the selected geometry in selection order")
                .clicked()
            {
                match self.tool.kind(&self.selection, self.value, &self.unit) {
                    Ok(kind) => {
                        source.add_constraint(kind);
                        changed = true;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.value).speed(0.1));
            ui.add(
                egui::TextEdit::singleline(&mut self.unit)
                    .desired_width(40.)
                    .hint_text("mm"),
            );
        });
        let mut remove = None;
        for (i, c) in source.sketch.constraints.iter_mut().enumerate() {
            let conflict = self
                .result
                .as_ref()
                .is_some_and(|r| r.evaluation.diagnostics.conflicts.contains(&c.id));
            ui.push_id(c.id.to_string(), |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        if conflict {
                            Color32::LIGHT_RED
                        } else {
                            ui.visuals().text_color()
                        },
                        format!("{} {}", i + 1, label(&c.kind)),
                    );
                    if crate::icons::compact_button(
                        ui,
                        crate::icons::Icon::Trash,
                        "Remove constraint",
                    )
                    .clicked()
                    {
                        remove = Some(c.id);
                    }
                });
                if let Some(d) = c.kind.dimension_mut() {
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(egui::DragValue::new(&mut d.value).speed(0.1))
                            .changed();
                        changed |= ui
                            .add(egui::TextEdit::singleline(&mut d.unit).desired_width(35.))
                            .changed();
                        changed |= ui.checkbox(&mut c.reference, "Reference").changed();
                    });
                    if let Some(measurement) = self
                        .result
                        .as_ref()
                        .and_then(|r| r.evaluation.measurements.get(&c.id))
                    {
                        ui.small(format!(
                            "Measured {:.4} {}",
                            measurement.value, measurement.unit
                        ));
                    }
                }
            });
        }
        if let Some(id) = remove {
            source.sketch.constraints.retain(|c| c.id != id);
            changed = true;
        }
        if changed {
            self.prepare(session, id, source);
        }
        if self.dirty() {
            ui.horizontal(|ui| {
                let ready = self.revision == session.revision()
                    && self.result.as_ref().is_some_and(|r| r.preview.is_some());
                if ui
                    .add_enabled(ready, egui::Button::new("Apply constraints"))
                    .clicked()
                    && let Err(e) = self.apply(session)
                {
                    self.error = Some(e);
                }
                if ui.button("Discard").clicked() {
                    self.discard();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{commands::Request, document::Document, sketch::Datum};
    fn setup() -> (Session, EntityId, Source, EntityId) {
        let mut session = Session::new(Document::new("Draft tests").unwrap()).unwrap();
        let (id, edits) = Source::create(Datum::Xy, "Sketch").unwrap();
        session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Ui),
                label: "New".into(),
                edits,
            })
            .unwrap();
        let (mut source, _, _) = Source::read(session.document(), id).unwrap();
        let edge = source.add_rectangle([0., 0.], [80., 60.]).unwrap();
        let edit = prepare_edit(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            id,
            source.clone(),
        )
        .unwrap();
        session.commit(edit.preview.unwrap()).unwrap();
        (session, id, source, edge)
    }
    fn attached_circle() -> (Session, EntityId, EntityId, EntityId, ReferenceReport) {
        use multicad_core::{document::*, references::*};
        let panel = Entity::new(
            "Panel",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.panel".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let solid =
            multicad_kernel::Solid::box_named([100., 60., 18.], panel.id, Default::default())
                .unwrap();
        let snapshot = solid.topology_snapshot(panel.id, 0).unwrap();
        let origin = Origin {
            source: panel.id,
            role: "cap.end".into(),
        };
        let face = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
            .unwrap();
        let mut intent = GeometryIntent::capture(&snapshot, face.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: origin,
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let mut link = Link::new(panel.id, "face");
        link.geometry = Some(intent);
        let plane = Entity::new(
            "Top",
            EntityData::Plane {
                origin_mm: [0.; 3],
                normal: [0., 0., 1.],
                anchor: Some(link),
            },
        );
        let mut source = Source {
            u: [1., 0., 0.],
            sketch: Default::default(),
        };
        let center = source.add_point([20., 25.]);
        let curve = EntityId::new();
        source.sketch.curves.push(model::Curve {
            id: curve,
            construction: false,
            kind: model::CurveKind::Circle {
                center,
                radius_mm: 3.,
            },
        });
        let sketch = Entity::new(
            "Attached circle",
            EntityData::Sketch {
                plane: plane.id,
                definition: source.definition().unwrap(),
                inputs: vec![],
            },
        );
        let mut document = Document::new("Attached dimension").unwrap();
        for entity in [panel, plane.clone(), sketch.clone()] {
            document.insert(entity).unwrap();
        }
        let report = inspect(
            &document,
            0,
            &std::collections::BTreeMap::from([(snapshot.owner, snapshot)]),
        );
        (
            Session::new(document).unwrap(),
            sketch.id,
            plane.id,
            curve,
            report,
        )
    }
    #[test]
    fn attached_circle_dimension_previews_and_commits_without_changing_its_anchor() {
        let (mut session, id, plane, curve, report) = attached_circle();
        let before = session.document().clone();
        assert!(Source::read(session.document(), id).is_err());
        let mut editor = ConstraintEditor::default();
        editor.set_evaluation(Some(report.clone()));
        editor.begin_dimension(&session, id, curve).unwrap();
        let kind = editor.dimension_kind(5., "mm").unwrap();
        assert!(matches!(kind,Kind::Radius {curve:c,..} if c==curve));
        let (mut source, frame, _) =
            Source::read_evaluated(session.document(), id, 0, &report).unwrap();
        assert_eq!(frame.origin, [0., 0., 18.]);
        source.add_constraint(kind);
        editor.prepare(&session, id, source);
        assert!(editor.preview_document().is_some(), "{:?}", editor.error);
        assert_eq!(session.document(), &before);
        assert_eq!(
            editor.preview_document().unwrap().get(plane),
            before.get(plane)
        );
        editor.apply(&mut session).unwrap();
        assert_eq!(session.revision(), 1);
        assert_eq!(session.document().get(plane), before.get(plane));
        let panel = report.entries[0].source;
        let snapshot =
            multicad_kernel::Solid::box_named([100., 60., 18.], panel, Default::default())
                .unwrap()
                .topology_snapshot(panel, 1)
                .unwrap();
        let current = multicad_core::references::inspect(
            session.document(),
            1,
            &std::collections::BTreeMap::from([(panel, snapshot)]),
        );
        let (source, frame, _) =
            Source::read_evaluated(session.document(), id, 1, &current).unwrap();
        assert_eq!(frame.origin, [0., 0., 18.]);
        let model::CurveKind::Circle { radius_mm, .. } = source.sketch.curves[0].kind else {
            panic!("circle")
        };
        assert!((radius_mm - 5.).abs() < 1e-6);
        session.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn absent_stale_or_unresolved_attachment_blocks_dimension_prepare_and_apply() {
        let (mut session, id, _, curve, valid) = attached_circle();
        let before = session.document().clone();
        let (mut source, _, _) = Source::read_evaluated(session.document(), id, 0, &valid).unwrap();
        source.add_constraint(Kind::Radius {
            curve,
            dimension: Dimension {
                value: 5.,
                unit: "mm".into(),
            },
        });
        let mut stale = valid.clone();
        stale.revision = 1;
        let unresolved =
            multicad_core::references::inspect(session.document(), 0, &Default::default());
        for evaluation in [None, Some(stale), Some(unresolved.clone())] {
            let mut editor = ConstraintEditor::default();
            editor.set_evaluation(evaluation);
            assert!(editor.begin_dimension(&session, id, curve).is_err());
            editor.prepare(&session, id, source.clone());
            assert!(editor.preview_document().is_none());
            assert!(editor.apply(&mut session).is_err());
            assert_eq!(session.document(), &before);
        }
        let mut editor = ConstraintEditor::default();
        editor.set_evaluation(Some(valid));
        editor.prepare(&session, id, source);
        assert!(editor.preview_document().is_some());
        editor.set_evaluation(Some(unresolved));
        assert!(editor.preview_document().is_none());
        assert!(editor.apply(&mut session).is_err());
        assert!(editor.dirty());
        assert_eq!(session.document(), &before);
        assert!(!session.can_undo());
    }
    #[test]
    fn conflict_draft_highlights_geometry_and_reference_toggle_recovers_atomically() {
        let (mut session, id, mut source, edge) = setup();
        let before = session.document().clone();
        for value in [80., 90.] {
            source.add_constraint(Kind::Length {
                line: edge,
                dimension: Dimension {
                    value,
                    unit: "mm".into(),
                },
            });
        }
        let mut editor = ConstraintEditor::default();
        editor.prepare(&session, id, source.clone());
        assert!(editor.dirty());
        assert!(editor.affected().contains(&edge));
        assert!(editor.preview_document().is_none());
        assert!(editor.apply(&mut session).is_err());
        assert_eq!(session.document(), &before);
        source.sketch.constraints.last_mut().unwrap().reference = true;
        editor.prepare(&session, id, source);
        assert!(editor.preview_document().is_some());
        assert_eq!(session.document(), &before);
        let revision = session.revision();
        editor.apply(&mut session).unwrap();
        assert_eq!(session.revision(), revision + 1);
        assert!(!editor.dirty());
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn discard_and_stale_apply_preserve_committed_document() {
        let (mut session, id, mut source, edge) = setup();
        let before = session.document().clone();
        source.add_constraint(Kind::Length {
            line: edge,
            dimension: Dimension {
                value: 100.,
                unit: "mm".into(),
            },
        });
        let mut editor = ConstraintEditor::default();
        editor.prepare(&session, id, source.clone());
        assert!(editor.dirty());
        editor.discard();
        assert_eq!(session.document(), &before);
        editor.prepare(&session, id, source);
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        let after = session.document().clone();
        assert!(editor.apply(&mut session).is_err());
        assert!(editor.dirty());
        assert_eq!(session.document(), &after);
        editor.discard();
        assert!(!editor.dirty());
    }
    #[test]
    fn explicit_legacy_upgrade_is_previewed_and_undo_restores_original_provider() {
        let mut session = Session::new(Document::new("Legacy").unwrap()).unwrap();
        let (id, edits) = multicad_core::sketch::create(Datum::Yz, "Old").unwrap();
        session
            .apply(Request {
                expected_revision: 0,
                actor: Actor::local(Channel::Ui),
                label: "Old".into(),
                edits,
            })
            .unwrap();
        let gid = EntityId::new();
        let edits = multicad_core::sketch::append(
            session.document(),
            id,
            multicad_core::sketch::Geometry::Rectangle {
                id: gid,
                a: [0., 0.],
                b: [80., 60.],
            },
        )
        .unwrap();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Rectangle".into(),
                edits,
            })
            .unwrap();
        let before = session.document().clone();
        let source = Source::from_legacy(session.document(), id).unwrap();
        assert_eq!(source.sketch.curves[0].id, gid);
        assert_eq!(source.sketch.curves.len(), 4);
        let mut editor = ConstraintEditor::default();
        editor.prepare(&session, id, source);
        assert_eq!(session.document(), &before);
        editor.apply(&mut session).unwrap();
        assert!(Source::read(session.document(), id).is_ok());
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
    }
}
