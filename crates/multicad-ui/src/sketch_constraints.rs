//! Constraint drafts never mutate the document. The checked solver transaction
//! supplies the only Apply-capable preview; invalid drafts remain repairable.
use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{self as model, ConstraintKind as Kind, Dimension, source::Source},
    document::{Document, EntityData, EntityId, Mutation},
    references::ReferenceReport,
};
#[cfg(test)]
use multicad_solver::prepare_edit;
use multicad_solver::{EvaluatedSketch, SketchEdit, SolveState, prepare_edit_with_report};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
struct DimensionFields {
    text: String,
    unit: String,
}
impl DimensionFields {
    fn from_dimension(dimension: &Dimension) -> Self {
        Self {
            text: match dimension {
                Dimension::Literal { value, .. } => value.to_string(),
                Dimension::Expression { expression } => expression.text.clone(),
            },
            unit: dimension.unit().into(),
        }
    }
    fn input(&self) -> Result<model::binding::DimensionInput, String> {
        let text = self.text.trim();
        if text.starts_with('=') {
            Ok(model::binding::DimensionInput::Expression {
                text: text.into(),
                unit: self.unit.clone(),
            })
        } else {
            let value: f64 = text
                .parse()
                .map_err(|_| "Enter a number or an expression beginning with =")?;
            if !value.is_finite() {
                return Err("Dimension must be finite".into());
            }
            Ok(model::binding::DimensionInput::Literal {
                value,
                unit: self.unit.clone(),
            })
        }
    }
}

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
        let dimension = Dimension::Literal {
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
fn solution_matches_source(source: &Source, solved: &model::Sketch) -> bool {
    source.sketch.points.len() == solved.points.len()
        && source.sketch.curves.len() == solved.curves.len()
        && source.sketch.constraints == solved.constraints
        && source
            .sketch
            .points
            .iter()
            .zip(&solved.points)
            .all(|(a, b)| a.id == b.id)
        && source
            .sketch
            .curves
            .iter()
            .zip(&solved.curves)
            .all(|(a, b)| {
                a.id == b.id
                    && a.construction == b.construction
                    && match (&a.kind, &b.kind) {
                        (
                            model::CurveKind::Circle { center: a, .. },
                            model::CurveKind::Circle { center: b, .. },
                        ) => a == b,
                        (a, b) => a == b,
                    }
            })
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
    current: Option<Arc<EvaluatedSketch>>,
    dimension_fields: BTreeMap<EntityId, DimensionFields>,
    preview_geometry: Option<Vec<multicad_core::sketch::Geometry>>,
    apply_label: String,
    canonical_draft: bool,
    point_display: Option<PointDisplay>,
}
struct PointDisplay {
    scope: (EntityId, u64, EntityId),
    positions: Result<Arc<BTreeMap<EntityId, [f64; 2]>>, String>,
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
            current: None,
            dimension_fields: BTreeMap::new(),
            preview_geometry: None,
            apply_label: "Apply constraints".into(),
            canonical_draft: false,
            point_display: None,
        }
    }
}
impl ConstraintEditor {
    pub fn set_current(&mut self, current: Option<Arc<EvaluatedSketch>>) {
        let unchanged = match (&self.current, &current) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if !unchanged {
            self.point_display = None;
        }
        self.current = current;
    }
    pub fn set_evaluation(&mut self, evaluation: Option<ReferenceReport>) {
        if self.evaluation != evaluation {
            self.result = None;
            self.error = None;
            self.current = None;
            self.preview_geometry = None;
            self.point_display = None;
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
        if let Some(current) = self.current.as_ref().filter(|current| {
            current.sketch == id
                && current.stamp.document == session.document().id()
                && current.stamp.revision == session.revision()
        }) {
            return Ok((current.canonical().clone(), current.frame, current.plane));
        }
        Source::read_for_evaluation(
            session.document(),
            id,
            session.revision(),
            self.evaluation.as_ref(),
        )
    }
    fn validate_frame(&self, session: &Session, id: EntityId) -> Result<(), String> {
        // The display facade also accepts a genuine legacy source during its
        // explicit upgrade. It never supplies a saved-datum attachment fallback.
        if session.document().get(id).is_some_and(|e|matches!(&e.data,multicad_core::document::EntityData::Sketch{definition,..} if definition.type_id==model::source::TYPE)) {
            return self.read_source(session,id).map(|_|()).map_err(|e|e.to_string());
        }
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
        self.choose_dimension(session, id, element)?;
        self.value = self
            .current
            .as_ref()
            .filter(|e| {
                e.sketch == id
                    && e.stamp.document == session.document().id()
                    && e.stamp.revision == session.revision()
            })
            .ok_or("Wait for the current sketch solution before dimensioning")?
            .geometry()
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
        Ok(())
    }
    /// Exact command entry supplies its own value and needs no measured guess.
    pub fn begin_dimension_exact(
        &mut self,
        session: &Session,
        id: EntityId,
        element: EntityId,
        value: f64,
        unit: &str,
    ) -> Result<(), String> {
        self.choose_dimension(session, id, element)?;
        self.value = value;
        self.unit = unit.into();
        Ok(())
    }
    fn choose_dimension(
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
        self.tool =
            match curve.kind {
                model::CurveKind::Line { .. } => Tool::Length,
                model::CurveKind::BSpline { .. } => return Err(
                    "Select spline control poles to add point distances or geometric constraints"
                        .into(),
                ),
                _ => Tool::Radius,
            };
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
        self.preview_geometry = None;
        self.error = None;
        self.owner = None;
        self.selection.clear();
        self.last_element = None;
        self.dimension_fields.clear();
        self.apply_label = "Apply constraints".into();
        self.canonical_draft = false;
        self.point_display = None;
    }
    pub fn preview_document(&self) -> Option<&Document> {
        if self.dirty() {
            self.result.as_ref()?.preview.as_ref().map(|p| p.document())
        } else {
            None
        }
    }
    pub fn preview_sketch(&self) -> Option<EntityId> {
        self.preview_document().and(self.owner)
    }
    pub fn semantic_preview(&self) -> Option<&EvaluatedSketch> {
        self.result
            .as_ref()
            .filter(|_| self.dirty())?
            .semantic
            .as_ref()
    }
    pub fn semantic_preview_geometry(&self) -> Option<&[multicad_core::sketch::Geometry]> {
        self.preview_geometry.as_deref()
    }
    pub fn prepare(&mut self, session: &Session, id: EntityId, source: Source) {
        self.prepare_source(session, id, source, false);
    }
    fn prepare_source(&mut self, session: &Session, id: EntityId, source: Source, canonical: bool) {
        let canonical = canonical
            || source.minimum_schema_version() >= 2
            || (self.owner == Some(id) && self.dirty() && self.canonical_draft);
        if self.owner != Some(id) || !self.dirty() {
            self.revision = session.revision();
        }
        self.owner = Some(id);
        self.draft = Some(source.clone());
        self.error = None;
        self.preview_geometry = None;
        self.apply_label = "Apply constraints".into();
        self.canonical_draft = canonical;
        self.point_display = None;
        if let Err(error) = self.validate_frame(session, id) {
            self.result = None;
            self.error = Some(error);
            return;
        }
        let prepared = if canonical {
            multicad_solver::prepare_canonical_edit(
                session,
                self.revision,
                Actor::local(Channel::Ui),
                id,
                source,
                self.evaluation.as_ref(),
            )
        } else {
            prepare_edit_with_report(
                session,
                self.revision,
                Actor::local(Channel::Ui),
                id,
                source,
                self.evaluation.as_ref(),
            )
        };
        match prepared {
            Ok(result) => {
                self.canonical_draft |= result.semantic.is_some();
                self.preview_geometry = result.semantic.as_ref().and_then(|e| e.geometry().ok());
                self.result = Some(result);
            }
            Err(e) => {
                self.result = None;
                self.error = Some(e.to_string());
            }
        }
    }
    fn visible_solution(
        &self,
        session: &Session,
        id: EntityId,
        verify_hash: bool,
    ) -> Result<&model::Sketch, String> {
        let solved = if self.dirty() {
            if self.owner != Some(id) || self.revision != session.revision() {
                return Err("Discard the stale or other sketch draft before fixing a point".into());
            }
            let result = self
                .result
                .as_ref()
                .ok_or("Resolve the current draft before fixing a point")?;
            let preview = result
                .preview
                .as_ref()
                .ok_or("Resolve the current draft before fixing a point")?;
            session
                .validate_preview(preview)
                .map_err(|e| e.to_string())?;
            if let Some(semantic) = &result.semantic
                && (semantic.sketch != id
                    || semantic.stamp.document != preview.document().id()
                    || semantic.stamp.revision != session.revision()
                    || (verify_hash
                        && !semantic.is_current(preview.document(), session.revision()))
                    || semantic.evaluation != result.evaluation)
            {
                return Err("The current draft solution is stale or mismatched".into());
            }
            result
                .evaluation
                .candidate
                .as_ref()
                .ok_or("Resolve the current draft before fixing a point")?
        } else {
            let current = self
                .current
                .as_ref()
                .filter(|e| {
                    e.sketch == id
                        && e.stamp.document == session.document().id()
                        && e.stamp.revision == session.revision()
                        && (!verify_hash || e.is_current(session.document(), session.revision()))
                })
                .ok_or("Wait for the current sketch solution before fixing a point")?;
            current
                .solved()
                .ok_or("The sketch has no current valid solution")?
        };
        Ok(solved)
    }
    fn visible_point(
        &self,
        session: &Session,
        id: EntityId,
        point: EntityId,
    ) -> Result<[f64; 2], String> {
        let solved = self.visible_solution(session, id, true)?;
        let position = solved
            .points
            .iter()
            .find(|p| p.id == point)
            .ok_or("Fix requires a current sketch point")?
            .position_mm;
        if position.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
            return Err("The current point position is invalid".into());
        }
        Ok(position)
    }
    fn display_positions(
        &mut self,
        session: &Session,
        id: EntityId,
    ) -> Option<Arc<BTreeMap<EntityId, [f64; 2]>>> {
        let scope = (session.document().id(), session.revision(), id);
        if self
            .point_display
            .as_ref()
            .is_none_or(|cached| cached.scope != scope)
        {
            // Worker publications and Prepared previews are immutable. Paint
            // checks their cheap scope/revision guards and reuses this map.
            // Explicit Fix/adopt boundaries additionally verify source hashes.
            let positions = self
                .visible_solution(session, id, false)
                .map(|s| Arc::new(s.points.iter().map(|p| (p.id, p.position_mm)).collect()));
            self.point_display = Some(PointDisplay { scope, positions });
        }
        self.point_display
            .as_ref()?
            .positions
            .as_ref()
            .ok()
            .cloned()
    }
    fn add_kind(
        &self,
        session: &Session,
        id: EntityId,
        source: &mut Source,
        kind: Kind,
    ) -> Result<(), String> {
        if let Kind::Fix { point } = kind {
            let position = self.visible_point(session, id, point)?;
            source
                .sketch
                .points
                .iter_mut()
                .find(|p| p.id == point)
                .ok_or("Fix requires a sketch point")?
                .position_mm = position;
        }
        source.add_constraint(kind);
        Ok(())
    }
    /// Adopt an already checked canonical operation without running the sketch
    /// solver again. A diagnostic-only draft remains repairable without Apply.
    pub(crate) fn accept_edit(
        &mut self,
        session: &Session,
        id: EntityId,
        edit: SketchEdit,
        label: &str,
    ) -> Result<(), String> {
        if self.dirty() {
            return Err(
                "Apply or discard the current draft before previewing another operation".into(),
            );
        }
        let label = label.trim();
        if label.is_empty() || label.len() > 128 {
            return Err("The operation needs a short Apply label".into());
        }
        let semantic = edit
            .semantic
            .as_ref()
            .ok_or("Canonical operations require a semantic source preview")?;
        if semantic.sketch != id
            || semantic.stamp.document != session.document().id()
            || semantic.stamp.revision != session.revision()
        {
            return Err("Operation preview belongs to a foreign or stale sketch".into());
        }
        if edit.evaluation != semantic.evaluation {
            return Err("Operation solution and semantic preview disagree".into());
        }
        let source = semantic.canonical().clone();
        let Some(EntityData::Sketch {
            plane, definition, ..
        }) = session.document().get(id).map(|e| &e.data)
        else {
            return Err("Select an existing sketch".into());
        };
        if definition.type_id != model::source::TYPE || *plane != semantic.plane {
            return Err("Operation preview belongs to another sketch source or plane".into());
        }
        let schema = definition
            .schema_version
            .max(source.minimum_schema_version())
            .max(2);
        let mut edits = source
            .replace(session.document(), id)
            .map_err(|e| e.to_string())?;
        for edit in &mut edits {
            if let Mutation::Replace(entity) = edit
                && let EntityData::Sketch { definition, .. } = &mut entity.data
            {
                definition.schema_version = schema;
            }
        }
        let candidate = session
            .document()
            .edited(&edits)
            .map_err(|e| e.to_string())?;
        let candidate = multicad_core::parameters::evaluate(&candidate)
            .map_err(|e| e.to_string())?
            .document;
        if !semantic.is_current(&candidate, session.revision()) {
            return Err("Operation semantic source hash is foreign or stale".into());
        }
        let (_, frame, plane) = Source::read_for_evaluation(
            &candidate,
            id,
            session.revision(),
            self.evaluation.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        if semantic.frame != frame || semantic.plane != plane {
            return Err("Operation preview uses a different current sketch frame".into());
        }
        if let Some(preview) = &edit.preview {
            session
                .validate_preview(preview)
                .map_err(|e| e.to_string())?;
            if preview.document() != &candidate || semantic.solved().is_none() {
                return Err("Operation preview and checked source candidate disagree".into());
            }
        } else if semantic.solved().is_some() {
            return Err("A valid operation solution needs its owned source preview".into());
        }
        let geometry = if let Some(solved) = semantic.solved() {
            let success = matches!(
                semantic.evaluation.diagnostics.state,
                SolveState::FullyConstrained | SolveState::Underconstrained
            );
            if !success || !solution_matches_source(&source, solved) {
                return Err(
                    "Operation solution does not match its canonical source geometry".into(),
                );
            }
            Source {
                u: source.u,
                sketch: solved.clone(),
            }
            .validate()
            .map_err(|e| e.to_string())?;
            Some(semantic.geometry().map_err(|e| e.to_string())?)
        } else {
            if matches!(
                semantic.evaluation.diagnostics.state,
                SolveState::FullyConstrained | SolveState::Underconstrained
            ) {
                return Err("Operation has an inconsistent solution status".into());
            }
            None
        };
        self.owner = Some(id);
        self.revision = session.revision();
        self.draft = Some(source);
        self.result = Some(edit);
        self.preview_geometry = geometry;
        self.error = None;
        self.apply_label = label.into();
        self.canonical_draft = true;
        self.point_display = None;
        self.dimension_fields.clear();
        self.selection.clear();
        self.last_element = None;
        Ok(())
    }
    pub fn apply(&mut self, session: &mut Session) -> Result<(), String> {
        self.validate_frame(
            session,
            self.owner
                .ok_or("Choose a sketch before applying constraints")?,
        )?;
        let preview = self
            .result
            .as_ref()
            .and_then(|r| r.preview.as_ref())
            .ok_or("Resolve the draft before applying")?;
        session
            .validate_preview(preview)
            .map_err(|e| e.to_string())?;
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
    pub fn selection(&self) -> &[EntityId] {
        &self.selection
    }
    pub fn select_geometry(
        &mut self,
        id: EntityId,
        element: Option<EntityId>,
        extend: bool,
    ) -> bool {
        if !extend {
            self.selection.clear();
            self.selection.push(id);
        } else {
            self.toggle(id);
        }
        let selected = self.selected(id);
        self.last_element = element.filter(|_| selected);
        selected
    }
    pub fn clear_geometry_selection(&mut self) {
        self.selection.clear();
        self.last_element = None;
    }
    pub fn free(&self, id: EntityId) -> bool {
        self.result
            .as_ref()
            .map(|r| &r.evaluation)
            .or_else(|| self.current.as_ref().map(|r| &r.evaluation))
            .is_some_and(|e| e.diagnostics.free_entities.contains(&id))
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
            if self.owner != Some(id) {
                self.selection.clear();
                self.last_element = None;
            }
            self.owner = Some(id);
            self.revision = session.revision();
            self.result = None;
            self.error = None;
            self.dimension_fields.clear();
            self.point_display = None;
        }
        let mut source = if let Some(draft) = &self.draft {
            draft.clone()
        } else if let Ok((source, _, _)) = read {
            source
        } else {
            ui.colored_label(Color32::LIGHT_RED, "Sketch source is unavailable");
            return;
        };
        self.selection.retain(|id| {
            source.sketch.points.iter().any(|p| p.id == *id)
                || source.sketch.curves.iter().any(|c| c.id == *id)
        });
        if self.last_element != element {
            self.last_element = element;
            self.selection = element.into_iter().collect();
        }
        let evaluation = self.result.as_ref().map(|r| &r.evaluation).or_else(|| {
            self.current
                .as_ref()
                .filter(|r| r.sketch == id && r.stamp.revision == session.revision())
                .map(|r| &r.evaluation)
        });
        if let Some(evaluation) = evaluation {
            let d = &evaluation.diagnostics;
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
                if evaluation.candidate.is_some() {
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
        let point_positions = self.display_positions(session, id);
        ui.collapsing("Select geometry (in order)", |ui| {
            if ui.small_button("Clear selection").clicked() {
                self.selection.clear();
            }
            for (i, p) in source.sketch.points.iter().enumerate() {
                if ui
                    .selectable_label(
                        self.selected(p.id),
                        point_positions
                            .as_ref()
                            .and_then(|points| points.get(&p.id))
                            .map(|p| format!("P{} · {:.2}, {:.2}", i + 1, p[0], p[1]))
                            .unwrap_or_else(|| {
                                format!("P{} · waiting for current solution", i + 1)
                            }),
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
                    model::CurveKind::BSpline { .. } => "Spline",
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
        let mut canonical_intent = false;
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
                        let fix = matches!(kind, Kind::Fix { .. });
                        match self.add_kind(session, id, &mut source, kind) {
                            Ok(()) => {
                                changed = true;
                                canonical_intent |= fix;
                            }
                            Err(error) => self.error = Some(error),
                        }
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
        let mut dimension_edit = None;
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
                    let fields = self.dimension_fields.entry(c.id).or_insert_with(||DimensionFields::from_dimension(d));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut fields.text).desired_width(125.).hint_text("80 or =$width / 2")).on_hover_text("A literal value or a formula beginning with =. Reference parameters with $name.");
                        ui.add(egui::TextEdit::singleline(&mut fields.unit).desired_width(35.)).on_hover_text("Dimension unit, such as mm or deg");
                    });
                    ui.horizontal(|ui| {
                        if ui.small_button("Preview value").on_hover_text("Validate this value or formula without changing the document").clicked() {
                            dimension_edit=Some((c.id, fields.input()));
                        }
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
        if let Some((constraint, input)) = dimension_edit {
            match input.and_then(|input| {
                model::binding::draft_dimension(
                    session.document(),
                    &source,
                    constraint,
                    &input,
                    false,
                )
                .map_err(|e| e.to_string())
            }) {
                Ok(updated) => {
                    source = updated;
                    changed = true;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if changed {
            self.prepare_source(session, id, source, canonical_intent);
        }
        if self.dirty() {
            ui.horizontal(|ui| {
                let ready = self.revision == session.revision()
                    && self.result.as_ref().is_some_and(|r| r.preview.is_some());
                if ui
                    .add_enabled(ready, egui::Button::new(&self.apply_label))
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
    #[test]
    fn expression_draft_previews_without_mutation_and_survives_apply_undo() {
        use multicad_core::document::{Entity, EntityData, Mutation};
        let (mut session, id, _, line) = setup();
        let width = Entity::new(
            "width",
            EntityData::Parameter {
                expression: "160".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Parameter".into(),
                edits: vec![Mutation::Insert(width)],
            })
            .unwrap();
        let before = session.document().clone();
        let mut source = Source::read(session.document(), id).unwrap().0;
        let constraint = source.add_constraint(Kind::Length {
            line,
            dimension: Dimension::Literal {
                value: 80.,
                unit: "mm".into(),
            },
        });
        let input = DimensionFields {
            text: "=$width / 2".into(),
            unit: "mm".into(),
        }
        .input()
        .unwrap();
        let source =
            model::binding::draft_dimension(session.document(), &source, constraint, &input, false)
                .unwrap();
        let mut editor = ConstraintEditor::default();
        editor.prepare(&session, id, source.clone());
        assert_eq!(
            session.document(),
            &before,
            "Preview changed canonical source"
        );
        let semantic = editor
            .semantic_preview()
            .expect("Expression preview missing");
        assert!((semantic.resolved_rhs[&constraint].value - 80.).abs() < 1e-9);
        assert!(editor.semantic_preview_geometry().is_some());
        editor.apply(&mut session).unwrap();
        let saved = Source::read_for_evaluation(session.document(), id, session.revision(), None)
            .unwrap()
            .0;
        assert_eq!(saved, source, "Apply serialized evaluated geometry");
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
        assert!(
            DimensionFields {
                text: "$width / 2".into(),
                unit: "mm".into()
            }
            .input()
            .is_err()
        );
        assert!(
            DimensionFields {
                text: "NaN".into(),
                unit: "mm".into()
            }
            .input()
            .is_err()
        );
    }
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
        editor.set_current(Some(Arc::new(
            multicad_solver::evaluate_document_sketch(
                session.document(),
                session.revision(),
                id,
                Some(&report),
            )
            .unwrap(),
        )));
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
            dimension: Dimension::Literal {
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
                dimension: Dimension::Literal {
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
            dimension: Dimension::Literal {
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

    fn bound_rectangle() -> (Session, EntityId, EntityId, EntityId) {
        use multicad_core::{
            document::{Entity, EntityData, Mutation},
            parameters::BoundExpression,
        };
        let (mut session, id, mut source, line) = setup();
        let width = Entity::new(
            "width",
            EntityData::Parameter {
                expression: "80".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Add parameter".into(),
                edits: vec![Mutation::Insert(width.clone())],
            })
            .unwrap();
        source.add_constraint(Kind::Fix {
            point: source.sketch.points[0].id,
        });
        source.add_constraint(Kind::Length {
            line,
            dimension: Dimension::Expression {
                expression: BoundExpression::bind(session.document(), "=$width", "mm", None)
                    .unwrap(),
            },
        });
        source.add_constraint(Kind::Length {
            line: source.sketch.curves[1].id,
            dimension: Dimension::Literal {
                value: 60.,
                unit: "mm".into(),
            },
        });
        let point = source.sketch.points[2].id;
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Bind rectangle".into(),
                edits: source.replace(session.document(), id).unwrap(),
            })
            .unwrap();
        set_width(&mut session, width.id, "160");
        (session, id, point, width.id)
    }
    fn set_width(session: &mut Session, id: EntityId, value: &str) {
        let mut entity = session.document().get(id).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut entity.data else {
            panic!("parameter")
        };
        *expression = value.into();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Set width".into(),
                edits: vec![Mutation::Replace(entity)],
            })
            .unwrap();
    }
    fn current(session: &Session, id: EntityId) -> Arc<EvaluatedSketch> {
        Arc::new(
            multicad_solver::evaluate_document_sketch(
                session.document(),
                session.revision(),
                id,
                None,
            )
            .unwrap(),
        )
    }
    fn source(session: &Session, id: EntityId) -> Source {
        Source::read_for_evaluation(session.document(), id, session.revision(), None)
            .unwrap()
            .0
    }
    fn show_constraints(
        ctx: &egui::Context,
        editor: &mut ConstraintEditor,
        session: &mut Session,
        id: EntityId,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600., 1400.),
                )),
                events,
                ..Default::default()
            },
            |ui| editor.show(ui, session, id, None),
        );
        output.textures_delta.clear();
        output
    }
    fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing UI text {label}"))
    }
    #[test]
    fn add_fix_click_captures_visible_parameter_point_without_baking_other_source_and_undo_restores_it()
     {
        let (mut session, id, point, _) = bound_rectangle();
        let before = session.document().clone();
        let authored = source(&session, id);
        assert_eq!(authored.sketch.points[2].position_mm, [80., 60.]);
        let mut editor = ConstraintEditor::default();
        editor.set_current(Some(current(&session, id)));
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| style.animation_time = 0.);
        let output = show_constraints(&ctx, &mut editor, &mut session, id, vec![]);
        let header = text_position(&output, "Select geometry (in order)");
        show_constraints(
            &ctx,
            &mut editor,
            &mut session,
            id,
            vec![
                egui::Event::PointerMoved(header),
                egui::Event::PointerButton {
                    pos: header,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: header,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let output = show_constraints(&ctx, &mut editor, &mut session, id, vec![]);
        text_position(&output, "P3 · 160.00, 60.00");
        assert_eq!(
            source(&session, id).sketch.points[2].position_mm,
            [80., 60.]
        );
        let cached = editor.display_positions(&session, id).unwrap();
        show_constraints(&ctx, &mut editor, &mut session, id, vec![]);
        assert!(Arc::ptr_eq(
            &cached,
            &editor.display_positions(&session, id).unwrap()
        ));
        editor.tool = Tool::Fix;
        editor.selection = vec![point];
        let output = show_constraints(&ctx, &mut editor, &mut session, id, vec![]);
        let p = text_position(&output, "Add");
        show_constraints(
            &ctx,
            &mut editor,
            &mut session,
            id,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(editor.preview_document().is_some(), "{:?}", editor.error);
        assert_eq!(session.document(), &before);
        let captured = editor.semantic_preview().unwrap().canonical();
        assert_eq!(captured.sketch.points[2].position_mm, [160., 60.]);
        for (old, new) in authored.sketch.points.iter().zip(&captured.sketch.points) {
            if old.id != point {
                assert_eq!(old, new);
            }
        }
        assert_eq!(captured.sketch.curves, authored.sketch.curves);
        assert_eq!(
            captured.sketch.constraints[..authored.sketch.constraints.len()],
            authored.sketch.constraints
        );
        assert!(
            matches!(captured.sketch.constraints.last().unwrap().kind,Kind::Fix {point:p} if p==point)
        );
        let revision = session.revision();
        editor.apply(&mut session).unwrap();
        assert_eq!(session.revision(), revision + 1);
        let evaluated = current(&session, id);
        assert_eq!(
            evaluated
                .solved()
                .unwrap()
                .points
                .iter()
                .find(|p| p.id == point)
                .unwrap()
                .position_mm,
            [160., 60.]
        );
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn fix_uses_visible_draft_candidate_and_rejects_missing_stale_or_conflicting_solutions() {
        use model::binding::{DimensionInput, draft_dimension};
        let (mut session, id, point, width) = bound_rectangle();
        let original = source(&session, id);
        let mut editor = ConstraintEditor::default();
        let mut missing = original.clone();
        assert!(
            editor
                .add_kind(&session, id, &mut missing, Kind::Fix { point })
                .is_err()
        );
        assert_eq!(missing, original);
        editor.set_current(Some(current(&session, id)));
        set_width(&mut session, width, "180");
        let mut stale = source(&session, id);
        let before = stale.clone();
        assert!(
            editor
                .add_kind(&session, id, &mut stale, Kind::Fix { point })
                .is_err()
        );
        assert_eq!(stale, before);
        editor.set_current(Some(current(&session, id)));
        let mut draft = source(&session, id);
        let constraint = draft
            .sketch
            .constraints
            .iter()
            .find(|c| {
                matches!(c.kind, Kind::Length { .. })
                    && c.kind.dimension().is_some_and(|d| d.expression().is_some())
            })
            .unwrap()
            .id;
        draft = draft_dimension(
            session.document(),
            &draft,
            constraint,
            &DimensionInput::Expression {
                text: "=$width + 20 mm".into(),
                unit: "mm".into(),
            },
            false,
        )
        .unwrap();
        editor.prepare(&session, id, draft.clone());
        let visible = editor
            .semantic_preview()
            .unwrap()
            .solved()
            .unwrap()
            .points
            .iter()
            .find(|p| p.id == point)
            .unwrap()
            .position_mm;
        assert!((visible[0] - 200.).abs() < 1e-8 && (visible[1] - 60.).abs() < 1e-8);
        editor
            .add_kind(&session, id, &mut draft, Kind::Fix { point })
            .unwrap();
        assert_eq!(draft.sketch.points[2].position_mm, visible);
        editor.prepare_source(&session, id, draft.clone(), true);
        assert!(editor.preview_document().is_some());
        draft.add_constraint(Kind::Length {
            line: draft.sketch.curves[0].id,
            dimension: Dimension::Literal {
                value: 999.,
                unit: "mm".into(),
            },
        });
        editor.prepare(&session, id, draft.clone());
        assert!(editor.preview_document().is_none());
        let before = draft.clone();
        assert!(
            editor
                .add_kind(&session, id, &mut draft, Kind::Fix { point })
                .is_err()
        );
        assert_eq!(draft, before);
    }
    fn operation(session: &Session, id: EntityId) -> SketchEdit {
        use multicad_core::sketch_pattern::{Pattern, PatternOptions, PatternRequest};
        let request = PatternRequest {
            selection: source(session, id)
                .sketch
                .curves
                .iter()
                .map(|c| c.id)
                .collect(),
            pattern: Pattern::Linear {
                step_mm: [300., 0.],
                count: 2,
            },
            options: PatternOptions::default(),
        };
        multicad_solver::prepare_pattern(
            session,
            session.revision(),
            Actor {
                id: "operation-client".into(),
                channel: Channel::Mcp,
            },
            id,
            None,
            &request,
        )
        .unwrap()
        .edit
    }
    #[test]
    fn checked_operation_adoption_keeps_original_preview_apply_label_and_one_undo_without_second_solve()
     {
        let (session, id, _, _) = bound_rectangle();
        let mut session = Session::new(session.document().clone()).unwrap();
        let before = session.document().clone();
        let edit = operation(&session, id);
        let points = edit.evaluation.candidate.as_ref().unwrap().points.as_ptr();
        let expected_source = edit.semantic.as_ref().unwrap().canonical().clone();
        let mut editor = ConstraintEditor::default();
        editor
            .accept_edit(&session, id, edit, "Apply pattern")
            .unwrap();
        assert_eq!(session.document(), &before);
        assert_eq!(
            editor
                .result
                .as_ref()
                .unwrap()
                .evaluation
                .candidate
                .as_ref()
                .unwrap()
                .points
                .as_ptr(),
            points,
            "Adoption must retain the already checked solution"
        );
        assert_eq!(editor.draft.as_ref(), Some(&expected_source));
        assert!(editor.semantic_preview_geometry().is_some());
        let ctx = egui::Context::default();
        let output = show_constraints(&ctx, &mut editor, &mut session, id, vec![]);
        text_position(&output, "Apply pattern");
        assert_eq!(editor.apply_label, "Apply pattern");
        editor.apply(&mut session).unwrap();
        assert_eq!(session.revision(), 1);
        assert_eq!(session.activity().count(), 1);
        assert_eq!(
            session.activity().last().unwrap().actor.id,
            "operation-client"
        );
        assert_eq!(source(&session, id), expected_source);
        assert_eq!(editor.apply_label, "Apply constraints");
        session.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn adoption_rejects_foreign_sessions_and_mismatched_metadata_without_overwriting_existing_drafts()
     {
        let (session, id, _, _) = bound_rectangle();
        let session = Session::new(session.document().clone()).unwrap();
        let foreign = Session::new(session.document().clone()).unwrap();
        let mut editor = ConstraintEditor::default();
        assert!(
            editor
                .accept_edit(&session, id, operation(&foreign, id), "Apply pattern")
                .unwrap_err()
                .contains("different session")
        );
        assert!(!editor.dirty());
        let mut edit = operation(&session, id);
        edit.semantic.as_mut().unwrap().stamp.source_hash =
            multicad_core::project_file::ContentHash::of(b"forged");
        assert!(
            editor
                .accept_edit(&session, id, edit, "Apply pattern")
                .is_err()
        );
        assert!(!editor.dirty());
        let mut edit = operation(&session, id);
        edit.semantic.as_mut().unwrap().sketch = EntityId::new();
        assert!(
            editor
                .accept_edit(&session, id, edit, "Apply pattern")
                .is_err()
        );
        assert!(!editor.dirty());
        let mut edit = operation(&session, id);
        edit.semantic.as_mut().unwrap().frame.origin[2] = 1.;
        assert!(
            editor
                .accept_edit(&session, id, edit, "Apply pattern")
                .is_err()
        );
        assert!(!editor.dirty());
        let mut edit = operation(&session, id);
        edit.evaluation.candidate.as_mut().unwrap().points[0].position_mm = [999., 999.];
        assert!(
            editor
                .accept_edit(&session, id, edit, "Apply pattern")
                .is_err()
        );
        assert!(!editor.dirty());
        editor
            .accept_edit(&session, id, operation(&session, id), "Apply pattern")
            .unwrap();
        let retained = editor.draft.clone();
        assert!(
            editor
                .accept_edit(&session, id, operation(&session, id), "Apply pattern")
                .is_err()
        );
        assert_eq!(editor.draft, retained);
        editor.discard();
        assert_eq!(editor.apply_label, "Apply constraints");
        let (plain, plain_id, plain_source, _) = setup();
        let edit = prepare_edit(
            &plain,
            plain.revision(),
            Actor::local(Channel::Ui),
            plain_id,
            plain_source,
        )
        .unwrap();
        assert!(edit.semantic.is_none());
        assert!(
            editor
                .accept_edit(&plain, plain_id, edit, "Apply move")
                .is_err()
        );
        assert!(!editor.dirty());
    }
    #[test]
    fn diagnostic_operation_drafts_are_repairable_and_stale_apply_never_commits() {
        let (session, id, _, width) = bound_rectangle();
        let mut session = Session::new(session.document().clone()).unwrap();
        let before = session.document().clone();
        let mut invalid = source(&session, id);
        invalid.add_constraint(Kind::Length {
            line: invalid.sketch.curves[0].id,
            dimension: Dimension::Literal {
                value: 999.,
                unit: "mm".into(),
            },
        });
        let edit = multicad_solver::prepare_canonical_edit(
            &session,
            0,
            Actor::local(Channel::Ui),
            id,
            invalid.clone(),
            None,
        )
        .unwrap();
        assert!(edit.preview.is_none());
        let mut editor = ConstraintEditor::default();
        editor
            .accept_edit(&session, id, edit, "Apply move")
            .unwrap();
        assert!(editor.dirty());
        assert_eq!(editor.draft, Some(invalid));
        assert!(editor.preview_document().is_none());
        assert!(editor.apply(&mut session).is_err());
        assert_eq!(session.document(), &before);
        editor.discard();
        let edit = operation(&session, id);
        editor
            .accept_edit(&session, id, edit, "Apply pattern")
            .unwrap();
        set_width(&mut session, width, "180");
        let changed = session.document().clone();
        assert!(editor.apply(&mut session).is_err());
        assert_eq!(session.document(), &changed);
        assert!(editor.dirty());
        assert!(editor.result.as_ref().unwrap().preview.is_some());
        editor.discard();
        let stale = operation(&session, id);
        set_width(&mut session, width, "200");
        assert!(
            editor
                .accept_edit(&session, id, stale, "Apply pattern")
                .is_err()
        );
        assert!(!editor.dirty());
    }
    #[test]
    fn literal_fix_draft_keeps_canonical_mode_through_later_constraint_repairs() {
        let (mut session, id, mut source, edge) = setup();
        let corner = source.sketch.points[2].id;
        let a = source.add_point([200., 0.]);
        let b = source.add_point([219., 1.]);
        let unrelated = source.add_line(a, b);
        source.add_constraint(Kind::Length {
            line: unrelated,
            dimension: Dimension::Literal {
                value: 30.,
                unit: "mm".into(),
            },
        });
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Rough unrelated line".into(),
                edits: source.replace(session.document(), id).unwrap(),
            })
            .unwrap();
        let before = session.document().clone();
        let mut editor = ConstraintEditor::default();
        editor.set_current(Some(current(&session, id)));
        editor
            .add_kind(&session, id, &mut source, Kind::Fix { point: corner })
            .unwrap();
        editor.prepare_source(&session, id, source.clone(), true);
        assert!(editor.semantic_preview().is_some());
        assert!(editor.canonical_draft);
        editor.set_evaluation(Some(ReferenceReport {
            revision: session.revision(),
            entries: vec![],
        }));
        let mut updated = editor.draft.clone().unwrap();
        updated.add_constraint(Kind::Length {
            line: edge,
            dimension: Dimension::Literal {
                value: 999.,
                unit: "mm".into(),
            },
        });
        updated.sketch.constraints.last_mut().unwrap().reference = true;
        editor.prepare(&session, id, updated.clone());
        assert_eq!(editor.semantic_preview().unwrap().canonical(), &updated);
        assert_eq!(
            editor.semantic_preview().unwrap().canonical().sketch.points[4..],
            source.sketch.points[4..]
        );
        assert_eq!(session.document(), &before);
        assert_eq!(editor.apply_label, "Apply constraints");
        editor.discard();
        assert!(!editor.canonical_draft);
    }
}
