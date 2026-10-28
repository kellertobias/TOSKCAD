//! Global read-only inspection; only explicit Pin/Update/Remove actions author
//! metadata through the canonical shared transaction API.
mod feed;
mod paint;
use crate::{
    camera::Camera,
    egui,
    viewport::{
        Hit, PickFilter, PickOptions, ReferenceSelection, Scene, SceneStamp, SectionPlane,
        Selection,
    },
};
pub use feed::{MeasurementBackend, MeasurementBackendFactory};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    inspection::*,
    references::{self, Resolution, TopologyKind},
};
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    A,
    B,
}
pub struct MeasurementInspector {
    feed: feed::Feed,
    active: bool,
    picking: Option<Side>,
    filter: PickFilter,
    refs: [Option<GeometryReference>; 2],
    query: Option<MeasurementQuery>,
    selected_pin: Option<EntityId>,
    pin_baseline: Option<PinnedMeasurement>,
    name: String,
    offset: [f64; 3],
    visible: bool,
    error: Option<String>,
    notice: Option<(String, bool)>,
    selection_cache: Option<(Arc<Scene>, [Option<ReferenceSelection>; 2])>,
    pick_cache: Option<(Arc<Scene>, PickKey, Option<Hit>)>,
}
pub struct MeasurementViewport<'a> {
    pub scene: Option<&'a Arc<Scene>>,
    pub camera: Camera,
    pub rect: egui::Rect,
    pub pointer: Option<egui::Pos2>,
    pub clicked: bool,
    pub section: Option<SectionPlane>,
}
#[derive(Clone, Copy, PartialEq)]
struct PickKey {
    stamp: SceneStamp,
    camera: Camera,
    rect: egui::Rect,
    point: egui::Pos2,
    filter: PickFilter,
    section: Option<SectionPlane>,
}
impl Default for MeasurementInspector {
    fn default() -> Self {
        Self {
            feed: Default::default(),
            active: false,
            picking: None,
            filter: PickFilter::Faces,
            refs: [None, None],
            query: None,
            selected_pin: None,
            pin_baseline: None,
            name: "Measurement".into(),
            offset: [0.; 3],
            visible: true,
            error: None,
            notice: None,
            selection_cache: None,
            pick_cache: None,
        }
    }
}
impl MeasurementInspector {
    pub fn set_factory(&mut self, f: MeasurementBackendFactory) {
        self.feed.set_factory(f);
    }
    pub fn has_factory(&self) -> bool {
        self.feed.has_factory()
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn pending(&self) -> bool {
        self.feed.pending()
    }
    pub fn open(&mut self) {
        self.active = true;
        if self.refs[0].is_none() {
            self.picking = Some(Side::A);
        } else if self.refs[1].is_none() {
            self.picking = Some(Side::B);
        }
    }
    pub fn suspend(&mut self) {
        self.feed.suspend();
    }
    pub fn close(&mut self) {
        self.active = false;
        self.picking = None;
    }
    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take()
    }
    pub fn tick(&mut self, s: &Session) {
        if let Some(id) = self.selected_pin
            && s.document().get(id).is_none()
        {
            self.selected_pin = None;
            self.pin_baseline = None;
        }
        self.feed
            .tick(s, self.query.as_ref().filter(|_| self.active), self.active);
    }
    fn update_query(&mut self) {
        self.selection_cache = None;
        self.query = self.refs[0]
            .as_ref()
            .zip(self.refs[1].as_ref())
            .map(|(a, b)| MeasurementQuery {
                a: a.clone(),
                b: b.clone(),
            });
        self.error = None;
    }
    fn reset(&mut self) {
        self.selection_cache = None;
        self.pick_cache = None;
        self.refs = [None, None];
        self.query = None;
        self.selected_pin = None;
        self.pin_baseline = None;
        self.name = "Measurement".into();
        self.offset = [0.; 3];
        self.visible = true;
        self.picking = Some(Side::A);
        self.error = None;
    }
    fn ready(&self, s: &Session) -> Option<&MeasurementValues> {
        if !self.active || self.picking.is_some() {
            return None;
        }
        self.feed
            .current(s, self.query.as_ref()?)?
            .result
            .as_ref()
            .ok()
    }
    pub fn pin(&mut self, s: &mut Session, allowed: bool, channel: Channel) -> Result<(), String> {
        if !allowed {
            return Err("Finish pending source edits before pinning a measurement".into());
        }
        if let (Some(id), Some(baseline)) = (self.selected_pin, &self.pin_baseline)
            && multicad_core::inspection::read(s.document(), id)
                .as_ref()
                .ok()
                != Some(baseline)
        {
            return Err(
                "Pinned definition changed; select it again to reload before updating".into(),
            );
        }
        self.ready(s)
            .ok_or("Wait for a current resolved native measurement before pinning")?;
        self.feed
            .stamp
            .ok_or("Wait for the current native measurement")?
            .check(s.document(), s.revision())?;
        let r = PinRequest {
            id: self.selected_pin.unwrap_or_default(),
            name: if self.name.trim().is_empty() {
                "Measurement".into()
            } else {
                self.name.clone()
            },
            query: self.query.clone().ok_or("Choose A and B")?,
            label_offset_mm: self.offset,
            visible: self.visible,
        };
        let p =
            prepare_pin(s, s.revision(), Actor::local(channel), &r).map_err(|e| e.to_string())?;
        s.commit(p).map_err(|e| e.to_string())?;
        self.selected_pin = Some(r.id);
        self.pin_baseline = multicad_core::inspection::read(s.document(), r.id).ok();
        self.notice = Some(("Pinned measurement updated · one Undo step".into(), false));
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, s: &mut Session, allow_pin: bool) {
        ui.strong("Measure");
        ui.small("Finite faces, exact edges and points · current model");
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("New measurement")
                .on_hover_text("Start a temporary A/B measurement without changing the model")
                .clicked()
            {
                self.reset();
            }
            if ui
                .button("Close Measure")
                .on_hover_text("Return to the ordinary Inspector; visible pins remain")
                .clicked()
            {
                self.close();
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (filter, label) in [
                (PickFilter::Faces, "Faces"),
                (PickFilter::Edges, "Edges"),
                (PickFilter::Vertices, "Points"),
            ] {
                ui.selectable_value(&mut self.filter, filter, label)
                    .on_hover_text("Choose the exact type to select in the model");
            }
        });
        for (index, side, label) in [(0, Side::A, "A"), (1, Side::B, "B")] {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    if index == 0 {
                        egui::Color32::from_rgb(244, 180, 65)
                    } else {
                        egui::Color32::from_rgb(85, 173, 241)
                    },
                    label,
                );
                if ui
                    .button(if self.picking == Some(side) {
                        format!("Picking {label}…")
                    } else {
                        format!("Replace {label}")
                    })
                    .on_hover_text("Select a replacement reference; the other side stays unchanged")
                    .clicked()
                {
                    self.picking = Some(side);
                }
            });
            let text = self.refs[index]
                .as_ref()
                .map(|r| {
                    let kind = match r.intent.kind {
                        TopologyKind::Face => "Face",
                        TopologyKind::Edge => "Edge",
                        TopologyKind::Vertex => "Point",
                    };
                    let key = self
                        .selection_cache
                        .as_ref()
                        .and_then(|(_, refs)| refs[index])
                        .map(|r| format!(" {}", r.key.ordinal));
                    format!(
                        "{} · {kind}{}",
                        s.document()
                            .get(r.source)
                            .map_or("Missing source", |e| e.name.as_str()),
                        key.unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| "Not selected".into());
            ui.label(text).on_hover_text("This is a persistent reference to the source geometry, including its selected subshape");
        }
        if self.picking.is_some() {
            ui.label("Click geometry in the current model");
            if ui
                .button("Cancel picking")
                .on_hover_text("Keep the previous references")
                .clicked()
            {
                self.picking = None;
            }
        }
        if self.feed.pending() {
            ui.label("Measuring current B-rep…");
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        if let Some(error) = self.error.as_ref().or(self.feed.error.as_ref()) {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(q) = &self.query
            && let Some(e) = self.feed.current(s, q)
        {
            match &e.result {
                Ok(v) => values(ui, v),
                Err(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, &e.message);
                }
            }
        }
        ui.separator();
        ui.label("Pinned name");
        ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()))
            .on_hover_text("Only Pin/Update writes this local label to the document");
        ui.checkbox(&mut self.visible, "Show retained measurement")
            .on_hover_text("Store whether this pin appears in the model");
        ui.collapsing("Label offset · world mm", |ui| {
            for (value, label) in self.offset.iter_mut().zip(["X", "Y", "Z"]) {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(egui::DragValue::new(value).speed(1.).suffix(" mm"))
                        .on_hover_text("Move the label without moving its measurement references");
                });
            }
        });
        let ready = allow_pin && self.ready(s).is_some();
        if ui
            .add_enabled(
                ready,
                egui::Button::new(if self.selected_pin.is_some() {
                    "Update pinned measurement"
                } else {
                    "Pin measurement"
                }),
            )
            .on_hover_text(
                "Retain only the exact references and label settings; measured values regenerate",
            )
            .on_disabled_hover_text(
                "Choose resolved A/B references and finish other pending edits first",
            )
            .clicked()
            && let Err(e) = self.pin(s, allow_pin, Channel::Ui)
        {
            self.error = Some(e);
        }
        ui.separator();
        ui.strong("Pinned measurements");
        let pins = self.feed.pins.clone();
        for pin in pins {
            if ui
                .selectable_label(self.selected_pin == Some(pin.id), &pin.name)
                .on_hover_text("Inspect or replace this retained measurement's references")
                .clicked()
            {
                self.pin_baseline = Some(pin.clone());
                self.refs = [Some(pin.query.a), Some(pin.query.b)];
                self.selected_pin = Some(pin.id);
                self.name = pin.name;
                self.offset = pin.label_offset_mm;
                self.visible = pin.visible;
                self.picking = None;
                self.update_query();
            }
            if let Some(e) = self.feed.batch.as_ref().and_then(|b| b.pinned.get(&pin.id)) {
                match &e.result {
                    Ok(v) => {
                        ui.small(format!("{:.3} mm", v.minimum_mm));
                    }
                    Err(e) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, &e.message);
                    }
                }
            }
        }
        if let Some(id) = self.selected_pin
            && ui
                .add_enabled(allow_pin, egui::Button::new("Remove pin"))
                .on_hover_text(
                    "Remove this retained measurement as one Undo step; geometry is unchanged",
                )
                .clicked()
        {
            match prepare_remove(s, s.revision(), Actor::local(Channel::Ui), id)
                .and_then(|p| s.commit(p))
            {
                Ok(_) => {
                    self.selected_pin = None;
                    self.notice = Some(("Pinned measurement removed".into(), false));
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if self.feed.error.is_some()
            && ui
                .button("Retry measurement")
                .on_hover_text("Re-evaluate the current canonical source and references")
                .clicked()
        {
            self.feed.retry();
        }
    }
    pub fn pointer(&mut self, s: &Session, view: MeasurementViewport<'_>) {
        let MeasurementViewport {
            scene,
            camera,
            rect,
            pointer,
            clicked,
            section,
        } = view;
        let Some(side) = self.picking.filter(|_| self.active) else {
            return;
        };
        let result = (|| -> Result<Option<GeometryReference>, String> {
            let scene = scene.ok_or("Wait for the current model")?;
            let stamp = self.feed.stamp.ok_or("Wait for current source")?;
            if !scene.is_full_model()
                || scene.stamp().document != s.document().id()
                || scene.stamp().revision != s.revision()
                || scene
                    .model_evaluation()
                    .is_none_or(|i| i.source_hash != stamp.source_hash)
            {
                return Err(
                    "Wait for the current complete model before choosing a measurement reference"
                        .into(),
                );
            }
            let Some(point) = pointer.filter(|p| rect.contains(*p)) else {
                return Ok(None);
            };
            let key = PickKey {
                stamp: scene.stamp(),
                camera,
                rect,
                point,
                filter: self.filter,
                section,
            };
            if self
                .pick_cache
                .as_ref()
                .is_none_or(|(previous, old, _)| !Arc::ptr_eq(previous, scene) || *old != key)
            {
                self.pick_cache = Some((
                    scene.clone(),
                    key,
                    scene.pick_screen_with(
                        camera,
                        rect,
                        point,
                        PickOptions {
                            filter: self.filter,
                            include_hidden: false,
                            section,
                        },
                    ),
                ));
            }
            let Some(hit) = self
                .pick_cache
                .as_ref()
                .and_then(|(_, _, hit)| hit.as_ref())
            else {
                return Ok(None);
            };
            if !clicked {
                return Ok(None);
            }
            let mut intent = hit
                .intent
                .clone()
                .ok_or("Selected geometry has no unique persistent provenance")?;
            let index = if side == Side::A { 0 } else { 1 };
            intent.id = self.refs[index]
                .as_ref()
                .map_or_else(EntityId::new, |r| r.intent.id);
            Ok(Some(GeometryReference {
                source: hit.instance,
                intent,
            }))
        })();
        match result {
            Ok(Some(reference)) => {
                let index = if side == Side::A { 0 } else { 1 };
                self.refs[index] = Some(reference);
                self.picking = if side == Side::A && self.refs[1].is_none() {
                    Some(Side::B)
                } else {
                    None
                };
                self.update_query();
            }
            Err(e) => self.error = Some(e),
            _ => {}
        }
    }
}
fn values(ui: &mut egui::Ui, v: &MeasurementValues) {
    ui.strong(format!("Minimum distance · {:.3} mm", v.minimum_mm));
    if let Some(d) = v.axis_components_mm {
        ui.label(format!(
            "ΔX {:+.3} · ΔY {:+.3} · ΔZ {:+.3} mm",
            d[0], d[1], d[2]
        ));
    } else if let Some(message) = &v.axis_warning {
        ui.small(message);
    }
    match &v.angle {
        ScalarMetric::Radians { value } => {
            ui.label(format!("Unoriented angle · {:.3}°", value.to_degrees()));
        }
        ScalarMetric::Unsupported { reason } => {
            ui.small(reason);
        }
        _ => {}
    }
    if let ScalarMetric::Millimeters { value } = v.parallel_plane_separation {
        ui.label(format!("Supporting-plane separation · {value:.3} mm"));
    }
    ui.collapsing("Accuracy",|ui|{ui.small(format!("Tolerance {} mm · Boolean fuzz {} mm",v.tolerance_mm,v.boolean_fuzzy_mm));ui.small(format!("{} native closest-point solution(s){}",v.total_solutions,if v.witnesses_complete{""}else{" · bounded witness display"}));if v.inner_solution{ui.small("A solid contains the other selection; zero distance does not distinguish contact from penetration.");}});
}

#[cfg(test)]
mod tests;
