//! Clip rational section curves in paper coordinates without flattening them.
use super::*;

pub(super) fn half_plane(
    curves: Vec<Curve>,
    axis: usize,
    bound: f64,
    sign: f64,
    tolerance: f64,
) -> Result<Vec<Curve>, GeometryError> {
    let distance = |point: Point| {
        let delta = point[axis] - bound;
        let epsilon =
            (32. * f64::EPSILON * point[axis].abs().max(bound.abs()).max(1.)).min(tolerance);
        if delta.abs() <= epsilon {
            0.
        } else {
            sign * delta
        }
    };
    let mut output = Vec::with_capacity(curves.len());
    for curve in curves {
        match curve {
            Curve::Line([a, b]) => {
                let [d, e] = [a, b].map(distance);
                if d >= 0. && e >= 0. {
                    output.push(curve);
                } else if d * e < 0. {
                    let mut hit = interpolate(a, b, d / (d - e));
                    hit[axis] = bound;
                    output.push(Curve::Line(if d > 0. { [a, hit] } else { [hit, b] }));
                }
            }
            Curve::Quadratic(controls) => {
                let depths = controls.map(|control| distance(control.point));
                if depths.iter().all(|&d| d >= 0.) {
                    output.push(curve);
                } else if depths.iter().any(|&d| d > 0.) {
                    let weighted = std::array::from_fn(|i| depths[i] * controls[i].weight);
                    for [start, end] in positive_intervals(weighted) {
                        let mut trimmed = controls;
                        if end < 1. {
                            trimmed = subdivide(trimmed, end)[0];
                        }
                        if start > 0. {
                            trimmed = subdivide(trimmed, start / end)[1];
                            trimmed[0].point[axis] = bound;
                        }
                        if end < 1. {
                            trimmed[2].point[axis] = bound;
                        }
                        output.push(Curve::Quadratic(trimmed));
                    }
                }
            }
        }
    }
    // Every odd incidence on the clip line needs one straight closing edge.
    // Pair adjacent events along the line, including disjoint material spans.
    let other = 1 - axis;
    let mut ends: Vec<_> = output
        .iter()
        .flat_map(|curve| curve.ends())
        .filter(|&point| distance(point) == 0.)
        .collect();
    ends.sort_unstable_by(|a, b| a[other].total_cmp(&b[other]));
    let mut pending = None;
    for group in ends.chunk_by(|a, b| {
        (a[other] - b[other]).abs()
            <= (32. * f64::EPSILON * a[other].abs().max(b[other].abs()).max(1.)).min(tolerance)
    }) {
        if group.len().is_multiple_of(2) {
            continue;
        }
        let point = group[0];
        if let Some(start) = pending.take() {
            output.push(Curve::Line([start, point]));
        } else {
            pending = Some(point);
        }
    }
    if pending.is_some() {
        return Err(err(
            GeometryErrorKind::InvalidResult,
            "Rust loft section window encountered an open boundary",
        ));
    }
    Ok(output)
}

fn positive_intervals([start, control, end]: [f64; 3]) -> Vec<[f64; 2]> {
    // These are the power coefficients of the signed homogeneous numerator.
    let a = (start - control) + (end - control);
    let b = 2. * (control - start);
    let c = start;
    let mut knots = [0., 1., 1., 1.];
    let mut count = 1;
    let mut add = |root| {
        if root > 0. && root < 1. {
            knots[count] = root;
            count += 1;
        }
    };
    if a == 0. {
        add(-c / b);
    } else {
        let discriminant = b.mul_add(b, -4. * a * c);
        if discriminant > 0. {
            let q = -0.5 * (b + discriminant.sqrt().copysign(b));
            add(q / a);
            add(c / q);
        } else if discriminant == 0. {
            add(-b / (2. * a));
        }
    }
    let knots = &mut knots[..=count];
    knots.sort_unstable_by(f64::total_cmp);
    let mut intervals: Vec<[f64; 2]> = Vec::new();
    for pair in knots.windows(2) {
        let t = (pair[0] + pair[1]) * 0.5;
        if a.mul_add(t, b).mul_add(t, c) > 0. {
            if let Some(previous) = intervals.last_mut()
                && previous[1] == pair[0]
            {
                previous[1] = pair[1];
            } else {
                intervals.push([pair[0], pair[1]]);
            }
        }
    }
    intervals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_handles_disjoint_spans_and_double_roots() {
        assert_eq!(positive_intervals([1., -1., 1.]), [[0., 1.]]);
        assert!(positive_intervals([-1., 1., -1.]).is_empty());
        let intervals = positive_intervals([1., -3., 1.]);
        assert_eq!(intervals.len(), 2);
        assert_eq!(intervals[0][0], 0.);
        assert_eq!(intervals[1][1], 1.);
        assert!((intervals[0][1] - 0.5 * (1. - 0.5_f64.sqrt())).abs() < 1e-15);
        assert!((intervals[1][0] - (1. - intervals[0][1])).abs() < 1e-15);
        assert_eq!(positive_intervals([-1., 0., 1.]), [[0.5, 1.]]);
    }
}
