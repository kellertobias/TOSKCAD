//! Certified spline chart for an analytic circular thread root.
use super::*;
use crate::helical_thread::RootCircle;

const DERIVATIVE_ORDER: f64 = (DEGREE + 1) as f64;
const FACTORIALS: [f64; JETS] = [1., 1., 2., 6., 24., 120.];

impl Spline {
    pub(super) fn rounded_root(
        circle: RootCircle,
        phases: [f64; 2],
        radii: [f64; 2],
        rate: f64,
        sense: f64,
        error: f64,
    ) -> Result<Self, StepError> {
        // Use circle angle as the chart coordinate. It removes the square
        // root from the radial profile while preserving phase and radius.
        let angles = phases.map(|p| ((p - circle.phase) / circle.radius).asin());
        let amplitude = sense * circle.radius / rate;
        let log_error = error.ln();
        let span_limit = [1.5_f64, 2., 2.5, 3.]
            .into_iter()
            .map(|rho| {
                // A(t) = (C - R cos t) exp(i s(c + R sin t)/rate) is
                // entire. On a disk of radius rho about real t, its norm
                // is at most (C + R cosh rho) exp(|sR/rate| sinh rho).
                // Cauchy bounds f^(12) by 12! times this norm / rho^12.
                // Substitute into the Hermite remainder at half-span.
                let log_bound = (circle.center_radius + circle.radius * rho.cosh()).ln()
                    + amplitude.abs() * rho.sinh();
                2. * rho * ((log_error - log_bound) / DERIVATIVE_ORDER).exp()
            })
            .fold(0., f64::max);
        let mut spline = Self::with_jets(angles[0], angles[1], span_limit, |angle| {
            rounded_jet(circle, angle, rate, sense)
        })?;

        // The exact same authored phase/radius endpoints also bound the
        // neighboring flank charts. Reuse their floating-point values at
        // the seam, rather than relying on asin/sin to round identically.
        let endpoint = |i: usize| affine_jet(phases[i] / rate, radii[i], 0., sense)[0];
        spline.poles[0] = endpoint(0);
        *spline.poles.last_mut().expect("nonempty spline") = endpoint(1);
        Ok(spline)
    }
}

fn rounded_jet(circle: RootCircle, t: f64, rate: f64, sense: f64) -> [Complex; JETS] {
    let (s, c) = t.sin_cos();
    let trig = [[s, c], [c, -s], [-s, -c], [-c, s]];
    let amplitude = sense * circle.radius / rate;
    let (sine, cosine) = (sense * ((circle.phase + circle.radius * s) / rate)).sin_cos();
    let mut radius =
        std::array::from_fn::<_, JETS, _>(|i| -circle.radius * trig[i % 4][1] / FACTORIALS[i]);
    radius[0] += circle.center_radius;

    // Compute normalized Taylor coefficients for exp(i*phase(t)/rate)
    // via e' = g' e. The constant term is the exact unit rotation.
    let mut spin = [[0.; 2]; JETS];
    spin[0] = [cosine, sine];
    for order in 1..JETS {
        let mut coeff = [0.; 2];
        for k in 1..=order {
            let imaginary = amplitude * trig[k % 4][0] * k as f64 / FACTORIALS[k];
            let [x, y] = spin[order - k];
            coeff[0] -= imaginary * y;
            coeff[1] += imaginary * x;
        }
        spin[order] = coeff.map(|x| x / order as f64);
    }
    std::array::from_fn(|order| {
        let mut coeff = [0.; 2];
        for k in 0..=order {
            for (sum, term) in coeff.iter_mut().zip(spin[order - k]) {
                *sum += radius[k] * term;
            }
        }
        coeff.map(|x| x * FACTORIALS[order])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        helical_thread::Spec,
        step_read::{verify_helical_screw, verify_helical_thread},
    };

    #[test]
    fn rounded_root_uses_a_bounded_curve_and_shares_both_flank_rims() {
        let crest = 0.13;
        let root_radius = 1. / (4. * 3_f64.sqrt());
        let profile = |left_hand| {
            Thread::new(Spec {
                major_radius_mm: 2.96,
                pitch_mm: 1.,
                length_mm: 4.2,
                depth_mm: (1. - crest) * 3_f64.sqrt() / 2. - root_radius,
                crest_width_mm: crest,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                starts: 1,
                left_hand,
                root_radius_mm: Some(root_radius),
            })
            .unwrap()
        };
        let [origin, u, v] = [[2., 3., 4.], [1., 0., 0.], [0., 1., 0.]];
        for left_hand in [false, true] {
            let thread = profile(left_hand);
            let chart = prepare(&thread, Closure::Shaft, origin, 1e-7).unwrap();
            let root = &chart.bands[1];
            let circle = thread.root_circle().unwrap();
            let invalid = Spline::rounded_root(
                circle,
                [circle.phase - 2. * circle.radius, circle.phase],
                [thread.root_radius_mm(); 2],
                thread.helical_rate().abs(),
                1.,
                1e-8,
            );
            assert_eq!(invalid.err(), Some(StepError::Limit));
            assert_eq!(root.poles[0], *chart.bands[0].poles.last().unwrap());
            assert_eq!(*root.poles.last().unwrap(), chart.bands[2].poles[0]);
            assert!((root.knots[0] + std::f64::consts::FRAC_PI_3).abs() < 1e-12);
            for (i, pair) in root.knots.windows(2).enumerate() {
                for j in 0..=24 {
                    let u = j as f64 / 24.;
                    let angle = pair[0] + (pair[1] - pair[0]) * u;
                    let phase = circle.phase + circle.radius * angle.sin();
                    let radius = thread.radius_at_phase(phase);
                    let (sin, cos) = (-phase / thread.helical_rate()).sin_cos();
                    let mut controls: [Complex; DEGREE + 1] = root.poles
                        [i * DEGREE..=i * DEGREE + DEGREE]
                        .try_into()
                        .unwrap();
                    for n in (1..=DEGREE).rev() {
                        for k in 0..n {
                            controls[k] = std::array::from_fn(|l| {
                                (1. - u) * controls[k][l] + u * controls[k + 1][l]
                            });
                        }
                    }
                    assert!(
                        (controls[0][0] - radius * cos).hypot(controls[0][1] - radius * sin)
                            < 1e-7 / 8.
                    );
                }
            }
            let shaft = write_helical_thread(&thread, None, origin, u, v, 1e-7).unwrap();
            let volume = verify_helical_thread(&shaft, &thread, None, origin, u, v, 1e-7).unwrap();
            assert!((volume - thread.volume_mm3()).abs() < 1e-6);
            let screw = Screw::new(
                thread,
                crate::headed_screw::ScrewSpec {
                    head: ScrewHead::ExternalHex {
                        across_flats_mm: 10.,
                        height_mm: 4.,
                    },
                    shank_length_mm: 0.,
                    shank_radius_mm: 3.,
                },
            )
            .unwrap();
            let headed = write_helical_screw(&screw, origin, u, v, 1e-7).unwrap();
            let volume = verify_helical_screw(&headed, &screw, origin, u, v, 1e-7).unwrap();
            assert!((volume - screw.volume_mm3()).abs() < 1e-6);
        }
    }
}
