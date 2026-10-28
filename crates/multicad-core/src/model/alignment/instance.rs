use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, Mutation},
    model::{
        MAX_TRANSFORM_TARGETS, PlacementBase, PlacementPrepared, RigidPose,
        common::{error, revision},
    },
};
pub const INSTANCE_ALIGNMENT_TYPE: &str = "model.instance_alignment";
pub const INSTANCE_ALIGNMENT_SCHEMA: u32 = 1;
pub const INSTANCE_ALIGNMENT_CAPABILITY: &str = "model.instance_alignment.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceAlignmentDefinition {
    pub owner: EntityId,
    pub base: PlacementBase,
    pub parameters: AlignmentParameters,
}
impl InstanceAlignmentDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.owner.is_nil() {
            return Err(invalid("Alignment driver requires an owning instance"));
        }
        match self.base {
            PlacementBase::Literal { pose } => pose.validate()?,
            PlacementBase::Previous { driver } if driver.is_nil() || driver == self.owner => {
                return Err(invalid("Alignment previous driver identity is invalid"));
            }
            _ => {}
        }
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: INSTANCE_ALIGNMENT_TYPE.into(),
            schema_version: INSTANCE_ALIGNMENT_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != INSTANCE_ALIGNMENT_TYPE
            || definition.schema_version != INSTANCE_ALIGNMENT_SCHEMA
        {
            return Err(invalid("Unsupported instance-alignment provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self, references: &[AlignmentInput]) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut all = links(&self.parameters, references)?;
        if let PlacementBase::Previous { driver } = self.base {
            all.push(Link::new(driver, "previous_pose"));
        }
        Ok(all)
    }
}
pub fn read_instance_alignment(
    document: &Document,
    driver: EntityId,
) -> Result<(InstanceAlignmentDefinition, Vec<AlignmentInput>), DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(driver).map(|e| &e.data)
    else {
        return Err(invalid("Instance alignment driver is missing"));
    };
    let source = InstanceAlignmentDefinition::from_definition(definition)?;
    let references = captured(inputs);
    let expected = source.inputs(&references)?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Instance alignment and canonical dependency links disagree",
        ));
    }
    if let Some(owner) = document.get(source.owner) {
        let EntityData::Instance { component, .. } = owner.data else {
            return Err(invalid("Instance alignment owner has the wrong kind"));
        };
        if let (Some(EntityData::Part { features, .. }), AlignmentAnchor::Geometry { reference }) = (
            document.get(component).map(|e| &e.data),
            &source.parameters.source,
        ) && references.iter().any(|input| {
            input.intent.id == *reference
                && input.source != component
                && !features.contains(&input.source)
        }) {
            return Err(invalid(
                "An instance's moving source anchor must refer to its local component producer, never its own world occurrence",
            ));
        }
    }
    if let PlacementBase::Previous { driver: previous } = source.base
        && let Some(entity) = document.get(previous)
        && super::super::placement::header(&entity.data)?.0 != source.owner
    {
        return Err(invalid(
            "Alignment driver cannot inherit another instance's placement chain",
        ));
    }
    for link in inputs {
        if let Some(entity) = document.get(link.target) {
            let valid = if link.geometry.is_some() {
                matches!(
                    entity.data,
                    EntityData::Feature { .. }
                        | EntityData::Instance { .. }
                        | EntityData::Part { .. }
                )
            } else if link.role == "previous_pose" {
                matches!(entity.data, EntityData::Extension { .. })
            } else if link.role.ends_with("/plane") {
                matches!(entity.data, EntityData::Plane { .. })
            } else {
                matches!(entity.data, EntityData::Parameter { .. })
            };
            if !valid {
                return Err(invalid(
                    "Instance alignment dependency has the wrong entity kind",
                ));
            }
        }
    }
    Ok((source, references))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceAlignmentRequest {
    pub instance: EntityId,
    pub driver: EntityId,
    pub parameters: AlignmentParameters,
    pub references: Vec<AlignmentInput>,
}
impl InstanceAlignmentRequest {
    pub fn new(
        instance: EntityId,
        parameters: AlignmentParameters,
        references: Vec<AlignmentInput>,
    ) -> Self {
        Self {
            instance,
            driver: EntityId::new(),
            parameters,
            references,
        }
    }
}
pub fn prepare_instance_alignments(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[InstanceAlignmentRequest],
) -> Result<PlacementPrepared, CommandError> {
    revision(session, expected_revision)?;
    if requests.is_empty() || requests.len() > MAX_TRANSFORM_TARGETS {
        return Err(error("Choose 1–128 explicit instance alignment targets"));
    }
    let document = session.document();
    let mut ids = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut edits = vec![];
    let mut instances = vec![];
    for request in requests {
        if request.driver.is_nil()
            || request.driver == document.id()
            || document.get(request.driver).is_some()
            || !ids.insert(request.driver)
            || !targets.insert(request.instance)
        {
            return Err(error(
                "Instance alignments require distinct targets and fresh non-nil driver identities",
            ));
        }
        request.parameters.numbers(session.parameters())?;
        let mut owner = document
            .get(request.instance)
            .ok_or_else(|| error("Instance alignment target is missing"))?
            .clone();
        let EntityData::Instance {
            component,
            placement,
            placement_source,
        } = &mut owner.data
        else {
            return Err(error("Choose an existing Part instance"));
        };
        if !matches!(
            document.get(*component).map(|e| &e.data),
            Some(EntityData::Part { .. })
        ) {
            return Err(error(
                "Subassembly alignment requires the assembly evaluator",
            ));
        }
        let base = match *placement_source {
            Some(driver) => {
                super::super::placement::check_next_driver(document, driver, request.instance)?;
                PlacementBase::Previous { driver }
            }
            None => PlacementBase::Literal {
                pose: RigidPose::from_placement(*placement)?,
            },
        };
        let source = InstanceAlignmentDefinition {
            owner: request.instance,
            base,
            parameters: request.parameters.clone(),
        };
        let mut driver = Entity::new(
            "Referenced instance alignment",
            EntityData::Extension {
                definition: source.definition()?,
                inputs: source.inputs(&request.references)?,
            },
        );
        driver.id = request.driver;
        *placement_source = Some(request.driver);
        edits.extend([Mutation::Insert(driver), Mutation::Replace(owner)]);
        instances.push(request.instance);
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Align referenced instances".into(),
        edits,
    })?;
    Ok(PlacementPrepared {
        prepared,
        instances,
    })
}
pub fn prepare_instance_alignment_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    driver: EntityId,
    parameters: &AlignmentParameters,
    references: &[AlignmentInput],
) -> Result<PlacementPrepared, CommandError> {
    revision(session, expected_revision)?;
    let (mut source, _) = read_instance_alignment(session.document(), driver)?;
    parameters.numbers(session.parameters())?;
    source.parameters = parameters.clone();
    let mut entity = session
        .document()
        .get(driver)
        .expect("read alignment driver")
        .clone();
    entity.data = EntityData::Extension {
        definition: source.definition()?,
        inputs: source.inputs(references)?,
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit referenced instance alignment".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(PlacementPrepared {
        prepared,
        instances: vec![source.owner],
    })
}
