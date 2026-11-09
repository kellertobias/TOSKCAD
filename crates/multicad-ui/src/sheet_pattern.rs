//! Native flat pattern and sheet sketch entry.
use eframe::egui::{self, Align2, Color32, Pos2, Stroke, Vec2};
use multicad_core::{
    commands::{Channel, Session},
    document::{Document, EntityId},
    manufacturing::{PlanarContours, PlanarEdge, PlanarLoop},
    model::CutoutFace,
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
    pub chart: Chart,
}

pub struct Chart {
    pub base: PlanarLoop,
    pub legs: Vec<(u32, [[f64; 2]; 4])>,
    pub bend_zones: Vec<[[f64; 2]; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Show {
    Hidden,
    Visible,
    Sketch(CutoutFace, Channel),
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
    result: Option<Result<Arc<Pattern>, String>>,
    backdrop: Option<Arc<Pattern>>,
    picking: Option<Channel>,
    authoring: bool,
    pick_error: Option<&'static str>,
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
            backdrop: None,
            picking: None,
            authoring: false,
            pick_error: None,
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

    pub(crate) fn part(&self) -> Option<EntityId> {
        self.view.as_ref().map(|view| view.source.2)
    }

    pub(crate) fn begin_pick(&mut self, channel: Channel) -> Result<(), String> {
        let view = self.view.as_mut().ok_or("Unfold a sheet first")?;
        if let Some(Err(error)) = &view.result {
            return Err(error.clone());
        }
        view.authoring = false;
        view.picking = Some(channel);
        view.pick_error = None;
        Ok(())
    }

    pub(crate) fn authoring(&self) -> bool {
        self.view.as_ref().is_some_and(|view| view.authoring)
    }

    pub(crate) fn set_authoring(&mut self, active: bool) {
        if let Some(view) = &mut self.view {
            view.authoring = active;
            view.picking = None;
        }
    }

    pub(crate) fn backdrop(&self) -> Option<Arc<Pattern>> {
        self.view.as_ref()?.backdrop.clone()
    }

    pub(crate) fn pending(&self) -> bool {
        self.view.as_ref().is_some_and(|view| view.result.is_none())
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.view
            .as_ref()?
            .result
            .as_ref()?
            .as_ref()
            .err()
            .map(String::as_str)
    }

    pub(crate) fn update(&mut self, session: &Session) {
        let Some(view) = &mut self.view else {
            return;
        };
        let document = session.document();
        if view.source.0 != document.id() || document.get(view.source.2).is_none() {
            self.close();
            return;
        }
        if view.source.1 != session.revision() {
            view.source.1 = session.revision();
            view.result = self.backend.as_mut().and_then(|backend| {
                backend
                    .request(document, view.source.1, view.source.2)
                    .err()
                    .map(Err)
            });
        }
        if view.result.is_none() {
            view.result = self
                .backend
                .as_mut()
                .and_then(|backend| backend.poll())
                .map(|result| result.map(Arc::new));
            if let Some(Ok(pattern)) = &view.result {
                view.backdrop = Some(pattern.clone());
            }
        }
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        scene: &crate::viewport::SceneFeed,
    ) -> Show {
        self.update(session);
        let Some(view) = &mut self.view else {
            return Show::Hidden;
        };
        if ui.is_enabled()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            if view.picking.take().is_some() {
                view.pick_error = None;
            } else {
                self.close();
                return Show::Hidden;
            }
        }
        let document = session.document();
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
            return Show::Visible;
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::same(crate::theme::GUTTER))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.heading("Flat pattern");
                    if view.backdrop.is_some() && ui.button("Fit pattern").clicked() {
                        view.pan = Vec2::ZERO;
                        view.scale = 1.;
                    }
                    if view.picking.is_some() && ui.button("Cancel sketch").clicked() {
                        view.picking = None;
                        view.pick_error = None;
                    }
                });
                if view.picking.is_some() {
                    ui.label("Click the base or a flat flange leg to start the sketch.");
                    if let Some(message) = view.pick_error {
                        ui.colored_label(Color32::LIGHT_YELLOW, message);
                    }
                }
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
                        let point = paint(
                            ui,
                            pattern,
                            &mut view.scale,
                            &mut view.pan,
                            view.picking.is_some(),
                        );
                        if let Some((point, scale)) = point
                            && let Some(channel) = view.picking
                        {
                            match pattern.face_at(point, scale) {
                                Ok(face) => return Show::Sketch(face, channel),
                                Err(error) => {
                                    view.pick_error = Some(error);
                                    ui.ctx().request_repaint();
                                }
                            }
                        }
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
                        if let Some(pattern) = &view.backdrop {
                            paint(ui, pattern, &mut view.scale, &mut view.pan, false);
                        }
                        ui.ctx().request_repaint_after(Duration::from_millis(32));
                    }
                }
                Show::Visible
            })
            .inner
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

impl Pattern {
    pub(crate) fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for edge in self.cut.loops.iter().flat_map(|wire| &wire.edges) {
            let [a, b] = edge.bounds();
            for axis in 0..2 {
                min[axis] = min[axis].min(a[axis]);
                max[axis] = max[axis].max(b[axis]);
            }
        }
        min.iter()
            .chain(&max)
            .all(|value| value.is_finite())
            .then_some([min, max])
    }

    pub(crate) fn paint_overlay(
        &self,
        painter: &egui::Painter,
        scale: f32,
        project: impl Fn([f64; 2]) -> Pos2,
    ) {
        self.paint_cut_overlay(painter, scale, project, &self.cut);
    }

    pub(crate) fn paint_cut_overlay(
        &self,
        painter: &egui::Painter,
        scale: f32,
        project: impl Fn([f64; 2]) -> Pos2,
        cut: &PlanarContours,
    ) {
        let stroke = Stroke::new(1.5, CUT);
        for edge in cut.loops.iter().flat_map(|wire| &wire.edges) {
            if let PlanarEdge::Line { a, b } = *edge {
                painter.line_segment([project(a), project(b)], stroke);
            } else {
                painter.add(egui::Shape::line(
                    edge_points(edge, scale).into_iter().map(&project).collect(),
                    stroke,
                ));
            }
        }
        for bend in &self.bends {
            let [a, b] = bend.line.map(&project);
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
        for center in &self.drills {
            let p = project(*center);
            for delta in [egui::vec2(3., 0.), egui::vec2(0., 3.)] {
                painter.line_segment([p - delta, p + delta], Stroke::new(1., DRILL));
            }
        }
    }

    fn face_at(&self, point: [f64; 2], scale: f32) -> Result<CutoutFace, &'static str> {
        let in_quad = |quad: &[[f64; 2]; 4]| {
            inside(
                point,
                quad.iter()
                    .copied()
                    .zip(quad.iter().copied().cycle().skip(1)),
            )
        };
        if self.chart.bend_zones.iter().any(in_quad) {
            return Err("This is a bend zone. Choose the base or a flat flange leg.");
        }
        let in_loop = |wire: &PlanarLoop| {
            inside(
                point,
                wire.edges.iter().flat_map(|edge| {
                    let points = edge_points(edge, scale);
                    points.clone().into_iter().zip(points.into_iter().skip(1))
                }),
            )
        };
        if self
            .cut
            .loops
            .iter()
            .any(|wire| !wire.outer && in_loop(wire))
        {
            return Err("This is an existing opening. Click on sheet material.");
        }
        if in_loop(&self.chart.base) {
            return Ok(CutoutFace::Base);
        }
        self.chart
            .legs
            .iter()
            .find(|(_, quad)| in_quad(quad))
            .map(|(index, _)| CutoutFace::Flange { index: *index })
            .ok_or("Click inside the sheet base or a flat flange leg.")
    }
}

// Screen picking uses the displayed contours. The native cut checks the entire exact region.
fn inside(point: [f64; 2], edges: impl Iterator<Item = ([f64; 2], [f64; 2])>) -> bool {
    edges
        .filter(|(a, b)| {
            ((a[1] > point[1]) != (b[1] > point[1]))
                && point[0] < a[0] + (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1])
        })
        .count()
        % 2
        != 0
}

fn paint(
    ui: &mut egui::Ui,
    pattern: &Pattern,
    zoom: &mut f32,
    pan: &mut Vec2,
    picking: bool,
) -> Option<([f64; 2], f32)> {
    let (response, painter) =
        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = response.rect;
    let [min, max] = pattern.bounds()?;
    let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    let fit = (rect.width() - 54.).max(1.) / (max[0] - min[0]).max(0.001) as f32;
    let fit = fit.min((rect.height() - 54.).max(1.) / (max[1] - min[1]).max(0.001) as f32);
    if response.dragged() {
        *pan += response.drag_delta();
    }
    if let Some(pointer) = response.hover_pos() {
        if picking {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let delta = ui.input(|input| input.smooth_scroll_delta.y);
        if delta != 0. {
            let next = (*zoom * (delta * 0.008).exp()).clamp(0.25, 24.);
            let anchor = pointer - rect.center();
            *pan = anchor + (*pan - anchor) * (next / *zoom);
            *zoom = next;
        }
    }
    let scale = fit * *zoom;
    let origin = rect.center() + *pan;
    let project = |point: [f64; 2]| {
        origin
            + egui::vec2(
                (point[0] - center[0]) as f32 * scale,
                (center[1] - point[1]) as f32 * scale,
            )
    };
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 0., Color32::from_rgb(19, 26, 35));
    pattern.paint_overlay(&painter, scale, project);
    if !picking || !response.clicked() {
        return None;
    }
    let pointer = response.interact_pointer_pos()? - origin;
    Some((
        [
            center[0] + f64::from(pointer.x / scale),
            center[1] - f64::from(pointer.y / scale),
        ],
        scale,
    ))
}

fn edge_points(edge: &PlanarEdge, scale: f32) -> Vec<[f64; 2]> {
    match *edge {
        PlanarEdge::Line { a, b } => vec![a, b],
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
            std::iter::once(a)
                .chain((1..segments).map(|step| {
                    let angle = start + sweep_rad * step as f64 / segments as f64;
                    [
                        center[0] + radius * angle.cos(),
                        center[1] + radius * angle.sin(),
                    ]
                }))
                .chain(std::iter::once(b))
                .collect()
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
                    assert_eq!(workspace.show(ui, &session, &feed), Show::Hidden);
                })
                .drop_without_applying_deltas();
        }
        assert_eq!(calls.cancels.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn flat_view_refreshes_revisions_and_closes_for_another_document() {
        let calls = Arc::new(Calls::default());
        let fake = calls.clone();
        let factory: BackendFactory = Arc::new(move || Box::new(Fake(fake.clone())));
        let mut session = Session::new(Document::new("Sheet").unwrap()).unwrap();
        let part_entity = multicad_core::document::Entity::new(
            "Sheet Part",
            multicad_core::document::EntityData::Part {
                features: vec![],
                material: None,
                tags: Default::default(),
            },
        );
        let part = part_entity.id;
        session
            .apply(multicad_core::commands::Request {
                expected_revision: 0,
                actor: multicad_core::commands::Actor::local(Channel::Ui),
                label: "Create test Part".into(),
                edits: vec![multicad_core::document::Mutation::Insert(part_entity)],
            })
            .unwrap();
        let doc = session.document();
        let feed = crate::viewport::SceneFeed::default();
        let mut workspace = Workspace::default();
        workspace
            .open(&factory, doc, session.revision(), part)
            .unwrap();
        workspace
            .open(&factory, doc, session.revision(), part)
            .unwrap();
        assert_eq!(calls.requests.load(Ordering::Relaxed), 1);

        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |ui| {
            assert_eq!(workspace.show(ui, &session, &feed), Show::Visible);
        })
        .drop_without_applying_deltas();
        workspace.set_authoring(true);
        let mut renamed = session.document().get(part).unwrap().clone();
        renamed.name = "Edited Sheet".into();
        session
            .apply(multicad_core::commands::Request {
                expected_revision: session.revision(),
                actor: multicad_core::commands::Actor::local(Channel::Ui),
                label: "Change the source while authoring".into(),
                edits: vec![multicad_core::document::Mutation::Replace(renamed)],
            })
            .unwrap();
        workspace.update(&session);
        assert_eq!(calls.requests.load(Ordering::Relaxed), 2);
        assert_eq!(workspace.part(), Some(part));
        assert!(workspace.authoring() && workspace.pending());

        let other = multicad_core::commands::Session::new(Document::new("Other").unwrap()).unwrap();
        ctx.run_ui(Default::default(), |ui| {
            assert_eq!(workspace.show(ui, &other, &feed), Show::Hidden);
        })
        .drop_without_applying_deltas();
        assert!(!workspace.active());
        assert_eq!(calls.cancels.load(Ordering::Relaxed), 1);
    }
}
