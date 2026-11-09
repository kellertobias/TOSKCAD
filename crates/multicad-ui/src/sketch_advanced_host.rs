//! Advanced tools share current sketch evaluation and native source acquisition.
use super::*;
use crate::{
    modeling::{AdvancedKind, AdvancedSelection},
    viewport::{PickFilter, PickOptions, Scene, SceneFeed, Selection},
};
use multicad_core::{
    model::{ProfileSource, SketchPathSource},
    project_file::ProjectResources,
    references::{self, GeometryIntent, Resolution, TopologyKind},
};
use std::sync::Arc;

impl SketchEditor {
    pub(super) fn refresh_advanced_sketch_context(
        &mut self,
        session: &Session,
    ) -> Result<(), String> {
        let batch = (
            self.sketch_evaluation.current_stamp(),
            self.sketch_evaluation.current_presentations().len(),
        );
        if self.advanced_context_stamp == Some(batch) {
            return Ok(());
        }
        if batch.1 == 0 {
            self.model
                .set_advanced_sketch_context(session, vec![], vec![])?;
            self.advanced_context_stamp = Some(batch);
            return Ok(());
        }
        let document = session.document();
        let hash = session.source_hash()?;
        let mut profiles = Vec::new();
        let mut paths = Vec::new();
        for presentation in self.sketch_evaluation.current_presentations() {
            let stamp = presentation.stamp;
            if Some(stamp) != batch.0
                || stamp.document != document.id()
                || stamp.revision != session.revision()
                || stamp.source_hash != hash
                || presentation.evaluated.solved().is_none()
            {
                continue;
            }
            let sketch = presentation.evaluated.sketch;
            profiles.extend(
                presentation
                    .regions
                    .regions
                    .iter()
                    .map(|region| ProfileSource {
                        sketch,
                        region: region.intent(),
                    }),
            );
            paths.extend(
                presentation
                    .evaluated
                    .canonical()
                    .sketch
                    .curves
                    .iter()
                    .filter(|curve| {
                        !curve.construction
                            && matches!(
                                curve.kind,
                                multicad_core::constraint_sketch::CurveKind::Line { .. }
                            )
                    })
                    .map(|curve| SketchPathSource {
                        sketch,
                        curves: vec![curve.id],
                    }),
            );
        }
        self.model
            .set_advanced_sketch_context(session, profiles, paths)?;
        self.advanced_context_stamp = Some(batch);
        Ok(())
    }

    pub fn begin_advanced_model(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        kind: AdvancedKind,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.modeling_available() {
            return Err("Apply or cancel pending edits before starting another tool".into());
        }
        let body = (self.tree_selection.len() == 1)
            .then(|| *self.tree_selection.first().expect("one selected source"))
            .filter(|id| {
                matches!(
                    session.document().get(*id).map(|e| &e.data),
                    Some(EntityData::Part { .. })
                )
            });
        let profile = self
            .selected_profiles(session.document(), session.revision())
            .ok()
            .and_then(|(sketch, regions)| {
                (regions.len() == 1).then(|| ProfileSource {
                    sketch,
                    region: regions[0].clone(),
                })
            });
        let captures = self
            .solid
            .selected_hit()
            .filter(|_| {
                matches!(
                    kind,
                    AdvancedKind::Shell | AdvancedKind::Thickness | AdvancedKind::Draft
                )
            })
            .filter(|hit| Some(hit.instance) == body && hit.key.kind == TopologyKind::Face)
            .and_then(|hit| hit.intent.clone())
            .into_iter()
            .collect();
        let scene = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()));
        self.finish()?;
        self.refresh_advanced_sketch_context(session)?;
        self.model.begin_advanced(
            session,
            resources,
            scene.map(AsRef::as_ref),
            kind,
            AdvancedSelection {
                body,
                profile,
                captures,
            },
            channel,
        )?;
        self.message = "Choose the explicit source geometry and controls in the Inspector. Apply after the preview is ready".into();
        Ok(())
    }

    pub(super) fn advanced_source_scene(
        &self,
        session: &Session,
        feed: &SceneFeed,
    ) -> Result<Arc<Scene>, String> {
        let input = self
            .model
            .advanced_input()
            .ok_or("Choose a source Part first")?;
        let scene = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
            .ok_or("Wait for current native geometry before selecting faces")?;
        if !scene.is_full_model()
            || scene
                .model_evaluation()
                .is_none_or(|info| session.source_hash().ok() != Some(info.source_hash))
        {
            return Err("Face selection requires the complete source-bound native model".into());
        }
        scene.isolated_source(input.producer)
    }

    pub(super) fn advanced_viewport(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        rect: Rect,
    ) {
        if !self.model.advanced_picking() {
            return;
        }
        let Ok(scene) = self.advanced_source_scene(session, feed) else {
            return;
        };
        let Some(input) = self.model.advanced_input().cloned() else {
            return;
        };
        let snapshot = scene.geometry_snapshot(input.producer);
        let hovered = response.hover_pos().and_then(|pointer| {
            scene.pick_screen_with(
                self.camera,
                rect,
                pointer,
                PickOptions {
                    filter: PickFilter::Faces,
                    include_hidden: false,
                    section: self.solid.section.plane(),
                },
            )
        });
        if response.clicked() {
            if let Some(hit) = &hovered {
                if let Some(capture) = &hit.intent {
                    let current = self.model.advanced_captures().unwrap_or_default();
                    let same = current.iter().position(|intent| {
                        matches!(snapshot.as_ref().map(|s| references::resolve(intent,s,session.revision())), Some(Resolution::Resolved {key,..}) if key==hit.key)
                    });
                    let modifiers = crate::selection_input::click_modifiers(ui, response);
                    let captures = if modifiers.shift || modifiers.command {
                        let mut captures = current.to_vec();
                        if let Some(index) = same {
                            captures.remove(index);
                        } else {
                            captures.push(capture.clone());
                        }
                        captures
                    } else {
                        vec![same.map_or_else(|| capture.clone(), |i| current[i].clone())]
                    };
                    if let Err(error) = self.model.set_advanced_captures(
                        session,
                        resources,
                        feed.visible().expect("current canonical source"),
                        captures,
                    ) {
                        self.message = error;
                    }
                } else {
                    self.message = "This face has no persistent native geometry name".into();
                }
            } else {
                self.message = "Choose a visible face of the upstream Part".into();
            }
        }
        let highlight = hovered.as_ref().map(|hit| hit.key).or_else(|| {
            self.model
                .advanced_captures()?
                .last()
                .and_then(|capture: &GeometryIntent| {
                    match references::resolve(capture, snapshot.as_ref()?, session.revision()) {
                        Resolution::Resolved { key, .. } => Some(key),
                        _ => None,
                    }
                })
        });
        self.solid
            .highlight_reference(highlight.map(|key| Selection::Face {
                instance: input.producer,
                ordinal: key.ordinal,
            }));
    }
}
