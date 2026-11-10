//! Safe solid operations in millimeters. Display meshes are detached copies;
//! callers rebuild exact solids from source rather than editing triangles.
use crate::{Shape, ffi, rust_prism_topology::PlacedPolygon};
use multicad_core::{
    document::EntityId,
    references::{Origin, TopologySnapshot},
};
use multicad_rust_kernel::{
    Circle, Rational, cylinder::AxialCylinder, polygon::PolygonPrism, prism::Prism,
};
use std::path::Path;
use std::rc::Rc;
mod annulus_body;
pub(crate) mod bore_treatment;
pub(crate) mod circular_pocket_body;
mod cosmetic_fastener_body;
mod helical_body;
mod linear_body;
mod orthogonal_body;
mod revolution_body;
mod rounded_slot_body;
mod rust_compound;
mod step_import;
use annulus_body::AnnulusBody;
pub use circular_pocket_body::CircularPocketTool;
use cosmetic_fastener_body::CosmeticFastenerBody;
use helical_body::HelicalBody;
use linear_body::LinearBody;
use orthogonal_body::OrthogonalBody;
pub use orthogonal_body::slots::RoundedSlotTool;
use revolution_body::RevolutionBody;
use rounded_slot_body::RoundedSlotBody;
use rust_compound::RustCompound;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryErrorKind {
    Input,
    Native,
    InvalidResult,
    Limit,
    Cancelled,
    Stale,
    Busy,
    Unsupported,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeometryError {
    pub operation: &'static str,
    pub kind: GeometryErrorKind,
    pub message: String,
}
impl std::fmt::Display for GeometryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.operation, self.message)
    }
}
impl std::error::Error for GeometryError {}
fn error(
    operation: &'static str,
    kind: GeometryErrorKind,
    message: impl Into<String>,
) -> GeometryError {
    GeometryError {
        operation,
        kind,
        message: message.into(),
    }
}
fn input(message: impl Into<String>) -> GeometryError {
    error("validate", GeometryErrorKind::Input, message)
}
fn native(operation: &'static str, e: impl std::fmt::Display) -> GeometryError {
    error(operation, GeometryErrorKind::Native, e.to_string())
}

fn rational_coordinate(value: f64, tolerance: f64) -> Option<Rational> {
    // Use the first continued-fraction convergent inside the error budget
    // so floating-point projection noise does not inflate denominators.
    let mut remainder = value.abs();
    let mut previous = [0_i128, 1];
    let mut current = [1_i128, 0];
    for _ in 0..32 {
        if !remainder.is_finite() || remainder > i64::MAX as f64 {
            break;
        }
        let whole = remainder.floor();
        let next = std::array::from_fn(|i| whole as i128 * current[i] + previous[i]);
        let [Ok(n), Ok(d)] = next.map(i64::try_from) else {
            break;
        };
        let Ok(candidate) = Rational::new(if value.is_sign_negative() { -n } else { n }, d) else {
            break;
        };
        if (candidate.as_f64() - value).abs() <= tolerance / 32. {
            return Some(candidate);
        }
        previous = current;
        current = next;
        remainder = 1. / (remainder - whole);
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeometryPolicy {
    pub linear_tolerance_mm: f64,
    /// Additional Boolean fuzz is explicit. Zero requests no added gap closing.
    pub boolean_fuzzy_mm: f64,
    pub minimum_feature_mm: f64,
    pub maximum_coordinate_mm: f64,
}
impl Default for GeometryPolicy {
    fn default() -> Self {
        Self {
            linear_tolerance_mm: 1e-7,
            boolean_fuzzy_mm: 0.,
            minimum_feature_mm: 1e-5,
            maximum_coordinate_mm: 1e7,
        }
    }
}
impl GeometryPolicy {
    pub fn validate(self) -> Result<(), GeometryError> {
        if ![
            self.linear_tolerance_mm,
            self.boolean_fuzzy_mm,
            self.minimum_feature_mm,
            self.maximum_coordinate_mm,
        ]
        .iter()
        .all(|v| v.is_finite())
            || !(1e-7..=1e-3).contains(&self.linear_tolerance_mm)
            || !(0. ..=self.linear_tolerance_mm).contains(&self.boolean_fuzzy_mm)
            || self.minimum_feature_mm < self.linear_tolerance_mm * 10.
            || self.maximum_coordinate_mm <= self.minimum_feature_mm
            || self.maximum_coordinate_mm > 1e7
        {
            return Err(input(
                "Invalid numerical policy: tolerance, fuzz, feature size or model bounds",
            ));
        }
        Ok(())
    }
    pub(crate) fn point(self, p: [f64; 3]) -> Result<(), GeometryError> {
        if p.iter()
            .any(|v| !v.is_finite() || v.abs() > self.maximum_coordinate_mm)
        {
            Err(input("Coordinate is nonfinite or outside model bounds"))
        } else {
            Ok(())
        }
    }
    pub(crate) fn length(self, v: f64) -> Result<(), GeometryError> {
        if !v.is_finite() || v < self.minimum_feature_mm || v > self.maximum_coordinate_mm {
            Err(input(
                "Length is below minimum feature size or outside model bounds",
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneFrame {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
}
impl Default for PlaneFrame {
    fn default() -> Self {
        Self {
            origin: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        }
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn point(p: [f64; 3]) -> ffi::Point3 {
    ffi::Point3 {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}
fn xyz(p: ffi::Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}
impl PlaneFrame {
    pub fn normal(self) -> [f64; 3] {
        cross(self.u, self.v)
    }
    pub fn point(self, p: [f64; 2]) -> [f64; 3] {
        std::array::from_fn(|i| self.origin[i] + self.u[i] * p[0] + self.v[i] * p[1])
    }
    pub(crate) fn checked(self, policy: GeometryPolicy) -> Result<ffi::Frame3, GeometryError> {
        policy.point(self.origin)?;
        if self.u.iter().chain(&self.v).any(|v| !v.is_finite())
            || (dot(self.u, self.u) - 1.).abs() > 1e-10
            || (dot(self.v, self.v) - 1.).abs() > 1e-10
            || dot(self.u, self.v).abs() > 1e-10
        {
            return Err(input("Plane axes must be finite and orthonormal"));
        }
        Ok(ffi::Frame3 {
            origin: point(self.origin),
            u: point(self.u),
            v: point(self.v),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidTransform {
    pub rotation: [[f64; 3]; 3],
    pub translation_mm: [f64; 3],
}
impl Default for RigidTransform {
    fn default() -> Self {
        Self {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [0.; 3],
        }
    }
}
impl RigidTransform {
    fn vector(self, vector: [f64; 3]) -> [f64; 3] {
        self.rotation.map(|row| dot(row, vector))
    }
    fn frame(self, frame: PlaneFrame) -> PlaneFrame {
        let origin = self.vector(frame.origin);
        PlaneFrame {
            origin: std::array::from_fn(|i| origin[i] + self.translation_mm[i]),
            u: self.vector(frame.u),
            v: self.vector(frame.v),
        }
    }
    fn validate(self, policy: GeometryPolicy) -> Result<(), GeometryError> {
        policy.point(self.translation_mm)?;
        if self.rotation.iter().flatten().any(|v| !v.is_finite()) {
            return Err(input("Rotation must be finite"));
        }
        for i in 0..3 {
            for j in 0..3 {
                if (dot(self.rotation[i], self.rotation[j]) - if i == j { 1. } else { 0. }).abs()
                    > 1e-10
                {
                    return Err(input(
                        "Rotation must be orthonormal; scaling and shear are not rigid placements",
                    ));
                }
            }
        }
        if (dot(cross(self.rotation[0], self.rotation[1]), self.rotation[2]) - 1.).abs() > 1e-10 {
            return Err(input("Rotation cannot reflect the solid"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileEdge {
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    Arc {
        center: [f64; 2],
        a: [f64; 2],
        b: [f64; 2],
        clockwise: bool,
    },
    Circle {
        center: [f64; 2],
        radius_mm: f64,
    },
}
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
impl ProfileEdge {
    fn ends(&self) -> ([f64; 2], [f64; 2]) {
        match *self {
            Self::Line { a, b } | Self::Arc { a, b, .. } => (a, b),
            Self::Circle { center, radius_mm } => {
                let p = [center[0] + radius_mm, center[1]];
                (p, p)
            }
        }
    }
    fn arc(&self) -> Option<(f64, f64, f64)> {
        if let Self::Arc {
            center,
            a,
            b,
            clockwise,
        } = *self
        {
            let r = distance(a, center);
            let start = (a[1] - center[1]).atan2(a[0] - center[0]);
            let end = (b[1] - center[1]).atan2(b[0] - center[0]);
            let sweep = if clockwise {
                -((start - end).rem_euclid(std::f64::consts::TAU))
            } else {
                (end - start).rem_euclid(std::f64::consts::TAU)
            };
            Some((r, start, sweep))
        } else {
            None
        }
    }
    fn area(&self) -> f64 {
        match *self {
            Self::Line { a, b } => (a[0] * b[1] - b[0] * a[1]) * 0.5,
            Self::Circle { radius_mm, .. } => std::f64::consts::PI * radius_mm * radius_mm,
            Self::Arc { center, a, b, .. } => {
                let (r, _, sweep) = self.arc().expect("arc");
                (center[0] * (b[1] - a[1]) - center[1] * (b[0] - a[0]) + r * r * sweep) * 0.5
            }
        }
    }
    fn reversed(&self) -> Self {
        match *self {
            Self::Line { a, b } => Self::Line { a: b, b: a },
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => Self::Arc {
                center,
                a: b,
                b: a,
                clockwise: !clockwise,
            },
            Self::Circle { .. } => self.clone(),
        }
    }
    fn checked(
        &self,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<ffi::CurveEdge, GeometryError> {
        let (a, b) = self.ends();
        policy.point(frame.point(a))?;
        policy.point(frame.point(b))?;
        let mut edge = ffi::CurveEdge {
            kind: 0,
            a: point(frame.point(a)),
            mid: point([0.; 3]),
            b: point(frame.point(b)),
            radius: 0.,
            source_index: 0,
        };
        match *self {
            Self::Line { .. } => policy.length(distance(a, b))?,
            Self::Circle { center, radius_mm } => {
                policy.length(radius_mm)?;
                policy.point(frame.point(center))?;
                edge.kind = 2;
                edge.a = point(frame.point(center));
                edge.radius = radius_mm;
            }
            Self::Arc { center, .. } => {
                policy.point(frame.point(center))?;
                let (r, start, sweep) = self.arc().expect("arc");
                policy.length(r)?;
                if (distance(b, center) - r).abs() > policy.linear_tolerance_mm
                    || distance(a, b) <= policy.linear_tolerance_mm
                    || r * sweep.abs() < policy.minimum_feature_mm
                {
                    return Err(input("Arc endpoints/radius are inconsistent or degenerate"));
                }
                let mid = start + sweep * 0.5;
                let mid = [center[0] + r * mid.cos(), center[1] + r * mid.sin()];
                policy.point(frame.point(mid))?;
                edge.kind = 1;
                edge.mid = point(frame.point(mid));
            }
        }
        Ok(edge)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub frame: PlaneFrame,
    pub outer: Vec<ProfileEdge>,
    pub holes: Vec<Vec<ProfileEdge>>,
}
impl Profile {
    pub fn rectangle(width: f64, depth: f64) -> Self {
        let points = [[0., 0.], [width, 0.], [width, depth], [0., depth]];
        Self {
            frame: PlaneFrame::default(),
            outer: (0..4)
                .map(|i| ProfileEdge::Line {
                    a: points[i],
                    b: points[(i + 1) % 4],
                })
                .collect(),
            holes: vec![],
        }
    }
    fn checked(
        &self,
        policy: GeometryPolicy,
    ) -> Result<(ffi::Frame3, Vec<ffi::CurveEdge>, Vec<u32>), GeometryError> {
        let frame = self.frame.checked(policy)?;
        let count = self.outer.len() + self.holes.iter().map(Vec::len).sum::<usize>();
        if count > 4096 || self.holes.len() > 128 {
            return Err(error(
                "profile",
                GeometryErrorKind::Limit,
                "Profile exceeds edge/hole budget",
            ));
        }
        let mut edges = vec![];
        let mut offsets = vec![0];
        let mut source_offset = 0;
        for wire in std::iter::once(&self.outer).chain(&self.holes) {
            if wire.is_empty() {
                return Err(input("Profile wires cannot be empty"));
            }
            for (i, e) in wire.iter().enumerate() {
                if matches!(e, ProfileEdge::Circle { .. }) && wire.len() != 1 {
                    return Err(input("A full-circle wire must contain only that circle"));
                }
                if distance(e.ends().1, wire[(i + 1) % wire.len()].ends().0)
                    > policy.linear_tolerance_mm
                {
                    return Err(input(
                        "Profile wire is not closed/connected within tolerance",
                    ));
                }
                e.checked(self.frame, policy)?;
            }
            let area = wire.iter().map(ProfileEdge::area).sum::<f64>();
            if !area.is_finite() || area.abs() < policy.minimum_feature_mm.powi(2) {
                return Err(input("Profile has degenerate signed area"));
            }
            if area < 0. {
                for (i, e) in wire.iter().enumerate().rev() {
                    let mut edge = e.reversed().checked(self.frame, policy)?;
                    edge.source_index = (source_offset + i) as u32;
                    edges.push(edge);
                }
            } else {
                for (i, e) in wire.iter().enumerate() {
                    let mut edge = e.checked(self.frame, policy)?;
                    edge.source_index = (source_offset + i) as u32;
                    edges.push(edge);
                }
            }
            source_offset += wire.len();
            offsets.push(edges.len() as u32);
        }
        Ok((frame, edges, offsets))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOperation {
    Union,
    Subtract,
    Intersect,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    /// Evaluation-local ordinal, never a persistent topology reference.
    pub index: u32,
    pub plane: Option<([f64; 3], [f64; 3])>,
    pub center_mm: [f64; 3],
    pub area_mm2: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayEdge {
    /// Evaluation-local index into the B-rep edge map, not a mesh edge.
    pub index: u32,
    pub points_mm: Vec<[f64; 3]>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayVertex {
    pub index: u32,
    pub position_mm: [f64; 3],
}
pub struct TopologyDisplay {
    pub topology: TopologySnapshot,
    pub edges: Vec<DisplayEdge>,
    pub vertices: Vec<DisplayVertex>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshOptions {
    pub linear_deflection_mm: f64,
    pub angular_deflection_rad: f64,
    pub maximum_triangles: u32,
}
impl Default for MeshOptions {
    fn default() -> Self {
        Self {
            linear_deflection_mm: 0.1,
            angular_deflection_rad: 0.3,
            maximum_triangles: 1_000_000,
        }
    }
}
/// Owned solid geometry and its provenance. Each solid stays on its owner thread.
/// ```compile_fail
/// fn requires_send<T: Send>() {}
/// requires_send::<multicad_kernel::Solid>();
/// ```
/// ```compile_fail
/// fn requires_sync<T: Sync>() {}
/// requires_sync::<multicad_kernel::Solid>();
/// ```
pub struct Solid {
    body: SolidBody,
    policy: GeometryPolicy,
    pub(crate) provenance: Vec<multicad_core::references::TopologyElement>,
    pub(crate) rust_origins: Vec<Origin>,
}

enum SolidBody {
    Occt(Shape),
    Prism(Rc<Prism>, PlaneFrame),
    PolygonPrism(Rc<PlacedPolygon>),
    AxialCylinder(Rc<AxialCylinder>, PlaneFrame, Option<Rc<CylinderSketch>>),
    Annulus(Rc<AnnulusBody>, PlaneFrame),
    CosmeticFastener(Rc<CosmeticFastenerBody>, PlaneFrame),
    Helical(Rc<HelicalBody>, PlaneFrame),
    Linear(Rc<LinearBody>, PlaneFrame),
    Revolved(Rc<RevolutionBody>, PlaneFrame),
    RoundedSlot(Rc<RoundedSlotBody>, PlaneFrame),
    RustCompound(Rc<RustCompound>),
    Orthogonal(Rc<OrthogonalBody>, PlaneFrame),
}

pub(crate) struct CylinderSketch {
    pub(crate) origin: Origin,
    pub(crate) center_uv: [f64; 2],
    pub(crate) reverse: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MeshUse {
    Display,
    Manufacturing,
}

impl Solid {
    /// Publish a straight-sided sketch extrusion with the original authored
    /// edge anchors, even if winding normalization reversed some edges.
    pub fn from_polygon_prism(
        prism: PolygonPrism,
        frame: PlaneFrame,
        feature: EntityId,
        origins: &[Origin],
        reversed: &[bool],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::from_polygon_prism_sweep(prism, frame, false, feature, origins, reversed, policy)
    }

    /// Keep the authored start and end cap identities when sweeping against
    /// the frame normal. The local prism always has positive height.
    pub fn from_polygon_prism_sweep(
        prism: PolygonPrism,
        frame: PlaneFrame,
        reverse_sweep: bool,
        feature: EntityId,
        origins: &[Origin],
        reversed: &[bool],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        policy.length(prism.height().as_f64())?;
        let line_count = prism.rings().iter().map(Vec::len).sum::<usize>();
        let edge_count = line_count + prism.circular_holes().len();
        let unique: std::collections::BTreeSet<_> = origins.iter().collect();
        if feature.is_nil()
            || origins.len() != edge_count
            || reversed.len() != edge_count
            || reversed[line_count..].iter().any(|reversed| *reversed)
            || unique.len() != edge_count
            || origins.iter().any(|origin| {
                origin.source.is_nil()
                    || origin.role.trim().is_empty()
                    || format!("side/{}/{}", origin.source, origin.role).len() > 256
            })
        {
            return Err(input(
                "Every polygon edge requires a distinct valid authored origin",
            ));
        }
        for circle in prism.circular_holes() {
            policy.length(circle.radius().as_f64())?;
        }
        let placed = PlacedPolygon {
            prism: Rc::new(prism),
            frame,
            reverse_sweep,
            feature,
            edges: origins
                .iter()
                .cloned()
                .zip(reversed.iter().copied())
                .collect(),
            pockets: Rc::default(),
            pocket_origins: Rc::default(),
            bore_treatments: None,
        };
        for length in placed.side_lengths() {
            policy.length(length)?;
        }
        let bounds = placed.bounds();
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::PolygonPrism(Rc::new(placed)),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn polygon_prism(&self) -> Option<&PlacedPolygon> {
        match &self.body {
            SolidBody::PolygonPrism(placed) => Some(placed),
            _ => None,
        }
    }

    /// Own a validated analytic prism in the Rust backend.
    pub fn from_rust_prism(prism: Prism, policy: GeometryPolicy) -> Result<Self, GeometryError> {
        Self::from_placed_prism(prism, PlaneFrame::default(), policy)
    }

    pub fn rust_box(
        feature: u128,
        size: [Rational; 3],
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let prism = Prism::rectangular(feature, size)
            .map_err(|cause| input(format!("Rust box failed: {cause:?}")))?;
        Self::from_placed_prism(prism, frame, policy)
    }

    fn from_placed_prism(
        prism: Prism,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if prism.feature() == 0 || prism.inner_loops().any(|(id, _)| id == 0) {
            return Err(input("Prism topology requires non-nil source identities"));
        }
        for size in prism.dimensions() {
            policy.length(size.as_f64())?;
        }
        if let Some(end) = prism.rounded_end() {
            policy.length(end.radius().as_f64())?;
        }
        for (_, circle) in prism.inner_loops() {
            policy.length(circle.radius().as_f64())?;
        }
        let solid = Self {
            body: SolidBody::Prism(Rc::new(prism), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        };
        let bounds = solid.bounds()?;
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(solid)
    }

    pub fn rust_prism(&self) -> Option<&Prism> {
        self.rust_prism_frame().map(|(prism, _)| prism)
    }

    pub(crate) fn rust_prism_frame(&self) -> Option<(&Prism, PlaneFrame)> {
        match &self.body {
            SolidBody::Prism(prism, frame) => Some((prism, *frame)),
            _ => None,
        }
    }

    pub fn from_axial_cylinder(
        cylinder: AxialCylinder,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let [x, y] = cylinder.circle().center().map(Rational::as_f64);
        if frame.origin != [x, y, cylinder.bottom().as_f64()]
            || frame.u[2] != 0.
            || frame.v[2] != 0.
            || frame.normal()[2] <= 0.
        {
            return Err(input(
                "Rust cylinder requires a matching frame with positive Z normal",
            ));
        }
        Self::build_axial_cylinder(cylinder, frame, None, policy)
    }

    /// Place a circular sketch sweep along the frame normal. The frame
    /// starts at the first rim center and points along the positive height.
    /// A reverse sweep retains the authored cap attachment basis.
    pub fn from_axial_cylinder_sweep(
        cylinder: AxialCylinder,
        frame: PlaneFrame,
        center_uv: [f64; 2],
        reverse: bool,
        origin: Origin,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if origin.source.is_nil()
            || origin.role.trim().is_empty()
            || format!("side/{}/{}", origin.source, origin.role).len() > 256
        {
            return Err(input(
                "Circular extrusion requires a valid authored curve origin",
            ));
        }
        policy.point([center_uv[0], center_uv[1], 0.])?;
        let sketch = CylinderSketch {
            origin,
            center_uv,
            reverse,
        };
        Self::build_axial_cylinder(cylinder, frame, Some(Rc::new(sketch)), policy)
    }

    fn build_axial_cylinder(
        cylinder: AxialCylinder,
        frame: PlaneFrame,
        sketch: Option<Rc<CylinderSketch>>,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if cylinder.feature() == 0 {
            return Err(input(
                "Cylinder topology requires a non-nil source identity",
            ));
        }
        policy.length(cylinder.circle().radius().as_f64())?;
        policy.length(cylinder.height().as_f64())?;
        frame.checked(policy)?;
        let solid = Self {
            body: SolidBody::AxialCylinder(Rc::new(cylinder), frame, sketch),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        };
        let bounds = solid.bounds()?;
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(solid)
    }

    pub fn axial_cylinder(&self) -> Option<&AxialCylinder> {
        let (cylinder, frame) = self.axial_cylinder_frame()?;
        let [x, y] = cylinder.circle().center().map(Rational::as_f64);
        (frame.origin == [x, y, cylinder.bottom().as_f64()]
            && frame.u[2] == 0.
            && frame.v[2] == 0.
            && frame.normal()[2] > 0.)
            .then_some(cylinder)
    }

    pub(crate) fn axial_cylinder_frame(&self) -> Option<(&AxialCylinder, PlaneFrame)> {
        match &self.body {
            SolidBody::AxialCylinder(cylinder, frame, _) => Some((cylinder, *frame)),
            _ => None,
        }
    }

    pub(crate) fn cylinder_sketch(&self) -> Option<&CylinderSketch> {
        match &self.body {
            SolidBody::AxialCylinder(_, _, sketch) => sketch.as_deref(),
            _ => None,
        }
    }

    /// Subtract an exact axial through-cylinder in the Rust backend.
    pub fn subtract_rust_prism_cylinder(
        &self,
        feature: u128,
        circle: Circle,
        bottom: Rational,
        top: Rational,
    ) -> Result<Self, GeometryError> {
        let Some((prism, frame)) = self.rust_prism_frame() else {
            return Err(error(
                "cylindrical cut",
                GeometryErrorKind::Unsupported,
                "Expected a Rust prism",
            ));
        };
        if frame != PlaneFrame::default() {
            return Err(error(
                "cylindrical cut",
                GeometryErrorKind::Unsupported,
                "Rust through-bores currently require an unplaced prism",
            ));
        }
        self.policy.point([0., 0., bottom.as_f64()])?;
        self.policy.point([0., 0., top.as_f64()])?;
        let next = prism
            .subtract_cylinder(feature, circle, bottom, top)
            .map_err(|reason| {
                error(
                    "cylindrical cut",
                    GeometryErrorKind::Input,
                    format!("{reason:?}"),
                )
            })?;
        Self::from_rust_prism(next, self.policy)
    }

    pub(crate) fn occt(&self) -> Result<&ffi::Shape, GeometryError> {
        match &self.body {
            SolidBody::Occt(shape) => Ok(&shape.0),
            _ => Err(error(
                "solid backend",
                GeometryErrorKind::Unsupported,
                "This operation is unavailable for the Rust solid",
            )),
        }
    }

    pub(crate) fn into_occt(self) -> Option<cxx::UniquePtr<ffi::Shape>> {
        match self.body {
            SolidBody::Occt(shape) => Some(shape.0),
            _ => None,
        }
    }

    pub fn geometry_policy(&self) -> GeometryPolicy {
        self.policy
    }
    pub fn uses_rust_geometry(&self) -> bool {
        !matches!(self.body, SolidBody::Occt(_))
    }
    pub(crate) fn rust_compound(&self) -> Option<&RustCompound> {
        match &self.body {
            SolidBody::RustCompound(compound) => Some(compound),
            _ => None,
        }
    }
    pub(crate) fn orthogonal_body(&self) -> Option<(&OrthogonalBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Orthogonal(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
    pub(crate) fn make_rust_compound(
        solids: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<Self, GeometryError> {
        RustCompound::assemble(solids, feature, policy, interrupt)
    }
    /// Another owner of the same immutable exact B-rep (and its provenance),
    /// without copying geometry. Kernel operations never modify their input
    /// shapes, so both owners observe identical geometry. Like every Solid
    /// the share stays on the thread that created it (`!Send`/`!Sync`).
    pub fn share(&self) -> Result<Self, GeometryError> {
        let body = match &self.body {
            SolidBody::Prism(prism, frame) => SolidBody::Prism(Rc::clone(prism), *frame),
            SolidBody::PolygonPrism(prism) => SolidBody::PolygonPrism(Rc::clone(prism)),
            SolidBody::AxialCylinder(cylinder, frame, sketch) => {
                SolidBody::AxialCylinder(Rc::clone(cylinder), *frame, sketch.clone())
            }
            SolidBody::Annulus(body, frame) => SolidBody::Annulus(Rc::clone(body), *frame),
            SolidBody::CosmeticFastener(body, frame) => {
                SolidBody::CosmeticFastener(Rc::clone(body), *frame)
            }
            SolidBody::Helical(body, frame) => SolidBody::Helical(Rc::clone(body), *frame),
            SolidBody::Linear(body, frame) => SolidBody::Linear(Rc::clone(body), *frame),
            SolidBody::Revolved(body, frame) => SolidBody::Revolved(Rc::clone(body), *frame),
            SolidBody::RoundedSlot(body, frame) => SolidBody::RoundedSlot(Rc::clone(body), *frame),
            SolidBody::RustCompound(compound) => SolidBody::RustCompound(Rc::clone(compound)),
            SolidBody::Orthogonal(body, frame) => SolidBody::Orthogonal(Rc::clone(body), *frame),
            SolidBody::Occt(shape) => {
                let handle = ffi::share_shape(&shape.0).map_err(|e| native("share", e))?;
                if handle.is_null() {
                    return Err(error(
                        "share",
                        GeometryErrorKind::InvalidResult,
                        "Kernel returned an empty shape",
                    ));
                }
                SolidBody::Occt(Shape(handle))
            }
        };
        Ok(Self {
            body,
            policy: self.policy,
            provenance: self.provenance.clone(),
            rust_origins: self.rust_origins.clone(),
        })
    }
    pub(crate) fn adopt(
        handle: cxx::UniquePtr<ffi::Shape>,
        policy: GeometryPolicy,
        operation: &'static str,
    ) -> Result<Self, GeometryError> {
        if handle.is_null() {
            return Err(error(
                operation,
                GeometryErrorKind::InvalidResult,
                "Kernel returned an empty shape",
            ));
        }
        let solid = Self {
            body: SolidBody::Occt(Shape(handle)),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        };
        if !ffi::valid(solid.occt()?).map_err(|e| native(operation, e))?
            || solid.solid_count()? == 0
        {
            return Err(error(
                operation,
                GeometryErrorKind::InvalidResult,
                "Result is not a valid volumetric solid",
            ));
        }
        let v = solid.volume_mm3()?;
        if !v.is_finite() || v <= 0. {
            return Err(error(
                operation,
                GeometryErrorKind::InvalidResult,
                "Result has no positive finite volume",
            ));
        }
        let bounds = solid.bounds()?;
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(solid)
    }
    pub fn box_solid(size_mm: [f64; 3], policy: GeometryPolicy) -> Result<Self, GeometryError> {
        policy.validate()?;
        for v in size_mm {
            policy.length(v)?;
        }
        Self::adopt(
            ffi::extrude(size_mm[0], size_mm[1], size_mm[2]).map_err(|e| native("box", e))?,
            policy,
            "box",
        )
    }
    pub fn cylinder(
        frame: PlaneFrame,
        radius_mm: f64,
        height_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        policy.length(radius_mm)?;
        policy.length(height_mm)?;
        Self::adopt(
            ffi::cylinder(&frame.checked(policy)?, radius_mm, height_mm)
                .map_err(|e| native("cylinder", e))?,
            policy,
            "cylinder",
        )
    }
    pub fn extrude_profile(
        profile: &Profile,
        height_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        policy.length(height_mm.abs())?;
        let (frame, edges, offsets) = profile.checked(policy)?;
        Self::adopt(
            ffi::profile_extrude(&frame, &edges, &offsets, height_mm)
                .map_err(|e| native("extrude", e))?,
            policy,
            "extrude",
        )
    }
    pub fn boolean(
        &self,
        other: &Self,
        operation: BooleanOperation,
        policy: GeometryPolicy,
    ) -> Result<Option<Self>, GeometryError> {
        policy.validate()?;
        if let Some(tool) = other.axial_cylinder()
            && operation == BooleanOperation::Subtract
            && policy.boolean_fuzzy_mm == 0.
            && policy == self.policy
            && policy == other.policy
        {
            if let Some(target) = self.polygon_prism() {
                return Self::subtract_polygon_cylinder(target, tool, policy).map(Some);
            }
            if self.rust_prism().is_some() {
                return self
                    .subtract_rust_prism_cylinder(
                        tool.feature(),
                        tool.circle(),
                        tool.bottom(),
                        tool.top(),
                    )
                    .map(Some);
            }
        }
        if self.uses_rust_geometry() && other.uses_rust_geometry() {
            return self.orthogonal_boolean(other, operation, policy);
        }
        let op = match operation {
            BooleanOperation::Union => 0,
            BooleanOperation::Subtract => 1,
            BooleanOperation::Intersect => 2,
        };
        let handle = ffi::boolean_shapes(self.occt()?, other.occt()?, op, policy.boolean_fuzzy_mm)
            .map_err(|e| native("boolean", e))?;
        if handle.is_null() {
            return Ok(None);
        }
        let mut result = Self::adopt(handle, policy, "boolean")?;
        result.inherit_provenance(&[self, other], None)?;
        Ok(Some(result))
    }

    fn subtract_polygon_cylinder(
        target: &PlacedPolygon,
        tool: &AxialCylinder,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let operation = "cylindrical cut";
        if target.frame != PlaneFrame::default() {
            return Err(error(
                operation,
                GeometryErrorKind::Unsupported,
                "Rust polygon through-cuts currently require an unplaced XY profile",
            ));
        }
        let zero = Rational::new(0, 1).expect("zero is an exact length");
        let height = target.prism.height();
        let (bottom, top) = if target.reverse_sweep {
            (
                zero.checked_sub(height)
                    .expect("negation fits the rational domain"),
                zero,
            )
        } else {
            (zero, height)
        };
        if tool.bottom() >= bottom || tool.top() <= top {
            return Err(error(
                operation,
                GeometryErrorKind::Input,
                "The tool must extend strictly through both polygon caps",
            ));
        }
        let result = Self::with_polygon_through_holes(
            target,
            [(
                tool.circle(),
                Origin {
                    source: EntityId::from_uuid_bits(tool.feature()),
                    role: "side.cylinder".into(),
                },
            )],
            policy,
        )?;
        if target.pockets.is_empty() {
            Ok(result)
        } else {
            result.with_polygon_pockets(target.pockets.to_vec(), target.pocket_origins.to_vec())
        }
    }
    /// Native COMMON query using OCCT's internal worker pool. Rust handles stay
    /// on their owner thread; the non-destructive operation owns its workers.
    /// Native validity, fuzzy tolerance and provenance rules match `boolean`.
    pub fn intersect_parallel(
        &self,
        other: &Self,
        policy: GeometryPolicy,
    ) -> Result<Option<Self>, GeometryError> {
        policy.validate()?;
        let handle = ffi::intersect_parallel(self.occt()?, other.occt()?, policy.boolean_fuzzy_mm)
            .map_err(|e| native("parallel intersection", e))?;
        if handle.is_null() {
            return Ok(None);
        }
        let mut result = Self::adopt(handle, policy, "parallel intersection")?;
        result.inherit_provenance(&[self, other], None)?;
        Ok(Some(result))
    }
    pub fn transformed(&self, transform: RigidTransform) -> Result<Self, GeometryError> {
        transform.validate(self.policy)?;
        if self.uses_rust_geometry() {
            if transform == RigidTransform::default() {
                return self.share();
            }
            let body = match &self.body {
                SolidBody::Prism(prism, frame) => {
                    SolidBody::Prism(Rc::clone(prism), transform.frame(*frame))
                }
                SolidBody::PolygonPrism(placed) => {
                    let mut placed = placed.as_ref().clone();
                    placed.frame = transform.frame(placed.frame);
                    SolidBody::PolygonPrism(Rc::new(placed))
                }
                SolidBody::AxialCylinder(cylinder, frame, sketch) => SolidBody::AxialCylinder(
                    Rc::clone(cylinder),
                    transform.frame(*frame),
                    sketch.clone(),
                ),
                SolidBody::Annulus(body, frame) => {
                    SolidBody::Annulus(Rc::clone(body), transform.frame(*frame))
                }
                SolidBody::Helical(body, frame) => {
                    SolidBody::Helical(Rc::clone(body), transform.frame(*frame))
                }
                SolidBody::CosmeticFastener(body, frame) => {
                    SolidBody::CosmeticFastener(Rc::clone(body), transform.frame(*frame))
                }
                SolidBody::Linear(body, frame) => {
                    SolidBody::Linear(Rc::clone(body), transform.frame(*frame))
                }
                SolidBody::Revolved(body, frame) => {
                    SolidBody::Revolved(Rc::clone(body), transform.frame(*frame))
                }
                SolidBody::RoundedSlot(body, frame) => {
                    SolidBody::RoundedSlot(Rc::clone(body), transform.frame(*frame))
                }
                SolidBody::RustCompound(compound) => {
                    SolidBody::RustCompound(compound.transformed(transform, self.policy)?)
                }
                SolidBody::Orthogonal(body, frame) => {
                    SolidBody::Orthogonal(Rc::clone(body), transform.frame(*frame))
                }
                SolidBody::Occt(_) => unreachable!("checked Rust body"),
            };
            let result = Self {
                body,
                policy: self.policy,
                provenance: vec![],
                rust_origins: self.rust_origins.clone(),
            };
            let bounds = result.bounds()?;
            self.policy.point(bounds.min)?;
            self.policy.point(bounds.max)?;
            return Ok(result);
        }
        let r: Vec<_> = transform.rotation.into_iter().flatten().collect();
        let mut result = Self::adopt(
            ffi::transform_shape(self.occt()?, &r, &point(transform.translation_mm))
                .map_err(|e| native("transform", e))?,
            self.policy,
            "transform",
        )?;
        result.inherit_provenance(&[self], Some(transform))?;
        Ok(result)
    }
    pub fn volume_mm3(&self) -> Result<f64, GeometryError> {
        if let Some((body, _)) = self.cosmetic_fastener_body() {
            return Ok(body.geometry.volume_mm3());
        }
        if let Some((body, _)) = self.rounded_slot_body() {
            return Ok(body.volume_mm3());
        }
        if let Some((body, _)) = self.linear_body() {
            return Ok(body.volume_mm3());
        }
        if let Some((body, _)) = self.revolution_body() {
            return Ok(body.geometry.volume_mm3());
        }
        if let Some((body, _)) = self.annulus_body() {
            return Ok(body.geometry.volume_mm3());
        }
        if let Some((body, _)) = self.helical_body() {
            return Ok(body.volume_mm3());
        }
        if let Some((body, _)) = self.orthogonal_body() {
            return Ok(body.volume_mm3());
        }
        if let Some(compound) = self.rust_compound() {
            return Ok(compound.volume_mm3());
        }
        if let Some(placed) = self.polygon_prism() {
            if let Some(treatments) = &placed.bore_treatments {
                return Ok(treatments.geometry.volume_mm3());
            }
            return Ok(multicad_rust_kernel::circular_pockets::volume_mm3(
                &placed.prism,
                &placed.pockets,
            ));
        }
        if let Some((cylinder, _)) = self.axial_cylinder_frame() {
            return Ok(std::f64::consts::PI * cylinder.volume_factor().as_f64());
        }
        if let Some(prism) = self.rust_prism() {
            return Ok(prism.volume().as_f64());
        }
        ffi::volume(self.occt()?).map_err(|e| native("volume", e))
    }
    pub fn solid_count(&self) -> Result<u32, GeometryError> {
        if let Some((body, _)) = self.orthogonal_body() {
            return Ok(body.geometry.solid_count());
        }
        if let Some(compound) = self.rust_compound() {
            return Ok(compound.solid_count());
        }
        if self.uses_rust_geometry() {
            return Ok(1);
        }
        ffi::solid_count(self.occt()?).map_err(|e| native("solid count", e))
    }
    pub fn bounds(&self) -> Result<Bounds, GeometryError> {
        if let Some((body, frame)) = self.cosmetic_fastener_body() {
            return Ok(body.bounds(frame));
        }
        if let Some((body, frame)) = self.rounded_slot_body() {
            return Ok(body.bounds(frame));
        }
        if let Some((body, frame)) = self.linear_body() {
            return Ok(body.bounds(frame));
        }
        if let Some((body, frame)) = self.revolution_body() {
            return Ok(body.bounds(frame));
        }
        if let Some((body, frame)) = self.annulus_body() {
            return Ok(body.bounds(frame));
        }
        if let Some((body, frame)) = self.helical_body() {
            return Ok(body.bounds(frame));
        }
        if let Some((body, frame)) = self.orthogonal_body() {
            return Ok(body.bounds(frame));
        }
        if let Some(compound) = self.rust_compound() {
            return Ok(compound.bounds());
        }
        if let Some(placed) = self.polygon_prism() {
            return Ok(placed.bounds());
        }
        if let Some(cylinder) = self.axial_cylinder() {
            let [min, max] = cylinder.bounds().map(|corner| corner.map(Rational::as_f64));
            return Ok(Bounds { min, max });
        }
        if let Some((cylinder, frame)) = self.axial_cylinder_frame() {
            let radius = cylinder.circle().radius().as_f64();
            let height = cylinder.height().as_f64();
            let axis = frame.normal();
            let extent: [f64; 3] = std::array::from_fn(|i| radius * frame.u[i].hypot(frame.v[i]));
            return Ok(Bounds {
                min: std::array::from_fn(|i| {
                    frame.origin[i] + (height * axis[i]).min(0.) - extent[i]
                }),
                max: std::array::from_fn(|i| {
                    frame.origin[i] + (height * axis[i]).max(0.) + extent[i]
                }),
            });
        }
        if let Some((prism, frame)) = self.rust_prism_frame() {
            if frame == PlaneFrame::default() {
                return Ok(Bounds {
                    min: [0.; 3],
                    max: prism.maximum().map(Rational::as_f64),
                });
            }
            let [length, width, height] = prism.dimensions().map(Rational::as_f64);
            let axis = frame.normal();
            let support = |u: f64, v: f64, n: f64| {
                let rectangle = (u * length).max(0.) + (v * width).max(0.);
                let cap = prism.rounded_end().map_or(rectangle, |circle| {
                    let radius = circle.radius().as_f64();
                    let arc = u * length + v * radius + radius * u.max(0.).hypot(v);
                    rectangle.max(arc)
                });
                cap + (n * height).max(0.)
            };
            return Ok(Bounds {
                min: std::array::from_fn(|i| {
                    frame.origin[i] - support(-frame.u[i], -frame.v[i], -axis[i])
                }),
                max: std::array::from_fn(|i| {
                    frame.origin[i] + support(frame.u[i], frame.v[i], axis[i])
                }),
            });
        }
        let b = ffi::bounds(self.occt()?).map_err(|e| native("bounds", e))?;
        Ok(Bounds {
            min: xyz(b.min),
            max: xyz(b.max),
        })
    }
    /// Measures the actual selected finite subshapes. Native handles remain on
    /// their owner thread; no display tessellation participates.
    pub fn minimum_distance(
        &self,
        a: crate::BRepSelection,
        other: &Self,
        b: crate::BRepSelection,
    ) -> Result<crate::MinimumDistance, GeometryError> {
        crate::distance::measure(
            self,
            a,
            other,
            b,
            self.policy
                .linear_tolerance_mm
                .max(other.policy.linear_tolerance_mm),
        )
    }
    /// Extract ordered exact wires on the explicitly chosen finite planar face.
    /// The frame must be coplanar; unsupported boundaries never become polylines.
    pub fn planar_face_contours(
        &self,
        face: u32,
        frame: PlaneFrame,
    ) -> Result<multicad_core::manufacturing::PlanarContours, GeometryError> {
        crate::planar_contours::extract(self, face, frame, self.policy)
    }
    pub fn faces(&self) -> Result<Vec<Face>, GeometryError> {
        if let Some((body, frame)) = self.cosmetic_fastener_body() {
            return Ok(body.faces(frame));
        }
        if let Some((body, frame)) = self.rounded_slot_body() {
            return Ok(body.faces(frame));
        }
        if let Some((body, frame)) = self.linear_body() {
            return Ok(body.faces(frame));
        }
        if let Some((body, frame)) = self.revolution_body() {
            return Ok(body.faces(frame));
        }
        if let Some((body, frame)) = self.annulus_body() {
            return Ok(body.faces(frame));
        }
        if let Some((body, frame)) = self.helical_body() {
            return Ok(body.faces(frame));
        }
        if let Some((body, frame)) = self.orthogonal_body() {
            return Ok(body.faces(frame));
        }
        if let Some(compound) = self.rust_compound() {
            return compound.faces();
        }
        if let Some(placed) = self.polygon_prism() {
            return Ok(placed.faces());
        }
        if let Some((cylinder, frame)) = self.axial_cylinder_frame() {
            return Ok(crate::rust_prism_topology::cylinder_faces(cylinder, frame));
        }
        if let Some((prism, frame)) = self.rust_prism_frame() {
            return Ok(crate::rust_prism_topology::faces(prism, frame));
        }
        // An exact affine image of a proven plane stays planar even when OCCT
        // stores it as a NURBS surface. Preserve that typed proof for picking;
        // no surface fitting or display triangles establish this descriptor.
        let planes: std::collections::BTreeMap<_, _> = self
            .provenance
            .iter()
            .filter_map(|e| match e.geometry {
                Some(multicad_core::references::AnalyticGeometry::Plane { frame })
                    if e.key.kind == multicad_core::references::TopologyKind::Face =>
                {
                    Some((e.key.ordinal, (frame.origin_mm, frame.normal())))
                }
                _ => None,
            })
            .collect();
        ffi::faces(self.occt()?)
            .map_err(|e| native("faces", e))
            .map(|faces| {
                faces
                    .into_iter()
                    .map(|f| Face {
                        index: f.index,
                        plane: f
                            .planar
                            .then_some((xyz(f.origin), xyz(f.normal)))
                            .or_else(|| planes.get(&f.index).copied()),
                        center_mm: xyz(f.center),
                        area_mm2: f.area,
                    })
                    .collect()
            })
    }
    fn mesh_options_valid(&self, options: MeshOptions) -> bool {
        (self.policy.linear_tolerance_mm.max(0.001)..=self.policy.maximum_coordinate_mm)
            .contains(&options.linear_deflection_mm)
            && (0.01..=1.).contains(&options.angular_deflection_rad)
            && (1..=5_000_000).contains(&options.maximum_triangles)
    }

    fn rust_mesh_options(options: MeshOptions) -> multicad_rust_kernel::display::Tessellation {
        multicad_rust_kernel::display::Tessellation {
            deflection: options.linear_deflection_mm,
            angle: options.angular_deflection_rad,
            triangle_limit: options.maximum_triangles as usize,
        }
    }

    fn rust_mesh(&self, options: MeshOptions, usage: MeshUse) -> Result<ffi::Mesh, GeometryError> {
        let rust_options = Self::rust_mesh_options(options);
        match &self.body {
            SolidBody::Prism(prism, frame) => crate::rust_display::placed_mesh(
                multicad_rust_kernel::display::rust_prism(prism, rust_options),
                *frame,
                false,
            ),
            SolidBody::PolygonPrism(placed) => {
                let mesh = if let Some(treatments) = &placed.bore_treatments {
                    multicad_rust_kernel::display::chamfered_polygon_prism(
                        &placed.prism,
                        &treatments.geometry,
                        rust_options,
                    )
                } else if placed.pockets.is_empty() {
                    multicad_rust_kernel::display::polygon_prism(&placed.prism, rust_options)
                } else {
                    multicad_rust_kernel::display::pocketed_polygon_prism(
                        &placed.prism,
                        &placed.pockets,
                        rust_options,
                    )
                };
                crate::rust_display::placed_mesh(mesh, placed.frame, placed.reverse_sweep)
            }
            SolidBody::AxialCylinder(cylinder, frame, _) => {
                let mut mesh = crate::rust_display::mesh(
                    multicad_rust_kernel::display::axial_cylinder(cylinder, rust_options),
                )?;
                let [x, y] = cylinder.circle().center().map(Rational::as_f64);
                let origin = [x, y, cylinder.bottom().as_f64()];
                if self.axial_cylinder().is_none() {
                    for point in mesh.vertices.as_chunks_mut::<3>().0 {
                        let local = std::array::from_fn(|i| point[i] - origin[i]);
                        *point = crate::rust_prism_topology::position(*frame, local);
                    }
                }
                Ok(mesh)
            }
            SolidBody::Annulus(body, frame) => crate::rust_display::placed_mesh(
                multicad_rust_kernel::annulus::tessellate(&body.geometry, rust_options),
                *frame,
                false,
            ),
            SolidBody::Helical(body, frame) => body.mesh(*frame, rust_options),
            SolidBody::CosmeticFastener(body, frame) => match usage {
                MeshUse::Display => body.mesh(*frame, rust_options),
                MeshUse::Manufacturing => Err(error(
                    "manufacturing mesh",
                    GeometryErrorKind::Unsupported,
                    "Cosmetic thread envelopes require modeled thread geometry for manufacturing",
                )),
            },
            SolidBody::Linear(body, frame) => {
                crate::rust_display::placed_mesh(body.mesh(rust_options), *frame, false)
            }
            SolidBody::Revolved(body, frame) => body.mesh(*frame, rust_options),
            SolidBody::RoundedSlot(body, frame) => body.mesh(*frame, rust_options),
            SolidBody::RustCompound(compound) => compound.mesh(options, usage),
            SolidBody::Orthogonal(body, frame) => body.mesh(*frame, rust_options, usage),
            SolidBody::Occt(_) => unreachable!("Rust-only tessellation"),
        }
    }

    pub fn mesh(&self, options: MeshOptions) -> Result<ffi::Mesh, GeometryError> {
        if !self.mesh_options_valid(options) {
            return Err(input("Invalid display tessellation settings/budget"));
        }
        let mesh = match &self.body {
            SolidBody::Occt(shape) => ffi::tessellate(
                &shape.0,
                options.linear_deflection_mm,
                options.angular_deflection_rad,
                options.maximum_triangles,
            )
            .map_err(|e| native("mesh", e))?,
            _ => self.rust_mesh(options, MeshUse::Display)?,
        };
        if mesh.vertices.is_empty()
            || !mesh.vertices.len().is_multiple_of(3)
            || !mesh.indices.len().is_multiple_of(3)
            || mesh.face_indices.len() != mesh.indices.len() / 3
            || mesh.vertices.iter().any(|v| !v.is_finite())
            || mesh
                .indices
                .iter()
                .any(|i| *i as usize >= mesh.vertices.len() / 3)
        {
            return Err(error(
                "mesh",
                GeometryErrorKind::InvalidResult,
                "Invalid display mesh",
            ));
        }
        // Retain only after every ordinary detached TriangleMesh validation.
        // Optional receipt allocation failure does not invalidate display data.
        if let SolidBody::Occt(shape) = &self.body {
            let _ = ffi::remember_validated_mesh(
                &shape.0,
                options.linear_deflection_mm,
                options.angular_deflection_rad,
                options.maximum_triangles,
                &mesh,
            );
        }
        Ok(mesh)
    }
    /// Fresh manufacturing triangulation with exact boundary identity.
    pub fn manufacturing_mesh(&self, options: MeshOptions) -> Result<ffi::Mesh, GeometryError> {
        Ok(self.manufacturing_mesh_with_receipt(options)?.mesh)
    }

    fn rust_manufacturing_mesh(
        &self,
        options: MeshOptions,
    ) -> Result<crate::ManufacturingMesh, GeometryError> {
        use multicad_rust_kernel::{
            display::{DisplayMesh, MeshError},
            manufacturing,
        };
        const OPERATION: &str = "manufacturing mesh";
        let ffi::Mesh {
            vertices,
            indices,
            face_indices,
        } = self.rust_mesh(options, MeshUse::Manufacturing)?;
        let tessellation = DisplayMesh {
            vertices,
            indices,
            face_indices,
        };
        let proof = manufacturing::certify(&tessellation).map_err(|cause| {
            let kind = match cause {
                MeshError::InvalidOptions => GeometryErrorKind::Input,
                MeshError::BudgetExceeded => GeometryErrorKind::Limit,
                MeshError::UnresolvedBoundary => GeometryErrorKind::InvalidResult,
            };
            error(
                OPERATION,
                kind,
                format!("Rust indexed mesh certification failed: {cause:?}"),
            )
        })?;
        let exact_volume = self.volume_mm3()?;
        let volume_tolerance = self.rust_mesh_volume_tolerance(options, exact_volume)?;
        if !volume_tolerance.is_finite()
            || (proof.signed_volume_mm3 - exact_volume).abs() > volume_tolerance
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::InvalidResult,
                "Rust manufacturing mesh volume differs from the exact solid beyond the chord error bound",
            ));
        }
        let DisplayMesh {
            vertices,
            indices,
            face_indices,
        } = tessellation;
        Ok(crate::ManufacturingMesh {
            mesh: ffi::Mesh {
                vertices,
                indices,
                face_indices,
            },
            maximum_displacement_mm: 0.,
            edge_pairs: proof.edge_pairs,
            vertex_links: proof.vertex_links,
            removed_native_pole_triangles: 0,
            removed_exact_zero_triangles: 0,
            exact_vertex_pairs: 0,
            internal_constraint_edges: 0,
            removed_meshing_internal_edges: 0,
            removed_meshing_constraint_wire_containers: 0,
            removed_meshing_internal_vertices: 0,
            remeshed_without_internal_constraints: false,
        })
    }

    fn rust_mesh_volume_tolerance(
        &self,
        options: MeshOptions,
        exact_volume: f64,
    ) -> Result<f64, GeometryError> {
        if let Some(compound) = self.rust_compound() {
            return compound.members().try_fold(0., |total, member| {
                Ok(total + member.rust_mesh_volume_tolerance(options, member.volume_mm3()?)?)
            });
        }
        let mut surface_area = 0.;
        let mut curved_area = 0.;
        for face in self.faces()? {
            surface_area += face.area_mm2;
            if face.plane.is_none() {
                curved_area += face.area_mm2;
            }
        }
        let chord_error = if let Some((body, _)) = self.helical_body() {
            body.volume_error_bound(Self::rust_mesh_options(options))
        } else if let Some((body, _)) = self.revolution_body() {
            multicad_rust_kernel::revolution_mesh::volume_error_bound(
                &body.geometry,
                Self::rust_mesh_options(options),
            )
        } else if let Some((body, _)) = self.annulus_body() {
            multicad_rust_kernel::annulus::volume_error_bound(
                &body.geometry,
                Self::rust_mesh_options(options),
            )
        } else {
            // Arc/chord area loss is bounded by sagitta times arc length.
            // Extruding this loss bounds volume by the curved wall area.
            options.linear_deflection_mm * curved_area
        };
        Ok(
            chord_error
                + self.policy.linear_tolerance_mm * surface_area * 2.
                + exact_volume * 1e-10,
        )
    }

    pub fn manufacturing_mesh_with_receipt(
        &self,
        options: MeshOptions,
    ) -> Result<crate::ManufacturingMesh, GeometryError> {
        if !self.mesh_options_valid(options) {
            return Err(input("Invalid manufacturing tessellation settings/budget"));
        }
        if self.uses_rust_geometry() {
            return self.rust_manufacturing_mesh(options);
        }
        let tolerance = self
            .policy
            .linear_tolerance_mm
            .min(options.linear_deflection_mm / 16.);
        let receipt = ffi::tessellate_manufacturing(
            self.occt()?,
            options.linear_deflection_mm,
            options.angular_deflection_rad,
            options.maximum_triangles,
            tolerance,
        )
        .map_err(|e| native("manufacturing mesh", e))?;
        let mesh = &receipt.mesh;
        if mesh.vertices.is_empty()
            || !mesh.vertices.len().is_multiple_of(3)
            || !mesh.indices.len().is_multiple_of(3)
            || mesh.face_indices.len() != mesh.indices.len() / 3
            || mesh.vertices.iter().any(|v| !v.is_finite())
            || mesh
                .indices
                .iter()
                .any(|i| *i as usize >= mesh.vertices.len() / 3)
            || !receipt.maximum_displacement_mm.is_finite()
            || receipt.maximum_displacement_mm < 0.
            || receipt.maximum_displacement_mm > tolerance
        {
            return Err(error(
                "manufacturing mesh",
                GeometryErrorKind::InvalidResult,
                "Invalid native manufacturing mesh or seam receipt",
            ));
        }
        Ok(receipt)
    }
    /// Publish exact topology and its viewport overlays together. Pass no
    /// deflection for topology alone, without sampling edges or vertices.
    pub fn topology_with_display(
        &self,
        owner: EntityId,
        revision: u64,
        deflection_mm: Option<f64>,
    ) -> Result<TopologyDisplay, GeometryError> {
        let topology = self.topology_snapshot(owner, revision)?;
        let (edges, vertices) = match deflection_mm {
            None => (Vec::new(), Vec::new()),
            Some(deflection) if self.uses_rust_geometry() => {
                self.validate_display_deflection(deflection)?;
                let edges = if let Some((body, frame)) = self.helical_body() {
                    body.display_edges(frame, deflection)?
                } else {
                    crate::rust_display::edges(&topology, deflection)?
                };
                (edges, crate::rust_display::vertices(&topology))
            }
            Some(deflection) => (self.display_edges(deflection)?, self.display_vertices()?),
        };
        Ok(TopologyDisplay {
            topology,
            edges,
            vertices,
        })
    }

    fn validate_display_deflection(&self, deflection_mm: f64) -> Result<(), GeometryError> {
        if !deflection_mm.is_finite()
            || deflection_mm < 0.001
            || deflection_mm > self.policy.maximum_coordinate_mm
        {
            return Err(input("Invalid edge display deflection"));
        }
        Ok(())
    }

    pub fn display_edges(&self, deflection_mm: f64) -> Result<Vec<DisplayEdge>, GeometryError> {
        self.validate_display_deflection(deflection_mm)?;
        if let Some((body, frame)) = self.helical_body() {
            return body.display_edges(frame, deflection_mm);
        }
        if let Some(topology) = self.rust_topology() {
            return crate::rust_display::edges(&topology?, deflection_mm);
        }
        let edges = ffi::display_edges(self.occt()?, deflection_mm)
            .map_err(|e| native("display edges", e))?;
        edges
            .into_iter()
            .map(|edge| {
                if edge.points.len() < 6
                    || !edge.points.len().is_multiple_of(3)
                    || edge.points.iter().any(|v| !v.is_finite())
                {
                    return Err(error(
                        "display edges",
                        GeometryErrorKind::InvalidResult,
                        "Invalid native edge display data",
                    ));
                }
                Ok(DisplayEdge {
                    index: edge.index,
                    points_mm: edge.points.as_chunks::<3>().0.to_vec(),
                })
            })
            .collect()
    }
    pub fn display_vertices(&self) -> Result<Vec<DisplayVertex>, GeometryError> {
        if let Some(topology) = self.rust_topology() {
            return Ok(crate::rust_display::vertices(&topology?));
        }
        ffi::vertices(self.occt()?)
            .map_err(|e| native("display vertices", e))?
            .into_iter()
            .map(|v| {
                let position_mm = xyz(v.point);
                self.policy.point(position_mm)?;
                Ok(DisplayVertex {
                    index: v.index,
                    position_mm,
                })
            })
            .collect()
    }
    pub(crate) fn rust_topology(
        &self,
    ) -> Option<Result<multicad_core::references::TopologySnapshot, GeometryError>> {
        let feature = match &self.body {
            SolidBody::PolygonPrism(placed) => placed.feature,
            SolidBody::Prism(prism, _) => EntityId::from_uuid_bits(prism.feature()),
            SolidBody::AxialCylinder(cylinder, ..) => EntityId::from_uuid_bits(cylinder.feature()),
            SolidBody::Annulus(body, _) => body.feature,
            SolidBody::Helical(body, _) => body.feature,
            SolidBody::CosmeticFastener(body, _) => body.feature,
            SolidBody::Linear(body, _) => body.feature,
            SolidBody::Revolved(body, _) => body.feature,
            SolidBody::RoundedSlot(body, _) => body.feature,
            SolidBody::RustCompound(compound) => compound.feature,
            SolidBody::Orthogonal(body, _) => body.feature,
            SolidBody::Occt(_) => return None,
        };
        Some(self.topology_snapshot(feature, 0))
    }
    /// Whether the active backend can encode this exact solid in STEP.
    pub fn supports_step_export(&self) -> bool {
        match &self.body {
            SolidBody::Occt(_)
            | SolidBody::PolygonPrism(_)
            | SolidBody::AxialCylinder(..)
            | SolidBody::Annulus(..)
            | SolidBody::Linear(..)
            | SolidBody::RoundedSlot(..) => true,
            SolidBody::Prism(prism, _) => prism.rounded_end().is_none(),
            SolidBody::RustCompound(compound) => compound.members().all(Self::supports_step_export),
            SolidBody::Helical(..) | SolidBody::CosmeticFastener(..) => false,
            SolidBody::Revolved(body, frame) => body.circular_sweep(*frame).is_some(),
            SolidBody::Orthogonal(body, _) => body.supports_step(),
        }
    }

    pub fn write_step(&self, path: &Path) -> Result<(), GeometryError> {
        use multicad_rust_kernel::step;

        let path_text = crate::step_path(path).map_err(|e| input(e.to_string()))?;
        if let SolidBody::Occt(shape) = &self.body {
            return ffi::write_step(&shape.0, path_text).map_err(|e| native("write STEP", e));
        }
        let operation = "write STEP";
        if self.helical_body().is_some() {
            return Err(error(
                operation,
                GeometryErrorKind::Unsupported,
                "STEP export of Rust helical surfaces is unavailable",
            ));
        }
        if !self.supports_step_export() {
            return Err(error(
                operation,
                GeometryErrorKind::Unsupported,
                "Rust STEP export requires supported geometry with one closed exterior shell per solid",
            ));
        }
        let mut writer = step::Writer::new();
        let encoded = self
            .append_step(&mut writer)
            .and_then(|()| writer.finish())
            .map_err(|e| {
                let kind = match e {
                    step::StepError::Unsupported => GeometryErrorKind::Unsupported,
                    step::StepError::Limit => GeometryErrorKind::Limit,
                    _ => GeometryErrorKind::InvalidResult,
                };
                error(operation, kind, format!("{e:?}"))
            })?;
        std::fs::write(path, encoded)
            .map_err(|e| error(operation, GeometryErrorKind::InvalidResult, e.to_string()))
    }

    fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        use multicad_rust_kernel::step;
        match &self.body {
            SolidBody::PolygonPrism(placed) => {
                if let Some(treatments) = &placed.bore_treatments {
                    writer.append_chamfered_polygon_prism(
                        &placed.prism,
                        treatments.geometry.chamfers(),
                        placed.frame.origin,
                        placed.frame.u,
                        placed.frame.v,
                        placed.reverse_sweep,
                    )
                } else if placed.pockets.is_empty() {
                    writer.append_polygon_prism(
                        &placed.prism,
                        placed.frame.origin,
                        placed.frame.u,
                        placed.frame.v,
                        placed.reverse_sweep,
                    )
                } else {
                    writer.append_pocketed_polygon_prism(
                        &placed.prism,
                        &placed.pockets,
                        placed.frame.origin,
                        placed.frame.u,
                        placed.frame.v,
                        placed.reverse_sweep,
                    )
                }
            }
            SolidBody::Prism(prism, frame) => {
                writer.append_rectangular_prism(prism, frame.origin, frame.u, frame.v)
            }
            SolidBody::AxialCylinder(cylinder, frame, _) => {
                if self.axial_cylinder().is_some() {
                    writer.append_axial_cylinder(cylinder, frame.u)
                } else {
                    writer.append_placed_cylinder(cylinder, frame.origin, frame.u, frame.v)
                }
            }
            SolidBody::Annulus(body, frame) => {
                writer.append_placed_annulus(&body.geometry, frame.origin, frame.u, frame.v)
            }
            SolidBody::Linear(body, frame) => body.append_step(writer, *frame),
            SolidBody::RoundedSlot(body, frame) => body.append_step(writer, *frame),
            SolidBody::Revolved(body, frame) => {
                let (base, radius, height, bore) = body
                    .circular_sweep(*frame)
                    .expect("checked round revolution");
                writer.append_placed_circular_sweep(
                    radius,
                    bore,
                    height,
                    base.origin,
                    base.u,
                    base.v,
                )
            }
            SolidBody::RustCompound(compound) => {
                for member in compound.members() {
                    member.append_step(writer)?;
                }
                Ok(())
            }
            SolidBody::Helical(..) | SolidBody::CosmeticFastener(..) => {
                Err(step::StepError::Unsupported)
            }
            SolidBody::Orthogonal(body, frame) => body.append_step(writer, *frame),
            SolidBody::Occt(_) => unreachable!("native STEP dispatched above"),
        }
    }

    /// Every solid in a STEP file as (representation name, exact BRep text).
    pub fn step_solids(path: &Path) -> Result<Vec<(String, String)>, GeometryError> {
        let path = crate::step_path(path).map_err(|e| input(e.to_string()))?;
        Ok(ffi::step_solids(path)
            .map_err(|e| native("read STEP", e))?
            .into_iter()
            .map(|s| (s.name, s.brep))
            .collect())
    }
    /// The exact shape as OCCT BRep text.
    pub fn to_brep(&self) -> Result<String, GeometryError> {
        ffi::shape_to_brep(self.occt()?).map_err(|e| native("write BRep", e))
    }
    /// Lossless native BRep representation digest for exact proof-cache keys.
    ///
    /// Hashes bounded (16 MiB) OCCT binary v4 output without triangulations or
    /// normals. Includes native geometry, tolerances, topology orientations and
    /// locations; excludes Rust provenance and display data. Equal physical
    /// geometry need not have an equal digest if its native representation or
    /// flags differ. This is not a canonical or cross-version geometry identity.
    /// Serialization/size failures return an error: callers must run the native
    /// query rather than substituting a lossy key or accepting a cached proof.
    pub fn exact_geometry_digest(
        &self,
    ) -> Result<multicad_core::project_file::ContentHash, GeometryError> {
        let bytes = ffi::shape_binary_geometry(self.occt()?, 16 * 1024 * 1024)
            .map_err(|e| native("binary geometry digest", e))?;
        Ok(multicad_core::project_file::ContentHash::of(&bytes))
    }
    /// Rebuild an exact shape from OCCT BRep text.
    pub fn from_brep(text: &str, policy: GeometryPolicy) -> Result<Self, GeometryError> {
        policy.validate()?;
        Self::adopt(
            ffi::shape_from_brep(text).map_err(|e| native("read BRep", e))?,
            policy,
            "read BRep",
        )
    }
    pub fn read_step(path: &Path, policy: GeometryPolicy) -> Result<Self, GeometryError> {
        policy.validate()?;
        let path = crate::step_path(path).map_err(|e| input(e.to_string()))?;
        Self::adopt(
            ffi::read_step(path).map_err(|e| native("read STEP", e))?,
            policy,
            "read STEP",
        )
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;
    use crate::{AffineTransform, CurvedEdge2d, CurvedProfile, RevolutionAxis};
    use multicad_core::{document::EntityId, references::Origin};

    #[test]
    fn legacy_cut_preserves_populated_source_and_shared_bounds_volume_and_validity() {
        let policy = GeometryPolicy::default();
        let original = Solid::cylinder(
            PlaneFrame {
                origin: [0., 3., 0.],
                ..Default::default()
            },
            3.,
            4.,
            policy,
        )
        .unwrap();
        let shared = original.share().unwrap();
        let bounds = original.bounds().unwrap();
        let volume = original.volume_mm3().unwrap();
        assert_eq!(shared.bounds().unwrap(), bounds);
        assert!((volume - 36. * std::f64::consts::PI).abs() < 1e-9);
        assert!(ffi::valid(original.occt().unwrap()).unwrap());
        assert!(ffi::valid(shared.occt().unwrap()).unwrap());

        // Exercise the legacy entry point, rather than the newer boolean API.
        // This removes the complete positive-X half of the offset cylinder.
        let cut = Solid::adopt(
            ffi::cut(shared.occt().unwrap(), 0., 4., 6., 4.).unwrap(),
            policy,
            "legacy cut regression",
        )
        .unwrap();
        assert!((cut.volume_mm3().unwrap() - 18. * std::f64::consts::PI).abs() < 1e-9);
        assert!(cut.bounds().unwrap().max[0].abs() < 1e-6);
        for source in [&original, &shared] {
            assert_eq!(source.bounds().unwrap(), bounds);
            assert!((source.volume_mm3().unwrap() - volume).abs() < 1e-9);
            assert!(ffi::valid(source.occt().unwrap()).unwrap());
            assert_eq!(source.solid_count().unwrap(), 1);
        }
        // A fresh reader has no shared cache, so unchanged cached numbers
        // cannot conceal an accidentally modified source B-rep.
        let fresh = Solid::from_brep(&original.to_brep().unwrap(), policy).unwrap();
        assert!((fresh.volume_mm3().unwrap() - volume).abs() < 1e-9);
        let fresh_bounds = fresh.bounds().unwrap();
        for axis in 0..3 {
            assert!((fresh_bounds.min[axis] - bounds.min[axis]).abs() < 1e-6);
            assert!((fresh_bounds.max[axis] - bounds.max[axis]).abs() < 1e-6);
        }
    }

    #[test]
    fn cold_trimmed_bounds_cache_is_consistent_across_concurrent_shared_native_owners() {
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 0., 1.],
            },
            outer: vec![
                CurvedEdge2d::Arc {
                    center: [0., 0.],
                    a: [0., -1.],
                    b: [0., 1.],
                    clockwise: false,
                },
                CurvedEdge2d::Line {
                    a: [0., 1.],
                    b: [0., -1.],
                },
            ],
            holes: vec![],
        };
        let feature = EntityId::new();
        let sphere = Solid::revolve_curved_profile_named(
            &profile,
            RevolutionAxis {
                origin_mm: [0.; 3],
                direction: [0., 0., 1.],
            },
            std::f64::consts::TAU,
            feature,
            &[
                Origin {
                    source: feature,
                    role: "arc".into(),
                },
                Origin {
                    source: feature,
                    role: "axis".into(),
                },
            ],
            Default::default(),
        )
        .unwrap();
        let ellipsoid = sphere
            .affine_transformed_named(
                AffineTransform {
                    matrix: [[1.2, 0., 0.], [0., 1.4, 0.], [0., 0., 1.6]],
                    translation_mm: [0.; 3],
                },
                EntityId::new(),
                Default::default(),
            )
            .unwrap();
        assert!(ffi::bounds_cache_concurrent_probe(ellipsoid.occt().unwrap()).unwrap());
        let bounds = ellipsoid.bounds().unwrap();
        for (axis, radius) in [1.2, 1.4, 1.6].into_iter().enumerate() {
            assert!((bounds.min[axis] + radius).abs() < 1e-6);
            assert!((bounds.max[axis] - radius).abs() < 1e-6);
        }
        let shared = ellipsoid.share().unwrap();
        assert_eq!(ellipsoid.bounds().unwrap(), shared.bounds().unwrap());
    }
}

#[cfg(test)]
mod volume_cache_tests {
    use super::*;
    use crate::{AffineTransform, HelicalThread};
    use multicad_core::document::EntityId;

    fn integrations(solid: &Solid) -> u64 {
        ffi::volume_integration_count(solid.occt().unwrap()).unwrap()
    }
    fn shares(a: &Solid, b: &Solid) -> bool {
        ffi::shares_volume_cache(a.occt().unwrap(), b.occt().unwrap())
    }

    #[test]
    fn completed_volume_certificates_follow_only_identical_or_exact_rigid_geometry() {
        let policy = GeometryPolicy::default();
        let source = Solid::box_solid([4., 5., 6.], policy).unwrap();
        let initial_bounds = source.bounds().unwrap();
        let shared = source.share().unwrap();
        let moved = shared
            .transformed(RigidTransform {
                rotation: [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]],
                translation_mm: [9., -3., 7.],
            })
            .unwrap();
        assert!(shares(&source, &shared));
        assert!(shares(&source, &moved));
        for solid in [&source, &shared, &moved] {
            for _ in 0..100 {
                assert!((solid.volume_mm3().unwrap() - 120.).abs() < 1e-10);
            }
            assert_eq!(integrations(solid), 1);
        }
        let fresh = Solid::from_brep(&moved.to_brep().unwrap(), policy).unwrap();
        assert!(!shares(&moved, &fresh));
        assert_eq!(integrations(&fresh), 1);
        assert!((fresh.volume_mm3().unwrap() - 120.).abs() < 1e-10);
        let path = tempfile::tempdir().unwrap();
        let step = path.path().join("independent-volume.step");
        moved.write_step(&step).unwrap();
        let readback = Solid::read_step(&step, policy).unwrap();
        assert!(!shares(&moved, &readback));
        assert_eq!(integrations(&readback), 1);
        assert!((readback.volume_mm3().unwrap() - 120.).abs() < 1e-10);

        let tool = Solid::box_solid([2., 5., 6.], policy).unwrap();
        let cut = source
            .boolean(&tool, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap();
        assert!(!shares(&source, &cut));
        assert_eq!(integrations(&cut), 1);
        assert!((cut.volume_mm3().unwrap() - 60.).abs() < 1e-10);
        let scaled = source
            .affine_transformed_named(
                AffineTransform {
                    matrix: [[2., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    translation_mm: [0.; 3],
                },
                EntityId::new(),
                policy,
            )
            .unwrap();
        assert!(!shares(&source, &scaled));
        assert_eq!(integrations(&scaled), 1);
        assert!((scaled.volume_mm3().unwrap() - 240.).abs() < 1e-9);
        let mirrored = source
            .mirrored_named(
                PlaneFrame {
                    origin: [0.; 3],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
                EntityId::new(),
                policy,
            )
            .unwrap();
        assert!(!shares(&source, &mirrored));
        assert_eq!(integrations(&mirrored), 1);
        assert!((mirrored.volume_mm3().unwrap() - 120.).abs() < 1e-10);
        assert_eq!(source.bounds().unwrap(), initial_bounds);
        assert_eq!(integrations(&source), 1);
        assert!((source.volume_mm3().unwrap() - 120.).abs() < 1e-10);
        assert!(ffi::valid(source.occt().unwrap()).unwrap());
    }

    #[test]
    fn cold_native_volume_cache_integrates_once_across_concurrent_shared_owners() {
        let cylinder = Solid::cylinder(Default::default(), 3., 4., Default::default()).unwrap();
        assert!(ffi::volume_cache_concurrent_probe(cylinder.occt().unwrap()).unwrap());
        assert_eq!(integrations(&cylinder), 1);
        assert!((cylinder.volume_mm3().unwrap() - 36. * std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn default_metric_thread_reuses_its_converged_native_volume_certificate() {
        let spec = HelicalThread {
            major_radius_mm: 1.5,
            pitch_mm: 0.5,
            length_mm: 10.,
            depth_mm: 5. * 3_f64.sqrt() / 32.,
            crest_width_mm: 0.5 / 8.,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            radial_taper: 0.,
            starts: 1,
            left_hand: false,
        };
        let solid = Solid::helical_thread_named(spec, EntityId::new(), Default::default()).unwrap();
        assert_eq!(
            integrations(&solid),
            1,
            "Native guard and adopt share one completed integration"
        );
        let initial = solid.volume_mm3().unwrap();
        let shared = solid.share().unwrap();
        let moved = shared.transformed(Default::default()).unwrap();
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            assert_eq!(moved.volume_mm3().unwrap(), initial);
        }
        if std::env::var_os("MULTICAD_NATIVE_THREAD_PROFILE").is_some() {
            eprintln!(
                "cached metric volume: 10000 queries {:?}, one native integration",
                started.elapsed()
            );
        }
        assert_eq!(integrations(&solid), 1);
        assert_eq!(integrations(&moved), 1);
        let fresh = Solid::from_brep(&solid.to_brep().unwrap(), Default::default()).unwrap();
        assert!(!shares(&solid, &fresh));
        let actual = fresh.volume_mm3().unwrap();
        let area: f64 = fresh.faces().unwrap().iter().map(|f| f.area_mm2).sum();
        assert!(
            (actual - initial).abs()
                <= GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * (actual + initial)
        );
    }
}

#[cfg(test)]
mod face_cache_tests {
    use super::*;
    fn count(s: &Solid) -> u64 {
        ffi::face_integration_count(s.occt().unwrap()).unwrap()
    }
    fn shares(a: &Solid, b: &Solid) -> bool {
        ffi::shares_face_cache(a.occt().unwrap(), b.occt().unwrap())
    }
    fn area(s: &Solid) -> f64 {
        s.faces().unwrap().iter().map(|f| f.area_mm2).sum()
    }

    #[test]
    fn completed_face_properties_share_only_identical_geometry_and_recompute_imports_and_edits() {
        let policy = GeometryPolicy::default();
        let source = Solid::box_solid([4., 5., 6.], policy).unwrap();
        let expected = source.faces().unwrap();
        assert_eq!(expected.len(), 6);
        assert!((area(&source) - 148.).abs() < 1e-10);
        let shared = source.share().unwrap();
        assert!(shares(&source, &shared));
        for _ in 0..50 {
            assert_eq!(shared.faces().unwrap(), expected);
        }
        assert_eq!(count(&source), 6);
        let moved = source
            .transformed(RigidTransform {
                rotation: [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]],
                translation_mm: [9., -3., 7.],
            })
            .unwrap();
        assert!(!shares(&source, &moved));
        let moved_faces = moved.faces().unwrap();
        for (before, after) in expected.iter().zip(&moved_faces) {
            assert!((before.area_mm2 - after.area_mm2).abs() < 1e-10);
            let [x, y, z] = before.center_mm;
            for (actual, want) in after.center_mm.into_iter().zip([x + 9., -z - 3., y + 7.]) {
                assert!((actual - want).abs() < 1e-10);
            }
        }
        assert_eq!(count(&moved), 0);
        let fresh = Solid::from_brep(&moved.to_brep().unwrap(), policy).unwrap();
        assert!(!shares(&moved, &fresh));
        assert!((area(&fresh) - 148.).abs() < 1e-10);
        assert_eq!(count(&fresh), 6);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("independent-faces.step");
        moved.write_step(&path).unwrap();
        let read = Solid::read_step(&path, policy).unwrap();
        assert!(!shares(&moved, &read));
        assert!((area(&read) - 148.).abs() < 1e-8);
        assert_eq!(count(&read), 6);
        let tool = Solid::box_solid([2., 5., 6.], policy).unwrap();
        let cut = source
            .boolean(&tool, BooleanOperation::Subtract, policy)
            .unwrap()
            .unwrap();
        assert!(!shares(&source, &cut));
        assert!((area(&cut) - 104.).abs() < 1e-10);
        assert_eq!(source.faces().unwrap(), expected);
        assert_eq!(count(&source), 6);
    }

    #[test]
    fn exact_identity_pose_preserves_numeric_face_cache_without_reusing_owner_metadata() {
        let policy = GeometryPolicy::default();
        let owner = multicad_core::document::EntityId::new();
        let source = Solid::box_named([4., 5., 6.], owner, policy).unwrap();
        let expected = source.faces().unwrap();
        let bounds = source.bounds().unwrap();
        let before = count(&source);
        let identity = source.transformed(RigidTransform::default()).unwrap();
        assert!(shares(&source, &identity));
        assert_eq!(identity.faces().unwrap(), expected);
        assert_eq!(identity.bounds().unwrap(), bounds);
        assert_eq!(
            identity.topology_snapshot(owner, 0).unwrap(),
            source.topology_snapshot(owner, 0).unwrap()
        );
        assert_eq!(count(&identity), before);
        let fresh = Solid::from_brep(&identity.to_brep().unwrap(), policy).unwrap();
        assert!(!shares(&source, &fresh));
        assert_eq!(fresh.faces().unwrap(), expected);
        let translated = source
            .transformed(RigidTransform {
                translation_mm: [1e-10, 0., 0.],
                ..RigidTransform::default()
            })
            .unwrap();
        assert!(!shares(&source, &translated));
        for (old, new) in expected.iter().zip(translated.faces().unwrap()) {
            assert!((new.center_mm[0] - old.center_mm[0] - 1e-10).abs() < 1e-12);
        }
        assert_eq!(source.faces().unwrap(), expected);
        assert_eq!(count(&source), before);
    }

    #[test]
    fn proper_rigid_face_properties_match_independent_native_measurements_and_intents() {
        use multicad_core::document::EntityId;
        let policy = GeometryPolicy::default();
        let owner = EntityId::new();
        let source = Solid::helical_thread_named(
            crate::HelicalThread {
                major_radius_mm: 1.5,
                pitch_mm: 0.5,
                length_mm: 18.,
                depth_mm: 5. * 3_f64.sqrt() / 32.,
                crest_width_mm: 0.5 / 8.,
                lower_flank_deg: 30.,
                upper_flank_deg: 30.,
                radial_taper: 0.,
                starts: 1,
                left_hand: false,
            },
            owner,
            policy,
        )
        .unwrap();
        let original = source.faces().unwrap();
        let original_brep = source.to_brep().unwrap();
        let original_bounds = source.bounds().unwrap();
        let original_volume = source.volume_mm3().unwrap();
        let original_count = count(&source);
        let first = source
            .transformed(RigidTransform {
                rotation: [[0.6, -0.8, 0.], [0.8, 0.6, 0.], [0., 0., 1.]],
                translation_mm: [7., -3., 12.],
            })
            .unwrap();
        let composed = first
            .transformed(RigidTransform {
                rotation: [[1., 0., 0.], [0., 0.6, -0.8], [0., 0.8, 0.6]],
                translation_mm: [-2., 8., 5.],
            })
            .unwrap();
        for moved in [&first, &composed] {
            assert!(!shares(&source, moved));
            assert_eq!(
                count(moved),
                0,
                "Proper rigid descriptor transport must avoid repeated quadrature"
            );
            let fresh = Solid::from_brep(&moved.to_brep().unwrap(), policy).unwrap();
            let cached = moved.faces().unwrap();
            let measured = fresh.faces().unwrap();
            assert_eq!(cached.len(), measured.len());
            for (a, b) in cached.iter().zip(&measured) {
                assert_eq!(a.index, b.index);
                assert_eq!(
                    a.plane, b.plane,
                    "Planar descriptors must use the unchanged output-native path"
                );
                assert!((a.area_mm2 - b.area_mm2).abs() < 1e-7 * (a.area_mm2 + b.area_mm2));
                for (x, y) in a.center_mm.into_iter().zip(b.center_mm) {
                    assert!((x - y).abs() < policy.linear_tolerance_mm);
                }
            }
            assert_eq!(count(moved), 0);
            let mut rebound = fresh.share().unwrap();
            // BRep transport carries no owner metadata. Reapply the already
            // verified ordinal receipt only to compare independent native
            // measurement with the same persistent reference input.
            rebound.provenance = moved.provenance.clone();
            let cached_refs = moved.topology_snapshot(owner, 0).unwrap();
            let measured_refs = rebound.topology_snapshot(owner, 0).unwrap();
            assert_eq!(cached_refs.elements.len(), measured_refs.elements.len());
            for (a, b) in cached_refs.elements.iter().zip(&measured_refs.elements) {
                assert_eq!(a.key, b.key);
                assert_eq!(a.origins, b.origins);
                assert_eq!(a.boundary, b.boundary);
                assert_eq!(a.frames, b.frames, "Anchored frame differs at {:?}", a.key);
                if a.key.kind == multicad_core::references::TopologyKind::Face {
                    assert_eq!(
                        a.geometry, b.geometry,
                        "Native plane descriptor differs at {:?}",
                        a.key
                    );
                }
            }
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("rigid-thread.step");
            moved.write_step(&path).unwrap();
            let step = Solid::read_step(&path, policy).unwrap();
            for (a, b) in moved
                .bounds()
                .unwrap()
                .min
                .into_iter()
                .chain(moved.bounds().unwrap().max)
                .zip(
                    step.bounds()
                        .unwrap()
                        .min
                        .into_iter()
                        .chain(step.bounds().unwrap().max),
                )
            {
                assert!((a - b).abs() < policy.linear_tolerance_mm * 2.);
            }
            let area: f64 = cached.iter().map(|f| f.area_mm2).sum();
            let bound = 2. * policy.linear_tolerance_mm * area
                + 1e-7 * (step.volume_mm3().unwrap() + moved.volume_mm3().unwrap());
            assert!((step.volume_mm3().unwrap() - moved.volume_mm3().unwrap()).abs() < bound);
        }
        assert!(
            source
                .transformed(RigidTransform {
                    translation_mm: [f64::NAN, 0., 0.],
                    ..Default::default()
                })
                .is_err()
        );
        assert_eq!(source.to_brep().unwrap(), original_brep);
        assert_eq!(source.bounds().unwrap(), original_bounds);
        assert_eq!(source.volume_mm3().unwrap(), original_volume);
        assert_eq!(source.faces().unwrap(), original);
        assert_eq!(count(&source), original_count);
        let box_source = Solid::box_named([4., 5., 6.], EntityId::new(), policy).unwrap();
        let scaled = box_source
            .affine_transformed_named(
                crate::AffineTransform {
                    matrix: [[2., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    translation_mm: [0.; 3],
                },
                EntityId::new(),
                policy,
            )
            .unwrap();
        assert!(!shares(&box_source, &scaled));
        assert_eq!(count(&scaled), 6);
        let mirrored = box_source
            .mirrored_named(
                PlaneFrame {
                    origin: [0.; 3],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
                EntityId::new(),
                policy,
            )
            .unwrap();
        assert!(!shares(&box_source, &mirrored));
        assert_eq!(count(&mirrored), 6);
    }

    #[test]
    fn cold_face_cache_serializes_eight_native_queries_without_crossing_rust_ownership() {
        let solid = Solid::cylinder(Default::default(), 3., 4., Default::default()).unwrap();
        assert!(ffi::face_cache_concurrent_probe(solid.occt().unwrap()).unwrap());
        assert!((area(&solid) - 42. * std::f64::consts::PI).abs() < 1e-9);
        let initial = count(&solid);
        for _ in 0..100 {
            assert_eq!(solid.faces().unwrap().len(), 3);
        }
        assert_eq!(count(&solid), initial);
    }
}

#[cfg(test)]
mod exact_geometry_digest_tests {
    use super::*;
    use multicad_core::document::EntityId;

    #[test]
    fn exact_geometry_digest_is_owner_independent_stable_and_display_independent() {
        let policy = GeometryPolicy::default();
        let owner = EntityId::new();
        let source = Solid::box_named([4., 5., 6.], owner, policy).unwrap();
        let bytes = source.to_brep().unwrap();
        let digest = source.exact_geometry_digest().unwrap();
        assert_eq!(
            source.share().unwrap().exact_geometry_digest().unwrap(),
            digest
        );
        assert_eq!(
            source
                .rebind_exclusive_owner(owner, EntityId::new())
                .unwrap()
                .exact_geometry_digest()
                .unwrap(),
            digest
        );
        let independent = Solid::box_named([4., 5., 6.], EntityId::new(), policy).unwrap();
        assert_eq!(independent.exact_geometry_digest().unwrap(), digest);
        let _ = source.mesh(MeshOptions::default()).unwrap();
        assert_eq!(source.exact_geometry_digest().unwrap(), digest);
        assert_eq!(source.to_brep().unwrap(), bytes);
        let changed = Solid::box_named([4., 5., 6.0001], owner, policy).unwrap();
        assert_ne!(changed.exact_geometry_digest().unwrap(), digest);
    }

    #[test]
    fn exact_geometry_digest_retains_translation_bits_lost_by_ascii_and_rotation() {
        let source = Solid::box_solid([4., 5., 6.], GeometryPolicy::default()).unwrap();
        let before = source.to_brep().unwrap();
        let moved = |x| {
            source
                .transformed(RigidTransform {
                    translation_mm: [x, 0., 0.],
                    ..Default::default()
                })
                .unwrap()
        };
        let a = moved(1.);
        let b = moved(f64::from_bits(1f64.to_bits() + 1));
        assert_eq!(
            a.to_brep().unwrap(),
            b.to_brep().unwrap(),
            "Pinned ASCII15 output must demonstrate the unsafe key collision"
        );
        assert_ne!(
            a.exact_geometry_digest().unwrap(),
            b.exact_geometry_digest().unwrap()
        );
        let angle = 0.123f64;
        let (s, c) = angle.sin_cos();
        let rotated = source
            .transformed(RigidTransform {
                rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
                ..Default::default()
            })
            .unwrap();
        assert_ne!(
            rotated.exact_geometry_digest().unwrap(),
            source.exact_geometry_digest().unwrap()
        );
        assert_eq!(source.to_brep().unwrap(), before);
    }

    #[test]
    fn exact_geometry_digest_includes_native_tolerance_and_curved_surfaces() {
        let policy = GeometryPolicy::default();
        let source = Solid::cylinder(PlaneFrame::default(), 3., 4., policy).unwrap();
        let before = source.to_brep().unwrap();
        let digest = source.exact_geometry_digest().unwrap();
        let independent = Solid::cylinder(PlaneFrame::default(), 3., 4., policy).unwrap();
        assert_eq!(independent.exact_geometry_digest().unwrap(), digest);
        let _ = source.mesh(MeshOptions::default()).unwrap();
        assert_eq!(source.exact_geometry_digest().unwrap(), digest);
        // Change only an actual stored vertex tolerance in the pinned public
        // BRep codec. No geometry constructor or tolerance policy is changed.
        let start = before.find("\nVe\n").unwrap() + 4;
        let end = start + before[start..].find('\n').unwrap();
        let tolerance: f64 = before[start..end].parse().unwrap();
        let mut edited = before.clone();
        edited.replace_range(start..end, &(tolerance * 2.).to_string());
        let changed = Solid::from_brep(&edited, policy).unwrap();
        assert_ne!(changed.exact_geometry_digest().unwrap(), digest);
        assert!((changed.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs() < 1e-10);
        assert_eq!(source.to_brep().unwrap(), before);
    }

    #[test]
    fn exact_geometry_digest_refuses_partial_or_unbounded_serialization() {
        let source = Solid::box_solid([4., 5., 6.], GeometryPolicy::default()).unwrap();
        let before = source.to_brep().unwrap();
        let complete = source.exact_geometry_digest().unwrap();
        for limit in [0, 16, 128, 16 * 1024 * 1024 + 1] {
            assert!(ffi::shape_binary_geometry(source.occt().unwrap(), limit).is_err());
            assert_eq!(source.exact_geometry_digest().unwrap(), complete);
        }
        assert_eq!(source.to_brep().unwrap(), before);
    }
}
