//! Drill a certified axial path with an optional wider convex end recess.
use super::*;
use std::f64::consts::PI;

mod patch;
mod proof;
#[cfg(test)]
mod tests;

const MAX_CUTS: usize = 64;
const MAX_RECESS_SIDES: usize = 64;
type XY = [f64; 2];

#[derive(Clone, Debug, PartialEq)]
pub struct AxialCut {
    pub start_mm: Point,
    pub end_mm: Point,
    pub radius_mm: f64,
    pub source: usize,
    pub wall_source_face: usize,
    pub cap_source_faces: [usize; 2],
    pub recess: Option<ConvexRecess>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvexRecess {
    /// One complete straight-edged extrusion section in physical coordinates.
    pub rim_mm: Vec<Point>,
    pub travel_mm: Point,
    pub source: usize,
    /// Source wall face for each directed rim edge.
    pub wall_source_faces: Vec<usize>,
    pub cap_source_faces: [usize; 2],
}

#[derive(Clone, Copy)]
struct Tube {
    start: Point,
    axes: [Point; 3],
    length: f64,
    radius: f64,
}

impl Tube {
    fn point(self, level: f64) -> Point {
        std::array::from_fn(|i| self.start[i] + level * self.axes[2][i])
    }

    fn lift(self, xy: XY, level: f64) -> Point {
        let point = self.point(level);
        std::array::from_fn(|i| point[i] + xy[0] * self.axes[0][i] + xy[1] * self.axes[1][i])
    }

    fn range(self, direction: Point, datum: Point) -> [f64; 2] {
        let a = dot(sub(self.start, datum), direction);
        let b = a + self.length * dot(self.axes[2], direction);
        let radial = self.radius * length3(cross(self.axes[2], direction));
        [a.min(b) - radial, a.max(b) + radial]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Mouth {
    face: usize,
    level: f64,
    entering: bool,
}

struct Path {
    entry: Mouth,
    exit: Option<Mouth>,
    reversed: bool,
}

struct RecessPlan {
    polygon: Vec<XY>,
    sides: Vec<usize>,
    area: f64,
    shoulder: f64,
    from_entry: bool,
    source: usize,
    shoulder_source: usize,
}

struct Plan<'a> {
    spec: &'a AxialCut,
    tube: Tube,
    path: Path,
    recess: Option<RecessPlan>,
    envelope: Tube,
}

impl Shell {
    /// Subtract separated axial cutters only after certifying their complete
    /// swept envelopes against the existing analytic boundary.
    pub fn cut_axial(&self, cuts: &[AxialCut], tolerance_mm: f64) -> Result<Self, Error> {
        if !(1e-10..=1e-3).contains(&tolerance_mm) || cuts.is_empty() {
            return Err(Error::Invalid);
        }
        if cuts.len() > MAX_CUTS {
            return Err(Error::Limit);
        }
        let mut plans: Vec<Plan<'_>> = Vec::with_capacity(cuts.len());
        for spec in cuts {
            let plan = Plan::prove(self, spec, tolerance_mm)?;
            if plans
                .iter()
                .any(|previous| !plan.disjoint(previous, tolerance_mm))
            {
                return Err(Error::Unsupported);
            }
            plans.push(plan);
        }
        let mut shell = self.clone();
        let mut removed = metrics::Sum::default();
        for plan in plans {
            removed.add(plan.append(&mut shell)?);
        }
        shell.volume_mm3 = positive_volume(shell.volume_mm3 - removed.value())?;
        Ok(shell)
    }
}

fn det([a, b]: XY, [c, d]: XY) -> f64 {
    a * d - b * c
}

fn difference(a: XY, b: XY) -> XY {
    [a[0] - b[0], a[1] - b[1]]
}
