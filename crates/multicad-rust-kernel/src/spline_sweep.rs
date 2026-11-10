//! Source spline sweeps reconstructed from a certified analytic boundary.

use crate::{
    display::{DisplayMesh, Footprint, MeshError, Tessellation},
    normal_sweep,
    revolution::{Sum, length, sub},
};

mod cubic;
pub use cubic::Cubic;

type Point = [f64; 2];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Limit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edge {
    Analytic(normal_sweep::Edge),
    Cubic(Cubic),
}

impl Edge {
    pub fn point(&self, fraction: f64) -> Point {
        match self {
            Self::Analytic(edge) => edge.point(fraction),
            Self::Cubic(edge) => edge.point(fraction),
        }
    }

    pub fn support(&self, direction: Point) -> f64 {
        match self {
            Self::Analytic(edge) => edge.support(direction),
            Self::Cubic(edge) => edge.support(direction),
        }
    }

    pub fn length_bounds_mm(&self, error_mm: f64) -> Result<[f64; 2], Error> {
        if !error_mm.is_finite() || error_mm <= 0. {
            return Err(Error::Invalid);
        }
        match self {
            Self::Analytic(edge) => Ok([edge.length_mm(); 2]),
            Self::Cubic(edge) => edge.length_bounds_mm(error_mm),
        }
    }

    /// Detached samples including both source endpoints.
    pub fn sample(&self, options: Tessellation) -> Result<Vec<Point>, MeshError> {
        options.check()?;
        self.sample_with_limit(options, options.triangle_limit)
    }

    fn sample_with_limit(
        &self,
        options: Tessellation,
        limit: usize,
    ) -> Result<Vec<Point>, MeshError> {
        match self {
            Self::Analytic(edge) => {
                let count = match edge.radius_mm().zip(edge.signed_sweep_rad()) {
                    Some((radius, angle)) => options.arc_segments(radius, angle)?.max(2),
                    None => 1,
                };
                if count > limit {
                    return Err(MeshError::BudgetExceeded);
                }
                Ok((0..=count)
                    .map(|i| edge.point(i as f64 / count as f64))
                    .collect())
            }
            Self::Cubic(edge) => edge.sample(options.deflection, options.angle, limit),
        }
    }

    fn moments(&self, origin: Point) -> [f64; 3] {
        match self {
            Self::Analytic(edge) => edge.moments(origin),
            Self::Cubic(edge) => edge.moments(origin),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sweep {
    outline: Vec<Edge>,
    hole: Option<normal_sweep::Edge>,
    height_mm: f64,
    area_mm2: f64,
    centroid_mm: Point,
    join_adjustment_mm: f64,
}

impl Sweep {
    /// The decoder has already proved the source rim simple, correctly
    /// oriented, nonoverlapping and matched to its translated opposite rim.
    pub(crate) fn from_verified(
        outline: Vec<Edge>,
        hole: Option<normal_sweep::Edge>,
        height_mm: f64,
        tolerance: f64,
    ) -> Result<Self, Error> {
        if !(3..=crate::polygon::MAX_VERTICES).contains(&outline.len())
            || !height_mm.is_finite() || height_mm <= tolerance
            || hole.is_some_and(|hole| {
                !matches!(hole, normal_sweep::Edge::Circle { clockwise: true, radius_mm, .. } if radius_mm > tolerance)
            })
        {
            return Err(Error::Invalid);
        }
        let mut join_adjustment_mm: f64 = 0.;
        for (edge, next) in outline.iter().zip(outline.iter().cycle().skip(1)) {
            let [a, b] = [edge.point(1.), next.point(0.)];
            let scale = a.into_iter().chain(b).map(f64::abs).fold(1., f64::max);
            let gap = length(sub(a, b));
            let roundoff = (32. * f64::EPSILON * scale)
                .max(tolerance * 1e-3)
                .min(tolerance);
            if gap > roundoff {
                return Err(Error::Invalid);
            }
            join_adjustment_mm = join_adjustment_mm.max(gap);
        }

        let origin = outline[0].point(0.);
        let mut sum = [Sum::default(); 3];
        for moments in outline
            .iter()
            .map(|edge| edge.moments(origin))
            .chain(hole.map(|hole| hole.moments(origin)))
        {
            for (sum, term) in sum.iter_mut().zip(moments) {
                sum.add(term);
            }
        }
        let [area_mm2, x, y] = sum.map(Sum::value);
        let centroid_mm = add(origin, [x / area_mm2, y / area_mm2]);
        if !area_mm2.is_finite()
            || area_mm2 <= tolerance * tolerance
            || !centroid_mm.into_iter().all(f64::is_finite)
        {
            return Err(Error::Invalid);
        }
        Ok(Self {
            outline,
            hole,
            height_mm,
            area_mm2,
            centroid_mm,
            join_adjustment_mm,
        })
    }

    pub fn outline(&self) -> &[Edge] {
        &self.outline
    }

    pub fn hole(&self) -> Option<normal_sweep::Edge> {
        self.hole
    }

    pub fn height_mm(&self) -> f64 {
        self.height_mm
    }

    pub fn area_mm2(&self) -> f64 {
        self.area_mm2
    }

    pub fn centroid_mm(&self) -> [f64; 3] {
        [
            self.centroid_mm[0],
            self.centroid_mm[1],
            self.height_mm / 2.,
        ]
    }

    pub fn volume_mm3(&self) -> f64 {
        self.area_mm2 * self.height_mm
    }

    pub fn surface_area_bounds_mm2(&self, error_mm2: f64) -> Result<[f64; 2], Error> {
        if !error_mm2.is_finite() || error_mm2 <= 0. {
            return Err(Error::Invalid);
        }
        let allowance = error_mm2 / self.height_mm / self.outline.len() as f64;
        let analytic_area = 2. * self.area_mm2
            + self.hole.map_or(0., normal_sweep::Edge::length_mm) * self.height_mm;
        if allowance <= 0. {
            return Err(Error::Invalid);
        }
        let mut budget = cubic::MAX_LENGTH_SPLITS;
        let mut sum = [Sum::default(); 2];
        for edge in &self.outline {
            let bounds = match edge {
                Edge::Analytic(edge) => [edge.length_mm(); 2],
                Edge::Cubic(edge) => edge.length_bounds_with_budget(allowance, &mut budget)?,
            };
            for (sum, term) in sum.iter_mut().zip(bounds) {
                sum.add(term);
            }
        }
        Ok(sum.map(|length| self.height_mm.mul_add(length.value(), analytic_area)))
    }

    pub fn support(&self, direction: [f64; 3]) -> f64 {
        if !direction.into_iter().all(f64::is_finite) {
            return f64::NAN;
        }
        let magnitude = direction.into_iter().map(f64::abs).fold(0., f64::max);
        if magnitude == 0. {
            return 0.;
        }
        let [x, y, z] = direction.map(|value| value / magnitude);
        let support = self
            .outline
            .iter()
            .map(|edge| edge.support([x, y]))
            .fold(f64::NEG_INFINITY, f64::max);
        (support + (z * self.height_mm).max(0.)) * magnitude
    }

    pub fn bounds(&self) -> [[f64; 3]; 2] {
        let axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        [
            axes.map(|axis| -self.support(axis.map(|v| -v))),
            axes.map(|axis| self.support(axis)),
        ]
    }

    /// The cap IDs are 0 and 1, followed by authored outline order and hole.
    pub fn mesh(&self, mut options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        options.deflection -= self.join_adjustment_mm;
        if options.deflection <= 0. {
            return Err(MeshError::UnresolvedBoundary);
        }
        let mut samples = Vec::new();
        let limit = (options.triangle_limit + 4) / 4;
        for (i, edge) in self.outline.iter().enumerate() {
            let mut points = edge.sample_with_limit(options, limit - samples.len())?;
            // The next edge supplies the canonical shared vertex.
            points.pop();
            samples.extend(points.into_iter().map(|point| (point, (i + 2) as u32)));
        }
        let mut footprint = Footprint::default();
        footprint.ring(samples.into_iter(), options)?;
        if let Some(normal_sweep::Edge::Circle {
            center, radius_mm, ..
        }) = self.hole
        {
            footprint.circle(
                center,
                radius_mm,
                (self.outline.len() + 2) as u32,
                true,
                options,
            )?;
        }
        footprint.tessellate(0., self.height_mm)
    }
}

fn add([x, y]: Point, [u, v]: Point) -> Point {
    [x + u, y + v]
}

fn scale([x, y]: Point, factor: f64) -> Point {
    [x * factor, y * factor]
}

#[cfg(test)]
mod tests;
