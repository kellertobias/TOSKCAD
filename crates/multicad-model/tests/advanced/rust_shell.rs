use super::common::*;
use multicad_core::{
    commands::{Request, Session},
    document::{Entity, EntityData, EntityId, Link, Mutation},
    model::{self, AdvancedParameters, AdvancedRequest, ShellParameters},
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, Origin, ReferenceAddress, Resolution,
    },
};
use multicad_kernel::{Bounds, GeometryPolicy, Solid};
use multicad_model::{Compiler, GeometryBackend, Model};

fn compile_rust(session: &Session) -> Model {
    Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap()
}

fn face_metrics(solid: &Solid) -> Vec<[i64; 8]> {
    let quantize = |value: f64| (value * 1e6).round() as i64;
    let mut result: Vec<_> = solid
        .faces()
        .unwrap()
        .into_iter()
        .map(|face| {
            let (point, normal) = face
                .plane
                .expect("rectangular shell must have planar faces");
            let level = point
                .into_iter()
                .zip(normal)
                .map(|(p, n)| p * n)
                .sum::<f64>();
            let [x, y, z] = normal;
            let [a, b, c] = face.center_mm;
            [x, y, z, level, face.area_mm2, a, b, c].map(quantize)
        })
        .collect();
    result.sort_unstable();
    result
}

#[test]
fn saved_rust_shell_rebuilds_signed_walls_and_linked_occurrences() {
    let (mut session, input) = fixture();
    let source = compile_rust(&session);
    let original = &source.snapshot().shapes[&input.producer].topology;
    let top = original.elements.iter().find(|element| {
        matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal() == [0.,0.,1.])
    }).unwrap();
    let opening = GeometryIntent::capture(original, top.key).unwrap();
    let selection_id = opening.id;
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Shell(ShellParameters {
            thickness: bound(&session, "wall", "mm"),
        }),
        vec![opening],
    );
    let p = model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(p.prepared).unwrap();

    let original = compile_rust(&session);
    let topology = &original.snapshot().shapes[&request.feature].topology;
    let opening = Origin {
        source: request.feature,
        role: format!("face/{selection_id}"),
    };
    let offset = Origin {
        source: request.feature,
        role: format!("offset/{}/cap.start", input.producer),
    };
    let find = |origin: &Origin| {
        topology
            .elements
            .iter()
            .find(|item| {
                item.origins.contains(origin)
                    && matches!(item.geometry, Some(AnalyticGeometry::Plane { .. }))
            })
            .unwrap()
    };
    let rim = find(&opening);
    let mut rim_intent = GeometryIntent::capture(topology, rim.key).unwrap();
    rim_intent.attachment = Some(Attachment {
        anchor: opening,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let reference = rim_intent.id;
    let datum = Entity::new(
        "Opening datum",
        EntityData::Plane {
            origin_mm: [0., 0., 10.],
            normal: [0., 0., 1.],
            anchor: Some(Link {
                target: request.feature,
                role: "face_attachment".into(),
                geometry: Some(rim_intent),
            }),
        },
    );
    let owner = datum.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach opening datum".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();

    let mut offset_intent = GeometryIntent::capture(topology, find(&offset).key).unwrap();
    offset_intent.attachment = Some(Attachment {
        anchor: offset,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });

    let retained = [
        references::TopologyKind::Face,
        references::TopologyKind::Edge,
        references::TopologyKind::Vertex,
    ]
    .map(|kind| {
        let element = topology
            .elements
            .iter()
            .find(|e| {
                e.key.kind == kind
                    && match e.geometry {
                        Some(AnalyticGeometry::Plane { frame }) => {
                            frame.normal() == [0., 0., -1.] && frame.origin_mm[2] == 0.
                        }
                        Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                            [a_mm, b_mm] == [[0.; 3], [0., 0., 10.]]
                                || [b_mm, a_mm] == [[0.; 3], [0., 0., 10.]]
                        }
                        Some(AnalyticGeometry::Point { position_mm }) => position_mm == [0.; 3],
                        _ => false,
                    }
            })
            .unwrap();
        GeometryIntent::capture(topology, element.key).unwrap()
    });

    for (width, thickness, volume, min, max) in [
        (30., 2., 2672., [0.; 3], [30., 20., 10.]),
        (40., 2., 3392., [0.; 3], [40., 20., 10.]),
        (40., -2., 4672., [-2.; 3], [42., 22., 10.]),
    ] {
        apply_parameter(&mut session, "width", &width.to_string());
        apply_parameter(&mut session, "wall", &thickness.to_string());
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let doc = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        let reopened = Session::new(doc).unwrap();
        let rebuilt = compile_rust(&reopened);
        let solid = rebuilt.solid(input.body).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.bounds().unwrap(), Bounds { min, max });
        let expected = face_metrics(solid);
        assert_eq!(expected.len(), 11);
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), volume);
        assert_eq!(rebuilt.snapshot().occurrences.len(), 1);
        for &id in rebuilt.snapshot().occurrences.keys() {
            let linked = rebuilt.solid(id).unwrap();
            near(linked.volume_mm3().unwrap(), volume);
            assert_eq!(linked.bounds().unwrap().min[0], min[0] + 80.);
        }
        let frames = |model: &Model| {
            let refs = &model.snapshot().references;
            assert!(refs.all_resolved());
            let Resolution::Resolved {
                frame: Some(rim), ..
            } = refs
                .get(ReferenceAddress { owner, reference })
                .unwrap()
                .resolution
            else {
                panic!("missing opening datum");
            };
            assert_eq!(rim.normal(), [0., 0., 1.]);
            near(rim.origin_mm[2], 10.);
            let topology = &model.snapshot().shapes[&request.feature].topology;
            let Resolution::Resolved {
                frame: Some(base), ..
            } = references::resolve(&offset_intent, topology, reopened.revision())
            else {
                panic!("missing named offset face");
            };
            for intent in &retained {
                let resolved = references::resolve(intent, topology, reopened.revision());
                assert!(
                    matches!(resolved, Resolution::Resolved { .. }),
                    "{intent:?}: {resolved:?}"
                );
            }
            near(base.origin_mm[2], thickness);
            assert_eq!(base.normal(), [0., 0., thickness.signum()]);
        };
        frames(&rebuilt);
        if cfg!(feature = "occt") {
            let native = Compiler::default()
                .compile_with_backend(
                    reopened.document(),
                    reopened.revision(),
                    GeometryBackend::OpenCascade,
                    None,
                )
                .unwrap();
            let native_solid = native.solid(input.body).unwrap();
            assert_eq!(face_metrics(native_solid), expected);
            near(native_solid.volume_mm3().unwrap(), volume);
            assert_eq!(native_solid.solid_count().unwrap(), 1);
            frames(&native);
        }
        if thickness < 0. {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("rectangular-shell.step");
            solid.write_step(&path).unwrap();
            let reopened = Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                EntityId::new(),
                GeometryPolicy::default(),
            )
            .unwrap();
            assert_eq!(face_metrics(&reopened), expected);
            near(reopened.volume_mm3().unwrap(), volume);
        }
    }
    let before = session.document().clone();
    let revision = session.revision();
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Shell(p) = &mut edit.parameters else {
        unreachable!()
    };
    p.thickness = model::length(1000.);
    let candidate = model::prepare_advanced_edit(&session, revision, actor(), &edit).unwrap();
    let failure = Compiler::default()
        .compile_with_backend(
            candidate.prepared.document(),
            revision,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(failure.entity, Some(request.feature));
    assert!(failure.message.contains("leaves no cavity"));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
}

#[test]
fn rust_shell_supports_placed_side_openings_and_rejects_multiple_or_curved_openings() {
    use multicad_kernel::{
        GeometryErrorKind, PlaneFrame, TreatmentFace,
        rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    };

    let policy = GeometryPolicy::default();
    let n = |value| Rational::new(value, 1).unwrap();
    let id = EntityId::new();
    let frame = PlaneFrame {
        origin: [17., 4., 3.],
        u: [0.6, 0.8, 0.],
        v: [-0.8, 0.6, 0.],
    };
    let solid = Solid::rust_box(id.uuid_bits(), [30, 20, 10].map(n), frame, policy).unwrap();
    let feature = EntityId::new();
    let selection = |ordinal, role| TreatmentFace {
        key: references::TopologyKey {
            kind: references::TopologyKind::Face,
            ordinal,
        },
        origin: Origin {
            source: feature,
            role,
        },
    };
    let cap = selection(5, "side-opening".into());
    for (wall, volume) in [(2., 3312.), (-2., 4752.)] {
        let result = solid
            .shell_named(std::slice::from_ref(&cap), wall, feature, policy)
            .unwrap();
        near(result.volume_mm3().unwrap(), volume);
        let snapshot = result.topology_snapshot(feature, 1).unwrap();
        let opening = snapshot
            .elements
            .iter()
            .find(|e| {
                matches!(e.geometry, Some(AnalyticGeometry::Plane { .. }))
                    && e.origins.contains(&cap.origin)
            })
            .unwrap();
        let Some(AnalyticGeometry::Plane { frame: rim }) = opening.geometry else {
            unreachable!()
        };
        for (actual, expected) in rim.normal().into_iter().zip([-0.6, -0.8, 0.]) {
            near(actual, expected);
        }
        let gap = std::array::from_fn::<_, 3, _>(|i| rim.origin_mm[i] - frame.origin[i])
            .into_iter()
            .zip(rim.normal())
            .map(|(a, b)| a * b)
            .sum();
        near(gap, 0.);
    }
    let multiple = solid
        .shell_named(
            &[cap, selection(1, "top-opening".into())],
            2.,
            feature,
            policy,
        )
        .err()
        .unwrap();
    assert_eq!(multiple.kind, GeometryErrorKind::Unsupported);
    assert!(multiple.message.contains("exactly one opening"));
    let cylinder = AxialCylinder::new(
        id.uuid_bits(),
        Circle::new([n(0); 2], n(4)).unwrap(),
        n(0),
        n(8),
    )
    .unwrap();
    let curved = Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), policy).unwrap();
    let error = curved
        .shell_named(&[selection(1, "top-opening".into())], 2., feature, policy)
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("one rectangular prism"));
}
