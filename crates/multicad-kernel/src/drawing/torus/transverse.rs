//! Offset transverse cuts of a ring torus. The general section is a quartic.
//! Export its analytic intersection as chords bounded by the paper tolerance.
use super::*;
use std::f64::consts::FRAC_PI_2;

type Point = [f64; 2];

struct Cut {
    major: f64,
    minor: f64,
    offset: f64,
}

impl Cut {
    fn inner(&self) -> f64 {
        self.major - self.minor
    }

    fn outer(&self) -> f64 {
        self.major + self.minor
    }

    fn across(&self, radial: f64) -> f64 {
        ((radial - self.offset).max(0.) * (radial + self.offset)).sqrt()
    }

    fn point(&self, branch: Branch, t: f64) -> Point {
        // Exact quadrant endpoints close tangent loops without a residual gap.
        let (sin, cos) = if t == 0. || t == TAU {
            (0., 1.)
        } else if t == PI {
            (0., -1.)
        } else if t == FRAC_PI_2 {
            (1., 0.)
        } else if t == 3. * FRAC_PI_2 {
            (-1., 0.)
        } else {
            t.sin_cos()
        };
        match branch {
            Branch::Tube(sign) => [
                sign * self.across(self.minor.mul_add(cos, self.major)),
                -sign * self.minor * sin,
            ],
            Branch::Merged => {
                let extent = self.across(self.outer());
                let x = extent * sin;
                let radial = x.hypot(self.offset);
                let ratio = (radial - self.inner()).max(0.) / (self.outer() + radial);
                [x, extent * cos * ratio.sqrt()]
            }
        }
    }

    fn second_derivative(&self, branch: Branch, ends: [Point; 2]) -> f64 {
        // Every interval lies between coordinate extrema. Endpoints bound
        // the radial coordinate and, on a tube branch, |sin(t)|.
        let [a, b] = ends;
        let xmin = a[0].abs().min(b[0].abs());
        let radial = xmin.hypot(self.offset);
        match branch {
            Branch::Tube(_) => {
                let axial = a[1].abs().max(b[1].abs());
                let curvature =
                    self.offset.powi(2) * axial.powi(2) / xmin.powi(3) + self.minor * radial / xmin;
                curvature.hypot(self.minor)
            }
            Branch::Merged => {
                let extent = self.across(self.outer());
                let sum = self.outer() + radial;
                let f = ((radial - self.inner()).max(0.) / sum).sqrt();
                if f == 0. {
                    return f64::INFINITY;
                }
                let g_prime = 2. * self.major / sum.powi(2);
                let g_second = 2. * g_prime / sum;
                let f_prime = g_prime / (2. * f);
                let f_second = g_second / (2. * f) + g_prime.powi(2) / (4. * f.powi(3));
                let xmax = a[0].abs().max(b[0].abs());
                let radial_prime = (xmax / xmax.hypot(self.offset)) * extent;
                let radial_second = (extent.powi(2) + xmax.powi(2)) / radial;
                let y_second = extent
                    * (1.
                        + f_prime * (2. * radial_prime + radial_second)
                        + f_second * radial_prime.powi(2));
                extent.hypot(y_second)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Branch {
    Tube(f64),
    Merged,
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust torus section exceeded its chord or output budget",
    )
}

fn append_chords(
    cut: &Cut,
    branch: Branch,
    stations: &[f64],
    tolerance: f64,
    remaining: &mut usize,
) -> Result<Vec<Point>, GeometryError> {
    let mut points = vec![cut.point(branch, stations[0])];
    for station in stations.windows(2) {
        let [start, end] = [station[0], station[1]];
        let mut pending = vec![(
            [start, end],
            [cut.point(branch, start), cut.point(branch, end)],
        )];
        while let Some(([lo, hi], ends)) = pending.pop() {
            let [du, dv] = std::array::from_fn(|axis| (ends[1][axis] - ends[0][axis]).abs());
            let length = du.hypot(dv);
            let box_error = if length > 0. { du * dv / length } else { 0. };
            let second_error = cut.second_derivative(branch, ends) * (hi - lo).powi(2) / 8.;
            // Either the monotone rectangle or the interpolation remainder
            // bounds the distance to the chord over the entire interval.
            if box_error.min(second_error) <= tolerance {
                *remaining = remaining.checked_sub(1).ok_or_else(limit)?;
                points.push(ends[1]);
            } else {
                let mid = (lo + hi) / 2.;
                if mid <= lo || mid >= hi || pending.len() >= 52 {
                    return Err(limit());
                }
                let point = cut.point(branch, mid);
                pending.push(([mid, hi], [point, ends[1]]));
                pending.push(([lo, mid], [ends[0], point]));
            }
        }
    }
    points.pop();
    Ok(points)
}

pub(super) fn project(
    input: &DrawingSolid<'_>,
    center: Point,
    across: Point,
    [major, minor]: [f64; 2],
    offset: f64,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let cut = Cut {
        major,
        minor,
        offset,
    };
    let mut remaining = (options.maximum_curves as usize).min(options.maximum_points as usize / 2);
    let policy = input.solid.geometry_policy();
    let tolerance = policy.linear_tolerance_mm;
    let deflection = options.chord_tolerance_mm / 2.;
    if cut.across(cut.outer()) <= tolerance {
        return Ok(Vec::new());
    }
    let quarters = [0., FRAC_PI_2, PI, 3. * FRAC_PI_2, TAU];
    let rings = if offset < cut.inner() {
        [-1., 1.]
            .map(|sign| {
                append_chords(
                    &cut,
                    Branch::Tube(sign),
                    &quarters,
                    deflection,
                    &mut remaining,
                )
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let mut stations = quarters.to_vec();
        if offset < major {
            let angle = (cut.across(major) / cut.across(cut.outer())).asin();
            stations = vec![
                0.,
                angle,
                FRAC_PI_2,
                PI - angle,
                PI,
                PI + angle,
                3. * FRAC_PI_2,
                TAU - angle,
                TAU,
            ];
        }
        if offset == cut.inner() {
            let middle = stations.len() / 2;
            [&stations[..=middle], &stations[middle..]]
                .into_iter()
                .map(|s| append_chords(&cut, Branch::Merged, s, deflection, &mut remaining))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![append_chords(
                &cut,
                Branch::Merged,
                &stations,
                deflection,
                &mut remaining,
            )?]
        }
    };
    let axial = [across[1], -across[0]];
    let rings = rings
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|[x, y]| {
                    std::array::from_fn(|i| across[i].mul_add(x, axial[i].mul_add(y, center[i])))
                })
                .collect()
        })
        .collect();
    section::sampled::publish(input, rings, options)
}
