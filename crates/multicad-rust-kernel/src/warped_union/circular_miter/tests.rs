use super::*;
use crate::display::{MeshError, Tessellation};

const RIGHT_ANGLE: [Point; 3] = [[0.; 3], [0., 0., 20.], [20., 0., 20.]];
const TOL: f64 = 1e-7;

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn trim(shell: &Shell, index: usize) -> EllipseArc {
    let Curve::Ellipse(arc) = shell.edges()[index].curve else {
        panic!("expected an exact circular or elliptical trim");
    };
    arc
}

#[test]
fn shares_the_exact_oblique_rim_and_keeps_two_end_disks() {
    let shell = circular_miter(RIGHT_ANGLE, 2., TOL).unwrap();
    assert_eq!(shell.solid_count(), 1);
    assert_eq!(shell.shell_count(), 1);
    assert_eq!(shell.vertices().len(), 12);
    assert_eq!(shell.edges().len(), 20);
    assert_eq!(shell.faces().len(), 10);
    close(shell.volume_mm3(), 160. * PI);
    close(
        shell.faces().iter().map(|face| face.area_mm2).sum(),
        168. * PI,
    );
    for (actual, expected) in shell
        .bounds_mm()
        .into_iter()
        .flatten()
        .zip([-2., -2., 0., 20., 2., 22.])
    {
        close(actual, expected);
    }
    let cap_keys: Vec<_> = shell
        .faces()
        .iter()
        .filter_map(|face| match face.support {
            Support::Plane { normal } => Some((face.source, face.source_face, normal)),
            Support::Cylinder { .. } => None,
        })
        .collect();
    assert_eq!(cap_keys, [(0, 0, [0., 0., -1.]), (1, 1, [1., 0., 0.])]);

    for edge in 4..8 {
        let arc = trim(&shell, edge);
        let ellipse = arc.ellipse();
        close(ellipse.semiaxes_mm()[0], 2. * std::f64::consts::SQRT_2);
        close(ellipse.semiaxes_mm()[1], 2.);
        assert_eq!(ellipse.center_mm(), RIGHT_ANGLE[1]);
        let point = ellipse.point_at(arc.start_rad() + FRAC_PI_2 / 3.);
        close(point[0].hypot(point[1]), 2.);
        close((point[2] - 20.).hypot(point[1]), 2.);
        close(dot(sub(point, RIGHT_ANGLE[1]), ellipse.normal()), 0.);
        let uses: Vec<_> = shell
            .faces()
            .iter()
            .flat_map(|face| {
                face.boundary
                    .iter()
                    .filter(move |use_| use_.edge == edge)
                    .map(move |use_| (face.source, use_.forward))
            })
            .collect();
        assert_eq!(uses, [(0, false), (1, true)]);
    }
}

#[test]
fn transports_an_obtuse_miter_and_integrates_unequal_sectors() {
    let path =
        [[0., 0., 0.], [0., 0., 20.], [12., 0., 4.]].map(|[x, y, z]| [13. + z, -7. + x, 42. + y]);
    let shell = circular_miter(path, 2., TOL).unwrap();
    let ellipse = trim(&shell, 4).ellipse();
    close(ellipse.semiaxes_mm()[0], 2. * 10_f64.sqrt());
    close(ellipse.semiaxes_mm()[1], 2.);
    assert_eq!(ellipse.center_mm(), path[1]);
    for leg in [0, 1] {
        let sectors: Vec<_> = shell
            .faces()
            .iter()
            .filter(|face| face.source == leg && face.source_face >= 2)
            .collect();
        assert_eq!(sectors.len(), 4);
        for (sector, delta_sine) in sectors.iter().zip([1., -1., -1., 1.]) {
            close(sector.area_mm2, 2. * (FRAC_PI_2 * 20. - 6. * delta_sine));
        }
    }
    let reversed = circular_miter([path[2], path[1], path[0]], 2., TOL).unwrap();
    close(shell.volume_mm3(), reversed.volume_mm3());
    for (a, b) in shell
        .bounds_mm()
        .into_iter()
        .flatten()
        .zip(reversed.bounds_mm().into_iter().flatten())
    {
        close(a, b);
    }
}

#[test]
fn mesh_shares_the_miter_samples_and_rejects_small_budgets() {
    let shell = circular_miter(RIGHT_ANGLE, 2., TOL).unwrap();
    let options = Tessellation {
        deflection: 0.01,
        angle: 1.,
        triangle_limit: 10_000,
    };
    let circle_steps = options.arc_segments(2., FRAC_PI_2).unwrap();
    let ellipse_steps = options
        .arc_segments(2. * std::f64::consts::SQRT_2, FRAC_PI_2)
        .unwrap();
    assert!(ellipse_steps > circle_steps);
    let mesh = shell.mesh(options).unwrap();
    assert_eq!(mesh.face_indices.len(), 24 * ellipse_steps - 4);
    let vertex = |index: u32| {
        let start = index as usize * 3;
        mesh.vertices[start..start + 3].try_into().unwrap()
    };
    let mut incidence = BTreeMap::<[u32; 2], [u32; 2]>::new();
    let mut volume = 0.;
    for triangle in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        volume += dot(vertex(a), cross(vertex(b), vertex(c))) / 6.;
        for (from, to) in [(a, b), (b, c), (c, a)] {
            assert_ne!(from, to);
            incidence.entry([from.min(to), from.max(to)]).or_default()[usize::from(from < to)] += 1;
        }
    }
    assert!(incidence.values().all(|counts| *counts == [1, 1]));
    assert!(volume > 0.992 * shell.volume_mm3() && volume < shell.volume_mm3());
    assert_eq!(
        shell.mesh(Tessellation {
            triangle_limit: 2,
            ..options
        }),
        Err(MeshError::BudgetExceeded)
    );
}

#[test]
fn rejects_invalid_collapsed_and_unresolved_paths() {
    for (path, radius, tolerance) in [
        (RIGHT_ANGLE, 2., f64::NAN),
        (RIGHT_ANGLE, f64::INFINITY, TOL),
        (RIGHT_ANGLE, TOL, TOL),
        (
            [[f64::NAN, 0., 0.], RIGHT_ANGLE[1], RIGHT_ANGLE[2]],
            2.,
            TOL,
        ),
        ([[0.; 3], [0.; 3], RIGHT_ANGLE[2]], 2., TOL),
    ] {
        assert_eq!(circular_miter(path, radius, tolerance), Err(Error::Invalid));
    }
    for path in [
        [[0.; 3], [0., 0., 20.], [0., 0., 30.]],
        [[0.; 3], [0., 0., 20.], [0., 0., 10.]],
        [[0.; 3], [0., 0., 20.], [1e-7, 0., 40.]],
        [[0.; 3], [0., 0., 20.], [2., 0., 20.]],
    ] {
        assert_eq!(circular_miter(path, 2., TOL), Err(Error::Unsupported));
    }
    for (path, radius) in [
        (RIGHT_ANGLE, 2. * MAX_COORDINATE_MM),
        (RIGHT_ANGLE, MAX_COORDINATE_MM),
        (
            RIGHT_ANGLE.map(|[x, y, z]| [x + 2. * MAX_COORDINATE_MM, y, z]),
            2.,
        ),
        (
            RIGHT_ANGLE.map(|[x, y, z]| [x, y + MAX_COORDINATE_MM, z]),
            2.,
        ),
    ] {
        assert_eq!(circular_miter(path, radius, TOL), Err(Error::Limit));
    }
    let collapsed_rim = RIGHT_ANGLE.map(|point| point.map(|x| x + 8e11));
    assert_eq!(
        circular_miter(collapsed_rim, 1e-6, TOL),
        Err(Error::Unsupported)
    );
    let large = 1e9;
    let oblique =
        [[0., 0., 0.], [0., 0., 20.], [12., 0., 36.]].map(|point| point.map(|x| x + large));
    assert_eq!(
        circular_miter(oblique, 1.001, 6e-8),
        Err(Error::Unsupported)
    );
}

#[test]
fn refuses_open_and_disconnected_loop_incidence() {
    let shell = circular_miter(RIGHT_ANGLE, 2., TOL).unwrap();
    let mut bad = shell.clone();
    bad.faces.pop();
    assert_eq!(prove_incidence(&bad), Err(Error::Invalid));
    let mut bad = shell;
    bad.faces[0].boundary.swap(0, 1);
    assert_eq!(prove_incidence(&bad), Err(Error::Invalid));
}

#[test]
fn cuts_through_the_joint_center_without_moving_the_plane_or_the_solid() {
    let shell = circular_miter(RIGHT_ANGLE, 2., TOL).unwrap();
    let origin = RIGHT_ANGLE[1];
    let normal = [0., 0., 1.];
    let loops = shell.section(origin, normal, TOL, 32).unwrap();
    let [boundary] = loops.as_slice() else {
        panic!("the rectangular and semicircular areas share one boundary");
    };
    let mut straight_length = 0.;
    let mut curved_sweep = 0.;
    for (edge, next) in boundary.iter().zip(boundary.iter().cycle().skip(1)) {
        let [a, b] = edge.ends_mm;
        close(a[2], origin[2]);
        close(length3(sub(b, next.ends_mm[0])), 0.);
        match edge.curve {
            Curve::Line => straight_length += length3(sub(a, b)),
            Curve::Ellipse(arc) => {
                for radius in arc.ellipse().semiaxes_mm() {
                    close(radius, 2.);
                }
                curved_sweep += arc.sweep_rad().abs();
            }
        }
    }
    close(straight_length, 44.);
    close(curved_sweep, PI);
    close(trim(&shell, 4).start_rad(), 0.);
    assert_eq!(shell.section(origin, normal, TOL, 1), Err(Error::Limit));
}

#[test]
fn repartition_does_not_certify_a_physical_tangent_or_cap_coincidence() {
    let shell = circular_miter(RIGHT_ANGLE, 2., TOL).unwrap();
    for (origin, normal) in [
        ([0., 2., 0.], [0., 1., 0.]),
        ([0.; 3], [0., 0., 1.]),
        (RIGHT_ANGLE[1], [1., 0., 1.]),
    ] {
        assert_eq!(
            shell.section(origin, normal, TOL, 32),
            Err(Error::Unsupported)
        );
    }
    let mut altered = shell.clone();
    altered.faces.pop();
    assert_eq!(
        altered.section(RIGHT_ANGLE[1], [0., 0., 1.], TOL, 32),
        Err(Error::Unsupported)
    );
    let mut altered = shell;
    altered.edges[4].curve = Curve::Line;
    assert_eq!(
        altered.section(RIGHT_ANGLE[1], [0., 0., 1.], TOL, 32),
        Err(Error::Unsupported)
    );

    // The shortest generator is still positive at construction tolerance,
    // but smaller than the section tolerance. Repartition cannot certify it.
    let bend_z = 2. + 5e-8;
    let path = [[0.; 3], [0., 0., bend_z], [20., 0., bend_z]];
    let tight = circular_miter(path, 2., 1e-10).unwrap();
    assert_eq!(
        tight.section(path[1], [0., 0., 1.], TOL, 32),
        Err(Error::Unsupported)
    );
}
