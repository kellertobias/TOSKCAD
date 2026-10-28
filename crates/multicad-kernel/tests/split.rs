use multicad_kernel::*;
use std::cell::Cell;
fn p() -> GeometryPolicy {
    Default::default()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-6, "{a} != {b}");
}
fn cube(size: [f64; 3], origin: [f64; 3]) -> Solid {
    Solid::box_solid(size, p())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: origin,
            ..Default::default()
        })
        .unwrap()
}
#[test]
fn semantic_partitions_change_size_without_swapping_and_recombine_exactly() {
    let target = cube([10.; 3], [0.; 3]);
    let before = target.bounds().unwrap();
    for x in [4., 6.] {
        let tool = cube([12.; 3], [x, -1., -1.]);
        let split = target.split_by_solid(&tool, p()).unwrap();
        near(split.inside.volume_mm3().unwrap(), (10. - x) * 100.);
        near(split.outside.volume_mm3().unwrap(), x * 100.);
        near(split.inside.bounds().unwrap().min[0], x);
        near(split.outside.bounds().unwrap().max[0], x);
        assert_eq!(split.inside.solid_count().unwrap(), 1);
        assert_eq!(split.outside.solid_count().unwrap(), 1);
        assert!(
            split
                .inside
                .boolean(&split.outside, BooleanOperation::Intersect, p())
                .unwrap()
                .is_none()
        );
        let restored = split
            .inside
            .boolean(&split.outside, BooleanOperation::Union, p())
            .unwrap()
            .unwrap();
        near(restored.volume_mm3().unwrap(), 1000.);
        assert_eq!(restored.bounds().unwrap(), before);
    }
    assert_eq!(target.bounds().unwrap(), before);
    near(target.volume_mm3().unwrap(), 1000.);
}
#[test]
fn enclosed_tool_yields_independent_inner_solid_and_outer_cavity() {
    let target = cube([10.; 3], [0.; 3]);
    let tool = cube([2.; 3], [4.; 3]);
    let split = target.split_by_solid(&tool, p()).unwrap();
    near(split.inside.volume_mm3().unwrap(), 8.);
    near(split.outside.volume_mm3().unwrap(), 992.);
    assert_eq!(split.outside.solid_count().unwrap(), 1);
}
#[test]
fn no_hit_tangency_and_containment_reject_the_whole_pair() {
    let target = cube([10.; 3], [0.; 3]);
    for (size, origin) in [
        ([2.; 3], [20.; 3]),
        ([2.; 3], [10., 0., 0.]),
        ([12.; 3], [-1.; 3]),
        ([10.; 3], [0.; 3]),
    ] {
        let tool = cube(size, origin);
        assert!(target.split_by_solid(&tool, p()).is_err());
        near(target.volume_mm3().unwrap(), 1000.);
    }
}
#[test]
fn disconnected_inside_or_outside_requires_explicit_further_split_not_ordinal_identity() {
    let target = cube([10.; 3], [0.; 3]);
    let slab = cube([2., 12., 12.], [4., -1., -1.]);
    let e = target.split_by_solid(&slab, p()).err().unwrap();
    assert!(e.message.contains("OutsideTool") && e.message.contains("disconnected"));
    let left = cube([2., 4., 10.], [0.; 3]);
    let right = cube([2., 4., 10.], [8., 0., 0.]);
    let bridge = cube([10., 4., 2.], [0.; 3]);
    let u = left
        .boolean(&bridge, BooleanOperation::Union, p())
        .unwrap()
        .unwrap()
        .boolean(&right, BooleanOperation::Union, p())
        .unwrap()
        .unwrap();
    assert_eq!(u.solid_count().unwrap(), 1);
    let top = cube([12., 6., 6.], [-1., -1., 5.]);
    let e = u.split_by_solid(&top, p()).err().unwrap();
    assert!(e.message.contains("InsideTool") && e.message.contains("disconnected"));
}
#[test]
fn cancellation_between_native_calls_discards_all_partitions() {
    let target = cube([10.; 3], [0.; 3]);
    let tool = cube([12.; 3], [4., -1., -1.]);
    let calls = Cell::new(0);
    let cancel = || {
        calls.set(calls.get() + 1);
        calls.get() >= 2
    };
    let e = target
        .split_by_solid_interruptible(&tool, p(), Some(&cancel))
        .err()
        .unwrap();
    assert_eq!(e.kind, GeometryErrorKind::Cancelled);
    assert_eq!(calls.get(), 2);
    near(target.volume_mm3().unwrap(), 1000.);
}
#[test]
fn disconnected_inputs_are_not_accepted_as_one_semantic_body() {
    let target = cube([10.; 3], [0.; 3]);
    let far = cube([2.; 3], [20.; 3]);
    let compound = target
        .boolean(&far, BooleanOperation::Union, p())
        .unwrap()
        .unwrap();
    assert_eq!(compound.solid_count().unwrap(), 2);
    assert_eq!(
        compound.split_by_solid(&target, p()).err().unwrap().kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        target.split_by_solid(&compound, p()).err().unwrap().kind,
        GeometryErrorKind::Input
    );
}

#[test]
fn small_positive_partitions_are_not_discarded_by_an_absolute_volume_threshold() {
    let edge = GeometryPolicy::default().minimum_feature_mm;
    let target = cube([edge; 3], [0.; 3]);
    let tool = cube([edge; 3], [edge / 2., 0., 0.]);
    let split = target.split_by_solid(&tool, p()).unwrap();
    let expected = edge.powi(3) / 2.;
    assert!(expected < 1e-12);
    for solid in [&split.inside, &split.outside] {
        let volume = solid.volume_mm3().unwrap();
        assert!(volume > 0. && volume.is_finite());
        assert!(
            (volume / expected - 1.).abs() < 1e-6,
            "{volume} != {expected}"
        );
        assert_eq!(solid.solid_count().unwrap(), 1);
    }
}

#[test]
fn accepted_near_tolerance_partitions_do_not_overlap_or_silently_change_membership() {
    let target = cube([10.; 3], [0.; 3]);
    let mut accepted = 0;
    for width in [1e-4, 1e-5, 1e-6, 5e-7, 2e-7, 1e-7, 5e-8, 1e-8] {
        let tool = cube([12.; 3], [10. - width, -1., -1.]);
        if let Ok(split) = target.split_by_solid(&tool, p()) {
            accepted += 1;
            let inside = split.inside.volume_mm3().unwrap();
            assert!(
                (inside / (100. * width) - 1.).abs() < 1e-4,
                "Accepted inside width {width}: volume {inside}"
            );
            assert!(
                split
                    .inside
                    .boolean(&split.outside, BooleanOperation::Intersect, p())
                    .unwrap()
                    .is_none(),
                "Accepted overlapping partitions at width {width}"
            );
        }
    }
    assert!(
        accepted > 0,
        "The test must exercise accepted native partitions"
    );
}
