//! Fillet and symmetric chamfer share explicit captured upstream edges. Only
//! canonical typed commands and complete native receipts can change the Part.
mod command;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui, viewport::Scene};
pub use command::EdgeTreatmentCommand;
use multicad_core::{
    document::{Document, EntityId},
    model::{self, BodyInput, EdgeTreatmentParameters, EdgeTreatmentProfile, EdgeTreatmentRequest},
    project_file::ContentHash,
    references::{self, GeometryIntent, Resolution, TopologyKind},
};
use std::collections::BTreeSet;
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
            vec![],
            choice.label,
        )?;
        panel.set_edges(scene, &edges)?;
        panel.picking = true;
        panel.picking_available = true;
        Ok(panel)
    }
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let (source, edges) =
            model::read_edge_treatment(document, feature).map_err(|e| e.to_string())?;
        let name = document
            .get(source.input.body)
            .map_or("Missing Part", |e| e.name.as_str())
            .to_owned();
        Self::from_source(source.input, feature, true, source.parameters, edges, name)
    }
    fn from_source(
        input: BodyInput,
        feature: EntityId,
        editing: bool,
        parameters: EdgeTreatmentParameters,
        edges: Vec<GeometryIntent>,
        name: String,
    ) -> Result<Self, String> {
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
        self.picking && self.picking_available
    }
    pub fn set_picking_available(&mut self, available: bool) {
        self.picking_available = available;
    }
    pub fn stop_picking(&mut self) {
        self.picking = false;
    }
    pub fn set_edges(&mut self, scene: &Scene, edges: &[GeometryIntent]) -> Result<bool, String> {
        if edges.len() > model::MAX_TREATMENT_EDGES {
            return Err("Select at most 128 explicit Part edges".into());
        }
        let snapshot = scene
            .geometry_snapshot(self.input.producer)
            .ok_or("The frozen upstream producer has no current native topology")?;
        let mut keys = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for edge in edges {
            edge.validate()?;
            if edge.kind != TopologyKind::Edge || !ids.insert(edge.id) {
                return Err("Select distinct edge captures from this Part".into());
            }
            let Resolution::Resolved { key, .. } =
                references::resolve(edge, &snapshot, scene.stamp().revision)
            else {
                return Err("An edge capture is missing, ambiguous or belongs to a different upstream source".into());
            };
            if !keys.insert(key) {
                return Err("Select each native edge only once".into());
            }
        }
        if self.edges == edges {
            return Ok(false);
        }
        self.edges = edges.to_vec();
        self.stamp.changed();
        Ok(true)
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
        Ok(if self.editing {
            EdgeTreatmentCommand::Edit {
                feature: self.feature,
                parameters,
                edges: self.edges.clone(),
            }
        } else {
            EdgeTreatmentCommand::Create(EdgeTreatmentRequest {
                feature: self.feature,
                input: self.input.clone(),
                parameters,
                edges: self.edges.clone(),
            })
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
        ui.horizontal_wrapped(|ui| {
            for (profile, label) in [
                (EdgeTreatmentProfile::Fillet, "Fillet"),
                (EdgeTreatmentProfile::Chamfer, "Chamfer"),
            ] {
                changed |= ui
                    .selectable_value(&mut self.profile, profile, label)
                    .on_hover_text(
                        "Switch profile while retaining the same Part, edges and bound dimension",
                    )
                    .changed();
            }
        });
        changed |= self.size.show(
            ui,
            if self.profile == EdgeTreatmentProfile::Fillet {
                "Radius"
            } else {
                "Symmetric distance"
            },
            "Positive millimeters or =$parameter; the complete native preview validates the solid",
        );
        if self.profile == EdgeTreatmentProfile::Chamfer {
            ui.small("Symmetric chamfer only. Angles and unequal distances are unsupported.");
        }
        ui.label(format!("{} captured edge(s)", self.edges.len()));
        if ui
            .add_enabled(
                self.picking_available,
                egui::Button::new(if self.picking {
                    "Selecting edges…"
                } else {
                    "Select / reselect edges"
                }),
            )
            .on_hover_text(
                "Pick the exact frozen upstream Part edges; Shift adds or removes captures",
            )
            .on_disabled_hover_text(
                "Wait for complete current native source geometry before picking new edges",
            )
            .clicked()
        {
            self.picking = !self.picking;
        }
        if self.picking() {
            ui.small("Click an edge in the source view. Shift toggles edges. Preview returns to the complete model.");
        }
        let mut remove = None;
        for (index, edge) in self.edges.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Edge capture {}", index + 1))
                    .on_hover_text(format!(
                        "Persistent capture {} · upstream producer {}",
                        edge.id, self.input.producer
                    ));
                if ui
                    .small_button("Remove")
                    .on_hover_text(
                        "Remove this capture from the local tool without changing geometry",
                    )
                    .clicked()
                {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.edges.remove(index);
            changed = true;
        }
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

#[cfg(test)]
mod tests;
