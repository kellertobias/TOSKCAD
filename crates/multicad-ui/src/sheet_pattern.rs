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

#[derive(Default)]
pub(crate) struct Workspace {
    backend: Option<Box<dyn Backend>>,
    view: Option<View>,
}

struct View {
    source: (EntityId, u64, EntityId),
    result: Option<Result<Pattern, String>>,
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
    ) -> Result<(), String> {
        let source = (document.id(), revision, part);
        if self
            .view
            .as_ref()
            .is_some_and(|view| view.source == source && !matches!(view.result, Some(Err(_))))
        {
            return Ok(());
        }
        self.close();
        let backend = self.backend.get_or_insert_with(|| factory());
        backend.request(document, revision, part)?;
        self.view = Some(View {
            source,
            result: None,
            scale: 1.,
            pan: Vec2::ZERO,
        });
        Ok(())
    }

    pub(crate) fn active(&self) -> bool {
        self.view.is_some()
    }

    pub(crate) fn close(&mut self) {
        if self.view.take().is_some()
            && let Some(backend) = &mut self.backend
        {
            backend.cancel();
        }
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &multicad_core::commands::Session,
        scene: &crate::viewport::SceneFeed,
    ) -> bool {
        let Some(view) = &mut self.view else {
            return false;
        };
        let document = session.document();
        if (view.source.0, view.source.1) != (document.id(), session.revision())
            || ui.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.close();
            return false;
        }
        if view.result.is_none() {
            view.result = self.backend.as_mut().and_then(|backend| backend.poll());
        }
        let part = view.source.2;
        let folded = crate::controls::inspector_panel(ui, "source-inspector")
            .show(ui, |ui| {
                crate::controls::inspector_layout(ui);
                ui.heading("Inspector");
                ui.separator();
                if let Some(sheet) = document.get(part) {
                    ui.strong(&sheet.name);
                }
                ui.separator();
                sheet_inspector(ui, true, current_bends(scene, session, part))
            })
            .inner;
        if folded {
            self.close();
            ui.ctx().request_repaint();
            return true;
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
                        paint(ui, pattern, &mut view.scale, &mut view.pan);
                    }
                    Some(Err(error)) => {
                        ui.colored_label(Color32::LIGHT_RED, error);
                        ui.label("Use Fold/Unfold in the Inspector to return to the 3D model.");
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

pub(crate) fn current_bends<'a>(
    feed: &'a crate::viewport::SceneFeed,
    session: &multicad_core::commands::Session,
    part: EntityId,
) -> Result<&'a [Bend], &'static str> {
    if !feed.is_current(session.document().id(), session.revision()) {
        return Err(if feed.error().is_some() {
            "Bend schedule unavailable until the Model rebuild succeeds."
        } else {
            "Waiting for the current native sheet…"
        });
    }
    feed.visible()
        .filter(|scene| scene.is_full_model())
        .and_then(|scene| scene.sheet_bend_schedule(part))
        .ok_or("Bend schedule is unavailable for this sheet.")
}

pub(crate) fn sheet_inspector(
    ui: &mut egui::Ui,
    flat: bool,
    bends: Result<&[Bend], &'static str>,
) -> bool {
    ui.strong("Sheet metal");
    let mut toggle = false;
    ui.horizontal_wrapped(|ui| {
        toggle = ui
            .add_enabled(flat || bends.is_ok(), egui::Button::new("Fold/Unfold"))
            .on_hover_text(if flat {
                "Return to the folded 3D view"
            } else {
                "Show the developed flat pattern"
            })
            .clicked();
        ui.small(if flat { "Flat view" } else { "Folded view" });
    });
    egui::CollapsingHeader::new("Bend schedule")
        .default_open(true)
        .show(ui, |ui| match bends {
            Ok([]) => {
                ui.label("This sheet has no bends.");
            }
            Ok(bends) => {
                ui.small("Proposed bend order");
                for bend in bends {
                    ui.strong(format!(
                        "{} · {:.2}° {}",
                        bend.order + 1,
                        bend.angle_deg,
                        bend.direction
                    ));
                    ui.label(format!(
                        "Inside radius {:.2} mm · Allowance {:.2} mm",
                        bend.inside_radius_mm, bend.allowance_mm
                    ));
                    ui.separator();
                }
            }
            Err(message) => {
                ui.label(message);
            }
        });
    toggle
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
        let session = multicad_core::commands::Session::new(doc.clone()).unwrap();
        let feed = crate::viewport::SceneFeed::default();
        let part = EntityId::new();
        let mut workspace = Workspace::default();
        for failed in [false, true] {
            workspace.open(&factory, &doc, 0, part).unwrap();
            if failed {
                workspace.view.as_mut().unwrap().result = Some(Err("Invalid sheet".into()));
            }
            workspace.close();
            assert!(!workspace.active());
            egui::Context::default()
                .run_ui(Default::default(), |ui| {
                    assert!(!workspace.show(ui, &session, &feed));
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
        let session = multicad_core::commands::Session::new(doc.clone()).unwrap();
        let feed = crate::viewport::SceneFeed::default();
        let part = EntityId::new();
        let mut workspace = Workspace::default();
        workspace.open(&factory, &doc, 0, part).unwrap();
        workspace.open(&factory, &doc, 0, part).unwrap();
        assert_eq!(calls.requests.load(Ordering::Relaxed), 1);

        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |ui| {
            assert!(workspace.show(ui, &session, &feed));
        })
        .drop_without_applying_deltas();
        let other = multicad_core::commands::Session::new(Document::new("Other").unwrap()).unwrap();
        ctx.run_ui(Default::default(), |ui| {
            assert!(!workspace.show(ui, &other, &feed));
        })
        .drop_without_applying_deltas();
        assert!(!workspace.active());
        assert_eq!(calls.cancels.load(Ordering::Relaxed), 1);
    }
}
