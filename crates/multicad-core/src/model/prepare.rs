use super::*;
use crate::{
    commands::{Actor, CommandError, ErrorCode, Prepared, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    sketch_regions::RegionIntent,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Allocate once when a tool begins; repeated previews reuse these identities.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeOutput {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub region: RegionIntent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeRequest {
    pub sketch: EntityId,
    pub outputs: Vec<ExtrudeOutput>,
    pub parameters: ExtrudeParameters,
    pub operation: Operation,
    pub targets: Vec<BodyInput>,
    pub keep_tools: bool,
    pub faces: Vec<FaceInput>,
}
impl ExtrudeRequest {
    pub fn new(
        sketch: EntityId,
        regions: Vec<RegionIntent>,
        parameters: ExtrudeParameters,
    ) -> Self {
        Self {
            sketch,
            outputs: regions
                .into_iter()
                .enumerate()
                .map(|(index, region)| ExtrudeOutput {
                    feature: EntityId::new(),
                    body: EntityId::new(),
                    name: format!("Extruded part {}", index + 1),
                    region,
                })
                .collect(),
            parameters,
            operation: Operation::NewBody,
            targets: vec![],
            keep_tools: false,
            faces: vec![],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedBody {
    pub feature: EntityId,
    pub body: EntityId,
}
#[derive(Clone, Debug)]
pub struct ModelPrepared {
    /// Source-only: a native service must additionally evaluate exact current
    /// sketch profiles/solids before offering this transaction for Apply.
    pub prepared: Prepared,
    pub outputs: Vec<CreatedBody>,
}
fn error(message: impl Into<String>, involved: Vec<EntityId>) -> CommandError {
    CommandError {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
        involved,
        parameter: None,
    }
}
fn revision(session: &Session, expected: u64) -> Result<(), CommandError> {
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
fn executable(source: &ExtrudeDefinition, session: &Session) -> Result<(), CommandError> {
    source.validate()?;
    if source.operation != Operation::NewBody {
        return Err(error(
            "Join/Cut/Intersect/Split extrusion is not supported by this model preparation yet",
            source.targets.iter().map(|t| t.body).collect(),
        ));
    }
    source.parameters.resolve_controls(session.parameters())?;
    Ok(())
}
fn termination_inputs(
    document: &Document,
    source: &ExtrudeDefinition,
    inputs: &[crate::document::Link],
) -> Result<(), CommandError> {
    for end in source.parameters.terminations() {
        match end {
            Termination::Distance { .. } => {}
            Termination::ToPlane { plane, .. } => {
                if !matches!(
                    document.get(*plane).map(|e| &e.data),
                    Some(EntityData::Plane { .. })
                ) {
                    return Err(error("Choose an existing termination Plane", vec![*plane]));
                }
            }
            Termination::ToFace { reference, .. } => {
                let link = inputs
                    .iter()
                    .find(|link| {
                        link.geometry
                            .as_ref()
                            .is_some_and(|intent| intent.id == *reference)
                    })
                    .ok_or_else(|| {
                        error("A termination face input is missing", vec![*reference])
                    })?;
                if !matches!(
                    document.get(link.target).map(|e| &e.data),
                    Some(EntityData::Feature {
                        suppressed: false,
                        ..
                    })
                ) {
                    return Err(error(
                        "Choose an existing, unsuppressed termination face producer",
                        vec![link.target],
                    ));
                }
            }
        }
    }
    Ok(())
}
fn authored_curves(
    document: &Document,
    sketch: EntityId,
) -> Result<BTreeSet<EntityId>, CommandError> {
    let Some(EntityData::Sketch { definition, .. }) = document.get(sketch).map(|e| &e.data) else {
        return Err(error("Choose an existing constrained sketch", vec![sketch]));
    };
    let source = Source::from_definition(definition)?;
    Ok(source
        .sketch
        .curves
        .into_iter()
        .filter(|curve| !curve.construction)
        .map(|curve| curve.id)
        .collect())
}
/// Prepare canonical new-body source without solving sketches or running a
/// kernel. Structural checks never claim that a profile is currently closed.
pub fn prepare_extrude(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ExtrudeRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.outputs.is_empty() || request.outputs.len() > MAX_EXTRUDE_OUTPUTS {
        return Err(error(
            format!("Select between 1 and {MAX_EXTRUDE_OUTPUTS} distinct profile areas"),
            vec![request.sketch],
        ));
    }
    let curves = authored_curves(session.document(), request.sketch)?;
    let mut ids = BTreeSet::new();
    let mut regions = Vec::new();
    let mut edits = vec![];
    let mut outputs = vec![];
    for output in &request.outputs {
        for id in [output.feature, output.body] {
            if id.is_nil()
                || id == session.document().id()
                || session.document().get(id).is_some()
                || !ids.insert(id)
            {
                return Err(error(
                    "Extrusion output identities must be new, distinct and non-nil",
                    vec![id],
                ));
            }
        }
        if output.name.trim().is_empty() || output.name.len() > 1000 {
            return Err(error(
                "Body name must contain 1–1000 bytes",
                vec![output.body],
            ));
        }
        if regions.contains(&output.region) {
            return Err(error("Select each profile area once", vec![request.sketch]));
        }
        regions.push(output.region.clone());
        if std::iter::once(&output.region.outer)
            .chain(&output.region.holes)
            .flat_map(|b| &b.curves)
            .any(|id| !curves.contains(id))
        {
            return Err(error(
                "Selected profile contains missing or construction curve identities",
                vec![request.sketch],
            ));
        }
        let source = ExtrudeDefinition {
            profile: ProfileSource {
                sketch: request.sketch,
                region: output.region.clone(),
            },
            output: BodyOutput { body: output.body },
            parameters: request.parameters.clone(),
            operation: request.operation,
            targets: request.targets.clone(),
            keep_tools: request.keep_tools,
        };
        executable(&source, session)?;
        let inputs = source.inputs(&request.faces)?;
        termination_inputs(session.document(), &source, &inputs)?;
        let mut feature = Entity::new(
            format!("{} extrusion", output.name),
            EntityData::Feature {
                definition: source.definition()?,
                inputs,
                suppressed: false,
            },
        );
        feature.id = output.feature;
        let mut part = Entity::new(
            &output.name,
            EntityData::Part {
                features: vec![output.feature],
                material: None,
                tags: BTreeSet::new(),
            },
        );
        part.id = output.body;
        edits.extend([Mutation::Insert(feature), Mutation::Insert(part)]);
        outputs.push(CreatedBody {
            feature: output.feature,
            body: output.body,
        });
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: "Extrude new bodies".into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
/// Edit one existing producer while preserving its Part, feature IDs, profile,
/// formula bindings and all unrelated canonical entities.
pub fn prepare_extrude_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &ExtrudeParameters,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_extrude(session.document(), feature)?;
    source.parameters = parameters.clone();
    let wanted = source.face_references();
    let EntityData::Feature { inputs, .. } =
        &session.document().get(feature).expect("read source").data
    else {
        unreachable!()
    };
    let faces: Vec<_> = inputs
        .iter()
        .filter_map(|link| {
            link.geometry
                .as_ref()
                .filter(|intent| wanted.contains(&intent.id))
                .map(|intent| FaceInput {
                    source: link.target,
                    intent: intent.clone(),
                })
        })
        .collect();
    prepare_extrude_edit_with_faces(
        session,
        expected_revision,
        actor,
        feature,
        parameters,
        &faces,
    )
}
/// Explicitly replace termination face inputs. Ordinary edits retain the
/// current link-owned selectors; callers supply fresh captures through this
/// path when changing or repairing the selected target.
pub fn prepare_extrude_edit_with_faces(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &ExtrudeParameters,
    faces: &[FaceInput],
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_extrude(session.document(), feature)?;
    source.parameters = parameters.clone();
    executable(&source, session)?;
    let mut entity = session
        .document()
        .get(feature)
        .expect("read source")
        .clone();
    let EntityData::Feature {
        definition, inputs, ..
    } = &mut entity.data
    else {
        unreachable!()
    };
    *definition = source.definition()?;
    *inputs = source.inputs(faces)?;
    termination_inputs(session.document(), &source, inputs)?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit extrusion".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature,
            body: source.output.body,
        }],
    })
}
