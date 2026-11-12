//! Outward arithmetic receipts of actual final Hermite controls. These bound
//! jet conditioning and span arithmetic rather than inferring it from size.
use super::*;
use crate::tapered_thread::interval::I;
type ComplexI = [I; 2];
type Jet = [ComplexI; JETS];
fn point(v: f64) -> I {
    I::point(v)
}
fn complex([a, b]: ComplexI, [c, d]: ComplexI) -> ComplexI {
    [a * c - b * d, a * d + b * c]
}
fn bounded(value: f64, budget: f64) -> Result<(), StepError> {
    if value.is_finite() && value <= budget {
        Ok(())
    } else {
        Err(StepError::Limit)
    }
}
fn extent(interval: I, nominal: f64) -> f64 {
    ((interval.lo - nominal)
        .abs()
        .max((interval.hi - nominal).abs()))
    .next_up()
}
fn controls(jet: Jet, length: I) -> [ComplexI; JETS] {
    std::array::from_fn(|i| {
        let mut p = jet[0];
        let mut weight = point(1.);
        for (k, d) in jet.iter().enumerate().take(i + 1).skip(1) {
            weight =
                weight * length * point((i - k + 1) as f64) / point(((DEGREE - k + 1) * k) as f64);
            p[0] = p[0] + d[0] * weight;
            p[1] = p[1] + d[1] * weight;
        }
        p
    })
}
fn certify(
    spline: &Spline,
    jet: impl Fn(f64) -> Jet,
    truncation: impl Fn(I) -> I,
    extra: f64,
    budget: f64,
) -> Result<(), StepError> {
    for (span, pair) in spline.knots.windows(2).enumerate() {
        let length = point(pair[1]) - point(pair[0]);
        let left = controls(jet(pair[0]), length);
        let right = controls(jet(pair[1]), -length);
        let remainder = truncation(length);
        for (i, exact) in left.into_iter().chain(right.into_iter().rev()).enumerate() {
            let nominal = spline.poles[span * DEGREE + i];
            let arithmetic =
                (extent(exact[0], nominal[0]) + extent(exact[1], nominal[1])).next_up();
            bounded((point(arithmetic) + remainder + point(extra)).hi, budget)?;
        }
    }
    Ok(())
}
pub(super) fn affine(
    spline: &Spline,
    domain: [f64; 2],
    radii: [f64; 2],
    sense: f64,
    budget: f64,
) -> Result<(), StepError> {
    let slope = (point(radii[1]) - point(radii[0])) / (point(domain[1]) - point(domain[0]));
    certify(
        spline,
        |t| {
            let angle = point(sense) * point(t);
            let rotation = [angle.cos(), angle.sin()];
            let radius = point(radii[0]) + slope * (point(t) - point(domain[0]));
            let mut power = [point(1.), point(0.)];
            std::array::from_fn(|k| {
                let radial = [radius, point(-sense * k as f64) * slope];
                let derivative = complex(complex(rotation, power), radial);
                power = complex(power, [point(0.), point(sense)]);
                derivative
            })
        },
        |length| {
            let derivative = point(radii[0].abs().max(radii[1].abs()))
                + point((DEGREE + 1) as f64) * point(slope.magnitude());
            derivative * power(length / point(2.)) / point(479_001_600.)
        },
        0.,
        budget,
    )
}
fn power(value: I) -> I {
    (0..DEGREE + 1).fold(point(1.), |product, _| product * value)
}
fn hyperbolic_upper(value: f64) -> I {
    point((value * (1. + 8. * f64::EPSILON)).next_up())
}
fn circle_remainder(c: Circle, rate: f64, length: I) -> I {
    let upper = [1.5_f64, 2., 2.5, 3.]
        .into_iter()
        .map(|rho| {
            let radial = point(c.center_radius_mm.abs())
                + point(c.center_error_mm)
                + point(c.radius_mm) * hyperbolic_upper(rho.cosh());
            let exponent = point(c.radius_mm) / point(rate) * hyperbolic_upper(rho.sinh());
            let maximum = radial * hyperbolic_upper(exponent.hi.exp());
            (maximum * power(length / point(2. * rho))).hi
        })
        .fold(f64::INFINITY, f64::min);
    point(upper)
}
fn circle_jet(c: Circle, t: f64, rate: f64, sense: f64) -> Jet {
    let s = point(t).sin();
    let co = point(t).cos();
    let trig = [[s, co], [co, -s], [-s, -co], [-co, s]];
    let center = I::new(
        (c.center_radius_mm - c.center_error_mm).next_down(),
        (c.center_radius_mm + c.center_error_mm).next_up(),
    );
    let mut radius = std::array::from_fn::<_, JETS, _>(|i| {
        point(c.radial_sign) * point(c.radius_mm) * trig[i % 4][1] / point(FACTORIALS[i])
    });
    radius[0] = radius[0] + center;
    let angle = point(sense) * (point(c.center_phase_mm) + point(c.radius_mm) * s) / point(rate);
    let mut spin = [[point(0.); 2]; JETS];
    spin[0] = [angle.cos(), angle.sin()];
    let amplitude = point(sense) * point(c.radius_mm) / point(rate);
    for order in 1..JETS {
        let mut coefficient = [point(0.); 2];
        for k in 1..=order {
            let imaginary = amplitude * trig[k % 4][0] * point(k as f64) / point(FACTORIALS[k]);
            coefficient[0] = coefficient[0] - imaginary * spin[order - k][1];
            coefficient[1] = coefficient[1] + imaginary * spin[order - k][0];
        }
        spin[order] = coefficient.map(|x| x / point(order as f64));
    }
    std::array::from_fn(|order| {
        let mut result = [point(0.); 2];
        for k in 0..=order {
            for (sum, term) in result.iter_mut().zip(spin[order - k]) {
                *sum = *sum + radius[k] * term;
            }
        }
        result.map(|x| x * point(FACTORIALS[order]))
    })
}
pub(super) fn circle(
    spline: &Spline,
    c: Circle,
    phases: [f64; 2],
    rate: f64,
    sense: f64,
    budget: f64,
) -> Result<(), StepError> {
    // A round-trip phase residual receipts the actually recovered asin ends.
    // Monotonic sin on this chart and a positive derivative bound turn it
    // into a parameter error, then bound the complete radial/rotational map.
    let mut transport = 0_f64;
    for (t, p) in [spline.knots[0], *spline.knots.last().expect("spline")]
        .into_iter()
        .zip(phases)
    {
        if t.abs() >= std::f64::consts::FRAC_PI_2 {
            return Err(StepError::Limit);
        }
        let sin = point(t).sin();
        let q = (point(p) - point(c.center_phase_mm)) / point(c.radius_mm);
        let max = sin.magnitude().max(q.magnitude());
        let cosine = (point(1.) - point(max) * point(max)).sqrt();
        if !cosine.valid() || cosine.lo <= 0. {
            return Err(StepError::Limit);
        }
        let residual = (point(c.center_phase_mm) + point(c.radius_mm) * sin - point(p))
            .magnitude()
            .next_up();
        let delta = point(residual) / (point(c.radius_mm) * point(cosine.lo));
        let radial =
            point(c.center_radius_mm.abs()) + point(c.center_error_mm) + point(c.radius_mm);
        let speed = point(c.radius_mm) * (point(1.) + radial / point(rate));
        transport = transport.max((delta * speed).hi);
        bounded(transport, budget)?;
    }
    certify(
        spline,
        |t| circle_jet(c, t, rate, sense),
        |length| circle_remainder(c, rate, length),
        transport,
        budget,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_controls_and_phase_receipts_reject_corruption_and_insufficient_precision() {
        let source = super::super::tests::thread(false, false, false);
        let splines = prepare(&source, [0.; 3], 1e-7).unwrap();
        let phases = source.profile_breaks();
        for band in [0, 2, 4] {
            let c = source.profile_circle(band).unwrap();
            let p = [phases[band], phases[band + 1]];
            let curve = &splines.bands[band];
            assert!(circle(curve, c, p, source.helical_rate().abs(), -1., 1e-7 / 32.).is_ok());
            assert_eq!(
                circle(curve, c, p, source.helical_rate().abs(), -1., 1e-30),
                Err(StepError::Limit)
            );
            let mut damaged = Spline {
                knots: curve.knots.clone(),
                poles: curve.poles.clone(),
            };
            damaged.poles[1][0] += 1e-5;
            assert_eq!(
                circle(&damaged, c, p, source.helical_rate().abs(), -1., 1e-7 / 32.),
                Err(StepError::Limit)
            );
            damaged.knots[0] = std::f64::consts::FRAC_PI_2;
            assert_eq!(
                circle(&damaged, c, p, source.helical_rate().abs(), -1., 1e-7 / 32.),
                Err(StepError::Limit)
            );
            let wrong = [c.center_phase_mm + 2. * c.radius_mm, p[1]];
            assert_eq!(
                circle(
                    curve,
                    c,
                    wrong,
                    source.helical_rate().abs(),
                    -1.,
                    1e-7 / 32.
                ),
                Err(StepError::Limit)
            );
        }
        assert_eq!(bounded(f64::INFINITY, 1.), Err(StepError::Limit));
    }
    #[test]
    fn high_amplitude_and_center_cancellation_keep_independent_point_error_bounds() {
        let theta = 80_f64.to_radians();
        let total = 0.8 / (2. * theta.cos());
        let rc = total * 0.99;
        let rr = total * 0.01;
        let depth = 1. / (2. * theta.tan()) - (rc + rr) * (1. / theta.sin() - 1.);
        let source = Thread::new(crate::rounded_thread::Spec {
            major_radius_mm: 0.1,
            pitch_mm: 1.,
            length_mm: 2.37,
            depth_mm: depth,
            crest_radius_mm: rc,
            root_radius_mm: rr,
            flank_angle_deg: 80.,
            starts: 1,
            left_hand: false,
        })
        .unwrap();
        assert!(source.profile_circle(0).unwrap().center_radius_mm < 0.);
        assert!(rc / source.helical_rate() > 10.);
        let splines = prepare(&source, [0.; 3], 1e-7).unwrap();
        for band in [0, 2, 4] {
            let c = source.profile_circle(band).unwrap();
            let curve = &splines.bands[band];
            for (span, pair) in curve.knots.windows(2).enumerate() {
                for i in 0..=32 {
                    let u = i as f64 / 32.;
                    let t = pair[0] + (pair[1] - pair[0]) * u;
                    let phase = c.center_phase_mm + c.radius_mm * t.sin();
                    let radius = source.radius_at_phase(phase);
                    let angle = -phase / source.helical_rate();
                    let exact = [radius * angle.cos(), radius * angle.sin()];
                    let mut poles: Vec<_> =
                        curve.poles[span * DEGREE..=span * DEGREE + DEGREE].to_vec();
                    for remaining in (1..=DEGREE).rev() {
                        for j in 0..remaining {
                            let a = poles[j];
                            let b = poles[j + 1];
                            poles[j] = std::array::from_fn(|k| (1. - u) * a[k] + u * b[k]);
                        }
                    }
                    assert!((poles[0][0] - exact[0]).hypot(poles[0][1] - exact[1]) <= 1e-7 / 16.);
                }
            }
        }
    }
}
