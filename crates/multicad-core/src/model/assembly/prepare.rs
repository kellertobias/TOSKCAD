use super::*;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation, Placement},
    model::{
        AlignmentInput, InstancePlacementDefinition, NumericTransform, PlacementBase, RigidPose,
        common::{error, revision},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub struct AssemblyPrepared {
    pub prepared: Prepared,
    pub instances: Vec<EntityId>,
    pub assemblies: Vec<EntityId>,
    pub joints: Vec<EntityId>,
    pub pose: Option<EntityId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointRequest {
    pub driver: EntityId,
    pub owner: EntityId,
    pub parameters: JointParameters,
    pub references: Vec<AlignmentInput>,
}
impl JointRequest {
    pub fn new(
        owner: EntityId,
        parameters: JointParameters,
        references: Vec<AlignmentInput>,
    ) -> Self {
        Self {
            driver: EntityId::new(),
            owner,
            parameters,
            references,
        }
    }
    pub fn existing(
        doc: &Document,
        driver: EntityId,
    ) -> Result<Self, crate::document::DocumentError> {
        let (s, references) = read_joint(doc, driver)?;
        Ok(Self {
            driver,
            owner: s.owner,
            parameters: s.parameters,
            references,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyRequest {
    pub assembly: EntityId,
    pub metadata: EntityId,
    pub name: String,
    pub members: Vec<EntityId>,
    pub definition_parts: Vec<EntityId>,
}
impl AssemblyRequest {
    pub fn new(
        name: impl Into<String>,
        members: Vec<EntityId>,
        definition_parts: Vec<EntityId>,
    ) -> Self {
        Self {
            assembly: EntityId::new(),
            metadata: EntityId::new(),
            name: name.into(),
            members,
            definition_parts,
        }
    }
    pub fn existing(
        doc: &Document,
        assembly: EntityId,
    ) -> Result<Self, crate::document::DocumentError> {
        let entity = doc
            .get(assembly)
            .ok_or_else(|| crate::document::DocumentError::Invalid("Assembly missing".into()))?;
        let EntityData::Assembly { instances } = &entity.data else {
            return Err(crate::document::DocumentError::Invalid(
                "Choose an Assembly".into(),
            ));
        };
        let (metadata, source) = doc
            .entities()
            .find_map(|e| {
                read_assembly(doc, e.id)
                    .ok()
                    .filter(|s| s.assembly == assembly)
                    .map(|s| (e.id, s))
            })
            .ok_or_else(|| {
                crate::document::DocumentError::Invalid(
                    "Reusable Assembly metadata is missing".into(),
                )
            })?;
        Ok(Self {
            assembly,
            metadata,
            name: entity.name.clone(),
            members: instances.clone(),
            definition_parts: source.definition_parts,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentPlacementRequest {
    pub component: EntityId,
    pub instance: EntityId,
    pub driver: EntityId,
    pub name: String,
    pub parameters: NumericTransform,
}
impl ComponentPlacementRequest {
    pub fn new(component: EntityId, parameters: NumericTransform) -> Self {
        Self {
            component,
            instance: EntityId::new(),
            driver: EntityId::new(),
            name: "Placed component".into(),
            parameters,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedPoseRequest {
    pub pose: EntityId,
    pub name: String,
    pub values: Vec<JointPoseValue>,
}
impl NamedPoseRequest {
    pub fn new(name: impl Into<String>, values: Vec<JointPoseValue>) -> Self {
        Self {
            pose: EntityId::new(),
            name: name.into(),
            values,
        }
    }
    pub fn existing(
        doc: &Document,
        pose: EntityId,
    ) -> Result<Self, crate::document::DocumentError> {
        Ok(Self {
            pose,
            name: doc
                .get(pose)
                .ok_or_else(|| {
                    crate::document::DocumentError::Invalid("Named pose missing".into())
                })?
                .name
                .clone(),
            values: read_named_pose(doc, pose)?.values,
        })
    }
}
fn fresh(doc: &Document, id: EntityId) -> Result<(), CommandError> {
    if id.is_nil() || id == doc.id() || doc.get(id).is_some() {
        return Err(error(
            "Assembly output identities must be fresh and non-nil",
        ));
    }
    Ok(())
}
fn extension(
    id: EntityId,
    name: &str,
    definition: crate::document::Definition,
    inputs: Vec<crate::document::Link>,
) -> Entity {
    let mut e = Entity::new(name, EntityData::Extension { definition, inputs });
    e.id = id;
    e
}
pub fn prepare_joint(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &JointRequest,
) -> Result<AssemblyPrepared, CommandError> {
    revision(session, expected_revision)?;
    fresh(session.document(), request.driver)?;
    let mut owner = session
        .document()
        .get(request.owner)
        .ok_or_else(|| error("Choose an existing moving Instance"))?
        .clone();
    let EntityData::Instance {
        component,
        placement,
        placement_source,
    } = &mut owner.data
    else {
        return Err(error(
            "A joint controls a placed Instance, preserving its source Part/Assembly",
        ));
    };
    if joint_driver(session.document(), request.owner)?.is_some() {
        return Err(error(
            "This Instance already has an incoming joint; edit that relationship instead",
        ));
    }
    request.parameters.resolve(session.parameters())?;
    let base = match *placement_source {
        Some(driver) => PlacementBase::Previous { driver },
        None => PlacementBase::Literal {
            pose: RigidPose::from_placement(*placement)?,
        },
    };
    let source = JointDefinition {
        owner: request.owner,
        component: *component,
        base,
        parameters: request.parameters.clone(),
    };
    let driver = extension(
        request.driver,
        request.parameters.kind.label(),
        source.definition()?,
        source.inputs(&request.references)?,
    );
    *placement_source = Some(request.driver);
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Create {} relationship", request.parameters.kind.label()),
        edits: vec![Mutation::Insert(driver), Mutation::Replace(owner)],
    })?;
    Ok(AssemblyPrepared {
        prepared,
        instances: vec![request.owner],
        assemblies: vec![],
        joints: vec![request.driver],
        pose: None,
    })
}
pub fn prepare_joint_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &JointRequest,
) -> Result<AssemblyPrepared, CommandError> {
    revision(session, expected_revision)?;
    let (mut source, _) = read_joint(session.document(), request.driver)?;
    if [&source.parameters.moving, &source.parameters.target]
        .iter()
        .any(|a| matches!(a, JointAnchor::GearPairFrame { .. }))
    {
        return Err(error(
            "This relationship is controlled by a gear pair; edit the pair drive or gear recipes",
        ));
    }
    if source.owner != request.owner
        || joint_driver(session.document(), request.owner)? != Some(request.driver)
    {
        return Err(error(
            "Edit the active incoming joint of the same moving Instance",
        ));
    }
    source.parameters = request.parameters.clone();
    source.parameters.resolve(session.parameters())?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit relationship".into(),
        edits: vec![Mutation::Replace(extension(
            request.driver,
            source.parameters.kind.label(),
            source.definition()?,
            source.inputs(&request.references)?,
        ))],
    })?;
    Ok(AssemblyPrepared {
        prepared,
        instances: vec![request.owner],
        assemblies: vec![],
        joints: vec![request.driver],
        pose: None,
    })
}
fn assembly(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &AssemblyRequest,
    edit: bool,
) -> Result<AssemblyPrepared, CommandError> {
    revision(session, expected_revision)?;
    let doc = session.document();
    if edit {
        let old = AssemblyRequest::existing(doc, request.assembly)?;
        if old.metadata != request.metadata {
            return Err(error("Assembly edit must preserve metadata identity"));
        }
    } else {
        fresh(doc, request.assembly)?;
        fresh(doc, request.metadata)?;
        if request.assembly == request.metadata {
            return Err(error("Assembly and metadata need distinct identities"));
        }
    }
    if request.members.is_empty()
        || request.members.len() > 4096
        || request.members.iter().collect::<BTreeSet<_>>().len() != request.members.len()
    {
        return Err(error(
            "Reusable Assembly needs 1..4096 distinct member Instances",
        ));
    }
    for member in &request.members {
        if !matches!(
            doc.get(*member).map(|e| &e.data),
            Some(EntityData::Instance { .. })
        ) {
            return Err(error("Assembly members must be explicit placed Instances"));
        }
        for e in doc.entities() {
            if let EntityData::Assembly { instances } = &e.data
                && e.id != request.assembly
                && instances.contains(member)
            {
                return Err(error(
                    "An Instance is already a member of another definition; place a separate Instance",
                ));
            }
        }
    }
    let source = AssemblyDefinition {
        assembly: request.assembly,
        definition_parts: request.definition_parts.clone(),
    };
    let mut entity = Entity::new(
        &request.name,
        EntityData::Assembly {
            instances: request.members.clone(),
        },
    );
    entity.id = request.assembly;
    let metadata = extension(
        request.metadata,
        "Reusable component definition",
        source.definition()?,
        source.inputs()?,
    );
    let mut state = doc.hierarchy().clone();
    for children in state.members.values_mut() {
        children.retain(|id| !request.members.contains(id));
    }
    state
        .members
        .insert(request.assembly, request.members.clone());
    if !edit {
        state
            .members
            .entry(doc.id())
            .or_default()
            .push(request.assembly);
    }
    let edits = vec![
        if edit {
            Mutation::Replace(entity)
        } else {
            Mutation::Insert(entity)
        },
        if edit {
            Mutation::Replace(metadata)
        } else {
            Mutation::Insert(metadata)
        },
        Mutation::SetHierarchy(state),
    ];
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: if edit {
            "Edit reusable component"
        } else {
            "Create reusable component"
        }
        .into(),
        edits,
    })?;
    Ok(AssemblyPrepared {
        prepared,
        instances: request.members.clone(),
        assemblies: vec![request.assembly],
        joints: vec![],
        pose: None,
    })
}
pub fn prepare_assembly(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &AssemblyRequest,
) -> Result<AssemblyPrepared, CommandError> {
    assembly(session, revision, actor, request, false)
}
pub fn prepare_assembly_edit(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &AssemblyRequest,
) -> Result<AssemblyPrepared, CommandError> {
    assembly(session, revision, actor, request, true)
}
pub fn prepare_place_component(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ComponentPlacementRequest,
) -> Result<AssemblyPrepared, CommandError> {
    revision(session, expected_revision)?;
    let doc = session.document();
    fresh(doc, request.instance)?;
    fresh(doc, request.driver)?;
    if request.instance == request.driver {
        return Err(error("Instance and driver identities must differ"));
    }
    if !matches!(
        doc.get(request.component).map(|e| &e.data),
        Some(EntityData::Part { .. } | EntityData::Assembly { .. })
    ) {
        return Err(error("Choose an existing Part or reusable Assembly"));
    }
    if matches!(doc.get(request.component).map(|e|&e.data),Some(EntityData::Assembly{instances})if instances.is_empty())
    {
        return Err(error(
            "A reusable Assembly needs explicit member Instances before it can be placed",
        ));
    }
    request.parameters.resolve(session.parameters())?;
    let source = InstancePlacementDefinition {
        owner: request.instance,
        base: PlacementBase::Literal {
            pose: Default::default(),
        },
        parameters: request.parameters.clone(),
    };
    let driver = extension(
        request.driver,
        "Component placement",
        source.definition()?,
        source.inputs()?,
    );
    let mut instance = Entity::new(
        &request.name,
        EntityData::Instance {
            component: request.component,
            placement: Placement::default(),
            placement_source: Some(request.driver),
        },
    );
    instance.id = request.instance;
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: "Place reusable component".into(),
        edits: vec![Mutation::Insert(driver), Mutation::Insert(instance)],
    })?;
    Ok(AssemblyPrepared {
        prepared,
        instances: vec![request.instance],
        assemblies: vec![],
        joints: vec![],
        pose: None,
    })
}
fn pose(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &NamedPoseRequest,
    edit: bool,
) -> Result<AssemblyPrepared, CommandError> {
    revision(session, expected_revision)?;
    if edit {
        read_named_pose(session.document(), request.pose)?;
    } else {
        fresh(session.document(), request.pose)?;
    }
    let source = NamedPoseDefinition {
        values: request.values.clone(),
    };
    let mut instances = vec![];
    for entry in &source.values {
        let (mut joint, _) = read_joint(session.document(), entry.joint)?;
        if joint_driver(session.document(), joint.owner)? != Some(entry.joint) {
            return Err(error("Named pose must reference active incoming joints"));
        }
        joint.parameters.values = entry.values.clone();
        joint.parameters.resolve(session.parameters())?;
        instances.push(joint.owner);
    }
    let entity = extension(
        request.pose,
        &request.name,
        source.definition()?,
        source.inputs()?,
    );
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: if edit {
            "Edit named pose"
        } else {
            "Save named pose"
        }
        .into(),
        edits: vec![if edit {
            Mutation::Replace(entity)
        } else {
            Mutation::Insert(entity)
        }],
    })?;
    Ok(AssemblyPrepared {
        prepared,
        instances,
        assemblies: vec![],
        joints: source.values.iter().map(|v| v.joint).collect(),
        pose: Some(request.pose),
    })
}
pub fn prepare_named_pose(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &NamedPoseRequest,
) -> Result<AssemblyPrepared, CommandError> {
    pose(session, revision, actor, request, false)
}
pub fn prepare_named_pose_edit(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &NamedPoseRequest,
) -> Result<AssemblyPrepared, CommandError> {
    pose(session, revision, actor, request, true)
}
pub fn prepare_apply_pose(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    pose: EntityId,
) -> Result<AssemblyPrepared, CommandError> {
    revision(session, expected_revision)?;
    let source = read_named_pose(session.document(), pose)?;
    let mut edits = vec![];
    let mut instances = vec![];
    let mut joints = vec![];
    for entry in source.values {
        let (mut joint, refs) = read_joint(session.document(), entry.joint)?;
        if joint_driver(session.document(), joint.owner)? != Some(entry.joint) {
            return Err(error(
                "Named pose targets an inactive joint; repair the pose",
            ));
        }
        joint.parameters.values = entry.values;
        joint.parameters.resolve(session.parameters())?;
        edits.push(Mutation::Replace(extension(
            entry.joint,
            joint.parameters.kind.label(),
            joint.definition()?,
            joint.inputs(&refs)?,
        )));
        instances.push(joint.owner);
        joints.push(entry.joint);
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!(
            "Apply {}",
            session.document().get(pose).expect("read pose").name
        ),
        edits,
    })?;
    Ok(AssemblyPrepared {
        prepared,
        instances,
        assemblies: vec![],
        joints,
        pose: Some(pose),
    })
}
