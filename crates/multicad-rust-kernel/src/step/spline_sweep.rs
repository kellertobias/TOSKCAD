//! Exact STEP exchange of decoded cubic-and-analytic normal sweeps.
use super::{cubic_trim::CubicTrim, normal_sweep::ProfileEdge, *};
use crate::spline_sweep::{Edge, Sweep};

pub fn write_spline_sweep(
    sweep: &Sweep,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_spline_sweep(sweep, origin, u, v, tolerance_mm))
}

pub(super) fn encode(
    file: &mut StepFile,
    sweep: &Sweep,
    placement: Placement,
    tolerance_mm: f64,
) -> Result<Entity, StepError> {
    if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let tolerance = tolerance_mm.min(SPUR_STEP_MAX_ERROR_MM);
    check_placement(sweep, &placement, tolerance)?;
    let outline = sweep
        .outline()
        .iter()
        .map(|edge| match edge {
            Edge::Analytic(edge) => ProfileEdge::Analytic(*edge),
            Edge::Cubic(cubic) => ProfileEdge::Cubic(CubicTrim::decoded(cubic, tolerance)),
        })
        .collect();
    let hole = sweep.hole().map(|hole| vec![ProfileEdge::Analytic(hole)]);
    normal_sweep::encode_rings(
        file,
        &mut std::iter::once(outline).chain(hole),
        sweep.height_mm(),
        &[],
        placement,
        false,
    )
}

fn check_placement(sweep: &Sweep, placement: &Placement, tolerance: f64) -> Result<(), StepError> {
    let [min, max] = sweep.bounds();
    let extent: f64 = min
        .into_iter()
        .zip(max)
        .map(|(lo, hi)| lo.abs().max(hi.abs()))
        .sum();
    let origin = placement
        .origin
        .into_iter()
        .map(f64::abs)
        .fold(0., f64::max);
    let [u, v] = [placement.u, placement.v];
    // STEP normalizes placement directions. Bound the change in a frame
    // whose supplied basis is only orthonormal within numerical precision.
    let frame_error = (dot(u, u) - 1.).abs() + (dot(v, v) - 1.).abs() + dot(u, v).abs();
    let outline = sweep.outline();
    let joins = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(edge, next)| {
            let [x, y] = edge.point(1.);
            let [a, b] = next.point(0.);
            (x - a).hypot(y - b)
        })
        .fold(0., f64::max);
    let error = 16. * f64::EPSILON * (origin + extent) + 4. * frame_error * extent + joins;
    if !error.is_finite() || error > tolerance {
        return Err(StepError::Limit);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
