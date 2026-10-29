//! Wall inspector: choose or define a reusable wall type, draw the path as
//! coordinates, set explicit instance overrides and place doors and windows
//! by segment and offset. New types and the wall commit in one transaction.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{
        self, BodyOutput, Hinge, Justification, ModelPrepared, NewType, NewTypeKind, Opening,
        OpeningKind, OpeningTypeDefinition, Swing, WallChange, WallDefinition, WallRequest,
        WallTypeDefinition, length,
    },
};

/// The typed wall intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub struct WallCommand {
    pub new_types: Vec<NewType>,
    pub feature: EntityId,
    pub name: String,
    pub wall: WallDefinition,
    pub editing: bool,
}
impl WallCommand {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let request;
        let change = if self.editing {
            WallChange::Edit {
                feature: self.feature,
                wall: &self.wall,
            }
        } else {
            request = WallRequest {
                feature: self.feature,
                body: self.wall.output.body,
                name: self.name.clone(),
                wall: self.wall.clone(),
            };
            WallChange::Create(&request)
        };
        model::prepare_wall_change(session, session.revision(), actor, &self.new_types, &change)
            .map_err(|e| e.to_string())
    }
    pub fn edit_id(&self) -> Option<EntityId> {
        self.editing.then_some(self.feature)
    }
}

struct OpeningRow {
    id: EntityId,
    /// An existing opening type, or a new one defined here.
    existing: Option<EntityId>,
    window: bool,
    hinge: Hinge,
    swing: Swing,
    /// One-based segment number shown to the user.
    segment: usize,
    offset: DimensionField,
    sized: bool,
    width: DimensionField,
    height: DimensionField,
    sill: DimensionField,
    new_type: EntityId,
}
impl OpeningRow {
    fn new(window: bool) -> Result<Self, String> {
        let (w, h, s) = if window {
            (1200., 1200., 900.)
        } else {
            (900., 2100., 0.)
        };
        Ok(Self {
            id: EntityId::new(),
            existing: None,
            window,
            hinge: Hinge::Start,
            swing: Swing::Left,
            segment: 1,
            offset: DimensionField::from_dimension(&length(500.), "mm")?,
            sized: false,
            width: DimensionField::from_dimension(&length(w), "mm")?,
            height: DimensionField::from_dimension(&length(h), "mm")?,
            sill: DimensionField::from_dimension(&length(s), "mm")?,
            new_type: EntityId::new(),
        })
    }
}

pub(super) struct WallPanel {
    stamp: FormStamp,
    feature: EntityId,
    body: EntityId,
    editing: bool,
    name: String,
    wall_type: Option<EntityId>,
    new_type: EntityId,
    type_name: String,
    thickness: DimensionField,
    height: DimensionField,
    justification: Justification,
    override_size: bool,
    path: String,
    closed: bool,
    base_z: DimensionField,
    openings: Vec<OpeningRow>,
    error: Option<String>,
}
fn literal(d: &Dimension) -> String {
    match d {
        Dimension::Literal { value, .. } => format!("{value}"),
        Dimension::Expression { expression } => expression.text.clone(),
    }
}
impl WallPanel {
    pub fn new(document: &Document, name: String) -> Result<Self, String> {
        let wall_type = model::wall_types(document).next().map(|e| e.id);
        Ok(Self {
            stamp: FormStamp::new(),
            feature: EntityId::new(),
            body: EntityId::new(),
            editing: false,
            name,
            wall_type,
            new_type: EntityId::new(),
            type_name: "Wall 200".into(),
            thickness: DimensionField::from_dimension(&length(200.), "mm")?,
            height: DimensionField::from_dimension(&length(2500.), "mm")?,
            justification: Justification::Center,
            override_size: false,
            path: "0, 0\n4000, 0".into(),
            closed: false,
            base_z: DimensionField::from_dimension(&length(0.), "mm")?,
            openings: vec![],
            error: None,
        })
    }
    pub fn existing(document: &Document, feature: EntityId, name: String) -> Result<Self, String> {
        let wall = model::read_wall(document, feature).map_err(|e| e.to_string())?;
        let ty = model::read_wall_type(document, wall.wall_type).map_err(|e| e.to_string())?;
        let mut panel = Self::new(document, name)?;
        panel.editing = true;
        panel.feature = feature;
        panel.body = wall.output.body;
        panel.wall_type = Some(wall.wall_type);
        panel.override_size = wall.thickness.is_some() || wall.height.is_some();
        panel.thickness =
            DimensionField::from_dimension(wall.thickness.as_ref().unwrap_or(&ty.thickness), "mm")?;
        panel.height =
            DimensionField::from_dimension(wall.height.as_ref().unwrap_or(&ty.height), "mm")?;
        panel.justification = wall.justification.unwrap_or(ty.justification);
        panel.path = wall
            .path
            .iter()
            .map(|[x, y]| format!("{}, {}", literal(x), literal(y)))
            .collect::<Vec<_>>()
            .join("\n");
        panel.closed = wall.closed;
        panel.base_z = DimensionField::from_dimension(&wall.base_z, "mm")?;
        for o in &wall.openings {
            let t =
                model::read_opening_type(document, o.opening_type).map_err(|e| e.to_string())?;
            let mut row = OpeningRow::new(matches!(t.kind, OpeningKind::Window))?;
            row.id = o.id;
            row.existing = Some(o.opening_type);
            row.segment = o.segment + 1;
            row.offset = DimensionField::from_dimension(&o.offset, "mm")?;
            row.sized = o.width.is_some() || o.height.is_some() || o.sill.is_some();
            row.width = DimensionField::from_dimension(o.width.as_ref().unwrap_or(&t.width), "mm")?;
            row.height =
                DimensionField::from_dimension(o.height.as_ref().unwrap_or(&t.height), "mm")?;
            row.sill = DimensionField::from_dimension(o.sill.as_ref().unwrap_or(&t.sill), "mm")?;
            panel.openings.push(row);
        }
        Ok(panel)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Wall"
    }
    fn points(&self) -> Result<Vec<[Dimension; 2]>, String> {
        self.path
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, line)| {
                let values: Vec<f64> = line
                    .split(',')
                    .map(|v| v.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("Path line {} must be \"x, y\" in millimeters", i + 1))?;
                match values.as_slice() {
                    [x, y] if x.is_finite() && y.is_finite() => Ok([length(*x), length(*y)]),
                    _ => Err(format!(
                        "Path line {} must be \"x, y\" in millimeters",
                        i + 1
                    )),
                }
            })
            .collect()
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<WallCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a wall name with 1–1000 bytes".into());
        }
        let mut new_types = vec![];
        let wall_type = match self.wall_type {
            Some(id) => id,
            None => {
                new_types.push(NewType {
                    id: self.new_type,
                    name: self.type_name.clone(),
                    kind: NewTypeKind::Wall(WallTypeDefinition {
                        thickness: self.thickness.resolve(document)?,
                        height: self.height.resolve(document)?,
                        justification: self.justification,
                        material: None,
                    }),
                });
                self.new_type
            }
        };
        let overrides = self.wall_type.is_some() && self.override_size;
        let mut openings = vec![];
        for row in &self.openings {
            let opening_type = match row.existing {
                Some(id) => id,
                None => {
                    new_types.push(NewType {
                        id: row.new_type,
                        name: format!(
                            "{} {}",
                            if row.window { "Window" } else { "Door" },
                            literal(&row.width.resolve(document)?)
                        ),
                        kind: NewTypeKind::Opening(OpeningTypeDefinition {
                            kind: if row.window {
                                OpeningKind::Window
                            } else {
                                OpeningKind::Door {
                                    hinge: row.hinge,
                                    swing: row.swing,
                                }
                            },
                            width: row.width.resolve(document)?,
                            height: row.height.resolve(document)?,
                            sill: row.sill.resolve(document)?,
                        }),
                    });
                    row.new_type
                }
            };
            let sized = row.existing.is_some() && row.sized;
            openings.push(Opening {
                id: row.id,
                opening_type,
                segment: row
                    .segment
                    .checked_sub(1)
                    .ok_or("Segments are numbered from 1")?,
                offset: row.offset.resolve(document)?,
                width: sized.then(|| row.width.resolve(document)).transpose()?,
                height: sized.then(|| row.height.resolve(document)).transpose()?,
                sill: sized.then(|| row.sill.resolve(document)).transpose()?,
            });
        }
        Ok(WallCommand {
            new_types,
            feature: self.feature,
            name: self.name.clone(),
            editing: self.editing,
            wall: WallDefinition {
                output: BodyOutput { body: self.body },
                wall_type,
                path: self.points()?,
                closed: self.closed,
                base_z: self.base_z.resolve(document)?,
                thickness: overrides
                    .then(|| self.thickness.resolve(document))
                    .transpose()?,
                height: overrides
                    .then(|| self.height.resolve(document))
                    .transpose()?,
                justification: self.wall_type.is_some().then_some(self.justification),
                openings,
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
        ui.strong("Wall");
        ui.small(if self.editing {
            "Edit this wall's path, overrides and openings. Its Part and opening identities are kept."
        } else {
            "Draw a wall of a reusable type. Preview builds the mitered solid with cut openings before Apply."
        });
        ui.label("Wall name");
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        ui.separator();
        ui.label("Wall type");
        let types: Vec<_> = model::wall_types(document)
            .map(|e| (e.id, e.name.clone()))
            .collect();
        let selected = self
            .wall_type
            .and_then(|id| types.iter().find(|(t, _)| *t == id).map(|(_, n)| n.clone()))
            .unwrap_or_else(|| "New type…".into());
        controls::inspector_combo(ui, "wall-type")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (id, name) in &types {
                    changed |= ui
                        .selectable_value(&mut self.wall_type, Some(*id), name)
                        .changed();
                }
                changed |= ui
                    .selectable_value(&mut self.wall_type, None, "New type…")
                    .changed();
            });
        match self.wall_type {
            None => {
                ui.label("Type name");
                changed |= ui.text_edit_singleline(&mut self.type_name).changed();
                changed |= self
                    .thickness
                    .show(ui, "Thickness", "Saved with the new reusable type");
                changed |= self
                    .height
                    .show(ui, "Height", "Saved with the new reusable type");
            }
            Some(id) => {
                if let Ok(t) = model::read_wall_type(document, id) {
                    ui.small(format!(
                        "Type: {} thick · {} high · {:?}. Editing the type later updates every wall that does not override it.",
                        literal(&t.thickness),
                        literal(&t.height),
                        t.justification
                    ));
                }
                changed |= ui
                    .checkbox(
                        &mut self.override_size,
                        "Override thickness and height for this wall",
                    )
                    .changed();
                if self.override_size {
                    changed |= self.thickness.show(ui, "Thickness", "This wall only");
                    changed |= self.height.show(ui, "Height", "This wall only");
                }
            }
        }
        ui.label("Justification");
        controls::inspector_combo(ui, "wall-justification")
            .selected_text(format!("{:?}", self.justification))
            .show_ui(ui, |ui| {
                for j in [
                    Justification::Center,
                    Justification::Left,
                    Justification::Right,
                ] {
                    changed |= ui
                        .selectable_value(&mut self.justification, j, format!("{j:?}"))
                        .changed();
                }
            })
            .response
            .on_hover_text("Side of the drawn path the wall occupies, looking along the path");
        ui.separator();
        ui.label("Path points (x, y in mm, one per line)");
        changed |= ui
            .add(
                egui::TextEdit::multiline(&mut self.path)
                    .desired_rows(4)
                    .desired_width(ui.available_width()),
            )
            .changed();
        changed |= ui.checkbox(&mut self.closed, "Close into a room").changed();
        changed |= self
            .base_z
            .show(ui, "Base elevation", "Height of the wall base above Z = 0");
        ui.separator();
        ui.strong("Doors and windows");
        let opening_types: Vec<_> = model::opening_types(document)
            .map(|e| (e.id, e.name.clone()))
            .collect();
        let mut remove = None;
        for (k, row) in self.openings.iter_mut().enumerate() {
            ui.push_id(row.id, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{}.", k + 1));
                    let label = row
                        .existing
                        .and_then(|id| {
                            opening_types
                                .iter()
                                .find(|(t, _)| *t == id)
                                .map(|(_, n)| n.clone())
                        })
                        .unwrap_or_else(|| {
                            if row.window {
                                "New window type".into()
                            } else {
                                "New door type".into()
                            }
                        });
                    controls::inspector_combo(ui, "opening-type")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (id, name) in &opening_types {
                                changed |= ui
                                    .selectable_value(&mut row.existing, Some(*id), name)
                                    .changed();
                            }
                            changed |= ui
                                .selectable_value(&mut row.existing, None, "New type…")
                                .changed();
                        });
                    if ui.small_button("Remove").clicked() {
                        remove = Some(k);
                    }
                });
                if row.existing.is_none() {
                    ui.horizontal(|ui| {
                        changed |= ui.radio_value(&mut row.window, false, "Door").changed();
                        changed |= ui.radio_value(&mut row.window, true, "Window").changed();
                    });
                    if !row.window {
                        ui.horizontal(|ui| {
                            changed |= ui
                                .radio_value(&mut row.hinge, Hinge::Start, "Hinge at start")
                                .changed();
                            changed |= ui
                                .radio_value(&mut row.hinge, Hinge::End, "Hinge at end")
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            changed |= ui
                                .radio_value(&mut row.swing, Swing::Left, "Opens left")
                                .changed();
                            changed |= ui
                                .radio_value(&mut row.swing, Swing::Right, "Opens right")
                                .changed();
                        });
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Segment");
                    changed |= ui
                        .add(egui::DragValue::new(&mut row.segment).range(1..=256))
                        .changed();
                });
                changed |= row.offset.show(
                    ui,
                    "Offset along segment",
                    "From the segment start point to the opening's start jamb",
                );
                if row.existing.is_some() {
                    changed |= ui
                        .checkbox(&mut row.sized, "Override size for this opening")
                        .changed();
                }
                if row.existing.is_none() || row.sized {
                    changed |= row.width.show(ui, "Width", "Opening width along the wall");
                    changed |= row
                        .height
                        .show(ui, "Height", "Opening height above its sill");
                    changed |= row.sill.show(
                        ui,
                        "Sill",
                        "Height of the opening bottom above the wall base",
                    );
                }
            });
            ui.separator();
        }
        if let Some(k) = remove {
            self.openings.remove(k);
            changed = true;
        }
        ui.horizontal(|ui| {
            for (label, window) in [("Add door", false), ("Add window", true)] {
                if ui.button(label).clicked()
                    && let Ok(row) = OpeningRow::new(window)
                {
                    self.openings.push(row);
                    changed = true;
                }
            }
        });
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
                    request: ModelRequest::Wall(Box::new(request)),
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

    #[test]
    fn panel_defines_types_and_a_wall_that_commit_and_reopen_for_editing() {
        let mut session = Session::new(Document::new("Panel").unwrap()).unwrap();
        let mut panel = WallPanel::new(session.document(), "Wall 1".into()).unwrap();
        assert!(
            panel.wall_type.is_none(),
            "an empty project starts with a new type"
        );
        panel.path = "0, 0\n4000, 0\n4000, 3000".into();
        panel.openings.push(OpeningRow::new(false).unwrap());
        let mut window = OpeningRow::new(true).unwrap();
        window.segment = 2;
        window.offset = DimensionField::from_dimension(&length(1000.), "mm").unwrap();
        panel.openings.push(window);
        let command = panel.request(session.document(), true).unwrap();
        assert_eq!(command.new_types.len(), 3);
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        let model = Compiler::default()
            .compile(prepared.prepared.document(), 0)
            .unwrap();
        let feature = prepared.outputs[0].feature;
        let volume = model.snapshot().shapes[&feature].volume_mm3;
        assert!(
            (volume - (7000. * 200. * 2500. - 900. * 200. * 2100. - 1200. * 200. * 1200.)).abs()
                < 1e-3
        );
        session.commit(prepared.prepared).unwrap();

        // Reopened for editing, the same panel reuses the saved types.
        let mut edit = WallPanel::existing(session.document(), feature, "Wall 1".into()).unwrap();
        assert!(edit.openings.iter().all(|o| o.existing.is_some()));
        edit.override_size = true;
        edit.height = DimensionField::from_dimension(&length(3000.), "mm").unwrap();
        let command = edit.request(session.document(), true).unwrap();
        assert!(command.new_types.is_empty());
        assert_eq!(command.edit_id(), Some(feature));
        let prepared = command
            .prepare(&session, Actor::local(Channel::Ui))
            .unwrap();
        let model = Compiler::default()
            .compile(prepared.prepared.document(), 1)
            .unwrap();
        let volume = model.snapshot().shapes[&feature].volume_mm3;
        assert!(
            (volume - (7000. * 200. * 3000. - 900. * 200. * 2100. - 1200. * 200. * 1200.)).abs()
                < 1e-3
        );

        // Malformed path lines are reported, not guessed.
        edit.path = "0, 0\nfour, 0".into();
        assert!(
            edit.request(session.document(), true)
                .unwrap_err()
                .contains("Path line 2")
        );
    }
}
