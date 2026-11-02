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

#[test]
fn authored_planes_conserve_volume_and_keep_positive_negative_sides_when_sizes_reverse() {
    let target = cube([10.; 3], [20., 30., 40.]);
    let bounds = target.bounds().unwrap();
    for z in [44., 46.] {
        let plane = PlaneFrame {
            origin: [20., 30., z],
            ..Default::default()
        };
        let pair = target.split_by_plane(plane, p()).unwrap();
        near(pair.positive.volume_mm3().unwrap(), (50. - z) * 100.);
        near(pair.negative.volume_mm3().unwrap(), (z - 40.) * 100.);
        near(pair.positive.bounds().unwrap().min[2], z);
        near(pair.negative.bounds().unwrap().max[2], z);
        assert!(
            pair.positive
                .boolean(&pair.negative, BooleanOperation::Intersect, p())
                .unwrap()
                .is_none()
        );
        near(
            pair.positive
                .boolean(&pair.negative, BooleanOperation::Union, p())
                .unwrap()
                .unwrap()
                .volume_mm3()
                .unwrap(),
            1000.,
        );
    }
    assert_eq!(target.bounds().unwrap(), bounds);
    near(target.volume_mm3().unwrap(), 1000.);
    let h = 0.5_f64.sqrt();
    let diagonal = PlaneFrame {
        origin: [25., 35., 40.],
        u: [h, -h, 0.],
        v: [0., 0., -1.],
    };
    let pair = target.split_by_plane(diagonal, p()).unwrap();
    near(pair.positive.volume_mm3().unwrap(), 500.);
    near(pair.negative.volume_mm3().unwrap(), 500.);
}
#[test]
fn plane_split_refuses_missing_tangent_invalid_and_disconnected_partitions_without_mutating_input()
{
    let target = cube([10.; 3], [0.; 3]);
    for z in [-1., 0., 10., 11.] {
        assert!(
            target
                .split_by_plane(
                    PlaneFrame {
                        origin: [0., 0., z],
                        ..Default::default()
                    },
                    p()
                )
                .is_err()
        );
    }
    assert!(
        target
            .split_by_plane(
                PlaneFrame {
                    u: [2., 0., 0.],
                    ..Default::default()
                },
                p()
            )
            .is_err()
    );
    assert!(
        target
            .split_by_plane(
                PlaneFrame {
                    origin: [f64::NAN, 0., 0.],
                    ..Default::default()
                },
                p()
            )
            .is_err()
    );
    let slot = cube([4., 12., 10.], [3., -1., 1.]);
    let connected = target
        .boolean(&slot, BooleanOperation::Subtract, p())
        .unwrap()
        .unwrap();
    assert_eq!(connected.solid_count().unwrap(), 1);
    let before = connected.volume_mm3().unwrap();
    let error = connected
        .split_by_plane(
            PlaneFrame {
                origin: [0., 0., 5.],
                ..Default::default()
            },
            p(),
        )
        .err()
        .unwrap();
    assert!(error.message.contains("disconnected"), "{error}");
    near(connected.volume_mm3().unwrap(), before);
    near(target.volume_mm3().unwrap(), 1000.);
}
#[test]
fn plane_split_cancellation_discards_both_native_outputs() {
    let target = cube([10.; 3], [0.; 3]);
    let checkpoints = Cell::new(0);
    let stop = || {
        let count = checkpoints.get() + 1;
        checkpoints.set(count);
        count >= 2
    };
    let error = target
        .split_by_plane_interruptible(
            PlaneFrame {
                origin: [0., 0., 5.],
                ..Default::default()
            },
            p(),
            Some(&stop),
        )
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Cancelled);
    near(target.volume_mm3().unwrap(), 1000.);
}
#[test]
fn named_plane_boundaries_have_semantic_cap_origins_and_preserve_exact_target_ancestry() {
    use multicad_core::{
        document::EntityId,
        references::{Origin, TopologyKind},
    };
    let source = EntityId::new();
    let feature = EntityId::new();
    let target = Solid::box_named([10.; 3], source, Default::default()).unwrap();
    for z in [4., 6.] {
        let split = target
            .split_by_plane_named(
                PlaneFrame {
                    origin: [0., 0., z],
                    ..Default::default()
                },
                feature,
                p(),
                None,
            )
            .unwrap();
        for (solid, role) in [
            (&split.positive, "split/plane/positive"),
            (&split.negative, "split/plane/negative"),
        ] {
            let snapshot = solid.topology_snapshot(feature, 3).unwrap();
            let cap = Origin {
                source: feature,
                role: role.into(),
            };
            let faces: Vec<_> = snapshot
                .elements
                .iter()
                .filter(|element| element.key.kind == TopologyKind::Face)
                .collect();
            assert_eq!(
                faces
                    .iter()
                    .filter(|element| element.origins.contains(&cap))
                    .count(),
                1
            );
            assert!(
                faces
                    .iter()
                    .any(|element| element.origins.iter().any(|origin| origin.source == source))
            );
            assert!(
                faces
                    .iter()
                    .filter(|element| element.origins.contains(&cap))
                    .all(|element| element.frames.iter().any(|frame| frame.origin == cap))
            );
        }
    }
}
