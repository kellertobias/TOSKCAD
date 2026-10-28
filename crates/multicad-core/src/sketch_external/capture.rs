use super::{
    Behavior, ExternalDefinition, Method, OutputIds,
    source::{install, invalid},
};
use crate::{
    constraint_sketch::{ConstraintKind, source::Source},
    document::{Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    references::{self, GeometryIntent, Resolution, TopologySnapshot},
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureInput {
    pub sketch: EntityId,
    pub source: EntityId,
    pub intent: GeometryIntent,
    pub method: Method,
    pub behavior: Behavior,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRequest {
    pub reference: EntityId,
    pub input: CaptureInput,
    pub output: OutputIds,
}
impl CaptureRequest {
    /// The topology map must originate from the current source-bound native model.
    pub fn new(
        document: &Document,
        revision: u64,
        input: CaptureInput,
        topologies: &BTreeMap<EntityId, TopologySnapshot>,
    ) -> Result<Self, DocumentError> {
        let reference = EntityId::new();
        let shape = geometry(document, revision, &input, topologies)?;
        Ok(Self {
            reference,
            input,
            output: OutputIds::allocate(&shape),
        })
    }
}
fn geometry(
    document: &Document,
    revision: u64,
    input: &CaptureInput,
    topologies: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<super::LocalGeometry, DocumentError> {
    if !matches!(
        document.get(input.source).map(|entity| &entity.data),
        Some(EntityData::Feature { .. })
    ) {
        return Err(invalid(
            "Choose a source feature with exact current topology. Placed occurrence references require the occurrence adapter",
        ));
    }
    let report = references::inspect(document, revision, topologies);
    let (_, frame, _) =
        Source::read_for_evaluation(document, input.sketch, revision, Some(&report))?;
    let snapshot = topologies
        .get(&input.source)
        .ok_or_else(|| invalid("Selected source has no current native topology"))?;
    if snapshot.owner != input.source {
        return Err(invalid("Native topology belongs to a different source"));
    }
    let geometry = match references::resolve(&input.intent, snapshot, revision) {
        Resolution::Resolved {
            geometry: Some(geometry),
            ..
        } => geometry,
        ref other => {
            return Err(invalid(format!(
                "Cannot capture unresolved or unsupported native geometry: {other:?}"
            )));
        }
    };
    super::project(geometry, frame, input.method)
}
/// Source edits only. A solver adapter must evaluate the complete candidate
/// with fresh native references before exposing a commit-capable transaction.
pub fn capture_edits(
    document: &Document,
    revision: u64,
    request: &CaptureRequest,
    topologies: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<Vec<Mutation>, DocumentError> {
    if request.reference.is_nil() || document.get(request.reference).is_some() {
        return Err(invalid(
            "External reference identity already exists or is nil",
        ));
    }
    let shape = geometry(document, revision, &request.input, topologies)?;
    let definition = ExternalDefinition {
        method: request.input.method,
        behavior: request.input.behavior,
        output: request.output.clone(),
    };
    definition.validate()?;
    let entity = document
        .get(request.input.sketch)
        .ok_or(DocumentError::Missing(request.input.sketch))?;
    let EntityData::Sketch {
        definition: old, ..
    } = &entity.data
    else {
        return Err(invalid("Choose a sketch"));
    };
    let mut source = Source::from_definition(old)?;
    install(&mut source.sketch, &definition, &shape, true)?;
    source.validate()?;
    let mut edits = source.replace(document, request.input.sketch)?;
    for mutation in &mut edits {
        if let Mutation::Replace(entity) = mutation
            && let EntityData::Sketch {
                definition, inputs, ..
            } = &mut entity.data
        {
            definition.schema_version = 4;
            inputs.push(super::link(request.reference));
        }
    }
    let mut link = Link::new(request.input.source, "source");
    link.geometry = Some(request.input.intent.clone());
    let mut external = Entity::new(
        "External sketch reference",
        EntityData::Extension {
            definition: definition.definition()?,
            inputs: vec![link],
        },
    );
    external.id = request.reference;
    edits.insert(0, Mutation::Insert(external));
    Ok(edits)
}
/// Refuse deleting dependents implicitly. Users can first delete their chosen
/// constraints/curves, then remove the reference and its protected output atomically.
pub fn remove_edits(
    document: &Document,
    sketch: EntityId,
    reference: EntityId,
) -> Result<Vec<Mutation>, DocumentError> {
    let binding = super::bindings(document, sketch)?
        .into_iter()
        .find(|b| b.reference == reference)
        .ok_or_else(|| invalid("Reference is not owned by this sketch"))?;
    let mut entity = document
        .get(sketch)
        .ok_or(DocumentError::Missing(sketch))?
        .clone();
    let EntityData::Sketch {
        definition, inputs, ..
    } = &mut entity.data
    else {
        unreachable!()
    };
    let mut source = Source::from_definition(definition)?;
    let points: BTreeSet<_> = binding.definition.output.points().into_iter().collect();
    let curve = binding.definition.output.curve();
    for other in &source.sketch.curves {
        if Some(other.id) != curve
            && curve_points(&other.kind)
                .iter()
                .any(|id| points.contains(id))
        {
            return Err(invalid(
                "Other sketch geometry depends on the external points; remove those dependents before removing this reference",
            ));
        }
    }
    let removed: BTreeSet<_> = points.iter().copied().chain(curve).collect();
    for constraint in &source.sketch.constraints {
        if constraint_ids(&constraint.kind)
            .iter()
            .any(|id| removed.contains(id))
        {
            return Err(invalid(format!(
                "Constraint {} depends on this reference; remove that constraint explicitly first",
                constraint.id
            )));
        }
    }
    source
        .sketch
        .points
        .retain(|point| !points.contains(&point.id));
    source.sketch.curves.retain(|other| Some(other.id) != curve);
    source.validate()?;
    let version = definition.schema_version;
    *definition = source.definition()?;
    definition.schema_version = version.max(4);
    inputs.retain(|link| !(link.role == super::ROLE && link.target == reference));
    Ok(vec![Mutation::Replace(entity), Mutation::Remove(reference)])
}
/// Explicitly change profile participation while retaining source/output IDs
/// and constraints attached to them. A solver adapter validates the candidate.
pub fn behavior_edits(
    document: &Document,
    sketch: EntityId,
    reference: EntityId,
    behavior: Behavior,
) -> Result<Vec<Mutation>, DocumentError> {
    let mut binding = super::bindings(document, sketch)?
        .into_iter()
        .find(|binding| binding.reference == reference)
        .ok_or_else(|| invalid("Reference is not owned by this sketch"))?;
    binding.definition.behavior = behavior;
    let mut external = document
        .get(reference)
        .ok_or(DocumentError::Missing(reference))?
        .clone();
    let EntityData::Extension { definition, .. } = &mut external.data else {
        unreachable!()
    };
    *definition = binding.definition.definition()?;
    let mut entity = document
        .get(sketch)
        .ok_or(DocumentError::Missing(sketch))?
        .clone();
    let EntityData::Sketch { definition, .. } = &mut entity.data else {
        unreachable!()
    };
    let mut source = Source::from_definition(definition)?;
    if let Some(id) = binding.definition.output.curve() {
        let curve = source
            .sketch
            .curves
            .iter_mut()
            .find(|curve| curve.id == id)
            .ok_or_else(|| invalid("Protected external curve is missing"))?;
        curve.construction = behavior == Behavior::ReferenceOnly;
    }
    let version = definition.schema_version;
    *definition = source.definition()?;
    definition.schema_version = version.max(4);
    Ok(vec![Mutation::Replace(external), Mutation::Replace(entity)])
}
fn curve_points(curve: &crate::constraint_sketch::CurveKind) -> Vec<EntityId> {
    use crate::constraint_sketch::CurveKind;
    match curve {
        CurveKind::Line { start, end } => vec![*start, *end],
        CurveKind::Circle { center, .. } => vec![*center],
        CurveKind::Arc {
            center, start, end, ..
        } => vec![*center, *start, *end],
        CurveKind::BSpline { poles, .. } => poles.clone(),
    }
}
fn constraint_ids(kind: &ConstraintKind) -> Vec<EntityId> {
    match kind {
        ConstraintKind::Fix { point } => vec![*point],
        ConstraintKind::Horizontal { line }
        | ConstraintKind::Vertical { line }
        | ConstraintKind::Length { line, .. } => vec![*line],
        ConstraintKind::Radius { curve, .. } | ConstraintKind::Diameter { curve, .. } => {
            vec![*curve]
        }
        ConstraintKind::Coincident { a, b }
        | ConstraintKind::Parallel { a, b }
        | ConstraintKind::Perpendicular { a, b }
        | ConstraintKind::Tangent { a, b }
        | ConstraintKind::Equal { a, b }
        | ConstraintKind::Distance { a, b, .. } => vec![*a, *b],
        ConstraintKind::Symmetric { a, b, about } => vec![*a, *b, *about],
        ConstraintKind::Angle { from, to, .. } => vec![*from, *to],
    }
}
