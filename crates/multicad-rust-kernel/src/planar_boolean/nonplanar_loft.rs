//! Linear interpolation of corresponding convex sections on parallel planes.
use super::*;

mod area;
mod projection;
pub use projection::{LoftDrawingEdge, LoftProjection};
mod section;
pub use section::LoftSectionCurve;

const MAX_EDGES: usize = 64;

/// One bilinear wall. Each pair is an authored section edge in its original
/// order. The parameters run along the section edge and along the loft.
#[derive(Clone, Copy, Debug)]
pub struct LoftWall {
    start: [Point; 2],
    end: [Point; 2],
}

impl LoftWall {
    pub fn point(self, u: f64, t: f64) -> Point {
        interpolate(
            interpolate(self.start[0], self.start[1], u),
            interpolate(self.end[0], self.end[1], u),
            t,
        )
    }

    fn vectors(self) -> (Point, Point, Point) {
        let edge = sub(self.start[1], self.start[0]);
        let rail = sub(self.end[0], self.start[0]);
        let twist = sub(sub(self.end[1], self.end[0]), edge);
        (edge, rail, twist)
    }

    /// Area by bounded quadrature of the exact surface's Jacobian.
    pub fn area_mm2(self) -> Result<f64, Error> {
        let (edge, rail, twist) = self.vectors();
        area::integrate(cross(edge, rail), cross(edge, twist), cross(twist, rail))
    }
}

/// A closed solid with planar caps and bilinear sides. Strict convexity
/// throughout the interval excludes crossing walls and zero-area sections.
#[derive(Clone, Debug, PartialEq)]
pub struct NonplanarLoft {
    rings: [Vec<Point>; 2],
    clockwise: bool,
}

impl NonplanarLoft {
    pub fn new(sections: [&[[f64; 2]]; 2], height: f64, tolerance: f64) -> Result<Self, Error> {
        validate_tolerance(tolerance)?;
        let [first, last] = sections;
        if first.len().max(last.len()) > MAX_EDGES {
            return Err(Error::Limit);
        }
        if first.len() < 3
            || first.len() != last.len()
            || !(tolerance..=MAX_COORDINATE).contains(&height)
            || height == tolerance
            || sections
                .into_iter()
                .flatten()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
        {
            return Err(Error::Invalid);
        }
        let rings = [(first, 0.), (last, height)]
            .map(|(ring, z)| ring.iter().map(|&[x, y]| [x, y, z]).collect());
        let loft = Self {
            rings,
            clockwise: cross2(sub2(first[1], first[0]), sub2(first[2], first[0])) < 0.,
        };
        loft.prove_sections(tolerance)?;
        Ok(loft)
    }

    fn prove_sections(&self, tolerance: f64) -> Result<(), Error> {
        let [first, last] = &self.rings;
        let sign = if self.clockwise { -1. } else { 1. };
        let coordinate_error = 64.
            * f64::EPSILON
            * first
                .iter()
                .chain(last)
                .flat_map(|p| p[..2].iter().map(|x| x.abs()))
                .fold(1., f64::max);
        for i in 0..first.len() {
            let j = (i + 1) % first.len();
            let [a, b] = [&first, &last].map(|ring| sub(ring[j], ring[i]));
            for k in (0..first.len()).filter(|&k| k != i && k != j) {
                let [c, d] = [&first, &last].map(|ring| sub(ring[k], ring[i]));
                let [p, q, r] = [
                    cross(a, c)[2],
                    (cross(a, d)[2] + cross(b, c)[2]) * 0.5,
                    cross(b, d)[2],
                ]
                .map(|value| sign * value);
                // The signed edge-to-vertex determinant is quadratic in the
                // station. Test its minimum, with a guard for the subtractions
                // and products at the original coordinate magnitude.
                let edge = norm(a).max(norm(b));
                let offset = norm(c).max(norm(d));
                let guard =
                    tolerance * edge + coordinate_error * (edge + offset + coordinate_error);
                if quadratic_min(p, q, r) <= guard {
                    return Err(Error::Unresolved);
                }
            }
        }
        Ok(())
    }

    pub fn rings(&self) -> &[Vec<Point>; 2] {
        &self.rings
    }

    pub fn clockwise(&self) -> bool {
        self.clockwise
    }

    pub fn bounds(&self) -> [Point; 2] {
        bounds(self.rings.iter().flatten().copied())
    }

    pub fn walls(&self) -> impl ExactSizeIterator<Item = LoftWall> + '_ {
        let [first, last] = &self.rings;
        (0..first.len()).map(|i| {
            let j = (i + 1) % first.len();
            LoftWall {
                start: [first[i], first[j]],
                end: [last[i], last[j]],
            }
        })
    }

    pub fn cap_areas_mm2(&self) -> [f64; 2] {
        self.rings
            .each_ref()
            .map(|ring| section_area(ring.iter().copied()))
    }

    pub fn volume_mm3(&self) -> f64 {
        let [a, b] = self.cap_areas_mm2();
        let middle = section_area(
            self.rings[0]
                .iter()
                .zip(&self.rings[1])
                .map(|(&a, &b)| interpolate(a, b, 0.5)),
        );
        // Section area is quadratic, so Simpson's formula is exact.
        self.rings[1][0][2] * (a + 4. * middle + b) / 6.
    }

    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        let count = self.rings[0].len();
        let divisions = self
            .walls()
            .map(|wall| divisions(wall, options))
            .fold(1., f64::max);
        // Bound before converting and allocating.
        if 2. * count as f64 * divisions * (divisions + 1.) > options.triangle_limit as f64 {
            return Err(MeshError::BudgetExceeded);
        }
        let divisions = divisions as usize;
        let perimeter = count * divisions;
        let mut mesh = DisplayMesh {
            vertices: Vec::with_capacity(3 * (perimeter * (divisions + 1) + 2)),
            indices: Vec::with_capacity(6 * perimeter * (divisions + 1)),
            face_indices: Vec::with_capacity(2 * perimeter * (divisions + 1)),
        };
        let parameter = |index: usize| index as f64 / divisions as f64;
        for row in 0..=divisions {
            for wall in self.walls() {
                for col in 0..divisions {
                    mesh.vertices
                        .extend(wall.point(parameter(col), parameter(row)));
                }
            }
        }
        let triangle = |mesh: &mut DisplayMesh, mut indices: [usize; 3], face| {
            if self.clockwise {
                indices.swap(1, 2);
            }
            mesh.triangle(indices, face);
        };
        for row in 0..divisions {
            for col in 0..perimeter {
                let next = (col + 1) % perimeter;
                let a = row * perimeter + col;
                let b = row * perimeter + next;
                let [d, c] = [a + perimeter, b + perimeter];
                let face = (2 + col / divisions) as u32;
                triangle(&mut mesh, [a, b, c], face);
                triangle(&mut mesh, [a, c, d], face);
            }
        }
        for (cap, ring) in self.rings.iter().enumerate() {
            let center = mesh.vertices.len() / 3;
            let average = scale(
                ring.iter()
                    .fold([0.; 3], |sum, &p| add(sum, sub(p, ring[0]))),
                1. / count as f64,
            );
            mesh.vertices.extend(add(ring[0], average));
            let offset = cap * divisions * perimeter;
            for col in 0..perimeter {
                let a = offset + col;
                let b = offset + (col + 1) % perimeter;
                triangle(
                    &mut mesh,
                    if cap == 0 {
                        [center, b, a]
                    } else {
                        [center, a, b]
                    },
                    cap as u32,
                );
            }
        }
        Ok(mesh)
    }
}

fn sub2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn interpolate(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| (1. - t) * a[i] + t * b[i])
}

fn quadratic_min(p: f64, q: f64, r: f64) -> f64 {
    if q < p.min(r) {
        let span = (p - q) + (r - q);
        let t = (p - q) / span;
        p - t * (p - q)
    } else {
        p.min(r)
    }
}

fn section_area(mut points: impl Iterator<Item = Point>) -> f64 {
    let origin = points.next().expect("certified section");
    let (area, _) = points.fold((0., origin), |(area, previous), current| {
        (
            area + cross(sub(previous, origin), sub(current, origin))[2],
            current,
        )
    });
    area.abs() * 0.5
}

fn divisions(wall: LoftWall, options: Tessellation) -> f64 {
    let (edge, rail, twist) = wall.vectors();
    // Triangles interpolate the bilinear wall with at most |twist|/4 error
    // per unit UV cell. Their normals match the wall at opposite corners.
    let displacement = (norm(twist) / (4. * options.deflection)).sqrt();
    let edge_end = sub(wall.end[1], wall.end[0]);
    let fraction = if dot(twist, twist) > 0. {
        (-dot(edge, twist) / dot(twist, twist)).clamp(0., 1.)
    } else {
        0.
    };
    let min_jacobian = rail[2] * norm(interpolate(edge, edge_end, fraction));
    let normal_change = norm(cross(edge, twist)) + norm(cross(twist, rail));
    let rotation = normal_change / (min_jacobian * (options.angle * 0.5).sin());
    displacement.max(rotation).ceil()
}

#[cfg(test)]
mod tests;
