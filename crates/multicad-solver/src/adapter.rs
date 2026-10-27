//! The only place production source identities become solver-local handles.
use multicad_core::{
    constraint_sketch::{Axis, ConstraintKind as Kind, CurveKind, Dimension, Sketch},
    document::EntityId,
    parameters::{EvaluatedParameters, Unit},
};
use sketch_solver::{Along, Constraint, Pair, Source, Status};
use std::collections::{BTreeMap, BTreeSet};

const COORDINATE_LIMIT_MM: f64 = 1e9;
const MAX_UNKNOWNS: usize = 512;
const MAX_CONSTRAINTS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputError {
    pub source: Option<EntityId>,
    pub message: String,
}
impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for InputError {}
fn error(source: Option<EntityId>, message: impl Into<String>) -> InputError {
    InputError {
        source,
        message: message.into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolveState {
    Underconstrained,
    FullyConstrained,
    Conflict,
    NotConverged,
    Flipped,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostics {
    pub state: SolveState,
    pub degrees_of_freedom: usize,
    pub free_entities: BTreeSet<EntityId>,
    pub conflicts: BTreeSet<EntityId>,
    pub redundant: BTreeSet<EntityId>,
    pub unmet: BTreeSet<EntityId>,
    pub residual_mm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    pub value: f64,
    pub unit: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    /// Only a valid, branch-preserving solve produces a candidate. The caller
    /// publishes it through the document transaction API; failures change nothing.
    pub candidate: Option<Sketch>,
    pub diagnostics: Diagnostics,
    pub measurements: BTreeMap<EntityId, Measurement>,
}

struct Adapter<'a> {
    solver: sketch_solver::Sketch,
    entities: BTreeMap<EntityId, sketch_solver::EntityId>,
    source_entities: Vec<EntityId>,
    source_constraints: Vec<EntityId>,
    parameters: Option<&'a EvaluatedParameters>,
}
impl Adapter<'_> {
    fn entity(&self, id: EntityId) -> Result<sketch_solver::EntityId, InputError> {
        self.entities
            .get(&id)
            .copied()
            .ok_or_else(|| error(Some(id), "Missing sketch entity"))
    }
    fn source(&self, source: Source) -> EntityId {
        match source {
            Source::Constraint(id) => self.source_constraints[id.index()],
            Source::Arc(id) => self.source_entities[id.index()],
        }
    }
    fn constraint(&self, kind: &Kind) -> Result<Constraint, InputError> {
        let e = |id| self.entity(id);
        Ok(match kind {
            Kind::Fix { point } => Constraint::Fix {
                entity: e(*point)?,
                at: None,
            },
            Kind::Coincident { a, b } => Constraint::Coincident(e(*a)?, e(*b)?),
            Kind::Horizontal { line } => Constraint::Horizontal(Pair::Line(e(*line)?)),
            Kind::Vertical { line } => Constraint::Vertical(Pair::Line(e(*line)?)),
            Kind::Parallel { a, b } => Constraint::Parallel(e(*a)?, e(*b)?),
            Kind::Perpendicular { a, b } => Constraint::Perpendicular(e(*a)?, e(*b)?),
            Kind::Tangent { a, b } => Constraint::Tangent(e(*a)?, e(*b)?),
            Kind::Equal { a, b } => Constraint::Equal(e(*a)?, e(*b)?),
            Kind::Symmetric { a, b, about } => Constraint::Symmetric {
                a: e(*a)?,
                b: e(*b)?,
                about: e(*about)?,
            },
            Kind::Distance {
                a,
                b,
                axis,
                dimension,
            } => Constraint::Distance {
                a: e(*a)?,
                b: e(*b)?,
                value: dimension_value(dimension, Unit::Millimeter, self.parameters)?,
                along: match axis {
                    Axis::Direct => Along::Direct,
                    Axis::X => Along::X,
                    Axis::Y => Along::Y,
                },
            },
            Kind::Length { line, dimension } => Constraint::Length {
                line: e(*line)?,
                value: dimension_value(dimension, Unit::Millimeter, self.parameters)?,
            },
            Kind::Radius { curve, dimension } => Constraint::Radius {
                curve: e(*curve)?,
                value: dimension_value(dimension, Unit::Millimeter, self.parameters)?,
            },
            Kind::Diameter { curve, dimension } => Constraint::Diameter {
                curve: e(*curve)?,
                value: dimension_value(dimension, Unit::Millimeter, self.parameters)?,
            },
            Kind::Angle {
                from,
                to,
                dimension,
            } => Constraint::Angle {
                from: e(*from)?,
                to: e(*to)?,
                degrees: dimension_value(dimension, Unit::Degree, self.parameters)?,
            },
        })
    }
}
fn dimension_value(
    d: &Dimension,
    target: Unit,
    parameters: Option<&EvaluatedParameters>,
) -> Result<f64, InputError> {
    d.value_in(target, parameters).map_err(|e| error(None, e))
}
fn dimension(kind: &Kind) -> Option<&Dimension> {
    match kind {
        Kind::Distance { dimension, .. }
        | Kind::Length { dimension, .. }
        | Kind::Radius { dimension, .. }
        | Kind::Diameter { dimension, .. }
        | Kind::Angle { dimension, .. } => Some(dimension),
        _ => None,
    }
}

/// Evaluate immutable source, independently of a UI, Python or a prior solve.
/// Limits bound the dense numerical work. UI cancellation can use `evaluate_with`.
pub fn evaluate(source: &Sketch) -> Result<Evaluation, InputError> {
    evaluate_with(source, None)
}
pub fn evaluate_with(
    source: &Sketch,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Evaluation, InputError> {
    evaluate_context(source, None, interrupt)
}
pub fn evaluate_with_parameters(
    source: &Sketch,
    parameters: &EvaluatedParameters,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Evaluation, InputError> {
    evaluate_context(source, Some(parameters), interrupt)
}
fn evaluate_context(
    source: &Sketch,
    parameters: Option<&EvaluatedParameters>,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Evaluation, InputError> {
    if source.points.len() > MAX_UNKNOWNS / 2
        || source.curves.len() > MAX_UNKNOWNS
        || source.constraints.len() > MAX_CONSTRAINTS
    {
        return Err(error(None, "Sketch exceeds interactive solver limits"));
    }
    let mut ids = BTreeSet::new();
    for id in source
        .points
        .iter()
        .map(|p| p.id)
        .chain(source.curves.iter().map(|c| c.id))
        .chain(source.constraints.iter().map(|c| c.id))
    {
        if id.is_nil() || !ids.insert(id) {
            return Err(error(Some(id), "Duplicate or nil sketch identity"));
        }
    }
    let mut a = Adapter {
        solver: sketch_solver::Sketch::new(),
        entities: BTreeMap::new(),
        source_entities: vec![],
        source_constraints: vec![],
        parameters,
    };
    for p in &source.points {
        if p.position_mm
            .iter()
            .any(|v| !v.is_finite() || v.abs() > COORDINATE_LIMIT_MM)
        {
            return Err(error(
                Some(p.id),
                "Point must have finite coordinates within ±1,000 km",
            ));
        }
        let handle = a
            .solver
            .point(Some(p.position_mm))
            .map_err(|e| error(Some(p.id), e.to_string()))?;
        a.entities.insert(p.id, handle);
        a.source_entities.push(p.id);
    }
    for c in &source.curves {
        if let CurveKind::BSpline { poles, basis } = &c.kind {
            basis
                .validate(poles.len())
                .map_err(|e| error(Some(c.id), e.to_string()))?;
            for pole in poles {
                a.entity(*pole)?;
            }
            continue;
        }
        let result = match c.kind {
            CurveKind::BSpline { .. } => unreachable!("spline poles handled separately"),
            CurveKind::Line { start, end } => a.solver.line(a.entity(start)?, a.entity(end)?),
            CurveKind::Arc {
                center,
                start,
                end,
                clockwise,
            } => a.solver.arc(
                a.entity(center)?,
                a.entity(start)?,
                a.entity(end)?,
                clockwise,
            ),
            CurveKind::Circle { center, radius_mm } => {
                if !radius_mm.is_finite() || radius_mm <= 0. || radius_mm > COORDINATE_LIMIT_MM {
                    return Err(error(
                        Some(c.id),
                        "Circle radius must be positive and finite",
                    ));
                }
                a.solver.circle(a.entity(center)?, Some(radius_mm))
            }
        };
        let handle = result.map_err(|e| error(Some(c.id), e.to_string()))?;
        a.solver
            .set_construction(handle, c.construction)
            .map_err(|e| error(Some(c.id), e.to_string()))?;
        a.entities.insert(c.id, handle);
        a.source_entities.push(c.id);
    }
    for c in &source.constraints {
        if let Some(spline) = c.kind.references().iter().find(|id| {
            source
                .curves
                .iter()
                .any(|curve| curve.id == **id && matches!(curve.kind, CurveKind::BSpline { .. }))
        }) {
            return Err(error(
                Some(c.id),
                format!(
                    "Constraint on spline curve {spline} is unsupported; constrain its control poles instead"
                ),
            ));
        }
        let constraint = a
            .constraint(&c.kind)
            .map_err(|e| error(Some(c.id), e.message))?;
        if c.reference {
            if dimension(&c.kind).is_none() {
                return Err(error(Some(c.id), "Only dimensions can be reference-only"));
            }
            // Validate types/values even for dimensions which add no equations.
            a.solver
                .clone()
                .add(constraint)
                .map_err(|e| error(Some(c.id), e.to_string()))?;
        } else {
            a.solver
                .add(constraint)
                .map_err(|e| error(Some(c.id), e.to_string()))?;
            a.source_constraints.push(c.id);
        }
    }
    let sol = a
        .solver
        .solve_with(&sketch_solver::SolveOptions {
            max_unknowns: MAX_UNKNOWNS,
            interrupt,
            ..Default::default()
        })
        .map_err(|e| error(None, e.to_string()))?;
    let dependencies = |d: &[sketch_solver::Dependency]| -> BTreeSet<EntityId> {
        d.iter()
            .flat_map(|d| std::iter::once(d.source).chain(d.with.iter().copied()))
            .map(|s| a.source(s))
            .collect()
    };
    let state = if !sol.conflicts.is_empty() {
        SolveState::Conflict
    } else if sol.status != Status::Solved {
        SolveState::NotConverged
    } else if !sol.flipped.is_empty() {
        SolveState::Flipped
    } else if sol.dof == 0 {
        SolveState::FullyConstrained
    } else {
        SolveState::Underconstrained
    };
    let diagnostics = Diagnostics {
        state,
        degrees_of_freedom: sol.dof,
        free_entities: sol
            .free
            .iter()
            .map(|f| a.source_entities[f.entity.index()])
            .collect(),
        conflicts: dependencies(&sol.conflicts),
        redundant: dependencies(&sol.redundant),
        unmet: sol.unmet.iter().map(|(s, _)| a.source(*s)).collect(),
        residual_mm: sol.residual,
    };
    if !matches!(
        state,
        SolveState::FullyConstrained | SolveState::Underconstrained
    ) {
        return Ok(Evaluation {
            candidate: None,
            diagnostics,
            measurements: BTreeMap::new(),
        });
    }
    let mut candidate = source.clone();
    for p in &mut candidate.points {
        p.position_mm = sol
            .point(a.entity(p.id)?)
            .ok_or_else(|| error(Some(p.id), "Missing solved point"))?;
        if p.position_mm
            .iter()
            .any(|v| !v.is_finite() || v.abs() > COORDINATE_LIMIT_MM)
        {
            return Err(error(
                Some(p.id),
                "Solved coordinate exceeds finite modeling limit",
            ));
        }
    }
    for c in &mut candidate.curves {
        if let CurveKind::BSpline { poles, .. } = &c.kind {
            let first = sol
                .point(a.entity(poles[0])?)
                .ok_or_else(|| error(Some(c.id), "Missing solved spline pole"))?;
            if poles.iter().all(|id| {
                sol.point(a.entity(*id).expect("mapped pole"))
                    .is_some_and(|p| (p[0] - first[0]).hypot(p[1] - first[1]) < 1e-7)
            }) {
                return Err(error(
                    Some(c.id),
                    "Solved spline control polygon is degenerate",
                ));
            }
            continue;
        }
        if let Some(r) = sol.radius(a.entity(c.id)?) {
            if !r.is_finite() || r <= 1e-7 || r > COORDINATE_LIMIT_MM {
                return Err(error(
                    Some(c.id),
                    "Solved radius is degenerate or outside limits",
                ));
            }
            if let CurveKind::Circle { radius_mm, .. } = &mut c.kind {
                *radius_mm = r;
            }
        }
        if let CurveKind::Line { start, end } = c.kind {
            let p = sol
                .point(a.entity(start)?)
                .ok_or_else(|| error(Some(c.id), "Missing line start"))?;
            let q = sol
                .point(a.entity(end)?)
                .ok_or_else(|| error(Some(c.id), "Missing line end"))?;
            if (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-7 {
                return Err(error(Some(c.id), "Solved line is degenerate"));
            }
        }
    }
    let measurements = candidate
        .constraints
        .iter()
        .filter(|c| dimension(&c.kind).is_some())
        .map(|c| {
            let d = dimension(&c.kind).expect("filtered dimensions");
            let unit = Unit::parse(d.unit()).ok_or_else(|| error(Some(c.id), "Unknown unit"))?;
            let (value, base) = measure(&c.kind, &a, &sol)?;
            Ok((
                c.id,
                Measurement {
                    value: value * base.scale() / unit.scale(),
                    unit: d.unit().into(),
                },
            ))
        })
        .collect::<Result<_, InputError>>()?;
    Ok(Evaluation {
        candidate: Some(candidate),
        diagnostics,
        measurements,
    })
}
fn measure(
    kind: &Kind,
    a: &Adapter,
    sol: &sketch_solver::Solution,
) -> Result<(f64, Unit), InputError> {
    let point = |id| {
        sol.point(a.entity(id)?)
            .ok_or_else(|| error(Some(id), "Measurement requires a point"))
    };
    let line = |id| -> Result<[[f64; 2]; 2], InputError> {
        match a.solver.entity(a.entity(id)?) {
            Some(sketch_solver::Entity::Line { start, end }) => Ok([
                sol.point(*start)
                    .ok_or_else(|| error(Some(id), "Missing start"))?,
                sol.point(*end)
                    .ok_or_else(|| error(Some(id), "Missing end"))?,
            ]),
            _ => Err(error(Some(id), "Measurement requires a line")),
        }
    };
    let direction = |id| -> Result<[f64; 2], InputError> {
        let [p, q] = line(id)?;
        Ok([q[0] - p[0], q[1] - p[1]])
    };
    let distance_line = |p: [f64; 2], id| -> Result<f64, InputError> {
        let [q, r] = line(id)?;
        let d = [r[0] - q[0], r[1] - q[1]];
        Ok(((p[0] - q[0]) * d[1] - (p[1] - q[1]) * d[0]).abs() / d[0].hypot(d[1]))
    };
    let value = match kind {
        Kind::Length { line, .. } => {
            let d = direction(*line)?;
            d[0].hypot(d[1])
        }
        Kind::Distance {
            a: x, b: y, axis, ..
        } => {
            if let (Ok(p), Ok(q)) = (point(*x), point(*y)) {
                match axis {
                    Axis::X => q[0] - p[0],
                    Axis::Y => q[1] - p[1],
                    Axis::Direct => (q[0] - p[0]).hypot(q[1] - p[1]),
                }
            } else if let Ok(p) = point(*x) {
                distance_line(p, *y)?
            } else if let Ok(p) = point(*y) {
                distance_line(p, *x)?
            } else {
                distance_line(line(*y)?[0], *x)?
            }
        }
        Kind::Radius { curve, .. } | Kind::Diameter { curve, .. } => {
            sol.radius(a.entity(*curve)?)
                .ok_or_else(|| error(Some(*curve), "Missing radius"))?
                * if matches!(kind, Kind::Diameter { .. }) {
                    2.
                } else {
                    1.
                }
        }
        Kind::Angle { from, to, .. } => {
            let x = direction(*from)?;
            let y = direction(*to)?;
            return Ok((
                (x[0] * y[1] - x[1] * y[0])
                    .atan2(x[0] * y[0] + x[1] * y[1])
                    .to_degrees(),
                Unit::Degree,
            ));
        }
        _ => return Err(error(None, "Not a dimensional constraint")),
    };
    if !value.is_finite() {
        return Err(error(None, "Nonfinite measurement"));
    }
    Ok((value, Unit::Millimeter))
}
