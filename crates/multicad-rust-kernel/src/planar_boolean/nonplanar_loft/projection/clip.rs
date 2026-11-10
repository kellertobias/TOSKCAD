//! Exact trim curves of an orthographic depth and window intersection.
use super::*;

type Limits = [Option<[f64; 2]>; 3];

pub(super) struct Clip<'a> {
    loft: &'a NonplanarLoft,
    inverse: [Point; 3],
    planes: Limits,
    front: Option<f64>,
}

impl<'a> Clip<'a> {
    fn new(
        loft: &'a NonplanarLoft,
        [u, v, n]: [Point; 3],
        planes: Limits,
        front: Option<f64>,
    ) -> Self {
        let w = cross(u, v);
        let determinant = dot(w, n);
        let inverse =
            [cross(v, n), cross(n, u), w].map(|column| scale(column, determinant.recip()));
        Self {
            loft,
            inverse,
            planes,
            front,
        }
    }

    pub(super) fn near(&self) -> f64 {
        self.planes[2].map_or(f64::INFINITY, |plane| plane[1])
    }

    pub(super) fn includes(&self, point: Point, epsilon: f64) -> bool {
        self.planes.iter().zip(point).all(|(limit, coordinate)| {
            limit.is_none_or(|[lo, hi]| (lo - epsilon..=hi + epsilon).contains(&coordinate))
        })
    }

    pub(super) fn crossings(&self, curve: Curve, cuts: &mut Vec<f64>) {
        for (axis, limits) in self.planes.iter().enumerate() {
            for plane in limits.iter().flatten() {
                cuts.extend(curve.crossings(axis, *plane));
            }
        }
    }

    pub(super) fn blocks(&self, point: Point, tolerance: f64) -> bool {
        self.front.is_some_and(|near| {
            near > point[2] + tolerance && self.inside_loft([point[0], point[1], near], tolerance)
        })
    }

    fn inside_loft(&self, [u, v, z]: Point, tolerance: f64) -> bool {
        let [a, b, c] = self.inverse;
        let p = add(add(scale(a, u), scale(b, v)), scale(c, z));
        let [lower, upper] = self.loft.rings();
        let t = p[2] / upper[0][2];
        if !(0. ..=1.).contains(&t) {
            return false;
        }
        let side = if self.loft.clockwise() { -1. } else { 1. };
        (0..lower.len()).all(|i| {
            let j = (i + 1) % lower.len();
            let a = paper(interpolate(lower[i], upper[i], t));
            let b = paper(interpolate(lower[j], upper[j], t));
            let edge = minus(b, a);
            side * determinant(edge, minus(paper(p), a)) >= -tolerance * length(edge)
        })
    }

    fn connectors(
        &self,
        rim: &[Curve],
        axis: usize,
        value: f64,
        z: f64,
        epsilon: f64,
    ) -> Vec<Curve> {
        let transverse = 1 - axis;
        let mut hits = Vec::new();
        let mut coincident = Vec::new();
        for &curve in rim {
            if curve.on_plane(axis, value, epsilon) {
                let ends = [curve.at(0.)[transverse], curve.at(1.)[transverse]];
                coincident.push([ends[0].min(ends[1]), ends[0].max(ends[1])]);
                hits.extend(ends);
                continue;
            }
            hits.extend(
                curve
                    .crossings(axis, value)
                    .into_iter()
                    .map(|t| curve.at(t)[transverse]),
            );
            hits.extend(
                [curve.at(0.), curve.at(1.)]
                    .into_iter()
                    .filter(|p| (p[axis] - value).abs() <= epsilon)
                    .map(|p| p[transverse]),
            );
        }
        hits.sort_unstable_by(f64::total_cmp);
        hits.dedup_by(|a, b| (*a - *b).abs() <= epsilon);
        let point = |along| {
            let mut point = [value, value, z];
            point[transverse] = along;
            point
        };
        hits.windows(2)
            .filter_map(|span| {
                let mid = span[0].midpoint(span[1]);
                if coincident.iter().any(|&[lo, hi]| (lo..=hi).contains(&mid))
                    || !self.inside_loft(point(mid), epsilon)
                {
                    return None;
                }
                Some(Curve::line(point(span[0]), point(span[1])))
            })
            .collect()
    }
}

pub(super) fn prepare<'a>(
    loft: &'a NonplanarLoft,
    rings: &[Vec<Point>; 2],
    options: LoftProjection,
    planes: Limits,
    epsilon: f64,
) -> Option<(Clip<'a>, Vec<Curve>)> {
    let mut active = [None; 6];
    for (axis, plane) in planes.into_iter().enumerate() {
        if let Some([lo, hi]) = plane {
            let [min, max] = rings
                .iter()
                .flatten()
                .map(|p| p[axis])
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[min, max], x| {
                    [min.min(x), max.max(x)]
                });
            if hi.min(max) - lo.max(min) <= epsilon {
                return None;
            }
            active[2 * axis] = (lo > min + epsilon).then_some(lo);
            active[2 * axis + 1] = (hi < max - epsilon).then_some(hi);
        }
    }
    let clip = Clip::new(loft, options.axes, planes, active[5]);
    let mut edges = Vec::new();
    for (i, plane) in active.into_iter().enumerate() {
        let Some(value) = plane else { continue };
        let axis = i / 2;
        let normal = options.axes[axis];
        let magnitude = norm(normal);
        let project = |p| {
            let mut p = options.axes.map(|axis| dot(axis, p));
            p[axis] = value;
            p
        };
        let rim = loft.section_in_frame(
            scale(normal, value / (magnitude * magnitude)),
            scale(normal, magnitude.recip()),
            options.tolerance,
        );
        let rim = rim.into_iter().map(|curve| match curve {
            LoftSectionCurve::Line(ends) => {
                let [a, b] = ends.map(project);
                Curve::line(a, b)
            }
            LoftSectionCurve::Quadratic { poles, weights } => {
                Curve::new(poles.map(project), weights)
            }
        });
        if axis < 2 {
            edges.extend(rim.flat_map(|curve| curve.monotone_spans(1 - axis)));
        } else {
            let rim: Vec<_> = rim.collect();
            for (i, side) in active[..4].iter().enumerate() {
                if let Some(side) = side {
                    edges.extend(clip.connectors(&rim, i / 2, *side, value, epsilon));
                }
            }
            edges.extend(rim);
        }
    }
    Some((clip, edges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_cap_ray_rejects_points_above_the_authored_stations() {
        let square = [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]];
        let loft = NonplanarLoft::new([&square, &square], 6., 1e-7).unwrap();
        let axes = [[1., 0., 0.], [0., 0.8, 0.6], [0., -0.6, 0.8]];
        let clip = Clip::new(&loft, axes, [None, None, Some([1.2, 3.])], Some(3.));
        assert!(clip.blocks([0., 2., 1.2], 1e-7));
        assert!(!clip.blocks([0., 7., 1.2], 1e-7));
    }

    #[test]
    fn existing_cut_rim_edge_does_not_get_a_duplicate_connector() {
        let square = [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]];
        let top = square.map(|[x, y]| [x + 0.4, y]);
        let loft = NonplanarLoft::new([&square, &top], 6., 1e-7).unwrap();
        let axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        let clip = Clip::new(
            &loft,
            axes,
            [Some([-0.8, 0.5]), None, Some([1., 3.])],
            Some(3.),
        );
        let near = square.map(|[x, y]| [x + 0.2, y, 3.]);
        let rim = std::array::from_fn::<_, 4, _>(|i| Curve::line(near[i], near[(i + 1) % 4]));
        assert!(clip.connectors(&rim, 0, -0.8, 3., 1e-7).is_empty());
        let edges = clip.connectors(&rim, 0, 0.5, 3., 1e-7);
        assert_eq!(edges.len(), 1);
        assert_eq!(
            [edges[0].at(0.), edges[0].at(1.)],
            [[0.5, -1., 3.], [0.5, 1., 3.]]
        );
    }
}
