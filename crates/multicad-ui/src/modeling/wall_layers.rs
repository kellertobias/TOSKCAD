//! Local construction draft and schematic authoring view. The canonical stack
//! supplies dimensions and identities; only a checked native preview can Apply.
use super::dimension::DimensionField;
use crate::{controls, egui};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use multicad_core::{
    commands::{PreparationSnapshot, Session},
    document::{Document, EntityData, EntityId},
    model::{
        self, WallLayer, WallLayerKind, WallLayerReference, WallLayerSide, WallLayerStack, length,
    },
};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Solid,
    Air,
    Coating,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid material",
            Self::Air => "Air cavity",
            Self::Coating => "Coating (zero volume)",
        }
    }
}
#[derive(Clone)]
struct Row {
    id: EntityId,
    name: String,
    role: String,
    kind: Kind,
    thickness: DimensionField,
    material: Option<EntityId>,
}
impl Row {
    fn from_layer(layer: &WallLayer) -> Result<Self, String> {
        Ok(Self {
            id: layer.id,
            name: layer.name.clone(),
            role: layer.role.clone().unwrap_or_default(),
            kind: match layer.construction {
                WallLayerKind::Solid { .. } => Kind::Solid,
                WallLayerKind::Cavity { .. } => Kind::Air,
                WallLayerKind::Coating { .. } => Kind::Coating,
            },
            thickness: DimensionField::from_dimension(
                layer.construction.thickness().unwrap_or(&length(1.)),
                "mm",
            )?,
            material: layer.construction.material(),
        })
    }
    fn layer(&self, document: &Document) -> Result<WallLayer, String> {
        Ok(WallLayer {
            id: self.id,
            name: self.name.clone(),
            role: (!self.role.trim().is_empty()).then(|| self.role.clone()),
            construction: match self.kind {
                Kind::Solid => WallLayerKind::Solid {
                    thickness: self.thickness.resolve(document)?,
                    material: self
                        .material
                        .ok_or("Select a material for this solid layer")?,
                },
                Kind::Air => WallLayerKind::Cavity {
                    thickness: self.thickness.resolve(document)?,
                },
                Kind::Coating => WallLayerKind::Coating {
                    material: self.material.ok_or("Select a coating material")?,
                },
            },
        })
    }
}
struct MaterialChoice {
    id: EntityId,
    name: String,
    color: Option<Color32>,
}
#[derive(Clone)]
struct Diagram {
    widths: Vec<f64>,
    total: f64,
    resolved: std::sync::Arc<model::ResolvedWallLayerStack>,
}
pub(super) struct LayerEditor {
    rows: Vec<Row>,
    selected: Option<EntityId>,
    pub reference: WallLayerReference,
    source: Option<PreparationSnapshot>,
    materials: Vec<MaterialChoice>,
    diagram: Option<Result<Diagram, String>>,
}
impl LayerEditor {
    pub fn new(document: &Document) -> Result<Self, String> {
        let material = document
            .entities()
            .find(|e| matches!(e.data, EntityData::Material { .. }))
            .map(|e| e.id);
        let row = Row {
            id: EntityId::new(),
            name: "Structure".into(),
            role: String::new(),
            kind: Kind::Solid,
            thickness: DimensionField::from_dimension(&length(200.), "mm")?,
            material,
        };
        let mut editor = Self {
            selected: Some(row.id),
            rows: vec![row],
            reference: WallLayerReference::Center,
            source: None,
            materials: vec![],
            diagram: None,
        };
        editor.refresh_materials(document);
        Ok(editor)
    }
    pub fn existing(
        document: &Document,
        stack: &WallLayerStack,
        reference: WallLayerReference,
    ) -> Result<Self, String> {
        let mut editor = Self::new(document)?;
        editor.rows = stack
            .layers
            .iter()
            .map(Row::from_layer)
            .collect::<Result<_, _>>()?;
        editor.selected = editor.rows.first().map(|r| r.id);
        editor.reference = reference;
        Ok(editor)
    }
    /// The controller supplies a process-local stamp and shared immutable source.
    /// Neither hashing nor parameter/catalog reconstruction runs on idle frames.
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
        self.diagram = None;
    }
    fn refresh_materials(&mut self, document: &Document) {
        let resolver = multicad_core::materials::Resolver::new(document).ok();
        self.materials = document
            .entities()
            .filter(|e| matches!(e.data, EntityData::Material { .. }))
            .map(|e| MaterialChoice {
                id: e.id,
                name: e.name.clone(),
                color: resolver
                    .as_ref()
                    .and_then(|r| r.resolve_material(e.id).ok())
                    .map(|p| {
                        Color32::from_rgba_unmultiplied(
                            p.appearance.color_rgba[0],
                            p.appearance.color_rgba[1],
                            p.appearance.color_rgba[2],
                            p.appearance.color_rgba[3],
                        )
                    }),
            })
            .collect();
        self.materials.sort_by(|a, b| a.name.cmp(&b.name));
    }
    pub fn stack(&self, document: &Document) -> Result<WallLayerStack, String> {
        let stack = WallLayerStack {
            layers: self
                .rows
                .iter()
                .map(|r| r.layer(document))
                .collect::<Result<_, _>>()?,
        };
        stack.validate().map_err(|e| e.to_string())?;
        Ok(stack)
    }
    fn resolve_diagram(&self, document: &Document) -> Result<Diagram, String> {
        let stack = self.stack(document)?;
        let parameters;
        let evaluated = if let Some(source) = &self.source {
            source.session().parameters()
        } else {
            parameters =
                multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
            &parameters
        };
        let resolved = stack
            .resolve(document, evaluated)
            .map_err(|e| e.to_string())?;
        resolved
            .bands(self.reference, false)
            .map_err(|e| e.to_string())?;
        Ok(Diagram {
            widths: resolved.layers.iter().map(|l| l.thickness_mm).collect(),
            total: resolved.thickness_mm,
            resolved: std::sync::Arc::new(resolved),
        })
    }
    fn add(&mut self, kind: Kind) {
        if self.rows.len() >= model::MAX_WALL_LAYERS {
            return;
        }
        let row = Row {
            id: EntityId::new(),
            name: match kind {
                Kind::Solid => "Material layer",
                Kind::Air => "Air cavity",
                Kind::Coating => "Surface coating",
            }
            .into(),
            role: String::new(),
            kind,
            thickness: DimensionField::from_dimension(
                &length(if kind == Kind::Air { 30. } else { 10. }),
                "mm",
            )
            .expect("valid literal"),
            material: self.materials.first().map(|m| m.id),
        };
        self.selected = Some(row.id);
        self.rows.push(row);
        self.diagram = None;
    }
    fn duplicate(&mut self, index: usize) {
        if self.rows.len() >= model::MAX_WALL_LAYERS {
            return;
        }
        let mut row = self.rows[index].clone();
        row.id = EntityId::new();
        while row.name.len() > 251 {
            row.name.pop();
        }
        row.name.push_str(" copy");
        self.selected = Some(row.id);
        self.rows.insert(index + 1, row);
        self.diagram = None;
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        flipped: bool,
        reference: Option<WallLayerReference>,
        editable: bool,
    ) -> bool {
        if self.diagram.is_none() {
            self.diagram = Some(self.resolve_diagram(document));
        }
        let diagram = self.diagram.as_ref().expect("resolved above").clone();
        ui.horizontal(|ui| {
            ui.strong(if flipped {
                "Inside ← Outside (flipped)"
            } else {
                "Outside → Inside"
            });
        });
        ui.small(
            "Bands use true proportions. Select thin layers or coatings by their labeled row.",
        );
        if let Ok(diagram) = &diagram {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 68.), Sense::hover());
            let mut x = if flipped { rect.right() } else { rect.left() };
            let mut picked = None;
            for (row, width) in self.rows.iter().zip(&diagram.widths) {
                let next = x + if flipped { -1. } else { 1. }
                    * rect.width()
                    * (*width / diagram.total) as f32;
                let band = Rect::from_min_max(
                    pos2(x.min(next), rect.top() + 6.),
                    pos2(x.max(next), rect.bottom() - 6.),
                );
                let selected = self.selected == Some(row.id);
                let color = row
                    .material
                    .and_then(|id| self.materials.iter().find(|m| m.id == id))
                    .and_then(|m| m.color)
                    .unwrap_or(ui.visuals().faint_bg_color);
                if row.kind == Kind::Coating {
                    ui.painter().line_segment(
                        [band.left_top(), band.left_bottom()],
                        Stroke::new(if selected { 3. } else { 1. }, color),
                    );
                } else {
                    ui.painter().rect_filled(band, 0., color);
                    ui.painter().rect_stroke(
                        band,
                        0.,
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
                        .interact(band, ui.id().with(row.id), Sense::click())
                        .on_hover_text(format!("{} · {} · {width} mm", row.name, row.kind.label()))
                        .clicked()
                    {
                        picked = Some(row.id);
                    }
                }
                x = next;
            }
            match diagram
                .resolved
                .bands(reference.unwrap_or(self.reference), false)
            {
                Ok(bands) => {
                    let fraction = bands.first().map_or(0., |b| b.outside_mm) / diagram.total;
                    let fraction = if flipped { 1. - fraction } else { fraction };
                    let reference_x = rect.left() + rect.width() * fraction as f32;
                    ui.painter().line_segment(
                        [
                            pos2(reference_x, rect.top()),
                            pos2(reference_x, rect.bottom()),
                        ],
                        Stroke::new(1.5, ui.visuals().selection.stroke.color),
                    );
                }
                Err(error) => {
                    ui.colored_label(Color32::LIGHT_RED, error.to_string());
                }
            }
            if let Some(id) = picked {
                self.selected = Some(id);
            }
            ui.label(format!("Total: {:.3} mm (derived)", diagram.total));
            ui.small("Accent line: drawn path reference. Flip mirrors placement, preserving the Outside-to-Inside order.");
        } else if let Err(error) = &diagram {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        let mut changed = false;
        for row in &self.rows {
            if ui
                .selectable_label(
                    self.selected == Some(row.id),
                    format!("{} · {}", row.name, row.kind.label()),
                )
                .clicked()
            {
                self.selected = Some(row.id);
            }
        }
        ui.add_enabled_ui(editable, |ui| {
        ui.horizontal_wrapped(|ui| {
            let room = self.rows.len() < model::MAX_WALL_LAYERS;
            if ui
                .add_enabled(room, egui::Button::new("Add solid"))
                .clicked()
            {
                self.add(Kind::Solid);
                changed = true;
            }
            if ui.add_enabled(room, egui::Button::new("Add air")).clicked() {
                self.add(Kind::Air);
                changed = true;
            }
            if ui
                .add_enabled(room, egui::Button::new("Add coating"))
                .clicked()
            {
                self.add(Kind::Coating);
                changed = true;
            }
        });
        if let Some(index) = self
            .selected
            .and_then(|id| self.rows.iter().position(|r| r.id == id))
        {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                    self.rows.swap(index, index - 1);
                    changed = true;
                }
                if ui
                    .add_enabled(index + 1 < self.rows.len(), egui::Button::new("Down"))
                    .clicked()
                {
                    self.rows.swap(index, index + 1);
                    changed = true;
                }
                if ui
                    .add_enabled(
                        self.rows.len() < model::MAX_WALL_LAYERS,
                        egui::Button::new("Duplicate"),
                    )
                    .clicked()
                {
                    self.duplicate(index);
                    changed = true;
                }
                if ui.button("Remove").clicked() {
                    self.rows.remove(index);
                    self.selected = self
                        .rows
                        .get(index.min(self.rows.len().saturating_sub(1)))
                        .map(|r| r.id);
                    changed = true;
                }
            });
        }
        if let Some(row) = self
            .selected
            .and_then(|id| self.rows.iter_mut().find(|r| r.id == id))
        {
            ui.label("Layer name");
            changed |= ui.text_edit_singleline(&mut row.name).changed();
            ui.label("Role (optional)");
            changed |= ui.text_edit_singleline(&mut row.role).changed();
            controls::inspector_combo(ui, "layer-kind")
                .selected_text(row.kind.label())
                .show_ui(ui, |ui| {
                    for kind in [Kind::Solid, Kind::Air, Kind::Coating] {
                        changed |= ui
                            .selectable_value(&mut row.kind, kind, kind.label())
                            .changed();
                    }
                });
            if row.kind != Kind::Coating {
                changed |= row.thickness.show(
                    ui,
                    "Layer thickness",
                    "The physical construction width; not a display exaggeration",
                );
            }
            if row.kind != Kind::Air {
                controls::inspector_combo(ui, "layer-material")
                    .selected_text(
                        self.materials
                            .iter()
                            .find(|m| Some(m.id) == row.material)
                            .map_or("Select material", |m| m.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for material in &self.materials {
                            changed |= ui
                                .selectable_value(
                                    &mut row.material,
                                    Some(material.id),
                                    &material.name,
                                )
                                .changed();
                        }
                    });
                if self.materials.is_empty() {
                    ui.small("Add a material in the Library to define solid layers and coatings.");
                }
                if row
                    .material
                    .and_then(|id| self.materials.iter().find(|m| m.id == id))
                    .is_some_and(|m| m.color.is_none())
                {
                    ui.colored_label(Color32::LIGHT_RED,"Material appearance needs repair; native preview will report the broken assignment.");
                }
            }
        }
        ui.label("Construction reference");
        let references = self.references();
        controls::inspector_combo(ui, "layer-reference")
            .selected_text(
                references
                    .iter()
                    .find(|(r, _)| *r == self.reference)
                    .map_or("Missing boundary — choose a reference", |(_, label)| {
                        label.as_str()
                    }),
            )
            .show_ui(ui, |ui| {
                for (reference, label) in references {
                    changed |= ui
                        .selectable_value(&mut self.reference, reference, label)
                        .changed();
                }
            });
        });
        if changed {
            self.diagram = None;
        }
        changed
    }
    pub fn references(&self) -> Vec<(WallLayerReference, String)> {
        let mut references = vec![
            (WallLayerReference::Outside, "Outside face".into()),
            (WallLayerReference::Center, "Construction center".into()),
            (WallLayerReference::Inside, "Inside face".into()),
        ];
        for row in &self.rows {
            for side in [WallLayerSide::Outside, WallLayerSide::Inside] {
                references.push((
                    WallLayerReference::Boundary {
                        layer: row.id,
                        side,
                    },
                    format!("{} · {side:?} boundary", row.name),
                ));
            }
        }
        references
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::Session,
        document::{Entity, Mutation},
    };
    fn fixture() -> (Session, WallLayerStack, EntityId) {
        let material = Entity::new(
            "Brick",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [160, 80, 60, 255],
                section_hatch: None,
            },
        );
        let document = Document::new("Layer editor")
            .unwrap()
            .edited(&[Mutation::Insert(material.clone())])
            .unwrap();
        let layer = |name: &str, construction| WallLayer {
            id: EntityId::new(),
            name: name.into(),
            role: None,
            construction,
        };
        let stack = WallLayerStack {
            layers: vec![
                layer(
                    "Paint",
                    WallLayerKind::Coating {
                        material: material.id,
                    },
                ),
                layer(
                    "Outer brick",
                    WallLayerKind::Solid {
                        material: material.id,
                        thickness: length(100.),
                    },
                ),
                layer(
                    "Air",
                    WallLayerKind::Cavity {
                        thickness: length(30.),
                    },
                ),
                layer(
                    "Lining",
                    WallLayerKind::Solid {
                        material: material.id,
                        thickness: length(10.),
                    },
                ),
            ],
        };
        (Session::new(document).unwrap(), stack, material.id)
    }
    fn frame(
        editor: &mut LayerEditor,
        ctx: &egui::Context,
        document: &Document,
        events: Vec<egui::Event>,
        flipped: bool,
        reference: Option<WallLayerReference>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(400., 1200.))),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                editor.show(ui, document, flipped, reference, false);
            },
        );
        output.textures_delta.clear();
        output
    }
    fn click(
        editor: &mut LayerEditor,
        ctx: &egui::Context,
        document: &Document,
        point: egui::Pos2,
    ) {
        for pressed in [true, false] {
            frame(
                editor,
                ctx,
                document,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                false,
                None,
            );
        }
    }
    #[test]
    fn edits_preserve_order_and_identity_and_removed_reference_requires_repair() {
        let (session, stack, _) = fixture();
        let mut editor =
            LayerEditor::existing(session.document(), &stack, WallLayerReference::Center).unwrap();
        editor.sync_source(&session);
        let original = session.document().clone();
        editor.duplicate(1);
        assert_ne!(editor.rows[1].id, editor.rows[2].id);
        assert_eq!(editor.rows[1].material, editor.rows[2].material);
        let id = editor.rows[2].id;
        editor.rows.swap(2, 4);
        assert_eq!(editor.rows[4].id, id);
        editor.rows[4].thickness = DimensionField::from_dimension(&length(25.), "mm").unwrap();
        assert_eq!(
            editor.resolve_diagram(session.document()).unwrap().total,
            165.
        );
        editor.reference = WallLayerReference::Boundary {
            layer: id,
            side: WallLayerSide::Inside,
        };
        editor.rows.remove(4);
        assert!(editor.resolve_diagram(session.document()).is_err());
        assert!(matches!(editor.reference,WallLayerReference::Boundary {layer,..} if layer==id));
        editor.reference = WallLayerReference::Outside;
        assert_eq!(
            editor.resolve_diagram(session.document()).unwrap().total,
            140.
        );
        assert_eq!(
            session.document(),
            &original,
            "a local construction draft never commits itself"
        );
    }
    #[test]
    fn painted_bands_and_coating_rows_select_layers_without_changing_source() {
        let (session, stack, _) = fixture();
        let mut editor =
            LayerEditor::existing(session.document(), &stack, WallLayerReference::Center).unwrap();
        editor.sync_source(&session);
        let ctx = egui::Context::default();
        let output = frame(&mut editor, &ctx, session.document(), vec![], false, None);
        let color = Color32::from_rgb(160, 80, 60);
        let bands: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::epaint::Shape::Rect(rect) = &s.shape {
                    (rect.fill == color).then_some(rect.rect)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(bands.len(), 2);
        let referenced = frame(
            &mut editor,
            &ctx,
            session.document(),
            vec![],
            false,
            Some(WallLayerReference::Inside),
        );
        let marker = referenced
            .shapes
            .iter()
            .find_map(|s| {
                if let egui::epaint::Shape::LineSegment { points, .. } = &s.shape {
                    ((points[0].y - points[1].y).abs() > 67.).then_some(points[0].x)
                } else {
                    None
                }
            })
            .expect("painted instance reference line");
        assert!(
            (marker - bands[1].right()).abs() < 0.01,
            "instance Inside reference must use the actual inside boundary"
        );

        assert!(bands[0].width() > bands[1].width() * 9.9);
        let before = editor.stack(session.document()).unwrap();
        click(&mut editor, &ctx, session.document(), bands[1].center());
        assert_eq!(editor.selected, Some(stack.layers[3].id));
        let output = frame(&mut editor, &ctx, session.document(), vec![], false, None);
        let coating = output
            .shapes
            .iter()
            .find_map(|s| {
                if let egui::epaint::Shape::Text(text) = &s.shape {
                    text.galley
                        .text()
                        .contains("Paint · Coating")
                        .then(|| Rect::from_min_size(text.pos, text.galley.size()))
                } else {
                    None
                }
            })
            .expect("actual painted coating row");
        click(&mut editor, &ctx, session.document(), coating.center());
        assert_eq!(editor.selected, Some(stack.layers[0].id));
        assert_eq!(editor.stack(session.document()).unwrap(), before);
        assert!(
            session.cached_source_hash().is_none(),
            "idle editor frames must not hash the document"
        );
        let flipped = frame(&mut editor, &ctx, session.document(), vec![], true, None);
        let mirrored: Vec<_> = flipped
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::epaint::Shape::Rect(rect) = &s.shape {
                    (rect.fill == color).then_some(rect.rect)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            mirrored[0].center().x > mirrored[1].center().x,
            "flip must mirror the same source layers"
        );
    }
    #[test]
    fn same_document_and_revision_in_another_session_refreshes_material_preview() {
        let (first, stack, material) = fixture();
        let mut editor =
            LayerEditor::existing(first.document(), &stack, WallLayerReference::Center).unwrap();
        editor.sync_source(&first);
        editor.diagram = Some(editor.resolve_diagram(first.document()));
        let mut entity = first.document().get(material).unwrap().clone();
        if let EntityData::Material { color_rgba, .. } = &mut entity.data {
            *color_rgba = [20, 80, 200, 255];
        }
        let second = Session::new(
            first
                .document()
                .edited(&[Mutation::Replace(entity)])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first.document().id(), second.document().id());
        assert_eq!(first.revision(), second.revision());
        editor.sync_source(&second);
        assert!(editor.diagram.is_none());
        assert_eq!(
            editor
                .materials
                .iter()
                .find(|m| m.id == material)
                .unwrap()
                .color,
            Some(Color32::from_rgb(20, 80, 200))
        );
        assert!(second.cached_source_hash().is_none());
    }
}
