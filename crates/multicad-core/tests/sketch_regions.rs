use multicad_core::{
    constraint_sketch::{Curve, CurveKind, Sketch, source::Source},
    document::EntityId,
    sketch_regions::{self, Problem},
};
use std::f64::consts::PI;
fn source() -> Source {
    Source {
        u: [1., 0., 0.],
        sketch: Sketch::default(),
    }
}
fn valid(s: &Source) -> Result<Vec<sketch_regions::Region>, Vec<sketch_regions::Diagnostic>> {
    let r = sketch_regions::extract(s);
    if r.diagnostics.is_empty() {
        Ok(r.regions)
    } else {
        Err(r.diagnostics)
    }
}
fn line(s: &mut Source, a: [f64; 2], b: [f64; 2]) -> EntityId {
    let a = s.add_point(a);
    let b = s.add_point(b);
    s.add_line(a, b)
}
fn circle(s: &mut Source, p: [f64; 2], radius_mm: f64) -> EntityId {
    let center = s.add_point(p);
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: false,
        kind: CurveKind::Circle { center, radius_mm },
    });
    id
}
fn arc(s: &mut Source, c: [f64; 2], a: [f64; 2], b: [f64; 2], clockwise: bool) -> EntityId {
    let center = s.add_point(c);
    let start = s.add_point(a);
    let end = s.add_point(b);
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: false,
        kind: CurveKind::Arc {
            center,
            start,
            end,
            clockwise,
        },
    });
    id
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn has(s: &Source, problem: Problem) {
    let ds = valid(s).unwrap_err();
    assert!(ds.iter().any(|d| d.problem == problem), "{ds:?}");
}

#[test]
fn disjoint_nested_holes_and_islands_are_exact_and_source_neutral() {
    let mut s = source();
    s.add_rectangle([0., 0.], [100., 80.]).unwrap();
    let hole = circle(&mut s, [50., 40.], 20.);
    let island = circle(&mut s, [50., 40.], 5.);
    s.add_rectangle([150., 0.], [170., 20.]).unwrap();
    let original = s.clone();
    let regions = valid(&s).unwrap();
    assert_eq!(s, original);
    assert_eq!(regions.len(), 4);
    let outer = regions
        .iter()
        .find(|r| r.outer.edges.len() == 4 && !r.holes.is_empty())
        .unwrap();
    assert_eq!(
        outer.holes[0]
            .intent
            .curves
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![hole]
    );
    near(outer.area_mm2, 8000. - PI * 400.);
    near(
        regions
            .iter()
            .find(|r| r.outer.intent.curves.contains(&island))
            .unwrap()
            .area_mm2,
        PI * 25.,
    );
    near(
        regions
            .iter()
            .find(|r| r.outer.intent.curves.contains(&hole))
            .unwrap()
            .area_mm2,
        PI * 375.,
    );
    near(regions.iter().map(|r| r.area_mm2).sum(), 8400.);
}
#[test]
fn mixed_arc_line_and_complementary_arcs_make_exact_boundaries() {
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [-10., 0.], false);
    line(&mut s, [-10., 0.], [10., 0.]);
    let region = valid(&s).unwrap().remove(0);
    near(region.area_mm2, 50. * PI);
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [-10., 0.], false);
    arc(&mut s, [0., 0.], [-10., 0.], [10., 0.], false);
    near(valid(&s).unwrap()[0].area_mm2, 100. * PI);
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [0., -10.], false);
    line(&mut s, [0., -10.], [10., 0.]);
    near(valid(&s).unwrap()[0].area_mm2, 75. * PI + 50.);
}
#[test]
fn containment_at_arc_line_junction_uses_a_consistent_ray_rule() {
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [-10., 0.], false);
    line(&mut s, [-10., 0.], [10., 0.]);
    circle(&mut s, [0., 4.], 1.);
    let rs = valid(&s).unwrap();
    assert_eq!(rs.len(), 2);
    near(
        rs.iter().find(|r| r.holes.len() == 1).unwrap().area_mm2,
        49. * PI,
    );
    // A separate shape whose representative endpoint is exactly at the cap's
    // arc/line y level must remain outside rather than become a false hole.
    s.add_rectangle([20., 0.], [22., 2.]).unwrap();
    assert_eq!(valid(&s).unwrap().len(), 3);
}
#[test]
fn boundary_intent_survives_source_order_direction_and_dimension_changes() {
    let mut s = source();
    s.add_rectangle([0., 0.], [20., 10.]).unwrap();
    let before = valid(&s).unwrap()[0].outer.intent.clone();
    s.sketch.curves.reverse();
    for c in &mut s.sketch.curves {
        if let CurveKind::Line { start, end } = &mut c.kind {
            std::mem::swap(start, end);
        }
    }
    for p in &mut s.sketch.points {
        p.position_mm[0] *= 2.;
    }
    let region = valid(&s).unwrap().remove(0);
    assert_eq!(region.outer.intent, before);
    near(region.area_mm2, 400.);
    let encoded = serde_json::to_vec(&before).unwrap();
    assert_eq!(
        serde_json::from_slice::<sketch_regions::BoundaryIntent>(&encoded).unwrap(),
        before
    );
}
#[test]
fn area_does_not_cancel_at_large_world_coordinates() {
    let mut s = source();
    s.add_rectangle([999_999_990., 999_999_990.], [999_999_991., 999_999_992.])
        .unwrap();
    near(valid(&s).unwrap()[0].area_mm2, 2.);
}
#[test]
fn endpoint_tolerance_is_explicit_and_does_not_modify_authored_points() {
    let mut s = source();
    line(&mut s, [0., 0.], [10., 0.]);
    line(&mut s, [10. + 5e-8, 0.], [0., 10.]);
    line(&mut s, [0., 10.], [0., 0.]);
    let before = s.clone();
    assert_eq!(valid(&s).unwrap().len(), 1);
    assert_eq!(s, before);
    assert!(
        sketch_regions::extract_with_tolerance(&s, 1e-9)
            .diagnostics
            .iter()
            .any(|d| d.problem == Problem::OpenBoundary)
    );
}
#[test]
fn construction_geometry_cannot_close_or_split_a_manufacturing_profile() {
    let mut s = source();
    s.add_rectangle([0., 0.], [10., 10.]).unwrap();
    line(&mut s, [-1., 5.], [11., 5.]);
    s.sketch.curves.last_mut().unwrap().construction = true;
    near(valid(&s).unwrap()[0].area_mm2, 100.);
    s.sketch.curves[0].construction = true;
    has(&s, Problem::OpenBoundary);
}
#[test]
fn open_ends_and_branch_junctions_identify_authored_curves() {
    let mut s = source();
    let id = line(&mut s, [0., 0.], [10., 0.]);
    let ds = valid(&s).unwrap_err();
    assert_eq!(ds.len(), 2);
    assert!(ds.iter().all(|d| d.curves == vec![id] && d.at_mm.is_some()));
    s.add_rectangle([0., 0.], [10., 10.]).unwrap();
    has(&s, Problem::AmbiguousJunction);
}
#[test]
fn crossing_and_touching_loops_require_explicit_authoring() {
    let mut s = source();
    for (a, b) in [
        ([0., 0.], [10., 10.]),
        ([10., 10.], [0., 10.]),
        ([0., 10.], [10., 0.]),
        ([10., 0.], [0., 0.]),
    ] {
        line(&mut s, a, b);
    }
    has(&s, Problem::Intersection);
    let mut s = source();
    circle(&mut s, [0., 0.], 10.);
    circle(&mut s, [20., 0.], 10.);
    has(&s, Problem::Intersection);
    let mut s = source();
    circle(&mut s, [0., 0.], 10.);
    circle(&mut s, [5., 0.], 7.);
    has(&s, Problem::Intersection);
}
#[test]
fn coincident_curves_and_invalid_arc_radii_are_not_silently_healed() {
    let mut s = source();
    circle(&mut s, [0., 0.], 10.);
    circle(&mut s, [0., 0.], 10.);
    has(&s, Problem::Overlap);
    let mut s = source();
    line(&mut s, [0., 0.], [10., 0.]);
    line(&mut s, [5., 0.], [15., 0.]);
    has(&s, Problem::Overlap);
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [0., 11.], false);
    has(&s, Problem::DegenerateCurve);
    let mut s = source();
    line(&mut s, [0., 0.], [0., 0.]);
    has(&s, Problem::DegenerateCurve);
}
#[test]
fn arc_intersections_use_the_actual_sweep_not_the_whole_support_circle() {
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [-10., 0.], false);
    line(&mut s, [-10., 0.], [10., 0.]);
    s.add_rectangle([-2., -8.], [2., -4.]).unwrap();
    assert_eq!(valid(&s).unwrap().len(), 2);
    let mut s = source();
    arc(&mut s, [0., 0.], [10., 0.], [-10., 0.], false);
    line(&mut s, [-10., 0.], [10., 0.]);
    s.add_rectangle([-2., 8.], [2., 12.]).unwrap();
    has(&s, Problem::Intersection);
}
#[test]
fn empty_source_and_invalid_policy_have_explicit_results() {
    assert!(valid(&source()).unwrap().is_empty());
    assert!(
        !sketch_regions::extract_with_tolerance(&source(), f64::NAN)
            .diagnostics
            .is_empty()
    );
}

#[test]
fn shallow_angle_crossing_is_not_mistaken_for_parallel_geometry() {
    let mut s = source();
    for (a, b) in [
        ([0., 0.], [1e9, 1e-4]),
        ([1e9, 1e-4], [1e9, -1e-4]),
        ([1e9, -1e-4], [0., 3e-4]),
        ([0., 3e-4], [0., 0.]),
    ] {
        line(&mut s, a, b);
    }
    has(&s, Problem::Intersection);
}

#[test]
fn selected_region_pins_holes_and_never_falls_back_to_a_different_area() {
    let mut s = source();
    s.add_rectangle([0., 0.], [100., 60.]).unwrap();
    let intent = sketch_regions::extract(&s).regions[0].intent();
    line(&mut s, [200., 0.], [210., 10.]);
    assert!(sketch_regions::extract(&s).resolve(&intent).is_some());
    circle(&mut s, [50., 30.], 5.);
    let result = sketch_regions::extract(&s);
    assert_eq!(result.regions.len(), 2);
    assert!(result.resolve(&intent).is_none());
    let annulus = result
        .regions
        .iter()
        .find(|r| !r.holes.is_empty())
        .unwrap()
        .intent();
    assert_ne!(annulus, intent);
    let encoded = serde_json::to_vec(&annulus).unwrap();
    assert!(
        result
            .resolve(&serde_json::from_slice(&encoded).unwrap())
            .is_some()
    );
}

#[test]
fn unrelated_open_and_invalid_geometry_preserves_closed_region_intent() {
    let mut s = source();
    s.add_rectangle([0., 0.], [20., 10.]).unwrap();
    let before = sketch_regions::extract(&s).regions[0].outer.intent.clone();
    let open = line(&mut s, [100., 0.], [110., 0.]);
    let bad = arc(&mut s, [150., 0.], [160., 0.], [150., 11.], false);
    let extraction = sketch_regions::extract(&s);
    assert_eq!(extraction.regions.len(), 1);
    assert_eq!(extraction.regions[0].outer.intent, before);
    assert!(
        extraction
            .diagnostics
            .iter()
            .any(|d| d.problem == Problem::OpenBoundary && d.curves.contains(&open))
    );
    assert!(
        extraction
            .diagnostics
            .iter()
            .any(|d| d.problem == Problem::DegenerateCurve && d.curves.contains(&bad))
    );
    s.sketch.points.iter_mut().find(|p|matches!(s.sketch.curves.iter().find(|c|c.id==open).unwrap().kind,CurveKind::Line {end,..} if end==p.id)).unwrap().position_mm[1]=25.;
    assert_eq!(sketch_regions::extract(&s).regions[0].outer.intent, before);
}
#[test]
fn a_crossing_stray_line_rejects_its_whole_loop_but_not_a_disjoint_profile() {
    let mut s = source();
    s.add_rectangle([0., 0.], [20., 10.]).unwrap();
    let safe = circle(&mut s, [100., 50.], 5.);
    line(&mut s, [-5., 5.], [25., 5.]);
    let r = sketch_regions::extract(&s);
    assert_eq!(r.regions.len(), 1);
    assert!(r.regions[0].outer.intent.curves.contains(&safe));
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.problem == Problem::Intersection)
    );
}
