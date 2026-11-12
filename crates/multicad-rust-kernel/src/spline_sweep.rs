//! Source spline sweeps reconstructed from a certified analytic boundary.

use crate::{
    display::{DisplayMesh, Footprint, MeshError, Tessellation},
    normal_sweep,
    revolution::{Sum, length, sub},
};

mod cubic;
pub(crate) mod validation;
pub use cubic::Cubic;

type Point = [f64; 2];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Limit,
}
/// Admission and cooperative control of an authored exact spline section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstructionError {
    Invalid,
    Unresolved,
    Limit,
    Cancelled,
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
    holes: Vec<Vec<Edge>>,
    height_mm: f64,
    area_mm2: f64,
    centroid_mm: Point,
    join_adjustment_mm: f64,
}

impl Sweep {
    pub fn new(
        outline: Vec<Edge>,
        hole: Option<normal_sweep::Edge>,
        height_mm: f64,
        tolerance: f64,
    ) -> Result<Self, ConstructionError> {
        Self::new_with_interrupt(outline, hole, height_mm, tolerance, None)
    }
    pub fn new_with_interrupt(
        outline: Vec<Edge>,
        hole: Option<normal_sweep::Edge>,
        height_mm: f64,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, ConstructionError> {
        Self::new_rings_with_interrupt(
            outline,
            hole.into_iter()
                .map(|edge| vec![Edge::Analytic(edge)])
                .collect(),
            height_mm,
            tolerance,
            interrupt,
        )
    }
    /// Construct one exterior and disjoint, strictly contained clockwise holes.
    pub fn new_rings(
        outline: Vec<Edge>,
        holes: Vec<Vec<Edge>>,
        height_mm: f64,
        tolerance: f64,
    ) -> Result<Self, ConstructionError> {
        Self::new_rings_with_interrupt(outline, holes, height_mm, tolerance, None)
    }
    pub fn new_rings_with_interrupt(
        outline: Vec<Edge>,
        holes: Vec<Vec<Edge>>,
        height_mm: f64,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, ConstructionError> {
        if !tolerance.is_finite() || tolerance <= 0. {
            return Err(ConstructionError::Invalid);
        }
        if interrupt.is_some_and(|interrupt| interrupt()) {
            return Err(ConstructionError::Cancelled);
        }
        if holes.len() > crate::polygon::MAX_VERTICES
            || outline
                .len()
                .saturating_add(holes.iter().map(Vec::len).sum::<usize>())
                > crate::polygon::MAX_VERTICES
            || height_mm > 1e8
        {
            return Err(ConstructionError::Limit);
        }
        let mut spans = 0_usize;
        for edge in outline.iter().chain(holes.iter().flatten()) {
            if interrupt.is_some_and(|interrupt| interrupt()) {
                return Err(ConstructionError::Cancelled);
            }
            spans += match edge {
                Edge::Cubic(curve) => curve.knots().len() - 1,
                Edge::Analytic(_) => 1,
            };
            if spans > crate::polygon::MAX_VERTICES {
                return Err(ConstructionError::Limit);
            }
        }

        // This fixed-size metric pass has no resource failure; the controlled
        // section certificate below owns the complete construction budget.
        let legacy = match holes.as_slice() {
            [] => Some(None),
            [ring] => match ring.as_slice() {
                [Edge::Analytic(edge @ normal_sweep::Edge::Circle { .. })] => Some(Some(*edge)),
                _ => None,
            },
            _ => None,
        };
        let result = match legacy {
            Some(hole) => Self::from_verified(outline, hole, height_mm, tolerance),
            None => Self::from_verified_rings(outline, holes, height_mm, tolerance),
        }
        .map_err(|_| ConstructionError::Invalid)?;
        validation::authored_rings(&result.outline, &result.holes, tolerance, interrupt)?;
        Ok(result)
    }
    /// The decoder has already proved the source rim simple, correctly
    /// oriented, nonoverlapping and matched to its translated opposite rim.
    pub(crate) fn from_verified(
        outline: Vec<Edge>,
        hole: Option<normal_sweep::Edge>,
        height_mm: f64,
        tolerance: f64,
    ) -> Result<Self, Error> {
        Self::from_verified_rings(
            outline,
            hole.into_iter()
                .map(|edge| vec![Edge::Analytic(edge)])
                .collect(),
            height_mm,
            tolerance,
        )
    }
    pub(crate) fn from_verified_rings(
        outline: Vec<Edge>,
        holes: Vec<Vec<Edge>>,
        height_mm: f64,
        tolerance: f64,
    ) -> Result<Self, Error> {
        if !(3..=crate::polygon::MAX_VERTICES).contains(&outline.len())
            || !height_mm.is_finite() || height_mm <= tolerance
            || holes.iter().any(|ring| ring.is_empty() || (ring.len() < 3 && !matches!(ring.as_slice(), [Edge::Analytic(normal_sweep::Edge::Circle { clockwise:true, radius_mm, .. })] if *radius_mm > tolerance)))
        {
            return Err(Error::Invalid);
        }
        let mut join_adjustment_mm: f64 = 0.;
        for ring in std::iter::once(&outline).chain(&holes) {
            for (edge, next) in ring.iter().zip(ring.iter().cycle().skip(1)) {
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
        }
        let origin = outline[0].point(0.);
        let mut sum = [Sum::default(); 3];
        for (index, ring) in std::iter::once(&outline).chain(&holes).enumerate() {
            let mut signed = Sum::default();
            for edge in ring {
                let moments = edge.moments(origin);
                signed.add(moments[0]);
                for (sum, term) in sum.iter_mut().zip(moments) {
                    sum.add(term);
                }
            }
            let area = signed.value();
            if !area.is_finite()
                || if index == 0 {
                    area <= tolerance * tolerance
                } else {
                    area >= -tolerance * tolerance
                }
            {
                return Err(Error::Invalid);
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
            holes,
            height_mm,
            area_mm2,
            centroid_mm,
            join_adjustment_mm,
        })
    }

    pub fn outline(&self) -> &[Edge] {
        &self.outline
    }

    /// Legacy singleton circular-bore view; use `holes()` for every actual loop.
    pub fn hole(&self) -> Option<normal_sweep::Edge> {
        match self.holes.as_slice() {
            [ring] => match ring.as_slice() {
                [Edge::Analytic(edge @ normal_sweep::Edge::Circle { .. })] => Some(*edge),
                _ => None,
            },
            _ => None,
        }
    }
    /// All actual inner boundary loops; wall order follows these stored rings.
    pub fn holes(&self) -> &[Vec<Edge>] {
        &self.holes
    }
    pub fn rings(&self) -> impl Iterator<Item = &[Edge]> {
        std::iter::once(self.outline.as_slice()).chain(self.holes.iter().map(Vec::as_slice))
    }
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.rings().flatten()
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
        let allowance = error_mm2 / self.height_mm / self.edges().count() as f64;
        let analytic_area = 2. * self.area_mm2;
        if allowance <= 0. {
            return Err(Error::Invalid);
        }
        let mut budget = cubic::MAX_LENGTH_SPLITS;
        let mut sum = [Sum::default(); 2];
        for edge in self.edges() {
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
        let mut footprint = Footprint::default();
        let limit = (options.triangle_limit + 4) / 4;
        let mut count = 0usize;
        let mut wall = 2_u32;
        for (ring_index, ring) in self.rings().enumerate() {
            let mut samples = Vec::new();
            for edge in ring {
                let mut points = edge.sample_with_limit(options, limit.saturating_sub(count))?;
                points.pop();
                count += points.len();
                samples.extend(points.into_iter().map(|point| (point, wall)));
                wall += 1;
            }
            if ring_index == 0 {
                footprint.ring(samples.into_iter(), options)?;
            } else {
                footprint.hole(samples.into_iter(), options)?;
            }
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
