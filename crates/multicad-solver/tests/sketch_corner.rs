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
