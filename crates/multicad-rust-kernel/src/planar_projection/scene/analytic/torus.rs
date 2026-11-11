//! Exact visibility against the outer surface of an axial or transverse torus.
use super::*;

pub(super) struct Torus {
    center: Point,
    major: f64,
    minor: f64,
    axis: Point,
    solid_depth_clip: bool,
}

impl AnalyticFace {
    /// An axial or edge-on ring torus in drawing coordinates.
    pub fn torus(
        center: Point,
        axis: Point,
        [major, minor]: [f64; 2],
    ) -> Result<Self, ProjectionError> {
        Self::torus_face(center, axis, [major, minor], false)
    }

    /// A retained ring torus, including material on the exposed depth caps.
    pub fn retained_torus(
        center: Point,
        axis: Point,
        radii: [f64; 2],
        depth: DepthRange,
    ) -> Result<Option<Self>, ProjectionError> {
        Ok(Self::torus_face(center, axis, radii, true)?.retained(depth))
    }

    fn torus_face(
        center: Point,
        axis: Point,
        [major, minor]: [f64; 2],
        solid_depth_clip: bool,
    ) -> Result<Self, ProjectionError> {
        if !bounded(center)
            || !(1. - 1e-9..=1. + 1e-9).contains(&dot(axis, axis))
            || !(1e-7..=1e7).contains(&major)
            || !(1e-7..major).contains(&minor)
        {
            return Err(ProjectionError::Invalid);
        }
        let projected = axis[0].hypot(axis[1]);
        let axis = if projected <= ANGULAR {
            [0., 0., 1.]
        } else if axis[2].abs() <= ANGULAR {
            [axis[0] / projected, axis[1] / projected, 0.]
        } else {
            return Err(ProjectionError::Invalid);
        };
        let extent: Point =
            std::array::from_fn(|i| major * axis[(i + 1) % 3].hypot(axis[(i + 2) % 3]) + minor);
        Ok(Self {
            surface: Surface::Torus(Torus {
                center,
                major,
                minor,
                axis,
                solid_depth_clip,
            }),
            bounds: Bounds {
                min: sub(center, extent),
                max: std::array::from_fn(|i| center[i] + extent[i]),
            },
            depth: [None; 2],
        })
    }
}

impl Torus {
    pub(super) fn hide(
        &self,
        path: Path,
        span: Span,
        gap: f64,
        roundoff: f64,
        limits: [Option<f64>; 2],
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        let axial = self.axis[2] == 1.;
        let [u, v] = if axial {
            [[1., 0., 0.], [0., 1., 0.]]
        } else {
            [[-self.axis[1], self.axis[0], 0.], self.axis]
        };
        let [u, v, depth] = [u, v, [0., 0., 1.]].map(|axis| path.coordinate(self.center, axis));
        let [low, high] = limits.map(|limit| limit.map(|value| value - self.center[2]));
        let [outer, inner] = [self.major + self.minor, self.major - self.minor];
        let gap_depth = depth.offset(gap);
        if axial {
            if limits == [None; 2]
                && [outer, inner]
                    .into_iter()
                    .any(|radius| path.coincident_circle(self.center, radius, gap, roundoff))
            {
                work.hidden.push(span);
                return Ok(());
            }
            for radius in [inner + roundoff, outer - roundoff] {
                u.circle(v, radius, span, &mut work.crossings);
            }
        } else {
            let extent = self.minor - roundoff;
            for side in [-1., 1.] {
                v.level(side * extent, span, &mut work.crossings);
                u.offset(-side * self.major)
                    .circle(v, extent, span, &mut work.crossings);
            }
        }
        work.charge(if path.harmonic { 384 } else { 128 })?;
        let mut equal_depth = |z: Trace| {
            let coordinates = if axial { [u, v, z] } else { [u, z, v] };
            torus_events(
                coordinates,
                self.major,
                self.minor,
                span,
                &mut work.crossings,
            );
        };
        equal_depth(gap_depth);
        for level in [low, high].into_iter().flatten() {
            equal_depth(Trace {
                coefficients: [level, 0., 0.],
                harmonic: path.harmonic,
            });
        }
        drawing_events::spans(
            &mut work.crossings,
            span,
            |t| {
                let [u, v] = [u.at(t), v.at(t)];
                let radial_offset = if axial {
                    (u.hypot(v) - self.major).abs()
                } else {
                    v.abs()
                };
                let Some(half) = chord(self.minor, radial_offset, roundoff) else {
                    return false;
                };
                let outer = if axial {
                    half
                } else {
                    let Some(outer) = chord(self.major + half, u.abs(), roundoff) else {
                        return false;
                    };
                    outer
                };
                let candidate = gap_depth.at(t);
                if !self.solid_depth_clip {
                    return outer > candidate
                        && low.is_none_or(|low| outer >= low)
                        && high.is_none_or(|high| outer <= high);
                }
                let inner = if axial {
                    0.
                } else {
                    chord(self.major - half, u.abs(), 0.).unwrap_or(0.)
                };
                let low = low.unwrap_or(f64::NEG_INFINITY).max(candidate);
                let high = high.unwrap_or(f64::INFINITY);
                [[-outer, -inner], [inner, outer]]
                    .into_iter()
                    .any(|[lo, hi]| hi.min(high) > lo.max(low))
            },
            |span| work.hidden.push(span),
        );
        Ok(())
    }
}

fn chord(radius: f64, offset: f64, roundoff: f64) -> Option<f64> {
    (offset < radius - roundoff).then(|| ((radius - offset) * (radius + offset)).sqrt())
}

// The last coordinate is along the torus axis. A line has an affine
// chart, and a rim has three bounded rational half-angle charts.
fn torus_events(traces: [Trace; 3], major: f64, minor: f64, [lo, hi]: Span, cuts: &mut Vec<f64>) {
    let solve = |numerators, weight, domain| {
        let polynomial = implicit(numerators, weight, major, minor);
        let extrema = crate::curved_projection::derivative(&polynomial);
        roots(&polynomial, domain)
            .into_iter()
            .chain(roots(&extrema, domain))
    };
    if !traces[0].harmonic {
        let scale = hi - lo;
        let numerators = traces.map(|trace| [trace.at(lo), trace.coefficients[1] * scale, 0.]);
        cuts.extend(solve(numerators, [1., 0., 0.], [0., 1.]).map(|t| scale.mul_add(t, lo)));
        return;
    }
    for (phase, sign) in [(0., 1.), (std::f64::consts::PI, -1.), (TAU, 1.)] {
        let start = lo.max(phase - FRAC_PI_2);
        let end = hi.min(phase + FRAC_PI_2);
        if start >= end {
            continue;
        }
        let numerators = traces.map(|trace| {
            let [a, b, c] = trace.coefficients;
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

fn implicit([u, v, axial]: [[f64; 3]; 3], weight: [f64; 3], major: f64, minor: f64) -> [f64; 9] {
    let uu = product::<3, 5>(u, u);
    let vv = product::<3, 5>(v, v);
    let a2 = product::<3, 5>(axial, axial);
    let w2 = product::<3, 5>(weight, weight);
    let radial: [f64; 5] = std::array::from_fn(|i| uu[i] + vv[i]);
    let offset = (major - minor) * (major + minor);
    let sum: [f64; 5] = std::array::from_fn(|i| w2[i].mul_add(offset, radial[i] + a2[i]));
    let squared = product::<5, 9>(sum, sum);
    let weighted_radial = product::<5, 9>(radial, w2);
    let factor = -4. * major * major;
    std::array::from_fn(|i| factor.mul_add(weighted_radial[i], squared[i]))
}

fn product<const N: usize, const M: usize>(a: [f64; N], b: [f64; N]) -> [f64; M] {
    let mut result = [0.; M];
    for (i, x) in a.into_iter().enumerate() {
        for (j, y) in b.into_iter().enumerate() {
            result[i + j] = x.mul_add(y, result[i + j]);
        }
    }
    result
}
