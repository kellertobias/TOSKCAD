//! A current native scene owns picks. A candidate scene never supplies face intent.
use super::*;
use crate::viewport::{PickFilter, PickOptions, Scene, SceneFeed, Selection};
use multicad_core::project_file::ProjectResources;
use std::sync::Arc;

impl SketchEditor {
    pub(super) fn begin_surface_structure(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if self.architecture.planar() {
            return Err("Switch to the 3D model to choose physical faces".into());
        }
        let scene = surface_canonical(session, feed)?;
        let hits = self.solid.selected_faces();
        self.finish()?;
        self.model
            .begin_surface_structure(session, resources, scene, &hits, channel)?;
        self.message =
            "Surface Structure · choose planar faces and physical relief, then Preview".into();
        Ok(())
    }

    pub(super) fn surface_structure_pick_scene(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
    ) -> Result<Option<Arc<Scene>>, String> {
        let source = if let Ok(canonical) = surface_canonical(session, feed) {
            canonical.clone()
        } else {
            let Some(source) = self.model.inspect_surface_source(session, resources)? else {
                return Ok(None);
            };
            source
        };
        match self.model.surface_structure_input() {
            Some(input) => source.isolated_source(input.producer).map(Some),
            None => Ok(Some(source)),
        }
    }

    pub(super) fn surface_structure_viewport(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        displayed: Option<&Scene>,
    ) {
        if !self.model.surface_structure_picking() || self.measurement.active() {
            return;
        }
        let Some(canonical) = surface_canonical(session, feed)
            .ok()
            .or_else(|| self.model.inspected_surface_scene())
            .cloned()
        else {
            return;
        };
        let Some(displayed) = displayed.filter(|scene| scene.stamp() == canonical.stamp()) else {
            return;
        };
        let hovered = response.hover_pos().and_then(|point| {
            displayed.pick_screen_with(
                self.camera,
                response.rect,
                point,
                PickOptions {
                    filter: PickFilter::Faces,
                    include_hidden: false,
                    section: self.modeling_section(),
                },
            )
        });
        if response.clicked() {
            if let Some(hit) = &hovered {
                let modifiers = crate::selection_input::click_modifiers(ui, response);
                if let Err(error) = self.model.pick_surface_structure_face(
                    session,
                    resources,
                    &canonical,
                    hit,
                    modifiers.shift || modifiers.command,
                ) {
                    self.message = error;
                }
            } else {
                self.message = "Click a visible planar face to apply surface relief".into();
            }
        }
        self.solid
            .highlight_reference(hovered.map(|hit| Selection::Face {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            }));
    }

    pub(crate) fn take_surface_height_map_request(&mut self) -> Option<crate::modeling::FormStamp> {
        self.model.take_surface_height_map_request()
    }

    pub(crate) fn surface_structure_form_matches(&self, stamp: crate::modeling::FormStamp) -> bool {
        self.model.stamp() == Some(stamp)
    }

    pub(crate) fn accept_surface_height_map(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        stamp: crate::modeling::FormStamp,
        pattern: multicad_core::model::SurfaceStructurePattern,
    ) -> Result<(), String> {
        self.model
            .accept_surface_height_map(session, resources, stamp, pattern)
    }
}

fn surface_canonical<'a>(session: &Session, feed: &'a SceneFeed) -> Result<&'a Arc<Scene>, String> {
    feed.visible()
        .filter(|scene| {
            feed.is_current(session.document().id(), session.revision()) && scene.is_full_model()
        })
        .ok_or_else(|| {
            "Wait for the current complete native model before choosing planar faces".into()
        })
}
