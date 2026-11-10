use super::*;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

const TOL: f64 = 1e-7;
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const Z: Point = [0., 0., 1.];

fn line(a: Point, b: Point) -> SectionEdge {
    SectionEdge {
        ends_mm: [a, b],
        curve: Curve::Line,
    }
}

fn polygon(points: &[Point]) -> Vec<SectionEdge> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| line(a, b))
        .collect()
}

fn plane(origin_mm: Point, inward_normal: Point) -> SectionClipPlane {
    SectionClipPlane {
        origin_mm,
        inward_normal,
    }
}

fn arc(ends: [Point; 2], radii: [f64; 2], start: f64, sweep: f64) -> SectionEdge {
    let ellipse = Ellipse::new([0.; 3], Z, X, radii).unwrap();
    SectionEdge {
        ends_mm: ends,
        curve: Curve::Ellipse(EllipseArc::new(ellipse, start, sweep).unwrap()),
    }
}

fn area(loops: &[Vec<SectionEdge>]) -> f64 {
    loops
        .iter()
        .flat_map(|loop_| {
            for (edge, next) in loop_.iter().zip(loop_.iter().cycle().skip(1)) {
                assert_eq!(edge.ends_mm[1], next.ends_mm[0]);
            }
            loop_
        })
        .map(|edge| {
            let [a, b] = edge.ends_mm;
            match edge.curve {
                Curve::Line => cross(a, b)[2] / 2.,
                Curve::Ellipse(arc) => {
                    let ellipse = arc.ellipse();
                    let [a_len, b_len] = ellipse.semiaxes_mm();
                    (cross(ellipse.center_mm(), sub(b, a))[2]
                        + a_len * b_len * arc.sweep_rad() * ellipse.normal()[2])
                        / 2.
                }
            }
        })
        .sum()
}

fn square_with_opening() -> Vec<Vec<SectionEdge>> {
    vec![
        polygon(&[[0., 0., 0.], [10., 0., 0.], [10., 8., 0.], [0., 8., 0.]]),
        polygon(&[[3., 3., 0.], [3., 5., 0.], [7., 5., 0.], [7., 3., 0.]]),
    ]
}

#[test]
fn a_lateral_window_opens_the_rectangular_void_without_covering_it() {
    let left = plane([5., 0., 0.], X);
    let loops = clip_section_loops(square_with_opening(), Z, &[left], TOL, 20).unwrap();
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].len(), 8);
    assert!((area(&loops) - 36.).abs() < TOL);
    for endpoints in [[[5., 8., 0.], [5., 5., 0.]], [[5., 3., 0.], [5., 0., 0.]]] {
        assert!(loops[0].contains(&SectionEdge {
            ends_mm: endpoints,
            curve: Curve::Line,
        }));
    }
    let window = [
        left,
        plane([9., 0., 0.], [-1., 0., 0.]),
        plane([0., 0.5, 0.], Y),
        plane([0., 7.5, 0.], [0., -1., 0.]),
    ];
    let loops = clip_section_loops(square_with_opening(), Z, &window, TOL, 20).unwrap();
    assert_eq!(loops.len(), 1);
    assert!((area(&loops) - 24.).abs() < TOL);
    assert!(
        loops[0]
            .iter()
            .flat_map(|edge| edge.ends_mm)
            .all(|p| { (5. ..=9.).contains(&p[0]) && (0.5..=7.5).contains(&p[1]) })
    );
}

#[test]
fn a_partial_radius_three_bend_keeps_the_exact_trim_and_adds_a_directed_wall() {
    let loops = vec![vec![
        line([1.5, 0., 0.], [3., 0., 0.]),
        arc([[3., 0., 0.], [0., 3., 0.]], [3.; 2], 0., FRAC_PI_2),
        line([0., 3., 0.], [0., 1.5, 0.]),
        arc(
            [[0., 1.5, 0.], [1.5, 0., 0.]],
            [1.5; 2],
            FRAC_PI_2,
            -FRAC_PI_2,
        ),
    ]];
    let cut = plane([2., 0., 0.], [-1., 0., 0.]);
    let loops = clip_section_loops(loops, Z, &[cut], TOL, 20).unwrap();
    assert_eq!(loops.len(), 1);
    let root = 5_f64.sqrt();
    let expected_area = (2. * root + 9. * (2_f64 / 3.).asin()) / 2. - 2.25 * FRAC_PI_4;
    assert!((area(&loops) - expected_area).abs() < TOL);
    let generated_wall = loops[0]
        .iter()
        .find(|edge| edge.curve == Curve::Line && edge.ends_mm.iter().all(|p| p[0] == 2.))
        .unwrap();
    assert_eq!(generated_wall.ends_mm[0], [2., 0., 0.]);
    assert!((generated_wall.ends_mm[1][1] - root).abs() < TOL);
    let outer_trim = loops[0]
        .iter()
        .find_map(|edge| match edge.curve {
            Curve::Ellipse(arc) if arc.ellipse().semiaxes_mm() == [3.; 2] => Some(arc),
            _ => None,
        })
        .unwrap();
    assert!((outer_trim.start_rad() - (2_f64 / 3.).acos()).abs() < TOL);
    assert!((outer_trim.start_rad() + outer_trim.sweep_rad() - FRAC_PI_2).abs() < TOL);
}

fn closed_ellipse() -> Vec<Vec<SectionEdge>> {
    vec![vec![arc([[3., 0., 0.]; 2], [3., 2.], 0., TAU)]]
}

#[test]
fn genuine_ellipse_is_retained_on_both_sides_of_its_parameter_seam() {
    let cut = plane([1., 0., 0.], X);
    let loops = clip_section_loops(closed_ellipse(), Z, &[cut], TOL, 5).unwrap();
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].len(), 3);
    let ratio = 1_f64 / 3.;
    let expected = 6. * (ratio.acos() - ratio * (1. - ratio * ratio).sqrt());
    assert!((area(&loops) - expected).abs() < TOL);
    assert_eq!(
        clip_section_loops(closed_ellipse(), Z, &[cut], TOL, 1),
        Err(Error::Limit),
    );
}

#[test]
fn enclosing_and_disjoint_planes_do_not_change_the_other_paths() {
    let loops = square_with_opening();
    assert_eq!(
        clip_section_loops(loops.clone(), Z, &[plane([-1., 0., 0.], X)], TOL, 20).unwrap(),
        loops,
    );
    assert!(
        clip_section_loops(loops, Z, &[plane([20., 0., 0.], X)], TOL, 20)
            .unwrap()
            .is_empty()
    );
    assert!(
        clip_section_loops(vec![], Z, &[plane([1., 0., 0.], X)], TOL, 20)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unresolved_tangency_vertex_contact_and_bad_inputs_fail_closed() {
    assert_eq!(
        clip_section_loops(closed_ellipse(), Z, &[plane([-3., 0., 0.], X)], TOL, 20),
        Err(Error::Unsupported),
    );
    assert_eq!(
        clip_section_loops(square_with_opening(), Z, &[plane([3., 0., 0.], X)], TOL, 20),
        Err(Error::Unsupported),
    );
    assert_eq!(
        clip_section_loops(closed_ellipse(), Z, &[plane([0.; 3], Z)], TOL, 20),
        Err(Error::Invalid),
    );
    let open = vec![vec![line([0.; 3], X)]];
    assert_eq!(
        clip_section_loops(open, Z, &[plane([0.; 3], X)], TOL, 20),
        Err(Error::Invalid),
    );
    assert_eq!(
        clip_section_loops(closed_ellipse(), Z, &[], f64::NAN, 20),
        Err(Error::Invalid),
    );
}

#[test]
fn input_limits_and_inconsistent_conic_ports_are_rejected() {
    let cut = plane([1., 0., 0.], X);
    assert_eq!(
        clip_section_loops(closed_ellipse(), Z, &[cut; MAX_PLANES + 1], TOL, 20),
        Err(Error::Limit),
    );
    assert_eq!(
        clip_section_loops(vec![vec![]], Z, &[cut], TOL, 20),
        Err(Error::Invalid),
    );
    let mut ellipse = closed_ellipse();
    ellipse[0][0].ends_mm = [[2.5, 0., 0.]; 2];
    assert_eq!(
        clip_section_loops(ellipse, Z, &[cut], TOL, 20),
        Err(Error::Invalid),
    );
}

#[test]
fn close_crossings_short_material_spans_and_reversed_ports_fail_closed() {
    let rectangle = |x, [a, b]: [f64; 2]| {
        polygon(&[[-1., a, 0.], [x, a, 0.], [x, b, 0.], [-1., b, 0.]])
    };
    let cut = [plane([0.; 3], X)];
    let close = vec![rectangle(1., [0., 2.]), rectangle(1., [2. + TOL / 2., 4.])];
    assert_eq!(
        clip_section_loops(close, Z, &cut, TOL, 20),
        Err(Error::Unsupported),
    );
    let narrow = vec![rectangle(1.5 * TOL, [0., 2.])];
    assert_eq!(
        clip_section_loops(narrow, Z, &cut, TOL, 20),
        Err(Error::Unsupported),
    );
    let reversed = vec![polygon(&[
        [-1., 0., 0.],
        [-1., 2., 0.],
        [1., 2., 0.],
        [1., 0., 0.],
    ])];
    assert_eq!(
        clip_section_loops(reversed, Z, &cut, TOL, 20),
        Err(Error::Unsupported),
    );
}

#[test]
fn an_off_plane_conic_root_cannot_be_snapped_into_the_boundary() {
    let mut cut = Cut::new(plane([0.; 3], X), Z, TOL).unwrap();
    // Simulate an intersection result from a different support plane.
    cut.plane = Plane::new([1., 0., 0.], X).unwrap();
    let mut graph = Graph::new(8);
    let mut crossings = Vec::new();
    assert_eq!(
        cut.split(closed_ellipse()[0][0], &mut graph, &mut crossings),
        Err(Error::Unsupported),
    );
    assert!(crossings.is_empty());
}
