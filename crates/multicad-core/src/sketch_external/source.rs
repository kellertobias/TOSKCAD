use crate::{
    constraint_sketch::{
        Constraint, ConstraintKind, Curve, CurveKind, Dimension, Point, Sketch, source::Source,
    },
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    references::GeometryIntent,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const TYPE_ID: &str = "sketch.external";
pub const CAPABILITY: &str = "sketch.external.1";
pub const ROLE: &str = "external";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Coplanar,
    Intersect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Behavior {
    ReferenceOnly,
    Derived,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputIds {
    Point {
        point: EntityId,
        fix: EntityId,
    },
    Line {
        curve: EntityId,
        start: EntityId,
        end: EntityId,
        fixes: [EntityId; 2],
    },
    Circle {
        curve: EntityId,
        center: EntityId,
        fix: EntityId,
        radius: EntityId,
    },
    Arc {
        curve: EntityId,
        center: EntityId,
        start: EntityId,
        end: EntityId,
        fixes: [EntityId; 3],
    },
}
impl OutputIds {
    pub fn points(&self) -> Vec<EntityId> {
        match self {
            Self::Point { point, .. } => vec![*point],
            Self::Line { start, end, .. } => vec![*start, *end],
            Self::Circle { center, .. } => vec![*center],
            Self::Arc {
                center, start, end, ..
            } => vec![*center, *start, *end],
        }
    }
    pub fn curve(&self) -> Option<EntityId> {
        match self {
            Self::Point { .. } => None,
            Self::Line { curve, .. } | Self::Circle { curve, .. } | Self::Arc { curve, .. } => {
                Some(*curve)
            }
        }
    }
    pub fn locks(&self) -> Vec<EntityId> {
        match self {
            Self::Point { fix, .. } => vec![*fix],
            Self::Line { fixes, .. } => fixes.to_vec(),
            Self::Circle { fix, radius, .. } => vec![*fix, *radius],
            Self::Arc { fixes, .. } => fixes.to_vec(),
        }
    }
    pub fn all(&self) -> Vec<EntityId> {
        self.points()
            .into_iter()
            .chain(self.curve())
            .chain(self.locks())
            .collect()
    }
    pub(super) fn allocate(shape: &super::LocalGeometry) -> Self {
        match shape {
            super::LocalGeometry::Point(_) => Self::Point {
                point: EntityId::new(),
                fix: EntityId::new(),
            },
            super::LocalGeometry::Line(..) => Self::Line {
                curve: EntityId::new(),
                start: EntityId::new(),
                end: EntityId::new(),
                fixes: std::array::from_fn(|_| EntityId::new()),
            },
            super::LocalGeometry::Circle { .. } => Self::Circle {
                curve: EntityId::new(),
                center: EntityId::new(),
                fix: EntityId::new(),
                radius: EntityId::new(),
            },
            super::LocalGeometry::Arc { .. } => Self::Arc {
                curve: EntityId::new(),
                center: EntityId::new(),
                start: EntityId::new(),
                end: EntityId::new(),
                fixes: std::array::from_fn(|_| EntityId::new()),
            },
        }
    }
    pub(super) fn matching(&self, curve: Option<&Curve>) -> bool {
        match (self, curve.map(|c| &c.kind)) {
            (Self::Point { .. }, None) => true,
            (
                Self::Line {
                    start: a, end: b, ..
                },
                Some(CurveKind::Line { start, end }),
            ) => a == start && b == end,
            (Self::Circle { center: a, .. }, Some(CurveKind::Circle { center, .. })) => a == center,
            (
                Self::Arc {
                    center: a,
                    start: b,
                    end: c,
                    ..
                },
                Some(CurveKind::Arc {
                    center, start, end, ..
                }),
            ) => a == center && b == start && c == end,
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalDefinition {
    pub method: Method,
    pub behavior: Behavior,
    pub output: OutputIds,
}
impl ExternalDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: TYPE_ID.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(invalid)?,
        })
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        let ids = self.output.all();
        let unique: BTreeSet<_> = ids.iter().copied().collect();
        if ids.iter().any(|id| id.is_nil()) || unique.len() != ids.len() {
            return Err(invalid(
                "External output identities must be distinct and non-nil",
            ));
        }
        if self.method == Method::Intersect && !matches!(self.output, OutputIds::Point { .. }) {
            return Err(invalid(
                "Intersection currently supports a finite line crossing the sketch plane, producing one point",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub reference: EntityId,
    pub source: EntityId,
    pub intent: GeometryIntent,
    pub definition: ExternalDefinition,
}
pub fn read(document: &Document, id: EntityId) -> Result<Binding, DocumentError> {
    let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
    let EntityData::Extension { definition, inputs } = &entity.data else {
        return Err(invalid(
            "External reference source record is missing or has the wrong kind",
        ));
    };
    if definition.type_id != TYPE_ID || definition.schema_version != 1 {
        return Err(invalid(
            "External reference provider/version is unavailable",
        ));
    }
    let value: ExternalDefinition =
        serde_json::from_value(definition.data.clone()).map_err(invalid)?;
    value.validate()?;
    if inputs.len() != 1 || inputs[0].role != "source" {
        return Err(invalid(
            "External reference requires one canonical source link",
        ));
    }
    let link = &inputs[0];
    let intent = link
        .geometry
        .clone()
        .ok_or_else(|| invalid("External source link has no persistent geometry intent"))?;
    intent.validate().map_err(invalid)?;
    Ok(Binding {
        reference: id,
        source: link.target,
        intent,
        definition: value,
    })
}
pub fn bindings(document: &Document, sketch: EntityId) -> Result<Vec<Binding>, DocumentError> {
    let entity = document.get(sketch).ok_or(DocumentError::Missing(sketch))?;
    let EntityData::Sketch { inputs, .. } = &entity.data else {
        return Err(invalid("Select a sketch"));
    };
    inputs
        .iter()
        .filter(|link| link.role == ROLE && !super::is_occurrence_reference(document, link.target))
        .map(|link| read(document, link.target))
        .collect()
}
pub fn link(reference: EntityId) -> Link {
    Link::new(reference, ROLE)
}
pub fn required_capability(document: &Document) -> bool {
    document.entities().any(
        |e| matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==TYPE_ID),
    )
}
pub fn validate(document: &Document) -> Result<(), DocumentError> {
    if document
        .entities()
        .filter(|e| super::is_occurrence_reference(document, e.id))
        .count()
        > super::MAX_OCCURRENCE_BINDINGS
    {
        return Err(invalid(
            "Occurrence sketch reference count exceeds its 4096-binding budget",
        ));
    }
    if !document.entities().any(|entity| match &entity.data {
        EntityData::Extension { definition, .. } => definition.type_id == TYPE_ID,
        EntityData::Sketch { inputs, .. } => inputs.iter().any(|link| link.role == ROLE),
        _ => false,
    }) {
        return Ok(());
    }
    let mut owned = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    let mut occurrences = std::collections::BTreeMap::<EntityId, usize>::new();
    for entity in document.entities() {
        if let EntityData::Sketch { definition, .. } = &entity.data
            && definition.type_id == crate::constraint_sketch::source::TYPE
        {
            let source = Source::from_definition(definition)?;
            for id in source
                .sketch
                .points
                .iter()
                .map(|p| p.id)
                .chain(source.sketch.curves.iter().map(|c| c.id))
                .chain(source.sketch.constraints.iter().map(|c| c.id))
            {
                *occurrences.entry(id).or_default() += 1;
            }
        }
    }
    for entity in document.entities() {
        if matches!(&entity.data,EntityData::Extension{definition,..}if definition.type_id==TYPE_ID)
        {
            if super::is_occurrence_reference(document, entity.id) {
                super::read_occurrence(document, entity.id)?;
            } else {
                read(document, entity.id)?;
            }
        }
    }
    for entity in document.entities() {
        let EntityData::Sketch {
            definition, inputs, ..
        } = &entity.data
        else {
            continue;
        };
        for link in inputs {
            if matches!(document.get(link.target).map(|entity|&entity.data),Some(EntityData::Extension{definition,..}) if definition.type_id==TYPE_ID)
                && link.role != ROLE
            {
                return Err(invalid(
                    "A sketch external reference must use its canonical owner link role",
                ));
            }
        }
        let external: Vec<_> = inputs.iter().filter(|link| link.role == ROLE).collect();
        if external.is_empty() {
            continue;
        }
        if definition.type_id != crate::constraint_sketch::source::TYPE
            || definition.schema_version != 4
        {
            return Err(invalid(
                "External sketch references require constrained sketch schema 4",
            ));
        }
        let source = Source::from_definition(definition)?;
        let mut all: BTreeSet<_> = source
            .sketch
            .points
            .iter()
            .map(|p| p.id)
            .chain(source.sketch.curves.iter().map(|c| c.id))
            .chain(source.sketch.constraints.iter().map(|c| c.id))
            .collect();
        for link in external {
            if link.geometry.is_some() || !owned.insert(link.target) {
                return Err(invalid(
                    "External reference must have exactly one owning sketch and a plain owner link",
                ));
            }
            if document.get(link.target).is_none() {
                continue;
            }
            if super::is_occurrence_reference(document, link.target) {
                let request = super::read_occurrence(document, link.target)?;
                if request.query.sketch != entity.id
                    || !super::occurrence_source::matching(&request, &source)
                {
                    return Err(invalid(
                        "Protected occurrence output or owning sketch changed",
                    ));
                }
                for id in request.output.all() {
                    if !outputs.insert(id)
                        || document.get(id).is_some()
                        || id == document.id()
                        || occurrences.get(&id).copied().unwrap_or(0) > 1
                    {
                        return Err(invalid(
                            "Occurrence output identity collides with canonical source",
                        ));
                    }
                }
                for lock in &request.output.locks {
                    if !all.insert(*lock) || occurrences.contains_key(lock) {
                        return Err(invalid(
                            "Occurrence native solver locks cannot appear in canonical source",
                        ));
                    }
                }
                continue;
            }
            let binding = read(document, link.target)?;
            let output = &binding.definition.output;
            for id in output.points() {
                if !source.sketch.points.iter().any(|p| p.id == id) {
                    return Err(invalid(
                        "A protected external point was removed; remove the reference atomically instead",
                    ));
                }
            }
            let curve = output
                .curve()
                .and_then(|id| source.sketch.curves.iter().find(|curve| curve.id == id));
            if !output.matching(curve)
                || curve.is_some_and(|curve| {
                    curve.construction != (binding.definition.behavior == Behavior::ReferenceOnly)
                })
            {
                return Err(invalid(
                    "Protected external curve family, point ownership or construction behavior changed",
                ));
            }
            for id in output.all() {
                if !outputs.insert(id) || document.get(id).is_some() || id == document.id() {
                    return Err(invalid(
                        "External output identity is shared or collides with document identity",
                    ));
                }
                if occurrences.get(&id).copied().unwrap_or(0) > 1 {
                    return Err(invalid(
                        "External output identity also occurs in another sketch",
                    ));
                }
            }
            for lock in output.locks() {
                if !all.insert(lock) || occurrences.contains_key(&lock) {
                    return Err(invalid(
                        "External solver locks cannot appear in canonical sketch source",
                    ));
                }
            }
        }
    }
    for entity in document.entities() {
        if matches!(&entity.data,EntityData::Extension{definition,..}if definition.type_id==TYPE_ID)
            && !owned.contains(&entity.id)
        {
            if super::is_occurrence_reference(document, entity.id) {
                let request = super::read_occurrence(document, entity.id)?;
                if request
                    .output
                    .all()
                    .iter()
                    .any(|id| occurrences.contains_key(id))
                {
                    return Err(invalid(
                        "Occurrence owner link was removed while protected output remains",
                    ));
                }
                continue;
            }
            let binding = read(document, entity.id)?;
            if binding
                .definition
                .output
                .all()
                .iter()
                .any(|id| occurrences.contains_key(id))
            {
                return Err(invalid(
                    "External owner link was removed while its protected output remains. Remove the reference and output atomically",
                ));
            }
        }
    }
    Ok(())
}
/// Authored guesses for protected outputs do not become direct-edit targets.
/// Their evaluated position belongs exclusively to the current source geometry.
pub fn validate_source_edit(
    document: &Document,
    sketch: EntityId,
    next: &Source,
) -> Result<(), DocumentError> {
    let Some(entity) = document.get(sketch) else {
        return Ok(());
    };
    let EntityData::Sketch {
        definition, inputs, ..
    } = &entity.data
    else {
        return Ok(());
    };
    if !inputs.iter().any(|l| l.role == ROLE) {
        return Ok(());
    }
    let old = Source::from_definition(definition)?;
    for request in super::occurrence_bindings(document, sketch)? {
        if request.output.points.iter().any(|id| {
            old.sketch.points.iter().find(|p| p.id == *id)
                != next.sketch.points.iter().find(|p| p.id == *id)
        }) || request.output.curve.is_some_and(|id| {
            old.sketch.curves.iter().find(|c| c.id == id)
                != next.sketch.curves.iter().find(|c| c.id == id)
        }) {
            return Err(invalid(
                "Occurrence external output is read-only; edit its source or explicitly repair/remove the reference",
            ));
        }
    }
    for binding in bindings(document, sketch)? {
        for id in binding.definition.output.points() {
            if old.sketch.points.iter().find(|p| p.id == id)
                != next.sketch.points.iter().find(|p| p.id == id)
            {
                return Err(invalid(
                    "External reference points are read-only; edit the source or remove the reference",
                ));
            }
        }
        if let Some(id) = binding.definition.output.curve()
            && old.sketch.curves.iter().find(|c| c.id == id)
                != next.sketch.curves.iter().find(|c| c.id == id)
        {
            return Err(invalid(
                "External reference curves are read-only; edit the source or remove the reference",
            ));
        }
    }
    Ok(())
}
pub(super) fn install(
    sketch: &mut Sketch,
    binding: &ExternalDefinition,
    shape: &super::LocalGeometry,
    append: bool,
) -> Result<(), DocumentError> {
    let construction = binding.behavior == Behavior::ReferenceOnly;
    let (points, kind) = match (&binding.output, shape) {
        (OutputIds::Point { point, .. }, super::LocalGeometry::Point(p)) => {
            (vec![(*point, *p)], None)
        }
        (OutputIds::Line { start, end, .. }, super::LocalGeometry::Line(a, b)) => (
            vec![(*start, *a), (*end, *b)],
            Some(CurveKind::Line {
                start: *start,
                end: *end,
            }),
        ),
        (OutputIds::Circle { center, .. }, super::LocalGeometry::Circle { center: c, radius }) => (
            vec![(*center, *c)],
            Some(CurveKind::Circle {
                center: *center,
                radius_mm: *radius,
            }),
        ),
        (
            OutputIds::Arc {
                center, start, end, ..
            },
            super::LocalGeometry::Arc {
                center: c,
                start: a,
                end: b,
                clockwise,
            },
        ) => (
            vec![(*center, *c), (*start, *a), (*end, *b)],
            Some(CurveKind::Arc {
                center: *center,
                start: *start,
                end: *end,
                clockwise: *clockwise,
            }),
        ),
        _ => {
            return Err(invalid(
                "External source changed geometry family; repair or replace this reference explicitly",
            ));
        }
    };
    for (id, position_mm) in points {
        if append {
            sketch.points.push(Point { id, position_mm });
        } else {
            sketch
                .points
                .iter_mut()
                .find(|point| point.id == id)
                .ok_or_else(|| invalid("External output point is missing"))?
                .position_mm = position_mm;
        }
    }
    if let Some(kind) = kind {
        let id = binding.output.curve().expect("curve output");
        if append {
            sketch.curves.push(Curve {
                id,
                construction,
                kind,
            });
        } else {
            sketch
                .curves
                .iter_mut()
                .find(|curve| curve.id == id)
                .ok_or_else(|| invalid("External output curve is missing"))?
                .kind = kind;
        }
    }
    Ok(())
}
pub(super) fn locks(binding: &ExternalDefinition, shape: &super::LocalGeometry) -> Vec<Constraint> {
    let output = &binding.output;
    let mut result: Vec<_> = output
        .points()
        .into_iter()
        .zip(output.locks())
        .map(|(point, id)| Constraint {
            id,
            reference: false,
            kind: ConstraintKind::Fix { point },
        })
        .collect();
    if let (
        OutputIds::Circle { curve, radius, .. },
        super::LocalGeometry::Circle { radius: value, .. },
    ) = (output, shape)
    {
        result.push(Constraint {
            id: *radius,
            reference: false,
            kind: ConstraintKind::Radius {
                curve: *curve,
                dimension: Dimension::Literal {
                    value: *value,
                    unit: "mm".into(),
                },
            },
        });
    }
    result
}
pub(super) fn invalid(error: impl std::fmt::Display) -> DocumentError {
    DocumentError::Invalid(error.to_string())
}
