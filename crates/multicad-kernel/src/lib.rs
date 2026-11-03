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
        /// One evaluation-local B-rep face index per display triangle.
        pub face_indices: Vec<u32>,
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
        pub source_index: u32,
    }
    pub struct Bounds3 {
        pub min: Point3,
        pub max: Point3,
    }
    /// One solid read from a STEP file: its representation item name, if
    /// any, and its exact located B-rep as OCCT BRep text.
    pub struct StepSolidData {
        pub name: String,
        pub brep: String,
    }
    pub struct SweepLimits {
        tolerance: f64,
        minimum_feature: f64,
        maximum_length: f64,
    }
    #[derive(Clone, Copy, Debug)]
    pub struct ThreadData {
        radius: f64,
        pitch: f64,
        height: f64,
        depth: f64,
        crest_width: f64,
        rounding: f64,
        lower_slope: f64,
        upper_slope: f64,
        taper: f64,
        overlap: f64,
        tolerance: f64,
        starts: u32,
        left: bool,
    }
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct ThreadFactoryStats {
        pub requests: u64,
        pub hits: u64,
        pub constructions: u64,
        pub continuous_attempts: u64,
        pub bounded_attempts: u64,
        pub entries: u64,
        pub serialized_bytes: u64,
        pub evictions: u64,
        pub unretained: u64,
    }
    pub struct GearData {
        teeth: u32,
        pitch_radius: f64,
        base_radius: f64,
        tip_radius: f64,
        root_radius: f64,
        fillet_radius: f64,
        half_base: f64,
        height: f64,
        bore: f64,
        twist: f64,
        gap: f64,
        style: u8,
        tolerance: f64,
    }
    pub struct FaceInfo {
        pub index: u32,
        pub planar: bool,
        pub origin: Point3,
        pub normal: Point3,
        pub center: Point3,
        pub area: f64,
    }
    pub struct TopologyNode {
        pub kind: u8,
        pub index: u32,
        pub incident_faces: Vec<u32>,
    }
    pub struct Ancestry {
        pub kind: u8,
        pub operand: u8,
        pub output: u32,
        pub input: u32,
    }
    pub struct Seed {
        pub face: u32,
        pub source_index: u32,
        pub role: u8,
    }
    pub struct EdgeInfo {
        pub index: u32,
        pub points: Vec<f64>,
    }
    pub struct VertexInfo {
        pub index: u32,
        pub point: Point3,
    }
    pub struct AnalyticEdgeData {
        pub index: u32,
        pub kind: u8,
        pub a: Point3,
        pub b: Point3,
        pub frame: Frame3,
        pub radius: f64,
        pub start: f64,
        pub sweep: f64,
    }
    pub struct SplineData {
        pub degree: u32,
        pub poles: Vec<f64>,
        pub knots: Vec<f64>,
        pub multiplicities: Vec<u32>,
        pub weights: Vec<f64>,
    }
    pub struct DistanceOptionsData {
        pub a_kind: u8,
        pub a_index: u32,
        pub b_kind: u8,
        pub b_index: u32,
        pub tolerance: f64,
        pub maximum: u32,
    }
    pub struct DistanceWitnessData {
        pub a: Point3,
        pub b: Point3,
    }
    pub struct DistanceData {
        pub distance: f64,
        pub total_solutions: u32,
        pub inner: bool,
        pub complete: bool,
        pub witnesses: Vec<DistanceWitnessData>,
    }
    pub struct DrawingOptionsData {
        pub frame: Frame3,
        pub section: bool,
        pub section_offset: f64,
        pub section_only: bool,
        pub depth: bool,
        pub depth_mm: f64,
        pub window_u: bool,
        pub u_min: f64,
        pub u_max: f64,
        pub window_v: bool,
        pub v_min: f64,
        pub v_max: f64,
        pub detail: bool,
        pub detail_center: Point3,
        pub detail_radius: f64,
        pub tolerance: f64,
        pub hidden: bool,
        pub tangents: bool,
        pub max_curves: u32,
        pub max_points: u32,
    }
    pub struct DrawingCurveData {
        pub source_index: u32,
        pub role: u8,
        pub kind: u8,
        pub degree: u32,
        pub periodic: bool,
        pub data: Vec<f64>,
        pub knots: Vec<f64>,
        pub multiplicities: Vec<u32>,
        pub preview: Vec<f64>,
    }
    pub struct PlanarSegmentData {
        pub edge_index: u32,
        pub kind: u8,
        pub a: Point3,
        pub b: Point3,
        pub center: Point3,
        pub sweep: f64,
    }
    pub struct PlanarWireData {
        pub outer: bool,
        pub segments: Vec<PlanarSegmentData>,
    }
    pub struct HoleAxisData {
        pub origin: Point3,
        pub axis: Point3,
        pub radius: f64,
    }
    pub struct CurvedEdgeData {
        pub kind: u8,
        pub a: Point3,
        pub b: Point3,
        pub center: Point3,
        pub radius: f64,
        pub clockwise: bool,
        pub spline: SplineData,
        pub source_index: u32,
    }
    pub struct CurveSampleData {
        pub point: Point3,
        pub derivative: Point3,
    }
    pub struct CurveProjectionData {
        pub point: Point3,
        pub tangent: Point3,
        pub parameter: f64,
        pub distance: f64,
    }
    pub struct SketchSupportData {
        pub kind: u8,
        pub index: u32,
    }
    pub struct SketchReferenceData {
        pub geometry: u8,
        pub point: Point3,
        pub curve: CurvedEdgeData,
        pub support: Vec<SketchSupportData>,
        pub has_parameter: bool,
        pub parameter: f64,
        pub has_interval: bool,
        pub first: f64,
        pub last: f64,
        pub tangent: bool,
        pub crossing: i8,
    }
    pub struct SketchReferenceCatalogData {
        pub results: Vec<SketchReferenceData>,
        pub coincident: bool,
    }
    pub struct LoftSectionData {
        pub frame: Frame3,
        pub edges: Vec<CurvedEdgeData>,
    }
    pub struct CurveIntersectionData {
        pub point: Point3,
        pub first: f64,
        pub second: f64,
    }
    pub struct CurveQueryData {
        pub points: Vec<CurveIntersectionData>,
        pub overlaps: u32,
    }
    unsafe extern "C++" {
        include!("bridge.hpp");
        include!("curves.hpp");
        include!("analytic.hpp");
        include!("drawing.hpp");
        include!("distance.hpp");
        include!("planar_contours.hpp");
        include!("advanced.hpp");
        include!("sketch_references.hpp");
        include!("threads.hpp");
        fn helical_thread(spec: &ThreadData) -> Result<UniquePtr<Shape>>;
        include!("gears.hpp");
        fn involute_gear(spec: &GearData) -> Result<UniquePtr<Shape>>;
        fn sketch_reference_geometry(
            shape: &Shape,
            kind: u8,
            index: u32,
            frame: &Frame3,
            method: u8,
            tolerance: f64,
        ) -> Result<SketchReferenceCatalogData>;
        type CompoundJob;
        fn compound_job() -> Result<UniquePtr<CompoundJob>>;
        fn compound_add(job: Pin<&mut CompoundJob>, shape: &Shape) -> Result<()>;
        fn compound_finish(job: Pin<&mut CompoundJob>) -> Result<UniquePtr<Shape>>;
        fn solid_component_at(shape: &Shape, index: u32) -> Result<UniquePtr<Shape>>;
        fn shell_solid(
            shape: &Shape,
            faces: &[u32],
            thickness: f64,
            tolerance: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn draft_solid(
            shape: &Shape,
            faces: &[u32],
            direction: &Point3,
            neutral: &Frame3,
            angle: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn minimum_distance(
            a: &Shape,
            b: &Shape,
            options: &DistanceOptionsData,
        ) -> Result<DistanceData>;
        type Shape;
        type DrawingJob;
        fn drawing_job(options: &DrawingOptionsData) -> Result<UniquePtr<DrawingJob>>;
        fn drawing_add(
            job: Pin<&mut DrawingJob>,
            shape: &Shape,
            hatch: bool,
            cross: bool,
            angle: f64,
            spacing: f64,
        ) -> Result<()>;
        fn planar_face_contours(
            shape: &Shape,
            index: u32,
            frame: &Frame3,
            tolerance: f64,
        ) -> Result<Vec<PlanarWireData>>;
        fn transfer_hole_axis(
            shape: &Shape,
            kind: u8,
            index: u32,
            tolerance: f64,
        ) -> Result<HoleAxisData>;
        fn drawing_run(job: Pin<&mut DrawingJob>) -> Result<Vec<DrawingCurveData>>;
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
        #[allow(dead_code)] // Private immutable-property cache regressions.
        fn volume_integration_count(shape: &Shape) -> Result<u64>;
        #[allow(dead_code)]
        fn shares_volume_cache(a: &Shape, b: &Shape) -> bool;
        #[allow(dead_code)]
        fn volume_cache_concurrent_probe(shape: &Shape) -> Result<bool>;
        #[allow(dead_code)] // Private completed face-property cache regressions.
        fn face_integration_count(shape: &Shape) -> Result<u64>;
        #[allow(dead_code)]
        fn shares_face_cache(a: &Shape, b: &Shape) -> bool;
        #[allow(dead_code)]
        fn face_cache_concurrent_probe(shape: &Shape) -> Result<bool>;
        fn thread_factory_stats() -> Result<ThreadFactoryStats>;
        fn remember_validated_thread(shape: &Shape) -> Result<bool>;
        #[allow(dead_code)] // Private bounded factory lifetime/invalidation tests.
        fn reset_thread_factory_cache() -> Result<()>;
        #[allow(dead_code)]
        fn thread_factory_test_limits(entries: u32, bytes: u64) -> Result<()>;
        #[allow(dead_code)]
        fn thread_factory_contains(data: &ThreadData) -> Result<bool>;
        fn valid(shape: &Shape) -> Result<bool>;
        fn write_step(shape: &Shape, path: &str) -> Result<()>;
        fn read_step(path: &str) -> Result<UniquePtr<Shape>>;
        fn step_solids(path: &str) -> Result<Vec<StepSolidData>>;
        fn shape_to_brep(shape: &Shape) -> Result<String>;
        fn shape_from_brep(text: &str) -> Result<UniquePtr<Shape>>;
        fn route_native_output_to_stderr();
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
        fn clip_half_space(
            shape: &Shape,
            plane: &Frame3,
            positive: bool,
            fuzzy: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn inspection_union(shape: &Shape) -> Result<UniquePtr<Shape>>;
        fn share_shape(shape: &Shape) -> Result<UniquePtr<Shape>>;
        fn transform_shape(
            shape: &Shape,
            rotation: &[f64],
            translation: &Point3,
        ) -> Result<UniquePtr<Shape>>;
        fn affine_transform_shape(
            shape: &Shape,
            matrix: &[f64],
            translation: &Point3,
        ) -> Result<UniquePtr<Shape>>;
        fn mirror_shape(shape: &Shape, plane: &Frame3) -> Result<UniquePtr<Shape>>;
        fn treat_edges(
            shape: &Shape,
            edges: &[u32],
            mode: u8,
            size: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn treat_edges_sided(
            shape: &Shape,
            edges: &[u32],
            sides: &[u32],
            mode: u8,
            first: f64,
            second: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn bounds(shape: &Shape) -> Result<Bounds3>;
        #[allow(dead_code)] // Called only by the native cache concurrency unit test.
        fn bounds_cache_concurrent_probe(shape: &Shape) -> Result<bool>;
        fn faces(shape: &Shape) -> Result<Vec<FaceInfo>>;
        fn topology(shape: &Shape) -> Result<Vec<TopologyNode>>;
        fn ancestry(shape: &Shape) -> Result<Vec<Ancestry>>;
        fn seeds(shape: &Shape) -> Result<Vec<Seed>>;
        fn display_edges(shape: &Shape, deflection: f64) -> Result<Vec<EdgeInfo>>;
        fn vertices(shape: &Shape) -> Result<Vec<VertexInfo>>;
        fn analytic_edges(shape: &Shape) -> Result<Vec<AnalyticEdgeData>>;
        fn solid_count(shape: &Shape) -> Result<u32>;
        fn curve_evaluate(curve: &CurvedEdgeData, parameter: f64) -> Result<CurveSampleData>;
        fn curve_project(
            curve: &CurvedEdgeData,
            point: &Point3,
            tolerance: f64,
        ) -> Result<Vec<CurveProjectionData>>;
        fn spline_segment(curve: &CurvedEdgeData, from: f64, to: f64) -> Result<SplineData>;
        fn curve_intersections(
            first: &CurvedEdgeData,
            second: &CurvedEdgeData,
            tolerance: f64,
            self_query: bool,
        ) -> Result<CurveQueryData>;
        fn curved_profile_area(
            frame: &Frame3,
            edges: &[CurvedEdgeData],
            offsets: &[u32],
            tolerance: f64,
        ) -> Result<f64>;
        fn curved_profile_signed_area(
            frame: &Frame3,
            edges: &[CurvedEdgeData],
            offsets: &[u32],
            tolerance: f64,
        ) -> Result<f64>;
        fn curved_profile_classify(
            frame: &Frame3,
            edges: &[CurvedEdgeData],
            offsets: &[u32],
            tolerance: f64,
            point: &Point3,
        ) -> Result<u8>;
        fn curved_profile_extrude_vector(
            frame: &Frame3,
            edges: &[CurvedEdgeData],
            offsets: &[u32],
            tolerance: f64,
            displacement: &Point3,
        ) -> Result<UniquePtr<Shape>>;
        fn curved_profile_revolve(
            frame: &Frame3,
            edges: &[CurvedEdgeData],
            offsets: &[u32],
            tolerance: f64,
            axis_origin: &Point3,
            axis_direction: &Point3,
            angle: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn curved_profile_to_plane(
            frame: &Frame3,
            edges: &[CurvedEdgeData],
            offsets: &[u32],
            limits: &SweepLimits,
            direction: &Point3,
            termination: &Frame3,
        ) -> Result<UniquePtr<Shape>>;
        fn curved_profile_ruled(
            start: &Frame3,
            end: &Frame3,
            start_edges: &[CurvedEdgeData],
            end_edges: &[CurvedEdgeData],
            offsets: &[u32],
            tolerance: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn curved_profile_pipe(
            frame: &Frame3,
            edges: &[CurvedEdgeData],
            path: &[Point3],
            tolerance: f64,
        ) -> Result<UniquePtr<Shape>>;
        fn curved_profile_loft(
            sections: &[LoftSectionData],
            ruled: bool,
            tolerance: f64,
        ) -> Result<UniquePtr<Shape>>;
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
mod distance;
pub use distance::{BRepSelection, DistanceWitness, MinimumDistance};
mod interference;
pub use interference::{SolidPairInspection, SolidPairRelation};
mod geometry;
pub use geometry::*;
mod affine;
pub use affine::AffineTransform;
mod advanced;
pub use advanced::TreatmentFace;
mod advanced_operations;
mod advanced_profiles;
mod threads;
pub use threads::{HelicalThread, RoundedHelicalThread, ThreadFactoryStats, thread_factory_stats};
mod gears;
pub use gears::{GearSweep, InvoluteGear};
mod edge_treatment;
pub use edge_treatment::{EdgeTreatment, SidedChamfer, SidedTreatmentEdge, TreatmentEdge};
mod drawing;
pub use drawing::*;
mod worker;
pub use worker::*;
mod curves;
mod sketch_queries;
pub use sketch_queries::NativeRegionQueries;
mod provenance;
mod revolve;
mod split;
mod taper;
mod to_plane;
pub use curves::*;
pub use revolve::RevolutionAxis;
pub use split::{PlaneSplit, SolidSplit};
/// Content identity of the exact native source tree, including local patches.
pub const fn source_identity() -> &'static str {
    env!("MULTICAD_OCCT_SOURCE_HASH")
}

/// Send OCCT messages and C++ `std::cout` to stderr, at warning level and
/// above. Required when stdout carries a protocol, such as `multicad --mcp`;
/// Rust's own stdout is unaffected. Idempotent and process-wide.
pub fn route_native_output_to_stderr() {
    ffi::route_native_output_to_stderr();
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

mod planar_contours;
pub use planar_contours::SourcedPlanarContours;
mod transferred_cuts;
pub use transferred_cuts::TransferHoleAxis;
mod spline_edits;
pub use spline_edits::*;

mod sketch_references;
pub use sketch_references::*;
