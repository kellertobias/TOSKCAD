//! Production native receipts for every advanced Inspector family.
use super::chamfer_tests::{canonical, click, frame, wait};
use super::*;
use crate::modeling::{AdvancedKind, AdvancedSelection, native_fixture};
use multicad_core::{
    constraint_sketch::source::Source,
    document::{Document, Mutation},
    model::{self, ProfileSource, SketchPathSource},
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent},
    sketch::Datum,
    sketch_regions,
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
struct Fixture {
    session: Session,
    body: EntityId,
    profiles: Vec<ProfileSource>,
    path: SketchPathSource,
}
fn profile(document: Document, name: &str, z: f64, size: [f64; 2]) -> (Document, ProfileSource) {
    let (sketch, edits) = Source::create(Datum::Xy, name).unwrap();
    let mut document = document.edited(&edits).unwrap();
    let (mut source, _, plane) = Source::read(&document, sketch).unwrap();
    source
        .add_rectangle([-size[0] / 2., -size[1] / 2.], [size[0] / 2., size[1] / 2.])
        .unwrap();
    document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let region = sketch_regions::extract(&source).regions[0].intent();
    let mut plane = document.get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane.data else {
        panic!()
    };
    origin_mm[2] = z;
    (
        document.edited(&[Mutation::Replace(plane)]).unwrap(),
        ProfileSource { sketch, region },
    )
}
fn fixture() -> Fixture {
    let mut seed = Session::new(Document::new("Native advanced controls").unwrap()).unwrap();
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
    let (document, first) = profile(seed.document().clone(), "First section", 0., [10., 8.]);
    let (document, second) = profile(document, "Second section", 20., [6., 4.]);
    let (sketch, edits) = Source::create(Datum::Yz, "Path").unwrap();
    let document = document.edited(&edits).unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let a = source.add_point([0., 0.]);
    let b = source.add_point([0., 30.]);
    let curve = source.add_line(a, b);
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    session.mark_saved(0).unwrap();
    Fixture {
        session,
        body: stock.body,
        profiles: vec![first, second],
        path: SketchPathSource {
            sketch,
            curves: vec![curve],
        },
    }
}
fn capture(scene: &Scene, body: EntityId, normal: [f64; 3]) -> GeometryIntent {
    let producer = scene
        .reference_instances()
        .iter()
        .find(|i| i.id == body)
        .unwrap()
        .geometry;
    let snapshot = scene.geometry_snapshot(producer).unwrap();
    let key = snapshot
        .elements
        .iter()
        .find(
            |e| matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()==normal),
        )
        .unwrap()
        .key;
    GeometryIntent::capture(&snapshot, key).unwrap()
}
#[test]
fn every_advanced_inspector_receives_native_geometry_commits_once_and_reopens_editable_source() {
    for kind in [
        AdvancedKind::Shell,
        AdvancedKind::Draft,
        AdvancedKind::Mirror,
        AdvancedKind::LinearPattern,
        AdvancedKind::CircularPattern,
        AdvancedKind::Sweep,
        AdvancedKind::Loft,
    ] {
        let Fixture {
            mut session,
            body,
            profiles,
            path,
        } = fixture();
        let original = session.document().clone();
        let scene = canonical(&session);
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
        controller
            .set_advanced_sketch_context(&session, profiles.clone(), vec![path.clone()])
            .unwrap();
        let captures = match kind {
            AdvancedKind::Shell => vec![capture(&scene, body, [0., 0., 1.])],
            AdvancedKind::Draft => vec![capture(&scene, body, [1., 0., 0.])],
            _ => vec![],
        };
        let selection = AdvancedSelection {
            body: kind.requires_body().then_some(body),
            profile: (!kind.requires_body()).then(|| profiles[0].clone()),
            captures,
        };
        controller
            .begin_advanced(
                &session,
                &Default::default(),
                kind.requires_body().then_some(scene.as_ref()),
                kind,
                selection,
                Channel::Ui,
            )
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        if kind == AdvancedKind::Sweep {
            click(&ctx, &mut controller, &mut session, "Add path segment");
            click(&ctx, &mut controller, &mut session, "Path · Curve 1");
        }
        if kind == AdvancedKind::Loft {
            click(&ctx, &mut controller, &mut session, "Add section (ordered)");
            click(
                &ctx,
                &mut controller,
                &mut session,
                "Second section · profile 1 (4 boundary curves)",
            );
        }
        assert!(!controller.can_apply(&session, &Default::default()));
        click(&ctx, &mut controller, &mut session, "Preview");
        wait(&mut controller, &session);
        assert!(
            controller.can_apply(&session, &Default::default()),
            "{kind:?}: {:?}",
            controller.error()
        );
        assert_eq!(session.document(), &original);
        assert_eq!(session.revision(), 0);
        click(&ctx, &mut controller, &mut session, "Apply");
        assert_eq!(session.revision(), 1, "{kind:?} did not commit once");
        assert!(!controller.active());
        let applied = session.document().clone();
        let feature=session.document().entities().find(|e|matches!(&e.data,EntityData::Feature{definition,..}if definition.type_id==model::ADVANCED_TYPE)).unwrap().id;
        let (definition, captures) = model::read_advanced(session.document(), feature).unwrap();
        let native = multicad_model::Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let volume = native
            .solid(definition.output.body)
            .unwrap()
            .volume_mm3()
            .unwrap();
        assert!(volume > 0.);
        assert!(native.snapshot().references.all_resolved());
        controller
            .begin_existing(&session, &Default::default(), feature, Channel::Ui)
            .unwrap();
        controller
            .set_feature_context(&session, &Default::default(), &canonical(&session))
            .unwrap();
        click(&ctx, &mut controller, &mut session, "Preview");
        wait(&mut controller, &session);
        assert!(
            controller.can_apply(&session, &Default::default()),
            "edit {kind:?}: {:?}",
            controller.error()
        );
        click(&ctx, &mut controller, &mut session, "Cancel");
        assert_eq!(session.document(), &applied);
        assert_eq!(session.revision(), 1);
        session.undo(1, actor()).unwrap();
        assert_eq!(session.document(), &original);
        session.redo(2, actor()).unwrap();
        assert_eq!(session.document(), &applied);
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(applied).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let opened = project_file::read(
            &mut std::io::Cursor::new(bytes),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let reopened = model::read_advanced(&opened.project.document, feature).unwrap();
        assert_eq!(reopened, (definition.clone(), captures));
        let model = multicad_model::Compiler::default()
            .compile(&opened.project.document, 0)
            .unwrap();
        assert!(
            (model
                .solid(definition.output.body)
                .unwrap()
                .volume_mm3()
                .unwrap()
                - volume)
                .abs()
                < 1e-5
        );
    }
}
fn replace_value(
    ctx: &egui::Context,
    c: &mut ModelToolController,
    s: &mut Session,
    old: &str,
    new: &str,
) {
    let out = frame(ctx, c, s, vec![]);
    let pos = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|(_, n)| n.value() == Some(old))
        .filter_map(|(_, n)| n.bounds())
        .find(|r| r.x1 - r.x0 > 50.)
        .map(|b| egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32))
        .expect("dimension input");
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
    frame(
        ctx,
        c,
        s,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text(new.into()),
        ],
    )
    .drop_without_applying_deltas();
}
#[test]
fn impossible_native_shell_retains_repair_controls_and_source_then_accepts_corrected_thickness() {
    let Fixture {
        mut session, body, ..
    } = fixture();
    let scene = canonical(&session);
    let original = session.document().clone();
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c.begin_advanced(
        &session,
        &Default::default(),
        Some(&scene),
        AdvancedKind::Shell,
        AdvancedSelection {
            body: Some(body),
            captures: vec![capture(&scene, body, [0., 0., 1.])],
            ..Default::default()
        },
        Channel::Ui,
    )
    .unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    replace_value(&ctx, &mut c, &mut session, "2", "1000");
    click(&ctx, &mut c, &mut session, "Preview");
    wait(&mut c, &session);
    assert!(c.error().is_some());
    assert!(!c.can_apply(&session, &Default::default()));
    assert!(c.advanced_active());
    assert_eq!(c.advanced_captures().unwrap().len(), 1);
    assert_eq!(session.document(), &original);
    assert!(!session.can_undo());
    assert_eq!(session.revision(), 0);
    replace_value(&ctx, &mut c, &mut session, "1000", "2");
    click(&ctx, &mut c, &mut session, "Preview");
    wait(&mut c, &session);
    assert!(
        c.can_apply(&session, &Default::default()),
        "{:?}",
        c.error()
    );
    click(&ctx, &mut c, &mut session, "Cancel");
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), 0);
    assert!(!session.can_undo());
}
