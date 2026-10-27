//! Owned native integration. Only this module exposes the audited native boundary.
#![deny(unsafe_code)]

// CXX owns shape lifetimes and translates std::exception into Rust Result.
#[allow(unsafe_code)]
#[cxx::bridge(namespace = "multicad::kernel")]
mod ffi {
    #[derive(Clone, Debug, PartialEq)]
    pub struct Mesh {
        pub vertices: Vec<f64>,
        pub indices: Vec<u32>,
    }
    #[derive(Clone, Copy, Debug)]
    pub struct Point3 {
        pub x: f64,
        pub y: f64,
        pub z: f64,
    }
    pub struct Frame3 {
        pub origin: Point3,
        pub u: Point3,
        pub v: Point3,
    }
    pub struct CurveEdge {
        pub kind: u8,
        pub a: Point3,
        pub mid: Point3,
        pub b: Point3,
        pub radius: f64,
    }
    pub struct Bounds3 {
        pub min: Point3,
        pub max: Point3,
    }
    pub struct FaceInfo {
        pub index: u32,
        pub planar: bool,
        pub origin: Point3,
        pub normal: Point3,
        pub center: Point3,
        pub area: f64,
    }
    unsafe extern "C++" {
        include!("bridge.hpp");
        type Shape;
        fn extrude(width: f64, depth: f64, height: f64) -> Result<UniquePtr<Shape>>;
        fn cut(
            shape: &Shape,
            x: f64,
            width: f64,
            depth: f64,
            height: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn mesh(shape: &Shape, deflection: f64) -> Result<Mesh>;
        fn volume(shape: &Shape) -> Result<f64>;
        fn valid(shape: &Shape) -> Result<bool>;
        fn write_step(shape: &Shape, path: &str) -> Result<()>;
        fn read_step(path: &str) -> Result<UniquePtr<Shape>>;
        fn cylinder(frame: &Frame3, radius: f64, height: f64) -> Result<UniquePtr<Shape>>;
        fn profile_extrude(
            frame: &Frame3,
            edges: &[CurveEdge],
            offsets: &[u32],
            height: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn boolean_shapes(
            a: &Shape,
            b: &Shape,
            operation: u8,
            fuzzy: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn transform_shape(
            shape: &Shape,
            rotation: &[f64],
            translation: &Point3,
        ) -> Result<UniquePtr<Shape>>;
        fn bounds(shape: &Shape) -> Result<Bounds3>;
        fn faces(shape: &Shape) -> Result<Vec<FaceInfo>>;
        fn solid_count(shape: &Shape) -> Result<u32>;
        fn tessellate(
            shape: &Shape,
            deflection: f64,
            angle: f64,
            max_triangles: u32,
        ) -> Result<Mesh>;
    }
}
pub use ffi::Mesh;
pub struct Shape(cxx::UniquePtr<ffi::Shape>);
pub type Error = Box<dyn std::error::Error>;
mod geometry;
pub use geometry::*;
/// Content identity of the exact native source tree, including local patches.
pub const fn source_identity() -> &'static str {
    env!("MULTICAD_OCCT_SOURCE_HASH")
}

fn step_path(path: &std::path::Path) -> Result<&str, Error> {
    let value = path.to_str().ok_or("STEP path must be UTF-8")?;
    if value.contains('\0') {
        return Err("STEP path contains a NUL byte".into());
    }
    Ok(value)
}

fn positive(values: &[f64]) -> Result<(), Error> {
    if values
        .iter()
        .all(|v| v.is_finite() && *v > 1e-7 && *v < 1e7)
    {
        Ok(())
    } else {
        Err("Dimensions must be finite, above 1e-7 mm and below 1e7 mm".into())
    }
}
impl Shape {
    pub fn extrude(width: f64, depth: f64, height: f64) -> Result<Self, Error> {
        positive(&[width, depth, height])?;
        Ok(Self(ffi::extrude(width, depth, height)?))
    }
    pub fn cut(&self, x: f64, width: f64, depth: f64, height: f64) -> Result<Self, Error> {
        positive(&[width, depth, height])?;
        if !x.is_finite() || x.abs() >= 1e7 {
            return Err("Invalid cut placement".into());
        }
        Ok(Self(ffi::cut(&self.0, x, width, depth, height)?))
    }
    pub fn mesh(&self, deflection: f64) -> Result<Mesh, Error> {
        positive(&[deflection])?;
        Ok(ffi::mesh(&self.0, deflection)?)
    }
    pub fn volume(&self) -> Result<f64, Error> {
        Ok(ffi::volume(&self.0)?)
    }
    pub fn valid(&self) -> Result<bool, Error> {
        Ok(ffi::valid(&self.0)?)
    }
    pub fn write_step(&self, path: &std::path::Path) -> Result<(), Error> {
        ffi::write_step(&self.0, step_path(path)?)?;
        Ok(())
    }
    pub fn read_step(path: &std::path::Path) -> Result<Self, Error> {
        Ok(Self(ffi::read_step(step_path(path)?)?))
    }
}
