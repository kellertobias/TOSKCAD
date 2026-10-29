//! Level inspector: define or edit levels and service envelopes, and place
//! slabs, pedestals and stairs on a level. Slabs and stairs are features with
//! one output Part; levels and envelopes are reusable source entities.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityData, EntityId},
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
}
impl ArchitectureKind {
    const ALL: [Self; 5] = [
        Self::Level,
        Self::Slab,
        Self::Pedestal,
        Self::Stair,
        Self::Envelope,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Level => "Level",
            Self::Slab => "Floor slab",
            Self::Pedestal => "Pedestal",
            Self::Stair => "Stair",
            Self::Envelope => "Service envelope",
        }
    }
}

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub enum ArchitectureCommand {
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
}
impl ArchitectureCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        let source = |prepared| ModelPrepared {
            prepared,
            outputs: vec![],
        };
        match self {
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
        }
        .map_err(|e| e.to_string())
    }
    /// The feature this command edits. Level and envelope edits change
    /// source entities, not features, so they run as ordinary transactions.
    pub fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::Slab { edit, .. } | Self::Stair { edit, .. } => *edit,
            Self::Level { .. } | Self::Envelope { .. } => None,
        }
    }
}

pub(super) struct ArchitecturePanel {
    stamp: FormStamp,
    kind: ArchitectureKind,
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
    outline: String,
    override_thickness: bool,
    thickness: DimensionField,
    to_level: Option<EntityId>,
    form: StairForm,
    start: [f64; 2],
    direction_deg: f64,
    width: DimensionField,
    risers: u32,
    going: DimensionField,
    target: Option<EntityId>,
    clearances: [DimensionField; 5],
    label: String,
    error: Option<String>,
}
fn literal(d: &Dimension) -> String {
    match d {
        Dimension::Literal { value, .. } => format!("{value}"),
        Dimension::Expression { expression } => expression.text.clone(),
    }
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
    pub fn new(document: &Document, name: String) -> Result<Self, String> {
        let level = model::levels(document).next().map(|e| e.id);
        Ok(Self {
            stamp: FormStamp::new(),
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
            outline: "0, 0\n6000, 0\n6000, 4000\n0, 4000".into(),
            override_thickness: false,
            thickness: mm(200.)?,
            to_level: None,
            form: StairForm::Straight,
            start: [0., 0.],
            direction_deg: 0.,
            width: mm(1000.)?,
            risers: 16,
            going: mm(270.)?,
            target: None,
            clearances: [mm(600.)?, mm(0.)?, mm(0.)?, mm(0.)?, mm(0.)?],
            label: String::new(),
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
            panel.outline = slab
                .outline
                .iter()
                .map(|[x, y]| format!("{}, {}", literal(x), literal(y)))
                .collect::<Vec<_>>()
                .join("\n");
            panel.override_thickness = slab.thickness.is_some();
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
    pub fn stamp(&self) -> FormStamp {
        self.stamp
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
    fn outline(&self) -> Result<Vec<[Dimension; 2]>, String> {
        self.outline
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, line)| {
                let values: Vec<f64> = line
                    .split(',')
                    .map(|v| v.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("Outline line {} must be \"x, y\" in mm", i + 1))?;
                match values.as_slice() {
                    [x, y] if x.is_finite() && y.is_finite() => Ok([length(*x), length(*y)]),
                    _ => Err(format!("Outline line {} must be \"x, y\" in mm", i + 1)),
                }
            })
            .collect()
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
        let creates = match self.kind {
            ArchitectureKind::Level | ArchitectureKind::Envelope => self.existing.is_none(),
            _ => self.feature.is_none(),
        };
        if creates && (self.name.trim().is_empty() || self.name.len() > 1000) {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let name = self.name.clone();
        Ok(match self.kind {
            ArchitectureKind::Level => ArchitectureCommand::Level {
                existing: self.existing,
                name,
                value: LevelDefinition {
                    elevation: self.elevation.resolve(document)?,
                    height: self.height.resolve(document)?,
                    slab_thickness: self.slab_thickness.resolve(document)?,
                },
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
                        outline: self.outline()?,
                        thickness: (pedestal || self.override_thickness)
                            .then(|| self.thickness.resolve(document))
                            .transpose()?,
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
                        changed = true;
                    }
                }
            });
        }
        let levels: Vec<_> = model::levels(document)
            .map(|e| (e.id, e.name.clone()))
            .collect();
        match self.kind {
            ArchitectureKind::Level | ArchitectureKind::Envelope => {
                let level = self.kind == ArchitectureKind::Level;
                let existing: Vec<_> = if level {
                    levels.clone()
                } else {
                    model::envelopes(document)
                        .map(|e| (e.id, e.name.clone()))
                        .collect()
                };
                let before = self.existing;
                if Self::level_combo(
                    ui,
                    "architecture-existing",
                    &existing,
                    &mut self.existing,
                    Some(if level { "New level" } else { "New envelope" }),
                ) {
                    changed = true;
                }
                if before != self.existing {
                    let loaded = if level {
                        self.load_level(document)
                    } else {
                        self.load_envelope(document)
                    };
                    if let Err(error) = loaded {
                        self.error = Some(error);
                    }
                }
            }
            _ => {}
        }
        let naming = match self.kind {
            ArchitectureKind::Level | ArchitectureKind::Envelope => self.existing.is_none(),
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
                ui.label("Outline points (x, y in mm, one per line)");
                changed |= ui
                    .add(
                        egui::TextEdit::multiline(&mut self.outline)
                            .desired_rows(4)
                            .desired_width(ui.available_width()),
                    )
                    .changed();
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

        panel.kind = ArchitectureKind::Slab;
        panel.outline = "0, 0\nfour, 0".into();
        assert!(
            panel
                .request(session.document(), true)
                .unwrap_err()
                .contains("Outline line 2")
        );
    }
}
