use super::*;
use crate::helical_tube_cut::Spec;

const ORIGIN: Point = [17., 4., -12.];
const U: Point = [0.8, 0., -0.6];
const V: Point = [0., 1., 0.];
const TOLERANCE: f64 = 1e-7;

fn spec() -> Spec {
    Spec {
        stock_radius_mm: 8.,
        stock_height_mm: 30.,
        tube_radius_mm: 0.6,
        pitch_mm: 2.,
        path_height_mm: 15.,
        start_height_mm: 16.2,
        start_angle_rad: 0.,
    }
}

#[test]
fn helical_cut_step_writes_one_closed_shell_with_shared_trim_edges() {
    for phase in [-TAU, -0.1, 0., 0.1, TAU] {
        let cut = Cut::new(
            Spec {
                start_angle_rad: phase,
                ..spec()
            },
            TOLERANCE,
        )
        .unwrap();
        assert!(supports_helical_tube_cut(cut, ORIGIN, TOLERANCE));
        let step = write_helical_tube_cut(cut, ORIGIN, U, V, TOLERANCE).unwrap();
        for (entity, count) in [
            ("MANIFOLD_SOLID_BREP(", 1),
            ("CLOSED_SHELL(", 1),
            ("ADVANCED_FACE(", 5),
            ("EDGE_CURVE(", 9),
            ("ORIENTED_EDGE(", 18),
            ("VERTEX_POINT(", 6),
            ("SEAM_CURVE(", 1),
            ("CYLINDRICAL_SURFACE(", 1),
            ("B_SPLINE_SURFACE_WITH_KNOTS(", 1),
            ("PLANE(", 3),
            ("CIRCLE(", 4),
            ("ELLIPSE(", 1),
        ] {
            assert_eq!(step.matches(entity).count(), count, "wrong {entity} count");
        }
        assert!(step.len() < crate::step_read::MAX_PLANAR_STEP_BYTES);
        assert_eq!(
            step,
            write_helical_tube_cut(cut, ORIGIN, U, V, TOLERANCE).unwrap()
        );
    }
}

#[test]
fn rejects_unbounded_seams_or_world_precision_before_serializing() {
    let cut = Cut::new(spec(), TOLERANCE).unwrap();
    for origin in [[1e7, 0., 0.], [f64::NAN, 0., 0.], [1e6, 0., 0.]] {
        assert!(!supports_helical_tube_cut(cut, origin, TOLERANCE));
    }
    assert!(!supports_helical_tube_cut(cut, ORIGIN, f64::NAN));
    let long = Cut::new(
        Spec {
            stock_height_mm: 620.,
            start_height_mm: 610.,
            ..spec()
        },
        TOLERANCE,
    )
    .unwrap();
    assert!(!supports_helical_tube_cut(long, ORIGIN, TOLERANCE));
    let mut writer = Writer::new();
    writer
        .append_box([Rational::new(4, 1).unwrap(); 3], [0.; 3], [1., 0., 0.], V)
        .unwrap();
    assert_eq!(
        writer.append_helical_tube_cut(long, ORIGIN, U, V, TOLERANCE),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer.append_helical_tube_cut(cut, ORIGIN, U, U, TOLERANCE),
        Err(StepError::InvalidPlacement)
    );
    let file = writer.finish().unwrap();
    assert_eq!(file.matches("MANIFOLD_SOLID_BREP(").count(), 1);
    assert!(!file.contains("SEAM_CURVE("));
}

#[test]
fn rejects_a_shared_vertex_outside_the_trim_certificate() {
    let mut file = StepFile::new();
    let vertices = [[0.; 3], [10., 0., 0.]].map(|point| file.vertex(point));
    let spline = Spline {
        degree: 1,
        knots: vec![0., 1.],
        multiplicities: vec![2, 2],
        poles_mm: vec![[0.; 3], [9., 0., 0.]],
    };
    let placement = Placement::new([0.; 3], [1., 0., 0.], V).unwrap();
    assert_eq!(
        curves::spline(&mut file, &spline, vertices, &placement, TOLERANCE).err(),
        Some(StepError::InvalidGeometry)
    );
    assert_eq!(
        check_error_budget(TOLERANCE, TOLERANCE),
        Err(StepError::Limit)
    );
    assert_eq!(
        curves::floor_cylinder_uv(Cut::new(spec(), TOLERANCE).unwrap(), 0., 0.).err(),
        Some(StepError::Limit)
    );
}
