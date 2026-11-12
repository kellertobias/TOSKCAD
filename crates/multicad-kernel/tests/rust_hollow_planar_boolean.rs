use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid, rust_kernel::Rational,
};

fn profile() -> CurvedProfile {
    let wire = |p: &[[f64; 2]]| {
        (0..p.len())
            .map(|i| CurvedEdge2d::Line {
                a: p[i],
                b: p[(i + 1) % p.len()],
            })
            .collect()
    };
    CurvedProfile {
        frame: PlaneFrame::default(),
        outer: wire(&[[0., 0.], [12., 0.], [12., 10.], [0., 10.]]),
        holes: vec![wire(&[[4., 3.], [4., 7.], [8., 7.], [8., 3.]])],
    }
}
fn box_at(size: [i64; 3], origin: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn posed_hollow_stock_cuts_keep_material_and_untouched_wall_intents() {
    let policy = GeometryPolicy::default();
    for reverse in [false, true] {
        let height = if reverse { -10. } else { 10. };
        let owner = EntityId::new();
        let origins: Vec<_> = (0..8)
            .map(|i| Origin {
                source: owner,
                role: format!("wall.{i}"),
            })
            .collect();
        let stock = Solid::rust_extrude_linear_named(
            &profile(),
            [0., 0., height],
            0.,
            owner,
            &origins,
            policy,
        )
        .unwrap();
        let tool = box_at([7, 12, 8], [6., -1., if reverse { -13. } else { 5. }]);
        for pose in [
            RigidTransform::default(),
            RigidTransform {
                rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
                translation_mm: [80., -35., 12.],
            },
        ] {
            let stock = stock.transformed(pose).unwrap();
            let tool = tool.transformed(pose).unwrap();
            let before = stock.topology_snapshot(owner, 0).unwrap();
            let left = before
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == "wall.3")
                })
                .unwrap();
            let intent = GeometryIntent::capture(&before, left.key).unwrap();
            let cut = stock
                .boolean(&tool, BooleanOperation::Subtract, policy)
                .unwrap()
                .unwrap();
            assert!(cut.uses_rust_geometry());
            assert_eq!(cut.solid_count().unwrap(), 1);
            near(stock.volume_mm3().unwrap(), 1040.);
            near(cut.volume_mm3().unwrap(), 780.);
            let after = cut.topology_snapshot(owner, 1).unwrap();
            let resolution = resolve(&intent, &after, 1);
            assert!(
                matches!(resolution, Resolution::Resolved { .. }),
                "{reverse} {pose:?}: {resolution:?}\nbefore: {left:?}\nafter: {:?}",
                after
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Face
                        && e.origins.iter().any(|o| o.role == "wall.3"))
                    .collect::<Vec<_>>()
            );
            for (p, occupied) in [
                ([2., 2., 2.], true),
                ([5., 5., 2.], false),
                ([9., 5., 2.], true),
                ([9., 5., 8.], false),
                ([2., 5., 8.], true),
                ([13., 5., 2.], false),
            ] {
                let [x, y, z] = p;
                let z = if reverse { -z } else { z };
                let probe = Solid::rust_box(
                    EntityId::new().uuid_bits(),
                    [Rational::new(1, 10).unwrap(); 3],
                    PlaneFrame {
                        origin: [x - 0.05, y - 0.05, z - 0.05],
                        ..Default::default()
                    },
                    policy,
                )
                .unwrap()
                .transformed(pose)
                .unwrap();
                let volume = cut
                    .boolean(&probe, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .map_or(0., |s| s.volume_mm3().unwrap());
                near(volume, if occupied { 0.001 } else { 0. });
            }
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("cut.step");
            cut.write_step(&path).unwrap();
            if multicad_kernel::OCCT_LINKED {
                let native = Solid::extrude_curved_profile_vector_named(
                    &profile(),
                    [0., 0., height],
                    owner,
                    &origins,
                    policy,
                )
                .unwrap()
                .transformed(pose)
                .unwrap();
                let native_tool = Solid::box_solid([7., 12., 8.], policy)
                    .unwrap()
                    .transformed(RigidTransform {
                        translation_mm: [6., -1., if reverse { -13. } else { 5. }],
                        ..Default::default()
                    })
                    .unwrap()
                    .transformed(pose)
                    .unwrap();
                let expected = native
                    .boolean(&native_tool, BooleanOperation::Subtract, policy)
                    .unwrap()
                    .unwrap();
                near(cut.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
                let imported = Solid::read_step(&path, policy).unwrap();
                assert_eq!(imported.solid_count().unwrap(), 1);
                near(imported.volume_mm3().unwrap(), 780.);
                let [a, b] = [cut.bounds().unwrap(), imported.bounds().unwrap()];
                for i in 0..3 {
                    near(a.min[i], b.min[i]);
                    near(a.max[i], b.max[i]);
                }
            }
        }
    }
}
