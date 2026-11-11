//! Ray visibility for an oblique ring torus.
use super::*;

impl Torus {
    pub(super) fn hide_oblique(
        &self,
        path: Path,
        span: Span,
        gap: f64,
        limits: [Option<f64>; 2],
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        let [a, b, c] = self.axis;
        let s = a.hypot(b);
        let [x, y, z] = [[a / s, b / s, 0.], [b / s, -a / s, 0.], [0., 0., 1.]]
            .map(|axis| path.coordinate(self.center, axis));
        let scale = self.major + self.minor;
        let span = if path.harmonic {
            span
        } else {
            let Some(span) =
                clip_line_span([x, y], [self.major * c.abs() + self.minor, scale], span)
            else {
                return Ok(());
            };
            span
        };
        let major = self.major / scale;
        let minor = self.minor / scale;
        let shape = RayShape { major, minor, s, c };
        work.charge(if path.harmonic { 2048 } else { 512 })?;
        discriminant_events([x, y], shape, scale, span, &mut work.crossings);
        let gap_depth = z.offset(gap);
        let limits = limits.map(|limit| limit.map(|value| value - self.center[2]));
        let mut equal_depth = |depth| {
            torus_events(
                [x.combine(-c, depth, s), y, x.combine(s, depth, c)],
                self.major,
                self.minor,
                span,
                &mut work.crossings,
            );
        };
        equal_depth(gap_depth);
        for level in limits.into_iter().flatten() {
            equal_depth(Trace {
                coefficients: [level, 0., 0.],
                harmonic: path.harmonic,
            });
        }
        drawing_events::spans(
            &mut work.crossings,
            span,
            |t| {
                let coefficients = shape.ray(x.at(t) / scale, y.at(t) / scale);
                let candidate = gap_depth.at(t) / scale;
                let [low, high] = limits.map(|value| value.map(|value| value / scale));
                if !self.solid_depth_clip && limits != [None; 2] {
                    return roots(&coefficients, [-1.0001, 1.0001])
                        .last()
                        .is_some_and(|&front| {
                            front > candidate
                                && low.is_none_or(|low| front >= low)
                                && high.is_none_or(|high| front <= high)
                        });
                }
                let lo = low.unwrap_or(-1.).max(candidate);
                let hi = high.unwrap_or(1.);
                if lo >= hi {
                    return false;
                }
                let evaluate = |at| evaluate_polynomial(&coefficients, at);
                let roundoff = 16. * f64::EPSILON;
                [lo, hi]
                    .into_iter()
                    .chain(roots(
                        &crate::curved_projection::derivative(&coefficients),
                        [lo, hi],
                    ))
                    .any(|at| evaluate(at) < -roundoff)
            },
            |span| work.hidden.push(span),
        );
        Ok(())
    }
}

fn clip_line_span(traces: [Trace; 2], extents: [f64; 2], mut span: Span) -> Option<Span> {
    // Build the discriminant in a local chart even for long stock edges.
    for (trace, extent) in traces.into_iter().zip(extents) {
        let slope = trace.coefficients[1];
        if slope == 0. {
            if trace.coefficients[0].abs() > extent {
                return None;
            }
            continue;
        }
        let mut limits = [-extent, extent].map(|level| (level - trace.coefficients[0]) / slope);
        if slope < 0. {
            limits.reverse();
        }
        span = [span[0].max(limits[0]), span[1].min(limits[1])];
        if span[0] >= span[1] {
            return None;
        }
    }
    Some(span)
}

#[derive(Clone, Copy)]
struct RayShape {
    major: f64,
    minor: f64,
    s: f64,
    c: f64,
}

impl RayShape {
    fn ray(self, x: f64, y: f64) -> [f64; 5] {
        let r2 = self.major.powi(2);
        let bias = (self.major - self.minor) * (self.major + self.minor);
        let sum = x.mul_add(x, y.mul_add(y, bias));
        let radial = (self.c * x).mul_add(self.c * x, y * y);
        [
            (-4. * r2).mul_add(radial, sum * sum),
            8. * r2 * self.c * self.s * x,
            (-4. * r2).mul_add(self.s * self.s, 2. * sum),
            0.,
            1.,
        ]
    }

    // For z^4 + p z^2 + q z + r, ray roots merge only where
    // 256 r^3 - 128 p^2 r^2 + 144 p q^2 r - 27 q^4 + 16 p^4 r - 4 p^3 q^2 = 0.
    // Homogenize z by the path's denominator before assembling this polynomial.
    fn discriminant(self, [x, y]: [[f64; 3]; 2], w: [f64; 3]) -> Vec<f64> {
        let r2 = self.major.powi(2);
        let bias = (self.major - self.minor) * (self.major + self.minor);
        let x2 = product::<3, 5>(x, x);
        let y2 = product::<3, 5>(y, y);
        let w2 = product::<3, 5>(w, w);
        let sum: [f64; 5] = std::array::from_fn(|i| w2[i].mul_add(bias, x2[i] + y2[i]));
        let p = std::array::from_fn::<_, 5, _>(|i| {
            (-4. * r2 * self.s.powi(2)).mul_add(w2[i], 2. * sum[i])
        });
        let q = multiply(&x, &w2)
            .into_iter()
            .map(|value| value * 8. * r2 * self.c * self.s)
            .collect::<Vec<_>>();
        let radial: [f64; 5] = std::array::from_fn(|i| self.c.powi(2).mul_add(x2[i], y2[i]));
        let rw = product::<5, 9>(radial, w2);
        let r = product::<5, 9>(sum, sum);
        let r = std::array::from_fn::<_, 9, _>(|i| (-4. * r2).mul_add(rw[i], r[i]));
        let pp = multiply(&p, &p);
        let qq = multiply(&q, &q);
        let rr = multiply(&r, &r);
        let ppp = multiply(&pp, &p);
        let mut output = vec![0.; 25];
        for (factor, terms) in [
            (256., multiply(&rr, &r)),
            (-128., multiply(&pp, &rr)),
            (144., multiply(&multiply(&p, &qq), &r)),
            (-27., multiply(&qq, &qq)),
            (16., multiply(&multiply(&pp, &pp), &r)),
            (-4., multiply(&ppp, &qq)),
        ] {
            for (out, value) in output.iter_mut().zip(terms) {
                *out = value.mul_add(factor, *out);
            }
        }
        output
    }
}

fn evaluate_polynomial(coefficients: &[f64], at: f64) -> f64 {
    coefficients
        .iter()
        .rev()
        .fold(0_f64, |value, coefficient| value.mul_add(at, *coefficient))
}

fn multiply(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut result = vec![0.; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            result[i + j] = x.mul_add(y, result[i + j]);
        }
    }
    result
}

fn discriminant_events(
    traces: [Trace; 2],
    shape: RayShape,
    scale: f64,
    [lo, hi]: Span,
    cuts: &mut Vec<f64>,
) {
    let solve = |numerators, weight, domain| {
        let polynomial = shape.discriminant(numerators, weight);
        let extrema = crate::curved_projection::derivative(&polynomial);
        roots(&polynomial, domain)
            .into_iter()
            .chain(roots(&extrema, domain))
    };
    if !traces[0].harmonic {
        let width = hi - lo;
        let numerators = traces.map(|trace| {
            [
                trace.at(lo) / scale,
                trace.coefficients[1] * width / scale,
                0.,
            ]
        });
        cuts.extend(solve(numerators, [1., 0., 0.], [0., 1.]).map(|t| width.mul_add(t, lo)));
        return;
    }
    for (phase, sign) in [(0., 1.), (std::f64::consts::PI, -1.), (TAU, 1.)] {
        let start = lo.max(phase - FRAC_PI_2);
        let end = hi.min(phase + FRAC_PI_2);
        if start >= end {
            continue;
        }
        let numerators = traces.map(|trace| {
            let [a, b, c] = trace.coefficients.map(|coefficient| coefficient / scale);
            [a + sign * b, 2. * sign * c, a - sign * b]
        });
        let domain = [start, end].map(|angle| ((angle - phase) / 2.).tan());
        cuts.extend(
            solve(numerators, [1., 0., 1.], domain)
                .map(|t| phase + 2. * t.atan())
                .filter(|&t| t > start && t < end),
        );
        if end < hi {
            cuts.push(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oblique_rays_reject_edges_outside_the_chart_and_ahead_of_the_slab() {
        let torus = Torus {
            center: [0.; 3],
            axis: [0.6, 0., 0.8],
            major: 6.,
            minor: 2.,
            solid_depth_clip: true,
        };
        let options = Options {
            tolerance: 1e-7,
            hidden_lines: true,
            tangent_edges: false,
            detail: None,
            segment_limit: 100,
        };
        for (origin, direction) in [
            ([0., 8.1, -10.], [1., 0., 0.]),
            ([7., 0., -10.], [1., 0., 0.]),
            ([-5., 0., 4.], [1., 0., 0.]),
        ] {
            let mut work = projection_work(&options, WORK_LIMIT).unwrap();
            torus
                .hide_oblique(
                    Path {
                        origin,
                        b: direction,
                        c: [0.; 3],
                        harmonic: false,
                    },
                    [0., 1.],
                    1e-7,
                    [Some(3.5), Some(3.7)],
                    &mut work,
                )
                .unwrap();
            assert!(work.hidden.is_empty());
        }
    }
}
