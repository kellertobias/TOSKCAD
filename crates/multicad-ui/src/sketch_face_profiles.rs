//! Planar model faces as Extrude / Revolve inputs (CAD-146). A face selected
//! in the viewer before the command (preselection) or picked during input
//! acquisition (Shift / Command / Control adds or removes) becomes a
//! persistent profile face reference through the shared core capture. The
//! display guide (arrow start, outward normal, straight edges as revolution
//! axis candidates) is derived from the outward tessellation and is never
//! manufacturing geometry; native Preview proves the exact boundary.
use super::*;
use crate::viewport::{Hit, PickFilter, PickOptions, Scene};
use multicad_core::model::{FaceInput, capture_profile_face};

#[derive(Clone, Debug)]
pub(super) struct ProfileFacePick {
    pub face: FaceInput,
    pub stamp: crate::viewport::SceneStamp,
    /// World frame of the profile, +N along the current outward normal.
    pub frame: Frame,
    /// Display-only face centroid in that frame's U,V.
    pub anchor: [f64; 2],
    pub lines: Vec<crate::modeling::GuideLine>,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

/// Capture one viewer face hit as a profile face with its display guide.
pub(super) fn capture_hit(
    document: &Document,
    revision: u64,
    scene: &Scene,
    hit: &Hit,
) -> Result<ProfileFacePick, String> {
    if hit.stamp != scene.stamp()
        || hit.stamp.document != document.id()
        || hit.stamp.revision != revision
    {
        return Err("The selected face is stale; select it again on the current model".into());
    }
    if hit.key.kind != multicad_core::references::TopologyKind::Face {
        return Err("Select a planar model face".into());
    }
    if !matches!(
        document.get(hit.geometry).map(|e| &e.data),
        Some(EntityData::Feature {
            suppressed: false,
            ..
        })
    ) {
        return Err("Only faces of an active modeling feature can be extruded or revolved".into());
    }
    let snapshot = scene
        .geometry_snapshot(hit.geometry)
        .ok_or("The face source is unavailable")?;
    let face = capture_profile_face(&snapshot, hit.key, None)?;
    let anchor = &face
        .intent
        .attachment
        .as_ref()
        .ok_or("The face has no planar source frame")?
        .anchor;
    let local = snapshot
        .elements
        .iter()
        .find(|e| e.key == hit.key)
        .and_then(|e| e.frames.iter().find(|f| f.origin == *anchor))
        .ok_or("The face has no planar source frame")?
        .frame;
    let transform = scene
        .instances()
        .iter()
        .find(|i| i.id == hit.instance)
        .ok_or("The face occurrence is unavailable")?
        .transform;
    let world = transform.frame(local);
    let geometry = scene
        .producer_geometry(hit.geometry)
        .ok_or("The face display geometry is unavailable")?;
    let (normal, centroid) = geometry
        .face_outward(hit.key.ordinal)
        .ok_or("The face display geometry is unavailable")?;
    let outward = transform.vector(normal);
    let centroid = transform.point(centroid);
    let n = world.normal();
    let alignment = dot(n, outward);
    if alignment.abs() < 0.999 {
        return Err("This face is curved; Extrude and Revolve need a planar face".into());
    }
    let flip = if alignment < 0. { -1. } else { 1. };
    let frame = Frame {
        origin: world.origin_mm,
        u: world.u,
        v: world.v.map(|v| v * flip),
        n: n.map(|v| v * flip),
    };
    let uv = |p: [f64; 3]| {
        let d = sub(p, frame.origin);
        [dot(d, frame.u), dot(d, frame.v)]
    };
    let lines = geometry
        .face_straight_edges(hit.key.ordinal)
        .into_iter()
        .map(|[a, b]| crate::modeling::GuideLine {
            id: EntityId::new(),
            a: uv(transform.point(a)),
            b: uv(transform.point(b)),
        })
        .collect();
    Ok(ProfileFacePick {
        face,
        stamp: scene.stamp(),
        frame,
        anchor: uv(centroid),
        lines,
    })
}

impl SketchEditor {
    /// A planar face selected in the viewer before Extrude / Revolve.
    pub(super) fn adopt_preselected_profile_face(&mut self, document: &Document, revision: u64) {
        self.profile_faces.clear();
        let Some(hit) = self.selected_face_hit() else {
            return;
        };
        let Some(scene) = self.display_scene.clone() else {
            return;
        };
        match capture_hit(document, revision, &scene, &hit) {
            Ok(pick) => self.profile_faces.push(pick),
            Err(error) => self.message = error,
        }
    }
    /// Current, unique face inputs ready for a face-profile operation.
    pub(super) fn profile_face_inputs(&self, revision: u64) -> Result<Vec<FaceInput>, String> {
        if self.profile_faces.is_empty() {
            return Err("Select one or more planar model faces".into());
        }
        if self
            .profile_faces
            .iter()
            .any(|p| p.stamp.revision != revision)
        {
            return Err("The selected faces are stale; select them again".into());
        }
        Ok(self.profile_faces.iter().map(|p| p.face.clone()).collect())
    }
    /// Display guide of the first face: arrow start, outward normal and its
    /// straight edges as explicit revolution axis candidates.
    pub(super) fn profile_face_guide(&self) -> Option<crate::modeling::ProfileGuide> {
        let first = self.profile_faces.first()?;
        Some(crate::modeling::ProfileGuide {
            sketch: first.face.source,
            frame: first.frame,
            anchor: first.anchor,
            lines: first.lines.clone(),
        })
    }
    /// Open Extrude / Revolve editing directly on the accepted faces. No
    /// feature is created until the operation's Apply or Enter commits.
    pub(super) fn begin_face_profile_model(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        tool: crate::modeling::ModelingTool,
        channel: Channel,
    ) -> Result<(), String> {
        if self
            .profile_inputs(session.document(), session.revision())
            .is_ok()
        {
            return Err(
                "Use either closed areas of one sketch or planar model faces in one operation; clear one of the selections"
                    .into(),
            );
        }
        let faces = self.profile_face_inputs(session.revision())?;
        let guide = self.profile_face_guide();
        self.finish()?;
        match tool {
            crate::modeling::ModelingTool::Extrude => {
                self.model
                    .begin_face_extrude(session, resources, faces.clone(), channel)?;
                if let Some(scene) = self.display_scene.as_deref() {
                    let _ = self.model.set_extrusion_context(session, resources, scene);
                }
            }
            crate::modeling::ModelingTool::Revolve => {
                self.model
                    .begin_face_revolve(session, resources, faces.clone(), channel)?
            }
            _ => return Err("Choose a profile modeling tool".into()),
        }
        let count = faces.len();
        self.message = match guide {
            Some(guide) => {
                self.model.set_profile_guide(guide);
                if tool == crate::modeling::ModelingTool::Revolve {
                    format!(
                        "Revolve · {count} planar face(s) · choose the axis (click a straight face edge or use the Inspector), then drag the angle handle or type an angle and press Enter"
                    )
                } else {
                    format!(
                        "Extrude · {count} planar face(s) · drag the arrow or type a signed distance (negative goes into the body), then press Enter"
                    )
                }
            }
            None => format!(
                "{} · {count} planar face(s) · use the Inspector, then Preview",
                tool.label()
            ),
        };
        Ok(())
    }
    /// Acquisition click without a selected sketch: pick a planar face.
    /// Shift, Command or Control adds or removes; a plain click replaces.
    pub(super) fn pick_profile_face(
        &mut self,
        session: &Session,
        scene: &Scene,
        rect: Rect,
        pointer: Pos2,
        toggle: bool,
    ) {
        let hit = scene.pick_screen_with(
            self.camera,
            rect,
            pointer,
            PickOptions {
                filter: PickFilter::Faces,
                include_hidden: false,
                section: self.solid.section.plane(),
            },
        );
        let Some(hit) = hit else {
            if !toggle {
                self.profile_faces.clear();
            }
            return;
        };
        let pick = match capture_hit(session.document(), session.revision(), scene, &hit) {
            Ok(pick) => pick,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let same = |p: &ProfileFacePick| {
            p.face.source == pick.face.source
                && p.face.intent.origins == pick.face.intent.origins
                && p.face.intent.boundary == pick.face.intent.boundary
        };
        if toggle {
            if let Some(index) = self.profile_faces.iter().position(same) {
                self.profile_faces.remove(index);
            } else {
                self.profile_faces.push(pick);
            }
        } else {
            self.profile_faces = vec![pick];
        }
        self.message = format!(
            "{} planar face(s) selected · Shift, Command or Control adds or removes · Use selected targets opens the operation",
            self.profile_faces.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        modeling::{ModelRequest, ModelingTool, native_fixture::NativeBackend},
        viewport::SceneFeed,
    };
    use multicad_core::model::{
        self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
    };
    use std::time::{Duration, Instant};

    const SIZE: Vec2 = Vec2::new(1000., 700.);

    /// Two 30 x 30 x 10 boxes at x = 0 and x = 65 with a current native scene.
    fn fixture() -> (Session, SceneFeed, SketchEditor) {
        let mut session = Session::new(Document::new("Face profiles").unwrap()).unwrap();
        for i in 0..2 {
            let request = PrimitiveRequest::new(
                format!("Box {}", i + 1),
                PrimitiveParameters::Box {
                    size: [length(30.), length(30.), length(10.)],
                    frame: PrimitiveFrame {
                        origin: [length(i as f64 * 65.), length(0.), length(0.)],
                        ..Default::default()
                    },
                },
            );
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
        let until = Instant::now() + Duration::from_secs(10);
        while !feed.is_current(session.document().id(), session.revision())
            && Instant::now() < until
        {
            feed.tick(session.document(), session.revision());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(feed.is_current(session.document().id(), session.revision()));
        let mut editor = SketchEditor {
            display_scene: feed.visible().cloned(),
            ..Default::default()
        };
        editor.set_model_backend(Box::new(NativeBackend::new()));
        editor.camera.target = [47., 15., 5.];
        editor.camera.pitch = std::f32::consts::FRAC_PI_2;
        editor.camera.yaw = 0.;
        (session, feed, editor)
    }
    fn screen(editor: &SketchEditor, p: [f64; 3]) -> Pos2 {
        let rect = Rect::from_min_size(Pos2::ZERO, SIZE);
        let s = editor.camera.project(p.map(|v| v as f32), rect);
        Pos2::new(s[0], s[1])
    }
    fn raw(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            events,
            ..Default::default()
        }
    }
    fn press_release(pos: Pos2, modifiers: egui::Modifiers) -> [Vec<egui::Event>; 2] {
        [
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
        ]
    }
    /// Select a face in the viewer outside any tool (real canvas picking).
    fn preselect(editor: &mut SketchEditor, session: &mut Session, feed: &SceneFeed, p: [f64; 3]) {
        let ctx = egui::Context::default();
        let pos = screen(editor, p);
        let [down, up] = press_release(pos, egui::Modifiers::NONE);
        for events in [vec![], vec![], down, up] {
            ctx.run_ui(raw(events), |ui| {
                editor.canvas(ui, session, false, feed.visible().cloned(), true)
            })
            .drop_without_applying_deltas();
        }
    }
    fn activate(
        editor: &mut SketchEditor,
        session: &Session,
        feed: &SceneFeed,
        tool: ModelingTool,
    ) {
        editor
            .activate_model_tool(session, &Default::default(), feed, tool, Channel::Ui)
            .unwrap();
    }
    fn acquire_click(
        editor: &mut SketchEditor,
        session: &Session,
        feed: &SceneFeed,
        p: [f64; 3],
        ctrl: bool,
    ) {
        let rect = Rect::from_min_size(Pos2::ZERO, SIZE);
        let ctx = egui::Context::default();
        let modifiers = egui::Modifiers {
            ctrl,
            ..Default::default()
        };
        let [down, up] = press_release(screen(editor, p), modifiers);
        for events in [vec![], down, up] {
            ctx.run_ui(raw(events), |ui| {
                let (_, response) = ui.allocate_exact_size(SIZE, Sense::click_and_drag());
                editor.target_viewport(ui, &response, session, &Default::default(), feed, rect);
            })
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn preselected_planar_face_opens_extrude_and_revolve_editing_immediately() {
        for tool in [ModelingTool::Extrude, ModelingTool::Revolve] {
            let (mut session, feed, mut editor) = fixture();
            let before = session.document().clone();
            let revision = session.revision();
            preselect(&mut editor, &mut session, &feed, [15., 15., 10.]);
            let hit = editor.selected_face_hit().expect("real face selection");
            activate(&mut editor, &session, &feed, tool);
            assert!(editor.model_active(), "{tool:?}: {}", editor.message);
            assert!(!editor.awaiting_model_targets());
            assert!(editor.model.operation_gizmo_active(), "{}", editor.message);
            assert!(editor.message.contains("planar face"), "{}", editor.message);
            match editor.model_request_for_tests(&session) {
                Some(ModelRequest::Extrude(request)) => {
                    assert_eq!(request.outputs.len(), 1);
                    let face = request.outputs[0].face.as_ref().expect("face input");
                    assert_eq!(face.source, hit.geometry);
                }
                Some(other) => panic!("unexpected {other:?}"),
                // Revolve has no request until its axis is chosen explicitly.
                None => assert_eq!(tool, ModelingTool::Revolve),
            }
            assert_eq!(session.revision(), revision);
            editor.cancel_model();
            assert!(editor.profile_faces.is_empty());
            assert_eq!(session.document(), &before);
            assert_eq!(session.revision(), revision);
        }
    }

    #[test]
    fn acquisition_control_click_accumulates_faces_and_apply_opens_editing_then_commits() {
        let (mut session, feed, mut editor) = fixture();
        let before = session.document().clone();
        let revision = session.revision();
        activate(&mut editor, &session, &feed, ModelingTool::Extrude);
        assert!(editor.awaiting_model_targets(), "{}", editor.message);
        // Plain click picks one top face, Control-click adds the other.
        acquire_click(&mut editor, &session, &feed, [15., 15., 10.], false);
        acquire_click(&mut editor, &session, &feed, [80., 15., 10.], true);
        assert_eq!(editor.profile_faces.len(), 2, "{}", editor.message);
        // A second Control-click on the same face removes it; a third adds it.
        acquire_click(&mut editor, &session, &feed, [80., 15., 10.], true);
        assert_eq!(editor.profile_faces.len(), 1, "{}", editor.message);
        acquire_click(&mut editor, &session, &feed, [80., 15., 10.], true);
        assert_eq!(editor.profile_faces.len(), 2, "{}", editor.message);
        // Viewer selection alone never opens the operation.
        assert!(editor.awaiting_model_targets(), "{}", editor.message);
        // Apply confirms the input set only: editing opens, no feature yet.
        editor.advance_model_targets(&session, &Default::default(), &feed, true);
        assert!(editor.model_active(), "{}", editor.message);
        assert_eq!(session.revision(), revision);
        assert_eq!(session.document(), &before);
        match editor.model_request_for_tests(&session) {
            Some(ModelRequest::Extrude(request)) => {
                assert_eq!(request.outputs.len(), 2);
                assert!(request.outputs.iter().all(|o| o.face.is_some()));
            }
            other => panic!("{other:?}"),
        }
        // Preview and Apply commit both face extrusions in one transaction.
        editor
            .preview_model(&session, &Default::default(), Channel::Ui)
            .unwrap();
        let until = Instant::now() + Duration::from_secs(20);
        while editor.model.pending() && Instant::now() < until {
            editor.model.poll(&session, &Default::default());
            std::thread::sleep(Duration::from_millis(2));
        }
        editor
            .apply_model(&mut session, &Default::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(session.revision(), revision + 1);
        let compiled = multicad_model::Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let faces: Vec<_> = session
            .document()
            .entities()
            .filter_map(|e| {
                model::read_extrude(session.document(), e.id)
                    .ok()
                    .filter(|s| s.profile.face().is_some())
                    .map(|_| e.id)
            })
            .collect();
        assert_eq!(faces.len(), 2);
        for feature in faces {
            let shape = &compiled.snapshot().shapes[&feature];
            assert!((shape.volume_mm3 - 30. * 30. * 10.).abs() < 1e-4);
            assert!((shape.bounds.min[2] - 10.).abs() < 1e-6);
            assert!((shape.bounds.max[2] - 20.).abs() < 1e-6);
        }
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
    }
}
