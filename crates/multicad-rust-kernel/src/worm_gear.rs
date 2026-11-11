//! Bounded ZA axial-straight worm and its generated worm wheel.
//!
//! The worm is the single-start right-hand helical thread of the native
//! recipe with an optional axial bore, placed with its threaded length
//! centered on local Z=0 and its phase-zero crest facing -X. The wheel is the
//! exact generating envelope of the native cutter. Every generating patch
//! keeps the wheel axial coordinate constant along its profile parameter, so
//! each axial section is a planar region: the blank disk less one tooth space
//! per sector and the optional bore. Volumes and areas integrate those exact
//! sections.

pub mod clearance;
mod interval;
mod jet;
pub mod wheel;
pub(crate) mod wheel_bounds;
pub(crate) mod wheel_fit;
mod wheel_mesh;
mod wheel_topology;
pub mod worm;

pub use wheel::{Wheel, WheelSpec};
pub use worm::{Worm, WormSpec};

pub type Point = [f64; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidDimensions,
    InvalidProfile,
    Limit,
}

pub use crate::bevel_gear::Containment;

/// Planar face data: a point on the plane and its outward unit normal.
pub type Plane = (Point, Point);

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub role: String,
    pub area_mm2: f64,
    pub center_mm: Point,
    pub plane: Option<Plane>,
}

/// One exact boundary curve sampled through its owning body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Curve {
    /// Worm helical rail at a profile break, over the threaded length.
    WormRail { rail: usize },
    /// Worm flank rim on one end plane.
    WormRim { cap: usize, rail: usize },
    /// Wheel section piece on one end plane.
    WheelProfile {
        sector: u32,
        end: usize,
        piece: usize,
    },
    /// Wheel piece junction across the face width.
    WheelJunction { sector: u32, joint: usize },
}

/// Edge geometry in local body coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Line([Point; 2]),
    /// Circle about local +Z at height `z_mm`.
    Arc {
        z_mm: f64,
        radius_mm: f64,
        start_rad: f64,
        sweep_rad: f64,
    },
    Sampled(Curve),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub faces: [u32; 2],
    pub vertices: [u32; 2],
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Topology {
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
    pub vertices_mm: Vec<Point>,
}

pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

pub(crate) fn rotate_z(p: Point, angle: f64) -> Point {
    let (s, c) = angle.sin_cos();
    [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]]
}

/// Chord count of a sampled curve for a deflection and turning budget.
pub(crate) fn chord_count(
    point: impl Fn(f64) -> Point,
    deflection: f64,
    angle: f64,
    limit: usize,
) -> Result<usize, crate::display::MeshError> {
    const FINE: usize = 64;
    let points: Vec<_> = (0..=FINE).map(|i| point(i as f64 / FINE as f64)).collect();
    let h = 1. / FINE as f64;
    let mut second = 0_f64;
    let mut turning = 0.;
    for w in points.windows(3) {
        let a = sub(w[1], w[0]);
        let b = sub(w[2], w[1]);
        second = second.max(norm(sub(b, a)) / (h * h));
        turning += norm(cross(a, b)).atan2(dot(a, b));
    }
    // Twice the sampled second derivative bounds the chord sagitta.
    let count = (2. * second / (8. * deflection))
        .sqrt()
        .ceil()
        .max((turning / angle).ceil())
        .max(1.);
    if count > limit as f64 {
        return Err(crate::display::MeshError::BudgetExceeded);
    }
    Ok(count as usize)
}

#[cfg(test)]
mod tests;
