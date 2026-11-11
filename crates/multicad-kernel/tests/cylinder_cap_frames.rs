use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    *,
};

fn near_point(actual: [f64; 3], expected: [f64; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-7, "{actual:?} != {expected:?}");
    }
}
fn source(profile: PlaneFrame, uv: [f64; 2], reverse: bool, owner: EntityId) -> Solid {
    let geometry = AxialCylinder::new(
        owner.uuid_bits(),
        Circle::new(
            [Rational::new(0, 1).unwrap(); 2],
            Rational::new(5, 1).unwrap(),
        )
        .unwrap(),
        Rational::new(0, 1).unwrap(),
        Rational::new(8, 1).unwrap(),
    )
    .unwrap();
    Solid::from_axial_cylinder_sweep(
        geometry,
        PlaneFrame {
            origin: profile.point(uv),
            u: profile.u,
            v: profile.v.map(|x| if reverse { -x } else { x }),
        },
        uv,
        reverse,
        Origin {
            source: owner,
            role: "circle".into(),
        },
        Default::default(),
    )
    .unwrap()
}
fn anchor(snapshot: &TopologySnapshot, owner: EntityId) -> ReferenceFrame {
    snapshot
        .elements
        .iter()
        .flat_map(|e| &e.frames)
        .find(|a| {
            a.origin
                == Origin {
                    source: owner,
                    role: "cap.end".into(),
                }
        })
        .unwrap()
        .frame
}

fn round_trip(operation: usize) {
    for (profile, uv, draft) in [
        (
            PlaneFrame {
                origin: [-1e6, 0., 0.],
                u: [(1_f64 + 8e-11).sqrt(), 0., 0.],
                v: [0., 1., 0.],
            },
            [1e6, 0.],
            false,
        ),
        (
            PlaneFrame {
                origin: [-1e6, -1e6, 0.],
                u: [1., 0., 0.],
                v: [8e-11, 1., 0.],
            },
            [1e6, 1e6],
            false,
        ),
        (
            PlaneFrame {
                origin: [-1e7, 0., 0.],
                u: [1. + 6e-15, 0., 0.],
                v: [0., 1., 0.],
            },
            [1e7, 0.],
            true,
        ),
    ] {
        if operation == 2 && !draft {
            continue;
        }
        for reverse in [false, true] {
            let owner = EntityId::new();
            let authored = source(profile, uv, reverse, owner);
            let original_snapshot = authored.topology_snapshot(owner, 0).unwrap();
            let original = anchor(&original_snapshot, owner);
            for mirror_twice in [false, true] {
                let stock = if mirror_twice {
                    authored
                        .mirrored_named(Default::default(), EntityId::new(), Default::default())
                        .unwrap()
                        .mirrored_named(Default::default(), EntityId::new(), Default::default())
                        .unwrap()
                } else {
                    source(profile, uv, reverse, owner)
                };
                let before = stock.topology_snapshot(owner, 0).unwrap();
                near_point(anchor(&before, owner).origin_mm, original.origin_mm);
                let feature = EntityId::new();
                let face = |ordinal| TreatmentFace {
                    key: TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal,
                    },
                    origin: Origin {
                        source: feature,
                        role: "selected".into(),
                    },
                };
                let placed = PlaneFrame {
                    origin: profile.point(uv),
                    u: profile.u,
                    v: profile.v.map(|x| if reverse { -x } else { x }),
                };
                let result = match operation {
                    0 => stock.treat_edges_named(
                        &[TreatmentEdge {
                            key: TopologyKey {
                                kind: TopologyKind::Edge,
                                ordinal: 0,
                            },
                            origin: Origin {
                                source: feature,
                                role: "selected".into(),
                            },
                        }],
                        EdgeTreatment::Chamfer { distance_mm: 0.5 },
                        feature,
                        Default::default(),
                    ),
                    1 => stock.shell_named(&[face(0)], 0.5, feature, Default::default()),
                    _ => stock.draft_faces_named(
                        &[face(2)],
                        placed.normal(),
                        placed,
                        5_f64.to_radians(),
                        feature,
                        Default::default(),
                    ),
                }
                .unwrap();
                let output = result.topology_snapshot(feature, 1).unwrap();
                let current = anchor(&output, owner);
                near_point(current.origin_mm, original.origin_mm);
                near_point(current.u, original.u);
                near_point(
                    current.v,
                    original
                        .v
                        .map(|x| if operation == 1 && reverse { -x } else { x }),
                );
                let rotate = |p: [f64; 3]| [-p[1], p[0], p[2]];
                let transformed = result
                    .transformed(RigidTransform {
                        rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
                        translation_mm: [3., -2., 7.],
                    })
                    .unwrap();
                let posed = transformed.topology_snapshot(feature, 2).unwrap();
                let transformed_anchor = anchor(&posed, owner);
                near_point(
                    transformed_anchor.origin_mm,
                    std::array::from_fn(|i| rotate(current.origin_mm)[i] + [3., -2., 7.][i]),
                );
                near_point(transformed_anchor.u, rotate(current.u));
                near_point(transformed_anchor.v, rotate(current.v));
                verify_geometry(&stock, &result, placed, operation);
                assert_eq!(
                    authored.topology_snapshot(owner, 0).unwrap(),
                    original_snapshot
                );
                assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
            }
        }
    }
}

#[test]
fn chamfer_retains_far_authored_cap_charts() {
    round_trip(0);
}
#[test]
fn shell_retains_far_authored_cap_charts() {
    round_trip(1);
}
#[test]
fn draft_retains_far_authored_cap_charts() {
    round_trip(2);
}

fn verify_geometry(stock: &Solid, result: &Solid, frame: PlaneFrame, operation: usize) {
    let policy = GeometryPolicy::default();
    let angle = 5_f64.to_radians();
    let expected = std::f64::consts::PI
        * match operation {
            0 => 25. * 8. - 0.5 * (5. * 0.5 - 0.25 / 3.),
            1 => 25. * 8. - 4.5_f64.powi(2) * 7.5,
            _ => {
                let r = 5. - 8. * angle.tan();
                8. / 3. * (25. + 5. * r + r * r)
            }
        };
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-5, "{a} != {b}");
    near(result.volume_mm3().unwrap(), expected);
    assert_eq!(result.solid_count().unwrap(), 1);
    result
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("corrected-cap.step");
    result.write_step(&path).unwrap();
    let imported = Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        EntityId::new(),
        policy,
    )
    .unwrap();
    near(imported.volume_mm3().unwrap(), expected);
    near_point(imported.bounds().unwrap().min, result.bounds().unwrap().min);
    near_point(imported.bounds().unwrap().max, result.bounds().unwrap().max);
    if OCCT_LINKED {
        let source_path = directory.path().join("source.step");
        stock.write_step(&source_path).unwrap();
        let native = Solid::read_step(&source_path, policy).unwrap();
        near_point(native.bounds().unwrap().min, stock.bounds().unwrap().min);
        near_point(native.bounds().unwrap().max, stock.bounds().unwrap().max);
        near(native.volume_mm3().unwrap(), stock.volume_mm3().unwrap());
        let feature = EntityId::new();
        let snapshot = native.topology_snapshot(feature, 0).unwrap();
        let key = snapshot.elements.iter().find(|e| matches!(e.geometry,
            Some(AnalyticGeometry::CircularArc { frame: circle, .. }) if circle.origin_mm.into_iter().zip(frame.origin).all(|(a,b)| (a-b).abs()<1e-6))).unwrap().key;
        let face = |index| TreatmentFace {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: index,
            },
            origin: Origin {
                source: feature,
                role: "native-control".into(),
            },
        };
        let native = match operation {
            0 => native.treat_edges_named(
                &[TreatmentEdge {
                    key,
                    origin: Origin {
                        source: feature,
                        role: "native-control".into(),
                    },
                }],
                EdgeTreatment::Chamfer { distance_mm: 0.5 },
                feature,
                policy,
            ),
            1 => {
                let cap = native
                    .faces()
                    .unwrap()
                    .into_iter()
                    .find(|f| {
                        f.plane.is_some_and(|p| {
                            p.0.into_iter()
                                .zip(frame.origin)
                                .all(|(a, b)| (a - b).abs() < 1e-6)
                        })
                    })
                    .unwrap()
                    .index;
                native.shell_named(&[face(cap)], 0.5, feature, policy)
            }
            _ => {
                let wall = native
                    .faces()
                    .unwrap()
                    .into_iter()
                    .find(|f| f.plane.is_none())
                    .unwrap()
                    .index;
                native.draft_faces_named(
                    &[face(wall)],
                    frame.normal(),
                    frame,
                    angle,
                    feature,
                    policy,
                )
            }
        }
        .unwrap();
        near(native.volume_mm3().unwrap(), expected);
        near_point(native.bounds().unwrap().min, result.bounds().unwrap().min);
        near_point(native.bounds().unwrap().max, result.bounds().unwrap().max);
        let read = Solid::read_step(&path, policy).unwrap();
        near(read.volume_mm3().unwrap(), expected);
        near_point(read.bounds().unwrap().min, result.bounds().unwrap().min);
        near_point(read.bounds().unwrap().max, result.bounds().unwrap().max);
    }
}
