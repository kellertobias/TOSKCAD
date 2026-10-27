use multicad_core::{
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, Sketch, source::Source},
    document::EntityId,
    project_file::ContentHash,
    sketch_trim::*,
};
use std::collections::BTreeSet;
fn source() -> Source {
    Source {
        u: [1., 0., 0.],
        sketch: Sketch::default(),
    }
}
fn line(s: &mut Source, a: [f64; 2], b: [f64; 2]) -> EntityId {
    let a = s.add_point(a);
    let b = s.add_point(b);
    s.add_line(a, b)
}
fn boundary(s: &mut Source, x: f64) -> EntityId {
    line(s, [x, -20.], [x, 20.])
}
fn round(s: &mut Source, center: [f64; 2], radius: f64) -> EntityId {
    let center = s.add_point(center);
    let id = EntityId::new();
    s.sketch.curves.push(Curve {
        id,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: radius,
        },
    });
    id
}
fn stamp() -> GeometryStamp {
    GeometryStamp {
        document: EntityId::new(),
        sketch: EntityId::new(),
        revision: 7,
        document_hash: ContentHash::of(b"fixture source"),
    }
}
fn trim(
    target: EntityId,
    pick: [f64; 2],
    boundaries: impl IntoIterator<Item = EntityId>,
) -> TrimRequest {
    TrimRequest::Trim(TrimLine {
        target,
        pick_mm: pick,
        boundaries: boundaries.into_iter().collect(),
    })
}
fn extend(
    target: EntityId,
    endpoint: Endpoint,
    boundaries: impl IntoIterator<Item = EntityId>,
) -> TrimRequest {
    TrimRequest::Extend(ExtendLine {
        target,
        endpoint,
        boundaries: boundaries.into_iter().collect(),
    })
}
fn run(s: &Source, r: &TrimRequest) -> Result<TrimDraft, TrimError> {
    let stamp = stamp();
    draft(
        s,
        stamp,
        DerivedGeometry {
            stamp,
            canonical_hash: canonical_hash(s)?,
            solved: Some(&s.sketch),
        },
        r,
        TrimPolicy::default(),
    )
}
fn positions(s: &Source, id: EntityId) -> [[f64; 2]; 2] {
    let c = s.sketch.curves.iter().find(|c| c.id == id).unwrap();
    let CurveKind::Line { start, end } = c.kind else {
        panic!("line")
    };
    [start, end].map(|id| {
        s.sketch
            .points
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .position_mm
    })
}
fn reject(s: &Source, r: &TrimRequest, kind: TrimErrorKind) -> TrimError {
    let before = s.clone();
    let e = run(s, r).unwrap_err();
    assert_eq!(e.kind, kind, "{}", e.message);
    assert_eq!(*s, before);
    e
}
#[test]
fn trimming_one_end_retains_curve_identity_and_never_moves_shared_neighbors() {
    let mut s = source();
    let a = s.add_point([0., 0.]);
    let b = s.add_point([100., 0.]);
    let target = s.add_line(a, b);
    s.sketch.curves[0].construction = true;
    let other = s.add_point([-20., -20.]);
    let neighbor = s.add_line(a, other);
    let cutter = boundary(&mut s, 40.);
    let original = s.clone();
    let d = run(&s, &trim(target, [20., 0.], [cutter])).unwrap();
    assert_eq!(s, original);
    assert_eq!(positions(&d.source, target), [[40., 0.], [100., 0.]]);
    assert_eq!(d.report.replacements[&target], vec![target]);
    assert_eq!(d.report.created_points.len(), 1);
    assert!(d.report.created_curves.is_empty());
    assert_eq!(d.report.detached_points[&a], BTreeSet::from([neighbor]));
    assert_eq!(
        d.source.sketch.points[..original.sketch.points.len()],
        original.sketch.points
    );
    assert_eq!(d.source.sketch.curves[1..], original.sketch.curves[1..]);
    assert!(d.source.sketch.curves[0].construction);
    let tail = run(&s, &trim(target, [90., 0.], [cutter])).unwrap();
    assert_eq!(positions(&tail.source, target), [[0., 0.], [40., 0.]]);
}
#[test]
fn interior_trim_removes_old_identity_and_explicitly_reports_both_new_fragments() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let left = boundary(&mut s, 30.);
    let right = boundary(&mut s, 70.);
    let before = s.clone();
    let d = run(&s, &trim(target, [50., 0.], [left, right])).unwrap();
    let pieces = &d.report.replacements[&target];
    assert_eq!(pieces.len(), 2);
    assert!(pieces.iter().all(|id| *id != target));
    assert_eq!(d.report.removed_curves, BTreeSet::from([target]));
    assert_eq!(d.report.created_curves, pieces.iter().copied().collect());
    assert_eq!(d.report.created_points.len(), 2);
    assert!(d.report.detached_points.is_empty());
    assert_eq!(positions(&d.source, pieces[0]), [[0., 0.], [30., 0.]]);
    assert_eq!(positions(&d.source, pieces[1]), [[70., 0.], [100., 0.]]);
    assert_eq!(s, before);
    assert_eq!(
        d.report.action,
        Action::Trim {
            removed_mm: [[30., 0.], [70., 0.]]
        }
    );
}
#[test]
fn extend_selects_nearest_actual_forward_boundary_and_does_not_extend_boundary_lines() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let near = boundary(&mut s, 150.);
    let far = boundary(&mut s, 200.);
    let miss = line(&mut s, [120., 10.], [120., 20.]);
    let d = run(&s, &extend(target, Endpoint::End, [near, far, miss])).unwrap();
    assert_eq!(positions(&d.source, target), [[0., 0.], [150., 0.]]);
    let left = boundary(&mut s, -50.);
    let farther = boundary(&mut s, -200.);
    let d = run(&s, &extend(target, Endpoint::Start, [near, left, farther])).unwrap();
    assert_eq!(positions(&d.source, target), [[-50., 0.], [100., 0.]]);
    reject(
        &s,
        &extend(target, Endpoint::End, [miss]),
        TrimErrorKind::NoIntersection,
    );
}
#[test]
fn circular_boundaries_are_analytic_and_arc_sweep_limits_real_intersections() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let circle = round(&mut s, [50., 0.], 20.);
    let d = run(&s, &trim(target, [50., 0.], [circle])).unwrap();
    let ids = &d.report.replacements[&target];
    assert_eq!(positions(&d.source, ids[0]), [[0., 0.], [30., 0.]]);
    assert_eq!(positions(&d.source, ids[1]), [[70., 0.], [100., 0.]]);
    let center = s.add_point([50., 0.]);
    let start = s.add_point([50., 20.]);
    let end = s.add_point([50., -20.]);
    let arc = EntityId::new();
    s.sketch.curves.push(Curve {
        id: arc,
        construction: false,
        kind: CurveKind::Arc {
            center,
            start,
            end,
            clockwise: true,
        },
    });
    let d = run(&s, &trim(target, [20., 0.], [arc])).unwrap();
    assert_eq!(positions(&d.source, target), [[70., 0.], [100., 0.]]);
    if let CurveKind::Arc { clockwise, .. } = &mut s.sketch.curves.last_mut().unwrap().kind {
        *clockwise = false;
    }
    let d = run(&s, &trim(target, [20., 0.], [arc])).unwrap();
    assert_eq!(positions(&d.source, target), [[30., 0.], [100., 0.]]);
    let external = round(&mut s, [150., 0.], 10.);
    let d = run(&s, &extend(target, Endpoint::End, [external])).unwrap();
    assert_eq!(positions(&d.source, target), [[0., 0.], [140., 0.]]);
    reject(
        &s,
        &trim(circle, [50., 20.], [target]),
        TrimErrorKind::UnsupportedTarget,
    );
    reject(
        &s,
        &extend(arc, Endpoint::End, [target]),
        TrimErrorKind::UnsupportedTarget,
    );
}
#[test]
fn tangent_near_duplicate_collinear_and_parallel_cases_never_invent_a_cut() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let tangent = round(&mut s, [50., 20.], 20.);
    reject(
        &s,
        &trim(target, [10., 0.], [tangent]),
        TrimErrorKind::AmbiguousIntersection,
    );
    let first = boundary(&mut s, 40.);
    let close = boundary(&mut s, 40. + 5e-8);
    let same = boundary(&mut s, 40.);
    for other in [close, same] {
        let e = reject(
            &s,
            &trim(target, [10., 0.], [first, other]),
            TrimErrorKind::AmbiguousIntersection,
        );
        assert!(e.involved.contains(&first) && e.involved.contains(&other));
    }
    let collinear = line(&mut s, [20., 0.], [80., 0.]);
    reject(
        &s,
        &trim(target, [10., 0.], [collinear]),
        TrimErrorKind::Overlap,
    );
    let parallel = line(&mut s, [0., 10.], [100., 10.]);
    reject(
        &s,
        &trim(target, [10., 0.], [parallel]),
        TrimErrorKind::NoIntersection,
    );
    let ahead = line(&mut s, [150., 0.], [200., 0.]);
    reject(
        &s,
        &extend(target, Endpoint::End, [ahead]),
        TrimErrorKind::Overlap,
    );
}
#[test]
fn nonfinite_off_curve_cut_endpoint_and_tolerance_sized_spans_are_rejected() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let cut = boundary(&mut s, 40.);
    for pick in [[20., 1.], [101., 0.], [40., 0.], [0., 0.], [f64::NAN, 0.]] {
        reject(&s, &trim(target, pick, [cut]), TrimErrorKind::InvalidPick);
    }
    let tiny = boundary(&mut s, 1e-8);
    reject(
        &s,
        &trim(target, [20., 0.], [tiny]),
        TrimErrorKind::AmbiguousIntersection,
    );
    reject(
        &s,
        &trim(target, [20., 0.], []),
        TrimErrorKind::InvalidRequest,
    );
    reject(
        &s,
        &trim(target, [20., 0.], [target]),
        TrimErrorKind::InvalidRequest,
    );
    reject(
        &s,
        &trim(target, [20., 0.], [EntityId::new()]),
        TrimErrorKind::InvalidRequest,
    );
}
#[test]
fn driving_reference_and_detached_endpoint_constraints_are_rejected_with_ids() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let cut = boundary(&mut s, 40.);
    let CurveKind::Line { start, .. } = s.sketch.curves[0].kind else {
        panic!("line")
    };
    let fixed = s.add_constraint(ConstraintKind::Fix { point: start });
    let e = reject(
        &s,
        &trim(target, [10., 0.], [cut]),
        TrimErrorKind::ConstraintConflict,
    );
    assert!(e.involved.contains(&fixed) && e.involved.contains(&start));
    let d = run(&s, &trim(target, [90., 0.], [cut])).unwrap();
    assert_eq!(d.source.sketch.constraints, s.sketch.constraints);
    assert!(d.report.preserved_constraints.contains(&fixed));
    s.sketch.constraints.clear();
    let length = s.add_constraint(ConstraintKind::Length {
        line: target,
        dimension: Dimension::Literal {
            value: 100.,
            unit: "mm".into(),
        },
    });
    for reference in [false, true] {
        s.sketch.constraints[0].reference = reference;
        let e = reject(
            &s,
            &trim(target, [90., 0.], [cut]),
            TrimErrorKind::ConstraintConflict,
        );
        assert!(e.involved.contains(&length) && e.involved.contains(&target));
    }
}
#[test]
fn derived_context_stamps_and_canonical_identity_are_preconditions_not_a_solver_claim() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let cut = boundary(&mut s, 40.);
    let request = trim(target, [10., 0.], [cut]);
    let expected = stamp();
    let hash = canonical_hash(&s).unwrap();
    let none = draft(
        &s,
        expected,
        DerivedGeometry {
            stamp: expected,
            canonical_hash: hash,
            solved: None,
        },
        &request,
        TrimPolicy::default(),
    )
    .unwrap_err();
    assert_eq!(none.kind, TrimErrorKind::Unsolved);
    for foreign in [
        GeometryStamp {
            revision: 8,
            ..expected
        },
        GeometryStamp {
            document: EntityId::new(),
            ..expected
        },
        GeometryStamp {
            sketch: EntityId::new(),
            ..expected
        },
        GeometryStamp {
            document_hash: ContentHash::of(b"other"),
            ..expected
        },
    ] {
        assert_eq!(
            draft(
                &s,
                expected,
                DerivedGeometry {
                    stamp: foreign,
                    canonical_hash: hash,
                    solved: Some(&s.sketch)
                },
                &request,
                TrimPolicy::default()
            )
            .unwrap_err()
            .kind,
            TrimErrorKind::InvalidContext
        );
    }
    assert_eq!(
        draft(
            &s,
            expected,
            DerivedGeometry {
                stamp: expected,
                canonical_hash: ContentHash::of(b"other"),
                solved: Some(&s.sketch)
            },
            &request,
            TrimPolicy::default()
        )
        .unwrap_err()
        .kind,
        TrimErrorKind::InvalidContext
    );
    let mut solved = s.sketch.clone();
    solved.curves[0].construction = true;
    assert_eq!(
        draft(
            &s,
            expected,
            DerivedGeometry {
                stamp: expected,
                canonical_hash: hash,
                solved: Some(&solved)
            },
            &request,
            TrimPolicy::default()
        )
        .unwrap_err()
        .kind,
        TrimErrorKind::InvalidContext
    );
}
#[test]
fn unrelated_solved_guesses_are_only_seeds_and_budget_failure_changes_nothing() {
    let mut s = source();
    let target = line(&mut s, [0., 0.], [100., 0.]);
    let cut = boundary(&mut s, 40.);
    let unrelated = line(&mut s, [200., 1.], [230., 2.]);
    let mut solved = s.sketch.clone();
    for p in solved.points.iter_mut().skip(4) {
        p.position_mm[1] += 10.;
    }
    let expected = stamp();
    let d = draft(
        &s,
        expected,
        DerivedGeometry {
            stamp: expected,
            canonical_hash: canonical_hash(&s).unwrap(),
            solved: Some(&solved),
        },
        &trim(target, [10., 0.], [cut]),
        TrimPolicy::default(),
    )
    .unwrap();
    assert_eq!(
        d.source.sketch.points[..s.sketch.points.len()],
        s.sketch.points
    );
    let id = s
        .sketch
        .curves
        .iter()
        .find(|c| c.id == unrelated)
        .unwrap()
        .id;
    assert_eq!(id, unrelated);
    assert_eq!(d.seed.points[..solved.points.len()], solved.points);
    while s.sketch.points.len() < 256 {
        s.add_point([500., 500.]);
    }
    reject(&s, &trim(target, [10., 0.], [cut]), TrimErrorKind::Budget);
}
