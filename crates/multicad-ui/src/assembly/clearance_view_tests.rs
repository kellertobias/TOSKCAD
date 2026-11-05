//! CAD-171: viewport evidence for Interference/Clearance results, using real
//! compiled solids, the production inspection worker and actual egui frames.
use super::*;
use crate::{
    assembly::clearance_view::{A_COLOR, B_COLOR, relation_style},
    camera::Camera,
    modeling::native_fixture,
    viewport::{Selection, VisibilityFilter},
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    hierarchy,
    model::*,
};
use multicad_model::{Compiler, assembly_inspection::AssemblyInspectionWorker};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

struct WorkerBackend(AssemblyInspectionWorker);
impl AssemblyInspectionBackend for WorkerBackend {
    fn request(
        &mut self,
        document: &Document,
        query: &AssemblyInspectionQuery,
    ) -> Result<u64, String> {
        self.0.submit(document, query.clone())
    }
    fn poll(&mut self) -> Option<AssemblyInspectionReply> {
        self.0.poll()
    }
    fn cancel(&mut self) {
        self.0.cancel();
    }
}
fn factory() -> AssemblyInspectionBackendFactory {
    Arc::new(|| {
        Box::new(WorkerBackend(
            AssemblyInspectionWorker::new(Compiler::default()).unwrap(),
        ))
    })
}
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
struct Fixture {
    session: Session,
    part: EntityId,
    instance: EntityId,
}
/// A 10 x 20 x 30 mm box at the origin and a placed copy translated by `at`.
fn fixture(at: [f64; 3]) -> Fixture {
    let mut session = Session::new(Document::new("Clearance witnesses").unwrap()).unwrap();
    let part = PrimitiveRequest::new(
        "Panel",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, actor(), &part)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let mut placed = ComponentPlacementRequest::new(part.body, Default::default());
    placed.parameters.translation = at.map(length);
    session
        .commit(
            prepare_place_component(&session, session.revision(), actor(), &placed)
                .unwrap()
                .prepared,
        )
        .unwrap();
    Fixture {
        session,
        part: part.body,
        instance: placed.instance,
    }
}
fn scene(session: &Session) -> Arc<Scene> {
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene =
        Arc::try_unwrap(native_fixture::scene(session.document(), model.snapshot(), 1).unwrap())
            .ok()
            .unwrap()
            .with_assembly_evaluation(Arc::new(model.assembly_evaluation()))
            .unwrap();
    let scene = Arc::new(scene);
    VisibilityFilter::default()
        .apply(&scene, session.document(), session.revision())
        .unwrap()
}
fn selection(f: &Fixture) -> AssemblySelection {
    AssemblySelection {
        occurrences: vec![
            OccurrencePath {
                root: f.part,
                members: vec![],
            },
            OccurrencePath {
                root: f.instance,
                members: vec![],
            },
        ],
        ..Default::default()
    }
}
fn inspected(f: &Fixture, scene: &Arc<Scene>) -> InspectionPanel {
    let factory = factory();
    let mut panel = InspectionPanel::new(
        false,
        f.session.document(),
        Some(scene),
        &selection(f),
        Some(&factory),
    );
    panel.request(&f.session).unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    while panel.pending {
        panel.poll(&f.session);
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(panel.error.is_none(), "{:?}", panel.error);
    panel
}
fn rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.))
}
fn frame(
    panel: &mut InspectionPanel,
    session: &Session,
    scene: &Arc<Scene>,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600., 1200.),
            )),
            events,
            ..Default::default()
        },
        |ui| panel.show(ui, session, Some(scene)),
    )
}
/// Text of one frame; consumes the output so its texture deltas are handled.
fn texts(out: egui::FullOutput) -> Vec<String> {
    let texts = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    out.drop_without_applying_deltas();
    texts
}
fn click(
    panel: &mut InspectionPanel,
    session: &Session,
    scene: &Arc<Scene>,
    ctx: &egui::Context,
    label: &str,
) {
    frame(panel, session, scene, ctx, vec![]).drop_without_applying_deltas();
    let out = frame(panel, session, scene, ctx, vec![]);
    let p = out
        .shapes
        .iter()
        .rev()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            panel,
            session,
            scene,
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
/// Paint one marker into a real egui frame and return its shapes.
fn painted(marker: &ClearanceMarker, camera: Camera, reveal: bool) -> Vec<egui::Shape> {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect()),
            ..Default::default()
        },
        |ui| marker.paint(ui.painter(), rect(), camera, reveal, None),
    );
    let clipped = std::mem::take(&mut out.shapes);
    out.drop_without_applying_deltas();
    let mut shapes = vec![];
    fn flatten(shape: egui::Shape, into: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(inner) => inner.into_iter().for_each(|s| flatten(s, into)),
            other => into.push(other),
        }
    }
    for clipped in clipped {
        flatten(clipped.shape, &mut shapes);
    }
    shapes
}
fn shape_texts(shapes: &[egui::Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}
fn filled_circles(shapes: &[egui::Shape], color: egui::Color32) -> usize {
    shapes
        .iter()
        .filter(|s| matches!(s, egui::Shape::Circle(c) if c.fill == color))
        .count()
}
fn ring(shapes: &[egui::Shape], color: egui::Color32) -> bool {
    shapes
        .iter()
        .any(|s| matches!(s, egui::Shape::Circle(c) if c.stroke.color == color && c.fill == egui::Color32::TRANSPARENT))
}
fn segments(shapes: &[egui::Shape], color: egui::Color32) -> usize {
    shapes
        .iter()
        .filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == color))
        .count()
}
fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
}

#[test]
fn selected_result_marks_both_occurrences_exact_witnesses_and_labeled_distance() {
    let f = fixture([40., 20., 5.]);
    let source = f.session.document().clone();
    let revision = f.session.revision();
    let scene = scene(&f.session);
    let panel = inspected(&f, &scene);
    let Some(AssemblyInspectionReport::Current(report)) = &panel.report else {
        panic!("current report")
    };
    let values = report.pairs[0].result.as_ref().unwrap();
    // The first known result is selected automatically when it arrives.
    let marker = panel.marker(&f.session, &scene).unwrap().unwrap();
    assert_eq!(marker.relation, PairRelation::Separated);
    assert_eq!(marker.minimum_clearance_mm, 30.);
    assert_eq!(
        marker.label(),
        "Meets required clearance · measured 30.000 mm · required 0.000 mm"
    );
    // Witnesses are exactly the native report points, not display geometry.
    assert_eq!(marker.witnesses, values.witnesses);
    assert_eq!(marker.stamp, report.stamp);
    for w in &marker.witnesses {
        assert!((w.a_mm[0] - 10.).abs() < 1e-6, "{w:?}");
        assert!((w.b_mm[0] - 40.).abs() < 1e-6, "{w:?}");
        let d = (0..3)
            .map(|i| (w.b_mm[i] - w.a_mm[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!((d - 30.).abs() < 1e-6);
    }
    let [a, b] = marker.bounds.map(Option::unwrap);
    assert!(
        near(a[0], [0., 0., 0.]) && near(a[1], [10., 20., 30.]),
        "{a:?}"
    );
    assert!(
        near(b[0], [40., 20., 5.]) && near(b[1], [50., 40., 35.]),
        "{b:?}"
    );
    assert_eq!(marker.displayed, [true, true]);
    assert_eq!(
        marker.highlight(),
        Selection::Parts(Arc::new(BTreeSet::from([f.part, f.instance])))
    );
    let mut camera = Camera::default();
    camera.fit(&marker.fit_points(), rect());
    let shapes = painted(&marker, camera, false);
    assert!(shape_texts(&shapes).contains(&marker.label()));
    assert!(filled_circles(&shapes, A_COLOR) >= 1);
    assert!(filled_circles(&shapes, B_COLOR) >= 1);
    let (_, separated) = relation_style(PairRelation::Separated);
    assert!(segments(&shapes, separated) >= 1);
    assert!(!ring(&shapes, separated));
    // Occurrence outlines appear only on explicit reveal while both are drawn.
    assert_eq!(segments(&shapes, A_COLOR), 0);
    let revealed = painted(&marker, camera, true);
    assert!(segments(&revealed, A_COLOR) >= 12 && segments(&revealed, B_COLOR) >= 12);
    assert_eq!(f.session.document(), &source);
    assert_eq!(f.session.revision(), revision);
}

#[test]
fn fit_result_frames_occurrences_and_witnesses_and_reveal_is_explicit_without_editing() {
    let f = fixture([250., 0., 0.]);
    let source = f.session.document().clone();
    let scene = scene(&f.session);
    let mut panel = inspected(&f, &scene);
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    let shown = texts(frame(&mut panel, &f.session, &scene, &ctx, vec![]));
    assert!(
        shown
            .iter()
            .any(|t| t.starts_with("Evaluated revision 2 · source ")),
        "{shown:?}"
    );
    assert!(
        !shown
            .iter()
            .any(|t| t.starts_with("Viewport markers hidden"))
    );
    assert!(!panel.take_fit());
    click(&mut panel, &f.session, &scene, &ctx, "Fit result");
    assert!(panel.take_fit());
    assert!(!panel.take_fit(), "a fit request is consumed once");
    assert!(!panel.reveal());
    click(&mut panel, &f.session, &scene, &ctx, "Reveal targets");
    assert!(panel.reveal());
    // Framing: a camera zoomed into the origin area cannot see the gap; the
    // fitted camera projects both boxes and all witnesses inside the viewport.
    let marker = panel.marker(&f.session, &scene).unwrap().unwrap();
    assert_eq!(marker.minimum_clearance_mm, 240.);
    let mut camera = Camera {
        scale: 40.,
        ..Default::default()
    };
    let visible = |camera: &Camera| {
        marker.fit_points().iter().all(|p| {
            let q = camera.project(*p, rect());
            rect().contains(egui::pos2(q[0], q[1]))
        })
    };
    assert!(!visible(&camera));
    camera.fit(&marker.fit_points(), rect());
    assert!(visible(&camera));
    assert_eq!(f.session.document(), &source);
    // Deselecting the result removes its viewport evidence.
    let name = format!(
        "{} / {}",
        path_name(f.session.document(), &marker.pair.a),
        path_name(f.session.document(), &marker.pair.b)
    );
    click(&mut panel, &f.session, &scene, &ctx, &name);
    assert!(panel.marker(&f.session, &scene).is_none());
}

#[test]
fn overlap_and_contact_have_distinct_viewport_feedback() {
    let mut styles = vec![];
    for (at, relation) in [
        ([5., 0., 0.], PairRelation::Overlapping),
        ([10., 0., 0.], PairRelation::ContactWithinTolerance),
    ] {
        let f = fixture(at);
        let scene = scene(&f.session);
        let panel = inspected(&f, &scene);
        let marker = panel.marker(&f.session, &scene).unwrap().unwrap();
        assert_eq!(marker.relation, relation);
        let (_, color) = relation_style(relation);
        let mut camera = Camera::default();
        camera.fit(&marker.fit_points(), rect());
        let shapes = painted(&marker, camera, false);
        assert!(ring(&shapes, color), "{relation:?} needs a visible ring");
        let label = marker.label();
        assert!(shape_texts(&shapes).contains(&label));
        if relation == PairRelation::Overlapping {
            assert!((marker.overlap_volume_mm3 - 3000.).abs() < 1e-3);
            assert!(label.starts_with("Interference · 3000.000 mm³ overlap"));
            // Cross mark plus the witness segment in the interference color.
            assert!(segments(&shapes, color) >= 3);
        } else {
            assert!(label.starts_with("Contact · meets required clearance"));
        }
        styles.push(color);
    }
    styles.push(relation_style(PairRelation::Separated).1);
    let unique: BTreeSet<_> = styles.iter().map(|c| c.to_array()).collect();
    assert_eq!(unique.len(), 3);
}

#[test]
fn hidden_occurrence_is_outlined_from_its_native_placement() {
    let mut f = fixture([40., 0., 0.]);
    let hide = hierarchy::prepare(
        &f.session,
        f.session.revision(),
        actor(),
        hierarchy::Action::SetVisible {
            id: f.instance,
            visible: false,
        },
    )
    .unwrap();
    f.session.commit(hide.prepared).unwrap();
    let scene = scene(&f.session);
    assert!(!scene.displays_instance(f.instance));
    let mut panel = inspected(&f, &scene);
    let marker = panel.marker(&f.session, &scene).unwrap().unwrap();
    assert_eq!(marker.displayed, [true, false]);
    assert_eq!(marker.hidden(), [false, true]);
    let b = marker.bounds[1].unwrap();
    assert!(near(b[0], [40., 0., 0.]) && near(b[1], [50., 20., 30.]));
    let mut camera = Camera::default();
    camera.fit(&marker.fit_points(), rect());
    let shapes = painted(&marker, camera, false);
    assert!(segments(&shapes, B_COLOR) >= 12, "hidden B is outlined");
    assert_eq!(segments(&shapes, A_COLOR), 0, "visible A needs reveal");
    let ctx = egui::Context::default();
    let shown = texts(frame(&mut panel, &f.session, &scene, &ctx, vec![]));
    assert!(
        shown
            .iter()
            .any(|t| t.starts_with("Second occurrence is hidden in the viewport")),
        "{shown:?}"
    );
}

#[test]
fn markers_never_draw_against_another_revision_or_source() {
    let mut f = fixture([40., 0., 0.]);
    let scene = scene(&f.session);
    let mut panel = inspected(&f, &scene);
    let Some(AssemblyInspectionReport::Current(report)) = panel.report.clone() else {
        panic!()
    };
    // Same revision but a different evaluated source hash.
    let mut foreign = report.clone();
    foreign.stamp.source_hash = multicad_core::project_file::ContentHash::of(b"other");
    assert!(
        ClearanceMarker::build(&foreign, 0, &scene)
            .unwrap_err()
            .contains("different model revision")
    );
    // A later canonical revision: the old result and old scene are stale.
    let later = fixture([60., 0., 0.]);
    let later_scene = self::scene(&later.session);
    assert!(ClearanceMarker::build(&report, 0, &later_scene).is_err());
    let hide = hierarchy::prepare(
        &f.session,
        f.session.revision(),
        actor(),
        hierarchy::Action::SetVisible {
            id: f.part,
            visible: false,
        },
    )
    .unwrap();
    f.session.commit(hide.prepared).unwrap();
    assert!(panel.marker(&f.session, &scene).unwrap().is_err());
    let ctx = egui::Context::default();
    let shown = texts(frame(&mut panel, &f.session, &scene, &ctx, vec![]));
    assert!(
        shown
            .iter()
            .any(|t| t.starts_with("Viewport markers hidden")),
        "{shown:?}"
    );
    assert!(!shown.contains(&"Fit result".to_string()));
}

#[test]
fn controller_exposes_marker_only_while_the_interference_tool_holds_a_current_result() {
    use crate::assembly::{AssemblyController, AssemblyTool};
    let f = fixture([40., 20., 5.]);
    let scene = scene(&f.session);
    let mut controller = AssemblyController::default();
    controller.set_query_factory(factory());
    controller
        .begin(
            AssemblyTool::Interference,
            &f.session,
            &Default::default(),
            Some(&scene),
            selection(&f),
            Channel::Ui,
        )
        .unwrap();
    assert!(controller.clearance_marker(&f.session, &scene).is_none());
    controller
        .preview(&f.session, &Default::default(), Channel::Ui)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    while controller.pending() {
        controller.poll(&f.session, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    let marker = controller.clearance_marker(&f.session, &scene).unwrap();
    assert_eq!(marker.minimum_clearance_mm, 30.);
    assert!(!controller.take_clearance_fit());
    assert!(!controller.clearance_reveal());
    // Interference stays read-only: nothing to apply, no candidate scene.
    assert!(!controller.can_apply(&f.session, &Default::default()));
    assert!(
        controller
            .display_scene(&f.session, &Default::default())
            .is_none()
    );
    controller.cancel();
    assert!(controller.clearance_marker(&f.session, &scene).is_none());
}
