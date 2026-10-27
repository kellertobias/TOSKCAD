//! Typed drawing input shared by native tools and external control. A draft
//! owns its proposed source; dropping it never changes canonical geometry.
use crate::{
    constraint_sketch::{ConstraintKind, Curve, CurveKind, source::Source},
    document::{DocumentError, EntityId},
};
use serde::{Deserialize, Serialize};

const MIN_LENGTH_MM: f64 = 1e-7;
const MAX_COORDINATE_MM: f64 = 1e9;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PointInput {
    Position([f64; 2]),
    Existing(EntityId),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DrawInput {
    Line {
        start: PointInput,
        end: PointInput,
    },
    Rectangle {
        a: PointInput,
        b: PointInput,
    },
    Circle {
        center: PointInput,
        radius_mm: f64,
    },
    Arc {
        center: PointInput,
        start: PointInput,
        end: PointInput,
        clockwise: bool,
    },
    /// Closed polylines reuse the first vertex for their closing edge. Do not
    /// repeat that vertex as the final input, which would add a zero-length edge.
    Polyline {
        vertices: Vec<PointInput>,
        closed: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawRequest {
    pub input: DrawInput,
    #[serde(default)]
    pub construction: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DrawDraft {
    pub source: Source,
    /// In authored drawing order, including every rectangle/polyline segment.
    pub created_curves: Vec<EntityId>,
}
fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn point(source: &mut Source, input: &PointInput) -> Result<(EntityId, [f64; 2]), DocumentError> {
    match *input {
        PointInput::Existing(id) => source
            .sketch
            .points
            .iter()
            .find(|p| p.id == id)
            .map(|p| (id, p.position_mm))
            .ok_or(DocumentError::Missing(id)),
        PointInput::Position(p) => {
            if p.iter()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE_MM)
            {
                return Err(invalid(
                    "Drawing coordinates must be finite millimeters within ±1e9",
                ));
            }
            Ok((source.add_point(p), p))
        }
    }
}
fn segment(a: [f64; 2], b: [f64; 2]) -> Result<(), DocumentError> {
    if distance(a, b) <= MIN_LENGTH_MM {
        Err(invalid("Drawing segment must be longer than 1e-7 mm"))
    } else {
        Ok(())
    }
}
fn bounds(point: [f64; 2]) -> Result<(), DocumentError> {
    if point.iter().any(|v| v.abs() > MAX_COORDINATE_MM) {
        Err(invalid(
            "Drawing curve extends beyond the ±1e9 mm modeling bounds",
        ))
    } else {
        Ok(())
    }
}
fn circular_bounds(
    center: [f64; 2],
    radius: f64,
    arc: Option<([f64; 2], [f64; 2], bool)>,
) -> Result<(), DocumentError> {
    let sweep = arc.map(|(a, b, clockwise)| {
        let start = (a[1] - center[1]).atan2(a[0] - center[0]);
        let end = (b[1] - center[1]).atan2(b[0] - center[0]);
        (
            start,
            if clockwise {
                -(start - end).rem_euclid(std::f64::consts::TAU)
            } else {
                (end - start).rem_euclid(std::f64::consts::TAU)
            },
        )
    });
    for i in 0..4 {
        let angle = i as f64 * std::f64::consts::FRAC_PI_2;
        if let Some((start, sweep)) = sweep {
            let travel = if sweep > 0. {
                (angle - start).rem_euclid(std::f64::consts::TAU)
            } else {
                (start - angle).rem_euclid(std::f64::consts::TAU)
            };
            if travel > sweep.abs() {
                continue;
            }
        }
        bounds([
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ])?;
    }
    Ok(())
}
fn append(source: &mut Source, kind: CurveKind, construction: bool, created: &mut Vec<EntityId>) {
    let id = EntityId::new();
    source.sketch.curves.push(Curve {
        id,
        kind,
        construction,
    });
    created.push(id);
}

/// Existing point input is an identity reference, not coordinate-based merging.
/// Rectangle corner reuse and polyline connectivity therefore become actual
/// shared points. No constraints, basis or existing identities are replaced.
pub fn draft(source: &Source, request: &DrawRequest) -> Result<DrawDraft, DocumentError> {
    source.validate()?;
    if matches!(&request.input,DrawInput::Polyline {vertices,..} if vertices.len()>512) {
        return Err(invalid("Polyline exceeds the 512-vertex drawing budget"));
    }
    let mut proposed = source.clone();
    let mut created = vec![];
    let construction = request.construction;
    match &request.input {
        DrawInput::Line { start, end } => {
            let (start, a) = point(&mut proposed, start)?;
            let (end, b) = point(&mut proposed, end)?;
            segment(a, b)?;
            append(
                &mut proposed,
                CurveKind::Line { start, end },
                construction,
                &mut created,
            );
        }
        DrawInput::Rectangle { a, b } => {
            let (a_id, a) = point(&mut proposed, a)?;
            let (b_id, b) = point(&mut proposed, b)?;
            if (a[0] - b[0]).abs() <= MIN_LENGTH_MM || (a[1] - b[1]).abs() <= MIN_LENGTH_MM {
                return Err(invalid("Rectangle width and height must exceed 1e-7 mm"));
            }
            let corners = [
                a_id,
                proposed.add_point([b[0], a[1]]),
                b_id,
                proposed.add_point([a[0], b[1]]),
            ];
            for i in 0..4 {
                append(
                    &mut proposed,
                    CurveKind::Line {
                        start: corners[i],
                        end: corners[(i + 1) % 4],
                    },
                    construction,
                    &mut created,
                );
                proposed.add_constraint(if i % 2 == 0 {
                    ConstraintKind::Horizontal { line: created[i] }
                } else {
                    ConstraintKind::Vertical { line: created[i] }
                });
            }
        }
        DrawInput::Circle { center, radius_mm } => {
            if !radius_mm.is_finite()
                || *radius_mm <= MIN_LENGTH_MM
                || *radius_mm > MAX_COORDINATE_MM
            {
                return Err(invalid(
                    "Circle radius must be finite and within (1e-7, 1e9] mm",
                ));
            }
            let (center, c) = point(&mut proposed, center)?;
            circular_bounds(c, *radius_mm, None)?;
            append(
                &mut proposed,
                CurveKind::Circle {
                    center,
                    radius_mm: *radius_mm,
                },
                construction,
                &mut created,
            );
        }
        DrawInput::Arc {
            center,
            start,
            end,
            clockwise,
        } => {
            let (center, c) = point(&mut proposed, center)?;
            let (start, a) = point(&mut proposed, start)?;
            let (end, b) = point(&mut proposed, end)?;
            segment(c, a)?;
            segment(a, b)?;
            let radius = distance(c, a);
            if radius > MAX_COORDINATE_MM || (radius - distance(c, b)).abs() > MIN_LENGTH_MM {
                return Err(invalid(
                    "Arc endpoints must lie on the chosen radius within 1e-7 mm; use Circle for a full circle",
                ));
            }
            circular_bounds(c, radius, Some((a, b, *clockwise)))?;
            append(
                &mut proposed,
                CurveKind::Arc {
                    center,
                    start,
                    end,
                    clockwise: *clockwise,
                },
                construction,
                &mut created,
            );
        }
        DrawInput::Polyline { vertices, closed } => {
            if vertices.len() < if *closed { 3 } else { 2 } {
                return Err(invalid(
                    "Polyline needs two vertices, or at least three when closed",
                ));
            }
            let vertices: Vec<_> = vertices
                .iter()
                .map(|p| point(&mut proposed, p))
                .collect::<Result<_, _>>()?;
            let count = if *closed {
                vertices.len()
            } else {
                vertices.len() - 1
            };
            for i in 0..count {
                let (start, a) = vertices[i];
                let (end, b) = vertices[(i + 1) % vertices.len()];
                segment(a, b)?;
                append(
                    &mut proposed,
                    CurveKind::Line { start, end },
                    construction,
                    &mut created,
                );
            }
        }
    }
    proposed.validate()?;
    Ok(DrawDraft {
        source: proposed,
        created_curves: created,
    })
}
