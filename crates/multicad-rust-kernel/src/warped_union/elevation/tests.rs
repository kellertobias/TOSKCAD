use super::*;
use crate::warped_union::section::tests::fixture;

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const Z: Point = [0., 0., 1.];
const TOL: f64 = 1e-7;

fn axes(axis: usize, sign: f64) -> [Point; 3] {
    let basis = [X, Y, Z];
    let u = basis[(axis + 1) % 3];
    let n = basis[axis].map(|value| value * sign);
    [u, cross(n, u), n]
}

fn line_present(elevation: &Elevation, ends: [Point; 2]) -> bool {
    elevation.lines.iter().any(|line| {
        [ends, [ends[1], ends[0]]].into_iter().any(|ends| {
            line.points
                .into_iter()
                .zip(ends)
                .all(|(a, b)| length3(sub(a, b)) <= TOL)
        })
    })
}

#[test]
fn all_cardinal_views_keep_exact_trims_and_source_identity() {
    for up in [-1., 1.] {
        let shell = fixture(up);
        for axis in 0..3 {
            for sign in [-1., 1.] {
                let elevation = shell
                    .elevation([0.; 3], axes(axis, sign), 17, TOL, false)
                    .unwrap();
                assert!(!elevation.blockers.is_empty());
                assert!(elevation.lines.iter().all(|line| line.source == 17));
                assert!(elevation.rims.iter().all(|rim| rim.source == 17));
                assert!(elevation.rims.iter().any(|rim| {
                    (rim.ellipse.major - rim.ellipse.minor).abs() <= TOL
                        && rim.depth_sine.abs() <= TOL
                }));
                assert!(elevation.rims.iter().any(|rim| {
                    rim.ellipse.minor == 0.
                        && (rim.ellipse.major - rim.depth_sine.abs()).abs() <= TOL
                }));
            }
        }
    }
}

#[test]
fn tangent_rails_are_optional_except_when_they_are_silhouettes() {
    let shell = fixture(1.);
    let root = [[2., 0., 0.], [8., 0., 0.]];
    let checked = |axis, tangent| {
        let axes = axes(axis, 1.);
        let view = View::new([0.; 3], axes, TOL).unwrap();
        let elevation = shell.elevation([0.; 3], axes, 0, TOL, tangent).unwrap();
        line_present(&elevation, root.map(|p| view.point(p)))
    };
    assert!(!checked(2, false));
    assert!(checked(2, true));
    assert!(checked(1, false));

    // A bore's quadrant partition is not a physical sharp or tangent edge.
    let top = shell.elevation([0.; 3], [X, Y, Z], 0, TOL, true).unwrap();
    assert!(!line_present(&top, [[6., 4., 0.], [6., 4., 1.]]));
    // The same generator is published once in a transverse silhouette.
    let side = axes(1, 1.);
    let view = View::new([0.; 3], side, TOL).unwrap();
    let side = shell.elevation([0.; 3], side, 0, TOL, true).unwrap();
    let bore = [[6., 4., 0.], [6., 4., 1.]].map(|p| view.point(p));
    assert_eq!(
        side.lines
            .iter()
            .filter(|line| { [bore, [bore[1], bore[0]]].contains(&line.points) })
            .count(),
        1
    );
}

#[test]
fn curved_patch_has_an_analytic_interior_silhouette() {
    let shell = fixture(1.);
    let face = shell
        .faces()
        .iter()
        .find(|face| face.source_face == 10)
        .unwrap();
    let Support::Cylinder {
        axis_origin_mm,
        axis_direction,
        radius_mm,
        ..
    } = face.support
    else {
        panic!("first bend face must be cylindrical");
    };
    let patch =
        CylinderPatch::new(&shell, face, axis_origin_mm, axis_direction, radius_mm, TOL).unwrap();
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let events = patch
        .silhouettes(&shell, [0., -q, -q], axis_direction, TOL)
        .unwrap();
    let [Silhouette::Interior(points)] = events.into_iter().flatten().collect::<Vec<_>>()[..]
    else {
        panic!("exactly one interior generator was expected");
    };
    for point in points {
        assert!((point[1] + q).abs() <= TOL);
        assert!((point[2] + 1. - q).abs() <= TOL);
    }
    assert!(((points[1][0] - points[0][0]).abs() - 6.).abs() <= TOL);
}

#[test]
fn unsupported_view_and_nonrigid_frame_are_rejected() {
    let shell = fixture(1.);
    let q = std::f64::consts::FRAC_1_SQRT_2;
    assert_eq!(
        shell
            .elevation([0.; 3], [X, [0., q, -q], [0., q, q]], 0, TOL, false)
            .err(),
        Some(Error::Unsupported),
    );
    assert_eq!(
        shell.elevation([0.; 3], [X, X, Z], 0, TOL, false).err(),
        Some(Error::Invalid),
    );
}

#[test]
fn refuses_inconsistent_incidence_and_unproved_trim_geometry() {
    let mut shell = fixture(1.);
    let project = |shell: &Shell| shell.elevation([0.; 3], [X, Y, Z], 0, TOL, false).err();
    let duplicate = shell.faces[0].boundary[0];
    shell.faces[0].boundary.push(duplicate);
    assert_eq!(project(&shell), Some(Error::Invalid));
    shell.faces[0].boundary.pop();
    shell.faces[0].holes.push(Vec::new());
    assert_eq!(project(&shell), Some(Error::Unsupported));
    shell.faces.remove(0);
    assert_eq!(project(&shell), Some(Error::Invalid));

    let view = View::new([0.; 3], [X, Y, Z], TOL).unwrap();
    let ellipse = Ellipse::new([0.; 3], Z, X, [2., 1.]).unwrap();
    let arc = EllipseArc::new(ellipse, 0., FRAC_PI_2).unwrap();
    assert_eq!(view.rims(0, arc).err(), Some(Error::Unsupported));
}

#[test]
fn blocker_geometry_errors_preserve_invalid_and_budget_diagnostics() {
    let mut shell = fixture(1.);
    let plane = shell
        .faces
        .iter()
        .position(|face| matches!(face.support, Support::Plane { normal } if normal == Z))
        .unwrap();
    let project = |shell: &Shell| shell.elevation([0.; 3], [X, Y, Z], 0, TOL, false).err();
    let support = shell.faces[plane].support;
    shell.faces[plane].support = Support::Plane {
        normal: [0., 0., 2.],
    };
    assert_eq!(project(&shell), Some(Error::Invalid));
    shell.faces[plane].support = support;

    let start = shell.edges.len();
    shell.edges.resize(
        start + 9000,
        Edge {
            vertices: [0, 1],
            curve: Curve::Line,
        },
    );
    shell.faces[plane]
        .boundary
        .extend((start..start + 9000).map(|edge| OrientedEdge {
            edge,
            forward: true,
        }));
    assert_eq!(project(&shell), Some(Error::Limit));
}
