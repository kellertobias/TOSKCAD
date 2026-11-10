//! Rational quadratics shared by wall silhouettes and oblique cut rims.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Curve {
    poles: [Point; 3],
    weights: [f64; 3],
    linear: bool,
}

impl Curve {
    pub(super) fn new(poles: [Point; 3], weights: [f64; 3]) -> Self {
        Self {
            poles,
            weights,
            linear: false,
        }
    }

    pub(super) fn line(a: Point, b: Point) -> Self {
        Self {
            poles: [a, interpolate(a, b, 0.5), b],
            weights: [1.; 3],
            linear: true,
        }
    }

    pub(super) fn through_midpoint(a: Point, m: Point, b: Point) -> Self {
        Self::new([a, add(m, sub(m, interpolate(a, b, 0.5))), b], [1.; 3])
    }

    pub(super) fn at(self, t: f64) -> Point {
        let [a, b, c] = self.poles;
        if self.linear {
            return interpolate(a, c, t);
        }
        let s = 1. - t;
        let [wa, wb, wc] = self.weights;
        let [p, q, r] = [s * s * wa, 2. * s * t * wb, t * t * wc];
        add(
            a,
            scale(
                add(scale(sub(b, a), q), scale(sub(c, a), r)),
                (p + q + r).recip(),
            ),
        )
    }

    fn split(self, t: f64) -> [Self; 2] {
        let blend = |(p, a), (q, b)| {
            let w = (1. - t) * a + t * b;
            (interpolate(p, q, t * b / w), w)
        };
        let [a, b, c] = std::array::from_fn(|i| (self.poles[i], self.weights[i]));
        let p = blend(a, b);
        let q = blend(b, c);
        let m = blend(p, q);
        [[a, p, m], [m, q, c]].map(|poles| Self::new(poles.map(|p| p.0), poles.map(|p| p.1)))
    }

    pub(super) fn slice(mut self, [a, b]: [f64; 2]) -> Self {
        if self.linear {
            return Self::line(self.at(a), self.at(b));
        }
        if b < 1. {
            self = self.split(b)[0];
        }
        if a > 0. {
            self = self.split(a / b)[1];
        }
        self
    }

    pub(super) fn publish(
        self,
        hidden: bool,
        epsilon: f64,
        tolerance: f64,
    ) -> Option<LoftDrawingEdge> {
        let [a, m, b] = self.poles;
        let chord = minus(paper(b), paper(a));
        let span = length(chord);
        if span <= tolerance {
            return None;
        }
        let curve = if determinant(chord, minus(paper(m), paper(a))).abs() <= epsilon * span {
            LoftSectionCurve::Line([a, b])
        } else {
            LoftSectionCurve::Quadratic {
                poles: self.poles,
                weights: self.weights,
            }
        };
        Some(LoftDrawingEdge { curve, hidden })
    }

    pub(super) fn bounds(self) -> [Paper; 2] {
        let points = self.poles.map(paper);
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

    fn power(values: [f64; 3]) -> [f64; 3] {
        let [a, b, c] = values;
        [a, 2. * (b - a), (a - b) + (c - b)]
    }

    fn coefficients(self, f: impl Fn(Point) -> f64) -> [f64; 3] {
        if self.linear {
            let start = f(self.poles[0]);
            return [start, f(self.poles[2]) - start, 0.];
        }
        Self::power(std::array::from_fn(|i| f(self.poles[i]) * self.weights[i]))
    }

    fn homogeneous(self, origin: Paper) -> [Point; 3] {
        let x = self.coefficients(|p| p[0] - origin[0]);
        let y = self.coefficients(|p| p[1] - origin[1]);
        let w = Self::power(self.weights);
        std::array::from_fn(|i| [x[i], y[i], w[i]])
    }

    pub(super) fn crossings(self, axis: usize, coordinate: f64) -> Vec<f64> {
        roots(&self.coefficients(|p| p[axis] - coordinate), [0., 1.])
    }

    pub(super) fn on_plane(self, axis: usize, coordinate: f64, tolerance: f64) -> bool {
        self.poles
            .iter()
            .all(|p| (p[axis] - coordinate).abs() <= tolerance)
    }

    /// An edge on a side face is collinear on paper, but its projection can
    /// reverse direction. Split its exact depth parameter at those reversals.
    pub(super) fn monotone_spans(self, axis: usize) -> Vec<Self> {
        let [a, b, c] = self.coefficients(|p| p[axis] - self.poles[0][axis]);
        let [u, v, w] = Self::power(self.weights);
        let mut cuts = vec![0.];
        cuts.extend(roots(
            &[b * u - a * v, 2. * (c * u - a * w), c * v - b * w],
            [0., 1.],
        ));
        cuts.push(1.);
        cuts.windows(2)
            .map(|span| self.slice([span[0], span[1]]))
            .collect()
    }

    pub(super) fn detail_crossings(self, center: Paper, radius: f64, cuts: &mut Vec<f64>) {
        let h = self.homogeneous(center);
        let x = h.map(|p| p[0]);
        let y = h.map(|p| p[1]);
        let w = h.map(|p| p[2] * radius);
        let polynomial = difference_of_products(x, x, w, w);
        let y2 = multiply(y, y);
        let radial = std::array::from_fn::<_, 5, _>(|i| polynomial[i] + y2[i]);
        cuts.extend(roots(&radial, [0., 1.]));
    }

    pub(super) fn intersections(self, other: Self, epsilon: f64, cuts: &mut Vec<f64>) {
        let [a, m, b] = other.poles.map(paper);
        let chord = minus(b, a);
        let h = self.homogeneous(a);
        if determinant(chord, minus(m, a)).abs() <= epsilon * length(chord) {
            cuts.extend(roots(&h.map(|p| determinant(chord, paper(p))), [0., 1.]));
        } else {
            // In the basis of the second curve's homogeneous power vectors,
            // its point has coordinates proportional to [1, t, t²].
            let [a, b, c] = other.homogeneous(a);
            let [p, q, r] = [cross(b, c), cross(c, a), cross(a, b)].map(|v| h.map(|p| dot(v, p)));
            cuts.extend(roots(&difference_of_products(p, r, q, q), [0., 1.]));
        }
        // Split at shared and coincident projected endpoints as well.
        let [u, v] = self
            .poles
            .map(paper)
            .each_ref()
            .iter()
            .skip(1)
            .fold([0_f64; 2], |span, p| {
                std::array::from_fn(|i| span[i].max((p[i] - self.poles[0][i]).abs()))
            });
        let axis = usize::from(v > u);
        for point in [a, b] {
            for t in roots(&self.coefficients(|p| p[axis] - point[axis]), [0., 1.]) {
                if length(minus(paper(self.at(t)), point)) <= epsilon {
                    cuts.push(t);
                }
            }
        }
    }
}

fn multiply(a: [f64; 3], b: [f64; 3]) -> [f64; 5] {
    [
        a[0] * b[0],
        a[0] * b[1] + a[1] * b[0],
        a[0] * b[2] + a[1] * b[1] + a[2] * b[0],
        a[1] * b[2] + a[2] * b[1],
        a[2] * b[2],
    ]
}

fn difference_of_products(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> [f64; 5] {
    let p = multiply(a, b);
    let q = multiply(c, d);
    std::array::from_fn(|i| p[i] - q[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_rim_with_coincident_projected_ends_splits_into_two_exact_spans() {
        let edge = Curve::new([[1., 0., 0.], [1., 4., 1.], [1., 0., 2.]], [1., 0.5, 1.]);
        let spans = edge.monotone_spans(1);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].at(0.), [1., 0., 0.]);
        let middle = [1., 4. / 3., 1.];
        assert!(norm(sub(spans[0].at(1.), middle)) < 1e-12);
        assert!(norm(sub(spans[1].at(0.), middle)) < 1e-12);
        assert_eq!(spans[1].at(1.), [1., 0., 2.]);
        assert!(
            spans
                .into_iter()
                .all(|span| span.publish(false, 1e-9, 1e-7).is_some())
        );
    }
}
