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
    let checked = |axes, tangent| {
        let view = View::new([0.; 3], axes, TOL).unwrap();
        let elevation = shell.elevation([0.; 3], axes, 0, TOL, tangent).unwrap();
        line_present(&elevation, root.map(|p| view.point(p)))
    };
    assert!(!checked(axes(2, 1.), false));
    assert!(checked(axes(2, 1.), true));
    assert!(checked(axes(1, 1.), false));
    assert!(checked([[-0.8, 0.6, 0.], Z, [0.6, 0.8, 0.]], false));

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
    for toward in [[0., -q, -q], [0.8, -0.6 * q, -0.6 * q]] {
        let events = patch
            .silhouettes(&shell, toward, axis_direction, TOL)
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
    assert!(
        patch
            .silhouettes(&shell, axis_direction, axis_direction, TOL)
            .unwrap()
            .into_iter()
            .all(|candidate| candidate.is_none())
    );
}

#[test]
fn oblique_view_preserves_exact_projected_web_rim_and_depth() {
    let n = [1., 2., 3.].map(|component| component / 14_f64.sqrt());
    let u = [-2., 1., 0.].map(|component| component / 5_f64.sqrt());
    for thickness in [-1., 1.] {
        for sign in [-1., 1.] {
            let n = n.map(|component| component * sign);
            let axes = [u, cross(n, u), n];
            let view = View::new([0.; 3], axes, TOL).unwrap();
            let [x, y, depth] = view.point([5., 4., 0.]);
            let elevation = fixture(thickness)
                .elevation([0.; 3], axes, 17, TOL, false)
                .unwrap();
            let rim_sweep: f64 = elevation
                .rims
                .iter()
                .filter_map(|rim| {
                    let [cx, cy] = rim.ellipse.center;
                    if (cx - x).hypot(cy - y) > TOL || (rim.depth - depth).abs() > TOL {
                        return None;
                    }
                    assert_eq!(rim.source, 17);
                    assert!((rim.ellipse.major - 1.).abs() <= TOL);
                    assert!((rim.ellipse.minor - n[2].abs()).abs() <= TOL);
                    assert!((rim.depth_sine.abs() - n[0].hypot(n[1])).abs() <= TOL);
                    Some(rim.ellipse.trim[1] - rim.ellipse.trim[0])
                })
                .sum();
            assert!((rim_sweep - TAU).abs() <= TOL);
        }
    }
}

#[test]
fn nonrigid_frame_is_rejected() {
    assert_eq!(
        fixture(1.)
            .elevation([0.; 3], [X, X, Z], 0, TOL, false)
            .err(),
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
}

#[test]
fn projected_miter_ellipse_retains_the_exact_trim_and_depth() {
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let ellipse = Ellipse::new([3., 4., 5.], Z, [q, q, 0.], [2., 1.]).unwrap();
    let arc = EllipseArc::new(ellipse, 0.2, -1.4).unwrap();
    let views = [
        [X, Z, [0., -1., 0.]],
        [X, [0., q, q], [0., -q, q]],
        [X, Y, Z],
    ];
    for axes in views {
        let view = View::new([0.; 3], axes, TOL).unwrap();
        let candidates: Vec<_> = view.ellipses(42, arc).collect();
        assert!(!candidates.is_empty());
        for candidate in candidates {
            assert_eq!(candidate.source, 42);
            assert_eq!(candidate.ellipse.minor == 0., axes == views[0]);
            let [a, b] = candidate.ellipse.trim;
            for theta in [a, (a + b) * 0.5, b] {
                let (sin, cos) = theta.sin_cos();
                let [x, y] = std::array::from_fn(|i| {
                    candidate.ellipse.center[i]
                        + candidate.ellipse.major * candidate.ellipse.u[i] * cos
                        + candidate.ellipse.minor * candidate.ellipse.v[i] * sin
                });
                let z = candidate.depth + candidate.depth_cosine * cos + candidate.depth_sine * sin;
                let point =
                    std::array::from_fn(|i| x * axes[0][i] + y * axes[1][i] + z * axes[2][i]);
                let parameter = ellipse.parameter_at(point, TOL).unwrap();
                let travel = (arc.start_rad() - parameter + TOL).rem_euclid(TAU);
                assert!(travel < 1.4 + 2. * TOL, "{parameter} / {travel}");
            }
        }
    }
}

#[test]
fn circular_miter_projects_trimmed_silhouettes_and_analytic_visible_and_hidden_rims() {
    use crate::{
        curved_projection::Curve,
        planar_projection::{Options, SceneExtras, project_scene_with_analytic_faces},
    };

    let shell = circular_miter([[0.; 3], [0., 0., 20.], [20., 0., 20.]], 2., TOL).unwrap();
    let side = shell
        .elevation([0.; 3], [X, Z, [0., -1., 0.]], 19, TOL, false)
        .unwrap();
    for [a, b] in [
        [[-2., 0.], [-2., 22.]],
        [[2., 0.], [2., 18.]],
        [[-2., 22.], [20., 22.]],
        [[2., 18.], [20., 18.]],
    ] {
        assert!(line_present(&side, [a, b].map(|[x, y]| [x, y, 0.])));
    }
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let oblique = shell
        .elevation([0.; 3], [X, [0., q, q], [0., -q, q]], 19, TOL, false)
        .unwrap();
    let (_, _, edges) = project_scene_with_analytic_faces(
        &[],
        &[],
        SceneExtras {
            lines: &oblique.lines,
            spatial_rims: &oblique.rims,
            spatial_ellipses: &oblique.spatial_ellipses,
            ..Default::default()
        },
        &oblique.blockers,
        Options {
            tolerance: TOL,
            hidden_lines: true,
            tangent_edges: false,
            detail: None,
            segment_limit: 1000,
        },
    )
    .unwrap();
    let mut coverage = [0.; 2];
    for edge in edges {
        assert_eq!(edge.source, 19);
        if let Curve::Ellipse(ellipse) = edge.edge.curve
            && ellipse.center[0].abs() <= TOL
            && (ellipse.center[1] - 20. * q).abs() <= TOL
        {
            assert!((ellipse.major.powi(2) - (4. + 8_f64.sqrt())).abs() <= TOL);
            assert!((ellipse.minor.powi(2) - (4. - 8_f64.sqrt())).abs() <= TOL);
            coverage[usize::from(edge.edge.hidden)] += ellipse.trim[1] - ellipse.trim[0];
        }
    }
    assert!(
        coverage.into_iter().all(|sweep| sweep > TOL),
        "{coverage:?}"
    );
    assert!((coverage.into_iter().sum::<f64>() - TAU).abs() <= TOL);
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
