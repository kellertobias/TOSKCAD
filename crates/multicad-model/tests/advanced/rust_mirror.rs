use super::common::*;
use multicad_core::{
    commands::{Request, Session},
    document::{Entity, EntityData, Link, Mutation},
    model::{self, AdvancedParameters, AdvancedRequest, MirrorParameters, PrimitiveFrame},
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, ReferenceAddress, Resolution,
    },
};
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn saved_cardinal_and_oblique_mirrors_rebuild_bound_plane_and_retain_authored_references() {
    saved_mirror([1., 0., 0.], [0., 1., 0.]);
    saved_mirror([0.8, 0., -0.6], [0., 1., 0.]);
}

fn saved_mirror(u: [f64; 3], v: [f64; 3]) {
    let normal = multicad_kernel::PlaneFrame {
        u,
        v,
        ..Default::default()
    }
    .normal();
    let vector = |p: [f64; 3]| -> [f64; 3] {
        let d = 2. * p.into_iter().zip(normal).map(|(a, b)| a * b).sum::<f64>();
        std::array::from_fn(|i| p[i] - d * normal[i])
    };
    let cap_normal = vector([0., 0., 1.]);
    let (mut session, input) = fixture();
    let compiler = Compiler::default();
    let build = |session: &Session, backend| {
        compiler
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap()
    };
    let original = build(&session, GeometryBackend::Rust);
    let topology = &original.snapshot().shapes[&input.producer].topology;
    let captures: Vec<_> = topology
        .elements
        .iter()
        .map(|element| {
            (
                GeometryIntent::capture(topology, element.key).unwrap(),
                element.geometry.unwrap(),
            )
        })
        .collect();
    assert_eq!(captures.len(), 26);

    let plane = PrimitiveFrame {
        origin: [
            model::length(0.),
            model::length(0.),
            bound(&session, "mirror_z", "mm"),
        ],
        u,
        v,
    };
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Mirror(MirrorParameters { plane }),
        vec![],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    near(
        build(&session, GeometryBackend::Rust)
            .solid(input.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        6000.,
    );
    session.redo(session.revision(), actor()).unwrap();
    let mirrored = build(&session, GeometryBackend::Rust);
    let reflected = &mirrored.snapshot().shapes[&request.feature].topology;
    let cap = reflected.elements.iter().find(|element| {
        matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal().into_iter().zip(cap_normal).all(|(a,b)| (a-b).abs()<1e-10))
    }).unwrap();
    let mut intent = GeometryIntent::capture(reflected, cap.key).unwrap();
    let anchor = cap.frames[0].origin.clone();
    intent.attachment = Some(Attachment {
        anchor,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let reference = intent.id;
    let datum = Entity::new(
        "Mirrored cap datum",
        EntityData::Plane {
            origin_mm: [0., 0., -10.],
            normal: [0., 0., -1.],
            anchor: Some(Link {
                target: request.feature,
                role: "face_attachment".into(),
                geometry: Some(intent),
            }),
        },
    );
    let owner = datum.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach datum to reflected cap".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();

    for (offset, height) in [(0., 10.), (2., 10.), (2., 15.)] {
        apply_parameter(&mut session, "mirror_z", &offset.to_string());
        apply_parameter(&mut session, "height", &height.to_string());
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
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
        let reopened = Session::new(opened.project.document).unwrap();
        let rust = build(&reopened, GeometryBackend::Rust);
        let solid = rust.solid(input.body).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        let bounds = solid.bounds().unwrap();
        let reflect = |p: [f64; 3]| {
            let p = vector(p);
            std::array::from_fn(|i| p[i] + 2. * offset * normal[2] * normal[i])
        };
        let corners: Vec<_> = [0., 30.]
            .into_iter()
            .flat_map(|x| {
                [0., 20.]
                    .into_iter()
                    .flat_map(move |y| [0., height].map(move |z| [x, y, z]))
            })
            .map(reflect)
            .collect();
        for i in 0..3 {
            near(
                bounds.min[i],
                corners.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min),
            );
            near(
                bounds.max[i],
                corners
                    .iter()
                    .map(|p| p[i])
                    .fold(f64::NEG_INFINITY, f64::max),
            );
        }
        near(solid.volume_mm3().unwrap(), 600. * height);
        let reflect = |[x, y, z]: [f64; 3]| reflect([x, y, if z > 0. { height } else { 0. }]);
        let topology = &rust.snapshot().shapes[&request.feature].topology;
        for (intent, geometry) in &captures {
            let Resolution::Resolved {
                geometry: Some(result),
                ..
            } = references::resolve(intent, topology, reopened.revision())
            else {
                panic!("Mirror lost authored {intent:?}");
            };
            match (*geometry, result) {
                (
                    AnalyticGeometry::Point { position_mm },
                    AnalyticGeometry::Point {
                        position_mm: actual,
                    },
                ) => {
                    for (a, b) in actual.into_iter().zip(reflect(position_mm)) {
                        near(a, b);
                    }
                }
                (
                    AnalyticGeometry::Line { a_mm, b_mm },
                    AnalyticGeometry::Line { a_mm: a, b_mm: b },
                ) => {
                    let expected = [reflect(a_mm), reflect(b_mm)];
                    let close = |a: [f64; 3], b: [f64; 3]| {
                        a.into_iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-8)
                    };
                    assert!(
                        (close(expected[0], a) && close(expected[1], b))
                            || (close(expected[0], b) && close(expected[1], a))
                    );
                }
                (AnalyticGeometry::Plane { frame }, AnalyticGeometry::Plane { frame: actual }) => {
                    let [x, y, z] = frame.normal();
                    let [a, b, c] = actual.normal();
                    for (a, b) in [a, b, c].into_iter().zip(vector([x, y, z])) {
                        near(a, b);
                    }
                    let expected = reflect(frame.origin_mm);
                    near(
                        expected[0] * a + expected[1] * b + expected[2] * c,
                        actual.origin_mm[0] * a + actual.origin_mm[1] * b + actual.origin_mm[2] * c,
                    );
                }
                _ => panic!("Unexpected mirrored geometry {result:?}"),
            }
        }
        for backend in [
            GeometryBackend::Rust,
            #[cfg(feature = "occt")]
            GeometryBackend::OpenCascade,
        ] {
            let rebuilt = build(&reopened, backend);
            let solid = rebuilt.solid(input.body).unwrap();
            near(solid.volume_mm3().unwrap(), 600. * height);
            assert_eq!(solid.solid_count().unwrap(), 1);
            let actual = solid.bounds().unwrap();
            for (a, b) in actual
                .min
                .into_iter()
                .chain(actual.max)
                .zip(bounds.min.into_iter().chain(bounds.max))
            {
                near(a, b);
            }
            let report = &rebuilt.snapshot().references;
            assert!(report.all_resolved());
            let resolution = &report
                .get(ReferenceAddress { owner, reference })
                .unwrap()
                .resolution;
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = resolution
            else {
                panic!("Mirrored downstream datum lost its attachment: {resolution:?}");
            };
            for (a, b) in frame.normal().into_iter().zip(cap_normal) {
                near(a, b);
            }
            for (a, b) in frame.origin_mm.into_iter().zip(reflect([0., 0., 10.])) {
                near(a, b);
            }
        }
    }
}

#[test]
fn cardinal_mirror_preserves_a_rectilinear_cavity_and_accepts_a_negative_normal() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        BooleanOperation, GeometryPolicy, PlaneFrame, Solid, rust_kernel::Rational,
    };

    let policy = GeometryPolicy::default();
    let stock = |size: [i64; 3], origin| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let a = stock([15, 10, 8], [0.; 3]);
    let b = stock([5, 4, 3], [2., 3., 1.]);
    let cut = a
        .boolean(&b, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let plane = PlaneFrame {
        origin: [10., 4., -5.],
        u: [0., 0.6, 0.8],
        v: [0., 0.8, -0.6],
    };
    let feature = EntityId::new();
    let mirrored = cut.mirrored_named(plane, feature, policy).unwrap();
    assert_eq!(mirrored.solid_count().unwrap(), 1);
    assert_eq!(mirrored.volume_mm3().unwrap(), 1140.);
    assert_eq!(mirrored.bounds().unwrap().min, [5., 0., 0.]);
    assert_eq!(mirrored.bounds().unwrap().max, [20., 10., 8.]);
    let cavity = stock([5, 4, 3], [13., 3., 1.]);
    assert!(
        mirrored
            .boolean(&cavity, BooleanOperation::Intersect, policy)
            .unwrap()
            .is_none()
    );
    let inside = stock([1, 1, 1], [6., 1., 1.]);
    let common = mirrored
        .boolean(&inside, BooleanOperation::Intersect, policy)
        .unwrap()
        .unwrap();
    assert_eq!(common.volume_mm3().unwrap(), 1.);
    let restored = mirrored
        .mirrored_named(plane, EntityId::new(), policy)
        .unwrap();
    assert_eq!(restored.bounds().unwrap(), cut.bounds().unwrap());
    assert_eq!(restored.volume_mm3().unwrap(), cut.volume_mm3().unwrap());
    let current = restored.topology_snapshot(feature, 1).unwrap();
    let source = cut.topology_snapshot(feature, 0).unwrap();
    assert_eq!(current.elements.len(), source.elements.len());
    for element in &source.elements {
        let intent = GeometryIntent::capture(&source, element.key).unwrap();
        assert!(matches!(
            references::resolve(&intent, &current, 1),
            Resolution::Resolved { .. }
        ));
    }
}

#[test]
fn rust_mirror_refuses_curved_geometry_without_moving_the_source() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
        rust_kernel::{Circle, Rational, cylinder::AxialCylinder, polygon::PolygonPrism},
    };

    let policy = GeometryPolicy::default();
    let integer = |n| Rational::new(n, 1).unwrap();
    let box_at = |frame| {
        Solid::rust_box(EntityId::new().uuid_bits(), [integer(4); 3], frame, policy).unwrap()
    };
    let flat = PlaneFrame::default();
    let stock = box_at(flat);
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([integer(0); 2], integer(1)).unwrap(),
        integer(0),
        integer(4),
    )
    .unwrap();
    let curved = Solid::from_axial_cylinder(cylinder, flat, policy).unwrap();
    let owner = EntityId::new();
    let triangle = Solid::from_polygon_prism(
        PolygonPrism::new(
            vec![[[0, 0], [4, 0], [0, 4]].map(|p| p.map(integer)).to_vec()],
            integer(4),
        )
        .unwrap(),
        flat,
        owner,
        &(0..3)
            .map(|i| multicad_core::references::Origin {
                source: owner,
                role: format!("triangle.edge.{i}"),
            })
            .collect::<Vec<_>>(),
        &[false; 3],
        policy,
    )
    .unwrap();
    for body in [&curved, &triangle] {
        let plane = flat;
        let message = "rectilinear stock";
        let before = body.bounds().unwrap();
        let error = body
            .mirrored_named(plane, EntityId::new(), policy)
            .err()
            .unwrap();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains(message));
        assert_eq!(body.bounds().unwrap(), before);
    }
    let outside = PlaneFrame {
        origin: [0., 0., policy.maximum_coordinate_mm],
        ..flat
    };
    for outside in [
        outside,
        PlaneFrame {
            u: [0.8, 0., -0.6],
            v: [0., 1., 0.],
            ..outside
        },
    ] {
        assert_eq!(
            stock
                .mirrored_named(outside, EntityId::new(), policy)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Input,
        );
    }
    assert_eq!(stock.bounds().unwrap().min, [0.; 3]);
}

#[test]
fn oblique_mirror_of_placed_stock_preserves_boundary_frames_mesh_and_step() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        BooleanOperation, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform, Solid,
        rust_kernel::Rational,
    };
    let policy = GeometryPolicy::default();
    let stock = |size: [i64; 3], origin| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let base = stock([15, 10, 8], [0.; 3]);
    let cut = base
        .boolean(
            &stock([5, 4, 4], [2., 3., -1.]),
            BooleanOperation::Subtract,
            policy,
        )
        .unwrap()
        .unwrap();
    for placement in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [11., -7., 23.],
            u: [0.8, 0., -0.6],
            v: [0.36, 0.8, 0.48],
        },
        PlaneFrame {
            origin: [11., -7., 23.],
            u: [0.8, 0., -0.6],
            v: [0., 1., 0.],
        },
    ] {
        let transform = RigidTransform {
            rotation: std::array::from_fn(|i| {
                [placement.u[i], placement.v[i], placement.normal()[i]]
            }),
            translation_mm: placement.origin,
        };
        for source in [&base, &cut] {
            let source = source.transformed(transform).unwrap();
            let feature = EntityId::new();
            let original = source.topology_snapshot(feature, 0).unwrap();
            let before = source.bounds().unwrap();
            for plane in [
                PlaneFrame::default(),
                PlaneFrame {
                    origin: [3., -2., 7.],
                    u: [0.8, 0., -0.6],
                    v: [0., 1., 0.],
                },
                PlaneFrame {
                    origin: [-2., 4., 8.],
                    u: [1., 0., 0.],
                    v: [0., -0.6, -0.8],
                },
            ] {
                let n = plane.normal();
                let vector = |p: [f64; 3]| -> [f64; 3] {
                    let d = 2. * p.into_iter().zip(n).map(|(a, b)| a * b).sum::<f64>();
                    std::array::from_fn(|i| p[i] - d * n[i])
                };
                let point = |p: [f64; 3]| {
                    let q = vector(p);
                    let d = 2.
                        * plane
                            .origin
                            .into_iter()
                            .zip(n)
                            .map(|(a, b)| a * b)
                            .sum::<f64>();
                    std::array::from_fn(|i| q[i] + d * n[i])
                };
                let close = |a: [f64; 3], b: [f64; 3]| {
                    for (a, b) in a.into_iter().zip(b) {
                        near(a, b);
                    }
                };
                let result = source
                    .mirrored_named(plane, EntityId::new(), policy)
                    .unwrap();
                near(result.volume_mm3().unwrap(), source.volume_mm3().unwrap());
                let current = result.topology_snapshot(feature, 1).unwrap();
                assert_eq!(original.elements.len(), current.elements.len());
                for element in &original.elements {
                    let intent = GeometryIntent::capture(&original, element.key).unwrap();
                    let Resolution::Resolved {
                        key,
                        geometry: Some(geometry),
                        ..
                    } = references::resolve(&intent, &current, 1)
                    else {
                        panic!("lost mirrored boundary {intent:?}")
                    };
                    match (element.geometry.unwrap(), geometry) {
                        (
                            AnalyticGeometry::Point { position_mm: a },
                            AnalyticGeometry::Point { position_mm: b },
                        ) => close(point(a), b),
                        (
                            AnalyticGeometry::Line { a_mm: a, b_mm: b },
                            AnalyticGeometry::Line { a_mm: c, b_mm: d },
                        ) => {
                            let a = point(a);
                            let b = point(b);
                            let distance = |a: [f64; 3], b: [f64; 3]| {
                                a.into_iter()
                                    .zip(b)
                                    .map(|(a, b)| (a - b).abs())
                                    .sum::<f64>()
                            };
                            assert!(
                                (distance(a, c) + distance(b, d))
                                    .min(distance(a, d) + distance(b, c))
                                    < 1e-7
                            );
                        }
                        (
                            AnalyticGeometry::Plane { frame: a },
                            AnalyticGeometry::Plane { frame: b },
                        ) => {
                            close(vector(a.normal()), b.normal());
                            let a = point(a.origin_mm);
                            near(
                                a.into_iter().zip(b.normal()).map(|(a, b)| a * b).sum(),
                                b.origin_mm
                                    .into_iter()
                                    .zip(b.normal())
                                    .map(|(a, b)| a * b)
                                    .sum(),
                            );
                            let mirrored = current.elements.iter().find(|e| e.key == key).unwrap();
                            for anchor in &element.frames {
                                let actual = mirrored
                                    .frames
                                    .iter()
                                    .find(|f| f.origin == anchor.origin)
                                    .unwrap();
                                close(point(anchor.frame.origin_mm), actual.frame.origin_mm);
                                close(vector(anchor.frame.u), actual.frame.u);
                                assert!(
                                    actual
                                        .frame
                                        .normal()
                                        .into_iter()
                                        .zip(b.normal())
                                        .map(|(a, b)| a * b)
                                        .sum::<f64>()
                                        > 1. - 1e-9
                                );
                            }
                        }
                        pair => panic!("unexpected mirror geometry {pair:?}"),
                    }
                }
                let receipt = result
                    .manufacturing_mesh_with_receipt(MeshOptions::default())
                    .unwrap();
                assert!(receipt.edge_pairs > 0 && receipt.vertex_links > 0);
                let folder = tempfile::tempdir().unwrap();
                let path = folder.path().join("oblique-mirror.step");
                result.write_step(&path).unwrap();
                let read = Solid::import_rust_step(
                    &std::fs::read_to_string(&path).unwrap(),
                    EntityId::new(),
                    policy,
                )
                .unwrap();
                near(read.volume_mm3().unwrap(), result.volume_mm3().unwrap());
                let bounds = result.bounds().unwrap();
                for read in [
                    Some(read),
                    if cfg!(feature = "occt") {
                        Some(Solid::read_step(&path, policy).unwrap())
                    } else {
                        None
                    },
                ]
                .into_iter()
                .flatten()
                {
                    near(read.volume_mm3().unwrap(), result.volume_mm3().unwrap());
                    close(read.bounds().unwrap().min, bounds.min);
                    close(read.bounds().unwrap().max, bounds.max);
                    near(
                        read.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                        result.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                    );
                }
                let restored = result
                    .mirrored_named(plane, EntityId::new(), policy)
                    .unwrap();
                close(restored.bounds().unwrap().min, before.min);
                close(restored.bounds().unwrap().max, before.max);
                assert_eq!(source.bounds().unwrap(), before);
                assert_eq!(source.topology_snapshot(feature, 0).unwrap(), original);
            }
        }
    }
}

#[test]
fn oblique_mirror_accepts_plane_axes_within_validation_tolerance() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{GeometryPolicy, PlaneFrame, Solid, rust_kernel::Rational};
    let policy = GeometryPolicy::default();
    let source = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [Rational::new(4, 1).unwrap(); 3],
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let exact = PlaneFrame {
        origin: [3., -2., 7.],
        u: [1., 0., 0.],
        v: [0., 0.6, 0.8],
    };
    let admitted = PlaneFrame {
        u: [(1_f64 + 8e-11).sqrt(), 0., 0.],
        ..exact
    };
    let expected = source
        .mirrored_named(exact, EntityId::new(), policy)
        .unwrap();
    let actual = source
        .mirrored_named(admitted, EntityId::new(), policy)
        .unwrap();
    for (a, b) in actual
        .bounds()
        .unwrap()
        .min
        .into_iter()
        .chain(actual.bounds().unwrap().max)
        .zip(
            expected
                .bounds()
                .unwrap()
                .min
                .into_iter()
                .chain(expected.bounds().unwrap().max),
        )
    {
        assert!((a - b).abs() < 1e-10);
    }
    near(actual.volume_mm3().unwrap(), 64.);
}
