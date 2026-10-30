//! A shared Solid is a second owner of the same immutable exact B-rep: it
//! reports identical geometry and topology, outlives the original, and using
//! either owner as an operation input never changes the other.
use multicad_core::document::EntityId;
use multicad_kernel::*;

fn fingerprint(solid: &Solid, owner: EntityId) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}",
        solid.volume_mm3().unwrap(),
        solid.bounds().unwrap(),
        solid.topology_snapshot(owner, 1).unwrap(),
        solid.mesh(MeshOptions::default()).unwrap(),
    )
}

#[test]
fn shared_solid_matches_and_outlives_its_source() {
    let owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let source = Solid::box_named([60., 40., 30.], owner, policy).unwrap();
    let expected = fingerprint(&source, owner);
    let shared = source.share().unwrap();
    assert_eq!(fingerprint(&shared, owner), expected);
    assert_eq!(shared.geometry_policy(), policy);
    drop(source);
    assert_eq!(fingerprint(&shared, owner), expected);
}

#[test]
fn operations_on_a_share_leave_the_other_owner_unchanged() {
    let owner = EntityId::new();
    let tool_owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let source = Solid::box_named([60., 40., 30.], owner, policy).unwrap();
    let tool = Solid::box_named([20., 20., 60.], tool_owner, policy).unwrap();
    let expected = fingerprint(&source, owner);
    let shared = source.share().unwrap();
    let first = shared
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let _moved = shared
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [5., 0., 0.],
        })
        .unwrap();
    assert_eq!(fingerprint(&source, owner), expected);
    assert_eq!(fingerprint(&shared, owner), expected);
    // The same operation on the other owner reproduces the same result.
    let second = source
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    assert_eq!(fingerprint(&first, owner), fingerprint(&second, owner));
}
