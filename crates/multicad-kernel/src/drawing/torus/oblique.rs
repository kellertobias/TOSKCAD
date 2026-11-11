//! Oblique silhouettes and plane cuts of a ring torus.
use super::*;
use multicad_rust_kernel::planar_projection::{ContourCandidate, LineCandidate};
use std::f64::consts::FRAC_PI_2;

type Point = [f64; 2];

#[derive(Clone, Copy)]
struct Harmonic {
    constant: f64,
    phase: f64,
    radius: f64,
}

impl Harmonic {
    fn new(constant: f64, cosine: f64, sine: f64) -> Self {
        Self {
            constant,
            phase: sine.atan2(cosine),
            radius: cosine.hypot(sine),
        }
    }

    fn at(self, t: f64) -> f64 {
        self.radius.mul_add((t - self.phase).cos(), self.constant)
    }

    fn roots(self) -> Vec<f64> {
        let quotient = -self.constant / self.radius;
        if !(-1.0..=1.).contains(&quotient) {
            return Vec::new();
        }
        let delta = quotient.acos();
        [self.phase - delta, self.phase + delta]
            .map(|t| t.rem_euclid(TAU))
            .to_vec()
    }

    fn range(self, [lo, hi]: [f64; 2]) -> [f64; 2] {
        let [a, b] = [self.at(lo), self.at(hi)];
        let extremum = |phase: f64| (phase - lo).rem_euclid(TAU) <= hi - lo;
        [
            if extremum(self.phase + PI) {
                self.constant - self.radius
            } else {
                a.min(b)
            },
            if extremum(self.phase) {
                self.constant + self.radius
            } else {
                a.max(b)
            },
        ]
    }
}

struct Slice {
    factors: [Harmonic; 2],
    axial: Harmonic,
    inclination: f64,
}

impl Slice {
    fn point(&self, t: f64, side: f64, seam: bool) -> Point {
        let t = t.rem_euclid(TAU);
        let [a, b] = self.factors.map(|f| f.at(t).max(0.));
        [
            self.axial.at(t),
            if seam {
                0.
            } else {
                side * (a * b).sqrt() / self.inclination
            },
        ]
    }

    fn error(&self, span: [f64; 2]) -> f64 {
        let [a, b] = self
            .factors
            .map(|f| f.range(span).map(|value| value.max(0.)));
        let minimum = (a[0] * b[0]).sqrt();
        let maximum = (a[1] * b[1]).sqrt();
        let axial = self.axial.range(span);
        let enclosure = (axial[1] - axial[0]).hypot((maximum - minimum) / self.inclination);
        if minimum == 0. {
            return enclosure;
        }
        // The second derivative of sqrt(a*b) bounds the interpolation
        // remainder. Each harmonic's amplitude also bounds its derivatives.
        let [da, db] = self.factors.map(|f| f.radius);
        let first = da * b[1] + db * a[1];
        let second = first + 2. * da * db;
        let derivative =
            (second / (2. * minimum) + first.powi(2) / (4. * minimum.powi(3))) / self.inclination;
        let remainder = self.axial.radius.hypot(derivative) * (span[1] - span[0]).powi(2) / 8.;
        remainder.min(enclosure)
    }

    fn sample(
        &self,
        span: [f64; 2],
        side: f64,
        seams: bool,
        tolerance: f64,
        remaining: &mut usize,
    ) -> Result<Vec<Point>, GeometryError> {
        let ends = span.map(|t| self.point(t, side, seams));
        let mut points = vec![ends[0]];
        let mut pending = vec![(span, ends)];
        while let Some(([lo, hi], ends)) = pending.pop() {
            if self.error([lo, hi]) <= tolerance {
                *remaining = remaining.checked_sub(1).ok_or_else(limit)?;
                points.push(ends[1]);
                continue;
            }
            let mid = lo.midpoint(hi);
            if mid <= lo || mid >= hi || pending.len() >= 52 {
                return Err(limit());
            }
            let point = self.point(mid, side, false);
            pending.push(([mid, hi], [point, ends[1]]));
            pending.push(([lo, mid], [ends[0], point]));
        }
        points.pop();
        Ok(points)
    }
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust oblique torus drawing exceeded its chord or output budget",
    )
}

pub(super) fn section(
    input: &DrawingSolid<'_>,
    center: [f64; 3],
    axis: [f64; 3],
    [major, minor]: [f64; 2],
    options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let [a, b, c] = axis;
    let s = a.hypot(b);
    let offset = depth - center[2];
    if offset.abs() >= s * major + minor {
        return Ok(Vec::new());
    }
    let slice = Slice {
        factors: [-1., 1.]
            .map(|sign| Harmonic::new(s * major + sign * offset, s * minor, -sign * c * minor)),
        axial: Harmonic::new(-c * offset / s, 0., minor / s),
        inclination: s,
    };
    let mut cuts: Vec<_> = slice
        .factors
        .into_iter()
        .flat_map(Harmonic::roots)
        .collect();
    cuts.sort_unstable_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() <= ANGULAR);
    let mut remaining = (options.maximum_curves as usize).min(options.maximum_points as usize / 2);
    let tolerance = options.chord_tolerance_mm / 4.;
    let mut rings = Vec::new();
    if cuts.is_empty() {
        for side in [-1., 1.] {
            rings.push(slice.sample([0., TAU], side, false, tolerance, &mut remaining)?);
        }
    } else {
        cuts.push(cuts[0] + TAU);
        for pair in cuts.windows(2) {
            let span = [pair[0], pair[1]];
            if slice
                .factors
                .into_iter()
                .any(|f| f.at(span[0].midpoint(span[1])) <= 0.)
            {
                continue;
            }
            let mut ring = slice.sample(span, 1., true, tolerance, &mut remaining)?;
            let back = slice.sample(span, -1., true, tolerance, &mut remaining)?;
            ring.push(slice.point(span[1], -1., true));
            ring.extend(back.into_iter().skip(1).rev());
            rings.push(ring);
        }
    }
    let rings = rings
        .into_iter()
        .map(|ring| {
            let mut ring: Vec<_> = ring
                .into_iter()
                .map(|[u, v]| [(a * u + b * v) / s, (b * u - a * v) / s])
                .collect();
            // Orient by the outward surface normal so nested hole loops
            // keep material on their left as well as the exterior loops.
            let orientation: f64 = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(p, q)| {
                    let xyz = [p[0], p[1], offset];
                    let axial = xyz.into_iter().zip(axis).map(|(x, d)| x * d).sum::<f64>();
                    let radial: [f64; 3] = std::array::from_fn(|i| xyz[i] - axis[i] * axial);
                    let radius = radial[0].hypot(radial[1]).hypot(radial[2]);
                    let normal: Point = std::array::from_fn(|i| {
                        radial[i] * (1. - major / radius) + axis[i] * axial
                    });
                    normal[0] * (q[1] - p[1]) - normal[1] * (q[0] - p[0])
                })
                .sum();
            if orientation < 0. {
                ring.reverse();
            }
            for point in &mut ring {
                for i in 0..2 {
                    point[i] += center[i];
                }
            }
            ring
        })
        .collect();
    section::sampled::publish(input, rings, options)
}

struct Silhouette {
    center: [f64; 3],
    projected_axis: Point,
    major: f64,
    minor: f64,
    s: f64,
    c: f64,
}

impl Silhouette {
    fn point(&self, t: f64, side: f64) -> [f64; 3] {
        let (sin, cos) = t.rem_euclid(TAU).sin_cos();
        let normal = self.c.hypot(self.s * cos);
        let u = -cos * (self.c * self.major + side * self.minor / normal);
        let v = sin * (self.major + side * self.minor * self.c / normal);
        let [a, b] = self.projected_axis;
        [
            a.mul_add(u, b.mul_add(v, self.center[0])),
            b.mul_add(u, (-a).mul_add(v, self.center[1])),
            (self.s * self.major).mul_add(cos, self.center[2]),
        ]
    }

    fn chord_error(&self, [lo, hi]: [f64; 2]) -> f64 {
        // Cuts at the quadrant boundaries ensure |cos| is monotone here.
        let [a, b] = [lo.cos().abs(), hi.cos().abs()];
        let normal = self.c.hypot(self.s * a.min(b));
        let c2 = self.c * self.c;
        let g = normal.recip();
        let g1 = self.s.powi(2) / (2. * normal.powi(3));
        let g2 = self.s.powi(2) / normal.powi(3) + 3. * self.s.powi(4) / (4. * normal.powi(5));
        let t_over_normal =
            c2 * (a.max(b) / normal.powi(3) + 3. * self.s.powi(2) * a.max(b) / normal.powi(5));
        let u2 = self.c * self.major + self.minor * t_over_normal;
        let v2 = self.major + self.minor * self.c * (g + 2. * g1 + g2);
        u2.hypot(v2).hypot(self.s * self.major) * (hi - lo).powi(2) / 8.
    }

    fn hidden(&self, angle: f64, side: f64, near: Option<f64>) -> bool {
        if side > 0. {
            return false;
        }
        let cos = angle.cos();
        let normal = self.c.hypot(self.s * cos);
        // Deflate the viewing-ray quartic by its double silhouette root
        // z = R*s*cos(t). The remaining quadratic supplies the other lobe.
        let hidden = if cos < 0. {
            normal < self.minor / (self.major * self.c)
        } else {
            normal.powi(3) < self.c * self.minor / self.major
        };
        hidden
            && near.is_none_or(|near| {
                let half = (4. * self.major * self.c * (self.minor / normal - self.major * self.c))
                    .max(0.)
                    .sqrt();
                -(self.s * self.major * cos) - half < near
            })
    }
}

pub(in crate::drawing) fn contours(
    center: [f64; 3],
    mut axis: [f64; 3],
    [major, minor]: [f64; 2],
    options: DrawingProjection,
    source: usize,
    face: usize,
) -> Result<(Vec<ContourCandidate>, Vec<LineCandidate>), GeometryError> {
    if axis[2] < 0. {
        axis = axis.map(|component| -component);
    }
    let s = axis[0].hypot(axis[1]);
    let shape = Silhouette {
        center,
        projected_axis: [axis[0] / s, axis[1] / s],
        major,
        minor,
        s,
        c: axis[2],
    };
    let mut visible = Vec::new();
    let mut hidden = Vec::new();
    let mut remaining = (options.maximum_curves as usize).min(options.maximum_points as usize / 2);
    let near = options.section_offset_mm.map(|depth| depth - center[2]);
    let far = near
        .zip(options.section_depth_mm)
        .map(|(near, depth)| near - depth);
    let depth_cuts: Vec<_> = [near, far]
        .into_iter()
        .flatten()
        .filter_map(|depth| {
            let cos = depth / (s * major);
            (-1. ..=1.).contains(&cos).then(|| cos.acos())
        })
        .flat_map(|t| [t, TAU - t])
        .collect();
    let quarters = [0., FRAC_PI_2, PI, 3. * FRAC_PI_2, TAU];
    for side in [1., -1.] {
        let mut cuts = quarters.to_vec();
        cuts.extend_from_slice(&depth_cuts);
        if side < 0. {
            for normal in [minor / (major * shape.c), (shape.c * minor / major).cbrt()] {
                if normal <= shape.c || normal >= 1. {
                    continue;
                }
                let cosine = ((normal - shape.c) * (normal + shape.c)).sqrt() / s;
                let angle = cosine.clamp(0., 1.).acos();
                cuts.extend([angle, PI - angle, PI + angle, TAU - angle]);
            }
            if let Some(near) = near {
                for cosine in
                    AnalyticFace::oblique_torus_cap_crossings([major, minor], [s, shape.c], near)
                {
                    let angle = cosine.acos();
                    cuts.extend([angle, TAU - angle]);
                }
            }
        }
        cuts.sort_unstable_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() <= ANGULAR);
        for pair in cuts.windows(2) {
            let span = [pair[0], pair[1]];
            let middle = span[0].midpoint(span[1]);
            let depth = s * major * middle.cos();
            if near.is_some_and(|near| depth > near) || far.is_some_and(|far| depth < far) {
                continue;
            }
            let is_hidden = shape.hidden(middle, side, near);
            if is_hidden && !options.hidden_lines {
                continue;
            }
            let mut pending = vec![(span, span.map(|t| shape.point(t, side)))];
            while let Some(([lo, hi], points)) = pending.pop() {
                if shape.chord_error([lo, hi]) <= options.chord_tolerance_mm / 2. {
                    remaining = remaining.checked_sub(1).ok_or_else(limit)?;
                    let line = LineCandidate { source, points };
                    if is_hidden {
                        hidden.push(line);
                    } else {
                        visible.push(ContourCandidate { line, face });
                    }
                    continue;
                }
                let mid = lo.midpoint(hi);
                if mid <= lo || mid >= hi || pending.len() >= 52 {
                    return Err(limit());
                }
                let point = shape.point(mid, side);
                pending.push(([mid, hi], [point, points[1]]));
                pending.push(([lo, mid], [points[0], point]));
            }
        }
    }
    Ok((visible, hidden))
}
