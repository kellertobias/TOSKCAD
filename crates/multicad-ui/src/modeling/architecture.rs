//! Level inspector: define or edit levels, service envelopes and drawing
//! guides, and place slabs, pedestals and stairs on a level. Slabs and stairs
//! are features with one output Part; the others are reusable source entities.
use super::{
    FormStamp, PreviewState, TerminationViewport,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
    guide_fields::GuideFields,
    outline::{OutlineEditor, OutlineKind, plan_targets},
};
use crate::{controls, egui};
mod stair_placement;
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId},
    drawings::{self, GuideDefinition},
    model::{
        self, BodyOutput, EnvelopeDefinition, LevelDefinition, ModelPrepared, SlabDefinition,
        SlabKind, StairDefinition, StairForm, Turn, length,
    },
};

/// What the panel creates or edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchitectureKind {
    Level,
    Slab,
    Pedestal,
    Stair,
    Envelope,
    Guide,
    Plant,
    Equipment,
}
impl ArchitectureKind {
    const ALL: [Self; 8] = [
        Self::Level,
        Self::Slab,
        Self::Pedestal,
        Self::Stair,
        Self::Envelope,
        Self::Guide,
        Self::Plant,
        Self::Equipment,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Level => "Level",
            Self::Slab => "Floor slab",
            Self::Pedestal => "Pedestal",
            Self::Stair => "Stair",
            Self::Envelope => "Service envelope",
            Self::Guide => "Drawing guide",
            Self::Plant => "Planting",
            Self::Equipment => "Equipment",
        }
    }
}

/// Which line of a closed room a floor outline follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RoomBoundary {
    /// The wall faces inside the room.
    Inside,
    /// The drawn wall path, keeping its parameter expressions.
    Path,
    /// The outer wall faces.
    Outside,
}
impl RoomBoundary {
    const ALL: [Self; 3] = [Self::Inside, Self::Path, Self::Outside];
    fn label(self) -> &'static str {
        match self {
            Self::Inside => "Inside faces",
            Self::Path => "Wall path",
            Self::Outside => "Outside faces",
        }
    }
}

/// The closed walls a floor outline can follow, by feature and name.
pub(super) fn rooms(document: &Document) -> Vec<(EntityId, String)> {
    model::walls(document)
        .filter(|e| model::read_wall(document, e.id).is_ok_and(|w| w.closed))
        .map(|e| (e.id, e.name.clone()))
        .collect()
}

/// The explicit boundary of exactly one closed room. Path keeps the authored
/// dimensions; face boundaries are the planned miter corners in millimeters.
pub(super) fn room_boundary(
    document: &Document,
    wall: EntityId,
    boundary: RoomBoundary,
) -> Result<Vec<[Dimension; 2]>, String> {
    let source = model::read_wall(document, wall).map_err(|e| e.to_string())?;
    if !source.closed {
        return Err("Choose a wall that is closed into a room".into());
    }
    if boundary == RoomBoundary::Path {
        return Ok(source.path);
    }
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
    let plan = model::resolve_wall(document, &parameters, &source)
        .and_then(|r| r.plan())
        .map_err(|e| e.to_string())?;
    let points = match boundary {
        RoomBoundary::Outside => plan.outer.points,
        _ => plan
            .holes
            .first()
            .map(|h| h.points.clone())
            .ok_or("The room has no inside boundary")?,
    };
    Ok(points
        .iter()
        .map(|p| [length(p[0]), length(p[1])])
        .collect())
}

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub enum ArchitectureCommand {
    Plant(super::plant::PlantCommand),
    Equipment(super::equipment::EquipmentCommand),
    Level {
        existing: Option<EntityId>,
        name: String,
        value: LevelDefinition,
    },
    Envelope {
        existing: Option<EntityId>,
        name: String,
        value: EnvelopeDefinition,
    },
    Slab {
        edit: Option<EntityId>,
        name: String,
        slab: SlabDefinition,
    },
    Stair {
        edit: Option<EntityId>,
        name: String,
        stair: StairDefinition,
    },
    Guide {
        existing: Option<EntityId>,
        name: String,
        value: GuideDefinition,
        generate: bool,
    },
}
impl ArchitectureCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        let source = |prepared| ModelPrepared {
            prepared,
            outputs: vec![],
        };
        match self {
            Self::Plant(command) => return command.prepare(session, actor),
            Self::Equipment(command) => return command.prepare(session, actor),
            Self::Level {
                existing: Some(id),
                value,
                ..
            } => model::prepare_level_edit(session, revision, actor, *id, value).map(source),
            Self::Level {
                existing: None,
                name,
                value,
            } => {
                model::prepare_level(session, revision, actor, name, value).map(|(_, p)| source(p))
            }
            Self::Envelope {
                existing: Some(id),
                value,
                ..
            } => model::prepare_envelope_edit(session, revision, actor, *id, value).map(source),
            Self::Envelope {
                existing: None,
                name,
                value,
            } => model::prepare_envelope(session, revision, actor, name, value)
                .map(|(_, p)| source(p)),
            Self::Slab {
                edit: Some(feature),
                slab,
                ..
            } => model::prepare_slab_edit(session, revision, actor, *feature, slab),
            Self::Slab {
                edit: None,
                name,
                slab,
            } => model::prepare_slab(session, revision, actor, name, slab, None),
            Self::Stair {
                edit: Some(feature),
                stair,
                ..
            } => model::prepare_stair_edit(session, revision, actor, *feature, stair),
            Self::Stair {
                edit: None,
                name,
                stair,
            } => model::prepare_stair(session, revision, actor, name, stair, None),
            Self::Guide {
                existing,
                name,
                value,
                generate,
            } => drawings::prepare_guide(
                session, revision, actor, *existing, name, value, *generate, None,
            )
            .map(|g| source(g.prepared)),
        }
        .map_err(|e| e.to_string())
    }
    /// The feature this command edits. Level and envelope edits change
    /// source entities, not features, so they run as ordinary transactions.
    pub fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::Slab { edit, .. } | Self::Stair { edit, .. } => *edit,
            Self::Level { .. }
            | Self::Envelope { .. }
            | Self::Guide { .. }
            | Self::Plant(_)
            | Self::Equipment(_) => None,
        }
    }
}

pub(super) struct ArchitecturePanel {
    stamp: FormStamp,
    kind: ArchitectureKind,
    plant: super::plant::PlantPanel,
    equipment: super::equipment::EquipmentPanel,
    /// The slab or stair feature being edited; its kind is then fixed.
    feature: Option<EntityId>,
    body: EntityId,
    name: String,
    /// Existing level or envelope to edit, or None for a new one.
    existing: Option<EntityId>,
    elevation: DimensionField,
    height: DimensionField,
    slab_thickness: DimensionField,
    level: Option<EntityId>,
    outline: OutlineEditor,
    /// The one room whose boundary the outline was taken from, if any.
    room: Option<EntityId>,
    room_boundary: RoomBoundary,
    targets: Option<(u64, Vec<[f64; 2]>)>,
    override_thickness: bool,
    thickness: DimensionField,
    /// Vertical-cutout holes of the edited slab, kept as they are; the
    /// vertical-cutout tool edits them.
    holes: Vec<model::SlabHole>,
    to_level: Option<EntityId>,
    form: StairForm,
    start: [f64; 2],
    direction_deg: f64,
    stair_placing: bool,
    stair_anchor: Option<[f64; 2]>,
    stair_snap: bool,
    width: DimensionField,
    risers: u32,
    going: DimensionField,
    target: Option<EntityId>,
    clearances: [DimensionField; 5],
    label: String,
    guide: GuideFields,
    error: Option<String>,
}
fn mm(value: f64) -> Result<DimensionField, String> {
    DimensionField::from_dimension(&length(value), "mm")
}
fn number(d: &Dimension) -> Result<f64, String> {
    match d {
        Dimension::Literal { value, .. } => Ok(*value),
        Dimension::Expression { expression } => Err(format!(
            "Stair start {} is an expression; edit it through the API",
            expression.text
        )),
    }
}
const CLEARANCES: [(&str, &str); 5] = [
    (
        "Front",
        "Clearance in front of the target (its local −Y side)",
    ),
    ("Back", "Clearance behind the target (its local +Y side)"),
    ("Left", "Clearance on the target's local −X side"),
    ("Right", "Clearance on the target's local +X side"),
    ("Top", "Clearance above the target"),
];
impl ArchitecturePanel {
    pub(super) fn level(
        document: &Document,
        name: String,
        existing: Option<EntityId>,
    ) -> Result<Self, String> {
        let mut panel = Self::new(document, name)?;
        panel.kind = ArchitectureKind::Level;
        panel.existing = existing;
        panel.load_level(document)?;
        Ok(panel)
    }
    pub(super) fn equipment_existing(
        document: &Document,
        record: EntityId,
    ) -> Result<Self, String> {
        let mut panel = Self::equipment(document)?;
        panel.equipment = super::equipment::EquipmentPanel::existing(document, record)?;
        Ok(panel)
    }
    pub(super) fn seed_new_level(&mut self, document: &Document, level: Option<EntityId>) {
        if self.feature.is_some() || self.existing.is_some() {
            return;
        }
        self.level = level;
        self.equipment.seed_new_level(document, level);
    }
    pub fn new(document: &Document, name: String) -> Result<Self, String> {
        let level = model::levels(document).next().map(|e| e.id);
        Ok(Self {
            stamp: FormStamp::new(),
            plant: super::plant::PlantPanel::new(document)?,
            equipment: super::equipment::EquipmentPanel::new(document),
            kind: if level.is_some() {
                ArchitectureKind::Slab
            } else {
                ArchitectureKind::Level
            },
            feature: None,
            body: EntityId::new(),
            name,
            existing: None,
            elevation: mm(0.)?,
            height: mm(2750.)?,
            slab_thickness: mm(200.)?,
            level,
            outline: OutlineEditor::from_mm(
                OutlineKind::SlabOutline,
                &[[0., 0.], [6000., 0.], [6000., 4000.], [0., 4000.]],
                true,
            ),
            room: None,
            room_boundary: RoomBoundary::Inside,
            targets: None,
            override_thickness: false,
            thickness: mm(200.)?,
            holes: vec![],
            to_level: None,
            form: StairForm::Straight,
            start: [0., 0.],
            direction_deg: 0.,
            stair_placing: false,
            stair_anchor: None,
            stair_snap: true,
            width: mm(1000.)?,
            risers: 16,
            going: mm(270.)?,
            target: None,
            clearances: [mm(600.)?, mm(0.)?, mm(0.)?, mm(0.)?, mm(0.)?],
            label: String::new(),
            guide: GuideFields::new(document)?,
            error: None,
        })
    }
    /// Reopen an applied slab, pedestal or stair feature for editing.
    pub fn existing(document: &Document, feature: EntityId, name: String) -> Result<Self, String> {
        let mut panel = Self::new(document, name)?;
        panel.feature = Some(feature);
        if let Ok(slab) = model::read_slab(document, feature) {
            panel.kind = match slab.kind {
                SlabKind::Slab => ArchitectureKind::Slab,
                SlabKind::Pedestal => ArchitectureKind::Pedestal,
            };
            panel.body = slab.output.body;
            panel.level = Some(slab.level);
            panel.outline = OutlineEditor::new(OutlineKind::SlabOutline, &slab.outline, true)?;
            panel.override_thickness = slab.thickness.is_some();
            panel.holes = slab.holes.clone();
            if let Some(t) = &slab.thickness {
                panel.thickness = DimensionField::from_dimension(t, "mm")?;
            }
            return Ok(panel);
        }
        let stair = model::read_stair(document, feature).map_err(|e| e.to_string())?;
        panel.kind = ArchitectureKind::Stair;
        panel.body = stair.output.body;
        panel.level = Some(stair.level);
        panel.to_level = stair.to_level;
        panel.form = stair.form;
        panel.start = [number(&stair.start[0])?, number(&stair.start[1])?];
        panel.direction_deg = stair.direction_deg;
        panel.width = DimensionField::from_dimension(&stair.width, "mm")?;
        panel.risers = stair.risers;
        panel.going = DimensionField::from_dimension(&stair.going, "mm")?;
        Ok(panel)
    }
    pub fn planting(document: &Document) -> Result<Self, String> {
        let mut panel = Self::new(document, "Plant".into())?;
        panel.kind = ArchitectureKind::Plant;
        Ok(panel)
    }
    pub fn planting_existing(document: &Document, record: EntityId) -> Result<Self, String> {
        let mut panel = Self::planting(document)?;
        panel.plant = super::plant::PlantPanel::existing(document, record)?;
        Ok(panel)
    }
    pub fn stairs(document: &Document, name: String) -> Result<Self, String> {
        let mut panel = Self::new(document, name)?;
        panel.kind = ArchitectureKind::Stair;
        panel.stair_placing = true;
        Ok(panel)
    }
    pub fn equipment_preset(
        document: &Document,
        kind: multicad_core::equipment::Kind,
    ) -> Result<Self, String> {
        let mut panel = Self::equipment(document)?;
        panel.equipment.select(kind);
        Ok(panel)
    }
    pub fn equipment(document: &Document) -> Result<Self, String> {
        let mut panel = Self::new(document, "Equipment".into())?;
        panel.kind = ArchitectureKind::Equipment;
        Ok(panel)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn repeat_equipment(&mut self, document: &Document) -> bool {
        if self.kind != ArchitectureKind::Equipment || !self.equipment.repeat_after_apply(document)
        {
            return false;
        }
        self.stamp.changed();
        self.error = None;
        true
    }
    pub fn label(&self) -> &'static str {
        self.kind.label()
    }
    fn load_level(&mut self, document: &Document) -> Result<(), String> {
        if let Some(id) = self.existing {
            let level = model::read_level(document, id).map_err(|e| e.to_string())?;
            self.elevation = DimensionField::from_dimension(&level.elevation, "mm")?;
            self.height = DimensionField::from_dimension(&level.height, "mm")?;
            self.slab_thickness = DimensionField::from_dimension(&level.slab_thickness, "mm")?;
        }
        Ok(())
    }
    fn load_envelope(&mut self, document: &Document) -> Result<(), String> {
        if let Some(id) = self.existing {
            let env = model::read_envelope(document, id).map_err(|e| e.to_string())?;
            self.target = Some(env.target);
            for (field, value) in self
                .clearances
                .iter_mut()
                .zip([&env.front, &env.back, &env.left, &env.right, &env.top])
            {
                *field = DimensionField::from_dimension(value, "mm")?;
            }
            self.label = env.label;
        }
        Ok(())
    }
    /// Replace the outline with the chosen room's boundary.
    fn use_room(&mut self, document: &Document) -> Result<(), String> {
        let wall = self
            .room
            .ok_or("Choose the room whose boundary the floor follows")?;
        let points = room_boundary(document, wall, self.room_boundary)?;
        self.outline.set_dimensions(&points, true)?;
        if let Ok(source) = model::read_wall(document, wall)
            && source.level.is_some()
        {
            self.level = source.level;
        }
        Ok(())
    }
    fn plane_z(&self, document: &Document) -> f64 {
        multicad_core::parameters::evaluate(document)
            .ok()
            .and_then(|p| {
                self.level
                    .and_then(|l| model::resolve_level(document, &p, l).ok())
            })
            .map_or(0., |l| l.elevation_mm)
    }
    /// True while an architecture outline or placement gesture uses the plan.
    pub fn outlines(&self) -> bool {
        matches!(
            self.kind,
            ArchitectureKind::Slab
                | ArchitectureKind::Pedestal
                | ArchitectureKind::Plant
                | ArchitectureKind::Equipment
                | ArchitectureKind::Stair
        )
    }
    /// Draw and edit the outline on the plan; returns whether inputs changed.
    pub fn viewport(
        &mut self,
        document: &Document,
        revision: u64,
        parameters: &multicad_core::parameters::EvaluatedParameters,
        view: TerminationViewport<'_>,
    ) -> bool {
        if self.kind == ArchitectureKind::Stair {
            return self.stair_viewport(document, parameters, view);
        }
        if self.kind == ArchitectureKind::Equipment {
            let changed = self.equipment.viewport(document, view);
            if changed {
                self.stamp.changed();
                self.error = None;
            }
            return changed;
        }
        if self.kind == ArchitectureKind::Plant {
            let changed = self.plant.viewport(view);
            if changed {
                self.stamp.changed();
                self.error = None;
            }
            return changed;
        }
        if !self.outlines() {
            return false;
        }
        if self.targets.as_ref().is_none_or(|(r, _)| *r != revision) {
            self.targets = Some((revision, plan_targets(document, self.feature)));
        }
        let targets = self
            .targets
            .as_ref()
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        let z = self.plane_z(document);
        // The chosen room's boundary is shown so its scope is explicit.
        if let Some(wall) = self.room
            && let Ok(points) = room_boundary(document, wall, self.room_boundary)
            && let Ok(parameters) = multicad_core::parameters::evaluate(document)
        {
            let screen: Vec<egui::Pos2> = points
                .iter()
                .filter_map(|[x, y]| {
                    let x = x
                        .value_in(
                            multicad_core::parameters::Unit::Millimeter,
                            Some(&parameters),
                        )
                        .ok()?;
                    let y = y
                        .value_in(
                            multicad_core::parameters::Unit::Millimeter,
                            Some(&parameters),
                        )
                        .ok()?;
                    let s = view
                        .camera
                        .project([x as f32, y as f32, z as f32], view.rect);
                    Some(egui::pos2(s[0], s[1]))
                })
                .collect();
            if screen.len() >= 3 {
                let mut closed = screen.clone();
                closed.push(screen[0]);
                view.painter.extend(egui::Shape::dashed_line(
                    &closed,
                    egui::Stroke::new(2., egui::Color32::from_rgb(110, 210, 120)),
                    10.,
                    5.,
                ));
            }
        }
        let changed = self.outline.viewport(document, &view, z, &targets);
        if changed {
            self.stamp.changed();
            self.error = None;
        }
        changed
    }
    fn stair(&self, document: &Document) -> Result<StairDefinition, String> {
        Ok(StairDefinition {
            output: BodyOutput { body: self.body },
            level: self.level.ok_or("Choose the stair's base level")?,
            to_level: self.to_level,
            form: self.form,
            start: [length(self.start[0]), length(self.start[1])],
            direction_deg: self.direction_deg,
            width: self.width.resolve(document)?,
            risers: self.risers,
            going: self.going.resolve(document)?,
        })
    }
    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<ArchitectureCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.kind == ArchitectureKind::Stair && self.stair_placing {
            self.level.ok_or("Choose the stair's base level")?;
            return Err("Click the first riser and walking direction, or turn off Place in view to use the current position".into());
        }
        if self.kind == ArchitectureKind::Plant {
            return self.plant.request(document).map(ArchitectureCommand::Plant);
        }
        if self.kind == ArchitectureKind::Equipment {
            return self
                .equipment
                .request(document)
                .map(ArchitectureCommand::Equipment);
        }
        let creates = match self.kind {
            ArchitectureKind::Level | ArchitectureKind::Envelope | ArchitectureKind::Guide => {
                self.existing.is_none()
            }
            _ => self.feature.is_none(),
        };
        if creates && (self.name.trim().is_empty() || self.name.len() > 1000) {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let name = self.name.clone();
        Ok(match self.kind {
            ArchitectureKind::Plant | ArchitectureKind::Equipment => unreachable!("handled above"),
            ArchitectureKind::Level => ArchitectureCommand::Level {
                existing: self.existing,
                name,
                value: LevelDefinition {
                    elevation: self.elevation.resolve(document)?,
                    height: self.height.resolve(document)?,
                    slab_thickness: self.slab_thickness.resolve(document)?,
                },
            },
            ArchitectureKind::Guide => ArchitectureCommand::Guide {
                existing: self.existing,
                name,
                value: self.guide.definition(document)?,
                generate: self.guide.generate,
            },
            ArchitectureKind::Envelope => {
                let [front, back, left, right, top] = &self.clearances;
                ArchitectureCommand::Envelope {
                    existing: self.existing,
                    name,
                    value: EnvelopeDefinition {
                        target: self
                            .target
                            .ok_or("Choose the Part or Instance to surround")?,
                        front: front.resolve(document)?,
                        back: back.resolve(document)?,
                        left: left.resolve(document)?,
                        right: right.resolve(document)?,
                        top: top.resolve(document)?,
                        label: self.label.clone(),
                    },
                }
            }
            ArchitectureKind::Slab | ArchitectureKind::Pedestal => {
                let pedestal = self.kind == ArchitectureKind::Pedestal;
                ArchitectureCommand::Slab {
                    edit: self.feature,
                    name,
                    slab: SlabDefinition {
                        output: BodyOutput { body: self.body },
                        level: self.level.ok_or("Choose the level")?,
                        kind: if pedestal {
                            SlabKind::Pedestal
                        } else {
                            SlabKind::Slab
                        },
                        outline: self.outline.dimensions(document)?,
                        thickness: (pedestal || self.override_thickness)
                            .then(|| self.thickness.resolve(document))
                            .transpose()?,
                        holes: self.holes.clone(),
                    },
                }
            }
            ArchitectureKind::Stair => ArchitectureCommand::Stair {
                edit: self.feature,
                name,
                stair: self.stair(document)?,
            },
        })
    }
    fn level_combo(
        ui: &mut egui::Ui,
        id: &str,
        levels: &[(EntityId, String)],
        value: &mut Option<EntityId>,
        none: Option<&str>,
    ) -> bool {
        let mut changed = false;
        let selected = value
            .and_then(|v| levels.iter().find(|(l, _)| *l == v).map(|(_, n)| n.clone()))
            .or(none.map(str::to_owned))
            .unwrap_or_else(|| "Choose a level".into());
        controls::inspector_combo(ui, id)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                if let Some(none) = none {
                    changed |= ui.selectable_value(value, None, none).changed();
                }
                for (level, name) in levels {
                    changed |= ui.selectable_value(value, Some(*level), name).changed();
                }
            });
        changed
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong(self.kind.label());
        let mut changed = false;
        if self.feature.is_none() {
            ui.horizontal_wrapped(|ui| {
                for kind in ArchitectureKind::ALL {
                    if ui
                        .selectable_value(&mut self.kind, kind, kind.label())
                        .changed()
                    {
                        self.existing = None;
                        self.stair_placing = kind == ArchitectureKind::Stair;
                        self.stair_anchor = None;
                        changed = true;
                    }
                }
            });
        }
        let levels: Vec<_> = model::levels(document)
            .map(|e| (e.id, e.name.clone()))
            .collect();
        match self.kind {
            ArchitectureKind::Level | ArchitectureKind::Envelope | ArchitectureKind::Guide => {
                let level = self.kind == ArchitectureKind::Level;
                let existing: Vec<_> = match self.kind {
                    ArchitectureKind::Level => levels.clone(),
                    ArchitectureKind::Envelope => model::envelopes(document)
                        .map(|e| (e.id, e.name.clone()))
                        .collect(),
                    _ => drawings::guides(document)
                        .map(|e| (e.id, e.name.clone()))
                        .collect(),
                };
                let before = self.existing;
                if Self::level_combo(
                    ui,
                    "architecture-existing",
                    &existing,
                    &mut self.existing,
                    Some(match self.kind {
                        ArchitectureKind::Level => "New level",
                        ArchitectureKind::Envelope => "New envelope",
                        _ => "New guide",
                    }),
                ) {
                    changed = true;
                }
                if before != self.existing {
                    let loaded = match (self.kind, self.existing) {
                        (ArchitectureKind::Guide, Some(id)) => self.guide.load(document, id),
                        (ArchitectureKind::Guide, None) => Ok(()),
                        _ if level => self.load_level(document),
                        _ => self.load_envelope(document),
                    };
                    if let Err(error) = loaded {
                        self.error = Some(error);
                    }
                }
            }
            _ => {}
        }
        let naming = match self.kind {
            ArchitectureKind::Plant | ArchitectureKind::Equipment => false,
            ArchitectureKind::Level | ArchitectureKind::Envelope | ArchitectureKind::Guide => {
                self.existing.is_none()
            }
            _ => self.feature.is_none(),
        };
        if naming {
            ui.label("Name");
            changed |= ui
                .add(egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()))
                .changed();
        }
        ui.separator();
        match self.kind {
            ArchitectureKind::Level => {
                ui.small("Editing a level rebuilds every slab, stair and wall placed on it.");
                changed |=
                    self.elevation
                        .show(ui, "Elevation", "Finished floor height above Z = 0");
                changed |= self
                    .height
                    .show(ui, "Floor-to-floor height", "Up to the next level");
                changed |= self.slab_thickness.show(
                    ui,
                    "Slab thickness",
                    "Default thickness of floor slabs on this level, below the floor",
                );
            }
            ArchitectureKind::Slab | ArchitectureKind::Pedestal => {
                ui.label("Level");
                changed |= Self::level_combo(ui, "slab-level", &levels, &mut self.level, None);
                let rooms = rooms(document);
                ui.label("Room boundary");
                Self::level_combo(
                    ui,
                    "slab-room",
                    &rooms,
                    &mut self.room,
                    Some("None (draw or enter the outline)"),
                );
                if self.room.is_some() {
                    ui.horizontal_wrapped(|ui| {
                        for b in RoomBoundary::ALL {
                            ui.radio_value(&mut self.room_boundary, b, b.label());
                        }
                    });
                    let name = rooms
                        .iter()
                        .find(|(id, _)| Some(*id) == self.room)
                        .map_or("", |(_, n)| n.as_str());
                    ui.small(format!(
                        "Only {name}'s boundary (green dashes in the viewport) is used; other rooms are not included. The outline is copied, so later wall edits do not move it."
                    ));
                    if ui.button("Use room boundary").clicked() {
                        match self.use_room(document) {
                            Ok(()) => changed = true,
                            Err(e) => self.error = Some(e),
                        }
                    }
                }
                let plane = format!("the level floor (z = {} mm)", self.plane_z(document));
                changed |= self.outline.show(ui, document, &plane);
                if self.kind == ArchitectureKind::Pedestal {
                    changed |= self.thickness.show(
                        ui,
                        "Pedestal height",
                        "Raised above the level's floor",
                    );
                } else {
                    changed |= ui
                        .checkbox(
                            &mut self.override_thickness,
                            "Override the level's slab thickness",
                        )
                        .changed();
                    if self.override_thickness {
                        changed |= self.thickness.show(ui, "Thickness", "Below the floor");
                    }
                }
            }
            ArchitectureKind::Stair => {
                if ui
                    .selectable_label(self.stair_placing, "Place in view")
                    .clicked()
                {
                    self.stair_placing = !self.stair_placing;
                    self.stair_anchor = None;
                    changed = true;
                }
                ui.small(if self.stair_placing {
                    if self.stair_anchor.is_some() {
                        "Click the walking direction from the first riser."
                    } else {
                        "Click the first riser on the base level, then the walking direction."
                    }
                } else {
                    "Use Place in view to reposition the stair graphically."
                });
                ui.checkbox(&mut self.stair_snap, "Snap placement to 10 mm");
                ui.label("From level");
                changed |= Self::level_combo(ui, "stair-level", &levels, &mut self.level, None);
                ui.label("To level");
                changed |= Self::level_combo(
                    ui,
                    "stair-to-level",
                    &levels,
                    &mut self.to_level,
                    Some("Floor-to-floor height of the base level"),
                );
                ui.label("Form");
                let forms = [
                    (StairForm::Straight, "Straight"),
                    (
                        StairForm::QuarterTurn { turn: Turn::Left },
                        "Quarter turn left",
                    ),
                    (
                        StairForm::QuarterTurn { turn: Turn::Right },
                        "Quarter turn right",
                    ),
                    (StairForm::HalfTurn { turn: Turn::Left }, "Half turn left"),
                    (StairForm::HalfTurn { turn: Turn::Right }, "Half turn right"),
                ];
                let selected = forms
                    .iter()
                    .find(|(f, _)| *f == self.form)
                    .map_or("Straight", |(_, l)| l);
                controls::inspector_combo(ui, "stair-form")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (form, label) in forms {
                            changed |= ui.selectable_value(&mut self.form, form, label).changed();
                        }
                    });
                ui.label("Start of the first riser (x, y mm)");
                ui.horizontal(|ui| {
                    for v in &mut self.start {
                        changed |= ui.add(egui::DragValue::new(v).speed(10.)).changed();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Direction °");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.direction_deg).range(-360.0..=360.0))
                        .changed();
                });
                changed |= self.width.show(ui, "Width", "Clear stair width");
                ui.horizontal(|ui| {
                    ui.label("Risers");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.risers).range(2..=64))
                        .changed();
                });
                changed |= self
                    .going
                    .show(ui, "Going", "Horizontal depth of one tread");
                if let Ok(stair) = self.stair(document)
                    && let Ok(parameters) = multicad_core::parameters::evaluate(document)
                    && let Ok(resolved) = model::resolve_stair(document, &parameters, &stair)
                {
                    let c = resolved.check;
                    ui.small(format!(
                        "Rise {:.0} mm · riser {:.1} mm · going {:.0} mm · 2R+G {:.0} mm · run {:.0} mm",
                        c.rise_mm, c.riser_mm, c.going_mm, c.step_measure_mm, c.run_mm
                    ));
                    for note in &c.notes {
                        ui.small(note);
                    }
                    ui.small(
                        "Geometric measures only; not a building-code or accessibility approval.",
                    );
                }
            }
            ArchitectureKind::Envelope => {
                ui.label("Target Part or Instance");
                let targets: Vec<_> = document
                    .entities()
                    .filter(|e| {
                        matches!(
                            e.data,
                            EntityData::Part { .. } | EntityData::Instance { .. }
                        )
                    })
                    .map(|e| (e.id, e.name.clone()))
                    .collect();
                changed |=
                    Self::level_combo(ui, "envelope-target", &targets, &mut self.target, None);
                for (field, (label, help)) in self.clearances.iter_mut().zip(CLEARANCES) {
                    changed |= field.show(ui, label, help);
                }
                ui.label("Label");
                changed |= ui.text_edit_singleline(&mut self.label).changed();
                ui.small("The envelope follows the target's placement and is checked for overlaps with check_clearances.");
            }
            ArchitectureKind::Plant => {
                changed |= self.plant.show(ui, document);
            }
            ArchitectureKind::Equipment => {
                changed |= self.equipment.show(ui, document);
            }
            ArchitectureKind::Guide => {
                changed |= self.guide.show(ui, document);
            }
        }
        ui.separator();
        form::preview_status(ui, self.stamp, preview);
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
            self.error = None;
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => match self.request(document, enabled) {
                Ok(request) => Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Architecture(Box::new(request)),
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply)
                if preview.ready(self.stamp) && self.request(document, enabled).is_ok() =>
            {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::commands::{Actor, Channel, Session};
    use multicad_model::Compiler;

    fn apply(session: &mut Session, panel: &ArchitecturePanel) -> ModelPrepared {
        let prepared = panel
            .request(session.document(), true)
            .unwrap()
            .prepare(session, Actor::local(Channel::Ui))
            .unwrap();
        Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
        prepared
    }
    #[test]
    fn graphical_stair_placement_uses_the_level_plane_and_survives_native_edit_history_and_reopen()
    {
        use multicad_core::project_file::{self, ProjectFile};
        let (mut session, levels, _, _) = crate::architecture_view::tests::building();
        let mut panel =
            ArchitecturePanel::stairs(session.document(), "Upper stair".into()).unwrap();
        panel.seed_new_level(session.document(), Some(levels[1]));
        assert!(
            panel
                .request(session.document(), true)
                .unwrap_err()
                .contains("first riser")
        );
        let before = session.document().clone();
        let ctx = egui::Context::default();
        let camera = crate::camera::Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            target: [0., 0., 3000.],
            scale: 0.1,
            perspective: false,
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        let click = |panel: &mut ArchitecturePanel, point: [f64; 2]| {
            let p = camera.project([point[0] as f32, point[1] as f32, 3000.], rect);
            let mut changed = false;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                changed = panel.viewport(
                    session.document(),
                    session.revision(),
                    session.parameters(),
                    TerminationViewport {
                        scene: None,
                        report: None,
                        camera,
                        rect,
                        pointer: Some(egui::pos2(p[0], p[1])),
                        clicked: true,
                        painter: ui.painter(),
                        section: None,
                    },
                );
            });
            output.textures_delta.clear();
            changed
        };
        assert!(click(&mut panel, [1003., 504.]));
        assert_eq!(panel.start, [1000., 500.]);
        assert!(!click(&mut panel, [1003., 504.]));
        assert!(panel.error.as_ref().unwrap().contains("direction away"));
        assert!(panel.request(session.document(), true).is_err());
        assert!(click(&mut panel, [1003., 1504.]));
        assert!(!panel.stair_placing);
        assert!((panel.direction_deg - 90.).abs() < 1e-9);
        assert_eq!(session.document(), &before);
        let prepared = apply(&mut session, &panel);
        let feature = prepared.outputs[0].feature;
        let body = prepared.outputs[0].body;
        session.commit(prepared.prepared).unwrap();
        let compiled = Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let native = compiled.snapshot();
        let shape = &native.shapes[&feature];
        assert_eq!(shape.solid_count, 1);
        for (actual, expected) in shape.bounds.min.iter().zip([500., 500., 3000.]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        for (actual, expected) in shape.bounds.max.iter().zip([1500., 4820., 6000.]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        let placed = session.document().clone();
        let mut edit =
            ArchitecturePanel::existing(session.document(), feature, "Upper stair".into()).unwrap();
        assert!(!edit.stair_placing);
        edit.stair_placing = true;
        // Move through the same real camera/plane hit path without changing identities.
        let move_click = |panel: &mut ArchitecturePanel, point: [f64; 2]| {
            let p = camera.project([point[0] as f32, point[1] as f32, 3000.], rect);
            let mut output = ctx.run_ui(Default::default(), |ui| {
                assert!(panel.viewport(
                    session.document(),
                    session.revision(),
                    session.parameters(),
                    TerminationViewport {
                        scene: None,
                        report: None,
                        camera,
                        rect,
                        pointer: Some(egui::pos2(p[0], p[1])),
                        clicked: true,
                        painter: ui.painter(),
                        section: None,
                    }
                ));
            });
            output.textures_delta.clear();
        };
        move_click(&mut edit, [2503., 504.]);
        move_click(&mut edit, [3503., 504.]);
        assert_eq!(
            session.document(),
            &placed,
            "a draft move is cancellable and source-neutral"
        );
        let prepared = apply(&mut session, &edit);
        assert_eq!(
            (prepared.outputs[0].feature, prepared.outputs[0].body),
            (feature, body)
        );
        session.commit(prepared.prepared).unwrap();
        let edited = session.document().clone();
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &placed);
        session
            .redo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &edited);
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(edited.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let stair = model::read_stair(&reopened.document, feature).unwrap();
        assert_eq!(stair.output.body, body);
        assert_eq!(stair.level, levels[1]);
        assert_eq!(stair.start, [length(2500.), length(500.)]);
        assert_eq!(stair.direction_deg, 0.);
        Compiler::default().compile(&reopened.document, 0).unwrap();
    }

    #[test]
    fn panel_creates_a_level_slab_stair_and_envelope_and_reopens_the_stair() {
        let mut session = Session::new(Document::new("Levels panel").unwrap()).unwrap();
        let mut panel = ArchitecturePanel::new(session.document(), "Ground".into()).unwrap();
        assert_eq!(panel.kind, ArchitectureKind::Level, "no level exists yet");
        let prepared = apply(&mut session, &panel);
        session.commit(prepared.prepared).unwrap();
        let level = model::levels(session.document()).next().unwrap().id;

        panel = ArchitecturePanel::new(session.document(), "Floor".into()).unwrap();
        assert_eq!(panel.kind, ArchitectureKind::Slab);
        assert_eq!(panel.level, Some(level));
        let prepared = apply(&mut session, &panel);
        let slab = prepared.outputs[0].body;
        session.commit(prepared.prepared).unwrap();

        panel = ArchitecturePanel::new(session.document(), "Stair".into()).unwrap();
        panel.kind = ArchitectureKind::Stair;
        let prepared = apply(&mut session, &panel);
        let stair = prepared.outputs[0].feature;
        session.commit(prepared.prepared).unwrap();

        panel = ArchitecturePanel::new(session.document(), "Access".into()).unwrap();
        panel.kind = ArchitectureKind::Envelope;
        assert!(
            panel.request(session.document(), true).is_err(),
            "needs a target"
        );
        panel.target = Some(slab);
        let prepared = apply(&mut session, &panel);
        session.commit(prepared.prepared).unwrap();
        assert_eq!(model::envelopes(session.document()).count(), 1);

        // Reopened, the stair keeps its Part and edits through its feature.
        let mut edit =
            ArchitecturePanel::existing(session.document(), stair, "Stair".into()).unwrap();
        assert_eq!(edit.kind, ArchitectureKind::Stair);
        edit.risers = 15;
        let command = edit.request(session.document(), true).unwrap();
        assert_eq!(command.edit_id(), Some(stair));
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(prepared.outputs[0].feature, stair);

        // Editing the level is an ordinary source transaction.
        let mut level_edit = ArchitecturePanel::new(session.document(), String::new()).unwrap();
        level_edit.kind = ArchitectureKind::Level;
        level_edit.existing = Some(level);
        level_edit.load_level(session.document()).unwrap();
        level_edit.elevation = mm(300.).unwrap();
        let command = level_edit.request(session.document(), true).unwrap();
        assert_eq!(command.edit_id(), None);
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        let model = Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
        assert!(
            model.snapshot().envelope_boxes.len() == 1,
            "the envelope box follows the rebuilt slab"
        );

        // A floor plan guide on the level generates one linked view; a
        // rerun from the reopened guide updates it instead of adding one.
        let mut guide = ArchitecturePanel::new(session.document(), "Plans".into()).unwrap();
        guide.kind = ArchitectureKind::Guide;
        assert_eq!(guide.guide.levels.len(), 1);
        let prepared = apply(&mut session, &guide);
        session.commit(prepared.prepared).unwrap();
        let guide_id = drawings::guides(session.document()).next().unwrap().id;
        let count = |s: &Session| {
            drawings::books(s.document())
                .map(|b| {
                    drawings::read(s.document(), b)
                        .unwrap()
                        .pages
                        .iter()
                        .map(|p| p.views.len())
                        .sum::<usize>()
                })
                .sum::<usize>()
        };
        assert_eq!(count(&session), 1);
        let mut rerun = ArchitecturePanel::new(session.document(), String::new()).unwrap();
        rerun.kind = ArchitectureKind::Guide;
        rerun.existing = Some(guide_id);
        rerun.guide.load(session.document(), guide_id).unwrap();
        rerun.guide.cut_height = mm(1100.).unwrap();
        let prepared = apply(&mut session, &rerun);
        session.commit(prepared.prepared).unwrap();
        assert_eq!(count(&session), 1, "the rerun updates the linked view");
        rerun.guide.levels.clear();
        assert!(
            rerun
                .request(session.document(), true)
                .unwrap_err()
                .contains("at least one level")
        );

        panel.kind = ArchitectureKind::Slab;
        panel.outline.set_text(1, 0, "four");
        assert!(
            panel
                .request(session.document(), true)
                .unwrap_err()
                .contains("Corner 2 x")
        );
    }

    /// Two closed 200 mm rooms side by side on the ground level.
    fn two_rooms(session: &mut Session, level: EntityId) -> [EntityId; 2] {
        use multicad_core::model::{
            Justification, NewType, NewTypeKind, WallChange, WallDefinition, WallRequest,
            WallTypeDefinition,
        };
        let ty = NewType::new(
            "Wall 200",
            NewTypeKind::Wall(WallTypeDefinition {
                thickness: length(200.),
                height: length(2500.),
                justification: Justification::Center,
                material: None,
            }),
        );
        let mut walls = [EntityId::new(); 2];
        for (k, x0) in [0., 6000.].into_iter().enumerate() {
            let path = [[0., 0.], [4000., 0.], [4000., 3000.], [0., 3000.]]
                .iter()
                .map(|p| [length(p[0] + x0), length(p[1])])
                .collect();
            let request = WallRequest::new(
                format!("Room {}", k + 1),
                WallDefinition {
                    layer_placement: None,
                    output: BodyOutput {
                        body: EntityId::new(),
                    },
                    wall_type: ty.id,
                    path,
                    closed: true,
                    level: Some(level),
                    base_z: length(0.),
                    thickness: None,
                    height: None,
                    justification: None,
                    openings: vec![],
                    cutouts: vec![],
                },
            );
            let types = if k == 0 { vec![ty.clone()] } else { vec![] };
            let prepared = model::prepare_wall_change(
                session,
                session.revision(),
                Actor::local(Channel::Ui),
                &types,
                &WallChange::Create(&request),
            )
            .unwrap();
            walls[k] = prepared.outputs[0].feature;
            session.commit(prepared.prepared).unwrap();
        }
        walls
    }

    #[test]
    fn a_floor_reuses_one_explicit_room_boundary_and_previews_only_that_room() {
        let mut session = Session::new(Document::new("Room floor").unwrap()).unwrap();
        let panel = ArchitecturePanel::new(session.document(), "Ground".into()).unwrap();
        let prepared = apply(&mut session, &panel);
        session.commit(prepared.prepared).unwrap();
        let level = model::levels(session.document()).next().unwrap().id;
        let [first, second] = two_rooms(&mut session, level);
        assert_eq!(rooms(session.document()).len(), 2);

        let mut floor = ArchitecturePanel::new(session.document(), "Floor".into()).unwrap();
        assert!(floor.outlines());
        assert!(
            floor.use_room(session.document()).is_err(),
            "a room must be chosen"
        );
        floor.room = Some(second);
        floor.room_boundary = RoomBoundary::Inside;
        floor.use_room(session.document()).unwrap();
        let points = floor.outline.positions(session.document()).unwrap();
        // Only room 2's inside faces: 3800 x 2800 between x = 6100 and 9900.
        assert_eq!(points.len(), 4);
        assert!(points.iter().all(|p| (6100. ..=9900.).contains(&p[0])));
        let prepared = apply(&mut session, &floor);
        let feature = prepared.outputs[0].feature;
        let model = Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
        let volume = model.snapshot().shapes[&feature].volume_mm3;
        assert!((volume - 3800. * 2800. * 200.).abs() < 1e-3, "{volume}");

        // The wall path keeps its authored dimensions; outside faces grow.
        floor.room = Some(first);
        floor.room_boundary = RoomBoundary::Path;
        floor.use_room(session.document()).unwrap();
        assert_eq!(
            floor.outline.dimensions(session.document()).unwrap(),
            model::read_wall(session.document(), first).unwrap().path
        );
        floor.room_boundary = RoomBoundary::Outside;
        floor.use_room(session.document()).unwrap();
        let points = floor.outline.positions(session.document()).unwrap();
        assert!(points.contains(&[-100., -100.]) && points.contains(&[4100., 3100.]));

        // The boundary overlay and outline paint through the viewport path.
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            let view = TerminationViewport {
                scene: None,
                report: None,
                camera: crate::camera::Camera::default(),
                rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640., 480.)),
                pointer: None,
                clicked: false,
                painter: &painter,
                section: None,
            };
            assert!(!floor.viewport(
                session.document(),
                session.revision(),
                session.parameters(),
                view
            ));
        })
        .drop_without_applying_deltas();
        let missing = room_boundary(session.document(), EntityId::new(), RoomBoundary::Inside);
        assert!(missing.is_err());
    }
}
