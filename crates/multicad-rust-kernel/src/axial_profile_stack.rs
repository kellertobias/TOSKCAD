//! Exact axial union of coincident profiles or a circular profile inscribed
//! against multiple sides of one enclosing rectangle.
use crate::normal_sweep::Edge as Edge2;
use std::{f64::consts::PI, ops::Range};

mod shell;
#[cfg(test)]
mod tests;

type Point = [f64; 3];
type Point2 = [f64; 2];
const MAX_OPERANDS: usize = 129;
const MAX_COORDINATE_MM: f64 = 1e7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    Circle { center: Point2, radius_mm: f64 },
    Rectangle { min: Point2, max: Point2 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Operand {
    pub span_mm: [f64; 2],
    pub profile: Profile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Unsupported,
    Disconnected,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Line,
    /// Sweep from the first vertex about the section's +Z axis.
    Arc {
        center_mm: Point,
        radius_mm: f64,
        sweep_rad: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub vertices: [usize; 2],
    pub curve: Curve,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Support {
    Plane {
        normal: Point,
    },
    /// This boundary wall is outward on the circle.
    Cylinder {
        center_mm: Point2,
        radius_mm: f64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrientedEdge {
    pub edge: usize,
    pub forward: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub support: Support,
    /// One simple loop, oriented with the outward support.
    pub boundary: Vec<OrientedEdge>,
    pub center_mm: Point,
    pub area_mm2: f64,
    pub sources: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileStack {
    tolerance_mm: f64,
    bounds: [Point; 2],
    volume: f64,
    vertices: Vec<Point>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Circle,
    Rectangle,
}

struct Slab {
    span: [f64; 2],
    section: Section,
    sources: Vec<usize>,
}

struct Rim2 {
    edges: Vec<Edge2>,
    /// Boundary pieces that share a single support surface.
    groups: Vec<Range<usize>>,
    center: Point2,
    area: f64,
    radius: Option<f64>,
}

struct Profiles {
    circle: Option<Rim2>,
    rectangle: Option<Rim2>,
    /// Ordered rectangle edge indices whose starting vertex touches the circle.
    contacts: Vec<usize>,
}

impl ProfileStack {
    /// Build an outward, shared-edge shell from overlapping or touching axial
    /// spans. Mixed profiles require exact contact on at least two rectangle
    /// sides so every transition patch has its own simple boundary.
    pub fn union(operands: &[Operand], tolerance_mm: f64, minimum_mm: f64) -> Result<Self, Error> {
        if operands.len() > MAX_OPERANDS {
            return Err(Error::Limit);
        }
        if operands.is_empty()
            || !tolerance_mm.is_finite()
            || tolerance_mm < 0.
            || !minimum_mm.is_finite()
            || minimum_mm <= tolerance_mm
        {
            return Err(Error::Invalid);
        }
        let mut circle = None;
        let mut rectangle = None;
        let mut levels = Vec::with_capacity(2 * operands.len());
        for operand in operands {
            let [lo, hi] = operand.span_mm;
            if !valid_coordinate(lo) || !valid_coordinate(hi) || hi - lo < minimum_mm {
                return Err(Error::Invalid);
            }
            operand.profile.check(minimum_mm)?;
            match operand.profile {
                Profile::Circle { center, radius_mm } => unique(&mut circle, (center, radius_mm))?,
                Profile::Rectangle { min, max } => unique(&mut rectangle, [min, max])?,
            }
            levels.extend([lo, hi]);
        }
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        let mut slabs: Vec<Slab> = Vec::new();
        for interval in levels.windows(2) {
            let [lo, hi] = [interval[0], interval[1]];
            let active = || {
                operands
                    .iter()
                    .enumerate()
                    .filter(|(_, operand)| operand.span_mm[0] <= lo && operand.span_mm[1] >= hi)
            };
            let section = if active().any(|(_, item)| item.profile.section() == Section::Rectangle)
            {
                Section::Rectangle
            } else {
                Section::Circle
            };
            let sources: Vec<_> = active()
                .filter_map(|(index, item)| (item.profile.section() == section).then_some(index))
                .collect();
            if sources.is_empty() {
                return Err(Error::Disconnected);
            }
            if let Some(previous) = slabs
                .last_mut()
                .filter(|previous| previous.section == section)
            {
                previous.span[1] = hi;
                merge_sources(&mut previous.sources, &sources);
            } else {
                slabs.push(Slab {
                    span: [lo, hi],
                    section,
                    sources,
                });
            }
        }
        if slabs
            .iter()
            .any(|slab| slab.span[1] - slab.span[0] < minimum_mm)
        {
            return Err(Error::Invalid);
        }
        let shapes = Profiles::new(circle, rectangle, minimum_mm)?;
        shell::build(&shapes, &slabs, tolerance_mm, minimum_mm)
    }

    pub(crate) fn tolerance_mm(&self) -> f64 {
        self.tolerance_mm
    }

    pub fn bounds_mm(&self) -> [Point; 2] {
        self.bounds
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume
    }

    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn faces(&self) -> &[Face] {
        &self.faces
    }
}

fn valid_coordinate(x: f64) -> bool {
    x.is_finite() && x.abs() <= MAX_COORDINATE_MM
}

impl Profile {
    fn section(self) -> Section {
        match self {
            Self::Circle { .. } => Section::Circle,
            Self::Rectangle { .. } => Section::Rectangle,
        }
    }

    fn check(self, minimum: f64) -> Result<(), Error> {
        let valid = match self {
            Self::Circle { center, radius_mm } => {
                radius_mm.is_finite()
                    && radius_mm >= minimum
                    && center.into_iter().all(|value| {
                        valid_coordinate(value - radius_mm) && valid_coordinate(value + radius_mm)
                    })
            }
            Self::Rectangle { min, max } => min
                .into_iter()
                .zip(max)
                .all(|(lo, hi)| valid_coordinate(lo) && valid_coordinate(hi) && hi - lo >= minimum),
        };
        valid.then_some(()).ok_or(Error::Invalid)
    }
}

impl Profiles {
    fn new(
        circle: Option<(Point2, f64)>,
        rectangle: Option<[Point2; 2]>,
        minimum: f64,
    ) -> Result<Self, Error> {
        let mut contacts = Vec::new();
        let mut tangent = [false; 4];
        let rectangle = rectangle
            .map(|[min, max]| {
                if let Some((center, r)) = circle {
                    let clearance = [
                        center[1] - r - min[1],
                        max[0] - (center[0] + r),
                        max[1] - (center[1] + r),
                        center[0] - r - min[0],
                    ];
                    if clearance
                        .iter()
                        .any(|&gap| gap < 0. || (gap > 0. && gap < minimum))
                    {
                        return Err(Error::Unsupported);
                    }
                    tangent = clearance.map(|gap| gap == 0.);
                    if tangent.into_iter().filter(|&side| side).count() < 2 {
                        return Err(Error::Unsupported);
                    }
                }
                let corners = [min, [max[0], min[1]], max, [min[0], max[1]]];
                let mut edges = Vec::with_capacity(8);
                let mut groups = Vec::with_capacity(4);
                for i in 0..4 {
                    let first = edges.len();
                    let [a, b] = [corners[i], corners[(i + 1) % 4]];
                    if tangent[i] {
                        let (center, _) = circle.expect("contacts need a circle");
                        let p = if i % 2 == 0 {
                            [center[0], a[1]]
                        } else {
                            [a[0], center[1]]
                        };
                        contacts.push(first + 1);
                        edges.extend([Edge2::Line { a, b: p }, Edge2::Line { a: p, b }]);
                    } else {
                        edges.push(Edge2::Line { a, b });
                    }
                    groups.push(first..edges.len());
                }
                Ok(Rim2 {
                    edges,
                    groups,
                    center: std::array::from_fn(|i| min[i].midpoint(max[i])),
                    area: (max[0] - min[0]) * (max[1] - min[1]),
                    radius: None,
                })
            })
            .transpose()?;
        let circle = circle.map(|(center, r)| {
            let directions = [[0., -1.], [1., 0.], [0., 1.], [-1., 0.]];
            let points: Vec<_> = directions
                .into_iter()
                .enumerate()
                .filter(|(i, _)| rectangle.is_none() || tangent[*i])
                .map(|(_, d)| std::array::from_fn(|i| center[i] + r * d[i]))
                .collect();
            let edges = points
                .iter()
                .copied()
                .zip(points.iter().copied().cycle().skip(1))
                .map(|(a, b)| Edge2::Arc {
                    center,
                    a,
                    b,
                    clockwise: false,
                })
                .collect();
            let groups = (0..points.len()).map(|i| i..i + 1).collect();
            Rim2 {
                edges,
                groups,
                center,
                area: PI * r * r,
                radius: Some(r),
            }
        });
        Ok(Self {
            circle,
            rectangle,
            contacts,
        })
    }

    fn rim(&self, section: Section) -> &Rim2 {
        match section {
            Section::Circle => self.circle.as_ref().expect("circle section exists"),
            Section::Rectangle => self.rectangle.as_ref().expect("rectangle section exists"),
        }
    }
}

fn unique<T: PartialEq>(slot: &mut Option<T>, profile: T) -> Result<(), Error> {
    if slot.as_ref().is_some_and(|known| *known != profile) {
        return Err(Error::Unsupported);
    }
    *slot = Some(profile);
    Ok(())
}

fn merge_sources(a: &mut Vec<usize>, b: &[usize]) {
    a.extend_from_slice(b);
    a.sort_unstable();
    a.dedup();
}
