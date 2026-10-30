//! Sketch fillet and chamfer: solver-checked, one undoable edit, failures
//! never change the document.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{ConstraintKind, CurveKind, Dimension, Sketch, source::Source},
    document::*,
    sketch::Datum,
    sketch_corner::{CornerTreatment, corner_draft, treatable_corners},
};
use multicad_solver::{SolveState, prepare_edit};

fn actor() -> Actor {
    Actor {
        id: "corner".into(),
        channel: Channel::Ui,
    }
}
/// Sketch with a 40 x 20 rectangle; returns the corner at (40, 0).
fn setup() -> (Session, EntityId, EntityId) {
    let (id, edits) = Source::create(Datum::Xy, "Corners").unwrap();
    let document = Document::new("Corners").unwrap().edited(&edits).unwrap();
    let (mut source, _, _) = Source::read(&document, id).unwrap();
    source.add_rectangle([0., 0.], [40., 20.]).unwrap();
    let document = document
        .edited(&source.replace(&document, id).unwrap())
        .unwrap();
    let (source, _, _) = Source::read(&document, id).unwrap();
    let corner = source
        .sketch
        .points
        .iter()
        .find(|p| p.position_mm == [40., 0.])
        .unwrap()
        .id;
    (Session::new(document).unwrap(), id, corner)
}
fn position(sketch: &Sketch, id: EntityId) -> [f64; 2] {
    sketch
        .points
        .iter()
        .find(|p| p.id == id)
        .unwrap()
        .position_mm
}
fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}
fn apply(
    session: &mut Session,
    id: EntityId,
    corner: EntityId,
    treatment: CornerTreatment,
) -> Vec<EntityId> {
    let (source, _, _) = Source::read(session.document(), id).unwrap();
    let draft = corner_draft(&source, corner, treatment).unwrap();
    let edit = prepare_edit(session, session.revision(), actor(), id, draft.source).unwrap();
    assert!(matches!(
        edit.evaluation.diagnostics.state,
        SolveState::Underconstrained | SolveState::FullyConstrained
    ));
    session
        .commit(edit.preview.expect("solvable corner"))
        .unwrap();
    draft.created_curves
}

#[test]
fn fillet_trims_both_lines_to_a_tangent_arc_and_one_undo_restores() {
    let (mut session, id, corner) = setup();
    let before = session.document().clone();
    assert_eq!(
        treatable_corners(&Source::read(&before, id).unwrap().0).len(),
        4
    );
    let created = apply(
        &mut session,
        id,
        corner,
        CornerTreatment::Fillet { radius_mm: 5. },
    );
    let (source, _, _) = Source::read(session.document(), id).unwrap();
    let arc = source
        .sketch
        .curves
        .iter()
        .find(|c| c.id == created[0])
        .unwrap();
    let CurveKind::Arc {
        center, start, end, ..
    } = arc.kind
    else {
        panic!("arc expected")
    };
    assert!(!arc.construction);
    assert!(close(position(&source.sketch, center), [35., 5.]));
    let tangents = [
        position(&source.sketch, start),
        position(&source.sketch, end),
    ];
    assert!(tangents.iter().any(|p| close(*p, [35., 0.])));
    assert!(tangents.iter().any(|p| close(*p, [40., 5.])));
    // The sharp stays as two construction lines; the edges now end at the arc.
    let helpers: Vec<_> = created[1..]
        .iter()
        .map(|id| source.sketch.curves.iter().find(|c| c.id == *id).unwrap())
        .collect();
    assert!(helpers.iter().all(|c| c.construction));
    assert!(source.sketch.curves.iter().filter(|c| !c.construction).all(
        |c| !matches!(c.kind, CurveKind::Line { start, end } if start == corner || end == corner)
    ));
    let tangency = source
        .sketch
        .constraints
        .iter()
        .filter(|c| matches!(c.kind, ConstraintKind::Tangent { .. }))
        .count();
    assert_eq!(tangency, 2);
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn solver_keeps_tangency_when_the_fillet_radius_dimension_changes() {
    let (mut session, id, corner) = setup();
    let created = apply(
        &mut session,
        id,
        corner,
        CornerTreatment::Fillet { radius_mm: 5. },
    );
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    for constraint in &mut source.sketch.constraints {
        if let ConstraintKind::Radius { dimension, .. } = &mut constraint.kind {
            *dimension = Dimension::Literal {
                value: 8.,
                unit: "mm".into(),
            };
        }
    }
    let edit = prepare_edit(&session, session.revision(), actor(), id, source).unwrap();
    let solved = edit.evaluation.candidate.expect("radius change solves");
    let CurveKind::Arc {
        center, start, end, ..
    } = solved
        .curves
        .iter()
        .find(|c| c.id == created[0])
        .unwrap()
        .kind
    else {
        panic!()
    };
    let center = position(&solved, center);
    let radius = |p: [f64; 2]| (p[0] - center[0]).hypot(p[1] - center[1]);
    assert!((radius(position(&solved, start)) - 8.).abs() < 1e-6);
    assert!((radius(position(&solved, end)) - 8.).abs() < 1e-6);
    // Each trimmed edge stays tangent (its line is 8 mm from the center) and
    // collinear with the construction virtual sharp.
    let sharp = position(&solved, corner);
    for tangent_point in [start, end] {
        let edge = solved
            .curves
            .iter()
            .find(|c| {
                !c.construction
                    && matches!(c.kind, CurveKind::Line { start: a, end: b }
                        if a == tangent_point || b == tangent_point)
            })
            .unwrap();
        let CurveKind::Line { start: a, end: b } = edge.kind else {
            unreachable!()
        };
        let (a, b) = (position(&solved, a), position(&solved, b));
        let d = [b[0] - a[0], b[1] - a[1]];
        let len = d[0].hypot(d[1]);
        let off = |p: [f64; 2]| ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]).abs() / len;
        assert!((off(center) - 8.).abs() < 1e-6, "edge not tangent");
        assert!(off(sharp) < 1e-6, "virtual sharp left the edge line");
    }
}

#[test]
fn chamfer_sets_back_both_edges_by_the_distance() {
    let (mut session, id, corner) = setup();
    let before = session.document().clone();
    let created = apply(
        &mut session,
        id,
        corner,
        CornerTreatment::Chamfer { distance_mm: 4. },
    );
    let (source, _, _) = Source::read(session.document(), id).unwrap();
    let CurveKind::Line { start, end } = source
        .sketch
        .curves
        .iter()
        .find(|c| c.id == created[0])
        .unwrap()
        .kind
    else {
        panic!()
    };
    let ends = [
        position(&source.sketch, start),
        position(&source.sketch, end),
    ];
    assert!(ends.iter().any(|p| close(*p, [36., 0.])));
    assert!(ends.iter().any(|p| close(*p, [40., 4.])));
    assert_eq!(
        source
            .sketch
            .constraints
            .iter()
            .filter(|c| matches!(c.kind, ConstraintKind::Length { .. }))
            .count(),
        2
    );
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn invalid_corners_and_sizes_are_rejected_before_any_edit() {
    let (session, id, corner) = setup();
    let before = session.document().clone();
    let (source, _, _) = Source::read(&before, id).unwrap();
    for treatment in [
        CornerTreatment::Fillet { radius_mm: 25. },
        CornerTreatment::Chamfer { distance_mm: 20. },
        CornerTreatment::Fillet { radius_mm: 0. },
        CornerTreatment::Chamfer {
            distance_mm: f64::NAN,
        },
    ] {
        assert!(
            corner_draft(&source, corner, treatment).is_err(),
            "{treatment:?}"
        );
    }
    // A point that is not a two-line corner.
    let mut lone = source.clone();
    let point = lone.add_point([100., 100.]);
    assert!(corner_draft(&lone, point, CornerTreatment::Fillet { radius_mm: 1. }).is_err());
    assert!(
        corner_draft(
            &source,
            EntityId::new(),
            CornerTreatment::Fillet { radius_mm: 1. }
        )
        .is_err()
    );
    // Collinear lines.
    let mut straight = source.clone();
    let a = straight.add_point([0., 50.]);
    let b = straight.add_point([10., 50.]);
    let c = straight.add_point([20., 50.]);
    straight.add_line(a, b);
    straight.add_line(b, c);
    assert!(corner_draft(&straight, b, CornerTreatment::Chamfer { distance_mm: 1. }).is_err());
    assert_eq!(session.document(), &before);
}

// ---- Curved corners (CAD-151) ----

use multicad_core::constraint_sketch::{Axis, Curve};

/// Line and arc specs over point indices.
enum Spec {
    Line(usize, usize),
    /// center, start, end, clockwise
    Arc(usize, usize, usize, bool),
}
/// Builds a sketch from points and curves; returns the session, the sketch id
/// and the point ids in input order.
fn build(points: &[[f64; 2]], curves: &[Spec]) -> (Session, EntityId, Vec<EntityId>) {
    let (id, edits) = Source::create(Datum::Xy, "Curved corners").unwrap();
    let document = Document::new("Curved corners")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut source, _, _) = Source::read(&document, id).unwrap();
    let ids: Vec<_> = points.iter().map(|p| source.add_point(*p)).collect();
    for spec in curves {
        let kind = match *spec {
            Spec::Line(a, b) => CurveKind::Line {
                start: ids[a],
                end: ids[b],
            },
            Spec::Arc(c, a, b, clockwise) => CurveKind::Arc {
                center: ids[c],
                start: ids[a],
                end: ids[b],
                clockwise,
            },
        };
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind,
        });
    }
    let document = document
        .edited(&source.replace(&document, id).unwrap())
        .unwrap();
    (Session::new(document).unwrap(), id, ids)
}
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn curve(sketch: &Sketch, id: EntityId) -> CurveKind {
    sketch
        .curves
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .kind
        .clone()
}
/// The non-construction curve (other than `skip`) that ends at `point`.
fn edge_at(sketch: &Sketch, point: EntityId, skip: EntityId) -> CurveKind {
    sketch
        .curves
        .iter()
        .find(|c| {
            !c.construction
                && c.id != skip
                && match c.kind {
                    CurveKind::Line { start, end } | CurveKind::Arc { start, end, .. } => {
                        start == point || end == point
                    }
                    _ => false,
                }
        })
        .unwrap()
        .kind
        .clone()
}
/// Checks that the fillet arc has radius `r` and is tangent at both ends to
/// the trimmed curve sharing that end: a line lies `r` from the fillet
/// center; for an arc, its center, the shared point and the fillet center
/// are collinear with center distance R + r or |R - r|.
fn assert_tangent_fillet(sketch: &Sketch, fillet: EntityId, r: f64) {
    let CurveKind::Arc {
        center, start, end, ..
    } = curve(sketch, fillet)
    else {
        panic!("fillet is an arc")
    };
    let q = position(sketch, center);
    for end_point in [start, end] {
        let t = position(sketch, end_point);
        assert!((distance(q, t) - r).abs() < 1e-6, "fillet radius");
        match edge_at(sketch, end_point, fillet) {
            CurveKind::Line { start: a, end: b } => {
                let (a, b) = (position(sketch, a), position(sketch, b));
                let d = [b[0] - a[0], b[1] - a[1]];
                let off = ((q[0] - a[0]) * d[1] - (q[1] - a[1]) * d[0]).abs() / d[0].hypot(d[1]);
                assert!((off - r).abs() < 1e-6, "line not tangent: {off}");
            }
            CurveKind::Arc {
                center: c,
                start: s,
                ..
            } => {
                let c = position(sketch, c);
                let big = distance(c, position(sketch, s));
                assert!(
                    (distance(c, t) - big).abs() < 1e-6,
                    "edge arc end left its circle"
                );
                let gap = distance(c, q);
                assert!(
                    (gap - (big + r)).abs() < 1e-6 || (gap - (big - r).abs()).abs() < 1e-6,
                    "arc not tangent: centers {gap}, radii {big} and {r}"
                );
                let cross = (t[0] - c[0]) * (q[1] - c[1]) - (t[1] - c[1]) * (q[0] - c[0]);
                assert!(
                    cross.abs() < 1e-6 * (1. + big * gap),
                    "tangent point not collinear"
                );
            }
            _ => unreachable!(),
        }
    }
}
fn set_dimensions(source: &mut Source, value: f64, pick: impl Fn(&ConstraintKind) -> bool) {
    for constraint in &mut source.sketch.constraints {
        if pick(&constraint.kind)
            && let Some(dimension) = constraint.kind.dimension_mut()
        {
            *dimension = Dimension::Literal {
                value,
                unit: "mm".into(),
            };
        }
    }
}

/// A line along the x axis meeting an arc (center (30,10), radius 10√2) at
/// (40,0) with a 135° turn.
fn line_arc() -> (Session, EntityId, EntityId) {
    let (session, id, ids) = build(
        &[[0., 0.], [40., 0.], [30., 10.], [40., 20.]],
        &[Spec::Line(0, 1), Spec::Arc(2, 1, 3, false)],
    );
    (session, id, ids[1])
}
/// Two quarter arcs meeting at (0,10) at a right angle.
fn arc_arc() -> (Session, EntityId, EntityId) {
    let (session, id, ids) = build(
        &[[0., 0.], [10., 0.], [0., 10.], [10., 10.], [10., 20.]],
        &[Spec::Arc(0, 1, 2, false), Spec::Arc(3, 2, 4, true)],
    );
    (session, id, ids[2])
}

#[test]
fn line_arc_fillet_is_tangent_to_both_and_one_undo_restores() {
    let (mut session, id, corner) = line_arc();
    let before = session.document().clone();
    assert!(treatable_corners(&Source::read(&before, id).unwrap().0).contains(&corner));
    let revision = session.revision();
    let created = apply(
        &mut session,
        id,
        corner,
        CornerTreatment::Fillet { radius_mm: 2. },
    );
    assert_eq!(session.revision(), revision + 1, "one transaction");
    let (source, _, _) = Source::read(session.document(), id).unwrap();
    assert_tangent_fillet(&source.sketch, created[0], 2.);
    // The fillet sits inside the corner: above the line and inside the arc.
    let CurveKind::Arc { center, .. } = curve(&source.sketch, created[0]) else {
        panic!()
    };
    let q = position(&source.sketch, center);
    assert!((q[1] - 2.).abs() < 1e-9);
    assert!((distance(q, [30., 10.]) - (200f64.sqrt() - 2.)).abs() < 1e-9);
    // Virtual sharp: a construction line and a construction arc on the
    // original arc's center reach the old corner.
    let helpers: Vec<_> = created[1..]
        .iter()
        .map(|h| source.sketch.curves.iter().find(|c| c.id == *h).unwrap())
        .collect();
    assert!(helpers.iter().all(|c| c.construction));
    assert!(
        helpers
            .iter()
            .any(|c| matches!(c.kind, CurveKind::Line { end, .. } if end == corner))
    );
    assert!(helpers.iter().any(|c| matches!(
        c.kind,
        CurveKind::Arc { start, clockwise: false, .. } if start == corner
    )));
    assert!(close(position(&source.sketch, corner), [40., 0.]));
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn line_arc_fillet_stays_tangent_when_the_radius_dimension_changes() {
    let (mut session, id, corner) = line_arc();
    let created = apply(
        &mut session,
        id,
        corner,
        CornerTreatment::Fillet { radius_mm: 2. },
    );
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    set_dimensions(&mut source, 3.5, |k| {
        matches!(k, ConstraintKind::Radius { .. })
    });
    let edit = prepare_edit(&session, session.revision(), actor(), id, source).unwrap();
    let solved = edit.evaluation.candidate.expect("radius change solves");
    assert_tangent_fillet(&solved, created[0], 3.5);
    // The virtual sharp stays on the original arc's circle.
    let CurveKind::Arc {
        center, start, end, ..
    } = curve(&solved, created[2])
    else {
        panic!("arc helper")
    };
    assert_eq!(start, corner);
    let c = position(&solved, center);
    assert!(
        (distance(c, position(&solved, corner)) - distance(c, position(&solved, end))).abs() < 1e-6
    );
}

#[test]
fn arc_arc_fillet_is_tangent_and_survives_a_radius_change() {
    let (mut session, id, corner) = arc_arc();
    let before = session.document().clone();
    let created = apply(
        &mut session,
        id,
        corner,
        CornerTreatment::Fillet { radius_mm: 1.5 },
    );
    let (source, _, _) = Source::read(session.document(), id).unwrap();
    assert_tangent_fillet(&source.sketch, created[0], 1.5);
    // The fillet rounds this corner: short trims, and a clockwise arc whose
    // center lies inside the right turn (above and right of the corner).
    let CurveKind::Arc {
        center,
        start,
        end,
        clockwise,
    } = curve(&source.sketch, created[0])
    else {
        panic!()
    };
    assert!(clockwise);
    let q = position(&source.sketch, center);
    assert!(q[0] > 0. && q[1] > 10., "{q:?}");
    for t in [start, end] {
        assert!(distance(position(&source.sketch, t), [0., 10.]) < 3.);
    }
    assert_eq!(
        source
            .sketch
            .constraints
            .iter()
            .filter(|c| matches!(c.kind, ConstraintKind::Tangent { .. }))
            .count(),
        2
    );
    assert!(
        created[1..]
            .iter()
            .all(|h| matches!(curve(&source.sketch, *h), CurveKind::Arc { .. }))
    );
    let mut changed = source.clone();
    set_dimensions(&mut changed, 2.5, |k| {
        matches!(k, ConstraintKind::Radius { .. })
    });
    let edit = prepare_edit(&session, session.revision(), actor(), id, changed).unwrap();
    assert_tangent_fillet(
        &edit.evaluation.candidate.expect("radius change solves"),
        created[0],
        2.5,
    );
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn chamfer_on_a_curved_corner_uses_straight_distance_from_the_corner() {
    for (mut session, id, corner) in [line_arc(), arc_arc()] {
        let before = session.document().clone();
        let created = apply(
            &mut session,
            id,
            corner,
            CornerTreatment::Chamfer { distance_mm: 3. },
        );
        let (source, _, _) = Source::read(session.document(), id).unwrap();
        let CurveKind::Line { start, end } = curve(&source.sketch, created[0]) else {
            panic!("chamfer is a line")
        };
        let sharp = position(&source.sketch, corner);
        for p in [start, end] {
            assert!((distance(position(&source.sketch, p), sharp) - 3.).abs() < 1e-9);
        }
        // Each arc setback is a point distance; each line setback a length.
        let dimensions = source
            .sketch
            .constraints
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    ConstraintKind::Length { .. }
                        | ConstraintKind::Distance {
                            axis: Axis::Direct,
                            ..
                        }
                )
            })
            .count();
        assert_eq!(dimensions, 2);
        // A changed setback re-solves with the chamfer ends on their curves.
        let mut changed = source.clone();
        set_dimensions(&mut changed, 2., |k| {
            matches!(
                k,
                ConstraintKind::Length { .. } | ConstraintKind::Distance { .. }
            )
        });
        let edit = prepare_edit(&session, session.revision(), actor(), id, changed).unwrap();
        let solved = edit.evaluation.candidate.expect("setback change solves");
        let sharp = position(&solved, corner);
        for p in [start, end] {
            let t = position(&solved, p);
            assert!((distance(t, sharp) - 2.).abs() < 1e-6);
            if let CurveKind::Arc {
                center, start: s, ..
            } = edge_at(&solved, p, created[0])
            {
                let c = position(&solved, center);
                assert!((distance(c, t) - distance(c, position(&solved, s))).abs() < 1e-6);
            }
        }
        session.undo(1, actor()).unwrap();
        assert_eq!(session.document(), &before);
    }
}

#[test]
fn unsupported_curved_corners_are_refused_without_changing_the_source() {
    // Tangent line-arc junction (a slot end): there is no corner to treat.
    let (session, id, ids) = build(
        &[[0., 0.], [40., 0.], [40., 10.], [40., 20.]],
        &[Spec::Line(0, 1), Spec::Arc(2, 1, 3, false)],
    );
    let (source, _, _) = Source::read(session.document(), id).unwrap();
    for treatment in [
        CornerTreatment::Fillet { radius_mm: 1. },
        CornerTreatment::Chamfer { distance_mm: 1. },
    ] {
        let error = corner_draft(&source, ids[1], treatment).unwrap_err();
        assert!(error.to_string().contains("tangentially"), "{error}");
    }
    // Sizes that do not fit on the short arcs.
    let (session, id, corner) = arc_arc();
    let before = session.document().clone();
    let (source, _, _) = Source::read(&before, id).unwrap();
    for treatment in [
        CornerTreatment::Fillet { radius_mm: 40. },
        CornerTreatment::Chamfer { distance_mm: 25. },
    ] {
        assert!(
            corner_draft(&source, corner, treatment).is_err(),
            "{treatment:?}"
        );
    }
    // An arc center is not a corner; nor is a point on three curves.
    let center = source.sketch.points[0].id;
    assert!(corner_draft(&source, center, CornerTreatment::Fillet { radius_mm: 1. }).is_err());
    let mut three = source.clone();
    let far = three.add_point([-20., 10.]);
    three.add_line(corner, far);
    assert!(!treatable_corners(&three).contains(&corner));
    assert!(corner_draft(&three, corner, CornerTreatment::Fillet { radius_mm: 1. }).is_err());
    assert_eq!(session.document(), &before);
}
