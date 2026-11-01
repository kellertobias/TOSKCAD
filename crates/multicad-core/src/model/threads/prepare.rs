use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub target: Option<BodyInput>,
    pub mode: ThreadMode,
    pub parameters: ThreadParameters,
}
impl ThreadRequest {
    pub fn new(name: impl Into<String>, parameters: ThreadParameters) -> Self {
        Self {
            feature: EntityId::new(),
            body: EntityId::new(),
            name: name.into(),
            target: None,
            mode: ThreadMode::NewExternal,
            parameters,
        }
    }
    pub fn receiving(
        name: impl Into<String>,
        mode: ThreadMode,
        target: BodyInput,
        parameters: ThreadParameters,
    ) -> Result<Self, DocumentError> {
        if mode == ThreadMode::NewExternal {
            return Err(invalid("Choose an internal or external receiving cut"));
        }
        Ok(Self {
            feature: EntityId::new(),
            body: target.body,
            name: name.into(),
            target: Some(target),
            mode,
            parameters,
        })
    }
}
pub fn prepare_thread(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ThreadRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    request
        .parameters
        .resolve(session.parameters(), request.mode)?;
    let source = ThreadDefinition {
        output: BodyOutput { body: request.body },
        target: request.target.clone(),
        mode: request.mode,
        parameters: request.parameters.clone(),
    };
    let definition = source.definition()?;
    let inputs = source.inputs()?;
    let Some(target) = &source.target else {
        return prepare_bodies(
            session,
            expected_revision,
            actor,
            "Create threaded shaft",
            vec![BodyDraft {
                feature: request.feature,
                body: request.body,
                name: request.name.clone(),
                definition,
                inputs,
            }],
        );
    };
    super::super::boolean::available(session.document(), target, true)?;
    if request.feature.is_nil()
        || request.feature == session.document().id()
        || session.document().get(request.feature).is_some()
    {
        return Err(error("Thread producer identity must be new and non-nil"));
    }
    if request.name.trim().is_empty() || request.name.len() > 1000 {
        return Err(error("Thread name must contain 1–1000 bytes"));
    }
    let mut feature = Entity::new(
        &request.name,
        EntityData::Feature {
            definition,
            inputs,
            suppressed: false,
        },
    );
    feature.id = request.feature;
    let mut part = session
        .document()
        .get(target.body)
        .expect("validated thread target")
        .clone();
    let EntityData::Part { features, .. } = &mut part.data else {
        unreachable!()
    };
    features.push(request.feature);
    let mut edits = vec![Mutation::Insert(feature), Mutation::Replace(part)];
    let index =
        crate::hierarchy::Index::new(session.document()).map_err(|e| error(e.to_string()))?;
    if let Some(parent) = index
        .parent(target.body)
        .map_err(|e| error(e.to_string()))?
    {
        let mut hierarchy = session.document().hierarchy().clone();
        hierarchy
            .members
            .entry(parent)
            .or_default()
            .push(request.feature);
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Cut receiving thread".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature: request.feature,
            body: request.body,
        }],
    })
}
/// Historical edits keep the original target predecessor and Part identity.
/// Changing a later feature never replaces the target with its current head.
pub fn prepare_thread_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    parameters: &ThreadParameters,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    super::super::extrude_group::reject_individual_edit(session.document(), feature)?;
    let mut source = read_thread(session.document(), feature)?;
    parameters.resolve(session.parameters(), source.mode)?;
    source.parameters = parameters.clone();
    prepare_edit(
        session,
        expected_revision,
        actor,
        "Edit thread",
        CreatedBody {
            feature,
            body: source.output.body,
        },
        source.definition()?,
        source.inputs()?,
    )
}
