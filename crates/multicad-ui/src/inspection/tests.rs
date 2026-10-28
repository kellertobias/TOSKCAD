use super::*;
use crate::viewport::{DisplayEdge, DisplayPoint, Geometry, Instance};
use multicad_core::{
    document::Mutation,
    model::{self, ModelEvaluationInfo, PrimitiveParameters, PrimitiveRequest, length},
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, inspection::measure_model};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Default)]
struct State {
    requests: Vec<(u64, MeasurementStamp, Option<MeasurementQuery>)>,
    replies: VecDeque<MeasurementReply>,
    cancel_count: usize,
}
struct NativeBackend(Arc<Mutex<State>>);
impl MeasurementBackend for NativeBackend {
    fn request(
        &mut self,
        d: &Document,
        stamp: MeasurementStamp,
        q: Option<&MeasurementQuery>,
    ) -> Result<u64, String> {
        stamp.check(d, stamp.revision)?;
        let m = Compiler::default()
            .compile(d, stamp.revision)
            .map_err(|e| e.to_string())?;
        let pins = multicad_core::inspection::pins(d)
            .map(|id| {
                let pin = multicad_core::inspection::read(d, id).unwrap();
                (id, measure_model(&m, &pin.query))
            })
            .collect();
        let mut state = self.0.lock().unwrap();
        let id = state.requests.len() as u64 + 1;
        state.requests.push((id, stamp, q.cloned()));
        state.replies.push_back(MeasurementReply {
            request: id,
            stamp,
            result: Ok(MeasurementBatch {
                stamp,
                transient: q.map(|q| measure_model(&m, q)),
                pinned: pins,
            }),
        });
        Ok(id)
    }
    fn poll(&mut self) -> Option<MeasurementReply> {
        self.0.lock().unwrap().replies.pop_front()
    }
    fn cancel(&mut self) {
        self.0.lock().unwrap().cancel_count += 1;
    }
}
fn inspector() -> (MeasurementInspector, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let shared = state.clone();
    let mut m = MeasurementInspector::default();
    m.set_factory(Arc::new(move || Box::new(NativeBackend(shared.clone()))));
    m.open();
    (m, state)
}
fn fixture() -> (Session, PrimitiveRequest) {
    let mut s = Session::new(Document::new("Bench clearance").unwrap()).unwrap();
    let r = PrimitiveRequest::new(
        "Bench",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(10.)],
            frame: Default::default(),
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), Actor::local(Channel::Ui), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    (s, r)
}
fn native_scene(s: &Session) -> Arc<Scene> {
    let m = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let snapshot = m.snapshot();
    let geometries = snapshot
        .shapes
        .iter()
        .map(|(id, shape)| {
            Arc::new(
                Geometry::new(
                    *id,
                    shape.mesh.vertices.as_chunks::<3>().0.to_vec(),
                    shape.mesh.indices.clone(),
                    shape.mesh.face_indices.clone(),
                    shape.topology.elements.clone(),
                )
                .unwrap()
                .with_topology_display(
                    shape
                        .edges
                        .iter()
                        .map(|e| DisplayEdge {
                            index: e.index,
                            points_mm: e.points_mm.clone(),
                        })
                        .collect(),
                    shape
                        .vertices
                        .iter()
                        .map(|p| DisplayPoint {
                            index: p.index,
                            position_mm: p.position_mm,
                        })
                        .collect(),
                    0.1,
                )
                .unwrap(),
            )
        })
        .collect();
    let instances = snapshot
        .bodies
        .iter()
        .map(|(body, producer)| Instance {
            id: *body,
            geometry: *producer,
            transform: Default::default(),
            color: [140, 170, 200, 255],
        })
        .collect();
    Arc::new(
        Scene::new(
            SceneStamp {
                document: s.document().id(),
                revision: s.revision(),
                generation: 1,
            },
            geometries,
            instances,
        )
        .unwrap()
        .with_reference_aliases(snapshot.body_sources.clone())
        .unwrap()
        .with_model_evaluation(Arc::new(ModelEvaluationInfo {
            source_hash: snapshot.stamp.source_hash,
            consumed_bodies: snapshot.consumed_bodies.clone(),
            cut_targets: snapshot.cut_targets.clone(),
        }))
        .unwrap(),
    )
}
fn camera(above: bool) -> Camera {
    Camera {
        yaw: 0.,
        pitch: if above {
            std::f32::consts::FRAC_PI_2
        } else {
            -std::f32::consts::FRAC_PI_2
        },
        scale: 20.,
        target: [5., 5., 5.],
        perspective: false,
    }
}
fn pick(m: &mut MeasurementInspector, s: &Session, scene: &Arc<Scene>, c: Camera, p: [f32; 3]) {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
    let p = c.project(p, rect);
    m.pointer(
        s,
        MeasurementViewport {
            scene: Some(scene),
            camera: c,
            rect,
            pointer: Some(egui::pos2(p[0], p[1])),
            clicked: true,
            section: None,
        },
    );
}
fn ready(m: &mut MeasurementInspector, s: &Session) {
    m.tick(s);
    m.tick(s);
    assert!(m.ready(s).is_some(), "{:?}", m.feed.error);
}
fn same_part_faces(m: &mut MeasurementInspector, s: &Session, scene: &Arc<Scene>) {
    m.tick(s);
    pick(m, s, scene, camera(true), [5., 5., 10.]);
    pick(m, s, scene, camera(false), [5., 5., 0.]);
    ready(m, s);
}
fn frame(
    m: &mut MeasurementInspector,
    s: &mut Session,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    m.tick(s);
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600., 1200.),
            )),
            events,
            ..Default::default()
        },
        |ui| m.show(ui, s, true),
    )
}
fn click(m: &mut MeasurementInspector, s: &mut Session, ctx: &egui::Context, label: &str) {
    frame(m, s, ctx, vec![]).drop_without_applying_deltas();
    let out = frame(m, s, ctx, vec![]);
    let p = out
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            m,
            s,
            ctx,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
#[test]
fn actual_same_part_face_picks_are_distinct_read_only_and_pin_is_undoable_metadata() {
    let (mut s, r) = fixture();
    let before = s.document().clone();
    let scene = native_scene(&s);
    let (mut m, _) = inspector();
    same_part_faces(&mut m, &s, &scene);
    let values = m.ready(&s).unwrap();
    assert!((values.minimum_mm - 10.).abs() < 1e-8);
    assert_eq!(values.axis_components_mm, Some([0., 0., -10.]));
    assert_eq!(m.refs[0].as_ref().unwrap().source, r.body);
    assert_eq!(m.refs[1].as_ref().unwrap().source, r.body);
    assert!(
        matches!(m.highlights(&s,&scene),Some(Selection::References([Some(a),Some(b)])) if a.instance==b.instance && a.key!=b.key)
    );
    assert_eq!(s.document(), &before);
    assert!(m.pin(&mut s, false, Channel::Ui).is_err());
    assert_eq!(s.document(), &before);
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    click(&mut m, &mut s, &ctx, "Pin measurement");
    let pin = m.selected_pin.unwrap();
    assert_eq!(multicad_core::inspection::pins(s.document()).count(), 1);
    assert_eq!(s.document().get(r.body), before.get(r.body));
    assert_eq!(s.document().get(r.feature), before.get(r.feature));
    let file = ProjectFile::new(s.document().clone()).unwrap();
    let mut bytes = Vec::new();
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let doc = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let reopened = Session::new(doc).unwrap();
    let native = Compiler::default().compile(reopened.document(), 0).unwrap();
    let definition = multicad_core::inspection::read(reopened.document(), pin).unwrap();
    assert_eq!(
        measure_model(&native, &definition.query)
            .result
            .unwrap()
            .minimum_mm,
        10.
    );
    s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert!(s.document().get(pin).is_none());
    s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert!(s.document().get(pin).is_some());
    ready(&mut m, &s);
    m.name = "Stock thickness".into();
    click(&mut m, &mut s, &ctx, "Update pinned measurement");
    assert_eq!(
        multicad_core::inspection::read(s.document(), pin)
            .unwrap()
            .name,
        "Stock thickness"
    );
    ready(&mut m, &s);
    click(&mut m, &mut s, &ctx, "Remove pin");
    assert!(s.document().get(pin).is_none());
    s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert!(s.document().get(pin).is_some());
}
#[test]
fn replacement_keeps_other_reference_and_stale_or_partial_scenes_cannot_pick_or_pin() {
    let (mut s, r) = fixture();
    let scene = native_scene(&s);
    let (mut m, _) = inspector();
    same_part_faces(&mut m, &s, &scene);
    let a = m.refs[0].as_ref().unwrap().intent.id;
    let b = m.refs[1].clone();
    m.picking = Some(Side::A);
    pick(
        &mut m,
        &s,
        &scene,
        Camera {
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: 0.,
            ..camera(true)
        },
        [10., 5., 5.],
    );
    assert_eq!(m.refs[0].as_ref().unwrap().intent.id, a);
    assert_eq!(m.refs[1], b);
    ready(&mut m, &s);
    assert_ne!(m.ready(&s).unwrap().key_a, m.ready(&s).unwrap().key_b);
    let refs = m.refs.clone();
    m.picking = Some(Side::A);
    let partial = Arc::new(
        Arc::try_unwrap(native_scene(&s))
            .ok()
            .unwrap()
            .with_source_scope(vec![r.feature])
            .unwrap(),
    );
    pick(&mut m, &s, &partial, camera(true), [5., 5., 10.]);
    assert_eq!(m.refs, refs);
    assert!(m.error.as_ref().unwrap().contains("complete"));
    s.commit(
        model::prepare_primitive_edit(
            &s,
            s.revision(),
            Actor::local(Channel::Ui),
            r.feature,
            &PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(14.)],
                frame: Default::default(),
            },
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    pick(&mut m, &s, &scene, camera(true), [5., 5., 10.]);
    assert_eq!(m.refs, refs);
    assert!(m.pin(&mut s, true, Channel::Ui).is_err());
}
#[test]
fn close_hides_temporary_overlay_but_regenerates_visible_pins_and_reopen_is_current() {
    let (mut s, _) = fixture();
    let scene = native_scene(&s);
    let (mut m, state) = inspector();
    same_part_faces(&mut m, &s, &scene);
    m.pin(&mut s, true, Channel::Ui).unwrap();
    ready(&mut m, &s);
    let revision = s.revision();
    m.close();
    m.tick(&s);
    m.tick(&s);
    assert!(m.ready(&s).is_none());
    assert!(state.lock().unwrap().requests.last().unwrap().2.is_none());
    assert!(m.feed.batch.as_ref().unwrap().transient.is_none());
    assert_eq!(m.feed.batch.as_ref().unwrap().pinned.len(), 1);
    assert_eq!(s.revision(), revision);
    m.open();
    ready(&mut m, &s);
    assert!(m.ready(&s).is_some());
    m.reset();
    m.close();
    m.tick(&s);
    assert_eq!(s.revision(), revision);
}
#[test]
fn actual_edge_and_point_picking_preserves_native_reference_kinds() {
    let (s, _) = fixture();
    let scene = native_scene(&s);
    for (filter, kind, a, b) in [
        (
            PickFilter::Edges,
            TopologyKind::Edge,
            [5., 0., 10.],
            [5., 10., 10.],
        ),
        (
            PickFilter::Vertices,
            TopologyKind::Vertex,
            [0., 0., 10.],
            [10., 10., 10.],
        ),
    ] {
        let (mut m, _) = inspector();
        m.filter = filter;
        m.tick(&s);
        pick(&mut m, &s, &scene, camera(true), a);
        pick(&mut m, &s, &scene, camera(true), b);
        ready(&mut m, &s);
        assert_eq!(m.refs[0].as_ref().unwrap().intent.kind, kind);
        assert_eq!(m.refs[1].as_ref().unwrap().intent.kind, kind);
        let expected = if kind == TopologyKind::Edge {
            10.
        } else {
            200_f64.sqrt()
        };
        assert!((m.ready(&s).unwrap().minimum_mm - expected).abs() < 1e-6);
    }
}
#[test]
fn source_edits_update_pins_and_removed_sources_show_missing_without_stale_numbers() {
    let (mut s, r) = fixture();
    let scene = native_scene(&s);
    let (mut m, _) = inspector();
    same_part_faces(&mut m, &s, &scene);
    m.pin(&mut s, true, Channel::Ui).unwrap();
    let id = m.selected_pin.unwrap();
    ready(&mut m, &s);
    s.commit(
        model::prepare_primitive_edit(
            &s,
            s.revision(),
            Actor::local(Channel::Ui),
            r.feature,
            &PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(14.)],
                frame: Default::default(),
            },
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    assert!(m.ready(&s).is_none());
    ready(&mut m, &s);
    assert_eq!(m.ready(&s).unwrap().minimum_mm, 14.);
    assert_eq!(
        m.feed.batch.as_ref().unwrap().pinned[&id]
            .result
            .as_ref()
            .unwrap()
            .minimum_mm,
        14.
    );
    s.commit(
        s.prepare(multicad_core::commands::Request {
            expected_revision: s.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Remove source".into(),
            edits: vec![Mutation::Remove(r.body), Mutation::Remove(r.feature)],
        })
        .unwrap(),
    )
    .unwrap();
    m.tick(&s);
    m.tick(&s);
    assert!(m.ready(&s).is_none());
    assert!(m.feed.batch.as_ref().unwrap().pinned[&id].result.is_err());
    assert!(m.pin(&mut s, true, Channel::Ui).is_err());
}
#[test]
fn a_pin_changed_elsewhere_requires_explicit_reload_before_update() {
    let (mut s, _) = fixture();
    let scene = native_scene(&s);
    let (mut m, _) = inspector();
    same_part_faces(&mut m, &s, &scene);
    m.pin(&mut s, true, Channel::Ui).unwrap();
    let id = m.selected_pin.unwrap();
    ready(&mut m, &s);
    let mut edited = s.document().get(id).unwrap().clone();
    edited.name = "Changed elsewhere".into();
    s.commit(
        s.prepare(multicad_core::commands::Request {
            expected_revision: s.revision(),
            actor: Actor::local(Channel::Mcp),
            label: "Remote label".into(),
            edits: vec![Mutation::Replace(edited)],
        })
        .unwrap(),
    )
    .unwrap();
    ready(&mut m, &s);
    assert!(
        m.pin(&mut s, true, Channel::Ui)
            .unwrap_err()
            .contains("changed")
    );
}
#[test]
fn foreign_stale_and_malformed_publications_never_enable_pin() {
    for corruption in 0..4 {
        let (s, _) = fixture();
        let scene = native_scene(&s);
        let (mut m, state) = inspector();
        same_part_faces(&mut m, &s, &scene);
        m.feed.retry();
        m.tick(&s);
        let mut replies = state.lock().unwrap();
        let reply = replies.replies.back_mut().unwrap();
        match corruption {
            0 => reply.stamp.source_hash = multicad_core::project_file::ContentHash::of(b"foreign"),
            1 => {
                reply
                    .result
                    .as_mut()
                    .unwrap()
                    .transient
                    .as_mut()
                    .unwrap()
                    .query
                    .b
                    .intent
                    .id = EntityId::new()
            }
            2 => {
                reply
                    .result
                    .as_mut()
                    .unwrap()
                    .transient
                    .as_mut()
                    .unwrap()
                    .result
                    .as_mut()
                    .unwrap()
                    .minimum_mm = -1.
            }
            _ => {
                reply
                    .result
                    .as_mut()
                    .unwrap()
                    .transient
                    .as_mut()
                    .unwrap()
                    .result
                    .as_mut()
                    .unwrap()
                    .axis_components_mm = Some([0., 0., 10.])
            }
        }
        drop(replies);
        m.tick(&s);
        assert!(m.ready(&s).is_none());
        assert!(m.feed.error.is_some());
    }
}
#[test]
fn obsolete_request_is_ignored_and_missing_pin_publication_is_rejected() {
    let (mut s, _) = fixture();
    let scene = native_scene(&s);
    let (mut m, state) = inspector();
    same_part_faces(&mut m, &s, &scene);
    m.feed.retry();
    m.tick(&s);
    let correct = state.lock().unwrap().replies.pop_back().unwrap();
    let mut old = correct.clone();
    old.request = 999;
    state.lock().unwrap().replies.push_back(old);
    m.tick(&s);
    assert!(m.ready(&s).is_none());
    assert!(m.feed.pending());
    state.lock().unwrap().replies.push_back(correct);
    m.tick(&s);
    assert!(m.ready(&s).is_some());
    m.pin(&mut s, true, Channel::Ui).unwrap();
    ready(&mut m, &s);
    m.feed.retry();
    m.tick(&s);
    state
        .lock()
        .unwrap()
        .replies
        .back_mut()
        .unwrap()
        .result
        .as_mut()
        .unwrap()
        .pinned
        .clear();
    m.tick(&s);
    assert!(m.ready(&s).is_none());
    assert!(m.feed.error.as_ref().unwrap().contains("omits"));
}
#[test]
fn inspector_width_stays_bounded_and_replacing_a_side_is_an_actual_ui_action() {
    let (mut s, _) = fixture();
    let scene = native_scene(&s);
    let (mut m, _) = inspector();
    same_part_faces(&mut m, &s, &scene);
    let before = s.document().clone();
    let b = m.refs[1].clone();
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    for _ in 0..60 {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1300., 1000.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::Panel::right("measurement-width")
                    .default_size(250.)
                    .resizable(true)
                    .show(ui, |ui| {
                        crate::controls::inspector_layout(ui);
                        let width = ui.available_width();
                        m.show(ui, &mut s, true);
                        assert!(
                            ui.min_rect().width() <= width + 0.5,
                            "{} > {width}",
                            ui.min_rect().width()
                        );
                    });
            },
        )
        .drop_without_applying_deltas();
    }
    click(&mut m, &mut s, &ctx, "Replace A");
    assert_eq!(m.picking, Some(Side::A));
    assert_eq!(m.refs[1], b);
    assert_eq!(s.document(), &before);
    click(&mut m, &mut s, &ctx, "Cancel picking");
    assert!(m.picking.is_none());
    assert!(m.ready(&s).is_some());
    click(&mut m, &mut s, &ctx, "Close Measure");
    assert!(!m.active());
    assert_eq!(s.document(), &before);
}
#[test]
fn opening_empty_inspection_does_not_compile_an_unused_second_native_model() {
    let (s, _) = fixture();
    let (mut m, state) = inspector();
    for _ in 0..30 {
        m.tick(&s);
    }
    assert!(state.lock().unwrap().requests.is_empty());
    assert!(!m.pending());
    assert!(m.feed.stamp.is_some());
    let scene = native_scene(&s);
    same_part_faces(&mut m, &s, &scene);
    assert_eq!(state.lock().unwrap().requests.len(), 1);
}
