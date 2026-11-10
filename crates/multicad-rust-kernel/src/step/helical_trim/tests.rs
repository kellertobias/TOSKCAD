use super::*;
use crate::helical_tube_cut::{Cut, Spec};

const TOLERANCE: f64 = 1e-7;

fn cut(phase: f64) -> Cut {
    Cut::new(
        Spec {
            stock_radius_mm: 8.,
            stock_height_mm: 30.,
            tube_radius_mm: 0.6,
            pitch_mm: 2.,
            path_height_mm: 15.,
            start_height_mm: 16.2,
            start_angle_rad: phase,
        },
        TOLERANCE,
    )
    .unwrap()
}

fn eval(mut poles: [Point; DEGREE + 1], t: f64) -> Point {
    for n in (1..=DEGREE).rev() {
        for i in 0..n {
            poles[i] = std::array::from_fn(|j| (1. - t) * poles[i][j] + t * poles[i + 1][j]);
        }
    }
    poles[0]
}

#[test]
fn verifies_both_actual_cut_rails_across_lifted_seams_and_matches_sampled_geometry() {
    for phase in [-TAU, -0.1, 0., 0.1, TAU] {
        for trim in cut(phase).cylinder_trims() {
            let rail = Rail::new(8., trim, TOLERANCE).unwrap();
            let spline = rail.spline().unwrap();
            let pcurve = rail.pcurve();
            assert_eq!(pcurve.domain, trim.azimuth_rad);
            assert!(pcurve.domain[1] - pcurve.domain[0] > 6. * TAU);
            assert_eq!(pcurve.velocity[0], 1.);
            assert!((pcurve.velocity[1] - 2. / TAU).abs() < 1e-13);
            let bound = rail.certify(pcurve, &spline).unwrap();
            assert!(bound > 0. && bound < TOLERANCE * 0.25);
            for (i, angles) in spline.knots.windows(2).enumerate() {
                let first = i * DEGREE;
                let controls = spline.poles_mm[first..=first + DEGREE].try_into().unwrap();
                for j in 0..=16 {
                    let t = j as f64 / 16.;
                    let azimuth = angles[0] + (angles[1] - angles[0]) * t;
                    let u = pcurve.velocity[0].mul_add(azimuth, pcurve.origin[0]);
                    let z = pcurve.velocity[1].mul_add(azimuth, pcurve.origin[1]);
                    let (sin, cos) = u.sin_cos();
                    let actual = eval(controls, t);
                    let error = (actual[0] - 8. * cos)
                        .hypot(actual[1] - 8. * sin)
                        .hypot(actual[2] - z);
                    assert!(error <= bound, "{error} exceeds {bound}");
                }
            }
        }
    }
}

#[test]
fn decoded_geometry_cannot_hide_a_shifted_phase_changed_lead_or_bad_spline() {
    let rail = Rail::new(8., cut(0.).cylinder_trims()[0], TOLERANCE).unwrap();
    let source = rail.spline().unwrap();
    let pcurve = rail.pcurve();
    let invalid = Err(StepError::InvalidGeometry);
    for bad in [
        Pcurve {
            domain: pcurve.domain.map(|a| a + TAU),
            ..pcurve
        },
        Pcurve {
            origin: [TAU, pcurve.origin[1]],
            ..pcurve
        },
        Pcurve {
            origin: [0., f64::NAN],
            ..pcurve
        },
        Pcurve {
            velocity: [1., pcurve.velocity[1] * 1.000_1],
            ..pcurve
        },
    ] {
        assert_eq!(rail.certify(bad, &source), invalid);
    }
    let mut malformed = source.clone();
    malformed.poles_mm[5][0] += TOLERANCE * 0.05;
    assert!(rail.certify(pcurve, &malformed).unwrap() > rail.certify(pcurve, &source).unwrap());
    malformed.poles_mm[5][0] += 3. * TOLERANCE;
    assert_eq!(rail.certify(pcurve, &malformed), invalid);
    let mut malformed = source.clone();
    malformed.poles_mm[5][2] = malformed.poles_mm[4][2];
    assert_eq!(rail.certify(pcurve, &malformed), invalid);
    let mut malformed = source.clone();
    malformed.knots[1] = malformed.knots[0];
    assert_eq!(rail.certify(pcurve, &malformed), invalid);
    let mut malformed = source.clone();
    malformed.multiplicities[0] = DEGREE;
    assert_eq!(rail.certify(pcurve, &malformed), invalid);
    let mut malformed = source;
    malformed.degree -= 1;
    assert_eq!(rail.certify(pcurve, &malformed), invalid);
}

#[test]
fn refuses_unbounded_nonfinite_or_reverse_charts() {
    let trim = cut(0.).cylinder_trims()[0];
    for (radius, trim, tolerance) in [
        (f64::NAN, trim, TOLERANCE),
        (8., trim, 0.),
        (
            8.,
            CylinderTrim {
                azimuth_rad: [1., 0.],
                ..trim
            },
            TOLERANCE,
        ),
        (
            8.,
            CylinderTrim {
                height_mm: [2., 1.],
                ..trim
            },
            TOLERANCE,
        ),
        (
            8.,
            CylinderTrim {
                height_mm: [f64::INFINITY, 1.],
                ..trim
            },
            TOLERANCE,
        ),
    ] {
        assert_eq!(
            Rail::new(radius, trim, tolerance).unwrap_err(),
            StepError::InvalidGeometry
        );
    }
    assert_eq!(
        Rail::new(1e7, trim, TOLERANCE).unwrap_err(),
        StepError::Limit
    );
    assert_eq!(
        Rail::new(0.01, trim, TOLERANCE).unwrap_err(),
        StepError::Limit
    );
}
