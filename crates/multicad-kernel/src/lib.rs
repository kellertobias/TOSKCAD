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
    pub struct SweepLimits {
        tolerance: f64,
        minimum_feature: f64,
        maximum_length: f64,
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
        fn minimum_distance(
            a: &Shape,
            b: &Shape,
            options: &DistanceOptionsData,
        ) -> Result<DistanceData>;
        include!("planar_contours.hpp");
