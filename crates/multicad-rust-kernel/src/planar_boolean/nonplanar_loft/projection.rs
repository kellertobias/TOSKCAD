//! Orthographic visibility on the exact bilinear walls of an isolated loft.
use super::*;
use crate::curved_projection::roots;

mod visibility;

#[derive(Clone, Copy, Debug)]
pub struct LoftProjection {
    /// Local-space vectors for paper horizontal, vertical and viewer depth.
    pub axes: [Point; 3],
    /// Restrict the loft to this fraction of its station interval.
    pub stations: [f64; 2],
    pub tolerance: f64,
    pub hidden_lines: bool,
    /// Optional circular detail region in paper coordinates.
    pub detail: Option<(Paper, f64)>,
    pub edge_limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoftDrawingEdge {
    /// Paper coordinates and depth in the supplied frame.
    pub curve: LoftSectionCurve,
    pub hidden: bool,
}

type Paper = [f64; 2];

fn paper([u, v, _]: Point) -> Paper {
    [u, v]
}

fn minus(a: Paper, b: Paper) -> Paper {
    [a[0] - b[0], a[1] - b[1]]
}

fn determinant(a: Paper, b: Paper) -> f64 {
    a[0].mul_add(b[1], -a[1] * b[0])
}

fn length([x, y]: Paper) -> f64 {
    x.hypot(y)
}

#[derive(Clone, Copy)]
struct Curve {
    // Coefficients in ascending order, relative to the start.
    power: [Point; 3],
}

impl Curve {
    fn line(a: Point, b: Point) -> Self {
        Self {
            power: [a, sub(b, a), [0.; 3]],
        }
    }

    fn at(self, t: f64) -> Point {
        let [a, b, c] = self.power;
        std::array::from_fn(|i| c[i].mul_add(t, b[i]).mul_add(t, a[i]))
    }

    fn slice(self, [a, b]: [f64; 2]) -> Self {
        let span = b - a;
        Self {
            power: [
                self.at(a),
                scale(add(self.power[1], scale(self.power[2], 2. * a)), span),
                scale(self.power[2], span * span),
            ],
        }
    }

    fn controls(self) -> [Point; 3] {
        [
            self.power[0],
            add(self.power[0], scale(self.power[1], 0.5)),
            self.at(1.),
        ]
    }

    fn publish(self, hidden: bool, epsilon: f64) -> Option<LoftDrawingEdge> {
        let [a, m, b] = self.controls();
        let chord = minus(paper(b), paper(a));
        let span = length(chord);
        if span <= epsilon {
            return None;
        }
        let curve = if determinant(chord, minus(paper(m), paper(a))).abs() <= epsilon * span {
            LoftSectionCurve::Line([a, b])
        } else {
            LoftSectionCurve::Quadratic {
                poles: [a, m, b],
                weights: [1.; 3],
            }
        };
        Some(LoftDrawingEdge { curve, hidden })
    }

    fn bounds(self) -> [Paper; 2] {
        let points = self.controls().map(paper);
        [false, true].map(|upper| {
            std::array::from_fn(|axis| {
                points
                    .iter()
                    .map(|p| p[axis])
                    .reduce(if upper { f64::max } else { f64::min })
                    .unwrap()
            })
        })
    }

    fn detail_crossings(self, center: Paper, radius: f64, cuts: &mut Vec<f64>) {
        let [p, q, r] = self.power.map(paper);
        let p = minus(p, center);
        let dot2 = |a: Paper, b: Paper| a[0].mul_add(b[0], a[1] * b[1]);
        cuts.extend(roots(
            &[
                dot2(p, p) - radius * radius,
                2. * dot2(p, q),
                dot2(q, q) + 2. * dot2(p, r),
                2. * dot2(q, r),
                dot2(r, r),
            ],
            [0., 1.],
        ));
    }

    fn intersections(self, other: Self, epsilon: f64, cuts: &mut Vec<f64>) {
        let [a, b, c] = other.power.map(paper);
        let [p, q, r] = self.power.map(paper);
        let delta = minus(p, a);
        let det = determinant(b, c);
        if det.abs() <= epsilon * length(b).max(length(c)) {
            let chord = minus(paper(other.at(1.)), a);
            // In a coincident projection, finite edge endpoints still split
            // the interval even though the implicit equation vanishes.
            let polynomial = [delta, q, r].map(|v| determinant(chord, v));
            cuts.extend(roots(&polynomial, [0., 1.]));
        } else {
            // Cross products recover the parameter and its square on the
            // second curve. Substitute the first curve into that equation.
            let [x, y, z] = [delta, q, r].map(|v| determinant(v, c));
            let [j, k, l] = [delta, q, r].map(|v| determinant(b, v));
            cuts.extend(roots(
                &[
                    j * det - x * x,
                    k * det - 2. * x * y,
                    l * det - 2. * x * z - y * y,
                    -2. * y * z,
                    -z * z,
                ],
                [0., 1.],
            ));
        }
        // Split at projected endpoints as well. This covers overlaps,
        // tangencies at shared vertices and endpoints of trimmed silhouettes.
        let axis = usize::from(q[1].abs().max(r[1].abs()) > q[0].abs().max(r[0].abs()));
        for point in [a, paper(other.at(1.))] {
            for t in roots(&[p[axis] - point[axis], q[axis], r[axis]], [0., 1.]) {
                if length(minus(paper(self.at(t)), point)) <= epsilon {
                    cuts.push(t);
                }
            }
        }
    }
}

fn overlap(a: [Paper; 2], b: [Paper; 2], tolerance: f64) -> bool {
    (0..2).all(|i| a[0][i] <= b[1][i] + tolerance && b[0][i] <= a[1][i] + tolerance)
}

impl NonplanarLoft {
    pub fn drawing_edges(&self, options: LoftProjection) -> Result<Vec<LoftDrawingEdge>, Error> {
        validate_tolerance(options.tolerance)?;
        let [start, end] = options.stations;
        if !(0. ..end).contains(&start)
            || !(0. ..=1.).contains(&end)
            || options.detail.is_some_and(|(center, radius)| {
                center.into_iter().any(|p| !p.is_finite()) || !radius.is_finite() || radius <= 0.
            })
            || options.axes.iter().flatten().any(|x| !x.is_finite())
            || options.axes.iter().enumerate().any(|(i, a)| {
                options
                    .axes
                    .iter()
                    .enumerate()
                    .any(|(j, b)| (dot(*a, *b) - f64::from(i == j)).abs() > 1e-8)
            })
        {
            return Err(Error::Invalid);
        }
        let [lower, upper] = self.rings();
        let rings = [start, end].map(|t| {
            lower
                .iter()
                .zip(upper)
                .map(|(&a, &b)| {
                    let point = interpolate(a, b, t);
                    options.axes.map(|axis| dot(axis, point))
                })
                .collect::<Vec<_>>()
        });
        let epsilon = (64.
            * f64::EPSILON
            * rings
                .iter()
                .flatten()
                .flatten()
                .map(|x| x.abs())
                .fold(1., f64::max))
        .min(options.tolerance);
        visibility::project(rings, options, epsilon)
    }
}

#[cfg(test)]
mod tests;
