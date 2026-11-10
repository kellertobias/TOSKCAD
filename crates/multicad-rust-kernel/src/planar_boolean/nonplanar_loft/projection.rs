//! Orthographic visibility on the exact bilinear walls of an isolated loft.
use super::*;
use crate::curved_projection::roots;

mod curve;
mod visibility;
use curve::Curve;

#[derive(Clone, Copy, Debug)]
pub struct LoftProjection {
    /// Local-space vectors for paper horizontal, vertical and viewer depth.
    pub axes: [Point; 3],
    /// Restrict the loft to this fraction of its station interval.
    pub stations: [f64; 2],
    /// Optional far and near planes in paper-depth coordinates.
    pub depth: Option<[f64; 2]>,
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

fn overlap(a: [Paper; 2], b: [Paper; 2], tolerance: f64) -> bool {
    (0..2).all(|i| a[0][i] <= b[1][i] + tolerance && b[0][i] <= a[1][i] + tolerance)
}

impl NonplanarLoft {
    pub fn drawing_edges(&self, options: LoftProjection) -> Result<Vec<LoftDrawingEdge>, Error> {
        validate_tolerance(options.tolerance)?;
        let [start, end] = options.stations;
        if !(0. ..end).contains(&start)
            || !(0. ..=1.).contains(&end)
            || options.depth.is_some_and(|[far, near]| {
                options.stations != [0., 1.] || !far.is_finite() || !near.is_finite() || far >= near
            })
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
        let mut cut_edges = Vec::new();
        let depth = if let Some([far, near]) = options.depth {
            let [min, max] = rings
                .iter()
                .flatten()
                .map(|p| p[2])
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[min, max], z| {
                    [min.min(z), max.max(z)]
                });
            if near.min(max) - far.max(min) <= epsilon {
                return Ok(Vec::new());
            }
            let normal = options.axes[2];
            let magnitude = norm(normal);
            for z in [far, near]
                .into_iter()
                .filter(|&z| z > min + epsilon && z < max - epsilon)
            {
                let curves = self.section_in_frame(
                    scale(normal, z / (magnitude * magnitude)),
                    scale(normal, magnitude.recip()),
                    options.tolerance,
                );
                for curve in curves {
                    let to_plane = |p| [dot(options.axes[0], p), dot(options.axes[1], p), z];
                    cut_edges.push(match curve {
                        LoftSectionCurve::Line(ends) => {
                            let [a, b] = ends.map(to_plane);
                            Curve::line(a, b)
                        }
                        LoftSectionCurve::Quadratic { poles, weights } => {
                            Curve::new(poles.map(to_plane), weights)
                        }
                    });
                }
            }
            Some(visibility::Depth::new(
                self,
                options.axes,
                [far, near],
                near < max - epsilon,
            ))
        } else {
            None
        };
        visibility::project(rings, options, epsilon, depth, cut_edges)
    }
}

#[cfg(test)]
mod tests;
