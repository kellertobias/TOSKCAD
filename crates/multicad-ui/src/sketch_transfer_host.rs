//! Source feature derivation stays inside the shared native command host.
use super::*;
use crate::{
    transfer::{TransferCaptureKind, TransferOutcome, TransferPick, TransferSelection},
    viewport::{PickFilter, PickOptions, Scene, SceneFeed, Selection},
};
use multicad_core::{
    model::OccurrencePath,
    project_file::ProjectResources,
    references::{AnalyticGeometry, GeometryIntent, TopologyKind},
};
use std::{collections::BTreeSet, sync::Arc};

pub(super) struct TransferSourceView {
    canonical: Arc<Scene>,
    pick: TransferPick,
    view: Arc<Scene>,
    camera: Camera,
}
impl SketchEditor {
    pub fn set_transfer_backend(&mut self, backend: Box<dyn crate::viewport::SceneBackend>) {
        self.transfer.set_backend(backend);
    }
    pub fn has_transfer_backend(&self) -> bool {
        self.transfer.has_backend()
    }
    pub fn begin_transfer_tool(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.modeling_available() || self.measurement.active() {
            return Err("Apply or cancel the active tool before transferring features".into());
        }
        let document = session.document();
        let ids: BTreeSet<_> = self
            .tree_selection
            .iter()
            .copied()
            .chain(self.solid.selected_ids())
            .collect();
        let scene = feed
            .visible()
            .filter(|_| feed.is_current(document.id(), session.revision()));
        let paths: BTreeSet<OccurrencePath> = scene
            .and_then(|scene| scene.assembly_evaluation())
            .into_iter()
            .flat_map(|receipt| &receipt.leaves)
            .filter(|leaf| ids.contains(&leaf.path.root) || ids.contains(&leaf.component))
            .map(|leaf| leaf.path.clone())
            .collect();
        let feature = ids
            .iter()
            .copied()
            .find(|id| crate::transfer::TransferController::editable(document, *id));
        let source =
            (ids.len() == 1 && paths.len() == 1).then(|| paths.first().expect("one path").clone());
        let targets = if ids.len() > 1 {
            paths.into_iter().collect()
        } else {
            vec![]
        };
        self.finish()?;
        self.transfer.begin(
            session,
            resources,
            scene,
            TransferSelection {
                source,
                targets,
                feature,
            },
            channel,
        )?;
        self.transfer_source_view = None;
        self.message =
            "Transfer features · choose source geometry and receiving faces in the Inspector"
                .into();
        Ok(())
    }
    pub(super) fn begin_transfer_edit(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        feature: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.modeling_available() || self.measurement.active() {
            return Err("Apply or cancel pending edits before editing a transfer".into());
        }
        self.finish()?;
        let scene = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()));
        self.transfer
            .begin_existing(session, resources, scene, feature, channel)?;
        self.transfer_source_view = None;
        Ok(())
    }
    pub(super) fn accept_transfer_outcome(&mut self, outcome: TransferOutcome) {
        self.tree_scene_stamp = None;
        self.solid.clear();
        match outcome {
            TransferOutcome::Applied { selected, label } => {
                self.tree_selection = selected.into_iter().collect();
                self.selected = None;
                self.message = format!("{label} applied · one undoable source edit");
            }
            TransferOutcome::Cancelled => {
                self.message = "Transfer tool closed · document unchanged".into()
            }
        }
    }
    pub(super) fn transfer_pick_scene(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Result<Option<Arc<Scene>>, String> {
        let Some(pick) = self.transfer.picker() else {
            if let Some(previous) = self.transfer_source_view.take() {
                self.camera = previous.camera;
                self.fit_requested = false;
            }
            return Ok(None);
        };
        let inspected = self.transfer.capture_scene(session, resources);
        let Some(canonical) = inspected.as_ref() else {
            return Ok(None);
        };
        if let Some(previous) = &self.transfer_source_view
            && previous.pick == pick
            && Arc::ptr_eq(&previous.canonical, canonical)
        {
            return Ok(Some(previous.view.clone()));
        }
        // A receiving producer is frozen before the machining operation. Its
        // current treated Part head must never replace that explicit source.
        let view = canonical.isolated_source(pick.producer)?;
        self.transfer_source_view = Some(TransferSourceView {
            canonical: canonical.clone(),
            pick,
            view: view.clone(),
            camera: self
                .transfer_source_view
                .as_ref()
                .map_or(self.camera, |previous| previous.camera),
        });
        self.fit_requested = true;
        Ok(Some(view))
    }
    pub(super) fn transfer_viewport(
        &mut self,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        rect: Rect,
    ) {
        let Some(pick) = self.transfer.picker() else {
            return;
        };
        let source = match self.transfer_pick_scene(session, resources) {
            Ok(Some(view)) => view,
            _ => return,
        };
        let Some(canonical) = self
            .transfer_source_view
            .as_ref()
            .map(|view| view.canonical.clone())
        else {
            return;
        };
        let Some(topology) = canonical.geometry_snapshot(pick.producer) else {
            return;
        };
        let eligible: BTreeSet<_> = topology
            .elements
            .iter()
            .filter(|element| match pick.kind {
                TransferCaptureKind::PlanarFace => {
                    matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
                }
                TransferCaptureKind::CircularFeature => match element.geometry {
                    Some(AnalyticGeometry::CircularArc { sweep_rad, .. }) => {
                        (sweep_rad.abs() - std::f64::consts::TAU).abs() < 1e-9
                    }
                    None => element.key.kind == TopologyKind::Face,
                    _ => false,
                },
            })
            .map(|element| element.key)
            .collect();
        let hit = response.hover_pos().and_then(|pointer| {
            let filters: &[PickFilter] = match pick.kind {
                TransferCaptureKind::CircularFeature => &[PickFilter::Edges, PickFilter::Faces],
                TransferCaptureKind::PlanarFace => &[PickFilter::Faces],
            };
            filters.iter().find_map(|filter| {
                source.pick_screen_matching(
                    self.camera,
                    rect,
                    pointer,
                    PickOptions {
                        filter: *filter,
                        include_hidden: false,
                        section: None,
                    },
                    |instance, key| instance == pick.producer && eligible.contains(&key),
                )
            })
        });
        self.solid
            .highlight_reference(hit.as_ref().map(|hit| match hit.key.kind {
                TopologyKind::Face => Selection::Face {
                    instance: hit.instance,
                    ordinal: hit.key.ordinal,
                },
                TopologyKind::Edge => Selection::Edge {
                    instance: hit.instance,
                    ordinal: hit.key.ordinal,
                },
                TopologyKind::Vertex => Selection::Vertex {
                    instance: hit.instance,
                    ordinal: hit.key.ordinal,
                },
            }));
        if !response.clicked() {
            return;
        }
        let Some(hit) = hit else {
            self.message = match pick.kind {
                TransferCaptureKind::CircularFeature => "Choose a complete circular edge or cylindrical face; Preview checks its exact axis".into(),
                TransferCaptureKind::PlanarFace => "Choose a planar face on this explicit source producer".into(),
            };
            return;
        };
        match GeometryIntent::capture(&topology, hit.key).and_then(|intent| {
            self.transfer
                .set_capture(session, resources, &canonical, pick.lane, intent)
        }) {
            Ok(()) => {
                self.message =
                    "Transfer reference selected · native Preview validates the complete result"
                        .into()
            }
            Err(error) => self.message = error,
        }
    }
}
