//! Machining inspector: cut a groove, rabbet, pocket, drilling or placed
//! hardware pattern into one or more panels. Positions are panel face
//! coordinates. An applied operation reopens with its targets fixed.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{
        self, MachiningDefinition, MachiningFace, MachiningKind, ModelPrepared, PanelAxis,
        PanelEdgeName, length,
    },
};

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub struct MachiningCommand {
    pub existing: Option<EntityId>,
    pub name: String,
    pub machining: MachiningDefinition,
}
impl MachiningCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        match self.existing {
            Some(id) => {
                model::prepare_machining_edit(session, revision, actor, id, &self.machining)
            }
            None => model::prepare_machining(session, revision, actor, &self.name, &self.machining),
        }
        .map(|p| p.model)
        .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Groove,
    Rabbet,
    Pocket,
    Drill,
    Hardware,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Top,
    Bottom,
    Edge,
}
const EDGES: [(PanelEdgeName, &str); 4] = [
    (PanelEdgeName::UStart, "Length start"),
    (PanelEdgeName::UEnd, "Length end"),
    (PanelEdgeName::VStart, "Width start"),
    (PanelEdgeName::VEnd, "Width end"),
];

pub(super) struct MachiningPanel {
    stamp: FormStamp,
    existing: Option<EntityId>,
    name: String,
    kind: Kind,
    side: Side,
    edge: PanelEdgeName,
    targets: Vec<EntityId>,
    along: PanelAxis,
    offset: DimensionField,
    width: DimensionField,
    depth: DimensionField,
    through: bool,
    center: [DimensionField; 2],
    size: [DimensionField; 2],
    holes: String,
    diameter: DimensionField,
    pattern: Option<EntityId>,
    at: [DimensionField; 2],
    turns: u8,
    error: Option<String>,
}
fn field(v: f64) -> DimensionField {
    DimensionField::from_dimension(&length(v), "mm").expect("literal")
}
fn of(d: &multicad_core::constraint_sketch::Dimension) -> Result<DimensionField, String> {
    DimensionField::from_dimension(d, "mm")
}
fn text(d: &multicad_core::constraint_sketch::Dimension) -> String {
    match d {
        multicad_core::constraint_sketch::Dimension::Literal { value, .. } => value.to_string(),
        multicad_core::constraint_sketch::Dimension::Expression { expression } => {
            expression.text.clone()
        }
    }
}
impl MachiningPanel {
    pub fn new(name: String) -> Self {
        Self {
            stamp: FormStamp::new(),
            existing: None,
            name,
            kind: Kind::Groove,
            side: Side::Top,
            edge: PanelEdgeName::VStart,
            targets: vec![],
            along: PanelAxis::U,
            offset: field(20.),
            width: field(6.),
            depth: field(8.),
            through: false,
            center: [field(100.), field(100.)],
            size: [field(40.), field(30.)],
            holes: "100, 50".into(),
            diameter: field(5.),
            pattern: None,
            at: [field(100.), field(100.)],
            turns: 0,
            error: None,
        }
    }
    /// Open the operation that owns an applied machining cut.
    pub fn existing(document: &Document, cut: EntityId) -> Result<Self, String> {
        let cut = model::read_machining_cut(document, cut).map_err(|e| e.to_string())?;
        let op = model::read_machining(document, cut.operation).map_err(|e| e.to_string())?;
        let mut form = Self::new(
            document
                .get(cut.operation)
                .map(|e| e.name.clone())
                .unwrap_or_default(),
        );
        form.existing = Some(cut.operation);
        form.targets = op.targets.clone();
        (form.side, form.edge) = match op.face {
            MachiningFace::Top => (Side::Top, form.edge),
            MachiningFace::Bottom => (Side::Bottom, form.edge),
            MachiningFace::Edge { edge } => (Side::Edge, edge),
        };
        match &op.kind {
            MachiningKind::Groove {
                along,
                offset,
                width,
                depth,
            } => {
                form.kind = Kind::Groove;
                form.along = *along;
                (form.offset, form.width, form.depth) = (of(offset)?, of(width)?, of(depth)?);
            }
            MachiningKind::Rabbet { edge, width, depth } => {
                form.kind = Kind::Rabbet;
                form.edge = *edge;
                (form.width, form.depth) = (of(width)?, of(depth)?);
            }
            MachiningKind::Pocket {
                center,
                size,
                depth,
            } => {
                form.kind = Kind::Pocket;
                form.center = [of(&center[0])?, of(&center[1])?];
                form.size = [of(&size[0])?, of(&size[1])?];
                form.depth = of(depth)?;
            }
            MachiningKind::Drill {
                holes,
                diameter,
                depth,
            } => {
                form.kind = Kind::Drill;
                form.holes = holes
                    .iter()
                    .map(|[u, v]| format!("{}, {}", text(u), text(v)))
                    .collect::<Vec<_>>()
                    .join("\n");
                form.diameter = of(diameter)?;
                form.through = depth.is_none();
                if let Some(d) = depth {
                    form.depth = of(d)?;
                }
            }
            MachiningKind::Hardware {
                pattern,
                at,
                quarter_turns,
            } => {
                form.kind = Kind::Hardware;
                form.pattern = Some(*pattern);
                form.at = [of(&at[0])?, of(&at[1])?];
                form.turns = *quarter_turns;
            }
        }
        Ok(form)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Machining"
    }
    fn hole_list(&self) -> Result<Vec<[multicad_core::constraint_sketch::Dimension; 2]>, String> {
        self.holes
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, line)| {
                let v: Vec<f64> = line
                    .split(',')
                    .map(|p| p.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("Hole line {} must be \"u, v\" in millimeters", i + 1))?;
                match v.as_slice() {
                    [u, w] => Ok([length(*u), length(*w)]),
                    _ => Err(format!(
                        "Hole line {} must be \"u, v\" in millimeters",
                        i + 1
                    )),
                }
            })
            .collect()
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<MachiningCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        if self.targets.is_empty() {
            return Err("Choose at least one target panel".into());
        }
        let r = |f: &DimensionField| f.resolve(document);
        let kind = match self.kind {
            Kind::Groove => MachiningKind::Groove {
                along: self.along,
                offset: r(&self.offset)?,
                width: r(&self.width)?,
                depth: r(&self.depth)?,
            },
            Kind::Rabbet => MachiningKind::Rabbet {
                edge: self.edge,
                width: r(&self.width)?,
                depth: r(&self.depth)?,
            },
            Kind::Pocket => MachiningKind::Pocket {
                center: [r(&self.center[0])?, r(&self.center[1])?],
                size: [r(&self.size[0])?, r(&self.size[1])?],
                depth: r(&self.depth)?,
            },
            Kind::Drill => MachiningKind::Drill {
                holes: self.hole_list()?,
                diameter: r(&self.diameter)?,
                depth: (!self.through).then(|| r(&self.depth)).transpose()?,
            },
            Kind::Hardware => MachiningKind::Hardware {
                pattern: self.pattern.ok_or("Choose a hardware pattern")?,
                at: [r(&self.at[0])?, r(&self.at[1])?],
                quarter_turns: self.turns,
            },
        };
        let face = match self.side {
            Side::Top => MachiningFace::Top,
            Side::Bottom => MachiningFace::Bottom,
            Side::Edge => MachiningFace::Edge { edge: self.edge },
        };
        Ok(MachiningCommand {
            existing: self.existing,
            name: self.name.clone(),
            machining: MachiningDefinition {
                kind,
                face,
                targets: self.targets.clone(),
            },
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
        ui.strong("Machining");
        ui.small("Modeled cuts in panel face coordinates (u along the length, v along the width). Not a CNC toolpath.");
        ui.label("Name");
        let mut changed = ui
            .add_enabled(
                self.existing.is_none(),
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        ui.horizontal_wrapped(|ui| {
            for (kind, label) in [
                (Kind::Groove, "Groove"),
                (Kind::Rabbet, "Rabbet"),
                (Kind::Pocket, "Pocket"),
                (Kind::Drill, "Drill"),
                (Kind::Hardware, "Hardware"),
            ] {
                changed |= ui.radio_value(&mut self.kind, kind, label).changed();
            }
        });
        let drilling = matches!(self.kind, Kind::Drill | Kind::Hardware);
        if !drilling && self.side == Side::Edge {
            self.side = Side::Top;
        }
        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(&mut self.side, Side::Top, "Top face")
                .changed();
            changed |= ui
                .radio_value(&mut self.side, Side::Bottom, "Bottom face")
                .changed();
            if drilling {
                changed |= ui.radio_value(&mut self.side, Side::Edge, "Edge").changed();
            }
        });
        if self.side == Side::Edge || self.kind == Kind::Rabbet {
            controls::inspector_combo(ui, "machining-edge")
                .selected_text(
                    EDGES
                        .iter()
                        .find(|(e, _)| *e == self.edge)
                        .map_or("", |e| e.1),
                )
                .show_ui(ui, |ui| {
                    for (edge, label) in EDGES {
                        changed |= ui.selectable_value(&mut self.edge, edge, label).changed();
                    }
                });
            if self.side == Side::Edge {
                ui.small("On an edge, the first coordinate runs along the edge and the second up the thickness.");
            }
        }
        ui.separator();
        match self.kind {
            Kind::Groove => {
                ui.horizontal(|ui| {
                    changed |= ui
                        .radio_value(&mut self.along, PanelAxis::U, "Along length")
                        .changed();
                    changed |= ui
                        .radio_value(&mut self.along, PanelAxis::V, "Along width")
                        .changed();
                });
                changed |= self
                    .offset
                    .show(ui, "Offset", "From the start edge across the groove");
                changed |= self.width.show(ui, "Width", "Groove width");
                changed |= self.depth.show(ui, "Depth", "Blind depth into the face");
            }
            Kind::Rabbet => {
                changed |= self
                    .width
                    .show(ui, "Width", "Rabbet width in from the edge");
                changed |= self.depth.show(ui, "Depth", "Blind depth into the face");
            }
            Kind::Pocket => {
                changed |= self.center[0].show(ui, "Center u", "Pocket center along the length");
                changed |= self.center[1].show(ui, "Center v", "Pocket center along the width");
                changed |= self.size[0].show(ui, "Length", "Pocket size along u");
                changed |= self.size[1].show(ui, "Width", "Pocket size along v");
                changed |= self.depth.show(ui, "Depth", "Blind depth into the face");
            }
            Kind::Drill => {
                ui.label("Hole centers (u, v in mm, one per line)");
                changed |= ui
                    .add(
                        egui::TextEdit::multiline(&mut self.holes)
                            .desired_rows(3)
                            .desired_width(ui.available_width()),
                    )
                    .changed();
                changed |= self.diameter.show(ui, "Diameter", "Hole diameter");
                changed |= ui.checkbox(&mut self.through, "Through").changed();
                if !self.through {
                    changed |= self.depth.show(ui, "Depth", "Blind hole depth");
                }
            }
            Kind::Hardware => {
                let patterns: Vec<_> = model::hardware_patterns(document)
                    .map(|e| (e.id, e.name.clone()))
                    .collect();
                if patterns.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, "No hardware patterns yet; create one with the MCP set_hardware_pattern tool.");
                }
                controls::inspector_combo(ui, "machining-pattern")
                    .selected_text(
                        self.pattern
                            .and_then(|p| patterns.iter().find(|(id, _)| *id == p))
                            .map_or("Choose a pattern".into(), |(_, n)| n.clone()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in &patterns {
                            changed |= ui
                                .selectable_value(&mut self.pattern, Some(*id), name)
                                .changed();
                        }
                    });
                changed |= self.at[0].show(ui, "Position u", "Pattern origin along the length");
                changed |= self.at[1].show(ui, "Position v", "Pattern origin along the width");
                ui.horizontal(|ui| {
                    ui.label("Quarter turns");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.turns).range(0..=3))
                        .changed();
                });
            }
        }
        ui.separator();
        ui.strong("Target panels");
        for e in model::panels(document) {
            let Ok(panel) = model::read_panel(document, e.id) else {
                continue;
            };
            if matches!(panel.source, model::PanelSource::Profile { .. }) {
                continue;
            }
            let part = panel.output.body;
            let name = document.get(part).map_or(String::new(), |e| e.name.clone());
            let mut on = self.targets.contains(&part);
            if ui
                .add_enabled(self.existing.is_none(), egui::Checkbox::new(&mut on, name))
                .changed()
            {
                if on {
                    self.targets.push(part);
                } else {
                    self.targets.retain(|t| *t != part);
                }
                changed = true;
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
                    request: ModelRequest::Machining(Box::new(request)),
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
    use multicad_core::{
        commands::{Actor, Channel},
        document::{Entity, EntityData},
        materials::{Grain, LibraryDefinition, Stock, StockKind, prepare_library},
        model::{
            BodyOutput, GrainAxis, PanelDefinition, PanelEdges, PanelPlane, PanelRequest,
            PanelSource,
        },
    };
    use multicad_model::Compiler;

    #[test]
    fn form_drills_through_and_reopens_with_its_targets() {
        let actor = || Actor::local(Channel::Ui);
        let mut s = Session::new(Document::new("Machining form").unwrap()).unwrap();
        let birch = Entity::new(
            "Birch",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [210, 180, 140, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: birch.id,
            specification: StockKind::Panel {
                thickness_mm: 18.,
                length_mm: 2500.,
                width_mm: 1250.,
                grain: Grain::None,
            },
        })
        .entity(EntityId::new(), "Ply 18")
        .unwrap();
        let stock_id = stock.id;
        s.commit(prepare_library(&s, 0, actor(), &[birch, stock]).unwrap())
            .unwrap();
        let request = PanelRequest::new(
            "Side",
            PanelDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                source: PanelSource::Rectangle {
                    stock: stock_id,
                    plane: PanelPlane::Xy,
                    origin: [length(0.), length(0.), length(0.)],
                    length: length(600.),
                    width: length(400.),
                },
                grain: GrainAxis::U,
                edges: PanelEdges::default(),
                oversize: None,
            },
        );
        let p = model::prepare_panel(&s, s.revision(), actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        let mut form = MachiningPanel::new("Cable".into());
        assert!(
            form.request(s.document(), true)
                .unwrap_err()
                .contains("target")
        );
        form.targets = vec![request.body];
        form.kind = Kind::Drill;
        form.holes = "300, 200\nfour, 1".into();
        assert!(
            form.request(s.document(), true)
                .unwrap_err()
                .contains("Hole line 2")
        );
        form.holes = "300, 200".into();
        form.through = true;
        form.diameter = field(20.);
        let prepared = form
            .request(s.document(), true)
            .unwrap()
            .prepare(&s, actor())
            .unwrap();
        let cut = prepared.outputs[0].feature;
        let model = Compiler::default()
            .compile(prepared.prepared.document(), 2)
            .unwrap();
        let v = model.solid(cut).unwrap().volume_mm3().unwrap();
        assert!((v - (600. * 400. * 18. - std::f64::consts::PI * 100. * 18.)).abs() < 1e-3);
        s.commit(prepared.prepared).unwrap();
        let edit = MachiningPanel::existing(s.document(), cut).unwrap();
        assert_eq!(edit.kind, Kind::Drill);
        assert!(edit.through);
        assert_eq!(edit.targets, vec![request.body]);
    }
}
