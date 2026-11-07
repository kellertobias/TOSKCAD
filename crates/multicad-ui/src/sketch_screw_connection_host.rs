//! Receiving picks use exact predecessor geometry, independently of the
//! connection's view-only section and exploded stack presentation.
use super::*;
use crate::{
    modeling::{HardwareLibraryRequest, ScrewConnectionPick, ScrewConnectionPickKind},
    viewport::{PickFilter, PickOptions, Scene, SceneFeed, SectionPlane, Selection},
};
use multicad_core::{
    project_file::ProjectResources,
    references::{AnalyticGeometry, TopologyKind},
};
use std::{collections::BTreeMap, sync::Arc};

pub(super) struct ConnectionPickView {
    canonical: Arc<Scene>,
    pick: ScrewConnectionPick,
    view: Arc<Scene>,
    camera: Camera,
}

impl SketchEditor {
    pub(crate) fn connection_needs_library(&self) -> bool {
        self.model.connection_needs_library()
    }
    pub(crate) fn connection_library_request(&self) -> Option<HardwareLibraryRequest> {
        self.model.connection_library_request()
    }
    pub(crate) fn accept_connection_library(
        &mut self,
        request: &HardwareLibraryRequest,
        definitions: Result<Vec<multicad_core::symbols::SymbolDefinition>, String>,
    ) -> Result<(), String> {
        self.model.accept_connection_library(request, definitions)
    }

    pub(super) fn connection_pick_scene(
        &mut self,
        session: &Session,
        feed: &SceneFeed,
    ) -> Result<Option<Arc<Scene>>, String> {
        let Some(pick) = self
            .model
            .screw_connection_picker()
            .filter(|_| !self.measurement.active())
        else {
            if let Some(previous) = self.connection_pick_view.take() {
                self.camera = previous.camera;
                self.fit_requested = false;
            }
            return Ok(None);
        };
        let canonical = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
            .ok_or("Wait for the current native receiver before picking")?
            .clone();
        if let Some(cache) = &self.connection_pick_view
            && cache.pick == pick
            && Arc::ptr_eq(&cache.canonical, &canonical)
        {
            return Ok(Some(cache.view.clone()));
        }
        let view = canonical.isolated_source(pick.receiver.producer)?;
        let camera = self
            .connection_pick_view
            .as_ref()
            .map_or(self.camera, |previous| previous.camera);
        self.connection_pick_view = Some(ConnectionPickView {
            canonical,
            pick,
            view: view.clone(),
            camera,
        });
        self.fit_requested = true;
        Ok(Some(view))
    }

    fn connection_display_hit(
        &self,
        presented: &Scene,
        ray: crate::viewport::Ray,
    ) -> Option<crate::viewport::DisplayLeafHit> {
        presented.pick_display_leaf(ray, self.modeling_section())
    }

    pub(super) fn connection_viewport(
        &mut self,
        response: &egui::Response,
        session: &Session,
        resources: &ProjectResources,
        feed: &SceneFeed,
        presented: Option<&Scene>,
        rect: Rect,
    ) {
        if self.model.screw_connection_picker().is_none() {
            if self.model.connection_view_options().is_none() {
                return;
            }
            self.solid.highlight_reference(None);
            if let Some(presented) = presented.filter(|scene| {
                self.model
                    .connection_display_current(session, resources, scene)
            }) {
                let hit = response
                    .hover_pos()
                    .and_then(|pointer| self.camera.ray(pointer, rect))
                    .and_then(|ray| self.connection_display_hit(presented, ray))
                    .filter(|hit| self.model.connection_leaf_selectable(&hit.path));
                if response.clicked()
                    && let Some(hit) = &hit
                    && self
                        .model
                        .focus_connection_leaf(session, resources, presented, &hit.path)
                {
                    response.ctx.memory_mut(|memory| {
                        if let Some(focused) = memory.focused() {
                            memory.surrender_focus(focused);
                        }
                    });
                }
                let selected = hit.map(|hit| hit.display_instance).or_else(|| {
                    presented
                        .display_leaf_paths()
                        .iter()
                        .find_map(|(id, path)| {
                            self.model.connection_leaf_focused(path).then_some(*id)
                        })
                });
                self.solid
                    .highlight_reference(selected.map(Selection::Part));
            }
            return;
        }
        let Ok(Some(view)) = self.connection_pick_scene(session, feed) else {
            return;
        };
        let Some(cache) = &self.connection_pick_view else {
            return;
        };
        let canonical = cache.canonical.clone();
        let producer = cache.pick.receiver.producer;
        let capture_kind = cache.pick.kind;
        let Some(snapshot) = canonical.geometry_snapshot(producer) else {
            return;
        };
        let hit = response.hover_pos().and_then(|pointer| {
            view.pick_screen_matching(
                self.camera,
                rect,
                pointer,
                PickOptions {
                    filter: PickFilter::Faces,
                    include_hidden: false,
                    section: None,
                },
                |instance, key| {
                    instance == producer
                        && key.kind == TopologyKind::Face
                        && snapshot.elements.iter().any(|element| {
                            element.key == key
                                && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
                                && !element.frames.is_empty()
                        })
                },
            )
        });
        self.solid
            .highlight_reference(hit.as_ref().map(|hit| Selection::Face {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            }));
        if !response.clicked() {
            return;
        }
        let Some(hit) = hit else {
            self.message = "Choose a named planar face on this receiving predecessor".into();
            return;
        };
        let result = multicad_core::model::capture_profile_face(&snapshot, hit.key, None).and_then(
            |mut face| {
                if let Some(attachment) = &mut face.intent.attachment
                    && let Some(frame) = snapshot
                        .elements
                        .iter()
                        .find(|element| element.key == hit.key)
                        .and_then(|element| {
                            element
                                .frames
                                .iter()
                                .find(|frame| frame.origin == attachment.anchor)
                        })
                {
                    let delta: [f64; 3] =
                        std::array::from_fn(|i| hit.point_mm[i] - frame.frame.origin_mm[i]);
                    attachment.offset_mm = [
                        delta.iter().zip(frame.frame.u).map(|(a, b)| a * b).sum(),
                        delta.iter().zip(frame.frame.v).map(|(a, b)| a * b).sum(),
                        0.,
                    ];
                }
                self.model
                    .set_connection_capture(session, resources, &canonical, face)
            },
        );
        match result {
            Ok(()) => {
                self.message = match capture_kind {
                    ScrewConnectionPickKind::EntryAxis => "Connection entry face and axis selected · Preview validates the complete connection",
                    ScrewConnectionPickKind::ReceiverEntry => "Receiving entry face captured · Preview follows its current native station",
                    ScrewConnectionPickKind::GripEnd => "Grip end face captured · Preview derives the current native stack grip",
                }.into()
            }
            Err(error) => self.message = error,
        }
    }

    pub(super) fn paint_connection_axis(&self, painter: &egui::Painter, rect: Rect) {
        if self.model.screw_connection_picker().is_some() {
            return;
        }
        let Some(pose) = self.connection_axis_pose else {
            return;
        };
        let project = |point: [f64; 3]| {
            let p = self.camera.project(point.map(|value| value as f32), rect);
            egui::pos2(p[0], p[1])
        };
        let origin = project(pose.translation_mm);
        let end = project(pose.point([0., 0., 25.]));
        if !origin.is_finite() || !end.is_finite() {
            return;
        }
        let color = egui::Color32::from_rgb(245, 190, 70);
        painter.arrow(origin, end - origin, egui::Stroke::new(2., color));
        painter.circle_stroke(origin, 5., egui::Stroke::new(2., color));
        painter.text(
            end,
            egui::Align2::LEFT_BOTTOM,
            "Connection +axis",
            egui::FontId::proportional(12.),
            color,
        );
    }

    pub(super) fn connection_presentation(
        &mut self,
        session: &Session,
        scene: &Arc<Scene>,
    ) -> Result<Arc<Scene>, String> {
        self.connection_section = None;
        self.connection_axis_pose = None;
        if self.measurement.active() {
            self.fit_requested |= self.connection_exploded_axis.take().is_some();
            return Ok(scene.clone());
        }
        let Some((section, explode)) = self.model.connection_view_options() else {
            self.fit_requested |= self.connection_exploded_axis.take().is_some();
            return Ok(scene.clone());
        };
        if self.model.screw_connection_picker().is_some() {
            return Ok(scene.clone());
        }
        let Some(axis) = self.model.connection_axis() else {
            return Ok(scene.clone());
        };
        let pose = axis
            .source
            .resolve(
                session.document(),
                session.parameters(),
                &axis.face,
                session.revision(),
                &scene.reference_snapshots(),
            )
            .map_err(|error| error.to_string())?;
        self.connection_axis_pose = Some(pose);
        if section {
            let normal = pose.vector([1., 0., 0.]);
            let offset = normal
                .into_iter()
                .zip(pose.translation_mm)
                .map(|(a, b)| a * b)
                .sum();
            self.connection_section = Some(SectionPlane::new(normal, offset)?);
        }
        if !explode {
            self.fit_requested |= self.connection_exploded_axis.take().is_some();
        }
        let roles = self.model.connection_hardware_roles();
        let Some(receipt) = scene.assembly_evaluation() else {
            return Ok(scene.clone());
        };
        let receiving = self.model.connection_receiving_paths();
        let mut hardware_bounds = BTreeMap::new();
        let mut stock_bounds = None;
        if explode {
            for leaf in &receipt.leaves {
                let hardware = roles.iter().any(|(id, _)| leaf.path.root == *id);
                let receiver = receiving.contains(&leaf.path);
                if !hardware && !receiver {
                    continue;
                }
                let bounds = scene
                    .leaf_display_bounds(&leaf.path)
                    .ok_or("Current connection leaf has no display bounds for exploded layout")?;
                let interval = projected_interval(bounds, pose)?;
                if hardware {
                    include_interval(
                        hardware_bounds.entry(leaf.path.root).or_insert(interval),
                        interval,
                    );
                }
                if receiver {
                    include_interval(stock_bounds.get_or_insert(interval), interval);
                }
            }
        }
        let exploded_stations = if explode {
            let Some(stock_bounds) = stock_bounds else {
                return Ok(scene.clone());
            };
            exploded_role_stations(&roles, &hardware_bounds, stock_bounds)?
        } else {
            BTreeMap::new()
        };
        let mut offsets = BTreeMap::new();
        for leaf in &receipt.leaves {
            if let Some((id, _)) = roles.iter().find(|(id, _)| leaf.path.root == *id) {
                offsets.insert(
                    leaf.path.clone(),
                    if explode {
                        pose.vector([
                            0.,
                            0.,
                            *exploded_stations
                                .get(id)
                                .ok_or("Displayed hardware has no valid exploded layout")?,
                        ])
                    } else {
                        [0.; 3]
                    },
                );
            }
        }
        for leaf in &receipt.leaves {
            if receiving.iter().any(|path| path == &leaf.path) {
                offsets.entry(leaf.path.clone()).or_insert([0.; 3]);
            }
        }
        if offsets.is_empty() {
            return Ok(scene.clone());
        }
        let roots = offsets
            .keys()
            .map(|path| path.root)
            .collect::<std::collections::BTreeSet<_>>();
        for leaf in &receipt.leaves {
            if roots.contains(&leaf.path.root) {
                offsets.entry(leaf.path.clone()).or_insert([0.; 3]);
            }
        }
        let presented = scene.presentation_leaf_offsets(&offsets, &BTreeMap::new())?;
        if explode && self.connection_exploded_axis != Some(axis.id) {
            self.connection_exploded_axis = Some(axis.id);
            self.fit_requested = true;
        }
        Ok(presented)
    }
}

fn include_interval(target: &mut [f64; 2], interval: [f64; 2]) {
    target[0] = target[0].min(interval[0]);
    target[1] = target[1].max(interval[1]);
}

/// Conservative projection of all eight corners encloses every drawn triangle.
fn projected_interval(
    bounds: [[f64; 3]; 2],
    axis: multicad_core::model::RigidPose,
) -> Result<[f64; 2], String> {
    if bounds.iter().flatten().any(|value| !value.is_finite())
        || (0..3).any(|i| bounds[0][i] > bounds[1][i])
    {
        return Err("Exploded layout needs finite ordered display bounds".into());
    }
    let normal = axis.vector([0., 0., 1.]);
    let mut interval = [f64::INFINITY, f64::NEG_INFINITY];
    for corner in 0..8 {
        let station = (0..3)
            .map(|i| {
                (bounds[usize::from(corner & (1 << i) != 0)][i] - axis.translation_mm[i])
                    * normal[i]
            })
            .sum::<f64>();
        include_interval(&mut interval, [station; 2]);
    }
    Ok(interval)
}

/// Move complete occupied intervals outward, retaining role and washer order.
fn exploded_role_stations(
    roles: &[(EntityId, multicad_core::model::ScrewConnectionHardwareRole)],
    bounds: &BTreeMap<EntityId, [f64; 2]>,
    stock: [f64; 2],
) -> Result<BTreeMap<EntityId, f64>, String> {
    use multicad_core::model::ScrewConnectionHardwareRole as Role;
    let hanger = roles.iter().any(|(_, role)| *role == Role::HangerBolt);
    let ids = |role| {
        roles
            .iter()
            .filter(move |(_, candidate)| *candidate == role)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
    };
    let mut negative = ids(Role::Insert);
    negative.extend(ids(Role::HeadWasher).into_iter().rev());
    if hanger {
        negative.extend(ids(Role::NutWasher).into_iter().rev());
        negative.extend(ids(Role::Nut));
        negative.extend(ids(Role::HangerBolt));
    } else {
        negative.extend(ids(Role::Screw));
    }
    let mut positive = vec![];
    if !hanger {
        positive.extend(ids(Role::NutWasher));
        positive.extend(ids(Role::Nut));
    }
    let mut offsets = BTreeMap::new();
    for (group, mut cursor, sign) in [(negative, stock[0], -1.), (positive, stock[1], 1.)] {
        for id in group {
            let Some(interval) = bounds.get(&id) else {
                continue;
            };
            if interval.iter().any(|value| !value.is_finite()) || interval[0] > interval[1] {
                return Err("Exploded hardware has invalid occupied bounds".into());
            }
            let offset = if sign < 0. {
                (cursor - 15. - interval[1]).min(-15.)
            } else {
                (cursor + 15. - interval[0]).max(15.)
            };
            if !offset.is_finite() || offset.abs() > 1e7 {
                return Err(
                    "Exploded hardware separation exceeds the display coordinate budget".into(),
                );
            }
            cursor = interval[usize::from(sign > 0.)] + offset;
            offsets.insert(id, offset);
        }
    }
    if bounds.keys().any(|id| !offsets.contains_key(id)) {
        return Err("Displayed hardware roles do not form a valid exploded layout".into());
    }
    Ok(offsets)
}

#[cfg(test)]
#[path = "../../multicad-model/tests/support/custom_screw_connections.rs"]
mod custom_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::native_fixture::screw_connection as fixture;

    fn enable_viewport_section(editor: &mut SketchEditor) {
        let context = egui::Context::default();
        let button = std::cell::Cell::new(egui::Pos2::ZERO);
        let mut frame = |events| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600., 420.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    button.set(ui.cursor().min + egui::vec2(12., 12.));
                    editor.solid.section.control(ui, None);
                },
            )
        };
        let event = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(vec![]).drop_without_applying_deltas();
        let mut click = |pos| {
            frame(event(pos, true)).drop_without_applying_deltas();
            frame(event(pos, false))
        };
        click(button.get()).drop_without_applying_deltas();
        let output = frame(vec![]);
        let enable = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Enable section" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("ordinary viewport section control");
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(event(enable, pressed)).drop_without_applying_deltas();
        }
        assert_eq!(
            editor.solid.section.plane(),
            Some(SectionPlane::new([1., 0., 0.], 0.).unwrap())
        );
    }

    #[test]
    fn connection_display_picks_follow_viewport_clipping_and_measure_uses_committed_scene() {
        use multicad_core::model::{
            self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
        };
        let (document, request, _) = fixture::fixture();
        let mut session = Session::new(document).unwrap();
        let actor = Actor::local(Channel::Ui);
        let foreground = PrimitiveRequest::new(
            "Clipped foreground",
            PrimitiveParameters::Box {
                size: std::array::from_fn(|_| length(2.)),
                frame: PrimitiveFrame {
                    origin: [length(-5.), length(0.), length(0.)],
                    ..Default::default()
                },
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor.clone(), &foreground)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        let prepared =
            model::prepare_screw_connection(&session, session.revision(), actor, &request).unwrap();
        session.commit(prepared.prepared).unwrap();
        let native = multicad_model::Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let committed =
            crate::modeling::native_fixture::scene(session.document(), native.snapshot(), 1)
                .unwrap();
        let offsets = committed
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .map(|leaf| (leaf.path.clone(), [0.; 3]))
            .collect();
        let presented = committed
            .presentation_leaf_offsets(&offsets, &BTreeMap::new())
            .unwrap();
        let ray = crate::viewport::Ray::new([-10., 1., 1.], [1., 0., 0.]).unwrap();
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(
            crate::modeling::native_fixture::NativeBackend::new(),
        ));
        editor
            .model
            .begin_existing(
                &session,
                &Default::default(),
                request.connection,
                Channel::Ui,
            )
            .unwrap();
        let stamp = editor.model.stamp();
        assert_eq!(
            editor
                .connection_display_hit(&presented, ray)
                .unwrap()
                .path
                .root,
            foreground.body
        );
        enable_viewport_section(&mut editor);
        let normal_section = editor.solid.section.plane();
        let receiving = &request.receivers[0].source.occurrence;
        assert_eq!(
            &editor.connection_display_hit(&presented, ray).unwrap().path,
            receiving
        );

        // Enable the actual display option through its painted checkbox.
        let context = egui::Context::default();
        let mut frame = |events| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 20000.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor
                        .model
                        .show(ui, &mut session, &Default::default())
                        .unwrap();
                },
            )
        };
        let output = frame(vec![]);
        let checkbox = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Exploded stack" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("painted exploded display control");
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(checkbox),
                egui::Event::PointerButton {
                    pos: checkbox,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ])
            .drop_without_applying_deltas();
        }
        assert_eq!(
            editor
                .model
                .connection_view_options()
                .map(|(_, explode)| explode),
            Some(true)
        );
        let view = editor
            .connection_presentation(&session, &committed)
            .unwrap();
        assert_eq!(view.scope(), &crate::viewport::SceneScope::Presentation);
        assert!(editor.connection_exploded_axis.is_some());
        editor.fit_requested = false;
        editor.connection_section = Some(SectionPlane::new([0., 0., 1.], 50.).unwrap());
        editor.measurement.open();
        assert_eq!(editor.modeling_section(), normal_section);
        assert!(
            editor
                .connection_pick_scene(&session, &SceneFeed::default())
                .unwrap()
                .is_none()
        );
        let measured = editor
            .connection_presentation(&session, &committed)
            .unwrap();
        assert!(Arc::ptr_eq(&measured, &committed));
        assert!(editor.connection_exploded_axis.is_none());
        assert!(
            editor.fit_requested,
            "Measure entry fits committed geometry"
        );
        // Simulate consuming that fit, then manually fitting within Measure.
        editor.fit_requested = false;
        editor
            .connection_presentation(&session, &committed)
            .unwrap();
        assert!(
            !editor.fit_requested,
            "Measure must not request repeated fits"
        );
        assert!(
            measured
                .pick_face_section(ray, editor.modeling_section())
                .is_some()
        );
        assert!(editor.connection_axis_pose.is_none());
        assert_eq!(editor.model.stamp(), stamp);
        editor.measurement.close();
        let resumed = editor
            .connection_presentation(&session, &committed)
            .unwrap();
        assert_eq!(resumed.scope(), &crate::viewport::SceneScope::Presentation);
        assert!(
            editor.fit_requested,
            "Resume fits the complete exploded layout"
        );
        assert!(editor.connection_exploded_axis.is_some());
        assert_eq!(editor.model.stamp(), stamp);
    }

    #[test]
    fn exploded_projection_encloses_world_bounds_on_a_rotated_axis() {
        let axis = multicad_core::model::RigidPose {
            rotation: [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]],
            translation_mm: [0.; 3],
        };
        assert_eq!(
            projected_interval([[1., 2., 3.], [4., 6., 8.]], axis).unwrap(),
            [-6., -2.]
        );
        assert!(projected_interval([[f64::NAN; 3], [1.; 3]], axis).is_err());
    }

    #[test]
    fn exploded_native_bounds_separate_twelve_twelve_hanger_and_long_screw() {
        use multicad_core::model::{self, ScrewConnectionHardwareRole as Role};
        let (hanger_document, mut hanger) =
            custom_fixture::fixture(model::ScrewConnectionMode::HangerBolt);
        for row in &mut hanger.hardware {
            let model::ScrewConnectionHardwareSource::Generated { part } = &mut row.source else {
                unreachable!()
            };
            match &mut part.parameters {
                model::MechanicalPartParameters::HexNut { thread, .. } => {
                    thread.length = model::length(2.5)
                }
                model::MechanicalPartParameters::Washer { thickness, .. } => {
                    *thickness = model::length(0.5)
                }
                model::MechanicalPartParameters::CustomConnection { parameters } => {
                    let model::CustomConnectionHardwareParameters::HangerBolt {
                        wood_thread,
                        metric_thread,
                        ..
                    } = parameters.as_ref()
                    else {
                        unreachable!()
                    };
                    assert_eq!(wood_thread.length, model::length(12.));
                    assert_eq!(metric_thread.length, model::length(12.));
                }
                _ => unreachable!(),
            }
        }
        let (screw_document, mut long_screw, _) = fixture::fixture();
        let model::ScrewConnectionHardwareSource::Generated { part } =
            &mut long_screw.hardware[0].source
        else {
            unreachable!()
        };
        let model::MechanicalPartParameters::MachineScrew {
            length_input,
            thread,
            ..
        } = &mut part.parameters
        else {
            unreachable!()
        };
        *length_input = Some(model::ScrewLengthInput::Underhead {
            value: model::length(24.),
        });
        thread.length = model::length(24.);
        for (document, request, is_hanger) in [
            (hanger_document, hanger, true),
            (screw_document, long_screw, false),
        ] {
            let mut session = Session::new(document).unwrap();
            let prepared =
                model::prepare_screw_connection(&session, 0, Actor::local(Channel::Ui), &request)
                    .unwrap();
            session.commit(prepared.prepared).unwrap();
            let source = session.document().clone();
            let native = multicad_model::Compiler::default()
                .compile(session.document(), session.revision())
                .unwrap();
            assert_eq!(native.snapshot().screw_connection_checks.len(), 1);
            let scene =
                crate::modeling::native_fixture::scene(session.document(), native.snapshot(), 1)
                    .unwrap();
            let pose = request
                .axis
                .source
                .resolve(
                    session.document(),
                    session.parameters(),
                    &request.axis.face,
                    session.revision(),
                    &scene.reference_snapshots(),
                )
                .unwrap();
            let roles = request
                .hardware
                .iter()
                .map(|row| (row.instance, row.role))
                .collect::<Vec<_>>();
            let receipt = scene.assembly_evaluation().unwrap();
            let mut hardware = BTreeMap::new();
            let mut stock = None;
            for leaf in &receipt.leaves {
                let interval =
                    projected_interval(scene.leaf_display_bounds(&leaf.path).unwrap(), pose)
                        .unwrap();
                if roles.iter().any(|(id, _)| *id == leaf.path.root) {
                    include_interval(hardware.entry(leaf.path.root).or_insert(interval), interval);
                }
                if request
                    .receivers
                    .iter()
                    .any(|row| row.source.occurrence == leaf.path)
                {
                    include_interval(stock.get_or_insert(interval), interval);
                }
            }
            let stock = stock.unwrap();
            let shifts = exploded_role_stations(&roles, &hardware, stock).unwrap();
            let shifted = |id: EntityId| hardware[&id].map(|value| value + shifts[&id]);
            let gap = |a: [f64; 2], b: [f64; 2]| (a[0] - b[1]).max(b[0] - a[1]);
            for (i, (id, _)) in roles.iter().enumerate() {
                assert!(gap(shifted(*id), stock) >= 15. - 1e-8);
                for (other, _) in &roles[i + 1..] {
                    assert!(
                        gap(shifted(*id), shifted(*other)) >= 15. - 1e-8,
                        "Every occupied hardware interval must be visibly separated"
                    );
                }
            }
            if is_hanger {
                let id = |role| {
                    roles
                        .iter()
                        .find(|(_, candidate)| *candidate == role)
                        .unwrap()
                        .0
                };
                assert!(shifted(id(Role::HangerBolt))[1] < shifted(id(Role::Nut))[0]);
                assert!(shifted(id(Role::Nut))[1] < shifted(id(Role::NutWasher))[0]);
                let old_bolt = hardware[&id(Role::HangerBolt)].map(|value| value - 45.);
                let old_nut = hardware[&id(Role::Nut)].map(|value| value - 30.);
                assert!(
                    gap(old_bolt, old_nut) < -0.1,
                    "The original fixed offsets reproduce the reported hanger overlap"
                );
            } else {
                let screw = roles
                    .iter()
                    .find(|(_, role)| *role == Role::Screw)
                    .unwrap()
                    .0;
                assert!(
                    gap(hardware[&screw].map(|value| value - 15.), stock) < -0.1,
                    "The original fixed offset leaves the long screw in stock"
                );
            }
            let offsets = receipt
                .leaves
                .iter()
                .map(|leaf| {
                    (
                        leaf.path.clone(),
                        pose.vector([0., 0., shifts.get(&leaf.path.root).copied().unwrap_or(0.)]),
                    )
                })
                .collect();
            let presented = scene
                .presentation_leaf_offsets(&offsets, &BTreeMap::new())
                .unwrap();
            for receiver in &request.receivers {
                let display = presented
                    .display_leaf_paths()
                    .iter()
                    .find(|(_, path)| *path == &receiver.source.occurrence)
                    .unwrap()
                    .0;
                assert_eq!(
                    presented.instance_bounds(*display),
                    scene.leaf_display_bounds(&receiver.source.occurrence)
                );
            }
            assert_eq!(session.document(), &source);
        }
    }

    #[test]
    fn exploded_layout_rejects_visible_mixed_screw_and_hanger_roles() {
        use multicad_core::model::ScrewConnectionHardwareRole as Role;
        let roles = [Role::Screw, Role::HangerBolt].map(|role| (EntityId::new(), role));
        let bounds = roles.iter().map(|(id, _)| (*id, [0., 12.])).collect();
        assert!(exploded_role_stations(&roles, &bounds, [0., 4.1]).is_err());
        // A new draft role without a presented native leaf receives no invented offset.
        let shown = [(roles[1].0, [0., 12.])].into();
        let offsets = exploded_role_stations(&roles, &shown, [0., 4.1]).unwrap();
        assert!(!offsets.contains_key(&roles[0].0));
    }

    #[test]
    fn exploded_stack_separates_head_and_nut_sides_without_changing_authored_roles() {
        use multicad_core::model::ScrewConnectionHardwareRole as Role;
        let roles = [
            Role::NutWasher,
            Role::HeadWasher,
            Role::Screw,
            Role::HeadWasher,
            Role::NutWasher,
            Role::Nut,
        ]
        .map(|role| (EntityId::new(), role));
        let before = roles;
        let layout = |roles: &[(EntityId, Role)]| {
            let bounds = roles.iter().map(|(id, _)| (*id, [0., 1.])).collect();
            exploded_role_stations(roles, &bounds, [0., 10.]).unwrap()
        };
        let moved = layout(&roles);
        assert!(moved[&roles[2].0] < moved[&roles[1].0] && moved[&roles[1].0] < moved[&roles[3].0]);
        assert!(moved[&roles[3].0] < 0. && moved[&roles[0].0] > 0.);
        assert!(moved[&roles[0].0] < moved[&roles[4].0] && moved[&roles[4].0] < moved[&roles[5].0]);
        assert_eq!(roles, before);
        let hanger_roles = [
            Role::HangerBolt,
            Role::NutWasher,
            Role::NutWasher,
            Role::Nut,
        ]
        .map(|role| (EntityId::new(), role));
        let hanger = layout(&hanger_roles);
        assert!(hanger.values().all(|offset| *offset < 0.));
        assert!(hanger[&hanger_roles[0].0] < hanger[&hanger_roles[3].0]);
        assert!(hanger[&hanger_roles[3].0] < hanger[&hanger_roles[1].0]);
        assert!(hanger[&hanger_roles[1].0] < hanger[&hanger_roles[2].0]);
        let insert_roles =
            [Role::HeadWasher, Role::Screw, Role::Insert].map(|role| (EntityId::new(), role));
        let installed = layout(&insert_roles);
        assert!(installed[&insert_roles[1].0] < installed[&insert_roles[0].0]);
        assert!(
            installed[&insert_roles[0].0] < installed[&insert_roles[2].0]
                && installed[&insert_roles[2].0] < 0.
        );
    }
}
