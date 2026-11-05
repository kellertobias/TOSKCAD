//! Assembly tools use the same document, native scene and fixed Inspector lane.
use super::*;
use crate::{
    assembly::{AnchorSide, AssemblyOutcome, AssemblySelection, AssemblyTool},
    viewport::{PickFilter, PickOptions, Scene, SceneFeed, Selection},
};
use std::{collections::BTreeMap, sync::Arc};

pub(super) struct PresentationCache {
    source: Arc<Scene>,
    offsets: BTreeMap<OccurrencePath, [f64; 3]>,
    view: Arc<Scene>,
}
pub(super) struct AnchorView {
    canonical: Arc<Scene>,
    instance: EntityId,
    view: Arc<Scene>,
    camera: Camera,
}
use multicad_core::{
    model::{AlignmentInput, OccurrencePath},
    project_file::{ContentHash, ProjectResources},
    references::{AnchoredFrame, Attachment, GeometryIntent, TopologyKind},
};

pub(super) struct PendingAnchor {
    side: AnchorSide,
    instance: EntityId,
    input: AlignmentInput,
    frames: Vec<AnchoredFrame>,
    document: EntityId,
    revision: u64,
    source: ContentHash,
}
impl SketchEditor {
    /// Definition members can be reused at several outer placements. Pick their
    /// actual native component mesh in its authored placement, explicitly scoped
    /// as source inspection, so an outer aggregate cannot masquerade as a member.
    pub(super) fn assembly_anchor_scene(
        &mut self,
        session: &Session,
        feed: &SceneFeed,
    ) -> Result<Option<Arc<Scene>>, String> {
        let Some(pick) = self.assembly.anchor_pick() else {
            if let Some(previous) = self.assembly_pick_view.take() {
                self.camera = previous.camera;
                self.fit_requested = false;
            }
            return Ok(None);
        };
        let canonical = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
            .ok_or("Wait for current native geometry before picking an anchor")?;
        if !canonical.is_full_model() {
            return Err("Anchor picking requires a current complete native model".into());
        }
        if let Some(cache) = &self.assembly_pick_view
            && cache.instance == pick.instance
            && Arc::ptr_eq(&cache.canonical, canonical)
        {
            return Ok(Some(cache.view.clone()));
        }
        let Some(EntityData::Instance { component, .. }) =
            session.document().get(pick.instance).map(|e| &e.data)
        else {
            return Err("Choose a placed component for this anchor".into());
        };
        let isolated = canonical.isolated_source(*component)?;
        let producer = isolated.instances()[0].geometry;
        let geometry = isolated
            .geometries()
            .get(&producer)
            .ok_or("Current component mesh is unavailable")?
            .clone();
        let pose = multicad_core::model::resolve_instance_pose_with_snapshots(
            session.document(),
            pick.instance,
            session.parameters(),
            session.revision(),
            &canonical.reference_snapshots(),
        )
        .map_err(|e| e.to_string())?;
        let view = Arc::new(
            Scene::new(
                canonical.stamp(),
                vec![geometry],
                vec![crate::viewport::Instance {
                    id: pick.instance,
                    geometry: producer,
                    transform: crate::viewport::Transform::from_rigid_pose(pose)?,
                    color: [145, 173, 195, 255],
                }],
            )?
            .with_source_scope(vec![*component])?,
        );
        self.assembly_pick_view = Some(AnchorView {
            camera: self
                .assembly_pick_view
                .as_ref()
                .map_or(self.camera, |previous| previous.camera),
            canonical: canonical.clone(),
            instance: pick.instance,
            view: view.clone(),
        });
        self.fit_requested = true;
        Ok(Some(view))
    }
    pub(super) fn assembly_exploded_scene(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        canonical: &Arc<Scene>,
    ) -> Result<Option<Arc<Scene>>, String> {
        let Some(offsets) = self.assembly.display_leaf_offsets(session, resources) else {
            self.assembly_presentation = None;
            return Ok(None);
        };
        if let Some(cache) = &self.assembly_presentation
            && Arc::ptr_eq(&cache.source, canonical)
            && cache.offsets == offsets
        {
            return Ok(Some(cache.view.clone()));
        }
        let resolver = multicad_core::materials::Resolver::new(session.document())
            .map_err(|e| e.to_string())?;
        let colors = canonical
            .assembly_evaluation()
            .into_iter()
            .flat_map(|receipt| &receipt.leaves)
            .map(|leaf| {
                resolver
                    .resolve(leaf.component)
                    .map(|p| (leaf.component, p.appearance.color_rgba))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|e| e.to_string())?;
        let view = canonical.presentation_leaf_offsets(&offsets, &colors)?;
        self.assembly_presentation = Some(PresentationCache {
            source: canonical.clone(),
            offsets,
            view: view.clone(),
        });
        Ok(Some(view))
    }
    /// Paint the selected clearance result's witnesses, labeled distance and
    /// occurrence outlines; an explicit Fit frames them by moving the camera.
    pub(super) fn assembly_clearance_paint(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        session: &Session,
        scene: Option<&Arc<Scene>>,
    ) {
        let marker = scene.and_then(|scene| self.assembly.clearance_marker(session, scene));
        let fit = self.assembly.take_clearance_fit();
        let Some(marker) = marker else {
            return;
        };
        if fit {
            self.camera.fit(&marker.fit_points(), rect);
            painter.ctx().request_repaint();
        }
        marker.paint(
            painter,
            rect,
            self.camera,
            self.assembly.clearance_reveal(),
            self.solid.section.plane(),
        );
    }
    pub fn set_assembly_backend(&mut self, backend: Box<dyn crate::viewport::SceneBackend>) {
        self.assembly.set_backend(backend);
    }
    pub fn has_assembly_backend(&self) -> bool {
        self.assembly.has_backend()
    }
    pub fn set_assembly_query_factory(
        &mut self,
        factory: crate::assembly::AssemblyInspectionBackendFactory,
    ) {
        self.assembly.set_query_factory(factory);
    }
    pub fn has_assembly_query_factory(&self) -> bool {
        self.assembly.has_query_factory()
    }
    pub(super) fn assembly_edit_actions(
        &self,
        document: &Document,
    ) -> Vec<(EntityId, &'static str)> {
        if self.tree_selection.len() != 1 {
            return vec![];
        }
        let id = *self.tree_selection.first().expect("one selected entry");
        let mut actions = Vec::new();
        let component = match document.get(id).map(|e| &e.data) {
            Some(EntityData::Instance { component, .. }) => {
                if let Ok(Some(joint)) = multicad_core::model::joint_driver(document, id) {
                    actions.push((joint, "Edit relationship"));
                }
                *component
            }
            Some(EntityData::Assembly { .. }) => id,
            _ => return actions,
        };
        if matches!(
            document.get(component).map(|e| &e.data),
            Some(EntityData::Assembly { .. })
        ) && multicad_core::symbols::owned_entities(document)
            .is_ok_and(|owned| !owned.contains(&component))
            && document.entities().any(|e| {
                multicad_core::model::read_assembly(document, e.id)
                    .is_ok_and(|source| source.assembly == component)
            })
        {
            actions.push((component, "Edit assembly definition"));
        }
        actions
    }
    fn begin_assembly_edit(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        entity: EntityId,
    ) -> Result<(), String> {
        if !self.modeling_available() {
            return Err("Apply or cancel pending edits before editing the assembly".into());
        }
        self.finish()?;
        let scene = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()));
        self.assembly
            .begin_existing(session, resources, scene, entity, Channel::Ui)?;
        self.assembly_anchor = None;
        Ok(())
    }
    pub(super) fn assembly_edit_controls(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
    ) {
        for (entity, label) in self.assembly_edit_actions(session.document()) {
            if ui.add_enabled(self.modeling_available(),egui::Button::new(label))
                .on_hover_text("Reopen the stored controls; a matching native Preview is required before Apply").clicked()
                && let Err(error)=self.begin_assembly_edit(session,resources,feed,entity){self.message=error;}
        }
    }
    pub fn begin_assembly_tool(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        tool: AssemblyTool,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.modeling_available() || self.measurement.active() {
            return Err(
                "Apply or cancel the active tool before starting an assembly command".into(),
            );
        }
        let document = session.document();
        let selected: std::collections::BTreeSet<_> = self
            .tree_selection
            .iter()
            .copied()
            .chain(self.solid.selected_ids())
            .collect();
        let instances: Vec<_> = selected
            .iter()
            .copied()
            .filter(|id| {
                matches!(
                    document.get(*id).map(|e| &e.data),
                    Some(EntityData::Instance { .. })
                )
            })
            .collect();
        let component = selected
            .iter()
            .find_map(|id| match document.get(*id).map(|e| &e.data) {
                Some(EntityData::Part { .. } | EntityData::Assembly { .. }) => Some(*id),
                Some(EntityData::Instance { component, .. }) => Some(*component),
                _ => None,
            });
        let joint = instances.first().and_then(|id| {
            multicad_core::model::joint_driver(document, *id)
                .ok()
                .flatten()
        });
        let pose = selected
            .iter()
            .copied()
            .find(|id| multicad_core::model::read_named_pose(document, *id).is_ok());
        let occurrences = selected
            .iter()
            .filter(|id| {
                matches!(
                    document.get(**id).map(|e| &e.data),
                    Some(EntityData::Part { .. } | EntityData::Instance { .. })
                )
            })
            .map(|id| OccurrencePath {
                root: *id,
                members: vec![],
            })
            .collect();
        let scene = feed
            .visible()
            .filter(|_| feed.is_current(document.id(), session.revision()));
        self.finish()?;
        self.assembly.begin(
            tool,
            session,
            resources,
            scene,
            AssemblySelection {
                component,
                instances,
                joint,
                pose,
                occurrences,
            },
            channel,
        )?;
        self.assembly_anchor = None;
        self.message = format!("{} · choose inputs in the Inspector", tool.label());
        Ok(())
    }
    pub(super) fn accept_assembly_outcome(&mut self, outcome: AssemblyOutcome) {
        self.assembly_anchor = None;
        self.tree_scene_stamp = None;
        self.assembly_presentation = None;
        self.solid.clear();
        match outcome {
            AssemblyOutcome::Applied { selected, label } => {
                self.tree_selection = selected.into_iter().collect();
                self.selected = None;
                self.message = format!("{label} applied · one undoable source edit");
            }
            AssemblyOutcome::Cancelled => {
                self.message = "Assembly tool closed · document unchanged".into()
            }
        }
    }
    pub(super) fn assembly_anchor_controls(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
    ) {
        let valid = self.assembly_anchor.as_ref().is_some_and(|pending| {
            pending.document == session.document().id()
                && pending.revision == session.revision()
                && ContentHash::document(session.document()).ok() == Some(pending.source)
                && self.assembly.anchor_pick().is_some_and(|pick| {
                    pick.instance == pending.instance && pick.side == pending.side
                })
        });
        if !valid {
            self.assembly_anchor = None;
            return;
        }
        let pending = self
            .assembly_anchor
            .as_ref()
            .expect("checked anchor choice");
        ui.strong("Choose a face reference frame");
        ui.small("This face has multiple authored origins. Choose one explicitly in component coordinates.");
        let mut chosen = None;
        for (index, frame) in pending.frames.iter().enumerate() {
            let [x, y, z] = frame.frame.origin_mm;
            if ui
                .button(format!("Frame {} · ({x:.2}, {y:.2}, {z:.2}) mm", index + 1))
                .on_hover_text(format!("Authored frame: {}", frame.origin.role))
                .clicked()
            {
                chosen = Some(index);
            }
        }
        if let Some(index) = chosen {
            let pending = self.assembly_anchor.take().expect("shown anchor choice");
            let mut input = pending.input;
            input.intent.attachment = Some(Attachment {
                anchor: pending.frames[index].origin.clone(),
                offset_mm: [0.; 3],
                angle_rad: 0.,
            });
            if let Some(scene) = feed
                .visible()
                .filter(|_| feed.is_current(session.document().id(), session.revision()))
                && let Err(error) =
                    self.assembly
                        .set_anchor_input(session, resources, scene, pending.side, input)
            {
                self.message = error;
            }
        }
    }
    pub(super) fn assembly_viewport(
        &mut self,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        rect: Rect,
    ) {
        let Some(pick) = self.assembly.anchor_pick() else {
            self.assembly_anchor = None;
            let displayed = self.assembly.display_scene(session, resources).or_else(|| {
                feed.visible()
                    .filter(|_| feed.is_current(session.document().id(), session.revision()))
                    .cloned()
            });
            if let Some(marker) = displayed
                .as_ref()
                .and_then(|scene| self.assembly.clearance_marker(session, scene))
            {
                self.solid.highlight_reference(Some(marker.highlight()));
            }
            return;
        };
        let Some(scene) = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
        else {
            return;
        };
        if !scene.is_full_model() {
            return;
        }
        let source_scene = match self.assembly_anchor_scene(session, feed) {
            Ok(Some(scene)) => scene,
            _ => return,
        };
        let hovered = response
            .hover_pos()
            .and_then(|pointer| {
                source_scene.pick_screen_with(
                    self.camera,
                    rect,
                    pointer,
                    PickOptions {
                        filter: PickFilter::Faces,
                        include_hidden: false,
                        section: self.solid.section.plane(),
                    },
                )
            })
            .filter(|hit| hit.instance == pick.instance && hit.key.kind == TopologyKind::Face);
        self.solid
            .highlight_reference(hovered.as_ref().map(|hit| Selection::Face {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            }));
        if !response.clicked() {
            return;
        }
        let Some(hit) = hovered else {
            self.message =
                "Choose a visible planar face on the selected relationship Instance".into();
            return;
        };
        let Some(EntityData::Instance { component, .. }) =
            session.document().get(pick.instance).map(|e| &e.data)
        else {
            return;
        };
        let local = scene
            .geometry_snapshot(*component)
            .or_else(|| scene.topology_snapshot(*component));
        let result = (|| -> Result<PendingAnchor, String> {
            let local =
                local.ok_or("The current component-local native topology is unavailable")?;
            let element = local
                .elements
                .iter()
                .find(|e| e.key == hit.key)
                .ok_or("The selected source face is unavailable")?;
            if element.frames.is_empty() {
                return Err("Choose a planar face with an authored reference frame".into());
            }
            Ok(PendingAnchor {
                side: pick.side,
                instance: pick.instance,
                input: AlignmentInput {
                    source: *component,
                    intent: GeometryIntent::capture(&local, hit.key)?,
                },
                frames: element.frames.clone(),
                document: session.document().id(),
                revision: session.revision(),
                source: ContentHash::document(session.document())?,
            })
        })();
        match result {
            Ok(mut pending) if pending.frames.len() == 1 => {
                pending.input.intent.attachment = Some(Attachment {
                    anchor: pending.frames[0].origin.clone(),
                    offset_mm: [0.; 3],
                    angle_rad: 0.,
                });
                if let Err(error) = self.assembly.set_anchor_input(
                    session,
                    resources,
                    scene,
                    pending.side,
                    pending.input,
                ) {
                    self.message = error;
                }
                self.assembly_anchor = None;
            }
            Ok(pending) => self.assembly_anchor = Some(pending),
            Err(error) => self.message = error,
        }
    }
}
