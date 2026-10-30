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
    BSpline {
        poles: Vec<PointInput>,
        basis: crate::constraint_sketch::SplineBasis,
        #[serde(default)]
        closed: bool,
    },
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
    /// Axis-aligned rectangle centred on `center` with one chosen corner.
    /// The center is an input location, not a retained sketch point.
    CenterRectangle {
        center: [f64; 2],
        corner: PointInput,
    },
    /// Rectangle centred on `center`: `corner` is one corner and `adjacent`
    /// gives the direction of the neighbouring corner, which is placed at the
    /// same distance from the center. The result may be rotated.
    CenterPointsRectangle {
        center: [f64; 2],
        corner: PointInput,
        adjacent: [f64; 2],
    },
    /// Rectangle whose first edge runs from `a` to `b`; the perpendicular
    /// offset of `height` from that edge sets the other side. May be rotated.
    ThreePointRectangle {
        a: PointInput,
        b: PointInput,
        height: [f64; 2],
    },
    /// Circle of `radius_mm` through `a` and `b`. `left` chooses the center on
    /// the left of the direction from `a` to `b`. The defining points are input
    /// locations, not retained sketch points.
    TwoPointCircle {
        a: [f64; 2],
        b: [f64; 2],
        radius_mm: f64,
        left: bool,
    },
    /// Circle through three non-collinear locations (not retained as points).
    ThreePointCircle {
        a: [f64; 2],
        b: [f64; 2],
        c: [f64; 2],
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

fn finite(point: [f64; 2]) -> Result<[f64; 2], DocumentError> {
    if point
        .iter()
        .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE_MM)
    {
        Err(invalid(
            "Drawing coordinates must be finite millimeters within ±1e9",
        ))
    } else {
        Ok(point)
    }
}
fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn scale(a: [f64; 2], factor: f64) -> [f64; 2] {
    [a[0] * factor, a[1] * factor]
}
/// Center of the circle through `a` and `b` with `radius` on the chosen side.
pub fn two_point_circle_center(
    a: [f64; 2],
    b: [f64; 2],
    radius: f64,
    left: bool,
) -> Result<[f64; 2], DocumentError> {
    let chord = distance(a, b);
    if chord <= MIN_LENGTH_MM {
        return Err(invalid("Circle points must be more than 1e-7 mm apart"));
    }
    if !radius.is_finite() || radius > MAX_COORDINATE_MM || radius < chord / 2. - MIN_LENGTH_MM {
        return Err(invalid(
            "Circle radius must be at least half the distance between its two points",
        ));
    }
    let offset = (radius * radius - chord * chord / 4.).max(0.).sqrt();
    let direction = scale(sub(b, a), 1. / chord);
    let normal = [-direction[1], direction[0]];
    let mid = scale(add(a, b), 0.5);
    Ok(add(mid, scale(normal, if left { offset } else { -offset })))
}
/// Circumcenter and radius of three non-collinear locations.
pub fn three_point_circle(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
) -> Result<([f64; 2], f64), DocumentError> {
    let (ab, ac) = (sub(b, a), sub(c, a));
    let cross = ab[0] * ac[1] - ab[1] * ac[0];
    let span = distance(a, b).max(distance(a, c)).max(distance(b, c));
    if span <= MIN_LENGTH_MM || cross.abs() <= 1e-9 * span * span {
        return Err(invalid(
            "Three-point circle needs three distinct, non-collinear points",
        ));
    }
    let (ab2, ac2) = (ab[0] * ab[0] + ab[1] * ab[1], ac[0] * ac[0] + ac[1] * ac[1]);
    let center = add(
        a,
        [
            (ac[1] * ab2 - ab[1] * ac2) / (2. * cross),
            (ab[0] * ac2 - ac[0] * ab2) / (2. * cross),
        ],
    );
    Ok((center, distance(center, a)))
}
/// Corners of a rectangle given by one edge `a`→`b` and a point setting the
/// perpendicular height.
pub fn three_point_rectangle_corners(
    a: [f64; 2],
    b: [f64; 2],
    height: [f64; 2],
) -> Result<[[f64; 2]; 4], DocumentError> {
    let length = distance(a, b);
    if length <= MIN_LENGTH_MM {
        return Err(invalid("Rectangle edge must be longer than 1e-7 mm"));
    }
    let direction = scale(sub(b, a), 1. / length);
    let normal = [-direction[1], direction[0]];
    let offset = sub(height, b);
    let h = offset[0] * normal[0] + offset[1] * normal[1];
    if h.abs() <= MIN_LENGTH_MM {
        return Err(invalid("Rectangle width and height must exceed 1e-7 mm"));
    }
    let shift = scale(normal, h);
    Ok([a, b, add(b, shift), add(a, shift)])
}
/// Corners of a centred rectangle from one corner and the direction of its
/// neighbouring corner (placed at the same distance from the center).
pub fn center_points_rectangle_corners(
    center: [f64; 2],
    corner: [f64; 2],
    adjacent: [f64; 2],
) -> Result<[[f64; 2]; 4], DocumentError> {
    let radius = distance(center, corner);
    let toward = sub(adjacent, center);
    let length = toward[0].hypot(toward[1]);
    if radius <= MIN_LENGTH_MM || length <= MIN_LENGTH_MM {
        return Err(invalid(
            "Rectangle corners must be away from the rectangle center",
        ));
    }
    let second = add(center, scale(toward, radius / length));
    let opposite = sub(scale(center, 2.), corner);
    if distance(corner, second) <= MIN_LENGTH_MM || distance(second, opposite) <= MIN_LENGTH_MM {
        return Err(invalid("Rectangle width and height must exceed 1e-7 mm"));
    }
    Ok([corner, second, opposite, sub(scale(center, 2.), second)])
}
/// Four closed edges through the corner points. Axis-aligned results keep the
/// horizontal/vertical constraints of the classic rectangle; rotated results
/// use parallel and perpendicular constraints so the solver keeps them
/// rectangular.
fn rectangle_edges(
    source: &mut Source,
    corners: [(EntityId, [f64; 2]); 4],
    construction: bool,
    created: &mut Vec<EntityId>,
) -> Result<(), DocumentError> {
    for i in 0..4 {
        segment(corners[i].1, corners[(i + 1) % 4].1)?;
        bounds(corners[i].1)?;
    }
    let first = created.len();
    for i in 0..4 {
        append(
            source,
            CurveKind::Line {
                start: corners[i].0,
                end: corners[(i + 1) % 4].0,
            },
            construction,
            created,
        );
    }
    let lines: Vec<EntityId> = created[first..].to_vec();
    let edge = sub(corners[1].1, corners[0].1);
    let tolerance = MIN_LENGTH_MM;
    if edge[1].abs() <= tolerance || edge[0].abs() <= tolerance {
        let first_horizontal = edge[1].abs() <= tolerance;
        for (i, line) in lines.iter().enumerate() {
            let line = *line;
            source.add_constraint(if (i % 2 == 0) == first_horizontal {
                ConstraintKind::Horizontal { line }
            } else {
                ConstraintKind::Vertical { line }
            });
        }
    } else {
        source.add_constraint(ConstraintKind::Parallel {
            a: lines[0],
            b: lines[2],
        });
        source.add_constraint(ConstraintKind::Parallel {
            a: lines[1],
            b: lines[3],
        });
        source.add_constraint(ConstraintKind::Perpendicular {
            a: lines[0],
            b: lines[1],
        });
    }
    Ok(())
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
        DrawInput::BSpline {
            poles,
            basis,
            closed,
        } => {
            if poles.is_empty() {
                return Err(invalid("Spline needs control poles"));
            }
            basis.validate(poles.len() + usize::from(*closed))?;
            let mut positions: Vec<_> = poles
                .iter()
                .map(|p| point(&mut proposed, p))
                .collect::<Result<_, _>>()?;
            let first = positions[0].1;
            if *closed && positions.first().map(|p| p.0) == positions.last().map(|p| p.0) {
                return Err(invalid(
                    "Closed spline input must omit the repeated final first pole",
                ));
            }
            if positions
                .iter()
                .all(|(_, p)| distance(first, *p) < MIN_LENGTH_MM)
            {
                return Err(invalid("Spline control polygon is degenerate"));
            }
            if *closed {
                positions.push(positions[0]);
            }
            append(
                &mut proposed,
                CurveKind::BSpline {
                    poles: positions.iter().map(|(id, _)| *id).collect(),
                    basis: basis.clone(),
                },
                construction,
                &mut created,
            );
        }
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
        DrawInput::CenterRectangle { center, corner } => {
            let center = finite(*center)?;
            let (corner_id, corner) = point(&mut proposed, corner)?;
            let opposite = sub(scale(center, 2.), corner);
            if (corner[0] - opposite[0]).abs() <= MIN_LENGTH_MM
                || (corner[1] - opposite[1]).abs() <= MIN_LENGTH_MM
            {
                return Err(invalid("Rectangle width and height must exceed 1e-7 mm"));
            }
            bounds(opposite)?;
            let corners = [
                (corner_id, corner),
                (
                    proposed.add_point([opposite[0], corner[1]]),
                    [opposite[0], corner[1]],
                ),
                (proposed.add_point(opposite), opposite),
                (
                    proposed.add_point([corner[0], opposite[1]]),
                    [corner[0], opposite[1]],
                ),
            ];
            rectangle_edges(&mut proposed, corners, construction, &mut created)?;
        }
        DrawInput::CenterPointsRectangle {
            center,
            corner,
            adjacent,
        } => {
            let center = finite(*center)?;
            let adjacent = finite(*adjacent)?;
            let (corner_id, corner) = point(&mut proposed, corner)?;
            let positions = center_points_rectangle_corners(center, corner, adjacent)?;
            let corners = [
                (corner_id, corner),
                (proposed.add_point(positions[1]), positions[1]),
                (proposed.add_point(positions[2]), positions[2]),
                (proposed.add_point(positions[3]), positions[3]),
            ];
            rectangle_edges(&mut proposed, corners, construction, &mut created)?;
        }
        DrawInput::ThreePointRectangle { a, b, height } => {
            let height = finite(*height)?;
            let (a_id, a) = point(&mut proposed, a)?;
            let (b_id, b) = point(&mut proposed, b)?;
            if a_id == b_id {
                return Err(invalid("Rectangle edge needs two different points"));
            }
            let positions = three_point_rectangle_corners(a, b, height)?;
            let corners = [
                (a_id, a),
                (b_id, b),
                (proposed.add_point(positions[2]), positions[2]),
                (proposed.add_point(positions[3]), positions[3]),
            ];
            rectangle_edges(&mut proposed, corners, construction, &mut created)?;
        }
        DrawInput::TwoPointCircle {
            a,
            b,
            radius_mm,
            left,
        } => {
            let (a, b) = (finite(*a)?, finite(*b)?);
            let center = two_point_circle_center(a, b, *radius_mm, *left)?;
            circular_bounds(center, *radius_mm, None)?;
            let center = proposed.add_point(center);
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
        DrawInput::ThreePointCircle { a, b, c } => {
            let (center, radius) = three_point_circle(finite(*a)?, finite(*b)?, finite(*c)?)?;
            if radius > MAX_COORDINATE_MM {
                return Err(invalid("Circle radius exceeds the ±1e9 mm modeling bounds"));
            }
            circular_bounds(center, radius, None)?;
            let center = proposed.add_point(center);
            append(
                &mut proposed,
                CurveKind::Circle {
                    center,
                    radius_mm: radius,
                },
                construction,
                &mut created,
            );
        }
    }
    proposed.validate()?;
    Ok(DrawDraft {
        source: proposed,
        created_curves: created,
    })
}
