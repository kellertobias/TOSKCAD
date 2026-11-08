use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session, SessionStamp},
    document::{Entity, EntityData, EntityId},
    materials::*,
};
mod preview;
#[derive(Clone, PartialEq)]
struct Draft {
    entity: Entity,
    style: Option<(EntityId, MaterialStyle)>,
    /// A new manufacturing material created together with a stock draft when
    /// the document has none yet (CAD-160). Committed in the same transaction.
    companion: Option<Entity>,
}
#[derive(Default)]
pub struct LibraryEditor {
    draft: Option<Draft>,
    original: Option<Draft>,
    revision: u64,
    document: Option<EntityId>,
    error: Option<String>,
    diagnostics: Option<(EntityId, u64, Vec<Diagnostic>)>,
    guard: Option<SessionStamp>,
    preview: preview::Lane,
}
impl LibraryEditor {
    pub fn cancel(&mut self) {
        self.preview.clear();
        self.preview.show_model = false;
        self.draft = None;
        self.original = None;
        self.error = None;
        self.guard = None;
        self.document = None;
        self.diagnostics = None;
    }
    pub fn dirty(&self) -> bool {
        self.draft != self.original
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn open(&mut self, s: &Session) {
        if !self.dirty()
            && self
                .guard
                .as_ref()
                .is_none_or(|guard| s.validate_stamp(guard).is_err())
        {
            let selected = self
                .draft
                .as_ref()
                .and_then(|d| s.document().get(d.entity.id))
                .cloned();
            self.draft = None;
            self.original = None;
            self.preview.clear();
            self.document = Some(s.document().id());
            self.revision = s.revision();
            self.guard = Some(s.stamp());
            if let Some(entity) = selected {
                self.select(s, entity);
            }
        }
    }
    fn select(&mut self, s: &Session, entity: Entity) {
        self.preview.clear();
        self.preview.show_model = false;
        let style = if matches!(entity.data, EntityData::Material { .. }) {
            material_style(s.document(), entity.id).ok().flatten()
        } else {
            None
        };
        self.draft = Some(Draft {
            entity,
            style,
            companion: None,
        });
        self.original = self.draft.clone();
        self.revision = s.revision();
        self.document = Some(s.document().id());
        self.guard = Some(s.stamp());
        self.error = None;
    }
    fn create(&mut self, s: &Session, kind: usize) {
        self.preview.clear();
        self.preview.show_model = false;
        let id = EntityId::new();
        let mut companion = None;
        let entity = match kind {
            0 => new_material("New material"),
            1 => {
                // Never let the stock point at itself: with no material in the
                // document, draft a companion material committed alongside it.
                let material = s
                    .document()
                    .entities()
                    .find(|e| matches!(e.data, EntityData::Material { .. }))
                    .map(|e| e.id)
                    .unwrap_or_else(|| {
                        let material = new_material("Stock material");
                        let material_id = material.id;
                        companion = Some(material);
                        material_id
                    });
                LibraryDefinition::Stock(Stock {
                    material,
                    specification: stock_default(0),
                })
                .entity(id, "New stock")
                .expect("finite defaults")
            }
            2 => LibraryDefinition::Appearance(Appearance::default())
                .entity(id, "New appearance")
                .expect("finite defaults"),
            _ => LibraryDefinition::Hatch(HatchStyle::default())
                .entity(id, "New section style")
                .expect("finite defaults"),
        };
        self.draft = Some(Draft {
            entity,
            style: None,
            companion,
        });
        self.original = None;
        self.document = Some(s.document().id());
        self.revision = s.revision();
        self.guard = Some(s.stamp());
        self.error = None;
    }
    fn prepare(&self, s: &Session, channel: Channel) -> Result<Prepared, String> {
        s.validate_stamp(self.guard.as_ref().ok_or("Open Project Materials first")?)
            .map_err(|e| e.to_string())?;
        if self.document != Some(s.document().id()) {
            return Err("Library draft belongs to another document".into());
        }
        let d = self
            .draft
            .as_ref()
            .ok_or("Choose or create a definition first")?;
        if let Some(problem) = missing_prerequisite(d, s.document()) {
            return Err(problem);
        }
        let mut entities = Vec::new();
        if let Some(material) = d
            .companion
            .as_ref()
            .filter(|m| stock_material(&d.entity) == Some(m.id))
        {
            entities.push(material.clone());
        }
        entities.push(d.entity.clone());
        if let Some((id, style)) = &d.style {
            entities.push(
                LibraryDefinition::MaterialStyle(style.clone())
                    .entity(*id, "Material defaults")
                    .map_err(|e| e.to_string())?,
            );
        }
        prepare_library(s, self.revision, Actor::local(channel), &entities)
            .map_err(|e| e.to_string())
    }
    fn create_stock(&mut self, s: &Session, kind: usize) {
        self.create(s, 1);
        let draft = self.draft.as_mut().expect("created stock");
        let material = stock_material(&draft.entity).expect("stock material");
        if let EntityData::Extension { definition, inputs } = &mut draft.entity.data {
            let stock = LibraryDefinition::Stock(Stock {
                material,
                specification: stock_default(kind),
            });
            definition.data = serde_json::to_value(&stock).expect("stock draft");
            *inputs = stock.links();
        }
        draft.entity.name =
            ["New panel stock", "New sheet stock", "New profile stock"][kind].into();
    }
    pub fn show_inline(
        &mut self,
        ui: &mut egui::Ui,
        s: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        editable: bool,
    ) {
        self.open(s);
        self.poll(s, resources);
        ui.strong("Materials and stock");
        ui.small("Shared Project definitions · Model tools select and assign these same entries.");
        ui.horizontal_wrapped(|ui| {
            for (label, kind) in [
                ("New material", 0),
                ("New panel stock", 1),
                ("New sheet stock", 2),
                ("New profile stock", 3),
                ("New appearance", 4),
                ("New hatch", 5),
            ] {
                if ui
                    .add_enabled(
                        editable && !self.dirty(),
                        egui::Button::new(label).min_size(egui::vec2(0., 30.)),
                    )
                    .on_hover_text("Create one reusable definition in this project")
                    .clicked()
                {
                    match kind {
                        1..=3 => self.create_stock(s, kind - 1),
                        4 => self.create(s, 2),
                        5 => self.create(s, 3),
                        _ => self.create(s, 0),
                    }
                }
            }
        });
        if !editable {
            ui.colored_label(
                Color32::LIGHT_YELLOW,
                "Apply or discard file-information or parameter edits before editing definitions.",
            );
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.preview.show_model, false, "Definitions");
            ui.selectable_value(&mut self.preview.show_model, true, "Affected model");
        });
        self.preview_status(ui);
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if self
            .guard
            .as_ref()
            .is_none_or(|g| s.validate_stamp(g).is_err())
        {
            ui.colored_label(
                Color32::LIGHT_YELLOW,
                "Source changed. Discard and reopen the definition before applying.",
            );
        }
        if self
            .diagnostics
            .as_ref()
            .is_none_or(|(id, rev, _)| *id != s.document().id() || *rev != s.revision())
        {
            self.diagnostics = Some((
                s.document().id(),
                s.revision(),
                diagnostics(s.document()).unwrap_or_default(),
            ));
        }
        if let Some((_, _, issues)) = &self.diagnostics
            && !issues.is_empty()
        {
            ui.collapsing(format!("Definition diagnostics ({})", issues.len()), |ui| {
                for issue in issues {
                    ui.colored_label(
                        Color32::LIGHT_YELLOW,
                        format!("{}: {}", issue.role, issue.message),
                    );
                }
            });
        }
        let blocked = self
            .draft
            .as_ref()
            .and_then(|d| missing_prerequisite(d, s.document()));
        if let Some(problem) = &blocked {
            ui.colored_label(Color32::LIGHT_RED, problem);
            if ui
                .button("Create a material first")
                .on_hover_text("Discard this stock draft and start a manufacturing material")
                .clicked()
            {
                self.create(s, 0);
            }
        }
        ui.separator();
        if self.preview.show_model {
            self.show_model(ui);
        } else {
            egui::ScrollArea::vertical()
                .id_salt("project-material-definitions")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut entries: Vec<_> = s
                        .document()
                        .entities()
                        .filter(|entity| {
                            matches!(entity.data, EntityData::Material { .. })
                                || matches!(
                                    read(entity),
                                    Ok(Some(
                                        LibraryDefinition::Stock(_)
                                            | LibraryDefinition::Appearance(_)
                                            | LibraryDefinition::Hatch(_)
                                    ))
                                )
                        })
                        .cloned()
                        .collect();
                    entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                    if ui.available_width() < 600. {
                        let selected = self.draft.as_ref().map(|d| d.entity.id);
                        let label = self
                            .draft
                            .as_ref()
                            .map_or("Choose a definition", |draft| draft.entity.name.as_str());
                        ui.label("Shared definitions");
                        let mut choice = None;
                        ui.add_enabled_ui(editable && !self.dirty() && !entries.is_empty(), |ui| {
                            crate::controls::inspector_combo(ui, "project-definition")
                                .selected_text(label)
                                .show_ui(ui, |ui| {
                                    for entry in &entries {
                                        if ui
                                            .selectable_label(
                                                selected == Some(entry.id),
                                                &entry.name,
                                            )
                                            .clicked()
                                        {
                                            choice = Some(entry.id);
                                        }
                                    }
                                });
                        });
                        if let Some(id) = choice
                            && let Some(entry) = entries.into_iter().find(|entry| entry.id == id)
                        {
                            self.select(s, entry);
                        }
                        ui.separator();
                        self.show_definition_editor(ui, s, editable);
                    } else {
                        ui.columns(2, |columns| {
                            columns[0].small(format!("{} shared definitions", entries.len()));
                            for entry in entries {
                                let selected =
                                    self.draft.as_ref().is_some_and(|d| d.entity.id == entry.id);
                                if columns[0]
                                    .add_enabled(
                                        editable && !self.dirty(),
                                        egui::Button::selectable(selected, &entry.name).wrap(),
                                    )
                                    .on_hover_text(format!(
                                        "Edit {} · stable identity {}",
                                        entry.name, entry.id
                                    ))
                                    .clicked()
                                {
                                    self.select(s, entry);
                                }
                            }
                            self.show_definition_editor(&mut columns[1], s, editable);
                        });
                    }
                });
        }
    }
    fn show_definition_editor(&mut self, ui: &mut egui::Ui, s: &Session, editable: bool) {
        ui.add_enabled_ui(editable, |ui| {
            if let Some(draft) = &mut self.draft {
                // The draft notice and compact/roomy layouts change the parent
                // UI. Keep field focus with this definition across both.
                let id = egui::Id::new((
                    "project-material-editor",
                    s.document().id(),
                    draft.entity.id,
                ));
                ui.scope_builder(egui::UiBuilder::new().id(id), |ui| edit_draft(ui, s, draft));
            } else {
                ui.label("Choose or create a reusable definition.");
            }
        });
    }
}
fn edit_draft(ui: &mut egui::Ui, s: &Session, d: &mut Draft) {
    let name = ui.label("Name");
    ui.add(egui::TextEdit::singleline(&mut d.entity.name).desired_width(f32::INFINITY))
        .labelled_by(name.id)
        .on_hover_text("Library display name; assignments retain the stable ID");
    let description = ui.label("Description");
    ui.add(
        egui::TextEdit::multiline(&mut d.entity.description)
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(description.id);
    if let EntityData::Material {
        density_kg_m3,
        color_rgba,
        ..
    } = &mut d.entity.data
    {
        let mut density = density_kg_m3.is_some();
        if ui
            .checkbox(&mut density, "Density (kg/m³)")
            .on_hover_text("Optional manufacturing density, independent of render appearance")
            .changed()
        {
            *density_kg_m3 = if density { Some(700.) } else { None };
        }
        if let Some(v) = density_kg_m3 {
            number(ui, "Density", v, " kg/m³");
        }
        ui.label("Base color");
        ui.color_edit_button_srgba_unmultiplied(color_rgba);
        let mut appearance = d
            .style
            .as_ref()
            .and_then(|(_, s)| s.appearance.as_ref().map(|r| r.id));
        let mut hatch = d
            .style
            .as_ref()
            .and_then(|(_, s)| s.hatch.as_ref().map(|r| r.id));
        let changed = choice(ui, "Default appearance", &mut appearance, s, 2)
            | choice(ui, "Default section hatch", &mut hatch, s, 3);
        let acknowledge = ui.button("Acknowledge current defaults").on_hover_text("Review the live definitions, then acknowledge their current content without changing their IDs").clicked();
        if changed || acknowledge {
            let capture =
                |id: Option<EntityId>| id.and_then(|id| LibraryRef::capture(s.document(), id).ok());
            let id = d.style.as_ref().map_or_else(EntityId::new, |v| v.0);
            d.style = Some((
                id,
                MaterialStyle {
                    material: d.entity.id,
                    appearance: capture(appearance),
                    hatch: capture(hatch),
                },
            ));
        }
    } else if let EntityData::Extension {
        definition: wire, ..
    } = &d.entity.data
        && let Ok(mut definition) = serde_json::from_value::<LibraryDefinition>(wire.data.clone())
    {
        match &mut definition {
            LibraryDefinition::Appearance(a) => {
                ui.label("Color");
                ui.color_edit_button_srgba_unmultiplied(&mut a.color_rgba);
                number(ui, "Roughness", &mut a.roughness, "");
                number(ui, "Metallic", &mut a.metallic, "");
            }
            LibraryDefinition::Hatch(h) => {
                egui::ComboBox::from_id_salt("pattern")
                    .selected_text(format!("{:?}", h.pattern))
                    .show_ui(ui, |ui| {
                        for p in [HatchPattern::None, HatchPattern::Lines, HatchPattern::Cross] {
                            ui.selectable_value(&mut h.pattern, p, format!("{p:?}"));
                        }
                    });
                number(ui, "Angle", &mut h.angle_deg, "°");
                number(ui, "Spacing (paper)", &mut h.spacing_mm, " mm");
                number(ui, "Line width (paper)", &mut h.line_width_mm, " mm");
            }
            LibraryDefinition::Stock(stock) => {
                let pending = d.companion.as_mut().filter(|m| m.id == stock.material);
                let has_existing = s
                    .document()
                    .entities()
                    .any(|e| matches!(e.data, EntityData::Material { .. }));
                if let Some(material) = pending {
                    ui.label("Manufacturing material (new)");
                    ui.text_edit_singleline(&mut material.name)
                        .on_hover_text("Created together with this stock in one undoable edit");
                    ui.small(
                        "No manufacturing material exists yet. Applying creates this material and the stock together.",
                    );
                }
                if has_existing || d.companion.is_none() {
                    let mut material = Some(stock.material);
                    let label = if d.companion.is_some() {
                        "Use existing material instead"
                    } else {
                        "Manufacturing material"
                    };
                    choice(ui, label, &mut material, s, 0);
                    if let Some(id) = material {
                        stock.material = id;
                    }
                }
                let mut kind = stock_index(&stock.specification);
                let old = kind;
                egui::ComboBox::from_id_salt("stock-kind")
                    .selected_text(["Panel", "Sheet metal", "Profile", "Printed material"][kind])
                    .show_ui(ui, |ui| {
                        for (i, n) in ["Panel", "Sheet metal", "Profile", "Printed material"]
                            .into_iter()
                            .enumerate()
                        {
                            ui.selectable_value(&mut kind, i, n);
                        }
                    });
                if old != kind {
                    stock.specification = stock_default(kind);
                }
                stock_controls(ui, &mut stock.specification);
                ui.small("Tools that use this stock read its live thickness or section. Preview checks affected geometry; a metadata assignment alone does not resize a Part.");
            }
            _ => {}
        }
        // Keep invalid numeric drafts editable; canonical validation runs on Apply.
        if let EntityData::Extension {
            definition: wire,
            inputs,
        } = &mut d.entity.data
        {
            wire.data = serde_json::to_value(&definition).expect("serializable library draft");
            *inputs = definition.links();
        }
    }
}
fn new_material(name: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Material {
            density_kg_m3: Some(700.),
            color_rgba: [150, 170, 190, 255],
            section_hatch: None,
        },
    )
}
fn stock_material(entity: &Entity) -> Option<EntityId> {
    let EntityData::Extension { definition, .. } = &entity.data else {
        return None;
    };
    match serde_json::from_value::<LibraryDefinition>(definition.data.clone()) {
        Ok(LibraryDefinition::Stock(stock)) => Some(stock.material),
        _ => None,
    }
}
/// Explains, by name, why a stock draft cannot be applied yet.
fn missing_prerequisite(d: &Draft, document: &multicad_core::document::Document) -> Option<String> {
    let material = stock_material(&d.entity)?;
    if d.companion.as_ref().is_some_and(|m| m.id == material) {
        return None;
    }
    let name = &d.entity.name;
    match document.get(material) {
        Some(e) if matches!(e.data, EntityData::Material { .. }) => None,
        Some(e) => Some(format!(
            "Stock '{name}' needs a manufacturing material, but '{}' is not a material.",
            e.name
        )),
        None => Some(format!(
            "Stock '{name}' needs a manufacturing material. Create or select a material first."
        )),
    }
}
pub(super) fn choice(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<EntityId>,
    s: &Session,
    kind: usize,
) -> bool {
    let old = *value;
    let name = value
        .and_then(|id| s.document().get(id).map(|e| e.name.clone()))
        .unwrap_or_else(|| {
            if value.is_some() {
                "Missing definition".into()
            } else {
                "Inherit / none".into()
            }
        });
    ui.label(label);
    egui::ComboBox::from_id_salt(label)
        .width(ui.available_width().min(220.))
        .truncate()
        .selected_text(name)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "Inherit / none");
            for e in s.document().entities() {
                let eligible = match kind {
                    0 => matches!(e.data, EntityData::Material { .. }),
                    1 => matches!(read(e), Ok(Some(LibraryDefinition::Stock(_)))),
                    2 => matches!(read(e), Ok(Some(LibraryDefinition::Appearance(_)))),
                    _ => matches!(read(e), Ok(Some(LibraryDefinition::Hatch(_)))),
                };
                if eligible {
                    ui.selectable_value(value, Some(e.id), &e.name);
                }
            }
        })
        .response
        .on_hover_text("Choose a reusable definition by its stable identity");
    old != *value
}
fn number(ui: &mut egui::Ui, label: &str, value: &mut f64, suffix: &str) {
    ui.horizontal_wrapped(|ui| {
        let label_response = ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.1).suffix(suffix))
            .labelled_by(label_response.id)
            .on_hover_text(label);
    });
}
fn stock_index(s: &StockKind) -> usize {
    match s {
        StockKind::Panel { .. } => 0,
        StockKind::Sheet { .. } => 1,
        StockKind::Profile { .. } => 2,
        StockKind::Print { .. } => 3,
    }
}
fn stock_default(i: usize) -> StockKind {
    match i {
        0 => StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2440.,
            width_mm: 1220.,
            grain: Grain::AlongLength,
        },
        1 => StockKind::Sheet {
            thickness_mm: 1.,
            length_mm: 2000.,
            width_mm: 1000.,
            inside_bend_radius_mm: 1.,
            k_factor: 0.4,
        },
        2 => StockKind::Profile {
            designation: "Rectangular tube".into(),
            length_mm: 6000.,
            width_mm: 30.,
            height_mm: 30.,
            wall_mm: Some(2.),
            shape: ProfileShape::Rectangular,
        },
        _ => StockKind::Print {
            process: "FDM".into(),
            nozzle_mm: Some(0.4),
            layer_height_mm: 0.2,
        },
    }
}
fn stock_controls(ui: &mut egui::Ui, s: &mut StockKind) {
    match s {
        StockKind::Panel {
            thickness_mm,
            length_mm,
            width_mm,
            grain,
        } => {
            number(ui, "Thickness", thickness_mm, " mm");
            number(ui, "Length", length_mm, " mm");
            number(ui, "Width", width_mm, " mm");
            egui::ComboBox::from_id_salt("grain")
                .selected_text(format!("{grain:?}"))
                .show_ui(ui, |ui| {
                    for g in [Grain::None, Grain::AlongLength, Grain::AcrossLength] {
                        ui.selectable_value(grain, g, format!("{g:?}"));
                    }
                });
        }
        StockKind::Sheet {
            thickness_mm,
            length_mm,
            width_mm,
            inside_bend_radius_mm,
            k_factor,
        } => {
            number(ui, "Thickness", thickness_mm, " mm");
            number(ui, "Length", length_mm, " mm");
            number(ui, "Width", width_mm, " mm");
            number(ui, "Inside bend radius", inside_bend_radius_mm, " mm");
            number(ui, "K factor", k_factor, "");
        }
        StockKind::Profile {
            designation,
            length_mm,
            width_mm,
            height_mm,
            wall_mm,
            shape,
        } => {
            let designation_label = ui.label("Designation");
            ui.text_edit_singleline(designation)
                .labelled_by(designation_label.id);
            let previous_shape = *shape;
            let shape_label = |shape: ProfileShape| match shape {
                ProfileShape::Rectangular => "Rectangular",
                ProfileShape::Round => "Round",
                ProfileShape::Angle => "Angle",
                ProfileShape::ItemLine8_40x40 => "item Line 8 40×40",
            };
            egui::ComboBox::from_id_salt("profile-shape")
                .selected_text(shape_label(*shape))
                .show_ui(ui, |ui| {
                    for s in [
                        ProfileShape::Rectangular,
                        ProfileShape::Round,
                        ProfileShape::Angle,
                        ProfileShape::ItemLine8_40x40,
                    ] {
                        ui.selectable_value(shape, s, shape_label(s));
                    }
                });
            if previous_shape != *shape {
                if *shape == ProfileShape::ItemLine8_40x40 {
                    *width_mm = 40.;
                    *height_mm = 40.;
                    *wall_mm = None;
                }
                if *shape == ProfileShape::Round {
                    *height_mm = *width_mm;
                }
                if *shape == ProfileShape::Angle && wall_mm.is_none() {
                    *wall_mm = Some(2.);
                }
            }
            number(ui, "Stock length", length_mm, " mm");
            if *shape == ProfileShape::ItemLine8_40x40 {
                ui.label("40 × 40 mm · four open 8 mm grooves · Ø6.8 mm core");
                ui.label("item 0.0.026.03 mounting dimensions with simplified groove flanks");
                return;
            }
            if *shape == ProfileShape::Round {
                let previous_diameter = *width_mm;
                number(ui, "Outside diameter", width_mm, " mm");
                if previous_diameter != *width_mm {
                    *height_mm = *width_mm;
                }
            } else {
                number(ui, "Width", width_mm, " mm");
                number(ui, "Height", height_mm, " mm");
            }
            if *shape != ProfileShape::Angle {
                let mut hollow = wall_mm.is_some();
                if ui.checkbox(&mut hollow, "Hollow section").changed() {
                    *wall_mm = if hollow { Some(2.) } else { None };
                }
            }
            if let Some(v) = wall_mm {
                number(
                    ui,
                    if *shape == ProfileShape::Angle {
                        "Leg thickness"
                    } else {
                        "Wall"
                    },
                    v,
                    " mm",
                );
            }
        }
        StockKind::Print {
            process,
            nozzle_mm,
            layer_height_mm,
        } => {
            ui.text_edit_singleline(process);
            if let Some(v) = nozzle_mm {
                number(ui, "Nozzle", v, " mm");
            }
            number(ui, "Layer height", layer_height_mm, " mm");
        }
    }
}

#[cfg(test)]
mod tests;
