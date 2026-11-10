//! Plane intersections of the authored caps and bilinear walls.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoftSectionCurve {
    Line([Point; 2]),
    /// A rational quadratic Bézier arc with strictly positive weights.
    Quadratic {
        poles: [Point; 3],
        weights: [f64; 3],
    },
}

impl NonplanarLoft {
    /// Intersects the exact boundary of the section face without building a mesh.
    /// Isolated support tangencies with no area are omitted. Points use the
    /// loft's local coordinates and the plane normal must be a unit vector.
    pub fn section(
        &self,
        origin: Point,
        normal: Point,
        tolerance: f64,
    ) -> Result<Vec<LoftSectionCurve>, Error> {
        validate_tolerance(tolerance)?;
        if origin
            .into_iter()
            .any(|v| !v.is_finite() || v.abs() > 4. * MAX_COORDINATE)
            || normal.into_iter().any(|v| !v.is_finite())
            || (norm(normal) - 1.).abs() > 1e-12
        {
            return Err(Error::Invalid);
        }
        let epsilon = (32.
            * f64::EPSILON
            * origin
                .into_iter()
                .chain(self.rings.iter().flatten().copied().flatten())
                .map(f64::abs)
                .fold(1., f64::max))
        .min(tolerance);
        let snap = |v: f64| if v.abs() <= epsilon { 0. } else { v };
        let distances = self.rings.each_ref().map(|ring| {
            ring.iter()
                .map(|&point| snap(dot(normal, sub(point, origin))))
                .collect::<Vec<_>>()
        });
        let [minimum, maximum] = distances.iter().flatten().fold(
            [f64::INFINITY, f64::NEG_INFINITY],
            |[minimum, maximum], &distance| [minimum.min(distance), maximum.max(distance)],
        );
        if minimum > 0. || maximum < 0. {
            return Ok(Vec::new());
        }
        let separates = minimum < 0. && maximum > 0.;
        let mut curves = Vec::new();
        for (ring, distances) in self.rings.iter().zip(&distances) {
            if separates || distances.iter().all(|&d| d == 0.) {
                cap(ring, distances, epsilon, &mut curves);
            }
        }
        let [lower, upper] = &distances;
        for (i, wall) in self.walls().enumerate() {
            let j = (i + 1) % lower.len();
            let rails = [[lower[i], upper[i]], [lower[j], upper[j]]];
            if separates || rails == [[0.; 2]; 2] {
                wall.section(rails, epsilon, &mut curves);
            }
        }
        Ok(curves)
    }
}

fn opposite(a: f64, b: f64) -> bool {
    a != 0. && b != 0. && a.is_sign_negative() != b.is_sign_negative()
}

fn edges(len: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..len).map(move |i| (i, (i + 1) % len))
}

fn cap(ring: &[Point], distances: &[f64], epsilon: f64, curves: &mut Vec<LoftSectionCurve>) {
    if distances.iter().all(|&d| d == 0.) {
        for (i, j) in edges(ring.len()) {
            line([ring[i], ring[j]], epsilon, curves);
        }
        return;
    }
    let mut hits = edges(ring.len()).filter_map(|(i, j)| {
        let [a, b] = [distances[i], distances[j]];
        if a == 0. {
            Some(ring[i])
        } else if opposite(a, b) {
            Some(interpolate(ring[i], ring[j], a / (a - b)))
        } else {
            None
        }
    });
    if let (Some(a), Some(b)) = (hits.next(), hits.next()) {
        line([a, b], epsilon, curves);
    }
}

fn line(ends: [Point; 2], epsilon: f64, curves: &mut Vec<LoftSectionCurve>) {
    let [a, b] = ends;
    let same = |x, y| norm(sub(x, y)) <= epsilon;
    if !same(a, b)
        && !curves.iter().any(|curve| match *curve {
            LoftSectionCurve::Line([c, d]) => {
                (same(a, c) && same(b, d)) || (same(a, d) && same(b, c))
            }
            LoftSectionCurve::Quadratic { .. } => false,
        })
    {
        curves.push(LoftSectionCurve::Line(ends));
    }
}

#[derive(Clone, Copy)]
struct Station {
    t: f64,
    distances: [f64; 2],
}

impl LoftWall {
    fn section_point(self, u: f64, t: f64) -> Point {
        // Keep shared cap and rail points bit-identical for the hatch graph.
        let edge_point = |edge: [Point; 2]| match u {
            0. => edge[0],
            1. => edge[1],
            _ => interpolate(edge[0], edge[1], u),
        };
        match t {
            0. => edge_point(self.start),
            1. => edge_point(self.end),
            _ => interpolate(edge_point(self.start), edge_point(self.end), t),
        }
    }

    fn section(self, rails: [[f64; 2]; 2], epsilon: f64, curves: &mut Vec<LoftSectionCurve>) {
        let mut interior = [None; 2];
        for (i, [a, b]) in rails.into_iter().enumerate() {
            if a == 0. && b == 0. {
                line([self.start[i], self.end[i]], epsilon, curves);
            } else if opposite(a, b) {
                let t = a / (a - b);
                let [c, d] = rails[1 - i];
                let distance = (d - c).mul_add(t, c);
                let mut distances = [0.; 2];
                if distance.abs() > epsilon {
                    distances[1 - i] = distance;
                }
                interior[i] = Some(Station { t, distances });
            }
        }
        if rails == [[0.; 2]; 2] {
            line(self.start, epsilon, curves);
            line(self.end, epsilon, curves);
            return;
        }
        let start = Station {
            t: 0.,
            distances: [rails[0][0], rails[1][0]],
        };
        let end = Station {
            t: 1.,
            distances: [rails[0][1], rails[1][1]],
        };
        let mut stations = [start; 4];
        let mut count = 1;
        for station in interior.into_iter().flatten() {
            // Both rails hit the plane at this station. One ruling suffices.
            if count == 2 && station.distances == [0.; 2] && stations[1].distances == [0.; 2] {
                continue;
            }
            stations[count] = station;
            count += 1;
        }
        stations[count] = end;
        let stations = &mut stations[..=count];
        stations.sort_by(|a, b| a.t.total_cmp(&b.t));
        for station in stations.iter().filter(|s| s.distances == [0.; 2]) {
            line(
                [
                    self.section_point(0., station.t),
                    self.section_point(1., station.t),
                ],
                epsilon,
                curves,
            );
        }
        for pair in stations.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            let mid = std::array::from_fn::<_, 2, _>(|i| a.distances[i] + b.distances[i]);
            if opposite(mid[0], mid[1]) {
                self.arc(a, b, epsilon, curves);
            }
        }
    }

    fn arc(self, a: Station, b: Station, epsilon: f64, curves: &mut Vec<LoftSectionCurve>) {
        // Positive homogeneous rail coefficients at the two end stations.
        let [p, q] = [a, b].map(|s| s.distances.map(f64::abs));
        let [wa, wb] = [p[0] + p[1], q[0] + q[1]];
        if wa == 0. || wb == 0. {
            // A common zero factors out of both affine distances, leaving
            // a constant edge parameter. The ruling itself was added above.
            let u = (p[0] + q[0]) / (wa + wb);
            line(
                [self.section_point(u, a.t), self.section_point(u, b.t)],
                epsilon,
                curves,
            );
            return;
        }
        let start = self.section_point(p[0] / wa, a.t);
        let end = self.section_point(q[0] / wb, b.t);
        let cross_start = self.section_point(q[0] / wb, a.t);
        let cross_end = self.section_point(p[0] / wa, b.t);
        let middle = interpolate(cross_start, cross_end, wa / (wa + wb));
        let chord = sub(end, start);
        let length = norm(chord);
        let along = dot(sub(middle, start), chord);
        if along >= 0.
            && along <= length * length
            && norm(cross(sub(middle, start), chord)) <= epsilon * length
        {
            line([start, end], epsilon, curves);
            return;
        }
        let factor = wa.max(wb);
        curves.push(LoftSectionCurve::Quadratic {
            poles: [start, middle, end],
            weights: [wa / factor, (wa + wb) / (2. * factor), wb / factor],
        });
    }
}

#[cfg(test)]
mod tests;
