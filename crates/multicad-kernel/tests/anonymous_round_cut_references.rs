#![cfg(not(feature = "occt"))]

use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CircularPocketTool, GeometryPolicy, PlaneFrame, RigidTransform, Solid,
};

#[test]
fn sequential_cuts_rebuild_anonymous_metadata_and_retain_public_floor_references() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let stock = Solid::box_solid([20., 16., 10.], policy).unwrap();
    let cutter = Solid::box_solid([3., 18., 4.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [5., -1., 8.],
            ..Default::default()
        })
        .unwrap();
    let anonymous = stock
        .boolean(&cutter, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    assert!(
        anonymous
            .topology_snapshot(owner, 0)
            .unwrap()
            .elements
            .iter()
            .all(|e| e.origins.is_empty())
    );
    let cut_origin = Origin {
        source: EntityId::new(),
        role: "named pocket".into(),
    };
    let tool = CircularPocketTool {
        frame: PlaneFrame {
            origin: [12., 5., 11.],
            ..Default::default()
        },
        center_uv: [0.; 2],
        radius_mm: 1.5,
        displacement_mm: [0., 0., -4.],
        origin: cut_origin.clone(),
    };
    let first = anonymous
        .rust_drill_circular_pockets(&[tool], policy)
        .unwrap();
    let snapshot = first.topology_snapshot(owner, 1).unwrap();
    let floor = snapshot
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins.contains(&cut_origin)
                && e.origins
                    .iter()
                    .any(|o| o.role.starts_with("pocket.floor/"))
        })
        .unwrap();
    let intent = GeometryIntent::capture(&snapshot, floor.key).unwrap();
    assert!(
        snapshot
            .elements
            .iter()
            .flat_map(|e| &e.origins)
            .all(|o| o.source == cut_origin.source)
    );
    let second_origin = Origin {
        source: EntityId::new(),
        role: "second pocket".into(),
    };
    let second = first
        .rust_drill_circular_pockets(
            &[CircularPocketTool {
                frame: PlaneFrame {
                    origin: [12., 11., 11.],
                    ..Default::default()
                },
                center_uv: [0.; 2],
                radius_mm: 1.,
                displacement_mm: [0., 0., -3.],
                origin: second_origin.clone(),
            }],
            policy,
        )
        .unwrap();
    let updated = second.topology_snapshot(owner, 2).unwrap();
    assert!(
        matches!(resolve(&intent, &updated, 2), Resolution::Resolved { key, .. } if key == floor.key)
    );
    assert!(
        updated
            .elements
            .iter()
            .any(|e| e.origins.contains(&second_origin))
    );
    assert!(second.volume_mm3().unwrap() < first.volume_mm3().unwrap());
    let anonymous_tool = Solid::cylinder(
        PlaneFrame {
            origin: [17., 11., 8.],
            ..Default::default()
        },
        0.8,
        3.,
        policy,
    )
    .unwrap();
    let third = second
        .boolean(&anonymous_tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let final_snapshot = third.topology_snapshot(owner, 3).unwrap();
    assert!(
        matches!(resolve(&intent, &final_snapshot, 3), Resolution::Resolved { key, .. } if key == floor.key)
    );
    assert!(
        final_snapshot
            .elements
            .iter()
            .flat_map(|e| &e.origins)
            .all(|o| { o.source == cut_origin.source || o.source == second_origin.source })
    );
    assert!(third.volume_mm3().unwrap() < second.volume_mm3().unwrap());
    assert_eq!(stock.volume_mm3().unwrap(), 3200.);
    assert!(
        anonymous
            .topology_snapshot(owner, 0)
            .unwrap()
            .elements
            .iter()
            .all(|e| e.origins.is_empty())
    );
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("cut.step");
    third.write_step(&path).unwrap();
    let reopened = Solid::read_step(&path, policy).unwrap();
    assert!((reopened.volume_mm3().unwrap() - third.volume_mm3().unwrap()).abs() < 1e-6);
}
