//! Wall inspector: choose or define a reusable wall type, draw the path as
//! the viewport, set explicit instance overrides and place doors and windows
//! by segment and offset. New types and the wall commit in one transaction.
use super::{
    FormStamp, PreviewState, TerminationViewport,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
    outline::{OutlineEditor, OutlineKind, plan_targets},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, PreparationSnapshot, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{
        self, BodyOutput, Hinge, Justification, ModelPrepared, NewType, NewTypeKind, Opening,
        OpeningKind, OpeningTypeDefinition, Swing, WallChange, WallDefinition, WallRequest,
        WallTypeDefinition, length,
    },
    parameters::Unit,
};

/// The typed wall intent sent to native preview and Apply.
#[derive(Clone, Debug)]
pub struct WallCommand {
    pub new_types: Vec<NewType>,
    pub construction: Option<model::WallConstructionEdit>,
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
        model::prepare_wall_construction_change(
            session,
            session.revision(),
            actor,
            &self.new_types,
            &change,
            self.construction.as_ref(),
        )
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
    symbol: crate::symbols::architecture::Binding,
    feature: EntityId,
    body: EntityId,
    editing: bool,
    name: String,
    wall_type: Option<EntityId>,
    new_type: EntityId,
    type_name: String,
    type_height: DimensionField,
    layers: Option<super::wall_layers::LayerEditor>,
    shared_edit: bool,
    layer_placement: model::WallLayerPlacement,
    junction_editor: super::wall_junctions::JunctionEditor,
    types: Vec<(EntityId, String)>,
    source: Option<PreparationSnapshot>,
    impacted: Option<(EntityId, Result<Vec<String>, String>)>,
    thickness: DimensionField,
    height: DimensionField,
    justification: Justification,
    override_size: bool,
    outline: OutlineEditor,
    /// Snap targets from other walls and slabs, cached per document revision.
    targets: Option<(u64, Vec<[f64; 2]>)>,
    level: Option<EntityId>,
    levels: Vec<(EntityId, String)>,
    base_z: DimensionField,
    openings: Vec<OpeningRow>,
    /// Vertical cutouts are edited with their Room tool; a wall edit keeps them.
    cutouts: Vec<model::VerticalCutout>,
    error: Option<String>,
}
fn literal(d: &Dimension) -> String {
    match d {
        Dimension::Literal { value, .. } => format!("{value}"),
        Dimension::Expression { expression } => expression.text.clone(),
    }
}
impl WallPanel {
    pub(super) fn seed_new_level(&mut self, level: Option<EntityId>) {
        if !self.editing {
            self.level = level;
        }
    }
    pub fn new(document: &Document, name: String) -> Result<Self, String> {
        let wall_type = model::wall_types(document).next().map(|e| e.id);
        let feature = EntityId::new();
        let body = EntityId::new();
        let symbol = crate::symbols::architecture::Binding::new(
            document,
            multicad_core::symbols::architecture::ArchitectureOutput::Wall { feature, body },
        )?;
        let mut panel = Self {
            stamp: FormStamp::new(),
            symbol,
            feature,
            body,
            editing: false,
            name,
            wall_type,
            new_type: EntityId::new(),
            type_name: "Wall 200".into(),
            type_height: DimensionField::from_dimension(&length(2500.), "mm")?,
            layers: None,
            shared_edit: false,
            layer_placement: Default::default(),
            junction_editor: Default::default(),
            types: model::wall_types(document)
                .map(|e| (e.id, e.name.clone()))
                .collect(),
            source: None,
            impacted: None,
            thickness: DimensionField::from_dimension(&length(200.), "mm")?,
            height: DimensionField::from_dimension(&length(2500.), "mm")?,
            justification: Justification::Center,
            override_size: false,
            outline: OutlineEditor::from_mm(OutlineKind::WallPath, &[[0., 0.], [4000., 0.]], false),
            targets: None,
            level: model::levels(document).next().map(|e| e.id),
            levels: model::levels(document)
                .map(|e| (e.id, e.name.clone()))
                .collect(),
            base_z: DimensionField::from_dimension(&length(0.), "mm")?,
            openings: vec![],
            cutouts: vec![],
            error: None,
        };
        let owned = multicad_core::symbols::owned_entities(document).map_err(|e| e.to_string())?;
        panel.types.retain(|(id, _)| !owned.contains(id));
        if panel.wall_type.is_some_and(|id| owned.contains(&id)) {
            panel.wall_type = panel.types.first().map(|(id, _)| *id);
        }
        panel.load_type(document)?;
        if panel.wall_type.is_none()
            && let Some(id) = panel.symbol.first()
        {
            panel.symbol.select(document, Some(id))?;
        }
        Ok(panel)
    }
    fn load_type(&mut self, document: &Document) -> Result<(), String> {
        self.shared_edit = false;
        self.impacted = None;
        let Some(id) = self.wall_type else {
            return Ok(());
        };
        self.type_name = document
            .get(id)
            .ok_or("Wall construction is missing")?
            .name
            .clone();
        match model::read_wall_construction(document, id).map_err(|e| e.to_string())? {
            model::WallTypeConstruction::Uniform(ty) => {
                self.layers = None;
                self.thickness = DimensionField::from_dimension(&ty.thickness, "mm")?;
                self.height = DimensionField::from_dimension(&ty.height, "mm")?;
                self.type_height = self.height.clone();
                self.justification = ty.justification;
            }
            model::WallTypeConstruction::Layered(ty) => {
                self.layers = Some(super::wall_layers::LayerEditor::existing(
                    document,
                    &ty.layers,
                    ty.reference,
                )?);
                self.type_height = DimensionField::from_dimension(&ty.height, "mm")?;
                self.height = self.type_height.clone();
            }
        }
        if let (Some(editor), Some(source)) = (&mut self.layers, &self.source) {
            editor.sync_source(source.session());
        }
        Ok(())
    }
    pub fn select_definition(
        &mut self,
        document: &Document,
        definition: EntityId,
    ) -> Result<(), String> {
        self.symbol.select(document, Some(definition))?;
        self.override_size = false;
        self.shared_edit = false;
        self.layer_placement.reference = None;
        self.stamp.changed();
        self.error = None;
        Ok(())
    }
    pub fn sync_source(&mut self, session: &Session) {
        self.symbol.sync_source(session);
        self.outline.sync_source(session);
        self.junction_editor.sync_source(session, self.feature);
        if self
            .source
            .as_ref()
            .is_none_or(|s| s.session().stamp() != session.stamp())
        {
            let owned =
                multicad_core::symbols::owned_entities_in_session(session).unwrap_or_default();
            self.types = model::wall_types(session.document())
                .filter(|e| !owned.contains(&e.id))
                .map(|e| (e.id, e.name.clone()))
                .collect();
            self.levels = model::levels(session.document())
                .map(|e| (e.id, e.name.clone()))
                .collect();
            self.source = Some(session.preparation_snapshot());
            self.impacted = None;
        }
        if let Some(editor) = &mut self.layers {
            editor.sync_source(session);
        }
    }
    pub fn existing(document: &Document, feature: EntityId, name: String) -> Result<Self, String> {
        let wall = model::read_wall(document, feature).map_err(|e| e.to_string())?;
        let ty =
            model::read_wall_construction(document, wall.wall_type).map_err(|e| e.to_string())?;
        let mut panel = Self::new(document, name)?;
        panel.editing = true;
        panel.feature = feature;
        panel.body = wall.output.body;
        if let Some(binding) = crate::symbols::architecture::Binding::existing(document, feature)? {
            panel.symbol = binding;
        }
        panel.wall_type = Some(wall.wall_type);
        panel.load_type(document)?;
        panel.override_size = wall.thickness.is_some() || wall.height.is_some();
        match ty {
            model::WallTypeConstruction::Uniform(ty) => {
                panel.thickness = DimensionField::from_dimension(
                    wall.thickness.as_ref().unwrap_or(&ty.thickness),
                    "mm",
                )?;
                panel.height = DimensionField::from_dimension(
                    wall.height.as_ref().unwrap_or(&ty.height),
                    "mm",
                )?;
                panel.justification = wall.justification.unwrap_or(ty.justification);
            }
            model::WallTypeConstruction::Layered(ty) => {
                panel.height = DimensionField::from_dimension(
                    wall.height.as_ref().unwrap_or(&ty.height),
                    "mm",
                )?;
                panel.layer_placement = wall
                    .layer_placement
                    .clone()
                    .ok_or("Layer placement is missing")?;
            }
        }
        panel.outline = OutlineEditor::new(OutlineKind::WallPath, &wall.path, wall.closed)?;
        panel.level = wall.level;
        panel.base_z = DimensionField::from_dimension(&wall.base_z, "mm")?;
        panel.cutouts = wall.cutouts.clone();
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
    /// World height of the plane the path is drawn on: level plus base.
    fn plane_z(&self, document: &Document, revision: Option<u64>) -> f64 {
        let rebuilt;
        let parameters = if let Some(source) = self.source.as_ref().filter(|s| {
            s.session().document().id() == document.id()
                && revision.is_none_or(|r| r == s.session().revision())
        }) {
            source.session().parameters()
        } else {
            rebuilt = match multicad_core::parameters::evaluate(document) {
                Ok(p) => p,
                Err(_) => return 0.,
            };
            &rebuilt
        };
        let level = self
            .level
            .and_then(|l| model::resolve_level(document, parameters, l).ok())
            .map_or(0., |l| l.elevation_mm);
        let base = self
            .base_z
            .resolve(document)
            .ok()
            .and_then(|d| d.value_in(Unit::Millimeter, Some(parameters)).ok())
            .unwrap_or(0.);
        level + base
    }
    /// Draw and edit the path on the plan; returns whether inputs changed.
    pub fn viewport(
        &mut self,
        document: &Document,
        revision: u64,
        view: TerminationViewport<'_>,
    ) -> bool {
        if self.targets.as_ref().is_none_or(|(r, _)| *r != revision) {
            let exclude = self.editing.then_some(self.feature);
            self.targets = Some((revision, plan_targets(document, exclude)));
        }
        let targets = self
            .targets
            .as_ref()
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        let z = self.plane_z(document, Some(revision));
        let changed = self.outline.viewport(document, &view, z, &targets);
        if changed {
            self.stamp.changed();
            self.error = None;
        }
        changed
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<WallCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a wall name with 1–1000 bytes".into());
        }
        let mut new_types = vec![];
        let wall_type = if let Some(construction) = self.symbol.construction() {
            construction
        } else {
            match self.wall_type {
                Some(id) => id,
                None => {
                    new_types.push(NewType {
                        id: self.new_type,
                        name: self.type_name.clone(),
                        kind: if let Some(editor) = &self.layers {
                            NewTypeKind::LayeredWall(model::LayeredWallTypeDefinition {
                                layers: editor.stack(document)?,
                                height: self.type_height.resolve(document)?,
                                reference: editor.reference,
                            })
                        } else {
                            NewTypeKind::Wall(WallTypeDefinition {
                                thickness: self.thickness.resolve(document)?,
                                height: self.height.resolve(document)?,
                                justification: self.justification,
                                material: None,
                            })
                        },
                    });
                    self.new_type
                }
            }
        };
        let layered = self.symbol.active() || self.layers.is_some();
        if !layered && !self.layer_placement.junctions.is_empty() {
            return Err("Remove joined ends explicitly before using a uniform construction".into());
        }
        let overrides =
            !self.symbol.active() && self.override_size && (self.wall_type.is_some() || layered);
        let construction = if self.shared_edit && !self.symbol.active() {
            let editor = self
                .layers
                .as_ref()
                .ok_or("Only layered constructions have shared layer editing")?;
            Some(model::WallConstructionEdit {
                id: self
                    .wall_type
                    .ok_or("Choose the shared construction to edit")?,
                name: self.type_name.clone(),
                value: model::LayeredWallTypeDefinition {
                    layers: editor.stack(document)?,
                    height: self.type_height.resolve(document)?,
                    reference: editor.reference,
                },
            })
        } else {
            None
        };
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
            construction,
            feature: self.feature,
            name: self.name.clone(),
            editing: self.editing,
            wall: WallDefinition {
                layer_placement: layered.then(|| self.layer_placement.clone()),
                output: BodyOutput { body: self.body },
                wall_type,
                path: self.outline.dimensions(document)?,
                closed: self.outline.closed(),
                level: self.level,
                base_z: self.base_z.resolve(document)?,
                thickness: (overrides && !layered)
                    .then(|| self.thickness.resolve(document))
                    .transpose()?,
                height: overrides
                    .then(|| self.height.resolve(document))
                    .transpose()?,
                justification: (self.wall_type.is_some() && !layered).then_some(self.justification),
                openings,
                cutouts: self.cutouts.clone(),
            },
        })
    }
    pub fn model_request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<ModelRequest, String> {
        let command = self.request(document, enabled)?;
        if !self.symbol.active() {
            return Ok(ModelRequest::Wall(Box::new(command)));
        }
        let new_opening_types = command
            .new_types
            .iter()
            .filter_map(|ty| match &ty.kind {
                NewTypeKind::Opening(value) => Some(
                    multicad_core::symbols::architecture::ArchitectureOpeningTypeRequest {
                        id: ty.id,
                        name: ty.name.clone(),
                        value: value.clone(),
                    },
                ),
                _ => None,
            })
            .collect();
        let context = multicad_core::symbols::architecture::ArchitectureContext::Wall {
            path: command.wall.path,
            closed: command.wall.closed,
            level: command.wall.level,
            base_z: command.wall.base_z,
            flipped: self.layer_placement.flipped,
            details: Some(
                multicad_core::symbols::architecture::ArchitectureWallDetails {
                    openings: Some(command.wall.openings),
                    cutouts: Some(command.wall.cutouts),
                    junctions: Some(self.layer_placement.junctions.clone()),
                    new_opening_types,
                },
            ),
        };
        self.symbol
            .command(document, &self.name, context)
            .map(|command| ModelRequest::SymbolConstruction(Box::new(command)))
    }
    fn construction_controls(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let mut changed = false;
        if self.wall_type.is_none() {
            ui.label("Type name");
            changed |= ui.text_edit_singleline(&mut self.type_name).changed();
            let mut layered = self.layers.is_some();
            if ui.checkbox(&mut layered, "Layered construction").changed() {
                if layered {
                    match super::wall_layers::LayerEditor::new(document) {
                        Ok(mut editor) => {
                            if let Some(source) = &self.source {
                                editor.sync_source(source.session());
                            }
                            self.layers = Some(editor);
                            self.type_height = self.height.clone();
                        }
                        Err(error) => self.error = Some(error),
                    }
                } else {
                    self.layers = None;
                }
                changed = true;
            }
        }
        if self.layers.is_some() {
            if let Some(id) = self.wall_type {
                if !self.shared_edit && ui.button("Edit shared construction").clicked() {
                    self.shared_edit = true;
                    changed = true;
                }
                if self.shared_edit {
                    ui.label("Shared type name");
                    changed |= ui.text_edit_singleline(&mut self.type_name).changed();
                    if self.impacted.as_ref().is_none_or(|(ty, _)| *ty != id) {
                        self.impacted = Some((
                            id,
                            model::wall_construction_users(document, id)
                                .map(|users| {
                                    users
                                        .into_iter()
                                        .map(|user| {
                                            document.get(user.body).map_or_else(
                                                || "Missing wall".into(),
                                                |e| e.name.clone(),
                                            )
                                        })
                                        .collect()
                                })
                                .map_err(|e| e.to_string()),
                        ));
                    }
                    match &self.impacted.as_ref().expect("populated above").1 {
                        Ok(users) => {
                            ui.strong(format!("Shared edit: {} existing wall(s)", users.len()));
                            for name in users {
                                ui.small(name);
                            }
                            ui.small("Preview checks the affected model. Height and reference overrides remain on their instances. Cancel leaves all walls unchanged.");
                        }
                        Err(error) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                    }
                    if ui.button("Discard construction edits").clicked() {
                        let instance_height = self.override_size.then(|| self.height.clone());
                        if let Err(error) = self.load_type(document) {
                            self.error = Some(error);
                        }
                        if let Some(height) = instance_height {
                            self.height = height;
                        }
                        changed = true;
                    }
                }
                if ui.button("Duplicate as a separate type").clicked() {
                    self.wall_type = None;
                    self.new_type = EntityId::new();
                    self.shared_edit = false;
                    while self.type_name.len() > 251 {
                        self.type_name.pop();
                    }
                    self.type_name.push_str(" copy");
                    changed = true;
                }
            }
            let editable = self.wall_type.is_none() || self.shared_edit;
            if editable {
                changed |= self.type_height.show(
                    ui,
                    "Type height",
                    "Shared construction height; instance overrides remain independent",
                );
            }
            let editor = self.layers.as_mut().expect("checked above");
            changed |= editor.show(
                ui,
                document,
                self.layer_placement.flipped,
                self.layer_placement.reference,
                editable,
            );
            changed |= ui
                .checkbox(
                    &mut self.layer_placement.flipped,
                    "Flip construction across path",
                )
                .changed();
            ui.label("This wall's path reference");
            let references = editor.references();
            controls::inspector_combo(ui, "wall-layer-reference")
                .selected_text(
                    self.layer_placement
                        .reference
                        .and_then(|r| {
                            references
                                .iter()
                                .find(|(v, _)| *v == r)
                                .map(|(_, name)| name.as_str())
                        })
                        .unwrap_or(if self.layer_placement.reference.is_none() {
                            "Use construction reference"
                        } else {
                            "Missing boundary — repair required"
                        }),
                )
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.layer_placement.reference,
                            None,
                            "Use construction reference",
                        )
                        .changed();
                    for (reference, label) in &references {
                        changed |= ui
                            .selectable_value(
                                &mut self.layer_placement.reference,
                                Some(*reference),
                                label,
                            )
                            .changed();
                    }
                });
            changed |= ui
                .checkbox(&mut self.override_size, "Override height for this wall")
                .changed();
            if self.override_size {
                changed |= self.height.show(ui, "Wall height", "This wall only");
            }
        } else {
            if self.wall_type.is_none() {
                changed |= self
                    .thickness
                    .show(ui, "Thickness", "Saved with the new reusable type");
                changed |= self
                    .height
                    .show(ui, "Height", "Saved with the new reusable type");
            } else {
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
            ui.label("Justification");
            controls::inspector_combo(ui, "wall-justification")
                .selected_text(format!("{:?}", self.justification))
                .show_ui(ui, |ui| {
                    for justification in [
                        Justification::Center,
                        Justification::Left,
                        Justification::Right,
                    ] {
                        changed |= ui
                            .selectable_value(
                                &mut self.justification,
                                justification,
                                format!("{justification:?}"),
                            )
                            .changed();
                    }
                });
        }
        changed
    }
    fn junction_controls(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        let (changed, align) = self.junction_editor.show(
            ui,
            &mut self.layer_placement.junctions,
            self.symbol.active() || self.layers.is_some(),
            self.outline.closed(),
        );
        if let Some(index) = align {
            let result = (|| -> Result<(), String> {
                let path = self.outline.dimensions(document)?;
                let rebuilt;
                let parameters = if let Some(source) = &self.source {
                    source.session().parameters()
                } else {
                    rebuilt =
                        multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
                    &rebuilt
                };
                let junction = &self.layer_placement.junctions[index];
                let point = junction
                    .aligned_endpoint(document, parameters, &path, self.outline.closed())
                    .map_err(|e| e.to_string())?;
                let corner = match junction.end {
                    model::WallEnd::Start => 0,
                    model::WallEnd::End => path.len() - 1,
                };
                self.outline.set_corner(corner, point);
                Ok(())
            })();
            self.junction_editor.notice = result.err();
        }
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
        match self.symbol.show_choice(ui, document) {
            Ok(value) => changed |= value,
            Err(error) => self.error = Some(error),
        }
        if self.symbol.active() {
            changed |= self.symbol.show_fields(ui);
            changed |= ui
                .checkbox(
                    &mut self.layer_placement.flipped,
                    "Flip construction across path",
                )
                .changed();
        } else {
            ui.label("Wall type");
            let types = &self.types;
            let previous_type = self.wall_type;
            let selected = self
                .wall_type
                .and_then(|id| types.iter().find(|(t, _)| *t == id).map(|(_, n)| n.clone()))
                .unwrap_or_else(|| "New type…".into());
            ui.add_enabled_ui(!self.shared_edit, |ui| {
                controls::inspector_combo(ui, "wall-type")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (id, name) in types {
                            changed |= ui
                                .selectable_value(&mut self.wall_type, Some(*id), name)
                                .changed();
                        }
                        changed |= ui
                            .selectable_value(&mut self.wall_type, None, "New type…")
                            .changed();
                    });
            });
            if previous_type != self.wall_type {
                if let Err(error) = self.load_type(document) {
                    self.error = Some(error);
                }
                self.override_size = false;
            }
            changed |= self.construction_controls(ui, document);
        }
        ui.separator();
        ui.label("Hosting level");
        controls::inspector_combo(ui, "wall-level")
            .selected_text(
                self.level
                    .and_then(|id| self.levels.iter().find(|(key, _)| *key == id))
                    .map_or("World plane", |(_, name)| name.as_str()),
            )
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut self.level, None, "World plane")
                    .changed();
                for (id, name) in &self.levels {
                    changed |= ui
                        .selectable_value(&mut self.level, Some(*id), name)
                        .changed();
                }
            });
        changed |= self.base_z.show(
            ui,
            "Base elevation",
            "Height of the wall base above its level",
        );
        let plane = format!(
            "the wall base plane (z = {} mm)",
            self.plane_z(document, None)
        );
        changed |= self.outline.show(ui, document, &plane);
        changed |= self.junction_controls(ui, document);
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
        if !self.cutouts.is_empty() {
            ui.small(format!(
                "{} vertical cutout(s) are kept. Edit them with Room → Vertical cutout; a path change that no longer fits them is refused.",
                self.cutouts.len()
            ));
        }
        ui.separator();
        form::preview_status(ui, self.stamp, preview);
        let validation = self.model_request(document, enabled);
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
            Some(FormAction::Preview) => match self.model_request(document, enabled) {
                Ok(request) => Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request,
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply)
                if preview.ready(self.stamp) && self.model_request(document, enabled).is_ok() =>
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
    fn painted_join_controls_align_only_the_endpoint_and_preview_real_t_contact() {
        use multicad_core::{
            document::{Entity, EntityData, Mutation},
            model::{
                LayeredWallTypeDefinition, WallLayer, WallLayerKind, WallLayerReference,
                WallLayerStack,
            },
        };
        let material = Entity::new(
            "Brick",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [160, 80, 60, 255],
                section_hatch: None,
            },
        );
        let mut session = Session::new(
            Document::new("Join form")
                .unwrap()
                .edited(&[Mutation::Insert(material.clone())])
                .unwrap(),
        )
        .unwrap();
        let actor = || Actor::local(Channel::Ui);
        let (ty, p) = model::prepare_layered_wall_type(
            &session,
            0,
            actor(),
            "Construction",
            &LayeredWallTypeDefinition {
                layers: WallLayerStack {
                    layers: vec![WallLayer {
                        id: EntityId::new(),
                        name: "Brick".into(),
                        role: None,
                        construction: WallLayerKind::Solid {
                            material: material.id,
                            thickness: length(150.),
                        },
                    }],
                },
                height: length(2700.),
                reference: WallLayerReference::Center,
            },
        )
        .unwrap();
        session.commit(p).unwrap();
        let host = model::WallRequest::new(
            "Main",
            WallDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                wall_type: ty,
                layer_placement: Some(Default::default()),
                path: vec![[length(0.), length(0.)], [length(4000.), length(0.)]],
                closed: false,
                level: None,
                base_z: length(0.),
                thickness: None,
                height: None,
                justification: None,
                openings: vec![],
                cutouts: vec![],
            },
        );
        let p = model::prepare_wall(&session, session.revision(), actor(), &host).unwrap();
        session.commit(p.prepared).unwrap();
        let mut panel = WallPanel::new(session.document(), "Branch".into()).unwrap();
        panel
            .outline
            .set_points(&[[2000., 3000.], [2050., 80.]], false);
        panel.sync_source(&session);
        let before = session.document().clone();
        let original_start = panel.outline.dimensions(session.document()).unwrap()[0].clone();
        let ctx = egui::Context::default();
        let frame = |panel: &mut WallPanel, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340., 3000.),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    panel.show(ui, session.document(), true, PreviewState::Idle);
                },
            );
            output.textures_delta.clear();
            output
        };
        let click = |panel: &mut WallPanel, label: &str| {
            let output = frame(panel, vec![]);
            let rect = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        (text.galley.text().replace('\n', " ") == label)
                            .then(|| egui::Rect::from_min_size(text.pos, text.galley.size()))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("painted control missing: {label}"));
            for pressed in [true, false] {
                frame(
                    panel,
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        click(&mut panel, "Add T at end");
        assert_eq!(panel.layer_placement.junctions.len(), 1);
        assert!(
            panel
                .request(session.document(), true)
                .unwrap()
                .prepare(&session, actor())
                .is_err()
        );
        click(&mut panel, "Align endpoint perpendicular to host");
        let points = panel.outline.dimensions(session.document()).unwrap();
        assert_eq!(points[0], original_start);
        assert!(
            (points[1][0]
                .value_in(Unit::Millimeter, Some(session.parameters()))
                .unwrap()
                - 2000.)
                .abs()
                < 1e-6
        );
        assert!(
            points[1][1]
                .value_in(Unit::Millimeter, Some(session.parameters()))
                .unwrap()
                .abs()
                < 1e-6
        );
        let request = panel.request(session.document(), true).unwrap();
        let p = request.prepare(&session, actor()).unwrap();
        let native = Compiler::default()
            .compile(p.prepared.document(), session.revision())
            .unwrap();
        assert!(
            (native.snapshot().shapes[&panel.feature].volume_mm3 - 150. * 2925. * 2700.).abs()
                < 0.01
        );
        assert_eq!(
            session.document(),
            &before,
            "clicks and native preview cannot commit the draft"
        );
    }

    #[test]
    fn layered_form_previews_shared_edits_and_duplicates_without_flattening_saved_walls() {
        use multicad_core::{
            document::{Entity, EntityData, Mutation},
            model::{
                LayeredWallTypeDefinition, WallLayer, WallLayerKind, WallLayerReference,
                WallLayerStack,
            },
            project_file::{self, ProjectFile},
        };
        let material = Entity::new(
            "Brick",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [160, 80, 60, 255],
                section_hatch: None,
            },
        );
        let document = Document::new("Layered forms")
            .unwrap()
            .edited(&[Mutation::Insert(material.clone())])
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let actor = || Actor::local(Channel::Ui);
        let mut value = LayeredWallTypeDefinition {
            layers: WallLayerStack {
                layers: vec![
                    WallLayer {
                        id: EntityId::new(),
                        name: "Brick".into(),
                        role: None,
                        construction: WallLayerKind::Solid {
                            material: material.id,
                            thickness: length(100.),
                        },
                    },
                    WallLayer {
                        id: EntityId::new(),
                        name: "Air".into(),
                        role: None,
                        construction: WallLayerKind::Cavity {
                            thickness: length(30.),
                        },
                    },
                    WallLayer {
                        id: EntityId::new(),
                        name: "Lining".into(),
                        role: None,
                        construction: WallLayerKind::Solid {
                            material: material.id,
                            thickness: length(20.),
                        },
                    },
                ],
            },
            height: length(2700.),
            reference: WallLayerReference::Outside,
        };
        let (ty, p) =
            model::prepare_layered_wall_type(&session, 0, actor(), "Shared exterior", &value)
                .unwrap();
        session.commit(p).unwrap();
        let mut first = WallPanel::new(session.document(), "First wall".into()).unwrap();
        first.sync_source(&session);
        first.layer_placement.flipped = true;
        let before = session.document().clone();
        let command = first.request(session.document(), true).unwrap();
        assert!(command.wall.layer_placement.as_ref().unwrap().flipped);
        assert!(command.wall.thickness.is_none());
        assert!(command.wall.justification.is_none());
        let preview = command.prepare(&session, actor()).unwrap();
        let native = Compiler::default()
            .compile(preview.prepared.document(), session.revision())
            .unwrap();
        assert_eq!(native.snapshot().shapes[&first.feature].solid_count, 2);
        assert_eq!(session.document(), &before);
        drop(preview);
        session
            .commit(command.prepare(&session, actor()).unwrap().prepared)
            .unwrap();
        let mut second = WallPanel::new(session.document(), "Second wall".into()).unwrap();
        second
            .outline
            .set_points(&[[0., 1000.], [3000., 1000.]], false);
        session
            .commit(
                second
                    .request(session.document(), true)
                    .unwrap()
                    .prepare(&session, actor())
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let mut edit =
            WallPanel::existing(session.document(), first.feature, "First wall".into()).unwrap();
        if let WallLayerKind::Solid { thickness, .. } = &mut value.layers.layers[0].construction {
            *thickness = length(120.);
        }
        edit.layers = Some(
            super::super::wall_layers::LayerEditor::existing(
                session.document(),
                &value.layers,
                value.reference,
            )
            .unwrap(),
        );
        edit.type_height = DimensionField::from_dimension(&length(3000.), "mm").unwrap();
        edit.shared_edit = true;
        let request = edit.request(session.document(), true).unwrap();
        assert!(request.construction.is_some());
        assert!(request.new_types.is_empty());
        let prepared = request.prepare(&session, actor()).unwrap();
        let native = Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
        assert!(
            (native.snapshot().shapes[&second.feature].volume_mm3 - 3000. * 140. * 3000.).abs()
                < 0.01
        );
        session.commit(prepared.prepared).unwrap();
        let shared = model::read_wall_construction(session.document(), ty).unwrap();
        let mut private =
            WallPanel::existing(session.document(), first.feature, "First wall".into()).unwrap();
        private.wall_type = None;
        private.new_type = EntityId::new();
        private.type_name = "Private variant".into();
        let private_id = private.new_type;
        let request = private.request(session.document(), true).unwrap();
        assert!(request.construction.is_none());
        assert_eq!(request.new_types.len(), 1);
        let prepared = request.prepare(&session, actor()).unwrap();
        Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
        session.commit(prepared.prepared).unwrap();
        assert_eq!(
            model::read_wall(session.document(), first.feature)
                .unwrap()
                .wall_type,
            private_id
        );
        assert_eq!(
            model::read_wall(session.document(), second.feature)
                .unwrap()
                .wall_type,
            ty
        );
        assert_eq!(
            model::read_wall_construction(session.document(), ty).unwrap(),
            shared
        );
        let mut bytes = Vec::new();
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let opened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        let panel =
            WallPanel::existing(&opened.document, first.feature, "First wall".into()).unwrap();
        assert!(panel.layers.is_some());
        assert!(panel.layer_placement.flipped);
        let native = Compiler::default().compile(&opened.document, 0).unwrap();
        assert_eq!(native.snapshot().shapes[&first.feature].solid_count, 2);
    }

    #[test]
    fn panel_defines_types_and_a_wall_that_commit_and_reopen_for_editing() {
        let mut session = Session::new(Document::new("Panel").unwrap()).unwrap();
        let mut panel = WallPanel::new(session.document(), "Wall 1".into()).unwrap();
        assert!(
            panel.wall_type.is_none(),
            "an empty project starts with a new type"
        );
        panel
            .outline
            .set_points(&[[0., 0.], [4000., 0.], [4000., 3000.]], false);
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

        // Malformed corner fields are reported, not guessed.
        edit.outline = OutlineEditor::from_mm(OutlineKind::WallPath, &[[0., 0.]], false);
        edit.outline.paste_points("0, 0\n4000, 0").unwrap();
        let mut broken = WallPanel::existing(session.document(), feature, "Wall 1".into()).unwrap();
        broken.outline = OutlineEditor::new(
            OutlineKind::WallPath,
            &[[length(0.), length(0.)], [length(1.), length(0.)]],
            false,
        )
        .unwrap();
        broken.outline.set_text(1, 0, "four");
        assert!(
            broken
                .request(session.document(), true)
                .unwrap_err()
                .contains("Corner 2 x")
        );
    }

    #[test]
    fn uniform_and_layered_rooms_drawn_by_viewport_clicks_preview_and_cancel() {
        use multicad_core::document::{Entity, EntityData, Mutation};
        for layered in [false, true] {
            let material = Entity::new(
                "Brick",
                EntityData::Material {
                    density_kg_m3: None,
                    color_rgba: [160, 80, 60, 255],
                    section_hatch: None,
                },
            );
            let session = Session::new(
                Document::new("Drawn room")
                    .unwrap()
                    .edited(&[Mutation::Insert(material.clone())])
                    .unwrap(),
            )
            .unwrap();
            let document = session.document();
            let mut panel = WallPanel::new(document, "Room".into()).unwrap();
            panel.sync_source(&session);
            if layered {
                let stack = model::WallLayerStack {
                    layers: (0..2)
                        .map(|index| model::WallLayer {
                            id: EntityId::new(),
                            name: format!("Brick {index}"),
                            role: None,
                            construction: model::WallLayerKind::Solid {
                                material: material.id,
                                thickness: length(100.),
                            },
                        })
                        .collect(),
                };
                panel.layers = Some(
                    super::super::wall_layers::LayerEditor::existing(
                        document,
                        &stack,
                        model::WallLayerReference::Center,
                    )
                    .unwrap(),
                );
            }
            panel.outline.start_drawing();
            for raw in [
                [2., 3.],
                [4004., -2.],
                [3998., 3003.],
                [-3., 2996.],
                [4., 4.],
            ] {
                let points = panel.outline.positions(document).unwrap();
                panel.outline.click(raw, &points, &[], 25.);
            }
            assert!(panel.outline.closed() && !panel.outline.capturing());
            let command = panel.request(document, true).unwrap();
            assert!(command.wall.closed);
            let prepared = command
                .prepare(&session, Actor::local(Channel::Ui))
                .unwrap();
            let model = Compiler::default()
                .compile(prepared.prepared.document(), 0)
                .unwrap();
            assert_eq!(
                model.snapshot().shapes[&prepared.outputs[0].feature].solid_count,
                if layered { 2 } else { 1 }
            );
            let volume = model.snapshot().shapes[&prepared.outputs[0].feature].volume_mm3;
            // Centerline perimeter 14 m x 200 mm x 2500 mm.
            assert!((volume - 14000. * 200. * 2500.).abs() < 1e-3, "{volume}");

            // Redrawing then cancelling restores the closed room exactly.
            let before = panel.outline.dimensions(document).unwrap();
            panel.outline.start_drawing();
            panel.outline.click([500., 500.], &[], &[], 25.);
            assert!(panel.outline.cancel());
            assert_eq!(panel.outline.dimensions(document).unwrap(), before);
            assert!(panel.outline.closed());
        }
    }
    #[test]
    fn pinned_wall_inspector_combines_window_context_and_reopens_without_private_editing() {
        use crate::symbols::architecture::test_support;
        use multicad_core::symbols::architecture::ArchitectureRole;
        let actor = || Actor::local(Channel::Ui);
        let (definition, _) = test_support::definition(ArchitectureRole::Wall);
        let (mut session, _) = test_support::consumer(&definition);
        let mut panel = WallPanel::new(session.document(), "Pinned wall".into()).unwrap();
        panel.sync_source(&session);
        assert!(panel.symbol.active());
        assert!(panel.types.is_empty());
        let mut opening = OpeningRow::new(true).unwrap();
        opening.width = DimensionField::from_dimension(&length(600.), "mm").unwrap();
        opening.height = DimensionField::from_dimension(&length(800.), "mm").unwrap();
        opening.sill = DimensionField::from_dimension(&length(900.), "mm").unwrap();
        panel.openings.push(opening);
        let ModelRequest::SymbolConstruction(command) =
            panel.model_request(session.document(), true).unwrap()
        else {
            panic!("A pinned wall cannot become a local type")
        };
        let before = session.document().clone();
        let prepared = command.prepare(&session, actor()).unwrap();
        let native = Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
        let volume = native.solid(panel.body).unwrap().volume_mm3().unwrap();
        assert!((volume - 1_238_400_000.).abs() < 0.01, "{volume}");
        assert_eq!(session.document(), &before);
        session.commit(prepared.prepared).unwrap();
        let mut reopened =
            WallPanel::existing(session.document(), panel.feature, "Pinned wall".into()).unwrap();
        reopened.sync_source(&session);
        assert!(reopened.symbol.active());
        assert!(reopened.types.is_empty());
        assert_eq!(reopened.openings.len(), 1);
        let ModelRequest::SymbolConstruction(command) =
            reopened.model_request(session.document(), true).unwrap()
        else {
            panic!("A reopened wall must keep the architectural pin")
        };
        assert_eq!(command.edit_id(), Some(panel.feature));
        let prepared = command.prepare(&session, actor()).unwrap();
        assert_eq!(
            multicad_core::model::read_wall(prepared.prepared.document(), panel.feature)
                .unwrap()
                .openings
                .len(),
            1
        );
        assert!(
            prepared
                .outputs
                .iter()
                .any(|output| output.body == panel.body)
        );
    }
}
