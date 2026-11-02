use super::{
    FormStamp, PreviewState, TerminationViewport,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, *},
    parameters::EvaluatedParameters,
};

struct BoxFields {
    id: EntityId,
    center: [DimensionField; 2],
    width: DimensionField,
    depth: DimensionField,
    rotation: f64,
    vertical: bool,
    bottom: DimensionField,
    top: DimensionField,
}
fn mm(value: f64) -> DimensionField {
    DimensionField::from_dimension(&length(value), "mm").expect("literal mm")
}
impl BoxFields {
    fn source(value: &CompoundCutoutBox) -> Result<Self, String> {
        let field = |value| DimensionField::from_dimension(value, "mm");
        Ok(Self {
            id: value.id,
            center: [
                field(&value.rectangle.center[0])?,
                field(&value.rectangle.center[1])?,
            ],
            width: field(&value.rectangle.width)?,
            depth: field(&value.rectangle.depth)?,
            rotation: value.rectangle.rotation_deg,
            vertical: value.vertical.is_some(),
            bottom: field(
                &value
                    .vertical
                    .as_ref()
                    .map_or(length(0.), |z| z.bottom.clone()),
            )?,
            top: field(
                &value
                    .vertical
                    .as_ref()
                    .map_or(length(3000.), |z| z.top.clone()),
            )?,
        })
    }
    fn value(&self, document: &Document) -> Result<CompoundCutoutBox, String> {
        Ok(CompoundCutoutBox {
            id: self.id,
            rectangle: PlanRectangle {
                center: [
                    self.center[0].resolve(document)?,
                    self.center[1].resolve(document)?,
                ],
                width: self.width.resolve(document)?,
                depth: self.depth.resolve(document)?,
                rotation_deg: self.rotation,
            },
            vertical: self
                .vertical
                .then(|| {
                    Ok::<_, String>(CutoutZRange {
                        bottom: self.bottom.resolve(document)?,
                        top: self.top.resolve(document)?,
                    })
                })
                .transpose()?,
        })
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Placement {
    Idle,
    Add,
    Move,
    Resize,
}
type Diagnostic = Result<Option<ResolvedCompoundCutout>, String>;
pub(super) struct CompoundCutoutPanel {
    stamp: FormStamp,
    id: EntityId,
    name: String,
    boxes: Vec<BoxFields>,
    selected: usize,
    receivers: Vec<CompoundCutoutReceiver>,
    choices: Vec<(String, CompoundCutoutReceiver, CompoundReceiverKind)>,
    fixed: bool,
    bottom: DimensionField,
    top: DimensionField,
    levels: Option<CutoutLevelRange>,
    placement: Placement,
    anchor: Option<[f64; 2]>,
    remove: bool,
    error: Option<String>,
    diagnostic: Option<(FormStamp, Diagnostic)>,
    source_revision: Option<u64>,
}
impl CompoundCutoutPanel {
    pub fn new(document: &Document) -> Result<Self, String> {
        Self::from_source(
            document,
            EntityId::new(),
            "Compound cutout".into(),
            CompoundCutoutDefinition {
                boxes: vec![CompoundCutoutBox::new(PlanRectangle {
                    center: [length(0.), length(0.)],
                    width: length(500.),
                    depth: length(500.),
                    rotation_deg: 0.,
                })],
                receivers: vec![],
                extent: CompoundCutoutExtent::ThroughReceivers,
                levels: None,
            },
            true,
        )
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
        let source = model::read_compound_cutout(document, id).map_err(|e| e.to_string())?;
        Self::from_source(
            document,
            id,
            document.get(id).ok_or("Cutout is missing")?.name.clone(),
            source,
            false,
        )
    }
    pub(super) fn from_source(
        document: &Document,
        id: EntityId,
        name: String,
        source: CompoundCutoutDefinition,
        placing: bool,
    ) -> Result<Self, String> {
        let mut choices = vec![];
        for entity in document.entities() {
            let EntityData::Part { features, .. } = &entity.data else {
                continue;
            };
            let Some(producer) = features
                .iter()
                .rev()
                .find(|id| {
                    matches!(
                        document.get(**id).map(|entity| &entity.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
                .copied()
            else {
                continue;
            };
            let receiver = source
                .receivers
                .iter()
                .find(|receiver| receiver.target.body == entity.id)
                .cloned()
                .unwrap_or_else(|| {
                    CompoundCutoutReceiver::new(BodyInput {
                        body: entity.id,
                        producer,
                    })
                });
            if let Ok(base) = model::receiver_base(document, &receiver.target) {
                choices.push((entity.name.clone(), receiver, base.kind));
            }
        }
        let (fixed, bottom, top) = match source.extent {
            CompoundCutoutExtent::ThroughReceivers => (false, mm(0.), mm(3000.)),
            CompoundCutoutExtent::Fixed(z) => (
                true,
                DimensionField::from_dimension(&z.bottom, "mm")?,
                DimensionField::from_dimension(&z.top, "mm")?,
            ),
        };
        Ok(Self {
            stamp: FormStamp::new(),
            id,
            name,
            boxes: source
                .boxes
                .iter()
                .map(BoxFields::source)
                .collect::<Result<_, _>>()?,
            selected: 0,
            receivers: source.receivers,
            choices,
            fixed,
            bottom,
            top,
            levels: source.levels,
            placement: if placing {
                Placement::Add
            } else {
                Placement::Idle
            },
            anchor: None,
            remove: false,
            error: None,
            diagnostic: None,
            source_revision: None,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn sync_source(&mut self, session: &multicad_core::commands::Session) {
        if self.source_revision != Some(session.revision()) {
            self.source_revision = Some(session.revision());
            self.diagnostic = None;
        }
    }
    pub fn label(&self) -> &'static str {
        "Cutout · Multiple boxes"
    }
    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<CompoundCutoutChange, String> {
        if !enabled {
            return Err("Native modeling is unavailable".into());
        }
        self.diagnose(document).map(|(change, _)| change)
    }
    fn diagnose(
        &self,
        document: &Document,
    ) -> Result<(CompoundCutoutChange, Option<ResolvedCompoundCutout>), String> {
        if self.remove {
            return Ok((CompoundCutoutChange::Remove(self.id), None));
        }
        if self.placement != Placement::Idle {
            return Err("Finish placing the box in the viewport".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Cutout name must contain 1–1000 bytes".into());
        }
        let definition = self.definition(document)?;
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
        let resolved = model::resolve_compound_cutout(document, &parameters, &definition)
            .map_err(|e| e.to_string())?;
        Ok((
            CompoundCutoutChange::Set(Box::new(CompoundCutoutRequest {
                feature: self.id,
                name: self.name.clone(),
                definition,
            })),
            Some(resolved),
        ))
    }
    fn definition(&self, document: &Document) -> Result<CompoundCutoutDefinition, String> {
        Ok(CompoundCutoutDefinition {
            boxes: self
                .boxes
                .iter()
                .map(|item| item.value(document))
                .collect::<Result<_, _>>()?,
            receivers: self.receivers.clone(),
            extent: if self.fixed {
                CompoundCutoutExtent::Fixed(CutoutZRange {
                    bottom: self.bottom.resolve(document)?,
                    top: self.top.resolve(document)?,
                })
            } else {
                CompoundCutoutExtent::ThroughReceivers
            },
            levels: self.levels.clone(),
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
        ui.strong(self.label());
        ui.small("One cutout from overlapping boxes.");
        let mut changed = ui.text_edit_singleline(&mut self.name).changed();
        ui.separator();
        let receiving =
            egui::CollapsingHeader::new(format!("Receiving parts ({})", self.receivers.len()))
                .id_salt("compound_receiver_options")
                .default_open(document.get(self.id).is_none() || self.receivers.is_empty())
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("compound_receivers")
                        .max_height(100.)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for kind in [CompoundReceiverKind::Wall, CompoundReceiverKind::Slab] {
                                for (name, receiver, receiver_kind) in &self.choices {
                                    if *receiver_kind != kind {
                                        continue;
                                    }
                                    let mut selected = self
                                        .receivers
                                        .iter()
                                        .any(|old| old.target.body == receiver.target.body);
                                    if ui
                                        .push_id(receiver.target.body, |ui| {
                                            ui.checkbox(&mut selected, name)
                                        })
                                        .inner
                                        .changed()
                                    {
                                        self.receivers
                                            .retain(|old| old.target.body != receiver.target.body);
                                        if selected {
                                            self.receivers.push(receiver.clone());
                                        }
                                        changed = true;
                                    }
                                }
                            }
                        });
                    if self.choices.is_empty() {
                        ui.label("Create a receiving wall or slab first.");
                    }
                    let levels: Vec<_> = model::levels(document)
                        .map(|entity| (entity.id, entity.name.clone()))
                        .collect();
                    let mut ranged = self.levels.is_some();
                    if ui.checkbox(&mut ranged, "Floor level range").changed() {
                        self.levels = if ranged {
                            levels.first().map(|(id, _)| CutoutLevelRange {
                                from: *id,
                                to: None,
                            })
                        } else {
                            None
                        };
                        changed = true;
                    }
                    if let Some(range) = &mut self.levels {
                        let mut from = range.from;
                        let mut to = range.to.unwrap_or(from);
                        let mut range_changed = false;
                        for (label, value) in [("From level", &mut from), ("To level", &mut to)] {
                            let old = *value;
                            ui.label(label);
                            egui::ComboBox::from_id_salt(label)
                                .selected_text(
                                    levels
                                        .iter()
                                        .find(|(id, _)| *id == *value)
                                        .map_or("Missing level", |(_, name)| name.as_str()),
                                )
                                .show_ui(ui, |ui| {
                                    for (id, name) in &levels {
                                        ui.selectable_value(value, *id, name);
                                    }
                                });
                            range_changed |= old != *value;
                        }
                        if range_changed {
                            range.from = from;
                            range.to = Some(to);
                            changed = true;
                        }
                        ui.small("Receivers remain explicit. Uncheck slabs outside this range.");
                    }
                });
        if receiving.body_response.is_none() {
            let names = self
                .receivers
                .iter()
                .map(|receiver| {
                    document
                        .get(receiver.target.body)
                        .map_or("Missing part", |entity| entity.name.as_str())
                })
                .collect::<Vec<_>>();
            ui.small(if names.is_empty() {
                "No receivers selected".into()
            } else {
                names.join(" · ")
            });
            if let Some(range) = &self.levels {
                let name = |id| {
                    document
                        .get(id)
                        .map_or("Missing level", |entity| entity.name.as_str())
                };
                ui.small(format!(
                    "Levels: {} … {}",
                    name(range.from),
                    name(range.to.unwrap_or(range.from))
                ));
            }
        }
        ui.separator();
        ui.strong("Vertical extent");
        changed |= ui
            .checkbox(&mut self.fixed, "Use fixed bottom and top")
            .changed();
        if self.fixed {
            changed |= self.bottom.show(ui, "Bottom", "World Z");
            changed |= self.top.show(ui, "Top", "World Z");
        } else {
            ui.small("Through the complete height of the selected receivers.");
        }
        if self
            .diagnostic
            .as_ref()
            .is_none_or(|(stamp, _)| *stamp != self.stamp)
            || changed
        {
            self.diagnostic = Some((
                self.stamp,
                self.diagnose(document).map(|(_, resolved)| resolved),
            ));
        }
        if let Some((stamp, Ok(Some(resolved)))) = &self.diagnostic
            && *stamp == self.stamp
        {
            ui.small(format!(
                "Shared Z: {:.1} … {:.1} mm",
                resolved.shared_z_mm[0], resolved.shared_z_mm[1]
            ));
        } else {
            ui.small("Shared Z: waiting for valid inputs");
        }
        ui.separator();
        ui.strong("Boxes");
        ui.horizontal_wrapped(|ui| {
            for (index, item) in self.boxes.iter().enumerate() {
                if ui
                    .add_enabled_ui(self.placement != Placement::Add, |ui| {
                        ui.selectable_label(index == self.selected, format!("Box {}", index + 1))
                    })
                    .inner
                    .on_hover_text(item.id.to_string())
                    .clicked()
                {
                    self.selected = index;
                    self.placement = Placement::Idle;
                    self.anchor = None;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.boxes.len() < MAX_COMPOUND_CUTOUT_BOXES
                        && self.placement == Placement::Idle,
                    egui::Button::new("Add box"),
                )
                .clicked()
            {
                let value = CompoundCutoutBox::new(PlanRectangle {
                    center: [length(0.), length(0.)],
                    width: length(500.),
                    depth: length(500.),
                    rotation_deg: 0.,
                });
                self.boxes
                    .push(BoxFields::source(&value).expect("literal box"));
                self.selected = self.boxes.len() - 1;
                self.placement = Placement::Add;
                self.anchor = None;
                changed = true;
            }
            if ui.button("Move box").clicked() {
                self.placement = Placement::Move;
                self.anchor = None;
                changed = true;
            }
            if ui.button("Resize box").clicked() {
                self.placement = Placement::Resize;
                self.anchor = None;
                changed = true;
            }
            if ui
                .add_enabled(self.boxes.len() > 1, egui::Button::new("Remove box"))
                .clicked()
            {
                self.boxes.remove(self.selected);
                self.selected = 0;
                self.placement = Placement::Idle;
                self.anchor = None;
                changed = true;
            }
        });
        ui.small(match self.placement {
            Placement::Add => "Click two opposite box corners in the viewport.",
            Placement::Move => "Click the new box center in the viewport.",
            Placement::Resize => {
                "Click the new opposite corner; the lower-left corner stays fixed."
            }
            Placement::Idle => {
                "Select a box to edit it. Gold wireframes show the complete cutting volume."
            }
        });
        if self.placement != Placement::Idle && ui.button("Finish graphical placement").clicked() {
            self.placement = Placement::Idle;
            self.anchor = None;
            changed = true;
        }
        ui.collapsing("Box dimensions", |ui| {
            if let Some(item) = self.boxes.get_mut(self.selected) {
                changed |= item.center[0].show(ui, "Center X", "World X");
                changed |= item.center[1].show(ui, "Center Y", "World Y");
                changed |= item.width.show(ui, "Width", "Local box X");
                changed |= item.depth.show(ui, "Depth", "Local box Y");
                ui.horizontal(|ui| {
                    ui.label("Rotation (°)");
                    changed |= ui.add(egui::DragValue::new(&mut item.rotation)).changed();
                });
                changed |= ui
                    .checkbox(&mut item.vertical, "Override this box's vertical extent")
                    .changed();
                if item.vertical {
                    changed |= item.bottom.show(ui, "Box bottom", "World Z");
                    changed |= item.top.show(ui, "Box top", "World Z");
                }
            }
        });
        let diagnostic = &self.diagnostic.as_ref().expect("diagnosed").1;
        let valid = diagnostic.is_ok() && enabled;
        if let Err(error) = diagnostic {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if document.get(self.id).is_some() {
            changed |= ui
                .checkbox(
                    &mut self.remove,
                    "Remove complete cutout from all receiving parts",
                )
                .changed();
        }
        form::preview_status(ui, self.stamp, preview);
        let action = form::actions(ui, valid, changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => {
                self.request(document, enabled)
                    .ok()
                    .map(|request| ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::CompoundCutout(Box::new(request)),
                    })
            }
            Some(FormAction::Apply) if preview.ready(self.stamp) => Some(ModelEvent::Apply),
            _ => None,
        }
    }
    pub fn viewport(
        &mut self,
        document: &Document,
        parameters: &EvaluatedParameters,
        view: TerminationViewport<'_>,
    ) -> bool {
        let Ok(definition) = self.definition(document) else {
            return false;
        };
        let resolved = model::compound_cutout_layout(document, parameters, &definition).ok();
        let z = resolved.as_ref().map_or(0., |value| value.shared_z_mm[0]);
        let point = view
            .pointer
            .filter(|p| view.rect.contains(*p))
            .and_then(|point| {
                view.camera.on_plane(
                    point,
                    view.rect,
                    multicad_core::sketch::Frame {
                        origin: [0., 0., z],
                        u: [1., 0., 0.],
                        v: [0., 1., 0.],
                        n: [0., 0., 1.],
                    },
                )
            });
        let mut changed = false;
        if view.clicked
            && self.placement != Placement::Idle
            && let Some(point) = point
        {
            let item = &mut self.boxes[self.selected];
            if self.placement == Placement::Move {
                item.center[0].set_literal(point[0], "mm");
                item.center[1].set_literal(point[1], "mm");
                self.placement = Placement::Idle;
                changed = true;
            } else {
                let start = if self.placement == Placement::Resize {
                    resolved
                        .as_ref()
                        .and_then(|value| value.boxes.get(self.selected))
                        .map(|item| item.rectangle.corners()[0])
                } else {
                    self.anchor
                };
                if let Some(start) = start {
                    let angle = if self.placement == Placement::Resize {
                        item.rotation
                    } else {
                        0.
                    };
                    let (s, c) = angle.to_radians().sin_cos();
                    let delta = [point[0] - start[0], point[1] - start[1]];
                    let width = (delta[0] * c + delta[1] * s).abs();
                    let depth = (-delta[0] * s + delta[1] * c).abs();
                    if width > 1e-3 && depth > 1e-3 {
                        item.center[0].set_literal((start[0] + point[0]) / 2., "mm");
                        item.center[1].set_literal((start[1] + point[1]) / 2., "mm");
                        item.width.set_literal(width, "mm");
                        item.depth.set_literal(depth, "mm");
                        item.rotation = angle;
                        self.placement = Placement::Idle;
                        self.anchor = None;
                        changed = true;
                    }
                } else {
                    self.anchor = Some(point);
                    changed = true;
                }
            }
        }
        if changed {
            self.stamp.changed();
        }
        let resolved = self
            .definition(document)
            .ok()
            .and_then(|source| model::compound_cutout_layout(document, parameters, &source).ok());
        if let Some(resolved) = resolved {
            let mut label_bounds: Vec<egui::Rect> = vec![];
            for (index, item) in resolved.boxes.iter().enumerate() {
                let points = item.corners().map(|point| {
                    let p = view.camera.project(point.map(|v| v as f32), view.rect);
                    egui::pos2(p[0], p[1])
                });
                let color = if index == self.selected {
                    egui::Color32::from_rgb(240, 190, 80)
                } else {
                    egui::Color32::from_rgb(150, 165, 180)
                };
                for i in 0..4 {
                    for (a, b) in [(i, (i + 1) % 4), (i + 4, (i + 1) % 4 + 4), (i, i + 4)] {
                        view.painter
                            .line_segment([points[a], points[b]], egui::Stroke::new(1.5, color));
                    }
                }
                let center = view.camera.project(
                    [
                        item.rectangle.center[0] as f32,
                        item.rectangle.center[1] as f32,
                        item.top_mm as f32,
                    ],
                    view.rect,
                );
                let galley = view.painter.layout_no_wrap(
                    format!("Box {}", index + 1),
                    egui::FontId::proportional(12.),
                    color,
                );
                let mut bounds = egui::Rect::from_min_size(
                    egui::pos2(center[0] + 4., center[1] - galley.size().y - 4.),
                    galley.size(),
                );
                for _ in 0..=label_bounds.len() {
                    if !label_bounds
                        .iter()
                        .any(|other| other.expand(2.).intersects(bounds))
                    {
                        break;
                    }
                    bounds = bounds.translate(egui::vec2(0., -galley.size().y - 4.));
                }
                view.painter.rect_filled(
                    bounds.expand(2.),
                    2.,
                    egui::Color32::from_black_alpha(190),
                );
                view.painter.galley(bounds.min, galley, color);
                label_bounds.push(bounds);
            }
        }
        if let Some(start) = self.anchor {
            let p = view
                .camera
                .project([start[0] as f32, start[1] as f32, z as f32], view.rect);
            view.painter.circle_stroke(
                egui::pos2(p[0], p[1]),
                5.,
                egui::Stroke::new(2., egui::Color32::GOLD),
            );
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::commands::{Actor, Channel};
    fn frame(
        panel: &mut CompoundCutoutPanel,
        ctx: &egui::Context,
        document: &Document,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(420., 2600.),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                panel.show(ui, document, true, PreviewState::Idle);
            },
        );
        output.textures_delta.clear();
        output
    }
    fn click(
        panel: &mut CompoundCutoutPanel,
        ctx: &egui::Context,
        document: &Document,
        label: &str,
    ) {
        let output = frame(panel, ctx, document, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    (text.galley.job.text == label)
                        .then(|| egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Missing painted button {label}"));
        for pressed in [true, false] {
            frame(
                panel,
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
            );
        }
    }
    #[test]
    fn painted_box_controls_and_native_viewport_clicks_preserve_one_compound_source() {
        let (session, _, _, _) = crate::architecture_view::tests::building();
        let before = session.document().clone();
        let mut panel = CompoundCutoutPanel::new(session.document()).unwrap();
        panel.receivers = panel
            .choices
            .iter()
            .map(|(_, receiver, _)| receiver.clone())
            .collect();
        let ctx = egui::Context::default();
        let point = |panel: &mut CompoundCutoutPanel, p| {
            super::super::panel_fixture::click(&session, p, |view| {
                panel.viewport(session.document(), session.parameters(), view)
            })
        };
        assert!(point(&mut panel, [200., 100., -200.]));
        assert!(point(&mut panel, [500., 300., -200.]));
        click(&mut panel, &ctx, session.document(), "Add box");
        assert_eq!(panel.boxes.len(), 2);
        assert!(point(&mut panel, [200., 200., -200.]));
        assert!(point(&mut panel, [350., 500., -200.]));
        let change = panel.request(session.document(), true).unwrap();
        let CompoundCutoutChange::Set(request) = &change else {
            panic!()
        };
        assert_eq!(request.definition.boxes.len(), 2);
        assert_eq!(request.definition.receivers.len(), 2);
        let first = request.definition.boxes[0].id;
        let second = request.definition.boxes[1].id;
        let prepared = model::prepare_compound_cutout(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &change,
        )
        .unwrap();
        let native = multicad_model::Compiler::default()
            .compile(prepared.prepared.document(), session.revision() + 1)
            .unwrap();
        assert_eq!(native.snapshot().bodies.len(), 2);
        click(&mut panel, &ctx, session.document(), "Move box");
        assert!(point(&mut panel, [300., 350., -200.]));
        click(&mut panel, &ctx, session.document(), "Resize box");
        assert!(point(&mut panel, [400., 500., -200.]));
        let source = panel.definition(session.document()).unwrap();
        assert_eq!(source.boxes[0].id, first);
        assert_eq!(source.boxes[1].id, second);
        click(&mut panel, &ctx, session.document(), "Remove box");
        assert_eq!(panel.boxes.len(), 1);
        assert_eq!(panel.boxes[0].id, first);
        assert_eq!(
            session.document(),
            &before,
            "Graphical editing and native Preview must not commit"
        );
    }
}
