use crate::{GeometryPolicy, HelicalThread, MeshOptions, PlaneFrame, RigidTransform, Solid, ffi};
use multicad_core::document::EntityId;

fn cylinder(radius: f64) -> Solid {
    Solid::cylinder(
        PlaneFrame {
            origin: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        radius,
        3.,
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn shares(a: &Solid, b: &Solid) -> bool {
    ffi::shares_mesh_identity(a.occt().unwrap(), b.occt().unwrap())
}
fn options() -> MeshOptions {
    MeshOptions::default()
}
fn reset() {
    ffi::reset_mesh_cache().unwrap();
}
#[test]
fn curved_mesh_warm_numeric_receipts_equal_forced_fresh_without_mutating_native_geometry() {
    reset();
    let owner = EntityId::new();
    let source = Solid::helical_thread_named(
        HelicalThread {
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
        },
        owner,
        GeometryPolicy::default(),
    )
    .unwrap();
    let before = source.to_brep().unwrap();
    let refs = source.topology_snapshot(owner, 0).unwrap();
    let volume = source.volume_mm3().unwrap();
    let bounds = source.bounds().unwrap();
    let first = source.mesh(options()).unwrap();
    let built = ffi::mesh_cache_stats().unwrap();
    assert_eq!(built.builds, 1);
    assert_eq!(built.hits, 0);
    assert_eq!(first, source.mesh(options()).unwrap());
    let warm = ffi::mesh_cache_stats().unwrap();
    assert_eq!(warm.builds, 1);
    assert_eq!(warm.hits, 1);
    let fresh = ffi::tessellate_fresh(
        source.occt().unwrap(),
        options().linear_deflection_mm,
        options().angular_deflection_rad,
        options().maximum_triangles,
    )
    .unwrap();
    assert_eq!(first, fresh);
    let alias = source.share().unwrap();
    let identity = source.transformed(RigidTransform::default()).unwrap();
    assert!(shares(&source, &alias));
    assert!(shares(&source, &identity));
    assert_eq!(first, alias.mesh(options()).unwrap());
    assert_eq!(first, identity.mesh(options()).unwrap());
    let second = Solid::helical_thread_named(
        HelicalThread {
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
        },
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    assert!(shares(&source, &second));
    assert_eq!(first, second.mesh(options()).unwrap());
    assert_eq!(source.to_brep().unwrap(), before);
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), refs);
    assert_eq!(source.volume_mm3().unwrap(), volume);
    assert_eq!(source.bounds().unwrap(), bounds);
}
#[test]
fn every_mesh_option_bit_is_keyed_and_failed_or_invalid_settings_publish_nothing() {
    reset();
    let source = cylinder(2.);
    source.mesh(options()).unwrap();
    let mut variants = [options(); 3];
    variants[0].linear_deflection_mm =
        f64::from_bits(variants[0].linear_deflection_mm.to_bits() + 1);
    variants[1].angular_deflection_rad =
        f64::from_bits(variants[1].angular_deflection_rad.to_bits() + 1);
    variants[2].maximum_triangles += 1;
    for value in variants {
        let before = ffi::mesh_cache_stats().unwrap();
        source.mesh(value).unwrap();
        let after = ffi::mesh_cache_stats().unwrap();
        assert_eq!(after.hits, before.hits);
        assert_eq!(after.builds, before.builds + 1);
    }
    let before = ffi::mesh_cache_stats().unwrap();
    assert!(
        source
            .mesh(MeshOptions {
                maximum_triangles: 1,
                ..options()
            })
            .is_err()
    );
    let after = ffi::mesh_cache_stats().unwrap();
    assert_eq!(after.hits, before.hits);
    assert_eq!(after.entries, before.entries);
    let before = ffi::mesh_cache_stats().unwrap();
    for value in [
        MeshOptions {
            linear_deflection_mm: f64::NAN,
            ..options()
        },
        MeshOptions {
            angular_deflection_rad: 0.,
            ..options()
        },
        MeshOptions {
            maximum_triangles: 0,
            ..options()
        },
    ] {
        assert!(source.mesh(value).is_err());
    }
    let after = ffi::mesh_cache_stats().unwrap();
    assert_eq!(after.requests, before.requests);
    assert_eq!(after.entries, before.entries);
}
#[test]
fn numeric_receipts_obey_lru_entry_total_and_per_receipt_bounds() {
    ffi::mesh_cache_test_limits(2, 32 * 1024 * 1024, 4 * 1024 * 1024).unwrap();
    let (a, b, c) = (cylinder(2.), cylinder(3.), cylinder(4.));
    a.mesh(options()).unwrap();
    b.mesh(options()).unwrap();
    a.mesh(options()).unwrap();
    c.mesh(options()).unwrap();
    let full = ffi::mesh_cache_stats().unwrap();
    assert_eq!(full.entries, 2);
    assert_eq!(full.evictions, 1);
    assert!(full.bytes <= 32 * 1024 * 1024);
    let before = full.builds;
    b.mesh(options()).unwrap();
    assert_eq!(ffi::mesh_cache_stats().unwrap().builds, before + 1);
    for (entries, total, receipt) in [
        (0, 32 * 1024 * 1024, 4 * 1024 * 1024),
        (2, 1, 1),
        (2, 32 * 1024 * 1024, 1),
    ] {
        ffi::mesh_cache_test_limits(entries, total, receipt).unwrap();
        a.mesh(options()).unwrap();
        let stats = ffi::mesh_cache_stats().unwrap();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.bytes, 0);
        assert!(stats.unretained > 0);
    }
    assert!(ffi::mesh_cache_test_limits(17, 32 * 1024 * 1024, 4 * 1024 * 1024).is_err());
    assert!(ffi::mesh_cache_test_limits(16, 32 * 1024 * 1024 + 1, 4 * 1024 * 1024).is_err());
    assert!(ffi::mesh_cache_test_limits(16, 32 * 1024 * 1024, 4 * 1024 * 1024 + 1).is_err());
    reset();
}
#[test]
fn changed_geometry_and_nonidentity_world_poses_and_imports_have_independent_mesh_identity() {
    reset();
    let policy = GeometryPolicy::default();
    let source = cylinder(2.);
    source.mesh(options()).unwrap();
    let moved = source
        .transformed(RigidTransform {
            translation_mm: [3., 4., 5.],
            ..Default::default()
        })
        .unwrap();
    let changed = cylinder(2.1);
    let imported = Solid::from_brep(&source.to_brep().unwrap(), policy).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mesh-source.step");
    source.write_step(&path).unwrap();
    let step = Solid::read_step(&path, policy).unwrap();
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
    for other in [&moved, &changed, &imported, &step, &mirrored] {
        assert!(!shares(&source, other));
        let before = ffi::mesh_cache_stats().unwrap();
        let mesh = other.mesh(options()).unwrap();
        let after = ffi::mesh_cache_stats().unwrap();
        assert_eq!(after.hits, before.hits);
        assert_eq!(after.builds, before.builds + 1);
        let fresh = ffi::tessellate_fresh(
            other.occt().unwrap(),
            options().linear_deflection_mm,
            options().angular_deflection_rad,
            options().maximum_triangles,
        )
        .unwrap();
        assert_eq!(mesh, fresh);
    }
}
#[test]
fn singleton_compound_reuse_preserves_face_ordinals_but_distinct_members_and_other_owners_stay_separate()
 {
    reset();
    let policy = GeometryPolicy::default();
    let source = cylinder(2.);
    let mesh = source.mesh(options()).unwrap();
    let singleton = Solid::compound_named(&[&source], EntityId::new(), policy).unwrap();
    let nested = Solid::compound_named(&[&singleton], EntityId::new(), policy).unwrap();
    for value in [&singleton, &nested] {
        assert!(shares(&source, value));
        assert_eq!(mesh, value.mesh(options()).unwrap());
    }
    let shared = source.share().unwrap();
    let dual = Solid::compound_named(&[&source, &shared], EntityId::new(), policy).unwrap();
    assert!(!shares(&source, &dual));
    let before = ffi::mesh_cache_stats().unwrap();
    let double = dual.mesh(options()).unwrap();
    assert_eq!(double.indices.len(), 2 * mesh.indices.len());
    assert_eq!(ffi::mesh_cache_stats().unwrap().builds, before.builds + 1);
    let parent = ffi::mesh_cache_stats().unwrap();
    let child = std::thread::spawn(|| {
        reset();
        let shape = cylinder(2.);
        shape.mesh(options()).unwrap();
        let stats = ffi::mesh_cache_stats().unwrap();
        (stats.hits, stats.builds, stats.entries)
    })
    .join()
    .unwrap();
    assert_eq!(child, (0, 1, 1));
    let after = ffi::mesh_cache_stats().unwrap();
    assert_eq!(after.hits, parent.hits);
    assert_eq!(after.builds, parent.builds);
    assert_eq!(after.entries, parent.entries);
    // TLS lifetime ends with the child; a new owner starts without its receipt.
    std::thread::spawn(|| {
        reset();
        assert_eq!(ffi::mesh_cache_stats().unwrap().entries, 0);
    })
    .join()
    .unwrap();
}
