use super::*;
use std::f64::consts::{FRAC_PI_2, PI};

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const Z: Point = [0., 0., 1.];
const TOL: f64 = 1e-8;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < TOL, "{actual} != {expected}");
}

fn near_point(actual: Point, expected: Point) {
    for (a, b) in actual.into_iter().zip(expected) {
        near(a, b);
    }
}

fn on_cylinder(point: Point, cylinder: Cylinder) {
    near(
        length(cross(sub(point, cylinder.origin_mm), cylinder.axis)),
        cylinder.radius_mm,
    );
}

fn on_plane(point: Point, plane: Plane) {
    near(dot(sub(point, plane.origin_mm), plane.normal), 0.);
}

fn short_arc(ellipse: Ellipse, endpoints: [Point; 2]) -> EllipseArc {
    let ccw = EllipseArc::from_endpoints(ellipse, endpoints, false, TOL).unwrap();
    let cw = EllipseArc::from_endpoints(ellipse, endpoints, true, TOL).unwrap();
    let arc = if ccw.sweep_rad() < -cw.sweep_rad() {
        ccw
    } else {
        cw
    };
    near_point(arc.endpoints()[0], endpoints[0]);
    near_point(arc.endpoints()[1], endpoints[1]);
    arc
}

/// These are the twelve 3D ellipse trims in the reinforced drawer STEP
/// oracle, using the four finite miter planes and their native trim vertices.
#[test]
fn reproduces_all_twelve_drawer_miter_and_seam_ellipses() {
    for (cx, sx) in [(21., -1.), (213., 1.)] {
        for (cy, sy) in [(5., -1.), (110.5, 1.)] {
            let cz = 252.5;
            let corner = [cx, cy, cz];
            let crosspiece = Cylinder::new([19., cy, cz], X, 2.).unwrap();
            let side = Cylinder::new([cx, 3., cz], Y, 2.).unwrap();
            let normal = [sx, sy, 0.];
            let x_center = [cx + sx * 0.05, cy, cz];
            let y_center = [cx, cy + sy * 0.05, cz];
            let plane = Plane::new(x_center, normal).unwrap();
            let joint = [cx + sx * 0.025, cy + sy * 0.025, 250.5001562561];
            let x_end = [cx + sx * (0.05 - 2.), cy + sy * 2., cz];
            let y_end = [cx + sx * 2., cy + sy * (0.05 - 2.), cz];

            for (cylinder, center, end) in [(crosspiece, x_center, x_end), (side, y_center, y_end)]
            {
                let ellipse = cylinder_plane(cylinder, plane).unwrap();
                near_point(ellipse.center_mm(), center);
                near(ellipse.semiaxes_mm()[0], 2.828427124746);
                near(ellipse.semiaxes_mm()[1], 2.);
                near(dot(ellipse.normal(), plane.normal), 1.);
                let arc = short_arc(ellipse, [joint, end]);
                assert!(arc.sweep_rad().abs() < FRAC_PI_2);
                let middle = ellipse.point_at(arc.start_rad() + 0.5 * arc.sweep_rad());
                on_cylinder(middle, cylinder);
                on_plane(middle, plane);
            }
            let ellipses = equal_perpendicular_cylinders(crosspiece, side, TOL).unwrap();
            for ellipse in ellipses {
                near_point(ellipse.center_mm(), corner);
                near(ellipse.semiaxes_mm()[0], 2.828427124746);
                near(ellipse.semiaxes_mm()[1], 2.);
                for t in [0., FRAC_PI_2, 0.71, 2.4, 4.8] {
                    on_cylinder(ellipse.point_at(t), crosspiece);
                    on_cylinder(ellipse.point_at(t), side);
                }
            }
            let seam = ellipses
                .into_iter()
                .find(|ellipse| dot(ellipse.normal(), plane.normal).abs() < TOL)
                .unwrap();
            let arc = short_arc(seam, [[cx, cy, cz - 2.], joint]);
            assert!(arc.sweep_rad().abs() < 0.013);
            let PlaneCrossings::Crossing(crossings) = seam.plane_crossings(plane, TOL).unwrap()
            else {
                panic!("the miter plane cuts the common ellipse");
            };
            assert!(
                crossings
                    .into_iter()
                    .any(|t| length(sub(seam.point_at(t), joint)) < TOL)
            );
        }
    }
}

#[test]
fn oblique_frames_preserve_exact_geometry_and_derivatives() {
    let origin = [120., 70., -40.];
    let a = [0.8, 0.6, 0.];
    let b = [-0.36, 0.48, 0.8];
    let axial = |direction: Point, shift| {
        Cylinder::new(
            std::array::from_fn(|i| origin[i] + direction[i] * shift),
            direction.map(|x| 11. * x),
            3.5,
        )
        .unwrap()
    };
    let one = axial(a, 15.);
    let two = axial(b, -9.);
    for ellipse in equal_perpendicular_cylinders(one, two, TOL).unwrap() {
        near_point(ellipse.center_mm(), origin);
        near_point(
            cross(ellipse.major_axis(), ellipse.minor_axis()),
            ellipse.normal(),
        );
        for t in [0.2, PI, TAU - 0.01] {
            let point = ellipse.point_at(t);
            near(ellipse.parameter_at(point, TOL).unwrap(), t);
            on_cylinder(point, one);
            on_cylinder(point, two);
            let tangent = ellipse.tangent_at(t);
            near(dot(tangent, ellipse.normal()), 0.);
            let h = 1e-4;
            near_point(
                tangent,
                sub(ellipse.point_at(t + h), ellipse.point_at(t - h)).map(|d| d / (2. * h)),
            );
        }
    }
    let plane = Plane::new([118., 63., -38.], [1., 2., 3.]).unwrap();
    let ellipse = cylinder_plane(one, plane).unwrap();
    for t in [0.1, 1.3, 2.8, 5.1] {
        let point = ellipse.point_at(t);
        on_plane(point, plane);
        on_cylinder(point, one);
    }
}

#[test]
fn parameters_trim_both_directions_and_bound_actual_extrema() {
    let ellipse = Ellipse::new([4., 2., 7.], Z, X, [3., 2.]).unwrap();
    near(
        ellipse.parameter_at(ellipse.point_at(-0.3), TOL).unwrap(),
        TAU - 0.3,
    );
    assert!(ellipse.parameter_at([4., 2., 7.], TOL).is_none());
    assert!(ellipse.parameter_at([7., 2., 7.1], TOL).is_none());
    assert!(ellipse.parameter_at([7., 2., 7.], f64::NAN).is_none());
    assert!(ellipse.parameter_at([7., 2., 7.], 2.).is_none());

    let full = EllipseArc::new(ellipse, 1.2, -TAU).unwrap();
    assert_eq!(full.ellipse(), ellipse);
    assert_eq!(full.range_along(X), [1., 7.]);
    assert_eq!(full.range_along(Y), [0., 4.]);
    assert_eq!(full.range_along(Z), [7., 7.]);
    let half = EllipseArc::new(ellipse, -FRAC_PI_2, PI).unwrap();
    let [lo, hi] = half.range_along(X);
    near(lo, 4.);
    near(hi, 7.);
    assert_eq!(half.range_along(Y), [0., 4.]);
    let quadrant = EllipseArc::new(ellipse, 0., -FRAC_PI_2).unwrap();
    assert_eq!(quadrant.range_along(X), [4., 7.]);
    assert_eq!(quadrant.range_along(Y), [0., 2.]);
    assert_eq!(EllipseArc::new(ellipse, 0., 0.), Err(Error::Invalid));
    assert_eq!(
        EllipseArc::new(ellipse, f64::INFINITY, 1.),
        Err(Error::Invalid)
    );
    assert_eq!(EllipseArc::new(ellipse, 0., f64::NAN), Err(Error::Invalid));
    assert_eq!(
        EllipseArc::new(ellipse, 0., TAU.next_up()),
        Err(Error::Invalid)
    );
    let point = ellipse.point_at(0.);
    let end = ellipse.point_at(1.);
    assert_eq!(
        EllipseArc::from_endpoints(ellipse, [point, point], false, TOL),
        Err(Error::Invalid)
    );
    assert_eq!(
        EllipseArc::from_endpoints(ellipse, [[0.; 3], end], false, TOL),
        Err(Error::Invalid)
    );
    assert_eq!(
        EllipseArc::from_endpoints(ellipse, [point, [0.; 3]], false, TOL),
        Err(Error::Invalid)
    );
    near(angle(-f64::EPSILON), 0.);
}

#[test]
fn plane_crossings_distinguish_outside_tangent_and_coincident_planes() {
    let ellipse = Ellipse::new([4., 2., 7.], Z, X, [3., 2.]).unwrap();
    let cut = |origin, normal| {
        ellipse
            .plane_crossings(Plane::new(origin, normal).unwrap(), TOL)
            .unwrap()
    };
    assert_eq!(cut([0., 0., 7.], Z), PlaneCrossings::Coincident);
    assert_eq!(cut([0., 0., 8.], Z), PlaneCrossings::None);
    assert_eq!(cut([8., 0., 0.], X), PlaneCrossings::None);
    assert_eq!(cut([7., 0., 0.], X), PlaneCrossings::Tangent(0.));
    assert_eq!(cut([1., 0., 0.], X), PlaneCrossings::Tangent(PI));
    assert_eq!(
        cut([4., 0., 0.], X),
        PlaneCrossings::Crossing([FRAC_PI_2, 3. * FRAC_PI_2])
    );
    assert_eq!(
        ellipse.plane_crossings(Plane::new([0.; 3], Z).unwrap(), -TOL),
        Err(Error::Invalid)
    );
}

#[test]
fn refuses_other_intersection_families_and_invalid_supports() {
    let base = Cylinder::new([0.; 3], X, 2.).unwrap();
    let plane = |origin, normal| Plane::new(origin, normal).unwrap();
    let cut = cylinder_plane(base, plane([8., 0., 0.], X)).unwrap();
    assert_eq!(cut.center_mm(), [8., 0., 0.]);
    assert_eq!(cut.semiaxes_mm(), [2., 2.]);
    let cut = cylinder_plane(Cylinder::new([0.; 3], Z, 3.).unwrap(), plane([0.; 3], Z)).unwrap();
    assert_eq!(cut.semiaxes_mm(), [3., 3.]);
    let cut = cylinder_plane(Cylinder::new([0.; 3], Y, 4.).unwrap(), plane([0.; 3], Y)).unwrap();
    assert_eq!(cut.semiaxes_mm(), [4., 4.]);
    let cut = cylinder_plane(
        Cylinder::new([0.; 3], [f64::MAX, f64::MAX, 0.], 4.).unwrap(),
        plane([0.; 3], [f64::from_bits(1), f64::from_bits(1), 0.]),
    )
    .unwrap();
    near(cut.semiaxes_mm()[0], 4.);
    assert_eq!(Plane::new([0.; 3], [f64::NAN, 1., 0.]), Err(Error::Invalid));
    assert_eq!(
        cylinder_plane(base, plane([0.; 3], Y)),
        Err(Error::Unsupported)
    );
    assert_eq!(
        cylinder_plane(base, plane([0., 1e12, 0.], [1., 2., 0.])),
        Err(Error::Limit)
    );
    assert_eq!(
        cylinder_plane(base, plane([0.; 3], [1e-13, 1., 0.])),
        Err(Error::Limit)
    );

    let second = |origin, direction, radius| Cylinder::new(origin, direction, radius).unwrap();
    for other in [
        second([0.; 3], Y, 3.),
        second([0.; 3], [1e-6, 1., 0.], 2.),
        second([0., 0., 1e-8], Y, 2.),
    ] {
        assert_eq!(
            equal_perpendicular_cylinders(base, other, 1e-4),
            Err(Error::Unsupported)
        );
    }
    assert_eq!(
        equal_perpendicular_cylinders(base, second([0.; 3], Y, 2.), -TOL),
        Err(Error::Invalid)
    );
    assert_eq!(Cylinder::new([0.; 3], X, 0.), Err(Error::Invalid));
    assert_eq!(Plane::new([f64::NAN, 0., 0.], X), Err(Error::Invalid));
    assert_eq!(Plane::new([2e12, 0., 0.], X), Err(Error::Limit));
    assert_eq!(Plane::new([0.; 3], [0.; 3]), Err(Error::Invalid));
    assert_eq!(Ellipse::new([0.; 3], Z, X, [1., 2.]), Err(Error::Invalid));
    assert_eq!(Ellipse::new([0.; 3], Z, Z, [2., 1.]), Err(Error::Invalid));
}
