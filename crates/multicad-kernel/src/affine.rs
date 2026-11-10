//! Exact orientation-preserving affine solid operations, separate from poses.
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, Solid, ffi};
use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, ReferenceFrame},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AffineTransform {
    pub matrix: [[f64; 3]; 3],
    pub translation_mm: [f64; 3],
}
impl Default for AffineTransform {
    fn default() -> Self {
        Self {
            matrix: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [0.; 3],
        }
    }
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "affine transform",
        kind,
        message: message.into(),
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
fn normalized(v: [f64; 3]) -> Result<[f64; 3], GeometryError> {
    let length = dot(v, v).sqrt();
    if !length.is_finite() || length <= 0. {
        return Err(failure(
            GeometryErrorKind::InvalidResult,
            "Affine frame has a degenerate direction",
        ));
    }
    Ok(v.map(|x| x / length))
}
impl AffineTransform {
    fn cofactors(self) -> [[f64; 3]; 3] {
        [
            cross(self.matrix[1], self.matrix[2]),
            cross(self.matrix[2], self.matrix[0]),
            cross(self.matrix[0], self.matrix[1]),
        ]
    }
    fn determinant(self) -> f64 {
        dot(self.matrix[0], cross(self.matrix[1], self.matrix[2]))
    }
    /// Matrix and inverse coefficients are bounded to 1e6. Positive determinant
    /// permits general affine geometry while excluding reflection and collapse.
    pub fn validate(self) -> Result<(), GeometryError> {
        GeometryPolicy::default().point(self.translation_mm)?;
        if self
            .matrix
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1e6)
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Affine matrix coefficients must be finite and at most 1e6 in magnitude",
            ));
        }
        let determinant = self.determinant();
        if !determinant.is_finite()
            || determinant <= 0.
            || self.cofactors().iter().flatten().any(|v| {
                !(*v / determinant).is_finite() || (*v / determinant).abs() > 1e6 * (1. + 1e-12)
            })
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Affine matrix must have a positive nonsingular determinant and bounded inverse",
            ));
        }
        Ok(())
    }
    fn vector(self, v: [f64; 3]) -> [f64; 3] {
        self.matrix.map(|row| dot(row, v))
    }
    fn point(self, p: [f64; 3]) -> [f64; 3] {
        let p = self.vector(p);
        std::array::from_fn(|i| p[i] + self.translation_mm[i])
    }
    pub(crate) fn frame(self, frame: ReferenceFrame) -> Result<ReferenceFrame, GeometryError> {
        frame
            .validate()
            .map_err(|e| failure(GeometryErrorKind::InvalidResult, e))?;
        // Cofactors / determinant are the inverse transpose. Its positive
        // scalar determinant can be omitted before normalizing the normal.
        let n = normalized(self.cofactors().map(|row| dot(row, frame.normal())))?;
        let u = normalized(self.vector(frame.u))?;
        let v = normalized(cross(n, u))?;
        let u = normalized(cross(v, n))?;
        let frame = ReferenceFrame {
            origin_mm: self.point(frame.origin_mm),
            u,
            v,
        };
        frame
            .validate()
            .map_err(|e| failure(GeometryErrorKind::InvalidResult, e))?;
        Ok(frame)
    }
    pub(crate) fn analytic(
        self,
        value: AnalyticGeometry,
    ) -> Result<Option<AnalyticGeometry>, GeometryError> {
        let value = match value {
            AnalyticGeometry::Point { position_mm } => AnalyticGeometry::Point {
                position_mm: self.point(position_mm),
            },
            AnalyticGeometry::Line { a_mm, b_mm } => AnalyticGeometry::Line {
                a_mm: self.point(a_mm),
                b_mm: self.point(b_mm),
            },
            AnalyticGeometry::Plane { frame } => AnalyticGeometry::Plane {
                frame: self.frame(frame)?,
            },
            AnalyticGeometry::CircularArc { .. } | AnalyticGeometry::EllipticArc { .. } => {
                return Ok(None);
            }
        };
        value
            .validate()
            .map_err(|e| failure(GeometryErrorKind::InvalidResult, e))?;
        Ok(Some(value))
    }
}
impl Solid {
    pub fn affine_transformed_named(
        &self,
        transform: AffineTransform,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        transform.validate()?;
        policy.point(transform.translation_mm)?;
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Affine feature identity is nil",
            ));
        }
        let matrix: Vec<_> = transform.matrix.into_iter().flatten().collect();
        let mut result = Self::adopt(
            ffi::affine_transform_shape(
                self.occt()?,
                &matrix,
                &ffi::Point3 {
                    x: transform.translation_mm[0],
                    y: transform.translation_mm[1],
                    z: transform.translation_mm[2],
                },
            )
            .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?,
            policy,
            "affine transform",
        )?;
        result.inherit_affine_provenance(self, transform, feature)?;
        Ok(result)
    }
}
