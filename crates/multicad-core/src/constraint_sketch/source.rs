//! Versioned canonical payload. Geometry handles and derived diagnostics are
//! deliberately absent; evaluators reconstruct them from this source.
use super::*;
use crate::{
    document::{Definition, Document, DocumentError, EntityData, Link, Mutation},
    parameters::Unit,
    sketch::{self, Frame},
};
use std::collections::BTreeSet;
pub const TYPE: &str = "core.constrained_sketch";
pub const V2_CAPABILITY: &str = "core.constrained_sketch.2";
const DIMENSION_LINK_PREFIX: &str = "dimension/";
pub fn requires_v2(document: &Document) -> bool {
    document.entities().any(|e|matches!(&e.data,EntityData::Sketch {definition,..} if definition.type_id==TYPE&&definition.schema_version==2))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub u: [f64; 3],
    pub sketch: Sketch,
}
fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
impl Source {
    /// Explicit source upgrade for older source snapshots; undo restores the
    /// original provider. A rectangle becomes four shared-corner lines.
    pub fn from_legacy(document: &Document, id: EntityId) -> Result<Self, DocumentError> {
        let (legacy, _, _) = sketch::Sketch::read(document, id)?;
        let mut source = Self {
            u: legacy.u,
            sketch: Sketch::default(),
        };
        for g in legacy.geometry {
            let id = g.id();
            match g {
                sketch::Geometry::Line { a, b, .. } => {
                    let start = source.add_point(a);
                    let end = source.add_point(b);
                    source.sketch.curves.push(Curve {
                        id,
                        construction: false,
                        kind: CurveKind::Line { start, end },
                    });
                }
                sketch::Geometry::Rectangle { a, b, .. } => {
                    let first = source.add_rectangle(a, b)?;
                    for curve in &mut source.sketch.curves {
                        if curve.id == first {
                            curve.id = id;
                        }
                    }
                    for constraint in &mut source.sketch.constraints {
                        if let ConstraintKind::Horizontal { line } = &mut constraint.kind
                            && *line == first
                        {
                            *line = id;
                        }
                    }
                }
                sketch::Geometry::Circle {
                    center, radius_mm, ..
                } => {
                    let center = source.add_point(center);
                    source.sketch.curves.push(Curve {
                        id,
                        construction: false,
                        kind: CurveKind::Circle { center, radius_mm },
                    });
                }
                sketch::Geometry::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                    ..
                } => {
                    let center = source.add_point(center);
                    let start = source.add_point(a);
                    let end = source.add_point(b);
                    source.sketch.curves.push(Curve {
                        id,
                        construction: false,
                        kind: CurveKind::Arc {
                            center,
                            start,
                            end,
                            clockwise,
                        },
                    });
                }
            }
        }
        source.validate()?;
        Ok(source)
    }
    pub fn create(
        datum: sketch::Datum,
        name: &str,
    ) -> Result<(EntityId, Vec<Mutation>), DocumentError> {
        let (id, mut edits) = sketch::create(datum, name)?;
        let source = Self {
            u: datum.frame().u,
            sketch: Sketch::default(),
        };
        for edit in &mut edits {
            if let Mutation::Insert(entity) = edit
                && entity.id == id
                && let EntityData::Sketch { definition, .. } = &mut entity.data
            {
                *definition = source.definition()?;
            }
        }
        Ok((id, edits))
    }
    /// Render/query facade for both source providers. Never write this derived
    /// view back over a constrained sketch: edits use the solver transaction.
    pub fn display_geometry(&self) -> Result<Vec<sketch::Geometry>, DocumentError> {
        if self.has_expressions() {
            return Err(invalid(
                "Parameter-driven sketch requires current evaluated geometry; authored point guesses are not a display or manufacturing result",
            ));
        }
        let p = |id| {
            self.sketch
                .points
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.position_mm)
                .ok_or_else(|| invalid("Curve point is missing"))
        };
        self.sketch
            .curves
            .iter()
            .map(|c| {
                Ok(match c.kind {
                    CurveKind::Line { start, end } => sketch::Geometry::Line {
                        id: c.id,
                        a: p(start)?,
                        b: p(end)?,
                    },
                    CurveKind::Circle { center, radius_mm } => sketch::Geometry::Circle {
                        id: c.id,
                        center: p(center)?,
                        radius_mm,
                    },
                    CurveKind::Arc {
                        center,
                        start,
                        end,
                        clockwise,
                    } => sketch::Geometry::Arc {
                        id: c.id,
                        center: p(center)?,
                        a: p(start)?,
                        b: p(end)?,
                        clockwise,
                    },
                })
            })
            .collect()
    }
    pub fn add_point(&mut self, position_mm: [f64; 2]) -> EntityId {
        let id = EntityId::new();
        self.sketch.points.push(Point { id, position_mm });
        id
    }
    pub fn add_line(&mut self, a: EntityId, b: EntityId) -> EntityId {
        let id = EntityId::new();
        self.sketch.curves.push(Curve {
            id,
            construction: false,
            kind: CurveKind::Line { start: a, end: b },
        });
        id
    }
    pub fn add_constraint(&mut self, kind: ConstraintKind) -> EntityId {
        let id = EntityId::new();
        self.sketch.constraints.push(Constraint {
            id,
            reference: false,
            kind,
        });
        id
    }
    /// Rectangle construction uses shared corner identities and four geometric
    /// constraints. Dimensions remain free until the author adds them.
    pub fn add_rectangle(&mut self, a: [f64; 2], b: [f64; 2]) -> Result<EntityId, DocumentError> {
        if a.iter().chain(&b).any(|v| !v.is_finite() || v.abs() > 1e9)
            || (a[0] - b[0]).abs() < 1e-7
            || (a[1] - b[1]).abs() < 1e-7
        {
            return Err(invalid("Rectangle sides must be nonzero"));
        }
        let points = [a, [b[0], a[1]], b, [a[0], b[1]]].map(|p| self.add_point(p));
        let mut lines = Vec::new();
        for i in 0..4 {
            let line = self.add_line(points[i], points[(i + 1) % 4]);
            lines.push(line);
            self.add_constraint(if i % 2 == 0 {
                ConstraintKind::Horizontal { line }
            } else {
                ConstraintKind::Vertical { line }
            });
        }
        Ok(lines[0])
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != TYPE || !(1..=2).contains(&definition.schema_version) {
            return Err(invalid("Unsupported constrained sketch provider/version"));
        }
        let source: Self = serde_json::from_value(definition.data.clone())
            .map_err(|e| invalid(format!("Invalid constrained sketch source: {e}")))?;
        source.validate()?;
        if definition.schema_version == 1 && source.has_expressions() {
            return Err(invalid(
                "Parameter-driven dimensions require constrained sketch schema 2",
            ));
        }
        Ok(source)
    }
    /// Literal-only new sketches remain schema 1 during the evaluated-UI
    /// transition. `replace` preserves schema 2 after its first introduction,
    /// including when the final expression is later replaced by a literal.
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: TYPE.into(),
            schema_version: if self.has_expressions() { 2 } else { 1 },
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    /// Structural validation belongs to the source boundary. Numerical solving
    /// belongs to the adapter and runs before its candidate is published.
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.u.iter().any(|v| !v.is_finite()) || (sketch::dot(self.u, self.u) - 1.).abs() > 1e-9
        {
            return Err(invalid("Invalid constrained sketch basis"));
        }
        if self.sketch.points.len() > 256
            || self.sketch.curves.len() > 512
            || self.sketch.constraints.len() > 1024
        {
            return Err(invalid("Constrained sketch exceeds interactive limits"));
        }
        let mut ids = BTreeSet::new();
        for id in self
            .sketch
            .points
            .iter()
            .map(|p| p.id)
            .chain(self.sketch.curves.iter().map(|c| c.id))
            .chain(self.sketch.constraints.iter().map(|c| c.id))
        {
            if id.is_nil() || !ids.insert(id) {
                return Err(invalid("Duplicate or nil constrained sketch identity"));
            }
        }
        let points: BTreeSet<_> = self.sketch.points.iter().map(|p| p.id).collect();
        let geometry: BTreeSet<_> = points
            .iter()
            .copied()
            .chain(self.sketch.curves.iter().map(|c| c.id))
            .collect();
        for point in &self.sketch.points {
            if point
                .position_mm
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1e9)
            {
                return Err(invalid("Invalid constrained sketch coordinate"));
            }
        }
        for curve in &self.sketch.curves {
            let refs = match curve.kind {
                CurveKind::Line { start, end } => vec![start, end],
                CurveKind::Arc {
                    center, start, end, ..
                } => vec![center, start, end],
                CurveKind::Circle { center, radius_mm } => {
                    if !radius_mm.is_finite() || !(1e-7..=1e9).contains(&radius_mm) {
                        return Err(invalid("Invalid circle radius"));
                    }
                    vec![center]
                }
            };
            if refs.iter().any(|id| !points.contains(id)) {
                return Err(invalid("Curve references a missing point"));
            }
        }
        for constraint in &self.sketch.constraints {
            let (refs, dimension, angle) = match &constraint.kind {
                ConstraintKind::Fix { point } => (vec![*point], None, false),
                ConstraintKind::Horizontal { line } | ConstraintKind::Vertical { line } => {
                    (vec![*line], None, false)
                }
                ConstraintKind::Coincident { a, b }
                | ConstraintKind::Parallel { a, b }
                | ConstraintKind::Perpendicular { a, b }
                | ConstraintKind::Tangent { a, b }
                | ConstraintKind::Equal { a, b } => (vec![*a, *b], None, false),
                ConstraintKind::Symmetric { a, b, about } => (vec![*a, *b, *about], None, false),
                ConstraintKind::Length { line, dimension } => (vec![*line], Some(dimension), false),
                ConstraintKind::Radius { curve, dimension }
                | ConstraintKind::Diameter { curve, dimension } => {
                    (vec![*curve], Some(dimension), false)
                }
                ConstraintKind::Distance {
                    a, b, dimension, ..
                } => (vec![*a, *b], Some(dimension), false),
                ConstraintKind::Angle {
                    from,
                    to,
                    dimension,
                } => (vec![*from, *to], Some(dimension), true),
            };
            if refs.iter().any(|id| !geometry.contains(id)) {
                return Err(invalid("Constraint references missing geometry"));
            }
            if constraint.reference && dimension.is_none() {
                return Err(invalid("Only dimensions can be reference-only"));
            }
            if let Some(d) = dimension {
                let unit = Unit::parse(d.unit())
                    .ok_or_else(|| invalid("Unknown sketch dimension unit"))?;
                let expected = if angle {
                    Unit::Degree
                } else {
                    Unit::Millimeter
                };
                if unit.dimension() != expected.dimension() {
                    return Err(invalid("Invalid sketch dimension units/value"));
                }
                match d {
                    Dimension::Literal { .. } => {
                        d.value_in(expected, None).map_err(invalid)?;
                    }
                    Dimension::Expression { expression } => {
                        expression.validate().map_err(|e| invalid(e.to_string()))?
                    }
                }
            }
        }
        Ok(())
    }
    pub fn read(
        document: &Document,
        id: EntityId,
    ) -> Result<(Self, Frame, EntityId), DocumentError> {
        Self::read_impl(document, id, None, false)
    }
    pub fn read_evaluated(
        document: &Document,
        id: EntityId,
        revision: u64,
        report: &crate::references::ReferenceReport,
    ) -> Result<(Self, Frame, EntityId), DocumentError> {
        Self::read_impl(document, id, Some((revision, report)), false)
    }
    /// Canonical source plus a guarded local plane for a numerical evaluator.
    /// Callers must solve dimensions before displaying or manufacturing v2 data.
    pub fn read_for_evaluation(
        document: &Document,
        id: EntityId,
        revision: u64,
        report: Option<&crate::references::ReferenceReport>,
    ) -> Result<(Self, Frame, EntityId), DocumentError> {
        Self::read_impl(document, id, report.map(|r| (revision, r)), true)
    }
    fn read_impl(
        document: &Document,
        id: EntityId,
        evaluated: Option<(u64, &crate::references::ReferenceReport)>,
        allow_semantic: bool,
    ) -> Result<(Self, Frame, EntityId), DocumentError> {
        let entity = document
            .get(id)
            .ok_or_else(|| invalid("Sketch does not exist"))?;
        let EntityData::Sketch {
            plane, definition, ..
        } = &entity.data
        else {
            return Err(invalid("Select a sketch"));
        };
        let source = Self::from_definition(definition)?;
        if definition.schema_version == 2 && !allow_semantic {
            return Err(invalid(
                "Constrained sketch schema 2 requires an EvaluatedSketch; raw authored geometry is unavailable",
            ));
        }
        let Some(EntityData::Plane { normal, .. }) = document.get(*plane).map(|e| &e.data) else {
            return Err(invalid("Sketch plane is missing"));
        };
        if sketch::dot(source.u, *normal).abs() > 1e-9 {
            return Err(invalid("Sketch basis must be orthogonal to its plane"));
        }
        let frame = crate::references::plane_frame(document, *plane, source.u, evaluated)?;
        Ok((source, frame, *plane))
    }
    pub fn replace(
        &self,
        document: &Document,
        id: EntityId,
    ) -> Result<Vec<Mutation>, DocumentError> {
        let mut entity = document
            .get(id)
            .ok_or_else(|| invalid("Sketch is missing"))?
            .clone();
        let EntityData::Sketch {
            definition, inputs, ..
        } = &mut entity.data
        else {
            return Err(invalid("Select a sketch"));
        };
        let previous_version = definition.schema_version;
        *definition = self.definition()?;
        if previous_version == 2 {
            definition.schema_version = 2;
        }
        inputs.retain(|link| !link.role.starts_with(DIMENSION_LINK_PREFIX));
        inputs.extend(self.dimension_links());
        Ok(vec![Mutation::Replace(entity)])
    }
    pub fn has_expressions(&self) -> bool {
        self.sketch
            .constraints
            .iter()
            .any(|c| c.kind.dimension().is_some_and(|d| d.expression().is_some()))
    }
    pub fn dimension_links(&self) -> Vec<Link> {
        self.sketch
            .constraints
            .iter()
            .flat_map(|c| {
                c.kind
                    .dimension()
                    .and_then(Dimension::expression)
                    .into_iter()
                    .flat_map(move |e| {
                        e.bindings.iter().map(move |b| {
                            Link::new(
                                b.target,
                                format!("{DIMENSION_LINK_PREFIX}{}/{}", c.id, b.alias),
                            )
                        })
                    })
            })
            .collect()
    }
}
pub(crate) fn validate(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        if let EntityData::Sketch {
            plane,
            definition,
            inputs,
        } = &entity.data
            && definition.type_id == TYPE
        {
            let source = Source::from_definition(definition)?;
            let expected = source.dimension_links();
            let actual: Vec<_> = inputs
                .iter()
                .filter(|l| l.role.starts_with(DIMENSION_LINK_PREFIX))
                .cloned()
                .collect();
            if actual.len() != expected.len() || expected.iter().any(|link| !actual.contains(link))
            {
                return Err(invalid(
                    "Sketch expression bindings and canonical dependency links disagree",
                ));
            }
            for link in expected {
                if document
                    .get(link.target)
                    .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
                {
                    return Err(invalid(
                        "Sketch expression binding targets a non-parameter entity",
                    ));
                }
            }
            if let Some(EntityData::Plane { normal, .. }) = document.get(*plane).map(|e| &e.data)
                && sketch::dot(source.u, *normal).abs() > 1e-9
            {
                return Err(invalid("Sketch basis must be orthogonal to its plane"));
            }
        }
    }
    Ok(())
}
