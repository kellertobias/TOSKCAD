//! Finite circular cylinder versus an unchanged straight polygon section.
//! The occupied section is integrated analytically; triangles are not used.
use super::AxialCylinder;
use crate::{
    linear_sweep::Sweep,
    revolution::{Sum, edges, inside},
};

type Point = [f64; 2];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unsupported,
    Unresolved,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    /// Finite nearest-section feature representatives, capped at 64 pairs.
    /// The first point belongs to the cylinder, the second to the prism.
    pub witnesses: Vec<[[f64; 3]; 2]>,
    pub total_solutions: u32,
    pub distance_mm: f64,
    pub distance_error_mm: f64,
    pub common_volume_mm3: f64,
    pub common_volume_error_mm3: f64,
}
fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn norm(a: Point) -> f64 {
    a[0].hypot(a[1])
}

/// All coordinates use the cylinder's lower-cap chart. The section is already
/// validated by Sweep; it must have identical lower and upper rings.
pub fn measure(cylinder: &AxialCylinder, prism: &Sweep, bottom_mm: f64) -> Result<Contact, Error> {
    if !bottom_mm.is_finite() || bottom_mm.abs() > 1e7 {
        return Err(Error::Invalid);
    }
    if prism.base() != prism.top() {
        return Err(Error::Unsupported);
    }
    let radius = cylinder.circle().radius().as_f64();
    let height = cylinder.height().as_f64();
    if radius > 1e7 || height > 1e7 {
        return Err(Error::Unsupported);
    }
    let rings = prism.base();
    let mut nearest = [f64::INFINITY; 2];
    let mut nearest_distance = f64::INFINITY;
    let mut candidates = Vec::new();
    for ring in rings {
        for [a, b] in edges(ring) {
            let d = sub(b, a);
            let t = (-dot(a, d) / dot(d, d)).clamp(0., 1.);
            let q = [d[0].mul_add(t, a[0]), d[1].mul_add(t, a[1])];
            let distance = norm(q);
            candidates.push(q);
            if distance < nearest_distance {
                nearest = q;
                nearest_distance = distance;
            }
        }
    }
    let center_inside =
        inside(&rings[0], [0.; 2]) && !rings[1..].iter().any(|ring| inside(ring, [0.; 2]));
    if center_inside {
        nearest = [0.; 2];
        nearest_distance = 0.;
    }
    let extent = rings
        .iter()
        .flatten()
        .copied()
        .map(norm)
        .fold(radius.max(1.), f64::max);
    let coordinate_error = 256. * f64::EPSILON * extent;
    let mut closest = Vec::new();
    if center_inside {
        closest.push([0.; 2]);
    } else {
        for candidate in candidates {
            if (norm(candidate) - nearest_distance).abs() <= coordinate_error
                && !closest
                    .iter()
                    .any(|&point| norm(sub(point, candidate)) <= coordinate_error)
            {
                closest.push(candidate);
            }
        }
    }
    // Do not turn an uncertain sliver or tangency into a false overlap/clearance.
    if (nearest_distance - radius).abs() <= coordinate_error {
        return Err(Error::Unresolved);
    }
    let (circle_xy, polygon_xy) = if nearest_distance > radius {
        (nearest.map(|p| radius * (p / nearest_distance)), nearest)
    } else {
        (nearest, nearest)
    };
    let top_mm = bottom_mm + prism.height_mm();
    let (cylinder_z, prism_z) = if bottom_mm > height {
        (height, bottom_mm)
    } else if top_mm < 0. {
        (0., top_mm)
    } else {
        let z = (bottom_mm.max(0.) + top_mm.min(height)) / 2.;
        (z, z)
    };
    let xy = norm(sub(circle_xy, polygon_xy));
    let distance = xy.hypot(cylinder_z - prism_z);
    let overlap_height = (height.min(top_mm) - bottom_mm.max(0.)).max(0.);
    let (area, area_error) = if nearest_distance > radius || overlap_height == 0. {
        (0., 0.)
    } else {
        intersection_area(rings, radius, extent)?
    };
    let axial_error = 16. * f64::EPSILON * (height + bottom_mm.abs() + prism.height_mm());
    let area_bound = if nearest_distance > radius {
        0.
    } else if overlap_height == 0. {
        std::f64::consts::PI * radius * radius
    } else {
        area + area_error
    };
    let total_solutions = closest.len() as u32;
    let witnesses = closest
        .into_iter()
        .take(64)
        .map(|point| {
            let circle = if nearest_distance > radius {
                point.map(|v| radius * (v / norm(point)))
            } else {
                point
            };
            [
                [circle[0], circle[1], cylinder_z],
                [point[0], point[1], prism_z],
            ]
        })
        .collect();
    Ok(Contact {
        witnesses,
        total_solutions,
        distance_mm: distance,
        distance_error_mm: coordinate_error + axial_error,
        common_volume_mm3: area * overlap_height,
        common_volume_error_mm3: area_error * overlap_height + area_bound * axial_error,
    })
}

fn intersection_area(rings: &[Vec<Point>], radius: f64, extent: f64) -> Result<(f64, f64), Error> {
    let mut area = Sum::default();
    let mut terms = 0;
    for ring in rings {
        for [a, b] in edges(ring) {
            let d = sub(b, a);
            let length = norm(d);
            let projection = -dot(a, d) / (length * length);
            let perpendicular = cross(a, d).abs() / length;
            let mut splits = vec![0., 1.];
            if perpendicular < radius {
                let delta = ((radius - perpendicular) * (radius + perpendicular)).sqrt() / length;
                for t in [projection - delta, projection + delta] {
                    if t > 0. && t < 1. {
                        splits.push(t);
                    }
                }
            }
            splits.sort_by(f64::total_cmp);
            for interval in splits.windows(2) {
                let point = |t: f64| [d[0].mul_add(t, a[0]), d[1].mul_add(t, a[1])];
                let p = point(interval[0]);
                let q = point(interval[1]);
                let midpoint = point((interval[0] + interval[1]) / 2.);
                area.add(if norm(midpoint) < radius {
                    cross(p, q) / 2.
                } else {
                    radius * radius * cross(p, q).atan2(dot(p, q)) / 2.
                });
                terms += 1;
            }
        }
    }
    let area = area.value();
    let error = 4096. * f64::EPSILON * terms as f64 * (extent * extent + radius * radius);
    if !area.is_finite() || area <= error || area > std::f64::consts::PI * radius * radius + error {
        return Err(Error::Unresolved);
    }
    Ok((area, error))
}

#[cfg(test)]
mod tests;
