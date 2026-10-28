//! One edge-treatment Inspector with explicit topology for sided chamfers.
mod command;
mod sides;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui, viewport::Scene};
pub use command::EdgeTreatmentCommand;
use multicad_core::{
    document::{Document, EntityId},
    model::{
        self, BodyInput, ChamferSettings, EdgeSide, EdgeTreatmentParameters, EdgeTreatmentProfile,
        EdgeTreatmentRequest,
    },
    project_file::ContentHash,
    references::{self, GeometryIntent, Resolution, TopologyKind},
};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChamferMode {
    Symmetric,
    TwoDistances,
    DistanceAngle,
}
impl ChamferMode {
    fn label(self) -> &'static str {
        match self {
            Self::Symmetric => "Symmetric distances",
            Self::TwoDistances => "Two distances",
            Self::DistanceAngle => "Distance + angle",
        }
    }
}
pub(super) struct EdgeTreatmentPanel {
    input: BodyInput,
    feature: EntityId,
    editing: bool,
    edges: Vec<GeometryIntent>,
    profile: EdgeTreatmentProfile,
    size: DimensionField,
    stamp: FormStamp,
    picking: bool,
    picking_available: bool,
    name: String,
    mode: ChamferMode,
    second: DimensionField,
    angle: DimensionField,
    sides: Vec<EdgeSide>,
    side_faces: Vec<GeometryIntent>,
    face_for: Option<EntityId>,
}
impl EdgeTreatmentPanel {
    pub fn new(
        document: &Document,
        revision: u64,
        scene: &Scene,
        body: EntityId,
        edges: Vec<GeometryIntent>,
    ) -> Result<Self, String> {
        if scene
            .model_evaluation()
            .is_none_or(|info| ContentHash::document(document).ok() != Some(info.source_hash))
        {
            return Err("Edge selection requires current source-bound native geometry".into());
        }
        let choice = super::body_choices::choices(document, revision, scene)?
            .into_iter()
            .find(|c| c.input.body == body)
            .ok_or(
                "Choose one current Part definition; Instance-only edge treatment is unsupported",
            )?;
        let mut panel = Self::from_source(
            choice.input,
            EntityId::new(),
            false,
            EdgeTreatmentParameters::default(),
            None,
            vec![],
            choice.label,
        )?;
        panel.set_edges(scene, &edges)?;
        panel.picking = true;
        panel.picking_available = true;
        Ok(panel)
    }
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let (source, captures) =
            model::read_edge_treatment(document, feature).map_err(|e| e.to_string())?;
        let name = document
            .get(source.input.body)
            .map_or("Missing Part", |e| e.name.as_str())
            .to_owned();
        Self::from_source(
            source.input,
            feature,
            true,
            source.parameters,
            source.chamfer,
            captures,
            name,
        )
    }
    fn from_source(
        input: BodyInput,
        feature: EntityId,
        editing: bool,
        parameters: EdgeTreatmentParameters,
        chamfer: Option<ChamferSettings>,
        captures: Vec<GeometryIntent>,
        name: String,
    ) -> Result<Self, String> {
        let (mode, second, angle, sides) = match chamfer {
            Some(ChamferSettings::TwoDistances { second, sides }) => {
                (ChamferMode::TwoDistances, second, model::angle(45.), sides)
            }
            Some(ChamferSettings::DistanceAngle { angle, sides }) => {
                (ChamferMode::DistanceAngle, model::length(2.), angle, sides)
            }
            None => (
                ChamferMode::Symmetric,
                model::length(2.),
                model::angle(45.),
                vec![],
            ),
        };
        let (edges, side_faces) = captures
            .into_iter()
            .partition(|c| c.kind == TopologyKind::Edge);
        Ok(Self {
            input,
            feature,
            editing,
            edges,
            profile: parameters.profile,
            size: DimensionField::from_dimension(&parameters.size, "mm")?,
            stamp: FormStamp::new(),
            picking: !editing,
            picking_available: false,
            name,
            mode,
            second: DimensionField::from_dimension(&second, "mm")?,
            angle: DimensionField::from_dimension(&angle, "deg")?,
            sides,
            side_faces,
            face_for: None,
        })
    }
    pub fn label(&self) -> &'static str {
        "Fillet / Chamfer"
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn input(&self) -> &BodyInput {
        &self.input
    }
    pub fn edges(&self) -> &[GeometryIntent] {
        &self.edges
    }
    pub fn picking(&self) -> bool {
        self.picking_available && (self.picking || self.face_for.is_some())
    }
    pub fn pick_kind(&self) -> Option<TopologyKind> {
        self.picking().then_some(if self.face_for.is_some() {
            TopologyKind::Face
        } else {
            TopologyKind::Edge
        })
    }
    pub fn pick_captures(&self) -> &[GeometryIntent] {
        if let Some(edge) = self.face_for {
            if let Some(face) = self
                .sides
                .iter()
                .find(|s| s.edge == edge)
                .and_then(|s| self.side_faces.iter().find(|c| c.id == s.face))
            {
                return std::slice::from_ref(face);
            }
            &[]
        } else {
            &self.edges
        }
    }
    pub fn set_picking_available(&mut self, available: bool) {
        self.picking_available = available;
    }
    pub fn stop_picking(&mut self) {
        self.picking = false;
        self.face_for = None;
    }
    pub fn set_edges(&mut self, scene: &Scene, edges: &[GeometryIntent]) -> Result<bool, String> {
        if edges.len() > model::MAX_TREATMENT_EDGES {
            return Err("Select at most 128 explicit Part edges".into());
        }
        validate_captures(scene, &self.input, edges, TopologyKind::Edge)?;
        if self.edges == edges {
            return Ok(false);
        }
        self.edges = edges.to_vec();
        self.prune_sides();
        self.stamp.changed();
        Ok(true)
    }
    pub fn set_pick_captures(
        &mut self,
        scene: &Scene,
        captures: &[GeometryIntent],
    ) -> Result<bool, String> {
        if self.face_for.is_some() {
            self.set_side_face(scene, captures)
        } else {
            self.set_edges(scene, captures)
        }
    }
    fn parameters(&self, document: &Document) -> Result<EdgeTreatmentParameters, String> {
        let parameters = EdgeTreatmentParameters {
            profile: self.profile,
            size: self.size.resolve(document)?,
        };
        parameters
            .resolved_size_mm(
                &multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        Ok(parameters)
    }
    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<EdgeTreatmentCommand, String> {
        if !enabled {
            return Err("The native edge-treatment provider is unavailable".into());
        }
        if self.edges.is_empty() {
            return Err("Select one or more explicit edges in the viewer before Preview".into());
        }
        let parameters = self.parameters(document)?;
        let chamfer = if self.profile == EdgeTreatmentProfile::Chamfer {
            match self.mode {
                ChamferMode::Symmetric => None,
                ChamferMode::TwoDistances => Some(ChamferSettings::TwoDistances {
                    second: self.second.resolve(document)?,
                    sides: self.sides.clone(),
                }),
                ChamferMode::DistanceAngle => Some(ChamferSettings::DistanceAngle {
                    angle: self.angle.resolve(document)?,
                    sides: self.sides.clone(),
                }),
            }
        } else {
            None
        };
        if let Some(settings) = &chamfer {
            settings
                .validate(&self.edges.iter().map(|c| c.id).collect::<Vec<_>>())
                .map_err(|e| e.to_string())?;
            settings
                .resolve(&multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
        let mut captures = self.edges.clone();
        if chamfer.is_some() {
            captures.extend(self.side_faces.clone());
        }
        let request = EdgeTreatmentRequest {
            feature: self.feature,
            input: self.input.clone(),
            parameters,
            edges: captures,
            chamfer,
        };
        Ok(if self.editing {
            EdgeTreatmentCommand::Edit(request)
        } else {
            EdgeTreatmentCommand::Create(request)
        })
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong(self.label());
        ui.label(&self.name);
        ui.small("Part definition · changes propagate to linked Instances. Instance-only edge treatment is unavailable.");
        let mut changed = false;
        ui.horizontal_wrapped(|ui|{for(profile,label)in[(EdgeTreatmentProfile::Fillet,"Fillet"),(EdgeTreatmentProfile::Chamfer,"Chamfer")]{changed|=ui.selectable_value(&mut self.profile,profile,label).on_hover_text("Switch profile while retaining the Part, selected edges and first bound dimension").changed();}});
        if self.profile == EdgeTreatmentProfile::Chamfer {
            controls::inspector_combo(ui, "chamfer-mode")
                .selected_text(self.mode.label())
                .show_ui(ui, |ui| {
                    for mode in [
                        ChamferMode::Symmetric,
                        ChamferMode::TwoDistances,
                        ChamferMode::DistanceAngle,
                    ] {
                        changed |= ui
                            .selectable_value(&mut self.mode, mode, mode.label())
                            .changed();
                    }
                });
        }
        if self.profile == EdgeTreatmentProfile::Fillet || self.mode == ChamferMode::Symmetric {
            self.sides.clear();
            self.side_faces.clear();
            self.face_for = None;
        }
        let label = if self.profile == EdgeTreatmentProfile::Fillet {
            "Radius"
        } else if self.mode == ChamferMode::Symmetric {
            "Symmetric distance"
        } else {
            "First distance"
        };
        changed |= self.size.show(
            ui,
            label,
            "Positive millimeters or =$parameter; the complete native preview validates the solid",
        );
        if self.profile == EdgeTreatmentProfile::Chamfer {
            match self.mode{
                ChamferMode::TwoDistances=>changed|=self.second.show(ui,"Second distance","Positive millimeters measured relative to each explicitly selected adjacent face"),
                ChamferMode::DistanceAngle=>changed|=self.angle.show(ui,"Chamfer angle","An angle strictly between zero and 90 degrees; orientation follows each explicit adjacent face"),
                ChamferMode::Symmetric=>{}
            }
        }
        ui.label(format!("{} captured edge(s)", self.edges.len()));
        if ui
            .add_enabled(
                self.picking_available,
                egui::Button::new(if self.picking && self.face_for.is_none() {
                    "Selecting edges…"
                } else {
                    "Select / reselect edges"
                }),
            )
            .on_hover_text(
                "Pick the exact frozen upstream Part edges; Shift adds or removes captures",
            )
            .on_disabled_hover_text(
                "Wait for complete current native source geometry before picking",
            )
            .clicked()
        {
            self.picking = !self.picking;
            self.face_for = None;
        }
        if self.picking() {
            ui.small(if self.face_for.is_some(){"Choose one adjacent face for the indicated edge. Native Preview checks adjacency and direction."}else{"Click an upstream edge. Shift toggles edges. Preview returns to the complete model."});
        }
        changed |= self.show_edge_sides(ui);
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        form::preview_status(ui, self.stamp, preview);
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
        }
        match action {
            Some(FormAction::Preview) => {
                let request = self.request(document, enabled).ok()?;
                let parameters = self.parameters(document).ok()?;
                self.size.remember(&parameters.size);
                if let Some(chamfer) = match &request {
                    EdgeTreatmentCommand::Create(r) | EdgeTreatmentCommand::Edit(r) => {
                        r.chamfer.as_ref()
                    }
                } {
                    match chamfer {
                        ChamferSettings::TwoDistances { second, .. } => {
                            self.second.remember(second)
                        }
                        ChamferSettings::DistanceAngle { angle, .. } => self.angle.remember(angle),
                    }
                }
                self.stop_picking();
                Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::EdgeTreatment(Box::new(request)),
                })
            }
            Some(FormAction::Apply) if !changed && preview.ready(self.stamp) => {
                Some(ModelEvent::Apply)
            }
            _ if changed => Some(ModelEvent::Changed),
            _ => None,
        }
    }
}
fn validate_captures(
    scene: &Scene,
    input: &BodyInput,
    captures: &[GeometryIntent],
    kind: TopologyKind,
) -> Result<(), String> {
    let snapshot = scene
        .geometry_snapshot(input.producer)
        .ok_or("The frozen upstream producer has no current native topology")?;
    let mut keys = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for capture in captures {
        capture.validate()?;
        if capture.kind != kind || !ids.insert(capture.id) {
            return Err("Select distinct captures of the requested topology kind".into());
        }
        let Resolution::Resolved { key, .. } =
            references::resolve(capture, &snapshot, scene.stamp().revision)
        else {
            return Err(
                "A capture is missing, ambiguous or belongs to a different upstream source".into(),
            );
        };
        if key.kind != kind || !keys.insert(key) {
            return Err("Select each native element only once".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
