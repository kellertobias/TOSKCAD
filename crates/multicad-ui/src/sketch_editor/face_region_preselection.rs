//! Selection-first Model interaction for a finished sketch on a finite face.
use super::{Document, EntityId, Pos2, Rect, Session, SketchEditor};
use crate::viewport::Scene;
use multicad_core::sketch_regions::supporting_face;

impl SketchEditor {
    pub(super) fn profile_pointer_allowed(&self, document: &Document, sketch: EntityId) -> bool {
        (self.editing
            || self.acquiring_profiles()
            || (supporting_face(document, sketch).is_some()
                || self.developed_viewport.is_some()
                    && super::developed_sheet::chart(document, sketch).is_some())
                && self.overlay_visible(sketch))
            && self.tool.is_none()
            && !self.moving
            && !self.picking
            && !self.measurement.active()
            && (!self.has_pending_edits() || self.acquiring_profiles())
    }

    pub(super) fn profile_fill_point(
        &self,
        session: &Session,
        rect: Rect,
        pointer: Pos2,
    ) -> Option<(EntityId, super::Frame, [f64; 2])> {
        let sketch = self.selected?;
        let presentation = self.sketch_evaluation.current(sketch)?;
        if !presentation
            .evaluated
            .is_current(session.document(), session.revision())
        {
            return None;
        }
        let frame = self.display_frame(sketch, presentation.evaluated.frame);
        let point = self.camera.on_plane(pointer, rect, frame)?;
        // Cached display fills only decide which input lane owns this click.
        // The sketch worker classifies the exact region and its boundaries.
        presentation
            .fills
            .iter()
            .any(|fill| fill.contains(point))
            .then_some((sketch, frame, point))
    }

    pub(super) fn face_region_point(
        &self,
        session: &Session,
        scene: &Scene,
        rect: Rect,
        pointer: Pos2,
    ) -> Option<(EntityId, [f64; 2])> {
        if scene.stamp().document != session.document().id()
            || scene.stamp().revision != session.revision()
        {
            return None;
        }
        let (sketch, frame, point) = self.profile_fill_point(session, rect, pointer)?;
        // Do not select a profile through foreground geometry or through an
        // opening in the supporting face.
        let ray = self.camera.ray(pointer, rect)?;
        let hit = scene.pick_face_section(ray, self.modeling_section())?;
        let point_world = frame.point(point);
        let distance: f64 = point_world
            .into_iter()
            .zip(ray.origin())
            .zip(ray.direction())
            .map(|((point, origin), axis)| (point - origin) * axis)
            .sum();
        ((hit.distance_mm - distance).abs() <= super::face_profiles::COPLANAR_DISTANCE_MM)
            .then_some((sketch, point))
    }
}
