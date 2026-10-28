//! Version-two canonical bindings and immutable output ownership. Current path
//! membership belongs to command/native preflight, never the saved-source reader.
use super::*;
use crate::{
    constraint_sketch::{
        Constraint, ConstraintKind, Curve, CurveKind, Dimension, Point, Sketch, source::Source,
    },
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    project_file::ContentHash,
    references::ReferenceReport,
    sketch::Frame,
};
use std::collections::BTreeSet;
pub const EXTERNAL_POSE_ROLE: &str = "sketch-external/pose/";
pub fn is_occurrence_reference(document: &Document, id: EntityId) -> bool {
    matches!(document.get(id).map(|e|&e.data),Some(EntityData::Extension{definition,..})if definition.type_id==TYPE_ID&&definition.schema_version==OCCURRENCE_SCHEMA)
}
fn layout(request: &OccurrenceCaptureRequest) -> Result<(), DocumentError> {
    request.query.validate()?;
    request.selector.validate()?;
    let o = &request.output;
    let n = o.points.len();
    let family = request.selector.family;
    let expected = match family {
        ExternalGeometryFamily::Point | ExternalGeometryFamily::Circle => n == 1,
        ExternalGeometryFamily::Line => n == 2,
        ExternalGeometryFamily::Arc => n == 3,
        ExternalGeometryFamily::BSpline => (2..=256).contains(&n),
    };
    let all = o.all();
    if !expected
        || o.curve.is_some() == (family == ExternalGeometryFamily::Point)
        || o.locks.len()
            != if family == ExternalGeometryFamily::Circle {
                2
            } else {
                n
            }
        || all.iter().any(|id| id.is_nil())
        || all.iter().collect::<BTreeSet<_>>().len() != all.len()
    {
        return Err(source::invalid(
            "Occurrence external output identities do not match their named family",
        ));
    }
    Ok(())
}
pub fn occurrence_inputs(request: &OccurrenceCaptureRequest) -> Result<Vec<Link>, DocumentError> {
    layout(request)?;
    let q = &request.query;
    let mut link = Link::new(q.source.source, "source");
    link.geometry = Some(q.intent.clone());
    let mut links = vec![
        link,
        Link::new(
            q.source.component,
            format!("{EXTERNAL_POSE_ROLE}source/component"),
        ),
    ];
    for (label, path) in std::iter::once(("source", &q.source.path))
        .chain(q.sketch_context.as_ref().map(|c| ("context", &c.path)))
    {
        links.push(Link::new(
            path.root,
            format!("{EXTERNAL_POSE_ROLE}{label}/root"),
        ));
        links.extend(
            path.members
                .iter()
                .enumerate()
                .map(|(i, id)| Link::new(*id, format!("{EXTERNAL_POSE_ROLE}{label}/member/{i}"))),
        );
    }
    if let Some(c) = &q.sketch_context {
        links.push(Link::new(
            c.component,
            format!("{EXTERNAL_POSE_ROLE}context/component"),
        ));
    }
    Ok(links)
}
pub fn read_occurrence(
    document: &Document,
    id: EntityId,
) -> Result<OccurrenceCaptureRequest, DocumentError> {
    let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
    let EntityData::Extension { definition, inputs } = &entity.data else {
        return Err(source::invalid(
            "Occurrence reference source is not an extension",
        ));
    };
    if definition.type_id != TYPE_ID || definition.schema_version != OCCURRENCE_SCHEMA {
        return Err(source::invalid(
            "Occurrence reference provider/version is unavailable",
        ));
    }
    let request: OccurrenceCaptureRequest =
        serde_json::from_value(definition.data.clone()).map_err(source::invalid)?;
    layout(&request)?;
    if request.query.reference != id || *inputs != occurrence_inputs(&request)? {
        return Err(source::invalid(
            "Occurrence reference payload and sole canonical links disagree",
        ));
    }
    Ok(request)
}
pub fn occurrence_bindings(
    document: &Document,
    sketch: EntityId,
) -> Result<Vec<OccurrenceCaptureRequest>, DocumentError> {
    let Some(EntityData::Sketch { inputs, .. }) = document.get(sketch).map(|e| &e.data) else {
        return Err(source::invalid("Choose a sketch"));
    };
    inputs
        .iter()
        .filter(|link| link.role == ROLE && is_occurrence_reference(document, link.target))
        .map(|link| read_occurrence(document, link.target))
        .collect()
}
fn definition(request: &OccurrenceCaptureRequest) -> Result<Definition, DocumentError> {
    layout(request)?;
    Ok(Definition {
        type_id: TYPE_ID.into(),
        schema_version: OCCURRENCE_SCHEMA,
        data: serde_json::to_value(request).map_err(source::invalid)?,
    })
}
pub(super) fn matching(request: &OccurrenceCaptureRequest, source: &Source) -> bool {
    let o = &request.output;
    if o.points
        .iter()
        .any(|id| !source.sketch.points.iter().any(|p| p.id == *id))
    {
        return false;
    }
    let c = o
        .curve
        .and_then(|id| source.sketch.curves.iter().find(|c| c.id == id));
    if c.is_some_and(|c| c.construction != (request.behavior == Behavior::ReferenceOnly)) {
        return false;
    }
    match (request.selector.family, c.map(|c| &c.kind)) {
        (ExternalGeometryFamily::Point, None) => true,
        (ExternalGeometryFamily::Line, Some(CurveKind::Line { start, end })) => {
            [*start, *end] == o.points[..]
        }
        (ExternalGeometryFamily::Circle, Some(CurveKind::Circle { center, .. })) => {
            *center == o.points[0]
        }
        (
            ExternalGeometryFamily::Arc,
            Some(CurveKind::Arc {
                center, start, end, ..
            }),
        ) => [*center, *start, *end] == o.points[..],
        (ExternalGeometryFamily::BSpline, Some(CurveKind::BSpline { poles, .. })) => {
            *poles == o.points
        }
        _ => false,
    }
}
pub(super) fn install_occurrence(
    sketch: &mut Sketch,
    request: &OccurrenceCaptureRequest,
    geometry: &ExternalGeometry,
    append: bool,
) -> Result<(), DocumentError> {
    request.output.validate_for(geometry)?;
    if request.selector.family != geometry.family() {
        return Err(source::invalid(
            "Native external family changed; explicitly repair",
        ));
    }
    for (id, position_mm) in request.output.points.iter().copied().zip(geometry.points()) {
        if append {
            sketch.points.push(Point { id, position_mm });
        } else {
            sketch
                .points
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| source::invalid("Protected occurrence point is missing"))?
                .position_mm = position_mm;
        }
    }
    let o = &request.output;
    let kind = match geometry {
        ExternalGeometry::Point { .. } => None,
        ExternalGeometry::Line { .. } => Some(CurveKind::Line {
            start: o.points[0],
            end: o.points[1],
        }),
        ExternalGeometry::Circle { radius_mm, .. } => Some(CurveKind::Circle {
            center: o.points[0],
            radius_mm: *radius_mm,
        }),
        ExternalGeometry::Arc { clockwise, .. } => Some(CurveKind::Arc {
            center: o.points[0],
            start: o.points[1],
            end: o.points[2],
            clockwise: *clockwise,
        }),
        ExternalGeometry::BSpline { basis, .. } => Some(CurveKind::BSpline {
            poles: o.points.clone(),
            basis: basis.clone(),
        }),
    };
    if let (Some(id), Some(kind)) = (o.curve, kind) {
        let curve = Curve {
            id,
            construction: request.behavior == Behavior::ReferenceOnly,
            kind,
        };
        if append {
            sketch.curves.push(curve);
        } else {
            *sketch
                .curves
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| source::invalid("Protected occurrence curve is missing"))? = curve;
        }
    }
    Ok(())
}
fn locks(request: &OccurrenceCaptureRequest, geometry: &ExternalGeometry) -> Vec<Constraint> {
    let mut result: Vec<_> = request
        .output
        .points
        .iter()
        .zip(&request.output.locks)
        .map(|(point, id)| Constraint {
            id: *id,
            reference: false,
            kind: ConstraintKind::Fix { point: *point },
        })
        .collect();
    if let ExternalGeometry::Circle { radius_mm, .. } = geometry {
        result.push(Constraint {
            id: request.output.locks[1],
            reference: false,
            kind: ConstraintKind::Radius {
                curve: request.output.curve.expect("validated circle output"),
                dimension: Dimension::Literal {
                    value: *radius_mm,
                    unit: "mm".into(),
                },
            },
        });
    }
    result
}
pub fn frame_hash(frame: Frame) -> Result<ContentHash, DocumentError> {
    serde_json::to_vec(&(frame.origin, frame.u, frame.v, frame.n))
        .map(|v| ContentHash::of(&v))
        .map_err(source::invalid)
}
pub(super) fn apply_occurrences(
    document: &Document,
    revision: u64,
    sketch: EntityId,
    frame: Frame,
    report: &ReferenceReport,
    solver: &mut Sketch,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut ids = BTreeSet::new();
    for request in occurrence_bindings(document, sketch)? {
        // Deferred structural links stay repairable in stored source, but a
        // reused receipt must still satisfy current occurrence/Part ownership.
        validate_occurrence_query(document, &request.query)?;
        let id = request.query.reference;
        if let Some(error) = report.external_errors.get(&id) {
            return Err(source::invalid(format!(
                "Occurrence reference {id}: {error}"
            )));
        }
        let r = report.external_results.get(&id).ok_or_else(|| {
            source::invalid("Occurrence reference requires an exact current native result receipt")
        })?;
        if r.document != document.id()
            || r.revision != revision
            || r.query_hash != request.query.hash()?
            || r.selector != request.selector
            || r.dependency_hash != crate::references::dependency_hash(document, id)?
            || r.plane_hash != frame_hash(frame)?
        {
            return Err(source::invalid(
                "Occurrence external geometry receipt belongs to stale or foreign source, plane, pose or selection",
            ));
        }
        let projection = external_pose_projection(document, id)?
            .ok_or_else(|| source::invalid("Occurrence pose projection unavailable"))?;
        if r.pose_hash != projection.fingerprint {
            return Err(source::invalid(
                "Occurrence placement changed after its native result receipt",
            ));
        }
        install_occurrence(solver, &request, &r.geometry, false)?;
        for lock in locks(&request, &r.geometry) {
            ids.insert(lock.id);
            solver.constraints.push(lock);
        }
    }
    Ok(ids)
}
impl OccurrenceCaptureRequest {
    pub fn repair(
        catalog: &ExternalCatalog,
        selector: &ResultSelector,
        behavior: Behavior,
        previous: &Self,
    ) -> Result<Self, DocumentError> {
        let mut next = Self::new(catalog, selector, behavior)?;
        if next.query.reference != previous.query.reference
            || next.query.sketch != previous.query.sketch
        {
            return Err(source::invalid(
                "Repair must retain reference and owning sketch identity",
            ));
        }
        let choice = select_external_choice(catalog, selector)?;
        if previous.selector.family != next.selector.family {
            return Err(source::invalid(
                "Family-changing repair requires explicit dependent removal and recapture",
            ));
        }
        previous.output.validate_for(&choice.geometry)?;
        next.output = previous.output.clone();
        Ok(next)
    }
}
pub fn occurrence_capture_edits(
    document: &Document,
    revision: u64,
    request: &OccurrenceCaptureRequest,
    catalog: &ExternalCatalog,
    repair: bool,
) -> Result<Vec<Mutation>, DocumentError> {
    layout(request)?;
    validate_occurrence_query(document, &request.query)?;
    catalog.validate()?;
    if catalog.document != document.id()
        || catalog.revision != revision
        || catalog.source_hash != ContentHash::document(document).map_err(source::invalid)?
        || catalog.query != request.query
    {
        return Err(source::invalid(
            "Native external catalogue belongs to stale or foreign source",
        ));
    }
    let choice = select_external_choice(catalog, &request.selector)?;
    if repair {
        let previous = read_occurrence(document, request.query.reference)?;
        if previous.query.sketch != request.query.sketch
            || previous.output != request.output
            || previous.selector.family != request.selector.family
        {
            return Err(source::invalid(
                "Repair must preserve the owning sketch and compatible output identities",
            ));
        }
    } else if document.get(request.query.reference).is_some() {
        return Err(source::invalid(
            "New external reference identity already exists",
        ));
    }
    let mut entity = document
        .get(request.query.sketch)
        .ok_or(DocumentError::Missing(request.query.sketch))?
        .clone();
    let EntityData::Sketch {
        definition: old,
        inputs,
        ..
    } = &mut entity.data
    else {
        return Err(source::invalid("Choose a sketch"));
    };
    let mut source = Source::from_definition(old)?;
    install_occurrence(&mut source.sketch, request, &choice.geometry, !repair)?;
    source.validate()?;
    *old = source.definition()?;
    old.schema_version = 4;
    if !repair {
        inputs.push(link(request.query.reference));
    }
    let mut external = Entity::new(
        "Occurrence sketch reference",
        EntityData::Extension {
            definition: definition(request)?,
            inputs: occurrence_inputs(request)?,
        },
    );
    external.id = request.query.reference;
    Ok(vec![
        if repair {
            Mutation::Replace(external)
        } else {
            Mutation::Insert(external)
        },
        Mutation::Replace(entity),
    ])
}
pub fn remove_occurrence_edits(
    document: &Document,
    sketch: EntityId,
    reference: EntityId,
) -> Result<Vec<Mutation>, DocumentError> {
    let request = read_occurrence(document, reference)?;
    if request.query.sketch != sketch {
        return Err(source::invalid("Reference is not owned by this sketch"));
    }
    let mut entity = document
        .get(sketch)
        .ok_or(DocumentError::Missing(sketch))?
        .clone();
    let EntityData::Sketch {
        definition, inputs, ..
    } = &mut entity.data
    else {
        return Err(source::invalid("Choose a sketch"));
    };
    let mut source = Source::from_definition(definition)?;
    let ids: BTreeSet<_> = request
        .output
        .points
        .iter()
        .copied()
        .chain(request.output.curve)
        .collect();
    for curve in &source.sketch.curves {
        if Some(curve.id) != request.output.curve
            && curve_points(&curve.kind).iter().any(|id| ids.contains(id))
        {
            return Err(source::invalid(
                "Other sketch geometry uses these external points; explicitly remove its dependents first",
            ));
        }
    }
    if source
        .sketch
        .constraints
        .iter()
        .any(|c| c.kind.references().iter().any(|id| ids.contains(id)))
    {
        return Err(source::invalid(
            "Constraints use this occurrence reference; remove the chosen dependent constraints explicitly first",
        ));
    }
    source.sketch.points.retain(|p| !ids.contains(&p.id));
    source
        .sketch
        .curves
        .retain(|c| Some(c.id) != request.output.curve);
    source.validate()?;
    *definition = source.definition()?;
    definition.schema_version = 4;
    inputs.retain(|l| !(l.role == ROLE && l.target == reference));
    Ok(vec![Mutation::Replace(entity), Mutation::Remove(reference)])
}
fn curve_points(kind: &CurveKind) -> Vec<EntityId> {
    match kind {
        CurveKind::Line { start, end } => vec![*start, *end],
        CurveKind::Circle { center, .. } => vec![*center],
        CurveKind::Arc {
            center, start, end, ..
        } => vec![*center, *start, *end],
        CurveKind::BSpline { poles, .. } => poles.clone(),
    }
}
