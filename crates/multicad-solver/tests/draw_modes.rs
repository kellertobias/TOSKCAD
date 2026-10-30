//! Rectangle and circle drawing modes: typed input, solver evaluation, one
//! undoable commit, and failure cases that leave the document unchanged.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{ConstraintKind, CurveKind, Dimension, Sketch, source::Source},
    document::*,
    sketch::Datum,
    sketch_draw::{DrawInput, DrawRequest, PointInput},
};
use multicad_solver::{draw::prepare_draw, prepare_edit};

fn actor() -> Actor {
    Actor {
        id: "drawing-modes".into(),
        channel: Channel::Ui,
    }
}
fn request(input: DrawInput) -> DrawRequest {
    DrawRequest {
        input,
        construction: false,
    }
}
fn setup() -> (Session, EntityId) {
    let (id, edits) = Source::create(Datum::Xy, "Modes").unwrap();
    let document = Document::new("Drawing modes")
        .unwrap()
        .edited(&edits)
        .unwrap();
    (Session::new(document).unwrap(), id)
}
fn position(sketch: &Sketch, id: EntityId) -> [f64; 2] {
    sketch
        .points
        .iter()
        .find(|p| p.id == id)
        .unwrap()
        .position_mm
}
fn line_ends(sketch: &Sketch, id: EntityId) -> ([f64; 2], [f64; 2]) {
    match sketch.curves.iter().find(|c| c.id == id).unwrap().kind {
        CurveKind::Line { start, end } => (position(sketch, start), position(sketch, end)),
        _ => panic!("line expected"),
    }
}
fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}
/// Closed loop of four lines with perpendicular neighbouring edges.
fn assert_rectangle(sketch: &Sketch, lines: &[EntityId], corners: [[f64; 2]; 4]) {
    assert_eq!(lines.len(), 4);
    for (i, line) in lines.iter().enumerate() {
        let (a, b) = line_ends(sketch, *line);
        assert!(
            close(a, corners[i]),
            "corner {i}: {a:?} vs {:?}",
            corners[i]
        );
        assert!(close(b, corners[(i + 1) % 4]));
        let (c, d) = line_ends(sketch, lines[(i + 1) % 4]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
        let cosine = (u[0] * v[0] + u[1] * v[1]) / (u[0].hypot(u[1]) * v[0].hypot(v[1]));
        assert!(
            cosine.abs() < 1e-6,
            "edges {i} and next are not perpendicular"
        );
    }
}

fn commit(input: DrawInput) -> (Session, EntityId, Vec<EntityId>, Document) {
    let (mut session, id) = setup();
    let original = session.document().clone();
    let draw = prepare_draw(&session, 0, actor(), id, None, &request(input)).unwrap();
    assert_eq!(session.document(), &original, "preview does not commit");
    let created = draw.created_curves.clone();
    session
        .commit(draw.edit.preview.expect("valid solve"))
        .unwrap();
    (session, id, created, original)
}

#[test]
fn rectangle_modes_create_constrained_rectangles_in_one_undoable_step() {
    let cases = [
        (
            DrawInput::CenterRectangle {
                center: [10., 5.],
                corner: PointInput::Position([30., 15.]),
            },
            [[30., 15.], [-10., 15.], [-10., -5.], [30., -5.]],
            "horizontal",
        ),
        (
            DrawInput::ThreePointRectangle {
                a: PointInput::Position([0., 0.]),
                b: PointInput::Position([10., 10.]),
                height: [0., 20.],
            },
            [[0., 0.], [10., 10.], [0., 20.], [-10., 10.]],
            "parallel",
        ),
        (
            DrawInput::CenterPointsRectangle {
                center: [0., 0.],
                corner: PointInput::Position([10., 0.]),
                // Direction only: the neighbour lands at the same radius.
                adjacent: [0., 50.],
            },
            [[10., 0.], [0., 10.], [-10., 0.], [0., -10.]],
            "parallel",
        ),
        (
            DrawInput::ThreePointRectangle {
                a: PointInput::Position([0., 0.]),
                b: PointInput::Position([40., 0.]),
                height: [25., 20.],
            },
            [[0., 0.], [40., 0.], [40., 20.], [0., 20.]],
            "horizontal",
        ),
    ];
    for (input, corners, kind) in cases {
        let (mut session, id, lines, original) = commit(input);
        let (source, _, _) = Source::read(session.document(), id).unwrap();
        assert_rectangle(&source.sketch, &lines, corners);
        let constraints: Vec<_> = source.sketch.constraints.iter().map(|c| &c.kind).collect();
        match kind {
            "horizontal" => {
                assert_eq!(constraints.len(), 4);
                assert!(constraints.iter().all(|c| matches!(
                    c,
                    ConstraintKind::Horizontal { .. } | ConstraintKind::Vertical { .. }
                )));
            }
            _ => {
                assert_eq!(constraints.len(), 3);
                assert!(
                    constraints
                        .iter()
                        .filter(|c| matches!(c, ConstraintKind::Parallel { .. }))
                        .count()
                        == 2
                );
                assert!(
                    constraints
                        .iter()
                        .any(|c| matches!(c, ConstraintKind::Perpendicular { .. }))
                );
            }
        }
        session.undo(1, actor()).unwrap();
        assert_eq!(session.document(), &original);
    }
}

#[test]
fn rotated_rectangles_stay_rectangular_when_the_solver_changes_an_edge() {
    let (session, id, lines, _) = commit(DrawInput::ThreePointRectangle {
        a: PointInput::Position([0., 0.]),
        b: PointInput::Position([30., 10.]),
        height: [20., 40.],
    });
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    source.add_constraint(ConstraintKind::Length {
        line: lines[0],
        dimension: Dimension::Literal {
            value: 50.,
            unit: "mm".into(),
        },
    });
    let edit = prepare_edit(&session, session.revision(), actor(), id, source).unwrap();
    let solved = edit.evaluation.candidate.expect("solvable rectangle edit");
    let ends: Vec<_> = lines.iter().map(|l| line_ends(&solved, *l)).collect();
    let length = |(a, b): ([f64; 2], [f64; 2])| (b[0] - a[0]).hypot(b[1] - a[1]);
    assert!((length(ends[0]) - 50.).abs() < 1e-6);
    assert!((length(ends[0]) - length(ends[2])).abs() < 1e-6);
    assert!((length(ends[1]) - length(ends[3])).abs() < 1e-6);
    let corners = [ends[0].0, ends[1].0, ends[2].0, ends[3].0];
    assert_rectangle(&solved, &lines, corners);
}

#[test]
fn circle_modes_place_the_solved_circle_through_the_chosen_points() {
    let (session, id, curves, _) = commit(DrawInput::ThreePointCircle {
        a: [10., 0.],
        b: [0., 10.],
        c: [-10., 0.],
    });
    let (source, _, _) = Source::read(session.document(), id).unwrap();
    let CurveKind::Circle { center, radius_mm } = source.sketch.curves[0].kind else {
        panic!()
    };
    assert_eq!(curves, vec![source.sketch.curves[0].id]);
    assert!(close(position(&source.sketch, center), [0., 0.]));
    assert!((radius_mm - 10.).abs() < 1e-9);
    for (left, expected) in [(true, [0., 3.]), (false, [0., -3.])] {
        let (mut session, id, _, original) = commit(DrawInput::TwoPointCircle {
            a: [-4., 0.],
            b: [4., 0.],
            radius_mm: 5.,
            left,
        });
        let (source, _, _) = Source::read(session.document(), id).unwrap();
        let CurveKind::Circle { center, radius_mm } = source.sketch.curves[0].kind else {
            panic!()
        };
        assert!(
            close(position(&source.sketch, center), expected),
            "left={left}"
        );
        assert_eq!(radius_mm, 5.);
        session.undo(1, actor()).unwrap();
        assert_eq!(session.document(), &original);
    }
}

#[test]
fn degenerate_mode_input_is_rejected_without_changing_the_document() {
    let (session, id) = setup();
    let before = session.document().clone();
    for input in [
        DrawInput::ThreePointCircle {
            a: [0., 0.],
            b: [5., 5.],
            c: [10., 10.],
        },
        DrawInput::TwoPointCircle {
            a: [0., 0.],
            b: [10., 0.],
            radius_mm: 4.,
            left: true,
        },
        DrawInput::TwoPointCircle {
            a: [1., 1.],
            b: [1., 1.],
            radius_mm: 4.,
            left: true,
        },
        DrawInput::ThreePointRectangle {
            a: PointInput::Position([0., 0.]),
            b: PointInput::Position([10., 0.]),
            height: [20., 0.],
        },
        DrawInput::CenterRectangle {
            center: [0., 0.],
            corner: PointInput::Position([0., 10.]),
        },
        DrawInput::CenterPointsRectangle {
            center: [0., 0.],
            corner: PointInput::Position([10., 0.]),
            adjacent: [20., 0.],
        },
        DrawInput::CenterRectangle {
            center: [f64::NAN, 0.],
            corner: PointInput::Position([10., 10.]),
        },
    ] {
        assert!(
            prepare_draw(&session, 0, actor(), id, None, &request(input.clone())).is_err(),
            "{input:?} accepted"
        );
        assert_eq!(session.document(), &before);
    }
}

#[test]
fn rectangle_modes_reuse_an_existing_corner_identity() {
    let (session, id) = setup();
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    let point = source.add_point([5., 5.]);
    let document = session
        .document()
        .edited(&source.replace(session.document(), id).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let draw = prepare_draw(
        &session,
        0,
        actor(),
        id,
        None,
        &request(DrawInput::ThreePointRectangle {
            a: PointInput::Existing(point),
            b: PointInput::Position([25., 5.]),
            height: [25., 15.],
        }),
    )
    .unwrap();
    session.commit(draw.edit.preview.unwrap()).unwrap();
    let (after, _, _) = Source::read(session.document(), id).unwrap();
    assert!(after.sketch.curves.iter().any(
        |c| matches!(c.kind, CurveKind::Line { start, end } if start == point || end == point)
    ));
    assert_eq!(
        after.sketch.points.len(),
        4,
        "one reused + three new corners"
    );
}
