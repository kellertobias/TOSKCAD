use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{rust_kernel::Rational, *};

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
fn pose() -> PlaneFrame {
    PlaneFrame {
        origin: [11., -7., 23.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    }
}
fn direction(frame: PlaneFrame, axis: usize, sense: f64) -> [f64; 3] {
    [frame.u, frame.v, frame.normal()][axis].map(|v| sense * v)
}
fn point(frame: PlaneFrame, local: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        frame.origin[i]
            + frame.u[i] * local[0]
            + frame.v[i] * local[1]
            + frame.normal()[i] * local[2]
    })
}
fn selected(solid: &Solid, normal: [f64; 3], feature: EntityId, role: String) -> TreatmentFace {
    let snapshot = solid.topology_snapshot(feature, 0).unwrap();
    TreatmentFace {
        key: snapshot.elements.iter().find(|e| {
            matches!(e.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal().into_iter().zip(normal).all(|(a,b)| (a-b).abs()<1e-10))
        }).unwrap().key,
        origin: Origin { source: feature, role },
    }
}
fn compare(a: &Solid, b: &Solid) {
    near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
    let (a_bounds, b_bounds) = (a.bounds().unwrap(), b.bounds().unwrap());
    for (a, b) in a_bounds
        .min
        .into_iter()
        .chain(a_bounds.max)
        .zip(b_bounds.min.into_iter().chain(b_bounds.max))
    {
        near(a, b);
    }
    let faces = b.faces().unwrap();
    assert_eq!(a.faces().unwrap().len(), faces.len());
    for face in a.faces().unwrap() {
        let (_, normal) = face.plane.unwrap();
        let other = faces
            .iter()
            .find(|other| {
                other
                    .plane
                    .unwrap()
                    .1
                    .into_iter()
                    .zip(normal)
                    .all(|(a, b)| (a - b).abs() < 1e-8)
            })
            .expect("same oriented support");
        near(face.area_mm2, other.area_mm2);
        for (a, b) in face.center_mm.into_iter().zip(other.center_mm) {
            near(a, b);
        }
    }
}

#[test]
fn tilted_box_draft_matches_cardinal_geometry_and_native_for_signed_pulls_and_offsets() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let local = Solid::rust_box(
        owner.uuid_bits(),
        [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let frame = pose();
    let transform = RigidTransform {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
        translation_mm: frame.origin,
    };
    let stock = local.transformed(transform).unwrap();
    let before = stock.topology_snapshot(owner, 0).unwrap();
    let references: Vec<_> = before
        .elements
        .iter()
        .map(|e| GeometryIntent::capture(&before, e.key).unwrap())
        .collect();
    #[cfg(feature = "occt")]
    let native_stock = Solid::box_solid([30., 20., 10.], policy)
        .unwrap()
        .transformed(transform)
        .unwrap();
    for up in 0..3 {
        let (u, v) = ((up + 1) % 3, (up + 2) % 3);
        for sides in [
            vec![(u, true)],
            vec![(u, false), (u, true)],
            vec![(u, true), (v, true)],
        ] {
            for sense in [-1., 1.] {
                for degrees in [-5_f64, 5.] {
                    for neutral_level in [0., 3.] {
                        let feature = EntityId::new();
                        let selections: Vec<_> = sides
                            .iter()
                            .map(|&(axis, positive)| {
                                selected(
                                    &stock,
                                    direction(frame, axis, if positive { 1. } else { -1. }),
                                    feature,
                                    format!("side.{axis}.{positive}"),
                                )
                            })
                            .collect();
                        let local_selections: Vec<_> = sides
                            .iter()
                            .map(|&(axis, positive)| {
                                selected(
                                    &local,
                                    direction(
                                        PlaneFrame::default(),
                                        axis,
                                        if positive { 1. } else { -1. },
                                    ),
                                    feature,
                                    format!("side.{axis}.{positive}"),
                                )
                            })
                            .collect();
                        let mut origin = [0.; 3];
                        origin[up] = neutral_level;
                        let neutral = PlaneFrame {
                            origin: point(frame, origin),
                            u: direction(frame, u, 1.),
                            v: direction(frame, v, 1.),
                        };
                        let local_neutral = PlaneFrame {
                            origin,
                            u: direction(PlaneFrame::default(), u, 1.),
                            v: direction(PlaneFrame::default(), v, 1.),
                        };
                        let result = stock
                            .draft_faces_named(
                                &selections,
                                direction(frame, up, sense),
                                neutral,
                                degrees.to_radians(),
                                feature,
                                policy,
                            )
                            .unwrap();
                        let expected = local
                            .draft_faces_named(
                                &local_selections,
                                direction(PlaneFrame::default(), up, sense),
                                local_neutral,
                                degrees.to_radians(),
                                feature,
                                policy,
                            )
                            .unwrap()
                            .transformed(transform)
                            .unwrap();
                        compare(&result, &expected);
                        let output = result.topology_snapshot(feature, 1).unwrap();
                        output.validate().unwrap();
                        for reference in &references {
                            assert!(
                                matches!(
                                    resolve(reference, &output, 1),
                                    Resolution::Resolved { .. }
                                ),
                                "lost {reference:?}"
                            );
                        }
                        for selection in &selections {
                            assert_eq!(
                                output
                                    .elements
                                    .iter()
                                    .filter(|e| e.key.kind == TopologyKind::Face
                                        && e.origins.contains(&selection.origin))
                                    .count(),
                                1
                            );
                        }
                        result
                            .manufacturing_mesh_with_receipt(Default::default())
                            .unwrap();
                        let folder = tempfile::tempdir().unwrap();
                        let path = folder.path().join("tilted-draft.step");
                        result.write_step(&path).unwrap();
                        let imported = Solid::import_rust_step(
                            &std::fs::read_to_string(&path).unwrap(),
                            feature,
                            policy,
                        )
                        .unwrap();
                        compare(&result, &imported);
                        #[cfg(feature = "occt")]
                        {
                            let native_selections: Vec<_> = sides
                                .iter()
                                .map(|&(axis, positive)| {
                                    selected(
                                        &native_stock,
                                        direction(frame, axis, if positive { 1. } else { -1. }),
                                        feature,
                                        format!("side.{axis}.{positive}"),
                                    )
                                })
                                .collect();
                            let native = native_stock
                                .draft_faces_named(
                                    &native_selections,
                                    direction(frame, up, sense),
                                    neutral,
                                    degrees.to_radians(),
                                    feature,
                                    policy,
                                )
                                .unwrap();
                            compare(&result, &native);
                            compare(&result, &Solid::read_step(&path, policy).unwrap());
                        }
                        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
                    }
                }
            }
        }
    }
}

#[test]
fn tilted_draft_rejects_unaligned_inputs_and_collapsing_results_without_mutation() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let frame = pose();
    let stock = Solid::rust_box(
        owner.uuid_bits(),
        [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        frame,
        policy,
    )
    .unwrap();
    let before = stock.topology_snapshot(owner, 0).unwrap();
    let feature = EntityId::new();
    let selection = selected(&stock, frame.u, feature, "draft.side".into());
    let neutral = PlaneFrame {
        origin: frame.origin,
        u: frame.u,
        v: frame.v,
    };
    for (pull, neutral, angle, kind) in [
        ([0., 0., 1.], neutral, 0.1, GeometryErrorKind::Unsupported),
        (
            frame.normal(),
            PlaneFrame {
                u: frame.v,
                v: frame.normal(),
                ..neutral
            },
            0.1,
            GeometryErrorKind::Unsupported,
        ),
        (
            frame.normal(),
            PlaneFrame {
                u: [1., 0., 0.],
                v: [0., 1., 0.],
                ..neutral
            },
            0.1,
            GeometryErrorKind::Unsupported,
        ),
        (
            frame.normal(),
            neutral,
            80_f64.to_radians(),
            GeometryErrorKind::InvalidResult,
        ),
        (
            frame.normal(),
            PlaneFrame {
                origin: point(frame, [1e7, 0., 0.]),
                u: std::array::from_fn(|i| frame.u[i] + 1e-14 * frame.normal()[i]),
                v: frame.v,
            },
            45_f64.to_radians(),
            GeometryErrorKind::Unsupported,
        ),
        (
            frame.normal(),
            PlaneFrame {
                origin: point(frame, [1e6, 0., 0.]),
                u: std::array::from_fn(|i| frame.u[i] + 1e-14 * frame.normal()[i]),
                v: frame.v,
            },
            -85_f64.to_radians(),
            GeometryErrorKind::Unsupported,
        ),
    ] {
        assert_eq!(
            stock
                .draft_faces_named(
                    std::slice::from_ref(&selection),
                    pull,
                    neutral,
                    angle,
                    feature,
                    policy
                )
                .err()
                .unwrap()
                .kind,
            kind
        );
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    }
}
