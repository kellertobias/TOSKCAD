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
