//! Solver-independent sketch source. Distances are explicitly unit-bearing;
//! coordinates and radius guesses use millimeters in the sketch plane.
use crate::document::EntityId;
use serde::{Deserialize, Serialize};

pub mod binding;
pub mod source;
mod spline;
pub mod text;
pub use spline::{MAX_SPLINE_POLES, SplineBasis};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sketch {
    pub points: Vec<Point>,
    pub curves: Vec<Curve>,
    pub constraints: Vec<Constraint>,
    /// Parametric text elements (CAD-142). Their exact glyph contours are
    /// derived during evaluation and are not solver points or curves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<text::SketchText>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub id: EntityId,
    pub position_mm: [f64; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    pub id: EntityId,
    pub construction: bool,
    pub kind: CurveKind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurveKind {
    BSpline {
        poles: Vec<EntityId>,
        basis: SplineBasis,
    },
    Line {
        start: EntityId,
        end: EntityId,
    },
    Arc {
        center: EntityId,
        start: EntityId,
        end: EntityId,
        clockwise: bool,
    },
    Circle {
        center: EntityId,
        radius_mm: f64,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Dimension {
    Literal {
        value: f64,
        unit: String,
    },
    Expression {
        expression: crate::parameters::BoundExpression,
    },
}
impl Dimension {
    pub fn unit(&self) -> &str {
        match self {
            Self::Literal { unit, .. } => unit,
            Self::Expression { expression } => &expression.unit,
        }
    }
    pub fn expression(&self) -> Option<&crate::parameters::BoundExpression> {
        match self {
            Self::Expression { expression } => Some(expression),
            _ => None,
        }
    }
    pub fn value_in(
        &self,
        target: crate::parameters::Unit,
        parameters: Option<&crate::parameters::EvaluatedParameters>,
    ) -> Result<f64, String> {
        let unit =
            crate::parameters::Unit::parse(self.unit()).ok_or("Unknown sketch dimension unit")?;
        if unit.dimension() != target.dimension() {
            return Err("Incompatible sketch dimension units".into());
        }
        let value = match self {
            Self::Literal { value, .. } => *value * unit.scale() / target.scale(),
            Self::Expression { expression } => expression
                .evaluate(
                    parameters
                        .ok_or("Parameter-driven sketch requires an evaluated parameter context")?,
                )
                .map_err(|e| e.to_string())?
                .in_unit(target)
                .ok_or("Incompatible expression dimension")?,
        };
        if !value.is_finite() || value.abs() > 1e9 {
            return Err("Sketch dimension result is nonfinite or exceeds limits".into());
        }
        Ok(value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Direct,
    X,
    Y,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub id: EntityId,
    /// Reference dimensions measure the solved geometry and add no equations.
    pub reference: bool,
    pub kind: ConstraintKind,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConstraintKind {
    Fix {
        point: EntityId,
    },
    Coincident {
        a: EntityId,
        b: EntityId,
    },
    Horizontal {
        line: EntityId,
    },
    Vertical {
        line: EntityId,
    },
    Parallel {
        a: EntityId,
        b: EntityId,
    },
    Perpendicular {
        a: EntityId,
        b: EntityId,
    },
    Tangent {
        a: EntityId,
        b: EntityId,
    },
    Equal {
        a: EntityId,
        b: EntityId,
    },
    Symmetric {
        a: EntityId,
        b: EntityId,
        about: EntityId,
    },
    Distance {
        a: EntityId,
        b: EntityId,
        axis: Axis,
        dimension: Dimension,
    },
    Length {
        line: EntityId,
        dimension: Dimension,
    },
    Radius {
        curve: EntityId,
        dimension: Dimension,
    },
    Diameter {
        curve: EntityId,
        dimension: Dimension,
    },
    Angle {
        from: EntityId,
        to: EntityId,
        dimension: Dimension,
    },
}
impl ConstraintKind {
    pub fn dimension(&self) -> Option<&Dimension> {
        match self {
            Self::Distance { dimension, .. }
            | Self::Length { dimension, .. }
            | Self::Radius { dimension, .. }
            | Self::Diameter { dimension, .. }
            | Self::Angle { dimension, .. } => Some(dimension),
            _ => None,
        }
    }
    pub fn references(&self) -> Vec<EntityId> {
        match self {
            Self::Fix { point } => vec![*point],
            Self::Horizontal { line } | Self::Vertical { line } | Self::Length { line, .. } => {
                vec![*line]
            }
            Self::Radius { curve, .. } | Self::Diameter { curve, .. } => vec![*curve],
            Self::Coincident { a, b }
            | Self::Parallel { a, b }
            | Self::Perpendicular { a, b }
            | Self::Tangent { a, b }
            | Self::Equal { a, b }
            | Self::Distance { a, b, .. } => vec![*a, *b],
            Self::Symmetric { a, b, about } => vec![*a, *b, *about],
            Self::Angle { from, to, .. } => vec![*from, *to],
        }
    }
    pub fn dimension_mut(&mut self) -> Option<&mut Dimension> {
        match self {
            Self::Distance { dimension, .. }
            | Self::Length { dimension, .. }
            | Self::Radius { dimension, .. }
            | Self::Diameter { dimension, .. }
            | Self::Angle { dimension, .. } => Some(dimension),
            _ => None,
        }
    }
}
