//! Straight sections swept between two plane trims.
//!
//! Every generator starts on the same XY section and runs along a common
//! unit direction. Affine travel distances define both caps. An invertible
//! shear maps the result back to a straight section with variable thickness.

use crate::{
    linear_sweep::{MAX_COORDINATE_MM, validate_cap},
    polygon::MAX_VERTICES,
    revolution::{edges, inside, sub},
};

type Point = [f64; 3];
type Uv = [f64; 2];
type Travel = [f64; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidProfile,
    InvalidDirection,
    InvalidExtent,
    Limit,
}

#[derive(Clone, Copy, Debug)]
struct Cap {
    travel: Travel,
    centroid: Point,
    outward: Point,
    area_mm2: f64,
}

#[derive(Clone, Debug)]
pub struct TrimmedSweep {
    rings: Vec<Vec<Uv>>,
    direction: Point,
    caps: [Cap; 2],
    volume_mm3: f64,
}

impl TrimmedSweep {
    /// The outer ring winds counterclockwise in XY and holes wind clockwise.
    /// The unit direction crosses the section toward +Z. Each cap specifies
    /// travel along the direction as `constant + dx * u + dy * v`, with a
    /// positive gap of at least `minimum_thickness_mm` on every generator.
    pub fn new(
        rings: Vec<Vec<Uv>>,
        direction: Point,
        travels: [Travel; 2],
        minimum_thickness_mm: f64,
    ) -> Result<Self, Error> {
        if !minimum_thickness_mm.is_finite() || minimum_thickness_mm <= 0. {
            return Err(Error::InvalidExtent);
        }
        let [dx, dy, dz] = direction;
        if direction.iter().any(|v| !v.is_finite())
            || dz < 1e-10
            || (dot(direction, direction) - 1.).abs() > 1e-10
        {
            return Err(Error::InvalidDirection);
        }
        if rings.is_empty() || rings.iter().any(|ring| ring.len() < 3) {
            return Err(Error::InvalidProfile);
        }
        if rings.iter().map(Vec::len).sum::<usize>() > MAX_VERTICES {
            return Err(Error::Limit);
        }
        let mut scale = 1_f64;
        for &coordinate in rings.iter().flatten().flatten() {
            if !coordinate.is_finite() {
                return Err(Error::InvalidProfile);
            }
            if coordinate.abs() > MAX_COORDINATE_MM {
                return Err(Error::Limit);
            }
            scale = scale.max(coordinate.abs());
        }
        let section =
            validate_cap(&rings, 64. * f64::EPSILON * scale).map_err(|_| Error::InvalidProfile)?;
        if travels.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::InvalidExtent);
        }
        let gap = std::array::from_fn(|i| travels[1][i] - travels[0][i]);
        for &uv in rings.iter().flatten() {
            let distances = travels.map(|travel| evaluate(travel, uv));
            let span = distances[1] - distances[0];
            let error = travels
                .iter()
                .map(|&travel| roundoff(travel, uv))
                .sum::<f64>();
            // The gap is affine, so positivity at the vertices proves it
            // throughout the entire section, including concave outlines.
            if !span.is_finite() || span < minimum_thickness_mm || span <= error {
                return Err(Error::InvalidExtent);
            }
            for distance in distances {
                if distance.abs() > MAX_COORDINATE_MM
                    || position(direction, uv, distance)
                        .iter()
                        .any(|p| p.abs() > MAX_COORDINATE_MM)
                {
                    return Err(Error::Limit);
                }
            }
        }
        let caps = [false, true].map(|end| {
            let travel @ [_, a, b] = travels[usize::from(end)];
            // This is the cross product of the affine cap's U/V tangents.
            // Its dot product with the sweep direction is positive (dz).
            let normal = [-dz * a, -dz * b, dy.mul_add(b, dx.mul_add(a, 1.))];
            let scale = norm(normal);
            let outward = normal.map(|v| if end { v / scale } else { -v / scale });
            Cap {
                travel,
                outward,
                centroid: position(
                    direction,
                    section.centroid,
                    evaluate(travel, section.centroid),
                ),
                area_mm2: section.area_mm2 * scale,
            }
        });
        // Integrate the affine travel gap over the section by evaluating it
        // at the centroid. The shear's Jacobian determinant is direction.z.
        let volume_mm3 = dz * section.area_mm2 * evaluate(gap, section.centroid);
        Ok(Self {
            rings,
            direction,
            caps,
            volume_mm3,
        })
    }

    pub fn rings(&self) -> &[Vec<Uv>] {
        &self.rings
    }

    pub fn direction(&self) -> Point {
        self.direction
    }

    pub fn travel(&self, end: bool, uv: Uv) -> f64 {
        evaluate(self.caps[usize::from(end)].travel, uv)
    }

    /// The retained affine cap laws, without inferring them from rounded
    /// evaluated normals or point differences.
    pub fn travel_coefficients(&self) -> [Travel; 2] {
        self.caps.map(|cap| cap.travel)
    }

    pub fn point(&self, end: bool, uv: Uv) -> Point {
        position(self.direction, uv, self.travel(end, uv))
    }

    pub fn cap_normal(&self, end: bool) -> Point {
        self.caps[usize::from(end)].outward
    }

    pub fn cap_centroid(&self, end: bool) -> Point {
        self.caps[usize::from(end)].centroid
    }

    pub fn cap_area_mm2(&self, end: bool) -> f64 {
        self.caps[usize::from(end)].area_mm2
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    pub fn support(&self, direction: Point) -> f64 {
        if direction.iter().any(|v| !v.is_finite()) {
            return f64::NAN;
        }
        let scale = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if scale == 0. {
            return 0.;
        }
        let direction = direction.map(|v| v / scale);
        self.rings[0]
            .iter()
            .flat_map(|&uv| [self.point(false, uv), self.point(true, uv)])
            .map(|point| dot(point, direction))
            .fold(f64::NEG_INFINITY, f64::max)
            * scale
    }

    /// Classify material by undoing the sweep shear. Tolerance is measured
    /// perpendicular to the cap planes and the generators.
    pub fn contains(&self, point: Point, tolerance_mm: f64) -> bool {
        if !point.iter().all(|v| v.is_finite()) || !tolerance_mm.is_finite() || tolerance_mm < 0. {
            return false;
        }
        if self.caps.iter().any(|cap| {
            let offset = std::array::from_fn(|i| point[i] - cap.centroid[i]);
            dot(offset, cap.outward) > tolerance_mm
        }) {
            return false;
        }
        let [x, y, z] = point;
        let [dx, dy, dz] = self.direction;
        let t = z / dz;
        let uv = [(-dx).mul_add(t, x), (-dy).mul_add(t, y)];
        if !uv.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.rings.iter().enumerate().all(|(i, ring)| {
            inside(ring, uv) == (i == 0)
                || edges(ring)
                    .any(|edge| generator_distance(self.direction, uv, edge) <= tolerance_mm)
        })
    }
}

fn evaluate([constant, dx, dy]: Travel, [x, y]: Uv) -> f64 {
    dy.mul_add(y, dx.mul_add(x, constant))
}

fn roundoff([constant, dx, dy]: Travel, [x, y]: Uv) -> f64 {
    let bound = 8. * f64::EPSILON;
    constant.abs() * bound + (dx.abs() * bound) * x.abs() + (dy.abs() * bound) * y.abs()
}

fn position([dx, dy, dz]: Point, [x, y]: Uv, distance: f64) -> Point {
    [
        dx.mul_add(distance, x),
        dy.mul_add(distance, y),
        dz * distance,
    ]
}

fn dot(a: Point, b: Point) -> f64 {
    a[2].mul_add(b[2], a[1].mul_add(b[1], a[0] * b[0]))
}

fn norm([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

fn generator_distance(direction: Point, p: Uv, [a, b]: [Uv; 2]) -> f64 {
    let [dx, dy, dz] = direction;
    // Taking a cross product with the unit direction gives the metric
    // perpendicular to the generators without subtracting nearly equal
    // vectors for a nearly horizontal sweep.
    let across = |[x, y]: Uv| [y * dz, -x * dz, x.mul_add(dy, -y * dx)];
    let ray = across(sub(p, a));
    let edge = across(sub(b, a));
    let t = (dot(ray, edge) / dot(edge, edge)).clamp(0., 1.);
    norm(std::array::from_fn(|i| (-t).mul_add(edge[i], ray[i])))
}

#[cfg(test)]
mod tests;
