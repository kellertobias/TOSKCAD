//! Solver-independent sketch source. Distances are explicitly unit-bearing;
//! coordinates and radius guesses use millimeters in the sketch plane.
use crate::document::EntityId;
use serde::{Deserialize, Serialize};

pub mod source;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sketch {
    pub points: Vec<Point>,
    pub curves: Vec<Curve>,
    pub constraints: Vec<Constraint>,
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
#[serde(deny_unknown_fields)]
pub struct Dimension {
    pub value: f64,
    pub unit: String,
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
