use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{CurveKind, source::Source},
    document::{Document, EntityData, Mutation},
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
use multicad_model::{Compiler, GeometryBackend, Model};
use std::{f64::consts::PI, io::Cursor};

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
fn unsupported_documents_return_explicit_errors() {
    let mut session = Session::new(Document::new("Unsupported Rust primitive").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Sphere",
        PrimitiveParameters::Sphere {
            radius: length(1.),
            frame: PrimitiveFrame::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = compile(&session, GeometryBackend::OpenCascade);
    near(
        native.solid(stock.feature).unwrap().volume_mm3().unwrap(),
        4. * PI / 3.,
    );
    let rust = compiler()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(rust.entity, Some(stock.feature));
    assert!(rust.message.contains("box and axial cylinder"));
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
