use multicad_core::{document::EntityId, references::AnalyticGeometry};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    RevolutionAxis, RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder, polygon::PolygonPrism},
};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 2e-6, "{actual} != {expected}");
}

fn same_shape(actual: &Solid, expected: &Solid) {
    assert!(actual.uses_rust_geometry());
    assert_eq!(actual.solid_count().unwrap(), 1);
    assert_eq!(
        actual.faces().unwrap().len(),
        expected.faces().unwrap().len()
    );
    near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
    let a = actual.bounds().unwrap();
    let b = expected.bounds().unwrap();
    for (actual, expected) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(actual, expected);
    }
}

fn text(source: &Solid) -> String {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shape.step");
    source.write_step(&path).unwrap();
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn rust_import_reconstructs_an_independently_written_open_cascade_box() {
    let policy = GeometryPolicy::default();
    let source = Solid::box_solid([6.25, 10.5, 3.], policy).unwrap();
    let imported = Solid::import_rust_step(&text(&source), EntityId::new(), policy).unwrap();
    same_shape(&imported, &source);
}

#[test]
fn rust_import_reconstructs_a_tilted_concave_extrusion_with_analytic_holes() {
    let policy = GeometryPolicy::default();
    let source_owner = EntityId::new();
    let imported_owner = EntityId::new();
    let rings = [
        vec![[0, 0], [12, 0], [12, 6], [6, 6], [6, 10], [0, 10]],
        vec![[2, 2], [2, 4], [4, 4], [4, 2]],
    ];
    let prism = PolygonPrism::with_circular_holes(
        rings
            .into_iter()
            .map(|ring| ring.into_iter().map(|p| p.map(n)).collect())
            .collect(),
        vec![Circle::new([n(9), n(3)], Rational::new(5, 4).unwrap()).unwrap()],
        Rational::new(10, 3).unwrap(),
    )
    .unwrap();
    let origins: Vec<_> = (0..11)
        .map(|index| multicad_core::references::Origin {
            source: source_owner,
            role: format!("profile/{index}"),
        })
        .collect();
    let source = Solid::from_polygon_prism_sweep(
        prism,
        PlaneFrame {
            origin: [2., -7., 5.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        true,
        source_owner,
        &origins,
        &[false; 11],
        policy,
    )
    .unwrap();
    let source_text = text(&source);
    let imported = Solid::import_rust_step(&source_text, imported_owner, policy).unwrap();
    same_shape(&imported, &source);
    let topology = imported.topology_snapshot(imported_owner, 0).unwrap();
    let round_edges: Vec<_> = topology
        .elements
        .iter()
        .filter_map(|element| match element.geometry {
            Some(AnalyticGeometry::CircularArc {
                radius_mm,
                sweep_rad,
                ..
            }) => Some((radius_mm, sweep_rad)),
            _ => None,
        })
        .collect();
    assert_eq!(round_edges.len(), 2);
    for (radius, sweep) in round_edges {
        near(radius, 1.25);
        near(sweep.abs(), std::f64::consts::TAU);
    }
    let reopened = Solid::import_rust_step(&source_text, imported_owner, policy).unwrap();
    assert_eq!(
        topology,
        reopened.topology_snapshot(imported_owner, 0).unwrap()
    );
    let reexported = Solid::import_rust_step(&text(&reopened), imported_owner, policy).unwrap();
    same_shape(&reexported, &source);
}

#[test]
fn rust_import_reconstructs_fractional_cylinder_without_tessellating() {
    let policy = GeometryPolicy::default();
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([n(0); 2], Rational::new(7, 3).unwrap()).unwrap(),
        n(0),
        Rational::new(11, 3).unwrap(),
    )
    .unwrap();
    let source = Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), policy)
        .unwrap()
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 0.8, -0.6], [0., 0.6, 0.8]],
            translation_mm: [13., 14., 15.],
        })
        .unwrap();
    let imported = Solid::import_rust_step(&text(&source), EntityId::new(), policy).unwrap();
    same_shape(&imported, &source);
    near(
        imported.volume_mm3().unwrap(),
        539. * std::f64::consts::PI / 27.,
    );
}

#[test]
fn rust_import_reconstructs_a_rectilinear_blind_pocket() {
    let policy = GeometryPolicy::default();
    let box_at = |size: [i64; 3], origin| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(n),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let stock = box_at([10; 3], [0.; 3]);
    let tool = box_at([2, 2, 6], [3., 3., 5.]);
    let pocket = stock
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let imported = Solid::import_rust_step(&text(&pocket), EntityId::new(), policy).unwrap();
    same_shape(&imported, &pocket);
    near(imported.volume_mm3().unwrap(), 980.);
}

#[test]
fn rust_import_refuses_a_valid_conical_step_surface() {
    let policy = GeometryPolicy::default();
    let points = [[0., 0.], [3., 0.], [1., 7.], [0., 7.]];
    let profile = CurvedProfile {
        frame: PlaneFrame {
            v: [0., 0., 1.],
            ..Default::default()
        },
        outer: points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    let cone = Solid::revolve_curved_profile(
        &profile,
        RevolutionAxis {
            origin_mm: [0.; 3],
            direction: [0., 0., 1.],
        },
        std::f64::consts::TAU,
        policy,
    )
    .unwrap();
    near(cone.volume_mm3().unwrap(), 91. * std::f64::consts::PI / 3.);
    let err = Solid::import_rust_step(&text(&cone), EntityId::new(), policy)
        .err()
        .unwrap();
    assert_eq!(err.kind, GeometryErrorKind::Unsupported);
}
