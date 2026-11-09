use crate::{
    AffineTransform, BooleanOperation, GeometryPolicy, HelicalThread, PlaneFrame, RigidTransform,
    Solid, ffi,
};
use multicad_core::document::EntityId;

fn checks(solid: &Solid) -> u64 {
    ffi::validity_check_count(solid.occt().unwrap()).unwrap()
}
fn shares(a: &Solid, b: &Solid) -> bool {
    ffi::shares_validity_cache(a.occt().unwrap(), b.occt().unwrap())
}
fn accepted_once(solid: &Solid) {
    assert_eq!(checks(solid), 1);
    for _ in 0..8 {
        assert!(ffi::valid(solid.occt().unwrap()).unwrap());
    }
    assert_eq!(checks(solid), 1);
}
#[test]
fn successful_native_checks_are_reused_only_for_same_geometry_and_exact_identity_locations() {
    let policy = GeometryPolicy::default();
    let source = Solid::box_solid([4., 5., 6.], policy).unwrap();
    accepted_once(&source);
    let original = source.to_brep().unwrap();
    let original_bounds = source.bounds().unwrap();
    let shared = source.share().unwrap();
    assert!(shares(&source, &shared));
    let identity = source.transformed(RigidTransform::default()).unwrap();
    assert!(shares(&source, &identity));
    accepted_once(&identity);
    let moved = source
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]],
            translation_mm: [9., -3., 7.],
        })
        .unwrap();
    assert!(!shares(&source, &moved));
    accepted_once(&moved);
    let located_identity = moved.transformed(RigidTransform::default()).unwrap();
    assert!(shares(&moved, &located_identity));
    assert!(!shares(&source, &located_identity));
    accepted_once(&located_identity);
    assert_eq!(moved.bounds().unwrap(), located_identity.bounds().unwrap());
    let scaled = source
        .affine_transformed_named(
            AffineTransform {
                matrix: [[2., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                translation_mm: [0.; 3],
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
    let mirrored = source
        .mirrored_named(
            PlaneFrame {
                origin: [0.; 3],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
    let tool = Solid::box_solid([2., 5., 6.], policy).unwrap();
    let cut = source
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let imported = Solid::from_brep(&original, policy).unwrap();
    for solid in [&scaled, &mirrored, &cut, &imported] {
        assert!(!shares(&source, solid));
        accepted_once(solid);
    }
    assert_eq!(source.to_brep().unwrap(), original);
    assert_eq!(source.bounds().unwrap(), original_bounds);
    assert!((source.volume_mm3().unwrap() - 120.).abs() < 1e-10);
    accepted_once(&source);
}
#[test]
fn thread_factory_validity_reuse_keeps_owner_specific_persistent_face_metadata_separate() {
    ffi::reset_thread_factory_cache().unwrap();
    let policy = GeometryPolicy::default();
    let spec = HelicalThread {
        major_radius_mm: 2.,
        pitch_mm: 1.,
        length_mm: 1.,
        depth_mm: 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 1. / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    };
    let first_owner = EntityId::new();
    let second_owner = EntityId::new();
    let first = Solid::helical_thread_named(spec, first_owner, policy).unwrap();
    let source = first.to_brep().unwrap();
    let before = first.topology_snapshot(first_owner, 0).unwrap();
    let second = Solid::helical_thread_named(spec, second_owner, policy).unwrap();
    assert!(shares(&first, &second));
    accepted_once(&first);
    accepted_once(&second);
    let snapshot = second.topology_snapshot(second_owner, 0).unwrap();
    assert!(
        snapshot
            .elements
            .iter()
            .flat_map(|e| e.origins.iter())
            .all(|o| o.source == second_owner)
    );
    assert_eq!(first.topology_snapshot(first_owner, 0).unwrap(), before);
    assert_eq!(first.to_brep().unwrap(), source);
}
#[test]
fn unsuccessful_checks_are_not_cached_and_concurrent_same_shape_queries_publish_once() {
    assert!(ffi::validity_cache_rejection_probe().unwrap());
    let solid = Solid::box_solid([4., 5., 6.], GeometryPolicy::default()).unwrap();
    assert!(ffi::validity_cache_concurrent_probe(solid.occt().unwrap()).unwrap());
    accepted_once(&solid);
}
