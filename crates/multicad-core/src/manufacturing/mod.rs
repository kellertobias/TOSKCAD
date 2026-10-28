//! Exact planar manufacturing contours. Coordinates remain model millimeters;
//! units and file serialization belong to output providers, never viewport data.
use serde::{Deserialize, Serialize};
pub const MAX_PLANAR_LOOPS: usize = 256;
pub const MAX_PLANAR_EDGES: usize = 16_384;
pub type Point2 = [f64; 2];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanarEdge {
    Line {
        a: Point2,
        b: Point2,
    },
    Arc {
        a: Point2,
        b: Point2,
        center: Point2,
        sweep_rad: f64,
    },
}
fn distance(a: Point2, b: Point2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn cross(a: Point2, b: Point2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
impl PlanarEdge {
    pub fn ends(&self) -> (Point2, Point2) {
        match *self {
            Self::Line { a, b } | Self::Arc { a, b, .. } => (a, b),
        }
    }
    pub fn reversed(&self) -> Self {
        match *self {
            Self::Line { a, b } => Self::Line { a: b, b: a },
            Self::Arc {
                a,
                b,
                center,
                sweep_rad,
            } => Self::Arc {
                a: b,
                b: a,
                center,
                sweep_rad: -sweep_rad,
            },
        }
    }
    pub fn signed_area(&self) -> f64 {
        match *self {
            Self::Line { a, b } => cross(a, b) * 0.5,
            Self::Arc {
                a,
                b,
                center,
                sweep_rad,
            } => {
                let r = distance(a, center);
                (cross(center, [b[0] - a[0], b[1] - a[1]]) + r * r * sweep_rad) * 0.5
            }
        }
    }
    pub fn validate(&self, tolerance: f64) -> Result<(), String> {
        let (a, b) = self.ends();
        if a.into_iter()
            .chain(b)
            .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err("Planar edge coordinates are outside finite model bounds".into());
        }
        match *self {
            Self::Line { a, b } if distance(a, b) <= tolerance => {
                Err("Planar line has zero or unresolved length".into())
            }
            Self::Arc {
                a,
                b,
                center,
                sweep_rad,
            } => {
                if center.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
                    || !sweep_rad.is_finite()
                    || sweep_rad.abs() < 1e-12
                    || sweep_rad.abs() > std::f64::consts::TAU + 1e-10
                {
                    return Err(
                        "Planar arc requires a finite nonzero sweep of at most one turn".into(),
                    );
                }
                let r = distance(a, center);
                if r <= tolerance || (distance(b, center) - r).abs() > tolerance {
                    return Err("Planar arc radius or endpoints are inconsistent".into());
                }
                let start = (a[1] - center[1]).atan2(a[0] - center[0]);
                let expected = [
                    center[0] + r * (start + sweep_rad).cos(),
                    center[1] + r * (start + sweep_rad).sin(),
                ];
                if distance(expected, b) > tolerance {
                    return Err("Planar arc sweep does not reach its finite endpoint".into());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    pub fn bounds(&self) -> [Point2; 2] {
        let (a, b) = self.ends();
        let mut min = [a[0].min(b[0]), a[1].min(b[1])];
        let mut max = [a[0].max(b[0]), a[1].max(b[1])];
        if let Self::Arc {
            center, sweep_rad, ..
        } = *self
        {
            let r = distance(a, center);
            let start = (a[1] - center[1]).atan2(a[0] - center[0]);
            for (angle, unit) in [
                (0., [1., 0.]),
                (std::f64::consts::FRAC_PI_2, [0., 1.]),
                (std::f64::consts::PI, [-1., 0.]),
                (3. * std::f64::consts::FRAC_PI_2, [0., -1.]),
            ] {
                let travel = if sweep_rad > 0. {
                    (angle - start).rem_euclid(std::f64::consts::TAU)
                } else {
                    (start - angle).rem_euclid(std::f64::consts::TAU)
                };
                if travel <= sweep_rad.abs() + 1e-12 {
                    let p = [center[0] + r * unit[0], center[1] + r * unit[1]];
                    for i in 0..2 {
                        min[i] = min[i].min(p[i]);
                        max[i] = max[i].max(p[i]);
                    }
                }
            }
        }
        [min, max]
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarLoop {
    pub outer: bool,
    pub edges: Vec<PlanarEdge>,
}
impl PlanarLoop {
    pub fn signed_area(&self) -> f64 {
        self.edges.iter().map(PlanarEdge::signed_area).sum()
    }
    pub fn normalize_winding(&mut self) {
        if (self.signed_area() > 0.) != self.outer {
            self.edges = self.edges.iter().rev().map(PlanarEdge::reversed).collect();
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarContours {
    pub frame: crate::references::ReferenceFrame,
    pub loops: Vec<PlanarLoop>,
}
impl PlanarContours {
    /// Producers establish topology/classification using native regions. This
    /// contract additionally checks finite curves, closure, budgets and winding.
    pub fn validate(&self, tolerance: f64) -> Result<(), String> {
        self.frame.validate()?;
        if !tolerance.is_finite() || tolerance <= 0. || tolerance > 0.01 {
            return Err("Invalid planar contour tolerance".into());
        }
        if self.loops.is_empty()
            || self.loops.len() > MAX_PLANAR_LOOPS
            || !self.loops.iter().any(|l| l.outer)
            || self.edge_count() > MAX_PLANAR_EDGES
        {
            return Err("Planar contour loop/edge budget exceeded or no outer loop exists".into());
        }
        if self.area_mm2() <= tolerance * tolerance {
            return Err("Planar material has no positive finite area".into());
        }
        for wire in &self.loops {
            if wire.edges.is_empty() {
                return Err("Planar loops cannot be empty".into());
            }
            for (i, e) in wire.edges.iter().enumerate() {
                e.validate(tolerance)?;
                if distance(e.ends().1, wire.edges[(i + 1) % wire.edges.len()].ends().0) > tolerance
                {
                    return Err("Planar loop has an open or disconnected boundary".into());
                }
            }
            let area = wire.signed_area();
            if !area.is_finite() || area.abs() <= tolerance * tolerance || (area > 0.) != wire.outer
            {
                return Err("Planar loop has degenerate area or invalid outer/hole winding".into());
            }
        }
        Ok(())
    }
    pub fn edge_count(&self) -> usize {
        self.loops.iter().map(|w| w.edges.len()).sum()
    }
    pub fn area_mm2(&self) -> f64 {
        self.loops.iter().map(PlanarLoop::signed_area).sum()
    }
    pub fn bounds_mm(&self) -> Option<[Point2; 2]> {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for edge in self.loops.iter().flat_map(|l| &l.edges) {
            let [a, b] = edge.bounds();
            for i in 0..2 {
                min[i] = min[i].min(a[i]);
                max[i] = max[i].max(b[i]);
            }
        }
        min.into_iter()
            .chain(max)
            .all(f64::is_finite)
            .then_some([min, max])
    }
}

#[cfg(test)]
mod tests;
