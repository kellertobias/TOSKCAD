//! Build an exact drawing shell from the certified sweep and its cut recipe.
use super::*;
use crate::warped_union::{MAX_METRIC_WORK, cylinder_patch::ANGULAR};
use std::f64::consts::FRAC_PI_2;

/// Materialize a normal sweep in its local profile coordinates.
/// The caps are at zero and the signed height.
pub fn normal_sweep_shell(sweep: &Sweep, reverse: bool, tolerance_mm: f64) -> Result<Shell, Error> {
    build(sweep, &[], sweep.volume_mm3(), reverse, tolerance_mm)
}

/// Materialize a normal sweep and its certified through openings together.
pub fn pierced_sweep_shell(
    pierced: &PiercedSweep,
    reverse: bool,
    tolerance_mm: f64,
) -> Result<Shell, Error> {
    build(
        pierced.sweep(),
        pierced.openings(),
        pierced.volume_mm3(),
        reverse,
        tolerance_mm,
    )
}

fn build(
    sweep: &Sweep,
    openings: &[CertifiedPiercing],
    volume_mm3: f64,
    reverse: bool,
    tolerance: f64,
) -> Result<Shell, Error> {
    if !(1e-7..=1e-3).contains(&tolerance) {
        return Err(Error::Invalid);
    }
    let profile = sweep.rings().iter().flatten();
    if profile.clone().any(|edge| {
        matches!(edge, ProfileEdge::Arc { .. })
            && edge
                .signed_sweep_rad()
                .is_some_and(|sweep| sweep.abs() > FRAC_PI_2 + ANGULAR)
    }) {
        return Err(Error::Unsupported);
    }
    let segments = profile
        .chain(openings.iter().flat_map(|opening| &opening.spec().outline))
        .map(|edge| {
            if matches!(edge, ProfileEdge::Circle { .. }) {
                4usize
            } else {
                1
            }
        })
        .sum::<usize>();
    // Two rims per edge, including subdivisions of complete circles.
    // Bound the shared builder's vertex matching work before allocation.
    let vertices = segments * 2;
    if vertices.saturating_mul(vertices) > MAX_METRIC_WORK {
        return Err(Error::Limit);
    }

    let frame = SweepFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
        direction: [0., 0., if reverse { -1. } else { 1. }],
    };
    let mut next_face = 2;
    let rings: Vec<Vec<_>> = sweep
        .rings()
        .iter()
        .map(|ring| {
            let first = next_face;
            next_face += ring.len();
            ring.iter().copied().zip(first..next_face).collect()
        })
        .collect();
    let rings = rings.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let mut builder = Builder::new(tolerance);
    builder.sweep(&rings, sweep, frame, [0, 1], std::iter::empty())?;

    if !openings.is_empty() {
        // Circular subdivisions can shift the builder's wall indices, but
        // every certified host is a straight wall and emits exactly one face.
        let mut walls = vec![0; next_face];
        for (index, face) in builder.faces.iter().enumerate() {
            walls[face.source_face] = index;
        }
        for opening in openings {
            let spec = opening.spec();
            let first = next_face;
            next_face += spec.outline.len();
            let mouths = spec.sides.map(|side| walls[side + 2]);
            builder.pierce(opening, frame, mouths, first..next_face)?;
        }
    }
    builder.finish(frame.bounds(sweep), volume_mm3)
}

#[cfg(test)]
mod tests;
