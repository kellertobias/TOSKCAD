use crate::{
    constraint_sketch::Dimension,
    document::DocumentError,
    model::{
        RigidPose,
        common::number,
        length,
        source::{check_dimension, invalid},
    },
    parameters::{EvaluatedParameters, Unit},
    references::ReferenceFrame,
};
use serde::{Deserialize, Serialize};

pub fn ratio(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "ratio".into(),
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleParameters {
    pub factors: [Dimension; 3],
    pub pivot: [Dimension; 3],
    /// Orthonormal world-space directions captured by the selected coordinate
    /// system. The pivot is independent of this basis's orientation.
    pub axes: [[f64; 3]; 3],
}
impl Default for ScaleParameters {
    fn default() -> Self {
        Self {
            factors: std::array::from_fn(|_| ratio(1.)),
            pivot: std::array::from_fn(|_| length(0.)),
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedScale {
    pub factors: [f64; 3],
    pub pivot_mm: [f64; 3],
    pub axes: [[f64; 3]; 3],
}
impl ScaleParameters {
    pub(super) fn dimensions(&self) -> Vec<&Dimension> {
        self.factors.iter().chain(self.pivot.iter()).collect()
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        let frame = ReferenceFrame {
            origin_mm: [0.; 3],
            u: self.axes[0],
            v: self.axes[1],
        };
        frame.validate().map_err(invalid)?;
        if frame
            .normal()
            .into_iter()
            .zip(self.axes[2])
            .any(|(a, b)| !b.is_finite() || (a - b).abs() > 1e-9)
        {
            return Err(invalid(
                "Scale coordinates must be right-handed and orthonormal",
            ));
        }
        for factor in &self.factors {
            check_dimension(factor, Unit::Ratio)?;
            if let Dimension::Literal { value, .. } = factor
                && *value <= 0.
            {
                return Err(invalid(
                    "Scale factors must be positive; zero and mirror factors are unsupported",
                ));
            }
        }
        for pivot in &self.pivot {
            check_dimension(pivot, Unit::Millimeter)?;
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<ResolvedScale, DocumentError> {
        self.validate()?;
        let result = ResolvedScale {
            factors: [
                number(&self.factors[0], Unit::Ratio, parameters)?,
                number(&self.factors[1], Unit::Ratio, parameters)?,
                number(&self.factors[2], Unit::Ratio, parameters)?,
            ],
            pivot_mm: [
                number(&self.pivot[0], Unit::Millimeter, parameters)?,
                number(&self.pivot[1], Unit::Millimeter, parameters)?,
                number(&self.pivot[2], Unit::Millimeter, parameters)?,
            ],
            axes: self.axes,
        };
        result.validate()?;
        Ok(result)
    }
}
impl ResolvedScale {
    pub fn validate(self) -> Result<(), DocumentError> {
        if self.factors.into_iter().any(|v| !v.is_finite() || v <= 0.)
            || !self.factors.into_iter().product::<f64>().is_finite()
            || self.factors.into_iter().product::<f64>() <= 0.
        {
            return Err(invalid(
                "Scale factors must form a finite positive nonsingular transform",
            ));
        }
        ScaleParameters {
            factors: self.factors.map(ratio),
            pivot: self.pivot_mm.map(length),
            axes: self.axes,
        }
        .validate()
    }
    pub fn affine(self) -> Result<([[f64; 3]; 3], [f64; 3]), DocumentError> {
        self.validate()?;
        let matrix = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                (0..3)
                    .map(|axis| self.axes[axis][i] * self.factors[axis] * self.axes[axis][j])
                    .sum::<f64>()
            })
        });
        let translation = std::array::from_fn(|i| {
            self.pivot_mm[i] - (0..3).map(|j| matrix[i][j] * self.pivot_mm[j]).sum::<f64>()
        });
        if matrix
            .iter()
            .flatten()
            .chain(translation.iter())
            .any(|v| !v.is_finite())
        {
            return Err(invalid("Scale matrix or pivot is out of range"));
        }
        Ok((matrix, translation))
    }
    /// Express a world scale in the local coordinates of an unchanged rigid
    /// occurrence. Its component geometry changes; the occurrence pose stays rigid.
    pub fn in_pose(self, pose: RigidPose) -> Result<Self, DocumentError> {
        self.validate()?;
        let inverse = pose.inverse()?;
        let result = Self {
            factors: self.factors,
            pivot_mm: inverse.point(self.pivot_mm),
            axes: self.axes.map(|axis| inverse.vector(axis)),
        };
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{Document, Entity, EntityData},
        model::{NumericTransform, angle},
        parameters::{self, BoundExpression},
    };
    fn point(affine: ([[f64; 3]; 3], [f64; 3]), p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (0..3).map(|j| affine.0[i][j] * p[j]).sum::<f64>() + affine.1[i])
    }
    fn close(a: [f64; 3], b: [f64; 3]) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < 1e-8, "{a} != {b}");
        }
    }
    #[test]
    fn positive_world_frame_scale_and_local_conjugation_preserve_explicit_pivot() {
        let p = parameters::evaluate(&Document::new("Scale math").unwrap()).unwrap();
        let scale = ScaleParameters {
            factors: [ratio(2.), ratio(3.), ratio(4.)],
            pivot: [length(7.), length(-9.), length(2.)],
            axes: [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]],
        }
        .resolve(&p)
        .unwrap();
        close(
            point(scale.affine().unwrap(), [8., -7., 5.]),
            [10., -5., 14.],
        );
        close(
            point(scale.affine().unwrap(), scale.pivot_mm),
            scale.pivot_mm,
        );
        let pose = NumericTransform {
            translation: [length(30.), length(40.), length(-10.)],
            axis: [1., 2., 3.],
            angle: angle(37.),
            ..Default::default()
        }
        .resolve(&p)
        .unwrap();
        let local = scale.in_pose(pose).unwrap();
        for p in [[0.; 3], [1., 2., 3.], [-5., 8., 10.]] {
            close(
                pose.point(point(local.affine().unwrap(), p)),
                point(scale.affine().unwrap(), pose.point(p)),
            );
        }
    }
    #[test]
    fn ratio_formulas_rebuild_by_identity_and_invalid_factors_are_rejected() {
        let mut d = Document::new("Scale factors").unwrap();
        let parameter = Entity::new(
            "factor",
            EntityData::Parameter {
                expression: "150".into(),
                unit: "%".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let id = parameter.id;
        d.insert(parameter).unwrap();
        let expression = BoundExpression::bind(&d, "=$factor * 2", "ratio", None).unwrap();
        let scale = ScaleParameters {
            factors: std::array::from_fn(|_| Dimension::Expression {
                expression: expression.clone(),
            }),
            ..Default::default()
        };
        assert_eq!(
            scale
                .resolve(&parameters::evaluate(&d).unwrap())
                .unwrap()
                .factors,
            [3.; 3]
        );
        assert!(
            scale
                .dimensions()
                .iter()
                .any(|d| d.expression().is_some_and(|e| e.bindings[0].target == id))
        );
        for factor in [0., -1., f64::NAN, f64::INFINITY, 1e-200] {
            let scale = ScaleParameters {
                factors: [ratio(factor), ratio(factor), ratio(factor)],
                ..Default::default()
            };
            assert!(scale.resolve(&parameters::evaluate(&d).unwrap()).is_err());
        }
        let mut invalid = ScaleParameters::default();
        invalid.axes[2] = [0., 0., -1.];
        assert!(invalid.validate().is_err());
    }
}
