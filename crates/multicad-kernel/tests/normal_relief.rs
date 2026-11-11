use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, GeometryPolicy, NormalReliefMotif, OCCT_LINKED, PlaneFrame,
    RigidTransform, Solid,
    rust_kernel::{Rational, normal_relief::Kind},
};

fn point(frame: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
    })
}

fn fixture(kind: Kind, frame: PlaneFrame, feature: EntityId) -> (Solid, Solid, EntityId) {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let blank = Solid::rust_box(
        owner.uuid_bits(),
        [12, 10, 3].map(|v| Rational::new(v, 1).unwrap()),
        frame,
        policy,
    )
    .unwrap();
    let cap = PlaneFrame {
        origin: point(frame, [0., 0., 3.]),
        ..frame
    };
    let face = blank
        .faces()
        .unwrap()
        .into_iter()
        .find(|f| {
            f.plane.is_some_and(|(_, n)| {
                n.into_iter()
                    .zip(frame.normal())
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    > 0.999
            })
        })
        .unwrap();
    let motifs = [([3., 3.], 1., 0.5), ([8., 6.], 0.75, 1.25)]
        .into_iter()
        .enumerate()
        .map(|(i, (center, radius_mm, height_mm))| NormalReliefMotif {
            boundary: vec![CurvedEdge2d::Circle { center, radius_mm }],
            height_mm,
            origin: Origin {
                source: feature,
                role: format!("motif/{i}"),
            },
        })
        .collect::<Vec<_>>();
    let relief = blank
        .rust_normal_relief_on_cap(face.index, cap, &motifs, kind, feature, policy, None)
        .unwrap();
    (blank, relief, owner)
}

#[test]
fn exact_relief_keeps_ancestry_picking_material_and_independent_exchange() {
    let policy = GeometryPolicy::default();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("relief.step");
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [7., -3., 2.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        for kind in [Kind::Raised, Kind::Recessed] {
            let feature = EntityId::new();
            let (blank, relief, owner) = fixture(kind, frame, feature);
            let sign = if kind == Kind::Raised { 1. } else { -1. };
            let expected = 360. + sign * std::f64::consts::PI * (0.5 + 0.75_f64.powi(2) * 1.25);
            assert!((relief.volume_mm3().unwrap() - expected).abs() < 1e-9);
            let before = blank.topology_snapshot(owner, 0).unwrap();
            let after = relief.topology_snapshot(owner, 1).unwrap();
            for face in before
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
            {
                let intent = GeometryIntent::capture(&before, face.key).unwrap();
                assert!(matches!(
                    resolve(&intent, &after, 1),
                    Resolution::Resolved { .. }
                ));
            }
            let mesh = relief
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            assert!(!mesh.mesh.indices.is_empty());
            relief.write_step(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let members = multicad_kernel::rust_kernel::step_read::read_import_compound(
                &text,
                policy.linear_tolerance_mm,
            )
            .unwrap();
            assert!(
                relief.step_readback_matches(
                    &after,
                    &members
                        .iter()
                        .map(|(r, s)| (r.clone(), Some(s.clone())))
                        .collect::<Vec<_>>(),
                    policy.linear_tolerance_mm
                )
            );
            let imported = Solid::import_rust_step(&text, feature, policy).unwrap();
            assert!((imported.volume_mm3().unwrap() - expected).abs() < 1e-6);
            imported
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
            let probes = if kind == Kind::Raised {
                vec![
                    ([3., 3., 3.2], true),
                    ([3., 3., 3.8], false),
                    ([8., 6., 4.], true),
                    ([6., 5., 3.2], false),
                    ([3., 3., 1.], true),
                ]
            } else {
                vec![
                    ([3., 3., 2.8], false),
                    ([3., 3., 2.], true),
                    ([8., 6., 2.], false),
                    ([8., 6., 1.], true),
                    ([6., 5., 2.8], true),
                ]
            };
            for &(p, expected) in &probes {
                assert_eq!(
                    relief.rust_normal_relief_contains(point(frame, p)).unwrap(),
                    expected
                );
                assert_eq!(
                    imported
                        .rust_normal_relief_contains(point(frame, p))
                        .unwrap(),
                    expected
                );
            }
            if OCCT_LINKED {
                let native = Solid::read_step(&path, policy).unwrap();
                assert_eq!(native.solid_count().unwrap(), 1);
                assert!((native.volume_mm3().unwrap() - expected).abs() < 1e-5);
                for &(p, expected) in &probes {
                    let probe = Solid::box_solid([0.01; 3], policy)
                        .unwrap()
                        .transformed(RigidTransform {
                            translation_mm: point(frame, p),
                            ..Default::default()
                        })
                        .unwrap();
                    let volume = native
                        .boolean(&probe, BooleanOperation::Intersect, policy)
                        .unwrap()
                        .map_or(0., |s| s.volume_mm3().unwrap());
                    assert!((volume / 1e-6 - f64::from(expected)).abs() < 1e-3);
                }
            }
            assert_eq!(blank.volume_mm3().unwrap(), 360.);
        }
    }
}
