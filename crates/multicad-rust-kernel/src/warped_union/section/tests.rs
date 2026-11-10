use super::*;
use crate::normal_sweep::Edge;

const TOL: f64 = 1e-7;
const X: Point = [1., 0., 0.];
const Z: Point = [0., 0., 1.];

fn rectangle<const N: usize>(corners: [[f64; 2]; N]) -> [(Edge, usize); N] {
    std::array::from_fn(|i| {
        (
            Edge::Line {
                a: corners[i],
                b: corners[(i + 1) % N],
            },
            i,
        )
    })
}

pub(in crate::warped_union) fn fixture(up: f64) -> Shell {
    let web = rectangle([[0., 0.], [2., 0.], [8., 0.], [10., 0.], [10., 8.], [0., 8.]]).map(
        |(edge, source)| SheetWebEdge {
            ends: [edge.start(), edge.end()],
            source,
            flange: (source == 1).then_some(0),
        },
    );
    let arc = |a, b, clockwise| Edge::Arc {
        center: [0., -1.],
        a,
        b,
        clockwise,
    };
    let line = |a, b| Edge::Line { a, b };
    let section = [
        arc([0., 0.], [1., -1.], true),
        line([1., -1.], [1., -3.]),
        line([1., -3.], [2., -3.]),
        line([2., -3.], [2., -1.]),
        arc([2., -1.], [0., 1.], false),
        line([0., 1.], [0., 0.]),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, edge)| (edge, 10 + i))
    .collect::<Vec<_>>();
    let opening = |x, z, outline| SheetFlangeOpening {
        walls: [1, 3],
        origin_mm: [x, -1., z * up],
        u: X,
        v: [0., 0., up],
        through_mm: [0., -1., 0.],
        outline,
    };
    let rect = rectangle([[0., 0.], [1.5, 0.], [1.5, 0.6], [0., 0.6]]);
    let circle = [(
        Edge::Circle {
            center: [0.; 2],
            radius_mm: 0.3,
            clockwise: false,
        },
        80,
    )];
    let web_circle = [(
        Edge::Circle {
            center: [5., 4.],
            radius_mm: 1.,
            clockwise: true,
        },
        81,
    )];
    formed_sheet_with_openings(
        &web,
        &[web_circle.to_vec()],
        [20, 21],
        &[SheetFlange {
            section: &section,
            ends: [30, 31],
        }],
        &[&[opening(3., -2.2, &rect), opening(6.5, -2., &circle)]],
        up,
        TOL,
        1e-5,
    )
    .unwrap()
}

fn area(loops: &[Vec<SectionEdge>], normal: Point) -> f64 {
    loops
        .iter()
        .flat_map(|wire| {
            for (current, next) in wire.iter().zip(wire.iter().cycle().skip(1)) {
                assert_eq!(current.ends_mm[1], next.ends_mm[0]);
            }
            wire
        })
        .map(|edge| {
            let [a, b] = edge.ends_mm;
            match edge.curve {
                Curve::Line => dot(cross(a, b), normal) / 2.,
                Curve::Ellipse(arc) => {
                    let ellipse = arc.ellipse();
                    let [r, s] = ellipse.semiaxes_mm();
                    let center = ellipse.center_mm();
                    (dot(cross(center, sub(b, a)), normal)
                        + r * s * arc.sweep_rad() * dot(ellipse.normal(), normal))
                        / 2.
                }
            }
        })
        .sum()
}

fn circular_miter_fixture() -> Shell {
    circular_miter([[0.; 3], [0., 0., 20.], [20., 0., 20.]], 2., TOL).unwrap()
}

fn disk_segment(offset: f64) -> f64 {
    4. * (offset / 2.).acos() - offset * (4. - offset * offset).sqrt()
}

#[test]
fn transverse_miter_sections_keep_the_shared_ellipse_crossings() {
    let shell = circular_miter_fixture();
    let straight = shell.section([0., 0., 10.], Z, TOL, 100).unwrap();
    assert_eq!(straight.len(), 1);
    assert!((area(&straight, Z) - 4. * std::f64::consts::PI).abs() < TOL);

    let half_width = 3.75_f64.sqrt();
    let loops = shell.section([0., 0., 20.5], Z, TOL, 100).unwrap();
    assert_eq!(loops.len(), 1);
    let expected = disk_segment(0.5) + 2. * 20.5 * half_width;
    assert!((area(&loops, Z) - expected).abs() < TOL);
    for y in [-half_width, half_width] {
        assert!(
            loops[0]
                .iter()
                .any(|edge| { length3(sub(edge.ends_mm[0], [-0.5, y, 20.5])) < TOL })
        );
    }
}

#[test]
fn oblique_miter_section_joins_two_exact_ellipse_segments() {
    let shell = circular_miter_fixture();
    let normal = unit([0.5, 0., 1.]).unwrap();
    let loops = shell.section([0., 0., 20.3], normal, TOL, 100).unwrap();
    assert_eq!(loops.len(), 1);
    let first = disk_segment(0.6);
    let second = 4. * std::f64::consts::PI - first;
    let expected = first * 1.25_f64.sqrt() + second * 5_f64.sqrt();
    assert!((area(&loops, normal) - expected).abs() < TOL);
    let mut joins = 0;
    for edge in &loops[0] {
        let Curve::Ellipse(arc) = edge.curve else {
            panic!("only the two cylinder walls cross this section");
        };
        let [major, minor] = arc.ellipse().semiaxes_mm();
        assert!((minor - 2.).abs() < TOL);
        assert!(
            (major - 5_f64.sqrt())
                .abs()
                .min((major - 2. * 5_f64.sqrt()).abs())
                < TOL
        );
        let [x, _, z] = edge.ends_mm[0];
        assert!((z + 0.5 * x - 20.3).abs() < TOL);
        joins += usize::from((x + z - 20.).abs() < TOL);
    }
    assert_eq!(joins, 2);
}

#[test]
fn multi_corner_sections_intersect_a_leg_with_two_different_tilted_rims() {
    let path = [[0.; 3], [0., 0., 20.], [20., 0., 20.], [20., 20., 20.]];
    let shell = circular_miter_chain(&path, 2., TOL).unwrap();
    let normal = unit([1., 0., 0.5]).unwrap();
    let loops = shell.section([10., 0., 20.], normal, TOL, 100).unwrap();
    assert_eq!(loops.len(), 1);
    assert!((area(&loops, normal) - 4. * std::f64::consts::PI * 1.25_f64.sqrt()).abs() < TOL);
    for edge in &loops[0] {
        let Curve::Ellipse(arc) = edge.curve else {
            panic!("the oblique cut crosses only the middle cylinder");
        };
        let [major, minor] = arc.ellipse().semiaxes_mm();
        assert!((major - 5_f64.sqrt()).abs() < TOL);
        assert!((minor - 2.).abs() < TOL);
    }

    let horizontal = shell.section([0., 0., 20.5], Z, TOL, 100).unwrap();
    assert_eq!(horizontal.len(), 1);
    // The cut retains a circular segment and two joined strips. The
    // slanted joint contributes opposite area changes to the two strips.
    let expected = disk_segment(0.5) + 2. * 3.75_f64.sqrt() * 40.5;
    assert!((area(&horizontal, Z) - expected).abs() < TOL);
}

#[test]
fn a_cylinder_chart_rejects_unverified_or_misplaced_tilted_rims() {
    let mut shell = circular_miter_fixture();
    let face = shell.faces[1].clone();
    let [cap, _, trim, _] = *face.boundary.as_slice() else {
        panic!("expected a ruled cylinder sector");
    };
    let inspect = |shell: &Shell| CylinderPatch::new(shell, &face, [0.; 3], Z, 2., TOL);
    let certificate = shell.matched_quarter_rims.take();
    assert!(matches!(inspect(&shell), Err(Error::Unsupported)));
    shell.matched_quarter_rims = certificate;
    let patch = inspect(&shell).unwrap();
    let malformed = CylinderPatch {
        arcs: [patch.arcs[1], patch.arcs[0]],
        rails: patch.rails,
    };
    assert_eq!(
        malformed.radial_chart([0., 0., 2.]).err(),
        Some(Error::Unsupported)
    );
    shell.edges[cap.edge].curve = shell.edges[trim.edge].curve;
    assert!(matches!(inspect(&shell), Err(Error::Unsupported)));
    let invalid = Ellipse::new([0.; 3], Z, X, [3., 2.]).unwrap();
    shell.edges[cap.edge].curve = Curve::Ellipse(EllipseArc::new(invalid, 0., FRAC_PI_2).unwrap());
    assert!(matches!(inspect(&shell), Err(Error::Unsupported)));
}

#[test]
fn section_follows_exact_bends_and_skips_web_and_flange_openings() {
    for up in [-1., 1.] {
        let shell = fixture(up);
        for normal in [X, [-1., 0., 0.]] {
            let uncut = 10. + 3. * std::f64::consts::FRAC_PI_4;
            let web_gap = 2. * (1_f64 - 0.3_f64.powi(2)).sqrt();
            let round_gap = 2. * (0.3_f64.powi(2) - 0.15_f64.powi(2)).sqrt();
            for (x, expected, count, arc_count) in [
                (0.5, 8., 1, 0),
                (3.5, uncut - 0.6, 2, 2),
                (5.3, uncut - web_gap, 2, 2),
                (6.65, uncut - round_gap, 2, 2),
            ] {
                let loops = shell.section([x, 0., 0.], normal, TOL, 100).unwrap();
                assert_eq!(loops.len(), count);
                assert_eq!(
                    loops
                        .iter()
                        .flatten()
                        .filter(|e| matches!(e.curve, Curve::Ellipse(_)))
                        .count(),
                    arc_count
                );
                assert!(
                    (area(&loops, normal) - expected).abs() < TOL,
                    "{x}: {} != {expected}",
                    area(&loops, normal)
                );
            }
        }
    }
}

#[test]
fn section_keeps_closed_circular_web_hole_in_a_face_on_cut() {
    let shell = fixture(1.);
    let loops = shell.section([0., 0., 0.5], Z, TOL, 100).unwrap();
    assert_eq!(loops.len(), 2);
    assert!((area(&loops, Z) - (80. - std::f64::consts::PI + 6. * 1.75_f64.sqrt())).abs() < TOL);
}

#[test]
fn sections_reject_unresolved_contact_ill_conditioned_cuts_and_excess_work() {
    let shell = fixture(1.);
    assert_eq!(shell.section([-1., 0., 0.], X, TOL, 100), Ok(vec![]));
    assert_eq!(shell.section([3.5, 0., 0.], X, TOL, 1), Err(Error::Limit));
    assert_eq!(
        shell.section([3.5, 0., 0.], X, f64::NAN, 100),
        Err(Error::Invalid)
    );
    assert_eq!(
        shell.section([f64::NAN, 0., 0.], X, TOL, 100),
        Err(Error::Invalid)
    );
    assert_eq!(
        shell.section([3.5, 0., 0.], [0.; 3], TOL, 100),
        Err(Error::Invalid)
    );
    assert_eq!(
        shell.section([2., 0., 0.], X, TOL, 100),
        Err(Error::Unsupported)
    );
    assert_eq!(
        shell.section([0., -1.4, 0.], [1e-13, -1., 1.], TOL, 100),
        Err(Error::Unsupported)
    );
}

#[test]
fn oblique_section_preserves_bend_ellipses_and_the_exact_web_bore_chord() {
    let slope = 0.05;
    let x = 4.73;
    let stretch = 1_f64.hypot(slope);
    let distance = (5. + slope * 4. - x) / stretch;
    let expected =
        (10. + 3. * std::f64::consts::FRAC_PI_4) * stretch - 2. * (1. - distance * distance).sqrt();
    for up in [-1., 1.] {
        let shell = fixture(up);
        for sign in [-1., 1.] {
            let normal = unit([sign, sign * slope, 0.]).unwrap();
            let loops = shell.section([x, 0., 0.], normal, TOL, 100).unwrap();
            assert_eq!(loops.len(), 2);
            assert!((area(&loops, normal) - expected).abs() < TOL);
            let arcs: Vec<_> = loops
                .iter()
                .flatten()
                .filter_map(|edge| match edge.curve {
                    Curve::Ellipse(arc) => Some(arc),
                    Curve::Line => None,
                })
                .collect();
            assert_eq!(arcs.len(), 2);
            for arc in arcs {
                let [major, minor] = arc.ellipse().semiaxes_mm();
                assert!(major > minor);
                assert!((major / minor - stretch).abs() < TOL);
                assert!((arc.sweep_rad().abs() - FRAC_PI_2).abs() < TOL);
            }
        }
    }
}

#[test]
fn oblique_bore_arcs_terminate_on_actual_rims_even_with_no_rail_crossing() {
    let shell = fixture(1.);
    let threshold = 1.21;
    for (point, normal, expected, root_count) in [
        (
            [5., 4., 0.25],
            [0.5, 0., 1.],
            4. * std::f64::consts::PI / 3.,
            2,
        ),
        (
            [5., 4., -threshold],
            [-1., -1., 1.],
            2. * (threshold / std::f64::consts::SQRT_2).acos(),
            2,
        ),
    ] {
        let normal = unit(normal).unwrap();
        let loops = shell.section(point, normal, TOL, 100).unwrap();
        area(&loops, normal);
        let arcs: Vec<_> = loops
            .iter()
            .flatten()
            .filter_map(|edge| match edge.curve {
                Curve::Ellipse(arc) if length3(sub(arc.ellipse().center_mm(), point)) < TOL => {
                    Some((edge.ends_mm, arc))
                }
                _ => None,
            })
            .collect();
        let mut roots = 0;
        let mut sweep = 0.;
        for (ends, arc) in &arcs {
            assert!(arc.sweep_rad() < 0.);
            sweep -= arc.sweep_rad();
            for (p, q) in ends.iter().zip(arc.endpoints()) {
                assert!(length3(sub(*p, q)) < TOL);
                assert!(dot(sub(*p, point), normal).abs() < TOL);
                assert!(((p[0] - 5.).hypot(p[1] - 4.) - 1.).abs() < TOL);
                assert!((-TOL..1. + TOL).contains(&p[2]));
                roots += usize::from(p[2].abs() < TOL);
            }
        }
        assert!((sweep - expected).abs() < TOL, "{sweep} != {expected}");
        assert_eq!(roots, root_count);
        if point[2] < 0. {
            assert_eq!(arcs.len(), 1, "Both trim roots lie on one cap quadrant");
        }
    }
}

#[test]
fn bend_extrema_outside_vertex_range_are_still_cut() {
    let normal = unit([0., -1., 1.]).unwrap();
    let loops = fixture(1.)
        .section([0., -1.4, 0.], normal, TOL, 100)
        .unwrap();
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].len(), 4);
    assert!((area(&loops, normal) - 12. * 1.12_f64.sqrt()).abs() < TOL);
}

#[test]
fn an_interior_tangency_and_a_coincident_arc_need_contact_resolution() {
    let ellipse = Ellipse::new([0.; 3], Z, X, [1.; 2]).unwrap();
    let arc = EllipseArc::new(ellipse, 0.2, 0.8).unwrap();
    for (t, supported) in [(0.5, false), (1.5, true)] {
        let point = ellipse.point_at(t);
        assert_eq!(
            arc_hits(arc, Plane::new(point, point).unwrap(), TOL),
            if supported {
                Ok(vec![])
            } else {
                Err(Error::Unsupported)
            }
        );
    }
    assert_eq!(
        arc_hits(arc, Plane::new([0.; 3], Z).unwrap(), TOL),
        Err(Error::Unsupported)
    );
}

fn section_state(shell: &Shell, normal: Point) -> Section {
    Section {
        origin: [3.5, 0., 0.],
        normal,
        tolerance: TOL,
        limit: 10,
        hits: vec![Vec::new(); shell.edges.len()],
        edges: BTreeMap::new(),
    }
}

#[test]
fn only_paired_cap_arcs_and_axial_rails_define_a_cylinder_chart() {
    let mut shell = fixture(1.);
    let face = &shell.faces[0];
    let Support::Cylinder {
        axis_origin_mm: origin,
        axis_direction: axis,
        radius_mm: radius,
        ..
    } = face.support
    else {
        panic!("The first face is a flange bend");
    };
    let mut altered = face.clone();
    altered.boundary.pop();
    assert!(matches!(
        CylinderPatch::new(&shell, &altered, origin, axis, radius, TOL),
        Err(Error::Unsupported)
    ));
    let arc = face.boundary[0].edge;
    let rail = face.boundary[1].edge;
    let Curve::Ellipse(original) = shell.edges[arc].curve else {
        panic!("Bend arc")
    };
    shell.edges[arc].curve = Curve::Line;
    assert!(matches!(
        CylinderPatch::new(&shell, face, origin, axis, radius, TOL),
        Err(Error::Unsupported)
    ));
    shell.edges[arc].curve = Curve::Ellipse(original);
    assert!(matches!(
        CylinderPatch::new(&shell, face, origin, axis, radius + 1., TOL),
        Err(Error::Unsupported)
    ));
    shell.edges[rail].curve = Curve::Ellipse(original);
    assert!(matches!(
        CylinderPatch::new(&shell, face, origin, axis, radius, TOL),
        Err(Error::Unsupported)
    ));
}

#[test]
fn inconsistent_face_intersections_fail_closed() {
    let shell = fixture(1.);
    let face = &shell.faces[0];
    let Support::Cylinder {
        axis_origin_mm: origin,
        axis_direction: axis,
        radius_mm: radius,
        outward,
    } = face.support
    else {
        panic!("The first face is a flange bend");
    };
    let hit = |edge, point| Hit {
        port: [edge, 0],
        point,
    };
    let [a, b] = [face.boundary[1].edge, face.boundary[3].edge];
    let mut state = section_state(&shell, X);
    state.hits[a].push(hit(a, [3.5, 100., 100.]));
    assert_eq!(
        state.cylinder(&shell, face, origin, axis, radius, outward),
        Err(Error::Unsupported)
    );
    state.hits[b].push(hit(b, [3.5, 101., 101.]));
    assert_eq!(
        state.cylinder(&shell, face, origin, axis, radius, outward),
        Err(Error::Unsupported)
    );
    state.normal = unit([1., 1., 0.]).unwrap();
    assert_eq!(
        state.cylinder(&shell, face, origin, axis, radius, outward),
        Err(Error::Unsupported)
    );
    state.normal = Z;
    let arc = face.boundary[0].edge;
    state.hits[arc].push(hit(arc, [2., 0., 0.]));
    assert_eq!(
        state.cylinder(&shell, face, origin, axis, radius, outward),
        Err(Error::Unsupported)
    );

    let mut state = section_state(&shell, X);
    state.hits[a].push(hit(a, [3.5, 2., 0.]));
    assert_eq!(state.planar(face, Z), Err(Error::Unsupported));
    assert_eq!(
        state.add([hit(a, [0.; 3]), hit(b, [0.; 3])], Curve::Line),
        Err(Error::Unsupported)
    );
    state
        .add([hit(a, [0.; 3]), hit(b, [0., 1., 0.])], Curve::Line)
        .unwrap();
    assert_eq!(
        state.add([hit(a, [0.; 3]), hit(b, [0., 1., 0.])], Curve::Line),
        Err(Error::Unsupported)
    );
    assert_eq!(state.loops(), Err(Error::Unsupported));
}

#[test]
fn oblique_section_refuses_inconsistent_trim_ports() {
    let shell = fixture(1.);
    let face = &shell.faces[0];
    let Support::Cylinder {
        axis_origin_mm: origin,
        axis_direction: axis,
        radius_mm: radius,
        outward,
    } = face.support
    else {
        panic!("The first face is a flange bend");
    };
    let patch = CylinderPatch::new(&shell, face, origin, axis, radius, TOL).unwrap();
    let (cap, reference) = patch.arcs[0];
    let [first, last] = patch.rails;
    let span = reference.sweep_rad().abs();
    let slip = 3. * TOL / radius;
    for (shift, samples) in [
        // A cap hit cannot also lie on a chart rail.
        (0., &[(cap, 0.)][..]),
        // One port or two coincident ports cannot bound an arc.
        (0., &[(first, 0.)][..]),
        (0., &[(first, 0.), (last, 0.)][..]),
        // The infinite ellipse does not lie within these finite cap levels.
        (30., &[(first, 0.), (last, span)][..]),
        // Both points can snap individually, but their ellipse sweep no
        // longer matches the certified angular span between the rails.
        (0., &[(first, slip), (last, span - slip)][..]),
    ] {
        let mut state = section_state(&shell, unit([1., 0.05, 0.]).unwrap());
        state.origin = translate([5., 0., 0.], axis, shift);
        for &(edge, phase) in samples {
            let angle = reference.start_rad() + reference.sweep_rad().signum() * phase;
            let point = reference.ellipse().point_at(angle);
            let level = dot(sub(state.origin, point), state.normal) / dot(axis, state.normal);
            state.hits[edge].push(Hit {
                port: [edge, 0],
                point: translate(point, axis, level),
            });
        }
        assert_eq!(
            state.cylinder(&shell, face, origin, axis, radius, outward),
            Err(Error::Unsupported),
            "shift {shift}, samples {samples:?}"
        );
    }
}
