use super::*;
use multicad_core::references::AnalyticGeometry;
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
        if self.active
            && let Some(v) = self.ready(session)
        {
            paint_value(painter, rect, camera, v, [0.; 3], "", section, true);
        }
        if let Some(batch) = &self.feed.batch {
            for pin in &self.feed.pins {
                if pin.visible
                    && let Some(Ok(value)) = batch.pinned.get(&pin.id).map(|m| &m.result)
                {
                    paint_value(
                        painter,
                        rect,
                        camera,
                        value,
                        pin.label_offset_mm,
                        &pin.name,
                        section,
                        false,
                    );
                }
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn paint_value(
    painter: &egui::Painter,
    rect: egui::Rect,
    camera: Camera,
    v: &MeasurementValues,
    offset: [f64; 3],
    name: &str,
    section: Option<SectionPlane>,
    active: bool,
) {
    let Some(w) = v.witnesses.first() else {
        return;
    };
    let segment = section.map_or(Some([w.a_mm, w.b_mm]), |p| p.clip_segment(w.a_mm, w.b_mm));
    let Some([a, b]) = segment else {
        return;
    };
    let project = |p: [f64; 3]| {
        let p = camera.project(p.map(|v| v as f32), rect);
        egui::pos2(p[0], p[1])
    };
    let a = project(a);
    let b = project(b);
    painter.line_segment(
        [a, b],
        egui::Stroke::new(
            if active { 1.8 } else { 1. },
            egui::Color32::from_rgb(140, 190, 220),
        ),
    );
    for (index, p) in [a, b].into_iter().enumerate() {
        painter.circle_filled(p, 3., color(index));
    }
    let label = std::array::from_fn(|i| (w.a_mm[i] + w.b_mm[i]) * 0.5 + offset[i]);
    if section.is_some_and(|p| !p.contains(label)) {
        return;
    }
    let position = project(label) + egui::vec2(6., -7.);
    let text = if name.is_empty() {
        format!("{:.3} mm", v.minimum_mm)
    } else {
        format!("{name} · {:.3} mm", v.minimum_mm)
    };
    painter.text(
        position,
        egui::Align2::LEFT_BOTTOM,
        text,
        egui::FontId::proportional(12.),
        egui::Color32::from_rgb(205, 225, 240),
    );
}
