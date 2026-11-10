use super::common::*;
use multicad_core::{
    commands::{Request, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, source::Source},
    document::{EntityData, EntityId, Mutation},
    model::{self, *},
    project_file::{self, ProjectFile},
    sketch::Datum,
    sketch_regions,
};
use std::f64::consts::PI;

fn add_sketch(
    session: &mut Session,
    datum: Datum,
    name: &str,
    z: f64,
    build: impl FnOnce(&mut Source, &Session),
) -> (EntityId, Source) {
    let (id, mut edits) = Source::create(datum, name).unwrap();
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit
            && let EntityData::Plane { origin_mm, .. } = &mut entity.data
        {
            origin_mm[2] = z;
        }
    }
    let doc = session.document().edited(&edits).unwrap();
    let mut source = Source::read(&doc, id).unwrap().0;
    build(&mut source, session);
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: name.into(),
            edits,
        })
        .unwrap();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: format!("Define {name}"),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    (id, source)
}
fn circle(
    session: &mut Session,
    name: &str,
    z: f64,
    r: f64,
    driven: bool,
) -> (ProfileSource, EntityId) {
    let mut region = None;
    let mut seam = EntityId::new();
    let (id, _) = add_sketch(session, Datum::Xy, name, z, |source, session| {
        let center = source.add_point([0., 0.]);
        seam = EntityId::new();
        source.sketch.curves.push(Curve {
            id: seam,
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: r,
            },
        });
        region = Some(sketch_regions::extract(source).regions[0].intent());
        source.add_constraint(ConstraintKind::Fix { point: center });
        if driven {
            source.add_constraint(ConstraintKind::Radius {
                curve: seam,
                dimension: bound(session, "wall", "mm"),
            });
        }
    });
    (
        ProfileSource {
            sketch: id,
            region: region.unwrap(),
        },
        seam,
    )
}
fn path(session: &mut Session) -> SketchPathSource {
    let mut line = EntityId::new();
    let (id, _) = add_sketch(session, Datum::Xz, "Ordered path", 0., |source, session| {
        let a = source.add_point([0., 0.]);
        let b = source.add_point([0., 40.]);
        line = source.add_line(a, b);
        source.add_constraint(ConstraintKind::Fix { point: a });
        source.add_constraint(ConstraintKind::Vertical { line });
        source.add_constraint(ConstraintKind::Length {
            line,
            dimension: bound(session, "pitch", "mm"),
        });
    });
    SketchPathSource {
        sketch: id,
        curves: vec![line],
    }
}

fn miter_path(session: &mut Session) -> SketchPathSource {
    let mut curves = vec![];
    let (sketch, _) = add_sketch(session, Datum::Xz, "Miter path", 0., |s, session| {
        let start = s.add_point([0., 0.]);
        let corner = s.add_point([0., 40.]);
        let end = s.add_point([20., 40.]);
        let first = s.add_line(start, corner);
        let second = s.add_line(corner, end);
        curves.extend([first, second]);
        s.add_constraint(ConstraintKind::Fix { point: start });
        s.add_constraint(ConstraintKind::Vertical { line: first });
        s.add_constraint(ConstraintKind::Horizontal { line: second });
        s.add_constraint(ConstraintKind::Length {
            line: first,
            dimension: bound(session, "pitch", "mm"),
        });
        s.add_constraint(ConstraintKind::Length {
            line: second,
            dimension: length(20.),
        });
    });
    SketchPathSource { sketch, curves }
}
fn miter_chain_path(session: &mut Session) -> SketchPathSource {
    let mut curves = vec![];
    let (sketch, _) = add_sketch(session, Datum::Xz, "Two-corner path", 0., |s, session| {
        let points = [[0., 0.], [0., 40.], [20., 40.], [20., 70.]].map(|p| s.add_point(p));
        let [a, b, c, d] = points;
        let [first, middle, last] = [s.add_line(a, b), s.add_line(b, c), s.add_line(c, d)];
        curves.extend([first, middle, last]);
        s.add_constraint(ConstraintKind::Fix { point: a });
        s.add_constraint(ConstraintKind::Vertical { line: first });
        s.add_constraint(ConstraintKind::Horizontal { line: middle });
        s.add_constraint(ConstraintKind::Vertical { line: last });
        for (line, dimension) in [
            (first, bound(session, "pitch", "mm")),
            (middle, length(20.)),
            (last, length(30.)),
        ] {
            s.add_constraint(ConstraintKind::Length { line, dimension });
        }
    });
    SketchPathSource { sketch, curves }
}

fn check_saved_rust_profile(
    session: &mut Session,
    request: &AdvancedRequest,
    volume: f64,
) -> multicad_model::Model {
    use multicad_model::{Compiler, GeometryBackend};
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let evaluate = |document, revision| {
        compiler
            .compile_sources_with_backend(
                document,
                revision,
                &[request.body],
                GeometryBackend::Rust,
                None,
            )
            .unwrap()
    };
    let revision = session.revision();
    let before = session.document().clone();
    let model = evaluate(&before, revision);
    let solid = model.solid(request.body).unwrap();
    assert!(solid.uses_rust_geometry());
    near(solid.volume_mm3().unwrap(), volume);
    assert_eq!(solid.solid_count().unwrap(), 1);
    assert!(
        !solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    let folder = tempfile::tempdir().unwrap();
    let step = folder.path().join("swept.step");
    assert!(solid.supports_step_export());
    solid.write_step(&step).unwrap();
    near(
        multicad_model::export::verify_rust_step_shape(
            &step,
            solid,
            &model.snapshot().shapes[&request.feature],
            1e-7,
        )
        .unwrap(),
        volume,
    );
    let imported = multicad_kernel::Solid::import_rust_step(
        &std::fs::read_to_string(step).unwrap(),
        request.feature,
        Default::default(),
    )
    .unwrap();
    near(imported.volume_mm3().unwrap(), volume);
    let bounds = solid.bounds().unwrap();
    let imported_bounds = imported.bounds().unwrap();
    for (actual, expected) in imported_bounds
        .min
        .into_iter()
        .chain(imported_bounds.max)
        .zip(bounds.min.into_iter().chain(bounds.max))
    {
        near(actual, expected);
    }

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(before.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, before);
    let restored = evaluate(&opened.project.document, revision);
    near(
        restored.solid(request.body).unwrap().volume_mm3().unwrap(),
        volume,
    );
    assert!(restored.snapshot().references.all_resolved());
    session.undo(revision, actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    restored
}

#[test]
fn sweep_rebuilds_solved_profile_and_ordered_path_without_display_geometry() {
    let (mut session, _) = fixture();
    let (profile, _) = circle(&mut session, "Sweep section", 0., 2., true);
    let path = path(&mut session);
    let request = AdvancedRequest::new_body(
        "Swept stock",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path),
    );
    commit(&mut session, &request);
    check_saved_rust_profile(&mut session, &request, PI * 4. * 40.);
    apply_parameter(&mut session, "pitch", "50");
    apply_parameter(&mut session, "wall", "3");
    check_saved_rust_profile(&mut session, &request, PI * 9. * 50.);
    let source = model::read_advanced(session.document(), request.feature)
        .unwrap()
        .0;
    assert_eq!(source.output.body, request.body);
    assert_eq!(source.profiles, request.profiles);
    assert_eq!(source.path, request.path);
}

#[test]
fn mitered_sweep_preserves_path_faces_after_edit_reopen_and_step() {
    use multicad_core::references::{
        self, AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologyKind,
    };
    use multicad_kernel::Solid;
    #[cfg(feature = "occt")]
    use multicad_model::{Compiler, GeometryBackend};

    let (mut session, _) = fixture();
    let (sketch, section) = add_sketch(&mut session, Datum::Xy, "Sweep section", 0., |s, _| {
        s.add_rectangle([-2., -1.], [2., 1.]).unwrap();
    });
    let profile = ProfileSource {
        sketch,
        region: sketch_regions::extract(&section).regions[0].intent(),
    };
    let path = miter_path(&mut session);
    let request = AdvancedRequest::new_body(
        "Mitered sweep",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path.clone()),
    );
    let preview = commit(&mut session, &request);
    let check_geometry = |solid: &Solid, bend_z: f64| {
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), 8. * (bend_z + 20.));
        near(
            solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum(),
            16. + 12. * (bend_z + 20.),
        );
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in
            bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip([-2., -1., 0., 20., 1., bend_z + 2.])
        {
            near(actual, expected);
        }
    };
    check_geometry(preview.solid(request.body).unwrap(), 40.);
    let initial = check_saved_rust_profile(&mut session, &request, 480.);
    check_geometry(initial.solid(request.body).unwrap(), 40.);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let path_origins = path.curves.iter().map(|&source| Origin {
        source,
        role: "path.edge".into(),
    });
    let side_intents = path_origins
        .map(|origin| {
            let sides = topology
                .elements
                .iter()
                .filter(|face| {
                    face.key.kind == TopologyKind::Face && face.origins.contains(&origin)
                })
                .collect::<Vec<_>>();
            assert_eq!(sides.len(), 4, "each leg owns its four exterior walls");
            sides
                .into_iter()
                .map(|side| GeometryIntent::capture(topology, side.key).unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let cap_intents = ["cap.start", "cap.end"].map(|role| {
        let face = topology
            .elements
            .iter()
            .find(|face| {
                face.key.kind == TopologyKind::Face
                    && face.origins.contains(&Origin {
                        source: request.feature,
                        role: role.into(),
                    })
            })
            .unwrap();
        GeometryIntent::capture(topology, face.key).unwrap()
    });

    apply_parameter(&mut session, "pitch", "55");
    let reopened = check_saved_rust_profile(&mut session, &request, 600.);
    let solid = reopened.solid(request.body).unwrap();
    check_geometry(solid, 55.);
    #[cfg(feature = "occt")]
    {
        let native = Compiler::default()
            .compile_sources_with_backend(
                session.document(),
                session.revision(),
                &[request.body],
                GeometryBackend::OpenCascade,
                None,
            )
            .unwrap();
        check_geometry(native.solid(request.body).unwrap(), 55.);
        let directory = tempfile::tempdir().unwrap();
        let step = directory.path().join("mitered-sweep.step");
        solid.write_step(&step).unwrap();
        check_geometry(&Solid::read_step(&step, Default::default()).unwrap(), 55.);
    }
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    let resolve = |intent| match references::resolve(intent, topology, topology.revision) {
        Resolution::Resolved {
            key,
            geometry: Some(AnalyticGeometry::Plane { frame }),
            ..
        } => (key, frame),
        other => panic!("miter sweep face did not resolve after reopening: {other:?}"),
    };
    let sides: std::collections::BTreeSet<_> = side_intents
        .iter()
        .flatten()
        .map(|intent| resolve(intent).0)
        .collect();
    assert_eq!(sides.len(), 8);
    let (_, start) = resolve(&cap_intents[0]);
    let (_, end) = resolve(&cap_intents[1]);
    near(start.origin_mm[2], 0.);
    near(start.normal()[2], -1.);
    near(end.origin_mm[0], 20.);
    near(end.normal()[0], 1.);
    assert_eq!(
        model::read_advanced(session.document(), request.feature)
            .unwrap()
            .0
            .path,
        Some(path),
    );
}

#[test]
fn concave_miter_keeps_the_notched_caps_and_authored_wall_after_edit() {
    use multicad_core::{
        drawings::{VectorCurve, VectorRole},
        references::{self, GeometryIntent, Resolution, TopologyKind},
    };
    use multicad_kernel::{DrawingProjection, DrawingSolid, Solid, project_drawing};

    let (mut session, _) = fixture();
    let mut notch = EntityId::nil();
    let (sketch, section) = add_sketch(&mut session, Datum::Xy, "U section", 0., |s, _| {
        let points = [
            [-3., -2.],
            [3., -2.],
            [3., 2.],
            [1., 2.],
            [1., 0.],
            [-1., 0.],
            [-1., 2.],
            [-3., 2.],
        ]
        .map(|point| s.add_point(point));
        for (i, (a, b)) in points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .enumerate()
        {
            let curve = s.add_line(a, b);
            if i == 4 {
                notch = curve;
            }
        }
    });
    let profile = ProfileSource {
        sketch,
        region: sketch_regions::extract(&section).regions[0].intent(),
    };
    let path = miter_path(&mut session);
    let request = AdvancedRequest::new_body(
        "Notched sweep",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path.clone()),
    );
    let native_preview = commit(&mut session, &request);
    let check_geometry = |solid: &Solid, bend_z: f64| {
        let path_length = bend_z + 20.;
        assert_eq!(solid.solid_count().unwrap(), 1);
        // The cross section and its perimeter are symmetric in X, so the
        // miter's shorter and longer generators cancel in both integrals.
        near(solid.volume_mm3().unwrap(), 20. * path_length);
        near(
            solid.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
            40. + 24. * path_length,
        );
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in
            bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip([-3., -2., 0., 20., 2., bend_z + 3.])
        {
            near(actual, expected);
        }
    };
    check_geometry(native_preview.solid(request.body).unwrap(), 40.);
    let initial = check_saved_rust_profile(&mut session, &request, 1_200.);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let notch = topology
        .elements
        .iter()
        .find(|face| {
            face.key.kind == TopologyKind::Face
                && face.origins.iter().any(|origin| origin.source == notch)
                && face
                    .origins
                    .iter()
                    .any(|origin| origin.source == path.curves[1])
        })
        .unwrap();
    let notch = GeometryIntent::capture(topology, notch.key).unwrap();

    apply_parameter(&mut session, "pitch", "55");
    let reopened = check_saved_rust_profile(&mut session, &request, 1_500.);
    let solid = reopened.solid(request.body).unwrap();
    check_geometry(solid, 55.);
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    assert!(matches!(
        references::resolve(&notch, topology, topology.revision),
        Resolution::Resolved { .. }
    ));
    let drawing = project_drawing(
        &[DrawingSolid { solid, hatch: None }],
        DrawingProjection {
            section_offset_mm: Some(10.),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        drawing
            .iter()
            .any(|vector| vector.role == VectorRole::Section
                && matches!(vector.curve, VectorCurve::Line { a, b }
            if a[1].abs() < 1e-7 && b[1].abs() < 1e-7
                && (a[0].min(b[0]) + 1.).abs() < 1e-7 && (a[0].max(b[0]) - 1.).abs() < 1e-7))
    );
    #[cfg(feature = "occt")]
    {
        let native = compile(&session);
        check_geometry(native.solid(request.body).unwrap(), 55.);
        let directory = tempfile::tempdir().unwrap();
        let step = directory.path().join("notched-sweep.step");
        solid.write_step(&step).unwrap();
        check_geometry(&Solid::read_step(&step, Default::default()).unwrap(), 55.);
    }
}

#[test]
fn circular_miter_rebuilds_its_exact_trim_after_saved_path_and_radius_edits() {
    use multicad_core::drawings::{
        DrawingView, Scale, VectorCurve, VectorRole, ViewEvaluation, ViewKind, ViewSource,
    };
    use multicad_core::references::{
        self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind,
    };
    use multicad_kernel::Solid;

    let (mut session, _) = fixture();
    let (profile, circle_curve) = circle(&mut session, "Round sweep section", 0., 2., true);
    let path = miter_path(&mut session);
    let request = AdvancedRequest::new_body(
        "Circular miter",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path.clone()),
    );
    let preview = commit(&mut session, &request);
    let check_geometry = |solid: &Solid, radius: f64, bend_z: f64| {
        let centerline = bend_z + 20.;
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(
            solid.volume_mm3().unwrap(),
            PI * radius.powi(2) * centerline,
        );
        near(
            solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum(),
            2. * PI * radius * (centerline + radius),
        );
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in bounds.min.into_iter().chain(bounds.max).zip([
            -radius,
            -radius,
            0.,
            20.,
            radius,
            bend_z + radius,
        ]) {
            near(actual, expected);
        }
    };
    check_geometry(preview.solid(request.body).unwrap(), 2., 40.);
    let initial = check_saved_rust_profile(&mut session, &request, PI * 240.);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let walls: Vec<_> = topology
        .elements
        .iter()
        .filter(|face| {
            face.key.kind == TopologyKind::Face
                && face
                    .origins
                    .iter()
                    .any(|origin| origin.source == circle_curve)
                && face
                    .origins
                    .iter()
                    .any(|origin| path.curves.contains(&origin.source))
        })
        .map(|face| GeometryIntent::capture(topology, face.key).unwrap())
        .collect();
    assert_eq!(walls.len(), 8);
    let trims: Vec<_> = topology
        .elements
        .iter()
        .filter_map(|edge| {
            if let Some(AnalyticGeometry::EllipticArc { semiaxes_mm, .. }) = edge.geometry {
                Some(semiaxes_mm)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(trims.len(), 4);
    for [major, minor] in trims {
        near(major, 2. * 2_f64.sqrt());
        near(minor, 2.);
    }

    apply_parameter(&mut session, "pitch", "55");
    apply_parameter(&mut session, "wall", "3");
    let reopened = check_saved_rust_profile(&mut session, &request, PI * 675.);
    let solid = reopened.solid(request.body).unwrap();
    check_geometry(solid, 3., 55.);
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    let resolved: std::collections::BTreeSet<_> = walls
        .iter()
        .map(
            |intent| match references::resolve(intent, topology, topology.revision) {
                Resolution::Resolved { key, .. } => key,
                diagnostic => {
                    panic!("Circular miter wall did not survive the edit: {diagnostic:?}")
                }
            },
        )
        .collect();
    assert_eq!(resolved.len(), 8);
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    let view = DrawingView {
        id: EntityId::new(),
        name: "Oblique miter".into(),
        source: ViewSource::Model {
            entities: vec![request.body],
        },
        frame: references::ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., diagonal, diagonal],
        },
        kind: ViewKind::Projection,
        position_mm: [0.; 2],
        scale: Scale {
            numerator: 1,
            denominator: 1,
        },
        hidden_lines: true,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    };
    let drawing_model = multicad_model::Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(
        session.document(),
        session.revision(),
        multicad_model::GeometryBackend::Rust,
        None,
    )
    .unwrap();
    let drawing = drawing_model
        .drawing_view(
            session.document(),
            session.revision(),
            &view,
            &Default::default(),
        )
        .unwrap();
    let ViewEvaluation::Ready { vectors, .. } = drawing else {
        panic!("Expected exact circular miter drawing: {drawing:?}");
    };
    assert!(
        vectors.iter().any(|vector| matches!(vector.curve,
            VectorCurve::Ellipse { center, major_mm, minor_mm, .. }
            if center[0].abs() < 1e-7 && (center[1] - 55. * diagonal).abs() < 1e-7
                && (major_mm - 3. * (1. + diagonal).sqrt()).abs() < 1e-7
                && (minor_mm - 3. * (1. - diagonal).sqrt()).abs() < 1e-7
        )),
        "the projected miter must retain its exact ellipse"
    );
    let section = multicad_kernel::project_drawing(
        &[multicad_kernel::DrawingSolid { solid, hatch: None }],
        multicad_kernel::DrawingProjection {
            section_offset_mm: Some(55.),
            ..Default::default()
        },
    )
    .unwrap();
    let mut arc_sweep = 0.;
    for vector in section
        .iter()
        .filter(|vector| vector.role == VectorRole::Section)
    {
        if let VectorCurve::Ellipse {
            major_mm,
            minor_mm,
            start_rad,
            end_rad,
            ..
        } = vector.curve
        {
            near(major_mm, 3.);
            near(minor_mm, 3.);
            arc_sweep += (end_rad - start_rad).abs();
        }
    }
    near(arc_sweep, PI);
    #[cfg(feature = "occt")]
    {
        let native = compile(&session);
        check_geometry(native.solid(request.body).unwrap(), 3., 55.);
        let directory = tempfile::tempdir().unwrap();
        let step = directory.path().join("circular-miter.step");
        solid.write_step(&step).unwrap();
        check_geometry(
            &Solid::read_step(&step, Default::default()).unwrap(),
            3.,
            55.,
        );
    }
}

#[test]
fn circular_miter_chain_rebuilds_and_keeps_both_middle_cylinder_trims() {
    use multicad_core::{
        drawings::{VectorCurve, VectorRole},
        references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind},
    };
    use multicad_kernel::{DrawingProjection, DrawingSolid, PlaneFrame, Solid, project_drawing};

    let (mut session, _) = fixture();
    let (profile, circle_curve) = circle(&mut session, "Circular chain section", 0., 2., true);
    let path = miter_chain_path(&mut session);
    let request = AdvancedRequest::new_body(
        "Two circular miters",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path.clone()),
    );
    let preview = commit(&mut session, &request);
    let check_geometry = |solid: &Solid, radius: f64, bend_z: f64| {
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(
            solid.volume_mm3().unwrap(),
            PI * radius.powi(2) * (bend_z + 50.),
        );
        near(
            solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum(),
            2. * PI * radius * (bend_z + 50. + radius),
        );
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in bounds.min.into_iter().chain(bounds.max).zip([
            -radius,
            -radius,
            0.,
            20. + radius,
            radius,
            bend_z + 30.,
        ]) {
            near(actual, expected);
        }
    };
    check_geometry(preview.solid(request.body).unwrap(), 2., 40.);
    let initial = check_saved_rust_profile(&mut session, &request, PI * 360.);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let middle_walls: Vec<_> = topology
        .elements
        .iter()
        .filter(|face| {
            face.key.kind == TopologyKind::Face
                && face
                    .origins
                    .iter()
                    .any(|origin| origin.source == circle_curve)
                && face
                    .origins
                    .iter()
                    .any(|origin| origin.source == path.curves[1])
        })
        .map(|face| GeometryIntent::capture(topology, face.key).unwrap())
        .collect();
    assert_eq!(middle_walls.len(), 4);

    apply_parameter(&mut session, "pitch", "55");
    apply_parameter(&mut session, "wall", "3");
    let reopened = check_saved_rust_profile(&mut session, &request, PI * 945.);
    let solid = reopened.solid(request.body).unwrap();
    check_geometry(solid, 3., 55.);
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    let resolved: std::collections::BTreeSet<_> = middle_walls
        .iter()
        .map(
            |intent| match references::resolve(intent, topology, topology.revision) {
                Resolution::Resolved { key, .. } => key,
                diagnostic => {
                    panic!("The middle sweep wall did not resolve after the edit: {diagnostic:?}")
                }
            },
        )
        .collect();
    assert_eq!(resolved.len(), 4);
    assert_eq!(
        topology
            .elements
            .iter()
            .filter(|edge| matches!(edge.geometry, Some(AnalyticGeometry::EllipticArc { .. })))
            .count(),
        8
    );

    let input = [DrawingSolid { solid, hatch: None }];
    let section = project_drawing(
        &input,
        DrawingProjection {
            frame: PlaneFrame {
                origin: [0.; 3],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            section_offset_mm: Some(10.),
            ..Default::default()
        },
    )
    .unwrap();
    let sweep: f64 = section
        .iter()
        .filter(|vector| vector.role == VectorRole::Section)
        .map(|vector| match vector.curve {
            VectorCurve::Ellipse {
                center,
                major_mm,
                minor_mm,
                start_rad,
                end_rad,
                ..
            } => {
                near(center[0], 0.);
                near(center[1], 55.);
                near(major_mm, 3.);
                near(minor_mm, 3.);
                (end_rad - start_rad).abs()
            }
            ref other => panic!("Expected the exact middle disk section, got {other:?}"),
        })
        .sum();
    near(sweep, 2. * PI);
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    let projection = project_drawing(
        &input,
        DrawingProjection {
            frame: PlaneFrame {
                v: [0., diagonal, diagonal],
                ..Default::default()
            },
            hidden_lines: true,
            ..Default::default()
        },
    )
    .unwrap();
    for x in [0., 20.] {
        assert!(
            projection.iter().any(|vector| matches!(vector.curve,
                VectorCurve::Ellipse { center, major_mm, minor_mm, .. }
                if (center[0] - x).abs() < 1e-7 && (center[1] - 55. * diagonal).abs() < 1e-7
                    && (major_mm - 3. * (1. + diagonal).sqrt()).abs() < 1e-7
                    && (minor_mm - 3. * (1. - diagonal).sqrt()).abs() < 1e-7
            )),
            "Missing exact projection of the joint at X={x}"
        );
    }
    #[cfg(feature = "occt")]
    {
        let native = compile(&session);
        check_geometry(native.solid(request.body).unwrap(), 3., 55.);
        let directory = tempfile::tempdir().unwrap();
        let step = directory.path().join("circular-miter-chain.step");
        solid.write_step(&step).unwrap();
        check_geometry(
            &Solid::read_step(&step, Default::default()).unwrap(),
            3.,
            55.,
        );
    }
}

#[test]
fn mixed_miter_keeps_a_stadium_profile_and_its_authored_walls_after_reopen() {
    use multicad_core::{
        drawings::{VectorCurve, VectorRole},
        references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind},
    };
    use multicad_kernel::{DrawingProjection, DrawingSolid, PlaneFrame, Solid, project_drawing};

    let (mut session, _) = fixture();
    let mut straight = EntityId::nil();
    let curved = EntityId::new();
    let (sketch, section) = add_sketch(&mut session, Datum::Xy, "Stadium profile", 0., |s, _| {
        let [a, b, c, d] = [[-3., -1.], [3., -1.], [3., 1.], [-3., 1.]].map(|p| s.add_point(p));
        straight = s.add_line(a, b);
        let right = s.add_point([3., 0.]);
        let left = s.add_point([-3., 0.]);
        s.sketch.curves.push(Curve {
            id: curved,
            construction: false,
            kind: CurveKind::Arc {
                center: right,
                start: b,
                end: c,
                clockwise: false,
            },
        });
        s.add_line(c, d);
        s.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Arc {
                center: left,
                start: d,
                end: a,
                clockwise: false,
            },
        });
    });
    let profile = ProfileSource {
        sketch,
        region: sketch_regions::extract(&section).regions[0].intent(),
    };
    let path = miter_chain_path(&mut session);
    let request = AdvancedRequest::new_body(
        "Stadium sweep through two bends",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path.clone()),
    );
    let preview = commit(&mut session, &request);
    let area = 12. + PI;
    let perimeter = 12. + 2. * PI;
    let check_geometry = |solid: &Solid, bend_z| {
        let length = bend_z + 50.;
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), area * length);
        near(
            solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum(),
            2. * area + perimeter * length,
        );
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in
            bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip([-4., -1., 0., 24., 1., bend_z + 30.])
        {
            near(actual, expected);
        }
    };
    check_geometry(preview.solid(request.body).unwrap(), 40.);
    let initial = check_saved_rust_profile(&mut session, &request, area * 90.);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let intents: Vec<_> = topology
        .elements
        .iter()
        .filter(|face| {
            face.key.kind == TopologyKind::Face
                && face
                    .origins
                    .iter()
                    .any(|origin| [straight, curved].contains(&origin.source))
                && face
                    .origins
                    .iter()
                    .any(|origin| origin.source == path.curves[1])
        })
        .map(|face| GeometryIntent::capture(topology, face.key).unwrap())
        .collect();
    assert_eq!(intents.len(), 3);

    apply_parameter(&mut session, "pitch", "55");
    let reopened = check_saved_rust_profile(&mut session, &request, area * 105.);
    let solid = reopened.solid(request.body).unwrap();
    check_geometry(solid, 55.);
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    let resolved: std::collections::BTreeSet<_> = intents
        .iter()
        .map(
            |intent| match references::resolve(intent, topology, topology.revision) {
                Resolution::Resolved { key, .. } => key,
                diagnostic => panic!("The stadium sweep wall did not resolve: {diagnostic:?}"),
            },
        )
        .collect();
    assert_eq!(resolved.len(), 3);
    assert!(topology.elements.iter().any(|edge| matches!(edge.geometry,
        Some(AnalyticGeometry::EllipticArc { semiaxes_mm: [a, b], .. })
            if (a - 2_f64.sqrt()).abs() < 1e-7 && (b - 1.).abs() < 1e-7)));

    let input = [DrawingSolid { solid, hatch: None }];
    let section = project_drawing(
        &input,
        DrawingProjection {
            frame: PlaneFrame {
                origin: [0.; 3],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            section_offset_mm: Some(10.),
            section_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut arc_span = 0.;
    let mut straight_span = 0.;
    for vector in section
        .iter()
        .filter(|vector| vector.role == VectorRole::Section)
    {
        match vector.curve {
            VectorCurve::Ellipse {
                major_mm,
                minor_mm,
                start_rad,
                end_rad,
                ..
            } => {
                near(major_mm, 1.);
                near(minor_mm, 1.);
                arc_span += (end_rad - start_rad).abs();
            }
            VectorCurve::Line { a, b } => straight_span += (b[0] - a[0]).hypot(b[1] - a[1]),
            ref other => panic!("Expected exact stadium section edges, got {other:?}"),
        }
    }
    near(arc_span, 2. * PI);
    near(straight_span, 12.);
    let joint = [3., -1., 52.];
    let u = [-0.2, 1., 0.].map(|v| v / 1.04_f64.sqrt());
    let normal = [1., 0.2, 0.3].map(|v| v / 1.13_f64.sqrt());
    let v = [
        -normal[2] * u[1],
        normal[2] * u[0],
        normal[0] * u[1] - normal[1] * u[0],
    ];
    let joint_view = DrawingProjection {
        frame: PlaneFrame {
            origin: joint,
            u,
            v,
        },
        section_offset_mm: Some(0.),
        section_only: true,
        ..Default::default()
    };
    let check_joint = |solid: &Solid| {
        let exact = project_drawing(&[DrawingSolid { solid, hatch: None }], joint_view).unwrap();
        let at_joint = |p: [f64; 2]| p[0].hypot(p[1]) < 1e-7;
        let mut touching = [0; 2];
        for vector in exact
            .iter()
            .filter(|vector| vector.role == VectorRole::Section)
        {
            match vector.curve {
                VectorCurve::Line { a, b } if [a, b].into_iter().any(at_joint) => touching[0] += 1,
                VectorCurve::Ellipse {
                    center,
                    u,
                    v,
                    major_mm,
                    minor_mm,
                    start_rad,
                    end_rad,
                } => {
                    let ends = [start_rad, end_rad].map(|angle| {
                        let (sin, cos) = angle.sin_cos();
                        std::array::from_fn(|i| {
                            center[i] + major_mm * cos * u[i] + minor_mm * sin * v[i]
                        })
                    });
                    if ends.into_iter().any(at_joint) {
                        near(major_mm * normal[2], 1.);
                        near(minor_mm, 1.);
                        touching[1] += 1;
                    }
                }
                _ => {}
            }
        }
        assert_eq!(touching, [1, 1]);
    };
    check_joint(solid);
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("stadium-miter.step");
    solid.write_step(&step).unwrap();
    let imported = Solid::import_rust_step(
        &std::fs::read_to_string(&step).unwrap(),
        request.feature,
        Default::default(),
    )
    .unwrap();
    check_joint(&imported);
    #[cfg(feature = "occt")]
    {
        let native = compile(&session);
        check_geometry(native.solid(request.body).unwrap(), 55.);
        check_geometry(&Solid::read_step(&step, Default::default()).unwrap(), 55.);
    }
}

#[test]
fn an_existing_sweep_rejects_a_path_changed_to_construction_without_source_mutation() {
    let (mut session, _) = fixture();
    let (profile, _) = circle(&mut session, "Sweep profile", 0., 2., false);
    let path = path(&mut session);
    let request = AdvancedRequest::new_body(
        "Swept stock",
        AdvancedParameters::Sweep(Default::default()),
        vec![profile],
        Some(path.clone()),
    );
    commit(&mut session, &request);
    let before = session.document().clone();
    let revision = session.revision();
    let EntityData::Sketch { definition, .. } = &before.get(path.sketch).unwrap().data else {
        panic!()
    };
    let mut source = Source::from_definition(definition).unwrap();
    source
        .sketch
        .curves
        .iter_mut()
        .find(|curve| curve.id == path.curves[0])
        .unwrap()
        .construction = true;
    let mut entity = before.get(path.sketch).unwrap().clone();
    let EntityData::Sketch { definition, .. } = &mut entity.data else {
        panic!()
    };
    *definition = source.definition().unwrap();
    let candidate = before.edited(&[Mutation::Replace(entity)]).unwrap();
    assert_eq!(
        model::read_advanced(&candidate, request.feature)
            .unwrap()
            .0
            .path,
        Some(path)
    );
    let failure = multicad_model::Compiler::default()
        .compile_sources_with_backend(
            &candidate,
            revision,
            &[request.body],
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(failure.entity, Some(request.feature));
    assert!(failure.message.contains("construction curve"));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    near(
        compile(&session)
            .solid(request.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        PI * 4. * 40.,
    );
}
#[test]
fn loft_uses_explicit_section_seams_and_current_bound_section_geometry() {
    let (mut session, _) = fixture();
    let (first, a) = circle(&mut session, "Loft start", 0., 2., true);
    let (second, b) = circle(&mut session, "Loft end", 20., 4., false);
    let request = AdvancedRequest::new_body(
        "Lofted stock",
        AdvancedParameters::Loft(LoftParameters {
            ruled: true,
            seams: vec![a, b],
        }),
        vec![first, second],
        None,
    );
    commit(&mut session, &request);
    check_saved_rust_profile(&mut session, &request, PI * 20. / 3. * (4. + 8. + 16.));
    apply_parameter(&mut session, "wall", "3");
    check_saved_rust_profile(&mut session, &request, PI * 20. / 3. * (9. + 12. + 16.));
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Loft(p) = &mut edit.parameters else {
        panic!()
    };
    p.ruled = false;
    let prepared =
        model::prepare_advanced_edit(&session, session.revision(), actor(), &edit).unwrap();
    session.commit(prepared.prepared).unwrap();
    check_saved_rust_profile(&mut session, &request, PI * 20. / 3. * (9. + 12. + 16.));
}

#[test]
fn ruled_circular_loft_rebuilds_every_band_and_preserves_step_geometry() {
    use multicad_model::Model;

    let (mut session, _) = fixture();
    let (first, a) = circle(&mut session, "Loft start", 0., 2., false);
    let (middle, b) = circle(&mut session, "Loft transition", 7., 2., true);
    let (last, c) = circle(&mut session, "Loft end", 20., 5., false);
    let request = AdvancedRequest::new_body(
        "Ruled circular loft",
        AdvancedParameters::Loft(LoftParameters {
            ruled: true,
            seams: vec![a, b, c],
        }),
        vec![first, middle, last],
        None,
    );
    commit(&mut session, &request);
    let check_bands = |model: &Model, radius: f64| {
        let solid = model.solid(request.body).unwrap();
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip([-5., -5., 0., 5., 5., 20.])
        {
            near(actual, expected);
        }
        let mut actual: Vec<_> = solid
            .faces()
            .unwrap()
            .into_iter()
            .filter_map(|face| face.plane.is_none().then_some(face.area_mm2))
            .collect();
        let mut expected = [
            PI * (2. + radius) * 7_f64.hypot(radius - 2.),
            PI * (radius + 5.) * 13_f64.hypot(5. - radius),
        ];
        actual.sort_unstable_by(f64::total_cmp);
        expected.sort_unstable_by(f64::total_cmp);
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.into_iter().zip(expected) {
            near(actual, expected);
        }
    };
    for radius in [2., 4.] {
        if radius > 2. {
            apply_parameter(&mut session, "wall", "4");
        }
        let frustum = |a, b, h| PI * h * (a * a + a * b + b * b) / 3.;
        let volume = frustum(2., radius, 7.) + frustum(radius, 5., 13.);
        #[cfg(feature = "occt")]
        {
            let native = compile(&session);
            near(
                native.solid(request.body).unwrap().volume_mm3().unwrap(),
                volume,
            );
            check_bands(&native, radius);
        }
        let reopened = check_saved_rust_profile(&mut session, &request, volume);
        check_bands(&reopened, radius);
        let (source, _) = model::read_advanced(session.document(), request.feature).unwrap();
        assert_eq!(source.profiles, request.profiles);
        assert_eq!(source.parameters, request.parameters);
    }
}

#[test]
fn ruled_polygon_loft_preserves_section_seams_after_edit_reopen_and_step() {
    use multicad_core::references::{
        self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind,
    };
    use multicad_model::{Compiler, GeometryBackend, Model};

    let (mut session, _) = fixture();
    let corners =
        |[x, y]: [f64; 2], [w, h]: [f64; 2]| [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    let sections = [
        ("Rectangular loft start", 0., [0., 0.], [6., 4.]),
        ("Rectangular loft middle", 12., [1., -1.], [10., 8.]),
        ("Rectangular loft end", 26., [-2., 2.], [4., 5.]),
    ];
    let [(first, a), (middle, b), (last, c)] = sections.map(|(name, z, origin, size)| {
        let mut seam = EntityId::nil();
        let (sketch, source) = add_sketch(&mut session, Datum::Xy, name, z, |source, _| {
            let points = corners(origin, size).map(|point| source.add_point(point));
            for (i, (a, b)) in points
                .into_iter()
                .zip(points.into_iter().cycle().skip(1))
                .enumerate()
            {
                let edge = source.add_line(a, b);
                if i == 0 {
                    seam = edge;
                }
            }
        });
        let region = sketch_regions::extract(&source).regions[0].intent();
        (ProfileSource { sketch, region }, seam)
    });
    let request = AdvancedRequest::new_body(
        "Ruled rectangular loft",
        AdvancedParameters::Loft(LoftParameters {
            ruled: true,
            seams: vec![a, b, c],
        }),
        vec![first, middle.clone(), last],
        None,
    );
    commit(&mut session, &request);
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let evaluate = |session: &Session, backend| {
        compiler
            .compile_sources_with_backend(
                session.document(),
                session.revision(),
                &[request.body],
                backend,
                None,
            )
            .unwrap()
    };
    let volume = |z, [w, h]: [f64; 2]| {
        // Each width and height varies linearly in its own band. The
        // product is quadratic, so Simpson's rule gives the exact volume.
        let band = |height, [a, b]: [f64; 2], [c, d]: [f64; 2]| {
            height * (2. * a * b + a * d + b * c + 2. * c * d) / 6.
        };
        band(z, [6., 4.], [w, h]) + band(26. - z, [w, h], [4., 5.])
    };
    let check_geometry = |model: &Model, volume, [min, max]: [[f64; 3]; 2]| {
        let solid = model.solid(request.body).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), volume);
        let bounds = solid.bounds().unwrap();
        for i in 0..3 {
            near(bounds.min[i], min[i]);
            near(bounds.max[i], max[i]);
        }
    };

    let initial = evaluate(&session, GeometryBackend::Rust);
    let initial_bounds = [[-2., -1., 0.], [11., 7., 26.]];
    let initial_volume = volume(12., [10., 8.]);
    check_geometry(&initial, initial_volume, initial_bounds);
    #[cfg(feature = "occt")]
    check_geometry(
        &evaluate(&session, GeometryBackend::OpenCascade),
        initial_volume,
        initial_bounds,
    );
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let face = |test: &dyn Fn(&references::TopologyElement) -> bool| {
        topology
            .elements
            .iter()
            .find(|element| element.key.kind == TopologyKind::Face && test(element))
            .unwrap()
    };
    let sides = [[a, b], [b, c]].map(|[start, end]| {
        let side = face(&|e| {
            e.origins.iter().any(|origin| origin.source == start)
                && e.origins.iter().any(|origin| origin.source == end)
        });
        assert!(side.origins.iter().any(|origin| {
            origin.source == request.feature && origin.role.starts_with(&format!("side/{start}/"))
        }));
        GeometryIntent::capture(topology, side.key).unwrap()
    });
    let caps = ["cap.start", "cap.end"].map(|role| {
        let cap = face(&|e| {
            e.origins
                .iter()
                .any(|origin| origin.source == request.feature && origin.role == role)
        });
        GeometryIntent::capture(topology, cap.key).unwrap()
    });

    let mut source = Source::read(session.document(), middle.sketch).unwrap().0;
    for (point, position) in source
        .sketch
        .points
        .iter_mut()
        .zip(corners([2., -2.], [11., 7.]))
    {
        point.position_mm = position;
    }
    let EntityData::Sketch { plane, .. } = session.document().get(middle.sketch).unwrap().data
    else {
        panic!("expected a sketch plane");
    };
    let mut plane = session.document().get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane.data else {
        panic!("expected a datum plane");
    };
    origin_mm[2] = 14.;
    let mut edits = source.replace(session.document(), middle.sketch).unwrap();
    edits.push(Mutation::Replace(plane));
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Move and resize the middle loft section".into(),
            edits,
        })
        .unwrap();
    assert_eq!(
        model::read_advanced(session.document(), request.feature)
            .unwrap()
            .0
            .profiles,
        request.profiles
    );
    let edited_volume = volume(14., [11., 7.]);
    let edited_bounds = [[-2., -2., 0.], [13., 7., 26.]];
    #[cfg(feature = "occt")]
    check_geometry(
        &evaluate(&session, GeometryBackend::OpenCascade),
        edited_volume,
        edited_bounds,
    );
    let reopened = check_saved_rust_profile(&mut session, &request, edited_volume);
    check_geometry(&reopened, edited_volume, edited_bounds);
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    let resolved = |intent| match references::resolve(intent, topology, topology.revision) {
        Resolution::Resolved {
            key,
            geometry: Some(AnalyticGeometry::Plane { frame }),
            ..
        } => (key, frame),
        other => panic!("loft face did not resolve after the saved section edit: {other:?}"),
    };
    let faces = reopened.solid(request.body).unwrap().faces().unwrap();
    for (intent, area) in sides
        .iter()
        .zip([8.5 * 200_f64.sqrt(), 7.5 * 160_f64.sqrt()])
    {
        let (key, _) = resolved(intent);
        near(faces[key.ordinal as usize].area_mm2, area);
    }
    for (intent, [z, sign]) in caps.iter().zip([[0., -1.], [26., 1.]]) {
        let (_, frame) = resolved(intent);
        near(frame.origin_mm[2], z);
        near(frame.normal()[2], sign);
    }
}

#[test]
fn smooth_planar_loft_keeps_the_intermediate_station_attachment() {
    use multicad_core::references::{self, Attachment, GeometryIntent, Resolution, TopologyKind};

    let (mut session, _) = fixture();
    let [a, b] = notched_loft_footprints();
    let interpolate = |t: f64| {
        std::array::from_fn(|i| std::array::from_fn(|j| (b[i][j] - a[i][j]).mul_add(t, a[i][j])))
    };
    let [(first, start), (middle, station), (last, end)] = [
        ("Smooth start", 0., a),
        ("Smooth station", 1.5, interpolate(0.375)),
        ("Smooth end", 4., b),
    ]
    .map(|(name, z, points)| loft_polygon(&mut session, name, z, points));
    let request = AdvancedRequest::new_body(
        "Smooth planar notch",
        AdvancedParameters::Loft(LoftParameters {
            seams: vec![start[0], station[0], end[0]],
            ..Default::default()
        }),
        vec![first, middle.clone(), last],
        None,
    );
    commit(&mut session, &request);
    let volume = 4. * (56. + 4. * 70.5 + 86.) / 6.;
    #[cfg(feature = "occt")]
    let native = compile(&session);
    #[cfg(feature = "occt")]
    near(
        native.solid(request.body).unwrap().volume_mm3().unwrap(),
        volume,
    );

    let initial = check_saved_rust_profile(&mut session, &request, volume);
    assert_eq!(
        initial.solid(request.body).unwrap().faces().unwrap().len(),
        10
    );
    #[cfg(feature = "occt")]
    {
        let area = |solid: &multicad_kernel::Solid| -> f64 {
            solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum()
        };
        near(
            area(native.solid(request.body).unwrap()),
            area(initial.solid(request.body).unwrap()),
        );
    }
    let notch = |model: &multicad_model::Model| {
        let topology = &model.snapshot().shapes[&request.feature].topology;
        topology
            .elements
            .iter()
            .find(|face| {
                face.key.kind == TopologyKind::Face
                    && [start[4], station[4], end[4]]
                        .into_iter()
                        .all(|id| face.origins.iter().any(|origin| origin.source == id))
            })
            .unwrap()
            .clone()
    };
    let face = notch(&initial);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let mut intent = GeometryIntent::capture(topology, face.key).unwrap();
    let anchor = face
        .frames
        .iter()
        .find(|anchor| anchor.origin.source == station[4])
        .unwrap();
    near(anchor.frame.origin_mm[2], 1.5);
    intent.attachment = Some(Attachment {
        anchor: anchor.origin.clone(),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });

    let mut source = Source::read(session.document(), middle.sketch).unwrap().0;
    for (point, position) in source.sketch.points.iter_mut().zip(interpolate(0.625)) {
        point.position_mm = position;
    }
    let EntityData::Sketch { plane, .. } = session.document().get(middle.sketch).unwrap().data
    else {
        panic!("expected the middle section plane");
    };
    let mut plane = session.document().get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane.data else {
        panic!("expected a datum plane");
    };
    origin_mm[2] = 2.5;
    let mut edits = source.replace(session.document(), middle.sketch).unwrap();
    edits.push(Mutation::Replace(plane));
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Move the smooth loft station along the same wall".into(),
            edits,
        })
        .unwrap();
    let restored = check_saved_rust_profile(&mut session, &request, volume);
    let topology = &restored.snapshot().shapes[&request.feature].topology;
    let Resolution::Resolved {
        key,
        frame: Some(frame),
        ..
    } = references::resolve(&intent, topology, topology.revision)
    else {
        panic!("the station attachment must resolve on the merged wall");
    };
    assert_eq!(key, notch(&restored).key);
    near(frame.origin_mm[2], 2.5);

    // A real change in the transition needs the general smooth provider.
    source.sketch.points[0].position_mm[0] += 0.5;
    let altered = session
        .document()
        .edited(&source.replace(session.document(), middle.sketch).unwrap())
        .unwrap();
    let error = multicad_model::Compiler::default()
        .compile_sources_with_backend(
            &altered,
            session.revision(),
            &[request.body],
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("a non-affine station must not be flattened");
    assert!(error.message.contains("straight transition"));
}

#[test]
fn smooth_coaxial_loft_keeps_the_exact_cone_and_all_section_origins() {
    use multicad_core::references::TopologyKind;

    let (mut session, _) = fixture();
    let (first, a) = circle(&mut session, "Cone start", 0., 2., false);
    let (middle, b) = circle(&mut session, "Cone station", 7., 3.05, false);
    let middle_sketch = middle.sketch;
    let (last, c) = circle(&mut session, "Cone end", 20., 5., false);
    let request = AdvancedRequest::new_body(
        "Smooth coaxial cone",
        AdvancedParameters::Loft(LoftParameters {
            seams: vec![a, b, c],
            ..Default::default()
        }),
        vec![first, middle, last],
        None,
    );
    commit(&mut session, &request);
    let volume = 20. * PI * (4. + 10. + 25.) / 3.;
    let area = PI * (4. + 25. + 7. * 20_f64.hypot(3.));
    let check = |model: &multicad_model::Model| {
        let solid = model.solid(request.body).unwrap();
        near(solid.volume_mm3().unwrap(), volume);
        near(
            solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum(),
            area,
        );
    };
    #[cfg(feature = "occt")]
    check(&compile(&session));
    let reopened = check_saved_rust_profile(&mut session, &request, volume);
    check(&reopened);
    assert_eq!(
        reopened.solid(request.body).unwrap().faces().unwrap().len(),
        3
    );
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    assert!(topology.elements.iter().any(|face| {
        face.key.kind == TopologyKind::Face
            && [a, b, c]
                .into_iter()
                .all(|id| face.origins.iter().any(|origin| origin.source == id))
    }));

    let mut source = Source::read(session.document(), middle_sketch).unwrap().0;
    let CurveKind::Circle { radius_mm, .. } = &mut source.sketch.curves[0].kind else {
        panic!("expected the authored circle");
    };
    *radius_mm += 0.25;
    let altered = session
        .document()
        .edited(&source.replace(session.document(), middle_sketch).unwrap())
        .unwrap();
    let error = multicad_model::Compiler::default()
        .compile_sources_with_backend(
            &altered,
            session.revision(),
            &[request.body],
            multicad_model::GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("a non-affine circular station must not be flattened");
    assert!(error.message.contains("straight transition"));
}

#[test]
fn default_two_section_loft_preserves_planar_notch_after_reopen() {
    let (mut session, _) = fixture();
    let [bottom, top] = notched_loft_footprints();
    let [(first, a), (last, b)] = [
        ("Default loft start", 0., bottom),
        ("Default loft end", 4., top),
    ]
    .map(|(name, z, points)| loft_polygon(&mut session, name, z, points));
    let parameters = LoftParameters {
        seams: vec![a[0], b[0]],
        ..Default::default()
    };
    assert!(!parameters.ruled);
    let request = AdvancedRequest::new_body(
        "Default notched loft",
        AdvancedParameters::Loft(parameters),
        vec![first, last],
        None,
    );
    commit(&mut session, &request);
    let volume = 4. * (56. + 4. * 70.5 + 86.) / 6.;
    #[cfg(feature = "occt")]
    let native = compile(&session);
    #[cfg(feature = "occt")]
    near(
        native.solid(request.body).unwrap().volume_mm3().unwrap(),
        volume,
    );
    let reopened = check_saved_rust_profile(&mut session, &request, volume);
    let solid = reopened.solid(request.body).unwrap();
    assert_eq!(solid.faces().unwrap().len(), 10);
    let bounds = solid.bounds().unwrap();
    for (actual, expected) in bounds
        .min
        .into_iter()
        .chain(bounds.max)
        .zip([0., -1., 0., 12., 10., 4.])
    {
        near(actual, expected);
    }
    #[cfg(feature = "occt")]
    {
        let native = native.solid(request.body).unwrap();
        let area = |solid: &multicad_kernel::Solid| {
            solid
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum()
        };
        near(area(native), area(solid));
    }
}

fn loft_polygon<const N: usize>(
    session: &mut Session,
    name: &str,
    z: f64,
    outline: [[f64; 2]; N],
) -> (ProfileSource, [EntityId; N]) {
    let mut curves = [EntityId::nil(); N];
    let (sketch, source) = add_sketch(session, Datum::Xy, name, z, |source, _| {
        let points = outline.map(|p| source.add_point(p));
        curves = std::array::from_fn(|i| source.add_line(points[i], points[(i + 1) % N]));
    });
    let region = sketch_regions::extract(&source).regions[0].intent();
    (ProfileSource { sketch, region }, curves)
}

fn notched_loft_footprints() -> [[[f64; 2]; 8]; 2] {
    [
        [
            [0., 0.],
            [10., 0.],
            [10., 8.],
            [7., 8.],
            [7., 2.],
            [3., 2.],
            [3., 8.],
            [0., 8.],
        ],
        [
            [1., -1.],
            [12., -1.],
            [12., 10.],
            [9., 10.],
            [9., 3.],
            [4., 3.],
            [4., 10.],
            [1., 10.],
        ],
    ]
}

#[test]
fn concave_ruled_loft_rebuilds_its_notch_and_saved_walls() {
    use multicad_core::{
        drawings::{VectorCurve, VectorRole},
        references::{self, GeometryIntent, Resolution, TopologyKind},
    };
    use multicad_kernel::{
        DrawingProjection, DrawingSolid, PlaneFrame, Solid, project_drawing, rust_kernel::Rational,
    };

    let (mut session, _) = fixture();
    let [bottom, middle] = notched_loft_footprints();
    let end = middle.map(|[x, y]| [0.75 * x + 1., 0.75 * y + 1.]);
    let sections = [
        ("Notched start", 0., bottom),
        ("Notched transition", 4., middle),
        ("Notched end", 10., end),
    ];
    let [(first, a), (middle, b), (last, c)] =
        sections.map(|(name, z, outline)| loft_polygon(&mut session, name, z, outline));
    let request = AdvancedRequest::new_body(
        "Notched ruled loft",
        AdvancedParameters::Loft(LoftParameters {
            ruled: true,
            seams: vec![a[0], b[0], c[0]],
        }),
        vec![first, middle.clone(), last],
        None,
    );
    commit(&mut session, &request);
    let volume = |z| {
        z * (56. + 4. * 70.5 + 86.) / 6. + (10. - z) * 86. * (1. + 0.75 + 0.75_f64.powi(2)) / 3.
    };
    let initial = check_saved_rust_profile(&mut session, &request, volume(4.));
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let notch = topology
        .elements
        .iter()
        .find(|face| {
            face.key.kind == TopologyKind::Face
                && [a[4], b[4]]
                    .into_iter()
                    .all(|id| face.origins.iter().any(|origin| origin.source == id))
        })
        .unwrap();
    let intent = GeometryIntent::capture(topology, notch.key).unwrap();
    let EntityData::Sketch { plane, .. } = session.document().get(middle.sketch).unwrap().data
    else {
        panic!("expected the middle sketch plane");
    };
    let mut plane = session.document().get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane.data else {
        panic!("expected a datum plane");
    };
    origin_mm[2] = 5.;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Move the middle notch section".into(),
            edits: vec![Mutation::Replace(plane)],
        })
        .unwrap();
    let reopened = check_saved_rust_profile(&mut session, &request, volume(5.));
    let solid = reopened.solid(request.body).unwrap();
    assert_eq!(solid.faces().unwrap().len(), 18);
    let topology = &reopened.snapshot().shapes[&request.feature].topology;
    assert!(matches!(
        references::resolve(&intent, topology, topology.revision),
        Resolution::Resolved { .. }
    ));
    let vectors = project_drawing(
        &[DrawingSolid { solid, hatch: None }],
        DrawingProjection {
            section_offset_mm: Some(2.5),
            section_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        vectors
            .iter()
            .any(|vector| vector.role == VectorRole::Section
                && matches!(vector.curve, VectorCurve::Line { a, b }
            if (a[1] - 2.5).abs() < 1e-7 && (b[1] - 2.5).abs() < 1e-7
                && (a[0].min(b[0]) - 3.5).abs() < 1e-7
                && (a[0].max(b[0]) - 8.).abs() < 1e-7))
    );
    let tool = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [1, 1, 3].map(|size| Rational::new(size, 1).unwrap()),
        PlaneFrame {
            origin: [1.2, 0.25, -1.],
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap();
    let cut_feature = EntityId::new();
    let cut = solid
        .rust_subtract_planar_tools(&[&tool], cut_feature, Default::default())
        .unwrap()
        .unwrap();
    assert_eq!(cut.solid_count().unwrap(), 1);
    near(cut.volume_mm3().unwrap(), volume(5.) - 2.);
    assert!(
        cut.topology_snapshot(cut_feature, 0)
            .unwrap()
            .elements
            .iter()
            .any(|face| face.key.kind == TopologyKind::Face
                && [a[4], b[4]]
                    .into_iter()
                    .all(|id| face.origins.iter().any(|origin| origin.source == id)))
    );
    #[cfg(feature = "occt")]
    {
        near(
            compile(&session)
                .solid(request.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            volume(5.),
        );
        let directory = tempfile::tempdir().unwrap();
        let step = directory.path().join("notched-loft.step");
        solid.write_step(&step).unwrap();
        let imported = multicad_kernel::Solid::read_step(&step, Default::default()).unwrap();
        near(imported.volume_mm3().unwrap(), volume(5.));
        assert_eq!(imported.solid_count().unwrap(), 1);
    }
}

#[test]
fn missing_seam_and_unsupported_curved_path_fail_without_committing_source() {
    let (mut session, _) = fixture();
    let (first, _) = circle(&mut session, "First", 0., 2., false);
    let (second, b) = circle(&mut session, "Second", 20., 4., false);
    let request = AdvancedRequest::new_body(
        "Bad seam",
        AdvancedParameters::Loft(LoftParameters {
            ruled: true,
            seams: vec![EntityId::new(), b],
        }),
        vec![first.clone(), second],
        None,
    );
    let original = session.document().clone();
    let revision = session.revision();
    assert!(model::prepare_advanced(&session, revision, actor(), &request).is_err());
    assert_eq!(session.document(), &original);
    let path = SketchPathSource {
        sketch: first.sketch,
        curves: vec![
            Source::read(session.document(), first.sketch)
                .unwrap()
                .0
                .sketch
                .curves[0]
                .id,
        ],
    };
    let request = AdvancedRequest::new_body(
        "Unsupported path",
        AdvancedParameters::Sweep(Default::default()),
        vec![first],
        Some(path),
    );
    let candidate = model::prepare_advanced(&session, revision, actor(), &request).unwrap();
    let result =
        multicad_model::Compiler::default().compile(candidate.prepared.document(), revision);
    assert!(result.err().unwrap().message.contains("arcs and splines"));
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), revision);
}
