use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder, rounded_cylinder::RoundedCylinder},
    *,
};
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn source(id: EntityId, frame: PlaneFrame, reverse: bool, authored: bool) -> Solid {
    let q = |n| Rational::new(n, 1).unwrap();
    let g = AxialCylinder::new(
        id.uuid_bits(),
        Circle::new([q(0); 2], q(5)).unwrap(),
        q(0),
        q(12),
    )
    .unwrap();
    if authored {
        Solid::from_axial_cylinder_sweep(
            g,
            frame,
            [3., -2.],
            reverse,
            Origin {
                source: id,
                role: "circle".into(),
            },
            Default::default(),
        )
        .unwrap()
    } else {
        Solid::from_axial_cylinder(g, PlaneFrame::default(), Default::default())
            .unwrap()
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
                translation_mm: frame.origin,
            })
            .unwrap()
    }
}
fn treatment(key: TopologyKey, feature: EntityId) -> TreatmentEdge {
    TreatmentEdge {
        key,
        origin: Origin {
            source: feature,
            role: format!("rim/{}", key.ordinal),
        },
    }
}
fn fillet(
    source: &Solid,
    owner: EntityId,
    ends: &[u32],
    radius: f64,
) -> (Solid, EntityId, Vec<TreatmentEdge>) {
    let id = EntityId::new();
    let snapshot = source.topology_snapshot(owner, 0).unwrap();
    let selected = ends
        .iter()
        .map(|&ordinal| {
            treatment(
                snapshot
                    .elements
                    .iter()
                    .find(|e| e.key.kind == TopologyKind::Edge && e.key.ordinal == ordinal)
                    .unwrap()
                    .key,
                id,
            )
        })
        .collect::<Vec<_>>();
    (
        source
            .treat_edges_named(
                &selected,
                EdgeTreatment::Fillet { radius_mm: radius },
                id,
                Default::default(),
            )
            .unwrap(),
        id,
        selected,
    )
}
#[test]
fn exact_cylinder_rim_fillet_retains_cap_frames_and_named_unselected_rims() {
    let policy = GeometryPolicy::default();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [17., -8., 13.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for (authored, reverse) in [(false, false), (true, false), (true, true)] {
            let owner = EntityId::new();
            let stock = source(owner, frame, reverse, authored)
                .with_member_origin(EntityId::new(), 7)
                .unwrap();
            let before = stock.topology_snapshot(owner, 0).unwrap();
            for (ends, radius) in [(vec![0], 1.), (vec![1], 3.), (vec![0, 1], 3.)] {
                let (rounded, id, edges) = fillet(&stock, owner, &ends, radius);
                let pure = RoundedCylinder::new(
                    5.,
                    12.,
                    [
                        ends.contains(&0).then_some(radius),
                        ends.contains(&1).then_some(radius),
                    ],
                    1e-6,
                    1e7,
                )
                .unwrap();
                near(rounded.volume_mm3().unwrap(), pure.volume_mm3());
                near(
                    rounded.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                    pure.surface_area_mm2(),
                );
                assert_eq!(rounded.solid_count().unwrap(), 1);
                assert!(rounded.uses_rust_geometry());
                let after = rounded.topology_snapshot(id, 1).unwrap();
                for cap in 0..2 {
                    let element = after
                        .elements
                        .iter()
                        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == cap)
                        .unwrap();
                    let Some(AnalyticGeometry::Plane { frame }) = element.geometry else {
                        panic!("planar cap")
                    };
                    let contour = rounded
                        .planar_face_contours_with_sources(
                            cap,
                            PlaneFrame {
                                origin: frame.origin_mm,
                                u: frame.u,
                                v: frame.v,
                            },
                        )
                        .unwrap();
                    let cap_radius = 5. - if ends.contains(&cap) { radius } else { 0. };
                    near(
                        contour.contours.area_mm2(),
                        std::f64::consts::PI * cap_radius * cap_radius,
                    );
                    assert_eq!(contour.edge_keys.len(), 1);
                }

                for old in &before.elements {
                    let mut intent = GeometryIntent::capture(&before, old.key).unwrap();
                    if old.key.kind == TopologyKind::Edge {
                        intent.boundary = Some(old.boundary.clone());
                    }
                    if old.key.kind == TopologyKind::Edge && ends.contains(&old.key.ordinal) {
                        assert!(!matches!(
                            resolve(&intent, &after, 1),
                            Resolution::Resolved { .. }
                        ));
                        continue;
                    }
                    let Resolution::Resolved { key, .. } = resolve(&intent, &after, 1) else {
                        panic!("lost {old:?}")
                    };
                    let new = after.elements.iter().find(|e| e.key == key).unwrap();
                    assert_eq!(old.frames.len(), new.frames.len());
                    for (a, b) in old.frames.iter().zip(&new.frames) {
                        assert_eq!(a.origin, b.origin);
                        for (a, b) in a
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
                            near(a, b);
                        }
                    }
                    assert_eq!(old.origins, new.origins);
                }
                assert_eq!(
                    after
                        .elements
                        .iter()
                        .filter(|e| e.key.kind == TopologyKind::Face
                            && edges.iter().any(|t| e.origins.contains(&t.origin)))
                        .count(),
                    ends.len()
                );
                let mesh = rounded
                    .manufacturing_mesh(MeshOptions {
                        linear_deflection_mm: 0.02,
                        ..Default::default()
                    })
                    .unwrap();
                assert!(!mesh.indices.is_empty());
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("rounded.step");
                rounded.write_step(&path).unwrap();
                let text = std::fs::read_to_string(&path).unwrap();
                let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
                near(imported.volume_mm3().unwrap(), pure.volume_mm3());
                let path2 = dir.path().join("again.step");
                imported.write_step(&path2).unwrap();
                for (a, b) in rounded
                    .bounds()
                    .unwrap()
                    .min
                    .into_iter()
                    .chain(rounded.bounds().unwrap().max)
                    .zip(
                        imported
                            .bounds()
                            .unwrap()
                            .min
                            .into_iter()
                            .chain(imported.bounds().unwrap().max),
                    )
                {
                    near(a, b);
                }
                if OCCT_LINKED {
                    let native = Solid::read_step(&path, policy).unwrap();
                    near(native.volume_mm3().unwrap(), pure.volume_mm3());
                    near(
                        native.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                        pure.surface_area_mm2(),
                    );
                }
            }
            assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
        }
    }
}
#[test]
fn sequential_asymmetric_fillet_preserves_existing_patch_and_refuses_collapsed_or_retreatment() {
    let owner = EntityId::new();
    let stock = source(owner, Default::default(), false, true);
    let before = stock.topology_snapshot(owner, 0).unwrap();
    let (first, id, _) = fillet(&stock, owner, &[0], 1.);
    let initial = first.topology_snapshot(id, 0).unwrap();
    // The remaining sharp end is edge2 after the first end's two trims.
    let (both, next, _) = fillet(&first, id, &[2], 3.);
    let pure = RoundedCylinder::new(5., 12., [Some(1.), Some(3.)], 1e-6, 1e7).unwrap();
    near(both.volume_mm3().unwrap(), pure.volume_mm3());
    let after = both.topology_snapshot(next, 1).unwrap();
    for old in initial
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face || e.key.ordinal < 2)
    {
        let mut intent = GeometryIntent::capture(&initial, old.key).unwrap();
        if old.key.kind == TopologyKind::Edge {
            intent.boundary = Some(old.boundary.clone());
        }
        assert!(matches!(
            resolve(&intent, &after, 1),
            Resolution::Resolved { .. }
        ));
    }
    for radius in [5., 12., f64::NAN] {
        let id = EntityId::new();
        let edge = treatment(
            before
                .elements
                .iter()
                .find(|e| e.key.kind == TopologyKind::Edge)
                .unwrap()
                .key,
            id,
        );
        assert!(
            stock
                .treat_edges_named(
                    &[edge],
                    EdgeTreatment::Fillet { radius_mm: radius },
                    id,
                    Default::default()
                )
                .is_err()
        );
    }
    let feature = EntityId::new();
    let trim = treatment(
        initial
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Edge && e.key.ordinal == 0)
            .unwrap()
            .key,
        feature,
    );
    assert_eq!(
        first
            .treat_edges_named(
                &[trim],
                EdgeTreatment::Fillet { radius_mm: 0.5 },
                feature,
                Default::default()
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
}
#[test]
fn rounded_projection_uses_quarter_patch_and_true_world_witnesses() {
    let id = EntityId::new();
    let (rounded, _, _) = fillet(
        &source(id, Default::default(), false, false),
        id,
        &[0, 1],
        1.,
    );
    assert_eq!(
        rounded
            .rust_revolution_contains([f64::NAN, 0., 0.])
            .unwrap_err()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        rounded
            .rust_revolution_contains([1e20, 0., 0.])
            .unwrap_err()
            .kind,
        GeometryErrorKind::Input
    );
    for (point, expected, inner) in [
        ([5., 0., 0.], 2_f64.sqrt() - 1., false),
        ([0., 0., -2.], 2., false),
        ([0., 0., 5.], 0., true),
        ([0., 0., 0.], 0., false),
        ([7., 0., 5.], 2., false),
    ] {
        let probe = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [Rational::new(1, 1).unwrap(); 3],
            PlaneFrame {
                origin: point,
                ..Default::default()
            },
            Default::default(),
        )
        .unwrap();
        let vertex = probe
            .display_vertices()
            .unwrap()
            .into_iter()
            .find(|v| v.position_mm == point)
            .unwrap()
            .index;
        let measure = rounded
            .minimum_distance(BRepSelection::Whole, &probe, BRepSelection::Vertex(vertex))
            .unwrap();
        near(measure.distance_mm, expected);
        assert_eq!(measure.inner_solution, inner);
        assert!(measure.witnesses_complete);
        let reverse = probe
            .minimum_distance(
                BRepSelection::Vertex(vertex),
                &rounded,
                BRepSelection::Whole,
            )
            .unwrap();
        near(reverse.distance_mm, expected);
        assert_eq!(measure.witnesses[0].a_mm, reverse.witnesses[0].b_mm);
        assert_eq!(
            rounded.rust_revolution_contains(point).unwrap(),
            expected == 0.
        );
    }
}
#[test]
fn independent_native_cylinder_rim_fillet_matches_exact_provider() {
    if !OCCT_LINKED {
        return;
    }
    for ends in [vec![0], vec![1], vec![0, 1]] {
        for radius in [1., 3.] {
            let owner = EntityId::new();
            let native =
                Solid::cylinder(PlaneFrame::default(), 5., 12., Default::default()).unwrap();
            let topology = native.topology_snapshot(owner, 0).unwrap();
            let mut rims: Vec<_> = topology
                .elements
                .iter()
                .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::CircularArc { .. })))
                .collect();
            rims.sort_by(|a, b| {
                let z = |e: &TopologyElement| match e.geometry.unwrap() {
                    AnalyticGeometry::CircularArc { frame, .. } => frame.origin_mm[2],
                    _ => unreachable!(),
                };
                z(a).total_cmp(&z(b))
            });
            assert_eq!(rims.len(), 2);
            let id = EntityId::new();
            let selected = ends
                .iter()
                .map(|&i| treatment(rims[i as usize].key, id))
                .collect::<Vec<_>>();
            let result = native
                .treat_edges_named(
                    &selected,
                    EdgeTreatment::Fillet { radius_mm: radius },
                    id,
                    Default::default(),
                )
                .unwrap();
            let pure = RoundedCylinder::new(
                5.,
                12.,
                [
                    ends.contains(&0).then_some(radius),
                    ends.contains(&1).then_some(radius),
                ],
                1e-6,
                1e7,
            )
            .unwrap();
            near(result.volume_mm3().unwrap(), pure.volume_mm3());
            near(
                result.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                pure.surface_area_mm2(),
            );
        }
    }
}
#[test]
fn rounded_members_keep_independent_exact_step_and_manifold_mesh() {
    let owner = EntityId::new();
    let (rounded, id, _) = fillet(
        &source(owner, Default::default(), false, true),
        owner,
        &[1],
        3.,
    );
    let receipt = rounded
        .manufacturing_mesh_with_receipt(MeshOptions {
            linear_deflection_mm: 0.01,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [Rational::new(2, 1).unwrap(); 3],
        PlaneFrame {
            origin: [20., 0., 0.],
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap();
    let compound =
        Solid::compound_named(&[&stock, &rounded], EntityId::new(), Default::default()).unwrap();
    assert_eq!(compound.solid_count().unwrap(), 2);
    assert!(compound.supports_step_export());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.step");
    compound.write_step(&path).unwrap();
    let imported = Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        EntityId::new(),
        Default::default(),
    )
    .unwrap();
    near(
        imported.volume_mm3().unwrap(),
        compound.volume_mm3().unwrap(),
    );
    assert_eq!(imported.solid_count().unwrap(), 2);
    let second = dir.path().join("mixed-again.step");
    imported.write_step(&second).unwrap();
    let before = rounded.topology_snapshot(id, 0).unwrap();
    let placed = rounded
        .transformed(RigidTransform {
            rotation: [[0., 0., 1.], [1., 0., 0.], [0., 1., 0.]],
            translation_mm: [11., -7., 19.],
        })
        .unwrap();
    let after = placed.topology_snapshot(id, 1).unwrap();
    for e in &before.elements {
        let intent = GeometryIntent::capture(&before, e.key).unwrap();
        assert!(matches!(
            resolve(&intent, &after, 1),
            Resolution::Resolved { .. }
        ));
    }
    let posed = placed
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    assert_eq!(posed.edge_pairs as usize * 2, posed.mesh.indices.len());
}
#[test]
fn inherited_distant_authored_cap_chart_is_reconstructed_within_policy() {
    for (u, v) in [
        ([(1_f64 + 8e-11).sqrt(), 0., 0.], [0., 1., 0.]),
        ([1., 0., 0.], [4e-11, 1., 0.]),
    ] {
        let owner = EntityId::new();
        let q = |n| Rational::new(n, 1).unwrap();
        let cylinder = AxialCylinder::new(
            owner.uuid_bits(),
            Circle::new([q(0); 2], q(5)).unwrap(),
            q(0),
            q(12),
        )
        .unwrap();
        let stock = Solid::from_axial_cylinder_sweep(
            cylinder,
            PlaneFrame {
                origin: [17., -8., 13.],
                u,
                v,
            },
            [1e6, -1e6],
            false,
            Origin {
                source: owner,
                role: "distant-circle".into(),
            },
            Default::default(),
        )
        .unwrap();
        let before = stock.topology_snapshot(owner, 0).unwrap();
        let (rounded, id, _) = fillet(&stock, owner, &[0, 1], 1.);
        let after = rounded.topology_snapshot(id, 1).unwrap();
        for old in before
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let intent = GeometryIntent::capture(&before, old.key).unwrap();
            let Resolution::Resolved { key, .. } = resolve(&intent, &after, 1) else {
                panic!()
            };
            let new = after.elements.iter().find(|e| e.key == key).unwrap();
            for (a, b) in old.frames.iter().zip(&new.frames) {
                assert_eq!(a.origin, b.origin);
                for (a, b) in a
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
                    near(a, b);
                }
            }
        }
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    }
}
#[test]
fn overlapping_end_fillets_fail_before_any_source_is_published() {
    let owner = EntityId::new();
    let q = |n| Rational::new(n, 1).unwrap();
    let cylinder = AxialCylinder::new(
        owner.uuid_bits(),
        Circle::new([q(0); 2], q(5)).unwrap(),
        q(0),
        q(3),
    )
    .unwrap();
    let stock =
        Solid::from_axial_cylinder(cylinder, Default::default(), Default::default()).unwrap();
    let before = stock.topology_snapshot(owner, 0).unwrap();
    let id = EntityId::new();
    let edges: Vec<_> = before
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .map(|e| treatment(e.key, id))
        .collect();
    assert_eq!(
        stock
            .treat_edges_named(
                &edges,
                EdgeTreatment::Fillet { radius_mm: 2. },
                id,
                Default::default()
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::InvalidResult
    );
    assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    near(stock.volume_mm3().unwrap(), 75. * std::f64::consts::PI);
}
#[test]
fn imported_rounded_regions_and_cap_datums_do_not_follow_arbitrary_step_axis_roles() {
    let feature = EntityId::new();
    let write = |radii, origin, v| {
        multicad_kernel::rust_kernel::step::write_rounded_cylinder(
            &RoundedCylinder::new(5., 12., radii, 1e-6, 1e7).unwrap(),
            origin,
            [1., 0., 0.],
            v,
            1e-7,
        )
        .unwrap()
    };
    let a = Solid::import_rust_step(
        &write([Some(1.), Some(2.)], [0.; 3], [0., 1., 0.]),
        feature,
        Default::default(),
    )
    .unwrap();
    let b = Solid::import_rust_step(
        &write([Some(2.), Some(1.)], [0., 0., 12.], [0., -1., 0.]),
        feature,
        Default::default(),
    )
    .unwrap();
    let c = Solid::import_rust_step(
        &write([Some(1.5), Some(2.)], [0.; 3], [0., 1., 0.]),
        feature,
        Default::default(),
    )
    .unwrap();
    let before = a.topology_snapshot(feature, 0).unwrap();
    let reversed = b.topology_snapshot(feature, 1).unwrap();
    let changed = c.topology_snapshot(feature, 2).unwrap();
    near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
    for old in &before.elements {
        let intent = GeometryIntent::capture(&before, old.key).unwrap();
        let Resolution::Resolved { key, .. } = resolve(&intent, &reversed, 1) else {
            panic!("physical region lost {old:?}")
        };
        let new = reversed.elements.iter().find(|e| e.key == key).unwrap();
        for anchor in &old.frames {
            let mut datum = intent.clone();
            datum.attachment = Some(Attachment {
                anchor: anchor.origin.clone(),
                offset_mm: [0.25, -0.5, 0.75],
                angle_rad: 0.3,
            });
            let Resolution::Resolved {
                frame: Some(expected),
                ..
            } = resolve(&datum, &before, 0)
            else {
                panic!()
            };
            let Resolution::Resolved {
                frame: Some(actual),
                ..
            } = resolve(&datum, &reversed, 1)
            else {
                panic!()
            };
            for (a, b) in actual
                .origin_mm
                .into_iter()
                .chain(actual.u)
                .chain(actual.v)
                .zip(
                    expected
                        .origin_mm
                        .into_iter()
                        .chain(expected.u)
                        .chain(expected.v),
                )
            {
                near(a, b);
            }
        }
        assert_eq!(old.origins, new.origins);
        if old.key.kind == TopologyKind::Face && old.key.ordinal == 0 {
            assert!(!matches!(
                resolve(&intent, &changed, 2),
                Resolution::Resolved { .. }
            ));
        }
    }
}
