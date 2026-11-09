use multicad_core::{
    document::EntityId,
    references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKey, TopologyKind},
};
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile, ProfileEdge, Solid,
    rust_kernel::{
        Circle, Rational, parse_length_mm,
        prism::{Location, Prism},
    },
};

fn integer(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn circle(x: i64, y: i64, radius: Rational) -> Circle {
    Circle::new([integer(x), integer(y)], radius).unwrap()
}

fn prism(owner: EntityId, height: Rational) -> Prism {
    Prism::extrude(owner.uuid_bits(), integer(40), integer(10), height).unwrap()
}

fn adopt(prism: Prism) -> Solid {
    Solid::from_rust_prism(prism, GeometryPolicy::default()).unwrap()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

#[test]
fn rust_rigid_rounded_prism_bounds_follow_the_arc_extrema() {
    let owner = EntityId::new();
    let original =
        adopt(Prism::extrude(owner.uuid_bits(), integer(4), integer(2), integer(3)).unwrap());
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let transform = multicad_kernel::RigidTransform {
        rotation: [[c, -c, 0.], [c, c, 0.], [0., 0., 1.]],
        translation_mm: [10., 20., 30.],
    };
    let world = original.transformed(transform).unwrap();
    let bounds = world.bounds().unwrap();
    for (actual, expected) in bounds.min.into_iter().chain(bounds.max).zip([
        10. - 4. * c,
        20.,
        30.,
        12. + 2. * c,
        22. + 6. * c,
        33.,
    ]) {
        near(actual, expected);
    }
    near(world.volume_mm3().unwrap(), original.volume_mm3().unwrap());
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
        .subtract_rust_prism_cylinder(
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
        rust.rust_prism().unwrap(),
        shared.rust_prism().unwrap()
    ));
    let probe = [integer(28), integer(10), integer(4)];
    assert_eq!(
        uncut.rust_prism().unwrap().classify(probe),
        Ok(Location::Interior)
    );
    assert_eq!(
        shared.rust_prism().unwrap().classify(probe),
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
    assert!(native.rust_prism().is_none());
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
            .subtract_rust_prism_cylinder(id, cutter, integer(-1), integer(9))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(
        body.subtract_rust_prism_cylinder(id, cutter, integer(0), integer(9))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        body.subtract_rust_prism_cylinder(id, cutter, integer(-1), integer(10_000_001))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(body.rust_prism().unwrap().inner_loops().len(), 0);
    assert!(Solid::from_rust_prism(prism(EntityId::nil(), integer(8)), policy).is_err());
    assert!(
        Solid::from_rust_prism(
            prism(owner, parse_length_mm("0.000001 mm").unwrap()),
            policy
        )
        .is_err()
    );
}

#[test]
fn placed_box_side_has_exact_contours_and_refuses_a_misaligned_bore() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let placement = PlaneFrame {
        origin: [7., -3., 2.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let stock = Solid::rust_box(
        owner.uuid_bits(),
        [integer(12), integer(5), integer(8)],
        placement,
        policy,
    )
    .unwrap();
    let right = PlaneFrame {
        origin: [7., 9., 2.],
        u: placement.v,
        v: placement.normal(),
    };
    let contour = stock.planar_face_contours(3, right).unwrap();
    near(contour.area_mm2(), 40.);
    assert_eq!(contour.edge_count(), 4);
    let error = stock
        .subtract_rust_prism_cylinder(
            EntityId::new().uuid_bits(),
            circle(10, 2, integer(1)),
            integer(-1),
            integer(9),
        )
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    near(stock.volume_mm3().unwrap(), 480.);
}

#[test]
fn rotated_box_bounds_and_mesh_follow_the_same_rigid_placement() {
    let (sin, cos) = 0.7_f64.sin_cos();
    let placement = PlaneFrame {
        origin: [-5., 3., -2.],
        u: [cos, sin, 0.],
        v: [-sin, cos, 0.],
    };
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [integer(12), integer(5), integer(8)],
        placement,
        GeometryPolicy::default(),
    )
    .unwrap();
    let bounds = stock.bounds().unwrap();
    for (actual, expected) in bounds.min.into_iter().zip([-5. - 5. * sin, 3., -2.]) {
        near(actual, expected);
    }
    for (actual, expected) in
        bounds
            .max
            .into_iter()
            .zip([-5. + 12. * cos, 3. + 12. * sin + 5. * cos, 6.])
    {
        near(actual, expected);
    }
    let mesh = stock.mesh(Default::default()).unwrap();
    let points = mesh.vertices.as_chunks::<3>().0;
    assert_eq!(points.len(), 8);
    for axis in 0..3 {
        let min = points.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
        let max = points
            .iter()
            .map(|p| p[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        near(min, bounds.min[axis]);
        near(max, bounds.max[axis]);
    }
}

#[test]
fn placed_polygon_publishes_hole_contours_and_original_reversed_edge_anchors() {
    use multicad_core::references::Origin;
    use multicad_kernel::rust_kernel::polygon::PolygonPrism;

    let profile = |corners: &[[i64; 2]]| corners.iter().map(|p| p.map(integer)).collect();
    let prism = PolygonPrism::new(
        vec![
            profile(&[[0, 0], [12, 0], [12, 5], [7, 5], [7, 9], [0, 9]]),
            profile(&[[2, 2], [2, 4], [4, 4], [4, 2]]),
        ],
        integer(3),
    )
    .unwrap();
    let owner = EntityId::new();
    let placement = PlaneFrame {
        origin: [5., 2., 7.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let origins: Vec<_> = (0..10)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "profile.edge".into(),
        })
        .collect();
    let policy = GeometryPolicy::default();
    assert!(
        Solid::from_polygon_prism(
            prism.clone(),
            placement,
            owner,
            &origins,
            &[false; 9],
            policy
        )
        .is_err()
    );
    let mut reversed = [false; 10];
    reversed[6..].fill(true);
    let solid = Solid::from_polygon_prism(prism, placement, owner, &origins, &reversed, policy)
        .unwrap()
        .share()
        .unwrap();
    near(solid.volume_mm3().unwrap(), 252.);
    assert_eq!(solid.bounds().unwrap().min, [5., 2., 7.]);
    assert_eq!(solid.bounds().unwrap().max, [8., 14., 16.]);
    let cap = solid
        .planar_face_contours_with_sources(
            1,
            PlaneFrame {
                origin: [8., 2., 7.],
                ..placement
            },
        )
        .unwrap();
    assert_eq!(cap.contours.loops.len(), 2);
    assert_eq!(cap.contours.edge_count(), 10);
    near(cap.contours.area_mm2(), 84.);
    let topology = solid.topology_snapshot(owner, 11).unwrap();
    for key in cap.edge_keys.iter().flatten() {
        assert!(topology.elements.iter().any(|element| element.key == *key));
    }
    let hole_side = topology
        .elements
        .iter()
        .find(|element| {
            element.key.kind == TopologyKind::Face && element.origins.contains(&origins[6])
        })
        .unwrap();
    assert_eq!(hole_side.key.ordinal, 8);
    let frame = hole_side.frames[0].frame;
    assert_eq!(frame.origin_mm, [5., 4., 11.]);
    assert_eq!(frame.u, [0., 0., -1.]);
    let side = solid
        .planar_face_contours(
            8,
            PlaneFrame {
                origin: frame.origin_mm,
                u: frame.u,
                v: frame.v,
            },
        )
        .unwrap();
    near(side.area_mm2(), 6.);
    assert_eq!(side.edge_count(), 4);
    let closing_side = solid
        .planar_face_contours(
            11,
            PlaneFrame {
                origin: [5., 4., 9.],
                u: [0., 1., 0.],
                v: [1., 0., 0.],
            },
        )
        .unwrap();
    near(closing_side.area_mm2(), 6.);
    assert_eq!(closing_side.edge_count(), 4);
    let mesh = solid.mesh(Default::default()).unwrap();
    assert_eq!(mesh.face_indices.iter().copied().max(), Some(11));
    assert_eq!(solid.solid_count().unwrap(), 1);
}

#[test]
fn polygon_cap_retains_its_exact_circle_and_its_cylindrical_wall() {
    use multicad_core::{manufacturing::PlanarEdge, references::Origin};
    use multicad_kernel::rust_kernel::polygon::PolygonPrism;
    use std::f64::consts::{PI, TAU};

    let profile = [[0, 0], [12, 0], [12, 5], [0, 5]]
        .map(|p| p.map(integer))
        .to_vec();
    let bore = circle(8, 2, integer(1));
    let prism = PolygonPrism::with_circular_holes(vec![profile], vec![bore], integer(4)).unwrap();
    let owner = EntityId::new();
    let origins: Vec<_> = (0..5)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "profile.edge".into(),
        })
        .collect();
    let policy = GeometryPolicy::default();
    assert!(
        Solid::from_polygon_prism(
            prism.clone(),
            PlaneFrame::default(),
            owner,
            &origins,
            &[false, false, false, false, true],
            policy,
        )
        .is_err()
    );
    let solid = Solid::from_polygon_prism(
        prism,
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 5],
        policy,
    )
    .unwrap();
    near(solid.volume_mm3().unwrap(), 240. - 4. * PI);
    let wall = solid.faces().unwrap().remove(6);
    assert_eq!(wall.plane, None);
    assert_eq!(wall.center_mm, [8., 2., 2.]);
    near(wall.area_mm2, 4. * TAU);
    let topology = solid.topology_snapshot(owner, 1).unwrap();
    let face = topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.key.ordinal == 6)
        .unwrap();
    assert!(face.origins.contains(&origins[4]));
    assert!(face.frames.is_empty());
    let contours = solid
        .planar_face_contours_with_sources(
            1,
            PlaneFrame {
                origin: [0., 0., 4.],
                ..PlaneFrame::default()
            },
        )
        .unwrap();
    near(contours.contours.area_mm2(), 60. - PI);
    let opening = &contours.contours.loops[1];
    assert!(!opening.outer);
    assert!(matches!(
        opening.edges.as_slice(),
        [PlanarEdge::Arc { center, sweep_rad, .. }] if *center == [8., 2.] && *sweep_rad == -TAU
    ));
    assert!(contours.edge_keys[1].iter().all(|key| {
        topology.elements.iter().any(|element| {
            element.key == *key
                && matches!(
                    element.geometry,
                    Some(AnalyticGeometry::CircularArc { radius_mm: 1., .. })
                )
        })
    }));
    assert!(
        solid
            .mesh(Default::default())
            .unwrap()
            .face_indices
            .contains(&6)
    );
}

#[test]
fn reversed_polygon_sweep_retains_authored_caps_and_outward_mesh_winding() {
    use multicad_core::references::Origin;
    use multicad_kernel::rust_kernel::polygon::PolygonPrism;

    let ring = [[0, 0], [6, 0], [6, 5], [0, 5]]
        .map(|p| p.map(integer))
        .to_vec();
    let prism =
        PolygonPrism::with_circular_holes(vec![ring], vec![circle(3, 2, integer(1))], integer(4))
            .unwrap();
    let owner = EntityId::new();
    let origins: Vec<_> = (0..5)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "profile.edge".into(),
        })
        .collect();
    let solid = Solid::from_polygon_prism_sweep(
        prism,
        PlaneFrame::default(),
        true,
        owner,
        &origins,
        &[false; 5],
        GeometryPolicy::default(),
    )
    .unwrap();
    assert_eq!(solid.bounds().unwrap().min, [0., 0., -4.]);
    assert_eq!(solid.bounds().unwrap().max, [6., 5., 0.]);
    let faces = solid.faces().unwrap();
    assert_eq!(faces[0].plane, Some(([0., 0., 0.], [0., 0., 1.])));
    assert_eq!(faces[1].plane, Some(([0., 0., -4.], [0., 0., -1.])));
    let topo = solid.topology_snapshot(owner, 2).unwrap();
    let face = |ordinal| {
        topo.elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face && element.key.ordinal == ordinal
            })
            .unwrap()
    };
    let end = &face(1).frames[0];
    assert_eq!(end.origin.role, "cap.end");
    assert_eq!(end.frame.origin_mm, [0., 0., -4.]);
    assert_eq!(end.frame.normal(), [0., 0., 1.]);
    assert_eq!(face(2).frames[0].frame.v, [0., 0., -1.]);
    let cap = solid
        .planar_face_contours(
            1,
            PlaneFrame {
                origin: end.frame.origin_mm,
                u: end.frame.u,
                v: end.frame.v,
            },
        )
        .unwrap();
    near(cap.area_mm2(), 30. - std::f64::consts::PI);
    assert_eq!(cap.loops.len(), 2);
    let mesh = solid.mesh(Default::default()).unwrap();
    let points = mesh.vertices.as_chunks::<3>().0;
    let volume: f64 = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| {
            let [[x, y, z], [a, b, c], [d, e, f]] = triangle.map(|i| points[i as usize]);
            (x * (b * f - c * e) + y * (c * d - a * f) + z * (a * e - b * d)) / 6.
        })
        .sum();
    assert!(
        (volume - solid.volume_mm3().unwrap()).abs() < 0.25,
        "{volume}"
    );
}

#[test]
fn exact_polygon_boolean_subtracts_only_strictly_through_cylinders() {
    use multicad_core::references::Origin;
    use multicad_kernel::rust_kernel::{cylinder::AxialCylinder, polygon::PolygonPrism};

    let profile = [[0, 0], [12, 0], [12, 5], [0, 5]]
        .map(|p| p.map(integer))
        .to_vec();
    let prism = PolygonPrism::new(vec![profile], integer(4)).unwrap();
    let owner = EntityId::new();
    let origins: Vec<_> = (0..4)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "profile.edge".into(),
        })
        .collect();
    let policy = GeometryPolicy::default();
    let stock = |frame| {
        Solid::from_polygon_prism(prism.clone(), frame, owner, &origins, &[false; 4], policy)
            .unwrap()
    };
    let target = stock(PlaneFrame::default());
    let negative = Solid::from_polygon_prism_sweep(
        prism.clone(),
        PlaneFrame::default(),
        true,
        owner,
        &origins,
        &[false; 4],
        policy,
    )
    .unwrap();
    let cutter = |id: EntityId, x, bottom, height| {
        let axial = AxialCylinder::new(
            id.uuid_bits(),
            circle(x, 2, integer(1)),
            integer(bottom),
            integer(height),
        )
        .unwrap();
        Solid::from_axial_cylinder(
            axial,
            PlaneFrame {
                origin: [x as f64, 2., bottom as f64],
                ..PlaneFrame::default()
            },
            policy,
        )
        .unwrap()
    };
    let subtract =
        |target: &Solid, tool: &Solid| target.boolean(tool, BooleanOperation::Subtract, policy);
    let tool_id = EntityId::new();
    let tool = cutter(tool_id, 8, -1, 6);
    let cut = subtract(&target, &tool).unwrap().unwrap();
    let native = Solid::box_solid([12., 5., 4.], policy).unwrap();
    let native_tool = Solid::cylinder(
        PlaneFrame {
            origin: [8., 2., -1.],
            ..PlaneFrame::default()
        },
        1.,
        6.,
        policy,
    )
    .unwrap();
    near(
        cut.volume_mm3().unwrap(),
        subtract(&native, &native_tool)
            .unwrap()
            .unwrap()
            .volume_mm3()
            .unwrap(),
    );
    let topology = cut.topology_snapshot(owner, 0).unwrap();
    assert!(
        topology
            .elements
            .iter()
            .any(|element| element.key.kind == TopologyKind::Face
                && element.origins.contains(&Origin {
                    source: tool_id,
                    role: "side.cylinder".into()
                }))
    );
    let second = subtract(&cut, &cutter(EntityId::new(), 3, -1, 6))
        .unwrap()
        .unwrap();
    near(
        second.volume_mm3().unwrap(),
        240. - 8. * std::f64::consts::PI,
    );
    let negative_cut = subtract(&negative, &cutter(EntityId::new(), 8, -5, 6))
        .unwrap()
        .unwrap();
    near(
        negative_cut.volume_mm3().unwrap(),
        cut.volume_mm3().unwrap(),
    );
    assert_eq!(negative_cut.bounds().unwrap().min, [0., 0., -4.]);
    assert_eq!(
        subtract(&negative, &tool).err().unwrap().kind,
        GeometryErrorKind::Input,
    );
    assert_eq!(
        subtract(&target, &cutter(EntityId::new(), 8, 0, 6))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input,
    );
    assert_eq!(
        subtract(&cut, &cutter(EntityId::new(), 10, -1, 6))
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input,
    );
    let placed = stock(PlaneFrame {
        origin: [0., 0., 2.],
        ..PlaneFrame::default()
    });
    assert_eq!(
        subtract(&placed, &tool).err().unwrap().kind,
        GeometryErrorKind::Unsupported,
    );
    near(target.volume_mm3().unwrap(), 240.);
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
