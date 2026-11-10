use multicad_core::{
    document::EntityId,
    drawings::{VectorCurve, VectorRole},
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, DrawingProjection, DrawingSolid, GeometryErrorKind, GeometryPolicy,
    PlaneFrame, ProjectedVector, Solid, project_drawing,
    rust_kernel::{Rational, polygon::PolygonPrism},
};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn rust_box(size: [i64; 3], frame: PlaneFrame) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(n),
        frame,
        GeometryPolicy::default(),
    )
    .unwrap()
}

fn sources<'a>(solids: &[&'a Solid]) -> Vec<DrawingSolid<'a>> {
    solids
        .iter()
        .map(|&solid| DrawingSolid { solid, hatch: None })
        .collect()
}

fn line(vector: &ProjectedVector) -> [[f64; 2]; 2] {
    let VectorCurve::Line { a, b } = vector.curve else {
        panic!("expected analytic line, got {:?}", vector.curve)
    };
    [a, b]
}

fn end_error(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> f64 {
    a.into_iter()
        .flatten()
        .zip(b.into_iter().flatten())
        .map(|(a, b)| (a - b).abs())
        .fold(0., f64::max)
}

#[test]
fn oblique_rust_box_matches_independent_native_exact_line_visibility() {
    let rust = rust_box([31, 23, 11], PlaneFrame::default());
    let native = Solid::box_solid([31., 23., 11.], GeometryPolicy::default()).unwrap();
    let options = DrawingProjection {
        frame: PlaneFrame {
            origin: [3., 5., 7.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
        hidden_lines: true,
        ..Default::default()
    };
    let actual = project_drawing(&sources(&[&rust]), options).unwrap();
    let expected = project_drawing(&sources(&[&native]), options).unwrap();
    assert_eq!(actual.len(), 12);
    assert_eq!(expected.len(), 12);
    assert!(actual.iter().any(|edge| edge.role == VectorRole::Hidden));
    let mut seen = vec![false; expected.len()];
    for edge in &actual {
        let ends = line(edge);
        let hit = expected.iter().enumerate().position(|(i, reference)| {
            if seen[i] || reference.role != edge.role {
                return false;
            }
            let other = line(reference);
            end_error(ends, other).min(end_error(ends, [other[1], other[0]])) < 1e-6
        });
        seen[hit.unwrap_or_else(|| panic!("unmatched line {edge:?}"))] = true;
        assert_eq!(edge.source_index, 0);
        assert_eq!(edge.preview_mm, ends);
    }
    let visible = project_drawing(
        &sources(&[&rust]),
        DrawingProjection {
            hidden_lines: false,
            ..options
        },
    )
    .unwrap();
    assert_eq!(
        visible,
        actual
            .into_iter()
            .filter(|edge| edge.role == VectorRole::Visible)
            .collect::<Vec<_>>()
    );
}

#[test]
fn panel_hole_and_rear_part_project_exact_in_a_rigidly_placed_view() {
    let feature = EntityId::new();
    let rings = [
        vec![[1, -2], [9, -2], [9, 2], [1, 2]],
        vec![[4, -1], [4, 1], [6, 1], [6, -1]],
    ];
    let prism = PolygonPrism::new(
        rings
            .into_iter()
            .map(|ring| ring.into_iter().map(|p| p.map(n)).collect())
            .collect(),
        n(1),
    )
    .unwrap();
    let frame = PlaneFrame {
        origin: [200., -250., 300.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    };
    let placed = |height: f64| PlaneFrame {
        origin: std::array::from_fn(|i| frame.origin[i] + frame.normal()[i] * height),
        ..frame
    };
    let origins: Vec<_> = (0..8)
        .map(|index| Origin {
            source: feature,
            role: format!("line/{index}"),
        })
        .collect();
    let front = Solid::from_polygon_prism(
        prism,
        placed(2.),
        feature,
        &origins,
        &[false; 8],
        Default::default(),
    )
    .unwrap();
    let rear = rust_box([10, 4, 1], placed(-2.));
    let output = project_drawing(
        &sources(&[&front, &rear]),
        DrawingProjection {
            frame,
            detail: Some(([5., 0.], 3.)),
            hidden_lines: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut visible: Vec<_> = output
        .iter()
        .filter(|v| v.source_index == 1 && v.role == VectorRole::Visible)
        .map(line)
        .filter(|ends| ends.iter().all(|point| point[1].abs() < 1e-7))
        .map(|[[a, _], [b, _]]| [a.min(b), a.max(b)])
        .collect();
    visible.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(visible.len(), 1);
    assert!((visible[0][0] - 4.).abs() < 1e-7);
    assert!((visible[0][1] - 6.).abs() < 1e-7);
    assert!(
        output
            .iter()
            .any(|v| v.source_index == 1 && v.role == VectorRole::Hidden)
    );
    assert!(
        output
            .iter()
            .any(|v| v.source_index == 0 && v.role == VectorRole::Visible)
    );
}

#[test]
fn orthogonal_boolean_hole_preserves_the_trimmed_opening() {
    let policy = GeometryPolicy::default();
    let box_ = rust_box([10, 10, 3], PlaneFrame::default());
    let cut = rust_box(
        [4, 4, 5],
        PlaneFrame {
            origin: [3., 3., -1.],
            ..Default::default()
        },
    );
    let plate = box_
        .boolean(&cut, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let output = project_drawing(&sources(&[&plate]), DrawingProjection::default()).unwrap();
    let mut lengths: Vec<_> = output
        .iter()
        .map(line)
        .map(|[[x1, y1], [x2, y2]]| (x2 - x1).hypot(y2 - y1))
        .collect();
    lengths.sort_by(f64::total_cmp);
    assert_eq!(lengths, [4., 4., 4., 4., 10., 10., 10., 10.]);
}

#[test]
fn unsupported_geometry_and_over_budget_output_fail_atomically() {
    let box_ = rust_box([10, 10, 3], PlaneFrame::default());
    let native = Solid::box_solid([5., 5., 3.], GeometryPolicy::default()).unwrap();
    assert_eq!(
        project_drawing(&sources(&[&box_, &native]), DrawingProjection::default())
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(
        project_drawing(
            &sources(&[&box_]),
            DrawingProjection {
                maximum_points: 7,
                ..Default::default()
            }
        )
        .unwrap_err()
        .kind,
        GeometryErrorKind::Limit
    );
}
