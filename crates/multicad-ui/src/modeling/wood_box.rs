use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    materials::{self, LibraryDefinition, StockKind},
    model::*,
};

type Diagnostic = Result<ResolvedWoodBox, String>;
pub(super) struct WoodBoxPanel {
    stamp: FormStamp,
    source: WoodBoxRequest,
    size: [DimensionField; 3],
    origin: [DimensionField; 3],
    rotation: DimensionField,
    stock: Option<EntityId>,
    diagnostic: Option<(FormStamp, Diagnostic)>,
    revision: Option<u64>,
}
fn stocks(document: &Document) -> Vec<(EntityId, String)> {
    document
        .entities()
        .filter_map(|entity| {
            let LibraryDefinition::Stock(stock) = materials::read(entity).ok().flatten()? else {
                return None;
            };
            let StockKind::Panel { thickness_mm, .. } = stock.specification else {
                return None;
            };
            let material = document
                .get(stock.material)
                .map_or("Missing material", |e| e.name.as_str());
            Some((
                entity.id,
                format!("{} · {thickness_mm} mm · {material}", entity.name),
            ))
        })
        .collect()
}
impl WoodBoxPanel {
    pub fn new(document: &Document, name: String) -> Result<Self, String> {
        let stock = stocks(document).first().map(|entry| entry.0);
        let mut source = WoodBoxRequest::new(
            name,
            WoodBoxDefinition::new(
                stock.unwrap_or_default(),
                length(600.),
                length(400.),
                length(300.),
            ),
        );
        source.definition.faces.remove(&WoodBoxFace::Top);
        let mut panel = Self::from_source(source)?;
        panel.stock = stock;
        Ok(panel)
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
        Self::from_source(WoodBoxRequest {
            id,
            name: document.get(id).ok_or("Wood box is missing")?.name.clone(),
            definition: read_wood_box(document, id).map_err(|e| e.to_string())?,
        })
    }
    pub(super) fn from_source(source: WoodBoxRequest) -> Result<Self, String> {
        let field = |d| DimensionField::from_dimension(d, "mm");
        Ok(Self {
            stamp: FormStamp::new(),
            size: [
                field(&source.definition.width)?,
                field(&source.definition.depth)?,
                field(&source.definition.height)?,
            ],
            origin: [
                field(&source.definition.origin[0])?,
                field(&source.definition.origin[1])?,
                field(&source.definition.origin[2])?,
            ],
            rotation: DimensionField::from_dimension(&source.definition.rotation, "deg")?,
            stock: Some(source.definition.stock),
            source,
            diagnostic: None,
            revision: None,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Wood Box"
    }
    pub fn sync_source(&mut self, session: &Session) {
        if self.revision != Some(session.revision()) {
            self.revision = Some(session.revision());
            self.diagnostic = None;
        }
    }
    fn value(&self, document: &Document) -> Result<WoodBoxRequest, String> {
        let mut request = self.source.clone();
        request.definition.stock = self
            .stock
            .ok_or("Define panel stock in Project → Materials, then choose it here")?;
        request.definition.width = self.size[0].resolve(document)?;
        request.definition.depth = self.size[1].resolve(document)?;
        request.definition.height = self.size[2].resolve(document)?;
        request.definition.origin = [
            self.origin[0].resolve(document)?,
            self.origin[1].resolve(document)?,
            self.origin[2].resolve(document)?,
        ];
        request.definition.rotation = self.rotation.resolve(document)?;
        if request.name.trim().is_empty() || request.name.len() > 1000 {
            return Err("Box name must contain 1–1000 bytes".into());
        }
        Ok(request)
    }
    fn diagnose(&self, document: &Document) -> Diagnostic {
        let request = self.value(document)?;
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
        resolve_wood_box(document, &parameters, &request.definition).map_err(|e| e.to_string())
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<WoodBoxChange, String> {
        if !enabled {
            return Err("Native modeling is unavailable".into());
        }
        self.diagnose(document)?;
        Ok(WoodBoxChange::Set(Box::new(self.value(document)?)))
    }
    pub fn viewport(&mut self, session: &Session, view: super::TerminationViewport<'_>) -> bool {
        self.sync_source(session);
        if self.diagnostic.as_ref().is_none_or(|d| d.0 != self.stamp) {
            self.diagnostic = Some((self.stamp, self.diagnose(session.document())));
        }
        if let Ok(layout) = &self.diagnostic.as_ref().expect("diagnosed").1 {
            paint_viewport(layout, view);
        }
        false
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        ui.heading("Wood Box");
        let mut changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.source.name)
                    .desired_width(controls::inspector_width(ui)),
            )
            .changed();
        ui.label("Outside dimensions");
        for (field, label) in self.size.iter_mut().zip(["Width", "Depth", "Height"]) {
            ui.horizontal(|ui| {
                changed |= field.show(
                    ui,
                    label,
                    "Outside design envelope; formulas may reference project parameters",
                );
            });
        }
        ui.label("Panel stock / thickness / material");
        let choices = stocks(document);
        let selected = choices
            .iter()
            .find(|c| Some(c.0) == self.stock)
            .map_or("Choose panel stock", |c| c.1.as_str());
        controls::inspector_combo(ui, "wood_box_stock")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (id, label) in &choices {
                    changed |= ui
                        .selectable_value(&mut self.stock, Some(*id), label)
                        .changed();
                }
            });
        ui.small("Shared stock is defined in Project → Materials.");
        ui.horizontal(|ui| {
            if ui.button("Open top").clicked() {
                self.source.definition.faces = WoodBoxFace::ALL
                    .into_iter()
                    .filter(|face| *face != WoodBoxFace::Top)
                    .collect();
                changed = true;
            }
            if ui.button("All faces").clicked() {
                self.source.definition.faces = WoodBoxFace::ALL.into_iter().collect();
                changed = true;
            }
        });
        let columns = if ui.available_width() < 230. { 2 } else { 3 };
        egui::Grid::new("wood_box_faces")
            .num_columns(columns)
            .show(ui, |ui| {
                for (index, face) in WoodBoxFace::ALL.into_iter().enumerate() {
                    let mut present = self.source.definition.faces.contains(&face);
                    let response = ui
                        .scope(|ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            ui.checkbox(&mut present, face.label())
                        })
                        .inner;
                    if response.changed() {
                        if present {
                            self.source.definition.faces.insert(face);
                        } else {
                            self.source.definition.faces.remove(&face);
                        }
                        changed = true;
                    }
                    if index % columns == columns - 1 {
                        ui.end_row();
                    }
                }
            });
        ui.collapsing("Placement", |ui| {
            for (field, label) in self.origin.iter_mut().zip(["X", "Y", "Z"]) {
                changed |= field.show(ui, label, "World origin of the outside envelope");
            }
            changed |= self.rotation.show(ui, "Rotation", "Rotation about world Z");
        });
        if changed {
            self.stamp.changed();
            self.diagnostic = None;
        }
        if self.diagnostic.as_ref().is_none_or(|d| d.0 != self.stamp) {
            self.diagnostic = Some((self.stamp, self.diagnose(document)));
        }
        let diagnostic = &self.diagnostic.as_ref().expect("diagnosed").1;
        if let Ok(layout) = diagnostic {
            ui.small(format!("Outside: {}", dimensions(layout.outside_mm)));
            ui.small(format!("Usable inner: {}", dimensions(layout.clear_mm)));
            ui.small(format!(
                "Assembly bounds: {}",
                dimensions(std::array::from_fn(
                    |i| layout.actual_bounds[1][i] - layout.actual_bounds[0][i]
                ))
            ));
            ui.small(format!(
                "{} separate panels · {:.1} mm stock",
                layout.panels.len(),
                layout.thickness_mm
            ));
        } else if let Err(error) = diagnostic {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Ok(layout) = diagnostic {
            ui.collapsing("Panel layout", |ui| diagram(ui, layout));
        }
        ui.collapsing("Butt corner policy", |ui| {
            ui.small("Sides span full depth and height. Front/back fit between sides; top/bottom fit between vertical faces. Missing faces remove their thickness allowance.");
        });
        form::preview_status(ui, self.stamp, preview);
        let action = form::actions(
            ui,
            diagnostic.is_ok() && enabled,
            changed,
            self.stamp,
            preview,
        );
        match action {
            Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
            Some(FormAction::Preview) => {
                self.request(document, enabled)
                    .ok()
                    .map(|change| ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::WoodBox(Box::new(change)),
                    })
            }
            Some(FormAction::Apply) if preview.ready(self.stamp) => Some(ModelEvent::Apply),
            _ if changed => Some(ModelEvent::Changed),
            _ => None,
        }
    }
}
fn dimensions(size: [f64; 3]) -> String {
    format!("{:.1} × {:.1} × {:.1} mm", size[0], size[1], size[2])
}
/// This lightweight projection uses the exact resolved panel corners. The main
/// viewport's Preview still requires the matching native geometry receipt.
fn diagram(ui: &mut egui::Ui, layout: &ResolvedWoodBox) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 150.), egui::Sense::hover());
    let project = |p: [f64; 3]| [p[0] - p[1] * 0.65, -p[2] + (p[0] + p[1]) * 0.32];
    let envelope = layout.envelope_corners.map(project);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for point in envelope {
        for i in 0..2 {
            min[i] = min[i].min(point[i]);
            max[i] = max[i].max(point[i]);
        }
    }
    let scale = ((rect.width() - 20.) as f64 / (max[0] - min[0]).max(1.))
        .min((rect.height() - 20.) as f64 / (max[1] - min[1]).max(1.));
    let screen = |p| {
        let p = project(p);
        rect.center()
            + egui::vec2(
                ((p[0] - (min[0] + max[0]) * 0.5) * scale) as f32,
                ((p[1] - (min[1] + max[1]) * 0.5) * scale) as f32,
            )
    };
    let painter = ui.painter_at(rect);
    for panel in &layout.panels {
        let points = panel.corners().map(screen);
        let color = egui::Color32::from_rgb(210, 179, 120);
        for a in 0..8 {
            for bit in [1, 2, 4] {
                let b = a ^ bit;
                if a < b {
                    painter.line_segment([points[a], points[b]], egui::Stroke::new(1., color));
                }
            }
        }
    }
    for (index, face) in WoodBoxFace::ALL.into_iter().enumerate() {
        let panel = layout.panels.iter().find(|panel| panel.face == face);
        let corners: Vec<_> = if let Some(panel) = panel {
            panel.corners().to_vec()
        } else {
            let (bit, present) = match face {
                WoodBoxFace::Left => (1, false),
                WoodBoxFace::Right => (1, true),
                WoodBoxFace::Front => (2, false),
                WoodBoxFace::Back => (2, true),
                WoodBoxFace::Bottom => (4, false),
                WoodBoxFace::Top => (4, true),
            };
            layout
                .envelope_corners
                .iter()
                .enumerate()
                .filter_map(|(i, p)| ((i & bit != 0) == present).then_some(*p))
                .collect()
        };
        let center = std::array::from_fn(|axis| {
            corners.iter().map(|p| p[axis]).sum::<f64>() / corners.len() as f64
        });
        let left = index % 2 == 0;
        let label = egui::pos2(
            if left {
                rect.left() + 4.
            } else {
                rect.right() - 4.
            },
            rect.top() + 35. + (index / 2) as f32 * 40.,
        );
        let color = if panel.is_some() {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let text = format!(
            "{}{}",
            face.label(),
            if panel.is_none() { " (off)" } else { "" }
        );
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.), color);
        let label_rect = egui::Rect::from_min_size(
            label - egui::vec2(if left { 0. } else { galley.size().x }, 7.),
            galley.size(),
        );
        painter.line_segment(
            [screen(center), label_rect.center()],
            egui::Stroke::new(0.5, color.gamma_multiply(0.4)),
        );
        painter.rect_filled(label_rect.expand(2.), 2., ui.visuals().panel_fill);
        painter.galley(label_rect.min, galley, color);
    }
}

fn paint_viewport(layout: &ResolvedWoodBox, view: super::TerminationViewport<'_>) {
    let painter = view.painter.with_clip_rect(view.rect);
    let screen = |point: [f64; 3]| {
        let p = view.camera.project(point.map(|v| v as f32), view.rect);
        egui::pos2(p[0], p[1])
    };
    let color = egui::Color32::from_rgb(242, 198, 113);
    for panel in &layout.panels {
        let corners = panel.corners().map(screen);
        for (a, point) in corners.iter().enumerate() {
            for bit in [1, 2, 4] {
                let b = a ^ bit;
                if a < b {
                    painter.line_segment(
                        [*point, corners[b]],
                        egui::Stroke::new(0.7, color.gamma_multiply(0.65)),
                    );
                }
            }
        }
    }
    if view.rect.width() < 220. || view.rect.height() < 150. {
        return;
    }
    let bounds = egui::Rect::from_points(&layout.envelope_corners.map(screen));
    for (index, face) in WoodBoxFace::ALL.into_iter().enumerate() {
        let panel = layout.panels.iter().find(|panel| panel.face == face);
        let (bit, present) = match face {
            WoodBoxFace::Left => (1, false),
            WoodBoxFace::Right => (1, true),
            WoodBoxFace::Front => (2, false),
            WoodBoxFace::Back => (2, true),
            WoodBoxFace::Bottom => (4, false),
            WoodBoxFace::Top => (4, true),
        };
        let corners: Vec<_> = layout
            .envelope_corners
            .iter()
            .enumerate()
            .filter_map(|(i, p)| ((i & bit != 0) == present).then_some(*p))
            .collect();
        let center = std::array::from_fn(|axis| {
            corners.iter().map(|p| p[axis]).sum::<f64>() / corners.len() as f64
        });
        let left = index % 2 == 0;
        let x = if left {
            (bounds.left() - 20.).max(view.rect.left() + 100.)
        } else {
            (bounds.right() + 20.).min(view.rect.right() - 100.)
        };
        let y = (bounds.top() + (index / 2) as f32 * bounds.height().max(100.) * 0.5)
            .clamp(view.rect.top() + 50., view.rect.bottom() - 70.);
        let label = format!(
            "{}{}",
            face.label(),
            if panel.is_none() { " (off)" } else { "" }
        );
        let galley = painter.layout_no_wrap(label, egui::FontId::proportional(13.), color);
        let rect = egui::Rect::from_min_size(
            egui::pos2(x - if left { galley.size().x } else { 0. }, y),
            galley.size(),
        );
        painter.line_segment(
            [screen(center), rect.center()],
            egui::Stroke::new(0.7, color),
        );
        painter.rect_filled(rect.expand(4.), 3., egui::Color32::from_rgb(25, 30, 37));
        painter.galley(rect.min, galley, color);
    }
    for (index, label) in [
        format!("Outside: {}", dimensions(layout.outside_mm)),
        format!("Usable inner: {}", dimensions(layout.clear_mm)),
    ]
    .into_iter()
    .enumerate()
    {
        let text = painter.layout_no_wrap(label, egui::FontId::proportional(12.), color);
        let rect = egui::Rect::from_min_size(
            view.rect.left_bottom() + egui::vec2(14., -48. + index as f32 * 20.),
            text.size(),
        );
        painter.rect_filled(rect.expand(3.), 2., egui::Color32::from_rgb(25, 30, 37));
        painter.galley(rect.min, text, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        context: &egui::Context,
        panel: &mut WoodBoxPanel,
        document: &Document,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 700.),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                egui::Panel::right("wood-box-painted-test")
                    .default_size(300.)
                    .resizable(false)
                    .show(ui, |ui| {
                        panel.show(ui, document, true, PreviewState::Idle);
                    });
            },
        );
        output.textures_delta.clear();
        output
    }
    #[test]
    fn wood_box_painted_face_checkbox_changes_live_dimensions_without_changing_source() {
        let (session, _) = super::super::panel_fixture::flat_panel();
        let before = session.document().clone();
        let mut panel = WoodBoxPanel::new(&before, "Drawer".into()).unwrap();
        let context = egui::Context::default();
        crate::theme::configure(&context);
        context.enable_accesskit();
        frame(&context, &mut panel, &before, vec![]);
        let output = frame(&context, &mut panel, &before, vec![]);
        let update = output.platform_output.accesskit_update.as_ref().unwrap();
        let bounds = update
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some("Top"))
                    .then(|| node.bounds())
                    .flatten()
            })
            .expect("Painted Top checkbox");
        let pos = egui::pos2(
            (bounds.x0 + bounds.x1) as f32 / 2.,
            (bounds.y0 + bounds.y1) as f32 / 2.,
        );
        let old = panel.stamp();
        for pressed in [true, false] {
            frame(
                &context,
                &mut panel,
                &before,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_ne!(panel.stamp(), old);
        let WoodBoxChange::Set(request) = panel.request(&before, true).unwrap() else {
            panic!("Set")
        };
        assert!(request.definition.faces.contains(&WoodBoxFace::Top));
        assert_eq!(
            panel.diagnose(&before).unwrap().clear_mm,
            [564., 364., 264.]
        );
        assert_eq!(session.document(), &before);
    }
}
