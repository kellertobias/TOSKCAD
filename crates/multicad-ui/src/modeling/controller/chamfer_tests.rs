//! Production UI/controller proof for explicit sided native chamfer receipts.
use super::*;
use crate::{inspector_actions, modeling::native_fixture};
use multicad_core::{
    document::Document,
    model::{self, PrimitiveParameters, PrimitiveRequest},
    references::{AnalyticGeometry, GeometryIntent, TopologyKind},
};
use multicad_model::Compiler;
use std::time::{Duration, Instant};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
pub(super) fn canonical(session: &Session) -> Arc<Scene> {
    let m = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    native_fixture::scene(session.document(), m.snapshot(), 0).unwrap()
}
pub(super) fn frame(
    ctx: &egui::Context,
    controller: &mut ModelToolController,
    session: &mut Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    controller.poll(session, &Default::default());
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360., 1000.),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            inspector_actions::scope(
                ui,
                Some(inspector_actions::Owner::Model),
                crate::registry::Mode::Model.color(),
                |ui| {
                    controller.show(ui, session, &Default::default()).unwrap();
                },
            );
        },
    )
}
pub(super) fn click(
    ctx: &egui::Context,
    c: &mut ModelToolController,
    s: &mut Session,
    label: &str,
) {
    frame(ctx, c, s, vec![]).drop_without_applying_deltas();
    let out = frame(ctx, c, s, vec![]);
    let pos = out
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .or_else(|| {
            out.platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .and_then(|(_, n)| n.bounds())
                .map(|b| egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32))
        })
        .unwrap_or_else(|| panic!("Missing {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            c,
            s,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
pub(super) fn wait(c: &mut ModelToolController, s: &Session) {
    let until = Instant::now() + Duration::from_secs(5);
    while c.pending() {
        c.poll(s, &Default::default());
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn native_sided_chamfer_mode_switch_retains_source_and_failed_nonadjacent_face_is_repairable() {
    let mut seed = Session::new(Document::new("Sided native Inspector").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(30.), length(20.), length(10.)],
            frame: Default::default(),
        },
    );
    seed.commit(
        model::prepare_primitive(&seed, seed.revision(), actor(), &stock)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut session = Session::new(seed.document().clone()).unwrap();
    session.mark_saved(0).unwrap();
    let original = session.document().clone();
    let scene = canonical(&session);
    let snapshot = scene.geometry_snapshot(stock.feature).unwrap();
    let edge=snapshot.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm:a,b_mm:b})if e.key.kind==TopologyKind::Edge&&a[0]==30.&&b[0]==30.&&a[1]==20.&&b[1]==20.)).unwrap();
    let face = |positive: bool| {
        snapshot.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()[0]==if positive{1.}else{-1.})).map(|e|GeometryIntent::capture(&snapshot,e.key).unwrap()).unwrap()
    };
    let edge = GeometryIntent::capture(&snapshot, edge.key).unwrap();
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c.begin_edge_treatment(
        &session,
        &Default::default(),
        &scene,
        stock.body,
        vec![edge.clone()],
        Channel::Ui,
    )
    .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    click(&ctx, &mut c, &mut session, "Chamfer");
    click(&ctx, &mut c, &mut session, "Symmetric distances");
    click(&ctx, &mut c, &mut session, "Two distances");
    click(&ctx, &mut c, &mut session, "Choose side face for edge 1");
    assert_eq!(c.edge_treatment_pick_kind(), Some(TopologyKind::Face));
    c.set_edge_treatment_pick_captures(&session, &Default::default(), &scene, vec![face(false)])
        .unwrap();
    click(&ctx, &mut c, &mut session, "Preview");
    wait(&mut c, &session);
    assert!(!c.can_apply(&session, &Default::default()));
    let error = c.error().expect("nonadjacent face must fail").to_owned();
    assert!(error.to_lowercase().contains("adjacent"), "{error}");
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), 0);
    assert!(!session.can_undo());
    click(&ctx, &mut c, &mut session, "Change side face for edge 1");
    c.set_edge_treatment_pick_captures(&session, &Default::default(), &scene, vec![face(true)])
        .unwrap();
    click(&ctx, &mut c, &mut session, "Preview");
    wait(&mut c, &session);
    assert!(
        c.can_apply(&session, &Default::default()),
        "{:?}",
        c.error()
    );
    assert_eq!(session.document(), &original);
    click(&ctx, &mut c, &mut session, "Apply");
    assert_eq!(session.revision(), 1);
    assert!(!c.active());
    let feature = match &session.document().get(stock.body).unwrap().data {
        EntityData::Part { features, .. } => *features.last().unwrap(),
        _ => panic!(),
    };
    let (source, captures) = model::read_edge_treatment(session.document(), feature).unwrap();
    assert!(matches!(
        source.chamfer,
        Some(model::ChamferSettings::TwoDistances { .. })
    ));
    assert_eq!(source.edges, vec![edge.id]);
    assert_eq!(captures.len(), 2);
    let applied = session.document().clone();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let volume = native.solid(stock.body).unwrap().volume_mm3().unwrap();
    assert!((volume - 5980.).abs() < 1e-5);
    c.begin_existing(&session, &Default::default(), feature, Channel::Ui)
        .unwrap();
    c.set_feature_context(&session, &Default::default(), &canonical(&session))
        .unwrap();
    click(&ctx, &mut c, &mut session, "Two distances");
    click(&ctx, &mut c, &mut session, "Distance + angle");
    click(&ctx, &mut c, &mut session, "Preview");
    wait(&mut c, &session);
    assert!(
        c.can_apply(&session, &Default::default()),
        "{:?}",
        c.error()
    );
    click(&ctx, &mut c, &mut session, "Cancel");
    assert_eq!(session.document(), &applied);
    assert_eq!(session.revision(), 1);
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(2, actor()).unwrap();
    assert_eq!(session.document(), &applied);
}
