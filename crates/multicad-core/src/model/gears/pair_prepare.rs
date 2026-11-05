use super::*;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Entity, Mutation, Placement},
    model::{
        AssemblyDefinition, ComponentPlacementRequest, CreatedBody, InstancePlacementDefinition,
        JointAnchor, JointDefinition, JointParameters, PlacementBase,
        common::{error, revision},
    },
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearPairRequest {
    pub pair: EntityId,
    pub name: String,
    pub gears: [GearRequest; 2],
    pub parameters: GearPairParameters,
    pub outputs: GearPairOutputs,
    /// Ordinary outer occurrence; the reusable pair definition does not own it.
    pub placement: ComponentPlacementRequest,
}
impl GearPairRequest {
    pub fn new(name: impl Into<String>, gears: [GearRequest; 2]) -> Self {
        let name = name.into();
        let outputs = GearPairOutputs {
            assembly: EntityId::new(),
            metadata: EntityId::new(),
            instances: [EntityId::new(), EntityId::new()],
            joints: [EntityId::new(), EntityId::new()],
        };
        let mut placement = ComponentPlacementRequest::new(outputs.assembly, Default::default());
        placement.name = name.clone();
        Self {
            pair: EntityId::new(),
            name,
            parameters: GearPairParameters {
                gears: gears.each_ref().map(|g| g.feature),
                frame: Default::default(),
                drive_angle: angle(0.),
            },
            gears,
            outputs,
            placement,
        }
    }
}
pub struct GearPairPrepared {
    pub prepared: Prepared,
    pub outputs: Vec<CreatedBody>,
    pub pair: EntityId,
    pub assembly: EntityId,
    pub instances: [EntityId; 2],
    pub occurrence: Option<EntityId>,
}
fn extension(id: EntityId, name: &str, definition: Definition, inputs: Vec<Link>) -> Entity {
    let mut e = Entity::new(name, EntityData::Extension { definition, inputs });
    e.id = id;
    e
}
/// Combine typed source into one checked candidate; native evaluation remains
/// required before committing the returned original-Session transaction.
pub fn prepare_gear_pair(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &GearPairRequest,
) -> Result<GearPairPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.parameters.gears != request.gears.each_ref().map(|g| g.feature) {
        return Err(error(
            "Pair feature references disagree with the two gear recipes",
        ));
    }
    if request.name.trim().is_empty() || request.name.len() > 1000 {
        return Err(error("A gear pair needs a name with 1–1000 bytes"));
    }
    if request.placement.component != request.outputs.assembly {
        return Err(error(
            "The initial gear-pair occurrence must place its own reusable Assembly",
        ));
    }
    request.placement.parameters.resolve(session.parameters())?;
    let bodies =
        super::prepare::prepare_gears(session, expected_revision, actor.clone(), &request.gears)?;
    let body_candidate = bodies.prepared.document();
    resolve_gear_pair_parameters(body_candidate, session.parameters(), &request.parameters)?;
    let source = GearPairDefinition {
        parameters: request.parameters.clone(),
        outputs: request.outputs.clone(),
    };
    let mut edits = vec![Mutation::Insert(extension(
        request.pair,
        &request.name,
        source.definition()?,
        source.parameters.inputs()?,
    ))];
    for member in [GearPairMember::A, GearPairMember::B] {
        let index = member.index();
        let joint = JointDefinition {
            owner: source.outputs.instances[index],
            component: request.gears[index].body,
            base: PlacementBase::Literal {
                pose: Default::default(),
            },
            parameters: JointParameters {
                moving: JointAnchor::GearPairFrame {
                    pair: request.pair,
                    member,
                    space: GearPairSpace::Local,
                },
                target: JointAnchor::GearPairFrame {
                    pair: request.pair,
                    member,
                    space: GearPairSpace::Mating,
                },
                ..Default::default()
            },
        };
        edits.push(Mutation::Insert(extension(
            source.outputs.joints[index],
            "Gear pair relationship",
            joint.definition()?,
            joint.inputs(&[])?,
        )));
        let mut instance = Entity::new(
            format!("{} member", request.gears[index].name),
            EntityData::Instance {
                component: request.gears[index].body,
                placement: Placement::default(),
                placement_source: Some(source.outputs.joints[index]),
            },
        );
        instance.id = source.outputs.instances[index];
        edits.push(Mutation::Insert(instance));
    }
    let mut assembly = Entity::new(
        &request.name,
        EntityData::Assembly {
            instances: source.outputs.instances.to_vec(),
        },
    );
    assembly.id = source.outputs.assembly;
    edits.push(Mutation::Insert(assembly));
    let metadata = AssemblyDefinition {
        assembly: source.outputs.assembly,
        definition_parts: request.gears.each_ref().map(|g| g.body).to_vec(),
    };
    edits.push(Mutation::Insert(extension(
        source.outputs.metadata,
        "Gear pair definition Parts",
        metadata.definition()?,
        metadata.inputs()?,
    )));
    let placement = InstancePlacementDefinition {
        owner: request.placement.instance,
        base: PlacementBase::Literal {
            pose: Default::default(),
        },
        parameters: request.placement.parameters.clone(),
    };
    edits.push(Mutation::Insert(extension(
        request.placement.driver,
        "Gear pair placement",
        placement.definition()?,
        placement.inputs()?,
    )));
    let mut occurrence = Entity::new(
        &request.placement.name,
        EntityData::Instance {
            component: source.outputs.assembly,
            placement: Placement::default(),
            placement_source: Some(request.placement.driver),
        },
    );
    occurrence.id = request.placement.instance;
    edits.push(Mutation::Insert(occurrence));
    let mut hierarchy = body_candidate.hierarchy().clone();
    hierarchy
        .members
        .insert(source.outputs.assembly, source.outputs.instances.to_vec());
    hierarchy
        .members
        .entry(body_candidate.id())
        .or_default()
        .push(source.outputs.assembly);
    hierarchy
        .members
        .entry(body_candidate.id())
        .or_default()
        .push(request.placement.instance);
    edits.push(Mutation::SetHierarchy(hierarchy));
    let candidate = body_candidate.edited(&edits)?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Create gear pair".into(),
        edits: crate::model::extrude_boolean::delta(session.document(), &candidate)?,
    })?;
    if prepared.document() != &candidate {
        return Err(error("Gear pair candidate changed while combining source"));
    }
    Ok(GearPairPrepared {
        prepared,
        outputs: bodies.outputs,
        pair: request.pair,
        assembly: source.outputs.assembly,
        instances: source.outputs.instances,
        occurrence: Some(request.placement.instance),
    })
}
pub fn prepare_gear_pair_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    pair: EntityId,
    parameters: &GearPairParameters,
) -> Result<GearPairPrepared, CommandError> {
    pair_edit(session, expected_revision, actor, pair, parameters, None)
}
pub fn prepare_gear_pair_geometry_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    pair: EntityId,
    parameters: &GearPairParameters,
    gears: &[GearParameters; 2],
) -> Result<GearPairPrepared, CommandError> {
    pair_edit(
        session,
        expected_revision,
        actor,
        pair,
        parameters,
        Some(gears),
    )
}
fn pair_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    pair: EntityId,
    parameters: &GearPairParameters,
    gears: Option<&[GearParameters; 2]>,
) -> Result<GearPairPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_gear_pair(session.document(), pair)?;
    if source.parameters.gears != parameters.gears {
        return Err(error(
            "A gear pair edit preserves both member recipes and their identities",
        ));
    }
    let mut candidate = session.document().clone();
    if let Some(gears) = gears {
        for (index, parameters) in gears.iter().enumerate() {
            // Derived Sessions combine checked source edits only. The caller
            // receives one transaction tied to the original live Session.
            let stage = Session::new(candidate)?;
            let edit = super::prepare::prepare_gear_edit_in_pair(
                &stage,
                stage.revision(),
                actor.clone(),
                source.parameters.gears[index],
                parameters,
            )?;
            candidate = edit.prepared.document().clone();
        }
    }
    resolve_gear_pair_parameters(&candidate, session.parameters(), parameters)?;
    source.parameters = parameters.clone();
    let name = &session
        .document()
        .get(pair)
        .ok_or_else(|| error("Gear pair is missing"))?
        .name;
    candidate = candidate.edited(&[Mutation::Replace(extension(
        pair,
        name,
        source.definition()?,
        source.parameters.inputs()?,
    ))])?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit gear pair".into(),
        edits: crate::model::extrude_boolean::delta(session.document(), &candidate)?,
    })?;
    if prepared.document() != &candidate {
        return Err(error("Gear pair edit changed while combining source"));
    }
    let outputs = parameters
        .gears
        .iter()
        .map(|id| {
            read_gear(&candidate, *id).map(|s| CreatedBody {
                feature: *id,
                body: s.output.body,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GearPairPrepared {
        prepared,
        outputs,
        pair,
        assembly: source.outputs.assembly,
        instances: source.outputs.instances,
        occurrence: None,
    })
}
