//! Graphical inputs and resolver-derived plan cues; native Preview owns solids.
use super::*;
use multicad_core::parameters::EvaluatedParameters;

impl ArchitecturePanel {
    pub(super) fn stair_viewport(
        &mut self,
        document: &Document,
        parameters: &EvaluatedParameters,
        view: TerminationViewport<'_>,
    ) -> bool {
        let elevation = self
            .level
            .ok_or_else(|| "Choose the stair's base level".to_owned())
            .and_then(|level| {
                model::resolve_level(document, parameters, level)
                    .map(|level| level.elevation_mm)
                    .map_err(|e| e.to_string())
            });
        let elevation = match elevation {
            Ok(value) => value,
            Err(error) => {
                view.painter.text(
                    view.rect.left_top() + egui::vec2(12., 12.),
                    egui::Align2::LEFT_TOP,
                    error,
                    egui::FontId::proportional(12.),
                    egui::Color32::LIGHT_RED,
                );
                return false;
            }
        };
        let frame = multicad_core::sketch::Frame {
            origin: [0., 0., elevation],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            n: [0., 0., 1.],
        };
        let hit = view
            .pointer
            .filter(|point| view.rect.contains(*point))
            .and_then(|point| view.camera.on_plane(point, view.rect, frame))
            .map(|point| {
                if self.stair_snap {
                    point.map(|value| (value / 10.).round() * 10.)
                } else {
                    point
                }
            });
        let mut changed = false;
        if self.stair_placing
            && view.clicked
            && let Some(point) = hit
        {
            if let Some(start) = self.stair_anchor {
                let delta = [point[0] - start[0], point[1] - start[1]];
                if delta[0].hypot(delta[1]) > 1e-6 {
                    self.direction_deg = delta[1].atan2(delta[0]).to_degrees();
                    self.stair_placing = false;
                    self.stair_anchor = None;
                    changed = true;
                } else {
                    self.error = Some("Choose a direction away from the first riser".into());
                }
            } else {
                self.start = point;
                self.stair_anchor = Some(point);
                changed = true;
            }
        }
        if changed {
            self.stamp.changed();
            self.error = None;
        }
        let mut definition = match self.stair(document) {
            Ok(value) => value,
            Err(_) => return changed,
        };
        if self.stair_placing
            && let Some(point) = hit
        {
            if let Some(start) = self.stair_anchor {
                let delta = [point[0] - start[0], point[1] - start[1]];
                if delta[0].hypot(delta[1]) > 1e-6 {
                    definition.direction_deg = delta[1].atan2(delta[0]).to_degrees();
                }
            } else {
                definition.start = point.map(length);
            }
        }
        if let Ok(resolved) = model::resolve_stair(document, parameters, &definition) {
            let project = |point: [f64; 2]| {
                let pixel = view.camera.project(
                    [point[0] as f32, point[1] as f32, elevation as f32],
                    view.rect,
                );
                egui::pos2(pixel[0], pixel[1])
            };
            let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(230, 190, 95));
            for piece in &resolved.pieces {
                let low = piece
                    .profile
                    .iter()
                    .map(|point| point[0])
                    .fold(f64::INFINITY, f64::min);
                let high = piece
                    .profile
                    .iter()
                    .map(|point| point[0])
                    .fold(f64::NEG_INFINITY, f64::max);
                let side = [-piece.along[1], piece.along[0]];
                let at = |along: f64, across: f64| {
                    std::array::from_fn(|axis| {
                        piece.origin[axis] + piece.along[axis] * along + side[axis] * across
                    })
                };
                let corners = [
                    at(low, 0.),
                    at(high, 0.),
                    at(high, piece.width_mm),
                    at(low, piece.width_mm),
                    at(low, 0.),
                ];
                view.painter.add(egui::Shape::line(
                    corners.into_iter().map(project).collect(),
                    stroke,
                ));
            }
            for segment in resolved.walking_line.windows(2) {
                let start = project(segment[0]);
                let end = project(segment[1]);
                view.painter.arrow(start, end - start, stroke);
            }
            if let Some(start) = resolved.walking_line.first() {
                let point = project(*start);
                view.painter.circle_filled(point, 3., stroke.color);
                view.painter.text(
                    point + egui::vec2(6., -6.),
                    egui::Align2::LEFT_BOTTOM,
                    "First riser",
                    egui::FontId::proportional(11.),
                    stroke.color,
                );
            }
        }
        changed
    }
}
