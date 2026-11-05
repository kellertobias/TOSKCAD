//! Safe solid operations in millimeters. Display meshes are detached copies;
//! callers rebuild exact solids from source rather than editing triangles.
use crate::{Shape, ffi};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryErrorKind {
    Input,
    Native,
    InvalidResult,
    Limit,
    Cancelled,
    Stale,
    Busy,
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
/// Opaque owned B-rep. It is deliberately not Send/Sync; a worker must construct,
/// use and destroy its native handles on its own thread.
/// ```compile_fail
/// fn requires_send<T: Send>() {}
/// requires_send::<multicad_kernel::Solid>();
/// ```
/// ```compile_fail
/// fn requires_sync<T: Sync>() {}
/// requires_sync::<multicad_kernel::Solid>();
/// ```
pub struct Solid {
    pub(crate) native: Shape,
    policy: GeometryPolicy,
    pub(crate) provenance: Vec<multicad_core::references::TopologyElement>,
}
impl Solid {
    pub fn geometry_policy(&self) -> GeometryPolicy {
        self.policy
    }
    /// Another owner of the same immutable exact B-rep (and its provenance),
    /// without copying geometry. Kernel operations never modify their input
    /// shapes, so both owners observe identical geometry. Like every Solid
    /// the share stays on the thread that created it (`!Send`/`!Sync`).
    pub fn share(&self) -> Result<Self, GeometryError> {
        let handle = ffi::share_shape(&self.native.0).map_err(|e| native("share", e))?;
        if handle.is_null() {
            return Err(error(
                "share",
                GeometryErrorKind::InvalidResult,
                "Kernel returned an empty shape",
            ));
        }
        Ok(Self {
            native: Shape(handle),
            policy: self.policy,
            provenance: self.provenance.clone(),
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
            native: Shape(handle),
            policy,
            provenance: vec![],
        };
        if !ffi::valid(&solid.native.0).map_err(|e| native(operation, e))?
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
        let op = match operation {
            BooleanOperation::Union => 0,
            BooleanOperation::Subtract => 1,
            BooleanOperation::Intersect => 2,
        };
        let handle =
            ffi::boolean_shapes(&self.native.0, &other.native.0, op, policy.boolean_fuzzy_mm)
                .map_err(|e| native("boolean", e))?;
        if handle.is_null() {
            return Ok(None);
        }
        let mut result = Self::adopt(handle, policy, "boolean")?;
        result.inherit_provenance(&[self, other], None)?;
        Ok(Some(result))
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
        let handle =
            ffi::intersect_parallel(&self.native.0, &other.native.0, policy.boolean_fuzzy_mm)
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
        let r: Vec<_> = transform.rotation.into_iter().flatten().collect();
        let mut result = Self::adopt(
            ffi::transform_shape(&self.native.0, &r, &point(transform.translation_mm))
                .map_err(|e| native("transform", e))?,
            self.policy,
            "transform",
        )?;
        result.inherit_provenance(&[self], Some(transform))?;
        Ok(result)
    }
    pub fn volume_mm3(&self) -> Result<f64, GeometryError> {
        ffi::volume(&self.native.0).map_err(|e| native("volume", e))
    }
    pub fn solid_count(&self) -> Result<u32, GeometryError> {
        ffi::solid_count(&self.native.0).map_err(|e| native("solid count", e))
    }
    pub fn bounds(&self) -> Result<Bounds, GeometryError> {
        let b = ffi::bounds(&self.native.0).map_err(|e| native("bounds", e))?;
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
        ffi::faces(&self.native.0)
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
    pub fn mesh(&self, options: MeshOptions) -> Result<ffi::Mesh, GeometryError> {
        if !options.linear_deflection_mm.is_finite()
            || options.linear_deflection_mm < self.policy.linear_tolerance_mm.max(0.001)
            || options.linear_deflection_mm > self.policy.maximum_coordinate_mm
            || !options.angular_deflection_rad.is_finite()
            || !(0.01..=1.).contains(&options.angular_deflection_rad)
            || options.maximum_triangles == 0
            || options.maximum_triangles > 5_000_000
        {
            return Err(input("Invalid display tessellation settings/budget"));
        }
        let mesh = ffi::tessellate(
            &self.native.0,
            options.linear_deflection_mm,
            options.angular_deflection_rad,
            options.maximum_triangles,
        )
        .map_err(|e| native("mesh", e))?;
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
        let _ = ffi::remember_validated_mesh(
            &self.native.0,
            options.linear_deflection_mm,
            options.angular_deflection_rad,
            options.maximum_triangles,
            &mesh,
        );
        Ok(mesh)
    }
    /// Manufacturing-only triangulation with native-topology seam identity.
    /// Display meshes and their cache remain unchanged. No proximity welds.
    pub fn manufacturing_mesh(&self, options: MeshOptions) -> Result<ffi::Mesh, GeometryError> {
        Ok(self.manufacturing_mesh_with_receipt(options)?.mesh)
    }
    pub fn manufacturing_mesh_with_receipt(
        &self,
        options: MeshOptions,
    ) -> Result<crate::ManufacturingMesh, GeometryError> {
        if !options.linear_deflection_mm.is_finite()
            || options.linear_deflection_mm < self.policy.linear_tolerance_mm.max(0.001)
            || options.linear_deflection_mm > self.policy.maximum_coordinate_mm
            || !options.angular_deflection_rad.is_finite()
            || !(0.01..=1.).contains(&options.angular_deflection_rad)
            || options.maximum_triangles == 0
            || options.maximum_triangles > 5_000_000
        {
            return Err(input("Invalid manufacturing tessellation settings/budget"));
        }
        let tolerance = self
            .policy
            .linear_tolerance_mm
            .min(options.linear_deflection_mm / 16.);
        let receipt = ffi::tessellate_manufacturing(
            &self.native.0,
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
    pub fn display_edges(&self, deflection_mm: f64) -> Result<Vec<DisplayEdge>, GeometryError> {
        if !deflection_mm.is_finite()
            || deflection_mm < 0.001
            || deflection_mm > self.policy.maximum_coordinate_mm
        {
            return Err(input("Invalid edge display deflection"));
        }
        let edges = ffi::display_edges(&self.native.0, deflection_mm)
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
        ffi::vertices(&self.native.0)
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
    pub fn write_step(&self, path: &Path) -> Result<(), GeometryError> {
        let path = crate::step_path(path).map_err(|e| input(e.to_string()))?;
        ffi::write_step(&self.native.0, path).map_err(|e| native("write STEP", e))
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
        ffi::shape_to_brep(&self.native.0).map_err(|e| native("write BRep", e))
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
        assert!(ffi::valid(&original.native.0).unwrap());
        assert!(ffi::valid(&shared.native.0).unwrap());

        // Exercise the legacy entry point, rather than the newer boolean API.
        // This removes the complete positive-X half of the offset cylinder.
        let cut = Solid::adopt(
            ffi::cut(&shared.native.0, 0., 4., 6., 4.).unwrap(),
            policy,
            "legacy cut regression",
        )
        .unwrap();
        assert!((cut.volume_mm3().unwrap() - 18. * std::f64::consts::PI).abs() < 1e-9);
        assert!(cut.bounds().unwrap().max[0].abs() < 1e-6);
        for source in [&original, &shared] {
            assert_eq!(source.bounds().unwrap(), bounds);
            assert!((source.volume_mm3().unwrap() - volume).abs() < 1e-9);
            assert!(ffi::valid(&source.native.0).unwrap());
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
        assert!(ffi::bounds_cache_concurrent_probe(&ellipsoid.native.0).unwrap());
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
        ffi::volume_integration_count(&solid.native.0).unwrap()
    }
    fn shares(a: &Solid, b: &Solid) -> bool {
        ffi::shares_volume_cache(&a.native.0, &b.native.0)
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
        assert!(ffi::valid(&source.native.0).unwrap());
    }

    #[test]
    fn cold_native_volume_cache_integrates_once_across_concurrent_shared_owners() {
        let cylinder = Solid::cylinder(Default::default(), 3., 4., Default::default()).unwrap();
        assert!(ffi::volume_cache_concurrent_probe(&cylinder.native.0).unwrap());
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
        ffi::face_integration_count(&s.native.0).unwrap()
    }
    fn shares(a: &Solid, b: &Solid) -> bool {
        ffi::shares_face_cache(&a.native.0, &b.native.0)
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
        assert!(ffi::face_cache_concurrent_probe(&solid.native.0).unwrap());
        assert!((area(&solid) - 42. * std::f64::consts::PI).abs() < 1e-9);
        let initial = count(&solid);
        for _ in 0..100 {
            assert_eq!(solid.faces().unwrap().len(), 3);
        }
        assert_eq!(count(&solid), initial);
    }
}
