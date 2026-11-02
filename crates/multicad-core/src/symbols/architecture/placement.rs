//! Source-only role placement. Native adapters must prove the complete prepared
//! candidate before presenting Apply; paths never scale construction geometry.
use super::super::source::entity;
use super::*;
use crate::{constraint_sketch::Dimension, model::*};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchitectureContext {
    Wall {
        path: Vec<[Dimension; 2]>,
        closed: bool,
        level: Option<EntityId>,
        base_z: Dimension,
        flipped: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<ArchitectureWallDetails>,
    },
    Fence {
        path: FencePath,
        level: Option<EntityId>,
        post_positions: Vec<FencePostPosition>,
    },
}
impl ArchitectureContext {
    pub fn role(&self) -> ArchitectureRole {
        match self {
            Self::Wall { .. } => ArchitectureRole::Wall,
            Self::Fence { .. } => ArchitectureRole::Fence,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ArchitecturePlaceRequest {
    pub record: EntityId,
    pub name: String,
    pub definition: EntityId,
    pub ids: BTreeMap<EntityId, EntityId>,
    pub inputs: ArchitectureInputs,
    pub output: ArchitectureOutput,
    pub context: ArchitectureContext,
}
impl ArchitecturePlaceRequest {
    pub fn new(
        document: &Document,
        definition: EntityId,
        name: impl Into<String>,
        inputs: ArchitectureInputs,
        context: ArchitectureContext,
    ) -> Result<Self, DocumentError> {
        let source = read_definition(document, definition)?;
        let blueprint = blueprint_for_definition(&source)?
            .ok_or_else(|| invalid("Choose an architectural symbol for Draw Wall or Draw Fence"))?;
        if blueprint.role != context.role() {
            return Err(invalid(
                "The selected symbol does not support this drawing tool's role",
            ));
        }
        let output = match context.role() {
            ArchitectureRole::Wall => ArchitectureOutput::Wall {
                feature: EntityId::new(),
                body: EntityId::new(),
            },
            ArchitectureRole::Fence => ArchitectureOutput::Fence {
                run: EntityId::new(),
                group: EntityId::new(),
            },
        };
        Ok(Self {
            record: EntityId::new(),
            name: name.into(),
            definition,
            ids: super::super::remap::identities(&source)?
                .into_iter()
                .map(|id| (id, EntityId::new()))
                .collect(),
            inputs,
            output,
            context,
        })
    }
}
#[derive(Clone, Debug)]
pub struct ArchitecturePrepared {
    pub record: EntityId,
    pub output: ArchitectureOutput,
    pub prepared: Prepared,
}
fn output_entities(
    request: &ArchitecturePlaceRequest,
    construction: EntityId,
) -> Result<Vec<Entity>, DocumentError> {
    match (&request.output, &request.context) {
        (
            ArchitectureOutput::Wall { feature, body },
            ArchitectureContext::Wall {
                path,
                closed,
                level,
                base_z,
                flipped,
                details,
            },
        ) => {
            let mut wall = WallDefinition {
                output: BodyOutput { body: *body },
                wall_type: construction,
                layer_placement: Some(WallLayerPlacement {
                    reference: None,
                    flipped: *flipped,
                    junctions: vec![],
                }),
                path: path.clone(),
                closed: *closed,
                level: *level,
                base_z: base_z.clone(),
                thickness: None,
                height: None,
                justification: None,
                openings: vec![],
                cutouts: vec![],
            };
            super::wall_details::apply(&mut wall, details);
            Ok(vec![
                entity(
                    *feature,
                    format!("{} feature", request.name),
                    EntityData::Feature {
                        definition: wall.definition()?,
                        inputs: wall.inputs(),
                        suppressed: false,
                    },
                ),
                entity(
                    *body,
                    &request.name,
                    EntityData::Part {
                        features: vec![*feature],
                        material: None,
                        tags: BTreeSet::new(),
                    },
                ),
            ])
        }
        (
            ArchitectureOutput::Fence { run, group },
            ArchitectureContext::Fence {
                path,
                level,
                post_positions,
            },
        ) => {
            let source = FenceRunDefinition {
                fence_type: construction,
                path: path.clone(),
                group: *group,
                level: *level,
                overrides: FenceInstanceOverrides::default(),
                post_positions: post_positions.clone(),
            };
            Ok(vec![
                entity(*run, &request.name, source.entity_data()?),
                entity(*group, &request.name, EntityData::Group {}),
            ])
        }
        _ => Err(invalid(
            "Architectural placement context and output role disagree",
        )),
    }
}
pub fn prepare_architecture_place(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &ArchitecturePlaceRequest,
) -> Result<ArchitecturePrepared, CommandError> {
    super::super::prepare::stale(session, revision)?;
    if request.name.trim().is_empty() || request.name.len() > 1000 {
        return Err(invalid("Architectural placement name must contain 1-1000 bytes").into());
    }
    let document = session.document();
    let definition = read_definition(document, request.definition)?;
    let blueprint = blueprint_for_definition(&definition)?
        .ok_or_else(|| invalid("Choose an architectural construction definition"))?;
    if blueprint.role != request.context.role() || blueprint.role != request.output.role() {
        return Err(
            invalid("Architectural symbol, receiving context and output roles must agree").into(),
        );
    }
    if request
        .ids
        .values()
        .copied()
        .chain([request.record])
        .chain(request.output.anchors())
        .any(|id| id.is_nil() || id == document.id() || document.get(id).is_some())
    {
        return Err(invalid("New architectural placement needs unused nonempty identities").into());
    }
    let graph = configured_source(&definition, &request.ids, &request.inputs, document)?;
    let value = ArchitectureInstance {
        definition: request.definition,
        pin: definition.pin()?,
        ids: request.ids.clone(),
        inputs: request.inputs.clone(),
        output: request.output.clone(),
    };
    let mut edits: Vec<_> = graph.into_values().map(Mutation::Insert).collect();
    edits.extend(super::wall_details::mutations(document, &request.context)?);
    edits.extend(
        output_entities(request, request.ids[&blueprint.construction])?
            .into_iter()
            .map(Mutation::Insert),
    );
    edits.push(Mutation::Insert(super::super::source::entity(
        request.record,
        "Architectural symbol source",
        value.entity_data()?,
    )));
    let mut hierarchy = document.hierarchy().clone();
    let children = hierarchy
        .members
        .entry(hierarchy.active_component.unwrap_or(document.id()))
        .or_default();
    match request.output {
        ArchitectureOutput::Wall { feature, body } => children.extend([body, feature]),
        ArchitectureOutput::Fence { group, .. } => {
            children.push(group);
            hierarchy.members.insert(group, Vec::new());
        }
    }
    edits.push(Mutation::SetHierarchy(hierarchy));
    let prepared = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: format!("Draw {}", request.name),
        edits,
    })?;
    Ok(ArchitecturePrepared {
        record: request.record,
        output: request.output.clone(),
        prepared,
    })
}
