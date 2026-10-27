use multicad_core::{
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::EntityId,
    project_file::ContentHash,
    sketch_trim::*,
};
use std::{collections::BTreeSet, f64::consts::PI};
fn source() -> Source {
    Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    }
}
fn line(s: &mut Source, a: [f64; 2], b: [f64; 2]) -> EntityId {
    let a = s.add_point(a);
    let b = s.add_point(b);
    s.add_line(a, b)
}
fn circle(s: &mut Source, c: [f64; 2], r: f64) -> EntityId {
    let center = s.add_point(c);
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: true,
        kind: CurveKind::Circle {
            center,
            radius_mm: r,
        },
    });
    id
}
fn arc(s: &mut Source, a: [f64; 2], b: [f64; 2], clockwise: bool) -> EntityId {
    let center = s.add_point([0., 0.]);
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
fn trim(
    target: EntityId,
    pick_mm: [f64; 2],
    boundaries: impl IntoIterator<Item = EntityId>,
) -> TrimRequest {
    TrimRequest::TrimCurve(TrimCurve {
        target,
        pick_mm,
        boundaries: boundaries.into_iter().collect(),
    })
}
fn extend(target: EntityId, endpoint: Endpoint, boundary: EntityId) -> TrimRequest {
    TrimRequest::ExtendArc(ExtendArc {
        target,
        endpoint,
        boundaries: BTreeSet::from([boundary]),
    })
}
fn run(s: &Source, r: &TrimRequest) -> Result<TrimDraft, TrimError> {
    let stamp = GeometryStamp {
        document: EntityId::new(),
        sketch: EntityId::new(),
        revision: 9,
        document_hash: ContentHash::of(b"current source"),
    };
    draft(
        s,
        stamp,
        DerivedGeometry {
            stamp,
            canonical_hash: canonical_hash(s).unwrap(),
            solved: Some(&s.sketch),
        },
        r,
        TrimPolicy::default(),
    )
}
fn endpoints(s: &Source, id: EntityId) -> ([[f64; 2]; 2], bool) {
    let c = s.sketch.curves.iter().find(|c| c.id == id).unwrap();
    let CurveKind::Arc {
        start,
        end,
        clockwise,
        ..
    } = c.kind
    else {
        panic!("arc")
    };
    (
        [start, end].map(|id| {
            s.sketch
                .points
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .position_mm
        }),
        clockwise,
    )
}
fn near(a: [f64; 2], b: [f64; 2]) {
    assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7, "{a:?} != {b:?}");
}
fn reject(s: &Source, r: &TrimRequest, kind: TrimErrorKind) -> TrimError {
    let original = s.clone();
    let error = run(s, r).unwrap_err();
    assert_eq!(error.kind, kind, "{}", error.message);
    assert_eq!(s, &original);
    error
}
#[test]
fn circle_to_arc_is_explicit_preserves_radius_constraints_and_keeps_target_identity() {
    let mut s = source();
    let target = circle(&mut s, [0., 0.], 5.);
    let cut = line(&mut s, [0., -10.], [0., 10.]);
    let radius = s.add_constraint(ConstraintKind::Radius {
        curve: target,
        dimension: Dimension::Literal {
            value: 5.,
            unit: "mm".into(),
        },
    });
    let diameter = s.add_constraint(ConstraintKind::Diameter {
        curve: target,
        dimension: Dimension::Literal {
            value: 10.,
            unit: "mm".into(),
        },
    });
    s.sketch.constraints.last_mut().unwrap().reference = true;
    let original = s.clone();
    let d = run(&s, &trim(target, [5., 0.], [cut])).unwrap();
    assert_eq!(s, original);
    assert_eq!(d.report.replacements[&target], vec![target]);
    assert_eq!(d.report.type_changes[&target], CurveTypeChange::CircleToArc);
    assert_eq!(d.report.parameterization, Parameterization::ArcRadians);
    assert_eq!(d.report.created_points.len(), 2);
    assert!(d.report.created_curves.is_empty());
    assert!(d.report.removed_curves.is_empty());
    assert!(d.source.sketch.curves[0].construction);
    assert_eq!(d.source.sketch.constraints, s.sketch.constraints);
    assert_eq!(
        d.report.preserved_constraints,
        BTreeSet::from([radius, diameter])
    );
    let (p, cw) = endpoints(&d.source, target);
    near(p[0], [0., 5.]);
    near(p[1], [0., -5.]);
    assert_eq!(p[0][0], 0.);
    assert_eq!(p[1][0], 0.);
    assert!(!cw);
    let Action::TrimArc {
        removed_sweep_rad, ..
    } = d.report.action
    else {
        panic!()
    };
    assert!((removed_sweep_rad - PI).abs() < 1e-10);
}
#[test]
fn arc_middle_trim_reports_two_fresh_fragments_and_preserves_all_old_points() {
    let mut s = source();
    let target = arc(&mut s, [5., 0.], [-5., 0.], false);
    let a = line(&mut s, [3., -10.], [3., 10.]);
    let b = line(&mut s, [-3., -10.], [-3., 10.]);
    let original = s.clone();
    let d = run(&s, &trim(target, [0., 5.], [a, b])).unwrap();
    assert_eq!(s, original);
    assert_eq!(
        &d.source.sketch.points[..s.sketch.points.len()],
        &s.sketch.points
    );
    assert_eq!(d.report.created_curves.len(), 2);
    assert_eq!(d.report.removed_curves, BTreeSet::from([target]));
    assert!(!d.report.replacements[&target].contains(&target));
    let (first, _) = endpoints(&d.source, d.report.replacements[&target][0]);
    let (last, _) = endpoints(&d.source, d.report.replacements[&target][1]);
    near(first[0], [5., 0.]);
    near(first[1], [3., 4.]);
    near(last[0], [-3., 4.]);
    near(last[1], [-5., 0.]);
    assert_eq!(d.source.sketch.curves[2..], s.sketch.curves[1..]);
    s.add_constraint(ConstraintKind::Radius {
        curve: target,
        dimension: Dimension::Literal {
            value: 5.,
            unit: "mm".into(),
        },
    });
    reject(
        &s,
        &trim(target, [0., 5.], [a, b]),
        TrimErrorKind::ConstraintConflict,
    );
}
#[test]
fn clockwise_arc_trim_respects_direction_and_detaches_no_neighbor_geometry() {
    let mut s = source();
    let target = arc(&mut s, [0., 5.], [5., 0.], true);
    let original_end = s.sketch.points[2].id;
    let other = s.add_point([8., 0.]);
    let neighbor = s.add_line(original_end, other);
    let cut = line(&mut s, [3., -10.], [3., 10.]);
    let original = s.clone();
    let q = 5. * std::f64::consts::FRAC_1_SQRT_2;
    let d = run(&s, &trim(target, [q, q], [cut])).unwrap();
    let (p, cw) = endpoints(&d.source, target);
    near(p[0], [0., 5.]);
    near(p[1], [3., 4.]);
    assert!(cw);
    assert_eq!(
        d.report.detached_points[&original_end],
        BTreeSet::from([neighbor])
    );
    assert_eq!(d.source.sketch.curves[1..], original.sketch.curves[1..]);
    s.add_constraint(ConstraintKind::Fix {
        point: original_end,
    });
    reject(
        &s,
        &trim(target, [q, q], [cut]),
        TrimErrorKind::ConstraintConflict,
    );
}
#[test]
fn arc_extension_selects_nearest_actual_contact_in_both_endpoint_directions() {
    let mut s = source();
    let target = arc(&mut s, [5., 0.], [0., 5.], false);
    let end = line(&mut s, [-3., -10.], [-3., 10.]);
    let start = line(&mut s, [-10., -3.], [10., -3.]);
    let d = run(&s, &extend(target, Endpoint::End, end)).unwrap();
    let (p, cw) = endpoints(&d.source, target);
    near(p[0], [5., 0.]);
    near(p[1], [-3., 4.]);
    assert!(!cw);
    let d = run(&s, &extend(target, Endpoint::Start, start)).unwrap();
    let (p, _) = endpoints(&d.source, target);
    near(p[0], [4., -3.]);
    near(p[1], [0., 5.]);
    assert_eq!(d.report.replacements[&target], vec![target]);
    let mut cw = source();
    let target = arc(&mut cw, [5., 0.], [0., -5.], true);
    let boundary = line(&mut cw, [-3., -10.], [-3., 10.]);
    let d = run(&cw, &extend(target, Endpoint::End, boundary)).unwrap();
    let (p, direction) = endpoints(&d.source, target);
    near(p[1], [-3., -4.]);
    assert!(direction);
    let mut closed = source();
    let target = circle(&mut closed, [0., 0.], 5.);
    let cut = line(&mut closed, [0., -10.], [0., 10.]);
    reject(
        &closed,
        &extend(target, Endpoint::End, cut),
        TrimErrorKind::UnsupportedTarget,
    );
}
#[test]
fn circle_trim_wraps_across_its_seam_without_creating_a_full_turn_arc() {
    let mut s = source();
    let target = circle(&mut s, [0., 0.], 5.);
    let cut = line(&mut s, [-10., 3.], [10., 3.]);
    let d = run(&s, &trim(target, [0., -5.], [cut])).unwrap();
    let (p, _) = endpoints(&d.source, target);
    near(p[0], [4., 3.]);
    near(p[1], [-4., 3.]);
    let Action::TrimArc {
        removed_sweep_rad, ..
    } = d.report.action
    else {
        panic!()
    };
    assert!(removed_sweep_rad > PI && removed_sweep_rad < 2. * PI);
}
#[test]
fn finite_arc_circle_and_line_boundaries_are_respected_and_ambiguous_geometry_rejects() {
    let mut s = source();
    let target = circle(&mut s, [0., 0.], 5.);
    let short = line(&mut s, [0., -2.], [0., 2.]);
    reject(
        &s,
        &trim(target, [5., 0.], [short]),
        TrimErrorKind::NoIntersection,
    );
    let tangent = line(&mut s, [5., -10.], [5., 10.]);
    reject(
        &s,
        &trim(target, [0., 5.], [tangent]),
        TrimErrorKind::AmbiguousIntersection,
    );
    let same = circle(&mut s, [0., 0.], 5.);
    reject(&s, &trim(target, [0., 5.], [same]), TrimErrorKind::Overlap);
    let a = line(&mut s, [0., -10.], [0., 10.]);
    let b = line(&mut s, [0., -20.], [0., 20.]);
    reject(
        &s,
        &trim(target, [5., 0.], [a, b]),
        TrimErrorKind::AmbiguousIntersection,
    );
    reject(&s, &trim(target, [0., 5.], [a]), TrimErrorKind::InvalidPick);
    reject(&s, &trim(target, [6., 0.], [a]), TrimErrorKind::InvalidPick);
    let mut s = source();
    let target = arc(&mut s, [5., 0.], [-5., 0.], false);
    let boundary = circle(&mut s, [5., 0.], 5.);
    let d = run(&s, &trim(target, [0., 5.], [boundary])).unwrap();
    let (p, _) = endpoints(&d.source, target);
    near(p[0], [5., 0.]);
    near(p[1], [2.5, (18.75_f64).sqrt()]);
    let mut s = source();
    let target = circle(&mut s, [0., 0.], 5.);
    let center = s.add_point([5., 0.]);
    let start = s.add_point([10., 0.]);
    let end = s.add_point([0., 0.]);
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: false,
        kind: CurveKind::Arc {
            center,
            start,
            end,
            clockwise: false,
        },
    });
    reject(
        &s,
        &trim(target, [0., -5.], [id]),
        TrimErrorKind::NoIntersection,
    );
}
#[test]
fn compatibility_line_requests_retain_restrictions_and_generic_line_trim_remains_valid() {
    let mut s = source();
    let target = arc(&mut s, [5., 0.], [0., 5.], false);
    let cut = line(&mut s, [3., -10.], [3., 10.]);
    reject(
        &s,
        &TrimRequest::Trim(TrimLine {
            target,
            pick_mm: [0., 5.],
            boundaries: BTreeSet::from([cut]),
        }),
        TrimErrorKind::UnsupportedTarget,
    );
    let target = line(&mut s, [0., 20.], [10., 20.]);
    let cut = line(&mut s, [4., 10.], [4., 30.]);
    let d = run(&s, &trim(target, [8., 20.], [cut])).unwrap();
    assert_eq!(d.report.parameterization, Parameterization::LineFraction);
    assert!(d.report.type_changes.is_empty());
    assert!(matches!(d.report.action, Action::Trim { .. }));
}

#[test]
fn exact_contact_points_allow_followup_line_trim_to_form_a_closed_semidisk() {
    let mut s = source();
    let target = circle(&mut s, [0., 0.], 5.);
    s.sketch.curves[0].construction = false;
    let cut = line(&mut s, [0., -10.], [0., 10.]);
    let first = run(&s, &trim(target, [5., 0.], [cut])).unwrap();
    let second = run(&first.source, &trim(cut, [0., -8.], [target])).unwrap();
    let third = run(&second.source, &trim(cut, [0., 8.], [target])).unwrap();
    let regions = multicad_core::sketch_regions::extract(&third.source);
    assert_eq!(regions.regions.len(), 1, "{:?}", regions.diagnostics);
    assert!((regions.regions[0].area_mm2 - 12.5 * PI).abs() < 1e-7);
    assert_eq!(
        regions.regions[0].outer.intent.curves,
        BTreeSet::from([target, cut])
    );
}
