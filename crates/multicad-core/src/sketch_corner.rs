//! Sketch corner fillet and chamfer. A treatment replaces the sharp corner
//! shared by exactly two lines with a tangent arc or a chamfer line. The
//! original corner point stays as a construction "virtual sharp" (two
//! construction lines kept collinear with the trimmed edges), so existing
//! references to it remain meaningful and the solver keeps the treatment's
//! shape. The draft owns its proposed source; dropping it changes nothing.
use crate::{
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::{DocumentError, EntityId},
};
use serde::{Deserialize, Serialize};

const MIN_LENGTH_MM: f64 = 1e-7;
const MAX_SIZE_MM: f64 = 1e9;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CornerTreatment {
    /// Tangent arc of this radius.
    Fillet { radius_mm: f64 },
    /// Straight chamfer set back this distance along both edges.
    Chamfer { distance_mm: f64 },
}
#[derive(Clone, Debug, PartialEq)]
pub struct CornerDraft {
    pub source: Source,
    /// The new arc or chamfer line, followed by the two construction lines.
    pub created_curves: Vec<EntityId>,
}

fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
fn literal(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}

/// Corner points at which a treatment can apply: points shared by exactly two
/// lines and no other curve.
pub fn treatable_corners(source: &Source) -> Vec<EntityId> {
    source
        .sketch
        .points
        .iter()
        .filter(|point| corner_lines(source, point.id).is_ok())
        .map(|point| point.id)
        .collect()
}

fn corner_lines(source: &Source, corner: EntityId) -> Result<[(usize, bool); 2], DocumentError> {
    let mut lines = vec![];
    for (index, curve) in source.sketch.curves.iter().enumerate() {
        match curve.kind {
            CurveKind::Line { start, end } if start == corner || end == corner => {
                if start == end {
                    return Err(invalid("Corner line is degenerate"));
                }
                lines.push((index, start == corner));
            }
            CurveKind::Line { .. } => {}
            CurveKind::Arc {
                center, start, end, ..
            } if [center, start, end].contains(&corner) => {
                return Err(invalid(
                    "Fillet and chamfer need a corner shared by exactly two lines",
                ));
            }
            CurveKind::Circle { center, .. } if center == corner => {
                return Err(invalid(
                    "Fillet and chamfer need a corner shared by exactly two lines",
                ));
            }
            CurveKind::BSpline { ref poles, .. } if poles.contains(&corner) => {
                return Err(invalid(
                    "Fillet and chamfer need a corner shared by exactly two lines",
                ));
            }
            _ => {}
        }
    }
    match lines.as_slice() {
        [a, b] => Ok([*a, *b]),
        _ => Err(invalid(
            "Fillet and chamfer need a corner shared by exactly two lines",
        )),
    }
}

pub fn corner_draft(
    source: &Source,
    corner: EntityId,
    treatment: CornerTreatment,
) -> Result<CornerDraft, DocumentError> {
    source.validate()?;
    let size = match treatment {
        CornerTreatment::Fillet { radius_mm } => radius_mm,
        CornerTreatment::Chamfer { distance_mm } => distance_mm,
    };
    if !size.is_finite() || size <= MIN_LENGTH_MM || size > MAX_SIZE_MM {
        return Err(invalid(
            "Fillet radius or chamfer distance must be finite and within (1e-7, 1e9] mm",
        ));
    }
    let position = |id: EntityId| {
        source
            .sketch
            .points
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.position_mm)
            .ok_or(DocumentError::Missing(id))
    };
    let at = position(corner)?;
    let lines = corner_lines(source, corner)?;
    let mut lengths = [0.; 2];
    let mut directions = [[0.; 2]; 2];
    for (i, (index, corner_is_start)) in lines.iter().enumerate() {
        let CurveKind::Line { start, end } = source.sketch.curves[*index].kind else {
            unreachable!("corner_lines returns lines");
        };
        let other = position(if *corner_is_start { end } else { start })?;
        let delta = [other[0] - at[0], other[1] - at[1]];
        let length = delta[0].hypot(delta[1]);
        if length <= MIN_LENGTH_MM {
            return Err(invalid("Corner line is degenerate"));
        }
        lengths[i] = length;
        directions[i] = [delta[0] / length, delta[1] / length];
    }
    let cosine =
        (directions[0][0] * directions[1][0] + directions[0][1] * directions[1][1]).clamp(-1., 1.);
    let angle = cosine.acos();
    if !(1e-6..=std::f64::consts::PI - 1e-6).contains(&angle) {
        return Err(invalid(
            "Fillet and chamfer need two lines meeting at an angle, not collinear lines",
        ));
    }
    let setback = match treatment {
        CornerTreatment::Fillet { radius_mm } => radius_mm / (angle / 2.).tan(),
        CornerTreatment::Chamfer { distance_mm } => distance_mm,
    };
    if lengths
        .iter()
        .any(|length| setback >= length - MIN_LENGTH_MM)
    {
        return Err(invalid(format!(
            "The treatment needs {setback:.3} mm along each line; a corner line is too short"
        )));
    }
    let mut proposed = source.clone();
    let tangent = |i: usize| {
        [
            at[0] + directions[i][0] * setback,
            at[1] + directions[i][1] * setback,
        ]
    };
    let points = [tangent(0), tangent(1)];
    let ids = [proposed.add_point(points[0]), proposed.add_point(points[1])];
    let mut edges = [EntityId::new(); 2];
    for (i, (index, corner_is_start)) in lines.iter().enumerate() {
        let curve = &mut proposed.sketch.curves[*index];
        if let CurveKind::Line { start, end } = &mut curve.kind {
            if *corner_is_start {
                *start = ids[i];
            } else {
                *end = ids[i];
            }
        }
        edges[i] = curve.id;
    }
    let construction = lines.map(|(index, _)| source.sketch.curves[index].construction);
    let mut created = vec![];
    let mut push = |source: &mut Source, kind, helper: bool| {
        let id = EntityId::new();
        source.sketch.curves.push(Curve {
            id,
            kind,
            construction: helper,
        });
        created.push(id);
        id
    };
    let both_construction = construction.iter().all(|c| *c);
    match treatment {
        CornerTreatment::Fillet { radius_mm } => {
            let bisector = [
                directions[0][0] + directions[1][0],
                directions[0][1] + directions[1][1],
            ];
            let norm = bisector[0].hypot(bisector[1]);
            let distance = radius_mm / (angle / 2.).sin();
            let center = [
                at[0] + bisector[0] / norm * distance,
                at[1] + bisector[1] / norm * distance,
            ];
            let cross = (points[0][0] - center[0]) * (points[1][1] - center[1])
                - (points[0][1] - center[1]) * (points[1][0] - center[0]);
            let center = proposed.add_point(center);
            let arc = push(
                &mut proposed,
                CurveKind::Arc {
                    center,
                    start: ids[0],
                    end: ids[1],
                    clockwise: cross < 0.,
                },
                both_construction,
            );
            for edge in edges {
                proposed.add_constraint(ConstraintKind::Tangent { a: edge, b: arc });
            }
            proposed.add_constraint(ConstraintKind::Radius {
                curve: arc,
                dimension: literal(radius_mm),
            });
        }
        CornerTreatment::Chamfer { .. } => {
            push(
                &mut proposed,
                CurveKind::Line {
                    start: ids[0],
                    end: ids[1],
                },
                both_construction,
            );
        }
    }
    for (i, edge) in edges.iter().enumerate() {
        let helper = push(
            &mut proposed,
            CurveKind::Line {
                start: ids[i],
                end: corner,
            },
            true,
        );
        proposed.add_constraint(ConstraintKind::Parallel {
            a: helper,
            b: *edge,
        });
        if let CornerTreatment::Chamfer { distance_mm } = treatment {
            proposed.add_constraint(ConstraintKind::Length {
                line: helper,
                dimension: literal(distance_mm),
            });
        }
    }
    proposed.validate()?;
    Ok(CornerDraft {
        source: proposed,
        created_curves: created,
    })
}
