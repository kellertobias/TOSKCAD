//! Shared, source-bound target acquisition for implemented modeling commands.
//! Selection is local UI state; only the model controller can commit geometry.
use super::*;
use crate::{modeling::ModelingTool, viewport::SceneFeed};
use multicad_core::project_file::{ContentHash, ProjectResources};
use std::collections::BTreeSet;

pub(super) struct Acquisition {
    tool: ModelingTool,
    channel: Channel,
    document: EntityId,
    revision: u64,
    source: ContentHash,
    session_guard: Prepared,
    started: bool,
}
impl Acquisition {
    fn profiles(&self) -> bool {
        matches!(self.tool, ModelingTool::Extrude | ModelingTool::Revolve)
    }
    fn accepts(&self, document: &Document, id: EntityId) -> bool {
        match document.get(id).map(|e| &e.data) {
            Some(EntityData::Sketch { .. }) => self.profiles(),
            Some(EntityData::Part { .. }) => !self.profiles(),
            Some(EntityData::Instance { component, .. }) => {
                matches!(
                    self.tool,
                    ModelingTool::Move
                        | ModelingTool::Copy
                        | ModelingTool::Align
                        | ModelingTool::Scale
                ) && (self.tool != ModelingTool::Scale
                    || matches!(
                        document.get(*component).map(|e| &e.data),
                        Some(EntityData::Part { .. })
                    ))
            }
            _ => false,
        }
    }
    fn minimum(&self) -> usize {
        if matches!(self.tool, ModelingTool::Boolean | ModelingTool::Split) {
            2
        } else {
            1
        }
    }
    fn maximum(&self) -> usize {
        if self.profiles()
            || matches!(
                self.tool,
                ModelingTool::Align | ModelingTool::EdgeTreatment | ModelingTool::Advanced(_)
            )
        {
            1
        } else {
            128
        }
    }
    fn current(&self, session: &Session) -> bool {
        self.document == session.document().id()
            && self.revision == session.revision()
            && session.validate_preview(&self.session_guard).is_ok()
    }
}
impl SketchEditor {
    /// Display guide for the Extrude arrow / Revolve handle and the straight
    /// sketch lines offered as explicit revolution axes. Display-only.
    pub(super) fn profile_guide(
        &self,
        document: &Document,
        revision: u64,
        sketch: EntityId,
        profiles: &[multicad_core::sketch_regions::RegionIntent],
    ) -> Result<crate::modeling::ProfileGuide, String> {
        let (solved, frame, _) = self
            .read_sketch(document, sketch, revision)
            .map_err(|error| error.to_string())?;
        let mut points = vec![];
        let mut lines = vec![];
        for geometry in &solved.geometry {
            let id = geometry.id();
            match geometry {
                Geometry::Line { a, b, .. } => {
                    points.extend([(id, *a), (id, *b)]);
                    lines.push(crate::modeling::GuideLine { id, a: *a, b: *b });
                }
                Geometry::Rectangle { a, b, .. } => {
                    let corners = [*a, [b[0], a[1]], *b, [a[0], b[1]]];
                    for i in 0..4 {
                        points.push((id, corners[i]));
                        lines.push(crate::modeling::GuideLine {
                            id,
                            a: corners[i],
                            b: corners[(i + 1) % 4],
                        });
                    }
                }
                Geometry::Circle {
                    center, radius_mm, ..
                } => {
                    for (du, dv) in [(1., 0.), (-1., 0.), (0., 1.), (0., -1.)] {
                        points.push((id, [center[0] + du * radius_mm, center[1] + dv * radius_mm]));
                    }
                }
                Geometry::Arc { a, b, .. } => points.extend([(id, *a), (id, *b)]),
                Geometry::BSpline { poles, .. } => points.extend(poles.iter().map(|p| (id, *p))),
            }
        }
        let selected: Vec<EntityId> = profiles
            .iter()
            .flat_map(|region| region.outer.curves.iter().copied())
            .collect();
        let anchor = crate::modeling::profile_anchor(&points, &selected)
            .ok_or("The selected sketch has no solved geometry for the operation handle")?;
        Ok(crate::modeling::ProfileGuide {
            sketch,
            frame,
            anchor,
            lines,
        })
    }
    #[cfg(test)]
    pub(super) fn model_request_for_tests(
        &self,
        session: &Session,
    ) -> Option<crate::modeling::ModelRequest> {
        self.model.request_for_tests(session)
    }
    /// Extrude arrow / Revolve handle hosting. Returns whether the gizmo owns
    /// this pointer gesture so it is not also used for target acquisition.
    pub(super) fn operation_viewport(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        session: &mut Session,
        resources: &ProjectResources,
        rect: Rect,
    ) -> bool {
        let label = self.model.label();
        let result = self.model.operation_viewport(
            ui,
            session,
            resources,
            crate::modeling::OperationViewport {
                camera: self.camera,
                rect,
                start_allowed: response.hovered()
                    || response.dragged_by(egui::PointerButton::Primary),
            },
        );
        let owned = match result {
            Ok(result) => {
                match result.outcome {
                    Some(crate::modeling::ModelToolOutcome::Applied(parts)) => {
                        self.accept_model_outputs(parts, label, true);
                    }
                    Some(crate::modeling::ModelToolOutcome::Cancelled) => self.cancel_model(),
                    Some(crate::modeling::ModelToolOutcome::Placed { parts, .. }) => {
                        self.accept_model_outputs(parts, "Modeling", true)
                    }
                    None => {}
                }
                result.input_owned
            }
            Err(error) => {
                self.message = error;
                true
            }
        };
        if self.model.pending() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        owned
    }
    /// Profiles for Extrude / Revolve: the explicitly chosen closed areas, or
    /// every closed area of a sketch selected as a whole.
    pub(super) fn profile_inputs(
        &self,
        document: &Document,
        revision: u64,
    ) -> Result<(EntityId, Vec<multicad_core::sketch_regions::RegionIntent>), String> {
        match self.selected_profiles(document, revision) {
            Ok(result) => Ok(result),
            Err(error) => {
                let Some(id) = self.selected else {
                    return Err(error);
                };
                // Only a sketch explicitly selected as a whole (tree or
                // viewer), not merely the open sketch, means all its areas.
                if self.whole_sketch != Some(id)
                    || !self.profiles.selected().is_empty()
                    || self.pending_profile_pick.is_some()
                    || !self.queued_profile_clicks.is_empty()
                {
                    return Err(error);
                }
                let presentation = self
                    .sketch_evaluation
                    .current(id)
                    .filter(|p| p.evaluated.is_current(document, revision))
                    .ok_or(error)?;
                let all: Vec<_> = presentation
                    .regions
                    .regions
                    .iter()
                    .map(|region| region.intent())
                    .collect();
                if all.is_empty() {
                    return Err("The selected sketch has no closed profile areas".into());
                }
                Ok((id, all))
            }
        }
    }
    /// Capability and conflict check, deliberately independent of selection.
    pub fn modeling_available(&self) -> bool {
        self.primitive_ready()
    }
    pub fn model_interaction_active(&self) -> bool {
        self.model.active()
            || self.assembly.active()
            || self.transfer.active()
            || self.symbols.dirty()
            || self.acquisition.is_some()
    }
    /// Operation preset for the next Boolean form (ribbon Boolean menu).
    pub fn set_boolean_preset(&mut self, operation: Option<multicad_core::model::Operation>) {
        self.model.set_boolean_preset(operation);
    }
    pub fn boolean_preset(&self) -> Option<multicad_core::model::Operation> {
        self.model.boolean_preset()
    }
    pub fn boolean_operation(&self) -> Option<multicad_core::model::Operation> {
        self.model.boolean_operation()
    }
    pub fn awaiting_model_targets(&self) -> bool {
        self.acquisition.is_some() && !self.model.active()
    }
    pub(super) fn acquiring_profiles(&self) -> bool {
        self.acquisition.as_ref().is_some_and(Acquisition::profiles)
    }
    /// Room door, window or vertical-cutout tool. A wall Part selected in the
    /// tree or viewer is the starting wall; otherwise the first wall.
    pub fn begin_room_opening(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        tool: ModelingTool,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before placing an opening"
                    .into(),
            );
        }
        if multicad_core::model::walls(session.document())
            .next()
            .is_none()
        {
            return Err("Draw a wall first with Room → Wall, then place openings in it".into());
        }
        let wall = self
            .tree_selection
            .iter()
            .find_map(|id| crate::modeling::ModelToolController::wall_of(session.document(), *id));
        self.finish()?;
        self.model
            .begin_opening(session, resources, tool, wall, channel)?;
        self.message = format!(
            "{} · click a wall face in the viewport or enter the segment and offset, then Preview",
            tool.label()
        );
        Ok(())
    }
    pub fn activate_model_tool(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        tool: ModelingTool,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.modeling_available() {
            return Err("Apply or cancel the current edits before starting another tool".into());
        }
        if matches!(tool, ModelingTool::Box | ModelingTool::Cylinder) {
            return self.begin_primitive(session, resources, tool, channel);
        }
        if tool == ModelingTool::Wall {
            return self.begin_wall(session, resources, channel);
        }
        if matches!(
            tool,
            ModelingTool::Door | ModelingTool::Window | ModelingTool::Cutout
        ) {
            return self.begin_room_opening(session, resources, tool, channel);
        }
        if tool == ModelingTool::Weld {
            return self.begin_weld(session, resources, channel);
        }
        if tool == ModelingTool::Connection {
            return self.begin_weld_connection(session, resources, channel);
        }
        if tool == ModelingTool::Member {
            return self.begin_weldment(session, resources, channel);
        }
        if tool == ModelingTool::Panel {
            return self.begin_cabinet(session, resources, channel);
        }
        if tool == ModelingTool::Joint {
            return self.begin_joint(session, resources, channel);
        }
        if tool == ModelingTool::Machining {
            return self.begin_machining(session, resources, channel);
        }
        if tool == ModelingTool::FdmAid {
            return self.begin_fdm_aid(session, resources, channel);
        }
        if tool == ModelingTool::Sheet {
            return self.begin_sheet(session, resources, channel);
        }
        if tool == ModelingTool::Level {
            return self.begin_architecture(session, resources, channel);
        }
        if let Some(
            kind @ (crate::modeling::AdvancedKind::Sweep | crate::modeling::AdvancedKind::Loft),
        ) = tool.advanced_kind()
        {
            return self.begin_advanced_model(session, resources, feed, kind, channel);
        }
        // A sketch selected in the tree or viewer outside sketch editing means
        // all of its closed areas; an open sketch still asks for its areas.
        let sketches: Vec<EntityId> = self
            .tree_selection
            .iter()
            .copied()
            .filter(|id| {
                matches!(
                    session.document().get(*id).map(|e| &e.data),
                    Some(EntityData::Sketch { .. })
                )
            })
            .collect();
        self.whole_sketch = (!self.editing && sketches.len() == 1).then(|| sketches[0]);
        if matches!(tool, ModelingTool::Extrude | ModelingTool::Revolve) && sketches.is_empty() {
            self.adopt_preselected_profile_face(session.document(), session.revision());
        } else {
            self.profile_faces.clear();
        }
        self.finish()?;
        let acquisition = Acquisition {
            tool,
            channel,
            document: session.document().id(),
            revision: session.revision(),
            source: ContentHash::document(session.document())?,
            session_guard: session
                .prepare(Request {
                    expected_revision: session.revision(),
                    actor: Actor::local(channel),
                    label: "Select modeling targets".into(),
                    edits: vec![],
                })
                .map_err(|e| e.to_string())?,
            started: false,
        };
        let before = self.tree_selection.len();
        // The Part owning a preselected planar face is that face's context,
        // not an extra unused input.
        let face_owners: BTreeSet<EntityId> = self
            .profile_faces
            .iter()
            .flat_map(|pick| {
                let source = pick.face.source;
                session
                    .document()
                    .entities()
                    .filter_map(move |e| match &e.data {
                        EntityData::Part { features, .. } if features.contains(&source) => {
                            Some(e.id)
                        }
                        _ => None,
                    })
            })
            .collect();
        let owned = self
            .tree_selection
            .iter()
            .filter(|id| face_owners.contains(id))
            .count();
        self.tree_selection
            .retain(|id| acquisition.accepts(session.document(), *id));
        let incompatible = (before - self.tree_selection.len()).saturating_sub(owned);
        let mut notice = None;
        if acquisition.profiles() {
            if self.tree_selection.len() > 1 {
                // One Extrude / Revolve feature reads profiles of one sketch.
                // Keep the whole selection visible and ask, never drop silently.
                notice = Some(format!(
                    "{} · {} sketches are selected; one feature uses the closed areas of one sketch. Keep one sketch selected (or choose its areas), then Apply",
                    tool.label(),
                    self.tree_selection.len()
                ));
                self.selected = None;
            } else {
                self.selected = self
                    .tree_selection
                    .first()
                    .copied()
                    .or(self.selected)
                    .filter(|id| acquisition.accepts(session.document(), *id));
                if let Some(id) = self.selected {
                    self.tree_selection = BTreeSet::from([id]);
                }
            }
            if incompatible > 0 && notice.is_none() {
                notice = Some(format!(
                    "{} · {incompatible} selected item(s) are neither sketch profiles nor planar model faces and were not used",
                    tool.label(),
                ));
            }
        } else {
            self.selected = None;
        }
        self.acquisition = Some(acquisition);
        self.message = if let Some(notice) = notice.clone() {
            notice
        } else if tool == ModelingTool::EdgeTreatment {
            "Fillet / Chamfer · select one Part definition, then its edges. Linked occurrences follow; Instance-only treatment is unsupported".into()
        } else {
            format!(
                "{} · select compatible targets; Shift-click adds or removes",
                tool.label()
            )
        };
        self.advance_model_targets(session, resources, feed, true);
        if let Some(notice) = notice
            && !self.message.starts_with(&notice)
        {
            self.message = format!("{notice}\n{}", self.message);
        }
        Ok(())
    }
    pub(super) fn reconcile_acquisition(&mut self, session: &Session) {
        if self
            .acquisition
            .as_ref()
            .is_some_and(|a| !a.current(session))
        {
            self.acquisition = None;
            self.model.cancel();
            self.message =
                "The source changed · transform cancelled; select the current objects again".into();
        }
    }
    pub(super) fn accept_target_selection(
        &mut self,
        document: &Document,
        selection: &BTreeSet<EntityId>,
    ) -> bool {
        let Some(a) = &self.acquisition else {
            return true;
        };
        if selection.len() > a.maximum() || selection.iter().any(|id| !a.accepts(document, *id)) {
            self.message = if a.tool == ModelingTool::EdgeTreatment {
                "Choose one Part definition. Linked occurrences follow; a placed Instance cannot be treated independently".into()
            } else if a.profiles() {
                "Select one sketch, then choose its closed areas".into()
            } else if a.maximum() == 1 {
                "Select one compatible Part or Instance".into()
            } else {
                "Select compatible Parts or Instances; Shift adds or removes targets".into()
            };
            return false;
        }
        true
    }
    pub(super) fn advance_model_targets(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        confirm: bool,
    ) {
        let Some(mut a) = self.acquisition.take() else {
            return;
        };
        if !a.current(session) {
            self.model.cancel();
            self.message = "The source changed · select current targets again".into();
            return;
        }
        if a.started {
            if let Some(scene) = feed
                .visible()
                .filter(|_| feed.is_current(a.document, a.revision))
            {
                let scaling = a.tool == ModelingTool::Scale;
                let targets = if scaling {
                    self.model.scale_targets()
                } else {
                    self.model.placement_targets()
                };
                let result = if targets == Some(&self.tree_selection) {
                    Ok(())
                } else if scaling {
                    self.model
                        .retarget_scale(session, resources, scene, &self.tree_selection)
                } else {
                    self.model
                        .retarget_placement(session, resources, scene, &self.tree_selection)
                };
                if let Err(error) = result {
                    self.message = error;
                    let targets = if scaling {
                        self.model.scale_targets()
                    } else {
                        self.model.placement_targets()
                    };
                    if let Some(targets) = targets {
                        self.tree_selection = targets.clone();
                        if let Err(error) = self.solid.select_source_entities(
                            session.document(),
                            session.revision(),
                            scene,
                            &self.tree_selection,
                        ) {
                            self.message = error;
                        }
                    }
                }
            }
            self.acquisition = Some(a);
            return;
        }
        if let Some(info) = feed
            .visible()
            .filter(|_| feed.is_current(a.document, a.revision))
            .and_then(|scene| scene.model_evaluation())
            && info.source_hash != a.source
        {
            self.message = "The native source receipt changed · wait for the current model".into();
            self.acquisition = Some(a);
            return;
        }
        let enough =
            self.tree_selection.len() >= a.minimum() && self.tree_selection.len() <= a.maximum();
        let profiles_ready = a.profiles()
            && (self
                .profile_inputs(session.document(), session.revision())
                .is_ok()
                || self.profile_face_inputs(session.revision()).is_ok());
        let ready = if a.profiles() {
            confirm && profiles_ready
        } else {
            enough
                && (confirm
                    || matches!(
                        a.tool,
                        ModelingTool::Move
                            | ModelingTool::Copy
                            | ModelingTool::Align
                            | ModelingTool::Scale
                            | ModelingTool::EdgeTreatment
                            | ModelingTool::Advanced(_)
                    ))
        };
        if !ready {
            self.acquisition = Some(a);
            return;
        }
        let result = match a.tool {
            ModelingTool::Advanced(kind) => {
                self.begin_advanced_model(session, resources, feed, kind, a.channel)
            }
            ModelingTool::EdgeTreatment => {
                self.begin_edge_treatment(session, resources, feed, a.channel)
            }
            ModelingTool::Scale => self.begin_scale(session, resources, feed, a.channel),
            ModelingTool::Move | ModelingTool::Copy => self.begin_placement(
                session,
                resources,
                feed,
                if a.tool == ModelingTool::Copy {
                    crate::modeling::PlacementMode::Copy
                } else {
                    crate::modeling::PlacementMode::Move
                },
                a.channel,
            ),
            ModelingTool::Align => self.begin_alignment(session, resources, feed, a.channel),
            ModelingTool::Extrude => self.begin_extrude(session, resources, a.channel),
            ModelingTool::Revolve => self.begin_revolve(session, resources, a.channel),
            ModelingTool::Boolean => self.begin_boolean(session, resources, feed, a.channel),
            ModelingTool::Split => self.begin_split(session, resources, feed, a.channel),
            _ => unreachable!("primitives have no target acquisition"),
        };
        match result {
            Ok(())
                if matches!(
                    a.tool,
                    ModelingTool::Move | ModelingTool::Copy | ModelingTool::Scale
                ) =>
            {
                a.started = true;
                self.acquisition = Some(a);
            }
            Ok(()) => {}
            Err(error) => {
                self.message = error;
                self.acquisition = Some(a);
            }
        }
    }
    pub(super) fn placement_basis(
        &self,
        session: &Session,
    ) -> Result<crate::modeling::GizmoBasis, String> {
        let frame = match self.coordinates {
            Coordinates::World => Datum::Xy.frame(),
            Coordinates::Custom(_) => self.basis(session.document(), session.revision())?,
            Coordinates::Object => {
                let scene = self
                    .display_scene
                    .as_deref()
                    .ok_or("Wait for current native geometry")?;
                let targets = self
                    .model
                    .placement_targets()
                    .or_else(|| self.model.scale_targets())
                    .ok_or("Start Move / Copy or Scale first")?;
                return crate::modeling::object_gizmo_basis(
                    session.document(),
                    session.revision(),
                    scene,
                    targets,
                )
                .map(|resolved| resolved.basis);
            }
        };
        Ok(crate::modeling::GizmoBasis {
            axes: [frame.u, frame.v, frame.n],
        })
    }
    /// Returns true when this stage owns the Inspector instead of a model form.
    pub(super) fn target_inspector(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
    ) -> bool {
        let Some(a) = &self.acquisition else {
            return false;
        };
        if a.started {
            ui.small(if a.tool == ModelingTool::Scale { "Shift-click objects to add/remove targets. Drag the center for uniform scale or an axis handle for one factor. Option/Alt creates a copy." } else { "Shift-click objects to add/remove targets. Drag arrows, planes or rings. Option/Alt creates a copy." });
            ui.separator();
            return false;
        }
        ui.strong(format!("{} · select targets", a.tool.label()));
        ui.label(if a.tool==ModelingTool::EdgeTreatment {"Select one Part definition in the tree or viewer, then its native edges. Linked occurrences follow; Instance-only treatment is unsupported."} else if a.profiles() { "Select a sketch in the tree, then choose one or more closed areas below or in the viewer. Or click planar model faces in the viewer (Shift, Command or Control adds)." } else { "Select objects in the tree or viewer. Shift-click adds or removes a target." });
        if a.profiles() && !self.profile_faces.is_empty() {
            ui.small(format!(
                "{} planar face(s) selected as profiles",
                self.profile_faces.len()
            ));
        }
        ui.small(format!("{} selected", self.tree_selection.len()));
        let profiles = a.profiles();
        if profiles && let Some(id) = self.selected {
            if let Some(p) = self.sketch_evaluation.current(id).filter(|p| {
                p.evaluated
                    .is_current(session.document(), session.revision())
            }) {
                self.profiles.show(ui, id, &p.regions);
            } else {
                ui.small("Waiting for current solved profiles…");
            }
        }
        if !feed.is_current(session.document().id(), session.revision()) && !profiles {
            ui.small("Waiting for the current native model…");
        }
        let enough = self
            .acquisition
            .as_ref()
            .is_some_and(|a| self.tree_selection.len() >= a.minimum());
        let ready = if profiles {
            self.profile_inputs(session.document(), session.revision())
                .is_ok()
                || self.profile_face_inputs(session.revision()).is_ok()
        } else {
            enough && feed.is_current(session.document().id(), session.revision())
        };
        let clicked = crate::inspector_actions::row(
            ui,
            crate::inspector_actions::Owner::Targets,
            None,
            crate::inspector_actions::Button::new(
                "Use selected targets",
                ready,
                "Open the same modeling controls as selection-first entry",
            ),
            crate::inspector_actions::Button::new(
                "Cancel tool",
                true,
                "Leave selection mode without changing the document",
            ),
        );
        if clicked.apply {
            self.advance_model_targets(session, resources, feed, true);
        }
        if clicked.cancel {
            self.cancel_model();
        }
        true
    }
    pub(super) fn target_viewport(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        rect: Rect,
    ) {
        let Some(a) = &self.acquisition else {
            return;
        };
        if !response.clicked() {
            return;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return;
        };
        let modifiers = crate::selection_input::click_modifiers(ui, response);
        if a.profiles() && self.selected.is_none() {
            let toggle = modifiers.shift || modifiers.command || modifiers.ctrl;
            if let Some(scene) = feed
                .visible()
                .filter(|_| feed.is_current(session.document().id(), session.revision()))
                .cloned()
            {
                self.pick_profile_face(session, &scene, rect, pointer, toggle);
            } else {
                self.message = "Wait for current native geometry before selecting faces".into();
            }
            return;
        }
        if a.profiles() {
            if let Some(id) = self.selected
                && let Ok((_, frame, _)) =
                    self.read_sketch(session.document(), id, session.revision())
                && let Some(point) = self.camera.on_plane(pointer, rect, frame)
                && let Err(error) = self.enqueue_profile_click(
                    session.document(),
                    session.revision(),
                    id,
                    point,
                    modifiers.shift || modifiers.command || modifiers.ctrl,
                )
            {
                self.message = error;
            }
            return;
        }
        let Some(scene) = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
        else {
            self.message = "Wait for current native geometry before selecting targets".into();
            return;
        };
        let hit = scene.pick_screen_with(
            self.camera,
            rect,
            pointer,
            crate::viewport::PickOptions {
                filter: crate::viewport::PickFilter::Parts,
                include_hidden: false,
                section: self.solid.section.plane(),
            },
        );
        let Some(hit) = hit else {
            return;
        };
        let id = hit.instance;
        let mut selection = if modifiers.shift || modifiers.command || modifiers.ctrl {
            self.tree_selection.clone()
        } else {
            BTreeSet::new()
        };
        if !selection.insert(id) {
            selection.remove(&id);
        }
        if !self.accept_target_selection(session.document(), &selection) {
            return;
        }
        self.tree_selection = selection;
        self.selected = None;
        if let Err(error) = self.solid.select_source_entities(
            session.document(),
            session.revision(),
            scene,
            &self.tree_selection,
        ) {
            self.message = error;
        }
        self.advance_model_targets(session, resources, feed, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::native_fixture::NativeBackend;
    use multicad_core::model::{
        self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
    };
    use std::time::{Duration, Instant};
    fn fixture() -> (Session, SceneFeed, SketchEditor, [EntityId; 2]) {
        let mut session =
            Session::new(Document::new("Tool-first native objects").unwrap()).unwrap();
        let mut ids = [EntityId::new(); 2];
        for (i, id) in ids.iter_mut().enumerate() {
            let request = PrimitiveRequest::new(
                format!("Box {}", i + 1),
                PrimitiveParameters::Box {
                    size: [length(30.), length(30.), length(30.)],
                    frame: PrimitiveFrame {
                        origin: [length(i as f64 * 65.), length(0.), length(0.)],
                        ..Default::default()
                    },
                },
            );
            *id = request.body;
            let prepared = model::prepare_primitive(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                &request,
            )
            .unwrap();
            session.commit(prepared.prepared).unwrap();
        }
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(NativeBackend::new()));
        let until = Instant::now() + Duration::from_secs(5);
        while !feed.is_current(session.document().id(), session.revision())
            && Instant::now() < until
        {
            feed.tick(session.document(), session.revision());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            feed.is_current(session.document().id(), session.revision()),
            "{:?}",
            feed.error()
        );
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(NativeBackend::new()));
        (session, feed, editor, ids)
    }
    fn pointer_frame(
        ctx: &egui::Context,
        editor: &mut SketchEditor,
        session: &Session,
        feed: &SceneFeed,
        mut events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Rect {
        let mut rect = Rect::NOTHING;
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 600.))),
                events,
                ..Default::default()
            },
            |ui| {
                let (r, response) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                rect = r;
                editor.target_viewport(ui, &response, session, &Default::default(), feed, r);
            },
        );
        output.textures_delta.clear();
        rect
    }
    fn click(
        ctx: &egui::Context,
        editor: &mut SketchEditor,
        session: &Session,
        feed: &SceneFeed,
        point: [f32; 3],
        rect: Rect,
        shift: bool,
    ) {
        let screen = editor.camera.project(point, rect);
        let pos = Pos2::new(screen[0], screen[1]);
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };
        pointer_frame(
            ctx,
            editor,
            session,
            feed,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
            modifiers,
        );
        // Release modifiers are authoritative even if Shift is released in the same frame.
        pointer_frame(
            ctx,
            editor,
            session,
            feed,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            egui::Modifiers::NONE,
        );
    }
    #[test]
    fn native_tool_first_click_and_shift_toggle_match_selection_first_and_cancel_is_neutral() {
        let (session, feed, mut editor, ids) = fixture();
        let source = session.document().clone();
        let revision = session.revision();
        editor
            .activate_model_tool(
                &session,
                &Default::default(),
                &feed,
                ModelingTool::Move,
                Channel::Ui,
            )
            .unwrap();
        assert!(editor.awaiting_model_targets());
        assert!(editor.has_pending_edits());
        let ctx = egui::Context::default();
        let rect = pointer_frame(
            &ctx,
            &mut editor,
            &session,
            &feed,
            vec![],
            Default::default(),
        );
        editor.camera.fit(&[[0., 0., 0.], [95., 30., 30.]], rect);
        click(
            &ctx,
            &mut editor,
            &session,
            &feed,
            [15., 15., 15.],
            rect,
            false,
        );
        assert!(editor.model.placement_active(), "{}", editor.message);
        assert_eq!(
            editor.model.placement_targets(),
            Some(&BTreeSet::from([ids[0]]))
        );
        click(
            &ctx,
            &mut editor,
            &session,
            &feed,
            [80., 15., 15.],
            rect,
            true,
        );
        assert_eq!(editor.model.placement_targets(), Some(&BTreeSet::from(ids)));
        click(
            &ctx,
            &mut editor,
            &session,
            &feed,
            [15., 15., 15.],
            rect,
            true,
        );
        assert_eq!(
            editor.model.placement_targets(),
            Some(&BTreeSet::from([ids[1]]))
        );
        click(
            &ctx,
            &mut editor,
            &session,
            &feed,
            [80., 15., 15.],
            rect,
            true,
        );
        assert_eq!(editor.model.placement_targets(), Some(&BTreeSet::new()));
        assert!(!editor.model_can_apply(&session, &Default::default()));
        editor.cancel_model();
        assert!(!editor.has_pending_edits());
        editor.tree_selection = BTreeSet::from([ids[1]]);
        editor
            .activate_model_tool(
                &session,
                &Default::default(),
                &feed,
                ModelingTool::Move,
                Channel::Ui,
            )
            .unwrap();
        assert_eq!(
            editor.model.placement_targets(),
            Some(&BTreeSet::from([ids[1]]))
        );
        editor.cancel();
        assert_eq!(session.document(), &source);
        assert_eq!(session.revision(), revision);
    }
    #[test]
    fn fit_retains_the_selected_native_target_after_transform_takes_ownership() {
        let (mut session, mut feed, mut editor, ids) = fixture();
        editor.tree_selection = BTreeSet::from([ids[0]]);
        editor
            .activate_model_tool(
                &session,
                &Default::default(),
                &feed,
                ModelingTool::Move,
                Channel::Ui,
            )
            .unwrap();
        assert!(editor.model.placement_active());
        editor.solid.clear();
        editor.request_fit();
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 860.))),
                ..Default::default()
            },
            |ui| editor.show_project(ui, &mut session, false, &mut feed, &Default::default()),
        )
        .drop_without_applying_deltas();
        for axis in 0..3 {
            assert!(
                (editor.camera.target[axis] - 15.).abs() < 1e-4,
                "{:?}",
                editor.camera.target
            );
        }
    }
    #[test]
    fn delayed_native_readiness_cannot_start_placement_while_measure_owns_input() {
        let (mut session, mut feed, mut editor, ids) = fixture();
        editor.tree_selection = BTreeSet::from([ids[0]]);
        editor
            .activate_model_tool(
                &session,
                &Default::default(),
                &SceneFeed::default(),
                ModelingTool::Move,
                Channel::Ui,
            )
            .unwrap();
        assert!(editor.awaiting_model_targets());
        editor.measurement.open();
        let ctx = egui::Context::default();
        let mut frame = |editor: &mut SketchEditor| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 860.))),
                    ..Default::default()
                },
                |ui| editor.show_project(ui, &mut session, false, &mut feed, &Default::default()),
            )
            .drop_without_applying_deltas();
        };
        frame(&mut editor);
        assert!(editor.measurement.active());
        assert!(editor.awaiting_model_targets());
        assert!(!editor.model.active());
        editor.measurement.close();
        frame(&mut editor);
        assert!(editor.model.placement_active());
        editor.cancel_model();
    }
    #[test]
    fn supported_tools_enter_selection_without_targets_and_stale_source_cancels() {
        let (mut session, feed, mut editor, ids) = fixture();
        for tool in [
            ModelingTool::Move,
            ModelingTool::Copy,
            ModelingTool::Scale,
            ModelingTool::EdgeTreatment,
            ModelingTool::Align,
            ModelingTool::Boolean,
            ModelingTool::Split,
            ModelingTool::Extrude,
            ModelingTool::Revolve,
        ] {
            editor.tree_selection.clear();
            editor.selected = None;
            assert!(editor.modeling_available());
            editor
                .activate_model_tool(&session, &Default::default(), &feed, tool, Channel::Ui)
                .unwrap();
            assert!(editor.awaiting_model_targets(), "{tool:?}");
            assert!(!editor.accept_target_selection(
                session.document(),
                &BTreeSet::from([session.document().id()])
            ));
            editor.cancel_model();
        }
        editor
            .activate_model_tool(
                &session,
                &Default::default(),
                &feed,
                ModelingTool::Move,
                Channel::Ui,
            )
            .unwrap();
        let mut body = session.document().get(ids[0]).unwrap().clone();
        body.name = "Renamed".into();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Rename target".into(),
                edits: vec![Mutation::Replace(body)],
            })
            .unwrap();
        let changed = session.document().clone();
        editor.reconcile_acquisition(&session);
        assert!(!editor.model_interaction_active());
        assert!(!editor.has_pending_edits());
        assert_eq!(session.document(), &changed);
    }
}
