use super::{CreatedBody, ModelPrepared, source::invalid};
use crate::{
    commands::{Actor, CommandError, ErrorCode, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    parameters::{EvaluatedParameters, Unit},
};
use std::collections::BTreeSet;

/// Preserve exact cardinal directions instead of introducing libm residuals.
/// Only exact equality after turn reduction is special; nearby authored angles
/// retain the ordinary trigonometric result from their original argument.
pub(super) fn cardinal_sin_cos(angle: f64) -> (f64, f64) {
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    let reduced = angle.rem_euclid(TAU);
    if reduced == 0. {
        (0., 1.)
    } else if reduced == FRAC_PI_2 {
        (1., 0.)
    } else if reduced == PI {
        (0., -1.)
    } else if reduced == 3. * FRAC_PI_2 {
        (-1., 0.)
    } else {
        angle.sin_cos()
    }
}

pub(super) fn parameter_links(dimensions: Vec<&Dimension>) -> Vec<Link> {
    dimensions
        .into_iter()
        .filter_map(Dimension::expression)
        .flat_map(|e| e.bindings.iter().map(|b| b.target))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|id| Link::new(id, format!("parameter/{id}")))
        .collect()
}
pub(super) fn validate_feature(
    document: &Document,
    id: EntityId,
    body: EntityId,
    inputs: &[Link],
    expected: &[Link],
) -> Result<(), DocumentError> {
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Model feature source and canonical input links disagree",
        ));
    }
    for link in inputs {
        if let Some(entity) = document.get(link.target) {
            let correct = if link.role == "profile" {
                matches!(entity.data, EntityData::Sketch { .. })
            } else {
                matches!(entity.data, EntityData::Parameter { .. })
            };
            if !correct {
                return Err(invalid("Model input has the wrong entity kind"));
            }
        }
    }
    if let Some(entity) = document.get(body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("Model output must identify a Part"));
        };
        if !features.contains(&id) {
            return Err(invalid(
                "Model output Part must contain its producing feature",
            ));
        }
    }
    Ok(())
}
pub(super) fn error(message: impl Into<String>) -> CommandError {
    CommandError {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
        involved: vec![],
        parameter: None,
    }
}
pub(super) fn revision(session: &Session, expected: u64) -> Result<(), CommandError> {
    if session.revision() != expected {
        return Err(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Model preparation source revision changed".into(),
            involved: vec![],
            parameter: None,
        });
    }
    Ok(())
}
pub(super) struct BodyDraft {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub definition: Definition,
    pub inputs: Vec<Link>,
}
pub(super) fn prepare_bodies(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    label: &str,
    bodies: Vec<BodyDraft>,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut ids = BTreeSet::new();
    let mut edits = vec![];
    let mut outputs = vec![];
    for draft in bodies {
        for id in [draft.feature, draft.body] {
            if id.is_nil()
                || id == session.document().id()
                || session.document().get(id).is_some()
                || !ids.insert(id)
            {
                return Err(error(
                    "Model output identities must be new, distinct and non-nil",
                ));
            }
        }
        if draft.name.trim().is_empty() || draft.name.len() > 1000 {
            return Err(error("Body name must contain 1–1000 bytes"));
        }
        let mut feature = Entity::new(
            format!("{} feature", draft.name),
            EntityData::Feature {
                definition: draft.definition,
                inputs: draft.inputs,
                suppressed: false,
            },
        );
        feature.id = draft.feature;
        let mut body = Entity::new(
            draft.name,
            EntityData::Part {
                features: vec![draft.feature],
                material: None,
                tags: BTreeSet::new(),
            },
        );
        body.id = draft.body;
        edits.extend([Mutation::Insert(feature), Mutation::Insert(body)]);
        outputs.push(CreatedBody {
            feature: draft.feature,
            body: draft.body,
        });
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
pub(super) fn prepare_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    label: &str,
    output: CreatedBody,
    definition: Definition,
    inputs: Vec<Link>,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut entity = session
        .document()
        .get(output.feature)
        .ok_or_else(|| error("Choose an existing model feature"))?
        .clone();
    let EntityData::Feature {
        definition: current,
        inputs: links,
        ..
    } = &mut entity.data
    else {
        return Err(error("Choose an existing model feature"));
    };
    *current = definition;
    *links = inputs;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![output],
    })
}

pub(super) fn number(
    d: &Dimension,
    unit: Unit,
    parameters: &EvaluatedParameters,
) -> Result<f64, DocumentError> {
    let value = d.value_in(unit, Some(parameters)).map_err(invalid)?;
    if !value.is_finite() || value.abs() > 1e9 {
        return Err(invalid(
            "Resolved model dimension is nonfinite or exceeds limits",
        ));
    }
    Ok(value)
}
