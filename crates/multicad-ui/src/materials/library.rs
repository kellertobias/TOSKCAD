use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Entity, EntityData, EntityId},
    materials::*,
};
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
    open: bool,
    draft: Option<Draft>,
    original: Option<Draft>,
    revision: u64,
    document: Option<EntityId>,
    error: Option<String>,
    diagnostics: Option<(EntityId, u64, Vec<Diagnostic>)>,
}
impl LibraryEditor {
    pub fn dirty(&self) -> bool {
        self.draft != self.original
    }
    pub fn open(&mut self, s: &Session) {
        self.open = true;
        if !self.dirty() {
            self.document = Some(s.document().id());
            self.revision = s.revision();
        }
    }
    fn select(&mut self, s: &Session, entity: Entity) {
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
        self.error = None;
    }
    fn create(&mut self, s: &Session, kind: usize) {
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
        self.error = None;
    }
    fn apply(&mut self, s: &mut Session) -> Result<(), String> {
        if self.document != Some(s.document().id()) {
            return Err("Library draft belongs to another document".into());
        }
        let Some(d) = self.draft.as_ref() else {
            return Ok(());
        };
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
        let p = prepare_library(s, self.revision, Actor::local(Channel::Ui), &entities)
            .map_err(|e| e.to_string())?;
        s.commit(p).map_err(|e| e.to_string())?;
        self.select(s, d.entity.clone());
        Ok(())
    }
    pub fn show(&mut self, ctx: &egui::Context, s: &mut Session) {
        if !self.open {
            return;
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
        let mut close = false;
        egui::Window::new("Materials and stock")
            .id(egui::Id::new("material-library"))
            .default_width(660.)
            .default_height(500.)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label("Manufacturing definitions, appearance and paper section styles are independent.");
                ui.horizontal_wrapped(|ui| {
                    for (i, label) in ["New material", "New stock", "New appearance", "New hatch"].into_iter().enumerate() {
                        if ui.add_enabled(!self.dirty(), egui::Button::new(label))
                            .on_hover_text("Create a reusable document library definition").clicked() {
                            self.create(s, i);
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(480.).show(ui, |ui| {
                    ui.columns(2, |columns| {
                        let entries: Vec<_> = s.document().entities().filter(|e| {
                            matches!(e.data, EntityData::Material { .. }) || matches!(read(e), Ok(Some(LibraryDefinition::Stock(_) | LibraryDefinition::Appearance(_) | LibraryDefinition::Hatch(_))))
                        }).cloned().collect();
                        for e in entries {
                            let selected = self.draft.as_ref().is_some_and(|d| d.entity.id == e.id);
                            if columns[0].add_enabled(!self.dirty(), egui::Button::selectable(selected, &e.name))
                                .on_hover_text(format!("Edit {}", e.name)).clicked() {
                                self.select(s, e);
                            }
                        }
                        if let Some(draft) = &mut self.draft {
                            edit_draft(&mut columns[1], s, draft);
                        } else {
                            columns[1].label("Choose or create a reusable definition.");
                        }
                    });
                });
                if self.document != Some(s.document().id()) || self.revision != s.revision() {
                    ui.colored_label(Color32::LIGHT_YELLOW, "Source changed. Discard and reopen the definition before applying.");
                }
                if let Some((_, _, diagnostics)) = &self.diagnostics && !diagnostics.is_empty() {
                    ui.collapsing(format!("Definition diagnostics ({})", diagnostics.len()), |ui| {
                        for diagnostic in diagnostics {
                            ui.colored_label(Color32::LIGHT_YELLOW, format!("{}: {}", diagnostic.role, diagnostic.message));
                        }
                    });
                }
                if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
                let blocked = self.draft.as_ref().and_then(|d| missing_prerequisite(d, s.document()));
                if let Some(problem) = &blocked {
                    ui.colored_label(Color32::LIGHT_RED, problem);
                    if ui.button("Create a material first").on_hover_text("Discard this stock draft and start a new manufacturing material").clicked() {
                        self.create(s, 0);
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(self.dirty() && blocked.is_none(), egui::Button::new("Apply library edits"))
                        .on_hover_text("Commit one undoable source transaction").clicked() {
                        self.error = self.apply(s).err();
                    }
                    if ui.button("Discard library edits").on_hover_text("Drop the draft; document remains unchanged").clicked() {
                        self.draft = None;
                        self.original = None;
                        self.error = None;
                        self.revision = s.revision();
                        self.document = Some(s.document().id());
                    }
                    if ui.add_enabled(!self.dirty(), egui::Button::new("Close library")).clicked() { close = true; }
                });
            });
        if close {
            self.open = false;
        }
    }
}
fn edit_draft(ui: &mut egui::Ui, s: &Session, d: &mut Draft) {
    ui.label("Name");
    ui.text_edit_singleline(&mut d.entity.name)
        .on_hover_text("Library display name; assignments retain the stable ID");
    ui.label("Description");
    ui.text_edit_multiline(&mut d.entity.description);
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
                ui.small("Stock metadata does not resize geometry or certify manufacturability.");
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
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.1).suffix(suffix))
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
            ui.text_edit_singleline(designation);
            egui::ComboBox::from_id_salt("profile-shape")
                .selected_text(format!("{shape:?}"))
                .show_ui(ui, |ui| {
                    for s in [
                        ProfileShape::Rectangular,
                        ProfileShape::Round,
                        ProfileShape::Angle,
                    ] {
                        ui.selectable_value(shape, s, format!("{s:?}"));
                    }
                });
            number(ui, "Stock length", length_mm, " mm");
            number(ui, "Width", width_mm, " mm");
            number(ui, "Height", height_mm, " mm");
            if let Some(v) = wall_mm {
                number(ui, "Wall", v, " mm");
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
mod tests {
    use super::*;
    use multicad_core::document::Document;
    fn text_rect(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<egui::Rect> {
        fn one(shape: &egui::epaint::Shape, text: &str) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == text => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                }
                egui::epaint::Shape::Vec(v) => v.iter().find_map(|s| one(s, text)),
                _ => None,
            }
        }
        shapes.iter().find_map(|s| one(&s.shape, text))
    }
    fn frame(
        ctx: &egui::Context,
        editor: &mut LibraryEditor,
        s: &mut Session,
        events: Vec<egui::Event>,
    ) -> Option<egui::Rect> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 800.),
                )),
                events,
                ..Default::default()
            },
            |ui| editor.show(ui.ctx(), s),
        );
        let rect = text_rect(&output.shapes, "Apply library edits");
        output.textures_delta.clear();
        rect
    }
    #[test]
    fn actual_apply_button_commits_catalog_draft_once_and_undo_restores() {
        let mut s = Session::new(Document::new("Library UI").unwrap()).unwrap();
        let before = s.document().clone();
        let mut editor = LibraryEditor::default();
        editor.open(&s);
        editor.create(&s, 0);
        let id = editor.draft.as_ref().unwrap().entity.id;
        editor.draft.as_mut().unwrap().entity.name = "Reusable oak".into();
        let ctx = egui::Context::default();
        let mut rect = None;
        for _ in 0..3 {
            rect = frame(&ctx, &mut editor, &mut s, vec![]);
        }
        assert!(editor.dirty());
        assert_eq!(s.document(), &before);
        let pos = rect.expect("Apply is rendered").center();
        let events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ];
        frame(&ctx, &mut editor, &mut s, events);
        assert_eq!(s.revision(), 1);
        assert_eq!(s.document().get(id).unwrap().name, "Reusable oak");
        assert!(!editor.dirty());
        assert_eq!(s.activity().last().unwrap().actor.channel, Channel::Ui);
        s.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
    }
    fn set_stock(editor: &mut LibraryEditor, f: impl FnOnce(&mut Stock)) {
        let d = editor.draft.as_mut().unwrap();
        let EntityData::Extension { definition, inputs } = &mut d.entity.data else {
            panic!("stock draft")
        };
        let Ok(LibraryDefinition::Stock(mut stock)) =
            serde_json::from_value::<LibraryDefinition>(definition.data.clone())
        else {
            panic!("stock draft")
        };
        f(&mut stock);
        let definition_value = LibraryDefinition::Stock(stock);
        definition.data = serde_json::to_value(&definition_value).unwrap();
        *inputs = definition_value.links();
    }
    #[test]
    fn new_stock_in_empty_library_drafts_a_companion_material_and_applies_once() {
        let mut s = Session::new(Document::new("Empty stock").unwrap()).unwrap();
        let before = s.document().clone();
        let mut editor = LibraryEditor::default();
        editor.open(&s);
        editor.create(&s, 1);
        let d = editor.draft.as_ref().unwrap();
        let stock_id = d.entity.id;
        let material = d.companion.clone().expect("companion material drafted");
        assert_eq!(stock_material(&d.entity), Some(material.id));
        assert_ne!(material.id, stock_id, "stock must never reference itself");
        assert!(missing_prerequisite(d, s.document()).is_none());
        // The real Apply button is enabled and commits both in one revision.
        let ctx = egui::Context::default();
        let mut rect = None;
        for _ in 0..3 {
            rect = frame(&ctx, &mut editor, &mut s, vec![]);
        }
        let pos = rect.expect("Apply is rendered").center();
        frame(
            &ctx,
            &mut editor,
            &mut s,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert_eq!(editor.error, None);
        assert_eq!(s.revision(), 1);
        assert!(matches!(
            s.document().get(material.id).unwrap().data,
            EntityData::Material { .. }
        ));
        assert!(matches!(
            read(s.document().get(stock_id).unwrap()),
            Ok(Some(LibraryDefinition::Stock(_)))
        ));
        assert!(diagnostics(s.document()).unwrap().is_empty());
        s.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
    }
    #[test]
    fn stock_with_missing_material_is_blocked_by_name_and_preserves_source() {
        let mut s = Session::new(Document::new("Missing stock material").unwrap()).unwrap();
        let mut editor = LibraryEditor::default();
        editor.open(&s);
        editor.create(&s, 1);
        let stock_id = editor.draft.as_ref().unwrap().entity.id;
        editor.draft.as_mut().unwrap().entity.name = "Birch ply".into();
        editor.draft.as_mut().unwrap().companion = None;
        set_stock(&mut editor, |stock| stock.material = stock_id);
        let before = s.document().clone();
        let problem = missing_prerequisite(editor.draft.as_ref().unwrap(), s.document()).unwrap();
        assert!(
            problem.contains("Stock 'Birch ply'") && problem.contains("manufacturing material")
        );
        let error = editor.apply(&mut s).unwrap_err();
        assert!(!error.contains("cycle"), "{error}");
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 0);
        // Apply is disabled and the explicit route to a material is offered.
        let ctx = egui::Context::default();
        let mut output: Option<egui::FullOutput> = None;
        for _ in 0..2 {
            if let Some(o) = output.as_mut() {
                o.textures_delta.clear();
            }
            output = Some(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 800.),
                    )),
                    ..Default::default()
                },
                |ui| editor.show(ui.ctx(), &mut s),
            ));
        }
        let mut output = output.unwrap();
        output.textures_delta.clear();
        let shapes = output.shapes;
        assert!(text_rect(&shapes, "Create a material first").is_some());
        assert!(text_rect(&shapes, &problem).is_some());
    }
    #[test]
    fn material_first_sequence_creates_panel_sheet_and_profile_stock() {
        let mut s = Session::new(Document::new("Material first").unwrap()).unwrap();
        let mut editor = LibraryEditor::default();
        editor.open(&s);
        editor.create(&s, 0);
        let material = editor.draft.as_ref().unwrap().entity.id;
        editor.apply(&mut s).unwrap();
        for kind in 0..3 {
            editor.draft = None;
            editor.original = None;
            editor.create(&s, 1);
            assert!(editor.draft.as_ref().unwrap().companion.is_none());
            set_stock(&mut editor, |stock| {
                stock.specification = stock_default(kind)
            });
            let id = editor.draft.as_ref().unwrap().entity.id;
            editor.apply(&mut s).unwrap();
            let Ok(Some(LibraryDefinition::Stock(stock))) = read(s.document().get(id).unwrap())
            else {
                panic!("stock committed")
            };
            assert_eq!(stock.material, material);
            assert_eq!(stock_index(&stock.specification), kind);
        }
        assert_eq!(s.revision(), 4);
        assert!(diagnostics(s.document()).unwrap().is_empty());
    }
    #[test]
    fn invalid_and_stale_catalog_drafts_reject_without_source_edits() {
        let mut s = Session::new(Document::new("Invalid library UI").unwrap()).unwrap();
        let mut editor = LibraryEditor::default();
        editor.create(&s, 0);
        if let EntityData::Material { density_kg_m3, .. } =
            &mut editor.draft.as_mut().unwrap().entity.data
        {
            *density_kg_m3 = Some(-10.);
        }
        let before = s.document().clone();
        assert!(editor.apply(&mut s).is_err());
        assert_eq!(s.document(), &before);
        editor.create(&s, 0);
        s.apply(multicad_core::commands::Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Rename".into(),
            edits: vec![multicad_core::document::Mutation::RenameDocument(
                "New revision".into(),
            )],
        })
        .unwrap();
        assert!(editor.apply(&mut s).unwrap_err().contains("Stale"));
        assert!(s.document().entities().next().is_none());
    }
}
