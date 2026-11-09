use eframe::egui::{self, Color32};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session, SessionStamp},
    document::{Entity, EntityData, EntityId},
    materials::*,
};
use std::collections::BTreeMap;
mod preview;
mod sample;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Materials,
    PanelStock,
    SheetStock,
    ProfileStock,
    PrintStock,
    Appearances,
    Hatches,
}
impl Category {
    const ALL: [Self; 7] = [
        Self::Materials,
        Self::PanelStock,
        Self::SheetStock,
        Self::ProfileStock,
        Self::PrintStock,
        Self::Appearances,
        Self::Hatches,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Materials => "Materials",
            Self::PanelStock => "Panel stock",
            Self::SheetStock => "Sheet stock",
            Self::ProfileStock => "Profile stock",
            Self::PrintStock => "Print stock",
            Self::Appearances => "Appearances",
            Self::Hatches => "Hatches",
        }
    }

    fn for_entity(entity: &Entity) -> Option<Self> {
        if matches!(entity.data, EntityData::Material { .. }) {
            return Some(Self::Materials);
        }
        Some(match read(entity).ok().flatten()? {
            LibraryDefinition::Stock(stock) => match stock.specification {
                StockKind::Panel { .. } => Self::PanelStock,
                StockKind::Sheet { .. } => Self::SheetStock,
                StockKind::Profile { .. } => Self::ProfileStock,
                StockKind::Print { .. } => Self::PrintStock,
            },
            LibraryDefinition::Appearance(_) => Self::Appearances,
            LibraryDefinition::Hatch(_) => Self::Hatches,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStock {
    Panel,
    Sheet,
    Profile,
    Batten,
}
impl ToolStock {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Panel => "Create panel stock…",
            Self::Sheet => "Create sheet stock…",
            Self::Profile => "Create tube or profile stock…",
            Self::Batten => "Create solid rectangular stock…",
        }
    }
}
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
    diagnostics: Option<(SessionStamp, Vec<Diagnostic>)>,
    guard: Option<SessionStamp>,
    preview: preview::Lane,
    confirm_on_ready: Option<Channel>,
    reveal_category: Option<Category>,
    catalog: Option<(SessionStamp, [Vec<EntityId>; Category::ALL.len()])>,
    swatches: BTreeMap<EntityId, [u8; 4]>,
}
impl LibraryEditor {
    pub fn cancel(&mut self) {
        self.preview.clear();
        self.confirm_on_ready = None;
        self.reveal_category = None;
        self.catalog = None;
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
        self.confirm_on_ready = None;
        self.reveal_category = Category::for_entity(&entity);
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
        self.confirm_on_ready = None;
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
                .entity(id, "New hatch")
                .expect("finite defaults"),
        };
        self.reveal_category = Category::for_entity(&entity);
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
        self.create_stock_from(
            s,
            [
                "New panel stock",
                "New sheet stock",
                "New profile stock",
                "New print stock",
            ][kind],
            stock_default(kind),
        );
    }
    fn create_stock_from(&mut self, s: &Session, name: &str, specification: StockKind) {
        self.create(s, 1);
        let draft = self.draft.as_mut().expect("created stock");
        let material = stock_material(&draft.entity).expect("stock material");
        if let EntityData::Extension { definition, inputs } = &mut draft.entity.data {
            let stock = LibraryDefinition::Stock(Stock {
                material,
                specification,
            });
            definition.data = serde_json::to_value(&stock).expect("stock draft");
            *inputs = stock.links();
        }
        draft.entity.name = name.into();
        self.reveal_category = Category::for_entity(&draft.entity);
    }
    pub(crate) fn for_model_stock(s: &Session, kind: ToolStock) -> Self {
        let mut editor = Self::default();
        match kind {
            ToolStock::Panel => editor.create_stock(s, 0),
            ToolStock::Sheet => editor.create_stock(s, 1),
            ToolStock::Profile => editor.create_stock(s, 2),
            ToolStock::Batten => editor.create_stock_from(
                s,
                "New solid rectangular stock",
                StockKind::Profile {
                    designation: "Solid rectangular bar".into(),
                    length_mm: 6000.,
                    width_mm: 30.,
                    height_mm: 30.,
                    wall_mm: None,
                    shape: ProfileShape::Rectangular,
                },
            ),
        }
        editor
    }
    pub(crate) fn show_model_stock(&mut self, ui: &mut egui::Ui, s: &Session, kind: ToolStock) {
        ui.strong(kind.label().trim_end_matches('…'));
        ui.small("Save a shared Project definition and select it in the current Model tool.");
        if let Some(draft) = &mut self.draft {
            ui.push_id(draft.entity.id, |ui| edit_draft(ui, s, draft, Some(kind)));
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
    pub(crate) fn prepare_model_stock(&self, s: &Session) -> Result<(EntityId, Prepared), String> {
        let draft = self
            .draft
            .as_ref()
            .ok_or("Start a stock definition first")?;
        let EntityData::Extension { definition, .. } = &draft.entity.data else {
            return Err("Choose a stock definition".into());
        };
        let definition: LibraryDefinition = serde_json::from_value(definition.data.clone())
            .map_err(|_| "Enter valid stock dimensions")?;
        let LibraryDefinition::Stock(stock) = &definition else {
            return Err("Choose a stock definition".into());
        };
        if draft.entity.name.trim().is_empty() {
            return Err("Enter a stock name".into());
        }
        if draft.companion.as_ref().is_some_and(|material| {
            material.id == stock.material && material.name.trim().is_empty()
        }) {
            return Err("Enter a manufacturing material name".into());
        }
        if definition.validate().is_err() {
            return Err(match stock.specification {
                StockKind::Panel { .. } => "Enter positive panel thickness, length, and width.",
                StockKind::Sheet { .. } => "Enter positive sheet dimensions, a nonnegative bend radius, and a K factor from 0 to 1.",
                StockKind::Profile { shape: ProfileShape::ItemLine8_40x40, .. } =>
                    "Enter a designation and a stock length above 0 and at most 6000 mm.",
                _ => "Enter a designation and positive section dimensions. Wall thickness must be positive and no more than half the smaller section dimension.",
            }.into());
        }
        self.prepare(s, Channel::Ui)
            .map(|prepared| (draft.entity.id, prepared))
    }
    pub(crate) fn model_stock_error(&mut self, error: String) {
        self.error = Some(error);
    }
    pub(crate) fn valid_panel_draft(&self) -> bool {
        let Some(draft) = &self.draft else {
            return true;
        };
        let EntityData::Extension { definition, .. } = &draft.entity.data else {
            return true;
        };
        let Ok(value) = serde_json::from_value::<LibraryDefinition>(definition.data.clone()) else {
            return true;
        };
        !matches!(
            &value,
            LibraryDefinition::Stock(Stock {
                specification: StockKind::Panel { .. },
                ..
            })
        ) || (!draft.entity.name.trim().is_empty() && value.validate().is_ok())
    }
    pub(crate) fn show_ribbon(&mut self, ui: &mut egui::Ui, s: &Session, editable: bool) {
        use crate::icons::{Icon, tool_with_hint};

        ui.horizontal_wrapped(|ui| {
            for (label, icon, kind) in [
                ("New material", Icon::Filled, 0),
                ("New panel stock", Icon::Rectangle, 1),
                ("New sheet stock", Icon::Page, 2),
                ("New profile stock", Icon::Extrude, 3),
                ("New print stock", Icon::PrintPlate, 4),
                ("New appearance", Icon::VisibleEdges, 5),
                ("New hatch", Icon::Grid, 6),
            ] {
                if ui
                    .add_enabled_ui(editable && !self.dirty(), |ui| {
                        tool_with_hint(ui, icon, label, "Create a shared Project definition")
                    })
                    .inner
                    .clicked()
                {
                    match kind {
                        1..=4 => self.create_stock(s, kind - 1),
                        5 => self.create(s, 2),
                        6 => self.create(s, 3),
                        _ => self.create(s, 0),
                    }
                }
            }
        });
    }

    pub fn show_inline(
        &mut self,
        ui: &mut egui::Ui,
        s: &mut Session,
        resources: &multicad_core::project_file::ProjectResources,
        editable: bool,
    ) {
        self.open(s);
        self.poll(s, resources);
        if !self.pending()
            && let Some(channel) = self.confirm_on_ready.take()
            && self.can_apply(s, resources)
            && let Err(error) = self.apply(s, resources, channel)
        {
            self.error = Some(error);
        }

        let width = ui.available_width();
        egui::Panel::left("project-material-tree")
            .exact_size((width * 0.24).clamp(110., 238.))
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(12),
            )
            .show(ui, |ui| self.show_tree(ui, s, editable));
        egui::Panel::right("project-material-preview")
            .exact_size((width * 0.28).clamp(116., 296.))
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(12),
            )
            .show(ui, |ui| sample::show(ui, s.document(), self.draft.as_ref()));

        use crate::inspector_actions::{self, Button, Owner};
        let dirty = self.dirty();
        let can_apply = editable && self.can_confirm(s);
        let pending = self.pending().then_some("Checking affected geometry…");
        let mut clicks = inspector_actions::Clicks::default();
        let accent = ui.visuals().selection.bg_fill;
        inspector_actions::scope_with_preview(
            ui,
            dirty.then_some(Owner::Materials),
            accent,
            true,
            |ui| {
                if dirty {
                    clicks = inspector_actions::row_with_progress(
                        ui,
                        Owner::Materials,
                        None,
                        Button::new("Apply", can_apply, "Apply the current definition"),
                        Button::new("Discard", true, "Discard edits to this definition"),
                        pending,
                    );
                }
                egui::Frame::new().inner_margin(16).show(ui, |ui| {
                    ui.strong("Configuration");
                    ui.add_space(8.);
                    if !editable {
                        ui.colored_label(
                            Color32::LIGHT_YELLOW,
                            "Finish the pending parameter edits before editing definitions.",
                        );
                    }
                    if let Some(error) = &self.error {
                        ui.colored_label(Color32::LIGHT_RED, error);
                    }
                    if self.pending() {
                        ui.horizontal_wrapped(|ui| {
                            ui.spinner();
                            ui.small("Checking affected geometry…");
                        });
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(16));
                    }
                    if self
                        .guard
                        .as_ref()
                        .is_none_or(|g| s.validate_stamp(g).is_err())
                    {
                        ui.colored_label(
                            Color32::LIGHT_YELLOW,
                            "The project changed. Discard this draft and reopen the definition.",
                        );
                    }
                    self.show_definition_editor(ui, s, editable && self.confirm_on_ready.is_none());
                    self.show_diagnostics(ui, s);
                });
            },
        );
        if clicks.cancel {
            self.discard(s);
        } else if clicks.apply
            && let Err(error) = self.confirm(s, resources, Channel::Ui)
        {
            self.error = Some(error);
        }
        if clicks.cancel || clicks.apply {
            ui.ctx().request_repaint();
        }
    }

    fn show_tree(&mut self, ui: &mut egui::Ui, s: &Session, editable: bool) {
        if self
            .catalog
            .as_ref()
            .is_none_or(|(stamp, _)| *stamp != s.stamp())
        {
            let mut categories: [Vec<&Entity>; Category::ALL.len()] = Default::default();
            for entity in s.document().entities() {
                if let Some(category) = Category::for_entity(entity) {
                    categories[category as usize].push(entity);
                }
            }
            let catalog = categories.map(|mut entries| {
                entries.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                entries.into_iter().map(|entry| entry.id).collect()
            });
            self.swatches = s
                .document()
                .entities()
                .filter_map(|entry| {
                    let Some(LibraryDefinition::MaterialStyle(style)) = read(entry).ok().flatten()
                    else {
                        return None;
                    };
                    let Some(LibraryDefinition::Appearance(appearance)) =
                        read(s.document().get(style.appearance?.id)?).ok().flatten()
                    else {
                        return None;
                    };
                    Some((style.material, appearance.color_rgba))
                })
                .collect();
            self.catalog = Some((s.stamp(), catalog));
        }
        let (_, catalog) = self.catalog.as_ref().expect("current definition catalog");
        let selected = self.draft.as_ref().map(|draft| draft.entity.id);
        let new_draft = self.draft.as_ref().filter(|_| self.original.is_none());
        let draft_category = new_draft.and_then(|draft| Category::for_entity(&draft.entity));
        let can_select = editable && !self.dirty();
        let reveal = self.reveal_category.take();
        let mut selection = None;
        ui.strong("Shared definitions");
        ui.add_space(8.);
        egui::ScrollArea::vertical()
            .id_salt("project-material-tree-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (category, entries) in Category::ALL.into_iter().zip(catalog) {
                    egui::CollapsingHeader::new(category.label())
                        .id_salt(category as usize)
                        .default_open(category == Category::Materials)
                        .open((reveal == Some(category)).then_some(true))
                        .show(ui, |ui| {
                            let new_draft = new_draft.filter(|_| draft_category == Some(category));
                            if entries.is_empty() && new_draft.is_none() {
                                ui.weak("No definitions yet");
                            }
                            for entry in entries.iter().filter_map(|&id| s.document().get(id)) {
                                let response = definition_row(
                                    ui,
                                    self.swatches.get(&entry.id).copied(),
                                    entry,
                                    selected == Some(entry.id),
                                    can_select,
                                );
                                if reveal == Some(category) && selected == Some(entry.id) {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    selection = Some(entry);
                                }
                            }
                            if let Some(draft) = new_draft {
                                definition_row(ui, None, &draft.entity, true, false);
                            }
                        });
                }
            });
        if let Some(entity) = selection {
            self.select(s, entity.clone());
        }
    }

    fn show_diagnostics(&mut self, ui: &mut egui::Ui, s: &Session) {
        if self
            .diagnostics
            .as_ref()
            .is_none_or(|(stamp, _)| *stamp != s.stamp())
        {
            self.diagnostics = Some((s.stamp(), diagnostics(s.document()).unwrap_or_default()));
        }
        if let Some((_, issues)) = &self.diagnostics
            && !issues.is_empty()
        {
            ui.add_space(8.);
            ui.collapsing(format!("Definition diagnostics ({})", issues.len()), |ui| {
                for issue in issues {
                    ui.colored_label(
                        Color32::LIGHT_YELLOW,
                        format!("{}: {}", issue.role, issue.message),
                    );
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
                ui.scope_builder(egui::UiBuilder::new().id(id), |ui| {
                    edit_draft(ui, s, draft, None)
                });
            } else {
                ui.label("Choose or create a reusable definition.");
            }
        });
    }
}
fn definition_row(
    ui: &mut egui::Ui,
    material_override: Option<[u8; 4]>,
    entry: &Entity,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        let category = Category::for_entity(entry);
        if matches!(
            category,
            Some(Category::Materials | Category::Hatches | Category::Appearances)
        ) {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18., 18.), egui::Sense::hover());
            sample::paint_swatch(ui.painter(), rect, material_override, entry);
        }
        ui.add_enabled(
            enabled,
            egui::Button::selectable(selected, &entry.name)
                .min_size(egui::vec2(ui.available_width(), 26.))
                .wrap(),
        )
    })
    .inner
}

fn edit_draft(ui: &mut egui::Ui, s: &Session, d: &mut Draft, model_stock: Option<ToolStock>) {
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
                let [r, g, b, alpha] = a.color_rgba;
                let mut rgb = [r, g, b];
                rgb_fields(ui, &mut rgb);
                a.color_rgba = [rgb[0], rgb[1], rgb[2], alpha];
                ui.horizontal(|ui| {
                    ui.label("Alpha");
                    ui.add(egui::DragValue::new(&mut a.color_rgba[3]));
                });
                number(ui, "Roughness", &mut a.roughness, "");
                number(ui, "Metallic", &mut a.metallic, "");
            }
            LibraryDefinition::Hatch(h) => {
                sample::hatch_picker(ui, h);
                ui.label("Hatch color");
                let mut rgb = h.color_rgb.unwrap_or([0; 3]);
                if rgb_fields(ui, &mut rgb) {
                    h.color_rgb = Some(rgb);
                }
                if h.color_rgb.is_some() && ui.small_button("Use default color").clicked() {
                    h.color_rgb = None;
                }
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
                if model_stock.is_none() {
                    let mut kind = stock_index(&stock.specification);
                    let old = kind;
                    egui::ComboBox::from_id_salt("stock-kind")
                        .selected_text(
                            ["Panel", "Sheet metal", "Profile", "Printed material"][kind],
                        )
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
                }
                stock_controls(
                    ui,
                    &mut stock.specification,
                    model_stock == Some(ToolStock::Batten),
                );
                if model_stock.is_none() {
                    ui.small("Tools that use this stock read its live thickness or section. Applying checks any affected geometry.");
                }
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
fn rgb_fields(ui: &mut egui::Ui, rgb: &mut [u8; 3]) -> bool {
    let before = *rgb;
    let id = ui.id().with("rgb-mode");
    let mut hex = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or(false);
    ui.horizontal(|ui| {
        ui.color_edit_button_srgb(rgb);
        ui.selectable_value(&mut hex, false, "RGB");
        ui.selectable_value(&mut hex, true, "Hex");
    });
    ui.ctx().data_mut(|data| data.insert_temp(id, hex));
    if hex {
        let field = id.with("hex-value");
        let [r, g, b] = *rgb;
        let mut value = ui
            .ctx()
            .data(|data| data.get_temp::<String>(field))
            .unwrap_or_else(|| format!("#{r:02X}{g:02X}{b:02X}"));
        let response = ui.add(
            egui::TextEdit::singleline(&mut value)
                .id(field)
                .hint_text("#RRGGBB")
                .desired_width(100.),
        );
        let focused = response.has_focus();
        let digits = value.trim();
        let digits = digits.strip_prefix('#').unwrap_or(digits);
        if let Some(packed) = (digits.len() == 6)
            .then(|| u32::from_str_radix(digits, 16).ok())
            .flatten()
        {
            let [_, r, g, b] = packed.to_be_bytes();
            *rgb = [r, g, b];
        } else if focused {
            ui.small("Enter six hexadecimal digits.");
        }
        ui.ctx().data_mut(|data| {
            if focused {
                data.insert_temp(field, value);
            } else {
                data.remove::<String>(field);
            }
        });
    } else {
        ui.horizontal(|ui| {
            for (label, channel) in ["R", "G", "B"].into_iter().zip(rgb.iter_mut()) {
                ui.add(egui::DragValue::new(channel).prefix(format!("{label} ")));
            }
        });
    }
    *rgb != before
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
fn stock_controls(ui: &mut egui::Ui, s: &mut StockKind, solid_rectangular: bool) {
    match s {
        StockKind::Panel {
            thickness_mm,
            length_mm,
            width_mm,
            grain,
        } => {
            for (label, value) in [
                ("Thickness", thickness_mm),
                ("Length", length_mm),
                ("Width", width_mm),
            ] {
                number(ui, label, value, " mm");
                if !value.is_finite() || *value <= 0. {
                    ui.colored_label(Color32::LIGHT_RED, format!("{label} must be above zero."));
                }
            }
            ui.label("Grain direction");
            let label = |grain| match grain {
                Grain::None => "No grain direction",
                Grain::AlongLength => "Along length",
                Grain::AcrossLength => "Across length",
            };
            egui::ComboBox::from_id_salt("grain")
                .selected_text(label(*grain))
                .show_ui(ui, |ui| {
                    for g in [Grain::None, Grain::AlongLength, Grain::AcrossLength] {
                        ui.selectable_value(grain, g, label(g));
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
            if !solid_rectangular {
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
            }
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
            if *shape != ProfileShape::Angle && !solid_rectangular {
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
