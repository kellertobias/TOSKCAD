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

#[test]
fn imported_spline_compound_retains_exact_geometry_and_viewport_edge_identity() {
    use multicad_core::references::TopologyKind;
    use multicad_kernel::{
        MeshOptions,
        rust_kernel::{
            spur_gear::{Profile, Spec},
            step::Writer,
        },
    };
    let policy = GeometryPolicy::default();
    let pressure = 20_f64.to_radians();
    let gear = Profile::new(
        Spec {
            teeth: 20,
            pitch_radius_mm: 20.,
            base_radius_mm: 20. * pressure.cos(),
            tip_radius_mm: 22.,
            root_radius_mm: 17.5,
            root_fillet_radius_mm: 0.4,
            half_base_angle_rad: std::f64::consts::PI / 40. + pressure.tan()
                - pressure
                - 0.05 / 40.,
            face_width_mm: 8.,
            bore_radius_mm: 2.,
        },
        1e-5,
        1e7,
    )
    .unwrap();
    let frame = PlaneFrame::default();
    let mut writer = Writer::new();
    writer
        .append_rectangular_prism(
            &multicad_kernel::rust_kernel::prism::Prism::rectangular(1, [n(2), n(3), n(4)])
                .unwrap(),
            [-100., 0., 0.],
            frame.u,
            frame.v,
        )
        .unwrap();
    writer
        .append_spur_gear(
            &gear,
            frame.origin,
            frame.u,
            frame.v,
            policy.linear_tolerance_mm,
        )
        .unwrap();
    let source = writer.finish().unwrap();
    let owner = EntityId::new();
    let solid = Solid::import_rust_step(&source, owner, policy).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 2);
    let volume = solid.volume_mm3().unwrap();
    assert!((volume - gear.volume_mm3() - 24.).abs() < 1e-4);
    let before = solid.topology_with_display(owner, 1, Some(0.05)).unwrap();
    let cubics: Vec<_> = before
        .topology
        .elements
        .iter()
        .filter(|edge| edge.key.kind == TopologyKind::Edge && edge.geometry.is_none())
        .collect();
    assert_eq!(cubics.len(), 80);
    assert!(cubics.iter().all(|edge| edge.key.ordinal >= 12));
    for edge in cubics {
        let overlay = before
            .edges
            .iter()
            .find(|candidate| candidate.index == edge.key.ordinal)
            .unwrap();
        assert!(overlay.points_mm.len() > 2);
    }

    let moved = solid
        .transformed(RigidTransform {
            rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
            translation_mm: [2., 3., 5.],
        })
        .unwrap();
    let after = moved.topology_with_display(owner, 2, Some(0.05)).unwrap();
    for (a, b) in before.edges.iter().zip(&after.edges) {
        assert_eq!(a.index, b.index);
        assert_eq!(a.points_mm.len(), b.points_mm.len());
        // Circular arcs may choose a different but equivalent starting frame.
        if before.topology.elements.iter().any(|edge| {
            edge.key.kind == TopologyKind::Edge
                && edge.key.ordinal == a.index
                && edge.geometry.is_none()
        }) {
            for (&[x, y, z], &[moved_x, moved_y, moved_z]) in a.points_mm.iter().zip(&b.points_mm) {
                near(moved_x, 2. - y);
                near(moved_y, 3. + x);
                near(moved_z, 5. + z);
            }
        }
    }
    assert_eq!(
        before
            .topology
            .elements
            .iter()
            .map(|e| (&e.key, &e.origins))
            .collect::<Vec<_>>(),
        after
            .topology
            .elements
            .iter()
            .map(|e| (&e.key, &e.origins))
            .collect::<Vec<_>>(),
    );
    let mesh = moved
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert!(mesh.edge_pairs > 0 && mesh.vertex_links > 0);
    let mut written = multicad_kernel::rust_kernel::step_read::read_analytic_compound(
        &text(&moved),
        policy.linear_tolerance_mm,
    )
    .unwrap();
    assert!(moved.step_readback_matches(
        &after.topology,
        &written,
        2. * policy.linear_tolerance_mm
    ));
    let splines = &mut written[1].splines;
    splines[0] = splines[1].clone();
    assert!(!moved.step_readback_matches(
        &after.topology,
        &written,
        2. * policy.linear_tolerance_mm
    ));
}

#[test]
fn rust_step_import_reconstructs_the_exact_trapezoid_after_rational_area_overflow() {
    use multicad_core::references::Origin;
    use multicad_kernel::{
        TreatmentFace,
        rust_kernel::step_read::{ImportProfile, read_analytic_solid, read_import_solid},
    };

    let policy = GeometryPolicy::default();
    let stock_owner = EntityId::new();
    let draft_owner = EntityId::new();
    let solid = Solid::rust_box(
        stock_owner.uuid_bits(),
        [40, 20, 10].map(n),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let topology = solid.topology_snapshot(stock_owner, 0).unwrap();
    let side = topology.elements.iter().find(|element|
        matches!(element.geometry, Some(AnalyticGeometry::Plane {frame}) if frame.normal() == [1.,0.,0.])
    ).unwrap();
    let draft = solid
        .draft_faces_named(
            &[TreatmentFace {
                key: side.key,
                origin: Origin {
                    source: draft_owner,
                    role: "selected draft face".into(),
                },
            }],
            [0., 0., 1.],
            PlaneFrame {
                origin: [0., 0., 3.],
                ..Default::default()
            },
            5_f64.to_radians(),
            draft_owner,
            policy,
        )
        .unwrap();
    let encoded = text(&draft);
    let (_, decoded) = read_import_solid(&encoded, policy.linear_tolerance_mm).unwrap();
    assert!(
        matches!(decoded.profile, ImportProfile::Polygon {circular_holes,..} if circular_holes.is_empty())
    );
    let imported_owner = EntityId::new();
    let imported = Solid::import_rust_step(&encoded, imported_owner, policy).unwrap();
    same_shape(&imported, &draft);
    let topology = draft.topology_snapshot(draft_owner, 0).unwrap();
    let open_shell = encoded.replacen("CLOSED_SHELL(", "OPEN_SHELL(", 1);
    assert!(Solid::import_rust_step(&open_shell, EntityId::new(), policy).is_err());
    for source in [encoded, text(&imported)] {
        let decoded = read_analytic_solid(&source, policy.linear_tolerance_mm).unwrap();
        assert!(Solid::step_topology_matches(
            &topology,
            &decoded,
            policy.linear_tolerance_mm
        ));
    }
}
