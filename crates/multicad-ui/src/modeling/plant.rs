//! Landscape controls use the same canonical source transactions as MCP.
use super::TerminationViewport;
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityData, EntityId, Placement},
    model::{self, ModelPrepared},
    plants::{self, PlantKind, SizeBasis},
    symbols,
};
#[derive(Clone, Debug)]
pub enum PlantCommand {
    Place(Box<plants::PlantPlaceRequest>),
    Edit(Box<plants::PlantEditRequest>),
}
impl PlantCommand {
    pub fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let prepared = match self {
            Self::Place(r) => plants::prepare_place(session, session.revision(), actor, r),
            Self::Edit(r) => plants::prepare_edit_request(session, session.revision(), actor, r),
        }
        .map_err(|e| e.to_string())?;
        Ok(ModelPrepared {
            prepared,
            outputs: vec![],
        })
    }
}
pub(super) struct PlantPanel {
    kind: PlantKind,
    seeds: [plants::PlantPlaceRequest; 2],
    existing: Option<EntityId>,
    name: String,
    height: f64,
    spread: f64,
    position: [f64; 3],
    rotation_deg: f64,
    size_basis: SizeBasis,
    label: String,
    plan_visible: bool,
    placing: bool,
    placed: bool,
}
impl PlantPanel {
    pub(super) fn existing(document: &Document, record: EntityId) -> Result<Self, String> {
        let mut panel = Self::new(document)?;
        panel.existing = Some(record);
        panel.load(document)?;
        Ok(panel)
    }
    pub fn new(document: &Document) -> Result<Self, String> {
        let session = Session::new(document.clone()).map_err(|e| e.to_string())?;
        let seed = |kind, height, spread| {
            plants::PlantPlaceRequest::new(
                &session,
                plants::generic_definition(kind).map_err(|e| e.to_string())?,
                "Plant".into(),
                height,
                spread,
                Placement::default(),
            )
            .map_err(|e| e.to_string())
        };
        Ok(Self {
            kind: PlantKind::Tree,
            seeds: [
                seed(PlantKind::Tree, 6000., 4000.)?,
                seed(PlantKind::Shrub, 800., 1000.)?,
            ],
            existing: None,
            name: "Tree".into(),
            height: 6000.,
            spread: 4000.,
            position: [0.; 3],
            rotation_deg: 0.,
            size_basis: SizeBasis::PlannedMature,
            label: String::new(),
            plan_visible: true,
            placing: true,
            placed: false,
        })
    }
    fn load(&mut self, document: &Document) -> Result<(), String> {
        let Some(record) = self.existing else {
            return Ok(());
        };
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
        let plant = plants::resolve(document, &parameters, record)
            .map_err(|e| e.to_string())?
            .ok_or("Choose a plant")?;
        let pose = model::resolve_instance_pose(document, plant.instance, &parameters)
            .map_err(|e| e.to_string())?;
        if pose.rotation[2][2] < 1. - 1e-8 {
            return Err("This plant is tilted; use Move / Copy to edit its pose".into());
        }
        self.position = pose.translation_mm;
        self.rotation_deg = pose.rotation[1][0].atan2(pose.rotation[0][0]).to_degrees();
        self.kind = plant.template.kind;
        self.height = plant.height_mm;
        self.spread = plant.spread_mm;
        self.name = document
            .get(plant.instance)
            .expect("resolved plant")
            .name
            .clone();
        self.label = plant.label.unwrap_or_default();
        self.size_basis = plant.size_basis;
        self.plan_visible = plant.plan_visible;
        self.placing = false;
        self.placed = true;
        Ok(())
    }
    pub fn request(&self, document: &Document) -> Result<PlantCommand, String> {
        if !self.placed {
            return Err("Click in the garden view to place the plant".into());
        }
        if [self.height, self.spread]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v > 1e7)
            || self.name.trim().is_empty()
        {
            return Err("Enter a name and positive height/spread within modeling limits".into());
        }
        let label = (!self.label.trim().is_empty()).then(|| self.label.clone());
        if let Some(record) = self.existing {
            let move_by = plants::planar_move(
                document,
                &multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?,
                record,
                self.position,
                self.rotation_deg,
            )
            .map_err(|e| e.to_string())?;
            Ok(PlantCommand::Edit(Box::new(plants::PlantEditRequest {
                record,
                height_mm: self.height,
                spread_mm: self.spread,
                name: self.name.clone(),
                settings: plants::PlantInstance {
                    symbol_instance: record,
                    size_basis: self.size_basis,
                    label,
                    plan_visible: self.plan_visible,
                },
                move_by,
            })))
        } else {
            let half = self.rotation_deg.to_radians() / 2.;
            let mut request = self.seeds[if self.kind == PlantKind::Tree { 0 } else { 1 }].clone();
            let template =
                plants::template_for(&request.definition.template, request.definition.root)
                    .map_err(|e| e.to_string())?
                    .expect("generic plant");
            request.placement.name = self.name.clone();
            request.placement.placement = Placement {
                translation_mm: self.position,
                rotation_xyzw: [0., 0., half.sin(), half.cos()],
            };
            for (id, value) in [
                (template.height_parameter, self.height),
                (template.spread_parameter, self.spread),
            ] {
                request.placement.overrides.insert(
                    id,
                    symbols::SymbolValue {
                        value,
                        unit: "mm".into(),
                    },
                );
            }
            request.settings.size_basis = self.size_basis;
            request.settings.label = label;
            request.settings.plan_visible = self.plan_visible;
            Ok(PlantCommand::Place(Box::new(request)))
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        let mut instances = vec![];
        if let Ok(parameters) = multicad_core::parameters::evaluate(document)
            && let Ok(resolver) = plants::Resolver::new(document, &parameters)
        {
            for entity in document.entities() {
                if let EntityData::Extension { definition, .. } = &entity.data
                    && definition.type_id == symbols::INSTANCE_TYPE
                    && let Ok(Some(plant)) = resolver.resolve(entity.id)
                {
                    instances.push((
                        entity.id,
                        document.get(plant.instance).expect("plant").name.clone(),
                    ));
                }
            }
        }
        let before = self.existing;
        controls::inspector_combo(ui, "plant-existing")
            .selected_text(
                self.existing
                    .and_then(|id| {
                        instances
                            .iter()
                            .find(|(i, _)| *i == id)
                            .map(|(_, n)| n.as_str())
                    })
                    .unwrap_or("New plant"),
            )
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut self.existing, None, "New plant")
                    .changed();
                for (id, name) in instances {
                    changed |= ui
                        .selectable_value(&mut self.existing, Some(id), name)
                        .changed();
                }
            });
        if before != self.existing {
            if self.existing.is_some() {
                if let Err(e) = self.load(document) {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                }
            } else {
                self.placed = false;
                self.placing = true;
            }
        }
        ui.horizontal(|ui| {
            ui.label("Plant");
            for (kind, label) in [(PlantKind::Tree, "Tree"), (PlantKind::Shrub, "Shrub")] {
                if ui
                    .add_enabled(
                        self.existing.is_none(),
                        egui::Button::selectable(self.kind == kind, label),
                    )
                    .clicked()
                {
                    self.kind = kind;
                    self.height = if kind == PlantKind::Tree { 6000. } else { 800. };
                    self.spread = if kind == PlantKind::Tree {
                        4000.
                    } else {
                        1000.
                    };
                    self.name = label.into();
                    changed = true;
                }
            }
        });
        ui.label("Name");
        changed |= ui.text_edit_singleline(&mut self.name).changed();
        for (label, value) in [
            ("Height (mm)", &mut self.height),
            ("Canopy / spread (mm)", &mut self.spread),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                changed |= ui
                    .add(egui::DragValue::new(value).speed(10.).range(1.0..=1e7))
                    .changed();
            });
        }
        ui.label("Size describes");
        ui.horizontal_wrapped(|ui| {
            changed |= ui
                .selectable_value(
                    &mut self.size_basis,
                    SizeBasis::PlannedMature,
                    "Planned / mature",
                )
                .changed();
            changed |= ui
                .selectable_value(&mut self.size_basis, SizeBasis::Current, "Current")
                .changed();
        });
        ui.small("Generic envelope. Spread is the outer canopy footprint; no species or growth prediction.");
        ui.label("Position / elevation (mm)");
        ui.horizontal(|ui| {
            for (label, value) in ["X", "Y", "Z"].into_iter().zip(&mut self.position) {
                ui.label(label);
                if ui.add(egui::DragValue::new(value).speed(10.)).changed() {
                    changed = true;
                    self.placed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Rotation °");
            changed |= ui
                .add(egui::DragValue::new(&mut self.rotation_deg).speed(1.))
                .changed();
        });
        if ui.selectable_label(self.placing, "Place in view").clicked() {
            self.placing = !self.placing;
            changed = true;
        }
        ui.small(if self.placing {
            "Click the garden plane to position the canopy center."
        } else {
            "Use Place in view to reposition graphically."
        });
        changed |= ui
            .checkbox(&mut self.plan_visible, "Show footprint in planting plans")
            .changed();
        ui.label("Optional plan label");
        changed |= ui.text_edit_singleline(&mut self.label).changed();
        changed
    }
    pub fn viewport(&mut self, view: TerminationViewport<'_>) -> bool {
        let frame = multicad_core::sketch::Frame {
            origin: [0., 0., self.position[2]],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            n: [0., 0., 1.],
        };
        let hit = view
            .pointer
            .and_then(|p| view.camera.on_plane(p, view.rect, frame));
        let center = if self.placing {
            hit.unwrap_or([self.position[0], self.position[1]])
        } else {
            [self.position[0], self.position[1]]
        };
        let points = (0..=64)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 64.;
                let p = view.camera.project(
                    [
                        (center[0] + self.spread / 2. * a.cos()) as f32,
                        (center[1] + self.spread / 2. * a.sin()) as f32,
                        self.position[2] as f32,
                    ],
                    view.rect,
                );
                egui::pos2(p[0], p[1])
            })
            .collect();
        view.painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(2., egui::Color32::from_rgb(95, 185, 110)),
        ));
        if self.placing
            && view.clicked
            && let Some(p) = hit
        {
            self.position[0] = p[0];
            self.position[1] = p[1];
            self.placed = true;
            self.placing = false;
            return true;
        }
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::commands::Channel;
    #[test]
    fn viewport_click_configures_a_cancellable_native_plant_then_reopens_and_edits_it() {
        let mut session = Session::new(Document::new("Garden UI").unwrap()).unwrap();
        let before = session.document().clone();
        let mut panel = PlantPanel::new(session.document()).unwrap();
        assert!(panel.request(session.document()).is_err());
        let context = egui::Context::default();
        context
            .run_ui(Default::default(), |ui| {
                let painter = ui.painter().clone();
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640., 480.));
                let view = TerminationViewport {
                    scene: None,
                    report: None,
                    camera: crate::camera::Camera {
                        yaw: 0.,
                        pitch: std::f32::consts::FRAC_PI_2,
                        scale: 0.05,
                        target: [1000., 2000., 0.],
                        perspective: false,
                    },
                    rect,
                    pointer: Some(rect.center()),
                    clicked: true,
                    painter: &painter,
                    section: None,
                };
                assert!(panel.viewport(view));
            })
            .drop_without_applying_deltas();
        assert_eq!(panel.position, [1000., 2000., 0.]);
        assert_eq!(session.document(), &before);
        let request = panel.request(session.document()).unwrap();
        let PlantCommand::Place(ref created) = request else {
            panic!()
        };
        let record = created.placement.record;
        let instance = created.placement.instance;
        let first = request
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        multicad_model::Compiler::default()
            .compile(first.prepared.document(), session.revision())
            .unwrap();
        assert_eq!(session.document(), &before);
        // Dropping the candidate is Cancel; repeating Preview retains the output IDs.
        drop(first);
        let prepared = request
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        assert_eq!(session.revision(), 1);
        let facts = crate::architecture_view::Levels::new(session.document(), session.revision());
        assert_eq!(facts.level_for(instance), Some(None));
        assert!(
            facts
                .rows
                .iter()
                .find(|row| row.id.is_none())
                .unwrap()
                .members
                .iter()
                .any(|member| member.id == instance && member.kind == "Plant")
        );
        let mut edit = PlantPanel::existing(session.document(), record).unwrap();
        assert_eq!(edit.position, [1000., 2000., 0.]);
        edit.spread = 4500.;
        edit.position = [3000., 4000., 200.];
        edit.rotation_deg = 90.;
        edit.label = "T1".into();
        let prepared = edit
            .request(session.document())
            .unwrap()
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        multicad_model::Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        let resolved = plants::resolve(session.document(), session.parameters(), record)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.instance, instance);
        assert_eq!(resolved.spread_mm, 4500.);
        assert_eq!(resolved.label.as_deref(), Some("T1"));
        assert_eq!(
            model::resolve_instance_pose(session.document(), instance, session.parameters())
                .unwrap()
                .translation_mm,
            [3000., 4000., 200.]
        );
    }
}
