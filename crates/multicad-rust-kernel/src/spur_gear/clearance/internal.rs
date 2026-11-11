//! Positive clearance or a strict common-interior point for an external
//! spur pinion nested in the cavity of an internal ring.
use super::{
    Error, MAX_PAIR_TESTS, MAX_REFINEMENTS, Placement, ResultKind, Rim, checkpoint, tip_radius,
};
use crate::{
    revolution::length,
    spur_gear::{Point, Profile, add, internal::Ring, scale, sub},
};
use std::{collections::HashMap, f64::consts::TAU};

/// Certify one fixed pose of a ring (`placements[0]`) and a pinion
/// (`placements[1]`). Like the external certificate, a result applies only
/// to these exact outlines and never extrapolates to continuous motion.
pub fn certify_internal(
    ring: &Ring,
    pinion: &Profile,
    placements: [Placement; 2],
    tolerance_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<ResultKind, Error> {
    let [ring_center, pinion_center] = placements.map(|p| p.center_mm);
    let delta = sub(pinion_center, ring_center);
    let distance = length(delta);
    if !(1e-7..=1e-3).contains(&tolerance_mm)
        || placements.into_iter().any(|p| !p.valid(1e-3))
        || distance <= tolerance_mm
    {
        return Err(Error::Invalid);
    }
    // Only candidates, midway between the inward ring tips and the pinion
    // tips: on the line of centers and at the three ring tooth centers
    // nearest to it. Winding tests certify membership in both solids.
    let reach = (ring.tip_radius_mm() + distance + tip_radius(pinion)) * 0.5;
    let [x, y] = placements[0].axes[0];
    let phase = y.atan2(x);
    let pitch = TAU / f64::from(ring.teeth());
    let nearest = ((delta[1].atan2(delta[0]) - phase) / pitch).round();
    let witnesses = [-1., 0., 1.]
        .map(|offset| {
            let (sin, cos) = (phase + (nearest + offset) * pitch).sin_cos();
            add(ring_center, [reach * cos, reach * sin])
        })
        .into_iter()
        .chain([add(ring_center, scale(delta, reach / distance))]);
    let mut deflection =
        (pinion.root_radius_mm.min(ring.tip_radius_mm()) * 2e-4).clamp(tolerance_mm, 0.01);
    for _ in 0..MAX_REFINEMENTS {
        checkpoint(interrupt)?;
        let cavity = Rim::from_outline(ring.cavity(), 0., placements[0], deflection, interrupt)?;
        let teeth = Rim::build(pinion, placements[1], deflection, interrupt)?;
        if nested_clear(&cavity, &teeth, ring.outer_radius_mm(), interrupt)? {
            return Ok(ResultKind::Clear);
        }
        let margin = cavity.tube_mm + cavity.guard_mm;
        if witnesses.clone().any(|witness| {
            length(sub(witness, ring_center)) < ring.outer_radius_mm() - margin
                && cavity.winding(witness, margin) == Some(false)
                && teeth.contains_strictly(witness)
        }) {
            return Ok(ResultKind::CommonInterior);
        }
        deflection *= 0.25;
    }
    Ok(ResultKind::Unresolved)
}

/// With disjoint boundaries the pinion is either inside the cavity, around
/// it, or in the ring material. Prove the first: one pinion boundary point
/// lies strictly in the cavity, one cavity point lies strictly beyond the
/// pinion and the whole pinion stays strictly inside the outside rim.
fn nested_clear(
    cavity: &Rim,
    teeth: &Rim,
    outer_radius_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<bool, Error> {
    let margin = cavity.tube_mm + teeth.tube_mm + cavity.guard_mm.max(teeth.guard_mm);
    let from = |center: Point| move |p: &Point| length(sub(*p, center));
    let vertices = |rim: &Rim| {
        rim.chords
            .iter()
            .map(|chord| chord.ends[0])
            .collect::<Vec<_>>()
    };
    let [cavity_points, pinion_points] = [vertices(cavity), vertices(teeth)];
    let ring_distance = from(cavity.placement.center_mm);
    let pinion_distance = from(teeth.placement.center_mm);
    let farthest = |points: &[Point], distance: &dyn Fn(&Point) -> f64| {
        points
            .iter()
            .copied()
            .max_by(|a, b| distance(a).total_cmp(&distance(b)))
            .expect("sampled rims have chords")
    };
    let outermost = farthest(&pinion_points, &ring_distance);
    let pinion_reach = pinion_distance(&farthest(&pinion_points, &pinion_distance));
    let beyond = pinion_distance(&farthest(&cavity_points, &pinion_distance));
    if ring_distance(&outermost) + margin >= outer_radius_mm
        || beyond - pinion_reach <= margin
        || cavity.winding(outermost, margin) != Some(true)
    {
        return Ok(false);
    }
    grid_clear(cavity, teeth, margin, MAX_PAIR_TESTS, interrupt)
}

/// The nested pinion shares most of the ring's bounding box, so pair its
/// chords through a uniform grid. A cell is at least as large as every
/// chord box plus the margin; chords in disjoint cells are apart.
fn grid_clear(
    a: &Rim,
    b: &Rim,
    margin: f64,
    pair_budget: usize,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<bool, Error> {
    let cell = a
        .chords
        .iter()
        .chain(&b.chords)
        .flat_map(|chord| [0, 1].map(|i| chord.bounds.max[i] - chord.bounds.min[i]))
        .fold(margin, f64::max)
        + margin;
    let key = |value: f64| (value / cell).floor() as i64;
    let cells = |bounds: &super::Bounds, grow: f64| {
        let [x, y] = [0, 1].map(|i| key(bounds.min[i] - grow)..=key(bounds.max[i] + grow));
        x.flat_map(move |x| y.clone().map(move |y| (x, y)))
    };
    let mut grid = HashMap::<_, Vec<usize>>::new();
    for (index, chord) in b.chords.iter().enumerate() {
        for cell in cells(&chord.bounds, margin) {
            grid.entry(cell).or_default().push(index);
        }
    }
    let mut tests = 0;
    for first in &a.chords {
        checkpoint(interrupt)?;
        for cell in cells(&first.bounds, 0.) {
            for &index in grid.get(&cell).into_iter().flatten() {
                tests += 1;
                if tests > pair_budget {
                    return Err(Error::Limit);
                }
                let second = b.chords[index];
                if !first.bounds.apart(second.bounds, margin) && !first.clear(second, margin) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
