use super::*;
use multicad_core::references::AnalyticGeometry;
mod angle;

fn color(index: usize) -> egui::Color32 {
    if index == 0 {
        egui::Color32::from_rgb(244, 180, 65)
    } else {
        egui::Color32::from_rgb(85, 173, 241)
    }
}
impl MeasurementInspector {
    pub fn highlights(&mut self, session: &Session, scene: &Arc<Scene>) -> Option<Selection> {
        if !self.active || !self.scene_current(session, scene) {
            return None;
        }
        if self
            .selection_cache
            .as_ref()
            .is_none_or(|(old, _)| !Arc::ptr_eq(old, scene))
        {
            let references = std::array::from_fn(|index| {
                let r = self.refs[index].as_ref()?;
                let snapshot = scene.topology_snapshot(r.source)?;
                match references::resolve(&r.intent, &snapshot, session.revision()) {
                    Resolution::Resolved { key, .. } => Some(ReferenceSelection {
                        instance: r.source,
                        key,
                    }),
                    _ => None,
                }
            });
            self.selection_cache = Some((scene.clone(), references));
        }
        Some(Selection::References(self.selection_cache.as_ref()?.1))
    }
    fn scene_current(&self, s: &Session, scene: &Scene) -> bool {
        self.feed.stamp.is_some_and(|stamp| {
            stamp.document == s.document().id()
                && stamp.revision == s.revision()
                && scene.is_full_model()
                && scene.stamp().document == stamp.document
                && scene.stamp().revision == stamp.revision
                && scene
                    .model_evaluation()
                    .is_some_and(|m| m.source_hash == stamp.source_hash)
        })
    }
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        camera: Camera,
        session: &Session,
        scene: Option<&Arc<Scene>>,
        section: Option<SectionPlane>,
    ) {
        let Some(scene) = scene.filter(|scene| self.scene_current(session, scene)) else {
            return;
        };
        if self.active
            && let Some(Selection::References(references)) = self.highlights(session, scene)
        {
            for (index, reference) in references.into_iter().enumerate() {
                if let Some(r) = reference
                    && r.key.kind == TopologyKind::Vertex
                    && let Some(snapshot) = scene.topology_snapshot(r.instance)
                    && let Some(AnalyticGeometry::Point { position_mm }) = snapshot
                        .elements
                        .iter()
                        .find(|e| e.key == r.key)
                        .and_then(|e| e.geometry)
                    && section.is_none_or(|p| p.contains(position_mm))
                {
                    let p = camera.project(position_mm.map(|v| v as f32), rect);
                    painter.circle_filled(egui::pos2(p[0], p[1]), 4., color(index));
                }
            }
        }
        let mut overlay = Overlay {
            painter,
            rect,
            camera,
            section,
            scene,
            labels: Vec::new(),
            cards: Vec::new(),
        };
        let active = self.query.as_ref().zip(self.ready(session));
        let active_pin = active.and_then(|(query, _)| {
            self.feed
                .pins
                .iter()
                .find(|pin| pin.visible && Some(pin.id) == self.selected_pin && pin.query == *query)
        });
        if let Some((query, value)) = active {
            overlay.measurement(query, value, active_pin, true);
        }
        if let Some(batch) = &self.feed.batch {
            for pin in &self.feed.pins {
                if pin.visible
                    && active_pin.is_none_or(|active| active.id != pin.id)
                    && let Some(Ok(value)) = batch.pinned.get(&pin.id).map(|m| &m.result)
                {
                    overlay.measurement(&pin.query, value, Some(pin), false);
                }
            }
        }
        painter.extend(overlay.cards);
    }
}

struct Overlay<'a> {
    painter: &'a egui::Painter,
    rect: egui::Rect,
    camera: Camera,
    section: Option<SectionPlane>,
    scene: &'a Scene,
    labels: Vec<egui::Rect>,
    cards: Vec<egui::Shape>,
}

const LINE: egui::Color32 = egui::Color32::from_rgb(164, 209, 236);
const LABEL: egui::Color32 = egui::Color32::from_rgb(231, 242, 250);

impl Overlay<'_> {
    fn point(&self, point: [f64; 3]) -> Option<egui::Pos2> {
        let [x, y, depth] = self.camera.project(point.map(|v| v as f32), self.rect);
        let screen = egui::pos2(x, y);
        (screen.is_finite() && (!self.camera.perspective || depth >= 0.1)).then_some(screen)
    }

    fn measurement(
        &mut self,
        query: &MeasurementQuery,
        value: &MeasurementValues,
        pin: Option<&PinnedMeasurement>,
        active: bool,
    ) {
        let Some(witness) = value.witnesses.first() else {
            return;
        };
        let painter = self.painter;
        let width = if active { 1.8 } else { 1.15 };
        let segment = self
            .section
            .map_or(Some([witness.a_mm, witness.b_mm]), |plane| {
                plane.clip_segment(witness.a_mm, witness.b_mm)
            });
        if let Some([a, b]) = segment
            && let Some(line) = self.camera.segment(a, b, self.rect)
        {
            painter.line_segment(line, egui::Stroke::new(width, LINE));
        }
        let points = [witness.a_mm, witness.b_mm].map(|point| {
            self.section
                .is_none_or(|plane| plane.contains(point))
                .then(|| self.point(point))
                .flatten()
        });
        if let [Some(a), Some(b)] = points
            && a.distance(b) < 2.
        {
            painter.circle_stroke(a, 5., egui::Stroke::new(width, color(0)));
            painter.circle_filled(b, 2.8, color(1));
        } else {
            for (index, point) in points.into_iter().enumerate() {
                if let Some(point) = point {
                    painter.circle_filled(point, 3., color(index));
                }
            }
        }

        let marker = angle::marker(
            self.scene,
            query,
            value,
            self.camera,
            self.rect,
            self.section,
        );
        if let Some(marker) = &marker {
            for (index, ray) in marker.rays.iter().enumerate() {
                if let Some(ray) = ray {
                    painter.line_segment(*ray, egui::Stroke::new(width, color(index)));
                }
            }
            for segment in &marker.arc {
                painter.line_segment(*segment, egui::Stroke::new(width, LABEL));
            }
        }
        let midpoint: [f64; 3] =
            std::array::from_fn(|i| witness.a_mm[i] * 0.5 + witness.b_mm[i] * 0.5);
        let offset = pin.map_or([0.; 3], |pin| pin.label_offset_mm);
        let label_point = std::array::from_fn(|i| midpoint[i] + offset[i]);
        if segment.is_none() && marker.is_none() {
            return;
        }
        let Some(anchor) = self
            .section
            .is_none_or(|plane| plane.contains(label_point))
            .then(|| self.point(label_point))
            .flatten()
        else {
            return;
        };
        let distance = crate::preferences::format_mm(value.minimum_mm);
        let metrics = match value.angle {
            ScalarMetric::Radians { value } if marker.is_some() => {
                let degrees = value.to_degrees();
                let angle = if degrees >= 0.1 {
                    format!("{degrees:.1}°")
                } else if degrees >= 0.001 {
                    format!("{degrees:.3}°")
                } else {
                    "<0.001°".into()
                };
                format!("{angle}  ·  {distance}")
            }
            _ => distance,
        };
        self.label(anchor, metrics, pin, active, marker.is_some());
    }

    fn label(
        &mut self,
        anchor: egui::Pos2,
        metrics: String,
        pin: Option<&PinnedMeasurement>,
        active: bool,
        has_angle: bool,
    ) {
        if !self.rect.expand(2.).contains(anchor) {
            return;
        }
        let metrics = self.painter.layout_no_wrap(
            metrics,
            egui::FontId::proportional(if active { 13. } else { 12. }),
            LABEL,
        );
        let title = pin.map(|pin| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                pin.name.clone(),
                egui::FontId::proportional(11.),
                LINE,
            );
            job.wrap.max_width = (self.rect.width() - 32.).clamp(1., 180.);
            job.wrap.max_rows = 1;
            self.painter.layout_job(job)
        });
        let title_size = title
            .as_ref()
            .map_or(egui::Vec2::ZERO, |title| title.size());
        let size = egui::vec2(
            metrics.size().x.max(title_size.x) + 14.,
            metrics.size().y + title_size.y + 8.,
        );
        let gap = if has_angle { 35. } else { 10. };
        let Some(card) = label_rect(self.rect, &self.labels, anchor, size, gap) else {
            return;
        };
        self.labels.push(card);
        let leader = egui::pos2(
            anchor.x.clamp(card.min.x, card.max.x),
            anchor.y.clamp(card.min.y, card.max.y),
        );
        if anchor.distance(leader) > 4. {
            self.painter.line_segment(
                [anchor, leader],
                egui::Stroke::new(1., LINE.gamma_multiply(0.62)),
            );
        }
        self.cards.extend([
            egui::Shape::rect_filled(
                card,
                5.,
                egui::Color32::from_rgba_unmultiplied(15, 24, 35, 240),
            ),
            egui::Shape::rect_stroke(
                card,
                5.,
                egui::Stroke::new(1., LINE.gamma_multiply(if active { 0.75 } else { 0.4 })),
                egui::StrokeKind::Inside,
            ),
        ]);
        let content = card.min + egui::vec2(7., 4.);
        if let Some(title) = title {
            self.cards.push(egui::Shape::galley(content, title, LINE));
        }
        self.cards.push(egui::Shape::galley(
            content + egui::vec2(0., title_size.y),
            metrics,
            LABEL,
        ));
    }
}

fn label_rect(
    viewport: egui::Rect,
    occupied: &[egui::Rect],
    anchor: egui::Pos2,
    size: egui::Vec2,
    gap: f32,
) -> Option<egui::Rect> {
    let bounds = viewport.shrink(7.);
    if size.x > bounds.width() || size.y > bounds.height() {
        return None;
    }
    let offsets = [
        egui::vec2(gap, -size.y - gap),
        egui::vec2(gap, gap),
        egui::vec2(-size.x - gap, -size.y - gap),
        egui::vec2(-size.x - gap, gap),
    ];
    // Put each pin beside its witness when there is room and move subsequent
    // labels vertically, with a leader, when several measurements share it.
    [0., 1., -1., 2., -2., 3., -3., 4., -4., 5., -5.]
        .into_iter()
        .flat_map(|row| offsets.map(|offset| offset + egui::vec2(0., row * (size.y + 5.))))
        .map(|offset| {
            let pos = anchor + offset;
            egui::Rect::from_min_size(
                egui::pos2(
                    pos.x.clamp(bounds.min.x, bounds.max.x - size.x),
                    pos.y.clamp(bounds.min.y, bounds.max.y - size.y),
                ),
                size,
            )
        })
        .find(|card| {
            occupied
                .iter()
                .all(|other| !card.expand(4.).intersects(*other))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_pins_can_share_a_witness_without_covering_each_other() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460., 380.));
        for anchor in [viewport.center(), viewport.min + egui::vec2(3., 3.)] {
            let mut occupied = Vec::new();
            for _ in 0..6 {
                let card = label_rect(viewport, &occupied, anchor, egui::vec2(120., 38.), 27.)
                    .expect("six nearby labels fit the viewport");
                assert!(viewport.contains_rect(card));
                assert!(occupied.iter().all(|other| !card.intersects(*other)));
                occupied.push(card);
            }
        }
    }
}
