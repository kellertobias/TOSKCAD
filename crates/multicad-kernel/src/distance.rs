//! Exact finite B-rep distance. Selection ordinals are evaluation-local and must
//! be obtained by resolving a persistent reference against the current solid.
use crate::{GeometryError, GeometryErrorKind, Solid, ffi};
mod circular;
mod planar;
mod rounded;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BRepSelection {
    Whole,
    Face(u32),
    Edge(u32),
    Vertex(u32),
}
impl BRepSelection {
    fn native(self) -> (u8, u32) {
        match self {
            Self::Whole => (0, 0),
            Self::Face(i) => (1, i),
            Self::Edge(i) => (2, i),
            Self::Vertex(i) => (3, i),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistanceWitness {
    pub a_mm: [f64; 3],
    pub b_mm: [f64; 3],
}
#[derive(Clone, Debug, PartialEq)]
pub struct MinimumDistance {
    pub distance_mm: f64,
    /// Ephemeral witnesses, never persisted as topology identities. Multiple
    /// solutions may make axis components nonunique even when the scalar is valid.
    pub witnesses: Vec<DistanceWitness>,
    pub witnesses_complete: bool,
    pub total_solutions: u32,
    pub inner_solution: bool,
}
fn error(kind: GeometryErrorKind, message: impl ToString) -> GeometryError {
    GeometryError {
        operation: "minimum distance",
        kind,
        message: message.to_string(),
    }
}
pub(crate) fn measure(
    a: &Solid,
    selection_a: BRepSelection,
    b: &Solid,
    selection_b: BRepSelection,
    tolerance: f64,
) -> Result<MinimumDistance, GeometryError> {
    if a.uses_rust_geometry() || b.uses_rust_geometry() {
        if !a.uses_rust_geometry() || !b.uses_rust_geometry() {
            return Err(error(
                GeometryErrorKind::Unsupported,
                "Exact distance requires both selections to use the same geometry backend",
            ));
        }
        if let Some(result) = rounded::measure(a, selection_a, b, selection_b, tolerance) {
            return result;
        }
        if let Some(result) = circular::measure(a, selection_a, b, selection_b, tolerance) {
            return result;
        }
        if selection_a == BRepSelection::Whole
            && selection_b == BRepSelection::Whole
            && let Some(result) = a.rust_parallel_cylinder_pair(b)
        {
            return result.map(|(distance, _)| distance);
        }
        return planar::measure(a, selection_a, b, selection_b, tolerance);
    }
    let (ak, ai) = selection_a.native();
    let (bk, bi) = selection_b.native();
    let result = ffi::minimum_distance(
        a.occt()?,
        b.occt()?,
        &ffi::DistanceOptionsData {
            a_kind: ak,
            a_index: ai,
            b_kind: bk,
            b_index: bi,
            tolerance,
            maximum: 64,
        },
    )
    .map_err(|e| error(GeometryErrorKind::InvalidResult, e))?;
    let point = |p: ffi::Point3| [p.x, p.y, p.z];
    let witnesses: Vec<_> = result
        .witnesses
        .into_iter()
        .map(|w| DistanceWitness {
            a_mm: point(w.a),
            b_mm: point(w.b),
        })
        .collect();
    if !result.distance.is_finite()
        || result.distance < 0.
        || witnesses.is_empty()
        || witnesses.iter().any(|w| {
            w.a_mm.iter().chain(&w.b_mm).any(|v| !v.is_finite())
                || ((0..3)
                    .map(|i| (w.b_mm[i] - w.a_mm[i]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    - result.distance)
                    .abs()
                    > tolerance * 4.
        })
    {
        return Err(error(
            GeometryErrorKind::InvalidResult,
            "Native distance witnesses disagree with the measured scalar",
        ));
    }
    Ok(MinimumDistance {
        distance_mm: result.distance,
        witnesses,
        witnesses_complete: result.complete,
        total_solutions: result.total_solutions,
        inner_solution: result.inner,
    })
}
