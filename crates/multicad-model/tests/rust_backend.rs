use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    project_file::{self, FileLimits, ProjectFile},
    references::{Origin, TopologyKind, TopologySnapshot},
    sketch::{self, CreationPlane, PlaneOptions},
};
use multicad_kernel::rust_kernel::{Rational, parse_length_mm, prism::Location};
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
fn reverse_straight_sketch_sweep_remains_explicitly_unsupported() {
    let (session, _, feature, _) = rust_polygon::fixture_with_height("-6");
    let native = compile(&session, GeometryBackend::OpenCascade);
    near(native.solid(feature).unwrap().volume_mm3().unwrap(), 552.);
    let rust = compiler()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(rust.entity, Some(feature));
    assert!(rust.message.contains("positive normal sweep"));
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
