use super::*;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const FULL: [Uv; 2] = [[ZERO; 2], [ONE; 2]];

fn rational(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

fn fixture(rise: Rational, trims: [[Uv; 2]; 2]) -> [Patch; 2] {
    // Left height is uv. The exact graph difference is
    // 4(u-1/2)² + 4(u-1/2)(v-1/2) + 8(v-1/2)² - rise.
    let left = std::array::from_fn(|i| std::array::from_fn(|j| rational((i * j) as i64, 4)));
    let right = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            let [i, j] = [i as i64, j as i64];
            let conic = 4 * i64::from(i == 2) + 8 * i64::from(j == 2) + i * j - 3 * i - 5 * j + 4;
            left[i as usize][j as usize]
                .checked_sub(rational(conic, 1))
                .unwrap()
                .checked_add(rise)
                .unwrap()
        })
    });
    [
        Patch::new(left, trims[0]).unwrap(),
        Patch::new(right, trims[1]).unwrap(),
    ]
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-13, "{a} != {b}");
}

#[test]
fn exact_nonplanar_loop_lies_on_both_trimmed_splines() {
    let trim = [[rational(1, 8); 2], [rational(7, 8); 2]];
    let [a, b] = fixture(rational(1, 8), [FULL, trim]);
    let Intersection::Closed(loop_a) = a.intersect(&b).unwrap() else {
        panic!("expected an interior loop");
    };
    let Intersection::Closed(loop_b) = b.intersect(&a).unwrap() else {
        panic!("negative-definite difference describes the same loop");
    };
    assert_eq!(loop_a.center_uv(), [rational(1, 2); 2]);
    for i in 0..=128 {
        let angle = i as f64 * TAU / 128.;
        let point = loop_a.point_at(angle);
        let [u, v, z] = point;
        let [du, dv] = [u - 0.5, v - 0.5];
        close(4. * du * du + 4. * du * dv + 8. * dv * dv, 0.125);
        close(z, u * v);
        close(a.point_at([u, v])[2], z);
        close(b.point_at([u, v])[2], z);
        close(loop_b.point_at(angle)[2], z);
        assert!([u, v].into_iter().all(|x| (0.125..0.875).contains(&x)));
    }
    let height_pair = |angle| (loop_a.point_at(angle)[2] + loop_a.point_at(angle + PI)[2]) * 0.5;
    close(height_pair(0.), 0.25);
    close(height_pair(FRAC_PI_2), 0.25 - 1. / 112.);
    assert!((height_pair(0.) - height_pair(FRAC_PI_2)).abs() > 1e-3);
}

#[test]
fn common_biquadratic_terms_cancel_in_the_exact_difference() {
    let [mut left, mut right] = fixture(rational(1, 8), [FULL; 2]);
    left.heights[2][2] = left.heights[2][2].checked_add(ONE).unwrap();
    right.heights[2][2] = right.heights[2][2].checked_add(ONE).unwrap();
    let Intersection::Closed(curve) = left.intersect(&right).unwrap() else {
        panic!("the common u²v² term leaves the conic unchanged");
    };
    for angle in [0., FRAC_PI_2, PI] {
        let [u, v, height] = curve.point_at(angle);
        close(height, u * v + (u * v).powi(2));
        close(right.point_at([u, v])[2], height);
    }
}

#[test]
fn exact_tangency_and_tiny_separation_do_not_depend_on_an_epsilon() {
    let near = rational(1, 10_i64.pow(15));
    let [left, right] = fixture(ZERO, [FULL; 2]);
    assert_eq!(
        left.intersect(&right).unwrap(),
        Intersection::Tangent {
            uv: [rational(1, 2); 2]
        }
    );
    let [left, right] = fixture(ZERO.checked_sub(near).unwrap(), [FULL; 2]);
    assert_eq!(left.intersect(&right), Ok(Intersection::Disjoint));
    let [left, right] = fixture(near, [FULL; 2]);
    assert!(matches!(
        left.intersect(&right),
        Ok(Intersection::Closed(_))
    ));
}

#[test]
fn trim_checks_handle_containment_disjointness_and_boundary_contact() {
    let mid = rational(1, 2);
    let above = rational(3, 5);
    let [left, right] = fixture(ZERO, [FULL, [[above, ZERO], [ONE; 2]]]);
    assert_eq!(left.intersect(&right), Ok(Intersection::Disjoint));
    let [left, right] = fixture(ZERO, [FULL, [[mid, ZERO], [ONE; 2]]]);
    assert_eq!(left.intersect(&right), Err(Error::TrimContact));

    let rise = rational(1, 8);
    let [left, right] = fixture(rise, [FULL, [[above; 2], [ONE; 2]]]);
    assert_eq!(left.intersect(&right), Ok(Intersection::Disjoint));
    let inside = [[rational(49, 100); 2], [rational(51, 100); 2]];
    let [left, right] = fixture(rise, [FULL, inside]);
    assert_eq!(left.intersect(&right), Ok(Intersection::Disjoint));

    let first = [[ZERO; 2], [mid, ONE]];
    let second = [[mid, ZERO], [ONE; 2]];
    let [left, right] = fixture(rise, [first, second]);
    assert_eq!(left.intersect(&right), Err(Error::TrimContact));
    let [left, right] = fixture(rise, [first, [[above, ZERO], [ONE; 2]]]);
    assert_eq!(left.intersect(&right), Ok(Intersection::Disjoint));
    let [left, right] = fixture(rise, [FULL, [[ZERO; 2], [above, ONE]]]);
    assert_eq!(left.intersect(&right), Err(Error::TrimContact));
}

#[test]
fn unsupported_polynomials_and_out_of_range_arithmetic_fail_closed() {
    let flat = Patch::new([[ZERO; 3]; 3], FULL).unwrap();
    assert_eq!(
        Patch::new([[ZERO; 3]; 3], [[ZERO; 2]; 2]),
        Err(Error::InvalidTrim)
    );
    let mut biquadratic = [[ZERO; 3]; 3];
    biquadratic[2][2] = ONE;
    let higher = Patch::new(biquadratic, FULL).unwrap();
    assert_eq!(flat.intersect(&higher), Err(Error::NonQuadratic));

    let saddle = std::array::from_fn(|i| {
        std::array::from_fn(|j| rational(i64::from(i == 2) - i64::from(j == 2), 1))
    });
    assert_eq!(
        flat.intersect(&Patch::new(saddle, FULL).unwrap()),
        Err(Error::NonDefinite)
    );
    let extreme = rational(10_i64.pow(18), 1);
    let mut heights = [[ZERO; 3]; 3];
    heights[1][0] = extreme;
    let steep = Patch::new(heights, FULL).unwrap();
    assert_eq!(
        steep.intersect(&flat),
        Err(Error::Arithmetic(crate::Error::OutOfRange))
    );

    let left = Patch::new([[extreme; 3]; 3], FULL).unwrap();
    let right = Patch::new([[rational(-10_i64.pow(18), 1); 3]; 3], FULL).unwrap();
    assert_eq!(
        left.intersect(&right),
        Err(Error::Arithmetic(crate::Error::OutOfRange))
    );
}

#[test]
fn a_planar_patch_cuts_a_circle_and_trim_tangency_is_refused() {
    let quarter = rational(1, 4);
    let controls = [quarter, rational(-1, 4), quarter];
    let bowl = Patch::new(
        controls.map(|u| controls.map(|v| u.checked_add(v).unwrap())),
        FULL,
    )
    .unwrap();
    let heights = [[rational(1, 16); 3]; 3];
    let plane = Patch::new(heights, FULL).unwrap();
    let Intersection::Closed(circle) = bowl.intersect(&plane).unwrap() else {
        panic!("expected an interior circle");
    };
    for angle in [0., FRAC_PI_2, PI, TAU] {
        let [u, v, z] = circle.point_at(angle);
        close((u - 0.5).hypot(v - 0.5), 0.25);
        close(z, 1. / 16.);
    }
    let tangent_trim = Patch::new(heights, [[quarter, ZERO], [ONE; 2]]).unwrap();
    assert_eq!(bowl.intersect(&tangent_trim), Err(Error::TrimContact));
}

#[test]
fn clipped_tilted_loop_is_lifted_to_both_independent_surfaces() {
    let midpoint = rational(1, 2);
    let left_trim = [[midpoint, ZERO], [ONE; 2]];
    let right_trim = [[ZERO, midpoint], [ONE; 2]];
    let [left, right] = fixture(rational(1, 8), [left_trim, right_trim]);
    let Trace::Clipped(result) = left.trace(&right).unwrap() else {
        panic!("expected one branch bounded by both input surfaces");
    };
    let [branch] = result.branches() else {
        panic!("expected one branch");
    };
    let [start, end] = branch.endpoints();
    assert_eq!(start.trim_coordinate(), (1, midpoint));
    assert_eq!(end.trim_coordinate(), (0, midpoint));
    for i in 0..=64 {
        let [u, v, z] = result.point_at(0, f64::from(i) / 64.).unwrap();
        assert!(u >= 0.5 - 1e-13 && v >= 0.5 - 1e-13);
        close(z, u * v);
        close(z, right.point_at([u, v])[2]);
    }
}
