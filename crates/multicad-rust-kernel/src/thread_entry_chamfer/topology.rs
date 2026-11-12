//! Direct source topology: semantic curves own their analytic construction.
//!
//! Nominal points are accompanied by source enclosures and never replace the
//! retained helical/conical material. No STEP record or display mesh is read.
use super::{interval::I, *};
use std::collections::{BTreeMap, BTreeSet};
mod builder;
mod coupling;
mod curve;
mod validate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseConstraint {
    ProfileKnot(usize),
    Valley,
    EntryIntersection { band: usize, half: ConeHalf },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhaseEndpoint {
    interval: Enclosure,
    nominal: f64,
    constraint: PhaseConstraint,
    contact: bool,
    exact_contact: bool,
}
impl PhaseEndpoint {
    pub fn interval(&self) -> Enclosure {
        self.interval
    }
    pub fn nominal(&self) -> f64 {
        self.nominal
    }
    pub fn constraint(&self) -> PhaseConstraint {
        self.constraint
    }
    fn range(self) -> I {
        I::new(self.interval.lower, self.interval.upper)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointSample {
    pub nominal_mm: Point,
    /// Euclidean displacement is at most this outward L1 bound.
    pub maximum_error_mm: f64,
    pub coordinates_mm: [Enclosure; 3],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SourceCurve {
    Intersection {
        start: u32,
        band: usize,
        phase: [PhaseEndpoint; 2],
        active: bool,
    },
    EntryArc {
        start: u32,
        phase: [PhaseEndpoint; 2],
    },
    EndProfile {
        start: u32,
        band: usize,
        phase: [PhaseEndpoint; 2],
    },
    WallRail {
        start: u32,
        band: usize,
        phase: PhaseEndpoint,
        active: bool,
    },
    ConeRail {
        start: u32,
        band: usize,
        phase: PhaseEndpoint,
    },
    Circle {
        radius_mm: f64,
        z_mm: f64,
    },
    NeckSeam {
        radius_mm: f64,
        stations_mm: [f64; 2],
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeUse {
    pub edge: usize,
    pub forward: bool,
    /// Whole angular-turn lift of this use relative to its shared curve.
    /// This preserves both periodic seam parameter branches explicitly.
    pub angular_turns: i32,
}
#[derive(Clone, Debug)]
pub struct BoundaryEdge {
    vertices: [usize; 2],
    source: SourceCurve,
    periodic_seam: bool,
}
impl BoundaryEdge {
    pub fn vertices(&self) -> [usize; 2] {
        self.vertices
    }
    pub fn source(&self) -> SourceCurve {
        self.source
    }
    pub fn is_periodic_seam(&self) -> bool {
        self.periodic_seam
    }
}
#[derive(Clone, Debug)]
pub struct BoundaryFace {
    role: Face,
    loops: Vec<Vec<EdgeUse>>,
}
impl BoundaryFace {
    pub fn role(&self) -> Face {
        self.role
    }
    /// First loop is exterior; subsequent loops bound actual material holes.
    pub fn loops(&self) -> &[Vec<EdgeUse>] {
        &self.loops
    }
}
/// Sealed, directly constructed source incidence and numerical endpoint proof.
#[derive(Clone, Debug)]
pub struct BoundaryTopology {
    source: Chamfer,
    vertices: Vec<PointSample>,
    edges: Vec<BoundaryEdge>,
    faces: Vec<BoundaryFace>,
    maximum_vertex_error_mm: f64,
    maximum_curve_binding_error_mm: f64,
}
// Untrusted producer output remains separate from the sealed public proof.
struct Candidate {
    source: Chamfer,
    vertices: Vec<PointSample>,
    edges: Vec<BoundaryEdge>,
    faces: Vec<BoundaryFace>,
    maximum_vertex_error_mm: f64,
    maximum_curve_binding_error_mm: f64,
}
impl BoundaryTopology {
    pub fn source(&self) -> &Chamfer {
        &self.source
    }
    pub fn vertices(&self) -> &[PointSample] {
        &self.vertices
    }
    pub fn edges(&self) -> &[BoundaryEdge] {
        &self.edges
    }
    pub fn faces(&self) -> &[BoundaryFace] {
        &self.faces
    }
    pub fn maximum_vertex_error_mm(&self) -> f64 {
        self.maximum_vertex_error_mm
    }
    pub fn maximum_curve_binding_error_mm(&self) -> f64 {
        self.maximum_curve_binding_error_mm
    }
    /// Combined endpoint reconstruction and whole shared-curve binding error.
    /// The constructor proves this does not exceed its requested tolerance.
    pub fn maximum_source_error_mm(&self) -> f64 {
        (I::point(self.maximum_vertex_error_mm) + I::point(self.maximum_curve_binding_error_mm)).hi
    }

    /// Enclose the actual retained analytic curve over a fractional range.
    /// A wide range returns a conservative box, not a fitted substitute.
    pub fn curve_bounds(&self, edge: usize, fraction: [f64; 2]) -> Result<[Enclosure; 3], Error> {
        let edge = self.edges.get(edge).ok_or(Error::Invalid)?;
        Ok(curve::bounds(&self.source, edge.source, fraction)?.map(enclosure))
    }
    pub fn curve_sample(&self, edge: usize, fraction: f64) -> Result<PointSample, Error> {
        let edge = self.edges.get(edge).ok_or(Error::Invalid)?;
        sample(curve::bounds(&self.source, edge.source, [fraction; 2])?)
    }
}
fn enclosure(v: I) -> Enclosure {
    Enclosure {
        lower: v.lo,
        upper: v.hi,
    }
}
fn sample(coordinates: [I; 3]) -> Result<PointSample, Error> {
    for v in coordinates {
        v.checked()?;
    }
    let nominal = coordinates.map(|v| v.lo.midpoint(v.hi));
    let mut error = I::point(0.);
    for (v, p) in coordinates.into_iter().zip(nominal) {
        error = error + I::point((v - I::point(p)).magnitude());
    }
    Ok(PointSample {
        nominal_mm: nominal,
        maximum_error_mm: error.checked()?.hi,
        coordinates_mm: coordinates.map(enclosure),
    })
}
impl Chamfer {
    pub fn boundary_topology(&self, options: QueryOptions) -> Result<BoundaryTopology, Error> {
        self.boundary_topology_with_interrupt(options, None)
    }
    pub fn boundary_topology_with_interrupt(
        &self,
        options: QueryOptions,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<BoundaryTopology, Error> {
        let mut work = Work::new(options, interrupt)?;
        builder::build(*self, options.tolerance, &mut work)
    }
}
#[cfg(test)]
mod tests;
