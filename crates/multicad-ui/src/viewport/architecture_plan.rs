//! Detached native plan graphics, derived on the geometry worker, never CAD source.
use super::Scene;
use crate::{camera::Camera, egui};
use multicad_core::{
    document::EntityId,
    drawings::{DrawingVector, VectorRole},
    project_file::ContentHash,
};
use std::{collections::BTreeSet, sync::Arc};

pub struct ArchitecturePlan {
    pub source_hash: ContentHash,
    pub sources: BTreeSet<EntityId>,
    pub vectors: Vec<DrawingVector>,
}
impl ArchitecturePlan {
    pub(super) fn validate(
        &self,
        source: ContentHash,
        roots: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        if self.source_hash != source || !self.sources.is_subset(roots) {
            return Err("Native plan belongs to another source or occurrence set".into());
        }
        if self.vectors.iter().any(|vector| {
            !self.sources.contains(&vector.source)
                || !vector.line_width_mm.is_finite()
                || vector.line_width_mm <= 0.
                || vector.preview_mm.len() < 2
                || vector
                    .preview_mm
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite())
        }) {
            return Err("Native plan contains invalid or unattributed graphics".into());
        }
        Ok(())
    }
    /// The original scene remains the hit/reference scene. These sources get
    /// their native section/plan graphics instead of an occluding wall top.
    pub fn surface_scene(&self, scene: &Arc<Scene>) -> Result<Arc<Scene>, String> {
        let visible: BTreeSet<_> = scene
            .instances()
            .iter()
            .filter(|instance| !self.sources.contains(&instance.id))
            .map(|instance| instance.id)
            .collect();
        if visible.len() == scene.instances().len() {
            Ok(scene.clone())
        } else {
            scene.visible_subset(&visible)
        }
    }
    pub fn paint(
        &self,
        painter: &egui::Painter,
        scene: &Scene,
        camera: Camera,
        rect: egui::Rect,
        selected: &BTreeSet<EntityId>,
        hovered: Option<EntityId>,
    ) {
        let visible: BTreeSet<_> = scene
            .instances()
            .iter()
            .map(|instance| instance.id)
            .collect();
        for vector in &self.vectors {
            if !visible.contains(&vector.source) {
                continue;
            }
            let points: Vec<_> = vector
                .preview_mm
                .iter()
                .map(|point| {
                    let pixel =
                        camera.project([point[0] as f32, point[1] as f32, camera.target[2]], rect);
                    egui::pos2(pixel[0], pixel[1])
                })
                .collect();
            let highlighted = selected.contains(&vector.source) || hovered == Some(vector.source);
            let color = if highlighted {
                egui::Color32::from_rgb(90, 170, 255)
            } else {
                match vector.role {
                    VectorRole::Section => egui::Color32::from_gray(235),
                    VectorRole::Hatch => egui::Color32::from_gray(155),
                    VectorRole::Visible => egui::Color32::from_gray(225),
                    VectorRole::Hidden => egui::Color32::from_gray(150),
                }
            };
            let stroke =
                egui::Stroke::new((vector.line_width_mm as f32 * 4.).clamp(0.5, 2.), color);
            if vector.role == VectorRole::Hidden {
                painter.extend(egui::Shape::dashed_line(&points, stroke, 5., 3.));
            } else {
                painter.add(egui::Shape::line(points, stroke));
            }
        }
    }
}
