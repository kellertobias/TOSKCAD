use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Curve, CurveKind, source::Source},
    document::{Document, EntityData, EntityId, Mutation},
    manufacturing::PlanarEdge,
    model::{
        self, Direction, Extent, ExtrudeParameters, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, Termination, length,
    },
    project_file::{self, FileLimits, ProjectFile},
    references::{self, GeometryIntent, Origin, Resolution, TopologyKind, TopologySnapshot},
    sketch::{self, CreationPlane, PlaneOptions},
};
use multicad_kernel::{
    PlaneFrame,
    rust_kernel::{Rational, parse_length_mm, prism::Location},
};
use multicad_model::{Compiler, EvaluationCache, EvaluationScope, GeometryBackend, Model};
use std::{f64::consts::PI, io::Cursor, sync::Arc};

#[path = "support/rust_panel.rs"]
mod rust_panel;
#[path = "support/rust_polygon.rs"]
mod rust_polygon;
use rust_panel::fixture;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
}

fn compiler() -> Compiler {
    Compiler::default()
}

fn compile(session: &Session, backend: GeometryBackend) -> Model {
    compiler()
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
}

fn reopen(session: &Session) -> Session {
    let project = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, FileLimits::default()).unwrap();
    let result = project_file::read(
        &mut Cursor::new(bytes),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&result.project.document, session.document());
    Session::new(result.project.document).unwrap()
}

#[test]
fn saved_front_panel_primitives_use_their_bound_placement_and_size_formulas() {
    let mut bytes = include_bytes!("../../../examples/front-panel.tcd").as_slice();
    let document = project_file::read(
        &mut bytes,
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let features = document
        .entities()
        .filter_map(|entity| {
            let EntityData::Feature { definition, .. } = &entity.data else {
                return None;
            };
            (definition.type_id == model::PRIMITIVE_TYPE
                && (entity.name == "Left end part plug feature"
                    || (entity.name.contains("Push button")
                        && definition.data.to_string().contains("$sheet_t - 4 mm"))))
            .then_some(entity.id)
        })
        .collect::<Vec<_>>();
    assert_eq!(features.len(), 2);
    let build = |backend| {
        compiler()
            .compile_sources_with_backend(&document, 0, &features, backend, None)
            .unwrap()
    };
    let native = build(GeometryBackend::OpenCascade);
    let rust = build(GeometryBackend::Rust);
    for feature in features {
        let a = rust.solid(feature).unwrap();
        let b = native.solid(feature).unwrap();
        near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
        let (a, b) = (a.bounds().unwrap(), b.bounds().unwrap());
        for axis in 0..3 {
            near(a.min[axis], b.min[axis]);
            near(a.max[axis], b.max[axis]);
        }
    }
}

#[test]
fn compound_fractional_lengths_drive_extrusion_and_primitive_through_rename_and_reopen() {
    use multicad_core::{
        constraint_sketch::Dimension, document::Entity, parameters::BoundExpression,
    };

    let (mut session, height, extrusion, _) = rust_polygon::fixture_with_height("=1 cm / 3");
    let parameter = |name, text: &str| {
        Entity::new(
            name,
            EntityData::Parameter {
                expression: text.into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        )
    };
    let span = parameter("span", "=$height * 2 + 1 mm / 6");
    let finish = parameter("finish", "=($span - 0.5 mm) / 3");
    let finish_id = finish.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Set derived model dimensions".into(),
            edits: vec![Mutation::Insert(span), Mutation::Insert(finish)],
        })
        .unwrap();

    let formula = |text| Dimension::Expression {
        expression: BoundExpression::bind(session.document(), text, "mm", None).unwrap(),
    };
    let mut parameters = model::read_extrude(session.document(), extrusion)
        .unwrap()
        .parameters;
    parameters.extent = Extent::OneSided {
        end: Termination::Distance {
            distance: formula("=($finish + $height) / 2"),
        },
    };
    let box_request = PrimitiveRequest::new(
        "Formula box",
        PrimitiveParameters::Box {
            size: [
                formula("=(1 / 3)"),
                formula("=$finish * 2"),
                formula("=(10 mm - 1 mm) / 4"),
            ],
            frame: PrimitiveFrame {
                origin: [formula("=1 mm / 9"), length(20.), length(4.)],
                ..Default::default()
            },
        },
    );
    let prepared = model::prepare_extrude_edit(
        &session,
        session.revision(),
        actor(),
        extrusion,
        &parameters,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor(), &box_request).unwrap();
    session.commit(prepared.prepared).unwrap();

    let verify = |session: &Session, depth: f64, width: f64| {
        let rust = compile(session, GeometryBackend::Rust);
        let extruded = rust.solid(extrusion).unwrap();
        assert!(extruded.uses_rust_geometry());
        let bounds = extruded.bounds().unwrap();
        near(bounds.min[1], -2. - depth);
        near(bounds.max[1], -2.);
        near(extruded.volume_mm3().unwrap(), 92. * depth);

        let primitive = rust.solid(box_request.feature).unwrap();
        assert!(primitive.uses_rust_geometry());
        let bounds = primitive.bounds().unwrap();
        let origin = [1. / 9., 20., 4.];
        let size = [1. / 3., width, 9. / 4.];
        for i in 0..3 {
            near(bounds.min[i], origin[i]);
            near(bounds.max[i], origin[i] + size[i]);
        }
        near(primitive.volume_mm3().unwrap(), size.iter().product());
    };
    verify(&session, 49. / 18., 38. / 9.);

    let renamed = |id, name: &str| {
        let mut parameter = session.document().get(id).unwrap().clone();
        parameter.name = name.into();
        Mutation::Replace(parameter)
    };
    let rename_edits = vec![
        renamed(height, "stock_height"),
        renamed(finish_id, "finish_width"),
        Mutation::Insert(parameter("height", "90")),
        Mutation::Insert(parameter("finish", "900")),
    ];
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Rename bound parameters and reuse their old names".into(),
            edits: rename_edits,
        })
        .unwrap();
    let mut session = reopen(&session);
    verify(&session, 49. / 18., 38. / 9.);

    let mut source = session.document().get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut source.data else {
        panic!("Expected the original height parameter");
    };
    *expression = "=17 mm / 3".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Change original renamed height".into(),
            edits: vec![Mutation::Replace(source)],
        })
        .unwrap();
    verify(&session, 14. / 3., 22. / 3.);
}

#[test]
fn rust_revolve_rebuilds_signed_and_full_sketch_sweeps_with_holes_and_native_face_intents() {
    let (sketch, edits) = Source::create(sketch::Datum::Xz, "Meridian with a hole").unwrap();
    let document = Document::new("Offset meridian revolution")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle([3., 2.], [6., 8.]).unwrap();
    source.add_rectangle([4., 4.], [5., 6.]).unwrap();
    let region = multicad_core::sketch_regions::extract(&source)
        .regions
        .into_iter()
        .find(|region| region.holes.len() == 1)
        .unwrap()
        .intent();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let mut parameters = model::RevolveParameters {
        axis_origin: [length(1.), length(2.)],
        angle: model::angle(90.),
        ..Default::default()
    };
    let request = model::RevolveRequest::new(sketch, vec![region], parameters.clone());
    let feature = request.outputs[0].feature;
    let body = request.outputs[0].body;
    let draft = model::prepare_revolve(&session, 0, actor(), &request).unwrap();
    assert!(session.document().get(feature).is_none());
    session.commit(draft.prepared).unwrap();
    let mut end_intent = None;
    for degrees in [90_f64, -90., 360., -360.] {
        if degrees != 90. {
            parameters.angle = model::angle(degrees);
            let edit = model::prepare_revolve_edit(
                &session,
                session.revision(),
                actor(),
                feature,
                &parameters,
            )
            .unwrap();
            session.commit(edit.prepared).unwrap();
        }
        let native_model = compile(&session, GeometryBackend::OpenCascade);
        let rust = compile(&session, GeometryBackend::Rust);
        let native_solid = native_model.solid(feature).unwrap();
        let native = &native_model.snapshot().shapes[&feature];
        let actual = &rust.snapshot().shapes[&feature];
        let solid = rust.solid(feature).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(actual.volume_mm3, 56. * degrees.to_radians().abs());
        near(actual.volume_mm3, native.volume_mm3);
        let (sin, cos) = (degrees.signum() * std::f64::consts::FRAC_PI_4).sin_cos();
        let occupied = |r, z| {
            solid
                .rust_revolution_contains([1. + r * cos, r * sin, z])
                .unwrap()
        };
        assert!(occupied(4.5, 5.));
        assert!(occupied(3.5, 3.));
        assert!(
            !occupied(3.5, 5.),
            "The swept internal cavity must be empty"
        );
        let [lo, hi] = if degrees.abs() == 360. {
            [[-4., -5., 2.], [6., 5., 8.]]
        } else if degrees > 0. {
            [[1., 0., 2.], [6., 5., 8.]]
        } else {
            [[1., -5., 2.], [6., 0., 8.]]
        };
        for (actual, expected) in actual
            .bounds
            .min
            .into_iter()
            .chain(actual.bounds.max)
            .zip(lo.into_iter().chain(hi))
        {
            near(actual, expected);
        }
        let faces = solid.faces().unwrap();
        let native_faces = native_solid.faces().unwrap();
        assert_eq!(faces.len(), native_faces.len());
        for face in native
            .topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let intent = GeometryIntent::capture(&native.topology, face.key).unwrap();
            let Resolution::Resolved { key, .. } =
                references::resolve(&intent, &actual.topology, session.revision())
            else {
                panic!("Lost a native revolved face intent at {degrees}°: {intent:?}");
            };
            let face_at = |faces: &[multicad_kernel::Face], ordinal| {
                let face = faces.iter().find(|face| face.index == ordinal).unwrap();
                (face.area_mm2, face.plane.is_some())
            };
            let (area, planar) = face_at(&faces, key.ordinal);
            let (expected, native_planar) = face_at(&native_faces, face.key.ordinal);
            near(area, expected);
            assert_eq!(planar, native_planar);
        }
        if degrees == 90. {
            let cap = native
                .topology
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face
                        && e.origins.iter().any(|o| o.role == "cap.end")
                })
                .unwrap();
            let mut intent = GeometryIntent::capture(&native.topology, cap.key).unwrap();
            intent.attachment = Some(references::Attachment {
                anchor: cap
                    .origins
                    .iter()
                    .find(|o| o.role == "cap.end")
                    .unwrap()
                    .clone(),
                offset_mm: [2., 0., 0.],
                angle_rad: 0.,
            });
            end_intent = Some(intent);
        }
        let resolved = references::resolve(
            end_intent.as_ref().unwrap(),
            &actual.topology,
            session.revision(),
        );
        if degrees.abs() == 360. {
            assert_eq!(resolved, Resolution::Missing);
        } else {
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = resolved
            else {
                panic!("End cap attachment did not survive a signed-angle edit: {resolved:?}");
            };
            let expected = references::resolve(
                end_intent.as_ref().unwrap(),
                &native.topology,
                session.revision(),
            );
            let Resolution::Resolved {
                frame: Some(expected),
                ..
            } = expected
            else {
                panic!("The OpenCascade control did not resolve its own end cap");
            };
            for (a, b) in frame
                .origin_mm
                .into_iter()
                .chain(frame.u)
                .chain(frame.v)
                .zip(
                    expected
                        .origin_mm
                        .into_iter()
                        .chain(expected.u)
                        .chain(expected.v),
                )
            {
                near(a, b);
            }
        }
        let receipt = solid
            .manufacturing_mesh_with_receipt(multicad_kernel::MeshOptions::default())
            .unwrap();
        assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
        assert_eq!(
            model::read_revolve(session.document(), feature)
                .unwrap()
                .output
                .body,
            body
        );
    }
    let reopened = reopen(&session);
    assert_eq!(
        model::read_revolve(reopened.document(), feature)
            .unwrap()
            .parameters,
        parameters
    );
    near(
        compile(&reopened, GeometryBackend::Rust)
            .solid(feature)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        112. * PI,
    );
}

#[test]
fn rust_full_circle_revolution_preserves_exact_torus_geometry_and_saved_face_identity() {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_kernel::{
        DrawingProjection, DrawingSolid, GeometryPolicy, MeshOptions, Solid, project_drawing,
        rust_kernel::{manufacturing, step_read::read_analytic_solid},
    };
    let (sketch, edits) = Source::create(sketch::Datum::Xz, "Torus section").unwrap();
    let document = Document::new("Exact torus")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let center = source.add_point([5., 0.]);
    source.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 1.,
        },
    });
    let region = multicad_core::sketch_regions::extract(&source).regions[0].intent();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let mut parameters = model::RevolveParameters::default();
    let request = model::RevolveRequest::new(sketch, vec![region], parameters.clone());
    let feature = request.outputs[0].feature;
    let part = request.outputs[0].body;
    let draft = model::prepare_revolve(&session, 0, actor(), &request).unwrap();
    session.commit(draft.prepared).unwrap();
    let first = compile(&session, GeometryBackend::Rust);
    let topology = &first.snapshot().shapes[&feature].topology;
    let face = topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face)
        .unwrap();
    let intent = GeometryIntent::capture(topology, face.key).unwrap();
    let stock = first.solid(part).unwrap();
    near(stock.volume_mm3().unwrap(), 10. * PI.powi(2));
    assert!(!stock.rust_revolution_contains([0.; 3]).unwrap());
    assert!(stock.rust_revolution_contains([5., 0., 0.]).unwrap());
    assert_eq!(stock.display_edges(0.1).unwrap().len(), 2);
    assert_eq!(stock.display_vertices().unwrap().len(), 1);
    let inputs = [DrawingSolid {
        solid: stock,
        hatch: None,
    }];
    let axial = project_drawing(&inputs, DrawingProjection::default()).unwrap();
    assert_eq!(axial.len(), 2);
    for (outline, radius) in axial.iter().zip([6., 4.]) {
        assert_eq!(outline.role, VectorRole::Visible);
        assert!(
            matches!(outline.curve, VectorCurve::Ellipse { major_mm, minor_mm, .. } if major_mm == radius && minor_mm == radius)
        );
    }
    let side = project_drawing(
        &inputs,
        DrawingProjection {
            frame: PlaneFrame {
                u: [0., 1., 0.],
                v: [0., 0., 1.],
                ..PlaneFrame::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(side.len(), 4);
    assert_eq!(
        side.iter()
            .filter(|edge| matches!(edge.curve, VectorCurve::Line { .. }))
            .count(),
        2
    );
    let section = project_drawing(
        &inputs,
        DrawingProjection {
            section_offset_mm: Some(0.6),
            section_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(section.len(), 2);
    for (boundary, radius) in section.iter().zip([5.8, 4.2]) {
        assert_eq!(boundary.role, VectorRole::Section);
        let VectorCurve::Ellipse {
            major_mm, minor_mm, ..
        } = boundary.curve
        else {
            panic!("expected analytic circle")
        };
        near(major_mm, radius);
        near(minor_mm, radius);
    }

    verify_torus_drawing_scene(stock);

    parameters.axis_origin[0] = length(1.);
    parameters.angle = model::angle(-360.);
    let edit =
        model::prepare_revolve_edit(&session, session.revision(), actor(), feature, &parameters)
            .unwrap();
    session.commit(edit.prepared).unwrap();
    let reopened = reopen(&session);
    let rust = compile(&reopened, GeometryBackend::Rust);
    let shape = &rust.snapshot().shapes[&feature];
    let solid = rust.solid(part).unwrap();
    assert!(matches!(
        references::resolve(&intent, &shape.topology, reopened.revision()),
        Resolution::Resolved { .. }
    ));
    near(shape.volume_mm3, 8. * PI.powi(2));
    near(solid.faces().unwrap()[0].area_mm2, 16. * PI.powi(2));
    for (actual, expected) in shape
        .bounds
        .min
        .into_iter()
        .chain(shape.bounds.max)
        .zip([-4., -5., -1., 6., 5., 1.])
    {
        near(actual, expected);
    }
    let mesh = solid.manufacturing_mesh(MeshOptions::default()).unwrap();
    let mesh = multicad_kernel::rust_kernel::display::DisplayMesh {
        vertices: mesh.vertices,
        indices: mesh.indices,
        face_indices: mesh.face_indices,
    };
    assert!(manufacturing::certify(&mesh).unwrap().signed_volume_mm3 > 0.);
    let exchange = tempfile::tempdir().unwrap();
    let exported = exchange.path().join("torus.step");
    solid.write_step(&exported).unwrap();
    let text = std::fs::read_to_string(&exported).unwrap();
    let policy = GeometryPolicy::default();
    let proof = read_analytic_solid(&text, policy.linear_tolerance_mm).unwrap();
    assert!(Solid::step_topology_matches(
        &shape.topology,
        &proof,
        policy.linear_tolerance_mm
    ));
    let imported = Solid::import_rust_step(&text, feature, policy).unwrap();
    near(imported.volume_mm3().unwrap(), shape.volume_mm3);
    assert!(!imported.rust_revolution_contains([1., 0., 0.]).unwrap());

    #[cfg(feature = "occt")]
    {
        let native = compile(&reopened, GeometryBackend::OpenCascade);
        let native = native.solid(part).unwrap();
        near(native.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
        near(
            native.faces().unwrap()[0].area_mm2,
            solid.faces().unwrap()[0].area_mm2,
        );
        let roundtrip = Solid::read_step(&exported, policy).unwrap();
        assert_eq!(roundtrip.solid_count().unwrap(), 1);
        near(roundtrip.volume_mm3().unwrap(), shape.volume_mm3);
        near(
            roundtrip.faces().unwrap()[0].area_mm2,
            solid.faces().unwrap()[0].area_mm2,
        );
    }

    parameters.angle = model::angle(90.);
    let partial =
        model::prepare_revolve_edit(&session, session.revision(), actor(), feature, &parameters)
            .unwrap();
    let Err(error) = compiler().compile_with_backend(
        partial.prepared.document(),
        session.revision(),
        GeometryBackend::Rust,
        None,
    ) else {
        panic!("Rust accepted an unsupported partial torus");
    };
    assert_eq!(error.entity, Some(feature));
    assert!(error.message.contains("straight"), "{error}");
}

fn verify_torus_drawing_scene(torus: &multicad_kernel::Solid) {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_kernel::{
        DrawingProjection, DrawingSolid, PlaneFrame, Solid, project_drawing, rust_kernel::Rational,
    };

    let stock = |origin, size: [i64; 3]| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|value| Rational::new(value, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            Default::default(),
        )
        .unwrap()
    };
    let project = |solids: &[&Solid], options| {
        let inputs: Vec<_> = solids
            .iter()
            .map(|&solid| DrawingSolid { solid, hatch: None })
            .collect();
        project_drawing(&inputs, options).unwrap()
    };
    let options = DrawingProjection {
        hidden_lines: true,
        chord_tolerance_mm: 1e-4,
        ..Default::default()
    };
    let rear = stock([-8., 0., -4.], [16, 1, 1]);
    let axial = project(&[torus, &rear], options);
    let axial_spans = |edges: &[multicad_kernel::ProjectedVector], role| {
        let mut ranges: Vec<_> = edges
            .iter()
            .filter_map(|edge| {
                if edge.source_index == 1
                    && edge.role == role
                    && let VectorCurve::Line { a, b } = edge.curve
                    && [a, b].into_iter().all(|p| p[1].abs() < 1e-8)
                {
                    return Some([a[0].min(b[0]), a[0].max(b[0])]);
                }
                None
            })
            .collect();
        ranges.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
        ranges.dedup_by(|a, b| (0..2).all(|i| (a[i] - b[i]).abs() < 1e-6));
        ranges
    };
    let visible = axial_spans(&axial, VectorRole::Visible);
    let hidden = axial_spans(&axial, VectorRole::Hidden);
    assert_eq!(visible.len(), 3);
    for (a, b) in visible
        .iter()
        .flatten()
        .copied()
        .zip([-8., -6., -4., 4., 6., 8.])
    {
        assert!((a - b).abs() < 1e-6);
    }
    for expected in [[-6., -4.], [4., 6.]] {
        assert!(
            hidden
                .iter()
                .any(|range| (0..2).all(|i| (range[i] - expected[i]).abs() < 1e-6))
        );
    }
    let retained = project_drawing(
        &[
            DrawingSolid {
                solid: torus,
                hatch: Some(multicad_kernel::DrawingHatch {
                    angle_rad: 0.,
                    spacing_mm: 0.5,
                    line_width_mm: 0.2,
                    pattern: multicad_core::materials::HatchPattern::Lines,
                }),
            },
            DrawingSolid {
                solid: &rear,
                hatch: None,
            },
        ],
        DrawingProjection {
            section_offset_mm: Some(0.6),
            section_depth_mm: Some(4.7),
            section_u_mm: Some([-9., 9.]),
            ..options
        },
    )
    .unwrap();
    let retained_stock = axial_spans(&retained, VectorRole::Visible);
    assert_eq!(retained_stock.len(), 3);
    for (actual, expected) in retained_stock
        .iter()
        .flatten()
        .zip([-8., -6., -4., 4., 6., 8.])
    {
        assert!((actual - expected).abs() < 1e-6, "{retained_stock:?}");
    }
    for (role, radii) in [
        (VectorRole::Section, [4.2, 5.8]),
        (VectorRole::Visible, [4., 6.]),
    ] {
        for radius in radii {
            assert!(retained.iter().any(|edge| edge.source_index == 0
                && edge.role == role
                && matches!(edge.curve, VectorCurve::Ellipse { major_mm, .. }
                    if (major_mm - radius).abs() < 1e-8)));
        }
    }

    let hatches: Vec<_> = retained
        .iter()
        .filter(|edge| edge.role == VectorRole::Hatch)
        .collect();
    assert!(!hatches.is_empty());
    for hatch in hatches {
        assert_eq!(hatch.source_index, 0);
        let VectorCurve::Line { a, b } = hatch.curve else {
            panic!("Expected straight hatch geometry");
        };
        let [u, v]: [f64; 2] = std::array::from_fn(|i| (a[i] + b[i]) / 2.);
        assert!((4.199..=5.801).contains(&u.hypot(v)));
    }

    let windowed_axial = project(
        &[torus, &rear],
        DrawingProjection {
            section_offset_mm: Some(2.),
            section_u_mm: Some([1.5, 3.]),
            ..options
        },
    );
    let axial_side: Vec<_> = visible_torus_side(&windowed_axial, 0, 3.)
        .flatten()
        .collect();
    assert!(
        axial_side
            .iter()
            .any(|y| (y.abs() - 7_f64.sqrt()).abs() < 1e-5)
    );
    assert!(
        axial_side
            .iter()
            .any(|y| (y.abs() - 27_f64.sqrt()).abs() < 1e-5)
    );
    assert!(axial_side.iter().all(|y| y.abs() >= 7_f64.sqrt() - 1e-5));
    let screen = PlaneFrame {
        u: [0., 1., 0.],
        v: [0., 0., 1.],
        ..Default::default()
    };
    let tilted_frame = PlaneFrame {
        u: [0.8, 0., 0.6],
        v: [0., 1., 0.],
        ..Default::default()
    };
    let normal = tilted_frame.normal();
    let rear_oblique = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [16, 1, 1].map(|length| Rational::new(length, 1).unwrap()),
        PlaneFrame {
            origin: std::array::from_fn(|i| -8. * tilted_frame.u[i] - 10. * normal[i]),
            ..tilted_frame
        },
        Default::default(),
    )
    .unwrap();
    let oblique = project(
        &[torus, &rear_oblique],
        DrawingProjection {
            frame: tilted_frame,
            ..options
        },
    );
    let visible_oblique = axial_spans(&oblique, VectorRole::Visible);
    assert_eq!(visible_oblique.len(), 3, "{visible_oblique:?}");
    for (actual, expected) in visible_oblique
        .into_iter()
        .flatten()
        .zip([-8., -5., -3., 3., 5., 8.])
    {
        assert!((actual - expected).abs() < 1e-6, "{actual} vs {expected}");
    }
    let steep_frame = PlaneFrame {
        u: [0.1, 0., 0.99_f64.sqrt()],
        ..tilted_frame
    };
    let steep = project(
        &[torus],
        DrawingProjection {
            frame: steep_frame,
            ..options
        },
    );
    assert!(steep.iter().any(|edge| edge.role == VectorRole::Visible));
    assert!(steep.iter().any(|edge| edge.role == VectorRole::Hidden));
    let c = 0.1_f64;
    let s = 0.99_f64.sqrt();
    let cosine = -0.5;
    let normal = c.hypot(s * cosine);
    let target = [
        -cosine * (c * 5. - 1. / normal),
        -0.75_f64.sqrt() * (5. - c / normal),
    ];
    let touches = |edge: &multicad_kernel::ProjectedVector, role| {
        edge.source_index == 0
            && edge.role == role
            && edge
                .preview_mm
                .iter()
                .any(|p| (p[0] - target[0]).hypot(p[1] - target[1]) < 0.025)
    };
    assert!(steep.iter().any(|edge| touches(edge, VectorRole::Hidden)));
    let retained = project(
        &[torus],
        DrawingProjection {
            frame: steep_frame,
            section_offset_mm: Some(-1.8),
            section_depth_mm: Some(1.4),
            section_u_mm: Some([-7., 7.]),
            section_v_mm: Some([-4.5, 4.5]),
            ..options
        },
    );
    assert!(
        retained
            .iter()
            .any(|edge| touches(edge, VectorRole::Visible))
    );
    assert!(retained.iter().any(|edge| edge.role == VectorRole::Section));
    assert!(visible_torus_side(&retained, 1, -4.5).next().is_some());
    let transverse_cut = DrawingProjection {
        frame: screen,
        section_only: true,
        section_offset_mm: Some(0.),
        ..options
    };
    verify_offset_torus_sections(torus, transverse_cut);
    verify_oblique_torus_sections(torus, transverse_cut);
    let behind = stock([-3., -8., 0.], [1, 16, 1]);
    let retained_transverse = project(
        &[torus, &behind],
        DrawingProjection {
            section_only: false,
            section_offset_mm: Some(-0.5),
            section_depth_mm: Some(2.5),
            section_u_mm: Some([-9., 9.]),
            ..transverse_cut
        },
    );
    assert!(retained_transverse.iter().any(|edge| edge.source_index == 0
        && edge.role == VectorRole::Section
        && matches!(edge.curve, VectorCurve::Line { .. })));
    assert!(retained_transverse.iter().any(|edge| edge.source_index == 0
        && edge.role == VectorRole::Visible
        && matches!(edge.curve, VectorCurve::Line { a, b }
            if (a[1] - 1.).abs() < 1e-8 && (b[1] - 1.).abs() < 1e-8)));
    let retained_visible = axial_spans(&retained_transverse, VectorRole::Visible);
    assert!(
        retained_visible
            .iter()
            .any(|[a, b]| (a + 12_f64.sqrt()).abs() < 1e-6 && (b - 12_f64.sqrt()).abs() < 1e-6),
        "{retained_visible:?}"
    );
    let windowed_transverse = project(
        &[torus],
        DrawingProjection {
            section_only: false,
            section_offset_mm: Some(-0.5),
            section_depth_mm: Some(2.5),
            section_u_mm: Some([-4.5, 7.]),
            ..transverse_cut
        },
    );
    assert!(
        visible_torus_side(&windowed_transverse, 0, -4.5)
            .next()
            .is_some()
    );
    let middle = stock([-1., -2., -2.], [2, 4, 1]);
    let transverse = project(&[torus, &middle], transverse_cut);
    assert_eq!(transverse.len(), 6);
    for x in [-5., 5.] {
        assert!(transverse.iter().any(|edge| edge.source_index == 0
            && edge.role == VectorRole::Section
            && matches!(edge.curve, VectorCurve::Ellipse {
                center: [u, v], major_mm: 1., minor_mm: 1., ..
            } if u == x && v == 0.)));
    }
    let clipped = project(
        &[torus],
        DrawingProjection {
            section_u_mm: Some([5., 7.]),
            ..transverse_cut
        },
    );
    assert!(
        clipped
            .iter()
            .any(|edge| matches!(edge.curve, VectorCurve::Line { a, b }
        if a[0] == 5. && b[0] == 5. && (a[1] - b[1]).abs() == 2.))
    );
    assert!(
        clipped
            .iter()
            .flat_map(|edge| &edge.preview_mm)
            .all(|p| p[0] >= 5. - 1e-7)
    );
    assert!(
        project(
            &[torus],
            DrawingProjection {
                section_offset_mm: Some(6.),
                ..transverse_cut
            }
        )
        .is_empty()
    );
    let front = stock([3., -7., -2.], [1, 14, 4]);
    let side = project(
        &[torus, &front],
        DrawingProjection {
            frame: screen,
            ..options
        },
    );
    assert!(side.iter().any(|edge| edge.source_index == 0
        && edge.role == VectorRole::Visible
        && matches!(edge.curve, VectorCurve::Line { a,b }
            if [a,b].into_iter().all(|p| (p[1]-1.).abs() < 1e-8)
                && (a[0].min(b[0])+3.).abs() < 1e-6 && (a[0].max(b[0])-3.).abs() < 1e-6)));
    let detail = project(
        &[torus],
        DrawingProjection {
            frame: screen,
            detail: Some(([0., 1.], 0.75)),
            ..options
        },
    );
    assert!(
        detail
            .iter()
            .any(|edge| matches!(edge.curve, VectorCurve::Line { a,b }
        if [a,b].into_iter().all(|p| (p[1]-1.).abs() < 1e-8)
            && (a[0].min(b[0])+0.75).abs() < 1e-6 && (a[0].max(b[0])-0.75).abs() < 1e-6))
    );

    let depth = project(
        &[torus],
        DrawingProjection {
            section_offset_mm: Some(0.6),
            section_depth_mm: Some(0.9),
            section_u_mm: Some([0., 7.]),
            ..options
        },
    );
    for role in [VectorRole::Section, VectorRole::Visible, VectorRole::Hidden] {
        assert!(depth.iter().any(|edge| edge.role == role));
    }
    assert!(
        depth
            .iter()
            .flat_map(|edge| &edge.preview_mm)
            .all(|p| p[0] >= -1e-7)
    );
    assert!(depth.iter().any(|edge| edge.role == VectorRole::Visible
        && matches!(edge.curve, VectorCurve::Ellipse { major_mm: 6., .. })));
    let stock = stock([7., 0., 0.], [2, 2, 1]);
    let mixed_cut = project(
        &[torus, &stock],
        DrawingProjection {
            section_only: true,
            section_offset_mm: Some(0.6),
            ..options
        },
    );
    assert!(
        mixed_cut
            .iter()
            .all(|edge| edge.role == VectorRole::Section)
    );
    assert_eq!(
        mixed_cut
            .iter()
            .filter(|edge| edge.source_index == 0)
            .count(),
        2
    );
    assert_eq!(
        mixed_cut
            .iter()
            .filter(|edge| edge.source_index == 1)
            .count(),
        4
    );

    #[cfg(feature = "occt")]
    {
        let directory = tempfile::tempdir().unwrap();
        let native = |solid: &Solid, name: &str| {
            let path = directory.path().join(name);
            solid.write_step(&path).unwrap();
            Solid::read_step(&path, Default::default()).unwrap()
        };
        let native_torus = native(torus, "torus.step");
        let bounds = native_torus.bounds().unwrap();
        let native_transverse = project(&[&native_torus], transverse_cut);
        for expected in [-5., 5.] {
            assert!(
                native_transverse
                    .iter()
                    .any(|edge| edge.role == VectorRole::Section
                        && matches!(edge.curve, VectorCurve::Ellipse {
                    center: [u, v], major_mm, minor_mm, ..
                } if (u - expected).abs() < 1e-6 && v.abs() < 1e-6
                    && (major_mm - 1.).abs() < 1e-6 && (minor_mm - 1.).abs() < 1e-6))
            );
        }
        assert!((bounds.min[0] + 6.).abs() < 1e-6 && (bounds.max[0] - 6.).abs() < 1e-6);
        let reference = project(&[&native_torus, &native(&rear, "stock.step")], options);
        let reference = axial_spans(&reference, VectorRole::Visible);
        assert!(reference.iter().any(|[lo, hi]| *lo < 0. && *hi > 0.));
        // The native projection can omit an exterior stock span at the
        // torus seam. The bounds and analytic intervals above certify
        // both exterior spans independently of the native HLR output.
        for expected in reference {
            assert!(
                visible.iter().any(|span| {
                    (0..2).all(|i| (span[i] - expected[i]).abs() < 3. * options.chord_tolerance_mm)
                }),
                "native stock interval {expected:?}"
            );
        }
    }
}

fn visible_torus_side(
    edges: &[multicad_kernel::ProjectedVector],
    axis: usize,
    level: f64,
) -> impl Iterator<Item = [f64; 2]> + '_ {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    edges.iter().filter_map(move |edge| match edge.curve {
        VectorCurve::Line { a, b }
            if edge.source_index == 0
                && edge.role == VectorRole::Visible
                && [a, b].into_iter().all(|p| (p[axis] - level).abs() < 1e-8) =>
        {
            Some([a[1 - axis], b[1 - axis]])
        }
        _ => None,
    })
}

fn verify_oblique_torus_sections(
    torus: &multicad_kernel::Solid,
    options: multicad_kernel::DrawingProjection,
) {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_kernel::{
        DrawingHatch, DrawingProjection, DrawingSolid, PlaneFrame, project_drawing,
    };
    let view = |s: f64, depth| DrawingProjection {
        frame: PlaneFrame {
            u: [(1. - s * s).sqrt(), 0., s],
            v: [0., 1., 0.],
            ..Default::default()
        },
        section_offset_mm: Some(depth),
        ..options
    };
    let input = DrawingSolid {
        solid: torus,
        hatch: None,
    };
    for (s, depth, loops) in [(0.6, 0., 2), (0.6, 3.5, 1), (0.1, 0., 2), (0.1, -0.8, 1)] {
        let result = project_drawing(std::slice::from_ref(&input), view(s, depth))
            .unwrap_or_else(|e| panic!("Oblique cut at inclination {s}, offset {depth}: {e}"));
        assert!(result.len() < 8_000);
        let c = (1. - s * s).sqrt();
        let mut first = None;
        let mut signed_area = 0.;
        let mut areas = Vec::new();
        for edge in &result {
            assert_eq!(edge.role, VectorRole::Section);
            let VectorCurve::Line { a, b } = edge.curve else {
                panic!("Expected bounded cut chords");
            };
            let [u, v]: [f64; 2] = std::array::from_fn(|i| a[i].midpoint(b[i]));
            let radius = (c * u - s * depth).hypot(v);
            let axial = s * u + c * depth;
            assert!(
                ((radius - 5.).hypot(axial) - 1.).abs() <= options.chord_tolerance_mm,
                "Inclination {s}, offset {depth}, chord midpoint ({u}, {v})"
            );
            signed_area += a[0] * b[1] - a[1] * b[0];
            if *first.get_or_insert(a) == b {
                areas.push(signed_area);
                signed_area = 0.;
                first = None;
            }
        }
        assert_eq!(areas.len(), loops);
        assert!(first.is_none());
        assert_eq!(
            areas.iter().filter(|&&a| a < 0.).count(),
            usize::from(s == 0.1 && depth == 0.)
        );
        // Sample the torus independently in its own axis coordinates,
        // intersecting each tube angle with the plane before projecting.
        for step in 0..180 {
            let angle = f64::from(step) * std::f64::consts::TAU / 180.;
            let (sin, cos) = angle.sin_cos();
            let radial = 5. + cos;
            let x = (c * sin - depth) / s;
            if x.abs() >= radial {
                continue;
            }
            let half = ((radial - x.abs()) * (radial + x.abs())).sqrt();
            for v in [-half, half] {
                let u = c * x + s * sin;
                assert!(
                    result.iter().any(|edge| {
                        let VectorCurve::Line { a, b } = edge.curve else {
                            return false;
                        };
                        let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
                        let [qx, qy] = [u - a[0], v - a[1]];
                        let t = ((qx * dx + qy * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
                        (qx - t * dx).hypot(qy - t * dy) <= options.chord_tolerance_mm
                    }),
                    "Inclination {s}, offset {depth}, exact cut point ({u}, {v})"
                );
            }
        }
    }
    let cropped = project_drawing(
        &[DrawingSolid {
            solid: torus,
            hatch: Some(DrawingHatch {
                angle_rad: 0.,
                spacing_mm: 0.25,
                line_width_mm: 0.05,
                pattern: multicad_core::materials::HatchPattern::Lines,
            }),
        }],
        DrawingProjection {
            section_u_mm: Some([-0.5, 4.]),
            ..view(0.1, 0.)
        },
    )
    .unwrap();
    let hatches: Vec<_> = cropped
        .iter()
        .filter(|edge| edge.role == VectorRole::Hatch)
        .collect();
    assert!(!hatches.is_empty());
    for edge in hatches {
        let VectorCurve::Line { a, b } = edge.curve else {
            panic!("Expected hatch lines");
        };
        let [u, v]: [f64; 2] = std::array::from_fn(|i| a[i].midpoint(b[i]));
        assert!(((0.99_f64.sqrt() * u).hypot(v) - 5.).hypot(0.1 * u) <= 1.001);
    }
    assert!(
        project_drawing(&[input], view(0.6, 4.1))
            .unwrap()
            .is_empty()
    );
}

fn verify_offset_torus_sections(
    torus: &multicad_kernel::Solid,
    options: multicad_kernel::DrawingProjection,
) {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_kernel::{DrawingHatch, DrawingProjection, DrawingSolid, project_drawing};

    let distance = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let d: [f64; 2] = std::array::from_fn(|i| b[i] - a[i]);
        let q: [f64; 2] = std::array::from_fn(|i| p[i] - a[i]);
        let t = ((q[0] * d[0] + q[1] * d[1]) / (d[0] * d[0] + d[1] * d[1])).clamp(0., 1.);
        (q[0] - t * d[0]).hypot(q[1] - t * d[1])
    };
    let input = DrawingSolid {
        solid: torus,
        hatch: None,
    };
    for (offset, loops) in [
        (2., 2),
        (3.9999, 2),
        (4., 2),
        (4.0001, 1),
        (4.5, 1),
        (5.5, 1),
        (5.9999, 1),
    ] {
        let vectors = project_drawing(
            std::slice::from_ref(&input),
            DrawingProjection {
                section_offset_mm: Some(offset),
                ..options
            },
        )
        .unwrap_or_else(|e| panic!("Offset {offset}: {e}"));
        assert!(
            vectors.len() < 5_000,
            "Offset {offset}: {} chords",
            vectors.len()
        );
        let mut closed = 0;
        let mut start = None;
        for vector in &vectors {
            assert_eq!(vector.role, VectorRole::Section);
            let VectorCurve::Line { a, b } = vector.curve else {
                panic!("Expected bounded section chords");
            };
            let [u, v]: [f64; 2] = std::array::from_fn(|i| (a[i] + b[i]) / 2.);
            assert!(
                ((u.hypot(offset) - 5.).hypot(v) - 1.).abs() <= options.chord_tolerance_mm,
                "Offset {offset}, chord midpoint ({u}, {v})"
            );
            if *start.get_or_insert(a) == b {
                closed += 1;
                start = None;
            }
        }
        assert_eq!(closed, loops, "Offset {offset}");
        assert_eq!(start, None);
        // Independent tube-angle samples, restricted to the radial portion
        // the cutting plane can actually reach.
        for step in 0..720 {
            let angle = f64::from(step) * std::f64::consts::TAU / 720.;
            let radial = 5. + angle.cos();
            if radial < offset {
                continue;
            }
            let x = ((radial - offset) * (radial + offset)).sqrt();
            for point in [[x, angle.sin()], [-x, angle.sin()]] {
                assert!(vectors.iter().any(|edge| matches!(edge.curve,
                    VectorCurve::Line { a, b } if distance(point, a, b) <= options.chord_tolerance_mm)),
                    "Offset {offset}, exact point {point:?}");
            }
        }
    }
    let cropped = project_drawing(
        &[DrawingSolid {
            solid: torus,
            hatch: Some(DrawingHatch {
                angle_rad: 0.,
                spacing_mm: 0.1,
                line_width_mm: 0.02,
                pattern: multicad_core::materials::HatchPattern::Lines,
            }),
        }],
        DrawingProjection {
            section_offset_mm: Some(4.5),
            section_u_mm: Some([-0.6, 2.]),
            section_v_mm: Some([-0.4, 0.7]),
            ..options
        },
    )
    .unwrap();
    assert!(cropped.iter().any(|edge| edge.role == VectorRole::Hatch));
    assert!(
        cropped
            .iter()
            .flat_map(|edge| &edge.preview_mm)
            .all(|[u, v]| (-0.600001..=2.000001).contains(u) && (-0.400001..=0.700001).contains(v))
    );
    assert!(
        cropped
            .iter()
            .filter(|edge| edge.role == VectorRole::Hatch)
            .all(|edge| {
                let [a, b] = &edge.preview_mm[..] else {
                    return false;
                };
                let u = (a[0] + b[0]) / 2.;
                let v = (a[1] + b[1]) / 2.;
                (u.hypot(4.5) - 5.).hypot(v) < 1.001
            })
    );
    let error = project_drawing(
        &[input],
        DrawingProjection {
            section_offset_mm: Some(2.),
            maximum_curves: 2,
            ..options
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, multicad_kernel::GeometryErrorKind::Limit);
}

#[test]
fn rust_annular_sketch_extrusion_preserves_both_circles_and_reverse_cap_frames() {
    let (sketch, edits) = Source::create(sketch::Datum::Xz, "Concentric circles").unwrap();
    let document = Document::new("Reverse annular sweep")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    let center = source.add_point([7., 9.]);
    let circles = [EntityId::new(), EntityId::new()];
    for (id, radius_mm) in circles.into_iter().zip([4., 1.5]) {
        source.sketch.curves.push(Curve {
            id,
            construction: false,
            kind: CurveKind::Circle { center, radius_mm },
        });
    }
    let region = multicad_core::sketch_regions::extract(&source)
        .regions
        .into_iter()
        .find(|region| region.holes.len() == 1)
        .unwrap()
        .intent();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let request = model::ExtrudeRequest::new(
        sketch,
        vec![region],
        ExtrudeParameters {
            direction: Direction {
                reverse: true,
                ..Default::default()
            },
            start_offset: length(2.),
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: length(6.),
                },
            },
            ..Default::default()
        },
    );
    let prepared = model::prepare_extrude(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let feature = request.outputs[0].feature;
    let reopened = reopen(&session);
    let native = compile(&reopened, GeometryBackend::OpenCascade);
    let rust = compile(&reopened, GeometryBackend::Rust);
    let actual = &rust.snapshot().shapes[&feature];
    let expected = &native.snapshot().shapes[&feature];
    near(actual.volume_mm3, PI * (16. - 2.25) * 6.);
    near(actual.volume_mm3, expected.volume_mm3);
    for (a, b) in actual
        .bounds
        .min
        .into_iter()
        .chain(actual.bounds.max)
        .zip(expected.bounds.min.into_iter().chain(expected.bounds.max))
    {
        near(a, b);
    }
    for face in expected
        .topology
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
    {
        let intent = GeometryIntent::capture(&expected.topology, face.key).unwrap();
        let Resolution::Resolved { key, .. } =
            references::resolve(&intent, &actual.topology, reopened.revision())
        else {
            panic!("Lost annular sketch face intent {intent:?}");
        };
        let exact = actual
            .topology
            .elements
            .iter()
            .find(|element| element.key == key)
            .unwrap();
        assert_eq!(exact.frames, face.frames);
    }
    for id in circles {
        assert!(actual.topology.elements.iter().any(|element| {
            element.key.kind == TopologyKind::Face
                && element.origins.iter().any(|origin| origin.source == id)
        }));
    }

    let original = reopened.document();
    let mut edited_source = Source::read(original, sketch).unwrap().0;
    let offset_center = edited_source.add_point([7.25, 9.]);
    edited_source
        .sketch
        .curves
        .iter_mut()
        .find(|curve| curve.id == circles[1])
        .unwrap()
        .kind = CurveKind::Circle {
        center: offset_center,
        radius_mm: 1.5,
    };
    let eccentric = original
        .edited(&edited_source.replace(original, sketch).unwrap())
        .unwrap();
    let Err(error) = compiler().compile_with_backend(&eccentric, 0, GeometryBackend::Rust, None)
    else {
        panic!("Rust annular extrusion accepted an eccentric bore");
    };
    assert!(
        error.message.contains("requires concentric circles"),
        "{error}"
    );
    assert_eq!(original, session.document());
}

#[test]
fn an_owner_cache_keeps_native_and_rust_solids_and_topology_separate() {
    let (session, _, extrusion, _, cutter, cut) = fixture(false, 28.);
    let revision = session.revision();
    let compiler = compiler();
    let mut cache = EvaluationCache::default();
    let producers = [extrusion, cutter, cut].into_iter().collect();
    let mut builds = Vec::new();
    for (offset, backend) in [
        GeometryBackend::OpenCascade,
        GeometryBackend::Rust,
        GeometryBackend::OpenCascade,
        GeometryBackend::Rust,
    ]
    .into_iter()
    .enumerate()
    {
        let revision = revision + offset as u64;
        let model = compiler
            .compile_cached_with_backend(&mut cache, session.document(), revision, backend, None)
            .unwrap();
        let stats = cache.last_compile();
        if offset < 2 {
            assert_eq!(stats.evaluated, producers);
            assert!(stats.reused.is_empty());
        } else {
            assert_eq!(stats.reused, producers);
            assert!(stats.evaluated.is_empty());
        }
        let rust = backend == GeometryBackend::Rust;
        assert_eq!(
            model.solid(cutter).unwrap().axial_cylinder().is_some(),
            rust
        );
        assert_eq!(model.solid(cut).unwrap().rust_prism().is_some(), rust);
        assert_eq!(model.snapshot().stamp.revision, revision);
        for shape in model.snapshot().shapes.values() {
            assert_eq!(shape.topology.revision, revision);
        }
        builds.push(model);
    }

    let topology = |i: usize| &builds[i].snapshot().shapes[&cutter].topology.elements;
    assert_ne!(topology(0), topology(1));
    assert_eq!(topology(0), topology(2));
    assert_eq!(topology(1), topology(3));
    let cutter_shape = |i: usize| &builds[i].snapshot().shapes[&cutter];
    assert!(!Arc::ptr_eq(&cutter_shape(0).mesh, &cutter_shape(1).mesh));
    for (first, reused) in [
        (cutter_shape(0), cutter_shape(2)),
        (cutter_shape(1), cutter_shape(3)),
    ] {
        assert!(Arc::ptr_eq(&first.mesh, &reused.mesh));
        assert!(Arc::ptr_eq(&first.edges, &reused.edges));
        assert!(Arc::ptr_eq(&first.vertices, &reused.vertices));
    }
    assert_eq!(
        builds[0].snapshot().shapes[&cutter].topology.revision,
        revision
    );
    assert_eq!(
        builds[1].snapshot().shapes[&cutter].topology.revision,
        revision + 1
    );
}

#[test]
fn completed_rust_model_reuse_requires_identical_source_revision_and_settings() {
    let (session, height, _, _, _, cut) = fixture(false, 28.);
    let document = session.document();
    let revision = session.revision();
    let compiler = compiler();
    let mut cache = EvaluationCache::default();
    let build = |cache: &mut EvaluationCache, compiler: Compiler, document, revision| {
        compiler
            .compile_cached_with_backend(cache, document, revision, GeometryBackend::Rust, None)
            .unwrap()
    };
    let cold = build(&mut cache, compiler, document, revision);
    assert!(!cache.last_compile().reused_full_model);
    let warm = build(&mut cache, compiler, document, revision);
    assert!(cache.last_compile().reused_full_model);
    assert!(std::ptr::eq(cold.snapshot(), warm.snapshot()));
    assert!(std::ptr::eq(
        cold.solid(cut).unwrap(),
        warm.solid(cut).unwrap()
    ));
    assert!(
        compiler
            .compile_cached_with_backend(
                &mut cache,
                document,
                revision,
                GeometryBackend::Rust,
                Some(&|| true),
            )
            .is_err()
    );
    assert!(!cache.last_compile().reused_full_model);

    let next = build(&mut cache, compiler, document, revision + 1);
    assert!(!cache.last_compile().reused_full_model);
    assert_eq!(next.snapshot().stamp.revision, revision + 1);
    let changed_settings = Compiler {
        omit_display: !compiler.omit_display,
        ..compiler
    };
    let _ = build(&mut cache, changed_settings, document, revision + 1);
    assert!(!cache.last_compile().reused_full_model);
    let mut parameter = document.get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "=10 mm / 3".into();
    let edited = document.edited(&[Mutation::Replace(parameter)]).unwrap();
    let fresh = build(&mut cache, changed_settings, &edited, revision + 1);
    assert!(!cache.last_compile().reused_full_model);
    assert_ne!(
        fresh.snapshot().stamp.source_hash,
        next.snapshot().stamp.source_hash
    );
    assert_ne!(
        fresh.solid(cut).unwrap().volume_mm3().unwrap(),
        next.solid(cut).unwrap().volume_mm3().unwrap()
    );
}

#[test]
fn cached_batch_cuts_each_retain_their_shared_tool_consumption() {
    let mut session = Session::new(Document::new("Two targets, one cutter").unwrap()).unwrap();
    let boxes = std::array::from_fn::<_, 2, _>(|_| {
        PrimitiveRequest::new(
            "Box",
            PrimitiveParameters::Box {
                size: [length(12.), length(8.), length(4.)],
                frame: PrimitiveFrame::default(),
            },
        )
    });
    let cutter = PrimitiveRequest::new(
        "Shared through-cutter",
        PrimitiveParameters::Cylinder {
            radius: length(1.),
            height: length(6.),
            frame: PrimitiveFrame {
                origin: [length(4.), length(4.), length(-1.)],
                ..PrimitiveFrame::default()
            },
        },
    );
    for request in boxes.iter().chain([&cutter]) {
        let edit =
            model::prepare_primitive(&session, session.revision(), actor(), request).unwrap();
        session.commit(edit.prepared).unwrap();
    }
    let cuts = boxes.map(|request| {
        model::BooleanRequest::new(
            model::Operation::Cut,
            model::BodyInput {
                body: request.body,
                producer: request.feature,
            },
            vec![model::BodyInput {
                body: cutter.body,
                producer: cutter.feature,
            }],
        )
    });
    let batch = model::prepare_booleans(&session, session.revision(), actor(), &cuts).unwrap();
    session.commit(batch.prepared).unwrap();

    let compiler = compiler();
    let revision = session.revision();
    let document = session.document();
    let mut cache = EvaluationCache::default();
    let mut compile = |document: &Document, revision: u64| {
        let result = compiler
            .compile_cached_with_backend(
                &mut cache,
                document,
                revision,
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        (result, cache.last_compile().clone())
    };
    assert!(
        compile(document, revision)
            .0
            .snapshot()
            .consumed_bodies
            .contains(&cutter.body)
    );
    for (removed, remaining) in [(&cuts[0], &cuts[1]), (&cuts[1], &cuts[0])] {
        let mut entity = document.get(removed.feature).unwrap().clone();
        let EntityData::Feature { suppressed, .. } = &mut entity.data else {
            unreachable!()
        };
        *suppressed = true;
        let edited = document.edited(&[Mutation::Replace(entity)]).unwrap();
        let (cached, stats) = compile(&edited, revision + 1);
        assert!(stats.reused.contains(&remaining.feature));
        let fresh = compiler
            .compile_with_backend(&edited, revision + 1, GeometryBackend::Rust, None)
            .unwrap();
        assert_eq!(cached.snapshot().bodies, fresh.snapshot().bodies);
        assert!(cached.snapshot().consumed_bodies.contains(&cutter.body));
    }
}

#[test]
fn a_scoped_rust_result_reuses_unchanged_tools_and_a_failed_edit_cannot_reuse_the_old_cut() {
    let (session, height, extrusion, body, cutter, cut) = fixture(false, 28.);
    let revision = session.revision();
    let document = session.document();
    let compiler = compiler();
    let mut cache = EvaluationCache::default();
    let scoped = compiler
        .compile_sources_cached_with_backend(
            &mut cache,
            document,
            revision,
            &[cutter],
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert_eq!(
        cache.last_compile().evaluated,
        [cutter].into_iter().collect()
    );
    assert_eq!(
        scoped.snapshot().scope,
        EvaluationScope::Sources(vec![cutter])
    );
    assert_eq!(scoped.snapshot().shapes.len(), 1);
    assert!(scoped.solid(cut).is_none());
    let first = compiler
        .compile_cached_with_backend(&mut cache, document, revision, GeometryBackend::Rust, None)
        .unwrap();
    assert_eq!(cache.last_compile().reused, [cutter].into_iter().collect());
    assert_eq!(
        cache.last_compile().evaluated,
        [extrusion, cut].into_iter().collect()
    );
    assert_eq!(first.snapshot().scope, EvaluationScope::Full);
    assert_eq!(first.snapshot().bodies.get(&body), Some(&cut));

    let with_height = |value: &str| {
        let mut parameter = document.get(height).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut parameter.data else {
            unreachable!()
        };
        *expression = value.into();
        document.edited(&[Mutation::Replace(parameter)]).unwrap()
    };
    let thinner = with_height("=10 mm / 3");
    // Two candidates can share a revision but have different geometry.
    let candidate_revision = revision + 1;
    let rebuilt = compiler
        .compile_cached_with_backend(
            &mut cache,
            &thinner,
            candidate_revision,
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert_eq!(cache.last_compile().reused, [cutter].into_iter().collect());
    assert_eq!(
        cache.last_compile().evaluated,
        [extrusion, cut].into_iter().collect()
    );
    let solid = rebuilt.solid(cut).unwrap();
    assert_eq!(solid.rust_prism().unwrap().dimensions()[2].ratio(), (10, 3));
    near(solid.volume_mm3().unwrap(), (800. + 25. * PI) * 10. / 3.);
    assert_eq!(
        rebuilt.snapshot().shapes[&cutter].topology.revision,
        candidate_revision
    );
    assert_eq!(first.snapshot().shapes[&cutter].topology.revision, revision);
    assert!(Arc::ptr_eq(
        &first.snapshot().shapes[&cutter].mesh,
        &rebuilt.snapshot().shapes[&cutter].mesh
    ));
    assert!(!Arc::ptr_eq(
        &first.snapshot().shapes[&cut].mesh,
        &rebuilt.snapshot().shapes[&cut].mesh
    ));
    assert_ne!(
        first.snapshot().stamp.source_hash,
        rebuilt.snapshot().stamp.source_hash
    );

    let blocked = with_height("10");
    let failure = compiler
        .compile_cached_with_backend(
            &mut cache,
            &blocked,
            candidate_revision,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("the unchanged tool does not pass through the thicker panel");
    assert_eq!(failure.entity, Some(cut));
    assert!(
        failure.message.contains("NotThrough"),
        "{}",
        failure.message
    );

    let retried = compiler
        .compile_cached_with_backend(
            &mut cache,
            &thinner,
            candidate_revision + 1,
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert!(cache.last_compile().evaluated.is_empty());
    assert_eq!(
        cache.last_compile().reused,
        [extrusion, cutter, cut].into_iter().collect()
    );
    assert_eq!(
        retried.snapshot().stamp.source_hash,
        rebuilt.snapshot().stamp.source_hash
    );
    assert_eq!(
        retried.snapshot().shapes[&cut].topology.revision,
        candidate_revision + 1
    );
    near(
        retried.solid(cut).unwrap().volume_mm3().unwrap(),
        solid.volume_mm3().unwrap(),
    );
}

#[test]
fn directed_draft_rebuilds_a_polygonal_hole_and_retains_cap_attachments() {
    let (mut session, feature, mut parameters) = rust_polygon::drafted_fixture();
    let anchor = Origin {
        source: feature,
        role: "cap.end".into(),
    };
    let rust = compile(&session, GeometryBackend::Rust);
    let topology = &rust.snapshot().shapes[&feature].topology;
    let key = topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.origins.contains(&anchor))
        .unwrap()
        .key;
    let attachment = sketch::prepare_new(
        &session,
        session.revision(),
        actor(),
        "Attached to swept end",
        CreationPlane::Face {
            snapshot: topology,
            key,
            anchor: Some(&anchor),
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let plane = attachment.plane;
    session.commit(attachment.prepared).unwrap();

    let distance = |value| Termination::Distance {
        distance: length(value),
    };
    let cases: [(Extent, bool, f64, f64, f64); 4] = [
        (parameters.extent.clone(), false, 5., 0., -6.),
        (parameters.extent.clone(), true, -5., 0., 6.),
        (Extent::Symmetric { total: length(10.) }, false, 0., -5., 5.),
        (
            Extent::TwoSided {
                forward: distance(7.),
                backward: distance(3.),
            },
            false,
            0.,
            -3.,
            7.,
        ),
    ];
    for (index, (extent, reverse, taper, start, end)) in cases.into_iter().enumerate() {
        if index > 0 {
            parameters.extent = extent;
            parameters.direction.reverse = reverse;
            parameters.taper = model::angle(taper);
            let edit = model::prepare_extrude_edit(
                &session,
                session.revision(),
                actor(),
                feature,
                &parameters,
            )
            .unwrap();
            session.commit(edit.prepared).unwrap();
        }

        let reopened = reopen(&session);
        let rust = compile(&reopened, GeometryBackend::Rust);
        let native = compile(&reopened, GeometryBackend::OpenCascade);
        let actual = &rust.snapshot().shapes[&feature];
        let expected = &native.snapshot().shapes[&feature];
        let path_mm = (end - start).abs();
        let normal_component = std::f64::consts::FRAC_1_SQRT_2;
        // Offsetting the 12×10 outline by d and the 4×4 hole by -d gives area 104 + 60d.
        let end_area = 104. + 60. * path_mm * taper.to_radians().tan();
        near(
            actual.volume_mm3,
            path_mm * normal_component * 104_f64.midpoint(end_area),
        );
        near(actual.volume_mm3, expected.volume_mm3);
        for (a, b) in actual
            .bounds
            .min
            .into_iter()
            .chain(actual.bounds.max)
            .zip(expected.bounds.min.into_iter().chain(expected.bounds.max))
        {
            near(a, b);
        }
        let rust_faces = rust.solid(feature).unwrap().faces().unwrap();
        let native_faces = native.solid(feature).unwrap().faces().unwrap();
        assert_eq!(rust_faces.len(), 10);
        for element in actual
            .topology
            .elements
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Face)
        {
            let counterpart = expected
                .topology
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.origins == element.origins)
                .unwrap_or_else(|| panic!("native face missing for {:?}", element.origins));
            let a = &rust_faces[element.key.ordinal as usize];
            let b = &native_faces[counterpart.key.ordinal as usize];
            near(a.area_mm2, b.area_mm2);
            for (a, b) in a.center_mm.into_iter().zip(b.center_mm) {
                near(a, b);
            }
        }
        let cap = sketch::inspect_plane(
            reopened.document(),
            plane,
            reopened.revision(),
            Some(&rust.snapshot().references),
        )
        .unwrap();
        near(cap.origin[0], normal_component * end);
        near(cap.origin[1], 0.);
        near(cap.origin[2], 2. + normal_component * end);
    }

    parameters.taper = model::angle(5.);
    let candidate =
        model::prepare_extrude_edit(&session, session.revision(), actor(), feature, &parameters)
            .unwrap();
    let failure = compiler()
        .compile_with_backend(
            candidate.prepared.document(),
            session.revision() + 1,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("A single-section sweep cannot build two-sided draft");
    assert_eq!(failure.entity, Some(feature));
    assert!(failure.message.contains("intermediate sweep section"));
}

#[test]
fn straight_sketch_reopens_and_matches_native_faces_with_a_polygonal_hole() {
    let (mut session, height, feature, hole_edge) = rust_polygon::fixture();
    let saved = reopen(&session);
    let face_intents = [
        Origin {
            source: feature,
            role: "cap.end".into(),
        },
        Origin {
            source: feature,
            role: format!("side/{hole_edge}/profile.edge"),
        },
    ];
    let planes = face_intents.map(|anchor| {
        let rust = compile(&session, GeometryBackend::Rust);
        let topology = &rust.snapshot().shapes[&feature].topology;
        let key = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face && element.origins.contains(&anchor)
            })
            .unwrap()
            .key;
        let attachment = sketch::prepare_new(
            &session,
            session.revision(),
            actor(),
            "Attached sketch",
            CreationPlane::Face {
                snapshot: topology,
                key,
                anchor: Some(&anchor),
            },
            PlaneOptions::default(),
        )
        .unwrap();
        let plane = attachment.plane;
        session.commit(attachment.prepared).unwrap();
        plane
    });
    let mut edit = session.document().get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut edit.data else {
        unreachable!()
    };
    *expression = "=10 mm / 3".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Fractional height".into(),
            edits: vec![Mutation::Replace(edit)],
        })
        .unwrap();
    for (session, height) in [(saved, 6.), (reopen(&session), 10. / 3.)] {
        let rust = compile(&session, GeometryBackend::Rust);
        let native = compile(&session, GeometryBackend::OpenCascade);
        let rust_solid = rust.solid(feature).unwrap();
        let native_solid = native.solid(feature).unwrap();
        near(rust_solid.volume_mm3().unwrap(), 92. * height);
        near(
            rust_solid.volume_mm3().unwrap(),
            native_solid.volume_mm3().unwrap(),
        );
        let bounds = rust_solid.bounds().unwrap();
        near(bounds.min[1], -2. - height);
        assert_eq!(bounds.max, [12., -2., 10.]);
        let actual = &rust.snapshot().shapes[&feature];
        let expected = &native.snapshot().shapes[&feature];
        assert_eq!(actual.edges.len(), 30);
        assert_eq!(actual.vertices.len(), 20);
        assert_eq!(
            actual
                .mesh
                .face_indices
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            12
        );
        let rust_faces = rust_solid.faces().unwrap();
        let native_faces = native_solid.faces().unwrap();
        for element in actual
            .topology
            .elements
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Face)
        {
            let corresponding = expected
                .topology
                .elements
                .iter()
                .find(|candidate| {
                    candidate.key.kind == TopologyKind::Face && candidate.origins == element.origins
                })
                .unwrap();
            assert_eq!(
                element.frames, corresponding.frames,
                "{:?}",
                element.origins
            );
            let a = &rust_faces[element.key.ordinal as usize];
            let b = &native_faces[corresponding.key.ordinal as usize];
            near(a.area_mm2, b.area_mm2);
            for (a, b) in a.center_mm.into_iter().zip(b.center_mm) {
                near(a, b);
            }
        }
        if session.document().get(planes[0]).is_some() {
            let inspect = |plane| {
                sketch::inspect_plane(
                    session.document(),
                    plane,
                    session.revision(),
                    Some(&rust.snapshot().references),
                )
                .unwrap()
            };
            near(inspect(planes[0]).origin[1], -2. - height);
            near(inspect(planes[1]).origin[1], -2.);
        }
    }
}

#[test]
fn a_saved_straight_face_extrusion_resolves_across_both_backends() {
    let (mut session, height, parent, _) = rust_polygon::fixture();
    let native = compile(&session, GeometryBackend::OpenCascade);
    let topology = &native.snapshot().shapes[&parent].topology;
    let cap = Origin {
        source: parent,
        role: "cap.end".into(),
    };
    let key = topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.origins.contains(&cap))
        .unwrap()
        .key;
    let face = model::capture_profile_face(topology, key, None).unwrap();
    let request = model::ExtrudeRequest::from_faces(
        vec![face],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: length(4.),
                },
            },
            ..Default::default()
        },
    );
    let feature = request.outputs[0].feature;
    let prepared = model::prepare_extrude(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = reopen(&session);
    let mut parameter = session.document().get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "7".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Move source cap".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();

    for (project, source_y) in [(original, -8.), (reopen(&session), -9.)] {
        let native = compile(&project, GeometryBackend::OpenCascade);
        let rust = compile(&project, GeometryBackend::Rust);
        let actual = &rust.snapshot().shapes[&feature];
        let expected = &native.snapshot().shapes[&feature];
        near(actual.volume_mm3, 92. * 4.);
        near(actual.volume_mm3, expected.volume_mm3);
        assert_eq!(actual.bounds.min, [0., source_y - 4., 0.]);
        assert_eq!(actual.bounds.max, [12., source_y, 10.]);
        let faces = |topology: &TopologySnapshot| {
            topology
                .elements
                .iter()
                .filter(|element| element.key.kind == TopologyKind::Face)
                .map(|element| element.origins.clone())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let actual_faces = faces(&actual.topology);
        assert_eq!(actual_faces.len(), 12);
        assert_eq!(actual_faces, faces(&expected.topology));
    }
}

#[test]
fn mixed_holes_keep_circular_caps_and_wall_ancestry_through_saved_edits() {
    let (mut session, height, parent, hole) = rust_polygon::fixture_with_circle();
    let native = compile(&session, GeometryBackend::OpenCascade);
    let source_topology = &native.snapshot().shapes[&parent].topology;
    let cap = Origin {
        source: parent,
        role: "cap.end".into(),
    };
    let wall = Origin {
        source: parent,
        role: format!("side/{hole}/profile.edge"),
    };
    let find_face = |topology: &TopologySnapshot, origin: &Origin| {
        topology
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(origin))
            .unwrap()
            .key
    };
    let wall_intent =
        GeometryIntent::capture(source_topology, find_face(source_topology, &wall)).unwrap();
    let face = model::capture_profile_face(source_topology, find_face(source_topology, &cap), None)
        .unwrap();
    let request = model::ExtrudeRequest::from_faces(
        vec![face],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: length(4.),
                },
            },
            ..Default::default()
        },
    );
    let child = request.outputs[0].feature;
    let prepared = model::prepare_extrude(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = reopen(&session);
    let mut parameter = session.document().get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        unreachable!()
    };
    *expression = "=10 mm / 3".into();
    let sketch = model::read_extrude(session.document(), parent)
        .unwrap()
        .profile
        .sketch_id()
        .unwrap();
    let mut sketch_source = Source::read(session.document(), sketch).unwrap().0;
    let CurveKind::Circle { radius_mm, .. } = &mut sketch_source
        .sketch
        .curves
        .iter_mut()
        .find(|curve| curve.id == hole)
        .unwrap()
        .kind
    else {
        unreachable!()
    };
    *radius_mm = 0.75;
    let mut edits = sketch_source.replace(session.document(), sketch).unwrap();
    edits.push(Mutation::Replace(parameter));
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Edit mixed-hole extrusion height and radius".into(),
            edits,
        })
        .unwrap();

    for (project, height, radius) in [(original, 6., 1.25_f64), (reopen(&session), 10. / 3., 0.75)]
    {
        let area = 92. - PI * radius.powi(2);
        let native = compile(&project, GeometryBackend::OpenCascade);
        let rust = compile(&project, GeometryBackend::Rust);
        let top_y = -2. - height;
        for (feature, height, end_y) in [(parent, height, top_y), (child, 4., top_y - 4.)] {
            let rust_solid = rust.solid(feature).unwrap();
            near(rust_solid.volume_mm3().unwrap(), area * height);
            near(
                rust_solid.volume_mm3().unwrap(),
                native.solid(feature).unwrap().volume_mm3().unwrap(),
            );
            let topology = &rust.snapshot().shapes[&feature].topology;
            let key = find_face(
                topology,
                &Origin {
                    source: feature,
                    role: "cap.end".into(),
                },
            );
            let contours = rust_solid
                .planar_face_contours(
                    key.ordinal,
                    PlaneFrame {
                        origin: [0., end_y, 0.],
                        u: [1., 0., 0.],
                        v: [0., 0., 1.],
                    },
                )
                .unwrap();
            assert_eq!(contours.loops.len(), 3);
            assert!(
                contours
                    .loops
                    .iter()
                    .any(|wire| !wire.outer && wire.edges.len() == 4)
            );
            let (start, end, center, sweep) = contours
                .loops
                .iter()
                .filter(|wire| !wire.outer)
                .find_map(|wire| match wire.edges.as_slice() {
                    [
                        PlanarEdge::Arc {
                            a,
                            b,
                            center,
                            sweep_rad,
                        },
                    ] => Some((*a, *b, *center, *sweep_rad)),
                    _ => None,
                })
                .unwrap();
            assert_eq!(start, end);
            assert_eq!(center, [9., 3.]);
            near((start[0] - center[0]).hypot(start[1] - center[1]), radius);
            near(sweep, -2. * PI);
        }

        let rust_topology = &rust.snapshot().shapes[&parent].topology;
        let rust_wall = find_face(rust_topology, &wall);
        let resolved = references::resolve(&wall_intent, rust_topology, project.revision());
        assert!(matches!(resolved, Resolution::Resolved { key, .. } if key == rust_wall));
        let rust_faces = rust.solid(parent).unwrap().faces().unwrap();
        let native_topology = &native.snapshot().shapes[&parent].topology;
        let native_wall = find_face(native_topology, &wall);
        let native_faces = native.solid(parent).unwrap().faces().unwrap();
        let wall_face = &rust_faces[rust_wall.ordinal as usize];
        let expected = &native_faces[native_wall.ordinal as usize];
        assert_eq!(wall_face.plane, None);
        near(wall_face.area_mm2, 2. * PI * radius * height);
        near(wall_face.area_mm2, expected.area_mm2);
        for (actual, expected) in wall_face.center_mm.into_iter().zip(expected.center_mm) {
            near(actual, expected);
        }
    }
}

#[test]
fn straight_xy_sketch_through_cut_keeps_body_head_and_cutter_provenance() {
    let (mut session, _, stock, _) = rust_polygon::build_fixture("6", true, sketch::Datum::Xy, 0.);
    let body = model::read_extrude(session.document(), stock)
        .unwrap()
        .output
        .body;
    let cutter = PrimitiveRequest::new(
        "Through-bore tool",
        PrimitiveParameters::Cylinder {
            radius: length(0.5),
            height: length(8.),
            frame: PrimitiveFrame {
                origin: [length(9.), length(1.), length(-1.)],
                ..Default::default()
            },
        },
    );
    let prepared =
        model::prepare_primitive(&session, session.revision(), actor(), &cutter).unwrap();
    session.commit(prepared.prepared).unwrap();
    let cut = model::BooleanRequest::new(
        model::Operation::Cut,
        model::BodyInput {
            body,
            producer: stock,
        },
        vec![model::BodyInput {
            body: cutter.body,
            producer: cutter.feature,
        }],
    );
    let prepared = model::prepare_boolean(&session, session.revision(), actor(), &cut).unwrap();
    session.commit(prepared.prepared).unwrap();

    let reopened = reopen(&session);
    let native = compile(&reopened, GeometryBackend::OpenCascade);
    let rust = compile(&reopened, GeometryBackend::Rust);
    assert_eq!(rust.snapshot().bodies, native.snapshot().bodies);
    assert_eq!(rust.snapshot().bodies.get(&body), Some(&cut.feature));
    assert_eq!(rust.snapshot().bodies.len(), 1);
    let actual = rust.solid(cut.feature).unwrap();
    let expected = native.solid(cut.feature).unwrap();
    near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
    near(
        actual.volume_mm3().unwrap(),
        (92. - PI * (1.25_f64.powi(2) + 0.5_f64.powi(2))) * 6.,
    );
    let topology = &rust.snapshot().shapes[&cut.feature].topology;
    let cutter_wall = topology
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face
                && element.origins.contains(&Origin {
                    source: cutter.feature,
                    role: "side.cylinder".into(),
                })
        })
        .unwrap();
    let faces = actual.faces().unwrap();
    let wall = &faces[cutter_wall.key.ordinal as usize];
    assert_eq!(wall.plane, None);
    assert_eq!(wall.center_mm, [9., 1., 3.]);
    near(wall.area_mm2, 6. * PI);
}

#[test]
fn signed_symmetric_and_two_sided_extrusions_preserve_saved_cap_and_side_frames() {
    use multicad_core::{constraint_sketch::Dimension, parameters::BoundExpression};

    let (mut session, height, feature, _) = rust_polygon::fixture_with_circle();
    let planes = ["cap.start", "cap.end"].map(|role| {
        let native = compile(&session, GeometryBackend::OpenCascade);
        let topology = &native.snapshot().shapes[&feature].topology;
        let anchor = Origin {
            source: feature,
            role: role.into(),
        };
        let key = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face && element.origins.contains(&anchor)
            })
            .unwrap()
            .key;
        let attachment = sketch::prepare_new(
            &session,
            session.revision(),
            actor(),
            role,
            CreationPlane::Face {
                snapshot: topology,
                key,
                anchor: Some(&anchor),
            },
            PlaneOptions::default(),
        )
        .unwrap();
        let plane = attachment.plane;
        session.commit(attachment.prepared).unwrap();
        plane
    });
    let mut dimension = session.document().get(height).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut dimension.data else {
        unreachable!()
    };
    *expression = "=10 mm / 3".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Set fractional sweep span".into(),
            edits: vec![Mutation::Replace(dimension)],
        })
        .unwrap();
    let distance = |value| Termination::Distance {
        distance: length(value),
    };
    let one = |value| Extent::OneSided {
        end: distance(value),
    };
    let symmetric = Extent::Symmetric {
        total: Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=$height", "mm", None).unwrap(),
        },
    };
    let two_sided = Extent::TwoSided {
        forward: distance(7.),
        backward: distance(3.),
    };
    let cases: [(Extent, bool, f64, f64); 7] = [
        (one(-6.), false, -2., 4.),
        (one(6.), true, -2., 4.),
        (one(-6.), true, -2., -8.),
        (symmetric.clone(), false, -1. / 3., -11. / 3.),
        (symmetric, true, -11. / 3., -1. / 3.),
        (two_sided.clone(), false, 1., -9.),
        (two_sided, true, -5., 5.),
    ];
    for (extent, reverse, start, end) in cases {
        let parameters = ExtrudeParameters {
            direction: Direction {
                reverse,
                ..Default::default()
            },
            start_offset: length(2.),
            extent,
            ..Default::default()
        };
        let edit = model::prepare_extrude_edit(
            &session,
            session.revision(),
            actor(),
            feature,
            &parameters,
        )
        .unwrap();
        session.commit(edit.prepared).unwrap();
        let reopened = reopen(&session);
        let rust = compile(&reopened, GeometryBackend::Rust);
        let native = compile(&reopened, GeometryBackend::OpenCascade);
        let actual = &rust.snapshot().shapes[&feature];
        let expected = &native.snapshot().shapes[&feature];
        near(
            actual.volume_mm3,
            (92. - 1.25_f64.powi(2) * PI) * (end - start).abs(),
        );
        near(actual.volume_mm3, expected.volume_mm3);
        near(actual.bounds.min[1], start.min(end));
        near(actual.bounds.max[1], start.max(end));
        let rust_faces = rust.solid(feature).unwrap().faces().unwrap();
        let native_faces = native.solid(feature).unwrap().faces().unwrap();
        assert_eq!(rust_faces.len(), native_faces.len());
        for element in actual
            .topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let counterpart = expected
                .topology
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.origins == element.origins)
                .unwrap();
            assert_eq!(element.frames.len(), counterpart.frames.len());
            for (a, b) in element.frames.iter().zip(&counterpart.frames) {
                assert_eq!(a.origin, b.origin);
                for (x, y) in a
                    .frame
                    .origin_mm
                    .into_iter()
                    .chain(a.frame.u)
                    .chain(a.frame.v)
                    .zip(
                        b.frame
                            .origin_mm
                            .into_iter()
                            .chain(b.frame.u)
                            .chain(b.frame.v),
                    )
                {
                    near(x, y);
                }
            }
            let a = &rust_faces[element.key.ordinal as usize];
            let b = &native_faces[counterpart.key.ordinal as usize];
            near(a.area_mm2, b.area_mm2);
            for (x, y) in a.center_mm.into_iter().zip(b.center_mm) {
                near(x, y);
            }
            match (a.plane, b.plane) {
                (Some((_, a)), Some((_, b))) => {
                    for (x, y) in a.into_iter().zip(b) {
                        near(x, y);
                    }
                }
                (None, None) => {}
                _ => panic!("face planarity differs for {:?}", element.origins),
            }
        }
        for (plane, y) in planes.into_iter().zip([start, end]) {
            let attached = sketch::inspect_plane(
                reopened.document(),
                plane,
                reopened.revision(),
                Some(&rust.snapshot().references),
            )
            .unwrap();
            near(attached.origin[1], y);
        }
    }
}

#[test]
fn saved_rounded_panel_rebuilds_and_moves_a_downstream_face_attachment() {
    for quarter_turn in [false, true] {
        let (mut session, height, extrusion, body, cutter, cut) = fixture(quarter_turn, 28.);
        let first = compile(&session, GeometryBackend::Rust);
        let native = compile(&session, GeometryBackend::OpenCascade);
        let exact = first.solid(cut).unwrap().rust_prism().unwrap();
        assert_eq!(exact.dimensions()[2].ratio(), (8, 1));
        let rust_cutter = first.solid(cutter).unwrap();
        let native_cutter = native.solid(cutter).unwrap();
        assert!(rust_cutter.axial_cylinder().is_some());
        near(
            rust_cutter.volume_mm3().unwrap(),
            native_cutter.volume_mm3().unwrap(),
        );
        let mut rust_faces = rust_cutter.faces().unwrap();
        let mut native_faces = native_cutter.faces().unwrap();
        let by_height = |a: &multicad_kernel::Face, b: &multicad_kernel::Face| {
            a.center_mm[2].total_cmp(&b.center_mm[2])
        };
        rust_faces.sort_by(by_height);
        native_faces.sort_by(by_height);
        assert_eq!(rust_faces.len(), native_faces.len());
        for (actual, expected) in rust_faces.into_iter().zip(native_faces) {
            near(actual.area_mm2, expected.area_mm2);
            for (a, b) in actual.center_mm.into_iter().zip(expected.center_mm) {
                near(a, b);
            }
        }
        let cylinder_cap = |model: &Model| {
            model.snapshot().shapes[&cutter]
                .topology
                .elements
                .iter()
                .flat_map(|element| &element.frames)
                .find(|anchor| anchor.origin.source == cutter && anchor.origin.role == "cap.end")
                .unwrap()
                .frame
        };
        assert_eq!(cylinder_cap(&first), cylinder_cap(&native));
        assert_eq!(first.snapshot().bodies.get(&body), Some(&cut));
        assert_eq!(first.snapshot().bodies.len(), 1);
        near(
            first.solid(cut).unwrap().volume_mm3().unwrap(),
            native.solid(cut).unwrap().volume_mm3().unwrap(),
        );
        let topology = &first.snapshot().shapes[&cut].topology;
        let cap = Origin {
            source: extrusion,
            role: "cap.end".into(),
        };
        let key = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face && element.origins.contains(&cap)
            })
            .unwrap()
            .key;
        let attachment = sketch::prepare_new(
            &session,
            session.revision(),
            actor(),
            "Cap sketch",
            CreationPlane::Face {
                snapshot: topology,
                key,
                anchor: Some(&cap),
            },
            PlaneOptions::default(),
        )
        .unwrap();
        near(attachment.frame.origin[2], 8.);
        let plane = attachment.plane;
        session.commit(attachment.prepared).unwrap();
        session = reopen(&session);
        let mut edit = session.document().get(height).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut edit.data else {
            unreachable!()
        };
        *expression = "=10 mm / 3".into();
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Fractional thickness".into(),
                edits: vec![Mutation::Replace(edit)],
            })
            .unwrap();
        session = reopen(&session);
        let second = compile(&session, GeometryBackend::Rust);
        let native = compile(&session, GeometryBackend::OpenCascade);
        let solid = second.solid(cut).unwrap();
        let exact = solid.rust_prism().unwrap();
        assert_eq!(exact.dimensions()[2], parse_length_mm("10/3 mm").unwrap());
        near(solid.volume_mm3().unwrap(), (800. + 25. * PI) * 10. / 3.);
        near(
            solid.volume_mm3().unwrap(),
            native.solid(cut).unwrap().volume_mm3().unwrap(),
        );
        let probe = [28, 10, 2].map(|value| Rational::new(value, 1).unwrap());
        assert_eq!(exact.classify(probe), Ok(Location::Exterior));
        let attached = sketch::inspect_plane(
            session.document(),
            plane,
            session.revision(),
            Some(&second.snapshot().references),
        )
        .unwrap();
        near(attached.origin[2], 10. / 3.);
    }
}

#[test]
fn a_tangent_bore_is_rejected_without_native_fallback() {
    let (session, _, _, _, _, cut) = fixture(false, 19.);
    let result = compiler()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(result.entity, Some(cut));
    assert!(result.message.contains("HoleContact"), "{}", result.message);
    let cancel = || true;
    let cancelled = compiler()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            Some(&cancel),
        )
        .err()
        .unwrap();
    assert!(multicad_model::is_superseded(&cancelled));
}

#[test]
fn sphere_round_trips_analytic_step_and_projects_exact_outline() {
    use multicad_core::drawings::{VectorCurve, VectorRole};
    use multicad_core::exchange::{self, ExportResult, SetDefinition};
    use multicad_kernel::{
        DrawingProjection, DrawingSolid, GeometryPolicy, Solid, project_drawing,
        rust_kernel::step_read::read_analytic_solid,
    };
    use multicad_model::export::{ExportWorker, Exporter};
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    let mut session = Session::new(Document::new("Sphere exchange").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Sphere",
        PrimitiveParameters::Sphere {
            radius: length(4.5),
            frame: PrimitiveFrame {
                origin: [17., 4., -12.].map(length),
                u: [0.8, 0., -0.6],
                v: [0., 1., 0.],
            },
        },
    );
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let rust = compile(&session, GeometryBackend::Rust);
    let solid = rust.solid(stock.feature).unwrap();
    assert!(solid.supports_step_export());
    let set = EntityId::new();
    let saved = exchange::prepare_set(
        &session,
        session.revision(),
        actor(),
        set,
        "Sphere STEP",
        &SetDefinition::default(),
    )
    .unwrap();
    session.commit(saved).unwrap();
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    let wait = || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(reply) = worker.poll() {
                break reply.result;
            }
            assert!(
                Instant::now() < deadline,
                "Sphere STEP worker did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    worker
        .preview(session.document(), session.revision(), set, actor())
        .unwrap();
    let ExportResult::Preview(preview) = wait() else {
        panic!("expected STEP preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run());
    let directory = tempfile::tempdir().unwrap();
    worker
        .run(
            session.document(),
            session.revision(),
            &preview.key,
            directory.path(),
            actor(),
        )
        .unwrap();
    let ExportResult::Run(receipt) = wait() else {
        panic!("expected verified STEP publication");
    };
    let receipt = receipt.unwrap();
    let [file] = receipt.files.as_slice() else {
        panic!("expected one published sphere");
    };
    let destination = Path::new(&receipt.directory).join(&file.artifact.output.filename);
    let source = std::fs::read_to_string(&destination).unwrap();
    let policy = GeometryPolicy::default();
    let decoded = read_analytic_solid(&source, policy.linear_tolerance_mm).unwrap();
    assert!(Solid::step_topology_matches(
        &rust.snapshot().shapes[&stock.feature].topology,
        &decoded,
        policy.linear_tolerance_mm,
    ));
    let imported = Solid::import_rust_step(&source, stock.feature, policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 1);
    assert_eq!(imported.bounds().unwrap(), solid.bounds().unwrap());
    near(imported.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
    near(
        imported.faces().unwrap()[0].area_mm2,
        solid.faces().unwrap()[0].area_mm2,
    );
    assert_eq!(
        imported.display_vertices().unwrap(),
        solid.display_vertices().unwrap()
    );

    #[cfg(feature = "occt")]
    {
        let native = Solid::read_step(&destination, policy).unwrap();
        assert_eq!(native.solid_count().unwrap(), 1);
        let bounds = native.bounds().unwrap();
        for (actual, expected) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip(decoded.bounds_mm.into_iter().flatten())
        {
            near(actual, expected);
        }
        near(native.volume_mm3().unwrap(), decoded.volume_mm3);
        near(
            native.faces().unwrap()[0].area_mm2,
            solid.faces().unwrap()[0].area_mm2,
        );
        let oracle = compile(&session, GeometryBackend::OpenCascade);
        let oracle = oracle.solid(stock.feature).unwrap();
        let oracle_path = directory.path().join("native-sphere.step");
        oracle.write_step(&oracle_path).unwrap();
        let independent = std::fs::read_to_string(oracle_path).unwrap();
        let imported = Solid::import_rust_step(&independent, stock.feature, policy).unwrap();
        near(imported.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
        assert_eq!(imported.bounds().unwrap(), solid.bounds().unwrap());
    }
    let input = DrawingSolid { solid, hatch: None };
    let outlines = project_drawing(&[input], DrawingProjection::default()).unwrap();
    let [outline] = outlines.as_slice() else {
        panic!("expected the sphere's single exact silhouette");
    };
    assert_eq!(outline.role, VectorRole::Visible);
    assert!(matches!(
        outline.curve,
        VectorCurve::Ellipse {
            center: [17., 4.],
            major_mm: 4.5,
            minor_mm: 4.5,
            ..
        }
    ));
}

#[test]
fn saved_placed_box_keeps_native_face_ancestry_and_rebuilds_after_edit() {
    let mut session = Session::new(Document::new("Placed box").unwrap()).unwrap();
    let frame = PrimitiveFrame {
        origin: [length(7.), length(-3.), length(2.)],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let box_parameters = |height| PrimitiveParameters::Box {
        size: [length(12.), length(5.), length(height)],
        frame: frame.clone(),
    };
    let stock = PrimitiveRequest::new("Box", box_parameters(8.));
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = reopen(&session);
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        stock.feature,
        &box_parameters(9.),
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    for (document, height) in [(original, 8.), (reopen(&session), 9.)] {
        let rust = compile(&document, GeometryBackend::Rust);
        let native = compile(&document, GeometryBackend::OpenCascade);
        let solid = rust.solid(stock.feature).unwrap();
        let exact = solid.rust_prism().unwrap();
        assert_eq!(
            exact.dimensions()[2],
            Rational::new(height as i64, 1).unwrap()
        );
        assert_eq!(solid.bounds().unwrap().min, [7., -3., 2.]);
        assert_eq!(solid.bounds().unwrap().max, [7. + height, 9., 7.]);
        near(solid.volume_mm3().unwrap(), 60. * height);
        let rust_shape = &rust.snapshot().shapes[&stock.feature];
        let native_shape = &native.snapshot().shapes[&stock.feature];
        assert_eq!(rust_shape.mesh.indices.len(), 36);
        assert_eq!(rust_shape.edges.len(), 12);
        assert_eq!(rust_shape.vertices.len(), 8);
        let rust_faces = solid.faces().unwrap();
        let native_faces = native.solid(stock.feature).unwrap().faces().unwrap();
        for actual in rust_shape
            .topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let expected = native_shape
                .topology
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Face && e.origins == actual.origins)
                .unwrap();
            assert_eq!(actual.frames, expected.frames);
            let a = &rust_faces[actual.key.ordinal as usize];
            let b = &native_faces[expected.key.ordinal as usize];
            near(a.area_mm2, b.area_mm2);
            for (a, b) in a.center_mm.into_iter().zip(b.center_mm) {
                near(a, b);
            }
        }
    }
}

#[test]
fn rust_document_publishes_mesh_and_picking_keys_in_the_normal_snapshot() {
    let (session, _, _, body, cutter, cut) = fixture(false, 28.);
    let model = compile(&session, GeometryBackend::Rust);
    assert_eq!(model.snapshot().bodies.get(&body), Some(&cut));
    for id in [cutter, cut] {
        let shape = &model.snapshot().shapes[&id];
        assert!(!shape.mesh.indices.is_empty());
        assert!(!shape.edges.is_empty());
        let expected = |kind| {
            shape
                .topology
                .elements
                .iter()
                .filter(|e| e.key.kind == kind)
                .map(|e| e.key.ordinal)
                .collect::<std::collections::BTreeSet<_>>()
        };
        let actual = shape.mesh.face_indices.iter().copied().collect();
        assert_eq!(expected(TopologyKind::Face), actual);
        assert_eq!(
            expected(TopologyKind::Edge),
            shape.edges.iter().map(|e| e.index).collect()
        );
        assert_eq!(
            expected(TopologyKind::Vertex),
            shape.vertices.iter().map(|v| v.index).collect()
        );
    }
    let hidden = Compiler {
        omit_display: true,
        ..Compiler::default()
    }
    .compile_with_backend(
        session.document(),
        session.revision(),
        GeometryBackend::Rust,
        None,
    )
    .unwrap();
    let hidden_shape = &hidden.snapshot().shapes[&cut];
    assert!(hidden_shape.mesh.indices.is_empty());
    assert_eq!(
        hidden_shape.topology,
        model.snapshot().shapes[&cut].topology
    );
}
