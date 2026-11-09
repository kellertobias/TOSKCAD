//! Model's read-only view of the native developed sheet.
use eframe::egui::{self, Align2, Color32, Pos2, Stroke, Vec2};
use multicad_core::{
    document::{Document, EntityId},
    manufacturing::{PlanarContours, PlanarEdge},
};
use std::{sync::Arc, time::Duration};

pub trait Backend: Send {
    fn request(&mut self, document: &Document, revision: u64, part: EntityId)
    -> Result<(), String>;
    fn poll(&mut self) -> Option<Result<Pattern, String>>;
    fn cancel(&mut self);
}

pub type BackendFactory = Arc<dyn Fn() -> Box<dyn Backend> + Send + Sync>;

pub struct Pattern {
    pub name: String,
    pub cut: PlanarContours,
    pub size_mm: [f64; 2],
    pub thickness_mm: f64,
    pub bends: Vec<Bend>,
    pub drills: Vec<[f64; 2]>,
}

pub struct Bend {
    pub order: usize,
    pub line: [[f64; 2]; 2],
    pub direction: &'static str,
    pub angle_deg: f64,
    pub inside_radius_mm: f64,
    pub allowance_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Unfold,
    Fold,
    BendSchedule,
}

#[derive(Default)]
pub(crate) struct Workspace {
    backend: Option<Box<dyn Backend>>,
    view: Option<View>,
}

struct View {
    source: (EntityId, u64, EntityId),
    result: Option<Result<Pattern, String>>,
    schedule: bool,
    scale: f32,
    pan: Vec2,
}

impl Workspace {
    pub(crate) fn open(
        &mut self,
        factory: &BackendFactory,
        document: &Document,
        revision: u64,
        part: EntityId,
        schedule: bool,
    ) -> Result<(), String> {
        let source = (document.id(), revision, part);
        if let Some(view) = &mut self.view
            && view.source == source
            && !matches!(view.result, Some(Err(_)))
        {
            view.schedule = schedule;
            return Ok(());
        }
        self.close();
        let backend = self.backend.get_or_insert_with(|| factory());
        backend.request(document, revision, part)?;
        self.view = Some(View {
            source,
            result: None,
            schedule,
            scale: 1.,
            pan: Vec2::ZERO,
        });
        Ok(())
    }

    pub(crate) fn active(&self) -> bool {
        self.view.is_some()
    }

    pub(crate) fn schedule(&self) -> bool {
        self.view.as_ref().is_some_and(|view| view.schedule)
    }

    pub(crate) fn close(&mut self) {
        if self.view.take().is_some()
            && let Some(backend) = &mut self.backend
        {
            backend.cancel();
        }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, document: EntityId, revision: u64) -> bool {
        let Some(view) = &mut self.view else {
            return false;
        };
        if (view.source.0, view.source.1) != (document, revision)
            || ui.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.close();
            return false;
        }
        if view.result.is_none() {
            view.result = self.backend.as_mut().and_then(|backend| backend.poll());
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::same(crate::theme::GUTTER))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.heading("Flat pattern");
                    if matches!(view.result, Some(Ok(_))) && ui.button("Fit pattern").clicked() {
                        view.pan = Vec2::ZERO;
                        view.scale = 1.;
                    }
                });
                match &view.result {
                    Some(Ok(pattern)) => {
                        ui.label(format!(
                            "{} · {:.2} × {:.2} mm · {:.2} mm thick · {} bends · {} drill points",
                            pattern.name,
                            pattern.size_mm[0],
                            pattern.size_mm[1],
                            pattern.thickness_mm,
                            pattern.bends.len(),
                            pattern.drills.len()
                        ));
                        ui.horizontal(|ui| {
                            ui.colored_label(CUT, "Cut contours");
                            ui.colored_label(BEND, "Bend lines");
                            ui.colored_label(DRILL, "Drill centers");
                        });
                        if view.schedule {
                            egui::Panel::right("sheet-bend-schedule")
                                .exact_size(245.)
                                .frame(egui::Frame::new().inner_margin(egui::Margin::same(10)))
                                .show(ui, |ui| schedule(ui, pattern));
                        }
                        paint(ui, pattern, &mut view.scale, &mut view.pan);
                    }
                    Some(Err(error)) => {
                        ui.colored_label(Color32::LIGHT_RED, error);
                        ui.label("Use Fold in the ribbon to return to the 3D model.");
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Generating flat pattern…");
                        });
                        ui.ctx().request_repaint_after(Duration::from_millis(32));
                    }
                }
            });
        true
    }
}

const CUT: Color32 = Color32::from_rgb(211, 228, 243);
const BEND: Color32 = Color32::from_rgb(255, 183, 91);
const DRILL: Color32 = Color32::from_rgb(102, 213, 237);

fn schedule(ui: &mut egui::Ui, pattern: &Pattern) {
    ui.heading("Bend schedule");
    ui.label("Proposed bend order");
    ui.separator();
    if pattern.bends.is_empty() {
        ui.label("This sheet has no bends.");
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        for bend in &pattern.bends {
            ui.strong(format!(
                "{} · {:.2}° {}",
                bend.order + 1,
                bend.angle_deg,
                bend.direction
            ));
            ui.label(format!(
                "Inside radius {:.2} mm\nAllowance {:.2} mm",
                bend.inside_radius_mm, bend.allowance_mm
            ));
            ui.separator();
        }
    });
}

fn paint(ui: &mut egui::Ui, pattern: &Pattern, zoom: &mut f32, pan: &mut Vec2) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::drag());
    let rect = response.rect;
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for edge in pattern.cut.loops.iter().flat_map(|loop_| &loop_.edges) {
        let [a, b] = edge.bounds();
        for axis in 0..2 {
            min[axis] = min[axis].min(a[axis]);
            max[axis] = max[axis].max(b[axis]);
        }
    }
    if !min.iter().chain(&max).all(|value| value.is_finite()) {
        return;
    }
    let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    let fit = (rect.width() - 54.).max(1.) / (max[0] - min[0]).max(0.001) as f32;
    let fit = fit.min((rect.height() - 54.).max(1.) / (max[1] - min[1]).max(0.001) as f32);
    if response.dragged() {
        *pan += response.drag_delta();
    }
    if let Some(pointer) = response.hover_pos() {
        let delta = ui.input(|input| input.smooth_scroll_delta.y);
        if delta != 0. {
            let next = (*zoom * (delta * 0.008).exp()).clamp(0.25, 24.);
            let anchor = pointer - rect.center();
            *pan = anchor + (*pan - anchor) * (next / *zoom);
            *zoom = next;
        }
    }
    let scale = fit * *zoom;
    let project = |point: [f64; 2]| {
        rect.center()
            + *pan
            + egui::vec2(
                (point[0] - center[0]) as f32 * scale,
                (center[1] - point[1]) as f32 * scale,
            )
    };
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 0., Color32::from_rgb(19, 26, 35));
    for edge in pattern.cut.loops.iter().flat_map(|loop_| &loop_.edges) {
        paint_edge(&painter, edge, scale, &project);
    }
    for bend in &pattern.bends {
        let [a, b] = bend.line.map(project);
        painter.extend(egui::Shape::dashed_line(
            &[a, b],
            Stroke::new(1.5, BEND),
            6.,
            4.,
        ));
        painter.text(
            a.lerp(b, 0.5) - egui::vec2(0., 4.),
            Align2::CENTER_BOTTOM,
            bend.order + 1,
            egui::FontId::proportional(12.),
            BEND,
        );
    }
    for center in &pattern.drills {
        let p = project(*center);
        for delta in [egui::vec2(3., 0.), egui::vec2(0., 3.)] {
            painter.line_segment([p - delta, p + delta], Stroke::new(1., DRILL));
        }
    }
}

fn paint_edge(
    painter: &egui::Painter,
    edge: &PlanarEdge,
    scale: f32,
    project: &impl Fn([f64; 2]) -> Pos2,
) {
    let stroke = Stroke::new(1.5, CUT);
    match *edge {
        PlanarEdge::Line { a, b } => {
            painter.line_segment([project(a), project(b)], stroke);
        }
        PlanarEdge::Arc {
            a,
            b,
            center,
            sweep_rad,
        } => {
            let dx = a[0] - center[0];
            let dy = a[1] - center[1];
            let radius = dx.hypot(dy);
            let start = dy.atan2(dx);
            let segments =
                ((radius * sweep_rad.abs() * f64::from(scale) / 5.).ceil() as usize).clamp(8, 512);
            let points = std::iter::once(project(a))
                .chain((1..segments).map(|step| {
                    let angle = start + sweep_rad * step as f64 / segments as f64;
                    project([
                        center[0] + radius * angle.cos(),
                        center[1] + radius * angle.sin(),
                    ])
                }))
                .chain(std::iter::once(project(b)))
                .collect();
            painter.add(egui::Shape::line(points, stroke));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Calls {
        requests: AtomicUsize,
        cancels: AtomicUsize,
    }

    struct Fake(Arc<Calls>);

    impl Backend for Fake {
        fn request(&mut self, _: &Document, _: u64, _: EntityId) -> Result<(), String> {
            self.0.requests.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn poll(&mut self) -> Option<Result<Pattern, String>> {
            None
        }

        fn cancel(&mut self) {
            self.0.cancels.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn closing_a_pending_or_failed_flat_view_drops_it() {
        let calls = Arc::new(Calls::default());
        let fake = calls.clone();
        let factory: BackendFactory = Arc::new(move || Box::new(Fake(fake.clone())));
        let doc = Document::new("Sheet").unwrap();
        let part = EntityId::new();
        let mut workspace = Workspace::default();
        for failed in [false, true] {
            workspace.open(&factory, &doc, 0, part, false).unwrap();
            if failed {
                workspace.view.as_mut().unwrap().result = Some(Err("Invalid sheet".into()));
            }
            workspace.close();
            assert!(!workspace.active());
            egui::Context::default()
                .run_ui(Default::default(), |ui| {
                    assert!(!workspace.show(ui, doc.id(), 0));
                })
                .drop_without_applying_deltas();
        }
        assert_eq!(calls.cancels.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn flat_view_reuses_current_request_and_cancels_after_source_changes() {
        let calls = Arc::new(Calls::default());
        let fake = calls.clone();
        let factory: BackendFactory = Arc::new(move || Box::new(Fake(fake.clone())));
        let doc = Document::new("Sheet").unwrap();
        let part = EntityId::new();
        let mut workspace = Workspace::default();
        workspace.open(&factory, &doc, 1, part, false).unwrap();
        workspace.open(&factory, &doc, 1, part, true).unwrap();
        assert!(workspace.schedule());
        assert_eq!(calls.requests.load(Ordering::Relaxed), 1);

        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |ui| {
            assert!(workspace.show(ui, doc.id(), 1));
        })
        .drop_without_applying_deltas();
        ctx.run_ui(Default::default(), |ui| {
            assert!(!workspace.show(ui, doc.id(), 2));
        })
        .drop_without_applying_deltas();
        assert!(!workspace.active());
        assert_eq!(calls.cancels.load(Ordering::Relaxed), 1);
    }
}
