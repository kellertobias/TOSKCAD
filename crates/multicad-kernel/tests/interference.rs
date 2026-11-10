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
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

fn rust_stock(size: [i64; 3], offset: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|n| rust_kernel::Rational::new(n, 1).unwrap()),
        PlaneFrame { origin: offset, ..Default::default() },
        p(),
    )
    .unwrap()
}

#[test]
fn rust_pair_verifies_gap_contact_interference_and_containment() {
    let a = rust_stock([10; 3], [0.; 3]);
    for (b, expected, volume, clearance) in [
        (rust_stock([10; 3], [10.5, 0., 0.]), SolidPairRelation::Separated, 0., 0.5),
        (rust_stock([10; 3], [10.; 3]), SolidPairRelation::ContactWithinTolerance, 0., 0.),
        (rust_stock([10; 3], [5., 0., 0.]), SolidPairRelation::Overlapping, 500., 0.),
        (rust_stock([2; 3], [2.; 3]), SolidPairRelation::Overlapping, 8., 0.),
    ] {
        let result = a.inspect_pair(&b, 1e-7, p()).unwrap();
        assert_eq!(result.relation, expected);
        near(result.common_volume_mm3, volume);
        near(result.minimum_distance.distance_mm, clearance);
        assert!(!result.minimum_distance.witnesses.is_empty());
    }
    let mixed = Solid::box_solid([10.; 3], p()).unwrap();
    assert_eq!(a.inspect_pair(&mixed, 1e-7, p()).unwrap_err().kind, GeometryErrorKind::Unsupported);
}

#[test]
fn rust_pair_does_not_fill_cavities_or_double_count_compound_members() {
    let box_ = rust_stock([20; 3], [0.; 3]);
    let cavity = rust_stock([8; 3], [6.; 3]);
    let hollow = box_.boolean(&cavity, BooleanOperation::Subtract, p()).unwrap().unwrap();
    let probe = rust_stock([4; 3], [8.; 3]);
    let clear = hollow.inspect_pair(&probe, 1e-7, p()).unwrap();
    assert_eq!(clear.relation, SolidPairRelation::Separated);
    near(clear.common_volume_mm3, 0.);
    near(clear.minimum_distance.distance_mm, 2.);

    let a = rust_stock([10; 3], [0.; 3]);
    let b = rust_stock([10; 3], [5., 0., 0.]);
    let compound = Solid::compound_named(&[&a, &b], EntityId::new(), p()).unwrap();
    let probe = rust_stock([4, 10, 10], [8., 0., 0.]);
    let overlap = compound.inspect_pair(&probe, 1e-7, p()).unwrap();
    assert_eq!(overlap.relation, SolidPairRelation::Overlapping);
    near(overlap.common_volume_mm3, 400.);
}

#[test]
fn rust_pair_certifies_rotated_clearance_even_when_world_bounds_overlap() {
    let d = 0.5_f64.sqrt();
    let rotation = [[d, -d, 0.], [d, d, 0.], [0., 0., 1.]];
    let measure = |solid: Solid| {
        let a = solid
            .transformed(RigidTransform { rotation, ..Default::default() })
            .unwrap();
        let b = moved(&a, [-3. * d, 3. * d, 0.]);
        let [a_box, b_box] = [&a, &b].map(|body| body.bounds().unwrap());
        assert!((0..3).all(|i| a_box.min[i] < b_box.max[i] && b_box.min[i] < a_box.max[i]));
        a.inspect_pair(&b, 1e-7, p()).unwrap()
    };
    let rust = measure(rust_stock([10, 2, 2], [0.; 3]));
    let native = measure(Solid::box_solid([10., 2., 2.], p()).unwrap());
    assert_eq!(rust.relation, SolidPairRelation::Separated);
    near(rust.minimum_distance.distance_mm, 1.);
    near(rust.minimum_distance.distance_mm, native.minimum_distance.distance_mm);
    near(rust.common_volume_mm3, 0.);
}
#[test]
fn exact_common_distinguishes_gap_touching_overlap_and_containment_without_mutation() {
    let owner = EntityId::new();
    let a = Solid::box_named([10.; 3], owner, p()).unwrap();
    let before = a.topology_snapshot(owner, 0).unwrap();
    for (offset, relation, volume, distance) in [
        ([10.5, 0., 0.], SolidPairRelation::Separated, 0., 0.5),
        (
            [10., 0., 0.],
            SolidPairRelation::ContactWithinTolerance,
            0.,
            0.,
        ),
        (
            [10., 10., 0.],
            SolidPairRelation::ContactWithinTolerance,
            0.,
            0.,
        ),
        ([10.; 3], SolidPairRelation::ContactWithinTolerance, 0., 0.),
        ([5., 0., 0.], SolidPairRelation::Overlapping, 500., 0.),
    ] {
        let b = moved(&a, offset);
        let result = a.inspect_pair(&b, 1e-7, p()).unwrap();
        assert_eq!(result.relation, relation);
        near(result.common_volume_mm3, volume);
        near(result.minimum_distance.distance_mm, distance);
        assert!(result.minimum_distance.witnesses.len() <= 64);
    }
    let b = moved(&Solid::box_solid([2.; 3], p()).unwrap(), [2.; 3]);
    let contained = a.inspect_pair(&b, 1e-7, p()).unwrap();
    assert_eq!(contained.relation, SolidPairRelation::Overlapping);
    near(contained.common_volume_mm3, 8.);
    assert!(contained.minimum_distance.inner_solution);
    assert_eq!(a.topology_snapshot(owner, 0).unwrap(), before);
    for tolerance in [0., 1e-9, f64::NAN, f64::INFINITY, 1.] {
        assert!(a.inspect_pair(&b, tolerance, p()).is_err());
    }
}
#[test]
fn hollow_solid_aabb_overlap_never_becomes_interference_or_zero_clearance() {
    let outer = Solid::cylinder(PlaneFrame::default(), 10., 10., p()).unwrap();
    let hole = Solid::cylinder(
        PlaneFrame {
            origin: [0., 0., -1.],
            ..Default::default()
        },
        6.,
        12.,
        p(),
    )
    .unwrap();
    let tube = outer
        .boolean(&hole, BooleanOperation::Subtract, p())
        .unwrap()
        .unwrap();
    let inside_hole = Solid::cylinder(
        PlaneFrame {
            origin: [0., 0., 2.],
            ..Default::default()
        },
        2.,
        6.,
        p(),
    )
    .unwrap();
    let result = tube.inspect_pair(&inside_hole, 1e-7, p()).unwrap();
    assert_eq!(result.relation, SolidPairRelation::Separated);
    near(result.common_volume_mm3, 0.);
    near(result.minimum_distance.distance_mm, 4.);
}
#[test]
fn overlapping_compound_members_count_actual_common_occupied_volume_once() {
    let a = Solid::box_solid([10.; 3], p()).unwrap();
    let b = moved(&a, [5., 0., 0.]);
    let pattern = Solid::compound_named(&[&a, &b], EntityId::new(), p()).unwrap();
    let probe = moved(
        &Solid::box_solid([4., 10., 10.], p()).unwrap(),
        [8., 0., 0.],
    );
    let result = pattern.inspect_pair(&probe, 1e-7, p()).unwrap();
    assert_eq!(result.relation, SolidPairRelation::Overlapping);
    near(result.common_volume_mm3, 400.);
    let distant = moved(&a, [20., 0., 0.]);
    let disjoint = Solid::compound_named(&[&a, &distant], EntityId::new(), p()).unwrap();
    let contained = moved(&Solid::box_solid([1.; 3], p()).unwrap(), [2.; 3]);
    let result = disjoint.inspect_pair(&contained, 1e-7, p()).unwrap();
    assert_eq!(result.relation, SolidPairRelation::Overlapping);
    near(result.common_volume_mm3, 1.);
    near(result.minimum_distance.distance_mm, 0.);
}
#[test]
fn rotated_intersecting_solids_keep_only_consistent_native_witnesses() {
    let a = Solid::box_solid([10.; 3], p()).unwrap();
    let angle: f64 = 1.;
    let (s, c) = angle.sin_cos();
    let b = a
        .transformed(RigidTransform {
            rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
            translation_mm: [0.; 3],
        })
        .unwrap();
    let result = a.inspect_pair(&b, 1e-7, p()).unwrap();
    assert_eq!(result.relation, SolidPairRelation::Overlapping);
    near(result.minimum_distance.distance_mm, 0.);
    assert!(!result.minimum_distance.witnesses.is_empty());
    for w in &result.minimum_distance.witnesses {
        near(
            (0..3)
                .map(|i| (w.a_mm[i] - w.b_mm[i]).powi(2))
                .sum::<f64>()
                .sqrt(),
            0.,
        );
    }
    assert!(!result.minimum_distance.witnesses_complete);
    assert!(
        result.minimum_distance.total_solutions > result.minimum_distance.witnesses.len() as u32
    );
}
#[test]
fn independent_native_owners_inspect_pairs_concurrently_without_source_mutation() {
    let ready = std::sync::Arc::new(std::sync::Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let ready = ready.clone();
            std::thread::spawn(move || {
                ready.wait();
                let owner = EntityId::new();
                let a = Solid::box_named([10.; 3], owner, p()).unwrap();
                let b = moved(&a, [5., 0., 0.]);
                let before = a.topology_snapshot(owner, 0).unwrap();
                for _ in 0..8 {
                    let result = a.inspect_pair(&b, 1e-7, p()).unwrap();
                    assert_eq!(result.relation, SolidPairRelation::Overlapping);
                    near(result.common_volume_mm3, 500.);
                    near(result.minimum_distance.distance_mm, 0.);
                }
                assert_eq!(a.topology_snapshot(owner, 0).unwrap(), before);
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}
