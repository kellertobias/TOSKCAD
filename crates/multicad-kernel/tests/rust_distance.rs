use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, TopologyKind},
};
use multicad_kernel::{
    BRepSelection, BooleanOperation, GeometryErrorKind, GeometryPolicy, MinimumDistance,
    PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};

const EPS: f64 = 1e-7;

fn stock(rust: bool, size: [i64; 3], origin: [f64; 3]) -> Solid {
    let policy = GeometryPolicy::default();
    if rust {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|v| Rational::new(v, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    } else {
        Solid::box_solid(size.map(|v| v as f64), policy)
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: origin,
                ..Default::default()
            })
            .unwrap()
    }
}

fn face(solid: &Solid, normal: [f64; 3]) -> BRepSelection {
    BRepSelection::Face(
        solid
            .faces()
            .unwrap()
            .into_iter()
            .find(|f| {
                f.plane.is_some_and(|(_, n)| {
                    n.into_iter().zip(normal).all(|(a, b)| (a - b).abs() < EPS)
                })
            })
            .unwrap()
            .index,
    )
}

fn vertex(solid: &Solid, point: [f64; 3]) -> BRepSelection {
    BRepSelection::Vertex(solid.topology_snapshot(EntityId::new(), 0).unwrap().elements
        .into_iter()
        .find(|element| {
            element.key.kind == TopologyKind::Vertex
                && matches!(element.geometry, Some(AnalyticGeometry::Point {position_mm}) if position_mm == point)
        })
        .unwrap().key.ordinal)
}

fn edge(solid: &Solid, start: [f64; 3], end: [f64; 3]) -> BRepSelection {
    BRepSelection::Edge(
        solid
            .topology_snapshot(EntityId::new(), 0)
            .unwrap()
            .elements
            .into_iter()
            .find(|element| {
                element.key.kind == TopologyKind::Edge
                    && matches!(element.geometry, Some(AnalyticGeometry::Line {a_mm,b_mm})
                    if [a_mm,b_mm] == [start,end] || [b_mm,a_mm] == [start,end])
            })
            .unwrap()
            .key
            .ordinal,
    )
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < EPS, "{actual} != {expected}");
}

fn check_witnesses(result: &MinimumDistance) {
    assert!(!result.witnesses.is_empty());
    assert_eq!(
        result.witnesses_complete,
        result.total_solutions.unwrap() as usize == result.witnesses.len()
    );
    for witness in &result.witnesses {
        let length = witness
            .a_mm
            .into_iter()
            .zip(witness.b_mm)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        near(length, result.distance_mm);
    }
}

#[test]
fn finite_rust_faces_and_skew_edges_match_native_scalar_and_witnesses() {
    let measure = |rust| {
        let a = stock(rust, [10, 8, 3], [0.; 3]);
        let b = stock(rust, [10, 8, 3], [30., 0., 0.]);
        let near_faces = a
            .minimum_distance(face(&a, [0., 0., 1.]), &b, face(&b, [0., 0., 1.]))
            .unwrap();
        let b = stock(rust, [10, 8, 3], [5., -3., 2.]);
        let left = edge(&a, [0.; 3], [10., 0., 0.]);
        let right = edge(&b, [5., -3., 2.], [5., 5., 2.]);
        let skew = a.minimum_distance(left, &b, right).unwrap();
        (near_faces, skew)
    };
    let (native_faces, native_edges) = measure(false);
    let (rust_faces, rust_edges) = measure(true);
    near(rust_faces.distance_mm, native_faces.distance_mm);
    near(rust_faces.distance_mm, 20.);
    near(rust_edges.distance_mm, native_edges.distance_mm);
    near(rust_edges.distance_mm, 2.);
    check_witnesses(&rust_faces);
    check_witnesses(&rust_edges);
    for witness in &rust_faces.witnesses {
        near(witness.a_mm[0], 10.);
        near(witness.b_mm[0], 30.);
        near(witness.a_mm[2], 3.);
        near(witness.b_mm[2], 3.);
    }
    assert_eq!(rust_edges.witnesses.len(), 1);
    assert_eq!(rust_edges.witnesses[0].a_mm, [5., 0., 0.]);
    assert_eq!(rust_edges.witnesses[0].b_mm, [5., 0., 2.]);
}

#[test]
fn rust_selected_face_respects_a_through_hole_and_actual_finite_boundaries() {
    let measure = |rust| {
        let outer = stock(rust, [10; 3], [0.; 3]);
        let punch = stock(rust, [6, 6, 12], [2., 2., -1.]);
        let plate = outer
            .boolean(
                &punch,
                BooleanOperation::Subtract,
                GeometryPolicy::default(),
            )
            .unwrap()
            .unwrap();
        let probe = stock(rust, [1; 3], [5., 5., 11.]);
        probe
            .minimum_distance(
                vertex(&probe, [5., 5., 11.]),
                &plate,
                face(&plate, [0., 0., 1.]),
            )
            .unwrap()
    };
    let native = measure(false);
    let rust = measure(true);
    near(rust.distance_mm, native.distance_mm);
    near(rust.distance_mm, 10_f64.sqrt());
    check_witnesses(&rust);
    assert!(rust.witnesses.iter().all(|w| {
        w.a_mm == [5., 5., 11.]
            && w.b_mm[2] == 10.
            && (w.b_mm[0] == 2. || w.b_mm[0] == 8. || w.b_mm[1] == 2. || w.b_mm[1] == 8.)
    }));
}

#[test]
fn nested_and_cavity_clearance_survives_rigid_placement() {
    let pose = RigidTransform {
        rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
        translation_mm: [80., -32., 44.],
    };
    let measure = |rust| {
        let outside = stock(rust, [10; 3], [0.; 3]);
        let inner = stock(rust, [2; 3], [3.; 3]).transformed(pose).unwrap();
        let cutter = stock(rust, [6; 3], [2.; 3]);
        let shell = outside
            .boolean(
                &cutter,
                BooleanOperation::Subtract,
                GeometryPolicy::default(),
            )
            .unwrap()
            .unwrap()
            .transformed(pose)
            .unwrap();
        let outside = outside.transformed(pose).unwrap();
        let contained = outside
            .minimum_distance(BRepSelection::Whole, &inner, BRepSelection::Whole)
            .unwrap();
        let free = shell
            .minimum_distance(BRepSelection::Whole, &inner, BRepSelection::Whole)
            .unwrap();
        (contained, free)
    };
    let (a, b) = measure(true);
    let (native_a, native_b) = measure(false);
    near(a.distance_mm, native_a.distance_mm);
    near(b.distance_mm, native_b.distance_mm);
    near(a.distance_mm, 0.);
    near(b.distance_mm, 1.);
    assert!(a.inner_solution);
    assert!(!b.inner_solution);
    check_witnesses(&a);
    check_witnesses(&b);
}

#[test]
fn whole_compound_containment_classifies_members_independently_and_refuses_curves() {
    let a = stock(true, [10; 3], [0.; 3]);
    let assembly =
        Solid::compound_named(&[&a, &a], EntityId::new(), GeometryPolicy::default()).unwrap();
    let inner = stock(true, [2; 3], [2.; 3]);
    let contained = assembly
        .minimum_distance(BRepSelection::Whole, &inner, BRepSelection::Whole)
        .unwrap();
    near(contained.distance_mm, 0.);
    assert!(contained.inner_solution);
    check_witnesses(&contained);

    assert_eq!(
        a.minimum_distance(BRepSelection::Face(u32::MAX), &inner, BRepSelection::Whole)
            .unwrap_err()
            .kind,
        GeometryErrorKind::Input,
    );
    let native = stock(false, [2; 3], [2.; 3]);
    assert_eq!(
        a.minimum_distance(BRepSelection::Whole, &native, BRepSelection::Whole)
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported,
    );
    let n = |v| Rational::new(v, 1).unwrap();
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([n(0), n(0)], n(2)).unwrap(),
        n(0),
        n(4),
    )
    .unwrap();
    let cylinder =
        Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), GeometryPolicy::default())
            .unwrap();
    let finite = a
        .minimum_distance(BRepSelection::Whole, &cylinder, BRepSelection::Whole)
        .unwrap();
    near(finite.distance_mm, 0.);
    assert!(finite.inner_solution);
    check_witnesses(&finite);
    assert_eq!(
        a.minimum_distance(BRepSelection::Whole, &cylinder, BRepSelection::Face(2))
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported,
    );
}
