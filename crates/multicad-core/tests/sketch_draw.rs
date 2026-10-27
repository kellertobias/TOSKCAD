use multicad_core::{
    constraint_sketch::{ConstraintKind, CurveKind, Sketch, source::Source},
    document::EntityId,
    sketch_draw::{self, DrawInput, DrawRequest, PointInput},
};
fn source() -> Source {
    Source {
        u: [0., 0., 1.],
        sketch: Sketch::default(),
    }
}
fn p(x: f64, y: f64) -> PointInput {
    PointInput::Position([x, y])
}
fn request(input: DrawInput) -> DrawRequest {
    DrawRequest {
        input,
        construction: false,
    }
}
#[test]
fn existing_points_are_true_identity_snaps_and_old_source_is_unchanged() {
    let mut source = source();
    let fixed = source.add_point([0., 0.]);
    let other = source.add_point([10., 0.]);
    source.add_line(fixed, other);
    source.add_constraint(ConstraintKind::Fix { point: fixed });
    let original = source.clone();
    let draft = sketch_draw::draft(
        &source,
        &request(DrawInput::Line {
            start: PointInput::Existing(other),
            end: p(10., 20.),
        }),
    )
    .unwrap();
    assert_eq!(source, original);
    assert_eq!(draft.source.u, original.u);
    assert_eq!(draft.source.sketch.constraints, original.sketch.constraints);
    assert_eq!(draft.source.sketch.points[..2], original.sketch.points);
    assert_eq!(draft.source.sketch.curves[..1], original.sketch.curves);
    assert_eq!(draft.created_curves.len(), 1);
    assert_eq!(draft.source.sketch.points.len(), 3);
    assert!(
        matches!(draft.source.sketch.curves[1].kind,CurveKind::Line {start,..} if start==other)
    );
    drop(draft);
    assert_eq!(source, original);
}
#[test]
fn rectangle_reuses_diagonal_points_and_adds_shared_corners_and_hv_constraints() {
    let mut s = source();
    let a = s.add_point([10., 20.]);
    let b = s.add_point([-30., -40.]);
    let mut input = request(DrawInput::Rectangle {
        a: PointInput::Existing(a),
        b: PointInput::Existing(b),
    });
    input.construction = true;
    let draft = sketch_draw::draft(&s, &input).unwrap();
    assert_eq!(draft.source.sketch.points.len(), 4);
    assert_eq!(draft.created_curves.len(), 4);
    assert!(draft.source.sketch.curves.iter().all(|c| c.construction));
    for i in 0..4 {
        let CurveKind::Line { start, end } = draft.source.sketch.curves[i].kind else {
            panic!("line")
        };
        let CurveKind::Line { start: next, .. } = draft.source.sketch.curves[(i + 1) % 4].kind
        else {
            panic!("line")
        };
        assert_eq!(end, next);
        if i == 0 {
            assert_eq!(start, a);
        }
        if i == 2 {
            assert_eq!(start, b);
        }
        assert_eq!(
            draft.source.sketch.constraints[i].kind,
            if i % 2 == 0 {
                ConstraintKind::Horizontal {
                    line: draft.created_curves[i],
                }
            } else {
                ConstraintKind::Vertical {
                    line: draft.created_curves[i],
                }
            }
        );
    }
}
#[test]
fn closed_and_open_polylines_have_shared_vertices_without_implicit_constraints() {
    for closed in [false, true] {
        let mut s = source();
        let first = s.add_point([0., 0.]);
        let draft = sketch_draw::draft(
            &s,
            &request(DrawInput::Polyline {
                vertices: vec![PointInput::Existing(first), p(20., 0.), p(20., 10.)],
                closed,
            }),
        )
        .unwrap();
        assert_eq!(draft.created_curves.len(), if closed { 3 } else { 2 });
        assert_eq!(draft.source.sketch.points.len(), 3);
        assert!(draft.source.sketch.constraints.is_empty());
        let CurveKind::Line { end, .. } = draft.source.sketch.curves.last().unwrap().kind else {
            panic!("line")
        };
        if closed {
            assert_eq!(end, first);
        }
    }
}
#[test]
fn circles_and_directed_arcs_preserve_exact_typed_geometry_and_construction() {
    let mut s = source();
    let center = s.add_point([0., 0.]);
    let start = s.add_point([10., 0.]);
    for clockwise in [false, true] {
        let mut r = request(DrawInput::Arc {
            center: PointInput::Existing(center),
            start: PointInput::Existing(start),
            end: p(0., 10.),
            clockwise,
        });
        r.construction = true;
        let draft = sketch_draw::draft(&s, &r).unwrap();
        assert_eq!(draft.source.sketch.points.len(), 3);
        let curve = &draft.source.sketch.curves[0];
        assert!(curve.construction);
        assert!(
            matches!(curve.kind,CurveKind::Arc {center:c,start:a,clockwise:direction,..} if c==center&&a==start&&direction==clockwise)
        );
    }
    let draft = sketch_draw::draft(
        &s,
        &request(DrawInput::Circle {
            center: PointInput::Existing(center),
            radius_mm: 12.5,
        }),
    )
    .unwrap();
    assert_eq!(draft.source.sketch.points.len(), 2);
    assert!(
        matches!(draft.source.sketch.curves[0].kind,CurveKind::Circle {center:c,radius_mm:12.5} if c==center)
    );
}
#[test]
fn invalid_geometry_missing_snaps_and_large_inputs_leave_source_untouched() {
    let s = source();
    let before = s.clone();
    let inputs = [
        DrawInput::Line {
            start: p(0., 0.),
            end: p(0., 0.),
        },
        DrawInput::Line {
            start: p(f64::NAN, 0.),
            end: p(1., 2.),
        },
        DrawInput::Line {
            start: PointInput::Existing(EntityId::new()),
            end: p(1., 2.),
        },
        DrawInput::Line {
            start: p(0., 0.),
            end: p(1e9 + 1., 0.),
        },
        DrawInput::Rectangle {
            a: p(0., 0.),
            b: p(0., 10.),
        },
        DrawInput::Circle {
            center: p(0., 0.),
            radius_mm: 0.,
        },
        DrawInput::Circle {
            center: p(0., 0.),
            radius_mm: f64::INFINITY,
        },
        DrawInput::Arc {
            center: p(0., 0.),
            start: p(10., 0.),
            end: p(0., 11.),
            clockwise: false,
        },
        DrawInput::Arc {
            center: p(0., 0.),
            start: p(10., 0.),
            end: p(10., 0.),
            clockwise: false,
        },
        DrawInput::Polyline {
            vertices: vec![p(0., 0.)],
            closed: false,
        },
        DrawInput::Polyline {
            vertices: vec![p(0., 0.), p(1., 0.)],
            closed: true,
        },
        DrawInput::Polyline {
            vertices: vec![p(0., 0.), p(10., 0.), p(0., 0.)],
            closed: true,
        },
        DrawInput::Polyline {
            vertices: vec![p(0., 0.); 513],
            closed: false,
        },
    ];
    for input in inputs {
        assert!(sketch_draw::draft(&s, &request(input)).is_err());
        assert_eq!(s, before);
    }
}
#[test]
fn curve_extrema_bounds_use_the_actual_arc_sweep() {
    let s = source();
    assert!(
        sketch_draw::draft(
            &s,
            &request(DrawInput::Circle {
                center: p(1e9, 0.),
                radius_mm: 10.
            })
        )
        .is_err()
    );
    for (clockwise, valid) in [(false, true), (true, false)] {
        let result = sketch_draw::draft(
            &s,
            &request(DrawInput::Arc {
                center: p(1e9, 0.),
                start: p(1e9, 10.),
                end: p(1e9, -10.),
                clockwise,
            }),
        );
        assert_eq!(result.is_ok(), valid);
    }
}
#[test]
fn source_budget_failure_is_atomic_but_reusing_existing_points_remains_possible() {
    let mut s = source();
    for i in 0..256 {
        s.add_point([i as f64, 0.]);
    }
    let before = s.clone();
    assert!(
        sketch_draw::draft(
            &s,
            &request(DrawInput::Circle {
                center: p(0., 10.),
                radius_mm: 5.
            })
        )
        .is_err()
    );
    assert_eq!(s, before);
    let r = sketch_draw::draft(
        &s,
        &request(DrawInput::Line {
            start: PointInput::Existing(s.sketch.points[0].id),
            end: PointInput::Existing(s.sketch.points[1].id),
        }),
    )
    .unwrap();
    assert_eq!(r.source.sketch.points.len(), 256);
}
#[test]
fn drawing_requests_have_a_checked_roundtrip_for_external_callers() {
    let r = DrawRequest {
        input: DrawInput::Polyline {
            vertices: vec![
                p(0., 0.),
                PointInput::Existing(EntityId::new()),
                p(10., 20.),
            ],
            closed: true,
        },
        construction: true,
    };
    let value = serde_json::to_value(&r).unwrap();
    assert_eq!(
        serde_json::from_value::<DrawRequest>(value.clone()).unwrap(),
        r
    );
    let mut malformed = value;
    malformed["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<DrawRequest>(malformed).is_err());
}
