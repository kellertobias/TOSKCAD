//! Finite polynomial boundary data and its independently checked ownership.
//! A manifold boundary is not yet a certified occupied radial solid.
pub use crate::tapered_thread::Error;
use crate::tapered_thread::{RetainedTensor, interval::I};
mod authored;
pub use authored::AuthoredBoundary;
mod axial;
mod topology;
pub use axial::AxialBoundary;
mod caps;
mod material;
mod ordering;
mod properties;
mod query;
pub use caps::CapBoundary;
pub use material::RadialBoundary;
pub use ordering::OrderedBoundary;
pub use properties::{FaceProperties, MassProperties};
pub use query::{Collars, Location};
mod trims;
pub use trims::TrimmedBoundary;
#[cfg(test)]
pub(crate) mod tests;
type Point = [f64; 3];
const MAX_CONTROLS: usize = 1_048_576;
struct Work<'a> {
    remaining: usize,
    interrupt: Option<&'a dyn Fn() -> bool>,
}
impl Work<'_> {
    fn step(&mut self) -> Result<(), Error> {
        self.charge(1)
    }
    fn charge(&mut self, amount: usize) -> Result<(), Error> {
        if self.interrupt.is_some_and(|f| f()) {
            return Err(Error::Cancelled);
        }
        self.remaining = self.remaining.checked_sub(amount).ok_or(Error::Limit)?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub origin: Point,
    pub normal: Point,
    pub reference: Point,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Spline { spans: usize, poles: Vec<Point> },
    Circle { axis: Axis, radius_mm: f64 },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    Tensor(RetainedTensor),
    Plane(Axis),
    Cone {
        axis: Axis,
        radius_mm: f64,
        semi_angle_rad: f64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub vertices: [usize; 2],
    pub curve: usize,
    pub forward: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub surface: usize,
    pub outer: Vec<(usize, bool)>,
    pub forward: bool,
}
/// Uncertified source values. Construction never changes their coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryInput {
    pub vertices: Vec<Point>,
    pub curves: Vec<Curve>,
    pub edges: Vec<Edge>,
    pub surfaces: Vec<Surface>,
    pub faces: Vec<Face>,
}
#[derive(Clone)]
pub struct BoundaryOwnership {
    input: BoundaryInput,
    topology: topology::Topology,
    sewing_error_mm: f64,
}
impl BoundaryOwnership {
    /// Certify finite data, curve endpoints and one closed oriented abstract
    /// manifold. Face trims and occupied material need separate proofs.
    pub fn new(
        input: BoundaryInput,
        tolerance: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, Error> {
        if !tolerance.is_finite() || tolerance <= 0. {
            return Err(Error::Invalid);
        }
        let mut work = Work {
            remaining: 2_000_000,
            interrupt,
        };
        work.step()?;
        if input.vertices.len() > 80
            || input.edges.len() > 112
            || input.faces.len() > 34
            || input.curves.len() > 112
            || input.surfaces.len() > 34
        {
            return Err(Error::Limit);
        }
        for face in &input.faces {
            if face.outer.len() > 80 {
                return Err(Error::Limit);
            }
        }
        let controls = input
            .surfaces
            .iter()
            .try_fold(0usize, |s, face| {
                s.checked_add(match face {
                    Surface::Tensor(t) => t.poles().len(),
                    _ => 0,
                })
            })
            .ok_or(Error::Limit)?;
        let controls = input
            .curves
            .iter()
            .try_fold(controls, |s, c| {
                s.checked_add(match c {
                    Curve::Spline { poles, .. } => poles.len(),
                    _ => 0,
                })
            })
            .ok_or(Error::Limit)?;
        if controls > MAX_CONTROLS || input.curves.len() > 112 || input.surfaces.len() > 34 {
            return Err(Error::Limit);
        }
        for &point in &input.vertices {
            work.step()?;
            checked_point(point)?;
        }
        for surface in &input.surfaces {
            work.step()?;
            match surface {
                Surface::Tensor(_) => {}
                Surface::Plane(a) => {
                    checked_axis(*a)?;
                }
                Surface::Cone {
                    axis,
                    radius_mm,
                    semi_angle_rad,
                } => {
                    checked_axis(*axis)?;
                    if !radius_mm.is_finite()
                        || *radius_mm <= 0.
                        || *radius_mm > 1e7
                        || !semi_angle_rad.is_finite()
                        || !(0. ..std::f64::consts::FRAC_PI_2).contains(semi_angle_rad)
                    {
                        return Err(Error::Invalid);
                    }
                }
            }
        }
        for curve in &input.curves {
            work.step()?;
            match curve {
                Curve::Spline { spans, poles } => {
                    if *spans == 0 {
                        return Err(Error::Invalid);
                    }
                    let count = 9usize
                        .checked_mul(*spans)
                        .and_then(|n| n.checked_add(1))
                        .ok_or(Error::Limit)?;
                    if count != poles.len() {
                        return Err(Error::Invalid);
                    }
                    for chunk in poles.chunks(128) {
                        work.step()?;
                        for &point in chunk {
                            checked_point(point)?;
                        }
                    }
                }
                Curve::Circle { axis, radius_mm } => {
                    checked_axis(*axis)?;
                    if !radius_mm.is_finite() || *radius_mm <= 0. || *radius_mm > 1e7 {
                        return Err(Error::Invalid);
                    }
                }
            }
        }
        let mut sewing_error = 0f64;
        for edge in &input.edges {
            work.step()?;
            let [a, b] = edge
                .vertices
                .map(|i| input.vertices.get(i).copied().ok_or(Error::Invalid));
            let [a, b] = [a?, b?];
            if a == b {
                return Err(Error::Invalid);
            }
            match input.curves.get(edge.curve).ok_or(Error::Invalid)? {
                Curve::Spline { poles, .. } => {
                    let ends = if edge.forward {
                        [poles[0], poles[poles.len() - 1]]
                    } else {
                        [poles[poles.len() - 1], poles[0]]
                    };
                    if ends != [a, b] {
                        return Err(Error::Invalid);
                    }
                }
                Curve::Circle { axis, radius_mm } => {
                    for point in [a, b] {
                        let error = circle_error(point, *axis, *radius_mm)?;
                        if error > tolerance / 16. {
                            return Err(Error::Unresolved);
                        }
                        sewing_error = sewing_error.max(error);
                    }
                }
            }
        }
        for face in &input.faces {
            work.step()?;
            if face.surface >= input.surfaces.len() {
                return Err(Error::Invalid);
            }
        }
        let edges = input.edges.iter().map(|e| e.vertices).collect::<Vec<_>>();
        let loops = input
            .faces
            .iter()
            .map(|f| f.outer.clone())
            .collect::<Vec<_>>();
        let topology = topology::certify(input.vertices.len(), &edges, &loops, &mut work)?;
        Ok(Self {
            input,
            topology,
            sewing_error_mm: sewing_error,
        })
    }
    pub fn input(&self) -> &BoundaryInput {
        &self.input
    }
    pub fn sewing_error_mm(&self) -> f64 {
        self.sewing_error_mm
    }
    pub fn edge_faces(&self) -> &[[usize; 2]] {
        &self.topology.edge_faces
    }
    pub fn vertex_edges(&self) -> &[Vec<usize>] {
        &self.topology.vertex_edges
    }
}
fn checked_point(p: Point) -> Result<(), Error> {
    if p.into_iter().all(|v| v.is_finite() && v.abs() <= 1e7) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}
fn dot(a: [I; 3], b: [I; 3]) -> I {
    a.into_iter()
        .zip(b)
        .fold(I::point(0.), |s, (a, b)| s + a * b)
}
fn cross(a: [I; 3], b: [I; 3]) -> [I; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(v: [I; 3]) -> Result<[I; 3], Error> {
    let length = dot(v, v).sqrt().positive()?;
    Ok(v.map(|v| v / length))
}
fn checked_axis(axis: Axis) -> Result<[[I; 3]; 3], Error> {
    checked_point(axis.origin)?;
    for v in axis.normal.into_iter().chain(axis.reference) {
        if !v.is_finite() {
            return Err(Error::Invalid);
        }
    }
    let normal = unit(axis.normal.map(I::point))?;
    let reference = axis.reference.map(I::point);
    let along = dot(reference, normal);
    let u = unit(std::array::from_fn(|i| reference[i] - normal[i] * along))?;
    Ok([u, cross(normal, u), normal])
}
fn circle_error(point: Point, axis: Axis, radius: f64) -> Result<f64, Error> {
    let normal = checked_axis(axis)?[2];
    let delta = std::array::from_fn(|i| I::point(point[i]) - I::point(axis.origin[i]));
    let height = dot(delta, normal);
    let radial = std::array::from_fn(|i| delta[i] - normal[i] * height);
    let radius_error = dot(radial, radial).sqrt() - I::point(radius);
    let error = (I::point(height.magnitude()) + I::point(radius_error.magnitude())).checked()?;
    Ok(error.hi)
}
