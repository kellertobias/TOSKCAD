use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{GeometryErrorKind, GeometryPolicy, RigidTransform, Solid};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};
fn policy() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn moved(source: &Solid, offset: f64) -> Solid {
    source
        .transformed(RigidTransform {
            translation_mm: [offset, 0., 0.],
            ..Default::default()
        })
        .unwrap()
}
fn component_id(solid: &Solid, owner: EntityId) -> EntityId {
    let snapshot = solid.topology_snapshot(owner, 0).unwrap();
    let tags = snapshot
        .elements
        .iter()
        .flat_map(|e| &e.origins)
        .filter(|origin| origin.source == owner && origin.role.starts_with("component/"))
        .map(|origin| origin.role.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(tags.len(), 1);
    EntityId::parse(
        tags.iter()
            .next()
            .unwrap()
            .strip_prefix("component/")
            .unwrap(),
    )
    .unwrap()
}
#[test]
fn exact_components_preserve_touching_overlapping_members_and_source_identity_under_reorder() {
    let source_id = EntityId::new();
    let owner = EntityId::new();
    let first = EntityId::new();
    let second = EntityId::new();
    let third = EntityId::new();
    let source = Solid::box_named([10.; 3], source_id, policy()).unwrap();
    let before = source.topology_snapshot(source_id, 0).unwrap();
    let a = moved(&source, 0.)
        .with_component_origin(owner, first)
        .unwrap();
    let b = moved(&source, 10.)
        .with_component_origin(owner, second)
        .unwrap();
    let c = moved(&source, 5.)
        .with_component_origin(owner, third)
        .unwrap();
    let selected = a.topology_snapshot(owner, 0).unwrap();
    let face = selected
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins.contains(&Origin {
                    source: source_id,
                    role: "cap.end".into(),
                })
        })
        .unwrap();
    let intent = GeometryIntent::capture(&selected, face.key).unwrap();
    for members in [[&a, &b, &c], [&c, &b, &a]] {
        let compound = Solid::compound_named(&members, owner, policy()).unwrap();
        let original = compound.topology_snapshot(owner, 1).unwrap();
        let children = compound.solid_components(policy()).unwrap();
        assert_eq!(children.len(), 3);
        let mut facts = BTreeMap::new();
        for child in &children {
            assert_eq!(child.solid_count().unwrap(), 1);
            assert!((child.volume_mm3().unwrap() - 1000.).abs() < 1e-6);
            let id = component_id(child, owner);
            facts.insert(id, child.bounds().unwrap().min[0]);
            let result = resolve(&intent, &child.topology_snapshot(owner, 2).unwrap(), 2);
            assert_eq!(
                matches!(result, Resolution::Resolved { .. }),
                id == first,
                "A member's face intent must not rebind to another component"
            );
        }
        assert_eq!(
            facts,
            BTreeMap::from([(first, 0.), (second, 10.), (third, 5.)])
        );
        assert_eq!(compound.topology_snapshot(owner, 1).unwrap(), original);
        assert!(
            (compound.volume_mm3().unwrap() - 3000.).abs() < 1e-6,
            "Extraction preserves a compound; it does not union overlapping volumes"
        );
    }
    assert_eq!(source.topology_snapshot(source_id, 0).unwrap(), before);
}
#[test]
fn copied_components_outlive_the_parent_and_have_independent_step_geometry() {
    let owner = EntityId::new();
    let member = EntityId::new();
    let source = Solid::box_named([12., 8., 4.], owner, policy())
        .unwrap()
        .with_component_origin(owner, member)
        .unwrap();
    let second = moved(&source, 30.);
    let compound = Solid::compound_named(&[&source, &second], owner, policy()).unwrap();
    let children = compound.solid_components(policy()).unwrap();
    drop(compound);
    drop(source);
    drop(second);
    let directory = tempfile::tempdir().unwrap();
    for (i, child) in children.into_iter().enumerate() {
        assert_eq!(component_id(&child, owner), member);
        let path = directory.path().join(format!("component-{i}.step"));
        child.write_step(&path).unwrap();
        let reopened = Solid::read_step(&path, policy()).unwrap();
        assert_eq!(reopened.solid_count().unwrap(), 1);
        assert!((reopened.volume_mm3().unwrap() - 384.).abs() < 1e-5);
        let expected = child.bounds().unwrap();
        let actual = reopened.bounds().unwrap();
        for axis in 0..3 {
            assert!((expected.min[axis] - actual.min[axis]).abs() < 1e-6);
            assert!((expected.max[axis] - actual.max[axis]).abs() < 1e-6);
        }
    }
}
#[test]
fn cancellation_and_identity_errors_do_not_publish_partial_copies_or_mutate_the_source() {
    let owner = EntityId::new();
    let member = EntityId::new();
    let source = Solid::box_named([10.; 3], owner, policy()).unwrap();
    assert!(
        moved(&source, 0.)
            .with_component_origin(EntityId::nil(), member)
            .is_err()
    );
    assert!(
        moved(&source, 0.)
            .with_component_origin(owner, EntityId::nil())
            .is_err()
    );
    let a = moved(&source, 0.)
        .with_component_origin(owner, member)
        .unwrap();
    assert!(
        moved(&a, 0.)
            .with_component_origin(owner, EntityId::new())
            .is_err()
    );
    let b = moved(&source, 20.)
        .with_component_origin(owner, EntityId::new())
        .unwrap();
    let compound = Solid::compound_named(&[&a, &b], owner, policy()).unwrap();
    let before = compound.topology_snapshot(owner, 0).unwrap();
    let calls = AtomicUsize::new(0);
    let error = match compound.solid_components_interruptible(
        policy(),
        Some(&|| calls.fetch_add(1, Ordering::SeqCst) >= 2),
    ) {
        Ok(_) => panic!("Cancelled extraction published partial copies"),
        Err(error) => error,
    };
    assert_eq!(error.kind, GeometryErrorKind::Cancelled);
    assert_eq!(compound.topology_snapshot(owner, 0).unwrap(), before);
    assert_eq!(compound.solid_components(policy()).unwrap().len(), 2);
    let calls = AtomicUsize::new(0);
    let error = match Solid::compound_named_interruptible(
        &[&a, &b],
        owner,
        policy(),
        Some(&|| calls.fetch_add(1, Ordering::SeqCst) >= 2),
    ) {
        Ok(_) => panic!("Cancelled construction published a partial compound"),
        Err(error) => error,
    };
    assert_eq!(error.kind, GeometryErrorKind::Cancelled);
    assert_eq!(compound.topology_snapshot(owner, 0).unwrap(), before);
}
