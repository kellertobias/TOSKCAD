use super::*;
#[test]
fn analytic_curves_require_the_entire_finite_oriented_arc() {
    let frame = Frame::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let expected = ReliefCurve::Profile {
        edge: ProfileEdge::Arc {
            center: [0.; 2],
            a: [1., 0.],
            b: [0., 1.],
            clockwise: false,
        },
        z_mm: 2.,
    };
    let arc = ArcEdge {
        center_mm: [0., 0., 2.],
        normal: [0., 0., 1.],
        radius_mm: 1.,
        radial_start: [1., 0., 0.],
        radial_end: [0., 1., 0.],
        sweep_rad: std::f64::consts::FRAC_PI_2,
    };
    assert!(curve_matches(Curve::Arc(arc), expected, true, frame, 1e-8));
    assert!(!curve_matches(Curve::Line, expected, true, frame, 1e-8));
    assert!(!curve_matches(
        Curve::Arc(arc),
        expected,
        false,
        frame,
        1e-8
    ));
    let line = ReliefCurve::Line {
        a_mm: [0.; 3],
        b_mm: [1.; 3],
    };
    assert!(curve_matches(Curve::Line, line, true, frame, 1e-8));
    assert!(!curve_matches(Curve::Arc(arc), line, true, frame, 1e-8));
    for changed in [
        ArcEdge {
            center_mm: [0.001, 0., 2.],
            ..arc
        },
        ArcEdge {
            radius_mm: 1.001,
            ..arc
        },
        ArcEdge {
            normal: [1., 0., 0.],
            ..arc
        },
        ArcEdge {
            sweep_rad: std::f64::consts::PI,
            ..arc
        },
    ] {
        assert!(!curve_matches(
            Curve::Arc(changed),
            expected,
            true,
            frame,
            1e-8
        ));
    }
    let actual = Curve::Circle(CircleEdge {
        center_mm: [0., 0., 2.],
        normal: [0., 0., -1.],
        radius_mm: 1.,
    });
    let expected = ReliefCurve::Profile {
        edge: ProfileEdge::Circle {
            center: [0.; 2],
            radius_mm: 1.,
            clockwise: true,
        },
        z_mm: 2.,
    };
    assert!(curve_matches(actual, expected, true, frame, 1e-8));
    assert!(!curve_matches(actual, expected, false, frame, 1e-8));
    let expected = ReliefCurve::Profile {
        edge: ProfileEdge::Line {
            a: [0.; 2],
            b: [1.; 2],
        },
        z_mm: 2.,
    };
    assert!(curve_matches(Curve::Line, expected, true, frame, 1e-8));
}
#[test]
fn finite_graph_correspondence_refuses_missing_or_ambiguous_vertices_and_edges() {
    let relief = super::super::tests::fixture(crate::normal_relief::Kind::Raised);
    let text =
        crate::step::write_normal_relief(&relief, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let entities = parse_entities(&text, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    let ids = shell.shell_faces().unwrap();
    shell.validate_faces(ids).unwrap();
    let frame = Frame::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let graph = relief.boundary();
    let original = vertices(&shell, graph, frame, 1e-8, &mut Work(MAX_WORK)).unwrap();
    let mut missing = graph.clone();
    missing.vertices.pop();
    assert!(vertices(&shell, &missing, frame, 1e-8, &mut Work(MAX_WORK)).is_err());
    let first = original[0];
    let point = shell.points[&first];
    shell.points.insert(first, [100.; 3]);
    assert!(vertices(&shell, graph, frame, 1e-8, &mut Work(MAX_WORK)).is_err());
    shell.points.insert(first, point);
    let second = original[1];
    let saved = shell.points[&second];
    shell.points.insert(second, point);
    assert!(vertices(&shell, graph, frame, 1e-8, &mut Work(MAX_WORK)).is_err());
    shell.points.insert(second, saved);
    let mapping = edges(&shell, graph, &original, frame, 1e-8, &mut Work(MAX_WORK)).unwrap();
    let id = mapping[0].0;
    let removed = shell.edges.remove(&id).unwrap();
    assert!(edges(&shell, graph, &original, frame, 1e-8, &mut Work(MAX_WORK)).is_err());
    shell.edges.insert(id, removed);
    let saved = shell.edges[&id].vertices;
    shell.edges.get_mut(&id).unwrap().vertices = [u32::MAX; 2];
    assert!(edges(&shell, graph, &original, frame, 1e-8, &mut Work(MAX_WORK)).is_err());
    shell.edges.get_mut(&id).unwrap().vertices = saved;
    assert!(edges(&shell, graph, &original, frame, 1e-8, &mut Work(MAX_WORK)).is_ok());
}
#[test]
fn a_different_support_class_is_not_hidden_by_a_valid_wire() {
    let relief = super::super::tests::fixture(crate::normal_relief::Kind::Raised);
    let text =
        crate::step::write_normal_relief(&relief, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let entities = parse_entities(&text, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    let ids = shell.shell_faces().unwrap();
    let faces = shell.validate_faces(ids.clone()).unwrap();
    let index = faces
        .iter()
        .position(|f| !matches!(f, Face::Planar { .. }))
        .unwrap();
    let plane = Surface::Plane {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    assert_eq!(
        support(
            &shell,
            ids[index],
            &faces[index],
            plane,
            Frame::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap(),
            1e-8,
            20.
        ),
        Err(ReadError::Invalid)
    );
}
#[test]
fn an_open_arc_is_not_a_full_periodic_circle_and_wrong_curve_labels_refuse() {
    let frame = Frame::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    let expected = ReliefCurve::Profile {
        edge: ProfileEdge::Arc {
            center: [0.; 2],
            a: [1., 0.],
            b: [0., 1.],
            clockwise: false,
        },
        z_mm: 2.,
    };
    assert!(!curve_matches(
        Curve::Circle(CircleEdge {
            center_mm: [0., 0., 2.],
            normal: [0., 0., 1.],
            radius_mm: 1.
        }),
        expected,
        true,
        frame,
        1e-8
    ));
    let relief = super::super::tests::fixture(crate::normal_relief::Kind::Raised);
    let text =
        crate::step::write_normal_relief(&relief, [0.; 3], frame.axes[0], frame.axes[1]).unwrap();
    let entities = parse_entities(&text, 1e-7).unwrap();
    let mut shell = Shell::new(&entities, 1e-7, true);
    let ids = shell.shell_faces().unwrap();
    shell.validate_faces(ids).unwrap();
    let vertices = vertices(&shell, relief.boundary(), frame, 1e-8, &mut Work(MAX_WORK)).unwrap();
    let mut wrong = relief.boundary().clone();
    wrong.edges[0].curve = expected;
    assert!(edges(&shell, &wrong, &vertices, frame, 1e-8, &mut Work(MAX_WORK)).is_err());
    assert!(
        edges(
            &shell,
            relief.boundary(),
            &vertices,
            frame,
            1e-8,
            &mut Work(MAX_WORK)
        )
        .is_ok()
    );
}
