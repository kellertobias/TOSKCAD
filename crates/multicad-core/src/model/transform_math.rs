//! Pure rigid placement math shared by source commands and native evaluators.
use super::{
    angle,
    common::{cardinal_sin_cos, number},
    length,
    source::invalid,
};
use crate::{
    constraint_sketch::Dimension,
    document::{DocumentError, Placement},
    parameters::{EvaluatedParameters, Unit},
    references::ReferenceFrame,
};
use serde::{Deserialize, Serialize};

type Point = [f64; 3];
fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn bounded(point: Point) -> bool {
    point.iter().all(|x| x.is_finite() && x.abs() <= 1e9)
}

/// Derived rigid pose. A rotation matrix avoids converting exact cardinal
/// controls through an approximate quaternion representation during evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RigidPose {
    pub rotation: [[f64; 3]; 3],
    pub translation_mm: Point,
}
impl Default for RigidPose {
    fn default() -> Self {
        Self {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [0.; 3],
        }
    }
}
impl RigidPose {
    pub fn validate(self) -> Result<(), DocumentError> {
        if !bounded(self.translation_mm) || self.rotation.iter().flatten().any(|x| !x.is_finite()) {
            return Err(invalid(
                "Rigid placement has nonfinite or out-of-range coordinates",
            ));
        }
        for i in 0..3 {
            for j in 0..3 {
                if (dot(self.rotation[i], self.rotation[j]) - if i == j { 1. } else { 0. }).abs()
                    > 1e-9
                {
                    return Err(invalid(
                        "Placement rotation must be orthonormal; scale and shear are unsupported",
                    ));
                }
            }
        }
        if (dot(cross(self.rotation[0], self.rotation[1]), self.rotation[2]) - 1.).abs() > 1e-9 {
            return Err(invalid(
                "Placement must be a proper rotation, without reflection",
            ));
        }
        Ok(())
    }
    pub fn from_placement(pose: Placement) -> Result<Self, DocumentError> {
        let norm = pose.rotation_xyzw.iter().map(|n| n * n).sum::<f64>();
        if !norm.is_finite() || (norm - 1.).abs() > 1e-9 {
            return Err(invalid("Legacy placement requires a unit quaternion"));
        }
        let [x, y, z, w] = pose.rotation_xyzw.map(|n| n / norm.sqrt());
        let result = Self {
            rotation: [
                [
                    1. - 2. * (y * y + z * z),
                    2. * (x * y - z * w),
                    2. * (x * z + y * w),
                ],
                [
                    2. * (x * y + z * w),
                    1. - 2. * (x * x + z * z),
                    2. * (y * z - x * w),
                ],
                [
                    2. * (x * z - y * w),
                    2. * (y * z + x * w),
                    1. - 2. * (x * x + y * y),
                ],
            ],
            translation_mm: pose.translation_mm,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn vector(self, vector: Point) -> Point {
        self.rotation.map(|row| dot(row, vector))
    }
    pub fn point(self, point: Point) -> Point {
        let rotated = self.vector(point);
        std::array::from_fn(|i| rotated[i] + self.translation_mm[i])
    }
    pub fn frame(self, frame: ReferenceFrame) -> ReferenceFrame {
        ReferenceFrame {
            origin_mm: self.point(frame.origin_mm),
            u: self.vector(frame.u),
            v: self.vector(frame.v),
        }
    }
    /// Apply this pose, then `next`. Composition never modifies source points.
    pub fn then(self, next: Self) -> Result<Self, DocumentError> {
        self.validate()?;
        next.validate()?;
        let result = Self {
            rotation: std::array::from_fn(|i| {
                std::array::from_fn(|j| {
                    (0..3)
                        .map(|k| next.rotation[i][k] * self.rotation[k][j])
                        .sum()
                })
            }),
            translation_mm: next.point(self.translation_mm),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn inverse(self) -> Result<Self, DocumentError> {
        self.validate()?;
        let rotation = std::array::from_fn(|i| std::array::from_fn(|j| self.rotation[j][i]));
        Ok(Self {
            rotation,
            translation_mm: rotation.map(|row| -dot(row, self.translation_mm)),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumericTransform {
    pub translation: [Dimension; 3],
    pub pivot: [Dimension; 3],
    /// Axis direction in the input's coordinate system; magnitude is immaterial.
    pub axis: Point,
    pub angle: Dimension,
}
impl Default for NumericTransform {
    fn default() -> Self {
        Self {
            translation: std::array::from_fn(|_| length(0.)),
            pivot: std::array::from_fn(|_| length(0.)),
            axis: [0., 0., 1.],
            angle: angle(0.),
        }
    }
}
impl NumericTransform {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        self.translation
            .iter()
            .chain(&self.pivot)
            .chain(std::iter::once(&self.angle))
            .collect()
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        for dimension in self.translation.iter().chain(&self.pivot) {
            super::source::check_dimension(dimension, Unit::Millimeter)?;
        }
        super::source::check_dimension(&self.angle, Unit::Radian)?;
        if !bounded(self.axis) || dot(self.axis, self.axis).sqrt() < 1e-12 {
            return Err(invalid("Choose a finite nonzero rotation axis"));
        }
        Ok(())
    }
    /// Rotate around the explicit pivot first, then translate. Units and stable
    /// parameter bindings remain authored data rather than baked coordinates.
    pub fn resolve(&self, parameters: &EvaluatedParameters) -> Result<RigidPose, DocumentError> {
        self.validate()?;
        let translation = self
            .translation
            .each_ref()
            .map(|d| number(d, Unit::Millimeter, parameters))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let pivot = self
            .pivot
            .each_ref()
            .map(|d| number(d, Unit::Millimeter, parameters))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let pivot: [f64; 3] = pivot.try_into().expect("three pivot coordinates");
        let angle = number(&self.angle, Unit::Radian, parameters)?;
        let length = dot(self.axis, self.axis).sqrt();
        let axis = self.axis.map(|x| x / length);
        let (sin, cos) = cardinal_sin_cos(angle);
        let [x, y, z] = axis;
        let skew = [[0., -z, y], [z, 0., -x], [-y, x, 0.]];
        let rotation = std::array::from_fn(|i| {
            std::array::from_fn(|j|if i==j {cos}else{0.}+(1.-cos)*axis[i]*axis[j]+sin*skew[i][j])
        });
        let mut pose = RigidPose {
            rotation,
            translation_mm: [0.; 3],
        };
        let rotated = pose.vector(pivot);
        pose.translation_mm = std::array::from_fn(|i| translation[i] + pivot[i] - rotated[i]);
        pose.validate()?;
        Ok(pose)
    }
}

/// Align complete explicit reference frames. No nearest face or arbitrary
/// inherited frame is selected here; the caller must supply resolved anchors.
pub fn align_frames(
    source: ReferenceFrame,
    target: ReferenceFrame,
    offset_mm: Point,
    roll_rad: f64,
    opposed: bool,
) -> Result<RigidPose, DocumentError> {
    source.validate().map_err(invalid)?;
    target.validate().map_err(invalid)?;
    if !bounded(offset_mm) || !roll_rad.is_finite() || roll_rad.abs() > 1e9 {
        return Err(invalid(
            "Alignment offset/roll is nonfinite or outside limits",
        ));
    }
    let (sin, cos) = cardinal_sin_cos(roll_rad);
    let u = std::array::from_fn(|i| target.u[i] * cos + target.v[i] * sin);
    let sign = if opposed { -1. } else { 1. };
    let v = std::array::from_fn(|i| (-target.u[i] * sin + target.v[i] * cos) * sign);
    let n = cross(u, v);
    let from = [source.u, source.v, source.normal()];
    let to = [u, v, n];
    let rotation = std::array::from_fn(|i| {
        std::array::from_fn(|j| (0..3).map(|k| to[k][i] * from[k][j]).sum())
    });
    let mut pose = RigidPose {
        rotation,
        translation_mm: [0.; 3],
    };
    let origin: Point = std::array::from_fn(|i| {
        target.origin_mm[i]
            + target.u[i] * offset_mm[0]
            + target.v[i] * offset_mm[1]
            + target.normal()[i] * offset_mm[2]
    });
    let rotated = pose.vector(source.origin_mm);
    pose.translation_mm = std::array::from_fn(|i| origin[i] - rotated[i]);
    pose.validate()?;
    Ok(pose)
}
