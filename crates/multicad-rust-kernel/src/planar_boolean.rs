//! Regularized Booleans of closed convex polyhedra, clipped against analytic planes.
//! An output face retains its supporting source face. Coplanar fragments can
//! share subdivision edges, but interior faces and disconnected vertex links
//! cannot survive the closed-shell proof.
use crate::{
    display::{DisplayMesh, MeshError, Tessellation},
    manufacturing,
};
use std::collections::BTreeMap;

mod clipping;
mod difference;
mod intersection;
mod loft;
mod material;
mod shell;
mod slab;
mod verify_boundary;
mod vertical_profile;
pub use loft::ruled_loft;
pub use material::CutMaterial;
pub use slab::Slab;
pub use vertical_profile::{ProfileFace, ProfilePiece};

type Point = [f64; 3];
const MAX_COORDINATE: f64 = 1e8;
const MAX_PIECES: usize = 256;
const MAX_PIECE_FACES: usize = 128;
const MAX_FACE_VERTICES: usize = 256;
const MAX_INCIDENCES: usize = 65_536;
const MAX_WORK: usize = 24_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Unresolved,
    Limit,
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: Point,
    point: Point,
}

impl Plane {
    fn distance(self, point: Point) -> f64 {
        dot(self.normal, sub(point, self.point))
    }

    fn same_direction(self, other: Self, tolerance: f64) -> bool {
        dot(self.normal, other.normal) > 0.
            && norm(cross(self.normal, other.normal)) <= 1e-12
            && self.distance(other.point).abs() <= tolerance
    }
}

#[derive(Clone, Debug)]
pub struct ConvexPiece {
    faces: Vec<Vec<Point>>,
    planes: Vec<Plane>,
    bounds: [Point; 2],
}

impl ConvexPiece {
    /// Rebuild transformed face loops and revalidate their convex boundary.
    pub fn transformed(
        &self,
        point: impl Fn([f64; 3]) -> [f64; 3],
        tolerance_mm: f64,
    ) -> Result<Self, Error> {
        let faces = self
            .faces
            .iter()
            .map(|face| face.iter().copied().map(&point).collect())
            .collect();
        Self::new(faces, tolerance_mm)
    }

    /// Validate a closed convex polyhedron with outward-oriented planar loops.
    pub fn new(faces: Vec<Vec<Point>>, tolerance_mm: f64) -> Result<Self, Error> {
        validate_tolerance(tolerance_mm)?;
        if faces.len() > MAX_PIECE_FACES || faces.iter().any(|f| f.len() > MAX_FACE_VERTICES) {
            return Err(Error::Limit);
        }
        if faces.len() < 4
            || faces.iter().any(|face| {
                face.len() < 3
                    || face
                        .iter()
                        .flatten()
                        .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
            })
        {
            return Err(Error::Invalid);
        }
        let planes = faces
            .iter()
            .map(|face| supporting_plane(face, tolerance_mm))
            .collect::<Result<Vec<_>, _>>()?;
        let mut boundary = shell::Boundary::new(tolerance_mm);
        for (source_face, (face, &plane)) in faces.iter().zip(&planes).enumerate() {
            if faces
                .iter()
                .flatten()
                .any(|&point| plane.distance(point) > tolerance_mm)
            {
                return Err(Error::Invalid);
            }
            boundary.add(face, plane.normal, 0, source_face)?;
        }
        boundary.prove()?;
        let bounds = bounds(faces.iter().flatten().copied());
        Ok(Self {
            faces,
            planes,
            bounds,
        })
    }

    pub fn from_box(min: Point, max: Point, tolerance_mm: f64) -> Result<Self, Error> {
        if min.into_iter().zip(max).any(|(lo, hi)| lo >= hi) {
            return Err(Error::Invalid);
        }
        let [x, y, z] = min;
        let [u, v, w] = max;
        Self::new(
            vec![
                vec![[x, y, z], [x, v, z], [u, v, z], [u, y, z]],
                vec![[x, y, w], [u, y, w], [u, v, w], [x, v, w]],
                vec![[x, y, z], [u, y, z], [u, y, w], [x, y, w]],
                vec![[u, y, z], [u, v, z], [u, v, w], [u, y, w]],
                vec![[u, v, z], [x, v, z], [x, v, w], [u, v, w]],
                vec![[x, v, z], [x, y, z], [x, y, w], [x, v, w]],
            ],
            tolerance_mm,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub vertices: Vec<usize>,
    pub normal: Point,
    pub source: usize,
    pub source_face: usize,
}

/// One connected closed component of a certified planar boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct Shell {
    /// Face ordinals in the parent boundary, in ascending order.
    pub faces: Vec<usize>,
    /// Negative for an inward shell bounding a cavity.
    pub signed_volume_mm3: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Union {
    vertices: Vec<Point>,
    faces: Vec<Face>,
    bounds: [Point; 2],
    volume_mm3: f64,
    solid_count: usize,
    shells: Vec<Shell>,
}

impl Union {
    pub fn new(pieces: &[ConvexPiece], tolerance_mm: f64) -> Result<Self, Error> {
        validate_tolerance(tolerance_mm)?;
        if pieces.is_empty() {
            return Err(Error::Invalid);
        }
        if pieces.len() > MAX_PIECES {
            return Err(Error::Limit);
        }
        Self::from_boundary(clipping::exterior(pieces, tolerance_mm, MAX_WORK)?)
    }

    fn from_boundary(
        shell::Boundary {
            vertices, faces, ..
        }: shell::Boundary,
    ) -> Result<Self, Error> {
        Self::from_indexed_boundary(vertices, faces)
    }

    fn from_indexed_boundary(vertices: Vec<Point>, faces: Vec<Face>) -> Result<Self, Error> {
        let proof = shell::prove(&vertices, &faces)?;
        let bounds = bounds(vertices.iter().copied());
        Ok(Self {
            vertices,
            faces,
            bounds,
            volume_mm3: proof.volume_mm3,
            solid_count: proof.solid_count,
            shells: proof.shells,
        })
    }

    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }

    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    pub fn bounds(&self) -> [Point; 2] {
        self.bounds
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    pub fn solid_count(&self) -> usize {
        self.solid_count
    }

    pub fn shell_count(&self) -> usize {
        self.shells.len()
    }

    /// Closed boundary components in order of first face ordinal.
    pub fn shells(&self) -> &[Shell] {
        &self.shells
    }

    pub fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        options.check()?;
        shell::mesh(&self.vertices, &self.faces, options.triangle_limit)
    }
}

fn validate_tolerance(tolerance: f64) -> Result<(), Error> {
    if !(1e-10..=1e-3).contains(&tolerance) {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn bounds(points: impl Iterator<Item = Point>) -> [Point; 2] {
    points.fold(
        [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]],
        |[min, max], p| {
            [
                std::array::from_fn(|i| min[i].min(p[i])),
                std::array::from_fn(|i| max[i].max(p[i])),
            ]
        },
    )
}

fn supporting_plane(face: &[Point], tolerance: f64) -> Result<Plane, Error> {
    let point = face[0];
    let area = face.windows(2).fold([0.; 3], |sum, pair| {
        add(sum, cross(sub(pair[0], point), sub(pair[1], point)))
    });
    let length = norm(area);
    if length <= tolerance * tolerance {
        return Err(Error::Invalid);
    }
    let normal = scale(area, 1. / length);
    let plane = Plane { normal, point };
    if face.iter().any(|&p| plane.distance(p).abs() > tolerance) {
        return Err(Error::Invalid);
    }
    for (&start, &end) in face.iter().zip(face.iter().cycle().skip(1)) {
        let tangent = sub(end, start);
        let length = norm(tangent);
        if length <= tolerance {
            return Err(Error::Invalid);
        }
        let inward = scale(cross(normal, tangent), 1. / length);
        if face
            .iter()
            .any(|&p| dot(inward, sub(p, start)) < -tolerance)
        {
            return Err(Error::Invalid);
        }
    }
    Ok(plane)
}

fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn scale(a: Point, s: f64) -> Point {
    a.map(|v| v * s)
}

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

fn cross([x, y, z]: Point, [u, v, w]: Point) -> Point {
    [y * w - z * v, z * u - x * w, x * v - y * u]
}

fn charge(budget: &mut usize, cost: usize) -> Result<(), Error> {
    *budget = budget.checked_sub(cost).ok_or(Error::Limit)?;
    Ok(())
}

#[cfg(test)]
mod tests;
