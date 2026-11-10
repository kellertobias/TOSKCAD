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
fn saved_cardinal_mirror_rebuilds_bound_plane_and_retains_authored_references() {
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

    let mut plane = PrimitiveFrame::default();
    plane.origin[2] = bound(&session, "mirror_z", "mm");
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Mirror(MirrorParameters { plane }),
        vec![],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let mirrored = build(&session, GeometryBackend::Rust);
    let reflected = &mirrored.snapshot().shapes[&request.feature].topology;
    let cap = reflected.elements.iter().find(|element| {
        matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal() == [0.,0.,-1.])
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
        assert_eq!(bounds.min, [0., 0., 2. * offset - height]);
        assert_eq!(bounds.max, [30., 20., 2. * offset]);
        near(solid.volume_mm3().unwrap(), 600. * height);
        let reflect = |[x, y, z]: [f64; 3]| [x, y, 2. * offset - if z > 0. { height } else { 0. }];
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
                    assert_eq!(actual, reflect(position_mm));
                }
                (
                    AnalyticGeometry::Line { a_mm, b_mm },
                    AnalyticGeometry::Line { a_mm: a, b_mm: b },
                ) => {
                    let expected = [reflect(a_mm), reflect(b_mm)];
                    assert!(expected == [a, b] || expected == [b, a]);
                }
                (AnalyticGeometry::Plane { frame }, AnalyticGeometry::Plane { frame: actual }) => {
                    let [x, y, z] = frame.normal();
                    let [a, b, c] = actual.normal();
                    assert_eq!([a, b, c], [x, y, -z]);
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
            assert_eq!(frame.normal(), [0., 0., -1.]);
            near(frame.origin_mm[2], bounds.min[2]);
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
fn rust_mirror_refuses_angled_or_curved_geometry_without_moving_the_source() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
        rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    };

    let policy = GeometryPolicy::default();
    let integer = |n| Rational::new(n, 1).unwrap();
    let box_at = |frame| {
        Solid::rust_box(EntityId::new().uuid_bits(), [integer(4); 3], frame, policy).unwrap()
    };
    let flat = PlaneFrame::default();
    let tilted = PlaneFrame {
        v: [0., 0.6, 0.8],
        ..flat
    };
    let stock = box_at(flat);
    let angled = box_at(tilted);
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([integer(0); 2], integer(1)).unwrap(),
        integer(0),
        integer(4),
    )
    .unwrap();
    let curved = Solid::from_axial_cylinder(cylinder, flat, policy).unwrap();
    for (body, plane, message) in [
        (&stock, tilted, "world-axis plane normal"),
        (&angled, flat, "world-aligned rectilinear stock"),
        (&curved, flat, "world-aligned rectilinear stock"),
    ] {
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
    assert_eq!(
        stock
            .mirrored_named(outside, EntityId::new(), policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input,
    );
}
