//! Bounded positive-clearance and strict common-interior proofs on exact
//! external spur outlines. Chords carry analytic error tubes and never
//! replace either manufacturing boundary.
use super::{Edge, Point, Profile, add, normal_sweep, scale, sub};
use crate::revolution::{cross, dot, length};

const MAX_CHORDS: usize = 32_768;
const MAX_PAIR_TESTS: usize = 1_000_000;
const MAX_REFINEMENTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub center_mm: Point,
    pub axes: [Point; 2],
    /// Maximum displacement from this projected rim across the common
    /// axial band, including the enclosing chart's placement roundoff.
    pub uncertainty_mm: f64,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            center_mm: [0.; 2],
            axes: [[1., 0.], [0., 1.]],
            uncertainty_mm: 0.,
        }
    }
}

impl Placement {
    fn valid(self, maximum_uncertainty: f64) -> bool {
        self.center_mm
            .into_iter()
            .all(|v| v.abs() <= super::MAX_COORDINATE_MM)
            && (length(self.axes[0]) - 1.).abs() <= 1e-12
            && (length(self.axes[1]) - 1.).abs() <= 1e-12
            && (cross(self.axes[0], self.axes[1]) - 1.).abs() <= 1e-12
            && dot(self.axes[0], self.axes[1]).abs() <= 1e-12
            && (0. ..=maximum_uncertainty).contains(&self.uncertainty_mm)
    }

    fn point(self, [u, v]: Point) -> Point {
        add(
            self.center_mm,
            add(scale(self.axes[0], u), scale(self.axes[1], v)),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResultKind {
    Clear,
    CommonInterior,
    Unresolved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Limit,
    Cancelled,
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Point,
    max: Point,
}

impl Bounds {
    fn points(points: impl IntoIterator<Item = Point>) -> Self {
        points.into_iter().fold(
            Self {
                min: [f64::INFINITY; 2],
                max: [f64::NEG_INFINITY; 2],
            },
            |mut bounds, point| {
                for (i, v) in point.into_iter().enumerate() {
                    bounds.min[i] = bounds.min[i].min(v);
                    bounds.max[i] = bounds.max[i].max(v);
                }
                bounds
            },
        )
    }

    fn apart(self, other: Self, margin: f64) -> bool {
        (0..2).any(|i| other.min[i] - self.max[i] > margin || self.min[i] - other.max[i] > margin)
    }

    fn has_point_outside(self, other: Self, margin: f64) -> bool {
        (0..2).any(|i| other.min[i] - self.min[i] > margin || self.max[i] - other.max[i] > margin)
    }
}

#[derive(Clone, Copy)]
struct Chord {
    ends: [Point; 2],
    bounds: Bounds,
}

impl Chord {
    fn new(a: Point, b: Point) -> Self {
        Self {
            ends: [a, b],
            bounds: Bounds::points([a, b]),
        }
    }

    fn axis_separates(self, other: Self, axis: Point, margin: f64) -> bool {
        let magnitude = length(axis);
        if magnitude == 0. {
            return false;
        }
        let axis = scale(axis, magnitude.recip());
        // Use a common nearby origin so the dot products do not lose the
        // separation to a large world-space translation.
        let origin = self.ends[0];
        let a = dot(sub(self.ends[1], origin), axis);
        let b = other.ends.map(|p| dot(sub(p, origin), axis));
        b[0].min(b[1]) - a.max(0.) > margin || a.min(0.) - b[0].max(b[1]) > margin
    }

    fn clear(self, other: Self, margin: f64) -> bool {
        let normal = |c: Self| {
            let [x, y] = sub(c.ends[1], c.ends[0]);
            [-y, x]
        };
        [normal(self), normal(other)]
            .into_iter()
            .chain(
                self.ends
                    .into_iter()
                    .flat_map(|a| other.ends.map(|b| sub(b, a))),
            )
            .any(|axis| self.axis_separates(other, axis, margin))
    }
}

pub(super) fn segment_tubes_clear(a: [Point; 2], b: [Point; 2], margin: f64) -> bool {
    let a = Chord::new(a[0], a[1]);
    let b = Chord::new(b[0], b[1]);
    a.bounds.apart(b.bounds, margin) || a.clear(b, margin)
}

struct Rim {
    chords: Vec<Chord>,
    bounds: Bounds,
    tube_mm: f64,
    guard_mm: f64,
    placement: Placement,
    bore_mm: f64,
}

impl Rim {
    fn build(
        profile: &Profile,
        placement: Placement,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        let mut points = Vec::new();
        let mut endpoints = Vec::with_capacity(profile.outline().len());
        let mut arc_fit_error: f64 = 0.;
        for item in profile.outline() {
            checkpoint(interrupt)?;
            let edge = item.edge;
            if let Edge::Planar(normal_sweep::Edge::Arc { center, a, b, .. }) = edge {
                // Arc evaluation returns the authored endpoint literally.
                // Enclose its radial difference from the analytic support.
                let fit = (length(sub(a, center)) - length(sub(b, center))).abs();
                arc_fit_error = arc_fit_error.max(fit);
            }
            let count = segment_count(edge, tolerance)?;
            if points.len() + count > MAX_CHORDS {
                return Err(Error::Limit);
            }
            endpoints.push((points.len(), placement.point(edge.point(1.))));
            points.extend((0..count).map(|i| placement.point(edge.point(i as f64 / count as f64))));
        }
        let bounds = Bounds::points(points.iter().copied());
        let scale = [bounds.min, bounds.max, placement.center_mm]
            .into_iter()
            .flatten()
            .map(f64::abs)
            .fold(1., f64::max);
        let guard_mm = 1024. * f64::EPSILON * scale;
        // Include the representation discrepancy at the shared analytic
        // edge vertices in the enclosure rather than leaving open chords.
        let mut join_error: f64 = 0.;
        for (i, (_, end)) in endpoints.iter().enumerate() {
            let next = endpoints[(i + 1) % endpoints.len()].0;
            join_error = join_error.max(length(sub(*end, points[next])));
        }
        let representation_error = join_error + arc_fit_error;
        if representation_error > guard_mm || guard_mm > tolerance * 0.25 {
            return Err(Error::Limit);
        }
        let chords = points
            .iter()
            .copied()
            .zip(points.iter().copied().cycle().skip(1))
            .map(|(a, b)| Chord::new(a, b))
            .collect();
        Ok(Self {
            chords,
            bounds,
            tube_mm: tolerance + representation_error + guard_mm + placement.uncertainty_mm,
            guard_mm,
            placement,
            bore_mm: profile.bore_radius_mm().unwrap_or(0.) * (1. + 4e-12),
        })
    }

    fn contains_strictly(&self, point: Point) -> bool {
        let margin = self.tube_mm + self.guard_mm;
        if length(sub(point, self.placement.center_mm)) <= self.bore_mm + margin {
            return false;
        }
        let point_chord = Chord::new(point, point);
        let mut inside = false;
        for chord in &self.chords {
            if !chord.bounds.apart(point_chord.bounds, margin) && !chord.clear(point_chord, margin)
            {
                return false;
            }
            let [a, b] = chord.ends;
            if (a[1] > point[1]) != (b[1] > point[1])
                && (cross(sub(b, a), sub(point, a)) > 0.) == (b[1] > a[1])
            {
                inside = !inside;
            }
        }
        inside
    }
}

fn segment_count(edge: Edge, tolerance: f64) -> Result<usize, Error> {
    // Error in linear interpolation is at most max|f''| / (8 n²)
    // for n uniform spans. Parameterize each exact curve on [0,1].
    let second = match edge {
        Edge::Planar(normal_sweep::Edge::Line { .. }) => 0.,
        Edge::Planar(edge) => {
            edge.radius_mm().ok_or(Error::Invalid)?
                * edge.signed_sweep_rad().ok_or(Error::Invalid)?.powi(2)
        }
        Edge::Involute(edge) => {
            let [a, b] = edge.interval();
            edge.base_radius_mm() * a.max(b).hypot(1.) * (b - a).powi(2)
        }
    };
    // Reserve half the tube for evaluating and placing the analytic edge.
    let needed = (second / (4. * tolerance)).sqrt().ceil();
    if !needed.is_finite() || needed > MAX_CHORDS as f64 {
        return Err(Error::Limit);
    }
    Ok((needed as usize).max(1))
}

fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), Error> {
    if interrupt.is_some_and(|f| f()) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn boundaries_clear(
    a: &Rim,
    b: &Rim,
    pair_budget: usize,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<bool, Error> {
    let margin = a.tube_mm + b.tube_mm + a.guard_mm.max(b.guard_mm);
    // If no exact boundaries meet, Jordan separation still admits nesting.
    // Prove one actual boundary point of each profile lies beyond the other
    // profile's bounding box, excluding either outer containment.
    if !a.bounds.has_point_outside(b.bounds, margin)
        || !b.bounds.has_point_outside(a.bounds, margin)
    {
        return Ok(false);
    }
    let right = b
        .chords
        .iter()
        .filter(|c| !c.bounds.apart(a.bounds, margin))
        .collect::<Vec<_>>();
    let mut tests = 0;
    for first in a
        .chords
        .iter()
        .filter(|c| !c.bounds.apart(b.bounds, margin))
    {
        checkpoint(interrupt)?;
        tests += right.len();
        if tests > pair_budget {
            return Err(Error::Limit);
        }
        for second in &right {
            if !first.bounds.apart(second.bounds, margin) && !first.clear(**second, margin) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn pitch_witness(a: &Profile, b: &Profile, placements: [Placement; 2]) -> Point {
    let centers = placements.map(|p| p.center_mm);
    let delta = sub(centers[1], centers[0]);
    // Only a candidate: the winding tests certify material membership. The
    // midpoint between the two tips is near the nominal pitch contact.
    let tip = |profile: &Profile| {
        profile
            .outline()
            .iter()
            .find(|edge| edge.role == super::ToothRole::Tip)
            .map_or(profile.root_radius_mm, |edge| length(edge.edge.point(0.)))
    };
    let distance = length(delta);
    let offset = (distance + tip(a) - tip(b)) * 0.5;
    add(centers[0], scale(delta, offset / distance))
}

/// Certify one fixed pair pose. A result only applies to the exact outlined
/// profiles and must never be extrapolated to a continuous motion cycle.
pub fn certify(
    [a, b]: [&Profile; 2],
    placements: [Placement; 2],
    tolerance_mm: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<ResultKind, Error> {
    certify_with_uncertainty([a, b], placements, tolerance_mm, 1e-3, interrupt)
}

fn certify_with_uncertainty(
    [a, b]: [&Profile; 2],
    placements: [Placement; 2],
    tolerance_mm: f64,
    maximum_uncertainty: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<ResultKind, Error> {
    if !(1e-7..=1e-3).contains(&tolerance_mm)
        || placements
            .into_iter()
            .any(|p| !p.valid(maximum_uncertainty))
        || length(sub(placements[0].center_mm, placements[1].center_mm)) <= tolerance_mm
    {
        return Err(Error::Invalid);
    }
    let witness = pitch_witness(a, b, placements);
    let mut deflection = (a.root_radius_mm.min(b.root_radius_mm) * 2e-4).clamp(tolerance_mm, 0.01);
    for _ in 0..MAX_REFINEMENTS {
        checkpoint(interrupt)?;
        let first = Rim::build(a, placements[0], deflection, interrupt)?;
        let second = Rim::build(b, placements[1], deflection, interrupt)?;
        if boundaries_clear(&first, &second, MAX_PAIR_TESTS, interrupt)? {
            return Ok(ResultKind::Clear);
        }
        if first.contains_strictly(witness) && second.contains_strictly(witness) {
            return Ok(ResultKind::CommonInterior);
        }
        deflection *= 0.25;
    }
    Ok(ResultKind::Unresolved)
}

mod chevron;
pub use chevron::certify_herringbone;

#[cfg(test)]
mod tests;
