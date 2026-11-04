//! Graphical equipment placement shares the canonical hosted source API.
use super::TerminationViewport;
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityData, EntityId},
    equipment::{
        self, Category, CeilingDatum, HostPlacement, Installation, Kind, MountClass, WallSide,
    },
    model::{self, ModelPrepared, angle, length},
    symbols,
};
use std::cell::RefCell;

#[derive(Clone, Debug)]
pub enum EquipmentCommand {
    Place(Box<equipment::EquipmentPlaceRequest>),
    Edit(Box<equipment::EquipmentEditRequest>),
}
impl EquipmentCommand {
    pub fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let prepared = match self {
            Self::Place(r) => equipment::prepare_place(session, session.revision(), actor, r),
            Self::Edit(r) => equipment::prepare_edit(session, session.revision(), actor, r),
        }
        .map_err(|e| e.to_string())?;
        Ok(ModelPrepared {
            prepared,
            outputs: vec![],
        })
    }
}
pub(super) struct EquipmentPanel {
    kind: Kind,
    category: Category,
    search: String,
    existing: Option<EntityId>,
    name: String,
    size: [f64; 3],
    floor: Option<EntityId>,
    wall: Option<EntityId>,
    segment: usize,
    side: WallSide,
    xy: [f64; 2],
    height: f64,
    rotation: f64,
    along: f64,
    standoff: f64,
    ceiling_auto: bool,
    ceiling: f64,
    installation: Installation,
    label: String,
    service: f64,
    door: f64,
    service_visible: bool,
    door_visible: bool,
    placing: bool,
    placed: bool,
    snap: bool,
    repeat: bool,
    error: Option<String>,
    seed: RefCell<Option<equipment::EquipmentPlaceRequest>>,
}
impl EquipmentPanel {
    pub(super) fn existing(document: &Document, record: EntityId) -> Result<Self, String> {
        let mut panel = Self::new(document);
        panel.existing = Some(record);
        panel.load(document, record)?;
        Ok(panel)
    }
    pub(super) fn seed_new_level(&mut self, document: &Document, level: Option<EntityId>) {
        if self.existing.is_some() {
            return;
        }
        self.floor = level;
        self.wall = model::walls(document)
            .find(|wall| model::read_wall(document, wall.id).is_ok_and(|wall| wall.level == level))
            .map(|wall| wall.id);
        *self.seed.borrow_mut() = None;
    }
    pub fn new(document: &Document) -> Self {
        let kind = Kind::PowerOutletTypeF;
        let p = kind.preset();
        Self {
            kind,
            category: p.category,
            search: String::new(),
            existing: None,
            name: p.name.into(),
            size: p.size_mm,
            floor: model::levels(document).next().map(|e| e.id),
            wall: model::walls(document).next().map(|e| e.id),
            segment: 0,
            side: WallSide::Left,
            xy: [0.; 2],
            height: p.mounting_height_mm,
            rotation: 0.,
            along: 0.,
            standoff: 0.,
            ceiling_auto: true,
            ceiling: 2500.,
            installation: Installation::Standalone,
            label: String::new(),
            service: 0.,
            door: 0.,
            service_visible: false,
            door_visible: false,
            placing: true,
            placed: false,
            snap: true,
            repeat: false,
            error: None,
            seed: RefCell::new(None),
        }
    }
    pub(super) fn select(&mut self, kind: Kind) {
        self.kind = kind;
        let p = kind.preset();
        self.category = p.category;
        self.name = p.name.into();
        self.size = p.size_mm;
        self.height = p.mounting_height_mm;
        self.installation = if p.supports_standalone {
            Installation::Standalone
        } else {
            Installation::BuiltIn
        };
        self.service = p.service_front_mm;
        self.door = p.door_front_mm;
        self.placed = false;
        self.placing = true;
        *self.seed.borrow_mut() = None;
    }
    pub fn repeat_after_apply(&mut self, document: &Document) -> bool {
        if !self.repeat || self.existing.is_some() {
            return false;
        }
        *self.seed.borrow_mut() = None;
        self.placed = false;
        self.placing = true;
        self.label.clear();
        self.error = None;
        self.name =
            super::names::GeneratedNames::new(document).numbered(self.kind.preset().name, "");
        true
    }
    fn host(&self, document: &Document) -> Result<HostPlacement, String> {
        Ok(match self.kind.preset().mount {
            MountClass::WallCenter | MountClass::WallBottom => {
                let wall = self.wall.ok_or("Choose a wall host")?;
                let source = model::read_wall(document, wall).map_err(|e| e.to_string())?;
                HostPlacement::Wall {
                    wall,
                    segment: self.segment,
                    point_count: source.path.len(),
                    closed: source.closed,
                    side: self.side,
                    along: length(self.along),
                    standoff: length(self.standoff),
                    height: length(self.height),
                    rotation: angle(self.rotation),
                }
            }
            MountClass::CeilingTop => HostPlacement::Ceiling {
                xy: self.xy.map(length),
                datum: if self.ceiling_auto {
                    CeilingDatum::LevelUnderside
                } else {
                    CeilingDatum::AboveFloor {
                        height: length(self.ceiling),
                    }
                },
                drop: length(self.height),
                rotation: angle(self.rotation),
            },
            _ => HostPlacement::Floor {
                xy: self.xy.map(length),
                height: length(self.height),
                rotation: angle(self.rotation),
            },
        })
    }
    fn load(&mut self, document: &Document, record: EntityId) -> Result<(), String> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
        let value = equipment::resolve(document, &parameters, record)
            .map_err(|e| e.to_string())?
            .ok_or("Choose equipment")?;
        self.kind = value.template.kind;
        self.category = self.kind.preset().category;
        self.size = value.size_mm;
        self.name = document
            .get(value.instance)
            .ok_or("Equipment is missing")?
            .name
            .clone();
        self.installation = value.settings.installation;
        self.label = value.settings.label.unwrap_or_default();
        self.service_visible = value.settings.service_visible;
        self.door_visible = value.settings.door_visible;
        for (id, field) in [
            (value.settings.service_envelope, &mut self.service),
            (value.settings.door_envelope, &mut self.door),
        ] {
            *field = id
                .map(|id| {
                    model::read_envelope(document, id).and_then(|e| {
                        e.front
                            .value_in(
                                multicad_core::parameters::Unit::Millimeter,
                                Some(&parameters),
                            )
                            .map_err(multicad_core::document::DocumentError::Invalid)
                    })
                })
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or(0.);
        }
        // Read intent before resolving hosts, so broken objects remain repairable.
        let (_, attachment) = value.attachment.ok_or("Use a hosted equipment instance")?;
        self.floor = Some(attachment.floor);
        let mm = |d: multicad_core::constraint_sketch::Dimension| {
            d.value_in(
                multicad_core::parameters::Unit::Millimeter,
                Some(&parameters),
            )
        };
        let deg = |d: multicad_core::constraint_sketch::Dimension| {
            d.value_in(multicad_core::parameters::Unit::Degree, Some(&parameters))
        };
        match attachment.placement.clone() {
            HostPlacement::Floor {
                xy,
                height,
                rotation,
            } => {
                self.xy = [mm(xy[0].clone())?, mm(xy[1].clone())?];
                self.height = mm(height)?;
                self.rotation = deg(rotation)?;
            }
            HostPlacement::Ceiling {
                xy,
                datum,
                drop,
                rotation,
            } => {
                self.xy = [mm(xy[0].clone())?, mm(xy[1].clone())?];
                self.height = mm(drop)?;
                self.rotation = deg(rotation)?;
                self.ceiling_auto = matches!(datum, CeilingDatum::LevelUnderside);
                if let CeilingDatum::AboveFloor { height } = datum {
                    self.ceiling = mm(height)?;
                }
            }
            HostPlacement::Wall {
                wall,
                segment,
                side,
                along,
                standoff,
                height,
                rotation,
                ..
            } => {
                self.wall = Some(wall);
                self.segment = segment;
                self.side = side;
                self.along = mm(along)?;
                self.standoff = mm(standoff)?;
                self.height = mm(height)?;
                self.rotation = deg(rotation)?;
            }
        }
        self.placed = true;
        self.placing = false;
        attachment
            .resolve(document, &parameters)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn request(&self, document: &Document) -> Result<EquipmentCommand, String> {
        let floor = self.floor.ok_or(
            "Create a Level in Model / Architecture, then select it as the equipment Floor datum before placing equipment.",
        )?;
        let parameters = multicad_core::parameters::evaluate(document).map_err(|error| {
            format!(
                "Cannot resolve the equipment Floor datum. Repair the project parameters: {error}"
            )
        })?;
        model::resolve_level(document, &parameters, floor).map_err(|error| {
            format!(
                "The selected equipment Floor datum is missing or invalid. Select a valid Level in Floor datum before placing equipment: {error}"
            )
        })?;
        if !self.placed {
            return Err("Click in the apartment view to position the equipment".into());
        }
        let session = Session::new(document.clone()).map_err(|e| e.to_string())?;
        let host = self.host(document)?;
        let (record, owner, mut settings, attachment, mut envelopes) =
            if let Some(record) = self.existing {
                let old = equipment::resolve(document, session.parameters(), record)
                    .map_err(|e| e.to_string())?
                    .ok_or("Choose equipment")?;
                let owner = old.instance;
                let envelopes = [old.settings.service_envelope, old.settings.door_envelope]
                    .into_iter()
                    .flatten()
                    .map(|id| {
                        model::read_envelope(document, id)
                            .map(|value| equipment::EnvelopeSource { id, value })
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                (
                    record,
                    owner,
                    old.settings,
                    equipment::Attachment {
                        owner,
                        floor,
                        placement: host.clone(),
                    },
                    envelopes,
                )
            } else {
                if self.seed.borrow().is_none() {
                    *self.seed.borrow_mut() = Some(
                        equipment::EquipmentPlaceRequest::new(
                            &session,
                            equipment::generic_definition(self.kind).map_err(|e| e.to_string())?,
                            self.name.clone(),
                            self.size,
                            floor,
                            host.clone(),
                        )
                        .map_err(|e| e.to_string())?,
                    );
                }
                let seed = self.seed.borrow();
                let seed = seed.as_ref().expect("seeded request");
                (
                    seed.placement.record,
                    seed.placement.instance,
                    seed.settings.clone(),
                    equipment::Attachment {
                        owner: seed.placement.instance,
                        floor,
                        placement: host,
                    },
                    seed.envelopes.clone(),
                )
            };
        settings.installation = self.installation;
        settings.label = (!self.label.trim().is_empty()).then(|| self.label.clone());
        settings.service_visible = self.service_visible;
        settings.door_visible = self.door_visible;
        for (distance, label, slot) in [
            (
                self.service,
                "Generic service access",
                &mut settings.service_envelope,
            ),
            (
                self.door,
                "Generic door access envelope",
                &mut settings.door_envelope,
            ),
        ] {
            if let Some(id) = *slot {
                if let Some(source) = envelopes.iter_mut().find(|s| s.id == id) {
                    source.value.front = length(distance);
                }
            } else if distance > 0. {
                let source = equipment::EnvelopeSource::front(owner, distance, label);
                *slot = Some(source.id);
                envelopes.push(source);
            }
        }
        if self.existing.is_some() {
            Ok(EquipmentCommand::Edit(Box::new(
                equipment::EquipmentEditRequest {
                    record,
                    name: self.name.clone(),
                    size_mm: self.size,
                    attachment,
                    settings,
                    envelopes,
                },
            )))
        } else {
            let mut request = self.seed.borrow().as_ref().expect("seed").clone();
            let template =
                equipment::template_for(&request.definition.template, request.definition.root)
                    .map_err(|e| e.to_string())?
                    .expect("equipment");
            request.placement.name = self.name.clone();
            for (id, value) in [
                template.width_parameter,
                template.depth_parameter,
                template.height_parameter,
            ]
            .into_iter()
            .zip(self.size)
            {
                request.placement.overrides.insert(
                    id,
                    symbols::SymbolValue {
                        value,
                        unit: "mm".into(),
                    },
                );
            }
            request.attachment = attachment;
            request.settings = settings;
            request.envelopes = envelopes;
            Ok(EquipmentCommand::Place(Box::new(request)))
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        let mut instances = vec![];
        if let Ok(parameters) = multicad_core::parameters::evaluate(document) {
            for e in document.entities() {
                if matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == symbols::INSTANCE_TYPE)
                    && let Ok(Some(value)) = equipment::resolve(document, &parameters, e.id)
                {
                    instances.push((
                        e.id,
                        document.get(value.instance).expect("instance").name.clone(),
                    ));
                }
            }
        }
        let before = self.existing;
        controls::inspector_combo(ui, "equipment-existing")
            .selected_text(
                instances
                    .iter()
                    .find(|(id, _)| Some(*id) == self.existing)
                    .map_or("New equipment", |(_, n)| n.as_str()),
            )
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut self.existing, None, "New equipment")
                    .changed();
                for (id, name) in instances {
                    changed |= ui
                        .selectable_value(&mut self.existing, Some(id), name)
                        .changed();
                }
            });
        if before != self.existing {
            if let Some(record) = self.existing {
                self.error = self.load(document, record).err();
            } else {
                self.select(self.kind);
            }
        }
        if self.existing.is_none() {
            controls::inspector_combo(ui, "equipment-category")
                .selected_text(self.category.label())
                .show_ui(ui, |ui| {
                    for category in Category::ALL {
                        ui.selectable_value(&mut self.category, category, category.label());
                    }
                });
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search equipment"));
            let query = self.search.to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("equipment-catalog")
                .max_height(150.)
                .show(ui, |ui| {
                    for kind in Kind::ALL {
                        let p = kind.preset();
                        if (query.is_empty() && p.category == self.category
                            || !query.is_empty()
                                && format!("{} {}", p.name.to_lowercase(), kind.search_terms())
                                    .contains(&query))
                            && ui.selectable_label(kind == self.kind, p.name).clicked()
                        {
                            self.select(kind);
                            changed = true;
                        }
                    }
                });
        }
        ui.small("Generic editable planning dimensions. ToskCAD generic v1 plan symbols.");
        if self.kind.preset().power_connector.is_some() {
            ui.small("Power connector: Type F (Schuko). Planning envelope only.");
        }
        ui.label("Name");
        changed |= ui.text_edit_singleline(&mut self.name).changed();
        for (label, value) in ["Width (mm)", "Depth (mm)", "Height (mm)"]
            .into_iter()
            .zip(&mut self.size)
        {
            changed |= number(ui, label, value, 1. ..=100_000.);
        }
        let floors: Vec<_> = model::levels(document)
            .map(|e| (e.id, e.name.clone()))
            .collect();
        ui.label("Floor datum");
        changed |= picker(ui, "equipment-floor", &mut self.floor, &floors);
        if matches!(
            self.kind.preset().mount,
            MountClass::WallCenter | MountClass::WallBottom
        ) {
            ui.label("Wall host / repair");
            let walls: Vec<_> = model::walls(document)
                .map(|e| (e.id, e.name.clone()))
                .collect();
            if picker(ui, "equipment-wall", &mut self.wall, &walls) {
                self.segment = 0;
                changed = true;
            }
            ui.horizontal(|ui| {
                changed |= ui
                    .selectable_value(&mut self.side, WallSide::Left, "Left face")
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.side, WallSide::Right, "Right face")
                    .changed();
            });
            changed |= number(ui, "Along wall (mm)", &mut self.along, 0. ..=1e9);
            changed |= number(ui, "From wall face (mm)", &mut self.standoff, 0. ..=1e6);
            ui.small("Click a wall segment in the view to choose its attachment position.");
        }
        if self.kind.preset().mount == MountClass::CeilingTop {
            changed |= ui
                .checkbox(&mut self.ceiling_auto, "Ceiling = next slab underside")
                .changed();
            if !self.ceiling_auto {
                changed |= number(
                    ui,
                    "Ceiling above floor (mm)",
                    &mut self.ceiling,
                    1. ..=100_000.,
                );
            }
            changed |= number(
                ui,
                "Drop below ceiling (mm)",
                &mut self.height,
                0. ..=100_000.,
            );
        } else {
            let label = match self.kind.preset().mount {
                MountClass::WallCenter => "Center above floor (mm)",
                MountClass::CounterTop => "Top above floor (mm)",
                _ => "Bottom above floor (mm)",
            };
            changed |= number(ui, label, &mut self.height, -100_000. ..=100_000.);
        }
        changed |= number(ui, "Rotation (°)", &mut self.rotation, -3600. ..=3600.);
        let p = self.kind.preset();
        if p.supports_built_in && p.supports_standalone {
            ui.horizontal(|ui| {
                changed |= ui
                    .selectable_value(
                        &mut self.installation,
                        Installation::Standalone,
                        "Standalone",
                    )
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.installation, Installation::BuiltIn, "Built in")
                    .changed();
            });
        } else {
            ui.small(if p.supports_built_in {
                "Built-in type"
            } else {
                "Standalone type"
            });
        }
        if ui.selectable_label(self.placing, "Place in view").clicked() {
            self.placing = !self.placing;
            changed = true;
        }
        ui.checkbox(&mut self.snap, "Snap to 100 mm grid / wall");
        if self.existing.is_none() {
            ui.checkbox(&mut self.repeat, "Keep placing this type").on_hover_text("After Apply, click to place another occurrence. Each placement remains one undoable edit.");
        }
        ui.label("Optional plan label");
        changed |= ui.text_edit_singleline(&mut self.label).changed();
        if p.service_front_mm > 0. || p.door_front_mm > 0. {
            ui.collapsing("Access and service envelopes", |ui| {
                changed |= number(ui, "Service front (mm)", &mut self.service, 0. ..=100_000.);
                changed |= ui.checkbox(&mut self.service_visible, "Show service envelope").changed();
                changed |= number(ui, "Door access front (mm)", &mut self.door, 0. ..=100_000.);
                changed |= ui.checkbox(&mut self.door_visible, "Show door access envelope").changed();
                ui.small("Conservative access boxes, separate from physical geometry; included in clearance checks.");
            });
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        changed
    }
    pub fn viewport(&mut self, document: &Document, view: TerminationViewport<'_>) -> bool {
        let Ok(parameters) = multicad_core::parameters::evaluate(document) else {
            return false;
        };
        let floor_z = self
            .floor
            .and_then(|id| model::resolve_level(document, &parameters, id).ok())
            .map_or(0., |l| l.elevation_mm);
        let frame = multicad_core::sketch::Frame {
            origin: [0., 0., floor_z],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            n: [0., 0., 1.],
        };
        let hit = view
            .pointer
            .and_then(|p| view.camera.on_plane(p, view.rect, frame));
        let mut host = self.host(document).ok();
        if self.placing
            && let Some(mut point) = hit
        {
            if self.snap {
                point = point.map(|v| (v / 100.).round() * 100.);
            }
            match &mut host {
                Some(HostPlacement::Floor { xy, .. } | HostPlacement::Ceiling { xy, .. }) => {
                    *xy = point.map(length)
                }
                Some(HostPlacement::Wall { segment, along, .. }) => {
                    if let Some(wall) = self
                        .wall
                        .and_then(|id| model::read_wall(document, id).ok())
                        .and_then(|s| model::resolve_wall(document, &parameters, &s).ok())
                    {
                        let count = if wall.closed {
                            wall.points.len()
                        } else {
                            wall.points.len() - 1
                        };
                        let closest = (0..count)
                            .map(|i| {
                                let a = wall.points[i];
                                let b = wall.points[(i + 1) % wall.points.len()];
                                let d = [b[0] - a[0], b[1] - a[1]];
                                let len = d[0].hypot(d[1]);
                                let t = ((point[0] - a[0]) * d[0] + (point[1] - a[1]) * d[1]) / len;
                                let t = t.clamp(0., len);
                                let distance = (point[0] - a[0] - d[0] * t / len)
                                    .hypot(point[1] - a[1] - d[1] * t / len);
                                (i, t, distance, len)
                            })
                            .min_by(|a, b| a.2.total_cmp(&b.2));
                        if let Some((i, mut offset, _, len)) = closest {
                            if self.snap {
                                offset = (offset / 100.).round() * 100.;
                            }
                            *segment = i;
                            *along = length(offset.min(len));
                        }
                    }
                }
                _ => {}
            }
        }
        let Some(host) = host else { return false };
        let attachment = equipment::Attachment {
            owner: EntityId::new(),
            floor: self.floor.unwrap_or_default(),
            placement: host.clone(),
        };
        let Ok(resolved) = attachment.resolve(document, &parameters) else {
            return false;
        };
        let wall_mounted = matches!(
            self.kind.preset().mount,
            MountClass::WallCenter | MountClass::WallBottom
        );
        let y = if wall_mounted {
            [-self.size[1], 0.]
        } else {
            [-self.size[1] / 2., self.size[1] / 2.]
        };
        let project = |p: [f64; 3]| {
            let world = resolved.pose.point(p);
            let p = view.camera.project(world.map(|v| v as f32), view.rect);
            egui::pos2(p[0], p[1])
        };
        let points = [
            [-self.size[0] / 2., y[0], 0.],
            [self.size[0] / 2., y[0], 0.],
            [self.size[0] / 2., y[1], 0.],
            [-self.size[0] / 2., y[1], 0.],
            [-self.size[0] / 2., y[0], 0.],
        ]
        .map(project)
        .to_vec();
        let stroke = egui::Stroke::new(2., egui::Color32::from_rgb(70, 190, 170));
        view.painter.add(egui::Shape::line(points, stroke));
        view.painter.arrow(
            project([0., 0., 0.]),
            project([0., y[0] - 200., 0.]) - project([0., 0., 0.]),
            stroke,
        );
        if self.placing && view.clicked && hit.is_some() {
            match host {
                HostPlacement::Floor { xy, .. } | HostPlacement::Ceiling { xy, .. } => {
                    self.xy = xy.map(|d| {
                        d.value_in(
                            multicad_core::parameters::Unit::Millimeter,
                            Some(&parameters),
                        )
                        .expect("literal")
                    })
                }
                HostPlacement::Wall { segment, along, .. } => {
                    self.segment = segment;
                    self.along = along
                        .value_in(
                            multicad_core::parameters::Unit::Millimeter,
                            Some(&parameters),
                        )
                        .expect("literal");
                }
            }
            self.placed = true;
            self.placing = false;
            self.error = None;
            return true;
        }
        false
    }
}
fn number(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).range(range).speed(10.))
            .changed()
    })
    .inner
}
fn picker(
    ui: &mut egui::Ui,
    salt: &str,
    value: &mut Option<EntityId>,
    choices: &[(EntityId, String)],
) -> bool {
    let mut changed = false;
    controls::inspector_combo(ui, salt)
        .selected_text(
            choices
                .iter()
                .find(|(id, _)| Some(*id) == *value)
                .map_or("Choose / repair host", |(_, n)| n.as_str()),
        )
        .show_ui(ui, |ui| {
            for (id, name) in choices {
                changed |= ui.selectable_value(value, Some(*id), name).changed();
            }
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{commands::Channel, model::LevelDefinition};
    #[test]
    fn missing_or_invalid_floor_reports_the_repair_before_requesting_a_viewport_pick() {
        use multicad_core::document::{Definition, Entity, Mutation};
        let document = Document::new("Missing equipment floor").unwrap();
        let mut panel = EquipmentPanel::new(&document);
        let error = panel.request(&document).unwrap_err();
        assert!(error.contains("Create a Level in Model / Architecture"));
        assert!(error.contains("Floor datum"));
        assert!(!error.contains("Click in the apartment view"));
        panel.floor = Some(EntityId::new());
        let error = panel.request(&document).unwrap_err();
        assert!(error.contains("missing or invalid"));
        assert!(error.contains("Select a valid Level"));
        let level = Entity::new(
            "Invalid floor",
            EntityData::Extension {
                definition: Definition {
                    type_id: model::LEVEL_KIND.into(),
                    schema_version: 1,
                    data: serde_json::to_value(LevelDefinition {
                        elevation: length(0.),
                        height: length(0.),
                        slab_thickness: length(200.),
                    })
                    .unwrap(),
                },
                inputs: vec![],
            },
        );
        panel.floor = Some(level.id);
        let invalid = document.edited(&[Mutation::Insert(level)]).unwrap();
        let before = invalid.clone();
        let error = panel.request(&invalid).unwrap_err();
        assert!(error.contains("Select a valid Level"));
        assert!(error.contains("Level height and slab thickness must be positive"));
        assert_eq!(invalid, before);
        assert!(!panel.placed);
        assert!(panel.seed.borrow().is_none());
    }

    #[test]
    fn a_valid_explicit_floor_keeps_pick_feedback_and_drives_the_prepared_attachment() {
        let mut session = Session::new(Document::new("Equipment floor choice").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let mut floors = Vec::new();
        for elevation in [100., 500.] {
            let (id, prepared) = model::prepare_level(
                &session,
                session.revision(),
                actor.clone(),
                "Floor",
                &LevelDefinition {
                    elevation: length(elevation),
                    height: length(2800.),
                    slab_thickness: length(200.),
                },
            )
            .unwrap();
            session.commit(prepared).unwrap();
            floors.push(id);
        }
        let before = session.document().clone();
        let revision = session.revision();
        let mut panel = EquipmentPanel::new(session.document());
        panel.select(Kind::WashingMachine);
        panel.floor = Some(floors[1]);
        assert_eq!(
            panel.request(session.document()).unwrap_err(),
            "Click in the apartment view to position the equipment"
        );
        let camera = crate::camera::Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            scale: 0.5,
            target: [1000., 2000., 500.],
            perspective: false,
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 600.));
        let point = camera.project([1000., 2000., 500.], rect);
        let mut picked = false;
        egui::Context::default()
            .run_ui(Default::default(), |ui| {
                picked = panel.viewport(
                    session.document(),
                    TerminationViewport {
                        scene: None,
                        report: None,
                        camera,
                        rect,
                        pointer: Some(egui::pos2(point[0], point[1])),
                        clicked: true,
                        painter: ui.painter(),
                        section: None,
                    },
                );
            })
            .drop_without_applying_deltas();
        assert!(picked);
        let command = panel.request(session.document()).unwrap();
        let EquipmentCommand::Place(request) = &command else {
            panic!("New equipment must prepare placement");
        };
        assert_eq!(request.attachment.floor, floors[1]);
        let resolved = request
            .attachment
            .resolve(session.document(), session.parameters())
            .unwrap();
        assert_eq!(resolved.pose.translation_mm, [1000., 2000., 500.]);
        let prepared = command.prepare(&session, actor).unwrap();
        session.validate_preview(&prepared.prepared).unwrap();
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
        assert!(panel.placed);
    }

    #[test]
    fn inspector_reuses_preview_identity_and_restores_hosted_equipment_for_editing() {
        let mut s = Session::new(Document::new("Inspector equipment").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let (floor, p) = model::prepare_level(
            &s,
            0,
            actor.clone(),
            "Finished floor",
            &LevelDefinition {
                elevation: length(100.),
                height: length(2800.),
                slab_thickness: length(200.),
            },
        )
        .unwrap();
        s.commit(p).unwrap();
        let mut panel = EquipmentPanel::new(s.document());
        panel.select(Kind::WashingMachine);
        panel.floor = Some(floor);
        panel.xy = [1000., 2000.];
        panel.placed = true;
        let first = panel.request(s.document()).unwrap();
        let second = panel.request(s.document()).unwrap();
        let EquipmentCommand::Place(request) = &first else {
            panic!()
        };
        let EquipmentCommand::Place(repeated) = second else {
            panic!()
        };
        assert_eq!(request.placement.instance, repeated.placement.instance);
        assert_eq!(
            request.settings.service_envelope,
            repeated.settings.service_envelope
        );
        let before = s.document().clone();
        let prepared = first.prepare(&s, actor.clone()).unwrap();
        assert_eq!(
            s.document(),
            &before,
            "Preview/Cancel leaves source untouched"
        );
        s.commit(prepared.prepared).unwrap();
        panel.repeat = true;
        assert!(panel.repeat_after_apply(s.document()));
        assert!(
            panel.request(s.document()).is_err(),
            "Repeated placement must wait for its next viewport pick"
        );
        panel.xy = [2300., 2000.];
        panel.placed = true;
        let EquipmentCommand::Place(next) = panel.request(s.document()).unwrap() else {
            panic!()
        };
        assert_ne!(next.placement.instance, request.placement.instance);
        assert_ne!(
            next.settings.service_envelope,
            request.settings.service_envelope
        );
        assert!(
            EquipmentCommand::Place(next)
                .prepare(&s, actor.clone())
                .is_ok()
        );
        let mut edit = EquipmentPanel::new(s.document());
        edit.existing = Some(request.placement.record);
        edit.load(s.document(), request.placement.record).unwrap();
        assert_eq!(edit.size, [600., 600., 850.]);
        assert_eq!(edit.xy, [1000., 2000.]);
        assert_eq!(edit.service, 600.);
        assert_eq!(edit.door, 450.);
        edit.size = [650., 620., 900.];
        edit.label = "Laundry".into();
        edit.service_visible = true;
        let candidate = edit
            .request(s.document())
            .unwrap()
            .prepare(&s, actor.clone())
            .unwrap();
        s.commit(candidate.prepared).unwrap();
        let resolved = equipment::resolve(s.document(), s.parameters(), request.placement.record)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.size_mm, [650., 620., 900.]);
        assert_eq!(resolved.settings.label.as_deref(), Some("Laundry"));
        assert_eq!(
            resolved.settings.service_envelope,
            request.settings.service_envelope
        );
        s.undo(s.revision(), actor).unwrap();
        assert_eq!(
            equipment::resolve(s.document(), s.parameters(), request.placement.record)
                .unwrap()
                .unwrap()
                .size_mm,
            [600., 600., 850.]
        );
    }
}
