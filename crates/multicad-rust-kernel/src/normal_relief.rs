//! Certified disjoint cap motifs on one unchanged analytic normal sweep.
use crate::{
    normal_sweep::{CapError, Edge, Sweep},
    revolution::Sum,
};

mod boundary;
mod mesh;
pub(crate) mod precision;
#[cfg(test)]
mod tests;
pub use boundary::{Boundary, BoundaryEdge, BoundaryFace, Curve, EdgeUse, FaceRole, Surface};

pub const MAX_MOTIFS: usize = 192;
pub const MAX_CAP_EDGES: usize = 16_384;
type Point = [f64; 3];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Raised,
    Recessed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unresolved,
    Limit,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Relief {
    base: Sweep,
    motifs: Vec<Sweep>,
    kind: Kind,
    cap: Sweep,
    tolerance: f64,
    minimum: f64,
    volume: f64,
    centroid: Point,
    area: f64,
    boundary_error: f64,
    boundary: Boundary,
}
fn cap_error(error: CapError) -> Error {
    match error {
        CapError::Cancelled => Error::Cancelled,
        CapError::Limit | CapError::Geometry(crate::normal_sweep::Error::Limit) => Error::Limit,
        _ => Error::Invalid,
    }
}
fn check(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), Error> {
    if interrupt.is_some_and(|f| f()) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn sum_error(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let part = sum - a;
    (sum, (a - (sum - part)) + (b - part))
}
fn perimeter(sweep: &Sweep) -> f64 {
    sweep
        .rings()
        .iter()
        .flatten()
        .map(|edge| edge.length_mm())
        .sum()
}
fn combined_rings(
    base: &Sweep,
    motifs: &[Sweep],
    tolerance_mm: f64,
    minimum_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<Vec<Edge>>, Error> {
    check(interrupt)?;
    if motifs.is_empty()
        || !tolerance_mm.is_finite()
        || tolerance_mm <= 0.
        || !minimum_mm.is_finite()
        || minimum_mm <= tolerance_mm
    {
        return Err(Error::Invalid);
    }
    if motifs.len() > MAX_MOTIFS {
        return Err(Error::Limit);
    }
    let count = base.rings().iter().map(Vec::len).sum::<usize>()
        + motifs
            .iter()
            .flat_map(Sweep::rings)
            .map(Vec::len)
            .sum::<usize>();
    if count > MAX_CAP_EDGES {
        return Err(Error::Limit);
    }
    let mut rings = base.rings().to_vec();
    for motif in motifs {
        check(interrupt)?;
        if motif.rings().len() != 1 {
            return Err(Error::Invalid);
        }
        rings.push(
            motif.rings()[0]
                .iter()
                .rev()
                .map(|edge| edge.reversed())
                .collect(),
        );
    }
    Ok(rings)
}
impl Relief {
    /// Prove strict XY containment/disjointness with all existing base holes.
    /// This creates no wall graph and applies no raised/recessed depth law.
    pub fn certify_footprints(
        base: &Sweep,
        motifs: &[Sweep],
        tolerance_mm: f64,
        minimum_mm: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(), Error> {
        let rings = combined_rings(base, motifs, tolerance_mm, minimum_mm, interrupt)?;
        Sweep::relief_cap(rings, minimum_mm, tolerance_mm, minimum_mm, interrupt)
            .map(|_| ())
            .map_err(cap_error)
    }
    pub fn new(
        base: Sweep,
        motifs: Vec<Sweep>,
        kind: Kind,
        tolerance_mm: f64,
        minimum_mm: f64,
    ) -> Result<Self, Error> {
        Self::new_with_interrupt(base, motifs, kind, tolerance_mm, minimum_mm, None)
    }
    pub fn new_with_interrupt(
        base: Sweep,
        motifs: Vec<Sweep>,
        kind: Kind,
        tolerance_mm: f64,
        minimum_mm: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        let rings = combined_rings(&base, &motifs, tolerance_mm, minimum_mm, interrupt)?;
        let mut boundary_error = 0_f64;
        for motif in &motifs {
            check(interrupt)?;
            if motif.height_mm() < minimum_mm {
                return Err(Error::Invalid);
            }
            let signed = if kind == Kind::Raised {
                motif.height_mm()
            } else {
                -motif.height_mm()
            };
            let (level, error) = sum_error(base.height_mm(), signed);
            boundary_error = boundary_error.max(error.abs());
            if kind == Kind::Recessed && (level - error.abs() < minimum_mm) {
                return Err(Error::Invalid);
            }
            if level.abs() + error.abs() > 1e7 {
                return Err(Error::Limit);
            }
        }
        for edge in rings.iter().flatten() {
            check(interrupt)?;
            if let Edge::Line { a, b } = *edge {
                let error = precision::line_error(a, b)?;
                boundary_error = boundary_error.max(error);
            }
            if let Some(radius) = edge.radius_mm() {
                let scale = edge
                    .start()
                    .into_iter()
                    .chain(edge.end())
                    .map(f64::abs)
                    .fold(radius.max(1.), f64::max);
                let arithmetic = 64. * f64::EPSILON * scale;
                if 8. * arithmetic / radius.min(edge.length_mm()) > 1e-6 {
                    return Err(Error::Unresolved);
                }
                boundary_error = boundary_error.max(arithmetic);
            }
            if let Edge::Arc { center, a, b, .. } = edge {
                let radius = (a[0] - center[0]).hypot(a[1] - center[1]);
                let other = (b[0] - center[0]).hypot(b[1] - center[1]);
                let scale = center
                    .iter()
                    .chain(a)
                    .chain(b)
                    .map(|v| v.abs())
                    .fold(1., f64::max);
                if (radius - other).abs() > (64. * f64::EPSILON * scale).min(tolerance_mm / 64.) {
                    return Err(Error::Unresolved);
                }
                boundary_error = boundary_error.max(64. * f64::EPSILON * scale);
            }
        }
        if boundary_error > tolerance_mm / 32. {
            return Err(Error::Unresolved);
        }
        let cap = Sweep::relief_cap(rings, base.height_mm(), tolerance_mm, minimum_mm, interrupt)
            .map_err(cap_error)?;
        // Integrate positive columns for recesses. Subtracting two nearly
        // equal full-height solids would needlessly cancel both mass and
        // first moments; the certified exposed cap gives an exact partition.
        let anchor = [
            base.centroid()[0],
            base.centroid()[1],
            base.height_mm() / 2.,
        ];
        let mut sums = [Sum::default(); 4];
        let mut add_column = |volume: f64, center: Point| {
            sums[0].add(volume);
            for i in 0..3 {
                sums[i + 1].add(volume * (center[i] - anchor[i]));
            }
        };
        let initial = if kind == Kind::Raised { &base } else { &cap };
        add_column(
            initial.volume_mm3(),
            [
                initial.centroid()[0],
                initial.centroid()[1],
                base.height_mm() / 2.,
            ],
        );
        let mut area = 2. * base.area_mm2() + base.height_mm() * perimeter(&base);
        for motif in &motifs {
            check(interrupt)?;
            let (height, z) = if kind == Kind::Raised {
                (motif.height_mm(), base.height_mm() + motif.height_mm() / 2.)
            } else {
                let floor = base.height_mm() - motif.height_mm();
                (floor, floor / 2.)
            };
            let center = motif.centroid();
            add_column(motif.area_mm2() * height, [center[0], center[1], z]);
            area += motif.height_mm() * perimeter(motif);
        }
        let [volume, x, y, z] = sums.map(Sum::value);
        let centroid = std::array::from_fn(|i| anchor[i] + [x, y, z][i] / volume);
        if !volume.is_finite()
            || volume <= 0.
            || !area.is_finite()
            || !centroid.iter().all(|v| v.is_finite())
        {
            return Err(Error::Unresolved);
        }
        let mut relief = Self {
            base,
            motifs,
            kind,
            cap,
            tolerance: tolerance_mm,
            minimum: minimum_mm,
            volume,
            centroid,
            area,
            boundary_error,
            boundary: Boundary::default(),
        };
        relief.boundary = boundary::build(&relief, interrupt)?;
        check(interrupt)?;
        Ok(relief)
    }
    pub fn base(&self) -> &Sweep {
        &self.base
    }
    pub fn motifs(&self) -> &[Sweep] {
        &self.motifs
    }
    pub fn exposed_cap(&self) -> &Sweep {
        &self.cap
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn tolerance_mm(&self) -> f64 {
        self.tolerance
    }
    pub fn minimum_mm(&self) -> f64 {
        self.minimum
    }
    pub fn boundary_error_mm(&self) -> f64 {
        self.boundary_error
    }
    pub fn volume_mm3(&self) -> f64 {
        self.volume
    }
    pub fn centroid_mm(&self) -> Point {
        self.centroid
    }
    pub fn surface_area_mm2(&self) -> f64 {
        self.area
    }
    pub fn boundary(&self) -> &Boundary {
        &self.boundary
    }
    pub fn motif_level_mm(&self, index: usize) -> f64 {
        let sign = if self.kind == Kind::Raised { 1. } else { -1. };
        self.base.height_mm() + sign * self.motifs[index].height_mm()
    }
    pub fn support(&self, direction: Point) -> f64 {
        let mut result = self.base.support(direction);
        if self.kind == Kind::Raised {
            for motif in &self.motifs {
                result =
                    result.max(motif.support(direction) + direction[2] * self.base.height_mm());
            }
        }
        result
    }
    pub fn bounds(&self) -> [Point; 2] {
        let axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        [
            axes.map(|axis| -self.support(axis.map(|v| -v))),
            axes.map(|axis| self.support(axis)),
        ]
    }
    pub fn contains(&self, point: Point, tolerance: f64) -> bool {
        if !point
            .iter()
            .chain([tolerance].iter())
            .all(|v| v.is_finite())
            || tolerance < 0.
        {
            return false;
        }
        if self.kind == Kind::Raised {
            return self.base.contains(point, tolerance)
                || self.motifs.iter().any(|motif| {
                    motif.contains(
                        [point[0], point[1], point[2] - self.base.height_mm()],
                        tolerance,
                    )
                });
        }
        if !self.base.contains(point, tolerance) {
            return false;
        }
        if self.cap.contains([point[0], point[1], 0.], tolerance) {
            return true;
        }
        self.motifs.iter().enumerate().any(|(i, motif)| {
            motif.contains([point[0], point[1], 0.], tolerance)
                && point[2] <= self.motif_level_mm(i) + tolerance
        })
    }
}
