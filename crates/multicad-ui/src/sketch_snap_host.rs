//! Canvas bridge for transient sketch placement aids (CAD-136). It resolves the
//! pointer against current solved geometry and paints crosses, the reference
//! midpoint marker, dotted assistive lines and the target label. Nothing here
//! changes the document; placement commits keep using the shared draw API.
use super::*;
use snap::{Axis, Query, SnapGeometry, TargetKind};

const REFERENCE_COLOR: Color32 = Color32::from_rgb(255, 170, 60);
const GUIDE_COLOR: Color32 = Color32::from_rgb(120, 200, 255);

fn to_screen(camera: Camera, frame: Frame, rect: Rect) -> impl Fn([f64; 2]) -> Option<[f32; 2]> {
    move |p| {
        let s = camera.project(frame.point(p).map(|v| v as f32), rect);
        (s[0].is_finite() && s[1].is_finite()).then_some([s[0], s[1]])
    }
}

impl SketchEditor {
    /// Starts a canvas frame. References live only while a draw tool is
    /// active in an editing sketch; leaving that state clears them.
    pub(super) fn begin_snap_frame(&mut self) {
        self.snap_assist.begin_frame();
        if !self.editing || self.tool.is_none() {
            self.snap_assist.clear();
        }
    }
    /// Explicitly removes all acquired references and pending dwell.
    pub fn clear_snap_references(&mut self) {
        self.snap_assist.clear();
    }
    /// Resolves the displayed and committed placement for one pointer sample.
    /// Geometry targets win over the grid; the grid applies only otherwise.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn snap_pointer(
        &mut self,
        ui: &egui::Ui,
        session: &Session,
        sketch: EntityId,
        frame: Frame,
        rect: Rect,
        pointer: Pos2,
        raw: [f64; 2],
    ) -> ([f64; 2], Option<EntityId>) {
        let grid = self.snap.then_some(self.spacing);
        self.snap_assist.bind(sketch);
        if !self.geometry_snap {
            self.snap_assist.pointer_left();
            return snap::placement(None, raw, grid);
        }
        let geometry = self
            .read_solved_source(session.document(), sketch, session.revision())
            .map(|(source, _, _)| SnapGeometry::from_sketch(&source.sketch))
            .unwrap_or_default();
        self.snap_assist.revalidate(&geometry);
        let project = to_screen(self.camera, frame, rect);
        let query = Query {
            pointer_mm: raw,
            pointer_px: [pointer.x, pointer.y],
            to_screen: &project,
        };
        let now = ui.input(|i| i.time);
        let outcome = self.snap_assist.update(&geometry, &query, now);
        if let Some(wait) = outcome.repaint_after {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.)));
        }
        match outcome.event {
            Some(snap::DwellEvent::Acquired(_)) => {
                self.message = format!(
                    "Reference acquired ({} held) · align horizontally or vertically · dwell again to release",
                    self.snap_assist.references().len()
                );
            }
            Some(snap::DwellEvent::Released(_)) => {
                self.message = "Reference released".into();
            }
            None => {}
        }
        snap::placement(outcome.target.as_ref(), raw, grid)
    }
    /// Paints references, the reference midpoint and the current target. Aids
    /// are projected from sketch coordinates every frame, so they stay
    /// anchored through pan and zoom.
    pub(super) fn paint_snap_aids(&self, painter: &egui::Painter, session: &Session, rect: Rect) {
        if !self.editing
            || self.tool.is_none()
            || self.snap_assist.references().is_empty() && self.snap_assist.shown().is_none()
        {
            return;
        }
        let Some(id) = self.selected else { return };
        let Ok((_, frame, _)) = self.read_sketch(session.document(), id, session.revision()) else {
            return;
        };
        let project = to_screen(self.camera, frame, rect);
        let screen = |p: [f64; 2]| project(p).map(|s| Pos2::new(s[0], s[1]));
        let references = self.snap_assist.references();
        for r in references {
            if let Some(c) = screen(r.position) {
                let stroke = Stroke::new(1.5, REFERENCE_COLOR);
                let d = 5.;
                painter.line_segment([c - Vec2::splat(d), c + Vec2::splat(d)], stroke);
                painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
            }
        }
        if let [a, b] = references
            && let Some(m) = screen([
                (a.position[0] + b.position[0]) / 2.,
                (a.position[1] + b.position[1]) / 2.,
            ])
        {
            let d = 4.;
            painter.add(egui::Shape::closed_line(
                vec![
                    m + Vec2::new(0., -d),
                    m + Vec2::new(d, 0.),
                    m + Vec2::new(0., d),
                    m + Vec2::new(-d, 0.),
                ],
                Stroke::new(1., REFERENCE_COLOR),
            ));
        }
        let Some(target) = self.snap_assist.shown() else {
            return;
        };
        let Some(at) = screen(target.position) else {
            return;
        };
        for guide in &target.guides {
            let Some(from) = screen(guide.from) else {
                continue;
            };
            let direction = at - from;
            let extension = if direction.length() > 1e-3 {
                direction.normalized() * 24.
            } else {
                Vec2::ZERO
            };
            painter.extend(egui::Shape::dotted_line(
                &[from, at + extension],
                GUIDE_COLOR,
                5.,
                1.,
            ));
        }
        let label = match target.kind {
            TargetKind::Guide(Axis::Horizontal) => "Horizontal",
            TargetKind::Guide(Axis::Vertical) => "Vertical",
            kind => kind.label(),
        };
        painter.text(
            at + Vec2::new(10., -10.),
            Align2::LEFT_BOTTOM,
            label,
            FontId::proportional(11.),
            GUIDE_COLOR,
        );
    }
}
