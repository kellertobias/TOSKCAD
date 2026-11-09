use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, source::Source},
    document::{EntityData, EntityId},
    exchange::LengthUnit,
    model::{self, BodyInput, BodyOutput, CutoutFace, ProfileSource, SheetProfileCutDefinition},
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, TopologyKind},
    sketch::{self, CreationPlane, PlaneOptions},
    sketch_regions,
};
use multicad_model::{
    Compiler, EvaluationCache, Model,
    export::{Exporter, sheet_dxf::SheetDxfLayers},
};

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

#[test]
fn developed_c_channel_flange_cut_matches_folded_geometry_and_millimeter_dxf() {
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
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let before = compiler.compile(session.document(), 0).unwrap();
    let flat_before = before.unfold(session.document(), part).unwrap();
    let face = CutoutFace::Flange { index: 0 };
    let [p0, p1, _, p3] = flat_before.chart.legs[0].quad;
    assert_eq!(flat_before.chart.legs[0].index, 0);
    assert_eq!(flat_before.chart.bend_zones.len(), flat_before.bends.len());
    let [x, y] = [45., 25.];
    let unit = |p: [f64; 2]| {
        let delta = [p[0] - p0[0], p[1] - p0[1]];
        let length = delta[0].hypot(delta[1]);
        delta.map(|d| d / length)
    };
    let center: [f64; 2] = std::array::from_fn(|i| p0[i] + x * unit(p1)[i] + y * unit(p3)[i]);
    let folded_frame = before
        .sheet_authoring_faces(session.document())
        .into_iter()
        .find(|frame| frame.part == part && frame.face == face)
        .unwrap();
    let folded_center: [f64; 3] = std::array::from_fn(|i| {
        folded_frame.origin_mm[i] + x * folded_frame.u[i] + y * folded_frame.v[i]
    });

    let created = sketch::prepare_new(
        &session,
        session.revision(),
        actor(),
        "Hole in developed flange",
        CreationPlane::DevelopedSheet { part, face },
        PlaneOptions::default(),
    )
    .unwrap();
    let sketch = created.sketch;
    session.commit(created.prepared).unwrap();
    let mut profile = source(&session, sketch);
    let center_id = profile.add_point(center);
    let curve = EntityId::new();
    profile.sketch.curves.push(Curve {
        id: curve,
        construction: false,
        kind: CurveKind::Circle {
            center: center_id,
            radius_mm: 2.5,
        },
    });
    profile.add_constraint(ConstraintKind::Fix { point: center_id });
    profile.add_constraint(ConstraintKind::Radius {
        curve,
        dimension: model::length(2.5),
    });
    let region = sketch_regions::extract(&profile).regions[0].intent();
    apply_sketch(&mut session, sketch, &profile);

    let feature = EntityId::new();
    let prepared = model::prepare_sheet_profile_cut(
        &session,
        session.revision(),
        actor(),
        feature,
        &SheetProfileCutDefinition {
            output: BodyOutput { body: part },
            input: BodyInput {
                body: part,
                producer: forming,
            },
            profile: ProfileSource { sketch, region },
            face,
        },
    )
    .unwrap();
    let mut cache = EvaluationCache::default();
    let preview = compiler
        .compile_cached(
            &mut cache,
            prepared.prepared.document(),
            session.revision() + 1,
            None,
        )
        .unwrap();
    let unfolded = preview.unfold(prepared.prepared.document(), part).unwrap();
    assert_eq!(
        preview.snapshot().verified_developed_cut(part),
        Some(&unfolded.cut)
    );
    assert_eq!(unfolded.cut.loops.len(), flat_before.cut.loops.len() + 1);
    assert_eq!(unfolded.bends, flat_before.bends);
    assert!(
        preview.snapshot().shapes[&feature]
            .topology
            .elements
            .iter()
            .any(|element| {
                matches!(
                    element.geometry,
                    Some(AnalyticGeometry::CircularArc { frame, radius_mm, .. })
                        if (radius_mm - 2.5).abs() < 1e-8
                            && frame.origin_mm.into_iter().zip(folded_center)
                                .all(|(a, b)| (a - b).abs() < 1e-7)
                )
            })
    );
    near(
        before.solid(forming).unwrap().volume_mm3().unwrap()
            - preview.solid(feature).unwrap().volume_mm3().unwrap(),
        unfolded.thickness_mm * std::f64::consts::PI * 2.5_f64.powi(2),
    );
    session.commit(prepared.prepared).unwrap();

    for constraint in &mut profile.sketch.constraints {
        if let ConstraintKind::Radius { dimension, .. } = &mut constraint.kind {
            *dimension = model::length(3.);
        }
    }
    apply_sketch(&mut session, sketch, &profile);
    let updated = compiler
        .compile_cached(&mut cache, session.document(), session.revision(), None)
        .unwrap();
    let flat_updated = updated.unfold(session.document(), part).unwrap();
    near(
        unfolded.area_mm2 - flat_updated.area_mm2,
        2.75 * std::f64::consts::PI,
    );

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
    let saved_chart = model::developed_sheet_sketch(&reopened.document, sketch)
        .unwrap()
        .unwrap();
    assert_eq!(
        (saved_chart.part, saved_chart.forming, saved_chart.face),
        (part, forming, face)
    );
    let export = Exporter { compiler }
        .developed_sheet_dxf(
            &reopened.document,
            0,
            part,
            LengthUnit::Millimeter,
            &SheetDxfLayers::default(),
            None,
        )
        .unwrap();
    assert_eq!(export.pattern, flat_updated);
    let drawing = ::dxf::Drawing::load(&mut export.bytes.as_slice()).unwrap();
    assert_eq!(
        drawing.header.default_drawing_units,
        ::dxf::enums::Units::Millimeters
    );
    let circle_polyline = drawing
        .entities()
        .filter(|entity| entity.common.layer == "CUT_INNER")
        .any(|entity| {
            let ::dxf::entities::EntityType::LwPolyline(polyline) = &entity.specific else {
                return false;
            };
            let [a, b] = polyline.vertices.as_slice() else {
                return false;
            };
            // A full circle is emitted as two exact bulged semicircles.
            (a.bulge.abs() - 1.).abs() < 1e-8
                && (b.bulge.abs() - 1.).abs() < 1e-8
                && ((a.x - b.x).hypot(a.y - b.y) - 6.).abs() < 1e-7
                && ((a.x + b.x) * 0.5 - center[0]).abs() < 1e-7
                && ((a.y + b.y) * 0.5 - center[1]).abs() < 1e-7
        });
    assert!(
        circle_polyline,
        "DXF must retain the resized circular cut at 1:1"
    );

    // Move the whole source circle to the developed bend/leg boundary. The
    // candidate must refuse it before the accepted cut can be changed.
    let [a, b, ..] = flat_updated.chart.legs[0].quad;
    let point = profile
        .sketch
        .points
        .iter_mut()
        .find(|p| p.id == center_id)
        .unwrap();
    point.position_mm = std::array::from_fn(|i| (a[i] + b[i]) * 0.5);
    let candidate = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Move the aperture across a bend".into(),
            edits: profile.replace(session.document(), sketch).unwrap(),
        })
        .unwrap();
    let error = compiler
        .compile(candidate.document(), session.revision() + 1)
        .err()
        .expect("a cut crossing the developed bend must be refused");
    assert!(
        error.message.contains("clear of its edges and bends"),
        "{error}"
    );
    assert_eq!(
        updated.unfold(session.document(), part).unwrap(),
        flat_updated
    );
}
