use multicad_core::{
    document::EntityId,
    manufacturing::PlanarEdge,
    references::{AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::*;
fn p() -> GeometryPolicy {
    Default::default()
}
fn origin(id: EntityId, role: &str) -> Origin {
    Origin {
        source: id,
        role: role.into(),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn rect(frame: PlaneFrame, a: [f64; 2], b: [f64; 2]) -> CurvedProfile {
    let points = [a, [b[0], a[1]], b, [a[0], b[1]]];
    CurvedProfile {
        frame,
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    }
}
fn circle(frame: PlaneFrame, center: [f64; 2], r: f64) -> CurvedProfile {
    CurvedProfile {
        frame,
        outer: vec![CurvedEdge2d::Circle {
            center,
            radius_mm: r,
        }],
        holes: vec![],
    }
}
fn names(n: usize) -> Vec<Origin> {
    (0..n)
        .map(|_| origin(EntityId::new(), "source-curve"))
        .collect()
}
fn xy(z: f64) -> PlaneFrame {
    PlaneFrame {
        origin: [0., 0., z],
        ..Default::default()
    }
}
fn face_at(s: &Solid, axis: usize, position: f64) -> u32 {
    s.faces()
        .unwrap()
        .iter()
        .find(|f| {
            f.plane
                .is_some_and(|(o, n)| (o[axis] - position).abs() < 1e-5 && n[axis].abs() > 0.99999)
        })
        .unwrap()
        .index
}
fn rail(pitch: f64, base: EntityId, bore: EntityId, names: &[Origin]) -> Solid {
    let rail = Solid::box_named([120., 20., 2.], base, p()).unwrap();
    rail.cut_transfer_profile_named(&circle(xy(2.), [pitch, 10.], 2.), -3., bore, names, p())
        .unwrap()
}
#[test]
fn source_face_keys_follow_exact_circle_and_line_wires_through_winding_and_world_frames() {
    let base = EntityId::new();
    let bore = EntityId::new();
    let name = names(1);
    let source = rail(30., base, bore, &name);
    let before = source.topology_snapshot(base, 0).unwrap();
    for frame in [
        xy(2.),
        PlaneFrame {
            origin: [0., 0., 2.],
            u: [0., 1., 0.],
            v: [1., 0., 0.],
        },
    ] {
        let sourced = source
            .planar_face_contours_with_sources(face_at(&source, 2, 2.), frame)
            .unwrap();
        assert_eq!(
            sourced.contours,
            source
                .planar_face_contours(face_at(&source, 2, 2.), frame)
                .unwrap()
        );
        for (wire, keys) in sourced.contours.loops.iter().zip(&sourced.edge_keys) {
            assert_eq!(wire.edges.len(), keys.len());
            assert_eq!(wire.signed_area() > 0., wire.outer);
            for (edge, key) in wire.edges.iter().zip(keys) {
                assert_eq!(key.kind, TopologyKind::Edge);
                let element = before.elements.iter().find(|e| e.key == *key).unwrap();
                assert!(!element.origins.is_empty());
                let uv = |a: [f64; 3]| {
                    let d = std::array::from_fn::<_, 3, _>(|i| a[i] - frame.origin[i]);
                    [
                        d.iter().zip(frame.u).map(|(a, b)| a * b).sum::<f64>(),
                        d.iter().zip(frame.v).map(|(a, b)| a * b).sum::<f64>(),
                    ]
                };
                match (edge, element.geometry.as_ref().unwrap()) {
                    (PlanarEdge::Line { a, b }, AnalyticGeometry::Line { a_mm, b_mm }) => {
                        let c = uv(*a_mm);
                        let d = uv(*b_mm);
                        let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
                        assert!(
                            (distance(*a, c) < 1e-5 && distance(*b, d) < 1e-5)
                                || (distance(*a, d) < 1e-5 && distance(*b, c) < 1e-5)
                        );
                    }
                    (
                        PlanarEdge::Arc {
                            center, sweep_rad, ..
                        },
                        AnalyticGeometry::CircularArc {
                            frame: arc,
                            radius_mm,
                            ..
                        },
                    ) => {
                        let c = uv(arc.origin_mm);
                        near(center[0], c[0]);
                        near(center[1], c[1]);
                        near(*radius_mm, 2.);
                        near(sweep_rad.abs(), std::f64::consts::TAU);
                        assert!(element.origins.contains(&name[0]));
                    }
                    other => panic!("key/geometry mismatch: {other:?}"),
                }
            }
        }
    }
    let first = source
        .planar_face_contours_with_sources(face_at(&source, 2, 2.), xy(2.))
        .unwrap();
    let hole = first.contours.loops.iter().position(|w| !w.outer).unwrap();
    let intent = GeometryIntent::capture(&before, first.edge_keys[hole][0]).unwrap();
    let edited = rail(45., base, bore, &name);
    let next = edited.topology_snapshot(base, 1).unwrap();
    let Resolution::Resolved { key, .. } = resolve(&intent, &next, 1) else {
        panic!("hole did not follow authored source identity")
    };
    let Some(AnalyticGeometry::CircularArc { frame, .. }) = next
        .elements
        .iter()
        .find(|e| e.key == key)
        .unwrap()
        .geometry
    else {
        panic!("not a real current circle")
    };
    near(frame.origin_mm[0], 45.);
    let world = source
        .transformed(RigidTransform {
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
            translation_mm: [200., 40., 100.],
        })
        .unwrap();
    let frame = PlaneFrame {
        origin: [202., 40., 100.],
        u: [0., 0., -1.],
        v: [0., 1., 0.],
    };
    let current = world
        .planar_face_contours_with_sources(face_at(&world, 0, 202.), frame)
        .unwrap();
    let hole = current.contours.loops.iter().find(|w| !w.outer).unwrap();
    let PlanarEdge::Arc { center, .. } = hole.edges[0] else {
        panic!("circle expected")
    };
    near(center[0], 30.);
    near(center[1], 10.);
    assert_eq!(source.topology_snapshot(base, 0).unwrap(), before);
}
#[test]
fn blind_through_and_rotated_receiving_cuts_preserve_real_history_and_step_geometry() {
    let base = EntityId::new();
    let target = Solid::box_named([120., 80., 18.], base, p()).unwrap();
    let original = target.topology_snapshot(base, 0).unwrap();
    let feature = EntityId::new();
    let names = names(1);
    let profile = circle(xy(18.), [30., 20.], 3.);
    let blind = target
        .cut_transfer_profile_named(&profile, -6., feature, &names, p())
        .unwrap();
    near(
        blind.volume_mm3().unwrap(),
        120. * 80. * 18. - std::f64::consts::PI * 9. * 6.,
    );
    let snapshot = blind.topology_snapshot(feature, 0).unwrap();
    let floor = snapshot
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face && e.origins.contains(&origin(feature, "cap.end"))
        })
        .unwrap();
    let intent = GeometryIntent::capture(&snapshot, floor.key).unwrap();
    let deeper = target
        .cut_transfer_profile_named(&profile, -8., feature, &names, p())
        .unwrap();
    let next = deeper.topology_snapshot(feature, 1).unwrap();
    assert!(matches!(
        resolve(&intent, &next, 1),
        Resolution::Resolved { .. }
    ));
    let depth = target
        .transfer_through_depth(profile.frame, true, p())
        .unwrap();
    assert!(depth < -18. && depth > -18.001);
    let through = target
        .cut_transfer_profile_named(&profile, depth, feature, &names, p())
        .unwrap();
    near(
        through.volume_mm3().unwrap(),
        120. * 80. * 18. - std::f64::consts::PI * 9. * 18.,
    );
    assert!(
        through
            .topology_snapshot(feature, 0)
            .unwrap()
            .elements
            .iter()
            .any(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&names[0]))
    );
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("receiving-part.step");
    through.write_step(&path).unwrap();
    let reopened = Solid::read_step(&path, p()).unwrap();
    near(
        reopened.volume_mm3().unwrap(),
        through.volume_mm3().unwrap(),
    );
    assert_eq!(reopened.solid_count().unwrap(), 1);
    let pose = RigidTransform {
        rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
        translation_mm: [200., 40., 100.],
    };
    let placed = target.transformed(pose).unwrap();
    let frame = PlaneFrame {
        origin: [218., 40., 100.],
        u: [0., 0., -1.],
        v: [0., 1., 0.],
    };
    let depth = placed.transfer_through_depth(frame, true, p()).unwrap();
    near(depth, -18. - p().minimum_feature_mm);
    let cut = placed
        .cut_transfer_profile_named(&circle(frame, [30., 20.], 3.), depth, feature, &names, p())
        .unwrap();
    near(cut.volume_mm3().unwrap(), through.volume_mm3().unwrap());
    let contours = cut
        .planar_face_contours(face_at(&cut, 0, 218.), frame)
        .unwrap();
    let hole = contours.loops.iter().find(|w| !w.outer).unwrap();
    let PlanarEdge::Arc { center, .. } = hole.edges[0] else {
        panic!("circle expected")
    };
    near(center[0], 30.);
    near(center[1], 20.);
    assert_eq!(target.topology_snapshot(base, 0).unwrap(), original);
}
#[test]
fn exact_clearance_support_is_bounded_and_rejects_collapse_or_uninstalled_offsets() {
    let circle = circle(xy(0.), [5., 6.], 2.);
    let expanded = circle.offset_closed_boundary(0.5, p()).unwrap();
    near(
        expanded.area_mm2(p()).unwrap(),
        std::f64::consts::PI * 2.5_f64.powi(2),
    );
    assert!(circle.offset_closed_boundary(-2., p()).is_err());
    let polygon = rect(xy(0.), [0., 0.], [10., 6.]);
    let wider = polygon.offset_closed_boundary(1., p()).unwrap();
    near(wider.area_mm2(p()).unwrap(), 12. * 8.);
    assert!(polygon.offset_closed_boundary(-3., p()).is_err());
    let mixed = CurvedProfile {
        frame: xy(0.),
        outer: vec![
            CurvedEdge2d::Line {
                a: [-5., 0.],
                b: [5., 0.],
            },
            CurvedEdge2d::Arc {
                center: [0., 0.],
                a: [5., 0.],
                b: [-5., 0.],
                clockwise: false,
            },
        ],
        holes: vec![],
    };
    near(
        mixed
            .offset_closed_boundary(0., p())
            .unwrap()
            .area_mm2(p())
            .unwrap(),
        std::f64::consts::PI * 12.5,
    );
    assert!(
        mixed
            .offset_closed_boundary(1., p())
            .unwrap_err()
            .message
            .contains("unsupported")
    );
    let panel = Solid::box_solid([20., 20., 4.], p()).unwrap();
    let feature = EntityId::new();
    let cut = panel
        .cut_transfer_profile_named(
            &CurvedProfile {
                frame: PlaneFrame {
                    origin: [10., 5., 4.],
                    ..Default::default()
                },
                ..mixed
            },
            -5.,
            feature,
            &names(2),
            p(),
        )
        .unwrap();
    near(
        cut.volume_mm3().unwrap(),
        1600. - std::f64::consts::PI * 12.5 * 4.,
    );
    for clearance in [f64::NAN, f64::INFINITY, 1e8, 1e-8] {
        assert!(polygon.offset_closed_boundary(clearance, p()).is_err());
    }
}
#[test]
fn invalid_direction_no_material_complete_removal_split_and_bad_origins_fail_without_mutation() {
    let owner = EntityId::new();
    let target = Solid::box_named([20., 20., 4.], owner, p()).unwrap();
    let before = target.topology_snapshot(owner, 0).unwrap();
    let feature = EntityId::new();
    assert!(target.transfer_through_depth(xy(4.), false, p()).is_err());
    assert!(target.transfer_through_depth(xy(2.), true, p()).is_err());
    assert!(
        target
            .cut_transfer_profile_named(&circle(xy(4.), [5., 5.], 2.), 3., feature, &names(1), p())
            .is_err()
    );
    assert!(
        target
            .cut_transfer_profile_named(
                &circle(xy(4.), [50., 50.], 2.),
                -5.,
                feature,
                &names(1),
                p()
            )
            .is_err()
    );
    assert!(
        target
            .cut_transfer_profile_named(
                &rect(xy(4.), [-1., -1.], [21., 21.]),
                -5.,
                feature,
                &names(4),
                p()
            )
            .is_err()
    );
    assert!(
        target
            .cut_transfer_profile_named(
                &rect(xy(4.), [8., -1.], [12., 21.]),
                -5.,
                feature,
                &names(4),
                p()
            )
            .is_err()
    );
    assert!(
        target
            .cut_transfer_profile_named(&circle(xy(4.), [5., 5.], 2.), -3., feature, &[], p())
            .is_err()
    );
    for depth in [0., f64::NAN, f64::INFINITY, 1e8] {
        assert!(
            target
                .cut_transfer_profile_named(
                    &circle(xy(4.), [5., 5.], 2.),
                    depth,
                    feature,
                    &names(1),
                    p()
                )
                .is_err()
        );
    }
    assert_eq!(target.topology_snapshot(owner, 0).unwrap(), before);
}
