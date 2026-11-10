//! Exact axis-aligned cross sections of orthogonal stock with blind recesses.
use crate::{
    orthogonal::{self, BooleanError, Box3, Operation, OrthogonalSolid},
    orthogonal_boundary::{self, Boundary, BoundaryError},
    orthogonal_pockets::{FacePlane, OrthogonalPocket},
    orthogonal_slots::{self, OrthogonalSlot},
    stadium::Stadium,
};
use std::f64::consts::{PI, TAU};

type Point = [f64; 2];
type Point3 = [f64; 3];
type Interval = [f64; 2];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Line([Point; 2]),
    Arc {
        center: Point,
        radius: f64,
        start_rad: f64,
        sweep_rad: f64,
    },
}

impl Edge {
    fn reversed(self) -> Self {
        match self {
            Self::Line([a, b]) => Self::Line([b, a]),
            Self::Arc {
                center,
                radius,
                start_rad,
                sweep_rad,
            } => Self::Arc {
                center,
                radius,
                start_rad: start_rad + sweep_rad,
                sweep_rad: -sweep_rad,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub axis: usize,
    pub depth_mm: f64,
    /// Window along the two positive local axes following the section axis.
    pub windows_mm: [Option<Interval>; 2],
    pub tolerance_mm: f64,
    pub segment_limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionError {
    Invalid,
    Unsupported,
    Limit,
}

type Result<T> = std::result::Result<T, SectionError>;

/// Counterclockwise tangent lines and semicircles of a positive stadium.
pub fn stadium_edges(profile: Stadium) -> [Edge; 4] {
    let corners = profile.corners();
    let centers = profile.centers();
    std::array::from_fn(|i| {
        let start = corners[i];
        if i.is_multiple_of(2) {
            Edge::Line([start, corners[(i + 1) % 4]])
        } else {
            let center = centers[usize::from(i == 1)];
            Edge::Arc {
                center,
                radius: profile.radius_mm(),
                start_rad: (start[1] - center[1]).atan2(start[0] - center[0]),
                sweep_rad: PI,
            }
        }
    })
}

fn ordered(a: f64, b: f64) -> Interval {
    [a.min(b), a.max(b)]
}

fn disk_interval(center: Point, radius: f64, axis: usize, depth: f64) -> Option<Interval> {
    let offset = (depth - center[axis]).abs();
    if offset >= radius {
        return None;
    }
    let span = ((radius - offset) * (radius + offset)).sqrt();
    Some([center[1 - axis] - span, center[1 - axis] + span])
}

/// Exact chord of a stadium along the other profile axis. The tangent
/// supporting plane contributes no cut, even along a straight side.
pub fn stadium_interval(profile: Stadium, axis: usize, depth: f64) -> Result<Option<Interval>> {
    if axis >= 2 || !depth.is_finite() {
        return Err(SectionError::Invalid);
    }
    let centers = profile.centers();
    let radius = profile.radius_mm();
    let [min, max] = ordered(centers[0][axis], centers[1][axis]);
    if depth <= min - radius || depth >= max + radius {
        return Ok(None);
    }

    // A stadium is the union of the strip between the tangencies and the
    // two end disks. Its intersection with the line is one convex interval.
    let mut interval = [f64::INFINITY, f64::NEG_INFINITY];
    let mut include = |value: f64| {
        interval[0] = interval[0].min(value);
        interval[1] = interval[1].max(value);
    };
    for center in centers {
        if let Some([lo, hi]) = disk_interval(center, radius, axis, depth) {
            include(lo);
            include(hi);
        }
    }
    let corners = profile.corners();
    for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
        if a[axis] == depth {
            include(a[1 - axis]);
        }
        if (a[axis] < depth && b[axis] > depth) || (b[axis] < depth && a[axis] > depth) {
            let t = (depth - a[axis]) / (b[axis] - a[axis]);
            include((b[1 - axis] - a[1 - axis]).mul_add(t, a[1 - axis]));
        }
    }
    if !interval.into_iter().all(f64::is_finite) || interval[0] >= interval[1] {
        return Err(SectionError::Unsupported);
    }
    Ok(Some(interval))
}

enum Mouth {
    Circle(Point3),
    Stadium([Point3; 2]),
}

struct Cavity {
    face: usize,
    mouth: Mouth,
    radius: f64,
    depth: f64,
}

enum Profile {
    Circle(Point, f64),
    Stadium(Stadium),
}

impl Profile {
    fn bounds(&self) -> [Interval; 2] {
        let (centers, radius) = match *self {
            Self::Circle(center, radius) => ([center; 2], radius),
            Self::Stadium(profile) => (profile.centers(), profile.radius_mm()),
        };
        std::array::from_fn(|i| {
            let [a, b] = ordered(centers[0][i], centers[1][i]);
            [a - radius, b + radius]
        })
    }

    fn chord(&self, axis: usize, depth: f64) -> Result<Option<Interval>> {
        match *self {
            Self::Circle(center, radius) => Ok(disk_interval(center, radius, axis, depth)),
            Self::Stadium(profile) => stadium_interval(profile, axis, depth),
        }
    }

    fn hole(self) -> Vec<Edge> {
        match self {
            Self::Circle(center, radius) => vec![Edge::Arc {
                center,
                radius,
                start_rad: 0.,
                sweep_rad: -TAU,
            }],
            Self::Stadium(profile) => stadium_edges(profile)
                .into_iter()
                .rev()
                .map(Edge::reversed)
                .collect(),
        }
    }
}

fn axes(axis: usize) -> [usize; 2] {
    [(axis + 1) % 3, (axis + 2) % 3]
}

fn lift_rect(mut bounds: [Interval; 2], options: Options) -> Result<Option<Box3>> {
    for (range, window) in bounds.iter_mut().zip(options.windows_mm) {
        if let Some([lo, hi]) = window {
            range[0] = range[0].max(lo);
            range[1] = range[1].min(hi);
        }
    }
    if bounds.iter().any(|[lo, hi]| lo >= hi) {
        return Ok(None);
    }
    if bounds
        .iter()
        .any(|[lo, hi]| hi - lo <= options.tolerance_mm)
    {
        return Err(SectionError::Unsupported);
    }
    let [[a, b], [c, d]] = bounds;
    Ok(Some(Box3 {
        min: [a, c, 0.],
        max: [b, d, 1.],
    }))
}

fn contains_hole(bounds: [Interval; 2], options: Options) -> Result<bool> {
    if bounds
        .iter()
        .zip(options.windows_mm)
        .any(|([a, b], window)| window.is_some_and(|[lo, hi]| *b <= lo || *a >= hi))
    {
        return Ok(false);
    }
    if bounds
        .iter()
        .zip(options.windows_mm)
        .any(|([a, b], window)| {
            window.is_some_and(|[lo, hi]| {
                *a - lo <= options.tolerance_mm || hi - *b <= options.tolerance_mm
            })
        })
    {
        return Err(SectionError::Unsupported);
    }
    Ok(true)
}

fn slice_cavity(
    cavity: Cavity,
    boundary: &Boundary,
    options: Options,
    notches: &mut Vec<Box3>,
    holes: &mut Vec<Vec<Edge>>,
) -> Result<()> {
    let face = boundary
        .faces
        .get(cavity.face)
        .ok_or(SectionError::Invalid)?;
    let plane = FacePlane::from_face(face).ok_or(SectionError::Invalid)?;
    if !cavity.radius.is_finite()
        || cavity.radius <= options.tolerance_mm
        || !cavity.depth.is_finite()
        || cavity.depth <= options.tolerance_mm
    {
        return Err(SectionError::Invalid);
    }
    let project = |point: Point3| {
        if point[plane.axis] != plane.level || point.iter().any(|v| !v.is_finite()) {
            return Err(SectionError::Invalid);
        }
        Ok(axes(plane.axis).map(|i| point[i]))
    };
    let profile = match cavity.mouth {
        Mouth::Circle(center) => Profile::Circle(project(center)?, cavity.radius),
        Mouth::Stadium([a, b]) => Profile::Stadium(
            Stadium::new([project(a)?, project(b)?], cavity.radius)
                .map_err(|_| SectionError::Invalid)?,
        ),
    };
    if options.axis == plane.axis {
        let inward = plane.sign * (plane.level - options.depth_mm);
        // The mouth is open at its stock face. A coincident blind floor is
        // itself material, so its cutter has no cross-sectional area there.
        if inward >= -options.tolerance_mm
            && inward < cavity.depth - options.tolerance_mm
            && contains_hole(profile.bounds(), options)?
        {
            holes.push(profile.hole());
        }
    } else {
        let cutter_axes = axes(plane.axis);
        let chord_axis = usize::from(options.axis == cutter_axes[1]);
        if let Some(chord) = profile.chord(chord_axis, options.depth_mm)? {
            let floor = plane.level - plane.sign * cavity.depth;
            let span = ordered(floor, plane.level);
            let bounds =
                axes(options.axis).map(|axis| if axis == plane.axis { span } else { chord });
            notches.extend(lift_rect(bounds, options)?);
        }
    }
    Ok(())
}

/// Cut matching orthogonal stock and certified blind recesses in local
/// coordinates. Results are closed wires around material, outer wires
/// counterclockwise and void wires clockwise about the positive section axis.
/// A lateral window may clip straight edges but must clear any curved void.
pub fn section(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    pockets: &[OrthogonalPocket],
    slots: &[OrthogonalSlot],
    options: Options,
) -> Result<Vec<Vec<Edge>>> {
    let tolerance = options.tolerance_mm;
    if options.axis >= 3
        || !options.depth_mm.is_finite()
        || !(1e-7..=1e-3).contains(&tolerance)
        || options
            .windows_mm
            .into_iter()
            .flatten()
            .any(|[a, b]| !a.is_finite() || !b.is_finite() || b - a <= tolerance)
    {
        return Err(SectionError::Invalid);
    }
    if pockets.len().saturating_add(slots.len()) > orthogonal_slots::slot_capacity(stock) {
        return Err(SectionError::Limit);
    }
    let [u, v] = axes(options.axis);
    let positive: Vec<_> = stock
        .cells()
        .iter()
        .filter(|cell| {
            cell.min[options.axis] - tolerance <= options.depth_mm
                && options.depth_mm <= cell.max[options.axis] + tolerance
        })
        .filter_map(|cell| {
            lift_rect(
                [[cell.min[u], cell.max[u]], [cell.min[v], cell.max[v]]],
                options,
            )
            .transpose()
        })
        .collect::<Result<_>>()?;
    if positive.is_empty() {
        return Ok(Vec::new());
    }
    let cavities = pockets
        .iter()
        .map(|pocket| Cavity {
            face: pocket.face,
            mouth: Mouth::Circle(pocket.center_mm),
            radius: pocket.radius_mm,
            depth: pocket.depth_mm,
        })
        .chain(slots.iter().map(|slot| Cavity {
            face: slot.face,
            mouth: Mouth::Stadium(slot.centers_mm),
            radius: slot.radius_mm,
            depth: slot.depth_mm,
        }));
    let mut holes = Vec::new();
    let mut notches = Vec::new();
    for cavity in cavities {
        slice_cavity(cavity, boundary, options, &mut notches, &mut holes)?;
    }
    let cut = orthogonal::boolean(&positive, &notches, Operation::Subtract, tolerance)
        .map_err(boolean_error)?;
    let Some(cut) = cut else {
        return Ok(Vec::new());
    };
    let cut = orthogonal_boundary::build(cut.quads()).map_err(boundary_error)?;
    let mut wires: Vec<Vec<Edge>> = cut
        .faces
        .into_iter()
        .filter(|face| face.normal == [0., 0., 1.])
        .flat_map(|face| face.rings)
        .map(|ring| {
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(&[ax, ay, _], &[bx, by, _])| Edge::Line([[ax, ay], [bx, by]]))
                .collect()
        })
        .collect();
    wires.extend(holes);
    if wires.iter().map(Vec::len).sum::<usize>() > options.segment_limit {
        return Err(SectionError::Limit);
    }
    Ok(wires)
}

fn boolean_error(error: BooleanError) -> SectionError {
    match error {
        BooleanError::Invalid => SectionError::Invalid,
        BooleanError::Unresolved => SectionError::Unsupported,
        BooleanError::Limit => SectionError::Limit,
    }
}

fn boundary_error(error: BoundaryError) -> SectionError {
    match error {
        BoundaryError::Invalid => SectionError::Invalid,
        BoundaryError::Unresolved => SectionError::Unsupported,
        BoundaryError::Limit => SectionError::Limit,
    }
}

#[cfg(test)]
mod tests;
