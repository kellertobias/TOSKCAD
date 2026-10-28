use multicad_core::document::EntityId;
use multicad_kernel::*;
fn p() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn moved(solid: &Solid, offset: [f64; 3]) -> Solid {
    solid
        .transformed(RigidTransform {
            translation_mm: offset,
            ..Default::default()
        })
        .unwrap()
}
#[test]
fn uuid_member_names_survive_compound_reorder_without_losing_ancestry() {
    use multicad_core::references::*;
    let part = EntityId::new();
    let assembly = EntityId::new();
    let first = EntityId::new();
    let second = EntityId::new();
    let source = Solid::box_named([10.; 3], part, p()).unwrap();
    let before = source.topology_snapshot(part, 0).unwrap();
    let a = moved(&source, [0.; 3])
        .with_occurrence_origin(assembly, first)
        .unwrap();
    let b = moved(&source, [20., 0., 0.])
        .with_occurrence_origin(assembly, second)
        .unwrap();
    let snapshot = a.topology_snapshot(assembly, 0).unwrap();
    let cap = snapshot
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins.contains(&Origin {
                    source: part,
                    role: "cap.end".into(),
                })
        })
        .unwrap();
    let intent = GeometryIntent::capture(&snapshot, cap.key).unwrap();
    for solids in [[&a, &b], [&b, &a]] {
        let compound = Solid::compound_named(&solids, assembly, p()).unwrap();
        assert!(matches!(
            resolve(
                &intent,
                &compound.topology_snapshot(assembly, 1).unwrap(),
                1
            ),
            Resolution::Resolved { .. }
        ));
    }
    assert_eq!(source.topology_snapshot(part, 0).unwrap(), before);
    assert!(
        moved(&source, [0.; 3])
            .with_occurrence_origin(
                assembly,
                EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap()
            )
            .is_err()
    );
}
