use super::*;
use crate::model::{
    CreatedBody, MAX_TRANSFORM_TARGETS, ModelPrepared,
    boolean::available,
    common::{error, revision},
};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, Mutation},
    hierarchy,
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlignmentRequest {
    pub feature: EntityId,
    pub name: String,
    pub input: BodyInput,
    pub parameters: AlignmentParameters,
    pub references: Vec<AlignmentInput>,
}
impl AlignmentRequest {
    pub fn new(
        input: BodyInput,
        parameters: AlignmentParameters,
        references: Vec<AlignmentInput>,
    ) -> Self {
        Self {
            feature: EntityId::new(),
            name: "Referenced alignment".into(),
            input,
            parameters,
            references,
        }
    }
}
pub fn prepare_alignment(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &AlignmentRequest,
) -> Result<ModelPrepared, CommandError> {
    prepare_alignments(
        session,
        expected_revision,
        actor,
        std::slice::from_ref(request),
    )
}
pub fn prepare_alignments(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[AlignmentRequest],
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if requests.is_empty() || requests.len() > MAX_TRANSFORM_TARGETS {
        return Err(error("Choose 1–128 explicit alignment targets"));
    }
    let document = session.document();
    let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    let mut hierarchy = document.hierarchy().clone();
    let mut ids = BTreeSet::new();
    let mut bodies = BTreeSet::new();
    let mut edits = vec![];
    let mut outputs = vec![];
    for request in requests {
        if request.feature.is_nil()
            || request.feature == document.id()
            || document.get(request.feature).is_some()
            || !ids.insert(request.feature)
            || !bodies.insert(request.input.body)
        {
            return Err(error(
                "Choose distinct Parts and fresh non-nil alignment feature identities",
            ));
        }
        if request.name.trim().is_empty() || request.name.len() > 1000 {
            return Err(error("Alignment name must contain 1–1000 bytes"));
        }
        available(document, &request.input, true)?;
        request.parameters.numbers(session.parameters())?;
        let source = AlignmentDefinition {
            input: request.input.clone(),
            output: BodyOutput {
                body: request.input.body,
            },
            parameters: request.parameters.clone(),
        };
        let mut feature = Entity::new(
            &request.name,
            EntityData::Feature {
                definition: source.definition()?,
                inputs: source.inputs(&request.references)?,
                suppressed: false,
            },
        );
        feature.id = request.feature;
        let mut part = document
            .get(request.input.body)
            .expect("validated Part")
            .clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        features.push(feature.id);
        if let Some(parent) = index.parent(part.id).map_err(|e| error(e.to_string()))? {
            hierarchy
                .members
                .entry(parent)
                .or_default()
                .push(feature.id);
        }
        edits.extend([Mutation::Insert(feature), Mutation::Replace(part)]);
        outputs.push(CreatedBody {
            feature: request.feature,
            body: request.input.body,
        });
    }
    if hierarchy != *document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Align referenced Parts".into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
pub fn prepare_alignment_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &AlignmentParameters,
    references: &[AlignmentInput],
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let (mut source, _) = read_alignment(session.document(), feature)?;
    available(session.document(), &source.input, false)?;
    parameters.numbers(session.parameters())?;
    source.parameters = parameters.clone();
    crate::model::common::prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit referenced alignment",
        CreatedBody {
            body: source.output.body,
            feature,
        },
        source.definition()?,
        source.inputs(references)?,
    )
}
