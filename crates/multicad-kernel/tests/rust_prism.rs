use multicad_core::{
    document::EntityId,
    references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKey, TopologyKind},
};
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile, ProfileEdge, Solid,
    rust_kernel::{
        Circle, Rational, parse_length_mm,
        prism::{Location, RoundedPrism},
    },
};

fn integer(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn circle(x: i64, y: i64, radius: Rational) -> Circle {
    Circle::new([integer(x), integer(y)], radius).unwrap()
}

fn prism(owner: EntityId, height: Rational) -> RoundedPrism {
    RoundedPrism::extrude(owner.uuid_bits(), integer(40), integer(10), height).unwrap()
}

fn adopt(prism: RoundedPrism) -> Solid {
    Solid::from_rounded_prism(prism, GeometryPolicy::default()).unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

#[test]
fn rust_bore_matches_the_independent_native_volume_and_all_face_centroids() {
    let panel = EntityId::new();
    let authored = EntityId::new();
    let cutter = EntityId::new();
    let uncut = adopt(
        prism(panel, integer(8))
            .with_inner_loop(authored.uuid_bits(), circle(12, 10, integer(3)))
            .unwrap(),
    );
    let rust = uncut
        .subtract_rounded_prism_cylinder(
            cutter.uuid_bits(),
            circle(28, 10, integer(4)),
            integer(-1),
            integer(9),
        )
        .unwrap();

    let profile = Profile {
        frame: PlaneFrame::default(),
        outer: vec![
            ProfileEdge::Line {
                a: [0., 0.],
                b: [40., 0.],
            },
            ProfileEdge::Arc {
                center: [40., 10.],
                a: [40., 0.],
                b: [40., 20.],
                clockwise: false,
            },
            ProfileEdge::Line {
                a: [40., 20.],
                b: [0., 20.],
            },
            ProfileEdge::Line {
                a: [0., 20.],
                b: [0., 0.],
            },
        ],
        holes: vec![vec![ProfileEdge::Circle {
            center: [12., 10.],
            radius_mm: 3.,
        }]],
    };
    let policy = GeometryPolicy::default();
    let native_uncut = Solid::extrude_profile(&profile, 8., policy).unwrap();
    let native_cutter = Solid::cylinder(
        PlaneFrame {
            origin: [28., 10., -1.],
            ..PlaneFrame::default()
        },
        4.,
        10.,
        policy,
    )
    .unwrap();
    let native = native_uncut
        .boolean(&native_cutter, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    near(
        uncut.volume_mm3().unwrap(),
        native_uncut.volume_mm3().unwrap(),
    );
    near(rust.volume_mm3().unwrap(), native.volume_mm3().unwrap());
    assert_eq!(rust.solid_count().unwrap(), 1);
    assert_eq!(rust.bounds().unwrap().min, [0.; 3]);
    assert_eq!(rust.bounds().unwrap().max, [50., 20., 8.]);
    let native_faces = native.faces().unwrap();
    let rust_faces = rust.faces().unwrap();
    assert_eq!(rust_faces.len(), native_faces.len());
    for face in rust_faces {
        let matching: Vec<_> = native_faces
            .iter()
            .filter(|candidate| {
                face.center_mm
                    .iter()
                    .zip(candidate.center_mm)
                    .all(|(a, b)| (a - b).abs() < 1e-8)
            })
            .collect();
        assert_eq!(matching.len(), 1, "{face:?}");
        near(face.area_mm2, matching[0].area_mm2);
        assert_eq!(face.plane.is_some(), matching[0].plane.is_some());
        if let (Some((_, rust)), Some((_, native))) = (face.plane, matching[0].plane) {
            for (a, b) in rust.into_iter().zip(native) {
                near(a, b);
            }
        }
    }
    let shared = rust.share().unwrap();
    assert!(std::ptr::eq(
        rust.rounded_prism().unwrap(),
        shared.rounded_prism().unwrap()
    ));
    let probe = [integer(28), integer(10), integer(4)];
    assert_eq!(
        uncut.rounded_prism().unwrap().classify(probe),
        Ok(Location::Interior)
    );
    assert_eq!(
        shared.rounded_prism().unwrap().classify(probe),
        Ok(Location::Exterior)
    );
}

#[test]
fn canonical_face_resolution_follows_source_id_and_updated_plane() {
    let owner = EntityId::new();
    let first = EntityId::new();
    let second = EntityId::new();
    let make = |height, loops: [(EntityId, Circle); 2]| {
        adopt(
            loops
                .into_iter()
                .fold(prism(owner, height), |body, (id, shape)| {
                    body.with_inner_loop(id.uuid_bits(), shape).unwrap()
                }),
        )
    };
    let a = circle(12, 10, integer(3));
    let b = circle(28, 10, integer(4));
    let revised_b = circle(28, 10, parse_length_mm("4.25 mm").unwrap());
    let original = make(integer(8), [(first, a), (second, b)]);
    let edited = make(
        parse_length_mm("10/3 mm").unwrap(),
        [(second, revised_b), (first, a)],
    );
    let before = original.topology_snapshot(owner, 1).unwrap();
    let after = edited.topology_snapshot(owner, 2).unwrap();
    let kind = TopologyKind::Face;
    let top = GeometryIntent::capture(&before, TopologyKey { kind, ordinal: 1 }).unwrap();
    let bore = GeometryIntent::capture(&before, TopologyKey { kind, ordinal: 7 }).unwrap();
    let Resolution::Resolved {
        geometry: Some(AnalyticGeometry::Plane { frame }),
        ..
    } = references::resolve(&top, &after, 2)
    else {
        panic!("top face failed to resolve")
    };
    near(frame.origin_mm[2], 10. / 3.);
    let Resolution::Resolved { key, .. } = references::resolve(&bore, &after, 2) else {
        panic!("bore failed to resolve")
    };
    assert_eq!(key.ordinal, 6);
    for (kind, count) in [
        (TopologyKind::Face, 8),
        (TopologyKind::Edge, 16),
        (TopologyKind::Vertex, 8),
    ] {
        assert_eq!(
            after.elements.iter().filter(|e| e.key.kind == kind).count(),
            count
        );
    }
    assert!(edited.topology_snapshot(EntityId::nil(), 2).is_err());
}

#[test]
fn unsupported_and_invalid_operations_leave_the_rust_body_intact() {
    let owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let body = adopt(prism(owner, integer(8)));
    let native = Solid::box_solid([1.; 3], policy).unwrap();
    assert!(native.rounded_prism().is_none());
    for (a, b) in [(&native, &body), (&body, &native)] {
        let error = a
            .boolean(b, BooleanOperation::Intersect, policy)
            .err()
            .unwrap();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    }
    let cutter = circle(28, 10, integer(4));
    let id = EntityId::new().uuid_bits();
    assert_eq!(
        native
            .subtract_rounded_prism_cylinder(id, cutter, integer(-1), integer(9))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(
        body.subtract_rounded_prism_cylinder(id, cutter, integer(0), integer(9))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        body.subtract_rounded_prism_cylinder(id, cutter, integer(-1), integer(10_000_001))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(body.rounded_prism().unwrap().inner_loops().len(), 0);
    assert!(Solid::from_rounded_prism(prism(EntityId::nil(), integer(8)), policy).is_err());
    assert!(
        Solid::from_rounded_prism(
            prism(owner, parse_length_mm("0.000001 mm").unwrap()),
            policy
        )
        .is_err()
    );
}

#[test]
fn exact_contours_preserve_both_bores_and_match_picking_edge_keys() {
    use multicad_core::manufacturing::PlanarEdge;
    use std::f64::consts::{PI, TAU};

    let owner = EntityId::new();
    let stock = adopt(
        prism(owner, integer(8))
            .with_inner_loop(EntityId::new().uuid_bits(), circle(12, 10, integer(3)))
            .unwrap()
            .with_inner_loop(EntityId::new().uuid_bits(), circle(28, 10, integer(4)))
            .unwrap(),
    );
    let topology = stock.topology_snapshot(owner, 0).unwrap();
    let frame = PlaneFrame {
        origin: [50., 0., 8.],
        u: [-1., 0., 0.],
        v: [0., 1., 0.],
    };
    let cap = stock.planar_face_contours_with_sources(1, frame).unwrap();
    near(cap.contours.area_mm2(), 800. + PI * 25.);
    assert_eq!(cap.contours.loops.len(), 3);
    assert!(cap.contours.loops[0].outer);
    assert!(
        cap.contours.loops[0]
            .edges
            .iter()
            .any(|edge| { matches!(edge, PlanarEdge::Arc { sweep_rad, .. } if *sweep_rad == PI) })
    );
    for wire in &cap.contours.loops[1..] {
        assert!(!wire.outer);
        assert!(
            matches!(wire.edges.as_slice(), [PlanarEdge::Arc { sweep_rad, .. }] if *sweep_rad == -TAU)
        );
    }
    for key in cap.edge_keys.into_iter().flatten() {
        assert!(topology.elements.iter().any(|e| e.key == key));
    }
    let side = stock
        .planar_face_contours(
            5,
            PlaneFrame {
                u: [0., 1., 0.],
                v: [0., 0., 1.],
                ..PlaneFrame::default()
            },
        )
        .unwrap();
    near(side.area_mm2(), 160.);
    assert_eq!(side.edge_count(), 4);
    assert!(stock.planar_face_contours(3, frame).is_err());
    assert!(
        stock
            .planar_face_contours(1, PlaneFrame::default())
            .is_err()
    );
    let (sin, cos) = 1e-6_f64.sin_cos();
    let tilted = PlaneFrame {
        origin: [0., 0., 8.],
        u: [cos, 0., sin],
        ..PlaneFrame::default()
    };
    assert!(
        stock
            .planar_face_contours(1, tilted)
            .unwrap_err()
            .message
            .contains("outside the contour plane")
    );
}

#[test]
fn cylinder_contours_are_analytic_at_both_caps_regardless_of_display_density() {
    use multicad_core::manufacturing::PlanarEdge;
    use multicad_kernel::rust_kernel::cylinder::AxialCylinder;
    use std::f64::consts::{PI, TAU};
    let id = EntityId::new();
    let profile = circle(5, 6, integer(2));
    let cylinder = Solid::from_axial_cylinder(
        AxialCylinder::new(id.uuid_bits(), profile, integer(-1), integer(4)).unwrap(),
        PlaneFrame {
            origin: [5., 6., -1.],
            u: [0., 1., 0.],
            v: [-1., 0., 0.],
        },
        GeometryPolicy::default(),
    )
    .unwrap();
    for (face, z) in [(0, -1.), (1, 3.)] {
        let contours = cylinder
            .planar_face_contours_with_sources(
                face,
                PlaneFrame {
                    origin: [5., 6., z],
                    ..PlaneFrame::default()
                },
            )
            .unwrap();
        near(contours.contours.area_mm2(), 4. * PI);
        assert_eq!(
            contours.edge_keys,
            [vec![TopologyKey {
                kind: TopologyKind::Edge,
                ordinal: face
            }]]
        );
        assert!(matches!(contours.contours.loops[0].edges.as_slice(),
            [PlanarEdge::Arc { sweep_rad, .. }] if *sweep_rad == TAU));
    }
    assert!(
        cylinder
            .mesh(multicad_kernel::MeshOptions {
                maximum_triangles: 1,
                ..Default::default()
            })
            .is_err()
    );
    let (sin, cos) = 1e-6_f64.sin_cos();
    let tilted = PlaneFrame {
        origin: [5., 6., 3.],
        u: [cos, 0., sin],
        ..PlaneFrame::default()
    };
    assert!(
        cylinder
            .planar_face_contours(1, tilted)
            .unwrap_err()
            .message
            .contains("outside the contour plane")
    );
}
