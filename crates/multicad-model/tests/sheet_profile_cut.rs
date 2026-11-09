use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, source::Source},
    document::{EntityData, EntityId},
    model::{self, BodyInput, BodyOutput, CutoutFace, ProfileSource, SheetProfileCutDefinition},
    project_file::{self, ProjectFile},
    references::TopologyKind,
    sketch::{self, CreationPlane, PlaneOptions},
    sketch_regions,
};
use multicad_model::{Compiler, EvaluationCache, Model};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn source(session: &Session, sketch: EntityId) -> Source {
    let EntityData::Sketch { definition, .. } = &session.document().get(sketch).unwrap().data
    else {
        panic!("Expected a sketch");
    };
    Source::from_definition(definition).unwrap()
}

fn apply_sketch(session: &mut Session, id: EntityId, source: &Source) {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Edit the sheet sketch".into(),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
}

fn cut(
    session: &mut Session,
    model: &Model,
    part: EntityId,
    face: CutoutFace,
    draw: impl FnOnce(&mut Source, &dyn Fn([f64; 2]) -> [f64; 2]),
) -> (EntityId, EntityId) {
    let producer = model.snapshot().body_sources[&part];
    let frames = model.sheet_authoring_faces(session.document());
    let frame = frames
        .iter()
        .find(|frame| frame.part == part && frame.face == face)
        .unwrap();
    let role = match face {
        CutoutFace::Base => "cap.start".to_string(),
        CutoutFace::Flange { index } => format!("sheet.flange.{index}.leg.a"),
    };
    let topology = &model.snapshot().shapes[&producer].topology;
    let element = topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
        .find(|element| {
            element.origins.iter().any(|origin| {
                origin.source == frame.feature && origin.role.rsplit('/').next() == Some(&role)
            }) && !element.frames.is_empty()
        })
        .unwrap();
    let created = sketch::prepare_new(
        session,
        session.revision(),
        actor(),
        "Cut profile",
        CreationPlane::Face {
            snapshot: topology,
            key: element.key,
            anchor: Some(&element.frames[0].origin),
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let id = created.sketch;
    let to_sketch = |[u, v]: [f64; 2]| {
        created.frame.coordinates(std::array::from_fn(|axis| {
            frame.origin_mm[axis] + u * frame.u[axis] + v * frame.v[axis]
        }))
    };
    session.commit(created.prepared).unwrap();
    let mut source = source(session, id);
    draw(&mut source, &to_sketch);
    let region = sketch_regions::extract(&source).regions[0].intent();
    apply_sketch(session, id, &source);
    let feature = EntityId::new();
    let request = SheetProfileCutDefinition {
        output: BodyOutput { body: part },
        input: BodyInput {
            body: part,
            producer,
        },
        profile: ProfileSource { sketch: id, region },
        face,
    };
    let prepared =
        model::prepare_sheet_profile_cut(session, session.revision(), actor(), feature, &request)
            .unwrap();
    session.commit(prepared.prepared).unwrap();
    (id, feature)
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn attached_sketches_cut_the_bundled_base_and_flange_and_remain_live() {
    let mut session = Session::new(
        project_file::read(
            &mut include_bytes!("../../../examples/front-panel.tcd").as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap();
    let part = session
        .document()
        .entities()
        .find(|entity| entity.name == "Front panel sheet")
        .unwrap()
        .id;
    let forming = model::sheet_feature(session.document(), part).unwrap();
    let hardware = model::sheet_hardware(session.document(), forming).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let mut cache = EvaluationCache::default();
    let mut compile = |session: &Session| {
        compiler
            .compile_cached(&mut cache, session.document(), session.revision(), None)
            .unwrap()
    };
    let baseline = compile(&session);
    let before = baseline.unfold(session.document(), part).unwrap();
    let before_volume = baseline.solid(forming).unwrap().volume_mm3().unwrap();

    let (circle, _) = cut(
        &mut session,
        &baseline,
        part,
        CutoutFace::Base,
        |source, uv| {
            let center = source.add_point(uv([20., 15.]));
            let curve = EntityId::new();
            source.sketch.curves.push(Curve {
                id: curve,
                construction: false,
                kind: CurveKind::Circle {
                    center,
                    radius_mm: 2.5,
                },
            });
            source.add_constraint(ConstraintKind::Fix { point: center });
            source.add_constraint(ConstraintKind::Radius {
                curve,
                dimension: model::length(2.5),
            });
        },
    );
    let native = compile(&session);
    let (_, final_feature) = cut(
        &mut session,
        &native,
        part,
        CutoutFace::Flange { index: 0 },
        |source, uv| {
            // A support frame can be rotated relative to the cutter frame.
            let corners = [[41., 22.], [49., 22.], [49., 28.], [41., 28.]];
            let points = corners.map(|corner| source.add_point(uv(corner)));
            for (a, b) in points.into_iter().zip(points.into_iter().cycle().skip(1)) {
                source.add_line(a, b);
            }
        },
    );
    let native = compile(&session);
    let flat = native.unfold(session.document(), part).unwrap();
    assert_eq!(flat.cut.loops.len(), before.cut.loops.len() + 2);
    assert_eq!(flat.bends, before.bends);
    assert_eq!(flat.feature, forming);
    assert_eq!(native.snapshot().body_sources[&part], final_feature);
    assert_eq!(
        model::sheet_hardware(session.document(), forming).unwrap(),
        hardware
    );
    let added_area = std::f64::consts::PI * 2.5_f64.powi(2) + 48.;
    near(before.area_mm2 - flat.area_mm2, added_area);
    near(
        before_volume - native.solid(final_feature).unwrap().volume_mm3().unwrap(),
        added_area * flat.thickness_mm,
    );
    assert!(flat.drills.iter().any(|drill| {
        drill.id.starts_with("profile_cut.")
            && (drill.diameter_mm - 5.).abs() < 1e-8
            && (drill.center[0] - 20.).abs() < 1e-8
            && (drill.center[1] - 15.).abs() < 1e-8
    }));

    let mut sketch = source(&session, circle);
    for constraint in &mut sketch.sketch.constraints {
        if let ConstraintKind::Radius { dimension, .. } = &mut constraint.kind {
            *dimension = model::length(3.);
        }
    }
    apply_sketch(&mut session, circle, &sketch);
    let native = compile(&session);
    let larger = native.unfold(session.document(), part).unwrap();
    near(flat.area_mm2 - larger.area_mm2, 2.75 * std::f64::consts::PI);

    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let read = compiler.compile(&reopened.document, 0).unwrap();
    assert_eq!(read.unfold(&reopened.document, part).unwrap(), larger);
    assert_eq!(
        model::sheet_hardware(&reopened.document, forming).unwrap(),
        hardware
    );

    session.undo(session.revision(), actor()).unwrap();
    let native = compile(&session);
    assert_eq!(native.unfold(session.document(), part).unwrap(), flat);
}
