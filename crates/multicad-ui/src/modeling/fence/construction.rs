//! Local construction draft and elevation of one representative bay, resolved
//! from the same canonical layout as native generation. No sample path is saved.
use super::super::dimension::DimensionField;
use crate::{controls, egui};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use multicad_core::{
    commands::{PreparationSnapshot, Session},
    document::{Document, EntityData, EntityId},
    model::*,
    parameters,
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq)]
enum PostKind {
    Rectangular,
    Round,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum InfillKind {
    Panel,
    Rail,
    Pickets,
}
impl InfillKind {
    fn label(self) -> &'static str {
        match self {
            Self::Panel => "Panel",
            Self::Rail => "Horizontal rail",
            Self::Pickets => "Vertical pickets / slats",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Selection {
    Post,
    Infill(EntityId),
}

#[derive(Clone)]
struct Row {
    id: EntityId,
    name: String,
    material: EntityId,
    kind: InfillKind,
    bottom: DimensionField,
    height: DimensionField,
    normal: DimensionField,
    depth: DimensionField,
    width: DimensionField,
    panel_width: DimensionField,
    maximum_gap: DimensionField,
    purchased: bool,
    cut: bool,
}
fn field(value: f64) -> DimensionField {
    DimensionField::from_dimension(&length(value), "mm").expect("Millimeters are length units")
}
impl Row {
    fn new(kind: InfillKind, material: EntityId) -> Self {
        Self {
            id: EntityId::new(),
            name: kind.label().into(),
            material,
            kind,
            bottom: field(if kind == InfillKind::Rail { 200. } else { 100. }),
            height: field(if kind == InfillKind::Rail { 60. } else { 1600. }),
            normal: field(if kind == InfillKind::Rail { 20. } else { 0. }),
            depth: field(if kind == InfillKind::Panel { 18. } else { 20. }),
            width: field(70.),
            panel_width: field(1910.),
            maximum_gap: field(100.),
            purchased: false,
            cut: false,
        }
    }
    fn existing(source: &FenceInfill) -> Result<Self, String> {
        let kind = match source.construction {
            FenceInfillKind::Panel { .. } => InfillKind::Panel,
            FenceInfillKind::Rail { .. } => InfillKind::Rail,
            FenceInfillKind::Pickets { .. } => InfillKind::Pickets,
        };
        let mut row = Self::new(kind, source.material);
        row.id = source.id;
        row.name = source.name.clone();
        row.bottom = DimensionField::from_dimension(&source.bottom, "mm")?;
        row.height = DimensionField::from_dimension(&source.height, "mm")?;
        row.normal = DimensionField::from_dimension(&source.normal_offset, "mm")?;
        let dimension = |d| DimensionField::from_dimension(d, "mm");
        match &source.construction {
            FenceInfillKind::Panel { thickness, width } => {
                row.depth = dimension(thickness)?;
                if let FencePanelWidth::Purchased { width, short_bay } = width {
                    row.purchased = true;
                    row.panel_width = dimension(width)?;
                    row.cut = *short_bay == FenceShortPanelPolicy::Cut;
                }
            }
            FenceInfillKind::Rail { depth } => row.depth = dimension(depth)?,
            FenceInfillKind::Pickets {
                width,
                depth,
                maximum_gap,
            } => {
                row.width = dimension(width)?;
                row.depth = dimension(depth)?;
                row.maximum_gap = dimension(maximum_gap)?;
            }
        }
        Ok(row)
    }
    fn value(&self, document: &Document) -> Result<FenceInfill, String> {
        Ok(FenceInfill {
            id: self.id,
            name: self.name.clone(),
            material: self.material,
            bottom: self.bottom.resolve(document)?,
            height: self.height.resolve(document)?,
            normal_offset: self.normal.resolve(document)?,
            construction: match self.kind {
                InfillKind::Panel => FenceInfillKind::Panel {
                    thickness: self.depth.resolve(document)?,
                    width: if self.purchased {
                        FencePanelWidth::Purchased {
                            width: self.panel_width.resolve(document)?,
                            short_bay: if self.cut {
                                FenceShortPanelPolicy::Cut
                            } else {
                                FenceShortPanelPolicy::Reject
                            },
                        }
                    } else {
                        FencePanelWidth::FitBay
                    },
                },
                InfillKind::Rail => FenceInfillKind::Rail {
                    depth: self.depth.resolve(document)?,
                },
                InfillKind::Pickets => FenceInfillKind::Pickets {
                    width: self.width.resolve(document)?,
                    depth: self.depth.resolve(document)?,
                    maximum_gap: self.maximum_gap.resolve(document)?,
                },
            },
        })
    }
}

pub(super) struct ConstructionEditor {
    post: PostKind,
    post_material: EntityId,
    post_width: DimensionField,
    post_depth: DimensionField,
    diameter: DimensionField,
    height: DimensionField,
    spacing: DimensionField,
    layout: FenceBayLayout,
    rows: Vec<Row>,
    selected: Selection,
    source: Option<PreparationSnapshot>,
    materials: Vec<(EntityId, String)>,
    colors: BTreeMap<EntityId, Color32>,
    example: Option<Result<Arc<FenceLayout>, String>>,
    example_vertices: [EntityId; 2],
}
impl ConstructionEditor {
    pub fn new(document: &Document) -> Result<Self, String> {
        let material = document
            .entities()
            .find(|e| matches!(e.data, EntityData::Material { .. }))
            .map(|e| e.id)
            .ok_or("Create a material before defining a fence")?;
        let mut value = Self {
            post: PostKind::Rectangular,
            post_material: material,
            post_width: field(90.),
            post_depth: field(90.),
            diameter: field(90.),
            height: field(1800.),
            spacing: field(2000.),
            layout: FenceBayLayout::ShortLast,
            rows: vec![Row::new(InfillKind::Panel, material)],
            selected: Selection::Post,
            source: None,
            materials: Vec::new(),
            colors: BTreeMap::new(),
            example: None,
            example_vertices: [EntityId::new(), EntityId::new()],
        };
        value.refresh_materials(document);
        Ok(value)
    }
    pub fn existing(document: &Document, source: &FenceConstruction) -> Result<Self, String> {
        let mut value = Self::new(document)?;
        value.post_material = source.post_material;
        value.height = DimensionField::from_dimension(&source.height, "mm")?;
        value.spacing = DimensionField::from_dimension(&source.preferred_post_spacing, "mm")?;
        value.layout = source.layout;
        match &source.post {
            FencePostProfile::Rectangular { width, depth } => {
                value.post_width = DimensionField::from_dimension(width, "mm")?;
                value.post_depth = DimensionField::from_dimension(depth, "mm")?;
            }
            FencePostProfile::Round { diameter } => {
                value.post = PostKind::Round;
                value.diameter = DimensionField::from_dimension(diameter, "mm")?;
            }
        }
        value.rows = source
            .infills
            .iter()
            .map(Row::existing)
            .collect::<Result<_, _>>()?;
        Ok(value)
    }
    pub fn sync_source(&mut self, session: &Session) {
        if self
            .source
            .as_ref()
            .is_some_and(|s| s.session().stamp() == session.stamp())
        {
            return;
        }
        self.source = Some(session.preparation_snapshot());
        self.refresh_materials(session.document());
        self.example = None;
    }
    fn refresh_materials(&mut self, document: &Document) {
        self.materials = document
            .entities()
            .filter(|e| matches!(e.data, EntityData::Material { .. }))
            .map(|e| (e.id, e.name.clone()))
            .collect();
        self.colors.clear();
        if let Ok(resolver) = multicad_core::materials::Resolver::new(document) {
            for (id, _) in &self.materials {
                if let Ok(properties) = resolver.resolve_material(*id) {
                    let c = properties.appearance.color_rgba;
                    self.colors
                        .insert(*id, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
                }
            }
        }
    }
    pub fn value(&self, document: &Document) -> Result<FenceConstruction, String> {
        let value = FenceConstruction {
            height: self.height.resolve(document)?,
            post: match self.post {
                PostKind::Rectangular => FencePostProfile::Rectangular {
                    width: self.post_width.resolve(document)?,
                    depth: self.post_depth.resolve(document)?,
                },
                PostKind::Round => FencePostProfile::Round {
                    diameter: self.diameter.resolve(document)?,
                },
            },
            post_material: self.post_material,
            preferred_post_spacing: self.spacing.resolve(document)?,
            layout: self.layout,
            infills: self
                .rows
                .iter()
                .map(|r| r.value(document))
                .collect::<Result<_, _>>()?,
        };
        value.validate().map_err(|e| e.to_string())?;
        Ok(value)
    }
    fn refresh_example(&mut self, document: &Document) {
        if self.example.is_some() {
            return;
        }
        self.example = Some((|| {
            let value = self.value(document)?;
            let owned;
            let parameters = if let Some(source) = &self.source {
                source.session().parameters()
            } else {
                owned = parameters::evaluate(document).map_err(|e| e.to_string())?;
                &owned
            };
            let pitch = value
                .resolve(document, parameters)
                .map_err(|e| e.to_string())?
                .preferred_post_spacing_mm;
            let path = FencePath {
                vertices: vec![
                    FenceVertex {
                        id: self.example_vertices[0],
                        point: [length(0.), length(0.)],
                    },
                    FenceVertex {
                        id: self.example_vertices[1],
                        point: [length(pitch), length(0.)],
                    },
                ],
                closed: false,
                base_z: length(0.),
            };
            plan_fence(document, parameters, &value, &path, &[])
                .map(Arc::new)
                .map_err(|e| e.to_string())
        })());
    }
    fn material(
        ui: &mut egui::Ui,
        choices: &[(EntityId, String)],
        selected: &mut EntityId,
    ) -> bool {
        let mut changed = false;
        egui::ComboBox::from_id_salt("material")
            .selected_text(
                choices
                    .iter()
                    .find(|(id, _)| id == selected)
                    .map_or("Missing material", |(_, name)| name.as_str()),
            )
            .width(controls::inspector_width(ui))
            .show_ui(ui, |ui| {
                for (id, name) in choices {
                    changed |= ui.selectable_value(selected, *id, name).changed();
                }
            });
        changed
    }
    fn diagram(&mut self, ui: &mut egui::Ui, document: &Document) {
        self.refresh_example(document);
        ui.small("Representative bay · elevation");
        // Validation can change after each typed character. Keep the same
        // footprint so fields, focus and the enclosing scroll position stay put.
        let (preview, _) = ui.allocate_exact_size(vec2(ui.available_width(), 280.), Sense::hover());
        let Some(Ok(layout)) = self.example.as_ref() else {
            if let Some(Err(error)) = &self.example {
                ui.place(
                    preview.shrink(12.),
                    egui::Label::new(egui::RichText::new(error).color(Color32::LIGHT_RED)).wrap(),
                );
            }
            return;
        };
        let layout = Arc::clone(layout);
        let rect = Rect::from_min_max(preview.min, preview.max - vec2(0., 50.));
        let post_width = match layout.construction.post {
            ResolvedFencePostProfile::Rectangular { width_mm, .. } => width_mm,
            ResolvedFencePostProfile::Round { diameter_mm } => diameter_mm,
        };
        let width = layout.construction.preferred_post_spacing_mm + post_width;
        let height = layout.construction.height_mm;
        let scale =
            ((rect.width() - 24.) as f64 / width).min((rect.height() - 40.) as f64 / height);
        let left = rect.center().x - (width * scale / 2.) as f32;
        let bottom = rect.bottom() - 24.;
        let area = |x: f64, z: f64, w: f64, h: f64| {
            Rect::from_min_max(
                pos2(
                    left + ((x - w / 2. + post_width / 2.) * scale) as f32,
                    bottom - ((z + h) * scale) as f32,
                ),
                pos2(
                    left + ((x + w / 2. + post_width / 2.) * scale) as f32,
                    bottom - (z * scale) as f32,
                ),
            )
        };
        let painter = ui.painter();
        let neutral = ui.visuals().widgets.inactive.bg_fill;
        for member in &layout.infills {
            let r = area(
                member.position_mm[0],
                member.position_mm[2],
                member.width_mm,
                member.height_mm,
            );
            let selected = self.selected == Selection::Infill(member.key.recipe);
            painter.rect(
                r,
                0.,
                self.colors
                    .get(&member.material)
                    .copied()
                    .unwrap_or(neutral),
                Stroke::new(
                    if selected { 2. } else { 1. },
                    if selected {
                        ui.visuals().selection.stroke.color
                    } else {
                        ui.visuals().text_color()
                    },
                ),
                StrokeKind::Inside,
            );
            if ui
                .interact(
                    r,
                    ui.id()
                        .with(("infill", member.key.recipe, member.key.ordinal)),
                    Sense::click(),
                )
                .clicked()
            {
                self.selected = Selection::Infill(member.key.recipe);
            }
        }
        for (index, post) in layout.posts.iter().enumerate() {
            let r = area(post.position_mm[0], 0., post_width, height);
            painter.rect(
                r,
                0.,
                self.colors
                    .get(&layout.construction.post_material)
                    .copied()
                    .unwrap_or(neutral),
                Stroke::new(
                    if self.selected == Selection::Post {
                        2.
                    } else {
                        1.
                    },
                    ui.visuals().text_color(),
                ),
                StrokeKind::Inside,
            );
            if ui
                .interact(r, ui.id().with(("post", index)), Sense::click())
                .clicked()
            {
                self.selected = Selection::Post;
            }
        }
        painter.line_segment(
            [
                pos2(left, bottom + 5.),
                pos2(left + (width * scale) as f32, bottom + 5.),
            ],
            Stroke::new(1., ui.visuals().weak_text_color()),
        );
        let status = format!(
            "Center spacing {}\nClear bay {}",
            crate::preferences::format_mm(layout.construction.preferred_post_spacing_mm),
            crate::preferences::format_mm(layout.bays[0].clear_width_mm)
        );
        ui.place(
            Rect::from_min_max(pos2(preview.left(), rect.bottom()), preview.max),
            egui::Label::new(egui::RichText::new(status).small().weak()).wrap(),
        )
        .on_hover_text("Click a post or infill, or use the labeled list below.");
    }
    fn show_post(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.label("Post profile");
        egui::ComboBox::from_id_salt("post-profile")
            .selected_text(match self.post {
                PostKind::Rectangular => "Rectangular",
                PostKind::Round => "Round",
            })
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut self.post, PostKind::Rectangular, "Rectangular")
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.post, PostKind::Round, "Round")
                    .changed();
            });
        ui.label("Post material");
        changed |= Self::material(ui, &self.materials, &mut self.post_material);
        egui::Grid::new("post-dimensions")
            .num_columns(2)
            .show(ui, |ui| match self.post {
                PostKind::Rectangular => {
                    changed |= self.post_width.show_compact(
                        ui,
                        ("fence-construction", "post-width"),
                        "Along-run width",
                        "Width along the local path direction",
                    );
                    ui.end_row();
                    changed |= self.post_depth.show_compact(
                        ui,
                        ("fence-construction", "post-depth"),
                        "Across-run depth",
                        "Depth across the local path direction",
                    );
                    ui.end_row();
                }
                PostKind::Round => {
                    changed |= self.diameter.show_compact(
                        ui,
                        ("fence-construction", "post-diameter"),
                        "Diameter",
                        "Physical circular post diameter",
                    );
                    ui.end_row();
                }
            });
        changed
    }

    fn show_infill(&mut self, ui: &mut egui::Ui, index: usize) -> bool {
        let mut changed = false;
        let can_duplicate = self.rows.len() < MAX_FENCE_INFILL_RECIPES;
        let can_down = index + 1 < self.rows.len();
        let row = &mut self.rows[index];
        egui::ComboBox::from_id_salt("infill-kind")
            .selected_text(row.kind.label())
            .show_ui(ui, |ui| {
                for kind in [InfillKind::Panel, InfillKind::Rail, InfillKind::Pickets] {
                    changed |= ui
                        .selectable_value(&mut row.kind, kind, kind.label())
                        .changed();
                }
            });
        ui.label("Infill name");
        changed |= ui.text_edit_singleline(&mut row.name).changed();
        ui.label("Infill material");
        changed |= Self::material(ui, &self.materials, &mut row.material);
        egui::Grid::new("infill-dimensions").num_columns(2).show(ui, |ui| {
            changed |= row.bottom.show_compact(ui, ("fence-infill", row.id, "bottom"), "Bottom above base", "Physical bottom measured from the run base");
            ui.end_row();
            changed |= row.height.show_compact(ui, ("fence-infill", row.id, "height"), "Height", "Physical height of this panel, rail or picket");
            ui.end_row();
            changed |= row.normal.show_compact(ui, ("fence-infill", row.id, "normal"), "Across-run offset", "Positive moves left of the drawn direction. Place rail faces against picket faces to avoid overlap.");
            ui.end_row();
            changed |= row.depth.show_compact(ui, ("fence-infill", row.id, "depth"), if row.kind == InfillKind::Panel { "Thickness" } else { "Depth" }, "Physical material depth across the path");
            ui.end_row();
            if row.kind == InfillKind::Pickets {
                changed |= row.width.show_compact(ui, ("fence-infill", row.id, "width"), "Picket width", "Repeated members retain this width");
                ui.end_row();
                changed |= row.maximum_gap.show_compact(ui, ("fence-infill", row.id, "maximum_gap"), "Maximum clear gap", "Equal end and interior gaps stay within this limit");
                ui.end_row();
            }
        });
        if row.kind == InfillKind::Panel {
            changed |= ui
                .checkbox(&mut row.purchased, "Purchased fixed-width panel")
                .changed();
            if row.purchased {
                egui::Grid::new("purchased-panel")
                    .num_columns(2)
                    .show(ui, |ui| {
                        changed |= row.panel_width.show_compact(
                            ui,
                            ("fence-infill", row.id, "panel_width"),
                            "Purchased width",
                            "Choose post spacing that fits this physical panel",
                        );
                        ui.end_row();
                    });
                changed |= ui
                    .checkbox(&mut row.cut, "Permit cutting down for shorter bays")
                    .changed();
                ui.small("Preview refuses a longer bay or an unapproved short panel.");
            } else {
                ui.small("Custom panel made to fit each clear bay.");
            }
        }
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(can_duplicate, egui::Button::new("Duplicate infill"))
                .clicked()
            {
                action = Some(RowAction::Duplicate);
            }
            if ui.button("Remove infill").clicked() {
                action = Some(RowAction::Remove);
            }
            if ui
                .add_enabled(index > 0, egui::Button::new("Move up"))
                .clicked()
            {
                action = Some(RowAction::Up);
            }
            if ui
                .add_enabled(can_down, egui::Button::new("Move down"))
                .clicked()
            {
                action = Some(RowAction::Down);
            }
        });
        match action {
            Some(RowAction::Duplicate) => {
                let mut row = self.rows[index].clone();
                row.id = EntityId::new();
                row.name.push_str(" copy");
                self.selected = Selection::Infill(row.id);
                self.rows.insert(index + 1, row);
            }
            Some(RowAction::Remove) => {
                self.rows.remove(index);
                self.selected = Selection::Post;
            }
            Some(RowAction::Up) => self.rows.swap(index, index - 1),
            Some(RowAction::Down) => self.rows.swap(index, index + 1),
            None => {}
        }
        changed || action.is_some()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document, editable: bool) -> bool {
        self.diagram(ui, document);
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(self.selected == Selection::Post, "Posts")
                .clicked()
            {
                self.selected = Selection::Post;
            }
            for row in &self.rows {
                if ui
                    .selectable_label(self.selected == Selection::Infill(row.id), &row.name)
                    .clicked()
                {
                    self.selected = Selection::Infill(row.id);
                }
            }
        });
        let mut changed = false;
        ui.add_enabled_ui(editable, |ui| {
            egui::Grid::new("construction-global")
                .num_columns(2)
                .show(ui, |ui| {
                    changed |= self.height.show_compact(
                        ui,
                        ("fence-construction", "post-height"),
                        "Post height",
                        "Infill keeps its own height when post height changes",
                    );
                    ui.end_row();
                    changed |= self.spacing.show_compact(
                        ui,
                        ("fence-construction", "post-spacing"),
                        "Preferred spacing",
                        "Center-to-center pitch; the run preview shows actual bay widths",
                    );
                    ui.end_row();
                });
            egui::ComboBox::from_id_salt("end-bay-layout")
                .selected_text(match self.layout {
                    FenceBayLayout::Equal => "Equal bays per segment",
                    FenceBayLayout::ShortLast => "Preferred spacing + short last bay",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.layout,
                            FenceBayLayout::Equal,
                            "Equal bays per segment",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.layout,
                            FenceBayLayout::ShortLast,
                            "Preferred spacing + short last bay",
                        )
                        .changed();
                });
            ui.separator();
            match self.selected {
                Selection::Post => changed |= self.show_post(ui),
                Selection::Infill(id) => {
                    if let Some(index) = self.rows.iter().position(|row| row.id == id) {
                        changed |= self.show_infill(ui, index);
                    }
                }
            }
            ui.horizontal_wrapped(|ui| {
                for kind in [InfillKind::Panel, InfillKind::Rail, InfillKind::Pickets] {
                    if ui
                        .add_enabled(
                            self.rows.len() < MAX_FENCE_INFILL_RECIPES,
                            egui::Button::new(format!("Add {}", kind.label())),
                        )
                        .clicked()
                    {
                        let row = Row::new(kind, self.post_material);
                        self.selected = Selection::Infill(row.id);
                        self.rows.push(row);
                        changed = true;
                    }
                }
            });
        });
        if changed {
            self.example = None;
        }
        changed
    }
}

#[derive(Clone, Copy)]
enum RowAction {
    Duplicate,
    Remove,
    Up,
    Down,
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        constraint_sketch::Dimension,
        document::{Entity, Mutation},
        parameters::{BoundExpression, Unit},
    };
    fn fixture() -> (Session, FenceConstruction, EntityId, EntityId) {
        let material = |name: &str, color| {
            Entity::new(
                name,
                EntityData::Material {
                    density_kg_m3: None,
                    color_rgba: color,
                    section_hatch: None,
                },
            )
        };
        let wood = material("Posts", [160, 110, 70, 255]);
        let panel = material("Infill", [80, 130, 170, 255]);
        let doc = Document::new("Bay editor")
            .unwrap()
            .edited(&[
                Mutation::Insert(wood.clone()),
                Mutation::Insert(panel.clone()),
            ])
            .unwrap();
        let value = FenceConstruction {
            height: length(1800.),
            post: FencePostProfile::Rectangular {
                width: length(90.),
                depth: length(90.),
            },
            post_material: wood.id,
            preferred_post_spacing: length(2000.),
            layout: FenceBayLayout::ShortLast,
            infills: vec![FenceInfill {
                id: EntityId::new(),
                name: "Custom panel".into(),
                material: panel.id,
                bottom: length(100.),
                height: length(1600.),
                normal_offset: length(0.),
                construction: FenceInfillKind::Panel {
                    thickness: length(18.),
                    width: FencePanelWidth::FitBay,
                },
            }],
        };
        (Session::new(doc).unwrap(), value, wood.id, panel.id)
    }
    fn frame(
        editor: &mut ConstructionEditor,
        ctx: &egui::Context,
        document: &Document,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(400., 1800.))),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                editor.show(ui, document, true);
            },
        );
        out.textures_delta.clear();
        out
    }
    fn click(
        editor: &mut ConstructionEditor,
        ctx: &egui::Context,
        document: &Document,
        p: egui::Pos2,
    ) {
        for pressed in [true, false] {
            frame(
                editor,
                ctx,
                document,
                vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }
    fn text(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    (text.galley.job.text == label)
                        .then(|| Rect::from_min_size(text.pos, text.galley.size()).center())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Missing control {label}"))
    }

    fn inspector_frame(
        editor: &mut ConstructionEditor,
        ctx: &egui::Context,
        document: &Document,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut fits = true;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200., 1000.))),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                let panel = egui::Panel::right("fence-fields-review")
                    .exact_size(width)
                    .show(ui, |ui| {
                        controls::inspector_layout(ui);
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let available = ui.available_width();
                            editor.show(ui, document, true);
                            fits &= ui.min_rect().width() <= available + 1.;
                        });
                    });
                fits &= panel.response.rect.width() <= width + 1.;
            },
        );
        if !fits {
            output.drop_without_applying_deltas();
            panic!("Fence inspector content exceeds {width}px");
        }
        output
    }

    #[test]
    fn compact_fields_keep_readable_labels_and_focus_through_invalid_bay_edits() {
        let (session, value, _, _) = fixture();
        let original = session.document().clone();
        for width in [480., 250., 160.] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut editor = ConstructionEditor::existing(session.document(), &value).unwrap();
            editor.sync_source(&session);
            for _ in 0..4 {
                inspector_frame(&mut editor, &ctx, session.document(), width, vec![])
                    .drop_without_applying_deltas();
            }
            let output = inspector_frame(&mut editor, &ctx, session.document(), width, vec![]);
            for label in [
                "Post height (mm)",
                "Preferred spacing (mm)",
                "Along-run width (mm)",
                "Across-run depth (mm)",
            ] {
                let galley = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(&text.galley)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("Missing {label}"));
                assert!(
                    galley.rows.len() <= if width < 200. { 3 } else { 2 },
                    "{width}px: {label} wraps into {} rows",
                    galley.rows.len()
                );
            }
            let status_bottom = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.text().starts_with("Center spacing ") =>
                    {
                        Some(text.pos.y + text.galley.size().y)
                    }
                    _ => None,
                })
                .expect("Visible actual bay dimensions");
            assert!(
                status_bottom < text(&output, "Posts").y,
                "Bay summary overlaps controls at {width}px"
            );
            let at = text(&output, "1800");
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                inspector_frame(
                    &mut editor,
                    &ctx,
                    session.document(),
                    width,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                )
                .drop_without_applying_deltas();
            }
            let focused = ctx.memory(|m| m.focused()).expect("height field focused");
            let field_rect = ctx.read_response(focused).unwrap().rect;
            assert!(field_rect.width() <= 130.);
            for digits in ["1600", "1800"] {
                let command = egui::Modifiers {
                    command: true,
                    ctrl: true,
                    ..Default::default()
                };
                inspector_frame(
                    &mut editor,
                    &ctx,
                    session.document(),
                    width,
                    vec![egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: command,
                    }],
                )
                .drop_without_applying_deltas();
                let mut expected = String::new();
                for digit in digits.chars() {
                    expected.push(digit);
                    inspector_frame(
                        &mut editor,
                        &ctx,
                        session.document(),
                        width,
                        vec![egui::Event::Text(digit.to_string())],
                    )
                    .drop_without_applying_deltas();
                    // Repaint the newly invalid/valid representative bay too.
                    let updated =
                        inspector_frame(&mut editor, &ctx, session.document(), width, vec![]);
                    let diagnostic = if let Some(Err(error)) = &editor.example {
                        Some((
                            error.clone(),
                            updated
                                .platform_output
                                .accesskit_update
                                .as_ref()
                                .unwrap()
                                .nodes
                                .iter()
                                .any(|(_, node)| {
                                    node.label() == Some(error.as_str())
                                        || node.value() == Some(error.as_str())
                                }),
                        ))
                    } else {
                        None
                    };
                    updated.drop_without_applying_deltas();
                    if let Some((error, accessible)) = diagnostic {
                        assert!(
                            accessible,
                            "invalid diagnostic {error} must remain accessible"
                        );
                    }
                    assert_eq!(editor.height.text, expected);
                    assert_eq!(ctx.memory(|m| m.focused()), Some(focused));
                    let current = ctx.read_response(focused).unwrap().rect;
                    assert!(
                        (current.min.y - field_rect.min.y).abs() < 0.1,
                        "field moved during invalid edit"
                    );
                    assert!(current.right() <= 1200. && current.left() >= 1200. - width);
                    assert_eq!(editor.post_width.text, "90");
                    assert_eq!(editor.post_depth.text, "90");
                    assert_eq!(editor.spacing.text, "2000");
                }
                assert_eq!(editor.example.as_ref().unwrap().is_ok(), digits == "1800");
            }
            assert_eq!(editor.value(session.document()).unwrap(), value);
        }
        assert_eq!(session.document(), &original);
        assert_eq!(session.revision(), 0);
    }

    #[test]
    fn compact_infill_fields_keep_semantic_identity_when_recipe_selection_changes() {
        let (session, value, _, _) = fixture();
        let mut editor = ConstructionEditor::existing(session.document(), &value).unwrap();
        editor.sync_source(&session);
        let first = editor.rows[0].id;
        let mut second = editor.rows[0].clone();
        second.id = EntityId::new();
        second.name = "Second panel".into();
        second.height.text = "1333".into();
        let second_id = second.id;
        editor.rows.push(second);
        let ctx = egui::Context::default();
        let mut ids = Vec::new();
        for (id, contents) in [(first, "1600"), (second_id, "1333"), (first, "1600")] {
            editor.selected = Selection::Infill(id);
            for _ in 0..3 {
                inspector_frame(&mut editor, &ctx, session.document(), 250., vec![])
                    .drop_without_applying_deltas();
            }
            let output = inspector_frame(&mut editor, &ctx, session.document(), 250., vec![]);
            let at = text(&output, contents);
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                inspector_frame(
                    &mut editor,
                    &ctx,
                    session.document(),
                    250.,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                )
                .drop_without_applying_deltas();
            }
            ids.push(
                ctx.memory(|memory| memory.focused())
                    .expect("infill height focused"),
            );
        }
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids[0], ids[2]);
        assert_eq!(editor.rows[0].height.text, "1600");
        assert_eq!(editor.rows[1].height.text, "1333");
        assert_eq!(session.revision(), 0);
    }

    #[test]
    fn painted_bay_selection_duplicate_and_reorder_preserve_canonical_recipe_identity() {
        let (session, value, _, _) = fixture();
        let before = session.document().clone();
        let mut editor = ConstructionEditor::existing(session.document(), &value).unwrap();
        editor.sync_source(&session);
        let ctx = egui::Context::default();
        let output = frame(&mut editor, &ctx, session.document(), vec![]);
        let panel = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Rect(rect) = &shape.shape {
                    (rect.fill == Color32::from_rgb(80, 130, 170)).then_some(rect.rect)
                } else {
                    None
                }
            })
            .expect("Actual material band");
        click(&mut editor, &ctx, session.document(), panel.center());
        assert_eq!(editor.selected, Selection::Infill(value.infills[0].id));
        assert_eq!(editor.value(session.document()).unwrap(), value);
        let output = frame(&mut editor, &ctx, session.document(), vec![]);
        click(
            &mut editor,
            &ctx,
            session.document(),
            text(&output, "Duplicate infill"),
        );
        let duplicated = editor.value(session.document()).unwrap();
        assert_eq!(duplicated.infills.len(), 2);
        assert_ne!(duplicated.infills[0].id, duplicated.infills[1].id);
        assert_eq!(
            duplicated.infills[0].construction,
            duplicated.infills[1].construction
        );
        let id = duplicated.infills[1].id;
        let output = frame(&mut editor, &ctx, session.document(), vec![]);
        click(
            &mut editor,
            &ctx,
            session.document(),
            text(&output, "Move up"),
        );
        assert_eq!(editor.value(session.document()).unwrap().infills[0].id, id);
        assert_eq!(session.document(), &before);
    }

    #[test]
    fn source_epoch_refresh_updates_materials_and_bound_values_for_same_id_revision() {
        let (session, mut value, wood, _) = fixture();
        let parameter = Entity::new(
            "height",
            EntityData::Parameter {
                expression: "1.8".into(),
                unit: "m".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let first = Session::new(
            session
                .document()
                .edited(&[Mutation::Insert(parameter.clone())])
                .unwrap(),
        )
        .unwrap();
        value.height = Dimension::Expression {
            expression: BoundExpression::bind(first.document(), "=$height", "mm", None).unwrap(),
        };
        let mut editor = ConstructionEditor::existing(first.document(), &value).unwrap();
        editor.sync_source(&first);
        editor.refresh_example(first.document());
        let first_layout = editor.example.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(first_layout.construction.height_mm, 1800.);
        let mut changed = parameter;
        changed.name = "retained_height".into();
        if let EntityData::Parameter { expression, .. } = &mut changed.data {
            *expression = "2".into();
        }
        let reused = Entity::new(
            "height",
            EntityData::Parameter {
                expression: "10".into(),
                unit: "m".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let mut color = first.document().get(wood).unwrap().clone();
        if let EntityData::Material { color_rgba, .. } = &mut color.data {
            *color_rgba = [20, 170, 80, 255];
        }
        let second = Session::new(
            first
                .document()
                .edited(&[
                    Mutation::Replace(changed),
                    Mutation::Insert(reused),
                    Mutation::Replace(color),
                ])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first.document().id(), second.document().id());
        assert_eq!(first.revision(), second.revision());
        editor.sync_source(&second);
        editor.refresh_example(second.document());
        assert_eq!(
            editor
                .example
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .construction
                .height_mm,
            2000.
        );
        assert_eq!(editor.colors[&wood], Color32::from_rgb(20, 170, 80));
        assert_eq!(
            editor
                .value(second.document())
                .unwrap()
                .height
                .value_in(Unit::Millimeter, Some(second.parameters()))
                .unwrap(),
            2000.
        );
    }
}
