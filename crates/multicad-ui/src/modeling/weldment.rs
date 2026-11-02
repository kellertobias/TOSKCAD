//! Weldment inspector: create tube and profile members from the solid lines
//! of a sketch or from a point pair, or edit one member in place. Profile
//! stock is defined in Project → Materials.
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
    materials::{self, LibraryDefinition, StockKind},
    model::{
        self, AxisAnchor, BodyOutput, ConnectionDefinition, CreatedBody, MemberDefinition,
        MemberPath, MemberRequest, ModelPrepared, SequenceDefinition, WeldDefinition, angle,
        length,
    },
};

/// The typed intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub enum WeldmentCommand {
    Members {
        requests: Vec<MemberRequest>,
        group: Option<String>,
    },
    EditMember {
        feature: EntityId,
        member: Box<MemberDefinition>,
    },
    /// Add a weld to a connection, or edit an existing weld by ID.
    Weld {
        existing: Option<EntityId>,
        weld: Box<WeldDefinition>,
    },
    /// Create the weld sequence, or replace an existing one's steps.
    Sequence {
        existing: Option<EntityId>,
        sequence: SequenceDefinition,
    },
    /// Create a connection, or edit an existing one by ID.
    Connection {
        existing: Option<EntityId>,
        name: String,
        connection: ConnectionDefinition,
    },
}
impl WeldmentCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let revision = session.revision();
        match self {
            Self::Members { requests, group } => {
                model::prepare_members(session, revision, actor, requests, group.as_deref())
            }
            Self::EditMember { feature, member } => {
                model::prepare_member_edit(session, revision, actor, *feature, member)
            }
            Self::Weld { existing, weld } => {
                let prepared = match existing {
                    Some(id) => model::prepare_weld_edit(session, revision, actor, *id, weld),
                    None => model::prepare_weld(session, revision, actor, weld).map(|(_, p)| p),
                }
                .map_err(|e| e.to_string())?;
                return Ok(ModelPrepared {
                    prepared,
                    outputs: vec![],
                });
            }
            Self::Sequence { existing, sequence } => {
                let prepared = model::prepare_sequence(
                    session,
                    revision,
                    actor,
                    *existing,
                    "Weld sequence",
                    sequence,
                )
                .map_err(|e| e.to_string())?
                .1;
                return Ok(ModelPrepared {
                    prepared,
                    outputs: vec![],
                });
            }
            Self::Connection {
                existing,
                name,
                connection,
            } => {
                let prepared = match existing {
                    Some(id) => {
                        model::prepare_connection_edit(session, revision, actor, *id, connection)
                    }
                    None => model::prepare_connection(session, revision, actor, name, connection)
                        .map(|(_, p)| p),
                }
                .map_err(|e| e.to_string())?;
                // Both connected members rebuild; preview them.
                let document = session.document();
                let outputs = connection
                    .members
                    .iter()
                    .filter_map(|part| {
                        Some(CreatedBody {
                            feature: model::member_feature(document, *part)?,
                            body: *part,
                        })
                    })
                    .collect();
                return Ok(ModelPrepared { prepared, outputs });
            }
        }
        .map_err(|e| e.to_string())
    }
    /// Member edits keep their feature.
    pub fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::EditMember { feature, .. } => Some(*feature),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathKind {
    Sketch,
    Points,
}
fn field(value: f64, unit: &'static str) -> DimensionField {
    let d = if unit == "deg" {
        angle(value)
    } else {
        length(value)
    };
    DimensionField::from_dimension(&d, unit).expect("literal field")
}
fn of(d: &Dimension, unit: &'static str) -> Result<DimensionField, String> {
    DimensionField::from_dimension(d, unit)
}
/// Profile stock entries: (stock, label).
fn profile_stocks(document: &Document) -> Vec<(EntityId, String)> {
    document
        .entities()
        .filter_map(|e| match materials::read(e).ok().flatten() {
            Some(LibraryDefinition::Stock(s)) => match s.specification {
                StockKind::Profile { shape, .. } => Some((
                    e.id,
                    format!("{} · {}", e.name, format!("{shape:?}").to_lowercase()),
                )),
                _ => None,
            },
            _ => None,
        })
        .collect()
}
fn sketches(document: &Document) -> Vec<(EntityId, String)> {
    document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
        .map(|e| (e.id, e.name.clone()))
        .collect()
}
fn combo(
    ui: &mut egui::Ui,
    id: &str,
    empty: &str,
    items: &[(EntityId, String)],
    value: &mut Option<EntityId>,
) -> bool {
    let selected = value
        .and_then(|v| items.iter().find(|(s, _)| *s == v))
        .map_or(empty.into(), |(_, n)| n.clone());
    let mut changed = false;
    controls::inspector_combo(ui, id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (s, n) in items {
                changed |= ui.selectable_value(value, Some(*s), n).changed();
            }
        });
    changed
}
fn anchor_radio(ui: &mut egui::Ui, label: &str, value: &mut AxisAnchor) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        for (a, text) in [
            (AxisAnchor::Min, "Min"),
            (AxisAnchor::Center, "Center"),
            (AxisAnchor::Max, "Max"),
        ] {
            changed |= ui.radio_value(value, a, text).changed();
        }
    });
    changed
}

pub(super) struct WeldmentPanel {
    stamp: FormStamp,
    /// The member feature being edited.
    existing: Option<EntityId>,
    body: EntityId,
    name: String,
    stock: Option<EntityId>,
    path_kind: PathKind,
    sketch: Option<EntityId>,
    /// An edited member's path, kept unless it is a point pair.
    path: Option<MemberPath>,
    start: [DimensionField; 3],
    end: [DimensionField; 3],
    anchor: [AxisAnchor; 2],
    rotation: DimensionField,
    extend_start: DimensionField,
    extend_end: DimensionField,
    group: bool,
    error: Option<String>,
}
impl WeldmentPanel {
    pub fn new(document: &Document, name: String) -> Self {
        let sketch = sketches(document).last().map(|s| s.0);
        Self {
            stamp: FormStamp::new(),
            existing: None,
            body: EntityId::new(),
            name,
            stock: profile_stocks(document).first().map(|s| s.0),
            path_kind: if sketch.is_some() {
                PathKind::Sketch
            } else {
                PathKind::Points
            },
            sketch,
            path: None,
            start: [field(0., "mm"), field(0., "mm"), field(0., "mm")],
            end: [field(1000., "mm"), field(0., "mm"), field(0., "mm")],
            anchor: [AxisAnchor::Center; 2],
            rotation: field(0., "deg"),
            extend_start: field(0., "mm"),
            extend_end: field(0., "mm"),
            group: true,
            error: None,
        }
    }
    /// Open an applied member for editing in place.
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let member = model::read_member(document, feature).map_err(|e| e.to_string())?;
        let mut form = Self::new(document, String::new());
        form.existing = Some(feature);
        form.body = member.output.body;
        form.name = document
            .get(member.output.body)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        form.stock = Some(member.stock);
        match &member.path {
            MemberPath::Points { start, end } => {
                form.path_kind = PathKind::Points;
                for i in 0..3 {
                    form.start[i] = of(&start[i], "mm")?;
                    form.end[i] = of(&end[i], "mm")?;
                }
            }
            MemberPath::SketchLine { sketch, .. } => {
                form.path_kind = PathKind::Sketch;
                form.sketch = Some(*sketch);
                form.path = Some(member.path.clone());
            }
        }
        form.anchor = member.anchor;
        if let Some(r) = &member.rotation {
            form.rotation = of(r, "deg")?;
        }
        if let Some(d) = &member.extend_start {
            form.extend_start = of(d, "mm")?;
        }
        if let Some(d) = &member.extend_end {
            form.extend_end = of(d, "mm")?;
        }
        Ok(form)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Tube / profile member"
    }
    fn optional(field: &DimensionField, document: &Document) -> Result<Option<Dimension>, String> {
        let d = field.resolve(document)?;
        Ok((!matches!(d, Dimension::Literal { value, .. } if value == 0.)).then_some(d))
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<WeldmentCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let stock = self
            .stock
            .ok_or("Choose profile stock; define it in Project → Materials")?;
        let points = || -> Result<MemberPath, String> {
            let p = |f: &[DimensionField; 3]| -> Result<[Dimension; 3], String> {
                Ok([
                    f[0].resolve(document)?,
                    f[1].resolve(document)?,
                    f[2].resolve(document)?,
                ])
            };
            Ok(MemberPath::Points {
                start: p(&self.start)?,
                end: p(&self.end)?,
            })
        };
        let rotation = self.rotation.resolve(document)?;
        let zero = matches!(rotation, Dimension::Literal { value, .. } if value == 0.);
        let template = MemberDefinition {
            output: BodyOutput { body: self.body },
            stock,
            path: MemberPath::Points {
                start: [0.; 3].map(length),
                end: [1., 0., 0.].map(length),
            },
            anchor: self.anchor,
            rotation: (!zero).then_some(rotation),
            extend_start: Self::optional(&self.extend_start, document)?,
            extend_end: Self::optional(&self.extend_end, document)?,
        };
        if let Some(feature) = self.existing {
            let path = match (&self.path, self.path_kind) {
                (Some(path), PathKind::Sketch) => path.clone(),
                _ => points()?,
            };
            return Ok(WeldmentCommand::EditMember {
                feature,
                member: Box::new(MemberDefinition { path, ..template }),
            });
        }
        let requests = match self.path_kind {
            PathKind::Sketch => {
                let sketch = self
                    .sketch
                    .ok_or("Choose the sketch holding the frame path")?;
                model::frame_member_requests(document, sketch, &self.name, &template)
                    .map_err(|e| e.to_string())?
            }
            PathKind::Points => vec![MemberRequest::new(
                self.name.clone(),
                MemberDefinition {
                    path: points()?,
                    ..template
                },
            )],
        };
        Ok(WeldmentCommand::Members {
            requests,
            group: (self.group && self.path_kind == PathKind::Sketch).then(|| self.name.clone()),
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
        let mut changed = false;
        ui.strong(self.label());
        ui.small(if self.existing.is_some() {
            "Edit this member. Its Part keeps its identity; a sketch-line member keeps following its line."
        } else {
            "Each solid line of the sketch becomes its own member Part, following the line through later sketch edits. Or place one member between two points."
        });
        ui.label(
            if self.existing.is_some() || self.path_kind == PathKind::Points {
                "Name"
            } else {
                "Name prefix"
            },
        );
        changed |= ui
            .add_enabled(
                self.existing.is_none(),
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        let stocks = profile_stocks(document);
        if stocks.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "No profile stock yet. Define tube, bar or angle stock in Project → Materials.",
            );
        }
        ui.label("Profile stock");
        changed |= combo(
            ui,
            "weld-stock",
            "Choose profile stock",
            &stocks,
            &mut self.stock,
        );
        ui.separator();
        let sketch_path = self.existing.is_some() && self.path.is_some();
        if self.existing.is_none() {
            ui.horizontal(|ui| {
                changed |= ui
                    .radio_value(&mut self.path_kind, PathKind::Sketch, "Sketch lines")
                    .changed();
                changed |= ui
                    .radio_value(&mut self.path_kind, PathKind::Points, "Two points")
                    .changed();
            });
        }
        match self.path_kind {
            PathKind::Sketch if sketch_path => {
                ui.small("Follows its sketch line; edit the sketch to move it.");
            }
            PathKind::Sketch => {
                changed |= combo(
                    ui,
                    "weld-sketch",
                    "Choose a sketch",
                    &sketches(document),
                    &mut self.sketch,
                );
                if let Some(sketch) = self.sketch {
                    match model::sketch_path_lines(document, sketch) {
                        Ok(lines) => {
                            ui.small(format!("{} solid lines become members", lines.len()));
                        }
                        Err(e) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, e.to_string());
                        }
                    }
                }
                changed |= ui
                    .checkbox(&mut self.group, "Group the members under the prefix")
                    .changed();
            }
            PathKind::Points => {
                for (axis, f) in ["X", "Y", "Z"].iter().zip(&mut self.start) {
                    changed |= f.show(ui, &format!("Start {axis}"), "World path start");
                }
                for (axis, f) in ["X", "Y", "Z"].iter().zip(&mut self.end) {
                    changed |= f.show(ui, &format!("End {axis}"), "World path end");
                }
            }
        }
        ui.separator();
        ui.label("Section placement");
        changed |= anchor_radio(ui, "Width", &mut self.anchor[0]);
        changed |= anchor_radio(ui, "Height", &mut self.anchor[1]);
        changed |= self.rotation.show(
            ui,
            "Rotation",
            "About the member axis; height starts along the sketch normal, or up",
        );
        changed |= self.extend_start.show(
            ui,
            "Extend start",
            "Extra length at the path start; negative trims",
        );
        changed |= self.extend_end.show(
            ui,
            "Extend end",
            "Extra length at the path end; negative trims",
        );
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
                    request: ModelRequest::Weldment(Box::new(request)),
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
        commands::{Channel, Request},
        constraint_sketch::source::Source,
        document::Entity,
        materials::{ProfileShape, Stock, prepare_library},
        sketch::Datum,
    };
    use multicad_model::Compiler;

    fn session() -> (Session, EntityId) {
        let mut s = Session::new(Document::new("Weldment form").unwrap()).unwrap();
        let steel = Entity::new(
            "Steel",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [120, 124, 130, 255],
                section_hatch: None,
            },
        );
        let shs = LibraryDefinition::Stock(Stock {
            material: steel.id,
            specification: StockKind::Profile {
                designation: "SHS 30".into(),
                length_mm: 6000.,
                width_mm: 30.,
                height_mm: 30.,
                wall_mm: Some(2.),
                shape: ProfileShape::Rectangular,
            },
        })
        .entity(EntityId::new(), "SHS 30")
        .unwrap();
        s.commit(prepare_library(&s, 0, Actor::local(Channel::Ui), &[steel, shs]).unwrap())
            .unwrap();
        let (sketch, edits) = Source::create(Datum::Xy, "Frame").unwrap();
        s.apply(Request {
            expected_revision: s.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Sketch".into(),
            edits,
        })
        .unwrap();
        let mut source = Source::read(s.document(), sketch).unwrap().0;
        let a = source.add_point([0., 0.]);
        let b = source.add_point([500., 0.]);
        let c = source.add_point([500., 300.]);
        source.add_line(a, b);
        source.add_line(b, c);
        let edits = source.replace(s.document(), sketch).unwrap();
        s.apply(Request {
            expected_revision: s.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Path".into(),
            edits,
        })
        .unwrap();
        (s, sketch)
    }

    #[test]
    fn form_turns_sketch_lines_into_members_and_edits_one_in_place() {
        let (mut s, sketch) = session();
        let form = WeldmentPanel::new(s.document(), "Rail".into());
        assert_eq!(form.path_kind, PathKind::Sketch);
        assert_eq!(form.sketch, Some(sketch));
        let command = form.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), None);
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(prepared.outputs.len(), 2);
        let list = Compiler::default()
            .compile(prepared.prepared.document(), s.revision() + 1)
            .unwrap()
            .member_cut_list(prepared.prepared.document())
            .unwrap();
        let rail2 = list.iter().find(|r| r.name == "Rail 2").unwrap();
        assert!((rail2.length_mm - 300.).abs() < 1e-6);
        let feature = rail2.feature;
        s.commit(prepared.prepared).unwrap();

        let mut edit = WeldmentPanel::existing(s.document(), feature).unwrap();
        assert!(edit.path.is_some());
        edit.extend_end = field(15., "mm");
        edit.anchor = [AxisAnchor::Min, AxisAnchor::Center];
        let command = edit.request(s.document(), true).unwrap();
        assert_eq!(command.edit_id(), Some(feature));
        let prepared = command.prepare(&s, Actor::local(Channel::Ui)).unwrap();
        let list = Compiler::default()
            .compile(prepared.prepared.document(), s.revision() + 1)
            .unwrap()
            .member_cut_list(prepared.prepared.document())
            .unwrap();
        let rail2 = list.iter().find(|r| r.feature == feature).unwrap();
        assert!((rail2.length_mm - 315.).abs() < 1e-6);
        assert_eq!(rail2.anchor[0], AxisAnchor::Min);

        // Without stock the form says where to define it.
        let mut empty = WeldmentPanel::new(s.document(), "X".into());
        empty.stock = None;
        assert!(
            empty
                .request(s.document(), true)
                .unwrap_err()
                .contains("Materials")
        );
    }
}
